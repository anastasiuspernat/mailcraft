// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a model writes is checked before it is kept; insights stay out of older files' way.
use super::*;
use crate::{Mailbox, Message};

fn answer(category: Option<&str>, summary: Option<&str>) -> Answer {
    Answer { category: category.map(str::to_string), summary: summary.map(str::to_string), ..Default::default() }
}

#[test]
fn categories_round_trip_by_code() {
    for c in InboxCategory::ALL {
        assert_eq!(InboxCategory::parse(c.code()), Some(c));
        assert_eq!(InboxCategory::parse(&format!("  {}  ", c.code().to_uppercase())), Some(c));
    }
    assert_eq!(InboxCategory::parse("social"), None);
    assert_eq!(InboxCategory::parse(""), None);
}

#[test]
fn a_classify_answer_needs_a_known_category() {
    assert_eq!(answer(Some("updates"), None).checked(JobKind::Classify).map(|c| c.category), Ok(Some(InboxCategory::Updates)));
    assert_eq!(answer(Some("spam"), None).checked(JobKind::Classify), Err(Refused::Category));
    assert_eq!(answer(None, None).checked(JobKind::Classify), Err(Refused::Missing));
}

#[test]
fn a_summary_answer_needs_both() {
    assert_eq!(answer(Some("primary"), None).checked(JobKind::Summarize), Err(Refused::Missing));
    assert_eq!(answer(None, Some("Hi.")).checked(JobKind::Summarize), Err(Refused::Missing));
    let ok = answer(Some("primary"), Some("Ada asks for the draft.")).checked(JobKind::Summarize);
    assert_eq!(ok.map(|c| c.summary.map(|s| s.text)), Ok(Some("Ada asks for the draft.".to_string())));
    // A conversation's summary needs no category.
    assert!(answer(None, Some("They agreed on Monday.")).checked(JobKind::SummarizeThread).is_ok());
}

#[test]
fn oversized_text_is_refused_not_cut() {
    let long = "a".repeat(MAX_SUMMARY_CHARS + 1);
    assert_eq!(answer(Some("primary"), Some(&long)).checked(JobKind::Summarize), Err(Refused::Text));
    let mut a = answer(Some("primary"), Some("Fine."));
    a.actions = vec!["x".into(); MAX_ACTIONS + 1];
    assert_eq!(a.checked(JobKind::Summarize), Err(Refused::Text));
    a.actions = vec!["y".repeat(MAX_ACTION_CHARS + 1)];
    assert_eq!(a.checked(JobKind::Summarize), Err(Refused::Text));
}

#[test]
fn text_is_one_clean_line() {
    let a = answer(Some("primary"), Some(" Line one\nline\u{0}two\u{202e}  three "));
    let s = a.checked(JobKind::Summarize).ok().and_then(|c| c.summary).map(|s| s.text);
    assert_eq!(s.as_deref(), Some("Line one line two three"));
}

#[test]
fn due_dates_must_exist() {
    let mut a = answer(Some("primary"), Some("Pay by then."));
    a.due = Some("2026-11-14".into());
    assert_eq!(a.checked(JobKind::Summarize).ok().and_then(|c| c.summary).and_then(|s| s.due), Date::new(2026, 11, 14));
    for bad in ["2026-02-30", "26-11-14", "2026-1-4", "tomorrow", "2026-11-14T00:00"] {
        a.due = Some(bad.into());
        assert_eq!(a.checked(JobKind::Summarize), Err(Refused::Due), "{bad}");
    }
    a.due = Some("null".into());
    assert!(a.checked(JobKind::Summarize).is_ok_and(|c| c.summary.is_some_and(|s| s.due.is_none())));
}

#[test]
fn content_hash_is_stable_and_tells_texts_apart() {
    // Fixed: a kept insight must still match after an update of the program.
    assert_eq!(content_hash(""), "cbf29ce484222325".to_string() + "6c62272e07bb0142");
    assert_eq!(content_hash("hello"), content_hash("hello"));
    assert_ne!(content_hash("hello"), content_hash("hellp"));
    assert_eq!(content_hash("x").len(), 32);
}

#[test]
fn messages_without_an_insight_are_written_as_before() {
    let m = Message { subject: "Hi".into(), ..Default::default() };
    let json = serde_json::to_string(&m).unwrap_or_default();
    assert!(!json.contains("insight"), "{json}");
    // An older file reads, and a damaged insight doesn't stop the message from reading.
    let back: Message = serde_json::from_str(&json).unwrap_or_default();
    assert_eq!(back.subject, "Hi");
    let damaged = json.replacen('{', r#"{"insight": {"category": 7},"#, 1);
    let read: Message = serde_json::from_str(&damaged).unwrap_or_default();
    assert_eq!(read.subject, "Hi");
    assert!(read.insight.is_none());
}

#[test]
fn an_insight_round_trips() {
    let insight = Insight {
        category: Some(InboxCategory::Transactions),
        category_source: CategorySource::Owner,
        summary: Some(Summary { text: "Your order shipped.".into(), actions: vec![], due: Date::new(2026, 12, 1) }),
        model_id: "test".into(),
        content_hash: content_hash("x"),
        ..Default::default()
    };
    let m = Message { insight: Some(Box::new(insight.clone())), ..Default::default() };
    let back: Message = serde_json::from_str(&serde_json::to_string(&m).unwrap_or_default()).unwrap_or_default();
    assert_eq!(back.insight.as_deref(), Some(&insight));
    assert!(insight.owner_said());
}

#[test]
fn sender_rules_are_tidied_and_capped() {
    let mut mb = Mailbox::default();
    mb.ai_sender_rules.insert("  Ada@Example.ORG ".into(), InboxCategory::Primary);
    mb.ai_sender_rules.insert(" ".into(), InboxCategory::Updates);
    for i in 0..MAX_SENDER_RULES + 5 {
        mb.ai_sender_rules.insert(format!("n{i}@example.org"), InboxCategory::Promotions);
    }
    mb.repair();
    assert!(mb.ai_sender_rules.len() <= MAX_SENDER_RULES);
    assert!(!mb.ai_sender_rules.contains_key(""));
    assert!(mb.ai_sender_rules.keys().all(|k| *k == k.trim().to_lowercase()));
    // A file whose rules hold something unknown still opens, without them.
    let mb: Mailbox = serde_json::from_str(r#"{"aiSenderRules": {"a@example.org": "social"}}"#).unwrap_or_default();
    assert!(mb.ai_sender_rules.is_empty());
}
