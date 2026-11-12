// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the owner teaches MailCraft about a sender, in the window
//! ([`mailcraft_model::sender_marks`]):
//!
//! - **Always Show in Priority**: the sender becomes a VIP, and their unread mail in Primary is
//!   always in the Priority box (`people.setVip`).
//! - **Junk learned from the owner**: once three of a sender's messages went to Junk E-mail,
//!   their later mail is marked as junk where it is (a mark on its row and a bar above it, with
//!   Not Junk), not moved.
//! - **NSFW**: a sender's pictures are drawn blurred, with a bar above the message and Unblur
//!   for that message, for this run; then Blur puts the blur back, and so does marking the
//!   sender again.
//! - **An emoji** for a sender, at the start of each of their rows in the list.
//!
//! The menu items are in the message menu; everything they change is kept on this computer only.

use egui::{RichText, Sense, Ui, vec2};
use mailcraft_model::Message;
use serde_json::json;

use crate::MailApp;
use crate::theme::{Tokens, regular, semibold};

/// The emoji offered for a sender, besides None.
pub(crate) const EMOJI: [&str; 30] = [
    "⭐", "❤", "🔥", "💼", "🏠", "👪", "🎸", "🎧", "🎵", "✈", "🚗", "💰", "📝", "📦", "🎁", "📰", "🎉", "🎓", "🏥", "⚽", "🎮", "🍕", "☕", "🐶",
    "📷", "🔒", "⚠", "💡", "📅", "🌱",
];

/// The sender items of the message menu, for the message `m` (and the others selected).
pub(crate) fn menu(app: &mut MailApp, ui: &mut Ui, m: &Message) {
    let ids = app.session.acting_on();
    let address = m.from.email.clone();
    // Priority: a VIP's unread mail is always there, and their mail is listed under VIPs.
    let vip = app.session.mailbox.is_vip(&address);
    let (label, tip) = if vip {
        (tl!("Stop Always Showing in Priority"), tl!("The sender stops being a VIP."))
    } else {
        (
            tl!("Always Show Sender in Priority"),
            tl!("The sender becomes a VIP: their unread mail always shows in Priority, and their mail is listed under VIPs."),
        )
    };
    if ui.button(label).on_hover_text(tip).clicked() {
        let _ = app.run("people.setVip", json!({"ids": ids, "vip": !vip}));
        ui.close();
    }
    // Junk learned from the owner: said here too, and taken back.
    if app.session.mailbox.is_learned_junk(&address) {
        if ui.button(tl!("Sender's Mail Isn't Junk")).clicked() {
            let _ = app.run("people.setJunk", json!({"ids": ids, "junk": false}));
            ui.close();
        }
    } else if ui
        .button(tl!("Mark Sender's Mail as Junk"))
        .on_hover_text(tl!("Their mail is marked as junk where it is, without being moved. MailCraft also learns this when three of their messages go to Junk E-mail."))
        .clicked()
    {
        let _ = app.run("people.setJunk", json!({"ids": ids, "junk": true}));
        ui.close();
    }
    let nsfw = app.session.mailbox.is_nsfw(&address);
    let (label, tip) = if nsfw {
        (tl!("Unmark Sender as NSFW"), tl!("Their pictures are drawn as they are again."))
    } else {
        (tl!("Mark Sender as NSFW"), tl!("Every picture in their mail is drawn blurred, with Unblur above the message."))
    };
    if ui.button(label).on_hover_text(tip).clicked() {
        let _ = app.run("people.setNsfw", json!({"ids": ids, "nsfw": !nsfw}));
        // Marked again, their mail unblurred before is blurred again.
        if !nsfw {
            let mb = &app.session.mailbox;
            app.unblurred.retain(|id| mb.message(*id).is_none_or(|m| !mb.is_nsfw(&m.from.email)));
        }
        ui.close();
    }
    let now = app.session.mailbox.sender_emoji_of(&address).map(str::to_string);
    ui.menu_button(tl!("Sender Emoji"), |ui| {
        ui.spacing_mut().item_spacing = vec2(2.0, 2.0);
        egui::Grid::new("sender_emoji").spacing(vec2(2.0, 2.0)).show(ui, |ui| {
            for (i, e) in EMOJI.iter().enumerate() {
                let on = now.as_deref() == Some(*e);
                if ui.add(egui::Button::selectable(on, RichText::new(*e).size(18.0)).min_size(vec2(30.0, 28.0))).clicked() {
                    let _ = app.run("people.setEmoji", json!({"ids": ids, "emoji": e}));
                    ui.close();
                }
                if i % 6 == 5 {
                    ui.end_row();
                }
            }
        });
        ui.separator();
        if ui.add_enabled(now.is_some(), egui::Button::new(tl!("None"))).clicked() {
            let _ = app.run("people.setEmoji", json!({"ids": ids, "emoji": null}));
            ui.close();
        }
    });
}

/// One bar above a message: an icon's colour, what it says, and its button. Whether the button
/// was pressed.
fn bar(ui: &mut Ui, t: &Tokens, hue: egui::Color32, text: &str, button: &str, hit: &'static str, app: &mut MailApp) -> bool {
    let mut pressed = false;
    let shown = egui::Frame::new()
        .fill(hue.gamma_multiply(0.10))
        .stroke(egui::Stroke::new(1.0, hue.gamma_multiply(0.5)))
        .corner_radius(6.0)
        .inner_margin(egui::Margin { left: 10, right: 6, top: 5, bottom: 5 })
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                let (r, _) = ui.allocate_exact_size(vec2(16.0, 16.0), Sense::hover());
                crate::icons::paint(ui.painter(), r.shrink(1.0), "warning", hue, hue);
                ui.add(egui::Label::new(RichText::new(text).font(regular(11.5)).color(t.text)).wrap());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    pressed = ui.button(RichText::new(button).font(semibold(11.5))).clicked();
                });
            });
        });
    app.hit.add(hit, 0, shown.response.rect);
    pressed
}

/// The bars above a message whose sender the owner taught MailCraft about: junk learned, and
/// pictures blurred.
pub(crate) fn bars(app: &mut MailApp, ui: &mut Ui, m: &Message, t: &Tokens) {
    let address = m.from.email.clone();
    if app.session.mailbox.is_learned_junk(&address) {
        let text = tl!("Marked as junk: you moved this sender's mail to Junk E-mail before.");
        if bar(ui, t, t.red, text, tl!("Not Junk"), "learnedJunkBar", app) {
            let _ = app.run("people.setJunk", json!({"address": address, "junk": false}));
        }
    }
    if app.session.mailbox.is_nsfw(&address) && has_pictures(m) {
        // Blurred, with Unblur; once unblurred, Blur puts it back.
        if crate::reading::blurred(app, m) {
            let text = tl!("Pictures are blurred: you marked this sender as NSFW.");
            if bar(ui, t, t.orange, text, tl!("Unblur"), "nsfwBar", app) {
                app.unblurred.insert(m.id);
            }
        } else if bar(ui, t, t.orange, tl!("Pictures are shown: this sender is marked NSFW."), tl!("Blur"), "nsfwBar", app) {
            app.unblurred.remove(&m.id);
        }
    }
}

/// Whether `m` may show a picture: one attached to it, or one in its HTML.
fn has_pictures(m: &Message) -> bool {
    m.attachments.iter().any(|a| a.mime.to_ascii_lowercase().starts_with("image/"))
        || m.body.html.as_deref().is_some_and(|h| h.to_ascii_lowercase().contains("<img"))
}
