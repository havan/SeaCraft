//! Control-point picking and dragging.
//!
//! Picking is screen-space: the nearest projected control point within a few
//! pixels wins, the nearer one if two are equally close. Dragging happens on a
//! plane through the grabbed point facing the camera, so the point follows the
//! cursor exactly in orthographic views and sensibly in perspective.

use eframe::egui::{Pos2, Rect};
use seacraft_core::math::{Plane, Vec3};

use crate::camera::{Ray, ViewTransform};

/// Pixels within which a control point can be grabbed.
pub const PICK_RADIUS: f32 = 9.0;

/// Index of the nearest projected point within `max_px` of `cursor`.
pub fn pick_point(
    view: &ViewTransform,
    rect: Rect,
    cursor: Pos2,
    pts: &[Vec3],
    max_px: f32,
) -> Option<usize> {
    let mut best: Option<(usize, f32, f64)> = None;
    for (i, p) in pts.iter().enumerate() {
        let Some(screen) = view.project(*p, rect) else {
            continue;
        };
        let d = screen.distance(cursor);
        if d > max_px {
            continue;
        }
        let depth = (*p - view.eye).length();
        if best.is_none_or(|(_, bd, bz)| d < bd || (d == bd && depth < bz)) {
            best = Some((i, d, depth));
        }
    }
    best.map(|(i, _, _)| i)
}

/// The plane a point is dragged on: through `center`, facing the camera.
pub fn drag_plane(view: &ViewTransform, center: Vec3) -> Plane {
    Plane::with_normal_point(view.forward, center)
}

/// Where `ray` meets `plane`, if it does in front of the eye.
pub fn ray_plane(ray: &Ray, plane: &Plane) -> Option<Vec3> {
    let denom = plane.n.dot(ray.dir);
    if denom.abs() < 1e-9 {
        return None;
    }
    let t = -(plane.n.dot(ray.origin) + plane.d) / denom;
    (t > 0.0).then(|| ray.point_at(t))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, ViewKind};
    use eframe::egui::{pos2, vec2};

    fn rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 600.0))
    }

    #[test]
    fn picks_nearest_projected_point_within_threshold() {
        let camera = Camera::ortho(ViewKind::Profile, Vec3::new(6.0, 0.0, 1.0), 8.0);
        let view = camera.transform();
        let pts = [
            Vec3::new(5.0, 0.0, 1.0),
            Vec3::new(6.0, 0.0, 1.0),
            Vec3::new(7.0, 0.0, 1.0),
        ];
        let on_middle = view.project(pts[1], rect()).unwrap();
        assert_eq!(
            pick_point(&view, rect(), on_middle, &pts, PICK_RADIUS),
            Some(1)
        );
        // 30 px away from any point: nothing.
        let far = pos2(on_middle.x, on_middle.y - 30.0);
        assert_eq!(pick_point(&view, rect(), far, &pts, PICK_RADIUS), None);
    }

    #[test]
    fn drag_plane_moves_point_with_cursor() {
        let camera = Camera::ortho(ViewKind::Profile, Vec3::new(6.0, 0.0, 1.0), 8.0);
        let view = camera.transform();
        let p = Vec3::new(6.0, 0.0, 1.0);
        let plane = drag_plane(&view, p);
        let start = view.project(p, rect()).unwrap();
        let hit = ray_plane(&view.ray(start, rect()), &plane).expect("centre ray hits the point");
        assert!((hit - p).length() < 1e-9);
        // 10 px to the right moves the point 10 px worth of world units.
        let moved = ray_plane(&view.ray(pos2(start.x + 10.0, start.y), rect()), &plane).unwrap();
        let world_per_px = 8.0 / rect().height() as f64;
        assert!(
            (moved.x - p.x - 10.0 * world_per_px).abs() < 1e-9,
            "moved to {moved:?}"
        );
        assert!((moved.z - p.z).abs() < 1e-9);
    }
}
