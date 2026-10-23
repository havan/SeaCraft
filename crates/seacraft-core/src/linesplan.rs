//! Linesplan: stations, buttocks and waterlines as SVG or DXF.
//!
//! FREE!ship derives its linesplan from the same intersection machinery used
//! for hydrostatics: cut the refined hull with a set of planes, chain the
//! resulting segments into polylines and project them onto the three
//! orthographic views. This module does exactly that and lays the curves out
//! as a conventional three-view drawing (profile, plan, body plan), either as
//! a standalone SVG or as an AutoCAD 2000 DXF with named layers.

use crate::dxf::{self, DxfEntity, DxfLayer, DxfShape, Vec2};
use crate::math::{Plane, Vec3};
use crate::mesh::Surface;
use crate::subdiv::{evaluate, SubdivisionMode, SubdivisionOptions};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy)]
pub struct LinesplanOptions {
    pub stations: usize,
    pub buttocks: usize,
    pub waterlines: usize,
    pub level: u8,
    pub mode: SubdivisionMode,
    /// Draw porcupine combs along the curves of every view.
    pub combs: bool,
    /// Metres a curvature of 1/m draws: the comb's gain, not a physical scale.
    /// Spikes are clamped to 12 % of the view so a sharp bilge cannot swamp the
    /// drawing.
    pub comb_scale: f64,
}

impl Default for LinesplanOptions {
    fn default() -> Self {
        Self {
            stations: 11,
            buttocks: 5,
            waterlines: 7,
            level: 3,
            mode: SubdivisionMode::Loop,
            combs: false,
            comb_scale: 0.6,
        }
    }
}

/// A polyline in one of the orthographic views: its 2D view coordinates and
/// the 3D curve it came from.
struct Curve {
    points: Vec<(f64, f64)>,
    points3: Vec<Vec3>,
    closed: bool,
    /// The cutting plane the 2D view coordinates live in, so a 2D point can be
    /// lifted back to 3D: `origin + u * pu + v * pv` with `(u, v) = view_basis(plane)`
    /// and `origin = plane.n * (-plane.d)`.
    plane: Plane,
}

/// A 3D intersection curve and its optional 3D curvature-comb spikes `(base, tip)`.
#[derive(Debug, Clone)]
pub struct CurveWithCombs {
    pub points: Vec<Vec3>,
    pub closed: bool,
    pub spikes: Vec<(Vec3, Vec3)>,
}

/// All 3D curves and optional curvature combs for the hull.
#[derive(Debug, Clone)]
pub struct LinesplanCurves3d {
    pub stations: Vec<CurveWithCombs>,
    pub buttocks: Vec<CurveWithCombs>,
    pub waterlines: Vec<CurveWithCombs>,
}

/// The stations, buttocks and waterlines as 3D curves, with their curvature
/// combs lifted back into 3D when `opts.combs` is set.
///
/// The comb sampling matches [`view_combs`] exactly (same gain, same spacing,
/// same clamp), so a GUI viewport draws the same porcupine the drawing does;
/// only the coordinates differ — 3D here, view-plane 2D in the SVG/DXF.
pub fn curves_3d(surface: &Surface, opts: LinesplanOptions) -> LinesplanCurves3d {
    let cut = cut_all(surface, opts);
    // Each view's comb extent: the body plan spans the beam, the profile the
    // depth, the plan the length/beam footprint.
    let b = (cut.max.y - cut.min.y).max(0.1);
    let d = (cut.max.z - cut.min.z).max(0.1);

    let group = |curves: &[Curve], extent: f64| -> Vec<CurveWithCombs> {
        let extent = extent.max(1e-6);
        let max_len = 0.12 * extent;
        let spacing = 0.02 * extent;
        curves
            .iter()
            .map(|c| {
                let spikes = if opts.combs {
                    let len = arc_length(&c.points);
                    let step = if len > 0.0 {
                        spacing.min(len / 8.0)
                    } else {
                        spacing
                    };
                    let raw = comb_spikes(&c.points, opts.comb_scale, max_len, step);
                    if raw.len() < 2 {
                        Vec::new()
                    } else {
                        let (u, v) = view_basis(&c.plane);
                        let origin = c.plane.n * (-c.plane.d);
                        let to3 = |(pu, pv): P2| origin + u * pu + v * pv;
                        raw.into_iter()
                            .map(|(base, tip)| (to3(base), to3(tip)))
                            .collect()
                    }
                } else {
                    Vec::new()
                };
                CurveWithCombs {
                    points: c.points3.clone(),
                    closed: c.closed,
                    spikes,
                }
            })
            .collect()
    };

    LinesplanCurves3d {
        stations: group(&cut.stations, b),
        buttocks: group(&cut.buttocks, d),
        waterlines: group(&cut.waterlines, b),
    }
}

/// The curves of every view plus the model's extents.
struct Cut {
    stations: Vec<Curve>,
    buttocks: Vec<Curve>,
    waterlines: Vec<Curve>,
    /// The cut at the project's design draft, for the waterplane.
    waterplane: Option<Curve>,
    min: Vec3,
    max: Vec3,
}

/// Triangles of every visible layer, mirrored where the layer is symmetric.
fn visible_triangles(
    surface: &Surface,
    subdiv: SubdivisionOptions,
) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    let mut mesh = evaluate(surface, subdiv);
    let sym: Vec<bool> = surface.layers.iter().map(|l| l.symmetric).collect();
    mesh.mirror_symmetric(&sym);
    let mut tris = Vec::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        if !surface.layers[mesh.face_layer[fi]].visible {
            continue;
        }
        for i in 1..f.len().saturating_sub(1) {
            tris.push([f[0], f[i], f[i + 1]]);
        }
    }
    (mesh.verts, tris)
}

/// The drawing basis of a cutting plane: `Plane::basis` with `v` turned to
/// point up (or to starboard for a waterline). For a buttock plane (normal
/// `+y`) `Plane::basis` gives `v = -z`, which drew the profile upside down
/// below its frame; the hydrostatic moments rely on `Plane::basis` as it is,
/// so the flip lives here.
fn view_basis(plane: &Plane) -> (Vec3, Vec3) {
    let (u, v) = plane.basis();
    if v.z < -1e-9 || (v.z.abs() <= 1e-9 && v.y < 0.0) {
        (u, v * -1.0)
    } else {
        (u, v)
    }
}

/// Curves where `plane` cuts the mesh, projected into the plane's view basis.
fn cut(verts: &[Vec3], tris: &[[usize; 3]], plane: &Plane, tol: f64) -> Vec<Curve> {
    let segs = crate::hydro::plane_segments(verts, tris, plane);
    let paths = crate::hydro::chain_paths(&segs, tol);
    let (u, v) = view_basis(plane);
    paths
        .into_iter()
        .filter(|(p, _)| p.len() >= 2)
        .map(|(p, closed)| Curve {
            points: p.iter().map(|q| (u.dot(*q), v.dot(*q))).collect(),
            points3: p,
            closed,
            plane: *plane,
        })
        .collect()
}

/// Cut the hull once for every view.
fn cut_all(surface: &Surface, opts: LinesplanOptions) -> Cut {
    let subdiv = SubdivisionOptions {
        level: opts.level,
        mode: opts.mode,
    };
    let (verts, tris) = visible_triangles(surface, subdiv);
    let (min, max) = surface.extents();
    let l = (max.x - min.x).max(0.1);
    let b = (max.y - min.y).max(0.1);
    let tol = l.max(b) * 1e-6;

    // Body plan: stations at x = const, viewed from the bow (u = y, v = z).
    let mut stations: Vec<Curve> = Vec::new();
    for i in 0..opts.stations.max(2) {
        let t = i as f64 / (opts.stations.max(2) - 1) as f64;
        let x = min.x + (max.x - min.x) * t;
        let plane = Plane::new(Vec3::new(1.0, 0.0, 0.0), -x);
        stations.extend(cut(&verts, &tris, &plane, tol));
    }
    // Profile: buttocks at y = const, viewed from starboard (u = x, v = z).
    let mut buttocks: Vec<Curve> = Vec::new();
    for i in 0..opts.buttocks.max(2) {
        let t = i as f64 / (opts.buttocks.max(2) - 1) as f64;
        let y = min.y + (max.y - min.y) * t;
        let plane = Plane::new(Vec3::new(0.0, 1.0, 0.0), -y);
        buttocks.extend(cut(&verts, &tris, &plane, tol));
    }
    // Plan: waterlines at z = const, viewed from above (u = x, v = y).
    let mut waterlines: Vec<Curve> = Vec::new();
    for i in 0..opts.waterlines.max(2) {
        let t = i as f64 / (opts.waterlines.max(2) - 1) as f64;
        let z = min.z + (max.z - min.z) * t;
        let plane = Plane::new(Vec3::new(0.0, 0.0, 1.0), -z);
        waterlines.extend(cut(&verts, &tris, &plane, tol));
    }
    // The design waterline, drawn highlighted in the plan view.
    let waterplane = if surface.settings.draft > 0.0 {
        let z = surface.lowest_hydrostatics_point() + surface.settings.draft;
        let plane = Plane::new(Vec3::new(0.0, 0.0, 1.0), -z);
        cut(&verts, &tris, &plane, tol)
            .into_iter()
            .max_by_key(|c| c.points.len())
    } else {
        None
    };

    Cut {
        stations,
        buttocks,
        waterlines,
        waterplane,
        min,
        max,
    }
}

/// A point in a view's own 2D coordinates.
type P2 = (f64, f64);
/// A comb spike: the point on the curve and the end of its spike.
type Spike = (P2, P2);

/// Circumcircle curvature of the polyline at `points[i]`, in 1/m.
///
/// `k = 4A / (ab·bc·ca)` for the triangle of the two neighbouring segments;
/// the cross product gives twice the area, so `k = 2|cross| / (ab·bc·ca)`.
/// A straight run (or a repeated point) is `0`.
fn curvature_at(points: &[P2], i: usize) -> f64 {
    if i == 0 || i + 1 >= points.len() {
        return 0.0;
    }
    let (a, b, c) = (points[i - 1], points[i], points[i + 1]);
    let len = |p: (f64, f64), q: (f64, f64)| ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
    let denom = len(a, b) * len(b, c) * len(c, a);
    if denom < 1e-12 {
        return 0.0;
    }
    let cross = (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0);
    2.0 * cross.abs() / denom
}

/// Porcupine comb for one drawn curve: a spike at every interior point whose
/// length is `curvature * scale`, pointing to the convex side.
///
/// The convex side is opposite the discrete second difference `a - 2b + c`,
/// which points at the centre of curvature. The pair is (base, tip), both in
/// view coordinates.
fn comb_spikes(points: &[P2], scale: f64, max_len: f64, spacing: f64) -> Vec<Spike> {
    if points.len() < 3 || scale <= 0.0 {
        return Vec::new();
    }
    // Curvature read at the comb's own sampling interval, not at every refined
    // vertex: the refined mesh is 5 mm apart, and a circumcircle through three
    // such points measures the mesh's discretisation, not the hull's fairness.
    let sampled = resample(points, spacing);
    if sampled.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::new();
    for i in 1..sampled.len() - 1 {
        let (a, b, c) = (sampled[i - 1], sampled[i], sampled[i + 1]);
        let dx = a.0 + c.0 - 2.0 * b.0;
        let dy = a.1 + c.1 - 2.0 * b.1;
        let bend = (dx * dx + dy * dy).sqrt();
        if bend < 1e-12 {
            continue;
        }
        let h = (curvature_at(&sampled, i) * scale).min(max_len);
        if !h.is_finite() {
            continue;
        }
        out.push((b, (b.0 - dx / bend * h, b.1 - dy / bend * h)));
    }
    out
}

/// Walk a polyline and return points every `spacing` metres of arc length,
/// with the original end point kept so the comb reaches the end of the curve.
fn resample(points: &[P2], spacing: f64) -> Vec<P2> {
    let mut out = vec![points[0]];
    if spacing <= 0.0 || points.len() < 2 {
        out.extend_from_slice(&points[1..]);
        return out;
    }
    let mut carry = 0.0;
    for w in points.windows(2) {
        let (a, b) = (w[0], w[1]);
        let seg = (b.0 - a.0).hypot(b.1 - a.1);
        if seg < 1e-12 {
            continue;
        }
        let mut pos = spacing - carry;
        while pos <= seg {
            let f = pos / seg;
            out.push((a.0 + (b.0 - a.0) * f, a.1 + (b.1 - a.1) * f));
            pos += spacing;
        }
        carry = seg - (pos - spacing);
    }
    if let Some(last) = points.last() {
        let prev = *out.last().unwrap();
        if (last.0 - prev.0).hypot(last.1 - prev.1) > 0.5 * spacing {
            out.push(*last);
        }
    }
    out
}

/// Comb spikes for every curve of one view, at a fixed gain.
///
/// A fixed gain is what makes combs readable: the same curvature draws the same
/// spike on every station, so a curve that bulges out of the family is the
/// unfair one. Normalising per view instead would rescale the whole view
/// whenever one point is sharp, and the combs collapse to nothing.
fn view_combs(curves: &[Curve], scale: f64, extent: f64) -> Vec<Vec<Spike>> {
    let extent = extent.max(1e-6);
    let max_len = 0.12 * extent;
    // ~50 spikes across the view: fine enough for a fairing check, coarse
    // enough that the spikes do not merge into a band. A short curve gets a
    // finer interval of its own so it still shows eight or so spikes.
    let spacing = 0.02 * extent;
    curves
        .iter()
        .map(|c| {
            let len = arc_length(&c.points);
            let step = if len > 0.0 {
                spacing.min(len / 8.0)
            } else {
                spacing
            };
            comb_spikes(&c.points, scale, max_len, step)
        })
        .filter(|s| s.len() >= 2)
        .collect()
}

/// Total length of a polyline.
fn arc_length(points: &[P2]) -> f64 {
    points
        .windows(2)
        .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
        .sum()
}

/// SVG paths for one comb: the spikes, and the line through their tips.
fn comb_svg(spikes: &[Spike], map: &dyn Fn(f64, f64) -> P2) -> (String, String) {
    let mut lines = String::new();
    let mut tips = String::new();
    for (i, (base, tip)) in spikes.iter().enumerate() {
        let (p0, p1) = (map(base.0, base.1), map(tip.0, tip.1));
        let _ = write!(lines, "M{:.2},{:.2}L{:.2},{:.2}", p0.0, p0.1, p1.0, p1.1);
        let _ = if i == 0 {
            write!(tips, "M{:.2},{:.2}", p1.0, p1.1)
        } else {
            write!(tips, "L{:.2},{:.2}", p1.0, p1.1)
        };
    }
    (lines, tips)
}

/// Extend a closed curve back to its first point so the SVG path closes.
fn svg_path(c: &Curve, map: &dyn Fn(f64, f64) -> (f64, f64)) -> String {
    let mut d = String::new();
    for (i, p) in c.points.iter().enumerate() {
        let (x, y) = map(p.0, p.1);
        if i == 0 {
            let _ = write!(d, "M{x:.2},{y:.2}");
        } else {
            let _ = write!(d, "L{x:.2},{y:.2}");
        }
    }
    if c.closed {
        d.push('Z');
    }
    d
}

/// Render the linesplan as a standalone SVG document.
pub fn svg(surface: &Surface, opts: LinesplanOptions) -> String {
    let cut = cut_all(surface, opts);
    let (min, max) = (cut.min, cut.max);
    let l = (max.x - min.x).max(0.1);
    let b = (max.y - min.y).max(0.1);
    let d = (max.z - min.z).max(0.1);

    // Panel geometry in millimetres (1 m = 1 unit here; scaled for print).
    let scale = 900.0 / l.max(2.0 * b);
    let margin = 40.0;
    // `b` is the full (mirrored) beam. The plan and body frames share this
    // height, so it must hold the beam and, in the body plan, the depth.
    let bw = (b * 1.1).max(d * 1.05) * scale;
    let lw = l * scale;
    let dw = d * scale;
    let top = margin;
    // The plan and body plan sit under the profile, below its comb label.
    let plan_y = top + dw + 60.0;
    let width = margin * 2.0 + lw + bw + 40.0;
    let height = plan_y + bw + margin + 20.0;

    let mut out = String::with_capacity(8192);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\">"
    );
    let _ = writeln!(
        out,
        "<title>{} - linesplan</title>",
        html_escape(&surface.settings.name)
    );
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<style>\n");
    out.push_str(".frame{fill:none;stroke:#111;stroke-width:0.8}\n");
    out.push_str(".waterline{fill:none;stroke:#2a6fdb;stroke-width:0.7}\n");
    out.push_str(".station{fill:none;stroke:#111;stroke-width:0.7}\n");
    out.push_str(".buttock{fill:none;stroke:#c0392b;stroke-width:0.7}\n");
    out.push_str(".centerline{fill:none;stroke:#888;stroke-width:0.5;stroke-dasharray:6 4}\n");
    out.push_str(".comb{fill:none;stroke:#2b8a3e;stroke-width:0.8}\n");
    out.push_str(".spike{fill:none;stroke:#7bbf8a;stroke-width:0.25}\n");
    out.push_str(".label{font:11px ui-sans-serif,system-ui,sans-serif;fill:#333}\n");
    out.push_str(".title{font:13px ui-sans-serif,system-ui,sans-serif;fill:#111}\n");
    out.push_str("</style>\n");

    // ---- profile (buttocks) ------------------------------------------
    let _ = writeln!(out, "<g id=\"profile\">");
    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{margin:.1}\" y=\"{:.1}\">Profile - buttocks</text>",
        top - 12.0
    );
    let _ = writeln!(
        out,
        "<rect class=\"frame\" x=\"{margin:.1}\" y=\"{top:.1}\" width=\"{lw:.1}\" height=\"{dw:.1}\"/>"
    );
    // Baseline (z = min) and waterlines inside the profile.
    let _ = writeln!(
        out,
        "<line class=\"centerline\" x1=\"{margin:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        top + dw,
        margin + lw,
        top + dw
    );
    let map_profile = |x: f64, z: f64| -> (f64, f64) {
        (margin + (x - min.x) * scale, top + dw - (z - min.z) * scale)
    };
    for c in &cut.buttocks {
        let d = svg_path(c, &map_profile);
        let _ = writeln!(out, "<path class=\"buttock\" d=\"{d}\"/>");
    }
    if opts.combs {
        let combs = view_combs(&cut.buttocks, opts.comb_scale, d);
        for comb in &combs {
            let (spikes, tips) = comb_svg(comb, &map_profile);
            let _ = writeln!(out, "<path class=\"spike\" d=\"{spikes}\"/>");
            let _ = writeln!(out, "<path class=\"comb\" d=\"{tips}\"/>");
        }
        if !combs.is_empty() {
            let _ = writeln!(
                out,
                "<text class=\"label\" x=\"{margin:.1}\" y=\"{:.1}\">combs: 1 m⁻¹ = {} m</text>",
                top + dw + 26.0,
                opts.comb_scale
            );
        }
    }
    for i in 0..opts.waterlines.max(2) {
        let t = i as f64 / (opts.waterlines.max(2) - 1) as f64;
        let z = min.z + (max.z - min.z) * t;
        let (_, y) = map_profile(min.x, z);
        let _ = writeln!(
            out,
            "<line class=\"waterline\" x1=\"{margin:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            margin + lw
        );
    }
    out.push_str("</g>\n");

    // ---- plan (waterlines) -------------------------------------------
    let _ = writeln!(out, "<g id=\"plan\">");
    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{margin:.1}\" y=\"{:.1}\">Plan - waterlines</text>",
        plan_y - 12.0
    );
    let _ = writeln!(
        out,
        "<rect class=\"frame\" x=\"{margin:.1}\" y=\"{plan_y:.1}\" width=\"{lw:.1}\" height=\"{bw:.1}\"/>"
    );
    let _ = writeln!(
        out,
        "<line class=\"centerline\" x1=\"{margin:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        plan_y + bw / 2.0,
        margin + lw,
        plan_y + bw / 2.0
    );
    let map_plan = |x: f64, y: f64| -> (f64, f64) {
        (margin + (x - min.x) * scale, plan_y + bw / 2.0 - y * scale)
    };
    for c in &cut.waterlines {
        let d = svg_path(c, &map_plan);
        let _ = writeln!(out, "<path class=\"waterline\" d=\"{d}\"/>");
    }
    if opts.combs {
        let combs = view_combs(&cut.waterlines, opts.comb_scale, b);
        for comb in &combs {
            let (spikes, tips) = comb_svg(comb, &map_plan);
            let _ = writeln!(out, "<path class=\"spike\" d=\"{spikes}\"/>");
            let _ = writeln!(out, "<path class=\"comb\" d=\"{tips}\"/>");
        }
        if !combs.is_empty() {
            let _ = writeln!(
                out,
                "<text class=\"label\" x=\"{margin:.1}\" y=\"{:.1}\">combs: 1 m⁻¹ = {} m</text>",
                plan_y + bw + 26.0,
                opts.comb_scale
            );
        }
    }
    // Station lines in the plan view.
    for i in 0..opts.stations.max(2) {
        let t = i as f64 / (opts.stations.max(2) - 1) as f64;
        let x = min.x + (max.x - min.x) * t;
        let px = margin + (x - min.x) * scale;
        let _ = writeln!(
            out,
            "<line class=\"station\" x1=\"{px:.1}\" y1=\"{plan_y:.1}\" x2=\"{px:.1}\" y2=\"{:.1}\"/>",
            plan_y + bw
        );
    }
    out.push_str("</g>\n");

    // ---- body plan (stations) ----------------------------------------
    let body_x = margin + lw + 40.0;
    let _ = writeln!(out, "<g id=\"bodyplan\">");
    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{body_x:.1}\" y=\"{:.1}\">Body plan - stations</text>",
        plan_y - 12.0
    );
    let _ = writeln!(
        out,
        "<rect class=\"frame\" x=\"{body_x:.1}\" y=\"{plan_y:.1}\" width=\"{bw:.1}\" height=\"{bw:.1}\"/>"
    );
    let _ = writeln!(
        out,
        "<line class=\"centerline\" x1=\"{:.1}\" y1=\"{plan_y:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        body_x + bw / 2.0,
        body_x + bw / 2.0,
        plan_y + bw
    );
    let map_body = |y: f64, z: f64| -> (f64, f64) {
        (
            body_x + bw / 2.0 + y * scale,
            plan_y + bw - (z - min.z) * scale,
        )
    };
    for c in &cut.stations {
        let d = svg_path(c, &map_body);
        let _ = writeln!(out, "<path class=\"station\" d=\"{d}\"/>");
    }
    if opts.combs {
        let combs = view_combs(&cut.stations, opts.comb_scale, b);
        for comb in &combs {
            let (spikes, tips) = comb_svg(comb, &map_body);
            let _ = writeln!(out, "<path class=\"spike\" d=\"{spikes}\"/>");
            let _ = writeln!(out, "<path class=\"comb\" d=\"{tips}\"/>");
        }
        if !combs.is_empty() {
            let _ = writeln!(
                out,
                "<text class=\"label\" x=\"{body_x:.1}\" y=\"{:.1}\">combs: 1 m⁻¹ = {} m</text>",
                plan_y + bw + 26.0,
                opts.comb_scale
            );
        }
    }
    out.push_str("</g>\n");

    // Scale bar and captions.
    let _ = writeln!(
        out,
        "<g id=\"captions\"><text class=\"label\" x=\"{margin:.1}\" y=\"{:.1}\">L {:.2} m   B {:.2} m   D {:.2} m   {:.2} m per {} px</text>",
        height - margin + 8.0,
        l,
        b,
        d,
        l / lw,
        lw.round() as i64
    );
    out.push_str("</g>\n</svg>\n");
    out
}

/// Render the linesplan as an AutoCAD 2000 DXF drawing, in metres.
///
/// The three views share the waterline axis, as a conventional linesplan does:
/// the profile sits at the model's own `(x, z)`, the plan below it in `(x, y)`
/// and the body plan to the right in `(y, z)`. Every curve is also written as a
/// 3D polyline at its true coordinates on its own `… 3D` layer, so the hull can
/// be handed to Rhino or ShipConstructor without rebuilding it from the views.
pub fn dxf(surface: &Surface, opts: LinesplanOptions) -> String {
    let cut = cut_all(surface, opts);
    let (min, max) = (cut.min, cut.max);
    let l = (max.x - min.x).max(0.1);
    let b = (max.y - min.y).max(0.1);
    let gap = 0.12 * l.max(2.0 * b);
    let plan_y0 = min.z - gap - b;
    let body_x0 = max.x + gap;

    let mut layers = vec![
        DxfLayer::new("Stations", 7),
        DxfLayer::new("Buttocks", 1),
        DxfLayer::new("Waterlines", 5),
        DxfLayer::new("Grid", 8).dashed(),
        DxfLayer::new("Centerline", 8).dashed(),
        DxfLayer::new("Waterplane", 4).dashed(),
    ];
    if opts.combs {
        layers.push(DxfLayer::new("Combs", 2));
    }

    let mut entities: Vec<DxfEntity> = Vec::new();
    let frame = |x0: f64, y0: f64, x1: f64, y1: f64| DxfShape::Polyline {
        points: vec![
            Vec2::new(x0, y0),
            Vec2::new(x1, y0),
            Vec2::new(x1, y1),
            Vec2::new(x0, y1),
            Vec2::new(x0, y0),
        ],
    };
    entities.push(DxfEntity::new("Grid", frame(min.x, min.z, max.x, max.z)));
    entities.push(DxfEntity::new(
        "Grid",
        frame(min.x, plan_y0, max.x, plan_y0 + b),
    ));
    entities.push(DxfEntity::new(
        "Grid",
        frame(body_x0, min.z, body_x0 + b, max.z),
    ));

    // Labels above each view.
    let text_height = 0.03 * l;
    // TEXT is centred on its point, so shift each label by half its width to
    // keep it inside the drawing's extents.
    let mut label = |x: f64, y: f64, s: &str| {
        let half = 0.4 * text_height * s.chars().count() as f64;
        entities.push(DxfEntity::new(
            "Grid",
            DxfShape::Text {
                at: Vec2::new(x + half, y),
                height: text_height,
                angle_deg: 0.0,
                text: s.to_owned(),
            },
        ));
    };
    label(min.x, max.z + 2.0 * text_height, "PROFILE - buttocks");
    label(min.x, plan_y0 - 2.0 * text_height, "PLAN - waterlines");
    label(body_x0, max.z + 2.0 * text_height, "BODY PLAN - stations");

    // The two centrelines (ship y = 0) and the keel line.
    entities.push(DxfEntity::new(
        "Centerline",
        DxfShape::Line(
            Vec2::new(min.x, plan_y0 - min.y),
            Vec2::new(max.x, plan_y0 - min.y),
        ),
    ));
    entities.push(DxfEntity::new(
        "Centerline",
        DxfShape::Line(
            Vec2::new(body_x0 - min.y, min.z),
            Vec2::new(body_x0 - min.y, max.z),
        ),
    ));
    entities.push(DxfEntity::new(
        "Centerline",
        DxfShape::Line(Vec2::new(min.x, min.z), Vec2::new(max.x, min.z)),
    ));

    // Body plan: stations in (y, z), to the right of the profile.
    for curve in &cut.stations {
        entities.push(DxfEntity::new(
            "Stations",
            DxfShape::Polyline {
                points: curve
                    .points
                    .iter()
                    .map(|(u, v)| Vec2::new(body_x0 + (u - min.y), *v))
                    .collect(),
            },
        ));
        entities.push(DxfEntity::new(
            "Stations 3D",
            DxfShape::Polyline3 {
                points: curve.points3.clone(),
            },
        ));
    }
    // Profile: buttocks in (x, z).
    for curve in &cut.buttocks {
        entities.push(DxfEntity::new(
            "Buttocks",
            DxfShape::Polyline {
                points: curve
                    .points
                    .iter()
                    .map(|(u, v)| Vec2::new(*u, *v))
                    .collect(),
            },
        ));
        entities.push(DxfEntity::new(
            "Buttocks 3D",
            DxfShape::Polyline3 {
                points: curve.points3.clone(),
            },
        ));
    }
    // Plan: waterlines in (x, y), below the profile.
    for curve in &cut.waterlines {
        entities.push(DxfEntity::new(
            "Waterlines",
            DxfShape::Polyline {
                points: curve
                    .points
                    .iter()
                    .map(|(u, v)| Vec2::new(*u, plan_y0 + (v - min.y)))
                    .collect(),
            },
        ));
        entities.push(DxfEntity::new(
            "Waterlines 3D",
            DxfShape::Polyline3 {
                points: curve.points3.clone(),
            },
        ));
    }
    // The design waterline, highlighted in the plan.
    if let Some(curve) = &cut.waterplane {
        entities.push(DxfEntity::new(
            "Waterplane",
            DxfShape::Polyline {
                points: curve
                    .points
                    .iter()
                    .map(|(u, v)| Vec2::new(*u, plan_y0 + (v - min.y)))
                    .collect(),
            },
        ));
    }

    // Combs, one set per view, drawn in the view's own coordinates so they sit
    // on the lines they belong to.
    if opts.combs {
        let d = (max.z - min.z).max(0.1);
        let mut push = |spikes: &[Spike], map: &dyn Fn(f64, f64) -> P2| {
            let mut tips = Vec::with_capacity(spikes.len());
            for (base, tip) in spikes {
                let (a, b) = (map(base.0, base.1), map(tip.0, tip.1));
                entities.push(DxfEntity::new(
                    "Combs",
                    DxfShape::Line(Vec2::new(a.0, a.1), Vec2::new(b.0, b.1)),
                ));
                tips.push(Vec2::new(b.0, b.1));
            }
            if tips.len() >= 2 {
                entities.push(DxfEntity::new("Combs", DxfShape::Polyline { points: tips }));
            }
        };
        let map_body = |u: f64, v: f64| (body_x0 + (u - min.y), v);
        let map_profile = |u: f64, v: f64| (u, v);
        let map_plan = |u: f64, v: f64| (u, plan_y0 + (v - min.y));
        for (curves, extent, map) in [
            (&cut.stations, b, &map_body as &dyn Fn(f64, f64) -> P2),
            (&cut.buttocks, d, &map_profile),
            (&cut.waterlines, b, &map_plan),
        ] {
            for comb in &view_combs(curves, opts.comb_scale, extent) {
                push(comb, map);
            }
        }
    }

    let mut all = layers.to_vec();
    all.push(DxfLayer::new("Stations 3D", 3));
    all.push(DxfLayer::new("Buttocks 3D", 3));
    all.push(DxfLayer::new("Waterlines 3D", 3));
    dxf::write_dxf(&all, &entities)
}

/// Export the hull curves as an IGES 5.3 file of B-spline curves (entity 126).
///
/// Every station, buttock and waterline is interpolated by a cubic spline, so
/// the receiving application gets exact curves instead of thousands of line
/// segments. The curves are in model coordinates (metres); see
/// [`crate::iges`] for what is not written (surfaces).
pub fn iges(surface: &Surface, opts: LinesplanOptions) -> String {
    let cut = cut_all(surface, opts);
    let mut fitted: Vec<(String, crate::iges::BsplineCurve)> = Vec::new();
    let groups = [
        ("STA", &cut.stations),
        ("BUT", &cut.buttocks),
        ("WL", &cut.waterlines),
    ];
    for (prefix, curves) in groups {
        for (i, curve) in curves.iter().enumerate() {
            // A closed ring is written open: the first point is repeated so the
            // spline returns to it, which every reader imports as one curve.
            let mut points = curve.points3.clone();
            if curve.closed {
                if let Some(first) = points.first().copied() {
                    points.push(first);
                }
            }
            if points.len() < 3 {
                continue;
            }
            if let Some(spline) = crate::iges::fit_curve(&points, 3) {
                fitted.push((format!("{prefix}{i}"), spline));
            }
        }
    }
    let name = format!("{}.igs", surface.settings.name.replace(' ', "_"));
    let file = crate::iges::IgesFile {
        name: &name,
        curves: fitted
            .iter()
            .map(|(label, curve)| (label.as_str(), curve))
            .collect(),
        units: 6, // metres
    };
    crate::iges::write_iges(&file)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_view_draws_its_curves_upright_inside_the_extents() {
        // Profile (x, z), plan (x, y) and body plan (y, z): each 2D point must
        // equal the matching coordinates of its 3D point. A flipped basis put
        // the profile at (x, -z), below its frame.
        let surface = crate::demo::demo_hull(false);
        let cut = cut_all(&surface, LinesplanOptions::default());
        type View<'a> = (&'a str, &'a [Curve], fn(Vec3) -> (f64, f64));
        let views: [View; 3] = [
            ("profile", &cut.buttocks, |p| (p.x, p.z)),
            ("plan", &cut.waterlines, |p| (p.x, p.y)),
            ("body", &cut.stations, |p| (p.y, p.z)),
        ];
        for (name, curves, expect) in views {
            assert!(!curves.is_empty(), "{name}: no curves");
            for c in curves {
                for (p2, p3) in c.points.iter().zip(&c.points3) {
                    let (eu, ev) = expect(*p3);
                    assert!(
                        (p2.0 - eu).abs() < 1e-9 && (p2.1 - ev).abs() < 1e-9,
                        "{name}: 2D {p2:?} is not the view of {p3:?}"
                    );
                }
            }
        }
        for c in &cut.buttocks {
            for &(_, z) in &c.points {
                assert!(
                    z >= cut.min.z - 1e-9 && z <= cut.max.z + 1e-9,
                    "profile z {z} outside the frame"
                );
            }
        }
    }

    #[test]
    fn the_svg_and_dxf_carry_the_same_views() {
        let surface = crate::demo::demo_hull(false);
        let opts = LinesplanOptions::default();
        let drawing = dxf(&surface, opts);
        assert!(drawing.starts_with("  0\nSECTION"));
        assert!(drawing.contains("AC1015"));
        for layer in [
            "Stations",
            "Buttocks",
            "Waterlines",
            "Grid",
            "Centerline",
            "Waterplane",
            "Stations 3D",
            "Waterlines 3D",
        ] {
            assert!(
                drawing.contains(&format!("\n{layer}\n")),
                "layer {layer} missing"
            );
        }
        // Entities: three frames + three labels + three centrelines, then every
        // curve twice (2D view and 3D copy), plus the design waterline.
        let polylines = drawing.matches("\nLWPOLYLINE\n").count();
        let poly3d = drawing.matches("\nPOLYLINE\n").count();
        assert!(polylines > 30, "2D polylines: {polylines}");
        // Three frames and the design waterline have no 3D twin.
        assert_eq!(
            polylines - poly3d,
            4,
            "every curve but the frames has a 3D twin"
        );
        // The SVG still renders.
        let svg = svg(&surface, opts);
        assert!(svg.contains("<svg") && svg.contains("bodyplan"));
    }

    /// A circle is the one curve whose curvature is known everywhere, and a
    /// straight run is the one whose comb must vanish.
    #[test]
    fn the_comb_reads_a_circles_curvature() {
        let r = 5.0;
        // Fine enough that resampling samples the geometry, not the joints.
        let n = 200;
        let points: Vec<(f64, f64)> = (0..=n)
            .map(|i| {
                let a = std::f64::consts::FRAC_PI_2 * i as f64 / n as f64;
                (r * a.cos(), r * a.sin())
            })
            .collect();
        for i in 1..points.len() - 1 {
            let k = curvature_at(&points, i);
            assert!(
                (k - 1.0 / r).abs() < 1e-6,
                "curvature {k} at {i}, want {}",
                1.0 / r
            );
        }
        // Spikes point away from the centre of curvature, i.e. outside the arc.
        let spikes = comb_spikes(&points, 2.0, 10.0, 0.05);
        assert!(
            spikes.len() > 120,
            "one spike per 5 cm of a 7.9 m arc, got {}",
            spikes.len()
        );
        // |k| = 1/5 and the scale is 2, so every spike is 0.4 long; the
        // resampled points sit on chords, which flattens the first and last few
        // by ~5 %.
        let mut lengths = Vec::new();
        for (base, tip) in &spikes {
            let rb = (base.0 * base.0 + base.1 * base.1).sqrt();
            let rt = (tip.0 * tip.0 + tip.1 * tip.1).sqrt();
            assert!(rt > rb, "spike should point outward: {rb} -> {rt}");
            lengths.push((tip.0 - base.0).hypot(tip.1 - base.1));
        }
        lengths.sort_by(f64::total_cmp);
        let median = lengths[lengths.len() / 2];
        assert!((median - 0.4).abs() < 0.02, "median spike length {median}");
        assert!(
            (0.3..0.5).contains(lengths.first().unwrap())
                && (0.3..0.5).contains(lengths.last().unwrap()),
            "spike lengths out of range: {} .. {}",
            lengths.first().unwrap(),
            lengths.last().unwrap()
        );

        let line: Vec<(f64, f64)> = (0..=5).map(|i| (i as f64, 0.0)).collect();
        assert!(
            comb_spikes(&line, 2.0, 10.0, 0.5).is_empty(),
            "a straight line has no comb"
        );
        assert_eq!(curvature_at(&line, 2), 0.0);
    }

    /// Combs are opt-in and land on their own layer, so a plain linesplan is
    /// unchanged.
    #[test]
    fn combs_are_drawn_only_when_asked() {
        let surface = crate::demo::demo_hull(false);
        let plain = LinesplanOptions::default();
        let with = LinesplanOptions {
            combs: true,
            ..plain
        };

        let svg_plain = svg(&surface, plain);
        let svg_combs = svg(&surface, with);
        assert!(!svg_plain.contains("class=\"comb\""), "no combs by default");
        assert!(svg_combs.contains("class=\"comb\""), "combs in every view");
        assert!(svg_combs.contains("class=\"spike\""));
        assert!(svg_combs.contains("1 m⁻¹"));

        let dxf_plain = dxf(&surface, plain);
        let dxf_combs = dxf(&surface, with);
        assert!(!dxf_plain.contains("\nCombs\n"));
        assert!(dxf_combs.contains("\nCombs\n"), "combs layer missing");
        // Every station, buttock and waterline contributes spikes: far more
        // lines than the plain drawing.
        let lines_plain = dxf_plain.matches("\nLINE\n").count();
        let lines_combs = dxf_combs.matches("\nLINE\n").count();
        assert!(
            lines_combs > lines_plain + 50,
            "expected combs to add lines: {lines_plain} -> {lines_combs}"
        );
    }

    /// The GUI draws stations, buttocks and waterlines in 3D, so the comb
    /// spikes must come back as 3D points, each still in its curve's cutting
    /// plane: a station spike spans only (y, z), a waterline only (x, y).
    #[test]
    fn curves_3d_emits_3d_comb_spikes_in_section_planes() {
        let surface = crate::demo::demo_hull(false);
        let curves = curves_3d(
            &surface,
            LinesplanOptions {
                combs: true,
                ..Default::default()
            },
        );

        assert!(!curves.stations.is_empty(), "no stations");
        assert!(!curves.buttocks.is_empty(), "no buttocks");
        assert!(!curves.waterlines.is_empty(), "no waterlines");

        let with_spikes =
            |cs: &[CurveWithCombs]| cs.iter().filter(|c| !c.spikes.is_empty()).count();
        for (name, cs) in [
            ("stations", &curves.stations),
            ("buttocks", &curves.buttocks),
            ("waterlines", &curves.waterlines),
        ] {
            assert!(with_spikes(cs) > 0, "{name} carry no 3D combs");
            for c in cs {
                assert!(!c.points.is_empty(), "{name} curve with no 3D points");
                assert!(c
                    .points
                    .iter()
                    .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite()));
            }
        }

        // Stations are cut at x = const, so a spike is pure (y, z).
        let mut checked = 0;
        for c in &curves.stations {
            for (base, tip) in &c.spikes {
                assert!(
                    (base.x - tip.x).abs() < 1e-9,
                    "station spike leaves x=const: {base:?} {tip:?}"
                );
                // Both ends sit on the station's plane, and so do the curve's
                // own points.
                assert!(base.x.is_finite() && tip.x.is_finite());
                checked += 1;
            }
        }
        assert!(checked > 0, "no station spikes to check");

        // Buttocks are cut at y = const, waterlines at z = const.
        for c in &curves.buttocks {
            for (base, tip) in &c.spikes {
                assert!(
                    (base.y - tip.y).abs() < 1e-9,
                    "buttock spike leaves y=const"
                );
            }
        }
        for c in &curves.waterlines {
            for (base, tip) in &c.spikes {
                assert!(
                    (base.z - tip.z).abs() < 1e-9,
                    "waterline spike leaves z=const"
                );
            }
        }

        // Combs are opt-in: without them the same curves arrive spike-free.
        let plain = curves_3d(&surface, LinesplanOptions::default());
        assert!(plain.stations.iter().all(|c| c.spikes.is_empty()));
        assert!(plain.buttocks.iter().all(|c| c.spikes.is_empty()));
        assert!(plain.waterlines.iter().all(|c| c.spikes.is_empty()));
        assert_eq!(plain.stations.len(), curves.stations.len());
    }
}
