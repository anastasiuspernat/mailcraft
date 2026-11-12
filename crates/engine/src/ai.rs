// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The smart Inbox and summaries, as far as the engine goes: what a model is to read and in what
//! order ([`next_jobs`], `ai.request`), and taking in what it made of it ([`apply`], `ai.apply`).
//! The engine runs no model: a host does (the desktop app's worker process; another platform's
//! own model), the same way it does a Send/Receive.
//!
//! Before a model is asked, a message may be placed by the owner's word (a sender they named,
//! [`mailcraft_model::Mailbox::ai_sender_rules`]) or by a fixed rule ([`rule_for`]): a sender the
//! owner knows goes to Primary, a bulk mailing that is plainly marketing to Promotions. Mail in Junk
//! E-mail, Drafts or anywhere but an Inbox is never looked at.
//!
//! What a model reads is built here ([`input`]): parsed fields, never raw MIME, never what the
//! sender hid (`hidden_text`), the text without quoted replies and signatures, cut to fit. It is
//! untrusted data, and the host says so to the model.

use std::collections::{HashSet, VecDeque};

use mailcraft_model::insight::{self, CategorySource, InboxCategory, Insight, Job, JobKind, JobResult, Outcome, Priority, ThreadSummary};
use mailcraft_model::{FolderKind, Id, Message};
use serde::{Deserialize, Serialize};

use crate::Session;

/// Characters of a message's text a model reads (about 1,500 tokens).
pub const BODY_CHARS: usize = 6_000;
/// Characters of each older message of a conversation (about 200 tokens), and of the whole.
pub const THREAD_OLDER_CHARS: usize = 800;
pub const THREAD_CHARS: usize = 12_000;
/// Most messages of a conversation read for its summary.
pub const THREAD_MESSAGES: usize = 12;
/// Most jobs one `ai.request` hands out.
pub const MAX_BATCH: usize = 32;
/// Most jobs waiting because the owner asked.
pub const MAX_ON_DEMAND: usize = 64;
/// Most groups of the list shown whole at once (`view.smartGroup`).
pub const MAX_OPEN_GROUPS: usize = 64;
/// Rows the Priority section shows before "n more from …".
pub const PRIORITY_ROWS: usize = 3;

/// The AI settings (File › Options › AI; `ai.options`). Kept with the window's settings, never in
/// the mailbox file.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct AiPrefs {
    /// Off until a model is chosen.
    pub enabled: bool,
    /// The model file (GGUF) the host loads. Empty: none chosen.
    pub model_path: String,
    /// `auto`: the graphics card when there is one that works; `off`: the processor only.
    pub gpu: GpuChoice,
    /// New mail is summarised as it comes; off, only when asked.
    pub summarize_new: bool,
    /// Messages with fewer words get no summary.
    pub summary_min_words: u32,
    /// How many days back mail that was already here is sorted and summarised.
    pub backfill_days: u32,
    /// Minutes without work after which the model is let go of (memory, the graphics card's).
    pub idle_unload_minutes: u32,
    /// When the owner turned it on: mail that came after is new, before is backfill.
    pub enabled_at: i64,
    /// A summary's first sentence in place of the preview in the message list, and summaries
    /// made for unread mail however short ("Show summaries in the message list").
    pub list_summaries: bool,
    /// Search by meaning: the embedding model (GGUF) the host loads for it. Empty: none, and a
    /// search finds words alone.
    pub search_model_path: String,
    /// Search by meaning is on (once an embedding model is chosen).
    pub search: bool,
}

impl Default for AiPrefs {
    fn default() -> Self {
        AiPrefs {
            enabled: false,
            model_path: String::new(),
            gpu: GpuChoice::Auto,
            summarize_new: true,
            summary_min_words: 120,
            backfill_days: 30,
            idle_unload_minutes: 5,
            enabled_at: 0,
            list_summaries: true,
            search_model_path: String::new(),
            search: true,
        }
    }
}

impl AiPrefs {
    pub fn is_default(&self) -> bool {
        *self == AiPrefs::default()
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuChoice {
    #[default]
    Auto,
    Off,
}

/// What the host says of its model (`ai.status`): set by the host, never saved.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct HostStatus {
    /// Whether this build or platform can run a model at all.
    pub available: bool,
    /// `vulkan`, `metal`, `cpu`, or empty while none is loaded.
    pub backend: String,
    /// The model's name, while one is loaded.
    pub model: String,
    pub loaded: bool,
    /// The last thing that went wrong, in English.
    pub error: String,
    /// What the graphics card's self-test said (`passed`, `failed: …`, empty when not run).
    pub self_test: String,
    /// How long the last job took.
    pub last_millis: u32,
    /// Jobs the host holds and hasn't answered yet.
    pub working: usize,
}

/// The engine's side of the work: what was asked for, what is out with the host. Never saved.
#[derive(Debug, Default)]
pub struct AiState {
    /// Asked for by the owner (a message or conversation opened, Summarize): first.
    pub on_demand: VecDeque<(Id, JobKind)>,
    /// Handed to the host and not answered yet.
    pub in_flight: HashSet<(Id, JobKind)>,
    /// Failed this session: not asked again until the owner asks.
    pub failed: HashSet<(Id, JobKind)>,
    pub host: HostStatus,
    /// Messages left to sort and summarise in the backfill window, as last counted.
    pub backlog: usize,
    /// Search by meaning: the words of a search whose meaning is to be asked for next.
    pub query_wanted: Option<String>,
    /// The searches whose meaning was asked for, newest first (at most [`MAX_QUERIES`]).
    pub queries_asked: VecDeque<String>,
    /// The meanings of the latest searches, newest first (at most [`MAX_QUERIES`]).
    pub queries: VecDeque<(String, std::sync::Arc<Vec<f32>>)>,
    /// Counts each meaning of a search that came: the list is built again for it.
    pub meaning_rev: u64,
    /// The meaning index ([`crate::vectors`]), opened for the mailbox file it is beside.
    pub vectors: Option<crate::vectors::Vectors>,
    pub vectors_for: Option<Option<std::path::PathBuf>>,
    /// Messages left to read for search by meaning, as last counted.
    pub embed_backlog: usize,
}

/// How many searches' meanings are kept.
pub const MAX_QUERIES: usize = 64;
/// Characters of a message's text an embedding reads (about 500 tokens).
pub const EMBED_CHARS: usize = 2_000;
/// How close a message must be to a search's meaning to be found by it (from -1 to 1), and the
/// most found that way: measured with Qwen3-Embedding, where mail that means the search scores
/// about 0.75 and other mail 0.2 to 0.45.
pub const MEANING_FLOOR: f32 = 0.55;
pub const MEANING_TOP: usize = 50;

/// Whether search by meaning is on: an embedding model chosen, and the switch on.
pub fn searches_by_meaning(s: &Session) -> bool {
    s.prefs.ai.search && !s.prefs.ai.search_model_path.is_empty()
}

/// The meaning index for the mailbox as it is kept now: opened beside its file (in memory for a
/// mailbox kept in none), with what the mailbox no longer has taken out.
pub fn vectors(s: &mut Session) -> &mut crate::vectors::Vectors {
    let path = s.path.clone();
    if s.ai.vectors.is_none() || s.ai.vectors_for.as_ref() != Some(&path) {
        let mut v = open_vectors(path.as_deref());
        let ids: HashSet<Id> = s.mailbox.messages.values().map(|m| m.id).collect();
        if v.retain(|id| ids.contains(&id)) > 0 {
            #[cfg(not(target_arch = "wasm32"))]
            v.flush();
        }
        s.ai.vectors = Some(v);
        s.ai.vectors_for = Some(path);
    }
    s.ai.vectors.get_or_insert_with(crate::vectors::Vectors::in_memory)
}

fn open_vectors(path: Option<&std::path::Path>) -> crate::vectors::Vectors {
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(p) = path {
        return crate::vectors::Vectors::open(p);
    }
    let _ = path;
    crate::vectors::Vectors::in_memory()
}

/// What an embedding reads of `m`: its subject, who it is from, and its own text (without quotes
/// and signature) cut to [`EMBED_CHARS`]. Never what the sender hid, attachments or headers. Its
/// body must be in memory.
pub fn embed_input(m: &Message) -> String {
    let who = if m.from.name.trim().is_empty() { &m.from.email } else { &m.from.name };
    format!("Subject: {}\nFrom: {}\n{}", line(&m.subject, 300), line(who, 120), cut(&strip_quotes(&m.body.text), EMBED_CHARS))
}

/// The words of the search in view that search by meaning reads: its plain words, when search by
/// meaning is on and the search is of at least two words or four letters, with no quoted phrase
/// and no address in it (those are found by their words alone).
pub fn meaning_text(s: &Session) -> Option<String> {
    if !searches_by_meaning(s) {
        return None;
    }
    let search = s.view.search.trim();
    if search.contains('"') || search.contains('@') {
        return None;
    }
    let text = mailcraft_model::search::Query::parse(search).free_text();
    let words = text.split_whitespace().count();
    (words >= 2 || text.chars().filter(|c| c.is_alphanumeric()).count() >= 4).then_some(text)
}

/// The meaning of the search `text`, when it came.
pub fn query_vector(s: &Session, text: &str) -> Option<std::sync::Arc<Vec<f32>>> {
    s.ai.queries.iter().find(|(t, _)| t == text).map(|(_, v)| v.clone())
}

/// Ask for the meaning of the search in view, when it is wanted and not asked for yet. Whether it
/// was asked for.
pub fn ask_meaning(s: &mut Session) -> bool {
    let Some(text) = meaning_text(s) else { return false };
    if query_vector(s, &text).is_some() || s.ai.queries_asked.contains(&text) {
        return false;
    }
    if s.ai.queries_asked.len() >= MAX_QUERIES {
        s.ai.queries_asked.pop_back();
    }
    s.ai.queries_asked.push_front(text.clone());
    s.ai.query_wanted = Some(text);
    true
}

/// The messages of `looked` that the search in view finds by meaning: those that pass its other
/// terms (`from:`, dates, `is:`…), whose meaning is at least [`MEANING_FLOOR`] close to the
/// search's, the [`MEANING_TOP`] closest. Empty when search by meaning is off, the search's
/// meaning hasn't come, or nothing is indexed.
pub fn meaning_hits(s: &Session, looked: &[&Message]) -> HashSet<Id> {
    let Some(text) = meaning_text(s) else { return HashSet::new() };
    let (Some(query), Some(vectors)) = (query_vector(s, &text), s.ai.vectors.as_ref()) else { return HashSet::new() };
    if vectors.is_empty() {
        return HashSet::new();
    }
    let filters = s.search_query().without_free_text();
    let passing = looked.iter().filter(|m| filters.is_empty() || filters.matches_in(m, |at| s.offset_at(at))).map(|m| m.id);
    let mut scored = vectors.scores(&query, passing);
    scored.retain(|(_, score)| *score >= MEANING_FLOOR);
    scored.sort_by(|a, b| b.1.total_cmp(&a.1).then(a.0.cmp(&b.0)));
    scored.into_iter().take(MEANING_TOP).map(|(id, _)| id).collect()
}

/// Whether `m` is mail the smart Inbox sorts: in an Inbox, not a draft, its account not a shared
/// mailbox's Junk.
pub fn sorts(s: &Session, m: &Message) -> bool {
    !m.draft && s.mailbox.folder(m.folder).is_some_and(|f| f.kind == FolderKind::Inbox)
}

/// The part of the Inbox `m` shows in: its insight's, else Primary (until it is sorted).
pub fn category_of(m: &Message) -> InboxCategory {
    m.insight.as_ref().and_then(|i| i.category).unwrap_or(InboxCategory::Primary)
}

/// Whether the list in view shows one part of the smart Inbox: one is chosen
/// ([`crate::ViewState::inbox_category`]), the smart Inbox is on, the list is of an Inbox or of
/// All Inboxes, and Gmail's own tabs aren't showing one of theirs there.
pub fn in_view(s: &Session) -> Option<InboxCategory> {
    let category = s.view.inbox_category?;
    (s.prefs.ai.enabled && lists_inboxes(s) && crate::gmail::in_view(s).is_none()).then_some(category)
}

/// Whether the list in view is the smart Inbox's All Mail: the whole Inbox (or All Inboxes) while
/// the smart Inbox is on, with no part chosen. Its Priority section is Primary's.
pub fn all_mail_in_view(s: &Session) -> bool {
    s.view.inbox_category.is_none() && s.prefs.ai.enabled && lists_inboxes(s) && crate::gmail::in_view(s).is_none()
}

/// Whether the smart Inbox is still reading mail for the list in view (sorting it, asking whether
/// it is a priority) while that list has the Priority box (Primary, All Mail): the box shows
/// then, saying "Reading mail…" while nothing is in it yet. Not while the model can't run.
pub fn reading_mail(s: &Session) -> bool {
    let shows = in_view(s) == Some(InboxCategory::Primary) || all_mail_in_view(s);
    let sorting = s.ai.in_flight.iter().any(|(_, kind)| matches!(kind, JobKind::Classify | JobKind::Summarize));
    shows && s.ai.host.available && s.ai.host.error.is_empty() && (sorting || s.ai.backlog > 0)
}

/// Whether the list in view is of an Inbox, or of All Inboxes ([`crate::all`]): what the smart
/// Inbox's tabs go over.
pub fn lists_inboxes(s: &Session) -> bool {
    match crate::all::in_view(s) {
        Some(which) => which == crate::all::AllMailbox::Inboxes,
        None => s.mailbox.folder(s.view.folder).is_some_and(|f| f.kind == FolderKind::Inbox),
    }
}

/// The Inboxes the list in view is of: every account's for All Inboxes, else the folder in view.
pub fn inboxes_in_view(s: &Session) -> HashSet<Id> {
    match crate::all::in_view(s) {
        Some(crate::all::AllMailbox::Inboxes) => crate::all::folders(&s.mailbox, crate::all::AllMailbox::Inboxes),
        _ => HashSet::from([s.view.folder]),
    }
}

/// What the list in view keeps of an Inbox while it shows one part of the smart Inbox: the
/// Inbox's messages in that part, and in conversation view every message of a conversation one
/// of them is in. Mail outside the Inbox (a search of more than it) stays as it is.
pub(crate) struct Shown {
    folders: HashSet<Id>,
    category: InboxCategory,
    threads: Option<(std::sync::Arc<mailcraft_model::ThreadIndex>, HashSet<(Id, mailcraft_model::ThreadKey)>)>,
}

impl Shown {
    pub(crate) fn of<'a>(s: &Session, messages: impl Iterator<Item = &'a Message>) -> Option<Shown> {
        let category = in_view(s)?;
        let folders = inboxes_in_view(s);
        let threads = s.view.conversations.then(|| {
            let index = s.threads();
            let keys = messages.filter(|m| folders.contains(&m.folder) && category_of(m) == category).map(|m| (m.account, index.key(m.id))).collect();
            (index, keys)
        });
        Some(Shown { folders, category, threads })
    }

    pub(crate) fn keeps(&self, m: &Message) -> bool {
        if !self.folders.contains(&m.folder) || category_of(m) == self.category {
            return true;
        }
        self.threads.as_ref().is_some_and(|(index, keys)| !keys.is_empty() && keys.contains(&(m.account, index.key(m.id))))
    }
}

/// A header of `m` by name, whatever its case.
fn header<'a>(m: &'a Message, name: &str) -> Option<&'a str> {
    m.headers.iter().find(|(k, _)| k.eq_ignore_ascii_case(name)).map(|(_, v)| v.as_str())
}

/// What the headers of `m` say of how it was sent.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Bulk {
    pub unsubscribe: bool,
    /// `bulk`, `list` or `junk` precedence.
    pub precedence: Option<&'static str>,
    pub auto_submitted: bool,
}

impl Bulk {
    pub fn of(m: &Message) -> Bulk {
        let precedence = header(m, "Precedence").map(|p| p.trim().to_ascii_lowercase()).and_then(|p| match p.as_str() {
            "bulk" => Some("bulk"),
            "list" => Some("list"),
            "junk" => Some("junk"),
            _ => None,
        });
        let auto_submitted = header(m, "Auto-Submitted").is_some_and(|v| !v.trim().eq_ignore_ascii_case("no"));
        Bulk { unsubscribe: header(m, "List-Unsubscribe").is_some(), precedence, auto_submitted }
    }

    pub fn any(self) -> bool {
        self.unsubscribe || self.precedence.is_some() || self.auto_submitted
    }
}

/// Words of a subject that say marketing, for the Promotions rule.
const MARKETING: &[&str] = &[
    "sale",
    "% off",
    "discount",
    "deal",
    "deals",
    "offer",
    "offers",
    "coupon",
    "promo",
    "save ",
    "free shipping",
    "limited time",
    "black friday",
    "shop now",
];

/// The part of the Inbox a fixed rule puts `m` in, if one does ([`CategorySource::Rule`]): a sender
/// the owner knows (a contact, or mail in Sent Items went to them) that isn't a bulk mailing is
/// Primary; a bulk mailing with a way to unsubscribe and a subject that says marketing is
/// Promotions. Everything else is for the model.
pub fn rule_for(s: &mut Session, m: &Message) -> Option<InboxCategory> {
    let bulk = Bulk::of(m);
    let address = m.from.email.trim().to_lowercase();
    if !bulk.any() && !address.is_empty() && s.knows(&address) {
        return Some(InboxCategory::Primary);
    }
    if bulk.unsubscribe && bulk.precedence.is_some() {
        let subject = m.subject.to_lowercase();
        if MARKETING.iter().any(|w| subject.contains(w)) {
            return Some(InboxCategory::Promotions);
        }
    }
    None
}

/// `text` without the quoted reply below it (lines starting `>`, and from an "On … wrote:" line
/// or an "-----Original Message-----" on) and without the signature (from a `-- ` line on).
pub fn strip_quotes(text: &str) -> String {
    let mut out = String::with_capacity(text.len().min(BODY_CHARS * 2));
    for line in text.lines() {
        let t = line.trim_end();
        if t == "--" || t == "-- " || line == "-- " {
            break;
        }
        let lower = t.trim().to_lowercase();
        if lower.starts_with("-----original message") || lower.starts_with("________________________________") {
            break;
        }
        if (lower.starts_with("on ") && lower.ends_with("wrote:")) || (lower.starts_with("from:") && out.lines().count() > 2) {
            break;
        }
        if t.trim_start().starts_with('>') {
            continue;
        }
        out.push_str(t);
        out.push('\n');
        if out.len() > BODY_CHARS * 2 {
            break;
        }
    }
    out.trim().to_string()
}

/// The first `max` characters of `text`, at a character boundary.
pub fn cut(text: &str, max: usize) -> &str {
    text.char_indices().nth(max).and_then(|(at, _)| text.get(..at)).unwrap_or(text)
}

/// How many words a message's own text (without quotes and signature) has.
pub fn words_of(m: &Message) -> usize {
    strip_quotes(&m.body.text).split_whitespace().count()
}

/// One line without control characters (a name or subject from a hostile message).
fn line(text: &str, max: usize) -> String {
    let one: String = text.chars().map(|c| if c.is_control() { ' ' } else { c }).collect();
    cut(one.split_whitespace().collect::<Vec<_>>().join(" ").as_str(), max).to_string()
}

/// What a model reads of `m` (spec: parsed fields, the owner's own addresses only as whether
/// they are in To, the text cut to [`BODY_CHARS`]). Its body must be in memory.
pub fn input(s: &mut Session, m: &Message, body_chars: usize) -> String {
    let owner: HashSet<String> = s.mailbox.accounts.iter().map(|a| a.email.trim().to_lowercase()).filter(|e| !e.is_empty()).collect();
    let to_owner = m.to.iter().any(|a| owner.contains(&a.email.trim().to_lowercase()));
    let bulk = Bulk::of(m);
    let address = m.from.email.trim().to_lowercase();
    let known = !address.is_empty() && s.mailbox.contact_by_email(&address).is_some();
    let replied = !address.is_empty() && s.knows(&address);
    let body = strip_quotes(&m.body.text);
    format!(
        "From: {} <{}>\nTo/Cc count: {} (owner in To: {})\nSubject: {}\nHeaders: list-unsubscribe={}, precedence={}, auto-submitted={}\nKnown contact: {}   Owner has written to this sender before: {}\nBody:\n{}",
        line(&m.from.name, 120),
        line(&m.from.email, 200),
        m.to.len().saturating_add(m.cc.len()),
        yes(to_owner),
        line(&m.subject, 300),
        yes(bulk.unsubscribe),
        bulk.precedence.unwrap_or("none"),
        yes(bulk.auto_submitted),
        yes(known),
        yes(replied),
        cut(&body, body_chars),
    )
}

fn yes(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

/// What a model reads of the conversation `newest` is the newest of: that message whole, the
/// older ones' beginnings, oldest first, within [`THREAD_CHARS`].
pub fn thread_input(s: &mut Session, newest: Id) -> Option<String> {
    let index = s.threads();
    let mut members: Vec<(i64, Id)> = index.members(newest).into_iter().filter_map(|id| s.mailbox.message(id).map(|m| (m.date, id))).collect();
    members.sort();
    let take = members.len().saturating_sub(THREAD_MESSAGES);
    let members: Vec<Id> = members.into_iter().skip(take).map(|(_, id)| id).collect();
    let mut out = String::new();
    let count = members.len();
    for (i, id) in members.into_iter().enumerate() {
        let _ = s.load_where(id, |_| false);
        let m = s.mailbox.message(id)?.clone();
        let last = i + 1 == count;
        let room = if last { THREAD_CHARS.saturating_sub(out.len()).max(BODY_CHARS / 2) } else { THREAD_OLDER_CHARS };
        out.push_str(&format!("--- Message {} of {count} ---\n", i + 1));
        out.push_str(&input(s, &m, room));
        out.push_str("\n\n");
        if out.len() > THREAD_CHARS * 2 {
            break;
        }
    }
    (!out.is_empty()).then_some(out)
}

/// The jobs to hand a host next, at most `max` ([`MAX_BATCH`]), the most wanted first: what the
/// owner asked for, then new mail, then mail from the backfill window, newest first. Messages a
/// rule or the owner's word places are placed here and then, without a model; a message already
/// out with the host, or that failed this session, is not handed out again.
pub fn next_jobs(s: &mut Session, max: usize) -> Vec<Job> {
    let max = max.clamp(1, MAX_BATCH);
    let today = s.today();
    let mut jobs: Vec<Job> = Vec::new();
    // 0. The meaning of the search in view.
    if let Some(text) = s.ai.query_wanted.take()
        && searches_by_meaning(s)
    {
        s.ai.in_flight.insert((0, JobKind::EmbedQuery));
        jobs.push(Job { id: 0, kind: JobKind::EmbedQuery, priority: Priority::Now, content_hash: insight::content_hash(&text), input: text, today });
    }
    // 1. Asked for.
    while jobs.len() < max {
        let Some((id, kind)) = s.ai.on_demand.pop_front() else { break };
        if s.ai.in_flight.contains(&(id, kind)) {
            continue;
        }
        if let Some(job) = job_for(s, id, kind, Priority::Now, today) {
            jobs.push(job);
        }
    }
    if s.prefs.ai.enabled {
        sort_jobs(s, max, today, &mut jobs);
    }
    // 4. Search by meaning: every message not read for it yet, newest first.
    if jobs.len() < max && searches_by_meaning(s) {
        embed_jobs(s, max, today, &mut jobs);
    }
    jobs
}

/// The messages to read for search by meaning, newest first, into `jobs` up to `max`.
fn embed_jobs(s: &mut Session, max: usize, today: mailcraft_model::time::Date, jobs: &mut Vec<Job>) {
    let _ = vectors(s);
    let Some(v) = s.ai.vectors.as_ref() else { return };
    let mut todo: Vec<(i64, Id)> = s
        .mailbox
        .messages
        .values()
        .filter(|m| !m.draft && !v.current(m.id, crate::vectors::fingerprint(m)))
        .map(|m| (crate::list::when(m), m.id))
        .collect();
    todo.sort_by(|a, b| b.cmp(a));
    s.ai.embed_backlog = todo.len();
    for (_, id) in todo {
        if jobs.len() >= max {
            break;
        }
        if s.ai.in_flight.contains(&(id, JobKind::Embed)) || s.ai.failed.contains(&(id, JobKind::Embed)) {
            continue;
        }
        let _ = s.load_where(id, |_| false);
        let Some(m) = s.mailbox.message(id) else { continue };
        let input = embed_input(m);
        s.ai.in_flight.insert((id, JobKind::Embed));
        jobs.push(Job { id, kind: JobKind::Embed, priority: Priority::Backfill, content_hash: insight::content_hash(&input), input, today });
    }
}

/// New mail, then the backfill window, newest first, to sort and summarise, into `jobs` up to
/// `max`.
fn sort_jobs(s: &mut Session, max: usize, today: mailcraft_model::time::Date, jobs: &mut Vec<Job>) {
    // 2 and 3. New mail, then the backfill window, newest first.
    let now = s.now();
    let since = now.saturating_sub(i64::from(s.prefs.ai.backfill_days).saturating_mul(86_400));
    let enabled_at = s.prefs.ai.enabled_at;
    let mut todo: Vec<(i64, Id)> =
        s.mailbox.messages.values().filter(|m| m.received >= since && needs_work(s, m)).filter(|m| sorts(s, m)).map(|m| (m.received, m.id)).collect();
    todo.sort_by(|a, b| b.cmp(a));
    s.ai.backlog = todo.len();
    for (received, id) in todo {
        if jobs.len() >= max {
            break;
        }
        let Some(m) = s.mailbox.message(id).cloned() else { continue };
        if place_without_model(s, &m) {
            s.ai.backlog = s.ai.backlog.saturating_sub(1);
            continue;
        }
        let kind = if wants_summary(s, &m) { JobKind::Summarize } else { JobKind::Classify };
        if s.ai.in_flight.contains(&(id, kind)) || s.ai.failed.contains(&(id, kind)) {
            continue;
        }
        let priority = if received >= enabled_at { Priority::New } else { Priority::Backfill };
        if let Some(job) = job_for(s, id, kind, priority, today) {
            jobs.push(job);
        }
    }
}

/// Whether `m` lacks what the smart Inbox keeps of it: a category, or (when summaries come for
/// new mail and it is long enough) a summary. A cheap test: no text is read.
fn needs_work(s: &Session, m: &Message) -> bool {
    let insight = m.insight.as_deref();
    let placed = insight.is_some_and(|i| i.category.is_some());
    let failed = insight.is_some_and(|i| !i.failed.is_empty());
    if !placed {
        return !failed;
    }
    if failed {
        return false;
    }
    // Sorted into Primary before the model was asked whether it is a priority, or by a rule.
    if asks_priority(m) {
        return true;
    }
    if !s.prefs.ai.summarize_new || m.unfetched {
        return false;
    }
    // The same test the job's kind is chosen by ([`wants_summary`]): a cheaper guess here that
    // disagreed with it would hand out a sort the message already has, again and again. (Only
    // sorted messages without a summary get this far, in the backfill window.)
    insight.is_some_and(|i| i.summary.is_none()) && m.body.text.len() >= summary_floor(s, m) && wants_summary(s, m)
}

/// Words below which an unread message gets a summary when summaries show in the list.
pub const LIST_SUMMARY_WORDS: usize = 12;

/// The fewest words `m` needs for a summary: [`AiPrefs::summary_min_words`], or for unread mail
/// while summaries show in the message list, [`LIST_SUMMARY_WORDS`].
fn summary_floor(s: &Session, m: &Message) -> usize {
    let min = s.prefs.ai.summary_min_words as usize;
    if s.prefs.ai.list_summaries && !m.read { min.min(LIST_SUMMARY_WORDS) } else { min }
}

/// Whether a summary is wanted for `m` as it is sorted: summaries for new mail on, the message
/// whole and long enough ([`summary_floor`]).
fn wants_summary(s: &Session, m: &Message) -> bool {
    s.prefs.ai.summarize_new && !m.unfetched && words_of(m) >= summary_floor(s, m)
}

/// Whether `m` is unread, in Primary, and the model hasn't said yet whether it is a priority (and
/// the owner hasn't either). Mail already read is never asked about.
fn asks_priority(m: &Message) -> bool {
    !m.read && m.insight.as_deref().is_some_and(|i| i.category == Some(InboxCategory::Primary) && i.priority.is_none() && i.priority_owner.is_none())
}

/// How long a priority without a due date stays one, in seconds.
pub const PRIORITY_FOR: i64 = 2 * 86_400;

/// Whether `m` is in the Priority section at the top of Primary: in Primary, and the owner put it
/// there, or it is unread mail from a VIP, or the model found it time-sensitive, the owner didn't take it out, and it is still
/// current: until the end of its due day, or for [`PRIORITY_FOR`] after it came when it has none.
pub fn is_priority(s: &Session, m: &Message) -> bool {
    let Some(i) = m.insight.as_deref() else { return false };
    if category_of(m) != InboxCategory::Primary || !sorts(s, m) {
        return false;
    }
    if let Some(owner) = i.priority_owner {
        return owner;
    }
    // A VIP's unread mail is always there (Always Show in Priority), however old.
    if !m.read && s.mailbox.is_vip(&m.from.email) {
        return true;
    }
    if i.priority != Some(true) {
        return false;
    }
    match i.summary.as_ref().and_then(|x| x.due) {
        Some(due) => due >= s.today(),
        None => crate::list::when(m) >= s.now().saturating_sub(PRIORITY_FOR),
    }
}

/// What of `m`'s insight the list is arranged by beyond its category: for telling whether a
/// change leaves the list as it was.
pub fn arranged_by(m: &Message) -> (Option<bool>, Option<bool>, bool) {
    let i = m.insight.as_deref();
    (i.and_then(|i| i.priority), i.and_then(|i| i.priority_owner), i.is_some_and(|i| i.summary.as_ref().and_then(|x| x.due).is_some()))
}

/// The owner puts message `id` in the Priority section (`true`) or takes it out (`false`); `None`
/// leaves it to the model again. Whether it changed.
pub fn set_priority(s: &mut Session, id: Id, priority: Option<bool>) -> bool {
    let Some(m) = s.mailbox.message(id) else { return false };
    if m.insight.as_deref().and_then(|i| i.priority_owner) == priority {
        return false;
    }
    let Ok(m) = s.mailbox.message_mut(id) else { return false };
    m.insight.get_or_insert_with(Default::default).priority_owner = priority;
    true
}

/// Place `m` by the owner's word for its sender or by a fixed rule, when one says, and when no
/// summary is wanted (one model call does both). Whether it was placed.
fn place_without_model(s: &mut Session, m: &Message) -> bool {
    let owner = s.mailbox.sender_rule(&m.from.email);
    let (category, source) = match owner {
        Some(c) => (c, CategorySource::Owner),
        // A VIP's mail is Primary.
        None if s.mailbox.is_vip(&m.from.email) => (InboxCategory::Primary, CategorySource::Rule),
        None => match rule_for(s, m) {
            Some(c) => (c, CategorySource::Rule),
            None => return false,
        },
    };
    let placed = m.insight.as_deref().is_some_and(|i| i.category.is_some());
    if !placed {
        set_category(s, m.id, category, source);
    }
    // Mail in Primary still wants the model's word on whether it is a priority, and a long
    // message its summary.
    let Some(now) = s.mailbox.message(m.id) else { return true };
    if asks_priority(now) {
        return false;
    }
    !(wants_summary(s, m) && m.insight.as_deref().is_none_or(|i| i.summary.is_none()))
}

/// Put message `id` in `category`, said by `source`. The owner's word is never overwritten by
/// anything but the owner's.
pub fn set_category(s: &mut Session, id: Id, category: InboxCategory, source: CategorySource) -> bool {
    let Some(m) = s.mailbox.message(id) else { return false };
    let current = m.insight.as_deref();
    if current.is_some_and(|i| i.owner_said()) && source != CategorySource::Owner {
        return false;
    }
    if current.is_some_and(|i| i.category == Some(category) && i.category_source == source) {
        return false;
    }
    let Ok(m) = s.mailbox.message_mut(id) else { return false };
    let insight = m.insight.get_or_insert_with(Default::default);
    insight.category = Some(category);
    insight.category_source = source;
    true
}

/// The job of `kind` for message `id`, with its text read: `None` when the message isn't there
/// or has nothing to read.
fn job_for(s: &mut Session, id: Id, kind: JobKind, priority: Priority, today: mailcraft_model::time::Date) -> Option<Job> {
    let input = match kind {
        JobKind::Embed | JobKind::EmbedQuery => return None,
        JobKind::SummarizeThread => thread_input(s, id)?,
        JobKind::Classify | JobKind::Summarize => {
            let _ = s.load_where(id, |_| false);
            let m = s.mailbox.message(id)?.clone();
            let chars = if kind == JobKind::Classify { BODY_CHARS / 4 } else { BODY_CHARS };
            input(s, &m, chars)
        }
    };
    s.ai.in_flight.insert((id, kind));
    Some(Job { id, kind, priority, content_hash: insight::content_hash(&input), input, today })
}

/// Ask for a summary of message `id` (or of its conversation), before anything else.
pub fn ask(s: &mut Session, id: Id, kind: JobKind) {
    s.ai.failed.remove(&(id, kind));
    if !s.ai.on_demand.contains(&(id, kind)) && !s.ai.in_flight.contains(&(id, kind)) {
        if s.ai.on_demand.len() >= MAX_ON_DEMAND {
            s.ai.on_demand.pop_back();
        }
        s.ai.on_demand.push_front((id, kind));
    }
}

/// What [`apply`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Applied {
    pub kept: usize,
    pub failed: usize,
    /// For a message no longer here, or whose text changed since: let go of.
    pub stale: usize,
    /// Out of bounds: refused (`Answer::checked`).
    pub refused: usize,
}

/// Take in what a host made of jobs: each checked ([`mailcraft_model::insight::Answer::checked`])
/// and kept on its message, unless the message is gone or its text changed since the job was
/// handed out. The owner's own placing is never changed. Nothing a model wrote is run, followed or
/// acted on; it is kept as text to show.
pub fn apply(s: &mut Session, results: Vec<JobResult>) -> Applied {
    let mut done = Applied::default();
    let mut embedded = false;
    for r in results {
        s.ai.in_flight.remove(&(r.id, r.kind));
        if r.kind.embeds() {
            match embedding(s, &r) {
                Some(true) => {
                    done.kept += 1;
                    embedded = true;
                }
                Some(false) => done.stale += 1,
                None => {
                    s.ai.failed.insert((r.id, r.kind));
                    done.failed += 1;
                }
            }
            continue;
        }
        if s.mailbox.message(r.id).is_none() {
            done.stale += 1;
            continue;
        }
        let model_id = line(&r.model_id, insight::MAX_MODEL_ID);
        match r.outcome {
            Outcome::Failed { reason } => {
                s.ai.failed.insert((r.id, r.kind));
                if let Ok(m) = s.mailbox.message_mut(r.id) {
                    m.insight.get_or_insert_with(Default::default).failed = line(&reason, 200);
                }
                done.failed += 1;
            }
            Outcome::Done(answer) => {
                let checked = match answer.checked(r.kind) {
                    Ok(c) => c,
                    Err(why) => {
                        s.ai.failed.insert((r.id, r.kind));
                        if let Ok(m) = s.mailbox.message_mut(r.id) {
                            m.insight.get_or_insert_with(Default::default).failed = why.reason().to_string();
                        }
                        done.refused += 1;
                        continue;
                    }
                };
                if !still_current(s, r.id, r.kind, &r.content_hash) {
                    done.stale += 1;
                    continue;
                }
                let Ok(m) = s.mailbox.message_mut(r.id) else { continue };
                let unread = !m.read;
                let insight = m.insight.get_or_insert_with(Default::default);
                keep(insight, r.kind, checked, model_id, r.content_hash, unread);
                done.kept += 1;
                // One less to sort, as the window says ("Sorting your Inbox… n left").
                s.ai.backlog = s.ai.backlog.saturating_sub(1);
            }
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    if embedded && let Some(v) = s.ai.vectors.as_mut() {
        v.flush();
    }
    let _ = embedded;
    done
}

/// Take in an embedding: a search's meaning, or a message's kept in the meaning index. `Some(true)`
/// kept, `Some(false)` let go of (the message is gone or changed since), `None` failed or refused.
fn embedding(s: &mut Session, r: &JobResult) -> Option<bool> {
    let Outcome::Done(answer) = &r.outcome else { return None };
    let checked = answer.checked(r.kind).ok()?;
    if r.kind == JobKind::EmbedQuery {
        let Some(text) = s.ai.queries_asked.iter().find(|t| insight::content_hash(t) == r.content_hash).cloned() else { return Some(false) };
        s.ai.queries.retain(|(t, _)| *t != text);
        if s.ai.queries.len() >= MAX_QUERIES {
            s.ai.queries.pop_back();
        }
        s.ai.queries.push_front((text, std::sync::Arc::new(checked.vector)));
        s.ai.meaning_rev = s.ai.meaning_rev.wrapping_add(1);
        return Some(true);
    }
    let _ = s.load_where(r.id, |_| false);
    let m = s.mailbox.message(r.id)?;
    if insight::content_hash(&embed_input(m)) != r.content_hash {
        return Some(false);
    }
    let fp = crate::vectors::fingerprint(m);
    let model = line(&r.model_id, insight::MAX_MODEL_ID);
    vectors(s).insert(r.id, fp, &checked.vector, &model);
    s.ai.embed_backlog = s.ai.embed_backlog.saturating_sub(1);
    Some(true)
}

/// Forget the meaning index, so that every message is read for it again.
pub fn reindex(s: &mut Session) -> usize {
    let n = vectors(s).retain(|_| false);
    #[cfg(not(target_arch = "wasm32"))]
    if let Some(v) = s.ai.vectors.as_mut() {
        v.flush();
    }
    s.ai.failed.retain(|(_, k)| *k != JobKind::Embed);
    n
}

fn keep(insight: &mut Insight, kind: JobKind, checked: insight::Checked, model_id: String, hash: String, unread: bool) {
    let insight::Checked { category, summary, priority, .. } = checked;
    // The owner's word and a fixed rule's stand: the model only fills in what they left.
    let placed = insight.category.is_some() && insight.category_source != CategorySource::Model;
    if let Some(c) = category
        && !placed
        && kind != JobKind::SummarizeThread
    {
        insight.category = Some(c);
        insight.category_source = CategorySource::Model;
    }
    if kind != JobKind::SummarizeThread {
        // Said once, so that it isn't asked again (an answer without it says no). Only unread
        // mail in Primary is a priority: the model's word on mail already read, or sorted
        // elsewhere, is a no.
        let primary = insight.category == Some(InboxCategory::Primary);
        insight.priority = Some(unread && primary && priority.unwrap_or(false));
    }
    match kind {
        JobKind::Embed | JobKind::EmbedQuery => {}
        JobKind::SummarizeThread => insight.thread = summary.map(|summary| ThreadSummary { summary, content_hash: hash }),
        JobKind::Summarize => {
            insight.summary = summary;
            insight.content_hash = hash;
        }
        JobKind::Classify => {
            if insight.content_hash.is_empty() {
                insight.content_hash = hash;
            }
        }
    }
    insight.model_id = model_id;
    insight.failed.clear();
}

/// Whether what a job read is still what message `id` holds: its hash against the input as it
/// would be built now. Only a summary is checked (a category outlives small edits).
fn still_current(s: &mut Session, id: Id, kind: JobKind, hash: &str) -> bool {
    match kind {
        JobKind::Classify => true,
        JobKind::Summarize => {
            let _ = s.load_where(id, |_| false);
            let Some(m) = s.mailbox.message(id).cloned() else { return false };
            insight::content_hash(&input(s, &m, BODY_CHARS)) == hash
        }
        JobKind::SummarizeThread => thread_input(s, id).is_some_and(|t| insight::content_hash(&t) == hash),
        JobKind::Embed | JobKind::EmbedQuery => true,
    }
}

/// Forget every insight (Options › Forget summaries): categories a model or a rule set, and every
/// summary. The owner's own placing stays. How many messages changed.
pub fn clear(s: &mut Session) -> usize {
    let ids: Vec<Id> = s.mailbox.messages.values().filter(|m| m.insight.is_some()).map(|m| m.id).collect();
    let mut n = 0;
    for id in ids {
        let Ok(m) = s.mailbox.message_mut(id) else { continue };
        let keep = m.insight.as_deref().filter(|i| i.owner_said() || i.priority_owner.is_some()).map(|i| Insight {
            category: i.category.filter(|_| i.owner_said()),
            category_source: if i.owner_said() { CategorySource::Owner } else { CategorySource::Model },
            priority_owner: i.priority_owner,
            ..Default::default()
        });
        m.insight = keep.map(Box::new);
        n += 1;
    }
    s.ai.failed.clear();
    s.ai.on_demand.clear();
    n
}

/// Message `id` is open in the reading pane: ask for its summary first, when the smart Inbox is on,
/// it is sorted mail long enough for one, and it has none (nor failed, nor is one on its way).
/// Whether it was asked for.
pub fn opened(s: &mut Session, id: Id) -> bool {
    if !s.prefs.ai.enabled {
        return false;
    }
    let Some(m) = s.mailbox.message(id).cloned() else { return false };
    let insight = m.insight.as_deref();
    if !sorts(s, &m) || m.unfetched || insight.is_some_and(|i| i.summary.is_some() || !i.failed.is_empty()) {
        return false;
    }
    let kind = JobKind::Summarize;
    if s.ai.in_flight.contains(&(id, kind)) || s.ai.failed.contains(&(id, kind)) || s.ai.on_demand.contains(&(id, kind)) {
        return false;
    }
    if words_of(&m) < summary_floor(s, &m) {
        return false;
    }
    s.ai.on_demand.push_front((id, kind));
    true
}

/// Whether a summary of `id` is on its way (asked for, or with the host).
pub fn summarizing(s: &Session, id: Id, kind: JobKind) -> bool {
    s.ai.in_flight.contains(&(id, kind)) || s.ai.on_demand.contains(&(id, kind))
}

/// Forget what was handed out (the host restarted, or another mailbox opened): it is asked for
/// again.
pub fn host_restarted(s: &mut Session) {
    s.ai.in_flight.clear();
}

/// A mailbox opened (or the program started) with the smart Inbox already on: its Inbox opens on
/// Primary, as it does when the smart Inbox is switched on. A part already chosen is kept.
pub fn opening_view(s: &mut Session) {
    if s.prefs.ai.enabled && s.view.inbox_category.is_none() {
        s.view.inbox_category = Some(InboxCategory::Primary);
    }
}
