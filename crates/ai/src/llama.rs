// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! llama.cpp through `llama-cpp-2`, in the worker process only.
//!
//! Settings that keep an AMD card under MoltenVK sound (`plan`: measured on the target machine):
//! one sequence, no unified KV cache, flash attention off, a context no larger than the job needs
//! (at most 8,192), all layers on the card. The worker's environment carries the rest
//! (`GGML_VK_DISABLE_F16`, the device, the ICD), set by the program that starts it.
//!
//! Sampling is greedy under a grammar, done the cheap way: the model's best token is checked
//! against the grammar alone, and the grammar runs over the whole vocabulary only when it refuses
//! that one (about 3 tokens a summary). Each token is accepted by the grammar exactly once: a
//! second accept makes llama.cpp throw, which ends the process.

use std::num::NonZeroU32;
use std::path::Path;
use std::time::Instant;

use llama_cpp_2::context::LlamaContext;
use llama_cpp_2::context::params::LlamaContextParams;
use llama_cpp_2::llama_backend::LlamaBackend as Llama;
use llama_cpp_2::llama_batch::LlamaBatch;
use llama_cpp_2::model::params::LlamaModelParams;
use llama_cpp_2::model::{LlamaChatMessage, LlamaChatTemplate, LlamaModel};
use llama_cpp_2::sampling::LlamaSampler;
use llama_cpp_2::token::LlamaToken;
use llama_cpp_2::token::data::LlamaTokenData;
use llama_cpp_2::token::data_array::LlamaTokenDataArray;
use mailcraft_model::insight::{EMBED_DIM, Job};

use crate::backend::{Backend, CancelFlag, GpuChoice, LoadInfo, ModelConfig, Ran};
use crate::{AiError, prompt};

/// The largest context a job gets, in tokens.
const MAX_CTX: u32 = 8_192;
/// Tokens decoded at once while reading the prompt.
const BATCH: usize = 512;

pub struct LlamaBackend {
    llama: Llama,
    loaded: Option<Loaded>,
}

struct Loaded {
    config: ModelConfig,
    /// The model that sorts and summarises, when one is chosen.
    chat: Option<(LlamaModel, LlamaChatTemplate)>,
    /// The embedding model, loaded at the first job that needs it.
    embed: Option<LlamaModel>,
}

/// Tokens of a message an embedding reads (its text was cut to about this already).
const EMBED_TOKENS: usize = 1_024;

impl LlamaBackend {
    pub fn new() -> Result<Self, AiError> {
        let mut llama = Llama::init().map_err(|e| AiError::Load(e.to_string()))?;
        // llama.cpp's own log lines say nothing of the mail, but the worker's output is the
        // protocol's and its errors go nowhere: keep them quiet.
        llama.void_logs();
        Ok(LlamaBackend { llama, loaded: None })
    }

    /// A model file, on the graphics card or not.
    fn model_file(&self, path: &Path, gpu: bool) -> Result<LlamaModel, AiError> {
        // (llama-cpp-2 asserts the file exists in a debug build: say so first.)
        crate::check_model_file(path)?;
        let params = LlamaModelParams::default().with_n_gpu_layers(if gpu { 999 } else { 0 });
        LlamaModel::load_from_file(&self.llama, path, &params).map_err(|e| AiError::Load(e.to_string()))
    }

    /// An embedding of the job's text ([`prompt::embed_text`]): the model's pooled numbers, the
    /// first [`EMBED_DIM`] of them, of length 1.
    fn embed(&mut self, job: &Job, cancel: &CancelFlag) -> Result<Ran, AiError> {
        let started = Instant::now();
        let loaded = self.loaded.as_ref().ok_or_else(|| AiError::Run("no model is loaded".into()))?;
        if loaded.config.embed_path.as_os_str().is_empty() {
            return Err(AiError::NoModel);
        }
        if loaded.embed.is_none() {
            let gpu = loaded.config.gpu == GpuChoice::Auto && Self::gpu_backend().is_some();
            let model = self.model_file(&loaded.config.embed_path, gpu)?;
            if let Some(l) = self.loaded.as_mut() {
                l.embed = Some(model);
            }
        }
        let loaded = self.loaded.as_ref().ok_or_else(|| AiError::Run("no model is loaded".into()))?;
        let model = loaded.embed.as_ref().ok_or(AiError::NoModel)?;
        let text = prompt::embed_text(job);
        let mut tokens = model.vocab().tokenize(text.as_bytes(), true, false);
        tokens.truncate(EMBED_TOKENS);
        if tokens.is_empty() {
            return Err(AiError::Run("nothing to embed".into()));
        }
        let n = u32::try_from(tokens.len()).unwrap_or(u32::MAX);
        let threads = threads(loaded.config.threads);
        let params = LlamaContextParams::default()
            .with_embeddings(true)
            .with_n_ctx(NonZeroU32::new(n.max(512)))
            .with_n_batch(n.max(512))
            .with_n_ubatch(n.max(512))
            .with_n_seq_max(1)
            .with_kv_unified(false)
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            .with_flash_attention_policy(llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_DISABLED);
        let mut ctx = model.new_context(&self.llama, params).map_err(|e| AiError::Run(e.to_string()))?;
        if cancel.is_cancelled() {
            return Err(AiError::Cancelled);
        }
        let mut batch = LlamaBatch::new(tokens.len(), 1);
        for (i, tok) in tokens.iter().enumerate() {
            batch.add(*tok, i32::try_from(i).unwrap_or(i32::MAX), &[0], true).map_err(|e| AiError::Run(e.to_string()))?;
        }
        ctx.decode(&mut batch).map_err(|e| AiError::Run(e.to_string()))?;
        let all = ctx.embeddings_seq_ith(0).map_err(|e| AiError::Run(e.to_string()))?;
        let mut vector: Vec<f32> = all.iter().take(EMBED_DIM).copied().collect();
        if vector.len() < EMBED_DIM {
            return Err(AiError::BadModel(format!("it makes {} numbers, fewer than {EMBED_DIM}", vector.len())));
        }
        let norm = vector.iter().map(|x| x * x).sum::<f32>().sqrt();
        if norm.is_nan() || norm <= 1e-6 || !norm.is_finite() {
            return Err(AiError::Run("the embedding was empty".into()));
        }
        vector.iter_mut().for_each(|x| *x /= norm);
        let answer = mailcraft_model::insight::Answer { vector, ..Default::default() };
        Ok(Ran { answer, millis: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX), tokens: n })
    }

    fn gpu_backend() -> Option<&'static str> {
        match crate::build_backend() {
            "vulkan" => Some("vulkan"),
            "metal" => Some("metal"),
            _ => None,
        }
    }
}

impl Backend for LlamaBackend {
    fn load(&mut self, cfg: &ModelConfig) -> Result<LoadInfo, AiError> {
        if self.loaded.as_ref().is_some_and(|l| l.config == *cfg) {
            let backend = if cfg.gpu == GpuChoice::Auto { Self::gpu_backend().unwrap_or("cpu") } else { "cpu" };
            return Ok(LoadInfo { backend: backend.into(), model_id: model_id(&cfg.model_path), millis: 0 });
        }
        self.unload();
        if cfg.is_empty() {
            return Err(AiError::NoModel);
        }
        let started = Instant::now();
        let gpu = if cfg.gpu == GpuChoice::Auto { Self::gpu_backend() } else { None };
        let chat = if cfg.model_path.as_os_str().is_empty() {
            None
        } else {
            let model = self.model_file(&cfg.model_path, gpu.is_some())?;
            let template = model.chat_template(None).map_err(|e| AiError::BadModel(format!("it has no chat template ({e})")))?;
            Some((model, template))
        };
        if !cfg.embed_path.as_os_str().is_empty() {
            crate::check_model_file(&cfg.embed_path)?;
        }
        self.loaded = Some(Loaded { config: cfg.clone(), chat, embed: None });
        let millis = u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX);
        let named = if cfg.model_path.as_os_str().is_empty() { &cfg.embed_path } else { &cfg.model_path };
        Ok(LoadInfo { backend: gpu.unwrap_or("cpu").into(), model_id: model_id(named), millis })
    }

    fn unload(&mut self) {
        self.loaded = None;
    }

    fn is_loaded(&self) -> bool {
        self.loaded.is_some()
    }

    fn run(&mut self, job: &Job, cancel: &CancelFlag) -> Result<Ran, AiError> {
        if job.kind.embeds() {
            return self.embed(job, cancel);
        }
        let loaded = self.loaded.as_ref().ok_or_else(|| AiError::Run("no model is loaded".into()))?;
        let (model, template) = loaded.chat.as_ref().ok_or(AiError::NoModel)?;
        let started = Instant::now();
        let chat = [
            LlamaChatMessage::new("system".into(), prompt::SYSTEM.into()).map_err(|e| AiError::Run(e.to_string()))?,
            LlamaChatMessage::new("user".into(), prompt::user_message(job)).map_err(|e| AiError::Run(e.to_string()))?,
        ];
        let text = model.apply_chat_template(template, &chat, true).map_err(|e| AiError::Run(e.to_string()))?;
        let vocab = model.vocab();
        let tokens = vocab.tokenize(text.as_bytes(), false, true);
        let max_new = prompt::max_tokens(job.kind);
        let need = u32::try_from(tokens.len().saturating_add(max_new).saturating_add(32)).unwrap_or(u32::MAX);
        if need > MAX_CTX {
            return Err(AiError::Run("the message is too long for the model".into()));
        }
        let threads = threads(loaded.config.threads);
        let params = LlamaContextParams::default()
            .with_n_ctx(NonZeroU32::new(need.max(1_024)))
            .with_n_batch(BATCH as u32)
            .with_n_ubatch(BATCH as u32)
            .with_n_seq_max(1)
            .with_kv_unified(false)
            .with_n_threads(threads)
            .with_n_threads_batch(threads)
            .with_flash_attention_policy(llama_cpp_sys_2::LLAMA_FLASH_ATTN_TYPE_DISABLED);
        let mut ctx = model.new_context(&self.llama, params).map_err(|e| AiError::Run(e.to_string()))?;
        // Read the prompt, a batch at a time; only its last token's logits are wanted.
        let mut batch = LlamaBatch::new(BATCH, 1);
        let last = tokens.len().saturating_sub(1);
        for (chunk_no, chunk) in tokens.chunks(BATCH).enumerate() {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            batch.clear();
            for (i, tok) in chunk.iter().enumerate() {
                let pos = chunk_no * BATCH + i;
                batch.add(*tok, pos as i32, &[0], pos == last).map_err(|e| AiError::Run(e.to_string()))?;
            }
            ctx.decode(&mut batch).map_err(|e| AiError::Run(e.to_string()))?;
        }
        let mut grammar = LlamaSampler::grammar(model, &prompt::grammar(job.kind), "root").map_err(|e| AiError::Run(e.to_string()))?;
        let mut out: Vec<u8> = Vec::new();
        let mut idx = batch.n_tokens() - 1;
        let mut written = 0u32;
        let start = i32::try_from(tokens.len()).unwrap_or(i32::MAX);
        for pos in (start..).take(max_new) {
            if cancel.is_cancelled() {
                return Err(AiError::Cancelled);
            }
            let tok = pick(&ctx, idx, &mut grammar);
            if vocab.is_eog(tok) {
                break;
            }
            out.extend(vocab.token_to_piece(tok, true, None));
            written += 1;
            batch.clear();
            batch.add(tok, pos, &[0], true).map_err(|e| AiError::Run(e.to_string()))?;
            idx = 0;
            ctx.decode(&mut batch).map_err(|e| AiError::Run(e.to_string()))?;
        }
        let text = String::from_utf8_lossy(&out);
        let answer = prompt::parse(&text).ok_or(AiError::Unreadable)?;
        Ok(Ran { answer, millis: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX), tokens: written })
    }
}

/// The model's best token at output `idx` that the grammar allows, accepted by the grammar.
fn pick(ctx: &LlamaContext<'_>, idx: i32, grammar: &mut LlamaSampler) -> LlamaToken {
    let logits = ctx.get_logits_ith(idx);
    let (best, logit) = logits.iter().enumerate().fold((0usize, f32::MIN), |b, (i, v)| if *v > b.1 { (i, *v) } else { b });
    let candidate = LlamaToken(i32::try_from(best).unwrap_or(0));
    let mut one = LlamaTokenDataArray::new(vec![LlamaTokenData::new(candidate, logit, 0.0)], false);
    grammar.apply(&mut one);
    let tok = if one.data.first().is_some_and(|d| d.logit().is_finite()) {
        candidate
    } else {
        let mut all = ctx.token_data_array_ith(idx);
        grammar.apply(&mut all);
        all.data.iter().filter(|d| d.logit().is_finite()).fold((candidate, f32::MIN), |b, d| if d.logit() > b.1 { (d.id(), d.logit()) } else { b }).0
    };
    grammar.accept(tok);
    tok
}

fn threads(asked: u16) -> i32 {
    if asked > 0 {
        return i32::from(asked);
    }
    let cores = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4);
    i32::try_from(cores.saturating_sub(1).max(1)).unwrap_or(4)
}

fn model_id(path: &Path) -> String {
    path.file_stem().map(|s| s.to_string_lossy().to_lowercase()).unwrap_or_default()
}
