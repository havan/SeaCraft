//! A bounding-volume hierarchy over triangles, for asking whether a straight
//! segment is blocked.
//!
//! The window uses it to tell control net in front of the hull from net behind
//! it: a point is hidden when the segment from the eye to the point meets the
//! refined hull first. Hits are two-sided, and a hit exactly at either end of
//! the segment does not count.

use crate::export::RenderMesh;
use crate::math::Vec3;

const LEAF_SIZE: usize = 4;

/// Axis-aligned bounding box.
#[derive(Debug, Clone, Copy)]
struct Aabb {
    min: Vec3,
    max: Vec3,
}

impl Aabb {
    const EMPTY: Self = Self {
        min: Vec3 {
            x: f64::INFINITY,
            y: f64::INFINITY,
            z: f64::INFINITY,
        },
        max: Vec3 {
            x: f64::NEG_INFINITY,
            y: f64::NEG_INFINITY,
            z: f64::NEG_INFINITY,
        },
    };

    fn from_points(a: Vec3, b: Vec3, c: Vec3) -> Self {
        Self {
            min: a.min(b).min(c),
            max: a.max(b).max(c),
        }
    }

    fn expand_point(&mut self, p: Vec3) {
        self.min = self.min.min(p);
        self.max = self.max.max(p);
    }

    fn union(self, other: Self) -> Self {
        Self {
            min: self.min.min(other.min),
            max: self.max.max(other.max),
        }
    }

    /// Slab intersection along segment `a + d * t` restricted to `t in [0, max_t]`.
    /// Returns the entry parameter `tmin` when the segment overlaps the box.
    fn intersect_segment(&self, a: Vec3, d: Vec3, max_t: f64) -> Option<f64> {
        let mut tmin = 0.0_f64;
        let mut tmax = max_t;
        if !slab_axis(a.x, d.x, self.min.x, self.max.x, &mut tmin, &mut tmax)
            || !slab_axis(a.y, d.y, self.min.y, self.max.y, &mut tmin, &mut tmax)
            || !slab_axis(a.z, d.z, self.min.z, self.max.z, &mut tmin, &mut tmax)
        {
            return None;
        }
        Some(tmin)
    }
}

fn slab_axis(
    origin: f64,
    dir: f64,
    min_b: f64,
    max_b: f64,
    tmin: &mut f64,
    tmax: &mut f64,
) -> bool {
    // Avoid `0.0 * inf = NaN` when the segment is parallel to and on a box face.
    if dir.abs() <= f64::MIN_POSITIVE {
        return origin >= min_b && origin <= max_b;
    }
    let inv = 1.0 / dir;
    let mut t0 = (min_b - origin) * inv;
    let mut t1 = (max_b - origin) * inv;
    if inv < 0.0 {
        std::mem::swap(&mut t0, &mut t1);
    }
    *tmin = tmin.max(t0);
    *tmax = tmax.min(t1);
    *tmin <= *tmax
}

#[derive(Debug, Clone, Copy)]
enum NodeKind {
    Leaf { start: usize, len: usize },
    Inner { left: usize, right: usize },
}

#[derive(Debug, Clone)]
struct Node {
    bounds: Aabb,
    kind: NodeKind,
}

struct TriEntry {
    tri: [usize; 3],
    bounds: Aabb,
    centroid: Vec3,
}

/// Flat AABB bounding-volume hierarchy over a static triangle soup.
#[derive(Debug, Clone)]
pub struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<[usize; 3]>,
    positions: Vec<Vec3>,
}

impl Bvh {
    /// Build a BVH over `positions` and indexed `triangles`.
    pub fn build(positions: &[Vec3], triangles: &[[usize; 3]]) -> Bvh {
        let mut entries: Vec<TriEntry> = triangles
            .iter()
            .copied()
            .filter(|&[i0, i1, i2]| {
                i0 < positions.len() && i1 < positions.len() && i2 < positions.len()
            })
            .map(|tri| {
                let a = positions[tri[0]];
                let b = positions[tri[1]];
                let c = positions[tri[2]];
                TriEntry {
                    tri,
                    bounds: Aabb::from_points(a, b, c),
                    centroid: (a + b + c) * (1.0 / 3.0),
                }
            })
            .collect();

        if entries.is_empty() {
            return Self {
                nodes: Vec::new(),
                tris: Vec::new(),
                positions: Vec::new(),
            };
        }

        let mut nodes = Vec::with_capacity(entries.len() * 2);
        let len = entries.len();
        build_node(&mut entries, 0, len, &mut nodes);
        let tris = entries.into_iter().map(|e| e.tri).collect();
        Self {
            nodes,
            tris,
            positions: positions.to_vec(),
        }
    }

    /// Build a BVH by fan-triangulating every polygon in `mesh`.
    pub fn of_render_mesh(mesh: &RenderMesh) -> Bvh {
        let mut tris = Vec::with_capacity(mesh.faces.len() * 2);
        for f in &mesh.faces {
            for k in 1..f.len().saturating_sub(1) {
                tris.push([f[0], f[k], f[k + 1]]);
            }
        }
        Self::build(&mesh.verts, &tris)
    }

    /// Closest hit parameter `t` in `(0.0, 1.0)` along segment `a -> b`, if any.
    pub fn segment_hit(&self, a: Vec3, b: Vec3) -> Option<f64> {
        if self.nodes.is_empty() {
            return None;
        }
        let d = b - a;
        let mut best_t = 1.0;
        let mut hit = false;
        self.nodes[0].bounds.intersect_segment(a, d, best_t)?;

        let mut stack = Vec::with_capacity(64);
        stack.push(0usize);
        while let Some(idx) = stack.pop() {
            let node = &self.nodes[idx];
            match node.kind {
                NodeKind::Leaf { start, len } => {
                    for &[i0, i1, i2] in &self.tris[start..start + len] {
                        if let Some(t) = segment_triangle(
                            a,
                            d,
                            self.positions[i0],
                            self.positions[i1],
                            self.positions[i2],
                        ) {
                            if t < best_t {
                                best_t = t;
                                hit = true;
                            }
                        }
                    }
                }
                NodeKind::Inner { left, right } => {
                    let tl = self.nodes[left].bounds.intersect_segment(a, d, best_t);
                    let tr = self.nodes[right].bounds.intersect_segment(a, d, best_t);
                    match (tl, tr) {
                        (Some(dl), Some(dr)) => {
                            // Push farther child first so the nearer child is popped first.
                            if dl < dr {
                                stack.push(right);
                                stack.push(left);
                            } else {
                                stack.push(left);
                                stack.push(right);
                            }
                        }
                        (Some(_), None) => stack.push(left),
                        (None, Some(_)) => stack.push(right),
                        (None, None) => {}
                    }
                }
            }
        }
        hit.then_some(best_t)
    }

    /// Whether segment `a -> b` intersects any triangle at `t` in `(0.0, 1.0)`.
    pub fn any_hit(&self, a: Vec3, b: Vec3) -> bool {
        if self.nodes.is_empty() {
            return false;
        }
        let d = b - a;
        if self.nodes[0].bounds.intersect_segment(a, d, 1.0).is_none() {
            return false;
        }

        let mut stack = Vec::with_capacity(64);
        stack.push(0usize);
        while let Some(idx) = stack.pop() {
            let node = &self.nodes[idx];
            match node.kind {
                NodeKind::Leaf { start, len } => {
                    for &[i0, i1, i2] in &self.tris[start..start + len] {
                        if segment_triangle(
                            a,
                            d,
                            self.positions[i0],
                            self.positions[i1],
                            self.positions[i2],
                        )
                        .is_some()
                        {
                            return true;
                        }
                    }
                }
                NodeKind::Inner { left, right } => {
                    if self.nodes[left]
                        .bounds
                        .intersect_segment(a, d, 1.0)
                        .is_some()
                    {
                        stack.push(left);
                    }
                    if self.nodes[right]
                        .bounds
                        .intersect_segment(a, d, 1.0)
                        .is_some()
                    {
                        stack.push(right);
                    }
                }
            }
        }
        false
    }

    /// Whether the BVH contains no triangles.
    pub fn is_empty(&self) -> bool {
        self.tris.is_empty()
    }
}

fn build_node(entries: &mut [TriEntry], start: usize, len: usize, nodes: &mut Vec<Node>) -> usize {
    if len <= LEAF_SIZE {
        let mut bounds = Aabb::EMPTY;
        for e in &entries[start..start + len] {
            bounds = bounds.union(e.bounds);
        }
        let idx = nodes.len();
        nodes.push(Node {
            bounds,
            kind: NodeKind::Leaf { start, len },
        });
        return idx;
    }

    let mut centroid_bounds = Aabb::EMPTY;
    for e in &entries[start..start + len] {
        centroid_bounds.expand_point(e.centroid);
    }
    let extent = centroid_bounds.max - centroid_bounds.min;
    let axis = if extent.x >= extent.y && extent.x >= extent.z {
        0
    } else if extent.y >= extent.z {
        1
    } else {
        2
    };

    let mid = len / 2;
    let slice = &mut entries[start..start + len];
    slice.select_nth_unstable_by(mid, |a, b| {
        let ca = axis_coord(a.centroid, axis);
        let cb = axis_coord(b.centroid, axis);
        ca.total_cmp(&cb)
    });

    let node_idx = nodes.len();
    nodes.push(Node {
        bounds: Aabb::EMPTY,
        kind: NodeKind::Leaf { start: 0, len: 0 },
    });

    let left = build_node(entries, start, mid, nodes);
    let right = build_node(entries, start + mid, len - mid, nodes);
    nodes[node_idx] = Node {
        bounds: nodes[left].bounds.union(nodes[right].bounds),
        kind: NodeKind::Inner { left, right },
    };
    node_idx
}

fn axis_coord(v: Vec3, axis: usize) -> f64 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Two-sided Möller-Trumbore intersection along segment `a + d * t`, returning `t` in `(0.0, 1.0)`.
fn segment_triangle(a: Vec3, d: Vec3, v0: Vec3, v1: Vec3, v2: Vec3) -> Option<f64> {
    let e1 = v1 - v0;
    let e2 = v2 - v0;
    let p = d.cross(e2);
    let det = e1.dot(p);
    if !det.is_finite() || det.abs() <= 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = a - v0;
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = d.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0 && t < 1.0).then_some(t)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn unit_box_triangles() -> (Vec<Vec3>, Vec<[usize; 3]>) {
        let verts = vec![
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, -1.0, -1.0),
            Vec3::new(1.0, 1.0, -1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
        ];
        let tris = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
            [0, 1, 5],
            [0, 5, 4],
            [3, 7, 6],
            [3, 6, 2],
        ];
        (verts, tris)
    }

    struct Lcg(u64);

    impl Lcg {
        fn next_u64(&mut self) -> u64 {
            self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1);
            self.0
        }

        fn next_f64(&mut self, lo: f64, hi: f64) -> f64 {
            let bits = self.next_u64() >> 11;
            let u = (bits as f64) / ((1u64 << 53) as f64);
            lo + (hi - lo) * u
        }

        fn next_vec3(&mut self, lo: f64, hi: f64) -> Vec3 {
            Vec3::new(
                self.next_f64(lo, hi),
                self.next_f64(lo, hi),
                self.next_f64(lo, hi),
            )
        }
    }

    type Scene = (Vec<Vec3>, Vec<[usize; 3]>, Vec<(Vec3, Vec3)>);

    fn lcg_scene(n_tris: usize, n_segs: usize) -> Scene {
        let mut rng = Lcg(0x9e37_79b9_7f4a_7c15);
        let mut verts = Vec::with_capacity(n_tris * 3);
        let mut tris = Vec::with_capacity(n_tris);
        for i in 0..n_tris {
            let c = rng.next_vec3(-2.0, 2.0);
            for _ in 0..3 {
                verts.push(c + rng.next_vec3(-1.0, 1.0));
            }
            let base = i * 3;
            tris.push([base, base + 1, base + 2]);
        }
        let mut segs = Vec::with_capacity(n_segs);
        for _ in 0..n_segs {
            segs.push((rng.next_vec3(-3.0, 3.0), rng.next_vec3(-3.0, 3.0)));
        }
        (verts, tris, segs)
    }

    fn brute_force_hit(
        positions: &[Vec3],
        triangles: &[[usize; 3]],
        a: Vec3,
        b: Vec3,
    ) -> Option<f64> {
        let d = b - a;
        let mut best: Option<f64> = None;
        for &[i0, i1, i2] in triangles {
            if let Some(t) = segment_triangle(a, d, positions[i0], positions[i1], positions[i2]) {
                if best.is_none_or(|bt| t < bt) {
                    best = Some(t);
                }
            }
        }
        best
    }

    #[test]
    fn box_segment_hits_at_quarter_and_beside_misses() {
        // Closed form by hand: box [-1, 1]^3 has near face at x = -1; segment from
        // (-2, 0, 0) to (2, 0, 0) has length 4 and crosses x = -1 at t = 1/4 = 0.25.
        let (verts, tris) = unit_box_triangles();
        let bvh = Bvh::build(&verts, &tris);
        assert!(!bvh.is_empty());
        let t = bvh
            .segment_hit(Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0))
            .unwrap();
        assert!((t - 0.25).abs() < 1e-12, "t = {t}");
        assert!(bvh.any_hit(Vec3::new(-2.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0)));
        assert_eq!(
            bvh.segment_hit(Vec3::new(-2.0, 2.0, 0.0), Vec3::new(2.0, 2.0, 0.0)),
            None
        );
        assert_eq!(
            bvh.segment_hit(Vec3::new(0.0, 0.0, 0.0), Vec3::new(0.0, 0.0, 0.0)),
            None
        );
        assert!(Bvh::build(&[], &[]).is_empty());
        assert_eq!(
            Bvh::build(&[], &[]).segment_hit(Vec3::ZERO, Vec3::new(1.0, 0.0, 0.0)),
            None
        );
    }

    #[test]
    fn lcg_random_triangles_and_segments_match_brute_force() {
        // Internal consistency against brute-force Möller-Trumbore over 200 triangles
        // and 200 segments from fixed LCG state_{n+1} = state_n * 6364136223846793005 + 1.
        let (verts, tris, segs) = lcg_scene(200, 200);
        let bvh = Bvh::build(&verts, &tris);
        for (a, b) in segs {
            let got = bvh.segment_hit(a, b);
            let want = brute_force_hit(&verts, &tris, a, b);
            assert_eq!(got.is_some(), want.is_some());
            if let (Some(g), Some(w)) = (got, want) {
                assert!((g - w).abs() < 1e-11, "{g} vs {w}");
            }
            assert_eq!(bvh.any_hit(a, b), want.is_some());
        }
    }

    #[test]
    fn of_render_mesh_triangulates_quad_and_hits() {
        // Closed form by hand: quad on plane x = 1 covering y, z in [-1, 1];
        // segment (0, 0.5, 0.5) -> (4, 0.5, 0.5) has length 4 and crosses x = 1 at t = 1/4 = 0.25.
        let mesh = RenderMesh {
            verts: vec![
                Vec3::new(1.0, -1.0, -1.0),
                Vec3::new(1.0, 1.0, -1.0),
                Vec3::new(1.0, 1.0, 1.0),
                Vec3::new(1.0, -1.0, 1.0),
            ],
            faces: vec![vec![0, 1, 2, 3]],
            face_layer: vec![0],
        };
        let bvh = Bvh::of_render_mesh(&mesh);
        let t = bvh
            .segment_hit(Vec3::new(0.0, 0.5, 0.5), Vec3::new(4.0, 0.5, 0.5))
            .unwrap();
        assert!((t - 0.25).abs() < 1e-12, "t = {t}");
    }
}
