// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Search by meaning: every message read for it, newest first; a search's meaning asked for and
//! taken in; mail that means the search found beside mail with its words, marked so, within the
//! search's other terms; nothing changes without an embedding model; reading again after a change.

use mailcraft_model::insight::{EMBED_DIM, Job, JobKind};
use mailcraft_model::{Address, Body, FolderKind, Id, Message};
use serde_json::{Value, json};

use crate::Session;
use crate::list::Row;

const NOW: i64 = 1_800_000_000;

/// Words that mean the same, for the fake model below.
const SAME: &[(&str, &[&str])] = &[("travel", &["flight", "booking", "plane"]), ("change", &["change", "rebooked", "moved"])];

/// A fake embedding: each word (its group's, when it has one) adds to one number.
fn embed(text: &str) -> Vec<f32> {
    let mut v = vec![0.0f32; EMBED_DIM];
    for word in text.to_lowercase().split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2) {
        let word = SAME.iter().find(|(_, ws)| ws.contains(&word)).map_or(word, |(n, _)| n);
        let slot = word.bytes().fold(7usize, |h, b| h.wrapping_mul(31).wrapping_add(usize::from(b))) % EMBED_DIM;
        v[slot] += if SAME.iter().any(|(n, _)| *n == word) { 3.0 } else { 1.0 };
    }
    v
}

fn setup() -> (Session, Id) {
    let mut s = Session::new(mailcraft_model::Mailbox::with_local_account("Owner", "owner@example.org"));
    let a = s.mailbox.accounts[0].id;
    let inbox = s.mailbox.folder_of_kind(a, FolderKind::Inbox).unwrap_or_default();
    s.clock.now = Some(NOW);
    s.run("ai.options", &json!({"searchModelPath": "/models/embed.gguf"})).unwrap();
    s.run("folder.select", &json!({"id": inbox})).unwrap();
    (s, inbox)
}

fn add(s: &mut Session, folder: Id, from: &str, subject: &str, text: &str, received: i64) -> Id {
    let m = Message {
        folder,
        from: Address::new("", from),
        to: vec![Address::new("", "owner@example.org")],
        subject: subject.into(),
        body: Body { text: text.into(), html: None },
        received,
        date: received,
        ..Default::default()
    };
    s.mailbox.add_message(m).unwrap()
}

/// The model answers every job handed out; what it was handed, kind by kind.
fn answer(s: &mut Session) -> Vec<(Id, JobKind)> {
    let mut handed = Vec::new();
    loop {
        let r = s.run("ai.request", &json!({"max": 32})).unwrap();
        let jobs: Vec<Job> = serde_json::from_value(r["jobs"].clone()).unwrap();
        if jobs.is_empty() {
            return handed;
        }
        let results: Vec<Value> = jobs
            .iter()
            .map(|j| json!({"id": j.id, "kind": j.kind, "contentHash": j.content_hash, "outcome": "done", "vector": embed(&j.input), "modelId": "embed"}))
            .collect();
        handed.extend(jobs.iter().map(|j| (j.id, j.kind)));
        s.run("ai.apply", &json!({"results": results})).unwrap();
    }
}

fn search(s: &mut Session, text: &str) -> Vec<Row> {
    s.run("view.search", &json!({"query": text})).unwrap_or_else(|_| {
        s.view.search = text.into();
        json!({})
    });
    crate::ai::ask_meaning(s);
    answer(s);
    s.rows().as_ref().clone()
}

#[test]
fn every_message_is_read_once_newest_first_and_again_when_it_changes() {
    let (mut s, inbox) = setup();
    let old = add(&mut s, inbox, "a@example.org", "Old", "Lunch next week?", NOW - 86_400);
    let new = add(&mut s, inbox, "b@example.org", "New", "Your booking was rebooked.", NOW - 60);
    let handed = answer(&mut s);
    assert_eq!(handed, vec![(new, JobKind::Embed), (old, JobKind::Embed)]);
    assert!(answer(&mut s).is_empty(), "read once");
    if let Ok(m) = s.mailbox.message_mut(old) {
        m.subject = "Old, changed".into();
    }
    assert_eq!(answer(&mut s), vec![(old, JobKind::Embed)]);
    // Read every message again.
    s.run("ai.reindexVectors", &json!({})).unwrap();
    assert_eq!(answer(&mut s).len(), 2);
    // The smart Inbox stays off: nothing is sorted.
    assert!(s.mailbox.message(new).is_some_and(|m| m.insight.is_none()));
}

#[test]
fn a_search_finds_mail_that_means_it_and_marks_it() {
    let (mut s, inbox) = setup();
    let rebooked = add(&mut s, inbox, "air@air.example", "Your booking was rebooked", "We rebooked you on the 9:40 departure.", NOW - 60);
    let words = add(&mut s, inbox, "ada@example.org", "Flight change", "The flight change form is attached.", NOW - 120);
    let garden = add(&mut s, inbox, "club@garden.example", "Spring sale", "Plants and compost.", NOW - 180);
    answer(&mut s);
    let rows = search(&mut s, "flight change");
    let found: Vec<(Id, bool)> = rows.iter().map(|r| (r.id, r.meaning)).collect();
    assert!(found.contains(&(words, false)), "{found:?}");
    assert!(found.contains(&(rebooked, true)), "{found:?}");
    assert!(!found.iter().any(|(id, _)| *id == garden), "{found:?}");
    // The search's other terms hold for what is found by meaning too.
    let rows = search(&mut s, "flight change from:ada");
    assert_eq!(rows.iter().map(|r| r.id).collect::<Vec<_>>(), vec![words]);
    // A quoted phrase, an address, or a single short word is found by its words alone.
    for text in ["\"flight change\"", "air@air.example", "fl"] {
        assert!(search(&mut s, text).iter().all(|r| !r.meaning), "{text}");
    }
}

#[test]
fn without_an_embedding_model_search_is_as_it_was() {
    let (mut s, inbox) = setup();
    let rebooked = add(&mut s, inbox, "air@air.example", "Your booking was rebooked", "We rebooked you.", NOW - 60);
    answer(&mut s);
    assert!(search(&mut s, "flight change").iter().any(|r| r.id == rebooked));
    s.run("ai.options", &json!({"search": false})).unwrap();
    assert!(search(&mut s, "flight change").is_empty());
    s.run("ai.options", &json!({"search": true, "searchModelPath": ""})).unwrap();
    assert!(search(&mut s, "flight change").is_empty());
    let r = s.run("ai.request", &json!({"max": 32})).unwrap();
    assert_eq!(r["jobs"], json!([]));
}

#[test]
fn a_wrong_embedding_is_refused() {
    let (mut s, inbox) = setup();
    let id = add(&mut s, inbox, "a@example.org", "Hi", "Hello", NOW);
    let r = s.run("ai.request", &json!({"max": 4})).unwrap();
    let job: Job = serde_json::from_value(r["jobs"][0].clone()).unwrap();
    let bad = json!({"results": [{"id": id, "kind": "embed", "contentHash": job.content_hash, "outcome": "done", "vector": [1.0, 2.0]}]});
    let applied = s.run("ai.apply", &bad).unwrap();
    assert_eq!(applied["failed"], 1);
    assert_eq!(s.ai.vectors.as_ref().map_or(0, |v| v.len()), 0);
}
