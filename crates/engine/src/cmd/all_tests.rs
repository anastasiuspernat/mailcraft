// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Mailboxes made of every account's mail: All Inboxes, VIPs, Flagged, All Drafts and All Sent
//! over the three accounts of the sample with several accounts; the smart Inbox and its Priority
//! over All Inboxes; VIPs.

use std::collections::HashSet;

use mailcraft_model::insight::{InboxCategory, Job, JobKind};
use mailcraft_model::{FlagStatus, FolderKind, Id, Mailbox};
use serde_json::{Value, json};

use crate::Session;
use crate::all::{self, AllMailbox};

const NOW: i64 = 1_800_000_000;

fn setup() -> Session {
    let mut s = Session::new(crate::sample::several_accounts(NOW, 0));
    s.clock.now = Some(NOW);
    s
}

/// The ids the list in view lists.
fn listed(s: &mut Session) -> HashSet<Id> {
    s.rows().iter().flat_map(|r| r.members.clone()).collect()
}

/// The ids of the messages in folders of `kind`, whichever account, that `keep` keeps.
fn of_kind(mb: &Mailbox, kind: FolderKind, keep: impl Fn(&mailcraft_model::Message) -> bool) -> HashSet<Id> {
    mb.messages.values().filter(|m| mb.folder(m.folder).is_some_and(|f| f.kind == kind) && keep(m)).map(|m| m.id).collect()
}

#[test]
fn the_sample_has_three_accounts_each_with_inbox_mail() {
    let s = setup();
    assert_eq!(s.mailbox.accounts.len(), 3);
    for a in &s.mailbox.accounts {
        let inbox = s.mailbox.folder_of_kind(a.id, FolderKind::Inbox).unwrap();
        assert!(s.mailbox.messages.values().any(|m| m.folder == inbox), "{}", a.email);
    }
}

#[test]
fn all_inboxes_lists_every_accounts_inbox_and_nothing_else() {
    let mut s = setup();
    let r = s.run("view.allInboxes", &json!({})).unwrap();
    assert_eq!(r["mailbox"], "inboxes");
    assert_eq!(r["folders"].as_array().unwrap().len(), 3);
    let want = of_kind(&s.mailbox, FolderKind::Inbox, |_| true);
    assert_eq!(listed(&mut s), want);
    let accounts: HashSet<Id> = want.iter().filter_map(|id| s.mailbox.message(*id)).map(|m| m.account).collect();
    assert_eq!(accounts.len(), 3, "mail of all three accounts");
    // Its counts: the Inboxes' messages, and the unread among them.
    let unread = want.iter().filter_map(|id| s.mailbox.message(*id)).filter(|m| !m.read).count();
    assert_eq!(r["messages"], want.len());
    assert_eq!(r["unread"], unread);
    assert_eq!(all::unread_in_inboxes(&mut s), unread);
}

#[test]
fn all_drafts_and_all_sent_gather_every_account() {
    let mut s = setup();
    s.run("view.allDrafts", &json!({})).unwrap();
    let drafts = of_kind(&s.mailbox, FolderKind::Drafts, |_| true);
    assert_eq!(listed(&mut s), drafts);
    assert!(drafts.len() >= 2);
    s.run("view.allSent", &json!({})).unwrap();
    let sent = of_kind(&s.mailbox, FolderKind::Sent, |_| true);
    assert_eq!(listed(&mut s), sent);
    let accounts: HashSet<Id> = sent.iter().filter_map(|id| s.mailbox.message(*id)).map(|m| m.account).collect();
    assert_eq!(accounts.len(), 3);
}

#[test]
fn flagged_lists_flagged_mail_of_every_folder_but_deleted_and_junk() {
    let mut s = setup();
    // One flagged in Deleted Items stays out.
    let a = s.mailbox.accounts[1].id;
    let deleted = s.mailbox.folder_of_kind(a, FolderKind::Deleted).unwrap();
    let id = *of_kind(&s.mailbox, FolderKind::Inbox, |m| m.account == a).iter().next().unwrap();
    s.mailbox.message_mut(id).unwrap().flag = FlagStatus::Flagged;
    s.mailbox.message_mut(id).unwrap().folder = deleted;
    let r = s.run("view.allFlagged", &json!({})).unwrap();
    let want: HashSet<Id> = s.mailbox.messages.values().filter(|m| m.flag == FlagStatus::Flagged && m.folder != deleted).map(|m| m.id).collect();
    assert!(want.len() >= 2, "the sample has flagged mail in two accounts");
    assert_eq!(listed(&mut s), want);
    assert!(!listed(&mut s).contains(&id));
    assert_eq!(r["messages"], want.len());
}

#[test]
fn vips_lists_their_mail_from_every_account() {
    let mut s = setup();
    s.run("view.vips", &json!({})).unwrap();
    let priya = |m: &mailcraft_model::Message| m.from.email == "priya@lumen.example";
    let want: HashSet<Id> = s
        .mailbox
        .messages
        .values()
        .filter(|m| priya(m))
        .filter(|m| s.mailbox.folder(m.folder).is_some_and(|f| !matches!(f.kind, FolderKind::Deleted | FolderKind::Junk)))
        .map(|m| m.id)
        .collect();
    let accounts: HashSet<Id> = want.iter().filter_map(|id| s.mailbox.message(*id)).map(|m| m.account).collect();
    assert!(accounts.len() >= 2, "Priya wrote to two accounts");
    assert_eq!(listed(&mut s), want);
    // Another VIP, from a message: their mail joins.
    let marco = s.mailbox.messages.values().find(|m| m.from.email == "marco@bandhall.example").map(|m| m.id).unwrap();
    let r = s.run("people.setVip", &json!({"ids": [marco], "vip": true})).unwrap();
    assert_eq!(r["addresses"], json!(["marco@bandhall.example"]));
    assert!(listed(&mut s).contains(&marco));
    // And leaves when they stop being one.
    s.run("people.setVip", &json!({"address": "Marco@BandHall.example", "vip": false})).unwrap();
    assert!(!listed(&mut s).contains(&marco));
    assert!(s.run("people.setVip", &json!({"address": "not an address", "vip": true})).is_err());
    assert!(s.run("people.setVip", &json!({"ids": [marco]})).is_err(), "`vip` is required");
}

#[test]
fn choosing_a_folder_leaves_the_mailbox_of_every_account() {
    let mut s = setup();
    s.run("view.allInboxes", &json!({})).unwrap();
    assert_eq!(all::in_view(&s), Some(AllMailbox::Inboxes));
    let sent = s.mailbox.folder_of_kind(s.mailbox.accounts[2].id, FolderKind::Sent).unwrap();
    s.run("folder.select", &json!({"id": sent})).unwrap();
    assert_eq!(all::in_view(&s), None);
    let only: HashSet<Id> = s.mailbox.messages.values().filter(|m| m.folder == sent).map(|m| m.id).collect();
    assert_eq!(listed(&mut s), only);
}

#[test]
fn the_folder_pane_switch_toggles() {
    let mut s = setup();
    assert!(s.prefs.all_mailboxes, "on by default");
    assert_eq!(s.run("view.showAllMailboxes", &json!({})).unwrap()["value"], false);
    assert_eq!(s.run("view.showAllMailboxes", &json!({"value": true})).unwrap()["value"], true);
}

/// The model answers every job: Primary, and a priority for mail whose subject has `urgent`.
fn answer(s: &mut Session, urgent: &str) {
    loop {
        let r = s.run("ai.request", &json!({"max": 32})).unwrap();
        let jobs: Vec<Job> = serde_json::from_value(r["jobs"].clone()).unwrap();
        if jobs.is_empty() {
            break;
        }
        let results: Vec<Value> = jobs
            .iter()
            .map(|j| {
                let hot = s.mailbox.message(j.id).is_some_and(|m| m.subject.contains(urgent));
                let mut r =
                    json!({"id": j.id, "kind": j.kind, "contentHash": j.content_hash, "outcome": "done", "category": "primary", "priority": hot});
                if j.kind != JobKind::Classify {
                    r["summary"] = json!("A note.");
                }
                r
            })
            .collect();
        s.run("ai.apply", &json!({"results": results})).unwrap();
    }
}

#[test]
fn primary_and_priority_gather_every_accounts_inbox() {
    let mut s = setup();
    s.run("ai.options", &json!({"modelPath": "/models/test.gguf", "enabled": true})).unwrap();
    s.run("view.allInboxes", &json!({})).unwrap();
    // Unread mail in two accounts is time-sensitive.
    answer(&mut s, "rebooked");
    let rows = s.rows();
    let priority: Vec<Id> = rows.iter().filter(|r| r.priority).map(|r| r.id).collect();
    assert!(!priority.is_empty());
    // The time-sensitive mail, and a VIP's unread mail (Priya is one in the sample).
    assert!(priority.iter().all(|id| s.mailbox.message(*id).is_some_and(|m| m.subject.contains("rebooked") || s.mailbox.is_vip(&m.from.email))));
    assert!(priority.iter().any(|id| s.mailbox.message(*id).is_some_and(|m| m.subject.contains("rebooked"))));
    // Primary of All Inboxes: Primary mail of all three accounts.
    assert_eq!(crate::ai::in_view(&s), Some(InboxCategory::Primary));
    let accounts: HashSet<Id> = rows.iter().filter_map(|r| s.mailbox.message(r.id)).map(|m| m.account).collect();
    assert_eq!(accounts.len(), 3, "{accounts:?}");
    // A part of the smart Inbox over All Inboxes: Promotions of every account.
    let ad = s.mailbox.messages.values().find(|m| m.subject.starts_with("20% off")).map(|m| m.id).unwrap();
    s.run("ai.setCategory", &json!({"ids": [ad], "category": "promotions"})).unwrap();
    s.run("view.inboxCategory", &json!({"category": "promotions"})).unwrap();
    assert!(listed(&mut s).contains(&ad));
    // VIPs and Flagged have no parts.
    s.run("view.vips", &json!({})).unwrap();
    assert_eq!(crate::ai::in_view(&s), None);
}

#[test]
fn a_vips_mail_is_primary() {
    let mut s = setup();
    s.run("ai.options", &json!({"modelPath": "/models/test.gguf", "enabled": true, "summaryMinWords": 5000})).unwrap();
    // The model would put everything in Promotions; Priya is a VIP.
    loop {
        let r = s.run("ai.request", &json!({"max": 32})).unwrap();
        let jobs: Vec<Job> = serde_json::from_value(r["jobs"].clone()).unwrap();
        if jobs.is_empty() {
            break;
        }
        let results: Vec<Value> = jobs
            .iter()
            .map(|j| json!({"id": j.id, "kind": j.kind, "contentHash": j.content_hash, "outcome": "done", "category": "promotions", "priority": false, "summary": "A note."}))
            .collect();
        s.run("ai.apply", &json!({"results": results})).unwrap();
    }
    let category = |s: &Session, from: &str| {
        s.mailbox.messages.values().filter(|m| m.from.email == from && crate::ai::sorts(s, m)).map(|m| crate::ai::category_of(m)).collect::<Vec<_>>()
    };
    assert!(category(&s, "priya@lumen.example").iter().all(|c| *c == InboxCategory::Primary));
    // A sender the model put in Promotions becomes a VIP: their sorted mail goes to Primary too.
    let sender = s
        .mailbox
        .messages
        .values()
        .find(|m| crate::ai::sorts(&s, m) && crate::ai::category_of(m) == InboxCategory::Promotions)
        .map(|m| m.from.email.clone())
        .unwrap();
    s.run("people.setVip", &json!({"address": sender, "vip": true})).unwrap();
    assert!(category(&s, &sender).iter().all(|c| *c == InboxCategory::Primary));
}

#[test]
fn counts_follow_read_marks_and_flags() {
    let mut s = setup();
    let counts = s.folder_counts();
    let before = all::held(&s.mailbox, &counts, AllMailbox::Vips);
    drop(counts);
    let unread_vip = s.mailbox.messages.values().find(|m| m.from.email == "priya@lumen.example" && !m.read).map(|m| m.id).unwrap();
    s.run("mail.markRead", &json!({"ids": [unread_vip]})).unwrap();
    let counts = s.folder_counts();
    let after = all::held(&s.mailbox, &counts, AllMailbox::Vips);
    assert_eq!(after.unread + 1, before.unread);
    assert_eq!(after.total, before.total);
    let flagged = all::held(&s.mailbox, &counts, AllMailbox::Flagged).total;
    drop(counts);
    let id = s.mailbox.messages.values().find(|m| m.flag != FlagStatus::Flagged && m.from.email == "rosa@family.example").map(|m| m.id).unwrap();
    s.run("mail.flag", &json!({"id": id})).unwrap();
    let counts = s.folder_counts();
    assert_eq!(all::held(&s.mailbox, &counts, AllMailbox::Flagged).total, flagged + 1);
}

#[test]
fn vips_are_kept_with_the_mailbox_and_tidied_when_read_back() {
    let mut mb = crate::sample::several_accounts(NOW, 0);
    let text = serde_json::to_string(&mb).unwrap();
    let back: Mailbox = serde_json::from_str(&text).unwrap();
    assert!(back.is_vip("PRIYA@lumen.example "));
    mb.vips.insert("  Odd@Example.ORG ".into());
    mb.vips.insert(String::new());
    mb.repair();
    assert!(mb.vips.contains("odd@example.org"));
    assert!(!mb.vips.contains(""));
}

#[test]
fn the_dock_badge_says_the_unread_count_and_nothing_for_none() {
    assert_eq!(all::badge_text(0), None);
    assert_eq!(all::badge_text(7).as_deref(), Some("7"));
    assert_eq!(all::badge_text(9_999).as_deref(), Some("9999"));
    assert_eq!(all::badge_text(123_456).as_deref(), Some("9999+"));
}
