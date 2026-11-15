// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The menu bar's bookkeeping, which is the same everywhere: items chosen wait to be taken, only
//! while they can be chosen, and a command's tag is read back only when it is one of ours.

use super::*;

#[test]
fn a_tag_is_read_back_only_from_our_own_commands() {
    assert_eq!(tag_of("mailcraft.12"), Some(12));
    for other in ["", "mailcraft.", "mailcraft.-1", "mailcraft.x", "other.3", "mailcraft.99999999999999999999999"] {
        assert_eq!(tag_of(other), None, "{other}");
    }
}

#[test]
fn what_is_chosen_waits_once_and_wakes_the_window() {
    let woken = std::rc::Rc::new(std::cell::Cell::new(0));
    let w = woken.clone();
    on_choice(move || w.set(w.get() + 1));
    set_enabled(vec![true, false]);
    assert!(is_enabled(0) && !is_enabled(1) && !is_enabled(7));
    choose(0);
    choose(0);
    assert_eq!((take(), take(), woken.get()), (vec![0, 0], vec![], 2));
    for _ in 0..MOST_CHOSEN + 5 {
        choose(1);
    }
    assert_eq!(take().len(), MOST_CHOSEN);
}
