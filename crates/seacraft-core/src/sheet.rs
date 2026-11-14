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

/// What a sheet covers on paper at `scale`, in millimetres from the sheet's own origin:
/// `[min_x, min_y, max_x, max_y]` over its paths and the whole of each label's text. `None` when
/// the sheet has nothing to draw.
fn extent_mm(sheet: &Sheet, scale: f64) -> Option<[f64; 4]> {
    let k = scale * 1000.0;
    let points = sheet
        .paths
        .iter()
        .flat_map(|p| p.points.iter().map(move |q| (q.0 * k, q.1 * k)));
    let corners = sheet.labels.iter().flat_map(|label| label_box_mm(label, k));
    points
        .chain(corners)
        .filter(|p| p.0.is_finite() && p.1.is_finite())
        .fold(None, |acc: Option<[f64; 4]>, (x, y)| {
            Some(acc.map_or([x, y, x, y], |b| {
                [b[0].min(x), b[1].min(y), b[2].max(x), b[3].max(y)]
            }))
        })
}

/// The base line of a label's text on paper, start and end, in millimetres; `k` is millimetres of
/// paper per metre of drawing.
fn label_line_mm(label: &Label, k: f64) -> ((f64, f64), (f64, f64)) {
    let width = text_width_pt(&label.text, label.height_mm * PT_PER_MM) / PT_PER_MM;
    let back = match label.anchor {
        Anchor::Start => 0.0,
        Anchor::Middle => width / 2.0,
        Anchor::End => width,
    };
    let (sin, cos) = label.angle_deg.to_radians().sin_cos();
    let start = (label.at.0 * k - back * cos, label.at.1 * k - back * sin);
    (start, (start.0 + width * cos, start.1 + width * sin))
}

/// The box a label's letters fill on paper, in millimetres, round from the base line's start:
/// they stand on the base line, a letter height tall.
fn label_box_mm(label: &Label, k: f64) -> [(f64, f64); 4] {
    let (start, end) = label_line_mm(label, k);
    let (sin, cos) = label.angle_deg.to_radians().sin_cos();
    let up = (-sin * label.height_mm, cos * label.height_mm);
    [
        start,
        end,
        (end.0 + up.0, end.1 + up.1),
        (start.0 + up.0, start.1 + up.1),
    ]
}

/// The error for sheets that cannot make a document: none at all, or one with nothing on it.
fn nothing_to_draw(sheets: &[Sheet], scale: f64) -> Result<(), String> {
    if sheets.is_empty() {
        return Err("nothing to draw: there are no sheets".into());
    }
    match sheets.iter().find(|s| extent_mm(s, scale).is_none()) {
        Some(sheet) => Err(format!("sheet `{}` has nothing to draw", sheet.name)),
        None => Ok(()),
    }
}

/// One page for each sheet, for a plotter. Every page carries the two 100 mm rules, the
/// instructions and the sheet's notes in a column on its left, and the drawing beside it, so a
/// large plot proves its own scale.
///
/// Fails when there is nothing to draw, when the setup is unusable, or when a page would be
/// longer than a PDF page can be.
pub fn pdf_single(sheets: &[Sheet], setup: &PageSetup) -> Result<Document, String> {
    setup.check()?;
    nothing_to_draw(sheets, setup.scale)?;
    let margin = setup.margin_mm;
    let mut pdf = Pdf::new(&setup.title);
    for sheet in sheets {
        let Some(e) = extent_mm(sheet, setup.scale) else {
            continue;
        };
        let lines = column_lines(sheet, setup);
        let column_mm = RULES_HEIGHT_MM + lines.len() as f64 * SMALL_LINE_MM + 2.0;
        let inner_height = (e[3] - e[1]).max(SINGLE_PAGE_MIN_HEIGHT_MM).max(column_mm);
        let width_pt = (e[2] - e[0] + CALIBRATION_COLUMN_MM + 2.0 * margin) * PT_PER_MM;
        let height_pt = (inner_height + 2.0 * margin) * PT_PER_MM;
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
        let corner = (margin * PT_PER_MM, margin * PT_PER_MM);
        calibration_rules(&mut page, corner, setup.imperial);
        let top_mm = RULES_HEIGHT_MM + lines.len() as f64 * SMALL_LINE_MM;
        for (i, line) in lines.iter().enumerate() {
            let y_mm = top_mm - (i + 1) as f64 * SMALL_LINE_MM + 1.0;
            let at = (corner.0 + 2.0 * PT_PER_MM, corner.1 + y_mm * PT_PER_MM);
            page.text(at, SMALL_PT, 0.0, Anchor::Start, 0.0, line);
        }
        // The drawing's lower-left corner sits one margin and the column in, one margin up.
        let k = setup.scale * 1000.0;
        let origin = (
            (e[0] - margin - CALIBRATION_COLUMN_MM) / k,
            (e[1] - margin) / k,
        );
        page.append(&draw(sheet, setup.scale, origin));
        pdf.add_page(width_pt, height_pt, page);
    }
    pdf.finish()
}

/// A sheet placed in a row of sheets, in millimetres of paper.
struct Placed<'a> {
    sheet: &'a Sheet,
    /// What the sheet covers, its labels' text included ([`extent_mm`]).
    extent_mm: [f64; 4],
    /// Where the sheet's left edge is.
    left_mm: f64,
    scale: f64,
}

impl Placed<'_> {
    /// A point of the sheet in millimetres, y up from the row's base line.
    fn mm(&self, p: (f64, f64)) -> (f64, f64) {
        let k = self.scale * 1000.0;
        (
            self.left_mm + p.0 * k - self.extent_mm[0],
            p.1 * k - self.extent_mm[1],
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
        let Some(e) = extent_mm(sheet, scale) else {
            continue;
        };
        let p = Placed {
            sheet,
            extent_mm: e,
            left_mm: left,
            scale,
        };
        // A sheet takes the room of its drawing or of its widest note, whichever is more.
        let notes = p.note_lines().map(note_width_mm).fold(0.0, f64::max);
        let slot = (e[2] - e[0]).max(notes);
        width = left + slot;
        left += slot + GAP_MM;
        height = height.max(e[3] - e[1]);
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

/// The most pages a tiled document may have.
pub const MAX_TILED_PAGES: usize = 1000;
/// The most tiles a sheet's grid may have before its tiles are looked at one by one.
const MAX_GRID_TILES: usize = 1_000_000;
/// The clear strip kept around a drawing inside its grid, in millimetres.
const PAD_MM: f64 = 5.0;
/// How far apart the crosses on a join line are, and how long their arms, in millimetres.
const CROSS_EVERY_MM: f64 = 50.0;
const CROSS_MM: f64 = 4.0;
/// The join lines, the border of the printable area and the map's grid.
const FAINT: Stroke = Stroke {
    width_mm: 0.1,
    grey: 0.5,
    dash_mm: None,
};

/// How a sheet is cut into pages.
#[derive(Clone, Debug, PartialEq)]
pub struct TileGrid {
    /// Tiles across.
    pub cols: usize,
    /// Tiles down.
    pub rows: usize,
    /// Whether the pages are turned on their side.
    pub landscape: bool,
    /// The part of a page that is printed on: paper less the margins, in millimetres.
    pub printable_mm: (f64, f64),
    /// From one tile to the next: the printable size less the overlap, in millimetres.
    pub step_mm: (f64, f64),
    /// `(col, row)` of each tile that is printed, in page order: row by row from the top.
    pub printed: Vec<(usize, usize)>,
}

impl TileGrid {
    /// The size of the area all the tiles cover together, in millimetres.
    fn size_mm(&self) -> (f64, f64) {
        (
            self.cols as f64 * self.step_mm.0 + self.printable_mm.0 - self.step_mm.0,
            self.rows as f64 * self.step_mm.1 + self.printable_mm.1 - self.step_mm.1,
        )
    }
}

/// A grid and where the drawing is in it.
struct Tiling {
    grid: TileGrid,
    /// The grid's lower-left corner, in millimetres of paper from the sheet's origin.
    corner_mm: (f64, f64),
}

/// A grid too large to look at tile by tile: how many tiles it has.
struct TooManyTiles(usize);

/// Whether the segment from `a` to `b` touches the rectangle `[x0, y0, x1, y1]`, by clipping its
/// parameter range against each side.
fn segment_touches(a: (f64, f64), b: (f64, f64), r: [f64; 4]) -> bool {
    let (mut t0, mut t1) = (0.0f64, 1.0f64);
    let d = (b.0 - a.0, b.1 - a.1);
    // Each side as `p * t <= q`.
    for (p, q) in [
        (-d.0, a.0 - r[0]),
        (d.0, r[2] - a.0),
        (-d.1, a.1 - r[1]),
        (d.1, r[3] - a.1),
    ] {
        if p == 0.0 {
            if q < 0.0 {
                return false;
            }
        } else if p < 0.0 {
            t0 = t0.max(q / p);
        } else {
            t1 = t1.min(q / p);
        }
    }
    t0 <= t1
}

/// The grid for one orientation. `Ok(None)` when the sheet has nothing to draw.
fn tiling(
    sheet: &Sheet,
    setup: &PageSetup,
    landscape: bool,
) -> Result<Option<Tiling>, TooManyTiles> {
    let Some(e) = extent_mm(sheet, setup.scale) else {
        return Ok(None);
    };
    let (pw, ph) = if landscape {
        (setup.paper.height_mm, setup.paper.width_mm)
    } else {
        (setup.paper.width_mm, setup.paper.height_mm)
    };
    let printable = (pw - 2.0 * setup.margin_mm, ph - 2.0 * setup.margin_mm);
    let step = (
        printable.0 - setup.overlap_mm,
        printable.1 - setup.overlap_mm,
    );
    let size = (e[2] - e[0] + 2.0 * PAD_MM, e[3] - e[1] + 2.0 * PAD_MM);
    let count = |length: f64, step: f64| -> usize {
        // `as` saturates, so an absurd size is a very large count and not a wrapped one.
        (((length - setup.overlap_mm) / step).ceil().max(1.0)) as usize
    };
    let (cols, rows) = (count(size.0, step.0), count(size.1, step.1));
    let tiles = cols.saturating_mul(rows);
    if tiles > MAX_GRID_TILES {
        return Err(TooManyTiles(tiles));
    }
    let mut grid = TileGrid {
        cols,
        rows,
        landscape,
        printable_mm: printable,
        step_mm: step,
        printed: Vec::new(),
    };
    let (gw, gh) = grid.size_mm();
    // The padded drawing sits in the middle of the grid.
    let corner_mm = (
        e[0] - PAD_MM - (gw - size.0) / 2.0,
        e[1] - PAD_MM - (gh - size.1) / 2.0,
    );

    // Each line marks the tiles it touches; only the tiles its bounding box reaches are tried.
    let k = setup.scale * 1000.0;
    let mut used = std::collections::BTreeSet::new();
    let mut mark = |a: (f64, f64), b: (f64, f64)| {
        if ![a.0, a.1, b.0, b.1].iter().all(|v| v.is_finite()) {
            return;
        }
        // Grid millimetres, x from the left and y down from the top, as rows are counted.
        let a = (a.0 - corner_mm.0, gh - (a.1 - corner_mm.1));
        let b = (b.0 - corner_mm.0, gh - (b.1 - corner_mm.1));
        let range = |lo: f64, hi: f64, step: f64, size: f64, n: usize| {
            let first = ((lo - size) / step).ceil().max(0.0) as usize;
            let last = ((hi / step).floor().max(0.0) as usize).min(n - 1);
            first..=last
        };
        for row in range(a.1.min(b.1), a.1.max(b.1), step.1, printable.1, rows) {
            for col in range(a.0.min(b.0), a.0.max(b.0), step.0, printable.0, cols) {
                let (x, y) = (col as f64 * step.0, row as f64 * step.1);
                if segment_touches(a, b, [x, y, x + printable.0, y + printable.1]) {
                    used.insert((row, col));
                }
            }
        }
    };
    for path in &sheet.paths {
        let on_paper = |p: &(f64, f64)| (p.0 * k, p.1 * k);
        for pair in path.points.windows(2) {
            mark(on_paper(&pair[0]), on_paper(&pair[1]));
        }
        if let (true, Some(first), Some(last)) =
            (path.closed, path.points.first(), path.points.last())
        {
            mark(on_paper(last), on_paper(first));
        }
    }
    // A label marks the tiles its letters reach: its box's four sides.
    for label in sheet.labels.iter().filter(|l| !l.text.is_empty()) {
        let corners = label_box_mm(label, k);
        for i in 0..4 {
            mark(corners[i], corners[(i + 1) % 4]);
        }
    }
    grid.printed = used.into_iter().map(|(row, col)| (col, row)).collect();
    Ok(Some(Tiling { grid, corner_mm }))
}

/// The grid to print: upright or on its side, whichever prints fewer pages; upright on a tie.
fn best_tiling(sheet: &Sheet, setup: &PageSetup) -> Result<Option<Tiling>, TooManyTiles> {
    match (tiling(sheet, setup, false), tiling(sheet, setup, true)) {
        (Ok(Some(upright)), Ok(Some(side))) => {
            let fewer = side.grid.printed.len() < upright.grid.printed.len();
            Ok(Some(if fewer { side } else { upright }))
        }
        (Ok(Some(one)), Err(_)) | (Err(_), Ok(Some(one))) => Ok(Some(one)),
        (Err(a), Err(b)) => Err(TooManyTiles(a.0.min(b.0))),
        _ => Ok(None),
    }
}

/// How `sheet` is cut into pages with this setup, or `None` when it has nothing to draw, the
/// setup is unusable, or the grid would have more than a million tiles.
pub fn tile_grid(sheet: &Sheet, setup: &PageSetup) -> Option<TileGrid> {
    setup.check().ok()?;
    best_tiling(sheet, setup).ok().flatten().map(|t| t.grid)
}

/// `[x0, y0, x1, y1]` of a tile's window in grid millimetres, y up, origin at the grid's
/// lower-left corner. Rows are counted from the top.
pub fn tile_window_mm(grid: &TileGrid, col: usize, row: usize) -> [f64; 4] {
    let x0 = col as f64 * grid.step_mm.0;
    let y1 = grid.size_mm().1 - row as f64 * grid.step_mm.1;
    [x0, y1 - grid.printable_mm.1, x0 + grid.printable_mm.0, y1]
}

/// A row's letter: A to Z, then AA, AB and so on.
pub fn row_name(row: usize) -> String {
    let mut n = row;
    let mut name = Vec::new();
    loop {
        name.push(b'A' + (n % 26) as u8);
        if n < 26 {
            break;
        }
        n = n / 26 - 1;
    }
    name.reverse();
    String::from_utf8(name).unwrap_or_default()
}

/// Whether a sheet gets a map page: when it is more than one tile, or has notes to put on paper.
fn has_map(sheet: &Sheet, grid: &TileGrid) -> bool {
    grid.printed.len() > 1 || sheet.notes.iter().any(|n| !n.trim().is_empty())
}

/// How many pages [`pdf_tiled`] would write: the calibration page, the map pages and the tiles,
/// without building the document. 0 when there is nothing to draw or the setup is unusable. A
/// sheet too large to look at tile by tile counts every tile of its grid.
pub fn page_count(sheets: &[Sheet], setup: &PageSetup) -> usize {
    if setup.check().is_err() {
        return 0;
    }
    let mut pages = 0usize;
    for sheet in sheets {
        pages = pages.saturating_add(match best_tiling(sheet, setup) {
            Ok(Some(t)) => t.grid.printed.len() + usize::from(has_map(sheet, &t.grid)),
            Ok(None) => 0,
            Err(TooManyTiles(tiles)) => tiles,
        });
    }
    if pages == 0 {
        0
    } else {
        pages.saturating_add(1)
    }
}

/// The join lines of a grid as one drawing in grid points: a dashed line down the middle of every
/// strip two tiles share, with a cross every 50 mm. `None` for a single tile.
fn join_lines(grid: &TileGrid) -> Option<Content> {
    if grid.cols < 2 && grid.rows < 2 {
        return None;
    }
    let (gw, gh) = grid.size_mm();
    let overlap = (
        grid.printable_mm.0 - grid.step_mm.0,
        grid.printable_mm.1 - grid.step_mm.1,
    );
    let dashed = Stroke {
        dash_mm: Some((1.0, 1.0)),
        ..FAINT
    };
    let mut c = Content::new();
    // `upright` lines run up the page at `at` across it; the others the other way.
    let mut line = |upright: bool, at: f64, length: f64| {
        let pt = |along: f64, across: f64| {
            let (x, y) = if upright {
                (at + across, along)
            } else {
                (along, at + across)
            };
            (x * PT_PER_MM, y * PT_PER_MM)
        };
        c.polyline(&[pt(0.0, 0.0), pt(length, 0.0)], false, dashed);
        let half = CROSS_MM / 2.0;
        let mut along = CROSS_EVERY_MM / 2.0;
        while along < length {
            c.polyline(
                &[pt(along - half, 0.0), pt(along + half, 0.0)],
                false,
                FAINT,
            );
            c.polyline(&[pt(along, -half), pt(along, half)], false, FAINT);
            along += CROSS_EVERY_MM;
        }
    };
    for col in 1..grid.cols {
        line(true, col as f64 * grid.step_mm.0 + overlap.0 / 2.0, gh);
    }
    for row in 1..grid.rows {
        line(
            false,
            gh - row as f64 * grid.step_mm.1 - overlap.1 / 2.0,
            gw,
        );
    }
    Some(c)
}

/// A tile's address: its row's letter and its column's number from 1.
fn address(col: usize, row: usize) -> String {
    format!("{}{}", row_name(row), col + 1)
}

/// The first page: how to print, the rules to measure, and where each sheet is.
fn calibration_page(setup: &PageSetup, index: &[String]) -> Content {
    let (pw, ph) = (setup.paper.width_mm, setup.paper.height_mm);
    let m = setup.margin_mm;
    let width_pt = (pw - 2.0 * m) * PT_PER_MM;
    let mut c = Content::new();
    calibration_rules(&mut c, (m * PT_PER_MM, m * PT_PER_MM), setup.imperial);
    // Words run down from the top of the printable area; the rules are at its foot.
    let mut y_mm = ph - m;
    let mut say = |c: &mut Content, size_pt: f64, text: &str| {
        for line in wrap_text(text, size_pt, width_pt) {
            y_mm -= size_pt / PT_PER_MM * 1.45;
            c.text(
                (m * PT_PER_MM, y_mm * PT_PER_MM),
                size_pt,
                0.0,
                Anchor::Start,
                0.0,
                &line,
            );
        }
    };
    if !setup.title.trim().is_empty() {
        say(&mut c, 14.0, setup.title.trim());
    }
    for line in CALIBRATION_LINES {
        say(&mut c, 10.0, line);
    }
    say(&mut c, 10.0, &format!("Scale {}", scale_label(setup.scale)));
    say(&mut c, 8.0, crate::NOT_VALIDATED_NOTE);
    y_mm -= 4.0;
    // The sheets, in as many columns as fit above the rules.
    let line_mm = SMALL_LINE_MM;
    let floor_mm = m + RULES_HEIGHT_MM + 4.0;
    let per_column = (((y_mm - floor_mm) / line_mm).floor().max(1.0)) as usize;
    let columns = ((pw - 2.0 * m) / 70.0).floor().max(1.0) as usize;
    let room = per_column * columns;
    for (i, entry) in index.iter().enumerate().take(room) {
        let text = if i + 1 == room && index.len() > room {
            format!("… and {} more sheets", index.len() - room + 1)
        } else {
            entry.clone()
        };
        let x_mm = m + (i / per_column) as f64 * (pw - 2.0 * m) / columns as f64;
        let line_y = y_mm - ((i % per_column) + 1) as f64 * line_mm;
        c.text(
            (x_mm * PT_PER_MM, line_y * PT_PER_MM),
            SMALL_PT,
            0.0,
            Anchor::Start,
            0.0,
            &text,
        );
    }
    c
}

/// A sheet's map page: its name and notes, and the whole sheet scaled to fit with the grid over
/// it and each printed tile's address and page number.
fn map_page(
    sheet: &Sheet,
    setup: &PageSetup,
    grid: &TileGrid,
    pattern: crate::pdf::FormId,
    first_tile_page: usize,
) -> Content {
    let (pw, ph) = (setup.paper.width_mm, setup.paper.height_mm);
    let m = setup.margin_mm;
    let width_pt = (pw - 2.0 * m) * PT_PER_MM;
    let mut c = Content::new();
    let mut y_mm = ph - m;
    let mut say = |c: &mut Content, size_pt: f64, text: &str| {
        for line in wrap_text(text, size_pt, width_pt) {
            y_mm -= size_pt / PT_PER_MM * 1.45;
            c.text(
                (m * PT_PER_MM, y_mm * PT_PER_MM),
                size_pt,
                0.0,
                Anchor::Start,
                0.0,
                &line,
            );
        }
    };
    say(&mut c, 14.0, &sheet.name);
    let pages = if grid.landscape {
        "pages on their side"
    } else {
        "upright pages"
    };
    let layout = format!(
        "Scale {} · {} x {} tiles, {} printed, {pages}",
        scale_label(setup.scale),
        grid.cols,
        grid.rows,
        grid.printed.len()
    );
    say(&mut c, 9.0, &layout);
    for note in &sheet.notes {
        say(&mut c, 9.0, note);
    }
    // The map fills what is left, keeping its shape.
    let (gw, gh) = grid.size_mm();
    let room = (pw - 2.0 * m, (y_mm - 6.0 - m).max(10.0));
    let shrink = (room.0 / gw).min(room.1 / gh).min(1.0);
    let corner = (m, y_mm - 6.0 - gh * shrink);
    let pt = |x_mm: f64, y_mm: f64| {
        (
            (corner.0 + x_mm * shrink) * PT_PER_MM,
            (corner.1 + y_mm * shrink) * PT_PER_MM,
        )
    };
    c.save();
    c.translate(corner.0 * PT_PER_MM, corner.1 * PT_PER_MM);
    c.scale(shrink);
    c.form(pattern);
    c.restore();
    // Each tile as the part that is its alone: the cells meet on the join lines.
    let overlap = (
        grid.printable_mm.0 - grid.step_mm.0,
        grid.printable_mm.1 - grid.step_mm.1,
    );
    let edge = |i: usize, n: usize, step: f64, overlap: f64, size: f64| match i {
        0 => 0.0,
        i if i == n => size,
        i => i as f64 * step + overlap / 2.0,
    };
    for col in 0..=grid.cols {
        let x = edge(col, grid.cols, grid.step_mm.0, overlap.0, gw);
        c.polyline(&[pt(x, 0.0), pt(x, gh)], false, FAINT);
    }
    for row in 0..=grid.rows {
        let y = gh - edge(row, grid.rows, grid.step_mm.1, overlap.1, gh);
        c.polyline(&[pt(0.0, y), pt(gw, y)], false, FAINT);
    }
    let cell = (
        gw / grid.cols as f64 * shrink,
        gh / grid.rows as f64 * shrink,
    );
    let size_pt = (cell.0.min(cell.1) * PT_PER_MM * 0.22).clamp(2.0, 9.0);
    for (i, (col, row)) in grid.printed.iter().enumerate() {
        let x = (edge(*col, grid.cols, grid.step_mm.0, overlap.0, gw)
            + edge(col + 1, grid.cols, grid.step_mm.0, overlap.0, gw))
            / 2.0;
        let y = gh
            - (edge(*row, grid.rows, grid.step_mm.1, overlap.1, gh)
                + edge(row + 1, grid.rows, grid.step_mm.1, overlap.1, gh))
                / 2.0;
        let at = pt(x, y);
        c.text(
            (at.0, at.1 + 0.2 * size_pt),
            size_pt,
            0.0,
            Anchor::Middle,
            0.35,
            &address(*col, *row),
        );
        let page = format!("page {}", first_tile_page + i);
        c.text(
            (at.0, at.1 - size_pt),
            size_pt,
            0.0,
            Anchor::Middle,
            0.35,
            &page,
        );
    }
    c
}

/// Full-size patterns tiled over ordinary paper: a calibration page, then for each sheet a map
/// page (when it is more than one tile or has notes) and its tiles, row by row from the top.
///
/// Neighbouring tiles share a strip one overlap wide, with a join line down its middle that is
/// printed on both. A tile nothing crosses is not printed.
///
/// Fails when there is nothing to draw, when the setup is unusable or the paper cannot hold the
/// 100 mm rules, or when the document would have more than [`MAX_TILED_PAGES`] pages.
pub fn pdf_tiled(sheets: &[Sheet], setup: &PageSetup) -> Result<Document, String> {
    setup.check()?;
    nothing_to_draw(sheets, setup.scale)?;
    let m = setup.margin_mm;
    let printable = (
        setup.paper.width_mm - 2.0 * m,
        setup.paper.height_mm - 2.0 * m,
    );
    if printable.0.min(printable.1) < CALIBRATION_COLUMN_MM || printable.0.max(printable.1) < 150.0
    {
        return Err(format!(
            "a {} x {} mm page with {m} mm margins has no room for the 100 mm rule that proves \
             the print's size: use paper at least {} x {} mm",
            setup.paper.width_mm,
            setup.paper.height_mm,
            CALIBRATION_COLUMN_MM + 2.0 * m,
            150.0 + 2.0 * m
        ));
    }
    let total = page_count(sheets, setup);
    if total > MAX_TILED_PAGES {
        return Err(format!(
            "the patterns would take {total} pages at {}, and a tiled document is limited to \
             {MAX_TILED_PAGES}: use a smaller scale, larger paper, or DXF",
            scale_label(setup.scale)
        ));
    }

    let mut tilings = Vec::new();
    for sheet in sheets {
        match best_tiling(sheet, setup) {
            Ok(Some(t)) => tilings.push((sheet, t)),
            // Neither can happen past the checks above; said plainly if one does.
            _ => return Err(format!("sheet `{}` could not be laid out", sheet.name)),
        }
    }
    // The page each sheet starts on, counting from 1 with the calibration page first.
    let mut index = Vec::new();
    let mut next = 2;
    for (sheet, t) in &tilings {
        let pages = t.grid.printed.len() + usize::from(has_map(sheet, &t.grid));
        index.push(match pages {
            1 => format!("{}: page {next}", sheet.name),
            _ => format!("{}: pages {next} to {}", sheet.name, next + pages - 1),
        });
        next += pages;
    }

    let mut pdf = Pdf::new(&setup.title);
    let upright = (
        setup.paper.width_mm * PT_PER_MM,
        setup.paper.height_mm * PT_PER_MM,
    );
    pdf.add_page(upright.0, upright.1, calibration_page(setup, &index));
    let k = setup.scale * 1000.0;
    let margin_pt = m * PT_PER_MM;
    for (sheet, t) in &tilings {
        let grid = &t.grid;
        let (gw, gh) = grid.size_mm();
        let bbox = [0.0, 0.0, gw * PT_PER_MM, gh * PT_PER_MM];
        let origin = (t.corner_mm.0 / k, t.corner_mm.1 / k);
        let pattern = pdf.add_form(bbox, draw(sheet, setup.scale, origin));
        let joins = join_lines(grid).map(|c| pdf.add_form(bbox, c));
        if has_map(sheet, grid) {
            let first_tile = pdf.page_count() + 2;
            pdf.add_page(
                upright.0,
                upright.1,
                map_page(sheet, setup, grid, pattern, first_tile),
            );
        }
        let (pw, ph) = (
            grid.printable_mm.0 * PT_PER_MM,
            grid.printable_mm.1 * PT_PER_MM,
        );
        for (col, row) in &grid.printed {
            let window = tile_window_mm(grid, *col, *row);
            let mut c = Content::new();
            c.save();
            c.clip_rect(margin_pt, margin_pt, pw, ph);
            c.translate(
                margin_pt - window[0] * PT_PER_MM,
                margin_pt - window[1] * PT_PER_MM,
            );
            c.form(pattern);
            if let Some(joins) = joins {
                c.form(joins);
            }
            c.restore();
            let (x0, y0, x1, y1) = (margin_pt, margin_pt, margin_pt + pw, margin_pt + ph);
            c.polyline(&[(x0, y0), (x1, y0), (x1, y1), (x0, y1)], true, FAINT);
            let mut caption = vec![
                sheet.name.clone(),
                address(*col, *row),
                format!("page {} of {total}", pdf.page_count() + 1),
            ];
            if !setup.title.trim().is_empty() {
                caption.push(setup.title.trim().to_owned());
            }
            caption.push(scale_label(setup.scale));
            let at = (margin_pt + 2.0 * PT_PER_MM, margin_pt + 2.0 * PT_PER_MM);
            c.text(at, 6.0, 0.0, Anchor::Start, 0.5, &caption.join(" · "));
            let (width, height) = if grid.landscape {
                (upright.1, upright.0)
            } else {
                upright
            };
            pdf.add_page(width, height, c);
        }
    }
    pdf.finish()
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

    /// A label ending at the drawing's left edge, as a waterline's name does, is inside the
    /// drawing's room: its first letter is not before the row's left edge at x = 0.
    #[test]
    fn svg_makes_room_for_a_label_s_whole_text() {
        let mut sheet = rect(1.0, 0.5);
        let text = "WL 0.250 m";
        sheet.labels.push(Label {
            at: (0.0, 0.25),
            height_mm: 4.0,
            angle_deg: 0.0,
            anchor: Anchor::End,
            text: text.into(),
        });
        let out = svg(&[sheet], 1.0);
        let at = out.find("<text x=\"").unwrap() + "<text x=\"".len();
        let x: f64 = out[at..at + out[at..].find('"').unwrap()].parse().unwrap();
        let width = text_width_pt(text, 4.0 * PT_PER_MM) / PT_PER_MM;
        assert!(
            x - width >= -1e-3,
            "the text runs from {} to {x}",
            x - width
        );
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
    // ---- The tiled document ----

    fn line(length: f64) -> Sheet {
        let mut s = Sheet::new("Line");
        s.paths.push(SheetPath {
            kind: PathKind::Cut,
            points: vec![(0.0, 0.0), (length, 0.0)],
            closed: false,
        });
        s
    }

    fn squares() -> Sheet {
        let mut s = Sheet::new("Squares");
        for (x, y) in [(0.0, 0.0), (0.5, 0.6)] {
            s.paths.push(SheetPath {
                kind: PathKind::Cut,
                points: vec![(x, y), (x + 0.02, y), (x + 0.02, y + 0.02), (x, y + 0.02)],
                closed: true,
            });
        }
        s
    }

    /// The `dx dy` of the one translation in a tile page's stream.
    fn translation(stream: &[u8]) -> (f64, f64) {
        let s = text(stream);
        let line = s
            .lines()
            .find(|l| l.starts_with("1 0 0 1 ") && l.ends_with(" cm"))
            .unwrap();
        let mut it = line[8..line.len() - 3]
            .split_whitespace()
            .map(|v| v.parse::<f64>().unwrap());
        (it.next().unwrap(), it.next().unwrap())
    }

    #[test]
    fn rows_are_lettered() {
        assert_eq!(
            [0, 25, 26, 27, 51, 52, 701, 702].map(row_name),
            ["A", "Z", "AA", "AB", "AZ", "BA", "ZZ", "AAA"].map(String::from)
        );
    }

    /// A4 with 10 mm margin and overlap: printable 190 x 277, step 180 x 267. The drawing is padded
    /// by 5 mm a side, so 150 x 250 mm asks for ceil(150 / 180) = 1 column and ceil(250 / 267) = 1
    /// row. Worked by hand from the grid formula.
    #[test]
    fn a_small_sheet_is_one_tile_and_has_no_map() {
        let g = tile_grid(&rect(0.150, 0.250), &PageSetup::default()).unwrap();
        assert_eq!(
            (g.cols, g.rows, g.landscape, g.printed.len()),
            (1, 1, false, 1)
        );
        assert_eq!(
            (g.printable_mm, g.step_mm),
            ((190.0, 277.0), (180.0, 267.0))
        );
        assert_eq!(page_count(&[rect(0.150, 0.250)], &PageSetup::default()), 2); // calibration + tile
        assert_eq!(tile_grid(&Sheet::new("Empty"), &PageSetup::default()), None);
    }

    /// 400 x 250 mm: portrait ceil(400 / 180) x ceil(250 / 267) = 3 x 1; landscape (step 267 x 180)
    /// 2 x 2 = 4. 300 x 300 mm is 2 x 2 either way. 250 x 400 mm: portrait 2 x 2, landscape 1 x 3.
    #[test]
    fn the_orientation_with_fewer_tiles_wins() {
        let setup = PageSetup::default();
        let g = tile_grid(&rect(0.400, 0.250), &setup).unwrap();
        assert_eq!((g.cols, g.rows, g.landscape), (3, 1, false));
        assert_eq!(page_count(&[rect(0.400, 0.250)], &setup), 5); // calibration + map + 3
        let tie = tile_grid(&rect(0.300, 0.300), &setup).unwrap();
        assert_eq!((tie.cols, tie.rows, tie.landscape), (2, 2, false));
        let tall = tile_grid(&rect(0.250, 0.400), &setup).unwrap();
        assert_eq!((tall.cols, tall.rows, tall.landscape), (1, 3, true));
        assert_eq!(tall.printable_mm, (277.0, 190.0));
    }

    /// Bounds 520 x 620 mm, padded 530 x 630: 3 x 3 portrait tiles, a grid of 550 x 811 mm with the
    /// drawing's corner at (15, 95.5). The first square is in the lower-left tile, row C, and the
    /// second in the upper-right, row A; rows are printed from the top.
    #[test]
    fn a_tile_nothing_crosses_is_not_printed() {
        let g = tile_grid(&squares(), &PageSetup::default()).unwrap();
        assert_eq!((g.cols, g.rows, g.landscape), (3, 3, false));
        assert_eq!(g.printed, [(2, 0), (0, 2)]);
        assert_eq!(page_count(&[squares()], &PageSetup::default()), 4); // calibration, map, 2 tiles
    }

    /// On upright pages the squares' grid has columns at 0..190, 180..370 and 360..550 mm and the
    /// drawing's origin at (15, 95.5). A label at (300, 300) mm starts in the middle column of the
    /// middle row; one about 90 mm long runs on into the right-hand column.
    #[test]
    fn a_label_is_printed_with_the_tile_its_text_reaches() {
        let printed = |words: &str| {
            let mut s = squares();
            s.labels.push(Label {
                at: (0.30, 0.30),
                height_mm: 5.0,
                angle_deg: 0.0,
                anchor: Anchor::Start,
                text: words.into(),
            });
            let upright = tiling(&s, &PageSetup::default(), false)
                .ok()
                .flatten()
                .unwrap();
            upright.grid.printed
        };
        assert_eq!(printed("x"), [(2, 0), (1, 1), (0, 2)]);
        let long = "Waterline 0.300 m above the keel";
        assert!(text_width_pt(long, 5.0 * PT_PER_MM) / PT_PER_MM > 60.0);
        assert_eq!(printed(long), [(2, 0), (1, 1), (2, 1), (0, 2)]);
        assert_eq!(printed(""), [(2, 0), (0, 2)]);
    }

    /// With no overlap, a label whose base line is 2 mm below a row's top edge rises 3 mm into
    /// the row above: that tile is printed too, or the tops of the letters are lost.
    #[test]
    fn a_label_is_printed_with_the_tile_its_letters_rise_into() {
        let setup = PageSetup {
            overlap_mm: 0.0,
            ..PageSetup::default()
        };
        let bare = tiling(&squares(), &setup, false).ok().flatten().unwrap();
        // The middle column's tile on the top row: nothing of the squares reaches it.
        assert!(
            !bare.grid.printed.contains(&(1, 0)),
            "{:?}",
            bare.grid.printed
        );
        let below = tile_window_mm(&bare.grid, 1, 1);
        let base = ((below[0] + below[2]) / 2.0, below[3] - 2.0);
        let mut s = squares();
        s.labels.push(Label {
            at: (
                (base.0 + bare.corner_mm.0) / 1000.0,
                (base.1 + bare.corner_mm.1) / 1000.0,
            ),
            height_mm: 5.0,
            angle_deg: 0.0,
            anchor: Anchor::Start,
            text: "x".into(),
        });
        let printed = tiling(&s, &setup, false)
            .ok()
            .flatten()
            .unwrap()
            .grid
            .printed;
        assert!(printed.contains(&(1, 1)), "{printed:?}");
        assert!(printed.contains(&(1, 0)), "{printed:?}");
    }

    #[test]
    fn neighbours_share_exactly_the_overlap_and_cover_the_drawing() {
        for (sheet, setup) in [
            (rect(0.400, 0.250), PageSetup::default()),
            (
                squares(),
                PageSetup {
                    overlap_mm: 25.0,
                    margin_mm: 7.0,
                    paper: Paper::LETTER,
                    ..PageSetup::default()
                },
            ),
            (
                rect(3.0, 1.2),
                PageSetup {
                    scale: 0.5,
                    paper: Paper::A3,
                    ..PageSetup::default()
                },
            ),
        ] {
            let g = tile_grid(&sheet, &setup).unwrap();
            let overlap = setup.overlap_mm;
            for row in 0..g.rows {
                for col in 0..g.cols {
                    let w = tile_window_mm(&g, col, row);
                    assert!((w[2] - w[0] - g.printable_mm.0).abs() < 1e-9);
                    assert!((w[3] - w[1] - g.printable_mm.1).abs() < 1e-9);
                    if col + 1 < g.cols {
                        let right = tile_window_mm(&g, col + 1, row);
                        assert!((w[2] - right[0] - overlap).abs() < 1e-9, "{w:?} {right:?}");
                    }
                    if row + 1 < g.rows {
                        let below = tile_window_mm(&g, col, row + 1);
                        assert!(
                            (w[1] - (below[3] - overlap)).abs() < 1e-9,
                            "{w:?} {below:?}"
                        );
                    }
                }
            }
            // The windows together span the grid, which holds the drawing and 5 mm around it.
            let first = tile_window_mm(&g, 0, g.rows - 1);
            let last = tile_window_mm(&g, g.cols - 1, 0);
            assert_eq!((first[0], first[1]), (0.0, 0.0));
            let b = sheet.bounds().unwrap();
            let k = setup.scale * 1000.0;
            assert!(last[2] >= (b[2] - b[0]) * k + 10.0 - 1e-9);
            assert!(last[3] >= (b[3] - b[1]) * k + 10.0 - 1e-9);
        }
    }

    /// A 700 mm line: landscape pages (step 267 mm) take ceil(700 / 267) = 3 tiles in one row,
    /// portrait would take 4.
    #[test]
    fn the_drawing_and_the_join_grid_are_each_written_once() {
        let doc = pdf_tiled(&[line(0.7)], &PageSetup::default()).unwrap();
        assert_eq!(doc.pages, 5); // calibration, map, three tiles
        assert_eq!(text(&doc.bytes).matches("/Subtype /Form").count(), 2);
        for page in 2..5 {
            let stream = text(&page_stream(&doc.bytes, page));
            assert!(stream.contains("/X0 Do\n/X1 Do\n"), "page {page}: {stream}");
            assert!(text(&doc.bytes).contains("/MediaBox [0 0 841.890 595.276]"));
        }
        // The map shows the same drawing, scaled down.
        assert!(text(&page_stream(&doc.bytes, 1)).contains("/X0 Do\n"));
    }

    /// The pattern and the join lines are placed by one translation, so a cross is at the same
    /// place of the pattern on both pages. Neighbours differ by one step: 267 mm = 756.850 pt.
    #[test]
    fn a_join_cross_is_at_the_same_sheet_position_on_both_pages() {
        let doc = pdf_tiled(&[line(0.7)], &PageSetup::default()).unwrap();
        let moves: Vec<(f64, f64)> = (2..5)
            .map(|p| translation(&page_stream(&doc.bytes, p)))
            .collect();
        for page in 2..5 {
            let stream = text(&page_stream(&doc.bytes, page));
            assert_eq!(stream.matches(" cm\n").count(), 1, "{stream}");
            // Clipped to the printable area, 277 x 190 mm inside 10 mm margins.
            assert!(
                stream.starts_with("q\n28.346 28.346 785.197 538.583 re W n\n1 0 0 1 "),
                "{stream}"
            );
        }
        assert_eq!(moves[0], (28.346, 28.346));
        for pair in moves.windows(2) {
            assert!((pair[0].0 - pair[1].0 - 756.850).abs() < 0.002, "{moves:?}");
            assert_eq!(pair[0].1, pair[1].1);
        }
        // The join form has a line down the middle of each shared strip: x = 267 + 5 and 534 + 5 mm.
        let s = text(&doc.bytes);
        for x in ["771.024", "1527.874"] {
            assert!(s.contains(&format!("{x} 0.000 m\n{x} 538.583 l\n")), "{x}");
        }
        assert!(s.contains("0.283 w 0.500 G [2.835 2.835] 0 d"));
    }

    #[test]
    fn the_first_page_tells_the_reader_how_to_print() {
        let setup = PageSetup {
            title: "Kayık".into(),
            ..PageSetup::default()
        };
        let doc = pdf_tiled(&[rect(0.4, 0.25), line(0.7)], &setup).unwrap();
        let shown = page_text(&doc.bytes, 0);
        for line in CALIBRATION_LINES {
            assert!(shown.iter().any(|s| s == line), "{line}");
        }
        for line in ["Scale 1:1", crate::NOT_VALIDATED_NOTE, "Kayık", "100 mm"] {
            assert!(shown.iter().any(|s| s == line), "{line}: {shown:?}");
        }
        // The sheets and where they are: a map and three tiles each.
        assert!(shown.iter().any(|s| s == "Rect: pages 2 to 5"), "{shown:?}");
        assert!(shown.iter().any(|s| s == "Line: pages 6 to 9"), "{shown:?}");
        let stream = page_stream(&doc.bytes, 0);
        assert_eq!(rules(&segments(&stream), 283.465), (1, 1));
        assert!(text(&doc.bytes).contains("/MediaBox [0 0 595.276 841.890]"));
        assert!(doc.missing.is_empty());
    }

    /// 4 in is 288 pt exactly.
    #[test]
    fn imperial_adds_the_inch_rule_and_changes_nothing_else() {
        let sheets = [rect(0.4, 0.25), squares()];
        let metric = pdf_tiled(&sheets, &PageSetup::default()).unwrap();
        let both = pdf_tiled(
            &sheets,
            &PageSetup {
                imperial: true,
                ..PageSetup::default()
            },
        )
        .unwrap();
        assert_eq!(metric.pages, both.pages);
        assert!(page_text(&both.bytes, 0).iter().any(|s| s == "4 in"));
        assert_eq!(
            rules(&segments(&page_stream(&both.bytes, 0)), 288.0),
            (1, 1)
        );
        assert_eq!(
            rules(&segments(&page_stream(&both.bytes, 0)), 283.465),
            (1, 1)
        );
        assert!(!page_text(&metric.bytes, 0)
            .iter()
            .any(|s| s.contains("4 in")));
        assert_eq!(
            rules(&segments(&page_stream(&metric.bytes, 0)), 288.0),
            (0, 0)
        );
        for page in 1..metric.pages {
            assert_eq!(
                page_stream(&metric.bytes, page),
                page_stream(&both.bytes, page),
                "page {page}"
            );
        }
    }

    #[test]
    fn tiles_are_captioned() {
        let mut sheet = rect(0.150, 0.250);
        sheet.name = "Station 1".into();
        let setup = PageSetup {
            title: "Demo".into(),
            ..PageSetup::default()
        };
        let doc = pdf_tiled(&[sheet.clone()], &setup).unwrap();
        assert_eq!(
            page_text(&doc.bytes, 1),
            ["Station 1 · A1 · page 2 of 2 · Demo · 1:1"]
        );
        // Without a title the caption has one part fewer.
        let doc = pdf_tiled(
            &[sheet],
            &PageSetup {
                scale: 0.5,
                ..PageSetup::default()
            },
        )
        .unwrap();
        assert_eq!(
            page_text(&doc.bytes, 1),
            ["Station 1 · A1 · page 2 of 2 · 1:2"]
        );
        // The squares are in the top row's third column and the third row's first column.
        let doc = pdf_tiled(&[squares()], &PageSetup::default()).unwrap();
        assert_eq!(
            page_text(&doc.bytes, 2),
            ["Squares · A3 · page 3 of 4 · 1:1"]
        );
        assert_eq!(
            page_text(&doc.bytes, 3),
            ["Squares · C1 · page 4 of 4 · 1:1"]
        );
    }

    /// A sheet with notes always has its map page, so the notes are on paper even for one tile.
    #[test]
    fn a_sheet_with_notes_gets_a_map_page_that_shows_them() {
        let mut sheet = rect(0.150, 0.250);
        sheet.name = "Station 1".into();
        sheet.notes = vec![
            "Mould line 6.0 mm inside the hull surface".into(),
            "Largest bevel 7.1°".into(),
        ];
        let setup = PageSetup::default();
        assert_eq!(page_count(&[sheet.clone()], &setup), 3);
        let doc = pdf_tiled(&[sheet], &setup).unwrap();
        let map = page_text(&doc.bytes, 1);
        for line in [
            "Station 1",
            "Mould line 6.0 mm inside the hull surface",
            "Largest bevel 7.1°",
            "A1",
            "page 3",
        ] {
            assert!(map.iter().any(|s| s == line), "{line}: {map:?}");
        }
        assert_eq!(
            page_text(&doc.bytes, 2),
            ["Station 1 · A1 · page 3 of 3 · 1:1"]
        );
        // The map of the squares names only the tiles that are printed.
        let doc = pdf_tiled(&[squares()], &setup).unwrap();
        let map = page_text(&doc.bytes, 1);
        assert!(
            map.iter().any(|s| s == "A3") && map.iter().any(|s| s == "C1"),
            "{map:?}"
        );
        assert!(!map.iter().any(|s| s == "B2" || s == "A1"), "{map:?}");
    }

    #[test]
    fn landscape_tiles_swap_the_media_box() {
        let doc = pdf_tiled(&[rect(0.250, 0.150)], &PageSetup::default()).unwrap();
        assert_eq!(doc.pages, 2);
        let s = text(&doc.bytes);
        // The calibration page is upright and the tile is on its side.
        assert_eq!(s.matches("/MediaBox [0 0 595.276 841.890]").count(), 1);
        assert_eq!(s.matches("/MediaBox [0 0 841.890 595.276]").count(), 1);
    }

    /// 100 x 30 m at full size on A4, landscape: 375 x 167 tiles, and the outline alone crosses
    /// about 2 x 375 + 2 x 167 of them.
    #[test]
    fn a_ship_at_full_size_is_refused_with_the_count() {
        let setup = PageSetup::default();
        let ship = [rect(100.0, 30.0)];
        let count = page_count(&ship, &setup);
        assert!(count > 1050 && count < 1150, "{count}");
        let err = pdf_tiled(&ship, &setup).unwrap_err();
        for needle in [count.to_string().as_str(), "1000", "scale", "DXF"] {
            assert!(err.contains(needle), "{needle}: {err}");
        }
        // At 1:50 it is a 2000 x 600 mm drawing: fine.
        assert!(pdf_tiled(
            &ship,
            &PageSetup {
                scale: 0.02,
                ..setup.clone()
            }
        )
        .is_ok());
        // Something the size of a country is refused without counting its tiles one by one.
        let err = pdf_tiled(&[rect(1.0e6, 1.0e6)], &setup).unwrap_err();
        assert!(err.contains("scale") && err.contains("DXF"), "{err}");
        assert!(page_count(&[rect(1.0e6, 1.0e6)], &setup) > 1_000_000);
    }

    #[test]
    fn the_count_matches_the_document() {
        let setup = PageSetup::default();
        let mut noted = rect(0.15, 0.25);
        noted.notes = vec!["A note".into()];
        for sheets in [
            vec![rect(0.150, 0.250)],
            vec![rect(0.400, 0.250), squares(), line(0.7)],
            vec![noted, rect(0.25, 0.15)],
        ] {
            assert_eq!(
                page_count(&sheets, &setup),
                pdf_tiled(&sheets, &setup).unwrap().pages
            );
        }
        assert_eq!(page_count(&[], &setup), 0);
        assert_eq!(
            page_count(
                &[rect(1.0, 1.0)],
                &PageSetup {
                    scale: 0.0,
                    ..setup
                }
            ),
            0
        );
    }

    #[test]
    fn a_tiled_document_needs_something_to_draw_and_room_for_the_rules() {
        let setup = PageSetup::default();
        for sheets in [
            vec![],
            vec![Sheet::new("Empty")],
            vec![rect(1.0, 1.0), Sheet::new("Empty")],
        ] {
            let err = pdf_tiled(&sheets, &setup).unwrap_err();
            assert!(err.contains("nothing to draw"), "{err}");
        }
        // 100 x 150 mm paper cannot hold a 100 mm rule inside its margins.
        let small = PageSetup {
            paper: Paper {
                width_mm: 100.0,
                height_mm: 150.0,
            },
            ..PageSetup::default()
        };
        let err = pdf_tiled(&[rect(0.1, 0.1)], &small).unwrap_err();
        assert!(err.contains("100 mm rule"), "{err}");
        assert!(pdf_tiled(
            &[rect(0.1, 0.1)],
            &PageSetup {
                scale: 20.0,
                ..PageSetup::default()
            }
        )
        .is_err());
    }
}
