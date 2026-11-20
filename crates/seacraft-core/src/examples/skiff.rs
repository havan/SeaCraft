//! 4.3 m flat-bottom plywood skiff for oars or a small outboard, built stitch-and-glue: a bottom
//! flat across and bent up forward, flared sides that curve in plan to a straight raked stem, and
//! a flat transom raked 12°. Every panel is cut from flat plywood, so every panel must lie flat.
//!
//! Targets (design targets chosen for this example, lines our own): LOA 4.3 m, beam 1.44 m at
//! the sheer and 1.0 m across the bottom, two people with oars or a 6 hp outboard, Δ 0.30 t,
//! 12 kn under power, KG 0.42 m estimated (people seated on thwarts). The coefficients in the
//! tests were measured on this net at authoring time and frozen.
//!
//! How the panels are made to lie flat:
//!
//! - The bottom is one control face wide between the centreline and the chine, and flat across:
//!   both rows of a station are at one height, so the bottom is a cylinder bent along its length.
//! - The sides are one control face wide between the chine and the open sheer. Each rung of the
//!   net, chine point to sheer point, is a ruling of a developable surface: the sheer point is
//!   where the sheer's tangent lies in the plane of the chine's tangent and the rung
//!   ([`sheer_partner`]), so the side's tangent plane does not turn along the rung. Where no
//!   such point exists (at the transom and at the stem) the rungs fan from one corner, a cone.
//! - The open sheer and the centreline are boundaries the kernel keeps as their control polygons;
//!   the chine is a crease, a cubic B-spline, so its control half-breadths are placed for the curve
//!   to pass through the design chine at every station ([`interpolating`]). Its heights are left as
//!   designed, equal to the centreline's, so the bottom stays flat across; where the rocker is
//!   straight the two agree exactly; elsewhere every refined face on the centreline is within half
//!   a degree of its mirror image (measured at authoring, level 3; the panels join at 2°).
//!
//! Layers: Bottom, Side, Transom, all developable. Creases: the chine and the transom outline.
//! Open boat: no deck.

use super::{
    add_oriented, close_half_transom, crease_loop, crease_path, design_condition, item, layer,
    pin_crease_junctions, plating, round_to_fef, settings, Built, Example, PLYWOOD,
};
use crate::loading::LoadingCondition;
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};
use crate::weather::BilgeKind;

/// Waterline length and breadth at T (m), measured at authoring time and frozen.
const LWL: f64 = 3.178;
const BWL: f64 = 1.119;
/// The design draft above the keel (m): chosen at authoring time for Δ 0.30 t.
const T: f64 = 0.118;
/// Depth amidships, keel to sheer (m).
const DEPTH: f64 = 0.47;
/// Transom rake from the vertical (top forward), degrees.
const TRANSOM_RAKE_DEG: f64 = 12.0;
/// The example's KG above the keel (m), estimated.
const KG: f64 = 0.42;
const SPEED_KN: f64 = 12.0;
/// Δ of the refined hull at T (t), measured at authoring time and frozen.
const DESIGN_DISPLACEMENT_T: f64 = 0.300;
/// LCB of the refined hull at T (model x, m), measured and frozen: the Design condition's LCG.
const DESIGN_LCG: f64 = 1.291;

/// Where the chine reaches the centreline: the forefoot, the foot of the stem (x, z).
const FOOT: (f64, f64) = (4.0, 0.22);
/// The stem head on the centreline (x, z).
const HEAD: (f64, f64) = (4.30, 0.72);
/// Chine stations from the transom (x = 0) to the forefoot, evenly spaced.
const CHINE_STATIONS: usize = 21;
/// Extra sheer points on a fan (where the rungs spread from one chine corner).
const FAN_POINTS: usize = 3;

fn rake() -> f64 {
    TRANSOM_RAKE_DEG.to_radians().tan()
}

/// The bottom's rocker: flat to `ROCKER.0`, a parabola to `ROCKER.1`, then straight on its
/// tangent to the forefoot. The straight run makes the narrow forefoot flat across exactly: where
/// heights are linear in x, the kernel's averaging keeps them so.
const ROCKER: (f64, f64) = (1.2, 3.4);

/// The parabola's coefficient that brings the rocker to the forefoot height.
fn rocker_a() -> f64 {
    let (a, b) = ROCKER;
    FOOT.1 / ((b - a).powi(2) + 2.0 * (b - a) * (FOOT.0 - b))
}

/// The design chine, transom to forefoot: widest (0.50 m) at x 1.5 m, flat on the keel line aft
/// and rising forward to the forefoot ([`ROCKER`]).
fn chine(x: f64) -> Vec3 {
    let y = if x <= 1.5 {
        0.50 - 0.08 * ((1.5 - x) / 1.5).powi(2)
    } else {
        0.50 * (1.0 - ((x - 1.5) / (FOOT.0 - 1.5)).powi(2))
    };
    let z = if x <= ROCKER.0 {
        0.0
    } else if x <= ROCKER.1 {
        rocker_a() * (x - ROCKER.0).powi(2)
    } else {
        rocker_a() * (ROCKER.1 - ROCKER.0) * (2.0 * x - ROCKER.0 - ROCKER.1)
    };
    Vec3::new(x, y.max(0.0), z)
}

/// The design sheer, transom to stem head: rising steadily forward, widest (0.72 m) at x 1.6 m.
fn sheer(x: f64) -> Vec3 {
    let y = if x <= 1.6 {
        0.72 - 0.10 * ((1.6 - x) / 1.6).powi(2)
    } else {
        0.72 * (1.0 - ((x - 1.6) / (HEAD.0 - 1.6)).powi(2))
    };
    let z = 0.39 + (HEAD.1 - 0.39) * (x / HEAD.0).powi(2);
    Vec3::new(x, y.max(0.0), z)
}

/// Where the sheer meets the transom plane `x = z tan(rake)`.
fn sheer_at_transom() -> f64 {
    let mut x = 0.0;
    for _ in 0..50 {
        x = sheer(x).z * rake();
    }
    x
}

fn tangent(f: fn(f64) -> Vec3, x: f64) -> Vec3 {
    let h = 1e-5;
    (f(x + h) - f(x - h)) * (0.5 / h)
}

/// The developability residual of the rung from the chine at `xc` to the sheer at `xs`: zero
/// when the sheer's tangent there lies in the plane of the chine's tangent and the rung.
fn residual(xc: f64, xs: f64) -> f64 {
    let c = chine(xc);
    let rung = sheer(xs) - c;
    tangent(chine, xc).cross(tangent(sheer, xs)).dot(rung)
}

/// The sheer x the rung from the chine at `xc` reaches as a ruling, searched over
/// `[lo, hi]`; `None` when there is none.
fn sheer_partner(xc: f64, lo: f64, hi: f64) -> Option<f64> {
    let n = 400;
    let at = |k: usize| lo + (hi - lo) * k as f64 / n as f64;
    let mut best: Option<f64> = None;
    for k in 0..n {
        let (a, b) = (at(k), at(k + 1));
        let (fa, fb) = (residual(xc, a), residual(xc, b));
        if fa == 0.0 || fa * fb < 0.0 {
            let (mut a, mut b, mut fa) = (a, b, fa);
            for _ in 0..60 {
                let m = 0.5 * (a + b);
                let fm = residual(xc, m);
                if fa * fm <= 0.0 {
                    b = m;
                } else {
                    a = m;
                    fa = fm;
                }
            }
            let root = 0.5 * (a + b);
            if best.is_none_or(|r| (root - xc).abs() < (r - xc).abs()) {
                best = Some(root);
            }
        }
    }
    best
}

/// Control points for a creased row whose refined curve passes through `q` in half-breadth: the
/// cubic B-spline limit of an interior point is `(P[i-1] + 4 P[i] + P[i+1]) / 6`, the ends are
/// pinned corners. `x` and `z` are kept as given (see the module notes).
fn interpolating(q: &[Vec3]) -> Vec<Vec3> {
    let n = q.len();
    let mut p = q.to_vec();
    // Jacobi iterations: diagonally dominant, converges fast.
    for _ in 0..200 {
        let old = p.clone();
        for i in 1..n - 1 {
            let v = (q[i] * 6.0 - old[i - 1] - old[i + 1]) * 0.25;
            p[i] = Vec3::new(q[i].x, v.y, q[i].z);
        }
    }
    p
}

/// One rung of the side: chine index and sheer index into their rows.
type Rung = (usize, usize);

/// The side's rows and rungs: chine x (one per station), sheer x, and the rungs between them,
/// both indices never decreasing, fans where one corner holds still.
fn side_layout() -> (Vec<f64>, Vec<f64>, Vec<Rung>) {
    let xc: Vec<f64> = (0..CHINE_STATIONS)
        .map(|i| FOOT.0 * i as f64 / (CHINE_STATIONS - 1) as f64)
        .collect();
    let xt = sheer_at_transom();
    let partners: Vec<Option<f64>> = xc.iter().map(|&x| sheer_partner(x, xt, HEAD.0)).collect();
    let mut xs: Vec<f64> = Vec::new();
    let mut rungs: Vec<Rung> = Vec::new();
    // Aft fan: the transom edge (chine corner to sheer corner) is the first rung.
    xs.push(xt);
    rungs.push((0, 0));
    let first = partners[0].unwrap_or(xt);
    if first > xt {
        for k in 1..=FAN_POINTS {
            xs.push(xt + (first - xt) * k as f64 / FAN_POINTS as f64);
            rungs.push((0, xs.len() - 1));
        }
    }
    for (i, p) in partners.iter().enumerate().skip(1).take(xc.len() - 2) {
        let last = *xs.last().unwrap();
        match p {
            Some(x) if *x > last + 1e-3 && *x < HEAD.0 - 1e-3 => {
                xs.push(*x);
                rungs.push((i, xs.len() - 1));
            }
            // No partner ahead: the sheer point holds still and the chine fans to it.
            _ => rungs.push((i, xs.len() - 1)),
        }
    }
    // Forward fan: from the forefoot to the stem head.
    let foot = xc.len() - 1;
    let last = *xs.last().unwrap();
    for k in 1..FAN_POINTS {
        xs.push(last + (HEAD.0 - last) * k as f64 / FAN_POINTS as f64);
        rungs.push((foot, xs.len() - 1));
    }
    xs.push(HEAD.0);
    rungs.push((foot, xs.len() - 1));
    (xc, xs, rungs)
}

pub(super) fn build() -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Bottom", 0xc8_a8_78, true, plating(PLYWOOD, 9.0)),
        layer(2, "Side", 0xe0_c8_98, true, plating(PLYWOOD, 6.0)),
        layer(3, "Transom", 0xa0_80_58, true, plating(PLYWOOD, 12.0)),
    ];
    s.settings = settings(Example::Skiff4m, LWL, BWL, T);

    let (xc, xs, rungs) = side_layout();
    let design_chine: Vec<Vec3> = xc
        .iter()
        .enumerate()
        .map(|(i, &x)| {
            if i + 1 == xc.len() {
                Vec3::new(FOOT.0, 0.0, FOOT.1)
            } else {
                chine(x)
            }
        })
        .collect();
    let chine_pts: Vec<usize> = interpolating(&design_chine)
        .into_iter()
        .map(|p| s.add_point(p))
        .collect();
    let sheer_pts: Vec<usize> = xs
        .iter()
        .enumerate()
        .map(|(k, &x)| {
            if k == 0 {
                // On the transom plane.
                let p = sheer(x);
                Vec3::new(p.z * rake(), p.y, p.z)
            } else if k + 1 == xs.len() {
                Vec3::new(HEAD.0, 0.0, HEAD.1)
            } else {
                sheer(x)
            }
        })
        .map(|p| s.add_point(p))
        .collect();
    // The bottom's centreline row, flat across with the chine at each station; the forefoot is
    // shared with the chine.
    let foot = *chine_pts.last().unwrap();
    let mut centre: Vec<usize> = design_chine[..design_chine.len() - 1]
        .iter()
        .map(|c| s.add_point(Vec3::new(c.x, 0.0, c.z)))
        .collect();
    centre.push(foot);

    let down = Vec3::new(0.0, 0.0, -1.0);
    for i in 0..centre.len() - 1 {
        add_oriented(
            &mut s,
            vec![centre[i], chine_pts[i], chine_pts[i + 1], centre[i + 1]],
            0,
            down,
        );
    }
    let out = Vec3::new(0.0, 1.0, 0.0);
    for w in rungs.windows(2) {
        let ((c0, s0), (c1, s1)) = (w[0], w[1]);
        add_oriented(
            &mut s,
            vec![chine_pts[c0], sheer_pts[s0], sheer_pts[s1], chine_pts[c1]],
            1,
            out,
        );
    }
    let apex = s.add_point(Vec3::new(
        s.points[sheer_pts[0]].p.x,
        0.0,
        s.points[sheer_pts[0]].p.z,
    ));
    let transom = close_half_transom(&mut s, 2, &[centre[0], chine_pts[0], sheer_pts[0]], apex);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &chine_pts);
    crease_loop(&mut s, &transom);
    pin_crease_junctions(&mut s);

    let project = Project {
        kg: Some(KG),
        requirements: Some(Requirements {
            name: "4.3 m plywood skiff".into(),
            lwl: None,
            bwl: None,
            draft: Some(T),
            depth: Some(DEPTH),
            displacement_t: Some(DESIGN_DISPLACEMENT_T),
            cb: None,
            lcb_frac: None,
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
                "Lightship (plywood hull, thwarts, oarlocks)",
                vec![
                    item("Helmsman at the tiller, seated", 0.08, 0.50, 0.55),
                    item("Passenger on the middle thwart", 0.08, 1.80, 0.55),
                    item("Outboard, 6 hp", 0.027, -0.15, 0.45),
                    item("Fuel, 12 l", 0.01, 0.40, 0.15),
                    item("Oars, anchor and gear", 0.02, 2.20, 0.15),
                ],
            )
        }],
        report: ReportSettings {
            kg: Some(KG),
            lcg_x: Some(DESIGN_LCG),
            speeds_kn: Some(vec![6.0, 9.0, 12.0, 15.0]),
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
    use crate::panels::{panel_set, PanelOptions};
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    /// Every panel lies flat within the default limits (2 mm seam drift, 0.1 % RMS stretch,
    /// `panels::Limits::default`), at the default level 3 and one level finer: the bottom in one
    /// piece across the centreline, the sides a mirrored pair, the transom one flat piece.
    #[test]
    fn skiff_panels_all_lie_flat() {
        let (s, _) = build();
        for level in [3, 4] {
            let opts = PanelOptions {
                subdivision: SubdivisionOptions {
                    level,
                    mode: SubdivisionMode::Loop,
                },
                ..Default::default()
            };
            let set = panel_set(&s, &opts).expect("panels");
            for r in &set.rows {
                println!(
                    "level {level} {}: copies {} rms {:.4} % max {:.4} % drift {:.2} mm sheets {:?}",
                    r.panel,
                    r.copies,
                    r.rms_stretch_pct,
                    r.max_stretch_pct,
                    r.max_seam_drift_mm,
                    r.sheets_each
                );
            }
            assert!(set.refused.is_empty(), "refused: {:?}", set.refused);
            let got: Vec<(&str, usize)> = set
                .rows
                .iter()
                .map(|r| (r.panel.as_str(), r.copies))
                .collect();
            assert_eq!(got, [("Bottom", 1), ("Side", 2), ("Transom", 1)]);
            for r in &set.rows {
                assert!(
                    r.lies_flat,
                    "level {level}: {} does not lie flat: {r:?}",
                    r.panel
                );
            }
        }
    }

    #[test]
    fn skiff_meets_the_board_targets() {
        let (s, _) = build();
        let h = checks::measure(&s);
        // Δ is the design target; the rest were measured on this net at authoring time.
        checks::assert_targets(
            "skiff",
            &h,
            &checks::Targets {
                lwl: LWL,
                bwl: BWL,
                displacement_t: 0.30,
                cb: 0.698,
                cp: 0.776,
                cm: 0.900,
                cwp: 0.883,
                lcb_frac: 0.398,
            },
        );
        // The frozen Design displacement and LCG are the measured ones.
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-3);
        assert!((h.lcb.x - DESIGN_LCG).abs() < 1e-3);
        // Particulars the table does not test: LOA 4.3 m, beam 1.44 m at the sheer, 1.0 m
        // across the bottom, depth amidships 0.47 m.
        let (min, max) = s.extents();
        assert!((max.x - min.x - 4.3).abs() < 1e-9, "LOA {}", max.x - min.x);
        assert!((2.0 * max.y - 1.44).abs() < 1e-3, "beam {}", 2.0 * max.y);
        assert!((2.0 * chine(1.5).y - 1.0).abs() < 1e-9);
        assert!((sheer(2.15).z - DEPTH).abs() < 5e-3);
    }

    #[test]
    fn skiff_keeps_its_transom_outline() {
        let (s, _) = build();
        checks::assert_transom_outline(&s, FOOT.0 / (CHINE_STATIONS - 1) as f64);
    }

    #[test]
    fn skiff_is_closed_stable_and_floats_level() {
        let (s, p) = build();
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "keel at z = 0");
        checks::assert_closed_below(&s);
        checks::assert_stability(&s, KG, DESIGN_LCG);
        checks::assert_free_trim(&s, &p);
    }

    #[test]
    fn skiff_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
    }
}
