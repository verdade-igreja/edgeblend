//! Standalone remote server for developing the phone page without the Windows app.
//!
//! ```text
//! cargo run -p edgeblend-remote --example serve [port] [config.toml]
//! ```
//! Uses a scratch config (default: `<temp>/edgeblend-remote-dev/blend.toml`, with history
//! next to it) and prints the magic link.

use edgeblend_config::history::Service as History;
use edgeblend_config::Config;
use std::path::PathBuf;
use std::sync::atomic::AtomicU64;
use std::sync::{Arc, Mutex};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args().skip(1);
    let port = args.next().and_then(|s| s.parse().ok()).unwrap_or(edgeblend_remote::DEFAULT_PORT);
    let path = args
        .next()
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("edgeblend-remote-dev").join("blend.toml"));
    let cfg = Arc::new(Mutex::new(Config::load_or_default(&path)));
    let history = History::start(cfg.clone(), path.clone());

    let (cfg_t, path_t) = (cfg.clone(), path.clone());
    let on_change = Arc::new(move |change| {
        let snapshot = cfg_t.lock().unwrap().clone();
        if let Err(e) = snapshot.save(&path_t) {
            eprintln!("save failed: {e:#}");
        }
        println!("{change:?}: pattern={:?} mute_mask={:#b}", snapshot.pattern, snapshot.mute_mask);
    });
    let handle = edgeblend_remote::start(cfg, Arc::new(AtomicU64::new(0)), Some(history), on_change, port)?;
    println!("config: {}", path.display());
    for u in &handle.urls {
        println!("url:    {u}");
    }
    println!("local:  http://127.0.0.1:{}/?t={}", handle.port, handle.url.rsplit("t=").next().unwrap_or(""));
    println!("Ctrl+C to stop.");
    loop {
        std::thread::park();
    }
}
