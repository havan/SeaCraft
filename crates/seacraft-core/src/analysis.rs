//! Surface fairing analysis: discrete curvature, zebra stripes and draft angle.
//!
//! A hull is faired by looking at how light reflects off it, not at its
//! polygons: zebra stripes show an unfair bump as a kink in the stripes, and a
//! curvature map shows it as a colour break. Both need per-vertex normals, and
//! the curvature map needs a real discrete curvature estimate, which is what
//! this module computes.
//!
//! Curvature follows Meyer, Desbrun, Schröder and Barr, *Discrete Differential
//! Geometry Operators for Triangulated 2-Manifolds* (2003):
//!
//! * mean curvature from the cotangent Laplacian, `Δp = 2 H A n`, so
//!   `|H| = |Δp| / (2A)`;
//! * Gaussian curvature from the angle deficit, `K = (2π − Σθ) / A`.
//!
//! Areas are barycentric (a third of each incident triangle), which is stable
//! on the skinny triangles a refined hull produces. The shading vocabulary
//! (zebra / curvature / draft, and their colour stops) is adapted from
//! Tangency Labs' solidcraft `render/src/analysis.rs` and the `analysis()`
//! function of its viewport shader; see `ATTRIBUTION.md`.

use crate::math::Vec3;

/// What the renderer should show instead of plain shading.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum AnalysisMode {
    /// Ordinary lit surface.
    #[default]
    Shaded,
    /// Reflection stripes: `stripes` bands across the view. An unfair surface
    /// breaks them.
    Zebra { stripes: f64 },
    /// Colour by mean curvature, reaching the "tight" end at `1 / radius`.
    Curvature { radius: f64 },
    /// Colour by draft angle away from `pull` (degrees).
    Draft { angle_deg: f64 },
}

impl AnalysisMode {
    pub fn label(self) -> &'static str {
        match self {
            AnalysisMode::Shaded => "shaded",
            AnalysisMode::Zebra { .. } => "zebra",
            AnalysisMode::Curvature { .. } => "curvature",
            AnalysisMode::Draft { .. } => "draft",
        }
    }

    pub fn is_shaded(self) -> bool {
        matches!(self, AnalysisMode::Shaded)
    }
}

/// Discrete curvature at one vertex, in 1/metres.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Curvature {
    /// Angle-deficit Gaussian curvature. Negative on saddles.
    pub gaussian: f64,
    /// Magnitude of the mean curvature: `1/r` on a sphere of radius `r`.
    pub mean: f64,
}

/// Area-weighted vertex normals (the sum of the incident face normals, which
/// weights each face by its area and is stable on non-planar quads).
pub fn vertex_normals(verts: &[Vec3], faces: &[Vec<usize>]) -> Vec<Vec3> {
    let mut normals = vec![Vec3::ZERO; verts.len()];
    for f in faces {
        if f.len() < 3 {
            continue;
        }
        let p0 = verts[f[0]];
        for i in 1..f.len() - 1 {
            let n = (verts[f[i]] - p0).cross(verts[f[i + 1]] - p0);
            for &v in f {
                if let Some(slot) = normals.get_mut(v) {
                    *slot += n;
                }
            }
        }
    }
    normals.iter().map(|n| n.norm()).collect()
}

/// Per-vertex Gaussian and mean curvature.
pub fn curvatures(verts: &[Vec3], faces: &[Vec<usize>]) -> Vec<Curvature> {
    let n = verts.len();
    let mut area = vec![0.0f64; n];
    let mut angle = vec![0.0f64; n];
    let mut laplacian = vec![Vec3::ZERO; n];
    let mut on_boundary = vec![false; n];

    let mut edge_uses: std::collections::HashMap<(usize, usize), usize> =
        std::collections::HashMap::new();
    for f in faces {
        for i in 0..f.len() {
            let a = f[i];
            let b = f[(i + 1) % f.len()];
            let key = if a < b { (a, b) } else { (b, a) };
            *edge_uses.entry(key).or_insert(0) += 1;
        }
    }
    for (f, uses) in &edge_uses {
        if *uses == 1 {
            on_boundary[f.0] = true;
            on_boundary[f.1] = true;
        }
    }

    for face in faces {
        if face.len() < 3 {
            continue;
        }
        // Fan triangulation: hull faces are quads, so this adds one diagonal
        // per face, which the area share absorbs.
        for i in 1..face.len() - 1 {
            let (ia, ib, ic) = (face[0], face[i], face[i + 1]);
            let (a, b, c) = (verts[ia], verts[ib], verts[ic]);
            let cross = (b - a).cross(c - a);
            let tri_area = 0.5 * cross.length();
            if tri_area < 1e-18 {
                continue;
            }
            for (v, p, q) in [(ia, b, c), (ib, c, a), (ic, a, b)] {
                area[v] += tri_area / 3.0;
                let (u, w) = ((p - verts[v]), (q - verts[v]));
                let len = (u.length() * w.length()).max(1e-18);
                angle[v] += (u.dot(w) / len).clamp(-1.0, 1.0).acos();
            }
            // Cotangent weights for the three edges, `w = cot(angle opposite)/2`.
            let cot = |o: Vec3, p: Vec3, q: Vec3| -> f64 {
                let (u, w) = (p - o, q - o);
                let cross = u.cross(w).length();
                if cross < 1e-18 {
                    0.0
                } else {
                    u.dot(w) / cross
                }
            };
            let w_ab = 0.5 * cot(c, a, b);
            let w_bc = 0.5 * cot(a, b, c);
            let w_ca = 0.5 * cot(b, c, a);
            for (i, j, w) in [(ia, ib, w_ab), (ib, ic, w_bc), (ic, ia, w_ca)] {
                laplacian[i] += (verts[j] - verts[i]) * w;
                laplacian[j] += (verts[i] - verts[j]) * w;
            }
        }
    }

    (0..n)
        .map(|i| {
            if area[i] < 1e-18 {
                return Curvature::default();
            }
            let full = if on_boundary[i] { std::f64::consts::PI } else { std::f64::consts::TAU };
            Curvature {
                gaussian: (full - angle[i]) / area[i],
                mean: laplacian[i].length() / (2.0 * area[i]),
            }
        })
        .collect()
}

/// Curvature a rendered pixel is asked about: interpolated from the vertices.
pub fn colour_stops() -> [[f64; 3]; 3] {
    // Flat (blue), mid (green), tight (red) — the false-colour convention.
    [
        [0.15, 0.35, 0.95],
        [0.10, 0.80, 0.25],
        [0.90, 0.15, 0.10],
    ]
}

/// The colour of one shaded point under `mode`.
///
/// `normal` is the surface normal, `view_dir` the unit vector from the surface
/// towards the eye, and `curvature` the interpolated discrete curvature (only
/// [`AnalysisMode::Curvature`] reads it).
pub fn analysis_color(
    mode: AnalysisMode,
    normal: Vec3,
    view_dir: Vec3,
    curvature: Curvature,
) -> [f64; 3] {
    // Shading and stripes flips the normal towards the eye; the draft angle
    // must use the surface's true orientation, or an undercut would read as a
    // pullable face.
    let true_normal = normal.norm();
    let n = if normal.dot(view_dir) < 0.0 { -normal } else { normal };
    let lit = 0.55 + 0.45 * n.dot(view_dir).max(0.0);
    match mode {
        AnalysisMode::Shaded => [lit, lit, lit],
        AnalysisMode::Zebra { stripes } => {
            // Upright light bars standing around the viewer: the stripe
            // follows the reflected ray's sideways direction.
            let reflected = reflect(-view_dir, n);
            let mut side = view_dir.cross(Vec3::new(0.0, 0.0, 1.0));
            if side.length() < 1e-3 {
                side = Vec3::new(1.0, 0.0, 0.0);
            }
            let s = (reflected.dot(side.norm()) * stripes * std::f64::consts::PI).sin();
            let c = if s > 0.0 { 0.95 } else { 0.08 };
            [c, c, c]
        }
        AnalysisMode::Draft { angle_deg } => {
            // Draft of the surface against the pull direction (+z).
            let a = (true_normal.dot(Vec3::new(0.0, 0.0, 1.0)))
                .clamp(-1.0, 1.0)
                .asin()
                .to_degrees();
            let stops = colour_stops();
            let c = if a >= angle_deg {
                stops[1] // can be pulled
            } else if a <= -angle_deg {
                stops[2] // undercut
            } else {
                stops[0] // borderline
            };
            [c[0] * lit, c[1] * lit, c[2] * lit]
        }
        AnalysisMode::Curvature { radius } => {
            let stops = colour_stops();
            let t = (curvature.mean * radius.max(1e-9)).clamp(0.0, 1.0);
            let c = if t < 0.5 {
                lerp(stops[0], stops[1], t * 2.0)
            } else {
                lerp(stops[1], stops[2], t * 2.0 - 1.0)
            };
            [c[0] * lit, c[1] * lit, c[2] * lit]
        }
    }
}

fn reflect(d: Vec3, n: Vec3) -> Vec3 {
    d - n * (2.0 * d.dot(n))
}

fn lerp(a: [f64; 3], b: [f64; 3], t: f64) -> [f64; 3] {
    let t = t.clamp(0.0, 1.0);
    [
        a[0] + (b[0] - a[0]) * t,
        a[1] + (b[1] - a[1]) * t,
        a[2] + (b[2] - a[2]) * t,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat grid in the z = 0 plane, `n` by `n` quads of size `h`.
    fn grid(n: usize, h: f64) -> (Vec<Vec3>, Vec<Vec<usize>>) {
        let mut verts = Vec::new();
        for j in 0..=n {
            for i in 0..=n {
                verts.push(Vec3::new(i as f64 * h, j as f64 * h, 0.0));
            }
        }
        let mut faces = Vec::new();
        for j in 0..n {
            for i in 0..n {
                let a = j * (n + 1) + i;
                faces.push(vec![a, a + 1, a + n + 2, a + n + 1]);
            }
        }
        (verts, faces)
    }

    /// A cylinder of radius `r` around the z axis: `n` around, `m` along.
    fn cylinder(r: f64, n: usize, m: usize, h: f64) -> (Vec<Vec3>, Vec<Vec<usize>>) {
        let mut verts = Vec::new();
        for j in 0..=m {
            for i in 0..n {
                let a = std::f64::consts::TAU * i as f64 / n as f64;
                verts.push(Vec3::new(r * a.cos(), r * a.sin(), j as f64 * h));
            }
        }
        let mut faces = Vec::new();
        for j in 0..m {
            for i in 0..n {
                let a = j * n + i;
                let b = j * n + (i + 1) % n;
                faces.push(vec![a, b, b + n, a + n]);
            }
        }
        (verts, faces)
    }

    #[test]
    fn a_plane_is_flat() {
        let (verts, faces) = grid(6, 0.5);
        let k = curvatures(&verts, &faces);
        // Interior vertices only: the rim is a boundary, where both estimates
        // are biased by the missing neighbour.
        for j in 1..6 {
            for i in 1..6 {
                let c = k[j * 7 + i];
                assert!(c.gaussian.abs() < 1e-9, "K {} at ({i},{j})", c.gaussian);
                assert!(c.mean.abs() < 1e-9, "H {} at ({i},{j})", c.mean);
            }
        }
    }

    #[test]
    fn a_cylinder_has_the_right_mean_curvature_and_no_gaussian_curvature() {
        let r = 2.0;
        let (verts, faces) = cylinder(r, 64, 4, 0.5);
        let k = curvatures(&verts, &faces);
        let inside: Vec<&Curvature> = (1..4)
            .flat_map(|j| (0..64).map(move |i| j * 64 + i))
            .map(|i| &k[i])
            .collect();
        let mean = inside.iter().map(|c| c.mean).sum::<f64>() / inside.len() as f64;
        // H = 1/(2r) for a cylinder: only one principal curvature is non-zero.
        let want = 1.0 / (2.0 * r);
        assert!((mean - want).abs() < 0.02 * want, "mean curvature {mean}, expected {want}");
        let gaussian = inside.iter().map(|c| c.gaussian.abs()).fold(0.0, f64::max);
        assert!(gaussian < 0.01, "a cylinder is developable: K = {gaussian}");
    }

    #[test]
    fn normals_point_out_of_the_cylinder() {
        let (verts, faces) = cylinder(2.0, 32, 2, 0.5);
        let normals = vertex_normals(&verts, &faces);
        for (i, n) in normals.iter().enumerate() {
            let radial = Vec3::new(verts[i].x, verts[i].y, 0.0).norm();
            assert!(n.dot(radial) > 0.99, "vertex {i}: {n:?} vs {radial:?}");
        }
    }

    #[test]
    fn zebra_alternates_and_draft_marks_undercuts() {
        let n = Vec3::new(0.0, 0.0, 1.0);
        let view = Vec3::new(0.0, 0.0, 1.0);
        let flat = Curvature::default();
        // Across a surface that turns away from the viewer the stripes must
        // alternate: that is the whole point of the mode.
        let shades: Vec<f64> = (0..40)
            .map(|i| {
                let tilt = i as f64 / 40.0;
                let normal = Vec3::new(tilt, 0.0, 1.0).norm();
                analysis_color(AnalysisMode::Zebra { stripes: 8.0 }, normal, view, flat)[0]
            })
            .collect();
        let dark = shades.iter().cloned().fold(f64::INFINITY, f64::min);
        let bright = shades.iter().cloned().fold(0.0f64, f64::max);
        assert!(bright - dark > 0.5, "stripes must alternate: {dark} .. {bright}");

        // An upward face passes a 3 degree draft; a downward one is an undercut.
        let up = analysis_color(AnalysisMode::Draft { angle_deg: 3.0 }, n, view, flat);
        let down = analysis_color(AnalysisMode::Draft { angle_deg: 3.0 }, -n, view, flat);
        assert!(up[1] > up[0] && up[1] > up[2], "upward face is green: {up:?}");
        assert!(down[0] > down[1] && down[0] > down[2], "undercut is red: {down:?}");
    }

    #[test]
    fn the_curvature_ramp_runs_blue_to_red() {
        let n = Vec3::new(0.0, 0.0, 1.0);
        let view = n;
        let flat = analysis_color(AnalysisMode::Curvature { radius: 1.0 }, n, view, Curvature { mean: 0.0, gaussian: 0.0 });
        let tight = analysis_color(AnalysisMode::Curvature { radius: 1.0 }, n, view, Curvature { mean: 5.0, gaussian: 0.0 });
        assert!(flat[2] > flat[0], "flat is blue: {flat:?}");
        assert!(tight[0] > tight[2], "tight is red: {tight:?}");
    }
}
