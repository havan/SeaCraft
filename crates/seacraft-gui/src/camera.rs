//! Cameras and projection for the four viewports.
//!
//! World convention follows the core: `x` forward, `y` starboard, `z` up.
//! Each camera produces a [`ViewTransform`] — an orthonormal basis plus the
//! scale factors needed to map world points to a screen rectangle and back.
//!
//! Screen bases (all orthographic views):
//! * profile — looking to starboard (`forward = +y`), `+x` right, `+z` up
//! * plan    — looking down (`forward = -z`), `+x` right, `+y` up
//! * body    — looking aft from the bow (`forward = +x`), `+y` left, `+z` up
//!
//! The right vector always follows `right = forward × up`, which keeps the
//! basis right-handed and the projection free of accidental mirroring.

use eframe::egui::{Pos2, Rect};
use seacraft_core::math::Vec3;

/// Smallest and largest useful viewing extent, in metres.
const MIN_EXTENT: f64 = 0.05;
const MAX_EXTENT: f64 = 5_000.0;
/// A framed model fills 1 / 1.2 of the cell: a 10 % margin on each side.
const FRAME_MARGIN: f64 = 1.2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewKind {
    Perspective,
    Profile,
    Plan,
    Body,
}

impl ViewKind {
    pub const ALL: [ViewKind; 4] = [
        ViewKind::Perspective,
        ViewKind::Profile,
        ViewKind::Plan,
        ViewKind::Body,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ViewKind::Perspective => "Perspective",
            ViewKind::Profile => "Profile",
            ViewKind::Plan => "Plan",
            ViewKind::Body => "Body plan",
        }
    }

    pub fn is_perspective(self) -> bool {
        matches!(self, ViewKind::Perspective)
    }
}

#[derive(Debug, Clone)]
pub struct Camera {
    pub kind: ViewKind,
    pub target: Vec3,
    /// Orbit yaw in radians (perspective only).
    pub yaw: f64,
    /// Orbit pitch in radians, clamped away from the poles.
    pub pitch: f64,
    /// Eye distance from the target, in metres (perspective only).
    pub distance: f64,
    /// Vertical world extent that must be visible (orthographic only).
    pub ortho_height: f64,
    /// Width over height of what must be visible; `0` constrains the height
    /// only. An orthographic camera must show `ortho_height * fit_aspect`
    /// metres across, a perspective camera `fit_aspect` times its vertical
    /// field of view. A cell narrower than this aspect shrinks the scale
    /// rather than clipping the model.
    pub fit_aspect: f64,
    pub fov_deg: f64,
}

impl Camera {
    /// Camera framing a box of the given size centred on `target`.
    pub fn new(kind: ViewKind, target: Vec3, size: Vec3) -> Self {
        let extent = size.length().max(MIN_EXTENT);
        let mut camera = Self {
            kind,
            target,
            yaw: 2.20,
            pitch: 0.30,
            distance: extent * 2.2,
            ortho_height: extent * 1.15,
            fit_aspect: 0.0,
            fov_deg: 45.0,
        };
        let half = size * 0.5;
        camera.frame(target - half, target + half);
        camera
    }

    /// Orthographic camera with an explicit visible height and no width
    /// constraint.
    pub fn ortho(kind: ViewKind, target: Vec3, height: f64) -> Self {
        Self {
            kind,
            target,
            yaw: 2.20,
            pitch: 0.30,
            distance: height * 2.0,
            ortho_height: height.max(MIN_EXTENT),
            fit_aspect: 0.0,
            fov_deg: 45.0,
        }
    }

    /// Build the projection basis for the current parameters.
    pub fn transform(&self) -> ViewTransform {
        let (eye, forward, up_hint) = match self.kind {
            ViewKind::Perspective => {
                let dir = Vec3::new(
                    self.pitch.cos() * self.yaw.cos(),
                    self.pitch.cos() * self.yaw.sin(),
                    self.pitch.sin(),
                );
                (
                    self.target + dir * self.distance,
                    -dir,
                    Vec3::new(0.0, 0.0, 1.0),
                )
            }
            // Far eye points keep the orthographic views depth-independent.
            ViewKind::Profile => (
                self.target - Vec3::new(0.0, 1.0, 0.0) * 1000.0,
                Vec3::new(0.0, 1.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
            ViewKind::Plan => (
                self.target + Vec3::new(0.0, 0.0, 1.0) * 1000.0,
                Vec3::new(0.0, 0.0, -1.0),
                Vec3::new(0.0, 1.0, 0.0),
            ),
            ViewKind::Body => (
                self.target - Vec3::new(1.0, 0.0, 0.0) * 1000.0,
                Vec3::new(1.0, 0.0, 0.0),
                Vec3::new(0.0, 0.0, 1.0),
            ),
        };

        let forward = forward.norm();
        let mut right = forward.cross(up_hint);
        if right.length() < 1e-9 {
            // Looking straight along the up hint: pick another reference.
            right = forward.cross(Vec3::new(1.0, 0.0, 0.0));
        }
        let right = right.norm();
        let up = right.cross(forward).norm();

        ViewTransform {
            eye,
            right,
            up,
            forward,
            ortho_height: self.ortho_height,
            fit_aspect: self.fit_aspect,
            fov_tan: (self.fov_deg.to_radians() * 0.5).tan(),
            perspective: self.kind.is_perspective(),
        }
    }

    /// Orbit around the target. `dx`/`dy` are drag deltas in pixels.
    pub fn orbit(&mut self, dx: f32, dy: f32) {
        const SENSITIVITY: f64 = 0.01;
        self.yaw += dx as f64 * SENSITIVITY;
        // Keep the eye away from the poles where the basis degenerates.
        self.pitch = (self.pitch + dy as f64 * SENSITIVITY).clamp(-1.5, 1.5);
    }

    /// Pan the target in the view plane. `dx`/`dy` are drag deltas in pixels.
    pub fn pan(&mut self, dx: f32, dy: f32, rect: Rect) {
        let view = self.transform();
        let world_per_px = view.world_per_px(rect, self.distance);
        self.target +=
            view.right * (-dx as f64 * world_per_px) + view.up * (dy as f64 * world_per_px);
    }

    /// Zoom by a multiplicative factor; values below 1.0 move closer.
    pub fn zoom(&mut self, factor: f64) {
        let factor = factor.clamp(0.05, 20.0);
        if self.kind.is_perspective() {
            self.distance = (self.distance * factor).clamp(MIN_EXTENT, MAX_EXTENT);
        } else {
            self.ortho_height = (self.ortho_height * factor).clamp(MIN_EXTENT, MAX_EXTENT);
        }
    }

    /// Zoom by `factor` keeping the world point under `cursor` where it is
    /// on screen. In perspective the point is taken on the plane through the
    /// target, facing the camera.
    pub fn zoom_at(&mut self, factor: f64, cursor: Pos2, rect: Rect) {
        let view = self.transform();
        let before = view.world_per_px(rect, self.distance);
        self.zoom(factor);
        let after = self.transform().world_per_px(rect, self.distance);
        let cx = (cursor.x - rect.center().x) as f64;
        let cy = (rect.center().y - cursor.y) as f64;
        // The cursor's point is target + (right cx + up cy) * world_per_px;
        // move the target so it stays put at the new scale.
        self.target += (view.right * cx + view.up * cy) * (before - after);
    }

    /// Frame a model from its bounding box: the box's projection onto this
    /// view, with a 10 % margin on each side, fills the cell's height or its
    /// width, whichever is tighter.
    pub fn frame(&mut self, min: Vec3, max: Vec3) {
        let centre = (min + max) * 0.5;
        self.target = centre;
        // The basis depends on the kind and orbit angles only.
        let view = self.transform();
        let corners: Vec<Vec3> = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                ) - centre
            })
            .collect();
        let floor = MIN_EXTENT * 0.5;
        let mut half_w = floor;
        let mut half_h = floor;
        for r in &corners {
            half_w = half_w.max(r.dot(view.right).abs());
            half_h = half_h.max(r.dot(view.up).abs());
        }
        self.fit_aspect = half_w / half_h;
        self.ortho_height = (2.0 * half_h * FRAME_MARGIN).clamp(MIN_EXTENT, MAX_EXTENT);

        // Perspective: the nearest eye at which every corner lies inside the
        // vertical field of view and the `fit_aspect` times wider horizontal
        // one, margin included. A corner `toward` metres nearer the eye than
        // the target sits at depth `distance - toward`.
        let tan_v = view.fov_tan / FRAME_MARGIN;
        let tan_h = tan_v * self.fit_aspect;
        let extent = (max - min).length().max(MIN_EXTENT);
        let mut distance = MIN_EXTENT;
        for r in &corners {
            let toward = -r.dot(view.forward);
            let need = (r.dot(view.up).abs() / tan_v)
                .max(r.dot(view.right).abs() / tan_h)
                .max(0.1 * extent);
            distance = distance.max(toward + need);
        }
        self.distance = distance.clamp(MIN_EXTENT, MAX_EXTENT);
    }
}

/// Orthonormal camera basis plus projection scale.
///
/// The scale is aspect-aware: the camera asks for a world height
/// (`ortho_height`, or the vertical field of view) and a width
/// (`fit_aspect` times that), and a cell shows both, so the scale is the
/// smaller of the height fit and the width fit. [`ViewTransform::project`],
/// [`ViewTransform::ray`], [`Camera::pan`] and the m/px caption all take it
/// from [`ViewTransform::fit_px`].
#[derive(Debug, Clone, Copy)]
pub struct ViewTransform {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Unit vector from the eye towards the target.
    pub forward: Vec3,
    /// World height that must be visible (orthographic).
    pub ortho_height: f64,
    /// Width over height that must be visible; `0` for height only.
    pub fit_aspect: f64,
    /// Tangent of half the vertical field of view that must be visible.
    pub fov_tan: f64,
    pub perspective: bool,
}

impl ViewTransform {
    /// Pixels spanning the required height in a `w × h` cell: the height,
    /// or less when the cell is too narrow for the required width.
    pub fn fit_px_wh(&self, w: f64, h: f64) -> f64 {
        let (w, h) = (w.max(1.0), h.max(1.0));
        if self.fit_aspect > 1e-9 {
            h.min(w / self.fit_aspect)
        } else {
            h
        }
    }

    /// [`ViewTransform::fit_px_wh`] for a screen rectangle.
    pub fn fit_px(&self, rect: Rect) -> f64 {
        self.fit_px_wh(rect.width() as f64, rect.height() as f64)
    }

    /// Pixels per metre (orthographic) or the focal length in pixels
    /// (perspective).
    fn scale(&self, rect: Rect) -> f64 {
        let fit = self.fit_px(rect);
        if self.perspective {
            fit / (2.0 * self.fov_tan)
        } else {
            fit / self.ortho_height.max(1e-6)
        }
    }

    /// Metres per pixel; in perspective at view depth `depth`.
    pub fn world_per_px(&self, rect: Rect, depth: f64) -> f64 {
        if self.perspective {
            depth / self.scale(rect)
        } else {
            1.0 / self.scale(rect)
        }
    }

    /// World height across the full height of a `w × h` cell
    /// (orthographic), at least `ortho_height`.
    pub fn visible_height_wh(&self, w: f64, h: f64) -> f64 {
        self.ortho_height * h.max(1.0) / self.fit_px_wh(w, h)
    }

    /// Tangent of half the vertical field of view across the full height of
    /// a `w × h` cell (perspective), at least `fov_tan`.
    pub fn visible_fov_tan_wh(&self, w: f64, h: f64) -> f64 {
        self.fov_tan * h.max(1.0) / self.fit_px_wh(w, h)
    }

    /// Project a world point into the viewport. `None` if it is behind a
    /// perspective camera or outside the depth range.
    pub fn project(&self, p: Vec3, rect: Rect) -> Option<Pos2> {
        let d = p - self.eye;
        let (x, y, z) = (d.dot(self.right), d.dot(self.up), d.dot(self.forward));
        let scale = self.scale(rect);
        let (sx, sy) = if self.perspective {
            if z <= 0.05 {
                return None;
            }
            (scale * x / z, scale * y / z)
        } else {
            (x * scale, y * scale)
        };
        // Screen y grows downward, world up grows upward.
        Some(Pos2::new(
            rect.center().x + sx as f32,
            rect.center().y - sy as f32,
        ))
    }

    /// Ray through a screen position, into the world.
    pub fn ray(&self, pos: Pos2, rect: Rect) -> Ray {
        let cx = (pos.x - rect.center().x) as f64;
        let cy = (rect.center().y - pos.y) as f64;
        let scale = self.scale(rect);
        if self.perspective {
            Ray {
                origin: self.eye,
                dir: (self.forward + self.right * (cx / scale) + self.up * (cy / scale)).norm(),
            }
        } else {
            let world_per_px = 1.0 / scale;
            Ray {
                origin: self.eye + self.right * (cx * world_per_px) + self.up * (cy * world_per_px),
                dir: self.forward,
            }
        }
    }
}

/// A world-space ray.
#[derive(Debug, Clone, Copy)]
pub struct Ray {
    pub origin: Vec3,
    pub dir: Vec3,
}

impl Ray {
    /// Point at parameter `t` along the ray.
    pub fn point_at(&self, t: f64) -> Vec3 {
        self.origin + self.dir * t
    }

    /// Parameter along the ray closest to `p`.
    pub fn closest_t(&self, p: Vec3) -> f64 {
        (p - self.origin).dot(self.dir)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eframe::egui::{pos2, vec2};

    fn rect() -> Rect {
        Rect::from_min_size(pos2(0.0, 0.0), vec2(400.0, 300.0))
    }

    #[test]
    fn target_projects_to_view_centre() {
        let rect = rect();
        for kind in ViewKind::ALL {
            let camera = Camera::new(kind, Vec3::new(6.0, 0.0, 0.9), Vec3::new(12.0, 4.0, 1.7));
            let view = camera.transform();
            let p = view
                .project(camera.target, rect)
                .expect("target must be visible");
            assert!(
                (p.x - rect.center().x).abs() < 0.5 && (p.y - rect.center().y).abs() < 0.5,
                "{kind:?} target projected to {p:?}, centre {:?}",
                rect.center()
            );
        }
    }

    #[test]
    fn profile_has_x_right_and_z_up() {
        let rect = rect();
        let camera = Camera::ortho(ViewKind::Profile, Vec3::ZERO, 12.0);
        let view = camera.transform();
        let origin = view.project(Vec3::ZERO, rect).unwrap();
        let right = view.project(Vec3::new(1.0, 0.0, 0.0), rect).unwrap();
        let up = view.project(Vec3::new(0.0, 0.0, 1.0), rect).unwrap();

        assert!(right.x > origin.x + 1.0, "+x must point right");
        assert!(
            (right.y - origin.y).abs() < 0.5,
            "+x must not move vertically"
        );
        assert!(up.y < origin.y - 1.0, "+z must point up the screen");
        assert!(
            (up.x - origin.x).abs() < 0.5,
            "+z must not move horizontally"
        );
    }

    #[test]
    fn ray_through_projected_point_returns_that_point() {
        let rect = rect();
        for kind in ViewKind::ALL {
            let camera = Camera::new(kind, Vec3::new(6.0, 1.0, 0.9), Vec3::new(12.0, 4.0, 1.7));
            let view = camera.transform();
            let world = Vec3::new(6.0, 1.0, 0.9);
            let screen = view.project(world, rect).expect("point visible");
            let ray = view.ray(screen, rect);
            let hit = ray.point_at(ray.closest_t(world));
            assert!(
                (hit - world).length() < 1e-6,
                "{kind:?}: ray hit {hit:?} for {world:?}"
            );
        }
    }

    fn framed(kind: ViewKind) -> (Camera, Vec3, Vec3) {
        // The demo hull's proportions: long, narrow and shallow.
        let (min, max) = (Vec3::new(0.0, -2.0, 0.0), Vec3::new(12.0, 2.0, 1.7));
        let mut camera = Camera::ortho(kind, Vec3::ZERO, 1.0);
        camera.frame(min, max);
        (camera, min, max)
    }

    fn corners(min: Vec3, max: Vec3) -> Vec<Vec3> {
        (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                )
            })
            .collect()
    }

    fn shapes() -> [Rect; 3] {
        [
            Rect::from_min_size(pos2(10.0, 20.0), vec2(1200.0, 200.0)),
            Rect::from_min_size(pos2(10.0, 20.0), vec2(180.0, 900.0)),
            rect(),
        ]
    }

    #[test]
    fn ray_inverts_project_in_wide_and_tall_cells() {
        for kind in ViewKind::ALL {
            let (camera, min, max) = framed(kind);
            let view = camera.transform();
            for rect in shapes() {
                for world in corners(min, max)
                    .into_iter()
                    .chain([Vec3::new(3.0, 0.7, 0.4), Vec3::new(11.0, -1.5, 1.6)])
                {
                    let screen = view.project(world, rect).expect("visible");
                    let ray = view.ray(screen, rect);
                    let hit = ray.point_at(ray.closest_t(world));
                    // 1e-6 m, plus what rounding the screen point to f32
                    // costs at this scale (a few 1e-7 m in a 1200 px cell).
                    let depth = (world - view.eye).dot(view.forward);
                    let rounding = f64::from(f32::EPSILON)
                        * f64::from(screen.x.abs() + screen.y.abs())
                        * view.world_per_px(rect, depth);
                    assert!(
                        (hit - world).length() < 1e-6 + rounding,
                        "{kind:?} in {rect:?}: ray hit {hit:?} for {world:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn a_framed_box_fits_every_cell_shape_with_a_margin() {
        for kind in ViewKind::ALL {
            let (camera, min, max) = framed(kind);
            let view = camera.transform();
            for rect in shapes() {
                let inner = rect.shrink2(rect.size() * 0.05);
                let fill = rect.shrink2(rect.size() * 0.2);
                let mut reaches_out = false;
                for corner in corners(min, max) {
                    let p = view.project(corner, rect).expect("visible");
                    assert!(
                        inner.contains(p),
                        "{kind:?}: corner {corner:?} at {p:?} outside {inner:?}"
                    );
                    // The margin is about 10 %, not half the cell.
                    reaches_out |= !fill.contains(p);
                }
                assert!(
                    reaches_out,
                    "{kind:?} in {rect:?}: the box is framed too small"
                );
            }
        }
    }

    #[test]
    fn a_narrower_cell_shows_a_smaller_hull_not_less_of_it() {
        let (camera, _, _) = framed(ViewKind::Profile);
        let view = camera.transform();
        let wide = Rect::from_min_size(pos2(0.0, 0.0), vec2(800.0, 300.0));
        let narrow = Rect::from_min_size(pos2(0.0, 0.0), vec2(300.0, 300.0));
        assert!(view.world_per_px(narrow, 1.0) > view.world_per_px(wide, 1.0));
        // A height-only camera keeps the old behaviour.
        let plain = Camera::ortho(ViewKind::Profile, Vec3::ZERO, 8.0).transform();
        assert!((plain.world_per_px(narrow, 1.0) - 8.0 / 300.0).abs() < 1e-12);
    }

    #[test]
    fn zoom_at_keeps_the_world_point_under_the_cursor() {
        for kind in ViewKind::ALL {
            for rect in shapes() {
                let (mut camera, _, _) = framed(kind);
                let cursor = rect.min + rect.size() * vec2(0.8, 0.3);
                // The point under the cursor on the plane through the target.
                let view = camera.transform();
                let ray = view.ray(cursor, rect);
                let t = (camera.target - ray.origin).dot(view.forward) / ray.dir.dot(view.forward);
                let world = ray.point_at(t);
                for factor in [0.7, 0.5, 1.6] {
                    camera.zoom_at(factor, cursor, rect);
                    let p = camera.transform().project(world, rect).expect("visible");
                    assert!(
                        (p - cursor).length() < 0.5,
                        "{kind:?} in {rect:?}: zoom {factor} moved the point to {p:?}, cursor {cursor:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn zoom_stays_within_limits() {
        let mut camera = Camera::new(ViewKind::Perspective, Vec3::ZERO, Vec3::new(12.0, 4.0, 2.0));
        for _ in 0..200 {
            camera.zoom(0.5);
        }
        assert!(camera.distance >= MIN_EXTENT);
        for _ in 0..200 {
            camera.zoom(2.0);
        }
        assert!(camera.distance <= MAX_EXTENT);
    }
}
