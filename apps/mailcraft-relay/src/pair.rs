// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPads, on the home network: the running relay listens on [`crate::PAIR_PORT`] for as long
//! as it runs ([`serve_forever`]; `mailcraft-relay pair` listens for ten minutes by itself when
//! the relay isn't running, [`serve`]). The protocol is `mailcraft_net::push_link`: an iPad pairs
//! with the six-digit code kept in the settings (`mailcraft-relay code` shows it with this
//! computer's address), proved by SPAKE2 so that the code never crosses the network, and gets a
//! link key; with it, sealed, it later sends its push address and the accounts it wants
//! notifications for, with what the relay signs in to them with ([`Door::asked`]), or asks to be
//! forgotten. Five pairing tries in an hour, and every try is refused until the hour is up.
//!
//! What is kept: the link key and the note key (the key a notification's text is sealed with for
//! that iPad) in the credential store; the device's id, name, push address and accounts in the
//! settings; an account's password or sign-in in the credential store, dropped with the account
//! once no device wants it.

use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use mailcraft_model::Security;
use mailcraft_net::push_link::{self, AccountState, Answer, Key, LinkedAccount, PairRequest, Request, Served};

use crate::config::{self, Device, Settings, Watched};
use crate::store;

/// How long `mailcraft-relay pair` waits for an iPad.
pub const WAIT: Duration = Duration::from_secs(10 * 60);
/// Pairing tries taken in an hour.
const MOST_TRIES: usize = 5;
/// How long a try counts.
const HOUR: Duration = Duration::from_secs(60 * 60);
/// The longest secret taken for an account.
const MOST_SECRET: usize = 8 * 1024;

/// The pairing tries of the last hour.
#[derive(Debug, Default)]
pub struct Tries {
    tried: Vec<Instant>,
}

impl Tries {
    /// Too many tries lately: nothing is taken.
    pub fn blocked(&mut self) -> bool {
        self.tried.retain(|t| t.elapsed() < HOUR);
        self.tried.len() >= MOST_TRIES
    }

    fn note(&mut self) {
        self.tried.push(Instant::now());
    }
}

/// A fresh six-digit code.
pub fn new_code() -> String {
    let mut bytes = [0u8; 4];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

/// This computer's name, for the iPad to show.
pub fn computer_name() -> String {
    let name = std::env::var("COMPUTERNAME").or_else(|_| std::env::var("HOSTNAME")).unwrap_or_default();
    let name: String = name.chars().filter(|c| !c.is_control()).take(60).collect();
    if name.is_empty() { "your computer".into() } else { name }
}

/// This computer's address on the home network: the one its route out leaves from. (Asking the
/// system which address a UDP socket would use sends nothing.)
pub fn home_address() -> Option<std::net::IpAddr> {
    let socket = UdpSocket::bind("0.0.0.0:0").ok()?;
    socket.connect("192.0.2.1:9").ok()?;
    let ip = socket.local_addr().ok()?.ip();
    (!ip.is_unspecified() && !ip.is_loopback()).then_some(ip)
}

fn now() -> u64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0)
}

/// One line of text from outside: no control characters, at most `most` characters.
fn line(text: &str, most: usize) -> String {
    text.chars().filter(|c| !c.is_control()).take(most).collect::<String>().trim().to_string()
}

/// How the accounts are doing, as the watchers say (by address, lower case; empty: watched).
pub type States = std::sync::Mutex<std::collections::HashMap<String, String>>;

/// The relay's side of the protocol, over the settings.
pub struct Door<'a> {
    pub settings: &'a Settings,
    pub tries: &'a mut Tries,
    /// How the accounts are doing (absent outside the running relay).
    pub states: Option<&'a States>,
    /// Called when the accounts to watch changed.
    pub changed: &'a dyn Fn(),
    /// A device that paired (for `mailcraft-relay pair` to say so).
    pub paired: Option<String>,
}

impl Door<'_> {
    fn state_of(&self, email: &str) -> String {
        let Some(states) = self.states else { return String::new() };
        states.lock().ok().and_then(|s| s.get(&email.to_ascii_lowercase()).cloned()).unwrap_or_else(|| "starting".into())
    }

    /// Take a device's accounts: their secrets into the credential store, their settings into
    /// the list; the accounts no device wants any more go, with their secrets.
    fn sync(&mut self, device: &str, token: &str, name: &str, accounts: Vec<LinkedAccount>) -> Result<Answer, String> {
        if !mailcraft_net::apns::is_device_token(token) {
            return Err("that isn't a push address".into());
        }
        let mut problems: Vec<AccountState> = Vec::new();
        let mut wanted: Vec<Watched> = Vec::new();
        // An account added here by hand keeps its own sign-in.
        let by_hand: Vec<String> = self.settings.read()?.accounts.iter().filter(|a| a.by_hand).map(|a| a.email.to_ascii_lowercase()).collect();
        for a in accounts.into_iter().take(push_link::MOST_ACCOUNTS) {
            let email = line(&a.email, 254);
            match checked(&a) {
                Ok(()) => {}
                Err(why) => {
                    problems.push(AccountState { email, problem: why });
                    continue;
                }
            }
            let watched = Watched {
                email: email.clone(),
                username: line(&a.username, 254),
                server: mailcraft_model::Server { host: a.server.host.to_ascii_lowercase(), port: a.server.port, security: a.server.security },
                sign_in: a.sign_in,
                client_id: line(&a.client_id, 200),
                by_hand: false,
            };
            wanted.push(watched.clone());
            if by_hand.contains(&email.to_ascii_lowercase()) {
                continue;
            }
            // The secret, kept as the other kind gone (a password become a sign-in, say).
            let other =
                Watched { sign_in: if watched.sign_in.is_some() { None } else { Some(mailcraft_model::OAuthProvider::Google) }, ..watched.clone() };
            if store::get(&watched.secret_key()).as_deref() != Some(a.secret.as_str())
                && let Err(e) = store::set(&watched.secret_key(), &a.secret)
            {
                problems.push(AccountState { email, problem: e });
                wanted.pop();
                continue;
            }
            store::delete(&other.secret_key());
        }
        let (name, token, device_id) = (line(name, 60), token.to_ascii_lowercase(), device.to_string());
        let (dropped, mine) = self.settings.update(|c| {
            for w in &wanted {
                let room = c.accounts.len() < config::MAX_ACCOUNTS;
                match c.accounts.iter_mut().find(|x| x.email.eq_ignore_ascii_case(&w.email)) {
                    // One added here by hand stays as it was set up.
                    Some(known) if known.by_hand => {}
                    Some(known) => *known = w.clone(),
                    None if room => c.accounts.push(w.clone()),
                    None => {}
                }
            }
            if let Some(d) = c.devices.iter_mut().find(|d| d.id == device_id) {
                d.token = token.clone();
                if !name.is_empty() {
                    d.name = name.clone();
                }
                d.accounts = wanted.iter().map(|w| w.email.to_ascii_lowercase()).collect();
                d.seen = now();
            }
            // (Another device that had this push address has a new one by now.)
            c.devices.retain(|d| d.id == device_id || !d.token.eq_ignore_ascii_case(&token));
            let dropped = drop_unwanted(c);
            let mine: Vec<String> =
                c.accounts.iter().filter(|a| c.devices.iter().any(|d| d.id == device_id && d.wants(a))).map(|a| a.email.clone()).collect();
            (dropped, mine)
        })?;
        forget_secrets(&dropped);
        (self.changed)();
        let mut states: Vec<AccountState> = mine.iter().map(|email| AccountState { email: email.clone(), problem: self.state_of(email) }).collect();
        states.extend(problems);
        Ok(Answer { relay: computer_name(), accounts: states })
    }

    fn forget(&mut self, device: &str) -> Result<Answer, String> {
        let dropped = self.settings.update(|c| {
            c.devices.retain(|d| d.id != device);
            drop_unwanted(c)
        })?;
        store::delete(&Device::link_key_name(device));
        store::delete(&Device::note_key_name(device));
        forget_secrets(&dropped);
        (self.changed)();
        log::info!("an iPad asked to be forgotten, and was");
        Ok(Answer { relay: computer_name(), accounts: vec![] })
    }
}

/// Whether an account from an iPad can be watched: an address, a server name, an encrypted
/// connection, a secret of a sensible size.
fn checked(a: &LinkedAccount) -> Result<(), String> {
    if !a.email.contains('@') || a.email.len() > 254 {
        return Err("that isn't an e-mail address".into());
    }
    if config::typed_server(&a.server.host, false).is_none() || a.server.port == 0 {
        return Err("its server isn't a host name".into());
    }
    if !matches!(a.server.security, Security::Tls | Security::StartTls) {
        return Err("its server isn't reached over an encrypted connection".into());
    }
    if a.secret.is_empty() || a.secret.len() > MOST_SECRET {
        return Err("there is nothing to sign in with".into());
    }
    Ok(())
}

/// Take the accounts no device wants out of `c` (and what was seen of them); the ones taken.
fn drop_unwanted(c: &mut config::Config) -> Vec<Watched> {
    let dropped = c.unwanted();
    for d in &dropped {
        c.accounts.retain(|a| !a.email.eq_ignore_ascii_case(&d.email));
        c.seen.remove(&d.email.to_ascii_lowercase());
    }
    dropped
}

fn forget_secrets(dropped: &[Watched]) {
    for d in dropped {
        store::delete(&d.secret_key());
        log::info!("{}: no iPad wants it any more: no longer watched, and its sign-in is forgotten", d.email);
    }
}

impl push_link::Relay for Door<'_> {
    fn code(&mut self) -> Option<String> {
        if self.tries.blocked() {
            return None;
        }
        config::pairing_code(self.settings.path(), false).map_err(|e| log::warn!("{e}")).ok()
    }

    fn tried(&mut self) {
        self.tries.note();
    }

    fn link_key(&mut self, device: &str) -> Option<Key> {
        let known = self.settings.read().ok()?.devices.iter().any(|d| !d.id.is_empty() && d.id == device);
        if !known {
            return None;
        }
        push_link::key_from(&store::get(&Device::link_key_name(device))?)
    }

    fn paired(&mut self, request: PairRequest, link: Key) -> Result<Answer, String> {
        let id = line(&request.device, 64);
        if id.len() < 16 || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
            return Err("that isn't a device id".into());
        }
        if !mailcraft_net::apns::is_device_token(&request.token) {
            return Err("that isn't a push address".into());
        }
        let note = push_link::key_from(&request.note_key).ok_or("that isn't a key")?;
        store::set(&Device::link_key_name(&id), &push_link::key_text(&link))?;
        store::set(&Device::note_key_name(&id), &push_link::key_text(&note))?;
        let name = line(&request.name, 60);
        let device = Device {
            id: id.clone(),
            token: request.token.to_ascii_lowercase(),
            name: if name.is_empty() { "iPad".into() } else { name },
            accounts: vec![],
            seen: now(),
        };
        let shown = device.name.clone();
        let added = self.settings.update(|c| c.pair(device))?;
        if !added {
            store::delete(&Device::link_key_name(&id));
            store::delete(&Device::note_key_name(&id));
            return Err(format!("{} devices are paired already: that is the most", config::MAX_DEVICES));
        }
        self.paired = Some(shown);
        Ok(Answer { relay: computer_name(), accounts: vec![] })
    }

    fn asked(&mut self, device: &str, request: Request) -> Result<Answer, String> {
        match request {
            Request::Sync { token, name, accounts } => self.sync(device, &token, &name, accounts),
            Request::Forget => self.forget(device),
        }
    }
}

/// Answer one connection.
fn answer(stream: TcpStream, door: &mut Door) -> Served {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(20)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(20)));
    push_link::serve(stream, door)
}

/// Listen on `listener` until an iPad pairs, [`WAIT`] passes, or too many codes were tried.
pub fn serve(listener: &TcpListener, settings: &Settings) -> Result<String, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut tries = Tries::default();
    while started.elapsed() < WAIT {
        match listener.accept() {
            Ok((stream, from)) => {
                let _ = stream.set_nonblocking(false);
                log::info!("pairing: a connection from {}", from.ip());
                let mut door = Door { settings, tries: &mut tries, states: None, changed: &|| {}, paired: None };
                let served = answer(stream, &mut door);
                if let Some(name) = door.paired.take() {
                    return Ok(name);
                }
                log::info!("pairing: {served:?}");
                if tries.blocked() {
                    return Err("too many tries: pairing stopped".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("the pairing stopped ({e})")),
        }
    }
    Err("no iPad paired within ten minutes".into())
}

/// Listen on `listener` for as long as the relay runs.
pub fn serve_forever(listener: &TcpListener, settings: &Settings, states: &States, changed: &dyn Fn()) {
    let mut tries = Tries::default();
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let from = stream.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
        let mut door = Door { settings, tries: &mut tries, states: Some(states), changed, paired: None };
        let served = answer(stream, &mut door);
        match (&door.paired, served) {
            (Some(name), _) => log::info!("paired with {name} at {from}"),
            (None, Served::Answered) => log::info!("an iPad at {from} brought its accounts up to date"),
            (None, other) => log::info!("a connection from {from}: {other:?}"),
        }
    }
}
