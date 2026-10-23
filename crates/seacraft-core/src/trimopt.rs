//! Trim optimisation: which trim is cheapest at a speed, at the same weight.
//!
//! A hull at a fixed displacement can be trimmed over a range — fuel, ballast
//! and cargo move, the skipper adjusts — and the resistance at a given speed
//! changes with it. This module scans that range and reports the curve, so the
//! answer is visible rather than just asserted:
//!
//! * For every trim, the draft is **solved** so the displacement holds. Trimming
//!   about the centre of flotation changes the immersed volume by itself; a scan
//!   that ignored that would compare a light hull with a heavy one and call the
//!   difference a trim effect.
//! * Each point is measured with the same code as `seacraft resistance`
//!   ([`resistance::HoltropInput::measure`] plus [`resistance::holtrop`]) on the
//!   refined mesh, so a scan cannot disagree with the single-speed command.
//!
//! What it does not claim: Holtrop–Mennen was fitted to even-keel hulls, so a
//! deep trim is an extrapolation. The scan reports the trims it used and the
//! model's own warnings; a minimum at the edge of the range is reported as such
//! and not as an optimum.

use serde::Serialize;

use crate::hydro::{Hull, HydroOptions};
use crate::mesh::Surface;
use crate::resistance::{self, HoltropInput};
use crate::subdiv::SubdivisionOptions;

/// One trim, at the draft that holds the displacement.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct TrimPoint {
    pub trim_m: f64,
    pub draft_m: f64,
    pub volume_m3: f64,
    pub lwl_m: f64,
    pub froude: f64,
    /// Total still-water resistance.
    pub rt_n: f64,
    /// Effective power, `RT · V`, in kilowatts.
    pub pe_kw: f64,
}

/// The whole scan.
#[derive(Debug, Clone, Serialize)]
pub struct TrimScan {
    pub speed_kn: f64,
    /// Displacement held, in cubic metres.
    pub target_volume_m3: f64,
    pub points: Vec<TrimPoint>,
    /// The cheapest trim in the scan; `None` only when no point could be measured.
    pub best: Option<TrimPoint>,
    /// True when the best point is an end of the range, i.e. not an interior
    /// minimum.
    pub at_boundary: bool,
    pub warnings: Vec<String>,
}

/// Drafts from `lo` to `hi` in `steps` equal steps, always including zero.
pub fn trim_range(from: f64, to: f64, steps: usize) -> Vec<f64> {
    let n = steps.max(2);
    (0..n)
        .map(|i| from + (to - from) * i as f64 / (n - 1) as f64)
        .collect()
}

/// Draft that displaces `target` at this trim, by bisection on a monotone
/// volume-versus-draft curve.
fn draft_for_volume(hull: &Hull, opts: &HydroOptions, trim: f64, target: f64) -> Option<f64> {
    let mut o = opts.clone();
    o.trim = trim;
    o.stations = 0;
    let mut lo = 0.005;
    let mut hi = (hull.model_max.z - hull.keel).max(0.01);
    o.draft = hi;
    if hull.hydrostatics(&o).volume_m3 < target {
        return None;
    }
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        o.draft = mid;
        let v = hull.hydrostatics(&o).volume_m3;
        if v < target {
            lo = mid;
        } else {
            hi = mid;
        }
        if hi - lo < 1e-9 {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Scan `trims` at `speed_kn`, holding the displacement given by `opts`.
///
/// With `hold_volume == false` the draft is left alone and only the trim
/// changes, which compares hulls of different weight — useful for a quick look,
/// and labelled as such in the report.
pub fn scan(
    surface: &Surface,
    subdiv: SubdivisionOptions,
    opts: &HydroOptions,
    speed_kn: f64,
    trims: &[f64],
    hold_volume: bool,
) -> Result<TrimScan, String> {
    if !(speed_kn.is_finite() && speed_kn > 0.0) {
        return Err(format!("trim: a speed of {speed_kn} kn is not positive"));
    }
    if trims.len() < 2 {
        return Err("trim: give at least two trims to scan".to_string());
    }
    let hull = Hull::new(surface, subdiv);
    let mut base = opts.clone();
    base.stations = 0;
    let target = hull.hydrostatics(&base).volume_m3;
    if target <= 0.0 {
        return Err("trim: the hull displaces nothing at this draft".to_string());
    }
    let speed_m_s = speed_kn * 0.514_444_444_444_444_4;
    let mut points = Vec::with_capacity(trims.len());
    let mut errors = Vec::new();
    for &trim in trims {
        let mut o = base.clone();
        o.trim = trim;
        if hold_volume {
            match draft_for_volume(&hull, &base, trim, target) {
                Some(d) => o.draft = d,
                None => {
                    errors.push(format!(
                        "trim {trim:+.3} m: the hull cannot carry the displacement"
                    ));
                    continue;
                }
            }
        }
        let Some(input) = HoltropInput::measure(&hull, &o) else {
            errors.push(format!("trim {trim:+.3} m: no waterline to measure"));
            continue;
        };
        let r = resistance::holtrop(&input, speed_m_s);
        points.push(TrimPoint {
            trim_m: trim,
            draft_m: o.draft,
            volume_m3: input.volume_m3,
            lwl_m: input.length_wl,
            froude: r.fn_,
            rt_n: r.rt_n,
            pe_kw: r.pe_w / 1000.0,
        });
    }
    if points.is_empty() {
        return Err(format!(
            "trim: nothing could be measured ({})",
            errors.join("; ")
        ));
    }
    let mut best = points[0];
    for p in &points {
        if p.rt_n < best.rt_n {
            best = *p;
        }
    }
    let at_boundary = points.len() > 1
        && (best.trim_m <= points[0].trim_m + 1e-9
            || best.trim_m >= points[points.len() - 1].trim_m - 1e-9);
    let mut warnings = Vec::new();
    if at_boundary {
        warnings.push(format!(
            "the cheapest trim ({:+.3} m) is at the edge of the range; widen it with --from/--to",
            best.trim_m
        ));
    }
    if points.iter().any(|p| p.trim_m.abs() > 0.5) {
        warnings.push(
            "Holtrop-Mennen was fitted to even-keel hulls; a trim beyond 0.5 m is an extrapolation"
                .to_string(),
        );
    }
    warnings.extend(errors);
    Ok(TrimScan {
        speed_kn,
        target_volume_m3: target,
        points,
        best: Some(best),
        at_boundary,
        warnings,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    fn opts(s: &Surface) -> HydroOptions {
        let mut o = HydroOptions::from_surface(
            s,
            SubdivisionOptions {
                level: 3,
                mode: Default::default(),
            },
        );
        o.draft = 0.62;
        o
    }

    #[test]
    fn zero_trim_reproduces_the_single_speed_command() {
        let s = demo::demo_hull(false);
        let o = opts(&s);
        let scan = scan(&s, o.subdivision, &o, 10.0, &[-0.3, 0.0, 0.3], true).expect("scans");
        let level = scan
            .points
            .iter()
            .find(|p| p.trim_m.abs() < 1e-12)
            .expect("the even-keel point");
        // The same measurement path, so the numbers must agree to the last bit
        // of the reported precision.
        let hull = Hull::new(&s, o.subdivision);
        let input = HoltropInput::measure(&hull, &o).expect("measures");
        let direct = resistance::holtrop(&input, 10.0 * 0.514_444_444_444_444_4);
        assert!(
            (level.rt_n - direct.rt_n).abs() < 1e-3,
            "{} vs {}",
            level.rt_n,
            direct.rt_n
        );
        assert!((level.volume_m3 - scan.target_volume_m3).abs() < 1e-6);
        // And with the CLI's own numbers for this hull (35.79 kW at 10 kn).
        assert!((level.pe_kw - 35.79).abs() < 0.1, "{:.2} kW", level.pe_kw);
    }

    #[test]
    fn the_displacement_holds_across_the_scan() {
        let s = demo::demo_hull(false);
        let o = opts(&s);
        let result =
            scan(&s, o.subdivision, &o, 10.0, &trim_range(-0.4, 0.4, 5), true).expect("scans");
        assert_eq!(result.points.len(), 5);
        for p in &result.points {
            assert!(
                (p.volume_m3 - result.target_volume_m3).abs() / result.target_volume_m3 < 1e-4,
                "trim {:+.2}: volume {:.4} against {:.4}",
                p.trim_m,
                p.volume_m3,
                result.target_volume_m3
            );
            assert!(p.rt_n > 0.0 && p.pe_kw > 0.0);
        }
        // Trimming needs a different draft to hold the same volume.
        let first = result.points.first().expect("a point");
        let last = result.points.last().expect("a point");
        assert!(
            (first.draft_m - last.draft_m).abs() > 1e-3,
            "{:?}",
            result.points
        );
    }

    #[test]
    fn the_best_point_is_the_cheapest_of_the_curve() {
        let s = demo::demo_hull(false);
        let o = opts(&s);
        let scan =
            scan(&s, o.subdivision, &o, 10.0, &trim_range(-0.3, 0.3, 7), true).expect("scans");
        let best = scan.best.expect("a best point");
        let min = scan
            .points
            .iter()
            .map(|p| p.rt_n)
            .fold(f64::INFINITY, f64::min);
        assert!((best.rt_n - min).abs() < 1e-9);
        // On this hull the cheapest trim is the aft end of the range: trimming
        // bow-down lengthens the waterline (10.70 m even keel, 11.27 m at
        // -0.30 m), which cuts the Froude number more than it costs in wave
        // resistance. The flag must say so rather than dress it up as an
        // interior optimum.
        assert!(scan.at_boundary, "{best:?}");
        let zero = scan
            .points
            .iter()
            .find(|p| p.trim_m.abs() < 1e-12)
            .expect("even keel");
        let bow_up = scan.points.last().expect("a point");
        assert!(bow_up.lwl_m < zero.lwl_m, "bow-up shortens the waterline");
        assert!(best.lwl_m > zero.lwl_m, "bow-down lengthens it");
    }

    #[test]
    fn an_edge_minimum_is_flagged_and_bad_speeds_are_refused() {
        let s = demo::demo_hull(false);
        let o = opts(&s);
        // A range entirely forward of the optimum must report the edge.
        let edge =
            scan(&s, o.subdivision, &o, 10.0, &trim_range(0.4, 0.6, 3), true).expect("scans");
        assert!(edge.at_boundary, "{edge:?}");
        assert!(edge
            .warnings
            .iter()
            .any(|w| w.contains("edge of the range")));
        assert!(scan(&s, o.subdivision, &o, 0.0, &[-0.1, 0.1], true).is_err());
        assert!(scan(&s, o.subdivision, &o, 10.0, &[0.0], true).is_err());
    }

    #[test]
    fn trim_range_includes_both_ends() {
        let t = trim_range(-0.4, 0.4, 5);
        assert_eq!(t.len(), 5);
        assert!((t[0] + 0.4).abs() < 1e-12 && (t[4] - 0.4).abs() < 1e-12);
        assert!(t.windows(2).all(|w| w[1] > w[0]));
        assert_eq!(trim_range(-0.4, 0.4, 0).len(), 2, "at least two points");
    }
}
