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

/// `f64::max` drops a NaN, so a NaN gap would never raise a largest gap and a band would pass on
/// it; the gap helpers refuse non-finite inputs instead.
#[test]
#[should_panic(expected = "not finite")]
fn amplitude_gap_refuses_nan() {
    amp_gap(f64::NAN, 1.0, 0.1);
}

#[test]
#[should_panic(expected = "not finite")]
fn phase_gap_refuses_nan() {
    phase_gap_deg(10.0, f64::NAN);
}

/// A NaN amplitude also fails `amp >= floor`, which would skip its phase without a word, and a
/// NaN where the tank cell is blank meets no gap at all: so every value SeaCraft returns is
/// checked finite before any comparison (`run` calls `assert_finite`).
#[test]
fn computed_values_must_be_finite() {
    let (rep, loads) = run(&WIGLEY_I, 0.3, vec![omega_for_wavelength(1.0)], 21, (40, 8));
    assert_finite(&rep, &loads);
    let mut bad = rep.clone();
    bad.frequencies[0].pitch_phase_deg = f64::NAN;
    let caught = std::panic::catch_unwind(|| assert_finite(&bad, &loads));
    assert!(caught.is_err(), "a NaN pitch phase passed");
    let mut bad = loads.clone();
    bad[0].heave_force.0 = f64::INFINITY;
    let caught = std::panic::catch_unwind(|| assert_finite(&rep, &bad));
    assert!(caught.is_err(), "an infinite heave force passed");
    let mut bad = rep.clone();
    bad.coefficients.as_mut().expect("coefficients")[0].b55 = f64::NAN;
    let caught = std::panic::catch_unwind(|| assert_finite(&bad, &loads));
    assert!(caught.is_err(), "a NaN B55 passed");
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

/// The restoring Journée's A55 reduction leaves out (§4.1 subtracts C55 = 2ρg∫y_w·x²dx alone):
/// ρg∇(KB − KG), with his printed ∇ and KG and the formula's KB. His forced-pitch model turned about
/// KR = 0.1875 m and §4.1 says nothing of compensating the buoyancy and weight moments about it, so
/// if the model weighed ρ∇ with G at the printed KG, his A55″ holds an offset of this over
/// ω²ρ∇L². Wigley I −58.6 N·m, Wigley III −40.4 N·m.
fn c55_missing(m: &Model) -> f64 {
    let kb = if m.alpha == 1.0 { 0.106893 } else { 0.117188 };
    RHO * G * m.volume * (kb - m.kg)
}

/// Bands on the gaps of the radiation coefficients at ω ≥ 4 rad/s, per (model, Fn, coefficient):
/// the larger of the two oscillation amplitudes' gaps at each frequency, largest over the
/// frequencies, rounded up to the next 0.01. The gaps are strip theory's, not only SeaCraft's:
/// Journée's own strip-theory curves (his Figures 2, 5 and 6, read by hand, not used here) show
/// B55 too high at ω 4 to 7 rad/s and too low at 12, B33 too low at 10 and 12, and A55 too low
/// from 6 rad/s, as SeaCraft does (maintainer's ruling, 2026-11-20: band them).
///
/// The rows at ω ≈ 3 rad/s (and 1, 2 rad/s on Wigley III at Fn 0.2) are printed but not banded:
/// there τ = ω_e·U/g is 0.33 to 0.66, around strip theory's limit of 1/4, several measured
/// coefficients jump between neighbouring frequencies, and their gaps (up to 23 times the floor)
/// would loosen the bands past catching a doubled coefficient.
const COEFFICIENT_BANDS: &[(&str, f64, &str, f64)] = &[
    // Wigley I, Fn 0.2
    ("Wigley I", 0.2, "A33", 0.11),
    ("Wigley I", 0.2, "B33", 0.61),
    ("Wigley I", 0.2, "A53", 1.41),
    ("Wigley I", 0.2, "B53", 1.32),
    ("Wigley I", 0.2, "A55", 0.33),
    ("Wigley I", 0.2, "B55", 0.53),
    ("Wigley I", 0.2, "A35", 2.32),
    ("Wigley I", 0.2, "B35", 2.91),
    // Wigley I, Fn 0.3
    ("Wigley I", 0.3, "A33", 0.08),
    ("Wigley I", 0.3, "B33", 0.65),
    ("Wigley I", 0.3, "A53", 2.89),
    ("Wigley I", 0.3, "B53", 0.90),
    ("Wigley I", 0.3, "A55", 0.35),
    ("Wigley I", 0.3, "B55", 1.03),
    ("Wigley I", 0.3, "A35", 2.13),
    ("Wigley I", 0.3, "B35", 1.19),
    // Wigley I, Fn 0.4
    ("Wigley I", 0.4, "A33", 0.13),
    ("Wigley I", 0.4, "B33", 0.70),
    ("Wigley I", 0.4, "A53", 4.62),
    ("Wigley I", 0.4, "B53", 0.93),
    ("Wigley I", 0.4, "A55", 0.37),
    ("Wigley I", 0.4, "B55", 1.78),
    ("Wigley I", 0.4, "A35", 1.43),
    ("Wigley I", 0.4, "B35", 0.36),
    // Wigley III, Fn 0.2
    ("Wigley III", 0.2, "A33", 0.17),
    ("Wigley III", 0.2, "B33", 0.30),
    ("Wigley III", 0.2, "A53", 1.08),
    ("Wigley III", 0.2, "B53", 2.00),
    ("Wigley III", 0.2, "A55", 0.22),
    ("Wigley III", 0.2, "B55", 0.57),
    ("Wigley III", 0.2, "A35", 1.02),
    ("Wigley III", 0.2, "B35", 1.82),
    // Wigley III, Fn 0.3
    ("Wigley III", 0.3, "A33", 0.16),
    ("Wigley III", 0.3, "B33", 0.29),
    ("Wigley III", 0.3, "A53", 2.83),
    ("Wigley III", 0.3, "B53", 1.52),
    ("Wigley III", 0.3, "A55", 0.33),
    ("Wigley III", 0.3, "B55", 0.91),
    ("Wigley III", 0.3, "A35", 2.00),
    ("Wigley III", 0.3, "B35", 1.73),
    // Wigley III, Fn 0.4
    ("Wigley III", 0.4, "A33", 0.19),
    ("Wigley III", 0.4, "B33", 0.45),
    ("Wigley III", 0.4, "A53", 3.98),
    ("Wigley III", 0.4, "B53", 1.50),
    ("Wigley III", 0.4, "A55", 0.35),
    ("Wigley III", 0.4, "B55", 1.50),
    ("Wigley III", 0.4, "A35", 1.60),
    ("Wigley III", 0.4, "B35", 1.15),
];

/// The lowest frequency the coefficient bands hold, in rad/s.
const BANDED_FROM: f64 = 4.0 - 0.1;

fn band(bands: &[(&str, f64, &str, f64)], model: &str, froude: f64, what: &str) -> f64 {
    bands
        .iter()
        .find(|b| b.0 == model && b.1 == froude && b.2 == what)
        .map_or(f64::INFINITY, |b| b.3)
}

/// The radiation coefficients of one model and speed from one forced-oscillation table:
/// (name, measured, SeaCraft) per row, in Journée's non-dimensional form and signs.
type Compared = Vec<(&'static str, Vec<(f64, f64, f64, f64)>)>;

fn coefficients_against(m: &Model, froude: f64, table: &[ForcedRow], heave: bool) -> Compared {
    let rows: Vec<&ForcedRow> = table.iter().filter(|r| r.froude == froude).collect();
    let u = speed(froude);
    let omegas = rows
        .iter()
        .map(|r| omega_for_encounter(omega_from_nd(r.omega_nd), u, G))
        .collect();
    let (rep, _) = run(m, froude, omegas, STATIONS, MESH);
    let got: Vec<_> = rep
        .coefficients
        .expect("coefficients")
        .iter()
        .map(|c| journee_coefficients(c, m))
        .collect();
    // (ω_e, oscillation amplitude, measured, SeaCraft)
    let column = |want: fn(&ForcedRow) -> Option<f64>, have: fn(&Coefficients) -> f64| {
        rows.iter()
            .zip(&got)
            .filter_map(|(r, c)| Some((omega_from_nd(r.omega_nd), r.amplitude, want(r)?, have(c))))
            .collect::<Vec<_>>()
    };
    if heave {
        vec![
            ("A33", column(|r| r.a_direct, |c| c.a33)),
            ("B33", column(|r| r.b_direct, |c| c.b33)),
            ("A53", column(|r| r.a_coupled, |c| c.a53)),
            ("B53", column(|r| r.b_coupled, |c| c.b53)),
        ]
    } else {
        vec![
            ("A55", column(|r| r.a_direct, |c| c.a55)),
            ("B55", column(|r| r.b_direct, |c| c.b55)),
            ("A35", column(|r| r.a_coupled, |c| c.a35)),
            ("B35", column(|r| r.b_coupled, |c| c.b35)),
        ]
    }
}

#[test]
fn radiation_coefficients_against_forced_oscillation() {
    let models = [
        (&WIGLEY_I, TABLE_2_I, TABLE_3_I),
        (&WIGLEY_III, TABLE_2_III, TABLE_3_III),
    ];
    let mut failures = Vec::new();
    for (m, heave_table, pitch_table) in models {
        for froude in [0.2, 0.3, 0.4] {
            let mut groups = coefficients_against(m, froude, heave_table, true);
            groups.extend(coefficients_against(m, froude, pitch_table, false));
            for (what, rows) in groups {
                let floor = floor_of(rows.iter().map(|r| r.2));
                let limit = band(COEFFICIENT_BANDS, m.name, froude, what);
                println!(
                    "{} Fn {froude} {what} (floor {floor:.4}, band {limit}):",
                    m.name
                );
                println!(
                    "   ω_e   amp    measured  SeaCraft    gap  {}",
                    if what == "A55" { "corrected gap" } else { "" }
                );
                // Pair the two amplitudes at each nominal frequency.
                let mut largest: f64 = 0.0;
                for nominal in FORCED_OMEGAS {
                    let at: Vec<_> = rows
                        .iter()
                        .filter(|r| (r.0 - nominal).abs() < 0.1)
                        .collect();
                    if at.is_empty() {
                        continue;
                    }
                    let mut gaps = Vec::new();
                    for &&(w, amp, want, got) in &at {
                        let gap = amp_gap(got, want, floor);
                        gaps.push(gap);
                        let mut extra = String::new();
                        if what == "A55" {
                            let shift = c55_missing(m) / (w * w * RHO * m.volume * L * L);
                            let c = amp_gap(got, want + shift, floor);
                            extra = format!("{c:7.3} (A55″ {:+.4})", want + shift);
                        }
                        println!("  {w:5.2} {amp:5.3} {want:9.4} {got:9.4} {gap:7.3} {extra}");
                    }
                    if nominal < BANDED_FROM {
                        println!("  (not banded: below 4 rad/s)");
                        continue;
                    }
                    largest = gaps.iter().copied().fold(largest, f64::max);
                    // A damping coupling above its floor with the wrong sign is a convention
                    // error, not strip theory: B35 and B53 carry ±U·A33, so a wrong pitch sign
                    // shows at every frequency. The one stop rule kept for the coefficients
                    // (maintainer's ruling, 2026-11-20). A35 and A53 are banded only: the tank's
                    // A53 on Wigley I at Fn 0.4 turns negative from 7 rad/s, where strip theory's
                    // U·B33/ω_e² (exact on these symmetric hulls) cannot.
                    if what == "B35" || what == "B53" {
                        for &&(w, _, want, got) in &at {
                            if want.abs() >= floor && want.signum() != got.signum() {
                                failures.push(format!(
                                    "STOP RULE {} Fn {froude} {what} ω {w:.2}: sign {want} against {got:.4}",
                                    m.name
                                ));
                            }
                        }
                    }
                }
                println!("  largest gap {largest:.5}");
                if largest > limit {
                    failures.push(format!(
                        "{} Fn {froude} {what}: {largest:.3} > band {limit}",
                        m.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Bands on the wave-load gaps per (model, Fn, quantity), over every row and both wave amplitudes:
/// amplitudes relative (with the group floor), next 0.01 above the measured gap; phases in degrees,
/// next 2° above, where both the tank's and SeaCraft's amplitudes reach the floor. That leaves out
/// the short-wave cancellation of the heave force on Wigley III (λ/L 0.75 at every speed and 0.5 at
/// Fn 0.3 and 0.4, where SeaCraft's Xw3″ is 0.025 to 0.040 against the tank's 0.06 to 0.12): there the computed phase
/// swings fast, and at Fn 0.4 it is 104° from the tank's, as Journée's own strip theory is
/// (maintainer's ruling, 2026-11-20). The rows stay printed, marked `(fl)`.
const LOAD_BANDS: &[(&str, f64, &str, f64)] = &[
    // Wigley I, Fn 0.2
    ("Wigley I", 0.2, "Xw3", 0.24),
    ("Wigley I", 0.2, "eps3", 30.0),
    ("Wigley I", 0.2, "Xw5", 0.13),
    ("Wigley I", 0.2, "eps5", 44.0),
    // Wigley I, Fn 0.3
    ("Wigley I", 0.3, "Xw3", 0.52),
    ("Wigley I", 0.3, "eps3", 46.0),
    ("Wigley I", 0.3, "Xw5", 0.31),
    ("Wigley I", 0.3, "eps5", 48.0),
    // Wigley I, Fn 0.4
    ("Wigley I", 0.4, "Xw3", 0.69),
    ("Wigley I", 0.4, "eps3", 24.0),
    ("Wigley I", 0.4, "Xw5", 0.57),
    ("Wigley I", 0.4, "eps5", 82.0),
    // Wigley III, Fn 0.2
    ("Wigley III", 0.2, "Xw3", 0.59),
    ("Wigley III", 0.2, "eps3", 22.0),
    ("Wigley III", 0.2, "Xw5", 0.13),
    ("Wigley III", 0.2, "eps5", 28.0),
    // Wigley III, Fn 0.3
    ("Wigley III", 0.3, "Xw3", 0.73),
    ("Wigley III", 0.3, "eps3", 30.0),
    ("Wigley III", 0.3, "Xw5", 0.25),
    ("Wigley III", 0.3, "eps5", 22.0),
    // Wigley III, Fn 0.4
    ("Wigley III", 0.4, "Xw3", 0.71),
    ("Wigley III", 0.4, "eps3", 22.0),
    ("Wigley III", 0.4, "Xw5", 0.43),
    ("Wigley III", 0.4, "eps5", 14.0),
];

/// Before a band is frozen: amplitudes are measured only, and a phase more than 90° off is a wrong
/// quadrant, which is a stop-rule case.
fn load_band(model: &str, froude: f64, what: &str) -> f64 {
    let b = band(LOAD_BANDS, model, froude, what);
    if b.is_finite() || !what.starts_with("eps") {
        b
    } else {
        90.0
    }
}

#[test]
fn wave_loads_against_the_restrained_model() {
    let mut failures = Vec::new();
    for (m, table) in [(&WIGLEY_I, TABLE_6_I), (&WIGLEY_III, TABLE_6_III)] {
        for froude in [0.2, 0.3, 0.4] {
            let rows: Vec<&WaveLoadRow> = table.iter().filter(|r| r.froude == froude).collect();
            let omegas = rows
                .iter()
                .map(|r| omega_for_wavelength(r.lambda_over_l))
                .collect();
            let (_, loads) = run(m, froude, omegas, STATIONS, MESH);
            // SeaCraft's Xw3″, ε3, Xw5″, ε5 in Journée's form (pitch moment bow down).
            let got: Vec<(f64, f64, f64, f64)> = loads
                .iter()
                .map(|l| {
                    let k = l.omega * l.omega / G;
                    (
                        l.heave_force.0 / m.c33,
                        l.heave_force.1.rem_euclid(360.0),
                        l.pitch_moment.0 / (k * m.c55),
                        bow_down_deg(l.pitch_moment.1),
                    )
                })
                .collect();
            let floor3 = floor_of(rows.iter().filter_map(|r| r.xw3));
            let floor5 = floor_of(rows.iter().filter_map(|r| r.xw5));
            println!(
                "{} Fn {froude} (floors Xw3″ {floor3:.3}, Xw5″ {floor5:.3}):",
                m.name
            );
            println!("  λ/L   ζa cm   Xw3″ tank  SeaCraft  gap    ε3 tank SeaCraft gap    Xw5″ tank  SeaCraft  gap    ε5 tank SeaCraft gap");
            let mut largest = [0.0f64; 4];
            for (r, g) in rows.iter().zip(&got) {
                let mut line = format!("  {:4.2} {:5.2}", r.lambda_over_l, r.zeta_a_cm);
                let quantities = [
                    (r.xw3, g.0, r.eps3_deg, g.1, floor3),
                    (r.xw5, g.2, r.eps5_deg, g.3, floor5),
                ];
                for (k, (amp, got_amp, eps, got_eps, floor)) in quantities.into_iter().enumerate() {
                    match amp {
                        Some(want) => {
                            let gap = amp_gap(got_amp, want, floor);
                            largest[2 * k] = largest[2 * k].max(gap);
                            line += &format!("  {want:8.3} {got_amp:8.3} {gap:6.3}");
                        }
                        None => line += &format!("  {:>8} {got_amp:8.3} {:>6}", "-", "-"),
                    }
                    match (eps, amp) {
                        (Some(want), Some(a)) if a >= floor && got_amp >= floor => {
                            let gap = phase_gap_deg(got_eps, want);
                            largest[2 * k + 1] = largest[2 * k + 1].max(gap);
                            line += &format!("  {want:6.0} {got_eps:7.1} {gap:5.1}");
                        }
                        (Some(want), _) => {
                            line += &format!("  {want:6.0} {got_eps:7.1} {:>5}", "(fl)")
                        }
                        (None, _) => line += &format!("  {:>6} {got_eps:7.1} {:>5}", "-", "-"),
                    }
                }
                println!("{line}");
            }
            for (what, gap) in ["Xw3", "eps3", "Xw5", "eps5"].iter().zip(largest) {
                let limit = load_band(m.name, froude, what);
                println!("  largest {what} gap {gap:.5} (band {limit})");
                if gap > limit {
                    failures.push(format!(
                        "{} Fn {froude} {what}: {gap:.3} > band {limit}",
                        m.name
                    ));
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}

/// Bands on the motion gaps per (model, Fn, quantity), over every row: amplitudes relative (with
/// the group floor), next 0.01 above the measured gap; phases in degrees, next 2° above, where both
/// the tank's and SeaCraft's amplitudes reach the floor.
const MOTION_BANDS: &[(&str, f64, &str, f64)] = &[
    // Wigley I, Fn 0.2
    ("Wigley I", 0.2, "z", 0.49),
    ("Wigley I", 0.2, "eps_z", 16.0),
    ("Wigley I", 0.2, "theta", 0.35),
    ("Wigley I", 0.2, "eps_theta", 24.0),
    // Wigley I, Fn 0.3
    ("Wigley I", 0.3, "z", 0.19),
    ("Wigley I", 0.3, "eps_z", 34.0),
    ("Wigley I", 0.3, "theta", 0.54),
    ("Wigley I", 0.3, "eps_theta", 32.0),
    // Wigley I, Fn 0.4
    ("Wigley I", 0.4, "z", 0.33),
    ("Wigley I", 0.4, "eps_z", 40.0),
    ("Wigley I", 0.4, "theta", 0.81),
    ("Wigley I", 0.4, "eps_theta", 40.0),
    // Wigley III, Fn 0.2
    ("Wigley III", 0.2, "z", 0.47),
    ("Wigley III", 0.2, "eps_z", 48.0),
    ("Wigley III", 0.2, "theta", 0.32),
    ("Wigley III", 0.2, "eps_theta", 22.0),
    // Wigley III, Fn 0.3
    ("Wigley III", 0.3, "z", 0.87),
    ("Wigley III", 0.3, "eps_z", 42.0),
    ("Wigley III", 0.3, "theta", 0.68),
    ("Wigley III", 0.3, "eps_theta", 44.0),
    // Wigley III, Fn 0.4
    ("Wigley III", 0.4, "z", 0.47),
    ("Wigley III", 0.4, "eps_z", 22.0),
    ("Wigley III", 0.4, "theta", 1.15),
    ("Wigley III", 0.4, "eps_theta", 58.0),
];

/// The stop rule for the motions: at Fn 0.2 and 0.3, λ/L ≥ 1.25 and away from resonance (more
/// than 0.15 in λ/L from the largest measured amplitude of the group), an amplitude more than 30 %
/// or a phase more than 30° from the tank suggests a fault rather than strip theory.
const MOTION_STOP: (f64, f64) = (0.30, 30.0);

/// Rows the motion stop rule fired on, banded instead (maintainer's ruling, 2026-11-20): (model,
/// Fn, motion, λ/L). Strip theory puts Wigley I's pitch peak at shorter waves than the tank (λ/L
/// 1.25 against 1.5 at Fn 0.3; Journée's own strip theory does too), from an A55 about 30 % low
/// and a B55 too high at the same encounter frequencies (the coefficient bands); at Fn 0.2, λ/L
/// 1.75 the tank's θ″ of 1.09 stands above both neighbours (0.95 at 1.6, 0.68 at 2.0).
const MOTION_STOP_EXEMPT: &[(&str, f64, &str, f64)] = &[
    ("Wigley I", 0.3, "theta", 1.25),
    ("Wigley I", 0.3, "theta", 1.75),
    ("Wigley I", 0.2, "theta", 1.75),
];

#[test]
fn motions_against_the_free_model() {
    let mut failures = Vec::new();
    for (m, table) in [(&WIGLEY_I, TABLE_8_I), (&WIGLEY_III, TABLE_8_III)] {
        for froude in [0.2, 0.3, 0.4] {
            let rows: Vec<&MotionRow> = table.iter().filter(|r| r.froude == froude).collect();
            let omegas = rows
                .iter()
                .map(|r| omega_for_wavelength(r.lambda_over_l))
                .collect();
            let (rep, _) = run(m, froude, omegas, STATIONS, MESH);
            // SeaCraft's z″, ε_z, θ″, ε_θ in Journée's form (θ″ = θa/(kζa)·L/λ, bow down).
            let got: Vec<(f64, f64, f64, f64)> = rep
                .frequencies
                .iter()
                .map(|p| {
                    (
                        p.heave_amp,
                        p.heave_phase_deg.rem_euclid(360.0),
                        p.pitch_amp / p.lambda_over_l,
                        bow_down_deg(p.pitch_phase_deg),
                    )
                })
                .collect();
            for (k, name) in ["z", "theta"].into_iter().enumerate() {
                let want_amp = |r: &MotionRow| if k == 0 { r.z } else { r.theta };
                let want_eps = |r: &MotionRow| if k == 0 { r.eps_z_deg } else { r.eps_theta_deg };
                let have = |g: &(f64, f64, f64, f64)| if k == 0 { (g.0, g.1) } else { (g.2, g.3) };
                let floor = floor_of(rows.iter().filter_map(|r| want_amp(r)));
                let peak = rows
                    .iter()
                    .filter_map(|r| Some((want_amp(r)?, r.lambda_over_l)))
                    .fold((0.0, 0.0), |a, b| if b.0 > a.0 { b } else { a })
                    .1;
                let exempt = |l: f64| {
                    MOTION_STOP_EXEMPT
                        .iter()
                        .any(|e| e.0 == m.name && e.1 == froude && e.2 == name && e.3 == l)
                };
                let near_peak = |l: f64| (l - peak).abs() <= 0.15 + 1e-9;
                let stop_region =
                    |l: f64| froude < 0.35 && l >= 1.25 - 1e-9 && !near_peak(l) && !exempt(l);
                println!(
                    "{} Fn {froude} {name}″ (floor {floor:.3}, peak at λ/L {peak}):",
                    m.name
                );
                println!("   λ/L  ζa cm    tank  SeaCraft    gap    ε tank SeaCraft   gap");
                let (mut largest_amp, mut largest_eps) = (0.0f64, 0.0f64);
                for (r, g) in rows.iter().zip(&got) {
                    let (amp, eps) = have(g);
                    let l = r.lambda_over_l;
                    let note = if stop_region(l) {
                        ""
                    } else if froude < 0.35 && l >= 1.25 - 1e-9 && near_peak(l) {
                        "  (near resonance: no stop rule)"
                    } else if froude < 0.35 && exempt(l) {
                        "  (named: no stop rule)"
                    } else {
                        ""
                    };
                    let mut line = format!("  {l:5.3} {:5.2}", r.zeta_a_cm);
                    match want_amp(r) {
                        Some(want) => {
                            let gap = amp_gap(amp, want, floor);
                            largest_amp = largest_amp.max(gap);
                            line += &format!("  {want:6.2} {amp:8.3} {gap:6.3}");
                            if stop_region(l) && gap > MOTION_STOP.0 {
                                failures.push(format!(
                                    "STOP RULE {} Fn {froude} {name}″ λ/L {l}: {want} against {amp:.3}",
                                    m.name
                                ));
                            }
                        }
                        None => line += &format!("  {:>6} {amp:8.3} {:>6}", "-", "-"),
                    }
                    match (want_eps(r), want_amp(r)) {
                        (Some(want), Some(a)) if a >= floor && amp >= floor => {
                            let gap = phase_gap_deg(eps, want);
                            largest_eps = largest_eps.max(gap);
                            line += &format!("  {want:6.0} {eps:7.1} {gap:5.1}");
                            if stop_region(l) && gap > MOTION_STOP.1 {
                                failures.push(format!(
                                    "STOP RULE {} Fn {froude} ε_{name} λ/L {l}: {want}° against {eps:.1}°",
                                    m.name
                                ));
                            }
                        }
                        (Some(want), _) => line += &format!("  {want:6.0} {eps:7.1} {:>5}", "(fl)"),
                        (None, _) => line += &format!("  {:>6} {eps:7.1} {:>5}", "-", "-"),
                    }
                    println!("{line}{note}");
                }
                for (what, gap) in [
                    (name, largest_amp),
                    (&format!("eps_{name}")[..], largest_eps),
                ] {
                    let limit = band(MOTION_BANDS, m.name, froude, what);
                    println!("  largest {what} gap {gap:.5} (band {limit})");
                    if gap > limit {
                        failures.push(format!(
                            "{} Fn {froude} {what}: {gap:.3} > band {limit}",
                            m.name
                        ));
                    }
                }
            }
        }
    }
    assert!(failures.is_empty(), "{failures:#?}");
}
