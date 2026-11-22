//! Blend engine controller shared by the panel and the tray thread. The panel's `update`
//! does not run while its window is hidden, so start/stop must not depend on it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use eframe::egui;

use edgeblend_config::history::Service as HistoryService;
use edgeblend_config::Config;
use edgeblend_engine::{Engine, Status};
use edgeblend_gpu::PreviewImage;
use edgeblend_remote::{Change, RemoteHandle};

/// What the panel needs to show a running phone remote.
#[derive(Clone)]
pub struct RemoteInfo {
    pub url: String,
    pub urls: Vec<String>,
    pub qr: Arc<Vec<Vec<bool>>>,
}

struct Remote {
    handle: RemoteHandle,
    info: RemoteInfo,
}

pub struct Blender {
    path: PathBuf,
    /// Config version history: list / save_milestone / restore / import_text (also for a remote UI).
    pub history: Arc<HistoryService>,
    pub shared: Arc<Mutex<Config>>,
    pub status: Arc<Mutex<Status>>,
    pub preview: Arc<Mutex<Option<PreviewImage>>>,
    engine: Mutex<Option<Engine>>,
    /// Bumped on every start so the panel can drop its stale preview texture.
    generation: AtomicU64,
    /// Config edit counter shared with the phone remote (orders its poll responses).
    pub rev: Arc<AtomicU64>,
    /// Serializes blend.toml writes (panel autosave, phone edits, shutdown).
    save_lock: Mutex<()>,
    /// Phone remote server; `None` = nothing listening.
    remote: Mutex<Option<Remote>>,
    pub remote_error: Mutex<Option<String>>,
    /// Ask the panel to show the remote window (set from the tray menu).
    pub remote_window: AtomicBool,
}

impl Blender {
    pub fn new(path: PathBuf, cfg: Config) -> Arc<Self> {
        let shared = Arc::new(Mutex::new(cfg));
        let history = HistoryService::start(shared.clone(), path.clone());
        Arc::new(Self {
            path,
            history,
            shared,
            status: Arc::new(Mutex::new(Status { info: "stopped".into(), ..Default::default() })),
            preview: Arc::new(Mutex::new(None)),
            engine: Mutex::new(None),
            generation: AtomicU64::new(0),
            rev: Arc::new(AtomicU64::new(0)),
            save_lock: Mutex::new(()),
            remote: Mutex::new(None),
            remote_error: Mutex::new(None),
            remote_window: AtomicBool::new(false),
        })
    }

    pub fn start(&self) {
        let mut engine = self.engine.lock().unwrap();
        *engine = None; // stop any previous run first
        *self.preview.lock().unwrap() = None;
        self.generation.fetch_add(1, Ordering::SeqCst);
        *engine = Some(Engine::start(self.shared.clone(), self.status.clone(), self.preview.clone()));
    }

    pub fn stop(&self) {
        *self.engine.lock().unwrap() = None;
    }

    pub fn running(&self) -> bool {
        self.engine.lock().unwrap().as_ref().is_some_and(|e| e.is_running())
    }

    pub fn generation(&self) -> u64 {
        self.generation.load(Ordering::SeqCst)
    }

    /// Save the live config to blend.toml and return what was saved. Snapshots inside the
    /// save lock, so the last write always holds the newest config; the config lock itself
    /// is never held during I/O.
    pub fn persist(&self) -> anyhow::Result<Config> {
        let _guard = self.save_lock.lock().unwrap();
        let cfg = self.shared.lock().unwrap().clone();
        cfg.save(&self.path)?;
        Ok(cfg)
    }

    /// Start the phone remote if it is not running. Errors are kept in `remote_error`.
    pub fn start_remote(self: &Arc<Self>, ctx: &egui::Context) {
        let mut remote = self.remote.lock().unwrap();
        if remote.is_some() {
            return;
        }
        let weak = Arc::downgrade(self);
        let ctx = ctx.clone();
        let on_change = Arc::new(move |change: Change| {
            let Some(b) = weak.upgrade() else { return };
            match change {
                Change::Edit => {
                    if let Err(e) = b.persist() {
                        eprintln!("remote edit: save failed: {e:#}");
                    }
                }
                // Restore already saved blend.toml; outputs are bound to displays.
                Change::Restored { displays_changed } => {
                    if displays_changed && b.running() {
                        b.start();
                    }
                }
            }
            ctx.request_repaint();
        });
        let started = edgeblend_remote::start(
            self.shared.clone(),
            self.rev.clone(),
            Some(self.history.clone()),
            on_change,
            edgeblend_remote::DEFAULT_PORT,
        );
        match started {
            Ok(handle) => {
                let info = RemoteInfo {
                    qr: Arc::new(edgeblend_remote::qr_matrix(&handle.url)),
                    url: handle.url.clone(),
                    urls: handle.urls.clone(),
                };
                *remote = Some(Remote { handle, info });
                *self.remote_error.lock().unwrap() = None;
            }
            Err(e) => *self.remote_error.lock().unwrap() = Some(format!("phone remote: {e:#}")),
        }
    }

    /// Stop listening. The server thread is joined outside the lock (bounded wait).
    pub fn stop_remote(&self) {
        let remote = self.remote.lock().unwrap().take();
        if let Some(r) = remote {
            r.handle.stop();
        }
    }

    pub fn remote_info(&self) -> Option<RemoteInfo> {
        self.remote.lock().unwrap().as_ref().map(|r| r.info.clone())
    }

    /// Stop blending and the remote, persist the live config (tray Quit exits without
    /// running `App::drop`).
    pub fn shutdown(&self) {
        self.stop_remote();
        self.stop();
        let _ = self.persist();
        self.history.shutdown();
    }
}
