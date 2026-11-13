//! Full-size patterns from a hull: what a builder cuts from plywood to set the boat up.
//!
//! A station mould is the hull's section at one station, drawn smaller by the thickness of the
//! skin so that the planking laid over it ends up where the hull surface was drawn. Everything is
//! cut from the refined mesh, like the offsets table.
//!
//! # The deduction
//!
//! Each segment of a section lies in one triangle of the hull, with unit normal `n`. With the
//! cutting plane's unit normal `N`, the part of `n` in the plane is `n_p = n - (n . N) N`. The
//! inside of a skin of thickness `t` meets the plane `t / |n_p|` in from the surface: more than
//! `t` where the surface crosses the plane at a slant, as it does towards the ends of a boat.
//! `|n_p|` is not let below 0.2, and segments held there are counted and reported.
//!
//! Which side is "in" is read from the section's own outline, not from the way the model's faces
//! are wound, so a model drawn inside out is still deducted inward.
//!
//! Nothing here says a pattern is right for a boat: the lines are geometry, checked against
//! closed-form shapes in the tests, and `DISCLAIMER.md` applies.

use serde::Serialize;

use crate::hydro::{chain_paths, plane_segments, Hull};
use crate::math::{Plane, Vec3};
use crate::sheet::{Anchor, Label, PathKind, Sheet, SheetPath};
use crate::units;

/// Below this the in-plane part of a surface normal is held, so a surface lying nearly in the
/// cutting plane does not ask for an unbounded deduction.
const MIN_IN_PLANE: f64 = 0.2;
/// How far marks reach beyond the section, in metres.
const MARK_REACH_M: f64 = 0.02;
/// Letter heights on paper, in millimetres.
const MARK_LABEL_MM: f64 = 3.0;
const TITLE_LABEL_MM: f64 = 5.0;
/// A moved segment may run backwards by this much, in metres, before the mould line is said to
/// cross itself: under the width of the line it is drawn with.
const BACKWARDS_TOL_M: f64 = 2e-4;
/// The most stations one set may have.
pub const MAX_STATIONS: usize = 500;

/// A drawing plane: a point on it and the two unit axes of the drawing. The plane's normal is
/// `u x v`.
#[derive(Clone, Copy, Debug)]
pub struct Frame {
    /// The point drawn at (0, 0).
    pub origin: Vec3,
    /// The drawing's x axis.
    pub u: Vec3,
    /// The drawing's y axis.
    pub v: Vec3,
}

impl Frame {
    /// The plane across the hull at `x`, seen looking forward: starboard to the right, up is up,
    /// and heights are the model's own. Its normal points forward.
    pub fn station(x: f64) -> Frame {
        Frame {
            origin: Vec3::new(x, 0.0, 0.0),
            u: Vec3::new(0.0, 1.0, 0.0),
            v: Vec3::new(0.0, 0.0, 1.0),
        }
    }

    /// The plane's unit normal.
    pub fn normal(&self) -> Vec3 {
        self.u.cross(self.v).norm()
    }

    fn to_2d(self, p: Vec3) -> (f64, f64) {
        let d = p - self.origin;
        (d.dot(self.u), d.dot(self.v))
    }

    fn is_finite(&self) -> bool {
        [self.origin, self.u, self.v]
            .iter()
            .all(|p| p.x.is_finite() && p.y.is_finite() && p.z.is_finite())
    }
}

/// How the hull surface meets the plane along one segment.
#[derive(Clone, Copy, Debug)]
pub struct Meeting {
    /// The unit normal of the segment in the drawing, pointing out of the hull.
    pub outward: (f64, f64),
    /// The length of the surface normal's part in the plane, `|n_p|`, before any floor: 1 where
    /// the surface crosses the plane squarely.
    pub in_plane: f64,
    /// The surface normal's part along the plane's normal, `n . N`, with `n` pointing out of the
    /// hull: positive where the hull gets smaller in the direction the plane faces.
    pub along: f64,
}

/// One line of a section, in the drawing's coordinates.
#[derive(Clone, Debug)]
pub struct SectionPath {
    /// The points in order.
    pub points: Vec<(f64, f64)>,
    /// Whether the last point joins the first.
    pub closed: bool,
    /// `meetings[i]` belongs to the segment from `points[i]` to `points[i + 1]` (wrapping when
    /// closed). `None`: the triangle the segment came from was not found.
    pub meetings: Vec<Option<Meeting>>,
}

/// Twice the signed area of a polygon, positive when it runs anticlockwise.
fn area2(points: &[(f64, f64)]) -> f64 {
    let n = points.len();
    (0..n)
        .map(|i| {
            let (a, b) = (points[i], points[(i + 1) % n]);
            a.0 * b.1 - b.0 * a.1
        })
        .sum()
}

/// Cut the hull with the frame's plane: the section's lines in the drawing, each segment with
/// how the surface meets the plane there. Faces lying in the plane are left out, so a plane on a
/// flat end gives that end's outline and not its triangles. Empty when the plane misses the hull.
pub fn cut(hull: &Hull, frame: &Frame) -> Vec<SectionPath> {
    if !frame.is_finite() {
        return Vec::new();
    }
    let normal = frame.normal();
    let plane = Plane::new(normal, -normal.dot(frame.origin));
    // Each triangle on its own, so that a segment knows the triangle it lies in.
    let mut segments = Vec::new();
    let mut facets: Vec<((f64, f64), Vec3)> = Vec::new();
    for tri in &hull.tris {
        let [a, b, c] = [hull.verts[tri[0]], hull.verts[tri[1]], hull.verts[tri[2]]];
        let in_plane = [a, b, c].iter().all(|p| plane.side(*p).abs() <= 1e-7);
        let n = (b - a).cross(c - a);
        if in_plane || n.length() < 1e-300 {
            continue;
        }
        for (p, q) in plane_segments(&hull.verts, std::slice::from_ref(tri), &plane) {
            let (p2, q2) = (frame.to_2d(p), frame.to_2d(q));
            facets.push((((p2.0 + q2.0) / 2.0, (p2.1 + q2.1) / 2.0), n.norm()));
            segments.push((p, q));
        }
    }
    let paths: Vec<(Vec<(f64, f64)>, bool)> = chain_paths(&segments, 1e-7)
        .into_iter()
        .map(|(points, closed)| (points.into_iter().map(|p| frame.to_2d(p)).collect(), closed))
        .collect();
    // A loop inside another is a hole: its inside is the other way.
    let loops: Vec<Vec<(f64, f64)>> = paths.iter().map(|(points, _)| points.clone()).collect();
    let nesting = crate::math::loop_signs(&loops);

    paths
        .into_iter()
        .zip(nesting)
        .map(|((points, closed), nest)| {
            let area = area2(&points);
            // +1 when the hull is on the left of the direction the path runs in.
            let turn = if area.abs() < 1e-12 {
                0.0
            } else {
                area.signum() * nest
            };
            let count = if closed {
                points.len()
            } else {
                points.len() - 1
            };
            let meetings = (0..count)
                .map(|i| {
                    let (p, q) = (points[i], points[(i + 1) % points.len()]);
                    let mid = ((p.0 + q.0) / 2.0, (p.1 + q.1) / 2.0);
                    let (_, n) = facets
                        .iter()
                        .map(|(m, n)| ((m.0 - mid.0).hypot(m.1 - mid.1), n))
                        .filter(|(d, _)| *d <= 1e-6)
                        .min_by(|a, b| a.0.total_cmp(&b.0))?;
                    let along = n.dot(normal);
                    let flat = (n.dot(frame.u), n.dot(frame.v));
                    let in_plane = flat.0.hypot(flat.1);
                    let length = (q.0 - p.0).hypot(q.1 - p.1);
                    // The segment's own normal, on the side the outline says is outside.
                    let outward = if turn != 0.0 && length > 0.0 {
                        (turn * (q.1 - p.1) / length, -turn * (q.0 - p.0) / length)
                    } else if in_plane > 0.0 {
                        (flat.0 / in_plane, flat.1 / in_plane)
                    } else {
                        return None;
                    };
                    // The model's normal may point in: read `along` for the one pointing out.
                    let agrees = flat.0 * outward.0 + flat.1 * outward.1 >= 0.0;
                    Some(Meeting {
                        outward,
                        in_plane,
                        along: if agrees { along } else { -along },
                    })
                })
                .collect();
            SectionPath {
                points,
                closed,
                meetings,
            }
        })
        .collect()
}

/// A section moved in by the skin.
#[derive(Clone, Debug)]
pub struct MouldLine {
    /// The moved lines, point for point with the section's, and whether each is closed. Two
    /// neighbouring points coincide where a very short segment closed up.
    pub paths: Vec<(Vec<(f64, f64)>, bool)>,
    /// How many segments met the plane so obliquely that their deduction was held at the floor.
    pub clamped: usize,
    /// Whether a moved line crosses itself or runs backwards: the skin is thicker than the
    /// hull's radius somewhere.
    pub crosses_itself: bool,
}

/// Where a vertex moves, given the outward normal and deduction of the segment before it and of
/// the segment after it.
fn vertex_offset(a: Option<((f64, f64), f64)>, b: Option<((f64, f64), f64)>) -> (f64, f64) {
    match (a, b) {
        (None, None) => (0.0, 0.0),
        (Some((m, d)), None) | (None, Some((m, d))) => (-d * m.0, -d * m.1),
        (Some((ma, da)), Some((mb, db))) => {
            let cross = ma.0 * mb.1 - ma.1 * mb.0;
            let dot = ma.0 * mb.0 + ma.1 * mb.1;
            if dot > 0.0 && cross.abs() < 10f64.to_radians().sin() {
                // Nearly straight: the mean of the two.
                (
                    -(da * ma.0 + db * mb.0) / 2.0,
                    -(da * ma.1 + db * mb.1) / 2.0,
                )
            } else if cross.abs() < 1e-9 {
                // Folded back on itself.
                (-da * ma.0, -da * ma.1)
            } else {
                // The mitre: the point that is `da` inside one segment and `db` inside the other.
                let v = (
                    (-da * mb.1 + db * ma.1) / cross,
                    (-db * ma.0 + da * mb.0) / cross,
                );
                let (len, cap) = (v.0.hypot(v.1), 4.0 * da.max(db));
                if len > cap {
                    (v.0 * cap / len, v.1 * cap / len)
                } else {
                    v
                }
            }
        }
    }
}

/// Whether segments `a0 a1` and `b0 b1` cross at a point inside both.
fn properly_cross(a0: (f64, f64), a1: (f64, f64), b0: (f64, f64), b1: (f64, f64)) -> bool {
    let side = |p: (f64, f64), q: (f64, f64), r: (f64, f64)| {
        (q.0 - p.0) * (r.1 - p.1) - (q.1 - p.1) * (r.0 - p.0)
    };
    let (d1, d2) = (side(b0, b1, a0), side(b0, b1, a1));
    let (d3, d4) = (side(a0, a1, b0), side(a0, a1, b1));
    d1 * d2 < 0.0 && d3 * d4 < 0.0
}

/// Move a section in by `skin_m`, each segment by `skin / max(|n_p|, 0.2)` along its own normal
/// and each vertex to where its two segments agree. A skin of zero, or less, returns the section.
pub fn mould_line(section: &[SectionPath], skin_m: f64) -> MouldLine {
    let mut out = MouldLine {
        paths: Vec::new(),
        clamped: 0,
        crosses_itself: false,
    };
    for path in section {
        if skin_m.is_nan() || skin_m <= 0.0 {
            out.paths.push((path.points.clone(), path.closed));
            continue;
        }
        let n = path.points.len();
        let moves: Vec<Option<((f64, f64), f64)>> = path
            .meetings
            .iter()
            .map(|m| m.map(|m| (m.outward, skin_m / m.in_plane.max(MIN_IN_PLANE))))
            .collect();
        out.clamped += path
            .meetings
            .iter()
            .flatten()
            .filter(|m| m.in_plane < MIN_IN_PLANE)
            .count();
        let segment = |i: usize| moves.get(i).copied().flatten();
        let moved: Vec<(f64, f64)> = (0..n)
            .map(|i| {
                let before = match (i, path.closed) {
                    (0, true) => segment(n - 1),
                    (0, false) => None,
                    _ => segment(i - 1),
                };
                let d = vertex_offset(before, segment(i));
                (path.points[i].0 + d.0, path.points[i].1 + d.1)
            })
            .collect();
        let mut moved = moved;
        // A segment that now runs the other way means the skin does not fit there. Where the
        // cutting plane clips the tip of a triangle the section has a segment a fraction of a
        // millimetre long, and a bend beside it turns that one over by less than a line's
        // width: those are closed up, not reported.
        let count = if path.closed { n } else { n - 1 };
        for i in 0..count {
            let j = (i + 1) % n;
            let (p, q) = (path.points[i], path.points[j]);
            let length = (q.0 - p.0).hypot(q.1 - p.1);
            if length == 0.0 {
                continue;
            }
            let along = ((q.0 - p.0) * (moved[j].0 - moved[i].0)
                + (q.1 - p.1) * (moved[j].1 - moved[i].1))
                / length;
            if along < -BACKWARDS_TOL_M {
                out.crosses_itself = true;
            } else if along < 0.0 {
                let middle = (
                    (moved[i].0 + moved[j].0) / 2.0,
                    (moved[i].1 + moved[j].1) / 2.0,
                );
                moved[i] = middle;
                moved[j] = middle;
            }
        }
        // Turned inside out as a whole, as a tube thinner than its skin is.
        if path.closed && area2(&path.points) * area2(&moved) < 0.0 {
            out.crosses_itself = true;
        }
        if !out.crosses_itself {
            let ends = |i: usize| (moved[i], moved[(i + 1) % n]);
            'pairs: for i in 0..count {
                for j in i + 2..count {
                    if path.closed && i == 0 && j == count - 1 {
                        continue;
                    }
                    let ((a0, a1), (b0, b1)) = (ends(i), ends(j));
                    if properly_cross(a0, a1, b0, b1) {
                        out.crosses_itself = true;
                        break 'pairs;
                    }
                }
            }
        }
        out.paths.push((moved, path.closed));
    }
    out
}

/// Which face of a mould goes on its station line, so that the mould needs no bevel to clear
/// the skin.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MouldFace {
    /// The hull gets smaller going forward here: the mould stands aft of its line.
    Forward,
    /// The hull gets smaller going aft here: the mould stands forward of its line.
    Aft,
    /// The hull is parallel here.
    Either,
}

/// What a pattern is drawn with.
#[derive(Clone, Debug, Default)]
pub struct TemplateOptions {
    /// The thickness of the skin to deduct, in metres. Zero draws the hull surface.
    pub skin_m: f64,
    /// Draw the starboard half only, to be flipped about the centreline.
    pub half: bool,
    /// The level of the strongback, a model height: open sections get legs up to it.
    pub strongback_z: Option<f64>,
    /// Waterlines to mark, as heights above the keel in metres.
    pub waterlines_m: Vec<f64>,
    /// The design waterline to mark, as a height above the keel in metres.
    pub dwl_m: Option<f64>,
    /// Add imperial readings beside the metric ones.
    pub imperial: bool,
}

/// One pattern and what was found while drawing it.
#[derive(Clone, Debug)]
pub struct Template {
    /// The drawing, its marks and its notes.
    pub sheet: Sheet,
    /// Where along the hull it is, for a station or a transom.
    pub x: Option<f64>,
    /// Which face of a station mould goes on its line.
    pub face: Option<MouldFace>,
    /// The largest angle between the surface and the plane's normal along the cut, in degrees.
    pub max_bevel_deg: f64,
    /// The cut line's largest distance from the centreline, in metres.
    pub half_breadth_m: f64,
    /// The longest straight piece of the hull's section, in metres: how coarse the mesh is.
    pub longest_segment_m: f64,
    /// What a builder should know before trusting this pattern.
    pub warnings: Vec<String>,
}

/// A section drawn as a pattern: the lines and what was measured, before marks and notes.
pub(crate) struct Outline {
    /// The lines to cut.
    pub(crate) cut: Vec<(Vec<(f64, f64)>, bool)>,
    /// The hull surface, when the cut line is inside it.
    pub(crate) surface: Vec<(Vec<(f64, f64)>, bool)>,
    /// The two ends of each open section line, on the cut line.
    pub(crate) sheer: Vec<(f64, f64)>,
    /// The length-weighted mean of `n . N` along the section.
    pub(crate) mean_along: f64,
    pub(crate) max_bevel_deg: f64,
    pub(crate) longest_segment_m: f64,
    /// `[min_u, min_v, max_u, max_v]` of the hull's section.
    pub(crate) bounds: [f64; 4],
    pub(crate) warnings: Vec<String>,
}

/// The pattern's lines for a section: the mould line `skin_m` inside the hull, each open line
/// closed straight across its ends (or by legs up to `legs_to`, a height in the drawing), and
/// the hull surface for reference. `plane` names the cutting plane in the oblique warning.
pub(crate) fn outline(
    section: &[SectionPath],
    skin_m: f64,
    legs_to: Option<f64>,
    plane: &str,
) -> Outline {
    let mould = mould_line(section, skin_m);
    let mut out = Outline {
        cut: Vec::new(),
        surface: Vec::new(),
        sheer: Vec::new(),
        mean_along: 0.0,
        max_bevel_deg: 0.0,
        longest_segment_m: 0.0,
        bounds: [f64::MAX, f64::MAX, f64::MIN, f64::MIN],
        warnings: Vec::new(),
    };
    let (mut weighted, mut total) = (0.0, 0.0);
    for path in section {
        let n = path.points.len();
        for (i, meeting) in path.meetings.iter().enumerate() {
            let (p, q) = (path.points[i], path.points[(i + 1) % n]);
            let length = (q.0 - p.0).hypot(q.1 - p.1);
            out.longest_segment_m = out.longest_segment_m.max(length);
            if let Some(m) = meeting {
                weighted += m.along * length;
                total += length;
                out.max_bevel_deg = out
                    .max_bevel_deg
                    .max(m.along.abs().min(1.0).asin().to_degrees());
            }
        }
        for (u, v) in &path.points {
            let b = &mut out.bounds;
            *b = [b[0].min(*u), b[1].min(*v), b[2].max(*u), b[3].max(*v)];
        }
    }
    if total > 0.0 {
        out.mean_along = weighted / total;
    }
    for (points, closed) in &mould.paths {
        let mut points = points.clone();
        if !closed {
            if let (Some(first), Some(last)) = (points.first().copied(), points.last().copied()) {
                out.sheer.extend([first, last]);
                if let Some(level) = legs_to {
                    points.extend([(last.0, level), (first.0, level)]);
                }
            }
        }
        // An open line is closed straight across: that edge is not hull, and was not moved.
        out.cut.push((points, true));
    }
    if skin_m > 0.0 {
        out.surface = section
            .iter()
            .map(|p| (p.points.clone(), p.closed))
            .collect();
    }
    if mould.crosses_itself {
        out.warnings.push(
            "skin is thicker than the hull's local radius here: the mould line crosses itself"
                .into(),
        );
    }
    if mould.clamped > 0 {
        out.warnings.push(format!(
            "{} segments are too oblique to the {plane} plane for a reliable deduction",
            mould.clamped
        ));
    }
    out
}

/// The parts of a line with `u >= 0`, as open runs cut at the centreline; the line itself when
/// none of it is on the other side.
fn starboard_runs(points: &[(f64, f64)], closed: bool) -> Vec<(Vec<(f64, f64)>, bool)> {
    const EPS: f64 = 1e-12;
    let inside = |p: &(f64, f64)| p.0 >= -EPS;
    if points.iter().all(inside) {
        return vec![(points.to_vec(), closed)];
    }
    // Start a closed line at a point on the other side, so that no run wraps around its end.
    let mut ring: Vec<(f64, f64)> = points.to_vec();
    if closed {
        let start = ring.iter().position(|p| !inside(p)).unwrap_or(0);
        ring.rotate_left(start);
        ring.push(ring[0]);
    }
    let mut runs = Vec::new();
    let mut run: Vec<(f64, f64)> = Vec::new();
    for pair in ring.windows(2) {
        let (a, b) = (pair[0], pair[1]);
        let crossing = || {
            let t = a.0 / (a.0 - b.0);
            (0.0, a.1 + t * (b.1 - a.1))
        };
        match (inside(&a), inside(&b)) {
            (true, true) => {
                if run.is_empty() {
                    run.push(a);
                }
                run.push(b);
            }
            (true, false) => {
                if run.is_empty() {
                    run.push(a);
                }
                if a.0 > EPS {
                    run.push(crossing());
                }
                runs.push(std::mem::take(&mut run));
            }
            (false, true) => {
                run.push(crossing());
                if b.0 > EPS {
                    run.push(b);
                }
            }
            (false, false) => {}
        }
    }
    runs.push(run);
    runs.into_iter()
        .filter(|r| r.len() >= 2)
        .map(|r| (r, false))
        .collect()
}

/// A straight mark between two points.
fn mark(a: (f64, f64), b: (f64, f64)) -> SheetPath {
    SheetPath {
        kind: PathKind::Mark,
        points: vec![a, b],
        closed: false,
    }
}

/// A label for a mark, 3 mm high.
fn mark_label(at: (f64, f64), anchor: Anchor, text: impl Into<String>) -> Label {
    Label {
        at,
        height_mm: MARK_LABEL_MM,
        angle_deg: 0.0,
        anchor,
        text: text.into(),
    }
}

/// Put an outline on a sheet: the cut and surface lines (the starboard half only when `half`),
/// and the largest distance of the cut line from the centreline.
pub(crate) fn draw_outline(sheet: &mut Sheet, outline: &Outline, half: bool) -> f64 {
    let mut reach = 0.0f64;
    for (kind, lines) in [
        (PathKind::Reference, &outline.surface),
        (PathKind::Cut, &outline.cut),
    ] {
        for (points, closed) in lines {
            let runs = if half {
                starboard_runs(points, *closed)
            } else {
                vec![(points.clone(), *closed)]
            };
            for (points, closed) in runs {
                if kind == PathKind::Cut {
                    reach = points.iter().fold(reach, |m, p| m.max(p.0.abs()));
                }
                sheet.paths.push(SheetPath {
                    kind,
                    points,
                    closed,
                });
            }
        }
    }
    reach
}

/// The mould for the station at `x`: the section there, moved in by the skin, with the
/// centreline, the baseline, the waterlines asked for and the sheer marked, and notes saying
/// what was deducted and which face goes on the station line. `index` counts from 1, of `count`.
///
/// `None` when the plane cuts nothing.
pub fn station_template(
    hull: &Hull,
    x: f64,
    index: usize,
    count: usize,
    opts: &TemplateOptions,
) -> Option<Template> {
    let section = cut(hull, &Frame::station(x));
    if section.is_empty() {
        return None;
    }
    let skin = if opts.skin_m.is_finite() {
        opts.skin_m.max(0.0)
    } else {
        0.0
    };
    let top = section
        .iter()
        .flat_map(|p| p.points.iter())
        .fold(f64::MIN, |m, p| m.max(p.1));
    let mut warnings = Vec::new();
    // Legs only go up: a level under this section's top is left out and said.
    let legs_to = match opts.strongback_z {
        Some(z) if z.is_finite() && z >= top - 1e-9 => Some(z),
        Some(_) => {
            warnings.push(
                "the strongback level is below the top of this section: no legs drawn".into(),
            );
            None
        }
        None => None,
    };
    let outline = outline(&section, skin, legs_to, "station");
    let [u0, v0, u1, v1] = outline.bounds;
    let name = format!("Station {index}");
    let mut sheet = Sheet::new(name.clone());
    let half_breadth_m = draw_outline(&mut sheet, &outline, opts.half);

    // Marks reach a little beyond the section on each side.
    let left = if opts.half {
        -MARK_REACH_M
    } else {
        u0.min(0.0) - MARK_REACH_M
    };
    let right = u1.max(0.0) + MARK_REACH_M;
    let highest = legs_to.unwrap_or(v1).max(v1);
    sheet.paths.push(mark(
        (0.0, v0 - MARK_REACH_M),
        (0.0, highest + MARK_REACH_M),
    ));
    sheet.labels.push(mark_label(
        (0.002, highest + MARK_REACH_M),
        Anchor::Start,
        "CL",
    ));
    let level = |sheet: &mut Sheet, v: f64, text: String| {
        sheet.paths.push(mark((left, v), (right, v)));
        sheet
            .labels
            .push(mark_label((right, v + 0.001), Anchor::End, text));
    };
    level(&mut sheet, hull.keel, "BL".into());
    let within = |v: f64| v.is_finite() && v > v0 - 1e-9 && v < v1 + 1e-9;
    for height in &opts.waterlines_m {
        if within(hull.keel + height) && *height > 0.0 {
            let text = format!("WL {}", units::length(*height, opts.imperial));
            level(&mut sheet, hull.keel + height, text);
        }
    }
    if let Some(height) = opts.dwl_m.filter(|h| *h > 0.0 && within(hull.keel + h)) {
        let text = format!("DWL {}", units::length(height, opts.imperial));
        level(&mut sheet, hull.keel + height, text);
    }
    if let Some(z) = legs_to {
        if outline.sheer.is_empty() {
            level(&mut sheet, z, "Strongback".into());
        }
    }
    // The sheer, where an open section ends: a tick outward from each end.
    for (i, end) in outline.sheer.iter().enumerate() {
        if opts.half && end.0 < 0.0 {
            continue;
        }
        let out = if end.0 < 0.0 {
            -MARK_REACH_M
        } else {
            MARK_REACH_M
        };
        sheet.paths.push(mark(*end, (end.0 + out, end.1)));
        if i == 0 || opts.half {
            let anchor = if out < 0.0 {
                Anchor::End
            } else {
                Anchor::Start
            };
            sheet
                .labels
                .push(mark_label((end.0 + out, end.1 + 0.001), anchor, "Sheer"));
        }
    }
    let middle = if opts.half {
        half_breadth_m / 2.0
    } else {
        (u0 + u1) / 2.0
    };
    sheet.labels.push(Label {
        at: (middle, (v0 + v1) / 2.0),
        height_mm: TITLE_LABEL_MM,
        angle_deg: 0.0,
        anchor: Anchor::Middle,
        text: name,
    });

    let face = if outline.mean_along > 0.01 {
        MouldFace::Forward
    } else if outline.mean_along < -0.01 {
        MouldFace::Aft
    } else {
        MouldFace::Either
    };
    sheet.notes.push(format!(
        "Station {index} of {count}, x = {}",
        units::length(x, opts.imperial)
    ));
    sheet.notes.push(skin_note(skin));
    sheet.notes.push(
        match face {
            MouldFace::Forward => "Station line on the FORWARD face of the mould",
            MouldFace::Aft => "Station line on the AFT face of the mould",
            MouldFace::Either => "Either face on the station line",
        }
        .into(),
    );
    sheet.notes.extend(measure_notes(&outline));
    if opts.half {
        sheet.notes.push(
            "Half mould: the starboard side. Flip the pattern about the centreline (CL) for the other side".into(),
        );
    }
    if let (Some(z), false) = (legs_to, outline.sheer.is_empty()) {
        sheet.notes.push(format!(
            "Legs run up to the strongback level, {} above the baseline",
            units::length(z - hull.keel, opts.imperial)
        ));
    }
    warnings.extend(outline.warnings.iter().cloned());
    sheet.notes.extend(warnings.iter().cloned());
    Some(Template {
        sheet,
        x: Some(x),
        face: Some(face),
        max_bevel_deg: outline.max_bevel_deg,
        half_breadth_m,
        longest_segment_m: outline.longest_segment_m,
        warnings,
    })
}

/// The note that says what the cut line is.
pub(crate) fn skin_note(skin_m: f64) -> String {
    if skin_m > 0.0 {
        format!(
            "Mould line {} inside the hull surface",
            units::mm_tenths(skin_m)
        )
    } else {
        "Mould line is the hull surface: no skin deducted".into()
    }
}

/// The notes that say how the surface meets the plane and how coarse the section is.
pub(crate) fn measure_notes(outline: &Outline) -> [String; 2] {
    [
        format!("Largest bevel {:.1}°", outline.max_bevel_deg),
        format!(
            "Longest straight segment {} mm",
            units::millimetres(outline.longest_segment_m)
        ),
    ]
}

/// Where the stations of a set are.
#[derive(Clone, Debug, PartialEq)]
pub enum StationChoice {
    /// This many, at the middles of equal slices of the hull's length: the stations of the
    /// offsets table.
    Count(usize),
    /// At every multiple of this many metres of model `x` inside the hull.
    Spacing(f64),
    /// At these model `x` positions, as given.
    At(Vec<f64>),
}

/// The `x` of each station. Fails for a count under 2 or over [`MAX_STATIONS`], a spacing that
/// is not more than zero or gives no station or too many, and an empty or non-finite list.
pub fn station_positions(hull: &Hull, choice: &StationChoice) -> Result<Vec<f64>, String> {
    let too_many =
        |n: usize| format!("{n} stations is more than the {MAX_STATIONS} a set may have");
    match choice {
        StationChoice::Count(n) if *n < 2 => Err("there must be at least 2 stations".into()),
        StationChoice::Count(n) if *n > MAX_STATIONS => Err(too_many(*n)),
        StationChoice::Count(n) => Ok(crate::curves::station_xs(hull, *n)),
        StationChoice::Spacing(s) => {
            if !s.is_finite() || *s <= 0.0 {
                return Err("the station spacing must be more than zero metres".into());
            }
            let (x0, x1) = (hull.model_min.x, hull.model_max.x);
            let tol = 1e-9 * (x1 - x0).abs().max(1.0);
            let (first, last) = (((x0 + tol) / s).ceil(), ((x1 - tol) / s).floor());
            let count = last - first + 1.0;
            if !count.is_finite() || count < 1.0 {
                return Err(format!(
                    "no multiple of {s} m lies inside the hull, which runs from x = {x0:.3} to {x1:.3} m"
                ));
            }
            if count > MAX_STATIONS as f64 {
                return Err(too_many(count.min(1e15) as usize));
            }
            let xs: Vec<f64> = (0..count as usize)
                .map(|i| (first + i as f64) * s)
                .collect();
            Ok(xs
                .into_iter()
                .filter(|x| *x > x0 + tol && *x < x1 - tol)
                .collect())
        }
        StationChoice::At(list) if list.is_empty() => Err("no station positions were given".into()),
        StationChoice::At(list) if list.iter().any(|x| !x.is_finite()) => {
            Err("a station position is not a number".into())
        }
        StationChoice::At(list) if list.len() > MAX_STATIONS => Err(too_many(list.len())),
        StationChoice::At(list) => Ok(list.clone()),
    }
}

#[cfg(test)]
pub(crate) mod testing {
    use super::*;

    /// A hull from loose triangles, each turned so that it faces away from `centre_of(its
    /// centroid)`; triangles with no area are dropped.
    fn hull_from(
        verts: Vec<Vec3>,
        tris: Vec<[usize; 3]>,
        centre_of: impl Fn(Vec3) -> Vec3,
    ) -> Hull {
        let mut out = Vec::new();
        for t in tris {
            let (a, b, c) = (verts[t[0]], verts[t[1]], verts[t[2]]);
            let n = (b - a).cross(c - a);
            if n.length() < 1e-14 {
                continue;
            }
            let centroid = (a + b + c) / 3.0;
            out.push(if n.dot(centroid - centre_of(centroid)) < 0.0 {
                [t[0], t[2], t[1]]
            } else {
                t
            });
        }
        let lo = verts
            .iter()
            .fold(Vec3::new(f64::MAX, f64::MAX, f64::MAX), |m, v| {
                Vec3::new(m.x.min(v.x), m.y.min(v.y), m.z.min(v.z))
            });
        let hi = verts
            .iter()
            .fold(Vec3::new(f64::MIN, f64::MIN, f64::MIN), |m, v| {
                Vec3::new(m.x.max(v.x), m.y.max(v.y), m.z.max(v.z))
            });
        Hull::from_panels(verts, out, lo.z, hi.x - lo.x, lo, hi)
    }

    /// A frustum along `x` from 0 to `length`, radius `r_aft` to `r_fwd`, open at both ends, its
    /// axis on y = z = 0, of `facets` flat strips.
    pub(crate) fn tube(r_aft: f64, r_fwd: f64, length: f64, facets: usize) -> Hull {
        let mut verts = Vec::new();
        for i in 0..facets {
            let a = std::f64::consts::TAU * i as f64 / facets as f64;
            verts.push(Vec3::new(0.0, r_aft * a.cos(), r_aft * a.sin()));
            verts.push(Vec3::new(length, r_fwd * a.cos(), r_fwd * a.sin()));
        }
        let mut tris = Vec::new();
        for i in 0..facets {
            let j = (i + 1) % facets;
            tris.push([2 * i, 2 * j, 2 * i + 1]);
            tris.push([2 * j, 2 * j + 1, 2 * i + 1]);
        }
        hull_from(verts, tris, |c| Vec3::new(c.x, 0.0, 0.0))
    }

    /// A box, `x` in `0..l`, `y` in `-b/2..b/2`, `z` in `0..d`, with or without its top.
    pub(crate) fn box_hull(l: f64, b: f64, d: f64, deck: bool) -> Hull {
        let h = b / 2.0;
        let verts: Vec<Vec3> = [
            (0.0, -h, 0.0),
            (l, -h, 0.0),
            (l, h, 0.0),
            (0.0, h, 0.0),
            (0.0, -h, d),
            (l, -h, d),
            (l, h, d),
            (0.0, h, d),
        ]
        .iter()
        .map(|(x, y, z)| Vec3::new(*x, *y, *z))
        .collect();
        let mut quads = vec![
            [0, 1, 2, 3],
            [0, 1, 5, 4],
            [3, 2, 6, 7],
            [0, 3, 7, 4],
            [1, 2, 6, 5],
        ];
        if deck {
            quads.push([4, 5, 6, 7]);
        }
        let tris = quads
            .iter()
            .flat_map(|q| [[q[0], q[1], q[2]], [q[0], q[2], q[3]]])
            .collect();
        hull_from(verts, tris, move |_| Vec3::new(l / 2.0, 0.0, d / 2.0))
    }
}

#[cfg(test)]
mod tests {
    use super::testing::*;
    use super::*;

    fn opts(skin: f64) -> TemplateOptions {
        TemplateOptions {
            skin_m: skin,
            ..Default::default()
        }
    }

    fn points_of(t: &Template, kind: PathKind) -> Vec<(f64, f64)> {
        t.sheet
            .paths
            .iter()
            .filter(|p| p.kind == kind)
            .flat_map(|p| p.points.iter().copied())
            .collect()
    }

    fn cut_points(t: &Template) -> Vec<(f64, f64)> {
        points_of(t, PathKind::Cut)
    }

    /// `[min_u, min_v, max_u, max_v]` of some points.
    fn span(points: &[(f64, f64)]) -> [f64; 4] {
        points
            .iter()
            .fold([f64::MAX, f64::MAX, f64::MIN, f64::MIN], |b, (u, v)| {
                [b[0].min(*u), b[1].min(*v), b[2].max(*u), b[3].max(*v)]
            })
    }

    fn assert_span(points: &[(f64, f64)], want: [f64; 4], tol: f64) {
        let got = span(points);
        for i in 0..4 {
            assert!((got[i] - want[i]).abs() <= tol, "{got:?} is not {want:?}");
        }
    }

    fn has_point(points: &[(f64, f64)], want: (f64, f64)) -> bool {
        points
            .iter()
            .any(|p| (p.0 - want.0).abs() < 1e-9 && (p.1 - want.1).abs() < 1e-9)
    }

    fn labels(t: &Template) -> Vec<&str> {
        t.sheet.labels.iter().map(|l| l.text.as_str()).collect()
    }

    /// Closed form: the inside of a skin t on a cylinder of radius R is a cylinder of radius R - t.
    /// The 720 flat strips are within R (1 - cos(pi / 720)) = 1e-5 of the cylinder.
    #[test]
    fn a_cylinder_mould_is_smaller_by_the_skin() {
        let t = station_template(&tube(1.0, 1.0, 4.0, 720), 2.0, 1, 1, &opts(0.012)).unwrap();
        let points = cut_points(&t);
        assert!(points.len() >= 720, "{}", points.len());
        for (u, v) in points {
            assert!(((u * u + v * v).sqrt() - 0.988).abs() < 5e-5, "{u} {v}");
        }
        assert_eq!(t.face, Some(MouldFace::Either));
        assert!(t.max_bevel_deg < 0.01);
        assert!((t.half_breadth_m - 0.988).abs() < 5e-5);
        assert!(t.warnings.is_empty(), "{:?}", t.warnings);
        // The hull surface is drawn around it for the eye.
        for (u, v) in points_of(&t, PathKind::Reference) {
            assert!(((u * u + v * v).sqrt() - 1.0).abs() < 2e-5);
        }
    }

    /// Closed form: on a cone of half-angle a = atan(0.5 / 4) = 7.125 deg the radius at the station
    /// shrinks by t / cos a. At x = 2, R = 0.75, so the mould radius is
    /// 0.75 - 0.012 * 1.007782 = 0.737907 and the bevel is a.
    #[test]
    fn a_cone_mould_is_deducted_along_the_slope() {
        let t = station_template(&tube(1.0, 0.5, 4.0, 720), 2.0, 1, 1, &opts(0.012)).unwrap();
        for (u, v) in cut_points(&t) {
            assert!(((u * u + v * v).sqrt() - 0.737907).abs() < 5e-5, "{u} {v}");
        }
        assert!(
            (t.max_bevel_deg - 7.125).abs() < 0.05,
            "{}",
            t.max_bevel_deg
        );
        assert_eq!(t.face, Some(MouldFace::Forward)); // narrowing forward
        assert!(t
            .sheet
            .notes
            .iter()
            .any(|n| n == "Station line on the FORWARD face of the mould"));
        let aft = station_template(&tube(0.5, 1.0, 4.0, 720), 2.0, 1, 1, &opts(0.012)).unwrap();
        assert_eq!(aft.face, Some(MouldFace::Aft));
        assert!(aft
            .sheet
            .notes
            .iter()
            .any(|n| n == "Station line on the AFT face of the mould"));
    }

    /// The deduction goes into the hull whichever way the model's faces are wound: the inside is
    /// read from the section's own shape.
    #[test]
    fn an_inside_out_mesh_is_still_deducted_inward() {
        let mut hull = tube(1.0, 0.5, 4.0, 720);
        for t in &mut hull.tris {
            t.swap(1, 2);
        }
        let t = station_template(&hull, 2.0, 1, 1, &opts(0.012)).unwrap();
        for (u, v) in cut_points(&t) {
            assert!(((u * u + v * v).sqrt() - 0.737907).abs() < 5e-5, "{u} {v}");
        }
        assert_eq!(t.face, Some(MouldFace::Forward));
    }

    /// Closed form: a 10 x 4 x 2 box with a deck and a 10 mm skin has a mould 3.98 x 1.98 with
    /// square corners.
    #[test]
    fn a_box_mould_has_mitred_corners() {
        let t = station_template(&box_hull(10.0, 4.0, 2.0, true), 5.0, 1, 1, &opts(0.01)).unwrap();
        let points = cut_points(&t);
        assert_span(&points, [-1.99, 0.01, 1.99, 1.99], 1e-9);
        for corner in [(-1.99, 0.01), (1.99, 0.01), (1.99, 1.99), (-1.99, 1.99)] {
            assert!(has_point(&points, corner), "{corner:?} in {points:?}");
        }
        let cut: Vec<_> = t
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Cut)
            .collect();
        assert_eq!(cut.len(), 1);
        assert!(cut[0].closed);
        assert_span(
            &points_of(&t, PathKind::Reference),
            [-2.0, 0.0, 2.0, 2.0],
            1e-9,
        );
        assert!((t.half_breadth_m - 1.99).abs() < 1e-9);
        assert!(t.max_bevel_deg < 1e-9);
    }

    #[test]
    fn an_open_section_is_not_deducted_across_the_sheer() {
        let t = station_template(&box_hull(10.0, 4.0, 2.0, false), 5.0, 1, 1, &opts(0.01)).unwrap();
        let points = cut_points(&t);
        assert_span(&points, [-1.99, 0.01, 1.99, 2.0], 1e-9);
        assert!(
            has_point(&points, (-1.99, 2.0)) && has_point(&points, (1.99, 2.0)),
            "{points:?}"
        );
        // The outline is closed straight across the sheer.
        let cut: Vec<_> = t
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Cut)
            .collect();
        assert!(cut.len() == 1 && cut[0].closed);
        assert!(labels(&t).contains(&"Sheer"));
    }

    #[test]
    fn strongback_legs_run_up_from_the_sheer() {
        let o = TemplateOptions {
            strongback_z: Some(2.5),
            ..opts(0.01)
        };
        let t = station_template(&box_hull(10.0, 4.0, 2.0, false), 5.0, 1, 1, &o).unwrap();
        let points = cut_points(&t);
        assert!(
            has_point(&points, (-1.99, 2.5)) && has_point(&points, (1.99, 2.5)),
            "{points:?}"
        );
        assert!(
            has_point(&points, (-1.99, 2.0)) && has_point(&points, (1.99, 2.0)),
            "{points:?}"
        );
        assert!(t
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Cut)
            .all(|p| p.closed));
        assert!(
            t.sheet
                .notes
                .iter()
                .any(|n| n == "Legs run up to the strongback level, 2.500 m above the baseline"),
            "{:?}",
            t.sheet.notes
        );
        // A decked section keeps its own outline and gets the level as a mark.
        let decked = station_template(&box_hull(10.0, 4.0, 2.0, true), 5.0, 1, 1, &o).unwrap();
        assert_span(&cut_points(&decked), [-1.99, 0.01, 1.99, 1.99], 1e-9);
        let marks: Vec<_> = decked
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Mark)
            .collect();
        assert!(marks.iter().any(|p| p.points.iter().all(|q| q.1 == 2.5)));
        assert!(labels(&decked).contains(&"Strongback"));
    }

    #[test]
    fn a_half_mould_stops_at_the_centreline() {
        let o = TemplateOptions {
            half: true,
            ..opts(0.01)
        };
        let t = station_template(&box_hull(10.0, 4.0, 2.0, true), 5.0, 1, 1, &o).unwrap();
        let points = cut_points(&t);
        assert!(points.iter().all(|p| p.0 >= -1e-12), "{points:?}");
        assert_span(&points, [0.0, 0.01, 1.99, 1.99], 1e-9);
        assert!(
            has_point(&points, (0.0, 0.01)) && has_point(&points, (0.0, 1.99)),
            "{points:?}"
        );
        // The half is open along the centreline, which is a mark to flip the pattern about.
        assert!(t
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Cut)
            .all(|p| !p.closed));
        let marks: Vec<_> = t
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Mark)
            .collect();
        assert!(marks.iter().any(|p| p.points.iter().all(|q| q.0 == 0.0)));
        assert!(points_of(&t, PathKind::Reference)
            .iter()
            .all(|p| p.0 >= -1e-12));
        assert!(
            t.sheet.notes.iter().any(|n| n.starts_with("Half mould")),
            "{:?}",
            t.sheet.notes
        );
        assert!((t.half_breadth_m - 1.99).abs() < 1e-9);
    }

    #[test]
    fn no_skin_draws_the_hull_and_says_so() {
        let t = station_template(&box_hull(10.0, 4.0, 2.0, true), 5.0, 1, 1, &opts(0.0)).unwrap();
        assert_span(&cut_points(&t), [-2.0, 0.0, 2.0, 2.0], 1e-9);
        assert!(t
            .sheet
            .notes
            .iter()
            .any(|n| n == "Mould line is the hull surface: no skin deducted"));
        assert!(points_of(&t, PathKind::Reference).is_empty());
    }

    #[test]
    fn the_notes_are_the_spec_s() {
        let o = TemplateOptions {
            imperial: true,
            ..opts(0.006)
        };
        let t = station_template(&box_hull(10.0, 4.0, 2.0, true), 2.4, 3, 11, &o).unwrap();
        let notes = &t.sheet.notes;
        assert_eq!(t.sheet.name, "Station 3");
        assert_eq!(notes[0], "Station 3 of 11, x = 2.400 m (7' 10 1/2\")");
        assert_eq!(notes[1], "Mould line 6.0 mm inside the hull surface");
        assert_eq!(notes[2], "Either face on the station line");
        assert_eq!(notes[3], "Largest bevel 0.0°");
        // The bottom is two triangles; their shared edge crosses x = 2.4 at y = -1.04, so the
        // longest run is from there to the far side, 3.04 m.
        assert_eq!(notes[4], "Longest straight segment 3040 mm");
        assert_eq!(notes.len(), 5);
        assert_eq!(t.x, Some(2.4));
        assert!(labels(&t).contains(&"Station 3"));
        // Metric alone without the option.
        let metric =
            station_template(&box_hull(10.0, 4.0, 2.0, true), 2.4, 3, 11, &opts(0.006)).unwrap();
        assert_eq!(metric.sheet.notes[0], "Station 3 of 11, x = 2.400 m");
        assert_eq!(metric.sheet.notes[1..], notes[1..]);
        assert_eq!(metric.sheet.paths, t.sheet.paths);
    }

    #[test]
    fn waterlines_and_the_baseline_are_marked() {
        let o = TemplateOptions {
            waterlines_m: vec![0.5, 1.0, 7.0],
            dwl_m: Some(0.8),
            ..opts(0.01)
        };
        let t = station_template(&box_hull(10.0, 4.0, 2.0, true), 5.0, 1, 1, &o).unwrap();
        let marks: Vec<_> = t
            .sheet
            .paths
            .iter()
            .filter(|p| p.kind == PathKind::Mark)
            .collect();
        for v in [0.0, 0.5, 0.8, 1.0] {
            let line = marks
                .iter()
                .find(|p| p.points.iter().all(|q| q.1 == v))
                .unwrap_or_else(|| panic!("a mark at {v}"));
            // Marks reach 20 mm beyond the hull on each side.
            assert_span(&line.points, [-2.02, v, 2.02, v], 1e-9);
        }
        // A waterline above the section is not drawn.
        assert!(!marks.iter().any(|p| p.points.iter().all(|q| q.1 == 7.0)));
        assert!(
            marks.iter().any(|p| p.points.iter().all(|q| q.0 == 0.0)),
            "a centreline"
        );
        for label in ["BL", "WL 0.500 m", "DWL 0.800 m", "WL 1.000 m", "CL"] {
            assert!(labels(&t).contains(&label), "{label} in {:?}", labels(&t));
        }
        let imperial = TemplateOptions {
            imperial: true,
            ..o
        };
        let t = station_template(&box_hull(10.0, 4.0, 2.0, true), 5.0, 1, 1, &imperial).unwrap();
        assert!(
            labels(&t).contains(&"WL 0.500 m (1' 7 11/16\")"),
            "{:?}",
            labels(&t)
        );
    }

    /// A section with a segment 0.15 mm long beside a 3 degree bend, as a plane clipping the tip of
    /// a triangle makes: moved in 6 mm the short segment would turn over by under 0.01 mm. It closes up
    /// and nothing is reported. The same bend with a 60 mm skin turns it over by 1.4 mm: reported.
    #[test]
    fn a_hair_of_a_segment_beside_a_bend_closes_up_quietly() {
        let bend = 3f64.to_radians();
        let (c, s) = (bend.cos(), bend.sin());
        // Along -u, the outside below: a long run, the short segment, then the bend and a long run.
        let points = vec![
            (0.0, 0.0),
            (-1.0, 0.0),
            (-1.00015, 0.0),
            (-1.00015 - c, s),
            (0.0, 2.0),
        ];
        let outward = |i: usize| {
            let (p, q): ((f64, f64), (f64, f64)) = (points[i], points[(i + 1) % 5]);
            let len = (q.0 - p.0).hypot(q.1 - p.1);
            ((q.1 - p.1) / -len, (q.0 - p.0) / len)
        };
        let meetings = (0..5)
            .map(|i| {
                Some(Meeting {
                    outward: outward(i),
                    in_plane: 1.0,
                    along: 0.0,
                })
            })
            .collect();
        let section = [SectionPath {
            points: points.clone(),
            closed: true,
            meetings,
        }];
        assert_eq!(outward(0), (0.0, -1.0));
        let thin = mould_line(&section, 0.006);
        assert!(!thin.crosses_itself);
        assert_eq!(
            thin.paths[0].0[1], thin.paths[0].0[2],
            "the short segment closed up"
        );
        assert!((thin.paths[0].0[1].1 - 0.006).abs() < 1e-4);
        assert!(mould_line(&section, 0.06).crosses_itself);
    }

    /// A 5 mm tube cannot hold a 12 mm skin: the mould line would turn inside out.
    #[test]
    fn a_skin_thicker_than_the_radius_is_warned_about() {
        let t = station_template(&tube(0.005, 0.005, 1.0, 64), 0.5, 1, 1, &opts(0.012)).unwrap();
        let warning =
            "skin is thicker than the hull's local radius here: the mould line crosses itself";
        assert_eq!(t.warnings, [warning]);
        assert!(
            t.sheet.notes.iter().any(|n| n == warning),
            "{:?}",
            t.sheet.notes
        );
        // A hull with room for its skin is not warned about.
        assert!(
            station_template(&tube(0.05, 0.05, 1.0, 64), 0.5, 1, 1, &opts(0.012))
                .unwrap()
                .warnings
                .is_empty()
        );
    }

    /// A cone 1 m in radius and 0.1 m long meets the station plane at 5.7 degrees: its in-plane
    /// normal is 0.0995 long, under the 0.2 floor, so the deduction is 0.01 / 0.2 = 0.05.
    #[test]
    fn a_surface_nearly_in_the_plane_is_clamped_and_counted() {
        let hull = tube(1.0, 0.0, 0.1, 64);
        let t = station_template(&hull, 0.05, 1, 1, &opts(0.01)).unwrap();
        assert_eq!(
            t.warnings,
            ["64 segments are too oblique to the station plane for a reliable deduction"]
        );
        for (u, v) in cut_points(&t) {
            let r = (u * u + v * v).sqrt();
            assert!(r > 0.5 - 0.05 - 1e-3 && r < 0.5 + 1e-9, "{r}");
        }
    }

    #[test]
    fn a_plane_that_misses_the_hull_gives_nothing() {
        let closed = box_hull(10.0, 4.0, 2.0, true);
        assert!(station_template(&closed, 99.0, 1, 1, &opts(0.01)).is_none());
        assert!(station_template(&closed, -0.5, 1, 1, &opts(0.01)).is_none());
        // The plane lies in the end face: whatever comes out, it is not a panic, and it is not a
        // mould larger than the hull.
        for x in [0.0, 10.0] {
            if let Some(t) = station_template(&closed, x, 1, 1, &opts(0.01)) {
                let s = span(&cut_points(&t));
                assert!(
                    s[0] >= -2.0 - 1e-9
                        && s[2] <= 2.0 + 1e-9
                        && s[1] >= -1e-9
                        && s[3] <= 2.0 + 1e-9,
                    "{s:?}"
                );
            }
        }
        for x in [f64::NAN, f64::INFINITY] {
            assert!(station_template(&closed, x, 1, 1, &opts(0.01)).is_none());
        }
    }

    #[test]
    fn station_positions_follow_the_choice() {
        let hull = tube(1.0, 1.0, 4.0, 16);
        assert_eq!(
            station_positions(&hull, &StationChoice::Count(11)),
            Ok(crate::curves::station_xs(&hull, 11))
        );
        assert_eq!(
            station_positions(&hull, &StationChoice::Spacing(0.5)),
            Ok(vec![0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5])
        );
        assert_eq!(
            station_positions(&hull, &StationChoice::Spacing(3.0)),
            Ok(vec![3.0])
        );
        assert_eq!(
            station_positions(&hull, &StationChoice::At(vec![3.0, 1.0, 99.0])),
            Ok(vec![3.0, 1.0, 99.0])
        );
        for bad in [
            StationChoice::Spacing(0.0),
            StationChoice::Spacing(-1.0),
            StationChoice::Spacing(f64::NAN),
            StationChoice::Spacing(5.0),
            StationChoice::Spacing(1e-6),
            StationChoice::Count(0),
            StationChoice::Count(1),
            StationChoice::Count(100_000),
            StationChoice::At(vec![]),
            StationChoice::At(vec![1.0, f64::NAN]),
        ] {
            assert!(station_positions(&hull, &bad).is_err(), "{bad:?}");
        }
    }

    /// Internal consistency, not an oracle: the mould is never wider than the offsets table's
    /// section, and is narrower by no more than the skin over the 0.2 floor.
    #[test]
    fn the_demo_hull_s_moulds_agree_with_its_offsets_table() {
        let hull = Hull::new(
            &crate::demo::demo_hull(true),
            crate::SubdivisionOptions::default(),
        );
        let table = crate::offsets::table_of_offsets(&hull, 11, &[], &[]);
        let xs = station_positions(&hull, &StationChoice::Count(11)).unwrap();
        assert_eq!(xs.len(), table.stations.len());
        for (i, (x, row)) in xs.iter().zip(&table.stations).enumerate() {
            let t = station_template(&hull, *x, i + 1, 11, &opts(0.006)).unwrap();
            assert!(
                t.half_breadth_m <= row.half_beam + 1e-9,
                "station {}: {} > {}",
                i + 1,
                t.half_breadth_m,
                row.half_beam
            );
            assert!(
                t.half_breadth_m >= row.half_beam - 5.0 * 0.006,
                "station {}: {} << {}",
                i + 1,
                t.half_breadth_m,
                row.half_beam
            );
            assert!(t.warnings.is_empty(), "station {}: {:?}", i + 1, t.warnings);
            assert!(t.max_bevel_deg < 60.0);
        }
    }
}
