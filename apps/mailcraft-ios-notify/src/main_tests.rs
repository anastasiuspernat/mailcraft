// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a notification says, from what the relay sealed.

use super::*;

fn note(count: usize, from: &str, subject: &str) -> Note {
    Note { account: "ada@example.org".into(), count, from: from.into(), subject: subject.into() }
}

#[test]
fn the_sender_over_the_subject_with_the_account_under() {
    let s = shown(&note(1, "Maya Okafor", "Mockups, round three"));
    assert_eq!((s.title.as_str(), s.subtitle.as_str(), s.body.as_str()), ("Maya Okafor", "ada@example.org", "Mockups, round three"));
}

#[test]
fn several_at_once_and_what_is_missing_or_hostile() {
    let s = shown(&note(3, "", "Report\u{7}\r\nBcc: x"));
    assert_eq!((s.title.as_str(), s.subtitle.as_str()), ("ada@example.org", ""));
    assert_eq!(s.body, "Report   Bcc: x\n3 new messages");
    assert_eq!(shown(&note(1, "Ben", "")).body, "(no subject)");
    assert!(shown(&note(1, &"x".repeat(500), "s")).title.chars().count() <= 80);
}
