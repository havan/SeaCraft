//! Pattern sheets drawn in a window: the same [`Sheet`]s the PDF prints, one under another in
//! model metres, with zoom on the scroll wheel, pan by dragging and fit on a double-click.
//! Nothing is recomputed here: what is drawn is what is printed.

use eframe::egui;
use seacraft_core::pdf::Anchor;
use seacraft_core::sheet::{PathKind, Sheet, DRAW_ORDER};

use crate::theme;

/// Space between two sheets, metres.
pub const GAP_M: f64 = 0.1;
/// The closest zoom, pixels per metre.
pub const MAX_ZOOM: f32 = 20_000.0;
/// Screen font size for titles and overall dimensions, points.
pub const PRIMARY_LABEL_PX: f32 = theme::BODY_SIZE;
/// Screen font size for offsets, seam lengths, stations and notes, points.
pub const DETAIL_LABEL_PX: f32 = theme::CAPTION_SIZE;
/// Minimum gap between neighbouring label bounding boxes on screen, points.
const LABEL_PAD_PX: f32 = 1.5;
/// Extra hit margin around a label for brightening on hover, points.
const HOVER_PAD_PX: f32 = 6.0;
/// Two labels whose anchors lie within this many screen points are treated as a pair (e.g. a
/// seam name and its length) so the second shifts clear of the first instead of being dropped.
const PAIR_DIST_PX: f32 = 24.0;
/// Minimum screen clearance between an outside overall dimension label and the cut outline.
const OUTSIDE_CLEAR_MARGIN_PX: f32 = 4.0;
/// Room left round a framed sheet, pixels.
const FRAME_MARGIN: f32 = 16.0;

/// What a sheet covers in its own metres: its lines and its labels' anchors. `None` for a sheet
/// with neither.
fn own_bounds(sheet: &Sheet) -> Option<[f64; 4]> {
    let points = sheet
        .paths
        .iter()
        .flat_map(|p| p.points.iter().copied())
        .chain(sheet.labels.iter().map(|l| l.at))
        .filter(|(x, y)| x.is_finite() && y.is_finite());
    points.fold(None, |b, (x, y)| {
        Some(match b {
            None => [x, y, x, y],
            Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
        })
    })
}

/// A sheet's shift from its own metres to the drawing's, and where it lands.
type Placed = ((f64, f64), [f64; 4]);

/// Each sheet's shift and where it lands, `[x0, y0, x1, y1]` in metres: left-aligned, the first at
/// the top and each next one [`GAP_M`] under the one before. A sheet with nothing on it is `None`
/// and takes no room.
fn stack(sheets: &[Sheet]) -> Vec<Option<Placed>> {
    let mut top = 0.0;
    sheets
        .iter()
        .map(|s| {
            let b = own_bounds(s)?;
            let (w, h) = (b[2] - b[0], b[3] - b[1]);
            let placed = [0.0, top - h, w, top];
            top -= h + GAP_M;
            Some(((-b[0], top + h + GAP_M - b[3]), placed))
        })
        .collect()
}

/// Where each sheet lands in the drawing (see [`stack`]).
pub fn placed_bounds(sheets: &[Sheet]) -> Vec<Option<[f64; 4]>> {
    stack(sheets)
        .into_iter()
        .map(|s| s.map(|(_, b)| b))
        .collect()
}

/// What the drawing should frame next.
#[derive(Clone, Copy, Debug, PartialEq)]
enum Pending {
    FitAll,
    Frame(usize),
}

/// Where the drawing is looking: kept by the window between frames.
#[derive(Clone, Debug, PartialEq)]
pub struct SheetView {
    /// Pixels per metre.
    pub zoom: f32,
    /// The point at the middle of the drawing, metres.
    pub centre: (f64, f64),
    pending: Option<Pending>,
}

impl Default for SheetView {
    fn default() -> Self {
        Self {
            zoom: 100.0,
            centre: (0.0, 0.0),
            pending: Some(Pending::FitAll),
        }
    }
}

/// The zoom that fits `b` in `rect`.
fn fit(rect: egui::Rect, b: [f64; 4]) -> f32 {
    let w = (rect.width() - 2.0 * FRAME_MARGIN).max(1.0) as f64;
    let h = (rect.height() - 2.0 * FRAME_MARGIN).max(1.0) as f64;
    (w / (b[2] - b[0]).max(1e-6)).min(h / (b[3] - b[1]).max(1e-6)) as f32
}

fn union(bounds: &[Option<[f64; 4]>]) -> Option<[f64; 4]> {
    bounds.iter().flatten().copied().reduce(|a, b| {
        [
            a[0].min(b[0]),
            a[1].min(b[1]),
            a[2].max(b[2]),
            a[3].max(b[3]),
        ]
    })
}

impl SheetView {
    /// Frame every sheet on the next frame.
    pub fn fit_all(&mut self) {
        self.pending = Some(Pending::FitAll);
    }

    /// Frame sheet `i` on the next frame.
    pub fn frame(&mut self, i: usize) {
        self.pending = Some(Pending::Frame(i));
    }

    /// Carries out a pending fit or frame and keeps the zoom between a quarter of fit-all and
    /// [`MAX_ZOOM`].
    pub fn settle(&mut self, rect: egui::Rect, bounds: &[Option<[f64; 4]>]) {
        let Some(all) = union(bounds) else {
            return;
        };
        let target = match self.pending.take() {
            Some(Pending::FitAll) => Some(all),
            Some(Pending::Frame(i)) => bounds.get(i).copied().flatten(),
            None => None,
        };
        if let Some(b) = target {
            self.zoom = fit(rect, b);
            self.centre = ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0);
        }
        let low = fit(rect, all) / 4.0;
        self.zoom = self.zoom.clamp(low.min(MAX_ZOOM), MAX_ZOOM);
    }

    /// Zooms by `factor` about `pivot`, which stays over the same point of the drawing.
    pub fn zoom_by(
        &mut self,
        factor: f32,
        pivot: egui::Pos2,
        rect: egui::Rect,
        bounds: &[Option<[f64; 4]>],
    ) {
        let before = self.to_world(rect, pivot);
        self.zoom *= factor;
        self.settle(rect, bounds);
        let after = self.to_world(rect, pivot);
        self.centre.0 += before.0 - after.0;
        self.centre.1 += before.1 - after.1;
    }

    /// A point of the drawing, metres, on the screen.
    pub fn to_screen(&self, rect: egui::Rect, (x, y): (f64, f64)) -> egui::Pos2 {
        rect.center()
            + egui::vec2(
                ((x - self.centre.0) * self.zoom as f64) as f32,
                (-(y - self.centre.1) * self.zoom as f64) as f32,
            )
    }

    /// The point of the drawing under a point of the screen, metres.
    pub fn to_world(&self, rect: egui::Rect, p: egui::Pos2) -> (f64, f64) {
        let d = p - rect.center();
        (
            self.centre.0 + d.x as f64 / self.zoom as f64,
            self.centre.1 - d.y as f64 / self.zoom as f64,
        )
    }
}

/// What a sheet's cut outline covers in its own metres, if any.
fn cut_bounds(sheet: &Sheet) -> Option<[f64; 4]> {
    sheet
        .paths
        .iter()
        .filter(|p| p.kind == PathKind::Cut)
        .flat_map(|p| p.points.iter().copied())
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .fold(None, |b, (x, y)| {
            Some(match b {
                None => [x, y, x, y],
                Some([x0, y0, x1, y1]) => [x0.min(x), y0.min(y), x1.max(x), y1.max(y)],
            })
        })
}

fn label_px(height_mm: f64) -> f32 {
    if height_mm >= 4.0 {
        PRIMARY_LABEL_PX
    } else {
        DETAIL_LABEL_PX
    }
}

/// Priority when overview labels compete for screen space: titles and overall dimensions first,
/// then seam lengths / stations / split marks, then offsets and secondary notes.
fn label_tier(label: &seacraft_core::sheet::Label) -> u8 {
    if label.height_mm >= 4.0 {
        return 0;
    }
    let t = label.text.as_str();
    if t.ends_with(" mm")
        || t.ends_with('"')
        || t.ends_with(')')
        || t.starts_with("St ")
        || (t.len() == 1 && t.chars().all(|c| c.is_ascii_alphabetic()))
    {
        1
    } else {
        2
    }
}

fn label_corners(pos: egui::Pos2, size: egui::Vec2, angle: f32) -> [egui::Pos2; 4] {
    let rot = egui::emath::Rot2::from_angle(angle);
    [
        pos,
        pos + rot * egui::vec2(size.x, 0.0),
        pos + rot * size,
        pos + rot * egui::vec2(0.0, size.y),
    ]
}

fn label_aabb(pos: egui::Pos2, size: egui::Vec2, angle: f32) -> egui::Rect {
    egui::Rect::from_points(&label_corners(pos, size, angle))
}

fn label_hovered(hover: Option<egui::Pos2>, pos: egui::Pos2, size: egui::Vec2, angle: f32) -> bool {
    let Some(p) = hover else {
        return false;
    };
    let local = (egui::emath::Rot2::from_angle(-angle) * (p - pos)).to_pos2();
    egui::Rect::from_min_size(egui::Pos2::ZERO, size)
        .expand(HOVER_PAD_PX)
        .contains(local)
}

struct PlacedLabel {
    galley: std::sync::Arc<egui::Galley>,
    pos: egui::Pos2,
    angle: f32,
    size: egui::Vec2,
    aabb: egui::Rect,
    anchor_screen: egui::Pos2,
    text: String,
    px: f32,
    hovered: bool,
}

/// Draws `sheets` in the space left in `ui`, tinting those `not_flat` marks and veiling the
/// whole when `faded` (an out-of-date drawing), and returns the sheet clicked, if any. Labels are
/// drawn at fixed screen sizes, dimmed at rest and highlighted when hovered. Scroll zooms about
/// the pointer, dragging pans, a double-click fits all. Screen readers hear "Drawing: N sheets".
pub fn show(
    ui: &mut egui::Ui,
    sheets: &[Sheet],
    not_flat: &[bool],
    faded: bool,
    view: &mut SheetView,
) -> Option<usize> {
    let size = ui.available_size().max(egui::vec2(120.0, 80.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::PLOT_BG);
    let stacked = stack(sheets);
    let bounds: Vec<Option<[f64; 4]>> = stacked.iter().map(|s| s.map(|(_, b)| b)).collect();
    let drawn = bounds.iter().flatten().count();
    response.widget_info(|| {
        egui::WidgetInfo::labeled(egui::Role::Canvas, true, format!("Drawing: {drawn} sheets"))
    });
    if union(&bounds).is_none() {
        painter.text(
            rect.center(),
            egui::Align2::CENTER_CENTER,
            "Nothing to draw",
            egui::FontId::proportional(14.0),
            theme::TEXT_MUTED,
        );
        return None;
    }
    if response.double_clicked() {
        view.fit_all();
    }
    view.settle(rect, &bounds);
    let hover_pos = response.hover_pos();
    if let Some(hover) = hover_pos {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            view.zoom_by((scroll * 0.0015).exp(), hover, rect, &bounds);
        }
    }
    if response.dragged() {
        let d = response.drag_delta();
        view.centre.0 -= d.x as f64 / view.zoom as f64;
        view.centre.1 += d.y as f64 / view.zoom as f64;
    }

    for (b, _) in bounds.iter().zip(not_flat).filter(|(_, bad)| **bad) {
        let Some(b) = b else { continue };
        let r = egui::Rect::from_two_pos(
            view.to_screen(rect, (b[0], b[1])),
            view.to_screen(rect, (b[2], b[3])),
        );
        painter.rect_filled(r.expand(4.0), 2.0, theme::PATTERN_NOT_FLAT);
    }
    let mut placed_labels: Vec<PlacedLabel> = Vec::new();
    for (placed, sheet) in stacked.iter().zip(sheets) {
        let Some(((dx, dy), _)) = placed else {
            continue;
        };
        let at = |(x, y): (f64, f64)| view.to_screen(rect, (x + dx, y + dy));
        // As on paper: the cut line over the marks, the marks over the references.
        let ordered = DRAW_ORDER
            .iter()
            .flat_map(|kind| sheet.paths.iter().filter(move |p| p.kind == *kind));
        for path in ordered {
            let mut points: Vec<egui::Pos2> = path.points.iter().map(|p| at(*p)).collect();
            if path.closed && points.len() > 2 {
                points.push(points[0]);
            }
            match path.kind {
                PathKind::Cut => {
                    painter.add(egui::Shape::line(
                        points,
                        egui::Stroke::new(2.0, theme::PATTERN_CUT),
                    ));
                }
                PathKind::Mark => {
                    painter.add(egui::Shape::line(
                        points,
                        egui::Stroke::new(1.0, theme::PATTERN_MARK),
                    ));
                }
                PathKind::Reference => {
                    let stroke = egui::Stroke::new(1.0, theme::PATTERN_REFERENCE);
                    painter.extend(egui::Shape::dashed_line(&points, stroke, 4.0, 3.0));
                }
            }
        }
        let cb = cut_bounds(sheet);
        let mut order: Vec<usize> = (0..sheet.labels.len()).collect();
        order.sort_by_key(|&i| label_tier(&sheet.labels[i]));
        for i in order {
            let label = &sheet.labels[i];
            if !label.height_mm.is_finite()
                || label.height_mm <= 0.0
                || label.text.trim().is_empty()
            {
                continue;
            }
            let tier = label_tier(label);
            let px = label_px(label.height_mm);
            let galley = painter.layout_no_wrap(
                label.text.clone(),
                egui::FontId::proportional(px),
                theme::PATTERN_LABEL_DIM,
            );
            let size = galley.size();
            let along = match label.anchor {
                Anchor::Start => 0.0,
                Anchor::Middle => 0.5,
                Anchor::End => 1.0,
            };
            // `at` is the text's baseline point; the galley is placed by its top-left corner and
            // turned about it (anticlockwise on paper is clockwise on a y-down screen).
            let angle = -(label.angle_deg.to_radians() as f32);
            let rot = egui::emath::Rot2::from_angle(angle);
            let anchor_screen = at(label.at);
            let mut y_off = -0.8 * size.y;
            if let Some(cb) = cb {
                let below_cut = label.at.1 < cb[1] && label.angle_deg.abs() < 15.0;
                let right_of_cut =
                    label.at.0 > cb[2] && (label.angle_deg.abs() - 90.0).abs() < 15.0;
                let gap_px = if below_cut {
                    anchor_screen.y - at((label.at.0, cb[1])).y
                } else if right_of_cut {
                    anchor_screen.x - at((cb[2], label.at.1)).x
                } else {
                    f32::INFINITY
                };
                if gap_px < size.y + OUTSIDE_CLEAR_MARGIN_PX {
                    y_off = 2.0;
                }
            }
            let pos = anchor_screen + rot * egui::vec2(-along * size.x, y_off);
            let aabb = label_aabb(pos, size, angle);
            let mut chosen = Some((pos, aabb));
            if let Some(hit) = placed_labels
                .iter()
                .find(|e| e.aabb.expand(LABEL_PAD_PX).intersects(aabb))
            {
                let close_pair = (anchor_screen - hit.anchor_screen).length() <= PAIR_DIST_PX
                    && (angle - hit.angle).abs() < 0.2;
                if close_pair {
                    let delta = anchor_screen - hit.anchor_screen;
                    let dir = if delta.length_sq() > 1e-6 {
                        delta.normalized()
                    } else {
                        rot * egui::vec2(0.0, 1.0)
                    };
                    let shifted = pos + dir * (hit.size.y + 2.0);
                    let shifted_aabb = label_aabb(shifted, size, angle);
                    if !placed_labels
                        .iter()
                        .any(|e| e.aabb.expand(LABEL_PAD_PX).intersects(shifted_aabb))
                    {
                        chosen = Some((shifted, shifted_aabb));
                    } else if tier > 0 {
                        chosen = None;
                    }
                } else if tier > 0 {
                    chosen = None;
                }
            }
            let Some((pos, aabb)) = chosen else {
                continue;
            };
            let hovered = label_hovered(hover_pos, pos, size, angle);
            placed_labels.push(PlacedLabel {
                galley,
                pos,
                angle,
                size,
                aabb,
                anchor_screen,
                text: label.text.clone(),
                px,
                hovered,
            });
        }
    }

    for pl in &placed_labels {
        if !pl.hovered {
            painter.add(
                egui::epaint::TextShape::new(pl.pos, pl.galley.clone(), theme::PATTERN_LABEL_DIM)
                    .with_angle(pl.angle),
            );
        }
    }
    for pl in placed_labels {
        if pl.hovered {
            let rot = egui::emath::Rot2::from_angle(pl.angle);
            let box_rect = egui::Rect::from_min_size(egui::Pos2::ZERO, pl.size).expand(2.0);
            let bg_pts = vec![
                pl.pos + rot * box_rect.left_top().to_vec2(),
                pl.pos + rot * box_rect.right_top().to_vec2(),
                pl.pos + rot * box_rect.right_bottom().to_vec2(),
                pl.pos + rot * box_rect.left_bottom().to_vec2(),
            ];
            painter.add(egui::Shape::convex_polygon(
                bg_pts,
                theme::HUD_BACKING,
                egui::Stroke::NONE,
            ));
            let galley = painter.layout_no_wrap(
                pl.text,
                egui::FontId::proportional(pl.px),
                theme::PATTERN_LABEL_HOVER,
            );
            painter.add(
                egui::epaint::TextShape::new(pl.pos, galley, theme::PATTERN_LABEL_HOVER)
                    .with_angle(pl.angle),
            );
        }
    }

    if faded {
        painter.rect_filled(rect, 0.0, theme::PATTERN_STALE_VEIL);
    }
    if response.clicked() {
        let p = response.interact_pointer_pos()?;
        let (x, y) = view.to_world(rect, p);
        return bounds
            .iter()
            .position(|b| b.is_some_and(|b| x >= b[0] && x <= b[2] && y >= b[1] && y <= b[3]));
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::sheet::{PathKind, SheetPath};

    fn rect_sheet(name: &str, w: f64, h: f64) -> Sheet {
        let mut s = Sheet::new(name);
        s.paths.push(SheetPath {
            kind: PathKind::Cut,
            points: vec![(0.0, 0.0), (w, 0.0), (w, h), (0.0, h)],
            closed: true,
        });
        s
    }

    fn screen() -> egui::Rect {
        egui::Rect::from_min_size(egui::pos2(100.0, 50.0), egui::vec2(600.0, 400.0))
    }

    #[test]
    fn sheets_stack_one_under_another_without_touching() {
        let sheets = [
            rect_sheet("A", 6.0, 1.0),
            rect_sheet("B", 0.3, 0.4),
            rect_sheet("C", 2.0, 0.5),
        ];
        let b = placed_bounds(&sheets);
        assert_eq!(b.len(), 3);
        for w in b.windows(2) {
            let (upper, lower) = (w[0].unwrap(), w[1].unwrap());
            assert!(
                lower[3] + GAP_M - 1e-12 <= upper[1],
                "{upper:?} above {lower:?}"
            );
            assert!((lower[0] - upper[0]).abs() < 1e-12, "left-aligned");
        }
    }

    #[test]
    fn fit_all_frames_every_sheet_and_frame_centres_one() {
        let sheets = [rect_sheet("A", 6.0, 1.0), rect_sheet("B", 0.3, 0.4)];
        let bounds = placed_bounds(&sheets);
        let mut v = SheetView::default();
        v.fit_all();
        v.settle(screen(), &bounds);
        for b in bounds.iter().flatten() {
            for corner in [(b[0], b[1]), (b[2], b[3])] {
                assert!(
                    screen().contains(v.to_screen(screen(), corner)),
                    "{corner:?}"
                );
            }
        }
        let fitted = v.zoom;
        v.frame(1);
        v.settle(screen(), &bounds);
        let b = bounds[1].unwrap();
        let centre = v.to_screen(screen(), ((b[0] + b[2]) / 2.0, (b[1] + b[3]) / 2.0));
        assert!((centre - screen().center()).length() < 0.5, "{centre:?}");
        assert!(v.zoom > fitted, "a small panel is framed closer");
        let back = v.to_world(screen(), centre);
        assert!((back.0 - (b[0] + b[2]) / 2.0).abs() < 1e-3);
    }

    #[test]
    fn zoom_stays_between_a_quarter_of_fit_all_and_the_limit() {
        let sheets = [rect_sheet("A", 6.0, 1.0)];
        let bounds = placed_bounds(&sheets);
        let mut v = SheetView::default();
        v.fit_all();
        v.settle(screen(), &bounds);
        let fitted = v.zoom;
        v.zoom_by(1e-6, screen().center(), screen(), &bounds);
        assert!(
            (v.zoom - fitted / 4.0).abs() < 1e-3,
            "{} vs {fitted}",
            v.zoom
        );
        v.zoom_by(1e9, screen().center(), screen(), &bounds);
        assert_eq!(v.zoom, MAX_ZOOM);
    }

    /// Every text painted in one headless frame of `show`.
    fn painted(sheets: &[Sheet]) -> Vec<String> {
        let ctx = egui::Context::default();
        let mut view = SheetView::default();
        let not_flat = vec![true; sheets.len()];
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            show(ui, sheets, &not_flat, false, &mut view);
        });
        out.textures_delta.clear();
        let mut texts = Vec::new();
        for clipped in out.shapes {
            if let egui::Shape::Text(t) = clipped.shape {
                texts.push(t.galley.text().to_owned());
            }
        }
        texts
    }

    #[test]
    fn nothing_to_draw_is_said_and_empty_sheets_do_not_panic() {
        assert!(painted(&[]).iter().any(|t| t == "Nothing to draw"));
        assert!(painted(&[Sheet::new("empty")])
            .iter()
            .any(|t| t == "Nothing to draw"));
        assert!(!painted(&[rect_sheet("A", 1.0, 1.0)])
            .iter()
            .any(|t| t == "Nothing to draw"));
    }

    /// The colour of every line painted in one headless frame of `show`, in paint order.
    fn line_colours(sheets: &[Sheet]) -> Vec<egui::Color32> {
        let ctx = egui::Context::default();
        let mut view = SheetView::default();
        let not_flat = vec![false; sheets.len()];
        let mut colours = Vec::new();
        // Two frames: the first only settles the pending fit.
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                show(ui, sheets, &not_flat, false, &mut view);
            });
            out.textures_delta.clear();
            fn walk(shape: &egui::Shape, colours: &mut Vec<egui::Color32>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, colours)),
                    egui::Shape::LineSegment { stroke, .. } => colours.push(stroke.color),
                    egui::Shape::Path(p) => {
                        if let egui::epaint::ColorMode::Solid(c) = p.stroke.color {
                            colours.push(c);
                        }
                    }
                    _ => {}
                }
            }
            colours.clear();
            for clipped in &out.shapes {
                walk(&clipped.shape, &mut colours);
            }
        }
        colours
    }

    #[test]
    fn the_cut_line_is_painted_over_marks_and_references_as_on_paper() {
        // Stored cut first, reference last, as the panels' plywood outlines are.
        let mut sheet = rect_sheet("A", 2.0, 1.0);
        sheet.paths.push(SheetPath {
            kind: PathKind::Reference,
            points: vec![(0.0, 0.0), (2.0, 0.0)],
            closed: false,
        });
        sheet.paths.push(SheetPath {
            kind: PathKind::Mark,
            points: vec![(1.0, 0.0), (1.0, 1.0)],
            closed: false,
        });
        let colours = line_colours(&[sheet]);
        let last = |c| colours.iter().rposition(|x| *x == c);
        let first = |c| colours.iter().position(|x| *x == c);
        let cut = first(theme::PATTERN_CUT).expect("the cut line");
        let mark = first(theme::PATTERN_MARK).expect("the mark");
        assert!(
            last(theme::PATTERN_REFERENCE).expect("the reference") < mark,
            "{colours:?}"
        );
        assert!(last(theme::PATTERN_MARK).unwrap() < cut, "{colours:?}");
    }

    struct DrawnText {
        text: String,
        size_px: f32,
        colour: egui::Color32,
        bounds: egui::Rect,
        centre: egui::Pos2,
    }

    fn collect_shapes(shape: &egui::Shape, texts: &mut Vec<DrawnText>, has_hud_backing: &mut bool) {
        match shape {
            egui::Shape::Vec(v) => v
                .iter()
                .for_each(|s| collect_shapes(s, texts, has_hud_backing)),
            egui::Shape::Rect(r) if r.fill == theme::HUD_BACKING => *has_hud_backing = true,
            egui::Shape::Path(p) if p.fill == theme::HUD_BACKING => *has_hud_backing = true,
            egui::Shape::Text(t) => {
                let section = t.galley.job.sections.first();
                let size_px = section.map_or(0.0, |s| s.format.font_id.size);
                let colour = section.map_or(t.fallback_color, |s| s.format.color);
                let sz = t.galley.size();
                let rot = egui::emath::Rot2::from_angle(t.angle);
                let corners = [
                    t.pos,
                    t.pos + rot * egui::vec2(sz.x, 0.0),
                    t.pos + rot * sz,
                    t.pos + rot * egui::vec2(0.0, sz.y),
                ];
                let bounds = egui::Rect::from_points(&corners);
                texts.push(DrawnText {
                    text: t.galley.text().to_owned(),
                    size_px,
                    colour,
                    bounds,
                    centre: t.pos + rot * (0.5 * sz),
                });
            }
            _ => {}
        }
    }

    fn run_sheet_frame(
        ctx: &egui::Context,
        sheets: &[Sheet],
        view: &mut SheetView,
        events: Vec<egui::Event>,
    ) -> (Vec<DrawnText>, bool) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(600.0, 400.0),
            )),
            events,
            ..Default::default()
        };
        let not_flat = vec![false; sheets.len()];
        let mut out = ctx.run_ui(input, |ui| {
            show(ui, sheets, &not_flat, false, view);
        });
        out.textures_delta.clear();
        let mut texts = Vec::new();
        let mut has_hud_backing = false;
        for clipped in &out.shapes {
            collect_shapes(&clipped.shape, &mut texts, &mut has_hud_backing);
        }
        (texts, has_hud_backing)
    }

    fn skiff_like_sheet() -> Sheet {
        let mut s = rect_sheet("Bottom", 4.3, 1.1);
        s.labels.push(seacraft_core::sheet::Label {
            at: (2.15, -0.038),
            height_mm: 5.0,
            angle_deg: 0.0,
            anchor: Anchor::Middle,
            text: "4350 mm".into(),
        });
        s.labels.push(seacraft_core::sheet::Label {
            at: (1.0, 0.002),
            height_mm: 3.0,
            angle_deg: 90.0,
            anchor: Anchor::Start,
            text: "565".into(),
        });
        s
    }

    #[test]
    fn dimensions_are_visible_at_fit_all_zoom_and_keep_constant_size() {
        let ctx = egui::Context::default();
        let sheets = [skiff_like_sheet()];
        let mut view = SheetView::default();
        let _ = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        let (texts, _) = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        let len = texts.iter().find(|t| t.text == "4350 mm").expect("overall");
        let off = texts.iter().find(|t| t.text == "565").expect("offset");
        assert_eq!(len.size_px, PRIMARY_LABEL_PX);
        assert_eq!(off.size_px, DETAIL_LABEL_PX);

        let bounds = placed_bounds(&sheets);
        let rect = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(600.0, 400.0));
        view.zoom_by(10.0, rect.center(), rect, &bounds);
        let (zoomed, _) = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        let len_z = zoomed
            .iter()
            .find(|t| t.text == "4350 mm")
            .expect("overall zoomed");
        assert_eq!(len_z.size_px, PRIMARY_LABEL_PX);
        if let Some(off_z) = zoomed.iter().find(|t| t.text == "565") {
            assert_eq!(off_z.size_px, DETAIL_LABEL_PX);
        }
    }

    #[test]
    fn labels_are_dimmed_at_rest_and_brighten_when_hovered() {
        let ctx = egui::Context::default();
        let sheets = [skiff_like_sheet()];
        let mut view = SheetView::default();
        let _ = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        let (rest, backing) = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        assert!(!backing);
        let len = rest.iter().find(|t| t.text == "4350 mm").expect("overall");
        let off = rest.iter().find(|t| t.text == "565").expect("offset");
        assert_eq!(len.colour, theme::PATTERN_LABEL_DIM);
        assert_eq!(off.colour, theme::PATTERN_LABEL_DIM);

        let pointer = len.centre;
        let (hovered, backing) = run_sheet_frame(
            &ctx,
            &sheets,
            &mut view,
            vec![egui::Event::PointerMoved(pointer)],
        );
        assert!(backing, "hovered label gets HUD backing");
        let len_h = hovered
            .iter()
            .find(|t| t.text == "4350 mm")
            .expect("overall hovered");
        let off_h = hovered
            .iter()
            .find(|t| t.text == "565")
            .expect("offset still dimmed");
        assert_eq!(len_h.colour, theme::PATTERN_LABEL_HOVER);
        assert_eq!(off_h.colour, theme::PATTERN_LABEL_DIM);
    }

    #[test]
    fn overlapping_labels_do_not_stack_on_top_of_each_other() {
        let ctx = egui::Context::default();
        let mut s = rect_sheet("Bottom", 4.3, 1.1);
        s.labels.push(seacraft_core::sheet::Label {
            at: (2.15, 1.088),
            height_mm: 3.0,
            angle_deg: 0.0,
            anchor: Anchor::Middle,
            text: "Stitch to Side".into(),
        });
        s.labels.push(seacraft_core::sheet::Label {
            at: (2.15, 1.0835),
            height_mm: 3.0,
            angle_deg: 0.0,
            anchor: Anchor::Middle,
            text: "4398 mm".into(),
        });
        let sheets = [s];
        let mut view = SheetView::default();
        let _ = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        let (texts, _) = run_sheet_frame(&ctx, &sheets, &mut view, vec![]);
        let seam = texts
            .iter()
            .find(|t| t.text == "Stitch to Side")
            .expect("seam label");
        let len = texts
            .iter()
            .find(|t| t.text == "4398 mm")
            .expect("seam length");
        assert!(
            !seam.bounds.intersects(len.bounds),
            "{:?} overlaps {:?}",
            seam.bounds,
            len.bounds
        );
    }

    #[test]
    fn zoom_allows_close_inspection() {
        let sheets = [rect_sheet("A", 6.0, 1.0)];
        let bounds = placed_bounds(&sheets);
        let mut v = SheetView::default();
        v.fit_all();
        v.settle(screen(), &bounds);
        v.zoom_by(1e6, screen().center(), screen(), &bounds);
        assert!(v.zoom >= 10_000.0, "zoom capped too low: {}", v.zoom);
    }
}
