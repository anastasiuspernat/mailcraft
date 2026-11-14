// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Whether the iPad is in its dark appearance, which the window library doesn't report on iPadOS.
//! [`install`] keeps the window's view, and [`dark`] asks it each time (the system's own answer,
//! following Settings › Display & Brightness and its schedule).

/// Keep `window`'s view, to ask it later. Nothing elsewhere than iPadOS.
pub fn install(window: &impl raw_window_handle::HasWindowHandle) {
    imp::install(window);
}

/// `Some(true)` when the window is dark, `Some(false)` when light, `None` when nothing says.
pub fn dark() -> Option<bool> {
    imp::dark()
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn install(_window: &impl raw_window_handle::HasWindowHandle) {}
    pub fn dark() -> Option<bool> {
        None
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::{MainThreadMarker, Message};
    use objc2_ui_kit::{UITraitEnvironment, UIUserInterfaceStyle, UIView};
    use raw_window_handle::RawWindowHandle;

    thread_local! {
        /// The window's view, kept while the window lives.
        static VIEW: RefCell<Option<Retained<UIView>>> = const { RefCell::new(None) };
    }

    pub fn install(window: &impl raw_window_handle::HasWindowHandle) {
        if MainThreadMarker::new().is_none() || VIEW.with(|v| v.borrow().is_some()) {
            return;
        }
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::UiKit(kit) = handle.as_raw() else { return };
        // SAFETY: winit's UiKit handle names the window's live UIView, and this runs on the main
        // thread; retaining it keeps it alive for as long as it is kept here.
        let view: Retained<UIView> = unsafe { kit.ui_view.cast::<UIView>().as_ref() }.retain();
        VIEW.with(|v| *v.borrow_mut() = Some(view));
    }

    pub fn dark() -> Option<bool> {
        MainThreadMarker::new()?;
        VIEW.with(|v| {
            let view = v.borrow();
            let traits = view.as_ref()?.traitCollection();
            // SAFETY: read on the main thread, where the view's traits belong.
            match unsafe { traits.userInterfaceStyle() } {
                UIUserInterfaceStyle::Dark => Some(true),
                UIUserInterfaceStyle::Light => Some(false),
                _ => None,
            }
        })
    }
}
