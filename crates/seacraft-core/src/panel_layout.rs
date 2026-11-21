//! Plywood panels nested onto sheets: each panel made into the pieces to cut (a copy, a turned
//! over copy, or the parts of one split to fit a sheet), the pieces packed by
//! [`crate::nest::pack`], and each sheet or blank drawn full size with its pieces on it.
//!
//! The layout is good and repeatable, not the fewest possible sheets.

use serde::{Deserialize, Serialize};

use crate::flatten::boundary_loops;
use crate::nest::{bbox, inside, pack, signed_area, Bin, Item, Pose, Pt};
use crate::panel_dims::is_offset_label;
use crate::panels::{FlatPanel, Stock, MAX_SHEETS_EACH};
use crate::pdf::Anchor;
use crate::sheet::{Label, PathKind, Sheet, SheetPath};
use crate::units::millimetres;

/// How the pieces are put on the plywood.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NestMode {
    /// No layout: the patterns alone, as before.
    #[default]
    None,
    /// Sheets joined end to end into blanks long enough for the longest piece.
    Blanks,
    /// A piece longer than a sheet cut into parts at straight joints.
    Split,
}

impl NestMode {
    pub const ALL: [NestMode; 3] = [NestMode::None, NestMode::Blanks, NestMode::Split];

    pub fn key(self) -> &'static str {
        match self {
            NestMode::None => "none",
            NestMode::Blanks => "blanks",
            NestMode::Split => "split",
        }
    }

    pub fn parse(s: &str) -> Result<NestMode, String> {
        let t = s.trim().to_ascii_lowercase();
        NestMode::ALL
            .into_iter()
            .find(|m| m.key() == t)
            .ok_or_else(|| format!("takes none, blanks or split, not `{s}`"))
    }

    /// How a count line names the layout.
    fn words(self) -> &'static str {
        match self {
            NestMode::None => "no nesting",
            NestMode::Blanks => "joined blanks",
            NestMode::Split => "split parts",
        }
    }
}

/// Nesting asked for: how, and the gap kept between pieces and from the sheet edges.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Nesting {
    pub mode: NestMode,
    pub gap_m: f64,
}

/// One piece on the layout.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct PlacedPiece {
    /// `Side`, `Side, turned over`, `Side 1 of 2`.
    pub label: String,
    pub panel: String,
    /// Index into [`Layout::sheets`].
    pub sheet: usize,
    pub pose: Pose,
    /// The piece's outline where it lies on its sheet, metres.
    #[serde(skip)]
    pub outline: Vec<Pt>,
}

/// A sheet or a blank of sheets joined end to end.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct LayoutSheet {
    pub name: String,
    pub length_m: f64,
    pub width_m: f64,
    pub sheets_joined: usize,
    /// Where each joint is from the blank's aft end, metres.
    pub joints_m: Vec<f64>,
}

/// A piece that is not on the layout, and why.
#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct NotPlaced {
    pub piece: String,
    pub reason: String,
}

/// The pieces nested on the plywood.
#[derive(Clone, Debug, Serialize)]
pub struct Layout {
    pub mode: NestMode,
    pub gap_m: f64,
    pub sheets: Vec<LayoutSheet>,
    pub pieces: Vec<PlacedPiece>,
    pub not_placed: Vec<NotPlaced>,
    /// Sheets of plywood: a blank of two counts two.
    pub plywood_sheets: usize,
    /// One drawing per sheet or blank, full size.
    #[serde(skip)]
    pub drawings: Vec<Sheet>,
}

impl Layout {
    /// The plywood count in words.
    pub fn count_line(&self) -> String {
        let sheets = if self.plywood_sheets == 1 {
            "sheet"
        } else {
            "sheets"
        };
        let mut line = format!(
            "{} plywood {sheets}, nested ({}), gap {} mm",
            self.plywood_sheets,
            self.mode.words(),
            millimetres(self.gap_m)
        );
        if !self.not_placed.is_empty() {
            let names: Vec<String> = self
                .not_placed
                .iter()
                .map(|n| format!("{} ({})", n.piece, n.reason))
                .collect();
            line.push_str(&format!("; not placed: {}", names.join(", ")));
        }
        line
    }
}

/// A piece to cut, in its pattern's frame before it is placed.
#[derive(Clone, Debug)]
pub(crate) struct Piece {
    pub label: String,
    pub panel: String,
    pub outline: Vec<Pt>,
    /// Its outline to cut, the marks to transfer and the words, in the same frame.
    pub drawing: Sheet,
}

/// How far an alignment mark reaches either side of a split joint, metres.
const JOINT_MARK_M: f64 = 0.015;
/// How far a joint moves at a time to get off a place the outline crosses more than twice.
const JOINT_STEP_M: f64 = 0.01;
/// Keep a joint this far from any outline point's x, so it never runs along an edge.
const JOINT_CLEAR_M: f64 = 1e-6;
/// Letter height of a piece's name on a layout, millimetres.
const PIECE_LABEL_MM: f64 = 5.0;
/// Letter height of a joint mark's letter, millimetres.
const JOINT_LABEL_MM: f64 = 3.0;

/// Where the square line at `x` crosses a closed outline, lowest first.
fn crossings(outline: &[Pt], x: f64) -> Vec<f64> {
    let n = outline.len();
    let mut ys: Vec<f64> = (0..n)
        .filter_map(|i| {
            let (p, q) = (outline[i], outline[(i + 1) % n]);
            ((p.0 >= x) != (q.0 >= x)).then(|| p.1 + (x - p.0) / (q.0 - p.0) * (q.1 - p.1))
        })
        .collect();
    ys.sort_by(f64::total_cmp);
    ys
}

/// The panel's outer outline, in its pattern's frame.
fn outline(panel: &FlatPanel) -> Vec<Pt> {
    boundary_loops(&panel.found.tris)
        .into_iter()
        .map(|lp| lp.iter().map(|&i| panel.xy[i]).collect::<Vec<Pt>>())
        .max_by(|a, b| signed_area(a).abs().total_cmp(&signed_area(b).abs()))
        .unwrap_or_default()
}

/// Where a panel is cut to fit sheets `usable_m` long: the x of each joint, in the pattern's
/// frame. Equal parts, as few as fit; a joint where the outline is crossed more than twice moves
/// aft, then forward, in 10 mm steps to the nearest place it is crossed twice. Empty when the
/// panel fits whole.
pub fn split_points(panel: &FlatPanel, usable_m: f64) -> Result<Vec<f64>, String> {
    split_outline(&outline(panel), usable_m)
}

fn split_outline(outline: &[Pt], usable: f64) -> Result<Vec<f64>, String> {
    if !(usable.is_finite() && usable > 0.0) {
        return Err("the sheet leaves no room inside the gaps".into());
    }
    let b = bbox(outline);
    let (x0, x1) = (b[0], b[2]);
    let length = x1 - x0;
    if length <= usable + 1e-12 {
        return Ok(Vec::new());
    }
    let n = (length / usable - 1e-12).ceil() as usize;
    if n > MAX_SHEETS_EACH {
        return Err(format!(
            "would take more than {MAX_SHEETS_EACH} parts: not split"
        ));
    }
    let clear_of_points = |x: f64| outline.iter().all(|p| (p.0 - x).abs() > JOINT_CLEAR_M);
    let mut joints = Vec::new();
    let mut prev = x0;
    for k in 1..n {
        let ideal = x0 + length * k as f64 / n as f64;
        let reach = (length / n as f64).max(JOINT_STEP_M);
        let steps = (reach / JOINT_STEP_M).ceil() as usize;
        let fits = |x: f64| {
            x - prev <= usable + 1e-12
                && x > prev + 1e-12
                && x1 - x <= (n - k) as f64 * usable + 1e-12
                && clear_of_points(x)
                && crossings(outline, x).len() == 2
        };
        let lo = (x1 - (n - k) as f64 * usable).max(prev);
        let hi = (prev + usable).min(x1);
        let found = (0..=steps)
            .flat_map(|j| {
                let d = j as f64 * JOINT_STEP_M;
                [ideal - d, ideal + d]
            })
            .find(|&x| fits(x))
            .or_else(|| {
                // Exact fallback over open sub-intervals between outline vertex x-coordinates in
                // [lo, hi], where the crossing count is piecewise constant.
                let mut cuts: Vec<f64> = vec![lo, hi];
                for p in outline {
                    if p.0 > lo && p.0 < hi {
                        cuts.push(p.0);
                    }
                }
                cuts.sort_by(f64::total_cmp);
                let mut candidates: Vec<f64> = Vec::new();
                let margin = 2.0 * JOINT_CLEAR_M;
                for w in cuts.windows(2) {
                    let (a, b) = (w[0], w[1]);
                    let x = if b - a > 2.0 * margin {
                        ideal.clamp(a + margin, b - margin)
                    } else {
                        (a + b) / 2.0
                    };
                    if fits(x) {
                        candidates.push(x);
                    }
                }
                candidates.sort_by(|a, b| {
                    (a - ideal)
                        .abs()
                        .total_cmp(&(b - ideal).abs())
                        .then_with(|| a.total_cmp(b))
                });
                candidates.into_iter().next()
            });
        let Some(x) = found else {
            return Err("no straight joint found".into());
        };
        joints.push(x);
        prev = x;
    }
    Ok(joints)
}

/// Sutherland–Hodgman: the part of a closed outline with `x ≥ a` (`keep_above`) or `x ≤ a`.
fn clip_half(poly: &[Pt], a: f64, keep_above: bool) -> Vec<Pt> {
    let inside = |p: Pt| if keep_above { p.0 >= a } else { p.0 <= a };
    let cut = |p: Pt, q: Pt| (a, p.1 + (a - p.0) / (q.0 - p.0) * (q.1 - p.1));
    let n = poly.len();
    let mut out = Vec::new();
    for i in 0..n {
        let (p, q) = (poly[i], poly[(i + 1) % n]);
        match (inside(p), inside(q)) {
            (true, true) => out.push(q),
            (true, false) => out.push(cut(p, q)),
            (false, true) => {
                out.push(cut(p, q));
                out.push(q);
            }
            (false, false) => {}
        }
    }
    out
}

fn clip_slab(poly: &[Pt], a: f64, b: f64) -> Vec<Pt> {
    clip_half(&clip_half(poly, a, true), b, false)
}

/// The runs of a polyline inside `a ≤ x ≤ b`.
fn clip_polyline(points: &[Pt], closed: bool, a: f64, b: f64) -> Vec<Vec<Pt>> {
    let mut pts = points.to_vec();
    if closed && !points.is_empty() {
        pts.push(points[0]);
    }
    let mut runs: Vec<Vec<Pt>> = Vec::new();
    let mut run: Vec<Pt> = Vec::new();
    for w in pts.windows(2) {
        let (p, q) = (w[0], w[1]);
        let (mut t0, mut t1) = (0.0_f64, 1.0_f64);
        let dx = q.0 - p.0;
        if dx.abs() < 1e-15 {
            if p.0 < a || p.0 > b {
                t0 = 1.0;
                t1 = 0.0;
            }
        } else {
            let (ta, tb) = ((a - p.0) / dx, (b - p.0) / dx);
            t0 = t0.max(ta.min(tb));
            t1 = t1.min(ta.max(tb));
        }
        if t0 > t1 {
            if run.len() >= 2 {
                runs.push(std::mem::take(&mut run));
            }
            run.clear();
            continue;
        }
        let at = |t: f64| (p.0 + t * dx, p.1 + t * (q.1 - p.1));
        let (s, e) = (at(t0), at(t1));
        if run.last() != Some(&s) {
            if run.len() >= 2 {
                runs.push(std::mem::take(&mut run));
            }
            run = vec![s];
        }
        run.push(e);
        if t1 < 1.0 {
            if run.len() >= 2 {
                runs.push(std::mem::take(&mut run));
            }
            run.clear();
        }
    }
    if run.len() >= 2 {
        runs.push(run);
    }
    runs
}

/// A spot inside an outline for its name: the middle of its box, or failing that the middle of
/// the first span of the square line through it.
fn name_spot(outline: &[Pt]) -> Pt {
    let b = bbox(outline);
    let c = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
    if inside(c, outline) {
        return c;
    }
    match crossings(outline, c.0).as_slice() {
        [lo, hi, ..] => (c.0, (lo + hi) / 2.0),
        _ => c,
    }
}

fn mirror_label(l: &Label) -> Label {
    Label {
        at: (l.at.0, -l.at.1),
        angle_deg: -l.angle_deg,
        ..l.clone()
    }
}

/// A piece's drawing from its panel's pattern: its outline to cut, the pattern's marks inside
/// `a ≤ x ≤ b`, and the pattern's words inside the outline, without the pattern's own name,
/// its copies line, its baseline offset ordinates, its reference lines (measurements, sheet
/// outlines) or its notes.
fn piece_drawing(
    pattern: &Sheet,
    panel: &str,
    outline: &[Pt],
    a: f64,
    b: f64,
    label: &str,
) -> Sheet {
    let mut s = Sheet::new(label);
    s.paths.push(SheetPath {
        kind: PathKind::Cut,
        points: outline.to_vec(),
        closed: true,
    });
    for p in &pattern.paths {
        if p.kind != PathKind::Mark {
            continue;
        }
        for run in clip_polyline(&p.points, p.closed, a, b) {
            s.paths.push(SheetPath {
                kind: PathKind::Mark,
                points: run,
                closed: false,
            });
        }
    }
    for l in &pattern.labels {
        let own_name =
            l.text == panel || l.text.starts_with("Cut 1") || l.text.starts_with("Cut 2");
        if !own_name && !is_offset_label(l) && l.at.0 >= a && l.at.0 <= b && inside(l.at, outline) {
            s.labels.push(l.clone());
        }
    }
    s.labels.push(Label {
        at: name_spot(outline),
        height_mm: PIECE_LABEL_MM,
        angle_deg: 0.0,
        anchor: Anchor::Middle,
        text: label.to_owned(),
    });
    s
}

/// Distance from `(x, y)` in direction `dir` (+1 or -1 along x) to the first edge of `outline`.
fn horizontal_clearance(outline: &[Pt], x: f64, y: f64, dir: f64) -> f64 {
    let n = outline.len();
    let mut best = f64::INFINITY;
    for i in 0..n {
        let (p, q) = (outline[i], outline[(i + 1) % n]);
        if (p.1 >= y) != (q.1 >= y) {
            let ix = p.0 + (y - p.1) / (q.1 - p.1) * (q.0 - p.0);
            let dist = (ix - x) * dir;
            if dist > 1e-9 {
                best = best.min(dist);
            }
        }
    }
    best
}

/// The marks across a split joint at `x`, on the part before it (`after` false) or after it.
/// Each tick and its letter are clipped to the available material in direction `s` so tapered
/// edges never leave marks floating outside the cut outline.
fn joint_marks(
    outline: &[Pt],
    x: f64,
    first_letter: u8,
    after: bool,
) -> (Vec<SheetPath>, Vec<Label>) {
    let ys = crossings(outline, x);
    let (lo, hi) = match ys.as_slice() {
        [lo, hi] => (*lo, *hi),
        _ => return (Vec::new(), Vec::new()),
    };
    let s = if after { 1.0 } else { -1.0 };
    let mut paths = Vec::new();
    let mut labels = Vec::new();
    for (k, f) in [0.25, 0.75].into_iter().enumerate() {
        let y = lo + f * (hi - lo);
        let avail = horizontal_clearance(outline, x, y, s);
        let (tick_len, label_off) = if avail >= JOINT_MARK_M + 0.003 {
            (JOINT_MARK_M, JOINT_MARK_M + 0.003)
        } else if avail.is_finite() && avail > 1e-6 {
            let tick = JOINT_MARK_M.min(avail * 0.65);
            let off = (tick + 0.0015).min(avail * 0.9);
            (tick, off)
        } else {
            (JOINT_MARK_M, JOINT_MARK_M + 0.003)
        };
        paths.push(SheetPath {
            kind: PathKind::Mark,
            points: vec![(x, y), (x + s * tick_len, y)],
            closed: false,
        });
        labels.push(Label {
            at: (x + s * label_off, y),
            height_mm: JOINT_LABEL_MM,
            angle_deg: 0.0,
            anchor: if after { Anchor::Start } else { Anchor::End },
            text: ((first_letter + k as u8) as char).to_string(),
        });
    }
    (paths, labels)
}

/// The letter after `n` pairs of joint marks: A, C, E … up to Y, then round again.
fn joint_letter(n: usize) -> u8 {
    b'A' + ((2 * n) % 26) as u8
}

/// The pieces to cut from every panel: each copy, a mirrored pair's second copy turned over,
/// and with [`NestMode::Split`] a piece longer than `usable_m` cut into parts. A panel that
/// cannot be split is returned with why.
pub(crate) fn pieces(
    panels: &[FlatPanel],
    patterns: &[Sheet],
    mode: NestMode,
    usable_m: f64,
) -> (Vec<Piece>, Vec<NotPlaced>) {
    let mut out = Vec::new();
    let mut refused = Vec::new();
    let mut joint_pairs = 0usize;
    for (panel, pattern) in panels.iter().zip(patterns) {
        let name = panel.found.name.clone();
        let base = outline(panel);
        if base.len() < 3 {
            refused.push(NotPlaced {
                piece: name,
                reason: "its outline could not be followed".into(),
            });
            continue;
        }
        let mirrored_pattern = Sheet {
            paths: pattern
                .paths
                .iter()
                .map(|p| SheetPath {
                    points: p.points.iter().map(|q| (q.0, -q.1)).collect(),
                    ..p.clone()
                })
                .collect(),
            labels: pattern.labels.iter().map(mirror_label).collect(),
            ..pattern.clone()
        };
        let copies = panel.found.copies.count();
        for copy in 0..copies {
            let (outline, pattern, label) = if copy == 0 {
                (base.clone(), pattern, name.clone())
            } else {
                (
                    base.iter().map(|q| (q.0, -q.1)).collect::<Vec<Pt>>(),
                    &mirrored_pattern,
                    format!("{name}, turned over"),
                )
            };
            let joints = if mode == NestMode::Split {
                match split_outline(&outline, usable_m) {
                    Ok(j) => j,
                    Err(why) => {
                        refused.push(NotPlaced {
                            piece: label,
                            reason: why,
                        });
                        continue;
                    }
                }
            } else {
                Vec::new()
            };
            if joints.is_empty() {
                let drawing = piece_drawing(pattern, &name, &outline, f64::MIN, f64::MAX, &label);
                out.push(Piece {
                    label,
                    panel: name.clone(),
                    outline,
                    drawing,
                });
                continue;
            }
            let b = bbox(&outline);
            let mut edges = vec![b[0]];
            edges.extend(&joints);
            edges.push(b[2]);
            let n = edges.len() - 1;
            let first_pair = joint_pairs;
            joint_pairs += joints.len();
            for i in 0..n {
                let (a, z) = (edges[i], edges[i + 1]);
                let part = clip_slab(&outline, a, z);
                let part_label = format!("{label} {} of {n}", i + 1);
                let mut drawing = piece_drawing(pattern, &name, &part, a, z, &part_label);
                if i > 0 {
                    let (p, l) = joint_marks(&outline, a, joint_letter(first_pair + i - 1), true);
                    drawing.paths.extend(p);
                    drawing.labels.extend(l);
                }
                if i + 1 < n {
                    let (p, l) = joint_marks(&outline, z, joint_letter(first_pair + i), false);
                    drawing.paths.extend(p);
                    drawing.labels.extend(l);
                }
                out.push(Piece {
                    label: part_label,
                    panel: name.clone(),
                    outline: part,
                    drawing,
                });
            }
        }
    }
    (out, refused)
}

/// The fewest whole sheets joined end to end that hold `need` metres inside the gaps, and the
/// blank's length; `None` past [`MAX_SHEETS_EACH`].
fn blank_for(stock: Stock, gap: f64, need: f64) -> Option<(usize, f64)> {
    let reach = stock.length_m - stock.scarf_m;
    (1..=MAX_SHEETS_EACH)
        .map(|k| (k, stock.length_m + (k - 1) as f64 * reach))
        .find(|&(_, len)| len - 2.0 * gap >= need - 1e-12)
}

/// Moves a drawing as `pose` moves `outline`: half a turn and turning over about the outline,
/// then its box's low corner to the pose's point. Words stay the right way up.
fn posed(drawing: &Sheet, outline: &[Pt], pose: Pose) -> (Vec<SheetPath>, Vec<Label>) {
    let turn = |p: Pt| {
        let y = if pose.turned_over { -p.1 } else { p.1 };
        if pose.end_for_end {
            (-p.0, -y)
        } else {
            (p.0, y)
        }
    };
    let turned: Vec<Pt> = outline.iter().map(|&p| turn(p)).collect();
    let b = bbox(&turned);
    let map = |p: Pt| {
        let t = turn(p);
        (t.0 - b[0] + pose.x, t.1 - b[1] + pose.y)
    };
    let paths = drawing
        .paths
        .iter()
        .map(|p| SheetPath {
            points: p.points.iter().map(|&q| map(q)).collect(),
            ..p.clone()
        })
        .collect();
    let labels = drawing
        .labels
        .iter()
        .map(|l| {
            let a = l.angle_deg.to_radians();
            let d = turn((a.cos(), a.sin()));
            let mut angle = d.1.atan2(d.0).to_degrees();
            let mut anchor = l.anchor;
            if angle > 90.0 + 1e-9 || angle <= -90.0 + 1e-9 {
                angle = if angle > 0.0 {
                    angle - 180.0
                } else {
                    angle + 180.0
                };
                anchor = match anchor {
                    Anchor::Start => Anchor::End,
                    Anchor::End => Anchor::Start,
                    a => a,
                };
            }
            Label {
                at: map(l.at),
                angle_deg: angle,
                anchor,
                ..l.clone()
            }
        })
        .collect();
    (paths, labels)
}

/// Nests every panel's pieces on `stock` as `nesting` says; `patterns[i]` is `panels[i]`'s
/// full-size pattern.
pub fn nest_panels(
    panels: &[FlatPanel],
    patterns: &[Sheet],
    stock: Stock,
    nesting: Nesting,
) -> Layout {
    let gap = nesting.gap_m;
    let usable = stock.length_m - 2.0 * gap;
    let (pieces, mut not_placed) = pieces(panels, patterns, nesting.mode, usable);
    let items: Vec<Item> = pieces
        .iter()
        .map(|p| Item {
            outline: p.outline.clone(),
            may_turn_over: true,
        })
        .collect();
    let new_bin = |need: f64| -> Option<Bin> {
        let length = match nesting.mode {
            NestMode::Blanks => blank_for(stock, gap, need)?.1,
            _ => stock.length_m,
        };
        Some(Bin {
            length,
            width: stock.width_m,
        })
    };
    let packing = pack(&items, &new_bin, gap);
    let mut layout = Layout {
        mode: nesting.mode,
        gap_m: gap,
        sheets: Vec::new(),
        pieces: Vec::new(),
        not_placed: Vec::new(),
        plywood_sheets: 0,
        drawings: Vec::new(),
    };
    let reach = stock.length_m - stock.scarf_m;
    for (b, bin) in packing.bins.iter().enumerate() {
        let k = if nesting.mode == NestMode::Blanks && reach > 0.0 {
            (((bin.length - stock.length_m) / reach).round().max(0.0) as usize) + 1
        } else {
            1
        };
        let name = if k == 1 {
            format!("Sheet {}", b + 1)
        } else {
            format!("Blank {} ({k} sheets)", b + 1)
        };
        let joints_m: Vec<f64> = (1..k)
            .map(|m| stock.length_m + (m - 1) as f64 * reach)
            .collect();
        layout.plywood_sheets += k;
        let mut drawing = Sheet::new(name.clone());
        drawing.paths.push(SheetPath {
            kind: PathKind::Reference,
            points: vec![
                (0.0, 0.0),
                (bin.length, 0.0),
                (bin.length, bin.width),
                (0.0, bin.width),
            ],
            closed: true,
        });
        for &j in &joints_m {
            drawing.paths.push(SheetPath {
                kind: PathKind::Reference,
                points: vec![(j, 0.0), (j, bin.width)],
                closed: false,
            });
        }
        let mut what = format!(
            "{} × {} mm",
            millimetres(bin.length),
            millimetres(bin.width)
        );
        if k > 1 {
            let at: Vec<String> = joints_m.iter().map(|j| millimetres(*j)).collect();
            what.push_str(&format!(
                ", {k} sheets of {} × {} mm joined",
                millimetres(stock.length_m),
                millimetres(stock.width_m)
            ));
            if stock.scarf_m > 0.0 {
                what.push_str(&format!(" with {} mm scarfs", millimetres(stock.scarf_m)));
            }
            what.push_str(&format!(
                ", joints at {} mm from the aft end",
                at.join(", ")
            ));
        }
        drawing.notes.push(what);
        drawing.notes.push(format!(
            "Pieces nested automatically, {} mm apart and from the edges: a good layout, not the \
             fewest possible sheets",
            millimetres(gap)
        ));
        drawing
            .notes
            .push("Check every piece against its full-size pattern before cutting".into());
        drawing.notes.push(crate::panels::THICKNESS_NOTE.into());
        drawing.notes.push(crate::NOT_VALIDATED_NOTE.into());
        layout.drawings.push(drawing);
        layout.sheets.push(LayoutSheet {
            name,
            length_m: bin.length,
            width_m: bin.width,
            sheets_joined: k,
            joints_m,
        });
    }
    let mut placements = packing.placements.clone();
    placements.sort_by_key(|p| (p.bin, p.item));
    for pl in placements {
        let piece = &pieces[pl.item];
        let (paths, labels) = posed(&piece.drawing, &piece.outline, pl.pose);
        let drawing = &mut layout.drawings[pl.bin];
        drawing.paths.extend(paths);
        drawing.labels.extend(labels);
        layout.pieces.push(PlacedPiece {
            label: piece.label.clone(),
            panel: piece.panel.clone(),
            sheet: pl.bin,
            pose: pl.pose,
            outline: pl.pose.place(&piece.outline),
        });
    }
    not_placed.extend(packing.unplaced.into_iter().map(|(i, reason)| NotPlaced {
        piece: pieces[i].label.clone(),
        reason,
    }));
    layout.not_placed = not_placed;
    // Carry each failing panel's flatness warning onto every sheet holding one of its pieces.
    let failing_notes: Vec<(&str, &str)> = panels
        .iter()
        .zip(patterns)
        .filter(|(p, _)| !p.lies_flat)
        .filter_map(|(p, pat)| {
            pat.notes
                .iter()
                .find(|n| n.starts_with("Does not lie flat"))
                .map(|n| (p.found.name.as_str(), n.as_str()))
        })
        .collect();
    if !failing_notes.is_empty() {
        for (b, d) in layout.drawings.iter_mut().enumerate() {
            let mut warnings = Vec::new();
            for &(panel_name, note) in &failing_notes {
                if layout
                    .pieces
                    .iter()
                    .any(|p| p.sheet == b && p.panel == panel_name)
                {
                    warnings.push(format!("{panel_name}: {note}"));
                }
            }
            if !warnings.is_empty() {
                warnings.append(&mut d.notes);
                d.notes = warnings;
            }
        }
    }
    // The cut lines drawn last, over marks and references, as on paper.
    for d in &mut layout.drawings {
        d.paths.sort_by_key(|p| match p.kind {
            PathKind::Reference => 0,
            PathKind::Mark => 1,
            PathKind::Cut => 2,
        });
    }
    layout
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::examples::{build, Example};
    use crate::nest::min_distance;
    use crate::panels::{measure_panels, panel_offsets, panel_sheet, test_flat, Limits};
    use crate::subdiv::SubdivisionOptions;

    fn skiff() -> (Vec<FlatPanel>, Vec<Sheet>) {
        let (s, _) = build(Example::Skiff4m).expect("skiff");
        let layers = crate::panels::default_layers(&s);
        let (panels, _, refused) = measure_panels(
            &s,
            SubdivisionOptions::default(),
            &layers,
            &[],
            Limits::default(),
        );
        assert!(refused.is_empty());
        let patterns = panels
            .iter()
            .map(|p| panel_sheet(p, &panel_offsets(p, 0.25), &[], Limits::default(), false))
            .collect();
        (panels, patterns)
    }

    fn stock(l: f64, w: f64, scarf: f64) -> Stock {
        Stock {
            length_m: l,
            width_m: w,
            scarf_m: scarf,
        }
    }

    fn nested(mode: NestMode) -> Nesting {
        Nesting { mode, gap_m: 0.01 }
    }

    fn assert_sound(layout: &Layout) {
        let gap = layout.gap_m;
        for p in &layout.pieces {
            let s = &layout.sheets[p.sheet];
            for q in &p.outline {
                assert!(
                    q.0 >= gap - 1e-9
                        && q.0 <= s.length_m - gap + 1e-9
                        && q.1 >= gap - 1e-9
                        && q.1 <= s.width_m - gap + 1e-9,
                    "{} off {} by the gap at {q:?}",
                    p.label,
                    s.name
                );
            }
        }
        for (i, a) in layout.pieces.iter().enumerate() {
            for b in &layout.pieces[i + 1..] {
                if a.sheet == b.sheet {
                    let d = min_distance(&a.outline, &b.outline);
                    assert!(d >= gap - 1e-9, "{} and {} {d} m apart", a.label, b.label);
                }
            }
        }
    }

    #[test]
    fn blanks_hold_the_skiff_without_overlap() {
        let (panels, patterns) = skiff();
        let layout = nest_panels(
            &panels,
            &patterns,
            stock(3.0, 1.5, 0.0),
            nested(NestMode::Blanks),
        );
        let mut labels: Vec<&str> = layout.pieces.iter().map(|p| p.label.as_str()).collect();
        labels.sort();
        assert_eq!(labels, ["Bottom", "Side", "Side, turned over", "Transom"]);
        assert!(layout.not_placed.is_empty(), "{:?}", layout.not_placed);
        assert!(layout.sheets.len() <= 3, "{:?}", layout.sheets);
        assert_eq!(layout.drawings.len(), layout.sheets.len());
        assert_sound(&layout);
        println!("{}", layout.count_line());
    }

    #[test]
    fn a_blank_is_as_long_as_its_sheets_less_the_scarfs() {
        assert_eq!(blank_for(stock(3.0, 1.5, 0.15), 0.01, 4.4), Some((2, 5.85)));
        let (panels, patterns) = skiff();
        let layout = nest_panels(
            &panels,
            &patterns,
            stock(3.0, 1.5, 0.15),
            nested(NestMode::Blanks),
        );
        let blank = layout
            .sheets
            .iter()
            .find(|s| s.sheets_joined == 2)
            .expect("a two-sheet blank");
        assert!((blank.length_m - 5.85).abs() < 1e-12, "{blank:?}");
        assert_eq!(blank.joints_m, [3.0]);
        assert_eq!(blank.name, "Blank 1 (2 sheets)");
        assert_eq!(
            layout.plywood_sheets,
            layout.sheets.iter().map(|s| s.sheets_joined).sum::<usize>()
        );
    }

    fn side_parts(layout_pieces: &[Piece]) -> Vec<&Piece> {
        layout_pieces
            .iter()
            .filter(|p| p.panel == "Side" && !p.label.contains("turned over"))
            .collect()
    }

    #[test]
    fn split_parts_add_up_and_fit() {
        let (panels, patterns) = skiff();
        let usable = 2.44 - 0.02;
        let (ps, refused) = pieces(&panels, &patterns, NestMode::Split, usable);
        assert!(refused.is_empty(), "{refused:?}");
        let parts = side_parts(&ps);
        let labels: Vec<&str> = parts.iter().map(|p| p.label.as_str()).collect();
        assert_eq!(labels, ["Side 1 of 2", "Side 2 of 2"]);
        let side = panels.iter().find(|p| p.found.name == "Side").unwrap();
        let whole = bbox(&outline(side));
        let sum: f64 = parts
            .iter()
            .map(|p| {
                let b = bbox(&p.outline);
                b[2] - b[0]
            })
            .sum();
        assert!((sum - (whole[2] - whole[0])).abs() < 1e-9, "{sum}");
        for p in &parts {
            let b = bbox(&p.outline);
            assert!(b[2] - b[0] <= usable + 1e-9);
        }
        let layout = nest_panels(
            &panels,
            &patterns,
            stock(2.44, 1.22, 0.0),
            nested(NestMode::Split),
        );
        assert!(layout.not_placed.is_empty(), "{:?}", layout.not_placed);
        assert!(layout.sheets.iter().all(|s| s.sheets_joined == 1));
        assert_sound(&layout);
        println!("{}", layout.count_line());
    }

    /// The mark ticks that end on the joint at `x`, as (y, letter) pairs.
    fn marks_at(p: &Piece, x: f64) -> Vec<(i64, String)> {
        let mut out: Vec<(i64, String)> = p
            .drawing
            .paths
            .iter()
            .filter(|q| {
                q.kind == PathKind::Mark
                    && q.points.len() == 2
                    && (q.points[0].0 - x).abs() < 1e-12
                    && (q.points[0].1 - q.points[1].1).abs() < 1e-12
            })
            .map(|q| {
                let y = q.points[0].1;
                let letter = p
                    .drawing
                    .labels
                    .iter()
                    .filter(|l| {
                        l.text.len() == 1
                            && (l.at.0 - x).abs() < JOINT_MARK_M + 0.01
                            && (l.at.1 - y).abs() < 0.002
                    })
                    .map(|l| l.text.clone())
                    .next()
                    .unwrap_or_default();
                ((y * 1e9).round() as i64, letter)
            })
            .collect();
        out.sort();
        out
    }

    #[test]
    fn joint_marks_match() {
        let (panels, patterns) = skiff();
        let (ps, _) = pieces(&panels, &patterns, NestMode::Split, 2.42);
        let parts = side_parts(&ps);
        let joint = bbox(&parts[0].outline)[2];
        let before = marks_at(parts[0], joint);
        let after = marks_at(parts[1], joint);
        assert_eq!(before.len(), 2, "{before:?}");
        assert_eq!(before, after);
        // Letters run on across the set, so no two joints share one: the Bottom's comes first.
        assert_eq!(
            before.iter().map(|m| m.1.as_str()).collect::<Vec<_>>(),
            ["C", "D"]
        );
        let bottom: Vec<&Piece> = ps.iter().filter(|p| p.panel == "Bottom").collect();
        let j = bbox(&bottom[0].outline)[2];
        let letters: Vec<String> = marks_at(bottom[0], j).into_iter().map(|m| m.1).collect();
        assert_eq!(letters, ["A", "B"]);
    }

    /// A 4 m by 1 m slab with a slot 0.2 m high from its forward end back to 1.5 m: the square
    /// line is crossed four times from 1.5 m forward.
    fn slotted() -> FlatPanel {
        test_flat(
            "Slotted",
            vec![
                (0.0, 0.0),
                (1.5, 0.0),
                (4.0, 0.0),
                (4.0, 0.4),
                (1.5, 0.4),
                (1.5, 0.6),
                (4.0, 0.6),
                (4.0, 1.0),
                (1.5, 1.0),
                (0.0, 1.0),
            ],
            vec![
                [1, 2, 3],
                [1, 3, 4],
                [0, 1, 4],
                [0, 4, 5],
                [0, 5, 9],
                [5, 8, 9],
                [5, 6, 7],
                [5, 7, 8],
            ],
        )
    }

    #[test]
    fn a_joint_moves_off_a_triple_crossing() {
        let p = slotted();
        let joints = split_points(&p, 2.98).expect("a joint");
        assert_eq!(joints.len(), 1);
        assert!(joints[0] < 1.5 && joints[0] > 1.4, "{joints:?}");
        assert_eq!(crossings(&outline(&p), joints[0]).len(), 2);
        let pattern = panel_sheet(&p, &panel_offsets(&p, 0.25), &[], Limits::default(), false);
        let (ps, refused) = pieces(&[p], &[pattern], NestMode::Split, 2.98);
        assert!(refused.is_empty());
        assert_eq!(ps.len(), 2);
        let sum: f64 = ps
            .iter()
            .map(|q| {
                let b = bbox(&q.outline);
                b[2] - b[0]
            })
            .sum();
        assert!((sum - 4.0).abs() < 1e-9);
        let area: f64 = ps.iter().map(|q| signed_area(&q.outline).abs()).sum();
        assert!((area - (4.0 - 2.5 * 0.2)).abs() < 1e-9, "{area}");
        // Too short a sheet leaves no straight joint.
        assert_eq!(
            split_points(&slotted(), 2.4),
            Err("no straight joint found".to_string())
        );
    }

    #[test]
    fn the_turned_over_copy_is_the_mirror() {
        let (panels, patterns) = skiff();
        let (ps, _) = pieces(&panels, &patterns, NestMode::Blanks, 10.0);
        let first = ps.iter().find(|p| p.label == "Side").unwrap();
        let second = ps.iter().find(|p| p.label == "Side, turned over").unwrap();
        assert_eq!(first.outline.len(), second.outline.len());
        for (a, b) in first.outline.iter().zip(&second.outline) {
            assert!((a.0 - b.0).abs() < 1e-12 && (a.1 + b.1).abs() < 1e-12);
        }
    }

    #[test]
    fn layout_drawings_cut_only_the_pieces() {
        let (panels, patterns) = skiff();
        let layout = nest_panels(
            &panels,
            &patterns,
            stock(3.0, 1.5, 0.0),
            nested(NestMode::Blanks),
        );
        let cuts: usize = layout
            .drawings
            .iter()
            .map(|d| d.paths.iter().filter(|p| p.kind == PathKind::Cut).count())
            .sum();
        assert_eq!(cuts, layout.pieces.len());
        for (d, s) in layout.drawings.iter().zip(&layout.sheets) {
            assert_eq!(d.name, s.name);
            let b = d.bounds().unwrap();
            assert!(b[0] >= -1e-9 && b[1] >= -1e-9);
            assert!(
                b[2] <= s.length_m + 1e-9 && b[3] <= s.width_m + 1e-9,
                "{b:?}"
            );
        }
    }

    #[test]
    fn nest_words_parse() {
        assert_eq!(NestMode::parse(" Blanks "), Ok(NestMode::Blanks));
        assert_eq!(NestMode::parse("split"), Ok(NestMode::Split));
        assert_eq!(NestMode::parse("none"), Ok(NestMode::None));
        assert_eq!(
            NestMode::parse("tetris"),
            Err("takes none, blanks or split, not `tetris`".into())
        );
    }

    #[test]
    fn tolerance_boundary_pieces_still_place() {
        let st = stock(2.44, 1.22, 0.0);
        let gap = 0.01;
        let usable = st.length_m - 2.0 * gap;
        // Length strictly between usable + 1e-12 and usable + 1e-9: must not fall between
        // panel_layout's and nest's tolerances.
        let len = usable + 5e-10;
        let p = test_flat(
            "Edge",
            vec![(0.0, 0.0), (len, 0.0), (len, 0.5), (0.0, 0.5)],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        let pattern = panel_sheet(&p, &panel_offsets(&p, 0.25), &[], Limits::default(), false);
        for mode in [NestMode::Split, NestMode::Blanks] {
            let layout = nest_panels(
                std::slice::from_ref(&p),
                std::slice::from_ref(&pattern),
                st,
                Nesting { mode, gap_m: gap },
            );
            assert!(
                layout.not_placed.is_empty(),
                "{mode:?}: {:?}",
                layout.not_placed
            );
            assert!(!layout.pieces.is_empty());
            assert_sound(&layout);
        }
    }

    #[test]
    fn three_way_split_and_mirrored_label_orientation() {
        let p = test_flat(
            "Long",
            vec![(0.0, 0.0), (6.5, 0.0), (6.5, 0.4), (0.0, 0.4)],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        let pattern = panel_sheet(&p, &panel_offsets(&p, 0.25), &[], Limits::default(), false);
        let (ps, refused) = pieces(
            std::slice::from_ref(&p),
            std::slice::from_ref(&pattern),
            NestMode::Split,
            2.42,
        );
        assert!(refused.is_empty(), "{refused:?}");
        let labels: Vec<&str> = ps.iter().map(|q| q.label.as_str()).collect();
        assert_eq!(labels, ["Long 1 of 3", "Long 2 of 3", "Long 3 of 3"]);
        let sum: f64 = ps
            .iter()
            .map(|q| {
                let b = bbox(&q.outline);
                b[2] - b[0]
            })
            .sum();
        assert!((sum - 6.5).abs() < 1e-9, "{sum}");
        let j1 = bbox(&ps[0].outline)[2];
        let j2 = bbox(&ps[1].outline)[2];
        assert_eq!(
            marks_at(&ps[0], j1)
                .into_iter()
                .map(|m| m.1)
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(
            marks_at(&ps[1], j1)
                .into_iter()
                .map(|m| m.1)
                .collect::<Vec<_>>(),
            ["A", "B"]
        );
        assert_eq!(
            marks_at(&ps[1], j2)
                .into_iter()
                .map(|m| m.1)
                .collect::<Vec<_>>(),
            ["C", "D"]
        );
        assert_eq!(
            marks_at(&ps[2], j2)
                .into_iter()
                .map(|m| m.1)
                .collect::<Vec<_>>(),
            ["C", "D"]
        );

        // Nested skiff sheets: every label stays readable in (-90°, 90°] and carries no orphaned
        // baseline offset ordinates.
        let (panels, patterns) = skiff();
        let layout = nest_panels(
            &panels,
            &patterns,
            stock(2.44, 1.22, 0.0),
            nested(NestMode::Split),
        );
        assert!(layout.not_placed.is_empty(), "{:?}", layout.not_placed);
        for d in &layout.drawings {
            for l in &d.labels {
                assert!(
                    l.angle_deg > -90.0 - 1e-9 && l.angle_deg <= 90.0 + 1e-9,
                    "upside-down label on {}: {l:?}",
                    d.name
                );
                assert!(
                    !is_offset_label(l),
                    "orphaned offset ordinate on {}: {l:?}",
                    d.name
                );
            }
        }
    }

    #[test]
    fn split_finds_a_joint_inside_a_narrow_feasible_interval() {
        // 4.838 m panel on 2.420 m usable length -> feasible joint range [2.418, 2.420].
        // A vertex at the ideal 2.419 m rejects the ideal and both 10 mm steps lie outside the
        // feasible range; the exact sub-interval fallback must still find a valid joint.
        let p = test_flat(
            "Narrow",
            vec![
                (0.0, 0.0),
                (2.419, 0.0),
                (4.838, 0.0),
                (4.838, 0.5),
                (2.419, 0.5),
                (0.0, 0.5),
            ],
            vec![[0, 1, 4], [0, 4, 5], [1, 2, 3], [1, 3, 4]],
        );
        let joints = split_points(&p, 2.420).expect("a joint inside [2.418, 2.420]");
        assert_eq!(joints.len(), 1);
        assert!(joints[0] >= 2.418 && joints[0] <= 2.420, "{joints:?}");
        assert!((joints[0] - 2.419).abs() > JOINT_CLEAR_M);
    }

    #[test]
    fn split_joint_marks_stay_inside_tapered_material() {
        // Tapered strip rising steeply after x = 1.0: at x = 1.0 the span is 4 mm ([0.0, 0.004]),
        // and both edges rise 1 mm per 10 mm so the lower mark (y = 0.001) hits the bottom edge
        // at dx = 10 mm (< 15 mm JOINT_MARK_M).
        let outline = vec![
            (0.0, -0.1),
            (1.0, 0.0),
            (2.0, 0.1),
            (2.0, 0.104),
            (1.0, 0.004),
            (0.0, -0.096),
        ];
        for after in [false, true] {
            let (paths, labels) = joint_marks(&outline, 1.0, b'A', after);
            assert_eq!(paths.len(), 2);
            assert_eq!(labels.len(), 2);
            for path in &paths {
                let tip = path.points[1];
                assert!(
                    inside(tip, &outline),
                    "after={after}: mark tip {tip:?} left the outline"
                );
            }
            for label in &labels {
                assert!(
                    inside(label.at, &outline),
                    "after={after}: mark label {:?} left the outline",
                    label.at
                );
            }
        }
    }

    #[test]
    fn non_flat_warning_is_carried_onto_nested_sheets_holding_the_panel() {
        let mut bad = test_flat(
            "Warped",
            vec![(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (0.0, 1.0)],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        bad.lies_flat = false;
        let good = test_flat(
            "Flat",
            vec![(0.0, 0.0), (2.0, 0.0), (2.0, 1.0), (0.0, 1.0)],
            vec![[0, 1, 2], [0, 2, 3]],
        );
        let bad_pat = panel_sheet(
            &bad,
            &panel_offsets(&bad, 0.25),
            &[],
            Limits::default(),
            false,
        );
        // Ensure the failure note is present as panel_sheet emits it for a non-flat panel.
        assert!(bad_pat
            .notes
            .iter()
            .any(|n| n.starts_with("Does not lie flat")));
        let good_pat = panel_sheet(
            &good,
            &panel_offsets(&good, 0.25),
            &[],
            Limits::default(),
            false,
        );
        // Each 2.0 x 1.0 m panel fills its own 2.44 x 1.22 m sheet.
        let layout = nest_panels(
            &[bad, good],
            &[bad_pat, good_pat],
            stock(2.44, 1.22, 0.0),
            nested(NestMode::Split),
        );
        assert_eq!(layout.sheets.len(), 2);
        let warped_sheet = layout
            .pieces
            .iter()
            .find(|p| p.panel == "Warped")
            .unwrap()
            .sheet;
        let flat_sheet = layout
            .pieces
            .iter()
            .find(|p| p.panel == "Flat")
            .unwrap()
            .sheet;
        assert_ne!(warped_sheet, flat_sheet);
        assert!(
            layout.drawings[warped_sheet]
                .notes
                .iter()
                .any(|n| n.starts_with("Warped: Does not lie flat")),
            "{:?}",
            layout.drawings[warped_sheet].notes
        );
        assert!(
            !layout.drawings[flat_sheet]
                .notes
                .iter()
                .any(|n| n.contains("Does not lie flat")),
            "{:?}",
            layout.drawings[flat_sheet].notes
        );
    }
}
