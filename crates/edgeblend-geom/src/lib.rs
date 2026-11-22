//! Pure geometry for projector quads: unit-square ↔ wall homographies.

/// Row-major 3×3 matrix. Applied as `p' = M * [x, y, 1]^T` with perspective divide.
pub type Mat3 = [[f32; 3]; 3];

/// Heckbert closed form: unit square `(0,0),(1,0),(1,1),(0,1)` → quad corners TL,TR,BR,BL.
pub fn square_to_quad(c: &[[f32; 2]; 4]) -> Mat3 {
    let [[x0, y0], [x1, y1], [x2, y2], [x3, y3]] = *c;

    let dx1 = x1 - x2;
    let dy1 = y1 - y2;
    let dx2 = x3 - x2;
    let dy2 = y3 - y2;
    let dx3 = x0 - x1 + x2 - x3;
    let dy3 = y0 - y1 + y2 - y3;

    if dx3.abs() < 1e-8 && dy3.abs() < 1e-8 {
        // Affine (parallelogram): second column is x2−x1 (= x3−x0 when dx3=0).
        return [
            [x1 - x0, x2 - x1, x0],
            [y1 - y0, y2 - y1, y0],
            [0.0, 0.0, 1.0],
        ];
    }

    let inv_det = 1.0 / (dx1 * dy2 - dx2 * dy1);
    let a13 = (dx3 * dy2 - dx2 * dy3) * inv_det;
    let a23 = (dx1 * dy3 - dy1 * dx3) * inv_det;
    [
        [x1 - x0 + a13 * x1, x3 - x0 + a23 * x3, x0],
        [y1 - y0 + a13 * y1, y3 - y0 + a23 * y3, y0],
        [a13, a23, 1.0],
    ]
}

/// Invert a 3×3 matrix. Returns `None` if the determinant is near zero.
pub fn invert(m: &Mat3) -> Option<Mat3> {
    let [[a, b, c], [d, e, f], [g, h, i]] = *m;
    let det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    Some([
        [
            (e * i - f * h) * inv,
            (c * h - b * i) * inv,
            (b * f - c * e) * inv,
        ],
        [
            (f * g - d * i) * inv,
            (a * i - c * g) * inv,
            (c * d - a * f) * inv,
        ],
        [
            (d * h - e * g) * inv,
            (b * g - a * h) * inv,
            (a * e - b * d) * inv,
        ],
    ])
}

/// Apply homography `m` to point `p` (perspective divide).
pub fn apply(m: &Mat3, p: [f32; 2]) -> [f32; 2] {
    let x = m[0][0] * p[0] + m[0][1] * p[1] + m[0][2];
    let y = m[1][0] * p[0] + m[1][1] * p[1] + m[1][2];
    let w = m[2][0] * p[0] + m[2][1] * p[1] + m[2][2];
    let inv_w = 1.0 / w;
    [x * inv_w, y * inv_w]
}

/// Even horizontal split of the unit wall into `n` projector quads.
///
/// Projector width `w = 1 / (n - (n-1)*f)`, step `w*(1-f)`, so the last right
/// edge lands exactly at 1 and neighbouring footprints overlap by fraction `f`.
pub fn even_split(n: usize, overlap_frac: f32) -> Vec<[[f32; 2]; 4]> {
    assert!(n >= 1);
    let f = overlap_frac.clamp(0.0, 0.999);
    let w = 1.0 / (n as f32 - (n as f32 - 1.0) * f);
    let step = w * (1.0 - f);
    (0..n)
        .map(|i| {
            let x0 = i as f32 * step;
            let x1 = x0 + w;
            [[x0, 0.0], [x1, 0.0], [x1, 1.0], [x0, 1.0]]
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: [f32; 2], b: [f32; 2], eps: f32) {
        assert!((a[0] - b[0]).abs() < eps && (a[1] - b[1]).abs() < eps, "{a:?} vs {b:?}");
    }

    #[test]
    fn square_to_quad_maps_corners() {
        let c = [[0.1, 0.2], [0.9, 0.15], [0.85, 0.9], [0.05, 0.95]];
        let h = square_to_quad(&c);
        approx(apply(&h, [0.0, 0.0]), c[0], 1e-5);
        approx(apply(&h, [1.0, 0.0]), c[1], 1e-5);
        approx(apply(&h, [1.0, 1.0]), c[2], 1e-5);
        approx(apply(&h, [0.0, 1.0]), c[3], 1e-5);
    }

    #[test]
    fn square_to_quad_affine_parallelogram() {
        // Axis-aligned rectangle is a parallelogram (affine case).
        let c = [[0.0, 0.0], [0.5, 0.0], [0.5, 1.0], [0.0, 1.0]];
        let h = square_to_quad(&c);
        approx(apply(&h, [0.0, 0.0]), c[0], 1e-5);
        approx(apply(&h, [1.0, 0.0]), c[1], 1e-5);
        approx(apply(&h, [1.0, 1.0]), c[2], 1e-5);
        approx(apply(&h, [0.0, 1.0]), c[3], 1e-5);
        approx(apply(&h, [0.5, 0.5]), [0.25, 0.5], 1e-5);
    }

    #[test]
    fn invert_round_trips() {
        let c = [[0.0, 0.0], [0.6, -0.05], [0.55, 1.1], [-0.05, 1.0]];
        let h = square_to_quad(&c);
        let hi = invert(&h).expect("invertible");
        for p in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0], [0.3, 0.7], [0.5, 0.5]] {
            let q = apply(&h, p);
            approx(apply(&hi, q), p, 1e-4);
        }
    }

    #[test]
    fn even_split_covers_unit_interval() {
        let f = 0.25_f32;
        for n in 1..=6 {
            let quads = even_split(n, f);
            assert_eq!(quads.len(), n);
            assert!((quads[0][0][0] - 0.0).abs() < 1e-5);
            assert!((quads[n - 1][1][0] - 1.0).abs() < 1e-5);
            for q in &quads {
                assert!((q[0][1] - 0.0).abs() < 1e-5);
                assert!((q[1][1] - 0.0).abs() < 1e-5);
                assert!((q[2][1] - 1.0).abs() < 1e-5);
                assert!((q[3][1] - 1.0).abs() < 1e-5);
            }
            if n >= 2 {
                let w = quads[0][1][0] - quads[0][0][0];
                let overlap = quads[0][1][0] - quads[1][0][0];
                assert!((overlap / w - f).abs() < 1e-5, "n={n} overlap frac");
            }
        }
    }
}
