// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Apple's sign-in sheet (ASWebAuthenticationSession) on iPad.
//!
//! [`open`] shows the provider's page in a sheet over MailCraft; the page sends the sign-in back
//! to MailCraft's loopback address (which keeps listening, MailCraft being in front) and
//! [`close`] takes the sheet away once it has come. When the owner closes the sheet themselves,
//! the `closed` callback says so, and the wait can be called off. Both may be called from any
//! thread: the sheet itself lives on the main thread.

/// Show `url` in the sign-in sheet. Whether it could be shown (on iPadOS, a page the sheet
/// takes); `closed` runs when the owner closes the sheet without the page having finished.
pub fn open(url: &str, closed: impl Fn() + Send + 'static) -> bool {
    imp::open(url, Box::new(closed))
}

/// Take the sheet away, if one is showing.
pub fn close() {
    imp::close();
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn open(_url: &str, _closed: Box<dyn Fn() + Send>) -> bool {
        false
    }
    pub fn close() {}
}

#[cfg(target_os = "ios")]
mod imp {
    use std::cell::RefCell;

    use block2::RcBlock;
    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{AllocAnyThread, MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_authentication_services::{ASWebAuthenticationPresentationContextProviding, ASWebAuthenticationSession};
    use objc2_foundation::{NSError, NSObject, NSObjectProtocol, NSOperationQueue, NSString, NSURL};
    use objc2_ui_kit::{UIApplication, UIWindow};

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class has no ivars to drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MailCraftSignInAnchor"]
        struct Anchor;

        // SAFETY: NSObjectProtocol has no extra requirements.
        unsafe impl NSObjectProtocol for Anchor {}

        // SAFETY: the method's signature is the protocol's: it takes the session and returns a
        // retained window (ASPresentationAnchor is UIWindow on iOS; the bindings declare the
        // method for macOS only, so it is given here, returning the window as an object).
        unsafe impl ASWebAuthenticationPresentationContextProviding for Anchor {
            #[unsafe(method_id(presentationAnchorForWebAuthenticationSession:))]
            fn presentation_anchor(&self, _session: &ASWebAuthenticationSession) -> Retained<NSObject> {
                let window = key_window(self.mtm());
                Retained::into_super(Retained::into_super(Retained::into_super(window)))
            }
        }
    );

    impl Anchor {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: `init` of NSObject, on an object just allocated.
            unsafe { msg_send![super(this), init] }
        }
    }

    /// The window the sheet is shown over: the app's key window, else its first, else a new one.
    fn key_window(mtm: MainThreadMarker) -> Retained<UIWindow> {
        let app = UIApplication::sharedApplication(mtm);
        #[allow(deprecated)]
        let key = app.keyWindow();
        #[allow(deprecated)]
        let first = || app.windows().firstObject();
        key.or_else(first).unwrap_or_else(|| UIWindow::new(mtm))
    }

    thread_local! {
        /// The sheet showing, and what it is shown over (the session holds that weakly).
        static SHOWING: RefCell<Option<(Retained<ASWebAuthenticationSession>, Retained<Anchor>)>> = const { RefCell::new(None) };
    }

    pub fn open(url: &str, closed: Box<dyn Fn() + Send>) -> bool {
        let url = url.to_string();
        let (tx, rx) = std::sync::mpsc::channel();
        let on_main = move || {
            let shown = MainThreadMarker::new().is_some_and(|mtm| show(mtm, &url, closed));
            let _ = tx.send(shown);
        };
        on_main_thread(on_main);
        rx.recv_timeout(std::time::Duration::from_secs(10)).unwrap_or(false)
    }

    fn show(mtm: MainThreadMarker, url: &str, closed: Box<dyn Fn() + Send>) -> bool {
        let Some(nsurl) = NSURL::URLWithString(&NSString::from_str(url)) else { return false };
        // Called when the sheet goes: by the owner (an error, "cancelled") or by [`close`].
        let done = RcBlock::new(move |_url: *mut NSURL, error: *mut NSError| {
            if !error.is_null() {
                closed();
            }
            SHOWING.with(|s| s.borrow_mut().take());
        });
        // The scheme is never followed: the page sends the sign-in to the loopback address.
        let scheme = NSString::from_str("mailcraft-sign-in");
        // SAFETY: an ASWebAuthenticationSession just allocated, initialised with a valid URL, a
        // scheme and a completion block that lives as long as the session holds it. (The newer
        // `initWithURL:callback:` is iPadOS 17.4 and later; this one works from 12.)
        #[allow(deprecated)]
        let session = unsafe {
            ASWebAuthenticationSession::initWithURL_callbackURLScheme_completionHandler(
                ASWebAuthenticationSession::alloc(),
                &nsurl,
                Some(&scheme),
                RcBlock::as_ptr(&done),
            )
        };
        let anchor = Anchor::new(mtm);
        // SAFETY: the anchor is kept alive beside the session (in SHOWING) for as long as it shows.
        unsafe { session.setPresentationContextProvider(Some(ProtocolObject::from_ref(&*anchor))) };
        // SAFETY: plain property set and start on the main thread, as the API asks.
        let started = unsafe {
            session.setPrefersEphemeralWebBrowserSession(false);
            session.start()
        };
        if started {
            SHOWING.with(|s| *s.borrow_mut() = Some((session, anchor)));
        }
        started
    }

    pub fn close() {
        let on_main = || {
            if let Some((session, _anchor)) = SHOWING.with(|s| s.borrow_mut().take()) {
                // SAFETY: cancel on the main thread, where the session was started.
                unsafe { session.cancel() };
            }
        };
        on_main_thread(on_main);
    }

    /// Run `work` on the main thread: now when this is it, else on Foundation's main operation
    /// queue (UIKit's objects live there).
    fn on_main_thread(work: impl FnOnce() + Send + 'static) {
        if MainThreadMarker::new().is_some() {
            work();
            return;
        }
        let once = std::sync::Mutex::new(Some(work));
        let block = RcBlock::new(move || {
            if let Some(work) = once.lock().ok().and_then(|mut w| w.take()) {
                work();
            }
        });
        // SAFETY: the block is `Send`-safe work (its state is behind a mutex) that the queue copies
        // and runs once on the main thread; it captures nothing tied to this thread.
        unsafe { NSOperationQueue::mainQueue().addOperationWithBlock(&block) };
    }
}
