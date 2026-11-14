//! Flatten a triangle mesh into the plane with the least stretch.
//!
//! [`flatten`] lays a panel flat so that every triangle keeps its 3D shape as nearly as least
//! squares allows: the as-rigid-as-possible (ARAP) local/global method of Liu, Zhang, Xu, Gotsman
//! and Gortler, "A Local/Global Approach to Mesh Parameterization", Symposium on Geometry
//! Processing 2008 (Computer Graphics Forum 27(5)), sections 4.3 to 4.4. The energy is their
//! equation 1: each triangle's flat edges against its true edges turned by the rotation that
//! fits them best, weighted by the cotangent of the opposite angle. The local step finds those
//! rotations; the global step solves their equation 4, a cotangent Laplacian that depends only on
//! the 3D mesh, here by conjugate gradients ([`solve_cg`]) with one vertex held.
//!
//! Two choices differ from the paper. It starts from [`crate::develop::unfold`]'s edge-joining
//! development instead of Floater's or LSCM's parameterisation, and a pattern with a triangle
//! turned over is refused ([`FlattenError::Fold`]) instead of being repaired: a folded pattern
//! is a panel that is too far from lying flat, and the builder should hear so.
//!
//! On a mesh that is exactly developable (a cylinder or cone faceted on its rulings, a folded
//! strip) the result keeps every edge length. On any other mesh it is the least stretch, which
//! [`strain`] reports. That is geometry: what plywood does when it is bent is not modelled.

use std::collections::HashMap;

use crate::math::Vec3;

/// A sparse symmetric matrix stored by rows, both triangles kept.
#[derive(Debug, Clone)]
pub struct SymMatrix {
    n: usize,
    rows: Vec<Vec<(usize, f64)>>,
}

impl SymMatrix {
    pub fn new(n: usize) -> Self {
        Self {
            n,
            rows: vec![Vec::new(); n],
        }
    }

    /// Adds `v` to `(i, j)` and, when `i != j`, to `(j, i)`.
    pub fn add(&mut self, i: usize, j: usize, v: f64) {
        self.add_one(i, j, v);
        if i != j {
            self.add_one(j, i, v);
        }
    }

    fn add_one(&mut self, i: usize, j: usize, v: f64) {
        let row = &mut self.rows[i];
        match row.iter_mut().find(|(c, _)| *c == j) {
            Some((_, a)) => *a += v,
            None => row.push((j, v)),
        }
    }

    /// `out = A x`.
    pub fn mul(&self, x: &[f64], out: &mut [f64]) {
        for (i, row) in self.rows.iter().enumerate() {
            out[i] = row.iter().map(|&(j, a)| a * x[j]).sum();
        }
    }

    pub fn diagonal(&self) -> Vec<f64> {
        (0..self.n)
            .map(|i| {
                self.rows[i]
                    .iter()
                    .find(|(c, _)| *c == i)
                    .map_or(0.0, |&(_, a)| a)
            })
            .collect()
    }
}

/// How a conjugate-gradient solve ended.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CgOutcome {
    pub iterations: usize,
    pub relative_residual: f64,
    pub converged: bool,
}

/// Solves `A x = b` for a symmetric positive-definite `A` by conjugate gradients with a Jacobi
/// (diagonal) preconditioner. `x` is the starting guess and receives the answer. Stops when
/// `‖b − A x‖ / ‖b‖ < tol` or after `max_iter` iterations.
pub fn solve_cg(a: &SymMatrix, b: &[f64], x: &mut [f64], tol: f64, max_iter: usize) -> CgOutcome {
    let n = a.n;
    let dot = |u: &[f64], v: &[f64]| u.iter().zip(v).map(|(p, q)| p * q).sum::<f64>();
    let b_norm = dot(b, b).sqrt();
    if b_norm == 0.0 {
        x.iter_mut().for_each(|v| *v = 0.0);
        return CgOutcome {
            iterations: 0,
            relative_residual: 0.0,
            converged: true,
        };
    }
    let inv_diag: Vec<f64> = a
        .diagonal()
        .iter()
        .map(|&d| if d > 0.0 { 1.0 / d } else { 1.0 })
        .collect();
    let mut ax = vec![0.0; n];
    a.mul(x, &mut ax);
    let mut r: Vec<f64> = b.iter().zip(&ax).map(|(p, q)| p - q).collect();
    let mut z: Vec<f64> = r.iter().zip(&inv_diag).map(|(p, q)| p * q).collect();
    let mut p = z.clone();
    let mut rz = dot(&r, &z);
    let mut ap = vec![0.0; n];
    let mut residual = dot(&r, &r).sqrt() / b_norm;
    let mut it = 0;
    while residual >= tol && it < max_iter {
        a.mul(&p, &mut ap);
        let pap = dot(&p, &ap);
        if pap <= 0.0 || !pap.is_finite() {
            break;
        }
        let alpha = rz / pap;
        for i in 0..n {
            x[i] += alpha * p[i];
            r[i] -= alpha * ap[i];
        }
        it += 1;
        residual = dot(&r, &r).sqrt() / b_norm;
        if residual < tol {
            break;
        }
        for i in 0..n {
            z[i] = r[i] * inv_diag[i];
        }
        let rz_new = dot(&r, &z);
        let beta = rz_new / rz;
        rz = rz_new;
        for i in 0..n {
            p[i] = z[i] + beta * p[i];
        }
    }
    CgOutcome {
        iterations: it,
        relative_residual: residual,
        converged: residual < tol,
    }
}

/// Welds vertices closer than `1e-9 ×` the bounding-box diagonal and drops triangles with an area
/// under `1e-14 × diagonal²` (a pole fan's collapsed triangles). Returns the vertices left in a
/// triangle, the triangles, and the map from each old vertex to its new index (`usize::MAX` for a
/// vertex left in no triangle).
pub fn clean(verts: &[Vec3], tris: &[[usize; 3]]) -> (Vec<Vec3>, Vec<[usize; 3]>, Vec<usize>) {
    if verts.is_empty() {
        return (Vec::new(), Vec::new(), Vec::new());
    }
    let (lo, hi) = verts
        .iter()
        .fold((verts[0], verts[0]), |(lo, hi), p| (lo.min(*p), hi.max(*p)));
    let diag = (hi - lo).length();
    let tol = 1e-9 * diag;
    // Weld: a hash grid with cells of the weld distance; a vertex joins the first earlier vertex
    // within reach in its own or a neighbouring cell.
    let cell = |p: Vec3| -> (i64, i64, i64) {
        if tol == 0.0 {
            return (0, 0, 0);
        }
        (
            ((p.x - lo.x) / tol).floor() as i64,
            ((p.y - lo.y) / tol).floor() as i64,
            ((p.z - lo.z) / tol).floor() as i64,
        )
    };
    let mut grid: HashMap<(i64, i64, i64), Vec<usize>> = HashMap::new();
    let mut weld = vec![0usize; verts.len()];
    for (i, &p) in verts.iter().enumerate() {
        let c = cell(p);
        let mut found = None;
        'search: for dx in -1..=1 {
            for dy in -1..=1 {
                for dz in -1..=1 {
                    if let Some(list) = grid.get(&(c.0 + dx, c.1 + dy, c.2 + dz)) {
                        if let Some(&j) = list.iter().find(|&&j| verts[j].dist(p) <= tol) {
                            found = Some(j);
                            break 'search;
                        }
                    }
                }
            }
        }
        match found {
            Some(j) => weld[i] = weld[j],
            None => {
                weld[i] = i;
                grid.entry(c).or_default().push(i);
            }
        }
    }
    let min_area = 1e-14 * diag * diag;
    let kept: Vec<[usize; 3]> = tris
        .iter()
        .map(|f| [weld[f[0]], weld[f[1]], weld[f[2]]])
        .filter(|f| {
            f[0] != f[1]
                && f[1] != f[2]
                && f[2] != f[0]
                && 0.5
                    * (verts[f[1]] - verts[f[0]])
                        .cross(verts[f[2]] - verts[f[0]])
                        .length()
                    >= min_area
        })
        .collect();
    let mut new_index = vec![usize::MAX; verts.len()];
    let mut out_v = Vec::new();
    for f in &kept {
        for &v in f {
            if new_index[v] == usize::MAX {
                new_index[v] = out_v.len();
                out_v.push(verts[v]);
            }
        }
    }
    let out_t = kept
        .iter()
        .map(|f| [new_index[f[0]], new_index[f[1]], new_index[f[2]]])
        .collect();
    let map = (0..verts.len()).map(|i| new_index[weld[i]]).collect();
    (out_v, out_t, map)
}

/// The mesh's boundary as closed vertex loops, each running the way its triangles wind. An edge
/// is on the boundary when only one triangle uses it.
pub fn boundary_loops(tris: &[[usize; 3]]) -> Vec<Vec<usize>> {
    let mut count: HashMap<(usize, usize), usize> = HashMap::new();
    for f in tris {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            *count.entry((a.min(b), a.max(b))).or_default() += 1;
        }
    }
    // Boundary half-edges a → b, in triangle order so the result does not depend on hashing.
    let mut next: HashMap<usize, Vec<usize>> = HashMap::new();
    let mut starts = Vec::new();
    for f in tris {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            if count[&(a.min(b), a.max(b))] == 1 {
                next.entry(a).or_default().push(b);
                starts.push(a);
            }
        }
    }
    let mut loops = Vec::new();
    for s in starts {
        if next.get(&s).is_none_or(|v| v.is_empty()) {
            continue;
        }
        let mut lp = vec![s];
        let mut at = s;
        while let Some(b) = next.get_mut(&at).and_then(|v| v.pop()) {
            if b == s {
                break;
            }
            lp.push(b);
            at = b;
        }
        loops.push(lp);
    }
    loops
}

/// Why a mesh could not be flattened.
#[derive(Debug, Clone, PartialEq)]
pub enum FlattenError {
    /// No triangles.
    Empty,
    /// The boundary is not one loop: a ring, a closed surface, or pieces.
    NotADisc { loops: usize },
    /// The flat pattern lies over itself at this triangle.
    Fold { triangle: usize },
    /// The flat outline crosses itself.
    SelfCrossing,
    /// The least-squares solve did not settle.
    NoConvergence,
}

impl std::fmt::Display for FlattenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            FlattenError::Empty => write!(f, "it has no surface"),
            FlattenError::NotADisc { loops: 0 } => {
                write!(
                    f,
                    "it is closed all round, so it cannot be cut from one flat sheet"
                )
            }
            FlattenError::NotADisc { loops } => write!(
                f,
                "its edge makes {loops} separate loops; one flat sheet has one (split it where it \
                 wraps round, or join its pieces)"
            ),
            FlattenError::Fold { triangle } => write!(
                f,
                "laid flat it folds over itself (at triangle {triangle}): it is too far from a \
                 shape that lies flat"
            ),
            FlattenError::SelfCrossing => {
                write!(f, "laid flat its outline crosses itself")
            }
            FlattenError::NoConvergence => write!(f, "the flattening did not settle"),
        }
    }
}

impl std::error::Error for FlattenError {}

/// A flat pattern: one 2D point per input vertex.
#[derive(Debug, Clone)]
pub struct Flat {
    pub xy: Vec<(f64, f64)>,
    /// Local/global iterations run.
    pub iterations: usize,
}

/// Twice-signed area halved: positive when `a, b, c` run anticlockwise.
pub fn signed_area(a: (f64, f64), b: (f64, f64), c: (f64, f64)) -> f64 {
    0.5 * ((b.0 - a.0) * (c.1 - a.1) - (b.1 - a.1) * (c.0 - a.0))
}

/// The triangle laid in its own plane: first vertex at the origin, second on +x.
fn isometric_frame(a: Vec3, b: Vec3, c: Vec3) -> [(f64, f64); 3] {
    let e = (b - a).norm();
    let n = (b - a).cross(c - a).norm();
    let g = n.cross(e);
    [
        (0.0, 0.0),
        ((b - a).dot(e), 0.0),
        ((c - a).dot(e), (c - a).dot(g)),
    ]
}

/// Flattens a cleaned triangle mesh with one boundary loop, keeping every triangle as close to
/// its true shape as least squares allows. See the module header.
pub fn flatten(verts: &[Vec3], tris: &[[usize; 3]]) -> Result<Flat, FlattenError> {
    if tris.is_empty() || verts.is_empty() {
        return Err(FlattenError::Empty);
    }
    let loops = boundary_loops(tris).len();
    if loops != 1 {
        return Err(FlattenError::NotADisc { loops });
    }
    let n = verts.len();

    // Seed: the edge-joining development, each vertex at the mean of its per-face positions.
    let seed = crate::develop::unfold(verts, tris);
    let mut u = vec![(0.0, 0.0); n];
    let mut hits = vec![0usize; n];
    for face in &seed.faces {
        for (&v, &p) in face.verts.iter().zip(&face.xy) {
            u[v].0 += p.0;
            u[v].1 += p.1;
            hits[v] += 1;
        }
    }
    for (p, &h) in u.iter_mut().zip(&hits) {
        if h > 0 {
            p.0 /= h as f64;
            p.1 /= h as f64;
        }
    }

    // Each triangle's own flat frame, and the cotangent of the angle opposite each edge
    // (half-edge k runs from corner k to corner k+1).
    let frames: Vec<[(f64, f64); 3]> = tris
        .iter()
        .map(|f| isometric_frame(verts[f[0]], verts[f[1]], verts[f[2]]))
        .collect();
    let mut weight: Vec<[f64; 3]> = frames
        .iter()
        .map(|x| {
            let mut w = [0.0; 3];
            for (k, wk) in w.iter_mut().enumerate() {
                let o = x[(k + 2) % 3];
                let (a, b) = (x[k], x[(k + 1) % 3]);
                let (ax, ay, bx, by) = (a.0 - o.0, a.1 - o.1, b.0 - o.0, b.1 - o.1);
                let cross = (ax * by - ay * bx).abs();
                *wk = if cross > 0.0 {
                    (ax * bx + ay * by) / cross
                } else {
                    0.0
                };
            }
            w
        })
        .collect();
    // An edge whose cotangent sum is not positive would make the system indefinite: give its
    // half-edges a small share of the mean positive weight instead. The energy keeps its zero
    // at an exact isometry, so a developable mesh still flattens exactly.
    let mut edge_sum: HashMap<(usize, usize), (f64, usize)> = HashMap::new();
    for (f, w) in tris.iter().zip(&weight) {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            let e = edge_sum.entry((a.min(b), a.max(b))).or_default();
            e.0 += w[k];
            e.1 += 1;
        }
    }
    let positive: Vec<f64> = edge_sum
        .values()
        .map(|e| e.0)
        .filter(|&s| s > 0.0)
        .collect();
    let mean = if positive.is_empty() {
        1.0
    } else {
        positive.iter().sum::<f64>() / positive.len() as f64
    };
    for (f, w) in tris.iter().zip(weight.iter_mut()) {
        for k in 0..3 {
            let (a, b) = (f[k], f[(k + 1) % 3]);
            let (sum, count) = edge_sum[&(a.min(b), a.max(b))];
            if sum <= 0.0 {
                w[k] = 1e-6 * mean / count as f64;
            }
        }
    }

    // The global matrix (Liu et al. eq. 4), vertex 0 held at its seed.
    let pin = 0usize;
    let unknown = |v: usize| {
        if v < pin {
            Some(v)
        } else if v == pin {
            None
        } else {
            Some(v - 1)
        }
    };
    let mut a = SymMatrix::new(n - 1);
    for (f, w) in tris.iter().zip(&weight) {
        for k in 0..3 {
            let (i, j) = (f[k], f[(k + 1) % 3]);
            let c = w[k];
            if let Some(ii) = unknown(i) {
                a.add(ii, ii, c);
            }
            if let Some(jj) = unknown(j) {
                a.add(jj, jj, c);
            }
            if let (Some(ii), Some(jj)) = (unknown(i), unknown(j)) {
                a.add(ii, jj, -c);
            }
        }
    }

    let energy = |u: &[(f64, f64)], rot: &[(f64, f64)]| -> f64 {
        let mut e = 0.0;
        for ((f, w), (x, &(c, s))) in tris.iter().zip(&weight).zip(frames.iter().zip(rot)) {
            for k in 0..3 {
                let (i, j) = (f[k], f[(k + 1) % 3]);
                let dx = (x[k].0 - x[(k + 1) % 3].0, x[k].1 - x[(k + 1) % 3].1);
                let r = (c * dx.0 - s * dx.1, s * dx.0 + c * dx.1);
                let du = (u[i].0 - u[j].0, u[i].1 - u[j].1);
                e += w[k] * ((du.0 - r.0).powi(2) + (du.1 - r.1).powi(2));
            }
        }
        e
    };
    // The scale of the energy, for an absolute stop on a mesh that flattens exactly.
    let scale: f64 = tris
        .iter()
        .zip(&weight)
        .zip(&frames)
        .map(|((_, w), x)| {
            (0..3)
                .map(|k| {
                    let dx = (x[k].0 - x[(k + 1) % 3].0, x[k].1 - x[(k + 1) % 3].1);
                    w[k].abs() * (dx.0 * dx.0 + dx.1 * dx.1)
                })
                .sum::<f64>()
        })
        .sum();

    let mut rot = vec![(1.0, 0.0); tris.len()];
    let mut bx = vec![0.0; n - 1];
    let mut by = vec![0.0; n - 1];
    let mut sx: Vec<f64> = (0..n).filter(|&v| v != pin).map(|v| u[v].0).collect();
    let mut sy: Vec<f64> = (0..n).filter(|&v| v != pin).map(|v| u[v].1).collect();
    let mut prev = f64::INFINITY;
    let mut iterations = 0;
    for it in 0..200 {
        iterations = it + 1;
        // Local step: the rotation closest to each triangle's map, from the cross-covariance
        // S = Σ w (u_i − u_j)(x_i − x_j)ᵀ; for 2 × 2 the closest rotation has angle
        // atan2(S10 − S01, S00 + S11), always orientation-keeping.
        for (t, (f, w)) in tris.iter().zip(&weight).enumerate() {
            let x = &frames[t];
            let mut m = [[0.0; 2]; 2];
            for k in 0..3 {
                let (i, j) = (f[k], f[(k + 1) % 3]);
                let du = [u[i].0 - u[j].0, u[i].1 - u[j].1];
                let dx = [x[k].0 - x[(k + 1) % 3].0, x[k].1 - x[(k + 1) % 3].1];
                for (r, row) in m.iter_mut().enumerate() {
                    for (c, cell) in row.iter_mut().enumerate() {
                        *cell += w[k] * du[r] * dx[c];
                    }
                }
            }
            let th = (m[1][0] - m[0][1]).atan2(m[0][0] + m[1][1]);
            rot[t] = (th.cos(), th.sin());
        }
        // Global step: right-hand side of eq. 4.
        bx.iter_mut().for_each(|v| *v = 0.0);
        by.iter_mut().for_each(|v| *v = 0.0);
        for (t, (f, w)) in tris.iter().zip(&weight).enumerate() {
            let x = &frames[t];
            let (c, s) = rot[t];
            for k in 0..3 {
                let (i, j) = (f[k], f[(k + 1) % 3]);
                let dx = (x[k].0 - x[(k + 1) % 3].0, x[k].1 - x[(k + 1) % 3].1);
                let r = (w[k] * (c * dx.0 - s * dx.1), w[k] * (s * dx.0 + c * dx.1));
                if let Some(ii) = unknown(i) {
                    bx[ii] += r.0;
                    by[ii] += r.1;
                    if j == pin {
                        bx[ii] += w[k] * u[pin].0;
                        by[ii] += w[k] * u[pin].1;
                    }
                }
                if let Some(jj) = unknown(j) {
                    bx[jj] -= r.0;
                    by[jj] -= r.1;
                    if i == pin {
                        bx[jj] += w[k] * u[pin].0;
                        by[jj] += w[k] * u[pin].1;
                    }
                }
            }
        }
        let max_iter = 20 * n;
        let ox = solve_cg(&a, &bx, &mut sx, 1e-12, max_iter);
        let oy = solve_cg(&a, &by, &mut sy, 1e-12, max_iter);
        if !ox.converged || !oy.converged {
            return Err(FlattenError::NoConvergence);
        }
        for (v, p) in u.iter_mut().enumerate() {
            if let Some(i) = unknown(v) {
                *p = (sx[i], sy[i]);
            }
        }
        let e = energy(&u, &rot);
        if e <= 1e-24 * scale || (prev.is_finite() && (prev - e).abs() <= 1e-10 * prev) {
            break;
        }
        prev = e;
    }

    // Outside up: the pattern is reflected when most triangles came out clockwise.
    let negative = tris
        .iter()
        .filter(|f| signed_area(u[f[0]], u[f[1]], u[f[2]]) < 0.0)
        .count();
    if 2 * negative > tris.len() {
        u.iter_mut().for_each(|p| p.1 = -p.1);
    }
    check_flat(tris, &u)?;
    Ok(Flat { xy: u, iterations })
}

/// Refuses a flat pattern with a triangle turned over or an outline that crosses itself.
pub(crate) fn check_flat(tris: &[[usize; 3]], xy: &[(f64, f64)]) -> Result<(), FlattenError> {
    if let Some(t) = tris
        .iter()
        .position(|f| signed_area(xy[f[0]], xy[f[1]], xy[f[2]]) <= 0.0)
    {
        return Err(FlattenError::Fold { triangle: t });
    }
    let mut segs = Vec::new();
    for lp in boundary_loops(tris) {
        for k in 0..lp.len() {
            segs.push((lp[k], lp[(k + 1) % lp.len()]));
        }
    }
    let orient = |a: (f64, f64), b: (f64, f64), c: (f64, f64)| signed_area(a, b, c).signum();
    for (s, &(a, b)) in segs.iter().enumerate() {
        for &(c, d) in &segs[s + 1..] {
            if a == c || a == d || b == c || b == d {
                continue;
            }
            let (pa, pb, pc, pd) = (xy[a], xy[b], xy[c], xy[d]);
            if orient(pa, pb, pc) * orient(pa, pb, pd) < 0.0
                && orient(pc, pd, pa) * orient(pc, pd, pb) < 0.0
            {
                return Err(FlattenError::SelfCrossing);
            }
        }
    }
    Ok(())
}

/// How far a flat pattern stretches its triangles' edges, as fractions of their 3D lengths.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Strain {
    /// Root mean square over every triangle edge, each weighted by its triangle's area.
    pub rms: f64,
    /// Largest over the edges of triangles that are not slivers (shortest altitude under
    /// 0.05 × the longest edge): a sliver's edges stretch a lot for a tiny change of shape.
    pub max: f64,
}

/// The strain of the pattern `xy` against the mesh.
pub fn strain(verts: &[Vec3], tris: &[[usize; 3]], xy: &[(f64, f64)]) -> Strain {
    let (mut sum, mut weight, mut max) = (0.0, 0.0, 0.0f64);
    for f in tris {
        let (a, b, c) = (verts[f[0]], verts[f[1]], verts[f[2]]);
        let area = 0.5 * (b - a).cross(c - a).length();
        let lengths = [a.dist(b), b.dist(c), c.dist(a)];
        let longest = lengths.iter().cloned().fold(0.0, f64::max);
        let sliver = longest == 0.0 || 2.0 * area / longest < 0.05 * longest;
        for k in 0..3 {
            let l3 = lengths[k];
            if l3 == 0.0 {
                continue;
            }
            let (p, q) = (xy[f[k]], xy[f[(k + 1) % 3]]);
            let l2 = ((q.0 - p.0).powi(2) + (q.1 - p.1).powi(2)).sqrt();
            let e = (l2 - l3) / l3;
            sum += area * e * e;
            weight += area;
            if !sliver {
                max = max.max(e.abs());
            }
        }
    }
    Strain {
        rms: if weight > 0.0 {
            (sum / weight).sqrt()
        } else {
            0.0
        },
        max,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use super::shapes;
    use crate::math::Vec3;

    fn len2((a, b): ((f64, f64), (f64, f64))) -> f64 {
        ((b.0 - a.0).powi(2) + (b.1 - a.1).powi(2)).sqrt()
    }

    fn flat_dist(xy: &[(f64, f64)], i: usize, j: usize) -> f64 {
        len2((xy[i], xy[j]))
    }

    /// Every triangle edge keeps its 3D length.
    fn assert_isometric(v: &[Vec3], t: &[[usize; 3]], xy: &[(f64, f64)], tol: f64) {
        for f in t {
            for k in 0..3 {
                let (i, j) = (f[k], f[(k + 1) % 3]);
                let d3 = v[i].dist(v[j]);
                let d2 = flat_dist(xy, i, j);
                assert!((d3 - d2).abs() < tol, "edge {i}-{j}: 3D {d3}, flat {d2}");
            }
        }
    }

    fn area3(v: &[Vec3], t: &[[usize; 3]]) -> f64 {
        t.iter()
            .map(|f| 0.5 * (v[f[1]] - v[f[0]]).cross(v[f[2]] - v[f[0]]).length())
            .sum()
    }

    fn area2(xy: &[(f64, f64)], t: &[[usize; 3]]) -> f64 {
        t.iter()
            .map(|f| signed_area(xy[f[0]], xy[f[1]], xy[f[2]]))
            .sum()
    }

    /// Closed form: a cylinder faceted on its rulings is a strip of flat rectangles, so it
    /// flattens with no stretch to a rectangle `len × facets·2r·sin(Δθ/2)`.
    #[test]
    fn a_cylinder_meshed_on_its_rulings_flattens_to_its_rectangle() {
        let (r, len, n, span) = (0.5, 2.0, 24usize, 120f64.to_radians());
        let (v, t) = shapes::ruled_cylinder(r, len, n, span);
        let flat = flatten(&v, &t).expect("flattens");
        assert_isometric(&v, &t, &flat.xy, 1e-9);
        let cols = n + 1;
        let w = n as f64 * 2.0 * r * (span / n as f64 / 2.0).sin();
        let (c00, c0n, c40, c4n) = (0, n, 4 * cols, 4 * cols + n);
        assert!((flat_dist(&flat.xy, c00, c40) - len).abs() < 1e-9);
        assert!((flat_dist(&flat.xy, c00, c0n) - w).abs() < 1e-9);
        assert!((flat_dist(&flat.xy, c40, c4n) - w).abs() < 1e-9);
        assert!((flat_dist(&flat.xy, c00, c4n) - (len * len + w * w).sqrt()).abs() < 1e-9);
    }

    /// Where the flat lines through the two end rulings meet: the cone's apex in the pattern.
    fn line_meet(p0: (f64, f64), p1: (f64, f64), q0: (f64, f64), q1: (f64, f64)) -> (f64, f64) {
        let (dx, dy) = (p1.0 - p0.0, p1.1 - p0.1);
        let (ex, ey) = (q1.0 - q0.0, q1.1 - q0.1);
        let den = dx * ey - dy * ex;
        let s = ((q0.0 - p0.0) * ey - (q0.1 - p0.1) * ex) / den;
        (p0.0 + s * dx, p0.1 + s * dy)
    }

    fn angle_at(o: (f64, f64), a: (f64, f64), b: (f64, f64)) -> f64 {
        let (ax, ay) = (a.0 - o.0, a.1 - o.1);
        let (bx, by) = (b.0 - o.0, b.1 - o.1);
        (ax * by - ay * bx).atan2(ax * bx + ay * by).abs()
    }

    /// Closed form: two rulings of a cone of half-angle α, Δθ apart round the axis, meet at the
    /// apex at `2·asin(sin α·sin(Δθ/2))`; the facets are flat, so the pattern is a sector of
    /// that angle times the facet count, radii the slant distances.
    #[test]
    fn a_cone_meshed_on_its_rulings_flattens_to_its_sector() {
        let (alpha, n, span) = (30f64.to_radians(), 36usize, std::f64::consts::PI);
        let (v, t) = shapes::ruled_cone(alpha, 0.3, 1.0, n, span);
        let flat = flatten(&v, &t).expect("flattens");
        assert_isometric(&v, &t, &flat.xy, 1e-9);
        let cols = n + 1;
        let apex = line_meet(flat.xy[0], flat.xy[cols], flat.xy[n], flat.xy[cols + n]);
        let sector = n as f64 * 2.0 * (alpha.sin() * (span / n as f64 / 2.0).sin()).asin();
        assert!((angle_at(apex, flat.xy[cols], flat.xy[cols + n]) - sector).abs() < 1e-9);
        for k in 0..cols {
            assert!((len2((apex, flat.xy[k])) - 0.3).abs() < 1e-9);
            assert!((len2((apex, flat.xy[cols + k])) - 1.0).abs() < 1e-9);
        }
    }

    /// The same closed form, with the cone meshed to its apex: a pole of 37 coincident vertices
    /// and 36 collapsed triangles, which `clean` welds and drops (Review Focus 3).
    #[test]
    fn a_cone_to_its_apex_flattens_after_cleaning() {
        let (alpha, n, span) = (30f64.to_radians(), 36usize, std::f64::consts::PI);
        let (v0, t0) = shapes::ruled_cone(alpha, 0.0, 1.0, n, span);
        let (v, t, map) = clean(&v0, &t0);
        assert_eq!(t.len(), n, "one collapsed triangle per facet is dropped");
        assert_eq!(v.len(), n + 2, "the pole is welded to one vertex");
        let flat = flatten(&v, &t).expect("flattens");
        assert_isometric(&v, &t, &flat.xy, 1e-9);
        let cols = n + 1;
        let apex = flat.xy[map[0]];
        let sector = n as f64 * 2.0 * (alpha.sin() * (span / n as f64 / 2.0).sin()).asin();
        assert!((angle_at(apex, flat.xy[map[cols]], flat.xy[map[cols + n]]) - sector).abs() < 1e-9);
        for k in 0..cols {
            assert!((len2((apex, flat.xy[map[cols + k]])) - 1.0).abs() < 1e-9);
        }
    }

    /// Convergence, stated as such: a faceted cylinder whose triangles are not on rulings is not
    /// developable, but its least stretch shrinks as the facets get finer.
    #[test]
    fn skewed_diagonals_leave_a_residual_that_shrinks() {
        let rms: Vec<f64> = [12, 24, 48]
            .iter()
            .map(|&n| {
                let (v, t) = shapes::skewed_cylinder(n);
                let flat = flatten(&v, &t).expect("flattens");
                strain(&v, &t, &flat.xy).rms
            })
            .collect();
        assert!(rms[0] > rms[1] && rms[1] > rms[2], "{rms:?}");
        assert!(rms[0] > 0.0);
        assert!(rms[2] < 1e-4, "{rms:?}");
    }

    /// Closed form: a strip folded along a straight line unfolds to the flat strip.
    #[test]
    fn a_folded_strip_flattens_exactly() {
        let (v, t) = shapes::folded_strip();
        let flat = flatten(&v, &t).expect("flattens");
        assert_isometric(&v, &t, &flat.xy, 1e-9);
        assert!((area2(&flat.xy, &t) - area3(&v, &t)).abs() < 1e-12);
        assert!((area3(&v, &t) - 0.2).abs() < 1e-12);
    }

    /// Best rigid (rotation + translation, no reflection) fit of `b` onto `a`; largest distance.
    fn rigid_misfit(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
        let n = a.len() as f64;
        let ca = a
            .iter()
            .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
        let cb = b
            .iter()
            .fold((0.0, 0.0), |s, p| (s.0 + p.0 / n, s.1 + p.1 / n));
        let (mut sxx, mut sxy) = (0.0, 0.0);
        for (p, q) in a.iter().zip(b) {
            let (px, py) = (p.0 - ca.0, p.1 - ca.1);
            let (qx, qy) = (q.0 - cb.0, q.1 - cb.1);
            sxx += qx * px + qy * py;
            sxy += qx * py - qy * px;
        }
        let th = sxy.atan2(sxx);
        let (c, s) = (th.cos(), th.sin());
        a.iter()
            .zip(b)
            .map(|(p, q)| {
                let (qx, qy) = (q.0 - cb.0, q.1 - cb.1);
                let (rx, ry) = (c * qx - s * qy + ca.0, s * qx + c * qy + ca.1);
                ((rx - p.0).powi(2) + (ry - p.1).powi(2)).sqrt()
            })
            .fold(0.0, f64::max)
    }

    #[test]
    fn the_result_does_not_depend_on_face_order_or_placement() {
        let (v, t) = shapes::skewed_cylinder(24);
        let a = flatten(&v, &t).expect("flattens");
        // Faces in a scrambled order, vertices turned and moved.
        let mut t2 = t.clone();
        let len = t2.len();
        for i in 0..len {
            t2.swap(i, (i * 37 + 11) % len);
        }
        let (c, s) = (0.6f64, 0.8f64);
        let v2: Vec<Vec3> = v
            .iter()
            .map(|p| Vec3::new(c * p.x - s * p.z + 3.0, p.y - 1.0, s * p.x + c * p.z + 0.5))
            .collect();
        let b = flatten(&v2, &t2).expect("flattens");
        let misfit = rigid_misfit(&a.xy, &b.xy);
        assert!(
            misfit < 1e-9,
            "misfit {misfit} it {} {}",
            a.iterations,
            b.iterations
        );
    }

    /// Gauss–Bonnet. Discrete and exact for any faceted disc: the angle defects at interior
    /// vertices plus the turning `π − θ` at boundary vertices sum to `2π`. Smooth: a cap of polar
    /// angle φ on a unit sphere holds curvature `2π(1 − cos φ)`. Each interior vertex stands for
    /// the surface half way to its neighbours, so the interior defects hold the cap out to half a
    /// ring short of the edge, `φ' = φ(R − ½)/R`, within the faceting error (2 %).
    /// A surface with curvature cannot lie flat, so the flattening must stretch.
    #[test]
    fn a_spherical_cap_has_its_gauss_bonnet_defect_and_stretches() {
        let polar = 40f64.to_radians();
        let (v, t) = shapes::cap(polar, 10, 48);
        let boundary: std::collections::HashSet<usize> =
            boundary_loops(&t).into_iter().flatten().collect();
        let mut angle = vec![0.0; v.len()];
        for f in &t {
            for k in 0..3 {
                let o = v[f[k]];
                let a = v[f[(k + 1) % 3]] - o;
                let b = v[f[(k + 2) % 3]] - o;
                angle[f[k]] += a.cross(b).length().atan2(a.dot(b));
            }
        }
        let defect: f64 = (0..v.len())
            .filter(|i| !boundary.contains(i))
            .map(|i| std::f64::consts::TAU - angle[i])
            .sum();
        let turning: f64 = boundary
            .iter()
            .map(|&i| std::f64::consts::PI - angle[i])
            .sum();
        assert!((defect + turning - std::f64::consts::TAU).abs() < 1e-9);
        let inner = polar * (10.0 - 0.5) / 10.0;
        let smooth = std::f64::consts::TAU * (1.0 - inner.cos());
        assert!(
            (defect - smooth).abs() < 0.02 * smooth,
            "{defect} vs {smooth}"
        );
        let flat = flatten(&v, &t).expect("flattens");
        assert!(strain(&v, &t, &flat.xy).rms > 0.001);
    }

    #[test]
    fn a_ring_is_not_a_disc() {
        let (v, t) = shapes::ring(24);
        assert!(matches!(
            flatten(&v, &t),
            Err(FlattenError::NotADisc { loops: 2 })
        ));
    }

    #[test]
    fn a_fold_is_refused() {
        let t = [[0, 1, 2], [0, 2, 3], [0, 3, 4], [0, 4, 1]];
        let good = [(0.0, 0.0), (1.0, 0.0), (0.0, 1.0), (-1.0, 0.0), (0.0, -1.0)];
        assert!(check_flat(&t, &good).is_ok());
        let mut folded = good;
        folded[2] = (1.5, -0.5);
        assert!(matches!(
            check_flat(&t, &folded),
            Err(FlattenError::Fold { .. })
        ));
    }

    /// A strip wound one and a fifth times round a spiral: every triangle the right way up,
    /// but the end lies over the start.
    #[test]
    fn a_self_crossing_outline_is_refused() {
        let n = 60;
        let mut xy = Vec::new();
        for k in 0..=n {
            let th = 2.4 * std::f64::consts::PI * k as f64 / n as f64;
            let r = 1.0 + 0.1 * th / std::f64::consts::TAU;
            for w in [0.0, 0.3] {
                xy.push(((r + w) * th.cos(), (r + w) * th.sin()));
            }
        }
        let mut t = Vec::new();
        for k in 0..n {
            let (a, b, c, d) = (2 * k, 2 * k + 1, 2 * k + 3, 2 * k + 2);
            t.push([a, b, c]);
            t.push([a, c, d]);
        }
        assert!(matches!(
            check_flat(&t, &xy),
            Err(FlattenError::SelfCrossing)
        ));
    }

    /// A 3×3 symmetric positive-definite system, solved exactly by Cramer's rule in the test.
    #[test]
    fn cg_solves_a_small_spd_system() {
        let m = [[4.0, 1.0, 0.0], [1.0, 3.0, 1.0], [0.0, 1.0, 2.0]];
        let b = [1.0, 2.0, 3.0];
        let mut a = SymMatrix::new(3);
        for (i, row) in m.iter().enumerate() {
            for (j, &v) in row.iter().enumerate().skip(i) {
                if v != 0.0 {
                    a.add(i, j, v);
                }
            }
        }
        let det3 = |c: [[f64; 3]; 3]| {
            c[0][0] * (c[1][1] * c[2][2] - c[1][2] * c[2][1])
                - c[0][1] * (c[1][0] * c[2][2] - c[1][2] * c[2][0])
                + c[0][2] * (c[1][0] * c[2][1] - c[1][1] * c[2][0])
        };
        let d = det3(m);
        let mut x = vec![0.0; 3];
        let out = solve_cg(&a, &b, &mut x, 1e-14, 100);
        assert!(out.converged, "{out:?}");
        for k in 0..3 {
            let mut c = m;
            for r in 0..3 {
                c[r][k] = b[r];
            }
            let exact = det3(c) / d;
            assert!((x[k] - exact).abs() < 1e-12, "x[{k}] = {} vs {exact}", x[k]);
        }
    }

    /// A path of `n` nodes with unit springs, node 0 held at 0 and a unit load on every other node.
    /// Node `i` carries the load of nodes `i..n`, so `x_i − x_{i−1} = n − i` and
    /// `x_i = Σ_{k=1..i} (n − k) = i(2n − 1 − i)/2` (n = 3: 2, 3).
    #[test]
    fn cg_on_a_path_laplacian_matches_closed_form() {
        let n = 50;
        // Unknowns are nodes 1..n, stored at 0..n-1.
        let mut a = SymMatrix::new(n - 1);
        for e in 0..n - 1 {
            // Edge between node e and node e+1.
            let (p, q) = (e, e + 1);
            if p > 0 {
                a.add(p - 1, p - 1, 1.0);
                a.add(p - 1, q - 1, -1.0);
            }
            a.add(q - 1, q - 1, 1.0);
        }
        let b = vec![1.0; n - 1];
        let mut x = vec![0.0; n - 1];
        let out = solve_cg(&a, &b, &mut x, 1e-14, 10 * n);
        assert!(out.converged, "{out:?}");
        for i in 1..n {
            let exact = (i * (2 * n - 1 - i)) as f64 / 2.0;
            assert!(
                (x[i - 1] - exact).abs() < 1e-9,
                "x_{i} = {} vs {exact}",
                x[i - 1]
            );
        }
    }
}

/// Closed-form test shapes, shared with the panel tests.
#[cfg(test)]
pub(crate) mod shapes {
    use crate::math::Vec3;

    type Mesh = (Vec<Vec3>, Vec<[usize; 3]>);

    /// Splits the grid quad `(i, k)` of a `rows × cols` vertex grid into two triangles along the
    /// diagonal from `(i, k)` to `(i + 1, k + 1)`.
    fn grid(rows: usize, cols: usize, at: impl Fn(usize, usize) -> Vec3) -> Mesh {
        let mut v = Vec::new();
        for i in 0..rows {
            for k in 0..cols {
                v.push(at(i, k));
            }
        }
        let id = |i: usize, k: usize| i * cols + k;
        let mut t = Vec::new();
        for i in 0..rows - 1 {
            for k in 0..cols - 1 {
                let (a, b, c, d) = (id(i, k), id(i, k + 1), id(i + 1, k + 1), id(i + 1, k));
                t.push([a, b, c]);
                t.push([a, c, d]);
            }
        }
        (v, t)
    }

    /// A cylinder of radius `r` and length `len` along x, `facets` flat facets over `span`
    /// radians, every quad bounded by two rulings (so each facet is a flat rectangle).
    pub fn ruled_cylinder(r: f64, len: f64, facets: usize, span: f64) -> Mesh {
        let rings = 5;
        grid(rings, facets + 1, |i, k| {
            let th = span * k as f64 / facets as f64;
            Vec3::new(
                len * i as f64 / (rings - 1) as f64,
                r * th.cos(),
                r * th.sin(),
            )
        })
    }

    /// The same cylinder (radius 0.5, length 2, 120°) with every other ring turned by half a
    /// facet, so the triangles are not on rulings and the mesh is not exactly developable.
    pub fn skewed_cylinder(facets: usize) -> Mesh {
        let (r, len, span, rings) = (0.5, 2.0, 120f64.to_radians(), 5);
        let dth = span / facets as f64;
        grid(rings, facets + 1, |i, k| {
            let shift = if i % 2 == 1 && k > 0 && k < facets {
                0.5 * dth
            } else {
                0.0
            };
            let th = dth * k as f64 + shift;
            Vec3::new(
                len * i as f64 / (rings - 1) as f64,
                r * th.cos(),
                r * th.sin(),
            )
        })
    }

    /// A cone with its apex at the origin and its axis along x, half-angle `alpha`, between
    /// slant distances `s0` and `s1`, `facets` facets over `span` radians. Each quad lies in the
    /// plane of its two rulings. With `s0 = 0` the inner ring is a pole of coincident vertices.
    pub fn ruled_cone(alpha: f64, s0: f64, s1: f64, facets: usize, span: f64) -> Mesh {
        grid(2, facets + 1, |i, k| {
            let s = if i == 0 { s0 } else { s1 };
            let th = span * k as f64 / facets as f64;
            Vec3::new(
                s * alpha.cos(),
                s * alpha.sin() * th.cos(),
                s * alpha.sin() * th.sin(),
            )
        })
    }

    /// A spherical cap of polar angle `polar` on a unit sphere about +z: a pole vertex, `rings`
    /// rings of `per_ring` vertices.
    pub fn cap(polar: f64, rings: usize, per_ring: usize) -> Mesh {
        let mut v = vec![Vec3::new(0.0, 0.0, 1.0)];
        for j in 1..=rings {
            let phi = polar * j as f64 / rings as f64;
            for k in 0..per_ring {
                let th = std::f64::consts::TAU * k as f64 / per_ring as f64;
                v.push(Vec3::new(
                    phi.sin() * th.cos(),
                    phi.sin() * th.sin(),
                    phi.cos(),
                ));
            }
        }
        let id = |j: usize, k: usize| 1 + (j - 1) * per_ring + k % per_ring;
        let mut t = Vec::new();
        for k in 0..per_ring {
            t.push([0, id(1, k), id(1, k + 1)]);
        }
        for j in 1..rings {
            for k in 0..per_ring {
                let (a, b, c, d) = (id(j, k), id(j + 1, k), id(j + 1, k + 1), id(j, k + 1));
                t.push([a, b, c]);
                t.push([a, c, d]);
            }
        }
        (v, t)
    }

    /// A 0.2 m × 1 m strip along x, folded 90° up across its middle.
    pub fn folded_strip() -> Mesh {
        grid(5, 2, |i, k| {
            let s = 0.25 * i as f64;
            let y = 0.2 * k as f64;
            if s <= 0.5 {
                Vec3::new(s, y, 0.0)
            } else {
                Vec3::new(0.5, y, s - 0.5)
            }
        })
    }

    /// An open cylinder all the way round: a ring with two edges.
    pub fn ring(facets: usize) -> Mesh {
        let (v, mut t) = grid(3, facets, |i, k| {
            let th = std::f64::consts::TAU * k as f64 / facets as f64;
            Vec3::new(0.5 * i as f64, th.cos(), th.sin())
        });
        for i in 0..2 {
            let (a, b, c, d) = (
                i * facets + facets - 1,
                i * facets,
                (i + 1) * facets,
                (i + 1) * facets + facets - 1,
            );
            t.push([a, b, c]);
            t.push([a, c, d]);
        }
        (v, t)
    }
}
