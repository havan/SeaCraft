//! Mesh and report export: OBJ, STL, PLY and a self contained HTML viewer.

use crate::math::Vec3;
use crate::mesh::Surface;
use crate::subdiv::{evaluate, SubdivisionOptions};
use std::fmt::Write as _;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UpAxis {
    /// `z` up (naval architecture / Rhino / Blender default).
    Z,
    /// `y` up, the convention FREE!ship itself uses when it writes OBJ.
    Y,
}

impl UpAxis {
    fn map(self, v: Vec3) -> (f64, f64, f64) {
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
pub fn render_mesh(surface: &Surface, level: u8, mode: crate::subdiv::SubdivisionMode) -> RenderMesh {
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
    RenderMesh { verts: mesh.verts, faces, face_layer }
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
    let _ = write!(out, "# {} vertices, {} faces\n", mesh.verts.len(), mesh.faces.len());
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
pub fn obj(surface: &Surface, level: u8, mode: crate::subdiv::SubdivisionMode, up: UpAxis) -> String {
    write_obj(&render_mesh(surface, level, mode), up, surface.settings.name.as_str())
}

/// Wavefront OBJ of the control net only.
pub fn obj_control(surface: &Surface, up: UpAxis) -> String {
    write_obj(&control_mesh(surface), up, &format!("{} (control net)", surface.settings.name))
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
    let size = (max.x - min.x).max(max.y - min.y).max(max.z - min.z).max(1.0);
    let cx = (min.x + max.x) * 0.5;
    let cy = (min.y + max.y) * 0.5;
    let cz = (min.z + max.z) * 0.5;

    let mut html = String::with_capacity(positions.len() + indices.len() + 4096);
    html.push_str("<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n");
    let _ = writeln!(html, "<title>{}</title>", title);
    html.push_str(
        "<style>html,body{margin:0;height:100%;background:#0b1016;color:#cfd8e3;font:13px/1.4 ui-sans-serif,system-ui,sans-serif}canvas{display:block}#hud{position:fixed;left:16px;top:14px;pointer-events:none;text-shadow:0 1px 2px #000}a{color:#7fb2ff}</style>\n</head>\n<body>\n<div id=\"hud\">",
    );
    let _ = write!(
        html,
        "<b>{}</b><br>{} verts &middot; {} faces &middot; subdivision level {}<br><span style=\"opacity:.65\">drag to orbit &middot; scroll to zoom</span></div>\n",
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
    let _ = writeln!(
        html,
        "const scene=new THREE.Scene();scene.background=new THREE.Color(0x0b1016);\
         const cam=new THREE.PerspectiveCamera(40,innerWidth/innerHeight,0.05,5000);\
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
         const grid=new THREE.GridHelper(200,200,0x22303f,0x16202b);grid.position.y={:.4};scene.add(grid);\
         ctl.target.set({:.3},{:.3},{:.3});ctl.update();\
         addEventListener('resize',()=>{{cam.aspect=innerWidth/innerHeight;cam.updateProjectionMatrix();r.setSize(innerWidth,innerHeight);}});\
         (function loop(){{requestAnimationFrame(loop);ctl.update();r.render(scene,cam);}})();\n",
        cx + size * 1.5,
        cy + size * 1.1,
        cz + size * 1.6,
        cz,
        cx,
        cz,
        cy
    );
    html.push_str("</script>\n</body>\n</html>\n");
    html
}

fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
