// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The system's credential store (Windows Credential Manager, the macOS keychain, the Secret
//! Service on Linux), for the relay's passwords, sign-ins and push key: filed under the service
//! name `MailCraft Relay`, apart from desktop MailCraft's. Windows keeps at most 2560 bytes in a
//! credential and a Microsoft refresh token can be longer, so a secret is kept in pieces of 1000
//! characters: `<key>#n` holds how many, `<key>#0`… hold them, as desktop MailCraft does.

/// What the entries are filed under.
const SERVICE: &str = "MailCraft Relay";
/// The most characters in one piece.
const PIECE: usize = 1000;
/// More pieces than any secret needs.
const MAX_PIECES: usize = 16;

/// The credential store's name for the push key.
pub const APPLE_KEY: &str = "apple-push-key";

#[cfg(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd"))]
fn entry(key: &str, part: &str) -> Result<keyring_core::Entry, String> {
    use std::sync::Arc;
    let closed = |_| "the system's credential store can't be opened".to_string();
    #[cfg(target_os = "windows")]
    let store: Arc<keyring_core::CredentialStore> = windows_native_keyring_store::Store::new().map_err(closed)?;
    #[cfg(target_os = "macos")]
    let store: Arc<keyring_core::CredentialStore> = apple_native_keyring_store::keychain::Store::new().map_err(closed)?;
    #[cfg(any(target_os = "linux", target_os = "freebsd"))]
    let store: Arc<keyring_core::CredentialStore> = zbus_secret_service_keyring_store::Store::new().map_err(closed)?;
    store.build(SERVICE, &format!("{key}#{part}"), None).map_err(|_| "the system's credential store can't be used".to_string())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd")))]
fn entry(_key: &str, _part: &str) -> Result<NoEntry, String> {
    Err("this system has no credential store the relay can use".into())
}

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd")))]
struct NoEntry;

#[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux", target_os = "freebsd")))]
impl NoEntry {
    fn get_password(&self) -> Result<String, ()> {
        Err(())
    }
    fn set_password(&self, _: &str) -> Result<(), ()> {
        Err(())
    }
    fn delete_credential(&self) -> Result<(), ()> {
        Err(())
    }
}

/// The secret kept under `key`.
pub fn get(key: &str) -> Option<String> {
    let count: usize = entry(key, "n").ok()?.get_password().ok()?.parse().ok().filter(|n| (1..=MAX_PIECES).contains(n))?;
    let mut secret = String::new();
    for i in 0..count {
        secret.push_str(&entry(key, &i.to_string()).ok()?.get_password().ok()?);
    }
    Some(secret)
}

/// Keep `secret` under `key`, in place of what was there.
pub fn set(key: &str, secret: &str) -> Result<(), String> {
    let chars: Vec<char> = secret.chars().collect();
    let pieces: Vec<String> = chars.chunks(PIECE).map(|c| c.iter().collect()).collect();
    if pieces.is_empty() || pieces.len() > MAX_PIECES {
        return Err("that is no secret the credential store can keep".into());
    }
    delete(key);
    // (The store's own words are never passed on: they may repeat what they were given.)
    let refused = || "the system's credential store couldn't keep it".to_string();
    for (i, piece) in pieces.iter().enumerate() {
        if entry(key, &i.to_string())?.set_password(piece).is_err() {
            delete(key);
            return Err(refused());
        }
    }
    if entry(key, "n")?.set_password(&pieces.len().to_string()).is_err() {
        delete(key);
        return Err(refused());
    }
    Ok(())
}

/// Forget what is kept under `key`.
pub fn delete(key: &str) {
    for i in 0..MAX_PIECES {
        if let Ok(e) = entry(key, &i.to_string()) {
            let _ = e.delete_credential();
        }
    }
    if let Ok(e) = entry(key, "n") {
        let _ = e.delete_credential();
    }
}
