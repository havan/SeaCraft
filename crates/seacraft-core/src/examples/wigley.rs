//! The Wigley parabolic hull: a hydrostatics benchmark with closed-form
//! answers (CLI and tests only).
//!
//! `y = (B/2) (1 − ξ²) (1 − ζ²)` below the waterline, with `ξ = 2x/L − 1`
//! and `ζ = (T − z)/T`, and wall-sided above it (`y = (B/2)(1 − ξ²)`) up to
//! a flat deck at `D`. L = 10 m, B = 1 m, T = 0.625 m, D = 1 m. Integrating
//! the offsets (our own arithmetic):
//!
//! * `∫₋₁¹(1 − ξ²) dξ / 2 = 2/3` and `∫₀¹(1 − ζ²) dζ = 2/3`, so
//!   `Cb = 4/9`, `Cm = Cwp = 2/3` and `Cp = Cb / Cm = 2/3`;
//! * the centroid of `1 − ζ²` on `0..1` is `ζ = (1/4)/(2/3) = 3/8`, so
//!   `KB = T − 3T/8 = 0.625 T`;
//! * `I_T = (2/3) ∫ y_wl³ dx = (2/3)(B/2)³ (L/2)(32/35) = 4 L B³ / 105`
//!   (`∫₋₁¹(1 − ξ²)³ dξ = 32/35`) and `∇ = (4/9) L B T`, so
//!   `BM = 9 B² / (105 T) = 0.0857 B²/T`;
//! * numerically `∇ = 2.7778 m³` (Δ 2.847 t in sea water), `KB = 0.3906 m`,
//!   `BM = 0.1371 m`, `KM = 0.5278 m`.
//!
//! The control points sit on the analytic offsets with each row's
//! half-breadths scaled by [`ROW_SCALE`]: factors frozen at authoring time
//! (the ignored `fit_rows` test re-derives them) that undo the
//! subdivision's inward pull, so the level-3 vertices of the control points
//! land on the analytic surface. Without them Cb comes out 1.2 % low. The
//! keel, the stems and the deck centreline lie on the half model's boundary,
//! which the kernel pins, so L and T need no correction.

use super::{
    close_half_deck, crease_path, design_condition, item, layer, round, round_to_fef, settings,
    station_grid, Built, Example,
};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};

pub(super) const L: f64 = 10.0;
pub(super) const B: f64 = 1.0;
pub(super) const T: f64 = 0.625;
const D: f64 = 1.0;
/// The example's KG above the keel (m): GM ≈ 0.18 m.
const KG: f64 = 0.35;
/// Froude number 0.3 on L = 10 m, in knots.
const SPEED_KN: f64 = 5.78;
/// Stations over the length (both ends on the centreline).
const STATIONS: usize = 21;
/// Row heights as fractions of T: keel, five below the waterline, the
/// waterline, then the wall-sided topside to the deck.
const ROWS: [f64; 9] = [0.0, 0.12, 0.3, 0.5, 0.7, 0.87, 1.0, 1.3, 1.6];
/// Δ of the refined hull at T (t), measured at authoring time and frozen;
/// the Design condition loads to it.
const DESIGN_DISPLACEMENT_T: f64 = 2.833;
/// Half-breadth correction per row for the subdivision's inward pull
/// (frozen at authoring time).
const ROW_SCALE: [f64; 9] = [
    1.0, 0.9282, 1.0280, 1.0219, 1.0221, 1.0143, 1.0067, 1.0042, 1.0047,
];

fn half_breadth(x: f64, z: f64) -> f64 {
    let xi = 2.0 * x / L - 1.0;
    let zeta = ((T - z) / T).max(0.0);
    0.5 * B * (1.0 - xi * xi) * (1.0 - zeta * zeta)
}

pub(super) fn build() -> Built {
    build_with(&ROW_SCALE)
}

fn build_with(row_scale: &[f64; 9]) -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Hull", 0x9a_b8_d8, false),
        layer(2, "Deck", 0xc8_b4_8c, false),
    ];
    s.settings = settings(Example::Wigley, L, B, T);
    let stations: Vec<Vec<Vec3>> = (0..STATIONS)
        .map(|i| {
            let x = L * i as f64 / (STATIONS - 1) as f64;
            ROWS.iter()
                .zip(row_scale)
                .map(|(&r, &k)| {
                    let z = (r * T).min(D);
                    Vec3::new(x, k * half_breadth(x, z), z)
                })
                .collect()
        })
        .collect();
    let grid = station_grid(&mut s, &stations, |_, _| 0);
    let sheer: Vec<usize> = grid.iter().map(|st| *st.last().unwrap()).collect();
    close_half_deck(&mut s, 1, &sheer);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &sheer);

    let project = Project {
        kg: Some(KG),
        requirements: Some(Requirements {
            name: "Wigley hull".into(),
            lwl: Some(L),
            bwl: Some(B),
            draft: Some(T),
            depth: Some(D),
            displacement_t: Some(round(1.025 * 4.0 / 9.0 * L * B * T, 3)),
            cb: Some(round(4.0 / 9.0, 4)),
            lcb_frac: Some(0.5),
            design_speed_kn: Some(SPEED_KN),
            kg: None,
        }),
        loading_conditions: vec![design_condition(
            DESIGN_DISPLACEMENT_T,
            0.5 * L,
            KG,
            "Model (hull and fittings)",
            vec![item("Ballast weights", 0.8, 0.5 * L, 0.2)],
        )],
        report: ReportSettings {
            kg: Some(KG),
            lcg_x: Some(0.5 * L),
            speeds_kn: Some(vec![3.0, 4.0, 5.0, 5.78, 7.0]),
            ..Default::default()
        },
        ..Default::default()
    };
    (s, project)
}

#[cfg(test)]
mod tests {
    use super::super::checks;
    use super::*;

    #[test]
    #[ignore]
    fn fit_rows() {
        let mut k = [1.0f64; 9];
        for it in 0..8 {
            let (s, _) = build_with(&k);
            let m = crate::subdiv::evaluate(&s, checks::level3());
            for (j, &r) in ROWS.iter().enumerate().skip(1) {
                let mut num = 0.0;
                let mut den = 0.0;
                for i in 1..STATIONS - 1 {
                    let pi = i * ROWS.len() + j;
                    let x = s.points[pi].p.x;
                    let z = (r * T).min(D);
                    num += half_breadth(x, z);
                    den += m.verts[pi].y;
                }
                k[j] *= num / den;
            }
            let h = checks::measure(&build_with(&k).0);
            println!(
                "it {it}: {k:?} BWL {:.5} Cb {:.5} Cm {:.5} Cp {:.5} Cwp {:.5} KB {:.5} BM {:.5}",
                h.bwl,
                h.block_coefficient,
                h.midship_coefficient,
                h.prismatic_coefficient,
                h.waterplane_coefficient,
                h.kb,
                h.bm_transverse
            );
        }
    }

    /// Closed form (module header): coefficients within 1 %, KB and BM
    /// within 2 %.
    #[test]
    fn wigley_matches_the_closed_form() {
        let (s, _) = build();
        let h = checks::measure(&s);
        let rel = |got: f64, want: f64| (got - want).abs() / want;
        let kb = 0.625 * T;
        let bm = 9.0 * B * B / (105.0 * T);
        println!(
            "wigley: Cb {:.5} Cp {:.5} Cm {:.5} Cwp {:.5} KB {:.5} ({kb:.5}) BM {:.5} ({bm:.5})",
            h.block_coefficient,
            h.prismatic_coefficient,
            h.midship_coefficient,
            h.waterplane_coefficient,
            h.kb,
            h.bm_transverse
        );
        assert!(
            rel(h.block_coefficient, 4.0 / 9.0) < 0.01,
            "Cb {}",
            h.block_coefficient
        );
        for (q, v) in [
            ("Cp", h.prismatic_coefficient),
            ("Cm", h.midship_coefficient),
            ("Cwp", h.waterplane_coefficient),
        ] {
            assert!(rel(v, 2.0 / 3.0) < 0.01, "{q} {v}");
        }
        assert!(rel(h.kb, kb) < 0.02, "KB {} vs {kb}", h.kb);
        assert!(
            rel(h.bm_transverse, bm) < 0.02,
            "BM {} vs {bm}",
            h.bm_transverse
        );
        assert!((h.lwl - L).abs() < 1e-9 && rel(h.bwl, B) < 0.01);
    }

    #[test]
    fn wigley_meets_the_board_targets() {
        let (s, p) = build();
        let h = checks::measure(&s);
        checks::assert_targets(
            "wigley",
            &h,
            &checks::Targets {
                lwl: L,
                bwl: B,
                displacement_t: 1.025 * 4.0 / 9.0 * L * B * T,
                cb: 4.0 / 9.0,
                cp: 2.0 / 3.0,
                cm: 2.0 / 3.0,
                cwp: 2.0 / 3.0,
                lcb_frac: 0.5,
            },
        );
        // The frozen Design displacement and LCG are the measured ones.
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-3);
        assert!((h.lcb.x - 0.5 * L).abs() < 1e-6);
        assert_eq!(p.requirements.as_ref().unwrap().lwl, Some(L));
    }

    #[test]
    fn wigley_is_closed_stable_and_floats_level() {
        let (s, p) = build();
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "keel at z = 0");
        checks::assert_closed_below(&s);
        checks::assert_stability(&s, KG, 0.5 * L);
        checks::assert_free_trim(&s, &p);
    }

    #[test]
    fn wigley_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
    }
}
