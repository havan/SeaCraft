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

/// Where the hull settles under a [`MassCondition`].
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Equilibrium {
    /// One stable heel: the first from upright on the side the weight pushes.
    Stable { state: HeelState },
    /// `TCG = 0` with a negative upright GM: the hull lolls to either side.
    Loll {
        port: HeelState,
        starboard: HeelState,
    },
    /// No stable equilibrium within ±90°; `last_heel_deg` is the heel the
    /// search reached on the side the weight pushes.
    Capsizes { last_heel_deg: f64 },
    /// The hull cannot float the displacement at all.
    CannotFloat,
}

impl Equilibrium {
    /// The heel the hull settles at from the side of `side` (`Loll` takes
    /// the root on `side`'s sign, starboard for `0`); `None` when it capsizes
    /// or cannot float.
    pub fn heel_for_side(&self, side: f64) -> Option<f64> {
        match self {
            Self::Stable { state } => Some(state.heel_deg),
            Self::Loll { port, starboard } => Some(if side < 0.0 {
                port.heel_deg
            } else {
                starboard.heel_deg
            }),
            Self::Capsizes { .. } | Self::CannotFloat => None,
        }
    }
}

impl Equilibrium {
    /// The outcome in words, as the CLI prints it and the view reads it:
    /// "heels 12.0° to starboard, trim +0.003 m, draft 1.050 m", "lolls 14.2°
    /// to either side", "capsizes: …" or "cannot float 1200.0 t" (`displacement_t`
    /// is only used by the last).
    pub fn describe(&self, displacement_t: f64) -> String {
        match self {
            Self::Stable { state } => {
                let mut words = heel_words(state.heel_deg);
                if let Some(t) = state.trim_m {
                    words.push_str(&format!(", trim {t:+.3} m"));
                }
                if let Some(d) = state.draft {
                    words.push_str(&format!(", draft {d:.3} m"));
                }
                words
            }
            Self::Loll { starboard, .. } => {
                format!("lolls {:.1}° to either side", starboard.heel_deg.abs())
            }
            Self::Capsizes { .. } => "capsizes: no stable equilibrium within ±90°".to_string(),
            Self::CannotFloat => format!("cannot float {displacement_t:.1} t"),
        }
    }
}

/// "heels 12.0° to starboard", "heels 3.0° to port" or "floats upright" (to
/// the printed tenth of a degree).
pub fn heel_words(heel_deg: f64) -> String {
    if heel_deg.abs() < 0.05 {
        "floats upright".to_string()
    } else {
        let side = if heel_deg > 0.0 { "starboard" } else { "port" };
        format!("heels {:.1}° to {side}", heel_deg.abs())
    }
}

const WEIGHT_FORM: &str =
    "a weight is t@x,y,z: tonnes at model x, y, z in metres, e.g. 2.0@5,1,1.6";

/// Read a weight written `t@x,y,z` (tonnes at a model point, metres). The
/// mass must be positive and every number finite.
pub fn parse_weight(text: &str) -> Result<WeightItem, String> {
    let bad = || format!("{WEIGHT_FORM}; got `{text}`");
    let (mass, at) = text.split_once('@').ok_or_else(bad)?;
    let mass: f64 = mass.trim().parse().map_err(|_| bad())?;
    let xyz: Vec<f64> = at
        .split(',')
        .map(|n| n.trim().parse::<f64>())
        .collect::<Result<_, _>>()
        .map_err(|_| bad())?;
    let [x, y, z] = xyz[..] else {
        return Err(bad());
    };
    if !(mass.is_finite() && mass > 0.0) || ![x, y, z].iter().all(|n| n.is_finite()) {
        return Err(bad());
    }
    Ok(WeightItem {
        name: String::new(),
        mass_t: mass,
        x,
        y,
        z,
    })
}

/// Read trial weights from JSON: an array of `{mass_t, x, y, z, name?}`.
/// Unnamed weights are called "Weight N" (1-based).
pub fn weights_from_json(v: &serde_json::Value) -> Result<Vec<WeightItem>, String> {
    let items = v
        .as_array()
        .ok_or("weights must be an array of {mass_t, x, y, z, name?}")?;
    items
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let num = |k: &str| -> Result<f64, String> {
                w.get(k)
                    .and_then(|n| n.as_f64())
                    .filter(|n| n.is_finite())
                    .ok_or_else(|| format!("weights[{i}]: `{k}` must be a finite number"))
            };
            let mass_t = num("mass_t")?;
            if mass_t <= 0.0 {
                return Err(format!("weights[{i}]: `mass_t` must be positive tonnes"));
            }
            let name = match w.get("name").and_then(|n| n.as_str()) {
                Some(n) if !n.trim().is_empty() => n.to_string(),
                _ => format!("Weight {}", i + 1),
            };
            Ok(WeightItem {
                name,
                mass_t,
                x: num("x")?,
                y: num("y")?,
                z: num("z")?,
            })
        })
        .collect()
}

/// A mass condition as a reader wants it: Δ, KG above the keel, LCG, TCG.
#[derive(Debug, Clone, Serialize)]
pub struct ConditionSummary {
    pub displacement_t: f64,
    pub kg_m: f64,
    pub lcg_m: f64,
    pub tcg_m: f64,
    /// The KG is the dock's estimate.
    pub kg_estimate: bool,
}

impl ConditionSummary {
    pub fn of(mass: &MassCondition, keel: f64) -> Self {
        Self {
            displacement_t: mass.displacement_t,
            kg_m: mass.kg(keel),
            lcg_m: mass.g.x,
            tcg_m: mass.tcg(),
            kg_estimate: mass.kg_estimate,
        }
    }
}

/// What `seacraft heel`, the MCP tool `heel_equilibrium` and the control
/// method `stability.heel` answer.
#[derive(Debug, Clone, Serialize)]
pub struct HeelReport {
    /// The condition before the trial weights.
    pub start: ConditionSummary,
    pub weights: Vec<WeightItem>,
    /// The start plus the weights.
    pub condition: ConditionSummary,
    /// Always true: the hull trims freely at every heel.
    pub free_to_trim: bool,
    pub equilibrium: Equilibrium,
    /// [`Equilibrium::describe`].
    pub summary: String,
    /// The state at the requested heel, when one was asked for and floats.
    pub at_heel: Option<HeelState>,
    /// Why the start is doubtful (a loading condition's warnings).
    pub warnings: Vec<String>,
}

/// The equilibrium of `start` plus `weights`, and the state at `at_heel`.
pub fn report(
    hull: &Hull,
    base: &HydroOptions,
    start: &MassCondition,
    weights: &[WeightItem],
    at_heel: Option<f64>,
) -> HeelReport {
    let mass = start.with_weights(weights);
    let equilibrium = equilibrium(hull, &mass, base, None);
    HeelReport {
        start: ConditionSummary::of(start, hull.keel),
        weights: weights.to_vec(),
        condition: ConditionSummary::of(&mass, hull.keel),
        free_to_trim: true,
        summary: equilibrium.describe(mass.displacement_t),
        equilibrium,
        at_heel: at_heel.and_then(|h| heel_state(hull, &mass, h, base, 0.0)),
        warnings: Vec::new(),
    }
}

/// The heel tolerance the equilibrium is refined to, degrees.
pub const EQUILIBRIUM_TOL_DEG: f64 = 0.005;

/// Below this `|TCG|` (metres) the condition is symmetric: the hull is
/// upright unless its upright GM is negative, when it lolls.
const SYMMETRIC_TCG_M: f64 = 1e-9;

/// The heel probed either side of upright to tell a stable upright hull from
/// one with a negative GM, degrees.
const UPRIGHT_PROBE_DEG: f64 = 0.5;

/// The stable equilibrium of `mass`, large-angle: the root of the lever
/// nearest upright, on the side the off-centre weight pushes, where a little
/// more heel is opposed (the lever rises through zero as the heel grows).
/// It is bracketed on `sweep` (a [`heel_sweep`] of this condition; one on
/// [`sweep_heels`] is computed when `None`) and refined by bisection on
/// [`heel_state`] to [`EQUILIBRIUM_TOL_DEG`]. A root past ±90° is
/// [`Equilibrium::Capsizes`].
pub fn equilibrium(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    sweep: Option<&[Option<HeelState>]>,
) -> Equilibrium {
    let owned;
    let sweep = match sweep {
        Some(s) => s,
        None => {
            owned = heel_sweep(hull, mass, &sweep_heels(), base);
            &owned
        }
    };
    let mut samples: Vec<&HeelState> = sweep.iter().flatten().collect();
    if samples.is_empty() {
        return Equilibrium::CannotFloat;
    }
    samples.sort_by(|a, b| a.heel_deg.total_cmp(&b.heel_deg));

    let tcg = mass.tcg();
    if tcg.abs() <= SYMMETRIC_TCG_M {
        let probe = heel_state(hull, mass, UPRIGHT_PROBE_DEG, base, 0.0);
        if probe.is_some_and(|p| p.lever_m > 0.0) {
            return match heel_state(hull, mass, 0.0, base, 0.0) {
                Some(state) => Equilibrium::Stable { state },
                None => Equilibrium::CannotFloat,
            };
        }
        // The upright sample's lever is rounding noise of either sign: each side's walk starts
        // from that side's probe instead, so a loll inside the first sample step is bracketed on
        // both sides rather than on whichever side the noise favours.
        let side_from = |side: f64| -> Option<HeelState> {
            let mut walk: Vec<&HeelState> = samples
                .iter()
                .copied()
                .filter(|s| s.heel_deg * side >= UPRIGHT_PROBE_DEG)
                .collect();
            let inner = heel_state(hull, mass, side * UPRIGHT_PROBE_DEG, base, 0.0)?;
            walk.push(&inner);
            walk.sort_by(|a, b| a.heel_deg.total_cmp(&b.heel_deg));
            root_on_side(hull, mass, base, &walk, side)
        };
        let stbd = side_from(1.0);
        let port = side_from(-1.0);
        return match (port, stbd) {
            (Some(port), Some(starboard)) => Equilibrium::Loll { port, starboard },
            (Some(state), None) | (None, Some(state)) => Equilibrium::Stable { state },
            (None, None) => Equilibrium::Capsizes {
                last_heel_deg: last_heel(&samples, 1.0),
            },
        };
    }
    let side = tcg.signum();
    match root_on_side(hull, mass, base, &samples, side) {
        Some(state) => Equilibrium::Stable { state },
        None => Equilibrium::Capsizes {
            last_heel_deg: last_heel(&samples, side),
        },
    }
}

/// The furthest heel sampled on `side`.
fn last_heel(samples: &[&HeelState], side: f64) -> f64 {
    let heels = samples.iter().map(|s| s.heel_deg * side);
    heels.fold(0.0, f64::max) * side
}

/// The first stable root from upright on `side` (+1 starboard, −1 port):
/// walking the samples away from upright, the first neighbouring pair whose
/// lever goes from below zero to zero or above as the heel *grows*.
fn root_on_side(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    samples: &[&HeelState],
    side: f64,
) -> Option<HeelState> {
    let mut walk: Vec<&HeelState> = samples
        .iter()
        .copied()
        .filter(|s| s.heel_deg * side >= 0.0)
        .collect();
    if side < 0.0 {
        walk.reverse();
    }
    walk.windows(2).find_map(|w| {
        // `lo` and `hi` in heel order, whichever way the walk runs.
        let (lo, hi) = if side > 0.0 {
            (w[0], w[1])
        } else {
            (w[1], w[0])
        };
        (lo.lever_m < 0.0 && hi.lever_m >= 0.0).then(|| refine(hull, mass, base, lo, hi))
    })
}

/// Bisect the bracket `lo.lever < 0 <= hi.lever` (in heel order) to
/// [`EQUILIBRIUM_TOL_DEG`] and return the state at the root.
fn refine(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    lo: &HeelState,
    hi: &HeelState,
) -> HeelState {
    let (mut lo, mut hi) = (lo.clone(), hi.clone());
    if hi.lever_m == 0.0 {
        return hi;
    }
    while hi.heel_deg - lo.heel_deg > EQUILIBRIUM_TOL_DEG {
        let mid = 0.5 * (lo.heel_deg + hi.heel_deg);
        let hint = lo.trim_m.or(hi.trim_m).unwrap_or(0.0);
        let Some(m) = heel_state(hull, mass, mid, base, hint) else {
            break;
        };
        if m.lever_m < 0.0 {
            lo = m;
        } else {
            hi = m;
        }
    }
    // The end of the final bracket nearer the root, by linear interpolation
    // of the lever, solved exactly there.
    let span = hi.lever_m - lo.lever_m;
    let at = if span > 0.0 {
        lo.heel_deg - lo.lever_m * (hi.heel_deg - lo.heel_deg) / span
    } else {
        hi.heel_deg
    };
    let hint = lo.trim_m.or(hi.trim_m).unwrap_or(0.0);
    heel_state(hull, mass, at, base, hint).unwrap_or(if -lo.lever_m < hi.lever_m { lo } else { hi })
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
    #[test]
    fn a_weight_reads_as_tonnes_at_a_point() {
        let w = parse_weight("2.0@5,1,-1.6").unwrap();
        assert_eq!((w.mass_t, w.x, w.y, w.z), (2.0, 5.0, 1.0, -1.6));
        let w = parse_weight(" 0.5 @ 4 , -0.8 , 1.5 ").unwrap();
        assert_eq!((w.mass_t, w.x, w.y, w.z), (0.5, 4.0, -0.8, 1.5));
        for bad in [
            "2@5,1",
            "x@1,2,3",
            "2",
            "2@1,2,3,4",
            "0@1,2,3",
            "-1@1,2,3",
            "inf@1,2,3",
            "2@1,nan,3",
        ] {
            let e = parse_weight(bad).unwrap_err();
            assert!(e.contains("t@x,y,z"), "{bad}: {e}");
        }
    }

    #[test]
    fn weights_read_from_json() {
        let v = serde_json::json!([
            {"mass_t": 2.0, "x": 5.0, "y": 1.0, "z": 1.6},
            {"mass_t": 0.5, "x": 4.0, "y": 0.0, "z": 3.0, "name": "Mast"},
        ]);
        let w = weights_from_json(&v).unwrap();
        assert_eq!(w.len(), 2);
        assert_eq!(w[0].name, "Weight 1");
        assert_eq!(w[1].name, "Mast");
        assert_eq!((w[1].mass_t, w[1].z), (0.5, 3.0));
        for bad in [
            serde_json::json!({"mass_t": 1.0}),
            serde_json::json!([{"mass_t": 1.0, "x": 0.0, "y": 0.0}]),
            serde_json::json!([{"mass_t": -1.0, "x": 0.0, "y": 0.0, "z": 0.0}]),
            serde_json::json!([{"mass_t": "2", "x": 0.0, "y": 0.0, "z": 0.0}]),
        ] {
            assert!(weights_from_json(&bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn the_outcome_reads_in_words() {
        let (_, hull, base, kg) = hulls().into_iter().next().unwrap();
        let mass = MassCondition::bare(&hull, &base, kg, None).unwrap();
        let mut s = heel_state(&hull, &mass, 12.04, &base, 0.0).unwrap();
        s.trim_m = Some(0.0031);
        s.draft = Some(1.05);
        let words = Equilibrium::Stable { state: s.clone() }.describe(42.0);
        assert_eq!(
            words,
            "heels 12.0° to starboard, trim +0.003 m, draft 1.050 m"
        );
        s.heel_deg = -3.0;
        let words = Equilibrium::Stable { state: s.clone() }.describe(42.0);
        assert!(words.starts_with("heels 3.0° to port"), "{words}");
        s.heel_deg = 0.001;
        let words = Equilibrium::Stable { state: s.clone() }.describe(42.0);
        assert!(words.starts_with("floats upright"), "{words}");
        let mut p = s.clone();
        p.heel_deg = -14.2;
        s.heel_deg = 14.2;
        let loll = Equilibrium::Loll {
            port: p,
            starboard: s,
        };
        assert_eq!(loll.describe(42.0), "lolls 14.2° to either side");
        assert_eq!(
            Equilibrium::Capsizes {
                last_heel_deg: 90.0
            }
            .describe(42.0),
            "capsizes: no stable equilibrium within ±90°"
        );
        assert_eq!(
            Equilibrium::CannotFloat.describe(1200.0),
            "cannot float 1200.0 t"
        );
    }
}
