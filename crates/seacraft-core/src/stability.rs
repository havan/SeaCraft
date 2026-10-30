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

pub use crate::weather::{
    evaluate_hull_weather, evaluate_weather, hull_windage, hull_windage_on_hull, k_factor,
    roll_period, rollback_angle_deg, s_factor, sample_weather_gz, x1_factor, x2_factor, BilgeKind,
    WeatherHydro, WeatherInput, WeatherResult, WindageItem, DEFAULT_PHI0_LIMIT_DEG,
    DEFAULT_WIND_PRESSURE_PA, GRAVITY_M_S2,
};

#[derive(Debug, Clone, Serialize)]
pub struct CrossCurvePoint {
    /// Displacement in tonnes.
    pub displacement_t: f64,
    /// Draft required to float at this displacement and heel.
    pub draft: Option<f64>,
    /// Trim required to float at this displacement and heel, in metres and
    /// positive bow-up. Equal to the requested trim unless the curve is free
    /// to trim.
    pub trim_m: Option<f64>,
    /// Righting lever from the keel, in metres.
    pub kn: Option<f64>,
    /// Righting lever from the centre of gravity, in metres.
    pub gz: Option<f64>,
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
/// The displacements a cross-curve table uses when none are given: half,
/// all and 125 % of the design displacement, unrounded so a boat under a
/// tonne still gets three positive rows.
pub fn default_displacements(design_t: f64) -> Vec<f64> {
    vec![0.5 * design_t, design_t, 1.25 * design_t]
}

pub fn upright_lcb(hull: &Hull, draft: f64) -> f64 {
    hull.buoyancy(draft, 0.0, 0.0).1.x
}

/// Measure a single floating condition free to trim so `LCB.x` matches `lcg_x`
/// (or the upright `LCB` at `opts.draft` when `lcg_x` is `None`), keeping the
/// displacement equal to the upright displacement at `opts.draft`.
pub fn hydrostatics_free_trim(
    hull: &Hull,
    opts: &HydroOptions,
    lcg_x: Option<f64>,
) -> (crate::hydro::Hydrostatics, FloatingCondition) {
    let (target_volume, upright_b) = hull.buoyancy(opts.draft, 0.0, 0.0);
    let target_lcg = lcg_x.unwrap_or(upright_b.x);
    let fc = if target_volume > 1e-12 {
        solve_floating(hull, target_volume, opts.heel_deg, target_lcg, opts.trim).unwrap_or(
            FloatingCondition {
                draft: opts.draft,
                trim: opts.trim,
            },
        )
    } else {
        FloatingCondition {
            draft: opts.draft,
            trim: opts.trim,
        }
    };
    let mut solved_opts = opts.clone();
    solved_opts.draft = fc.draft;
    solved_opts.trim = fc.trim;
    (hull.hydrostatics(&solved_opts), fc)
}

/// Compute `KN` and `GZ` for every (heel, displacement) pair.
///
/// The pairs are independent solves and run on every available core; each is
/// computed by the same code as [`cross_curves_serial`], so the two return
/// the same numbers bit for bit.
pub fn cross_curves(
    hull: &Hull,
    heels_deg: &[f64],
    displacements_t: &[f64],
    cond: &StabilityCondition,
    base: &HydroOptions,
) -> Vec<CrossCurve> {
    let pairs: Vec<(f64, f64)> = heels_deg
        .iter()
        .flat_map(|&heel| displacements_t.iter().map(move |&d| (heel, d)))
        .collect();
    let points = crate::par::par_map(&pairs, |&(heel, disp_t)| {
        cross_curve_point(hull, heel, disp_t, cond, base)
    });
    group_cross_curves(heels_deg, displacements_t.len(), points)
}

/// [`cross_curves`] on the calling thread only, one pair after another (the
/// reference the parallel version is checked against).
pub fn cross_curves_serial(
    hull: &Hull,
    heels_deg: &[f64],
    displacements_t: &[f64],
    cond: &StabilityCondition,
    base: &HydroOptions,
) -> Vec<CrossCurve> {
    let points = heels_deg
        .iter()
        .flat_map(|&heel| {
            displacements_t
                .iter()
                .map(move |&disp_t| cross_curve_point(hull, heel, disp_t, cond, base))
        })
        .collect();
    group_cross_curves(heels_deg, displacements_t.len(), points)
}

/// Split the heel-major list of points into one curve per heel.
fn group_cross_curves(
    heels_deg: &[f64],
    per_heel: usize,
    points: Vec<CrossCurvePoint>,
) -> Vec<CrossCurve> {
    let mut points = points.into_iter();
    heels_deg
        .iter()
        .map(|&heel| CrossCurve {
            heel_deg: heel,
            points: points.by_ref().take(per_heel).collect(),
        })
        .collect()
}

/// `KN` and `GZ` at one heel and displacement (`None`s when no floating
/// condition is found or the lever is not finite).
fn cross_curve_point(
    hull: &Hull,
    heel: f64,
    disp_t: f64,
    cond: &StabilityCondition,
    base: &HydroOptions,
) -> CrossCurvePoint {
    let none = CrossCurvePoint {
        displacement_t: disp_t,
        draft: None,
        trim_m: None,
        kn: None,
        gz: None,
    };
    // The hull's own volume carries Δ / appendage coefficient, as in
    // `hydro::compute` and the loading-condition solve.
    let target_volume =
        disp_t * 1000.0 / (base.water_density * base.appendage_coefficient.max(1e-6));
    let solved = match cond.lcg_x {
        Some(lcg) => solve_floating(hull, target_volume, heel, lcg, cond.trim),
        None => solve_draft(hull, target_volume, cond.trim, heel).map(|draft| FloatingCondition {
            draft,
            trim: cond.trim,
        }),
    };
    let Some(fc) = solved else {
        return none;
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
    if gz.is_finite() && kn.is_finite() {
        CrossCurvePoint {
            displacement_t: disp_t,
            draft: Some(fc.draft),
            trim_m: Some(fc.trim),
            kn: Some(kn),
            gz: Some(gz),
        }
    } else {
        none
    }
}

/// IMO A.749 / 2008 IS Code general intact stability criteria for a GZ curve.
#[derive(Debug, Clone, Serialize)]
pub struct ImoCriteria {
    pub displacement_t: f64,
    /// Area under the GZ curve from 0 to 30 deg (m·rad); IMO minimum 0.055.
    pub area_0_30_m_rad: f64,
    /// Area under the GZ curve from 0 to 40 deg (m·rad); IMO minimum 0.090.
    pub area_0_40_m_rad: f64,
    /// Area under the GZ curve from 30 to 40 deg (m·rad); IMO minimum 0.030.
    pub area_30_40_m_rad: f64,
    /// Maximum GZ at heel >= 30 deg (m); IMO minimum 0.20.
    pub max_gz_30_plus_m: f64,
    /// Angle of maximum GZ (deg); IMO minimum 25.0.
    pub angle_of_max_gz_deg: f64,
    /// Initial metacentric height GM0 (m); IMO minimum 0.15. Estimated from the GZ slope
    /// over the first heel step unless set by [`ImoCriteria::with_metacentric_gm`].
    pub gm0_m: f64,
    /// Whether all six criteria pass.
    pub passed: bool,
}

/// `GZ` at an arbitrary heel from a heel-sorted tabulated curve, linearly
/// interpolated, clamped to the ends.
fn gz_at(pts: &[(f64, f64)], heel_deg: f64) -> f64 {
    let Some(&(h0, g0)) = pts.first() else {
        return 0.0;
    };
    if heel_deg <= h0 {
        return g0;
    }
    for w in pts.windows(2) {
        let ((ha, ga), (hb, gb)) = (w[0], w[1]);
        if heel_deg <= hb {
            if hb <= ha {
                return gb;
            }
            let t = (heel_deg - ha) / (hb - ha);
            return ga + t * (gb - ga);
        }
    }
    pts.last().map(|&(_, g)| g).unwrap_or(0.0)
}

/// Righting area between two heels, in m·rad. Negative levers contribute no
/// positive righting area, so the interpolated endpoints and every interior
/// sample are clamped at zero; the integration is trapezoidal in degrees.
///
/// The interval is clipped to the sampled heel range: past the last tabulated
/// heel there is no GZ curve, so a curve that stops at 20 deg contributes
/// nothing to the 30–40 deg area rather than a constant extension of its last
/// ordinate.
fn area_between(pts: &[(f64, f64)], a_deg: f64, b_deg: f64) -> f64 {
    if pts.len() < 2 || !a_deg.is_finite() || !b_deg.is_finite() || b_deg <= a_deg {
        return 0.0;
    }
    let lo = a_deg.max(pts[0].0);
    let hi = b_deg.min(pts[pts.len() - 1].0);
    if hi <= lo {
        return 0.0;
    }
    let mut samples: Vec<(f64, f64)> = Vec::with_capacity(pts.len() + 2);
    samples.push((lo, gz_at(pts, lo).max(0.0)));
    samples.extend(
        pts.iter()
            .filter(|(h, _)| *h > lo && *h < hi)
            .map(|&(h, g)| (h, g.max(0.0))),
    );
    samples.push((hi, gz_at(pts, hi).max(0.0)));
    let mut area = 0.0;
    for w in samples.windows(2) {
        area += 0.5 * (w[0].1 + w[1].1) * (w[1].0 - w[0].0);
    }
    area * std::f64::consts::PI / 180.0
}

/// Fit a parabola through three points and return the vertex `(x, y)`.
///
/// Assumes `p1.0 < p2.0 < p3.0` and `p2.1 >= p1.1`, `p2.1 >= p3.1`. If the
/// points do not form a downward parabola or the vertex falls outside
/// `[p1.0, p3.0]`, falls back to `p2`.
pub fn fit_parabola_vertex(p1: (f64, f64), p2: (f64, f64), p3: (f64, f64)) -> (f64, f64) {
    let (x1, y1) = p1;
    let (x2, y2) = p2;
    let (x3, y3) = p3;
    let u1 = x1 - x2;
    let u3 = x3 - x2;
    if u1.abs() < 1e-12 || u3.abs() < 1e-12 {
        return (x2, y2);
    }
    let d1 = (y1 - y2) / u1;
    let d3 = (y3 - y2) / u3;
    let denom = u3 - u1;
    if denom.abs() < 1e-12 {
        return (x2, y2);
    }
    let a = (d3 - d1) / denom;
    let b = d3 - a * u3;
    if a < -1e-12 {
        let u_v = -b / (2.0 * a);
        if u_v >= u1 && u_v <= u3 {
            let y_v = y2 - b * b / (4.0 * a);
            return (x2 + u_v, y_v);
        }
    }
    (x2, y2)
}

/// Evaluate the IMO A.749 / 2008 IS Code Part A general intact stability
/// criteria on the `disp_index`-th point of every heel curve.
///
/// The curve must reach at least 30 deg for the 30–40 deg area and the
/// "maximum GZ above 30 deg" checks to mean anything; a shorter curve simply
/// fails them (the area past its end contributes nothing).
pub fn evaluate_imo_criteria(curves: &[CrossCurve], disp_index: usize) -> Option<ImoCriteria> {
    let mut pts: Vec<(f64, f64)> = Vec::new();
    let mut displacement_t = 0.0;
    for c in curves {
        let Some(p) = c.points.get(disp_index) else {
            continue;
        };
        let Some(gz) = p.gz else {
            continue;
        };
        if !gz.is_finite() || !c.heel_deg.is_finite() || c.heel_deg < 0.0 {
            continue;
        }
        if pts.is_empty() {
            displacement_t = p.displacement_t;
        }
        pts.push((c.heel_deg, gz));
    }
    if pts.len() < 2 {
        return None;
    }
    pts.sort_by(|a, b| a.0.total_cmp(&b.0));

    let area_0_30 = area_between(&pts, 0.0, 30.0);
    let area_0_40 = area_between(&pts, 0.0, 40.0);
    let area_30_40 = area_between(&pts, 30.0, 40.0);

    // Highest sample, earliest sample wins a tie (the conservative angle).
    let mut best_idx = 0;
    for (i, &(_, g)) in pts.iter().enumerate().skip(1) {
        if g > pts[best_idx].1 {
            best_idx = i;
        }
    }
    let (angle_of_max, peak_gz) = if best_idx > 0 && best_idx + 1 < pts.len() {
        fit_parabola_vertex(pts[best_idx - 1], pts[best_idx], pts[best_idx + 1])
    } else {
        pts[best_idx]
    };

    let (h_min, h_max) = (pts[0].0, pts[pts.len() - 1].0);
    let mut max_gz_30 = f64::NEG_INFINITY;
    for &(h, g) in &pts {
        if h >= 30.0 {
            max_gz_30 = max_gz_30.max(g);
        }
    }
    if h_min <= 30.0 && h_max >= 30.0 {
        max_gz_30 = max_gz_30.max(gz_at(&pts, 30.0));
    }
    if angle_of_max >= 30.0 {
        max_gz_30 = max_gz_30.max(peak_gz);
    }
    let max_gz_30 = if max_gz_30.is_finite() {
        max_gz_30
    } else {
        0.0
    };

    // Slope at the origin: the standard `GM0 ≈ dGZ/dheel` at zero heel.
    let mut gm0 = 0.0;
    if let Some(w) = pts.windows(2).find(|w| w[1].0 > w[0].0) {
        gm0 = (w[1].1 - w[0].1) / deg_to_rad(w[1].0 - w[0].0);
    }

    let mut c = ImoCriteria {
        displacement_t,
        area_0_30_m_rad: area_0_30,
        area_0_40_m_rad: area_0_40,
        area_30_40_m_rad: area_30_40,
        max_gz_30_plus_m: max_gz_30,
        angle_of_max_gz_deg: angle_of_max,
        gm0_m: gm0,
        passed: false,
    };
    c.passed = c.all_pass();
    Some(c)
}

impl ImoCriteria {
    /// All six A.749 criteria.
    fn all_pass(&self) -> bool {
        self.area_0_30_m_rad >= 0.055
            && self.area_0_40_m_rad >= 0.090
            && self.area_30_40_m_rad >= 0.030
            && self.max_gz_30_plus_m >= 0.20
            && self.angle_of_max_gz_deg >= 25.0
            && self.gm0_m >= 0.15
    }

    /// Use a known metacentric height (`KM − KG`, free-surface corrected)
    /// for the GM0 criterion instead of the GZ secant over the first heel
    /// step, which reads low on a curve that bends over early.
    pub fn with_metacentric_gm(mut self, gm0_m: f64) -> Self {
        if gm0_m.is_finite() {
            self.gm0_m = gm0_m;
            self.passed = self.all_pass();
        }
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::Hull;

    #[test]
    fn cross_curves_float_the_hull_volume_without_the_appendages() {
        // Δ at an appendage coefficient of 1.05 must sink the bare hull as far
        // as Δ / 1.05 does with none, as in `hydro::compute` and loading.
        let surface = crate::demo::demo_hull(false);
        let hull = Hull::new(&surface, Default::default());
        let mut base = HydroOptions::from_surface(&surface, Default::default());
        let cond = StabilityCondition::fixed(1.0, 0.0);
        base.appendage_coefficient = 1.05;
        let with = cross_curves(&hull, &[0.0], &[12.0], &cond, &base);
        base.appendage_coefficient = 1.0;
        let bare = cross_curves(&hull, &[0.0], &[12.0 / 1.05], &cond, &base);
        let (a, b) = (
            with[0].points[0].draft.unwrap(),
            bare[0].points[0].draft.unwrap(),
        );
        assert!((a - b).abs() < 1e-6, "{a} vs {b}");
    }

    /// The parallel cross curves are the serial ones bit for bit, fixed
    /// and free to trim (round-5 decision: `to_bits`, not a tolerance).
    #[test]
    fn parallel_cross_curves_equal_the_serial_ones_bit_for_bit() {
        let surface = crate::demo::demo_hull(false);
        let hull = Hull::new(&surface, Default::default());
        let base = HydroOptions::from_surface(&surface, Default::default());
        let heels: Vec<f64> = (0..=8).map(|i| i as f64 * 10.0).collect();
        let disps = [8.0, 10.0, 12.0, 14.0, 1.0e6];
        let bits = |v: Option<f64>| v.map(f64::to_bits);
        for lcg in [None, Some(upright_lcb(&hull, 0.62))] {
            let cond = StabilityCondition {
                kg: 2.2,
                trim: 0.0,
                lcg_x: lcg,
            };
            let par = cross_curves(&hull, &heels, &disps, &cond, &base);
            let ser = cross_curves_serial(&hull, &heels, &disps, &cond, &base);
            assert_eq!(par.len(), ser.len());
            for (a, b) in par.iter().zip(&ser) {
                assert_eq!(a.heel_deg.to_bits(), b.heel_deg.to_bits());
                assert_eq!(a.points.len(), b.points.len());
                for (p, q) in a.points.iter().zip(&b.points) {
                    assert_eq!(p.displacement_t.to_bits(), q.displacement_t.to_bits());
                    assert_eq!(bits(p.draft), bits(q.draft), "draft at {}°", a.heel_deg);
                    assert_eq!(bits(p.trim_m), bits(q.trim_m), "trim at {}°", a.heel_deg);
                    assert_eq!(bits(p.kn), bits(q.kn), "KN at {}°", a.heel_deg);
                    assert_eq!(bits(p.gz), bits(q.gz), "GZ at {}°", a.heel_deg);
                }
            }
            // The realistic displacements solved.
            assert!(par[1].points[2].gz.is_some());
        }
    }

    #[test]
    fn a_metacentric_gm_replaces_the_secant_and_decides_the_gm0_criterion() {
        let c = ImoCriteria {
            displacement_t: 10.0,
            area_0_30_m_rad: 0.1,
            area_0_40_m_rad: 0.2,
            area_30_40_m_rad: 0.05,
            max_gz_30_plus_m: 0.3,
            angle_of_max_gz_deg: 30.0,
            gm0_m: 0.10,
            passed: false,
        };
        let ok = c.clone().with_metacentric_gm(0.40);
        assert_eq!(ok.gm0_m, 0.40);
        assert!(ok.passed);
        let low = c.clone().with_metacentric_gm(0.12);
        assert!(!low.passed);
        assert_eq!(c.with_metacentric_gm(f64::NAN).gm0_m, 0.10);
    }
    use crate::subdiv::SubdivisionOptions;

    fn point(heel_deg: f64, gz: f64) -> CrossCurve {
        CrossCurve {
            heel_deg,
            points: vec![CrossCurvePoint {
                displacement_t: 20.0,
                draft: Some(0.5),
                trim_m: Some(0.0),
                kn: Some(gz + 2.0 * deg_to_rad(heel_deg).sin()),
                gz: Some(gz),
            }],
        }
    }

    #[test]
    fn free_trim_hydrostatics_put_the_lcb_on_the_lcg() {
        let surface = crate::demo::demo_hull(false);
        let opts = HydroOptions {
            draft: 0.62,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1025.0,
            appendage_coefficient: 1.0,
            mainframe_x: None,
            stations: 11,
            subdivision: SubdivisionOptions {
                level: 2,
                mode: Default::default(),
            },
        };
        let hull = Hull::new(&surface, opts.subdivision);
        let (upright_volume, upright_b) = hull.buoyancy(opts.draft, 0.0, 0.0);

        // `None` must reproduce the upright condition: LCB at the upright LCB,
        // displacement at the upright displacement.
        let (h, fc) = hydrostatics_free_trim(&hull, &opts, None);
        assert!(fc.trim.abs() < 1e-3, "upright trim {}", fc.trim);
        assert!((fc.draft - opts.draft).abs() < 1e-3, "draft {}", fc.draft);
        assert!(
            (h.lcb.x - upright_b.x).abs() < 1e-3,
            "LCB {} vs upright {}",
            h.lcb.x,
            upright_b.x
        );
        assert!(
            (h.volume_m3 - upright_volume).abs() < 1e-3 * upright_volume,
            "volume {} vs {}",
            h.volume_m3,
            upright_volume
        );

        // A centre of gravity forward of the upright LCB must be reached by
        // trimming, and the reported LCB has to actually match it.
        let lcg = upright_b.x + 0.30;
        let (h, fc) = hydrostatics_free_trim(&hull, &opts, Some(lcg));
        assert!(
            (h.lcb.x - lcg).abs() < 1e-3,
            "LCB {} did not reach the requested {lcg} (trim {})",
            h.lcb.x,
            fc.trim
        );
        assert!(
            (h.volume_m3 - upright_volume).abs() < 1e-3 * upright_volume,
            "volume changed with trim: {} vs {}",
            h.volume_m3,
            upright_volume
        );
        assert!((h.draft - fc.draft).abs() < 1e-12);
        assert!((h.trim_m - fc.trim).abs() < 1e-12);
    }

    #[test]
    fn imo_areas_are_exact_for_a_linear_gz_curve() {
        // GZ = k * heel has an analytic area k/2 (b^2 - a^2) in metre-radians,
        // which the trapezoid reproduces exactly because the curve is linear.
        let k = 0.01;
        let curves: Vec<CrossCurve> = (0..=9)
            .map(|i| point(10.0 * i as f64, k * 10.0 * i as f64))
            .collect();
        let c = evaluate_imo_criteria(&curves, 0).expect("criteria");

        let rad = std::f64::consts::PI / 180.0;
        let want = |a: f64, b: f64| 0.5 * k * (b * b - a * a) * rad;
        assert!((c.area_0_30_m_rad - want(0.0, 30.0)).abs() < 1e-12);
        assert!((c.area_0_40_m_rad - want(0.0, 40.0)).abs() < 1e-12);
        assert!((c.area_30_40_m_rad - want(30.0, 40.0)).abs() < 1e-12);
        assert!((c.max_gz_30_plus_m - k * 90.0).abs() < 1e-12);
        assert!((c.angle_of_max_gz_deg - 90.0).abs() < 1e-12);
        // Slope at the origin in radians: k * 180/pi.
        assert!((c.gm0_m - k / rad).abs() < 1e-12, "GM0 {}", c.gm0_m);
        assert!((c.displacement_t - 20.0).abs() < 1e-12);
        assert!(c.passed, "linear GZ {k} should satisfy the criteria: {c:?}");
    }

    #[test]
    fn imo_interpolates_between_samples_and_clamps_negative_levers() {
        // Samples only at 0, 45 and 90 deg, so 30 and 40 deg are interpolated
        // ordinates and 45 deg is the only interior sample.
        let curves = vec![point(0.0, 0.0), point(45.0, 0.40), point(90.0, -0.10)];
        let c = evaluate_imo_criteria(&curves, 0).expect("criteria");
        let rad = std::f64::consts::PI / 180.0;
        let gz30 = 0.40 * 30.0 / 45.0;
        let gz40 = 0.40 * 40.0 / 45.0;
        let trap = |a: f64, ga: f64, b: f64, gb: f64| 0.5 * (ga + gb) * (b - a) * rad;

        assert!((c.area_0_30_m_rad - trap(0.0, 0.0, 30.0, gz30)).abs() < 1e-12);
        // No interior sample lies inside [0, 40]: the whole interval is one
        // trapezoid between the interpolated endpoints.
        assert!((c.area_0_40_m_rad - trap(0.0, 0.0, 40.0, gz40)).abs() < 1e-12);
        assert!((c.area_30_40_m_rad - trap(30.0, gz30, 40.0, gz40)).abs() < 1e-12);
        assert!((c.max_gz_30_plus_m - (0.40 + 1.0 / 720.0)).abs() < 1e-12);
        assert!((c.angle_of_max_gz_deg - 42.5).abs() < 1e-12);
        // GM0 from the first two samples: (0.40 - 0.0) / 45 deg in radians.
        assert!((c.gm0_m - 0.40 / deg_to_rad(45.0)).abs() < 1e-12);
        assert!(c.passed, "{c:?}");

        // A negative lever contributes no righting area: the -0.10 m sample at
        // 60 deg is clamped to zero rather than subtracting from the integral.
        let negative = vec![
            point(0.0, 0.0),
            point(30.0, 0.25),
            point(40.0, 0.10),
            point(60.0, -0.10),
        ];
        let c = evaluate_imo_criteria(&negative, 0).expect("criteria");
        let clamped_trap = 0.5 * (0.25 + 0.10) * 10.0 * rad;
        assert!((c.area_30_40_m_rad - clamped_trap).abs() < 1e-12);
        // Past the end of the table the ordinate is clamped, never extrapolated.
        assert!((gz_at(&[(0.0, 0.0), (45.0, 0.30)], 90.0) - 0.30).abs() < 1e-12);
    }

    #[test]
    fn imo_flags_a_failing_curve() {
        // Same shape, half the lever: areas fall below the minima even though
        // the angle of maximum GZ is fine.
        let k = 0.002;
        let curves: Vec<CrossCurve> = (0..=9)
            .map(|i| point(10.0 * i as f64, k * 10.0 * i as f64))
            .collect();
        let c = evaluate_imo_criteria(&curves, 0).expect("criteria");
        assert!(c.area_0_30_m_rad < 0.055, "{c:?}");
        assert!(!c.passed);
        // GM0 is 0.002/rad = 0.1146 m, below the 0.15 m minimum, so that
        // criterion alone would also fail.
        assert!(c.gm0_m < 0.15, "GM0 {}", c.gm0_m);

        // A curve that never reaches 30 deg fails the 30-40 deg checks.
        let short = vec![point(0.0, 0.0), point(10.0, 0.1), point(20.0, 0.2)];
        let c = evaluate_imo_criteria(&short, 0).expect("criteria");
        assert_eq!(c.max_gz_30_plus_m, 0.0);
        assert_eq!(c.area_30_40_m_rad, 0.0);
        assert!(!c.passed);

        // Too few points is not a curve.
        assert!(evaluate_imo_criteria(&[point(0.0, 0.0)], 0).is_none());
        assert!(evaluate_imo_criteria(&[], 0).is_none());
        // A missing displacement index skips every curve.
        assert!(evaluate_imo_criteria(&curves, 7).is_none());
    }
}
