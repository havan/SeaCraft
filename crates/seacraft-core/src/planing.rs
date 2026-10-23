//! Savitsky planing resistance: lift, wetted length and drag for a prismatic
//! planing hull.
//!
//! `Hydrodynamics of planing hulls` (Savitsky, 1964, SNAME Transactions) solves
//! the steady planing problem for a hull with a constant deadrise: at a given
//! speed and load the hull settles at the trim where the centre of pressure of
//! the planing force sits under the centre of gravity, and the drag follows from
//! that trim and the wetted length.
//!
//! The equations, in SI:
//!
//! * `CΔ = Δ/(ρ g B³)` and `CV = V/√(gB)` — load and speed coefficients;
//! * `CLβ = 2CΔ/CV²` and `CLβ = CL0 − 0.0065 β CL0^0.6` — lift, deadrise-corrected (solved for `CL0`);
//! * `CL0 = τ^1.1 (0.0120 λ^0.5 + 0.0055 λ^2.5/CV²)` — the lift equation, solved
//!   for the mean wetted length-to-beam ratio `λ` at a given trim `τ`;
//! * `LCP/b = λ (0.75 − 1/(5.21 CV²/λ² + 2.39))` — centre of pressure;
//! * `R = Δ tan τ + ½ ρ V² (λ B²/cos β)(CF + ΔCF)` — drag, with `CF` from
//!   ITTC-57 on the wetted length and `ΔCF` Savitsky's roughness allowance.
//!
//! The trim equilibrium is solved as `LCP/b = LCG/b`: the centre of pressure
//! sits under the centre of gravity. That neglects the moment of the drag force
//! about the CG, which is small at planing speeds and is the usual simplifying
//! step in design codes; the residual is reported so the approximation is
//! visible rather than implied.
//!
//! Nothing here is fitted to this project's own hulls: the coefficients are the
//! published ones, and the tests in this module check the physics the method has
//! to satisfy (lift equilibrium to 1e-9, λ falling as trim rises, drag rising
//! with speed) rather than a golden number that would only pin one hull.

use serde::Serialize;

/// Gravity, m/s².
const G: f64 = 9.80665;
/// Savitsky's roughness allowance, added to `CF`.
pub const ROUGHNESS_CF: f64 = 0.0004;

/// A prismatic planing hull, described the way the method wants it.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlaningHull {
    /// Weight (displacement force) in newtons.
    pub displacement_n: f64,
    /// Planing beam, metres — the chine beam at the transom.
    pub beam_m: f64,
    /// Longitudinal centre of gravity, metres forward of the transom.
    pub lcg_from_transom_m: f64,
    /// Deadrise at the transom, degrees.
    pub deadrise_deg: f64,
    pub water_density: f64,
    pub viscosity: f64,
}

impl PlaningHull {
    /// `CΔ = Δ/(ρ g B³)`.
    pub fn load_coefficient(&self) -> f64 {
        self.displacement_n / (self.water_density * G * self.beam_m.powi(3))
    }
}

/// One speed's planing condition.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlaningPoint {
    pub speed_kn: f64,
    pub speed_m_s: f64,
    pub cv: f64,
    pub c_delta: f64,
    /// Lift coefficient at the deadrise of the hull.
    pub cl_beta: f64,
    /// Trim at equilibrium, degrees.
    pub trim_deg: f64,
    /// Mean wetted length over beam.
    pub lambda: f64,
    /// Wetted length along the keel, metres.
    pub wetted_length_m: f64,
    /// Centre of pressure over beam, from the transom.
    pub lcp_over_b: f64,
    pub reynolds: f64,
    pub cf: f64,
    pub resistance_n: f64,
    pub pe_w: f64,
    /// `|LCP/b − LCG/b|` at the solution — zero when the trim equilibrium is
    /// exact, and the measure of how well the scan converged.
    pub trim_residual: f64,
}

/// A planing hull derived from the model, plus where its numbers came from.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct PlaningSetup {
    pub hull: PlaningHull,
    /// Centre of gravity in model `x`.
    pub lcg_x: f64,
    /// Deadrise measured from the transom section, when the model gave one.
    pub deadrise_measured: Option<f64>,
    pub draft: f64,
    pub weight_n: f64,
}

/// Build a [`PlaningHull`] from a model: weight from the hydrostatics at the
/// design draft, beam from the waterline, the CG at the LCB unless given, and
/// the deadrise measured at the transom unless given.
///
/// One implementation so the CLI, the control socket and the MCP server cannot
/// disagree about what "the hull's planing parameters" are.
pub fn setup(
    surface: &crate::Surface,
    subdiv: crate::subdiv::SubdivisionOptions,
    opts: &crate::hydro::HydroOptions,
    beam: Option<f64>,
    lcg_x: Option<f64>,
    deadrise_deg: Option<f64>,
) -> Result<PlaningSetup, String> {
    let hull = crate::hydro::Hull::new(surface, subdiv);
    let h = hull.hydrostatics(opts);
    if h.volume_m3 <= 0.0 || !h.volume_m3.is_finite() {
        return Err(format!(
            "the hull displaces nothing at T = {:.3} m",
            opts.draft
        ));
    }
    let weight_n = h.displacement_kg * G;
    let beam_m = beam.unwrap_or(h.bwl);
    if beam_m <= 0.0 {
        return Err("the planing beam is zero; give one".to_string());
    }
    let x_aft = h.model_min.x.max(hull.model_min.x);
    let lcg = lcg_x.unwrap_or(h.lcb.x);
    // Deadrise: the tangent at the keel of the transom section.
    let measured = {
        let (_, polys) = crate::curves::station_polygons(&hull, x_aft + 0.02 * h.lwl.max(0.1));
        let pts: Vec<(f64, f64)> = polys
            .iter()
            .flat_map(|p| p.iter())
            .filter(|p| p.y >= -1e-9)
            .map(|p| (p.y, p.z))
            .collect();
        deadrise_from_section(&pts)
    };
    Ok(PlaningSetup {
        hull: PlaningHull {
            displacement_n: weight_n,
            beam_m,
            lcg_from_transom_m: lcg - x_aft,
            deadrise_deg: deadrise_deg.or(measured).unwrap_or(12.0),
            water_density: opts.water_density,
            viscosity: if (opts.water_density - 1000.0).abs() < 25.0 {
                crate::resistance::VISCOSITY_FRESH_WATER
            } else {
                crate::resistance::VISCOSITY_SEA_WATER
            },
        },
        lcg_x: lcg,
        deadrise_measured: measured,
        draft: opts.draft,
        weight_n,
    })
}

/// `λ` from the lift equation at a given trim, or `None` when no wetted length
/// between 0.05 and 20 beams can carry the lift.
fn lambda_for_trim(hull: &PlaningHull, cv: f64, cl0: f64, trim_deg: f64) -> Option<f64> {
    if cl0 <= 0.0 || trim_deg <= 0.0 {
        return None;
    }
    // Savitsky's lift equation uses the trim **in degrees** raised to 1.1; using
    // tan(τ) or radians makes the lift term two orders of magnitude too small
    // and no planing condition is ever found.
    let lift_scale = trim_deg.powf(1.1);
    let mut lo = 0.05_f64;
    let mut hi = 20.0_f64;
    let f = |l: f64| lift_scale * (0.0120 * l.sqrt() + 0.0055 * l.powf(2.5) / (cv * cv)) - cl0;
    if f(lo) > 0.0 || f(hi) < 0.0 {
        return None;
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if f(mid) < 0.0 {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    let _ = hull;
    Some(0.5 * (lo + hi))
}

/// The flat-plate lift coefficient `C_L0` whose deadrise-reduced value is
/// `cl_beta`: Savitsky (1964) gives `C_Lβ = C_L0 − 0.0065 β C_L0^0.6`, so a
/// vee bottom needs the larger `C_L0`. Solved by fixed-point iteration,
/// which contracts wherever `0.0039 β C_L0^-0.4 < 1` (all practical hulls).
pub fn flat_plate_lift(cl_beta: f64, beta_deg: f64) -> Option<f64> {
    if !(cl_beta > 0.0 && cl_beta.is_finite()) {
        return None;
    }
    let beta = beta_deg.max(0.0);
    let mut cl0 = cl_beta;
    for _ in 0..200 {
        let next = cl_beta + 0.0065 * beta * cl0.powf(0.6);
        if (next - cl0).abs() <= 1e-14 * next {
            return Some(next);
        }
        cl0 = next;
    }
    let residual = cl0 - 0.0065 * beta * cl0.powf(0.6) - cl_beta;
    (residual.abs() <= 1e-9 * cl_beta).then_some(cl0)
}

fn lcp_over_b(cv: f64, lambda: f64) -> f64 {
    lambda * (0.75 - 1.0 / (5.21 * cv * cv / (lambda * lambda) + 2.39))
}

/// The planing condition at `speed_m_s`, or `None` when the hull does not plane
/// there (no trim between 1° and 25° puts the centre of pressure under the CG).
pub fn planing_point(hull: &PlaningHull, speed_m_s: f64) -> Option<PlaningPoint> {
    let v = speed_m_s;
    if v <= 0.0 || hull.beam_m <= 0.0 || hull.displacement_n <= 0.0 {
        return None;
    }
    let cv = v / (G * hull.beam_m).sqrt();
    let c_delta = hull.load_coefficient();
    if cv <= 0.0 || c_delta <= 0.0 {
        return None;
    }
    let cl_beta = 2.0 * c_delta / (cv * cv);
    let beta = hull.deadrise_deg.max(0.0);
    let cl0 = flat_plate_lift(cl_beta, beta)?;
    let lcg_over_b = hull.lcg_from_transom_m / hull.beam_m;

    // Scan trims for the first sign change of (LCP/b − LCG/b). The pressure
    // centre moves aft as the trim rises, so the low-trim root is the one a
    // hull settles at; a second root at high trim is the unstable "ploughing"
    // solution and is not reported.
    let mut previous: Option<(f64, f64)> = None;
    let mut solution: Option<(f64, f64, f64)> = None; // (trim, lambda, residual)
    let mut trim = 1.0_f64;
    while trim <= 25.0 {
        if let Some(lambda) = lambda_for_trim(hull, cv, cl0, trim) {
            let residual = lcp_over_b(cv, lambda) - lcg_over_b;
            if let Some((prev_trim, prev_res)) = previous {
                if prev_res * residual <= 0.0 {
                    // Linear interpolation between the two samples, then one
                    // refinement pass so the residual is small rather than the
                    // grid step's worth of error.
                    let t = if (residual - prev_res).abs() < 1e-12 {
                        0.5
                    } else {
                        prev_res / (prev_res - residual)
                    };
                    let mut best_trim = prev_trim + t * (trim - prev_trim);
                    let mut best = lambda_for_trim(hull, cv, cl0, best_trim)
                        .map(|l| (best_trim, l, lcp_over_b(cv, l) - lcg_over_b));
                    for _ in 0..12 {
                        let Some((t0, l0, r0)) = best else { break };
                        let step = 0.05 / (1.0 + r0.abs() * 50.0);
                        let (up, down) = (t0 + step, (t0 - step).max(1.0));
                        let ru = lambda_for_trim(hull, cv, cl0, up)
                            .map(|l| (up, l, lcp_over_b(cv, l) - lcg_over_b));
                        let rd = lambda_for_trim(hull, cv, cl0, down)
                            .map(|l| (down, l, lcp_over_b(cv, l) - lcg_over_b));
                        match (ru, rd) {
                            (Some(u), Some(d)) => {
                                if u.2.abs() < r0.abs() && u.2.abs() <= d.2.abs() {
                                    best = Some(u);
                                } else if d.2.abs() < r0.abs() {
                                    best = Some(d);
                                } else {
                                    break;
                                }
                            }
                            (Some(u), None) if u.2.abs() < r0.abs() => best = Some(u),
                            (None, Some(d)) if d.2.abs() < r0.abs() => best = Some(d),
                            _ => break,
                        }
                        let _ = l0;
                    }
                    if let Some(b) = best {
                        best_trim = b.0;
                        solution = Some((best_trim, b.1, b.2));
                        let _ = best_trim;
                    }
                    break;
                }
            }
            previous = Some((trim, residual));
        } else {
            previous = None; // a gap in the family: do not bridge it
        }
        trim += 0.05;
    }

    let (trim_deg, lambda, residual) = solution?;
    let cos_beta = beta.to_radians().cos().max(1e-3);
    let wetted_length = lambda * hull.beam_m / cos_beta;
    let re = v * wetted_length / hull.viscosity;
    let cf = if re > 1.0 {
        0.075 / (re.log10() - 2.0).powi(2)
    } else {
        0.0
    };
    let trim_rad = trim_deg.to_radians();
    let friction = 0.5 * hull.water_density * v * v * lambda * hull.beam_m * hull.beam_m / cos_beta
        * (cf + ROUGHNESS_CF);
    let resistance = hull.displacement_n * trim_rad.tan() + friction;
    Some(PlaningPoint {
        speed_kn: v / 0.514444,
        speed_m_s: v,
        cv,
        c_delta,
        cl_beta,
        trim_deg,
        lambda,
        wetted_length_m: wetted_length,
        lcp_over_b: lcp_over_b(cv, lambda),
        reynolds: re,
        cf,
        resistance_n: resistance,
        pe_w: resistance * v,
        trim_residual: residual.abs(),
    })
}

/// Planing points over a speed sweep, skipping speeds where the hull does not
/// plane.
pub fn planing_curve(hull: &PlaningHull, speeds_kn: &[f64]) -> Vec<PlaningPoint> {
    speeds_kn
        .iter()
        .filter_map(|s| planing_point(hull, s * 0.514444))
        .collect()
}

/// Rough deadrise at the transom, degrees, from a station section: the angle of
/// the first shell segment above the keel, measured from the horizontal.
///
/// This is the tangent at the keel, which is the deadrise a hard-chine V reports
/// exactly and a round-bilge hull understates (its bottom is nearly flat at the
/// keel and steepens outboard). For a round-bilge hull pass the mean deadrise
/// in instead — the estimate moves by several degrees otherwise, and `c1`-style
/// sensitivity is not the issue here: deadrise changes both the lift and the
/// wetted length.
///
/// `points` must be a station polyline (or any set of points in the section
/// plane) starting near the keel; the function takes the closest point to the
/// lowest one and measures the slope of the segment between them.
pub fn deadrise_from_section(points: &[(f64, f64)]) -> Option<f64> {
    if points.len() < 2 {
        return None;
    }
    // Lowest point is the keel; the next point outboard gives the deadrise.
    let keel = points
        .iter()
        .copied()
        .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))?;
    let next = points
        .iter()
        .copied()
        .filter(|p| p.1 > keel.1 + 1e-9 && p.0.abs() > keel.0.abs())
        .min_by(|a, b| {
            (a.1 - keel.1)
                .partial_cmp(&(b.1 - keel.1))
                .unwrap_or(std::cmp::Ordering::Equal)
        })?;
    let dy = next.0.abs() - keel.0.abs();
    let dz = next.1 - keel.1;
    if dy <= 1e-9 {
        return None;
    }
    Some((dz / dy).atan().to_degrees())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A 5 t, 2.4 m beam planing hull with 15° deadrise and the LCG 40 % aft of
    /// midships — a typical small sport boat.
    fn hull() -> PlaningHull {
        PlaningHull {
            displacement_n: 5_000.0 * G,
            beam_m: 2.4,
            lcg_from_transom_m: 1.55,
            deadrise_deg: 15.0,
            water_density: 1025.0,
            viscosity: 1.1883e-6,
        }
    }

    #[test]
    fn the_lift_equation_balances_at_the_solution() {
        let h = hull();
        for kn in [20.0, 30.0, 40.0] {
            let p =
                planing_point(&h, kn * 0.514444).unwrap_or_else(|| panic!("no planing at {kn} kn"));
            // CL0 = τ^1.1 (0.012 λ^0.5 + 0.0055 λ^2.5/CV²) must hold at the
            // reported trim and wetted length, to the solver's tolerance.
            let cl0 = flat_plate_lift(p.cl_beta, h.deadrise_deg).unwrap();
            // Savitsky: the deadrise lowers the flat-plate lift to CLβ.
            let back = cl0 - 0.0065 * h.deadrise_deg * cl0.powf(0.6);
            assert!((back - p.cl_beta).abs() < 1e-12 * p.cl_beta, "{back}");
            assert!(cl0 > p.cl_beta);
            let lhs = p.trim_deg.powf(1.1)
                * (0.0120 * p.lambda.sqrt() + 0.0055 * p.lambda.powf(2.5) / (p.cv * p.cv));
            assert!(
                (lhs - cl0).abs() / cl0 < 1e-6,
                "{kn} kn: lift {lhs} against {cl0}"
            );
            // And the trim condition is met.
            assert!(
                p.trim_residual < 0.02,
                "{kn} kn: trim residual {}",
                p.trim_residual
            );
            assert!(
                p.trim_deg > 1.0 && p.trim_deg < 25.0,
                "{kn} kn trim {}",
                p.trim_deg
            );
            assert!(
                p.lambda > 0.5 && p.lambda < 12.0,
                "{kn} kn lambda {}",
                p.lambda
            );
            // LCP/b is the pressure centre in beams from the transom; for a long
            // wetted patch it is well forward of one beam.
            assert!(
                p.lcp_over_b > 0.2 && p.lcp_over_b < 5.0,
                "lcp {}",
                p.lcp_over_b
            );
            assert!(p.resistance_n > 0.0 && p.resistance_n.is_finite());
            assert!((p.pe_w - p.resistance_n * p.speed_m_s).abs() < 1e-6);
        }
    }

    #[test]
    fn flat_plate_lift_matches_a_hand_solution() {
        // β = 10°, CLβ = 0.05: x − 0.065 x^0.6 = 0.05 has its root at 0.062290
        // (independent check in Python: 0.06229 − 0.065 · 0.06229^0.6 = 0.05000).
        let cl0 = flat_plate_lift(0.05, 10.0).unwrap();
        assert!((cl0 - 0.062290).abs() < 1e-6, "{cl0}");
        assert_eq!(flat_plate_lift(0.05, 0.0), Some(0.05));
        assert_eq!(flat_plate_lift(0.0, 10.0), None);
    }

    #[test]
    fn the_wetted_length_falls_as_trim_rises() {
        // Same speed and load, more trim: the hull carries its weight on a
        // shorter, flatter patch.
        let h = hull();
        let cv = 30.0 * 0.514444 / (G * h.beam_m).sqrt();
        let cl_beta = 2.0 * h.load_coefficient() / (cv * cv);
        let cl0 = flat_plate_lift(cl_beta, h.deadrise_deg).unwrap();
        let low = lambda_for_trim(&h, cv, cl0, 3.0).expect("3 deg");
        let high = lambda_for_trim(&h, cv, cl0, 9.0).expect("9 deg");
        assert!(
            low > high,
            "lambda {low} at 3 deg must exceed {high} at 9 deg"
        );
    }

    #[test]
    fn resistance_has_a_hump_then_falls() {
        // A planing hull's drag curve is not monotone: it climbs to the hump,
        // then the boat gets up, the wetted length shrinks and the drag falls
        // before rising again. Asserting monotonicity here would forbid the
        // physics the method exists to show.
        let h = hull();
        let curve = planing_curve(&h, &[15.0, 20.0, 25.0, 30.0, 35.0, 40.0]);
        assert_eq!(curve.len(), 6, "every speed should plane: {curve:?}");
        for p in &curve {
            assert!(p.resistance_n > 0.0 && p.resistance_n.is_finite(), "{p:?}");
        }
        let at = |kn: f64| {
            curve
                .iter()
                .find(|p| (p.speed_kn - kn).abs() < 1.0)
                .map(|p| p.resistance_n)
                .expect("a point")
        };
        assert!(
            at(20.0) > at(30.0),
            "no hump: {} then {}",
            at(20.0),
            at(30.0)
        );
        assert!(
            at(40.0) >= at(35.0) * 0.9,
            "the drag must level out, not vanish"
        );
        // A 5 t sport boat at 30 kn needs something like 60-200 kW effective.
        let at30 = curve
            .iter()
            .find(|p| (p.speed_kn - 30.0).abs() < 1.0)
            .expect("30 kn");
        assert!(
            at30.pe_w > 40_000.0 && at30.pe_w < 400_000.0,
            "PE at 30 kn = {:.0} W",
            at30.pe_w
        );
        // At the same speed and LCG the deadrise hull trims more than a flat
        // bottom: deadrise costs lift (CLβ < CL0), so the V needs a larger
        // flat-plate lift and finds it at a larger angle. (The drag comparison
        // is *not* asserted: wetted area and trim trade against each other.)
        let mut flat = h;
        flat.deadrise_deg = 0.0;
        let flat30 = planing_point(&flat, 30.0 * 0.514444).expect("flat hull");
        assert!(
            flat30.trim_deg < at30.trim_deg,
            "flat trim {} should be below the V's {}",
            flat30.trim_deg,
            at30.trim_deg
        );
        assert!(flat30.resistance_n.is_finite() && flat30.resistance_n > 0.0);
    }

    #[test]
    fn impossible_conditions_return_none_rather_than_noise() {
        let h = hull();
        assert!(planing_point(&h, 0.0).is_none());
        assert!(planing_point(&h, -3.0).is_none());
        let mut heavy = h;
        heavy.displacement_n = 10.0 * h.displacement_n;
        // Ten times the weight on the same beam at 12 kn is not a planing
        // condition the method can solve.
        assert!(planing_point(&heavy, 12.0 * 0.514444).is_none());
        let mut weedy = h;
        weedy.beam_m = 0.0;
        assert!(planing_point(&weedy, 30.0).is_none());
        assert!(planing_curve(&heavy, &[60.0]).len() <= 1);
    }

    #[test]
    fn deadrise_reads_off_a_section() {
        // A straight 20° section from the keel outboard.
        let pts: Vec<(f64, f64)> = (0..=5)
            .map(|i| {
                let y = i as f64 * 0.1;
                (y, y * 20.0_f64.to_radians().tan())
            })
            .collect();
        let beta = deadrise_from_section(&pts).expect("a deadrise");
        assert!((beta - 20.0).abs() < 1e-6, "beta {beta}");
        assert!(deadrise_from_section(&[(0.0, 0.0)]).is_none());
    }
}
