// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! MailCraft's own application delegate, which the window library leaves out: it takes the push
//! address ([`crate::push`]), builds the menu bar ([`crate::menus`]) and receives what is chosen
//! there. A responder, so that it is the last link of the responder chain, where UIKit asks for
//! the menus and sends the menus' actions.

#![cfg(target_os = "ios")]

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
use objc2_foundation::{NSData, NSError, NSObject, NSObjectProtocol};
use objc2_ui_kit::{UIApplication, UIApplicationDelegate, UICommand, UIMenuBuilder, UIResponder};

define_class!(
    // SAFETY: UIResponder has no subclassing requirements for a class that overrides only the
    // methods below with their own signatures, and the class has no ivars to drop.
    #[unsafe(super(UIResponder, NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "MailCraftAppDelegate"]
    pub(crate) struct Delegate;

    // SAFETY: NSObjectProtocol has no extra requirements.
    unsafe impl NSObjectProtocol for Delegate {}

    // SAFETY: the two methods have UIApplicationDelegate's signatures; every other method of the
    // protocol is optional and left out.
    unsafe impl UIApplicationDelegate for Delegate {
        #[unsafe(method(application:didRegisterForRemoteNotificationsWithDeviceToken:))]
        fn did_register(&self, _app: &UIApplication, token: &NSData) {
            crate::push::keep(Ok(crate::push::hex(&token.to_vec())));
        }

        #[unsafe(method(application:didFailToRegisterForRemoteNotificationsWithError:))]
        fn did_fail(&self, _app: &UIApplication, error: &NSError) {
            crate::push::keep(Err(error.localizedDescription().to_string()));
        }
    }

    // SAFETY: the methods have UIResponder's signatures (`buildMenuWithBuilder:`,
    // `validateCommand:`), and the menus' own action takes the command that was chosen.
    impl Delegate {
        #[unsafe(method(buildMenuWithBuilder:))]
        fn build_menu(&self, builder: &ProtocolObject<dyn UIMenuBuilder>) {
            crate::menus::build(builder);
        }

        #[unsafe(method(validateCommand:))]
        fn validate_command(&self, command: &UICommand) {
            crate::menus::validate(command);
        }

        #[unsafe(method(mailcraftMenu:))]
        fn chosen(&self, command: &UICommand) {
            crate::menus::chosen(command);
        }
    }
);

impl Delegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: `init` of UIResponder, on an object just allocated.
        unsafe { msg_send![super(this), init] }
    }
}

thread_local! {
    /// Our delegate, kept for the rest of the run (UIKit holds a delegate weakly).
    static OURS: RefCell<Option<Retained<Delegate>>> = const { RefCell::new(None) };
}

/// Whether the app's delegate is ours: set it when the app has none. False when another delegate
/// is there (it is left alone) or off the main thread.
pub(crate) fn ensure() -> bool {
    let Some(mtm) = MainThreadMarker::new() else { return false };
    if OURS.with(|o| o.borrow().is_some()) {
        return true;
    }
    let app = UIApplication::sharedApplication(mtm);
    // SAFETY: only whether there is one is looked at.
    if unsafe { app.delegate() }.is_some() {
        return false;
    }
    let ours = Delegate::new(mtm);
    // SAFETY: the delegate is kept in OURS below for the rest of the run.
    unsafe { app.setDelegate(Some(ProtocolObject::from_ref(&*ours))) };
    OURS.with(|o| *o.borrow_mut() = Some(ours));
    true
}
