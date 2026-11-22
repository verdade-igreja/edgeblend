//! Immutable config version history.
//!
//! One file per version in `<config dir>/history/`, named `YYYYMMDD-HHMMSS-mmm.toml` (UTC).
//! A file is written once (temp file, then a no-overwrite link/rename) and never modified
//! or deleted by this code. Each file holds the full config plus a `[meta]` table.
//!
//! [`History`] is the plain file store. [`Service`] adds the background "historian" that
//! records automatic versions of the shared live config, plus `restore`.

use anyhow::{anyhow, bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::Config;

pub const MAX_NAME_CHARS: usize = 80;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Kind {
    Milestone,
    #[serde(other)]
    Auto,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    Desktop,
    Remote,
    Restore,
    Import,
    #[serde(other)]
    Auto,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct Meta {
    created: String,
    created_ms: i64,
    kind: Kind,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    name: Option<String>,
    source: Source,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Entry {
    /// File stem; the handle to pass to `load` / `restore`.
    pub id: String,
    /// RFC 3339 UTC, e.g. `2026-11-22T15:30:12.123Z`.
    pub created: String,
    pub created_ms: i64,
    pub kind: Kind,
    pub name: Option<String>,
    pub source: Source,
    /// Set when the file could not be parsed; such entries cannot be loaded.
    pub broken: Option<String>,
}

pub struct History {
    dir: PathBuf,
}

static TMP_COUNTER: AtomicU64 = AtomicU64::new(0);

impl History {
    /// Folder next to the config file: `<config dir>/history`.
    pub fn dir_for(config_path: &Path) -> PathBuf {
        config_path.parent().filter(|p| !p.as_os_str().is_empty()).unwrap_or(Path::new(".")).join("history")
    }

    /// Does not touch the disk; the folder is created on the first write.
    pub fn open(dir: impl Into<PathBuf>) -> Self {
        Self { dir: dir.into() }
    }

    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// Newest first. Unreadable files are listed with `broken` set; never panics.
    pub fn list(&self) -> Vec<Entry> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return Vec::new() };
        let mut ids: Vec<String> = rd
            .flatten()
            .filter_map(|e| {
                let name = e.file_name().into_string().ok()?;
                let id = name.strip_suffix(".toml")?.to_string();
                valid_id(&id).then_some(id)
            })
            .collect();
        ids.sort_unstable_by(|a, b| b.cmp(a));
        ids.into_iter().map(|id| self.entry(id)).collect()
    }

    fn entry(&self, id: String) -> Entry {
        match self.read(&id) {
            Ok((meta, _)) => Entry {
                id,
                created: meta.created,
                created_ms: meta.created_ms,
                kind: meta.kind,
                name: meta.name,
                source: meta.source,
                broken: None,
            },
            Err(e) => {
                let ms = parse_id_ms(&id).unwrap_or(0);
                Entry {
                    created: format_rfc3339(ms),
                    created_ms: ms,
                    id,
                    kind: Kind::Auto,
                    name: None,
                    source: Source::Auto,
                    broken: Some(format!("{e:#}")),
                }
            }
        }
    }

    fn read(&self, id: &str) -> Result<(Meta, Config)> {
        ensure!(valid_id(id), "invalid history id {id:?}");
        let path = self.dir.join(format!("{id}.toml"));
        let text = std::fs::read_to_string(&path).with_context(|| format!("reading {}", path.display()))?;
        let table: toml::Table = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        let meta: Meta = table
            .get("meta")
            .cloned()
            .ok_or_else(|| anyhow!("missing [meta]"))?
            .try_into()
            .context("bad [meta]")?;
        let mut cfg: Config = toml::Value::Table(table).try_into().context("bad config")?;
        cfg.ensure_geometry();
        Ok((meta, cfg))
    }

    pub fn load(&self, id: &str) -> Result<Config> {
        self.read(id).map(|(_, c)| c)
    }

    /// Config of the newest version that parses.
    pub fn newest_config(&self) -> Option<Config> {
        self.list().into_iter().find(|e| e.broken.is_none()).and_then(|e| self.load(&e.id).ok())
    }

    /// Writes a version unless it equals the newest one. Returns the new id, if written.
    pub fn record_auto(&self, cfg: &Config, source: Source) -> Result<Option<String>> {
        self.record_auto_at(cfg, source, now_ms())
    }

    /// Always writes, even if identical to the newest version.
    pub fn save_milestone(&self, cfg: &Config, name: Option<String>, source: Source) -> Result<String> {
        self.write(cfg, Kind::Milestone, sanitize_name(name.as_deref().unwrap_or("")), source, now_ms())
    }

    pub(crate) fn record_auto_at(&self, cfg: &Config, source: Source, ms: i64) -> Result<Option<String>> {
        if self.newest_config().is_some_and(|n| same_config(&n, cfg)) {
            return Ok(None);
        }
        self.write(cfg, Kind::Auto, None, source, ms).map(Some)
    }

    pub(crate) fn write(&self, cfg: &Config, kind: Kind, name: Option<String>, source: Source, ms: i64) -> Result<String> {
        std::fs::create_dir_all(&self.dir).with_context(|| format!("creating {}", self.dir.display()))?;
        let meta = Meta { created: format_rfc3339(ms), created_ms: ms, kind, name, source };
        let mut table = toml::Table::try_from(cfg)?;
        table.insert("meta".into(), toml::Value::try_from(&meta)?);
        let text = format!("# edgeblend config version (written once, never modified)\n{}", toml::to_string_pretty(&table)?);

        let tmp = self.dir.join(format!(
            ".tmp-{}-{}",
            std::process::id(),
            TMP_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::write(&tmp, text).with_context(|| format!("writing {}", tmp.display()))?;
        let base = format_id(ms);
        let result = (|| {
            for n in 0..10_000u32 {
                let id = if n == 0 { base.clone() } else { format!("{base}-{n}") };
                let dest = self.dir.join(format!("{id}.toml"));
                match std::fs::hard_link(&tmp, &dest) {
                    Ok(()) => return Ok(id),
                    Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                    Err(_) => {
                        // Filesystem without hard links: fall back to exclusive create.
                        use std::io::Write;
                        let body = std::fs::read(&tmp)?;
                        match std::fs::OpenOptions::new().write(true).create_new(true).open(&dest) {
                            Ok(mut f) => {
                                f.write_all(&body)?;
                                return Ok(id);
                            }
                            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                            Err(e) => return Err(e.into()),
                        }
                    }
                }
            }
            bail!("could not find a free history file name")
        })();
        let _ = std::fs::remove_file(&tmp);
        result
    }
}

fn same_config(a: &Config, b: &Config) -> bool {
    a == b
}

/// Ids are file stems we generate: ASCII alphanumerics and `-`, never a path.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && id.len() <= 48 && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
}

/// Trim, drop control and path characters, cap the length. `None` if nothing is left.
pub fn sanitize_name(raw: &str) -> Option<String> {
    let s: String = raw
        .chars()
        .filter(|c| !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|'))
        .collect();
    let s: String = s.trim().chars().take(MAX_NAME_CHARS).collect();
    let s = s.trim().to_string();
    (!s.is_empty()).then_some(s)
}

pub fn now_ms() -> i64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

fn civil_from_ms(ms: i64) -> (i64, u32, u32, u32, u32, u32, u32) {
    let secs = ms.div_euclid(1000);
    let milli = ms.rem_euclid(1000) as u32;
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400) as u32;
    // Howard Hinnant's civil_from_days.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = yoe + era * 400 + i64::from(m <= 2);
    (y, m, d, sod / 3600, sod % 3600 / 60, sod % 60, milli)
}

pub fn format_id(ms: i64) -> String {
    let (y, mo, d, h, mi, s, milli) = civil_from_ms(ms);
    format!("{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}-{milli:03}")
}

pub fn format_rfc3339(ms: i64) -> String {
    let (y, mo, d, h, mi, s, milli) = civil_from_ms(ms);
    format!("{y:04}-{mo:02}-{d:02}T{h:02}:{mi:02}:{s:02}.{milli:03}Z")
}

/// Parse the leading `YYYYMMDD-HHMMSS-mmm` of an id (fallback time for broken files).
pub fn parse_id_ms(id: &str) -> Option<i64> {
    let b = id.as_bytes();
    if b.len() < 19 || b[8] != b'-' || b[15] != b'-' || !b[..19].iter().enumerate().all(|(i, c)| i == 8 || i == 15 || c.is_ascii_digit()) {
        return None;
    }
    let n = |r: std::ops::Range<usize>| id[r].parse::<i64>().ok();
    let (y, m, d) = (n(0..4)?, n(4..6)?, n(6..8)?);
    let (h, mi, s, ms) = (n(9..11)?, n(11..13)?, n(13..15)?, n(16..19)?);
    let y2 = if m <= 2 { y - 1 } else { y };
    let era = y2.div_euclid(400);
    let yoe = y2.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    Some(((days * 86_400 + h * 3600 + mi * 60 + s) * 1000) + ms)
}

/// Pure debounce logic: when has the live config been stable long enough to record?
/// Time is passed in, so this is testable without sleeping.
pub struct Settler {
    recorded: Option<Config>,
    candidate: Option<Config>,
    since_ms: u64,
    settle_ms: u64,
}

impl Settler {
    pub fn new(recorded: Option<Config>, settle_ms: u64) -> Self {
        Self { recorded, candidate: None, since_ms: 0, settle_ms }
    }

    /// Feed the current live config. Returns true when it differs from the last recorded
    /// version and has been unchanged for `settle_ms`; call `mark_recorded` after writing.
    pub fn observe(&mut self, cfg: &Config, now_ms: u64) -> bool {
        if self.recorded.as_ref().is_some_and(|r| same_config(r, cfg)) {
            self.candidate = None;
            return false;
        }
        if !self.candidate.as_ref().is_some_and(|c| same_config(c, cfg)) {
            self.candidate = Some(cfg.clone());
            self.since_ms = now_ms;
            return false;
        }
        now_ms.saturating_sub(self.since_ms) >= self.settle_ms
    }

    pub fn mark_recorded(&mut self, cfg: &Config) {
        self.recorded = Some(cfg.clone());
        self.candidate = None;
    }
}

const POLL: Duration = Duration::from_millis(500);
const SETTLE_MS: u64 = 2000;
const HINT_TTL: Duration = Duration::from_secs(10);

struct Inner {
    history: History,
    shared: Arc<Mutex<Config>>,
    cfg_path: PathBuf,
    hint: Mutex<Option<(Source, Instant)>>,
}

impl Inner {
    fn live(&self) -> Config {
        self.shared.lock().unwrap().clone() // lock released before any I/O
    }

    fn take_hint(&self) -> Source {
        match self.hint.lock().unwrap().take() {
            Some((s, at)) if at.elapsed() < HINT_TTL => s,
            _ => Source::Auto,
        }
    }
}

/// Shared handle: background historian plus list / milestone / restore for any UI.
pub struct Service {
    inner: Arc<Inner>,
    stop: Mutex<Option<mpsc::Sender<()>>>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Service {
    /// `cfg_path` is blend.toml; history lives in its sibling `history/` folder.
    pub fn start(shared: Arc<Mutex<Config>>, cfg_path: PathBuf) -> Arc<Self> {
        let history = History::open(History::dir_for(&cfg_path));
        let inner = Arc::new(Inner { history, shared, cfg_path, hint: Mutex::new(None) });
        let (tx, rx) = mpsc::channel::<()>();
        let worker = inner.clone();
        let thread = std::thread::Builder::new()
            .name("edgeblend-historian".into())
            .spawn(move || {
                let cfg = worker.live();
                let mut settler = Settler::new(worker.history.newest_config(), SETTLE_MS);
                if worker.history.list().is_empty() && worker.history.record_auto(&cfg, Source::Auto).is_ok() {
                    settler.mark_recorded(&cfg);
                }
                let t0 = Instant::now();
                loop {
                    match rx.recv_timeout(POLL) {
                        Err(RecvTimeoutError::Timeout) => {}
                        _ => return, // shutdown (message or sender dropped)
                    }
                    let cfg = worker.live();
                    if settler.observe(&cfg, t0.elapsed().as_millis() as u64) {
                        let src = worker.take_hint();
                        match worker.history.record_auto(&cfg, src) {
                            Ok(_) => settler.mark_recorded(&cfg),
                            Err(e) => eprintln!("history: {e:#}"), // retried next tick
                        }
                    }
                }
            })
            .ok();
        Arc::new(Self { inner, stop: Mutex::new(Some(tx)), thread: Mutex::new(thread) })
    }

    pub fn dir(&self) -> &Path {
        self.inner.history.dir()
    }

    pub fn list(&self) -> Vec<Entry> {
        self.inner.history.list()
    }

    pub fn load(&self, id: &str) -> Result<Config> {
        self.inner.history.load(id)
    }

    /// Label the next automatic version (e.g. `Source::Remote` right after a remote edit).
    /// Expires after a few seconds.
    pub fn hint_source(&self, source: Source) {
        *self.inner.hint.lock().unwrap() = Some((source, Instant::now()));
    }

    /// Snapshot the live config as a named milestone (always written). Returns its id.
    pub fn save_milestone(&self, name: Option<String>, source: Source) -> Result<String> {
        let cfg = self.inner.live();
        self.inner.history.save_milestone(&cfg, name, source)
    }

    /// Load a version into the live config (keeping the runtime `mute_mask`) and save
    /// blend.toml. The historian then records it, labelled `Restore`. Deletes nothing.
    pub fn restore(&self, id: &str) -> Result<()> {
        let cfg = self.inner.history.load(id)?;
        self.apply(cfg, Source::Restore)
    }

    /// Import config text (blend.toml, export or snapshot file). Parses and validates first;
    /// on error nothing changes. On success saves a "before import" milestone of the current
    /// config, then applies the import.
    pub fn import_text(&self, text: &str) -> Result<()> {
        let cfg = Config::parse_import(text)?;
        self.save_milestone(Some("before import".into()), Source::Desktop)?;
        self.apply(cfg, Source::Import)
    }

    fn apply(&self, cfg: Config, source: Source) -> Result<()> {
        cfg.validate().context("that version is not valid")?;
        *self.inner.shared.lock().unwrap() = cfg.clone(); // lock released before I/O
        self.hint_source(source);
        cfg.save(&self.inner.cfg_path)
    }

    /// Stop the historian and record the final state. Idempotent.
    pub fn shutdown(&self) {
        drop(self.stop.lock().unwrap().take());
        if let Some(t) = self.thread.lock().unwrap().take() {
            let _ = t.join();
            let cfg = self.inner.live();
            let _ = self.inner.history.record_auto(&cfg, self.inner.take_hint());
        }
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        self.shutdown();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Tmp(PathBuf);
    impl Tmp {
        fn new() -> Self {
            static N: AtomicU64 = AtomicU64::new(0);
            let p = std::env::temp_dir().join(format!("edgeblend-hist-{}-{}", std::process::id(), N.fetch_add(1, Ordering::Relaxed)));
            std::fs::create_dir_all(&p).unwrap();
            Tmp(p)
        }
    }
    impl Drop for Tmp {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    fn cfg(gamma: f32) -> Config {
        Config { gamma, ..Config::default() }
    }

    #[test]
    fn time_formatting_round_trips() {
        let ms = 1_795_361_412_123; // 2026-11-22T15:30:12.123Z
        assert_eq!(format_rfc3339(ms), "2026-11-22T15:30:12.123Z");
        assert_eq!(format_id(ms), "20261122-153012-123");
        assert_eq!(parse_id_ms("20261122-153012-123"), Some(ms));
        assert_eq!(format_rfc3339(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(parse_id_ms("20000229-000000-000-4"), Some(951_782_400_000));
    }

    #[test]
    fn never_overwrites_same_millisecond() {
        let t = Tmp::new();
        let h = History::open(t.0.join("history"));
        let a = h.write(&cfg(1.0), Kind::Auto, None, Source::Auto, 5_000).unwrap();
        let before = std::fs::read(h.dir().join(format!("{a}.toml"))).unwrap();
        let b = h.write(&cfg(2.0), Kind::Auto, None, Source::Auto, 5_000).unwrap();
        let c = h.write(&cfg(3.0), Kind::Auto, None, Source::Auto, 5_000).unwrap();
        assert_ne!(a, b);
        assert_ne!(b, c);
        assert_eq!(before, std::fs::read(h.dir().join(format!("{a}.toml"))).unwrap());
        assert_eq!(h.load(&a).unwrap().gamma, 1.0);
        assert_eq!(h.load(&c).unwrap().gamma, 3.0);
        assert_eq!(h.list().len(), 3);
        // No temp files left behind.
        assert!(std::fs::read_dir(h.dir()).unwrap().all(|e| !e.unwrap().file_name().to_string_lossy().starts_with(".tmp")));
    }

    #[test]
    fn auto_dedupes_but_milestone_always_writes() {
        let t = Tmp::new();
        let h = History::open(t.0.join("history"));
        assert!(h.record_auto_at(&cfg(2.2), Source::Auto, 1000).unwrap().is_some());
        assert!(h.record_auto_at(&cfg(2.2), Source::Auto, 2000).unwrap().is_none());
        let mut muted = cfg(2.2);
        muted.mute_mask = 0b101; // persisted: a real change
        assert!(h.record_auto_at(&muted, Source::Auto, 2500).unwrap().is_some());
        assert!(h.record_auto_at(&cfg(2.2), Source::Auto, 2600).unwrap().is_some());
        assert!(h.record_auto_at(&cfg(1.8), Source::Auto, 3000).unwrap().is_some());
        h.save_milestone(&cfg(1.8), Some("same as last".into()), Source::Desktop).unwrap();
        let l = h.list();
        assert_eq!(l.len(), 5);
        assert_eq!(l[0].kind, Kind::Milestone);
        assert_eq!(l[0].name.as_deref(), Some("same as last"));
        assert_eq!(l[0].source, Source::Desktop);
    }

    #[test]
    fn list_is_newest_first() {
        let t = Tmp::new();
        let h = History::open(t.0.join("history"));
        for (i, ms) in [3000, 1000, 2000].into_iter().enumerate() {
            h.write(&cfg(1.0 + i as f32), Kind::Auto, None, Source::Auto, ms).unwrap();
        }
        let ms: Vec<i64> = h.list().iter().map(|e| e.created_ms).collect();
        assert_eq!(ms, vec![3000, 2000, 1000]);
    }

    #[test]
    fn bad_ids_rejected() {
        let t = Tmp::new();
        let h = History::open(t.0.join("history"));
        std::fs::write(t.0.join("secret.toml"), "x = 1").unwrap();
        for id in ["..", "../secret", "..\\secret", "a/b", "", "/etc/passwd", "C:\\x", "a.b", "x\0y"] {
            assert!(h.load(id).is_err(), "{id:?}");
        }
        assert!(!valid_id(&"a".repeat(49)));
    }

    #[test]
    fn broken_files_are_tolerated() {
        let t = Tmp::new();
        let h = History::open(t.0.join("history"));
        let good = h.write(&cfg(1.5), Kind::Auto, None, Source::Auto, 1_000_000).unwrap();
        std::fs::write(h.dir().join("20990101-000000-000.toml"), "this is = not [valid").unwrap();
        std::fs::write(h.dir().join("20990102-000000-000.toml"), "gamma = 1.0\n").unwrap(); // no [meta]
        std::fs::write(h.dir().join("notes.txt"), "hi").unwrap();
        let l = h.list();
        assert_eq!(l.len(), 3);
        assert!(l[0].broken.is_some() && l[1].broken.is_some() && l[2].broken.is_none());
        assert!(h.load("20990101-000000-000").is_err());
        assert_eq!(h.load(&good).unwrap().gamma, 1.5);
        // Newest *valid* version is used for dedupe.
        assert!(h.record_auto_at(&cfg(1.5), Source::Auto, 2_000_000).unwrap().is_none());
        assert!(History::open(t.0.join("missing")).list().is_empty());
    }

    #[test]
    fn config_round_trips_with_mute_mask() {
        let t = Tmp::new();
        let h = History::open(t.0.join("history"));
        let mut c = cfg(1.9);
        c.projectors = 2;
        c.reset_geometry();
        c.mute_mask = 3;
        let id = h.save_milestone(&c, Some("a/b:c*\n".into()), Source::Auto).unwrap();
        assert_eq!(h.load(&id).unwrap(), c);
        assert_eq!(h.list()[0].name.as_deref(), Some("abc"));
    }

    #[test]
    fn name_sanitizing() {
        assert_eq!(sanitize_name("  "), None);
        assert_eq!(sanitize_name(" Sunday \t setup "), Some("Sunday  setup".into()));
        assert_eq!(sanitize_name(&"x".repeat(500)).unwrap().chars().count(), MAX_NAME_CHARS);
        assert_eq!(sanitize_name("../..\\x"), Some("....x".into()));
    }

    #[test]
    fn settle_logic() {
        let mut s = Settler::new(Some(cfg(1.0)), 2000);
        assert!(!s.observe(&cfg(1.0), 0)); // unchanged
        assert!(!s.observe(&cfg(2.0), 100)); // change seen
        assert!(!s.observe(&cfg(2.0), 1500));
        assert!(!s.observe(&cfg(3.0), 2000)); // still dragging: timer restarts
        assert!(!s.observe(&cfg(3.0), 3999));
        assert!(s.observe(&cfg(3.0), 4000)); // stable for 2 s
        s.mark_recorded(&cfg(3.0));
        assert!(!s.observe(&cfg(3.0), 9000));
        assert!(!s.observe(&cfg(1.0), 9100)); // back to an old value is a change
    }

    #[test]
    fn settler_without_history_records_after_settle() {
        let mut s = Settler::new(None, 2000);
        assert!(!s.observe(&cfg(1.0), 0));
        assert!(s.observe(&cfg(1.0), 2000));
    }

    #[test]
    fn restore_round_trip_and_service() {
        let t = Tmp::new();
        let path = t.0.join("blend.toml");
        let shared = Arc::new(Mutex::new(cfg(2.2)));
        let svc = Service::start(shared.clone(), path.clone());
        let id = svc.save_milestone(Some("good".into()), Source::Desktop).unwrap();
        {
            let mut g = shared.lock().unwrap();
            g.gamma = 1.1;
            g.mute_mask = 4;
        }

        svc.restore(&id).unwrap();
        let live = shared.lock().unwrap().clone();
        assert_eq!(live.gamma, 2.2);
        assert_eq!(live.mute_mask, 0); // restore takes the file's value
        assert_eq!(Config::load(&path).unwrap().gamma, 2.2);
        svc.shutdown();
        svc.shutdown(); // idempotent
        let l = svc.list();
        assert!(l.iter().any(|e| e.id == id && e.kind == Kind::Milestone));
        assert!(svc.restore("../x").is_err());
        // Nothing was deleted: the milestone file is still intact.
        assert_eq!(svc.load(&id).unwrap().gamma, 2.2);
    }

    #[test]
    fn import_snapshots_current_then_applies_and_bad_import_changes_nothing() {
        let t = Tmp::new();
        let path = t.0.join("blend.toml");
        let shared = Arc::new(Mutex::new(cfg(2.2)));
        let svc = Service::start(shared.clone(), path);
        assert!(svc.import_text("garbage = [").is_err());
        assert!(svc.import_text("projectors = 50").is_err());
        assert_eq!(shared.lock().unwrap().gamma, 2.2);
        assert!(svc.list().iter().all(|e| e.kind != Kind::Milestone));
        let imp = Config { gamma: 1.4, mute_mask: 2, ..Config::default() };
        svc.import_text(&toml::to_string_pretty(&imp).unwrap()).unwrap();
        assert_eq!(*shared.lock().unwrap(), imp);
        let l: Vec<_> = svc.list().into_iter().filter(|e| e.kind == Kind::Milestone).collect();
        assert_eq!(l.len(), 1);
        assert_eq!(l[0].name.as_deref(), Some("before import"));
        assert_eq!(svc.load(&l[0].id).unwrap().gamma, 2.2);
        svc.shutdown();
    }

    #[test]
    fn historian_records_startup_version() {
        let t = Tmp::new();
        let shared = Arc::new(Mutex::new(cfg(2.0)));
        let svc = Service::start(shared, t.0.join("blend.toml"));
        svc.shutdown();
        assert_eq!(svc.list().len(), 1);
    }
}
