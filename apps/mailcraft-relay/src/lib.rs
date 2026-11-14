// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The owner's own push relay for MailCraft on iPad.
//!
//! iPadOS keeps no connection to a mail server open for an app in the background, so an iPad
//! hears of new mail only when something tells Apple's push service to tell it. This program does
//! that, on a computer of the owner's that is always on: it watches each Inbox the owner adds
//! (IMAP IDLE, `mailcraft_net::watch`), and when unread mail arrives it sends the paired iPads a
//! notification through Apple (`mailcraft_net::apns`), signed with the owner's own push key.
//!
//! What goes where:
//! - Passwords, sign-ins (Microsoft's refresh token) and the push key are kept in the system's
//!   credential store (`store`), never in a file or a log.
//! - The settings file (`config`) holds the accounts' addresses and servers, the paired iPads'
//!   push addresses (device tokens), the push key's id and the team's, and per account the UID
//!   it has looked up to.
//! - A notification says which account has new mail and how many messages: never a sender, a
//!   subject or anything else of a message, which Apple would see.
//! - Pairing (`pair`) happens on the home network, once: the iPad sends its push address and a
//!   code the relay shows.

#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod commands;
pub mod config;
pub mod logfile;
pub mod pair;
pub mod store;
pub mod watcher;

#[cfg(test)]
mod config_tests;
#[cfg(test)]
mod pair_tests;

/// The iPad app the notifications are for (its bundle id).
pub const TOPIC: &str = "crispy.pixels.mailcraft";
/// The port the relay listens on, on the home network, while it pairs.
pub const PAIR_PORT: u16 = 7993;
/// The port on this computer alone that a running relay holds, so that only one runs.
pub const ONE_ONLY_PORT: u16 = 7994;
