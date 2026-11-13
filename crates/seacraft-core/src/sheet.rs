//! A drawing sheet: a 2D drawing in metres, and the writers that put it on paper or send it to a
//! machine.
//!
//! Everything a builder prints goes through [`Sheet`], so the tiled PDF, the single-page PDF, the
//! SVG and the DXF of one pattern are the same lines. x runs right and y up.
//!
//! Line widths and label heights are millimetres on paper: a drawing printed at 1:10 has the same
//! line width as one at 1:1.

use std::fmt::Write as _;

use crate::dxf::{write_dxf_in, DxfEntity, DxfLayer, DxfShape, DxfUnits, Vec2};
pub use crate::pdf::Anchor;
use crate::pdf::{
    text_width_pt, wrap_text, Content, Document, Pdf, Stroke, MAX_PAGE_PT, PT_PER_MM,
};
use crate::units::scale_label;

/// What a line on a sheet is for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PathKind {
    /// Saw here. Drawn 0.35 mm wide, black.
    Cut,
    /// Transfer to the wood: a centreline, a waterline, a station line. Drawn 0.18 mm wide, black.
    Mark,
    /// For the eye only, such as the hull surface outside a mould line. Drawn 0.18 mm wide, grey,
    /// dashed.
    Reference,
}

/// A line on a sheet, through `points` in metres.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetPath {
    /// What the line is for, which decides how it is drawn.
    pub kind: PathKind,
    /// The points, in order.
    pub points: Vec<(f64, f64)>,
    /// Whether the last point joins the first.
    pub closed: bool,
}

/// One line of text on a sheet.
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    /// Where its anchor is, in metres.
    pub at: (f64, f64),
    /// The letter size on paper, in millimetres; the scale does not change it.
    pub height_mm: f64,
    /// Turned anticlockwise by this much.
    pub angle_deg: f64,
    /// Which point of the text is at `at`.
    pub anchor: Anchor,
    /// The words.
    pub text: String,
}

/// A drawing: lines and labels in metres, and notes that go beside it on paper.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Sheet {
    /// What the sheet is, as its pages are captioned: `Station 3`.
    pub name: String,
    /// The lines.
    pub paths: Vec<SheetPath>,
    /// The text on the drawing.
    pub labels: Vec<Label>,
    /// What a builder must know about this sheet, one line each.
    pub notes: Vec<String>,
}

impl Sheet {
    /// An empty sheet.
    pub fn new(name: impl Into<String>) -> Self {
        Sheet {
            name: name.into(),
            ..Sheet::default()
        }
    }

    /// `[min_x, min_y, max_x, max_y]` over path points and label anchors, in metres. `None` when
    /// the sheet has neither. Points that are not finite numbers are left out.
    pub fn bounds(&self) -> Option<[f64; 4]> {
        let points = self.paths.iter().flat_map(|p| p.points.iter().copied());
        let anchors = self.labels.iter().map(|l| l.at);
        points
            .chain(anchors)
            .filter(|p| p.0.is_finite() && p.1.is_finite())
            .fold(None, |acc: Option<[f64; 4]>, (x, y)| {
                Some(acc.map_or([x, y, x, y], |b| {
                    [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
                }))
            })
    }
}

/// A paper size, in millimetres, as held upright.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Paper {
    /// The short side of the named sizes.
    pub width_mm: f64,
    /// The long side of the named sizes.
    pub height_mm: f64,
}

impl Paper {
    /// ISO A4, 210 x 297 mm.
    pub const A4: Paper = Paper {
        width_mm: 210.0,
        height_mm: 297.0,
    };
    /// ISO A3, 297 x 420 mm.
    pub const A3: Paper = Paper {
        width_mm: 297.0,
        height_mm: 420.0,
    };
    /// US Letter, 8.5 x 11 in.
    pub const LETTER: Paper = Paper {
        width_mm: 215.9,
        height_mm: 279.4,
    };

    /// Read `a4`, `a3`, `letter`, or a size in millimetres as `WxH` with each side from 50 to
    /// 5000.
    pub fn parse(text: &str) -> Result<Paper, String> {
        let name = text.trim().to_ascii_lowercase();
        match name.as_str() {
            "a4" => return Ok(Paper::A4),
            "a3" => return Ok(Paper::A3),
            "letter" => return Ok(Paper::LETTER),
            _ => {}
        }
        let side = |part: &str| {
            part.trim()
                .parse::<f64>()
                .ok()
                .filter(|v| (50.0..=5000.0).contains(v))
        };
        match name.split('x').collect::<Vec<_>>()[..] {
            [w, h] => match (side(w), side(h)) {
                (Some(width_mm), Some(height_mm)) => Ok(Paper { width_mm, height_mm }),
                _ => Err(format!(
                    "paper `{}`: each side must be a number of millimetres from 50 to 5000",
                    text.trim()
                )),
            },
            _ => Err(format!(
                "`{}` is not a paper size: use a4, a3, letter, or WxH in millimetres such as 610x914",
                text.trim()
            )),
        }
    }
}

/// How a document is laid out on paper.
#[derive(Clone, Debug, PartialEq)]
pub struct PageSetup {
    /// The paper a tiled document is printed on.
    pub paper: Paper,
    /// The strip around each page a printer cannot reach, in millimetres.
    pub margin_mm: f64,
    /// How much neighbouring tiles share, in millimetres.
    pub overlap_mm: f64,
    /// Paper size over real size: 1 is full size, 0.1 is 1:10.
    pub scale: f64,
    /// Add imperial readings beside the metric ones.
    pub imperial: bool,
    /// The design's name, for captions.
    pub title: String,
}

impl Default for PageSetup {
    fn default() -> Self {
        PageSetup {
            paper: Paper::A4,
            margin_mm: 10.0,
            overlap_mm: 10.0,
            scale: 1.0,
            imperial: false,
            title: String::new(),
        }
    }
}

impl PageSetup {
    /// Refuse a setup no page can be made from: a margin or overlap that is negative or not a
    /// number, a paper with less than 20 mm of fresh drawing per page, a scale outside 1:1000 to
    /// 10:1.
    pub fn check(&self) -> Result<(), String> {
        let sides = [self.paper.width_mm, self.paper.height_mm];
        if sides.iter().any(|s| !s.is_finite() || *s <= 0.0) {
            return Err("the paper has no size".into());
        }
        for (name, v) in [("margin", self.margin_mm), ("overlap", self.overlap_mm)] {
            if !v.is_finite() || v < 0.0 {
                return Err(format!("the {name} must be zero or more millimetres"));
            }
        }
        if sides
            .iter()
            .any(|s| s - 2.0 * self.margin_mm - self.overlap_mm < 20.0)
        {
            return Err(format!(
                "a {} x {} mm page with {} mm margins and {} mm of overlap leaves less than 20 mm \
                 of new drawing per page: use a smaller margin or overlap, or larger paper",
                self.paper.width_mm, self.paper.height_mm, self.margin_mm, self.overlap_mm
            ));
        }
        if !self.scale.is_finite() || !(0.001..=10.0).contains(&self.scale) {
            return Err("the scale must be between 1:1000 and 10:1".into());
        }
        Ok(())
    }

    /// Points on paper per metre of drawing.
    pub(crate) fn pt_per_m(&self) -> f64 {
        self.scale * 1000.0 * PT_PER_MM
    }
}

/// The three instructions every printed pattern carries beside its rules.
pub(crate) const CALIBRATION_LINES: [&str; 3] = [
    "Print at 100 % (actual size). Do not use \"fit to page\".",
    "Measure both rules. If either is not 100 mm, do not cut.",
    "Cut on the centre of the heavy line.",
];

/// The width of the column a single page keeps for the rules, in millimetres.
pub(crate) const CALIBRATION_COLUMN_MM: f64 = 120.0;
/// The least height of a single page inside its margins, so the rules always fit.
pub(crate) const SINGLE_PAGE_MIN_HEIGHT_MM: f64 = 150.0;
/// The size of the words beside the rules, in points.
pub(crate) const SMALL_PT: f64 = 7.0;
/// The line spacing of those words, in millimetres.
pub(crate) const SMALL_LINE_MM: f64 = 3.5;
/// How far above the rules' corner the words may start, in millimetres: the rules end below it.
pub(crate) const RULES_HEIGHT_MM: f64 = 118.0;

/// The gap between sheets laid side by side in an SVG or a DXF, in millimetres.
const GAP_MM: f64 = 50.0;
/// The margin around an SVG, in millimetres.
const SVG_MARGIN_MM: f64 = 10.0;
/// Notes under a sheet in an SVG or a DXF: letter height, line spacing, and the gap above the
/// first line, in millimetres.
const NOTE_HEIGHT_MM: f64 = 3.0;
const NOTE_LINE_MM: f64 = 5.0;
const NOTE_GAP_MM: f64 = 10.0;

/// How a kind of line is drawn.
pub(crate) fn stroke_for(kind: PathKind) -> Stroke {
    match kind {
        PathKind::Cut => Stroke {
            width_mm: 0.35,
            grey: 0.0,
            dash_mm: None,
        },
        PathKind::Mark => Stroke {
            width_mm: 0.18,
            grey: 0.0,
            dash_mm: None,
        },
        PathKind::Reference => Stroke {
            width_mm: 0.18,
            grey: 0.55,
            dash_mm: Some((2.0, 1.0)),
        },
    }
}

/// The order kinds are drawn in, so that the cut line lies on top.
const DRAW_ORDER: [PathKind; 3] = [PathKind::Reference, PathKind::Mark, PathKind::Cut];

/// The sheet's paths and labels in points, with `origin_m` at (0, 0). Every path and label is
/// written once.
pub(crate) fn draw(sheet: &Sheet, scale: f64, origin_m: (f64, f64)) -> Content {
    let k = scale * 1000.0 * PT_PER_MM;
    let to_pt = |p: (f64, f64)| ((p.0 - origin_m.0) * k, (p.1 - origin_m.1) * k);
    let mut c = Content::new();
    for kind in DRAW_ORDER {
        for path in sheet.paths.iter().filter(|p| p.kind == kind) {
            let points: Vec<(f64, f64)> = path.points.iter().map(|p| to_pt(*p)).collect();
            c.polyline(&points, path.closed, stroke_for(kind));
        }
    }
    for label in &sheet.labels {
        c.text(
            to_pt(label.at),
            label.height_mm * PT_PER_MM,
            label.angle_deg,
            label.anchor,
            0.0,
            &label.text,
        );
    }
    c
}

/// The two 100 mm rules (and the 4 in rules when `imperial`) with their captions, lower-left
/// corner at `at_pt`. They fit in 120 mm x 118 mm ([`RULES_HEIGHT_MM`]), and no rule crosses
/// another.
pub(crate) fn calibration_rules(content: &mut Content, at_pt: (f64, f64), imperial: bool) {
    let fine = Stroke {
        width_mm: 0.18,
        grey: 0.0,
        dash_mm: None,
    };
    // One rule along the page and one across it: `along` picks which. `offset_mm` is how far the
    // rule is from the corner's edge, and `start_mm` where along it the rule begins.
    let mut rule = |offset_mm: f64, start_mm: f64, length_mm: f64, step_mm: f64, caption: &str| {
        for along in [true, false] {
            let pt = |a_mm: f64, b_mm: f64| {
                let (x, y) = if along { (a_mm, b_mm) } else { (b_mm, a_mm) };
                (at_pt.0 + x * PT_PER_MM, at_pt.1 + y * PT_PER_MM)
            };
            let ends = [pt(start_mm, offset_mm), pt(start_mm + length_mm, offset_mm)];
            content.polyline(&ends, false, fine);
            let ticks = (length_mm / step_mm).round() as usize;
            for i in 0..=ticks {
                let tall = if i == 0 || i == ticks || 2 * i == ticks {
                    4.0
                } else {
                    2.0
                };
                let a = start_mm + i as f64 * step_mm;
                content.polyline(&[pt(a, offset_mm), pt(a, offset_mm + tall)], false, fine);
            }
            let (angle, lift) = if along { (0.0, 5.0) } else { (90.0, 7.5) };
            let middle = pt(start_mm + length_mm / 2.0, offset_mm + lift);
            content.text(middle, SMALL_PT, angle, Anchor::Middle, 0.0, caption);
        }
    };
    rule(4.0, 8.0, 100.0, 10.0, "100 mm");
    if imperial {
        // 4 in is 101.6 mm, with a tick at every inch. It starts past the metric rules' corner.
        let inch_mm = crate::units::M_PER_IN * 1000.0;
        rule(14.0, 16.0, 4.0 * inch_mm, inch_mm, "4 in");
    }
}

/// The words a single page carries in its calibration column, top line first.
fn column_lines(sheet: &Sheet, setup: &PageSetup) -> Vec<String> {
    let width_pt = (CALIBRATION_COLUMN_MM - 4.0) * PT_PER_MM;
    let mut lines = Vec::new();
    let heading = if setup.title.trim().is_empty() {
        sheet.name.clone()
    } else {
        format!("{} · {}", sheet.name, setup.title.trim())
    };
    lines.extend(wrap_text(&heading, SMALL_PT, width_pt));
    for note in &sheet.notes {
        lines.extend(wrap_text(note, SMALL_PT, width_pt));
    }
    lines.push(String::new());
    lines.extend(CALIBRATION_LINES.map(str::to_owned));
    lines.push(format!("Scale {}", scale_label(setup.scale)));
    lines.extend(wrap_text(crate::NOT_VALIDATED_NOTE, SMALL_PT, width_pt));
    lines
}

/// One page for each sheet, for a plotter. Every page carries the two 100 mm rules, the
/// instructions and the sheet's notes in a column on its left, and the drawing beside it, so a
/// large plot proves its own scale.
///
/// Fails when there is nothing to draw, when the setup is unusable, or when a page would be
/// longer than a PDF page can be.
pub fn pdf_single(sheets: &[Sheet], setup: &PageSetup) -> Result<Document, String> {
    setup.check()?;
    if sheets.is_empty() {
        return Err("nothing to draw: there are no sheets".into());
    }
    let k = setup.pt_per_m();
    let margin_pt = setup.margin_mm * PT_PER_MM;
    let column_pt = CALIBRATION_COLUMN_MM * PT_PER_MM;
    let mut pdf = Pdf::new(&setup.title);
    for sheet in sheets {
        let b = sheet
            .bounds()
            .ok_or_else(|| format!("sheet `{}` has nothing to draw", sheet.name))?;
        let lines = column_lines(sheet, setup);
        let column_mm = RULES_HEIGHT_MM + lines.len() as f64 * SMALL_LINE_MM + 2.0;
        let inner_height_pt = ((b[3] - b[1]) * k)
            .max(SINGLE_PAGE_MIN_HEIGHT_MM * PT_PER_MM)
            .max(column_mm * PT_PER_MM);
        let width_pt = (b[2] - b[0]) * k + column_pt + 2.0 * margin_pt;
        let height_pt = inner_height_pt + 2.0 * margin_pt;
        if width_pt > MAX_PAGE_PT || height_pt > MAX_PAGE_PT {
            return Err(format!(
                "sheet `{}` at {} needs a page {:.2} x {:.2} m, and a PDF page can be at most \
                 5.08 m on a side: use the tiled PDF, a smaller scale, or DXF",
                sheet.name,
                scale_label(setup.scale),
                width_pt / PT_PER_MM / 1000.0,
                height_pt / PT_PER_MM / 1000.0
            ));
        }
        let mut page = Content::new();
        calibration_rules(&mut page, (margin_pt, margin_pt), setup.imperial);
        let top_mm = RULES_HEIGHT_MM + lines.len() as f64 * SMALL_LINE_MM;
        for (i, line) in lines.iter().enumerate() {
            let y_mm = top_mm - (i + 1) as f64 * SMALL_LINE_MM + 1.0;
            let at = (margin_pt + 2.0 * PT_PER_MM, margin_pt + y_mm * PT_PER_MM);
            page.text(at, SMALL_PT, 0.0, Anchor::Start, 0.0, line);
        }
        // The drawing's lower-left corner sits one margin and the column in, one margin up.
        let origin = (b[0] - (margin_pt + column_pt) / k, b[1] - margin_pt / k);
        page.append(&draw(sheet, setup.scale, origin));
        pdf.add_page(width_pt, height_pt, page);
    }
    pdf.finish()
}

/// A sheet placed in a row of sheets, in millimetres of paper.
struct Placed<'a> {
    sheet: &'a Sheet,
    bounds: [f64; 4],
    /// Where the sheet's left edge is.
    left_mm: f64,
    scale: f64,
}

impl Placed<'_> {
    /// A point of the sheet in millimetres, y up from the row's base line.
    fn mm(&self, p: (f64, f64)) -> (f64, f64) {
        (
            self.left_mm + (p.0 - self.bounds[0]) * self.scale * 1000.0,
            (p.1 - self.bounds[1]) * self.scale * 1000.0,
        )
    }

    /// The sheet's name, then its notes.
    fn note_lines(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.sheet.name.as_str())
            .chain(self.sheet.notes.iter().map(String::as_str))
            .filter(|l| !l.trim().is_empty())
    }
}

/// The width of one line of a note, in millimetres.
fn note_width_mm(line: &str) -> f64 {
    text_width_pt(line, NOTE_HEIGHT_MM * PT_PER_MM) / PT_PER_MM
}

/// The sheets that have something to draw, left to right with [`GAP_MM`] between them and their
/// lowest points on one line; with the row's width, height and the most note lines of any sheet.
fn row(sheets: &[Sheet], scale: f64) -> (Vec<Placed<'_>>, f64, f64, usize) {
    let mut placed = Vec::new();
    let (mut left, mut width, mut height, mut lines) = (0.0, 0.0, 0.0f64, 0);
    for sheet in sheets {
        let Some(bounds) = sheet.bounds() else {
            continue;
        };
        let p = Placed {
            sheet,
            bounds,
            left_mm: left,
            scale,
        };
        // A sheet takes the room of its drawing or of its widest note, whichever is more.
        let notes = p.note_lines().map(note_width_mm).fold(0.0, f64::max);
        let slot = ((bounds[2] - bounds[0]) * scale * 1000.0).max(notes);
        width = left + slot;
        left += slot + GAP_MM;
        height = height.max((bounds[3] - bounds[1]) * scale * 1000.0);
        lines = lines.max(p.note_lines().count());
        placed.push(p);
    }
    (placed, width, height, lines)
}

/// Text as XML character data.
fn xml(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            c if c.is_control() => out.push(' '),
            c => out.push(c),
        }
    }
    out
}

/// The sheets as one SVG, left to right with a 50 mm gap, sized in millimetres so that a program
/// which honours the size opens it at `scale`. Each sheet's name and notes are written under it.
/// There is no rule to measure in this file: check a known dimension after opening it.
pub fn svg(sheets: &[Sheet], scale: f64) -> String {
    let (placed, width, height, lines) = row(sheets, scale);
    let notes = if lines == 0 {
        0.0
    } else {
        NOTE_GAP_MM + lines as f64 * NOTE_LINE_MM
    };
    let m = SVG_MARGIN_MM;
    let (w, h) = (width + 2.0 * m, height + notes + 2.0 * m);
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" \
         width=\"{w:.3}mm\" height=\"{h:.3}mm\" viewBox=\"{:.3} {:.3} {w:.3} {h:.3}\">",
        -m, -m
    );
    let font = "font-family=\"DejaVu Sans, Verdana, sans-serif\"";
    for p in &placed {
        // SVG's y runs down: the row's base line is at `height`.
        let at = |q: (f64, f64)| {
            let (x, y) = p.mm(q);
            (x, height - y)
        };
        let _ = writeln!(out, "<g id=\"{}\">", xml(&p.sheet.name));
        for kind in DRAW_ORDER {
            let s = stroke_for(kind);
            let grey = (s.grey * 255.0).round() as u8;
            let dash = match s.dash_mm {
                Some((on, off)) => format!(" stroke-dasharray=\"{on} {off}\""),
                None => String::new(),
            };
            for path in p.sheet.paths.iter().filter(|path| path.kind == kind) {
                let finite = path
                    .points
                    .iter()
                    .all(|q| q.0.is_finite() && q.1.is_finite());
                if path.points.len() < 2 || !finite {
                    continue;
                }
                let mut d = String::new();
                for (i, q) in path.points.iter().enumerate() {
                    let (x, y) = at(*q);
                    let _ = write!(d, "{} {x:.3} {y:.3} ", if i == 0 { 'M' } else { 'L' });
                }
                if path.closed {
                    d.push('Z');
                }
                let _ = writeln!(
                    out,
                    "<path d=\"{}\" fill=\"none\" stroke=\"#{grey:02x}{grey:02x}{grey:02x}\" \
                     stroke-width=\"{}\" stroke-linejoin=\"round\"{dash}/>",
                    d.trim_end(),
                    s.width_mm
                );
            }
        }
        for label in &p.sheet.labels {
            let (x, y) = at(label.at);
            if !x.is_finite() || !y.is_finite() || label.text.is_empty() {
                continue;
            }
            let anchor = match label.anchor {
                Anchor::Start => "start",
                Anchor::Middle => "middle",
                Anchor::End => "end",
            };
            let turn = if label.angle_deg == 0.0 {
                String::new()
            } else {
                format!(
                    " transform=\"rotate({:.3} {x:.3} {y:.3})\"",
                    -label.angle_deg
                )
            };
            let _ = writeln!(
                out,
                "<text x=\"{x:.3}\" y=\"{y:.3}\" font-size=\"{}\" text-anchor=\"{anchor}\" {font}{turn}>{}</text>",
                label.height_mm,
                xml(&label.text)
            );
        }
        for (i, line) in p.note_lines().enumerate() {
            let y = height + NOTE_GAP_MM + i as f64 * NOTE_LINE_MM;
            let _ = writeln!(
                out,
                "<text x=\"{:.3}\" y=\"{y:.3}\" font-size=\"{NOTE_HEIGHT_MM}\" {font}>{}</text>",
                p.left_mm,
                xml(line)
            );
        }
        out.push_str("</g>\n");
    }
    out.push_str("</svg>\n");
    out
}

/// The sheets as one DXF in millimetres, left to right with a 50 mm gap, on layers `CUT`, `MARK`,
/// `REFERENCE` and `TEXT`. Closed paths are closed polylines. Each sheet's name and notes are
/// written under it. Text is placed by its middle, so where a label starts depends on the
/// reading program's font.
pub fn dxf(sheets: &[Sheet], scale: f64) -> String {
    let layers = [
        DxfLayer::new("CUT", 7),
        DxfLayer::new("MARK", 5),
        DxfLayer::new("REFERENCE", 8),
        DxfLayer::new("TEXT", 3),
    ];
    let (placed, ..) = row(sheets, scale);
    let mut entities = Vec::new();
    for p in &placed {
        for path in &p.sheet.paths {
            let finite = path
                .points
                .iter()
                .all(|q| q.0.is_finite() && q.1.is_finite());
            if path.points.len() < 2 || !finite {
                continue;
            }
            let points: Vec<Vec2> = path
                .points
                .iter()
                .map(|q| {
                    let (x, y) = p.mm(*q);
                    Vec2::new(x, y)
                })
                .collect();
            let layer = match path.kind {
                PathKind::Cut => "CUT",
                PathKind::Mark => "MARK",
                PathKind::Reference => "REFERENCE",
            };
            let shape = if path.closed {
                DxfShape::ClosedPolyline { points }
            } else {
                DxfShape::Polyline { points }
            };
            entities.push(DxfEntity::new(layer, shape));
        }
        for label in &p.sheet.labels {
            let (x, y) = p.mm(label.at);
            if !x.is_finite() || !y.is_finite() || label.text.is_empty() {
                continue;
            }
            // DXF text here is placed by its middle: move there from the label's anchor.
            let half = text_width_pt(&label.text, label.height_mm * PT_PER_MM) / PT_PER_MM / 2.0;
            let along = match label.anchor {
                Anchor::Start => half,
                Anchor::Middle => 0.0,
                Anchor::End => -half,
            };
            let (sin, cos) = label.angle_deg.to_radians().sin_cos();
            entities.push(DxfEntity::new(
                "TEXT",
                DxfShape::Text {
                    at: Vec2::new(x + along * cos, y + along * sin),
                    height: label.height_mm,
                    angle_deg: label.angle_deg,
                    text: label.text.clone(),
                },
            ));
        }
        for (i, line) in p.note_lines().enumerate() {
            let y = -(NOTE_GAP_MM + i as f64 * NOTE_LINE_MM);
            entities.push(DxfEntity::new(
                "TEXT",
                DxfShape::Text {
                    at: Vec2::new(p.left_mm + note_width_mm(line) / 2.0, y),
                    height: NOTE_HEIGHT_MM,
                    angle_deg: 0.0,
                    text: line.to_owned(),
                },
            ));
        }
    }
    write_dxf_in(DxfUnits::Millimetres, &layers, &entities)
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A sheet holding one closed rectangle `w` by `h` metres with its corner at the origin.
    pub(crate) fn rect(w: f64, h: f64) -> Sheet {
        let mut s = Sheet::new("Rect");
        s.paths.push(SheetPath {
            kind: PathKind::Cut,
            points: vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)],
            closed: true,
        });
        s
    }

    /// Every two-point line in a content stream, as `(x0, y0, x1, y1)` in points.
    pub(crate) fn segments(stream: &[u8]) -> Vec<[f64; 4]> {
        let text = String::from_utf8_lossy(stream).into_owned();
        let lines: Vec<&str> = text.lines().collect();
        let point = |line: &str, op: &str| -> Option<(f64, f64)> {
            let mut it = line.strip_suffix(op)?.split_whitespace();
            let p = (it.next()?.parse().ok()?, it.next()?.parse().ok()?);
            it.next().is_none().then_some(p)
        };
        lines
            .windows(3)
            .filter(|w| w[2] == "S")
            .filter_map(|w| {
                let (a, b) = (point(w[0], " m")?, point(w[1], " l")?);
                Some([a.0, a.1, b.0, b.1])
            })
            .collect()
    }

    /// How many of `segments` are `length` points long (to 0.002) and run along x, and along y.
    pub(crate) fn rules(segments: &[[f64; 4]], length: f64) -> (usize, usize) {
        let is = |s: &&[f64; 4], dx: bool| {
            let (along, across) = if dx {
                (s[2] - s[0], s[3] - s[1])
            } else {
                (s[3] - s[1], s[2] - s[0])
            };
            (along - length).abs() < 0.002 && across == 0.0
        };
        (
            segments.iter().filter(|s| is(s, true)).count(),
            segments.iter().filter(|s| is(s, false)).count(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;
    use crate::pdf::{page_stream, page_text};

    fn text(bytes: &[u8]) -> String {
        String::from_utf8_lossy(bytes).into_owned()
    }

    /// Group codes and values of a DXF in file order.
    fn pairs(dxf: &str) -> Vec<(i32, String)> {
        let lines: Vec<&str> = dxf.lines().collect();
        lines
            .chunks(2)
            .map(|c| {
                (
                    c[0].trim().parse().unwrap(),
                    c.get(1).unwrap_or(&"").trim().to_string(),
                )
            })
            .collect()
    }

    /// One DXF entity: its kind, its layer and its other group codes.
    type Entity = (String, String, Vec<(i32, String)>);

    /// Each entity in a DXF's ENTITIES section.
    fn entities(dxf: &str) -> Vec<Entity> {
        let p = pairs(dxf);
        let start = p
            .iter()
            .position(|(c, v)| (*c, v.as_str()) == (2, "ENTITIES"))
            .unwrap();
        let mut out: Vec<Entity> = Vec::new();
        for (c, v) in &p[start + 1..] {
            match (*c, v.as_str()) {
                (0, "ENDSEC") => break,
                (0, kind) => out.push((kind.to_string(), String::new(), Vec::new())),
                (8, layer) => out.last_mut().unwrap().1 = layer.to_string(),
                _ => out.last_mut().unwrap().2.push((*c, v.clone())),
            }
        }
        out
    }

    fn values(codes: &[(i32, String)], code: i32) -> Vec<f64> {
        codes
            .iter()
            .filter(|(c, _)| *c == code)
            .map(|(_, v)| v.parse().unwrap())
            .collect()
    }

    #[test]
    fn bounds_cover_paths_and_label_anchors() {
        let mut s = rect(2.0, 1.0);
        s.labels.push(Label {
            at: (3.0, -1.0),
            height_mm: 3.0,
            angle_deg: 0.0,
            anchor: Anchor::Start,
            text: "x".into(),
        });
        assert_eq!(s.bounds(), Some([0.0, -1.0, 3.0, 1.0]));
        assert_eq!(Sheet::new("empty").bounds(), None);
        // A point that is not a number is not part of the drawing.
        s.paths[0].points.push((f64::NAN, 9.0));
        assert_eq!(s.bounds(), Some([0.0, -1.0, 3.0, 1.0]));
    }

    #[test]
    fn paper_names_and_sizes() {
        assert_eq!(Paper::parse("A4"), Ok(Paper::A4));
        assert_eq!(
            Paper::A4,
            Paper {
                width_mm: 210.0,
                height_mm: 297.0
            }
        );
        assert_eq!(
            Paper::parse(" a3 "),
            Ok(Paper {
                width_mm: 297.0,
                height_mm: 420.0
            })
        );
        assert_eq!(
            Paper::parse("letter"),
            Ok(Paper {
                width_mm: 215.9,
                height_mm: 279.4
            })
        );
        assert_eq!(
            Paper::parse("610x914"),
            Ok(Paper {
                width_mm: 610.0,
                height_mm: 914.0
            })
        );
        assert_eq!(Paper::parse("215.9X279.4"), Ok(Paper::LETTER));
        for bad in [
            "a5",
            "10x10",
            "610x",
            "x",
            "6000x100",
            "",
            "nanx100",
            "100x100x100",
        ] {
            assert!(Paper::parse(bad).is_err(), "{bad}");
        }
    }

    /// Width (1000 + 120 + 20) mm = 3231.496 pt, height (500 + 20) mm = 1474.016 pt. The drawing's
    /// corner is one margin and the calibration column in: 130 mm = 368.504 pt, and one margin up:
    /// 28.346 pt. 100 mm is 283.465 pt.
    #[test]
    fn a_single_page_is_the_drawing_the_calibration_column_and_the_margins() {
        let doc = pdf_single(&[rect(1.0, 0.5)], &PageSetup::default()).unwrap();
        assert_eq!(doc.pages, 1);
        assert!(doc.missing.is_empty());
        assert!(text(&doc.bytes).contains("/MediaBox [0 0 3231.496 1474.016]"));
        let stream = page_stream(&doc.bytes, 0);
        assert!(
            text(&stream).contains("368.504 28.346 m\n3203.150 28.346 l\n"),
            "{}",
            text(&stream)
        );
        let shown = page_text(&doc.bytes, 0);
        for line in CALIBRATION_LINES {
            assert!(shown.iter().any(|s| s == line), "{line}");
        }
        assert!(shown.iter().any(|s| s == "Scale 1:1"), "{shown:?}");
        assert!(shown.iter().any(|s| s == "100 mm"), "{shown:?}");
        assert!(!shown.iter().any(|s| s.contains("4 in")), "{shown:?}");
        assert_eq!(rules(&segments(&stream), 283.465), (1, 1));
        // The rules sit inside the column, left of the drawing.
        for s in segments(&stream).iter().filter(|s| s[0] < 368.0) {
            assert!(s[0] >= 28.346 && s[2] <= 368.504 && s[1] >= 28.346, "{s:?}");
        }
    }

    #[test]
    fn every_single_page_carries_the_rules() {
        let mut second = rect(0.3, 0.2);
        second.name = "Second".into();
        second.notes = vec!["A note about the second sheet".into()];
        let setup = PageSetup {
            title: "Kayık".into(),
            imperial: true,
            ..PageSetup::default()
        };
        let doc = pdf_single(&[rect(1.0, 0.5), second], &setup).unwrap();
        assert_eq!(doc.pages, 2);
        for page in 0..2 {
            let shown = page_text(&doc.bytes, page);
            for line in CALIBRATION_LINES {
                assert!(shown.iter().any(|s| s == line), "page {page}: {line}");
            }
            let stream = page_stream(&doc.bytes, page);
            assert_eq!(rules(&segments(&stream), 283.465), (1, 1), "page {page}");
            // 4 in is 288 pt.
            assert_eq!(rules(&segments(&stream), 288.0), (1, 1), "page {page}");
            assert!(shown.iter().any(|s| s == "4 in"), "{shown:?}");
        }
        let shown = page_text(&doc.bytes, 1);
        assert!(
            shown.iter().any(|s| s == "A note about the second sheet"),
            "{shown:?}"
        );
        assert!(
            shown
                .iter()
                .any(|s| s.contains("Second") && s.contains("Kayık")),
            "{shown:?}"
        );
        assert!(!page_text(&doc.bytes, 0)
            .iter()
            .any(|s| s.contains("second sheet")));
    }

    /// (100 + 120 + 20) mm = 680.315 pt wide; the height is the 150 mm floor + 20 mm = 481.890 pt.
    #[test]
    fn scale_shrinks_the_drawing_not_the_line_width() {
        let setup = PageSetup {
            scale: 0.1,
            ..PageSetup::default()
        };
        let doc = pdf_single(&[rect(1.0, 0.5)], &setup).unwrap();
        assert!(text(&doc.bytes).contains("/MediaBox [0 0 680.315 481.890]"));
        let stream = text(&page_stream(&doc.bytes, 0));
        // 0.35 mm is 0.992 pt, and the rectangle is 100 mm = 283.465 pt wide from 368.504.
        assert!(stream.contains("0.992 w 0.000 G [] 0 d"), "{stream}");
        assert!(
            stream.contains("368.504 28.346 m\n651.969 28.346 l\n"),
            "{stream}"
        );
        assert!(page_text(&doc.bytes, 0).iter().any(|s| s == "Scale 1:10"));
    }

    #[test]
    fn a_sheet_too_long_for_one_page_is_refused() {
        let err = pdf_single(&[rect(6.0, 1.0)], &PageSetup::default()).unwrap_err();
        for needle in ["5.08 m", "tiled", "DXF"] {
            assert!(err.contains(needle), "{needle}: {err}");
        }
        // 4.9 m of drawing with the column and margins is 5.04 m: it fits.
        assert!(pdf_single(&[rect(4.9, 1.0)], &PageSetup::default()).is_ok());
    }

    #[test]
    fn nothing_to_draw_is_an_error() {
        let setup = PageSetup::default();
        for sheets in [
            vec![],
            vec![Sheet::new("Empty")],
            vec![rect(1.0, 1.0), Sheet::new("Empty")],
        ] {
            let err = pdf_single(&sheets, &setup).unwrap_err();
            assert!(err.contains("nothing to draw"), "{err}");
        }
    }

    #[test]
    fn a_bad_setup_is_refused() {
        let ok = PageSetup::default();
        assert_eq!(ok.check(), Ok(()));
        assert_eq!(
            (ok.paper, ok.margin_mm, ok.overlap_mm, ok.scale),
            (Paper::A4, 10.0, 10.0, 1.0)
        );
        assert!(!ok.imperial && ok.title.is_empty());
        for bad in [
            PageSetup {
                margin_mm: -1.0,
                ..ok.clone()
            },
            PageSetup {
                margin_mm: f64::NAN,
                ..ok.clone()
            },
            PageSetup {
                overlap_mm: -1.0,
                ..ok.clone()
            },
            // A4 less two 10 mm margins is 190 mm wide: 180 mm of overlap leaves a 10 mm step.
            PageSetup {
                overlap_mm: 180.0,
                ..ok.clone()
            },
            PageSetup {
                margin_mm: 100.0,
                ..ok.clone()
            },
            PageSetup {
                scale: 0.0,
                ..ok.clone()
            },
            PageSetup {
                scale: f64::NAN,
                ..ok.clone()
            },
            PageSetup {
                scale: 11.0,
                ..ok.clone()
            },
            PageSetup {
                scale: 0.0005,
                ..ok.clone()
            },
            PageSetup {
                paper: Paper {
                    width_mm: f64::NAN,
                    height_mm: 297.0,
                },
                ..ok.clone()
            },
        ] {
            assert!(bad.check().is_err(), "{bad:?}");
            assert!(pdf_single(&[rect(1.0, 0.5)], &bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn svg_opens_at_size() {
        let mut sheet = rect(1.0, 0.5);
        sheet.paths.push(SheetPath {
            kind: PathKind::Reference,
            points: vec![(0.0, 0.0), (1.0, 0.5)],
            closed: false,
        });
        sheet.labels.push(Label {
            at: (0.5, 0.25),
            height_mm: 4.0,
            angle_deg: 90.0,
            anchor: Anchor::Middle,
            text: "A & B <Şile>".into(),
        });
        let out = svg(&[sheet], 1.0);
        let root = &out[out.find("<svg").unwrap()..];
        let root = &root[..root.find('>').unwrap()];
        // The drawing is 1000 x 500 mm with 10 mm around it.
        assert!(root.contains("width=\"1020.000mm\""), "{root}");
        // Below the drawing: 10 mm, then the sheet's name on one 5 mm line.
        assert!(root.contains("height=\"535.000mm\""), "{root}");
        assert!(
            root.contains("viewBox=\"-10.000 -10.000 1020.000 535.000\""),
            "{root}"
        );
        // y runs down in SVG: the rectangle's top is at 0 and its base at 500.
        assert!(
            out.contains(
                "d=\"M 0.000 500.000 L 1000.000 500.000 L 1000.000 0.000 L 0.000 0.000 Z\""
            ),
            "{out}"
        );
        assert!(out.contains("stroke-width=\"0.35\""));
        assert!(out.contains("stroke-dasharray=\"2 1\""));
        assert!(out.contains("A &amp; B &lt;Şile&gt;"), "{out}");
        assert!(out.contains("text-anchor=\"middle\""));
        assert!(out.contains("rotate(-90.000 500.000 250.000)"), "{out}");
        // Nothing to draw is still a document.
        assert!(svg(&[], 1.0).contains("</svg>"));
    }

    #[test]
    fn dxf_has_named_layers_and_millimetres() {
        let mut sheet = rect(1.0, 0.5);
        sheet.paths.push(SheetPath {
            kind: PathKind::Mark,
            points: vec![(0.5, 0.0), (0.5, 0.5)],
            closed: false,
        });
        sheet.paths.push(SheetPath {
            kind: PathKind::Reference,
            points: vec![(0.0, 0.0), (1.0, 0.5)],
            closed: false,
        });
        sheet.labels.push(Label {
            at: (0.5, 0.25),
            height_mm: 4.0,
            angle_deg: 0.0,
            anchor: Anchor::Middle,
            text: "CL".into(),
        });
        sheet.notes = vec!["Station 3 of 11".into()];
        let out = dxf(&[sheet], 1.0);
        let p = pairs(&out);
        for layer in ["CUT", "MARK", "REFERENCE", "TEXT"] {
            assert!(p.iter().any(|(c, v)| *c == 2 && v == layer), "{layer}");
        }
        let units = p
            .iter()
            .position(|(c, v)| (*c, v.as_str()) == (9, "$INSUNITS"))
            .unwrap();
        assert_eq!(p[units + 1], (70, "4".to_string()));
        let e = entities(&out);
        let cut: Vec<_> = e.iter().filter(|(_, layer, _)| layer == "CUT").collect();
        assert_eq!(cut.len(), 1);
        assert_eq!(cut[0].0, "LWPOLYLINE");
        assert_eq!(values(&cut[0].2, 70), [129.0]);
        assert_eq!(values(&cut[0].2, 10), [0.0, 1000.0, 1000.0, 0.0]);
        assert_eq!(values(&cut[0].2, 20), [0.0, 0.0, 500.0, 500.0]);
        let mark: Vec<_> = e.iter().filter(|(_, layer, _)| layer == "MARK").collect();
        assert_eq!(values(&mark[0].2, 70), [128.0]);
        assert_eq!(
            e.iter()
                .filter(|(_, layer, _)| layer == "REFERENCE")
                .count(),
            1
        );
        let words: Vec<&str> = e
            .iter()
            .filter(|(kind, layer, _)| kind == "TEXT" && layer == "TEXT")
            .map(|(_, _, codes)| codes.iter().find(|(c, _)| *c == 1).unwrap().1.as_str())
            .collect();
        assert_eq!(words, ["CL", "Rect", "Station 3 of 11"]);
        // The label is 4 mm high in a millimetre drawing.
        let label = e.iter().find(|(kind, _, _)| kind == "TEXT").unwrap();
        assert_eq!(values(&label.2, 40), [4.0]);
    }

    #[test]
    fn sheets_sit_side_by_side_with_a_50_mm_gap() {
        let sheets = [rect(1.0, 0.5), rect(1.0, 0.5)];
        let out = svg(&sheets, 1.0);
        assert!(
            out.contains("d=\"M 1050.000 500.000 L 2050.000 500.000 "),
            "{out}"
        );
        assert!(out.contains("width=\"2070.000mm\""));
        let e = entities(&dxf(&sheets, 1.0));
        let cut: Vec<_> = e.iter().filter(|(_, layer, _)| layer == "CUT").collect();
        assert_eq!(values(&cut[1].2, 10), [1050.0, 2050.0, 2050.0, 1050.0]);
        // At 1:10 the drawings are 100 mm wide and the gap is still 50 mm of paper.
        let e = entities(&dxf(&sheets, 0.1));
        let cut: Vec<_> = e.iter().filter(|(_, layer, _)| layer == "CUT").collect();
        assert_eq!(values(&cut[1].2, 10), [150.0, 250.0, 250.0, 150.0]);
    }
}
