//! Painting the heel view's two drawings: the body plan (looking forward, the hull turned by the
//! heel) and the profile (from starboard, turned by the trim), water level in both.
//!
//! Drawing coordinates are metres in the [`EarthFrame`](super::geometry::EarthFrame): horizontal
//! (across or along) and height above the water. A [`View2`] maps them to the screen; each
//! drawing fits the hull at its own scale so a long ship's section stays readable. The pure parts
//! (the mapping, the arrows, the lever's dimension) are tested; the painters only place what they
//! compute.

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
    let trimmed_height = diag + (frame.up.x.abs() * (max.x - min.x)) as f32;
    let body_ppm = (body.width() / (1.15 * diag))
        .min(body.height() / (1.15 * trimmed_height))
        .max(1e-3);
    let profile_ppm = (profile.width() / (1.08 * length))
        .min(profile.height() / (1.15 * trimmed_height))
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

/// The colour of a lever relative to the heel direction: righting or capsizing.
/// A positive core lever pushes towards smaller signed heel (port), so for nonzero
/// heel it rights the hull when `heel_deg * lever_m >= 0`.
pub fn is_righting(heel_deg: f64, lever_m: f64) -> bool {
    if heel_deg.abs() > 1e-12 {
        heel_deg * lever_m >= 0.0
    } else {
        lever_m.abs() <= 1e-12
    }
}

/// The colour of a lever relative to the heel direction: righting or capsizing.
pub fn lever_colour(heel_deg: f64, lever_m: f64) -> egui::Color32 {
    if is_righting(heel_deg, lever_m) {
        theme::LEVER_RIGHTING
    } else {
        theme::LEVER_CAPSIZING
    }
}

/// Total weight / buoyancy force in kilonewtons (`displacement_t * g`, with standard `g = 9.80665 m/s²`).
pub fn weight_force_kn(displacement_t: f64) -> f64 {
    displacement_t * seacraft_core::resistance::GRAVITY
}

/// Righting (or capsizing) turning moment in kilonewton-metres (`Δ * g * lever_m`).
pub fn righting_moment_knm(displacement_t: f64, lever_m: f64) -> f64 {
    weight_force_kn(displacement_t) * lever_m
}

/// Format a non-negative force magnitude in kN cleanly (never shows `-0.00`).
pub fn format_force_kn(kn: f64) -> String {
    let a = kn.abs();
    if a < 0.005 {
        "0.00".to_string()
    } else if a < 10.0 {
        format!("{a:.2}")
    } else {
        format!("{a:.1}")
    }
}

/// Format a non-negative moment magnitude in kN·m cleanly (never shows `-0.00`).
pub fn format_moment_knm(knm: f64) -> String {
    let a = knm.abs();
    if a < 0.005 {
        "0.00".to_string()
    } else if a < 100.0 {
        format!("{a:.2}")
    } else {
        format!("{a:.1}")
    }
}

/// Split a non-negative mass (in metric tonnes) into `(formatted_number, unit)`,
/// choosing kilograms below 1 t and tonnes from 1 t upward (never shows `-0`).
pub fn format_mass_parts(tonnes: f64) -> (String, &'static str) {
    let a = tonnes.abs();
    if a < 0.9995 {
        let kg = a * 1000.0;
        let num = if kg < 0.05 {
            "0".to_string()
        } else if kg < 9.95 {
            format!("{kg:.1}")
        } else {
            format!("{kg:.0}")
        };
        (num, "kg")
    } else {
        let num = if a < 9.995 {
            format!("{a:.2}")
        } else if a < 999.5 {
            format!("{a:.1}")
        } else {
            format!("{a:.0}")
        };
        (num, "t")
    }
}

/// Format a non-negative mass (in metric tonnes) in `kg` below 1 t and `t` from 1 t upward.
pub fn format_mass(tonnes: f64) -> String {
    let (val, unit) = format_mass_parts(tonnes);
    format!("{val} {unit}")
}

/// Capsize alert state painted into the viewports.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum CapsizeAlert {
    /// Condition settles safely (or is awaiting equilibrium).
    #[default]
    None,
    /// Condition has no stable equilibrium within ±90°.
    ConditionCapsizes,
    /// Condition has an equilibrium, but letting go at this heel rolls over.
    PastVanishingAngle,
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

/// The sky, water fill and waterline across the drawing. When `capsize` is active the sky washes
/// subtly crimson above the horizon so an impending capsize stands out immediately in the scene.
pub fn paint_backdrop(
    painter: &egui::Painter,
    rect: egui::Rect,
    view: &View2,
    capsize: CapsizeAlert,
) {
    painter.rect_filled(rect, 2.0, theme::PLOT_BG);
    let y = view
        .to_screen([0.0, 0.0])
        .y
        .clamp(rect.top(), rect.bottom());
    if capsize != CapsizeAlert::None && y > rect.top() + 1.0 {
        let mut sky = egui::Mesh::default();
        sky.colored_vertex(
            egui::pos2(rect.left(), rect.top()),
            theme::HEEL_SKY_CAPSIZE_TOP,
        );
        sky.colored_vertex(
            egui::pos2(rect.right(), rect.top()),
            theme::HEEL_SKY_CAPSIZE_TOP,
        );
        sky.colored_vertex(egui::pos2(rect.right(), y), theme::HEEL_SKY_CAPSIZE_HORIZON);
        sky.colored_vertex(egui::pos2(rect.left(), y), theme::HEEL_SKY_CAPSIZE_HORIZON);
        sky.add_triangle(0, 1, 2);
        sky.add_triangle(0, 2, 3);
        painter.add(egui::Shape::Mesh(std::sync::Arc::new(sky)));
    }
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

struct PillStyle {
    text: egui::Color32,
    bg: egui::Color32,
    border: egui::Stroke,
}

/// Draw a pill-backed label, clamped inside `bounds` when `bounds` is finite.
fn paint_label_pill(
    painter: &egui::Painter,
    bounds: egui::Rect,
    anchor: egui::Pos2,
    align: egui::Align2,
    text: String,
    font: egui::FontId,
    style: PillStyle,
) {
    let galley = painter.layout_no_wrap(text, font, style.text);
    let raw = align
        .anchor_size(anchor, galley.size())
        .expand2(egui::vec2(5.0, 2.0));
    let safe = bounds.shrink(4.0);
    let mut pill = raw;
    if safe.width() > pill.width() {
        if pill.left() < safe.left() {
            pill = pill.translate(egui::vec2(safe.left() - pill.left(), 0.0));
        } else if pill.right() > safe.right() {
            pill = pill.translate(egui::vec2(safe.right() - pill.right(), 0.0));
        }
    }
    if safe.height() > pill.height() {
        if pill.top() < safe.top() {
            pill = pill.translate(egui::vec2(0.0, safe.top() - pill.top()));
        } else if pill.bottom() > safe.bottom() {
            pill = pill.translate(egui::vec2(0.0, safe.bottom() - pill.bottom()));
        }
    }
    painter.rect(pill, 3.0, style.bg, style.border, egui::StrokeKind::Inside);
    painter.galley(pill.left_top() + egui::vec2(5.0, 2.0), galley, style.text);
}

/// Geometry of the turning-moment arc around the body plan hull.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MomentArc {
    pub center: egui::Pos2,
    pub radius: f32,
    /// Angle in screen radians (`0` right, `+pi/2` down, increasing clockwise).
    pub start_rad: f32,
    /// Angle in screen radians. `end_rad > start_rad` is clockwise on screen.
    pub end_rad: f32,
    pub clockwise: bool,
    pub righting: bool,
    pub moment_knm: f64,
}

impl MomentArc {
    /// Sample `n + 1` points from `start_rad` to `end_rad`.
    pub fn sample(&self, from: f32, to: f32, segments: usize) -> Vec<egui::Pos2> {
        (0..=segments)
            .map(|i| {
                let t = i as f32 / segments.max(1) as f32;
                let a = from + (to - from) * t;
                self.center + egui::vec2(a.cos(), a.sin()) * self.radius
            })
            .collect()
    }

    /// Position along the arc at `t in [0, 1]`.
    pub fn point_at(&self, t: f32) -> egui::Pos2 {
        let a = self.start_rad + (self.end_rad - self.start_rad) * t;
        self.center + egui::vec2(a.cos(), a.sin()) * self.radius
    }

    /// Tangent unit vector in the sweep direction at `end_rad`.
    pub fn end_tangent(&self) -> egui::Vec2 {
        let sign = if self.end_rad >= self.start_rad {
            1.0
        } else {
            -1.0
        };
        egui::vec2(-self.end_rad.sin() * sign, self.end_rad.cos() * sign)
    }
}

/// Build the curved turning-couple arc beside the heeled hull section in `rect`.
/// Returns `None` when the turning moment is negligible (< 0.005 kN·m).
pub fn moment_arc(rect: egui::Rect, hull_rect: egui::Rect, f: Forces) -> Option<MomentArc> {
    let moment_knm = righting_moment_knm(f.displacement_t, f.lever_m);
    if moment_knm.abs() < 0.005 {
        return None;
    }
    let righting = is_righting(f.heel_deg, f.lever_m);
    // Positive core lever rotates towards port (CCW on screen); negative turns starboard (CW).
    let clockwise = f.lever_m < 0.0;
    let on_starboard = f.heel_deg > 1e-6 || (f.heel_deg.abs() <= 1e-6 && clockwise);
    let sweep = (0.46 + 0.44 * (f.lever_m.abs() / 0.35).min(1.0)) as f32;
    let pi = std::f32::consts::PI;
    let (start_rad, end_rad) = match (on_starboard, clockwise) {
        // Starboard capsizing (CW downward into water)
        (true, true) => (-0.08 * pi, -0.08 * pi + sweep),
        // Starboard righting (CCW upward out of water)
        (true, false) => (0.28 * pi, 0.28 * pi - sweep),
        // Port righting (CW upward out of water)
        (false, true) => (0.72 * pi, 0.72 * pi + sweep),
        // Port capsizing (CCW downward into water)
        (false, false) => (1.08 * pi, 1.08 * pi - sweep),
    };
    let center = if hull_rect.is_positive() {
        hull_rect.center()
    } else {
        rect.center()
    };
    let base_r = 0.5 * hull_rect.width().max(hull_rect.height()) + 14.0;
    let safe = rect.shrink2(egui::vec2(18.0, 26.0));
    let max_x = if on_starboard {
        (safe.right() - center.x).max(20.0)
    } else {
        (center.x - safe.left()).max(20.0)
    };
    let max_y = (center.y - safe.top())
        .min(safe.bottom() - center.y)
        .max(20.0);
    let radius = base_r.clamp(28.0, max_x.min(max_y));
    Some(MomentArc {
        center,
        radius,
        start_rad,
        end_rad,
        clockwise,
        righting,
        moment_knm,
    })
}

fn paint_arrowhead(
    painter: &egui::Painter,
    tip: egui::Pos2,
    dir: egui::Vec2,
    colour: egui::Color32,
) {
    let dir = dir.normalized();
    if !dir.is_finite() || dir.length_sq() < 1e-6 {
        return;
    }
    let side = egui::vec2(-dir.y, dir.x) * 4.5;
    let back = tip - dir * 9.0;
    painter.add(egui::Shape::convex_polygon(
        vec![tip, back + side, back - side],
        colour,
        egui::Stroke::NONE,
    ));
}

fn paint_viewport_border(painter: &egui::Painter, rect: egui::Rect, capsize: CapsizeAlert) {
    let stroke = if capsize != CapsizeAlert::None {
        egui::Stroke::new(1.5, theme::FAIL)
    } else {
        egui::Stroke::new(1.0, theme::FRAME)
    };
    painter.rect_stroke(rect, 2.0, stroke, egui::StrokeKind::Inside);
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
    pub capsize: CapsizeAlert,
}

/// The two centres, the heel, the lever and the displacement.
#[derive(Debug, Clone, Copy)]
pub struct Forces {
    pub b: Point2,
    pub g: Point2,
    pub heel_deg: f64,
    pub lever_m: f64,
    pub displacement_t: f64,
}

/// Paint the body plan in `rect`.
pub fn paint_body_plan(painter: &egui::Painter, rect: egui::Rect, view: &View2, s: &BodyPlanScene) {
    paint_backdrop(painter, rect, view, s.capsize);
    let mut hull_rect = egui::Rect::NOTHING;
    for line in s.stations.iter().chain(s.section.iter()) {
        for &p in line {
            hull_rect.extend_with(view.to_screen(p));
        }
    }
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
        let force_kn = format_force_kn(weight_force_kn(f.displacement_t));
        let mass = format_mass(f.displacement_t);
        paint_label_pill(
            painter,
            rect,
            up.to + egui::vec2(5.0, 0.0),
            egui::Align2::LEFT_TOP,
            format!("Buoyancy {force_kn} kN ({mass})"),
            font.clone(),
            PillStyle {
                text: theme::BUOYANCY,
                bg: theme::HUD_BACKING,
                border: egui::Stroke::NONE,
            },
        );
        paint_label_pill(
            painter,
            rect,
            down.to + egui::vec2(5.0, 0.0),
            egui::Align2::LEFT_BOTTOM,
            format!("Weight {force_kn} kN ({mass})"),
            font.clone(),
            PillStyle {
                text: theme::WEIGHT,
                bg: theme::HUD_BACKING,
                border: egui::Stroke::NONE,
            },
        );
        // The lever between the two verticals, above both centres.
        let colour = lever_colour(f.heel_deg, f.lever_m);
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
        paint_label_pill(
            painter,
            rect,
            egui::pos2(0.5 * (p0.x + p1.x), p0.y - 4.0),
            egui::Align2::CENTER_BOTTOM,
            format!("lever {} m", super::signed(f.lever_m)),
            font.clone(),
            PillStyle {
                text: colour,
                bg: theme::HUD_BACKING,
                border: egui::Stroke::NONE,
            },
        );
        // Curved turning-moment arc around the heeled hull.
        if let Some(arc) = moment_arc(rect, hull_rect, f) {
            let pts = arc.sample(arc.start_rad, arc.end_rad, 20);
            painter.add(egui::Shape::line(pts, egui::Stroke::new(2.5, colour)));
            paint_arrowhead(painter, arc.point_at(1.0), arc.end_tangent(), colour);
            if !arc.righting {
                let sign = if arc.end_rad >= arc.start_rad {
                    1.0
                } else {
                    -1.0
                };
                let ghost_end = arc.end_rad + sign * 0.45;
                let ghost_pts = arc.sample(arc.end_rad, ghost_end, 10);
                painter.extend(egui::Shape::dashed_line(
                    &ghost_pts,
                    egui::Stroke::new(1.5, theme::HEEL_MOMENT_GHOST),
                    4.0,
                    3.0,
                ));
            }
            let mid = arc.point_at(0.5);
            let out = (mid - arc.center).normalized();
            let on_right = out.x >= 0.0;
            let label = format!(
                "{} {} kN·m",
                if arc.righting {
                    "Righting"
                } else if s.capsize != CapsizeAlert::None {
                    "Capsizing"
                } else {
                    "Heeling"
                },
                format_moment_knm(arc.moment_knm),
            );
            paint_label_pill(
                painter,
                rect,
                mid + out * 10.0,
                if on_right {
                    egui::Align2::LEFT_CENTER
                } else {
                    egui::Align2::RIGHT_CENTER
                },
                label,
                font,
                PillStyle {
                    text: colour,
                    bg: theme::HUD_BACKING,
                    border: egui::Stroke::new(1.0, colour),
                },
            );
        }
        paint_centre(painter, view.to_screen(f.b), "B", theme::BUOYANCY);
        paint_centre(painter, view.to_screen(f.g), "G", theme::WEIGHT);
    }
    for (at, label, selected) in s.weights {
        paint_weight(painter, view.to_screen(*at), label, *selected);
    }
    if s.capsize != CapsizeAlert::None {
        let badge = match s.capsize {
            CapsizeAlert::ConditionCapsizes => "⚠ Capsizes",
            CapsizeAlert::PastVanishingAngle => "⚠ Past vanishing angle",
            CapsizeAlert::None => unreachable!(),
        };
        paint_label_pill(
            painter,
            rect,
            rect.left_top() + egui::vec2(8.0, 8.0),
            egui::Align2::LEFT_TOP,
            badge.to_string(),
            egui::FontId::proportional(theme::SMALL_SIZE),
            PillStyle {
                text: theme::TEXT_ON_ACCENT,
                bg: theme::HEEL_CAPSIZE_BADGE_BG,
                border: egui::Stroke::new(1.0, theme::FAIL),
            },
        );
    }
    paint_viewport_border(painter, rect, s.capsize);
}

/// What the profile shows: the silhouette, B and G, the weights.
pub struct ProfileScene<'a> {
    pub top: &'a Envelope,
    pub bottom: &'a Envelope,
    pub b: Option<Point2>,
    pub g: Option<Point2>,
    pub weights: &'a [(Point2, String, bool)],
    pub capsize: CapsizeAlert,
}

/// Paint the profile in `rect`.
pub fn paint_profile(painter: &egui::Painter, rect: egui::Rect, view: &View2, s: &ProfileScene) {
    paint_backdrop(painter, rect, view, s.capsize);
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
    paint_viewport_border(painter, rect, s.capsize);
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

    #[test]
    fn trimmed_body_plan_fits_inside_rect() {
        use crate::heel_view::geometry::HullOutlines;
        use seacraft_core::hydro::waterplane_for;
        use seacraft_core::math::{Plane, Vec3};
        let outlines = HullOutlines {
            stations: Vec::new(),
            min: Vec3::new(0.0, -1.0, 0.0),
            max: Vec3::new(100.0, 1.0, 2.0),
        };
        let body = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 230.0));
        let profile = egui::Rect::from_min_size(egui::pos2(310.0, 0.0), egui::vec2(600.0, 230.0));

        // Zero trim matches the pure section diagonal scale.
        let level_frame = EarthFrame::from_plane(&Plane::new(Vec3::new(0.0, 0.0, 1.0), -1.0));
        let (level_body, _) = fit_views(body, profile, &outlines, &level_frame);
        let diag = (2.0f32).hypot(2.0);
        assert!(
            (level_body.ppm - 230.0 / (1.15 * diag)).abs() < 1e-3,
            "{}",
            level_body.ppm
        );

        // 10 m trim on a 100 x 2 x 2 m box must still fit every corner inside `body`.
        let trim_frame = EarthFrame::from_plane(&waterplane_for(0.0, 100.0, 1.0, 10.0, 0.0));
        let (trimmed_body, _) = fit_views(body, profile, &outlines, &trim_frame);
        for x in [0.0, 100.0] {
            for y in [-1.0, 1.0] {
                for z in [0.0, 2.0] {
                    let (_, c, h) = trim_frame.to_earth(Vec3::new(x, y, z));
                    let p = trimmed_body.to_screen([c, h]);
                    assert!(
                        body.contains(p),
                        "corner ({x},{y},{z}) -> {p:?} clipped by {body:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn lever_colour_accounts_for_heel_direction() {
        // Righting on either tack: positive lever at starboard heel, negative lever at port heel.
        assert_eq!(lever_colour(15.0, 0.25), theme::LEVER_RIGHTING);
        assert_eq!(lever_colour(-15.0, -0.25), theme::LEVER_RIGHTING);
        // Capsizing on either tack: negative lever at starboard heel, positive lever at port heel.
        assert_eq!(lever_colour(15.0, -0.25), theme::LEVER_CAPSIZING);
        assert_eq!(lever_colour(-15.0, 0.25), theme::LEVER_CAPSIZING);
        // Zero lever on either tack is not capsizing; nonzero lever upright is capsizing.
        assert_eq!(lever_colour(15.0, 0.0), theme::LEVER_RIGHTING);
        assert_eq!(lever_colour(-15.0, 0.0), theme::LEVER_RIGHTING);
        assert_eq!(lever_colour(0.0, 0.0), theme::LEVER_RIGHTING);
        assert_eq!(lever_colour(0.0, 0.2), theme::LEVER_CAPSIZING);
        assert_eq!(lever_colour(0.0, -0.2), theme::LEVER_CAPSIZING);
    }

    #[test]
    fn force_and_moment_match_si_definitions() {
        // Source: definition (kN = t * g, with g = seacraft_core::resistance::GRAVITY = 9.80665 m/s^2).
        assert!((weight_force_kn(10.0) - 98.0665).abs() < 1e-9);
        assert!((righting_moment_knm(10.0, -0.2) + 19.6133).abs() < 1e-9);
        assert_eq!(format_force_kn(9.84), "9.84");
        assert_eq!(format_force_kn(124.46), "124.5");
        assert_eq!(format_moment_knm(-0.001), "0.00");
        assert_eq!(format_moment_knm(12.345), "12.35");
        assert_eq!(format_moment_knm(123.45), "123.5");
    }

    #[test]
    fn format_mass_switches_between_kg_and_tonnes() {
        assert_eq!(format_mass(0.0), "0 kg");
        assert_eq!(format_mass(-1e-9), "0 kg");
        assert_eq!(format_mass(0.0005), "0.5 kg");
        assert_eq!(format_mass(0.0084), "8.4 kg");
        assert_eq!(format_mass(0.00996), "10 kg");
        assert_eq!(format_mass(0.45), "450 kg");
        assert_eq!(format_mass(0.9994), "999 kg");
        assert_eq!(format_mass(0.9996), "1.00 t");
        assert_eq!(format_mass(1.0), "1.00 t");
        assert_eq!(format_mass(6.37), "6.37 t");
        assert_eq!(format_mass(9.95), "9.95 t");
        assert_eq!(format_mass(9.996), "10.0 t");
        assert_eq!(format_mass(12.76), "12.8 t");
        assert_eq!(format_mass(250.4), "250.4 t");
        assert_eq!(format_mass(999.6), "1000 t");
        assert_eq!(format_mass(4520.0), "4520 t");
    }

    #[test]
    fn moment_arc_turns_clockwise_for_starboard_capsize_and_ccw_for_starboard_righting() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 260.0));
        let hull = egui::Rect::from_center_size(rect.center(), egui::vec2(100.0, 80.0));

        let cap_stbd = Forces {
            b: [0.0, -0.2],
            g: [0.1, 0.5],
            heel_deg: 25.0,
            lever_m: -0.15,
            displacement_t: 10.0,
        };
        let a = moment_arc(rect, hull, cap_stbd).expect("capsize arc");
        assert!(!a.righting);
        assert!(a.clockwise);
        assert!(a.end_rad > a.start_rad, "screen CW increases angle");
        assert!(rect.contains(a.point_at(0.0)) && rect.contains(a.point_at(1.0)));

        let right_stbd = Forces {
            lever_m: 0.15,
            ..cap_stbd
        };
        let a = moment_arc(rect, hull, right_stbd).expect("righting arc");
        assert!(a.righting);
        assert!(!a.clockwise);
        assert!(a.end_rad < a.start_rad, "screen CCW decreases angle");

        let cap_port = Forces {
            heel_deg: -25.0,
            lever_m: 0.15,
            ..cap_stbd
        };
        let a = moment_arc(rect, hull, cap_port).expect("port capsize arc");
        assert!(!a.righting);
        assert!(!a.clockwise);
        assert!(a.end_rad < a.start_rad);

        let balanced = Forces {
            lever_m: 0.0,
            ..cap_stbd
        };
        assert!(moment_arc(rect, hull, balanced).is_none());
    }
}
