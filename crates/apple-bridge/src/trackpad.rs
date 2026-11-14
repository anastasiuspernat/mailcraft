// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPad's trackpad and mouse (a Magic Keyboard's): two-finger scrolling and where the pointer
//! is, which the window library doesn't pass on. [`install`] adds two gesture recognizers to the
//! window's view: a pan that only scrolling starts (never a finger on the screen, which keeps
//! scrolling as before) and a hover; [`take`] hands over what they saw since the last call.

/// What the trackpad or mouse did, in the view's points.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Pointer {
    /// The pointer is at `(x, y)`.
    Moved {
        x: f64,
        y: f64,
    },
    /// It left the window.
    Gone,
    /// A scroll began, went on by `(dx, dy)` (the content's move, natural scrolling), or ended.
    ScrollStart {
        x: f64,
        y: f64,
    },
    Scroll {
        x: f64,
        y: f64,
        dx: f64,
        dy: f64,
    },
    ScrollEnd,
}

/// Add the recognizers to `window`'s view, once. Nothing elsewhere than iPadOS.
pub fn install(window: &impl raw_window_handle::HasWindowHandle) {
    imp::install(window);
}

/// What was seen since the last call.
pub fn take() -> Vec<Pointer> {
    QUEUE.lock().map(|mut q| std::mem::take(&mut *q)).unwrap_or_default()
}

/// The most kept between two frames.
const MOST: usize = 512;

static QUEUE: std::sync::Mutex<Vec<Pointer>> = std::sync::Mutex::new(Vec::new());

#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
fn push(p: Pointer) {
    if let Ok(mut q) = QUEUE.lock()
        && q.len() < MOST
    {
        q.push(p);
    }
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn install(_window: &impl raw_window_handle::HasWindowHandle) {}
}

#[cfg(target_os = "ios")]
mod imp {
    use std::cell::{Cell, RefCell};

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_foundation::{NSArray, NSNumber, NSObject, NSObjectProtocol};
    use objc2_ui_kit::{UIGestureRecognizerState, UIHoverGestureRecognizer, UIPanGestureRecognizer, UIScrollTypeMask, UIView};
    use raw_window_handle::RawWindowHandle;

    use super::{Pointer, push};

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class has no ivars to drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MailCraftTrackpad"]
        struct Target;

        // SAFETY: NSObjectProtocol has no extra requirements.
        unsafe impl NSObjectProtocol for Target {}

        impl Target {
            // SAFETY: the action's signature is UIKit's: one argument, the recognizer.
            #[unsafe(method(scrolled:))]
            fn scrolled(&self, r: &UIPanGestureRecognizer) {
                let Some(view) = r.view() else { return };
                let at = r.locationInView(Some(&view));
                let moved = r.translationInView(Some(&view));
                match r.state() {
                    UIGestureRecognizerState::Began => {
                        LAST.with(|l| l.set((moved.x, moved.y)));
                        push(Pointer::ScrollStart { x: at.x, y: at.y });
                    }
                    UIGestureRecognizerState::Changed => {
                        let (lx, ly) = LAST.with(|l| l.replace((moved.x, moved.y)));
                        push(Pointer::Scroll { x: at.x, y: at.y, dx: moved.x - lx, dy: moved.y - ly });
                    }
                    _ => {
                        LAST.with(|l| l.set((0.0, 0.0)));
                        push(Pointer::ScrollEnd);
                    }
                }
            }

            // SAFETY: as above.
            #[unsafe(method(hovered:))]
            fn hovered(&self, r: &UIHoverGestureRecognizer) {
                match r.state() {
                    UIGestureRecognizerState::Began | UIGestureRecognizerState::Changed => {
                        if let Some(view) = r.view() {
                            let at = r.locationInView(Some(&view));
                            push(Pointer::Moved { x: at.x, y: at.y });
                        }
                    }
                    _ => push(Pointer::Gone),
                }
            }
        }
    );

    impl Target {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: `init` of NSObject, on an object just allocated.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The target and its two recognizers.
    type Kept = (Retained<Target>, Retained<UIPanGestureRecognizer>, Retained<UIHoverGestureRecognizer>);

    thread_local! {
        /// The pan's translation last seen, to hand on the difference.
        static LAST: Cell<(f64, f64)> = const { Cell::new((0.0, 0.0)) };
        /// The target and recognizers, kept while the window lives (a recognizer holds its
        /// target weakly).
        static KEPT: RefCell<Option<Kept>> = const { RefCell::new(None) };
    }

    pub fn install(window: &impl raw_window_handle::HasWindowHandle) {
        let Some(mtm) = MainThreadMarker::new() else { return };
        if KEPT.with(|k| k.borrow().is_some()) {
            return;
        }
        let Ok(handle) = window.window_handle() else { return };
        let RawWindowHandle::UiKit(kit) = handle.as_raw() else { return };
        // SAFETY: winit's UiKit handle names the window's live UIView, borrowed for as long as
        // `handle` is, and this runs on the main thread.
        let view: &UIView = unsafe { kit.ui_view.cast::<UIView>().as_ref() };
        let target = Target::new(mtm);
        let target_object: &AnyObject = &target;
        // SAFETY: a recognizer just allocated, its target an object kept alive in KEPT and its
        // action a method the target's class defines above.
        let pan =
            unsafe { UIPanGestureRecognizer::initWithTarget_action(UIPanGestureRecognizer::alloc(mtm), Some(target_object), Some(sel!(scrolled:))) };
        // Scrolling only: no finger on the screen starts it (touches keep scrolling as before).
        pan.setAllowedScrollTypesMask(UIScrollTypeMask::All);
        pan.setAllowedTouchTypes(&NSArray::<NSNumber>::new());
        pan.setCancelsTouchesInView(false);
        // SAFETY: as for the pan.
        let hover = unsafe {
            UIHoverGestureRecognizer::initWithTarget_action(UIHoverGestureRecognizer::alloc(mtm), Some(target_object), Some(sel!(hovered:)))
        };
        hover.setCancelsTouchesInView(false);
        view.addGestureRecognizer(&pan);
        view.addGestureRecognizer(&hover);
        KEPT.with(|k| *k.borrow_mut() = Some((target, pan, hover)));
    }
}
