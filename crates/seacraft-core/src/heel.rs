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
//! `g` on the centreline the lever uses the same solved waterplane and
//! [`righting_lever`] helper as the cross curves' GZ (identical bit for bit when
//! the solved plane and `g.z` match identically, and agreeing numerically in
//! general).
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
    /// Whether an open hull's artificial deck lid reaches below this waterplane.
    pub lid_submerged: bool,
    /// The solved waterplane (`side <= 0` submerged).
    #[serde(skip)]
    pub plane: Plane,
}

/// Warning emitted when a reported heel state relies on an open hull's artificial lid.
pub const LID_SUBMERGED_WARNING: &str =
    "the hull is open (no deck faces) and its open edge is below the \
    waterline; it is measured closed by a lid, as if the deck were watertight";

/// Hull volume required to float `mass` at `base`'s water density and appendage coefficient.
fn target_volume(mass: &MassCondition, base: &HydroOptions) -> f64 {
    mass.displacement_t * 1000.0 / (base.water_density * base.appendage_coefficient.max(1e-6))
}

/// Maximum submerged volume of the closed hull (m³).
fn hull_max_volume(hull: &Hull) -> f64 {
    let up = Vec3::new(0.0, 0.0, 1.0);
    let (_, full) = hull.extent_along(up);
    if !full.is_finite() {
        return 0.0;
    }
    hull.buoyancy_in(&crate::hydro::waterplane_at_offset(up, full + 1.0))
        .0
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
    let volume = target_volume(mass, base);
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
        lid_submerged: hull.lid_submerged(&wl.plane),
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
    /// No equilibrium could be determined (for example an unreachable LCG or a
    /// solver gap across a crossing).
    Unresolved { reason: String },
}

impl Equilibrium {
    /// The heel the hull settles at from the side of `side` (`Loll` takes
    /// the root on `side`'s sign, starboard for `0`); `None` when it capsizes,
    /// cannot float, or is unresolved.
    pub fn heel_for_side(&self, side: f64) -> Option<f64> {
        match self {
            Self::Stable { state } => Some(state.heel_deg),
            Self::Loll { port, starboard } => Some(if side < 0.0 {
                port.heel_deg
            } else {
                starboard.heel_deg
            }),
            Self::Capsizes { .. } | Self::CannotFloat | Self::Unresolved { .. } => None,
        }
    }
}

impl Equilibrium {
    /// The outcome in words, as the CLI prints it and the view reads it:
    /// "heels 12.0° to starboard, trim +0.003 m, draft 1.050 m", "lolls 14.2°
    /// to either side", "capsizes: …" or "cannot float 1200.0 t" (`displacement_t`
    /// is only used by `CannotFloat`).
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
            Self::Loll { port, starboard } => {
                let p = format!("{:.1}", port.heel_deg.abs());
                let s = format!("{:.1}", starboard.heel_deg.abs());
                if p == s {
                    format!("lolls {s}° to either side")
                } else {
                    format!("lolls {p}° to port or {s}° to starboard")
                }
            }
            Self::Capsizes { .. } => "capsizes: no stable equilibrium within ±90°".to_string(),
            Self::CannotFloat => format!("cannot float {displacement_t:.1} t"),
            Self::Unresolved { reason } => reason.clone(),
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
    let at_heel = at_heel.and_then(|h| heel_state(hull, &mass, h, base, 0.0));
    let eq_lid = match &equilibrium {
        Equilibrium::Stable { state } => state.lid_submerged,
        Equilibrium::Loll { port, starboard } => port.lid_submerged || starboard.lid_submerged,
        Equilibrium::Capsizes { .. }
        | Equilibrium::CannotFloat
        | Equilibrium::Unresolved { .. } => false,
    };
    let at_lid = at_heel.as_ref().is_some_and(|s| s.lid_submerged);
    let warnings = if eq_lid || at_lid {
        vec![LID_SUBMERGED_WARNING.to_string()]
    } else {
        Vec::new()
    };
    HeelReport {
        start: ConditionSummary::of(start, hull.keel),
        weights: weights.to_vec(),
        condition: ConditionSummary::of(&mass, hull.keel),
        free_to_trim: true,
        summary: equilibrium.describe(mass.displacement_t),
        equilibrium,
        at_heel,
        warnings,
    }
}

/// The heel tolerance the equilibrium is refined to, degrees.
pub const EQUILIBRIUM_TOL_DEG: f64 = 0.005;

/// Below this `|TCG|` (metres) the condition is symmetric when upright fails.
const SYMMETRIC_TCG_M: f64 = 1e-9;

/// Below this `|lever_m|` (metres) upright is treated as an equilibrium rather than
/// carrying a nonzero heeling moment.
const UPRIGHT_LEVER_M: f64 = 1e-9;

enum SideSearch {
    Found(HeelState),
    Upright,
    Unresolved(String),
    None,
}

/// Initial free-trim upright GM (`n.z * (I_eff / V) + B.z - G.z`) from the closed
/// waterplane's moments at `u`, returning `None` when the waterplane has no closed loops.
fn upright_waterplane_gm(hull: &Hull, u: &HeelState) -> Option<f64> {
    let (v, _) = hull.buoyancy_in(&u.plane);
    if v <= 1e-12 || u.plane.n.z <= 1e-9 {
        return None;
    }
    let (i_t, i_l, i_tl) = hull.waterplane_inertias(&u.plane)?;
    if !i_t.is_finite() || i_l <= 1e-12 || !i_tl.is_finite() {
        return None;
    }
    // Free-trim condition delta(B.x) = 0 couples pitch into roll as
    // delta_theta = -(I_TL / I_L) * delta_alpha, giving effective inertia
    // I_eff = I_T - I_TL^2 / I_L and lever slope n.z * (I_eff / V) + (B.z - G.z).
    let i_eff = (i_t - (i_tl * i_tl) / i_l).max(0.0);
    Some(u.plane.n.z * (i_eff / v) + (u.b.z - u.g.z))
}

/// Free-trim initial transverse metacentric height at upright (`d(lever)/dφ` in m/rad),
/// accounting for heel–trim coupling through `I_TL`, with a small-angle finite-difference
/// fallback when waterplane inertias are unavailable.
pub fn upright_free_trim_gm(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    u: &HeelState,
) -> Option<f64> {
    if let Some(gm) = upright_waterplane_gm(hull, u) {
        return Some(gm);
    }
    let hint = u.trim_m.unwrap_or(0.0);
    let stbd = heel_state(hull, mass, EQUILIBRIUM_TOL_DEG, base, hint)?;
    let port = heel_state(hull, mass, -EQUILIBRIUM_TOL_DEG, base, hint)?;
    Some((stbd.lever_m - port.lever_m) / (2.0 * EQUILIBRIUM_TOL_DEG.to_radians()))
}

/// Whether `u` at 0° is locally stable with trim free (`LCB.x == G.x`), accounting for
/// heel–trim coupling through the waterplane product of inertia `I_TL`.
fn upright_locally_stable(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    u: &HeelState,
) -> bool {
    if let Some(gm) = upright_waterplane_gm(hull, u) {
        return gm >= 0.0;
    }
    let stbd = heel_state(
        hull,
        mass,
        EQUILIBRIUM_TOL_DEG,
        base,
        u.trim_m.unwrap_or(0.0),
    );
    let port = heel_state(
        hull,
        mass,
        -EQUILIBRIUM_TOL_DEG,
        base,
        u.trim_m.unwrap_or(0.0),
    );
    stbd.is_some_and(|s| s.lever_m >= 0.0) && port.is_some_and(|p| p.lever_m <= 0.0)
}

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
    if sweep.iter().all(Option::is_none) {
        let tv = target_volume(mass, base);
        let mv = hull_max_volume(hull);
        if !tv.is_finite() || tv <= 0.0 || !mv.is_finite() || mv <= 0.0 || tv > mv {
            return Equilibrium::CannotFloat;
        }
        return Equilibrium::Unresolved {
            reason: "no free-trim solution found".into(),
        };
    }

    let zero_idx = sweep
        .iter()
        .position(|o| o.as_ref().is_some_and(|s| s.heel_deg == 0.0));
    let upright = zero_idx
        .and_then(|i| sweep[i].clone())
        .or_else(|| heel_state(hull, mass, 0.0, base, 0.0));

    let side_slots = |side: f64| -> Vec<Option<&HeelState>> {
        if let Some(zi) = zero_idx {
            if side > 0.0 {
                sweep[zi + 1..].iter().map(|o| o.as_ref()).collect()
            } else {
                sweep[..zi].iter().rev().map(|o| o.as_ref()).collect()
            }
        } else if side > 0.0 {
            let start = sweep
                .iter()
                .rposition(|o| o.as_ref().is_some_and(|s| s.heel_deg < 0.0))
                .map_or(0, |i| i + 1);
            sweep[start..].iter().map(|o| o.as_ref()).collect()
        } else {
            let end = sweep
                .iter()
                .position(|o| o.as_ref().is_some_and(|s| s.heel_deg > 0.0))
                .unwrap_or(sweep.len());
            sweep[..end].iter().rev().map(|o| o.as_ref()).collect()
        }
    };

    let Some(u) = upright else {
        if mass.tcg().abs() <= SYMMETRIC_TCG_M {
            return Equilibrium::Unresolved {
                reason: "no free-trim solution upright".into(),
            };
        }
        let side = mass.tcg().signum();
        let slots = side_slots(side);
        if let Some(Some(s1)) = slots.first() {
            if s1.lever_m * side >= 0.0 {
                return Equilibrium::Unresolved {
                    reason: "no free-trim waterline near upright".into(),
                };
            }
        }
        return match walk_side(hull, mass, base, &slots, side, None) {
            SideSearch::Found(state) => Equilibrium::Stable { state },
            SideSearch::Unresolved(reason) => Equilibrium::Unresolved { reason },
            SideSearch::None | SideSearch::Upright => Equilibrium::Capsizes {
                last_heel_deg: last_heel(sweep, side),
            },
        };
    };

    if u.lever_m.abs() > UPRIGHT_LEVER_M {
        // A positive upright lever rotates towards port (−); negative towards starboard (+).
        let side = -u.lever_m.signum();
        let slots = side_slots(side);
        return match walk_side(hull, mass, base, &slots, side, Some(&u)) {
            SideSearch::Found(state) => Equilibrium::Stable { state },
            SideSearch::Unresolved(reason) => Equilibrium::Unresolved { reason },
            SideSearch::None | SideSearch::Upright => Equilibrium::Capsizes {
                last_heel_deg: last_heel(sweep, side),
            },
        };
    }

    if upright_locally_stable(hull, mass, base, &u) {
        return Equilibrium::Stable { state: u };
    }

    let loll_on_side = |side: f64| -> SideSearch {
        let slots = side_slots(side);
        let Some(&first) = slots.first() else {
            return SideSearch::Unresolved("no free-trim waterline near upright".into());
        };
        let Some(s1) = first else {
            return SideSearch::Unresolved("no free-trim waterline near upright".into());
        };
        if s1.lever_m * side < 0.0 {
            return walk_side(hull, mass, base, &slots[1..], side, Some(s1));
        }
        // The loll root lies inside (0°, s1.heel_deg]: step inward from s1 until the
        // lever turns unstable, then refine that bracket.
        let mut outer = s1.clone();
        let mut h = outer.heel_deg * 0.5;
        while h.abs() >= 1e-4 {
            let hint = outer.trim_m.unwrap_or(0.0);
            let Some(s) = heel_state(hull, mass, h, base, hint) else {
                return SideSearch::Unresolved("no free-trim waterline near upright".into());
            };
            if s.lever_m * side < 0.0 {
                if outer.lever_m == 0.0 {
                    return SideSearch::Found(outer);
                }
                let (lo, hi) = if side > 0.0 {
                    (&s, &outer)
                } else {
                    (&outer, &s)
                };
                return match refine(hull, mass, base, lo, hi) {
                    Some(state) => SideSearch::Found(state),
                    None => {
                        SideSearch::Unresolved("no free-trim waterline while refining loll".into())
                    }
                };
            }
            outer = s;
            h *= 0.5;
        }
        SideSearch::Upright
    };

    let stbd = loll_on_side(1.0);
    let port = loll_on_side(-1.0);
    match (port, stbd) {
        (SideSearch::Found(port), SideSearch::Found(starboard)) => {
            Equilibrium::Loll { port, starboard }
        }
        (SideSearch::Unresolved(reason), _) | (_, SideSearch::Unresolved(reason)) => {
            Equilibrium::Unresolved { reason }
        }
        (SideSearch::Found(state), _) | (_, SideSearch::Found(state)) => {
            Equilibrium::Stable { state }
        }
        (SideSearch::Upright, SideSearch::Upright) => Equilibrium::Stable { state: u },
        _ => Equilibrium::Capsizes {
            last_heel_deg: last_heel(sweep, 1.0),
        },
    }
}

/// The furthest heel sampled on `side`.
fn last_heel(sweep: &[Option<HeelState>], side: f64) -> f64 {
    sweep
        .iter()
        .flatten()
        .map(|s| s.heel_deg * side)
        .fold(0.0, f64::max)
        * side
}

/// Walk `slots` away from upright on `side` (+1 starboard, −1 port), preserving `None`
/// gaps so an unsolved span is never bridged into a crossing.
fn walk_side(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    slots: &[Option<&HeelState>],
    side: f64,
    initial: Option<&HeelState>,
) -> SideSearch {
    let mut prev = initial;
    let mut saw_gap = false;
    for &slot in slots {
        let Some(cur) = slot else {
            if prev.is_some() {
                saw_gap = true;
            }
            continue;
        };
        if let Some(p) = prev {
            let exact_zero = cur.lever_m == 0.0 && p.lever_m * side < 0.0;
            let (lo, hi) = if side > 0.0 { (p, cur) } else { (cur, p) };
            let bracket = lo.lever_m < 0.0 && hi.lever_m >= 0.0;
            if exact_zero || bracket {
                if saw_gap {
                    return SideSearch::Unresolved(
                        "no free-trim waterline across the equilibrium crossing".into(),
                    );
                }
                if exact_zero {
                    return SideSearch::Found(cur.clone());
                }
                return match refine(hull, mass, base, lo, hi) {
                    Some(state) => SideSearch::Found(state),
                    None => SideSearch::Unresolved(
                        "no free-trim waterline while refining equilibrium".into(),
                    ),
                };
            }
        }
        prev = Some(cur);
        saw_gap = false;
    }
    if saw_gap || prev.is_none() {
        return SideSearch::Unresolved(
            "no free-trim waterline across the remaining heel range".into(),
        );
    }
    SideSearch::None
}

/// Bisect the bracket `lo.lever < 0 <= hi.lever` (in heel order) to
/// [`EQUILIBRIUM_TOL_DEG`] and return the state at the root.
fn refine(
    hull: &Hull,
    mass: &MassCondition,
    base: &HydroOptions,
    lo: &HeelState,
    hi: &HeelState,
) -> Option<HeelState> {
    let (mut lo, mut hi) = (lo.clone(), hi.clone());
    if hi.lever_m == 0.0 {
        return Some(hi);
    }
    while hi.heel_deg - lo.heel_deg > EQUILIBRIUM_TOL_DEG {
        let mid = 0.5 * (lo.heel_deg + hi.heel_deg);
        let hint = lo.trim_m.or(hi.trim_m).unwrap_or(0.0);
        let m = heel_state(hull, mass, mid, base, hint)?;
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
    heel_state(hull, mass, at, base, hint)
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
            port: p.clone(),
            starboard: s.clone(),
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
        let mut pa = p.clone();
        pa.heel_deg = -14.2;
        s.heel_deg = 16.4;
        let asym = Equilibrium::Loll {
            port: pa,
            starboard: s,
        };
        assert_eq!(
            asym.describe(42.0),
            "lolls 14.2° to port or 16.4° to starboard"
        );
        assert_eq!(
            Equilibrium::Unresolved {
                reason: "no free-trim solution found".into()
            }
            .describe(42.0),
            "no free-trim solution found"
        );
    }

    fn box_hull(deck: bool, y_shift: f64, lcg: Option<f64>) -> (Hull, HydroOptions, MassCondition) {
        let (l, b, d) = (10.0, 4.0, 3.0);
        let mut s = crate::Surface::new();
        s.layers = vec![crate::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Default::default()
        }];
        s.settings = crate::ProjectSettings {
            name: "box".into(),
            length: l,
            beam: b,
            draft: 1.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..Default::default()
        };
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ] {
            s.add_point(Vec3::new(x, y - b * 0.5 + y_shift, z));
        }
        for face in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            if !deck && face == [4, 5, 6, 7] {
                continue;
            }
            s.add_face(face, 0);
        }
        s.initialize_trace();
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = crate::mesh::VertexKind::Corner;
        }
        let sub = level1();
        let base = HydroOptions::from_surface(&s, sub);
        let hull = Hull::new(&s, sub);
        let mass = MassCondition::bare(&hull, &base, 1.6, lcg).unwrap();
        (hull, base, mass)
    }

    #[test]
    fn lid_submerged_warns_only_for_open_hull_with_wet_lid() {
        let (open, base_o, mass_o) = box_hull(false, 0.0, Some(5.0));
        let upright = heel_state(&open, &mass_o, 0.0, &base_o, 0.0).unwrap();
        assert!(!upright.lid_submerged);
        let at90 = heel_state(&open, &mass_o, 90.0, &base_o, 0.0).unwrap();
        assert!(at90.lid_submerged);
        let r_open_up = report(&open, &base_o, &mass_o, &[], Some(0.0));
        assert!(r_open_up.warnings.is_empty(), "{:?}", r_open_up.warnings);
        let r_open_90 = report(&open, &base_o, &mass_o, &[], Some(90.0));
        assert!(
            r_open_90
                .warnings
                .iter()
                .any(|w| w == LID_SUBMERGED_WARNING),
            "{:?}",
            r_open_90.warnings
        );

        let (closed, base_c, mass_c) = box_hull(true, 0.0, Some(5.0));
        let c90 = heel_state(&closed, &mass_c, 90.0, &base_c, 0.0).unwrap();
        assert!(!c90.lid_submerged);
        let r_closed_90 = report(&closed, &base_c, &mass_c, &[], Some(90.0));
        assert!(
            r_closed_90.warnings.is_empty(),
            "{:?}",
            r_closed_90.warnings
        );
    }

    #[test]
    fn unreachable_lcg_is_unresolved_not_cannot_float() {
        let (hull, base, mass) = box_hull(true, 0.0, Some(11.0));
        let eq = equilibrium(&hull, &mass, &base, None);
        assert!(
            matches!(&eq, Equilibrium::Unresolved { reason } if reason.contains("free-trim")),
            "got {eq:?}"
        );
        assert_eq!(eq.heel_for_side(1.0), None);
    }

    #[test]
    fn shifted_hull_with_centred_cg_seeks_equilibrium_away_from_upright() {
        // Box shifted 0.2 m to starboard with G on the model centreline (TCG = 0):
        // upright has lever +0.2 m, so the hull must heel to port to balance B and G.
        let (hull, base, mass) = box_hull(true, 0.2, Some(5.0));
        let eq = equilibrium(&hull, &mass, &base, None);
        let Equilibrium::Stable { state } = eq else {
            panic!("expected Stable, got {eq:?}");
        };
        assert!(
            state.heel_deg < -1.0,
            "expected port heel: {}",
            state.heel_deg
        );
        assert!(state.lever_m.abs() < 1e-3, "lever {}", state.lever_m);
    }

    #[test]
    fn sweep_gap_across_crossing_and_failed_refine_stay_unresolved() {
        let (hull, base, mut mass) = box_hull(true, 0.0, Some(5.0));
        mass.g.y = 0.05;
        let full = heel_sweep(&hull, &mass, &sweep_heels(), &base);
        // Drop samples from +2° to +30° so the crossing near 12° spans an unsolved gap.
        let gapped: Vec<Option<HeelState>> = full
            .iter()
            .cloned()
            .map(|o| o.filter(|s| !(2.0..=30.0).contains(&s.heel_deg)))
            .collect();
        let eq = equilibrium(&hull, &mass, &base, Some(&gapped));
        assert!(matches!(eq, Equilibrium::Unresolved { .. }), "got {eq:?}");

        // Keep samples through +2° (where the starboard lever is still negative) and replace
        // +4° through +90° with None: losing the rest of the search side must return Unresolved,
        // never Capsizes.
        let truncated: Vec<Option<HeelState>> = full
            .into_iter()
            .map(|o| o.filter(|s| s.heel_deg <= 2.0))
            .collect();
        let eq = equilibrium(&hull, &mass, &base, Some(&truncated));
        assert!(matches!(eq, Equilibrium::Unresolved { .. }), "got {eq:?}");

        // Force refine() to fail its midpoint solve by overloading mass on a synthetic bracket.
        let lo = heel_state(&hull, &mass, 0.0, &base, 0.0).unwrap();
        let hi = heel_state(&hull, &mass, 20.0, &base, 0.0).unwrap();
        let mut heavy = mass;
        heavy.displacement_t = 500.0;
        assert!(refine(&hull, &heavy, &base, &lo, &hi).is_none());
    }

    /// Source: analytic. A 10 x 4 x 3 m box at 1 m draft sheared in plan by `y += 0.4 * (x - 5)`
    /// with KG = 1.90 m and LCG = 5.0 m has KB = 0.50 m, I_T = 320/3, I_L = 1000/3, I_TL = 400/3
    /// and V = 40 m^3, giving fixed-trim GM = +1.267 m but free-trim GM = -0.067 m (unstable
    /// upright once pitch couples with roll).
    #[test]
    fn sheared_box_with_negative_free_trim_gm_is_unstable_upright() {
        let (l, b, d) = (10.0, 4.0, 3.0);
        let mut s = crate::Surface::new();
        s.layers = vec![crate::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Default::default()
        }];
        s.settings = crate::ProjectSettings {
            name: "sheared-box".into(),
            length: l,
            beam: b,
            draft: 1.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..Default::default()
        };
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ] {
            let y_sheared = (y - b * 0.5) + 0.4 * (x - 5.0);
            s.add_point(Vec3::new(x, y_sheared, z));
        }
        for face in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            s.add_face(face, 0);
        }
        s.initialize_trace();
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = crate::mesh::VertexKind::Corner;
        }
        let sub = level1();
        let base = HydroOptions::from_surface(&s, sub);
        let hull = Hull::new(&s, sub);
        let mass = MassCondition::bare(&hull, &base, 1.90, Some(5.0)).unwrap();
        let upright = heel_state(&hull, &mass, 0.0, &base, 0.0).unwrap();
        assert!(!upright_locally_stable(&hull, &mass, &base, &upright));
        let free_gm = upright_free_trim_gm(&hull, &mass, &base, &upright).unwrap();
        assert!(
            (free_gm + 1.0 / 15.0).abs() < 1e-4,
            "free-trim GM {free_gm} vs -0.0667"
        );
        let eq = equilibrium(&hull, &mass, &base, None);
        assert!(
            !matches!(&eq, Equilibrium::Stable { state } if state.heel_deg.abs() < 1e-3),
            "sheared box with negative free-trim GM must not be Stable at 0°: {eq:?}"
        );

        // Unsheared box with positive GM remains Stable upright.
        let (unsheared, base_u, mass_u) = box_hull(true, 0.0, Some(5.0));
        match equilibrium(&unsheared, &mass_u, &base_u, None) {
            Equilibrium::Stable { state } => assert!(state.heel_deg.abs() < 1e-6),
            other => panic!("expected Stable upright, got {other:?}"),
        }
    }

    /// Source: analytic. Twin 10 x 2 m pontoons at y = +/-3 (starboard closed deck z = 3,
    /// port open boundary z = 0.5 + 0.1x) at waterplane z = 1.0 m with G = (B.x, B.y, 1.0):
    /// V = 37.5 m^3, KB = 43/90 m, I_eff = 2270/11 m^4, giving free-trim GM = +4.9808 m
    /// rather than omitting the open pontoon (-0.344 m).
    #[test]
    fn open_and_closed_twin_pontoons_remain_locally_stable_upright() {
        let mut s = crate::Surface::new();
        s.layers = vec![crate::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Default::default()
        }];
        s.settings = crate::ProjectSettings {
            name: "twin-pontoons".into(),
            length: 10.0,
            beam: 8.0,
            draft: 1.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..Default::default()
        };
        let add_pontoon =
            |s: &mut crate::Surface, y0: f64, y1: f64, z_top: fn(f64) -> f64, closed: bool| {
                let base = s.points.len();
                for (x, y, z) in [
                    (0.0, y0, 0.0),
                    (10.0, y0, 0.0),
                    (10.0, y1, 0.0),
                    (0.0, y1, 0.0),
                    (0.0, y0, z_top(0.0)),
                    (10.0, y0, z_top(10.0)),
                    (10.0, y1, z_top(10.0)),
                    (0.0, y1, z_top(0.0)),
                ] {
                    s.add_point(Vec3::new(x, y, z));
                }
                let faces = [
                    [0, 3, 2, 1],
                    [4, 5, 6, 7],
                    [0, 1, 5, 4],
                    [1, 2, 6, 5],
                    [2, 3, 7, 6],
                    [3, 0, 4, 7],
                ];
                for (idx, f) in faces.into_iter().enumerate() {
                    if !closed && idx == 1 {
                        continue;
                    }
                    s.add_face(f.into_iter().map(|k| base + k).collect(), 0);
                }
            };
        add_pontoon(&mut s, 2.0, 4.0, |_| 3.0, true);
        add_pontoon(&mut s, -4.0, -2.0, |x| 0.5 + 0.1 * x, false);
        s.initialize_trace();
        s.edges.iter_mut().for_each(|e| e.crease = true);
        s.points
            .iter_mut()
            .for_each(|p| p.kind = crate::mesh::VertexKind::Corner);

        let sub = level1();
        let base = HydroOptions::from_surface(&s, sub);
        let hull = Hull::new(&s, sub);
        let wp = crate::math::Plane::new(Vec3::new(0.0, 0.0, 1.0), -1.0);
        let (v, b) = hull.buoyancy_in(&wp);
        let mass = MassCondition {
            displacement_t: v * base.water_density * base.appendage_coefficient / 1000.0,
            g: Vec3::new(b.x, b.y, 1.0),
            kg_estimate: false,
        };
        let u = HeelState {
            heel_deg: 0.0,
            b,
            g: mass.g,
            lever_m: 0.0,
            gz_m: b.y,
            heeling_arm_m: mass.g.y,
            draft: Some(1.0),
            trim_m: Some(0.0),
            displacement_t: mass.displacement_t,
            lid_submerged: hull.lid_submerged(&wp),
            plane: wp,
        };
        assert!(upright_locally_stable(&hull, &mass, &base, &u));
        let gm = upright_free_trim_gm(&hull, &mass, &base, &u).unwrap();
        let expected = (2270.0 / 11.0) / 37.5 + (43.0 / 90.0 - 1.0);
        assert!(
            (gm - expected).abs() < 1e-3,
            "free-trim GM {gm} vs {expected}"
        );
    }
}
