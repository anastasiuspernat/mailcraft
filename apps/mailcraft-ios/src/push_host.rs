// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Push from the owner's own relay (`apps/mailcraft-relay`, `mailcraft_ui_egui::push_options`),
//! over `mailcraft_net::push_link` on the home network (the relay's port 7993).
//!
//! Pairing proves the six-digit code the relay shows without sending it, and leaves a link key
//! here (in the keychain, `push_keys`), with which every later exchange is sealed: this iPad's
//! push address (sent at every start, so that a new one Apple gives reaches the relay) and its
//! accounts with what the relay signs in to them with, whenever they change. Away from home, they
//! wait, and are tried again every few minutes while MailCraft runs. Pairing also hands the relay
//! the note key this iPad made, with which the relay seals each notification's text for the
//! notification extension to open.
//!
//! `push.json` beside the mailbox keeps this device's id (random), the computer's address and its
//! name: nothing secret.

use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mailcraft_net::push_link::{self, LinkError, LinkedAccount, PairRequest, Request};
use mailcraft_ui_egui::push_options::{PushAccount, PushHost, PushState};
use serde::{Deserialize, Serialize};

use crate::push_keys;

/// How long connecting, and then each read or write, may take.
const PATIENCE: Duration = Duration::from_secs(8);
/// How long a refused first connection is tried again while iPadOS asks about the local network.
const PERMISSION_WAIT: Duration = Duration::from_secs(60);
/// How often accounts that wait are tried again.
const RETRY_EVERY: Duration = Duration::from_secs(5 * 60);
/// How long a sync waits for the push address from Apple.
const ADDRESS_WAIT: Duration = Duration::from_secs(60);

/// What `push.json` keeps.
#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Kept {
    #[serde(default)]
    device: String,
    #[serde(default)]
    computer: String,
    #[serde(default)]
    paired_with: String,
}

#[derive(Default)]
struct Going {
    kept: Kept,
    pairing: bool,
    problem: Option<String>,
    /// Accounts to send, until they have gone.
    waiting: Option<Vec<PushAccount>>,
    /// A thread is sending them.
    sending: bool,
    relay_accounts: Vec<(String, String)>,
    heard: Option<i64>,
}

pub struct Push {
    file: Option<PathBuf>,
    going: Arc<Mutex<Going>>,
    ctx: egui::Context,
}

fn read_kept(file: Option<&PathBuf>) -> Kept {
    file.and_then(|f| std::fs::read(f).ok()).and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default()
}

fn write_kept(file: Option<&PathBuf>, kept: &Kept) {
    let Some(file) = file else { return };
    if let Ok(text) = serde_json::to_vec(kept)
        && let Err(e) = mailcraft_engine::store::write_atomic(file, &text)
    {
        log::warn!("push: what it is paired with couldn't be kept: {e}");
    }
}

fn now() -> i64 {
    mailcraft_time::now()
}

impl Push {
    /// Asks Apple for this iPad's push address; `file` keeps what it is paired with.
    pub fn new(ctx: egui::Context, file: Option<PathBuf>) -> Push {
        mailcraft_apple_bridge::push::register();
        let mut kept = read_kept(file.as_ref());
        // Paired by the first version (no link key): pair again.
        if !kept.paired_with.is_empty() && push_keys::get("link").is_none() {
            kept.paired_with.clear();
        }
        Push { file, going: Arc::new(Mutex::new(Going { kept, ..Default::default() })), ctx }
    }
}

/// A connection to the relay at `computer`.
fn connect(computer: &str) -> Result<TcpStream, String> {
    let address = (computer, push_link::PORT)
        .to_socket_addrs()
        .map_err(|_| "that computer's name couldn't be found: type its address, as the relay shows it".to_string())?
        .next()
        .ok_or("that computer's name couldn't be found")?;
    // The first connection to the home network is refused while iPadOS asks the owner whether
    // MailCraft may use it: tried again until they have had time to answer.
    let started = Instant::now();
    let stream = loop {
        match TcpStream::connect_timeout(&address, PATIENCE) {
            Ok(stream) => break stream,
            Err(_) if started.elapsed() < PERMISSION_WAIT => std::thread::sleep(Duration::from_secs(2)),
            Err(_) => return Err("the relay didn't answer: is it running, is this iPad on the same network, and is Local Network on for MailCraft in Settings › Privacy & Security?".to_string()),
        }
    };
    let _ = stream.set_read_timeout(Some(PATIENCE * 4));
    let _ = stream.set_write_timeout(Some(PATIENCE * 4));
    Ok(stream)
}

/// Wait a while for this iPad's push address from Apple.
fn address() -> Option<String> {
    let started = Instant::now();
    loop {
        if let Some(token) = mailcraft_apple_bridge::push::token() {
            return Some(token);
        }
        if mailcraft_apple_bridge::push::failure().is_some() || started.elapsed() > ADDRESS_WAIT {
            return None;
        }
        std::thread::sleep(Duration::from_secs(1));
    }
}

fn linked(a: &PushAccount) -> LinkedAccount {
    LinkedAccount {
        email: a.email.clone(),
        username: a.username.clone(),
        server: mailcraft_model::Server { host: a.host.clone(), port: a.port, security: a.security },
        sign_in: a.sign_in,
        client_id: a.client_id.clone(),
        secret: a.secret.clone(),
    }
}

/// A fresh device id: 16 random bytes in hexadecimal.
fn new_device_id() -> Result<String, String> {
    let bytes = push_link::new_key().map_err(|e| e.to_string())?;
    Ok(bytes.iter().take(16).map(|b| format!("{b:02x}")).collect())
}

fn pair_with(computer: &str, code: &str, device: &str, token: &str) -> Result<String, String> {
    // A note key of this iPad's own, kept for good: the relay seals notifications with it.
    let note = match push_keys::get("note") {
        Some(key) => key,
        None => {
            let key = push_link::new_key().map_err(|e| e.to_string())?;
            push_keys::put("note", &key)?;
            key
        }
    };
    let stream = connect(computer)?;
    let request = PairRequest { device: device.into(), token: token.into(), name: "iPad".into(), note_key: push_link::key_text(&note) };
    let (answer, link) = push_link::pair(stream, code, &request).map_err(|e| match e {
        LinkError::WrongCode => "the code is wrong: type the one the relay shows (`mailcraft-relay code`)".to_string(),
        other => other.to_string(),
    })?;
    push_keys::put("link", &link)?;
    Ok(line(&answer.relay))
}

fn line(text: &str) -> String {
    text.chars().filter(|c| !c.is_control()).take(60).collect()
}

impl Push {
    /// Send what waits, in the background, until it has gone (tried again every few minutes).
    fn send_waiting(&self) {
        let Ok(mut g) = self.going.lock() else { return };
        if g.sending || g.waiting.is_none() || g.kept.paired_with.is_empty() {
            return;
        }
        g.sending = true;
        drop(g);
        let (going, ctx) = (self.going.clone(), self.ctx.clone());
        let spawned = std::thread::Builder::new().name("push sync".into()).spawn(move || {
            while let Some((kept, accounts)) = going.lock().ok().and_then(|g| Some((g.kept.clone(), g.waiting.clone()?))) {
                let outcome = match (address(), push_keys::get("link")) {
                    (None, _) => Err(LinkError::Io("Apple hasn't given this iPad its push address yet".into())),
                    (_, None) => Err(LinkError::NotPaired),
                    (Some(token), Some(link)) => {
                        let request = Request::Sync { token, name: "iPad".into(), accounts: accounts.iter().map(linked).collect() };
                        connect(&kept.computer).map_err(LinkError::Io).and_then(|s| push_link::exchange(s, &kept.device, &link, &request))
                    }
                };
                let Ok(mut g) = going.lock() else { break };
                match outcome {
                    Ok(answer) => {
                        // (Taken off only when nothing newer came meanwhile.)
                        if g.waiting.as_ref() == Some(&accounts) {
                            g.waiting = None;
                        }
                        g.relay_accounts = answer.accounts.into_iter().map(|a| (line(&a.email), a.problem.chars().take(200).collect())).collect();
                        g.heard = Some(now());
                        g.problem = None;
                        log::info!("push: the relay has this iPad's accounts");
                    }
                    Err(LinkError::NotPaired) => {
                        g.kept.paired_with.clear();
                        g.waiting = None;
                        g.problem = Some("The relay doesn't know this iPad any more: pair it again.".into());
                    }
                    Err(LinkError::Untrusted) => {
                        g.waiting = None;
                        g.problem = Some(
                            "Something at that address couldn't prove it is your relay: nothing was sent. Pair again if the relay was set up anew."
                                .into(),
                        );
                    }
                    Err(e) => log::info!("push: the accounts wait ({e})"),
                }
                if g.waiting.is_none() {
                    g.sending = false;
                    break;
                }
                drop(g);
                ctx.request_repaint();
                std::thread::sleep(RETRY_EVERY);
            }
            ctx.request_repaint();
        });
        if spawned.is_err()
            && let Ok(mut g) = self.going.lock()
        {
            g.sending = false;
        }
    }
}

impl PushHost for Push {
    fn state(&self) -> PushState {
        let address = mailcraft_apple_bridge::push::token().map(Ok).or_else(|| mailcraft_apple_bridge::push::failure().map(Err));
        let Ok(g) = self.going.lock() else { return PushState { address, ..Default::default() } };
        PushState {
            address,
            paired_with: (!g.kept.paired_with.is_empty()).then(|| g.kept.paired_with.clone()),
            pairing: g.pairing,
            problem: g.problem.clone(),
            relay_accounts: g.relay_accounts.clone(),
            heard: g.heard,
            waiting: g.waiting.is_some(),
        }
    }

    fn pair(&self, computer: &str, code: &str) {
        let missing = if computer.trim().is_empty() {
            Some("Type the computer's address, as \"mailcraft-relay code\" shows it.".to_string())
        } else if code.trim().len() != 6 || !code.trim().bytes().all(|b| b.is_ascii_digit()) {
            Some("The code is the six digits \"mailcraft-relay code\" shows.".to_string())
        } else {
            None
        };
        let token = mailcraft_apple_bridge::push::token();
        let missing = missing.or_else(|| match (&token, mailcraft_apple_bridge::push::failure()) {
            (Some(_), _) => None,
            (None, Some(why)) => Some(format!("Apple gave this iPad no push address: {why}")),
            (None, None) => Some("Apple hasn't given this iPad its push address yet: try again in a moment.".into()),
        });
        let Ok(mut g) = self.going.lock() else { return };
        if g.pairing {
            return;
        }
        if let Some(why) = missing {
            g.problem = Some(why);
            return;
        }
        let Some(token) = token else { return };
        let device = if g.kept.device.is_empty() {
            match new_device_id() {
                Ok(id) => id,
                Err(e) => {
                    g.problem = Some(e);
                    return;
                }
            }
        } else {
            g.kept.device.clone()
        };
        (g.pairing, g.problem) = (true, None);
        drop(g);
        // Notifications have to be allowed for the relay's to show.
        crate::notifications::ask_permission();
        let (computer, code, going, ctx, file) =
            (computer.trim().to_string(), code.trim().to_string(), self.going.clone(), self.ctx.clone(), self.file.clone());
        let after = Push { file: self.file.clone(), going: self.going.clone(), ctx: self.ctx.clone() };
        let spawned = std::thread::Builder::new().name("push pairing".into()).spawn(move || {
            let result = pair_with(&computer, &code, &device, &token);
            if let Ok(mut g) = going.lock() {
                g.pairing = false;
                match result {
                    Ok(name) => {
                        log::info!("push: paired with the relay");
                        g.kept = Kept { device, computer, paired_with: name };
                        write_kept(file.as_ref(), &g.kept);
                        // The accounts go next: the window hands them over again.
                        g.relay_accounts.clear();
                    }
                    Err(why) => g.problem = Some(why),
                }
            }
            after.send_waiting();
            ctx.request_repaint();
        });
        if spawned.is_err()
            && let Ok(mut g) = self.going.lock()
        {
            (g.pairing, g.problem) = (false, Some("the pairing couldn't start".into()));
        }
    }

    fn forget(&self) {
        let Ok(mut g) = self.going.lock() else { return };
        let kept = std::mem::take(&mut g.kept);
        g.kept.device = kept.device.clone();
        (g.waiting, g.relay_accounts, g.heard, g.problem) = (None, vec![], None, None);
        write_kept(self.file.as_ref(), &g.kept);
        drop(g);
        // The relay is told, when it can be reached; the keys go here either way.
        let link = push_keys::get("link");
        push_keys::forget("link");
        if let Some(link) = link
            && !kept.computer.is_empty()
        {
            let _ = std::thread::Builder::new().name("push forget".into()).spawn(move || {
                match connect(&kept.computer).map_err(LinkError::Io).and_then(|s| push_link::exchange(s, &kept.device, &link, &Request::Forget)) {
                    Ok(_) => log::info!("push: the relay forgot this iPad"),
                    Err(e) => log::info!("push: the relay couldn't be told ({e}); `mailcraft-relay unpair` there forgets this iPad"),
                }
            });
        }
    }

    fn sync(&self, accounts: Vec<PushAccount>) {
        if let Ok(mut g) = self.going.lock() {
            g.waiting = Some(accounts);
        }
        self.send_waiting();
    }
}
