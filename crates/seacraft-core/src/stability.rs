//! Cross curves of stability and righting levers.
//!
//! For a set of heel angles the hull is trimmed to a constant displacement by
//! solving for the draft, and the righting lever is measured from the keel.
//! This is the same quantity FREE!ship reports as `KN` (and derives `GZ` from
//! once `KG` is known): `GZ = KN - KG * sin(heel)`.
//!
//! Two floating conditions are supported:
//!
//! * **fixed trim** — the lookout most tools print by default: hold the trim
//!   and heel the hull, solving only for draft. Fast, but it violates
//!   longitudinal equilibrium, because a heeled hull with a wide transom and a
//!   fine bow gains buoyancy aft and loses it forward, so `LCB` walks away from
//!   `LCG` and the righting lever is measured with an unbalanced hull.
//! * **free to trim** — solve `(draft, trim)` so the submerged volume matches
//!   the displacement *and* the longitudinal centre of buoyancy sits at the
//!   longitudinal centre of gravity. This is what the hull actually does when
//!   it heels, and it lowers `KN` towards the true value.
//!
//! Both use the same fast path (`Hull::buoyancy`), so a free-trim curve costs
//! about four draft solves instead of one.

use crate::hydro::{waterplane_for, Hull, HydroOptions};
use crate::math::{deg_to_rad, Vec3};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CrossCurvePoint {
    /// Displacement in tonnes.
    pub displacement_t: f64,
    /// Draft required to float at this displacement and heel.
    pub draft: f64,
    /// Trim required to float at this displacement and heel, in metres and
    /// positive bow-up. Equal to the requested trim unless the curve is free
    /// to trim.
    pub trim_m: f64,
    /// Righting lever from the keel, in metres.
    pub kn: f64,
    /// Righting lever from the centre of gravity, in metres.
    pub gz: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CrossCurve {
    pub heel_deg: f64,
    pub points: Vec<CrossCurvePoint>,
}

/// How the hull is allowed to float while heeling.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct StabilityCondition {
    /// Vertical centre of gravity above the keel.
    pub kg: f64,
    /// Trim held fixed, and the starting guess when free to trim.
    pub trim: f64,
    /// `Some(x)`: trim freely so the longitudinal centre of buoyancy stays at
    /// model `x` — the longitudinal centre of gravity. `None`: hold `trim`.
    pub lcg_x: Option<f64>,
}

impl StabilityCondition {
    /// Hold the trim while heeling.
    pub fn fixed(kg: f64, trim: f64) -> Self {
        Self {
            kg,
            trim,
            lcg_x: None,
        }
    }

    /// Trim to equilibrium at a given longitudinal centre of gravity.
    pub fn free_to_trim(kg: f64, lcg_x: f64) -> Self {
        Self {
            kg,
            trim: 0.0,
            lcg_x: Some(lcg_x),
        }
    }

    /// Whether the solve is free to trim.
    pub fn is_free(&self) -> bool {
        self.lcg_x.is_some()
    }
}

/// A solved floating condition.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct FloatingCondition {
    pub draft: f64,
    /// Positive bow-up, metres.
    pub trim: f64,
}

/// Solve the draft on a bracket that is already known to straddle the target.
/// Returns `None` when it does not, so the caller can widen.
///
/// Displacement is monotone in draft and, for a wall-sided hull, very nearly
/// linear, so a secant step inside the bracket converges in a handful of
/// integrations where plain bisection needs ~30. The bracket is what keeps it
/// safe: a secant landing outside the middle 90 % of the interval is replaced
/// by the bisection midpoint, so a volume jump (a chine at the waterline)
/// cannot stall it.
fn draft_on_bracket(
    hull: &Hull,
    target_volume: f64,
    trim: f64,
    heel_deg: f64,
    mut lo: f64,
    mut hi: f64,
) -> Option<f64> {
    let mut vlo = hull.displaced_volume(lo, trim, heel_deg);
    let mut vhi = hull.displaced_volume(hi, trim, heel_deg);
    if vlo >= target_volume {
        return if lo <= 0.0 { Some(0.0) } else { None };
    }
    if vhi < target_volume {
        return None;
    }
    let tol = 1e-9 * target_volume.max(1.0);
    for _ in 0..80 {
        let mut d = lo + (target_volume - vlo) * (hi - lo) / (vhi - vlo);
        let margin = 0.05 * (hi - lo);
        if !d.is_finite() || d <= lo + margin || d >= hi - margin {
            d = 0.5 * (lo + hi);
        }
        let v = hull.displaced_volume(d, trim, heel_deg);
        if (v - target_volume).abs() <= tol {
            return Some(d);
        }
        if v < target_volume {
            lo = d;
            vlo = v;
        } else {
            hi = d;
            vhi = v;
        }
        if (hi - lo) < 1e-9 {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Find the draft at which the hull displaces `target_volume` (m3).
pub fn solve_draft(hull: &Hull, target_volume: f64, trim: f64, heel_deg: f64) -> Option<f64> {
    let lo = 0.0f64;
    let mut hi = (hull.model_max.z - hull.keel).max(0.1) * 2.0 + 1.0;
    // Expand until the hull floats the target displacement.
    for _ in 0..40 {
        if hull.displaced_volume(hi, trim, heel_deg) >= target_volume {
            break;
        }
        hi *= 1.6;
    }
    draft_on_bracket(hull, target_volume, trim, heel_deg, lo, hi)
}

/// Solve draft *and* trim so the hull floats `target_volume` with the
/// longitudinal centre of buoyancy at `lcg_x`.
///
/// Outer secant on the trim (the `LCB`/trim relation is almost linear), inner
/// bisection on the draft for each trim. Returns `None` when no trim within
/// ±45 % of the length puts `LCB` on `lcg_x` — an unreachable centre of
/// gravity, not a solver wobble.
pub fn solve_floating(
    hull: &Hull,
    target_volume: f64,
    heel_deg: f64,
    lcg_x: f64,
    trim_hint: f64,
) -> Option<FloatingCondition> {
    let max_trim = (0.45 * hull.length).max(0.01);
    // One (trim -> draft, residual) sample: solve the volume first, then read
    // the LCB. A `draft_hint` from the previous trim keeps the draft bracket
    // tight, which halves the integrations a free-trim curve costs.
    let sample = |trim: f64, draft_hint: Option<f64>| -> Option<(f64, f64)> {
        let draft = draft_hint
            .and_then(|d| {
                let w = (0.25 * d).clamp(0.02, 0.5);
                draft_on_bracket(hull, target_volume, trim, heel_deg, (d - w).max(0.0), d + w)
            })
            .or_else(|| solve_draft(hull, target_volume, trim, heel_deg))?;
        let (_, b) = hull.buoyancy(draft, trim, heel_deg);
        Some((draft, b.x - lcg_x))
    };

    let mut t0 = trim_hint.clamp(-max_trim, max_trim);
    let (d0, mut r0) = sample(t0, None)?;
    if r0.abs() < 1e-6 {
        return Some(FloatingCondition {
            draft: d0,
            trim: t0,
        });
    }
    // Probe in one direction, then the other if the first is blocked by the
    // clamp, to get a secant pair.
    let step = (0.05 * hull.length).clamp(1e-3, max_trim);
    let mut t1 = (t0 + step).clamp(-max_trim, max_trim);
    if (t1 - t0).abs() < 1e-9 {
        t1 = (t0 - step).clamp(-max_trim, max_trim);
    }
    let (mut d1, mut r1) = sample(t1, Some(d0))?;
    let (mut lo, mut hi) = if r0 <= 0.0 {
        (t0, f64::NAN)
    } else {
        (f64::NAN, t0)
    };
    if r1 <= 0.0 {
        lo = t1;
    } else {
        hi = t1;
    }
    for _ in 0..30 {
        // Secant step with a bisection fallback when it leaves the bracket or
        // repeats a sample, so discretisation noise cannot stall the solve.
        let denom = r1 - r0;
        let mut t2 = if denom.abs() > 1e-12 {
            t1 - r1 * (t1 - t0) / denom
        } else {
            f64::NAN
        };
        if !t2.is_finite() || t2 <= -max_trim || t2 >= max_trim {
            t2 = if lo.is_finite() && hi.is_finite() {
                0.5 * (lo + hi)
            } else if lo.is_finite() {
                (lo + 0.25 * hull.length).min(max_trim)
            } else {
                (hi - 0.25 * hull.length).max(-max_trim)
            };
        }
        let (d2, r2) = sample(t2, Some(d1))?;
        if r2.abs() < 1e-6 {
            return Some(FloatingCondition {
                draft: d2,
                trim: t2,
            });
        }
        if r2 <= 0.0 {
            lo = t2;
        } else {
            hi = t2;
        }
        // Keep the bracket consistent with the pair we just used, then move on.
        if lo.is_finite() && hi.is_finite() && lo > hi {
            std::mem::swap(&mut lo, &mut hi);
        }
        if (t2 - t1).abs() < 1e-9 {
            break;
        }
        t0 = t1;
        r0 = r1;
        t1 = t2;
        r1 = r2;
        d1 = d2;
    }
    None
}

/// Longitudinal centre of buoyancy of the upright hull at `draft`. The natural
/// `LCG` for a free-to-trim solve when the model carries no mass distribution
/// of its own.
pub fn upright_lcb(hull: &Hull, draft: f64) -> f64 {
    hull.buoyancy(draft, 0.0, 0.0).1.x
}

/// Compute `KN` and `GZ` for every (heel, displacement) pair.
pub fn cross_curves(
    hull: &Hull,
    heels_deg: &[f64],
    displacements_t: &[f64],
    cond: &StabilityCondition,
    base: &HydroOptions,
) -> Vec<CrossCurve> {
    let mut out = Vec::with_capacity(heels_deg.len());
    for &heel in heels_deg {
        let mut points = Vec::with_capacity(displacements_t.len());
        for &disp_t in displacements_t {
            let target_volume = disp_t * 1000.0 / base.water_density;
            let solved = match cond.lcg_x {
                Some(lcg) => solve_floating(hull, target_volume, heel, lcg, cond.trim),
                None => solve_draft(hull, target_volume, cond.trim, heel).map(|draft| {
                    FloatingCondition {
                        draft,
                        trim: cond.trim,
                    }
                }),
            };
            let Some(fc) = solved else {
                points.push(CrossCurvePoint {
                    displacement_t: disp_t,
                    draft: f64::NAN,
                    trim_m: f64::NAN,
                    kn: f64::NAN,
                    gz: f64::NAN,
                });
                continue;
            };
            // Only the centre of buoyancy is needed, so take the fast path:
            // a full `hydrostatics` call would also chain the waterline and cut
            // every station, none of which a righting lever uses.
            let (_, b) = hull.buoyancy(fc.draft, fc.trim, heel);
            let wp = waterplane_for(hull.keel, hull.length, fc.draft, fc.trim, heel);
            let t = wp.n.cross(Vec3::new(1.0, 0.0, 0.0)).norm();
            let g = Vec3::new(0.0, 0.0, hull.keel + cond.kg);
            let gz = (b - g).dot(t);
            let kn = gz + cond.kg * deg_to_rad(heel).sin();
            points.push(CrossCurvePoint {
                displacement_t: disp_t,
                draft: fc.draft,
                trim_m: fc.trim,
                kn,
                gz,
            });
        }
        out.push(CrossCurve { heel_deg: heel, points });
    }
    out
}
