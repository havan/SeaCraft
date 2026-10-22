//! Subdivision surface kernel.
//!
//! This reproduces the scheme used by FREE!ship (and its DELFTship/FreeShip+
//! descendants). It is a hybrid Loop / Catmull-Clark refinement:
//!
//! * an edge point is always the edge midpoint (`CalculateEdgePoint`),
//! * a face point is the face centroid; triangles only get one in
//!   Catmull-Clark mode, so in Loop mode a triangle splits into four
//!   triangles (`CalculateFacePoint`),
//! * a *vertex* point starts as a copy of its parent and is then displaced by
//!   the averaging pass at the end of every refinement step (`Averaging`):
//!   `P' = P + a * (C - P)` where `C` is the angle weighted average of the
//!   adjacent face centres and `a` depends on the face valences,
//! * crease and boundary vertices are interpolated with the cubic B-spline
//!   crease rule `P' = 0.5 P + 0.25 (N1 + N2)`,
//! * corner vertices never move.
//!
//! The exact constants matter for hull fairness, so they follow the original
//! source (see `mg_freeship::TFreeSubdivisionPoint.Averaging` in FreeGeometry.pas).

use crate::math::Vec3;
use crate::mesh::{ordered, Surface, VertexKind};
use std::collections::{HashMap, HashSet};
use std::f64::consts::PI;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubdivisionMode {
    /// Triangles stay triangles (Loop); polygons with >3 points become quads.
    Loop,
    /// Every face gets a centroid, so triangles become three quads.
    CatmullClark,
}

impl Default for SubdivisionMode {
    fn default() -> Self {
        SubdivisionMode::Loop
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SubdivisionOptions {
    pub level: u8,
    pub mode: SubdivisionMode,
}

impl Default for SubdivisionOptions {
    fn default() -> Self {
        Self { level: 3, mode: SubdivisionMode::Loop }
    }
}

/// A polygonal mesh carrying everything the refinement needs.
#[derive(Debug, Clone, Default)]
pub struct SubdivMesh {
    pub verts: Vec<Vec3>,
    pub faces: Vec<Vec<usize>>,
    /// Layer index per face (parallel to `faces`).
    pub face_layer: Vec<usize>,
    /// Explicitly creased edges, as ordered index pairs.
    pub creases: HashSet<(usize, usize)>,
    /// Per-vertex classification hints (kept across refinement levels).
    pub hints: Vec<VertexKind>,
}

impl SubdivMesh {
    pub fn from_surface(surface: &Surface) -> Self {
        Self {
            verts: surface.points.iter().map(|p| p.p).collect(),
            faces: surface.faces.clone(),
            face_layer: surface.face_layer.clone(),
            creases: surface
                .edges
                .iter()
                .filter(|e| e.crease)
                .map(|e| e.key())
                .collect(),
            hints: surface.points.iter().map(|p| p.kind).collect(),
        }
    }

    pub fn face_count(&self) -> usize {
        self.faces.len()
    }

    pub fn vertex_count(&self) -> usize {
        self.verts.len()
    }

    /// Edge incidence: ordered endpoint pair -> indices of adjacent faces.
    fn edge_faces(&self) -> (HashMap<(usize, usize), Vec<usize>>, Vec<(usize, usize)>) {
        let mut map: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (fi, f) in self.faces.iter().enumerate() {
            for i in 0..f.len() {
                let k = ordered(f[i], f[(i + 1) % f.len()]);
                map.entry(k).or_default().push(fi);
            }
        }
        let mut keys: Vec<(usize, usize)> = map.keys().copied().collect();
        keys.sort_unstable();
        (map, keys)
    }

    /// Split every face once.
    pub fn refine_once(&self, mode: SubdivisionMode) -> SubdivMesh {
        if self.faces.is_empty() {
            return self.clone();
        }
        let (efaces, _keys) = self.edge_faces();
        let nv = self.verts.len();

        // Stable order for edge points.
        let mut edge_keys: Vec<(usize, usize)> = efaces.keys().copied().collect();
        edge_keys.sort_unstable();
        let edge_index: HashMap<(usize, usize), usize> =
            edge_keys.iter().enumerate().map(|(i, k)| (*k, i)).collect();

        let hard_edge = |k: &(usize, usize)| -> bool {
            self.creases.contains(k) || efaces.get(k).map(|f| f.len() != 2).unwrap_or(false)
        };

        // ---- new points -------------------------------------------------
        let mut new_verts: Vec<Vec3> = Vec::with_capacity(nv + edge_keys.len() + self.faces.len());
        let mut new_hints: Vec<VertexKind> = Vec::with_capacity(new_verts.capacity());

        // vertex points: copies of the old vertices, displaced by averaging
        for (i, v) in self.verts.iter().enumerate() {
            new_verts.push(*v);
            new_hints.push(self.hints.get(i).copied().unwrap_or(VertexKind::Regular));
        }
        // edge points: midpoints
        for k in &edge_keys {
            new_verts.push((self.verts[k.0] + self.verts[k.1]) * 0.5);
            new_hints.push(if hard_edge(k) { VertexKind::Crease } else { VertexKind::Regular });
        }
        // face points
        let mut face_point: Vec<Option<usize>> = vec![None; self.faces.len()];
        for (fi, f) in self.faces.iter().enumerate() {
            let wants = f.len() > 3 || mode == SubdivisionMode::CatmullClark;
            if wants {
                let mut c = Vec3::ZERO;
                for &p in f {
                    c += self.verts[p];
                }
                c = c / f.len() as f64;
                face_point[fi] = Some(new_verts.len());
                new_verts.push(c);
                new_hints.push(VertexKind::Regular);
            }
        }

        // ---- new faces --------------------------------------------------
        let mut new_faces: Vec<Vec<usize>> = Vec::with_capacity(self.faces.len() * 4);
        let mut new_layers: Vec<usize> = Vec::with_capacity(self.faces.len() * 4);
        for (fi, f) in self.faces.iter().enumerate() {
            let n = f.len();
            let ev: Vec<usize> = (0..n)
                .map(|i| nv + edge_index[&ordered(f[i], f[(i + 1) % n])])
                .collect();
            match face_point[fi] {
                Some(fp) => {
                    for i in 0..n {
                        let prev = if i == 0 { n - 1 } else { i - 1 };
                        new_faces.push(vec![f[i], ev[i], fp, ev[prev]]);
                        new_layers.push(self.face_layer[fi]);
                    }
                }
                None => {
                    debug_assert_eq!(n, 3);
                    for i in 0..3 {
                        new_faces.push(vec![f[i], ev[i], ev[(i + 2) % 3]]);
                        new_layers.push(self.face_layer[fi]);
                    }
                    new_faces.push(vec![ev[0], ev[1], ev[2]]);
                    new_layers.push(self.face_layer[fi]);
                }
            }
        }

        // ---- creases propagate to the two child edges of each hard edge ---
        let mut new_creases: HashSet<(usize, usize)> = HashSet::new();
        for k in &edge_keys {
            if hard_edge(k) {
                let e = nv + edge_index[k];
                new_creases.insert(ordered(k.0, e));
                new_creases.insert(ordered(e, k.1));
            }
        }

        let mut mesh = SubdivMesh {
            verts: new_verts,
            faces: new_faces,
            face_layer: new_layers,
            creases: new_creases,
            hints: new_hints,
        };
        mesh.average();
        mesh
    }

    /// The FREE!ship "averaging procedure": one simultaneous smoothing pass
    /// over every point of the freshly refined mesh.
    fn average(&mut self) {
        let (efaces, keys) = self.edge_faces();
        let n = self.verts.len();

        // adjacency
        let mut point_faces: Vec<Vec<usize>> = vec![Vec::new(); n];
        for (fi, f) in self.faces.iter().enumerate() {
            for &p in f {
                point_faces[p].push(fi);
            }
        }
        let mut crease_nbrs: Vec<Vec<usize>> = vec![Vec::new(); n];
        let mut is_boundary = vec![false; n];
        for k in &keys {
            let faces = &efaces[k];
            if faces.len() == 1 {
                is_boundary[k.0] = true;
                is_boundary[k.1] = true;
            }
            if self.creases.contains(k) || faces.len() != 2 {
                crease_nbrs[k.0].push(k.1);
                crease_nbrs[k.1].push(k.0);
            }
        }

        let old = self.verts.clone();
        let mut out = old.clone();

        for i in 0..n {
            let kind = if is_boundary[i] || self.hints.get(i) == Some(&VertexKind::Corner) {
                VertexKind::Corner
            } else if self.hints.get(i) == Some(&VertexKind::Crease)
                || crease_nbrs[i].len() == 2
            {
                VertexKind::Crease
            } else {
                VertexKind::Regular
            };
            match kind {
                VertexKind::Corner => {}
                VertexKind::Crease => {
                    let nb = &crease_nbrs[i];
                    if nb.len() >= 2 {
                        let mut acc = old[i] * 0.5;
                        for &j in nb {
                            acc += old[j] * 0.25;
                        }
                        out[i] = acc;
                    }
                }
                _ => {
                    let faces = &point_faces[i];
                    if faces.is_empty() {
                        continue;
                    }
                    let mut centre = Vec3::ZERO;
                    let mut total_w = 0.0;
                    let mut nq = 0usize;
                    let mut nt = 0usize;
                    let mut valid = true;
                    for &fi in faces {
                        let f = &self.faces[fi];
                        match f.len() {
                            3 => {
                                let mut c = Vec3::ZERO;
                                for &p in f {
                                    let w = if p == i { 1.0 / 4.0 } else { 3.0 / 8.0 };
                                    c += old[p] * w;
                                }
                                let w = PI / 3.0;
                                centre += c * w;
                                total_w += w;
                                nt += 1;
                            }
                            4 => {
                                let mut c = Vec3::ZERO;
                                for &p in f {
                                    c += old[p] * 0.25;
                                }
                                let w = PI / 2.0;
                                centre += c * w;
                                total_w += w;
                                nq += 1;
                            }
                            _ => {
                                valid = false;
                            }
                        }
                    }
                    if !valid || total_w == 0.0 {
                        continue;
                    }
                    centre = centre / total_w;
                    let nf = faces.len() as f64;
                    let a = if nq == 0 {
                        5.0 / 3.0
                            - 8.0 / 3.0 * (3.0 / 8.0 + 0.25 * (2.0 * PI / nf).cos()).powi(2)
                    } else if nt == 0 {
                        4.0 / nf
                    } else if nq == 0 && nt == 3 {
                        1.5
                    } else {
                        12.0 / (3.0 * nq as f64 + 2.0 * nt as f64)
                    };
                    out[i] = old[i] + (centre - old[i]) * a;
                }
            }
        }
        self.verts = out;
    }

    /// Mirror every face whose layer is marked symmetric across `y = 0`,
    /// reversing the winding so the normal keeps pointing outwards.
    pub fn mirror_symmetric(&mut self, layer_symmetric: &[bool]) {
        let mut map: HashMap<usize, usize> = HashMap::new();
        let mut new_faces: Vec<Vec<usize>> = Vec::new();
        let mut new_layers: Vec<usize> = Vec::new();
        let mut added: Vec<Vec3> = Vec::new();
        for (fi, f) in self.faces.iter().enumerate() {
            let li = self.face_layer[fi];
            if !layer_symmetric.get(li).copied().unwrap_or(false) {
                continue;
            }
            let mut nf = Vec::with_capacity(f.len());
            for &p in f.iter().rev() {
                let idx = match map.get(&p) {
                    Some(&i) => i,
                    None => {
                        let mut q = self.verts[p];
                        q.y = -q.y;
                        let i = self.verts.len() + added.len();
                        added.push(q);
                        map.insert(p, i);
                        i
                    }
                };
                nf.push(idx);
            }
            new_faces.push(nf);
            new_layers.push(li);
        }
        self.verts.extend(added);
        self.faces.extend(new_faces);
        self.face_layer.extend(new_layers);
        self.hints.resize(self.verts.len(), VertexKind::Regular);
    }

    /// Fan-triangulate all faces.
    pub fn triangles(&self) -> Vec<[usize; 3]> {
        let mut out = Vec::new();
        for f in &self.faces {
            for i in 1..f.len().saturating_sub(1) {
                out.push([f[0], f[i], f[i + 1]]);
            }
        }
        out
    }

    pub fn edges(&self) -> Vec<(usize, usize)> {
        let (_, keys) = self.edge_faces();
        keys
    }

    pub fn extents(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for v in &self.verts {
            min = min.min(*v);
            max = max.max(*v);
        }
        if !min.is_finite() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (min, max)
        }
    }

    pub fn contains_nan(&self) -> bool {
        self.verts.iter().any(|v| !v.is_finite())
    }

    /// Largest deviation from the control net extents, used by the tests to
    /// check that refinement does not blow up.
    pub fn surface_area(&self) -> f64 {
        self.faces.iter().map(|f| crate::math::polygon_area3(&f.iter().map(|&i| self.verts[i]).collect::<Vec<_>>())).sum()
    }

    /// True when every interior edge has exactly two adjacent faces.
    pub fn is_watertight(&self) -> bool {
        let (efaces, _) = self.edge_faces();
        efaces.values().all(|f| f.len() == 2)
    }
}

/// Refine the control net to the requested level.
pub fn evaluate(surface: &Surface, opts: SubdivisionOptions) -> SubdivMesh {
    let mut mesh = SubdivMesh::from_surface(surface);
    for _ in 0..opts.level.min(6) {
        mesh = mesh.refine_once(opts.mode);
    }
    mesh
}

/// Refine `levels` times and mirror symmetric layers, giving the full hull.
pub fn evaluate_full(surface: &Surface, opts: SubdivisionOptions) -> SubdivMesh {
    let mut mesh = evaluate(surface, opts);
    if opts.level > 0 {
        let sym: Vec<bool> = surface.layers.iter().map(|l| l.symmetric).collect();
        mesh.mirror_symmetric(&sym);
    }
    mesh
}

#[cfg(test)]
fn box_surface(crease_all: bool) -> Surface {
    let mut s = Surface::new();
    let pts = [
        (0.0, 0.0, 0.0),
        (2.0, 0.0, 0.0),
        (2.0, 1.0, 0.0),
        (0.0, 1.0, 0.0),
        (0.0, 0.0, 1.0),
        (2.0, 0.0, 1.0),
        (2.0, 1.0, 1.0),
        (0.0, 1.0, 1.0),
    ];
    for p in pts {
        s.add_point(Vec3::new(p.0, p.1, p.2));
    }
    let quads = [
        vec![0, 3, 2, 1], // bottom
        vec![4, 5, 6, 7], // top
        vec![0, 1, 5, 4], // y=0 side
        vec![1, 2, 6, 5], // x=2 side
        vec![2, 3, 7, 6], // y=1 side
        vec![3, 0, 4, 7], // x=0 side
    ];
    for q in quads {
        s.add_face(q, 0);
    }
    if crease_all {
        s.rebuild_edges();
        for e in s.edges.iter_mut() {
            e.crease = true;
        }
        for p in s.points.iter_mut() {
            p.kind = VertexKind::Corner;
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn creased_box_is_preserved() {
        let s = box_surface(true);
        let m = evaluate(
            &s,
            SubdivisionOptions { level: 3, mode: SubdivisionMode::Loop },
        );
        let (min, max) = m.extents();
        assert!((min - Vec3::new(0.0, 0.0, 0.0)).length() < 1e-9, "min {:?}", min);
        assert!((max - Vec3::new(2.0, 1.0, 1.0)).length() < 1e-9, "max {:?}", max);
        // Volume of the refined mesh must stay that of the box.
        let mut vol = 0.0;
        for t in m.triangles() {
            let (a, b, c) = (m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]);
            vol += a.dot(b.cross(c)) / 6.0;
        }
        assert!((vol.abs() - 2.0).abs() < 1e-6, "volume {}", vol);
    }

    #[test]
    fn smooth_box_shrinks_towards_the_limit_surface() {
        let s = box_surface(false);
        let m = evaluate(
            &s,
            SubdivisionOptions { level: 3, mode: SubdivisionMode::Loop },
        );
        let (min, max) = m.extents();
        // Every original corner is a boundary vertex, so it stays pinned; the
        // faces bulge inwards, hence the volume drops.
        assert!(min.z >= -1e-9);
        assert!(max.z <= 1.0 + 1e-9);
        let mut vol = 0.0;
        for t in m.triangles() {
            let (a, b, c) = (m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]);
            vol += a.dot(b.cross(c)) / 6.0;
        }
        assert!(vol > 0.0 && vol < 2.0, "volume {}", vol);
    }

    #[test]
    fn loop_mode_splits_triangles_into_four() {
        let mut s = Surface::new();
        for p in [(0.0, 0.0, 0.0), (1.0, 0.0, 0.0), (0.0, 1.0, 0.0)] {
            s.add_point(Vec3::new(p.0, p.1, p.2));
        }
        s.add_face(vec![0, 1, 2], 0);
        s.initialize_trace();
        let m = evaluate(&s, SubdivisionOptions { level: 2, mode: SubdivisionMode::Loop });
        assert_eq!(m.faces.len(), 16);
        let mc = evaluate(&s, SubdivisionOptions { level: 2, mode: SubdivisionMode::CatmullClark });
        assert_eq!(mc.faces.len(), 12);
    }

    #[test]
    fn mirroring_symmetric_layers_makes_a_closed_symmetric_surface() {
        // A creased box kept exact through subdivision: mirroring across y gives
        // a 2x2x1 block, so the volume doubles and the mesh stays watertight.
        let mut s = box_surface(true);
        s.layers[0].symmetric = true;
        let m = evaluate_full(&s, SubdivisionOptions { level: 2, mode: SubdivisionMode::Loop });
        let (min, max) = m.extents();
        assert!((min.y + max.y).abs() < 1e-12, "min {:?} max {:?}", min, max);
        assert!(min.y < -0.999 && max.y > 0.999);
        let mut vol = 0.0;
        for t in m.triangles() {
            let (a, b, c) = (m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]);
            vol += a.dot(b.cross(c)) / 6.0;
        }
        assert!((vol.abs() - 4.0).abs() < 1e-6, "volume {}", vol);
        // Watertight: every edge shared by exactly two triangles.
        let mut counts = std::collections::HashMap::new();
        for t in m.triangles() {
            for k in 0..3 {
                let key = crate::mesh::ordered(t[k], t[(k + 1) % 3]);
                *counts.entry(key).or_insert(0usize) += 1;
            }
        }
        assert!(counts.values().all(|&c| c == 2), "open edges: {}", counts.values().filter(|&&c| c != 2).count());
    }
}
