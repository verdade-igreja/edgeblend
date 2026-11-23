//! Edit ops sent by the phone (`POST /api/edit`), applied to the config under its lock.
//! Pure functions: no I/O, so they are tested without a server.

use anyhow::{bail, ensure, Result};
use edgeblend_config::{Config, Pattern};
use serde::Deserialize;

pub const UNDO_MAX: usize = 100;

/// Corner components are clamped to this inclusive range after geometry edits.
pub const CORNER_MIN: f32 = -2.0;
pub const CORNER_MAX: f32 = 3.0;
/// Max absolute nudge step accepted (larger values are clamped).
pub const NUDGE_MAX: f32 = 0.1;
/// Matches the desktop / phone gamma slider.
pub const GAMMA_MIN: f32 = 1.0;
pub const GAMMA_MAX: f32 = 3.0;
/// Matches the desktop / phone curve slider.
pub const CURVE_MIN: f32 = 0.5;
pub const CURVE_MAX: f32 = 4.0;
/// Upper bound for mute bits (the desktop panel allows 1..=6 projectors).
pub const MAX_PROJECTORS: u32 = 6;

/// One phone-local geometry undo entry: restore only this projector's corners.
#[derive(Debug, Clone, PartialEq)]
pub struct GeometryUndo {
    pub projector: usize,
    /// Corners before the phone op.
    pub previous: [[f32; 2]; 4],
    /// Corners immediately after the phone op (stale if someone else edited since).
    pub post: [[f32; 2]; 4],
    /// Entries sharing a group (one `nudge_all`) are undone together.
    pub group: u64,
}

/// Selection target for a nudge: corner `c0..c3`, edge `e0..e3`, or whole quad `all`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Handle {
    C0,
    C1,
    C2,
    C3,
    E0,
    E1,
    E2,
    E3,
    All,
}

impl Handle {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "c0" => Some(Self::C0),
            "c1" => Some(Self::C1),
            "c2" => Some(Self::C2),
            "c3" => Some(Self::C3),
            "e0" => Some(Self::E0),
            "e1" => Some(Self::E1),
            "e2" => Some(Self::E2),
            "e3" => Some(Self::E3),
            "all" => Some(Self::All),
            _ => None,
        }
    }

    /// Corner indices this handle moves (TL=0, TR=1, BR=2, BL=3).
    pub fn corners(self) -> &'static [usize] {
        match self {
            Self::C0 => &[0],
            Self::C1 => &[1],
            Self::C2 => &[2],
            Self::C3 => &[3],
            // e0 top=c0,c1; e1 right=c1,c2; e2 bottom=c2,c3; e3 left=c3,c0
            Self::E0 => &[0, 1],
            Self::E1 => &[1, 2],
            Self::E2 => &[2, 3],
            Self::E3 => &[3, 0],
            Self::All => &[0, 1, 2, 3],
        }
    }
}

/// JSON body for `POST /api/edit`.
#[derive(Debug, Clone, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Op {
    Nudge {
        projector: usize,
        handle: String,
        dx: f32,
        dy: f32,
    },
    /// Same as `Nudge`, applied to every projector at once (one undo step).
    NudgeAll {
        handle: String,
        dx: f32,
        dy: f32,
    },
    SetCorners {
        projector: usize,
        corners: [[f32; 2]; 4],
    },
    /// `value` is the pattern's serde name (as listed in `/api/state` `patterns`).
    Pattern {
        value: Pattern,
    },
    /// Black out (`muted: true`) or restore one projector (`mute_mask` bit).
    SetMute {
        projector: usize,
        muted: bool,
    },
    BlendRamps {
        value: bool,
    },
    Gamma {
        value: f32,
    },
    Curve {
        value: f32,
    },
    Undo,
}

fn next_group(undo: &[GeometryUndo]) -> u64 {
    undo.last().map_or(0, |u| u.group.wrapping_add(1))
}

fn nudge_one(
    cfg: &mut Config,
    projector: usize,
    h: Handle,
    dx: f32,
    dy: f32,
    group: u64,
    undo: &mut Vec<GeometryUndo>,
) {
    let dx = dx.clamp(-NUDGE_MAX, NUDGE_MAX);
    let dy = dy.clamp(-NUDGE_MAX, NUDGE_MAX);
    let previous = cfg.geometry[projector].corners;
    let corners = &mut cfg.geometry[projector].corners;
    for &ci in h.corners() {
        corners[ci][0] += dx;
        corners[ci][1] += dy;
    }
    clamp_corners(corners);
    let post = *corners;
    push_geometry_undo(undo, GeometryUndo { projector, previous, post, group });
}

fn push_geometry_undo(undo: &mut Vec<GeometryUndo>, entry: GeometryUndo) {
    undo.push(entry);
    while undo.len() > UNDO_MAX {
        undo.remove(0);
    }
}

fn require_projector(cfg: &Config, i: usize) -> Result<()> {
    ensure!(
        i < cfg.geometry.len(),
        "projector index {i} out of range (have {})",
        cfg.geometry.len()
    );
    Ok(())
}

fn require_finite(v: f32, name: &str) -> Result<()> {
    ensure!(v.is_finite(), "{name} must be finite");
    Ok(())
}

fn clamp_corners(corners: &mut [[f32; 2]; 4]) {
    for c in corners.iter_mut() {
        c[0] = c[0].clamp(CORNER_MIN, CORNER_MAX);
        c[1] = c[1].clamp(CORNER_MIN, CORNER_MAX);
    }
}

fn corners_eq(a: &[[f32; 2]; 4], b: &[[f32; 2]; 4]) -> bool {
    a.iter()
        .zip(b.iter())
        .all(|(p, q)| p[0] == q[0] && p[1] == q[1])
}

/// Apply `op` to `cfg`, updating the phone geometry undo stack when needed.
pub fn apply(cfg: &mut Config, op: &Op, undo: &mut Vec<GeometryUndo>) -> Result<()> {
    match op {
        Op::Nudge {
            projector,
            handle,
            dx,
            dy,
        } => {
            require_finite(*dx, "dx")?;
            require_finite(*dy, "dy")?;
            require_projector(cfg, *projector)?;
            let h = Handle::parse(handle)
                .ok_or_else(|| anyhow::anyhow!("invalid handle '{handle}'"))?;
            let group = next_group(undo);
            nudge_one(cfg, *projector, h, *dx, *dy, group, undo);
        }
        Op::NudgeAll { handle, dx, dy } => {
            require_finite(*dx, "dx")?;
            require_finite(*dy, "dy")?;
            let h = Handle::parse(handle)
                .ok_or_else(|| anyhow::anyhow!("invalid handle '{handle}'"))?;
            ensure!(!cfg.geometry.is_empty(), "no projectors");
            let group = next_group(undo);
            for p in 0..cfg.geometry.len() {
                nudge_one(cfg, p, h, *dx, *dy, group, undo);
            }
        }
        Op::SetCorners {
            projector,
            corners,
        } => {
            require_projector(cfg, *projector)?;
            for (i, c) in corners.iter().enumerate() {
                require_finite(c[0], &format!("corners[{i}].x"))?;
                require_finite(c[1], &format!("corners[{i}].y"))?;
            }
            let previous = cfg.geometry[*projector].corners;
            let mut next = *corners;
            clamp_corners(&mut next);
            cfg.geometry[*projector].corners = next;
            let group = next_group(undo);
            push_geometry_undo(
                undo,
                GeometryUndo {
                    projector: *projector,
                    previous,
                    post: next,
                    group,
                },
            );
        }
        Op::Pattern { value } => {
            cfg.pattern = *value;
        }
        Op::SetMute { projector, muted } => {
            ensure!(
                *projector < cfg.projectors.clamp(1, MAX_PROJECTORS) as usize,
                "projector index {projector} out of range (have {})",
                cfg.projectors
            );
            let bit = 1u32 << projector;
            if *muted {
                cfg.mute_mask |= bit;
            } else {
                cfg.mute_mask &= !bit;
            }
        }
        Op::BlendRamps { value } => {
            cfg.blend_ramps = *value;
        }
        Op::Gamma { value } => {
            require_finite(*value, "gamma")?;
            cfg.gamma = value.clamp(GAMMA_MIN, GAMMA_MAX);
        }
        Op::Curve { value } => {
            require_finite(*value, "curve")?;
            cfg.curve = value.clamp(CURVE_MIN, CURVE_MAX);
        }
        Op::Undo => {
            let Some(top) = undo.last() else {
                bail!("nothing to undo");
            };
            let group = top.group;
            let start = undo
                .iter()
                .rposition(|u| u.group != group)
                .map_or(0, |i| i + 1);
            let stale = undo[start..].iter().any(|u| {
                u.projector >= cfg.geometry.len()
                    || !corners_eq(&cfg.geometry[u.projector].corners, &u.post)
            });
            if stale {
                // A projector was changed by someone else since this phone op.
                undo.clear();
                bail!("nothing to undo");
            }
            for entry in undo.drain(start..).rev() {
                cfg.geometry[entry.projector].corners = entry.previous;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
#[allow(clippy::needless_range_loop)]
mod tests {
    use super::*;
    use edgeblend_config::Config;

    fn cfg3() -> Config {
        let mut c = Config::default();
        c.ensure_geometry();
        assert_eq!(c.geometry.len(), 3);
        c
    }

    #[test]
    fn corner_nudge_moves_one() {
        let mut cfg = cfg3();
        let before = cfg.geometry[0].corners;
        let mut undo = Vec::new();
        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 0,
                handle: "c0".into(),
                dx: 0.01,
                dy: -0.02,
            },
            &mut undo,
        )
        .unwrap();
        assert!((cfg.geometry[0].corners[0][0] - (before[0][0] + 0.01)).abs() < 1e-6);
        assert!((cfg.geometry[0].corners[0][1] - (before[0][1] - 0.02)).abs() < 1e-6);
        for i in 1..4 {
            assert_eq!(cfg.geometry[0].corners[i], before[i]);
        }
        assert_eq!(undo.len(), 1);
        assert_eq!(undo[0].projector, 0);
        assert_eq!(undo[0].previous, before);
        assert_eq!(undo[0].post, cfg.geometry[0].corners);
    }

    #[test]
    fn edge_nudge_moves_exactly_two_corners() {
        let mut cfg = cfg3();
        let before = cfg.geometry[1].corners;
        let mut undo = Vec::new();
        // e0 = top = c0, c1
        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 1,
                handle: "e0".into(),
                dx: 0.05,
                dy: 0.0,
            },
            &mut undo,
        )
        .unwrap();
        assert!((cfg.geometry[1].corners[0][0] - (before[0][0] + 0.05)).abs() < 1e-6);
        assert!((cfg.geometry[1].corners[1][0] - (before[1][0] + 0.05)).abs() < 1e-6);
        assert_eq!(cfg.geometry[1].corners[2], before[2]);
        assert_eq!(cfg.geometry[1].corners[3], before[3]);

        // e1 = right = c1, c2
        let before = cfg.geometry[1].corners;
        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 1,
                handle: "e1".into(),
                dx: 0.0,
                dy: 0.1,
            },
            &mut undo,
        )
        .unwrap();
        assert_eq!(cfg.geometry[1].corners[0], before[0]);
        assert!((cfg.geometry[1].corners[1][1] - (before[1][1] + 0.1)).abs() < 1e-6);
        assert!((cfg.geometry[1].corners[2][1] - (before[2][1] + 0.1)).abs() < 1e-6);
        assert_eq!(cfg.geometry[1].corners[3], before[3]);
    }

    #[test]
    fn all_moves_four_corners() {
        let mut cfg = cfg3();
        let before = cfg.geometry[2].corners;
        let mut undo = Vec::new();
        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 2,
                handle: "all".into(),
                dx: -0.03,
                dy: 0.04,
            },
            &mut undo,
        )
        .unwrap();
        for i in 0..4 {
            assert!((cfg.geometry[2].corners[i][0] - (before[i][0] - 0.03)).abs() < 1e-6);
            assert!((cfg.geometry[2].corners[i][1] - (before[i][1] + 0.04)).abs() < 1e-6);
        }
    }

    #[test]
    fn undo_restores_only_that_projector() {
        let mut cfg = cfg3();
        let snap = cfg.geometry.clone();
        let mut undo = Vec::new();
        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 0,
                handle: "c2".into(),
                dx: 0.05,
                dy: 0.05,
            },
            &mut undo,
        )
        .unwrap();
        // Simulate a desktop edit on another projector after the phone op.
        cfg.geometry[1].corners[0][0] += 0.02;
        let other = cfg.geometry[1].corners;

        assert_ne!(cfg.geometry[0].corners[2], snap[0].corners[2]);
        apply(&mut cfg, &Op::Undo, &mut undo).unwrap();
        assert_eq!(cfg.geometry[0].corners, snap[0].corners);
        assert_eq!(cfg.geometry[1].corners, other);
        assert!(undo.is_empty());
        assert!(apply(&mut cfg, &Op::Undo, &mut undo).is_err());
    }

    #[test]
    fn undo_clears_when_same_projector_changed_externally() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 0,
                handle: "c0".into(),
                dx: 0.01,
                dy: 0.0,
            },
            &mut undo,
        )
        .unwrap();
        assert_eq!(undo.len(), 1);
        // Desktop (or another client) changed the same projector.
        cfg.geometry[0].corners[1][0] += 0.03;
        assert!(apply(&mut cfg, &Op::Undo, &mut undo).is_err());
        assert!(undo.is_empty());
    }

    #[test]
    fn bad_projector_index_rejected() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        let err = apply(
            &mut cfg,
            &Op::Nudge {
                projector: 9,
                handle: "c0".into(),
                dx: 0.0,
                dy: 0.0,
            },
            &mut undo,
        );
        assert!(err.is_err());
        assert!(undo.is_empty());
    }

    #[test]
    fn bad_handle_rejected() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        let err = apply(
            &mut cfg,
            &Op::Nudge {
                projector: 0,
                handle: "cx".into(),
                dx: 0.0,
                dy: 0.0,
            },
            &mut undo,
        );
        assert!(err.is_err());
        assert!(undo.is_empty());
    }

    #[test]
    fn non_finite_rejected_and_ranges_clamped() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        assert!(apply(
            &mut cfg,
            &Op::Nudge {
                projector: 0,
                handle: "c0".into(),
                dx: f32::NAN,
                dy: 0.0,
            },
            &mut undo,
        )
        .is_err());
        assert!(apply(
            &mut cfg,
            &Op::Gamma {
                value: f32::INFINITY,
            },
            &mut undo,
        )
        .is_err());
        assert!(apply(
            &mut cfg,
            &Op::Curve {
                value: f32::NAN,
            },
            &mut undo,
        )
        .is_err());
        assert!(apply(
            &mut cfg,
            &Op::SetCorners {
                projector: 0,
                corners: [[0.0, 0.0], [1.0, f32::NAN], [1.0, 1.0], [0.0, 1.0]],
            },
            &mut undo,
        )
        .is_err());

        apply(
            &mut cfg,
            &Op::Nudge {
                projector: 0,
                handle: "c0".into(),
                dx: 1.0,
                dy: -1.0,
            },
            &mut undo,
        )
        .unwrap();
        // |dx|,|dy| clamped to 0.1 before apply
        let c0 = cfg.geometry[0].corners[0];
        assert!(c0[0].is_finite() && c0[1].is_finite());

        apply(&mut cfg, &Op::Gamma { value: 0.0 }, &mut undo).unwrap();
        assert!((cfg.gamma - GAMMA_MIN).abs() < 1e-6);
        apply(&mut cfg, &Op::Gamma { value: 9.0 }, &mut undo).unwrap();
        assert!((cfg.gamma - GAMMA_MAX).abs() < 1e-6);
        apply(&mut cfg, &Op::Curve { value: 0.1 }, &mut undo).unwrap();
        assert!((cfg.curve - CURVE_MIN).abs() < 1e-6);
        apply(&mut cfg, &Op::Curve { value: 9.0 }, &mut undo).unwrap();
        assert!((cfg.curve - CURVE_MAX).abs() < 1e-6);

        let far = [[10.0, -10.0], [10.0, 10.0], [-10.0, 10.0], [-10.0, -10.0]];
        apply(
            &mut cfg,
            &Op::SetCorners {
                projector: 1,
                corners: far,
            },
            &mut undo,
        )
        .unwrap();
        for c in &cfg.geometry[1].corners {
            assert!(c[0] >= CORNER_MIN && c[0] <= CORNER_MAX);
            assert!(c[1] >= CORNER_MIN && c[1] <= CORNER_MAX);
        }
    }

    #[test]
    fn set_corners_and_pattern() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        let corners = [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]];
        apply(
            &mut cfg,
            &Op::SetCorners {
                projector: 0,
                corners,
            },
            &mut undo,
        )
        .unwrap();
        assert_eq!(cfg.geometry[0].corners, corners);
        apply(
            &mut cfg,
            &Op::Pattern {
                value: Pattern::Diagnostic,
            },
            &mut undo,
        )
        .unwrap();
        assert_eq!(cfg.pattern, Pattern::Diagnostic);
        // pattern does not push undo
        assert_eq!(undo.len(), 1);
        apply(
            &mut cfg,
            &Op::BlendRamps { value: false },
            &mut undo,
        )
        .unwrap();
        assert!(!cfg.blend_ramps);
        assert_eq!(undo.len(), 1);
        apply(&mut cfg, &Op::Gamma { value: 2.4 }, &mut undo).unwrap();
        assert!((cfg.gamma - 2.4).abs() < 1e-6);
        apply(&mut cfg, &Op::Curve { value: 1.5 }, &mut undo).unwrap();
        assert!((cfg.curve - 1.5).abs() < 1e-6);
        assert_eq!(undo.len(), 1);
    }

    #[test]
    fn set_mute_toggles_one_bit() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        apply(&mut cfg, &Op::SetMute { projector: 1, muted: true }, &mut undo).unwrap();
        assert_eq!(cfg.mute_mask, 0b010);
        apply(&mut cfg, &Op::SetMute { projector: 2, muted: true }, &mut undo).unwrap();
        assert_eq!(cfg.mute_mask, 0b110);
        apply(&mut cfg, &Op::SetMute { projector: 1, muted: false }, &mut undo).unwrap();
        assert_eq!(cfg.mute_mask, 0b100);
        assert!(undo.is_empty());
    }

    #[test]
    fn set_mute_rejects_projector_out_of_range() {
        let mut cfg = cfg3();
        let mut undo = Vec::new();
        for p in [3, 31, 32, 1000, usize::MAX] {
            assert!(apply(&mut cfg, &Op::SetMute { projector: p, muted: true }, &mut undo).is_err());
        }
        assert_eq!(cfg.mute_mask, 0);
    }

    #[test]
    fn ops_parse_from_json_by_name() {
        let op: Op = serde_json::from_str(r#"{"op":"pattern","value":"white"}"#).unwrap();
        assert!(matches!(op, Op::Pattern { value: Pattern::White }));
        let op: Op = serde_json::from_str(r#"{"op":"set_mute","projector":2,"muted":true}"#).unwrap();
        assert!(matches!(op, Op::SetMute { projector: 2, muted: true }));
        // Unknown pattern, negative index, missing field, unknown op: all rejected.
        for bad in [
            r#"{"op":"pattern","value":"nope"}"#,
            r#"{"op":"set_mute","projector":-1,"muted":true}"#,
            r#"{"op":"set_mute","projector":0}"#,
            r#"{"op":"format_disk"}"#,
            r#"{"op":"gamma","value":"2"}"#,
        ] {
            assert!(serde_json::from_str::<Op>(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn nudge_all_moves_every_projector_and_undoes_as_one() {
        let mut cfg = cfg3();
        let snap = cfg.geometry.clone();
        let mut undo = Vec::new();
        let one = Op::Nudge { projector: 0, handle: "c0".into(), dx: 0.01, dy: 0.0 };
        apply(&mut cfg, &one, &mut undo).unwrap();
        let after_first = cfg.geometry.clone();
        let op: Op =
            serde_json::from_str(r#"{"op":"nudge_all","handle":"e0","dx":0,"dy":-0.02}"#).unwrap();
        apply(&mut cfg, &op, &mut undo).unwrap();
        for p in 0..3 {
            for c in [0, 1] {
                let want = after_first[p].corners[c][1] - 0.02;
                assert!((cfg.geometry[p].corners[c][1] - want).abs() < 1e-6);
            }
            assert_eq!(cfg.geometry[p].corners[2], after_first[p].corners[2]);
        }
        assert_eq!(undo.len(), 4);
        apply(&mut cfg, &Op::Undo, &mut undo).unwrap();
        assert_eq!(cfg.geometry, after_first);
        assert_eq!(undo.len(), 1);
        apply(&mut cfg, &Op::Undo, &mut undo).unwrap();
        assert_eq!(cfg.geometry, snap);
    }
}
