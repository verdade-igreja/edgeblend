use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

pub mod history;

use edgeblend_geom::even_split;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    /// Display whose desktop is captured (the canvas).
    pub source: String,
    /// Display that feeds the video-wall box / projectors.
    pub output: String,
    pub projectors: u32,
    /// Nominal wall canvas size in pixels (content / pattern coordinates).
    pub canvas: [u32; 2],
    /// Physical wall size in metres (width, height). Used by the calibration editor.
    #[serde(default = "default_wall_m")]
    pub wall_m: [f32; 2],
    /// Per-projector wall quads (TL, TR, BR, BL) in normalized wall coords.
    pub geometry: Vec<Projector>,
    /// Overlap fraction used when (re)generating an even split (0..~0.8).
    pub overlap_frac: f32,
    pub gamma: f32,
    /// Blend curve exponent: 1 = linear, 2 = smooth S-curve.
    pub curve: f32,
    /// What to show: the desktop or a calibration pattern.
    pub pattern: Pattern,
    /// Apply the blend ramps (turn off to see raw overlap while aligning).
    pub blend_ramps: bool,
    pub show_cursor: bool,
    /// Flip the output left/right (video-wall box or projectors mirror the image).
    pub mirror: bool,
    /// The box feeds the projectors right-to-left: reverse which slice each tile shows.
    pub reverse_order: bool,
    /// Live preview shown in the control panel.
    pub preview: Preview,
    /// Bit i set = projector i is blacked out (diagnostics). Saved like everything else so a
    /// config file is the whole setup; the panel warns while any projector is off.
    pub mute_mask: u32,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Projector {
    /// Wall landing of the projector image corners: TL, TR, BR, BL.
    pub corners: [[f32; 2]; 4],
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Pattern {
    #[default]
    Desktop,
    Diagnostic,
    Alignment,
    Gray,
    White,
    VLines,
    HLines,
    Diagonal,
}

impl Pattern {
    pub const ALL: [Pattern; 8] = [
        Pattern::Desktop,
        Pattern::Diagnostic,
        Pattern::Alignment,
        Pattern::Gray,
        Pattern::White,
        Pattern::VLines,
        Pattern::HLines,
        Pattern::Diagonal,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Pattern::Desktop => "Desktop",
            Pattern::Diagnostic => "Diagnostic (for photos)",
            Pattern::Alignment => "Alignment grid",
            Pattern::Gray => "Flat gray (blend check)",
            Pattern::White => "White",
            Pattern::VLines => "Vertical lines",
            Pattern::HLines => "Horizontal lines",
            Pattern::Diagonal => "Diagonal lines",
        }
    }

    pub fn shader_id(self) -> f32 {
        Self::ALL.iter().position(|&p| p == self).unwrap() as f32
    }

    pub fn next(self) -> Self {
        let i = Self::ALL.iter().position(|&p| p == self).unwrap();
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Preview {
    Off,
    #[default]
    Wall,
    Signal,
}

impl Preview {
    pub const ALL: [Preview; 3] = [Preview::Off, Preview::Wall, Preview::Signal];

    pub fn label(self) -> &'static str {
        match self {
            Preview::Off => "Off",
            Preview::Wall => "Wall (ideal)",
            Preview::Signal => "Signal to box",
        }
    }
}

fn default_wall_m() -> [f32; 2] {
    [9.0, 1.6875]
}

impl Default for Config {
    fn default() -> Self {
        let mut cfg = Self {
            source: r"\\.\DISPLAY1".into(),
            output: r"\\.\DISPLAY2".into(),
            projectors: 3,
            canvas: [5760, 1080],
            wall_m: default_wall_m(),
            geometry: Vec::new(),
            overlap_frac: 0.25,
            gamma: 2.2,
            curve: 2.0,
            pattern: Pattern::Desktop,
            blend_ramps: true,
            show_cursor: true,
            mirror: false,
            reverse_order: false,
            preview: Preview::Wall,
            mute_mask: 0,
        };
        cfg.ensure_geometry();
        cfg
    }
}

impl Config {
    /// %APPDATA%\edgeblend\blend.toml (falls back to ./blend.toml).
    pub fn default_path() -> PathBuf {
        std::env::var_os("APPDATA")
            .map(|a| PathBuf::from(a).join("edgeblend").join("blend.toml"))
            .unwrap_or_else(|| PathBuf::from("blend.toml"))
    }

    /// Make `geometry.len() == projectors`. Existing (calibrated) quads are kept; extra
    /// ones are truncated, missing ones are taken from an even split from `overlap_frac`.
    pub fn ensure_geometry(&mut self) {
        let n = self.projectors.max(1) as usize;
        let have = self.geometry.len();
        if have == n {
            return;
        }
        if have > n {
            self.geometry.truncate(n);
            return;
        }
        let fresh = even_split(n, self.overlap_frac);
        if have == 0 {
            self.geometry = fresh.into_iter().map(|corners| Projector { corners }).collect();
        } else {
            self.geometry
                .extend(fresh.into_iter().skip(have).map(|corners| Projector { corners }));
        }
    }

    /// Replace geometry with a fresh even split (uses current `projectors` / `overlap_frac`).
    pub fn reset_geometry(&mut self) {
        self.geometry.clear();
        self.ensure_geometry();
    }

    pub fn load(path: &Path) -> Result<Self> {
        let text = std::fs::read_to_string(path).with_context(|| format!("reading {}", path.display()))?;
        let mut cfg: Self = toml::from_str(&text).with_context(|| format!("parsing {}", path.display()))?;
        cfg.ensure_geometry();
        Ok(cfg)
    }

    /// Parse config text from any source: a plain blend.toml, an export, or a history
    /// snapshot (its `[meta]` table is ignored). Validates; on error nothing is applied.
    pub fn parse_import(text: &str) -> Result<Self> {
        let mut cfg: Self = toml::from_str(text).context("not a valid edgeblend config file")?;
        cfg.ensure_geometry();
        cfg.validate().context("config values are invalid")?;
        Ok(cfg)
    }

    /// Atomic write of this config to an arbitrary path (export); same format as `save`.
    pub fn export(&self, path: &Path) -> Result<()> {
        self.save(path)
    }

    /// Load, or defaults if the file is missing or broken.
    pub fn load_or_default(path: &Path) -> Self {
        if !path.exists() {
            return Self::default();
        }
        Self::load(path).unwrap_or_else(|e| {
            // Keep the broken file so autosave can't destroy the calibration in it.
            let bad = path.with_extension(format!(
                "toml.bad-{}",
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map(|d| d.as_secs())
                    .unwrap_or(0)
            ));
            let _ = std::fs::copy(path, &bad);
            eprintln!("{e:#}; using defaults (broken file kept as {})", bad.display());
            Self::default()
        })
    }

    /// Atomic save: write a temp file next to the target, then rename over it.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let body = toml::to_string_pretty(self)?;
        // Unique temp name: the panel, the phone remote and history restore may save at once.
        static SAVE_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let n = SAVE_COUNTER.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let tmp = path.with_extension(format!("toml.tmp-{}-{n}", std::process::id()));
        std::fs::write(&tmp, format!("# edgeblend config (managed by the app UI)\n{body}"))?;
        std::fs::rename(&tmp, path)
            .inspect_err(|_| {
                let _ = std::fs::remove_file(&tmp);
            })
            .with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
    }

    /// Apply onto `self` only what changed from `before` to `after` (field by field; mute bits
    /// one by one). Lets an editor that worked on a copy write back without overwriting edits
    /// someone else (phone remote, hotkeys) made to the shared config in the meantime.
    pub fn merge_changes(&mut self, before: &Config, after: &Config) {
        // Exhaustive destructuring: a new field fails to compile until it is merged here.
        let Config {
            source,
            output,
            projectors,
            canvas,
            wall_m,
            geometry,
            overlap_frac,
            gamma,
            curve,
            pattern,
            blend_ramps,
            show_cursor,
            mirror,
            reverse_order,
            preview,
            mute_mask,
        } = after;
        macro_rules! take {
            ($($f:ident),*) => {$(
                if *$f != before.$f {
                    self.$f = $f.clone();
                }
            )*};
        }
        take!(
            source, output, projectors, canvas, wall_m, geometry, overlap_frac, gamma, curve,
            pattern, blend_ramps, show_cursor, mirror, reverse_order, preview
        );
        let flipped = before.mute_mask ^ *mute_mask;
        self.mute_mask = (self.mute_mask & !flipped) | (*mute_mask & flipped);
        if self.projectors as usize != self.geometry.len() {
            self.ensure_geometry();
        }
    }

    pub fn validate(&self) -> Result<()> {
        ensure!(self.projectors >= 1, "projectors must be >= 1");
        ensure!(self.projectors <= 6, "projectors must be <= 6");
        ensure!(self.canvas[0] > 0 && self.canvas[1] > 0, "canvas size must be > 0");
        ensure!(self.wall_m[0] > 0.0 && self.wall_m[1] > 0.0, "wall_m must be > 0");
        ensure!(
            self.overlap_frac >= 0.0 && self.overlap_frac < 1.0,
            "overlap_frac must be in [0, 1)"
        );
        ensure!(self.gamma > 0.0, "gamma must be > 0");
        ensure!(self.curve > 0.0, "curve must be > 0");
        ensure!(
            self.geometry.len() == self.projectors as usize,
            "geometry length must match projectors"
        );
        ensure!(
            !self.source.eq_ignore_ascii_case(&self.output),
            "source and output must be different displays (it would capture itself)"
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_keeps_concurrent_edits_to_other_fields() {
        let before = Config::default();
        // The panel changed gamma on its copy...
        let mut mine = before.clone();
        mine.gamma = 2.6;
        // ...while someone else (phone, hotkey) changed the pattern in the shared config.
        let mut shared = before.clone();
        shared.pattern = Pattern::White;

        shared.merge_changes(&before, &mine);
        assert_eq!(shared.gamma, 2.6);
        assert_eq!(shared.pattern, Pattern::White);
    }

    #[test]
    fn merge_applies_mute_bits_individually() {
        let before = Config::default();
        let mut mine = before.clone();
        mine.mute_mask = 0b001; // panel muted P1
        let mut shared = before.clone();
        shared.mute_mask = 0b100; // phone muted P3 meanwhile

        shared.merge_changes(&before, &mine);
        assert_eq!(shared.mute_mask, 0b101);
    }

    #[test]
    fn merge_without_changes_is_a_no_op() {
        let before = Config::default();
        let mut shared = before.clone();
        shared.show_cursor = !shared.show_cursor;
        let expect = shared.clone();
        shared.merge_changes(&before, &before.clone());
        assert_eq!(shared, expect);
    }

    #[test]
    fn merge_changed_projectors_brings_geometry_along() {
        let before = Config::default();
        let mut mine = before.clone();
        mine.projectors = 2;
        mine.ensure_geometry();
        let mut shared = before.clone();
        shared.merge_changes(&before, &mine);
        assert_eq!(shared.projectors, 2);
        assert_eq!(shared.geometry.len(), 2);
    }

    #[test]
    fn old_file_without_mute_mask_loads_as_zero() {
        let c = Config::parse_import("gamma = 2.0\n").unwrap();
        assert_eq!(c.mute_mask, 0);
    }

    #[test]
    fn mute_mask_is_persisted() {
        let c = Config { mute_mask: 5, ..Config::default() };
        let text = toml::to_string_pretty(&c).unwrap();
        assert_eq!(Config::parse_import(&text).unwrap().mute_mask, 5);
    }

    #[test]
    fn import_accepts_plain_file() {
        let text = toml::to_string_pretty(&Config { gamma: 1.7, ..Config::default() }).unwrap();
        assert_eq!(Config::parse_import(&text).unwrap().gamma, 1.7);
    }

    #[test]
    fn import_rejects_garbage_and_invalid() {
        assert!(Config::parse_import("this is = not [toml").is_err());
        assert!(Config::parse_import("gamma = \"x\"").is_err());
        assert!(Config::parse_import("projectors = 99\n").is_err());
        assert!(Config::parse_import("gamma = -1.0\n").is_err());
    }

    #[test]
    fn import_ignores_history_meta() {
        let text = format!(
            "{}\n[meta]\ncreated = \"2026-01-01T00:00:00.000Z\"\ncreated_ms = 1\nkind = \"milestone\"\nsource = \"desktop\"\n",
            toml::to_string_pretty(&Config { gamma: 1.3, ..Config::default() }).unwrap()
        );
        assert_eq!(Config::parse_import(&text).unwrap().gamma, 1.3);
    }

    #[test]
    fn concurrent_saves_do_not_clash() {
        let dir = std::env::temp_dir().join(format!("edgeblend-save-{}", std::process::id()));
        let path = dir.join("blend.toml");
        let threads: Vec<_> = (0..8)
            .map(|i| {
                let path = path.clone();
                std::thread::spawn(move || {
                    for _ in 0..20 {
                        Config { gamma: 1.0 + i as f32 * 0.1, ..Config::default() }.save(&path).unwrap();
                    }
                })
            })
            .collect();
        for t in threads {
            t.join().unwrap();
        }
        assert!(Config::load(&path).is_ok());
        let leftovers = std::fs::read_dir(&dir).unwrap().count();
        let _ = std::fs::remove_dir_all(&dir);
        assert_eq!(leftovers, 1, "temp files left behind");
    }
}
