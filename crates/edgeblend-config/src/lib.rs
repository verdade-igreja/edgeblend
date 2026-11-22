use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

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
    /// Bit i set = projector i is blacked out (diagnostics). Runtime only, never saved,
    /// so a projector can't stay accidentally dark at the next start.
    #[serde(skip)]
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
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, format!("# edgeblend config (managed by the app UI)\n{body}"))?;
        std::fs::rename(&tmp, path).with_context(|| format!("replacing {}", path.display()))?;
        Ok(())
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
