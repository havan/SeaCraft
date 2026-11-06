//! 9 m Aegean piyade kayık: a narrow round-bilge body on a deep keel and
//! deadwood, a pointed raked stem, strong sheer and a small transom above
//! the waterline with the sternpost below it, for an inboard diesel.
//!
//! Source of the targets: B. İ. Turan, "Tırhandil ve Piyade Tipi Tekne
//! Formlarının Tasarım ve Mühendislik Perspektifinden Karşılaştırılması",
//! *Journal of Marine and Engineering Technology (JoINMET)* 2(2), 78–90,
//! 2022. The dimensions and coefficients are the means of the 12 surveyed
//! piyade boats, taken from that paper's tables; the lines are SeaCraft's own. LOA 9.00, LWL 7.97 (0.886 LOA), B 2.96 (0.329 LOA),
//! BWL 2.85 (0.962 B), T 0.907 (BWL/T 3.142) measured to the bottom of the
//! keel as in Turan, freeboard 0.71 (0.784 T), D ≈ 1.62, ∇ 4.49 m³,
//! Δ ≈ 4.60 t, Cb 0.218, Cm 0.490, Cp 0.445, Cwp 0.629, LCB 47.4 % LWL from
//! aft, service speed 6–8 kn.
//!
//! The low Cb and Cm come from the draft being taken to the keel bottom:
//! amidships the lowest 0.36 m (two fifths of T) is keel, 0.14 m thick,
//! and the deadwood rises to 0.8 m at both ends. The
//! keel is therefore part of the hydrostatic hull, and it is in the same
//! control net as the canoe body (rows 0–3 of every station, on the Keel
//! layer) rather than a separate closed half-shell. On a real piyade the
//! deadwood is faired into the garboards, which the net's unpinned root row
//! reproduces as a fillet; a separate shell would need its root cap to sit
//! flush on the refined canoe bottom, and any gap or overlap there would be
//! an error in two fifths of the draft. The price is that the Keel layer
//! cannot be taken out of the hydrostatics without opening the hull (as
//! for the runabout's Bottom layer).
//!
//! Stations run aft to forward. Station 0 is the sternpost (rows 0–6 on the
//! centreline, from the heel up to the transom's bottom just above the
//! waterline) and the transom outline (rows 6–8 on one plane raked aft);
//! the last station is the stem profile on the centreline. Both are the
//! half model's centreline boundary, which the kernel pins, so the keel
//! profile, the sternpost and the stem are the control polylines.
//!
//! Not met: Turan's mean half-angle of entrance is 49.25° (47–52°); this
//! hull's design waterline leaves the stem at ≈ 27°. A fuller bow
//! waterline moves the LCF and, through the bow sections, the LCB forward
//! of 47.4 % and raises Cwp past 0.629 (one trial with up to 0.2 m more
//! half-breadth at the forward stations, and the sheer widened to match,
//! gave LCB 48.0 %, Cwp 0.641), and the
//! forward sections are already at the hollowest flare the net carries.
//! The binding targets (the coefficients) won over the angle.
//!
//! Layers: Hull (canoe body), Keel (keel and deadwood), Transom, Deck (a
//! flat top at the sheer: a real piyade is open forward of a half deck
//! aft, so GZ beyond deck-edge immersion assumes the boat is covered).
//! Creases: the keel's bottom corners, the sheer and the transom outline.
//!
//! KG: 0.95 m above the keel bottom (≈ 0.59 D), an estimate. The design's
//! starting point is 0.6 D; it is rounded down because the lowest 0.36 m
//! of D is the keel, solid timber with the engine bearers and the diesel
//! on top of it: 0.95 m is ≈ 0.47 of the canoe-body depth above the
//! rabbet (0.59 m of 1.26 m), against the ≈ 0.55 usual for an open wooden
//! boat measured from its own bottom. GM is then ≈ 1.17 m: GM is not
//! the binding check on this beamy hull, so KG was not tuned.

use super::{
    close_half_deck, close_half_transom, crease_loop, crease_path, design_condition, item, layer,
    pin_crease_junctions, plating, round_to_fef, settings, station_grid, Built, Example, Plating,
    OAK, TIMBER,
};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};

#[cfg(test)]
const LOA: f64 = 9.00;
const LWL: f64 = 7.97;
#[cfg(test)]
const B: f64 = 2.96;
const BWL: f64 = 2.85;
const T: f64 = 0.907;
const DEPTH: f64 = 1.62;
/// The example's KG above the keel bottom (m), estimated (module header).
const KG: f64 = 0.95;
const SPEED_KN: f64 = 7.0;
/// Turan's means (the targets).
const TARGET_DISPLACEMENT_T: f64 = 4.60;
const TARGET_CB: f64 = 0.218;
#[cfg(test)]
const TARGET_CP: f64 = 0.445;
#[cfg(test)]
const TARGET_CM: f64 = 0.490;
#[cfg(test)]
const TARGET_CWP: f64 = 0.629;
const TARGET_LCB_FRAC: f64 = 0.474;
/// Transom rake from the vertical (top aft), degrees.
const TRANSOM_RAKE_DEG: f64 = 27.0;
/// Δ of the refined hull at T (t), measured at authoring time and frozen.
const DESIGN_DISPLACEMENT_T: f64 = 4.604;
/// LCB of the refined hull at T (model x, m), measured and frozen: the
/// Design condition's LCG.
const DESIGN_LCG: f64 = 3.800;
/// The Keel layer wraps a solid oak keel and deadwood about 0.14 m thick,
/// not a planked shell. Its weight estimate uses the equivalent plate:
/// a slab of thickness `w` has twice its side area as skin, so half the
/// keel's thickness, 70 mm, over the skin gives the timber's mass.
const KEEL_PLATING: Plating = plating(OAK, 70.0);

/// Stations between the sternpost and the stem, aft to forward:
/// `(x, keel z, keel half-width, root z, waterline y, bilge exponent,
/// sheer y, sheer z)`. The canoe body runs from the keel's root
/// `(keel half-width, root z)` to the waterline `(waterline y, T)` on a
/// superellipse quadrant of the given exponent (2: an ellipse, 1: a
/// straight V, below 1 the hollow garboard of a wineglass section). The
/// waterline breadths and exponents were derived once, at authoring time,
/// from a sectional-area curve and a waterline shape fitted to the targets,
/// then frozen.
const STATIONS: [Station; 11] = [
    (0.45, 0.00, 0.060, 0.80, 0.353, 0.60, 0.75, 1.78),
    (1.15, 0.00, 0.070, 0.72, 0.771, 0.60, 1.12, 1.70),
    (1.90, 0.00, 0.070, 0.57, 1.098, 1.19, 1.36, 1.66),
    (2.65, 0.00, 0.070, 0.42, 1.318, 1.56, 1.46, 1.63),
    (3.35, 0.00, 0.070, 0.37, 1.436, 2.09, 1.48, 1.62),
    (3.99, 0.00, 0.070, 0.36, 1.470, 2.39, 1.48, 1.62),
    (4.65, 0.00, 0.070, 0.37, 1.426, 1.59, 1.47, 1.63),
    (5.35, 0.01, 0.070, 0.42, 1.284, 0.82, 1.38, 1.68),
    (6.05, 0.06, 0.070, 0.55, 1.047, 0.60, 1.20, 1.75),
    (6.70, 0.16, 0.065, 0.70, 0.748, 0.60, 0.94, 1.84),
    (7.25, 0.31, 0.060, 0.80, 0.443, 0.60, 0.60, 1.94),
];

/// The sternpost on the centreline, heel to the transom's bottom: `(x, z)`
/// for rows 0–6.
const STERNPOST: [(f64, f64); 7] = [
    (0.30, 0.00),
    (0.27, 0.12),
    (0.23, 0.30),
    (0.18, 0.48),
    (0.12, 0.66),
    (0.06, 0.82),
    (-0.02, 1.00),
];

/// The transom's outline above its bottom (rows 7 and 8): `(y, z)`; `x`
/// follows the transom plane.
const TRANSOM: [(f64, f64); 2] = [(0.40, 1.42), (0.50, 1.85)];

/// The stem profile on the centreline, forefoot to stem head: `(x, z)`
/// for rows 0–8.
const STEM: [(f64, f64); 9] = [
    (7.52, 0.42),
    (7.60, 0.50),
    (7.68, 0.58),
    (7.76, 0.66),
    (7.84, 0.74),
    (7.91, 0.82),
    (7.99, 0.907),
    (8.27, 1.45),
    (8.55, 2.05),
];

/// The nine rows of a station, keel to sheer: keel bottom on the
/// centreline, the keel's bottom corner, its side, the root, two bilge
/// rows, the waterline, the topside and the sheer.
#[allow(clippy::too_many_arguments)]
fn section(zk: f64, kw: f64, zr: f64, ywl: f64, m: f64, ys: f64, zs: f64) -> [(f64, f64); 9] {
    let quad = |deg: f64| -> (f64, f64) {
        let (s, c) = deg.to_radians().sin_cos();
        let e = 2.0 / m;
        (kw + (ywl - kw) * s.powf(e), T - (T - zr) * c.powf(e))
    };
    [
        (0.0, zk),
        (kw, zk),
        (kw, 0.5 * (zk + zr)),
        (kw, zr),
        quad(30.0),
        quad(60.0),
        (ywl, T),
        (ywl + 0.7 * (ys - ywl), 0.5 * (T + zs)),
        (ys, zs),
    ]
}

type Station = (f64, f64, f64, f64, f64, f64, f64, f64);

pub(super) fn build() -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Hull", 0xe8_e0_d0, false, plating(TIMBER, 25.0)),
        layer(2, "Keel", 0x8a_6a_4a, false, KEEL_PLATING),
        layer(3, "Transom", 0x80_90_a0, false, plating(TIMBER, 30.0)),
        layer(4, "Deck", 0xc8_b4_8c, false, plating(TIMBER, 20.0)),
    ];
    s.settings = settings(Example::Kayik9m, LWL, BWL, T);

    let rake = TRANSOM_RAKE_DEG.to_radians().tan();
    let (xb, zb) = STERNPOST[6];
    let mut stern: Vec<Vec3> = STERNPOST
        .iter()
        .map(|&(x, z)| Vec3::new(x, 0.0, z))
        .collect();
    stern.extend(
        TRANSOM
            .iter()
            .map(|&(y, z)| Vec3::new(xb - (z - zb) * rake, y, z)),
    );
    let mut stations: Vec<Vec<Vec3>> = vec![stern];
    stations.extend(STATIONS.iter().map(|&(x, zk, kw, zr, ywl, m, ys, zs)| {
        section(zk, kw, zr, ywl, m, ys, zs)
            .iter()
            .map(|&(y, z)| Vec3::new(x, y, z))
            .collect()
    }));
    stations.push(STEM.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect());

    // Keel and deadwood below the root (rows 0..3), canoe body above.
    let grid = station_grid(&mut s, &stations, |_, j| if j < 3 { 1 } else { 0 });
    let sheer: Vec<usize> = grid.iter().map(|st| st[8]).collect();
    let keel_corner: Vec<usize> = grid.iter().map(|st| st[1]).collect();
    let centre = close_half_deck(&mut s, 3, &sheer);
    let transom = close_half_transom(&mut s, 2, &grid[0][6..], centre[0]);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &keel_corner);
    crease_path(&mut s, &sheer);
    crease_loop(&mut s, &transom);
    pin_crease_junctions(&mut s);

    let project = Project {
        kg: Some(KG),
        requirements: Some(Requirements {
            name: "9 m piyade kayık (Turan 2022 means)".into(),
            lwl: Some(LWL),
            bwl: Some(BWL),
            draft: Some(T),
            depth: Some(DEPTH),
            displacement_t: Some(TARGET_DISPLACEMENT_T),
            cb: Some(TARGET_CB),
            lcb_frac: Some(TARGET_LCB_FRAC),
            design_speed_kn: Some(SPEED_KN),
            kg: None,
        }),
        loading_conditions: vec![design_condition(
            DESIGN_DISPLACEMENT_T,
            DESIGN_LCG,
            KG,
            "Lightship (hull, keel, half deck, inboard diesel)",
            vec![
                item("Crew (2)", 0.16, 1.20, 1.70),
                item("Diesel, 100 l", 0.085, 1.40, 0.70),
                item("Fresh water and stores", 0.06, 2.50, 1.00),
                item("Fishing gear and catch", 0.45, 4.00, 1.00),
            ],
        )],
        report: ReportSettings {
            kg: Some(KG),
            lcg_x: Some(DESIGN_LCG),
            speeds_kn: Some(vec![4.0, 5.0, 6.0, 7.0, 8.0]),
            include_resistance: Some(true),
            include_planing: Some(false),
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
    use crate::hydro::{plane_segments, waterplane, Hull, HydroOptions};
    use crate::stability::{cross_curves, solve_floating, StabilityCondition};

    /// The refined transom keeps its sheer corner and the topsides do not
    /// pinch in towards it.
    #[test]
    fn kayik_keeps_its_transom_outline() {
        let (s, _) = build();
        checks::assert_transom_outline(&s, STATIONS[0].0);
    }
    use crate::subdiv::evaluate_full;

    fn layer_index(s: &Surface, name: &str) -> usize {
        s.layers.iter().position(|l| l.name == name).unwrap()
    }

    /// Points of the faces on layer `name`.
    fn layer_points(s: &Surface, name: &str) -> Vec<Vec3> {
        let li = layer_index(s, name);
        s.faces
            .iter()
            .zip(&s.face_layer)
            .filter(|(_, &l)| l == li)
            .flat_map(|(f, _)| f.iter().map(|&p| s.points[p].p))
            .collect()
    }

    /// The design waterline's half-angle of entrance (degrees): the chord
    /// from the stem to the refined waterline 2.5 % of LWL aft of it.
    fn entrance_half_angle(s: &Surface, h: &crate::hydro::Hydrostatics) -> f64 {
        let full = evaluate_full(s, checks::level3());
        let wp = waterplane(s, T, 0.0, 0.0);
        let segs = plane_segments(&full.verts, &full.triangles(), &wp);
        let x_fwd = h.wl_aft_x + h.lwl;
        let d = 0.025 * h.lwl;
        // Half-breadth at x_fwd - d: interpolate along the crossing segments.
        let x = x_fwd - d;
        let y = segs
            .iter()
            .filter(|(a, b)| (a.x - x) * (b.x - x) <= 0.0 && a.x != b.x)
            .map(|(a, b)| (a.y + (b.y - a.y) * (x - a.x) / (b.x - a.x)).abs())
            .fold(0.0, f64::max);
        (y / d).atan().to_degrees()
    }

    #[test]
    fn kayik_meets_the_turan_means() {
        let (s, _) = build();
        let h = checks::measure(&s);
        checks::assert_targets(
            "kayik",
            &h,
            &checks::Targets {
                lwl: LWL,
                bwl: BWL,
                displacement_t: TARGET_DISPLACEMENT_T,
                cb: TARGET_CB,
                cp: TARGET_CP,
                cm: TARGET_CM,
                cwp: TARGET_CWP,
                lcb_frac: TARGET_LCB_FRAC,
            },
        );
        // The frozen Design displacement and LCG are the measured ones.
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-3);
        assert!((h.lcb.x - DESIGN_LCG).abs() < 1e-3);
        // Particulars the table does not test: LOA 9.00 m, B 2.96 m, depth
        // 1.62 m to the sheer amidships (freeboard 0.71 m).
        let (min, max) = s.extents();
        assert!((max.x - min.x - LOA).abs() < 0.01, "LOA {}", max.x - min.x);
        assert!((2.0 * max.y - B).abs() < 1e-9, "B {}", 2.0 * max.y);
        let mid = STATIONS
            .iter()
            .find(|st| (st.0 - (h.wl_aft_x + 0.5 * h.lwl)).abs() < 0.05)
            .expect("a station amidships");
        assert!((mid.7 - DEPTH).abs() < 1e-9, "D {}", mid.7);
        println!(
            "kayik: LOA {:.3} B {:.3} D {DEPTH} KB {:.4} KM {:.4} entrance half-angle {:.1}°",
            max.x - min.x,
            2.0 * max.y,
            h.kb,
            h.km_transverse,
            entrance_half_angle(&s, &h)
        );
    }

    /// The draft is to the bottom of the keel, as in Turan: the keel and
    /// deadwood are hydrostatic and a third of T deep; the stem is pointed
    /// and raked; the transom is small, flat and clear of the water.
    #[test]
    fn kayik_has_a_deep_keel_a_raked_stem_and_a_small_transom() {
        let (s, _) = build();
        let names: Vec<&str> = s.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Hull", "Keel", "Transom", "Deck"]);
        assert!(s
            .layers
            .iter()
            .all(|l| l.symmetric && l.use_in_hydrostatics));
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "keel bottom at z = 0");

        // Keel: from z = 0 up to the canoe body, at most 0.14 m thick.
        let keel = layer_points(&s, "Keel");
        assert!(keel.iter().all(|p| p.y <= 0.07 + 1e-9), "keel too wide");
        let canoe_bottom = layer_points(&s, "Hull")
            .iter()
            .map(|p| p.z)
            .fold(f64::INFINITY, f64::min);
        assert!(
            canoe_bottom > 0.35 * T,
            "the canoe body reaches {canoe_bottom} m: the keel is not deep"
        );

        // Transom: one plane raked aft, its bottom above the waterline, and
        // narrower than half the beam.
        let rake = TRANSOM_RAKE_DEG.to_radians().tan();
        let (xb, zb) = STERNPOST[6];
        let transom = layer_points(&s, "Transom");
        for q in &transom {
            assert!(
                (q.x - (xb - (q.z - zb) * rake)).abs() < 2e-5,
                "{q:?} off the transom plane"
            );
        }
        assert!(transom.iter().all(|q| q.z > T), "the transom is immersed");
        let half_width = transom.iter().map(|q| q.y).fold(0.0, f64::max);
        assert!(
            2.0 * half_width < 0.5 * B,
            "transom {} m wide",
            2.0 * half_width
        );

        // Stem: a centreline polyline raked forward all the way up.
        for w in STEM.windows(2) {
            assert!(w[1].0 > w[0].0 && w[1].1 > w[0].1, "stem not raked");
        }
        let h = checks::measure(&s);
        // Pointed, not bluff: Turan's mean is 49.25°, this hull ≈ 27° (the
        // gap is explained in the module header).
        let angle = entrance_half_angle(&s, &h);
        assert!(
            (20.0..60.0).contains(&angle),
            "entrance half-angle {angle}°"
        );
    }

    #[test]
    fn kayik_is_closed_stable_and_floats_level() {
        let (s, p) = build();
        checks::assert_closed_below(&s);
        let (gm, _) = checks::assert_stability(&s, KG, DESIGN_LCG);
        assert!(gm > 0.5, "a beamy workboat: GM {gm}");
        checks::assert_free_trim(&s, &p);
    }

    /// Free to trim at the design displacement and LCG, the righting lever
    /// is positive from the upright to deck-edge immersion (the angle at
    /// which the sheer first touches the water).
    #[test]
    fn kayik_rights_itself_up_to_deck_edge_immersion() {
        let (s, _) = build();
        let hull = Hull::new(&s, checks::level3());
        let volume = DESIGN_DISPLACEMENT_T * 1000.0 / s.settings.water_density;
        let sheer: Vec<Vec3> = s
            .points
            .iter()
            .map(|p| p.p)
            .filter(|p| p.y > 0.0 && layer_points(&s, "Deck").contains(p))
            .collect();
        let immersed = |heel: f64| -> bool {
            let fc = solve_floating(&hull, volume, heel, DESIGN_LCG, 0.0).expect("floats");
            let wp = waterplane(&s, fc.draft, fc.trim, heel);
            sheer.iter().any(|&q| wp.side(q) <= 0.0)
        };
        assert!(!immersed(0.0));
        let (mut lo, mut hi) = (0.0, 60.0);
        assert!(immersed(hi));
        while hi - lo > 0.1 {
            let mid = 0.5 * (lo + hi);
            if immersed(mid) {
                hi = mid;
            } else {
                lo = mid;
            }
        }
        let deck_edge = lo;
        let mut heels: Vec<f64> = (1..)
            .map(|k| 5.0 * k as f64)
            .take_while(|&a| a < deck_edge)
            .collect();
        heels.push(deck_edge);
        let base = HydroOptions::from_surface(&s, checks::level3());
        let curves = cross_curves(
            &hull,
            &heels,
            &[DESIGN_DISPLACEMENT_T],
            &StabilityCondition::free_to_trim(KG, DESIGN_LCG),
            &base,
        );
        for c in &curves {
            let gz = c.points[0].gz.expect("GZ");
            println!("kayik: heel {:5.2}° GZ {gz:.4} m", c.heel_deg);
            assert!(gz > 0.0, "GZ {gz} at {}°", c.heel_deg);
        }
        println!("kayik: deck-edge immersion at {deck_edge:.2}°");
        assert!(deck_edge > 15.0, "deck edge immersed at {deck_edge}°");
    }

    #[test]
    fn kayik_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
        let r = p.requirements.as_ref().unwrap();
        assert_eq!(r.lwl, Some(LWL));
        assert_eq!(r.draft, Some(T));
        assert!(Example::Kayik9m.source().contains("Turan (2022)"));
    }
}
