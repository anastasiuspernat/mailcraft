// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! The program's side: a thread of its own (`ai`) that holds the jobs handed to it, starts the
//! worker process when there is work ([`Spawn`]), runs one job at a time (what the owner asked
//! for first), and hands the results back ([`Client::take_results`]), waking the window.
//!
//! - **Self-test:** each time the model is loaded on a graphics card, a fixed message with a
//!   known answer is run first. A wrong answer (a driver that computes garbage) or a worker that
//!   dies there makes it start again on the processor, and says so in [`Status::self_test`].
//! - **Idle:** after [`Client::configure`]'s idle time without a job, the worker is stopped, which
//!   frees everything it held (the graphics card's memory too).
//! - **A worker that dies** fails the job it had; three deaths in a row stop the work until the
//!   settings change, so a model that can't run doesn't start over and over.
//! - **Quit:** [`Client::shutdown`] stops the worker (it is killed, not waited for) and the thread,
//!   within the time given.

use std::collections::VecDeque;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use mailcraft_model::insight::{Job, JobResult, Outcome, Priority};

use crate::backend::{Backend, GpuChoice, ModelConfig};
use crate::protocol::{MAX_LINE, Request, Response, line, read};
use crate::{AiError, worker};

/// How long a job may take before its worker is taken for stuck.
pub const JOB_TIMEOUT: Duration = Duration::from_secs(180);
/// How long a load may take (the first on a graphics card compiles its programs).
pub const LOAD_TIMEOUT: Duration = Duration::from_secs(300);
/// Worker deaths in a row after which the work stops until the settings change.
pub const MAX_CRASHES: u32 = 3;
/// Most jobs held.
pub const MAX_HELD: usize = 256;

/// A worker process, or a stand-in for one.
pub trait Worker: Send {
    fn send(&mut self, request: &Request) -> Result<(), AiError>;
    fn recv(&mut self, timeout: Duration) -> Result<Response, AiError>;
    /// Stop it now.
    fn kill(&mut self);
}

/// What starts a worker.
pub trait Spawn: Send {
    fn spawn(&self) -> Result<Box<dyn Worker>, AiError>;
}

/// The worker as a process: this program again with its own arguments and environment.
#[derive(Clone, Debug, Default)]
pub struct WorkerCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    /// Set in the worker's environment only (the graphics settings llama.cpp and MoltenVK read).
    pub envs: Vec<(String, String)>,
}

impl Spawn for WorkerCommand {
    fn spawn(&self) -> Result<Box<dyn Worker>, AiError> {
        let mut cmd = Command::new(&self.program);
        cmd.args(&self.args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null());
        for (k, v) in &self.envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| AiError::Worker(e.kind().to_string()))?;
        let stdin = child.stdin.take().ok_or_else(|| AiError::Worker("no input".into()))?;
        let stdout = child.stdout.take().ok_or_else(|| AiError::Worker("no output".into()))?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("ai-worker-out".into())
            .spawn(move || read_lines(BufReader::new(stdout), tx))
            .map_err(|e| AiError::Worker(e.to_string()))?;
        Ok(Box::new(ProcessWorker { child, stdin, lines: rx }))
    }
}

/// Lines of `out` into `tx` until it ends.
fn read_lines(mut out: impl BufRead, tx: Sender<String>) {
    let mut buf = String::new();
    loop {
        buf.clear();
        match std::io::Read::take(out.by_ref(), MAX_LINE as u64 + 1).read_line(&mut buf) {
            Ok(0) | Err(_) => return,
            Ok(_) => {
                if tx.send(std::mem::take(&mut buf)).is_err() {
                    return;
                }
            }
        }
    }
}

struct ProcessWorker {
    child: Child,
    stdin: ChildStdin,
    lines: Receiver<String>,
}

impl Worker for ProcessWorker {
    fn send(&mut self, request: &Request) -> Result<(), AiError> {
        self.stdin.write_all(line(request).as_bytes()).and_then(|_| self.stdin.flush()).map_err(|e| AiError::Worker(e.kind().to_string()))
    }
    fn recv(&mut self, timeout: Duration) -> Result<Response, AiError> {
        recv_line(&self.lines, timeout)
    }
    fn kill(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

impl Drop for ProcessWorker {
    fn drop(&mut self) {
        self.kill();
    }
}

fn recv_line(lines: &Receiver<String>, timeout: Duration) -> Result<Response, AiError> {
    let deadline = Instant::now() + timeout;
    loop {
        let left = deadline.saturating_duration_since(Instant::now());
        match lines.recv_timeout(left) {
            Ok(l) => match read::<Response>(&l) {
                Some(r) => return Ok(r),
                // Not ours (something a library printed): read on.
                None => continue,
            },
            Err(RecvTimeoutError::Timeout) => return Err(AiError::Worker("it didn't answer in time".into())),
            Err(RecvTimeoutError::Disconnected) => return Err(AiError::Worker("it ended".into())),
        }
    }
}

/// A worker that runs [`worker::serve`] on a thread of this process, over pipes, with any
/// [`Backend`]: for tests, and for a platform that runs its model in-process.
pub struct InProcess<F: Fn() -> Box<dyn Backend> + Send + 'static>(pub F);

impl<F: Fn() -> Box<dyn Backend> + Send + 'static> Spawn for InProcess<F> {
    fn spawn(&self) -> Result<Box<dyn Worker>, AiError> {
        let (req_r, req_w) = std::io::pipe().map_err(|e| AiError::Worker(e.to_string()))?;
        let (resp_r, resp_w) = std::io::pipe().map_err(|e| AiError::Worker(e.to_string()))?;
        let mut backend = (self.0)();
        std::thread::Builder::new()
            .name("ai-in-process".into())
            .spawn(move || worker::serve(BufReader::new(req_r), resp_w, backend.as_mut()))
            .map_err(|e| AiError::Worker(e.to_string()))?;
        let (tx, rx) = mpsc::channel();
        std::thread::Builder::new()
            .name("ai-in-process-out".into())
            .spawn(move || read_lines(BufReader::new(resp_r), tx))
            .map_err(|e| AiError::Worker(e.to_string()))?;
        Ok(Box::new(PipeWorker { input: Some(req_w), lines: rx }))
    }
}

struct PipeWorker {
    input: Option<std::io::PipeWriter>,
    lines: Receiver<String>,
}

impl Worker for PipeWorker {
    fn send(&mut self, request: &Request) -> Result<(), AiError> {
        let input = self.input.as_mut().ok_or_else(|| AiError::Worker("stopped".into()))?;
        input.write_all(line(request).as_bytes()).and_then(|_| input.flush()).map_err(|e| AiError::Worker(e.to_string()))
    }
    fn recv(&mut self, timeout: Duration) -> Result<Response, AiError> {
        recv_line(&self.lines, timeout)
    }
    fn kill(&mut self) {
        // Its input ends: the serving thread returns.
        self.input = None;
    }
}

/// What the client says of itself (`ai.status`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Status {
    /// The backend the worker was built with (`none` for a build without a model engine).
    pub built: String,
    /// Where the model runs now (`vulkan`, `metal`, `cpu`), while it is loaded.
    pub backend: String,
    pub model: String,
    pub loaded: bool,
    pub error: String,
    pub self_test: String,
    pub last_millis: u32,
    /// Jobs held, the one running included.
    pub held: usize,
    /// Stopped after [`MAX_CRASHES`] worker deaths in a row, until the settings change.
    pub stopped: bool,
}

enum Cmd {
    Configure(Option<ModelConfig>, Duration),
    Submit(Vec<Job>),
    Quit,
}

#[derive(Default)]
struct Shared {
    results: Vec<JobResult>,
    status: Status,
}

/// The program's handle on its model.
pub struct Client {
    tx: Sender<Cmd>,
    shared: Arc<Mutex<Shared>>,
    thread: Option<JoinHandle<()>>,
}

impl Client {
    /// Start the client's thread; no worker runs until there is work and a model is configured.
    /// `wake` is called when results are ready (the window repaints).
    pub fn start(spawn: Box<dyn Spawn>, wake: Box<dyn Fn() + Send>) -> Result<Client, AiError> {
        let (tx, rx) = mpsc::channel();
        let shared = Arc::new(Mutex::new(Shared::default()));
        let thread_shared = shared.clone();
        let thread = std::thread::Builder::new()
            .name("ai".into())
            .spawn(move || Runner::new(spawn, rx, thread_shared, wake).run())
            .map_err(|e| AiError::Worker(e.to_string()))?;
        Ok(Client { tx, shared, thread: Some(thread) })
    }

    /// The model to use (`None`: none; the worker stops), and the idle time after which it stops.
    pub fn configure(&self, config: Option<ModelConfig>, idle: Duration) {
        let _ = self.tx.send(Cmd::Configure(config, idle));
    }

    /// Hand jobs over; those the owner asked for go before the rest.
    pub fn submit(&self, jobs: Vec<Job>) {
        if !jobs.is_empty() {
            let _ = self.tx.send(Cmd::Submit(jobs));
        }
    }

    /// The results ready since the last call.
    pub fn take_results(&self) -> Vec<JobResult> {
        self.shared.lock().map(|mut s| std::mem::take(&mut s.results)).unwrap_or_default()
    }

    pub fn status(&self) -> Status {
        self.shared.lock().map(|s| s.status.clone()).unwrap_or_default()
    }

    /// Stop the worker and the thread, waiting at most `within` for the thread.
    pub fn shutdown(&mut self, within: Duration) {
        let _ = self.tx.send(Cmd::Quit);
        let Some(thread) = self.thread.take() else { return };
        let deadline = Instant::now() + within;
        while !thread.is_finished() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        if thread.is_finished() {
            let _ = thread.join();
        }
        // Else it is left to end by itself: its worker is killed at its next step.
    }
}

impl Drop for Client {
    fn drop(&mut self) {
        self.shutdown(Duration::from_millis(500));
    }
}

/// The client's thread.
struct Runner {
    spawn: Box<dyn Spawn>,
    rx: Receiver<Cmd>,
    shared: Arc<Mutex<Shared>>,
    wake: Box<dyn Fn() + Send>,
    config: Option<ModelConfig>,
    idle: Duration,
    queue: VecDeque<Job>,
    worker: Option<Box<dyn Worker>>,
    loaded: bool,
    /// The graphics card failed its self-test or the worker died on it: the processor from now on.
    gpu_failed: bool,
    tested: bool,
    crashes: u32,
    last_work: Instant,
}

impl Runner {
    fn new(spawn: Box<dyn Spawn>, rx: Receiver<Cmd>, shared: Arc<Mutex<Shared>>, wake: Box<dyn Fn() + Send>) -> Self {
        Runner {
            spawn,
            rx,
            shared,
            wake,
            config: None,
            idle: Duration::from_secs(300),
            queue: VecDeque::new(),
            worker: None,
            loaded: false,
            gpu_failed: false,
            tested: false,
            crashes: 0,
            last_work: Instant::now(),
        }
    }

    fn status(&self, f: impl FnOnce(&mut Status)) {
        if let Ok(mut s) = self.shared.lock() {
            f(&mut s.status);
            s.status.held = self.queue.len();
        }
    }

    fn run(mut self) {
        loop {
            let wait = if self.queue.is_empty() || self.config.is_none() || self.stopped() { Duration::from_millis(500) } else { Duration::ZERO };
            match self.rx.recv_timeout(wait) {
                Ok(cmd) => {
                    if !self.take(cmd) {
                        break;
                    }
                    // Take every waiting command before working.
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => break,
                Err(RecvTimeoutError::Timeout) => {}
            }
            if self.queue.is_empty() || self.config.is_none() || self.stopped() {
                if self.worker.is_some() && self.last_work.elapsed() >= self.idle {
                    self.stop_worker();
                }
                continue;
            }
            self.work_one();
        }
        self.stop_worker();
    }

    fn stopped(&self) -> bool {
        self.crashes >= MAX_CRASHES
    }

    /// Whether to go on.
    fn take(&mut self, cmd: Cmd) -> bool {
        match cmd {
            Cmd::Quit => return false,
            Cmd::Configure(config, idle) => {
                self.idle = idle;
                if config != self.config {
                    self.stop_worker();
                    self.config = config;
                    self.gpu_failed = false;
                    self.tested = false;
                    self.crashes = 0;
                    self.status(|s| {
                        s.error.clear();
                        s.self_test.clear();
                        s.stopped = false;
                    });
                }
            }
            Cmd::Submit(jobs) => {
                for job in jobs {
                    if self.queue.iter().any(|q| q.id == job.id && q.kind == job.kind) {
                        continue;
                    }
                    if job.priority == Priority::Now {
                        // In front of everything but earlier asks.
                        let at = self.queue.iter().position(|q| q.priority != Priority::Now).unwrap_or(self.queue.len());
                        self.queue.insert(at, job);
                    } else if self.queue.len() < MAX_HELD {
                        self.queue.push_back(job);
                    }
                }
                self.status(|_| {});
            }
        }
        true
    }

    fn config_now(&self) -> Option<ModelConfig> {
        let mut c = self.config.clone()?;
        if self.gpu_failed {
            c.gpu = GpuChoice::Off;
        }
        Some(c)
    }

    /// Start the worker and load the model, testing the graphics card once. Errors fail the
    /// next job.
    fn ready(&mut self) -> Result<(), AiError> {
        let config = self.config_now().ok_or(AiError::NoModel)?;
        if self.worker.is_none() {
            let mut w = self.spawn.spawn()?;
            match w.recv(Duration::from_secs(30))? {
                Response::Ready { backend } => self.status(|s| s.built = backend),
                Response::Error { error } => return Err(AiError::Worker(error)),
                _ => return Err(AiError::Worker("it said something unexpected".into())),
            }
            self.worker = Some(w);
            self.loaded = false;
        }
        if !self.loaded {
            let w = self.worker.as_mut().ok_or_else(|| AiError::Worker("gone".into()))?;
            w.send(&Request::Load { config: config.clone() })?;
            match w.recv(LOAD_TIMEOUT)? {
                Response::Loaded { info } => {
                    self.loaded = true;
                    let on_gpu = info.backend != "cpu";
                    self.status(|s| {
                        s.backend = info.backend.clone();
                        s.model = info.model_id.clone();
                        s.loaded = true;
                    });
                    if on_gpu && !self.tested {
                        self.self_test()?;
                    }
                }
                Response::Error { error } => return Err(AiError::Load(error)),
                _ => return Err(AiError::Worker("it said something unexpected".into())),
            }
        }
        Ok(())
    }

    /// The graphics card's self-test; on failure, start again on the processor.
    fn self_test(&mut self) -> Result<(), AiError> {
        let w = self.worker.as_mut().ok_or_else(|| AiError::Worker("gone".into()))?;
        let answer = w.send(&Request::SelfTest).and_then(|_| w.recv(JOB_TIMEOUT));
        self.tested = true;
        match answer {
            Ok(Response::SelfTest { passed: true, millis }) => {
                self.status(|s| s.self_test = format!("passed ({millis} ms)"));
                Ok(())
            }
            other => {
                let why = match other {
                    Ok(Response::SelfTest { .. }) => "wrong answer".to_string(),
                    Ok(_) => "unexpected answer".to_string(),
                    Err(e) => e.to_string(),
                };
                log::warn!("ai: the graphics card failed the self-test ({why}); using the processor");
                self.gpu_failed = true;
                self.stop_worker();
                self.status(|s| s.self_test = format!("failed: {why}; using the processor"));
                // Start again on the processor (no self-test there).
                self.ready()
            }
        }
    }

    fn work_one(&mut self) {
        if let Err(e) = self.ready() {
            self.died(e);
            return;
        }
        let Some(job) = self.queue.pop_front() else { return };
        let Some(w) = self.worker.as_mut() else { return };
        let started = Instant::now();
        let answer = w.send(&Request::Run { job: job.clone() }).and_then(|_| w.recv(JOB_TIMEOUT));
        self.last_work = Instant::now();
        let result = match answer {
            Ok(Response::Done { result }) => {
                self.crashes = 0;
                result
            }
            Ok(_) => fail(&job, "the model's worker answered something unexpected", started),
            Err(e) => {
                let r = fail(&job, &e.to_string(), started);
                self.died(e);
                r
            }
        };
        let millis = result.millis;
        if let Ok(mut s) = self.shared.lock() {
            s.results.push(result);
            s.status.last_millis = millis;
            s.status.held = self.queue.len();
        }
        (self.wake)();
    }

    /// The worker failed (or couldn't start): stop it, and after a death on the graphics card
    /// go on with the processor.
    fn died(&mut self, e: AiError) {
        let on_gpu = self.config_now().is_some_and(|c| c.gpu == GpuChoice::Auto) && self.loaded;
        self.stop_worker();
        self.crashes += 1;
        if on_gpu {
            self.gpu_failed = true;
        }
        log::warn!("ai: {e}");
        let stopped = self.stopped();
        if stopped {
            // Fail what is held, so the window says so instead of waiting.
            let held: Vec<Job> = self.queue.drain(..).collect();
            if let Ok(mut s) = self.shared.lock() {
                for job in held {
                    s.results.push(fail(&job, "the model keeps failing; check File › Options › AI", Instant::now()));
                }
            }
            (self.wake)();
        }
        self.status(|s| {
            s.error = e.to_string();
            s.stopped = stopped;
        });
    }

    fn stop_worker(&mut self) {
        if let Some(mut w) = self.worker.take() {
            let _ = w.send(&Request::Quit);
            w.kill();
        }
        self.loaded = false;
        self.status(|s| {
            s.loaded = false;
            s.backend.clear();
        });
    }
}

fn fail(job: &Job, reason: &str, started: Instant) -> JobResult {
    JobResult {
        id: job.id,
        kind: job.kind,
        content_hash: job.content_hash.clone(),
        outcome: Outcome::Failed { reason: reason.to_string() },
        model_id: String::new(),
        millis: u32::try_from(started.elapsed().as_millis()).unwrap_or(u32::MAX),
    }
}
