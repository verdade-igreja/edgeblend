//! Blend engine controller shared by the panel and the tray thread. The panel's `update`
//! does not run while its window is hidden, so start/stop must not depend on it.

use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use edgeblend_config::history::Service as HistoryService;
use edgeblend_config::Config;
use edgeblend_engine::{Engine, Status};
use edgeblend_gpu::PreviewImage;

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

    /// Stop blending and persist the live config (tray Quit exits without running `App::drop`).
    pub fn shutdown(&self) {
        self.stop();
        let cfg = self.shared.lock().unwrap().clone();
        let _ = cfg.save(&self.path);
        self.history.shutdown();
    }
}
