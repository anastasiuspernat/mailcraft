// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pairing an iPad, on the home network: the running relay listens on [`crate::PAIR_PORT`] for as
//! long as it runs ([`serve_forever`]; `mailcraft-relay pair` listens for ten minutes by itself
//! when the relay isn't running, [`serve`]), with the six-digit code kept in its settings
//! (`mailcraft-relay code` shows it with this computer's address). The iPad (File › Options ›
//! Push from your computer) connects, and sends one line of JSON with the code, its push address
//! from Apple and its name. The relay answers with one line: yes, with this computer's name, or
//! no. After five wrong codes in an hour, every try is refused until the hour is up.
//!
//! What crosses the network is the push address and the code, and nothing else: a push address
//! is no secret (only the owner's push key can send to it), but the code keeps somebody else on
//! the network from adding theirs.

use std::io::{BufRead, BufReader, Read as _, Write};
use std::net::{TcpListener, TcpStream, UdpSocket};
use std::time::{Duration, Instant};

use serde::Deserialize;

use crate::config::Device;

/// How long the relay waits for an iPad.
pub const WAIT: Duration = Duration::from_secs(10 * 60);
/// Wrong codes taken in an hour.
const MOST_WRONG: usize = 5;
/// How long a wrong code counts.
const HOUR: Duration = Duration::from_secs(60 * 60);

/// The wrong codes of the last hour.
#[derive(Debug, Default)]
pub struct Tries {
    wrong: Vec<Instant>,
}

impl Tries {
    /// Too many wrong codes lately: nothing is taken.
    pub fn blocked(&mut self) -> bool {
        self.wrong.retain(|t| t.elapsed() < HOUR);
        self.wrong.len() >= MOST_WRONG
    }

    fn note_wrong(&mut self) {
        self.wrong.push(Instant::now());
    }
}
/// The longest line taken from an iPad.
const MOST_LINE: usize = 2048;

#[derive(Deserialize)]
struct Asked {
    code: String,
    token: String,
    #[serde(default)]
    name: String,
}

/// What came of one iPad's line.
#[derive(Debug, PartialEq, Eq)]
pub enum Heard {
    /// The right code and a push address.
    Paired(Device),
    /// A wrong code.
    Wrong,
    /// Not a pairing at all.
    Junk,
}

/// A fresh six-digit code.
pub fn new_code() -> String {
    let mut bytes = [0u8; 4];
    let _ = ring::rand::SecureRandom::fill(&ring::rand::SystemRandom::new(), &mut bytes);
    format!("{:06}", u32::from_le_bytes(bytes) % 1_000_000)
}

/// What one line from an iPad means, for the pairing with `code`.
pub fn hear(line: &str, code: &str) -> Heard {
    let Ok(asked) = serde_json::from_str::<Asked>(line.trim()) else { return Heard::Junk };
    if !mailcraft_net::apns::is_device_token(&asked.token) {
        return Heard::Junk;
    }
    if asked.code.trim() != code {
        return Heard::Wrong;
    }
    let name: String = asked.name.chars().filter(|c| !c.is_control()).take(60).collect();
    Heard::Paired(Device { token: asked.token.to_ascii_lowercase(), name: if name.trim().is_empty() { "iPad".into() } else { name } })
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

/// Answer one connection; `None` while nothing was paired by it.
fn serve_one(stream: TcpStream, code: &str, tries: &mut Tries) -> Option<Device> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().ok()?).take(MOST_LINE as u64);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
    if tries.blocked() {
        let mut stream = stream;
        let _ = stream.write_all(format!("{}\n", serde_json::json!({ "ok": false, "why": "too many tries" })).as_bytes());
        return None;
    }
    let heard = hear(&line, code);
    let reply = match &heard {
        Heard::Paired(_) => serde_json::json!({ "ok": true, "relay": computer_name() }),
        Heard::Wrong => serde_json::json!({ "ok": false, "why": "wrong code" }),
        Heard::Junk => serde_json::json!({ "ok": false, "why": "not a pairing" }),
    };
    let mut stream = stream;
    let _ = stream.write_all(format!("{reply}\n").as_bytes());
    match heard {
        Heard::Paired(device) => Some(device),
        Heard::Wrong => {
            tries.note_wrong();
            None
        }
        Heard::Junk => None,
    }
}

/// Listen on `listener` until an iPad pairs with `code`, [`WAIT`] passes, or too many codes were wrong.
pub fn serve(listener: &TcpListener, code: &str) -> Result<Device, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut tries = Tries::default();
    while started.elapsed() < WAIT {
        match listener.accept() {
            Ok((stream, from)) => {
                let _ = stream.set_nonblocking(false);
                log::info!("pairing: a connection from {}", from.ip());
                if let Some(device) = serve_one(stream, code, &mut tries) {
                    return Ok(device);
                }
                if tries.blocked() {
                    return Err("too many wrong codes: pairing stopped".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("the pairing stopped ({e})")),
        }
    }
    Err("no iPad paired within ten minutes".into())
}

/// Listen on `listener` for as long as the relay runs: each connection is checked against the
/// code `code` gives at the time (it may have been renewed), and an iPad that pairs goes to `paired`.
pub fn serve_forever(listener: &TcpListener, code: impl Fn() -> Option<String>, paired: impl Fn(Device)) {
    let mut tries = Tries::default();
    for stream in listener.incoming() {
        let Ok(stream) = stream else {
            std::thread::sleep(Duration::from_secs(1));
            continue;
        };
        let from = stream.peer_addr().map(|a| a.ip().to_string()).unwrap_or_default();
        let Some(code) = code() else { continue };
        match serve_one(stream, &code, &mut tries) {
            Some(device) => {
                log::info!("paired with an iPad at {from}");
                paired(device);
            }
            None => log::info!("pairing: a connection from {from} paired nothing"),
        }
    }
}
