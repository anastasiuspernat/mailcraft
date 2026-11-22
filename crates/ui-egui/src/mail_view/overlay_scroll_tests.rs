// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The message list where scroll bars float over what they scroll, as a Mac's do: no lane, no
//! arrows, rows to the list's right edge.

use mailcraft_engine::Session;
use serde_json::json;

use super::*;
use crate::Services;

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

type Window = egui_kittest::Harness<'static, MailApp>;

/// The sample mailbox with 60 more messages in the Inbox, so that the list scrolls.
fn app(overlay: bool) -> MailApp {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    app.overlay_scroll_bars = overlay;
    let inbox = app.session.view.folder;
    for i in 0..60u32 {
        let from = mailcraft_model::Address::new(format!("Sender {i}"), format!("s{i}@example.org"));
        let m = Message {
            folder: inbox,
            subject: format!("Message {i}"),
            from,
            date: NOW - 3600,
            received: NOW - 3600 - i64::from(i),
            ..Default::default()
        };
        app.session.mailbox.add_message(m).unwrap();
    }
    app.session.mailbox_changed();
    app
}

fn window(app: MailApp) -> Window {
    let mut harness = egui_kittest::Harness::builder()
        .with_accessibility_check(false)
        .allow_missing_glyphs()
        .with_size(vec2(1280.0, 800.0))
        .with_step_dt(1.0 / 60.0)
        .build_ui_state(
            |ui, app: &mut MailApp| {
                let ctx = ui.ctx().clone();
                app.logic(&ctx);
                app.ui(ui);
            },
            app,
        );
    harness.run_steps(4);
    let _ = harness.state_mut().execute("ui.dismissReminder", json!({"all": true}));
    harness.run_steps(3);
    harness
}

/// The rows that were drawn, top to bottom, each with where it is.
fn drawn(harness: &mut Window) -> Vec<(Row, Rect)> {
    let rows = harness.state_mut().session.rows();
    rows.iter().filter_map(|row| harness.state().hit.rect("message", row.id).map(|rect| (row.clone(), rect))).collect()
}

#[test]
fn rows_reach_the_lists_right_edge_and_there_are_no_arrows() {
    let mut lane = window(app(false));
    let mut overlay = window(app(true));
    let (with_lane, floating) = (drawn(&mut lane), drawn(&mut overlay));
    let (Some((_, kept)), Some((_, full))) = (with_lane.first(), floating.first()) else { panic!("no rows drawn") };
    // The same list, as wide: its rows start where they did and end where the lane would start.
    assert_eq!(kept.min.x, full.min.x);
    assert!((full.max.x - kept.max.x - SCROLL_LANE).abs() < 0.01, "{kept:?} {full:?}");
    // The header's arrangement moves out with them.
    let sort = |h: &Window| h.state().hit.rect("listSort", 1).unwrap();
    assert!((sort(&overlay).max.x - sort(&lane).max.x - SCROLL_LANE).abs() < 0.01);
    // The list scrolls, with the lane's arrows in one window and none in the other.
    assert!(lane.state().hit.rect("listScroll", 0).is_some() && lane.state().hit.rect("listScroll", 1).is_some());
    assert!(overlay.state().hit.rect("listScroll", 0).is_none() && overlay.state().hit.rect("listScroll", 1).is_none());
}

/// Without a lane or arrows, the list still scrolls under the wheel.
#[test]
fn a_list_with_a_floating_bar_still_scrolls() {
    let mut harness = window(app(true));
    let rows = drawn(&mut harness);
    let Some((row, was)) = rows.get(3).cloned() else { panic!("too few rows") };
    harness.event(egui::Event::PointerMoved(was.center()));
    harness.step();
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -200.0),
        phase: egui::TouchPhase::Move,
        modifiers: egui::Modifiers::NONE,
        source: egui::MouseWheelSource::Unknown,
    });
    harness.run_steps(30);
    let now = harness.state().hit.rect("message", row.id);
    assert!(now.is_none_or(|now| now.min.y < was.min.y), "{now:?} {was:?}");
    assert!(harness.state().hit.rect("listScroll", 0).is_none());
}

/// Off a Mac, the choice is the lane's unless a test asks for the other.
#[test]
fn only_a_mac_floats_its_scroll_bars_by_default() {
    let app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    assert_eq!(app.overlay_scroll_bars, cfg!(target_os = "macos"));
}

/// The floating bar shows while the list moves and a moment after, then fades away.
#[test]
fn the_floating_bar_shows_while_the_list_moves_then_fades() {
    assert_eq!(overlay_fade(0.0), 1.0);
    assert_eq!(overlay_fade(OVERLAY_STAYS), 1.0);
    let half = overlay_fade(OVERLAY_STAYS + OVERLAY_FADES / 2.0);
    assert!(half > 0.4 && half < 0.6, "{half}");
    assert_eq!(overlay_fade(OVERLAY_STAYS + OVERLAY_FADES), 0.0);
    assert_eq!(overlay_fade(f64::INFINITY), 0.0, "a list never moved shows no bar");

    let mut harness = window(app(true));
    let moved = |h: &Window| h.ctx.data(|d| d.get_temp::<ListMoved>(moved_key())).map(|m| m.at);
    assert_eq!(moved(&harness), Some(f64::NEG_INFINITY), "opening the list is no move");
    let rows = drawn(&mut harness);
    let middle = rows[rows.len() / 2].1.center();
    harness.event(egui::Event::PointerMoved(middle));
    harness.event(egui::Event::MouseWheel {
        unit: egui::MouseWheelUnit::Point,
        delta: vec2(0.0, -200.0),
        modifiers: egui::Modifiers::NONE,
        phase: egui::TouchPhase::Move,
        source: egui::MouseWheelSource::Unknown,
    });
    harness.run_steps(10);
    assert!(moved(&harness).is_some_and(f64::is_finite), "a scroll is a move");
}
