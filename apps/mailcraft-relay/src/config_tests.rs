// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use mailcraft_model::{OAuthProvider, Security};

use crate::config::{Config, Device, Seen, Watched, server_for, typed_server};
use crate::logfile::stamp;
use crate::watcher::new_mail_text;

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("mailcraft-relay-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

#[test]
fn servers_from_the_address_or_typed() {
    let (gmail, ms) = server_for("ada@gmail.com").unwrap();
    assert_eq!((gmail.host.as_str(), gmail.port, ms), ("imap.gmail.com", 993, false));
    let (outlook, ms) = server_for("ada@outlook.com").unwrap();
    assert!(ms, "signs in on Microsoft's page");
    assert_eq!(outlook.security, Security::Tls);
    assert_eq!(server_for("ada@lumen.example").unwrap().0.host, "imap.lumen.example");
    assert!(server_for("no-at-sign").is_none());
    assert_eq!(typed_server("Mail.Example.org:1993", false).unwrap().host, "mail.example.org");
    assert_eq!(typed_server("mail.example.org", true).unwrap().port, 143);
    for bad in ["", "mail example", "a/b:993", "x:notaport", "x:99999"] {
        assert!(typed_server(bad, false).is_none(), "{bad}");
    }
}

#[test]
fn a_microsoft_account_signs_in_with_the_builds_id() {
    let w = Watched { email: "ada@outlook.com".into(), microsoft: true, ..Default::default() };
    let a = w.account("client-1");
    assert_eq!(a.oauth.as_ref().map(|o| (o.provider, o.client_id.as_str())), Some((OAuthProvider::Microsoft, "client-1")));
    assert_eq!(w.secret_key(), "signin:ada@outlook.com");
    let p = Watched { email: "Ada@Example.org".into(), ..Default::default() };
    assert!(p.account("client-1").oauth.is_none());
    assert_eq!(p.secret_key(), "password:ada@example.org");
}

#[test]
fn settings_round_trip_and_hold_no_secret() {
    let dir = temp("round");
    let path = crate::config::file_in(&dir);
    assert_eq!(Config::load(&path).unwrap(), Config::default(), "none yet");
    let mut config = Config::default();
    config.accounts.push(Watched { email: "ada@example.org".into(), server: typed_server("imap.example.org", false).unwrap(), ..Default::default() });
    assert!(config.pair(Device { token: "ab".repeat(32), name: "iPad".into() }));
    assert!(config.pair(Device { token: "AB".repeat(32), name: "Ada's iPad".into() }), "the same iPad, renamed");
    assert_eq!(config.devices.len(), 1);
    config.seen.insert("ada@example.org".into(), Seen { uid_validity: 11, next: 8 });
    config.save(&path).unwrap();
    assert_eq!(Config::load(&path).unwrap(), config);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(!text.to_lowercase().contains("password\""), "{text}");
    std::fs::write(&path, "{ not json").unwrap();
    assert!(Config::load(&path).is_err(), "a damaged file is an error, never an empty start");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn bad_devices_and_too_many_are_left_out() {
    let mut config = Config::default();
    for i in 0..15 {
        let _ = config.pair(Device { token: format!("{i:02}").repeat(32), name: String::new() });
    }
    assert_eq!(config.devices.len(), crate::config::MAX_DEVICES);
    let dir = temp("bad");
    let path = crate::config::file_in(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(&path, r#"{"devices":[{"token":"../../etc"},{"token":"abcdabcdabcdabcdabcdabcdabcdabcd"}]}"#).unwrap();
    assert_eq!(Config::load(&path).unwrap().devices.len(), 1);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn words_and_dates() {
    assert_eq!(new_mail_text(1), "1 new message");
    assert_eq!(new_mail_text(4), "4 new messages");
    assert_eq!(stamp(0), "1970-01-01 00:00:00");
    assert_eq!(stamp(1_763_100_000), "2025-11-14 06:00:00");
    assert_eq!(stamp(951_782_400), "2000-02-29 00:00:00");
}
