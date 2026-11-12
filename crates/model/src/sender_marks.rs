// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What the owner taught MailCraft about senders, besides VIPs: junk learned from their marks,
//! senders whose pictures are blurred (NSFW), and an emoji for a sender. Each is kept by the
//! sender's address in lower case, on this computer only, never sent anywhere, at most
//! [`MAX_SENDERS`] senders each, and tidied when a mailbox is read ([`Mailbox::repair`]).

use crate::Mailbox;
use crate::mailbox::vip_key;

/// The most senders each mark is kept for.
pub const MAX_SENDERS: usize = 10_000;
/// How many times the owner moves a sender's mail to Junk E-mail before their later mail is
/// marked as junk.
pub const JUNK_LEARNED: u8 = 3;
/// The longest emoji kept for a sender, in bytes (a family or a flag with its joiners fits).
pub const MAX_EMOJI_BYTES: usize = 32;

/// `text` made fit to be a sender's emoji: trimmed, one short run of characters (at most
/// [`MAX_EMOJI_BYTES`] bytes and 8 characters), none of them a control, format-only space or
/// letter or digit of a script (an emoji, a symbol, a mark). `None` when nothing is left of it.
pub fn emoji(text: &str) -> Option<String> {
    let t = text.trim();
    if t.is_empty() || t.len() > MAX_EMOJI_BYTES || t.chars().count() > 8 {
        return None;
    }
    let fits = t.chars().all(|c| !c.is_control() && !c.is_whitespace() && !c.is_alphanumeric() && !matches!(c, '<' | '>' | '"' | '\\' | '/'));
    fits.then(|| t.to_string())
}

impl Mailbox {
    /// Whether `address`'s mail is marked as junk: the owner moved their mail to Junk E-mail
    /// [`JUNK_LEARNED`] times, and hasn't said Not Junk since.
    pub fn is_learned_junk(&self, address: &str) -> bool {
        !self.junk_marks.is_empty() && self.junk_marks.get(&vip_key(address)).is_some_and(|n| *n >= JUNK_LEARNED)
    }

    /// The owner moved one of `address`'s messages to Junk E-mail: counted. Whether that made
    /// the sender learned junk just now.
    pub fn count_junk(&mut self, address: &str) -> bool {
        let key = vip_key(address);
        if key.is_empty() || (!self.junk_marks.contains_key(&key) && self.junk_marks.len() >= MAX_SENDERS) {
            return false;
        }
        let n = self.junk_marks.entry(key).or_insert(0);
        let was = *n;
        *n = n.saturating_add(1).min(JUNK_LEARNED);
        was < JUNK_LEARNED && *n >= JUNK_LEARNED
    }

    /// The owner said `address`'s mail is junk (`true`: learned at once) or not (forgotten).
    /// Whether anything changed.
    pub fn set_learned_junk(&mut self, address: &str, junk: bool) -> bool {
        let key = vip_key(address);
        if key.is_empty() {
            return false;
        }
        if !junk {
            return self.junk_marks.remove(&key).is_some();
        }
        if !self.junk_marks.contains_key(&key) && self.junk_marks.len() >= MAX_SENDERS {
            return false;
        }
        self.junk_marks.insert(key, JUNK_LEARNED) != Some(JUNK_LEARNED)
    }

    /// Whether `address`'s pictures are drawn blurred.
    pub fn is_nsfw(&self, address: &str) -> bool {
        !self.nsfw_senders.is_empty() && self.nsfw_senders.contains(&vip_key(address))
    }

    /// Blur `address`'s pictures (`true`) or not. Whether that changed anything.
    pub fn set_nsfw(&mut self, address: &str, nsfw: bool) -> bool {
        let key = vip_key(address);
        if key.is_empty() {
            return false;
        }
        if !nsfw {
            return self.nsfw_senders.remove(&key);
        }
        self.nsfw_senders.len() < MAX_SENDERS && self.nsfw_senders.insert(key)
    }

    /// The emoji the owner gave `address`, if any.
    pub fn sender_emoji_of(&self, address: &str) -> Option<&str> {
        if self.sender_emoji.is_empty() {
            return None;
        }
        self.sender_emoji.get(&vip_key(address)).map(String::as_str)
    }

    /// Give `address` an emoji (made fit by [`emoji`]), or take it away (`None`). Whether that
    /// changed anything; an emoji that isn't one changes nothing.
    pub fn set_sender_emoji(&mut self, address: &str, emoji: Option<&str>) -> bool {
        let key = vip_key(address);
        if key.is_empty() {
            return false;
        }
        let Some(text) = emoji else { return self.sender_emoji.remove(&key).is_some() };
        let Some(text) = self::emoji(text) else { return false };
        if !self.sender_emoji.contains_key(&key) && self.sender_emoji.len() >= MAX_SENDERS {
            return false;
        }
        self.sender_emoji.insert(key, text.clone()) != Some(text)
    }

    /// Keys in lower case and none empty, counts at most [`JUNK_LEARNED`] and none zero, emoji
    /// that are emoji, at most [`MAX_SENDERS`] of each. Whether anything changed.
    pub(crate) fn repair_sender_marks(&mut self) -> bool {
        let mut changed = false;
        let marks = std::mem::take(&mut self.junk_marks);
        let before = marks.len();
        for (k, n) in marks {
            let key = vip_key(&k);
            let n2 = n.min(JUNK_LEARNED);
            if key.is_empty() || n2 == 0 || self.junk_marks.len() >= MAX_SENDERS {
                changed = true;
                continue;
            }
            changed |= key != k || n2 != n;
            self.junk_marks.insert(key, n2);
        }
        changed |= self.junk_marks.len() != before;
        let nsfw = std::mem::take(&mut self.nsfw_senders);
        let before = nsfw.len();
        for k in nsfw {
            let key = vip_key(&k);
            changed |= key != k;
            if !key.is_empty() && self.nsfw_senders.len() < MAX_SENDERS {
                self.nsfw_senders.insert(key);
            }
        }
        changed |= self.nsfw_senders.len() != before;
        let emoji = std::mem::take(&mut self.sender_emoji);
        let before = emoji.len();
        for (k, e) in emoji {
            let key = vip_key(&k);
            match self::emoji(&e) {
                Some(e2) if !key.is_empty() && self.sender_emoji.len() < MAX_SENDERS => {
                    changed |= key != k || e2 != e;
                    self.sender_emoji.insert(key, e2);
                }
                _ => changed = true,
            }
        }
        changed |= self.sender_emoji.len() != before;
        changed
    }
}

#[cfg(test)]
#[path = "sender_marks_tests.rs"]
mod sender_marks_tests;
