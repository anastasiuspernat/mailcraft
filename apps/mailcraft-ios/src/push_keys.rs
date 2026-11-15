// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The keys of push from the owner's relay, in the iPad's keychain (shared by MailCraft and its
//! notification extension, `apps/mailcraft-ios-notify`, which are signed with the same keychain
//! access group first): `link`, which the exchanges with the relay are sealed with, and `note`,
//! which a notification's text is. Readable after the iPad has been unlocked once since it
//! started (a notification that comes while it is locked is opened too), and never copied to
//! another device or a backup. Never in a file or a log.

use mailcraft_net::push_link::Key;
use security_framework::access_control::{ProtectionMode, SecAccessControl};
use security_framework::passwords::{PasswordOptions, delete_generic_password_options, generic_password, set_generic_password_options};

/// What the entries are filed under.
const SERVICE: &str = "MailCraft push";

fn options(name: &str) -> PasswordOptions {
    PasswordOptions::new_generic_password(SERVICE, name)
}

/// Keep `key` as `name`, in place of what was there.
pub fn put(name: &str, key: &Key) -> Result<(), String> {
    forget(name);
    let mut options = options(name);
    let access = SecAccessControl::create_with_protection(Some(ProtectionMode::AccessibleAfterFirstUnlockThisDeviceOnly), 0)
        .map_err(|_| "the iPad's keychain couldn't be used".to_string())?;
    options.set_access_control(access);
    set_generic_password_options(key, options).map_err(|_| "the iPad's keychain couldn't keep the key".to_string())
}

/// The key kept as `name`.
pub fn get(name: &str) -> Option<Key> {
    generic_password(options(name)).ok()?.try_into().ok()
}

/// Forget the key kept as `name`.
pub fn forget(name: &str) {
    let _ = delete_generic_password_options(options(name));
}
