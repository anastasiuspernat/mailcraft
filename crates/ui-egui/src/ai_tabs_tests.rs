// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The smart Inbox's tabs above the Inbox: the four parts, then All Mail, on one line.

use egui::accesskit::Role;
use egui_kittest::kittest::{NodeT as _, Queryable};
use serde_json::json;

use crate::nav::tests::window;

type Harness = egui_kittest::Harness<'static, crate::MailApp>;

const NAMES: [&str; 5] = ["Primary", "Transactions", "Updates", "Promotions", "All Mail"];

/// The window on the sample's Inbox with the smart Inbox on.
fn smart_window(width: f32) -> Harness {
    let mut harness = window();
    harness.set_size(egui::vec2(width, 800.0));
    let app = harness.state_mut();
    app.execute("ai.options", json!({"modelPath": "/models/test.gguf", "enabled": true})).unwrap();
    let inbox = app.session.mailbox.folder_of_kind(app.session.mailbox.default_account().unwrap().id, mailcraft_model::FolderKind::Inbox).unwrap();
    app.execute("folder.select", json!({"id": inbox})).unwrap();
    harness.run_steps(4);
    harness
}

/// The tabs, by name, with where each is.
fn tabs(harness: &Harness) -> Vec<(String, egui::Rect)> {
    let mut found: Vec<(String, egui::Rect)> = harness
        .query_all_by_role(Role::Tab)
        .filter_map(|n| {
            let label = n.accesskit_node().label()?;
            let name = NAMES.iter().find(|c| label == **c || label.starts_with(&format!("{c},")))?;
            let b = n.accesskit_node().bounding_box()?;
            Some((name.to_string(), egui::Rect::from_min_max(egui::pos2(b.x0 as f32, b.y0 as f32), egui::pos2(b.x1 as f32, b.y1 as f32))))
        })
        .collect();
    found.sort_by(|a, b| a.1.min.x.total_cmp(&b.1.min.x));
    found
}

#[test]
fn all_mail_is_the_last_tab_and_shows_the_whole_inbox() {
    let mut harness = smart_window(1280.0);
    // The list the window's width, for every tab to be in sight.
    harness.state_mut().execute("view.readingPane", json!({"value": "bottom"})).unwrap();
    harness.run_steps(4);
    let names: Vec<String> = tabs(&harness).into_iter().map(|(n, _)| n).collect();
    assert_eq!(names, NAMES);
    harness.get_by_label("All Mail").click();
    harness.run_steps(4);
    assert_eq!(harness.state().session.view.inbox_category, None);
    // Clicking it again keeps the whole Inbox; Primary takes it back.
    harness.get_by_label("All Mail").click();
    harness.run_steps(4);
    assert_eq!(harness.state().session.view.inbox_category, None);
    harness.get_by_label_contains("Primary").click();
    harness.run_steps(4);
    assert_eq!(harness.state().session.view.inbox_category, Some(mailcraft_model::InboxCategory::Primary));
}

#[test]
fn the_tabs_keep_to_one_line_that_scrolls_in_a_narrow_list() {
    // The reading pane on the right leaves the list too narrow for every tab.
    let mut harness = smart_window(1280.0);
    let found = tabs(&harness);
    assert_eq!(found.len(), NAMES.len());
    let top = found.first().map(|(_, r)| r.min.y).unwrap();
    assert!(found.iter().all(|(_, r)| (r.min.y - top).abs() < 0.5), "{found:?}");
    // All Mail is past the list's edge: a click there lands on the reading pane.
    harness.get_by_label("All Mail").click();
    harness.run_steps(4);
    assert_eq!(harness.state().session.view.inbox_category, Some(mailcraft_model::InboxCategory::Primary));
    // Chosen from the menu, it is scrolled into sight, with its neighbour.
    harness.state_mut().execute("view.inboxCategory", json!({"category": "all"})).unwrap();
    harness.run_steps(6);
    harness.get_by_label_contains("Promotions").click();
    harness.run_steps(4);
    assert_eq!(harness.state().session.view.inbox_category, Some(mailcraft_model::InboxCategory::Promotions));
    harness.get_by_label("All Mail").click();
    harness.run_steps(4);
    assert_eq!(harness.state().session.view.inbox_category, None);
}
