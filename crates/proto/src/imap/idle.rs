// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! Waiting for news of the selected mailbox: IDLE (RFC 2177), and NOOP for a server without it.
//!
//! IDLE asks the server to say at once when the mailbox changes, so that a program watching for
//! new mail hears of it in seconds instead of at its next look. The server's word is untagged
//! data (`* 4 EXISTS` when a message arrives); IDLE ends with `DONE` from the client and the
//! server's tagged answer. RFC 2177 asks that it be renewed before the server's 30 minutes of
//! patience run out, and a socket that hears nothing for a long time may be dead: the caller
//! gives [`Imap::idle`] its longest wait and sets the socket's read timeout, and a read that times
//! out is an error after which the session is not used again.

use std::io::{Read, Write};
use std::time::{Duration, Instant};

use super::{Imap, Response, ignore};
use crate::ProtoError;
use crate::imap::parse::{Status, Value};

/// How an [`Imap::idle`] ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Idled {
    /// The server said the mailbox changed: a message came, went or had its flags changed.
    Changed,
    /// The longest wait passed without a change.
    Quiet,
}

/// Untagged data that says the selected mailbox changed.
fn is_change(values: &[Value]) -> bool {
    let word = values.get(1).and_then(Value::atom).unwrap_or("");
    ["EXISTS", "EXPUNGE", "FETCH", "RECENT"].iter().any(|w| word.eq_ignore_ascii_case(w))
}

impl<S: Read + Write> Imap<S> {
    /// Whether the server said it can IDLE.
    pub fn offers_idle(&self) -> bool {
        self.offers("IDLE")
    }

    /// NOOP: nothing, but the server tells what changed meanwhile (`exists` of the selected
    /// mailbox is kept up to date).
    pub fn noop(&mut self) -> Result<(), ProtoError> {
        self.run(super::Command::new("NOOP"), &mut ignore).map(|_| ())
    }

    /// IDLE on the selected mailbox until the server says it changed, or until `longest` has
    /// passed (looked at as each line arrives), or until a read times out (the caller sets the
    /// socket's read timeout to about `longest`: a server that sends nothing while it idles, as
    /// Gmail does, ends the wait quietly). Then DONE, and the server's answer to the IDLE: a
    /// server that doesn't answer that either is gone, and that is an error.
    pub fn idle(&mut self, longest: Duration) -> Result<Idled, ProtoError> {
        let started = Instant::now();
        let tag = self.tag();
        log::trace!("C: {tag} IDLE");
        self.wire.write(format!("{tag} IDLE\r\n").as_bytes())?;
        self.wire.flush()?;
        self.text_seen = 0;
        // The go-ahead, or a refusal (a server that can't IDLE says BAD).
        if let Some(line) = self.read_until(&tag, true, &mut ignore)? {
            return Err(match line.status {
                Status::Ok => ProtoError::Protocol("the server ended IDLE before it began".into()),
                _ => ProtoError::Rejected(self.refusal(&line)),
            });
        }
        let mut heard = Idled::Quiet;
        let _ = self.wire.take_timed_out();
        for _ in 0..self.response_cap {
            if started.elapsed() >= longest {
                break;
            }
            // Each line is counted by itself: a wait of many minutes may hear many keep-alives.
            self.text_seen = 0;
            let response = match self.read_response() {
                Ok(response) => response,
                Err(_) if self.wire.take_timed_out() => break,
                Err(e) => return Err(e),
            };
            match response {
                Response::Data(values) => {
                    self.note_data(&values);
                    if is_change(&values) {
                        heard = Idled::Changed;
                        break;
                    }
                }
                Response::Status { tag: None, line } => {
                    if line.status == Status::Bye {
                        return Err(ProtoError::Io(format!("the server ended the session: {}", self.refusal(&line))));
                    }
                    self.note_status(&line);
                }
                Response::Status { tag: Some(_), line } => {
                    return Err(match line.status {
                        Status::Ok => ProtoError::Protocol("the server ended IDLE by itself".into()),
                        _ => ProtoError::Rejected(self.refusal(&line)),
                    });
                }
                Response::GoAhead => return Err(ProtoError::Protocol("the server asked for more during IDLE".into())),
            }
        }
        log::trace!("C: DONE");
        self.wire.write(b"DONE\r\n")?;
        self.wire.flush()?;
        self.text_seen = 0;
        match self.read_until(&tag, false, &mut ignore)? {
            Some(line) if line.status == Status::Ok => Ok(heard),
            Some(line) => Err(ProtoError::Rejected(self.refusal(&line))),
            None => Err(ProtoError::Protocol("the server asked for more after DONE".into())),
        }
    }
}
