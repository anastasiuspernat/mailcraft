// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the owner teaches MailCraft about senders besides VIPs ([`mailcraft_model::sender_marks`]):
//! junk learned from their marks, senders whose pictures are blurred, and an emoji for a sender.
//! Each is by the sender's address, kept on this computer only.

use serde_json::{Value, json};

use super::all::senders;
use super::{bad, fail};
use crate::{CmdResult, CommandSpec, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new("people.setJunk", "Sender's Mail Is Junk", "Message menu › Junk", set_junk).params(
            r#"{"ids"?: [message ids] (default: the selection) | "address"?: string, "junk": bool} — the senders' later mail is marked as junk (not moved), as it is once the owner has moved 3 of a sender's messages to Junk E-mail; false forgets them, as Not Junk does. Answers {"junk", "addresses", "changed"}"#,
        ),
        CommandSpec::new("people.setNsfw", "NSFW Sender", "Message menu › NSFW", set_nsfw).params(
            r#"{"ids"?: [message ids] (default: the selection) | "address"?: string, "nsfw": bool} — the pictures in those senders' mail are drawn blurred, with Unblur above the message. Answers {"nsfw", "addresses", "changed"}"#,
        ),
        CommandSpec::new("people.setEmoji", "Sender Emoji", "Message menu › Sender Emoji", set_emoji).params(
            r#"{"ids"?: [message ids] (default: the selection) | "address"?: string, "emoji": string | null} — an emoji shown on each of the senders' rows in the message list (one short run of emoji or symbols, no letters or digits); null takes it away. Answers {"emoji", "addresses", "changed"}"#,
        ),
    ]
}

/// More senders than are kept refuse the whole command.
fn room(kept: usize, adding: usize) -> Result<(), crate::CmdError> {
    if kept.saturating_add(adding) > mailcraft_model::sender_marks::MAX_SENDERS {
        return Err(fail("at most 10,000 senders can be marked so"));
    }
    Ok(())
}

fn set_junk(s: &mut Session, v: &Value) -> CmdResult {
    let junk = p::bool(v, "junk").ok_or_else(|| bad("`junk` is true or false"))?;
    let addresses = senders(s, v)?;
    if junk {
        room(s.mailbox.junk_marks.len(), addresses.len())?;
    }
    let mut changed = false;
    for a in &addresses {
        changed |= s.mailbox.set_learned_junk(a, junk);
    }
    Ok(json!({"junk": junk, "addresses": addresses, "changed": changed}))
}

fn set_nsfw(s: &mut Session, v: &Value) -> CmdResult {
    let nsfw = p::bool(v, "nsfw").ok_or_else(|| bad("`nsfw` is true or false"))?;
    let addresses = senders(s, v)?;
    if nsfw {
        room(s.mailbox.nsfw_senders.len(), addresses.len())?;
    }
    let mut changed = false;
    for a in &addresses {
        changed |= s.mailbox.set_nsfw(a, nsfw);
    }
    Ok(json!({"nsfw": nsfw, "addresses": addresses, "changed": changed}))
}

fn set_emoji(s: &mut Session, v: &Value) -> CmdResult {
    let emoji = match v.get("emoji") {
        Some(Value::Null) => None,
        Some(Value::String(e)) => {
            Some(mailcraft_model::sender_marks::emoji(e).ok_or_else(|| bad("`emoji` is an emoji (no letters or digits), or null"))?)
        }
        _ => return Err(bad("`emoji` is an emoji, or null")),
    };
    let addresses = senders(s, v)?;
    if emoji.is_some() {
        room(s.mailbox.sender_emoji.len(), addresses.len())?;
    }
    let mut changed = false;
    for a in &addresses {
        changed |= s.mailbox.set_sender_emoji(a, emoji.as_deref());
    }
    Ok(json!({"emoji": emoji, "addresses": addresses, "changed": changed}))
}

#[cfg(test)]
#[path = "senders_tests.rs"]
mod senders_tests;
