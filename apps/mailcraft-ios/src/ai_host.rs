// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The smart Inbox's model on iPad: Apple's on-device model (Foundation Models, through
//! `mailcraft-apple-bridge`), run on a thread of its own in the app (iPadOS has no worker
//! processes). The jobs, the prompts and the checks of what comes back are the desktop's
//! (`mailcraft_ai::prompt`, `Answer::checked` in the engine); the model can't be held to a
//! grammar, so the question also says the answer's shape in words.
//!
//! Search by meaning needs an embedding model, which Apple's has not: those jobs fail, saying so.

use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use mailcraft_apple_bridge::model::{self, Availability};
use mailcraft_engine::ai::HostStatus;
use mailcraft_model::insight::{Job, JobKind, JobResult, Outcome};
use mailcraft_ui_egui::ai_host::{AiConfig, AiHost};

/// What Options shows as the model, and what `modelPath` holds when it is chosen.
pub const MODEL_ID: &str = "apple-on-device-model";
/// How long one answer may take.
const PATIENCE: Duration = Duration::from_secs(90);

#[derive(Default)]
struct State {
    results: Vec<JobResult>,
    working: usize,
    on: bool,
    last_millis: u32,
}

pub struct AppleModel {
    state: Arc<Mutex<State>>,
    jobs: Option<Sender<Job>>,
}

impl AppleModel {
    pub fn new(ctx: egui::Context) -> AppleModel {
        let state = Arc::new(Mutex::new(State::default()));
        let (tx, rx) = channel::<Job>();
        let shared = state.clone();
        let spawned = std::thread::Builder::new().name("apple model".into()).spawn(move || {
            while let Ok(job) = rx.recv() {
                let result = run(&job);
                if let Ok(mut s) = shared.lock() {
                    s.working = s.working.saturating_sub(1);
                    s.last_millis = result.millis;
                    s.results.push(result);
                }
                ctx.request_repaint();
            }
        });
        let jobs = match spawned {
            Ok(_) => Some(tx),
            Err(e) => {
                log::error!("couldn't start the model's thread: {e}");
                None
            }
        };
        AppleModel { state, jobs }
    }
}

/// One job, answered by the model and read as the desktop reads its model's answers.
fn run(job: &Job) -> JobResult {
    let started = Instant::now();
    let outcome = match job.kind {
        JobKind::Embed | JobKind::EmbedQuery => Outcome::Failed { reason: "search by meaning isn't available on iPad".into() },
        _ => {
            let question = format!("{}\n\n{}", mailcraft_ai::prompt::user_message(job), mailcraft_ai::prompt::json_shape(job.kind));
            match model::respond(mailcraft_ai::prompt::SYSTEM, &question, PATIENCE) {
                Ok(text) => match mailcraft_ai::prompt::parse(&text) {
                    Some(answer) => Outcome::Done(answer),
                    None => Outcome::Failed { reason: "the model's answer couldn't be read".into() },
                },
                // (Not the model's words: only that it failed.)
                Err(_) => Outcome::Failed { reason: "Apple's model didn't answer".into() },
            }
        }
    };
    JobResult {
        id: job.id,
        kind: job.kind,
        content_hash: job.content_hash.clone(),
        outcome,
        model_id: MODEL_ID.into(),
        millis: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    }
}

impl AiHost for AppleModel {
    fn configure(&self, config: Option<AiConfig>) {
        if let Ok(mut s) = self.state.lock() {
            s.on = config.is_some_and(|c| c.model_path == MODEL_ID);
        }
    }
    fn submit(&self, jobs: Vec<Job>) {
        let Some(tx) = &self.jobs else { return };
        for job in jobs {
            if let Ok(mut s) = self.state.lock() {
                s.working = s.working.saturating_add(1);
            }
            let _ = tx.send(job);
        }
    }
    fn take_results(&self) -> Vec<JobResult> {
        self.state.lock().map(|mut s| std::mem::take(&mut s.results)).unwrap_or_default()
    }
    fn status(&self) -> HostStatus {
        let (on, working, last_millis) = self.state.lock().map(|s| (s.on, s.working, s.last_millis)).unwrap_or_default();
        let availability = model::availability();
        let error = match availability {
            Availability::Available => String::new(),
            Availability::NotOnThisDevice => {
                "Apple's on-device model isn't ready on this iPad (Apple Intelligence off, still downloading, or not supported)".into()
            }
            Availability::NotInThisBuild => "this build of MailCraft has no Apple on-device model".into(),
        };
        let ready = availability == Availability::Available;
        HostStatus {
            available: ready,
            backend: if ready && on { "apple".into() } else { String::new() },
            model: if ready && on { "Apple on-device model".into() } else { String::new() },
            loaded: ready && on,
            error: if on { error } else { String::new() },
            self_test: String::new(),
            last_millis,
            working,
        }
    }
    fn check_model(&self, path: &str) -> Result<u64, String> {
        if path != MODEL_ID {
            return Err("on iPad, only Apple's on-device model is used".into());
        }
        match model::availability() {
            Availability::Available => Ok(0),
            Availability::NotOnThisDevice => Err("this iPad can't run Apple's on-device model now (turn on Apple Intelligence in Settings)".into()),
            Availability::NotInThisBuild => Err("this build has no Apple on-device model".into()),
        }
    }
    fn bundled_model(&self) -> Option<String> {
        Some(MODEL_ID.into())
    }
}
