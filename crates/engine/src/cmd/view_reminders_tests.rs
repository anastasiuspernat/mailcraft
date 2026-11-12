// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The reminders still to come, with the moment each goes off: what iPadOS is handed in advance.

use serde_json::json;

use super::upcoming_reminders;
use crate::Session;

/// Wednesday 2026-10-14 16:00 UTC.
const NOW: i64 = 1_791_993_600;

#[test]
fn upcoming_reminders_go_off_their_minutes_before_and_only_ahead() {
    let mut s = Session::at(mailcraft_model::Mailbox::with_local_account("Owner", "owner@example.org"), NOW, 0);
    // Tomorrow at ten, fifteen minutes ahead; one an hour ago; one without a reminder; a daily one.
    s.run("calendar.new", &json!({"title": "Dentist", "start": "2026-10-15T10:00", "reminder": 15})).unwrap();
    s.run("calendar.new", &json!({"title": "Past", "start": "2026-10-14T15:00", "reminder": 5})).unwrap();
    s.run("calendar.new", &json!({"title": "Quiet", "start": "2026-10-15T11:00", "reminder": null})).unwrap();
    s.run("calendar.new", &json!({"title": "Stand-up", "start": "2026-10-15T09:30", "reminder": 10, "recurrence": {"freq": "daily"}})).unwrap();
    let now = s.local_now();
    let coming = upcoming_reminders(&s, now, 2, 64);
    let said: Vec<(&str, i64)> = coming.iter().map(|(r, at)| (r.title.as_str(), *at)).collect();
    let at = |h: i64, m: i64| NOW + 8 * 3600 + h * 3600 + m * 60; // midnight of the 15th is 8 h past 16:00
    assert_eq!(said, [("Stand-up", at(9, 20)), ("Dentist", at(9, 45)), ("Stand-up", at(24 + 9, 20))], "{said:?}");
    // At most `max`, soonest first.
    assert_eq!(upcoming_reminders(&s, now, 2, 1).len(), 1);
}
