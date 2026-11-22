//! `GET /api/state` body: what the phone needs to draw the projectors and buttons.

use edgeblend_config::{Config, Pattern};
use serde::Serialize;

#[derive(Debug, Serialize)]
pub struct PatternInfo {
    /// serde name, sent back in `{"op":"pattern","value":<name>}`.
    pub name: String,
    pub label: &'static str,
}

#[derive(Debug, Serialize)]
pub struct State {
    /// Bumped on every edit (phone or desktop); the phone drops responses older than it has.
    pub rev: u64,
    pub projectors: u32,
    pub overlap_frac: f32,
    /// One entry per projector, wall order left to right.
    pub muted: Vec<bool>,
    pub pattern: Pattern,
    /// Every pattern the app knows, in menu order (`Pattern::ALL`).
    pub patterns: Vec<PatternInfo>,
    pub mirror: bool,
    pub reverse_order: bool,
    pub blend_ramps: bool,
    pub gamma: f32,
    pub curve: f32,
    pub canvas: [u32; 2],
    pub wall_m: [f32; 2],
    /// Per-projector wall quads (TL, TR, BR, BL), normalized.
    pub geometry: Vec<[[f32; 2]; 4]>,
}

pub fn patterns() -> Vec<PatternInfo> {
    Pattern::ALL
        .iter()
        .map(|&p| PatternInfo {
            name: serde_json::to_value(p)
                .ok()
                .and_then(|v| v.as_str().map(str::to_owned))
                .unwrap_or_default(),
            label: p.label(),
        })
        .collect()
}

pub fn snapshot(cfg: &Config, rev: u64) -> State {
    let n = cfg.projectors.clamp(1, crate::apply::MAX_PROJECTORS);
    State {
        rev,
        projectors: n,
        overlap_frac: cfg.overlap_frac,
        muted: (0..n).map(|i| cfg.mute_mask & (1 << i) != 0).collect(),
        pattern: cfg.pattern,
        patterns: patterns(),
        mirror: cfg.mirror,
        reverse_order: cfg.reverse_order,
        blend_ramps: cfg.blend_ramps,
        gamma: cfg.gamma,
        curve: cfg.curve,
        canvas: cfg.canvas,
        wall_m: cfg.wall_m,
        geometry: cfg.geometry.iter().map(|p| p.corners).collect(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_cover_all_with_serde_names() {
        let list = patterns();
        assert_eq!(list.len(), Pattern::ALL.len());
        assert_eq!(list[0].name, "desktop");
        for (info, p) in list.iter().zip(Pattern::ALL) {
            assert_eq!(info.label, p.label());
            // The name round-trips to the same variant, so the phone can send it back.
            let back: Pattern = serde_json::from_value(serde_json::json!(info.name)).unwrap();
            assert_eq!(back, p);
        }
    }

    #[test]
    fn snapshot_reports_mutes_per_projector() {
        // A stray high bit must not add entries.
        let cfg = Config { mute_mask: 0b101 | (1 << 9), ..Config::default() };
        let s = snapshot(&cfg, 7);
        assert_eq!(s.rev, 7);
        assert_eq!(s.projectors, 3);
        assert_eq!(s.muted, vec![true, false, true]);
        assert_eq!(s.geometry.len(), 3);
    }

    #[test]
    fn snapshot_json_shape() {
        let cfg = Config { pattern: Pattern::Gray, ..Config::default() };
        let v = serde_json::to_value(snapshot(&cfg, 1)).unwrap();
        assert_eq!(v["pattern"], "gray");
        assert_eq!(v["patterns"][0]["name"], "desktop");
        assert_eq!(v["patterns"][0]["label"], "Desktop");
        assert!(v["overlap_frac"].is_number());
        assert!(v.get("mute_mask").is_none());
    }
}
