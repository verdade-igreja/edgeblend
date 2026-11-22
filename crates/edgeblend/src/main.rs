#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod instance;
mod log;
mod watchdog;

use anyhow::Result;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use windows::Win32::System::Console::{AttachConsole, ATTACH_PARENT_PROCESS};
use windows::Win32::UI::HiDpi::*;

use edgeblend_config::Config;
use edgeblend_gpu::display;

const USAGE: &str = "edgeblend [config.toml] [--start] [--headless] [--list] [--watchdog]
  (no flags)  open the control panel
  --start     open the control panel and start blending immediately
  --headless  blend without UI, using the config file (Ctrl+Alt+Q quits)
  --list      list displays
  --watchdog  restart automatically after a crash (exits when the app is quit normally)

The panel has a tray icon; closing the window hides it there. Log: %APPDATA%\\edgeblend\\edgeblend.log";

fn main() {
    // Release builds have no console of their own; borrow the parent's so --list/--help print.
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
    log::install_panic_hook();
    if let Err(e) = real_main() {
        let msg = format!("fatal: {e:#}");
        log::write(&msg);
        eprintln!("{msg}");
        std::process::exit(1);
    }
}

fn real_main() -> Result<()> {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    let args: Vec<String> = std::env::args().skip(1).collect();
    let has = |f: &str| args.iter().any(|a| a == f);

    if has("--help") || has("-h") {
        println!("{USAGE}");
        return Ok(());
    }
    if has("--list") {
        for d in display::enumerate()? {
            println!(
                "{:<14} {:>5}x{:<5} at ({:>5},{:>5})  on {}",
                d.name, d.width(), d.height(), d.rect.left, d.rect.top, d.adapter_name
            );
        }
        return Ok(());
    }

    if has("--watchdog") {
        let _guard = match instance::acquire(instance::WATCHDOG) {
            Some(g) => g,
            None => return Ok(()), // already supervised
        };
        let child_args: Vec<String> = args.iter().filter(|a| *a != "--watchdog").cloned().collect();
        return watchdog::run(&child_args);
    }

    let _guard = match instance::acquire(instance::PANEL) {
        Some(g) => g,
        None => {
            edgeblend_ui::show_existing_panel();
            return Ok(());
        }
    };

    let cfg_path = args
        .iter()
        .find(|a| !a.starts_with("--"))
        .map(PathBuf::from)
        .unwrap_or_else(Config::default_path);
    println!("config: {}", cfg_path.display());

    if has("--headless") {
        let cfg = Arc::new(Mutex::new(Config::load_or_default(&cfg_path)));
        let status = Mutex::new(edgeblend_engine::Status::default());
        let preview = Mutex::new(None);
        println!("blending; Ctrl+Alt+T test pattern, Ctrl+Alt+C cursor, Ctrl+Alt+Q quit");
        return edgeblend_engine::run(&cfg, &status, &AtomicBool::new(false), &preview);
    }

    edgeblend_ui::run(cfg_path, has("--start"))
}
