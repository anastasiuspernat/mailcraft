// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The smart Inbox's row of tabs above an Inbox's list: Primary, Transactions, Updates and
//! Promotions, each with its unread count (`view.inboxCategory`), and last All Mail, the whole
//! Inbox (as View › Smart Inbox › All Mail). Shown while the smart Inbox is on and the folder in view
//! is an Inbox whose Gmail tabs aren't showing. The tabs stay on one line, which scrolls sideways
//! when the list is too narrow for them; the arrow keys and Enter work as on any tab. While mail
//! already here is being sorted, the row says how much is left.

use egui::{Rect, Sense, Stroke, Ui, pos2, vec2};
use mailcraft_model::InboxCategory;
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, bold, paint_clipped, regular, semibold};

const ROW_H: f32 = 32.0;
const LEFT: f32 = 22.0;
const PAD: f32 = 8.0;
const NAME_SIZE: f32 = 13.0;
const COUNT_SIZE: f32 = 11.0;

/// A part's name in the window's language.
pub(crate) fn name(c: InboxCategory) -> &'static str {
    match c {
        InboxCategory::Primary => tl!("Primary"),
        InboxCategory::Transactions => tl!("Transactions"),
        InboxCategory::Updates => tl!("Updates"),
        InboxCategory::Promotions => tl!("Promotions"),
    }
}

/// Whether the row shows above the list in view.
pub(crate) fn shown(app: &MailApp) -> bool {
    let s = &app.session;
    s.prefs.ai.enabled
        && s.view.module == mailcraft_engine::Module::Mail
        && mailcraft_engine::ai::lists_inboxes(s)
        && mailcraft_engine::gmail::in_view(s).is_none()
        && !(s.view.gmail_tabs && mailcraft_engine::gmail::has_categories(s, s.view.folder))
}

/// The tabs, in order: each part of the Inbox, then All Mail (`None`), the whole Inbox.
const TABS: [Option<InboxCategory>; 5] =
    [Some(InboxCategory::Primary), Some(InboxCategory::Transactions), Some(InboxCategory::Updates), Some(InboxCategory::Promotions), None];

/// A tab's name: a part's, or All Mail.
fn tab_name(tab: Option<InboxCategory>) -> &'static str {
    tab.map_or(tl!("All Mail"), name)
}

pub(crate) fn row(app: &mut MailApp, ui: &mut Ui) {
    if !shown(app) {
        return;
    }
    let t = Tokens::get(ui.ctx());
    let inbox = app.session.view.folder;
    let now = app.session.view.inbox_category;
    let counts = app.session.folder_counts();
    // (Over All Inboxes, every account's Inbox.)
    let inboxes = mailcraft_engine::ai::inboxes_in_view(&app.session);
    let ctx = ui.ctx().clone();
    let measure = |text: String, font| ctx.fonts_mut(|f| f.layout_no_wrap(text, font, t.text).size().x);
    // Each tab: what it shows, its unread (none beside All Mail: it is the others' sum), its x
    // in the strip and its width.
    let mut x = LEFT - PAD;
    let tabs: Vec<(Option<InboxCategory>, usize, f32, f32, f32)> = TABS
        .into_iter()
        .map(|tab| {
            let unread = tab.map_or(0, |c| inboxes.iter().map(|f| counts.smart(*f, c).unread).sum::<usize>());
            let name_w = measure(tab_name(tab).to_string(), semibold(NAME_SIZE));
            let pill_w = if unread > 0 { (measure(unread.to_string(), bold(COUNT_SIZE)) + 10.0).max(18.0) + 6.0 } else { 0.0 };
            let w = name_w + pill_w + 2.0 * PAD;
            let at = x;
            x += w;
            (tab, unread, name_w, pill_w, at)
        })
        .collect();
    let strip_w = x + PAD;
    // What is left to sort, at the row's end when the tabs leave room for it.
    let left = app.session.ai.backlog + app.session.ai.host.working;
    let note = (left > 0).then(|| tln!(left, "Sorting your Inbox… {n} left", "Sorting your Inbox… {n} left"));
    let width = ui.available_width();
    let (r, _) = ui.allocate_exact_size(vec2(width, ROW_H), Sense::hover());
    ui.painter().hline(r.x_range(), r.max.y - 0.5, Stroke::new(1.0, t.border));
    if let Some(note) = &note {
        let note_w = measure(note.clone(), regular(11.5));
        if strip_w + note_w + 16.0 < width {
            let at = pos2(r.max.x - 12.0 - note_w, r.center().y - 1.0);
            paint_clipped(ui.painter(), at, note_w + 1.0, note, regular(11.5), t.text_dim);
        }
    }
    // One line that scrolls sideways when the list is narrower than the tabs, never wraps.
    let mut line = ui.new_child(egui::UiBuilder::new().max_rect(r).layout(egui::Layout::left_to_right(egui::Align::Min)));
    egui::ScrollArea::horizontal().id_salt(("smart_tabs", inbox)).auto_shrink([false, false]).show(&mut line, |ui| {
        let (strip, _) = ui.allocate_exact_size(vec2(strip_w, ROW_H), Sense::hover());
        // The chosen tab is brought into sight when it changes (from the menu, a key, a command).
        let seen_id = ui.scope_id().with("seen");
        let seen: Option<Option<InboxCategory>> = ui.ctx().data(|d| d.get_temp(seen_id));
        for (i, (tab, unread, name_w, pill_w, x)) in tabs.into_iter().enumerate() {
            let w = name_w + pill_w + 2.0 * PAD;
            let rect = Rect::from_min_max(pos2(strip.min.x + x, strip.min.y + 2.0), pos2(strip.min.x + x + w, strip.max.y));
            let resp = ui.interact(rect, ui.scope_id().with(("smart_tab", inbox, tab.map_or("all", |c| c.code()))), Sense::click());
            app.hit.add("smartTab", i as u64, rect);
            let on = now == tab;
            if on && seen != Some(now) {
                ui.scroll_to_rect(rect, None);
                ui.ctx().data_mut(|d| d.insert_temp(seen_id, now));
            }
            if resp.has_focus() {
                resp.scroll_to_me(None);
            }
            let under = Rect::from_min_max(pos2(rect.min.x + 2.0, rect.max.y - 2.5), pos2(rect.max.x - 2.0, rect.max.y));
            if on {
                ui.painter().rect_filled(under, 1.0, t.accent_text);
            } else if resp.hovered() {
                ui.painter().rect_filled(rect.shrink2(vec2(1.0, 3.0)), 4.0, t.hover_subtle);
                ui.painter().rect_filled(under, 1.0, t.border_strong);
            }
            if resp.has_focus() {
                ui.painter().rect_stroke(rect.shrink2(vec2(1.0, 3.0)), 4.0, Stroke::new(1.5, t.accent), egui::StrokeKind::Inside);
            }
            let mid = rect.center().y - 1.0;
            let font = if on { semibold(NAME_SIZE) } else { regular(NAME_SIZE) };
            let ink = if on { t.accent_text } else { t.text };
            paint_clipped(ui.painter(), pos2(rect.min.x + PAD, mid), name_w + 1.0, tab_name(tab), font, ink);
            if unread > 0 {
                let galley = ui.painter().layout_no_wrap(unread.to_string(), bold(COUNT_SIZE), t.accent_soft_text);
                let pill = Rect::from_min_size(pos2(rect.min.x + PAD + name_w + 6.0, mid - 8.5), vec2(pill_w - 6.0, 17.0));
                ui.painter().rect_filled(pill, 8.5, t.accent_soft);
                ui.painter().galley(pill.center() - galley.size() / 2.0, galley, t.accent_soft_text);
            }
            let said =
                if unread > 0 { format!("{}, {}", tab_name(tab), tln!(unread, "{n} unread", "{n} unread")) } else { tab_name(tab).to_string() };
            resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Tab, true, &said));
            ui.ctx().accesskit_node_builder(resp.id, |node| node.set_selected(on));
            if resp.clicked() && !on {
                let _ = app.run("view.inboxCategory", json!({"category": tab.map_or("all", |c| c.code())}));
            }
        }
    });
}

/// The message menu's Move to (the smart Inbox's parts), for Inbox mail while the smart Inbox is
/// on: each part, and under them the same for every later message from the senders.
pub(crate) fn menu(app: &mut MailApp, ui: &mut Ui) {
    if !app.session.prefs.ai.enabled {
        return;
    }
    let ids = app.session.acting_on();
    let s = &app.session;
    if !ids.iter().filter_map(|id| s.mailbox.message(*id)).any(|m| mailcraft_engine::ai::sorts(s, m)) {
        return;
    }
    ui.menu_button(tl!("Move to Inbox part"), |ui| {
        for c in InboxCategory::ALL {
            if ui.add(egui::Button::new(name(c)).min_size(vec2(170.0, 0.0))).clicked() {
                let _ = app.run("ai.setCategory", json!({"ids": ids, "category": c.code()}));
                ui.close();
            }
        }
        ui.separator();
        ui.menu_button(tl!("Always for this sender"), |ui| {
            for c in InboxCategory::ALL {
                if ui.add(egui::Button::new(name(c)).min_size(vec2(170.0, 0.0))).clicked() {
                    let _ = app.run("ai.setCategory", json!({"ids": ids, "category": c.code(), "sender": true}));
                    ui.close();
                }
            }
        });
    });
    // The Priority section: out of it, or into it, by the owner's word.
    let s = &app.session;
    let primary: Vec<&mailcraft_model::Message> = ids
        .iter()
        .filter_map(|id| s.mailbox.message(*id))
        .filter(|m| mailcraft_engine::ai::sorts(s, m) && mailcraft_engine::ai::category_of(m) == InboxCategory::Primary)
        .collect();
    if primary.is_empty() {
        return;
    }
    let urgent = primary.iter().any(|m| mailcraft_engine::ai::is_priority(s, m));
    let (label, priority) = if urgent { (tl!("Not a Priority"), false) } else { (tl!("Mark as Priority"), true) };
    if ui.button(label).clicked() {
        let _ = app.run("ai.setPriority", json!({"ids": ids, "priority": priority}));
        ui.close();
    }
}

/// Whether rows of the list show the smart Inbox's summaries: it is on, and so are summaries in the
/// list.
pub(crate) fn summaries_shown(s: &mailcraft_engine::Session) -> bool {
    s.prefs.ai.enabled && s.prefs.ai.list_summaries
}

/// The summary a row of the list shows in place of its preview: the smart Inbox is on, summaries
/// show in the list, and the message has one.
pub(crate) fn list_summary<'a>(s: &mailcraft_engine::Session, m: &'a mailcraft_model::Message) -> Option<&'a str> {
    if !summaries_shown(s) || !mailcraft_engine::ai::sorts(s, m) {
        return None;
    }
    m.insight.as_deref().and_then(|i| i.summary.as_ref()).map(|x| x.text.as_str()).filter(|t| !t.trim().is_empty())
}

/// Names as a list in words: "Ada", "Ada and Ben", "Ada, Ben and Cy", and "and 2 others" past three.
fn names_in_words(names: &[String], others: usize) -> String {
    let and = |a: &str, b: &str| crate::i18n::fmt(tl!("{first} and {last}"), &[("first", a), ("last", b)]);
    match (names, others) {
        ([], _) => String::new(),
        ([one], 0) => one.clone(),
        (many, 0) => {
            let (last, rest) = many.split_last().map(|(l, r)| (l.as_str(), r.join(", "))).unwrap_or_default();
            and(&rest, last)
        }
        (many, n) => and(&many.join(", "), &tln!(n, "{n} other", "{n} others")),
    }
}

/// What a "more" row says: "3 more from Shop", "5 more from Ada, Ben and Cy", or, when they are
/// shown, "Show fewer".
pub(crate) fn more_text(more: &mailcraft_engine::list::More) -> String {
    if more.open {
        return tl!("Show fewer").to_string();
    }
    let n = more.count;
    let from = names_in_words(&more.names, more.others);
    crate::i18n::fmt(&tln!(n, "{n} more from {from}", "{n} more from {from}"), &[("from", &from)])
}

/// A "more" row of the smart Inbox's list: one line, with the folded rows' unread count; a click
/// shows them, or folds them again (`view.smartGroup`).
pub(crate) fn more_row(app: &mut MailApp, ui: &mut Ui, row: &mailcraft_engine::list::Row, rect: Rect, left: f32) {
    let Some(more) = &row.more else { return };
    let t = Tokens::get(ui.ctx());
    let resp =
        ui.interact(rect, ui.scope_id().with(("smart_more", &more.key, row.id)), Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    app.hit.add("smartMore", row.id, rect);
    if resp.hovered() {
        ui.painter().rect_filled(rect.shrink2(vec2(2.0, 1.0)), 4.0, t.hover);
    }
    let p = ui.painter();
    let y = rect.center().y;
    let x = rect.min.x + left;
    p.text(pos2(x - 12.0, y), egui::Align2::CENTER_CENTER, if more.open { "▾" } else { "▸" }, regular(11.0), t.accent);
    let text = more_text(more);
    let w = paint_clipped(p, pos2(x, y), (rect.max.x - x - 60.0).max(0.0), &text, semibold(11.5), t.accent);
    if !more.open && row.unread > 0 {
        let unread = tln!(row.unread, "{n} unread", "{n} unread");
        paint_clipped(p, pos2(x + w + 8.0, y), (rect.max.x - x - w - 16.0).max(0.0), &unread, regular(11.0), t.text_dim);
    }
    let said = text.clone();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, &said));
    if resp.clicked() {
        let _ = app.run("view.smartGroup", json!({"key": more.key, "open": !more.open}));
    }
}

/// The part a row's message wears as a chip: while the smart Inbox is on and the whole Inbox is
/// listed (View › Smart Inbox › All Mail), for its sorted mail; Primary goes without one.
pub(crate) fn chip(s: &mailcraft_engine::Session, m: &mailcraft_model::Message) -> Option<InboxCategory> {
    if !s.prefs.ai.enabled || s.view.inbox_category.is_some() || !mailcraft_engine::ai::sorts(s, m) {
        return None;
    }
    m.insight.as_deref().and_then(|i| i.category).filter(|c| *c != InboxCategory::Primary)
}

/// What the list says under the search box of search by meaning, while a search is made: how
/// many it found by meaning, that the search's meaning is on its way, or how much mail is left to
/// read for it.
pub(crate) fn meaning_note(s: &mailcraft_engine::Session, rows: &[mailcraft_engine::list::Row]) -> Option<String> {
    let text = mailcraft_engine::ai::meaning_text(s)?;
    let left = s.ai.embed_backlog;
    if mailcraft_engine::ai::query_vector(s, &text).is_none() {
        return Some(tl!("Looking for mail that means the same…").to_string());
    }
    let found = rows.iter().filter(|r| r.meaning).count();
    let said = match found {
        0 => tl!("Nothing more found by meaning.").to_string(),
        n => tln!(n, "{n} more found by meaning.", "{n} more found by meaning."),
    };
    Some(if left > 0 { format!("{said} {}", tln!(left, "{n} message still to read for it.", "{n} messages still to read for it.")) } else { said })
}
