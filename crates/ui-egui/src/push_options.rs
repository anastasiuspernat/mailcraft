// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! File › Options › Push from your computer (iPad): pairing with the owner's own relay
//! (`apps/mailcraft-relay`), which tells this iPad through Apple when mail arrives. The host does
//! the pairing and the exchanges that follow ([`PushHost`]); the window shows where it stands,
//! takes the computer's address and the code the relay shows, and hands the host this iPad's
//! accounts whenever they change ([`follow`]), each with what the relay signs in with: its
//! password, or its sign-in's refresh token. They go to the relay sealed, on the home network, and
//! are kept there in the computer's credential store; an account removed here is dropped there.

use egui::Ui;
use mailcraft_model::{AccountKind, OAuthProvider, Security};

use crate::MailApp;
use crate::theme::Tokens;

/// An account for the relay to watch, with what it signs in with.
#[derive(Clone, PartialEq, Eq)]
pub struct PushAccount {
    pub email: String,
    pub username: String,
    pub host: String,
    pub port: u16,
    pub security: Security,
    pub sign_in: Option<OAuthProvider>,
    pub client_id: String,
    /// The password, or the sign-in's refresh token.
    pub secret: String,
}

impl std::fmt::Debug for PushAccount {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("PushAccount").field("email", &self.email).field("host", &self.host).finish_non_exhaustive()
    }
}

/// Where push stands, as the host knows it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PushState {
    /// This device's push address from Apple: `None` while it hasn't come, `Some(Err)` when the
    /// system gave none.
    pub address: Option<Result<String, String>>,
    /// The computer this device is paired with.
    pub paired_with: Option<String>,
    /// A pairing is under way.
    pub pairing: bool,
    /// Why the last pairing or exchange didn't work.
    pub problem: Option<String>,
    /// The accounts the relay watches for this iPad, as it last said: each with what is wrong
    /// (empty: watched).
    pub relay_accounts: Vec<(String, String)>,
    /// When the relay was last heard from (seconds since 1970).
    pub heard: Option<i64>,
    /// Accounts wait to be sent (the relay wasn't reachable: away from home).
    pub waiting: bool,
}

/// The host's side: the push address, and pairing with a relay.
pub trait PushHost {
    fn state(&self) -> PushState;
    /// Pair with the relay at `computer` (an address on the home network) with `code`, in the
    /// background; [`PushHost::state`] says how it went, or what is missing to begin.
    fn pair(&self, computer: &str, code: &str);
    /// Forget the pairing: the relay is told so (when it can be reached), and the keys go here.
    fn forget(&self);
    /// This iPad's accounts are these now: send them to the relay (now, or when it can next be
    /// reached), with this iPad's push address.
    fn sync(&self, accounts: Vec<PushAccount>);
}

/// The accounts the relay can watch (IMAP, with a password or a sign-in to hand over), with what
/// they sign in with.
pub fn accounts(app: &MailApp) -> Vec<PushAccount> {
    let mut out = Vec::new();
    for a in &app.session.mailbox.accounts {
        if a.kind != AccountKind::Imap || a.shared_of.is_some() {
            continue;
        }
        let Some(incoming) = &a.incoming else { continue };
        let (sign_in, client_id, secret) = match &a.oauth {
            Some(oauth) => {
                let refresh = app.tokens.get(&a.id).map(|t| t.refresh.clone()).filter(|r| !r.is_empty()).or_else(|| app.remembered("refresh", a.id));
                let client_id = if !oauth.client_id.is_empty() {
                    oauth.client_id.clone()
                } else if oauth.provider == OAuthProvider::Google {
                    app.services.google_client_id.clone()
                } else {
                    app.services.microsoft_client_id.clone()
                };
                (Some(oauth.provider), client_id, refresh)
            }
            None => (None, String::new(), app.passwords.get(&a.id).cloned().or_else(|| app.remembered("password", a.id))),
        };
        let Some(secret) = secret.filter(|s| !s.is_empty()) else { continue };
        out.push(PushAccount {
            email: a.email.trim().to_string(),
            username: a.username.clone(),
            host: incoming.host.clone(),
            port: incoming.port,
            security: incoming.security,
            sign_in,
            client_id,
            secret,
        });
    }
    out
}

/// Once a frame: when this iPad is paired and its accounts (or what they sign in with) changed
/// since they were last handed over, hand them to the host.
pub fn follow(app: &mut MailApp) {
    let Some(host) = app.services.push.as_ref() else { return };
    if host.state().paired_with.is_none() {
        app.push_handed = None;
        return;
    }
    let list = accounts(app);
    let mut hasher = std::collections::hash_map::DefaultHasher::new();
    std::hash::Hash::hash(
        &format!("{:?}", list.iter().map(|a| (&a.email, &a.username, &a.host, a.port, &a.client_id, &a.secret)).collect::<Vec<_>>()),
        &mut hasher,
    );
    let print = std::hash::Hasher::finish(&hasher);
    if app.push_handed != Some(print) {
        app.push_handed = Some(print);
        host.sync(list);
    }
}

/// The section's rows.
pub fn options(app: &MailApp, ui: &mut Ui, t: &Tokens) {
    let Some(host) = app.services.push.as_ref() else { return };
    let state = host.state();
    ui.label(egui::RichText::new(tl!(
        "A computer of yours that is always on tells this iPad when new mail arrives, through Apple, even while MailCraft is closed. Install MailCraft relay there, and enter the address and code that \"mailcraft-relay code\" shows (this iPad and the computer on the same network, this once)."
    ))
    .color(t.text_dim));
    match (&state.paired_with, &state.address) {
        (Some(computer), _) => {
            ui.label(tl!("Paired with {computer}.").replace("{computer}", computer));
        }
        (None, Some(Err(why))) => {
            ui.label(egui::RichText::new(tl!("This iPad has no push address: {why}").replace("{why}", why)).color(t.text_dim));
        }
        (None, None) => {
            ui.label(egui::RichText::new(tl!("Waiting for this iPad's push address from Apple…")).color(t.text_dim));
        }
        (None, Some(Ok(_))) => {
            ui.label(tl!("Not paired."));
        }
    }
    let id = ui.make_persistent_id("push_pairing");
    let (mut computer, mut code) = ui.data_mut(|d| d.get_temp::<(String, String)>(id)).unwrap_or_default();
    ui.horizontal(|ui| {
        ui.label(tl!("Computer"));
        ui.add(egui::TextEdit::singleline(&mut computer).hint_text("192.168.1.20").desired_width(160.0));
        ui.label(tl!("Code"));
        ui.add(egui::TextEdit::singleline(&mut code).hint_text("123456").desired_width(80.0));
        // Always there to press (but while a pairing is under way): what is missing is said then.
        let label = if state.paired_with.is_some() { tl!("Pair again") } else { tl!("Pair") };
        if ui.add_enabled(!state.pairing, egui::Button::new(label)).clicked() {
            host.pair(computer.trim(), code.trim());
        }
        if state.pairing {
            ui.spinner();
        }
        if state.paired_with.is_some() && !state.pairing && ui.button(tl!("Forget")).clicked() {
            host.forget();
        }
    });
    ui.data_mut(|d| d.insert_temp(id, (computer, code)));
    if let Some(problem) = &state.problem {
        ui.label(egui::RichText::new(problem).color(t.text_dim));
    }
    if state.paired_with.is_some() {
        if state.waiting {
            ui.label(
                egui::RichText::new(tl!("Your accounts go to the relay when this iPad is next on the same network as the computer."))
                    .color(t.text_dim),
            );
        }
        if let Some(heard) = state.heard {
            let when = crate::dates::list_label(heard, app.session.now(), app.session.offset_at(heard), app.session.prefs.twelve_hour);
            ui.label(egui::RichText::new(tl!("Last heard from the relay: {when}").replace("{when}", &when)).color(t.text_dim));
        }
        for (email, problem) in &state.relay_accounts {
            let line = if problem.is_empty() {
                tl!("{account}: watched").replace("{account}", email)
            } else {
                tl!("{account}: {problem}").replace("{account}", email).replace("{problem}", problem)
            };
            ui.label(egui::RichText::new(line).color(if problem.is_empty() { t.text_dim } else { t.red }));
        }
    }
}
