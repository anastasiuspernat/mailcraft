// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mailboxes made of every account's mail, at the top of the folder pane: All Inboxes, VIPs,
//! Flagged, All Drafts and All Sent.
//!
//! None of them is a folder and none copies a message: each is the list of some folders of every
//! account (each account's Inbox, Drafts or Sent Items; for VIPs and Flagged every folder but
//! Deleted Items, Junk E-mail and what is under them), and for VIPs and Flagged only the mail of
//! a VIP or with a flag. The list in view shows one while [`crate::ViewState::all`] names it and
//! the folder it was chosen over is still the folder in view: anything that shows another folder
//! leaves it.

use std::collections::HashSet;

use mailcraft_model::{FlagStatus, FolderKind, Id, Mailbox, Message};
use serde::{Deserialize, Serialize};

use crate::Session;
use crate::counts::{FolderCounts, Held};

/// One of the mailboxes made of every account's mail.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum AllMailbox {
    /// Every account's Inbox.
    Inboxes,
    /// The mail of the senders the owner marked as VIPs.
    Vips,
    /// Every flagged message.
    Flagged,
    /// Every account's Drafts.
    Drafts,
    /// Every account's Sent Items.
    Sent,
}

impl AllMailbox {
    pub const ALL: [AllMailbox; 5] = [AllMailbox::Inboxes, AllMailbox::Vips, AllMailbox::Flagged, AllMailbox::Drafts, AllMailbox::Sent];

    /// Its name in commands and in the view's settings.
    pub fn code(self) -> &'static str {
        match self {
            AllMailbox::Inboxes => "inboxes",
            AllMailbox::Vips => "vips",
            AllMailbox::Flagged => "flagged",
            AllMailbox::Drafts => "drafts",
            AllMailbox::Sent => "sent",
        }
    }

    pub fn parse(code: &str) -> Option<AllMailbox> {
        AllMailbox::ALL.into_iter().find(|a| a.code() == code)
    }

    /// Its name in English (the window's language has its own through the catalogue).
    pub fn name(self) -> &'static str {
        match self {
            AllMailbox::Inboxes => "All Inboxes",
            AllMailbox::Vips => "VIPs",
            AllMailbox::Flagged => "Flagged",
            AllMailbox::Drafts => "All Drafts",
            AllMailbox::Sent => "All Sent",
        }
    }

    /// The kind of folder each account gives it, for those that are one folder of each account.
    pub fn kind(self) -> Option<FolderKind> {
        match self {
            AllMailbox::Inboxes => Some(FolderKind::Inbox),
            AllMailbox::Drafts => Some(FolderKind::Drafts),
            AllMailbox::Sent => Some(FolderKind::Sent),
            AllMailbox::Vips | AllMailbox::Flagged => None,
        }
    }

    /// Whether its count beside it is of unread mail (All Inboxes, VIPs) rather than of all of it.
    pub fn counts_unread(self) -> bool {
        matches!(self, AllMailbox::Inboxes | AllMailbox::Vips)
    }
}

/// The mailbox made of every account's mail the list shows, if it shows one: the view names it,
/// and the folder it was chosen over is still the folder in view.
pub fn in_view(s: &Session) -> Option<AllMailbox> {
    let (which, over) = s.view.all?;
    (over == s.view.folder && s.view.module == crate::Module::Mail).then_some(which)
}

/// The folders `which` is made of: each account's folder of its kind, or for VIPs and Flagged
/// every folder but Deleted Items, Junk E-mail and what is under them.
pub fn folders(mb: &Mailbox, which: AllMailbox) -> HashSet<Id> {
    match which.kind() {
        Some(kind) => mb.folders.iter().filter(|f| f.kind == kind).map(|f| f.id).collect(),
        None => {
            let hidden: HashSet<Id> =
                mb.folders.iter().filter(|f| matches!(f.kind, FolderKind::Deleted | FolderKind::Junk)).flat_map(|f| mb.subtree(f.id)).collect();
            mb.folders.iter().map(|f| f.id).filter(|id| !hidden.contains(id)).collect()
        }
    }
}

/// Whether `m`, in one of the folders of `which`, is in it: for VIPs from a VIP, for Flagged
/// flagged; every message of the others.
pub fn keeps(mb: &Mailbox, which: AllMailbox, m: &Message) -> bool {
    match which {
        AllMailbox::Vips => mb.is_vip(&m.from.email),
        AllMailbox::Flagged => m.flag == FlagStatus::Flagged,
        AllMailbox::Inboxes | AllMailbox::Drafts | AllMailbox::Sent => true,
    }
}

/// The folder the list stands on while it shows `which`: the default account's folder of its kind
/// (its Inbox for VIPs and Flagged), else any account's. `None` when no account has one.
pub fn anchor(mb: &Mailbox, which: AllMailbox) -> Option<Id> {
    let kind = which.kind().unwrap_or(FolderKind::Inbox);
    mb.default_account()
        .and_then(|a| mb.folder_of_kind(a.id, kind))
        .or_else(|| mb.accounts_in_order().into_iter().find_map(|a| mb.folder_of_kind(a.id, kind)))
}

/// What `which` holds: its folders' messages (for VIPs and Flagged, theirs that it keeps).
pub fn held(mb: &Mailbox, counts: &FolderCounts, which: AllMailbox) -> Held {
    let mut held = Held::default();
    for f in folders(mb, which) {
        let h = match which {
            AllMailbox::Vips => counts.vips(f),
            AllMailbox::Flagged => counts.flagged(f),
            AllMailbox::Inboxes | AllMailbox::Drafts | AllMailbox::Sent => counts.folder(f),
        };
        held.total = held.total.saturating_add(h.total);
        held.unread = held.unread.saturating_add(h.unread);
    }
    held
}

/// What the Dock icon's badge says for `unread` messages: the number, "9999+" past that, and no
/// badge for none.
pub fn badge_text(unread: usize) -> Option<String> {
    match unread {
        0 => None,
        1..=9_999 => Some(unread.to_string()),
        _ => Some("9999+".to_string()),
    }
}

/// Unread mail in every account's Inbox: what the Dock's badge says.
pub fn unread_in_inboxes(s: &mut Session) -> usize {
    let counts = s.folder_counts();
    held(&s.mailbox, &counts, AllMailbox::Inboxes).unread
}
