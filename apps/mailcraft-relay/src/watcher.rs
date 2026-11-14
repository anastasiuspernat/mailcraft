// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The relay at work: a thread for each account that keeps its Inbox open and waits for news
//! (`mailcraft_net::watch`), and when unread mail has come, a notification to every paired iPad
//! (`mailcraft_net::apns`). A connection that breaks is made again, waiting longer each time it
//! fails (30 seconds, doubling, at most 15 minutes); an account that signs in with Microsoft
//! gets a fresh access token at each connection and reconnects every 45 minutes, before the
//! token runs out. Mail that came while the relay was stopped is announced when it starts again,
//! once: what has been looked at is kept per account (`Seen`).

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mailcraft_net::apns::{Apns, Key, PRODUCTION, PushError, new_mail_payload};

use crate::config::{self, Config, Seen, Watched};
use crate::store;

/// The first wait after a failure, and the longest.
const FIRST_RETRY: Duration = Duration::from_secs(30);
const LONGEST_RETRY: Duration = Duration::from_secs(15 * 60);
/// How long a connection of an account that signs in on its provider's page is kept.
const TOKEN_CONNECTION: Duration = Duration::from_secs(45 * 60);

/// "1 new message", "3 new messages".
pub fn new_mail_text(count: usize) -> String {
    if count == 1 { "1 new message".into() } else { format!("{count} new messages") }
}

/// The settings, read and written by one thread at a time.
pub struct Shared {
    path: PathBuf,
    lock: Mutex<()>,
    apns: Mutex<Apns>,
}

impl Shared {
    /// For the settings at `path`, with their push key.
    pub fn open(path: PathBuf) -> Result<Shared, String> {
        let config = Config::load(&path)?;
        Ok(Shared { apns: Mutex::new(apns(&config)?), path, lock: Mutex::new(()) })
    }

    /// Change the settings with `f`, read fresh from the file (the pairing may have changed it).
    fn update(&self, f: impl FnOnce(&mut Config)) {
        let Ok(_held) = self.lock.lock() else { return };
        match Config::load(&self.path) {
            Ok(mut config) => {
                f(&mut config);
                if let Err(e) = config.save(&self.path) {
                    log::warn!("{e}");
                }
            }
            Err(e) => log::warn!("{e}"),
        }
    }

    fn read(&self) -> Option<Config> {
        let _held = self.lock.lock().ok()?;
        Config::load(&self.path).map_err(|e| log::warn!("{e}")).ok()
    }

    /// Tell every paired iPad that `email` has `count` new messages; an iPad that is gone is
    /// unpaired. Whether any was told.
    pub fn notify(&self, email: &str, count: usize) -> bool {
        let Some(config) = self.read() else { return false };
        let payload = new_mail_payload(email, count, &new_mail_text(count));
        let mut told = false;
        let mut gone = Vec::new();
        for device in &config.devices {
            let sent = match self.apns.lock() {
                Ok(mut apns) => apns.send(&device.token, crate::TOPIC, &payload),
                Err(_) => Err(PushError::Failed("the push key is unusable".into())),
            };
            match sent {
                Ok(()) => told = true,
                Err(PushError::Gone(why)) => {
                    log::warn!("{} is no longer there ({why}): unpaired", device.name);
                    gone.push(device.token.clone());
                }
                Err(PushError::Failed(why)) => log::warn!("{} wasn't told: {why}", device.name),
            }
        }
        if !gone.is_empty() {
            self.update(|c| c.devices.retain(|d| !gone.contains(&d.token)));
        }
        told
    }
}

/// The push sender from the key in the credential store and its ids in the settings.
pub fn apns(config: &Config) -> Result<Apns, String> {
    let ids = config.apple.as_ref().ok_or("no push key yet: run `mailcraft-relay apple-key` first")?;
    let text = store::get(store::APPLE_KEY).ok_or("the push key isn't in the credential store: run `mailcraft-relay apple-key` again")?;
    Ok(Apns::new(Key::from_p8(&text, &ids.key_id, &ids.team_id)?, PRODUCTION))
}

/// What the account signs in with now: its password, or a fresh access token from its sign-in.
fn secret(account: &Watched, oauth_account: &mailcraft_model::Account) -> Result<String, String> {
    let kept = store::get(&account.secret_key()).ok_or("nothing to sign in with: run `mailcraft-relay add` for it again")?;
    let Some(oauth) = &oauth_account.oauth else { return Ok(kept) };
    let tokens = mailcraft_net::oauth::refresh(oauth, &kept)
        .map_err(|e| format!("the Microsoft sign-in is no good any more ({e}): run `mailcraft-relay add` for it again"))?;
    if !tokens.refresh.is_empty() && tokens.refresh != kept {
        let _ = store::set(&account.secret_key(), &tokens.refresh);
    }
    Ok(tokens.access)
}

/// Watch one account for ever.
fn watch(shared: &Shared, account: Watched) {
    let key = account.email.to_ascii_lowercase();
    let mut retry = FIRST_RETRY;
    loop {
        let net_account = account.account();
        let opened = secret(&account, &net_account).and_then(|s| mailcraft_net::watch::open(&net_account, &s).map_err(|e| e.to_string()));
        let mut inbox = match opened {
            Ok(inbox) => inbox,
            Err(e) => {
                log::warn!("{}: {e}; trying again in {} s", account.email, retry.as_secs());
                std::thread::sleep(retry);
                retry = (retry * 2).min(LONGEST_RETRY);
                continue;
            }
        };
        retry = FIRST_RETRY;
        log::info!("{}: watching the Inbox{}", account.email, if inbox.idles() { "" } else { " (the server can't IDLE: looking every minute)" });
        let since = Instant::now();
        let known = shared.read().and_then(|c| c.seen.get(&key).copied());
        // A new Inbox (or one whose UIDs were renumbered) starts from now: nothing old is announced.
        let mut from = match known {
            Some(seen) if seen.uid_validity == inbox.uid_validity() => seen.next,
            _ => inbox.uid_next(),
        };
        let validity = inbox.uid_validity();
        let ended = loop {
            match inbox.unread_since(from) {
                Ok((unread, next)) => {
                    if !unread.is_empty() {
                        log::info!("{}: {} new", account.email, unread.len());
                        shared.notify(&account.email, unread.len());
                    }
                    if next != from || known.is_none() {
                        from = next;
                        shared.update(|c| {
                            c.seen.insert(key.clone(), Seen { uid_validity: validity, next });
                        });
                    }
                }
                Err(e) => break e.to_string(),
            }
            if account.sign_in.is_some() && since.elapsed() > TOKEN_CONNECTION {
                break "time for a fresh sign-in token".into();
            }
            if let Err(e) = inbox.wait() {
                break e.to_string();
            }
        };
        log::info!("{}: connecting again ({ended})", account.email);
        inbox.close();
    }
}

/// Run the relay for every account in the settings in `folder`, until the process is stopped.
pub fn run(folder: &Path) -> Result<(), String> {
    let _one = std::net::TcpListener::bind(("127.0.0.1", crate::ONE_ONLY_PORT)).map_err(|_| "the relay is running already".to_string())?;
    let path = config::file_in(folder);
    let config = Config::load(&path)?;
    if config.accounts.is_empty() {
        return Err("no accounts to watch: add one with `mailcraft-relay add you@example.com`".into());
    }
    let shared = Arc::new(Shared::open(path)?);
    if config.devices.is_empty() {
        log::warn!("no iPad is paired yet: `mailcraft-relay pair`");
    }
    log::info!("the relay is running for {} account(s)", config.accounts.len());
    let threads: Vec<_> = config
        .accounts
        .into_iter()
        .map(|account| {
            let shared = shared.clone();
            std::thread::spawn(move || watch(&shared, account))
        })
        .collect();
    for t in threads {
        let _ = t.join();
    }
    Ok(())
}
