// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Senders the owner teaches: junk learned at the third mark and forgotten by Not Junk, NSFW
//! senders, an emoji for a sender, and a VIP's unread mail always in Priority.

use mailcraft_model::{Address, FolderKind, Id, Message};
use serde_json::json;

use crate::Session;

const NOW: i64 = 1_800_000_000;

fn setup() -> (Session, Id) {
    let mut s = Session::new(mailcraft_model::Mailbox::with_local_account("Owner", "owner@example.org"));
    s.clock.now = Some(NOW);
    let inbox = s.mailbox.folder_of_kind(s.mailbox.accounts[0].id, FolderKind::Inbox).unwrap();
    s.run("folder.select", &json!({"id": inbox})).unwrap();
    (s, inbox)
}

fn add(s: &mut Session, folder: Id, from: &str, subject: &str, received: i64) -> Id {
    let m = Message { folder, from: Address::new("", from), subject: subject.into(), received, date: received, ..Default::default() };
    s.mailbox.add_message(m).unwrap()
}

#[test]
fn the_third_message_moved_to_junk_teaches_that_the_sender_is_junk() {
    let (mut s, inbox) = setup();
    let ids: Vec<Id> = (0..4).map(|i| add(&mut s, inbox, "Deals@Spam.example", &format!("Deal {i}"), NOW - 100 * i)).collect();
    let other = add(&mut s, inbox, "friend@ok.example", "Hi", NOW - 50);
    for (i, id) in ids.iter().take(3).enumerate() {
        let r = s.run("mail.junk", &json!({"ids": [id]})).unwrap();
        let learned = r["learnedJunk"].as_array().unwrap();
        assert_eq!(learned.len(), usize::from(i == 2), "learned at the third: {r}");
    }
    assert!(s.mailbox.is_learned_junk("deals@spam.example"));
    // Their later mail stays in the Inbox, marked.
    let later = add(&mut s, inbox, "deals@spam.example", "Another deal", NOW);
    assert_eq!(s.mailbox.message(later).map(|m| m.folder), Some(inbox));
    assert!(!s.mailbox.is_learned_junk("friend@ok.example"));
    // Moving mail already in Junk E-mail there again counts nothing.
    let junk = s.mailbox.folder_of_kind(s.mailbox.accounts[0].id, FolderKind::Junk).unwrap();
    assert_eq!(s.mailbox.message(ids[0]).map(|m| m.folder), Some(junk));
    // Not Junk forgets the sender.
    s.run("mail.notJunk", &json!({"ids": [later]})).unwrap();
    assert!(!s.mailbox.is_learned_junk("deals@spam.example"));
    // And it can be said directly, and undone.
    s.run("people.setJunk", &json!({"ids": [other], "junk": true})).unwrap();
    assert!(s.mailbox.is_learned_junk("friend@ok.example"));
    s.run("edit.undo", &json!({})).unwrap();
    assert!(!s.mailbox.is_learned_junk("friend@ok.example"));
    let _ = ids[3];
}

#[test]
fn nsfw_and_emoji_are_kept_by_sender_and_checked() {
    let (mut s, inbox) = setup();
    let id = add(&mut s, inbox, "Pics@Site.example", "Photos", NOW);
    let r = s.run("people.setNsfw", &json!({"ids": [id], "nsfw": true})).unwrap();
    assert_eq!(r["addresses"], json!(["pics@site.example"]));
    assert!(s.mailbox.is_nsfw("pics@site.example"));
    s.run("people.setNsfw", &json!({"address": "pics@site.example", "nsfw": false})).unwrap();
    assert!(!s.mailbox.is_nsfw("pics@site.example"));
    s.run("people.setEmoji", &json!({"ids": [id], "emoji": "📷"})).unwrap();
    assert_eq!(s.mailbox.sender_emoji_of("pics@site.example"), Some("📷"));
    for bad in [json!("abc"), json!(5), json!("<b>"), json!("")] {
        assert!(s.run("people.setEmoji", &json!({"ids": [id], "emoji": bad})).is_err(), "{bad}");
    }
    assert!(s.run("people.setEmoji", &json!({"ids": [id]})).is_err(), "emoji or null is required");
    s.run("people.setEmoji", &json!({"ids": [id], "emoji": null})).unwrap();
    assert_eq!(s.mailbox.sender_emoji_of("pics@site.example"), None);
    assert!(s.run("people.setNsfw", &json!({"address": "not an address", "nsfw": true})).is_err());
}

#[test]
fn a_vips_unread_mail_is_always_in_priority_until_the_owner_says_otherwise() {
    let (mut s, inbox) = setup();
    s.run("ai.options", &json!({"modelPath": "/models/test.gguf", "enabled": true})).unwrap();
    // A week old: past any time a priority lasts.
    let id = add(&mut s, inbox, "priya@lumen.example", "Roadmap", NOW - 7 * 86_400);
    s.run("people.setVip", &json!({"address": "priya@lumen.example", "vip": true})).unwrap();
    // Sorted by the rule a VIP is (no model needed).
    s.run("ai.request", &json!({"max": 8})).unwrap();
    let m = s.mailbox.message(id).cloned().unwrap();
    assert!(crate::ai::is_priority(&s, &m));
    assert!(s.rows().first().is_some_and(|r| r.id == id && r.priority));
    // Read, it leaves.
    s.run("mail.markRead", &json!({"ids": [id]})).unwrap();
    let m = s.mailbox.message(id).cloned().unwrap();
    assert!(!crate::ai::is_priority(&s, &m));
    s.run("mail.markUnread", &json!({"ids": [id]})).unwrap();
    // Not a Priority wins over VIP.
    s.run("ai.setPriority", &json!({"ids": [id], "priority": false})).unwrap();
    let m = s.mailbox.message(id).cloned().unwrap();
    assert!(!crate::ai::is_priority(&s, &m));
}
