// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The message list: which messages show for the folder, filter and search in view, in what
//! order, under which group headings.

use std::collections::{BTreeSet, HashMap, HashSet};

use mailcraft_model::search::Query;
use mailcraft_model::{FlagStatus, FolderKind, Id, Importance, Message, ThreadIndex, ThreadKey};
use mailcraft_time::{Date, DateGroup, Timestamp};
use serde::Serialize;

use crate::{Filter, Scope, Session, SortBy};

/// One row of the message list.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Row {
    /// The message the row shows (the newest of its conversation).
    pub id: Id,
    /// Every message the row stands for, newest first (just `id` outside conversation view, and
    /// for a message listed under its open conversation): those of its conversation the list
    /// shows, which Delete and Move take with it (#485, #503).
    pub members: Vec<Id>,
    /// In conversation view, the messages of its conversation in the account's other folders (a
    /// reply in Sent Items), newest first (#503): counted with `members` and listed under the row
    /// when it is open, but not what the row stands for, as in Outlook.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub others: Vec<Id>,
    pub unread: usize,
    /// The group heading above this row, when it starts a group (`Today`, `Last Week`).
    pub group: Option<String>,
    /// Whether the heading is one of the owner's categories (to be drawn as spelt, never
    /// translated) and not a date, importance or one of the program's own (`Pinned`, no category).
    pub category: bool,
    /// Which of the message's rows this is: arranged by categories, a message filed under two
    /// categories is listed under each, as Outlook lists it, and the second row is copy 1. Under
    /// an open conversation, the newest message's own row is copy 1 too: copy 0 is the
    /// conversation's row.
    pub copy: u16,
    /// For a message listed under its open conversation (#503): the message the conversation's
    /// row shows.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub under: Option<Id>,
    /// For a conversation's row: it is open, and its messages are listed under it.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub open: bool,
    /// The smart Inbox: a row that stands for rows folded away ("3 more from Shop"), or, when they
    /// are shown, the one that folds them again. Its `id` is the first of them, its copy
    /// [`MORE_COPY`], its `members` all their messages.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub more: Option<More>,
    /// The smart Inbox: a row shown with the one above it (a sender's other mail, shown whole),
    /// which takes no heading of its own.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub grouped: bool,
    /// The smart Inbox: a row of the Priority section at the top of Primary.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub priority: bool,
    /// A search found it by meaning alone: it hasn't the search's words.
    #[serde(skip_serializing_if = "std::ops::Not::not")]
    pub meaning: bool,
}

/// What a "more" row of the smart Inbox stands for ([`Row::more`]).
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct More {
    /// What `view.smartGroup` shows or folds: `priority`, or `from:` and a sender's address.
    pub key: String,
    /// How many rows it stands for.
    pub count: usize,
    /// The senders of those rows, the first three that differ, as they are shown.
    pub names: Vec<String>,
    /// How many other senders there are beyond `names`.
    pub others: usize,
    /// Whether the rows are shown (above it) rather than folded away.
    pub open: bool,
}

/// The copy number of a "more" row ([`Row::more`]), which no message's own row has.
pub const MORE_COPY: u16 = u16::MAX;

/// The heading of the Priority section at the top of Primary.
pub const PRIORITY: &str = "Priority";

impl Row {
    /// The row of one message, `copy` of its rows.
    fn of(m: &Message, copy: u16) -> Row {
        Row {
            id: m.id,
            members: vec![m.id],
            others: Vec::new(),
            unread: usize::from(!m.read),
            group: None,
            category: false,
            copy,
            under: None,
            open: false,
            more: None,
            grouped: false,
            priority: false,
            meaning: false,
        }
    }

    /// How many messages the row's conversation has: what its count says, and what is listed
    /// under it when it is open (#503).
    pub fn size(&self) -> usize {
        self.members.len().saturating_add(self.others.len())
    }

    /// Whether it is the row of a conversation of more than one message, which opens.
    pub fn conversation(&self) -> bool {
        self.under.is_none() && self.size() > 1
    }
}

/// When a message counts as having arrived: its received time, or its date when it has none.
pub fn when(m: &Message) -> Timestamp {
    if m.received != 0 { m.received } else { m.date }
}

/// The first words of a message on one line, at most `max_chars` characters (and `…` when there
/// is more): what a message list shows under the subject.
///
/// It is what the message says to someone reading it: taken from the HTML when there is HTML
/// (the opening line a sender hid for message lists included), otherwise from the text, with
/// markup, character references, invisible padding, rules, quoted text and web addresses left out
/// ([`mailcraft_html::preview`], [`mailcraft_html::text_preview`]). It is worked out from the
/// message each time and stored nowhere, so it costs a short read of the body's first words: a
/// list that draws the same rows again and again keeps what it was given.
pub fn preview(m: &Message, max_chars: usize) -> String {
    let said = m.body.html.as_deref().map(|html| mailcraft_html::preview(html, max_chars)).unwrap_or_default();
    if said.is_empty() { mailcraft_html::text_preview(&m.body.text, max_chars) } else { said }
}

/// A row's heading, compared with the last row's as it is: the words of a date's are written
/// only for the rows a heading starts at (two date groups never have the same words).
#[derive(PartialEq)]
enum Heading {
    Date(DateGroup),
    Words(String),
}

impl Heading {
    fn label(&self) -> String {
        match self {
            Heading::Date(d) => d.label().to_string(),
            Heading::Words(w) => w.clone(),
        }
    }
}

fn heading(s: &Session, m: &Message, today: Date) -> Option<Heading> {
    match s.view.sort {
        SortBy::Date if !m.pinned => Some(Heading::Date(DateGroup::of(s.local(when(m)).date, today, s.prefs.week_start))),
        _ => group_label(s, m).map(Heading::Words),
    }
}

fn group_label(s: &Session, m: &Message) -> Option<String> {
    if m.pinned {
        return Some("Pinned".into());
    }
    match s.view.sort {
        SortBy::Date => Some(DateGroup::of(s.local(when(m)).date, s.today(), s.prefs.week_start).label().to_string()),
        SortBy::From => Some(m.from.display().to_string()),
        SortBy::Importance => Some(
            match m.importance {
                Importance::High => "High",
                Importance::Normal => "Normal",
                Importance::Low => "Low",
            }
            .to_string(),
        ),
        // Each row of a message arranged by categories has a heading of its own (`by_category`).
        SortBy::Categories => None,
        SortBy::Subject | SortBy::Size => None,
    }
}

/// The heading of the things that have no category, in a list arranged by categories.
pub const NO_CATEGORY: &str = "No category";

/// How a category name is compared: Unicode lower case, for sorting, for telling two spellings
/// apart and for its heading alike.
pub fn fold(name: &str) -> String {
    name.to_lowercase()
}

/// One spelling for each category name however it was written, by its [`fold`]: the lists of the
/// accounts of `items`, each given by its account, first, then the mailbox's other lists, then the
/// first spelling met among the items' categories. One map per list build, so no name is searched
/// for item by item.
pub fn category_spellings<'a>(s: &'a Session, items: impl IntoIterator<Item = (Id, &'a [String])>) -> HashMap<String, &'a str> {
    let items: Vec<(Id, &[String])> = items.into_iter().collect();
    let accounts: BTreeSet<Id> = items.iter().map(|(account, _)| *account).collect();
    let mut spelling: HashMap<String, &str> = HashMap::new();
    for k in accounts.iter().flat_map(|account| s.mailbox.categories_of(*account)).chain(s.mailbox.all_categories()) {
        spelling.entry(fold(&k.name)).or_insert(k.name.as_str());
    }
    for (_, categories) in items {
        for c in categories.iter().map(|c| c.trim()).filter(|c| !c.is_empty()) {
            spelling.entry(fold(c)).or_insert(c);
        }
    }
    spelling
}

/// An item's categories one by one, A to Z, each once whatever capitals it was given in (folded
/// names, see [`fold`]): the headings an item arranged by categories is listed under.
pub fn category_names(categories: &[String]) -> Vec<String> {
    let mut names: Vec<String> = categories.iter().map(|c| c.trim()).filter(|c| !c.is_empty()).map(fold).collect();
    names.sort();
    names.dedup();
    names
}

pub fn rows(s: &Session) -> Vec<Row> {
    rows_with(s, &HashMap::new(), &HashMap::new())
}

/// Which messages the list looks at: those of the folder in view; for a search of its
/// subfolders, those of the folder and every folder under it; for a search of the mailbox in
/// view, those of every folder of its account; for a search of every mailbox, every message. A
/// search of the mailbox or of every mailbox leaves out Deleted Items, Junk E-mail and their
/// subfolders (#493).
pub fn in_view(s: &Session) -> impl Fn(&Message) -> bool + '_ {
    let (everywhere, folders) = view_folders(s);
    move |m: &Message| folders.contains(&m.folder) != everywhere
}

/// The folders the list looks in ([`in_view`]), or, for a search of every mailbox (`true`),
/// the folders it leaves out.
fn view_folders(s: &Session) -> (bool, HashSet<Id>) {
    let mb = &s.mailbox;
    let folder = s.view.folder;
    let scope = if Query::parse(&s.view.search).is_empty() { Scope::Folder } else { s.view.scope };
    // A mailbox of every account's mail is its folders, whatever a search's scope but every mailbox.
    if let Some(which) = crate::all::in_view(s)
        && scope != Scope::Everywhere
    {
        return (false, crate::all::folders(mb, which));
    }
    // Deleted and junk mail stays out of a search of a whole mailbox (`account`) or of all.
    let hidden = |account: Option<Id>| -> HashSet<Id> {
        mb.folders
            .iter()
            .filter(|f| account.is_none_or(|a| f.account == a) && matches!(f.kind, FolderKind::Deleted | FolderKind::Junk))
            .flat_map(|f| mb.subtree(f.id))
            .collect()
    };
    // The folders looked in, or (for every mailbox) the folders left out.
    let (everywhere, folders): (bool, HashSet<Id>) = match scope {
        Scope::Folder => (false, HashSet::from([folder])),
        Scope::Subfolders => (false, mb.subtree(folder).into_iter().collect()),
        Scope::Mailbox => {
            let account = mb.folder(folder).map(|f| f.account);
            let hidden = hidden(account);
            let of_account = mb.folders.iter().filter(|f| account == Some(f.account) && !hidden.contains(&f.id));
            (false, of_account.map(|f| f.id).collect())
        }
        Scope::Everywhere => (true, hidden(None)),
    };
    (everywhere, folders)
}

/// The messages the list looks at ([`in_view`]): those of the folders looked in, from the
/// folders' own lists when the session has them counted ([`Session::fresh_counts`]: a few
/// hundred thousand messages in other folders are then not walked for a list of one), else
/// every message that is in view.
fn looked_at<'a>(s: &'a Session, everywhere: bool, folders: &HashSet<Id>) -> Vec<&'a Message> {
    let mb = &s.mailbox;
    match s.fresh_counts() {
        Some(counts) if !everywhere => {
            folders.iter().flat_map(|f| mb.messages.get_sorted(counts.ids(*f))).map(|m| m.as_ref()).filter(|m| folders.contains(&m.folder)).collect()
        }
        _ => mb.messages.values().map(|m| m.as_ref()).filter(|m| folders.contains(&m.folder) != everywhere).collect(),
    }
}

/// [`rows`], for a search answered in part already: `decided` says whether each message the word
/// index answered for matches (`Session::search_words`), and `searched` whether each whose body
/// was brought in and let go of again while the search ran did (`Session::load_bodies_for_search`),
/// which a body named only can no longer say.
pub fn rows_with(s: &Session, decided: &HashMap<Id, bool>, searched: &HashMap<Id, bool>) -> Vec<Row> {
    listing(s, decided, searched).0
}

/// The addresses of the owner's accounts, for To Me.
fn me(s: &Session) -> Vec<&str> {
    s.mailbox.accounts.iter().map(|a| a.email.as_str()).filter(|e| !e.is_empty()).collect()
}

/// Whether `m` matches the filter in view (All, Unread, Flagged…); `me` as from [`me`].
fn filtered(s: &Session, m: &Message, me: &[&str]) -> bool {
    match s.view.filter {
        Filter::All => true,
        Filter::Unread => !m.read,
        Filter::Flagged => m.flag == FlagStatus::Flagged,
        Filter::Attachments => m.has_attachments(),
        Filter::ToMe => m.to.iter().any(|a| me.iter().any(|e| a.email.eq_ignore_ascii_case(e))),
    }
}

/// Of `ids`, those the list in view lists now, without building it (#428): in the folder in view,
/// and matching the filter or kept. For a list with no search: what a search finds, only the
/// list can say.
pub(crate) fn lists(s: &Session, ids: &[Id]) -> std::collections::HashSet<Id> {
    let in_view = in_view(s);
    let me = me(s);
    let kept = s.kept();
    // (The folder's messages are looked through only for conversations of a Gmail category.)
    let gmail = crate::gmail::Shown::of(s, s.mailbox.messages.values().map(|m| m.as_ref()));
    let smart = crate::ai::Shown::of(s, s.mailbox.messages.values().map(|m| m.as_ref()));
    let all = crate::all::in_view(s);
    let shows = |m: &Message| {
        in_view(m)
            && ((gmail.as_ref().is_none_or(|g| g.keeps(m))
                && smart.as_ref().is_none_or(|g| g.keeps(m))
                && all.is_none_or(|a| crate::all::keeps(&s.mailbox, a, m))
                && filtered(s, m, &me))
                || kept.as_ref().is_some_and(|kept| kept.contains(&m.id)))
    };
    ids.iter().copied().filter(|id| s.mailbox.message(*id).is_some_and(shows)).collect()
}

/// [`rows_with`], the messages the rows list only because they are selected (#428: they were
/// in this list when they were selected and no longer match the filter or the search), and how
/// many messages the list looks at are not fetched whole (a search of them reads their headers
/// and preview alone, R3.2; counted here, in the one pass over the mail, not at every frame).
pub(crate) fn listing(s: &Session, decided: &HashMap<Id, bool>, searched: &HashMap<Id, bool>) -> (Vec<Row>, Vec<Id>, usize) {
    let mb = &s.mailbox;
    let query = s.search_query();
    let (everywhere, folders) = view_folders(s);
    let me = me(s);
    let kept = s.kept();
    let mut kept_only = Vec::new();
    let mut unfetched = 0usize;
    let looked = looked_at(s, everywhere, &folders);
    // A Gmail category of the Inbox shows the Inbox's mail Gmail put in it (#911).
    let gmail = crate::gmail::Shown::of(s, looked.iter().copied());
    // A part of the smart Inbox shows the Inbox's mail sorted into it.
    let smart = crate::ai::Shown::of(s, looked.iter().copied());
    // VIPs and Flagged keep their own mail of the folders they look in.
    let all = crate::all::in_view(s);
    // What the search finds by meaning, beside its words (`crate::ai::meaning_hits`).
    let meaning = if query.is_empty() { HashSet::new() } else { crate::ai::meaning_hits(s, &looked) };
    let mut by_meaning: HashSet<Id> = HashSet::new();
    let mut shown: Vec<&Message> = looked
        .into_iter()
        .filter(|m| {
            unfetched = unfetched.saturating_add(usize::from(m.unfetched));
            let words = || match decided.get(&m.id).or_else(|| searched.get(&m.id).filter(|_| m.body_is_kept())) {
                Some(hit) => *hit,
                None => query.matches_in(m, |at| s.offset_at(at)),
            };
            let hit = gmail.as_ref().is_none_or(|g| g.keeps(m))
                && smart.as_ref().is_none_or(|g| g.keeps(m))
                && all.is_none_or(|a| crate::all::keeps(mb, a, m))
                && filtered(s, m, &me)
                && (query.is_empty()
                    || words()
                    || (meaning.contains(&m.id) && {
                        by_meaning.insert(m.id);
                        true
                    }));
            // A selected message stays while it is selected, though it was read or its flag was
            // cleared; `in_view` above still applies, so one deleted, or moved out of the folder in
            // view, is gone.
            if !hit && kept.as_ref().is_some_and(|kept| kept.contains(&m.id)) {
                kept_only.push(m.id);
                return true;
            }
            hit
        })
        .collect();

    let desc = s.view.sort_descending;
    if matches!(s.view.sort, SortBy::Date | SortBy::Importance | SortBy::Categories) {
        sort_by_dates(&mut shown, s.view.sort, desc);
    } else {
        sort_by_words(&mut shown, s.view.sort, desc);
    }

    if s.view.sort == SortBy::Categories {
        return (by_category(s, &shown, desc), kept_only, unfetched);
    }

    let mut out: Vec<Row> = Vec::new();
    if s.view.conversations && s.view.sort == SortBy::Date {
        // One row per conversation, where its newest (or, ascending, oldest) message sorts: the
        // messages of one thread with one topic (#795).
        let threads = s.threads();
        let mut index: HashMap<(Id, ThreadKey), usize> = HashMap::new();
        for m in &shown {
            match index.entry((m.account, threads.key(m.id))) {
                std::collections::hash_map::Entry::Occupied(at) => {
                    if let Some(row) = out.get_mut(*at.get()) {
                        row.members.push(m.id);
                        row.unread += usize::from(!m.read);
                    }
                }
                std::collections::hash_map::Entry::Vacant(at) => {
                    at.insert(out.len());
                    out.push(Row::of(m, 0));
                }
            }
        }
        // Without a search, the rest of each conversation, in the account's other folders: a
        // reply in Sent Items is part of it (#503). (A search lists what it finds where it looks.)
        if query.is_empty() {
            elsewhere(s, &me, kept.as_deref(), (&threads, &index), &mut out, &mut kept_only);
        }
        let newest = |id: &Id| std::cmp::Reverse(mb.message(*id).map(|m| (when(m), m.id)));
        for row in &mut out {
            // Newest first inside a conversation, and the row shows the newest.
            // (Each one's key worked out once: a lookup at every comparison took a third of the
            // time a list of 100,000 messages took to build.)
            if row.members.len() > 1 {
                row.members.sort_by_cached_key(newest);
            }
            if row.others.len() > 1 {
                row.others.sort_by_cached_key(newest);
            }
            if let Some(first) = row.members.first() {
                row.id = *first;
            }
        }
        if !s.view.open.is_empty() {
            out = opened(s, &threads, out);
        }
    } else {
        out.extend(shown.iter().map(|m| Row::of(m, 0)));
    }

    // Show in Groups off: the sorted rows, no headings. (A message under its open conversation is
    // under its conversation's heading.) A row of each message `shown` has, in its order, is
    // that message's, without looking it up.
    if s.prefs.show_groups {
        let direct = out.len() == shown.len() && out.iter().zip(&shown).all(|(r, m)| r.id == m.id);
        let mut last: Option<Heading> = None;
        let today = s.today();
        for (i, row) in out.iter_mut().enumerate().filter(|(_, r)| r.under.is_none()) {
            let m = if direct { shown.get(i).copied() } else { mb.message(row.id) };
            let heading = m.and_then(|m| heading(s, m, today));
            if heading != last {
                row.group = heading.as_ref().map(Heading::label);
                last = heading;
            }
        }
    }
    let mut out = arrange_smart(s, out);
    if !by_meaning.is_empty() {
        for row in &mut out {
            row.meaning = row.more.is_none() && row.members.iter().all(|id| by_meaning.contains(id));
        }
    }
    (out, kept_only, unfetched)
}

/// The list of a part of the smart Inbox, by date, newest first, without a search, as Apple's
/// mail arranges it: in Primary (and All Mail, the whole Inbox) the time-sensitive mail first, in
/// a Priority section that shows [`crate::ai::PRIORITY_ROWS`] and folds the rest into "n more
/// from …"; in Transactions,
/// Updates and Promotions each sender's mail together, its newest row and "n more from …" for
/// the rest. A folded group opens with `view.smartGroup`. Any other list is left as it is.
fn arrange_smart(s: &Session, rows: Vec<Row>) -> Vec<Row> {
    let primary = match crate::ai::in_view(s) {
        Some(category) => category == mailcraft_model::InboxCategory::Primary,
        None if crate::ai::all_mail_in_view(s) => true,
        None => return rows,
    };
    if s.view.sort != SortBy::Date || !s.view.sort_descending || !Query::parse(&s.view.search).is_empty() {
        return rows;
    }
    let mb = &s.mailbox;
    // A row with the rows of its open conversation under it, as one.
    let mut blocks: Vec<Vec<Row>> = Vec::new();
    for row in rows {
        match blocks.last_mut() {
            Some(block) if row.under.is_some() => block.push(row),
            _ => blocks.push(vec![row]),
        }
    }
    let open = |key: &str| s.view.smart_open.iter().any(|k| k == key);
    let mut out: Vec<Row> = Vec::new();
    if primary {
        let urgent =
            |b: &Vec<Row>| b.first().is_some_and(|h| h.members.iter().any(|id| mb.message(*id).is_some_and(|m| crate::ai::is_priority(s, m))));
        let (priority, rest): (Vec<Vec<Row>>, Vec<Vec<Row>>) = blocks.into_iter().partition(urgent);
        // Folding a single row away would save nothing.
        let folds = priority.len() > crate::ai::PRIORITY_ROWS + 1;
        let shown = !folds || open(PRIORITY);
        let mut hidden: Vec<Row> = Vec::new();
        for (i, block) in priority.into_iter().enumerate() {
            let past = i >= crate::ai::PRIORITY_ROWS;
            for mut row in block {
                if row.under.is_none() {
                    row.group = (i == 0).then(|| PRIORITY.to_string());
                    row.category = false;
                    row.priority = true;
                    if past {
                        hidden.push(row.clone());
                    }
                }
                if shown || !past {
                    out.push(row);
                }
            }
        }
        if folds && let Some(more) = more_row(s, PRIORITY, &hidden, shown) {
            out.push(more);
        }
        let start = out.len();
        out.extend(rest.into_iter().flatten());
        if let Some(rest) = out.get_mut(start..) {
            regroup(s, rest);
        }
    } else {
        // Each sender's rows, in the order of their newest.
        let sender = |b: &Vec<Row>| b.first().and_then(|h| mb.message(h.id)).map(|m| m.from.email.trim().to_lowercase()).filter(|e| !e.is_empty());
        let mut at: HashMap<String, usize> = HashMap::new();
        let mut groups: Vec<(Option<String>, Vec<Vec<Row>>)> = Vec::new();
        for block in blocks {
            match sender(&block) {
                Some(key) => match at.get(&key) {
                    Some(i) => {
                        if let Some(group) = groups.get_mut(*i) {
                            group.1.push(block);
                        }
                    }
                    None => {
                        at.insert(key.clone(), groups.len());
                        groups.push((Some(key), vec![block]));
                    }
                },
                None => groups.push((None, vec![block])),
            }
        }
        for (address, blocks) in groups {
            let mut blocks = blocks.into_iter();
            out.extend(blocks.next().unwrap_or_default());
            let rest: Vec<Vec<Row>> = blocks.collect();
            let Some(address) = address.filter(|_| !rest.is_empty()) else { continue };
            let key = format!("from:{address}");
            let shown = open(&key);
            let heads: Vec<Row> = rest.iter().filter_map(|b| b.first().cloned()).collect();
            if shown {
                for mut row in rest.into_iter().flatten() {
                    if row.under.is_none() {
                        row.grouped = true;
                        row.group = None;
                    }
                    out.push(row);
                }
            }
            if let Some(more) = more_row(s, &key, &heads, shown) {
                out.push(more);
            }
        }
        regroup(s, &mut out);
    }
    out
}

/// The "more" row of the smart Inbox for `heads`, the rows it folds away or that are shown above
/// it ([`Row::more`]).
fn more_row(s: &Session, key: &str, heads: &[Row], open: bool) -> Option<Row> {
    let first = heads.first()?;
    let mut names: Vec<String> = Vec::new();
    let mut others: HashSet<String> = HashSet::new();
    for head in heads {
        let Some(m) = s.mailbox.message(head.id) else { continue };
        let name = m.from.display().to_string();
        if names.contains(&name) || others.contains(&name) {
            continue;
        }
        if names.len() < 3 {
            names.push(name);
        } else {
            others.insert(name);
        }
    }
    Some(Row {
        id: first.id,
        members: heads.iter().flat_map(|h| h.members.iter().copied()).collect(),
        others: Vec::new(),
        unread: heads.iter().map(|h| h.unread).sum(),
        group: None,
        category: false,
        copy: MORE_COPY,
        under: None,
        open: false,
        more: Some(More { key: key.to_string(), count: heads.len(), names, others: others.len(), open }),
        grouped: true,
        priority: first.priority,
        meaning: false,
    })
}

/// The date headings of `rows` again, after the smart Inbox moved rows about: a row shown with
/// the one above it, or a "more" row, takes none.
fn regroup(s: &Session, rows: &mut [Row]) {
    let mut last: Option<Heading> = None;
    let today = s.today();
    for row in rows.iter_mut().filter(|r| r.under.is_none()) {
        row.group = None;
        row.category = false;
        if !s.prefs.show_groups || row.grouped || row.more.is_some() {
            continue;
        }
        let heading = s.mailbox.message(row.id).and_then(|m| heading(s, m, today));
        if heading != last {
            row.group = heading.as_ref().map(Heading::label);
            last = heading;
        }
    }
}

/// `shown` in the order of a sort by name or size (From, Subject, Size): pinned first, ids
/// breaking ties. A sender's or a subject's name is folded once for each message, not at each
/// comparison.
fn sort_by_words(shown: &mut Vec<&Message>, sort: SortBy, desc: bool) {
    let name = |m: &Message| match sort {
        SortBy::From => m.from.display().to_lowercase(),
        SortBy::Subject => m.topic().to_lowercase(),
        _ => String::new(),
    };
    let mut keyed: Vec<(String, &Message)> = shown.iter().map(|m| (name(m), *m)).collect();
    keyed.sort_by(|(an, a), (bn, b)| {
        let key = match sort {
            // Within one sender or subject, newest first when the names run A to Z.
            SortBy::From | SortBy::Subject => an.cmp(bn).then(when(b).cmp(&when(a))),
            // As the row says it (#273): the server's size for what isn't here; one not known
            // counts as the smallest.
            SortBy::Size => a.list_size().cmp(&b.list_size()),
            SortBy::Importance => a.importance.cmp(&b.importance).then(when(a).cmp(&when(b))),
            // Newest first; `by_category` then lists each under its categories.
            SortBy::Categories => when(b).cmp(&when(a)),
            SortBy::Date => when(a).cmp(&when(b)),
        };
        // Pinned messages stay on top whichever way the list is sorted; ids break ties so the
        // order never depends on the map's iteration.
        // The categories, not the dates, are what turning the list over turns over.
        let key = if desc && sort != SortBy::Categories { key.reverse() } else { key };
        b.pinned.cmp(&a.pinned).then(key).then(b.id.cmp(&a.id))
    });
    shown.clear();
    shown.extend(keyed.into_iter().map(|k| k.1));
}

/// `shown` in the order of a sort by date, importance or categories, as [`sort_by_words`]
/// would put it: from what the order needs of each message, taken once and side by side,
/// rather than read from both messages at each of the two million comparisons a list of
/// 100,000 makes.
fn sort_by_dates(shown: &mut Vec<&Message>, sort: SortBy, desc: bool) {
    let mut keyed: Vec<(bool, Timestamp, Importance, Id, &Message)> = shown.iter().map(|m| (m.pinned, when(m), m.importance, m.id, *m)).collect();
    keyed.sort_unstable_by(|a, b| {
        let key = match sort {
            SortBy::Importance => a.2.cmp(&b.2).then(a.1.cmp(&b.1)),
            SortBy::Categories => b.1.cmp(&a.1),
            _ => a.1.cmp(&b.1),
        };
        let key = if desc && sort != SortBy::Categories { key.reverse() } else { key };
        b.0.cmp(&a.0).then(key).then(b.3.cmp(&a.3))
    });
    shown.clear();
    shown.extend(keyed.into_iter().map(|k| k.4));
}

/// The messages of the conversations of `rows` in the account's other folders (#503), as the
/// reading pane's conversation takes them ([`Mailbox::conversation_skips`]: none in Deleted Items
/// or Junk E-mail or under them, unless the list is of that folder), that the filter in view lists or that stay
/// in it while selected (`kept`, #428, and then in `kept_only` too): each in its row's `others`.
/// For a list without a search, all of whose rows are of the folder in view; `index` is where
/// each conversation's row is, by its account and its key.
///
/// [`Mailbox::conversation_skips`]: mailcraft_model::Mailbox::conversation_skips
fn elsewhere(
    s: &Session,
    me: &[&str],
    kept: Option<&HashSet<Id>>,
    (threads, index): (&ThreadIndex, &HashMap<(Id, ThreadKey), usize>),
    rows: &mut [Row],
    kept_only: &mut Vec<Id>,
) {
    let mb = &s.mailbox;
    let folder = s.view.folder;
    let Some(account) = mb.folder(folder).map(|f| f.account) else { return };
    let skips = mb.conversation_skips(account, folder);
    // The account's other folders, from their own lists when the session has them counted
    // ([`Session::fresh_counts`]), else from a walk over every message.
    let others: Vec<&Message> = match s.fresh_counts() {
        Some(counts) => mb
            .folders
            .iter()
            .filter(|f| f.account == account && f.id != folder && !skips.contains(&f.id))
            .flat_map(|f| counts.ids(f.id))
            .filter_map(|id| mb.message(*id))
            .collect(),
        None => mb.messages.values().map(|m| m.as_ref()).collect(),
    };
    for m in others {
        if m.account != account || m.folder == folder || skips.contains(&m.folder) {
            continue;
        }
        let hit = filtered(s, m, me);
        if !hit && !kept.is_some_and(|kept| kept.contains(&m.id)) {
            continue;
        }
        if let Some(row) = index.get(&(m.account, threads.key(m.id))).and_then(|i| rows.get_mut(*i)) {
            row.others.push(m.id);
            if !hit {
                kept_only.push(m.id);
            }
        }
    }
}

/// `rows`, with the messages of each open conversation (#503) listed under its row, newest first,
/// whichever folder each is in. The newest one's own row there is its copy 1: copy 0 is the
/// conversation's row.
fn opened(s: &Session, threads: &ThreadIndex, rows: Vec<Row>) -> Vec<Row> {
    let mb = &s.mailbox;
    let open: HashSet<&(Id, ThreadKey)> = s.view.open.iter().collect();
    let mut out = Vec::with_capacity(rows.len());
    for mut row in rows {
        row.open = row.conversation() && mb.message(row.id).is_some_and(|m| open.contains(&(m.account, threads.key(m.id))));
        if !row.open {
            out.push(row);
            continue;
        }
        let mut under: Vec<&Message> = row.members.iter().chain(&row.others).filter_map(|id| mb.message(*id)).collect();
        under.sort_by_key(|m| std::cmp::Reverse((when(m), m.id)));
        let head = row.id;
        let listed: Vec<Row> = under.into_iter().map(|m| Row { under: Some(head), ..Row::of(m, u16::from(m.id == head)) }).collect();
        out.push(row);
        out.extend(listed);
    }
    out
}

/// The list arranged by categories, as Outlook arranges it: a message is listed under each of its
/// categories, those with none under "No category" first, then the categories A to Z (Z to A
/// turned over), newest first under each. A pinned message is listed once, on top. `shown` is
/// sorted already: pinned first, then newest first.
///
/// Show in Groups off doesn't apply: a message is listed under each of its categories, which takes
/// the headings, so they stay. Outlook does the same: unticking it there leaves a list arranged by
/// categories grouped as it was (observed 2026-10-22).
fn by_category(s: &Session, shown: &[&Message], desc: bool) -> Vec<Row> {
    let spelling = category_spellings(s, shown.iter().map(|m| (m.account, m.categories.as_slice())));
    // (heading's folded name, message); the sort is stable, so under each heading `shown`'s order stays.
    let mut placed: Vec<(Option<String>, &Message)> = Vec::new();
    for m in shown {
        let names = if m.pinned { Vec::new() } else { category_names(&m.categories) };
        if names.is_empty() {
            placed.push((None, m));
        }
        placed.extend(names.into_iter().map(|name| (Some(name), *m)));
    }
    placed.sort_by_cached_key(|(name, m)| (std::cmp::Reverse(m.pinned), if desc { Err(std::cmp::Reverse(name.clone())) } else { Ok(name.clone()) }));
    let mut copies: HashMap<Id, u16> = HashMap::new();
    let mut last: Option<(String, bool)> = None;
    let mut out = Vec::with_capacity(placed.len());
    for (name, m) in placed {
        let copy = copies.entry(m.id).or_default();
        let mut row = Row::of(m, *copy);
        *copy = copy.saturating_add(1);
        let (label, category) = if m.pinned {
            ("Pinned".to_string(), false)
        } else {
            match name {
                Some(n) => (spelling.get(&n).map_or(n.clone(), |k| k.to_string()), true),
                None => (NO_CATEGORY.to_string(), false),
            }
        };
        let key = (fold(&label), category);
        if last.as_ref() != Some(&key) {
            row.group = Some(label);
            row.category = category;
            last = Some(key);
        }
        out.push(row);
    }
    out
}

/// The row a message is in (its first, when it is listed more than once): its own before a
/// conversation's that has it (#503: under its open conversation).
pub fn position(rows: &[Row], id: Id) -> Option<usize> {
    rows.iter().position(|r| r.id == id).or_else(|| rows.iter().position(|r| r.members.contains(&id)))
}

/// The row numbered `copy` of a message's rows, or its first when it has fewer.
pub fn position_of(rows: &[Row], id: Id, copy: u16) -> Option<usize> {
    rows.iter().position(|r| r.id == id && r.copy == copy).or_else(|| position(rows, id))
}

/// How many messages (or conversations) the list shows: one listed under several categories
/// counts once, the messages listed under an open conversation are its row's, and those the
/// smart Inbox folded into "n more from …" count as listed.
pub fn listed(rows: &[Row]) -> usize {
    // What the smart Inbox folded away is in the list all the same.
    let folded: usize = rows.iter().filter_map(|r| r.more.as_ref()).filter(|m| !m.open).map(|m| m.count).sum();
    rows.iter().filter(|r| r.copy == 0 && r.under.is_none()).count().saturating_add(folded)
}
