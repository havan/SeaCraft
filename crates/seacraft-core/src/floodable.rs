//! Floodable length curves by trial waterlines.
//!
//! The floodable length at a point is the length of a compartment, with that point as its centre, that can be
//! flooded without immersing any part of the margin line when the ship has no list; the margin line is at least
//! 76 mm below the bulkhead deck at side (Rawson & Tupper, *Basic Ship Theory*, Vol. 1, 5th ed., pp. 155-156;
//! Gillmer & Johnson, *Introduction to Naval Architecture*, 1982, pp. 180-182). Permissible length is the
//! floodable length times a factor of subdivision; a compartment passes when its length does not exceed the
//! permissible length at its centre (the triangle check of Rawson & Tupper, with base angle tan⁻¹ 2).
//!
//! The method is Gillmer & Johnson's. Each trial waterline is tangent to the margin line. The hull
//! floats it with the flooded compartment's water counted as lost buoyancy, so the lost volume is
//! `v = V_W − ∇` and its centroid follows from the moment balance `V_W · x_B − ∇ · LCG = v · x_c`
//! (Gillmer & Johnson Eq. 10-1 and 10-2, there by added weight, which gives the same waterline). The
//! compartment is the full-breadth slice of the hull under that waterline holding `v / μ` with its centroid at
//! `x_c`, found from the sectional areas under the waterline (Bonjean curves in Gillmer & Johnson). Compartments are
//! full breadth and the ship upright; the deck edge is the moulded sheer, with no deck thickness.

use crate::damage::{hull_deck_edge_vertices, resolve_particulars, DamageCase, Particulars};
use crate::hydro::{chain_paths, plane_segments, section_area, waterplane_for, Hull, HydroOptions};
use crate::i18n::{Key, Msg};
use crate::math::{Plane, Vec3};
use crate::mesh::Surface;
use serde::{Deserialize, Serialize};
use std::fmt::Write as _;

/// The margin line's depth below the deck edge (m): 76 mm, the SOLAS 1960 figure.
pub const DEFAULT_MARGIN_M: f64 = 0.076;
/// Permeabilities drawn by default, one curve each.
pub const DEFAULT_PERMEABILITIES: [f64; 3] = [0.60, 0.85, 0.95];
/// Sectional-area stations per trial waterline by default.
pub const DEFAULT_STATIONS: usize = 201;
/// Most trial waterlines one computation runs.
const MAX_WATERLINES: usize = 400;

/// What to compute: permeabilities, margin line, and the optional bulkhead check.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct FloodableOptions {
    /// One curve per permeability, each in (0, 1].
    pub permeabilities: Vec<f64>,
    /// Margin line depth below the deck edge (m).
    pub margin_m: f64,
    /// Factor of subdivision in (0, 1]: permissible length = F × floodable length.
    pub factor: Option<f64>,
    /// Watertight bulkhead positions (model x, m), strictly increasing, inside the hull.
    pub bulkheads: Vec<f64>,
    /// Sectional-area stations per trial waterline (at least 11).
    pub stations: usize,
}

impl Default for FloodableOptions {
    fn default() -> Self {
        Self {
            permeabilities: DEFAULT_PERMEABILITIES.to_vec(),
            margin_m: DEFAULT_MARGIN_M,
            factor: None,
            bulkheads: Vec::new(),
            stations: DEFAULT_STATIONS,
        }
    }
}

/// One point of a floodable length curve: the compartment `[x_aft, x_fwd]` that floods the hull to the trial
/// waterline at `draft` and `trim_m`.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct FloodablePoint {
    /// Centre of the compartment (m, model x).
    pub x_mid: f64,
    /// Floodable length (m).
    pub length: f64,
    pub x_aft: f64,
    pub x_fwd: f64,
    /// Trim of the trial waterline (m, + bow-up).
    pub trim_m: f64,
    /// Draft of the trial waterline amidships above the keel, as [`waterplane_for`] takes it (m).
    pub draft: f64,
}

/// A compartment between neighbouring bulkheads (or a bulkhead and a hull end), judged at its centre.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CompartmentCheck {
    pub x_aft: f64,
    pub x_fwd: f64,
    pub length: f64,
    /// Floodable length at the compartment's centre, if the curve reaches it.
    pub floodable: Option<f64>,
    /// Permissible length at the centre (factor × floodable).
    pub permissible: Option<f64>,
    /// `None` when the curve does not reach the centre: not judged, never passed.
    pub passed: Option<bool>,
    pub reason: Option<Msg>,
}

/// The floodable length curve for one permeability.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FloodableCurve {
    pub permeability: f64,
    /// Sorted by `x_mid`.
    pub points: Vec<FloodablePoint>,
    pub checks: Vec<CompartmentCheck>,
}

/// Floodable length curves of a hull at one loading.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct FloodableResult {
    pub displacement_t: f64,
    pub lcg: f64,
    /// t/m³.
    pub water_density: f64,
    pub margin_m: f64,
    /// The hull's ends (m, model x): no compartment reaches past them.
    pub hull_x_aft: f64,
    pub hull_x_fwd: f64,
    pub factor: Option<f64>,
    /// Margin line points `(x, z)`, sorted by x.
    pub margin_line: Vec<[f64; 2]>,
    pub curves: Vec<FloodableCurve>,
    pub warnings: Vec<Msg>,
    /// The hull's centreline profile `(x, z)` polylines, for the plot; not part of the JSON.
    #[serde(skip)]
    pub profile: Vec<Vec<[f64; 2]>>,
}

/// Floodable length curves of `surface` at the loading `case` resolves to (its compartments are not used).
pub fn evaluate(
    surface: &Surface,
    hopts: &HydroOptions,
    case: &DamageCase,
    opts: &FloodableOptions,
) -> Result<FloodableResult, String> {
    let hull = Hull::new(surface, hopts.subdivision);
    let p = resolve_particulars(surface, hopts, &hull, case);
    compute(&hull, &p, opts)
}

/// Floodable length curves of a refined hull at the loading `p`.
pub fn compute(
    hull: &Hull,
    p: &Particulars,
    opts: &FloodableOptions,
) -> Result<FloodableResult, String> {
    compute_with_deck_edge(hull, p, opts, &hull_deck_edge_vertices(hull))
}

fn compute_with_deck_edge(
    hull: &Hull,
    p: &Particulars,
    opts: &FloodableOptions,
    deck_edge: &[Vec3],
) -> Result<FloodableResult, String> {
    validate(opts)?;
    validate_particulars(p)?;
    if deck_edge.is_empty() {
        return Err("the hull has no deck edge to draw a margin line under".into());
    }
    let mut margin: Vec<Vec3> = deck_edge
        .iter()
        .map(|q| Vec3::new(q.x, q.y, q.z - opts.margin_m))
        .collect();
    margin.sort_by(|a, b| a.x.total_cmp(&b.x));
    // The ends of the hull's own panels: `verts` also holds points no panel uses.
    let (x_aft, x_fwd) =
        hull.tris
            .iter()
            .flatten()
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), &i| {
                let x = hull.verts[i].x;
                (lo.min(x), hi.max(x))
            });
    if x_aft >= x_fwd {
        return Err("the hull has no panels to float".into());
    }
    validate_bulkheads(&opts.bulkheads, x_aft, x_fwd)?;
    let grid = Grid::new(x_aft, x_fwd, opts.stations);
    let closed = hull.closed_tris();
    let eval = |trim: f64| -> Trial {
        let (draft, wp) = tangent_waterline(hull, &margin, trim);
        let (v_w, b) = hull.buoyancy_in(&wp);
        let lost = v_w - p.target_volume;
        let mut trial = Trial {
            trim,
            points: vec![None; opts.permeabilities.len()],
        };
        if lost <= 1e-9 * p.target_volume.max(1.0) {
            return trial;
        }
        let x_c = (v_w * b.x - p.target_volume * p.lcg) / lost;
        if !x_c.is_finite() {
            return trial;
        }
        let areas = crate::par::par_map(&grid.stations, |&x| {
            section_area(&hull.verts, &closed, x, &wp)
        });
        let cum = grid.cumulative(&areas);
        for (slot, &mu) in trial.points.iter_mut().zip(&opts.permeabilities) {
            *slot = cum
                .compartment(lost / mu, x_c)
                .map(|(a, b)| FloodablePoint {
                    x_mid: 0.5 * (a + b),
                    length: b - a,
                    x_aft: a,
                    x_fwd: b,
                    trim_m: trim,
                    draft,
                });
        }
        trial
    };

    let even = eval(0.0);
    if even.points.iter().all(Option::is_none) {
        let (_, wp) = tangent_waterline(hull, &margin, 0.0);
        if hull.buoyancy_in(&wp).0 <= p.target_volume {
            return Err(format!(
                "a displacement of {:.3} t already sinks the hull to its margin line at even keel",
                p.displacement_t
            ));
        }
    }
    let trials = sweep(hull, &margin, even, &eval, x_fwd - x_aft);

    let mut warnings = Vec::new();
    let curves = opts
        .permeabilities
        .iter()
        .enumerate()
        .map(|(i, &mu)| {
            let mut points: Vec<FloodablePoint> =
                trials.iter().filter_map(|t| t.points[i]).collect();
            points.sort_by(|a, b| a.x_mid.total_cmp(&b.x_mid));
            if points.is_empty() {
                warnings.push(Msg::new(Key::WarnFloodableNoCurve).arg("mu", format!("{mu:.2}")));
            }
            let mut curve = FloodableCurve {
                permeability: mu,
                points,
                checks: Vec::new(),
            };
            if !opts.bulkheads.is_empty() {
                curve.checks =
                    check_compartments(&curve, &opts.bulkheads, x_aft, x_fwd, opts.factor);
            }
            curve
        })
        .collect();
    Ok(FloodableResult {
        displacement_t: p.displacement_t,
        lcg: p.lcg,
        water_density: p.density,
        margin_m: opts.margin_m,
        hull_x_aft: x_aft,
        hull_x_fwd: x_fwd,
        factor: opts.factor,
        margin_line: margin.iter().map(|q| [q.x, q.z]).collect(),
        curves,
        warnings,
        profile: centreline_profile(hull),
    })
}

/// The hull cut at y = 0, as `(x, z)` polylines.
fn centreline_profile(hull: &Hull) -> Vec<Vec<[f64; 2]>> {
    let plane = Plane::new(Vec3::new(0.0, 1.0, 0.0), 0.0);
    let segs = plane_segments(&hull.verts, &hull.closed_tris(), &plane);
    let tol = 1e-7 * (hull.model_max.x - hull.model_min.x).max(1.0);
    chain_paths(&segs, tol)
        .into_iter()
        .map(|(pts, closed)| {
            let mut line: Vec<[f64; 2]> = pts.iter().map(|p| [p.x, p.z]).collect();
            if closed {
                if let Some(&first) = line.first() {
                    line.push(first);
                }
            }
            line
        })
        .collect()
}

fn validate(opts: &FloodableOptions) -> Result<(), String> {
    if opts.permeabilities.is_empty() {
        return Err("give at least one permeability".into());
    }
    for &mu in &opts.permeabilities {
        if !(mu > 0.0 && mu <= 1.0) {
            return Err(format!("permeability {mu} is outside (0, 1]"));
        }
    }
    if !(opts.margin_m.is_finite() && opts.margin_m >= 0.0) {
        return Err(format!(
            "margin line depth {} m is not a length",
            opts.margin_m
        ));
    }
    if let Some(f) = opts.factor {
        if !(f > 0.0 && f <= 1.0) {
            return Err(format!("factor of subdivision {f} is outside (0, 1]"));
        }
    }
    if opts.stations < 11 {
        return Err(format!(
            "{} stations are too few; use at least 11",
            opts.stations
        ));
    }
    Ok(())
}

/// The loading must be a ship: a positive displacement and water density, the volume they float, and an LCG.
fn validate_particulars(p: &Particulars) -> Result<(), String> {
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !positive(p.displacement_t) || !positive(p.target_volume) {
        return Err(format!(
            "a displacement of {} t is not a ship to flood",
            p.displacement_t
        ));
    }
    if !positive(p.density) {
        return Err(format!("water density {} t/m³ is not positive", p.density));
    }
    if !p.lcg.is_finite() {
        return Err(format!("LCG {} m is not a position", p.lcg));
    }
    Ok(())
}

fn validate_bulkheads(bulkheads: &[f64], x_aft: f64, x_fwd: f64) -> Result<(), String> {
    for &x in bulkheads {
        if !(x > x_aft && x < x_fwd) {
            return Err(format!(
                "bulkhead at x = {x} m is not inside the hull ({x_aft:.3} to {x_fwd:.3} m)"
            ));
        }
    }
    if bulkheads.windows(2).any(|w| w[1] <= w[0]) {
        return Err("bulkhead positions must increase from aft to forward, each once".into());
    }
    Ok(())
}

/// The compartments the bulkheads make, from the aft end of the hull to its forward end, each judged against
/// the permissible length at its centre: the triangle on it, as high as it is long, stays under the
/// permissible curve exactly when its length does not exceed the permissible length there.
fn check_compartments(
    curve: &FloodableCurve,
    bulkheads: &[f64],
    x_aft: f64,
    x_fwd: f64,
    factor: Option<f64>,
) -> Vec<CompartmentCheck> {
    let f = factor.unwrap_or(1.0);
    let mut bounds = Vec::with_capacity(bulkheads.len() + 2);
    bounds.push(x_aft);
    bounds.extend_from_slice(bulkheads);
    bounds.push(x_fwd);
    bounds
        .windows(2)
        .map(|w| {
            let (a, b) = (w[0], w[1]);
            let floodable = floodable_at(curve, 0.5 * (a + b));
            let permissible = floodable.map(|l| f * l);
            CompartmentCheck {
                x_aft: a,
                x_fwd: b,
                length: b - a,
                floodable,
                permissible,
                passed: permissible.map(|p| b - a <= p),
                reason: floodable
                    .is_none()
                    .then(|| Msg::new(Key::FloodableCheckOutsideCurve)),
            }
        })
        .collect()
}

/// One trial waterline and the compartment it gives for each permeability.
#[derive(Debug, Clone)]
struct Trial {
    trim: f64,
    points: Vec<Option<FloodablePoint>>,
}

/// Trims outward from even keel in both directions, past trims where no permeability gives a point yet, until
/// three in a row give none after one has; then trims between neighbours whose points lie far apart along the
/// hull or where a curve ends. Each direction has its own share of the waterlines, so a long run one way does
/// not starve the other.
fn sweep(
    hull: &Hull,
    margin: &[Vec3],
    even: Trial,
    eval: &dyn Fn(f64) -> Trial,
    span: f64,
) -> Vec<Trial> {
    let low = margin.iter().map(|q| q.z).fold(f64::INFINITY, f64::min);
    let step = 0.05 * (low - hull.keel).max(1e-3);
    let reach = 2.0 * (hull.model_max.z - hull.keel).max(1e-3);
    let even_hit = even.points.iter().any(Option::is_some);
    let mut trials = vec![even];
    let share = MAX_WATERLINES / 4;
    for sign in [1.0, -1.0] {
        let mut seen = even_hit;
        let mut misses = 0;
        for k in 1..=share {
            let trim = sign * step * k as f64;
            if trim.abs() > reach {
                break;
            }
            let t = eval(trim);
            let any = t.points.iter().any(Option::is_some);
            trials.push(t);
            seen |= any;
            misses = if any { 0 } else { misses + 1 };
            if seen && misses >= 3 {
                break;
            }
        }
    }
    trials.sort_by(|a, b| a.trim.total_cmp(&b.trim));

    let gap = span / 50.0;
    let min_dt = (step / 64.0).max(1e-4);
    for _ in 0..16 {
        let mut inserts = Vec::new();
        for w in trials.windows(2) {
            if w[1].trim - w[0].trim <= min_dt {
                continue;
            }
            let wide = w[0].points.iter().zip(&w[1].points).any(|pair| match pair {
                (Some(a), Some(b)) => (a.x_mid - b.x_mid).abs() > gap,
                (Some(_), None) | (None, Some(_)) => true,
                (None, None) => false,
            });
            if wide {
                inserts.push(0.5 * (w[0].trim + w[1].trim));
            }
        }
        if inserts.is_empty() {
            break;
        }
        for trim in inserts {
            if trials.len() >= MAX_WATERLINES {
                break;
            }
            trials.push(eval(trim));
        }
        trials.sort_by(|a, b| a.trim.total_cmp(&b.trim));
    }
    trials
}

/// Stations along the hull: strictly inside its ends, where a section cut is well defined, plus the ends.
struct Grid {
    /// The hull's aft end, the stations, the forward end.
    xs: Vec<f64>,
    stations: Vec<f64>,
}

impl Grid {
    fn new(x_aft: f64, x_fwd: f64, n: usize) -> Self {
        let inset = 1e-6 * (x_fwd - x_aft);
        let (a, b) = (x_aft + inset, x_fwd - inset);
        let stations: Vec<f64> = (0..n)
            .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
            .collect();
        let mut xs = Vec::with_capacity(n + 2);
        xs.push(x_aft);
        xs.extend(&stations);
        xs.push(x_fwd);
        Self { xs, stations }
    }

    /// Cumulative volume and moment from the aft end, by trapezoids over the sectional areas (the end
    /// sections taken equal to their neighbours).
    fn cumulative(&self, areas: &[f64]) -> Cumulative {
        let mut a = Vec::with_capacity(areas.len() + 2);
        a.push(areas[0]);
        a.extend(areas);
        a.push(areas[areas.len() - 1]);
        let mut q = vec![0.0; self.xs.len()];
        let mut m = vec![0.0; self.xs.len()];
        for k in 1..self.xs.len() {
            let (x0, x1) = (self.xs[k - 1], self.xs[k]);
            let dq = 0.5 * (a[k - 1] + a[k]) * (x1 - x0);
            // ∫ x A dx over a linear A, exactly.
            let dm = (x1 - x0) / 6.0 * (a[k - 1] * (2.0 * x0 + x1) + a[k] * (x0 + 2.0 * x1));
            q[k] = q[k - 1] + dq;
            m[k] = m[k - 1] + dm;
        }
        Cumulative {
            xs: self.xs.clone(),
            q,
            m,
        }
    }
}

/// Volume Q(x) and moment M(x) of the hull under a waterline from its aft end to x.
struct Cumulative {
    xs: Vec<f64>,
    q: Vec<f64>,
    m: Vec<f64>,
}

impl Cumulative {
    /// The first x where Q reaches `u`, and M there. Within an interval Q and M are taken to grow in
    /// proportion (the interval's volume spread evenly over it), which keeps both exact at the stations.
    fn at_volume(&self, u: f64) -> (f64, f64) {
        let n = self.q.len();
        let k = self.q.partition_point(|&q| q < u).clamp(1, n - 1);
        let (q0, q1) = (self.q[k - 1], self.q[k]);
        let (x0, x1) = (self.xs[k - 1], self.xs[k]);
        if q1 - q0 <= 0.0 {
            return (x0, self.m[k - 1]);
        }
        let f = ((u - q0) / (q1 - q0)).clamp(0.0, 1.0);
        (
            x0 + f * (x1 - x0),
            self.m[k - 1] + f * (self.m[k] - self.m[k - 1]),
        )
    }

    /// The compartment `[a, b]` holding `need` with its centroid at `x_c`, or `None` past the hull's ends.
    fn compartment(&self, need: f64, x_c: f64) -> Option<(f64, f64)> {
        let total = *self.q.last()?;
        if need.is_nan() || need <= 0.0 || need > total {
            return None;
        }
        let centroid = |u: f64| -> (f64, f64, f64) {
            let (a, ma) = self.at_volume(u);
            let (b, mb) = self.at_volume(u + need);
            (a, b, (mb - ma) / need)
        };
        let (mut lo, mut hi) = (0.0, total - need);
        let (_, _, g_lo) = centroid(lo);
        let (_, _, g_hi) = centroid(hi);
        let tol = 1e-9 * (self.xs[self.xs.len() - 1] - self.xs[0]).max(1.0);
        if x_c < g_lo - tol || x_c > g_hi + tol {
            return None;
        }
        for _ in 0..100 {
            let mid = 0.5 * (lo + hi);
            if centroid(mid).2 < x_c {
                lo = mid;
            } else {
                hi = mid;
            }
            if hi - lo <= 1e-13 * total {
                break;
            }
        }
        let (a, b, _) = centroid(0.5 * (lo + hi));
        Some((a, b))
    }
}

/// The trial waterline of trim `trim` tangent to the margin points: its draft and plane.
///
/// [`waterplane_for`] puts the water at `keel + draft + trim/2` at x = 0 and `keel + draft − trim/2` at
/// x = L, so its height is `c − (trim / L)·x` with `c = keel + draft + trim/2`; touching the margin line from
/// below means `c = min_i (z_i + (trim / L)·x_i)`.
fn tangent_waterline(hull: &Hull, margin: &[Vec3], trim: f64) -> (f64, Plane) {
    let l = hull.length.max(1e-6);
    let c = margin
        .iter()
        .map(|q| q.z + trim / l * q.x)
        .fold(f64::INFINITY, f64::min);
    let draft = c - 0.5 * trim - hull.keel;
    (
        draft,
        waterplane_for(hull.keel, hull.length, draft, trim, 0.0),
    )
}

/// Floodable length at `x`, interpolated along the curve; `None` outside it.
pub fn floodable_at(curve: &FloodableCurve, x: f64) -> Option<f64> {
    let pts = &curve.points;
    let first = pts.first()?;
    let last = pts.last()?;
    if x < first.x_mid || x > last.x_mid {
        return None;
    }
    let k = pts.partition_point(|p| p.x_mid < x);
    if k == 0 {
        return Some(first.length);
    }
    let (p0, p1) = (pts[k - 1], pts[k.min(pts.len() - 1)]);
    if p1.x_mid - p0.x_mid <= 0.0 {
        return Some(p0.length.min(p1.length));
    }
    let f = (x - p0.x_mid) / (p1.x_mid - p0.x_mid);
    Some(p0.length + f * (p1.length - p0.length))
}

/// The curves as a standalone SVG chart in English: see [`svg_tr`].
pub fn svg(res: &FloodableResult, title: &str) -> String {
    svg_tr(res, title, crate::i18n::Tr::new(crate::i18n::Locale::En))
}

/// The curves as a standalone SVG chart: floodable length over x at one scale for both axes, so the terminals
/// and the bulkhead triangles have their true 2:1 slopes; one curve per permeability, the permissible length of
/// the first curve dashed when a factor is set, a triangle on each compartment of the first curve's check, and
/// below them the hull profile (centreline), the deck edge at side, the margin line and the bulkheads on their
/// own vertical scale.
pub fn svg_tr(res: &FloodableResult, title: &str, tr: crate::i18n::Tr) -> String {
    use crate::curves::{chart_frame, esc};
    let (w, h) = (960.0, 600.0);
    let (ml, mr, mt) = (64.0, 110.0, 46.0);
    let band = 110.0; // profile band height (px)
    let gap = 44.0;
    let plot_w = w - ml - mr;
    let plot_h = h - mt - gap - band - 40.0;
    let span = (res.hull_x_fwd - res.hull_x_aft).max(1e-6);
    let longest = res
        .curves
        .iter()
        .flat_map(|c| c.points.iter().map(|p| p.length))
        .fold(0.0, f64::max)
        .max(1e-6);
    // One scale for both axes (px per metre).
    let k = (plot_w / span).min(plot_h / (1.08 * longest));
    let x0 = ml + 0.5 * (plot_w - k * span);
    let base = mt + plot_h;
    let sx = |x: f64| x0 + (x - res.hull_x_aft) * k;
    let sy = |l: f64| base - l * k;
    let top_len = plot_h / k;

    let mut out = String::with_capacity(16 * 1024);
    chart_frame(&mut out, title, w, h);
    out.push_str("<style>\n");
    for (i, colour) in ["#1f5fa8", "#b5532a", "#2e7d4f", "#7b3fa0"]
        .iter()
        .enumerate()
    {
        let _ = writeln!(out, ".c{i}{{stroke:{colour}}}");
    }
    out.push_str(".permissible{fill:none;stroke:#1f5fa8;stroke-width:1.2;stroke-dasharray:6 4}\n");
    out.push_str(".terminal{fill:none;stroke:#777;stroke-width:0.8;stroke-dasharray:2 3}\n");
    out.push_str(".triangle{fill-opacity:0.12;stroke-width:0.9}\n");
    out.push_str(".pass{fill:#2e7d4f;stroke:#2e7d4f}\n.fail{fill:#c0392b;stroke:#c0392b}\n");
    out.push_str(".unjudged{fill:#999;stroke:#999;stroke-dasharray:3 3}\n");
    out.push_str(".hull{fill:none;stroke:#111;stroke-width:1}\n");
    out.push_str(".deck{fill:none;stroke:#555;stroke-width:0.8}\n");
    out.push_str(".margin{fill:none;stroke:#c0392b;stroke-width:1;stroke-dasharray:5 3}\n");
    out.push_str(".bulkhead{fill:none;stroke:#555;stroke-width:1}\n");
    out.push_str("</style>\n");
    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{ml:.0}\" y=\"26\">{}</text>",
        esc(title)
    );

    // Axes and grid.
    let _ = writeln!(
        out,
        "<rect class=\"axis\" x=\"{:.1}\" y=\"{mt:.1}\" width=\"{:.1}\" height=\"{plot_h:.1}\"/>",
        sx(res.hull_x_aft),
        k * span
    );
    let step = nice_step(span.max(top_len) / 8.0);
    let mut x = (res.hull_x_aft / step).ceil() * step;
    while x <= res.hull_x_fwd + 1e-9 {
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{0:.1}\" y1=\"{mt:.1}\" x2=\"{0:.1}\" y2=\"{base:.1}\"/>\n<text class=\"label\" x=\"{0:.1}\" y=\"{1:.1}\" text-anchor=\"middle\">{2}</text>",
            sx(x),
            base + 14.0,
            fmt_tick(x)
        );
        x += step;
    }
    let mut l = step;
    while l <= top_len + 1e-9 {
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{0:.1}\" y1=\"{1:.1}\" x2=\"{2:.1}\" y2=\"{1:.1}\"/>\n<text class=\"label\" x=\"{3:.1}\" y=\"{4:.1}\" text-anchor=\"end\">{5}</text>",
            sx(res.hull_x_aft),
            sy(l),
            sx(res.hull_x_fwd),
            sx(res.hull_x_aft) - 4.0,
            sy(l) + 4.0,
            fmt_tick(l)
        );
        l += step;
    }
    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" transform=\"rotate(-90 {0:.1} {1:.1})\" text-anchor=\"middle\">{}</text>",
        sx(res.hull_x_aft) - 40.0,
        mt + 0.5 * plot_h,
        esc(tr.s(Key::FloodableSvgLengthAxis))
    );

    // Terminals: l = 2 (x − x_aft) and l = 2 (x_fwd − x), up to the top of the plot.
    let rise = 0.5 * top_len;
    let _ = writeln!(
        out,
        "<line class=\"terminal\" x1=\"{:.1}\" y1=\"{base:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        sx(res.hull_x_aft),
        sx(res.hull_x_aft + rise),
        sy(top_len)
    );
    let _ = writeln!(
        out,
        "<line class=\"terminal\" x1=\"{:.1}\" y1=\"{base:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
        sx(res.hull_x_fwd),
        sx(res.hull_x_fwd - rise),
        sy(top_len)
    );

    // Bulkhead triangles on the first curve's check.
    if let Some(first) = res.curves.first() {
        for c in &first.checks {
            let class = match c.passed {
                Some(true) => "pass",
                Some(false) => "fail",
                None => "unjudged",
            };
            let mid = 0.5 * (c.x_aft + c.x_fwd);
            let _ = writeln!(
                out,
                "<polygon class=\"triangle {class}\" points=\"{:.1},{base:.1} {:.1},{:.1} {:.1},{base:.1}\"/>",
                sx(c.x_aft),
                sx(mid),
                sy(c.length.min(top_len)),
                sx(c.x_fwd)
            );
        }
    }

    // Curves, and the first curve's permissible length.
    let poly = |pts: &mut dyn Iterator<Item = (f64, f64)>| -> String {
        pts.map(|(x, l)| format!("{:.1},{:.1}", sx(x), sy(l.min(top_len))))
            .collect::<Vec<_>>()
            .join(" ")
    };
    // Labels at each curve's forward end, kept 13 px apart.
    let mut labels: Vec<(f64, f64)> = Vec::new();
    for (i, c) in res.curves.iter().enumerate() {
        if c.points.is_empty() {
            continue;
        }
        let _ = writeln!(
            out,
            "<polyline class=\"curve c{}\" points=\"{}\"/>",
            i % 4,
            poly(&mut c.points.iter().map(|p| (p.x_mid, p.length)))
        );
        let last = c.points[c.points.len() - 1];
        labels.push((sy(last.length.min(top_len)) + 4.0, c.permeability));
    }
    labels.sort_by(|a, b| a.0.total_cmp(&b.0));
    for i in 1..labels.len() {
        labels[i].0 = labels[i].0.max(labels[i - 1].0 + 13.0);
    }
    for (y, mu) in labels {
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{y:.1}\">μ = {mu:.2}</text>",
            sx(res.hull_x_fwd) + 6.0
        );
    }
    if let (Some(f), Some(first)) = (res.factor, res.curves.first()) {
        if !first.points.is_empty() {
            let _ = writeln!(
                out,
                "<polyline class=\"permissible\" points=\"{}\"/>",
                poly(&mut first.points.iter().map(|p| (p.x_mid, f * p.length)))
            );
            let _ = writeln!(
                out,
                "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
                sx(res.hull_x_aft) + 4.0,
                mt + 14.0,
                esc(&tr.fmt(
                    Key::FloodableSvgPermissible,
                    &[
                        ("factor", &format!("{f:.2}")),
                        ("mu", &format!("{:.2}", first.permeability))
                    ]
                ))
            );
        }
    }
    if res.curves.iter().all(|c| c.points.is_empty()) {
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            ml,
            mt + 30.0,
            esc(tr.s(Key::FloodableSvgEmpty))
        );
    }

    // Profile band: hull, margin line, bulkheads.
    let band_top = base + gap;
    let (z_lo, z_hi) = res
        .profile
        .iter()
        .flatten()
        .chain(res.margin_line.iter())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p[1]), hi.max(p[1]))
        });
    if z_lo.is_finite() && z_hi > z_lo {
        let kz = (band / (z_hi - z_lo)).min(k);
        let sz = |z: f64| band_top + band - (z - z_lo) * kz;
        for line in &res.profile {
            let pts: Vec<String> = line
                .iter()
                .map(|p| format!("{:.1},{:.1}", sx(p[0]), sz(p[1])))
                .collect();
            let _ = writeln!(
                out,
                "<polyline class=\"hull\" points=\"{}\"/>",
                pts.join(" ")
            );
        }
        let margin = margin_profile(&res.margin_line);
        let deck: Vec<String> = margin
            .iter()
            .map(|p| format!("{:.1},{:.1}", sx(p[0]), sz(p[1] + res.margin_m)))
            .collect();
        let _ = writeln!(
            out,
            "<polyline class=\"deck\" points=\"{}\"/>",
            deck.join(" ")
        );
        let pts: Vec<String> = margin
            .iter()
            .map(|p| format!("{:.1},{:.1}", sx(p[0]), sz(p[1])))
            .collect();
        let _ = writeln!(
            out,
            "<polyline class=\"margin\" points=\"{}\"/>",
            pts.join(" ")
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\">{}</text>",
            sx(res.hull_x_fwd) + 6.0,
            sz(margin.last().map_or(z_hi, |p| p[1])) + 4.0,
            esc(tr.s(Key::FloodableSvgMarginLine))
        );
        if let Some(first) = res.curves.first() {
            for c in first.checks.iter().skip(1) {
                let _ = writeln!(
                    out,
                    "<line class=\"bulkhead\" x1=\"{0:.1}\" y1=\"{1:.1}\" x2=\"{0:.1}\" y2=\"{2:.1}\"/>",
                    sx(c.x_aft),
                    sz(z_lo),
                    sz(z_hi)
                );
            }
        }
    }
    out.push_str("</svg>\n");
    out
}

/// The lowest margin point at each x, so a deck edge read on both sides draws one line.
fn margin_profile(margin: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let mut out: Vec<[f64; 2]> = Vec::new();
    for &p in margin {
        match out.last_mut() {
            Some(last) if (last[0] - p[0]).abs() < 1e-9 => last[1] = last[1].min(p[1]),
            _ => out.push(p),
        }
    }
    out
}

fn nice_step(raw: f64) -> f64 {
    let mag = 10f64.powf(raw.max(1e-9).log10().floor());
    let r = raw / mag;
    mag * if r <= 1.0 {
        1.0
    } else if r <= 2.0 {
        2.0
    } else if r <= 5.0 {
        5.0
    } else {
        10.0
    }
}

fn fmt_tick(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::damage::make_box_barge;
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    const L: f64 = 10.0;
    const B: f64 = 4.0;
    const D: f64 = 2.0;
    const M: f64 = DEFAULT_MARGIN_M;

    fn barge_hull() -> Hull {
        Hull::new(
            &make_box_barge(L, B, D),
            SubdivisionOptions {
                level: 0,
                mode: SubdivisionMode::Loop,
            },
        )
    }

    /// The barge floating `draft` at even keel in water of 1 t/m³, LCG amidships.
    fn barge_particulars(draft: f64) -> Particulars {
        let displacement_t = L * B * draft;
        Particulars {
            displacement_t,
            kg: 1.0,
            lcg: 0.5 * L,
            tcg: 0.0,
            density: 1.0,
            target_volume: displacement_t,
        }
    }

    fn margin_points(hull: &Hull) -> Vec<Vec3> {
        hull_deck_edge_vertices(hull)
            .into_iter()
            .map(|p| Vec3::new(p.x, p.y, p.z - M))
            .collect()
    }

    /// ∫_a^b x^k h(x) dx for the barge's waterline height above the keel h(x) = c − s·x, clamped to [0, D]:
    /// exact, split where h meets 0 or D.
    fn int_h(a: f64, b: f64, c: f64, s: f64, k: i32) -> f64 {
        let mut cuts = vec![a, b];
        if s.abs() > 1e-15 {
            for level in [0.0, D] {
                let x = (c - level) / s;
                if x > a && x < b {
                    cuts.push(x);
                }
            }
        }
        cuts.sort_by(f64::total_cmp);
        let mut total = 0.0;
        for w in cuts.windows(2) {
            let (x0, x1) = (w[0], w[1]);
            let h_mid = c - s * 0.5 * (x0 + x1);
            if h_mid <= 0.0 {
                continue;
            }
            if h_mid >= D {
                // ∫ x^k D dx
                total += D * (x1.powi(k + 1) - x0.powi(k + 1)) / f64::from(k + 1);
            } else {
                // ∫ x^k (c − s x) dx
                total += c * (x1.powi(k + 1) - x0.powi(k + 1)) / f64::from(k + 1)
                    - s * (x1.powi(k + 2) - x0.powi(k + 2)) / f64::from(k + 2);
            }
        }
        total
    }

    #[test]
    fn tangent_waterline_touches_the_margin_line_and_stays_below_it() {
        let hull = barge_hull();
        let margin = margin_points(&hull);
        for trim in [-0.5, 0.0, 0.5] {
            let (_, wp) = tangent_waterline(&hull, &margin, trim);
            let lowest = margin
                .iter()
                .map(|p| wp.side(*p))
                .fold(f64::INFINITY, f64::min);
            assert!(lowest.abs() < 1e-9, "trim {trim}: lowest side {lowest}");
        }
    }

    /// Even keel, LCG amidships: the lost volume μ·l·B·z_m equals L·B·z_m − L·B·T, so
    /// l = L (z_m − T) / (μ z_m), centred amidships (derived here from the lost-buoyancy balance).
    #[test]
    fn box_barge_even_keel_point_matches_closed_form() {
        let hull = barge_hull();
        let t = 0.80;
        let res = compute(&hull, &barge_particulars(t), &FloodableOptions::default()).unwrap();
        let z_m = D - M;
        assert_eq!(res.curves.len(), 3);
        for curve in &res.curves {
            let mu = curve.permeability;
            let p = curve
                .points
                .iter()
                .find(|p| p.trim_m.abs() < 1e-9)
                .unwrap_or_else(|| panic!("μ {mu}: no even-keel point"));
            let expected = L * (z_m - t) / (mu * z_m);
            assert!(
                (p.x_mid - 0.5 * L).abs() < 1e-3,
                "μ {mu}: x_mid {}",
                p.x_mid
            );
            assert!(
                (p.length / expected - 1.0).abs() < 1e-3,
                "μ {mu}: length {} vs {expected}",
                p.length
            );
        }
    }

    /// Every point, trimmed or not, satisfies the lost-buoyancy balance computed with exact integrals of the
    /// barge's linear waterline: μ B ∫_a^b h = B ∫_0^L h − ∇ and μ B ∫_a^b x h = B ∫_0^L x h − ∇ · LCG.
    fn assert_balance(t: f64) {
        let hull = barge_hull();
        let res = compute(&hull, &barge_particulars(t), &FloodableOptions::default()).unwrap();
        let vol = L * B * t;
        let mut n = 0;
        for curve in &res.curves {
            let mu = curve.permeability;
            for p in &curve.points {
                // h(x) = draft + trim/2 − trim·x/L above the keel (z = 0).
                let (c, s) = (p.draft + 0.5 * p.trim_m, p.trim_m / L);
                let v_w = B * int_h(0.0, L, c, s, 0);
                let m_w = B * int_h(0.0, L, c, s, 1);
                let v_c = mu * B * int_h(p.x_aft, p.x_fwd, c, s, 0);
                let m_c = mu * B * int_h(p.x_aft, p.x_fwd, c, s, 1);
                assert!(
                    (v_c / (v_w - vol) - 1.0).abs() < 2e-3,
                    "μ {mu} trim {}: volume {v_c} vs {}",
                    p.trim_m,
                    v_w - vol
                );
                assert!(
                    (m_c / (m_w - vol * 0.5 * L) - 1.0).abs() < 2e-3,
                    "μ {mu} trim {}: moment {m_c} vs {}",
                    p.trim_m,
                    m_w - vol * 0.5 * L
                );
                n += 1;
            }
        }
        assert!(n > 20, "only {n} points");
    }

    #[test]
    fn box_barge_trimmed_points_satisfy_the_analytic_balance() {
        assert_balance(0.80);
    }

    #[test]
    fn curve_covers_the_hull_and_stops_inside_its_ends() {
        let hull = barge_hull();
        let res = compute(
            &hull,
            &barge_particulars(0.80),
            &FloodableOptions::default(),
        )
        .unwrap();
        assert_eq!((res.hull_x_aft, res.hull_x_fwd), (0.0, L));
        for curve in &res.curves {
            assert!(curve.points.len() > 5, "μ {}", curve.permeability);
            for p in &curve.points {
                assert!(p.x_aft >= -1e-9 && p.x_fwd <= L + 1e-9 && p.x_aft < p.x_fwd);
                assert!((p.x_mid - 0.5 * (p.x_aft + p.x_fwd)).abs() < 1e-12);
                assert!((p.length - (p.x_fwd - p.x_aft)).abs() < 1e-12);
            }
            for w in curve.points.windows(2) {
                assert!(w[0].x_mid <= w[1].x_mid);
                assert!(w[1].x_mid - w[0].x_mid <= L / 50.0 + 1e-9, "gap {:?}", w);
            }
        }
    }

    /// A light barge at large trims lifts one end of its bottom clear of the water: sections there are zero,
    /// and the balance must still hold with finite, positive lengths.
    #[test]
    fn a_dried_end_keeps_the_solve_finite() {
        let hull = barge_hull();
        let res = compute(
            &hull,
            &barge_particulars(0.20),
            &FloodableOptions::default(),
        )
        .unwrap();
        let dried = res.curves.iter().flat_map(|c| &c.points).any(|p| {
            let (c, s) = (p.draft + 0.5 * p.trim_m, p.trim_m / L);
            c < 0.0 || c - s * L < 0.0
        });
        assert!(dried, "no waterline dried an end");
        for p in res.curves.iter().flat_map(|c| &c.points) {
            assert!(p.length.is_finite() && p.length > 0.0 && p.x_mid.is_finite());
        }
        assert_balance(0.20);
    }

    #[test]
    fn heavier_than_the_margin_is_an_error() {
        let hull = barge_hull();
        let err = compute(
            &hull,
            &barge_particulars(1.95),
            &FloodableOptions::default(),
        )
        .unwrap_err();
        assert!(err.contains("margin line"), "{err}");
    }

    #[test]
    fn bad_permeability_is_an_error() {
        let hull = barge_hull();
        for mu in [0.0, 1.2, f64::NAN] {
            let opts = FloodableOptions {
                permeabilities: vec![0.85, mu],
                ..FloodableOptions::default()
            };
            let err = compute(&hull, &barge_particulars(0.8), &opts).unwrap_err();
            assert!(
                err.contains("permeability") && err.contains(&format!("{mu}")),
                "{err}"
            );
        }
    }

    #[test]
    fn a_hull_without_a_deck_edge_is_an_error() {
        let hull = barge_hull();
        let err = compute_with_deck_edge(
            &hull,
            &barge_particulars(0.8),
            &FloodableOptions::default(),
            &[],
        )
        .unwrap_err();
        assert!(err.contains("deck edge"), "{err}");
    }

    /// A loading that is not a ship (no displacement, no water, an LCG that is not a number) is an error, never
    /// a curve computed from NaN.
    #[test]
    fn unphysical_particulars_are_errors() {
        let hull = barge_hull();
        let good = barge_particulars(0.8);
        let cases = [
            Particulars {
                displacement_t: 0.0,
                target_volume: 0.0,
                ..good
            },
            Particulars {
                displacement_t: -5.0,
                target_volume: -5.0,
                ..good
            },
            Particulars {
                displacement_t: f64::NAN,
                target_volume: f64::NAN,
                ..good
            },
            Particulars {
                density: 0.0,
                ..good
            },
            Particulars {
                lcg: f64::NAN,
                ..good
            },
            Particulars {
                lcg: f64::INFINITY,
                ..good
            },
        ];
        for p in cases {
            let res = compute(&hull, &p, &FloodableOptions::default());
            assert!(res.is_err(), "{p:?} gave {:?}", res.map(|r| r.curves));
        }
    }

    /// Hull ends come from the hull's own panels: a vertex the panels do not use (a layer that is not part of
    /// the hydrostatic hull) does not move them.
    #[test]
    fn hull_ends_ignore_vertices_outside_the_panels() {
        let mut hull = barge_hull();
        hull.verts.push(Vec3::new(-3.0, 0.0, 1.0));
        hull.verts.push(Vec3::new(L + 3.0, 0.0, 1.0));
        let res = compute(&hull, &barge_particulars(0.8), &FloodableOptions::default()).unwrap();
        assert_eq!((res.hull_x_aft, res.hull_x_fwd), (0.0, L));
    }

    /// A permeability's curve does not depend on which other permeabilities are asked with it. Light (8 t)
    /// the demo hull at even keel needs more than its whole volume for μ 0.60, so that curve starts only
    /// several trims out from even keel.
    #[test]
    fn a_curve_does_not_depend_on_the_other_permeabilities() {
        let surface = crate::demo::demo_hull(false);
        let hopts = HydroOptions::from_surface(
            &surface,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        let case = DamageCase {
            displacement_t: Some(8.0),
            kg: Some(0.0),
            ..DamageCase::default()
        };
        let run = |permeabilities: Vec<f64>| {
            let opts = FloodableOptions {
                permeabilities,
                ..FloodableOptions::default()
            };
            evaluate(&surface, &hopts, &case, &opts).unwrap()
        };
        let alone = run(vec![0.60]);
        let together = run(vec![0.60, 0.85, 0.95]);
        let (a, t) = (&alone.curves[0], &together.curves[0]);
        assert!(t.points.len() > 10, "together: {} points", t.points.len());
        assert!(a.points.len() > 10, "alone: {} points", a.points.len());
        let x = 0.5 * (t.points[0].x_mid + t.points[t.points.len() - 1].x_mid);
        let (la, lt) = (floodable_at(a, x).unwrap(), floodable_at(t, x).unwrap());
        assert!(
            (la / lt - 1.0).abs() < 1e-2,
            "at x {x}: {la} alone, {lt} together"
        );
    }

    fn check_opts(bulkheads: Vec<f64>, factor: Option<f64>) -> FloodableOptions {
        FloodableOptions {
            permeabilities: vec![0.85],
            factor,
            bulkheads,
            ..FloodableOptions::default()
        }
    }

    #[test]
    fn bulkheads_split_the_hull_and_are_judged_at_their_midpoints() {
        let hull = barge_hull();
        let opts = check_opts(vec![2.0, 4.0, 6.0, 8.0], Some(0.5));
        let res = compute(&hull, &barge_particulars(0.8), &opts).unwrap();
        let curve = &res.curves[0];
        assert_eq!(res.factor, Some(0.5));
        assert_eq!(curve.checks.len(), 5);
        let bounds = [0.0, 2.0, 4.0, 6.0, 8.0, L];
        let mut judged = 0;
        for (c, w) in curve.checks.iter().zip(bounds.windows(2)) {
            assert_eq!((c.x_aft, c.x_fwd), (w[0], w[1]));
            assert!((c.length - (w[1] - w[0])).abs() < 1e-12);
            if let Some(f) = c.floodable {
                assert_eq!(Some(f), floodable_at(curve, 0.5 * (w[0] + w[1])));
                let permissible = c.permissible.unwrap();
                assert!((permissible - 0.5 * f).abs() < 1e-12);
                assert_eq!(c.passed, Some(c.length <= permissible));
                judged += 1;
            }
        }
        assert!(judged >= 3, "{judged} judged");
        // Amidships the floodable length is 6.87 m (μ 0.85): permissible 3.44 m, so a 2 m compartment passes.
        assert_eq!(curve.checks[2].passed, Some(true));
    }

    #[test]
    fn a_compartment_outside_the_curve_is_not_judged() {
        let hull = barge_hull();
        // The μ 0.85 curve starts near x = 1.1 m: a 1 m end compartment (centre 0.5 m) lies outside it.
        let opts = check_opts(vec![1.0, 9.0], None);
        let res = compute(&hull, &barge_particulars(0.8), &opts).unwrap();
        let first = &res.curves[0].checks[0];
        assert!(res.curves[0].points[0].x_mid > 0.5);
        assert_eq!(first.passed, None);
        assert_eq!(first.floodable, None);
        assert!(first.reason.is_some());
        // Without a factor, permissible length is the floodable length itself (F = 1).
        let mid = &res.curves[0].checks[1];
        assert_eq!(mid.permissible, mid.floodable);
    }

    #[test]
    fn unsorted_or_outside_bulkheads_are_errors() {
        let hull = barge_hull();
        for bulkheads in [
            vec![4.0, 2.0],
            vec![3.0, 3.0],
            vec![-1.0],
            vec![11.0],
            vec![f64::NAN],
        ] {
            let err = compute(
                &hull,
                &barge_particulars(0.8),
                &check_opts(bulkheads.clone(), None),
            )
            .unwrap_err();
            assert!(err.contains("bulkhead"), "{bulkheads:?}: {err}");
        }
    }

    #[test]
    fn factor_outside_zero_one_is_an_error() {
        let hull = barge_hull();
        for f in [0.0, 1.5, f64::NAN] {
            let err = compute(
                &hull,
                &barge_particulars(0.8),
                &check_opts(vec![5.0], Some(f)),
            )
            .unwrap_err();
            assert!(err.contains("factor of subdivision"), "{f}: {err}");
        }
    }

    fn demo_hydro_options() -> (Surface, HydroOptions) {
        let surface = crate::demo::demo_hull(false);
        let hopts = HydroOptions::from_surface(
            &surface,
            SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
        );
        (surface, hopts)
    }

    /// Two SeaCraft methods agree (not an external source): flooding the compartment a curve point gives, as an
    /// ordinary damage case (lost buoyancy, compartment clipped to the hull), floats the demo hull upright with
    /// its deck edge 76 mm clear, i.e. the margin line awash, at the point's trim. Tolerances: the curve
    /// integrates 201 sectional areas, the damage case a clipped solid of the refined hull; 5 mm of freeboard
    /// and 20 mm of trim cover that difference on a 12 m hull.
    #[test]
    fn curve_points_flood_the_demo_hull_to_its_margin_line() {
        use crate::damage::{Compartment, CompartmentSpec};
        let (surface, hopts) = demo_hydro_options();
        let opts = FloodableOptions {
            permeabilities: vec![0.85],
            ..FloodableOptions::default()
        };
        let res = evaluate(&surface, &hopts, &DamageCase::default(), &opts).unwrap();
        let pts = &res.curves[0].points;
        assert!(pts.len() > 10, "{} points", pts.len());
        let hull = Hull::new(&surface, hopts.subdivision);
        for p in [pts[0], pts[pts.len() / 2], pts[pts.len() - 1]] {
            let case = DamageCase {
                name: "floodable point".into(),
                compartments: vec![CompartmentSpec::Inline(Compartment {
                    name: "flooded".into(),
                    x_min: p.x_aft,
                    x_max: p.x_fwd,
                    y_min: -1e3,
                    y_max: 1e3,
                    z_min: hull.keel - 1.0,
                    z_max: hull.model_max.z + 1.0,
                    permeability: 0.85,
                    clip_to_hull: true,
                })],
                displacement_t: Some(res.displacement_t),
                kg: Some(0.75 * (hull.model_max.z - hull.keel)),
                lcg: Some(res.lcg),
                water_density: Some(res.water_density),
                ..DamageCase::default()
            };
            let d = crate::damage::evaluate(&surface, &hopts, &case).unwrap();
            assert!(
                d.heel_deg.abs() < 0.05,
                "x {}: heel {}",
                p.x_mid,
                d.heel_deg
            );
            let fb = d.freeboard_m.unwrap();
            assert!(
                (fb - DEFAULT_MARGIN_M).abs() < 0.005,
                "x {}: freeboard {fb} m",
                p.x_mid
            );
            assert!(
                (d.trim_m - p.trim_m).abs() < 0.02,
                "x {}: trim {} vs {}",
                p.x_mid,
                d.trim_m,
                p.trim_m
            );
        }
    }

    /// Not published: recorded from SeaCraft on 2026-11-26 (demo hull, level 3, design draft, μ 0.85), to
    /// catch a change, not to show the numbers are right (`docs/oracles.md`). The shape agrees with Gillmer &
    /// Johnson's remark that the curve is "usually greatest amidships and least near the quarter length": here
    /// least at the aft end, where the transom terminates it.
    #[test]
    fn demo_hull_curve_recorded() {
        let (surface, hopts) = demo_hydro_options();
        let opts = FloodableOptions {
            permeabilities: vec![0.85],
            ..FloodableOptions::default()
        };
        let res = evaluate(&surface, &hopts, &DamageCase::default(), &opts).unwrap();
        let close = |got: f64, want: f64| (got / want - 1.0).abs() < 5e-3;
        assert!(close(res.displacement_t, 12.1), "Δ {}", res.displacement_t);
        assert!(close(res.lcg, 4.30), "LCG {}", res.lcg);
        let curve = &res.curves[0];
        assert!(curve.points.len() > 40, "{} points", curve.points.len());
        let min = curve
            .points
            .iter()
            .min_by(|a, b| a.length.total_cmp(&b.length))
            .unwrap();
        assert!(close(min.length, 1.39), "least {} m", min.length);
        assert!(close(min.x_mid, 0.701), "least at x {} m", min.x_mid);
        let mid = floodable_at(curve, 6.0).unwrap();
        assert!(close(mid, 3.37), "at x = 6 m: {mid} m");
    }

    #[test]
    fn svg_has_a_curve_per_permeability_and_a_triangle_per_compartment() {
        let hull = barge_hull();
        let opts = FloodableOptions {
            factor: Some(0.5),
            bulkheads: vec![2.0, 4.0, 6.0, 8.0],
            ..FloodableOptions::default()
        };
        let res = compute(&hull, &barge_particulars(0.8), &opts).unwrap();
        assert!(!res.profile.is_empty());
        let en = svg(&res, "Barge <test>");
        assert_eq!(en.matches("class=\"curve").count(), 3);
        assert_eq!(en.matches("class=\"triangle").count(), 5);
        assert_eq!(en.matches("class=\"permissible\"").count(), 1);
        assert_eq!(en.matches("class=\"terminal\"").count(), 2);
        assert!(en.contains("class=\"margin\""));
        assert!(en.contains("class=\"hull\""));
        assert!(en.contains("Barge &lt;test&gt;"));
        assert!(en.contains("Floodable length"));
        let tr = svg_tr(&res, "Duba", crate::i18n::Tr::new(crate::i18n::Locale::Tr));
        assert!(tr.contains("Su alabilir boy"), "{tr}");
        assert!(tr.contains("Sınır hattı"));
    }
}
