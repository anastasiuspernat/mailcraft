// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The Priority box's rows, as Apple's Mail on iPad has them: the sender, then the summary on the
//! same line, two lines at most, no subject and no time.

use mailcraft_engine::Session;
use mailcraft_model::{FolderKind, InboxCategory, Insight, Summary};
use serde_json::json;

use super::*;
use crate::Services;

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

#[test]
fn a_priority_row_is_two_lines_of_sender_and_summary() {
    let mut app = MailApp::new(Session::at(mailcraft_engine::sample::mailbox(NOW, 0), NOW, 0), Services::default());
    app.autosave = false;
    app.auto_sync = false;
    app.execute("ai.options", json!({"modelPath": "/models/test.gguf", "enabled": true})).unwrap();
    let account = app.session.mailbox.accounts[0].id;
    let inbox = app.session.mailbox.folder_of_kind(account, FolderKind::Inbox).unwrap();
    app.execute("folder.select", json!({"id": inbox})).unwrap();
    let id = app.session.mailbox.messages.values().find(|m| m.folder == inbox && !m.read).map(|m| m.id).unwrap();
    let summary = "Asks whether Thursday still works for the review and wants the slides by noon.";
    app.session.mailbox.message_mut(id).unwrap().insight = Some(Box::new(Insight {
        category: Some(InboxCategory::Primary),
        priority_owner: Some(true),
        summary: Some(Summary { text: summary.into(), ..Default::default() }),
        ..Default::default()
    }));
    app.session.mailbox_changed();
    let mut harness =
        egui_kittest::Harness::builder().with_accessibility_check(false).allow_missing_glyphs().with_size(vec2(1280.0, 900.0)).build_ui_state(
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
    let row = harness.state().hit.rect("message", id).expect("the Priority row is drawn");
    assert_eq!(row.height(), PRIORITY_ROW_H);
    let text = harness.state().hit.rect("priorityText", id).expect("its text");
    // The text has the row's width: no time's column at its end.
    assert_eq!(text.max.x, row.max.x - TIME_RIGHT);
    assert!(text.height() <= 2.0 * PRIORITY_PITCH);
    let sender = harness.state().hit.rect("messageSender", id).unwrap();
    assert!(sender.center().y < text.center().y, "the sender is on the first line");
}
