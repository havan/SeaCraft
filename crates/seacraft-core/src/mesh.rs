//! Control net (the editable hull definition) and layer bookkeeping.
//!
//! The control net mirrors FREE!ship's model: a polygonal control mesh made of
//! control points, control edges (optionally *crease* edges) and control faces
//! grouped into layers. Layers may be marked `symmetric`, in which case only
//! `y >= 0` is modelled and the surface is mirrored across the centreline when
//! it is evaluated, exported or measured.

use crate::math::{polygon_area3, Vec3};
use std::collections::HashMap;

/// Vertex classification used by the subdivision scheme.
///
/// * `Regular` - smoothed by the Loop/Catmull-Clark style rules
/// * `Crease`  - lies on a crease (hard) edge, interpolated along the crease
/// * `Corner`  - pinned in place (model boundary or user defined)
/// * `Dart`    - crease tip; treated as regular by the averaging pass
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum VertexKind {
    #[default]
    Regular,
    Crease,
    Dart,
    Corner,
}

impl VertexKind {
    pub fn code(self) -> u8 {
        match self {
            VertexKind::Regular => 0,
            VertexKind::Crease => 1,
            VertexKind::Dart => 2,
            VertexKind::Corner => 3,
        }
    }
    pub fn from_code(c: u8) -> Self {
        match c {
            1 => VertexKind::Crease,
            2 => VertexKind::Dart,
            3 => VertexKind::Corner,
            _ => VertexKind::Regular,
        }
    }
}

#[derive(Debug, Clone)]
pub struct ControlPoint {
    pub p: Vec3,
    pub kind: VertexKind,
}

#[derive(Debug, Clone)]
pub struct ControlEdge {
    pub a: usize,
    pub b: usize,
    pub crease: bool,
}

impl ControlEdge {
    pub fn key(&self) -> (usize, usize) {
        ordered(self.a, self.b)
    }
}

#[derive(Debug, Clone)]
pub struct Layer {
    pub name: String,
    pub id: i32,
    pub color: i32,
    pub visible: bool,
    pub developable: bool,
    pub symmetric: bool,
    pub use_for_intersections: bool,
    pub use_in_hydrostatics: bool,
    pub show_in_linesplan: bool,
    pub density: f64,
    pub thickness: f64,
}

impl Default for Layer {
    fn default() -> Self {
        Self {
            name: "Layer 1".into(),
            id: 1,
            color: 0x00_80_ff,
            visible: true,
            developable: false,
            symmetric: true,
            use_for_intersections: true,
            use_in_hydrostatics: true,
            show_in_linesplan: true,
            density: 7850.0,
            thickness: 0.005,
        }
    }
}

/// Project metadata, the equivalent of FREE!ship's `TFreeProjectSettings`.
#[derive(Debug, Clone)]
pub struct ProjectSettings {
    pub name: String,
    pub designer: String,
    pub created_by: String,
    pub comment: String,
    pub length: f64,
    pub beam: f64,
    pub draft: f64,
    pub water_density: f64,
    pub appendage_coefficient: f64,
    pub imperial: bool,
    pub main_particulars_set: bool,
    pub mainframe_location: f64,
}

impl Default for ProjectSettings {
    fn default() -> Self {
        Self {
            name: "Untitled".into(),
            designer: String::new(),
            created_by: "SeaCraft".into(),
            comment: String::new(),
            length: 0.0,
            beam: 0.0,
            draft: 0.0,
            water_density: 1025.0,
            appendage_coefficient: 1.0,
            imperial: false,
            main_particulars_set: false,
            mainframe_location: 0.0,
        }
    }
}

pub fn ordered(a: usize, b: usize) -> (usize, usize) {
    if a <= b {
        (a, b)
    } else {
        (b, a)
    }
}

/// The editable definition of a hull.
#[derive(Debug, Clone, Default)]
pub struct Surface {
    pub points: Vec<ControlPoint>,
    pub edges: Vec<ControlEdge>,
    pub faces: Vec<Vec<usize>>,
    pub face_layer: Vec<usize>,
    pub layers: Vec<Layer>,
    pub settings: ProjectSettings,
}

impl Surface {
    pub fn new() -> Self {
        Self {
            layers: vec![Layer::default()],
            ..Default::default()
        }
    }

    pub fn add_point(&mut self, p: Vec3) -> usize {
        self.points.push(ControlPoint { p, kind: VertexKind::Regular });
        self.points.len() - 1
    }

    pub fn add_face(&mut self, pts: Vec<usize>, layer: usize) -> usize {
        self.faces.push(pts);
        self.face_layer.push(layer);
        self.faces.len() - 1
    }

    /// Rebuild the edge list from the face list, preserving existing crease
    /// flags. Called after editing or loading.
    pub fn rebuild_edges(&mut self) {
        let mut crease: HashMap<(usize, usize), bool> = HashMap::new();
        for e in &self.edges {
            crease.insert(e.key(), e.crease);
        }
        let mut map: HashMap<(usize, usize), ControlEdge> = HashMap::new();
        for f in &self.faces {
            for i in 0..f.len() {
                let a = f[i];
                let b = f[(i + 1) % f.len()];
                let k = ordered(a, b);
                map.entry(k).or_insert(ControlEdge {
                    a: k.0,
                    b: k.1,
                    crease: *crease.get(&k).unwrap_or(&false),
                });
            }
        }
        let mut edges: Vec<ControlEdge> = map.into_values().collect();
        edges.sort_by_key(|e| e.key());
        self.edges = edges;
    }

    /// Number of faces attached to every edge, indexed like `self.edges`.
    pub fn edge_face_counts(&self) -> Vec<usize> {
        let mut index: HashMap<(usize, usize), usize> = HashMap::new();
        for (i, e) in self.edges.iter().enumerate() {
            index.insert(e.key(), i);
        }
        let mut counts = vec![0usize; self.edges.len()];
        for f in &self.faces {
            for i in 0..f.len() {
                let k = ordered(f[i], f[(i + 1) % f.len()]);
                if let Some(&ei) = index.get(&k) {
                    counts[ei] += 1;
                }
            }
        }
        counts
    }

    /// Boundary points (fewer than two incident faces) become corners, and
    /// edges with a single adjacent face become creases. This is what
    /// FREE!ship's `Initialize` does, and it makes open boundaries (deck
    /// openings, centreline of a half model) behave as hard edges.
    pub fn initialize_trace(&mut self) {
        self.rebuild_edges();
        let counts = self.edge_face_counts();
        for (e, &c) in self.edges.iter_mut().zip(counts.iter()) {
            if c != 2 {
                e.crease = true;
            }
        }
        let mut face_count = vec![0usize; self.points.len()];
        for f in &self.faces {
            for &p in f {
                face_count[p] += 1;
            }
        }
        for (p, &n) in self.points.iter_mut().zip(face_count.iter()) {
            if n < 2 {
                p.kind = VertexKind::Corner;
            }
        }
    }

    pub fn extents(&self) -> (Vec3, Vec3) {
        let mut min = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
        let mut max = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
        for p in &self.points {
            min = min.min(p.p);
            max = max.max(p.p);
        }
        if !min.is_finite() {
            (Vec3::ZERO, Vec3::ZERO)
        } else {
            (min, max)
        }
    }

    /// Lowest point of the hydrostatic layers, the datum FREE!ship uses to
    /// place the waterline when a draft is specified.
    pub fn lowest_hydrostatics_point(&self) -> f64 {
        let mut z = f64::INFINITY;
        for (fi, f) in self.faces.iter().enumerate() {
            if !self.layers[self.face_layer[fi]].use_in_hydrostatics {
                continue;
            }
            for &pi in f {
                z = z.min(self.points[pi].p.z);
            }
        }
        if z.is_finite() {
            z
        } else {
            self.extents().0.z
        }
    }

    /// Highest point of the hydrostatic layers.
    pub fn highest_hydrostatics_point(&self) -> f64 {
        let mut z = f64::NEG_INFINITY;
        for (fi, f) in self.faces.iter().enumerate() {
            if !self.layers[self.face_layer[fi]].use_in_hydrostatics {
                continue;
            }
            for &pi in f {
                z = z.max(self.points[pi].p.z);
            }
        }
        if z.is_finite() {
            z
        } else {
            self.extents().1.z
        }
    }

    /// Surface area of the control net (not subdivided).
    pub fn control_area(&self) -> f64 {
        self.faces
            .iter()
            .map(|f| polygon_area3(&f.iter().map(|&i| self.points[i].p).collect::<Vec<_>>()))
            .sum()
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.points.is_empty() {
            return Err("surface has no control points".into());
        }
        if self.faces.is_empty() {
            return Err("surface has no control faces".into());
        }
        for (fi, f) in self.faces.iter().enumerate() {
            if f.len() < 3 {
                return Err(format!("face {} has fewer than 3 points", fi));
            }
            for &p in f {
                if p >= self.points.len() {
                    return Err(format!("face {} references missing point {}", fi, p));
                }
            }
            if self.face_layer.get(fi).copied().unwrap_or(usize::MAX) >= self.layers.len() {
                return Err(format!("face {} references a missing layer", fi));
            }
            let mut seen = std::collections::HashSet::new();
            for &p in f {
                if !seen.insert(p) {
                    return Err(format!("face {} references point {} twice", fi, p));
                }
            }
        }
        Ok(())
    }
}
