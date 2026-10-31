//! Project sidecar: design data that FREE!ship's `.fef` has no place for.
//!
//! A model `hull.fef` keeps its loading conditions, compartments, damage
//! cases, superstructure windage and report settings in `hull.seacraft.json`
//! next to it, so the `.fef` stays readable by FREE!ship. The file is JSON
//! with a `format` tag and an integer `version`; a reader refuses a newer
//! version instead of dropping what it does not understand, and keeps unknown
//! top-level keys so a round trip through an older SeaCraft loses nothing.
//!
//! ```json
//! { "format": "seacraft-project", "version": 1,
//!   "loading_conditions": [ ... ], "compartments": [ ... ],
//!   "damage_cases": [ ... ], "windage": [ ... ], "report": { ... },
//!   "requirements": { ... } }
//! ```

use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::damage::{Compartment, CompartmentSpec, DamageCase};
use crate::loading::LoadingCondition;
use crate::report::ReportOptions;
use crate::seastate::WaveSpectrum;
use crate::weather::WindageItem;

/// The `format` tag of a project file.
pub const PROJECT_FORMAT: &str = "seacraft-project";
/// The newest project file version this build reads and the one it writes.
pub const PROJECT_VERSION: u32 = 1;

/// Everything SeaCraft stores beside a model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Project {
    pub format: String,
    pub version: u32,
    #[serde(default)]
    pub loading_conditions: Vec<LoadingCondition>,
    /// Damageable spaces, shared by every damage case (referred to by name).
    #[serde(default)]
    pub compartments: Vec<Compartment>,
    #[serde(default)]
    pub damage_cases: Vec<DamageCase>,
    /// Superstructure and deck windage, applied to every loading condition in
    /// addition to the condition's own `windage_extra`.
    #[serde(default)]
    pub windage: Vec<WindageItem>,
    #[serde(default)]
    pub report: ReportSettings,
    /// The design brief the hull was created from: targets the GUI shows
    /// measured values against. Targets never move geometry.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requirements: Option<Requirements>,
    /// The vertical centre of gravity above the keel (m) the user set for
    /// the bare-hull stability (GM, GZ cross curves). `None` until set: a
    /// model carries no KG of its own, and none is invented for it.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kg: Option<f64>,
    /// Keys written by a newer SeaCraft, kept for the round trip.
    #[serde(flatten)]
    pub extra: serde_json::Map<String, serde_json::Value>,
}

impl Default for Project {
    fn default() -> Self {
        Self {
            format: PROJECT_FORMAT.to_owned(),
            version: PROJECT_VERSION,
            loading_conditions: Vec::new(),
            compartments: Vec::new(),
            damage_cases: Vec::new(),
            windage: Vec::new(),
            report: ReportSettings::default(),
            requirements: None,
            kg: None,
            extra: serde_json::Map::new(),
        }
    }
}

/// Design requirements: the principal dimensions and targets a design was
/// started from. Every field is optional; a `None` is "no target". Lengths in
/// metres, displacement in tonnes, `lcb_frac` as a fraction of `LWL` from its
/// aft end. `draft`, `depth` and `kg` are all measured from the keel — the
/// model's lowest hydrostatic point (a fin keel's tip on a yacht) — so the
/// depth exceeds the draft.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct Requirements {
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub name: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lwl: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bwl: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub depth: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub displacement_t: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cb: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcb_frac: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_speed_kn: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kg: Option<f64>,
}

impl Requirements {
    /// Every number given is finite and positive (`lcb_frac` in `0..1`).
    pub fn validate(&self) -> Result<(), String> {
        let fields = [
            ("lwl", self.lwl),
            ("bwl", self.bwl),
            ("draft", self.draft),
            ("depth", self.depth),
            ("displacement_t", self.displacement_t),
            ("cb", self.cb),
            ("design_speed_kn", self.design_speed_kn),
            ("kg", self.kg),
        ];
        for (name, value) in fields {
            if let Some(v) = value {
                if !(v.is_finite() && v > 0.0) {
                    return Err(format!("requirements: `{name}` must be > 0, got {v}"));
                }
            }
        }
        if let Some(v) = self.lcb_frac {
            if !(v.is_finite() && v > 0.0 && v < 1.0) {
                return Err(format!(
                    "requirements: `lcb_frac` must be between 0 and 1, got {v}"
                ));
            }
        }
        Ok(())
    }
}

/// Seakeeping configuration for the design report.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ReportSeakeeping {
    pub speed_kn: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sea: Option<WaveSpectrum>,
}

/// Saved report choices; `None` fields keep [`ReportOptions::from_surface`]'s default.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct ReportSettings {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub draft: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kg: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub lcg_x: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub heels: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub speeds_kn: Option<Vec<f64>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub level: Option<u8>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_resistance: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_planing: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_conditions: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub damage_cases: Option<Vec<String>>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub include_weather: Option<bool>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub seakeeping: Option<ReportSeakeeping>,
}

impl ReportSettings {
    /// True when no custom report settings are specified.
    pub fn is_empty(&self) -> bool {
        *self == Self::default()
    }

    /// Overlay the saved choices on `base`.
    pub fn apply(&self, mut base: ReportOptions) -> ReportOptions {
        if let Some(v) = self.draft {
            base.draft = v;
        }
        if let Some(v) = self.kg {
            base.kg = v;
        }
        if self.lcg_x.is_some() {
            base.lcg_x = self.lcg_x;
        }
        if let Some(v) = &self.heels {
            base.heels = v.clone();
        }
        if let Some(v) = &self.speeds_kn {
            base.speeds_kn = v.clone();
        }
        if let Some(v) = self.level {
            base.level = v;
        }
        if let Some(v) = self.include_resistance {
            base.include_resistance = v;
        }
        if let Some(v) = self.include_planing {
            base.include_planing = v;
        }
        base
    }
}

/// `hull.fef` → `hull.seacraft.json`, in the same folder.
pub fn sidecar_path(model: &Path) -> PathBuf {
    model.with_extension("seacraft.json")
}

impl Project {
    /// True when there is nothing worth writing.
    pub fn is_empty(&self) -> bool {
        self.loading_conditions.is_empty()
            && self.compartments.is_empty()
            && self.damage_cases.is_empty()
            && self.windage.is_empty()
            && self.report.is_empty()
            && self.requirements.is_none()
            && self.kg.is_none()
            && self.extra.is_empty()
    }

    /// Parse a project file's text, refusing other formats and newer versions.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("not JSON: {e}"))?;
        match value.get("format").and_then(|f| f.as_str()) {
            Some(PROJECT_FORMAT) => {}
            Some(other) => return Err(format!("format is `{other}`, not `{PROJECT_FORMAT}`")),
            None => return Err(format!("no `format` field (expected `{PROJECT_FORMAT}`)")),
        }
        let version = value.get("version").and_then(|v| v.as_u64()).unwrap_or(0);
        if version == 0 || version > PROJECT_VERSION as u64 {
            return Err(format!(
                "project version {version} is not supported (this build reads 1..={PROJECT_VERSION})"
            ));
        }
        let project: Project = serde_json::from_value(value).map_err(|e| e.to_string())?;
        project.validate()?;
        Ok(project)
    }

    /// Pretty JSON, always tagged with this build's format and version.
    pub fn to_json(&self) -> String {
        let mut out = self.clone();
        out.format = PROJECT_FORMAT.to_owned();
        out.version = PROJECT_VERSION;
        let mut text = serde_json::to_string_pretty(&out).expect("project serialises");
        text.push('\n');
        text
    }

    /// Read a project file.
    pub fn load(path: &Path) -> Result<Self, String> {
        let text = std::fs::read_to_string(path)
            .map_err(|e| format!("cannot read {}: {e}", path.display()))?;
        Self::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// The sidecar of `model`, or `None` when there is none.
    pub fn load_for_model(model: &Path) -> Result<Option<Self>, String> {
        let path = sidecar_path(model);
        if !path.exists() {
            return Ok(None);
        }
        Self::load(&path).map(Some)
    }

    /// Write the file through a temporary sibling and a rename, so a failed
    /// write never leaves half a project behind.
    pub fn save(&self, path: &Path) -> Result<(), String> {
        crate::io::write_atomic(path, self.to_json())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))
    }

    /// Check names are unique and every entry is valid on its own.
    pub fn validate(&self) -> Result<(), String> {
        unique(
            "loading condition",
            self.loading_conditions.iter().map(|c| &c.name),
        )?;
        unique("compartment", self.compartments.iter().map(|c| &c.name))?;
        unique("damage case", self.damage_cases.iter().map(|c| &c.name))?;
        if let Some(kg) = self.kg {
            if !(kg.is_finite() && kg >= 0.0) {
                return Err(format!("`kg` must be a finite height >= 0, got {kg}"));
            }
        }
        for c in &self.loading_conditions {
            crate::loading::validate(c)
                .map_err(|e| format!("loading condition `{}`: {e}", c.name))?;
        }
        for c in &self.compartments {
            crate::damage::validate_compartment(c)?;
        }
        for c in &self.damage_cases {
            let err = |e: String| format!("damage case `{}`: {e}", c.name);
            if let Some(ref name) = c.loading_condition_name {
                self.loading_condition(Some(name)).map_err(err)?;
            }
            crate::damage::validate_entries(c).map_err(err)?;
            // A compartment named in a case is a project compartment or a
            // tank of the case's loading condition.
            let resolved = self.resolve_damage_case(c)?;
            let tanks = resolved.loading_condition.as_ref().map(|lc| &lc.tanks);
            for spec in &resolved.compartments {
                if let CompartmentSpec::Named(name) = spec {
                    if !tanks.is_some_and(|t| t.iter().any(|t| t.name == *name)) {
                        return Err(err(format!(
                            "no compartment or loading-condition tank named `{name}`"
                        )));
                    }
                }
            }
        }
        for w in &self.windage {
            if !(w.area_m2.is_finite() && w.area_m2 >= 0.0 && w.z_centroid.is_finite()) {
                return Err(format!(
                    "windage `{}`: area must be >= 0 and z finite",
                    w.name
                ));
            }
        }
        if let Some(r) = &self.requirements {
            r.validate()?;
        }
        if let Some(sk) = &self.report.seakeeping {
            if let Some(sea) = &sk.sea {
                sea.validate()
                    .map_err(|e| format!("report seakeeping sea: {e}"))?;
            }
        }
        Ok(())
    }

    /// A loading condition by name, or the first one when `name` is `None`.
    pub fn loading_condition(&self, name: Option<&str>) -> Result<&LoadingCondition, String> {
        pick("loading condition", &self.loading_conditions, name, |c| {
            &c.name
        })
    }

    /// A damage case by name, or the first one when `name` is `None`.
    pub fn damage_case(&self, name: Option<&str>) -> Result<&DamageCase, String> {
        pick("damage case", &self.damage_cases, name, |c| &c.name)
    }

    /// `cond` with the project's windage added in front of its own.
    pub fn with_windage(&self, cond: &LoadingCondition) -> LoadingCondition {
        let mut out = cond.clone();
        out.windage_extra = self
            .windage
            .iter()
            .chain(&cond.windage_extra)
            .cloned()
            .collect();
        out
    }

    /// `case` with compartments named after a project compartment made inline,
    /// and `loading_condition_name` resolved to a loading condition with windage.
    pub fn resolve_damage_case(&self, case: &DamageCase) -> Result<DamageCase, String> {
        let mut out = case.clone();
        for spec in &mut out.compartments {
            if let CompartmentSpec::Named(name) = spec {
                if let Some(c) = self.compartments.iter().find(|c| c.name == *name) {
                    *spec = CompartmentSpec::Inline(c.clone());
                }
            }
        }
        if let Some(ref name) = out.loading_condition_name {
            if out.loading_condition.is_none() {
                let cond = self.loading_condition(Some(name))?;
                out.loading_condition = Some(self.with_windage(cond));
            }
        }
        Ok(out)
    }
}

fn unique<'a>(what: &str, names: impl Iterator<Item = &'a String>) -> Result<(), String> {
    let mut seen = std::collections::HashSet::new();
    for n in names {
        if !seen.insert(n) {
            return Err(format!("two {what}s are named `{n}`"));
        }
    }
    Ok(())
}

fn pick<'a, T>(
    what: &str,
    items: &'a [T],
    name: Option<&str>,
    name_of: impl Fn(&T) -> &String,
) -> Result<&'a T, String> {
    match name {
        None => items
            .first()
            .ok_or_else(|| format!("the project has no {what}")),
        Some(n) => items.iter().find(|t| name_of(t) == n).ok_or_else(|| {
            let known: Vec<&str> = items.iter().map(|t| name_of(t).as_str()).collect();
            format!("no {what} named `{n}` (have: {})", known.join(", "))
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_project() -> Project {
        let lc: LoadingCondition =
            serde_json::from_str(include_str!("../../../examples/demo-loadcase.json")).unwrap();
        let dc: DamageCase =
            serde_json::from_str(include_str!("../../../examples/demo-damage.json")).unwrap();
        Project {
            compartments: match &dc.compartments[0] {
                CompartmentSpec::Inline(c) => vec![c.clone()],
                CompartmentSpec::Named(_) => unreachable!(),
            },
            damage_cases: vec![DamageCase {
                compartments: vec![CompartmentSpec::Named("Midship compartment".into())],
                ..dc
            }],
            loading_conditions: vec![lc],
            windage: vec![WindageItem {
                name: "Deckhouse".into(),
                area_m2: 3.5,
                z_centroid: 2.4,
                profile: None,
            }],
            report: ReportSettings {
                kg: Some(1.1),
                speeds_kn: Some(vec![5.0, 7.0]),
                loading_conditions: Some(vec!["Demo condition".into()]),
                damage_cases: Some(vec!["Damage Case".into()]),
                include_weather: Some(true),
                seakeeping: Some(ReportSeakeeping {
                    speed_kn: 6.0,
                    kg: Some(1.1),
                    sea: Some(WaveSpectrum::Bretschneider { hs: 1.5, tp: 6.0 }),
                }),
                ..Default::default()
            },
            ..Default::default()
        }
    }

    #[test]
    fn round_trips_through_json() {
        let p = demo_project();
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back, p);
        assert!(!p.is_empty());
        assert!(Project::default().is_empty());
    }

    #[test]
    fn loads_legacy_project_without_new_report_keys() {
        let legacy_json = r#"{
            "format": "seacraft-project",
            "version": 1,
            "report": {
                "kg": 1.1,
                "speeds_kn": [5.0, 7.0]
            }
        }"#;
        let p = Project::from_json(legacy_json).unwrap();
        assert_eq!(p.report.kg, Some(1.1));
        assert_eq!(p.report.speeds_kn, Some(vec![5.0, 7.0]));
        assert_eq!(p.report.loading_conditions, None);
        assert_eq!(p.report.damage_cases, None);
        assert_eq!(p.report.include_weather, None);
        assert_eq!(p.report.seakeeping, None);
    }

    #[test]
    fn keeps_unknown_keys_and_refuses_newer_versions_and_other_formats() {
        let mut v: serde_json::Value = serde_json::from_str(&demo_project().to_json()).unwrap();
        v["stix"] = serde_json::json!({ "sail_area_m2": 40.0 });
        let p = Project::from_json(&v.to_string()).unwrap();
        assert_eq!(p.extra["stix"]["sail_area_m2"], 40.0);
        let again: serde_json::Value = serde_json::from_str(&p.to_json()).unwrap();
        assert_eq!(again["stix"]["sail_area_m2"], 40.0);

        v["version"] = serde_json::json!(PROJECT_VERSION + 1);
        assert!(Project::from_json(&v.to_string())
            .unwrap_err()
            .contains("not supported"));
        v["version"] = serde_json::json!(1);
        v["format"] = serde_json::json!("something-else");
        assert!(Project::from_json(&v.to_string()).is_err());
        // A bare loading condition is not a project.
        assert!(Project::from_json(include_str!("../../../examples/demo-loadcase.json")).is_err());
    }

    #[test]
    fn rejects_duplicate_names() {
        let mut p = demo_project();
        p.loading_conditions.push(p.loading_conditions[0].clone());
        assert!(Project::from_json(&p.to_json())
            .unwrap_err()
            .contains("named"));
    }

    #[test]
    fn picks_by_name_and_merges_windage_and_compartments() {
        let p = demo_project();
        let first = p.loading_condition(None).unwrap();
        assert_eq!(p.loading_condition(Some(&first.name)).unwrap(), first);
        assert!(p
            .loading_condition(Some("nope"))
            .unwrap_err()
            .contains("have:"));

        let merged = p.with_windage(first);
        assert_eq!(merged.windage_extra.len(), first.windage_extra.len() + 1);
        assert_eq!(merged.windage_extra[0].name, "Deckhouse");

        let case = p.resolve_damage_case(p.damage_case(None).unwrap()).unwrap();
        assert!(matches!(&case.compartments[0], CompartmentSpec::Inline(c) if c.x_min == 3.5));
    }

    #[test]
    fn case_using_loading_condition_name_resolves_to_condition_inline() {
        let p = demo_project();
        let cond = p.loading_condition(None).unwrap();
        let base_case = p.damage_case(None).unwrap();

        let case_by_name = DamageCase {
            name: "Damage by condition name".into(),
            loading_condition_name: Some(cond.name.clone()),
            loading_condition: None,
            ..base_case.clone()
        };

        let case_inline = DamageCase {
            name: "Damage by condition name".into(),
            loading_condition_name: Some(cond.name.clone()),
            loading_condition: Some(p.with_windage(cond)),
            ..base_case.clone()
        };

        let resolved_by_name = p.resolve_damage_case(&case_by_name).unwrap();
        let resolved_inline = p.resolve_damage_case(&case_inline).unwrap();

        assert_eq!(resolved_by_name, resolved_inline);
        assert!(resolved_by_name.loading_condition.is_some());
    }

    #[test]
    fn validate_checks_damage_case_loading_condition_name_exists() {
        let mut p = demo_project();
        p.damage_cases.push(DamageCase {
            name: "Missing cond case".into(),
            loading_condition_name: Some("nonexistent_cond".into()),
            ..Default::default()
        });
        let err = p.validate().unwrap_err();
        assert!(err.contains("damage case `Missing cond case`"));
        assert!(err.contains("nonexistent_cond"));
    }

    /// R8-2 A05. Compartments are checked on their own, and a damage case's
    /// fields and compartment names are checked against the project — a
    /// case being built (no compartments yet) still saves.
    #[test]
    fn validate_checks_compartments_and_damage_case_references() {
        assert!(demo_project().validate().is_ok());

        let mut p = demo_project();
        p.compartments[0].permeability = 1.5;
        let err = p.validate().unwrap_err();
        assert!(err.contains("permeability"), "{err}");

        let mut p = demo_project();
        p.damage_cases[0].compartments = vec![CompartmentSpec::Named("Engine room".into())];
        let err = p.validate().unwrap_err();
        assert!(err.contains("Engine room"), "{err}");

        // A tank of the case's loading condition is a valid name.
        let mut p = demo_project();
        let tank = p.loading_conditions[0].tanks[0].name.clone();
        p.damage_cases[0].compartments = vec![CompartmentSpec::Named(tank)];
        p.damage_cases[0].loading_condition_name = Some(p.loading_conditions[0].name.clone());
        p.validate().expect("a loading-condition tank resolves");

        let mut p = demo_project();
        p.damage_cases[0].water_density = Some(1025.0);
        assert!(p.validate().unwrap_err().contains("t/m³"));

        let mut p = demo_project();
        p.damage_cases[0].compartments.clear();
        p.validate()
            .expect("a case without compartments yet still saves");
    }

    #[test]
    fn report_settings_overlay_only_what_is_set() {
        let surface = crate::demo::demo_hull(true);
        let base = ReportOptions::from_surface(&surface);
        let opts = demo_project().report.apply(base.clone());
        assert_eq!(opts.kg, 1.1);
        assert_eq!(opts.speeds_kn, vec![5.0, 7.0]);
        assert_eq!(opts.draft, base.draft);
        assert_eq!(opts.heels, base.heels);
    }

    #[test]
    fn a_set_kg_round_trips_and_alone_makes_a_project_worth_writing() {
        let p = Project {
            kg: Some(1.3),
            ..Default::default()
        };
        assert!(!p.is_empty(), "a KG alone must be saved");
        assert!(p.to_json().contains("\"kg\": 1.3"));
        assert_eq!(Project::from_json(&p.to_json()).unwrap(), p);
        // Unset, nothing is written; an old file without it reads as unset.
        assert!(!Project::default().to_json().contains("\"kg\""));
        let old = r#"{ "format": "seacraft-project", "version": 1 }"#;
        assert_eq!(Project::from_json(old).unwrap().kg, None);
        let bad = r#"{ "format": "seacraft-project", "version": 1, "kg": -1.0 }"#;
        assert!(Project::from_json(bad).is_err());
    }

    #[test]
    fn requirements_round_trip_and_alone_make_a_project_worth_writing() {
        let p = Project {
            requirements: Some(Requirements {
                name: "Motor launch 12".into(),
                lwl: Some(12.0),
                bwl: Some(3.6),
                draft: Some(0.75),
                depth: Some(1.8),
                displacement_t: Some(12.0),
                cb: Some(0.36),
                lcb_frac: Some(0.52),
                design_speed_kn: Some(8.0),
                kg: Some(1.2),
            }),
            ..Default::default()
        };
        assert!(!p.is_empty(), "requirements alone must be saved");
        let back = Project::from_json(&p.to_json()).unwrap();
        assert_eq!(back, p);
        // An old sidecar has no `requirements` key and still loads.
        let old = r#"{ "format": "seacraft-project", "version": 1, "windage": [] }"#;
        assert_eq!(Project::from_json(old).unwrap().requirements, None);
        // And a project without them does not write the key.
        assert!(!Project::default().to_json().contains("requirements"));
        // Nonsense targets are refused.
        let mut bad = p.clone();
        bad.requirements.as_mut().unwrap().lcb_frac = Some(1.5);
        assert!(Project::from_json(&bad.to_json())
            .unwrap_err()
            .contains("lcb_frac"));
    }

    #[test]
    fn saves_next_to_the_model_and_loads_back() {
        let dir = std::env::temp_dir().join(format!("seacraft-project-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let model = dir.join("hull.fef");
        assert_eq!(sidecar_path(&model), dir.join("hull.seacraft.json"));
        assert_eq!(Project::load_for_model(&model).unwrap(), None);
        let p = demo_project();
        p.save(&sidecar_path(&model)).unwrap();
        assert_eq!(Project::load_for_model(&model).unwrap(), Some(p));
        assert!(!dir.join("hull.seacraft.json.tmp").exists());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
