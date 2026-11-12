// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The meaning index: for each message, what an embedding model on this computer made of its
//! words, as [`EMBED_DIM`] numbers, so that a search finds mail that means what was asked even
//! when it doesn't use those words (search by meaning, beside the word index of [`crate::words`]).
//!
//! ```text
//! mailbox.mailcraft           the mailbox (store.rs)
//! mailbox.mailcraft.words     the word index
//! mailbox.mailcraft.vectors   this
//! ```
//!
//! - **Derived data.** Everything in it can be made again from the mailbox: a file that is
//!   damaged, of another version, or of another model is thrown away and made again, and opening
//!   the mailbox never waits for it or needs it.
//! - **Small:** each number is kept as one byte, the message's numbers scaled to its largest
//!   (a search compares them with the search's own numbers at full precision): about 400 bytes a
//!   message, 80 MB for 200,000.
//! - **The file** is a header (the version, how many numbers, the model's name) and a log of
//!   fixed-size records, each a message's id, a fingerprint of what was read ([`fingerprint`]),
//!   its numbers and a checksum; a later record for a message stands for it, and one with no
//!   numbers takes it out. Records are appended as they come ([`Vectors::flush`]); when what was
//!   taken out or replaced outweighs what is left, the file is written again whole, to a
//!   temporary file renamed over. A record cut short at the end is left out; any other damage
//!   makes the whole file go.
//! - **Private as the mail:** the file is for its owner only, never synchronised, exported or
//!   sent, and nothing of it is logged. Removing MailCraft's data removes it.
//! - **Bounded:** at most [`MAX_VECTORS`] messages; a file is read only up to their size.
//!
//! In the browser it is kept in memory alone.

use std::collections::HashMap;
#[cfg(not(target_arch = "wasm32"))]
use std::path::{Path, PathBuf};

use mailcraft_model::insight::EMBED_DIM;
use mailcraft_model::{Id, Message};

/// The most messages the index holds.
pub const MAX_VECTORS: usize = 2_000_000;
/// The file's first bytes, with its version.
#[cfg(not(target_arch = "wasm32"))]
const MAGIC: &[u8; 8] = b"MCVECT01";
/// A record: id, fingerprint, scale, the numbers, checksum.
#[cfg(not(target_arch = "wasm32"))]
const RECORD: usize = 8 + 8 + 4 + EMBED_DIM + 8;

/// A message's numbers, a byte each, and what they were scaled by.
#[derive(Clone, Debug, PartialEq)]
struct Entry {
    fp: u64,
    scale: f32,
    q: Box<[i8]>,
}

/// What a message is read as for its meaning, as one number: its subject, who it is from, when it
/// was sent and how long its text is. Cheap: no text is read, so every message can be looked at
/// each time work is handed out. A message whose fingerprint changed is read again.
pub fn fingerprint(m: &Message) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    let mut add = |bytes: &[u8]| {
        for b in bytes {
            h = (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3);
        }
        h = (h ^ 0xff).wrapping_mul(0x0000_0100_0000_01b3);
    };
    add(m.subject.as_bytes());
    add(m.from.name.as_bytes());
    add(m.from.email.as_bytes());
    add(&m.date.to_le_bytes());
    let text = m.kept.as_deref().and_then(|k| k.text).map_or(m.body.text.len() as u64, |r| r.len);
    add(&text.to_le_bytes());
    h
}

/// The meaning index (see the module's notes).
#[derive(Debug, Default)]
pub struct Vectors {
    /// The embedding model the numbers are from; empty until the first comes.
    model: String,
    entries: HashMap<Id, Entry>,
    /// The file, when there is one, and what is still to be appended to it.
    #[cfg(not(target_arch = "wasm32"))]
    file: Option<PathBuf>,
    #[cfg(not(target_arch = "wasm32"))]
    pending: Vec<u8>,
    /// Records in the file that a later one stands for.
    #[cfg(not(target_arch = "wasm32"))]
    dead: usize,
    /// The file must be written again whole (a new model, or it was damaged).
    #[cfg(not(target_arch = "wasm32"))]
    rewrite: bool,
}

impl Vectors {
    /// An index kept in memory alone.
    pub fn in_memory() -> Vectors {
        Vectors::default()
    }

    /// The index beside the mailbox file `mailbox`: what its file holds, or an empty one when
    /// there is none or it can't be believed (it is written again whole).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn open(mailbox: &Path) -> Vectors {
        let file = path_for(mailbox);
        let mut v = Vectors { file: Some(file.clone()), ..Default::default() };
        match std::fs::read(&file) {
            Ok(bytes) => {
                if v.read(&bytes).is_err() {
                    v.entries.clear();
                    v.model.clear();
                    v.rewrite = true;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(_) => v.rewrite = true,
        }
        v
    }

    /// The file it is kept in.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn file(&self) -> Option<&Path> {
        self.file.as_deref()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn read(&mut self, bytes: &[u8]) -> Result<(), ()> {
        let head = bytes.get(..MAGIC.len() + 3).ok_or(())?;
        if head.get(..MAGIC.len()) != Some(MAGIC.as_slice()) {
            return Err(());
        }
        let dim = u16::from_le_bytes([*head.get(8).ok_or(())?, *head.get(9).ok_or(())?]);
        if usize::from(dim) != EMBED_DIM {
            return Err(());
        }
        let name_len = usize::from(*head.get(10).ok_or(())?);
        let start = 11 + name_len;
        let name = bytes.get(11..start).ok_or(())?;
        self.model = String::from_utf8(name.to_vec()).map_err(|_| ())?;
        let body = bytes.get(start..).ok_or(())?;
        // A record cut short at the end (the program stopped while writing) is left out.
        let whole = body.len() / RECORD;
        if whole > MAX_VECTORS.saturating_mul(2) {
            return Err(());
        }
        let mut records = 0usize;
        for rec in body.as_chunks::<RECORD>().0.iter().take(whole) {
            let (data, sum) = rec.split_at(RECORD - 8);
            if checksum(data).to_le_bytes() != *sum {
                return Err(());
            }
            let id = u64::from_le_bytes(data.get(0..8).and_then(|b| b.try_into().ok()).ok_or(())?);
            let fp = u64::from_le_bytes(data.get(8..16).and_then(|b| b.try_into().ok()).ok_or(())?);
            let scale = f32::from_le_bytes(data.get(16..20).and_then(|b| b.try_into().ok()).ok_or(())?);
            records += 1;
            if scale == 0.0 {
                self.entries.remove(&id);
                continue;
            }
            if !scale.is_finite() || scale < 0.0 {
                return Err(());
            }
            let q: Box<[i8]> = data.get(20..20 + EMBED_DIM).ok_or(())?.iter().map(|b| *b as i8).collect();
            if self.entries.len() >= MAX_VECTORS && !self.entries.contains_key(&id) {
                return Err(());
            }
            self.entries.insert(id, Entry { fp, scale, q });
        }
        self.dead = records.saturating_sub(self.entries.len());
        Ok(())
    }

    /// How many messages it holds.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The embedding model its numbers are from.
    pub fn model(&self) -> &str {
        &self.model
    }

    /// Whether it holds message `id` as read with fingerprint `fp`.
    pub fn current(&self, id: Id, fp: u64) -> bool {
        self.entries.get(&id).is_some_and(|e| e.fp == fp)
    }

    /// Keep `vector` (of length 1) for message `id` read as `fp`, from `model`. Numbers from
    /// another model than the ones it holds make it start again: the two can't be compared.
    pub fn insert(&mut self, id: Id, fp: u64, vector: &[f32], model: &str) {
        if vector.len() != EMBED_DIM {
            return;
        }
        if !model.is_empty() && model != self.model {
            if !self.model.is_empty() {
                self.entries.clear();
                #[cfg(not(target_arch = "wasm32"))]
                {
                    self.rewrite = true;
                    self.pending.clear();
                }
            }
            self.model = crate::ai::cut(model, 80).to_string();
        }
        if self.entries.len() >= MAX_VECTORS && !self.entries.contains_key(&id) {
            return;
        }
        let largest = vector.iter().fold(0.0f32, |m, x| m.max(x.abs()));
        if largest.is_nan() || largest <= 0.0 || !largest.is_finite() {
            return;
        }
        let q: Box<[i8]> = vector.iter().map(|x| (x / largest * 127.0).round().clamp(-127.0, 127.0) as i8).collect();
        let scale = largest / 127.0;
        let entry = Entry { fp, scale, q };
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.entries.contains_key(&id) {
                self.dead = self.dead.saturating_add(1);
            }
            self.record(id, &entry);
        }
        self.entries.insert(id, entry);
    }

    /// Take out every message `keep` says no to (gone from the mailbox). How many went.
    pub fn retain(&mut self, keep: impl Fn(Id) -> bool) -> usize {
        let gone: Vec<Id> = self.entries.keys().copied().filter(|id| !keep(*id)).collect();
        for id in &gone {
            self.entries.remove(id);
            #[cfg(not(target_arch = "wasm32"))]
            {
                self.dead = self.dead.saturating_add(2);
                self.record(*id, &Entry { fp: 0, scale: 0.0, q: vec![0; EMBED_DIM].into_boxed_slice() });
            }
        }
        gone.len()
    }

    /// How close each of `ids` it holds is to `query` (of length 1): from -1 to 1.
    pub fn scores(&self, query: &[f32], ids: impl Iterator<Item = Id>) -> Vec<(Id, f32)> {
        if query.len() != EMBED_DIM {
            return Vec::new();
        }
        ids.filter_map(|id| {
            let e = self.entries.get(&id)?;
            let dot: f32 = e.q.iter().zip(query).map(|(a, b)| f32::from(*a) * b).sum();
            Some((id, dot * e.scale))
        })
        .collect()
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn record(&mut self, id: Id, e: &Entry) {
        if self.file.is_none() {
            return;
        }
        let mut data = Vec::with_capacity(RECORD);
        data.extend_from_slice(&id.to_le_bytes());
        data.extend_from_slice(&e.fp.to_le_bytes());
        data.extend_from_slice(&e.scale.to_le_bytes());
        data.extend(e.q.iter().map(|x| *x as u8));
        let sum = checksum(&data);
        data.extend_from_slice(&sum.to_le_bytes());
        self.pending.extend_from_slice(&data);
    }

    /// Write what changed to the file: appended, or the whole again when that is due. Errors are
    /// let go: the index is made again from the mailbox when it can't be read.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn flush(&mut self) {
        let Some(file) = self.file.clone() else { return };
        let whole = self.rewrite || self.dead > self.entries.len().max(1_000);
        if whole {
            let _ = self.write_whole(&file);
            return;
        }
        if self.pending.is_empty() {
            return;
        }
        let fresh = !file.exists();
        if fresh {
            let _ = self.write_whole(&file);
            return;
        }
        use std::io::Write;
        let opened = std::fs::OpenOptions::new().append(true).open(&file);
        if let Ok(mut f) = opened
            && f.write_all(&self.pending).is_ok()
        {
            self.pending.clear();
        }
    }

    #[cfg(not(target_arch = "wasm32"))]
    fn write_whole(&mut self, file: &Path) -> std::io::Result<()> {
        let mut out = Vec::with_capacity(16 + self.entries.len() * RECORD);
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(EMBED_DIM as u16).to_le_bytes());
        let name = self.model.as_bytes();
        let name = name.get(..name.len().min(255)).unwrap_or_default();
        out.push(name.len() as u8);
        out.extend_from_slice(name);
        let saved_file = self.file.take();
        let saved_pending = std::mem::take(&mut self.pending);
        // Each entry as a record (`record` writes to `pending` only while there is a file).
        self.file = Some(file.to_path_buf());
        let mut ids: Vec<&Id> = self.entries.keys().collect();
        ids.sort();
        let entries: Vec<(Id, Entry)> = ids.into_iter().filter_map(|id| self.entries.get(id).map(|e| (*id, e.clone()))).collect();
        for (id, e) in &entries {
            self.record(*id, e);
        }
        out.append(&mut self.pending);
        self.file = saved_file;
        drop(saved_pending);
        let tmp = file.with_extension("vectors.tmp");
        write_private(&tmp, &out)?;
        std::fs::rename(&tmp, file)?;
        self.dead = 0;
        self.rewrite = false;
        Ok(())
    }

    /// Remove the file (MailCraft's data removed).
    #[cfg(not(target_arch = "wasm32"))]
    pub fn remove_file(mailbox: &Path) {
        let _ = std::fs::remove_file(path_for(mailbox));
    }
}

/// The file beside the mailbox file `mailbox`.
#[cfg(not(target_arch = "wasm32"))]
pub fn path_for(mailbox: &Path) -> PathBuf {
    let mut name = mailbox.as_os_str().to_owned();
    name.push(".vectors");
    PathBuf::from(name)
}

#[cfg(not(target_arch = "wasm32"))]
fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let mut f = options.open(path)?;
    f.write_all(bytes)?;
    f.sync_all()
}

#[cfg(not(target_arch = "wasm32"))]
fn checksum(data: &[u8]) -> u64 {
    data.iter().fold(0xcbf2_9ce4_8422_2325u64, |h, b| (h ^ u64::from(*b)).wrapping_mul(0x0000_0100_0000_01b3))
}

#[cfg(test)]
#[path = "vectors_tests.rs"]
mod vectors_tests;
