// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The relay at work: a thread for each account that keeps its Inbox open and waits for news
//! (`mailcraft_net::watch`), and when unread mail has come, a notification to each paired iPad
//! that wants that account (`mailcraft_net::apns`). Accounts come and go while it runs (an iPad
//! brings its own, `pair`): the list is looked at again whenever it changes and every half
//! minute, and a watcher whose account went stops at its next wake. A connection that breaks is
//! made again, waiting longer each time it fails (30 seconds, doubling, at most 15 minutes; an
//! hour after the server refused the sign-in, so that a changed password doesn't get the account
//! locked); an account that signs in on its provider's page gets a fresh access token at each
//! connection and reconnects every 45 minutes, before the token runs out. Mail that came while
//! the relay was stopped is announced when it starts again, once: what has been looked at is kept
//! per account (`Seen`).
//!
//! A notification to an iPad that paired with a note key says "New mail" to Apple and carries the
//! account, the newest message's sender and its subject sealed for that iPad alone
//! (`push_link::seal_note`), which its notification extension opens; notifications of one account
//! replace one another on the iPad (`apns-collapse-id`, an id that names nothing).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mailcraft_net::apns::{Apns, Key, Notice, PRODUCTION, PushError, new_mail_payload, sealed_payload};
use mailcraft_net::push_link::{self, Note};

use crate::config::{self, Config, Device, Seen, Settings, Watched};
use crate::pair::States;
use crate::store;

/// The first wait after a failure, and the longest.
const FIRST_RETRY: Duration = Duration::from_secs(30);
const LONGEST_RETRY: Duration = Duration::from_secs(15 * 60);
/// The wait after the server refused the sign-in.
const REFUSED_RETRY: Duration = Duration::from_secs(60 * 60);
/// How long a connection of an account that signs in on its provider's page is kept.
const TOKEN_CONNECTION: Duration = Duration::from_secs(45 * 60);
/// How often the list of accounts is looked at without being told.
const LOOK_EVERY: Duration = Duration::from_secs(30);

/// "1 new message", "3 new messages".
pub fn new_mail_text(count: usize) -> String {
    if count == 1 { "1 new message".into() } else { format!("{count} new messages") }
}

/// The settings, the push sender and how each account is doing, shared by the threads.
pub struct Shared {
    pub settings: Settings,
    apns: Mutex<Apns>,
    pub states: States,
}

/// What a notification says of the new mail.
#[derive(Clone, Debug, Default)]
pub struct News {
    pub count: usize,
    pub from: String,
    pub subject: String,
}

impl Shared {
    /// For the settings at `path`, with their push key.
    pub fn open(path: PathBuf) -> Result<Shared, String> {
        let config = Config::load(&path)?;
        Ok(Shared { apns: Mutex::new(apns(&config)?), settings: Settings::new(path), states: States::default() })
    }

    pub(crate) fn update(&self, f: impl FnOnce(&mut Config)) {
        if let Err(e) = self.settings.update(f) {
            log::warn!("{e}");
        }
    }

    pub(crate) fn read(&self) -> Option<Config> {
        self.settings.read().map_err(|e| log::warn!("{e}")).ok()
    }

    fn set_state(&self, email: &str, state: &str) {
        if let Ok(mut s) = self.states.lock() {
            s.insert(email.to_ascii_lowercase(), state.to_string());
        }
    }

    /// Tell every paired iPad that wants `account` of `news`; an iPad that is gone is unpaired.
    /// Whether any was told.
    pub fn notify(&self, account: &Watched, news: &News) -> bool {
        let Some(config) = self.read() else { return false };
        let mut told = false;
        let mut gone = Vec::new();
        for device in config.devices.iter().filter(|d| d.wants(account)) {
            let note_key =
                (!device.id.is_empty()).then(|| store::get(&Device::note_key_name(&device.id))).flatten().and_then(|k| push_link::key_from(&k));
            let (payload, collapse) = match &note_key {
                Some(key) => {
                    let note = Note { account: account.email.clone(), count: news.count, from: news.from.clone(), subject: news.subject.clone() };
                    let thread = push_link::opaque_id(key, &account.email);
                    match push_link::seal_note(key, &note) {
                        Ok(sealed) => (sealed_payload(&sealed, &thread, "MailCraft", "New mail"), Some(thread)),
                        Err(e) => {
                            log::warn!("{} wasn't told: {e}", device.name);
                            continue;
                        }
                    }
                }
                // Paired by the first version: the account and the count, as then.
                None => (new_mail_payload(&account.email, news.count, &new_mail_text(news.count)), None),
            };
            let notice = Notice { collapse_id: collapse.as_deref() };
            let sent = match self.apns.lock() {
                Ok(mut apns) => apns.send(&device.token, crate::TOPIC, &payload, &notice),
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
    let kept = store::get(&account.secret_key()).ok_or("nothing to sign in with: add it again")?;
    let Some(oauth) = &oauth_account.oauth else { return Ok(kept) };
    let tokens = mailcraft_net::oauth::refresh(oauth, &kept).map_err(|e| format!("the sign-in is no good any more ({e}): sign in again"))?;
    if !tokens.refresh.is_empty() && tokens.refresh != kept {
        let _ = store::set(&account.secret_key(), &tokens.refresh);
    }
    Ok(tokens.access)
}

/// Whether an error says the server refused the sign-in itself.
fn refused(error: &str) -> bool {
    error.contains("sign-in failed") || error.contains("no good any more") || error.contains("nothing to sign in with")
}

/// The account `email` as the settings have it now, when it is still watched.
fn current(shared: &Shared, email: &str) -> Option<Watched> {
    shared.read()?.accounts.into_iter().find(|a| a.email.eq_ignore_ascii_case(email))
}

/// Sleep `wait`, in steps, unless `stop` is set meanwhile; whether to go on.
fn rest(wait: Duration, stop: &AtomicBool) -> bool {
    let started = Instant::now();
    while started.elapsed() < wait {
        if stop.load(Ordering::Relaxed) {
            return false;
        }
        std::thread::sleep(Duration::from_secs(1).min(wait));
    }
    !stop.load(Ordering::Relaxed)
}

/// Watch one account until it is stopped or no longer in the settings.
fn watch(shared: &Shared, email: &str, stop: &AtomicBool) {
    let key = email.to_ascii_lowercase();
    let mut retry = FIRST_RETRY;
    while !stop.load(Ordering::Relaxed) {
        let Some(account) = current(shared, email) else { break };
        let net_account = account.account();
        let opened = secret(&account, &net_account).and_then(|s| mailcraft_net::watch::open(&net_account, &s).map_err(|e| e.to_string()));
        let mut inbox = match opened {
            Ok(inbox) => inbox,
            Err(e) => {
                let wait = if refused(&e) { REFUSED_RETRY } else { retry };
                shared.set_state(email, &e);
                log::warn!("{email}: {e}; trying again in {} min", wait.as_secs().div_ceil(60));
                if !rest(wait, stop) {
                    break;
                }
                retry = (retry * 2).min(LONGEST_RETRY);
                continue;
            }
        };
        retry = FIRST_RETRY;
        shared.set_state(email, "");
        log::info!("{email}: watching the Inbox{}", if inbox.idles() { "" } else { " (the server can't IDLE: looking every minute)" });
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
                    if let Some(newest) = unread.iter().max().copied() {
                        log::info!("{email}: {} new", unread.len());
                        let (from_who, subject) = inbox.about(newest).unwrap_or_default();
                        let news = News { count: unread.len(), from: from_who, subject };
                        // (Read again: an iPad may have stopped wanting it meanwhile.)
                        if let Some(account) = current(shared, email) {
                            shared.notify(&account, &news);
                        }
                    }
                    if next != from || known.is_none() {
                        from = next;
                        shared.update(|c| {
                            if c.accounts.iter().any(|a| a.email.eq_ignore_ascii_case(email)) {
                                c.seen.insert(key.clone(), Seen { uid_validity: validity, next });
                            }
                        });
                    }
                }
                Err(e) => break e.to_string(),
            }
            if stop.load(Ordering::Relaxed) {
                break "no longer watched".into();
            }
            if account.sign_in.is_some() && since.elapsed() > TOKEN_CONNECTION {
                break "time for a fresh sign-in token".into();
            }
            if let Err(e) = inbox.wait() {
                break e.to_string();
            }
        };
        inbox.close();
        if stop.load(Ordering::Relaxed) {
            break;
        }
        log::info!("{email}: connecting again ({ended})");
    }
    if let Ok(mut s) = shared.states.lock() {
        s.remove(&key);
    }
    log::info!("{email}: no longer watched");
}

/// A watcher thread and how to stop it.
struct Running {
    stop: Arc<AtomicBool>,
    thread: JoinHandle<()>,
}

/// Start a watcher for each account in the settings that has none, and stop those whose account
/// went.
fn reconcile(shared: &Arc<Shared>, running: &mut HashMap<String, Running>) {
    let Some(config) = shared.read() else { return };
    let wanted: Vec<String> = config.accounts.iter().map(|a| a.email.to_ascii_lowercase()).collect();
    running.retain(|email, r| {
        let keep = wanted.contains(email) && !r.thread.is_finished();
        if !keep {
            r.stop.store(true, Ordering::Relaxed);
        }
        keep
    });
    for email in wanted {
        if running.contains_key(&email) {
            continue;
        }
        let stop = Arc::new(AtomicBool::new(false));
        let (s, flag, who) = (shared.clone(), stop.clone(), email.clone());
        let thread = std::thread::spawn(move || watch(&s, &who, &flag));
        running.insert(email, Running { stop, thread });
    }
}

/// Run the relay for the settings in `folder`, until the process is stopped.
pub fn run(folder: &Path) -> Result<(), String> {
    let _one = std::net::TcpListener::bind(("127.0.0.1", crate::ONE_ONLY_PORT)).map_err(|_| "the relay is running already".to_string())?;
    let path = config::file_in(folder);
    let config = Config::load(&path)?;
    let shared = Arc::new(Shared::open(path)?);
    if config.devices.is_empty() {
        log::warn!("no iPad is paired yet: `mailcraft-relay code` shows what to enter on it");
    }
    log::info!("the relay is running for {} account(s)", config.accounts.len());
    let (changed, told): (Sender<()>, Receiver<()>) = channel();
    // The iPads, on the home network, for as long as the relay runs.
    match std::net::TcpListener::bind(("0.0.0.0", crate::PAIR_PORT)) {
        Ok(listener) => {
            let shared = shared.clone();
            let changed = Mutex::new(changed);
            std::thread::spawn(move || {
                let tell = || {
                    if let Ok(c) = changed.lock() {
                        let _ = c.send(());
                    }
                };
                crate::pair::serve_forever(&listener, &shared.settings, &shared.states, &tell)
            });
        }
        Err(e) => log::warn!("iPads aren't listened for (port {}: {e})", crate::PAIR_PORT),
    }
    let mut running: HashMap<String, Running> = HashMap::new();
    loop {
        reconcile(&shared, &mut running);
        // Until told of a change, or a while passes.
        if let Err(std::sync::mpsc::RecvTimeoutError::Disconnected) = told.recv_timeout(LOOK_EVERY) {
            std::thread::sleep(LOOK_EVERY);
        }
    }
}
