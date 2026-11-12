// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! macOS: the unread mail of every account's Inbox as the badge on the Dock icon, and no badge
//! when there is none. Written from the public documentation of `NSDockTile`, through the
//! generated bindings' safe calls only.

use std::sync::atomic::{AtomicUsize, Ordering};

use objc2::MainThreadMarker;
use objc2_app_kit::NSApplication;
use objc2_foundation::NSString;

/// What the badge says now (`usize::MAX`: not set yet), so that it is set only when it changes.
static SHOWN: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Show `unread` on the Dock icon. Call on the main thread (the window's frames are drawn there);
/// elsewhere nothing is done and the next call tries again.
pub fn show(unread: usize) {
    if SHOWN.load(Ordering::Relaxed) == unread {
        return;
    }
    let Some(mtm) = MainThreadMarker::new() else { return };
    let label = mailcraft_engine::all::badge_text(unread).map(|text| NSString::from_str(&text));
    NSApplication::sharedApplication(mtm).dockTile().setBadgeLabel(label.as_deref());
    SHOWN.store(unread, Ordering::Relaxed);
}
