// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Push from the owner's own relay (`apps/mailcraft-relay`, `mailcraft_ui_egui::push_options`):
//! this iPad's push address comes from Apple (`mailcraft_apple_bridge::push`), and pairing sends
//! it, with the code the relay shows, to the relay on the home network, on its port 7993: one
//! line of JSON each way. The computer it is paired with is kept in `push.json` beside the
//! mailbox, for the window to say so; nothing else is kept.

use std::io::{BufRead, BufReader, Read as _, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use mailcraft_ui_egui::push_options::{PushHost, PushState};

/// The relay's pairing port.
const PAIR_PORT: u16 = 7993;
/// How long connecting, and then each read or write, may take.
const PATIENCE: Duration = Duration::from_secs(8);

#[derive(Default)]
struct Going {
    pairing: bool,
    problem: Option<String>,
    paired_with: Option<String>,
}

pub struct Push {
    file: Option<PathBuf>,
    going: Arc<Mutex<Going>>,
    ctx: egui::Context,
}

fn read_paired(file: Option<&PathBuf>) -> Option<String> {
    let bytes = std::fs::read(file?).ok()?;
    let value: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    value.get("pairedWith")?.as_str().map(|s| s.chars().take(60).collect())
}

impl Push {
    /// Asks Apple for this iPad's push address; `file` keeps whom it is paired with.
    pub fn new(ctx: egui::Context, file: Option<PathBuf>) -> Push {
        mailcraft_apple_bridge::push::register();
        let paired_with = read_paired(file.as_ref());
        Push { file, going: Arc::new(Mutex::new(Going { paired_with, ..Default::default() })), ctx }
    }
}

/// Send `token` and `code` to the relay at `computer`, and read its answer: the computer's name.
fn pair_with(computer: &str, code: &str, token: &str) -> Result<String, String> {
    let address = (computer, PAIR_PORT)
        .to_socket_addrs()
        .map_err(|_| "that computer's name couldn't be found: type its address, as the relay shows it".to_string())?
        .next()
        .ok_or("that computer's name couldn't be found")?;
    let stream = TcpStream::connect_timeout(&address, PATIENCE)
        .map_err(|_| "the relay didn't answer: is \"mailcraft-relay pair\" running, and is this iPad on the same network?".to_string())?;
    let _ = stream.set_read_timeout(Some(PATIENCE));
    let _ = stream.set_write_timeout(Some(PATIENCE));
    // (iPadOS gives an app the device's own name only with an entitlement Apple hands out.)
    let line = serde_json::json!({ "code": code, "token": token, "name": "iPad" }).to_string();
    let mut writer = stream.try_clone().map_err(|e| e.to_string())?;
    writer.write_all(format!("{line}\n").as_bytes()).map_err(|_| "the relay stopped answering".to_string())?;
    let mut answer = String::new();
    BufReader::new(stream).take(2048).read_line(&mut answer).map_err(|_| "the relay stopped answering".to_string())?;
    let value: serde_json::Value = serde_json::from_str(answer.trim()).map_err(|_| "that isn't MailCraft relay".to_string())?;
    if value.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
        let relay = value.get("relay").and_then(|r| r.as_str()).unwrap_or("your computer");
        Ok(relay.chars().filter(|c| !c.is_control()).take(60).collect())
    } else if value.get("why").and_then(|w| w.as_str()) == Some("wrong code") {
        Err("the code is wrong: type the one the relay shows".into())
    } else {
        Err("the relay said no".into())
    }
}

impl PushHost for Push {
    fn state(&self) -> PushState {
        let address = mailcraft_apple_bridge::push::token().map(Ok).or_else(|| mailcraft_apple_bridge::push::failure().map(Err));
        let Ok(going) = self.going.lock() else { return PushState { address, ..Default::default() } };
        PushState { address, paired_with: going.paired_with.clone(), pairing: going.pairing, problem: going.problem.clone() }
    }

    fn pair(&self, computer: &str, code: &str) {
        let Some(token) = mailcraft_apple_bridge::push::token() else { return };
        if let Ok(mut going) = self.going.lock() {
            if going.pairing {
                return;
            }
            (going.pairing, going.problem) = (true, None);
        }
        // Notifications have to be allowed for the relay's to show.
        crate::notifications::ask_permission();
        let (computer, code, going, ctx, file) = (computer.to_string(), code.to_string(), self.going.clone(), self.ctx.clone(), self.file.clone());
        let spawned = std::thread::Builder::new().name("push pairing".into()).spawn(move || {
            let result = pair_with(&computer, &code, &token);
            if let Ok(name) = &result
                && let Some(file) = &file
                && let Err(e) = std::fs::write(file, serde_json::json!({ "pairedWith": name }).to_string())
            {
                log::warn!("whom push is paired with couldn't be kept: {e}");
            }
            if let Ok(mut g) = going.lock() {
                g.pairing = false;
                match result {
                    Ok(name) => {
                        log::info!("push: paired with the relay");
                        g.paired_with = Some(name);
                    }
                    Err(why) => g.problem = Some(why),
                }
            }
            ctx.request_repaint();
        });
        if spawned.is_err()
            && let Ok(mut g) = self.going.lock()
        {
            (g.pairing, g.problem) = (false, Some("the pairing couldn't start".into()));
        }
    }

    fn forget(&self) {
        if let Some(file) = &self.file {
            let _ = std::fs::remove_file(file);
        }
        if let Ok(mut g) = self.going.lock() {
            g.paired_with = None;
        }
    }
}
