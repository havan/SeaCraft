//! Pattern sheets drawn in a window: the same [`Sheet`]s the PDF prints, one under another in
//! model metres, with zoom on the scroll wheel, pan by dragging and fit on a double-click.
//! Nothing is recomputed here: what is drawn is what is printed.

use eframe::egui;
use seacraft_core::pdf::Anchor;
use seacraft_core::sheet::{PathKind, Sheet};

use crate::theme;

/// Space between two sheets, metres.
pub const GAP_M: f64 = 0.1;
/// The closest zoom, pixels per metre.
pub const MAX_ZOOM: f32 = 2000.0;
/// A label smaller than this, in pixels, is not drawn.
const MIN_LABEL_PX: f32 = 6.0;
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

/// Draws `sheets` in the space left in `ui`, tinting those `not_flat` marks and veiling the
/// whole when `faded` (an out-of-date drawing), and returns the sheet clicked, if any. Scroll
/// zooms about the pointer, dragging pans, a double-click fits all. Screen readers hear
/// "Drawing: N sheets".
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
    if let Some(hover) = response.hover_pos() {
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
    for (placed, sheet) in stacked.iter().zip(sheets) {
        let Some(((dx, dy), _)) = placed else {
            continue;
        };
        let at = |(x, y): (f64, f64)| view.to_screen(rect, (x + dx, y + dy));
        for path in &sheet.paths {
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
        for label in &sheet.labels {
            let px = (label.height_mm / 1000.0) as f32 * view.zoom;
            if px.is_nan() || px < MIN_LABEL_PX || label.text.trim().is_empty() {
                continue;
            }
            let galley = painter.layout_no_wrap(
                label.text.clone(),
                egui::FontId::proportional(px),
                theme::PATTERN_LABEL,
            );
            let along = match label.anchor {
                Anchor::Start => 0.0,
                Anchor::Middle => 0.5,
                Anchor::End => 1.0,
            };
            // `at` is the text's baseline point; the galley is placed by its top-left corner and
            // turned about it (anticlockwise on paper is clockwise on a y-down screen).
            let angle = -(label.angle_deg.to_radians() as f32);
            let offset = egui::vec2(-along * galley.size().x, -0.8 * galley.size().y);
            let pos = at(label.at) + egui::emath::Rot2::from_angle(angle) * offset;
            painter.add(
                egui::epaint::TextShape::new(pos, galley, theme::PATTERN_LABEL).with_angle(angle),
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
}
