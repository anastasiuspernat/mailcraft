// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The parts of the push relay in this crate: the least of HTTP/2 (`h2`), the provider token and
//! answers of Apple's push service (`apns`), and the Inbox watcher (`watch`) against the fake
//! IMAP server.

use std::io::{Cursor, Read, Write};

use mailcraft_charset::base64::{STANDARD, URL_SAFE_NO_PAD};
use mailcraft_model::{Account, AccountKind, Security, Server};
use ring::rand::SystemRandom;
use ring::signature::{ECDSA_P256_SHA256_FIXED, ECDSA_P256_SHA256_FIXED_SIGNING, EcdsaKeyPair, KeyPair, UnparsedPublicKey};

use crate::apns::{Key, PushError, is_device_token, new_mail_payload};
use crate::fake::{FakeImap, FakeMailbox, FakeMessage, ImapWorld};
use crate::h2::testing::frame;

/// A connection whose server says `answer`; what the client wrote is kept.
struct Wire {
    answer: Cursor<Vec<u8>>,
    written: Vec<u8>,
}

impl Read for Wire {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.answer.read(buf)
    }
}

impl Write for Wire {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.written.extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn wire(frames: &[Vec<u8>]) -> Wire {
    Wire { answer: Cursor::new(frames.concat()), written: Vec::new() }
}

fn post(w: &mut Wire) -> Result<crate::h2::Answer, String> {
    crate::h2::post(w, "api.push.apple.com", "/3/device/abcd", &[("apns-topic", "org.example.app")], b"{}")
}

#[test]
fn a_request_and_its_accepted_answer() {
    let mut w = wire(&[frame(0x4, 0, 0, &[0, 3, 0, 0, 0, 100]), frame(0x4, 1, 0, &[]), frame(0x1, 0x4 | 0x1, 1, &[0x88])]);
    let answer = post(&mut w).unwrap();
    assert_eq!(answer.status, Some(200));
    assert!(answer.body.is_empty());
    let written = &w.written;
    assert!(written.starts_with(b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n"));
    // The request's headers: POST, https, the authority and path as literals, then ours.
    let headers = frame(
        0x1,
        0x4,
        1,
        &[&[0x83, 0x87, 0x01, 18][..], b"api.push.apple.com", &[0x04, 14], b"/3/device/abcd", &[0x00, 10], b"apns-topic", &[15], b"org.example.app"]
            .concat(),
    );
    assert!(written.windows(headers.len()).any(|w| w == headers.as_slice()), "the HEADERS frame");
    assert!(written.windows(11).any(|w| w == frame(0x0, 0x1, 1, b"{}").as_slice()), "the body ends the stream");
    assert!(written.ends_with(&frame(0x4, 1, 0, &[])), "the server's settings acknowledged");
}

#[test]
fn a_refusal_comes_with_its_reason() {
    let mut w = wire(&[
        frame(0x6, 0, 0, b"12345678"),
        frame(0x1, 0x4, 1, &[0x8c]),
        frame(0x8, 0, 0, &[0, 0, 1, 0]),
        frame(0x0, 0x1, 1, br#"{"reason":"BadDeviceToken"}"#),
    ]);
    let answer = post(&mut w).unwrap();
    assert_eq!(answer.status, Some(400));
    assert_eq!(answer.body, br#"{"reason":"BadDeviceToken"}"#);
    assert!(w.written.windows(17).any(|x| x == frame(0x6, 1, 0, b"12345678").as_slice()), "the ping answered");
}

#[test]
fn a_status_written_as_a_literal_is_read() {
    // `:status 410` as a literal with the name of static entry 8, then the body.
    let mut w = wire(&[frame(0x1, 0x4, 1, &[0x08, 3, b'4', b'1', b'0']), frame(0x0, 0x1, 1, br#"{"reason":"Unregistered"}"#)]);
    assert_eq!(post(&mut w).unwrap().status, Some(410));
}

#[test]
fn broken_answers_are_errors() {
    for frames in [
        vec![],
        vec![frame(0x7, 0, 0, &[0, 0, 0, 0, 0, 0, 0, 2, b'n', b'o'])],
        vec![frame(0x3, 0, 1, &[0, 0, 0, 2])],
        vec![frame(0x0, 0x8 | 0x1, 1, &[9, b'x'])],
    ] {
        let mut w = wire(&frames);
        assert!(post(&mut w).is_err(), "{frames:?}");
    }
    // A frame larger than agreed is not read.
    let mut big = vec![0x01, 0x00, 0x00, 0x0, 0x1, 0, 0, 0, 1];
    big.extend(vec![0u8; 70_000]);
    let mut w = Wire { answer: Cursor::new(big), written: Vec::new() };
    assert!(post(&mut w).is_err());
}

fn p8(pkcs8: &[u8]) -> String {
    let b64 = STANDARD.encode(pkcs8);
    let lines: Vec<String> = b64.as_bytes().chunks(64).map(|c| String::from_utf8_lossy(c).into_owned()).collect();
    format!("-----BEGIN PRIVATE KEY-----\n{}\n-----END PRIVATE KEY-----\n", lines.join("\n"))
}

#[test]
fn the_provider_token_is_signed_by_the_key() {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let pair = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, pkcs8.as_ref(), &rng).unwrap();
    let key = Key::from_p8(&p8(pkcs8.as_ref()), "abc123defg", "TEAM123456").unwrap();
    let token = crate::apns::testing::token(&key, 1_700_000_000);
    let parts: Vec<&str> = token.split('.').collect();
    assert_eq!(parts.len(), 3);
    let header: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[0]).unwrap()).unwrap();
    let claims: serde_json::Value = serde_json::from_slice(&URL_SAFE_NO_PAD.decode(parts[1]).unwrap()).unwrap();
    assert_eq!(header, serde_json::json!({"alg": "ES256", "kid": "ABC123DEFG"}));
    assert_eq!(claims, serde_json::json!({"iss": "TEAM123456", "iat": 1_700_000_000}));
    let signature = URL_SAFE_NO_PAD.decode(parts[2]).unwrap();
    UnparsedPublicKey::new(&ECDSA_P256_SHA256_FIXED, pair.public_key().as_ref())
        .verify(format!("{}.{}", parts[0], parts[1]).as_bytes(), &signature)
        .expect("ES256, as Apple checks it");
    assert!(!format!("{key:?}").contains("pkcs8") || !format!("{key:?}").contains(&STANDARD.encode(pkcs8.as_ref())), "the key isn't shown");
}

#[test]
fn keys_and_ids_that_are_no_good() {
    let rng = SystemRandom::new();
    let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_FIXED_SIGNING, &rng).unwrap();
    let text = p8(pkcs8.as_ref());
    assert!(Key::from_p8(&text, "short", "TEAM123456").is_err());
    assert!(Key::from_p8(&text, "ABC123DEFG", "team-12345").is_err());
    assert!(Key::from_p8("hello", "ABC123DEFG", "TEAM123456").is_err());
    assert!(Key::from_p8("-----BEGIN PRIVATE KEY-----\nAAAA\n-----END PRIVATE KEY-----", "ABC123DEFG", "TEAM123456").is_err());
    assert!(is_device_token(&"ab".repeat(32)));
    for bad in ["", "xyz", &"a".repeat(33), &"0".repeat(202), "abcd/../ef01abcd/../ef01abcd/../ef01"] {
        assert!(!is_device_token(bad), "{bad}");
    }
}

#[test]
fn what_apple_said() {
    use crate::apns::testing::verdict;
    assert_eq!(verdict(Some(200), b""), Ok(()));
    assert!(matches!(verdict(Some(400), br#"{"reason":"BadDeviceToken"}"#), Err(PushError::Gone(_))));
    assert!(matches!(verdict(Some(410), br#"{"reason":"Unregistered","timestamp":1}"#), Err(PushError::Gone(_))));
    assert!(matches!(verdict(Some(403), br#"{"reason":"InvalidProviderToken"}"#), Err(PushError::Failed(_))));
    assert!(matches!(verdict(None, b""), Err(PushError::Failed(_))));
    assert!(matches!(verdict(Some(500), b"junk"), Err(PushError::Failed(_))));
}

#[test]
fn the_payload_says_nothing_of_the_messages() {
    let payload: serde_json::Value = serde_json::from_str(&new_mail_payload("ada@example.org", 3, "3 new messages")).unwrap();
    assert_eq!(payload["aps"]["alert"]["title"], "ada@example.org");
    assert_eq!(payload["aps"]["alert"]["body"], "3 new messages");
    assert_eq!(payload["count"], 3);
    assert_eq!(payload.as_object().unwrap().len(), 2, "aps and the count, nothing more");
}

fn inbox_world() -> ImapWorld {
    let message = |uid: u32, flags: &str| FakeMessage {
        uid,
        flags: flags.into(),
        internal_date: " 1-Jul-2003 09:52:37 +0000".into(),
        body: b"Subject: hi\r\n\r\nhi\r\n".to_vec(),
        claimed_size: None,
    };
    ImapWorld {
        user: "ada".into(),
        password: "hunter2".into(),
        mailboxes: vec![FakeMailbox {
            raw: "INBOX".into(),
            flags: "\\HasNoChildren".into(),
            uid_validity: 11,
            messages: vec![message(1, "\\Seen"), message(4, ""), message(7, "\\Flagged")],
            refuse_open: false,
            next_uid: 0,
        }],
        ..Default::default()
    }
}

#[test]
fn the_inbox_is_watched_read_only_and_new_unread_mail_found() {
    let imap = FakeImap::imap(inbox_world());
    let account = Account {
        email: "ada@example.org".into(),
        kind: AccountKind::Imap,
        incoming: Some(Server { host: "127.0.0.1".into(), port: imap.port, security: Security::None }),
        username: "ada".into(),
        ..Default::default()
    };
    let mut inbox = crate::watch::open(&account, "hunter2").unwrap();
    assert_eq!((inbox.uid_validity(), inbox.uid_next()), (11, 8));
    assert_eq!(inbox.unread_since(1).unwrap(), (vec![4, 7], 8));
    assert_eq!(inbox.unread_since(5).unwrap(), (vec![7], 8));
    // Nothing newer: `8:*` names message 7 all the same, which is not counted.
    assert_eq!(inbox.unread_since(8).unwrap(), (vec![], 8));
    inbox.close();
    assert!(imap.world().sent("EXAMINE") >= 1, "opened to be looked at only");
    assert_eq!(imap.world().sent("SELECT"), 0);
    assert_eq!(imap.world().sent("STORE"), 0, "nothing marked");
}
