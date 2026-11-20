//! Journée (1992), Report 0909 (Delft University of Technology): the Wigley I and III models, an
//! exact mesh of their formula, and the helpers that compare SeaCraft with his tables.
//!
//! Hull formula (§2, p. 2), with ξ = 2x/L from −1 at the stern to 1 at the bow (the report prints
//! −0.5 to 0.5; its offsets and volumes show the range is −1 to 1 on L/2), η = 2y/B and ζ the depth
//! below the waterline over d:
//!
//! `η = (1 − ζ²)(1 − ξ²)(1 + a2 ξ² + a4 ξ⁴) + α ζ² (1 − ζ⁸)(1 − ξ²)⁴`, a2 = 0.2, a4 = 0.
//!
//! **Gap floor.** A relative gap is `|got − want| / max(|want|, floor)`. For each (hull, Fn,
//! quantity) the floor is 10 % of the largest measured |value| in that group, so a measurement
//! near zero is compared in units of the group's scale rather than in percent of itself. Phases
//! are compared only where the measured amplitude is at least its floor, and so is SeaCraft's where
//! it computes the quantity itself (wave loads, motions): below it the phase is noise, or swings
//! fast through a cancellation.

use seacraft_core::hydro::{Hull, HydroOptions};
use seacraft_core::math::Vec3;
use seacraft_core::seakeeping::{
    head_sea_raos_on_hull, GlobalCoefficients, SeakeepingReport, SeakeepingSpec, WaveLoads,
};
use seacraft_core::subdiv::SubdivisionOptions;

/// Model length between perpendiculars (m).
pub const L: f64 = 3.0;
/// Draft (m).
pub const D: f64 = 0.1875;
/// Tank water density (kg/m³), fresh water.
pub const RHO: f64 = 1000.0;
/// Gravity (m/s²). Journée's printed C33 6119 implies 9.806; the 0.04 % is below every band.
pub const G: f64 = 9.81;
mod tables;
pub use tables::*;

/// The `a2` of the hull formula, common to all four models.
const A2: f64 = 0.2;

/// One model's particulars as printed in §2 (p. 2).
#[derive(Debug, Clone, Copy)]
pub struct Model {
    pub name: &'static str,
    /// The α of the hull formula.
    pub alpha: f64,
    /// Beam (m).
    pub b: f64,
    /// Printed displacement volume ∇ (m³).
    pub volume: f64,
    /// Printed KG (m).
    pub kg: f64,
    /// Printed pitch radius of gyration (m).
    pub kyy: f64,
    /// Printed heave restoring coefficient (N/m).
    pub c33: f64,
    /// Printed pitch restoring coefficient (N·m), ρg·I_L alone (see the spec's C55 note).
    pub c55: f64,
}

pub const WIGLEY_I: Model = Model {
    name: "Wigley I",
    alpha: 1.0,
    b: 0.300,
    volume: 0.0946,
    kg: 0.1700,
    kyy: 0.7500,
    c33: 6119.0,
    c55: 2874.0,
};

pub const WIGLEY_III: Model = Model {
    name: "Wigley III",
    alpha: 0.0,
    b: 0.300,
    volume: 0.0780,
    kg: 0.1700,
    kyy: 0.7500,
    c33: 6119.0,
    c55: 2874.0,
};

/// The half-breadth at `x` from the stern (0 to L) and `z` above the keel: the formula below the
/// waterline, and its waterline shape (ζ = 0) carried straight up above it.
pub fn half_breadth(m: &Model, x: f64, z: f64) -> f64 {
    let xi = 2.0 * x / L - 1.0;
    let zeta = ((D - z) / D).clamp(0.0, 1.0);
    let ends = 1.0 - xi * xi;
    let z2 = zeta * zeta;
    let eta =
        (1.0 - z2) * ends * (1.0 + A2 * xi * xi) + m.alpha * z2 * (1.0 - z2.powi(4)) * ends.powi(4);
    0.5 * m.b * eta
}

/// Both sides of the formula, faceted on `nx` columns over `0..L` and `nz` rows over `0..D`, plus
/// wall-sided topside rows at 0.25 and 0.30 m. The keel row and both stems lie on `y = 0` and are
/// shared by the two sides, so the only open edge is the sheer, which [`Hull::from_panels`] closes
/// with a flat deck. Normals point out.
pub fn exact_hull(m: &Model, nx: usize, nz: usize) -> Hull {
    let mut rows: Vec<f64> = (0..=nz).map(|j| D * j as f64 / nz as f64).collect();
    rows.extend([0.25, 0.30]);
    let nr = rows.len();
    let mut verts = Vec::new();
    // index[side][i][j]; the centreline vertices are pushed once.
    let mut index = [Vec::new(), Vec::new()];
    for i in 0..=nx {
        let x = L * i as f64 / nx as f64;
        let (mut stbd, mut port) = (Vec::with_capacity(nr), Vec::with_capacity(nr));
        for (j, &z) in rows.iter().enumerate() {
            if i == 0 || i == nx || j == 0 {
                verts.push(Vec3::new(x, 0.0, z));
                stbd.push(verts.len() - 1);
                port.push(verts.len() - 1);
            } else {
                let y = half_breadth(m, x, z);
                verts.push(Vec3::new(x, y, z));
                stbd.push(verts.len() - 1);
                verts.push(Vec3::new(x, -y, z));
                port.push(verts.len() - 1);
            }
        }
        index[0].push(stbd);
        index[1].push(port);
    }
    let mut tris = Vec::new();
    for i in 0..nx {
        for j in 0..nr - 1 {
            for (side, s) in index.iter().enumerate() {
                let (a, b, c, d) = (s[i][j], s[i][j + 1], s[i + 1][j + 1], s[i + 1][j]);
                // The diagonal mirrors at midship, so the facets are as symmetric fore and aft as
                // the formula.
                let quad = if 2 * i < nx {
                    [[a, b, c], [a, c, d]]
                } else {
                    [[a, b, d], [b, c, d]]
                };
                for [p, q, r] in quad {
                    // Starboard runs (z, then x), which turns the normal to +y; port is its
                    // mirror and runs the other way.
                    tris.push(if side == 0 { [p, q, r] } else { [p, r, q] });
                }
            }
        }
    }
    // Keep only the triangles with three distinct vertices (the stems collapse onto y = 0).
    tris.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    Hull::from_panels(
        verts,
        tris,
        0.0,
        L,
        Vec3::new(0.0, -m.b / 2.0, 0.0),
        Vec3::new(L, m.b / 2.0, 0.30),
    )
}

/// Upright at the design draft, tank water, midship at L/2, 21 stations.
pub fn hydro_options() -> HydroOptions {
    HydroOptions {
        draft: D,
        trim: 0.0,
        heel_deg: 0.0,
        water_density: RHO,
        appendage_coefficient: 1.0,
        mainframe_x: Some(L / 2.0),
        stations: 21,
        subdivision: SubdivisionOptions::default(),
    }
}

/// The speed for a Froude number on L, in m/s.
pub fn speed(froude: f64) -> f64 {
    froude * (G * L).sqrt()
}

/// The run of one model at one Froude number, at the given wave frequencies.
pub fn spec(m: &Model, froude: f64, omegas: Vec<f64>, stations: usize) -> SeakeepingSpec {
    SeakeepingSpec {
        draft: D,
        speed_kn: speed(froude) / 0.5144444444444445,
        kg: m.kg,
        kyy: Some(m.kyy),
        stations,
        omegas,
        rho: RHO,
        g: G,
        ..Default::default()
    }
}

/// The wave frequency that meets the hull at `omega_e` in head seas: the positive root of
/// ω + ω²U/g = ω_e (ω = ω_e at U = 0), in the form that does not cancel.
pub fn omega_for_encounter(omega_e: f64, u: f64, g: f64) -> f64 {
    2.0 * omega_e / (1.0 + (1.0 + 4.0 * u * omega_e / g).sqrt())
}

/// `|got − want| / max(|want|, floor)`; see the gap floor in the module documentation.
pub fn amp_gap(got: f64, want: f64, floor: f64) -> f64 {
    (got - want).abs() / want.abs().max(floor)
}

/// The difference of two phases in degrees, wrapped into [0, 180].
pub fn phase_gap_deg(got: f64, want: f64) -> f64 {
    let d = (got - want).rem_euclid(360.0);
    d.min(360.0 - d)
}

/// The frequency in rad/s of a non-dimensional ω″ = ω√(L/g).
pub fn omega_from_nd(omega_nd: f64) -> f64 {
    omega_nd * (G / L).sqrt()
}

/// The deep-water wave frequency in rad/s of a wave λ/L long.
pub fn omega_for_wavelength(lambda_over_l: f64) -> f64 {
    (2.0 * std::f64::consts::PI * G / (lambda_over_l * L)).sqrt()
}

/// One model at one Froude number and the given wave frequencies, on an exact mesh of `mesh`
/// (columns, rows below the waterline) with `stations` strips.
pub fn run(
    m: &Model,
    froude: f64,
    omegas: Vec<f64>,
    stations: usize,
    mesh: (usize, usize),
) -> (SeakeepingReport, Vec<WaveLoads>) {
    let hull = exact_hull(m, mesh.0, mesh.1);
    head_sea_raos_on_hull(&hull, &hydro_options(), &spec(m, froude, omegas, stations))
        .expect("the exact Wigley hull solves")
}

/// SeaCraft's coefficients in Journée's non-dimensional form and signs: divided by his printed ∇
/// (§4.1), and the couplings negated, because his pitch is positive bow down and SeaCraft's bow
/// up (`coupling_signs_match_after_mapping` is the check).
#[derive(Debug, Clone, Copy)]
pub struct Coefficients {
    pub a33: f64,
    pub b33: f64,
    pub a53: f64,
    pub b53: f64,
    pub a55: f64,
    pub b55: f64,
    pub a35: f64,
    pub b35: f64,
}

pub fn journee_coefficients(c: &GlobalCoefficients, m: &Model) -> Coefficients {
    let mass = RHO * m.volume;
    let root = (G / L).sqrt();
    Coefficients {
        a33: c.a33 / mass,
        b33: c.b33 / (mass * root),
        a53: -c.a53 / (mass * L),
        b53: -c.b53 / (mass * L * root),
        a55: c.a55 / (mass * L * L),
        b55: c.b55 / (mass * L * L * root),
        a35: -c.a35 / (mass * L),
        b35: -c.b35 / (mass * L * root),
    }
}

/// SeaCraft's pitch phase (bow up) in Journée's convention (bow down), in [0, 360).
pub fn bow_down_deg(bow_up_deg: f64) -> f64 {
    (bow_up_deg + 180.0).rem_euclid(360.0)
}

/// The floor of a group of measured values: 10 % of the largest |value| (module documentation).
pub fn floor_of(values: impl IntoIterator<Item = f64>) -> f64 {
    0.1 * values.into_iter().fold(0.0, |a: f64, v| a.max(v.abs()))
}

/// Strips along the waterline (see `stations_and_mesh_have_converged`).
pub const STATIONS: usize = 81;
/// Exact-mesh columns over L and rows below the waterline (see `stations_and_mesh_have_converged`).
pub const MESH: (usize, usize) = (200, 40);
