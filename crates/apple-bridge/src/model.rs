// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Apple's on-device model (Foundation Models) on iPad, through the Swift functions in
//! `swift/MailCraftModel.swift`.

use std::time::Duration;

/// Whether the model is there to answer, and if not, why.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Availability {
    Available,
    /// This iPad can't (not Apple Intelligence capable, not turned on, not downloaded yet).
    NotOnThisDevice,
    /// This build has no Foundation Models (built without Xcode 26), or isn't for iPadOS.
    NotInThisBuild,
}

/// Whether the model can answer now.
pub fn availability() -> Availability {
    imp::availability()
}

/// Ask the model `prompt` under `instructions`, waiting at most `patience` for its answer.
pub fn respond(instructions: &str, prompt: &str, patience: Duration) -> Result<String, String> {
    imp::respond(instructions, prompt, patience)
}

#[cfg(any(not(target_os = "ios"), mailcraft_no_swift))]
mod imp {
    use super::Availability;

    pub fn availability() -> Availability {
        Availability::NotInThisBuild
    }
    pub fn respond(_: &str, _: &str, _: std::time::Duration) -> Result<String, String> {
        Err("Apple's on-device model isn't in this build".into())
    }
}

#[cfg(all(target_os = "ios", not(mailcraft_no_swift)))]
mod imp {
    use std::ffi::{CStr, CString, c_char, c_int, c_void};
    use std::sync::mpsc::{Sender, channel};
    use std::time::Duration;

    use super::Availability;

    type Done = extern "C" fn(*mut c_void, *const c_char, c_int);

    // SAFETY (for the block): the two functions are defined in swift/MailCraftModel.swift with
    // `@_cdecl`, with exactly these C signatures; the static library is linked by build.rs.
    unsafe extern "C" {
        fn mailcraft_model_available() -> c_int;
        fn mailcraft_model_respond(instructions: *const c_char, prompt: *const c_char, context: *mut c_void, done: Done);
    }

    pub fn availability() -> Availability {
        // SAFETY: takes nothing, returns an int; safe to call from any thread.
        match unsafe { mailcraft_model_available() } {
            1 => Availability::Available,
            0 => Availability::NotOnThisDevice,
            _ => Availability::NotInThisBuild,
        }
    }

    /// Called by Swift exactly once per request, with the context [`respond`] made.
    extern "C" fn done(context: *mut c_void, text: *const c_char, failed: c_int) {
        if context.is_null() {
            return;
        }
        // SAFETY: `context` is the Box<Sender> leaked by `respond` for this one call, and Swift
        // calls back exactly once, so it is reclaimed exactly once.
        let tx = unsafe { Box::from_raw(context.cast::<Sender<Result<String, String>>>()) };
        let said = if text.is_null() {
            String::new()
        } else {
            // SAFETY: Swift passes a NUL-terminated UTF-8 string valid for the duration of the call.
            unsafe { CStr::from_ptr(text) }.to_string_lossy().into_owned()
        };
        let _ = tx.send(if failed == 0 { Ok(said) } else { Err(said) });
    }

    pub fn respond(instructions: &str, prompt: &str, patience: Duration) -> Result<String, String> {
        let instructions = CString::new(instructions.replace('\0', " ")).map_err(|e| e.to_string())?;
        let prompt = CString::new(prompt.replace('\0', " ")).map_err(|e| e.to_string())?;
        let (tx, rx) = channel();
        let context = Box::into_raw(Box::new(tx)).cast::<c_void>();
        // SAFETY: both strings are NUL-terminated and outlive the call (Swift copies them before
        // returning); `context` is reclaimed by `done`, which Swift calls exactly once.
        unsafe { mailcraft_model_respond(instructions.as_ptr(), prompt.as_ptr(), context, done) };
        rx.recv_timeout(patience).map_err(|_| "Apple's model took too long to answer".to_string())?
    }
}
