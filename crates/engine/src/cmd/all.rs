// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mailboxes made of every account's mail ([`crate::all`]): showing All Inboxes, VIPs, Flagged,
//! All Drafts and All Sent, marking senders as VIPs, and whether the folder pane lists them.

use mailcraft_model::insight::{CategorySource, InboxCategory};
use serde_json::{Value, json};

use super::{bad, fail, targets};
use crate::all::{self, AllMailbox};
use crate::{CmdResult, CommandSpec, Module, Session, p};

pub fn specs() -> Vec<CommandSpec> {
    const SHOWS: &str = r#"{} — answers {"mailbox", "folders": [ids], "messages", "unread"}"#;
    vec![
        CommandSpec::new("view.allInboxes", "All Inboxes", "Folder pane › All Inboxes", |s, _| show(s, AllMailbox::Inboxes)).reads().params(SHOWS),
        CommandSpec::new("view.vips", "VIPs", "Folder pane › VIPs", |s, _| show(s, AllMailbox::Vips)).reads().params(SHOWS),
        CommandSpec::new("view.allFlagged", "Flagged", "Folder pane › Flagged", |s, _| show(s, AllMailbox::Flagged)).reads().params(SHOWS),
        CommandSpec::new("view.allDrafts", "All Drafts", "Folder pane › All Drafts", |s, _| show(s, AllMailbox::Drafts)).reads().params(SHOWS),
        CommandSpec::new("view.allSent", "All Sent", "Folder pane › All Sent", |s, _| show(s, AllMailbox::Sent)).reads().params(SHOWS),
        CommandSpec::new("view.showAllMailboxes", "Show All Inboxes, VIPs and Flagged", "File › Options › Mail", show_all)
            .reads()
            .params(r#"{"value"?: bool} (default: toggle) — All Inboxes, VIPs, Flagged, All Drafts and All Sent at the top of the folder pane"#),
        CommandSpec::new("people.setVip", "VIP", "Home › VIP", set_vip).params(
            r#"{"ids"?: [message ids] (default: the selection) | "address"?: string, "vip": bool} — the senders of those messages, or that address, become VIPs or stop being VIPs. Their mail is listed under VIPs and kept in Primary. Kept on this computer only. Answers {"vip", "addresses", "changed"}"#,
        ),
    ]
}

/// Show `which` in the list, over its anchor folder ([`all::anchor`]).
fn show(s: &mut Session, which: AllMailbox) -> CmdResult {
    let folder = all::anchor(&s.mailbox, which).ok_or_else(|| fail("there is no account with that folder"))?;
    if all::in_view(s) != Some(which) {
        s.sel.messages.clear();
    }
    s.view.folder = folder;
    s.view.module = Module::Mail;
    s.view.gmail = None;
    s.view.all = Some((which, folder));
    // A search belongs to the folder it was typed in.
    s.view.search.clear();
    let counts = s.folder_counts();
    let held = all::held(&s.mailbox, &counts, which);
    let mut folders: Vec<_> = all::folders(&s.mailbox, which).into_iter().collect();
    folders.sort_unstable();
    Ok(json!({"mailbox": which.code(), "folders": folders, "messages": held.total, "unread": held.unread}))
}

fn show_all(s: &mut Session, v: &Value) -> CmdResult {
    s.prefs.all_mailboxes = p::bool(v, "value").unwrap_or(!s.prefs.all_mailboxes);
    Ok(json!({"value": s.prefs.all_mailboxes}))
}

fn set_vip(s: &mut Session, v: &Value) -> CmdResult {
    let vip = p::bool(v, "vip").ok_or_else(|| bad("`vip` is true or false"))?;
    let addresses = senders(s, v)?;
    if vip && s.mailbox.vips.len().saturating_add(addresses.len()) > mailcraft_model::MAX_VIPS {
        return Err(fail("at most 10,000 senders can be VIPs"));
    }
    let mut changed = false;
    for a in &addresses {
        changed |= s.mailbox.set_vip(a, vip);
    }
    // Their Inbox mail already sorted goes to Primary, unless the owner placed it.
    if vip && changed {
        let theirs: Vec<_> = s
            .mailbox
            .messages
            .values()
            .filter(|m| crate::ai::sorts(s, m) && s.mailbox.is_vip(&m.from.email))
            .filter(|m| m.insight.as_deref().is_some_and(|i| i.category.is_some() && !i.owner_said()))
            .map(|m| m.id)
            .collect();
        for id in theirs {
            crate::ai::set_category(s, id, InboxCategory::Primary, CategorySource::Rule);
        }
    }
    Ok(json!({"vip": vip, "addresses": addresses, "changed": changed}))
}

/// The senders a command about senders is for: `address`, or the senders of the messages `ids`
/// (default: the selection), in lower case, each once.
pub(crate) fn senders(s: &Session, v: &Value) -> Result<Vec<String>, crate::CmdError> {
    let mut addresses: Vec<String> = match p::str(v, "address") {
        Some(a) => {
            let a = a.trim().to_lowercase();
            if a.is_empty() || a.len() > 320 || a.chars().any(char::is_control) || !a.contains('@') {
                return Err(bad("`address` is an email address"));
            }
            vec![a]
        }
        None => targets(s, v)?.into_iter().filter_map(|id| s.mailbox.message(id)).map(|m| m.from.email.trim().to_lowercase()).collect(),
    };
    addresses.retain(|a| !a.is_empty());
    addresses.sort();
    addresses.dedup();
    if addresses.is_empty() {
        return Err(fail("those messages have no sender's address"));
    }
    Ok(addresses)
}

#[cfg(test)]
#[path = "all_tests.rs"]
mod all_tests;
