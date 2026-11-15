// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The relay's side of the iPads, over an in-memory connection and an in-memory credential store:
//! pairing, an iPad's accounts coming and going, two iPads with accounts of their own, Forget.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};

use mailcraft_model::{OAuthProvider, Security, Server};
use mailcraft_net::push_link::{self, LinkedAccount, PairRequest, Request, Served};

use crate::config::{Config, Device, Settings, Watched, file_in};
use crate::pair::{Door, Tries, new_code};
use crate::store;

struct End {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    held: Vec<u8>,
}

fn pipe() -> (End, End) {
    let (a_tx, b_rx) = channel();
    let (b_tx, a_rx) = channel();
    (End { tx: a_tx, rx: a_rx, held: vec![] }, End { tx: b_tx, rx: b_rx, held: vec![] })
}

impl Read for End {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.held.is_empty() {
            match self.rx.recv() {
                Ok(b) => self.held = b,
                Err(_) => return Ok(0),
            }
        }
        let n = buf.len().min(self.held.len());
        buf[..n].copy_from_slice(&self.held[..n]);
        self.held.drain(..n);
        Ok(n)
    }
}

impl Write for End {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let _ = self.tx.send(buf.to_vec());
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-relay-pair-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// One connection: `ipad` on its end, the relay's door on the other.
fn talk<T: Send + 'static>(settings: &Settings, tries: &mut Tries, ipad: impl FnOnce(End) -> T + Send + 'static) -> (T, Served, Option<String>) {
    let (relay_end, ipad_end) = pipe();
    let ipad = std::thread::spawn(move || ipad(ipad_end));
    let mut door = Door { settings, tries, states: None, changed: &|| {}, paired: None };
    let served = push_link::serve(relay_end, &mut door);
    (ipad.join().unwrap(), served, door.paired)
}

fn token(n: u8) -> String {
    format!("{n:02x}").repeat(32)
}

fn pair_one(settings: &Settings, device: &str, n: u8) -> push_link::Key {
    let code = crate::config::pairing_code(settings.path(), false).unwrap();
    let request = PairRequest { device: device.into(), token: token(n), name: format!("iPad {n}"), note_key: push_link::key_text(&[n; 32]) };
    let ((answer, link), served, paired) = {
        let (r, s, p) = talk(settings, &mut Tries::default(), move |end| push_link::pair(end, &code, &request));
        (r.unwrap(), s, p)
    };
    assert_eq!((served, paired.as_deref()), (Served::Paired, Some(format!("iPad {n}").as_str())));
    assert!(!answer.relay.is_empty());
    link
}

fn account(email: &str, secret: &str) -> LinkedAccount {
    LinkedAccount {
        email: email.into(),
        username: String::new(),
        server: Server { host: "imap.example.org".into(), port: 993, security: Security::Tls },
        sign_in: None,
        client_id: String::new(),
        secret: secret.into(),
    }
}

fn sync(settings: &Settings, device: &str, link: push_link::Key, n: u8, accounts: Vec<LinkedAccount>) -> push_link::Answer {
    let (device, request) = (device.to_string(), Request::Sync { token: token(n), name: format!("iPad {n}"), accounts });
    let (answer, served, _) = talk(settings, &mut Tries::default(), move |end| push_link::exchange(end, &device, &link, &request));
    assert_eq!(served, Served::Answered);
    answer.unwrap()
}

fn secret_of(config: &Config, email: &str) -> Option<String> {
    store::get(&config.account(email)?.secret_key())
}

#[test]
fn a_code_is_six_digits() {
    for _ in 0..50 {
        let code = new_code();
        assert_eq!(code.len(), 6);
        assert!(code.bytes().all(|b| b.is_ascii_digit()));
    }
}

#[test]
fn two_ipads_bring_their_own_accounts_and_take_them_away() {
    let dir = temp("two");
    let settings = Settings::new(file_in(&dir));
    let (a, b) = ("aaaaaaaaaaaaaaaa-1", "bbbbbbbbbbbbbbbb-2");
    let (link_a, link_b) = (pair_one(&settings, a, 1), pair_one(&settings, b, 2));
    assert_eq!(store::get(&Device::note_key_name(a)), Some(push_link::key_text(&[1; 32])));

    let answer = sync(&settings, a, link_a, 1, vec![account("ada@pair.example", "pw-a"), account("shared@pair.example", "pw-s")]);
    assert_eq!(answer.accounts.len(), 2);
    sync(&settings, b, link_b, 2, vec![account("bo@pair.example", "pw-b"), account("shared@pair.example", "pw-s2")]);
    let config = settings.read().unwrap();
    assert_eq!(config.accounts.len(), 3);
    assert_eq!(secret_of(&config, "shared@pair.example").as_deref(), Some("pw-s2"), "the latest secret is kept");
    let wants = |device: &str, email: &str| {
        let c = settings.read().unwrap();
        let d = c.devices.iter().find(|d| d.id == device).unwrap().clone();
        c.account(email).is_some_and(|w| d.wants(w))
    };
    assert!(wants(a, "ada@pair.example") && !wants(b, "ada@pair.example") && wants(b, "shared@pair.example"));
    // The settings file holds no secret.
    let text = std::fs::read_to_string(file_in(&dir)).unwrap();
    assert!(!text.contains("pw-") && !text.contains(&push_link::key_text(&link_a)));

    // A leaves the shared account out: B still wants it, so it stays; A's own goes, with its secret.
    sync(&settings, a, link_a, 1, vec![account("shared@pair.example", "pw-s2")]);
    sync(&settings, a, link_a, 1, vec![]);
    let config = settings.read().unwrap();
    assert!(config.account("ada@pair.example").is_none() && store::get("password:ada@pair.example").is_none());
    assert!(config.account("shared@pair.example").is_some());

    // B is forgotten: its accounts go too, and its keys.
    let (answer, _, _) = talk(&settings, &mut Tries::default(), move |end| push_link::exchange(end, b, &link_b, &Request::Forget));
    answer.unwrap();
    let config = settings.read().unwrap();
    assert!(config.accounts.is_empty() && config.devices.len() == 1);
    assert!(store::get(&Device::link_key_name(b)).is_none() && store::get("password:shared@pair.example").is_none());
    // And can't ask again.
    let (answer, served, _) = talk(&settings, &mut Tries::default(), move |end| push_link::exchange(end, b, &link_b, &Request::Forget));
    assert_eq!((answer.unwrap_err(), served), (push_link::LinkError::NotPaired, Served::NotPaired));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_account_added_by_hand_stays_as_it_was_set_up() {
    let dir = temp("hand");
    let settings = Settings::new(file_in(&dir));
    settings
        .update(|c| {
            c.accounts.push(Watched {
                email: "own@pair.example".into(),
                server: crate::config::typed_server("imap.gmail.com", false).unwrap(),
                sign_in: Some(OAuthProvider::Google),
                by_hand: true,
                ..Default::default()
            })
        })
        .unwrap();
    store::set("signin:own@pair.example", "kept-token").unwrap();
    let id = "cccccccccccccccc-3";
    let link = pair_one(&settings, id, 3);
    sync(&settings, id, link, 3, vec![account("own@pair.example", "other")]);
    let config = settings.read().unwrap();
    let own = config.account("own@pair.example").unwrap();
    assert!(own.by_hand && own.sign_in.is_some());
    assert_eq!(store::get("signin:own@pair.example").as_deref(), Some("kept-token"));
    // Leaving it out drops nothing that was added by hand.
    sync(&settings, id, link, 3, vec![]);
    assert!(settings.read().unwrap().account("own@pair.example").is_some());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn what_an_ipad_may_not_bring() {
    let dir = temp("bad");
    let settings = Settings::new(file_in(&dir));
    let id = "dddddddddddddddd-4";
    let link = pair_one(&settings, id, 4);
    let mut plain = account("plain@pair.example", "pw");
    plain.server.security = Security::None;
    let mut no_secret = account("empty@pair.example", "");
    no_secret.secret.clear();
    let mut bad_host = account("host@pair.example", "pw");
    bad_host.server.host = "evil host/..".into();
    let answer = sync(&settings, id, link, 4, vec![plain, no_secret, bad_host, account("not-an-address", "pw")]);
    assert_eq!(answer.accounts.iter().filter(|a| !a.problem.is_empty()).count(), 4);
    assert!(settings.read().unwrap().accounts.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn five_tries_an_hour_close_pairing() {
    let dir = temp("tries");
    let settings = Settings::new(file_in(&dir));
    let code = crate::config::pairing_code(settings.path(), false).unwrap();
    let wrong = if code == "000000" { "111111".to_string() } else { "000000".to_string() };
    let mut tries = Tries::default();
    for _ in 0..5 {
        let request = PairRequest { device: "eeeeeeeeeeeeeeee-5".into(), token: token(5), name: "x".into(), note_key: push_link::key_text(&[5; 32]) };
        let w = wrong.clone();
        let (r, served, _) = talk(&settings, &mut tries, move |end| push_link::pair(end, &w, &request));
        assert_eq!((r.unwrap_err(), served), (push_link::LinkError::WrongCode, Served::WrongCode));
    }
    let request = PairRequest { device: "eeeeeeeeeeeeeeee-5".into(), token: token(5), name: "x".into(), note_key: push_link::key_text(&[5; 32]) };
    let (r, served, _) = talk(&settings, &mut tries, move |end| push_link::pair(end, &code, &request));
    assert_eq!((r.unwrap_err(), served), (push_link::LinkError::TooManyTries, Served::Closed));
    assert!(settings.read().unwrap().devices.is_empty());
    let _ = std::fs::remove_dir_all(&dir);
}
