//! Lackenby-style station shift: reshape the sectional area curve in place.
//!
//! Scaling a hull (see [`crate::transform`]) changes its size; it does not
//! change its *distribution* of volume. Moving a hull's prismatic coefficient
//! and longitudinal centre of buoyancy to chosen values without altering the
//! section shapes is A. J. Lackenby's 1957 method: shift every station
//! longitudinally by `δ(x)`, keeping the ends fixed, so the sectional area
//! curve is stretched where it is fat and compressed where it is fine.
//!
//! # What is implemented, and what is not
//!
//! Lackenby tabulates two polynomials that are solving aids for the linearised
//! problem. This crate does not reproduce those tables from memory — a
//! mis-remembered coefficient would look plausible and be wrong. Instead the
//! shift is written as two basis functions with the same *shape* and the
//! coefficients are **solved numerically**, so the hull that comes out really
//! does measure at the requested `Cp` and `LCB`:
//!
//! ```text
//! δ(x) = L · [ a · f_cp(t) + b · f_lcb(t) ],   t = (x - x0) / L  in [0, 1]
//! f_cp(t)  = t (1 - t)                 symmetric: fills or empties the middle
//! f_lcb(t) = t (1 - t) (2t - 1)        antisymmetric: moves volume forward or aft
//! ```
//!
//! Both vanish at `t = 0` and `t = 1`, so the stem and the transom stay put and
//! the waterline length is preserved to first order. `a` and `b` are found by a
//! damped Newton iteration on the two measured responses, and every evaluation
//! is a real hydrostatic computation on the refined mesh — not an integral of
//! the same control net the shift was applied to.
//!
//! # Authority, measured, not assumed
//!
//! Two knobs with the ends pinned are a *local* adjustment. On the 12 m demo
//! hull at its design draft (Cp 0.7548, LCB -9.85 %) `0.005` of `Cp` and
//! `0.5 %` of `LWL` in `LCB` are reached in two or three passes, and asking for
//! `0.03` of `Cp` stops at `0.7364` — volume down 2.3 %, waterline down 1.1 %,
//! `converged: false`, with the residual in the report. That is the honest
//! behaviour: a bigger move means changing the ends, i.e. inverting the
//! sectional area curve properly, which is a different (and much larger)
//! algorithm and is on the backlog rather than half-implemented here.
//!
//! What the method cannot do at all: add volume. `Cp` up with `Am` and `L`
//! unchanged means more displacement, and the sections are what they are.

use serde::Serialize;

use crate::hydro::{self, HydroOptions};
use crate::mesh::Surface;

/// What the hull should measure afterwards.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct LackenbyTarget {
    /// Target prismatic coefficient, `V / (Am · LWL)`.
    pub cp: f64,
    /// Target `LCB` as a percentage of `LWL`, positive forward of midships.
    pub lcb_pct: f64,
}

impl LackenbyTarget {
    pub fn new(cp: f64, lcb_pct: f64) -> Self {
        Self { cp, lcb_pct }
    }
}

/// The state of the hull before and after the shift.
#[derive(Debug, Clone, Serialize)]
pub struct LackenbyReport {
    pub target: LackenbyTarget,
    pub before: HullState,
    pub after: HullState,
    /// Solved coefficients of the two basis functions.
    pub coefficients: (f64, f64),
    /// Station shift at midships, in metres (the largest the basis allows).
    pub midship_shift_m: f64,
    pub iterations: usize,
    pub converged: bool,
    /// `after - before` residual on the two targets, so a caller can judge it.
    pub cp_error: f64,
    pub lcb_error: f64,
}

/// The two numbers the shift is fitted to, plus what they cost.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct HullState {
    pub cp: f64,
    pub lcb_pct: f64,
    pub volume_m3: f64,
    pub lwl: f64,
    pub lcb_x: f64,
}

impl HullState {
    fn measure(h: &hydro::Hydrostatics, x0: f64, lwl: f64) -> Self {
        Self {
            cp: h.prismatic_coefficient,
            lcb_pct: if lwl > 1e-9 {
                (h.lcb.x - x0) / lwl * 100.0 - 50.0
            } else {
                0.0
            },
            volume_m3: h.volume_m3,
            lwl,
            lcb_x: h.lcb.x,
        }
    }
}

/// `δ(x)/L` for the two basis functions, at `t` clamped to `[0, 1]`.
fn basis(t: f64) -> (f64, f64) {
    let t = t.clamp(0.0, 1.0);
    let hump = t * (1.0 - t);
    (hump, hump * (2.0 * t - 1.0))
}

/// Apply a shift of `(a, b)` to a copy of `surface`, measured from `x0` with
/// length `lwl`. Points outside the submerged run are left where they are.
pub fn shift(surface: &Surface, a: f64, b: f64, x0: f64, lwl: f64) -> Surface {
    let mut out = surface.clone();
    if a == 0.0 && b == 0.0 {
        return out;
    }
    for p in out.points.iter_mut() {
        let t = (p.p.x - x0) / lwl;
        if !(-1e-9..=1.0 + 1e-9).contains(&t) {
            continue;
        }
        let (f_cp, f_lcb) = basis(t);
        p.p.x += lwl * (a * f_cp + b * f_lcb);
    }
    out
}

/// Solve `(a, b)` so the hull measures at `target`, then return it.
///
/// `opts` supplies the draft, trim, heel and subdivision; only `cp` and
/// `lcb_pct` of the target are used. The iteration is damped Newton on a
/// finite-difference Jacobian: four measurements per pass, at most `max_iter`
/// passes, stopping at `tol` (in `Cp` and percentage points of `LWL`).
pub fn fit(
    surface: &Surface,
    target: LackenbyTarget,
    opts: &HydroOptions,
    tol: f64,
    max_iter: usize,
) -> Result<(Surface, LackenbyReport), String> {
    if !(0.30..=0.95).contains(&target.cp) {
        return Err(format!(
            "Lackenby: a target Cp of {:.3} is outside 0.30..0.95",
            target.cp
        ));
    }
    if target.lcb_pct.abs() > 10.0 {
        return Err(format!(
            "Lackenby: a target LCB of {:+.2} % LWL is outside +/-10 %",
            target.lcb_pct
        ));
    }
    let measure = |s: &Surface| -> Result<(HullState, f64, f64), String> {
        let mut o = opts.clone();
        o.stations = o.stations.max(21);
        let h = hydro::compute(s, &o);
        if h.volume_m3.abs() < 1e-9 {
            return Err("Lackenby: the hull displaces nothing at this draft".into());
        }
        Ok((HullState::measure(&h, h.wl_aft_x, h.lwl), h.wl_aft_x, h.lwl))
    };

    let (before, x0, lwl) = measure(surface)?;
    if lwl < 1e-6 {
        return Err("Lackenby: the hull has no waterline length".into());
    }
    let residual = |state: &HullState| (state.cp - target.cp, state.lcb_pct - target.lcb_pct);

    // A 1e-3 step moves the mesh by ~3 mm and changes Cp by less than the
    // station-sampling noise, so the finite differences are taken with a step
    // the measurement can actually resolve, and centrally to cancel the bias.
    let eps = 5e-3;
    let mut a = 0.0_f64;
    let mut b = 0.0_f64;
    let mut iterations = 0;
    let mut converged = false;
    let mut after = before;
    let mut best = (
        residual(&before).0.abs().max(residual(&before).1.abs()),
        0.0,
        0.0,
    );

    for pass in 0..max_iter {
        iterations = pass + 1;
        let (r1, r2) = residual(&after);
        if r1.abs() <= tol && r2.abs() <= tol {
            converged = true;
            break;
        }
        // Central differences on the two coefficients, from the *original*
        // surface, so `a` and `b` stay absolute rather than incremental.
        let probe = |da: f64, db: f64| -> Result<(f64, f64), String> {
            let s = shift(surface, a + da, b + db, x0, lwl);
            let (state, _, _) = measure(&s)?;
            Ok(residual(&state))
        };
        let (ap, bp) = probe(eps, 0.0)?;
        let (am, bm) = probe(-eps, 0.0)?;
        let (cp, dp) = probe(0.0, eps)?;
        let (cm, dm) = probe(0.0, -eps)?;
        let j11 = (ap - am) / (2.0 * eps);
        let j21 = (bp - bm) / (2.0 * eps);
        let j12 = (cp - cm) / (2.0 * eps);
        let j22 = (dp - dm) / (2.0 * eps);
        let det = j11 * j22 - j12 * j21;
        if !det.is_finite() || det.abs() < 1e-9 {
            return Err(format!(
                "Lackenby: the response is singular at Cp {:.4}, LCB {:+.2} %; \
                 the targets are not reachable by a station shift",
                after.cp, after.lcb_pct
            ));
        }
        let da = -(j22 * r1 - j12 * r2) / det;
        let db = -(-j21 * r1 + j11 * r2) / det;
        // Line search: a full step can overshoot into a folded curve, so halve
        // until the residual stops getting worse.
        let mut scale = 1.0;
        let mut accepted = None;
        for _ in 0..6 {
            let (na, nb) = (a + da * scale, b + db * scale);
            if na.abs() > 1.0 || nb.abs() > 1.0 {
                scale *= 0.5;
                continue;
            }
            let s = shift(surface, na, nb, x0, lwl);
            let (state, _, _) = measure(&s)?;
            let (e1, e2) = residual(&state);
            let err = e1.abs().max(e2.abs());
            if err < best.0 {
                accepted = Some((na, nb, state, err));
                break;
            }
            scale *= 0.5;
        }
        match accepted {
            Some((na, nb, state, err)) => {
                a = na;
                b = nb;
                after = state;
                best = (err, a, b);
            }
            // No step improves the fit any further.
            None => break,
        }
    }
    if residual(&after).0.abs() <= tol && residual(&after).1.abs() <= tol {
        converged = true;
    }

    let out = shift(surface, a, b, x0, lwl);
    let (final_state, _, _) = measure(&out)?;
    let (cp_error, lcb_error) = residual(&final_state);
    let report = LackenbyReport {
        target,
        before,
        after: final_state,
        coefficients: (a, b),
        midship_shift_m: lwl * 0.25 * a,
        iterations,
        converged,
        cp_error,
        lcb_error,
    };
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::subdiv::SubdivisionOptions;

    fn opts() -> HydroOptions {
        let s = demo::demo_hull(false);
        let mut o = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 3,
                mode: Default::default(),
            },
        );
        o.stations = 41;
        o
    }

    #[test]
    fn basis_vanishes_at_both_ends() {
        assert_eq!(basis(0.0), (0.0, 0.0));
        assert_eq!(basis(1.0), (0.0, 0.0));
        assert!(basis(0.5).0 > 0.0);
        assert!(basis(0.4).1 < 0.0 && basis(0.6).1 > 0.0, "antisymmetric");
    }

    #[test]
    fn zero_shift_is_the_identity() {
        let s = demo::demo_hull(false);
        let out = shift(&s, 0.0, 0.0, s.extents().0.x, 10.0);
        for (a, b) in s.points.iter().zip(out.points.iter()) {
            assert_eq!(a.p, b.p);
        }
    }

    #[test]
    fn the_ends_and_length_survive_a_shift() {
        let s = demo::demo_hull(false);
        let o = opts();
        let h = hydro::compute(&s, &o);
        let out = shift(&s, 0.02, 0.01, h.wl_aft_x, h.lwl);
        let h2 = hydro::compute(&out, &o);
        assert!(
            (h2.lwl - h.lwl).abs() / h.lwl < 0.005,
            "LWL moved {:.4} -> {:.4}",
            h.lwl,
            h2.lwl
        );
    }

    #[test]
    fn a_hull_with_overhangs_is_measured_on_its_waterline() {
        // The yacht has a counter stern: its waterline starts forward of the
        // model's aft end. Lackenby must measure LCB from the waterline, as
        // the resistance methods do, and shift the whole run up to the bow.
        use crate::parametric::{generate, HullKind, HullSpec};
        let spec = HullSpec::default_for(HullKind::Yacht);
        let (s, _) = generate(&spec).expect("yacht");
        let mut o = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: Default::default(),
            },
        );
        o.draft = spec.draft;
        o.stations = 41;
        let h = hydro::compute(&s, &o);
        assert!(
            h.wl_aft_x > h.model_min.x + 0.2,
            "the yacht should overhang aft: wl {} vs model {}",
            h.wl_aft_x,
            h.model_min.x
        );
        let lcb_pct = |h: &hydro::Hydrostatics| (h.lcb.x - h.wl_aft_x) / h.lwl * 100.0 - 50.0;
        let target = LackenbyTarget::new(h.prismatic_coefficient + 0.01, lcb_pct(&h) + 0.5);
        let (out, report) = fit(&s, target, &o, 1e-3, 8).expect("fits");
        assert!(report.converged, "{report:?}");
        let mut o2 = o.clone();
        o2.stations = o.stations.max(21);
        let after = hydro::compute(&out, &o2);
        assert!(
            (lcb_pct(&after) - target.lcb_pct).abs() < 0.05,
            "LCB {} vs target {}",
            lcb_pct(&after),
            target.lcb_pct
        );
        assert!((after.prismatic_coefficient - target.cp).abs() < 2e-3);
    }

    #[test]
    fn same_target_returns_the_same_hull() {
        let s = demo::demo_hull(false);
        let o = opts();
        let h = hydro::compute(&s, &o);
        let target = LackenbyTarget::new(h.prismatic_coefficient, 0.0);
        let lcb_pct = (h.lcb.x - h.wl_aft_x) / h.lwl * 100.0 - 50.0;
        let target = LackenbyTarget::new(target.cp, lcb_pct);
        let (_, report) = fit(&s, target, &o, 1e-3, 4).expect("fits");
        assert!(report.converged);
        assert!(
            report.coefficients.0.abs() < 1e-3,
            "{:?}",
            report.coefficients
        );
        assert!(
            report.coefficients.1.abs() < 1e-3,
            "{:?}",
            report.coefficients
        );
    }

    #[test]
    fn it_reaches_a_new_prismatic_coefficient() {
        let s = demo::demo_hull(false);
        let o = opts();
        let h = hydro::compute(&s, &o);
        let lcb_pct = (h.lcb.x - h.wl_aft_x) / h.lwl * 100.0 - 50.0;
        // Inside the basis's authority (see the module header): a 0.5 % nudge.
        let target = LackenbyTarget::new(h.prismatic_coefficient - 0.005, lcb_pct);
        let (out, report) = fit(&s, target, &o, 1e-3, 10).expect("fits");
        assert!(report.converged, "{report:?}");
        assert!(
            (report.after.cp - target.cp).abs() < 1e-3,
            "Cp {:.4} for a target of {:.4}",
            report.after.cp,
            target.cp
        );
        // The trimmed hull is a different hull, not a rescaled copy.
        let h2 = hydro::compute(&out, &o);
        assert!((h2.lwl - h.lwl).abs() / h.lwl < 0.01);
        assert!(h2.volume_m3 > 0.0);
    }

    #[test]
    fn an_unreachable_target_is_reported_as_unreached() {
        // A 2 % Cp change is beyond what two station-shift knobs do without
        // folding the entrance; the fit must say so rather than claim success.
        let s = demo::demo_hull(false);
        let o = opts();
        let h = hydro::compute(&s, &o);
        let lcb_pct = (h.lcb.x - h.wl_aft_x) / h.lwl * 100.0 - 50.0;
        let target = LackenbyTarget::new(h.prismatic_coefficient - 0.03, lcb_pct);
        let (out, report) = fit(&s, target, &o, 1e-3, 8).expect("a result, honestly labelled");
        assert!(!report.converged, "{report:?}");
        assert!(report.cp_error.abs() > 5e-3, "{report:?}");
        // Whatever it did produce is still a hull: positive volume, sane LWL.
        let h2 = hydro::compute(&out, &o);
        assert!(
            h2.volume_m3 > 0.5 * h.volume_m3,
            "volume {:.3}",
            h2.volume_m3
        );
        assert!((h2.lwl - h.lwl).abs() / h.lwl < 0.05);
    }

    #[test]
    fn it_moves_the_longitudinal_centre_of_buoyancy() {
        let s = demo::demo_hull(false);
        let o = opts();
        let h = hydro::compute(&s, &o);
        let lcb_pct = (h.lcb.x - h.wl_aft_x) / h.lwl * 100.0 - 50.0;
        let target = LackenbyTarget::new(h.prismatic_coefficient, lcb_pct + 0.5);
        let (_, report) = fit(&s, target, &o, 1e-2, 10).expect("fits");
        assert!(report.converged, "{report:?}");
        assert!(
            (report.after.lcb_pct - target.lcb_pct).abs() < 0.02,
            "LCB {:.3} % for a target of {:.3} %",
            report.after.lcb_pct,
            target.lcb_pct
        );
        assert!(report.after.lcb_x > report.before.lcb_x, "forward is +x");
    }

    #[test]
    fn impossible_targets_are_refused() {
        let s = demo::demo_hull(false);
        let o = opts();
        assert!(fit(&s, LackenbyTarget::new(0.10, 0.0), &o, 1e-3, 3).is_err());
        assert!(fit(&s, LackenbyTarget::new(0.70, 40.0), &o, 1e-3, 3).is_err());
    }
}
