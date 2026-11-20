//! Strip theory against Journée's towing-tank measurements on Wigley I and III in head waves:
//! J.M.J. Journée, *Experiments and Calculations on 4 Wigley Hull Forms in Head Waves*, Delft
//! University of Technology, Ship Hydromechanics Laboratory, Report 0909, May 1992. The hulls and
//! particulars are from §2 (pp. 2 to 3); the page of each table is on its constant in
//! [`journee_1992`]. See `docs/oracles.md` §46 onward.
//!
//! The hulls are exact meshes of Journée's formula (no control net, no subdivision), run through
//! the same station cutting, close-fit sections and strip integration as `head_sea_raos`.

mod journee_1992;

use journee_1992::*;

fn rel(got: f64, want: f64) -> f64 {
    (got - want).abs() / want.abs()
}

#[test]
fn helpers_handle_roots_and_wraps() {
    assert_eq!(omega_for_encounter(5.0, 0.0, 9.81), 5.0);
    let u = 2.16998;
    let w = omega_for_encounter(6.0, u, 9.81);
    assert!(w > 0.0);
    assert!((w + w * w * u / 9.81 - 6.0).abs() < 1e-12, "root {w}");
    assert_eq!(phase_gap_deg(359.0, 1.0), 2.0);
    assert_eq!(phase_gap_deg(369.0, 9.0), 0.0);
    assert_eq!(phase_gap_deg(10.0, 190.0), 180.0);
    assert_eq!(phase_gap_deg(-90.0, 270.0), 0.0);
    assert_eq!(amp_gap(1.0, 0.0, 0.1), 10.0);
    assert_eq!(amp_gap(1.1, 1.0, 0.1), (1.1f64 - 1.0).abs());
    // Fn 0.4 on L = 3 m is U = 2.16998 m/s (Global Constraints of the plan).
    let s = spec(&WIGLEY_I, 0.4, vec![4.0], 21);
    assert!((s.speed_kn * 0.5144444444444445 - 2.16998).abs() < 1e-5);
    assert_eq!((s.draft, s.rho, s.g, s.kg), (D, RHO, G, 0.17));
    assert_eq!(s.kyy, Some(0.75));
}

/// The hydrostatics the formula gives, integrated by hand (our own arithmetic, checked in the
/// planning session against Journée's printed ∇): x = (L/2)ξ, z = d(1 − ζ),
/// `∫(1 − ξ²)(1 + 0.2ξ²)dξ = 1.386667`, `∫(1 − ξ²)⁴dξ = 256/315`,
/// `∫ξ²(1 − ξ²)(1 + 0.2ξ²)dξ = 2/3 − 0.32 − 0.4/7`; in ζ, `∫(1 − ζ²) = 2/3`, `∫ζ(1 − ζ²) = 1/4`,
/// `∫ζ²(1 − ζ⁸) = 1/3 − 1/11`, `∫ζ³(1 − ζ⁸) = 1/6`.
struct Closed {
    volume: f64,
    kb: f64,
    awp: f64,
    i_l: f64,
}

fn closed_form(m: &Model) -> Closed {
    let fx = 2.0 - 0.8 * 2.0 / 3.0 - 0.2 * 2.0 / 5.0;
    let fx4 = 256.0 / 315.0;
    let fx2 = 2.0 / 3.0 - 0.8 * 2.0 / 5.0 - 0.2 * 2.0 / 7.0;
    let area = fx * 2.0 / 3.0 + m.alpha * fx4 * (1.0 / 3.0 - 1.0 / 11.0);
    let first = fx / 4.0 + m.alpha * fx4 / 6.0;
    Closed {
        volume: m.b * (L / 2.0) * D * area,
        kb: D * (1.0 - first / area),
        awp: m.b * (L / 2.0) * fx,
        i_l: m.b * (L / 2.0).powi(3) * fx2,
    }
}

/// The facets' relative error against the formula on the (200, 40) mesh, measured 2026-11-20 (the
/// facets are inscribed, so the volume comes out small): Wigley I volume 4.17e-4, KB 2.84e-4,
/// Awp 2.88e-5, I_L 5.83e-5; Wigley III volume 1.40e-4, KB 1.02e-5, Awp and I_L as Wigley I. Each
/// tolerance is twice its error, but never looser than the 0.05 % the test started from: Wigley I's
/// α ζ²(1 − ζ⁸) term turns sharply at the keel, which forty rows facet less well.
const FORMULA_TOLERANCES: [(&str, [f64; 4]); 2] = [
    ("Wigley I", [5e-4, 5e-4, 6e-5, 1.2e-4]),
    ("Wigley III", [3e-4, 3e-5, 6e-5, 1.2e-4]),
];

#[test]
fn exact_hulls_match_the_formula_and_the_printed_particulars() {
    for (m, (name, tol)) in [&WIGLEY_I, &WIGLEY_III].into_iter().zip(FORMULA_TOLERANCES) {
        assert_eq!(m.name, name);
        let want = closed_form(m);
        let h = exact_hull(m, 200, 40).hydrostatics(&hydro_options());
        let checks = [
            ("volume", h.volume_m3, want.volume, tol[0]),
            ("KB", h.kb, want.kb, tol[1]),
            ("Awp", h.waterplane_area_m2, want.awp, tol[2]),
            ("I_L", h.waterplane_inertia_longitudinal, want.i_l, tol[3]),
        ];
        for (what, got, want, tol) in checks {
            let e = rel(got, want);
            println!("{name} {what}: mesh {got:.6}, formula {want:.6}, error {e:.2e}");
            assert!(e < tol, "{name} {what}: {got} against {want}");
        }
        // Journée's printed particulars, from the same formula with his g.
        let c33 = RHO * G * h.waterplane_area_m2;
        let c55 = RHO * G * h.waterplane_inertia_longitudinal;
        assert!(
            rel(c33, m.c33) < 0.001,
            "{name} C33 {c33} against {}",
            m.c33
        );
        assert!(
            rel(c55, m.c55) < 0.001,
            "{name} C55 {c55} against {}",
            m.c55
        );
        assert!(
            (h.volume_m3 - m.volume).abs() <= 0.00005,
            "{name} ∇ {} against the printed {}",
            h.volume_m3,
            m.volume
        );
    }
    assert!(rel(closed_form(&WIGLEY_I).volume, 0.094623) < 1e-5);
    assert!(rel(closed_form(&WIGLEY_III).volume, 0.078000) < 1e-5);
    assert!(rel(closed_form(&WIGLEY_I).kb, 0.106893) < 1e-5);
    assert!(rel(closed_form(&WIGLEY_III).kb, 0.117188) < 1e-5);
    assert!(rel(closed_form(&WIGLEY_I).awp, 0.624) < 1e-5);
    assert!(rel(closed_form(&WIGLEY_I).i_l, 0.293143) < 1e-5);
}
