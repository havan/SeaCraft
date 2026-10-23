//! Minimal vector / plane math.
//!
//! Coordinate system (same as FREE!ship, documented explicitly):
//! * `x` points forward (bow is +x)
//! * `y` points to starboard (the model is normally drawn for y > 0 and
//!   mirrored by the layer's `symmetric` flag)
//! * `z` points up, keel at low z
//!
//! Units are metres inside the core. Imperial input is converted at the edges.

use serde::Serialize;
use std::ops::{Add, AddAssign, Div, Mul, Neg, Sub, SubAssign};

pub const EPS: f64 = 1e-9;

#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Vec3 = Vec3 {
        x: 0.0,
        y: 0.0,
        z: 0.0,
    };

    pub fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn dot(self, o: Vec3) -> f64 {
        self.x * o.x + self.y * o.y + self.z * o.z
    }

    pub fn cross(self, o: Vec3) -> Vec3 {
        Vec3::new(
            self.y * o.z - self.z * o.y,
            self.z * o.x - self.x * o.z,
            self.x * o.y - self.y * o.x,
        )
    }

    pub fn length(self) -> f64 {
        self.dot(self).sqrt()
    }

    pub fn norm(self) -> Vec3 {
        let l = self.length();
        if l < EPS {
            Vec3::ZERO
        } else {
            self / l
        }
    }

    pub fn dist(self, o: Vec3) -> f64 {
        (self - o).length()
    }

    pub fn lerp(self, o: Vec3, t: f64) -> Vec3 {
        self + (o - self) * t
    }

    pub fn min(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.min(o.x), self.y.min(o.y), self.z.min(o.z))
    }

    pub fn max(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x.max(o.x), self.y.max(o.y), self.z.max(o.z))
    }

    /// Component-wise absolute value.
    pub fn abs(self) -> Vec3 {
        Vec3::new(self.x.abs(), self.y.abs(), self.z.abs())
    }

    pub fn is_finite(self) -> bool {
        self.x.is_finite() && self.y.is_finite() && self.z.is_finite()
    }
}

impl Add for Vec3 {
    type Output = Vec3;
    fn add(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x + o.x, self.y + o.y, self.z + o.z)
    }
}
impl AddAssign for Vec3 {
    fn add_assign(&mut self, o: Vec3) {
        *self = *self + o;
    }
}
impl Sub for Vec3 {
    type Output = Vec3;
    fn sub(self, o: Vec3) -> Vec3 {
        Vec3::new(self.x - o.x, self.y - o.y, self.z - o.z)
    }
}
impl SubAssign for Vec3 {
    fn sub_assign(&mut self, o: Vec3) {
        *self = *self - o;
    }
}
impl Mul<f64> for Vec3 {
    type Output = Vec3;
    fn mul(self, s: f64) -> Vec3 {
        Vec3::new(self.x * s, self.y * s, self.z * s)
    }
}
impl Div<f64> for Vec3 {
    type Output = Vec3;
    fn div(self, s: f64) -> Vec3 {
        Vec3::new(self.x / s, self.y / s, self.z / s)
    }
}
impl Neg for Vec3 {
    type Output = Vec3;
    fn neg(self) -> Vec3 {
        Vec3::new(-self.x, -self.y, -self.z)
    }
}

/// Plane in Hessian normal form: `n . p + d = 0`, `n` normalised.
///
/// The normal points to the "positive" side (above the waterline when used for
/// hydrostatics); points with a signed distance <= 0 are submerged.
#[derive(Debug, Clone, Copy)]
pub struct Plane {
    pub n: Vec3,
    pub d: f64,
}

impl Plane {
    pub fn new(n: Vec3, d: f64) -> Self {
        let len = n.length();
        if len < EPS {
            // No direction to normalise by: fall back to a horizontal plane
            // and keep `d` as given rather than dividing by ~0.
            return Self {
                n: Vec3::new(0.0, 0.0, 1.0),
                d,
            };
        }
        Self {
            n: n / len,
            d: d / len,
        }
    }

    /// Plane through three points; normal follows the right-hand rule.
    pub fn through(p1: Vec3, p2: Vec3, p3: Vec3) -> Self {
        let n = (p2 - p1).cross(p3 - p1).norm();
        Self { n, d: -n.dot(p1) }
    }

    /// Plane with a given normal through a point.
    pub fn with_normal_point(n: Vec3, p: Vec3) -> Self {
        let n = n.norm();
        Self { n, d: -n.dot(p) }
    }

    pub fn side(&self, p: Vec3) -> f64 {
        self.n.dot(p) + self.d
    }

    pub fn project(&self, p: Vec3) -> Vec3 {
        p - self.n * self.side(p)
    }

    /// Intersection parameter along the segment `a -> b` (assumes a crossing).
    pub fn segment_parameter(&self, a: Vec3, b: Vec3) -> f64 {
        let sa = self.side(a);
        let sb = self.side(b);
        if (sa - sb).abs() < EPS {
            0.5
        } else {
            sa / (sa - sb)
        }
    }

    /// Orthonormal basis `(u, v)` spanning the plane, `u` as close to +x as
    /// possible. For an upright waterplane this is exactly `(x, y)`.
    pub fn basis(&self) -> (Vec3, Vec3) {
        let x = Vec3::new(1.0, 0.0, 0.0);
        let mut u = x - self.n * self.n.dot(x);
        if u.length() < 1e-7 {
            u = Vec3::new(0.0, 1.0, 0.0) - self.n * self.n.y;
        }
        let u = u.norm();
        let v = self.n.cross(u).norm();
        (u, v)
    }
}

pub fn deg_to_rad(d: f64) -> f64 {
    d * std::f64::consts::PI / 180.0
}

pub fn rad_to_deg(r: f64) -> f64 {
    r * 180.0 / std::f64::consts::PI
}

/// Area of a planar polygon (fan triangulation), always non-negative.
pub fn polygon_area3(points: &[Vec3]) -> f64 {
    if points.len() < 3 {
        return 0.0;
    }
    let mut acc = Vec3::ZERO;
    let p0 = points[0];
    for i in 1..points.len() - 1 {
        acc += (points[i] - p0).cross(points[i + 1] - p0);
    }
    0.5 * acc.length()
}

/// Signed area of a 2D polygon (counter-clockwise positive).
pub fn polygon_area2(points: &[(f64, f64)]) -> f64 {
    polygon_moments_2d(points).area
}

/// Area, first moments and second moments of a planar polygon about the
/// origin of its 2D coordinate system.
#[derive(Debug, Clone, Copy, Default)]
pub struct PolygonMoments {
    /// Signed area.
    pub area: f64,
    /// `integral x dA`
    pub mx: f64,
    /// `integral y dA`
    pub my: f64,
    /// `integral y^2 dA`
    pub ix: f64,
    /// `integral x^2 dA`
    pub iy: f64,
}

impl PolygonMoments {
    pub fn from_polygon(points: &[(f64, f64)]) -> Self {
        let mut m = Self::default();
        let n = points.len();
        if n < 3 {
            return m;
        }
        for i in 0..n {
            let (x1, y1) = points[i];
            let (x2, y2) = points[(i + 1) % n];
            let cross = x1 * y2 - x2 * y1;
            m.area += cross;
            m.mx += (x1 + x2) * cross;
            m.my += (y1 + y2) * cross;
            m.ix += (y1 * y1 + y1 * y2 + y2 * y2) * cross;
            m.iy += (x1 * x1 + x1 * x2 + x2 * x2) * cross;
        }
        m.area *= 0.5;
        m.mx /= 6.0;
        m.my /= 6.0;
        m.ix /= 12.0;
        m.iy /= 12.0;
        m
    }

    pub fn accumulate(&mut self, other: &PolygonMoments) {
        self.area += other.area;
        self.mx += other.mx;
        self.my += other.my;
        self.ix += other.ix;
        self.iy += other.iy;
    }

    /// Every moment multiplied by `k` (`-1.0` reverses the orientation).
    pub fn scaled(&self, k: f64) -> Self {
        Self {
            area: self.area * k,
            mx: self.mx * k,
            my: self.my * k,
            ix: self.ix * k,
            iy: self.iy * k,
        }
    }
}

/// Even-odd point-in-polygon test in 2D.
pub fn point_in_polygon(p: (f64, f64), poly: &[(f64, f64)]) -> bool {
    let mut inside = false;
    let n = poly.len();
    for i in 0..n {
        let (a, b) = (poly[i], poly[(i + 1) % n]);
        if (a.1 > p.1) != (b.1 > p.1) {
            let x = a.0 + (p.1 - a.1) / (b.1 - a.1) * (b.0 - a.0);
            if p.0 < x {
                inside = !inside;
            }
        }
    }
    inside
}

/// The sign each loop of a set of closed 2D loops should carry: `+1` for an
/// outer boundary, `-1` for a hole (a loop inside an odd number of others).
/// Chained cuts come out in arbitrary directions, so the sign is taken from
/// the nesting, not from the winding.
pub fn loop_signs(loops: &[Vec<(f64, f64)>]) -> Vec<f64> {
    loops
        .iter()
        .enumerate()
        .map(|(i, l)| {
            let Some(&probe) = l.first() else {
                return 1.0;
            };
            let depth = loops
                .iter()
                .enumerate()
                .filter(|&(j, other)| j != i && other.len() >= 3 && point_in_polygon(probe, other))
                .count();
            if depth % 2 == 0 {
                1.0
            } else {
                -1.0
            }
        })
        .collect()
}

/// Moments of one loop with the orientation `sign` asks for.
pub fn oriented_moments(points: &[(f64, f64)], sign: f64) -> PolygonMoments {
    let m = PolygonMoments::from_polygon(points);
    if m.area * sign < 0.0 {
        m.scaled(-1.0)
    } else {
        m
    }
}

pub fn polygon_moments_2d(points: &[(f64, f64)]) -> PolygonMoments {
    PolygonMoments::from_polygon(points)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn loop_signs_count_separate_loops_and_subtract_holes() {
        let square = |x0: f64, y0: f64, s: f64, ccw: bool| {
            let mut v = vec![(x0, y0), (x0 + s, y0), (x0 + s, y0 + s), (x0, y0 + s)];
            if !ccw {
                v.reverse();
            }
            v
        };
        // Two pontoons walked in opposite directions: the areas add.
        let cat = vec![square(0.0, 0.0, 1.0, true), square(3.0, 0.0, 1.0, false)];
        let total: f64 = cat
            .iter()
            .zip(loop_signs(&cat))
            .map(|(l, s)| oriented_moments(l, s).area)
            .sum();
        assert!((total - 2.0).abs() < 1e-12, "{total}");
        // A 4 m² plate with a 1 m² hole, both walked counter-clockwise.
        let holed = vec![square(0.0, 0.0, 2.0, true), square(0.5, 0.5, 1.0, true)];
        let total: f64 = holed
            .iter()
            .zip(loop_signs(&holed))
            .map(|(l, s)| oriented_moments(l, s).area)
            .sum();
        assert!((total - 3.0).abs() < 1e-12, "{total}");
    }

    #[test]
    fn a_plane_without_a_normal_keeps_a_finite_offset() {
        let p = Plane::new(Vec3::ZERO, 5.0);
        assert!(p.d.is_finite());
        assert_eq!(p.n, Vec3::new(0.0, 0.0, 1.0));
    }

    #[test]
    fn plane_basis_is_orthonormal() {
        let p = Plane::new(Vec3::new(0.1, 0.2, 1.0), -3.0);
        let (u, v) = p.basis();
        assert!((u.length() - 1.0).abs() < 1e-12);
        assert!((v.length() - 1.0).abs() < 1e-12);
        assert!(u.dot(v).abs() < 1e-12);
        assert!(u.dot(p.n).abs() < 1e-12);
    }

    #[test]
    fn upright_plane_basis_matches_xy() {
        let p = Plane::new(Vec3::new(0.0, 0.0, 1.0), -2.0);
        let (u, v) = p.basis();
        assert!((u - Vec3::new(1.0, 0.0, 0.0)).length() < 1e-12);
        assert!((v - Vec3::new(0.0, 1.0, 0.0)).length() < 1e-12);
    }

    #[test]
    fn triangle_area() {
        let a = Vec3::new(0.0, 0.0, 0.0);
        let b = Vec3::new(1.0, 0.0, 0.0);
        let c = Vec3::new(0.0, 2.0, 0.0);
        assert!((polygon_area3(&[a, b, c]) - 1.0).abs() < 1e-12);
    }
}
