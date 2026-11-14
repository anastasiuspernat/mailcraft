// SPDX-FileCopyrightText: MailCraft contributors
// SPDX-License-Identifier: MIT OR Apache-2.0
//! MailCraft on iPad.
//!
//! Runs the same [`mailcraft_ui_egui::MailApp`] as the desktop app, through eframe's native runner
//! on iPadOS (winit's UIKit backend, wgpu on Metal). Built by `packaging/ios/build.sh` (the
//! simulator, or `--device`), and on GitHub with Xcode 26 for TestFlight (`.github/workflows/ios.yml`).
//!
//! The mailbox and the window's settings are kept in the app's own Application Support folder;
//! Send/Receive, the account checks and the keychain are the desktop's (`mailcraft-net`, the
//! desktop's `secrets.rs`); signing in shows the provider's page in Apple's sign-in sheet
//! (`mailcraft-apple-bridge`, the desktop's `sign_in.rs`); reminders and new mail go through
//! iPadOS's notifications (`notifications.rs`); the smart Inbox runs on Apple's on-device model
//! (`ai_host.rs`).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]

#[cfg(target_os = "ios")]
fn main() {
    logging::start();
    if let Err(e) = ios::run() {
        log::error!("MailCraft couldn't start: {e}");
    }
}

/// The app's log: `logs/mailcraft.log` beside the mailbox (the one before is kept as
/// `mailcraft.1.log`), with MailCraft's own records from `info` up and the rest from `warn`, and
/// every panic. The console of Xcode or `simctl launch --console-pty` gets them too.
#[cfg(target_os = "ios")]
mod logging {
    use std::io::Write as _;
    use std::sync::Mutex;

    struct FileLog(Mutex<Option<std::fs::File>>);

    static LOG: FileLog = FileLog(Mutex::new(None));

    impl log::Log for FileLog {
        fn enabled(&self, m: &log::Metadata<'_>) -> bool {
            let ours = m.target().starts_with("mailcraft");
            m.level() <= if ours { log::Level::Info } else { log::Level::Warn }
        }
        fn log(&self, r: &log::Record<'_>) {
            if !self.enabled(r.metadata()) {
                return;
            }
            let line =
                format!("{} {} [{}] {}: {}", mailcraft_time::now(), r.level(), std::thread::current().name().unwrap_or("-"), r.target(), r.args());
            eprintln!("{line}");
            if let Ok(mut file) = self.0.lock()
                && let Some(f) = file.as_mut()
            {
                let _ = writeln!(f, "{line}");
            }
        }
        fn flush(&self) {}
    }

    pub fn start() {
        if let Some(dir) = crate::ios::data_dir().map(|d| d.join("logs"))
            && mailcraft_engine::store::create_private_dir_all(&dir).is_ok()
        {
            let path = dir.join("mailcraft.log");
            let _ = std::fs::rename(&path, dir.join("mailcraft.1.log"));
            if let Ok(f) = std::fs::File::create(&path)
                && let Ok(mut slot) = LOG.0.lock()
            {
                *slot = Some(f);
            }
        }
        let _ = log::set_logger(&LOG).map(|()| log::set_max_level(log::LevelFilter::Info));
        let before = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            log::error!("panic: {info}");
            before(info);
        }));
        log::info!("MailCraft {} for iPad", env!("CARGO_PKG_VERSION"));
    }
}

#[cfg(not(target_os = "ios"))]
fn main() {
    eprintln!("mailcraft-ios only runs on iPadOS: build it with packaging/ios/build.sh");
}

#[cfg(target_os = "ios")]
mod ai_host;
#[cfg(target_os = "ios")]
mod notifications;
/// The iPad's keychain, through the same module as the desktop's credential store.
#[cfg(target_os = "ios")]
#[path = "../../mailcraft/src/secrets.rs"]
mod secrets;
/// Signing in on the provider's page, through the same module as the desktop's: here the page
/// shows in Apple's sign-in sheet over MailCraft.
#[cfg(target_os = "ios")]
#[path = "../../mailcraft/src/sign_in.rs"]
mod sign_in;

#[cfg(target_os = "ios")]
mod ios {
    use std::path::{Path, PathBuf};

    use mailcraft_engine::Session;
    use mailcraft_model::Mailbox;
    use mailcraft_ui_egui::{MailApp, Services, UiState};

    /// The window, with where its settings are kept and what was last written there.
    struct App {
        app: MailApp,
        prefs_path: Option<PathBuf>,
        last_prefs: Vec<u8>,
        looked: f64,
        /// The trackpad's recognizers are on the window's view.
        trackpad: bool,
    }

    /// How often the settings are compared with what was written (seconds): iPadOS may stop the
    /// app at any time without telling it, so nothing waits for a quit.
    const PREFS_EVERY: f64 = 2.0;

    /// The thread that writes the mailbox file, as on the desktop.
    fn saver(ctx: &egui::Context, path: &Path) -> Option<Box<dyn mailcraft_engine::io::BackgroundSave>> {
        use mailcraft_engine::io::BackgroundSave as _;
        let wake = ctx.clone();
        match mailcraft_engine::store::Saver::spawn(move || wake.request_repaint()) {
            Ok(saver) => {
                saver.follow(path);
                Some(Box::new(saver))
            }
            Err(e) => {
                log::error!("couldn't start the mailbox saver: {e}");
                None
            }
        }
    }

    impl eframe::App for App {
        fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
            self.app.logic(ctx);
            // The window's settings, written when they changed.
            let now = ctx.input(|i| i.time);
            if now - self.looked >= PREFS_EVERY {
                self.looked = now;
                if let Some(path) = &self.prefs_path
                    && let Ok(bytes) = serde_json::to_vec_pretty(&self.app.prefs())
                    && bytes != self.last_prefs
                {
                    match mailcraft_engine::store::write_atomic(path, &bytes) {
                        Ok(()) => self.last_prefs = bytes,
                        Err(e) => log::warn!("the window's settings couldn't be saved: {e}"),
                    }
                }
            }
            // (An idle window draws nothing, so it looks again by itself.)
            ctx.request_repaint_after(std::time::Duration::from_secs_f64(PREFS_EVERY));
        }
        /// The trackpad's and mouse's pointer and scrolling (`mailcraft_apple_bridge::trackpad`),
        /// as egui's own events, in its points (the interface size scales them).
        fn raw_input_hook(&mut self, ctx: &egui::Context, raw: &mut egui::RawInput) {
            use mailcraft_apple_bridge::trackpad::Pointer;
            let zoom = f64::from(ctx.zoom_factor()).max(0.1);
            let at = |x: f64, y: f64| egui::pos2((x / zoom) as f32, (y / zoom) as f32);
            let modifiers = ctx.input(|i| i.modifiers);
            for p in mailcraft_apple_bridge::trackpad::take() {
                let wheel = |delta: egui::Vec2, phase| egui::Event::MouseWheel {
                    unit: egui::MouseWheelUnit::Point,
                    delta,
                    phase,
                    source: egui::MouseWheelSource::Trackpad,
                    modifiers,
                };
                match p {
                    Pointer::Moved { x, y } => raw.events.push(egui::Event::PointerMoved(at(x, y))),
                    Pointer::Gone => raw.events.push(egui::Event::PointerGone),
                    Pointer::ScrollStart { x, y } => {
                        raw.events.push(egui::Event::PointerMoved(at(x, y)));
                        raw.events.push(wheel(egui::Vec2::ZERO, egui::TouchPhase::Start));
                    }
                    Pointer::Scroll { x, y, dx, dy } => {
                        raw.events.push(egui::Event::PointerMoved(at(x, y)));
                        raw.events.push(wheel(egui::vec2((dx / zoom) as f32, (dy / zoom) as f32), egui::TouchPhase::Move));
                    }
                    Pointer::ScrollEnd => raw.events.push(wheel(egui::Vec2::ZERO, egui::TouchPhase::End)),
                }
            }
        }

        fn ui(&mut self, ui: &mut egui::Ui, frame: &mut eframe::Frame) {
            // The trackpad's scrolling and pointer, which the window library doesn't pass on.
            if !self.trackpad {
                mailcraft_apple_bridge::trackpad::install(frame);
                self.trackpad = true;
            }
            // The window keeps out from under the status bar and the home indicator: laid out in
            // the safe area, with the window's own colour behind both.
            let canvas = mailcraft_ui_egui::theme::Tokens::get(ui.ctx()).canvas;
            ui.painter().rect_filled(ui.ctx().viewport_rect(), 0.0, canvas);
            let safe = ui.ctx().content_rect();
            ui.scope_builder(egui::UiBuilder::new().max_rect(safe), |ui| self.app.ui(ui));
        }
    }

    /// Where MailCraft keeps its mailbox and its settings on the iPad: the app's own Application
    /// Support folder, private to it (`$HOME` is the app's container).
    pub(crate) fn data_dir() -> Option<PathBuf> {
        std::env::var_os("HOME").map(|home| PathBuf::from(home).join("Library").join("Application Support").join("MailCraft"))
    }

    /// The mailbox to open and the file it is kept in. The first time, the sample of three
    /// accounts is written there, to have something to read until accounts are added. A file
    /// that can't be read is never written over: the window starts empty and keeps nothing.
    fn first_mailbox(path: &Path) -> (Mailbox, Option<PathBuf>) {
        if path.exists() {
            return match mailcraft_engine::store::open_for_window(path, &mailcraft_engine::io::OpenProgress::default()) {
                Ok(opened) => (opened.mailbox, Some(path.to_path_buf())),
                Err(e) => {
                    log::error!("the mailbox couldn't be read, so nothing is saved over it: {e}");
                    (Mailbox::with_local_account("", ""), None)
                }
            };
        }
        let now = mailcraft_time::now();
        let mailbox = mailcraft_engine::sample::several_accounts(now, mailcraft_time::local_offset_minutes(now));
        if let Some(dir) = path.parent()
            && let Err(e) = mailcraft_engine::store::create_private_dir_all(dir)
        {
            log::error!("couldn't make the folder for the mailbox: {e}");
            return (mailbox, None);
        }
        match mailcraft_engine::store::save(path, &mailbox) {
            Ok(_) => (mailbox, Some(path.to_path_buf())),
            Err(e) => {
                log::error!("couldn't write the first mailbox: {e}");
                (mailbox, None)
            }
        }
    }

    /// Run `work` on a thread of its own; the answer comes back through the channel and wakes the
    /// window. A dropped sender (the thread couldn't start) reads as "stopped unexpectedly".
    fn background<T: Send + 'static>(ctx: &egui::Context, name: &str, work: impl FnOnce() -> T + Send + 'static) -> std::sync::mpsc::Receiver<T> {
        let (tx, rx) = std::sync::mpsc::channel();
        let wake = ctx.clone();
        let spawned = std::thread::Builder::new().name(name.into()).spawn(move || {
            let _ = tx.send(work());
            wake.request_repaint();
        });
        if let Err(e) = spawned {
            log::error!("couldn't start {name}: {e}");
        }
        rx
    }

    /// What the window can ask of the iPad: Send/Receive, the check of a new account and the
    /// look-up of its provider, the keychain, and notifications. (Signing in with Microsoft or
    /// Google, and the rest the desktop host offers, come later.)
    fn services(ctx: egui::Context) -> Services {
        let (sync_ctx, mx_ctx, test_ctx, sign_in_ctx, ai_ctx) = (ctx.clone(), ctx.clone(), ctx.clone(), ctx.clone(), ctx);
        let sync_cancel = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let cancel = sync_cancel.clone();
        Services {
            // As on the desktop: mail arrives in pieces as the run goes, through a short channel.
            sync: Some(Box::new(move |request| {
                use std::sync::atomic::Ordering;
                let (tx, rx) = std::sync::mpsc::sync_channel(8);
                let (wake, cancel) = (sync_ctx.clone(), cancel.clone());
                cancel.store(false, Ordering::Relaxed);
                let spawned = std::thread::Builder::new().name("send/receive".into()).spawn(move || {
                    mailcraft_net::sync_with(&request, &cancel, &mut |piece| {
                        if tx.send(piece).is_err() {
                            cancel.store(true, Ordering::Relaxed);
                        }
                        wake.request_repaint();
                    });
                });
                if let Err(e) = spawned {
                    log::error!("couldn't start Send/Receive: {e}");
                }
                rx
            })),
            sync_cancel,
            lookup_mx: Some(Box::new(move |domain| {
                background(&mx_ctx, "mail exchangers", move || mailcraft_net::lookup_mx(&domain).map_err(|e| e.to_string()))
            })),
            test_account: Some(Box::new(move |account, secret| {
                background(&test_ctx, "account check", move || mailcraft_net::test_account(&account, &secret).map_err(|e| e.to_string()))
            })),
            // The provider's page in Apple's sign-in sheet: MailCraft stays in front, so the
            // sign-in comes back to its loopback address as on the desktop; the sheet goes once it
            // has, and closing it calls the wait off.
            sign_in: Some(Box::new(move |request| {
                let ctx = sign_in_ctx.clone();
                background(&sign_in_ctx, "sign-in", move || {
                    let cancel = request.cancel.clone();
                    let open = |url: &str| {
                        let cancel = cancel.clone();
                        mailcraft_apple_bridge::auth_sheet::open(url, move || cancel.store(true, std::sync::atomic::Ordering::Relaxed))
                    };
                    let tokens = crate::sign_in::sign_in_with(request, &open);
                    mailcraft_apple_bridge::auth_sheet::close();
                    ctx.request_repaint();
                    tokens
                })
            })),
            microsoft_client_id: crate::sign_in::microsoft_client_id(),
            google_client_id: crate::sign_in::google_client_id(),
            google_client_secret: crate::sign_in::google_client_secret(),
            secrets: crate::secrets::store(),
            alert: Some(Box::new(crate::notifications::Notifications::default())),
            // The smart Inbox with Apple's on-device model.
            ai: Some(Box::new(crate::ai_host::AppleModel::new(ai_ctx))),
            ..Default::default()
        }
    }

    /// The window's settings kept beside the mailbox, if they can be read.
    fn read_prefs(path: &Path) -> Option<UiState> {
        let bytes = std::fs::read(path).ok()?;
        serde_json::from_slice(&bytes).map_err(|e| log::warn!("the window's settings couldn't be read: {e}")).ok()
    }

    pub fn run() -> Result<(), String> {
        // The device is asked for what its adapter offers, not wgpu's defaults: the simulator's
        // Metal falls short of them (15 inter-stage shader variables where 16 are asked).
        let mut setup = eframe::egui_wgpu::WgpuSetupCreateNew::without_display_handle();
        setup.device_descriptor = std::sync::Arc::new(|adapter: &eframe::wgpu::Adapter| {
            let offered = adapter.limits();
            eframe::wgpu::DeviceDescriptor {
                label: Some("MailCraft"),
                required_limits: eframe::wgpu::Limits { max_texture_dimension_2d: offered.max_texture_dimension_2d.min(8192), ..offered },
                ..Default::default()
            }
        });
        let wgpu_options = eframe::egui_wgpu::WgpuConfiguration { wgpu_setup: eframe::egui_wgpu::WgpuSetup::CreateNew(setup), ..Default::default() };
        let options = eframe::NativeOptions { renderer: eframe::Renderer::Wgpu, wgpu_options, ..Default::default() };
        eframe::run_native(
            "MailCraft",
            options,
            Box::new(|cc| {
                let dir = data_dir();
                let (mailbox, path) = match &dir {
                    Some(dir) => first_mailbox(&dir.join("mailbox.mailcraft")),
                    None => (Mailbox::with_local_account("", ""), None),
                };
                let kept = path.is_some();
                let mut session = Session::new(mailbox);
                session.path = path;
                session.index_words = true;
                let services = services(cc.egui_ctx.clone());
                let mut app = MailApp::new(session, services);
                app.autosave = kept;
                if let Some(path) = app.session.path.clone() {
                    app.services.saver = saver(&cc.egui_ctx, &path);
                }
                let prefs_path = dir.filter(|_| kept).map(|d| d.join("ui.json"));
                match prefs_path.as_deref().and_then(read_prefs) {
                    Some(ui) => app.apply_prefs(ui),
                    None => {
                        app.fresh_prefs();
                        // A first start on iPad: a little larger than the desktop's 100 %, for
                        // fingers (File › Options › General changes it).
                        if let Err(e) = app.execute("ui.interfaceSize", serde_json::json!({"value": 110})) {
                            log::warn!("the first interface size wasn't set: {e}");
                        }
                    }
                }
                app.index_words_later();
                // (None written yet: the first look writes them.)
                let last = if prefs_path.as_deref().is_some_and(Path::exists) {
                    serde_json::to_vec_pretty(&app.prefs()).unwrap_or_default()
                } else {
                    Vec::new()
                };
                Ok(Box::new(App { app, prefs_path, last_prefs: last, looked: 0.0, trackpad: false }))
            }),
        )
        .map_err(|e| e.to_string())
    }
}
