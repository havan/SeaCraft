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
    /// Vertical world extent visible in the viewport (orthographic only).
    pub ortho_height: f64,
    pub fov_deg: f64,
}

impl Camera {
    /// Camera framing a model with the given bounding size.
    pub fn new(kind: ViewKind, target: Vec3, size: Vec3) -> Self {
        let extent = size.length().max(MIN_EXTENT);
        Self {
            kind,
            target,
            yaw: 2.20,
            pitch: 0.30,
            distance: extent * 2.2,
            ortho_height: extent * 1.15,
            fov_deg: 45.0,
        }
    }

    /// Orthographic camera with an explicit visible height.
    pub fn ortho(kind: ViewKind, target: Vec3, height: f64) -> Self {
        Self {
            kind,
            target,
            yaw: 2.20,
            pitch: 0.30,
            distance: height * 2.0,
            ortho_height: height.max(MIN_EXTENT),
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
                (self.target + dir * self.distance, -dir, Vec3::new(0.0, 0.0, 1.0))
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
        let height = rect.height().max(1.0) as f64;
        let world_per_px = if view.perspective {
            2.0 * self.distance * view.fov_tan / height
        } else {
            self.ortho_height / height
        };
        self.target += view.right * (-dx as f64 * world_per_px)
            + view.up * (dy as f64 * world_per_px);
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

    /// Frame a model from its bounding box.
    pub fn frame(&mut self, min: Vec3, max: Vec3) {
        let extent = (max - min).length().max(MIN_EXTENT);
        self.target = (min + max) * 0.5;
        self.distance = (extent * 2.2).clamp(MIN_EXTENT, MAX_EXTENT);
        self.ortho_height = (extent * 1.15).clamp(MIN_EXTENT, MAX_EXTENT);
    }
}

/// Orthonormal camera basis plus projection scale.
#[derive(Debug, Clone, Copy)]
pub struct ViewTransform {
    pub eye: Vec3,
    pub right: Vec3,
    pub up: Vec3,
    /// Unit vector from the eye towards the target.
    pub forward: Vec3,
    pub ortho_height: f64,
    pub fov_tan: f64,
    pub perspective: bool,
}

impl ViewTransform {
    /// Project a world point into the viewport. `None` if it is behind a
    /// perspective camera or outside the depth range.
    pub fn project(&self, p: Vec3, rect: Rect) -> Option<Pos2> {
        let d = p - self.eye;
        let (x, y, z) = (d.dot(self.right), d.dot(self.up), d.dot(self.forward));
        let (sx, sy) = if self.perspective {
            if z <= 0.05 {
                return None;
            }
            let focal = rect.height() as f64 / (2.0 * self.fov_tan);
            (focal * x / z, focal * y / z)
        } else {
            let scale = rect.height() as f64 / self.ortho_height.max(1e-6);
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
        if self.perspective {
            let focal = rect.height() as f64 / (2.0 * self.fov_tan);
            Ray {
                origin: self.eye,
                dir: (self.forward + self.right * (cx / focal) + self.up * (cy / focal)).norm(),
            }
        } else {
            let world_per_px = self.ortho_height / rect.height().max(1.0) as f64;
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
            let p = view.project(camera.target, rect).expect("target must be visible");
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
        assert!((right.y - origin.y).abs() < 0.5, "+x must not move vertically");
        assert!(up.y < origin.y - 1.0, "+z must point up the screen");
        assert!((up.x - origin.x).abs() < 0.5, "+z must not move horizontally");
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
