// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
use mailcraft_engine::Session;
use mailcraft_model::FolderKind;
use serde_json::json;

use super::*;

#[test]
fn the_gap_follows_the_finger_and_stops_growing() {
    assert_eq!(pulled(0.0), 0.0);
    assert_eq!(pulled(-20.0), 0.0);
    assert!(pulled(PULL_TO_REFRESH) > 0.0 && pulled(PULL_TO_REFRESH) < PULL_TO_REFRESH);
    assert_eq!(pulled(10_000.0), MOST_GAP);
}

#[test]
fn a_list_of_every_account_refreshes_them_all_and_a_folder_its_own_account() {
    let mut s = Session::new(mailcraft_engine::sample::several_accounts(1_780_000_000, 0));
    s.run("view.allInboxes", &json!({})).unwrap();
    assert_eq!(target(&s), None);
    let second = s.mailbox.accounts[1].id;
    let inbox = s.mailbox.folder_of_kind(second, FolderKind::Inbox).unwrap();
    s.run("folder.select", &json!({"id": inbox})).unwrap();
    assert_eq!(target(&s), Some(second));
}

#[test]
fn a_trackpad_pull_grows_only_at_the_top_and_scrolling_back_takes_it_away() {
    // Fingers drawing the content down past the top pull it.
    let mut down = 0.0;
    for _ in 0..8 {
        down = pulled_on(down, 10.0, true);
    }
    assert!(down >= PULL_TO_REFRESH);
    // Drawn back up, the pull shrinks and never goes below none.
    assert_eq!(pulled_on(down, -500.0, true), 0.0);
    // Away from the top there is no pull.
    assert_eq!(pulled_on(down, 10.0, false), 0.0);
}
