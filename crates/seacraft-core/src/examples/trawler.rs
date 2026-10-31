//! 22 m steel stern trawler: a round-bilge fishing vessel with a skeg over
//! the aft third, an immersed transom raked 25° (top aft), a raked stem, a
//! sheer rising forward, a closed deck and a 1 m bulwark.
//!
//! Targets (examples board, Opus table; "representative of vessels of this
//! size", not any one boat): LOA 22.0 m, LWL 19.5 m, BWL 6.4 m (BOA 6.8 m),
//! T 2.6 m, D 3.4 m, Δ 173 t, Cb 0.52, Cp 0.62, Cm 0.84, Cwp 0.80, LCB 47 %
//! LWL from aft, 11 kn (Fn 0.41), KG 2.50 m (assumed, about 0.74 D).
//!
//! The draft is measured from the skeg bottom (the lowest hydrostatic
//! point, `z = 0`). Amidships the canoe-body keel is at `z = 0.30` with a
//! 0.05 m bar keel below it; over the aft third the skeg carries the keel
//! line 0.3 m below that, and deepens into the deadwood where the run
//! rises to the transom. With the canoe body 2.3 m deep amidships, Cm 0.84
//! on the full 2.6 m draft calls for a nearly flat floor and a bilge radius
//! of about 1 m.
//!
//! Layers: Hull, Transom, Deck (closed, so GZ at large angles is real) in
//! hydrostatics; Bulwark (a separate strip on top of the sheer, sharing no
//! edge with the hull so the sheer stays manifold) not in hydrostatics.
//! Creases: the transom outline, the sheer and the bulwark top; the keel,
//! skeg and stem are the half model's centreline boundary, which the kernel
//! pins.
//!
//! The station table was calibrated once against level-3 Loop hydrostatics
//! and frozen; [`build`] runs no solver.

use super::{
    close_half_deck, close_half_transom, crease_loop, crease_path, design_condition, item, layer,
    pin_crease_junctions, plating, round_to_fef, settings, station_grid, Built, Example, STEEL,
};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};

const LWL: f64 = 19.5;
const BWL: f64 = 6.4;
const T: f64 = 2.6;
const DEPTH: f64 = 3.4;
/// The example's KG above the keel (skeg bottom), m, assumed.
const KG: f64 = 2.50;
const SPEED_KN: f64 = 11.0;
/// Transom rake from the vertical (top aft), degrees.
const TRANSOM_RAKE_DEG: f64 = 25.0;
/// Bulwark height above the sheer (m).
const BULWARK_H: f64 = 1.0;
/// Δ of the refined hull at T (t), measured at authoring time and frozen.
const DESIGN_DISPLACEMENT_T: f64 = 173.87;
/// LCB of the refined hull at T (model x, m), measured and frozen: the
/// Design condition's LCG.
const DESIGN_LCG: f64 = 9.195;

/// Stations aft to forward:
/// `[x, keel z, skeg half-width, floor z at the skeg root, bilge half-breadth,
/// bilge z, bilge offset, WL half-breadth, sheer half-breadth, sheer z]`.
/// Station 0 is the transom (its `x` is where the transom plane meets the
/// waterline); the stem is [`STEM`].
const STATIONS: [[f64; 10]; 14] = [
    [0.0, 2.05, 0.08, 2.20, 2.10, 2.30, 0.15, 2.30, 2.80, 3.30],
    [1.2, 0.80, 0.10, 2.01, 2.45, 2.11, 0.35, 2.65, 3.10, 3.31],
    [2.6, 0.00, 0.10, 1.46, 2.70, 1.61, 0.60, 2.95, 3.28, 3.32],
    [4.2, 0.00, 0.10, 0.96, 2.95, 1.16, 0.80, 3.10, 3.36, 3.34],
    [5.8, 0.00, 0.10, 0.62, 3.12, 0.77, 0.95, 3.16, 3.40, 3.36],
    [7.4, 0.15, 0.09, 0.41, 3.16, 0.51, 1.00, 3.16, 3.40, 3.38],
    [9.0, 0.25, 0.08, 0.30, 3.16, 0.40, 1.00, 3.16, 3.40, 3.40],
    [10.6, 0.25, 0.08, 0.30, 3.15, 0.42, 1.05, 3.15, 3.40, 3.43],
    [12.2, 0.26, 0.08, 0.33, 2.93, 0.62, 1.10, 3.00, 3.36, 3.50],
    [13.8, 0.28, 0.08, 0.38, 2.50, 0.90, 1.10, 2.66, 3.25, 3.62],
    [15.3, 0.32, 0.07, 0.46, 1.96, 1.25, 0.90, 2.20, 3.02, 3.78],
    [16.7, 0.40, 0.06, 0.56, 1.35, 1.65, 0.70, 1.65, 2.65, 3.95],
    [17.9, 0.55, 0.05, 0.73, 0.80, 1.95, 0.45, 1.10, 2.10, 4.10],
    [18.8, 0.95, 0.04, 1.05, 0.32, 2.20, 0.20, 0.55, 1.40, 4.20],
];

/// The stem profile on the centreline, forefoot to stem head: `(x, z)` for
/// the eight hull rows (the last is the sheer), then the bulwark top.
const STEM: [(f64, f64); 9] = [
    (19.00, 1.15),
    (19.05, 1.30),
    (19.10, 1.50),
    (19.20, 1.80),
    (19.30, 2.10),
    (19.40, 2.35),
    (19.50, 2.60),
    (20.56, 4.25),
    (21.2073, 5.25),
];

/// The eight hull rows of a station, keel to sheer, as `(y, z)`: keel, skeg
/// bottom corner, skeg root, floor, bilge corner, upper bilge, waterline,
/// sheer.
fn section(st: &[f64; 10]) -> [(f64, f64); 8] {
    let [_, zk, ws, zr, hb, zc, r, hw, hs, zs] = *st;
    let y_floor = (hb - r).max(ws + 0.3 * (hb - ws));
    let z_floor = zr + (zc - zr) * (y_floor - ws) / (hb - ws);
    let t = (r / (T - zc)).min(0.7);
    [
        (0.0, zk),
        (ws, zk),
        (ws, zr),
        (y_floor, z_floor),
        (hb, zc),
        (hb + t * (hw - hb), zc + t * (T - zc)),
        (hw, T),
        (hs, zs),
    ]
}

pub(super) fn build() -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Hull", 0x1f_4e_79, false, plating(STEEL, 8.0)),
        layer(2, "Transom", 0x7a_20_20, false, plating(STEEL, 8.0)),
        layer(3, "Deck", 0xc8_b4_8c, false, plating(STEEL, 6.0)),
        layer(4, "Bulwark", 0xe6_e6_e6, false, plating(STEEL, 5.0)),
    ];
    s.layers[3].use_in_hydrostatics = false;
    s.settings = settings(Example::Trawler22m, LWL, BWL, T);

    // The transom is one plane through (x, z = T), raked top aft.
    let rake = TRANSOM_RAKE_DEG.to_radians().tan();
    let mut hull: Vec<Vec<Vec3>> = Vec::new();
    let mut bulwark: Vec<Vec<Vec3>> = Vec::new();
    for (i, st) in STATIONS.iter().enumerate() {
        let x_at = |z: f64| {
            if i == 0 {
                st[0] - (z - T) * rake
            } else {
                st[0]
            }
        };
        hull.push(
            section(st)
                .iter()
                .map(|&(y, z)| Vec3::new(x_at(z), y, z))
                .collect(),
        );
        let (hs, zs) = (st[8], st[9]);
        bulwark.push(vec![
            Vec3::new(x_at(zs), hs, zs),
            Vec3::new(x_at(zs + BULWARK_H), hs, zs + BULWARK_H),
        ]);
    }
    hull.push(
        STEM[..8]
            .iter()
            .map(|&(x, z)| Vec3::new(x, 0.0, z))
            .collect(),
    );
    bulwark.push(
        STEM[7..]
            .iter()
            .map(|&(x, z)| Vec3::new(x, 0.0, z))
            .collect(),
    );

    let grid = station_grid(&mut s, &hull, |_, _| 0);
    let sheer: Vec<usize> = grid.iter().map(|st| st[7]).collect();
    let centre = close_half_deck(&mut s, 2, &sheer);
    let transom = close_half_transom(&mut s, 1, &grid[0], centre[0]);
    // The bulwark has its own copy of the sheer points: a hull edge with
    // the side, the deck and the bulwark on it would be non-manifold.
    let bulwark = station_grid(&mut s, &bulwark, |_, _| 3);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &sheer);
    crease_loop(&mut s, &transom);
    let top: Vec<usize> = bulwark.iter().map(|st| st[1]).collect();
    crease_path(&mut s, &top);
    pin_crease_junctions(&mut s);

    let project = Project {
        kg: Some(KG),
        requirements: Some(Requirements {
            name: "22 m stern trawler".into(),
            lwl: Some(LWL),
            bwl: Some(BWL),
            draft: Some(T),
            depth: Some(DEPTH),
            displacement_t: Some(173.0),
            cb: Some(0.52),
            lcb_frac: Some(0.47),
            design_speed_kn: Some(SPEED_KN),
            kg: None,
        }),
        loading_conditions: vec![design_condition(
            DESIGN_DISPLACEMENT_T,
            DESIGN_LCG,
            KG,
            "Lightship (steel hull, machinery, outfit, gantry)",
            vec![
                item("Fuel oil", 14.0, 7.0, 1.0),
                item("Fresh water", 3.0, 16.0, 1.2),
                item("Catch in hold", 18.0, 11.0, 1.5),
                item("Ice", 5.0, 11.0, 1.6),
                item("Fishing gear (nets, doors)", 4.0, 1.5, 4.0),
                item("Crew (6) and stores", 1.5, 14.0, 4.5),
            ],
        )],
        report: ReportSettings {
            kg: Some(KG),
            lcg_x: Some(DESIGN_LCG),
            speeds_kn: Some(vec![7.0, 8.0, 9.0, 10.0, 11.0, 12.0]),
            include_resistance: Some(true),
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
    use crate::hydro::HydroOptions;

    /// The refined transom keeps its sheer corner and the side does not
    /// pinch in towards it (the first station forward is at 1.2 m).
    #[test]
    fn trawler_keeps_its_transom_outline() {
        let (s, _) = build();
        checks::assert_transom_outline(&s, STATIONS[1][0]);
    }

    #[test]
    fn trawler_meets_the_board_targets() {
        let (s, _) = build();
        let h = checks::measure(&s);
        checks::assert_targets(
            "trawler",
            &h,
            &checks::Targets {
                lwl: LWL,
                bwl: BWL,
                displacement_t: 173.0,
                cb: 0.52,
                cp: 0.62,
                cm: 0.84,
                cwp: 0.80,
                lcb_frac: 0.47,
            },
        );
        // The frozen Design displacement and LCG are the measured ones.
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-2);
        assert!((h.lcb.x - DESIGN_LCG).abs() < 1e-3);
        // Particulars the table does not test: BOA 6.8 m and LOA 22.0 m
        // over the bulwark, depth 3.4 m amidships.
        let (min, max) = s.extents();
        assert!((2.0 * max.y - 6.8).abs() < 1e-9, "BOA {}", 2.0 * max.y);
        assert!((max.x - min.x - 22.0).abs() < 1e-3, "LOA {}", max.x - min.x);
        let mid = STATIONS.iter().find(|st| st[0] == 9.0).unwrap();
        assert!((mid[9] - DEPTH).abs() < 1e-9);
    }

    #[test]
    fn trawler_has_a_skeg_a_flat_raked_transom_and_a_dry_bulwark() {
        let (s, _) = build();
        // Every transom point lies on one plane raked 25° (top aft).
        let rake = TRANSOM_RAKE_DEG.to_radians().tan();
        let transom = s.layers.iter().position(|l| l.name == "Transom").unwrap();
        let fi = s.face_layer.iter().position(|&l| l == transom).unwrap();
        for &p in &s.faces[fi] {
            let q = s.points[p].p;
            assert!(
                (q.x + (q.z - T) * rake).abs() < 2e-5,
                "{q:?} off the transom plane"
            );
        }
        // The skeg: the keel line is 0.3 m below the midship canoe keel over
        // the aft third (x from 2.6 to 5.8); the midbody has a 0.05 m bar keel.
        for st in &STATIONS {
            if (2.0..6.0).contains(&st[0]) {
                assert_eq!(st[1], 0.0);
                assert!(st[3] - st[1] >= 0.3);
            } else if (8.0..14.0).contains(&st[0]) {
                assert!(st[1] >= 0.25 && st[3] - st[1] <= 0.1 + 1e-9, "{st:?}");
            }
        }
        // Bulwark: off the hydrostatics, wholly above the sheer.
        let names: Vec<&str> = s.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Hull", "Transom", "Deck", "Bulwark"]);
        assert!(s.layers.iter().all(|l| l.symmetric));
        let in_hydro: Vec<bool> = s.layers.iter().map(|l| l.use_in_hydrostatics).collect();
        assert_eq!(in_hydro, [true, true, true, false]);
        let bulwark = 3;
        for (f, &l) in s.faces.iter().zip(&s.face_layer) {
            if l == bulwark {
                assert!(f.iter().all(|&p| s.points[p].p.z >= 3.3 - 1e-9));
            }
        }
        // Turning the bulwark into hydrostatics changes nothing at T.
        let mut wet = s.clone();
        wet.layers[bulwark].use_in_hydrostatics = true;
        let (a, b) = (checks::measure(&s), checks::measure(&wet));
        assert!((a.volume_m3 - b.volume_m3).abs() < 1e-9);
        assert!(s.edges.iter().filter(|e| e.crease).count() >= 3 * STATIONS.len());
    }

    #[test]
    fn trawler_is_closed_stable_and_floats_level() {
        let (s, p) = build();
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "skeg bottom at z = 0");
        checks::assert_closed_below(&s);
        let (gm, _) = checks::assert_stability(&s, KG, DESIGN_LCG);
        assert!(gm > 0.5 && gm < 1.0, "GM {gm}");
        checks::assert_free_trim(&s, &p);
    }

    /// IMO A.749 (2008 IS Code Part A 2.2) at the Design condition, KG as
    /// assumed: run and reported (`--nocapture`), never tuned to pass.
    #[test]
    fn trawler_design_condition_imo_a749() {
        let (s, p) = build();
        let cond = p.loading_condition(Some("Design")).unwrap();
        let opts = HydroOptions::from_surface(&s, checks::level3());
        let res = crate::loading::evaluate(&s, &opts, cond);
        let imo = res.imo_criteria.expect("IMO criteria evaluated");
        println!(
            "trawler IMO A.749: area 0-30 {:.4} (0.055) 0-40 {:.4} (0.090) 30-40 {:.4} (0.030) \
             GZmax>=30 {:.3} (0.20) angle {:.1} (25) GM0 {:.3} (0.15) -> {}",
            imo.area_0_30_m_rad,
            imo.area_0_40_m_rad,
            imo.area_30_40_m_rad,
            imo.max_gz_30_plus_m,
            imo.angle_of_max_gz_deg,
            imo.gm0_m,
            if imo.passed { "PASS" } else { "FAIL" }
        );
        assert!((imo.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-2);
        assert!((res.kg_fluid - KG).abs() < 5e-3, "KG {}", res.kg_fluid);
    }

    #[test]
    fn trawler_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
    }
}
