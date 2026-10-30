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

/// Distance along `ray` to triangle `a b c` (Möller–Trumbore), either side.
fn ray_triangle(ray: &Ray, a: Vec3, b: Vec3, c: Vec3) -> Option<f64> {
    let e1 = b - a;
    let e2 = c - a;
    let p = ray.dir.cross(e2);
    let det = e1.dot(p);
    if det.abs() < 1e-12 {
        return None;
    }
    let inv = 1.0 / det;
    let s = ray.origin - a;
    let u = s.dot(p) * inv;
    if !(-1e-9..=1.0 + 1e-9).contains(&u) {
        return None;
    }
    let q = s.cross(e1);
    let v = ray.dir.dot(q) * inv;
    if v < -1e-9 || u + v > 1.0 + 1e-9 {
        return None;
    }
    let t = e2.dot(q) * inv;
    (t > 0.0).then_some(t)
}

/// The control face under `ray`: the nearest face of a visible layer that
/// the ray crosses, testing each polygon as a fan of triangles and, on a
/// symmetric layer, its mirror image too (clicking the port side of a half
/// model picks the starboard face it mirrors).
pub fn pick_face(ray: &Ray, surface: &seacraft_core::Surface) -> Option<usize> {
    let mut best: Option<(usize, f64)> = None;
    for (fi, face) in surface.faces.iter().enumerate() {
        let Some(layer) = surface
            .face_layer
            .get(fi)
            .and_then(|&l| surface.layers.get(l))
        else {
            continue;
        };
        if !layer.visible || face.len() < 3 {
            continue;
        }
        let Some(pts) = face
            .iter()
            .map(|&i| surface.points.get(i).map(|p| p.p))
            .collect::<Option<Vec<Vec3>>>()
        else {
            continue;
        };
        let sides: &[f64] = if layer.symmetric {
            &[1.0, -1.0]
        } else {
            &[1.0]
        };
        for &side in sides {
            let at = |i: usize| Vec3::new(pts[i].x, pts[i].y * side, pts[i].z);
            for k in 1..pts.len() - 1 {
                if let Some(t) = ray_triangle(ray, at(0), at(k), at(k + 1)) {
                    if best.is_none_or(|(_, bt)| t < bt) {
                        best = Some((fi, t));
                    }
                }
            }
        }
    }
    best.map(|(fi, _)| fi)
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
    fn picks_the_nearest_control_face_and_its_mirror() {
        let s = seacraft_core::demo::demo_hull(true);
        // Looking straight down into the open hull: the first face the ray
        // meets is the bottom face under it.
        let down = |x: f64, y: f64| Ray {
            origin: Vec3::new(x, y, 10.0),
            dir: Vec3::new(0.0, 0.0, -1.0),
        };
        let hit = pick_face(&down(5.0, 0.3), &s).expect("a face under the ray");
        let f = &s.faces[hit];
        // The hit face spans the ray's (x, y).
        let xs = f.iter().map(|&i| s.points[i].p.x);
        let (lo, hi) = xs.fold((f64::MAX, f64::MIN), |(a, b), x| (a.min(x), b.max(x)));
        assert!(lo <= 5.0 && 5.0 <= hi, "{lo}..{hi}");
        // The port side of a half model picks the same starboard face.
        assert_eq!(pick_face(&down(5.0, -0.3), &s), Some(hit));
        // Beside the hull: nothing.
        assert_eq!(pick_face(&down(5.0, 3.5), &s), None);
        // From astern, the transom is the first face met.
        let aft = Ray {
            origin: Vec3::new(-10.0, 0.4, 0.5),
            dir: Vec3::new(1.0, 0.0, 0.0),
        };
        assert_eq!(
            pick_face(&aft, &s).map(|f| s.face_layer[f]),
            Some(seacraft_core::demo::LAYER_TRANSOM)
        );
        // A hidden layer cannot be picked.
        let mut hidden = s.clone();
        hidden.layers[seacraft_core::demo::LAYER_TRANSOM].visible = false;
        assert_ne!(
            pick_face(&aft, &hidden).map(|f| hidden.face_layer[f]),
            Some(seacraft_core::demo::LAYER_TRANSOM)
        );
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
