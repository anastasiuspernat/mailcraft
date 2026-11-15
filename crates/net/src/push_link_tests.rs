// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPad and the relay over an in-memory connection: pairing with the right code and a wrong
//! one, the exchanges that follow, and what a stranger, an old iPad or a changed message gets.

use std::io::{Read, Write};
use std::sync::mpsc::{Receiver, Sender, channel};

use mailcraft_model::{Security, Server};

use crate::push_link::*;

/// One end of an in-memory connection.
struct End {
    tx: Sender<Vec<u8>>,
    rx: Receiver<Vec<u8>>,
    held: Vec<u8>,
    /// Changes every byte sent after this many (a meddler on the way), or never.
    flip_after: Option<usize>,
    sent: usize,
}

fn pipe() -> (End, End) {
    let (a_tx, b_rx) = channel();
    let (b_tx, a_rx) = channel();
    let end = |tx, rx| End { tx, rx, held: Vec::new(), flip_after: None, sent: 0 };
    (end(a_tx, a_rx), end(b_tx, b_rx))
}

impl Read for End {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.held.is_empty() {
            match self.rx.recv() {
                Ok(bytes) => self.held = bytes,
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
        let mut bytes = buf.to_vec();
        if let Some(after) = self.flip_after {
            for (i, b) in bytes.iter_mut().enumerate() {
                // Letters of the base64 only, so that the line still reads as JSON.
                if self.sent + i >= after && b.is_ascii_lowercase() {
                    *b = if *b == b'z' { b'a' } else { *b + 1 };
                }
            }
        }
        self.sent += bytes.len();
        let _ = self.tx.send(bytes);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[derive(Default)]
struct Fake {
    code: String,
    tries: usize,
    devices: Vec<(String, Key)>,
    asked: Vec<Request>,
}

impl Relay for Fake {
    fn code(&mut self) -> Option<String> {
        (self.tries < 5).then(|| self.code.clone())
    }
    fn tried(&mut self) {
        self.tries += 1;
    }
    fn link_key(&mut self, device: &str) -> Option<Key> {
        self.devices.iter().find(|(d, _)| d == device).map(|(_, k)| *k)
    }
    fn paired(&mut self, request: PairRequest, link: Key) -> Result<Answer, String> {
        self.devices.push((request.device, link));
        Ok(Answer { relay: "NUC".into(), accounts: vec![] })
    }
    fn asked(&mut self, _device: &str, request: Request) -> Result<Answer, String> {
        self.asked.push(request);
        Ok(Answer { relay: "NUC".into(), accounts: vec![AccountState { email: "ada@example.org".into(), problem: String::new() }] })
    }
}

fn request() -> PairRequest {
    PairRequest { device: "dev-1".into(), token: "ab".repeat(32), name: "iPad".into(), note_key: key_text(&[7; 32]) }
}

/// Run `ipad` against the relay `fake`, with `relay_end` set up first.
fn run<T: Send + 'static>(fake: Fake, ipad: impl FnOnce(End) -> T + Send + 'static, meddle: Option<usize>) -> (T, Served, Fake) {
    let (mut relay_end, ipad_end) = pipe();
    relay_end.flip_after = meddle;
    let ipad = std::thread::spawn(move || ipad(ipad_end));
    let mut fake = fake;
    let served = serve(relay_end, &mut fake);
    (ipad.join().unwrap(), served, fake)
}

#[test]
fn the_right_code_pairs_and_both_ends_get_the_same_link_key() {
    let fake = Fake { code: "123456".into(), ..Default::default() };
    let (paired, served, fake) = run(fake, |end| pair(end, " 123456 ", &request()), None);
    let (answer, link) = paired.unwrap();
    assert_eq!((answer.relay.as_str(), served), ("NUC", Served::Paired));
    assert_eq!(fake.devices, vec![("dev-1".to_string(), link)]);
}

#[test]
fn a_wrong_code_pairs_nothing_and_counts() {
    let fake = Fake { code: "123456".into(), ..Default::default() };
    let (paired, served, fake) = run(fake, |end| pair(end, "654321", &request()), None);
    assert_eq!((paired.unwrap_err(), served), (LinkError::WrongCode, Served::WrongCode));
    assert!(fake.devices.is_empty() && fake.tries == 1);
    // After five tries in all, nothing is taken, the right code included.
    let fake = Fake { code: "123456".into(), tries: 5, ..Default::default() };
    let (paired, served, _) = run(fake, |end| pair(end, "123456", &request()), None);
    assert_eq!((paired.unwrap_err(), served), (LinkError::TooManyTries, Served::Closed));
}

#[test]
fn a_paired_device_asks_and_is_answered() {
    let link = [9u8; 32];
    let fake = Fake { devices: vec![("dev-1".into(), link)], ..Default::default() };
    let sync = Request::Sync {
        token: "cd".repeat(32),
        name: "iPad".into(),
        accounts: vec![LinkedAccount {
            email: "ada@example.org".into(),
            username: String::new(),
            server: Server { host: "imap.example.org".into(), port: 993, security: Security::Tls },
            sign_in: None,
            client_id: String::new(),
            secret: "hunter2".into(),
        }],
    };
    let asked = sync.clone();
    let (answer, served, fake) = run(fake, move |end| exchange(end, "dev-1", &link, &asked), None);
    assert_eq!((answer.unwrap().accounts.len(), served), (1, Served::Answered));
    assert_eq!(fake.asked, vec![sync]);
    // The secret is never in a debug line.
    assert!(!format!("{:?}", fake.asked).contains("hunter2"));
}

#[test]
fn a_stranger_a_wrong_link_key_and_a_changed_message_get_nothing() {
    // A device the relay doesn't know.
    let (answer, served, _) = run(Fake::default(), |end| exchange(end, "dev-9", &[1; 32], &Request::Forget), None);
    assert_eq!((answer.unwrap_err(), served), (LinkError::NotPaired, Served::NotPaired));
    // The right id with another key: the relay's proof doesn't hold.
    let fake = Fake { devices: vec![("dev-1".into(), [9; 32])], ..Default::default() };
    let (answer, served, fake) = run(fake, |end| exchange(end, "dev-1", &[8; 32], &Request::Forget), None);
    assert_eq!(answer.unwrap_err(), LinkError::Untrusted);
    assert!(served != Served::Answered && fake.asked.is_empty());
    // Something on the way changes what the relay sends after its opening line.
    let fake = Fake { devices: vec![("dev-1".into(), [9; 32])], ..Default::default() };
    let (answer, _, _) = run(fake, |end| exchange(end, "dev-1", &[9; 32], &Request::Forget), Some(20));
    assert!(answer.is_err());
}

#[test]
fn an_ipad_of_the_first_version_is_refused_without_its_code_being_read() {
    let (mut relay_end, mut ipad_end) = pipe();
    ipad_end.write_all(b"{\"code\":\"123456\",\"token\":\"ab\",\"name\":\"iPad\"}\n").unwrap();
    let mut fake = Fake { code: "123456".into(), ..Default::default() };
    assert_eq!(serve(&mut relay_end, &mut fake), Served::Junk);
    assert_eq!(fake.tries, 0);
    let mut said = vec![0u8; 256];
    let n = ipad_end.read(&mut said).unwrap();
    assert!(String::from_utf8_lossy(&said[..n]).contains("old version"));
}

#[test]
fn a_note_opens_only_with_its_key_and_ids_say_nothing_of_the_account() {
    let key = [3u8; 32];
    let note = Note { account: "ada@example.org".into(), count: 2, from: "Maya".into(), subject: "Mockups".into() };
    let sealed = seal_note(&key, &note).unwrap();
    assert!(!sealed.contains("ada") && !sealed.contains("Maya"));
    assert_eq!(open_note(&key, &sealed), Some(note));
    assert_eq!(open_note(&[4u8; 32], &sealed), None);
    for junk in ["", "!!", "AAAA", &"A".repeat(20_000)] {
        assert_eq!(open_note(&key, junk), None);
    }
    let id = opaque_id(&key, "Ada@Example.org");
    assert_eq!((id.len(), id == opaque_id(&key, "ada@example.org")), (32, true));
    assert!(!id.contains("ada"));
    assert_eq!(key_from(&key_text(&key)), Some(key));
    assert_eq!(key_from("short"), None);
}
