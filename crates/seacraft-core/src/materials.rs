//! The laminate and seam schedule a builder enters for a hull: glass per layer, tape, fillet and plywood
//! stock. The figures are the builder's own; nothing here carries a built-in glass or resin value.
//!
//! This module holds the types and their validation; the numbers they feed are measured elsewhere.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::i18n::{Key, Msg};
use crate::math::{polygon_area3, Vec3};
use crate::mesh::Surface;
use crate::panel_choices::{self, Choices};
use crate::panels::Stock;
use crate::subdiv::{self, SubdivisionOptions};

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

/// Mass and centre of everything counted.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Total {
    pub mass_kg: f64,
    pub mass_t: f64,
    pub centre: [f64; 3],
}

/// What the schedule adds up to on a surface. Seams and plywood are added by later measurements.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct Estimate {
    pub layers: Vec<LayerRow>,
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

/// Weighs the shell, glass and resin of every layer on the refined half mesh. A symmetric layer
/// counts each face twice (the mirror at -y), except a face lying in the centreline plane.
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
