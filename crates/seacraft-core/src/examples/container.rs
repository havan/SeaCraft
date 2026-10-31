//! "KCS-like" 230 m container ship: a bulbous bow in the hull net, a
//! parallel midbody, a closed skeg and a transom stern whose bottom sits
//! just above the design waterline, with a flat deck.
//!
//! The particulars are those of the KRISO Container Ship (KCS) benchmark
//! (SIMMAN 2008): Lpp 230 m, LWL 232.5 m, B 32.2 m, D 19.0 m, T 10.8 m,
//! ∇ 52 030 m³ (Δ 53 331 t in sea water), CM 0.985, LCB 1.48 % Lpp aft of
//! midship, 24 kn (Fn 0.26). The lines are SeaCraft's own: this is not the
//! KCS offset table, only a hull matched to its particulars. SeaCraft's Cb is
//! on LWL, so the target is 52 030 / (232.5 · 32.2 · 10.8) = 0.644, not the
//! published 0.6505 on Lpp; Cp = Cb / CM = 0.654.
//!
//! Coordinates: `x = 0` at the aft perpendicular, the forward perpendicular
//! (the stem at the waterline) at `x = Lpp`, midship at `x = 115`; keel at
//! `z = 0`. The waterline runs from the counter, about 2.6 m aft of the AP,
//! to the FP.
//!
//! **KG** is not a published figure (the KG 7.28 m / GM 0.60 m pair quoted
//! for KCS is inconsistent with its KM of about 14.7 m). The example's KG is
//! the measured KM at the design draft minus 0.60 m, i.e. GM 0.60 m, and is
//! labelled that way in the sidecar.
//!
//! **Bulb** (Kracht's parameters): protrusion 0.025 Lpp ahead of the FP,
//! nose at 0.55 T above the base, breadth about 0.15 B, the bulb's top more
//! than 1 m below the waterline, a pinched neck row between the bulb and the
//! waterline. The net is one all-quad station grid: the last stations bend
//! round the bulb (each row runs to its own point on the stem profile), so
//! the stem profile is non-monotonic in `x`. The stem, keel and deck
//! centreline are the half model's centreline boundary, which the kernel
//! pins: the stem profile stays the control polyline in [`STEM`].
//!
//! Stations from the counter to the forebody are generated from [`SECTIONS`]
//! (a superellipse bilge from the flat of bottom to the side, concave aft for
//! the skeg); the fullness of every station outside the parallel midbody was
//! fitted once to a target sectional-area curve (the ignored `fit_sections`
//! test re-derives it) and frozen.
//!
//! Layers: Hull, Transom, Deck (closed). Creases: the deck edge and the
//! transom outline. The sidecar carries the Design loading condition with
//! two tanks (a pressed-up double-bottom ballast tank, a slack fuel deep
//! tank), the No. 1 hold as a compartment and its flooding as a damage case.

use super::{
    close_half_deck, close_half_transom, crease_loop, crease_path, design_condition, item, layer,
    plating, round, round_to_fef, settings, station_grid, Built, Example, STEEL,
};
use crate::damage::{Compartment, CompartmentSpec, DamageCase};
use crate::loading::{evaluate_tank, Tank, WeightItem};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};

const LPP: f64 = 230.0;
const LWL: f64 = 232.5;
const B: f64 = 32.2;
const T: f64 = 10.8;
const D: f64 = 19.0;
/// KCS ∇ 52 030 m³ in sea water (t).
const TARGET_DISPLACEMENT_T: f64 = 53_331.0;
const TARGET_CB: f64 = 0.644;
/// LCB 1.48 % Lpp aft of midship, as a fraction of LWL from the waterline's
/// nominal aft end (`LWL − Lpp` aft of the AP).
const TARGET_LCB_FRAC: f64 = (0.5 * LPP - 0.0148 * LPP + (LWL - LPP)) / LWL;
const SPEED_KN: f64 = 24.0;
/// KM at the design draft (m), measured at authoring time and frozen.
const KM_DESIGN: f64 = 14.653;
/// The design GM (m) the example's KG is derived from.
const GM_DESIGN: f64 = 0.60;
/// The example's KG above the keel: measured KM − 0.60 m.
const KG: f64 = KM_DESIGN - GM_DESIGN;
/// Δ of the refined hull at T (t), measured at authoring time and frozen.
const DESIGN_DISPLACEMENT_T: f64 = 53_349.2;
/// LCB of the refined hull at T (model x, m), measured and frozen: the
/// Design condition's LCG.
const DESIGN_LCG: f64 = 111.516;

/// Number of rows per station, keel to deck edge.
const ROWS: usize = 12;

/// A regular station: `(x, keel z, flat half-width, bilge-top z,
/// superellipse exponent, WL half-breadth, deck half-breadth)`.
type Sec = (f64, f64, f64, f64, f64, f64, f64);

const SECTIONS: [Sec; 21] = [
    (0.0, 9.40, 3.000, 10.300, 1.500, 10.00, 13.60),
    (4.5, 1.50, 0.800, 10.200, 0.450, 10.80, 14.00),
    (10.0, 0.40, 0.900, 10.000, 0.450, 11.60, 14.50),
    (17.0, 0.00, 0.939, 9.892, 0.425, 12.80, 15.00),
    (25.0, 0.00, 1.604, 9.459, 0.524, 14.20, 15.50),
    (34.5, 0.00, 2.517, 8.936, 0.643, 15.10, 15.90),
    (46.0, 0.00, 4.280, 7.970, 0.864, 15.80, 16.10),
    (57.5, 0.00, 6.351, 6.841, 1.122, 16.10, 16.10),
    (69.0, 0.00, 9.148, 5.249, 1.486, 16.10, 16.10),
    (80.5, 0.00, 13.100, 3.000, 2.000, 16.10, 16.10),
    (92.0, 0.00, 13.100, 3.000, 2.000, 16.10, 16.10),
    (103.5, 0.00, 13.100, 3.000, 2.000, 16.10, 16.10),
    (115.0, 0.00, 13.100, 3.000, 2.000, 16.10, 16.10),
    (126.5, 0.00, 13.100, 3.000, 2.000, 16.10, 16.10),
    (138.0, 0.00, 13.100, 3.000, 2.000, 16.10, 16.10),
    (149.5, 0.00, 9.919, 4.746, 1.688, 15.95, 16.10),
    (161.0, 0.00, 6.430, 6.617, 1.354, 15.45, 16.10),
    (172.5, 0.00, 4.004, 7.864, 1.131, 14.30, 15.90),
    (184.0, 0.00, 2.162, 8.910, 0.945, 12.55, 15.40),
    (195.5, 0.00, 1.160, 9.593, 0.823, 10.00, 14.40),
    (207.0, 0.00, 0.816, 9.963, 0.757, 6.90, 12.50),
];

/// The transom outline (station 0) at `TRANSOM_X`, keel row to deck edge.
const TRANSOM_X: f64 = -3.0;
const TRANSOM: [(f64, f64); ROWS] = [
    (0.0, 11.0),
    (2.2, 11.03),
    (4.4, 11.1),
    (6.6, 11.2),
    (8.6, 11.35),
    (10.2, 11.6),
    (11.2, 12.0),
    (11.7, 12.8),
    (12.0, 13.8),
    (12.3, 15.5),
    (12.5, 17.2),
    (12.6, 19.0),
];

/// The bulb section (rows 0..=7): `(half-breadth, z)`.
const BULB: [(f64, f64); 8] = [
    (0.0, 0.0),
    (1.0, 0.35),
    (2.05, 1.4),
    (2.7, 3.4),
    (2.82, 5.94),
    (2.55, 7.8),
    (1.8, 9.0),
    (0.8, 9.7),
];

/// The stem profile on the centreline, keel row to deck edge: `(x, z)`.
const STEM: [(f64, f64); ROWS] = [
    (226.0, 0.0),
    (230.0, 0.6),
    (232.8, 1.6),
    (234.8, 3.4),
    (235.75, 5.94),
    (235.0, 7.8),
    (232.0, 9.0),
    (230.4, 9.65),
    (230.0, 10.8),
    (231.2, 13.6),
    (233.6, 16.4),
    (236.5, 19.0),
];

/// Bow stations between the last regular station and the stem: how far
/// along its run to the stem point each row is (the waterline and above;
/// the keel, bulb and neck rows), and the blend from the last regular
/// section to [`BULB`].
const BOW: [(f64, f64, f64); 4] = [
    (0.45, 0.45, 0.6),
    (0.72, 0.72, 1.0),
    (0.86, 0.88, 1.0),
    (0.95, 0.975, 1.0),
];
/// Where the bulb rows reach full breadth, as a fraction of their run; the
/// nose ahead of it is elliptic in plan.
const NOSE_START: f64 = 0.72;

/// Arc-length fractions of rows 3..=7 along the bilge curve and side.
const GIRTH: [f64; 5] = [0.15, 0.32, 0.5, 0.67, 0.84];

fn section(sec: &Sec) -> Vec<(f64, f64)> {
    let &(_, zk, bf, h, n, bw, bd) = sec;
    // Bilge curve (superellipse quadrant) then the side to the waterline.
    let mut pts: Vec<(f64, f64)> = (0..=200)
        .map(|k| {
            let th = std::f64::consts::FRAC_PI_2 * k as f64 / 200.0;
            let u = th.sin().powf(2.0 / n);
            let v = th.cos().powf(2.0 / n);
            (bf + (bw - bf) * u, h - (h - zk) * v)
        })
        .collect();
    pts.push((bw, T));
    let mut acc = vec![0.0];
    for w in pts.windows(2) {
        let d = ((w[1].0 - w[0].0).powi(2) + (w[1].1 - w[0].1).powi(2)).sqrt();
        acc.push(acc.last().unwrap() + d);
    }
    let total = *acc.last().unwrap();
    let at = |f: f64| -> (f64, f64) {
        let s = f * total;
        let k = acc.partition_point(|&a| a < s).clamp(1, pts.len() - 1);
        let t = (s - acc[k - 1]) / (acc[k] - acc[k - 1]).max(1e-12);
        (
            pts[k - 1].0 + t * (pts[k].0 - pts[k - 1].0),
            pts[k - 1].1 + t * (pts[k].1 - pts[k - 1].1),
        )
    };
    let mut out = vec![(0.0, zk), (0.5 * bf, zk), (bf, zk)];
    out.extend(GIRTH.iter().map(|&f| at(f)));
    out.push((bw, T));
    out.push((bw + 0.42 * (bd - bw), T + (D - T) / 3.0));
    out.push((bw + 0.8 * (bd - bw), T + 2.0 * (D - T) / 3.0));
    out.push((bd, D));
    out
}

pub(super) fn build() -> Built {
    build_with(&SECTIONS)
}

fn build_with(sections: &[Sec]) -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Hull", 0x3a_3a_3a, false, plating(STEEL, 18.0)),
        layer(2, "Transom", 0x7a_20_20, false, plating(STEEL, 18.0)),
        layer(3, "Deck", 0x9c_2a_2a, false, plating(STEEL, 15.0)),
    ];
    s.settings = settings(Example::Container230m, LWL, B, T);

    let mut stations: Vec<Vec<Vec3>> = Vec::new();
    stations.push(
        TRANSOM
            .iter()
            .map(|&(y, z)| Vec3::new(TRANSOM_X, y, z))
            .collect(),
    );
    for sec in sections {
        stations.push(
            section(sec)
                .into_iter()
                .map(|(y, z)| Vec3::new(sec.0, y, z))
                .collect(),
        );
    }
    let last = sections[sections.len() - 1];
    let reference = section(&last);
    let x_ref = last.0;
    // Each row runs from the last regular station to its own stem point, so
    // the bow stations bend round the bulb.
    for &(su, sb, w) in &BOW {
        let st: Vec<Vec3> = (0..ROWS)
            .map(|j| {
                let (xs, zs) = STEM[j];
                let (yr, zr) = reference[j];
                if j < BULB.len() {
                    // Keel (y = 0), bulb and neck rows: blend into the bulb section,
                    // then round the nose (elliptic in plan). The rows under
                    // the waterline turn into the neck later.
                    let sf = if j == 0 { su } else { sb };
                    let x = x_ref + sf * (xs - x_ref);
                    let (yb, zb) = BULB[j];
                    let w = if j > 5 {
                        w * (1.0 - 0.25 * (j - 5) as f64)
                    } else {
                        w
                    };
                    let u = ((sf - NOSE_START) / (1.0 - NOSE_START)).max(0.0);
                    let r = (1.0 - u * u).max(0.0).sqrt();
                    Vec3::new(x, ((1.0 - w) * yr + w * yb) * r, (1.0 - w) * zr + w * zb)
                } else {
                    // The waterline and above: a fine entrance.
                    let sf = su;
                    let x = x_ref + sf * (xs - x_ref);
                    let g = (1.0 - sf) * (1.0 + 0.06 * sf);
                    Vec3::new(x, yr * g, zr + sf * (zs - zr))
                }
            })
            .collect();
        stations.push(st);
    }
    stations.push(STEM.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect());

    let grid = station_grid(&mut s, &stations, |_, _| 0);
    let sheer: Vec<usize> = grid.iter().map(|st| st[ROWS - 1]).collect();
    let centre = close_half_deck(&mut s, 2, &sheer);
    let transom = close_half_transom(&mut s, 1, &grid[0], centre[0]);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &sheer);
    crease_loop(&mut s, &transom);

    (s, project())
}

/// A box tank on the centreline, inside the hull (not clipped).
fn tank(name: &str, x: (f64, f64), half_width: f64, z: (f64, f64), fill: f64, rho: f64) -> Tank {
    Tank {
        name: name.into(),
        x_min: x.0,
        x_max: x.1,
        y_min: -half_width,
        y_max: half_width,
        z_min: z.0,
        z_max: z.1,
        fill,
        density_t_m3: rho,
        permeability: 0.98,
        clip_to_hull: false,
    }
}

/// A pressed-up double-bottom ballast tank amidships (no free surface) and a
/// slack fuel deep tank forward of the engine room.
fn tanks() -> Vec<Tank> {
    vec![
        tank(
            "No. 3 double-bottom water ballast (C)",
            (103.5, 126.5),
            12.0,
            (0.0, 2.0),
            1.0,
            1.025,
        ),
        tank(
            "HFO deep tank (C)",
            (50.0, 60.0),
            4.0,
            (2.0, 14.0),
            0.9,
            0.95,
        ),
    ]
}

/// The forward hold, from the collision bulkhead (0.074 Lpp aft of the FP)
/// to the No. 2 hold, inner bottom to deck; SOLAS permeability for
/// container spaces.
fn no1_hold() -> Compartment {
    Compartment {
        name: "No. 1 hold".into(),
        x_min: 190.0,
        x_max: 213.0,
        y_min: -0.5 * B,
        y_max: 0.5 * B,
        z_min: 2.0,
        z_max: D,
        permeability: 0.70,
        clip_to_hull: true,
    }
}

fn project() -> Project {
    let tanks = tanks();
    // The tanks are boxes, so their contents are known without the hull;
    // they count in the balance that sets the lightship's mass and centre.
    let tank_weights: Vec<WeightItem> = tanks
        .iter()
        .map(|t| {
            let e = evaluate_tank(t, None);
            WeightItem {
                name: t.name.clone(),
                mass_t: e.mass_t,
                x: e.x,
                y: e.y,
                z: e.z,
            }
        })
        .collect();
    let items = vec![
        item(
            "Containers in the holds and on deck (2 600 TEU at 12 t)",
            31_200.0,
            120.0,
            16.5,
        ),
        item(
            "HFO in the double-bottom and wing tanks",
            3_200.0,
            75.0,
            3.0,
        ),
        item("Diesel oil and lubricants", 350.0, 30.0, 6.0),
        item(
            "Stores, fresh water, crew and provisions",
            450.0,
            25.0,
            18.0,
        ),
    ];
    let mut balance = items.clone();
    balance.extend(tank_weights);
    let mut design = design_condition(
        DESIGN_DISPLACEMENT_T,
        DESIGN_LCG,
        KG,
        "Lightship (hull, machinery, outfit)",
        balance,
    );
    design.items = items;
    design.tanks = tanks;

    let hold = no1_hold();
    Project {
        kg: Some(round(KG, 3)),
        requirements: Some(Requirements {
            name: "230 m container ship (KCS-like); KG = measured KM − 0.60 m".into(),
            lwl: Some(LWL),
            bwl: Some(B),
            draft: Some(T),
            depth: Some(D),
            displacement_t: Some(TARGET_DISPLACEMENT_T),
            cb: Some(TARGET_CB),
            lcb_frac: Some(round(TARGET_LCB_FRAC, 4)),
            design_speed_kn: Some(SPEED_KN),
            kg: None,
        }),
        damage_cases: vec![DamageCase {
            name: "No. 1 hold flooded".into(),
            compartments: vec![CompartmentSpec::Named(hold.name.clone())],
            loading_condition_name: Some("Design".into()),
            ..Default::default()
        }],
        compartments: vec![hold],
        loading_conditions: vec![design],
        report: ReportSettings {
            kg: Some(round(KG, 3)),
            lcg_x: Some(DESIGN_LCG),
            speeds_kn: Some(vec![16.0, 18.0, 20.0, 22.0, 24.0, 26.0]),
            loading_conditions: Some(vec!["Design".into()]),
            damage_cases: Some(vec!["No. 1 hold flooded".into()]),
            ..Default::default()
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::super::checks;
    use super::*;

    /// Target sectional-area curve `(x, A / Am)` (authoring only): Cp 0.654
    /// on LWL, centroid 1.5 % Lpp aft of midship, parallel midbody from
    /// station 7 to 12, the bulb's area at the FP. The counter stations
    /// (x ≤ 10 m) are hand-set and come out fuller than this curve.
    const SAC: [(f64, f64); 23] = [
        (-2.5, 0.0),
        (0.0, 0.04),
        (11.5, 0.15),
        (23.0, 0.30),
        (34.5, 0.485),
        (46.0, 0.68),
        (57.5, 0.83),
        (69.0, 0.93),
        (80.5, 0.985),
        (92.0, 1.0),
        (103.5, 1.0),
        (115.0, 1.0),
        (126.5, 1.0),
        (138.0, 0.99),
        (149.5, 0.94),
        (161.0, 0.835),
        (172.5, 0.684),
        (184.0, 0.505),
        (195.5, 0.34),
        (207.0, 0.22),
        (218.5, 0.11),
        (230.0, 0.09),
        (235.75, 0.0),
    ];

    fn sac_target(x: f64) -> f64 {
        let k = SAC.partition_point(|p| p.0 < x).clamp(1, SAC.len() - 1);
        let (a, b) = (SAC[k - 1], SAC[k]);
        a.1 + (x - a.0) / (b.0 - a.0) * (b.1 - a.1)
    }

    /// A station of fullness `k` (0 = finest, 1 = the midship section with
    /// its 3 m bilge radius): the flat of bottom widens, the bilge top drops
    /// and the superellipse fills out (concave aft for the skeg) with `k`.
    fn sec_from_k(x: f64, bw: f64, bd: f64, k: f64) -> Sec {
        let n0 = if x < 115.0 { 0.4 } else { 0.75 };
        (
            x,
            0.0,
            0.8 + (bw - 3.0 - 0.8) * k,
            3.0 + (1.0 - k) * (10.0 - 3.0),
            n0 + (2.0 - n0) * k,
            bw,
            bd,
        )
    }

    fn areas(sections: &[Sec]) -> Vec<f64> {
        let (s, _) = build_with(sections);
        let hull = crate::hydro::Hull::new(&s, checks::level3());
        let wp = crate::hydro::waterplane(&s, T, 0.0, 0.0);
        let am = crate::hydro::section_area(&hull.verts, &hull.tris, 115.0, &wp);
        sections
            .iter()
            .map(|sec| crate::hydro::section_area(&hull.verts, &hull.tris, sec.0, &wp) / am)
            .collect()
    }

    /// Re-derives the fullness of the calibrated stations (`SECTIONS[3..]`
    /// outside the parallel midbody) against the target SAC and prints the
    /// table to freeze.
    #[test]
    #[ignore]
    fn fit_sections() {
        let mut secs: Vec<Sec> = SECTIONS.to_vec();
        let free: Vec<usize> = (3..secs.len())
            .filter(|&i| !(80.0..139.0).contains(&secs[i].0))
            .collect();
        let mut ks: Vec<f64> = free.iter().map(|&i| sac_target(secs[i].0)).collect();
        for it in 0..24 {
            for (n, &i) in free.iter().enumerate() {
                secs[i] = sec_from_k(secs[i].0, secs[i].5, secs[i].6, ks[n]);
            }
            let a = areas(&secs);
            let mut worst = 0.0f64;
            for (n, &i) in free.iter().enumerate() {
                let e = sac_target(secs[i].0) - a[i];
                worst = worst.max(e.abs());
                ks[n] = (ks[n] + 0.6 * e).clamp(0.0, 1.0);
            }
            println!("iteration {it}: worst SAC error {worst:.4}");
        }
        let a = areas(&secs);
        for (i, sec) in secs.iter().enumerate() {
            println!(
                "    ({:.1}, {:.2}, {:.3}, {:.3}, {:.3}, {:.2}, {:.2}), // a {:.3} target {:.3}",
                sec.0,
                sec.1,
                sec.2,
                sec.3,
                sec.4,
                sec.5,
                sec.6,
                a[i],
                sac_target(sec.0)
            );
        }
    }

    #[test]
    fn container_meets_the_board_targets() {
        let (s, _) = build();
        let h = checks::measure(&s);
        checks::assert_targets(
            "container",
            &h,
            &checks::Targets {
                lwl: LWL,
                bwl: B,
                displacement_t: TARGET_DISPLACEMENT_T,
                cb: TARGET_CB,
                cp: 0.654,
                cm: 0.985,
                cwp: 0.82,
                lcb_frac: TARGET_LCB_FRAC,
            },
        );
        // The frozen Design displacement, LCG and KM are the measured ones.
        println!(
            "container: Δ {:.3} t, LCB x {:.4} m, KM {:.4} m, KB {:.4} m, S {:.0} m²",
            h.displacement_t, h.lcb.x, h.km_transverse, h.kb, h.wetted_surface_m2
        );
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 0.1);
        assert!((h.lcb.x - DESIGN_LCG).abs() < 1e-3);
        assert!((h.km_transverse - KM_DESIGN).abs() < 1e-3);
        // The waterline ends at the FP; LCB 1.48 % Lpp aft of midship
        // within the board's 0.5 % of LWL; Cm at midship (Lpp / 2) is the
        // same as at mid-LWL (both in the parallel midbody).
        assert!((h.wl_aft_x + h.lwl - LPP).abs() < 0.05, "WL fore end");
        let lcb_aft = (0.5 * LPP - h.lcb.x) / LPP;
        println!("LCB {:.3} % Lpp aft of midship", 100.0 * lcb_aft);
        assert!((lcb_aft - 0.0148).abs() < 0.005 * LWL / LPP);
        let hull = crate::hydro::Hull::new(&s, checks::level3());
        let mut opts = crate::hydro::HydroOptions::from_surface(&s, checks::level3());
        opts.mainframe_x = Some(0.5 * LPP);
        let at_midship = hull.hydrostatics(&opts);
        assert!((at_midship.midship_coefficient - h.midship_coefficient).abs() < 1e-4);
        // Depth 19 m, transom above the waterline, LOA about 240 m.
        let (min, max) = s.extents();
        assert!((max.z - D).abs() < 1e-9 && min.z == 0.0);
        assert!((max.x - min.x - 240.0).abs() < 1.0, "LOA {}", max.x - min.x);
        let transom = s.layers.iter().position(|l| l.name == "Transom").unwrap();
        let fi = s.face_layer.iter().position(|&l| l == transom).unwrap();
        let bottom = s.faces[fi]
            .iter()
            .map(|&p| s.points[p].p.z)
            .fold(f64::INFINITY, f64::min);
        assert!(bottom > T && bottom < T + 0.5, "transom bottom {bottom}");
        let names: Vec<&str> = s.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Hull", "Transom", "Deck"]);
    }

    /// The bulb against Kracht's parameters, measured on the refined hull.
    #[test]
    fn container_bulb_has_the_kracht_proportions() {
        let (s, _) = build();
        let mesh = crate::subdiv::evaluate(&s, checks::level3());
        // Protrusion and nose height: the stem profile is pinned.
        let nose = mesh
            .verts
            .iter()
            .filter(|p| p.z < T)
            .fold(Vec3::ZERO, |a, &p| if p.x > a.x { p } else { a });
        let protrusion = (nose.x - LPP) / LPP;
        // Breadth: the widest point of the hull ahead of the FP below the WL
        // (only the bulb is there).
        let ahead: Vec<&Vec3> = mesh.verts.iter().filter(|p| p.x > LPP && p.z < T).collect();
        let breadth = 2.0 * ahead.iter().map(|p| p.y).fold(0.0, f64::max);
        // Top: the highest point of the bulb ahead of the FP.
        let top = ahead
            .iter()
            .filter(|p| p.x > LPP + 0.5)
            .map(|p| p.z)
            .fold(0.0, f64::max);
        // Section area at the FP against the midship section.
        let hull = crate::hydro::Hull::new(&s, checks::level3());
        let wp = crate::hydro::waterplane(&s, T, 0.0, 0.0);
        let area = |x| crate::hydro::section_area(&hull.verts, &hull.tris, x, &wp);
        let fp_ratio = area(LPP) / area(0.5 * LPP);
        println!(
            "bulb: protrusion {:.4} Lpp, nose {:.3} T, breadth {:.4} B, top {:.2} m below WL, A_FP {:.3} Am",
            protrusion,
            nose.z / T,
            breadth / B,
            T - top,
            fp_ratio
        );
        assert!((protrusion - 0.025).abs() < 1e-3);
        assert!((nose.z / T - 0.55).abs() < 0.01);
        assert!((breadth / B - 0.15).abs() < 0.015, "breadth {breadth}");
        assert!(T - top >= 1.0, "bulb top {top}");
        assert!((0.06..0.12).contains(&fp_ratio), "A_FP / Am {fp_ratio}");

        // Smooth, no fold: across every interior edge of the refined bow
        // (x > 200 m, below the deck) neighbouring faces turn by less than
        // 35° (the steepest bend is at the pinned stem polyline's corners),
        // and every face there still faces outboard or along the hull.
        let normal = |f: &Vec<usize>| {
            let mut n = Vec3::ZERO;
            for k in 0..f.len() {
                let (a, b) = (mesh.verts[f[k]], mesh.verts[f[(k + 1) % f.len()]]);
                n.x += (a.y - b.y) * (a.z + b.z);
                n.y += (a.z - b.z) * (a.x + b.x);
                n.z += (a.x - b.x) * (a.y + b.y);
            }
            n * (1.0 / n.length())
        };
        let mut edge_faces: std::collections::HashMap<(usize, usize), Vec<usize>> =
            Default::default();
        for (fi, f) in mesh.faces.iter().enumerate() {
            if mesh.face_layer[fi] != 0 {
                continue;
            }
            let c = f.iter().fold(Vec3::ZERO, |a, &v| a + mesh.verts[v]) * (1.0 / f.len() as f64);
            if c.x < 200.0 || c.z > D - 0.5 {
                continue;
            }
            assert!(normal(f).y > -0.05, "a bow face turned inboard at {c:?}");
            for k in 0..f.len() {
                let (a, b) = (f[k], f[(k + 1) % f.len()]);
                edge_faces.entry((a.min(b), a.max(b))).or_default().push(fi);
            }
        }
        let (worst, at) = edge_faces
            .iter()
            .filter(|(_, fs)| fs.len() == 2)
            .map(|(e, fs)| {
                let d = normal(&mesh.faces[fs[0]]).dot(normal(&mesh.faces[fs[1]]));
                (d.clamp(-1.0, 1.0).acos().to_degrees(), mesh.verts[e.0])
            })
            .fold((0.0, Vec3::ZERO), |a, b| if b.0 > a.0 { b } else { a });
        println!("bulb: steepest bend between refined faces {worst:.1}° at {at:?}");
        assert!(
            worst < 35.0,
            "a fold or kink in the bow: {worst}° at {at:?}"
        );
    }

    #[test]
    fn container_is_closed_and_stable() {
        let (s, _) = build();
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "keel at z = 0");
        checks::assert_closed_below(&s);
        let (gm, _) = checks::assert_stability(&s, KG, DESIGN_LCG);
        assert!((gm - GM_DESIGN).abs() < 1e-3, "GM {gm}");
    }

    /// The Design condition (tanks included) floats level at the design
    /// draft; IMO A.749 is run at it and reported, never tuned.
    #[test]
    fn container_design_condition_floats_level_and_meets_imo_a749() {
        let (s, p) = build();
        let cond = p.loading_condition(Some("Design")).unwrap();
        let opts = crate::hydro::HydroOptions::from_surface(&s, checks::level3());
        // The shared acceptance check counts the tanks too (R8-2 A09).
        let (t_free, trim_free) = checks::assert_free_trim(&s, &p);
        let r = crate::loading::evaluate(&s, &opts, cond);
        assert!((t_free - r.draft).abs() < 1e-6 && (trim_free - r.trim_m).abs() < 1e-6);
        println!(
            "design: Δ {:.1} t LCG {:.3} KG {:.3} FSC {:.4} -> T {:.3} trim {:.3} GM solid {:.3} fluid {:.3}",
            r.displacement_t, r.lcg, r.kg_solid, r.fsc_m, r.draft, r.trim_m, r.gm_solid, r.gm_fluid
        );
        assert!((r.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 0.01);
        assert!((r.lcg - DESIGN_LCG).abs() < 2e-3);
        assert!((r.kg_solid - KG).abs() < 2e-3);
        assert!(r.trim_m.abs() < 0.005 * LWL, "trim {}", r.trim_m);
        assert!((r.draft - T).abs() < 0.02, "draft {}", r.draft);
        assert!((r.gm_solid - GM_DESIGN).abs() < 0.01);
        // Two tanks: the ballast pressed up (no free surface), the fuel
        // slack; their contents are what the balance assumed.
        assert_eq!(r.tanks.len(), 2);
        let hull = crate::hydro::Hull::new(&s, checks::level3());
        for (t, summary) in cond.tanks.iter().zip(&r.tanks) {
            let clipped = crate::loading::evaluate_tank(
                &Tank {
                    clip_to_hull: true,
                    ..t.clone()
                },
                Some(&hull),
            );
            assert!(
                (clipped.moulded_volume_m3 / summary.moulded_volume_m3 - 1.0).abs() < 0.01,
                "tank `{}` is not inside the hull",
                t.name
            );
        }
        assert_eq!(r.tanks[0].fsm_t_m, 0.0);
        assert!(r.tanks[1].fsm_t_m > 0.0 && r.fsc_m < 0.02);
        let imo = r.imo_criteria.as_ref().expect("IMO A.749 evaluated");
        println!(
            "IMO A.749: area 0-30 {:.4} 0-40 {:.4} 30-40 {:.4} m·rad, GZmax {:.3} m at {:.1}°, GM0 {:.3} m -> {}",
            imo.area_0_30_m_rad,
            imo.area_0_40_m_rad,
            imo.area_30_40_m_rad,
            imo.max_gz_30_plus_m,
            imo.angle_of_max_gz_deg,
            imo.gm0_m,
            if imo.passed { "pass" } else { "fail" }
        );
        assert!(imo.passed, "IMO A.749 at the design condition: {imo:?}");
    }

    /// Flooding the No. 1 hold: reported, with the residual criteria.
    #[test]
    fn container_no1_hold_damage_case_evaluates() {
        let (s, p) = build();
        let case = p
            .resolve_damage_case(p.damage_case(Some("No. 1 hold flooded")).unwrap())
            .unwrap();
        let opts = crate::hydro::HydroOptions::from_surface(&s, checks::level3());
        let r = crate::damage::evaluate(&s, &opts, &case).unwrap();
        println!(
            "damage: lost {:.0} m³, T {:.3} (sinkage {:.3}) trim {:.3} heel {:.2}°, freeboard {:?}, found {}",
            r.lost_volume_m3, r.draft, r.sinkage_m, r.trim_m, r.heel_deg, r.freeboard_m,
            r.equilibrium_found
        );
        println!("damage: {:?}", r.criteria);
        println!("damage warnings: {:?}", r.warnings);
        assert!(r.equilibrium_found);
        assert!(r.lost_volume_m3 > 0.0);
        assert!(r.trim_m < 0.0, "flooding forward trims by the bow");
        assert!(r.sinkage_m > 0.0);
    }

    #[test]
    fn container_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
        assert_eq!(p.loading_conditions[0].tanks.len(), 2);
        assert_eq!(p.compartments.len(), 1);
        assert_eq!(p.damage_cases.len(), 1);
        p.resolve_damage_case(&p.damage_cases[0])
            .and_then(|c| crate::damage::validate(&c))
            .expect("the damage case resolves and validates");
    }
}
