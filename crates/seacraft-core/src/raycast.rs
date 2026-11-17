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
/// How many nodes a traversal can have waiting; the builder keeps the tree no deeper than this.
const STACK: usize = 64;
/// The levels of the tree that may split at the spatial midpoint.
const SPATIAL_LEVELS: usize = 24;

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

    /// Slab intersection using precomputed reciprocal directions (`0.0` when parallel).
    #[inline]
    fn intersect_precomputed(
        &self,
        a: Vec3,
        inv_d: Vec3,
        parallel: [bool; 3],
        max_t: f64,
    ) -> Option<f64> {
        let mut tmin = 0.0_f64;
        let mut tmax = max_t;
        if !slab_axis_inv(
            a.x,
            inv_d.x,
            parallel[0],
            self.min.x,
            self.max.x,
            &mut tmin,
            &mut tmax,
        ) || !slab_axis_inv(
            a.y,
            inv_d.y,
            parallel[1],
            self.min.y,
            self.max.y,
            &mut tmin,
            &mut tmax,
        ) || !slab_axis_inv(
            a.z,
            inv_d.z,
            parallel[2],
            self.min.z,
            self.max.z,
            &mut tmin,
            &mut tmax,
        ) {
            return None;
        }
        Some(tmin)
    }
}

#[inline]
fn precompute_dir(d: Vec3) -> (Vec3, [bool; 3]) {
    let px = d.x.abs() <= f64::MIN_POSITIVE;
    let py = d.y.abs() <= f64::MIN_POSITIVE;
    let pz = d.z.abs() <= f64::MIN_POSITIVE;
    let inv = Vec3::new(
        if px { 0.0 } else { 1.0 / d.x },
        if py { 0.0 } else { 1.0 / d.y },
        if pz { 0.0 } else { 1.0 / d.z },
    );
    (inv, [px, py, pz])
}

#[inline]
fn slab_axis_inv(
    origin: f64,
    inv: f64,
    parallel: bool,
    min_b: f64,
    max_b: f64,
    tmin: &mut f64,
    tmax: &mut f64,
) -> bool {
    // Avoid `0.0 * inf = NaN` when the segment is parallel to and on a box face.
    if parallel {
        return origin >= min_b && origin <= max_b;
    }
    let t0 = (min_b - origin) * inv;
    let t1 = (max_b - origin) * inv;
    *tmin = tmin.max(t0.min(t1));
    *tmax = tmax.min(t0.max(t1));
    *tmin <= *tmax
}

#[derive(Debug, Clone, Copy)]
enum NodeKind {
    Leaf { start: u32, len: u32 },
    Inner { left: u32, right: u32 },
}

#[derive(Debug, Clone)]
struct Node {
    bounds: Aabb,
    kind: NodeKind,
}

#[derive(Debug, Clone, Copy)]
struct Tri {
    v0: Vec3,
    e1: Vec3,
    e2: Vec3,
}

struct TriEntry {
    tri: Tri,
    bounds: Aabb,
    centroid: Vec3,
}

/// Flat AABB bounding-volume hierarchy over a static triangle soup.
#[derive(Debug, Clone)]
pub struct Bvh {
    nodes: Vec<Node>,
    tris: Vec<Tri>,
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
                    tri: Tri {
                        v0: a,
                        e1: b - a,
                        e2: c - a,
                    },
                    bounds: Aabb::from_points(a, b, c),
                    centroid: (a + b + c) * (1.0 / 3.0),
                }
            })
            .collect();

        if entries.is_empty() {
            return Self {
                nodes: Vec::new(),
                tris: Vec::new(),
            };
        }

        let mut root_centroids = Aabb::EMPTY;
        for e in &entries {
            root_centroids.expand_point(e.centroid);
        }
        let root_ext = root_centroids.max - root_centroids.min;
        let scale = Vec3::new(
            0.2 / root_ext.x.max(1e-9),
            1.0 / root_ext.y.max(1e-9),
            1.0 / root_ext.z.max(1e-9),
        );

        let mut nodes = Vec::with_capacity(entries.len() * 2);
        let len = entries.len();
        build_node(&mut entries, 0, len, scale, 0, &mut nodes);
        let tris = entries.into_iter().map(|e| e.tri).collect();
        Self { nodes, tris }
    }

    /// The number of nodes on the longest way from the root to a leaf.
    #[cfg(test)]
    fn depth(&self) -> usize {
        fn below(nodes: &[Node], at: usize) -> usize {
            match nodes[at].kind {
                NodeKind::Leaf { .. } => 1,
                NodeKind::Inner { left, right } => {
                    1 + below(nodes, left as usize).max(below(nodes, right as usize))
                }
            }
        }
        if self.nodes.is_empty() {
            0
        } else {
            below(&self.nodes, 0)
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
        let (inv_d, parallel) = precompute_dir(d);
        let mut best_t = 1.0;
        let mut hit = false;
        self.nodes[0]
            .bounds
            .intersect_precomputed(a, inv_d, parallel, best_t)?;

        let mut stack = [0u32; STACK];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let idx = stack[sp] as usize;
            let node = &self.nodes[idx];
            match node.kind {
                NodeKind::Leaf { start, len } => {
                    let s = start as usize;
                    for tri in &self.tris[s..s + len as usize] {
                        if let Some(t) = segment_triangle(a, d, tri) {
                            if t < best_t {
                                best_t = t;
                                hit = true;
                            }
                        }
                    }
                }
                NodeKind::Inner { left, right } => {
                    let tl = self.nodes[left as usize]
                        .bounds
                        .intersect_precomputed(a, inv_d, parallel, best_t);
                    let tr = self.nodes[right as usize]
                        .bounds
                        .intersect_precomputed(a, inv_d, parallel, best_t);
                    match (tl, tr) {
                        (Some(dl), Some(dr)) => {
                            // Push farther child first so the nearer child is popped first.
                            if dl < dr {
                                stack[sp] = right;
                                stack[sp + 1] = left;
                            } else {
                                stack[sp] = left;
                                stack[sp + 1] = right;
                            }
                            sp += 2;
                        }
                        (Some(_), None) => {
                            stack[sp] = left;
                            sp += 1;
                        }
                        (None, Some(_)) => {
                            stack[sp] = right;
                            sp += 1;
                        }
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
        let (inv_d, parallel) = precompute_dir(d);
        if self.nodes[0]
            .bounds
            .intersect_precomputed(a, inv_d, parallel, 1.0)
            .is_none()
        {
            return false;
        }

        let mut stack = [0u32; STACK];
        let mut sp = 1usize;
        while sp > 0 {
            sp -= 1;
            let idx = stack[sp] as usize;
            let node = &self.nodes[idx];
            match node.kind {
                NodeKind::Leaf { start, len } => {
                    let s = start as usize;
                    for tri in &self.tris[s..s + len as usize] {
                        if segment_triangle(a, d, tri).is_some() {
                            return true;
                        }
                    }
                }
                NodeKind::Inner { left, right } => {
                    let tl = self.nodes[left as usize]
                        .bounds
                        .intersect_precomputed(a, inv_d, parallel, 1.0);
                    let tr = self.nodes[right as usize]
                        .bounds
                        .intersect_precomputed(a, inv_d, parallel, 1.0);
                    match (tl, tr) {
                        (Some(dl), Some(dr)) => {
                            if dl < dr {
                                stack[sp] = right;
                                stack[sp + 1] = left;
                            } else {
                                stack[sp] = left;
                                stack[sp + 1] = right;
                            }
                            sp += 2;
                        }
                        (Some(_), None) => {
                            stack[sp] = left;
                            sp += 1;
                        }
                        (None, Some(_)) => {
                            stack[sp] = right;
                            sp += 1;
                        }
                        (None, None) => {}
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

fn build_node(
    entries: &mut [TriEntry],
    start: usize,
    len: usize,
    scale: Vec3,
    depth: usize,
    nodes: &mut Vec<Node>,
) -> u32 {
    if len <= LEAF_SIZE {
        let mut bounds = Aabb::EMPTY;
        for e in &entries[start..start + len] {
            bounds = bounds.union(e.bounds);
        }
        let idx = nodes.len() as u32;
        nodes.push(Node {
            bounds,
            kind: NodeKind::Leaf {
                start: start as u32,
                len: len as u32,
            },
        });
        return idx;
    }

    let mut centroid_bounds = Aabb::EMPTY;
    for e in &entries[start..start + len] {
        centroid_bounds.expand_point(e.centroid);
    }
    let extent = centroid_bounds.max - centroid_bounds.min;
    let sx = extent.x * scale.x;
    let sy = extent.y * scale.y;
    let sz = extent.z * scale.z;
    let axis = if sx >= sy && sx >= sz {
        0
    } else if sy >= sz {
        1
    } else {
        2
    };

    let mid_coord =
        0.5 * (axis_coord(centroid_bounds.min, axis) + axis_coord(centroid_bounds.max, axis));
    let slice = &mut entries[start..start + len];
    let mut i = 0usize;
    let mut j = len;
    while i < j {
        if axis_coord(slice[i].centroid, axis) < mid_coord {
            i += 1;
        } else {
            j -= 1;
            slice.swap(i, j);
        }
    }
    let margin = (len / 8).max(1);
    // A spatial split may leave up to 7/8 on one side, which alone would let a
    // crowded mesh grow a tree deeper than a traversal's stack. Past
    // `SPATIAL_LEVELS` every split is at the median, which halves the count:
    // at most 32 more levels for any mesh a `u32` can index, 56 in all.
    let mid = if depth >= SPATIAL_LEVELS || i < margin || i > len - margin {
        let m = len / 2;
        slice.select_nth_unstable_by(m, |a, b| {
            axis_coord(a.centroid, axis).total_cmp(&axis_coord(b.centroid, axis))
        });
        m
    } else {
        i
    };

    let node_idx = nodes.len();
    nodes.push(Node {
        bounds: Aabb::EMPTY,
        kind: NodeKind::Leaf { start: 0, len: 0 },
    });

    let left = build_node(entries, start, mid, scale, depth + 1, nodes);
    let right = build_node(entries, start + mid, len - mid, scale, depth + 1, nodes);
    nodes[node_idx] = Node {
        bounds: nodes[left as usize]
            .bounds
            .union(nodes[right as usize].bounds),
        kind: NodeKind::Inner { left, right },
    };
    node_idx as u32
}

fn axis_coord(v: Vec3, axis: usize) -> f64 {
    match axis {
        0 => v.x,
        1 => v.y,
        _ => v.z,
    }
}

/// Two-sided Möller-Trumbore intersection along segment `a + d * t`, returning `t` in `(0.0, 1.0)`.
#[inline]
fn segment_triangle(a: Vec3, d: Vec3, tri: &Tri) -> Option<f64> {
    let p = d.cross(tri.e2);
    let det = tri.e1.dot(p);
    if !det.is_finite() || det.abs() <= 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = a - tri.v0;
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(tri.e1);
    let v = d.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = tri.e2.dot(q) * inv;
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
            let v0 = positions[i0];
            let tri = Tri {
                v0,
                e1: positions[i1] - v0,
                e2: positions[i2] - v0,
            };
            if let Some(t) = segment_triangle(a, d, &tri) {
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
    #[test]
    fn a_lopsided_mesh_does_not_outgrow_the_traversal_stack() {
        // Triangles whose centroids crowd towards x = 0 as u^4.596 (u uniform): the spatial midpoint then
        // leaves 0.86 of them on one side at every level (0.5^(1 / 4.596) = 0.86), just inside the most
        // lopsided split the builder accepts. Without a cap that is ln(n / 4) / ln(1 / 0.86) = 79 levels for
        // 600 000 triangles, more than the 64 the traversal keeps on its stack. A segment along x passes
        // through every level.
        let n = 600_000usize;
        let mut verts = Vec::with_capacity(n * 3);
        let mut tris = Vec::with_capacity(n);
        for k in 0..n {
            let x = ((k as f64 + 0.5) / n as f64).powf(4.596);
            verts.push(Vec3::new(x, -1e-3, -1e-3));
            verts.push(Vec3::new(x, 1e-3, -1e-3));
            verts.push(Vec3::new(x, 0.0, 1e-3));
            tris.push([3 * k, 3 * k + 1, 3 * k + 2]);
        }
        let bvh = Bvh::build(&verts, &tris);
        assert!(bvh.depth() <= STACK, "depth {}", bvh.depth());
        let (a, b) = (Vec3::new(-1.0, 0.0, 0.0), Vec3::new(2.0, 0.0, 0.0));
        assert!(bvh.any_hit(a, b));
        // The nearest triangle is the first one, at x = (0.5 / n)^4.596, effectively 0: t = 1 / 3.
        let t = bvh.segment_hit(a, b).unwrap();
        assert!((t - 1.0 / 3.0).abs() < 1e-9, "{t}");
        // From the far end the nearest is the last one, at x = (1 - 0.5 / n)^4.596.
        let last = (1.0 - 0.5 / n as f64).powf(4.596);
        let t = bvh.segment_hit(b, a).unwrap();
        assert!((t - (2.0 - last) / 3.0).abs() < 1e-9, "{t}");
    }
}
