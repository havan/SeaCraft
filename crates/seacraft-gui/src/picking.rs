//! Control-point picking and dragging.
//!
//! Picking is screen-space: the nearest projected control point within a few
//! pixels wins, the nearer one if two are equally close. Dragging happens on a
//! plane through the grabbed point facing the camera, so the point follows the
//! cursor exactly in orthographic views and sensibly in perspective.

use eframe::egui::{Pos2, Rect};
use seacraft_core::math::{Plane, Vec3};
use seacraft_core::Surface;

use crate::camera::{Ray, ViewTransform};
use crate::render::HiddenNet;
use crate::visibility::NetVisibility;

/// Pixels within which a control point can be grabbed.
pub const PICK_RADIUS: f32 = 9.0;

/// Index of the nearest projected point within `max_px` of `cursor`.
pub fn pick_point(
    view: &ViewTransform,
    rect: Rect,
    cursor: Pos2,
    pts: &[Vec3],
    max_px: f32,
    vis: Option<&NetVisibility>,
    mode: HiddenNet,
) -> Option<usize> {
    let vis = vis.filter(|_| mode != HiddenNet::Same);
    let mut best: Option<(usize, bool, f32, f64)> = None;
    for (i, p) in pts.iter().enumerate() {
        let Some(screen) = view.project(*p, rect) else {
            continue;
        };
        let d = screen.distance(cursor);
        if d > max_px {
            continue;
        }
        let hidden = vis.is_some_and(|v| !v.points.get(i).copied().unwrap_or(true));
        if hidden && mode == HiddenNet::Hidden {
            continue;
        }
        let depth = (*p - view.eye).length();
        if best.is_none_or(|(_, b_hid, bd, bz)| {
            (!hidden && b_hid) || (hidden == b_hid && (d < bd || (d == bd && depth < bz)))
        }) {
            best = Some((i, hidden, d, depth));
        }
    }
    best.map(|(i, _, _, _)| i)
}

/// Indices of control points whose screen projections in `cell` lie inside `rect`.
///
/// In [`HiddenNet::Dimmed`] and [`HiddenNet::Hidden`], points marked hidden in
/// `vis` are skipped; in [`HiddenNet::Same`] (or when `vis` is `None`), all
/// points inside `rect` are returned.
pub fn points_in_rect(
    surface: &Surface,
    view: &ViewTransform,
    cell: Rect,
    rect: Rect,
    vis: Option<&NetVisibility>,
    mode: HiddenNet,
) -> Vec<usize> {
    let vis = vis.filter(|_| mode != HiddenNet::Same);
    let mut out = Vec::new();
    for (i, cp) in surface.points.iter().enumerate() {
        if vis.is_some_and(|v| !v.points.get(i).copied().unwrap_or(true)) {
            continue;
        }
        let Some(pos) = view.project(cp.p, cell) else {
            continue;
        };
        if rect.contains(pos) {
            out.push(i);
        }
    }
    out
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
            pick_point(
                &view,
                rect(),
                on_middle,
                &pts,
                PICK_RADIUS,
                None,
                HiddenNet::Dimmed
            ),
            Some(1)
        );
        // 30 px away from any point: nothing.
        let far = pos2(on_middle.x, on_middle.y - 30.0);
        assert_eq!(
            pick_point(
                &view,
                rect(),
                far,
                &pts,
                PICK_RADIUS,
                None,
                HiddenNet::Dimmed
            ),
            None
        );
    }

    #[test]
    fn pick_point_prefers_visible_in_dimmed_skips_in_hidden_and_ignores_in_same() {
        // Closed form by hand: ortho Profile view in an 800x600 rect with ortho_height = 8.0
        // has scale = 600 / 8 = 75 px/m along +x (right). At cursor = (400, 300) (the projection
        // of (6, 0, 1)), a hidden point at x = 6.0 + 2/75 projects 2 px right of cursor, and a
        // visible point at x = 6.0 + 5/75 projects 5 px right of cursor (both within max_px = 9).
        let camera = Camera::ortho(ViewKind::Profile, Vec3::new(6.0, 0.0, 1.0), 8.0);
        let view = camera.transform();
        let r = rect();
        let cursor = view.project(Vec3::new(6.0, 0.0, 1.0), r).unwrap();
        let world_per_px = 8.0 / r.height() as f64;
        let pts = [
            Vec3::new(6.0 + 2.0 * world_per_px, -1.0, 1.0), // index 0: hidden, 2 px from cursor
            Vec3::new(6.0 + 5.0 * world_per_px, 1.0, 1.0),  // index 1: visible, 5 px from cursor
        ];
        let vis = NetVisibility {
            points: vec![false, true],
            hidden_spans: Vec::new(),
        };
        assert_eq!(
            pick_point(
                &view,
                r,
                cursor,
                &pts,
                PICK_RADIUS,
                Some(&vis),
                HiddenNet::Dimmed
            ),
            Some(1),
            "Dimmed mode must prefer the visible point within max_px over a closer hidden point"
        );
        assert_eq!(
            pick_point(
                &view,
                r,
                cursor,
                &pts,
                PICK_RADIUS,
                Some(&vis),
                HiddenNet::Same
            ),
            Some(0),
            "Same mode must pick the closer (hidden) point"
        );
        // When no visible point is within max_px (e.g. max_px = 3.0, only index 0 at 2 px is in reach):
        assert_eq!(
            pick_point(&view, r, cursor, &pts, 3.0, Some(&vis), HiddenNet::Dimmed),
            Some(0),
            "Dimmed mode must still pick a hidden point when no visible point is within max_px"
        );
        assert_eq!(
            pick_point(&view, r, cursor, &pts, 3.0, Some(&vis), HiddenNet::Hidden),
            None,
            "Hidden mode must skip hidden points"
        );
    }

    #[test]
    fn points_in_rect_respects_hidden_net_mode() {
        // Closed form by hand: ortho Profile view in an 800x600 rect centred at (6, 0, 1)
        // with ortho_height = 8.0 projects (6, ±1, 1) to (400, 300) and (20, 0, 1) outside
        // a 100x100 rect centred at (400, 300).
        let camera = Camera::ortho(ViewKind::Profile, Vec3::new(6.0, 0.0, 1.0), 8.0);
        let view = camera.transform();
        let cell = rect();
        let center = view.project(Vec3::new(6.0, 0.0, 1.0), cell).unwrap();
        let marquee = Rect::from_center_size(center, vec2(100.0, 100.0));

        let mut surface = Surface::new();
        surface.add_point(Vec3::new(6.0, -1.0, 1.0)); // 0: hidden
        surface.add_point(Vec3::new(6.0, 1.0, 1.0)); // 1: visible
        surface.add_point(Vec3::new(20.0, 0.0, 1.0)); // 2: visible, outside marquee

        let vis = NetVisibility {
            points: vec![false, true, true],
            hidden_spans: Vec::new(),
        };
        assert_eq!(
            points_in_rect(
                &surface,
                &view,
                cell,
                marquee,
                Some(&vis),
                HiddenNet::Dimmed
            ),
            vec![1]
        );
        assert_eq!(
            points_in_rect(
                &surface,
                &view,
                cell,
                marquee,
                Some(&vis),
                HiddenNet::Hidden
            ),
            vec![1]
        );
        assert_eq!(
            points_in_rect(&surface, &view, cell, marquee, Some(&vis), HiddenNet::Same),
            vec![0, 1]
        );
        assert_eq!(
            points_in_rect(&surface, &view, cell, marquee, None, HiddenNet::Dimmed),
            vec![0, 1]
        );
    }

    #[test]
    fn clicking_coincident_port_and_starboard_twins_in_profile_selects_starboard_in_dimmed_mode() {
        // Closed form by hand: on the full (unmirrored) demo hull at midship station x = 5.20,
        // the starboard bilge point (y > 0) and its port twin (y < 0) project to the same screen
        // position in Profile. Even when the port twin is nudged by 1 mm in x so its projection
        // is 0 px from the click and ~0.04 px closer than the starboard point (both well within
        // PICK_RADIUS = 9 px), clicking in Dimmed mode selects the visible starboard point.
        let ctx = eframe::egui::Context::default();
        let mut app = crate::App::headless_for_test(None);
        let run = |ctx: &eframe::egui::Context,
                   app: &mut crate::App,
                   events: Vec<eframe::egui::Event>| {
            let input = eframe::egui::RawInput {
                screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), vec2(1600.0, 1000.0))),
                events,
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| app.draw(ui));
            out.textures_delta.clear();
        };
        run(&ctx, &mut app, Vec::new());
        run(&ctx, &mut app, Vec::new());

        let surface = app.surface().unwrap();
        let (sb_idx, sb_pt) = surface
            .points
            .iter()
            .enumerate()
            .find(|(_, p)| (p.p.x - 5.20).abs() < 1e-6 && p.p.y > 1.0 && p.p.z < 0.8)
            .map(|(i, p)| (i, p.p))
            .expect("starboard midship bilge point");
        let pt_idx = surface
            .points
            .iter()
            .enumerate()
            .find(|(_, p)| {
                (p.p.x - sb_pt.x).abs() < 1e-6
                    && (p.p.y + sb_pt.y).abs() < 1e-6
                    && (p.p.z - sb_pt.z).abs() < 1e-6
            })
            .map(|(i, _)| i)
            .expect("port midship bilge twin");

        // Nudge the port twin by 1 mm in x so its projection is strictly closer to `pos`
        // than `sb_idx` while still coinciding within < 0.05 px on screen.
        let nudged = Vec3::new(sb_pt.x + 1e-3, -sb_pt.y, sb_pt.z);
        app.move_point(pt_idx, nudged).unwrap();
        run(&ctx, &mut app, Vec::new());

        let profile_rect = app.viewport_rect(1);
        let pos = app.cameras()[1]
            .transform()
            .project(nudged, profile_rect)
            .unwrap();
        let btn = |pressed| eframe::egui::Event::PointerButton {
            pos,
            button: eframe::egui::PointerButton::Primary,
            pressed,
            modifiers: eframe::egui::Modifiers::NONE,
        };
        run(
            &ctx,
            &mut app,
            vec![eframe::egui::Event::PointerMoved(pos), btn(true)],
        );
        run(&ctx, &mut app, vec![btn(false)]);
        run(&ctx, &mut app, Vec::new());

        assert_eq!(
            app.selected(),
            Some(sb_idx),
            "clicking coincident twins in Profile in Dimmed mode must select the visible starboard point ({sb_idx}), not the hidden port twin ({pt_idx})"
        );
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
