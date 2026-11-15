// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! MailCraft's notification extension on iPad (`MailCraftNotify.appex`, inside MailCraft.app):
//! iPadOS runs it for each notification from the owner's relay, which carries its text (the
//! account, the newest message's sender and subject) sealed with the note key this iPad gave the
//! relay when it paired (`mailcraft_net::push_link`). It opens that with the key from the keychain
//! and hands back what to show; Apple only ever carried "New mail". It reads nothing else, keeps
//! nothing, and sends nothing anywhere.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "ios")]
#[path = "../../mailcraft-ios/src/push_keys.rs"]
#[allow(dead_code)]
mod push_keys;

#[cfg(target_os = "ios")]
fn main() {
    std::process::exit(mailcraft_apple_bridge::notification_service::run(open));
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("mailcraft-ios-notify is MailCraft's notification extension on iPad");
}

/// What a notification shows, from its sealed text.
#[cfg(target_os = "ios")]
fn open(sealed: &str) -> Option<mailcraft_apple_bridge::notification_service::Shown> {
    let key = push_keys::get("note")?;
    let note = mailcraft_net::push_link::open_note(&key, sealed)?;
    Some(shown(&note))
}

/// The words of a notification: who it is from, over the subject, with the account under the
/// sender when there is room; "3 new messages" when more came at once.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn shown(note: &Note) -> Shown {
    let tidy = |text: &str, most: usize| text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(most).collect::<String>().trim().to_string();
    let (account, from, subject) = (tidy(&note.account, 120), tidy(&note.from, 80), tidy(&note.subject, 160));
    let title = if from.is_empty() { account.clone() } else { from };
    let subtitle = if title == account { String::new() } else { account };
    let subject = if subject.is_empty() { "(no subject)".to_string() } else { subject };
    let body = if note.count > 1 { format!("{subject}\n{} new messages", note.count) } else { subject };
    Shown { title, subtitle, body }
}

#[cfg(target_os = "ios")]
use mailcraft_apple_bridge::notification_service::Shown;
#[cfg(target_os = "ios")]
use mailcraft_net::push_link::Note;

#[cfg(not(target_os = "ios"))]
#[allow(dead_code)]
struct Note {
    account: String,
    count: usize,
    from: String,
    subject: String,
}

#[cfg(not(target_os = "ios"))]
#[allow(dead_code)]
#[derive(Debug, PartialEq, Eq)]
struct Shown {
    title: String,
    subtitle: String,
    body: String,
}

#[cfg(test)]
mod main_tests;
