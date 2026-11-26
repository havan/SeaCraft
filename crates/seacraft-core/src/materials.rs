//! The laminate and seam schedule a builder enters for a hull: glass per layer, tape, fillet and plywood
//! stock. The figures are the builder's own; nothing here carries a built-in glass or resin value.
//!
//! This module holds the types and their validation; the numbers they feed are measured elsewhere.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::panel_choices::{self, Choices};
use crate::panels::Stock;

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
            "`{field}` must be finite and more than zero, got {v}"
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
            return Err("`resin_per_glass` is required when a ply or tape is given".to_owned());
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
                return Err("`seams.tape.plies` must be at least 1, got 0".to_owned());
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

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

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
