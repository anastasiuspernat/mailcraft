// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use std::io::{BufRead, BufReader, Write};

use crate::pair::{Heard, hear, new_code, serve};

fn token() -> String {
    "0a".repeat(32)
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
fn what_an_ipad_may_say() {
    let line = format!(r#"{{"code":"123456","token":"{}","name":"Ada's iPad\u0007"}}"#, token().to_uppercase());
    match hear(&line, "123456") {
        Heard::Paired(d) => assert_eq!((d.token, d.name), (token(), "Ada's iPad".to_string())),
        other => panic!("{other:?}"),
    }
    assert_eq!(hear(&line, "654321"), Heard::Wrong);
    assert_eq!(hear(r#"{"code":"123456","token":"nothex"}"#, "123456"), Heard::Junk);
    assert_eq!(hear("GET / HTTP/1.1", "123456"), Heard::Junk);
    match hear(&format!(r#"{{"code":" 123456 ","token":"{}"}}"#, token()), "123456") {
        Heard::Paired(d) => assert_eq!(d.name, "iPad"),
        other => panic!("{other:?}"),
    }
}

fn say(port: u16, line: &str) -> String {
    let mut s = std::net::TcpStream::connect(("127.0.0.1", port)).unwrap();
    s.write_all(format!("{line}\n").as_bytes()).unwrap();
    let mut answer = String::new();
    BufReader::new(s).read_line(&mut answer).unwrap();
    answer
}

#[test]
fn pairing_over_the_network() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || serve(&listener, "111222"));
    assert!(say(port, &format!(r#"{{"code":"000000","token":"{}"}}"#, token())).contains("\"ok\":false"));
    let answer = say(port, &format!(r#"{{"code":"111222","token":"{}","name":"iPad"}}"#, token()));
    assert!(answer.contains("\"ok\":true"), "{answer}");
    assert_eq!(server.join().unwrap().unwrap().token, token());
}

#[test]
fn five_wrong_codes_end_it() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let server = std::thread::spawn(move || serve(&listener, "111222"));
    for _ in 0..5 {
        say(port, &format!(r#"{{"code":"999999","token":"{}"}}"#, token()));
    }
    assert!(server.join().unwrap().is_err());
}

#[test]
fn the_kept_code_is_made_once_and_renewed_on_request() {
    let dir = std::env::temp_dir().join(format!("mailcraft-relay-code-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path = crate::config::file_in(&dir);
    let first = crate::config::pairing_code(&path, false).unwrap();
    assert_eq!(crate::config::pairing_code(&path, false).unwrap(), first, "kept");
    let mut renewed = crate::config::pairing_code(&path, true).unwrap();
    // (One in a million: the same digits again.)
    if renewed == first {
        renewed = crate::config::pairing_code(&path, true).unwrap();
    }
    assert_ne!(renewed, first);
    assert_eq!(crate::config::pairing_code(&path, false).unwrap(), renewed);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn listening_for_good_pairs_and_blocks_guessing() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || crate::pair::serve_forever(&listener, || Some("424242".to_string()), move |d| tx.send(d).unwrap()));
    for _ in 0..5 {
        assert!(say(port, &format!(r#"{{"code":"111111","token":"{}"}}"#, token())).contains("wrong code"));
    }
    // Blocked now, even with the right code.
    assert!(say(port, &format!(r#"{{"code":"424242","token":"{}"}}"#, token())).contains("too many tries"));
    assert!(rx.try_recv().is_err());
}

#[test]
fn listening_for_good_pairs() {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || crate::pair::serve_forever(&listener, || Some("424242".to_string()), move |d| tx.send(d).unwrap()));
    assert!(say(port, &format!(r#"{{"code":"424242","token":"{}"}}"#, token())).contains("\"ok\":true"));
    assert_eq!(rx.recv_timeout(std::time::Duration::from_secs(5)).unwrap().token, token());
}
