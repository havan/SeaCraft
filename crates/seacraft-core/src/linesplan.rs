//! Linesplan: stations, buttocks and waterlines as an SVG drawing.
//!
//! FREE!ship derives its linesplan from the same intersection machinery used
//! for hydrostatics: cut the refined hull with a set of planes, chain the
//! resulting segments into polylines and project them onto the three
//! orthographic views. This module does exactly that and lays the curves out
//! as a conventional three-view drawing (profile, plan, body plan).

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
}

impl Default for LinesplanOptions {
    fn default() -> Self {
        Self { stations: 11, buttocks: 5, waterlines: 7, level: 3, mode: SubdivisionMode::Loop }
    }
}

/// A polyline in one of the orthographic views, in view coordinates.
struct Curve {
    points: Vec<(f64, f64)>,
    closed: bool,
}

/// Triangles of every visible layer, mirrored where the layer is symmetric.
fn visible_triangles(surface: &Surface, subdiv: SubdivisionOptions) -> (Vec<Vec3>, Vec<[usize; 3]>) {
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

/// Curves where `plane` cuts the mesh, projected into the plane's basis.
fn cut(verts: &[Vec3], tris: &[[usize; 3]], plane: &Plane, tol: f64) -> Vec<Curve> {
    let segs = crate::hydro::plane_segments(verts, tris, plane);
    let paths = crate::hydro::chain_paths(&segs, tol);
    let (u, v) = plane.basis();
    paths
        .into_iter()
        .filter(|(p, _)| p.len() >= 2)
        .map(|(p, closed)| Curve {
            points: p.iter().map(|q| (u.dot(*q), v.dot(*q))).collect(),
            closed,
        })
        .collect()
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
    let subdiv = SubdivisionOptions { level: opts.level, mode: opts.mode };
    let (verts, tris) = visible_triangles(surface, subdiv);
    let (min, max) = surface.extents();
    let l = (max.x - min.x).max(0.1);
    let b = (max.y - min.y).max(0.1);
    let d = (max.z - min.z).max(0.1);
    let tol = l.max(b) * 1e-6;

    // Body plan: stations at x = const, viewed from the bow (u = y, v = z).
    let mut stations: Vec<Curve> = Vec::new();
    for i in 0..opts.stations {
        let t = i as f64 / (opts.stations - 1).max(1) as f64;
        let x = min.x + (max.x - min.x) * t;
        let plane = Plane::new(Vec3::new(1.0, 0.0, 0.0), -x);
        for c in cut(&verts, &tris, &plane, tol) {
            stations.push(c);
        }
    }
    // Profile: buttocks at y = const, viewed from starboard (u = x, v = z).
    let mut buttocks: Vec<Curve> = Vec::new();
    for i in 0..opts.buttocks {
        let t = i as f64 / (opts.buttocks - 1).max(1) as f64;
        let y = min.y + (max.y - min.y) * t;
        let plane = Plane::new(Vec3::new(0.0, 1.0, 0.0), -y);
        for c in cut(&verts, &tris, &plane, tol) {
            buttocks.push(c);
        }
    }
    // Plan: waterlines at z = const, viewed from above (u = x, v = y).
    let mut waterlines: Vec<Curve> = Vec::new();
    for i in 0..opts.waterlines {
        let t = i as f64 / (opts.waterlines - 1).max(1) as f64;
        let z = min.z + (max.z - min.z) * t;
        let plane = Plane::new(Vec3::new(0.0, 0.0, 1.0), -z);
        for c in cut(&verts, &tris, &plane, tol) {
            waterlines.push(c);
        }
    }

    // Panel geometry in millimetres (1 m = 1 unit here; scaled for print).
    let scale = 900.0 / l.max(2.0 * b);
    let margin = 40.0;
    let bw = b * 2.0 * scale;
    let lw = l * scale;
    let dw = d * scale;
    let top = margin;
    let profile_y = top + dw + 40.0;
    let plan_y = profile_y + bw + 40.0;
    let width = margin * 2.0 + lw + bw + 40.0;
    let height = plan_y + bw + margin + 20.0;

    let mut out = String::with_capacity(8192);
    out.push_str("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width:.0}\" height=\"{height:.0}\" viewBox=\"0 0 {width:.0} {height:.0}\">"
    );
    let _ = writeln!(out, "<title>{} - linesplan</title>", html_escape(&surface.settings.name));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<style>\n");
    out.push_str(".frame{fill:none;stroke:#111;stroke-width:0.8}\n");
    out.push_str(".waterline{fill:none;stroke:#2a6fdb;stroke-width:0.7}\n");
    out.push_str(".station{fill:none;stroke:#111;stroke-width:0.7}\n");
    out.push_str(".buttock{fill:none;stroke:#c0392b;stroke-width:0.7}\n");
    out.push_str(".centerline{fill:none;stroke:#888;stroke-width:0.5;stroke-dasharray:6 4}\n");
    out.push_str(".label{font:11px ui-sans-serif,system-ui,sans-serif;fill:#333}\n");
    out.push_str(".title{font:13px ui-sans-serif,system-ui,sans-serif;fill:#111}\n");
    out.push_str("</style>\n");

    // ---- profile (buttocks) ------------------------------------------
    let _ = writeln!(out, "<g id=\"profile\">");
    let _ = writeln!(out, "<text class=\"title\" x=\"{margin:.1}\" y=\"{:.1}\">Profile - buttocks</text>", top - 12.0);
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
    for c in &buttocks {
        let d = svg_path(c, &map_profile);
        let _ = writeln!(out, "<path class=\"buttock\" d=\"{d}\"/>");
    }
    for i in 0..opts.waterlines {
        let t = i as f64 / (opts.waterlines - 1).max(1) as f64;
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
    let _ = writeln!(out, "<text class=\"title\" x=\"{margin:.1}\" y=\"{:.1}\">Plan - waterlines</text>", plan_y - 12.0);
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
    for c in &waterlines {
        let d = svg_path(c, &map_plan);
        let _ = writeln!(out, "<path class=\"waterline\" d=\"{d}\"/>");
    }
    // Station lines in the plan view.
    for i in 0..opts.stations {
        let t = i as f64 / (opts.stations - 1).max(1) as f64;
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
    let _ = writeln!(out, "<text class=\"title\" x=\"{body_x:.1}\" y=\"{:.1}\">Body plan - stations</text>", plan_y - 12.0);
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
        (body_x + bw / 2.0 + y * scale, plan_y + bw - (z - min.z) * scale)
    };
    for c in &stations {
        let d = svg_path(c, &map_body);
        let _ = writeln!(out, "<path class=\"station\" d=\"{d}\"/>");
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

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
