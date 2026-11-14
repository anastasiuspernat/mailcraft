// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pairing an iPad, on the home network: the relay listens on [`crate::PAIR_PORT`] for ten
//! minutes and shows a six-digit code and this computer's address; the iPad (File › Options ›
//! Push from your computer) connects, and sends one line of JSON with the code, its push address
//! from Apple and its name. The relay answers with one line: yes, with this computer's name, or
//! no. Five wrong codes end the pairing.
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
/// Wrong codes taken before the pairing ends.
const MOST_WRONG: usize = 5;
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
fn serve_one(stream: TcpStream, code: &str, wrong: &mut usize) -> Option<Device> {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let mut reader = BufReader::new(stream.try_clone().ok()?).take(MOST_LINE as u64);
    let mut line = String::new();
    reader.read_line(&mut line).ok()?;
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
            *wrong += 1;
            None
        }
        Heard::Junk => None,
    }
}

/// Listen on `listener` until an iPad pairs with `code`, [`WAIT`] passes, or too many codes were wrong.
pub fn serve(listener: &TcpListener, code: &str) -> Result<Device, String> {
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let started = Instant::now();
    let mut wrong = 0;
    while started.elapsed() < WAIT {
        match listener.accept() {
            Ok((stream, from)) => {
                let _ = stream.set_nonblocking(false);
                log::info!("pairing: a connection from {}", from.ip());
                if let Some(device) = serve_one(stream, code, &mut wrong) {
                    return Ok(device);
                }
                if wrong >= MOST_WRONG {
                    return Err("too many wrong codes: pairing stopped".into());
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => std::thread::sleep(Duration::from_millis(200)),
            Err(e) => return Err(format!("the pairing stopped ({e})")),
        }
    }
    Err("no iPad paired within ten minutes".into())
}
