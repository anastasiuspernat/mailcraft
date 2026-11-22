// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Whether MailCraft is in the background on iPadOS (another app in front, the screen locked).
//! The window library goes on drawing there, and iPadOS doesn't want an app's drawing in the
//! background: a program that keeps its main thread busy that way can be stopped by the system
//! when it is brought back.

/// `true` while the app is in the background; `false` in front (or about to be, or with Control
/// Center pulled over it), and elsewhere than iPadOS. Asked on the main thread only: `false`
/// from any other.
pub fn in_background() -> bool {
    imp::in_background()
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn in_background() -> bool {
        false
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use objc2::MainThreadMarker;
    use objc2_ui_kit::{UIApplication, UIApplicationState};

    pub fn in_background() -> bool {
        MainThreadMarker::new().is_some_and(|mtm| UIApplication::sharedApplication(mtm).applicationState() == UIApplicationState::Background)
    }
}
