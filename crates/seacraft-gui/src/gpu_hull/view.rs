//! View uniforms that reproduce [`ViewTransform::project`] on the GPU.
//!
//! Positions are sent relative to the geometry's origin, so `f32` keeps its
//! precision far from the world origin. The matrix is built in `f64` and
//! corrects for egui rounding the callback's viewport to whole pixels.

use eframe::egui::{Color32, Rect};
use seacraft_core::math::Vec3;

use crate::camera::ViewTransform;
use crate::render::key_light;

/// The nearest depth drawn in perspective, as in [`ViewTransform::project`].
const PERSPECTIVE_NEAR: f64 = 0.05;

/// What the hull shaders read. 128 bytes, no padding; the WGSL struct mirrors it.
#[repr(C)]
#[derive(Clone, Copy, Debug, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct ViewUniform {
    /// Column major, from positions relative to the geometry's origin.
    pub clip_from_local: [[f32; 4]; 4],
    /// Relative to the origin; `w` is 1 in perspective and 0 otherwise.
    pub eye: [f32; 4],
    /// `-forward`.
    pub ortho_to_eye: [f32; 4],
    pub light: [f32; 4],
    /// Premultiplied, gamma space, 0..1.
    pub wire_colour: [f32; 4],
}

/// The pixel edges `(left, top, right, bottom)` egui gives a callback for
/// `rect`: each edge rounded, as `epaint::ViewportInPixels::from_points` does.
/// That also clamps to the screen, which this has no size for, so a rect that
/// leaves the screen is wider here than what egui draws.
fn pixel_edges(rect: Rect, pixels_per_point: f32) -> (f32, f32, f32, f32) {
    let edge = |v: f32| (pixels_per_point * v).round();
    let (l, t) = (edge(rect.min.x), edge(rect.min.y));
    (l, t, edge(rect.max.x).max(l), edge(rect.max.y).max(t))
}

/// What egui's callback viewport covers, in points.
pub fn snapped_rect(rect: Rect, pixels_per_point: f32) -> Rect {
    let (l, t, r, b) = pixel_edges(rect, pixels_per_point);
    Rect::from_min_max(
        (l / pixels_per_point, t / pixels_per_point).into(),
        (r / pixels_per_point, b / pixels_per_point).into(),
    )
}

/// Size in pixels of the offscreen target for `rect`; `None` when either side
/// is 0 px.
pub fn target_size(rect: Rect, pixels_per_point: f32) -> Option<[u32; 2]> {
    let (l, t, r, b) = pixel_edges(rect, pixels_per_point);
    let (w, h) = ((r - l) as u32, (b - t) as u32);
    (w > 0 && h > 0).then_some([w, h])
}

fn box_corners(min: Vec3, max: Vec3) -> impl Iterator<Item = Vec3> {
    (0..8).map(move |i| {
        Vec3::new(
            if i & 1 == 0 { min.x } else { max.x },
            if i & 2 == 0 { min.y } else { max.y },
            if i & 4 == 0 { min.z } else { max.z },
        )
    })
}

/// `(near, far)` along the view direction enclosing the box. The 0.05 m
/// near and `near > 0` bind perspective only: orthographic depth is linear.
/// `far > near` always.
pub fn depth_range(view: &ViewTransform, min: Vec3, max: Vec3) -> (f64, f64) {
    let (mut zmin, mut zmax) = (f64::INFINITY, f64::NEG_INFINITY);
    for c in box_corners(min, max) {
        let z = (c - view.eye).dot(view.forward);
        zmin = zmin.min(z);
        zmax = zmax.max(z);
    }
    let pad = 0.01 * (zmax - zmin) + 1e-3;
    if view.perspective {
        let near = PERSPECTIVE_NEAR.max(zmin - pad);
        (near, (zmax + pad).max(near + 1.0))
    } else {
        (zmin - pad, zmax + pad)
    }
}

/// The uniform for drawing geometry inside `min..max` (world) into `rect`.
pub fn view_uniform(
    view: &ViewTransform,
    rect: Rect,
    pixels_per_point: f32,
    origin: Vec3,
    min: Vec3,
    max: Vec3,
    wire: Color32,
) -> ViewUniform {
    let snapped = snapped_rect(rect, pixels_per_point);
    let (rw, rh) = (
        snapped.width().max(1e-6) as f64,
        snapped.height().max(1e-6) as f64,
    );
    let scale = view.scale(rect);
    let (c, s) = (rect.center(), snapped.center());
    let bx = 2.0 * (c.x - s.x) as f64 / rw;
    let by = 2.0 * (s.y - c.y) as f64 / rh;
    let (ax, ay) = (2.0 * scale / rw, 2.0 * scale / rh);
    let (near, far) = depth_range(view, min, max);
    let (zk, zc) = if view.perspective {
        (far / (far - near), -far * near / (far - near))
    } else {
        (1.0 / (far - near), -near / (far - near))
    };

    // With d = p - eye and p = origin + q: each clip row is a vector over q
    // and a constant, the constant being -(vector . (eye - origin)).
    let e = view.eye - origin;
    let (r, u, f) = (view.right, view.up, view.forward);
    let w_row = if view.perspective { f } else { Vec3::ZERO };
    let x_row = r * ax + w_row * bx;
    let y_row = u * ay + w_row * by;
    let z_row = f * zk;
    // Orthographic `w` is the constant 1, so `bx * w` joins the constants.
    let (bx_c, by_c) = if view.perspective {
        (0.0, 0.0)
    } else {
        (bx, by)
    };
    let consts = [
        -x_row.dot(e) + bx_c,
        -y_row.dot(e) + by_c,
        -z_row.dot(e) + zc,
        if view.perspective { -f.dot(e) } else { 1.0 },
    ];
    let rows = [x_row, y_row, z_row, w_row];
    let mut m = [[0f32; 4]; 4];
    for (row, v) in rows.iter().enumerate() {
        m[0][row] = v.x as f32;
        m[1][row] = v.y as f32;
        m[2][row] = v.z as f32;
        m[3][row] = consts[row] as f32;
    }

    let light = key_light(view);
    let lin = |v: Vec3, w: f32| [v.x as f32, v.y as f32, v.z as f32, w];
    let ch = |v: u8| f32::from(v) / 255.0;
    let [wr, wg, wb, wa] = wire.to_array();
    ViewUniform {
        clip_from_local: m,
        eye: lin(e, if view.perspective { 1.0 } else { 0.0 }),
        ortho_to_eye: lin(-f, 0.0),
        light: lin(light, 0.0),
        wire_colour: [ch(wr), ch(wg), ch(wb), ch(wa)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, ViewKind};
    use crate::theme;
    use eframe::egui::{epaint::ViewportInPixels, pos2};

    fn corners_and_centre(c: Vec3, size: Vec3) -> (Vec<Vec3>, Vec3, Vec3) {
        let (min, max) = (c - size * 0.5, c + size * 0.5);
        let mut pts: Vec<Vec3> = box_corners(min, max).collect();
        pts.push(c);
        (pts, min, max)
    }

    /// Clip position of `p`, as the GPU computes it.
    fn clip(u: &ViewUniform, p: Vec3, origin: Vec3) -> [f32; 4] {
        let q = [
            (p.x - origin.x) as f32,
            (p.y - origin.y) as f32,
            (p.z - origin.z) as f32,
            1.0,
        ];
        let mut out = [0f32; 4];
        for (row, o) in out.iter_mut().enumerate() {
            *o = (0..4).map(|col| u.clip_from_local[col][row] * q[col]).sum();
        }
        out
    }

    fn to_screen(u: &ViewUniform, p: Vec3, origin: Vec3, snapped: Rect) -> (f32, f32) {
        let c = clip(u, p, origin);
        let (nx, ny) = (c[0] / c[3], c[1] / c[3]);
        (
            snapped.center().x + nx * snapped.width() * 0.5,
            snapped.center().y - ny * snapped.height() * 0.5,
        )
    }

    fn camera(kind: ViewKind, c: Vec3, size: Vec3, fit_aspect: Option<f64>) -> Camera {
        let mut cam = Camera::new(kind, c, size);
        cam.frame_aspect(c - size * 0.5, c + size * 0.5, fit_aspect);
        cam
    }

    #[test]
    fn the_uniform_lands_where_project_does() {
        let size = Vec3::new(12.0, 4.0, 3.0);
        let cases = [
            (
                pos2(10.0, 20.0),
                pos2(510.0, 420.0),
                1.0,
                Vec3::ZERO,
                size,
                None,
            ),
            (
                pos2(0.0, 0.0),
                pos2(120.0, 500.0),
                1.0,
                Vec3::ZERO,
                size,
                Some(3.0),
            ),
            (
                pos2(10.3, 20.7),
                pos2(510.6, 420.2),
                1.25,
                Vec3::ZERO,
                size,
                None,
            ),
            (
                pos2(13.37, 21.91),
                pos2(499.2, 388.66),
                1.5,
                Vec3::ZERO,
                size,
                None,
            ),
            (
                pos2(10.0, 20.0),
                pos2(510.0, 420.0),
                1.0,
                Vec3::new(5000.0, 0.0, 0.0),
                Vec3::new(230.0, 32.0, 20.0),
                None,
            ),
        ];
        for (min, max, ppp, centre, size, fit) in cases {
            let rect = Rect::from_min_max(min, max);
            let snapped = snapped_rect(rect, ppp);
            for kind in ViewKind::ALL {
                let mut cam = camera(kind, centre, size, fit);
                if let Some(a) = fit {
                    if kind != ViewKind::Perspective {
                        cam.fit_aspect = a;
                    }
                }
                let view = cam.transform();
                let (pts, bmin, bmax) = corners_and_centre(centre, size);
                let u = view_uniform(&view, rect, ppp, centre, bmin, bmax, theme::WIREFRAME);
                for p in pts {
                    let want = view.project(p, rect).expect("in front of the camera");
                    let got = to_screen(&u, p, centre, snapped);
                    assert!(
                        (got.0 - want.x).abs() < 0.01 && (got.1 - want.y).abs() < 0.01,
                        "{kind:?} {rect:?} ppp {ppp}: {got:?} vs {want:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn the_body_view_stays_mirrored() {
        let size = Vec3::new(12.0, 4.0, 3.0);
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let view = camera(ViewKind::Body, Vec3::ZERO, size, None).transform();
        let (_, bmin, bmax) = corners_and_centre(Vec3::ZERO, size);
        let u = view_uniform(&view, rect, 1.0, Vec3::ZERO, bmin, bmax, theme::WIREFRAME);
        let p = Vec3::new(0.0, 1.0, 0.0);
        assert!(view.project(p, rect).unwrap().x > rect.center().x);
        assert!(to_screen(&u, p, Vec3::ZERO, rect).0 > rect.center().x);
    }

    #[test]
    fn snapped_rect_matches_egui() {
        let rects = [
            (10.0, 20.0, 510.0, 420.0),
            (10.3, 20.7, 510.6, 420.2),
            (0.4, 0.5, 100.5, 50.49),
            (13.37, 21.91, 499.2, 388.66),
        ];
        for (a, b, c, d) in rects {
            for ppp in [1.0f32, 1.25, 1.5, 2.0] {
                let rect = Rect::from_min_max(pos2(a, b), pos2(c, d));
                let v = ViewportInPixels::from_points(&rect, ppp, [10000, 10000]);
                let s = snapped_rect(rect, ppp);
                for (got, want) in [
                    (s.min.x * ppp, v.left_px),
                    (s.min.y * ppp, v.top_px),
                    (s.width() * ppp, v.width_px),
                    (s.height() * ppp, v.height_px),
                ] {
                    assert!(
                        (got - want as f32).abs() < 1e-3,
                        "{rect:?} ppp {ppp}: {got} vs {want}"
                    );
                }
            }
        }
    }

    #[test]
    fn target_size_is_none_for_an_empty_cell() {
        let r = |w: f32, h: f32| Rect::from_min_size(pos2(5.0, 5.0), (w, h).into());
        assert_eq!(target_size(r(0.0, 50.0), 1.0), None);
        assert_eq!(target_size(r(50.0, 0.0), 1.0), None);
        assert_eq!(target_size(r(0.2, 50.0), 1.0), None);
        let rect = r(100.4, 50.0);
        let s = snapped_rect(rect, 1.5);
        let got = target_size(rect, 1.5).unwrap();
        assert_eq!(
            got,
            [
                (s.width() * 1.5).round() as u32,
                (s.height() * 1.5).round() as u32
            ]
        );
    }

    #[test]
    fn depth_range_is_valid_inside_the_box_and_for_a_flat_hull() {
        let size = Vec3::new(12.0, 4.0, 3.0);
        let (_, min, max) = corners_and_centre(Vec3::ZERO, size);
        let mut cam = camera(ViewKind::Perspective, Vec3::ZERO, size, Some(1.25));
        cam.distance = 0.5; // the eye is inside the box
        let (near, far) = depth_range(&cam.transform(), min, max);
        assert_eq!(near, 0.05);
        assert!(far > near);

        let flat = Vec3::new(12.0, 4.0, 0.0);
        for kind in [ViewKind::Plan, ViewKind::Profile] {
            let (_, fmin, fmax) = corners_and_centre(Vec3::ZERO, flat);
            let (n, f) = depth_range(
                &camera(kind, Vec3::ZERO, flat, None).transform(),
                fmin,
                fmax,
            );
            assert!(f > n, "{kind:?}");
        }

        let mut far_cam = camera(ViewKind::Perspective, Vec3::ZERO, size, Some(1.25));
        far_cam.distance = 100.0;
        assert!(depth_range(&far_cam.transform(), min, max).0 > 0.05);
    }

    #[test]
    fn the_near_corner_maps_to_0_and_the_far_corner_to_1() {
        let size = Vec3::new(12.0, 4.0, 3.0);
        let rect = Rect::from_min_max(pos2(0.0, 0.0), pos2(400.0, 300.0));
        let (pts, min, max) = corners_and_centre(Vec3::ZERO, size);
        for kind in ViewKind::ALL {
            let view = camera(kind, Vec3::ZERO, size, None).transform();
            let u = view_uniform(&view, rect, 1.0, Vec3::ZERO, min, max, theme::WIREFRAME);
            let zs: Vec<f32> = pts[..8]
                .iter()
                .map(|&p| {
                    let c = clip(&u, p, Vec3::ZERO);
                    c[2] / c[3]
                })
                .collect();
            for z in &zs {
                assert!((-1e-4..=1.0 + 1e-4).contains(z), "{kind:?}: {z}");
            }
            let (lo, hi) = zs
                .iter()
                .fold((f32::MAX, f32::MIN), |(a, b), &z| (a.min(z), b.max(z)));
            assert!(lo < 0.05 && hi > 0.5, "{kind:?}: {lo}..{hi}");
        }
    }
}
