// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPad and the owner's push relay (`apps/mailcraft-relay`) talking on the home network: the
//! pairing, then every later exchange, and the notification text only the iPad can read.
//!
//! **Pairing** proves that both ends know the six-digit code the relay shows, without the code
//! crossing the network and without a recording of the exchange letting anybody try codes at
//! leisure: SPAKE2 (a password-authenticated key exchange, RFC 9382), with the iPad as side A and
//! the relay as side B. Each end then shows that it holds the same key (HMAC-SHA-256 of a label),
//! the relay first; the iPad's proof comes with its request, sealed. A wrong code is found out
//! with one guess per connection, and the relay counts them. From the key come a key for the
//! pairing's own messages and the **link key**, which both ends keep (in the system's credential
//! store) for the exchanges that follow.
//!
//! **An exchange** later (the iPad's push address, its accounts, Forget) starts afresh each time:
//! each end sends a random nonce and an ephemeral X25519 key; the session's keys come from the
//! link key and the Diffie-Hellman secret (HKDF-SHA-256), so a recording can't be read later even
//! by somebody who learns the link key, and only an end that holds the link key can take part.
//! The relay proves itself first; the iPad's proof comes with its request.
//!
//! Every message after the opening one is sealed with ChaCha20-Poly1305, its nonce a counter (odd
//! from the iPad, even from the relay), so nothing can be read, changed, replayed or reordered.
//! Lines are JSON, at most [`MOST_LINE`] bytes; anything else ends the connection.
//!
//! **A notification's text** (which account, who from, the subject) is sealed with a key the iPad
//! made and gave the relay when it paired, so that Apple carries it without reading it; the
//! iPad's notification extension opens it ([`open_note`]).

use std::io::{BufRead, BufReader, Read, Write};

use mailcraft_charset::base64::STANDARD;
use mailcraft_model::{OAuthProvider, Server};
use ring::aead::{CHACHA20_POLY1305, LessSafeKey, Nonce, UnboundKey};
use ring::agreement::{EphemeralPrivateKey, UnparsedPublicKey, X25519, agree_ephemeral};
use ring::hmac;
use ring::rand::{SecureRandom, SystemRandom};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use spake2::{Ed25519Group, Identity, Password, Spake2};

/// The port the relay listens on, on the home network.
pub const PORT: u16 = 7993;
/// The version of this protocol.
pub const VERSION: u32 = 2;
/// The longest line taken.
pub const MOST_LINE: usize = 64 * 1024;
/// The most accounts one request names.
pub const MOST_ACCOUNTS: usize = 20;
/// The length of every key here.
pub const KEY_LEN: usize = 32;

const IPAD: &[u8] = b"MailCraft iPad";
const RELAY: &[u8] = b"MailCraft relay";

/// A key of [`KEY_LEN`] bytes.
pub type Key = [u8; KEY_LEN];

/// Why an exchange didn't happen.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LinkError {
    /// The code was wrong (the pairing).
    WrongCode,
    /// The relay refuses every try for a while.
    TooManyTries,
    /// The relay doesn't know this device (any more): pair again.
    NotPaired,
    /// The other end isn't what it says, or changed what was sent.
    Untrusted,
    /// The other end said no, with its reason.
    Refused(String),
    /// The connection.
    Io(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::WrongCode => write!(f, "the code is wrong"),
            LinkError::TooManyTries => write!(f, "too many wrong codes lately: try again in an hour"),
            LinkError::NotPaired => write!(f, "the relay doesn't know this device: pair it again"),
            LinkError::Untrusted => write!(f, "the other end couldn't prove it is the relay this device paired with"),
            LinkError::Refused(why) => write!(f, "the relay said no: {why}"),
            LinkError::Io(why) => write!(f, "{why}"),
        }
    }
}

/// What the iPad sends when it pairs.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairRequest {
    /// The device's own id, made once (random), which a new push address keeps.
    pub device: String,
    /// Its push address from Apple (hexadecimal).
    pub token: String,
    pub name: String,
    /// The key its notifications' text is sealed with (base64).
    pub note_key: String,
}

impl std::fmt::Debug for PairRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PairRequest").field("device", &self.device).field("name", &self.name).finish_non_exhaustive()
    }
}

/// An account the iPad wants notifications for, with what the relay signs in with.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LinkedAccount {
    pub email: String,
    #[serde(default)]
    pub username: String,
    pub server: Server,
    /// Signs in on its provider's page; the secret is then the sign-in's refresh token.
    #[serde(default)]
    pub sign_in: Option<OAuthProvider>,
    /// The application id the sign-in was made for.
    #[serde(default)]
    pub client_id: String,
    /// The password, or the refresh token.
    pub secret: String,
}

impl std::fmt::Debug for LinkedAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkedAccount").field("email", &self.email).field("server", &self.server.host).finish_non_exhaustive()
    }
}

/// What the iPad asks once linked.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "camelCase")]
pub enum Request {
    /// Its push address now, its name, and every account it wants notifications for (the whole
    /// list: an account left out is dropped for this device).
    Sync { token: String, name: String, accounts: Vec<LinkedAccount> },
    /// Forget this device.
    Forget,
}

/// How an account is doing on the relay.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountState {
    pub email: String,
    /// Empty while it is watched; else what is wrong.
    #[serde(default)]
    pub problem: String,
}

/// The relay's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Answer {
    /// The computer's name.
    pub relay: String,
    #[serde(default)]
    pub accounts: Vec<AccountState>,
}

/// The text of a notification, sealed for the iPad.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Note {
    /// The account.
    #[serde(rename = "a")]
    pub account: String,
    /// How many new messages.
    #[serde(rename = "n")]
    pub count: usize,
    /// Who the newest is from.
    #[serde(rename = "f", default)]
    pub from: String,
    /// Its subject.
    #[serde(rename = "s", default)]
    pub subject: String,
}

// ---- keys and seals

fn random<const N: usize>() -> Result<[u8; N], LinkError> {
    let mut bytes = [0u8; N];
    SystemRandom::new().fill(&mut bytes).map_err(|_| LinkError::Io("no random numbers".into()))?;
    Ok(bytes)
}

/// A fresh key (a note key, a device id's bytes).
pub fn new_key() -> Result<Key, LinkError> {
    random()
}

struct Len;
impl ring::hkdf::KeyType for Len {
    fn len(&self) -> usize {
        KEY_LEN
    }
}

/// HKDF-SHA-256: `ikm` with `salt`, for `info`.
fn derive(salt: &[u8], ikm: &[u8], info: &[&[u8]]) -> Result<Key, LinkError> {
    let prk = ring::hkdf::Salt::new(ring::hkdf::HKDF_SHA256, salt).extract(ikm);
    let mut out = [0u8; KEY_LEN];
    prk.expand(info, Len).and_then(|okm| okm.fill(&mut out)).map_err(|_| LinkError::Io("a key couldn't be made".into()))?;
    Ok(out)
}

fn proof(key: &Key, who: &[u8]) -> Vec<u8> {
    hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), who).as_ref().to_vec()
}

fn proves(key: &Key, who: &[u8], said: &str) -> bool {
    let Ok(said) = STANDARD.decode(said.as_bytes()) else { return false };
    hmac::verify(&hmac::Key::new(hmac::HMAC_SHA256, key), who, &said).is_ok()
}

/// The seal of one session: ChaCha20-Poly1305 with counters for nonces.
struct Seal {
    key: LessSafeKey,
    send: u64,
    receive: u64,
}

impl Seal {
    /// For the end `ipad` (its counters odd) or the relay (even).
    fn new(key: &Key, ipad: bool) -> Result<Seal, LinkError> {
        let unbound = UnboundKey::new(&CHACHA20_POLY1305, key).map_err(|_| LinkError::Io("the seal couldn't be made".into()))?;
        let (send, receive) = if ipad { (1, 2) } else { (2, 1) };
        Ok(Seal { key: LessSafeKey::new(unbound), send, receive })
    }

    fn nonce(n: u64) -> Nonce {
        let mut bytes = [0u8; 12];
        bytes[4..].copy_from_slice(&n.to_be_bytes());
        Nonce::assume_unique_for_key(bytes)
    }

    fn seal(&mut self, value: &impl Serialize) -> Result<String, LinkError> {
        let mut text = serde_json::to_vec(value).map_err(|e| LinkError::Io(e.to_string()))?;
        self.key
            .seal_in_place_append_tag(Self::nonce(self.send), ring::aead::Aad::empty(), &mut text)
            .map_err(|_| LinkError::Io("a message couldn't be sealed".into()))?;
        self.send = self.send.saturating_add(2);
        Ok(STANDARD.encode(&text))
    }

    fn open<T: DeserializeOwned>(&mut self, sealed: &str) -> Result<T, LinkError> {
        let mut bytes = STANDARD.decode(sealed.as_bytes()).map_err(|_| LinkError::Untrusted)?;
        let text = self.key.open_in_place(Self::nonce(self.receive), ring::aead::Aad::empty(), &mut bytes).map_err(|_| LinkError::Untrusted)?;
        self.receive = self.receive.saturating_add(2);
        serde_json::from_slice(text).map_err(|_| LinkError::Untrusted)
    }
}

/// The text of a notification, sealed with the iPad's note key: base64 of a random nonce and the
/// sealed JSON.
pub fn seal_note(key: &Key, note: &Note) -> Result<String, LinkError> {
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key).map_err(|_| LinkError::Io("the seal couldn't be made".into()))?;
    let nonce: [u8; 12] = random()?;
    let mut text = serde_json::to_vec(note).map_err(|e| LinkError::Io(e.to_string()))?;
    LessSafeKey::new(unbound)
        .seal_in_place_append_tag(Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(b"note"), &mut text)
        .map_err(|_| LinkError::Io("the note couldn't be sealed".into()))?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&text);
    Ok(STANDARD.encode(&out))
}

/// A notification's text, opened with the iPad's note key; `None` when it isn't one sealed with it.
pub fn open_note(key: &Key, sealed: &str) -> Option<Note> {
    if sealed.len() > 8 * 1024 {
        return None;
    }
    let bytes = STANDARD.decode(sealed.as_bytes()).ok()?;
    let (nonce, rest) = bytes.split_at_checked(12)?;
    let nonce: [u8; 12] = nonce.try_into().ok()?;
    let unbound = UnboundKey::new(&CHACHA20_POLY1305, key).ok()?;
    let mut rest = rest.to_vec();
    let text = LessSafeKey::new(unbound).open_in_place(Nonce::assume_unique_for_key(nonce), ring::aead::Aad::from(b"note"), &mut rest).ok()?;
    serde_json::from_slice(text).ok()
}

/// An id for an account that says nothing of it to Apple (a notification's thread and collapse
/// id): 16 bytes of HMAC with the note key, in hexadecimal.
pub fn opaque_id(key: &Key, account: &str) -> String {
    let tag = hmac::sign(&hmac::Key::new(hmac::HMAC_SHA256, key), account.to_ascii_lowercase().as_bytes());
    tag.as_ref().iter().take(16).map(|b| format!("{b:02x}")).collect()
}

/// A key from its base64.
pub fn key_from(text: &str) -> Option<Key> {
    STANDARD.decode(text.trim().as_bytes()).ok()?.try_into().ok()
}

/// A key as base64.
pub fn key_text(key: &Key) -> String {
    STANDARD.encode(key)
}

// ---- lines

struct Line<S: Read + Write> {
    stream: BufReader<S>,
}

impl<S: Read + Write> Line<S> {
    fn new(stream: S) -> Self {
        Line { stream: BufReader::new(stream) }
    }

    fn send(&mut self, value: &serde_json::Value) -> Result<(), LinkError> {
        let text = format!("{value}\n");
        let w = self.stream.get_mut();
        w.write_all(text.as_bytes()).and_then(|_| w.flush()).map_err(|e| LinkError::Io(format!("the connection broke ({e})")))
    }

    fn receive(&mut self) -> Result<serde_json::Value, LinkError> {
        let mut bytes = Vec::new();
        let read = (&mut self.stream)
            .take(MOST_LINE as u64 + 1)
            .read_until(b'\n', &mut bytes)
            .map_err(|e| LinkError::Io(format!("the connection broke ({e})")))?;
        if read == 0 {
            return Err(LinkError::Io("the other end closed the connection".into()));
        }
        if bytes.len() > MOST_LINE || bytes.last() != Some(&b'\n') {
            return Err(LinkError::Untrusted);
        }
        serde_json::from_slice(&bytes).map_err(|_| LinkError::Untrusted)
    }
}

fn text<'a>(value: &'a serde_json::Value, key: &str) -> &'a str {
    value.get(key).and_then(serde_json::Value::as_str).unwrap_or("")
}

/// A refusal in an answer, as an error.
fn refusal(value: &serde_json::Value) -> Option<LinkError> {
    if value.get("ok").and_then(serde_json::Value::as_bool) != Some(false) {
        return None;
    }
    Some(match text(value, "why") {
        "wrong code" => LinkError::WrongCode,
        "too many tries" => LinkError::TooManyTries,
        "not paired" => LinkError::NotPaired,
        "old version" => LinkError::Refused("the iPad and the relay are different versions: update both".into()),
        why => LinkError::Refused(why.chars().filter(|c| !c.is_control()).take(200).collect()),
    })
}

fn no(why: &str) -> serde_json::Value {
    serde_json::json!({ "ok": false, "why": why })
}

// ---- the iPad's end

/// Pair over `stream` with `code`: the relay's answer, and the link key to keep.
pub fn pair<S: Read + Write>(stream: S, code: &str, request: &PairRequest) -> Result<(Answer, Key), LinkError> {
    let mut line = Line::new(stream);
    let (state, message) = Spake2::<Ed25519Group>::start_a(&Password::new(code.trim().as_bytes()), &Identity::new(IPAD), &Identity::new(RELAY));
    line.send(&serde_json::json!({ "v": VERSION, "kind": "pair", "spake": STANDARD.encode(&message) }))?;
    let heard = line.receive()?;
    if let Some(e) = refusal(&heard) {
        return Err(e);
    }
    let theirs = STANDARD.decode(text(&heard, "spake").as_bytes()).map_err(|_| LinkError::Untrusted)?;
    let shared = state.finish(&theirs).map_err(|_| LinkError::Untrusted)?;
    let (confirm, sealing, link) = pair_keys(&shared)?;
    // A relay that can't prove the same key was given another code.
    if !proves(&confirm, RELAY, text(&heard, "confirm")) {
        return Err(LinkError::WrongCode);
    }
    let mut seal = Seal::new(&sealing, true)?;
    line.send(&serde_json::json!({ "confirm": STANDARD.encode(proof(&confirm, IPAD)), "box": seal.seal(request)? }))?;
    let heard = line.receive()?;
    if let Some(e) = refusal(&heard) {
        return Err(e);
    }
    let answer: Answer = seal.open(text(&heard, "box"))?;
    Ok((answer, link))
}

fn pair_keys(shared: &[u8]) -> Result<(Key, Key, Key), LinkError> {
    let salt = b"MailCraft pairing v2";
    Ok((derive(salt, shared, &[b"confirm"])?, derive(salt, shared, &[b"seal"])?, derive(salt, shared, &[b"link"])?))
}

fn link_keys(link: &Key, secret: &[u8], transcript: &[&[u8]]) -> Result<(Key, Key), LinkError> {
    let mut info: Vec<&[u8]> = vec![b"MailCraft link v2"];
    info.extend_from_slice(transcript);
    let session = derive(link, secret, &info)?;
    Ok((derive(&session, b"", &[b"confirm"])?, derive(&session, b"", &[b"seal"])?))
}

/// Ask the relay `request` over `stream`, as `device` with its link key.
pub fn exchange<S: Read + Write>(stream: S, device: &str, link: &Key, request: &Request) -> Result<Answer, LinkError> {
    let mut line = Line::new(stream);
    let rng = SystemRandom::new();
    let mine = EphemeralPrivateKey::generate(&X25519, &rng).map_err(|_| LinkError::Io("no random numbers".into()))?;
    let my_public = mine.compute_public_key().map_err(|_| LinkError::Io("no key".into()))?.as_ref().to_vec();
    let my_nonce: [u8; 16] = random()?;
    line.send(&serde_json::json!({
        "v": VERSION, "kind": "link", "device": device,
        "nonce": STANDARD.encode(my_nonce), "key": STANDARD.encode(&my_public),
    }))?;
    let heard = line.receive()?;
    if let Some(e) = refusal(&heard) {
        return Err(e);
    }
    let their_nonce = STANDARD.decode(text(&heard, "nonce").as_bytes()).map_err(|_| LinkError::Untrusted)?;
    let their_public = STANDARD.decode(text(&heard, "key").as_bytes()).map_err(|_| LinkError::Untrusted)?;
    if their_nonce.len() != 16 || their_public.len() != 32 {
        return Err(LinkError::Untrusted);
    }
    let secret = agree_ephemeral(mine, &UnparsedPublicKey::new(&X25519, &their_public), |s| s.to_vec()).map_err(|_| LinkError::Untrusted)?;
    let (confirm, sealing) = link_keys(link, &secret, &[device.as_bytes(), &my_nonce, &their_nonce, &my_public, &their_public])?;
    if !proves(&confirm, RELAY, text(&heard, "confirm")) {
        return Err(LinkError::Untrusted);
    }
    let mut seal = Seal::new(&sealing, true)?;
    line.send(&serde_json::json!({ "confirm": STANDARD.encode(proof(&confirm, IPAD)), "box": seal.seal(request)? }))?;
    let heard = line.receive()?;
    if let Some(e) = refusal(&heard) {
        return Err(e);
    }
    seal.open(text(&heard, "box"))
}

// ---- the relay's end

/// What the relay knows and does, for [`serve`].
pub trait Relay {
    /// The pairing code now; `None`: pairing is closed (too many tries lately).
    fn code(&mut self) -> Option<String>;
    /// A code was tried. Every try counts, right or wrong: an end that guessed may hang up after
    /// the relay's proof has told it whether the guess was right.
    fn tried(&mut self);
    /// The link key of `device`, when it is paired.
    fn link_key(&mut self, device: &str) -> Option<Key>;
    /// A device paired, with the link key to keep.
    fn paired(&mut self, request: PairRequest, link: Key) -> Result<Answer, String>;
    /// A paired device asks.
    fn asked(&mut self, device: &str, request: Request) -> Result<Answer, String>;
}

/// What came of one connection.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Served {
    Paired,
    Answered,
    WrongCode,
    Closed,
    NotPaired,
    /// Not this protocol, an older version, a broken connection or an end that couldn't prove
    /// itself.
    Junk,
}

/// Answer one connection on the relay's side.
pub fn serve<S: Read + Write>(stream: S, relay: &mut impl Relay) -> Served {
    let mut line = Line::new(stream);
    let Ok(opening) = line.receive() else { return Served::Junk };
    if opening.get("v").and_then(serde_json::Value::as_u64) != Some(u64::from(VERSION)) {
        // (An iPad of the first version sends its code in the clear: it is refused unread.)
        let _ = line.send(&no("old version"));
        return Served::Junk;
    }
    match text(&opening, "kind") {
        "pair" => serve_pair(&mut line, &opening, relay),
        "link" => serve_link(&mut line, &opening, relay),
        _ => Served::Junk,
    }
}

fn serve_pair<S: Read + Write>(line: &mut Line<S>, opening: &serde_json::Value, relay: &mut impl Relay) -> Served {
    let Some(code) = relay.code() else {
        let _ = line.send(&no("too many tries"));
        return Served::Closed;
    };
    let Ok(theirs) = STANDARD.decode(text(opening, "spake").as_bytes()) else { return Served::Junk };
    let (state, message) = Spake2::<Ed25519Group>::start_b(&Password::new(code.as_bytes()), &Identity::new(IPAD), &Identity::new(RELAY));
    let Ok(shared) = state.finish(&theirs) else { return Served::Junk };
    let Ok((confirm, sealing, link)) = pair_keys(&shared) else { return Served::Junk };
    relay.tried();
    if line.send(&serde_json::json!({ "spake": STANDARD.encode(&message), "confirm": STANDARD.encode(proof(&confirm, RELAY)) })).is_err() {
        return Served::Junk;
    }
    // (An iPad that was given another code finds out from the relay's proof and hangs up.)
    let Ok(heard) = line.receive() else { return Served::WrongCode };
    if !proves(&confirm, IPAD, text(&heard, "confirm")) {
        let _ = line.send(&no("wrong code"));
        return Served::WrongCode;
    }
    let Ok(mut seal) = Seal::new(&sealing, false) else { return Served::Junk };
    let Ok(request) = seal.open::<PairRequest>(text(&heard, "box")) else { return Served::Junk };
    match relay.paired(request, link).and_then(|answer| seal.seal(&answer).map_err(|e| e.to_string())) {
        Ok(sealed) => {
            let _ = line.send(&serde_json::json!({ "ok": true, "box": sealed }));
            Served::Paired
        }
        Err(why) => {
            let _ = line.send(&no(&why));
            Served::Junk
        }
    }
}

fn serve_link<S: Read + Write>(line: &mut Line<S>, opening: &serde_json::Value, relay: &mut impl Relay) -> Served {
    let device = text(opening, "device").to_string();
    let Some(link) = relay.link_key(&device) else {
        let _ = line.send(&no("not paired"));
        return Served::NotPaired;
    };
    let (Ok(their_nonce), Ok(their_public)) = (STANDARD.decode(text(opening, "nonce").as_bytes()), STANDARD.decode(text(opening, "key").as_bytes()))
    else {
        return Served::Junk;
    };
    if their_nonce.len() != 16 || their_public.len() != 32 {
        return Served::Junk;
    }
    let rng = SystemRandom::new();
    let Ok(mine) = EphemeralPrivateKey::generate(&X25519, &rng) else { return Served::Junk };
    let Ok(my_public) = mine.compute_public_key().map(|k| k.as_ref().to_vec()) else { return Served::Junk };
    let Ok(my_nonce) = random::<16>() else { return Served::Junk };
    let Ok(secret) = agree_ephemeral(mine, &UnparsedPublicKey::new(&X25519, &their_public), |s| s.to_vec()) else { return Served::Junk };
    let Ok((confirm, sealing)) = link_keys(&link, &secret, &[device.as_bytes(), &their_nonce, &my_nonce, &their_public, &my_public]) else {
        return Served::Junk;
    };
    let hello = serde_json::json!({ "nonce": STANDARD.encode(my_nonce), "key": STANDARD.encode(&my_public), "confirm": STANDARD.encode(proof(&confirm, RELAY)) });
    if line.send(&hello).is_err() {
        return Served::Junk;
    }
    let Ok(heard) = line.receive() else { return Served::Junk };
    if !proves(&confirm, IPAD, text(&heard, "confirm")) {
        return Served::Junk;
    }
    let Ok(mut seal) = Seal::new(&sealing, false) else { return Served::Junk };
    let Ok(request) = seal.open::<Request>(text(&heard, "box")) else { return Served::Junk };
    match relay.asked(&device, request).and_then(|answer| seal.seal(&answer).map_err(|e| e.to_string())) {
        Ok(sealed) => {
            let _ = line.send(&serde_json::json!({ "ok": true, "box": sealed }));
            Served::Answered
        }
        Err(why) => {
            let _ = line.send(&no(&why));
            Served::Answered
        }
    }
}
