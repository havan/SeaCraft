//! The hull heeled under a mass condition: one heeled state, a sweep of them,
//! and (below) the large-angle equilibrium the hull settles at.
//!
//! A [`MassCondition`] is a displacement and a centre of gravity `g` in model
//! coordinates (`z` absolute, not above the keel), possibly off the
//! centreline. At each heel the hull is floated at that displacement **always
//! free to trim** (the longitudinal centre of buoyancy on `g.x`), with
//! [`solve_waterline`] — the solver the GZ curves use — and the
//! righting lever is measured as the cross curves measure
//! it, along the water surface across the hull, but to the actual `g`. With
//! `g` on the centreline the lever *is* the cross curves' GZ, bit for bit.
//!
//! The lever with an off-centre `g` splits into the curve-convention pair:
//! `lever = GZ − heeling arm`, where `GZ` takes `g` on the centreline and the
//! heeling arm is `g`'s offset along the lever direction (`TCG · cos φ` at
//! zero trim). Sign convention as everywhere in the crate: `y` to starboard,
//! a positive heel immerses starboard, a positive lever rights the hull.

use crate::hydro::{Hull, HydroOptions};
use crate::loading::{LoadingResult, WeightItem};
use crate::math::{Plane, Vec3};
use crate::stability::{righting_lever, solve_waterline, StabilityCondition};
use serde::Serialize;

/// A displacement and its centre of gravity.
#[derive(Debug, Clone, Serialize)]
pub struct MassCondition {
    /// Displacement in tonnes.
    pub displacement_t: f64,
    /// Centre of gravity in model coordinates (`z` absolute).
    pub g: Vec3,
    /// Whether the KG is the dock's estimate rather than a typed one.
    pub kg_estimate: bool,
}

impl MassCondition {
    /// The bare hull floating upright at the dock draft: displacement from
    /// the upright volume (× density × appendage coefficient, as the dock),
    /// `g` at `lcg` (else the upright LCB), on the centreline, `kg` above the
    /// keel. `None` when the hull displaces nothing at that draft.
    pub fn bare(hull: &Hull, base: &HydroOptions, kg: f64, lcg: Option<f64>) -> Option<Self> {
        let (volume, b) = hull.buoyancy(base.draft, 0.0, 0.0);
        if volume.is_nan() || volume <= 0.0 {
            return None;
        }
        Some(Self {
            displacement_t: volume * base.water_density * base.appendage_coefficient / 1000.0,
            g: Vec3::new(lcg.unwrap_or(b.x), 0.0, hull.keel + kg),
            kg_estimate: false,
        })
    }

    /// A loading condition's result: its displacement, LCG, TCG and the
    /// fluid KG (free surface included as the usual KG correction).
    pub fn from_loading(r: &LoadingResult, keel: f64) -> Self {
        Self {
            displacement_t: r.displacement_t,
            g: Vec3::new(r.lcg, r.tcg, keel + r.kg_fluid),
            kg_estimate: false,
        }
    }

    /// This condition plus point weights, mass-weighted. Items with no
    /// positive mass are ignored.
    pub fn with_weights(&self, weights: &[WeightItem]) -> Self {
        let mut mass = self.displacement_t;
        let mut moment = self.g * self.displacement_t;
        for w in weights.iter().filter(|w| w.mass_t > 0.0) {
            mass += w.mass_t;
            moment += Vec3::new(w.x, w.y, w.z) * w.mass_t;
        }
        Self {
            displacement_t: mass,
            g: if mass > 0.0 { moment / mass } else { self.g },
            kg_estimate: self.kg_estimate,
        }
    }

    /// Vertical centre of gravity above the keel.
    pub fn kg(&self, keel: f64) -> f64 {
        self.g.z - keel
    }

    /// Transverse centre of gravity (+ to starboard).
    pub fn tcg(&self) -> f64 {
        self.g.y
    }
}

/// The hull floating at one heel under a [`MassCondition`].
#[derive(Debug, Clone, Serialize)]
pub struct HeelState {
    pub heel_deg: f64,
    /// Centre of buoyancy, model coordinates.
    pub b: Vec3,
    /// Centre of gravity, model coordinates.
    pub g: Vec3,
    /// The actual righting lever, to the (possibly off-centre) `g`.
    pub lever_m: f64,
    /// The lever with `g` moved to the centreline: the GZ curve's value.
    pub gz_m: f64,
    /// `gz_m − lever_m`: what the off-centre `g` takes off the lever.
    pub heeling_arm_m: f64,
    /// Draft as [`crate::stability::CrossCurvePoint::draft`]; `None` near ±90°.
    pub draft: Option<f64>,
    /// Trim, metres, positive bow-up; `None` with `draft`.
    pub trim_m: Option<f64>,
    pub displacement_t: f64,
    /// The solved waterplane (`side <= 0` submerged).
    #[serde(skip)]
    pub plane: Plane,
}

/// Float `mass` at `heel_deg`, free to trim, and measure the lever.
/// `trim_hint` seeds the trim solve. `None` when the hull cannot float the
/// displacement (or no trim puts the LCB on `g.x`).
pub fn heel_state(
    hull: &Hull,
    mass: &MassCondition,
    heel_deg: f64,
    base: &HydroOptions,
    trim_hint: f64,
) -> Option<HeelState> {
    // The hull's own volume carries Δ / appendage coefficient, as in the
    // cross curves (same expression, so the same volume bit for bit).
    let volume =
        mass.displacement_t * 1000.0 / (base.water_density * base.appendage_coefficient.max(1e-6));
    let mut cond = StabilityCondition::free_to_trim(mass.kg(hull.keel), mass.g.x);
    cond.trim = trim_hint;
    let wl = solve_waterline(hull, volume, heel_deg, &cond)?;
    let (v, b) = hull.buoyancy_in(&wl.plane);
    if v.is_nan() || v <= 0.0 {
        return None;
    }
    let lever = righting_lever(&wl.plane, b, mass.g);
    let gz = righting_lever(&wl.plane, b, Vec3::new(mass.g.x, 0.0, mass.g.z));
    if !(lever.is_finite() && gz.is_finite()) {
        return None;
    }
    Some(HeelState {
        heel_deg,
        b,
        g: mass.g,
        lever_m: lever,
        gz_m: gz,
        heeling_arm_m: gz - lever,
        draft: wl.floating.map(|f| f.draft),
        trim_m: wl.floating.map(|f| f.trim),
        displacement_t: mass.displacement_t,
        plane: wl.plane,
    })
}

/// The heel step of [`sweep_heels`], degrees.
pub const SWEEP_STEP_DEG: f64 = 2.0;

/// −90° … +90° by [`SWEEP_STEP_DEG`].
pub fn sweep_heels() -> Vec<f64> {
    let n = (90.0 / SWEEP_STEP_DEG).round() as i64;
    (-n..=n).map(|i| i as f64 * SWEEP_STEP_DEG).collect()
}

/// [`heel_state`] at each heel, in order, on every core.
pub fn heel_sweep(
    hull: &Hull,
    mass: &MassCondition,
    heels_deg: &[f64],
    base: &HydroOptions,
) -> Vec<Option<HeelState>> {
    crate::par::par_map(heels_deg, |&h| heel_state(hull, mass, h, base, 0.0))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::{Hull, HydroOptions};
    use crate::loading::WeightItem;
    use crate::math::Vec3;
    use crate::stability::{cross_curves, StabilityCondition};
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    fn level1() -> SubdivisionOptions {
        SubdivisionOptions {
            level: 1,
            mode: SubdivisionMode::Loop,
        }
    }

    /// The demo hull and two example hulls, each with a KG at 40 % of the
    /// hull's depth (an arbitrary but plausible centre of gravity).
    fn hulls() -> Vec<(String, Hull, HydroOptions, f64)> {
        let mut out = Vec::new();
        let demo = crate::demo::demo_hull(false);
        out.push((
            "demo".to_string(),
            Hull::new(&demo, Default::default()),
            HydroOptions::from_surface(&demo, Default::default()),
            2.2,
        ));
        for ex in [
            crate::examples::Example::Yacht10m,
            crate::examples::Example::Runabout6m,
        ] {
            let (surface, _) = crate::examples::build(ex).unwrap();
            let hull = Hull::new(&surface, level1());
            let base = HydroOptions::from_surface(&surface, level1());
            let kg = 0.4 * (hull.model_max.z - hull.keel);
            out.push((ex.to_string(), hull, base, kg));
        }
        out
    }

    /// Source: same code path. With G on the centreline the view's lever
    /// is the cross-curve GZ at the same displacement, free to trim at the
    /// same LCG — the picture and the curves never disagree.
    #[test]
    fn centreline_lever_is_the_cross_curve_gz() {
        for (name, hull, base, kg) in hulls() {
            let mass = MassCondition::bare(&hull, &base, kg, None).unwrap();
            let heels = [0.0, 10.0, 30.0, 60.0, 90.0];
            let cond = StabilityCondition::free_to_trim(kg, mass.g.x);
            let curves = cross_curves(&hull, &heels, &[mass.displacement_t], &cond, &base);
            for (heel, curve) in heels.iter().zip(&curves) {
                let s = heel_state(&hull, &mass, *heel, &base, 0.0)
                    .unwrap_or_else(|| panic!("{name}: no state at {heel}°"));
                let gz = curve.points[0].gz.unwrap();
                assert!(
                    (s.lever_m - gz).abs() < 1e-12,
                    "{name} {heel}°: lever {} vs GZ {gz}",
                    s.lever_m
                );
                assert_eq!(s.heeling_arm_m, 0.0, "{name} {heel}°");
            }
        }
    }

    /// Source: definition. Moving G off the centreline changes only the
    /// lever, by G's offset along the lever direction; the waterplane (and
    /// so GZ with G on the centreline) does not move.
    #[test]
    fn off_centre_g_splits_into_gz_and_heeling_arm() {
        let (_, hull, base, kg) = hulls().into_iter().next().unwrap();
        let centre = MassCondition::bare(&hull, &base, kg, None).unwrap();
        let mut off = centre.clone();
        off.g.y = 0.3;
        for heel in [0.0, 15.0, 40.0] {
            let c = heel_state(&hull, &centre, heel, &base, 0.0).unwrap();
            let s = heel_state(&hull, &off, heel, &base, 0.0).unwrap();
            assert!((s.gz_m - c.lever_m).abs() < 1e-12, "{heel}°");
            let t = s.plane.n.cross(Vec3::new(1.0, 0.0, 0.0)).norm();
            assert!(
                (s.heeling_arm_m - 0.3 * t.y).abs() < 1e-9,
                "{heel}°: arm {} vs {}",
                s.heeling_arm_m,
                0.3 * t.y
            );
            assert!((s.lever_m - (s.gz_m - s.heeling_arm_m)).abs() < 1e-12);
        }
    }

    /// Source: arithmetic (mass-weighted centroid).
    #[test]
    fn weights_combine_by_mass() {
        let base = MassCondition {
            displacement_t: 10.0,
            g: Vec3::new(4.0, 0.0, 1.5),
            kg_estimate: false,
        };
        let w = WeightItem {
            name: "w".into(),
            mass_t: 2.0,
            x: 5.0,
            y: 1.0,
            z: 2.0,
        };
        let ignored = WeightItem {
            name: "zero".into(),
            mass_t: 0.0,
            x: 100.0,
            y: 100.0,
            z: 100.0,
        };
        let m = base.with_weights(&[w, ignored]);
        assert!((m.displacement_t - 12.0).abs() < 1e-12);
        assert!((m.g.x - 50.0 / 12.0).abs() < 1e-12);
        assert!((m.g.y - 2.0 / 12.0).abs() < 1e-12);
        assert!((m.g.z - 19.0 / 12.0).abs() < 1e-12);
        assert!((m.tcg() - 2.0 / 12.0).abs() < 1e-12);
        assert!((m.kg(0.5) - (19.0 / 12.0 - 0.5)).abs() < 1e-12);
    }

    /// Source: same code path as the Hydrostatics dock's displacement.
    #[test]
    fn bare_condition_uses_the_dock_draft() {
        for (name, hull, base, kg) in hulls() {
            let mass = MassCondition::bare(&hull, &base, kg, None).unwrap();
            let dock = hull.hydrostatics(&base).displacement_t;
            assert!(
                (mass.displacement_t - dock).abs() <= 1e-9 * dock,
                "{name}: {} vs {dock}",
                mass.displacement_t
            );
            assert!((mass.kg(hull.keel) - kg).abs() < 1e-12, "{name}");
            assert_eq!(mass.tcg(), 0.0);
        }
    }

    #[test]
    fn a_condition_heavier_than_the_hull_has_no_state() {
        let (_, hull, base, kg) = hulls().into_iter().next().unwrap();
        let full = hull
            .buoyancy(hull.model_max.z - hull.keel + 1.0, 0.0, 0.0)
            .0;
        let mut mass = MassCondition::bare(&hull, &base, kg, None).unwrap();
        mass.displacement_t =
            10.0 * full * base.water_density * base.appendage_coefficient / 1000.0;
        assert!(heel_state(&hull, &mass, 0.0, &base, 0.0).is_none());
        assert!(heel_state(&hull, &mass, 30.0, &base, 0.0).is_none());
    }
}
