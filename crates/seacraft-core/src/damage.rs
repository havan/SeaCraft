//! Deterministic damage stability by the lost-buoyancy method.
//!
//! Under the lost-buoyancy method:
//! * The flooded compartment volume below the current waterplane is removed from
//!   the buoyant hull, with permeability $\mu$ ($0 \le \mu \le 1$, default 0.95).
//! * Ship displacement $\Delta$ and centre of gravity $KG$ remain constant (the
//!   flooded water is treated as lost buoyancy, not added weight).
//! * Damaged equilibrium is solved for draft, trim (and heel/list if the damage
//!   is asymmetric) so that net buoyant volume $V_{net} = \Delta / \rho$ and
//!   net longitudinal centre of buoyancy $LCB_{net} = LCG$.
//! * Residual GZ curve is sampled from the damaged equilibrium (heels 0..60° at
//!   fine steps, free trim).
//! * Indicative SOLAS-style residual criteria (SOLAS II-1 Reg. 7-2 spirit) are
//!   evaluated: positive range $\ge 15^\circ$, max $GZ \ge 0.10$ m, area
//!   $\ge 0.015$ m·rad within the range.

use crate::curves::station_polygons;
use crate::hydro::{waterplane_at_offset, waterplane_for, Hull, HydroOptions};
use crate::loading::LoadingCondition;
use crate::math::{deg_to_rad, Plane, Vec3};
use crate::mesh::Surface;
use crate::stability::{solve_floating, FloatingCondition, MIN_COS_HEEL};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;

fn default_permeability() -> f64 {
    0.95
}

/// A damage compartment defined by an axis-aligned bounding box, structural
/// permeability, and optional hull clipping.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Compartment {
    pub name: String,
    pub x_min: f64,
    pub x_max: f64,
    pub y_min: f64,
    pub y_max: f64,
    pub z_min: f64,
    pub z_max: f64,
    /// Structural / floodable permeability ($\mu \in [0, 1]$, default 0.95).
    #[serde(default = "default_permeability")]
    pub permeability: f64,
    /// Intersect the bounding box with the hull's moulded volume when `true`.
    #[serde(default)]
    pub clip_to_hull: bool,
}

/// Specification of a compartment in a damage case: either inline or by name
/// (looked up in an associated loading condition's tanks).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum CompartmentSpec {
    Inline(Compartment),
    Named(String),
}

/// A complete damage case: name, compartments to flood, and loading particulars
/// (`kg`, `displacement_t`, optional `lcg`/`tcg`, or derived from a [`LoadingCondition`]).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Default)]
pub struct DamageCase {
    pub name: String,
    #[serde(default)]
    pub compartments: Vec<CompartmentSpec>,
    #[serde(default)]
    pub kg: Option<f64>,
    #[serde(default)]
    pub displacement_t: Option<f64>,
    #[serde(default)]
    pub lcg: Option<f64>,
    #[serde(default)]
    pub tcg: Option<f64>,
    /// A loading condition of the project, by name; [`crate::project::Project::resolve_damage_case`]
    /// fills `loading_condition` from it. As with an inline condition, `displacement_t`
    /// and `kg` set together override the condition's values.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub loading_condition_name: Option<String>,
    #[serde(default)]
    pub loading_condition: Option<LoadingCondition>,
    #[serde(default)]
    pub water_density: Option<f64>,
}

/// Evaluated summary of one flooded compartment at damaged equilibrium.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompartmentSummary {
    pub name: String,
    pub permeability: f64,
    pub clip_to_hull: bool,
    /// Moulded volume of the compartment below the damaged waterplane (m³).
    pub submerged_volume_m3: f64,
    /// Net lost buoyancy volume ($\mu \cdot \text{submerged\_volume\_m3}$, m³).
    pub lost_volume_m3: f64,
    /// Centroid of the submerged compartment volume in model coordinates (m).
    pub centroid: Vec3,
}

/// One ordinate of the residual righting lever curve from damaged equilibrium.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DamageGzPoint {
    /// Absolute heel angle in degrees (+ to starboard).
    pub heel_deg: f64,
    /// Relative heel from damaged equilibrium ($\ge 0^\circ$).
    pub relative_heel_deg: f64,
    /// Solved draft required to float at this heel (m).
    pub draft: Option<f64>,
    /// Solved trim required for longitudinal equilibrium (m, + bow-up).
    pub trim_m: Option<f64>,
    /// Residual righting lever righting towards equilibrium (m).
    pub gz: Option<f64>,
}

/// Indicative residual stability criteria in the spirit of SOLAS II-1 Reg. 7-2.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ResidualCriteria {
    /// Angle of equilibrium heel in degrees.
    pub equilibrium_heel_deg: f64,
    /// Range of positive residual righting levers from equilibrium to vanishing angle (deg); minimum 15.0°.
    pub positive_range_deg: f64,
    /// Maximum residual righting lever (m); minimum 0.10 m.
    pub max_gz_m: f64,
    /// Angle of maximum residual GZ relative to equilibrium (deg).
    pub angle_of_max_gz_deg: f64,
    /// Area under the positive residual GZ curve within the positive range (m·rad); minimum 0.015 m·rad.
    pub area_m_rad: f64,
    /// Whether all three criteria pass.
    pub passed: bool,
    /// Explanatory status label.
    pub label: String,
}

/// Full damage stability evaluation result.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct DamageResult {
    pub case_name: String,
    /// Ship displacement held constant (tonnes).
    pub displacement_t: f64,
    /// Vertical centre of gravity above keel held constant (m).
    pub kg: f64,
    /// Longitudinal centre of gravity held constant (m).
    pub lcg: f64,
    /// Transverse centre of gravity held constant (m, + to starboard).
    pub tcg: f64,
    /// Water density used (t/m³).
    pub water_density: f64,
    /// Intact upright equilibrium draft (m).
    pub intact_draft: f64,
    /// Intact upright equilibrium trim (m).
    pub intact_trim_m: f64,
    /// Whether a stable damaged equilibrium condition was found.
    pub equilibrium_found: bool,
    /// Damaged equilibrium draft (m).
    pub draft: f64,
    /// Damaged equilibrium trim (m, + bow-up).
    pub trim_m: f64,
    /// Damaged equilibrium heel / list angle (deg, + to starboard).
    pub heel_deg: f64,
    /// Equilibrium sinkage ($T_{damaged} - T_{intact}$, m).
    pub sinkage_m: f64,
    /// Minimum freeboard to the margin / deck edge at equilibrium (m), if determinable.
    pub freeboard_m: Option<f64>,
    /// Net buoyant volume at equilibrium (m³).
    pub net_volume_m3: f64,
    /// Total flooded water volume inside compartments below the damaged waterline (m³).
    pub lost_volume_m3: f64,
    /// Net longitudinal centre of buoyancy (m).
    pub lcb_x: f64,
    /// Net transverse centre of buoyancy (m).
    pub tcb_y: f64,
    /// Net vertical centre of buoyancy above keel (m).
    pub kb: f64,
    /// Breakdown of flooded compartments.
    pub compartments: Vec<CompartmentSummary>,
    /// Residual GZ curve sampled from damaged equilibrium.
    pub residual_gz: Vec<DamageGzPoint>,
    /// Indicative SOLAS-style residual criteria summary.
    pub criteria: Option<ResidualCriteria>,
    /// Non-fatal warnings emitted during evaluation.
    pub warnings: Vec<String>,
}

/// Validate a [`DamageCase`] before evaluation.
pub fn validate(case: &DamageCase) -> Result<(), String> {
    if case.compartments.is_empty() {
        return Err("damage case must contain at least one compartment".into());
    }
    if let Some(ref lc_name) = case.loading_condition_name {
        if case.loading_condition.is_none() {
            return Err(format!(
                "damage case `{}` refers to loading condition `{}` by name; evaluate it through a project file",
                case.name, lc_name
            ));
        }
    }
    validate_entries(case)
}

/// The checks of [`validate`] that hold for a case stored in a project as
/// well as one about to be evaluated: everything but "at least one
/// compartment" (a case being built in the design dock has none yet) and a
/// loading condition given only by name (the project resolves it).
pub(crate) fn validate_entries(case: &DamageCase) -> Result<(), String> {
    if case.name.trim().is_empty() {
        return Err("damage case name must not be empty".into());
    }
    for spec in &case.compartments {
        if let CompartmentSpec::Inline(c) = spec {
            validate_compartment(c)?;
        }
    }
    if let Some(lc) = &case.loading_condition {
        crate::loading::validate(lc).map_err(|e| format!("loading condition: {e}"))?;
    }
    if let Some(kg) = case.kg {
        if !kg.is_finite() || kg < 0.0 {
            return Err("`kg` must be finite and non-negative".into());
        }
    }
    if let Some(disp) = case.displacement_t {
        if !disp.is_finite() || disp <= 0.0 {
            return Err("`displacement_t` must be finite and positive".into());
        }
    }
    if let Some(density) = case.water_density {
        // Tonnes per m³ here (1.025 for sea water); 1025 would be kg/m³ and
        // shrink the displaced volume a thousandfold.
        if !density.is_finite() || !(0.5..=2.0).contains(&density) {
            return Err(format!(
                "`water_density` is in t/m³ and must be between 0.5 and 2.0 (got {density})"
            ));
        }
    }
    Ok(())
}

pub(crate) fn validate_compartment(c: &Compartment) -> Result<(), String> {
    if c.name.trim().is_empty() {
        return Err("compartment name must not be empty".into());
    }
    let coords = [c.x_min, c.x_max, c.y_min, c.y_max, c.z_min, c.z_max];
    if coords.iter().any(|v| !v.is_finite()) {
        return Err(format!(
            "compartment `{}`: box bounds must be finite",
            c.name
        ));
    }
    if c.x_max < c.x_min || c.y_max < c.y_min || c.z_max < c.z_min {
        return Err(format!(
            "compartment `{}`: max bounds must be >= min bounds",
            c.name
        ));
    }
    if !c.permeability.is_finite() || !(0.0..=1.0).contains(&c.permeability) {
        return Err(format!(
            "compartment `{}`: permeability must be in 0..=1",
            c.name
        ));
    }
    Ok(())
}

/// A closed, watertight triangulated polyhedron representing the compartment's
/// boundary in model coordinates (either an exact box or hull-clipped prism slices).
#[derive(Debug, Clone)]
pub struct CompartmentSolid {
    pub comp: Compartment,
    pub verts: Vec<Vec3>,
    pub tris: Vec<[usize; 3]>,
}

impl CompartmentSolid {
    /// Build a closed triangulated solid for the compartment.
    pub fn build(comp: &Compartment, hull: &Hull) -> Self {
        if !comp.clip_to_hull {
            return Self::build_box(comp);
        }
        Self::build_clipped(comp, hull)
    }

    /// Build an exact 12-triangle closed box with outward normals.
    fn build_box(comp: &Compartment) -> Self {
        let (x0, x1) = (comp.x_min, comp.x_max);
        let (y0, y1) = (comp.y_min, comp.y_max);
        let (z0, z1) = (comp.z_min, comp.z_max);

        let verts = vec![
            Vec3::new(x0, y0, z0), // 0
            Vec3::new(x1, y0, z0), // 1
            Vec3::new(x1, y1, z0), // 2
            Vec3::new(x0, y1, z0), // 3
            Vec3::new(x0, y0, z1), // 4
            Vec3::new(x1, y0, z1), // 5
            Vec3::new(x1, y1, z1), // 6
            Vec3::new(x0, y1, z1), // 7
        ];
        let tris = vec![
            // bottom (-z)
            [0, 2, 1],
            [0, 3, 2],
            // top (+z)
            [4, 5, 6],
            [4, 6, 7],
            // aft (-x)
            [0, 4, 7],
            [0, 7, 3],
            // fwd (+x)
            [1, 2, 6],
            [1, 6, 5],
            // port (-y)
            [0, 1, 5],
            [0, 5, 4],
            // stbd (+y)
            [3, 7, 6],
            [3, 6, 2],
        ];
        Self {
            comp: comp.clone(),
            verts,
            tris,
        }
    }

    /// Build a closed polyhedron by slicing along x and clipping the hull's
    /// transverse section polygons against the compartment's bounding box.
    fn build_clipped(comp: &Compartment, hull: &Hull) -> Self {
        let xa = comp.x_min.max(hull.model_min.x);
        let xb = comp.x_max.min(hull.model_max.x);
        let za = comp.z_min.max(hull.keel);
        let zb = comp.z_max.min(hull.model_max.z);

        if xb <= xa || zb <= za || comp.y_max <= comp.y_min {
            return Self {
                comp: comp.clone(),
                verts: Vec::new(),
                tris: Vec::new(),
            };
        }

        const SLICES: usize = 32;
        let dx = (xb - xa) / SLICES as f64;
        let mut verts: Vec<Vec3> = Vec::new();
        let mut tris: Vec<[usize; 3]> = Vec::new();

        for i in 0..SLICES {
            let x_mid = xa + (i as f64 + 0.5) * dx;
            let x_lo = xa + i as f64 * dx;
            let x_hi = x_lo + dx;

            let (_, polys) = station_polygons(hull, x_mid);
            for poly in polys {
                let pts_2d: Vec<(f64, f64)> = poly.iter().map(|p| (p.y, p.z)).collect();
                let clipped = clip_poly_2d(&pts_2d, comp.y_min, comp.y_max, za, zb);
                if clipped.len() < 3 {
                    continue;
                }
                let mut ordered_pts = clipped;
                let area = poly_area_2d(&ordered_pts);
                if area.abs() < 1e-12 {
                    continue;
                }
                if area < 0.0 {
                    ordered_pts.reverse();
                }

                // Add an extruded prism between x_lo and x_hi.
                let m = ordered_pts.len();
                let base = verts.len();
                for p in &ordered_pts {
                    verts.push(Vec3::new(x_lo, p.0, p.1));
                }
                for p in &ordered_pts {
                    verts.push(Vec3::new(x_hi, p.0, p.1));
                }

                // Side faces (outward normal for CCW loop).
                for j in 0..m {
                    let j_next = (j + 1) % m;
                    let bj = base + j;
                    let bj_next = base + j_next;
                    let fj = base + m + j;
                    let fj_next = base + m + j_next;
                    tris.push([bj, bj_next, fj_next]);
                    tris.push([bj, fj_next, fj]);
                }

                // Front face at x_hi (normal +x).
                let face_tris = triangulate_poly_2d(&ordered_pts);
                for t in &face_tris {
                    tris.push([base + m + t[0], base + m + t[1], base + m + t[2]]);
                }

                // Back face at x_lo (normal -x, reversed winding).
                for t in &face_tris {
                    tris.push([base + t[0], base + t[2], base + t[1]]);
                }
            }
        }

        Self {
            comp: comp.clone(),
            verts,
            tris,
        }
    }
}

/// Signed 2D area of a polygon.
fn poly_area_2d(pts: &[(f64, f64)]) -> f64 {
    let n = pts.len();
    if n < 3 {
        return 0.0;
    }
    let mut area = 0.0;
    for i in 0..n {
        let (x1, y1) = pts[i];
        let (x2, y2) = pts[(i + 1) % n];
        area += x1 * y2 - x2 * y1;
    }
    0.5 * area
}

/// Clip a 2D polygon against an axis-aligned bounding box `[y_min, y_max] × [z_min, z_max]`
/// using Sutherland–Hodgman.
fn clip_poly_2d(
    poly: &[(f64, f64)],
    y_min: f64,
    y_max: f64,
    z_min: f64,
    z_max: f64,
) -> Vec<(f64, f64)> {
    let mut current = poly.to_vec();
    // 1. y >= y_min
    current = clip_half_plane_2d(
        &current,
        |p| p.0 - y_min,
        |p, q, t| (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)),
    );
    // 2. y <= y_max
    current = clip_half_plane_2d(
        &current,
        |p| y_max - p.0,
        |p, q, t| (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)),
    );
    // 3. z >= z_min
    current = clip_half_plane_2d(
        &current,
        |p| p.1 - z_min,
        |p, q, t| (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)),
    );
    // 4. z <= z_max
    current = clip_half_plane_2d(
        &current,
        |p| z_max - p.1,
        |p, q, t| (p.0 + t * (q.0 - p.0), p.1 + t * (q.1 - p.1)),
    );
    current
}

fn clip_half_plane_2d<F, I>(poly: &[(f64, f64)], dist: F, interp: I) -> Vec<(f64, f64)>
where
    F: Fn((f64, f64)) -> f64,
    I: Fn((f64, f64), (f64, f64), f64) -> (f64, f64),
{
    if poly.len() < 3 {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(poly.len() + 2);
    let n = poly.len();
    for i in 0..n {
        let a = poly[i];
        let b = poly[(i + 1) % n];
        let da = dist(a);
        let db = dist(b);
        if da >= -1e-9 {
            out.push(a);
        }
        if (da < -1e-9 && db > 1e-9) || (da > 1e-9 && db < -1e-9) {
            let t = da / (da - db);
            out.push(interp(a, b, t));
        }
    }
    out
}

/// Triangulate a simple 2D CCW polygon using ear clipping with fan fallback.
fn triangulate_poly_2d(pts: &[(f64, f64)]) -> Vec<[usize; 3]> {
    let n = pts.len();
    if n < 3 {
        return Vec::new();
    }
    if n == 3 {
        return vec![[0, 1, 2]];
    }
    let mut indices: Vec<usize> = (0..n).collect();
    let mut tris = Vec::with_capacity(n - 2);

    let cross = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| -> f64 {
        (b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0)
    };

    let pt_in_tri = |p: (f64, f64), a: (f64, f64), b: (f64, f64), c: (f64, f64)| -> bool {
        let cp1 = cross(a, b, p);
        let cp2 = cross(b, c, p);
        let cp3 = cross(c, a, p);
        (cp1 >= -1e-9 && cp2 >= -1e-9 && cp3 >= -1e-9)
            || (cp1 <= 1e-9 && cp2 <= 1e-9 && cp3 <= 1e-9)
    };

    let mut count = 0;
    while indices.len() > 3 && count < n * n {
        let m = indices.len();
        let mut ear_found = false;
        for i in 0..m {
            let prev = indices[(i + m - 1) % m];
            let curr = indices[i];
            let next = indices[(i + 1) % m];
            let a = pts[prev];
            let b = pts[curr];
            let c = pts[next];
            if cross(a, b, c) > 1e-9 {
                let mut contains_other = false;
                for &idx in &indices {
                    if idx != prev && idx != curr && idx != next && pt_in_tri(pts[idx], a, b, c) {
                        contains_other = true;
                        break;
                    }
                }
                if !contains_other {
                    tris.push([prev, curr, next]);
                    indices.remove(i);
                    ear_found = true;
                    break;
                }
            }
        }
        if !ear_found {
            break;
        }
        count += 1;
    }
    if indices.len() == 3 {
        tris.push([indices[0], indices[1], indices[2]]);
    } else {
        let i0 = indices[0];
        for k in 1..indices.len() - 1 {
            tris.push([i0, indices[k], indices[k + 1]]);
        }
    }
    tris
}

/// Compute net buoyant volume and net centre of buoyancy by the lost-buoyancy method:
///
/// $$V_{net} = V_{intact} - \sum \mu_k V_{k,submerged}$$
///
/// $$\vec{B}_{net} = \frac{V_{intact}\vec{B}_{intact} - \sum \mu_k V_{k,submerged}\vec{b}_k}{V_{net}}$$
pub fn damaged_buoyancy(
    hull: &Hull,
    solids: &[CompartmentSolid],
    draft: f64,
    trim: f64,
    heel_deg: f64,
) -> (f64, Vec3) {
    let wp = waterplane_for(hull.keel, hull.length, draft, trim, heel_deg);
    damaged_buoyancy_in(hull, solids, &wp)
}

/// [`damaged_buoyancy`] below an arbitrary waterplane (`side <= 0` is
/// submerged), for planes a draft above the keel cannot describe.
pub fn damaged_buoyancy_in(hull: &Hull, solids: &[CompartmentSolid], wp: &Plane) -> (f64, Vec3) {
    let origin = wp.project(Vec3::ZERO);
    let (intact_vol, intact_moment, _, _, _) = crate::hydro::integrate(&hull.verts, &hull.tris, wp);

    let mut net_vol = intact_vol;
    let mut net_moment = intact_moment;

    for s in solids {
        let mu = s.comp.permeability;
        if s.verts.is_empty() || s.tris.is_empty() {
            continue;
        }
        let (comp_vol, comp_moment, _, _, _) = crate::hydro::integrate(&s.verts, &s.tris, wp);
        net_vol -= mu * comp_vol;
        net_moment -= comp_moment * mu;
    }

    let b = if net_vol > 1e-12 {
        net_moment / net_vol + origin
    } else {
        Vec3::ZERO
    };
    (net_vol.max(0.0), b)
}

/// Solve draft on a bracket so damaged net volume equals `target_volume`.
/// `None` when the bracket does not straddle it — a target already met at
/// `lo` included: that is not a clamp to `lo`, the real waterplane then cuts
/// the hull's vertical lower down (below the keel for a light hull at a
/// steep heel or a trimmed one), and [`solve_damaged_draft`] finds it on the
/// plane's offset.
fn damaged_draft_on_bracket(
    hull: &Hull,
    solids: &[CompartmentSolid],
    target_volume: f64,
    trim: f64,
    heel_deg: f64,
    mut lo: f64,
    mut hi: f64,
) -> Option<f64> {
    let mut vlo = damaged_buoyancy(hull, solids, lo, trim, heel_deg).0;
    let mut vhi = damaged_buoyancy(hull, solids, hi, trim, heel_deg).0;
    if vlo >= target_volume || vhi < target_volume {
        return None;
    }
    let tol = 1e-8 * target_volume.max(1.0);
    for _ in 0..80 {
        let mut d = lo + (target_volume - vlo) * (hi - lo) / (vhi - vlo);
        let margin = 0.05 * (hi - lo);
        if !d.is_finite() || d <= lo + margin || d >= hi - margin {
            d = 0.5 * (lo + hi);
        }
        let v = damaged_buoyancy(hull, solids, d, trim, heel_deg).0;
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

/// Solve draft so the damaged ship floats `target_volume` at a given trim and heel.
///
/// The draft above the keel is searched first (every upright and moderately
/// heeled condition, unchanged). Where none exists — a light or trimmed hull
/// at a steep heel, whose waterplane cuts the hull's vertical below the keel,
/// or a heel past 90° — the waterplane is found on its offset along its own
/// normal, bracketed by the hull's extent along it, and returned as the
/// (possibly negative) draft [`waterplane_for`] places it at. `None` when the
/// hull cannot float the volume, or within [`MIN_COS_HEEL`] of ±90° heel,
/// where no draft describes the waterplane.
pub fn solve_damaged_draft(
    hull: &Hull,
    solids: &[CompartmentSolid],
    target_volume: f64,
    trim: f64,
    heel_deg: f64,
) -> Option<f64> {
    let cos = deg_to_rad(heel_deg).cos();
    if cos.abs() < MIN_COS_HEEL {
        return None;
    }
    if cos > 0.0 {
        let lo = 0.0f64;
        let mut hi = (hull.model_max.z - hull.keel).max(0.1) * 2.0 + 1.0;
        for _ in 0..40 {
            if damaged_buoyancy(hull, solids, hi, trim, heel_deg).0 >= target_volume {
                break;
            }
            hi *= 1.6;
        }
        if let Some(d) =
            damaged_draft_on_bracket(hull, solids, target_volume, trim, heel_deg, lo, hi)
        {
            return Some(d);
        }
    }
    // The plane's normal does not depend on the draft; its offset `s` along
    // it submerges more of the hull as it grows, at every heel.
    let n = waterplane_for(hull.keel, hull.length, 0.0, trim, heel_deg).n;
    let (dry, full) = hull.extent_along(n);
    let volume = |s: f64| damaged_buoyancy_in(hull, solids, &waterplane_at_offset(n, s)).0;
    let s = crate::stability::root_on_bracket(volume, target_volume, dry, full, None)?;
    // `waterplane_for` puts (0, 0, keel + draft + trim/2) on the plane.
    Some(s / n.z - hull.keel - 0.5 * trim)
}

/// Solve draft *and* trim so the damaged ship floats `target_volume` with `LCB_net` at `lcg_x`.
pub fn solve_damaged_floating(
    hull: &Hull,
    solids: &[CompartmentSolid],
    target_volume: f64,
    heel_deg: f64,
    lcg_x: f64,
    trim_hint: f64,
) -> Option<FloatingCondition> {
    // One (trim -> draft, residual) sample; a draft hint from the previous
    // trim keeps the bracket tight.
    let sample = |trim: f64, draft_hint: Option<f64>| -> Option<(f64, f64)> {
        let draft = draft_hint
            .and_then(|d| {
                let w = (0.25 * d).clamp(0.02, 0.5);
                damaged_draft_on_bracket(hull, solids, target_volume, trim, heel_deg, d - w, d + w)
            })
            .or_else(|| solve_damaged_draft(hull, solids, target_volume, trim, heel_deg))?;
        let (_, b) = damaged_buoyancy(hull, solids, draft, trim, heel_deg);
        Some((draft, b.x - lcg_x))
    };
    crate::stability::secant_on_trim(hull.length, trim_hint, sample)
        .map(|(draft, trim)| FloatingCondition { draft, trim })
}

/// Find boundary vertices of the hull representing the sheer/deck edge
/// (excluding half-model centerline vertices on y = 0).
fn hull_deck_boundary_vertices(hull: &Hull) -> Vec<Vec3> {
    let mut edge_counts: HashMap<(usize, usize), usize> = HashMap::new();
    for t in &hull.tris {
        for i in 0..3 {
            let a = t[i];
            let b = t[(i + 1) % 3];
            let k = if a < b { (a, b) } else { (b, a) };
            *edge_counts.entry(k).or_default() += 1;
        }
    }
    let mut pts = Vec::new();
    let mut seen_verts: std::collections::HashSet<usize> = std::collections::HashSet::new();
    for ((a, b), count) in edge_counts {
        if count == 1 {
            let va = hull.verts[a];
            let vb = hull.verts[b];
            // Skip pure centreline edges (y == 0 on both ends) of a half-model.
            if va.y.abs() < 1e-4 && vb.y.abs() < 1e-4 {
                continue;
            }
            if seen_verts.insert(a) {
                pts.push(va);
            }
            if seen_verts.insert(b) {
                pts.push(vb);
            }
        }
    }
    pts
}

/// Resolve all compartment specifications into concrete [`Compartment`] instances.
pub fn resolve_compartments(
    case: &DamageCase,
    loading_cond: Option<&LoadingCondition>,
) -> Result<Vec<Compartment>, String> {
    let mut out = Vec::with_capacity(case.compartments.len());
    for spec in &case.compartments {
        match spec {
            CompartmentSpec::Inline(c) => {
                validate_compartment(c)?;
                out.push(c.clone());
            }
            CompartmentSpec::Named(name) => {
                let lc = case.loading_condition.as_ref().or(loading_cond).ok_or_else(|| {
                    format!("compartment `{name}` was given by name but no loading condition was provided")
                })?;
                let tank = lc.tanks.iter().find(|t| t.name == *name).ok_or_else(|| {
                    format!("compartment `{name}` not found in loading condition tanks")
                })?;
                let c = Compartment {
                    name: tank.name.clone(),
                    x_min: tank.x_min,
                    x_max: tank.x_max,
                    y_min: tank.y_min,
                    y_max: tank.y_max,
                    z_min: tank.z_min,
                    z_max: tank.z_max,
                    permeability: tank.permeability,
                    clip_to_hull: tank.clip_to_hull,
                };
                validate_compartment(&c)?;
                out.push(c);
            }
        }
    }
    Ok(out)
}

/// Full evaluation of deterministic damage stability by the lost-buoyancy method.
pub fn evaluate(
    surface: &Surface,
    opts: &HydroOptions,
    case: &DamageCase,
) -> Result<DamageResult, String> {
    validate(case)?;
    let compartments = resolve_compartments(case, None)?;
    let hull = Hull::new(surface, opts.subdivision);
    let density = case
        .water_density
        .unwrap_or(opts.water_density / 1000.0)
        .max(0.1);
    // The refined hull carries no appendages: Δ includes them, the hull
    // floats Δ / (ρ · c_app), as in `hydro`, `stability` and `loading`.
    let app_coeff = opts.appendage_coefficient.max(1e-6);

    // Derive loading particulars (displacement_t, kg, lcg, tcg).
    let (displacement_t, kg, lcg, tcg) = match (case.displacement_t, case.kg) {
        (Some(d), Some(k)) => {
            let lcg = case.lcg.unwrap_or_else(|| {
                let (target_v, b) = hull.buoyancy(opts.draft, 0.0, 0.0);
                if target_v > 1e-12 {
                    b.x
                } else {
                    0.5 * (hull.model_min.x + hull.model_max.x)
                }
            });
            let tcg = case.tcg.unwrap_or(0.0);
            (d, k, lcg, tcg)
        }
        _ => {
            if let Some(lc) = &case.loading_condition {
                let res = crate::loading::evaluate(surface, opts, lc);
                (
                    case.displacement_t.unwrap_or(res.displacement_t),
                    case.kg.unwrap_or(res.kg_fluid),
                    case.lcg.unwrap_or(res.lcg),
                    case.tcg.unwrap_or(res.tcg),
                )
            } else {
                let intact_draft = if opts.draft > 0.0 {
                    opts.draft
                } else {
                    surface.settings.draft
                };
                let (target_v, b) = hull.buoyancy(intact_draft, 0.0, 0.0);
                let d = target_v * app_coeff * density;
                let k = (hull.model_max.z - hull.keel) * 0.75;
                (
                    case.displacement_t.unwrap_or(d),
                    case.kg.unwrap_or(k),
                    case.lcg.unwrap_or(b.x),
                    case.tcg.unwrap_or(0.0),
                )
            }
        }
    };

    let target_volume = displacement_t / (density * app_coeff);
    let mut warnings = Vec::new();

    // 1. Intact equilibrium floating condition.
    let intact_fc =
        solve_floating(&hull, target_volume, 0.0, lcg, 0.0).unwrap_or(FloatingCondition {
            draft: opts.draft,
            trim: 0.0,
        });

    // 2. Build solids for each compartment.
    let solids: Vec<CompartmentSolid> = compartments
        .iter()
        .map(|c| CompartmentSolid::build(c, &hull))
        .collect();

    // 3. Damaged equilibrium solve: draft, trim, heel.
    let g = Vec3::new(lcg, tcg, hull.keel + kg);
    let righting_lever_at_heel =
        |phi: f64, trim_hint: f64| -> Option<(FloatingCondition, f64, Vec3)> {
            let fc = solve_damaged_floating(&hull, &solids, target_volume, phi, lcg, trim_hint)?;
            let (_, b) = damaged_buoyancy(&hull, &solids, fc.draft, fc.trim, phi);
            let wp = waterplane_for(hull.keel, hull.length, fc.draft, fc.trim, phi);
            let t = wp.n.cross(Vec3::new(1.0, 0.0, 0.0)).norm();
            let gz = (b - g).dot(t);
            Some((fc, gz, b))
        };

    let mut eq_fc = FloatingCondition {
        draft: intact_fc.draft,
        trim: intact_fc.trim,
    };
    let mut eq_heel = 0.0;
    let mut eq_b = Vec3::ZERO;
    let mut equilibrium_found = false;

    if let Some((fc0, gz0, b0)) = righting_lever_at_heel(0.0, intact_fc.trim) {
        eq_fc = fc0;
        eq_b = b0;
        if gz0.abs() < 1e-4 {
            // Symmetric flooding (no transverse list).
            eq_heel = 0.0;
            equilibrium_found = true;
        } else {
            // Asymmetric damage: search for list angle phi where GZ(phi) == 0.
            let sign = if gz0 < 0.0 { 1.0 } else { -1.0 };
            let mut phi_lo = 0.0;
            let mut gz_lo = gz0;
            let mut phi_hi = f64::NAN;
            let mut gz_hi = f64::NAN;

            for step in &[1.0, 2.0, 4.0, 8.0, 15.0, 25.0, 40.0, 60.0] {
                let phi = sign * *step;
                if let Some((_, gz, _)) = righting_lever_at_heel(phi, eq_fc.trim) {
                    if (gz_lo <= 0.0 && gz >= 0.0) || (gz_lo >= 0.0 && gz <= 0.0) {
                        phi_hi = phi;
                        gz_hi = gz;
                        break;
                    }
                    phi_lo = phi;
                    gz_lo = gz;
                } else {
                    break;
                }
            }

            if phi_hi.is_finite() {
                let mut p0 = phi_lo;
                let mut g0 = gz_lo;
                let mut p1 = phi_hi;
                let mut g1 = gz_hi;
                for _ in 0..25 {
                    let denom = g1 - g0;
                    let mut p_next = if denom.abs() > 1e-12 {
                        p1 - g1 * (p1 - p0) / denom
                    } else {
                        0.5 * (p0 + p1)
                    };
                    if !p_next.is_finite()
                        || p_next <= p0.min(p1) - 0.1
                        || p_next >= p0.max(p1) + 0.1
                    {
                        p_next = 0.5 * (p0 + p1);
                    }
                    if let Some((fc_cur, gz_cur, b_cur)) =
                        righting_lever_at_heel(p_next, eq_fc.trim)
                    {
                        if gz_cur.abs() < 1e-5 || (p1 - p0).abs() < 1e-4 {
                            eq_fc = fc_cur;
                            eq_heel = p_next;
                            eq_b = b_cur;
                            equilibrium_found = true;
                            break;
                        }
                        if (g0 <= 0.0 && gz_cur <= 0.0) || (g0 >= 0.0 && gz_cur >= 0.0) {
                            p0 = p_next;
                            g0 = gz_cur;
                        } else {
                            p1 = p_next;
                            g1 = gz_cur;
                        }
                    } else {
                        break;
                    }
                }
            }
        }
    }

    if !equilibrium_found {
        warnings.push("damaged equilibrium not found within -60..+60 deg heel".into());
    }

    // 4. Per-compartment submerged volume & centroid at damaged equilibrium.
    let eq_wp = waterplane_for(hull.keel, hull.length, eq_fc.draft, eq_fc.trim, eq_heel);
    let mut comp_summaries = Vec::with_capacity(solids.len());
    let mut total_lost_vol = 0.0;

    for s in &solids {
        let (vol, m, _, _, _) = if s.verts.is_empty() || s.tris.is_empty() {
            (0.0, Vec3::ZERO, 0.0, Vec3::ZERO, Vec3::ZERO)
        } else {
            crate::hydro::integrate(&s.verts, &s.tris, &eq_wp)
        };
        let c = if vol > 1e-12 {
            m / vol + eq_wp.project(Vec3::ZERO)
        } else {
            Vec3::new(
                0.5 * (s.comp.x_min + s.comp.x_max),
                0.5 * (s.comp.y_min + s.comp.y_max),
                0.5 * (s.comp.z_min + s.comp.z_max),
            )
        };
        let lost = vol * s.comp.permeability;
        total_lost_vol += lost;
        comp_summaries.push(CompartmentSummary {
            name: s.comp.name.clone(),
            permeability: s.comp.permeability,
            clip_to_hull: s.comp.clip_to_hull,
            submerged_volume_m3: vol,
            lost_volume_m3: lost,
            centroid: c,
        });
    }

    // 5. Margin / deck edge freeboard at damaged equilibrium.
    let boundary_pts = hull_deck_boundary_vertices(&hull);
    let freeboard_m = if boundary_pts.is_empty() {
        None
    } else {
        let min_fb = boundary_pts
            .iter()
            .map(|p| eq_wp.side(*p))
            .fold(f64::INFINITY, f64::min);
        Some(min_fb)
    };

    if let Some(fb) = freeboard_m {
        if fb < 0.0 {
            warnings.push(format!(
                "deck edge is immersed at damaged equilibrium (freeboard {:.3} m)",
                fb
            ));
        }
    }

    // 6. Residual GZ curve from damaged equilibrium (0..60° relative heel).
    let sign_heel = if eq_heel < 0.0 { -1.0 } else { 1.0 };
    let mut residual_gz: Vec<DamageGzPoint> = Vec::new();
    let mut hint_trim = eq_fc.trim;

    for k in 0..=30 {
        let rel_deg = k as f64 * 2.0;
        let abs_heel = eq_heel + sign_heel * rel_deg;
        match righting_lever_at_heel(abs_heel, hint_trim) {
            Some((fc, gz_abs, _)) => {
                hint_trim = fc.trim;
                let gz_res = sign_heel * gz_abs;
                residual_gz.push(DamageGzPoint {
                    heel_deg: abs_heel,
                    relative_heel_deg: rel_deg,
                    draft: Some(fc.draft),
                    trim_m: Some(fc.trim),
                    gz: Some(gz_res),
                });
            }
            None => {
                residual_gz.push(DamageGzPoint {
                    heel_deg: abs_heel,
                    relative_heel_deg: rel_deg,
                    draft: None,
                    trim_m: None,
                    gz: None,
                });
            }
        }
    }

    // 7. Residual criteria summary (SOLAS II-1 Reg. 7-2 style).
    let criteria = evaluate_residual_criteria(eq_heel, &residual_gz);

    let (net_vol, _) = damaged_buoyancy(&hull, &solids, eq_fc.draft, eq_fc.trim, eq_heel);
    let sinkage_m = eq_fc.draft - intact_fc.draft;

    Ok(DamageResult {
        case_name: case.name.clone(),
        displacement_t,
        kg,
        lcg,
        tcg,
        water_density: density,
        intact_draft: intact_fc.draft,
        intact_trim_m: intact_fc.trim,
        equilibrium_found,
        draft: eq_fc.draft,
        trim_m: eq_fc.trim,
        heel_deg: eq_heel,
        sinkage_m,
        freeboard_m,
        net_volume_m3: net_vol,
        lost_volume_m3: total_lost_vol,
        lcb_x: eq_b.x,
        tcb_y: eq_b.y,
        kb: eq_b.z - hull.keel,
        compartments: comp_summaries,
        residual_gz,
        criteria,
        warnings,
    })
}

/// Evaluate indicative SOLAS-style residual criteria from the residual GZ curve.
fn evaluate_residual_criteria(
    eq_heel: f64,
    gz_points: &[DamageGzPoint],
) -> Option<ResidualCriteria> {
    if gz_points.len() < 2 {
        return None;
    }
    // The curve up to the first heel without an equilibrium: past that gap the
    // curve is unknown, so it is neither bridged nor counted.
    let valid_points: Vec<(f64, f64)> = gz_points
        .iter()
        .map_while(|p| match p.gz {
            Some(g) if g.is_finite() => Some((p.relative_heel_deg, g)),
            _ => None,
        })
        .collect();

    if valid_points.len() < 2 {
        return None;
    }

    // Find angle of vanishing stability theta_v where residual GZ drops <= 0.
    // A curve that never rises above zero has no positive range at all; one
    // that stays positive is credited up to its last known heel.
    let mut theta_v = if valid_points.iter().any(|&(_, g)| g > 0.0) {
        valid_points[valid_points.len() - 1].0
    } else {
        0.0
    };
    for i in 1..valid_points.len() {
        let (t0, g0) = valid_points[i - 1];
        let (t1, g1) = valid_points[i];
        if g1 <= 0.0 && g0 > 0.0 {
            let zero = t0 + g0 * (t1 - t0) / (g0 - g1);
            theta_v = zero;
            break;
        }
    }

    let positive_range_deg = theta_v.max(0.0);

    // Max GZ and angle of max GZ within the positive range.
    let mut max_gz_m = 0.0;
    let mut angle_of_max = 0.0;
    for &(t, g) in &valid_points {
        if t <= theta_v && g > max_gz_m {
            max_gz_m = g;
            angle_of_max = t;
        }
    }

    // Trapezoidal area under the positive GZ curve up to theta_v in m·rad.
    let mut area_deg = 0.0;
    for w in valid_points.windows(2) {
        let (t0, mut g0) = w[0];
        let (t1, mut g1) = w[1];
        if t0 >= theta_v {
            break;
        }
        let t_end = t1.min(theta_v);
        if t1 > theta_v {
            let interp_g = g0 + (g1 - g0) * (theta_v - t0) / (t1 - t0);
            g1 = interp_g;
        }
        g0 = g0.max(0.0);
        g1 = g1.max(0.0);
        area_deg += 0.5 * (g0 + g1) * (t_end - t0);
    }
    let area_m_rad = area_deg * std::f64::consts::PI / 180.0;

    let passed = positive_range_deg >= 15.0 && max_gz_m >= 0.10 && area_m_rad >= 0.015;

    Some(ResidualCriteria {
        equilibrium_heel_deg: eq_heel,
        positive_range_deg,
        max_gz_m,
        angle_of_max_gz_deg: angle_of_max,
        area_m_rad,
        passed,
        label: "indicative, SOLAS-style (not class-approved)".to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    fn gz_curve(values: &[Option<f64>]) -> Vec<DamageGzPoint> {
        values
            .iter()
            .enumerate()
            .map(|(i, &gz)| DamageGzPoint {
                heel_deg: 5.0 * i as f64,
                relative_heel_deg: 5.0 * i as f64,
                draft: gz.map(|_| 1.0),
                trim_m: gz.map(|_| 0.0),
                gz,
            })
            .collect()
    }

    #[test]
    fn a_residual_curve_that_never_rises_has_no_positive_range() {
        let pts = gz_curve(&[Some(0.0), Some(-0.02), Some(-0.05), Some(-0.1)]);
        let c = evaluate_residual_criteria(0.0, &pts).unwrap();
        assert_eq!(c.positive_range_deg, 0.0);
        assert_eq!(c.area_m_rad, 0.0);
        assert!(!c.passed);
    }

    #[test]
    fn residual_criteria_stop_at_the_first_heel_without_equilibrium() {
        // Positive to 10°, no equilibrium at 15° and 20°, positive again at 25°:
        // only 0..10° is known, so the range ends there and nothing is bridged.
        let pts = gz_curve(&[Some(0.0), Some(0.05), Some(0.08), None, None, Some(0.3)]);
        let c = evaluate_residual_criteria(0.0, &pts).unwrap();
        assert_eq!(c.positive_range_deg, 10.0);
        assert!((c.max_gz_m - 0.08).abs() < 1e-12);
        let want = (0.5 * 0.05 * 5.0 + 0.5 * (0.05 + 0.08) * 5.0) * std::f64::consts::PI / 180.0;
        assert!((c.area_m_rad - want).abs() < 1e-12, "{}", c.area_m_rad);
    }

    #[test]
    fn a_density_in_kg_per_m3_is_refused() {
        let case = DamageCase {
            name: "kg per m3".into(),
            compartments: vec![CompartmentSpec::Named("Hold".into())],
            water_density: Some(1025.0),
            ..DamageCase::default()
        };
        let err = validate(&case).unwrap_err();
        assert!(err.contains("t/m³"), "{err}");
    }

    fn make_box_barge(length: f64, beam: f64, depth: f64) -> Surface {
        let mut s = Surface::new();
        s.layers = vec![crate::mesh::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..crate::mesh::Layer::default()
        }];
        s.settings = crate::mesh::ProjectSettings {
            name: "barge".into(),
            length,
            beam,
            draft: depth,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..crate::mesh::ProjectSettings::default()
        };
        let (l, b) = (length, beam);
        let corners = [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, depth),
            (l, 0.0, depth),
            (l, b, depth),
            (0.0, b, depth),
        ];
        for (x, y, z) in corners {
            s.add_point(Vec3::new(x, y - b * 0.5, z));
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
        s
    }

    /// Derived analytic test:
    /// Box barge L×B×D at draft T with a full-breadth midship compartment of length l
    /// and permeability μ extending above the waterline:
    /// Lost-buoyancy sinkage gives T' = T · L / (L - μ · l) exactly (no trim, symmetric).
    /// Asserts computed draft within 0.5 % of the derived analytic oracle.
    #[test]
    fn box_barge_lost_buoyancy_sinkage_matches_analytic_derived_oracle() {
        let (l_barge, b_barge, d_barge) = (10.0, 4.0, 2.0);
        let t_initial = 0.80;
        let l_comp = 2.0;
        let mu = 0.95;

        // Analytic derivation (derived here):
        // Intact displaced volume V0 = L * B * T.
        // Flooded compartment of length l and permeability mu loses volume mu * l * B * T'.
        // Remaining buoyant volume is (L - mu * l) * B * T' = V0 = L * B * T.
        // Therefore, T' = T * L / (L - mu * l).
        let t_analytic = t_initial * l_barge / (l_barge - mu * l_comp);

        let surface = make_box_barge(l_barge, b_barge, d_barge);
        let opts = HydroOptions {
            draft: t_initial,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            mainframe_x: None,
            stations: 11,
            subdivision: SubdivisionOptions {
                level: 0,
                mode: SubdivisionMode::Loop,
            },
        };

        let disp_t = l_barge * b_barge * t_initial * 1.0; // 32 tonnes at rho = 1.0 t/m3
        let case = DamageCase {
            name: "Midship flood".into(),
            compartments: vec![CompartmentSpec::Inline(Compartment {
                name: "Midship hold".into(),
                x_min: 4.0,
                x_max: 4.0 + l_comp,
                y_min: -0.5 * b_barge,
                y_max: 0.5 * b_barge,
                z_min: 0.0,
                z_max: d_barge,
                permeability: mu,
                clip_to_hull: false,
            })],
            kg: Some(0.8),
            displacement_t: Some(disp_t),
            lcg: Some(0.5 * l_barge),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: Some(1.0),
        };

        let res = evaluate(&surface, &opts, &case).expect("damage solve succeeds");
        assert!(res.equilibrium_found, "equilibrium must be found");
        let err = (res.draft - t_analytic).abs() / t_analytic;
        assert!(
            err < 0.005,
            "draft {:.4} m differs from analytic oracle {:.4} m by {:.3} % (> 0.5 %)",
            res.draft,
            t_analytic,
            err * 100.0
        );
        assert!(res.trim_m.abs() < 1e-3, "midship flood has no trim");
        assert!(res.heel_deg.abs() < 1e-3, "symmetric flood has no heel");
    }

    /// Sign check: an off-centre compartment on starboard (y > 0) produces
    /// equilibrium heel to starboard (heel > 0).
    #[test]
    fn off_centre_starboard_compartment_produces_positive_starboard_heel() {
        let (l_barge, b_barge, d_barge) = (12.0, 4.0, 2.0);
        let t_initial = 0.60;
        let surface = make_box_barge(l_barge, b_barge, d_barge);
        let opts = HydroOptions {
            draft: t_initial,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            mainframe_x: None,
            stations: 11,
            subdivision: SubdivisionOptions {
                level: 0,
                mode: SubdivisionMode::Loop,
            },
        };

        let disp_t = l_barge * b_barge * t_initial * 1.0;
        let case = DamageCase {
            name: "Starboard wing flood".into(),
            compartments: vec![CompartmentSpec::Inline(Compartment {
                name: "Starboard hold".into(),
                x_min: 3.0,
                x_max: 9.0,
                y_min: 0.2, // entirely on starboard side
                y_max: 1.8,
                z_min: 0.0,
                z_max: 1.5,
                permeability: 0.95,
                clip_to_hull: false,
            })],
            kg: Some(0.8),
            displacement_t: Some(disp_t),
            lcg: Some(6.0),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: Some(1.0),
        };

        let res = evaluate(&surface, &opts, &case).expect("damage solve succeeds");
        assert!(res.equilibrium_found, "equilibrium found");
        assert!(
            res.heel_deg > 0.5,
            "starboard flood must list to starboard (+ heel), got {:.2} deg",
            res.heel_deg
        );
    }

    /// A compartment entirely above the waterline changes nothing.
    #[test]
    fn compartment_above_the_waterline_changes_nothing() {
        let surface = make_box_barge(10.0, 4.0, 2.0);
        let opts = HydroOptions {
            draft: 0.6,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            mainframe_x: None,
            stations: 11,
            subdivision: SubdivisionOptions {
                level: 0,
                mode: SubdivisionMode::Loop,
            },
        };
        let disp_t = 10.0 * 4.0 * 0.6 * 1.0;
        let case = DamageCase {
            name: "Superstructure hold above waterline".into(),
            compartments: vec![CompartmentSpec::Inline(Compartment {
                name: "Upper hold".into(),
                x_min: 2.0,
                x_max: 8.0,
                y_min: -1.0,
                y_max: 1.0,
                z_min: 1.2, // well above T = 0.6 m
                z_max: 2.0,
                permeability: 0.95,
                clip_to_hull: false,
            })],
            kg: Some(0.7),
            displacement_t: Some(disp_t),
            lcg: Some(5.0),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: Some(1.0),
        };
        let res = evaluate(&surface, &opts, &case).expect("damage solve succeeds");
        assert!(res.equilibrium_found);
        assert!((res.draft - 0.6).abs() < 1e-4, "draft must remain at 0.6 m");
        assert!(res.sinkage_m.abs() < 1e-4, "sinkage must be zero");
        assert!(res.heel_deg.abs() < 1e-4, "heel must be zero");
        assert!(res.lost_volume_m3 < 1e-6, "lost volume must be zero");
    }

    /// On a fine bow, a compartment with clip_to_hull: true reduces lost volume
    /// compared to the unclipped box.
    #[test]
    fn fine_bow_clip_to_hull_reduces_lost_volume_vs_unclipped_box() {
        let surface = crate::demo::demo_hull(false);
        let opts = HydroOptions {
            draft: 0.62,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1025.0,
            appendage_coefficient: 1.0,
            mainframe_x: None,
            stations: 21,
            subdivision: SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        };

        // Bow compartment spanning x = 9.0..11.0 m, where the demo hull tapers to a fine stem.
        // Bounding box y = -1.5..1.5 m is much wider than the hull at the bow.
        let comp_unclipped = Compartment {
            name: "Bow hold unclipped".into(),
            x_min: 9.0,
            x_max: 11.0,
            y_min: -1.5,
            y_max: 1.5,
            z_min: 0.0,
            z_max: 1.5,
            permeability: 0.95,
            clip_to_hull: false,
        };
        let comp_clipped = Compartment {
            name: "Bow hold clipped".into(),
            clip_to_hull: true,
            ..comp_unclipped.clone()
        };

        let case_unclipped = DamageCase {
            name: "Bow unclipped".into(),
            compartments: vec![CompartmentSpec::Inline(comp_unclipped)],
            kg: Some(1.5),
            displacement_t: Some(10.0),
            lcg: Some(4.3),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: Some(1.025),
        };
        let case_clipped = DamageCase {
            name: "Bow clipped".into(),
            compartments: vec![CompartmentSpec::Inline(comp_clipped)],
            ..case_unclipped.clone()
        };

        let res_unclipped = evaluate(&surface, &opts, &case_unclipped).expect("unclipped ok");
        let res_clipped = evaluate(&surface, &opts, &case_clipped).expect("clipped ok");

        assert!(
            res_clipped.lost_volume_m3 < res_unclipped.lost_volume_m3 * 0.70,
            "clipped bow lost volume ({:.3} m3) must be significantly less than unclipped ({:.3} m3)",
            res_clipped.lost_volume_m3,
            res_unclipped.lost_volume_m3
        );
    }

    /// Demo damage case finds equilibrium with finite residual GZ.
    #[test]
    fn demo_damage_case_finds_equilibrium_with_finite_residual_gz() {
        let surface = crate::demo::demo_hull(false);
        let opts = HydroOptions {
            draft: 0.62,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1025.0,
            appendage_coefficient: 1.0,
            mainframe_x: None,
            stations: 21,
            subdivision: SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        };

        let case = DamageCase {
            name: "Demo midship damage".into(),
            compartments: vec![CompartmentSpec::Inline(Compartment {
                name: "Midship compartment".into(),
                x_min: 3.5,
                x_max: 5.0, // 1.5 m long midship
                y_min: -1.2,
                y_max: 1.2,
                z_min: 0.0,
                z_max: 1.5,
                permeability: 0.95,
                clip_to_hull: true,
            })],
            kg: Some(1.10),
            displacement_t: Some(12.147),
            lcg: Some(4.297),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: Some(1.025),
        };

        let res = evaluate(&surface, &opts, &case).expect("demo damage evaluates");
        assert!(res.equilibrium_found, "equilibrium found");
        assert!(res.sinkage_m > 0.0, "flooding must cause sinkage");
        assert!(res.residual_gz.len() >= 10, "residual GZ sampled");

        let crit = res.criteria.expect("residual criteria present");
        assert!(crit.positive_range_deg > 0.0, "finite positive range");
        assert!(crit.max_gz_m > 0.0, "finite max GZ");
        assert!(crit.area_m_rad > 0.0, "finite residual area");
    }

    #[test]
    fn validate_rejects_unresolved_loading_condition_name() {
        let case = DamageCase {
            name: "Case 1".into(),
            compartments: vec![CompartmentSpec::Inline(Compartment {
                name: "Comp".into(),
                x_min: 0.0,
                x_max: 1.0,
                y_min: -0.5,
                y_max: 0.5,
                z_min: 0.0,
                z_max: 0.5,
                permeability: 0.95,
                clip_to_hull: false,
            })],
            loading_condition_name: Some("Departure".into()),
            loading_condition: None,
            ..Default::default()
        };
        let err = validate(&case).unwrap_err();
        assert_eq!(
            err,
            "damage case `Case 1` refers to loading condition `Departure` by name; evaluate it through a project file"
        );
    }

    /// A compartment of zero permeability: present, but it floods nothing.
    fn dry_case(name: &str, displacement_t: Option<f64>, kg: Option<f64>) -> DamageCase {
        DamageCase {
            name: name.into(),
            compartments: vec![CompartmentSpec::Inline(Compartment {
                name: "Void".into(),
                x_min: 4.0,
                x_max: 6.0,
                y_min: -1.0,
                y_max: 1.0,
                z_min: 0.0,
                z_max: 2.0,
                permeability: 0.0,
                clip_to_hull: false,
            })],
            kg,
            displacement_t,
            lcg: Some(5.0),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: Some(1.0),
        }
    }

    fn barge_opts(draft: f64, appendage_coefficient: f64) -> HydroOptions {
        HydroOptions {
            draft,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: 1000.0,
            appendage_coefficient,
            mainframe_x: None,
            stations: 11,
            subdivision: SubdivisionOptions {
                level: 0,
                mode: SubdivisionMode::Loop,
            },
        }
    }

    /// R8-2 A01. A light box barge (L 10, B 4, D 2 m at T 0.3 m, KG 0.5 m)
    /// heeled until its bilge leaves the water floats on a triangular
    /// section at the low corner: legs `a` along the bottom and `a tan φ` up
    /// the side, `½ a² tan φ = B T`, centroid `(B/2 − a/3, a tan φ / 3)`, so
    /// `GZ = (B/2 − a/3) cos φ + (a tan φ / 3 − KG) sin φ` while `a ≤ B` and
    /// `a tan φ ≤ D` (8.5° … 59.0°). Past `tan φ = 8T/B` (31°) the waterplane
    /// through the keel on the centreline already floats more than ∇: the
    /// draft solve used to clamp to 0 there and report the larger section.
    #[test]
    fn residual_gz_of_a_light_barge_follows_the_bilge_out_triangle() {
        let (b, t, kg) = (4.0, 0.3, 0.5);
        let surface = make_box_barge(10.0, b, 2.0);
        let case = dry_case("light barge", Some(10.0 * b * t), Some(kg));
        let res = evaluate(&surface, &barge_opts(t, 1.0), &case).expect("evaluates");
        assert!(res.equilibrium_found && res.heel_deg.abs() < 1e-6);
        let mut checked = 0;
        for p in &res.residual_gz {
            let phi = p.relative_heel_deg;
            if !(16.0..=58.0).contains(&phi) {
                continue;
            }
            let (s, c) = deg_to_rad(phi).sin_cos();
            let a = (2.0 * b * t / s * c).sqrt();
            let want = (0.5 * b - a / 3.0) * c + (a * s / c / 3.0 - kg) * s;
            let gz = p.gz.unwrap_or_else(|| panic!("no GZ at {phi}°"));
            assert!((gz - want).abs() < 1e-6, "GZ({phi}°) = {gz}, want {want}");
            checked += 1;
        }
        assert_eq!(checked, 22);
    }

    /// R8-2 A03. Δ includes the appendages; the refined hull does not. With
    /// an appendage coefficient of 1.05, Δ = 1.05 ρ L B T floats the box at T,
    /// as `hydro`, `stability` and `loading` float it — intact and damaged.
    #[test]
    fn damage_floats_the_bare_hull_volume_like_intact_stability() {
        let surface = make_box_barge(10.0, 4.0, 2.0);
        let opts = barge_opts(0.6, 1.05);
        let disp = 10.0 * 4.0 * 0.6 * 1.05;
        let given = evaluate(&surface, &opts, &dry_case("given", Some(disp), Some(0.7))).unwrap();
        assert!(
            (given.intact_draft - 0.6).abs() < 1e-6,
            "{}",
            given.intact_draft
        );
        assert!((given.draft - 0.6).abs() < 1e-6, "{}", given.draft);
        // Without a displacement it is the one the hull has at `opts.draft`.
        let derived = evaluate(&surface, &opts, &dry_case("derived", None, None)).unwrap();
        assert!(
            (derived.displacement_t - disp).abs() < 1e-9,
            "{}",
            derived.displacement_t
        );
        assert!((derived.intact_draft - 0.6).abs() < 1e-6);
    }
}
