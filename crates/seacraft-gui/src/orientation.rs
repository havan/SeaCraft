//! Viewport orientation overlay: axis triad and flat-view edge labels.

use eframe::egui::{self, Align2, Pos2, Rect, Stroke};
use seacraft_core::math::Vec3;

use crate::camera::ViewTransform;
use crate::theme;

/// How a world axis projects onto the viewport plane.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Facing {
    InPlane,
    Toward,
    Away,
}

/// One projected axis of the corner triad.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AxisGlyph {
    pub label: &'static str,
    pub dir: egui::Vec2,
    pub facing: Facing,
}

/// Which margin of a flat viewport cell an edge label sits on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
    Top,
    Bottom,
}

const AXIS_ARM_PX: f32 = 20.0;
const AXIS_LABEL_OFFSET_PX: f32 = 9.0;
const RING_RADIUS_PX: f32 = 4.5;

/// Project the three world axes onto `view` to describe the corner triad.
pub fn triad(view: &ViewTransform) -> [AxisGlyph; 3] {
    triad_tr(
        view,
        seacraft_core::i18n::Tr::new(seacraft_core::i18n::Locale::En),
    )
}

/// Project the three world axes onto `view` with localized axis labels.
pub fn triad_tr(view: &ViewTransform, tr: seacraft_core::i18n::Tr) -> [AxisGlyph; 3] {
    use seacraft_core::i18n::Key;
    [
        (Vec3::new(1.0, 0.0, 0.0), tr.s(Key::ViewOrientFwd)),
        (Vec3::new(0.0, 1.0, 0.0), tr.s(Key::ViewOrientStbd)),
        (Vec3::new(0.0, 0.0, 1.0), tr.s(Key::ViewOrientUp)),
    ]
    .map(|(axis, label)| {
        let sx = axis.dot(view.right);
        let sy = -axis.dot(view.up);
        let sz = axis.dot(view.forward);
        let len = sx.hypot(sy);
        if len < 1e-3 {
            let facing = if sz < 0.0 {
                Facing::Toward
            } else {
                Facing::Away
            };
            AxisGlyph {
                label,
                dir: egui::Vec2::ZERO,
                facing,
            }
        } else {
            AxisGlyph {
                label,
                dir: egui::vec2((sx / len) as f32, (sy / len) as f32),
                facing: Facing::InPlane,
            }
        }
    })
}

/// Return the margin labels (`Aft`/`Fwd`, `Port`/`Stbd`) aligned with a flat view's edges.
pub fn edge_labels(view: &ViewTransform) -> Vec<(Side, &'static str)> {
    edge_labels_tr(
        view,
        seacraft_core::i18n::Tr::new(seacraft_core::i18n::Locale::En),
    )
}

/// Return localized margin labels aligned with a flat view's edges.
pub fn edge_labels_tr(
    view: &ViewTransform,
    tr: seacraft_core::i18n::Tr,
) -> Vec<(Side, &'static str)> {
    use seacraft_core::i18n::Key;
    if view.perspective {
        return Vec::new();
    }
    let mut out = Vec::with_capacity(4);
    for (axis, neg, pos) in [
        (
            Vec3::new(1.0, 0.0, 0.0),
            tr.s(Key::ViewOrientAft),
            tr.s(Key::ViewOrientFwd),
        ),
        (
            Vec3::new(0.0, 1.0, 0.0),
            tr.s(Key::ViewOrientPort),
            tr.s(Key::ViewOrientStbd),
        ),
    ] {
        let rx = axis.dot(view.right);
        if rx.abs() > 0.999 {
            let (l, r) = if rx > 0.0 { (neg, pos) } else { (pos, neg) };
            out.push((Side::Left, l));
            out.push((Side::Right, r));
        }
        let uy = axis.dot(view.up);
        if uy.abs() > 0.999 {
            let (t, b) = if uy > 0.0 { (pos, neg) } else { (neg, pos) };
            out.push((Side::Top, t));
            out.push((Side::Bottom, b));
        }
    }
    out
}

/// Paint the axis triad and margin edge labels inside `cell`.
pub fn draw(painter: &egui::Painter, cell: Rect, view: &ViewTransform) {
    draw_tr(
        painter,
        cell,
        view,
        seacraft_core::i18n::Tr::new(seacraft_core::i18n::Locale::En),
    );
}

/// Paint the localized axis triad and margin edge labels inside `cell`.
pub fn draw_tr(
    painter: &egui::Painter,
    cell: Rect,
    view: &ViewTransform,
    tr: seacraft_core::i18n::Tr,
) {
    draw_triad(painter, cell, view, tr);
    for (side, text) in edge_labels_tr(view, tr) {
        let (pos, align) = match side {
            Side::Left => (
                egui::pos2(cell.left() + 8.0, cell.center().y),
                Align2::LEFT_CENTER,
            ),
            Side::Right => (
                egui::pos2(cell.right() - 8.0, cell.center().y),
                Align2::RIGHT_CENTER,
            ),
            // Clear of the top-left title, top-right grid caption and corner buttons.
            Side::Top => (
                egui::pos2(cell.center().x, cell.top() + 28.0),
                Align2::CENTER_TOP,
            ),
            // Clear of the bottom-left scale bar and bottom-centre axis-lock caption.
            Side::Bottom => (
                egui::pos2(cell.center().x, cell.bottom() - 28.0),
                Align2::CENTER_BOTTOM,
            ),
        };
        crate::render::hud_caption(painter, pos, align, text.to_owned());
    }
}

fn draw_triad(
    painter: &egui::Painter,
    cell: Rect,
    view: &ViewTransform,
    tr: seacraft_core::i18n::Tr,
) {
    // Shifted slightly inward from (-38, -34) so a 20 px arm + label stays inside `cell`.
    let origin = cell.right_bottom() + egui::vec2(-46.0, -40.0);
    let colours = [theme::AXIS_X, theme::AXIS_Y, theme::AXIS_Z];
    let glyphs = triad_tr(view, tr);

    let mut bounds = Rect::from_center_size(origin, egui::Vec2::splat(RING_RADIUS_PX * 2.0));
    let mut labels = Vec::with_capacity(3);
    let mut out_of_plane_row = 0;

    for (glyph, colour) in glyphs.into_iter().zip(colours) {
        let label_pos = match glyph.facing {
            Facing::InPlane => {
                let tip = origin + glyph.dir * AXIS_ARM_PX;
                bounds.extend_with(tip);
                tip + glyph.dir * AXIS_LABEL_OFFSET_PX
            }
            Facing::Toward | Facing::Away => {
                let pos = origin + egui::vec2(-8.0, 14.0 + out_of_plane_row as f32 * 13.0);
                out_of_plane_row += 1;
                pos
            }
        };
        let galley = painter.layout_no_wrap(
            glyph.label.to_owned(),
            theme::caption_font(),
            theme::TEXT_SECONDARY,
        );
        let text_rect = Align2::CENTER_CENTER.anchor_size(label_pos, galley.size());
        bounds |= text_rect;
        labels.push((glyph, colour, text_rect.min, galley));
    }

    painter.rect_filled(
        bounds.expand2(egui::vec2(4.0, 2.0)),
        2.0,
        theme::HUD_BACKING,
    );

    for (glyph, colour, text_min, galley) in labels {
        let stroke = Stroke::new(1.5, colour);
        match glyph.facing {
            Facing::InPlane => {
                let tip = origin + glyph.dir * AXIS_ARM_PX;
                painter.line_segment([origin, tip], stroke);
            }
            Facing::Toward => {
                painter.circle_stroke(origin, RING_RADIUS_PX, stroke);
                painter.circle_filled(origin, 2.0, colour);
            }
            Facing::Away => {
                painter.circle_stroke(origin, RING_RADIUS_PX, stroke);
                let d = 2.5;
                painter.line_segment(
                    [
                        Pos2::new(origin.x - d, origin.y - d),
                        Pos2::new(origin.x + d, origin.y + d),
                    ],
                    stroke,
                );
                painter.line_segment(
                    [
                        Pos2::new(origin.x - d, origin.y + d),
                        Pos2::new(origin.x + d, origin.y - d),
                    ],
                    stroke,
                );
            }
        }
        painter.galley(text_min, galley, theme::TEXT_SECONDARY);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, ViewKind};

    #[test]
    fn triad_and_edge_labels_match_each_view_basis() {
        // Closed form by hand from each camera's (right, up, forward) basis.
        let size = Vec3::new(12.0, 4.0, 2.0);
        let prof = Camera::new(ViewKind::Profile, Vec3::ZERO, size).transform();
        let t_prof = triad(&prof);
        assert_eq!(
            (t_prof[0].label, t_prof[0].facing),
            ("Fwd", Facing::InPlane)
        );
        assert!((t_prof[0].dir - egui::vec2(1.0, 0.0)).length() < 1e-5);
        assert_eq!(
            (t_prof[1].label, t_prof[1].facing, t_prof[1].dir),
            ("Stbd", Facing::Toward, egui::Vec2::ZERO)
        );
        assert_eq!((t_prof[2].label, t_prof[2].facing), ("Up", Facing::InPlane));
        assert!((t_prof[2].dir - egui::vec2(0.0, -1.0)).length() < 1e-5);
        assert_eq!(
            edge_labels(&prof),
            vec![(Side::Left, "Aft"), (Side::Right, "Fwd")]
        );

        let plan = Camera::new(ViewKind::Plan, Vec3::ZERO, size).transform();
        assert_eq!(triad(&plan)[2].facing, Facing::Toward);
        assert_eq!(
            edge_labels(&plan),
            vec![
                (Side::Left, "Aft"),
                (Side::Right, "Fwd"),
                (Side::Top, "Stbd"),
                (Side::Bottom, "Port"),
            ]
        );

        let body = Camera::new(ViewKind::Body, Vec3::ZERO, size).transform();
        assert_eq!(triad(&body)[0].facing, Facing::Away);
        assert_eq!(
            edge_labels(&body),
            vec![(Side::Left, "Port"), (Side::Right, "Stbd")]
        );

        let persp = Camera::new(ViewKind::Perspective, Vec3::ZERO, size).transform();
        assert!(edge_labels(&persp).is_empty());
    }
}
