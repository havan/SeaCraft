//! Painting the heel view's two drawings: the body plan (looking forward, the hull turned by the
//! heel) and the profile (from starboard, turned by the trim), water level in both.
//!
//! Drawing coordinates are metres in the [`EarthFrame`](super::geometry::EarthFrame): horizontal
//! (across or along) and height above the water. A [`View2`] maps them to the screen; both
//! drawings share one scale so a metre is the same length in each. The pure parts (the mapping,
//! the arrows, the lever's dimension) are tested; the painters only place what they compute.

use eframe::egui;

use super::geometry::{EarthFrame, Envelope, HullOutlines, Point2};
use crate::theme;

/// Drawing metres to screen points: `origin` is the drawing's (0, 0), `ppm` points per metre.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View2 {
    pub origin: egui::Pos2,
    pub ppm: f32,
}

impl View2 {
    pub fn to_screen(&self, p: Point2) -> egui::Pos2 {
        self.origin + egui::vec2(p[0] as f32 * self.ppm, -(p[1] as f32) * self.ppm)
    }

    pub fn from_screen(&self, s: egui::Pos2) -> Point2 {
        let d = s - self.origin;
        [f64::from(d.x / self.ppm), f64::from(-d.y / self.ppm)]
    }
}

/// Fit each drawing at its own scale: the body plan holds the hull's section turned through any
/// heel (its breadth-depth diagonal), the profile its length and that diagonal; each centred on
/// the hull's middle. One shared scale would shrink a long ship's section to a few points.
pub fn fit_views(
    body: egui::Rect,
    profile: egui::Rect,
    outlines: &HullOutlines,
    frame: &EarthFrame,
) -> (View2, View2) {
    let (min, max) = (outlines.min, outlines.max);
    let diag = ((max.y - min.y).hypot(max.z - min.z)).max(1e-3) as f32;
    let length = (max.x - min.x).max(1e-3) as f32;
    let body_ppm = (body.width().min(body.height()) / (1.15 * diag)).max(1e-3);
    let profile_ppm = (profile.width() / (1.08 * length))
        .min(profile.height() / (1.15 * diag))
        .max(1e-3);
    let (a, c, h) = frame.to_earth((min + max) * 0.5);
    let at = |rect: egui::Rect, x: f64, ppm: f32| View2 {
        origin: rect.center() - egui::vec2(x as f32 * ppm, -(h as f32) * ppm),
        ppm,
    };
    (at(body, c, body_ppm), at(profile, a, profile_ppm))
}

/// A straight arrow on the screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Arrow {
    pub from: egui::Pos2,
    pub to: egui::Pos2,
}

/// The buoyancy arrow up from B and the weight arrow down from G, both `len` points long: they
/// are the same force, Δ.
pub fn force_arrows(view: &View2, b: Point2, g: Point2, len: f32) -> (Arrow, Arrow) {
    let (pb, pg) = (view.to_screen(b), view.to_screen(g));
    (
        Arrow {
            from: pb,
            to: pb - egui::vec2(0.0, len),
        },
        Arrow {
            from: pg,
            to: pg + egui::vec2(0.0, len),
        },
    )
}

/// The lever's dimension line at height `h`: from G's vertical to B's. Its length is the lever,
/// as the drawing's horizontal is the direction the lever is measured along.
pub fn lever_dimension(view: &View2, b: Point2, g: Point2, h: f64) -> (egui::Pos2, egui::Pos2) {
    (view.to_screen([g[0], h]), view.to_screen([b[0], h]))
}

/// Points above the lever's dimension line kept for its label.
pub const LEVER_LABEL_ROOM: f32 = 18.0;

/// The height (drawing metres) of the lever's dimension line: above B and G by most of an arrow
/// (`len` points), but never so high that its label leaves the drawing.
pub fn lever_height(view: &View2, rect: egui::Rect, b: Point2, g: Point2, len: f32) -> f64 {
    let above = b[1].max(g[1]) + f64::from(len * 0.6 / view.ppm);
    let highest = view.from_screen(egui::pos2(rect.center().x, rect.top() + LEVER_LABEL_ROOM))[1];
    above.min(highest)
}

/// The colour of a lever: righting or capsizing.
pub fn lever_colour(lever_m: f64) -> egui::Color32 {
    if lever_m >= 0.0 {
        theme::LEVER_RIGHTING
    } else {
        theme::LEVER_CAPSIZING
    }
}

fn arrow(painter: &egui::Painter, a: Arrow, colour: egui::Color32) {
    let stroke = egui::Stroke::new(2.0, colour);
    painter.line_segment([a.from, a.to], stroke);
    let dir = (a.to - a.from).normalized();
    let side = egui::vec2(-dir.y, dir.x) * 4.0;
    let back = a.to - dir * 8.0;
    painter.add(egui::Shape::convex_polygon(
        vec![a.to, back + side, back - side],
        colour,
        egui::Stroke::NONE,
    ));
}

fn polyline(painter: &egui::Painter, view: &View2, pts: &[Point2], stroke: egui::Stroke) {
    if pts.len() < 2 {
        return;
    }
    painter.add(egui::Shape::line(
        pts.iter().map(|&p| view.to_screen(p)).collect(),
        stroke,
    ));
}

/// The water: a faint fill below the surface and the surface line, across the drawing.
pub fn paint_water(painter: &egui::Painter, rect: egui::Rect, view: &View2) {
    let y = view
        .to_screen([0.0, 0.0])
        .y
        .clamp(rect.top(), rect.bottom());
    painter.rect_filled(
        egui::Rect::from_min_max(egui::pos2(rect.left(), y), rect.right_bottom()),
        0.0,
        theme::WATER_FILL,
    );
    painter.line_segment(
        [egui::pos2(rect.left(), y), egui::pos2(rect.right(), y)],
        egui::Stroke::new(1.5, theme::WATER_LINE),
    );
}

/// A centre (B or G) as a ringed dot with its letter.
pub fn paint_centre(painter: &egui::Painter, at: egui::Pos2, letter: &str, colour: egui::Color32) {
    painter.circle(at, 4.0, colour, egui::Stroke::new(1.0, theme::PLOT_BG));
    painter.text(
        at + egui::vec2(6.0, -6.0),
        egui::Align2::LEFT_BOTTOM,
        letter,
        egui::FontId::proportional(theme::SMALL_SIZE),
        colour,
    );
}

/// A trial weight: a square with its mass, ringed when selected.
pub fn paint_weight(painter: &egui::Painter, at: egui::Pos2, label: &str, selected: bool) {
    let r = egui::Rect::from_center_size(at, egui::vec2(9.0, 9.0));
    painter.rect_filled(r, 1.0, theme::TRIAL_WEIGHT);
    if selected {
        painter.rect_stroke(
            r.expand(3.0),
            2.0,
            egui::Stroke::new(1.5, theme::TEXT_PRIMARY),
            egui::StrokeKind::Outside,
        );
    }
    painter.text(
        at + egui::vec2(8.0, 0.0),
        egui::Align2::LEFT_CENTER,
        label,
        egui::FontId::proportional(theme::SMALL_SIZE),
        theme::TRIAL_WEIGHT,
    );
}

/// What the body plan shows at one heel, in drawing metres.
pub struct BodyPlanScene<'a> {
    pub stations: &'a [Vec<Point2>],
    pub section: &'a [Vec<Point2>],
    /// B and G projected, the lever and Δ; `None` with no condition (no KG): the hull only.
    pub forces: Option<Forces>,
    pub weights: &'a [(Point2, String, bool)],
}

/// The two centres, the lever and the displacement.
#[derive(Debug, Clone, Copy)]
pub struct Forces {
    pub b: Point2,
    pub g: Point2,
    pub lever_m: f64,
    pub displacement_t: f64,
}

/// Paint the body plan in `rect`.
pub fn paint_body_plan(painter: &egui::Painter, rect: egui::Rect, view: &View2, s: &BodyPlanScene) {
    painter.rect_filled(rect, 2.0, theme::PLOT_BG);
    paint_water(painter, rect, view);
    for line in s.stations {
        polyline(
            painter,
            view,
            line,
            egui::Stroke::new(1.0, theme::HEEL_STATION),
        );
    }
    for line in s.section {
        polyline(
            painter,
            view,
            line,
            egui::Stroke::new(2.0, theme::HEEL_SECTION),
        );
    }
    if let Some(f) = s.forces {
        let len = (0.28 * rect.height()).clamp(24.0, 90.0);
        let (up, down) = force_arrows(view, f.b, f.g, len);
        arrow(painter, up, theme::BUOYANCY);
        arrow(painter, down, theme::WEIGHT);
        let font = egui::FontId::proportional(theme::SMALL_SIZE);
        painter.text(
            up.to + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_TOP,
            format!("Buoyancy {:.1} t", f.displacement_t),
            font.clone(),
            theme::BUOYANCY,
        );
        painter.text(
            down.to + egui::vec2(4.0, 0.0),
            egui::Align2::LEFT_BOTTOM,
            format!("Weight {:.1} t", f.displacement_t),
            font.clone(),
            theme::WEIGHT,
        );
        // The lever between the two verticals, above both centres.
        let colour = lever_colour(f.lever_m);
        let h = lever_height(view, rect, f.b, f.g, len);
        let (p0, p1) = lever_dimension(view, f.b, f.g, h);
        let thin = egui::Stroke::new(1.0, colour);
        for (from, to) in [(view.to_screen(f.g), p0), (view.to_screen(f.b), p1)] {
            painter.extend(egui::Shape::dashed_line(&[from, to], thin, 3.0, 3.0));
        }
        painter.line_segment([p0, p1], egui::Stroke::new(2.0, colour));
        for p in [p0, p1] {
            painter.line_segment([p - egui::vec2(0.0, 4.0), p + egui::vec2(0.0, 4.0)], thin);
        }
        painter.text(
            egui::pos2(0.5 * (p0.x + p1.x), p0.y - 3.0),
            egui::Align2::CENTER_BOTTOM,
            format!("lever {} m", super::signed(f.lever_m)),
            font,
            colour,
        );
        paint_centre(painter, view.to_screen(f.b), "B", theme::BUOYANCY);
        paint_centre(painter, view.to_screen(f.g), "G", theme::WEIGHT);
    }
    for (at, label, selected) in s.weights {
        paint_weight(painter, view.to_screen(*at), label, *selected);
    }
}

/// What the profile shows: the silhouette, B and G, the weights.
pub struct ProfileScene<'a> {
    pub top: &'a Envelope,
    pub bottom: &'a Envelope,
    pub b: Option<Point2>,
    pub g: Option<Point2>,
    pub weights: &'a [(Point2, String, bool)],
}

/// Paint the profile in `rect`.
pub fn paint_profile(painter: &egui::Painter, rect: egui::Rect, view: &View2, s: &ProfileScene) {
    painter.rect_filled(rect, 2.0, theme::PLOT_BG);
    paint_water(painter, rect, view);
    let stroke = egui::Stroke::new(2.0, theme::HEEL_SECTION);
    let pts = |e: &Envelope| -> Vec<Point2> { e.iter().map(|&(a, h)| [a, h]).collect() };
    let (top, bottom) = (pts(s.top), pts(s.bottom));
    polyline(painter, view, &top, stroke);
    polyline(painter, view, &bottom, stroke);
    // Close the ends.
    for (t, b) in [(top.first(), bottom.first()), (top.last(), bottom.last())] {
        if let (Some(&t), Some(&b)) = (t, b) {
            polyline(painter, view, &[t, b], stroke);
        }
    }
    if let Some(b) = s.b {
        paint_centre(painter, view.to_screen(b), "B", theme::BUOYANCY);
    }
    if let Some(g) = s.g {
        paint_centre(painter, view.to_screen(g), "G", theme::WEIGHT);
    }
    for (at, label, selected) in s.weights {
        paint_weight(painter, view.to_screen(*at), label, *selected);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heel_view::geometry::EarthFrame;
    use seacraft_core::heel::{heel_state, MassCondition};
    use seacraft_core::hydro::{Hull, HydroOptions};

    fn view() -> View2 {
        View2 {
            origin: egui::pos2(200.0, 150.0),
            ppm: 37.5,
        }
    }

    /// A long ship's section fills the body plan rather than shrinking to the profile's scale.
    #[test]
    fn each_drawing_fits_the_hull_at_its_own_scale() {
        use crate::heel_view::geometry::HullOutlines;
        use seacraft_core::math::{Plane, Vec3};
        let outlines = HullOutlines {
            stations: Vec::new(),
            min: Vec3::new(0.0, -16.0, 0.0),
            max: Vec3::new(230.0, 16.0, 20.0),
        };
        let frame = EarthFrame::from_plane(&Plane::new(Vec3::new(0.0, 0.0, 1.0), -10.0));
        let rect = |x: f32| egui::Rect::from_min_size(egui::pos2(x, 0.0), egui::vec2(300.0, 230.0));
        let (body, profile) = fit_views(rect(0.0), rect(310.0), &outlines, &frame);
        let diag = (32.0f32).hypot(20.0);
        // The section's turning diagonal takes most of the body plan's 230 pt height...
        assert!(body.ppm * diag > 0.8 * 230.0, "body plan {} pt/m", body.ppm);
        // ...while the profile still holds the whole length.
        assert!(profile.ppm * 230.0 <= 300.0, "profile {} pt/m", profile.ppm);
    }

    /// G high above a shallow hull: the lever's dimension and its label stay inside the drawing.
    #[test]
    fn the_lever_label_stays_inside_the_drawing() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 230.0));
        let v = View2 {
            origin: egui::pos2(150.0, 130.0),
            ppm: 50.0,
        };
        // G 2.4 m above the water is 10 pt below the top; the arrows are 60 pt long.
        let (b, g) = ([0.0, -0.3], [0.0, 2.4]);
        let h = lever_height(&v, rect, b, g, 60.0);
        let y = v.to_screen([0.0, h]).y;
        assert!(
            y - LEVER_LABEL_ROOM >= rect.top() - 1e-3,
            "dimension at {y}"
        );
        // With room, it sits above both centres.
        let low = lever_height(&v, rect, b, [0.0, 0.5], 60.0);
        assert!(low > 0.5, "{low}");
    }

    #[test]
    fn screen_round_trip() {
        let v = view();
        let p = [1.25, -0.4];
        let q = v.from_screen(v.to_screen(p));
        assert!(
            (q[0] - p[0]).abs() < 1e-4 && (q[1] - p[1]).abs() < 1e-4,
            "{q:?}"
        );
        // Up on the drawing is up on the screen.
        assert!(v.to_screen([0.0, 1.0]).y < v.to_screen([0.0, 0.0]).y);
    }

    #[test]
    fn arrow_lengths_are_equal() {
        let v = view();
        let (up, down) = force_arrows(&v, [0.3, -0.2], [-0.1, 0.9], 60.0);
        let len = |a: &Arrow| (a.to - a.from).length();
        assert!((len(&up) - len(&down)).abs() < 1e-4);
        assert!((len(&up) - 60.0).abs() < 1e-4);
        assert!(up.to.y < up.from.y, "buoyancy points up");
        assert!(down.to.y > down.from.y, "weight points down");
        assert_eq!(up.from, v.to_screen([0.3, -0.2]));
        assert_eq!(down.from, v.to_screen([-0.1, 0.9]));
    }

    /// Source: definition. The body plan's horizontal is the lever direction, so the dimension
    /// between the verticals through B and G, measured on the screen, is the lever.
    #[test]
    fn gz_dimension_spans_the_lever() {
        let surface = seacraft_core::demo::demo_hull(false);
        let sub = seacraft_core::subdiv::SubdivisionOptions {
            level: 1,
            mode: Default::default(),
        };
        let hull = Hull::new(&surface, sub);
        let base = HydroOptions::from_surface(&surface, sub);
        let mut mass = MassCondition::bare(&hull, &base, 2.2, None).unwrap();
        mass.g.y = 0.05;
        let v = view();
        for heel in [-30.0, 5.0, 25.0] {
            let s = heel_state(&hull, &mass, heel, &base, 0.0).unwrap();
            let f = EarthFrame::from_plane(&s.plane);
            let (_, bc, bh) = f.to_earth(s.b);
            let (_, gc, gh) = f.to_earth(s.g);
            let (p0, p1) = lever_dimension(&v, [bc, bh], [gc, gh], 0.5);
            assert!((p0.y - p1.y).abs() < 1e-6, "level");
            let metres = f64::from((p1 - p0).length() / v.ppm);
            assert!(
                (metres - s.lever_m.abs()).abs() < 1e-5,
                "{heel}°: {metres} vs {}",
                s.lever_m
            );
        }
    }
}
