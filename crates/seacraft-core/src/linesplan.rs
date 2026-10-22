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
}

impl Default for LinesplanOptions {
    fn default() -> Self {
        Self { stations: 11, buttocks: 5, waterlines: 7, level: 3, mode: SubdivisionMode::Loop }
    }
}

/// A polyline in one of the orthographic views: its 2D view coordinates and
/// the 3D curve it came from.
struct Curve {
    points: Vec<(f64, f64)>,
    points3: Vec<Vec3>,
    closed: bool,
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
            points3: p,
            closed,
        })
        .collect()
}

/// Cut the hull once for every view.
fn cut_all(surface: &Surface, opts: LinesplanOptions) -> Cut {
    let subdiv = SubdivisionOptions { level: opts.level, mode: opts.mode };
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
        cut(&verts, &tris, &plane, tol).into_iter().max_by_key(|c| c.points.len())
    } else {
        None
    };

    Cut { stations, buttocks, waterlines, waterplane, min, max }
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
    for c in &cut.buttocks {
        let d = svg_path(c, &map_profile);
        let _ = writeln!(out, "<path class=\"buttock\" d=\"{d}\"/>");
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
    for c in &cut.waterlines {
        let d = svg_path(c, &map_plan);
        let _ = writeln!(out, "<path class=\"waterline\" d=\"{d}\"/>");
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
    for c in &cut.stations {
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

    let layers = [
        DxfLayer::new("Stations", 7),
        DxfLayer::new("Buttocks", 1),
        DxfLayer::new("Waterlines", 5),
        DxfLayer::new("Grid", 8).dashed(),
        DxfLayer::new("Centerline", 8).dashed(),
        DxfLayer::new("Waterplane", 4).dashed(),
    ];

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
    entities.push(DxfEntity::new("Grid", frame(min.x, plan_y0, max.x, plan_y0 + b)));
    entities.push(DxfEntity::new("Grid", frame(body_x0, min.z, body_x0 + b, max.z)));

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
        DxfShape::Line(Vec2::new(min.x, plan_y0 - min.y), Vec2::new(max.x, plan_y0 - min.y)),
    ));
    entities.push(DxfEntity::new(
        "Centerline",
        DxfShape::Line(Vec2::new(body_x0 - min.y, min.z), Vec2::new(body_x0 - min.y, max.z)),
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
            DxfShape::Polyline3 { points: curve.points3.clone() },
        ));
    }
    // Profile: buttocks in (x, z).
    for curve in &cut.buttocks {
        entities.push(DxfEntity::new(
            "Buttocks",
            DxfShape::Polyline {
                points: curve.points.iter().map(|(u, v)| Vec2::new(*u, *v)).collect(),
            },
        ));
        entities.push(DxfEntity::new(
            "Buttocks 3D",
            DxfShape::Polyline3 { points: curve.points3.clone() },
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
            DxfShape::Polyline3 { points: curve.points3.clone() },
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
        curves: fitted.iter().map(|(label, curve)| (label.as_str(), curve)).collect(),
        units: 6, // metres
    };
    crate::iges::write_iges(&file)
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

#[cfg(test)]
mod tests {
    use super::*;

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
            assert!(drawing.contains(&format!("\n{layer}\n")), "layer {layer} missing");
        }
        // Entities: three frames + three labels + three centrelines, then every
        // curve twice (2D view and 3D copy), plus the design waterline.
        let polylines = drawing.matches("\nLWPOLYLINE\n").count();
        let poly3d = drawing.matches("\nPOLYLINE\n").count();
        assert!(polylines > 30, "2D polylines: {polylines}");
        // Three frames and the design waterline have no 3D twin.
        assert_eq!(polylines - poly3d, 4, "every curve but the frames has a 3D twin");
        // The SVG still renders.
        let svg = svg(&surface, opts);
        assert!(svg.contains("<svg") && svg.contains("bodyplan"));
    }
}
