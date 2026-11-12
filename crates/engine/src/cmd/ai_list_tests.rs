// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The smart Inbox's list: the Priority section at the top of Primary (the model's word, the
//! owner's, how long it lasts, "n more from …") and each sender's mail together in the other
//! parts.

use mailcraft_model::insight::{InboxCategory, Job, JobKind};
use mailcraft_model::{Address, Body, FolderKind, Id, Message};
use serde_json::{Value, json};

use crate::Session;
use crate::list::{MORE_COPY, PRIORITY, Row};

const NOW: i64 = 1_800_000_000;

fn setup() -> (Session, Id) {
    let mut s = Session::new(mailcraft_model::Mailbox::with_local_account("Owner", "owner@example.org"));
    let a = s.mailbox.accounts[0].id;
    let inbox = s.mailbox.folder_of_kind(a, FolderKind::Inbox).unwrap_or_default();
    s.clock.now = Some(NOW);
    s.prefs.show_groups = true;
    s.run("ai.options", &json!({"modelPath": "/models/test.gguf", "enabled": true, "summaryMinWords": 200})).unwrap();
    s.run("folder.select", &json!({"id": inbox})).unwrap();
    (s, inbox)
}

fn add(s: &mut Session, folder: Id, name: &str, from: &str, subject: &str, received: i64) -> Id {
    let m = Message {
        folder,
        from: Address::new(name, from),
        to: vec![Address::new("", "owner@example.org")],
        subject: subject.into(),
        body: Body { text: "Hello there, a short note.".into(), html: None },
        received,
        date: received,
        ..Default::default()
    };
    s.mailbox.add_message(m).unwrap()
}

/// The model answers every job handed out: `category` for all, priority for the ids in `urgent`.
fn answer(s: &mut Session, category: &str, urgent: &[Id], due: Option<&str>) {
    loop {
        let r = s.run("ai.request", &json!({"max": 32})).unwrap();
        let jobs: Vec<Job> = serde_json::from_value(r["jobs"].clone()).unwrap();
        if jobs.is_empty() {
            break;
        }
        let results: Vec<Value> = jobs
            .iter()
            .map(|j| {
                let mut r = json!({"id": j.id, "kind": j.kind, "contentHash": j.content_hash, "outcome": "done", "category": category, "priority": urgent.contains(&j.id)});
                if j.kind != JobKind::Classify {
                    r["summary"] = json!("A note.");
                    r["due"] = json!(due);
                }
                r
            })
            .collect();
        s.run("ai.apply", &json!({"results": results})).unwrap();
    }
}

fn rows(s: &mut Session) -> Vec<Row> {
    s.rows().as_ref().clone()
}

#[test]
fn time_sensitive_mail_comes_first_under_priority() {
    let (mut s, inbox) = setup();
    let old = add(&mut s, inbox, "Ada", "ada@example.org", "Lunch some day", NOW - 3_600);
    let urgent = add(&mut s, inbox, "Ben", "ben@example.org", "Meeting moved to today", NOW - 7_200);
    answer(&mut s, "primary", &[urgent], None);
    assert_eq!(s.mailbox.message(urgent).and_then(|m| m.insight.as_ref()).and_then(|i| i.priority), Some(true));
    let list = rows(&mut s);
    assert_eq!(list.iter().map(|r| r.id).collect::<Vec<_>>(), vec![urgent, old]);
    assert_eq!(list[0].group.as_deref(), Some(PRIORITY));
    assert!(list[0].priority && !list[1].priority);
    // The rest has its date heading again.
    assert!(list[1].group.as_deref().is_some_and(|g| g != PRIORITY), "{:?}", list[1].group);
    // Nothing is asked again: the model said for each.
    let again: Vec<Job> = serde_json::from_value(s.run("ai.request", &json!({"max": 32})).unwrap()["jobs"].clone()).unwrap();
    assert!(again.is_empty(), "{again:?}");
}

#[test]
fn the_owner_takes_mail_out_of_priority_and_the_model_never_puts_it_back() {
    let (mut s, inbox) = setup();
    let urgent = add(&mut s, inbox, "Ben", "ben@example.org", "Due today", NOW - 60);
    answer(&mut s, "primary", &[urgent], None);
    assert!(rows(&mut s)[0].priority);
    s.run("ai.setPriority", &json!({"ids": [urgent], "priority": false})).unwrap();
    assert!(!rows(&mut s)[0].priority);
    // Forgetting summaries keeps the owner's word.
    s.run("ai.clearInsights", &json!({})).unwrap();
    answer(&mut s, "primary", &[urgent], None);
    assert!(!rows(&mut s)[0].priority);
    // And the owner can put it back, or leave it to the model again.
    s.run("ai.setPriority", &json!({"ids": [urgent], "priority": null})).unwrap();
    assert!(rows(&mut s)[0].priority);
    assert!(s.run("ai.setPriority", &json!({"ids": [urgent], "priority": "yes"})).is_err());
}

#[test]
fn a_priority_lasts_until_its_due_day_or_two_days() {
    let (mut s, inbox) = setup();
    let plain = add(&mut s, inbox, "Ben", "ben@example.org", "Call me", NOW - 60);
    answer(&mut s, "primary", &[plain], None);
    assert!(rows(&mut s)[0].priority);
    s.clock.now = Some(NOW + 2 * 86_400 + 60);
    assert!(!rows(&mut s).iter().any(|r| r.priority), "two days on, it is ordinary mail");
    // One with a due date lasts to the end of that day, whenever it came.
    let (mut s, inbox) = setup();
    s.run("ai.options", &json!({"summaryMinWords": 1})).unwrap();
    let dated = add(&mut s, inbox, "Ben", "ben@example.org", "Report", NOW - 60);
    let today = s.today();
    let due = format!("{:04}-{:02}-{:02}", today.year, today.month, today.day);
    answer(&mut s, "primary", &[dated], Some(&due));
    assert!(rows(&mut s)[0].priority);
    s.clock.now = Some(NOW + 86_400);
    assert!(!rows(&mut s).iter().any(|r| r.priority), "the day after its due day");
}

#[test]
fn more_than_four_priorities_fold_into_n_more_from() {
    let (mut s, inbox) = setup();
    let names = ["Ada", "Ben", "Cy", "Di", "Ed", "Flo"];
    let ids: Vec<Id> = names
        .iter()
        .enumerate()
        .map(|(i, n)| add(&mut s, inbox, n, &format!("{}@example.org", n.to_lowercase()), "Today", NOW - 60 * (i as i64 + 1)))
        .collect();
    answer(&mut s, "primary", &ids, None);
    let list = rows(&mut s);
    assert_eq!(list.len(), 4, "{list:?}");
    let more = list[3].more.clone().unwrap_or_else(|| panic!("no more row: {list:?}"));
    assert_eq!(
        (more.key.as_str(), more.count, more.names.clone(), more.others, more.open),
        ("Priority", 3, vec!["Di".to_string(), "Ed".into(), "Flo".into()], 0, false)
    );
    assert_eq!(list[3].copy, MORE_COPY);
    assert_eq!(list[3].members, ids[3..].to_vec());
    // It is no message of its own, and what it folds away is counted as listed.
    assert_eq!(crate::list::listed(&list), 6);
    s.run("view.smartGroup", &json!({"key": "Priority"})).unwrap();
    let list = rows(&mut s);
    assert_eq!(list.len(), 7);
    assert!(list[6].more.as_ref().is_some_and(|m| m.open));
    s.run("view.smartGroup", &json!({"key": "Priority", "open": false})).unwrap();
    assert_eq!(rows(&mut s).len(), 4);
    // Four fold nothing: there would be one row behind "1 more".
    let (mut s, inbox) = setup();
    let ids: Vec<Id> = names[..4].iter().map(|n| add(&mut s, inbox, n, &format!("{}@example.org", n.to_lowercase()), "Today", NOW - 60)).collect();
    answer(&mut s, "primary", &ids, None);
    assert!(rows(&mut s).iter().all(|r| r.more.is_none()));
}

#[test]
fn each_senders_mail_comes_together_in_the_other_parts() {
    let (mut s, inbox) = setup();
    let shop1 = add(&mut s, inbox, "Shop", "news@shop.example", "Week 3", NOW - 60);
    let air = add(&mut s, inbox, "Air", "fly@air.example", "Your miles", NOW - 120);
    let shop2 = add(&mut s, inbox, "Shop", "NEWS@shop.example", "Week 2", NOW - 180);
    let shop3 = add(&mut s, inbox, "Shop", "news@shop.example", "Week 1", NOW - 86_400 * 3);
    answer(&mut s, "updates", &[], None);
    s.run("view.inboxCategory", &json!({"category": "updates"})).unwrap();
    let list = rows(&mut s);
    assert_eq!(list.iter().map(|r| (r.id, r.copy)).collect::<Vec<_>>(), vec![(shop1, 0), (shop2, MORE_COPY), (air, 0)], "{list:?}");
    let more = list[1].more.clone().unwrap_or_else(|| panic!("no more row"));
    assert_eq!((more.key.as_str(), more.count, more.names.clone()), ("from:news@shop.example", 2, vec!["Shop".to_string()]));
    assert_eq!(list[1].members, vec![shop2, shop3]);
    // Headings are for the rows that start a day, not for what is folded.
    assert!(list[0].group.is_some());
    assert!(list[1].group.is_none() && list[2].group.is_none());
    s.run("view.smartGroup", &json!({"key": "from:news@shop.example"})).unwrap();
    let list = rows(&mut s);
    assert_eq!(list.iter().map(|r| r.id).collect::<Vec<_>>(), vec![shop1, shop2, shop3, shop2, air]);
    assert!(list[1].grouped && list[2].grouped && list[1].group.is_none() && list[2].group.is_none());
    // A list sorted otherwise, or a search, is left as it was.
    s.run("view.sort", &json!({"by": "from"})).unwrap();
    assert!(rows(&mut s).iter().all(|r| r.more.is_none() && !r.grouped));
}

#[test]
fn short_unread_mail_gets_a_summary_for_the_list_and_read_mail_doesnt() {
    let (mut s, inbox) = setup();
    let text = "Could you send me the signed form before our call on Thursday afternoon please?";
    let unread = add(&mut s, inbox, "Ada", "ada@example.org", "Form", NOW - 60);
    let read = add(&mut s, inbox, "Ben", "ben@example.org", "Form too", NOW - 120);
    for id in [unread, read] {
        if let Ok(m) = s.mailbox.message_mut(id) {
            m.body.text = text.into();
        }
    }
    if let Ok(m) = s.mailbox.message_mut(read) {
        m.read = true;
    }
    let r = s.run("ai.request", &json!({"max": 32})).unwrap();
    let jobs: Vec<Job> = serde_json::from_value(r["jobs"].clone()).unwrap();
    let kind = |id| jobs.iter().find(|j| j.id == id).map(|j| j.kind);
    assert_eq!(kind(unread), Some(JobKind::Summarize));
    assert_eq!(kind(read), Some(JobKind::Classify));
    s.run("ai.options", &json!({"listSummaries": false})).unwrap();
    s.run("ai.clearInsights", &json!({})).unwrap();
    s.ai.in_flight.clear();
    let r = s.run("ai.request", &json!({"max": 32})).unwrap();
    let jobs: Vec<Job> = serde_json::from_value(r["jobs"].clone()).unwrap();
    assert!(jobs.iter().all(|j| j.kind == JobKind::Classify));
    let _ = InboxCategory::Primary;
}

#[test]
fn only_unread_mail_in_primary_is_a_priority() {
    let (mut s, inbox) = setup();
    let unread = add(&mut s, inbox, "Ben", "ben@example.org", "Due today", NOW - 60);
    let read = add(&mut s, inbox, "Cy", "cy@example.org", "Also due today", NOW - 120);
    s.mailbox.message_mut(read).unwrap().read = true;
    answer(&mut s, "primary", &[unread, read], None);
    let priority = |s: &Session, id| s.mailbox.message(id).and_then(|m| m.insight.as_ref()).and_then(|i| i.priority);
    assert_eq!(priority(&s, unread), Some(true));
    assert_eq!(priority(&s, read), Some(false), "the model's yes on read mail is a no");
    let list = rows(&mut s);
    assert!(list.iter().find(|r| r.id == unread).is_some_and(|r| r.priority));
    assert!(list.iter().find(|r| r.id == read).is_some_and(|r| !r.priority));
}

#[test]
fn mail_sorted_elsewhere_is_never_a_priority() {
    let (mut s, inbox) = setup();
    let id = add(&mut s, inbox, "Shop", "shop@example.org", "Sale ends today", NOW - 60);
    answer(&mut s, "promotions", &[id], None);
    assert_eq!(s.mailbox.message(id).and_then(|m| m.insight.as_ref()).and_then(|i| i.priority), Some(false));
}

#[test]
fn read_mail_sorted_into_primary_by_a_rule_isnt_asked_about_priority() {
    let (mut s, inbox) = setup();
    let id = add(&mut s, inbox, "Ada", "ada@example.org", "Hello", NOW - 60);
    s.mailbox.message_mut(id).unwrap().read = true;
    s.run("ai.setCategory", &json!({"ids": [id], "category": "primary"})).unwrap();
    let jobs: Vec<Job> = serde_json::from_value(s.run("ai.request", &json!({"max": 32})).unwrap()["jobs"].clone()).unwrap();
    assert!(jobs.iter().all(|j| j.id != id), "{jobs:?}");
}

#[test]
fn all_mail_has_primarys_priority_section_first_and_the_rest_by_date() {
    let (mut s, inbox) = setup();
    let old = add(&mut s, inbox, "Ada", "ada@example.org", "Lunch some day", NOW - 7_200);
    let urgent = add(&mut s, inbox, "Ben", "ben@example.org", "Meeting moved to today", NOW - 10_800);
    answer(&mut s, "primary", &[urgent], None);
    let deal = add(&mut s, inbox, "Shop", "deals@shop.example", "Half price today", NOW - 3_600);
    let deal2 = add(&mut s, inbox, "Shop", "deals@shop.example", "Last chance", NOW - 5_400);
    answer(&mut s, "promotions", &[], None);
    s.run("view.inboxCategory", &json!({"category": "all"})).unwrap();
    let list = rows(&mut s);
    // Priority first, then everything else by date: no sender's mail folded together.
    assert_eq!(list.iter().map(|r| r.id).collect::<Vec<_>>(), vec![urgent, deal, deal2, old]);
    assert_eq!(list[0].group.as_deref(), Some(PRIORITY));
    assert!(list[0].priority && list[1..].iter().all(|r| !r.priority && r.more.is_none()));
    // Transactions has no Priority section.
    s.run("view.inboxCategory", &json!({"category": "transactions"})).unwrap();
    assert!(rows(&mut s).iter().all(|r| !r.priority));
}

#[test]
fn primary_and_all_mail_say_mail_is_being_read_until_it_is() {
    let (mut s, inbox) = setup();
    add(&mut s, inbox, "Ben", "ben@example.org", "Meeting moved to today", NOW - 60);
    // No model that can run: nothing is being read.
    assert!(!crate::ai::reading_mail(&s));
    s.ai.host.available = true;
    s.run("ai.request", &json!({"max": 1})).unwrap();
    assert!(crate::ai::reading_mail(&s));
    s.run("view.inboxCategory", &json!({"category": "all"})).unwrap();
    assert!(crate::ai::reading_mail(&s));
    s.run("view.inboxCategory", &json!({"category": "transactions"})).unwrap();
    assert!(!crate::ai::reading_mail(&s), "only where the Priority box is");
    s.run("view.inboxCategory", &json!({"category": "primary"})).unwrap();
    // A model that failed to load reads nothing.
    s.ai.host.error = "no model".into();
    assert!(!crate::ai::reading_mail(&s));
    s.ai.host.error.clear();
    s.ai.in_flight.clear();
    answer(&mut s, "primary", &[], None);
    assert!(!crate::ai::reading_mail(&s), "all read");
}
