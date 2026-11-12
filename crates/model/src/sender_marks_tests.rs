// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use super::*;

#[test]
fn junk_is_learned_at_the_third_mark_and_forgotten_by_not_junk() {
    let mut mb = Mailbox::with_local_account("Owner", "owner@example.org");
    assert!(!mb.count_junk("Spam@Bad.example"));
    assert!(!mb.count_junk("spam@bad.example "));
    assert!(!mb.is_learned_junk("spam@bad.example"));
    assert!(mb.count_junk("SPAM@bad.example"));
    assert!(mb.is_learned_junk("Spam@Bad.Example"));
    assert!(!mb.count_junk("spam@bad.example"), "learned once");
    assert!(mb.set_learned_junk("spam@bad.example", false));
    assert!(!mb.is_learned_junk("spam@bad.example"));
    assert!(mb.set_learned_junk("other@bad.example", true));
    assert!(mb.is_learned_junk("other@bad.example"));
    assert!(!mb.count_junk(""));
}

#[test]
fn nsfw_senders_are_kept_by_address() {
    let mut mb = Mailbox::with_local_account("Owner", "owner@example.org");
    assert!(mb.set_nsfw("Pics@Site.example", true));
    assert!(!mb.set_nsfw("pics@site.example", true));
    assert!(mb.is_nsfw("PICS@site.example"));
    assert!(mb.set_nsfw("pics@site.example", false));
    assert!(!mb.is_nsfw("pics@site.example"));
}

#[test]
fn an_emoji_is_an_emoji_and_nothing_else() {
    assert_eq!(emoji(" 🎸 ").as_deref(), Some("🎸"));
    assert_eq!(emoji("👨‍👩‍👧").as_deref(), Some("👨‍👩‍👧"));
    assert_eq!(emoji("★").as_deref(), Some("★"));
    for bad in ["", "  ", "a", "ok", "1", "<b>", "🎸\n🎸", "\u{202E}x", "🎸🎸🎸🎸🎸🎸🎸🎸🎸"] {
        assert_eq!(emoji(bad), None, "{bad:?}");
    }
    let mut mb = Mailbox::with_local_account("Owner", "owner@example.org");
    assert!(mb.set_sender_emoji("Rosa@Home.example", Some("🌹")));
    assert_eq!(mb.sender_emoji_of("rosa@home.example"), Some("🌹"));
    assert!(!mb.set_sender_emoji("rosa@home.example", Some("rose")));
    assert!(mb.set_sender_emoji("rosa@home.example", None));
    assert_eq!(mb.sender_emoji_of("rosa@home.example"), None);
}

#[test]
fn marks_read_from_a_file_are_tidied() {
    let mut mb = Mailbox::with_local_account("Owner", "owner@example.org");
    mb.junk_marks.insert(" Spam@Bad.example".into(), 200);
    mb.junk_marks.insert("zero@bad.example".into(), 0);
    mb.nsfw_senders.insert("PICS@site.example".into());
    mb.sender_emoji.insert("rosa@home.example".into(), "not an emoji".into());
    mb.sender_emoji.insert("Marco@Band.example".into(), "🥁".into());
    assert!(mb.repair_sender_marks());
    assert_eq!(mb.junk_marks.get("spam@bad.example"), Some(&JUNK_LEARNED));
    assert!(!mb.junk_marks.contains_key("zero@bad.example"));
    assert!(mb.is_nsfw("pics@site.example"));
    assert_eq!(mb.sender_emoji_of("rosa@home.example"), None);
    assert_eq!(mb.sender_emoji_of("marco@band.example"), Some("🥁"));
    assert!(!mb.repair_sender_marks(), "tidy now");
    // And through a file: the marks come back as they were.
    let json = serde_json::to_string(&mb).unwrap();
    let back: Mailbox = serde_json::from_str(&json).unwrap();
    assert!(back.is_learned_junk("spam@bad.example") && back.is_nsfw("pics@site.example"));
    assert_eq!(back.sender_emoji_of("marco@band.example"), Some("🥁"));
}
