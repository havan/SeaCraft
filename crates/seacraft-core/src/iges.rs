//! IGES 5.3 export of hull cut curves and B-spline surface patches
//! (entities 126 and 128).
//!
//! Rhino, ShipConstructor and SolidWorks all read IGES, and a naval architect
//! wants the station, buttock and waterline curves as exact splines, not as
//! polylines: a polyline becomes thousands of tiny lines in the receiving
//! application and cannot be used for framing offsets.
//!
//! Each curve is fitted by **global interpolation**: chord-length parameters,
//! an averaging knot vector and a solve of the (n+1) × (n+1) collocation
//! system, which makes the spline pass exactly through every sampled point.
//! The result is a degree-3 clamped, non-rational B-spline, written as IGES
//! entity 126 with unit weights.
//!
//! Entity 128 (B-spline surface) carries a lofted patch: [`fit_surface`]
//! interpolates a regular grid the same way — averaged chord-length parameters
//! in each direction, averaging knot vectors, then a `u` collocation solve per
//! column followed by a `v` solve per row — so the patch passes through every
//! grid point. `export::iges_surface` builds those grids from the refined
//! hull's station curves (starboard and port halves separately, since a
//! subdivided hull has no rectangular parametrisation of its own). Both are
//! written by [`write_iges_surfaces`].
//!
//! The tensor-product surface interpolation is adapted from Tangency Labs'
//! `solidcore` (`crates/solidcore/src/nurbs/srf_fit.rs`), MIT OR Apache-2.0;
//! see `ATTRIBUTION.md` for the full provenance. That project's
//! `nurbs/iges.rs` is a reader, not a writer, so the file layout below is
//! SeaCraft's own.
//!
//! File layout follows the IGES 5.3 specification: Start, Global, Directory
//! Entry, Parameter Data and Terminate sections, 80-column records with the
//! section letter in column 73 and a 7-digit sequence number in 74–80.

use crate::math::Vec3;
use std::fmt::Write as _;

/// A clamped B-spline curve with optional weights (all 1.0 when not rational).
#[derive(Debug, Clone, PartialEq)]
pub struct BsplineCurve {
    pub degree: usize,
    /// `control.len() + degree + 1` knots.
    pub knots: Vec<f64>,
    pub control: Vec<Vec3>,
    pub weights: Vec<f64>,
}

impl BsplineCurve {
    /// Whether every weight is one, i.e. the curve is polynomial.
    pub fn is_rational(&self) -> bool {
        self.weights.iter().any(|w| (*w - 1.0).abs() > 1e-12)
    }

    /// Evaluate with de Boor's algorithm.
    pub fn eval(&self, u: f64) -> Vec3 {
        de_boor(self.degree, &self.knots, &self.control, u)
    }
}

/// de Boor's algorithm on a borrowed control polygon.
fn de_boor(degree: usize, knots: &[f64], control: &[Vec3], u: f64) -> Vec3 {
    let p = degree;
    let n = control.len();
    if n == 0 {
        return Vec3::ZERO;
    }
    if n <= p || knots.len() < n + p + 1 {
        return control[0];
    }
    let lo = knots[p];
    let hi = knots[n];
    let u = u.clamp(lo, hi);
    // Find the knot span.
    let mut k = p;
    while k < n - 1 && knots[k + 1] <= u {
        k += 1;
    }
    let mut d: Vec<Vec3> = (0..=p)
        .map(|j| control[(k + j).saturating_sub(p).min(n - 1)])
        .collect();
    for r in 1..=p {
        for j in (r..=p).rev() {
            let i = k + j - p;
            let denom = knots[i + p + 1 - r] - knots[i];
            let alpha = if denom.abs() < 1e-12 {
                0.0
            } else {
                (u - knots[i]) / denom
            };
            d[j] = d[j - 1] * (1.0 - alpha) + d[j] * alpha;
        }
    }
    d[p]
}

/// A tensor-product B-spline surface with optional weights (all 1.0 when not
/// rational).
///
/// `control[i][j]` is the pole at `(u_i, v_j)`: `nu` rows of `nv` points, with
/// `knots_u.len() == nu + degree_u + 1` and `knots_v.len() == nv + degree_v + 1`.
/// `weights` is a grid of the same shape, so the surface is rational when any
/// weight differs from one.
#[derive(Debug, Clone, PartialEq)]
pub struct BsplineSurface {
    pub degree_u: usize,
    pub degree_v: usize,
    /// `nu + degree_u + 1` knots.
    pub knots_u: Vec<f64>,
    /// `nv + degree_v + 1` knots.
    pub knots_v: Vec<f64>,
    /// `nu` rows of `nv` poles.
    pub control: Vec<Vec<Vec3>>,
    /// `nu` rows of `nv` weights.
    pub weights: Vec<Vec<f64>>,
}

impl BsplineSurface {
    /// Whether any weight is not one, i.e. the surface is rational.
    pub fn is_rational(&self) -> bool {
        self.weights
            .iter()
            .flatten()
            .any(|w| (*w - 1.0).abs() > 1e-12)
    }

    /// Evaluate at `(u, v)` with de Boor's algorithm.
    ///
    /// For the given `u` every row of poles is a B-spline curve along `v`
    /// (evaluated by de Boor), and those curves are blended by the row basis
    /// functions `N_i(u)`. A rational surface is evaluated in homogeneous
    /// coordinates, the weights being carried as a fourth coordinate and
    /// divided out at the end. A surface whose knot vectors are too short to
    /// span its poles, or which has no poles at all, evaluates to the origin
    /// rather than indexing out of bounds.
    pub fn eval(&self, u: f64, v: f64) -> Vec3 {
        let nu = self.control.len();
        let nv = self.control.first().map_or(0, Vec::len);
        if nu == 0 || nv == 0 || self.knots_u.len() < nu + 1 || self.knots_v.len() < nv + 1 {
            return Vec3::ZERO;
        }
        // Charts built by hand may claim more knots than they carry; never ask
        // de Boor for more than the vectors can support.
        let p = self.degree_u.min(nu - 1).min(self.knots_u.len() - nu - 1);
        let q = self.degree_v.min(nv - 1).min(self.knots_v.len() - nv - 1);
        let rational = self.is_rational();
        let mut num = Vec3::ZERO;
        let mut den = 0.0;
        for i in 0..nu {
            let weight_i = basis(i, p, u, &self.knots_u);
            if weight_i == 0.0 {
                continue;
            }
            let row = &self.control[i];
            if rational {
                // Split the row into numerator `sum N_j w_ij P_ij` and
                // denominator `sum N_j w_ij`, each a plain B-spline curve.
                let ws = self
                    .weights
                    .get(i)
                    .cloned()
                    .unwrap_or_else(|| vec![1.0; nv]);
                let scaled: Vec<Vec3> = row.iter().zip(&ws).map(|(point, w)| *point * *w).collect();
                let wrow: Vec<Vec3> = ws.iter().map(|w| Vec3::new(*w, *w, *w)).collect();
                num += de_boor(q, &self.knots_v, &scaled, v) * weight_i;
                den += de_boor(q, &self.knots_v, &wrow, v).x * weight_i;
            } else {
                num += de_boor(q, &self.knots_v, row, v) * weight_i;
                den += weight_i;
            }
        }
        if den.abs() > 1e-12 {
            num / den
        } else {
            num
        }
    }
}

/// The Cox–de Boor basis function `N_{i,p}(u)`.
fn basis(i: usize, p: usize, u: f64, knots: &[f64]) -> f64 {
    // A clamped curve's right end belongs to the span *before* the last knot;
    // evaluating exactly at it leaves every `N_{i,0}` false and makes the
    // last collocation row all zeros (a singular system). Nudging the query
    // keeps the recursion consistent and costs at most 1e-12 of accuracy.
    let last = knots[knots.len() - 1];
    let u = if u >= last { last - 1e-12 } else { u };
    if p == 0 {
        return if knots[i] <= u && u < knots[i + 1] {
            1.0
        } else {
            0.0
        };
    }
    let mut sum = 0.0;
    let d1 = knots[i + p] - knots[i];
    if d1.abs() > 1e-12 {
        sum += (u - knots[i]) / d1 * basis(i, p - 1, u, knots);
    }
    let d2 = knots[i + p + 1] - knots[i + 1];
    if d2.abs() > 1e-12 {
        sum += (knots[i + p + 1] - u) / d2 * basis(i + 1, p - 1, u, knots);
    }
    sum
}

/// Interpolate `points` with a clamped B-spline of `degree`.
///
/// Returns `None` when there are too few points, the degree is out of range, or
/// the points repeat so much that the system is singular.
pub fn fit_curve(points: &[Vec3], degree: usize) -> Option<BsplineCurve> {
    let n = points.len();
    let p = degree.clamp(1, 3).min(n.saturating_sub(1));
    if n < 2 || p == 0 {
        return None;
    }
    // Chord-length parameters, normalised to 0..1.
    let mut params = vec![0.0f64; n];
    let mut total = 0.0;
    for i in 1..n {
        total += points[i].dist(points[i - 1]);
        params[i] = total;
    }
    if total < 1e-12 {
        return None;
    }
    for value in &mut params {
        *value /= total;
    }
    // Averaging knot vector: p+1 zeros, interior knots, p+1 ones.
    let knots = averaging_knots(&params, p);

    // Collocation system N * P = Q, solved by Gaussian elimination with
    // partial pivoting. n is a few hundred at most, so a dense solve is fine.
    let mut a = vec![vec![0.0f64; n + 1]; n];
    for (row, &u) in params.iter().enumerate() {
        for (col, cell) in a[row][..n].iter_mut().enumerate() {
            *cell = basis(col, p, u, &knots);
        }
        a[row][n] = 0.0;
    }
    // One solve per component: the matrix is the same, only the right-hand
    // side changes.
    let mut control = vec![Vec3::ZERO; n];
    for axis in 0..3 {
        let mut m: Vec<Vec<f64>> = a.clone();
        for (row, point) in points.iter().enumerate() {
            m[row][n] = match axis {
                0 => point.x,
                1 => point.y,
                _ => point.z,
            }
        }
        let solved = solve(&mut m)?;
        for (i, value) in solved.iter().enumerate() {
            match axis {
                0 => control[i].x = *value,
                1 => control[i].y = *value,
                _ => control[i].z = *value,
            }
        }
    }
    Some(BsplineCurve {
        degree: p,
        knots,
        control,
        weights: vec![1.0; n],
    })
}

/// Averaging knot vector for the parameters `t` and degree `p`: `p + 1` zeros,
/// the interior knots as the mean of `p` consecutive parameters, `p + 1` ones.
///
/// Requires `1 <= p < t.len()`; the result has `t.len() + p + 1` knots.
fn averaging_knots(t: &[f64], p: usize) -> Vec<f64> {
    let n = t.len();
    let mut knots = vec![0.0; p + 1];
    for j in 1..n - p {
        knots.push(t[j..j + p].iter().sum::<f64>() / p as f64);
    }
    knots.extend(std::iter::repeat_n(1.0, p + 1));
    knots
}

/// Collocation matrix `N_k(t_i)`: one row per parameter, one column per pole.
fn collocation(t: &[f64], p: usize, knots: &[f64]) -> Vec<Vec<f64>> {
    t.iter()
        .map(|u| (0..t.len()).map(|k| basis(k, p, *u, knots)).collect())
        .collect()
}

/// Averaged chord-length parameters for `lines` sequences of `count` points,
/// each point given by `point(line, index)`.
///
/// `t[0]` is 0, `t[count - 1]` is 1, and every interior value is the mean of
/// the normalised cumulative chord lengths of all the sequences whose total
/// length does not vanish (averaging over the columns of a grid is what makes
/// the surface's `u` and `v` parameters follow the data rather than the
/// indexing). `None` when a direction has fewer than two points or every
/// sequence in it is degenerate — the same cases that leave the collocation
/// system without a solution.
fn averaged_params(
    count: usize,
    lines: usize,
    point: impl Fn(usize, usize) -> Vec3,
) -> Option<Vec<f64>> {
    if count < 2 || lines == 0 {
        return None;
    }
    let mut t = vec![0.0; count];
    let mut used = 0usize;
    for line in 0..lines {
        let mut total = 0.0;
        let mut cum = vec![0.0; count];
        for (i, cell) in cum.iter_mut().enumerate().skip(1) {
            total += point(line, i).dist(point(line, i - 1));
            *cell = total;
        }
        if total <= 1e-12 {
            continue;
        }
        used += 1;
        for (value, distance) in t[1..count - 1].iter_mut().zip(&cum[1..count - 1]) {
            *value += distance / total;
        }
    }
    if used == 0 {
        return None;
    }
    for value in &mut t[1..count - 1] {
        *value /= used as f64;
    }
    t[count - 1] = 1.0;
    // A parameter direction that does not advance gives a singular system.
    if t.windows(2).any(|w| w[1] <= w[0]) {
        return None;
    }
    Some(t)
}

/// Interpolate a regular grid of points with a clamped tensor-product B-spline
/// surface.
///
/// `grid[i][j]` becomes the point at `(params_u[i], params_v[j])`, so the
/// surface passes exactly through every grid point: averaged chord-length
/// parameters, averaging knot vectors, a collocation solve along `u` for every
/// column, then one along `v` for every row of that intermediate grid. The
/// degrees are clamped to `1..=3` and to one less than the number of points,
/// and the weights are all one (the fit is not rational).
///
/// Returns `None` for a grid with fewer than two rows or columns, a ragged
/// grid, a direction whose parameters cannot advance (repeated points), or a
/// collocation system that is singular.
pub fn fit_surface(grid: &[Vec<Vec3>], degree_u: usize, degree_v: usize) -> Option<BsplineSurface> {
    let nu = grid.len();
    if nu < 2 {
        return None;
    }
    let nv = grid.first().map_or(0, Vec::len);
    if nv < 2 || grid.iter().any(|row| row.len() != nv) {
        return None;
    }
    let p = degree_u.clamp(1, 3).min(nu - 1);
    let q = degree_v.clamp(1, 3).min(nv - 1);
    let params_u = averaged_params(nu, nv, |j, i| grid[i][j])?;
    let params_v = averaged_params(nv, nu, |i, j| grid[i][j])?;
    let knots_u = averaging_knots(&params_u, p);
    let knots_v = averaging_knots(&params_v, q);

    // Along u, column by column: every column of the grid is one curve of the
    // intermediate net.
    let a_u = collocation(&params_u, p, &knots_u);
    let mut intermediate = vec![vec![Vec3::ZERO; nv]; nu];
    for j in 0..nv {
        let rhs: Vec<Vec3> = (0..nu).map(|i| grid[i][j]).collect();
        let solved = solve_vec3(&a_u, &rhs)?;
        for (i, value) in solved.iter().enumerate() {
            intermediate[i][j] = *value;
        }
    }
    // Along v, row by row: this turns the intermediate net into the control net.
    let a_v = collocation(&params_v, q, &knots_v);
    let mut control = vec![vec![Vec3::ZERO; nv]; nu];
    for i in 0..nu {
        control[i] = solve_vec3(&a_v, &intermediate[i])?;
    }
    Some(BsplineSurface {
        degree_u: p,
        degree_v: q,
        knots_u,
        knots_v,
        control,
        weights: vec![vec![1.0; nv]; nu],
    })
}

/// Solve `a * x = rhs` for vector right-hand sides: one augmented system per
/// component, so the matrix is factored three times. `None` if singular.
fn solve_vec3(a: &[Vec<f64>], rhs: &[Vec3]) -> Option<Vec<Vec3>> {
    let n = a.len();
    if rhs.len() != n || n == 0 {
        return None;
    }
    let mut out = vec![Vec3::ZERO; n];
    for axis in 0..3 {
        let mut m = vec![vec![0.0f64; n + 1]; n];
        for (row, (source, point)) in m.iter_mut().zip(a.iter().zip(rhs)) {
            row[..n].copy_from_slice(source);
            row[n] = match axis {
                0 => point.x,
                1 => point.y,
                _ => point.z,
            };
        }
        let solved = solve(&mut m)?;
        for (i, value) in solved.iter().enumerate() {
            match axis {
                0 => out[i].x = *value,
                1 => out[i].y = *value,
                _ => out[i].z = *value,
            }
        }
    }
    Some(out)
}

/// Solve an augmented `n × (n+1)` system in place; `None` if singular.
fn solve(m: &mut [Vec<f64>]) -> Option<Vec<f64>> {
    let n = m.len();
    for col in 0..n {
        let pivot = (col..n).max_by(|a, b| m[*a][col].abs().total_cmp(&m[*b][col].abs()))?;
        if m[pivot][col].abs() < 1e-12 {
            return None;
        }
        m.swap(col, pivot);
        let scale = m[col][col];
        for cell in m[col][col..=n].iter_mut() {
            *cell /= scale;
        }
        for row in 0..n {
            if row == col {
                continue;
            }
            let factor = m[row][col];
            if factor == 0.0 {
                continue;
            }
            // Split so the pivot row and the target row are borrowed separately.
            let (lo, hi) = m.split_at_mut(row.max(col));
            let (pivot_row, target_row) = if row < col {
                (&hi[0], &mut lo[row])
            } else {
                (&lo[col], &mut hi[0])
            };
            for (t, s) in target_row[col..=n].iter_mut().zip(&pivot_row[col..=n]) {
                *t -= factor * s;
            }
        }
    }
    Some((0..n).map(|i| m[i][n]).collect())
}

// ---- IGES file ---------------------------------------------------------

/// A record: 72 columns of data, the section letter, then the sequence number.
fn record(out: &mut String, letter: char, seq: usize, data: &str) {
    let mut line: String = data.chars().take(72).collect();
    while line.chars().count() < 72 {
        line.push(' ');
    }
    // The sequence number is a seven-digit zero-filled field, as every
    // reference writer emits it.
    let _ = writeln!(out, "{line}{letter}{seq:07}");
}

/// Pack a comma-separated parameter string into lines of at most 64
/// characters, which is exactly what columns 1-64 hold: 65-72 are the
/// directory-entry pointer and 73-80 the section letter and record number.
///
/// The split happens *only* at a comma, so a number is never cut in two — and
/// the line is guaranteed to fit, which it was not when the split waited for
/// both 64 characters and a comma: a long token then pushed the line past
/// column 72 and the record writer silently dropped the tail.
fn param_lines(text: &str) -> Vec<String> {
    let mut lines = Vec::new();
    let mut current = String::new();
    let mut width = 0usize;
    for token in text.split_inclusive(',') {
        let len = token.chars().count();
        if width + len > 64 && width > 0 {
            lines.push(std::mem::take(&mut current));
            width = 0;
        }
        current.push_str(token);
        width += len;
    }
    if !current.is_empty() {
        lines.push(current);
    }
    if lines.is_empty() {
        lines.push(";".to_owned());
    }
    lines
}

/// A drawing to write: a name and the curves, each with a label.
pub struct IgesFile<'a> {
    pub name: &'a str,
    pub curves: Vec<(&'a str, &'a BsplineCurve)>,
    /// Units code from the IGES spec: 1 inch, 2 millimetre, 4 foot, 6 metre.
    pub units: u8,
}

/// One entity of a file: a B-spline curve (126) or a B-spline surface (128).
enum Entity<'a> {
    Curve(&'a BsplineCurve),
    Surface(&'a BsplineSurface),
}

impl Entity<'_> {
    /// The IGES entity type number, declared on both directory records.
    fn type_code(&self) -> &'static str {
        match self {
            Entity::Curve(_) => "126",
            Entity::Surface(_) => "128",
        }
    }

    /// The parameter data, from the entity type to the terminating `;`.
    fn parameters(&self) -> String {
        match self {
            Entity::Curve(curve) => curve_parameters(curve),
            Entity::Surface(surface) => surface_parameters(surface),
        }
    }
}

/// Write an IGES 5.3 file of B-spline curves (entity 126).
pub fn write_iges(file: &IgesFile<'_>) -> String {
    let entities: Vec<(&str, Entity<'_>)> = file
        .curves
        .iter()
        .map(|(label, curve)| (*label, Entity::Curve(curve)))
        .collect();
    let summary = format!("{} B-spline curves, entity 126", file.curves.len());
    write_entities(file.name, &entities, file.units, &summary)
}

/// Write an IGES 5.3 file of B-spline surfaces (entity 128): one patch per
/// record, each labelled in its directory entry.
///
/// `units` is the IGES unit code — 6 for metres, which is the unit model
/// coordinates are in (1 inch, 2 millimetre and 4 foot are the other useful
/// codes). An empty slice still writes a structurally valid file, just with no
/// geometry in it.
pub fn write_iges_surfaces(name: &str, surfaces: &[(&str, &BsplineSurface)], units: u8) -> String {
    let entities: Vec<(&str, Entity<'_>)> = surfaces
        .iter()
        .map(|(label, surface)| (*label, Entity::Surface(surface)))
        .collect();
    let summary = format!("{} B-spline surfaces, entity 128", surfaces.len());
    write_entities(name, &entities, units, &summary)
}

/// Write the Start, Global, Directory Entry and Parameter Data sections of a
/// file holding `entities`, each with its directory-entry label.
fn write_entities(name: &str, entities: &[(&str, Entity<'_>)], units: u8, summary: &str) -> String {
    // The Global section, with parameter and record delimiters declared first.
    let stamp = "261022.000000";
    let global = format!(
        "1H,,1H;,{sender},{name},{system},{version},32,38,6,308,15,{name},1.0,{units},{unit_name},1,0.0,13H{stamp},1.0E-06,10000.0,{author},{org},11,0,13H{stamp};",
        sender = hollerith("SeaCraft"),
        name = hollerith(name),
        system = hollerith("SeaCraft"),
        version = hollerith(env!("CARGO_PKG_VERSION")),
        units = units,
        unit_name = hollerith(unit_name(units)),
        author = hollerith("SeaCraft"),
        org = hollerith("SeaCraft"),
        stamp = stamp,
    );

    // Directory and parameter data are built together: the DE entry needs the
    // sequence number of its first parameter line and how many there are.
    let mut de = String::new();
    let mut pd = String::new();
    let mut de_seq = 0usize;
    let mut pd_seq = 0usize;

    for (label, entity) in entities {
        let params = entity.parameters();
        let lines = param_lines(&params);
        let first_pd = pd_seq + 1;
        for line in &lines {
            pd_seq += 1;
            let mut data = line.clone();
            let pad = 64usize.saturating_sub(data.chars().count());
            data.push_str(&" ".repeat(pad));
            // Columns 65-72: the directory-entry record this data belongs to.
            // **Every** parameter line carries it, not just the last: Open
            // Cascade groups the parameter data of an entity by reading this
            // field on each line, and a block whose earlier lines leave it
            // blank transfers as nothing at all, without an error.
            data.push_str(&format!(" {:07}", de_seq + 1));
            debug_assert!(
                data.chars().count() <= 72,
                "parameter line overflows its 72 columns"
            );
            record(&mut pd, 'P', pd_seq, &data);
        }

        // Two directory records, nine eight-character fields each.
        let field = |v: String| format!("{v:>8}");
        let first = [
            field(entity.type_code().into()),
            field(format!("{first_pd}")),
            field("0".into()),
            field("0".into()),
            field("0".into()),
            field("0".into()),
            field("0".into()),
            field("0".into()),
            // Status: blank status 0, not subordinate, geometry, no hierarchy.
            // Written as an eight-digit field, which is what readers parse.
            field("00000000".into()),
        ]
        .concat();
        de_seq += 1;
        record(&mut de, 'D', de_seq, &first);

        let mut entry_name = label.chars().take(8).collect::<String>();
        while entry_name.chars().count() < 8 {
            entry_name.push(' ');
        }
        let second = [
            field(entity.type_code().into()),
            field("0".into()),
            field("0".into()),
            field(format!("{}", lines.len())),
            field("0".into()),
            field("".into()),
            field("".into()),
            entry_name,
            field("0".into()),
        ]
        .concat();
        de_seq += 1;
        record(&mut de, 'D', de_seq, &second);
    }

    let mut out = String::new();
    let start = format!(
        "SeaCraft {} - {} ({})",
        env!("CARGO_PKG_VERSION"),
        name,
        summary
    );
    for (letter, text) in [('S', start.as_str()), ('G', global.as_str())] {
        let lines = if letter == 'G' {
            split_global(text)
        } else {
            vec![text.to_owned()]
        };
        for (i, line) in lines.iter().enumerate() {
            record(&mut out, letter, i + 1, line);
        }
    }
    out.push_str(&de);
    out.push_str(&pd);

    // Terminate: the record counts of the four sections and the total.
    let counts = |letter: char| {
        out.lines()
            .filter(|l| l.chars().nth(72) == Some(letter))
            .count()
    };
    let (s, g, d, p) = (counts('S'), counts('G'), counts('D'), counts('P'));
    let total = s + g + d + p + 1;
    // Each count is an eight-character field holding the section letter and a
    // seven-digit, space-padded count.
    let mut data = String::new();
    for (letter, count) in [('S', s), ('G', g), ('D', d), ('P', p)] {
        let _ = write!(data, "{letter}{count:>7}");
    }
    record(&mut out, 'T', total, &data);
    out
}

/// The Global section is one long parameter string; wrap it at 72 columns
/// without splitting a Hollerith string.
fn split_global(text: &str) -> Vec<String> {
    // Tokenise first, then pack. A record may be broken "anywhere except
    // inside a Hollerith string", but a reader concatenates the data fields
    // of consecutive records before splitting on commas — so a break placed
    // between the digits of a number silently glues two fields together
    // (`32,38,6` + `308,15` reads back as `32,38,6308,15`, which shifts the
    // unit code and scale out of alignment). Only break at a top-level comma.
    let mut tokens: Vec<String> = Vec::new();
    let mut current = String::new();
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        let ch = chars[i];
        current.push(ch);
        if ch == 'H' && i > 0 && chars[i - 1].is_ascii_digit() {
            // `13H...`: copy that many characters verbatim, commas included.
            let mut digits = String::new();
            let mut j = i;
            while j > 0 && chars[j - 1].is_ascii_digit() {
                j -= 1;
                digits.insert(0, chars[j]);
            }
            let count: usize = digits.parse().unwrap_or(0);
            for _ in 0..count {
                i += 1;
                if i < chars.len() {
                    current.push(chars[i]);
                }
            }
        }
        if ch == ',' || ch == ';' {
            tokens.push(std::mem::take(&mut current));
        }
        i += 1;
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    let mut lines: Vec<String> = Vec::new();
    let mut line = String::new();
    let mut width = 0usize;
    for token in tokens {
        let len = token.chars().count();
        if width + len > 72 && width > 0 {
            lines.push(std::mem::take(&mut line));
            width = 0;
        }
        line.push_str(&token);
        width += len;
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

fn hollerith(text: &str) -> String {
    format!("{}H{}", text.chars().count(), text)
}

fn unit_name(units: u8) -> &'static str {
    match units {
        1 => "IN",
        2 => "MM",
        3 => "CM",
        4 => "FT",
        6 => "M",
        _ => "M",
    }
}

/// A real in parameter-data form: twelve decimals, trailing zeros and the
/// decimal point dropped, which is what every reference writer emits and what
/// keeps the records compact.
fn number(value: f64) -> String {
    format!("{value:.12}")
        .trim_end_matches('0')
        .trim_end_matches('.')
        .to_string()
}

/// Entity 126 parameter data: count, degree, flags, knots, weights, points.
fn curve_parameters(curve: &BsplineCurve) -> String {
    let n = curve.control.len();
    let rational = curve.is_rational();
    // Entity 126: K, M, PROP1 planar, PROP2 closed, PROP3 polynomial,
    // PROP4 periodic, then the knots, the weights (rational only), the control
    // points and the parameter range.
    //
    // PROP3 is the flag that matters: 1 means polynomial (non-rational) and 0
    // means rational, which promises a weight list after the knots. Write 0 for
    // a curve with no weights and every reader consumes the control points as
    // weights, finds a short pole list, and drops the entity — msc/Open Cascade
    // report it as loaded and then transfer nothing, with no error.
    let mut fields: Vec<String> = Vec::new();
    fields.push((n - 1).to_string());
    fields.push(curve.degree.to_string());
    // Non-planar: a planar entity (PROP1 = 1) must also carry its unit normal
    // as three more parameters, and OCCT fails the whole entity when it is
    // missing. The curves are planar, but saying so buys nothing.
    fields.push("0".to_owned());
    fields.push("0".to_owned()); // open
    fields.push(if rational { "0" } else { "1" }.to_owned()); // polynomial
    fields.push("0".to_owned()); // non-periodic
    for k in &curve.knots {
        fields.push(number(*k));
    }
    // Always emit the weight list, even when every weight is one.
    //
    // Open Cascade's own IGES writer declares `PROP3 = 1` (polynomial) and
    // still writes `K+1` unit weights; its reader then accepts the entity.
    // A writer that honours the "rational only" reading of PROP3 and omits
    // the weights produces a block that loads without complaint and transfers
    // to nothing. Mirror the reference writer: flags say polynomial, weights
    // are present anyway.
    for w in &curve.weights {
        fields.push(number(*w));
    }
    for p in &curve.control {
        for v in [p.x, p.y, p.z] {
            fields.push(number(v));
        }
    }
    fields.push(number(curve.knots[curve.degree]));
    fields.push(number(curve.knots[n]));
    let mut text = String::from("126,");
    text.push_str(&fields.join(","));
    text.push(';');
    text
}

/// Entity 128 parameter data: pole counts, degrees, flags, knot vectors,
/// weights, control points and the parameter ranges.
fn surface_parameters(srf: &BsplineSurface) -> String {
    let nu = srf.control.len();
    let nv = srf.control.first().map_or(0, Vec::len);
    if nu == 0 || nv == 0 {
        return "128;".to_owned();
    }
    // IGES counts the *last index* of the pole grid, not the number of poles,
    // exactly as entity 126 does for a curve.
    let (k1, k2) = (nu - 1, nv - 1);
    let rational = srf.is_rational();
    let mut fields: Vec<String> = Vec::new();
    fields.push(k1.to_string());
    fields.push(k2.to_string());
    fields.push(srf.degree_u.to_string());
    fields.push(srf.degree_v.to_string());
    fields.push("0".to_owned()); // open in u
    fields.push("0".to_owned()); // open in v
                                 // PROP3: 1 means polynomial (non-rational). As for entity 126, the flags
                                 // say polynomial while the weight list is written anyway, which is what
                                 // Open Cascade's own writer does and what its reader expects.
    fields.push(if rational { "0" } else { "1" }.to_owned());
    fields.push("0".to_owned()); // non-periodic in u
    fields.push("0".to_owned()); // non-periodic in v
    for k in srf.knots_u.iter().chain(&srf.knots_v) {
        fields.push(number(*k));
    }
    // The weights and then the control points, each list with the `i` index
    // running fastest (the order the spec defines), so `W(i,j)` precedes
    // `W(i+1,j)`.
    let weight = |i: usize, j: usize| {
        srf.weights
            .get(i)
            .and_then(|row| row.get(j))
            .copied()
            .unwrap_or(1.0)
    };
    for j in 0..=k2 {
        for i in 0..=k1 {
            fields.push(number(weight(i, j)));
        }
    }
    for j in 0..=k2 {
        for i in 0..=k1 {
            let p = srf.control[i][j];
            for v in [p.x, p.y, p.z] {
                fields.push(number(v));
            }
        }
    }
    // The parameter range is the surface's own domain, taken from the knot
    // vectors as they were written (which are normalised to 0..1 here).
    let u0 = srf
        .knots_u
        .get(srf.degree_u.min(nu - 1))
        .copied()
        .unwrap_or(0.0);
    let u1 = srf.knots_u.get(nu).copied().unwrap_or(1.0);
    let v0 = srf
        .knots_v
        .get(srf.degree_v.min(nv - 1))
        .copied()
        .unwrap_or(0.0);
    let v1 = srf.knots_v.get(nv).copied().unwrap_or(1.0);
    for value in [u0, u1, v0, v1] {
        fields.push(number(value));
    }
    let mut text = String::from("128,");
    text.push_str(&fields.join(","));
    text.push(';');
    text
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_interpolating_spline_passes_through_every_point() {
        // A parabola in the x-z plane, sampled unevenly, as a station curve
        // would be.
        let points: Vec<Vec3> = (0..12)
            .map(|i| {
                let x = i as f64 * 0.7;
                Vec3::new(x, 0.0, 0.3 * x * x - 0.2 * x)
            })
            .collect();
        let curve = fit_curve(&points, 3).expect("fit");
        assert_eq!(curve.degree, 3);
        assert_eq!(curve.control.len(), points.len());
        assert_eq!(curve.knots.len(), points.len() + 4);
        assert!(!curve.is_rational());
        // Chord-length parameters, exactly as the fit used them.
        let mut total = 0.0;
        let mut params = vec![0.0];
        for i in 1..points.len() {
            total += points[i].dist(points[i - 1]);
            params.push(total);
        }
        for (p, u) in points.iter().zip(&params) {
            let got = curve.eval(u / total);
            assert!(
                got.dist(*p) < 1e-9,
                "curve misses {p:?} by {}",
                got.dist(*p)
            );
        }
        // Degenerate input is refused, not panicked on.
        assert!(fit_curve(&[Vec3::ZERO; 4], 3).is_none());
        assert!(fit_curve(&[Vec3::ZERO], 3).is_none());
    }

    #[test]
    fn the_file_has_eighty_column_records_and_matching_counts() {
        let points: Vec<Vec3> = (0..20)
            .map(|i| Vec3::new(i as f64, 0.5 * (i as f64).sin(), 0.0))
            .collect();
        let curve = fit_curve(&points, 3).unwrap();
        let text = write_iges(&IgesFile {
            name: "hull.igs",
            curves: vec![("STATION1", &curve)],
            units: 6,
        });
        for line in text.lines() {
            assert_eq!(line.chars().count(), 80, "not 80 columns: {line:?}");
            let letter = line.chars().nth(72).unwrap();
            assert!("SG DPT".contains(letter), "bad section letter {letter:?}");
            let seq: usize = line[73..].trim().parse().unwrap();
            assert!(seq > 0);
        }
        // The terminate record counts every section.
        let last = text.lines().last().unwrap();
        assert_eq!(last.chars().nth(72), Some('T'));
        let counts = |letter: char| {
            text.lines()
                .filter(|l| l.chars().nth(72) == Some(letter))
                .count()
        };
        assert!(counts('S') == 1 && counts('G') >= 1 && counts('D') == 2);
        assert!(counts('P') >= 4, "20 points need several parameter lines");
        let terminate: usize = last[73..].trim().parse().unwrap();
        assert_eq!(
            terminate,
            counts('S') + counts('G') + counts('D') + counts('P') + 1
        );

        // The directory entry points at the first parameter line and says how
        // many there are.
        let d: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('D'))
            .collect();
        // Nine eight-character fields per record: the parameter pointer is the
        // second field of the first, the parameter count the fourth of the
        // second.
        let first_pd: usize = d[0][8..16].trim().parse().unwrap();
        let pd_count: usize = d[1][24..32].trim().parse().unwrap();
        let p_lines: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('P'))
            .collect();
        assert_eq!(first_pd, 1);
        assert_eq!(pd_count, p_lines.len());
        // Entity type 126 is declared on both directory records.
        assert_eq!(&d[0][0..8].trim(), &"126");
        assert_eq!(&d[1][0..8].trim(), &"126");
        // The parameter data starts with the entity type and ends with ';'.
        assert!(p_lines[0].starts_with("126,"), "{:?}", &p_lines[0][..24]);
        assert!(
            p_lines.last().unwrap()[..64].trim_end().ends_with(';'),
            "the parameter data must be terminated: {:?}",
            p_lines.last().unwrap()
        );
    }

    /// Join the G section the way a reader does — data columns 1-72 only,
    /// consecutive records concatenated — and split on commas.
    fn global_tokens(text: &str) -> Vec<String> {
        let mut joined = String::new();
        for line in text.lines().filter(|l| l.chars().nth(72) == Some('G')) {
            joined.push_str(line[..72].trim_end());
        }
        joined.split(',').map(|t| t.trim().to_owned()).collect()
    }

    fn one_station() -> BsplineCurve {
        let points: Vec<Vec3> = (0..20)
            .map(|i| Vec3::new(i as f64, 0.5 * (i as f64).sin(), 0.0))
            .collect();
        fit_curve(&points, 3).unwrap()
    }

    #[test]
    fn a_record_break_never_falls_inside_a_number() {
        // Readers concatenate the 72 data columns of consecutive G records
        // before splitting on commas. A break placed between the digits of a
        // number therefore glues two fields into one: "32,38,6" + "308,15"
        // reads back as "32,38,6308,15" and every later field shifts by one,
        // silently moving the unit code. The splitter must only break at a
        // top-level comma.
        let curve = one_station();
        let text = write_iges(&IgesFile {
            name: "hull.igs",
            curves: vec![("STATION1", &curve)],
            units: 6,
        });
        let tokens = global_tokens(&text);
        // The seven fixed integers after the preprocessor version.
        let tail = &tokens[tokens.len() - 19..];
        let joined = tail.join(",");
        assert!(
            joined.contains("32,38,6,308,15"),
            "integer fields must survive the record break intact: {joined:?}"
        );
        assert!(
            !joined.contains("6308"),
            "fields were glued together: {joined:?}"
        );
        // The unit code and its name are distinct fields, and the code is
        // the one that was asked for.
        assert!(
            tokens.iter().any(|t| t == "6"),
            "unit code missing from {tokens:?}"
        );
        assert!(
            tokens.iter().any(|t| t == "1HM"),
            "unit name missing from {tokens:?}"
        );
    }

    #[test]
    fn every_curve_carries_its_weight_list() {
        // Open Cascade loads a 126 whose weight list is absent without raising
        // an error and then transfers nothing. Entity 126 must always carry
        // K+1 weights, even when they are all one.
        let curve = one_station();
        assert!(!curve.is_rational(), "the fit produces a polynomial curve");
        let params = curve_parameters(&curve);
        let tokens: Vec<&str> = params.trim_end_matches(';').split(',').collect();
        let k: usize = tokens[1].parse().unwrap();
        let m: usize = tokens[2].parse().unwrap();
        let nk = k + m + 2;
        // 1 type + 6 K/M/PROP fields + knots + weights + poles + V0/V1.
        assert_eq!(
            tokens.len(),
            1 + 6 + nk + (k + 1) + 3 * (k + 1) + 2,
            "parameter count must include the weight list"
        );
        let weights = &tokens[7 + nk..7 + nk + k + 1];
        assert!(
            weights.iter().all(|w| *w == "1"),
            "unit weights expected for a polynomial curve, got {weights:?}"
        );
        // PROP3 stays 1 (polynomial) while the weights are present, matching
        // the reference writer's convention.
        assert_eq!(tokens[5], "1", "PROP3 should declare a polynomial curve");
    }

    #[test]
    fn parameter_sequence_numbers_are_zero_filled_and_point_at_their_entity() {
        let curve = one_station();
        let text = write_iges(&IgesFile {
            name: "hull.igs",
            curves: vec![("STATION1", &curve)],
            units: 6,
        });
        let p_lines: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('P'))
            .collect();
        for (i, line) in p_lines.iter().enumerate() {
            // Columns 74-80: seven digits, zero filled.
            assert_eq!(&line[73..], &format!("{:07}", i + 1), "bad sequence field");
            // Columns 65-72: the owning directory entry, " " + seven digits.
            assert_eq!(&line[64..72], &format!(" {:07}", 1), "bad pointer field");
        }
    }

    /// A curved 6 × 5 point grid, the shape a lofted set of station curves has:
    /// six stations along x, five girth points up a section.
    fn curved_grid() -> Vec<Vec<Vec3>> {
        (0..6)
            .map(|i| {
                let x = i as f64 * 1.5;
                (0..5)
                    .map(|j| {
                        let t = j as f64 / 4.0;
                        Vec3::new(x, 0.8 * t + 0.05 * x, 0.6 + 1.2 * t + 0.2 * (0.4 * x).sin())
                    })
                    .collect()
            })
            .collect()
    }

    #[test]
    fn an_interpolating_surface_passes_through_every_grid_point() {
        let grid = curved_grid();
        let srf = fit_surface(&grid, 3, 3).expect("fit");
        assert_eq!(srf.degree_u, 3);
        assert_eq!(srf.degree_v, 3);
        assert_eq!(srf.control.len(), 6);
        assert!(srf.control.iter().all(|row| row.len() == 5));
        assert_eq!(srf.knots_u.len(), 6 + 4);
        assert_eq!(srf.knots_v.len(), 5 + 4);
        assert_eq!(srf.weights, vec![vec![1.0; 5]; 6]);
        assert!(!srf.is_rational());

        // The parameters the fit is documented to use: averaged chord lengths,
        // recomputed here so the test would fail if the surface were evaluated
        // anywhere but at its own interpolation points.
        let params_u = averaged_params(6, 5, |j, i| grid[i][j]).expect("u parameters");
        let params_v = averaged_params(5, 6, |i, j| grid[i][j]).expect("v parameters");
        assert_eq!(params_u.first(), Some(&0.0));
        assert_eq!(params_u.last(), Some(&1.0));
        assert!(
            params_u.windows(2).all(|w| w[0] < w[1]),
            "u parameters must advance"
        );
        assert!(
            params_v.windows(2).all(|w| w[0] < w[1]),
            "v parameters must advance"
        );
        for (i, row) in grid.iter().enumerate() {
            for (j, point) in row.iter().enumerate() {
                let got = srf.eval(params_u[i], params_v[j]);
                assert!(
                    got.dist(*point) < 1e-9,
                    "surface misses {point:?} by {}",
                    got.dist(*point)
                );
            }
        }

        // Degenerate grids are refused rather than panicked on: a single row, a
        // ragged grid, and one whose rows all coincide.
        assert!(fit_surface(&grid[..1], 3, 3).is_none());
        assert!(fit_surface(&[grid[0][..2].to_vec(), grid[1][..3].to_vec()], 3, 3).is_none());
        let flat = vec![vec![Vec3::ZERO; 4]; 3];
        assert!(fit_surface(&flat, 3, 3).is_none());
    }

    #[test]
    fn entity_128_surface_file_has_eighty_column_records_and_matching_counts() {
        let grid = curved_grid();
        let srf = fit_surface(&grid, 3, 3).unwrap();
        let text = write_iges_surfaces("hull.igs", &[("STBD", &srf)], 6);

        // Every record is 80 columns: 72 of data, the section letter and a
        // seven-digit sequence number.
        for line in text.lines() {
            assert_eq!(line.chars().count(), 80, "not 80 columns: {line:?}");
            let letter = line.chars().nth(72).unwrap();
            assert!("SGDPT".contains(letter), "bad section letter {letter:?}");
            assert!(line[73..].trim().parse::<usize>().unwrap() > 0);
        }

        // The directory entries declare entity 128 and point at the parameter
        // data they describe.
        let d: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('D'))
            .collect();
        assert_eq!(d.len(), 2, "one surface is two directory records");
        assert_eq!(&d[0][0..8].trim(), &"128");
        assert_eq!(&d[1][0..8].trim(), &"128");
        let first_pd: usize = d[0][8..16].trim().parse().unwrap();
        let pd_count: usize = d[1][24..32].trim().parse().unwrap();
        let p_lines: Vec<&str> = text
            .lines()
            .filter(|l| l.chars().nth(72) == Some('P'))
            .collect();
        assert_eq!(first_pd, 1);
        assert_eq!(pd_count, p_lines.len());
        assert!(p_lines[0].starts_with("128,"), "{:?}", &p_lines[0][..24]);
        assert!(
            p_lines.last().unwrap()[..64].trim_end().ends_with(';'),
            "the parameter data must be terminated: {:?}",
            p_lines.last().unwrap()
        );

        // Parameter count: the entity type, nine flags and counts, both knot
        // vectors, one weight and three coordinates per pole, and the four
        // parameter range values.
        let mut joined = String::new();
        for line in &p_lines {
            joined.push_str(line[..64].trim_end());
        }
        let tokens: Vec<&str> = joined.trim_end_matches(';').split(',').collect();
        let (nku, nkv) = (srf.knots_u.len(), srf.knots_v.len());
        let n = srf.control.len() * srf.control[0].len();
        assert_eq!(tokens.len(), 1 + 9 + nku + nkv + n + 3 * n + 4);
        assert_eq!(tokens[0], "128");
        assert_eq!(tokens[1], "5", "K1 is the last pole index in u");
        assert_eq!(tokens[2], "4", "K2 is the last pole index in v");
        assert_eq!(tokens[3], "3", "M1 is the degree in u");
        assert_eq!(tokens[4], "3", "M2 is the degree in v");
        assert_eq!(tokens[7], "1", "PROP3 declares a polynomial surface");
        // Every weight, then every pole, with the i index running fastest.
        let weights = &tokens[10 + nku + nkv..10 + nku + nkv + n];
        assert!(
            weights.iter().all(|w| *w == "1"),
            "unit weights expected: {weights:?}"
        );
        let coords = &tokens[10 + nku + nkv + n..];
        assert_eq!(
            coords.len(),
            3 * n + 4,
            "3 coordinates per pole plus the range"
        );
        // The first pole written is the pole at (i = 0, j = 0).
        assert_eq!(coords[0], number(srf.control[0][0].x));
        assert_eq!(coords[1], number(srf.control[0][0].y));
        assert_eq!(coords[2], number(srf.control[0][0].z));
        assert_eq!(coords[3], number(srf.control[1][0].x), "i must run fastest");
        let range = &coords[3 * n..];
        assert_eq!(range.len(), 4);
        assert_eq!(range[0], "0", "the u domain starts at 0");
        assert_eq!(range[1], "1", "the u domain ends at 1");
        assert_eq!(range[2], "0", "the v domain starts at 0");
        assert_eq!(range[3], "1", "the v domain ends at 1");
    }
}
