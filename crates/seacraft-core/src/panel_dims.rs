//! Measurements drawn on a panel's pattern: its overall length and width, the offsets from its
//! baseline at the rows of its offsets table, and each seam's length along the flat edge.
//!
//! Every line here is a [`PathKind::Reference`] line, so a cutting machine never cuts one; the
//! numbers are the ones already measured ([`crate::panels::panel_offsets`],
//! [`crate::panels::SeamMeasure::length_flat_m`]), only written on the drawing.

use crate::panels::{extent, seam_spots, FlatPanel, PanelOffsets, TITLE_LABEL_MM};
use crate::pdf::Anchor;
use crate::sheet::{Label, PathKind, SheetPath};
use crate::units::{ft_in, ft_in_8ths, millimetres};

/// Letter height of the offsets and seam lengths on paper, millimetres.
pub const DIM_LABEL_MM: f64 = 3.0;
/// How far the overall dimension lines stand off the pattern's extent, metres.
const DIM_OFFSET_M: f64 = 0.04;
/// Half the length of a dimension line's end tick and of an offsets row's baseline tick, metres.
const DIM_TICK_M: f64 = 0.005;
/// The gap between a line and the text written along it, metres at full size.
const TEXT_GAP_M: f64 = 0.002;

/// The measurements on one pattern, kept apart so each kind can be checked.
#[derive(Debug, Clone, Default)]
pub(crate) struct Dims {
    pub paths: Vec<SheetPath>,
    /// Overall length, then overall width.
    pub size: Vec<Label>,
    /// One per ordinate, row by row, lowest first.
    pub offsets: Vec<Label>,
    /// One per seam label on the pattern.
    pub seams: Vec<Label>,
}

/// A length in whole millimetres, and with `imperial` feet and inches after it in brackets:
/// `4398 mm (14' 5 1/8")`.
fn length_text(m: f64, imperial: bool) -> String {
    if imperial {
        format!("{} mm ({})", millimetres(m), ft_in(m))
    } else {
        format!("{} mm", millimetres(m))
    }
}

/// An offset as the offsets tables write it: whole millimetres, or feet-inches-eighths.
fn ordinate_text(m: f64, imperial: bool) -> String {
    if imperial {
        ft_in_8ths(m)
    } else {
        millimetres(m)
    }
}

fn reference(points: Vec<(f64, f64)>) -> SheetPath {
    SheetPath {
        kind: PathKind::Reference,
        points,
        closed: false,
    }
}

fn text(at: (f64, f64), height_mm: f64, angle_deg: f64, anchor: Anchor, text: String) -> Label {
    Label {
        at,
        height_mm,
        angle_deg,
        anchor,
        text,
    }
}

/// The measurements of `panel` as reference lines and labels, to add to its pattern.
pub fn dimensions(
    panel: &FlatPanel,
    offsets: &PanelOffsets,
    imperial: bool,
) -> (Vec<SheetPath>, Vec<Label>) {
    let d = dims(panel, offsets, imperial);
    let labels = d.size.into_iter().chain(d.offsets).chain(d.seams).collect();
    (d.paths, labels)
}

pub(crate) fn dims(panel: &FlatPanel, offsets: &PanelOffsets, imperial: bool) -> Dims {
    let mut d = Dims::default();
    if panel.xy.is_empty() {
        return d;
    }
    let (x0, y0, length, width) = extent(panel);
    let (x1, y1) = (x0 + length, y0 + width);

    // Overall length, below the pattern: the line, its end ticks and extension lines up to the
    // pattern's extreme points.
    let yl = y0 - DIM_OFFSET_M;
    d.paths.push(reference(vec![(x0, yl), (x1, yl)]));
    for x in [x0, x1] {
        d.paths
            .push(reference(vec![(x, yl - DIM_TICK_M), (x, y0 - TEXT_GAP_M)]));
    }
    d.size.push(text(
        (x0 + length / 2.0, yl + TEXT_GAP_M),
        TITLE_LABEL_MM,
        0.0,
        Anchor::Middle,
        length_text(length, imperial),
    ));
    // Overall width, to the right of it; the text reads upward on the near side of the line.
    let xw = x1 + DIM_OFFSET_M;
    d.paths.push(reference(vec![(xw, y0), (xw, y1)]));
    for y in [y0, y1] {
        d.paths
            .push(reference(vec![(x1 + TEXT_GAP_M, y), (xw + DIM_TICK_M, y)]));
    }
    d.size.push(text(
        (xw - TEXT_GAP_M, y0 + width / 2.0),
        TITLE_LABEL_MM,
        90.0,
        Anchor::Middle,
        length_text(width, imperial),
    ));

    // The offsets: a tick on the baseline at each row, and each crossing's ordinate written
    // just inside the outline, reading up from a crossing the outline is entered at (the even
    // ones, counting up from the baseline) and down to one it is left at.
    for row in &offsets.rows {
        let x = x0 + row.along_m;
        d.paths
            .push(reference(vec![(x, y0 - DIM_TICK_M), (x, y0 + DIM_TICK_M)]));
        for (k, &o) in row.ordinates_m.iter().enumerate() {
            let entering = k % 2 == 0;
            let (dy, anchor) = if entering {
                (TEXT_GAP_M, Anchor::Start)
            } else {
                (-TEXT_GAP_M, Anchor::End)
            };
            d.offsets.push(text(
                (x + DIM_LABEL_MM / 2000.0, y0 + o + dy),
                DIM_LABEL_MM,
                90.0,
                anchor,
                ordinate_text(o, imperial),
            ));
        }
    }

    // Each seam's flat length, a line further inside than what the edge is stitched to.
    let below = (DIM_LABEL_MM + 1.5) / 1000.0;
    for spot in seam_spots(panel) {
        let Some(m) = panel.measures.seams.get(spot.seam) else {
            continue;
        };
        d.seams.push(text(
            (
                spot.at.0 + spot.inward.0 * below,
                spot.at.1 + spot.inward.1 * below,
            ),
            DIM_LABEL_MM,
            spot.angle_deg,
            Anchor::Middle,
            length_text(m.length_flat_m, imperial),
        ));
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{build, Example};
    use crate::math::Vec3;
    use crate::panels::{
        measure_panels, panel_offsets, Copies, FlatPanel, FoundPanel, Limits, Measures,
    };
    use crate::sheet::PathKind;
    use crate::subdiv::SubdivisionOptions;
    use crate::units::millimetres;
    use std::collections::HashMap;

    fn flat_panels(s: &crate::Surface) -> Vec<FlatPanel> {
        let layers = crate::panels::default_layers(s);
        let (panels, _, refused) = measure_panels(
            s,
            SubdivisionOptions::default(),
            &layers,
            &[],
            Limits::default(),
        );
        assert!(refused.is_empty(), "{refused:?}");
        panels
    }

    fn named<'a>(ps: &'a [FlatPanel], name: &str) -> &'a FlatPanel {
        ps.iter().find(|p| p.found.name == name).expect(name)
    }

    /// A "C" opening to +x, 3 m by 3 m with 1 m arms: the square line at x = 2 m crosses it four
    /// times, at y = 0, 1, 2 and 3 m. Built by hand, so the numbers are by construction.
    fn c_shape() -> FlatPanel {
        let xy = vec![
            (0.0, 0.0),
            (1.0, 0.0),
            (3.0, 0.0),
            (3.0, 1.0),
            (1.0, 1.0),
            (1.0, 2.0),
            (3.0, 2.0),
            (3.0, 3.0),
            (1.0, 3.0),
            (0.0, 3.0),
        ];
        let tris = vec![
            [1, 2, 3],
            [1, 3, 4],
            [0, 1, 4],
            [0, 4, 5],
            [0, 5, 9],
            [5, 8, 9],
            [5, 6, 7],
            [5, 7, 8],
        ];
        let verts: Vec<Vec3> = xy.iter().map(|&(x, y)| Vec3::new(x, 0.0, y)).collect();
        FlatPanel {
            found: FoundPanel {
                name: "C".into(),
                layer: 0,
                copies: Copies::One,
                mesh_ids: (0..verts.len()).collect(),
                verts,
                tris,
                welded: HashMap::new(),
                mirror: HashMap::new(),
                centreline: Vec::new(),
                seams: Vec::new(),
            },
            xy,
            measures: Measures {
                seams: Vec::new(),
                rms_stretch: 0.0,
                max_stretch: 0.0,
                level_shift_m: None,
                edge_join_gap_m: 0.0,
                area_3d_m2: 7.0,
                area_flat_m2: 7.0,
                unsettled_m: None,
            },
            lies_flat: true,
        }
    }

    fn bbox(p: &FlatPanel) -> [f64; 4] {
        p.xy.iter()
            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, q| {
                [b[0].min(q.0), b[1].min(q.1), b[2].max(q.0), b[3].max(q.1)]
            })
    }

    #[test]
    fn the_overall_size_is_the_extent() {
        let s = crate::panels::pram_fixture(0.0, 0.0);
        let ps = flat_panels(&s);
        for p in [named(&ps, "Side"), &c_shape()] {
            let b = bbox(p);
            let d = dims(p, &panel_offsets(p, 0.25), false);
            let texts: Vec<&str> = d.size.iter().map(|l| l.text.as_str()).collect();
            assert_eq!(
                texts,
                [
                    format!("{} mm", millimetres(b[2] - b[0])),
                    format!("{} mm", millimetres(b[3] - b[1]))
                ],
                "{}",
                p.found.name
            );
            for l in &d.size {
                let (x, y) = l.at;
                assert!(
                    x < b[0] || x > b[2] || y < b[1] || y > b[3],
                    "{}: {} inside the outline's box at {:?}",
                    p.found.name,
                    l.text,
                    l.at
                );
            }
        }
    }

    #[test]
    fn offsets_are_written_at_every_crossing() {
        let s = crate::panels::pram_fixture(0.0, 0.0);
        for p in &flat_panels(&s) {
            let offsets = panel_offsets(p, 0.25);
            let d = dims(p, &offsets, false);
            let want: Vec<String> = offsets
                .rows
                .iter()
                .flat_map(|r| r.ordinates_m.iter().map(|&o| millimetres(o)))
                .collect();
            let got: Vec<&str> = d.offsets.iter().map(|l| l.text.as_str()).collect();
            assert_eq!(got, want, "{}", p.found.name);
        }
    }

    #[test]
    fn a_notched_outline_writes_every_ordinate() {
        let p = c_shape();
        let offsets = panel_offsets(&p, 1.0);
        let row = offsets
            .rows
            .iter()
            .find(|r| (r.along_m - 2.0).abs() < 1e-9)
            .expect("a row at 2 m");
        assert_eq!(row.ordinates_m.len(), 4, "{row:?}");
        let d = dims(&p, &offsets, false);
        let at_two: Vec<&str> = d
            .offsets
            .iter()
            .filter(|l| (l.at.0 - 2.0).abs() < 0.01)
            .map(|l| l.text.as_str())
            .collect();
        assert_eq!(at_two, ["0", "1000", "2000", "3000"]);
    }

    #[test]
    fn seam_lengths_are_labelled() {
        let (s, _) = build(Example::Skiff4m).expect("skiff");
        let ps = flat_panels(&s);
        let side = named(&ps, "Side");
        let d = dims(side, &panel_offsets(side, 0.25), false);
        let texts: Vec<&str> = d.seams.iter().map(|l| l.text.as_str()).collect();
        assert!(!side.measures.seams.is_empty());
        for m in &side.measures.seams {
            let want = format!("{} mm", millimetres(m.length_flat_m));
            assert!(
                texts.contains(&want.as_str()),
                "{}: {want} not in {texts:?}",
                m.name
            );
        }
    }

    #[test]
    fn measurements_are_never_cut_lines() {
        let (s, _) = build(Example::Skiff4m).expect("skiff");
        for p in &flat_panels(&s) {
            let (paths, labels) = dimensions(p, &panel_offsets(p, 0.25), false);
            assert!(!paths.is_empty() && !labels.is_empty());
            assert!(paths.iter().all(|q| q.kind == PathKind::Reference));
        }
    }

    #[test]
    fn imperial_adds_feet_and_inches() {
        let p = c_shape();
        let d = dims(&p, &panel_offsets(&p, 1.0), true);
        assert_eq!(d.size[0].text, "3000 mm (9' 10 1/8\")");
    }
}
