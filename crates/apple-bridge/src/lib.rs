// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The one place MailCraft calls Apple's frameworks with `unsafe` code (the owner's exception to
//! the workspace's `forbid(unsafe_code)`): what has no safe binding, behind a safe interface.
//!
//! - [`auth_sheet`]: Apple's sign-in sheet (ASWebAuthenticationSession) on iPad, which shows the
//!   provider's page over MailCraft, so that MailCraft keeps running while the owner signs in and
//!   the sign-in comes back to it on its loopback address as on the desktop.
//! - [`model`]: Apple's on-device model (Foundation Models) for the smart Inbox on iPad, through
//!   two Swift functions (`swift/MailCraftModel.swift`, built by `build.rs`).
//!
//! Elsewhere (not iPadOS) everything here does nothing and says so.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod auth_sheet;
pub mod model;
