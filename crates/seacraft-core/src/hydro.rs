//! Hydrostatics and stability.
//!
//! The measurement pipeline follows FREE!ship's `TFreeHydrostaticCalc`:
//!
//! 1. refine the control net to the requested precision,
//! 2. mirror symmetric layers so the full hull is measured,
//! 3. clip every triangle against the waterplane and integrate the submerged
//!    volume / wetted surface with the divergence theorem,
//! 4. find the waterline by intersecting the mesh with the waterplane, chain
//!    the resulting segments into closed loops and integrate area, centre of
//!    flotation and second moments along them,
//! 5. derive the usual coefficients, metacentric radii and section areas.
//!
//! Sign convention: `x` forward, `y` starboard, `z` up. Heel is positive to
//! starboard, `trim` is a length in metres and is positive when the bow sits
//! higher than the stern.

use crate::math::{deg_to_rad, PolygonMoments, Vec3};
use crate::math::{polygon_area3, Plane};
use crate::mesh::Surface;
use crate::subdiv::{evaluate, SubdivisionOptions};
use serde::Serialize;
use std::collections::HashMap;

const CLIP_EPS: f64 = 1e-7;

#[derive(Debug, Clone)]
pub struct HydroOptions {
    pub draft: f64,
    pub trim: f64,
    pub heel_deg: f64,
    pub water_density: f64,
    pub appendage_coefficient: f64,
    /// Station at which the midship section is measured. `None` uses the
    /// longitudinal centre of flotation.
    pub mainframe_x: Option<f64>,
    pub stations: usize,
    pub subdivision: SubdivisionOptions,
}

impl HydroOptions {
    pub fn from_surface(s: &Surface, subdiv: SubdivisionOptions) -> Self {
        Self {
            draft: s.settings.draft,
            trim: 0.0,
            heel_deg: 0.0,
            water_density: s.settings.water_density,
            appendage_coefficient: s.settings.appendage_coefficient,
            mainframe_x: if s.settings.mainframe_location != 0.0 {
                Some(s.settings.mainframe_location)
            } else {
                None
            },
            stations: 21,
            subdivision: subdiv,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct SectionArea {
    pub x: f64,
    pub area: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Warning {
    pub code: &'static str,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Default)]
pub struct Hydrostatics {
    // input state
    pub draft: f64,
    pub trim_m: f64,
    pub heel_deg: f64,
    pub water_density: f64,
    pub appendage_coefficient: f64,

    // volume
    pub volume_m3: f64,
    pub displacement_kg: f64,
    pub displacement_t: f64,
    pub lcb: Vec3,
    pub kb: f64,

    // areas
    pub wetted_surface_m2: f64,
    pub waterplane_area_m2: f64,
    pub waterplane_inertia_transverse: f64,
    pub waterplane_inertia_longitudinal: f64,
    pub lcf: Vec3,
    pub lwl: f64,
    pub bwl: f64,
    pub submerged_length: f64,
    pub submerged_beam: f64,
    pub absolute_draft: f64,

    // metacentres
    pub km_transverse: f64,
    pub km_longitudinal: f64,
    pub bm_transverse: f64,
    pub bm_longitudinal: f64,

    // coefficients
    pub block_coefficient: f64,
    pub prismatic_coefficient: f64,
    pub midship_coefficient: f64,
    pub waterplane_coefficient: f64,
    pub vertical_prismatic_coefficient: f64,
    pub mainframe_area_m2: f64,
    pub mainframe_x: f64,

    pub sac: Vec<SectionArea>,
    pub lateral_area_m2: f64,
    pub lateral_centroid: Vec3,

    pub model_min: Vec3,
    pub model_max: Vec3,
    pub warnings: Vec<Warning>,
}

impl Hydrostatics {
    pub fn gm(&self, kg: f64) -> f64 {
        self.km_transverse - kg
    }
}

/// The full hull as triangles, restricted to hydrostatic layers and mirrored
/// where the layer is symmetric.
fn hull_triangles(surface: &Surface, opts: &HydroOptions) -> (Vec<Vec3>, Vec<[usize; 3]>) {
    let mesh = evaluate(surface, opts.subdivision);
    let mut verts = mesh.verts.clone();
    let mut tris: Vec<[usize; 3]> = Vec::new();
    let mut mirrored_verts: HashMap<usize, usize> = HashMap::new();

    for (fi, f) in mesh.faces.iter().enumerate() {
        let layer = &surface.layers[mesh.face_layer[fi]];
        if !layer.use_in_hydrostatics {
            continue;
        }
        for i in 1..f.len().saturating_sub(1) {
            tris.push([f[0], f[i], f[i + 1]]);
        }
        if layer.symmetric {
            let mut map = |p: usize, verts: &mut Vec<Vec3>| -> usize {
                *mirrored_verts.entry(p).or_insert_with(|| {
                    let mut q = verts[p];
                    q.y = -q.y;
                    verts.push(q);
                    verts.len() - 1
                })
            };
            for i in 1..f.len().saturating_sub(1) {
                let a = map(f[0], &mut verts);
                let b = map(f[i + 1], &mut verts);
                let c = map(f[i], &mut verts);
                tris.push([a, b, c]);
            }
        }
    }
    (verts, tris)
}

/// Waterplane for a draft / trim / heel combination.
pub fn waterplane(surface: &Surface, draft: f64, trim: f64, heel_deg: f64) -> Plane {
    let (min, max) = surface.extents();
    let length = if surface.settings.length > 0.0 {
        surface.settings.length
    } else {
        max.x - min.x
    };
    waterplane_for(
        surface.lowest_hydrostatics_point(),
        length,
        draft,
        trim,
        heel_deg,
    )
}

pub fn waterplane_for(keel_z: f64, length: f64, draft: f64, trim: f64, heel_deg: f64) -> Plane {
    let length = length.max(1e-6);
    let heel = deg_to_rad(heel_deg);
    // `trim` is positive when the bow is higher than the stern, so the stern
    // (x = 0) is the deeper end in ship coordinates.
    let z1 = keel_z + draft + trim * 0.5;
    let z2 = keel_z + draft - trim * 0.5;
    let p1 = Vec3::new(0.0, 0.0, z1);
    let p2 = Vec3::new(length, 0.0, z2);
    // Heel is positive to starboard. In ship-fixed coordinates that raises the
    // water surface towards +y (the ship turns, not the sea), so the starboard
    // side goes deeper. The slope is `tan`, not `sin`: the surface must make
    // exactly `heel_deg` with the horizontal, or the angle reported is not the
    // angle floated and GZ is measured at the wrong heel.
    let p3 = Vec3::new(0.0, 1.0, z1 + heel.tan());
    Plane::through(p1, p2, p3)
}

/// Clip a triangle to the half space `side <= 0`.
fn clip_below(tri: [Vec3; 3], plane: &Plane) -> Vec<Vec3> {
    let s = [plane.side(tri[0]), plane.side(tri[1]), plane.side(tri[2])];
    let mut out = Vec::with_capacity(4);
    for i in 0..3 {
        let a = tri[i];
        let b = tri[(i + 1) % 3];
        let sa = s[i];
        let sb = s[(i + 1) % 3];
        if sa <= CLIP_EPS {
            out.push(a);
        }
        if (sa < -CLIP_EPS && sb > CLIP_EPS) || (sa > CLIP_EPS && sb < -CLIP_EPS) {
            let t = sa / (sa - sb);
            out.push(a.lerp(b, t));
        }
    }
    out
}

/// Integrate volume, moments and wetted surface of the submerged hull.
fn integrate(
    verts: &[Vec3],
    tris: &[[usize; 3]],
    plane: &Plane,
) -> (f64, Vec3, f64, Vec3, Vec3) {
    let origin = plane.project(Vec3::ZERO);
    let mut volume = 0.0f64;
    let mut moment = Vec3::ZERO;
    let mut wetted = 0.0f64;
    let mut smin = Vec3::new(f64::INFINITY, f64::INFINITY, f64::INFINITY);
    let mut smax = Vec3::new(f64::NEG_INFINITY, f64::NEG_INFINITY, f64::NEG_INFINITY);
    let mut any_submerged = false;

    for t in tris {
        let tri = [verts[t[0]], verts[t[1]], verts[t[2]]];
        let inside = tri.iter().filter(|p| plane.side(**p) <= CLIP_EPS).count();
        if inside == 0 {
            continue;
        }
        let poly = clip_below(tri, plane);
        if poly.len() < 3 {
            continue;
        }
        any_submerged = true;
        for p in &poly {
            smin = smin.min(*p);
            smax = smax.max(*p);
        }
        wetted += polygon_area3(&poly);
        let p0 = poly[0] - origin;
        for i in 1..poly.len() - 1 {
            let p1 = poly[i] - origin;
            let p2 = poly[i + 1] - origin;
            let v = p0.dot(p1.cross(p2)) / 6.0;
            if v != 0.0 {
                volume += v;
                moment += (p0 + p1 + p2) * (0.25 * v);
            }
        }
    }
    if volume < 0.0 {
        volume = -volume;
        moment = -moment;
    }
    if !any_submerged {
        smin = Vec3::ZERO;
        smax = Vec3::ZERO;
    }
    (volume, moment, wetted, smin, smax)
}

/// Segments where the mesh crosses a plane.
pub fn plane_segments(verts: &[Vec3], tris: &[[usize; 3]], plane: &Plane) -> Vec<(Vec3, Vec3)> {
    let mut segs = Vec::new();
    for t in tris {
        let tri = [verts[t[0]], verts[t[1]], verts[t[2]]];
        let s = [plane.side(tri[0]), plane.side(tri[1]), plane.side(tri[2])];
        if s.iter().all(|v| *v > CLIP_EPS) || s.iter().all(|v| *v < -CLIP_EPS) {
            continue;
        }
        let mut pts: Vec<Vec3> = Vec::with_capacity(3);
        for i in 0..3 {
            if s[i].abs() <= CLIP_EPS {
                pts.push(tri[i]);
            }
        }
        for i in 0..3 {
            let j = (i + 1) % 3;
            if (s[i] < -CLIP_EPS && s[j] > CLIP_EPS) || (s[i] > CLIP_EPS && s[j] < -CLIP_EPS) {
                let t = s[i] / (s[i] - s[j]);
                pts.push(tri[i].lerp(tri[j], t));
            }
        }
        // Deduplicate.
        let mut uniq: Vec<Vec3> = Vec::new();
        for p in pts {
            if !uniq.iter().any(|q| q.dist(p) < 1e-9) {
                uniq.push(p);
            }
        }
        if uniq.len() == 2 {
            segs.push((uniq[0], uniq[1]));
        } else if uniq.len() > 2 {
            // Pick the two extreme points; a triangle in the plane yields a
            // degenerate segment.
            let mut best = (0usize, 1usize, 0.0);
            for i in 0..uniq.len() {
                for j in i + 1..uniq.len() {
                    let d = uniq[i].dist(uniq[j]);
                    if d > best.2 {
                        best = (i, j, d);
                    }
                }
            }
            segs.push((uniq[best.0], uniq[best.1]));
        }
    }
    segs
}

/// Chain segments into ordered polylines. The returned flag says whether the
/// polyline closed on itself. Endpoints are snapped to `tol` so duplicate
/// vertices from mirrored halves join up.
pub fn chain_paths(segs: &[(Vec3, Vec3)], tol: f64) -> Vec<(Vec<Vec3>, bool)> {
    if segs.is_empty() {
        return Vec::new();
    }
    // Snap endpoints by proximity, not by grid rounding: the two triangles
    // sharing an edge compute the crossing point from opposite directions, so
    // the results differ in the last bits and a rounded grid key would split
    // the polyline at every row boundary.
    let cell = tol.max(1e-12);
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut points: Vec<Vec3> = Vec::new();
    let id = |p: Vec3, points: &mut Vec<Vec3>, grid: &mut HashMap<(i64, i64, i64), Vec<usize>>| -> usize {
        let key = |v: Vec3| {
            (
                (v.x / cell).floor() as i64,
                (v.y / cell).floor() as i64,
                (v.z / cell).floor() as i64,
            )
        };
        let (kx, ky, kz) = key(p);
        for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(ids) = grid.get(&(kx + dx, ky + dy, kz + dz)) {
                        for &i in ids {
                            if points[i].dist(p) <= tol {
                                return i;
                            }
                        }
                    }
                }
            }
        }
        let i = points.len();
        points.push(p);
        grid.entry((kx, ky, kz)).or_default().push(i);
        i
    };
    // A mesh edge that is a quad diagonal belongs to four triangles, so the
    // plane reports the same crossing twice. Deduplicate or the walk stops at
    // the resulting degree-4 vertices.
    let mut edges: Vec<(usize, usize)> = Vec::with_capacity(segs.len());
    let mut seen: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for (a, b) in segs {
        let ia = id(*a, &mut points, &mut grid);
        let ib = id(*b, &mut points, &mut grid);
        if ia != ib {
            let key = if ia < ib { (ia, ib) } else { (ib, ia) };
            if seen.insert(key) {
                edges.push((ia, ib));
            }
        }
    }

    let mut adj: Vec<Vec<usize>> = vec![Vec::new(); points.len()];
    for (i, (a, b)) in edges.iter().enumerate() {
        adj[*a].push(i);
        adj[*b].push(i);
    }
    let mut used = vec![false; edges.len()];
    let mut paths: Vec<(Vec<Vec3>, bool)> = Vec::new();

    // Walk away from the start edge in both directions so a path that is
    // entered in its middle still comes out whole.
    let walk = |mut cur: usize,
                start_vertex: usize,
                used: &mut Vec<bool>,
                adj: &Vec<Vec<usize>>,
                edges: &Vec<(usize, usize)>|
     -> (Vec<usize>, bool) {
        let mut seq = vec![cur];
        let mut closed = false;
        loop {
            let next = adj[cur].iter().copied().find(|&e| !used[e]);
            match next {
                Some(e) => {
                    used[e] = true;
                    let (x, y) = edges[e];
                    cur = if x == cur { y } else { x };
                    if cur == start_vertex {
                        closed = true;
                        break;
                    }
                    seq.push(cur);
                }
                None => break,
            }
            if seq.len() > edges.len() + 2 {
                break;
            }
        }
        (seq, closed)
    };

    for start_edge in 0..edges.len() {
        if used[start_edge] {
            continue;
        }
        used[start_edge] = true;
        let (a, b) = edges[start_edge];
        let (fwd, closed) = walk(b, start_edge, &mut used, &adj, &edges);
        let mut poly: Vec<Vec3> = Vec::new();
        poly.push(points[a]);
        for i in fwd.iter() {
            poly.push(points[*i]);
        }
        if !closed {
            // Continue on the other side of the start edge.
            let (mut back, _) = walk(a, start_edge, &mut used, &adj, &edges);
            back.remove(0); // drop the duplicate of `a`
            back.reverse();
            let mut full: Vec<Vec3> = back.iter().map(|i| points[*i]).collect();
            full.extend(poly);
            poly = full;
        } else {
            poly.pop(); // drop duplicate closing point
        }
        if poly.len() >= 2 {
            paths.push((poly, closed));
        }
    }
    paths
}

/// Chain segments into closed polygons only.
fn chain_loops(segs: &[(Vec3, Vec3)], tol: f64) -> Vec<Vec<Vec3>> {
    chain_paths(segs, tol)
        .into_iter()
        .filter(|(_, closed)| *closed)
        .map(|(p, _)| p)
        .collect()
}

/// Close open polylines with a straight run along the waterplane. A hull that
/// is open at the deck produces open station cuts; the section area convention
/// bounds them with the waterline, which this restores.
pub fn close_along_waterplane(paths: Vec<(Vec<Vec3>, bool)>) -> Vec<Vec<Vec3>> {
    paths
        .into_iter()
        .map(|(mut p, closed)| {
            if !closed && p.len() >= 2 {
                p.push(p[0]);
            }
            p
        })
        .collect()
}

/// Area, centroid and second moments of a set of planar loops expressed in a
/// plane basis. Returns `None` when no closed loop was found.
fn loop_moments(loops: &[Vec<Vec3>], plane: &Plane) -> Option<PolygonMoments> {
    if loops.is_empty() {
        return None;
    }
    let (u, v) = plane.basis();
    let mut total = PolygonMoments::default();
    for poly in loops {
        let pts: Vec<(f64, f64)> = poly.iter().map(|p| (u.dot(*p), v.dot(*p))).collect();
        total.accumulate(&PolygonMoments::from_polygon(&pts));
    }
    if total.area.abs() < 1e-12 {
        return None;
    }
    Some(total)
}

/// Area and moments of a set of plane loops, clipped to the submerged side of
/// the waterplane. Coordinates are expressed in the plane's own 2D basis.
pub fn clipped_loop_moments(loops: &[Vec<Vec3>], plane: &Plane, wp: &Plane) -> Option<PolygonMoments> {
    if loops.is_empty() {
        return None;
    }
    let (u, v) = plane.basis();
    let mut total = PolygonMoments::default();
    for poly in loops {
        let pts: Vec<(f64, f64)> = poly.iter().map(|p| (u.dot(*p), v.dot(*p))).collect();
        let n = pts.len();
        if n < 3 {
            continue;
        }
        let mut clipped: Vec<(f64, f64)> = Vec::with_capacity(n + 2);
        for i in 0..n {
            let j = (i + 1) % n;
            let sa = wp.side(poly[i]);
            let sb = wp.side(poly[j]);
            if sa <= CLIP_EPS {
                clipped.push(pts[i]);
            }
            if (sa < -CLIP_EPS && sb > CLIP_EPS) || (sa > CLIP_EPS && sb < -CLIP_EPS) {
                let t = sa / (sa - sb);
                clipped.push((
                    pts[i].0 + (pts[j].0 - pts[i].0) * t,
                    pts[i].1 + (pts[j].1 - pts[i].1) * t,
                ));
            }
        }
        if clipped.len() >= 3 {
            total.accumulate(&crate::math::polygon_moments_2d(&clipped));
        }
    }
    if total.area.abs() < 1e-12 {
        None
    } else {
        Some(total)
    }
}

/// Area of the cross section of the hull at a longitudinal station, below the
/// waterplane.
pub fn section_area(verts: &[Vec3], tris: &[[usize; 3]], x: f64, wp: &Plane) -> f64 {
    let plane = Plane::new(Vec3::new(1.0, 0.0, 0.0), -x);
    let segs = plane_segments(verts, tris, &plane);
    let scale = (wp.n.x.abs() + 1.0).max(1.0);
    let _ = scale;
    let paths = chain_paths(&segs, 1e-7);
    let polys = close_along_waterplane(paths);
    clipped_loop_moments(&polys, &plane, wp)
        .map(|m| m.area.abs())
        .unwrap_or(0.0)
}

/// A hull refined to the requested precision and ready to be measured. Build
/// it once and call [`Hull::hydrostatics`] for every floating condition - this
/// is what the stability solver does.
pub struct Hull {
    pub verts: Vec<Vec3>,
    pub tris: Vec<[usize; 3]>,
    pub keel: f64,
    pub length: f64,
    pub model_min: Vec3,
    pub model_max: Vec3,
}

impl Hull {
    pub fn new(surface: &Surface, subdiv: SubdivisionOptions) -> Self {
        let (verts, tris) = hull_triangles(
            surface,
            &HydroOptions {
                draft: 0.0,
                trim: 0.0,
                heel_deg: 0.0,
                water_density: surface.settings.water_density,
                appendage_coefficient: 1.0,
                mainframe_x: None,
                stations: 0,
                subdivision: subdiv,
            },
        );
        let (min, max) = surface.extents();
        let length = if surface.settings.length > 0.0 {
            surface.settings.length
        } else {
            max.x - min.x
        };
        Self {
            verts,
            tris,
            keel: surface.lowest_hydrostatics_point(),
            length,
            model_min: min,
            model_max: max,
        }
    }

    pub fn hydrostatics(&self, opts: &HydroOptions) -> Hydrostatics {
        self.hydrostatics_inner(opts)
    }

    /// Submerged volume only - fast path used by the stability solver.
    pub fn displaced_volume(&self, draft: f64, trim: f64, heel_deg: f64) -> f64 {
        self.buoyancy(draft, trim, heel_deg).0
    }

    /// Submerged volume and centre of buoyancy in one pass, without the
    /// waterplane chaining, section areas or coefficients that
    /// [`Hull::hydrostatics`] builds. This is what a floating-condition solver
    /// iterates on; `b` is in model coordinates.
    pub fn buoyancy(&self, draft: f64, trim: f64, heel_deg: f64) -> (f64, Vec3) {
        let wp = waterplane_for(self.keel, self.length, draft, trim, heel_deg);
        let origin = wp.project(Vec3::ZERO);
        let (volume, moment, _, _, _) = integrate(&self.verts, &self.tris, &wp);
        let b = if volume > 1e-12 {
            moment / volume + origin
        } else {
            Vec3::ZERO
        };
        (volume, b)
    }

    fn hydrostatics_inner(&self, opts: &HydroOptions) -> Hydrostatics {
    let verts = &self.verts;
    let tris = &self.tris;
    let wp = waterplane_for(self.keel, self.length, opts.draft, opts.trim, opts.heel_deg);
    let mut result = Hydrostatics {
        draft: opts.draft,
        trim_m: opts.trim,
        heel_deg: opts.heel_deg,
        water_density: opts.water_density,
        appendage_coefficient: opts.appendage_coefficient,
        ..Default::default()
    };

    let model_min = self.model_min;
    let model_max = self.model_max;
    result.model_min = model_min;
    result.model_max = model_max;

    let origin = wp.project(Vec3::ZERO);
    let (volume, moment, wetted, smin, smax) = integrate(verts, tris, &wp);
    result.volume_m3 = volume * opts.appendage_coefficient;
    result.wetted_surface_m2 = wetted;
    result.displacement_kg = volume * opts.appendage_coefficient * opts.water_density;
    result.displacement_t = result.displacement_kg / 1000.0;

    if volume > 1e-12 {
        // `moment` is taken about the foot of the perpendicular from the world
        // origin onto the waterplane, so shift back into model coordinates.
        let b = moment / volume + origin;
        result.lcb = b;
        result.kb = b.z - self.keel;
    } else {
        result.warnings.push(Warning {
            code: "nothing_submerged",
            message: "the model is not submerged at this draft".into(),
        });
    }

    result.submerged_length = smax.x - smin.x;
    result.submerged_beam = smax.y - smin.y;
    result.absolute_draft = if volume > 1e-12 {
        let mut d: f64 = 0.0;
        for v in verts.iter() {
            let s = wp.side(*v);
            if s < 0.0 {
                d = d.max(-s);
            }
        }
        d
    } else {
        0.0
    };

    // ---- waterplane ----------------------------------------------------
    let segs = plane_segments(verts, tris, &wp);
    let (min, max) = (model_min, model_max);
    let scale = (max.x - min.x).max(max.y - min.y).max(1.0);
    let loops = chain_loops(&segs, scale * 1e-7);
    if let Some(m) = loop_moments(&loops, &wp) {
        let (u, v) = wp.basis();
        let area = m.area.abs();
        result.waterplane_area_m2 = area;
        let cx = m.mx / m.area;
        let cy = m.my / m.area;
        result.lcf = u * cx + v * cy;
        result.waterplane_inertia_transverse = (m.ix - cy * cy * m.area).abs();
        result.waterplane_inertia_longitudinal = (m.iy - cx * cx * m.area).abs();

        // Waterline length/beam from the actual intersection points.
        if let Some(poly) = loops.iter().max_by_key(|p| p.len()) {
            let (mut umin, mut umax) = (f64::INFINITY, f64::NEG_INFINITY);
            let (mut vmin, mut vmax) = (f64::INFINITY, f64::NEG_INFINITY);
            for p in poly {
                let pu = u.dot(*p);
                let pv = v.dot(*p);
                umin = umin.min(pu);
                umax = umax.max(pu);
                vmin = vmin.min(pv);
                vmax = vmax.max(pv);
            }
            result.lwl = umax - umin;
            result.bwl = vmax - vmin;
        }
    } else if !segs.is_empty() {
        result.warnings.push(Warning {
            code: "waterline_not_closed",
            message: "waterline segments did not form a closed loop; waterplane properties are unavailable".into(),
        });
    }
    if result.lwl == 0.0 {
        // Fall back to the submerged extents.
        result.lwl = result.submerged_length;
        result.bwl = result.submerged_beam;
    }

    // ---- metacentres ---------------------------------------------------
    if result.volume_m3 > 1e-12 {
        result.bm_transverse = result.waterplane_inertia_transverse / result.volume_m3;
        result.bm_longitudinal = result.waterplane_inertia_longitudinal / result.volume_m3;
        result.km_transverse = result.kb + result.bm_transverse;
        result.km_longitudinal = result.kb + result.bm_longitudinal;
    }

    // ---- coefficients --------------------------------------------------
    let draft = opts.draft;
    if draft > 0.0 && result.volume_m3 > 0.0 {
        let l = if result.lwl > 0.0 { result.lwl } else { result.submerged_length };
        let b = if result.bwl > 0.0 { result.bwl } else { result.submerged_beam };
        if l * b * draft > 0.0 {
            result.block_coefficient = result.volume_m3 / (l * b * draft);
        }
        if result.waterplane_area_m2 * draft > 0.0 {
            result.vertical_prismatic_coefficient =
                result.volume_m3 / (result.waterplane_area_m2 * draft);
        }
        if l * b > 0.0 {
            result.waterplane_coefficient = result.waterplane_area_m2 / (l * b);
        }
        result.mainframe_x = opts.mainframe_x.unwrap_or(result.lcf.x);
        let am = section_area(verts, tris, result.mainframe_x, &wp);
        result.mainframe_area_m2 = am;
        if am * l > 0.0 {
            result.prismatic_coefficient = result.volume_m3 / (am * l);
        }
        if b * draft > 0.0 {
            result.midship_coefficient = am / (b * draft);
        }
    }

    // ---- section area curve -------------------------------------------
    if opts.stations > 1 && result.volume_m3 > 0.0 {
        let x0 = smin.x.max(model_min.x);
        let x1 = smax.x.min(model_max.x);
        let n = opts.stations;
        for i in 0..n {
            let t = i as f64 / (n - 1) as f64;
            let x = x0 + (x1 - x0) * t;
            let a = section_area(verts, tris, x, &wp);
            result.sac.push(SectionArea { x, area: a });
        }
    }

    // ---- lateral area (centreline profile) -----------------------------
    let centre = Plane::new(Vec3::new(0.0, 1.0, 0.0), 0.0);
    let lsegs = plane_segments(verts, tris, &centre);
    let lpaths = chain_paths(&lsegs, 1e-7);
    let lloops = close_along_waterplane(lpaths);
    if let Some(m) = clipped_loop_moments(&lloops, &centre, &wp) {
        let (u, v) = centre.basis();
        result.lateral_area_m2 = m.area.abs();
        let cx = m.mx / m.area;
        let cy = m.my / m.area;
        result.lateral_centroid = u * cx + v * cy;
    }

    result
    }
}

/// Compute hydrostatics for one floating condition.
pub fn compute(surface: &Surface, opts: &HydroOptions) -> Hydrostatics {
    Hull::new(surface, opts.subdivision).hydrostatics(opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The waterplane must make exactly the requested angle with the
    /// horizontal, and positive heel must put starboard deeper.
    #[test]
    fn waterplane_tilts_by_the_heel_angle_towards_starboard() {
        for heel in [0.0, 5.0, 10.0, 30.0, -20.0] {
            let wp = waterplane_for(0.0, 12.0, 1.0, 0.0, heel);
            // The slope along y is `tan(heel)`, in the plane's own terms.
            let at = |y: f64| {
                // Height of the surface at (x = 6, y) given the plane.
                let x = 6.0;
                -(wp.n.x * x + wp.n.y * y + wp.d) / wp.n.z
            };
            let slope = (at(1.0) - at(0.0)).abs();
            let expected = deg_to_rad(heel).tan().abs();
            assert!(
                (slope - expected).abs() < 1e-9,
                "heel {heel}: slope {slope} vs {expected}"
            );
            // Positive heel: starboard (y > 0) is deeper, so a point there is
            // below the surface.
            let starboard = wp.side(Vec3::new(6.0, 2.0, 0.5));
            let port = wp.side(Vec3::new(6.0, -2.0, 0.5));
            if heel > 0.0 {
                assert!(starboard < port, "heel {heel} must sink starboard: {starboard} vs {port}");
            } else if heel < 0.0 {
                assert!(starboard > port, "heel {heel} must sink port: {starboard} vs {port}");
            }
        }
    }

    #[test]
    fn trim_raises_the_bow() {
        let wp = waterplane_for(0.0, 12.0, 1.0, 0.4, 0.0);
        let bow = wp.side(Vec3::new(12.0, 0.0, 1.0));
        let stern = wp.side(Vec3::new(0.0, 0.0, 1.0));
        assert!(bow > 0.0 && stern < 0.0, "positive trim lifts the bow: {bow} / {stern}");
    }
}