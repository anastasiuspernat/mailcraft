// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Watching an account's Inbox for new mail, for the owner's push relay (`apps/mailcraft-relay`):
//! one connection that is signed in, has the Inbox open read-only (EXAMINE: nothing is marked
//! read, moved or changed), and waits for the server's word with IDLE (RFC 2177), or with a NOOP
//! a minute where the server can't IDLE. Nothing of a message is read but its UID, its flags and,
//! for the newest new one, the sender and the subject in its header ([`Inbox::about`]).

use std::time::Duration;

use mailcraft_model::Account;
use mailcraft_proto::imap::{Idled, Imap};

use crate::NetError;
use crate::sync::open_imap;
use crate::transport::Stream;

/// How long one IDLE lasts before it is renewed on the same connection: well inside the 29
/// minutes RFC 2177 allows, and short enough that a connection that died without a word is
/// noticed (it doesn't answer the renewal). A read waits as long: a server that says nothing
/// while it idles (Gmail) ends the wait quietly, not the connection.
const IDLE_FOR: Duration = Duration::from_secs(9 * 60);
/// How often a server that can't IDLE is asked.
const POLL_EVERY: Duration = Duration::from_secs(60);

/// An Inbox being watched.
pub struct Inbox {
    imap: Imap<Stream<'static>>,
    uid_validity: u32,
    uid_next: u32,
    idle: bool,
}

impl std::fmt::Debug for Inbox {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Inbox").field("uid_validity", &self.uid_validity).field("uid_next", &self.uid_next).field("idle", &self.idle).finish()
    }
}

/// Sign in to `account`'s incoming server with `secret` (its password, or for an account that
/// signs in with OAuth, an access token) and open its Inbox read-only.
pub fn open(account: &Account, secret: &str) -> Result<Inbox, NetError> {
    let mut imap = open_imap(account, secret)?;
    let idle = imap.capabilities()?.iter().any(|c| c == "IDLE");
    let selected = imap.select("INBOX", true)?;
    Ok(Inbox { imap, uid_validity: selected.uid_validity, uid_next: selected.uid_next, idle })
}

impl Inbox {
    /// The Inbox's UIDVALIDITY: when it changes, UIDs seen before mean nothing.
    pub fn uid_validity(&self) -> u32 {
        self.uid_validity
    }

    /// The UID the next message will have, as the server said when the Inbox was opened.
    pub fn uid_next(&self) -> u32 {
        self.uid_next
    }

    /// Whether the server can IDLE (else it is asked every minute).
    pub fn idles(&self) -> bool {
        self.idle
    }

    /// Wait until the server says the Inbox changed, or some minutes have passed. An error means
    /// the connection is no good any more.
    pub fn wait(&mut self) -> Result<(), NetError> {
        if !self.idle {
            std::thread::sleep(POLL_EVERY);
            return Ok(self.imap.noop()?);
        }
        self.imap.get_mut().listen_for(IDLE_FOR).map_err(|e| NetError::Proto(mailcraft_proto::ProtoError::Io(e.to_string())))?;
        let idled: Result<Idled, _> = self.imap.idle(IDLE_FOR);
        self.imap.get_mut().listen_done().map_err(|e| NetError::Proto(mailcraft_proto::ProtoError::Io(e.to_string())))?;
        idled.map(|_| ()).map_err(NetError::from)
    }

    /// The UIDs of the unread messages that came from `from` on (in UID order), and the UID to
    /// look from next time.
    pub fn unread_since(&mut self, from: u32) -> Result<(Vec<u32>, u32), NetError> {
        let from = from.max(1);
        let fetched = self.imap.uid_fetch_flags(&format!("{from}:*"))?;
        // `n:*` names the last message even when it is older than `n`.
        let newer: Vec<_> = fetched.into_iter().filter(|f| f.uid >= from).collect();
        let next = newer.iter().map(|f| f.uid.saturating_add(1)).max().unwrap_or(from);
        let unread = newer.iter().filter(|f| !f.flags.iter().any(|flag| flag.eq_ignore_ascii_case("\\Seen"))).map(|f| f.uid).collect();
        Ok((unread, next))
    }

    /// Who message `uid` is from (the name, else the address) and its subject, from its header
    /// alone, read without marking it read (BODY.PEEK, and the Inbox is open read-only).
    pub fn about(&mut self, uid: u32) -> Result<(String, String), NetError> {
        let Some(fetched) = self.imap.uid_fetch_header(uid)? else { return Ok(Default::default()) };
        let message = mailcraft_mime::parse(fetched.body.as_deref().unwrap_or(&[]));
        let from = if message.from.name.trim().is_empty() { message.from.email } else { message.from.name };
        let tidy =
            |text: &str, most: usize| text.chars().map(|c| if c.is_control() { ' ' } else { c }).take(most).collect::<String>().trim().to_string();
        Ok((tidy(&from, 80), tidy(&message.subject, 160)))
    }

    /// Say goodbye to the server.
    pub fn close(self) {
        let mut imap = self.imap;
        imap.get_mut().leave();
        let _ = imap.logout();
    }
}
