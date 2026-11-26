//! The laminate and seam schedule a builder enters for a hull: glass per layer, tape, fillet and plywood
//! stock. The figures are the builder's own; nothing here carries a built-in glass or resin value.
//!
//! The types, their validation, and the estimate: layer areas, shell, glass and resin, and the seams'
//! tape and fillets, all measured on the refined half mesh.

use std::collections::BTreeMap;
use std::f64::consts::PI;

use serde::{Deserialize, Serialize};

use crate::i18n::{Key, Msg};
use crate::math::{polygon_area3, Vec3};
use crate::mesh::Surface;
use crate::panel_choices::{self, Choices};
use crate::panels::Stock;
use crate::subdiv::{self, SubdivMesh, SubdivisionOptions};

/// Which face of the shell a ply sits on. It does not change the mass.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    Outside,
    Inside,
}

/// One ply of glass cloth.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Ply {
    /// Glass mass per square metre of shell.
    pub g_m2: f64,
    pub side: Side,
}

/// Which faces of a seam carry tape.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TapeSides {
    Outside,
    Inside,
    Both,
}

impl TapeSides {
    /// How many faces of the seam are taped.
    pub fn count(self) -> f64 {
        match self {
            TapeSides::Outside | TapeSides::Inside => 1.0,
            TapeSides::Both => 2.0,
        }
    }
}

/// Tape laid over a seam.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Tape {
    pub width_mm: f64,
    /// Glass mass per square metre of tape, per ply.
    pub g_m2: f64,
    pub plies: u32,
    pub sides: TapeSides,
}

/// The cove filling the inside of a seam.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fillet {
    pub radius_mm: f64,
    pub density_kg_m3: f64,
}

/// The seam schedule; every part is optional.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Seams {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tape: Option<Tape>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fillet: Option<Fillet>,
}

/// The plywood sheets the panels are cut from.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlywoodStock {
    /// `LENGTHxWIDTH` in millimetres, such as `2440x1220`.
    pub stock_mm: String,
    #[serde(default)]
    pub scarf_mm: f64,
}

impl PlywoodStock {
    /// The stock in the units the panel layout takes, checked as `seacraft panels` checks it.
    pub fn stock(&self) -> Result<Stock, String> {
        let choices = Choices {
            stock_mm: self.stock_mm.clone(),
            scarf_mm: self.scarf_mm,
            ..Choices::default()
        };
        panel_choices::check(&choices)
            .map(|c| c.stock)
            .map_err(|e| e.to_string())
    }
}

/// A hull's laminate and seam schedule, as stored in the project file.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Materials {
    /// Resin mass per unit glass mass, for every ply and the tape.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub resin_per_glass: Option<f64>,
    /// Plies by layer name; a name applies to every layer of that name.
    #[serde(default)]
    pub layers: BTreeMap<String, Vec<Ply>>,
    #[serde(default)]
    pub seams: Seams,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub plywood: Option<PlywoodStock>,
}

fn positive(field: &str, v: f64) -> Result<(), String> {
    if v.is_finite() && v > 0.0 {
        Ok(())
    } else {
        Err(format!(
            "{field} must be finite and more than zero, got {v}"
        ))
    }
}

impl Materials {
    /// True when any ply or tape carries glass.
    pub fn has_glass(&self) -> bool {
        self.layers.values().any(|p| !p.is_empty()) || self.seams.tape.is_some()
    }

    /// Checks every number is finite and positive; a refusal names the field.
    pub fn validate(&self) -> Result<(), String> {
        if let Some(r) = self.resin_per_glass {
            positive("resin_per_glass", r)?;
        } else if self.has_glass() {
            return Err("resin_per_glass is required when a ply or tape is given".to_owned());
        }
        for (name, plies) in &self.layers {
            for (i, ply) in plies.iter().enumerate() {
                positive(&format!("layers.{name}[{i}].g_m2"), ply.g_m2)?;
            }
        }
        if let Some(t) = &self.seams.tape {
            positive("seams.tape.width_mm", t.width_mm)?;
            positive("seams.tape.g_m2", t.g_m2)?;
            if t.plies < 1 {
                return Err("seams.tape.plies must be at least 1, got 0".to_owned());
            }
        }
        if let Some(f) = &self.seams.fillet {
            positive("seams.fillet.radius_mm", f.radius_mm)?;
            positive("seams.fillet.density_kg_m3", f.density_kg_m3)?;
        }
        if let Some(p) = &self.plywood {
            p.stock().map_err(|e| format!("plywood: {e}"))?;
        }
        Ok(())
    }
}

/// One layer's measured area, shell, glass and resin.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayerRow {
    pub layer: String,
    pub area_m2: f64,
    pub thickness_mm: f64,
    pub density_kg_m3: f64,
    pub shell_kg: f64,
    /// Glass mass per square metre, summed over the plies.
    pub glass_g_m2: f64,
    pub glass_kg: f64,
    pub resin_kg: f64,
    pub total_kg: f64,
    pub centre: [f64; 3],
}

/// One group of seams: every seam edge between the same two layer names.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SeamRow {
    /// The two layer names, sorted, so `Bottom / Side` and `Side / Bottom` group together.
    pub between: [String; 2],
    /// Length over both sides of the hull: an edge off the centreline between symmetric layers
    /// counts twice (port and starboard), a centreline edge once.
    pub length_m: f64,
    pub tape_kg: f64,
    pub tape_resin_kg: f64,
    pub fillet_kg: f64,
    pub total_kg: f64,
    /// Centre of the seam edges by length.
    pub centre: [f64; 3],
    /// True when any edge of the group folds the hull inward (a spray rail, a reverse chine):
    /// those edges get tape and no inside fillet.
    pub concave: bool,
}

/// Mass and centre of everything counted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Total {
    pub mass_kg: f64,
    pub mass_t: f64,
    pub centre: [f64; 3],
}

/// What the schedule adds up to on a surface. Plywood is added by a later measurement.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Estimate {
    pub layers: Vec<LayerRow>,
    pub seams: Vec<SeamRow>,
    pub total: Total,
    /// Schedule names that match no layer of the surface.
    pub unmatched_layers: Vec<String>,
    pub warnings: Vec<Msg>,
    /// True when the schedule carries any glass.
    pub has_glass: bool,
}

/// Accumulates a mass-weighted centre.
#[derive(Clone, Default)]
struct Centroid {
    weight: f64,
    sum: Vec3,
}

impl Centroid {
    fn add(&mut self, weight: f64, at: Vec3) {
        self.weight += weight;
        self.sum += at * weight;
    }

    fn at(&self) -> Vec3 {
        if self.weight > 0.0 {
            self.sum * (1.0 / self.weight)
        } else {
            Vec3::default()
        }
    }

    fn centre(&self) -> [f64; 3] {
        let c = self.at();
        [c.x, c.y, c.z]
    }
}

/// Area of a face as `polygon_area3` takes it (the same as the exports and `plating_weights`), and
/// the centroid of its fan triangles weighted by their areas.
fn polygon_centroid(poly: &[Vec3]) -> (f64, Vec3) {
    let mut c = Centroid::default();
    for i in 1..poly.len().saturating_sub(1) {
        let tri = [poly[0], poly[i], poly[i + 1]];
        c.add(
            polygon_area3(&tri),
            (tri[0] + tri[1] + tri[2]) * (1.0 / 3.0),
        );
    }
    (polygon_area3(poly), c.at())
}

/// Weighs the shell, glass and resin of every layer, and the tape and fillets of every seam, on the
/// refined half mesh. A symmetric layer counts each face twice (the mirror at -y), except a face
/// lying in the centreline plane.
pub fn estimate(surface: &Surface, schedule: &Materials, opts: SubdivisionOptions) -> Estimate {
    let mesh = subdiv::evaluate(surface, opts);
    let mut area = vec![Centroid::default(); surface.layers.len()];
    for (face, &li) in mesh.faces.iter().zip(&mesh.face_layer) {
        let Some(acc) = area.get_mut(li) else {
            continue;
        };
        let poly: Vec<Vec3> = face.iter().map(|&i| mesh.verts[i]).collect();
        let (a, at) = polygon_centroid(&poly);
        acc.add(a, at);
        let on_centreline = poly.iter().all(|p| p.y.abs() < 1e-9);
        if surface.layers[li].symmetric && !on_centreline {
            acc.add(a, Vec3::new(at.x, -at.y, at.z));
        }
    }

    let resin = schedule.resin_per_glass.unwrap_or(0.0);
    let mut warnings = Vec::new();
    let mut rows = Vec::new();
    let mut total = Centroid::default();
    for (layer, acc) in surface.layers.iter().zip(&area) {
        let glass_g_m2: f64 = schedule
            .layers
            .get(&layer.name)
            .map_or(0.0, |p| p.iter().map(|p| p.g_m2).sum());
        let a = acc.weight;
        let shell_kg = a * layer.thickness * layer.density;
        let glass_kg = a * glass_g_m2 / 1000.0;
        let resin_kg = glass_kg * resin;
        let total_kg = shell_kg + glass_kg + resin_kg;
        total.add(total_kg, acc.at());
        if !(layer.density > 0.0 && layer.thickness > 0.0) {
            warnings.push(Msg::new(Key::MaterialsWarnNoPlating).arg("layer", &layer.name));
        }
        rows.push(LayerRow {
            layer: layer.name.clone(),
            area_m2: a,
            thickness_mm: layer.thickness * 1000.0,
            density_kg_m3: layer.density,
            shell_kg,
            glass_g_m2,
            glass_kg,
            resin_kg,
            total_kg,
            centre: acc.centre(),
        });
    }
    let seam_edges = find_seams(surface, &mesh, &mut warnings);
    let seams = weigh_seams(&seam_edges, schedule, &mut total);
    let unmatched_layers: Vec<String> = schedule
        .layers
        .keys()
        .filter(|k| !surface.layers.iter().any(|l| &l.name == *k))
        .cloned()
        .collect();
    for name in &unmatched_layers {
        warnings.push(Msg::new(Key::MaterialsWarnUnmatchedLayer).arg("name", name));
    }
    Estimate {
        layers: rows,
        seams,
        total: Total {
            mass_kg: total.weight,
            mass_t: total.weight / 1000.0,
            centre: total.centre(),
        },
        unmatched_layers,
        warnings,
        has_glass: schedule.has_glass(),
    }
}

/// Cross-section of a cove of radius `r` filling a corner of inside angle `theta` (radians):
/// the kite between the corner and the two tangent points, r² cot(θ/2), less the circular
/// sector, r² (π − θ)/2.
pub fn fillet_area(r: f64, theta: f64) -> f64 {
    r * r * (1.0 / (theta / 2.0).tan() - (PI - theta) / 2.0)
}

/// Within this of a straight fold (180°) a seam edge is flat: no seam.
const FLAT: f64 = 5.0 * PI / 180.0;

/// One seam edge of the refined half mesh.
struct SeamEdge {
    between: [String; 2],
    length: f64,
    mid: Vec3,
    /// Also counted at -y, the port copy.
    mirrored: bool,
    /// Inside angle, radians: below π convex, above π concave.
    theta: f64,
    convex: bool,
}

impl SeamEdge {
    /// Below 5° the two faces almost close on each other; above 355° (concave) likewise.
    fn closed(&self) -> bool {
        self.theta <= FLAT || self.theta >= 2.0 * PI - FLAT
    }
}

/// Two layer names, sorted, so either order groups together.
fn pair(a: &str, b: &str) -> [String; 2] {
    let mut p = [a.to_owned(), b.to_owned()];
    p.sort();
    p
}

fn label(p: &[String; 2]) -> String {
    format!("{} / {}", p[0], p[1])
}

/// True when `face` runs from `a` to `b` along one of its sides.
fn winds(face: &[usize], a: usize, b: usize) -> bool {
    (0..face.len()).any(|i| face[i] == a && face[(i + 1) % face.len()] == b)
}

/// Unit outward normal of a face as its winding gives it (zero when degenerate).
fn face_normal(mesh: &SubdivMesh, face: &[usize]) -> Vec3 {
    let mut acc = Vec3::ZERO;
    let p0 = mesh.verts[face[0]];
    for i in 1..face.len().saturating_sub(1) {
        acc += (mesh.verts[face[i]] - p0).cross(mesh.verts[face[i + 1]] - p0);
    }
    acc.norm()
}

/// Finds the seam edges of the refined half mesh: an edge creased or between two layers whose
/// fold is not flat, or a creased centreline edge of a symmetric layer folded against its mirror.
fn find_seams(surface: &Surface, mesh: &SubdivMesh, warnings: &mut Vec<Msg>) -> Vec<SeamEdge> {
    let normals: Vec<Vec3> = mesh.faces.iter().map(|f| face_normal(mesh, f)).collect();
    let layer = |fi: usize| surface.layers.get(mesh.face_layer[fi]);
    let mut warn = |key: Key, between: String| {
        let msg = Msg::new(key).arg("between", &between);
        if !warnings.contains(&msg) {
            warnings.push(msg);
        }
    };
    let (efaces, keys) = mesh.edge_faces();
    let mut seams = Vec::new();
    for (a, b) in keys {
        let crease = mesh.creases.contains(&(a, b));
        let (pa, pb) = (mesh.verts[a], mesh.verts[b]);
        // The edge as face `f` runs along it.
        let way = |f: usize| {
            if winds(&mesh.faces[f], a, b) {
                (a, b)
            } else {
                (b, a)
            }
        };
        let (f1, n2, l2, centreline) = match *efaces[&(a, b)].as_slice() {
            [f1, f2] => {
                let (Some(l1), Some(l2)) = (layer(f1), layer(f2)) else {
                    continue;
                };
                if !crease && mesh.face_layer[f1] == mesh.face_layer[f2] {
                    continue;
                }
                if way(f1) == way(f2) {
                    warn(
                        Key::MaterialsWarnSeamWinding,
                        label(&pair(&l1.name, &l2.name)),
                    );
                    continue;
                }
                (f1, normals[f2], l2, false)
            }
            [f] => {
                let Some(l) = layer(f) else { continue };
                if !(crease && l.symmetric && pa.y.abs() < 1e-9 && pb.y.abs() < 1e-9) {
                    continue;
                }
                let n = normals[f];
                (f, Vec3::new(n.x, -n.y, n.z), l, true)
            }
            _ => continue,
        };
        let Some(l1) = layer(f1) else { continue };
        let n1 = normals[f1];
        let (s, t) = way(f1);
        let along = mesh.verts[t] - mesh.verts[s];
        let length = along.length();
        if n1 == Vec3::ZERO || n2 == Vec3::ZERO || length == 0.0 {
            continue;
        }
        let phi = n1.dot(n2).clamp(-1.0, 1.0).acos();
        let convex = n1.cross(n2).dot(along) > 0.0;
        let theta = if convex { PI - phi } else { PI + phi };
        if (PI - FLAT..=PI + FLAT).contains(&theta) {
            continue;
        }
        seams.push(SeamEdge {
            between: pair(&l1.name, &l2.name),
            length,
            mid: (pa + pb) * 0.5,
            mirrored: !centreline && l1.symmetric && l2.symmetric,
            theta,
            convex,
        });
    }
    for edge in &seams {
        if edge.closed() {
            warn(Key::MaterialsWarnSeamClosed, label(&edge.between));
        }
    }
    seams
}

/// The inside angle of every seam edge between two layers, for the tests.
#[cfg(test)]
pub(crate) fn seam_angles(
    surface: &Surface,
    opts: SubdivisionOptions,
    between: [&str; 2],
) -> Vec<f64> {
    let mesh = subdiv::evaluate(surface, opts);
    find_seams(surface, &mesh, &mut Vec::new())
        .into_iter()
        .filter(|e| e.between == [between[0].to_owned(), between[1].to_owned()])
        .map(|e| e.theta)
        .collect()
}

#[derive(Default)]
struct SeamGroup {
    length: Centroid,
    fillet: Centroid,
    concave: bool,
}

/// Groups the seam edges by layer pair and weighs their tape and fillets; adds them to `total`.
fn weigh_seams(seams: &[SeamEdge], schedule: &Materials, total: &mut Centroid) -> Vec<SeamRow> {
    let fillet = schedule
        .seams
        .fillet
        .as_ref()
        .map(|f| (f.radius_mm / 1000.0, f.density_kg_m3));
    let mut groups: BTreeMap<&[String; 2], SeamGroup> = BTreeMap::new();
    for edge in seams {
        let g = groups.entry(&edge.between).or_default();
        let kg = match fillet {
            Some((r, density)) if edge.convex && !edge.closed() => {
                edge.length * fillet_area(r, edge.theta) * density
            }
            _ => 0.0,
        };
        let copies = if edge.mirrored {
            vec![edge.mid, Vec3::new(edge.mid.x, -edge.mid.y, edge.mid.z)]
        } else {
            vec![edge.mid]
        };
        for at in copies {
            g.length.add(edge.length, at);
            g.fillet.add(kg, at);
        }
        g.concave |= !edge.convex;
    }
    let tape_kg_per_m = schedule.seams.tape.as_ref().map_or(0.0, |t| {
        t.width_mm / 1000.0 * f64::from(t.plies) * t.g_m2 / 1000.0 * t.sides.count()
    });
    let resin = schedule.resin_per_glass.unwrap_or(0.0);
    groups
        .into_iter()
        .map(|(between, g)| {
            let tape_kg = g.length.weight * tape_kg_per_m;
            let tape_resin_kg = tape_kg * resin;
            let fillet_kg = g.fillet.weight;
            total.add(tape_kg + tape_resin_kg, g.length.at());
            total.add(fillet_kg, g.fillet.at());
            SeamRow {
                between: between.clone(),
                length_m: g.length.weight,
                tape_kg,
                tape_resin_kg,
                fillet_kg,
                total_kg: tape_kg + tape_resin_kg + fillet_kg,
                centre: g.length.centre(),
                concave: g.concave,
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::VertexKind;
    use crate::subdiv::SubdivisionMode;
    use serde_json::json;

    /// Open box x 0..2, y 0..1, z 0..1 on three layers: Bottom (z=0), Side (y=1), End (x=0, x=2); no
    /// face on y=0 or z=1; all layers symmetric; every edge creased, every point a corner.
    fn half_box() -> Surface {
        let mut s = Surface::new();
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (2.0, 0.0, 0.0),
            (2.0, 1.0, 0.0),
            (0.0, 1.0, 0.0),
            (0.0, 0.0, 1.0),
            (2.0, 0.0, 1.0),
            (2.0, 1.0, 1.0),
            (0.0, 1.0, 1.0),
        ] {
            s.add_point(Vec3::new(x, y, z));
        }
        s.add_face(vec![0, 3, 2, 1], 0); // bottom, outward is -z
        s.add_face(vec![2, 3, 7, 6], 1); // y = 1
        s.add_face(vec![1, 2, 6, 5], 2); // x = 2
        s.add_face(vec![3, 0, 4, 7], 2); // x = 0
        s.layers.clear();
        for (name, mm) in [("Bottom", 10.0), ("Side", 6.0), ("End", 12.0)] {
            s.layers.push(crate::mesh::Layer {
                name: name.into(),
                symmetric: true,
                thickness: mm / 1000.0,
                density: 500.0,
                ..Default::default()
            });
        }
        s.rebuild_edges();
        for e in s.edges.iter_mut() {
            e.crease = true;
        }
        for p in s.points.iter_mut() {
            p.kind = VertexKind::Corner;
        }
        s
    }

    fn l3() -> SubdivisionOptions {
        SubdivisionOptions {
            level: 3,
            mode: SubdivisionMode::Loop,
        }
    }

    fn close(a: f64, b: f64) {
        assert!((a - b).abs() < 1e-9, "{a} vs {b}");
    }

    fn close3(a: [f64; 3], b: [f64; 3]) {
        for i in 0..3 {
            close(a[i], b[i]);
        }
    }

    #[test]
    fn half_box_areas_and_centres_are_exact() {
        let e = estimate(&half_box(), &Materials::default(), l3());
        assert_eq!(e.layers.len(), 3);
        for (row, c) in e
            .layers
            .iter()
            .zip([[1.0, 0.0, 0.0], [1.0, 0.0, 0.5], [1.0, 0.0, 0.5]])
        {
            close(row.area_m2, 4.0);
            close3(row.centre, c);
        }
    }

    #[test]
    fn half_box_shell_masses() {
        let e = estimate(&half_box(), &Materials::default(), l3());
        close(e.layers[0].shell_kg, 20.0);
        close(e.layers[1].shell_kg, 12.0);
        close(e.layers[2].shell_kg, 24.0);
        close(e.total.mass_kg, 56.0);
        close(e.total.mass_t, 0.056);
        close3(e.total.centre, [1.0, 0.0, 18.0 / 56.0]);
        assert!(!e.has_glass);
    }

    #[test]
    fn plies_add_glass_and_resin() {
        let m = Materials {
            resin_per_glass: Some(1.2),
            layers: BTreeMap::from([(
                "Bottom".to_owned(),
                vec![
                    Ply {
                        g_m2: 200.0,
                        side: Side::Outside,
                    },
                    Ply {
                        g_m2: 200.0,
                        side: Side::Inside,
                    },
                ],
            )]),
            ..Default::default()
        };
        let e = estimate(&half_box(), &m, l3());
        close(e.layers[0].glass_g_m2, 400.0);
        close(e.layers[0].glass_kg, 1.6);
        close(e.layers[0].resin_kg, 1.92);
        close(e.layers[0].total_kg, 20.0 + 1.6 + 1.92);
        for row in &e.layers[1..] {
            assert_eq!((row.glass_kg, row.resin_kg), (0.0, 0.0));
        }
        assert!(e.has_glass);
        assert!(e.warnings.is_empty());
    }

    #[test]
    fn unmatched_layer_is_reported_not_dropped() {
        let m = Materials {
            resin_per_glass: Some(1.0),
            layers: BTreeMap::from([(
                "Keel".to_owned(),
                vec![Ply {
                    g_m2: 200.0,
                    side: Side::Outside,
                }],
            )]),
            ..Default::default()
        };
        let plain = estimate(&half_box(), &Materials::default(), l3());
        let e = estimate(&half_box(), &m, l3());
        assert_eq!(e.unmatched_layers, vec!["Keel".to_owned()]);
        assert_eq!(e.warnings.len(), 1);
        assert_eq!(e.warnings[0].key(), Some(Key::MaterialsWarnUnmatchedLayer));
        assert_eq!(e.layers, plain.layers);
    }

    #[test]
    fn no_schedule_still_weighs_the_shell() {
        let e = estimate(&half_box(), &Materials::default(), l3());
        assert!(!e.has_glass);
        assert!(e.unmatched_layers.is_empty() && e.warnings.is_empty());
        close(
            e.layers[0].shell_kg + e.layers[1].shell_kg + e.layers[2].shell_kg,
            56.0,
        );
    }

    #[test]
    fn hidden_layer_still_counts() {
        let mut s = half_box();
        s.layers[1].visible = false;
        let e = estimate(&s, &Materials::default(), l3());
        close(e.layers[1].area_m2, 4.0);
    }

    #[test]
    fn zero_plating_warns_and_weighs_nothing() {
        let mut s = half_box();
        s.layers[0].density = 0.0;
        let e = estimate(&s, &Materials::default(), l3());
        assert_eq!(e.layers[0].shell_kg, 0.0);
        assert_eq!(e.warnings.len(), 1);
        assert_eq!(e.warnings[0].key(), Some(Key::MaterialsWarnNoPlating));
    }

    #[test]
    fn a_face_on_the_centreline_counts_once() {
        let mut s = Surface::new();
        for (x, z) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            s.add_point(Vec3::new(x, 0.0, z));
        }
        s.add_face(vec![0, 1, 2, 3], 0);
        s.layers.truncate(1);
        s.layers[0].symmetric = true;
        let e = estimate(&s, &Materials::default(), l3());
        close(e.layers[0].area_m2, 1.0);
    }

    #[test]
    fn a_layer_that_is_not_symmetric_counts_once_off_the_centreline() {
        let mut s = half_box();
        s.layers[1].symmetric = false;
        let e = estimate(&s, &Materials::default(), l3());
        close(e.layers[1].area_m2, 2.0);
        close3(e.layers[1].centre, [1.0, 1.0, 0.5]);
        close(e.layers[1].shell_kg, 6.0);
    }

    #[test]
    fn a_layer_with_no_faces_has_no_area() {
        let mut s = half_box();
        s.layers.push(crate::mesh::Layer {
            name: "Deck".into(),
            symmetric: true,
            thickness: 0.01,
            density: 500.0,
            ..Default::default()
        });
        let e = estimate(&s, &Materials::default(), l3());
        assert_eq!(e.layers.len(), 4);
        assert_eq!(e.layers[3].area_m2, 0.0);
        assert_eq!(e.layers[3].total_kg, 0.0);
        close(e.total.mass_kg, 56.0);
        close3(e.total.centre, [1.0, 0.0, 18.0 / 56.0]);
    }

    fn tape_and_fillet() -> Materials {
        Materials {
            resin_per_glass: Some(1.0),
            seams: Seams {
                tape: Some(Tape {
                    width_mm: 75.0,
                    g_m2: 200.0,
                    plies: 1,
                    sides: TapeSides::Both,
                }),
                fillet: Some(Fillet {
                    radius_mm: 12.0,
                    density_kg_m3: 1000.0,
                }),
            },
            ..Default::default()
        }
    }

    fn group<'a>(e: &'a Estimate, a: &str, b: &str) -> &'a SeamRow {
        e.seams
            .iter()
            .find(|r| r.between == [a.to_owned(), b.to_owned()])
            .unwrap_or_else(|| panic!("no {a} / {b} seam in {:?}", e.seams))
    }

    fn names(e: &Estimate) -> Vec<String> {
        e.seams
            .iter()
            .map(|r| format!("{} / {}", r.between[0], r.between[1]))
            .collect()
    }

    /// One face per layer: an `n`-point surface whose every point is a corner, `creased` edges
    /// creased and the layers named, none symmetric unless `symmetric`.
    fn plates(
        points: &[(f64, f64, f64)],
        faces: &[(Vec<usize>, usize)],
        layers: &[&str],
        symmetric: bool,
        creased: &[(usize, usize)],
    ) -> Surface {
        let mut s = Surface::new();
        for &(x, y, z) in points {
            s.add_point(Vec3::new(x, y, z));
        }
        for (f, l) in faces {
            s.add_face(f.clone(), *l);
        }
        s.layers.clear();
        for name in layers {
            s.layers.push(crate::mesh::Layer {
                name: (*name).into(),
                symmetric,
                thickness: 0.01,
                density: 500.0,
                ..Default::default()
            });
        }
        s.rebuild_edges();
        for e in s.edges.iter_mut() {
            e.crease = creased.contains(&e.key());
        }
        for p in s.points.iter_mut() {
            p.kind = VertexKind::Corner;
        }
        s
    }

    #[test]
    fn fillet_area_closed_forms() {
        assert!((fillet_area(1.0, PI / 2.0) - (1.0 - PI / 4.0)).abs() < 1e-12);
        assert!((fillet_area(1.0, 2.0 * PI / 3.0) - (1.0 / 3f64.sqrt() - PI / 6.0)).abs() < 1e-12);
        assert!((fillet_area(0.012, PI / 2.0) - 0.012 * 0.012 * (1.0 - PI / 4.0)).abs() < 1e-15);
    }

    #[test]
    fn half_box_seams() {
        let e = estimate(&half_box(), &Materials::default(), l3());
        assert_eq!(
            names(&e),
            vec!["Bottom / End", "Bottom / Side", "End / Side"],
            "no Bottom / Bottom, no End / End: flat across the centreline"
        );
        for (row, c) in e
            .seams
            .iter()
            .zip([[1.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 0.0, 0.5]])
        {
            close(row.length_m, 4.0);
            close3(row.centre, c);
            assert!(!row.concave);
            assert_eq!(row.total_kg, 0.0);
        }
        for pair in [["Bottom", "End"], ["Bottom", "Side"], ["End", "Side"]] {
            for t in seam_angles(&half_box(), l3(), pair) {
                close(t, PI / 2.0);
            }
        }
        assert!(e.warnings.is_empty(), "{:?}", e.warnings);
        close(e.total.mass_kg, 56.0);
    }

    #[test]
    fn half_box_tape_and_fillet() {
        let e = estimate(&half_box(), &tape_and_fillet(), l3());
        assert_eq!(e.seams.len(), 3);
        let fillet = 4.0 * 0.012 * 0.012 * (1.0 - PI / 4.0) * 1000.0;
        for row in &e.seams {
            close(row.tape_kg, 0.12);
            close(row.tape_resin_kg, 0.12);
            close(row.fillet_kg, fillet);
            close(row.total_kg, 0.24 + fillet);
        }
        let seams = 3.0 * (0.24 + fillet);
        close(e.total.mass_kg, 56.0 + seams);
        // Seam centres: two at z = 0, one at z = 0.5, all at x = 1 and y = 0.
        let z = (18.0 + seams / 3.0 * 0.5) / (56.0 + seams);
        close3(e.total.centre, [1.0, 0.0, z]);
        assert!(e.warnings.is_empty(), "{:?}", e.warnings);
    }

    /// Half a V bottom: from the keel (x, 0, 0) to the chine (x, 1, 0.5), x from 0 to 2.
    fn half_v(crease_keel: bool) -> Surface {
        plates(
            &[
                (0.0, 0.0, 0.0),
                (2.0, 0.0, 0.0),
                (2.0, 1.0, 0.5),
                (0.0, 1.0, 0.5),
            ],
            &[(vec![0, 3, 2, 1], 0)],
            &["Bottom"],
            true,
            if crease_keel { &[(0, 1)] } else { &[] },
        )
    }

    #[test]
    fn v_bottom_keel_is_one_seam_counted_once() {
        let s = half_v(true);
        let e = estimate(&s, &tape_and_fillet(), l3());
        assert_eq!(names(&e), vec!["Bottom / Bottom"]);
        let row = group(&e, "Bottom", "Bottom");
        close(row.length_m, 2.0);
        close3(row.centre, [1.0, 0.0, 0.0]);
        assert!(!row.concave);
        let theta = PI - 2.0 * 0.5f64.atan();
        let angles = seam_angles(&s, l3(), ["Bottom", "Bottom"]);
        assert!(!angles.is_empty());
        for t in angles {
            close(t, theta);
        }
        close(row.tape_kg, 2.0 * 0.075 * 0.2 * 2.0);
        close(row.fillet_kg, 2.0 * fillet_area(0.012, theta) * 1000.0);
    }

    #[test]
    fn flat_bottom_across_centreline_has_no_keel_seam() {
        let e = estimate(&half_box(), &tape_and_fillet(), l3());
        assert!(e
            .seams
            .iter()
            .all(|r| r.between != ["Bottom".to_owned(), "Bottom".to_owned()]));
        assert!(e
            .seams
            .iter()
            .all(|r| r.between != ["End".to_owned(), "End".to_owned()]));
    }

    #[test]
    fn colour_only_boundary_is_not_a_seam() {
        let s = plates(
            &[
                (0.0, 0.0, 0.0),
                (1.0, 0.0, 0.0),
                (2.0, 0.0, 0.0),
                (2.0, 1.0, 0.0),
                (1.0, 1.0, 0.0),
                (0.0, 1.0, 0.0),
            ],
            &[(vec![0, 5, 4, 1], 0), (vec![1, 4, 3, 2], 1)],
            &["Aft", "Fore"],
            true,
            &[],
        );
        let e = estimate(&s, &tape_and_fillet(), l3());
        assert!(e.seams.is_empty(), "{:?}", e.seams);
        assert!(e.warnings.is_empty(), "{:?}", e.warnings);
    }

    /// Face A in z = 0, y from 0 to 1 (outward -z); face B in y = 1 from z = 0 to `z_b`
    /// at y = `y_b` (outward as wound); they share the creased edge (0, 1, 0)-(2, 1, 0).
    fn fold(b: [(f64, f64, f64); 2]) -> Surface {
        plates(
            &[
                (0.0, 0.0, 0.0),
                (2.0, 0.0, 0.0),
                (2.0, 1.0, 0.0),
                (0.0, 1.0, 0.0),
                b[0],
                b[1],
            ],
            &[(vec![0, 3, 2, 1], 0), (vec![2, 3, 5, 4], 0)],
            &["Hull"],
            false,
            &[(2, 3)],
        )
    }

    #[test]
    fn concave_crease_gets_tape_but_no_fillet() {
        let s = fold([(2.0, 1.0, -1.0), (0.0, 1.0, -1.0)]);
        let e = estimate(&s, &tape_and_fillet(), l3());
        assert_eq!(names(&e), vec!["Hull / Hull"]);
        let row = &e.seams[0];
        assert!(row.concave);
        close(row.length_m, 2.0);
        close3(row.centre, [1.0, 1.0, 0.0]);
        assert_eq!(row.fillet_kg, 0.0);
        assert!(row.tape_kg > 0.0);
        close(row.tape_kg, 2.0 * 0.075 * 0.2 * 2.0);
        for t in seam_angles(&s, l3(), ["Hull", "Hull"]) {
            close(t, 1.5 * PI);
        }
        assert!(e.warnings.is_empty(), "{:?}", e.warnings);
    }

    #[test]
    fn a_closed_seam_warns_and_has_no_fillet() {
        // B folds back over A to (y = 0, z = 0.05): an inside angle of atan(0.05), under 5°.
        let s = fold([(2.0, 0.0, 0.05), (0.0, 0.0, 0.05)]);
        let e = estimate(&s, &tape_and_fillet(), l3());
        assert_eq!(names(&e), vec!["Hull / Hull"]);
        let row = &e.seams[0];
        assert!(!row.concave);
        assert_eq!(row.fillet_kg, 0.0);
        close(row.tape_kg, 2.0 * 0.075 * 0.2 * 2.0);
        for t in seam_angles(&s, l3(), ["Hull", "Hull"]) {
            close(t, 0.05f64.atan());
        }
        assert_eq!(e.warnings.len(), 1, "{:?}", e.warnings);
        assert_eq!(e.warnings[0].key(), Some(Key::MaterialsWarnSeamClosed));
    }

    #[test]
    fn two_faces_winding_an_edge_the_same_way_warn_and_are_not_a_seam() {
        let mut s = fold([(2.0, 1.0, -1.0), (0.0, 1.0, -1.0)]);
        s.faces[1].reverse();
        let e = estimate(&s, &tape_and_fillet(), l3());
        assert!(e.seams.is_empty(), "{:?}", e.seams);
        assert_eq!(e.warnings.len(), 1, "{:?}", e.warnings);
        assert_eq!(e.warnings[0].key(), Some(Key::MaterialsWarnSeamWinding));
    }

    #[test]
    fn a_seam_with_a_layer_that_is_not_symmetric_counts_once() {
        let mut s = half_box();
        s.layers[1].symmetric = false;
        let e = estimate(&s, &Materials::default(), l3());
        let row = group(&e, "Bottom", "Side");
        close(row.length_m, 2.0);
        close3(row.centre, [1.0, 1.0, 0.0]);
        close(group(&e, "Bottom", "End").length_m, 4.0);
        close(group(&e, "End", "Side").length_m, 2.0);
    }

    fn ply(g: f64) -> Ply {
        Ply {
            g_m2: g,
            side: Side::Outside,
        }
    }

    fn with_ply(g: f64) -> Materials {
        Materials {
            resin_per_glass: Some(1.0),
            layers: BTreeMap::from([("Bottom".to_owned(), vec![ply(g)])]),
            ..Default::default()
        }
    }

    #[test]
    fn schedule_round_trips_through_json() {
        let v = json!({
            "resin_per_glass": 1.0,
            "layers": {
                "Bottom": [{"g_m2": 200.0, "side": "outside"}, {"g_m2": 200.0, "side": "inside"}],
                "Side": [{"g_m2": 200.0, "side": "outside"}]
            },
            "seams": {
                "tape": {"width_mm": 75.0, "g_m2": 200.0, "plies": 1, "sides": "both"},
                "fillet": {"radius_mm": 12.0, "density_kg_m3": 1000.0}
            },
            "plywood": {"stock_mm": "2440x1220", "scarf_mm": 0.0}
        });
        let m: Materials = serde_json::from_value(v.clone()).unwrap();
        m.validate().unwrap();
        assert_eq!(m.layers["Bottom"].len(), 2);
        assert_eq!(m.seams.tape.as_ref().unwrap().sides.count(), 2.0);
        assert_eq!(serde_json::to_value(&m).unwrap(), v);
    }

    #[test]
    fn resin_ratio_required_with_any_ply_or_tape() {
        let mut m = with_ply(200.0);
        m.resin_per_glass = None;
        assert!(m.validate().unwrap_err().contains("resin_per_glass"));
        let t = Materials {
            seams: Seams {
                tape: Some(Tape {
                    width_mm: 75.0,
                    g_m2: 200.0,
                    plies: 1,
                    sides: TapeSides::Inside,
                }),
                fillet: None,
            },
            ..Default::default()
        };
        assert!(t.validate().unwrap_err().contains("resin_per_glass"));
        assert!(Materials::default().validate().is_ok());
    }

    #[test]
    fn refusals_name_the_field() {
        let e = with_ply(0.0).validate().unwrap_err();
        assert!(e.contains("layers.Bottom[0].g_m2"), "{e}");
        let mut m = with_ply(200.0);
        m.seams.tape = Some(Tape {
            width_mm: 75.0,
            g_m2: 200.0,
            plies: 0,
            sides: TapeSides::Both,
        });
        assert!(m.validate().unwrap_err().contains("seams.tape.plies"));
        let mut m = with_ply(200.0);
        m.seams.fillet = Some(Fillet {
            radius_mm: f64::NAN,
            density_kg_m3: 1000.0,
        });
        assert!(m.validate().unwrap_err().contains("seams.fillet.radius_mm"));
        let mut m = with_ply(200.0);
        m.plywood = Some(PlywoodStock {
            stock_mm: "2440x".into(),
            scarf_mm: 0.0,
        });
        let e = m.validate().unwrap_err();
        assert!(e.contains("stock_mm") && e.contains("2440x"), "{e}");
    }

    #[test]
    fn zero_scarf_is_valid() {
        let p = PlywoodStock {
            stock_mm: "2440x1220".into(),
            scarf_mm: 0.0,
        };
        assert_eq!(p.stock(), Ok(Stock::default()));
    }
}
