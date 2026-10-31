//! 6.5 m hard-chine outboard runabout: a V-bottom planing boat with a flat
//! transom raked 12° for an outboard bracket, a raked stem and a flat deck.
//!
//! Targets (examples board, Opus table; "representative of production
//! boats of this size", not any one model): LWL 5.6 m, BWL 2.2 m (BOA
//! 2.5 m), T 0.38 m, D 1.25 m, Δ 1.9 t, Cb 0.39, Cp 0.68, Cm 0.58, Cwp 0.80,
//! LCB 42 % LWL from aft, 30 kn, KG 0.65 m (assumed, about half the depth).
//!
//! Cm 0.58 at BWL 2.2 m and T 0.38 m leaves room for about 17° of deadrise
//! amidships with the chine below the waterline: a 24° bottom there puts the
//! chine above the waterline and Cm near 0.5. The net follows the targets,
//! so the bottom is a moderate V aft (≈17° at the transom and amidships)
//! warping to a fine entry forward.
//!
//! Layers: Bottom (keel to chine), Topsides (chine to sheer), Transom, Deck.
//! Creases: the chines, the sheer, the transom outline; the keel and the
//! stem are the half model's centreline boundary, which the kernel pins.

use super::{
    close_half_deck, close_half_transom, crease_loop, crease_path, design_condition, item, layer,
    plating, round_to_fef, settings, station_grid, Built, Example, GRP,
};
use crate::loading::LoadingCondition;
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};
use crate::weather::BilgeKind;

const LWL: f64 = 5.6;
const BWL: f64 = 2.2;
const T: f64 = 0.38;
const DEPTH: f64 = 1.25;
/// The example's KG above the keel (m), assumed.
const KG: f64 = 0.65;
const SPEED_KN: f64 = 30.0;
/// Transom rake from the vertical (top forward), degrees.
const TRANSOM_RAKE_DEG: f64 = 12.0;
/// Δ of the refined hull at T (t), measured at authoring time and frozen.
const DESIGN_DISPLACEMENT_T: f64 = 1.922;
/// LCB of the refined hull at T (model x, m), measured and frozen: the
/// Design condition's LCG.
const DESIGN_LCG: f64 = 2.337;

/// Stations aft to forward: `(x, keel z, chine y, chine z, sheer y, sheer z)`.
/// Station 0 is the transom (its `x` is where the transom plane meets the
/// waterline); the stem is [`STEM`].
const STATIONS: [(f64, f64, f64, f64, f64, f64); 10] = [
    (0.00, 0.140, 0.94, 0.355, 1.15, 1.15),
    (0.60, 0.090, 1.00, 0.330, 1.20, 1.16),
    (1.20, 0.040, 1.05, 0.310, 1.23, 1.17),
    (1.90, 0.008, 1.08, 0.297, 1.25, 1.18),
    (2.60, 0.000, 1.09, 0.295, 1.25, 1.20),
    (3.30, 0.008, 1.06, 0.320, 1.24, 1.23),
    (3.95, 0.035, 0.97, 0.380, 1.18, 1.27),
    (4.50, 0.085, 0.80, 0.450, 1.06, 1.31),
    (5.00, 0.165, 0.57, 0.540, 0.86, 1.35),
    (5.40, 0.270, 0.30, 0.620, 0.56, 1.39),
];

/// The stem profile on the centreline, forefoot to stem head: `(x, z)` for
/// the keel, bottom, chine, topside and sheer rows.
const STEM: [(f64, f64); 5] = [
    (5.585, 0.36),
    (5.665, 0.46),
    (5.78, 0.62),
    (6.05, 1.00),
    (6.42, 1.42),
];

/// Bottom and topside rows between the creases.
fn section(zk: f64, yc: f64, zc: f64, ys: f64, zs: f64) -> [(f64, f64); 5] {
    [
        (0.0, zk),
        (0.5 * yc, 0.5 * (zk + zc)),
        (yc, zc),
        (yc + 0.6 * (ys - yc), zc + 0.5 * (zs - zc)),
        (ys, zs),
    ]
}

pub(super) fn build() -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Bottom", 0xd0_d0_d0, true, plating(GRP, 10.0)),
        layer(2, "Topsides", 0xff_ff_ff, true, plating(GRP, 8.0)),
        layer(3, "Transom", 0x80_90_a0, false, plating(GRP, 10.0)),
        layer(4, "Deck", 0x6a_8c_af, false, plating(GRP, 6.0)),
    ];
    s.settings = settings(Example::Runabout6m, LWL, BWL, T);

    let rake = TRANSOM_RAKE_DEG.to_radians().tan();
    let mut stations: Vec<Vec<Vec3>> = STATIONS
        .iter()
        .enumerate()
        .map(|(i, &(x, zk, yc, zc, ys, zs))| {
            section(zk, yc, zc, ys, zs)
                .iter()
                .map(|&(y, z)| {
                    // The transom is one plane through (x, z = T).
                    let x = if i == 0 { x + (z - T) * rake } else { x };
                    Vec3::new(x, y, z)
                })
                .collect()
        })
        .collect();
    stations.push(STEM.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect());

    // Bottom below the chine (rows 0..2), topsides above.
    let grid = station_grid(&mut s, &stations, |_, j| if j < 2 { 0 } else { 1 });
    let sheer: Vec<usize> = grid.iter().map(|st| st[4]).collect();
    let chine: Vec<usize> = grid.iter().map(|st| st[2]).collect();
    let centre = close_half_deck(&mut s, 3, &sheer);
    let transom = close_half_transom(&mut s, 2, &grid[0], centre[0]);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &chine);
    crease_path(&mut s, &sheer);
    crease_loop(&mut s, &transom);

    let project = Project {
        kg: Some(KG),
        requirements: Some(Requirements {
            name: "6.5 m outboard runabout".into(),
            lwl: Some(LWL),
            bwl: Some(BWL),
            draft: Some(T),
            depth: Some(DEPTH),
            displacement_t: Some(1.9),
            cb: Some(0.39),
            lcb_frac: Some(0.42),
            design_speed_kn: Some(SPEED_KN),
            kg: None,
        }),
        loading_conditions: vec![LoadingCondition {
            // Hard chines: the weather criterion's sharp-bilge k = 0.7.
            bilge: BilgeKind::Sharp,
            ..design_condition(
                DESIGN_DISPLACEMENT_T,
                DESIGN_LCG,
                KG,
                "Lightship (hull, console, 150 hp outboard)",
                vec![
                    item("Crew and passengers (4)", 0.30, 2.40, 0.95),
                    item("Fuel, 150 l", 0.11, 1.20, 0.35),
                    item("Gear and stores", 0.08, 3.50, 0.70),
                ],
            )
        }],
        report: ReportSettings {
            kg: Some(KG),
            lcg_x: Some(DESIGN_LCG),
            speeds_kn: Some(vec![15.0, 20.0, 25.0, 30.0, 35.0]),
            include_planing: Some(true),
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
    fn runabout_meets_the_board_targets() {
        let (s, _) = build();
        let h = checks::measure(&s);
        checks::assert_targets(
            "runabout",
            &h,
            &checks::Targets {
                lwl: LWL,
                bwl: BWL,
                displacement_t: 1.9,
                cb: 0.39,
                cp: 0.68,
                cm: 0.58,
                cwp: 0.80,
                lcb_frac: 0.42,
            },
        );
        // The frozen Design displacement and LCG are the measured ones.
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-3);
        assert!((h.lcb.x - DESIGN_LCG).abs() < 1e-3);
        // Particulars the table does not test: BOA 2.5 m, LOA about 6.5 m,
        // depth 1.25 m amidships, the transom raked 12°.
        let (min, max) = s.extents();
        assert!((2.0 * max.y - 2.5).abs() < 1e-9, "BOA {}", 2.0 * max.y);
        assert!((max.x - min.x - 6.5).abs() < 0.05, "LOA {}", max.x - min.x);
    }

    #[test]
    fn runabout_has_hard_chines_and_a_flat_raked_transom() {
        let (s, _) = build();
        // Every transom point lies on one plane raked 12° (top forward).
        let rake = TRANSOM_RAKE_DEG.to_radians().tan();
        let transom = s.layers.iter().position(|l| l.name == "Transom").unwrap();
        let fi = s.face_layer.iter().position(|&l| l == transom).unwrap();
        for &p in &s.faces[fi] {
            let q = s.points[p].p;
            assert!(
                (q.x - (q.z - T) * rake).abs() < 2e-5,
                "{q:?} off the transom plane"
            );
        }
        // The chine is creased from the transom to the stem.
        let creased = s.edges.iter().filter(|e| e.crease).count();
        assert!(creased >= 2 * STATIONS.len(), "{creased} crease edges");
        let names: Vec<&str> = s.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Bottom", "Topsides", "Transom", "Deck"]);
        // A hard-chine hull rolls with the IS Code's sharp-bilge factor
        // k = 0.7, not the round bilge's 1.0 (R8-2 A10).
        let (_, p) = build();
        let design = p.loading_condition(Some("Design")).unwrap();
        assert_eq!(design.bilge, BilgeKind::Sharp);
        assert!(s
            .layers
            .iter()
            .all(|l| l.symmetric && l.use_in_hydrostatics));
    }

    #[test]
    fn runabout_is_closed_stable_and_floats_level() {
        let (s, p) = build();
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "keel at z = 0");
        checks::assert_closed_below(&s);
        let (gm, _) = checks::assert_stability(&s, KG, DESIGN_LCG);
        assert!(gm > 1.0, "a beamy planing boat: GM {gm}");
        checks::assert_free_trim(&s, &p);
    }

    #[test]
    fn runabout_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
    }
}
