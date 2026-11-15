// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The notification extension's own program (`apps/mailcraft-ios-notify`): iPadOS starts it when a
//! notification marked `mutable-content` arrives for MailCraft, and shows what it hands back. It
//! is a subclass of UNNotificationServiceExtension, `MailCraftNotificationService` (the class the
//! extension's Info.plist names), whose one method takes the sealed text from the payload (`m`),
//! asks the program to open it, and replaces the title, the subtitle and the body with what that
//! says. When anything is missing or doesn't open, the notification shows as it came ("New
//! mail"). [`run`] hands the process to iPadOS (`NSExtensionMain`) and doesn't return.

/// What a notification shows.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    pub title: String,
    pub subtitle: String,
    pub body: String,
}

/// Run the extension: `open` turns the sealed text of a notification into what it shows.
pub fn run(open: fn(&str) -> Option<Shown>) -> i32 {
    #[cfg(target_os = "ios")]
    return ios::run(open);
    #[cfg(not(target_os = "ios"))]
    {
        let _ = open;
        1
    }
}

#[cfg(target_os = "ios")]
mod ios {
    use std::ffi::{CString, c_char, c_int};
    use std::ptr::NonNull;
    use std::sync::OnceLock;

    use objc2::rc::Retained;
    use objc2::runtime::AnyObject;
    use objc2::{ClassType, define_class, msg_send};
    use objc2_foundation::{NSObject, NSObjectProtocol, NSString};
    use objc2_user_notifications::{
        UNMutableNotificationContent, UNNotificationContent, UNNotificationRequest, UNNotificationServiceExtension, UNNotificationSound,
    };

    use super::Shown;

    static OPEN: OnceLock<fn(&str) -> Option<Shown>> = OnceLock::new();

    unsafe extern "C" {
        /// Foundation's entry point for an app extension.
        fn NSExtensionMain(argc: c_int, argv: *mut *mut c_char) -> c_int;
    }

    /// The sealed text in `content`'s payload, when there is one.
    fn sealed(content: &UNNotificationContent) -> Option<String> {
        let info = content.userInfo();
        let key = NSString::from_str("m");
        // SAFETY: the key is an NSString, which is what the dictionary's keys are compared with.
        let value: Option<Retained<AnyObject>> = unsafe { msg_send![&*info, objectForKey: &*key] };
        Some(value?.downcast_ref::<NSString>()?.to_string())
    }

    define_class!(
        // SAFETY: UNNotificationServiceExtension is meant to be subclassed, with these methods
        // overridden; the class has no ivars to drop.
        #[unsafe(super(UNNotificationServiceExtension, NSObject))]
        #[name = "MailCraftNotificationService"]
        struct Service;

        // SAFETY: NSObjectProtocol has no extra requirements.
        unsafe impl NSObjectProtocol for Service {}

        // SAFETY: the method has UNNotificationServiceExtension's signature.
        impl Service {
            #[unsafe(method(didReceiveNotificationRequest:withContentHandler:))]
            fn did_receive(&self, request: &UNNotificationRequest, handler: &block2::DynBlock<dyn Fn(NonNull<UNNotificationContent>)>) {
                let came = request.content();
                let shown = sealed(&came).and_then(|text| OPEN.get().and_then(|open| open(&text)));
                let Some(shown) = shown else {
                    handler.call((NonNull::from(&*came),));
                    return;
                };
                let content = UNMutableNotificationContent::new();
                content.setTitle(&NSString::from_str(&shown.title));
                content.setSubtitle(&NSString::from_str(&shown.subtitle));
                content.setBody(&NSString::from_str(&shown.body));
                content.setThreadIdentifier(&came.threadIdentifier());
                content.setSound(Some(&UNNotificationSound::defaultSound()));
                let content: &UNNotificationContent = &content;
                handler.call((NonNull::from(content),));
            }

            // (The answer goes before the method returns: there is nothing left to hand in.)
            #[unsafe(method(serviceExtensionTimeWillExpire))]
            fn time_will_expire(&self) {}
        }
    );

    pub fn run(open: fn(&str) -> Option<Shown>) -> i32 {
        let _ = OPEN.set(open);
        // Registered before iPadOS looks for it by name.
        let _ = Service::class();
        let args: Vec<CString> = std::env::args().filter_map(|a| CString::new(a).ok()).collect();
        let mut argv: Vec<*mut c_char> = args.iter().map(|a| a.as_ptr().cast_mut()).collect();
        argv.push(std::ptr::null_mut());
        let argc = c_int::try_from(args.len()).unwrap_or(0);
        // SAFETY: argv holds `argc` pointers to C strings that live (in `args`) for as long as the
        // process does, and a null after them, as `main`'s do.
        unsafe { NSExtensionMain(argc, argv.as_mut_ptr()) }
    }
}
