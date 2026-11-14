// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use std::time::Duration;

use super::*;
use crate::script::Script;

const LONG: Duration = Duration::from_secs(600);

#[test]
fn new_mail_ends_the_wait() {
    let mut s = Script::new("+ idling\r\n* OK Still here\r\n* 5 EXISTS\r\nA0001 OK IDLE terminated\r\n");
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.idle(LONG).unwrap(), Idled::Changed);
    assert_eq!(imap.selected.exists, 5, "what the server said is kept");
    assert_eq!(s.sent(), "A0001 IDLE\r\nDONE\r\n");
}

#[test]
fn every_kind_of_change_counts() {
    for news in ["* 3 EXPUNGE", "* 2 FETCH (FLAGS (\\Seen))", "* 1 RECENT", "* 9 exists"] {
        let mut s = Script::new(format!("+ idling\r\n{news}\r\nA0001 OK done\r\n"));
        assert_eq!(Imap::resume(&mut s).idle(LONG).unwrap(), Idled::Changed, "{news}");
    }
}

#[test]
fn the_longest_wait_ends_it_quietly() {
    let mut s = Script::new("+ idling\r\n* OK Still here\r\nA0001 OK IDLE terminated\r\n");
    let mut imap = Imap::resume(&mut s);
    assert_eq!(imap.idle(Duration::ZERO).unwrap(), Idled::Quiet);
    assert_eq!(s.sent(), "A0001 IDLE\r\nDONE\r\n");
}

#[test]
fn a_refusal_a_goodbye_or_silence_is_an_error() {
    let mut s = Script::new("A0001 BAD unknown command\r\n");
    assert!(matches!(Imap::resume(&mut s).idle(LONG), Err(ProtoError::Rejected(_))));
    let mut s = Script::new("+ idling\r\n* BYE shutting down\r\n");
    assert!(matches!(Imap::resume(&mut s).idle(LONG), Err(ProtoError::Io(_))));
    let mut s = Script::new("+ idling\r\n");
    assert!(matches!(Imap::resume(&mut s).idle(LONG), Err(ProtoError::Io(_))), "the stream ran dry");
    let mut s = Script::new("+ idling\r\nA0001 OK ended by itself\r\n");
    assert!(matches!(Imap::resume(&mut s).idle(LONG), Err(ProtoError::Protocol(_))));
    let mut s = Script::new("+ idling\r\n* 1 EXISTS\r\nA0001 NO not now\r\n");
    assert!(matches!(Imap::resume(&mut s).idle(LONG), Err(ProtoError::Rejected(_))));
}

#[test]
fn idle_offered_and_noop() {
    let mut s = Script::new("A0001 OK [CAPABILITY IMAP4rev1 IDLE] done\r\n* 7 EXISTS\r\nA0002 OK done\r\n");
    let mut imap = Imap::resume(&mut s);
    assert!(!imap.offers_idle(), "not before the server said");
    imap.capabilities().unwrap();
    assert!(imap.offers_idle());
    imap.noop().unwrap();
    assert_eq!(imap.selected.exists, 7);
    assert_eq!(s.sent(), "A0001 CAPABILITY\r\nA0002 NOOP\r\n");
}
