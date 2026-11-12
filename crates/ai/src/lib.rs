// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! MailCraft's model on this computer, for the smart Inbox and summaries.
//!
//! The engine says what a model is to read (`ai.request`, [`mailcraft_model::insight::Job`]); this
//! crate runs it and hands back what came of it ([`mailcraft_model::insight::JobResult`]), which
//! the engine checks and keeps (`ai.apply`). Only the apps depend on it (`cargo xtask layers`):
//! the engine and the window build for the browser without it.
//!
//! The model runs in a **worker process** (the desktop app started again with `--ai-worker`),
//! never in the window's: llama.cpp ends the process it runs in when the graphics card fails, and
//! the mail program must not go with it. [`client::Client`] starts the worker when there is work,
//! talks to it over its standard input and output ([`protocol`]), stops it when it has been idle
//! a while (which frees the graphics card's memory) and when the program quits, and starts it
//! again on the processor when the graphics card's self-test fails or the worker dies.
//!
//! Mail is untrusted and so is what a model writes: the prompt says the mail is data
//! ([`prompt`]), a grammar holds the answer to a few fields, and the engine checks every field.
//! Prompts and answers are never logged; only lengths and timings are.
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

pub mod backend;
pub mod client;
#[cfg(all(feature = "llama", not(target_arch = "wasm32")))]
pub mod llama;
pub mod prompt;
pub mod protocol;
pub mod worker;

pub use backend::{Backend, CancelFlag, FakeBackend, GpuChoice, ModelConfig};

/// What went wrong, in English: shown on hover and in `ai.status`, never the model's own words.
#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
pub enum AiError {
    #[error("no model is chosen")]
    NoModel,
    #[error("the model file can't be used: {0}")]
    BadModel(String),
    #[error("the model couldn't be loaded: {0}")]
    Load(String),
    #[error("the model failed: {0}")]
    Run(String),
    #[error("the model's answer wasn't readable")]
    Unreadable,
    #[error("stopped")]
    Cancelled,
    #[error("this build has no model engine")]
    Unavailable,
    #[error("the model's worker stopped: {0}")]
    Worker(String),
}

/// Which backend this build runs a model on: `vulkan`, `metal`, `cpu`, or `none` (built without
/// llama.cpp).
pub fn build_backend() -> &'static str {
    if cfg!(feature = "vulkan") {
        "vulkan"
    } else if cfg!(feature = "metal") {
        "metal"
    } else if cfg!(feature = "llama") {
        "cpu"
    } else {
        "none"
    }
}

/// The first `max` characters of `text`, at a character boundary.
pub(crate) fn prompt_cut(text: &str, max: usize) -> &str {
    text.char_indices().nth(max).and_then(|(at, _)| text.get(..at)).unwrap_or(text)
}

/// The first bytes of every GGUF file.
pub const GGUF_MAGIC: &[u8; 4] = b"GGUF";

/// Whether `path` is a readable GGUF file, and its size: the Options page's check of a chosen
/// model file.
pub fn check_model_file(path: &std::path::Path) -> Result<u64, AiError> {
    use std::io::Read;
    let meta = std::fs::metadata(path).map_err(|e| AiError::BadModel(e.kind().to_string()))?;
    if !meta.is_file() {
        return Err(AiError::BadModel("it isn't a file".into()));
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(path).and_then(|mut f| f.read_exact(&mut magic)).map_err(|e| AiError::BadModel(e.kind().to_string()))?;
    if &magic != GGUF_MAGIC {
        return Err(AiError::BadModel("it isn't a GGUF model".into()));
    }
    Ok(meta.len())
}

#[cfg(test)]
#[path = "tests.rs"]
mod tests;
