// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The summary above a message's text, written by the model on this computer
//! ([`mailcraft_model::Insight`]): a block the owner can fold away, headed "Summary" with a small
//! AI mark, then the summary, what the mail asks for, and by when.
//!
//! What a model wrote is untrusted, like the mail it read: it is shown as plain text, never as
//! markup, and nothing in it is a link or does anything.
//!
//! - **Summarising…** with a spinner while it is on its way;
//! - nothing for a message shorter than the setting's word count;
//! - **Summary unavailable**, with why on hover, when it failed;
//! - a **Summarize** button where none was made (summaries for new mail off), and **Summarize
//!   conversation** for a message of a conversation, whose summary shows the same way.

use egui::{Sense, Ui, vec2};
use mailcraft_model::Message;
use mailcraft_model::insight::{JobKind, Summary};
use serde_json::json;

use super::Place;
use crate::MailApp;
use crate::theme::{Tokens, regular, semibold};

fn open_id(id: u64) -> egui::Id {
    egui::Id::unique(("aiSummaryOpen", id))
}

pub(super) fn block(app: &mut MailApp, ui: &mut Ui, m: &Message, place: Place, t: &Tokens) {
    let s = &app.session;
    if !s.prefs.ai.enabled || app.services.ai.is_none() {
        return;
    }
    let insight = m.insight.as_deref();
    let summary = insight.and_then(|i| i.summary.clone());
    let failed = insight.map(|i| i.failed.clone()).unwrap_or_default();
    let summarizing = mailcraft_engine::ai::summarizing(s, m.id, JobKind::Summarize);
    let long = mailcraft_engine::ai::words_of(m) >= s.prefs.ai.summary_min_words as usize;
    // The conversation's, kept on its newest message.
    let threads = s.threads();
    let members = threads.members(m.id);
    let newest = members.iter().filter_map(|id| s.mailbox.message(*id).map(|x| (x.date, x.id))).max().map(|(_, id)| id);
    let thread = newest.and_then(|id| s.mailbox.message(id)).and_then(|n| n.insight.as_deref()).and_then(|i| i.thread.clone());
    let thread_running = newest.is_some_and(|id| mailcraft_engine::ai::summarizing(s, id, JobKind::SummarizeThread));
    let conversation = members.len() > 1;

    if summary.is_none() && thread.is_none() && !summarizing && !thread_running && !long && !conversation {
        return;
    }
    let shown = egui::Frame::NONE
        .fill(t.surface)
        .stroke(egui::Stroke::new(1.0, t.border))
        .corner_radius(6.0)
        .inner_margin(egui::Margin { left: 10, right: 10, top: 6, bottom: 6 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing = vec2(6.0, 3.0);
            if let Some(sum) = &summary {
                titled(ui, m.id, tl!("Summary"), sum, t);
            } else if summarizing {
                waiting(ui, tl!("Summarizing…"), t);
            } else if !failed.is_empty() && long {
                ui.label(egui::RichText::new(tl!("Summary unavailable")).font(regular(11.5)).color(t.text_dim)).on_hover_text(tl!(&failed));
            } else if long
                && ui.button(tl!("Summarize")).on_hover_text(tl!("A model on this computer reads this message and summarises it.")).clicked()
            {
                let _ = app.run("ai.summarize", json!({"id": m.id}));
            }
            if conversation {
                if let Some(th) = &thread {
                    titled(ui, m.id.wrapping_add(1 << 62), tl!("Conversation summary"), &th.summary, t);
                } else if thread_running {
                    waiting(ui, tl!("Summarizing the conversation…"), t);
                } else if ui.button(tl!("Summarize conversation")).clicked() {
                    let _ = app.run("ai.summarize", json!({"id": m.id, "conversation": true}));
                }
            }
        });
    if place == Place::Pane {
        app.hit.add("aiSummary", m.id, shown.response.rect);
    }
}

fn waiting(ui: &mut Ui, text: &str, t: &Tokens) {
    ui.horizontal(|ui| {
        ui.add(egui::Spinner::new().size(12.0).color(t.accent));
        ui.label(egui::RichText::new(text).font(regular(11.5)).color(t.text_dim));
    });
}

/// A summary under its title, folded or open (open at first), with the AI mark.
fn titled(ui: &mut Ui, key: u64, title: &str, sum: &Summary, t: &Tokens) {
    let id = open_id(key);
    let open = ui.data(|d| d.get_temp::<bool>(id)).unwrap_or(true);
    let head = ui.horizontal(|ui| {
        ui.label(egui::RichText::new(if open { "▾" } else { "▸" }).font(regular(11.0)).color(t.text_dim));
        ui.label(egui::RichText::new(title).font(semibold(11.5)).color(t.text));
        egui::Frame::NONE
            .stroke(egui::Stroke::new(1.0, t.border_strong))
            .corner_radius(3.0)
            .inner_margin(egui::Margin { left: 4, right: 4, top: 0, bottom: 0 })
            .show(ui, |ui| ui.add(egui::Label::new(egui::RichText::new(tl!("AI")).font(regular(9.5)).color(t.text_dim)).selectable(false)))
            .response
            .on_hover_text(tl!("Written by a model on this computer from the message's text. It can be wrong."));
    });
    let resp = head.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand);
    let said = if open { tl!("Fold the summary") } else { tl!("Show the summary") };
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, said));
    if resp.clicked() {
        ui.data_mut(|d| d.insert_temp(id, !open));
    }
    if !open {
        return;
    }
    // Plain text only: never markup, never a link.
    ui.add(egui::Label::new(egui::RichText::new(&sum.text).font(regular(12.5)).color(t.text)).wrap());
    for a in &sum.actions {
        ui.add(egui::Label::new(egui::RichText::new(format!("•  {a}")).font(regular(12.0)).color(t.text)).wrap());
    }
    if let Some(due) = sum.due {
        let text = crate::i18n::fmt(tl!("Due {date}"), &[("date", &crate::dates::weekday_month_day(due))]);
        ui.label(egui::RichText::new(text).font(semibold(11.5)).color(t.text_dim));
    }
}
