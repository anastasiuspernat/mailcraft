// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The iPad's trackpad and mouse (a Magic Keyboard's): two-finger scrolling and where the pointer
//! is, which the window library doesn't pass on. [`install`] gives the window's view the scrolling
//! of a real `UIScrollView`, so that it moves as everywhere else on the iPad (the system's own
//! acceleration and coasting after the fingers lift): an invisible scroll view that takes no
//! touches, whose pan recognizer is moved onto the window's view and only answers scrolling
//! (never a finger on the screen, which keeps scrolling as before), and a hover recognizer for the
//! pointer. What the scroll view's offset does is handed on as it happens; [`take`] hands over what
//! was seen since the last call, and [`on_input`] names who to wake when something was.

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

/// Call `wake` (on the main thread) whenever something arrives, so that a window that is idle draws
/// a frame and takes it.
pub fn on_input(wake: impl Fn() + 'static) {
    imp::on_input(wake);
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
    pub fn on_input(_wake: impl Fn() + 'static) {}
}

#[cfg(target_os = "ios")]
mod imp {
    use std::cell::{Cell, RefCell};

    use objc2::rc::Retained;
    use objc2::runtime::{AnyObject, ProtocolObject};
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send, sel};
    use objc2_core_foundation::{CGPoint, CGSize};
    use objc2_foundation::{NSArray, NSNumber, NSObject, NSObjectProtocol};
    use objc2_ui_kit::{UIGestureRecognizerState, UIHoverGestureRecognizer, UIScrollTypeMask, UIScrollView, UIScrollViewDelegate, UIView};
    use raw_window_handle::RawWindowHandle;

    use super::{Pointer, push};

    /// The scroll view's content: far more than any one scroll goes, both ways from the middle.
    const ROOM: f64 = 4_000_000.0;
    const MIDDLE: CGPoint = CGPoint { x: ROOM / 2.0, y: ROOM / 2.0 };

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class has no ivars to drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MailCraftTrackpad"]
        struct Target;

        // SAFETY: NSObjectProtocol has no extra requirements.
        unsafe impl NSObjectProtocol for Target {}

        // SAFETY: the methods below have UIScrollViewDelegate's signatures.
        unsafe impl UIScrollViewDelegate for Target {
            #[unsafe(method(scrollViewWillBeginDragging:))]
            fn will_begin_dragging(&self, sv: &UIScrollView) {
                let pan = sv.panGestureRecognizer();
                let at = pan.view().map(|v| pan.locationInView(Some(&v))).unwrap_or(CGPoint { x: 0.0, y: 0.0 });
                // A new scroll while the last one coasts: that one is over (the system stopped it).
                if SCROLLING.with(|s| s.replace(true)) {
                    push(Pointer::ScrollEnd);
                }
                push(Pointer::ScrollStart { x: at.x, y: at.y });
                wake();
            }

            #[unsafe(method(scrollViewDidScroll:))]
            fn did_scroll(&self, sv: &UIScrollView) {
                let now = sv.contentOffset();
                let was = LAST.with(|l| l.replace(now));
                let (dx, dy) = (was.x - now.x, was.y - now.y);
                if (dx != 0.0 || dy != 0.0) && SCROLLING.with(Cell::get) {
                    let at = HOVER.with(Cell::get);
                    push(Pointer::Scroll { x: at.x, y: at.y, dx, dy });
                    wake();
                }
            }

            #[unsafe(method(scrollViewDidEndDragging:willDecelerate:))]
            fn did_end_dragging(&self, sv: &UIScrollView, decelerate: bool) {
                if !decelerate {
                    finish(sv);
                }
            }

            #[unsafe(method(scrollViewDidEndDecelerating:))]
            fn did_end_decelerating(&self, sv: &UIScrollView) {
                finish(sv);
            }
        }

        impl Target {
            // SAFETY: the action's signature is UIKit's: one argument, the recognizer.
            #[unsafe(method(hovered:))]
            fn hovered(&self, r: &UIHoverGestureRecognizer) {
                match r.state() {
                    UIGestureRecognizerState::Began | UIGestureRecognizerState::Changed => {
                        if let Some(view) = r.view() {
                            let at = r.locationInView(Some(&view));
                            HOVER.with(|h| h.set(at));
                            push(Pointer::Moved { x: at.x, y: at.y });
                        }
                    }
                    _ => push(Pointer::Gone),
                }
                wake();
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

    /// A scroll is over: say so, and put the offset back in the middle for the next one.
    fn finish(sv: &UIScrollView) {
        if SCROLLING.with(|s| s.replace(false)) {
            push(Pointer::ScrollEnd);
            wake();
        }
        LAST.with(|l| l.set(MIDDLE));
        sv.setContentOffset(MIDDLE);
    }

    fn wake() {
        WAKE.with(|w| {
            if let Some(w) = &*w.borrow() {
                w();
            }
        });
    }

    /// The target, the scroll view and the hover recognizer.
    type Kept = (Retained<Target>, Retained<UIScrollView>, Retained<UIHoverGestureRecognizer>);

    thread_local! {
        /// The scroll view's offset last seen, to hand on the difference.
        static LAST: Cell<CGPoint> = const { Cell::new(MIDDLE) };
        /// Between the start and the end of a scroll, its coasting included.
        static SCROLLING: Cell<bool> = const { Cell::new(false) };
        /// Where the pointer was last seen.
        static HOVER: Cell<CGPoint> = const { Cell::new(CGPoint { x: 0.0, y: 0.0 }) };
        /// Who to wake.
        static WAKE: RefCell<Option<Box<dyn Fn()>>> = const { RefCell::new(None) };
        /// The target, scroll view and recognizer, kept while the window lives (a recognizer
        /// holds its target, and a scroll view its delegate, weakly).
        static KEPT: RefCell<Option<Kept>> = const { RefCell::new(None) };
    }

    pub fn on_input(wake: impl Fn() + 'static) {
        WAKE.with(|w| *w.borrow_mut() = Some(Box::new(wake)));
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

        // The scroll view: the window's size, takes no touches (so every touch still reaches the
        // window's view), draws nothing, and only lends its scrolling.
        let sv = UIScrollView::initWithFrame(UIScrollView::alloc(mtm), view.bounds());
        sv.setUserInteractionEnabled(false);
        sv.setShowsVerticalScrollIndicator(false);
        sv.setShowsHorizontalScrollIndicator(false);
        sv.setScrollsToTop(false);
        sv.setBounces(false);
        sv.setDirectionalLockEnabled(true);
        sv.setContentSize(CGSize { width: ROOM, height: ROOM });
        sv.setContentOffset(MIDDLE);
        // SAFETY: the delegate is kept alive in KEPT for as long as the scroll view.
        unsafe { sv.setDelegate(Some(ProtocolObject::from_ref(&*target))) };
        let pan = sv.panGestureRecognizer();
        // Scrolling only: no finger on the screen starts it (touches keep scrolling as before).
        pan.setAllowedScrollTypesMask(UIScrollTypeMask::All);
        pan.setAllowedTouchTypes(&NSArray::<NSNumber>::new());
        pan.setCancelsTouchesInView(false);
        view.addSubview(&sv);
        // The pan goes onto the window's view, where the pointer is (the scroll view takes none).
        view.addGestureRecognizer(&pan);

        let target_object: &AnyObject = &target;
        // SAFETY: a recognizer just allocated, its target an object kept alive in KEPT and its
        // action a method the target's class defines above.
        let hover = unsafe {
            UIHoverGestureRecognizer::initWithTarget_action(UIHoverGestureRecognizer::alloc(mtm), Some(target_object), Some(sel!(hovered:)))
        };
        hover.setCancelsTouchesInView(false);
        view.addGestureRecognizer(&hover);
        KEPT.with(|k| *k.borrow_mut() = Some((target, sv, hover)));
    }
}
