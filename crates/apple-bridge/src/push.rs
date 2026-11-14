// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Push notifications from the owner's own relay (`apps/mailcraft-relay`): [`register`] asks
//! iPadOS for this iPad's push address (its device token), which Apple hands to the
//! application's delegate. The window library owns that delegate and has no say in this, so the
//! two methods Apple calls are added to its class while the app runs; they keep what they are
//! given for [`token`] and [`failure`]. The notifications themselves are shown by iPadOS and need
//! nothing of the app. Nothing elsewhere than iPadOS.

use std::sync::Mutex;

/// What Apple said: the device token (hexadecimal), or why there is none.
static TOKEN: Mutex<Option<Result<String, String>>> = Mutex::new(None);

fn keep(said: Result<String, String>) {
    if let Ok(mut t) = TOKEN.lock() {
        *t = Some(said);
    }
}

/// Ask for this iPad's push address, once a run.
pub fn register() {
    imp::register();
}

/// This iPad's push address, once Apple has given it.
pub fn token() -> Option<String> {
    TOKEN.lock().ok()?.clone()?.ok()
}

/// Why Apple gave none, when it said.
pub fn failure() -> Option<String> {
    TOKEN.lock().ok()?.clone()?.err()
}

/// Bytes as lower-case hexadecimal.
#[cfg_attr(not(target_os = "ios"), allow(dead_code))]
pub(crate) fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(not(target_os = "ios"))]
mod imp {
    pub fn register() {
        super::keep(Err("push notifications are for iPad".into()));
    }
}

#[cfg(target_os = "ios")]
mod imp {
    use std::sync::Once;

    use objc2::runtime::{AnyClass, AnyObject, Imp, Sel};
    use objc2::{MainThreadMarker, sel};
    use objc2_foundation::{NSData, NSError};
    use objc2_ui_kit::UIApplication;

    /// `application:didRegisterForRemoteNotificationsWithDeviceToken:`.
    extern "C-unwind" fn registered(_this: &AnyObject, _cmd: Sel, _app: &AnyObject, token: Option<&NSData>) {
        match token {
            Some(token) => super::keep(Ok(super::hex(&token.to_vec()))),
            None => super::keep(Err("Apple gave no push address".into())),
        }
    }

    /// `application:didFailToRegisterForRemoteNotificationsWithError:`.
    extern "C-unwind" fn failed(_this: &AnyObject, _cmd: Sel, _app: &AnyObject, error: Option<&NSError>) {
        let why = error.map(|e| e.localizedDescription().to_string()).unwrap_or_else(|| "iPadOS gave no push address".into());
        super::keep(Err(why));
    }

    /// Add `imp` as `sel` to `class`, with the type encoding of two object arguments and no result.
    ///
    /// # Safety
    /// `imp` must be an `extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, Option<&T>)` for an
    /// Objective-C object type `T`, which is what that encoding says.
    unsafe fn add(class: &AnyClass, sel: Sel, imp: Imp) {
        let class = class as *const AnyClass as *mut AnyClass;
        // SAFETY: the class is a live class from the runtime; adding a method that it doesn't have
        // is what the runtime offers this for (a class that has one keeps its own: the call says
        // no and changes nothing); the encoding `v@:@@` matches `imp`, as the caller promises.
        unsafe { objc2::ffi::class_addMethod(class, sel, imp, c"v@:@@".as_ptr()) };
    }

    pub fn register() {
        static ONCE: Once = Once::new();
        let Some(mtm) = MainThreadMarker::new() else { return };
        ONCE.call_once(|| {
            let app = UIApplication::sharedApplication(mtm);
            // SAFETY: the delegate is the window library's, alive for as long as the app is; it is
            // used here only to find its class.
            let Some(delegate) = (unsafe { app.delegate() }) else {
                super::keep(Err("the app has no delegate to give the push address to".into()));
                return;
            };
            let object: &AnyObject = delegate.as_ref();
            let class = object.class();
            type Two<T> = extern "C-unwind" fn(&AnyObject, Sel, &AnyObject, Option<&T>);
            // SAFETY: both functions have the shape `add` asks for (NSData and NSError arguments),
            // and a function pointer of that shape is called by the runtime as an `Imp`.
            unsafe {
                add(class, sel!(application:didRegisterForRemoteNotificationsWithDeviceToken:), std::mem::transmute::<Two<NSData>, Imp>(registered));
                add(class, sel!(application:didFailToRegisterForRemoteNotificationsWithError:), std::mem::transmute::<Two<NSError>, Imp>(failed));
            }
            app.registerForRemoteNotifications();
        });
    }
}
