//! Cross curves of stability and righting levers.
//!
//! For a set of heel angles the hull is trimmed to a constant displacement by
//! solving for the draft, and the righting lever is measured from the keel.
//! This is the same quantity FREE!ship reports as `KN` (and derives `GZ` from
//! once `KG` is known): `GZ = KN - KG * sin(heel)`.

use crate::hydro::{waterplane_for, Hull, HydroOptions};
use crate::math::deg_to_rad;
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct CrossCurvePoint {
    /// Displacement in tonnes.
    pub displacement_t: f64,
    /// Draft required to float at this displacement and heel.
    pub draft: f64,
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

/// Find the draft at which the hull displaces `target_volume` (m3).
pub fn solve_draft(hull: &Hull, target_volume: f64, trim: f64, heel_deg: f64) -> Option<f64> {
    let mut lo = 0.0f64;
    let mut hi = (hull.model_max.z - hull.keel).max(0.1) * 2.0 + 1.0;
    // Expand until the hull floats the target displacement.
    for _ in 0..40 {
        if hull.displaced_volume(hi, trim, heel_deg) >= target_volume {
            break;
        }
        hi *= 1.6;
    }
    if hull.displaced_volume(hi, trim, heel_deg) < target_volume {
        return None;
    }
    for _ in 0..80 {
        let mid = 0.5 * (lo + hi);
        if hull.displaced_volume(mid, trim, heel_deg) < target_volume {
            lo = mid;
        } else {
            hi = mid;
        }
        if (hi - lo) < 1e-9 {
            break;
        }
    }
    Some(0.5 * (lo + hi))
}

/// Compute `KN` and `GZ` for every (heel, displacement) pair.
pub fn cross_curves(
    hull: &Hull,
    heels_deg: &[f64],
    displacements_t: &[f64],
    kg: f64,
    trim: f64,
    base: &HydroOptions,
) -> Vec<CrossCurve> {
    let mut out = Vec::with_capacity(heels_deg.len());
    for &heel in heels_deg {
        let mut points = Vec::with_capacity(displacements_t.len());
        for &disp_t in displacements_t {
            let target_volume = disp_t * 1000.0 / base.water_density;
            let draft = match solve_draft(hull, target_volume, trim, heel) {
                Some(d) => d,
                None => {
                    points.push(CrossCurvePoint {
                        displacement_t: disp_t,
                        draft: f64::NAN,
                        kn: f64::NAN,
                        gz: f64::NAN,
                    });
                    continue;
                }
            };
            let mut opts = base.clone();
            opts.draft = draft;
            opts.trim = trim;
            opts.heel_deg = heel;
            let hydro = hull.hydrostatics(&opts);
            let b = hydro.lcb;
            let wp = waterplane_for(hull.keel, hull.length, draft, trim, heel);
            let t = wp.n.cross(crate::math::Vec3::new(1.0, 0.0, 0.0)).norm();
            let g = crate::math::Vec3::new(0.0, 0.0, hull.keel + kg);
            let gz = (b - g).dot(t);
            let kn = gz + kg * deg_to_rad(heel).sin();
            points.push(CrossCurvePoint {
                displacement_t: disp_t,
                draft,
                kn,
                gz,
            });
        }
        out.push(CrossCurve { heel_deg: heel, points });
    }
    out
}
