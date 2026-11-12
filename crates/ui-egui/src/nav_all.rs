// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The top of the folder pane when the owner has two accounts or more: All Inboxes (each
//! account's Inbox under it), VIPs, Flagged, All Drafts and All Sent (each account's under them),
//! mailboxes made of every account's mail ([`mailcraft_engine::all`]). File › Options turns them
//! off (`view.showAllMailboxes`).

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_engine::all::{self, AllMailbox};
use mailcraft_engine::counts::FolderCounts;
use mailcraft_model::{Folder, Id};

use super::{
    ALL_ROW_H, CHEVRON_X, COUNT_RIGHT, Count, HEADING_SIZE, HIGHLIGHT_LEFT, NAME_LEFT, NAME_SIZE, ROW_CHEVRON, TEXT_RISE, chevron, folder_row,
    highlight, owners,
};
use crate::theme::{Tokens, bold, paint_clipped, regular};
use crate::{MailApp, icons};

/// The ids All Inboxes, All Drafts and All Sent are folded or opened under in
/// `UiState::collapsed_folders`, past any account's or folder's. All Inboxes shows its Inboxes
/// unless it is listed there; All Drafts and All Sent show theirs only while they are.
pub(super) const ALL_INBOXES: Id = u64::MAX - 1;
pub(super) const ALL_DRAFTS: Id = u64::MAX - 2;
pub(super) const ALL_SENT: Id = u64::MAX - 3;

/// How far the name of one of these rows starts after its icon.
const ICON_W: f32 = 22.0;

/// Whether the folder pane starts with them: they are on, and the owner has two accounts at least.
pub(super) fn shown(app: &MailApp) -> bool {
    app.session.prefs.all_mailboxes && owners(&app.session.mailbox).len() > 1
}

/// What `which` is called in the window's language.
pub(crate) fn name(which: AllMailbox) -> &'static str {
    match which {
        AllMailbox::Inboxes => tl!("All Inboxes"),
        AllMailbox::Vips => tl!("VIPs"),
        AllMailbox::Flagged => tl!("Flagged"),
        AllMailbox::Drafts => tl!("All Drafts"),
        AllMailbox::Sent => tl!("All Sent"),
    }
}

/// The command that shows `which`.
fn command(which: AllMailbox) -> &'static str {
    match which {
        AllMailbox::Inboxes => "view.allInboxes",
        AllMailbox::Vips => "view.vips",
        AllMailbox::Flagged => "view.allFlagged",
        AllMailbox::Drafts => "view.allDrafts",
        AllMailbox::Sent => "view.allSent",
    }
}

fn icon(which: AllMailbox) -> &'static str {
    match which {
        AllMailbox::Inboxes => "inbox",
        AllMailbox::Vips => "star",
        AllMailbox::Flagged => "flag",
        AllMailbox::Drafts => "drafts",
        AllMailbox::Sent => "sent",
    }
}

/// Whether `which` shows each account's folder under it, and the id that is folded or opened.
fn opens(app: &MailApp, which: AllMailbox) -> Option<(bool, Id)> {
    let listed = |id| app.ui.collapsed_folders.contains(&id);
    match which {
        AllMailbox::Inboxes => Some((!listed(ALL_INBOXES), ALL_INBOXES)),
        AllMailbox::Drafts => Some((listed(ALL_DRAFTS), ALL_DRAFTS)),
        AllMailbox::Sent => Some((listed(ALL_SENT), ALL_SENT)),
        AllMailbox::Vips | AllMailbox::Flagged => None,
    }
}

/// What a row of them shows beside its name: All Inboxes and VIPs their unread, Flagged and All
/// Drafts all they hold, All Sent nothing.
fn count(app: &MailApp, counts: &FolderCounts, which: AllMailbox) -> Count {
    let held = all::held(&app.session.mailbox, counts, which);
    match which {
        AllMailbox::Inboxes | AllMailbox::Vips if held.unread > 0 => Count::Unread(held.unread),
        AllMailbox::Flagged | AllMailbox::Drafts if held.total > 0 => Count::Total(held.total),
        _ => Count::Nothing,
    }
}

/// The rows, at the top of the pane.
pub(super) fn section(app: &mut MailApp, ui: &mut Ui, counts: &FolderCounts) {
    let now = all::in_view(&app.session);
    for which in AllMailbox::ALL {
        let opened = opens(app, which);
        let n = count(app, counts, which);
        row(app, ui, which, n, opened, now == Some(which));
        if opened.is_some_and(|(open, _)| open)
            && let Some(kind) = which.kind()
        {
            let folders: Vec<Folder> = app
                .session
                .mailbox
                .accounts_in_order()
                .into_iter()
                .filter_map(|a| app.session.mailbox.folder_of_kind(a.id, kind))
                .filter_map(|id| app.session.mailbox.folder(id).cloned())
                .collect();
            for f in &folders {
                let held = counts.folder(f.id);
                folder_row(app, ui, f, 1, false, "all", Count::of(f.kind, held.total, held.unread), false);
            }
        }
    }
}

/// One of them: its icon, its name and its count, `selected` while the list shows it, with a
/// chevron when it has each account's folder under it (`opened`: whether they show, and the id
/// they fold under).
fn row(app: &mut MailApp, ui: &mut Ui, which: AllMailbox, count: Count, opened: Option<(bool, Id)>, selected: bool) {
    let t = Tokens::get(ui.ctx());
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), ALL_ROW_H), Sense::hover());
    let resp = ui.interact(r, ui.scope_id().with(("all_mailbox", which.code())), Sense::click());
    app.hit.add("allMailbox", AllMailbox::ALL.iter().position(|a| *a == which).unwrap_or(0) as u64, r);
    let hl = highlight(r);
    if selected {
        ui.painter().rect_filled(hl, 6.0, t.accent_soft);
        ui.painter().rect_filled(Rect::from_center_size(pos2(hl.min.x + 1.5, hl.center().y), vec2(3.0, 14.0)), 1.5, t.selection_edge);
    } else if resp.hovered() {
        ui.painter().rect_filled(hl, 6.0, t.hover_nav);
    }
    if resp.has_focus() {
        ui.painter().rect_stroke(hl, 6.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
    }
    let x = r.min.x;
    if let Some((open, id)) = opened {
        let tr = Rect::from_min_max(pos2(x + HIGHLIGHT_LEFT, r.min.y), pos2(x + NAME_LEFT, r.max.y));
        let tresp = ui.interact(tr, ui.scope_id().with(("all_twisty", which.code())), Sense::click());
        chevron(ui.painter(), pos2(x + CHEVRON_X, r.center().y), ROW_CHEVRON, open, if tresp.hovered() { t.accent_text } else { t.text });
        if tresp.clicked() {
            let _ = app.execute("ui.toggleFolder", serde_json::json!({"id": id}));
        }
    }
    let ink = if selected { t.accent_soft_text } else { t.text };
    let icon_r = Rect::from_center_size(pos2(x + NAME_LEFT + 8.0, r.center().y), vec2(16.0, 16.0));
    icons::paint(ui.painter(), icon_r, icon(which), ink, t.accent);
    let mut name_end = r.max.x - COUNT_RIGHT;
    if count != Count::Nothing {
        let (fill, text) = if matches!(count, Count::Unread(_)) { (t.accent_soft, t.accent_soft_text) } else { (t.pill_quiet, t.text_dim) };
        let fill = if selected && matches!(count, Count::Unread(_)) { t.surface } else { fill };
        let galley = ui.painter().layout_no_wrap(count.digits(), bold(NAME_SIZE - 1.0), text);
        let w = (galley.size().x + 12.0).max(20.0);
        let pill = Rect::from_center_size(pos2(r.max.x - COUNT_RIGHT + 4.0 - w / 2.0, r.center().y), vec2(w, 18.0));
        ui.painter().rect_filled(pill, 9.0, fill);
        ui.painter().galley(pill.center() - galley.size() / 2.0, galley, text);
        name_end = pill.min.x - 6.0;
    }
    let left = x + NAME_LEFT + ICON_W;
    // As large as the pane's headings (Favorites, an account's name).
    let font = if selected { bold(HEADING_SIZE) } else { regular(HEADING_SIZE) };
    paint_clipped(ui.painter(), pos2(left, r.center().y - TEXT_RISE), (name_end - left).max(0.0), name(which), font, ink);
    let said = match count {
        Count::Unread(n) => format!("{}, {}", name(which), tln!(n, "{n} unread", "{n} unread")),
        Count::Total(n) => format!("{}, {}", name(which), tln!(n, "{n} message", "{n} messages")),
        Count::Nothing => name(which).to_string(),
    };
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::TreeItem, true, selected, &said));
    if resp.clicked() {
        let _ = app.run(command(which), serde_json::json!({}));
    }
}
