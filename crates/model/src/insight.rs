// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What a model on this computer made of a message: which part of the Inbox it belongs in
//! ([`InboxCategory`]) and a short summary ([`Summary`]). Kept on the message ([`Insight`]), on this
//! computer only: it never goes to a server, into an exported file, or into a reply or forward.
//!
//! The work itself is described here too ([`Job`], [`JobResult`]): the engine says what a model is
//! to read (`ai.request`), a host runs a model on it (the desktop app's worker, or another
//! platform's own model) and hands back what came of it, which the engine checks and keeps
//! (`ai.apply`). What a model writes is untrusted, like the mail it read: [`Answer::checked`] is
//! the one gate it passes, and it is only ever shown as plain text.

use serde::{Deserialize, Serialize};

use crate::Id;
use crate::time::Date;

/// The most characters a summary's text keeps.
pub const MAX_SUMMARY_CHARS: usize = 600;
/// The most action items a summary keeps, and the most characters of each.
pub const MAX_ACTIONS: usize = 5;
pub const MAX_ACTION_CHARS: usize = 120;
/// The most characters of the name of the model that wrote an insight.
pub const MAX_MODEL_ID: usize = 80;
/// The most senders the owner may have told which part of the Inbox their mail goes to.
pub const MAX_SENDER_RULES: usize = 10_000;
/// How many numbers a message's or a search's meaning is kept as (an embedding model's first
/// ones, which such models make to stand alone).
pub const EMBED_DIM: usize = 384;

/// One of the four parts of the Inbox (the smart Inbox).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum InboxCategory {
    /// From people, or addressed to the owner and needing attention.
    Primary,
    /// Receipts, orders, shipping, bookings, bills, security codes.
    Transactions,
    /// Newsletters, notifications, alerts, notices that need nothing done.
    Updates,
    /// Marketing, deals, offers.
    Promotions,
}

impl InboxCategory {
    /// The four, in the order the Inbox shows them.
    pub const ALL: [InboxCategory; 4] = [InboxCategory::Primary, InboxCategory::Transactions, InboxCategory::Updates, InboxCategory::Promotions];

    /// Its code, as files, commands, the search box and a model write it.
    pub fn code(self) -> &'static str {
        match self {
            InboxCategory::Primary => "primary",
            InboxCategory::Transactions => "transactions",
            InboxCategory::Updates => "updates",
            InboxCategory::Promotions => "promotions",
        }
    }

    /// From its code, whatever the case and the spaces round it; nothing else.
    pub fn parse(text: &str) -> Option<InboxCategory> {
        let text = text.trim();
        InboxCategory::ALL.into_iter().find(|c| c.code().eq_ignore_ascii_case(text))
    }

    /// Its name in English (the window translates it).
    pub fn name(self) -> &'static str {
        match self {
            InboxCategory::Primary => "Primary",
            InboxCategory::Transactions => "Transactions",
            InboxCategory::Updates => "Updates",
            InboxCategory::Promotions => "Promotions",
        }
    }
}

/// Who put a message in its part of the Inbox.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum CategorySource {
    /// A model on this computer.
    #[default]
    Model,
    /// A fixed rule (the owner wrote to the sender before, a known contact, a bulk mailing).
    Rule,
    /// The owner, by moving it or by naming its sender. Never changed by anything else.
    Owner,
}

/// A short summary of a message or a conversation: plain text, never markup.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Summary {
    /// One to three sentences.
    pub text: String,
    /// What the mail asks for, in so many words.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    /// By when, when the mail says.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<Date>,
}

/// A conversation's summary, kept on its newest message.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct ThreadSummary {
    pub summary: Summary,
    /// [`content_hash`] of what the model read: the conversation as it was.
    pub content_hash: String,
}

/// What a model made of one message ([`crate::Message::insight`]). Local only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Insight {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<InboxCategory>,
    pub category_source: CategorySource,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<Summary>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadSummary>,
    /// The model that wrote it (empty for a rule's or the owner's word).
    #[serde(skip_serializing_if = "String::is_empty")]
    pub model_id: String,
    /// [`content_hash`] of what the model read: when the message's text changes, it is read again.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub content_hash: String,
    /// Summarising it failed, and why, in English (shown on hover). Cleared by a later success.
    #[serde(skip_serializing_if = "String::is_empty")]
    pub failed: String,
    /// Whether the model found it time-sensitive (something due today or tomorrow, a request to
    /// act, a trip or a delivery today): the Priority section at the top of Primary. `None`:
    /// not asked yet (mail sorted before the model was asked this, or placed by a rule).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<bool>,
    /// The owner's word on it (`ai.setPriority`): `Some(false)` took it out of Priority,
    /// `Some(true)` put it there. Never changed by anything else.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority_owner: Option<bool>,
}

impl Insight {
    /// Whether nothing is kept.
    pub fn is_empty(&self) -> bool {
        self.category.is_none()
            && self.summary.is_none()
            && self.thread.is_none()
            && self.failed.is_empty()
            && self.priority.is_none()
            && self.priority_owner.is_none()
    }

    /// The owner said where it goes: nothing else changes that.
    pub fn owner_said(&self) -> bool {
        self.category.is_some() && self.category_source == CategorySource::Owner
    }
}

/// A short, stable fingerprint of `text` (two FNV-1a hashes, 32 hex digits): the same in every
/// build and on every platform, so a kept insight knows the text it was made from. Not for
/// security.
pub fn content_hash(text: &str) -> String {
    let mut a: u64 = 0xcbf2_9ce4_8422_2325;
    let mut b: u64 = 0x6c62_272e_07bb_0142;
    for byte in text.bytes() {
        a = (a ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3);
        b = (b ^ u64::from(byte.rotate_left(3))).wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{a:016x}{b:016x}")
}

/// What a model is asked to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum JobKind {
    /// Which part of the Inbox it belongs in, alone (a short message).
    Classify,
    /// Its part of the Inbox and a summary, in one answer.
    Summarize,
    /// A summary of the conversation the message is the newest of.
    SummarizeThread,
    /// The meaning of a message as numbers ([`EMBED_DIM`]), for search by meaning.
    Embed,
    /// The meaning of a search's words (the job's `id` is 0).
    EmbedQuery,
}

impl JobKind {
    pub fn code(self) -> &'static str {
        match self {
            JobKind::Classify => "classify",
            JobKind::Summarize => "summarize",
            JobKind::SummarizeThread => "summarizeThread",
            JobKind::Embed => "embed",
            JobKind::EmbedQuery => "embedQuery",
        }
    }

    /// Whether it is for search by meaning (an embedding model's) rather than the smart Inbox.
    pub fn embeds(self) -> bool {
        matches!(self, JobKind::Embed | JobKind::EmbedQuery)
    }
}

/// How soon a job is wanted: the one asked for first.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum Priority {
    /// The owner opened it or asked for it.
    Now,
    /// Mail that just came in.
    New,
    /// Mail that was here before.
    Backfill,
}

/// One piece of work for a model: what it reads, as the engine wrote it ([`Job::input`]: the
/// message's fields as plain lines, its text cut to fit), and how to answer.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Job {
    /// The message (for a conversation, its newest message).
    pub id: Id,
    pub kind: JobKind,
    pub priority: Priority,
    /// The text the model reads. Mail is untrusted: whoever builds the prompt says so.
    pub input: String,
    /// [`content_hash`] of `input`: handed back, so a result for text since changed is let go.
    pub content_hash: String,
    /// Today, for "by Friday".
    pub today: Date,
}

/// What came of a [`Job`].
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct JobResult {
    pub id: Id,
    pub kind: JobKind,
    pub content_hash: String,
    #[serde(flatten)]
    pub outcome: Outcome,
    /// The model that answered.
    #[serde(default)]
    pub model_id: String,
    /// How long it took.
    #[serde(default)]
    pub millis: u32,
}

/// A model's answer, or why there is none.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", tag = "outcome")]
pub enum Outcome {
    Done(Answer),
    /// It failed (in English; never the model's own words).
    Failed {
        reason: String,
    },
}

/// A model's answer as it wrote it (JSON with these fields), before [`Answer::checked`].
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Answer {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub category: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub due: Option<String>,
    /// Whether it is time-sensitive (see [`Insight::priority`]).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub priority: Option<bool>,
    /// For an embedding: its numbers.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub vector: Vec<f32>,
}

/// An [`Answer`] that passed [`Answer::checked`].
#[derive(Clone, Debug, PartialEq)]
pub struct Checked {
    pub category: Option<InboxCategory>,
    pub summary: Option<Summary>,
    pub priority: Option<bool>,
    /// For an embedding: [`EMBED_DIM`] numbers, of length 1.
    pub vector: Vec<f32>,
}

/// Why an answer was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Refused {
    /// A category that isn't one of the four.
    Category,
    /// A kind of job whose answer lacks what it must hold.
    Missing,
    /// Text over its length, or that isn't text to show.
    Text,
    /// A due date that isn't `YYYY-MM-DD`, or doesn't exist.
    Due,
    /// An embedding of another length, or with numbers that aren't.
    Vector,
}

impl Refused {
    pub fn reason(self) -> &'static str {
        match self {
            Refused::Category => "the model named a category that doesn't exist",
            Refused::Missing => "the model's answer was incomplete",
            Refused::Text => "the model's answer was too long or not plain text",
            Refused::Due => "the model gave a date that doesn't exist",
            Refused::Vector => "the embedding model's answer wasn't one",
        }
    }
}

impl Answer {
    /// The answer, if it holds what `kind` asks for and nothing out of bounds: a category of the
    /// four, a summary of at most [`MAX_SUMMARY_CHARS`] characters, at most [`MAX_ACTIONS`]
    /// actions of at most [`MAX_ACTION_CHARS`] each, a due date that exists. Text has its control
    /// characters (but for none) and format characters taken out and is trimmed; nothing is cut
    /// to fit: an answer over a limit is refused, never shown in part.
    pub fn checked(&self, kind: JobKind) -> Result<Checked, Refused> {
        if kind.embeds() {
            let v = &self.vector;
            if v.len() != EMBED_DIM || !v.iter().all(|x| x.is_finite()) {
                return Err(Refused::Vector);
            }
            let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
            if norm.is_nan() || norm <= 1e-6 {
                return Err(Refused::Vector);
            }
            return Ok(Checked { category: None, summary: None, priority: None, vector: v.iter().map(|x| x / norm).collect() });
        }
        let category = match self.category.as_deref() {
            Some(c) => Some(InboxCategory::parse(c).ok_or(Refused::Category)?),
            None => None,
        };
        let summary = match self.summary.as_deref() {
            Some(text) => {
                let text = clean(text, MAX_SUMMARY_CHARS)?;
                if self.actions.len() > MAX_ACTIONS {
                    return Err(Refused::Text);
                }
                let mut actions = Vec::with_capacity(self.actions.len());
                for a in &self.actions {
                    let a = clean(a, MAX_ACTION_CHARS)?;
                    if !a.is_empty() {
                        actions.push(a);
                    }
                }
                let due = match self.due.as_deref().map(str::trim).filter(|d| !d.is_empty() && *d != "null") {
                    Some(d) => Some(parse_day(d).ok_or(Refused::Due)?),
                    None => None,
                };
                (!text.is_empty()).then_some(Summary { text, actions, due })
            }
            None => None,
        };
        let complete = match kind {
            JobKind::Classify => category.is_some(),
            JobKind::Summarize => category.is_some() && summary.is_some(),
            JobKind::SummarizeThread => summary.is_some(),
            JobKind::Embed | JobKind::EmbedQuery => false,
        };
        if !complete {
            return Err(Refused::Missing);
        }
        Ok(Checked { category, summary, priority: self.priority, vector: Vec::new() })
    }
}

/// `text` on one line, without control or format characters, trimmed; refused over `max`
/// characters.
fn clean(text: &str, max: usize) -> Result<String, Refused> {
    let out: String = text
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .filter(|c| !mailcraft_names::is_format(*c))
        .collect::<String>()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    if out.chars().count() > max {
        return Err(Refused::Text);
    }
    Ok(out)
}

/// A day written `YYYY-MM-DD`, when it exists.
pub fn parse_day(text: &str) -> Option<Date> {
    let mut parts = text.trim().splitn(3, '-');
    let (y, m, d) = (parts.next()?, parts.next()?, parts.next()?);
    if y.len() != 4 || m.len() != 2 || d.len() != 2 || !(y.bytes().chain(m.bytes()).chain(d.bytes())).all(|b| b.is_ascii_digit()) {
        return None;
    }
    Date::new(y.parse().ok()?, m.parse().ok()?, d.parse().ok()?)
}

#[cfg(test)]
#[path = "insight_tests.rs"]
mod insight_tests;
