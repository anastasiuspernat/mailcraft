// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! `mailcraft-relay-service`: the relay at work, with no console window (Windows starts it at
//! sign-in: `mailcraft-relay install`). What it does goes to its log.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

fn main() {
    let Some(folder) = mailcraft_relay::config::folder() else { return };
    mailcraft_relay::logfile::start(&folder, false);
    if let Err(e) = mailcraft_relay::watcher::run(&folder) {
        log::error!("{e}");
    }
}
