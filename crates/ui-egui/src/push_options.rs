// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! File › Options › Push from your computer (iPad): pairing with the owner's own relay
//! (`apps/mailcraft-relay`), which tells this iPad through Apple when mail arrives. The host does
//! the pairing ([`PushHost`]); the window shows where it stands and takes the computer's address
//! and the code the relay shows.

use egui::Ui;

use crate::MailApp;
use crate::theme::Tokens;

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
    /// Why the last pairing didn't work.
    pub problem: Option<String>,
}

/// The host's side: the push address, and pairing with a relay.
pub trait PushHost {
    fn state(&self) -> PushState;
    /// Pair with the relay at `computer` (an address on the home network) with `code`, in the
    /// background; [`PushHost::state`] says how it went, or what is missing to begin.
    fn pair(&self, computer: &str, code: &str);
    /// Forget the pairing here (the relay keeps sending until it is told the device is gone).
    fn forget(&self);
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
}
