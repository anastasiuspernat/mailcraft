// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! In the window: a sender's emoji on their rows, the junk tag and bar of junk learned from the
//! owner with Not Junk, and an NSFW sender's pictures blurred until Unblur.

use egui_kittest::kittest::Queryable;
use mailcraft_model::{Address, Attachment, Body, FolderKind, Id, Message};
use serde_json::json;

use crate::nav::tests::window;

type Harness = egui_kittest::Harness<'static, crate::MailApp>;

/// The window on the sample's Inbox with a message from `from` (with a picture in it) selected.
fn with_message(from: &str) -> (Harness, Id) {
    let mut harness = window();
    let app = harness.state_mut();
    let inbox = app.session.mailbox.folder_of_kind(app.session.mailbox.default_account().unwrap().id, FolderKind::Inbox).unwrap();
    let m = Message {
        folder: inbox,
        from: Address::new("Pat Lee", from),
        subject: "Photos from Saturday".into(),
        body: Body { text: "Here they are.".into(), html: Some("<p>Here they are.</p><img src=\"cid:p1\">".into()) },
        attachments: vec![Attachment {
            name: "p1.png".into(),
            mime: "image/png".into(),
            content_id: "p1".into(),
            inline: true,
            ..Default::default()
        }],
        received: 1_791_993_000,
        date: 1_791_993_000,
        ..Default::default()
    };
    let id = app.session.mailbox.add_message(m).unwrap();
    app.execute("folder.select", json!({"id": inbox})).unwrap();
    app.execute("mail.select", json!({"id": id})).unwrap();
    harness.run_steps(4);
    (harness, id)
}

fn hit(harness: &Harness, kind: &str, id: Id) -> bool {
    harness.state().hit.rect(kind, id).is_some()
}

#[test]
fn a_senders_emoji_is_on_their_rows() {
    let (mut harness, id) = with_message("pat@photos.example");
    assert!(!hit(&harness, "messageEmoji", id));
    harness.state_mut().execute("people.setEmoji", json!({"ids": [id], "emoji": "📷"})).unwrap();
    harness.run_steps(3);
    assert!(hit(&harness, "messageEmoji", id));
    harness.state_mut().execute("people.setEmoji", json!({"ids": [id], "emoji": null})).unwrap();
    harness.run_steps(3);
    assert!(!hit(&harness, "messageEmoji", id));
}

#[test]
fn junk_learned_shows_a_tag_and_a_bar_whose_not_junk_forgets_it() {
    let (mut harness, id) = with_message("pat@photos.example");
    assert!(!hit(&harness, "messageLearnedJunk", id) && !hit(&harness, "learnedJunkBar", 0));
    harness.state_mut().execute("people.setJunk", json!({"ids": [id], "junk": true})).unwrap();
    harness.run_steps(3);
    assert!(hit(&harness, "messageLearnedJunk", id));
    assert!(hit(&harness, "learnedJunkBar", 0));
    // Still in the Inbox: marked, not moved.
    let m = harness.state().session.mailbox.message(id).cloned().unwrap();
    assert!(!harness.state().session.mailbox.in_junk(m.folder));
    harness.get_by_label("Not Junk").click();
    harness.run_steps(3);
    assert!(!harness.state().session.mailbox.is_learned_junk("pat@photos.example"));
    assert!(!hit(&harness, "learnedJunkBar", 0));
}

#[test]
fn an_nsfw_senders_pictures_are_blurred_until_unblur() {
    let (mut harness, id) = with_message("pat@photos.example");
    let m = harness.state().session.mailbox.message(id).cloned().unwrap();
    assert!(!crate::reading::blurred(harness.state(), &m));
    harness.state_mut().execute("people.setNsfw", json!({"ids": [id], "nsfw": true})).unwrap();
    harness.run_steps(3);
    assert!(crate::reading::blurred(harness.state(), &m));
    assert!(hit(&harness, "nsfwBar", 0));
    harness.get_by_label("Unblur").click();
    harness.run_steps(3);
    assert!(!crate::reading::blurred(harness.state(), &m), "unblurred for this message");
    // The bar stays, with Blur, which puts the blur back.
    assert!(hit(&harness, "nsfwBar", 0));
    harness.get_by_label("Blur").click();
    harness.run_steps(3);
    assert!(crate::reading::blurred(harness.state(), &m));
    assert!(harness.state().session.mailbox.is_nsfw("pat@photos.example"), "the sender stays marked");
}
