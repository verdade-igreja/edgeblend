//! `--watchdog`: re-run ourselves and restart on crash. A clean exit (code 0, e.g. tray
//! Quit) ends the watchdog too.

use anyhow::{Context, Result};
use std::os::windows::process::CommandExt;
use std::process::Command;
use std::time::{Duration, Instant};

use crate::log;

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const MIN_DELAY: Duration = Duration::from_secs(2);
const MAX_DELAY: Duration = Duration::from_secs(60);
const STABLE_AFTER: Duration = Duration::from_secs(5 * 60);

pub fn run(child_args: &[String]) -> Result<()> {
    let exe = std::env::current_exe().context("locating own exe")?;
    let mut delay = MIN_DELAY;
    log::write(&format!("watchdog started: {} {}", exe.display(), child_args.join(" ")));
    loop {
        let started = Instant::now();
        let status = Command::new(&exe).args(child_args).creation_flags(CREATE_NO_WINDOW).status();
        match status {
            Ok(s) if s.success() => {
                log::write("watchdog: child exited normally; stopping");
                return Ok(());
            }
            Ok(s) => log::write(&format!("watchdog: child exited with {s}")),
            Err(e) => log::write(&format!("watchdog: could not start child: {e}")),
        }
        if started.elapsed() > STABLE_AFTER {
            delay = MIN_DELAY;
        }
        log::write(&format!("watchdog: restarting in {}s", delay.as_secs()));
        std::thread::sleep(delay);
        delay = (delay * 2).min(MAX_DELAY);
    }
}
