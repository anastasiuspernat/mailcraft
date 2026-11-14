// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The window's side of the smart Inbox: each frame it hands the host's model the next jobs
//! (`ai.request`) while the host holds few, takes in what came back (`ai.apply`), asks for the
//! summary of a long message as it is opened, and keeps `ai.status` up to date. The host itself
//! (the desktop app's worker process) is [`AiHost`]; the browser has none.

use std::time::{Duration, Instant};

use mailcraft_engine::ai::{GpuChoice, HostStatus};
use mailcraft_model::Id;
use mailcraft_model::insight::{Job, JobResult};
use serde_json::json;

use crate::MailApp;

/// The host's model, as the window sees it.
pub trait AiHost {
    /// The model file and settings to use; `None` stops it.
    fn configure(&self, config: Option<AiConfig>);
    /// Hand jobs over.
    fn submit(&self, jobs: Vec<Job>);
    /// What came back since the last call.
    fn take_results(&self) -> Vec<JobResult>;
    /// What the host says of itself; `held` is the jobs it holds.
    fn status(&self) -> HostStatus;
    /// Whether a file is a model it can load, and its size in bytes.
    fn check_model(&self, path: &str) -> Result<u64, String>;
    /// A model that came with the program (the app bundle's `Resources/models`), if any.
    fn bundled_model(&self) -> Option<String> {
        None
    }
    /// A model that is part of the system rather than a file (Apple's on-device model on iPad):
    /// what `modelPath` names it by. Options then shows no file, graphics card or unloading, and
    /// the switch alone turns it on.
    fn fixed_model(&self) -> Option<String> {
        None
    }
    /// Whether this host reads mail for search by meaning (an embedding model). Apple's
    /// on-device model has none.
    fn embeds(&self) -> bool {
        true
    }
}

/// The settings a host needs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AiConfig {
    /// The smart Inbox's model; empty while it is off.
    pub model_path: String,
    /// Search by meaning's embedding model; empty while it is off.
    pub embed_path: String,
    pub gpu: GpuChoice,
    pub idle: Duration,
}

/// Jobs handed out at once, and the most the host holds before more are asked for.
const BATCH: u64 = 4;
const HOLD: usize = 2;
/// How often the mailbox is looked through for work when nothing else woke it.
const LOOK_EVERY: Duration = Duration::from_secs(2);

/// What the window remembers between frames.
#[derive(Default)]
pub struct AiTicks {
    configured: Option<Option<AiConfig>>,
    last_look: Option<Instant>,
    /// The revision of the mailbox at the last look that found nothing to do.
    idle_at: Option<u64>,
    /// The message last opened, so it is asked about once.
    opened: Option<Id>,
    /// The mailbox the jobs out were handed for.
    opening: u64,
}

pub(crate) fn tick(app: &mut MailApp, ctx: &egui::Context) {
    let Some(host) = app.services.ai.as_ref() else { return };
    let prefs = app.session.prefs.ai.clone();
    // The host follows the settings.
    let inbox = !prefs.model_path.is_empty() && prefs.enabled;
    let meaning = mailcraft_engine::ai::searches_by_meaning(&app.session);
    let config = (inbox || meaning).then(|| AiConfig {
        model_path: if inbox { prefs.model_path.clone() } else { String::new() },
        embed_path: if meaning { prefs.search_model_path.clone() } else { String::new() },
        gpu: prefs.gpu,
        idle: Duration::from_secs(u64::from(prefs.idle_unload_minutes.max(1)) * 60),
    });
    if app.ai.configured.is_none() {
        // The first frame: the Inbox opens on Primary when the smart Inbox is on.
        mailcraft_engine::ai::opening_view(&mut app.session);
    }
    if app.ai.configured.as_ref() != Some(&config) {
        host.configure(config.clone());
        app.ai.configured = Some(config);
        app.ai.idle_at = None;
        mailcraft_engine::ai::host_restarted(&mut app.session);
    }
    // Another mailbox: what was out was for the one before.
    let opening = app.session.openings();
    if app.ai.opening != opening {
        app.ai.opening = opening;
        let _ = host.take_results();
        mailcraft_engine::ai::host_restarted(&mut app.session);
        mailcraft_engine::ai::opening_view(&mut app.session);
        app.ai.idle_at = None;
    }
    // Results first.
    let results = host.take_results();
    if !results.is_empty() {
        if let Err(e) = app.session.run("ai.apply", &json!({"results": results})) {
            log::warn!("ai: results not taken in: {e}");
        }
        app.ai.idle_at = None;
        ctx.request_repaint();
    }
    let mut status = host.status();
    // A message opened: its summary first.
    let focus = app.session.sel.focus();
    if focus != app.ai.opened {
        app.ai.opened = focus;
        if let Some(id) = focus
            && mailcraft_engine::ai::opened(&mut app.session, id)
        {
            app.ai.idle_at = None;
        }
    }
    // A search's meaning, asked for as it is typed.
    if mailcraft_engine::ai::ask_meaning(&mut app.session) {
        app.ai.idle_at = None;
    }
    // More work, while the host holds little.
    let asked = !app.session.ai.on_demand.is_empty() || app.session.ai.query_wanted.is_some();
    let due = app.ai.last_look.is_none_or(|t| t.elapsed() >= LOOK_EVERY) && app.ai.idle_at != Some(app.session.rev());
    if status.available && status.working < HOLD && (asked || ((prefs.enabled || meaning) && due)) {
        app.ai.last_look = Some(Instant::now());
        match app.session.run("ai.request", &json!({"max": BATCH})) {
            Ok(answer) => {
                let jobs: Vec<Job> = serde_json::from_value(answer["jobs"].clone()).unwrap_or_default();
                if jobs.is_empty() {
                    app.ai.idle_at = Some(app.session.rev());
                } else {
                    status.working += jobs.len();
                    host.submit(jobs);
                }
            }
            Err(e) => log::warn!("ai: no work handed out: {e}"),
        }
    }
    let busy = status.working > 0 || !app.session.ai.on_demand.is_empty();
    app.session.ai.host = status;
    if busy {
        ctx.request_repaint_after(Duration::from_millis(250));
    } else if prefs.enabled || meaning {
        ctx.request_repaint_after(LOOK_EVERY);
    }
}
