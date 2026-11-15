// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Notifications shown while MailCraft is open: iPadOS keeps an app's own notifications (new
//! mail from the relay, a reminder) out of sight while that app is in front unless the app's
//! notification delegate says to show them. [`show_while_open`] makes one that says: as a
//! banner, in the list, with its sound.

/// Show MailCraft's notifications while it is in front too. Once; on the main thread.
pub fn show_while_open() {
    #[cfg(target_os = "ios")]
    ios::install();
}

#[cfg(target_os = "ios")]
mod ios {
    use std::cell::RefCell;

    use objc2::rc::Retained;
    use objc2::runtime::ProtocolObject;
    use objc2::{MainThreadMarker, MainThreadOnly, define_class, msg_send};
    use objc2_foundation::{NSObject, NSObjectProtocol};
    use objc2_user_notifications::{UNNotification, UNNotificationPresentationOptions, UNUserNotificationCenter, UNUserNotificationCenterDelegate};

    define_class!(
        // SAFETY: NSObject has no subclassing requirements, and the class has no ivars to drop.
        #[unsafe(super(NSObject))]
        #[thread_kind = MainThreadOnly]
        #[name = "MailCraftNotificationDelegate"]
        struct Shower;

        // SAFETY: NSObjectProtocol has no extra requirements.
        unsafe impl NSObjectProtocol for Shower {}

        // SAFETY: the method has UNUserNotificationCenterDelegate's signature; the protocol's
        // other methods are optional and left out.
        unsafe impl UNUserNotificationCenterDelegate for Shower {
            #[unsafe(method(userNotificationCenter:willPresentNotification:withCompletionHandler:))]
            fn will_present(
                &self,
                _center: &UNUserNotificationCenter,
                _notification: &UNNotification,
                handler: &block2::DynBlock<dyn Fn(UNNotificationPresentationOptions)>,
            ) {
                handler.call((UNNotificationPresentationOptions::Banner
                    | UNNotificationPresentationOptions::List
                    | UNNotificationPresentationOptions::Sound,));
            }
        }
    );

    impl Shower {
        fn new(mtm: MainThreadMarker) -> Retained<Self> {
            let this = Self::alloc(mtm).set_ivars(());
            // SAFETY: `init` of NSObject, on an object just allocated.
            unsafe { msg_send![super(this), init] }
        }
    }

    thread_local! {
        /// The delegate: the notification center holds it weakly, so it is kept here.
        static KEPT: RefCell<Option<Retained<Shower>>> = const { RefCell::new(None) };
    }

    pub fn install() {
        let Some(mtm) = MainThreadMarker::new() else { return };
        KEPT.with(|kept| {
            let mut kept = kept.borrow_mut();
            if kept.is_some() {
                return;
            }
            let shower = Shower::new(mtm);
            UNUserNotificationCenter::currentNotificationCenter().setDelegate(Some(ProtocolObject::from_ref(&*shower)));
            *kept = Some(shower);
        });
    }
}
