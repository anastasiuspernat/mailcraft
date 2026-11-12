// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The smart Inbox's model for the window ([`mailcraft_ui_egui::ai_host::AiHost`]): a
//! [`mailcraft_ai::client::Client`] that starts this program again as its worker
//! (`mailcraft --ai-worker`), so that llama.cpp failing on the graphics card ends the worker and
//! never the window.
//!
//! The worker's environment carries what llama.cpp and MoltenVK read at start, which this
//! program can't set on itself (`unsafe_code` is forbidden, and `std::env::set_var` is unsafe):
//! no half-precision maths on the graphics card (an AMD card under MoltenVK computes garbage
//! with it), the first card, and on macOS the bundle's MoltenVK when the app carries one.

use std::path::PathBuf;
use std::time::Duration;

use mailcraft_ai::client::{Client, WorkerCommand};
use mailcraft_ai::{GpuChoice, ModelConfig};
use mailcraft_engine::ai::HostStatus;
use mailcraft_model::insight::{Job, JobResult};
use mailcraft_ui_egui::ai_host::{AiConfig, AiHost};

/// The argument that makes this program the worker.
pub const WORKER_ARG: &str = "--ai-worker";

/// The worker's `main`: serve the program that started it, and end with it.
pub fn worker_main() -> ! {
    watch_parent();
    mailcraft_ai::worker::run_worker();
    std::process::exit(0)
}

/// End the worker as soon as the program that started it is gone (a Mac's quit ends the
/// process without closing the worker's input while a job runs): on Unix its parent becomes
/// another process.
fn watch_parent() {
    #[cfg(unix)]
    {
        let parent = std::os::unix::process::parent_id();
        let _ = std::thread::Builder::new().name("ai-parent".into()).spawn(move || {
            loop {
                std::thread::sleep(Duration::from_millis(250));
                if std::os::unix::process::parent_id() != parent {
                    std::process::exit(0);
                }
            }
        });
    }
}

/// The worker's command: this program, its environment for the graphics card.
fn worker_command() -> Option<WorkerCommand> {
    let program = std::env::current_exe().ok()?;
    let mut envs = vec![
        // An AMD card under MoltenVK computes garbage in half precision (plan: measured).
        ("GGML_VK_DISABLE_F16".to_string(), "1".to_string()),
        ("GGML_VK_VISIBLE_DEVICES".to_string(), "0".to_string()),
    ];
    if let Some(icd) = moltenvk_icd(&program) {
        envs.push(("VK_ICD_FILENAMES".to_string(), icd.to_string_lossy().into_owned()));
    }
    Some(WorkerCommand { program, args: vec![WORKER_ARG.to_string()], envs })
}

/// The MoltenVK driver description the Vulkan loader is pointed at on macOS: the app bundle's
/// own (`Contents/Resources/vulkan/icd.d`), else Homebrew's for a build run from source.
fn moltenvk_icd(program: &std::path::Path) -> Option<PathBuf> {
    if !cfg!(target_os = "macos") {
        return None;
    }
    let bundled = program.parent()?.parent()?.join("Resources/vulkan/icd.d/MoltenVK_icd.json");
    let candidates = [
        bundled,
        PathBuf::from("/usr/local/opt/molten-vk/etc/vulkan/icd.d/MoltenVK_icd.json"),
        PathBuf::from("/opt/homebrew/opt/molten-vk/etc/vulkan/icd.d/MoltenVK_icd.json"),
    ];
    candidates.into_iter().find(|p| p.is_file())
}

/// The window's handle on its model.
pub struct Host {
    client: Option<Client>,
    /// What the worker was built with, from the first answer; this build's until then.
    built: &'static str,
}

impl Host {
    /// A host whose worker wakes `ctx` when results are ready.
    pub fn new(ctx: egui::Context) -> Host {
        let client = worker_command().and_then(|cmd| Client::start(Box::new(cmd), Box::new(move || ctx.request_repaint())).ok());
        Host { client, built: mailcraft_ai::build_backend() }
    }
}

impl AiHost for Host {
    fn configure(&self, config: Option<AiConfig>) {
        let Some(client) = &self.client else { return };
        let (config, idle) = match config {
            Some(c) => {
                let gpu = match c.gpu {
                    mailcraft_engine::ai::GpuChoice::Auto => GpuChoice::Auto,
                    mailcraft_engine::ai::GpuChoice::Off => GpuChoice::Off,
                };
                (Some(ModelConfig::new(PathBuf::from(c.model_path), gpu).with_embed(PathBuf::from(c.embed_path))), c.idle)
            }
            None => (None, Duration::from_secs(300)),
        };
        client.configure(config, idle);
    }

    fn submit(&self, jobs: Vec<Job>) {
        if let Some(client) = &self.client {
            client.submit(jobs);
        }
    }

    fn take_results(&self) -> Vec<JobResult> {
        self.client.as_ref().map(Client::take_results).unwrap_or_default()
    }

    fn status(&self) -> HostStatus {
        let Some(client) = &self.client else {
            return HostStatus { error: "the model's worker couldn't be started".into(), ..Default::default() };
        };
        let s = client.status();
        let built = if s.built.is_empty() { self.built } else { s.built.as_str() };
        HostStatus {
            available: built != "none",
            backend: s.backend,
            model: s.model,
            loaded: s.loaded,
            error: s.error,
            self_test: s.self_test,
            last_millis: s.last_millis,
            working: s.held,
        }
    }

    fn check_model(&self, path: &str) -> Result<u64, String> {
        mailcraft_ai::check_model_file(std::path::Path::new(path)).map_err(|e| e.to_string())
    }

    fn bundled_model(&self) -> Option<String> {
        let program = std::env::current_exe().ok()?;
        bundled_model_beside(&program)
    }
}

/// The first GGUF model in the app bundle's `Contents/Resources/models` (`package.sh --bundle-model`).
fn bundled_model_beside(program: &std::path::Path) -> Option<String> {
    let dir = program.parent()?.parent()?.join("Resources/models");
    let mut models: Vec<PathBuf> =
        std::fs::read_dir(dir).ok()?.flatten().map(|e| e.path()).filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("gguf"))).collect();
    models.sort();
    models.into_iter().next().map(|p| p.to_string_lossy().into_owned())
}

impl Drop for Host {
    fn drop(&mut self) {
        // Quitting never waits long for the model (its worker is killed).
        if let Some(mut client) = self.client.take() {
            client.shutdown(Duration::from_millis(800));
        }
    }
}

#[cfg(test)]
#[path = "ai_runner_tests.rs"]
mod ai_runner_tests;
