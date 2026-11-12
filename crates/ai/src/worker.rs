// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The worker process's side: read [`Request`]s a line at a time, run them on a [`Backend`], write
//! [`Response`]s. It ends when its input ends (the program quit or died) or on `quit`.

use std::io::{BufRead, Write};
use std::time::Instant;

use mailcraft_model::insight::{JobResult, Outcome};

use crate::backend::{Backend, CancelFlag};
use crate::protocol::{MAX_LINE, Request, Response, line, read};
use crate::{AiError, prompt};

/// Serve requests from `input` until it ends or says `quit`.
pub fn serve(input: impl BufRead, mut output: impl Write, backend: &mut dyn Backend) {
    let cancel = CancelFlag::new();
    let mut model_id = String::new();
    let mut config: Option<crate::ModelConfig> = None;
    let say = |out: &mut dyn Write, r: &Response| -> bool { out.write_all(line(r).as_bytes()).and_then(|_| out.flush()).is_ok() };
    if !say(&mut output, &Response::Ready { backend: crate::build_backend().into() }) {
        return;
    }
    let mut input = input;
    let mut buf = String::new();
    loop {
        buf.clear();
        // A line at most MAX_LINE long: what is longer is read and dropped.
        match std::io::Read::take(input.by_ref(), MAX_LINE as u64 + 1).read_line(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {}
        }
        let Some(request) = read::<Request>(&buf) else {
            if !say(&mut output, &Response::Error { error: "a request wasn't readable".into() }) {
                return;
            }
            continue;
        };
        let response = match request {
            Request::Quit => {
                backend.unload();
                return;
            }
            Request::Unload => {
                backend.unload();
                Response::Unloaded
            }
            Request::Load { config: asked } => match backend.load(&asked) {
                Ok(info) => {
                    model_id = info.model_id.clone();
                    config = Some(asked);
                    Response::Loaded { info }
                }
                Err(e) => Response::Error { error: e.to_string() },
            },
            Request::SelfTest => {
                let started = Instant::now();
                let (chat, embed) =
                    config.as_ref().map_or((true, false), |c| (!c.model_path.as_os_str().is_empty(), !c.embed_path.as_os_str().is_empty()));
                let chat_ok = !chat
                    || match backend.run(&prompt::self_test_job(), &cancel) {
                        Ok(ran) => prompt::self_test_passed(&ran.answer),
                        Err(_) => false,
                    };
                let embed_ok = !embed || embeddings_sound(backend, &cancel);
                Response::SelfTest { passed: chat_ok && embed_ok, millis: millis(started) }
            }
            Request::Run { job } => {
                let started = Instant::now();
                let outcome = match backend.run(&job, &cancel) {
                    Ok(ran) => Outcome::Done(ran.answer),
                    Err(AiError::Unreadable) => Outcome::Failed { reason: AiError::Unreadable.to_string() },
                    Err(e) => Outcome::Failed { reason: e.to_string() },
                };
                log::debug!("ai job {} ({}): input {} chars, {} ms", job.id, job.kind.code(), job.input.len(), millis(started));
                Response::Done {
                    result: JobResult {
                        id: job.id,
                        kind: job.kind,
                        content_hash: job.content_hash,
                        outcome,
                        model_id: model_id.clone(),
                        millis: millis(started),
                    },
                }
            }
        };
        if !say(&mut output, &response) {
            return;
        }
    }
}

/// The embedding model's self-test ([`prompt::EMBED_TEST`]).
fn embeddings_sound(backend: &mut dyn Backend, cancel: &CancelFlag) -> bool {
    let mut vectors = Vec::new();
    for text in prompt::EMBED_TEST {
        let job = mailcraft_model::insight::Job {
            id: 0,
            kind: mailcraft_model::insight::JobKind::Embed,
            priority: mailcraft_model::insight::Priority::Now,
            input: text.to_string(),
            content_hash: mailcraft_model::insight::content_hash(text),
            today: mailcraft_model::time::Date::new(2026, 11, 10).unwrap_or_default(),
        };
        match backend.run(&job, cancel) {
            Ok(ran) => vectors.push(ran.answer.vector),
            Err(_) => return false,
        }
    }
    prompt::embed_test_passed(&vectors)
}

fn millis(since: Instant) -> u32 {
    u32::try_from(since.elapsed().as_millis()).unwrap_or(u32::MAX)
}

/// The worker process's `main`: serve on standard input and output with this build's backend.
pub fn run_worker() {
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    #[cfg(all(feature = "llama", not(target_arch = "wasm32")))]
    {
        match crate::llama::LlamaBackend::new() {
            Ok(mut backend) => serve(stdin.lock(), stdout.lock(), &mut backend),
            Err(e) => {
                let _ = stdout.lock().write_all(line(&Response::Error { error: e.to_string() }).as_bytes());
            }
        }
    }
    #[cfg(not(all(feature = "llama", not(target_arch = "wasm32"))))]
    {
        let mut none = Unavailable;
        serve(stdin.lock(), stdout.lock(), &mut none);
    }
}

/// A build without llama.cpp: nothing loads.
#[cfg(not(all(feature = "llama", not(target_arch = "wasm32"))))]
struct Unavailable;

#[cfg(not(all(feature = "llama", not(target_arch = "wasm32"))))]
impl Backend for Unavailable {
    fn load(&mut self, _: &crate::ModelConfig) -> Result<crate::backend::LoadInfo, AiError> {
        Err(AiError::Unavailable)
    }
    fn unload(&mut self) {}
    fn is_loaded(&self) -> bool {
        false
    }
    fn run(&mut self, _: &mailcraft_model::insight::Job, _: &CancelFlag) -> Result<crate::backend::Ran, AiError> {
        Err(AiError::Unavailable)
    }
}
