//! Vertex data for the GPU hull: the refined mesh as an unshared triangle list, each face fanned
//! from its first corner, with the layer colour and the unit face normal.
//!
//! Skipping follows [`crate::render::shaded_hull`]: a face with fewer than three corners, or a
//! zero-area one, emits nothing. Positions are relative to the centre of the emitted box, so a hull
//! far from the origin keeps its precision in `f32`. Shading is not done here: the shader lights
//! each vertex from its normal, so one geometry serves every view.

use std::ops::Range;

use seacraft_core::export::{self, RenderMesh};
use seacraft_core::math::Vec3;
use seacraft_core::mesh::Surface;

use crate::render::layer_color;

/// One corner of one triangle: 28 bytes, uploaded as is.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct HullVertex {
    /// Metres from [`HullGeometry::origin`].
    pub position: [f32; 3],
    /// Unit face normal.
    pub normal: [f32; 3],
    /// The layer colour as RGBA, alpha 255.
    pub colour: [u8; 4],
}

/// The hull as vertex data, plain data that can be shared across threads.
pub struct HullGeometry {
    /// What the geometry was built from ([`crate::render::geometry_key`]).
    pub key: u64,
    /// The centre of the box of the emitted vertices; positions are relative to it.
    pub origin: Vec3,
    /// World-space box of the emitted vertices; both zero when there are none.
    pub min: Vec3,
    pub max: Vec3,
    /// Triangle list, not shared.
    pub vertices: Vec<HullVertex>,
    /// Line list relative to `origin`, each emitted face's own outline; empty unless built with
    /// wireframe.
    pub edges: Vec<[f32; 3]>,
}

/// The largest vertex buffer allocated, whatever the device allows.
pub const MAX_BUFFER_BYTES: u64 = 128 * 1024 * 1024;

fn relative(p: Vec3, origin: Vec3) -> [f32; 3] {
    let d = p - origin;
    [d.x as f32, d.y as f32, d.z as f32]
}

/// Build the vertex data for `mesh`, coloured by the layers of `surface`.
pub fn build_geometry(
    mesh: &RenderMesh,
    surface: &Surface,
    key: u64,
    wireframe: bool,
) -> HullGeometry {
    // First pass: the faces that are drawn, with their normals, and the box they span.
    let mut kept: Vec<(usize, Vec3)> = Vec::with_capacity(mesh.faces.len());
    let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut poly: Vec<Vec3> = Vec::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        if f.len() < 3 {
            continue;
        }
        poly.clear();
        poly.extend(f.iter().map(|&i| mesh.verts[i]));
        let normal = export::newell_normal(&poly);
        if normal.length() < 1e-12 {
            continue;
        }
        for p in &poly {
            min = Vec3::new(min.x.min(p.x), min.y.min(p.y), min.z.min(p.z));
            max = Vec3::new(max.x.max(p.x), max.y.max(p.y), max.z.max(p.z));
        }
        kept.push((fi, normal.norm()));
    }
    if kept.is_empty() {
        return HullGeometry {
            key,
            origin: Vec3::ZERO,
            min: Vec3::ZERO,
            max: Vec3::ZERO,
            vertices: Vec::new(),
            edges: Vec::new(),
        };
    }
    let origin = (min + max) * 0.5;

    let mut vertices = Vec::new();
    let mut edges = Vec::new();
    for (fi, normal) in kept {
        let f = &mesh.faces[fi];
        let c = layer_color(surface, mesh.face_layer.get(fi).copied());
        let colour = [c.r(), c.g(), c.b(), 255];
        let n = [normal.x as f32, normal.y as f32, normal.z as f32];
        let vertex = |k: usize| HullVertex {
            position: relative(mesh.verts[f[k]], origin),
            normal: n,
            colour,
        };
        for k in 1..f.len() - 1 {
            vertices.extend([vertex(0), vertex(k), vertex(k + 1)]);
        }
        if wireframe {
            for i in 0..f.len() {
                edges.push(relative(mesh.verts[f[i]], origin));
                edges.push(relative(mesh.verts[f[(i + 1) % f.len()]], origin));
            }
        }
    }
    HullGeometry {
        key,
        origin,
        min,
        max,
        vertices,
        edges,
    }
}

/// Split `count` items of `item_bytes` into contiguous ranges of at most `max_bytes` each, every
/// length a multiple of `unit` (3 for triangles, 2 for lines), so no primitive straddles two
/// buffers. `count` is expected to be a multiple of `unit`. No items give no ranges.
pub fn chunk_ranges(
    count: usize,
    item_bytes: u64,
    unit: usize,
    max_bytes: u64,
) -> Vec<Range<usize>> {
    let unit = unit.max(1);
    let fit = (max_bytes / item_bytes.max(1)) as usize;
    let step = (fit / unit * unit).max(unit);
    let mut ranges = Vec::new();
    let mut start = 0;
    while start < count {
        let end = count.min(start.saturating_add(step));
        ranges.push(start..end);
        start = end;
    }
    ranges
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::mesh::Layer;

    fn v(x: f64, y: f64, z: f64) -> Vec3 {
        Vec3::new(x, y, z)
    }

    fn layer(color: i32) -> Layer {
        Layer {
            name: "l".into(),
            id: 0,
            color,
            visible: true,
            developable: false,
            symmetric: false,
            use_for_intersections: true,
            use_in_hydrostatics: true,
            show_in_linesplan: true,
            density: 0.0,
            thickness: 0.0,
        }
    }

    fn surface(colors: &[i32]) -> Surface {
        Surface {
            layers: colors.iter().map(|&c| layer(c)).collect(),
            ..Surface::default()
        }
    }

    fn mesh(verts: Vec<Vec3>, faces: Vec<Vec<usize>>, layers: Vec<usize>) -> RenderMesh {
        RenderMesh {
            verts,
            faces,
            face_layer: layers,
        }
    }

    fn square(x0: f64) -> Vec<Vec3> {
        vec![
            v(x0, 0.0, 0.0),
            v(x0 + 1.0, 0.0, 0.0),
            v(x0 + 1.0, 1.0, 0.0),
            v(x0, 1.0, 0.0),
        ]
    }

    #[test]
    fn a_triangle_quad_and_pentagon_give_3_6_and_9_vertices() {
        let verts = vec![
            v(0.0, 0.0, 0.0),
            v(1.0, 0.0, 0.0),
            v(1.0, 1.0, 0.0),
            v(0.0, 1.0, 0.0),
            v(-0.5, 0.5, 0.0),
        ];
        let m = mesh(
            verts,
            vec![vec![0, 1, 2], vec![0, 1, 2, 3], vec![0, 1, 2, 3, 4]],
            vec![0, 0, 0],
        );
        let g = build_geometry(&m, &surface(&[0x112233]), 1, false);
        assert_eq!(g.vertices.len(), 18);
        assert_eq!(g.key, 1);
    }

    #[test]
    fn a_quad_fan_covers_its_area() {
        let m = mesh(square(0.0), vec![vec![0, 1, 2, 3]], vec![0]);
        let g = build_geometry(&m, &surface(&[0]), 0, false);
        assert_eq!(g.vertices.len(), 6);
        let p = |i: usize| {
            let a = g.vertices[i].position;
            v(a[0] as f64, a[1] as f64, a[2] as f64)
        };
        let area: f64 = g
            .vertices
            .chunks(3)
            .enumerate()
            .map(|(t, _)| {
                let (a, b, c) = (p(3 * t), p(3 * t + 1), p(3 * t + 2));
                export::newell_normal(&[a, b, c]).length() * 0.5
            })
            .sum();
        assert!((area - 1.0).abs() < 1e-6, "area {area}");
        for vert in &g.vertices {
            let n = vert.normal;
            assert!(n[0].abs() < 1e-6 && n[1].abs() < 1e-6 && (n[2] - 1.0).abs() < 1e-6);
        }
    }

    #[test]
    fn each_vertex_carries_its_layer_colour_opaque() {
        let mut verts = square(0.0);
        verts.extend(square(2.0));
        let m = mesh(verts, vec![vec![0, 1, 2, 3], vec![4, 5, 6, 7]], vec![0, 1]);
        let s = surface(&[0x102030, 0xa0b0c0]);
        let g = build_geometry(&m, &s, 0, false);
        for (i, vert) in g.vertices.iter().enumerate() {
            let layer = i / 6;
            let c = crate::theme::layer_colour(s.layers[layer].color);
            assert_eq!(vert.colour, [c.r(), c.g(), c.b(), 255], "vertex {i}");
        }
        assert_ne!(g.vertices[0].colour, g.vertices[6].colour);
    }

    #[test]
    fn a_face_with_no_valid_layer_takes_the_default_colour() {
        let m = mesh(square(0.0), vec![vec![0, 1, 2, 3]], vec![]);
        let g = build_geometry(&m, &surface(&[0x102030]), 0, false);
        let c = crate::theme::HULL_DEFAULT;
        assert_eq!(g.vertices[0].colour, [c.r(), c.g(), c.b(), 255]);
    }

    #[test]
    fn degenerate_faces_are_skipped() {
        let verts = vec![v(0.0, 0.0, 0.0), v(1.0, 0.0, 0.0), v(2.0, 0.0, 0.0)];
        let m = mesh(verts, vec![vec![0, 1], vec![0, 1, 2]], vec![0, 0]);
        let g = build_geometry(&m, &surface(&[0]), 0, true);
        assert!(g.vertices.is_empty());
        assert!(g.edges.is_empty());
        assert_eq!(g.min, Vec3::ZERO);
        assert_eq!(g.max, Vec3::ZERO);
    }

    #[test]
    fn an_empty_mesh_builds_empty_geometry() {
        let m = mesh(vec![], vec![], vec![]);
        let g = build_geometry(&m, &surface(&[]), 7, true);
        assert!(g.vertices.is_empty() && g.edges.is_empty());
        assert_eq!(g.origin, Vec3::ZERO);
        assert_eq!((g.min, g.max), (Vec3::ZERO, Vec3::ZERO));
    }

    #[test]
    fn positions_are_relative_to_the_box_centre() {
        let m = mesh(square(5000.0), vec![vec![0, 1, 2, 3]], vec![0]);
        let g = build_geometry(&m, &surface(&[0]), 0, false);
        assert!((g.origin.x - 5000.5).abs() < 1e-9);
        assert_eq!(g.min.x, 5000.0);
        assert_eq!(g.max.x, 5001.0);
        for (vert, input) in g.vertices.iter().zip([0usize, 1, 2, 0, 2, 3]) {
            let p = vert.position;
            assert!(p.iter().all(|c| c.abs() <= 0.5 + 1e-6));
            let w = g.origin + v(p[0] as f64, p[1] as f64, p[2] as f64);
            assert!((w - m.verts[input]).length() < 1e-3);
        }
    }

    #[test]
    fn edges_are_built_only_for_wireframe() {
        let m = mesh(square(0.0), vec![vec![0, 1, 2, 3]], vec![0]);
        let s = surface(&[0]);
        assert_eq!(build_geometry(&m, &s, 0, true).edges.len(), 8);
        assert!(build_geometry(&m, &s, 0, false).edges.is_empty());
    }

    #[test]
    fn chunks_stay_under_the_cap_and_lose_nothing() {
        for (count, item_bytes, unit) in [(15_000_000usize, 28u64, 3usize), (15_000_000, 12, 2)] {
            let ranges = chunk_ranges(count, item_bytes, unit, MAX_BUFFER_BYTES);
            assert!(ranges.len() > 1);
            let mut next = 0;
            for r in &ranges {
                assert_eq!(r.start, next);
                assert!((r.len() as u64) * item_bytes <= MAX_BUFFER_BYTES);
                assert_eq!(r.len() % unit, 0);
                next = r.end;
            }
            assert_eq!(next, count);
        }
        assert!(chunk_ranges(0, 28, 3, MAX_BUFFER_BYTES).is_empty());
    }

    #[test]
    fn vertex_is_28_bytes_and_sendable() {
        fn sync<T: Send + Sync>() {}
        sync::<HullGeometry>();
        assert_eq!(std::mem::size_of::<HullVertex>(), 28);
    }
}
