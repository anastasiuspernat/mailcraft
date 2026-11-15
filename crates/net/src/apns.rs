// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Apple's push service (APNs): a notification to an iPad's MailCraft, sent by the owner's own
//! relay (`apps/mailcraft-relay`). Apple's documentation, "Sending notification requests to
//! APNs" and "Establishing a token-based connection to APNs", is the specification.
//!
//! The request is an HTTP/2 POST to `api.push.apple.com` (`crate::h2`) with a JSON body, the app's
//! bundle id as its topic and a provider token: a JWT signed with ES256 by the owner's own key
//! (the `.p8` file Apple gives a developer account, with its key id and the team's id). The key
//! never leaves this computer, and the token is made again only every 40 minutes, as Apple asks
//! (between 20 and 60). The connection is kept for the next notification, as Apple asks, and
//! made again when it fails or has been idle for a while. Nothing goes with a notification but
//! what its payload says: the relay seals which account has new mail, who from and its subject
//! with a key only the iPad holds (`push_link::seal_note`), and Apple sees "New mail".

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mailcraft_charset::base64::{STANDARD, URL_SAFE_NO_PAD};
use mailcraft_model::{Security, Server};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair};

use crate::transport::{self, Patience, Service};

/// Apple's push server for apps from the App Store and TestFlight.
pub const PRODUCTION: &str = "api.push.apple.com";
/// Apple's push server for apps built for development.
pub const SANDBOX: &str = "api.sandbox.push.apple.com";
/// How long a provider token is used.
const TOKEN_LIFE: Duration = Duration::from_secs(40 * 60);
/// The longest payload Apple takes for a notification.
const MAX_PAYLOAD: usize = 4096;
/// A connection idle this long is made again rather than trusted.
const IDLE_CONNECTION: Duration = Duration::from_secs(20 * 60);
/// How long Apple keeps a notification for a device that is off: then it is dropped.
const KEEP_FOR: Duration = Duration::from_secs(24 * 60 * 60);

/// The owner's key for Apple's push service.
pub struct Key {
    key_id: String,
    team_id: String,
    pkcs8: Vec<u8>,
}

impl std::fmt::Debug for Key {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Key").field("key_id", &self.key_id).field("team_id", &self.team_id).finish_non_exhaustive()
    }
}

/// An id Apple gives (a key's, a team's): ten letters and digits.
fn is_apple_id(id: &str) -> bool {
    id.len() == 10 && id.bytes().all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
}

impl Key {
    /// The key from the text of Apple's `.p8` file (PEM, PKCS #8), its id and the team's.
    pub fn from_p8(text: &str, key_id: &str, team_id: &str) -> Result<Key, String> {
        let (key_id, team_id) = (key_id.trim().to_ascii_uppercase(), team_id.trim().to_ascii_uppercase());
        if !is_apple_id(&key_id) {
            return Err("a key id is ten letters and digits, as Apple shows it".into());
        }
        if !is_apple_id(&team_id) {
            return Err("a team id is ten letters and digits, as Apple shows it".into());
        }
        let inner: String = text
            .lines()
            .map(str::trim)
            .skip_while(|l| !l.starts_with("-----BEGIN PRIVATE KEY-----"))
            .skip(1)
            .take_while(|l| !l.starts_with("-----END"))
            .collect();
        let pkcs8 = STANDARD.decode(inner.as_bytes()).map_err(|_| "this isn't the text of a .p8 key from Apple".to_string())?;
        EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &pkcs8, &SystemRandom::new())
            .map_err(|_| "this .p8 file isn't an Apple push key (a P-256 key)".to_string())?;
        Ok(Key { key_id, team_id, pkcs8 })
    }

    /// A provider token issued at `iat` (seconds since 1970).
    fn token(&self, iat: u64) -> Result<String, String> {
        let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &self.pkcs8, &SystemRandom::new())
            .map_err(|_| "the push key can't be used".to_string())?;
        let header = serde_json::json!({ "alg": "ES256", "kid": self.key_id }).to_string();
        let claims = serde_json::json!({ "iss": self.team_id, "iat": iat }).to_string();
        let signed = format!("{}.{}", URL_SAFE_NO_PAD.encode(header), URL_SAFE_NO_PAD.encode(claims));
        let signature = pair.sign(&SystemRandom::new(), signed.as_bytes()).map_err(|_| "the push key couldn't sign".to_string())?;
        Ok(format!("{signed}.{}", URL_SAFE_NO_PAD.encode(signature.as_ref())))
    }
}

/// Why a notification wasn't taken.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PushError {
    /// The device's token is no good any more (the app was removed, or the token is for another
    /// app or server): stop sending to it.
    Gone(String),
    /// Anything else, Apple's reason when it gave one.
    Failed(String),
}

impl std::fmt::Display for PushError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PushError::Gone(why) => write!(f, "the device is no longer there ({why})"),
            PushError::Failed(why) => write!(f, "{why}"),
        }
    }
}

/// A device's token as the iPad gives it: hexadecimal.
pub fn is_device_token(token: &str) -> bool {
    (32..=200).contains(&token.len()) && token.len().is_multiple_of(2) && token.bytes().all(|b| b.is_ascii_hexdigit())
}

/// What goes with one notification besides its payload.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Notice<'a> {
    /// Notifications with the same id replace one another on the device (at most 64 bytes).
    pub collapse_id: Option<&'a str>,
}

/// A connection kept to Apple: the stream, the next stream id, when it was last used.
struct Open {
    stream: transport::Stream<'static>,
    next: u32,
    used: Instant,
}

/// A sender of notifications: the key, the token made with it, and the connection.
pub struct Apns {
    key: Key,
    host: String,
    token: Option<(String, Instant)>,
    open: Option<Open>,
}

impl Apns {
    /// To Apple's production server (`PRODUCTION`), or its sandbox.
    pub fn new(key: Key, host: &str) -> Apns {
        Apns { key, host: host.to_string(), token: None, open: None }
    }

    fn provider_token(&mut self) -> Result<String, PushError> {
        if let Some((token, made)) = &self.token
            && made.elapsed() < TOKEN_LIFE
        {
            return Ok(token.clone());
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let token = self.key.token(now).map_err(PushError::Failed)?;
        self.token = Some((token.clone(), Instant::now()));
        Ok(token)
    }

    fn connect(&self) -> Result<Open, PushError> {
        let server = Server { host: self.host.clone(), port: 443, security: Security::Tls };
        let config = crate::tls::client_config(crate::tls::Purpose::Push).map_err(PushError::Failed)?;
        let (stream, _) = transport::open_with(&server, Service::Http, &Patience::default(), Some(config))
            .map_err(|e| PushError::Failed(format!("Apple's push server couldn't be reached: {e}")))?;
        if stream.alpn().as_deref() != Some(b"h2".as_slice()) {
            return Err(PushError::Failed("Apple's push server didn't agree to HTTP/2".into()));
        }
        Ok(Open { stream, next: 1, used: Instant::now() })
    }

    /// Send `payload` (JSON) to `device` for the app `topic` (its bundle id), as an alert, on the
    /// kept connection (made again once when it fails).
    pub fn send(&mut self, device: &str, topic: &str, payload: &str, notice: &Notice) -> Result<(), PushError> {
        if !is_device_token(device) {
            return Err(PushError::Gone("not a device token".into()));
        }
        if payload.len() > MAX_PAYLOAD {
            return Err(PushError::Failed("the notification is too long".into()));
        }
        let bearer = format!("bearer {}", self.provider_token()?);
        let expires = (SystemTime::now() + KEEP_FOR).duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).to_string();
        let mut headers = vec![
            ("authorization", bearer.as_str()),
            ("apns-topic", topic),
            ("apns-push-type", "alert"),
            ("apns-priority", "10"),
            ("apns-expiration", expires.as_str()),
            ("content-type", "application/json"),
        ];
        if let Some(id) = notice.collapse_id.filter(|id| !id.is_empty() && id.len() <= 64) {
            headers.push(("apns-collapse-id", id));
        }
        let path = format!("/3/device/{device}");
        for attempt in 0..2 {
            let stale = self.open.as_ref().is_none_or(|o| o.used.elapsed() > IDLE_CONNECTION || o.next > crate::h2::LAST_STREAM);
            if stale {
                self.open = None;
                self.open = Some(self.connect()?);
            }
            let Some(open) = self.open.as_mut() else { continue };
            let stream_id = open.next;
            open.next = open.next.saturating_add(2);
            open.used = Instant::now();
            match crate::h2::post_on(&mut open.stream, stream_id, &self.host, &path, &headers, payload.as_bytes()) {
                Ok(answer) => return verdict(answer.status, &answer.body),
                // A kept connection Apple has let go of: once more on a new one.
                Err(e) => {
                    self.open = None;
                    if attempt == 1 || stream_id == 1 {
                        return Err(PushError::Failed(format!("Apple's push server: {e}")));
                    }
                }
            }
        }
        Err(PushError::Failed("Apple's push server couldn't be reached".into()))
    }
}

/// What Apple's answer means.
pub(crate) fn verdict(status: Option<u16>, body: &[u8]) -> Result<(), PushError> {
    let reason = serde_json::from_slice::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("reason").and_then(|r| r.as_str()).map(|r| r.chars().take(80).collect::<String>()));
    match (status, reason) {
        (Some(200), None) => Ok(()),
        (_, Some(reason)) if ["BadDeviceToken", "Unregistered", "DeviceTokenNotForTopic", "ExpiredToken"].contains(&reason.as_str()) => {
            Err(PushError::Gone(reason))
        }
        (Some(410), _) => Err(PushError::Gone("Unregistered".into())),
        (_, Some(reason)) => Err(PushError::Failed(format!("Apple refused the notification: {reason}"))),
        (Some(status), None) => Err(PushError::Failed(format!("Apple answered {status}"))),
        (None, None) => Err(PushError::Failed("Apple's answer couldn't be read".into())),
    }
}

/// The payload of a notification that `account` has `count` new messages: the account and the
/// count, and nothing of the messages.
pub fn new_mail_payload(account: &str, count: usize, body: &str) -> String {
    let account: String = account.chars().take(200).collect();
    serde_json::json!({
        "aps": {
            "alert": { "title": account, "body": body },
            "sound": "default",
            "thread-id": account,
        },
        "count": count,
    })
    .to_string()
}

/// The payload of a notification whose text is sealed for the iPad (`push_link::seal_note`):
/// Apple carries `title` and `body` (words that say nothing of the mail, shown as they are when
/// the iPad can't open the rest), and the iPad's notification extension (`mutable-content`)
/// replaces them with what `sealed` says. `thread` groups an account's notifications without
/// naming it (`push_link::opaque_id`).
pub fn sealed_payload(sealed: &str, thread: &str, title: &str, body: &str) -> String {
    serde_json::json!({
        "aps": {
            "alert": { "title": title, "body": body },
            "sound": "default",
            "mutable-content": 1,
            "thread-id": thread,
        },
        "m": sealed,
    })
    .to_string()
}

#[cfg(test)]
pub(crate) mod testing {
    /// A token made with `key` at `iat`, for the tests.
    pub(crate) fn token(key: &super::Key, iat: u64) -> String {
        key.token(iat).unwrap()
    }
    pub(crate) use super::verdict;
}
