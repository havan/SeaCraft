//! The heel view's geometry: the hull seen from the water's frame, and points picked on the
//! drawings back in model coordinates.
//!
//! The drawings keep the water level and turn the hull, so every point is measured in an
//! [`EarthFrame`] built from the solved waterplane: `up` its normal, `across` the direction the
//! righting lever is measured along (the lever's own `n × x̂`, to starboard when upright) and
//! `along` the third axis, forward. Heights are above the water (0 on the waterplane). The body
//! plan draws `(across, up)`, looking forward with starboard on the right; the profile draws
//! `(along, up)`, from starboard with the bow on the right. Nothing here paints.

use seacraft_core::curves::{station_polygons, station_xs};
use seacraft_core::hydro::Hull;
use seacraft_core::math::{Plane, Vec3};

/// A point of a drawing: `(horizontal, height above the water)`, metres.
pub type Point2 = [f64; 2];

/// An envelope of the profile: `(along, height)` points, aft to forward.
pub type Envelope = Vec<(f64, f64)>;

/// The water's frame at one solved waterplane.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct EarthFrame {
    /// The waterplane's normal: up.
    pub up: Vec3,
    /// Across the hull along the water surface: the righting lever's direction.
    pub across: Vec3,
    /// Along the hull, forward: `across × up`.
    pub along: Vec3,
    /// The plane's `d` (`up · p + offset` is the height above the water).
    pub offset: f64,
}

impl EarthFrame {
    pub fn from_plane(plane: &Plane) -> Self {
        let up = plane.n;
        let across = up.cross(Vec3::new(1.0, 0.0, 0.0)).norm();
        Self {
            up,
            across,
            along: across.cross(up),
            offset: plane.d,
        }
    }

    /// `(along, across, height above the water)` of a model point.
    pub fn to_earth(&self, p: Vec3) -> (f64, f64, f64) {
        (
            self.along.dot(p),
            self.across.dot(p),
            self.up.dot(p) + self.offset,
        )
    }

    /// The model point at `across = c`, height `h`, on the section `x` (a body-plan pick).
    pub fn model_at_x(&self, c: f64, h: f64, x: f64) -> Option<Vec3> {
        solve3(
            [self.across, self.up, Vec3::new(1.0, 0.0, 0.0)],
            [c, h - self.offset, x],
        )
    }

    /// The model point at `along = a`, height `h`, in the plane `y` (a profile pick). `None`
    /// when that plane lies flat in the drawing (at ±90° heel).
    pub fn model_at_y(&self, a: f64, h: f64, y: f64) -> Option<Vec3> {
        solve3(
            [self.along, self.up, Vec3::new(0.0, 1.0, 0.0)],
            [a, h - self.offset, y],
        )
    }
}

/// Solve `rows[i] · p = rhs[i]` (Cramer's rule); `None` when the rows are (nearly) dependent.
fn solve3(rows: [Vec3; 3], rhs: [f64; 3]) -> Option<Vec3> {
    let [r1, r2, r3] = rows;
    let (c23, c31, c12) = (r2.cross(r3), r3.cross(r1), r1.cross(r2));
    let det = r1.dot(c23);
    if det.abs() < 1e-9 {
        return None;
    }
    Some((c23 * rhs[0] + c31 * rhs[1] + c12 * rhs[2]) / det)
}

/// The hull cut at stations, once per hull: the drawings project these at every heel.
#[derive(Debug, Clone, Default)]
pub struct HullOutlines {
    /// `(x, polylines)` aft to forward; open polylines are kept as they come.
    pub stations: Vec<(f64, Vec<Vec<Vec3>>)>,
}

impl HullOutlines {
    /// `n` stations at the midpoints of equal bands ([`station_xs`]) plus one just inside each end,
    /// so the profile reaches the hull's ends.
    pub fn new(hull: &Hull, n: usize) -> Self {
        let length = hull.model_max.x - hull.model_min.x;
        let inset = 1e-3 * length;
        let mut xs = vec![hull.model_min.x + inset];
        xs.extend(station_xs(hull, n));
        xs.push(hull.model_max.x - inset);
        Self {
            stations: xs
                .into_iter()
                .map(|x| (x, Self::section_at(hull, x)))
                .collect(),
        }
    }

    /// The hull's section at `x`, without empty polylines.
    pub fn section_at(hull: &Hull, x: f64) -> Vec<Vec<Vec3>> {
        let (_, polys) = station_polygons(hull, x);
        polys.into_iter().filter(|p| !p.is_empty()).collect()
    }
}

/// Polylines projected onto the body plan: `(across, height)`.
pub fn project_section(polys: &[Vec<Vec3>], frame: &EarthFrame) -> Vec<Vec<Point2>> {
    polys
        .iter()
        .map(|poly| {
            poly.iter()
                .map(|&p| {
                    let (_, c, h) = frame.to_earth(p);
                    [c, h]
                })
                .collect()
        })
        .collect()
}

/// Every station projected onto the body plan.
pub fn body_plan(outlines: &HullOutlines, frame: &EarthFrame) -> Vec<Vec<Point2>> {
    outlines
        .stations
        .iter()
        .flat_map(|(_, polys)| project_section(polys, frame))
        .collect()
}

/// The hull's side silhouette in the profile: per station, its highest and lowest point
/// `(along, height)`, as top and bottom envelopes sorted aft to forward. Stations with no outline
/// are left out of both.
pub fn profile_silhouette(outlines: &HullOutlines, frame: &EarthFrame) -> (Envelope, Envelope) {
    let mut top = Vec::new();
    let mut bottom = Vec::new();
    for (_, polys) in &outlines.stations {
        let pts = polys.iter().flatten().map(|&p| {
            let (a, _, h) = frame.to_earth(p);
            (a, h)
        });
        let mut hi: Option<(f64, f64)> = None;
        let mut lo: Option<(f64, f64)> = None;
        for p in pts {
            if hi.is_none_or(|q| p.1 > q.1) {
                hi = Some(p);
            }
            if lo.is_none_or(|q| p.1 < q.1) {
                lo = Some(p);
            }
        }
        if let (Some(hi), Some(lo)) = (hi, lo) {
            top.push(hi);
            bottom.push(lo);
        }
    }
    top.sort_by(|a, b| a.0.total_cmp(&b.0));
    bottom.sort_by(|a, b| a.0.total_cmp(&b.0));
    (top, bottom)
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::heel::{heel_state, MassCondition};
    use seacraft_core::hydro::{Hull, HydroOptions};
    use seacraft_core::math::Vec3;
    use seacraft_core::subdiv::SubdivisionOptions;
    use seacraft_core::{Layer, ProjectSettings, Surface};

    /// A creased box, 10 × 4 × 3 m, centred on the centreline, with or without its deck face
    /// (as `seacraft-core/tests/analytic.rs` builds it).
    fn box_surface(deck: bool) -> Surface {
        let (l, b, d) = (10.0, 4.0, 3.0);
        let mut s = Surface::new();
        s.layers = vec![Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Layer::default()
        }];
        s.settings = ProjectSettings {
            name: "box".into(),
            length: l,
            beam: b,
            draft: 1.0,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..ProjectSettings::default()
        };
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ] {
            s.add_point(Vec3::new(x, y - b * 0.5, z));
        }
        for face in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            if !deck && face == [4, 5, 6, 7] {
                continue;
            }
            s.add_face(face, 0);
        }
        s.initialize_trace();
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = seacraft_core::mesh::VertexKind::Corner;
        }
        s
    }

    fn floating_box(deck: bool) -> (Hull, HydroOptions, MassCondition) {
        let s = box_surface(deck);
        let sub = SubdivisionOptions {
            level: 1,
            mode: Default::default(),
        };
        let base = HydroOptions::from_surface(&s, sub);
        let hull = Hull::new(&s, sub);
        let mass = MassCondition::bare(&hull, &base, 1.6, Some(5.0)).unwrap();
        (hull, base, mass)
    }

    fn frame_at(heel: f64) -> (Hull, EarthFrame) {
        let (hull, base, mass) = floating_box(true);
        let s = heel_state(&hull, &mass, heel, &base, 0.0).unwrap();
        (hull, EarthFrame::from_plane(&s.plane))
    }

    #[test]
    fn earth_frame_is_orthonormal_and_water_is_zero() {
        for heel in [0.0, 25.0, 80.0] {
            let (_, f) = frame_at(heel);
            for v in [f.up, f.across, f.along] {
                assert!((v.length() - 1.0).abs() < 1e-12, "{heel}°");
            }
            assert!(f.up.dot(f.across).abs() < 1e-12);
            assert!(f.up.dot(f.along).abs() < 1e-12);
            assert!(f.across.dot(f.along).abs() < 1e-12);
            // Right-handed, as x, y, z: along × across = up.
            assert!((f.along.cross(f.across) - f.up).length() < 1e-12);
            // A point on the waterplane is at height 0.
            let on = f.up * (-f.offset);
            assert!(f.to_earth(on).2.abs() < 1e-12, "{heel}°");
        }
    }

    #[test]
    fn round_trip_model_at_x() {
        for heel in [0.0, 25.0, 80.0] {
            let (_, f) = frame_at(heel);
            let p = Vec3::new(3.0, 1.2, 2.1);
            let (_, c, h) = f.to_earth(p);
            let q = f.model_at_x(c, h, p.x).unwrap();
            assert!((q - p).length() < 1e-9, "{heel}°: {q:?}");
        }
    }

    #[test]
    fn round_trip_model_at_y() {
        for heel in [0.0, 25.0, 80.0] {
            let (_, f) = frame_at(heel);
            let p = Vec3::new(3.0, 0.0, 2.1);
            let (a, _, h) = f.to_earth(p);
            let q = f.model_at_y(a, h, 0.0).unwrap();
            assert!((q - p).length() < 1e-9, "{heel}°: {q:?}");
        }
    }

    #[test]
    fn positive_heel_puts_starboard_down() {
        let (_, f) = frame_at(20.0);
        let stbd = f.to_earth(Vec3::new(5.0, 1.5, 1.0)).2;
        let port = f.to_earth(Vec3::new(5.0, -1.5, 1.0)).2;
        assert!(stbd < port, "starboard {stbd} vs port {port}");
        // Looking forward, starboard is on the right: larger `across`.
        assert!(f.to_earth(Vec3::new(5.0, 1.5, 1.0)).1 > f.to_earth(Vec3::new(5.0, -1.5, 1.0)).1);
    }

    #[test]
    fn box_profile_envelope_is_the_box() {
        let (hull, f) = frame_at(0.0);
        let outlines = HullOutlines::new(&hull, 25);
        let (top, bottom) = profile_silhouette(&outlines, &f);
        assert!(top.len() >= 25 && bottom.len() == top.len());
        for &(_, h) in &top {
            assert!((h - 2.0).abs() < 1e-6, "top {h}");
        }
        for &(_, h) in &bottom {
            assert!((h + 1.0).abs() < 1e-6, "bottom {h}");
        }
        // Sorted aft to forward, and spanning nearly the whole length.
        assert!(top.windows(2).all(|w| w[0].0 <= w[1].0));
        assert!(top.last().unwrap().0 - top[0].0 > 9.9);
    }

    #[test]
    fn open_deck_outline_does_not_panic() {
        let (hull, base, mass) = floating_box(false);
        let s = heel_state(&hull, &mass, 60.0, &base, 0.0).unwrap();
        let f = EarthFrame::from_plane(&s.plane);
        let outlines = HullOutlines::new(&hull, 25);
        let lines = body_plan(&outlines, &f);
        assert!(!lines.is_empty());
        let section = project_section(&HullOutlines::section_at(&hull, 5.0), &f);
        assert!(section.iter().all(|l| !l.is_empty()));
        let (top, bottom) = profile_silhouette(&outlines, &f);
        assert_eq!(top.len(), bottom.len());
    }
}
