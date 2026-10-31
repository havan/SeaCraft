//! 10 m fin-keel cruising yacht: a light canoe body with a counter transom,
//! a closed flat deck, a swept fin keel and a spade rudder.
//!
//! Targets (examples board, Opus table; "representative of production
//! boats of this size", not any one model): LOA 10.4 m, LWL 9.0 m, BWL
//! 2.9 m (BOA 3.4 m), canoe-body draft Tc 0.45 m, total draft 1.85 m,
//! canoe-body depth 1.45 m, Δ 5.0 t; canoe body Cb 0.40, Cp 0.55, Cm 0.73,
//! Cwp 0.70, LCB 46.5 % LWL from aft; 6.5 kn; KG 1.76 m above the keel tip
//! (3.3 t at 0.35 m above the DWL and 1.6 t of ballast 1.0 m below it),
//! GM ≈ 1.8 m.
//!
//! The keel tip is the lowest hydrostatic point, so it is the datum: it sits
//! at `z = 0`, the design waterline at `z = T` = 1.85 m and the bottom of the
//! canoe body at `z = T − Tc` = 1.40 m. Draft-based coefficients of the whole
//! hull would divide by the total draft (Cb ≈ 0.1), so the canoe-body
//! coefficients are measured with the appendage layers out of the
//! hydrostatics and the draft set to Tc; Δ, KB, GM and GZ with them in.
//!
//! Layers: Hull (canoe body and transom), Deck (closed, so the large-angle
//! GZ is real), Fin keel and Rudder. The fin and the rudder are closed half
//! shells of their own (a foil skin, a tip cap and a root cap) whose root
//! cap sits just inside the canoe body, so switching either out of the
//! hydrostatics leaves every layer watertight; the volume they share with
//! the canoe body is a fraction of a percent of ∇ (a test measures it).
//! Creases: the sheer, the transom outline and the fin and rudder tip and
//! root outlines; the keel, the stem, the deck centreline and the foils'
//! leading and trailing edges are the half model's centreline boundary,
//! which the kernel pins (they stay the control polylines).

use super::{
    add_oriented, close_half_deck, close_half_transom, crease_loop, crease_path, design_condition,
    item, layer, plating, round_to_fef, settings, station_grid, Built, Example, Plating, GRP, LEAD,
};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::{Project, ReportSettings, Requirements};
use crate::weather::{WindageItem, WindageRect};

const LWL: f64 = 9.0;
const BWL: f64 = 2.9;
/// Total draft, keel tip to design waterline (m): the design draft.
const T: f64 = 1.85;
/// Canoe-body draft (m).
const TC: f64 = 0.45;
/// Canoe-body depth, bottom to sheer amidships (m).
const DEPTH: f64 = 1.45;
/// Depth on the draft's datum, keel tip to sheer amidships (m): what the
/// requirements carry, so draft and depth are measured from the same point.
const DEPTH_FROM_KEEL_TIP: f64 = T - TC + DEPTH;
/// Design waterline height above the keel tip.
const DWL: f64 = T;
/// The example's KG above the keel tip (m), representative.
const KG: f64 = 1.76;
const SPEED_KN: f64 = 6.5;
/// Δ of the refined hull, appendages in, at T (t), measured at authoring
/// time and frozen.
const DESIGN_DISPLACEMENT_T: f64 = 4.954;
/// LCB of the refined hull, appendages in, at T (model x, m), measured and
/// frozen: the Design condition's LCG.
const DESIGN_LCG: f64 = 4.161;

/// Canoe-body stations aft to forward:
/// `(x, keel z, bilge y, bilge z, exponent, sheer y, sheer z)`. Below the
/// bilge point `(y, z)` the section is a superellipse quarter of the given
/// exponent from the keel; in the water the bilge point is on the DWL.
/// Station 0 is the transom (a vertical plane, its bottom 0.07 m above the
/// DWL); the stem is [`STEM`]. Calibrated at authoring time against the
/// canoe-body targets (the ignored `calibration_report` test measures a
/// candidate table) and frozen.
const STATIONS: [Station; 12] = [
    (-0.60, 1.920, 0.95, 2.020, 3.0, 1.22, 2.72),
    (0.00, 1.850, 0.95, 1.980, 3.0, 1.40, 2.74),
    (0.80, 1.720, 0.94, DWL, 2.1, 1.52, 2.76),
    (1.60, 1.580, 1.13, DWL, 2.1, 1.61, 2.78),
    (2.50, 1.470, 1.33, DWL, 2.05, 1.67, 2.80),
    (3.40, 1.400, 1.465, DWL, 2.0, 1.70, 2.83),
    (4.30, 1.400, 1.465, DWL, 2.0, 1.70, 2.86),
    (5.20, 1.430, 1.39, DWL, 2.0, 1.67, 2.89),
    (6.10, 1.490, 1.15, DWL, 1.9, 1.58, 2.93),
    (7.00, 1.560, 0.86, DWL, 1.75, 1.42, 2.97),
    (7.80, 1.630, 0.55, DWL, 1.55, 1.18, 3.00),
    (8.50, 1.690, 0.25, DWL, 1.3, 0.82, 3.03),
];

/// The stem profile on the centreline, forefoot to stem head, one `(x, z)`
/// per row; it meets the DWL at the forward end of the LWL.
const STEM: [(f64, f64); 7] = [
    (8.86, 1.74),
    (8.90, 1.77),
    (8.94, 1.80),
    (8.97, 1.83),
    (LWL, DWL),
    (9.40, 2.45),
    (9.80, 3.06),
];

/// Parameter angles of the rows on the superellipse, keel to bilge point.
const BILGE_ROWS: [f64; 5] = [0.0, 0.45, 0.85, 1.22, std::f64::consts::FRAC_PI_2];

/// A foil appendage: a trapezoid planform below the canoe body.
struct Foil {
    /// Root leading edge (model x).
    root_le_x: f64,
    root_chord: f64,
    tip_chord: f64,
    /// Root heights above the keel tip at the leading and trailing edges.
    root_z: f64,
    root_z_te: f64,
    tip_z: f64,
    /// Leading-edge sweep, degrees (positive: tip aft).
    sweep_deg: f64,
    /// Maximum thickness over chord.
    thickness: f64,
}

/// Fin keel: root chord 1.25 m, tip 0.75 m, span 1.40 m below the canoe
/// body, leading edge swept 15°, 12 % thick; its root cap is 0.03 m inside
/// the canoe body.
const FIN: Foil = Foil {
    root_le_x: 4.45,
    root_chord: 1.25,
    tip_chord: 0.75,
    root_z: T - TC + 0.03,
    root_z_te: T - TC + 0.03,
    tip_z: 0.0,
    sweep_deg: 15.0,
    thickness: 0.12,
};

/// The fin is a solid lead casting (the ballast keel), not a plated shell.
/// Its weight estimate uses the equivalent plate: the casting's volume
/// (about 0.12 m³ for this NACA 0012 planform, both sides) over its skin
/// area (about 3.05 m² at level 3) is 40 mm, so area × 40 mm × lead gives
/// the casting's mass, about 1.4 t.
const FIN_KEEL_PLATING: Plating = plating(LEAD, 40.0);

/// Spade rudder: chord 0.50 m at the root, 0.40 m at the tip, about 1.05 m
/// of span below the hull, leading edge 0.97 m forward of the aft end of the
/// LWL, 12 % thick; its root cap follows the rising buttock 0.04 m inside
/// the canoe body.
const RUDDER: Foil = Foil {
    root_le_x: 0.97,
    root_chord: 0.50,
    tip_chord: 0.40,
    root_z: 1.71,
    root_z_te: 1.80,
    tip_z: 0.70,
    sweep_deg: 8.0,
    thickness: 0.12,
};

/// Chordwise stations of the foil skin, trailing edge to leading edge, as
/// fractions of the chord from the leading edge.
const CHORD_ROWS: [f64; 6] = [1.0, 0.72, 0.46, 0.26, 0.10, 0.0];
/// Spanwise rows, tip to root, as fractions of the span.
const SPAN_ROWS: [f64; 3] = [0.0, 0.5, 1.0];

/// NACA four-digit symmetric half-thickness at chord fraction `x` for a
/// thickness ratio `t`, closed trailing edge.
fn naca_half_thickness(x: f64, t: f64) -> f64 {
    5.0 * t
        * (0.2969 * x.sqrt() - 0.1260 * x - 0.3516 * x * x + 0.2843 * x.powi(3)
            - 0.1036 * x.powi(4))
}

/// Canoe-body section rows, keel to sheer.
fn section(zk: f64, yb: f64, zb: f64, n: f64, ys: f64, zs: f64) -> Vec<(f64, f64)> {
    let mut rows: Vec<(f64, f64)> = BILGE_ROWS
        .iter()
        .map(|&th| {
            let (s, c) = (th.sin(), th.cos());
            (yb * s.powf(2.0 / n), zb - (zb - zk) * c.powf(2.0 / n))
        })
        .collect();
    rows.push((yb + 0.55 * (ys - yb), zb + 0.45 * (zs - zb)));
    rows.push((ys, zs));
    rows
}

/// Add a foil appendage on `layer`: its skin, tip cap and root cap.
/// Returns the tip and root cap outlines, to crease.
fn add_foil(s: &mut Surface, layer: usize, f: &Foil) -> (Vec<usize>, Vec<usize>) {
    let sweep = f.sweep_deg.to_radians().tan();
    // stations[i][j]: chordwise i (trailing edge to leading edge, aft to
    // forward), spanwise j (tip to root, upward): the hull's ordering, so
    // `station_grid` winds the skin outward.
    let stations: Vec<Vec<Vec3>> = CHORD_ROWS
        .iter()
        .map(|&c| {
            SPAN_ROWS
                .iter()
                .map(|&sf| {
                    let span = f.root_z + c * (f.root_z_te - f.root_z) - f.tip_z;
                    let z = f.tip_z + sf * span;
                    let le = f.root_le_x - (1.0 - sf) * (f.root_z - f.tip_z) * sweep;
                    let chord = f.tip_chord + sf * (f.root_chord - f.tip_chord);
                    Vec3::new(
                        le - c * chord,
                        chord * naca_half_thickness(c, f.thickness),
                        z,
                    )
                })
                .collect()
        })
        .collect();
    let grid = station_grid(s, &stations, |_, _| layer);
    let tip: Vec<usize> = grid.iter().map(|st| st[0]).collect();
    let root: Vec<usize> = grid.iter().map(|st| *st.last().unwrap()).collect();
    let tip = add_oriented(s, tip, layer, Vec3::new(0.0, 0.0, -1.0));
    let root = add_oriented(s, root, layer, Vec3::new(0.0, 0.0, 1.0));
    (tip, root)
}

type Station = (f64, f64, f64, f64, f64, f64, f64);

pub(super) fn build() -> Built {
    build_from(&STATIONS, &STEM, &FIN, &RUDDER)
}

fn build_from(table: &[Station], stem: &[(f64, f64)], fin: &Foil, rudder: &Foil) -> Built {
    let mut s = Surface::new();
    s.layers = vec![
        layer(1, "Hull", 0xf2_f2_ee, false, plating(GRP, 10.0)),
        layer(2, "Deck", 0xc8_b4_8c, false, plating(GRP, 8.0)),
        layer(3, "Fin keel", 0x40_40_40, false, FIN_KEEL_PLATING),
        layer(4, "Rudder", 0xb0_30_30, false, plating(GRP, 8.0)),
    ];
    s.settings = settings(Example::Yacht10m, LWL, BWL, T);

    let mut stations: Vec<Vec<Vec3>> = table
        .iter()
        .map(|&(x, zk, yb, zb, n, ys, zs)| {
            section(zk, yb, zb, n, ys, zs)
                .into_iter()
                .map(|(y, z)| Vec3::new(x, y, z))
                .collect()
        })
        .collect();
    stations.push(stem.iter().map(|&(x, z)| Vec3::new(x, 0.0, z)).collect());

    let grid = station_grid(&mut s, &stations, |_, _| 0);
    let sheer: Vec<usize> = grid.iter().map(|st| *st.last().unwrap()).collect();
    let centre = close_half_deck(&mut s, 1, &sheer);
    let transom = close_half_transom(&mut s, 0, &grid[0], centre[0]);
    let (fin_tip, fin_root) = add_foil(&mut s, 2, fin);
    let (rudder_tip, rudder_root) = add_foil(&mut s, 3, rudder);
    round_to_fef(&mut s);
    s.initialize_trace();
    crease_path(&mut s, &sheer);
    crease_loop(&mut s, &transom);
    for ring in [&fin_tip, &fin_root, &rudder_tip, &rudder_root] {
        crease_loop(&mut s, ring);
    }

    let project = Project {
        kg: Some(KG),
        requirements: Some(Requirements {
            name: "10 m cruising yacht".into(),
            lwl: Some(LWL),
            bwl: Some(BWL),
            draft: Some(T),
            // The canoe-body depth (1.45 m) sits on the canoe bottom, not on
            // the keel tip the draft is measured from.
            depth: Some(DEPTH_FROM_KEEL_TIP),
            displacement_t: Some(5.0),
            // Cb is a canoe-body target (appendages out, draft Tc); the
            // whole hull's Cb on the total draft is not comparable.
            cb: None,
            lcb_frac: Some(0.465),
            design_speed_kn: Some(SPEED_KN),
            kg: None,
        }),
        loading_conditions: vec![design_condition(
            DESIGN_DISPLACEMENT_T,
            DESIGN_LCG,
            KG,
            "Lightship (hull, rig, engine, ballast keel)",
            vec![
                item("Crew (4)", 0.32, 2.20, 2.70),
                item("Fresh water, 150 l", 0.15, 4.60, 1.75),
                item("Fuel, 60 l", 0.05, 2.80, 1.75),
                item("Stores and gear", 0.20, 5.00, 2.10),
            ],
        )],
        windage: windage(),
        report: ReportSettings {
            kg: Some(KG),
            lcg_x: Some(DESIGN_LCG),
            speeds_kn: Some(vec![4.0, 5.0, 6.0, 6.5, 7.0, 7.5]),
            ..Default::default()
        },
        ..Default::default()
    };
    (s, project)
}

/// Lateral windage at bare poles beyond the hull's own profile (which the
/// weather criterion measures from the net): a coachroof and sprayhood the
/// net does not model, the mast, the standing rigging and the furled sails.
/// Representative areas, not measured.
fn windage() -> Vec<WindageItem> {
    vec![
        WindageItem::from_profile(
            "Coachroof and sprayhood (representative)",
            WindageRect {
                x_min: 2.2,
                x_max: 6.2,
                z_min: 2.85,
                z_max: 3.35,
            },
        ),
        WindageItem {
            name: "Mast, 13.5 m above deck (representative)".into(),
            area_m2: 2.43,
            z_centroid: 9.61,
            profile: Some(WindageRect {
                x_min: 5.55,
                x_max: 5.73,
                z_min: 2.86,
                z_max: 16.36,
            }),
        },
        WindageItem {
            name: "Standing rigging, equivalent area (representative)".into(),
            area_m2: 1.2,
            z_centroid: 9.0,
            profile: None,
        },
        WindageItem {
            name: "Furled genoa and stowed mainsail (representative)".into(),
            area_m2: 3.0,
            z_centroid: 6.5,
            profile: None,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::super::checks;
    use super::*;
    use crate::hydro::{Hull, HydroOptions};
    use crate::stability::{cross_curves, StabilityCondition};
    use crate::subdiv::evaluate;

    /// The canoe body alone: the appendage layers out of the hydrostatics
    /// and the design draft set to Tc above the canoe body's bottom.
    fn canoe_body(s: &Surface) -> Surface {
        let mut c = s.clone();
        for l in &mut c.layers {
            if l.name == "Fin keel" || l.name == "Rudder" {
                l.use_in_hydrostatics = false;
            }
        }
        c.settings.draft = TC;
        c
    }

    /// Authoring aid: `YACHT_TABLE=file cargo test --release -p
    /// seacraft-core calibration_report -- --ignored --nocapture` measures
    /// a station table read from `file` (12 station lines of 7 numbers, then
    /// 7 stem lines of 2) instead of the frozen one.
    #[test]
    #[ignore]
    fn calibration_report() {
        let (s, _) = match std::env::var("YACHT_TABLE") {
            Ok(path) => {
                let text = std::fs::read_to_string(path).unwrap();
                let rows: Vec<Vec<f64>> = text
                    .lines()
                    .map(|l| l.split_whitespace().map(|w| w.parse().unwrap()).collect())
                    .filter(|r: &Vec<f64>| !r.is_empty())
                    .collect();
                let table: Vec<Station> = rows[..12]
                    .iter()
                    .map(|r| (r[0], r[1], r[2], r[3], r[4], r[5], r[6]))
                    .collect();
                let stem: Vec<(f64, f64)> = rows[12..19].iter().map(|r| (r[0], r[1])).collect();
                build_from(&table, &stem, &FIN, &RUDDER)
            }
            Err(_) => build(),
        };
        let c = checks::measure(&canoe_body(&s));
        let h = checks::measure(&s);
        println!(
            "canoe: Δ {:.4} LWL {:.4} BWL {:.4} Cb {:.4} Cp {:.4} Cm {:.4} Cwp {:.4} LCB {:.4} %  keel {:.4} aft {:.4}",
            c.displacement_t,
            c.lwl,
            c.bwl,
            c.block_coefficient,
            c.prismatic_coefficient,
            c.midship_coefficient,
            c.waterplane_coefficient,
            100.0 * checks::lcb_frac(&c),
            canoe_body(&s).lowest_hydrostatics_point(),
            c.wl_aft_x,
        );
        println!(
            "full: Δ {:.4} LCB x {:.4} ({:.3} %) KB {:.4} BM {:.4} KM {:.4} GM {:.4}",
            h.displacement_t,
            h.lcb.x,
            100.0 * checks::lcb_frac(&h),
            h.kb,
            h.bm_transverse,
            h.km_transverse,
            h.gm(KG)
        );
        let sac: Vec<String> = c
            .sac
            .iter()
            .map(|a| format!("{:.2}:{:.3}", a.x, a.area))
            .collect();
        println!("SAC {}", sac.join(" "));
        // Waterline half-breadths of the refined canoe body, 0.5 m bins.
        let m = evaluate(&s, checks::level3());
        let mut wl = [0.0f64; 21];
        for f in m
            .faces
            .iter()
            .zip(&m.face_layer)
            .filter(|(_, &l)| l == 0)
            .map(|(f, _)| f)
        {
            for k in 0..f.len() {
                let (a, b) = (m.verts[f[k]], m.verts[f[(k + 1) % f.len()]]);
                if (a.z - DWL) * (b.z - DWL) < 0.0 {
                    let t = (DWL - a.z) / (b.z - a.z);
                    let p = a + (b - a) * t;
                    let i = ((p.x + 0.25) / 0.5).floor().clamp(0.0, 20.0) as usize;
                    wl[i] = wl[i].max(p.y);
                }
            }
        }
        let wl: Vec<String> = wl
            .iter()
            .enumerate()
            .map(|(i, y)| format!("{:.1}:{y:.3}", 0.5 * i as f64))
            .collect();
        println!("WL {}", wl.join(" "));
        let (min, max) = s.extents();
        println!(
            "extents {min:?} {max:?} full LWL {:.4} BWL {:.4}",
            h.lwl, h.bwl
        );
        for (name, foil, layer) in [("fin", &FIN, 2), ("rudder", &RUDDER, 3)] {
            let (o, c) = root_overlap(&s, foil, layer);
            println!(
                "{name}: overlap {:.4} % clearance {c:.4}",
                100.0 * o / h.volume_m3
            );
        }
        let heels: Vec<f64> = (0..=18).map(|k| 10.0 * k as f64).collect();
        let gz: Vec<String> = gz_curve(&s, h.displacement_t, h.lcb.x, &heels)
            .iter()
            .map(|(a, g)| format!("{a:.0}:{g:.3}"))
            .collect();
        println!("GZ {}", gz.join(" "));
    }

    #[test]
    fn yacht_canoe_body_meets_the_board_targets() {
        let (s, _) = build();
        let canoe = canoe_body(&s);
        let bottom = canoe.lowest_hydrostatics_point();
        assert!((bottom - (T - TC)).abs() < 1e-9, "canoe bottom at {bottom}");
        let c = checks::measure(&canoe);
        checks::assert_targets(
            "yacht canoe body",
            &c,
            &checks::Targets {
                lwl: LWL,
                bwl: BWL,
                // The canoe body's share of Δ the targets imply.
                displacement_t: 1.025 * 0.40 * LWL * BWL * TC,
                cb: 0.40,
                cp: 0.55,
                cm: 0.73,
                cwp: 0.70,
                lcb_frac: 0.465,
            },
        );
    }

    #[test]
    fn yacht_with_appendages_meets_displacement_and_stability() {
        let (s, _) = build();
        assert_eq!(s.lowest_hydrostatics_point(), 0.0, "keel tip at z = 0");
        let h = checks::measure(&s);
        println!(
            "yacht: Δ {:.4} t LCB {:.4} KB {:.4} BM {:.4} KM {:.4} GM {:.4}",
            h.displacement_t,
            h.lcb.x,
            h.kb,
            h.bm_transverse,
            h.km_transverse,
            h.gm(KG)
        );
        assert!(
            (h.displacement_t / 5.0 - 1.0).abs() < 0.02,
            "Δ {} vs 5.0 t",
            h.displacement_t
        );
        assert!((h.lwl / LWL - 1.0).abs() < 0.01, "LWL {}", h.lwl);
        assert!((h.bwl / BWL - 1.0).abs() < 0.01, "BWL {}", h.bwl);
        // The frozen Design displacement and LCG are the measured ones.
        assert!((h.displacement_t - DESIGN_DISPLACEMENT_T).abs() < 1e-3);
        assert!((h.lcb.x - DESIGN_LCG).abs() < 1e-3);
        let (gm, _) = checks::assert_stability(&s, KG, DESIGN_LCG);
        assert!((gm - 1.8).abs() < 0.25, "GM {gm} vs ≈ 1.8 m");
        // Particulars the table does not test: LOA 10.4 m, BOA 3.4 m.
        let (min, max) = s.extents();
        assert!((max.x - min.x - 10.4).abs() < 1e-9, "LOA {}", max.x - min.x);
        assert!((2.0 * max.y - 3.4).abs() < 1e-9, "BOA {}", 2.0 * max.y);
    }

    /// GZ free to trim at `disp_t`, `kg` and `lcg`, straight from the cross
    /// curves at every heel — 90° and past it included.
    fn gz_at_heels(s: &Surface, disp_t: f64, kg: f64, lcg: f64, heels: &[f64]) -> Vec<f64> {
        let hull = Hull::new(s, checks::level3());
        let base = HydroOptions::from_surface(s, checks::level3());
        cross_curves(
            &hull,
            heels,
            &[disp_t],
            &StabilityCondition::free_to_trim(kg, lcg),
            &base,
        )
        .iter()
        .map(|c| {
            c.points[0]
                .gz
                .unwrap_or_else(|| panic!("no GZ at {}°", c.heel_deg))
        })
        .collect()
    }

    /// The GZ curve free to trim at `disp_t`, KG and `lcg` from 0° to 180°.
    fn gz_curve(s: &Surface, disp_t: f64, lcg: f64, heels: &[f64]) -> Vec<(f64, f64)> {
        let gz = gz_at_heels(s, disp_t, KG, lcg, heels);
        heels.iter().copied().zip(gz).collect()
    }

    /// The same curve read off the hull turned upside down (rotated 180°
    /// about x, so its keel datum is the old deck top): heel `h` of the yacht
    /// is heel `h − 180°` of the inverted hull. GZ is the lever towards
    /// smaller heel, so `GZ(h) = GZ_inv(h − 180°)`, and as GZ is odd in heel,
    /// `GZ(h) = −GZ_inv(180° − h)`. Each heel is then solved on the other
    /// side of 90°, an independent check of the large-angle solve.
    fn gz_curve_inverted(s: &Surface, disp_t: f64, lcg: f64, heels: &[f64]) -> Vec<(f64, f64)> {
        let mut inv = s.clone();
        for p in &mut inv.points {
            p.p = Vec3::new(p.p.x, -p.p.y, -p.p.z);
        }
        let kg_inv = s.highest_hydrostatics_point() - KG;
        let mirrored: Vec<f64> = heels.iter().map(|h| 180.0 - h).collect();
        let gz = gz_at_heels(&inv, disp_t, kg_inv, lcg, &mirrored);
        heels
            .iter()
            .copied()
            .zip(gz.into_iter().map(|g| -g))
            .collect()
    }

    /// The angle of vanishing stability with the deck closed, free to trim
    /// at the design Δ and KG, is past 110°, and GZ is positive up to it.
    #[test]
    fn yacht_vanishing_stability_is_past_110_degrees() {
        let (s, _) = build();
        // Every 5° from 0° to 180°, 90° itself included, with 89° and 91°
        // either side of it to check the curve is continuous there.
        let mut heels: Vec<f64> = (0..=36).map(|k| 5.0 * k as f64).collect();
        heels.extend([89.0, 91.0]);
        heels.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let gz = gz_curve(&s, DESIGN_DISPLACEMENT_T, DESIGN_LCG, &heels);
        let inverted = gz_curve_inverted(&s, DESIGN_DISPLACEMENT_T, DESIGN_LCG, &heels);
        for ((h, g), (_, gi)) in gz.iter().zip(&inverted) {
            println!("yacht GZ({h:5.1}°) = {g:+.4}  (inverted hull {gi:+.4})");
            assert!(
                (g - gi).abs() < 1e-4,
                "{h}°: GZ {g} but the inverted hull gives {gi}"
            );
        }
        let at = |h: f64| gz.iter().find(|p| p.0 == h).unwrap().1;
        assert!(
            (at(89.0) - at(90.0)).abs() < 0.03 && (at(91.0) - at(90.0)).abs() < 0.03,
            "GZ jumps across 90°: {} / {} / {}",
            at(89.0),
            at(90.0),
            at(91.0)
        );
        let avs = gz
            .windows(2)
            .skip(1)
            .find(|w| w[0].1 > 0.0 && w[1].1 <= 0.0)
            .map(|w| w[0].0 + (w[1].0 - w[0].0) * w[0].1 / (w[0].1 - w[1].1))
            .expect("GZ changes sign before 180°");
        println!("yacht AVS {avs:.1}°");
        assert!(avs > 110.0, "AVS {avs}°");
        assert!(gz[0].1.abs() < 1e-3 && gz[gz.len() - 1].1.abs() < 1e-2);
        assert!(gz[1..].iter().filter(|p| p.0 < avs).all(|p| p.1 > 0.0));
    }

    /// Where a foil meets the canoe body: the volume the two share (the
    /// vertical extent of the foil above the canoe bottom, summed over a grid
    /// of columns, both sides), and the lowest point of the root cap above
    /// the canoe bottom (negative: a gap between root and hull).
    fn root_overlap(s: &Surface, foil: &Foil, layer: usize) -> (f64, f64) {
        let m = evaluate(s, checks::level3());
        let hull_layer = s.layers.iter().position(|l| l.name == "Hull").unwrap();
        let x0 = foil.root_le_x - foil.root_chord - 0.05;
        let x1 = foil.root_le_x + 0.05;
        let y1 = 0.6 * foil.thickness * foil.root_chord + 0.02;
        let near = |li: usize, zmin: f64| -> Vec<[Vec3; 3]> {
            m.faces
                .iter()
                .zip(&m.face_layer)
                .filter(|(_, &l)| l == li)
                .flat_map(|(f, _)| (1..f.len() - 1).map(move |k| [f[0], f[k], f[k + 1]]))
                .map(|t| [m.verts[t[0]], m.verts[t[1]], m.verts[t[2]]])
                .filter(|t| {
                    t.iter().any(|p| p.z > zmin)
                        && t.iter().any(|p| p.x > x0)
                        && t.iter().any(|p| p.x < x1)
                        && t.iter().any(|p| p.y < y1)
                })
                .collect()
        };
        let foil_tris = near(layer, foil.root_z - 0.3);
        let hull_tris = near(hull_layer, 0.0);
        // z where the vertical line through (x, y) crosses each triangle.
        let hits = |tris: &[[Vec3; 3]], x: f64, y: f64| -> Vec<f64> {
            tris.iter()
                .filter_map(|[a, b, c]| {
                    let d = (b.x - a.x) * (c.y - a.y) - (c.x - a.x) * (b.y - a.y);
                    if d.abs() < 1e-14 {
                        return None;
                    }
                    let u = ((x - a.x) * (c.y - a.y) - (c.x - a.x) * (y - a.y)) / d;
                    let v = ((b.x - a.x) * (y - a.y) - (x - a.x) * (b.y - a.y)) / d;
                    (u >= 0.0 && v >= 0.0 && u + v <= 1.0)
                        .then_some(a.z + u * (b.z - a.z) + v * (c.z - a.z))
                })
                .collect()
        };
        let (nx, ny) = (160, 40);
        let (dx, dy) = ((x1 - x0) / nx as f64, y1 / ny as f64);
        let mut overlap = 0.0;
        for i in 0..nx {
            for j in 0..ny {
                let (x, y) = (x0 + (i as f64 + 0.5) * dx, (j as f64 + 0.5) * dy);
                let f = hits(&foil_tris, x, y);
                let Some(top) = f.iter().copied().reduce(f64::max) else {
                    continue;
                };
                let bottom = hits(&hull_tris, x, y)
                    .into_iter()
                    .reduce(f64::min)
                    .expect("the canoe body is above the foil");
                overlap += (top - bottom).max(0.0) * dx * dy;
            }
        }
        // The root cap (the foil's upward-facing triangles): each of its
        // points is inside the canoe body, above the bottom.
        let mut min_clearance = f64::INFINITY;
        for [a, b, c] in &foil_tris {
            if (*b - *a).cross(*c - *a).norm().z > 0.5 {
                for p in [a, b, c] {
                    let bottom = hits(&hull_tris, p.x, p.y.max(1e-9))
                        .into_iter()
                        .reduce(f64::min)
                        .expect("the canoe body is above the root");
                    min_clearance = min_clearance.min(p.z - bottom);
                }
            }
        }
        (2.0 * overlap, min_clearance)
    }

    #[test]
    fn yacht_appendages_barely_overlap_the_canoe_body() {
        let (s, _) = build();
        let h = checks::measure(&s);
        let mut total = 0.0;
        for (name, foil, layer) in [("fin", &FIN, 2), ("rudder", &RUDDER, 3)] {
            let (overlap, clearance) = root_overlap(&s, foil, layer);
            println!(
                "yacht {name}: overlap {overlap:.5} m³ ({:.3} % of ∇), root above the bottom by ≥ {clearance:.4} m",
                100.0 * overlap / h.volume_m3
            );
            assert!(clearance > 0.0, "{name}: a gap between root and canoe body");
            total += overlap;
        }
        assert!(
            total < 0.005 * h.volume_m3,
            "appendage overlap {total} m³ is not under 0.5 % of ∇ {}",
            h.volume_m3
        );
    }

    #[test]
    fn yacht_layers_and_creases() {
        let (s, _) = build();
        let names: Vec<&str> = s.layers.iter().map(|l| l.name.as_str()).collect();
        assert_eq!(names, ["Hull", "Deck", "Fin keel", "Rudder"]);
        assert!(s
            .layers
            .iter()
            .all(|l| l.symmetric && l.use_in_hydrostatics));
        // Every layer is its own closed half shell: no edge is shared
        // between layers, so any of them can leave the hydrostatics.
        let counts = s.edge_face_counts();
        for (e, &c) in s.edges.iter().zip(&counts) {
            if c == 1 {
                let (a, b) = (s.points[e.a].p, s.points[e.b].p);
                assert!(a.y == 0.0 && b.y == 0.0, "open edge off the centreline");
            }
        }
        // Sheer (one edge per station), transom outline (seven stern points
        // and the deck centreline), two tip and two root outlines, on top of
        // the boundary edges the trace initialisation creases.
        let creased = s.edges.iter().filter(|e| e.crease).count();
        assert!(creased >= STATIONS.len() + 8 + 4 * CHORD_ROWS.len());
    }

    #[test]
    fn yacht_is_closed_and_floats_level() {
        let (s, p) = build();
        checks::assert_closed_below(&s);
        checks::assert_closed_below(&canoe_body(&s));
        checks::assert_free_trim(&s, &p);
    }

    #[test]
    fn yacht_round_trips_and_its_sidecar_validates() {
        let (s, p) = build();
        checks::assert_fef_round_trip(&s);
        checks::assert_project_valid(&p);
        assert_eq!(p.windage.len(), 4);
        assert!(p.windage.iter().all(|w| w.name.contains("representative")));
        // The requirements' depth is the model's sheer amidships above the
        // keel tip, the draft's datum (R8-2 A06).
        let r = p.requirements.as_ref().unwrap();
        let mid = 0.5 * LWL;
        let w = STATIONS
            .windows(2)
            .find(|w| w[0].0 <= mid && mid <= w[1].0)
            .unwrap();
        let sheer = w[0].6 + (mid - w[0].0) / (w[1].0 - w[0].0) * (w[1].6 - w[0].6);
        let depth = r.depth.unwrap();
        assert!(
            (depth - sheer).abs() < 0.05,
            "depth {depth} vs sheer {sheer}"
        );
        assert!(depth > r.draft.unwrap());
    }
}
