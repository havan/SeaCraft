//! Table of offsets: half-breadths at stations × waterlines, plus buttock
//! heights at stations × buttocks.
//!
//! This is the drawing-office table a yard builds from: rows are stations along
//! the hull, columns are waterlines, and each cell is the half-breadth `y` where
//! the station section meets that waterline. A second table gives the height of
//! the hull shell on each buttock plane.
//!
//! Datum (see `skills/seacraft-hull-numerics/SKILL.md`): `x` forward, `y` starboard, `z` up, metres. Both
//! tables are read from the **refined** mesh, upright and at even keel:
//!
//! * waterline ordinates are heights **above the keel** (the baseline every
//!   draft and `KB`/`KM` in this crate is measured from);
//! * buttock ordinates are distances **from the centreline to starboard**.
//!
//! A cell is `None` when the station section does not reach that waterline
//! (the section is above it) or that buttock (the hull is narrower there) —
//! a printed table shows that as `-`, which is not the same as 0.000 on the
//! centreline. Treating "not reached" as zero is how a table silently acquires
//! a false knuckle.
//!
//! The section polylines come from [`crate::curves::station_polygons`], i.e. the
//! same cut, chaining and deck closing the Bonjean curves use, so the two
//! tables can never disagree about where the hull surface is.

use crate::curves::station_polygons;
use crate::hydro::Hull;
use crate::math::Vec3;
use serde::Serialize;
use std::fmt::Write as _;

/// One station of the table.
#[derive(Debug, Clone, Serialize)]
pub struct OffsetStation {
    /// Station position, model `x` in metres.
    pub x: f64,
    /// Lowest point of the section, model `z`.
    pub keel_z: f64,
    /// Sheer (deck edge) height of the section, model `z`.
    pub deck_z: f64,
    /// Largest half-breadth on this station, metres.
    pub half_beam: f64,
    /// Half-breadths at [`TableOfOffsets::waterlines_m`], in order.
    pub waterline_half_breadths: Vec<Option<f64>>,
    /// Lowest hull height (model `z`) at [`TableOfOffsets::buttocks_m`], in
    /// order: the buttock curve of the profile view.
    pub buttock_heights: Vec<Option<f64>>,
}

/// The two tables, with their ordinate lists.
#[derive(Debug, Clone, Serialize)]
pub struct TableOfOffsets {
    /// Keel height in model `z` (the datum the waterline ordinates rise from).
    pub keel_z: f64,
    /// Waterline heights above the keel, metres, ascending.
    pub waterlines_m: Vec<f64>,
    /// Buttock offsets from the centreline, metres, ascending.
    pub buttocks_m: Vec<f64>,
    pub stations: Vec<OffsetStation>,
}

impl TableOfOffsets {
    /// Widest half-breadth anywhere in the table.
    pub fn max_half_breadth(&self) -> f64 {
        self.stations
            .iter()
            .fold(0.0_f64, |m, s| m.max(s.half_beam))
    }
}

/// The edges of a section polyline **including the closing one**.
///
/// `chain_paths` drops the repeated closing vertex and reports the path as
/// closed instead, so a plain `windows(2)` walk silently omits the last edge.
/// On a box that edge is the flat bottom, and the buttock lookup happily
/// returned the deck height for every offset — the section looked bottomless.
/// A path closed by `close_along_waterplane` repeats its first point, so the
/// extra wrap edge is zero length and contributes no crossing.
fn edges(poly: &[Vec3]) -> impl Iterator<Item = (Vec3, Vec3)> + '_ {
    let n = poly.len();
    (0..n).filter_map(move |i| {
        if n < 2 {
            return None;
        }
        Some((poly[i], poly[(i + 1) % n]))
    })
}

/// Largest `y` where the (closed) section polylines cross height `z`.
///
/// The polylines run from the deck down around the keel and back up, and are
/// closed with a chord across the deck, so the starboard crossing is the
/// maximum `y` of the crossings at that height.
fn half_breadth_at(polys: &[Vec<Vec3>], z: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for poly in polys {
        for edge in edges(poly) {
            let (a, b) = edge;
            let (za, zb) = (a.z - z, b.z - z);
            // Count a crossing when the edge strictly straddles the height; an
            // edge lying in the plane contributes nothing (it is the deck).
            if (za < 0.0 && zb >= 0.0) || (za >= 0.0 && zb < 0.0) {
                let t = za / (za - zb);
                let y = a.y + (b.y - a.y) * t;
                if best.is_none_or(|m| y > m) {
                    best = Some(y);
                }
            }
        }
    }
    best.map(|y| y.max(0.0)).filter(|y| y.is_finite())
}

/// Lowest `z` where the section polylines cross the buttock plane `y`.
///
/// That is the buttock curve the profile view draws: the hull's bottom at that
/// offset. A section crosses a buttock twice (bottom and deck edge); the lower
/// crossing is the one a builder lays off, so it is the one reported. `None`
/// when the station is narrower than the buttock.
fn buttock_height_at(polys: &[Vec<Vec3>], y: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for poly in polys {
        for edge in edges(poly) {
            let (a, b) = edge;
            let (ya, yb) = (a.y - y, b.y - y);
            if (ya < 0.0 && yb >= 0.0) || (ya >= 0.0 && yb < 0.0) {
                let t = ya / (ya - yb);
                let z = a.z + (b.z - a.z) * t;
                if best.is_none_or(|m| z < m) {
                    best = Some(z);
                }
            }
        }
    }
    best.filter(|z| z.is_finite())
}

/// Default waterline ordinates: `count` heights from just above the keel to the
/// highest sheer, measured above the keel.
pub fn default_waterlines(hull: &Hull, count: usize) -> Vec<f64> {
    let n = count.max(2);
    let depth = (hull.model_max.z - hull.keel).max(0.0);
    (1..=n).map(|i| depth * i as f64 / n as f64).collect()
}

/// Default buttock ordinates: `count` offsets spread over the model's widest
/// half-breadth.
pub fn default_buttocks(hull: &Hull, count: usize) -> Vec<f64> {
    let n = count.max(2);
    let half = hull.verts.iter().map(|v| v.y.abs()).fold(0.0_f64, f64::max);
    (1..=n).map(|i| half * i as f64 / n as f64).collect()
}

/// Build the table for `stations` stations (midpoint sampled, see
/// [`crate::curves::station_xs`]) at the given waterline and buttock ordinates.
///
/// `waterlines_m` are heights above the keel and `buttocks_m` are starboard
/// offsets, both in metres; they are used in the order given, so sort them
/// before calling if the table should read bottom-up and centreline-out.
pub fn table_of_offsets(
    hull: &Hull,
    stations: usize,
    waterlines_m: &[f64],
    buttocks_m: &[f64],
) -> TableOfOffsets {
    let xs = crate::curves::station_xs(hull, stations);
    let mut rows = Vec::with_capacity(xs.len());
    for x in xs {
        let (_, polys) = station_polygons(hull, x);
        let keel_z = polys
            .iter()
            .flat_map(|p| p.iter())
            .fold(f64::INFINITY, |m, v| m.min(v.z));
        let deck_z = polys
            .iter()
            .flat_map(|p| p.iter())
            .fold(f64::NEG_INFINITY, |m, v| m.max(v.z));
        let half_beam = polys
            .iter()
            .flat_map(|p| p.iter())
            .fold(0.0_f64, |m, v| m.max(v.y));
        let waterline_half_breadths = waterlines_m
            .iter()
            .map(|wl| half_breadth_at(&polys, hull.keel + wl))
            .collect();
        let buttock_heights = buttocks_m
            .iter()
            .map(|b| buttock_height_at(&polys, *b))
            .collect();
        rows.push(OffsetStation {
            x,
            keel_z: if keel_z.is_finite() {
                keel_z
            } else {
                hull.keel
            },
            deck_z: if deck_z.is_finite() {
                deck_z
            } else {
                hull.keel
            },
            half_beam: half_beam.max(0.0),
            waterline_half_breadths,
            buttock_heights,
        });
    }
    TableOfOffsets {
        keel_z: hull.keel,
        waterlines_m: waterlines_m.to_vec(),
        buttocks_m: buttocks_m.to_vec(),
        stations: rows,
    }
}

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn cell(v: Option<f64>) -> String {
    match v {
        Some(x) => format!("{x:.3}"),
        None => "-".to_string(),
    }
}

/// The table as a standalone HTML page: both tables, station keel and sheer
/// heights, and the ordinate lists the cells belong to.
pub fn html(table: &TableOfOffsets, title: &str) -> String {
    let mut out = String::with_capacity(4096);
    let _ = write!(
        out,
        "<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<title>{}</title>\n",
        esc(title)
    );
    out.push_str(
        "<style>\n\
         body{font:13px/1.45 ui-sans-serif,system-ui,sans-serif;color:#1b1b1b;margin:28px;background:#fff}\n\
         h1{font-size:17px;margin:0 0 4px}h2{font-size:14px;margin:26px 0 8px}\n\
         .note{color:#555;margin:0 0 16px}\n\
         table{border-collapse:collapse;margin:0 0 8px}\n\
         th,td{border:1px solid #c8c8c8;padding:3px 8px;text-align:right;font-variant-numeric:tabular-nums}\n\
         th{background:#f2f5f8;font-weight:600}\n\
         td.x,th.x{background:#fafafa;font-weight:600;text-align:left}\n\
         tfoot td{border:none;color:#555;padding-top:6px}\n\
         </style>\n</head>\n<body>\n",
    );
    let _ = writeln!(out, "<h1>{}</h1>", esc(title));
    let _ = writeln!(
        out,
        "<p class=\"note\">Metres. Stations on x (forward), half-breadths to starboard from the centreline, \
         waterlines measured above the keel ({:.3} m in model z). &ldquo;-&rdquo; means the section does not \
         reach that ordinate.</p>",
        table.keel_z
    );

    // ---- waterline table ------------------------------------------------
    out.push_str(
        "<h2>Half-breadths at waterlines</h2>\n<table>\n<thead><tr><th class=\"x\">x [m]</th>",
    );
    for wl in &table.waterlines_m {
        let _ = write!(out, "<th>WL {wl:.2}</th>");
    }
    out.push_str("<th>keel z</th><th>sheer z</th><th>B/2</th></tr></thead>\n<tbody>\n");
    for s in &table.stations {
        let _ = write!(out, "<tr><td class=\"x\">{:.3}</td>", s.x);
        for v in &s.waterline_half_breadths {
            let _ = write!(out, "<td>{}</td>", cell(*v));
        }
        let _ = writeln!(
            out,
            "<td>{:.3}</td><td>{:.3}</td><td>{:.3}</td></tr>",
            s.keel_z, s.deck_z, s.half_beam
        );
    }
    out.push_str("</tbody>\n</table>\n");

    // ---- buttock table --------------------------------------------------
    out.push_str("<h2>Heights at buttocks</h2>\n<table>\n<thead><tr><th class=\"x\">x [m]</th>");
    for b in &table.buttocks_m {
        let _ = write!(out, "<th>B {b:.2}</th>");
    }
    out.push_str("</tr></thead>\n<tbody>\n");
    for s in &table.stations {
        let _ = write!(out, "<tr><td class=\"x\">{:.3}</td>", s.x);
        for v in &s.buttock_heights {
            let _ = write!(out, "<td>{}</td>", cell(*v));
        }
        out.push_str("</tr>\n");
    }
    out.push_str("</tbody>\n</table>\n<p class=\"note\">Buttock heights are the lowest model z at which \
                  the hull reaches the offset in the column (the buttock curve drawn in the profile \
                  view).</p>\n</body>\n</html>\n");
    out
}

/// The table as a printable PDF: half-breadths, then heights on buttocks, in millimetres, and
/// with `imperial` the same two again in feet, inches and eighths. Heights are above the keel
/// datum, [`TableOfOffsets::keel_z`].
pub fn pdf(
    table: &TableOfOffsets,
    title: &str,
    imperial: bool,
) -> Result<crate::pdf::Document, String> {
    use crate::pdf::{add_table, Pdf, Table};
    use crate::units::{ft_in_8ths, millimetres};
    if table.stations.is_empty() {
        return Err("the table of offsets has no stations: nothing to tabulate".into());
    }
    let subtitle = "Heights are above the baseline (keel). \"-\" means the section does not reach.";
    let mut doc = Pdf::new(title);
    // The unit in the titles, the x column's heading, and how a length and an x are written.
    type Write = fn(f64) -> String;
    let mut systems: Vec<(&str, &str, Write, Write)> =
        vec![("millimetres", "x [m]", millimetres, |x| format!("{x:.3}"))];
    if imperial {
        systems.push(("feet-inches-eighths", "x [ft-in-8]", ft_in_8ths, ft_in_8ths));
    }
    for (unit, x_head, length, x) in systems {
        let cell = |v: Option<f64>| v.map_or_else(|| "-".to_owned(), length);
        let height = |z: f64| length(z - table.keel_z);
        let station = |i: usize| (i + 1).to_string();

        let mut header = vec!["Station".to_owned(), x_head.to_owned()];
        header.extend(
            table
                .waterlines_m
                .iter()
                .map(|h| format!("WL {}", length(*h))),
        );
        header.extend(["Keel", "Sheer", "B/2"].map(String::from));
        let rows: Vec<Vec<String>> = table
            .stations
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut row = vec![station(i), x(s.x)];
                row.extend(s.waterline_half_breadths.iter().map(|v| cell(*v)));
                row.extend([height(s.keel_z), height(s.deck_z), length(s.half_beam)]);
                row
            })
            .collect();
        let half_breadths = format!("{title}: half-breadths, {unit}");
        add_table(
            &mut doc,
            &Table {
                title: &half_breadths,
                subtitle,
                header: &header,
                rows: &rows,
                footer: crate::NOT_VALIDATED_NOTE,
                repeat: 2,
            },
        );

        let mut header = vec!["Station".to_owned(), x_head.to_owned()];
        header.extend(table.buttocks_m.iter().map(|b| format!("B {}", length(*b))));
        let rows: Vec<Vec<String>> = table
            .stations
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let mut row = vec![station(i), x(s.x)];
                row.extend(
                    s.buttock_heights
                        .iter()
                        .map(|v| cell(v.map(|z| z - table.keel_z))),
                );
                row
            })
            .collect();
        let buttocks = format!("{title}: heights on buttocks, {unit}");
        add_table(
            &mut doc,
            &Table {
                title: &buttocks,
                subtitle,
                header: &header,
                rows: &rows,
                footer: crate::NOT_VALIDATED_NOTE,
                repeat: 2,
            },
        );
    }
    doc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::HydroOptions;
    use crate::mesh::VertexKind;
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};
    use crate::Surface;

    fn barge(length: f64, beam: f64, depth: f64) -> Surface {
        let mut s = Surface::new();
        s.layers = vec![crate::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..crate::Layer::default()
        }];
        s.settings = crate::ProjectSettings {
            name: "barge".into(),
            length,
            beam,
            draft: depth,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..crate::ProjectSettings::default()
        };
        let (l, b, d) = (length, beam, depth);
        let corners = [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ];
        for (x, y, z) in corners {
            s.add_point(Vec3::new(x, y - b * 0.5, z));
        }
        for face in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            s.add_face(face, 0);
        }
        s.initialize_trace();
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = VertexKind::Corner;
        }
        s
    }

    #[test]
    fn box_offsets_are_the_box() {
        let s = barge(10.0, 4.0, 3.0);
        let hull = Hull::new(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        let table = table_of_offsets(&hull, 9, &[0.5, 1.5, 2.5, 3.5], &[0.5, 1.9, 2.5]);
        assert_eq!(table.stations.len(), 9);
        for st in &table.stations {
            assert!(
                (st.half_beam - 2.0).abs() < 1e-9,
                "half beam {}",
                st.half_beam
            );
            assert!((st.keel_z - 0.0).abs() < 1e-9);
            assert!((st.deck_z - 3.0).abs() < 1e-9);
            // Inside the box the side is vertical, so every waterline below the
            // deck is the same half-breadth; above it the cell is empty.
            for (i, v) in st.waterline_half_breadths.iter().enumerate() {
                if i < 3 {
                    assert!(
                        (v.expect("a crossing below the deck") - 2.0).abs() < 1e-9,
                        "waterline {}: {v:?}",
                        i
                    );
                } else {
                    assert_eq!(*v, None, "waterline above the deck must be empty");
                }
            }
            // A box has a flat bottom: every buttock inside the half beam reads
            // z = 0 (the buttock curve lies on the keel), the wider one is empty.
            assert!(
                st.buttock_heights[0].unwrap().abs() < 1e-9,
                "{:?}",
                st.buttock_heights
            );
            assert!(
                st.buttock_heights[1].unwrap().abs() < 1e-9,
                "{:?}",
                st.buttock_heights
            );
            assert_eq!(st.buttock_heights[2], None);
        }
    }

    #[test]
    fn demo_offsets_match_the_hydrostatic_beam() {
        let s = crate::demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
        );
        let hull = Hull::new(&s, opts.subdivision);
        let hydro = crate::hydro::compute(&s, &opts);
        let wls = default_waterlines(&hull, 10);
        let table = table_of_offsets(&hull, 21, &wls, &default_buttocks(&hull, 4));

        // The section maximum is the model's half beam at the sheer (2.0 m),
        // which is wider than the waterline beam: the topsides flare.
        assert!(
            (table.max_half_breadth() - 2.0).abs() < 0.02,
            "widest half-breadth {}",
            table.max_half_breadth()
        );

        // A waterline at the design draft reproduces half the measured BWL.
        let at_design = table_of_offsets(&hull, 41, &[opts.draft], &[]);
        let widest = at_design
            .stations
            .iter()
            .filter_map(|s| s.waterline_half_breadths[0])
            .fold(0.0_f64, f64::max);
        let want = hydro.bwl * 0.5;
        assert!(
            (widest - want).abs() / want < 0.01,
            "half-breadth at T = {:.2} was {widest:.4}, BWL/2 is {want:.4}",
            opts.draft
        );

        // Nothing may exceed the hull's own beam.
        for st in &table.stations {
            assert!(st.half_beam <= 2.02, "station {} too wide", st.x);
            for v in st.waterline_half_breadths.iter().flatten() {
                assert!(*v >= -1e-9 && *v <= 2.02, "offset {v} at x {}", st.x);
            }
        }
        let mid = &table.stations[10];
        assert!(mid.half_beam > 1.5, "midship half beam {}", mid.half_beam);
        // A deep waterline is reached near midships and not at the ends. (The
        // topmost default waterline sits at the highest sheer, a single point
        // at the stem, so it is not a useful check.)
        let deep: Vec<bool> = table
            .stations
            .iter()
            .map(|s| s.waterline_half_breadths[table.waterlines_m.len() - 2].is_some())
            .collect();
        assert!(
            deep.iter().any(|x| *x),
            "a deep waterline is reached somewhere"
        );
        assert!(
            deep.iter().any(|x| !*x),
            "a deep waterline is not reached at the ends"
        );
    }

    #[test]
    fn html_table_is_well_formed() {
        let s = barge(10.0, 4.0, 3.0);
        let hull = Hull::new(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        // The second waterline is above the deck, so half its cells are empty.
        let table = table_of_offsets(&hull, 5, &[1.0, 4.0], &[1.0]);
        let page = html(&table, "barge <offsets>");
        assert!(page.starts_with("<!DOCTYPE html>"));
        assert!(page.trim_end().ends_with("</html>"));
        assert_eq!(
            page.matches("<tr>").count(),
            2 + 5 + 5,
            "two headers, ten rows"
        );
        assert!(
            page.contains("barge &lt;offsets&gt;"),
            "the title is escaped"
        );
        assert!(
            !page.contains("NaN") && !page.contains("inf"),
            "no invalid numbers"
        );
        // An empty cell is a dash, not a zero.
        assert!(page.contains("<td>-</td>"));
    }

    fn box_table(stations: usize, waterlines: &[f64], buttocks: &[f64]) -> TableOfOffsets {
        let s = barge(10.0, 4.0, 2.0);
        let hull = Hull::new(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        table_of_offsets(&hull, stations, waterlines, buttocks)
    }

    const SUBTITLE: &str =
        "Heights are above the baseline (keel). \"-\" means the section does not reach.";

    /// Closed form: the box is 4 m wide, so every half-breadth below the deck is 2000 mm.
    #[test]
    fn a_box_table_reads_in_millimetres() {
        let doc = pdf(&box_table(5, &[1.0], &[1.0]), "Box", false).unwrap();
        assert_eq!(doc.pages, 2);
        let text = crate::pdf::shown_text(&doc.bytes);
        for want in [
            "2000",
            "Box: half-breadths, millimetres",
            "Box: heights on buttocks, millimetres",
            SUBTITLE,
            crate::NOT_VALIDATED_NOTE,
            "WL 1000",
            "B 1000",
        ] {
            assert!(text.iter().any(|t| t == want), "{want} in {text:?}");
        }
        assert!(doc.missing.is_empty(), "{:?}", doc.missing);
    }

    #[test]
    fn a_cell_the_section_does_not_reach_is_a_dash() {
        // The box is 2 m deep: a waterline at 3 m is above every section.
        let doc = pdf(&box_table(5, &[3.0], &[1.0]), "Box", false).unwrap();
        let page = crate::pdf::page_text(&doc.bytes, 0);
        // Each row reads station, x, the waterline's cell, keel, sheer, half-breadth.
        let row = page.iter().position(|t| t == "1").unwrap();
        assert_eq!(page[row + 2], "-", "{page:?}");
        assert_eq!(&page[row + 3..row + 6], ["0", "2000", "2000"], "{page:?}");
    }

    /// Closed form: 2.0 m = 78.740 in = 1260/16 in = 6 ft 6 12/16 in, so `6-6-6`.
    #[test]
    fn imperial_adds_its_tables_after_the_metric_ones() {
        let table = box_table(5, &[1.0], &[1.0]);
        let metric = pdf(&table, "Box", false).unwrap();
        let both = pdf(&table, "Box", true).unwrap();
        assert_eq!(both.pages, 4);
        for page in 0..2 {
            assert_eq!(
                crate::pdf::page_stream(&both.bytes, page),
                crate::pdf::page_stream(&metric.bytes, page),
                "page {page}"
            );
        }
        let third = crate::pdf::page_text(&both.bytes, 2);
        assert!(third.iter().any(|t| t == "6-6-6"), "{third:?}");
        assert!(
            third
                .iter()
                .any(|t| t == "Box: half-breadths, feet-inches-eighths"),
            "{third:?}"
        );
        let fourth = crate::pdf::page_text(&both.bytes, 3);
        assert!(
            fourth
                .iter()
                .any(|t| t == "Box: heights on buttocks, feet-inches-eighths"),
            "{fourth:?}"
        );
    }

    #[test]
    fn long_tables_continue_with_the_header_repeated() {
        // 40 stations are more rows than a page holds.
        let doc = pdf(&box_table(40, &[1.0], &[1.0]), "Box", false).unwrap();
        assert_eq!(doc.pages, 4, "two pages for each table");
        for page in 0..2 {
            let text = crate::pdf::page_text(&doc.bytes, page);
            assert!(text.iter().any(|t| t == "Station"), "page {page}");
        }
        // 20 waterlines are more columns than a page holds.
        let wls: Vec<f64> = (1..=20).map(|i| i as f64 * 0.1).collect();
        let doc = pdf(&box_table(5, &wls, &[1.0]), "Box", false).unwrap();
        assert_eq!(doc.pages, 3, "two pages of half-breadths, one of buttocks");
        for page in 0..2 {
            let text = crate::pdf::page_text(&doc.bytes, page);
            for head in ["Station", "x [m]"] {
                assert!(text.iter().any(|t| t == head), "{head} on page {page}");
            }
        }
        let second = crate::pdf::page_text(&doc.bytes, 1);
        assert!(second.iter().any(|t| t == "B/2"), "{second:?}");
    }

    #[test]
    fn an_empty_table_is_an_error() {
        let mut table = box_table(5, &[1.0], &[1.0]);
        table.stations.clear();
        let err = pdf(&table, "Box", false).unwrap_err();
        assert!(err.contains("nothing to tabulate"), "{err}");
    }
}
