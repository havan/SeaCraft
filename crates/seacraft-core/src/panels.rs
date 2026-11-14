//! Plywood panels: the hull's panels found from its layers and creases, laid flat.
//!
//! A panel is a piece of one layer bounded by creases, layer boundaries and open edges, found on
//! the **half** mesh from [`crate::subdiv::evaluate`] (the mirrored full mesh drops the creases on
//! its mirrored side). A symmetric panel that meets its mirror flat across the centreline (under
//! [`CENTRELINE_JOIN_DEG`]) is one piece spanning both sides; any other symmetric panel is cut
//! twice, once turned over.

use std::collections::{HashMap, HashSet};

use serde::Serialize;

use crate::flatten::{boundary_loops, clean, FlattenError};
use crate::math::Vec3;
use crate::mesh::{ordered, Layer, Surface};
use crate::pdf::Anchor;
use crate::sheet::{Label, PathKind, Sheet, SheetPath};
use crate::subdiv::{evaluate, SubdivisionOptions};
use crate::templates::{station_positions, StationChoice};

/// A symmetric panel is joined to its mirror when every face along the centreline is within
/// this angle of its mirror image.
pub const CENTRELINE_JOIN_DEG: f64 = 2.0;

/// How many copies of a panel are cut.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub enum Copies {
    One,
    /// Port and starboard: cut two, one turned over.
    MirroredPair,
}

impl Copies {
    pub fn count(self) -> usize {
        match self {
            Copies::One => 1,
            Copies::MirroredPair => 2,
        }
    }
}

/// What a panel's edge is stitched to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub enum Mate {
    Panel(String),
    /// The panel's own mirror image, across `y = 0`.
    Centreline,
    /// Nothing: the sheer or another free edge.
    Open,
}

/// A run of a panel's edge with one mate.
#[derive(Debug, Clone, PartialEq)]
pub struct Seam {
    pub mate: Mate,
    /// Ordered vertex ids in the half mesh, so a seam and its mate share ids.
    pub verts: Vec<usize>,
    /// On a panel joined to its mirror, the seam runs on through the mirror image of `verts`
    /// across the centreline, and is measured whole.
    pub across: bool,
}

impl Seam {
    /// `edge to Side`, `centreline` or `open edge`.
    pub fn name(&self) -> String {
        match &self.mate {
            Mate::Panel(p) => format!("edge to {p}"),
            Mate::Centreline => "centreline".into(),
            Mate::Open => "open edge".into(),
        }
    }
}

/// A panel found on the hull, still in 3D.
#[derive(Debug, Clone)]
pub struct FoundPanel {
    pub name: String,
    pub layer: usize,
    pub copies: Copies,
    /// Cleaned local mesh: a joined panel holds both sides.
    pub verts: Vec<Vec3>,
    pub tris: Vec<[usize; 3]>,
    /// Each local vertex's id in the half mesh; `usize::MAX` for a mirrored vertex.
    pub mesh_ids: Vec<usize>,
    /// Half-mesh ids welded onto a local vertex that `mesh_ids` gives another id (coincident
    /// points, as at a pole): every half-mesh id of the panel is in one or the other.
    pub welded: HashMap<usize, usize>,
    /// On a panel joined to its mirror, each local vertex's mirror image (a centreline point is
    /// its own); empty otherwise.
    pub mirror: HashMap<usize, usize>,
    /// On a panel joined to its mirror, the runs of half-mesh ids along the centreline it was
    /// joined across; empty otherwise.
    pub centreline: Vec<Vec<usize>>,
    pub seams: Vec<Seam>,
}

impl FoundPanel {
    /// The local vertex for a half-mesh id.
    pub fn local(&self, mesh_id: usize) -> Option<usize> {
        self.mesh_ids
            .iter()
            .position(|&m| m == mesh_id)
            .or_else(|| self.welded.get(&mesh_id).copied())
    }

    /// A seam's local vertices, with a point that welding made twice kept once, and the seam's
    /// mirror image joined on where it runs across the centreline; `None` when a vertex is not
    /// on the panel.
    pub fn seam_local(&self, seam: &Seam) -> Option<Vec<usize>> {
        let ids = self.half_local(&seam.verts)?;
        if !seam.across {
            return Some(ids);
        }
        let mut mirrored = self.mirror_local(&ids)?;
        mirrored.reverse();
        let mut full = if on_centreline(self.verts[ids[0]]) {
            mirrored.extend_from_slice(&ids);
            mirrored
        } else {
            let mut full = ids;
            full.extend_from_slice(&mirrored);
            full
        };
        full.dedup();
        Some(full)
    }

    /// The mirror image of a run of local vertices; `None` when the panel is not joined.
    pub fn mirror_local(&self, ids: &[usize]) -> Option<Vec<usize>> {
        ids.iter().map(|i| self.mirror.get(i).copied()).collect()
    }

    /// Local vertices for a run of half-mesh ids, with a point that welding made twice kept once.
    pub fn half_local(&self, mesh_ids: &[usize]) -> Option<Vec<usize>> {
        let index: HashMap<usize, usize> = self
            .mesh_ids
            .iter()
            .enumerate()
            .filter(|(_, &m)| m != usize::MAX)
            .map(|(i, &m)| (m, i))
            .chain(self.welded.iter().map(|(&m, &i)| (m, i)))
            .collect();
        let mut ids: Vec<usize> = mesh_ids
            .iter()
            .map(|m| index.get(m).copied())
            .collect::<Option<_>>()?;
        ids.dedup();
        Some(ids)
    }
}

/// A panel that could not be made, and why.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Refusal {
    pub panel: String,
    pub reason: String,
}

/// The layers panels are made from when the user names none: those marked developable, or every
/// layer used in hydrostatics when none is.
pub fn default_layers(surface: &Surface) -> Vec<usize> {
    let developable: Vec<usize> = (0..surface.layers.len())
        .filter(|&i| surface.layers[i].developable)
        .collect();
    if !developable.is_empty() {
        return developable;
    }
    (0..surface.layers.len())
        .filter(|&i| surface.layers[i].use_in_hydrostatics)
        .collect()
}

fn find(parent: &mut [usize], i: usize) -> usize {
    let mut r = i;
    while parent[r] != r {
        r = parent[r];
    }
    let mut j = i;
    while parent[j] != r {
        let next = parent[j];
        parent[j] = r;
        j = next;
    }
    r
}

/// Newell normal of a polygon (not normalised).
fn newell(pts: &[Vec3]) -> Vec3 {
    let mut n = Vec3::ZERO;
    for i in 0..pts.len() {
        let (a, b) = (pts[i], pts[(i + 1) % pts.len()]);
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

fn on_centreline(p: Vec3) -> bool {
    p.y.abs() < 1e-9
}

/// Finds the panels of `layers` on the refined half mesh. Every layer's pieces are named (so a
/// seam can name its mate in a layer not asked for); only the asked layers' panels are returned,
/// with the panels that could not be made as refusals.
pub fn find_panels(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    layers: &[usize],
) -> (Vec<FoundPanel>, Vec<Refusal>) {
    let mesh = evaluate(surface, subdivision);
    let nf = mesh.faces.len();
    let mut edge_faces: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        for i in 0..f.len() {
            edge_faces
                .entry(ordered(f[i], f[(i + 1) % f.len()]))
                .or_default()
                .push(fi);
        }
    }

    // Pieces: faces of one layer joined across edges that are not creased.
    let mut parent: Vec<usize> = (0..nf).collect();
    for (e, fs) in &edge_faces {
        if mesh.creases.contains(e) {
            continue;
        }
        for i in 0..fs.len() {
            for j in i + 1..fs.len() {
                if mesh.face_layer[fs[i]] == mesh.face_layer[fs[j]] {
                    let (a, b) = (find(&mut parent, fs[i]), find(&mut parent, fs[j]));
                    if a != b {
                        parent[a.max(b)] = a.min(b);
                    }
                }
            }
        }
    }
    let mut comp_of_root: HashMap<usize, usize> = HashMap::new();
    let mut comps: Vec<Vec<usize>> = Vec::new();
    let mut face_comp = vec![0usize; nf];
    for (fi, slot) in face_comp.iter_mut().enumerate() {
        let r = find(&mut parent, fi);
        let c = *comp_of_root.entry(r).or_insert_with(|| {
            comps.push(Vec::new());
            comps.len() - 1
        });
        comps[c].push(fi);
        *slot = c;
    }
    let comp_layer: Vec<usize> = comps.iter().map(|c| mesh.face_layer[c[0]]).collect();

    // Names: the layer's name, numbered by centroid height then position along when the layer
    // has several pieces.
    let centroid = |faces: &[usize]| -> Vec3 {
        let (mut sum, mut area) = (Vec3::ZERO, 0.0);
        for &fi in faces {
            let pts: Vec<Vec3> = mesh.faces[fi].iter().map(|&v| mesh.verts[v]).collect();
            let a = 0.5 * newell(&pts).length();
            let c = pts.iter().fold(Vec3::ZERO, |s, p| s + *p) / pts.len() as f64;
            sum += c * a;
            area += a;
        }
        if area > 0.0 {
            sum / area
        } else {
            sum
        }
    };
    let mut names = vec![String::new(); comps.len()];
    for layer in 0..surface.layers.len() {
        let mut ids: Vec<(i64, i64, usize)> = (0..comps.len())
            .filter(|&c| comp_layer[c] == layer)
            .map(|c| {
                let g = centroid(&comps[c]);
                (
                    (g.z * 1000.0).round() as i64,
                    (g.x * 1000.0).round() as i64,
                    c,
                )
            })
            .collect();
        ids.sort();
        let base = &surface.layers[layer].name;
        for (k, &(_, _, c)) in ids.iter().enumerate() {
            names[c] = if ids.len() == 1 {
                base.clone()
            } else {
                format!("{base} {}", k + 1)
            };
        }
    }

    let mut panels = Vec::new();
    let mut refused = Vec::new();
    let mut order: Vec<usize> = (0..comps.len())
        .filter(|&c| layers.contains(&comp_layer[c]))
        .collect();
    order.sort_by(|&a, &b| (comp_layer[a], &names[a]).cmp(&(comp_layer[b], &names[b])));
    for c in order {
        let faces = &comps[c];
        let name = names[c].clone();
        let layer = comp_layer[c];
        let symmetric = surface.layers[layer].symmetric;
        let in_comp = |fi: usize| face_comp[fi] == c;

        // Edges of the piece, and which are on its boundary.
        let mut tris_half: Vec<[usize; 3]> = Vec::new();
        for &fi in faces {
            let f = &mesh.faces[fi];
            for i in 1..f.len().saturating_sub(1) {
                tris_half.push([f[0], f[i], f[i + 1]]);
            }
        }
        // Faces wound both ways put one directed edge in two faces; the boundary cannot then be
        // followed, nor the panel's outside told from its inside.
        let mut directed = HashSet::new();
        let one_way = tris_half
            .iter()
            .flat_map(|t| [(t[0], t[1]), (t[1], t[2]), (t[2], t[0])])
            .all(|e| directed.insert(e));
        let loops = boundary_loops(&tris_half);
        let loops_on_edges = loops.iter().all(|lp| {
            (0..lp.len()).all(|i| edge_faces.contains_key(&ordered(lp[i], lp[(i + 1) % lp.len()])))
        });
        if !one_way || !loops_on_edges {
            refused.push(Refusal {
                panel: name,
                reason: "its faces do not all face the same way: turn the faces of this layer so \
                         they all face out"
                    .into(),
            });
            continue;
        }
        let on_boundary: HashSet<usize> = loops.iter().flatten().copied().collect();

        // A crease that ends inside the piece leaves faces on both its sides in the piece.
        let mut inner_creases: Vec<(usize, usize)> = Vec::new();
        for &fi in faces {
            let f = &mesh.faces[fi];
            for i in 0..f.len() {
                let e = ordered(f[i], f[(i + 1) % f.len()]);
                if mesh.creases.contains(&e)
                    && edge_faces[&e].iter().filter(|&&g| in_comp(g)).count() >= 2
                {
                    inner_creases.push(e);
                }
            }
        }
        if !inner_creases.is_empty() {
            inner_creases.sort_unstable();
            inner_creases.dedup();
            let mut count: HashMap<usize, usize> = HashMap::new();
            for &(a, b) in &inner_creases {
                *count.entry(a).or_default() += 1;
                *count.entry(b).or_default() += 1;
            }
            let end = inner_creases
                .iter()
                .flat_map(|&(a, b)| [a, b])
                .find(|v| count[v] == 1 && !on_boundary.contains(v))
                .unwrap_or(inner_creases[0].0);
            let p = mesh.verts[end];
            refused.push(Refusal {
                panel: name,
                reason: format!(
                    "a crease ends inside the panel at x {:.2} m, z {:.2} m: carry the crease to \
                     the panel's edge or split the layer there",
                    p.x, p.z
                ),
            });
            continue;
        }

        // Joined across the centreline when it meets its mirror flat there.
        let mut centreline_faces = Vec::new();
        for lp in &loops {
            for i in 0..lp.len() {
                let (a, b) = (lp[i], lp[(i + 1) % lp.len()]);
                let e = ordered(a, b);
                if on_centreline(mesh.verts[a])
                    && on_centreline(mesh.verts[b])
                    && edge_faces[&e].len() == 1
                {
                    centreline_faces.push(edge_faces[&e][0]);
                }
            }
        }
        let limit = CENTRELINE_JOIN_DEG.to_radians();
        let joined = symmetric
            && !centreline_faces.is_empty()
            && centreline_faces.iter().all(|&fi| {
                let pts: Vec<Vec3> = mesh.faces[fi].iter().map(|&v| mesh.verts[v]).collect();
                let n = newell(&pts).norm();
                let m = Vec3::new(n.x, -n.y, n.z);
                n.dot(m).clamp(-1.0, 1.0).acos() < limit
            });

        // Seams: the boundary split where the mate changes.
        let mut seams = Vec::new();
        for lp in &loops {
            let k = lp.len();
            let mates: Vec<Mate> = (0..k)
                .map(|i| {
                    let (a, b) = (lp[i], lp[(i + 1) % k]);
                    let mut others: Vec<usize> = edge_faces[&ordered(a, b)]
                        .iter()
                        .map(|&g| face_comp[g])
                        .filter(|&g| g != c)
                        .collect();
                    others.sort_unstable();
                    if let Some(&o) = others.first() {
                        Mate::Panel(names[o].clone())
                    } else if symmetric
                        && on_centreline(mesh.verts[a])
                        && on_centreline(mesh.verts[b])
                    {
                        Mate::Centreline
                    } else {
                        Mate::Open
                    }
                })
                .collect();
            let start = (0..k).find(|&i| mates[i] != mates[(i + k - 1) % k]);
            match start {
                None => {
                    let mut verts = lp.clone();
                    verts.push(lp[0]);
                    seams.push(Seam {
                        mate: mates[0].clone(),
                        verts,
                        across: false,
                    });
                }
                Some(s0) => {
                    let mut i = s0;
                    loop {
                        let mate = mates[i].clone();
                        let mut verts = vec![lp[i]];
                        let mut j = i;
                        loop {
                            verts.push(lp[(j + 1) % k]);
                            j = (j + 1) % k;
                            if j == s0 || mates[j] != mate {
                                break;
                            }
                        }
                        seams.push(Seam {
                            mate,
                            verts,
                            across: false,
                        });
                        i = j;
                        if i == s0 {
                            break;
                        }
                    }
                }
            }
        }
        let mut centreline = Vec::new();
        if joined {
            centreline = seams
                .iter()
                .filter(|s| s.mate == Mate::Centreline)
                .map(|s| s.verts.clone())
                .collect();
            seams.retain(|s| s.mate != Mate::Centreline);
            for s in &mut seams {
                let ends = [s.verts[0], s.verts[s.verts.len() - 1]];
                s.across = ends.iter().any(|&v| on_centreline(mesh.verts[v]));
            }
        }
        let rank = |m: &Mate| match m {
            Mate::Panel(_) => 0,
            Mate::Centreline => 1,
            Mate::Open => 2,
        };
        seams.sort_by(|a, b| {
            let key = |s: &Seam| {
                let label = match &s.mate {
                    Mate::Panel(p) => p.clone(),
                    _ => String::new(),
                };
                (rank(&s.mate), label)
            };
            key(a).cmp(&key(b)).then_with(|| {
                let (pa, pb) = (mesh.verts[a.verts[0]], mesh.verts[b.verts[0]]);
                pa.x.total_cmp(&pb.x).then(pa.z.total_cmp(&pb.z))
            })
        });

        // The piece as a local mesh, with its mirror when joined.
        let mut local: HashMap<usize, usize> = HashMap::new();
        let mut verts = Vec::new();
        let mut ids = Vec::new();
        let mut tris = Vec::new();
        for t in &tris_half {
            let mut lt = [0usize; 3];
            for (slot, &v) in lt.iter_mut().zip(t) {
                *slot = *local.entry(v).or_insert_with(|| {
                    verts.push(mesh.verts[v]);
                    ids.push(v);
                    verts.len() - 1
                });
            }
            tris.push(lt);
        }
        let mut mirror: HashMap<usize, usize> = HashMap::new();
        if joined {
            let half = tris.len();
            for ti in 0..half {
                let t = tris[ti];
                let mut mt = [0usize; 3];
                for (slot, &v) in mt.iter_mut().zip(t.iter().rev()) {
                    *slot = if on_centreline(verts[v]) {
                        v
                    } else {
                        *mirror.entry(v).or_insert_with(|| {
                            let p = verts[v];
                            verts.push(Vec3::new(p.x, -p.y, p.z));
                            ids.push(usize::MAX);
                            verts.len() - 1
                        })
                    };
                }
                tris.push(mt);
            }
        }
        let (cv, ct, map) = clean(&verts, &tris);
        let mut mesh_ids = vec![usize::MAX; cv.len()];
        let mut welded = HashMap::new();
        for (old, &new) in map.iter().enumerate() {
            if new == usize::MAX || ids[old] == usize::MAX {
                continue;
            }
            if mesh_ids[new] == usize::MAX {
                mesh_ids[new] = ids[old];
            } else {
                welded.insert(ids[old], new);
            }
        }
        let mut mirror_local = HashMap::new();
        if joined {
            for (old, &id) in ids.iter().enumerate() {
                if id == usize::MAX {
                    continue;
                }
                let image = if on_centreline(verts[old]) {
                    Some(old)
                } else {
                    mirror.get(&old).copied()
                };
                if let Some(image) = image {
                    let (a, b) = (map[old], map[image]);
                    if a != usize::MAX && b != usize::MAX {
                        mirror_local.insert(a, b);
                        mirror_local.insert(b, a);
                    }
                }
            }
        }
        let loops_after = boundary_loops(&ct).len();
        if ct.is_empty() || loops_after != 1 {
            let err = if ct.is_empty() {
                FlattenError::Empty
            } else {
                FlattenError::NotADisc { loops: loops_after }
            };
            refused.push(Refusal {
                panel: name,
                reason: err.to_string(),
            });
            continue;
        }
        let copies = if joined || !symmetric {
            Copies::One
        } else {
            Copies::MirroredPair
        };
        panels.push(FoundPanel {
            name,
            layer,
            copies,
            verts: cv,
            tris: ct,
            mesh_ids,
            welded,
            mirror: mirror_local,
            centreline,
            seams,
        });
    }
    (panels, refused)
}

/// When a panel is said not to lie flat. Provisional project judgement, not a published rule;
/// `docs/oracles.md` says so.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Limits {
    /// Largest seam drift, metres.
    pub seam_drift_m: f64,
    /// Largest RMS stretch, a fraction.
    pub rms_stretch: f64,
}

impl Default for Limits {
    fn default() -> Self {
        Self {
            seam_drift_m: 0.002,
            rms_stretch: 0.001,
        }
    }
}

/// How one seam's flat length follows its 3D length.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeamMeasure {
    pub name: String,
    /// Largest |flat − 3D| cumulative length at the station ticks and the far end, measured from
    /// either end.
    pub drift_m: f64,
    pub length_3d_m: f64,
    pub length_flat_m: f64,
}

/// How far two seams stitched together differ in flat length at the station ticks.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Mismatch {
    pub a: String,
    pub b: String,
    /// The seam's name on panel `a`.
    pub seam: String,
    pub max_m: f64,
}

/// What was measured on one flat panel.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Measures {
    pub seams: Vec<SeamMeasure>,
    pub rms_stretch: f64,
    pub max_stretch: f64,
    /// Largest distance between this subdivision level's outline and the next level's (the one
    /// before at the cap), placed by the best rigid fit of their corners; `None` when they could
    /// not be paired.
    pub level_shift_m: Option<f64>,
    /// The edge-joining development's gap ([`crate::develop::unfold`]), a second, independent
    /// measure.
    pub edge_join_gap_m: f64,
    pub area_3d_m2: f64,
    pub area_flat_m2: f64,
    /// `None` when the flattening settled; otherwise how far its points may still have had to
    /// move ([`crate::flatten::Flat::unsettled_m`]).
    pub unsettled_m: Option<f64>,
}

impl Measures {
    /// The seam with the largest drift.
    pub fn worst_seam(&self) -> Option<&SeamMeasure> {
        self.seams
            .iter()
            .max_by(|a, b| a.drift_m.total_cmp(&b.drift_m))
    }

    pub fn max_seam_drift_m(&self) -> f64 {
        self.worst_seam().map_or(0.0, |s| s.drift_m)
    }
}

/// Whether a panel lies flat within `limits`: its seams and stretch inside them, and its
/// flattening settled (an unsettled pattern's numbers are not final).
pub fn lies_flat(measures: &Measures, limits: Limits) -> bool {
    measures.max_seam_drift_m() <= limits.seam_drift_m
        && measures.rms_stretch <= limits.rms_stretch
        && measures.unsettled_m.is_none()
}

/// A panel laid flat and measured.
#[derive(Debug, Clone)]
pub struct FlatPanel {
    pub found: FoundPanel,
    /// One point per `found.verts`, metres, oriented by [`orient`].
    pub xy: Vec<(f64, f64)>,
    pub measures: Measures,
    pub lies_flat: bool,
}

/// The largest difference between two cumulative lengths along one seam, at its ticks, measured
/// from the first tick and again from the last. `s3d` and `sflat` start at 0 and end at the
/// seam's end.
pub(crate) fn drift_from_both_ends(s3d: &[f64], sflat: &[f64]) -> f64 {
    let (Some(&l3), Some(&lf)) = (s3d.last(), sflat.last()) else {
        return 0.0;
    };
    s3d.iter()
        .zip(sflat)
        .map(|(&a, &b)| (b - a).abs().max(((lf - b) - (l3 - a)).abs()))
        .fold(0.0, f64::max)
}

/// Cumulative 3D and flat lengths along `ids` (local vertices) at each station crossing, with 0
/// first and the seam's end last.
fn ticks(
    verts: &[Vec3],
    xy: &[(f64, f64)],
    ids: &[usize],
    stations: &[f64],
) -> (Vec<f64>, Vec<f64>) {
    let (mut t3, mut tf) = (vec![0.0], vec![0.0]);
    let (mut s3, mut sf) = (0.0, 0.0);
    for w in ids.windows(2) {
        let (a, b) = (w[0], w[1]);
        let l3 = verts[a].dist(verts[b]);
        let lf = ((xy[b].0 - xy[a].0).powi(2) + (xy[b].1 - xy[a].1).powi(2)).sqrt();
        let (xa, xb) = (verts[a].x, verts[b].x);
        if xa != xb {
            let mut hits: Vec<f64> = stations
                .iter()
                .map(|&x| (x - xa) / (xb - xa))
                .filter(|&t| t > 0.0 && t <= 1.0)
                .collect();
            hits.sort_by(f64::total_cmp);
            for t in hits {
                t3.push(s3 + t * l3);
                tf.push(sf + t * lf);
            }
        }
        s3 += l3;
        sf += lf;
    }
    if t3.last() != Some(&s3) {
        t3.push(s3);
        tf.push(sf);
    }
    (t3, tf)
}

/// Turns a flat pattern so the builder reads it: outside face up (it already is: [`flatten`]
/// keeps the triangles' winding), long axis along x, forward to the right (flat x grows with
/// model x; for a panel across the hull, such as a transom, up is up), lowest corner at the
/// origin.
///
/// [`flatten`]: crate::flatten::flatten
pub fn orient(found: &FoundPanel, xy: &mut [(f64, f64)]) {
    let n = xy.len() as f64;
    if xy.is_empty() {
        return;
    }
    let (mx, my) = xy
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
    let (mut cxx, mut cxy, mut cyy) = (0.0, 0.0, 0.0);
    for p in xy.iter() {
        let (dx, dy) = (p.0 - mx, p.1 - my);
        cxx += dx * dx;
        cxy += dx * dy;
        cyy += dy * dy;
    }
    let th = 0.5 * (2.0 * cxy).atan2(cxx - cyy);
    let (c, s) = (th.cos(), th.sin());
    for p in xy.iter_mut() {
        let (dx, dy) = (p.0 - mx, p.1 - my);
        *p = (c * dx + s * dy, -s * dx + c * dy);
    }
    let m3 = found.verts.iter().fold(Vec3::ZERO, |s, v| s + *v) / n;
    let along: f64 = xy
        .iter()
        .zip(&found.verts)
        .map(|(p, v)| p.0 * (v.x - m3.x))
        .sum();
    let span = found
        .verts
        .iter()
        .map(|v| (v.x - m3.x).abs())
        .fold(0.0, f64::max);
    let turn = if span > 1e-6 {
        along < 0.0
    } else {
        let up: f64 = xy
            .iter()
            .zip(&found.verts)
            .map(|(p, v)| p.1 * (v.z - m3.z))
            .sum();
        up < 0.0
    };
    if turn {
        for p in xy.iter_mut() {
            *p = (-p.0, -p.1);
        }
    }
    let (x0, y0) = xy
        .iter()
        .fold((f64::MAX, f64::MAX), |m, p| (m.0.min(p.0), m.1.min(p.1)));
    for p in xy.iter_mut() {
        *p = (p.0 - x0, p.1 - y0);
    }
}

fn flat_pattern(found: &FoundPanel) -> Result<crate::flatten::Flat, FlattenError> {
    let mut flat = crate::flatten::flatten(&found.verts, &found.tris)?;
    orient(found, &mut flat.xy);
    Ok(flat)
}

/// Flattens and measures one panel, without the level shift (which needs the surface).
pub(crate) fn measure_found(
    found: FoundPanel,
    stations: &[f64],
    limits: Limits,
) -> Result<FlatPanel, Refusal> {
    let flat = flat_pattern(&found).map_err(|e| Refusal {
        panel: found.name.clone(),
        reason: e.to_string(),
    })?;
    let xy = flat.xy;
    let st = crate::flatten::strain(&found.verts, &found.tris, &xy);
    let mut seams = Vec::new();
    for seam in &found.seams {
        let Some(ids) = found.seam_local(seam) else {
            return Err(Refusal {
                panel: found.name.clone(),
                reason: format!(
                    "its {} could not be followed on the panel's mesh, so it was not measured",
                    seam.name()
                ),
            });
        };
        let (t3, tf) = ticks(&found.verts, &xy, &ids, stations);
        seams.push(SeamMeasure {
            name: seam.name(),
            drift_m: drift_from_both_ends(&t3, &tf),
            length_3d_m: t3.last().copied().unwrap_or(0.0),
            length_flat_m: tf.last().copied().unwrap_or(0.0),
        });
    }
    let area_3d_m2 = found
        .tris
        .iter()
        .map(|t| {
            let v = &found.verts;
            0.5 * (v[t[1]] - v[t[0]]).cross(v[t[2]] - v[t[0]]).length()
        })
        .sum();
    let area_flat_m2 = found
        .tris
        .iter()
        .map(|t| crate::flatten::signed_area(xy[t[0]], xy[t[1]], xy[t[2]]))
        .sum();
    let measures = Measures {
        seams,
        rms_stretch: st.rms,
        max_stretch: st.max,
        level_shift_m: None,
        edge_join_gap_m: crate::develop::unfold(&found.verts, &found.tris).max_gap_m,
        area_3d_m2,
        area_flat_m2,
        unsettled_m: flat.unsettled_m,
    };
    Ok(FlatPanel {
        lies_flat: lies_flat(&measures, limits),
        found,
        xy,
        measures,
    })
}

/// The 3D and flat position of each seam end of a panel.
fn corners(found: &FoundPanel, xy: &[(f64, f64)]) -> Vec<(Vec3, (f64, f64))> {
    let mut out: Vec<(Vec3, (f64, f64))> = Vec::new();
    for seam in &found.seams {
        let Some(ids) = found.seam_local(seam) else {
            continue;
        };
        for &i in [ids.first(), ids.last()].into_iter().flatten() {
            if !out.iter().any(|(p, _)| p.dist(found.verts[i]) < 1e-9) {
                out.push((found.verts[i], xy[i]));
            }
        }
    }
    out
}

/// A turn and a move in the plane: `q` goes to `rot · (q − from) + to`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct RigidFit {
    cos: f64,
    sin: f64,
    from: (f64, f64),
    to: (f64, f64),
}

impl RigidFit {
    pub(crate) fn apply(&self, q: (f64, f64)) -> (f64, f64) {
        let (qx, qy) = (q.0 - self.from.0, q.1 - self.from.1);
        (
            self.cos * qx - self.sin * qy + self.to.0,
            self.sin * qx + self.cos * qy + self.to.1,
        )
    }
}

/// The best rigid fit (no reflection) of the points `b` onto the matched points `a`.
pub(crate) fn rigid_fit(a: &[(f64, f64)], b: &[(f64, f64)]) -> RigidFit {
    let n = a.len().max(1) as f64;
    let ca = a
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
    let cb = b
        .iter()
        .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
    let (mut sxx, mut sxy) = (0.0, 0.0);
    for (p, q) in a.iter().zip(b) {
        let (px, py) = (p.0 - ca.0, p.1 - ca.1);
        let (qx, qy) = (q.0 - cb.0, q.1 - cb.1);
        sxx += qx * px + qy * py;
        sxy += qx * py - qy * px;
    }
    let th = sxy.atan2(sxx);
    RigidFit {
        cos: th.cos(),
        sin: th.sin(),
        from: cb,
        to: ca,
    }
}

/// The largest distance between matched points after the best rigid fit (no reflection) of `b`
/// onto `a`.
#[cfg(test)]
pub(crate) fn rigid_misfit(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    if a.is_empty() {
        return 0.0;
    }
    let fit = rigid_fit(a, b);
    a.iter()
        .zip(b)
        .map(|(p, &q)| {
            let r = fit.apply(q);
            (r.0 - p.0).hypot(r.1 - p.1)
        })
        .fold(0.0, f64::max)
}

/// The largest distance between two closed outlines, either way round, after the best rigid
/// fit of `corners_b` onto `corners_a` has placed `b`.
pub(crate) fn outline_shift(
    a: &[(f64, f64)],
    b: &[(f64, f64)],
    corners_a: &[(f64, f64)],
    corners_b: &[(f64, f64)],
) -> f64 {
    let fit = rigid_fit(corners_a, corners_b);
    let b: Vec<(f64, f64)> = b.iter().map(|&q| fit.apply(q)).collect();
    // The largest distance from a point of `from` to the closed polyline `to`.
    let directed = |from: &[(f64, f64)], to: &[(f64, f64)]| -> f64 {
        from.iter()
            .map(|&p| {
                (0..to.len())
                    .map(|k| segment_distance(p, to[k], to[(k + 1) % to.len()]))
                    .fold(f64::INFINITY, f64::min)
            })
            .fold(0.0, f64::max)
    };
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    directed(a, &b).max(directed(&b, a))
}

fn segment_distance(p: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let l2 = dx * dx + dy * dy;
    let t = if l2 > 0.0 {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / l2).clamp(0.0, 1.0)
    } else {
        0.0
    };
    (p.0 - a.0 - t * dx).hypot(p.1 - a.1 - t * dy)
}

/// Finds, flattens and measures the panels of `layers`. `stations` are the model `x` of the
/// station ticks. Returns the panels, every pair of stitched seams compared, and the panels that
/// could not be made.
pub fn measure_panels(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    layers: &[usize],
    stations: &[f64],
    limits: Limits,
) -> (Vec<FlatPanel>, Vec<Mismatch>, Vec<Refusal>) {
    let (found, mut refused) = find_panels(surface, subdivision, layers);
    let mut panels = Vec::new();
    for f in found {
        match measure_found(f, stations, limits) {
            Ok(p) => panels.push(p),
            Err(r) => refused.push(r),
        }
    }

    // Level shift: the same panels one level finer (one coarser at the cap).
    let level = subdivision.level.min(6);
    let other = SubdivisionOptions {
        level: if level >= 6 { level - 1 } else { level + 1 },
        ..subdivision
    };
    let (other_found, _) = find_panels(surface, other, layers);
    for p in &mut panels {
        let Some(q) = other_found.iter().find(|q| q.name == p.found.name) else {
            continue;
        };
        let Ok(qxy) = flat_pattern(q).map(|f| f.xy) else {
            continue;
        };
        let here = corners(&p.found, &p.xy);
        let there = corners(q, &qxy);
        let (mut a, mut b) = (Vec::new(), Vec::new());
        for (p3, pf) in &here {
            if let Some((_, qf)) = there.iter().find(|(q3, _)| q3.dist(*p3) < 1e-3) {
                a.push(*pf);
                b.push(*qf);
            }
        }
        if a.len() >= 2 {
            let outline = |found: &FoundPanel, xy: &[(f64, f64)]| -> Vec<(f64, f64)> {
                boundary_loops(&found.tris)
                    .into_iter()
                    .next()
                    .unwrap_or_default()
                    .into_iter()
                    .map(|i| xy[i])
                    .collect()
            };
            p.measures.level_shift_m = Some(outline_shift(
                &outline(&p.found, &p.xy),
                &outline(q, &qxy),
                &a,
                &b,
            ));
        }
    }

    // Seams stitched together: the same half-mesh vertices, from two panels.
    let mut mismatches = Vec::new();
    for i in 0..panels.len() {
        for j in i + 1..panels.len() {
            let (pa, pb) = (&panels[i], &panels[j]);
            for sa in &pa.found.seams {
                if sa.mate != Mate::Panel(pb.found.name.clone()) {
                    continue;
                }
                let mut rev = sa.verts.clone();
                rev.reverse();
                let Some(sb) = pb
                    .found
                    .seams
                    .iter()
                    .find(|sb| sb.verts == sa.verts || sb.verts == rev)
                else {
                    continue;
                };
                // Whole across the centreline when both panels are joined; the halves otherwise.
                let seam = Seam {
                    across: sa.across && sb.across,
                    ..sa.clone()
                };
                let (Some(ia), Some(ib)) = (pa.found.seam_local(&seam), pb.found.seam_local(&seam))
                else {
                    continue;
                };
                let (_, fa) = ticks(&pa.found.verts, &pa.xy, &ia, stations);
                let (_, fb) = ticks(&pb.found.verts, &pb.xy, &ib, stations);
                mismatches.push(Mismatch {
                    a: pa.found.name.clone(),
                    b: pb.found.name.clone(),
                    seam: sa.name(),
                    max_m: drift_from_both_ends(&fa, &fb),
                });
            }
        }
    }
    (panels, mismatches, refused)
}

/// Letter heights on paper, in millimetres, as the mould patterns use them.
const MARK_LABEL_MM: f64 = 3.0;
const TITLE_LABEL_MM: f64 = 5.0;
/// How long a seam tick is, metres.
const TICK_M: f64 = 0.02;
/// How far a seam's label sits inside the edge, metres.
const SEAM_LABEL_INSET_M: f64 = 0.015;

/// Wording every panel sheet carries.
pub const THICKNESS_NOTE: &str = "Developed on the modelled surface: no plywood thickness allowed";
pub const MEASURED_NOTE: &str = "Measured against the model; not a bending test of plywood.";

/// A point where a station plane crosses the panel: its key and its flat position.
type PlanePoint = ((usize, usize), (f64, f64));

/// Where a station plane `x = X` crosses an edge: the key that names the point (a vertex when
/// the plane passes through it) and the edge parameter.
fn plane_point(verts: &[Vec3], a: usize, b: usize, x: f64) -> ((usize, usize), f64) {
    let t = ((x - verts[a].x) / (verts[b].x - verts[a].x)).clamp(0.0, 1.0);
    if t <= 1e-12 {
        ((a, a), 0.0)
    } else if t >= 1.0 - 1e-12 {
        ((b, b), 1.0)
    } else {
        (ordered(a, b), t)
    }
}

/// Each station's line on the flat panel: the station plane cut through the 3D panel, every
/// crossing point carried to the pattern by where it lies on its triangle's edge, chained into
/// polylines. A station that crosses the panel in several places gives several lines.
pub fn station_marks(
    panel: &FlatPanel,
    stations: &[(String, f64)],
) -> Vec<(String, Vec<(f64, f64)>)> {
    let verts = &panel.found.verts;
    let xy = &panel.xy;
    let mut out = Vec::new();
    for (name, x) in stations {
        let side = |v: usize| verts[v].x >= *x;
        let mut segs: Vec<[PlanePoint; 2]> = Vec::new();
        for t in &panel.found.tris {
            let mut ends = Vec::new();
            for k in 0..3 {
                let (a, b) = (t[k], t[(k + 1) % 3]);
                if side(a) != side(b) {
                    let (key, u) = plane_point(verts, a, b, *x);
                    let p = (
                        xy[a].0 + u * (xy[b].0 - xy[a].0),
                        xy[a].1 + u * (xy[b].1 - xy[a].1),
                    );
                    ends.push((key, p));
                }
            }
            if ends.len() == 2 && ends[0].0 != ends[1].0 {
                segs.push([ends[0], ends[1]]);
            }
        }
        // Chain the segments end to end.
        let mut at: HashMap<(usize, usize), Vec<usize>> = HashMap::new();
        for (i, sg) in segs.iter().enumerate() {
            for e in sg {
                at.entry(e.0).or_default().push(i);
            }
        }
        let mut used = vec![false; segs.len()];
        let mut lines: Vec<Vec<(f64, f64)>> = Vec::new();
        for i in 0..segs.len() {
            if used[i] {
                continue;
            }
            used[i] = true;
            let mut keys = std::collections::VecDeque::from([segs[i][0].0, segs[i][1].0]);
            let mut pts = std::collections::VecDeque::from([segs[i][0].1, segs[i][1].1]);
            for front in [false, true] {
                loop {
                    let end = if front { keys[0] } else { keys[keys.len() - 1] };
                    let Some(&j) = at[&end].iter().find(|&&j| !used[j]) else {
                        break;
                    };
                    used[j] = true;
                    let other = if segs[j][0].0 == end {
                        segs[j][1]
                    } else {
                        segs[j][0]
                    };
                    if front {
                        keys.push_front(other.0);
                        pts.push_front(other.1);
                    } else {
                        keys.push_back(other.0);
                        pts.push_back(other.1);
                    }
                }
            }
            lines.push(pts.into_iter().collect());
        }
        // Longest first, so a station's main line comes before any stub.
        lines.sort_by(|a, b| polyline_length(b).total_cmp(&polyline_length(a)));
        for line in lines {
            out.push((name.clone(), line));
        }
    }
    out
}

fn polyline_length(pts: &[(f64, f64)]) -> f64 {
    pts.windows(2)
        .map(|w| ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt())
        .sum()
}

/// The point at the middle of a flat polyline's length, and its direction there.
fn midpoint(pts: &[(f64, f64)]) -> Option<((f64, f64), (f64, f64))> {
    let half = polyline_length(pts) / 2.0;
    let mut run = 0.0;
    for w in pts.windows(2) {
        let (dx, dy) = (w[1].0 - w[0].0, w[1].1 - w[0].1);
        let l = (dx * dx + dy * dy).sqrt();
        if l > 0.0 && run + l >= half {
            let t = (half - run) / l;
            return Some(((w[0].0 + t * dx, w[0].1 + t * dy), (dx / l, dy / l)));
        }
        run += l;
    }
    None
}

/// An angle in degrees that keeps text along `d` the right way up.
fn reading_angle(d: (f64, f64)) -> f64 {
    let mut a = d.1.atan2(d.0).to_degrees();
    if a > 90.0 {
        a -= 180.0;
    } else if a < -90.0 {
        a += 180.0;
    }
    a
}

/// The first note of a panel that does not lie flat.
pub fn failing_heading(measures: &Measures, limits: Limits) -> String {
    let (drift, seam) = measures
        .worst_seam()
        .map_or((0.0, String::from("edge")), |s| (s.drift_m, s.name.clone()));
    let mut heading = format!(
        "Does not lie flat: must stretch {:.2} % (limit {:.1} %), seam drift {:.1} mm along the {} (limit {:.0} mm)",
        measures.rms_stretch * 100.0,
        limits.rms_stretch * 100.0,
        drift * 1000.0,
        seam,
        limits.seam_drift_m * 1000.0
    );
    if measures.unsettled_m.is_some() {
        heading.push_str("; the flattening stopped at its limit before it settled");
    }
    heading
}

/// How far each of a panel's stitched edges differs from its mate's at the ticks, as a sheet
/// note; `None` for a panel stitched to nothing. A mate measured apart from this panel (not in
/// the set, or its edge not paired) is said to be not compared.
pub fn mismatch_note(panel: &FlatPanel, mismatches: &[Mismatch]) -> Option<String> {
    let me = &panel.found.name;
    let mut mates: Vec<&str> = panel
        .found
        .seams
        .iter()
        .filter_map(|s| match &s.mate {
            Mate::Panel(m) => Some(m.as_str()),
            _ => None,
        })
        .collect();
    mates.sort_unstable();
    mates.dedup();
    if mates.is_empty() {
        return None;
    }
    let parts: Vec<String> = mates
        .iter()
        .map(|&mate| {
            let worst = mismatches
                .iter()
                .filter(|m| (&m.a == me && m.b == mate) || (m.a == mate && &m.b == me))
                .map(|m| m.max_m)
                .fold(None, |w: Option<f64>, d| Some(w.map_or(d, |w| w.max(d))));
            match worst {
                Some(d) => format!("{mate} {:.1} mm", d * 1000.0),
                None => format!("{mate} not compared"),
            }
        })
        .collect();
    Some(format!(
        "Against the mating edge, largest difference at the ticks: {}",
        parts.join("; ")
    ))
}

/// What an unsettled flattening may still be off by, as a sheet note.
pub fn unsettled_note(unsettled_m: f64) -> String {
    if unsettled_m.is_finite() {
        format!(
            "Flattening stopped at its iteration limit: points may still move about {:.2} mm",
            unsettled_m * 1000.0
        )
    } else {
        "Flattening stopped at its iteration limit: how far points may still move could not be \
         estimated"
            .into()
    }
}

/// The full-size pattern of one panel: its outline to cut, the station lines and the seam ticks
/// to mark, what each edge is stitched to, and what was measured.
pub fn panel_sheet(
    panel: &FlatPanel,
    stations: &[(String, f64)],
    limits: Limits,
    imperial: bool,
) -> Sheet {
    let found = &panel.found;
    let xy = &panel.xy;
    let mut sheet = Sheet::new(found.name.clone());
    let mark_label = |at: (f64, f64), angle_deg: f64, anchor: Anchor, text: String| Label {
        at,
        height_mm: MARK_LABEL_MM,
        angle_deg,
        anchor,
        text,
    };

    let loops = boundary_loops(&found.tris);
    for lp in &loops {
        sheet.paths.push(SheetPath {
            kind: PathKind::Cut,
            points: lp.iter().map(|&i| xy[i]).collect(),
            closed: true,
        });
    }

    for (name, line) in station_marks(panel, stations) {
        let top = if line[0].1 >= line[line.len() - 1].1 {
            line[0]
        } else {
            line[line.len() - 1]
        };
        sheet.labels.push(mark_label(
            (top.0, top.1 + 0.005),
            90.0,
            Anchor::Start,
            name.clone(),
        ));
        sheet.paths.push(SheetPath {
            kind: PathKind::Mark,
            points: line,
            closed: false,
        });
    }

    // A tick across the edge wherever a station crosses it.
    for lp in &loops {
        for k in 0..lp.len() {
            let (a, b) = (lp[k], lp[(k + 1) % lp.len()]);
            let (xa, xb) = (found.verts[a].x, found.verts[b].x);
            for (_, x) in stations {
                if (xa >= *x) == (xb >= *x) {
                    continue;
                }
                let t = ((x - xa) / (xb - xa)).clamp(0.0, 1.0);
                let p = (
                    xy[a].0 + t * (xy[b].0 - xy[a].0),
                    xy[a].1 + t * (xy[b].1 - xy[a].1),
                );
                let (dx, dy) = (xy[b].0 - xy[a].0, xy[b].1 - xy[a].1);
                let l = (dx * dx + dy * dy).sqrt().max(1e-12);
                let n = (-dy / l * TICK_M / 2.0, dx / l * TICK_M / 2.0);
                sheet.paths.push(SheetPath {
                    kind: PathKind::Mark,
                    points: vec![(p.0 - n.0, p.1 - n.1), (p.0 + n.0, p.1 + n.1)],
                    closed: false,
                });
            }
        }
    }

    // A joined panel's centreline, which a seam across it is aligned by.
    for run in &found.centreline {
        let Some(ids) = found.half_local(run) else {
            continue;
        };
        let pts: Vec<(f64, f64)> = ids.iter().map(|&i| xy[i]).collect();
        if let Some((p, d)) = midpoint(&pts) {
            let off = (
                -d.1 * SEAM_LABEL_INSET_M / 3.0,
                d.0 * SEAM_LABEL_INSET_M / 3.0,
            );
            sheet.labels.push(mark_label(
                (p.0 + off.0, p.1 + off.1),
                reading_angle(d),
                Anchor::Middle,
                "Centreline".into(),
            ));
        }
        sheet.paths.push(SheetPath {
            kind: PathKind::Mark,
            points: pts,
            closed: false,
        });
    }

    // What each edge is stitched to, inside the edge at its middle; on a joined panel, a seam
    // that does not run across the centreline is labelled on both sides.
    let mut runs = Vec::new();
    for seam in &found.seams {
        let Some(ids) = found.seam_local(seam) else {
            continue;
        };
        if !seam.across {
            if let Some(m) = found.mirror_local(&ids) {
                runs.push((seam, m));
            }
        }
        runs.push((seam, ids));
    }
    for (seam, ids) in runs {
        let pts: Vec<(f64, f64)> = ids.iter().map(|&i| xy[i]).collect();
        let Some((p, d)) = midpoint(&pts) else {
            continue;
        };
        // The outline runs anticlockwise, so the panel is on the left of a seam that runs with
        // it; a seam that runs against it has the panel on its right.
        let inward = (-d.1, d.0);
        let probe = (
            p.0 + inward.0 * SEAM_LABEL_INSET_M,
            p.1 + inward.1 * SEAM_LABEL_INSET_M,
        );
        let inside = point_in_panel(found, xy, probe);
        let s = if inside { 1.0 } else { -1.0 };
        let at = (
            p.0 + s * inward.0 * SEAM_LABEL_INSET_M,
            p.1 + s * inward.1 * SEAM_LABEL_INSET_M,
        );
        let text = match &seam.mate {
            Mate::Panel(m) => format!("Stitch to {m}"),
            Mate::Centreline => "Centreline: stitch to its mirror".into(),
            Mate::Open => "Open edge".into(),
        };
        sheet
            .labels
            .push(mark_label(at, reading_angle(d), Anchor::Middle, text));
    }

    // Title, copies and the tick rule in the middle of the panel.
    let (lo, hi) = xy.iter().fold(
        ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN)),
        |(lo, hi), p| {
            (
                (lo.0.min(p.0), lo.1.min(p.1)),
                (hi.0.max(p.0), hi.1.max(p.1)),
            )
        },
    );
    let centre = ((lo.0 + hi.0) / 2.0, (lo.1 + hi.1) / 2.0);
    sheet.labels.push(Label {
        at: (centre.0, centre.1 + 0.01),
        height_mm: TITLE_LABEL_MM,
        angle_deg: 0.0,
        anchor: Anchor::Middle,
        text: found.name.clone(),
    });
    let copies = match found.copies {
        Copies::One => "Cut 1".to_string(),
        Copies::MirroredPair => "Cut 2, one turned over".to_string(),
    };
    sheet.labels.push(mark_label(
        (centre.0, centre.1 - 0.005),
        0.0,
        Anchor::Middle,
        copies,
    ));
    if !stations.is_empty() {
        sheet.labels.push(mark_label(
            (centre.0, centre.1 - 0.015),
            0.0,
            Anchor::Middle,
            "Ticks match from either end".into(),
        ));
    }

    let m = &panel.measures;
    if !panel.lies_flat {
        sheet.notes.push(failing_heading(m, limits));
    }
    if let Some(d) = m.unsettled_m {
        sheet.notes.push(unsettled_note(d));
    }
    sheet.notes.push(format!(
        "Stretch: {:.3} % RMS, {:.3} % largest (limit {:.1} % RMS)",
        m.rms_stretch * 100.0,
        m.max_stretch * 100.0,
        limits.rms_stretch * 100.0
    ));
    let drifts: Vec<String> = m
        .seams
        .iter()
        .map(|s| format!("{} {:.1} mm", s.name, s.drift_m * 1000.0))
        .collect();
    sheet.notes.push(format!(
        "Seam drift from either end (limit {:.0} mm): {}",
        limits.seam_drift_m * 1000.0,
        drifts.join("; ")
    ));
    sheet.notes.push(match m.level_shift_m {
        Some(d) => format!(
            "Outline shift to the next subdivision level: {:.2} mm",
            d * 1000.0
        ),
        None => "Outline shift to the next subdivision level: not measured".into(),
    });
    sheet.notes.push(format!(
        "Edge-joining gap: {:.1} mm",
        m.edge_join_gap_m * 1000.0
    ));
    sheet.notes.push(format!(
        "Size {} by {}; area {:.3} m² flat, {:.3} m² on the model",
        crate::units::length(hi.0 - lo.0, imperial),
        crate::units::length(hi.1 - lo.1, imperial),
        m.area_flat_m2,
        m.area_3d_m2
    ));
    sheet.notes.push(THICKNESS_NOTE.into());
    sheet.notes.push(MEASURED_NOTE.into());
    sheet.notes.push(crate::NOT_VALIDATED_NOTE.into());
    sheet
}

/// Whether a flat point lies on the panel.
fn point_in_panel(found: &FoundPanel, xy: &[(f64, f64)], p: (f64, f64)) -> bool {
    found.tris.iter().any(|t| {
        let (a, b, c) = (xy[t[0]], xy[t[1]], xy[t[2]]);
        let s1 = crate::flatten::signed_area(a, b, p);
        let s2 = crate::flatten::signed_area(b, c, p);
        let s3 = crate::flatten::signed_area(c, a, p);
        s1 >= 0.0 && s2 >= 0.0 && s3 >= 0.0
    })
}

/// One row of a panel's offsets: how far along the baseline, and where the outline crosses the
/// square line there, measured off the baseline, lowest first.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct OffsetRow {
    pub along_m: f64,
    pub ordinates_m: Vec<f64>,
}

/// A panel's outline as offsets from a straight baseline, for laying it out on the sheet with a
/// batten instead of a full-size print.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PanelOffsets {
    pub panel: String,
    pub rows: Vec<OffsetRow>,
}

/// The offsets of a flat panel from a baseline along its lowest edge (`y` = the pattern's least
/// `y`), with rows every `spacing_m` from its aft end (least `x`) and one at its forward end.
pub fn panel_offsets(panel: &FlatPanel, spacing_m: f64) -> PanelOffsets {
    let xy = &panel.xy;
    let loops = boundary_loops(&panel.found.tris);
    let (x0, x1, y0) = xy.iter().fold((f64::MAX, f64::MIN, f64::MAX), |m, p| {
        (m.0.min(p.0), m.1.max(p.0), m.2.min(p.1))
    });
    let width = x1 - x0;
    let mut along = Vec::new();
    if spacing_m > 0.0 && spacing_m.is_finite() && width > 0.0 {
        let mut k = 0usize;
        while (k as f64) * spacing_m < width - 1e-9 * width.max(1.0) {
            along.push(k as f64 * spacing_m);
            k += 1;
        }
    }
    along.push(width.max(0.0));
    // The end rows are taken a hair inside the panel, where the square line still crosses it.
    let nudge = 1e-9 * width.max(1.0);
    let rows = along
        .into_iter()
        .map(|a| {
            let x = (x0 + a).clamp(x0 + nudge, x1 - nudge);
            let mut ys = Vec::new();
            for lp in &loops {
                for k in 0..lp.len() {
                    let (p, q) = (xy[lp[k]], xy[lp[(k + 1) % lp.len()]]);
                    if (p.0 >= x) != (q.0 >= x) {
                        let t = (x - p.0) / (q.0 - p.0);
                        ys.push(p.1 + t * (q.1 - p.1) - y0);
                    }
                }
            }
            ys.sort_by(f64::total_cmp);
            ys.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
            OffsetRow {
                along_m: a,
                ordinates_m: ys,
            }
        })
        .collect();
    PanelOffsets {
        panel: panel.found.name.clone(),
        rows,
    }
}

fn csv_field(text: &str) -> String {
    if text.contains([',', '"', '\n']) {
        format!("\"{}\"", text.replace('"', "\"\""))
    } else {
        text.to_owned()
    }
}

/// Every panel's offsets as CSV, whole millimetres: `panel,along_mm,crossing,ordinate_mm`, one
/// line per crossing, crossings numbered from the baseline up.
pub fn offsets_csv(all: &[PanelOffsets]) -> String {
    let mut out = String::from("panel,along_mm,crossing,ordinate_mm\n");
    for o in all {
        let name = csv_field(&o.panel);
        for r in &o.rows {
            for (i, y) in r.ordinates_m.iter().enumerate() {
                out.push_str(&format!(
                    "{name},{},{},{}\n",
                    crate::units::millimetres(r.along_m),
                    i + 1,
                    crate::units::millimetres(*y)
                ));
            }
        }
    }
    out
}

/// Every panel's offsets as tables, one per panel (and a second in feet, inches and eighths with
/// `imperial`), A4 landscape.
pub fn offsets_pdf(
    all: &[PanelOffsets],
    title: &str,
    imperial: bool,
) -> Result<crate::pdf::Document, String> {
    use crate::pdf::{add_table, Pdf, Table};
    use crate::units::{ft_in_8ths, millimetres};
    if all.is_empty() {
        return Err("there are no panel offsets to tabulate".into());
    }
    let subtitle =
        "Square off a straight baseline along the panel's lowest edge, from its aft end. \
                    Crossings count up from the baseline; \"-\" means the row has fewer.";
    let mut doc = Pdf::new(title);
    type Write = fn(f64) -> String;
    let mut systems: Vec<(&str, Write)> = vec![("millimetres", millimetres)];
    if imperial {
        systems.push(("feet-inches-eighths", ft_in_8ths));
    }
    for o in all {
        let crossings = o
            .rows
            .iter()
            .map(|r| r.ordinates_m.len())
            .max()
            .unwrap_or(0);
        for (unit, length) in &systems {
            let mut header = vec!["Along".to_owned()];
            header.extend((1..=crossings).map(|i| format!("Crossing {i}")));
            let rows: Vec<Vec<String>> = o
                .rows
                .iter()
                .map(|r| {
                    let mut row = vec![length(r.along_m)];
                    row.extend((0..crossings).map(|i| {
                        r.ordinates_m
                            .get(i)
                            .map_or_else(|| "-".into(), |y| length(*y))
                    }));
                    row
                })
                .collect();
            let heading = format!("{title}: {} offsets, {unit}", o.panel);
            add_table(
                &mut doc,
                &Table {
                    title: &heading,
                    subtitle,
                    header: &header,
                    rows: &rows,
                    footer: crate::NOT_VALIDATED_NOTE,
                    repeat: 1,
                },
            );
        }
    }
    doc.finish()
}

/// The plywood the panels are cut from.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Stock {
    pub length_m: f64,
    pub width_m: f64,
    /// How much each joined sheet overlaps the one before; 0 for butt joints.
    pub scarf_m: f64,
}

impl Default for Stock {
    fn default() -> Self {
        Self {
            length_m: 2.44,
            width_m: 1.22,
            scarf_m: 0.0,
        }
    }
}

/// How many sheets one panel takes, laid end to end along its length.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct StockRow {
    pub panel: String,
    pub copies: usize,
    /// Sheets for one copy; `None` when it could not be laid out.
    pub sheets_each: Option<usize>,
    /// Where each sheet's reach ends along the panel, from its aft end, metres.
    pub joints_m: Vec<f64>,
    pub note: Option<String>,
}

/// The pattern's extent: `(min x, min y, length, width)`.
fn extent(panel: &FlatPanel) -> (f64, f64, f64, f64) {
    let (x0, y0, x1, y1) = panel
        .xy
        .iter()
        .fold((f64::MAX, f64::MAX, f64::MIN, f64::MIN), |m, p| {
            (m.0.min(p.0), m.1.min(p.1), m.2.max(p.0), m.3.max(p.1))
        });
    (x0, y0, x1 - x0, y1 - y0)
}

/// Lays one panel along sheets of `stock` joined end to end, with no nesting: an upper bound
/// on what it takes.
pub fn lay_out(panel: &FlatPanel, stock: Stock) -> StockRow {
    let (_, _, length, width) = extent(panel);
    let mut row = StockRow {
        panel: panel.found.name.clone(),
        copies: panel.found.copies.count(),
        sheets_each: None,
        joints_m: Vec::new(),
        note: None,
    };
    let reach = stock.length_m - stock.scarf_m.max(0.0);
    if width > stock.width_m + 1e-9 {
        row.note = Some(format!(
            "wider than the stock ({} mm > {} mm): not laid out",
            crate::units::millimetres(width),
            crate::units::millimetres(stock.width_m)
        ));
        return row;
    }
    if reach <= 0.0 || !reach.is_finite() {
        row.note = Some("the scarf is as long as the sheet: not laid out".into());
        return row;
    }
    let mut end = stock.length_m;
    let mut sheets = 1;
    while end < length - 1e-9 {
        row.joints_m.push(end);
        end += reach;
        sheets += 1;
    }
    row.sheets_each = Some(sheets);
    row
}

/// The sheets of a laid-out panel drawn on its pattern for the eye: each sheet's outline and
/// each joint, cut back to the pattern's extent so they add no paper to a tiled print.
pub fn stock_paths(row: &StockRow, panel: &FlatPanel, stock: Stock) -> Vec<SheetPath> {
    let Some(sheets) = row.sheets_each else {
        return Vec::new();
    };
    let (x0, y0, length, width) = extent(panel);
    let (x1, y1) = (x0 + length, y0 + width.min(stock.width_m));
    let reference = |points: Vec<(f64, f64)>, closed: bool| SheetPath {
        kind: PathKind::Reference,
        points,
        closed,
    };
    let mut out = Vec::new();
    for k in 0..sheets {
        let start = if k == 0 {
            x0
        } else {
            x0 + row.joints_m[k - 1] - stock.scarf_m.max(0.0)
        };
        let (a, b) = (start, (start + stock.length_m).min(x1));
        out.push(reference(vec![(a, y0), (b, y0), (b, y1), (a, y1)], true));
    }
    for j in &row.joints_m {
        out.push(reference(vec![(x0 + j, y0), (x0 + j, y1)], false));
    }
    out
}

/// Sheets for every copy of every laid-out panel.
pub fn total_sheets(rows: &[StockRow]) -> usize {
    rows.iter()
        .filter_map(|r| r.sheets_each.map(|s| s * r.copies))
        .sum()
}

/// What a panel set is made with.
#[derive(Debug, Clone)]
pub struct PanelOptions {
    pub subdivision: SubdivisionOptions,
    /// Layer names; `None` takes [`default_layers`].
    pub layers: Option<Vec<String>>,
    pub stations: StationChoice,
    pub limits: Limits,
    pub offset_spacing_m: f64,
    pub stock: Stock,
    pub imperial: bool,
}

impl Default for PanelOptions {
    fn default() -> Self {
        Self {
            subdivision: SubdivisionOptions::default(),
            layers: None,
            stations: StationChoice::Count(11),
            limits: Limits::default(),
            offset_spacing_m: 0.25,
            stock: Stock::default(),
            imperial: false,
        }
    }
}

/// One panel in the summary table.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct PanelRow {
    pub panel: String,
    pub copies: usize,
    pub lies_flat: bool,
    pub rms_stretch_pct: f64,
    pub max_stretch_pct: f64,
    pub max_seam_drift_mm: f64,
    pub level_shift_mm: Option<f64>,
    pub edge_join_gap_mm: f64,
    pub area_m2: f64,
    pub sheets_each: Option<usize>,
    /// `None` when the flattening settled; otherwise how far its points may still move, mm.
    pub unsettled_mm: Option<f64>,
}

/// Every panel of a hull: patterns, measures, offsets and the plywood they take.
#[derive(Debug, Clone)]
pub struct PanelSet {
    pub title: String,
    pub sheets: Vec<Sheet>,
    pub rows: Vec<PanelRow>,
    pub mismatches: Vec<Mismatch>,
    pub refused: Vec<Refusal>,
    pub offsets: Vec<PanelOffsets>,
    pub stock: Vec<StockRow>,
    pub total_sheets: usize,
    /// Station names and model `x`, as the patterns mark them.
    pub stations: Vec<(String, f64)>,
}

impl PanelSet {
    /// No panel refused and every panel within the limits.
    pub fn all_lie_flat(&self) -> bool {
        self.refused.is_empty() && self.rows.iter().all(|r| r.lies_flat)
    }

    /// The plywood count in words, naming any panel it leaves out and why: one not laid out on
    /// the stock, or one that could not be laid flat at all.
    pub fn sheet_count_line(&self) -> String {
        let mut left_out: Vec<String> = self
            .stock
            .iter()
            .filter(|r| r.sheets_each.is_none())
            .map(|r| {
                let why = r.note.as_deref().unwrap_or("not laid out");
                let why = why.split(" (").next().unwrap_or(why);
                format!("{} ({})", r.panel, why.trim_end_matches(": not laid out"))
            })
            .collect();
        left_out.extend(
            self.refused
                .iter()
                .map(|r| format!("{} (could not be laid flat)", r.panel)),
        );
        if left_out.is_empty() {
            return format!(
                "{} plywood sheets, end to end, no nesting: an upper bound",
                self.total_sheets
            );
        }
        let all = self.rows.len() + self.refused.len();
        format!(
            "{} plywood sheets for {} of {all} panels, end to end, no nesting: an upper bound for \
             those; not counted: {}",
            self.total_sheets,
            all - left_out.len(),
            left_out.join(", ")
        )
    }
}

fn positive(v: f64) -> bool {
    v.is_finite() && v > 0.0
}

/// The layer indices named, or the defaults. An unknown name or a layer with no faces is an
/// error listing the model's layers.
fn chosen_layers(surface: &Surface, names: Option<&[String]>) -> Result<Vec<usize>, String> {
    let all: Vec<&str> = surface.layers.iter().map(|l| l.name.as_str()).collect();
    let listing = || format!("the model's layers are {}", all.join(", "));
    let Some(names) = names else {
        let layers = default_layers(surface);
        if layers.is_empty() {
            return Err(format!(
                "no layer is marked developable or used in hydrostatics; {}",
                listing()
            ));
        }
        return Ok(layers);
    };
    let mut out = Vec::new();
    for name in names {
        let Some(i) = surface.layers.iter().position(|l| l.name == *name) else {
            return Err(format!("there is no layer named `{name}`; {}", listing()));
        };
        if !surface.face_layer.contains(&i) {
            return Err(format!("layer `{name}` has no faces; {}", listing()));
        }
        if !out.contains(&i) {
            out.push(i);
        }
    }
    Ok(out)
}

/// Finds, flattens and measures the panels and draws their patterns.
pub fn panel_set(surface: &Surface, opts: &PanelOptions) -> Result<PanelSet, String> {
    let l = opts.limits;
    if !positive(l.seam_drift_m) || !positive(l.rms_stretch) {
        return Err("the seam drift and stretch limits must be more than zero".into());
    }
    if !positive(opts.offset_spacing_m) {
        return Err("the offset spacing must be more than zero".into());
    }
    let st = opts.stock;
    if !positive(st.length_m) || !positive(st.width_m) {
        return Err("the plywood sheet must have a length and a width".into());
    }
    if !st.scarf_m.is_finite() || st.scarf_m < 0.0 || st.scarf_m >= st.length_m {
        return Err("the scarf must be from 0 to less than the sheet's length".into());
    }
    let layers = chosen_layers(surface, opts.layers.as_deref())?;
    let hull = crate::hydro::Hull::new(surface, opts.subdivision);
    if hull.tris.is_empty() {
        return Err("the model has no surface to make panels from".into());
    }
    let xs = station_positions(&hull, &opts.stations)?;
    let stations: Vec<(String, f64)> = xs
        .iter()
        .enumerate()
        .map(|(i, &x)| (format!("St {}", i + 1), x))
        .collect();
    let (panels, mismatches, refused) =
        measure_panels(surface, opts.subdivision, &layers, &xs, opts.limits);
    if panels.is_empty() && refused.is_empty() {
        return Err("no panel was found on the chosen layers".into());
    }
    let name = surface.settings.name.trim();
    let mut set = PanelSet {
        title: if name.is_empty() {
            "Untitled hull".into()
        } else {
            name.to_owned()
        },
        sheets: Vec::new(),
        rows: Vec::new(),
        mismatches,
        refused,
        offsets: Vec::new(),
        stock: Vec::new(),
        total_sheets: 0,
        stations: stations.clone(),
    };
    for p in &panels {
        let mut sheet = panel_sheet(p, &stations, opts.limits, opts.imperial);
        let row = lay_out(p, st);
        sheet.paths.extend(stock_paths(&row, p, st));
        let plywood = match (&row.note, row.sheets_each) {
            (Some(note), _) => format!("Plywood: {note}"),
            (None, Some(n)) => {
                let joints: Vec<String> = row
                    .joints_m
                    .iter()
                    .map(|j| crate::units::length(*j, opts.imperial))
                    .collect();
                let sheets = if n == 1 { "sheet" } else { "sheets" };
                let mut text = format!(
                    "Plywood: {n} {sheets} of {} × {} mm each copy",
                    crate::units::millimetres(st.length_m),
                    crate::units::millimetres(st.width_m)
                );
                if !joints.is_empty() {
                    text.push_str(&format!(", joints {} from the aft end", joints.join(", ")));
                }
                text
            }
            (None, None) => "Plywood: not laid out".into(),
        };
        // Before the three closing sentences.
        let at = sheet.notes.len().saturating_sub(3);
        sheet.notes.insert(at, plywood);
        if let Some(note) = mismatch_note(p, &set.mismatches) {
            let at = sheet
                .notes
                .iter()
                .position(|n| n.starts_with("Seam drift"))
                .map_or(0, |i| i + 1);
            sheet.notes.insert(at, note);
        }
        let m = &p.measures;
        set.rows.push(PanelRow {
            panel: p.found.name.clone(),
            copies: p.found.copies.count(),
            lies_flat: p.lies_flat,
            rms_stretch_pct: m.rms_stretch * 100.0,
            max_stretch_pct: m.max_stretch * 100.0,
            max_seam_drift_mm: m.max_seam_drift_m() * 1000.0,
            level_shift_mm: m.level_shift_m.map(|d| d * 1000.0),
            edge_join_gap_mm: m.edge_join_gap_m * 1000.0,
            area_m2: m.area_flat_m2,
            sheets_each: row.sheets_each,
            unsettled_mm: m.unsettled_m.map(|d| d * 1000.0),
        });
        set.offsets.push(panel_offsets(p, opts.offset_spacing_m));
        set.stock.push(row);
        set.sheets.push(sheet);
    }
    set.total_sheets = total_sheets(&set.stock);
    // A panel that could not be made is named on every pattern, before the closing sentences.
    for sheet in &mut set.sheets {
        for r in &set.refused {
            let at = sheet.notes.len().saturating_sub(3);
            sheet
                .notes
                .insert(at, format!("Not in this set: {}: {}", r.panel, r.reason));
        }
    }
    Ok(set)
}

/// Writes a panel set in `format`, as [`crate::templates::write`] writes moulds.
pub fn write(
    set: &PanelSet,
    format: crate::templates::Format,
    setup: &crate::sheet::PageSetup,
) -> Result<crate::templates::Written, String> {
    use crate::templates::{Format, Written};
    setup.check()?;
    if set.sheets.is_empty() {
        return Err("no panel could be drawn".into());
    }
    let mut setup = setup.clone();
    if setup.title.trim().is_empty() {
        setup.title = set.title.clone();
    }
    let pdf = |doc: crate::pdf::Document| Written {
        pages: Some(doc.pages),
        missing: doc.missing,
        bytes: doc.bytes,
    };
    let text = |text: String| Written {
        bytes: text.into_bytes(),
        pages: None,
        missing: Vec::new(),
    };
    match format {
        Format::PdfTiled => crate::sheet::pdf_tiled(&set.sheets, &setup).map(pdf),
        Format::PdfSingle => crate::sheet::pdf_single(&set.sheets, &setup).map(pdf),
        Format::Svg => Ok(text(crate::sheet::svg(&set.sheets, setup.scale))),
        Format::Dxf => Ok(text(crate::sheet::dxf(&set.sheets, setup.scale))),
    }
}

/// A 2.4 m pram for tests: **not an example hull**. Flat bottom (`deadrise_deg` 0) or a V bottom,
/// flared sides that are planar when `side_bow_m` is 0 and bowed outward amidships by it
/// otherwise, a stern transom 0.40 m deep and a bow transom 0.50 m deep. Layers `Bottom` and
/// `Side` (symmetric, developable) and `Transoms` (symmetric); creases on the chines and round
/// both transoms; the sheer is open.
#[doc(hidden)]
pub fn pram_fixture(deadrise_deg: f64, side_bow_m: f64) -> Surface {
    pram_net(deadrise_deg, side_bow_m).0
}

/// The pram and the control points of its sides' middle row, stern to bow.
fn pram_net(deadrise_deg: f64, side_bow_m: f64) -> (Surface, Vec<usize>) {
    let length = 2.4;
    let xs = [0.0, 0.6, 1.2, 1.8, 2.4];
    let (b_stern, b_bow) = (0.45, 0.35);
    let (h_stern, h_bow) = (0.40, 0.50);
    let flare = 15f64.to_radians();
    let tan_d = deadrise_deg.to_radians().tan();
    let lerp = |a: f64, b: f64, x: f64| a + (b - a) * x / length;
    let chine = |x: f64| {
        let b = lerp(b_stern, b_bow, x);
        Vec3::new(x, b, b * tan_d)
    };
    // The side plane holds the straight chine and the flare direction.
    let w = Vec3::new(0.0, flare.sin(), flare.cos());
    let along = (chine(length) - chine(0.0)).norm();
    let mut out = along.cross(w).norm();
    if out.y < 0.0 {
        out = out * -1.0;
    }
    let side = |x: f64, frac: f64| {
        let k = chine(x);
        let h = (lerp(h_stern, h_bow, x) - k.z) / w.z;
        k + w * (h * frac)
    };

    let mut s = Surface::new();
    s.layers = vec![
        Layer {
            name: "Bottom".into(),
            id: 1,
            developable: true,
            ..Layer::default()
        },
        Layer {
            name: "Side".into(),
            id: 2,
            developable: true,
            ..Layer::default()
        },
        Layer {
            name: "Transoms".into(),
            id: 3,
            ..Layer::default()
        },
    ];
    let mut c = Vec::new();
    let mut k = Vec::new();
    let mut m = Vec::new();
    let mut t = Vec::new();
    let mut top = Vec::new();
    for &x in &xs {
        c.push(s.add_point(Vec3::new(x, 0.0, 0.0)));
        k.push(s.add_point(chine(x)));
        let bow = side_bow_m * (std::f64::consts::PI * x / length).sin();
        m.push(s.add_point(side(x, 0.5) + out * bow));
        t.push(s.add_point(side(x, 1.0)));
        top.push(s.add_point(Vec3::new(x, 0.0, lerp(h_stern, h_bow, x))));
    }
    let n = xs.len();
    for i in 0..n - 1 {
        s.add_face(vec![c[i], k[i], k[i + 1], c[i + 1]], 0);
        s.add_face(vec![k[i], m[i], m[i + 1], k[i + 1]], 1);
        s.add_face(vec![m[i], t[i], t[i + 1], m[i + 1]], 1);
    }
    s.add_face(vec![c[0], top[0], t[0], m[0], k[0]], 2);
    let l = n - 1;
    s.add_face(vec![c[l], k[l], m[l], t[l], top[l]], 2);
    s.rebuild_edges();
    let mut creases = Vec::new();
    for i in 0..n - 1 {
        creases.push((k[i], k[i + 1]));
    }
    for e in [0, l] {
        creases.push((c[e], k[e]));
        creases.push((k[e], m[e]));
        creases.push((m[e], t[e]));
    }
    set_creases(&mut s, &creases);
    (s, m)
}

fn set_creases(s: &mut Surface, pairs: &[(usize, usize)]) {
    for e in &mut s.edges {
        if pairs
            .iter()
            .any(|&(a, b)| crate::mesh::ordered(a, b) == e.key())
        {
            e.crease = true;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::flatten::boundary_loops;
    use crate::subdiv::SubdivisionOptions;

    fn all(s: &Surface) -> Vec<usize> {
        (0..s.layers.len()).collect()
    }

    fn panel<'a>(ps: &'a [FoundPanel], name: &str) -> &'a FoundPanel {
        ps.iter()
            .find(|p| p.name == name)
            .unwrap_or_else(|| panic!("no panel {name}: {:?}", names(ps)))
    }

    fn names(ps: &[FoundPanel]) -> Vec<String> {
        ps.iter().map(|p| p.name.clone()).collect()
    }

    fn centroid_x(p: &FoundPanel) -> f64 {
        p.verts.iter().map(|v| v.x).sum::<f64>() / p.verts.len() as f64
    }

    #[test]
    fn a_flat_bottom_pram_has_one_bottom_two_sides_two_transoms() {
        let s = pram_fixture(0.0, 0.0);
        let (ps, refused) = find_panels(&s, SubdivisionOptions::default(), &all(&s));
        assert!(refused.is_empty(), "{refused:?}");
        let mut got = names(&ps);
        got.sort();
        assert_eq!(got, ["Bottom", "Side", "Transoms 1", "Transoms 2"]);
        assert_eq!(panel(&ps, "Bottom").copies, Copies::One);
        assert_eq!(panel(&ps, "Side").copies, Copies::MirroredPair);
        assert_eq!(panel(&ps, "Transoms 1").copies, Copies::One);
        assert_eq!(panel(&ps, "Transoms 2").copies, Copies::One);
        assert!(centroid_x(panel(&ps, "Transoms 1")) < centroid_x(panel(&ps, "Transoms 2")));
        for p in &ps {
            assert_eq!(boundary_loops(&p.tris).len(), 1, "{} is not a disc", p.name);
            assert_eq!(p.mesh_ids.len(), p.verts.len());
        }
        // The joined bottom spans both sides of the centreline.
        let b = panel(&ps, "Bottom");
        assert!(b.verts.iter().any(|v| v.y < -0.3) && b.verts.iter().any(|v| v.y > 0.3));
    }

    #[test]
    fn a_v_bottom_is_split_at_the_keel() {
        let s = pram_fixture(12.0, 0.0);
        let (ps, refused) = find_panels(&s, SubdivisionOptions::default(), &all(&s));
        assert!(refused.is_empty(), "{refused:?}");
        let b = panel(&ps, "Bottom");
        assert_eq!(b.copies, Copies::MirroredPair);
        assert!(b.seams.iter().any(|s| s.mate == Mate::Centreline));
        assert!(b.verts.iter().all(|v| v.y >= 0.0));
    }

    #[test]
    fn seams_name_their_mates() {
        let s = pram_fixture(0.0, 0.0);
        let (ps, _) = find_panels(&s, SubdivisionOptions::default(), &all(&s));
        let side = panel(&ps, "Side");
        let mates: Vec<Mate> = side.seams.iter().map(|s| s.mate.clone()).collect();
        assert_eq!(
            mates,
            [
                Mate::Panel("Bottom".into()),
                Mate::Panel("Transoms 1".into()),
                Mate::Panel("Transoms 2".into()),
                Mate::Open
            ]
        );
        let seam_names: Vec<String> = side.seams.iter().map(Seam::name).collect();
        assert_eq!(
            seam_names,
            [
                "edge to Bottom",
                "edge to Transoms 1",
                "edge to Transoms 2",
                "open edge"
            ]
        );
        let chine_side = &side.seams[0].verts;
        let bottom = panel(&ps, "Bottom");
        let chine_bottom = &bottom
            .seams
            .iter()
            .find(|s| s.mate == Mate::Panel("Side".into()))
            .expect("bottom meets the side")
            .verts;
        let mut reversed = chine_bottom.clone();
        reversed.reverse();
        assert!(chine_side == chine_bottom || *chine_side == reversed);
        assert!(chine_side.len() > 2);
    }

    fn summary(ps: &[FoundPanel]) -> Vec<(String, Copies, Vec<String>, usize)> {
        let mut v: Vec<_> = ps
            .iter()
            .map(|p| {
                (
                    p.name.clone(),
                    p.copies,
                    p.seams.iter().map(Seam::name).collect(),
                    p.tris.len(),
                )
            })
            .collect();
        v.sort_by(|a, b| a.0.cmp(&b.0));
        v
    }

    #[test]
    fn names_do_not_depend_on_face_order() {
        let s = pram_fixture(0.0, 0.0);
        let mut p = s.clone();
        let n = p.faces.len();
        for i in 0..n {
            let j = (i * 7 + 3) % n;
            p.faces.swap(i, j);
            p.face_layer.swap(i, j);
        }
        let opts = SubdivisionOptions::default();
        let (a, _) = find_panels(&s, opts, &all(&s));
        let (b, _) = find_panels(&p, opts, &all(&p));
        assert_eq!(summary(&a), summary(&b));
    }

    #[test]
    fn a_crease_that_ends_inside_a_panel_is_refused() {
        let (mut s, m) = pram_net(0.0, 0.0);
        set_creases(&mut s, &[(m[0], m[1]), (m[1], m[2])]);
        let (ps, refused) = find_panels(&s, SubdivisionOptions::default(), &all(&s));
        assert!(!names(&ps).contains(&"Side".to_string()));
        let r = refused
            .iter()
            .find(|r| r.panel == "Side")
            .expect("Side refused");
        assert!(r.reason.contains("x 1.20 m"), "{}", r.reason);
        assert!(
            r.reason
                .contains("carry the crease to the panel's edge or split the layer"),
            "{}",
            r.reason
        );
        // The other panels are still found.
        assert!(names(&ps).contains(&"Bottom".to_string()));
    }

    #[test]
    fn a_layer_that_is_not_symmetric_gives_one_copy() {
        let mut s = pram_fixture(0.0, 0.0);
        s.layers[1].symmetric = false;
        let (ps, _) = find_panels(&s, SubdivisionOptions::default(), &all(&s));
        let side = panel(&ps, "Side");
        assert_eq!(side.copies, Copies::One);
        assert!(side.verts.iter().all(|v| v.y > 0.0));
    }

    #[test]
    fn only_the_asked_layers_are_returned_but_mates_keep_their_names() {
        let s = pram_fixture(0.0, 0.0);
        let (ps, _) = find_panels(&s, SubdivisionOptions::default(), &[1]);
        assert_eq!(names(&ps), ["Side"]);
        assert_eq!(ps[0].seams[1].mate, Mate::Panel("Transoms 1".into()));
    }

    const STATIONS: [f64; 4] = [0.3, 0.9, 1.5, 2.1];

    fn measured(s: &Surface, level: u8) -> (Vec<FlatPanel>, Vec<Mismatch>, Vec<Refusal>) {
        let opts = SubdivisionOptions {
            level,
            ..SubdivisionOptions::default()
        };
        measure_panels(s, opts, &all(s), &STATIONS, Limits::default())
    }

    fn flat<'a>(ps: &'a [FlatPanel], name: &str) -> &'a FlatPanel {
        ps.iter().find(|p| p.found.name == name).expect(name)
    }

    /// Closed form: every panel of the flat pram is planar, so it lies flat with no stretch and
    /// every seam keeps its length.
    #[test]
    fn a_flat_pram_lies_flat_with_true_seams() {
        let s = pram_fixture(0.0, 0.0);
        let (ps, _, refused) = measured(&s, 3);
        assert!(refused.is_empty(), "{refused:?}");
        assert_eq!(ps.len(), 4);
        for p in &ps {
            let m = &p.measures;
            assert!(p.lies_flat, "{}: {m:?}", p.found.name);
            assert!(m.rms_stretch < 1e-9, "{}: {m:?}", p.found.name);
            for seam in &m.seams {
                assert!(seam.drift_m < 1e-6, "{}: {seam:?}", p.found.name);
            }
            assert!(
                (m.area_flat_m2 - m.area_3d_m2).abs() < 1e-9,
                "{}: {m:?}",
                p.found.name
            );
        }
    }

    /// Arithmetic: drift 3 mm reached at the first tick and kept, or reached only at the end,
    /// is 3 mm from whichever end it is measured.
    #[test]
    fn drift_is_measured_from_both_ends() {
        let a = drift_from_both_ends(&[0.0, 1.0, 2.0], &[0.0, 1.003, 2.003]);
        let b = drift_from_both_ends(&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.003]);
        assert!((a - 0.003).abs() < 1e-12, "{a}");
        assert!((b - 0.003).abs() < 1e-12, "{b}");
        // The same seams measured from the other end.
        let a2 = drift_from_both_ends(&[0.0, 1.0, 2.0], &[0.0, 1.0, 2.003]);
        let b2 = drift_from_both_ends(&[0.0, 1.0, 2.0], &[0.0, 1.003, 2.003]);
        assert!((a2 - a).abs() < 1e-12 && (b2 - b).abs() < 1e-12);
        // Drift in the middle that comes back by the end still counts.
        let c = drift_from_both_ends(&[0.0, 1.0, 2.0], &[0.0, 1.004, 2.0]);
        assert!((c - 0.004).abs() < 1e-12, "{c}");
    }

    #[test]
    fn mating_seams_are_compared() {
        let s = pram_fixture(0.0, 0.0);
        let (_, mismatches, _) = measured(&s, 3);
        let chine = mismatches
            .iter()
            .find(|m| (m.a == "Bottom" && m.b == "Side") || (m.a == "Side" && m.b == "Bottom"))
            .unwrap_or_else(|| panic!("{mismatches:?}"));
        assert!(chine.max_m < 1e-6, "{chine:?}");
        // Every seam to a found panel is compared once: 2 per transom + the chine.
        assert_eq!(mismatches.len(), 5, "{mismatches:?}");
    }

    #[test]
    fn a_face_turned_the_other_way_refuses_its_panel() {
        let mut s = pram_fixture(0.0, 0.0);
        let fi = s.face_layer.iter().position(|&l| l == 1).unwrap() + 3;
        s.faces[fi].reverse();
        s.rebuild_edges();
        let (ps, refused) = find_panels(&s, SubdivisionOptions::default(), &all(&s));
        assert!(
            !names(&ps).contains(&"Side".to_string()),
            "{:?}",
            names(&ps)
        );
        let r = refused
            .iter()
            .find(|r| r.panel == "Side")
            .expect("Side refused");
        assert!(r.reason.contains("do not all face the same way"), "{r:?}");
    }

    /// The side's three bow points made one (a pole, as a stem often is): the welded copies of
    /// that point must still carry the chine. Identity: the chine is one 3D polyline shared by
    /// both panels, so its 3D length is the same on each, and the pair is compared.
    #[test]
    fn a_seam_through_a_welded_point_is_still_measured() {
        let mut s = pram_fixture(0.0, 0.0);
        let stem = Vec3::new(2.4, 0.2, 0.3);
        for p in &mut s.points {
            if (p.p.x - 2.4).abs() < 1e-9 && p.p.y > 1e-6 {
                p.p = stem;
                p.kind = crate::mesh::VertexKind::Corner;
            }
        }
        let (ps, mismatches, _) = measured(&s, 2);
        let side = flat(&ps, "Side");
        let bottom = flat(&ps, "Bottom");
        let chine = |p: &FlatPanel, mate: &str| {
            p.measures
                .seams
                .iter()
                .find(|m| m.name.contains(mate))
                .cloned()
                .unwrap_or_else(|| panic!("{}: {:?}", p.found.name, p.measures.seams))
        };
        let on_side = chine(side, "Bottom");
        let on_bottom = chine(bottom, "Side");
        assert!(on_bottom.length_3d_m > 2.0, "{on_bottom:?}");
        assert!(
            (on_side.length_3d_m - on_bottom.length_3d_m).abs() < 1e-9,
            "{on_side:?} {on_bottom:?}"
        );
        assert!(
            mismatches
                .iter()
                .any(|m| [&m.a, &m.b].contains(&&"Side".to_string())
                    && [&m.a, &m.b].contains(&&"Bottom".to_string())),
            "{mismatches:?}"
        );
    }

    /// Convergence, stated as such: on a curved side the outline moves less between finer levels.
    #[test]
    fn a_curved_side_reports_its_level_shift() {
        let s = pram_fixture(0.0, 0.08);
        let coarse = flat(&measured(&s, 1).0, "Side").measures.level_shift_m;
        let fine = flat(&measured(&s, 3).0, "Side").measures.level_shift_m;
        let (coarse, fine) = (coarse.expect("measured"), fine.expect("measured"));
        assert!(coarse.is_finite() && fine.is_finite());
        assert!(fine < coarse, "level 3→4 {fine} vs level 1→2 {coarse}");
    }

    /// Closed form: two unit squares with the same corners, one with the middle of an edge
    /// pushed out 10 mm, differ by 10 mm, wherever the second lies; the corners alone see none.
    #[test]
    fn the_outline_shift_sees_a_bulge_between_corners() {
        let a = [(0.0, 0.0), (0.5, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)];
        let (c, s) = (0.8f64, 0.6f64);
        let place = |p: (f64, f64)| (c * p.0 - s * p.1 + 3.0, s * p.0 + c * p.1 - 2.0);
        let b: Vec<(f64, f64)> = [(0.0, 0.0), (0.5, -0.01), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)]
            .into_iter()
            .map(place)
            .collect();
        let corners_a = [a[0], a[2], a[3], a[4]];
        let corners_b = [b[0], b[2], b[3], b[4]];
        assert!(rigid_misfit(&corners_a, &corners_b) < 1e-12);
        let shift = outline_shift(&a, &b, &corners_a, &corners_b);
        assert!((shift - 0.01).abs() < 1e-12, "{shift}");
    }

    /// Measured, a regression pin: the bowed side's outline moves 0.057 mm from level 3 to 4
    /// (the review's probe, largest distance between the outlines after the corner fit), its
    /// corners only 0.0099 mm.
    #[test]
    fn the_level_shift_is_the_outline_not_the_corners() {
        let s = pram_fixture(0.0, 0.08);
        let shift = flat(&measured(&s, 3).0, "Side")
            .measures
            .level_shift_m
            .expect("measured");
        assert!(shift > 4e-5 && shift < 8e-5, "{shift}");
    }

    #[test]
    fn the_edge_join_gap_is_kept() {
        let s = pram_fixture(0.0, 0.08);
        let (ps, _, _) = measured(&s, 2);
        let side = flat(&ps, "Side");
        let gap = crate::develop::unfold(&side.found.verts, &side.found.tris).max_gap_m;
        assert_eq!(side.measures.edge_join_gap_m, gap);
        assert!(gap > 0.0);
    }

    #[test]
    fn oriented_forward_right_outside_up() {
        let s = pram_fixture(0.0, 0.0);
        let (ps, _, _) = measured(&s, 3);
        let side = flat(&ps, "Side");
        for t in &side.found.tris {
            assert!(crate::flatten::signed_area(side.xy[t[0]], side.xy[t[1]], side.xy[t[2]]) > 0.0);
        }
        let right = (0..side.xy.len())
            .max_by(|&a, &b| side.xy[a].0.total_cmp(&side.xy[b].0))
            .unwrap();
        let fore = side
            .found
            .verts
            .iter()
            .map(|v| v.x)
            .fold(f64::MIN, f64::max);
        assert!((side.found.verts[right].x - fore).abs() < 1e-9);
        // Long axis along x.
        let span = |f: fn(&(f64, f64)) -> f64| {
            let v: Vec<f64> = side.xy.iter().map(f).collect();
            v.iter().cloned().fold(f64::MIN, f64::max) - v.iter().cloned().fold(f64::MAX, f64::min)
        };
        assert!(span(|p| p.0) > 2.0 * span(|p| p.1));
    }

    #[test]
    fn limits_decide_lies_flat() {
        let (v, t) = crate::flatten::shapes::cap(40f64.to_radians(), 6, 24);
        let loops = boundary_loops(&t);
        let mut ring = loops[0].clone();
        ring.push(ring[0]);
        let found = FoundPanel {
            name: "Cap".into(),
            layer: 0,
            copies: Copies::One,
            mesh_ids: (0..v.len()).collect(),
            welded: HashMap::new(),
            mirror: HashMap::new(),
            centreline: Vec::new(),
            verts: v,
            tris: t,
            seams: vec![Seam {
                mate: Mate::Open,
                verts: ring,
                across: false,
            }],
        };
        let wide = Limits {
            seam_drift_m: 1e3,
            rms_stretch: 1.0,
        };
        let r = measure_found(found.clone(), &[0.0], wide)
            .expect("flattens")
            .measures
            .rms_stretch;
        assert!(r > 0.001);
        let tight = Limits {
            rms_stretch: 0.99 * r,
            ..wide
        };
        let loose = Limits {
            rms_stretch: 1.01 * r,
            ..wide
        };
        assert!(
            !measure_found(found.clone(), &[0.0], tight)
                .unwrap()
                .lies_flat
        );
        assert!(measure_found(found, &[0.0], loose).unwrap().lies_flat);
    }

    fn hand_made(name: &str, mesh: (Vec<Vec3>, Vec<[usize; 3]>)) -> FoundPanel {
        let (v, t) = mesh;
        let mut ring = boundary_loops(&t)[0].clone();
        ring.push(ring[0]);
        FoundPanel {
            name: name.into(),
            layer: 0,
            copies: Copies::One,
            mesh_ids: (0..v.len()).collect(),
            welded: HashMap::new(),
            mirror: HashMap::new(),
            centreline: Vec::new(),
            verts: v,
            tris: t,
            seams: vec![Seam {
                mate: Mate::Open,
                verts: ring,
                across: false,
            }],
        }
    }

    fn named(xs: &[f64]) -> Vec<(String, f64)> {
        xs.iter()
            .enumerate()
            .map(|(i, &x)| (format!("St {}", i + 1), x))
            .collect()
    }

    /// Closed form: a cylinder faceted on its rulings flattens to a rectangle on which every
    /// cross-section is a straight line across, as far apart as the sections are.
    #[test]
    fn station_marks_on_a_cylinder_are_straight_and_spaced() {
        let mesh = crate::flatten::shapes::ruled_cylinder(0.5, 2.0, 24, 120f64.to_radians());
        let panel = measure_found(hand_made("Tube", mesh), &[], Limits::default()).unwrap();
        let marks = station_marks(&panel, &named(&[0.5, 1.0, 1.5]));
        assert_eq!(marks.len(), 3, "{marks:?}");
        let mut xs = Vec::new();
        for (name, pts) in &marks {
            assert!(pts.len() >= 2, "{name}");
            // Across the long axis (flat x), so every point has the same flat x.
            let x0 = pts[0].0;
            for p in pts {
                assert!((p.0 - x0).abs() < 1e-9, "{name}: {p:?} vs {x0}");
            }
            let span = pts.iter().map(|p| p.1).fold(f64::MIN, f64::max)
                - pts.iter().map(|p| p.1).fold(f64::MAX, f64::min);
            let width = 24.0 * 2.0 * 0.5 * (120f64.to_radians() / 48.0).sin();
            assert!((span - width).abs() < 1e-9, "{name}: {span} vs {width}");
            xs.push(x0);
        }
        assert_eq!(marks[0].0, "St 1");
        assert!((xs[1] - xs[0] - 0.5).abs() < 1e-9 && (xs[2] - xs[1] - 0.5).abs() < 1e-9);
    }

    fn texts(sheet: &crate::sheet::Sheet) -> Vec<String> {
        sheet.labels.iter().map(|l| l.text.clone()).collect()
    }

    #[test]
    fn a_panel_sheet_has_its_cut_marks_and_words() {
        use crate::sheet::PathKind;
        let s = pram_fixture(0.0, 0.0);
        let (ps, _, _) = measured(&s, 3);
        let side = flat(&ps, "Side");
        let stations = named(&STATIONS);
        let sheet = panel_sheet(side, &stations, Limits::default(), false);
        assert_eq!(sheet.name, "Side");
        let cuts: Vec<_> = sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Cut)
            .collect();
        assert_eq!(cuts.len(), 1);
        assert!(cuts[0].closed);
        let outline: std::collections::HashSet<(u64, u64)> = boundary_loops(&side.found.tris)[0]
            .iter()
            .map(|&i| (side.xy[i].0.to_bits(), side.xy[i].1.to_bits()))
            .collect();
        assert_eq!(cuts[0].points.len(), outline.len());
        assert!(cuts[0]
            .points
            .iter()
            .all(|p| outline.contains(&(p.0.to_bits(), p.1.to_bits()))));
        let marks = sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Mark)
            .count();
        // Four station lines, and a tick where each crosses the chine and the sheer.
        assert_eq!(marks, 4 + 8, "{marks}");
        let words = texts(&sheet);
        for w in [
            "Side",
            "Cut 2, one turned over",
            "Stitch to Bottom",
            "Stitch to Transoms 1",
            "Stitch to Transoms 2",
            "Open edge",
            "Ticks match from either end",
            "St 1",
            "St 4",
        ] {
            assert!(words.iter().any(|t| t == w), "no label {w:?} in {words:?}");
        }
        for n in [
            crate::NOT_VALIDATED_NOTE,
            "Developed on the modelled surface: no plywood thickness allowed",
            "Measured against the model; not a bending test of plywood.",
        ] {
            assert!(sheet.notes.iter().any(|t| t == n), "no note {n:?}");
        }
        assert!(sheet.notes.iter().any(|t| t.starts_with("Stretch: ")));
        assert!(sheet.notes.iter().any(|t| t.contains("edge to Bottom")));
        assert!(!sheet.notes[0].starts_with("Does not lie flat"));
    }

    #[test]
    fn a_joined_panel_says_cut_one_and_its_centreline_seam_is_labelled() {
        let s = pram_fixture(12.0, 0.0);
        let (ps, _, _) = measured(&s, 2);
        let words = texts(&panel_sheet(
            flat(&ps, "Bottom"),
            &[],
            Limits::default(),
            false,
        ));
        assert!(
            words.iter().any(|t| t == "Cut 2, one turned over"),
            "{words:?}"
        );
        assert!(
            words
                .iter()
                .any(|t| t == "Centreline: stitch to its mirror"),
            "{words:?}"
        );
        let s = pram_fixture(0.0, 0.0);
        let (ps, _, _) = measured(&s, 2);
        let words = texts(&panel_sheet(
            flat(&ps, "Bottom"),
            &[],
            Limits::default(),
            false,
        ));
        assert!(words.iter().any(|t| t == "Cut 1"), "{words:?}");
    }

    /// Closed form: the flat pram's transoms are 2 × 0.35 m and 2 × 0.45 m wide at the bottom
    /// (`pram_net`), and its bottom is 2.4 m long on the centreline.
    #[test]
    fn a_joined_panel_measures_its_seams_across_the_centreline_and_marks_it() {
        use crate::sheet::PathKind;
        let s = pram_fixture(0.0, 0.0);
        let (ps, _, _) = measured(&s, 2);
        let widths = |p: &FlatPanel, prefix: &str| -> Vec<f64> {
            let mut w: Vec<f64> = p
                .measures
                .seams
                .iter()
                .filter(|m| m.name.starts_with(prefix))
                .map(|m| m.length_3d_m)
                .collect();
            w.sort_by(f64::total_cmp);
            w
        };
        let bottom = flat(&ps, "Bottom");
        let w = widths(bottom, "edge to Transoms");
        assert_eq!(w.len(), 2, "{w:?}");
        assert!(
            (w[0] - 0.70).abs() < 1e-6 && (w[1] - 0.90).abs() < 1e-6,
            "{w:?}"
        );
        let mut t: Vec<f64> = ["Transoms 1", "Transoms 2"]
            .iter()
            .flat_map(|n| widths(flat(&ps, n), "edge to Bottom"))
            .collect();
        t.sort_by(f64::total_cmp);
        assert!(
            t.len() == 2 && (t[0] - 0.70).abs() < 1e-6 && (t[1] - 0.90).abs() < 1e-6,
            "{t:?}"
        );

        let sheet = panel_sheet(bottom, &[], Limits::default(), false);
        let words = texts(&sheet);
        assert_eq!(
            words.iter().filter(|t| *t == "Stitch to Side").count(),
            2,
            "both chines labelled: {words:?}"
        );
        assert!(words.iter().any(|t| t == "Centreline"), "{words:?}");
        let marks: Vec<&crate::sheet::SheetPath> = sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Mark)
            .collect();
        assert_eq!(marks.len(), 1, "the centreline, and no stations");
        let length: f64 = marks[0]
            .points
            .windows(2)
            .map(|w| (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1))
            .sum();
        assert!((length - 2.4).abs() < 1e-6, "{length}");
    }

    #[test]
    fn a_failing_panel_is_headed_so() {
        let mesh = crate::flatten::shapes::cap(40f64.to_radians(), 6, 24);
        let limits = Limits::default();
        let panel = measure_found(hand_made("Cap", mesh), &[0.0], limits).unwrap();
        assert!(!panel.lies_flat);
        let sheet = panel_sheet(&panel, &named(&[0.0]), limits, false);
        let m = &panel.measures;
        let worst = m.worst_seam().unwrap();
        let heading = format!(
            "Does not lie flat: must stretch {:.2} % (limit {:.1} %), seam drift {:.1} mm along the {} (limit {:.0} mm)",
            m.rms_stretch * 100.0,
            limits.rms_stretch * 100.0,
            worst.drift_m * 1000.0,
            worst.name,
            limits.seam_drift_m * 1000.0
        );
        assert_eq!(sheet.notes[0], heading);
        let all: String = sheet
            .notes
            .iter()
            .chain(sheet.labels.iter().map(|l| &l.text))
            .cloned()
            .collect::<Vec<_>>()
            .join("\n")
            .to_lowercase();
        assert!(!all.contains("developable"));
    }

    /// A pattern the flattening could not settle is not judged to lie flat, and its sheet and
    /// table row say how far it may still move.
    #[test]
    fn an_unsettled_pattern_is_noted_and_does_not_lie_flat() {
        let mut panel = flat_by_hand(
            "Plank",
            vec![(0.0, 0.0), (1.0, 0.0), (1.0, 0.2), (0.0, 0.2)],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        let limits = Limits::default();
        assert!(lies_flat(&panel.measures, limits));
        panel.measures.unsettled_m = Some(0.0005);
        assert!(!lies_flat(&panel.measures, limits));
        panel.lies_flat = false;
        let sheet = panel_sheet(&panel, &[], limits, false);
        assert!(
            sheet.notes[0].contains("the flattening stopped at its limit before it settled"),
            "{:?}",
            sheet.notes
        );
        assert!(
            sheet.notes.iter().any(|n| {
                n
                == "Flattening stopped at its iteration limit: points may still move about 0.50 mm"
            }),
            "{:?}",
            sheet.notes
        );
    }

    /// A flat panel made directly from its pattern, for the tests that only read the outline.
    fn flat_by_hand(name: &str, xy: Vec<(f64, f64)>, tris: Vec<[usize; 3]>) -> FlatPanel {
        let verts: Vec<Vec3> = xy.iter().map(|p| Vec3::new(p.0, p.1, 0.0)).collect();
        FlatPanel {
            found: FoundPanel {
                name: name.into(),
                layer: 0,
                copies: Copies::One,
                mesh_ids: (0..verts.len()).collect(),
                welded: HashMap::new(),
                mirror: HashMap::new(),
                centreline: Vec::new(),
                verts,
                tris,
                seams: Vec::new(),
            },
            xy,
            measures: Measures {
                seams: Vec::new(),
                rms_stretch: 0.0,
                max_stretch: 0.0,
                level_shift_m: None,
                edge_join_gap_m: 0.0,
                area_3d_m2: 0.0,
                area_flat_m2: 0.0,
                unsettled_m: None,
            },
            lies_flat: true,
        }
    }

    /// Two rows of points, `inner[k]` and `outer[k]`, joined into a strip of triangles.
    fn strip(inner: &[(f64, f64)], outer: &[(f64, f64)]) -> (Vec<(f64, f64)>, Vec<[usize; 3]>) {
        let n = inner.len();
        let mut xy = inner.to_vec();
        xy.extend_from_slice(outer);
        let mut t = Vec::new();
        for k in 0..n - 1 {
            t.push([k, n + k, n + k + 1]);
            t.push([k, n + k + 1, k + 1]);
        }
        (xy, t)
    }

    /// Closed form: a 2.4 m × 0.6 m rectangle stands 0 and 600 mm off its baseline everywhere.
    #[test]
    fn a_rectangle_has_constant_ordinates() {
        let lower: Vec<(f64, f64)> = (0..=8).map(|i| (0.3 * i as f64, 0.0)).collect();
        let upper: Vec<(f64, f64)> = (0..=8).map(|i| (0.3 * i as f64, 0.6)).collect();
        let (xy, t) = strip(&upper, &lower);
        let panel = flat_by_hand("Plank", xy, t);
        let off = panel_offsets(&panel, 0.25);
        assert_eq!(off.panel, "Plank");
        let along: Vec<f64> = off.rows.iter().map(|r| r.along_m).collect();
        let mut want: Vec<f64> = (0..10).map(|i| 0.25 * i as f64).collect();
        want.push(2.4);
        assert_eq!(along.len(), want.len(), "{along:?}");
        for (a, w) in along.iter().zip(&want) {
            assert!((a - w).abs() < 1e-12, "{along:?}");
        }
        for r in &off.rows {
            assert_eq!(r.ordinates_m.len(), 2, "{r:?}");
            assert!(
                r.ordinates_m[0].abs() < 1e-6 && (r.ordinates_m[1] - 0.6).abs() < 1e-6,
                "{r:?}"
            );
        }
    }

    /// Closed form: the development of a cone of half-angle 30° taken 180° round, between slant
    /// 0.3 and 1, is a sector of angle `S = 1440·2·asin(sin 30°·sin(π/2880))` (1440 facets, whose
    /// chords lie within `1 − cos(δ/2) ≈ 1.5e-7` m of the arcs). Its outline is the two arcs and
    /// the two end rulings; a row at `x` crosses them where the circles and lines meet `x`.
    #[test]
    fn the_cone_sector_ordinates_follow_its_arcs() {
        let n = 1440;
        let delta = 2.0 * (0.5 * (std::f64::consts::PI / n as f64 / 2.0).sin()).asin();
        let half = n as f64 * delta / 2.0;
        let ray = |r: f64| -> Vec<(f64, f64)> {
            (0..=n)
                .map(|k| {
                    let phi = -half + k as f64 * delta;
                    (r * phi.cos(), r * phi.sin())
                })
                .collect()
        };
        let (xy, t) = strip(&ray(0.3), &ray(1.0));
        let panel = flat_by_hand("Sector", xy, t);
        let off = panel_offsets(&panel, 0.25);
        let x0 = 0.3 * half.cos();
        let y0 = -half.sin();
        for row in &off.rows[1..4] {
            let x = x0 + row.along_m;
            let mut want = Vec::new();
            for r in [0.3f64, 1.0] {
                if x <= r {
                    let y = (r * r - x * x).sqrt();
                    if y.atan2(x) <= half {
                        want.extend([-y, y]);
                    }
                }
            }
            let r = x / half.cos();
            if (0.3..=1.0).contains(&r) {
                let y = x * half.tan();
                want.extend([-y, y]);
            }
            want.sort_by(f64::total_cmp);
            let got = &row.ordinates_m;
            assert_eq!(got.len(), want.len(), "x {x}: {got:?} vs {want:?}");
            for (g, w) in got.iter().zip(&want) {
                assert!((g - (w - y0)).abs() < 1e-6, "x {x}: {got:?} vs {want:?}");
            }
        }
    }

    #[test]
    fn csv_and_pdf_carry_every_row() {
        let s = pram_fixture(0.0, 0.0);
        let (ps, _, _) = measured(&s, 2);
        let all: Vec<PanelOffsets> = ps.iter().map(|p| panel_offsets(p, 0.25)).collect();
        let csv = offsets_csv(&all);
        let mut lines = csv.lines();
        assert_eq!(lines.next(), Some("panel,along_mm,crossing,ordinate_mm"));
        let cells: usize = all
            .iter()
            .flat_map(|o| o.rows.iter().map(|r| r.ordinates_m.len()))
            .sum();
        assert_eq!(csv.lines().count(), cells + 1);
        assert!(csv.lines().any(|l| l.starts_with("Transoms 1,0,1,")));
        let doc = offsets_pdf(&all, "Pram", false).expect("pdf");
        assert!(doc.bytes.starts_with(b"%PDF-"));
        assert!(doc.pages >= all.len());
        let imperial = offsets_pdf(&all, "Pram", true).expect("pdf");
        assert!(imperial.pages >= 2 * all.len());
        assert!(offsets_pdf(&[], "Pram", false).is_err());
    }

    #[test]
    fn a_name_with_a_comma_is_quoted_in_the_csv() {
        let off = PanelOffsets {
            panel: "Side, upper".into(),
            rows: vec![OffsetRow {
                along_m: 0.0,
                ordinates_m: vec![0.1234],
            }],
        };
        assert_eq!(
            offsets_csv(&[off]),
            "panel,along_mm,crossing,ordinate_mm\n\"Side, upper\",0,1,123\n"
        );
    }

    fn plank(length: f64, width: f64, copies: Copies) -> FlatPanel {
        let lower = [(0.0, 0.0), (length, 0.0)];
        let upper = [(0.0, width), (length, width)];
        let (xy, t) = strip(&upper, &lower);
        let mut p = flat_by_hand("Plank", xy, t);
        p.found.copies = copies;
        p
    }

    #[test]
    fn a_panel_shorter_than_a_sheet_takes_one() {
        let row = lay_out(&plank(2.0, 0.5, Copies::One), Stock::default());
        assert_eq!(row.sheets_each, Some(1));
        assert!(row.joints_m.is_empty());
        assert_eq!(row.note, None);
    }

    /// Arithmetic: the first sheet reaches 2.44 m and each further one 2.44 m less the scarf.
    #[test]
    fn butt_and_scarf_counts() {
        let butt = lay_out(&plank(5.0, 0.5, Copies::One), Stock::default());
        assert_eq!(butt.sheets_each, Some(3));
        assert_eq!(butt.joints_m.len(), 2);
        assert!((butt.joints_m[0] - 2.44).abs() < 1e-12 && (butt.joints_m[1] - 4.88).abs() < 1e-12);
        let scarfed = lay_out(
            &plank(5.0, 0.5, Copies::One),
            Stock {
                scarf_m: 0.2,
                ..Stock::default()
            },
        );
        assert_eq!(scarfed.sheets_each, Some(3));
        assert!((scarfed.joints_m[0] - 2.44).abs() < 1e-12);
        assert!((scarfed.joints_m[1] - 4.68).abs() < 1e-12);
        // Reference rectangles for three sheets and two joint lines.
        let paths = stock_paths(&scarfed, &plank(5.0, 0.5, Copies::One), Stock::default());
        assert_eq!(paths.len(), 5);
        assert!(paths
            .iter()
            .all(|p| p.kind == crate::sheet::PathKind::Reference));
        // Drawn within the pattern, so they add no paper to print.
        for p in &paths {
            for q in &p.points {
                assert!((-1e-12..=5.0 + 1e-12).contains(&q.0), "{q:?}");
                assert!((-1e-12..=0.5 + 1e-12).contains(&q.1), "{q:?}");
            }
        }
    }

    #[test]
    fn a_panel_wider_than_the_stock_is_reported() {
        let rows = [lay_out(&plank(2.0, 1.3, Copies::One), Stock::default())];
        assert_eq!(rows[0].sheets_each, None);
        assert_eq!(
            rows[0].note.as_deref(),
            Some("wider than the stock (1300 mm > 1220 mm): not laid out")
        );
        assert_eq!(total_sheets(&rows), 0);
        assert!(stock_paths(&rows[0], &plank(2.0, 1.3, Copies::One), Stock::default()).is_empty());
    }

    #[test]
    fn total_counts_copies() {
        let rows = [
            lay_out(&plank(2.0, 0.5, Copies::MirroredPair), Stock::default()),
            lay_out(&plank(4.0, 0.5, Copies::One), Stock::default()),
        ];
        assert_eq!(rows[0].copies, 2);
        assert_eq!(rows[1].sheets_each, Some(2));
        assert_eq!(total_sheets(&rows), 4);
    }

    #[test]
    fn panel_set_on_the_flat_pram() {
        let mut s = pram_fixture(0.0, 0.0);
        s.layers[2].developable = true;
        let set = panel_set(&s, &PanelOptions::default()).expect("a set");
        assert!(set.all_lie_flat(), "{:?} {:?}", set.rows, set.refused);
        let names: Vec<&str> = set.rows.iter().map(|r| r.panel.as_str()).collect();
        assert_eq!(names, ["Bottom", "Side", "Transoms 1", "Transoms 2"]);
        assert_eq!(set.sheets.len(), 4);
        assert_eq!(set.offsets.len(), 4);
        assert_eq!(set.rows[1].copies, 2);
        // Bottom 1 + Side 2 × 1 + two transoms 1 each.
        assert_eq!(set.total_sheets, 5);
        assert_eq!(set.title, s.settings.name);
        // The stock outline is drawn on each pattern for the eye.
        assert!(set.sheets[0]
            .paths
            .iter()
            .any(|p| p.kind == crate::sheet::PathKind::Reference));
    }

    /// A sheet count that leaves panels out says which, and why; a refused panel is named on
    /// every pattern of the set, so the file does not pass for the whole boat.
    #[test]
    fn panels_left_out_are_named_in_the_count_and_on_every_sheet() {
        let mut s = pram_fixture(0.0, 0.0);
        let fi = s.face_layer.iter().position(|&l| l == 1).unwrap() + 3;
        s.faces[fi].reverse();
        s.rebuild_edges();
        let opts = PanelOptions {
            stock: Stock {
                width_m: 0.5,
                ..Stock::default()
            },
            ..PanelOptions::default()
        };
        let set = panel_set(&s, &opts).expect("a set");
        assert_eq!(set.rows.len(), 1, "{:?}", set.rows);
        assert_eq!(set.total_sheets, 0);
        let line = set.sheet_count_line();
        assert!(
            line.starts_with("0 plywood sheets for 0 of 2 panels"),
            "{line}"
        );
        assert!(line.contains("Bottom (wider than the stock)"), "{line}");
        assert!(line.contains("Side (could not be laid flat)"), "{line}");
        assert!(
            set.sheets[0]
                .notes
                .iter()
                .any(|n| n.starts_with("Not in this set: Side: its faces do not all face")),
            "{:?}",
            set.sheets[0].notes
        );
        let whole = panel_set(&pram_fixture(0.0, 0.0), &PanelOptions::default()).expect("a set");
        assert_eq!(
            whole.sheet_count_line(),
            format!(
                "{} plywood sheets, end to end, no nesting: an upper bound",
                whole.total_sheets
            )
        );
    }

    /// Each pattern says how far its stitched edges differ from their mates' at the ticks; a
    /// mate that was not measured with it is said to be not compared.
    #[test]
    fn mating_mismatch_is_printed_on_each_pattern() {
        let s = pram_fixture(0.0, 0.08);
        let set = panel_set(&s, &PanelOptions::default()).expect("a set");
        let chine = set
            .mismatches
            .iter()
            .find(|m| m.a == "Bottom" && m.b == "Side")
            .expect("chine compared");
        assert!(chine.max_m > 0.001, "{chine:?}");
        let side = set.rows.iter().position(|r| r.panel == "Side").unwrap();
        let note = set.sheets[side]
            .notes
            .iter()
            .find(|n| n.starts_with("Against the mating edge"))
            .unwrap_or_else(|| panic!("{:?}", set.sheets[side].notes));
        assert!(
            note.contains(&format!("Bottom {:.1} mm", chine.max_m * 1000.0)),
            "{note}"
        );
        // The transoms are not in the default set (not marked developable).
        assert!(note.contains("Transoms 1 not compared"), "{note}");
    }

    #[test]
    fn panel_set_takes_the_developable_layers_by_default_and_names_on_request() {
        let s = pram_fixture(0.0, 0.0);
        let set = panel_set(&s, &PanelOptions::default()).expect("a set");
        let names: Vec<&str> = set.rows.iter().map(|r| r.panel.as_str()).collect();
        assert_eq!(names, ["Bottom", "Side"]);
        let opts = PanelOptions {
            layers: Some(vec!["Transoms".into()]),
            ..PanelOptions::default()
        };
        let set = panel_set(&s, &opts).expect("a set");
        assert_eq!(set.rows.len(), 2);
    }

    #[test]
    fn an_unknown_layer_lists_the_layers() {
        let s = pram_fixture(0.0, 0.0);
        let opts = PanelOptions {
            layers: Some(vec!["Keel".into()]),
            ..PanelOptions::default()
        };
        let e = panel_set(&s, &opts).unwrap_err();
        for w in ["Keel", "Bottom", "Side", "Transoms"] {
            assert!(e.contains(w), "{e}");
        }
        let mut s2 = s.clone();
        s2.layers.push(Layer {
            name: "Deck".into(),
            ..Layer::default()
        });
        let opts = PanelOptions {
            layers: Some(vec!["Deck".into()]),
            ..PanelOptions::default()
        };
        let e = panel_set(&s2, &opts).unwrap_err();
        assert!(e.contains("Deck") && e.contains("no faces"), "{e}");
    }

    #[test]
    fn a_failing_panel_makes_the_set_not_lie_flat() {
        let s = pram_fixture(0.0, 0.08);
        let opts = PanelOptions {
            limits: Limits {
                seam_drift_m: 0.002,
                rms_stretch: 1e-6,
            },
            ..PanelOptions::default()
        };
        let set = panel_set(&s, &opts).expect("a set");
        assert!(!set.all_lie_flat());
        let side = set.rows.iter().find(|r| r.panel == "Side").unwrap();
        assert!(!side.lies_flat);
        assert!(set.sheets[1].notes[0].starts_with("Does not lie flat"));
    }

    #[test]
    fn write_gives_pdf_svg_dxf() {
        use crate::sheet::PageSetup;
        use crate::templates::Format;
        let s = pram_fixture(0.0, 0.0);
        let set = panel_set(&s, &PanelOptions::default()).expect("a set");
        let setup = PageSetup::default();
        let pdf = write(&set, Format::PdfTiled, &setup).expect("pdf");
        assert!(pdf.bytes.starts_with(b"%PDF-"));
        assert!(pdf.pages.unwrap() > 1);
        let svg = String::from_utf8(write(&set, Format::Svg, &setup).unwrap().bytes).unwrap();
        for r in &set.rows {
            assert!(svg.contains(&r.panel), "{}", r.panel);
        }
        let dxf = String::from_utf8(write(&set, Format::Dxf, &setup).unwrap().bytes).unwrap();
        assert!(dxf.contains("CUT"));
    }

    /// Closed form: a planar panel lies flat with every edge its true length. The demo's transom
    /// is one 11-sided planar face whose subdivision has many obtuse triangles, where some
    /// edges' cotangent weights sum below zero.
    #[test]
    fn a_planar_transom_with_obtuse_triangles_flattens_exactly() {
        for symmetric in [false, true] {
            let s = crate::demo::demo_hull(symmetric);
            let (ps, refused) = find_panels(&s, SubdivisionOptions::default(), &[2]);
            assert!(refused.is_empty(), "{refused:?}");
            let p = &ps[0];
            assert!(p.verts.iter().all(|v| v.x.abs() < 1e-12), "not planar");
            let flat = crate::flatten::flatten(&p.verts, &p.tris).expect("flattens");
            let st = crate::flatten::strain(&p.verts, &p.tris, &flat.xy);
            assert!(st.max < 1e-9, "{st:?}");
        }
    }

    /// The pram as a user has it: written to a `.fef` and read back, its points rounded to the
    /// file's decimals. The transoms' straight sheer and bottom are many nearly collinear
    /// segments, which must not read as an outline crossing itself.
    #[test]
    fn a_pram_read_back_from_its_file_still_flattens() {
        for (d, b) in [(0.0, 0.0), (12.0, 0.0), (0.0, 0.08)] {
            let text = crate::fef::write(&pram_fixture(d, b));
            let s = crate::fef::parse(&text).expect("reads back");
            let (ps, _, refused) = measured(&s, 3);
            assert!(refused.is_empty(), "deadrise {d}, bow {b}: {refused:?}");
            assert_eq!(ps.len(), 4);
        }
    }

    /// A planar panel read back from a file is planar to the file's rounding: the loop must
    /// see that it has stopped moving, not run to its limit.
    #[test]
    fn a_planar_panel_read_back_from_its_file_settles_at_once() {
        let s = crate::fef::parse(&crate::fef::write(&pram_fixture(0.0, 0.0))).expect("reads");
        let opts = SubdivisionOptions {
            level: 3,
            ..SubdivisionOptions::default()
        };
        let (ps, _) = find_panels(&s, opts, &all(&s));
        for p in &ps {
            let f = crate::flatten::flatten(&p.verts, &p.tris).expect("flattens");
            assert!(
                f.unsettled_m.is_none() && f.iterations < 50,
                "{}: {} iterations, {:?}",
                p.name,
                f.iterations,
                f.unsettled_m
            );
        }
    }

    #[test]
    fn default_layers_prefers_developable() {
        let mut s = pram_fixture(0.0, 0.0);
        assert_eq!(default_layers(&s), [0, 1]);
        for l in &mut s.layers {
            l.developable = false;
        }
        assert_eq!(default_layers(&s), [0, 1, 2]);
    }
}
