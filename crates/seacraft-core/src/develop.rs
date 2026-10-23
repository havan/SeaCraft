//! Developable plate expansion: unroll a hull patch into the flat sheet a
//! plate would be cut from.
//!
//! A surface is developable when it can be flattened without stretching —
//! Gaussian curvature `K = 0` everywhere. Ship plating is developable by
//! intent, but a subdivision surface rarely is exactly, so the useful answer is
//! not "yes/no" but **how much the flattened patch refuses to close**.
//!
//! [`unfold`] does the standard edge-joining development: place one face in a
//! plane, then walk the patch breadth-first and lay each neighbour flat across
//! the edge it shares with a face already placed. Every face is placed by an
//! isometry, so the areas are preserved exactly and the strain is not in the
//! faces — it is in the seams. A vertex shared by several faces ends up with
//! one 2D position per face; the diameter of that set is the *gap*: how far the
//! plate would tear or overlap if it were cut flat. [`Unfolded::max_gap_m`] and
//! [`Unfolded::gap_ratio`] report it, and [`Unfolded::developable`] applies the
//! rule of thumb most yards use — under 0.5 % of the plate diagonal closes up
//! with normal fairing allowance.
//!
//! Gaussian curvature comes from [`crate::analysis::curvatures`], the same
//! discrete operator the analysis shading uses, so the numbers agree with the
//! curvature false-colour in the viewport.

use crate::analysis::curvatures;
use crate::math::Vec3;
use serde::Serialize;
use std::collections::HashMap;
use std::fmt::Write as _;

/// Which axis a band is cut across.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum BandAxis {
    /// Across `x`: a band between two stations.
    X,
    /// Across `y`: a band between two buttocks.
    Y,
    /// Across `z`: a band between two waterlines.
    Z,
}

impl BandAxis {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "x" => Some(Self::X),
            "y" => Some(Self::Y),
            "z" => Some(Self::Z),
            _ => None,
        }
    }

    fn of(&self, p: Vec3) -> f64 {
        match self {
            Self::X => p.x,
            Self::Y => p.y,
            Self::Z => p.z,
        }
    }
}

/// One flattened face: its 3D vertex ids and the 2D position of each.
#[derive(Debug, Clone, Serialize)]
pub struct FlatFace {
    pub verts: Vec<usize>,
    /// 2D coordinates, metres, in the plate's own frame.
    pub xy: Vec<(f64, f64)>,
}

/// The result of flattening a patch.
#[derive(Debug, Clone, Serialize)]
pub struct Unfolded {
    pub faces: Vec<FlatFace>,
    /// Total 3D area of the patch, m².
    pub area_3d_m2: f64,
    /// Total flattened area, m². Equal to `area_3d_m2` by construction (every
    /// face is placed by an isometry); a difference means a bug, not a stretch.
    pub area_2d_m2: f64,
    /// Largest seam opening over the patch, m.
    pub max_gap_m: f64,
    /// RMS seam opening, m — what the fairing allowance has to absorb on
    /// average.
    pub rms_gap_m: f64,
    /// `max_gap_m` over the plate diagonal, dimensionless.
    pub gap_ratio: f64,
    /// Mean absolute Gaussian curvature over the patch, 1/m².
    pub mean_abs_gaussian: f64,
    /// `gap_ratio` under the 0.5 % rule of thumb.
    pub developable: bool,
    /// Plate extents in the flat pattern, m.
    pub width_m: f64,
    pub height_m: f64,
}

/// Thickness of the seam opening, relative to the plate diagonal, under which a
/// patch is called developable.
pub const DEVELOPABLE_GAP_RATIO: f64 = 0.005;

/// A patch selection: a face qualifies when its centroid is inside every slab,
/// and (optionally) on the starboard side of the centreline.
///
/// One slab alone gives a band that wraps the whole girth, and a band that
/// wraps is a *ring*: developing it accumulates the ring closure into the seam
/// metric, so a perfectly developable strake reports a gap the size of the
/// circumference. Two slabs — between two waterlines **and** two stations, or
/// between two buttocks and two stations — cut out the simply connected plate a
/// yard would actually order.
#[derive(Debug, Clone, Default)]
pub struct PatchSelection {
    pub slabs: Vec<(BandAxis, f64, f64)>,
    /// Keep only faces with a non-negative centroid `y`.
    pub starboard_only: bool,
}

impl PatchSelection {
    /// One slab across `axis`.
    pub fn band(axis: BandAxis, lo: f64, hi: f64) -> Self {
        Self {
            slabs: vec![(axis, lo, hi)],
            starboard_only: false,
        }
    }

    /// Two slabs, the usual plate: a strake between two waterlines over a run
    /// between two stations.
    pub fn plate(axis: BandAxis, lo: f64, hi: f64, along: BandAxis, from: f64, to: f64) -> Self {
        Self {
            slabs: vec![(axis, lo, hi), (along, from, to)],
            starboard_only: true,
        }
    }
}

/// Faces selected by `sel`.
pub fn select(verts: &[Vec3], faces: &[[usize; 3]], sel: &PatchSelection) -> Vec<[usize; 3]> {
    faces
        .iter()
        .filter(|f| {
            let c = (verts[f[0]] + verts[f[1]] + verts[f[2]]) / 3.0;
            if sel.starboard_only && c.y < 0.0 {
                return false;
            }
            sel.slabs.iter().all(|(axis, lo, hi)| {
                let (lo, hi) = if lo <= hi { (*lo, *hi) } else { (*hi, *lo) };
                let v = axis.of(c);
                v >= lo && v <= hi
            })
        })
        .copied()
        .collect()
}

/// Faces whose centroid lies in the slab `lo..=hi` across `axis`.
///
/// A strake between two waterlines is `BandAxis::Z`; a plate between two
/// buttocks is `BandAxis::Y`. Triangles are returned with their original vertex
/// indices, so the caller keeps the mesh they came from.
pub fn band(
    verts: &[Vec3],
    faces: &[[usize; 3]],
    axis: BandAxis,
    lo: f64,
    hi: f64,
) -> Vec<[usize; 3]> {
    let (lo, hi) = if lo <= hi { (lo, hi) } else { (hi, lo) };
    faces
        .iter()
        .filter(|f| {
            let c = (verts[f[0]] + verts[f[1]] + verts[f[2]]) / 3.0;
            let v = axis.of(c);
            v >= lo && v <= hi
        })
        .copied()
        .collect()
}

/// Flatten `faces` (triangles into `verts`) into a plane.
pub fn unfold(verts: &[Vec3], faces: &[[usize; 3]]) -> Unfolded {
    let mut out = Unfolded {
        faces: Vec::new(),
        area_3d_m2: 0.0,
        area_2d_m2: 0.0,
        max_gap_m: 0.0,
        rms_gap_m: 0.0,
        gap_ratio: 0.0,
        mean_abs_gaussian: 0.0,
        developable: false,
        width_m: 0.0,
        height_m: 0.0,
    };
    if faces.is_empty() {
        return out;
    }

    // Edge -> faces, for the breadth-first walk.
    let mut edge_faces: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (i, f) in faces.iter().enumerate() {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            edge_faces.entry(ordered(a, b)).or_default().push(i);
        }
    }

    let mut placed: Vec<Option<FlatFace>> = vec![None; faces.len()];
    let mut queue = std::collections::VecDeque::new();

    // Seed with the **largest** face, not the first one. A degenerate face (two
    // coincident vertices, as a pole fan produces) has no direction, so seeding
    // on it collapses the whole pattern to a point — every later placement
    // inherits the zero frame.
    let mut seed = 0usize;
    let mut seed_area = -1.0;
    for (i, f) in faces.iter().enumerate() {
        let a = 0.5
            * (verts[f[1]] - verts[f[0]])
                .cross(verts[f[2]] - verts[f[0]])
                .length();
        if a > seed_area {
            seed_area = a;
            seed = i;
        }
    }
    if seed_area <= 1e-12 {
        return out; // nothing with area to unfold
    }
    {
        let f = faces[seed];
        let (a, b, c) = (verts[f[0]], verts[f[1]], verts[f[2]]);
        let e = (b - a).norm();
        let n = (b - a).cross(c - a).norm();
        let g = n.cross(e);
        let xy: Vec<(f64, f64)> = [a, b, c]
            .iter()
            .map(|p| ((*p - a).dot(e), (*p - a).dot(g)))
            .collect();
        placed[seed] = Some(FlatFace {
            verts: f.to_vec(),
            xy,
        });
        queue.push_back(seed);
    }

    while let Some(parent) = queue.pop_front() {
        let Some(parent_face) = placed[parent].clone() else {
            continue;
        };
        let pf = faces[parent];
        for k in 0..3 {
            let (ia, ib) = (pf[k], pf[(k + 1) % 3]);
            let Some(neighbours) = edge_faces.get(&ordered(ia, ib)) else {
                continue;
            };
            for &child in neighbours {
                if child == parent || placed[child].is_some() {
                    continue;
                }
                // 2D positions of the shared edge, from the parent.
                let pa = parent_face
                    .verts
                    .iter()
                    .position(|v| *v == ia)
                    .map(|i| parent_face.xy[i]);
                let pb = parent_face
                    .verts
                    .iter()
                    .position(|v| *v == ib)
                    .map(|i| parent_face.xy[i]);
                let (Some(pa), Some(pb)) = (pa, pb) else {
                    continue;
                };

                let cf = faces[child];
                let (a3, b3) = (verts[ia], verts[ib]);
                let e3 = (b3 - a3).norm();
                let n3 = (verts[cf[1]] - verts[cf[0]])
                    .cross(verts[cf[2]] - verts[cf[0]])
                    .norm();
                let g3 = n3.cross(e3);

                let e2x = pb.0 - pa.0;
                let e2y = pb.1 - pa.1;
                let len = (e2x * e2x + e2y * e2y).sqrt().max(1e-12);
                let e2 = (e2x / len, e2y / len);
                let g2 = (-e2.1, e2.0);

                // Local 3D coordinates of the child's vertices in the edge frame.
                let mut xy: Vec<(f64, f64)> = Vec::with_capacity(3);
                for &vi in &cf {
                    let d = verts[vi] - a3;
                    let (u, w) = (d.dot(e3), d.dot(g3));
                    xy.push((pa.0 + e2.0 * u + g2.0 * w, pa.1 + e2.1 * u + g2.1 * w));
                }
                // The child must land on the far side of the shared edge from
                // the parent's centroid, or the plate folds back over itself.
                let side = |p: (f64, f64)| e2.0 * (p.1 - pa.1) - e2.1 * (p.0 - pa.0);
                let parent_side = side(parent_face_centroid(&parent_face));
                let child_side = side(centroid2(&xy));
                if parent_side * child_side > 0.0 {
                    // Mirror the child across the edge line.
                    for p in &mut xy {
                        let dx = p.0 - pa.0;
                        let dy = p.1 - pa.1;
                        let along = dx * e2.0 + dy * e2.1;
                        *p = (
                            pa.0 + e2.0 * along - g2.0 * (dx * g2.0 + dy * g2.1),
                            pa.1 + e2.1 * along - g2.1 * (dx * g2.0 + dy * g2.1),
                        );
                    }
                }
                placed[child] = Some(FlatFace {
                    verts: cf.to_vec(),
                    xy,
                });
                queue.push_back(child);
            }
        }
    }

    // ---- metrics --------------------------------------------------------
    let mut per_vertex: HashMap<usize, Vec<(f64, f64)>> = HashMap::new();
    for (i, face) in faces.iter().enumerate() {
        let Some(f) = placed[i].as_ref() else {
            continue;
        };
        for (k, &vi) in face.iter().enumerate() {
            per_vertex.entry(vi).or_default().push(f.xy[k]);
        }
        let a = verts[face[0]];
        let b = verts[face[1]];
        let c = verts[face[2]];
        out.area_3d_m2 += 0.5 * (b - a).cross(c - a).length();
        let p = &f.xy;
        out.area_2d_m2 += 0.5
            * ((p[1].0 - p[0].0) * (p[2].1 - p[0].1) - (p[2].0 - p[0].0) * (p[1].1 - p[0].1)).abs();
        out.faces.push(f.clone());
    }

    let mut gap_sum_sq = 0.0;
    let mut gap_count = 0;
    for positions in per_vertex.values() {
        if positions.len() < 2 {
            continue;
        }
        let mut worst = 0.0_f64;
        for i in 0..positions.len() {
            for j in i + 1..positions.len() {
                let dx = positions[i].0 - positions[j].0;
                let dy = positions[i].1 - positions[j].1;
                worst = worst.max((dx * dx + dy * dy).sqrt());
            }
        }
        out.max_gap_m = out.max_gap_m.max(worst);
        gap_sum_sq += worst * worst;
        gap_count += 1;
    }
    out.rms_gap_m = if gap_count > 0 {
        (gap_sum_sq / gap_count as f64).sqrt()
    } else {
        0.0
    };

    let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
    let (mut maxx, mut maxy) = (f64::NEG_INFINITY, f64::NEG_INFINITY);
    for f in &out.faces {
        for p in &f.xy {
            minx = minx.min(p.0);
            maxx = maxx.max(p.0);
            miny = miny.min(p.1);
            maxy = maxy.max(p.1);
        }
    }
    out.width_m = (maxx - minx).max(0.0);
    out.height_m = (maxy - miny).max(0.0);
    let diagonal = (out.width_m * out.width_m + out.height_m * out.height_m).sqrt();
    out.gap_ratio = if diagonal > 1e-9 {
        out.max_gap_m / diagonal
    } else {
        0.0
    };

    // Gaussian curvature over the patch's own vertices.
    let mut used: Vec<usize> = per_vertex.keys().copied().collect();
    used.sort_unstable();
    if !used.is_empty() {
        let remap: HashMap<usize, usize> = used.iter().enumerate().map(|(i, v)| (*v, i)).collect();
        let sub_verts: Vec<Vec3> = used.iter().map(|v| verts[*v]).collect();
        let sub_faces: Vec<Vec<usize>> = out
            .faces
            .iter()
            .map(|f| {
                f.verts
                    .iter()
                    .filter_map(|v| remap.get(v).copied())
                    .collect()
            })
            .filter(|f: &Vec<usize>| f.len() == 3)
            .collect();
        // Boundary vertices are excluded: their angle deficit is a property of
        // the patch edge, not of the surface (a strip of a cylinder is flat, but
        // its rim reports K of hundreds of 1/m2 because the incident area is a
        // sliver). `curvatures` marks them from one-sided edges, same test.
        let mut edge_uses: HashMap<(usize, usize), usize> = HashMap::new();
        for f in &sub_faces {
            for k in 0..f.len() {
                *edge_uses
                    .entry(ordered(f[k], f[(k + 1) % f.len()]))
                    .or_insert(0) += 1;
            }
        }
        let mut boundary: Vec<bool> = vec![false; sub_verts.len()];
        for ((a, b), uses) in &edge_uses {
            if *uses == 1 {
                boundary[*a] = true;
                boundary[*b] = true;
            }
        }
        let all = curvatures(&sub_verts, &sub_faces);
        let interior: Vec<f64> = all
            .iter()
            .enumerate()
            .filter(|(i, _)| !boundary[*i])
            .map(|(_, c)| c.gaussian.abs())
            .filter(|k| k.is_finite())
            .collect();
        let sample = if interior.is_empty() {
            all.iter()
                .map(|c| c.gaussian.abs())
                .filter(|k| k.is_finite())
                .collect::<Vec<f64>>()
        } else {
            interior
        };
        if !sample.is_empty() {
            out.mean_abs_gaussian = sample.iter().sum::<f64>() / sample.len() as f64;
        }
    }

    out.developable = out.gap_ratio < DEVELOPABLE_GAP_RATIO && out.faces.len() > 1;
    out
}

fn ordered(a: usize, b: usize) -> (usize, usize) {
    if a < b {
        (a, b)
    } else {
        (b, a)
    }
}

fn centroid2(xy: &[(f64, f64)]) -> (f64, f64) {
    let n = xy.len().max(1) as f64;
    (
        xy.iter().map(|p| p.0).sum::<f64>() / n,
        xy.iter().map(|p| p.1).sum::<f64>() / n,
    )
}

fn parent_face_centroid(f: &FlatFace) -> (f64, f64) {
    centroid2(&f.xy)
}

/// The flat pattern as a standalone SVG: every face drawn, the patch outline
/// stroked, and the seam gap and Gaussian curvature in the caption.
pub fn svg(unfolded: &Unfolded, title: &str) -> String {
    let mut out = String::with_capacity(4096);
    let pad = 40.0;
    let scale = {
        let longest = unfolded.width_m.max(unfolded.height_m).max(0.01);
        640.0 / longest
    };
    // The caption is longer than most plates are wide, so the canvas has to fit
    // both the drawing and the text (11 px per character, roughly).
    let caption_chars = title.chars().count().max(96) as f64;
    let w = (unfolded.width_m * scale + 2.0 * pad)
        .max(24.0 + caption_chars * 6.6)
        .min(1600.0);
    let h = unfolded.height_m * scale + 2.0 * pad + 56.0;
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">"
    );
    let _ = writeln!(out, "<title>{}</title>", esc(title));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str(
        "<style>\n.face{fill:#cfe3f7;fill-opacity:0.55;stroke:#7aa7d0;stroke-width:0.4}\n",
    );
    out.push_str(".outline{fill:none;stroke:#123f66;stroke-width:1.2}\n");
    out.push_str(".label{font:12px ui-sans-serif,system-ui,sans-serif;fill:#222}\n</style>\n");

    // Local extents, so the drawing starts at the margin.
    let (mut minx, mut miny) = (f64::INFINITY, f64::INFINITY);
    for f in &unfolded.faces {
        for p in &f.xy {
            minx = minx.min(p.0);
            miny = miny.min(p.1);
        }
    }
    if !minx.is_finite() {
        minx = 0.0;
    }
    if !miny.is_finite() {
        miny = 0.0;
    }
    // SVG y grows downwards; flip so the plate reads like a drawing.
    let map = |p: (f64, f64)| {
        (
            pad + (p.0 - minx) * scale,
            pad + (unfolded.height_m - (p.1 - miny)) * scale,
        )
    };

    // Outline: edges used by exactly one face.
    let mut edge_use: HashMap<(usize, usize), usize> = HashMap::new();
    for f in &unfolded.faces {
        for k in 0..f.verts.len() {
            *edge_use
                .entry(ordered(f.verts[k], f.verts[(k + 1) % f.verts.len()]))
                .or_insert(0) += 1;
        }
    }
    for f in &unfolded.faces {
        let pts: Vec<String> =
            f.xy.iter()
                .map(|p| {
                    let (x, y) = map(*p);
                    format!("{x:.2},{y:.2}")
                })
                .collect();
        let _ = writeln!(
            out,
            "<polygon class=\"face\" points=\"{}\"/>",
            pts.join(" ")
        );
    }
    for f in &unfolded.faces {
        for k in 0..f.verts.len() {
            let key = ordered(f.verts[k], f.verts[(k + 1) % f.verts.len()]);
            if edge_use.get(&key).copied().unwrap_or(0) != 1 {
                continue;
            }
            let (x1, y1) = map(f.xy[k]);
            let (x2, y2) = map(f.xy[(k + 1) % f.verts.len()]);
            let _ = writeln!(
                out,
                "<line class=\"outline\" x1=\"{x1:.2}\" y1=\"{y1:.2}\" x2=\"{x2:.2}\" y2=\"{y2:.2}\"/>"
            );
        }
    }
    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"{pad:.0}\" y=\"24\">{}</text>",
        esc(title)
    );
    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"{pad:.0}\" y=\"{:.0}\">{:.3} x {:.3} m   gap {:.1} mm ({:.3} %)   |K| {:.4} 1/m2   {}</text>",
        h - 12.0,
        unfolded.width_m,
        unfolded.height_m,
        unfolded.max_gap_m * 1000.0,
        unfolded.gap_ratio * 100.0,
        unfolded.mean_abs_gaussian,
        if unfolded.developable { "developable" } else { "not developable" }
    );
    out.push_str("</svg>\n");
    out
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A flat strip: `n` quads in the x–y plane, split into triangles.
    fn plate() -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let (nx, ny) = (6, 3);
        let mut verts = Vec::new();
        for j in 0..=ny {
            for i in 0..=nx {
                verts.push(Vec3::new(i as f64 * 0.5, j as f64 * 0.4, 0.0));
            }
        }
        let idx = |i: usize, j: usize| j * (nx + 1) + i;
        let mut faces = Vec::new();
        for j in 0..ny {
            for i in 0..nx {
                let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
                faces.push([a, b, c]);
                faces.push([a, c, d]);
            }
        }
        (verts, faces)
    }

    /// A cylinder of radius `r` about the x axis, as a strip of quad triangles.
    fn cylinder(r: f64, length: f64, around: usize, along: usize) -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let mut verts = Vec::new();
        for j in 0..=along {
            let x = length * j as f64 / along as f64;
            for i in 0..=around {
                let t = std::f64::consts::PI * i as f64 / around as f64; // half cylinder
                verts.push(Vec3::new(x, r * t.cos(), r * t.sin()));
            }
        }
        let idx = |i: usize, j: usize| j * (around + 1) + i;
        let mut faces = Vec::new();
        for j in 0..along {
            for i in 0..around {
                let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
                faces.push([a, b, c]);
                faces.push([a, c, d]);
            }
        }
        (verts, faces)
    }

    /// A spherical cap, which cannot be flattened without tearing.
    fn sphere(r: f64, rings: usize, sectors: usize) -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let mut verts = Vec::new();
        // phi from 0 (pole) to 0.7 rad: a cap.
        for j in 0..=rings {
            let phi = 0.7 * j as f64 / rings as f64;
            for i in 0..sectors {
                let theta = std::f64::consts::TAU * i as f64 / sectors as f64;
                verts.push(Vec3::new(
                    r * phi.sin() * theta.cos(),
                    r * phi.sin() * theta.sin(),
                    r * phi.cos(),
                ));
            }
        }
        let idx = |i: usize, j: usize| j * sectors + i % sectors;
        let mut faces = Vec::new();
        for j in 0..rings {
            for i in 0..sectors {
                let (a, b, c, d) = (idx(i, j), idx(i + 1, j), idx(i + 1, j + 1), idx(i, j + 1));
                faces.push([a, b, c]);
                faces.push([a, c, d]);
            }
        }
        (verts, faces)
    }

    #[test]
    fn a_flat_plate_flattens_exactly() {
        let (v, f) = plate();
        let u = unfold(&v, &f);
        assert_eq!(u.faces.len(), f.len());
        assert!(u.max_gap_m < 1e-12, "gap {}", u.max_gap_m);
        assert!((u.area_2d_m2 - u.area_3d_m2).abs() < 1e-12);
        assert!((u.area_3d_m2 - 3.0 * 1.2).abs() < 1e-9, "{}", u.area_3d_m2);
        assert!(u.developable);
    }

    #[test]
    fn a_cylinder_unrolls_without_tearing() {
        // A cylinder is developable: K = 0, so the flattened strip must close.
        let (v, f) = cylinder(1.0, 3.0, 24, 6);
        let u = unfold(&v, &f);
        assert!(
            u.max_gap_m < 1e-9,
            "gap {} m on a developable surface",
            u.max_gap_m
        );
        assert!(u.developable, "gap ratio {}", u.gap_ratio);
        // The flat pattern is the arc length by the axial length.
        let arc = std::f64::consts::PI * 1.0;
        assert!(
            (u.width_m - arc).abs() < 0.02 || (u.height_m - arc).abs() < 0.02,
            "extents {} x {} for an arc of {arc:.3}",
            u.width_m,
            u.height_m
        );
        assert!(
            u.mean_abs_gaussian < 0.05,
            "|K| {} on a cylinder",
            u.mean_abs_gaussian
        );
    }

    #[test]
    fn a_spherical_cap_cannot_be_flattened() {
        let r = 1.0;
        let (v, f) = sphere(r, 8, 16);
        let u = unfold(&v, &f);
        assert!(
            !u.developable,
            "a sphere cap reported as developable: {}",
            u.gap_ratio
        );
        assert!(
            u.max_gap_m > 0.02,
            "gap {} m is too small for a sphere cap",
            u.max_gap_m
        );
        // Areas are still preserved: the strain is in the seams, not the faces.
        assert!((u.area_2d_m2 - u.area_3d_m2).abs() < 1e-9);
        // Discrete Gaussian curvature of a unit sphere is about 1 (1/m²) — the
        // angle deficit over the patch's area.
        assert!(
            u.mean_abs_gaussian > 0.5 && u.mean_abs_gaussian < 2.0,
            "|K| {} 1/m2 on a unit sphere",
            u.mean_abs_gaussian
        );
    }

    #[test]
    fn a_band_selects_a_strake() {
        let (v, f) = cylinder(1.0, 3.0, 16, 8);
        // Radial band: z between 0.3 and 0.9 keeps the upper strake.
        let b = band(&v, &f, BandAxis::Z, 0.3, 0.9);
        assert!(
            !b.is_empty() && b.len() < f.len(),
            "{} of {}",
            b.len(),
            f.len()
        );
        for face in &b {
            let c = (v[face[0]] + v[face[1]] + v[face[2]]) / 3.0;
            assert!(c.z >= 0.3 - 1e-9 && c.z <= 0.9 + 1e-9, "{c:?}");
        }
        // Reversed limits are accepted.
        assert_eq!(band(&v, &f, BandAxis::Z, 0.9, 0.3).len(), b.len());
        assert_eq!(BandAxis::parse("Z"), Some(BandAxis::Z));
        assert_eq!(BandAxis::parse("w"), None);
        // An empty selection unfolds to an empty, non-developable report.
        let none = unfold(&v, &[]);
        assert_eq!(none.faces.len(), 0);
        assert!(!none.developable);
        assert_eq!(none.max_gap_m, 0.0);
    }

    #[test]
    fn a_plate_selection_is_one_side_and_simply_connected() {
        let (v, f) = cylinder(1.0, 3.0, 16, 8);
        let sel = PatchSelection::plate(BandAxis::Z, 0.3, 0.9, BandAxis::X, 0.5, 2.0);
        let picked = select(&v, &f, &sel);
        assert!(!picked.is_empty() && picked.len() < f.len());
        for face in &picked {
            let c = (v[face[0]] + v[face[1]] + v[face[2]]) / 3.0;
            assert!(c.z >= 0.3 - 1e-9 && c.z <= 0.9 + 1e-9, "{c:?}");
            assert!(c.x >= 0.5 - 1e-9 && c.x <= 2.0 + 1e-9, "{c:?}");
            assert!(c.y >= -1e-9, "port faces must be cut away: {c:?}");
        }
        // A band that wraps the whole girth develops the ring closure into the
        // gap; the two-slab plate does not.
        let wrap = unfold(&v, &band(&v, &f, BandAxis::Z, 0.3, 0.9));
        let plate = unfold(&v, &picked);
        assert!(
            plate.gap_ratio <= wrap.gap_ratio + 1e-12,
            "plate {} vs wrap {}",
            plate.gap_ratio,
            wrap.gap_ratio
        );
        assert!(
            plate.developable,
            "a cylinder strake must be developable: {}",
            plate.gap_ratio
        );
    }

    #[test]
    fn svg_has_the_outline_and_caption() {
        let (v, f) = plate();
        let page = svg(&unfold(&v, &f), "plate <A>");
        assert!(page.starts_with("<?xml"));
        assert!(page.trim_end().ends_with("</svg>"));
        assert!(page.contains("plate &lt;A&gt;"));
        assert!(!page.contains("NaN") && !page.contains("inf"));
        assert!(page.matches("<polygon").count() >= 1);
        assert!(page.matches("<line").count() >= 4, "the patch outline");
        assert!(page.contains("developable"));
    }
}
