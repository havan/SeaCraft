//! Strip theory against Journée's towing-tank measurements on Wigley I and III in head waves:
//! J.M.J. Journée, *Experiments and Calculations on 4 Wigley Hull Forms in Head Waves*, Delft
//! University of Technology, Ship Hydromechanics Laboratory, Report 0909, May 1992. The hulls and
//! particulars are from §2 (pp. 2 to 3); the page of each table is on its constant in
//! [`journee_1992`]. See `docs/oracles.md` §46 onward.
//!
//! The hulls are exact meshes of Journée's formula (no control net, no subdivision), run through
//! the same station cutting, close-fit sections and strip integration as `head_sea_raos`.

// Tasks still to come read the remaining table fields.
#[allow(dead_code)]
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

/// The forced-oscillation frequencies Journée ran: 3 to 12 rad/s, and also 1 and 2 rad/s on
/// Wigley III at Fn 0.2 (Table 2-III and 3-III print ω″ 0.55 and 1.10 there).
const FORCED_OMEGAS: [f64; 10] = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0];

#[test]
fn tables_are_consistent() {
    let froudes = [0.2, 0.3, 0.4];
    let forced = [
        (TABLE_2_I, 48, [0.025, 0.050]),
        (TABLE_2_III, 52, [0.025, 0.050]),
        (TABLE_3_I, 48, [1.5, 3.0]),
        (TABLE_3_III, 52, [1.5, 3.0]),
    ];
    for (table, rows, amplitudes) in forced {
        assert_eq!(table.len(), rows);
        for r in table {
            assert!(froudes.contains(&r.froude));
            assert!(amplitudes.contains(&r.amplitude));
            let w = omega_from_nd(r.omega_nd);
            assert!(
                FORCED_OMEGAS.iter().any(|t| (w - t).abs() < 0.1),
                "ω″ {} is {w} rad/s",
                r.omega_nd
            );
        }
    }
    // λ/L runs 0.5 to 2.0, and to 2.367 on one row of Table 8-III.
    let lambdas = |l: f64| (0.5..=2.4).contains(&l);
    for (table, rows) in [(TABLE_6_I, 37), (TABLE_6_III, 42)] {
        assert_eq!(table.len(), rows);
        assert!(table
            .iter()
            .all(|r| froudes.contains(&r.froude) && lambdas(r.lambda_over_l)));
    }
    for (table, rows) in [(TABLE_8_I, 34), (TABLE_8_III, 64)] {
        assert_eq!(table.len(), rows);
        assert!(table
            .iter()
            .all(|r| froudes.contains(&r.froude) && lambdas(r.lambda_over_l)));
    }
}

/// Very long waves are followed: heave in phase with the crest at the centre of gravity and the
/// pitch on the wave slope, which is 270° bow down; at λ/L = 2 both phases fall in the quadrant
/// Table 8-I measured. This is the proof of the pitch sign map (bow up + 180° = bow down).
#[test]
fn signs_and_phases_follow_journee() {
    let omegas = vec![omega_for_wavelength(20.0), omega_for_wavelength(2.0)];
    let (rep, _) = run(&WIGLEY_I, 0.2, omegas, STATIONS, MESH);
    let long = &rep.frequencies[0];
    println!(
        "λ/L 20: heave {:.3} at {:.1}°, pitch bow down {:.1}°",
        long.heave_amp,
        long.heave_phase_deg,
        bow_down_deg(long.pitch_phase_deg)
    );
    assert!(phase_gap_deg(long.heave_phase_deg, 0.0) < 5.0);
    assert!(phase_gap_deg(bow_down_deg(long.pitch_phase_deg), 270.0) < 5.0);
    let two = &rep.frequencies[1];
    let measured = TABLE_8_I
        .iter()
        .find(|r| r.froude == 0.2 && r.lambda_over_l == 2.0)
        .expect("Table 8-I, Fn 0.2, λ/L 2");
    let (ez, et) = (measured.eps_z_deg.unwrap(), measured.eps_theta_deg.unwrap());
    let (gz, gt) = (two.heave_phase_deg, bow_down_deg(two.pitch_phase_deg));
    println!("λ/L 2: heave {gz:.1}° (measured {ez}), pitch {gt:.1}° (measured {et})");
    assert!(phase_gap_deg(gz, ez) < 30.0);
    assert!(phase_gap_deg(gt, et) < 30.0);
}

/// Wherever Journée's coupled damping B35″ (forced pitch) or B53″ (forced heave) on Wigley I at
/// Fn 0.3 and ω ≥ 5 rad/s stands above its floor, SeaCraft's, negated for the pitch sign, has the
/// same sign.
#[test]
fn coupling_signs_match_after_mapping() {
    let froude = 0.3;
    let u = speed(froude);
    for (table, which) in [(TABLE_2_I, "B53"), (TABLE_3_I, "B35")] {
        let rows: Vec<_> = table.iter().filter(|r| r.froude == froude).collect();
        let floor = floor_of(rows.iter().filter_map(|r| r.b_coupled));
        let rows: Vec<_> = rows
            .into_iter()
            .filter(|r| omega_from_nd(r.omega_nd) >= 5.0 - 0.1)
            .collect();
        let omegas = rows
            .iter()
            .map(|r| omega_for_encounter(omega_from_nd(r.omega_nd), u, G))
            .collect();
        let (rep, _) = run(&WIGLEY_I, froude, omegas, STATIONS, MESH);
        let coefficients = rep.coefficients.expect("coefficients");
        let mut compared = 0;
        for (r, c) in rows.iter().zip(&coefficients) {
            let got = journee_coefficients(c, &WIGLEY_I);
            let got = if which == "B53" { got.b53 } else { got.b35 };
            let want = r.b_coupled.unwrap();
            println!(
                "{which} ω″ {}: measured {want}, SeaCraft {got:.4}",
                r.omega_nd
            );
            if want.abs() >= floor {
                assert_eq!(got.signum(), want.signum(), "{which} at ω″ {}", r.omega_nd);
                compared += 1;
            }
        }
        assert!(compared >= 4, "{which}: {compared} points above the floor");
    }
}

/// What the convergence check compares, on Wigley I at Fn 0.3: heave and pitch amplitude and phase
/// at λ/L 0.75 to 2.0, and B33″, B55″ at ω_e 4 to 12 rad/s. Amplitudes are grouped by quantity so
/// each gets its own floor.
fn convergence_values(stations: usize, mesh: (usize, usize)) -> (Vec<Vec<f64>>, Vec<Vec<f64>>) {
    let froude = 0.3;
    let lambdas = [0.75, 1.0, 1.25, 1.5, 1.75, 2.0];
    let encounters = [4.0, 5.0, 6.0, 7.0, 8.0, 10.0, 12.0];
    let u = speed(froude);
    let mut omegas: Vec<f64> = lambdas.iter().map(|&l| omega_for_wavelength(l)).collect();
    omegas.extend(encounters.iter().map(|&we| omega_for_encounter(we, u, G)));
    let (rep, _) = run(&WIGLEY_I, froude, omegas, stations, mesh);
    let n = lambdas.len();
    let motions = &rep.frequencies[..n];
    let coefficients: Vec<_> = rep.coefficients.expect("coefficients")[n..]
        .iter()
        .map(|c| journee_coefficients(c, &WIGLEY_I))
        .collect();
    let amplitudes = vec![
        motions.iter().map(|p| p.heave_amp).collect(),
        motions.iter().map(|p| p.pitch_amp).collect(),
        coefficients.iter().map(|c| c.b33).collect(),
        coefficients.iter().map(|c| c.b55).collect(),
    ];
    let phases = vec![
        motions.iter().map(|p| p.heave_phase_deg).collect(),
        motions.iter().map(|p| p.pitch_phase_deg).collect(),
    ];
    (amplitudes, phases)
}

/// The largest change from `a` to the finer `b`: amplitudes relative (with the group floor), phases
/// in degrees.
fn largest_change(
    a: &(Vec<Vec<f64>>, Vec<Vec<f64>>),
    b: &(Vec<Vec<f64>>, Vec<Vec<f64>>),
) -> (f64, f64) {
    let mut amp: f64 = 0.0;
    for (ga, gb) in a.0.iter().zip(&b.0) {
        let floor = floor_of(gb.iter().copied());
        for (x, y) in ga.iter().zip(gb) {
            amp = amp.max(amp_gap(*x, *y, floor));
        }
    }
    // Phases only where the amplitude (heave, pitch: the first two groups) reaches its floor.
    let mut phase: f64 = 0.0;
    for ((ga, gb), amps) in a.1.iter().zip(&b.1).zip(&b.0) {
        let floor = floor_of(amps.iter().copied());
        for ((x, y), amp) in ga.iter().zip(gb).zip(amps) {
            if amp.abs() >= floor {
                phase = phase.max(phase_gap_deg(*x, *y));
            }
        }
    }
    (amp, phase)
}

/// `STATIONS` and `MESH` are the smallest of 21, 41, 81 stations and meshes (100, 20), (200, 40),
/// (400, 80) whose step to the next finer moves every compared value by at most 0.5 % and 0.5°.
/// Measured 2026-11-20 (amplitudes, phases): stations 21 → 41 1.93 %, 2.40°; 41 → 81 0.48 %, 0.59°;
/// 81 → 161 0.12 %, 0.14°; mesh (100, 20) → (200, 40) 0.62 %, 0.39°; (200, 40) → (400, 80)
/// 0.15 %, 0.10°. So 81 stations on the (200, 40) mesh; this test holds both steps beyond it.
#[test]
fn stations_and_mesh_have_converged() {
    assert_eq!((STATIONS, MESH), (81, (200, 40)));
    let chosen = convergence_values(STATIONS, MESH);
    for (what, finer) in [
        ("stations", convergence_values(2 * STATIONS - 1, MESH)),
        (
            "mesh",
            convergence_values(STATIONS, (2 * MESH.0, 2 * MESH.1)),
        ),
    ] {
        let (amp, phase) = largest_change(&chosen, &finer);
        println!(
            "finer {what}: amplitudes {:.3} %, phases {phase:.3}°",
            100.0 * amp
        );
        assert!(amp <= 0.005 && phase <= 0.5, "{what}: {amp}, {phase}°");
    }
}
