//! Mesh and report export: OBJ, STL, PLY, 3MF, an IGES surface patch and a
//! self contained HTML viewer.

use crate::linesplan::{LinesplanCurves3d, LinesplanOptions};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::subdiv::{evaluate, SubdivisionMode, SubdivisionOptions};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpAxis {
    /// `z` up (naval architecture / Rhino / Blender default).
    Z,
    /// `y` up, the convention FREE!ship itself uses when it writes OBJ.
    Y,
}

impl UpAxis {
    /// The model point in this axis convention.
    pub fn map(self, v: Vec3) -> (f64, f64, f64) {
        match self {
            UpAxis::Z => (v.x, v.y, v.z),
            UpAxis::Y => (v.y, v.z, v.x),
        }
    }
}

/// A render-ready polygonal mesh of the visible hull.
pub struct RenderMesh {
    pub verts: Vec<Vec3>,
    pub faces: Vec<Vec<usize>>,
    /// Layer index per face, parallel to `faces`, so a renderer can colour by
    /// layer. Empty only for meshes that carry no layer information.
    pub face_layer: Vec<usize>,
}

/// Refine the hull for display, dropping hidden layers and mirroring the
/// symmetric ones.
pub fn render_mesh(
    surface: &Surface,
    level: u8,
    mode: crate::subdiv::SubdivisionMode,
) -> RenderMesh {
    let mut mesh = evaluate(surface, SubdivisionOptions { level, mode });
    let sym: Vec<bool> = surface.layers.iter().map(|l| l.symmetric).collect();
    mesh.mirror_symmetric(&sym);
    // Drop hidden layers only after mirroring, which needs the per-face layer.
    let mut faces: Vec<Vec<usize>> = Vec::new();
    let mut face_layer: Vec<usize> = Vec::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        if surface.layers[mesh.face_layer[fi]].visible {
            faces.push(f.clone());
            face_layer.push(mesh.face_layer[fi]);
        }
    }
    RenderMesh {
        verts: mesh.verts,
        faces,
        face_layer,
    }
}

pub fn control_mesh(surface: &Surface) -> RenderMesh {
    RenderMesh {
        verts: surface.points.iter().map(|p| p.p).collect(),
        faces: surface.faces.clone(),
        face_layer: surface.face_layer.clone(),
    }
}

/// Newell's method: an unnormalised normal whose length is twice the area of
/// the polygon. Robust for non-planar quads, which a refined hull quad is.
pub fn newell_normal(points: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    let len = points.len();
    if len < 3 {
        return n;
    }
    for i in 0..len {
        let a = points[i];
        let b = points[(i + 1) % len];
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

fn write_obj(mesh: &RenderMesh, up: UpAxis, header: &str) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# {} - SeaCraft {}", header, crate::VERSION);
    let _ = writeln!(
        out,
        "# {} vertices, {} faces",
        mesh.verts.len(),
        mesh.faces.len()
    );
    for v in &mesh.verts {
        let (a, b, c) = up.map(*v);
        let _ = writeln!(out, "v {:.6} {:.6} {:.6}", a, b, c);
    }
    for f in &mesh.faces {
        out.push('f');
        for &i in f {
            let _ = write!(out, " {}", i + 1);
        }
        out.push('\n');
    }
    out
}

/// Wavefront OBJ of the refined hull.
pub fn obj(
    surface: &Surface,
    level: u8,
    mode: crate::subdiv::SubdivisionMode,
    up: UpAxis,
) -> String {
    write_obj(
        &render_mesh(surface, level, mode),
        up,
        surface.settings.name.as_str(),
    )
}

/// Wavefront OBJ of the control net only.
pub fn obj_control(surface: &Surface, up: UpAxis) -> String {
    write_obj(
        &control_mesh(surface),
        up,
        &format!("{} (control net)", surface.settings.name),
    )
}

/// Binary STL of the refined hull.
pub fn stl(surface: &Surface, level: u8, mode: crate::subdiv::SubdivisionMode) -> Vec<u8> {
    let mesh = render_mesh(surface, level, mode);
    let mut tris = 0usize;
    for f in &mesh.faces {
        tris += f.len().saturating_sub(2);
    }
    let mut out = vec![0u8; 80];
    let header = format!("SeaCraft {} - {}", crate::VERSION, surface.settings.name);
    let bytes = header.as_bytes();
    out[..bytes.len().min(80)].copy_from_slice(&bytes[..bytes.len().min(80)]);
    out.extend_from_slice(&(tris as u32).to_le_bytes());

    let push = |out: &mut Vec<u8>, n: Vec3, a: Vec3, b: Vec3, c: Vec3| {
        for v in [n, a, b, c] {
            out.extend_from_slice(&(v.x as f32).to_le_bytes());
            out.extend_from_slice(&(v.y as f32).to_le_bytes());
            out.extend_from_slice(&(v.z as f32).to_le_bytes());
        }
        out.extend_from_slice(&0u16.to_le_bytes());
    };

    for f in &mesh.faces {
        for i in 1..f.len().saturating_sub(1) {
            let a = mesh.verts[f[0]];
            let b = mesh.verts[f[i]];
            let c = mesh.verts[f[i + 1]];
            let n = (b - a).cross(c - a).norm();
            push(&mut out, n, a, b, c);
        }
    }
    out
}

/// 3MF (Core 1.3) package of the refined hull, in metres (`unit="meter"`).
///
/// The archive is stored, not deflated: `threemf::write_3mf` writes a
/// dependency-free ZIP, and the mesh is welded at 1 µm first so a slicer sees a
/// watertight solid rather than one with a crack along every layer seam.
pub fn threemf(surface: &Surface, level: u8, mode: crate::subdiv::SubdivisionMode) -> Vec<u8> {
    let mesh = render_mesh(surface, level, mode);
    crate::threemf::write_3mf(&surface.settings.name, &mesh.verts, &mesh.faces)
}

/// ASCII PLY of the refined hull.
pub fn ply(surface: &Surface, level: u8, mode: crate::subdiv::SubdivisionMode) -> String {
    let mesh = render_mesh(surface, level, mode);
    let mut out = String::new();
    let _ = writeln!(out, "ply");
    let _ = writeln!(out, "format ascii 1.0");
    let _ = writeln!(out, "comment SeaCraft {}", crate::VERSION);
    let _ = writeln!(out, "element vertex {}", mesh.verts.len());
    let _ = writeln!(out, "property float x");
    let _ = writeln!(out, "property float y");
    let _ = writeln!(out, "property float z");
    let _ = writeln!(out, "element face {}", mesh.faces.len());
    let _ = writeln!(out, "property list uchar int vertex_index");
    let _ = writeln!(out, "end_header");
    for v in &mesh.verts {
        let _ = writeln!(out, "{:.6} {:.6} {:.6}", v.x, v.y, v.z);
    }
    for f in &mesh.faces {
        let _ = write!(out, "{}", f.len());
        for &i in f {
            let _ = write!(out, " {}", i);
        }
        out.push('\n');
    }
    out
}

/// IGES 5.3 file of B-spline surface patches (entity 128) lofted through the
/// hull's station curves.
///
/// A subdivided hull has no rectangular parametrisation of its own, so the
/// patches are built from the linesplan's station cuts — the same cuts that
/// draw the body plan. Every station plane is split at the centreline into its
/// starboard (`y >= 0`) and port (`y <= 0`) halves, each half is resampled to
/// 25 points from keel to deck, and the rows, sorted by station position `x`,
/// are interpolated with a bicubic patch per side. Measured in metres, written
/// with the IGES metre unit code (6), like `linesplan::iges`.
///
/// A station whose section is too flat to resample is left out, and so is a
/// second curve in the same plane (a section cut in two by a deck opening);
/// with fewer than two stations left the file carries no entity at all. So are
/// the planes at the model's own ends, which lie in the transom and the stem
/// faces — the cut there is a fan of fragments, not a section.
pub fn iges_surface(surface: &Surface, level: u8, mode: SubdivisionMode) -> String {
    // 25 girth points per station: enough to carry a bilge radius around, few
    // enough to keep the parameter data readable.
    const COLUMNS: usize = 25;
    let curves = crate::linesplan::curves_3d(
        surface,
        LinesplanOptions {
            level,
            mode,
            ..Default::default()
        },
    );
    let (min, max) = surface.extents();
    let tol = ((max.x - min.x).max(max.y - min.y).max(max.z - min.z)).max(0.1) * 1e-6;
    let mut patches: Vec<(&str, crate::iges::BsplineSurface)> = Vec::new();
    for (label, starboard) in [("STBD", true), ("PORT", false)] {
        let grid = station_grid(&curves, starboard, COLUMNS, tol);
        if let Some(patch) = crate::iges::fit_surface(&grid, 3, 3) {
            patches.push((label, patch));
        }
    }
    let patches: Vec<(&str, &crate::iges::BsplineSurface)> = patches
        .iter()
        .map(|(label, patch)| (*label, patch))
        .collect();
    crate::iges::write_iges_surfaces(&surface.settings.name, &patches, 6)
}

/// The station sections of one side as a `stations × columns` grid of points,
/// with the rows sorted by station position `x`.
fn station_grid(
    curves: &LinesplanCurves3d,
    starboard: bool,
    columns: usize,
    tol: f64,
) -> Vec<Vec<Vec3>> {
    // The planes at the extreme `x` of the mesh lie *in* its end faces (the
    // transom and the stem). There the cut is not a section at all but every
    // in-plane edge of those faces, which arrives as a fan of degenerate
    // fragments; and a plane that touches the mesh's own bounding plane cannot
    // cross a face transversally anyway. Such a station is no loft column.
    let mut lo = f64::INFINITY;
    let mut hi = f64::NEG_INFINITY;
    for station in &curves.stations {
        for point in &station.points {
            lo = lo.min(point.x);
            hi = hi.max(point.x);
        }
    }
    let mut rows: Vec<(f64, Vec<Vec3>)> = Vec::new();
    for station in &curves.stations {
        if station.points.is_empty() {
            continue;
        }
        let x = station.points.iter().map(|p| p.x).sum::<f64>() / station.points.len() as f64;
        if x <= lo + tol || x >= hi - tol {
            continue;
        }
        let branch = side_branch(&station.points, station.closed, starboard);
        if branch.len() < 2 {
            continue;
        }
        let Some(sampled) = resample(&branch, columns) else {
            continue;
        };
        // A grazing cut can leave a sliver whose collocation system is
        // numerically singular.
        if arc_length(&sampled) <= tol {
            continue;
        }
        rows.push((x, sampled));
    }
    rows.sort_by(|a, b| a.0.total_cmp(&b.0));
    // Two curves in the same plane would put two equal parameters in the grid,
    // which makes the collocation system singular: keep the longer section.
    let mut out: Vec<Vec<Vec3>> = Vec::with_capacity(rows.len());
    let mut last_x = f64::NEG_INFINITY;
    for (x, row) in rows {
        if x - last_x <= tol {
            if out
                .last()
                .is_some_and(|previous| arc_length(&row) > arc_length(previous))
            {
                out.pop();
                out.push(row);
            }
            continue;
        }
        last_x = x;
        out.push(row);
    }
    out
}

/// One side of a station cut, ordered from the keel to the sheer: starboard
/// keeps every point with `y >= 0`, port every point with `y <= 0`, a
/// micrometre of slack being what puts the centreline point into both (which
/// is where the two patches meet).
///
/// The points keep the order the chain walk produced, so a flat bottom or a
/// deck — many points at one height — stays in sequence. A closed section is
/// a ring that wraps around: its side is the longest run of points on that
/// side, taken cyclically. The run is then turned so it starts at its lower
/// end. Should the cut cross the centreline more than twice, the longest run
/// is the one used.
fn side_branch(points: &[Vec3], closed: bool, starboard: bool) -> Vec<Vec3> {
    const EPS: f64 = 1e-6;
    let on_side = |p: &Vec3| if starboard { p.y >= -EPS } else { p.y <= EPS };
    let n = points.len();
    let mut branch: Vec<Vec3> = match points.iter().position(|p| !on_side(p)) {
        None => points.to_vec(),
        Some(first_off) => {
            // Walk once around (or along) the chain from a point off the side,
            // so no run is split by the start.
            let order: Vec<usize> = if closed {
                (0..n).map(|k| (first_off + k) % n).collect()
            } else {
                (0..n).collect()
            };
            let mut best: Vec<Vec3> = Vec::new();
            let mut run: Vec<Vec3> = Vec::new();
            for i in order {
                if on_side(&points[i]) {
                    run.push(points[i]);
                } else {
                    if run.len() > best.len() {
                        best = std::mem::take(&mut run);
                    }
                    run.clear();
                }
            }
            if run.len() > best.len() {
                best = run;
            }
            best
        }
    };
    if let (Some(a), Some(b)) = (branch.first(), branch.last()) {
        if a.z > b.z {
            branch.reverse();
        }
    }
    branch.dedup_by(|a, b| a.dist(*b) < 1e-12);
    branch
}

/// The length of an ordered polyline.
fn arc_length(points: &[Vec3]) -> f64 {
    points.windows(2).map(|w| w[0].dist(w[1])).sum()
}

/// Resample an ordered polyline to exactly `count` points, equally spaced by
/// arc length. `None` when the polyline has no length to walk.
fn resample(points: &[Vec3], count: usize) -> Option<Vec<Vec3>> {
    let total = arc_length(points);
    if count < 2 || points.len() < 2 || total <= 1e-12 {
        return None;
    }
    let mut out = Vec::with_capacity(count);
    let mut segment = 0usize;
    let mut walked = 0.0;
    for k in 0..count {
        let target = total * k as f64 / (count - 1) as f64;
        while segment + 2 < points.len() {
            let length = points[segment + 1].dist(points[segment]);
            if walked + length >= target {
                break;
            }
            walked += length;
            segment += 1;
        }
        let length = points[segment + 1].dist(points[segment]);
        let t = if length > 0.0 {
            ((target - walked) / length).clamp(0.0, 1.0)
        } else {
            0.0
        };
        out.push(points[segment].lerp(points[segment + 1], t));
    }
    Some(out)
}

/// A self contained three.js viewer (three.js is pulled from a CDN, the model
/// data is embedded so the file works without any other asset).
pub fn viewer_html(surface: &Surface, level: u8, mode: crate::subdiv::SubdivisionMode) -> String {
    let mesh = render_mesh(surface, level, mode);
    let mut positions = String::with_capacity(mesh.verts.len() * 24);
    let mut indices = String::with_capacity(mesh.faces.len() * 30);
    for v in &mesh.verts {
        let _ = write!(positions, "{:.5},{:.5},{:.5},", v.x, v.y, v.z);
    }
    for f in &mesh.faces {
        for i in 1..f.len().saturating_sub(1) {
            let _ = write!(indices, "{},{},{},", f[0], f[i], f[i + 1]);
        }
    }
    let positions = positions.trim_end_matches(',').to_string();
    let indices = indices.trim_end_matches(',').to_string();
    let title = html_escape(&surface.settings.name);
    let (min, max) = surface.extents();
    let size = (max.x - min.x)
        .max(max.y - min.y)
        .max(max.z - min.z)
        .max(1.0);
    let cx = (min.x + max.x) * 0.5;
    let cy = (min.y + max.y) * 0.5;
    let cz = (min.z + max.z) * 0.5;

    let mut html = String::with_capacity(positions.len() + indices.len() + 4096);
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(html, "<title>{}</title>", title);
    html.push_str(
        "<style>html,body{margin:0;height:100%;background:#0b1016;color:#cfd8e3;font:13px/1.4 ui-sans-serif,system-ui,sans-serif}canvas{display:block}#hud{position:fixed;left:16px;top:14px;pointer-events:none;text-shadow:0 1px 2px #000}a{color:#7fb2ff}</style>\n</head>\n<body>\n<div id=\"hud\">",
    );
    let _ = writeln!(
        html,
        "<b>{}</b><br>{} verts &middot; {} faces &middot; subdivision level {}<br><span style=\"opacity:.65\">drag to orbit &middot; scroll to zoom</span></div>",
        title,
        mesh.verts.len(),
        mesh.faces.len(),
        level
    );
    html.push_str(
        "<script type=\"importmap\">{\"imports\":{\"three\":\"https://unpkg.com/three@0.160.0/build/three.module.js\",\"three/addons/\":\"https://unpkg.com/three@0.160.0/examples/jsm/\"}}</script>\n<script type=\"module\">\nimport * as THREE from 'three';\nimport { OrbitControls } from 'three/addons/controls/OrbitControls.js';\n",
    );
    let _ = writeln!(html, "const P=[{}];", positions);
    let _ = writeln!(html, "const I=[{}];", indices);
    // The geometry is SeaCraft's own: x forward, y starboard, z up. Three.js
    // defaults to y-up, which would lay the hull on its side, so the whole
    // scene is switched to z-up before anything is built and the ground grid
    // is turned into the XY plane.
    let _ = writeln!(
        html,
        "THREE.Object3D.DEFAULT_UP.set(0,0,1);\
         const scene=new THREE.Scene();scene.background=new THREE.Color(0x0b1016);\
         const cam=new THREE.PerspectiveCamera(40,innerWidth/innerHeight,0.05,5000);\
         cam.up.set(0,0,1);\
         cam.position.set({:.2},{:.2},{:.2});\
         const r=new THREE.WebGLRenderer({{antialias:true}});r.setSize(innerWidth,innerHeight);\
         r.setPixelRatio(Math.min(devicePixelRatio,2));document.body.appendChild(r.domElement);\
         const ctl=new OrbitControls(cam,r.domElement);ctl.enableDamping=true;\
         scene.add(new THREE.HemisphereLight(0xbfd8ff,0x1b2430,1.1));\
         const key=new THREE.DirectionalLight(0xffffff,2.1);key.position.set(1,1.4,.8);scene.add(key);\
         const fill=new THREE.DirectionalLight(0x93b7ff,.7);fill.position.set(-1,-.6,-.8);scene.add(fill);\
         const g=new THREE.BufferGeometry();\
         g.setAttribute('position',new THREE.Float32BufferAttribute(P,3));\
         g.setIndex(I);g.computeVertexNormals();\
         const m=new THREE.MeshStandardMaterial({{color:0x9fb4c7,metalness:0.15,roughness:0.55,side:THREE.DoubleSide,flatShading:false}});\
         const mesh=new THREE.Mesh(g,m);scene.add(mesh);\
         const w=new THREE.LineSegments(new THREE.WireframeGeometry(g),new THREE.LineBasicMaterial({{color:0x39506b,transparent:true,opacity:.35}}));scene.add(w);\
         const grid=new THREE.GridHelper(200,200,0x22303f,0x16202b);grid.rotation.x=Math.PI/2;grid.position.z={:.4};scene.add(grid);\
         ctl.target.set({:.3},{:.3},{:.3});ctl.update();\
         addEventListener('resize',()=>{{cam.aspect=innerWidth/innerHeight;cam.updateProjectionMatrix();r.setSize(innerWidth,innerHeight);}});\
         (function loop(){{requestAnimationFrame(loop);ctl.update();r.render(scene,cam);}})();\n",
        cx + size * 1.4,
        cy - size * 1.6,
        cz + size * 1.1,
        // The grid sits at the keel: it reads as the ground the hull rests on.
        min.z,
        cx,
        cy,
        cz
    );
    html.push_str("</script>\n</body>\n</html>\n");
    html
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
    fn a_flat_bottomed_station_keeps_its_chain_order() {
        // A barge section walked as a ring: deck centre, starboard deck edge,
        // bilge, flat bottom to the keel, then the port half back up.
        let ring: Vec<Vec3> = [
            (0.0, 2.0),
            (1.0, 2.0),
            (2.0, 2.0),
            (2.0, 1.0),
            (2.0, 0.0),
            (1.5, 0.0),
            (1.0, 0.0),
            (0.5, 0.0),
            (0.0, 0.0),
            (-1.0, 0.0),
            (-2.0, 0.0),
            (-2.0, 2.0),
            (-1.0, 2.0),
        ]
        .iter()
        .map(|&(y, z)| Vec3::new(3.0, y, z))
        .collect();
        let star = side_branch(&ring, true, true);
        let ys: Vec<f64> = star.iter().map(|p| p.y).collect();
        let zs: Vec<f64> = star.iter().map(|p| p.z).collect();
        // Keel first, out along the bottom, up the side, in along the deck.
        assert_eq!(ys, vec![0.0, 0.5, 1.0, 1.5, 2.0, 2.0, 2.0, 1.0, 0.0]);
        assert_eq!(zs, vec![0.0, 0.0, 0.0, 0.0, 0.0, 1.0, 2.0, 2.0, 2.0]);
        let port = side_branch(&ring, true, false);
        assert_eq!(port.first().map(|p| (p.y, p.z)), Some((0.0, 0.0)));
        assert_eq!(port.last().map(|p| (p.y, p.z)), Some((0.0, 2.0)));
    }

    /// The parameter data of every parameter record, joined the way a reader
    /// joins it: the 64 data columns of consecutive lines, concatenated.
    fn parameter_data(text: &str) -> String {
        let mut joined = String::new();
        for line in text.lines().filter(|l| l.chars().nth(72) == Some('P')) {
            joined.push_str(line[..64].trim_end());
        }
        joined
    }

    #[test]
    fn iges_surface_export_writes_a_patch_per_side_of_the_hull() {
        // The symmetric half model is the usual case: the station cuts only
        // close around the girth once the layer has been mirrored.
        let hull = crate::demo::demo_hull(true);
        let text = iges_surface(&hull, 3, SubdivisionMode::Loop);
        for line in text.lines() {
            assert_eq!(line.chars().count(), 80, "not 80 columns: {line:?}");
        }

        // Two patches — one per side of the centreline — two directory records
        // and one parameter block each.
        let d: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('D'))
            .collect();
        assert_eq!(d.len(), 4, "two patches, two directory records each");
        for record in &d {
            assert_eq!(
                &record[0..8].trim(),
                &"128",
                "every entity is a B-spline surface"
            );
        }
        let labels: Vec<String> = d
            .chunks(2)
            .map(|pair| pair[1][56..64].trim().to_owned())
            .collect();
        assert_eq!(labels, vec!["STBD".to_owned(), "PORT".to_owned()]);
        let p: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('P'))
            .collect();
        assert!(p[0].starts_with("128,"), "{:?}", &p[0][..16]);
        let first_pd: usize = d[0][8..16].trim().parse().unwrap();
        let pd_count: usize = d[1][24..32].trim().parse().unwrap();
        assert_eq!(first_pd, 1);
        assert!(
            pd_count > 0 && pd_count < p.len(),
            "each entity owns a block of records"
        );

        // The sections the patches were built from: one row per usable station,
        // 25 girth points each, sorted from the after end forward.
        let curves = crate::linesplan::curves_3d(
            &hull,
            LinesplanOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
                ..Default::default()
            },
        );
        let (min, max) = hull.extents();
        let tol = ((max.x - min.x).max(max.y - min.y).max(max.z - min.z)).max(0.1) * 1e-6;
        let grid = station_grid(&curves, true, 25, tol);
        assert!(
            grid.len() >= 2,
            "the demo hull has stations to loft through"
        );
        assert!(
            grid.iter().all(|row| row.len() == 25),
            "every row is 25 girth points"
        );
        assert!(
            grid.windows(2).all(|w| w[0][0].x < w[1][0].x),
            "rows sorted by x"
        );
        assert!(grid
            .iter()
            .flatten()
            .all(|p| p.y >= -1e-6 && p.x >= min.x - 1e-9 && p.x <= max.x + 1e-9));

        // Interpolation is exact at the grid, so the file carries the hull's own
        // corner points: the after keel point and the fore sheer point of each
        // patch are in its parameter data.
        let joined = parameter_data(&text);
        let numbers: Vec<f64> = joined
            .split(',')
            .filter_map(|t| t.trim().parse().ok())
            .collect();
        for corner in [grid[0][0], grid[grid.len() - 1][24]] {
            for v in [corner.x, corner.y, corner.z] {
                assert!(
                    numbers.iter().any(|n| (n - v).abs() < 1e-9),
                    "coordinate {v} of {corner:?} is missing from the file"
                );
            }
        }
    }
}
