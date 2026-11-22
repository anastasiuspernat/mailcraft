// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Pull to refresh, on a touch screen: with the message list at its top, a finger that takes
//! hold of it and draws it down opens a gap above the rows with a spinner in it; let go far
//! enough down and Send/Receive runs, for every account when the list is one of every account's
//! (All Inboxes and the like), else for the account of the folder in view. The spinner turns
//! for a moment at least, and while that run goes on (for a short while at most); the sync
//! panel says the rest.
//!
//! On iPad a trackpad's two fingers pull too: scrolling up past the list's top draws it down the
//! same way, and lifting them far enough down refreshes (the coasting after they lift doesn't).
//!
//! Nothing here on a computer with a mouse: there, drawing a row down drags the message.

use egui::{Rect, Ui, pos2, vec2};
use serde_json::json;

use crate::MailApp;
use crate::theme::Tokens;

/// How far down the finger goes before letting go refreshes.
pub(crate) const PULL_TO_REFRESH: f32 = 64.0;
/// The tallest the gap gets while pulling.
const MOST_GAP: f32 = 90.0;
/// The gap the spinner turns in after letting go.
const SPIN_GAP: f32 = 44.0;
/// How long the spinner turns at least, and at most, in seconds.
const SPIN_LEAST: f64 = 0.7;
const SPIN_MOST: f64 = 10.0;

#[derive(Clone, Copy, Default)]
struct Pull {
    /// Where the finger came down, with the list at its top.
    from: Option<f32>,
    /// A trackpad's fingers took hold of the list at its top (iPad).
    wheel: bool,
    /// How far the finger, or the fingers, have drawn the list down.
    down: f32,
    /// When a refresh was asked for (the input's time).
    since: Option<f64>,
}

fn key() -> egui::Id {
    egui::Id::unique("pullToRefresh")
}

fn pull(ui: &Ui) -> Pull {
    ui.data(|d| d.get_temp::<Pull>(key())).unwrap_or_default()
}

/// The gap above the rows for this frame, with its spinner painted in it; `0` when there is
/// none. `region` is where the list goes.
pub(crate) fn gap(app: &MailApp, ui: &mut Ui, t: &Tokens) -> f32 {
    let mut p = pull(ui);
    let now = ui.input(|i| i.time);
    let running = app.sync_rx.is_some() || app.graph_rx.is_some();
    if let Some(since) = p.since {
        let age = now - since;
        if age >= SPIN_MOST || (age >= SPIN_LEAST && !running) {
            p.since = None;
            ui.data_mut(|d| d.insert_temp(key(), p));
        }
    }
    let height = if p.since.is_some() { SPIN_GAP } else { pulled(p.down) };
    if height < 1.0 {
        return 0.0;
    }
    let (r, _) = ui.allocate_exact_size(vec2(ui.available_width(), height), egui::Sense::hover());
    ui.painter().rect_filled(r, 0.0, t.list);
    let size = 18.0;
    let at = Rect::from_center_size(pos2(r.center().x, r.max.y - SPIN_GAP / 2.0), vec2(size, size));
    if p.since.is_some() {
        egui::Spinner::new().size(size).color(t.accent).paint_at(ui, at);
    } else {
        // The ring fills as the finger nears the point where letting go refreshes.
        let share = (p.down / PULL_TO_REFRESH).clamp(0.0, 1.0);
        let color = if share >= 1.0 { t.accent } else { t.text_dim };
        let points: Vec<egui::Pos2> = (0..=32)
            .map(|k| {
                let a = -std::f32::consts::FRAC_PI_2 + share * std::f32::consts::TAU * (k as f32 / 32.0);
                at.center() + vec2(a.cos(), a.sin()) * (size / 2.0 - 1.0)
            })
            .collect();
        ui.painter().add(egui::Shape::line(points, egui::Stroke::new(2.0, color)));
    }
    ui.ctx().request_repaint();
    height
}

/// The gap for a finger `down` points down: the further, the less it gives.
fn pulled(down: f32) -> f32 {
    if down <= 0.0 { 0.0 } else { (down * 0.7).min(MOST_GAP) }
}

/// Follow the finger over `region` (the list and its gap), the list scrolled `offset` from its
/// top; letting go far enough down refreshes.
pub(crate) fn track(app: &mut MailApp, ui: &Ui, region: Rect, offset: f32) {
    let touch = ui.input(|i| i.has_touch_screen());
    let mut p = pull(ui);
    let before = (p.from, p.wheel, p.down, p.since);
    let (pressed, down, released, origin, at) = ui.input(|i| {
        (i.pointer.primary_pressed(), i.pointer.primary_down(), i.pointer.primary_released(), i.pointer.press_origin(), i.pointer.latest_pos())
    });
    let top = offset <= 0.5;
    let mut let_go = false;
    if !touch {
        p.from = None;
    } else {
        if pressed && p.since.is_none() && top && origin.is_some_and(|o| region.contains(o)) {
            p.from = origin.map(|o| o.y);
            p.wheel = false;
            p.down = 0.0;
        }
        if let Some(from) = p.from {
            if down && top {
                p.down = at.map_or(p.down, |at| (at.y - from).max(0.0));
            } else {
                // Let go, or the list scrolled instead (no pull then).
                let_go = released || !down;
                p.from = None;
            }
        }
    }
    if app.trackpad_pull {
        let over = at.is_some_and(|at| region.contains(at));
        for (phase, dy) in trackpad(ui) {
            match phase {
                egui::TouchPhase::Start => {
                    if p.since.is_none() && p.from.is_none() && top && over {
                        p.wheel = true;
                        p.down = 0.0;
                    }
                }
                egui::TouchPhase::Move if p.wheel => {
                    p.down = pulled_on(p.down, dy, top);
                }
                egui::TouchPhase::End | egui::TouchPhase::Cancel if p.wheel => {
                    let_go = phase == egui::TouchPhase::End;
                    p.wheel = false;
                }
                _ => {}
            }
        }
    } else {
        p.wheel = false;
    }
    if p.from.is_none() && !p.wheel {
        if let_go && p.down >= PULL_TO_REFRESH && refresh(app) {
            p.since = Some(ui.input(|i| i.time));
        }
        p.down = 0.0;
    }
    if (p.from, p.wheel, p.down, p.since) != before {
        ui.data_mut(|d| d.insert_temp(key(), p));
        ui.ctx().request_repaint();
    }
}

/// This frame's trackpad scrolling (fingers on it, not the coasting after): each event's phase and
/// how far it moved the content down (natural scrolling).
fn trackpad(ui: &Ui) -> Vec<(egui::TouchPhase, f32)> {
    ui.input(|i| {
        i.events
            .iter()
            .filter_map(|e| match e {
                egui::Event::MouseWheel { source: egui::MouseWheelSource::Trackpad, phase, delta, unit, .. } => {
                    let dy = if *unit == egui::MouseWheelUnit::Point { delta.y } else { 0.0 };
                    Some((*phase, dy))
                }
                _ => None,
            })
            .collect()
    })
}

/// How far a trackpad pull is after the fingers moved the content `dy` down: it grows only
/// with the list at its top, and scrolling back into the list takes it away.
fn pulled_on(down: f32, dy: f32, top: bool) -> f32 {
    if top { (down + dy).max(0.0) } else { 0.0 }
}

/// Send/Receive for what the list shows: every account for a list of all of them, else the
/// account of the folder in view. Whether it started (or is under way already).
fn refresh(app: &mut MailApp) -> bool {
    if app.sync_rx.is_some() || app.graph_rx.is_some() {
        return true;
    }
    let params = match target(&app.session) {
        Some(account) => json!({ "account": account }),
        None => json!({}),
    };
    // (A failure shows in the status bar, as the button's does.)
    app.run("ui.sync", params).is_ok()
}

/// The account a pull refreshes: `None` for every account (a list of all of them).
pub(crate) fn target(s: &mailcraft_engine::Session) -> Option<mailcraft_model::Id> {
    if mailcraft_engine::all::in_view(s).is_some() {
        return None;
    }
    s.mailbox.folder(s.view.folder).map(|f| f.account)
}

#[cfg(test)]
#[path = "pull_refresh_tests.rs"]
mod pull_refresh_tests;
