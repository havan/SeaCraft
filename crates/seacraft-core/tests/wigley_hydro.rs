//! The Wigley hull's hydrostatics over drafts and its GZ curve, against the
//! published hull equation (Kajitani et al. 1983, p. 8):
//! `y = ±(B/2)(1 − ξ²)(1 − ζ²)`, `ξ = 2x/L − 1`, `ζ = (T − z)/T`, wall-sided
//! above `T` to a flat deck at `D`. See `docs/oracles.md` §44 and §45.
//!
//! Two levels, so a failure says which part is wrong:
//!
//! - an exact faceted mesh of the equation (no subdivision) checks the
//!   measurement code against closed forms we integrated from the equation
//!   ourselves (not published), with a tolerance set by the facet size;
//! - the `examples/wigley.fef` control net, refined as the app measures it,
//!   checks the example's fit away from the design draft it was tuned at.

use seacraft_core::fef;
use seacraft_core::hydro::{Hull, HydroOptions, Hydrostatics};
use seacraft_core::math::Vec3;
use seacraft_core::subdiv::{SubdivisionMode, SubdivisionOptions};

const L: f64 = 10.0;
const B: f64 = 1.0;
const T: f64 = 0.625;
const D: f64 = 1.0;
const RHO: f64 = 1025.0;

/// The drafts both levels are measured at: well below `T`, at it, and on the
/// wall-sided topside above it.
const DRAFTS: [f64; 5] = [0.25 * T, 0.5 * T, 0.75 * T, T, 0.8];

fn rel(got: f64, want: f64) -> f64 {
    (got - want).abs() / want.abs()
}

/// The half-breadth the equation gives at `(x, z)`; wall-sided above `T`.
fn half_breadth(x: f64, z: f64) -> f64 {
    let xi = 2.0 * x / L - 1.0;
    let zeta = ((T - z) / T).max(0.0);
    0.5 * B * (1.0 - xi * xi) * (1.0 - zeta * zeta)
}

/// Both sides of the equation, faceted directly on a grid of `nx` columns over
/// `0..L` and `nz` rows over `0..T`, plus rows at 0.8 m and `D` (the topside is
/// ruled, so two rows are exact there). The keel row and both stems lie on
/// `y = 0` and are shared by the sides, so the only open edge is the sheer,
/// which [`Hull::from_panels`] closes with a flat deck. Normals point out.
fn exact_hull(nx: usize, nz: usize) -> Hull {
    let mut rows: Vec<f64> = (0..=nz).map(|j| T * j as f64 / nz as f64).collect();
    rows.extend([0.8, D]);
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
                let y = half_breadth(x, z);
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
                // The diagonal mirrors at midship, so the facets are as
                // symmetric fore and aft as the equation (LCB, LCF at L/2).
                let quad = if 2 * i < nx {
                    [[a, b, c], [a, c, d]]
                } else {
                    [[a, b, d], [b, c, d]]
                };
                for [p, q, r] in quad {
                    // Starboard runs (z, then x), which turns the normal to
                    // +y; port is its mirror and runs the other way.
                    tris.push(if side == 0 { [p, q, r] } else { [p, r, q] });
                }
            }
        }
    }
    // Drop the triangles that collapse onto the centreline at the stems.
    tris.retain(|t| t[0] != t[1] && t[1] != t[2] && t[0] != t[2]);
    Hull::from_panels(
        verts,
        tris,
        0.0,
        L,
        Vec3::new(0.0, -B / 2.0, 0.0),
        Vec3::new(L, B / 2.0, D),
    )
}

/// The hydrostatics the equation gives at draft `t`, integrated by hand (our
/// own arithmetic, not published). Inertias are about the centreline and the
/// waterplane's own centroid.
struct Closed {
    volume: f64,
    kb: f64,
    awp: f64,
    i_t: f64,
    i_l: f64,
    am: f64,
    lcb_x: f64,
    lcf_x: f64,
}

fn closed_form(t: f64) -> Closed {
    // ∫(1 − ξ²)dx = 2L/3, ∫(1 − ξ²)³dx = (L/2)(32/35), ∫ξ²(1 − ξ²)dx = (L/2)(4/15).
    let tau = t.min(T) / T;
    let w = 2.0 * tau - tau * tau; // (1 − ζ²) at the waterline
    let below = tau * tau - tau.powi(3) / 3.0; // ∫(1 − ζ²)dz / T
    let below_moment = T * T * (2.0 * tau.powi(3) / 3.0 - tau.powi(4) / 4.0);
    let above = (t - T).max(0.0);
    let volume = 2.0 / 3.0 * L * B * (T * below + above);
    let moment = 2.0 / 3.0 * L * B * (below_moment + above * (T + t) / 2.0);
    Closed {
        volume,
        kb: moment / volume,
        awp: 2.0 / 3.0 * L * B * w,
        i_t: 2.0 / 3.0 * (B / 2.0 * w).powi(3) * (L / 2.0) * (32.0 / 35.0),
        i_l: B * w * (L / 2.0).powi(3) * (4.0 / 15.0),
        am: B * (T * below + above),
        lcb_x: L / 2.0,
        lcf_x: L / 2.0,
    }
}

/// Upright, level, sea water, sections at 21 stations, midship at `L/2`,
/// refined the way the CLI measures (Loop, level 3).
fn base_options(draft: f64) -> HydroOptions {
    HydroOptions {
        draft,
        trim: 0.0,
        heel_deg: 0.0,
        water_density: RHO,
        appendage_coefficient: 1.0,
        mainframe_x: Some(L / 2.0),
        stations: 21,
        subdivision: SubdivisionOptions {
            level: 3,
            mode: SubdivisionMode::Loop,
        },
    }
}

#[test]
fn the_closed_forms_reduce_to_the_design_draft_values() {
    // The design-draft forms in the module header of `examples/wigley.rs`.
    let c = closed_form(T);
    assert!((c.volume - 4.0 / 9.0 * L * B * T).abs() < 1e-12);
    assert!((c.kb - 0.625 * T).abs() < 1e-12);
    assert!((c.i_t / c.volume - 9.0 * B * B / (105.0 * T)).abs() < 1e-12);
    // Continuous at T from above.
    let above = closed_form(T + 1e-9);
    assert!((above.volume - c.volume).abs() < 1e-8);
    assert!((above.kb - c.kb).abs() < 1e-8);
}

#[test]
fn exact_mesh_matches_the_closed_forms_over_drafts() {
    let hull = exact_hull(400, 120);
    let mut worst = 0.0f64;
    println!("draft  quantity      got          want         gap %");
    for t in DRAFTS {
        let h = hull.hydrostatics(&base_options(t));
        let c = closed_form(t);
        for (name, got, want) in compared(&h, &c) {
            let gap = rel(got, want);
            worst = worst.max(gap);
            println!("{t:.4} {name:8} {got:12.6} {want:12.6} {:8.4}", 100.0 * gap);
            assert!(gap < 1e-3, "{name} at {t}: {got} vs {want}");
        }
        assert!(
            (h.lcb.x - c.lcb_x).abs() < 1e-6 * L,
            "LCB at {t}: {}",
            h.lcb.x
        );
        assert!(
            (h.lcf.x - c.lcf_x).abs() < 1e-6 * L,
            "LCF at {t}: {}",
            h.lcf.x
        );
    }
    println!("worst gap {:.4} %", 100.0 * worst);
}

#[test]
fn exact_mesh_wetted_surface_matches_kajitani() {
    // Kajitani et al. (1983), p. 6: S = 0.661 L (2T + B), printed to three
    // figures, so it carries about 0.05 % of its own rounding (§39).
    let hull = exact_hull(400, 120);
    let s = hull.hydrostatics(&base_options(T)).wetted_surface_m2;
    let want = 0.661 * L * (2.0 * T + B);
    println!("wetted {s:.5} vs {want:.5}: {:.4} %", 100.0 * rel(s, want));
    assert!(rel(s, want) < 1e-3, "{s} vs {want}");
}

/// `examples/wigley.fef`, the file the app opens, refined as the CLI measures
/// it by default.
fn example_hull() -> Hull {
    let surface =
        fef::parse(include_str!("../../../examples/wigley.fef")).expect("wigley.fef parses");
    Hull::new(&surface, base_options(T).subdivision)
}

/// The six quantities both levels compare: (name, measured, closed form).
fn compared(h: &Hydrostatics, c: &Closed) -> [(&'static str, f64, f64); 6] {
    [
        ("volume", h.volume_m3, c.volume),
        ("KB", h.kb, c.kb),
        ("Awp", h.waterplane_area_m2, c.awp),
        ("BM_T", h.bm_transverse, c.i_t / c.volume),
        ("BM_L", h.bm_longitudinal, c.i_l / c.volume),
        ("Am", h.mainframe_area_m2, c.am),
    ]
}

/// Bands (%) for the example against the closed forms, per draft (rows, as
/// [`DRAFTS`]) and quantity (columns, as [`compared`]): the measured gap
/// rounded up to the next 0.1 % plus 0.1 %. Measured on 2026-11-20 (%):
///
/// | draft | volume |   KB  |  Awp  |  BM_T |  BM_L |   Am  |
/// |-------|--------|-------|-------|-------|-------|-------|
/// | 0.25T | −5.55  | +1.03 | −2.11 | −0.39 | +3.35 | −5.40 |
/// | 0.5T  | −1.86  | +0.92 | −0.01 | +2.16 | +1.59 | −1.71 |
/// | 0.75T | −0.83  | +0.64 | +0.31 | +2.06 | +0.85 | −0.67 |
/// | T     | −0.48  | +0.44 | −0.07 | +0.56 | +0.13 | −0.33 |
/// | 0.8 m | −0.34  | +0.32 | −0.01 | +0.61 | +0.04 | −0.18 |
///
/// These record the model's gap, not the measurement's, which the exact mesh
/// bounds at 0.01 %: the control net's rows were scaled to match the closed
/// forms at `T` only, and Loop refinement pulls the limit surface in near the
/// keel, so the example is a design-draft fit that thins below about 0.5T.
const EXAMPLE_BAND_PCT: [[f64; 6]; 5] = [
    [5.7, 1.2, 2.3, 0.5, 3.5, 5.6],
    [2.0, 1.1, 0.2, 2.3, 1.7, 1.9],
    [1.0, 0.8, 0.5, 2.2, 1.0, 0.8],
    [0.6, 0.6, 0.2, 0.7, 0.3, 0.5],
    [0.5, 0.5, 0.2, 0.8, 0.2, 0.3],
];

#[test]
fn example_hull_follows_the_closed_forms_off_design() {
    let hull = example_hull();
    let mut failures = Vec::new();
    println!("draft  quantity      got          want         gap %");
    for (t, bands) in DRAFTS.into_iter().zip(EXAMPLE_BAND_PCT) {
        let h = hull.hydrostatics(&base_options(t));
        let c = closed_form(t);
        for ((name, got, want), band) in compared(&h, &c).into_iter().zip(bands) {
            let gap = 100.0 * (got - want) / want;
            println!("{t:.4} {name:8} {got:12.6} {want:12.6} {gap:+8.4}");
            if gap.abs() > band {
                failures.push(format!("{name} at {t}: {gap:+.4} % outside {band} %"));
            }
        }
        // Fore-aft symmetric, so centred whatever the fit.
        assert!(
            (h.lcb.x - L / 2.0).abs() < 1e-6 * L,
            "LCB at {t}: {}",
            h.lcb.x
        );
        assert!(
            (h.lcf.x - L / 2.0).abs() < 1e-6 * L,
            "LCF at {t}: {}",
            h.lcf.x
        );
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

#[test]
fn example_wetted_surface_matches_kajitani() {
    // Kajitani et al. (1983), p. 6; measured +0.04 % on 2026-11-20, so 0.2 %
    // leaves the formula's own rounding (about 0.05 %) and the fit's room.
    let s = example_hull()
        .hydrostatics(&base_options(T))
        .wetted_surface_m2;
    let want = 0.661 * L * (2.0 * T + B);
    println!(
        "wetted {s:.5} vs {want:.5}: {:+.4} %",
        100.0 * (s / want - 1.0)
    );
    assert!(rel(s, want) < 2e-3, "{s} vs {want}");
}
