// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! What runs a model ([`Backend`]): llama.cpp in a real build ([`crate::llama`]), and
//! [`FakeBackend`], which answers by fixed rules, for every test.

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use mailcraft_model::insight::{Answer, Job, JobKind};
use serde::{Deserialize, Serialize};

use crate::AiError;

/// Whether to use the graphics card.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum GpuChoice {
    /// The graphics card when the build has a backend for one and it passes the self-test.
    #[default]
    Auto,
    /// The processor only.
    Off,
}

/// Which model to load, and how.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ModelConfig {
    /// The model that sorts and summarises; empty: none (search by meaning alone).
    pub model_path: PathBuf,
    /// The embedding model for search by meaning; empty: none.
    #[serde(default)]
    pub embed_path: PathBuf,
    pub gpu: GpuChoice,
    /// The context, in tokens.
    pub ctx_tokens: u32,
    /// Processor threads (0: as many as there are cores, less one).
    pub threads: u16,
}

impl ModelConfig {
    pub fn new(model_path: PathBuf, gpu: GpuChoice) -> Self {
        ModelConfig { model_path, embed_path: PathBuf::new(), gpu, ctx_tokens: 4096, threads: 0 }
    }

    /// With an embedding model for search by meaning.
    pub fn with_embed(mut self, embed_path: PathBuf) -> Self {
        self.embed_path = embed_path;
        self
    }

    /// Whether there is anything to load.
    pub fn is_empty(&self) -> bool {
        self.model_path.as_os_str().is_empty() && self.embed_path.as_os_str().is_empty()
    }
}

/// What a load says of itself.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoadInfo {
    /// `vulkan`, `metal` or `cpu`: where it runs.
    pub backend: String,
    /// The model's name (its file's, without the extension).
    pub model_id: String,
    pub millis: u32,
}

/// What one job made: the model's answer as read from its JSON, and how long it took.
#[derive(Clone, Debug, PartialEq)]
pub struct Ran {
    pub answer: Answer,
    pub millis: u32,
    /// Tokens written.
    pub tokens: u32,
}

/// Set to stop a job between two tokens.
#[derive(Clone, Debug, Default)]
pub struct CancelFlag(Arc<AtomicBool>);

impl CancelFlag {
    pub fn new() -> Self {
        CancelFlag::default()
    }
    pub fn cancel(&self) {
        self.0.store(true, Ordering::SeqCst);
    }
    pub fn reset(&self) {
        self.0.store(false, Ordering::SeqCst);
    }
    pub fn is_cancelled(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }
}

/// Something that runs a model.
pub trait Backend: Send {
    /// Load the model (nothing to do when this one is loaded already).
    fn load(&mut self, cfg: &ModelConfig) -> Result<LoadInfo, AiError>;
    /// Let go of it (memory, the graphics card's too).
    fn unload(&mut self);
    fn is_loaded(&self) -> bool;
    /// Run one job.
    fn run(&mut self, job: &Job, cancel: &CancelFlag) -> Result<Ran, AiError>;
}

/// Answers by fixed rules, for tests: no model needed. A mail with a way to unsubscribe is
/// Promotions, one whose subject speaks of an order or a receipt is Transactions, a notification
/// Updates, the rest Primary; the summary is the subject's words. `garbage` makes it answer as a
/// faulty graphics card does (for the self-test's tests).
#[derive(Clone, Debug, Default)]
pub struct FakeBackend {
    pub loaded: Option<ModelConfig>,
    /// Answer garbage while loaded on a graphics card.
    pub garbage_on_gpu: bool,
    /// Fail every job.
    pub fail: bool,
    /// Wait this long in each job (checking the cancel flag), to test timing.
    pub delay_ms: u64,
    /// Every job's input, for tests that look at what the model was given.
    pub seen: Vec<String>,
}

impl Backend for FakeBackend {
    fn load(&mut self, cfg: &ModelConfig) -> Result<LoadInfo, AiError> {
        if cfg.is_empty() {
            return Err(AiError::NoModel);
        }
        self.loaded = Some(cfg.clone());
        let backend = if cfg.gpu == GpuChoice::Auto { "vulkan" } else { "cpu" };
        Ok(LoadInfo { backend: backend.into(), model_id: "fake".into(), millis: 1 })
    }

    fn unload(&mut self) {
        self.loaded = None;
    }

    fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    fn run(&mut self, job: &Job, cancel: &CancelFlag) -> Result<Ran, AiError> {
        let Some(cfg) = &self.loaded else { return Err(AiError::Run("not loaded".into())) };
        let gpu = cfg.gpu == GpuChoice::Auto;
        self.seen.push(crate::prompt::user_message(job));
        let mut waited = 0;
        while waited < self.delay_ms {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
            waited += 5;
        }
        if self.fail {
            return Err(AiError::Run("the fake was told to fail".into()));
        }
        if job.kind.embeds() {
            if cfg.embed_path.as_os_str().is_empty() {
                return Err(AiError::NoModel);
            }
            // A faulty card's numbers: the same whatever the words.
            let vector = if self.garbage_on_gpu && gpu { vec![0.05; mailcraft_model::insight::EMBED_DIM] } else { fake_embedding(&job.input) };
            return Ok(Ran { answer: Answer { vector, ..Default::default() }, millis: 1, tokens: 8 });
        }
        if cfg.model_path.as_os_str().is_empty() {
            return Err(AiError::NoModel);
        }
        if self.garbage_on_gpu && gpu {
            let answer = Answer {
                category: Some("updates".into()),
                summary: Some("和地区 reperc thems \u{200B}\u{200B} anthrop".into()),
                ..Default::default()
            };
            return Ok(Ran { answer, millis: 1, tokens: 8 });
        }
        Ok(Ran { answer: fake_answer(job), millis: 1, tokens: 8 })
    }
}

fn fake_answer(job: &Job) -> Answer {
    let text = job.input.to_lowercase();
    let category = if text.contains("list-unsubscribe=yes") {
        "promotions"
    } else if ["order", "receipt", "invoice", "shipped"].iter().any(|w| text.contains(w)) {
        "transactions"
    } else if text.contains("notification") || text.contains("auto-submitted=yes") {
        "updates"
    } else {
        "primary"
    };
    let subject = job.input.lines().find_map(|l| l.strip_prefix("Subject: ")).unwrap_or_default();
    let summary = (job.kind != JobKind::Classify).then(|| {
        // The self-test's mail gets the answer a sound model gives.
        if subject == "Q3 report draft" {
            "Ada asks you to review the attached Q3 report draft and send comments by Friday 14 November.".to_string()
        } else {
            format!("About: {}.", crate::prompt_cut(subject, 200))
        }
    });
    // Time-sensitive, as a sound model would say of these words in a subject.
    let urgent = ["today", "tomorrow", "urgent", "moved", "asap", "due"].iter().any(|w| subject.to_lowercase().contains(w));
    Answer {
        category: (job.kind != JobKind::SummarizeThread).then(|| category.to_string()),
        due: summary.as_ref().filter(|_| subject == "Q3 report draft").map(|_| "2026-11-14".to_string()),
        summary,
        actions: Vec::new(),
        priority: (job.kind != JobKind::SummarizeThread).then_some(urgent && category == "primary"),
        vector: Vec::new(),
    }
}

/// Words that mean the same to the fake embedding model, each group under one name: so that a
/// test can search by meaning ("flight change" finds "booking rebooked").
const FAKE_SAME: &[(&str, &[&str])] = &[
    ("travel", &["flight", "flights", "booking", "plane", "trip", "airline", "boarding"]),
    ("change", &["change", "changed", "rebooked", "moved", "rescheduled", "delayed"]),
    ("money", &["invoice", "bill", "payment", "receipt", "paid"]),
];

/// The fake embedding of `text`: each word (its group's name, when it has one) adds to one of the
/// numbers its letters choose, and the whole is made of length 1. The same text gives the same
/// numbers; texts that share words or meanings come close.
pub fn fake_embedding(text: &str) -> Vec<f32> {
    let dim = mailcraft_model::insight::EMBED_DIM;
    let mut v = vec![0.0f32; dim];
    let lower = text.to_lowercase();
    for word in lower.split(|c: char| !c.is_alphanumeric()).filter(|w| w.len() > 2) {
        let word = FAKE_SAME.iter().find(|(_, ws)| ws.contains(&word)).map_or(word, |(name, _)| name);
        let slot = word.bytes().fold(0x811c_9dc5u32, |h, b| (h ^ u32::from(b)).wrapping_mul(0x0100_0193)) as usize % dim;
        if let Some(x) = v.get_mut(slot) {
            *x += 1.0;
        }
    }
    let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if norm > 0.0 {
        v.iter_mut().for_each(|x| *x /= norm);
    } else if let Some(x) = v.first_mut() {
        *x = 1.0;
    }
    v
}
