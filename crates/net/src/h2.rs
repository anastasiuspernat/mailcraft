// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The least of HTTP/2 (RFC 9113) that one request needs: Apple's push service speaks nothing
//! else. A connection carries one POST on stream 1 and is closed after its answer.
//!
//! The request's headers are written with HPACK (RFC 7541) without the dynamic table or Huffman
//! coding, which a server must accept. Of the answer's headers only the status is read, when it
//! is written the ways a server writes it in the first place (a static-table entry, or a literal
//! after `:status`'s name, not Huffman-coded); the body comes in DATA frames, which HPACK has no
//! part in. The server's SETTINGS are acknowledged and its PINGs answered; nothing else it may
//! send is needed for one small request and its answer.

use std::io::{Read, Write};

/// What every HTTP/2 connection begins with (RFC 9113 §3.4).
const PREFACE: &[u8] = b"PRI * HTTP/2.0\r\n\r\nSM\r\n\r\n";
/// The largest frame either side sends without having said otherwise (RFC 9113 §6.5.2).
const MAX_FRAME: usize = 16_384;
/// The most of an answer's body taken.
const MAX_BODY: usize = 16_384;
/// The most frames read for one answer.
const MAX_FRAMES: usize = 256;

const DATA: u8 = 0x0;
const HEADERS: u8 = 0x1;
const RST_STREAM: u8 = 0x3;
const SETTINGS: u8 = 0x4;
const PING: u8 = 0x6;
const GOAWAY: u8 = 0x7;
const CONTINUATION: u8 = 0x9;

const END_STREAM: u8 = 0x1;
const ACK: u8 = 0x1;
const END_HEADERS: u8 = 0x4;
const PADDED: u8 = 0x8;
const PRIORITY: u8 = 0x20;

/// The stream the one request goes on.
const STREAM: u32 = 1;

/// The server's answer.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Answer {
    /// `None` when the status was written in a way that isn't read here.
    pub(crate) status: Option<u16>,
    pub(crate) body: Vec<u8>,
}

/// An HPACK integer with an `n`-bit prefix, the prefix's other bits `first` (RFC 7541 §5.1).
fn integer(out: &mut Vec<u8>, first: u8, n: u32, value: usize) {
    let most = (1usize << n) - 1;
    if value < most {
        out.push(first | value as u8);
        return;
    }
    out.push(first | most as u8);
    let mut rest = value - most;
    while rest >= 128 {
        out.push((rest % 128) as u8 | 0x80);
        rest /= 128;
    }
    out.push(rest as u8);
}

/// An HPACK string, not Huffman-coded (RFC 7541 §5.2).
fn string(out: &mut Vec<u8>, text: &str) {
    integer(out, 0, 7, text.len());
    out.extend_from_slice(text.as_bytes());
}

/// The header block of a POST to `authority` and `path`, with `headers` (names in lower case).
fn header_block(authority: &str, path: &str, headers: &[(&str, &str)]) -> Vec<u8> {
    let mut out = Vec::new();
    // Indexed fields of the static table (RFC 7541 Appendix A): 3 is `:method POST`, 7 `:scheme https`.
    out.push(0x80 | 3);
    out.push(0x80 | 7);
    // Literals without indexing, with the name from the static table: 1 is `:authority`, 4 `:path`.
    integer(&mut out, 0, 4, 1);
    string(&mut out, authority);
    integer(&mut out, 0, 4, 4);
    string(&mut out, path);
    for (name, value) in headers {
        out.push(0);
        string(&mut out, name);
        string(&mut out, value);
    }
    out
}

fn frame(out: &mut Vec<u8>, kind: u8, flags: u8, stream: u32, payload: &[u8]) {
    let len = payload.len();
    out.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8, kind, flags]);
    out.extend_from_slice(&(stream & 0x7fff_ffff).to_be_bytes());
    out.extend_from_slice(payload);
}

/// The status from the start of a header block, when it is written as servers write it.
fn status(block: &[u8]) -> Option<u16> {
    let first = *block.first()?;
    if first & 0x80 != 0 {
        // An indexed field: the static table's statuses are 8 to 14.
        return match first & 0x7f {
            8 => Some(200),
            9 => Some(204),
            10 => Some(206),
            11 => Some(304),
            12 => Some(400),
            13 => Some(404),
            14 => Some(500),
            _ => None,
        };
    }
    // A literal whose name is `:status` from the static table (8 to 14), with or without indexing.
    let index = if first & 0x40 != 0 { first & 0x3f } else { first & 0x0f };
    if !(8..=14).contains(&index) {
        return None;
    }
    let length = *block.get(1)?;
    if length & 0x80 != 0 || length > 3 {
        return None;
    }
    let digits = block.get(2..2 + usize::from(length))?;
    std::str::from_utf8(digits).ok()?.parse().ok()
}

/// One frame from the server: its type, flags, stream and payload.
fn read_frame(io: &mut impl Read) -> Result<(u8, u8, u32, Vec<u8>), String> {
    let mut head = [0u8; 9];
    io.read_exact(&mut head).map_err(|e| format!("the connection broke ({e})"))?;
    let len = (usize::from(head[0]) << 16) | (usize::from(head[1]) << 8) | usize::from(head[2]);
    if len > MAX_FRAME {
        return Err("the server sent a frame larger than was agreed".into());
    }
    let stream = u32::from_be_bytes([head[5], head[6], head[7], head[8]]) & 0x7fff_ffff;
    let mut payload = vec![0u8; len];
    io.read_exact(&mut payload).map_err(|e| format!("the connection broke ({e})"))?;
    Ok((head[3], head[4], stream, payload))
}

/// A frame's payload without its padding (and, for HEADERS, its priority).
fn unpadded(kind: u8, flags: u8, payload: &[u8]) -> Result<&[u8], String> {
    let mut rest = payload;
    let mut pad = 0usize;
    if flags & PADDED != 0 {
        let (first, after) = rest.split_first().ok_or("an empty padded frame")?;
        pad = usize::from(*first);
        rest = after;
    }
    if kind == HEADERS && flags & PRIORITY != 0 {
        rest = rest.get(5..).ok_or("a HEADERS frame too short for its priority")?;
    }
    rest.get(..rest.len().checked_sub(pad).ok_or("more padding than frame")?).ok_or_else(|| "more padding than frame".into())
}

/// POST `body` to `path` at `authority` over `io`, a fresh connection that agreed on HTTP/2
/// ("h2"), and read the answer.
pub(crate) fn post(io: &mut (impl Read + Write), authority: &str, path: &str, headers: &[(&str, &str)], body: &[u8]) -> Result<Answer, String> {
    if body.len() > MAX_FRAME {
        return Err("the request is too large".into());
    }
    let mut out = PREFACE.to_vec();
    frame(&mut out, SETTINGS, 0, 0, &[]);
    let block = header_block(authority, path, headers);
    if block.len() > MAX_FRAME {
        return Err("the request's headers are too large".into());
    }
    frame(&mut out, HEADERS, END_HEADERS, STREAM, &block);
    frame(&mut out, DATA, END_STREAM, STREAM, body);
    io.write_all(&out).and_then(|()| io.flush()).map_err(|e| format!("the request couldn't be sent ({e})"))?;

    let mut answer = Answer::default();
    let mut headers_seen = false;
    for _ in 0..MAX_FRAMES {
        let (kind, flags, stream, payload) = read_frame(io)?;
        match (kind, stream) {
            (SETTINGS, 0) if flags & ACK == 0 => {
                let mut ack = Vec::new();
                frame(&mut ack, SETTINGS, ACK, 0, &[]);
                io.write_all(&ack).and_then(|()| io.flush()).map_err(|e| format!("the connection broke ({e})"))?;
            }
            (PING, 0) if flags & ACK == 0 => {
                let mut pong = Vec::new();
                frame(&mut pong, PING, ACK, 0, &payload);
                io.write_all(&pong).and_then(|()| io.flush()).map_err(|e| format!("the connection broke ({e})"))?;
            }
            (GOAWAY, 0) => {
                let said = String::from_utf8_lossy(payload.get(8..).unwrap_or(&[])).chars().take(200).collect::<String>();
                return Err(format!("the server closed the connection{}", if said.is_empty() { String::new() } else { format!(": {said}") }));
            }
            (RST_STREAM, STREAM) => return Err("the server called the request off".into()),
            (HEADERS, STREAM) | (CONTINUATION, STREAM) => {
                let block = unpadded(kind, flags, &payload)?;
                if kind == HEADERS && !headers_seen {
                    answer.status = status(block);
                    headers_seen = true;
                }
                if flags & END_STREAM != 0 && kind == HEADERS {
                    return Ok(answer);
                }
            }
            (DATA, STREAM) => {
                let data = unpadded(kind, flags, &payload)?;
                if answer.body.len().saturating_add(data.len()) <= MAX_BODY {
                    answer.body.extend_from_slice(data);
                }
                if flags & END_STREAM != 0 {
                    return Ok(answer);
                }
            }
            // Acknowledgements, window updates, priorities, frames of other streams.
            _ => {}
        }
    }
    Err("the server sent too much without answering".into())
}

#[cfg(test)]
pub(crate) mod testing {
    /// A frame as the server writes it, for the tests.
    pub(crate) fn frame(kind: u8, flags: u8, stream: u32, payload: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        super::frame(&mut out, kind, flags, stream, payload);
        out
    }
}
