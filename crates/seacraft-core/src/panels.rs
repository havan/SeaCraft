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
use crate::subdiv::{evaluate, SubdivisionOptions};

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
    pub seams: Vec<Seam>,
}

impl FoundPanel {
    /// The local vertex for a half-mesh id.
    pub fn local(&self, mesh_id: usize) -> Option<usize> {
        self.mesh_ids.iter().position(|&m| m == mesh_id)
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
        let loops = boundary_loops(&tris_half);
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
                        seams.push(Seam { mate, verts });
                        i = j;
                        if i == s0 {
                            break;
                        }
                    }
                }
            }
        }
        if joined {
            seams.retain(|s| s.mate != Mate::Centreline);
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
        if joined {
            let mut mirror: HashMap<usize, usize> = HashMap::new();
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
        for (old, &new) in map.iter().enumerate() {
            if new != usize::MAX && mesh_ids[new] == usize::MAX {
                mesh_ids[new] = ids[old];
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
            seams,
        });
    }
    (panels, refused)
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
