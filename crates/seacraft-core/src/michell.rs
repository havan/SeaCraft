//! Michell's thin-ship wave resistance theory (1898) for slender hulls and catamarans.
//!
//! Evaluates the Michell wave-making resistance integral over the centreplane
//! `y = f(x, z) >= 0` with `x` along the hull, `z <= 0` below the calm waterline
//! down to draft `T`:
//!
//! ```text
//! R_w = (4 rho g^2 / (pi U^2)) * \int_0^{pi/2} (P^2 + Q^2) sec^3(theta) d(theta)
//! P + i Q = \iint f_x(x, z) exp(k0 z sec^2(theta)) exp(i k0 x sec(theta)) dx dz
//! ```
//!
//! where `k0 = g / U^2` and `f_x = df/dx`.
//!
//! For catamarans with demihull separation `s` (centreline-to-centreline), the
//! integrand `(P^2 + Q^2)` is multiplied by the interference factor:
//!
//! ```text
//! 2 * (1 + cos(k0 s sec(theta) tan(theta)))
//! ```
//!
//! As `s -> infinity`, the oscillating cosine term averages to zero under integration,
//! yielding `2 * R_w(monohull)`.
//!
//! Wave resistance coefficient is non-dimensionalised on wetted surface `S`:
//!
//! ```text
//! Cw = R_w / (0.5 * rho * U^2 * S)
//! ```

use crate::hydro::{self, Hull, HydroOptions};
use crate::i18n::{Key, Msg};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::resistance::{self, Resistance, WaveBranch};
use crate::subdiv::SubdivisionOptions;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// A 2D regular tensor-product grid of half-breadths `y = f(x, z) >= 0`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MichellGrid {
    /// Longitudinal coordinates `x` in metres, sorted ascending.
    pub xs: Vec<f64>,
    /// Vertical coordinates `z` below the waterline in metres (`z <= 0`), sorted ascending from `-T` to `0`.
    pub zs: Vec<f64>,
    /// Half-breadths `f(x_i, z_j)` in metres, indexed as `half_breadths[i][j]`.
    pub half_breadths: Vec<Vec<f64>>,
    /// Waterline length `L` (m).
    pub length_wl: f64,
    /// Waterline beam `B` (m).
    pub beam_wl: f64,
    /// Design draft `T` (m).
    pub draft: f64,
    /// Hull wetted surface area `S` (m²), both sides.
    pub wetted_surface_m2: f64,
}

impl MichellGrid {
    /// Validate dimensions and grid consistency.
    pub fn validate(&self) -> Result<(), String> {
        if self.xs.len() < 3 {
            return Err("grid must have at least 3 stations".into());
        }
        if self.zs.len() < 3 {
            return Err("grid must have at least 3 waterlines".into());
        }
        if self.half_breadths.len() != self.xs.len() {
            return Err("half_breadths length must match xs length".into());
        }
        for (i, col) in self.half_breadths.iter().enumerate() {
            if col.len() != self.zs.len() {
                return Err(format!("half_breadths[{i}] length must match zs length"));
            }
        }
        if self.length_wl <= 0.0 || self.beam_wl <= 0.0 || self.draft <= 0.0 {
            return Err("principal dimensions must be positive".into());
        }
        if self.wetted_surface_m2 <= 0.0 {
            return Err("wetted surface must be positive".into());
        }
        Ok(())
    }

    /// Immersed area of the station at `xs[i]`, both sides (m²), by the trapezoid rule in `z`.
    pub fn section_area(&self, i: usize) -> f64 {
        let f = &self.half_breadths[i];
        let half: f64 = self
            .zs
            .windows(2)
            .zip(f.windows(2))
            .map(|(z, y)| 0.5 * (y[0] + y[1]) * (z[1] - z[0]))
            .sum();
        2.0 * half
    }

    /// Wetted transom area over the largest section area: the immersed area of the aftmost
    /// station (least `x`, since `x` points forward) divided by the largest immersed station area.
    /// Zero for a hull that closes at the stern, such as the Wigley hull; zero for an empty grid.
    pub fn transom_area_ratio(&self) -> f64 {
        let Some(aft) = (0..self.xs.len()).min_by(|&a, &b| self.xs[a].total_cmp(&self.xs[b]))
        else {
            return 0.0;
        };
        let largest = (0..self.xs.len())
            .map(|i| self.section_area(i))
            .fold(0.0, f64::max);
        if largest > 0.0 {
            self.section_area(aft) / largest
        } else {
            0.0
        }
    }
}

/// Options controlling the Michell integral evaluation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MichellOptions {
    /// Number of longitudinal stations for grid extraction (default 65).
    pub nx: usize,
    /// Number of vertical waterline levels for grid extraction (default 25).
    pub nz: usize,
    /// Number of wave angle quadrature points in `[0, pi/2)` (default 128).
    pub ntheta: usize,
    /// Demihull centreline separation `s` for catamarans in metres (None for monohull).
    pub catamaran_spacing: Option<f64>,
}

impl Default for MichellOptions {
    fn default() -> Self {
        Self {
            nx: 65,
            nz: 25,
            ntheta: 128,
            catamaran_spacing: None,
        }
    }
}

/// Result of a Michell wave resistance calculation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MichellResult {
    /// Wave-making resistance in Newtons.
    pub rw_n: f64,
    /// Wave resistance coefficient `Cw = R_w / (0.5 * rho * U^2 * S)`.
    pub cw: f64,
    /// Froude number on waterline length `Fn = U / sqrt(g * LWL)`.
    pub fn_: f64,
    /// Diagnostic note on convergence and configuration.
    pub converged_note: String,
}

/// Analytical Wigley parabolic hull grid:
/// `y = (B / 2) * (1 - (2x / L)^2) * (1 - (z / T)^2)`
/// for `x \in [-L/2, L/2]` and `z \in [-T, 0]`.
pub fn wigley_grid(length: f64, beam: f64, draft: f64, nx: usize, nz: usize) -> MichellGrid {
    let nx = nx.max(5);
    let nz = nz.max(5);
    let half_l = length * 0.5;
    let half_b = beam * 0.5;

    let xs: Vec<f64> = (0..nx)
        .map(|i| -half_l + length * (i as f64) / ((nx - 1) as f64))
        .collect();
    let zs: Vec<f64> = (0..nz)
        .map(|j| -draft + draft * (j as f64) / ((nz - 1) as f64))
        .collect();

    let mut half_breadths = vec![vec![0.0; nz]; nx];
    let mut wetted_surface = 0.0;

    for i in 0..nx {
        let x = xs[i];
        let term_x = (1.0 - (2.0 * x / length).powi(2)).max(0.0);
        for j in 0..nz {
            let z = zs[j];
            let term_z = (1.0 - (z / draft).powi(2)).max(0.0);
            half_breadths[i][j] = half_b * term_x * term_z;
        }
    }

    // Integrate wetted surface area S = 2 * \iint \sqrt{1 + f_x^2 + f_z^2} dx dz
    for i in 0..nx - 1 {
        let dx = xs[i + 1] - xs[i];
        let x_mid = 0.5 * (xs[i] + xs[i + 1]);
        for j in 0..nz - 1 {
            let dz = zs[j + 1] - zs[j];
            let z_mid = 0.5 * (zs[j] + zs[j + 1]);
            let fx = -2.0 * half_b * x_mid / (half_l * half_l) * (1.0 - (z_mid / draft).powi(2));
            let fz =
                -2.0 * half_b * (1.0 - (2.0 * x_mid / length).powi(2)) * z_mid / (draft * draft);
            let ds = (1.0 + fx * fx + fz * fz).sqrt() * dx * dz;
            wetted_surface += 2.0 * ds;
        }
    }

    MichellGrid {
        xs,
        zs,
        half_breadths,
        length_wl: length,
        beam_wl: beam,
        draft,
        wetted_surface_m2: wetted_surface,
    }
}

/// Helper building a `MichellGrid` from a refined mesh `Surface` at the given draft.
pub fn michell_grid_from_surface(
    surface: &Surface,
    draft: f64,
    nx: usize,
    nz: usize,
    subdiv: SubdivisionOptions,
) -> Option<MichellGrid> {
    if draft <= 0.0 {
        return None;
    }
    let hull = Hull::new(surface, subdiv);
    let mut opts = HydroOptions::from_surface(surface, subdiv);
    opts.draft = draft;
    let h = hull.hydrostatics(&opts);
    if h.volume_m3 <= 0.0 || h.lwl <= 0.0 || h.bwl <= 0.0 {
        return None;
    }

    let wp = hydro::waterplane_for(hull.keel, hull.length, draft, 0.0, 0.0);
    let segs = hydro::plane_segments(&hull.verts, &hull.tris, &wp);
    let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (a, b) in &segs {
        x_min = x_min.min(a.x).min(b.x);
        x_max = x_max.max(a.x).max(b.x);
    }
    if !x_min.is_finite() || !x_max.is_finite() || x_max <= x_min {
        return None;
    }
    let lwl = x_max - x_min;

    let nx = nx.max(9);
    let nz = nz.max(5);

    let xs: Vec<f64> = (0..nx)
        .map(|i| x_min + lwl * (i as f64) / ((nx - 1) as f64))
        .collect();
    let zs: Vec<f64> = (0..nz)
        .map(|j| -draft + draft * (j as f64) / ((nz - 1) as f64))
        .collect();

    let mut half_breadths = vec![vec![0.0; nz]; nx];

    for (i, &x) in xs.iter().enumerate() {
        // Clamp slightly inside ends to avoid degenerate boundary cuts.
        let cut_x = x.clamp(x_min + 0.0005 * lwl, x_max - 0.0005 * lwl);
        let (_, polys) = crate::curves::station_polygons(&hull, cut_x);
        for (j, &z) in zs.iter().enumerate() {
            // Model height above keel datum
            let z_model = hull.keel + draft + z;
            let y = half_breadth_at(&polys, z_model).unwrap_or(0.0).max(0.0);
            half_breadths[i][j] = y;
        }
    }

    Some(MichellGrid {
        xs,
        zs,
        half_breadths,
        length_wl: lwl,
        beam_wl: h.bwl,
        draft,
        wetted_surface_m2: h.wetted_surface_m2,
    })
}

/// Helper internal to sample half-breadth from section polygons (matching offsets.rs).
fn half_breadth_at(polys: &[Vec<Vec3>], z: f64) -> Option<f64> {
    let mut best: Option<f64> = None;
    for poly in polys {
        let n = poly.len();
        if n < 2 {
            continue;
        }
        for i in 0..n {
            let a = poly[i];
            let b = poly[(i + 1) % n];
            let (za, zb) = (a.z - z, b.z - z);
            if (za < 0.0 && zb >= 0.0) || (za >= 0.0 && zb < 0.0) {
                let t = za / (za - zb);
                let y = a.y + (b.y - a.y) * t;
                if best.is_none_or(|m| y > m) {
                    best = Some(y);
                }
            }
        }
    }
    best.map(|y| y.max(0.0)).filter(|y| y.is_finite())
}

/// Compute Michell wave resistance from a pre-built `MichellGrid`.
pub fn michell_wave_resistance_from_grid(
    grid: &MichellGrid,
    speed_m_s: f64,
    rho: f64,
    g: f64,
    opts: &MichellOptions,
) -> MichellResult {
    let u = speed_m_s.max(1e-6);
    let fn_ = u / (g * grid.length_wl).sqrt();
    let k0 = g / (u * u);

    let nx = grid.xs.len();
    let nz = grid.zs.len();
    let ntheta = opts.ntheta.max(32);

    // Wave angle integration: midpoint rule over theta in [0, pi / 2)
    let dtheta = (PI * 0.5) / (ntheta as f64);
    let mut integral = 0.0;
    let mut z_sums = Vec::with_capacity(nx.saturating_sub(1));

    for k in 0..ntheta {
        let theta = (k as f64 + 0.5) * dtheta;
        let cos_th = theta.cos();
        let sin_th = theta.sin();
        let sec_th = 1.0 / cos_th;
        let sec2_th = sec_th * sec_th;
        let sec3_th = sec2_th * sec_th;

        let kx = k0 * sec_th;
        let kz = k0 * sec2_th;

        // P + iQ = sum_i Z_i(theta) X(x_i, x_{i+1}; kx): see `column_z_integrals` and `x_integral`.
        column_z_integrals(grid, kz, &mut z_sums);
        let mut p_val = 0.0;
        let mut q_val = 0.0;
        for (i, z_sum) in z_sums.iter().enumerate() {
            let (ix_cos, ix_sin) = x_integral(grid.xs[i], grid.xs[i + 1], kx);
            p_val += z_sum * ix_cos;
            q_val += z_sum * ix_sin;
        }

        let mut amp2 = p_val * p_val + q_val * q_val;

        // Catamaran interference factor: 2 * (1 + cos(k0 * s * sec(theta) * tan(theta)))
        if let Some(s) = opts.catamaran_spacing {
            let tan_th = sin_th / cos_th;
            let arg = k0 * s * sec_th * tan_th;
            let interference = 2.0 * (1.0 + arg.cos());
            amp2 *= interference;
        }

        integral += amp2 * sec3_th * dtheta;
    }

    let factor = (4.0 * rho * g * g) / (PI * u * u);
    let rw_n = factor * integral;

    // Normalised wave resistance coefficient Cw = Rw / (0.5 * rho * U^2 * S),
    // with S of both demihulls for a catamaran (Rw is for both).
    let q_dyn = 0.5 * rho * u * u * grid.wetted_surface_m2 * hull_count(opts);
    let cw = if q_dyn > 1e-9 { rw_n / q_dyn } else { 0.0 };

    let converged_note = format!(
        "grid {}x{}, {} theta points{}",
        nx,
        nz,
        ntheta,
        if let Some(s) = opts.catamaran_spacing {
            format!(", catamaran spacing s={s:.2}m")
        } else {
            String::new()
        }
    );

    MichellResult {
        rw_n,
        cw,
        fn_,
        converged_note,
    }
}

/// X(a, b; k) = \int_a^b exp(i k x) dx as (re, im), in the form
/// `(b - a) sinc(k (b - a) / 2) exp(i k m)` with `m = (a + b) / 2`. The small-argument branch keeps
/// the sinc accurate as `k (b - a) -> 0`.
pub(crate) fn x_integral(a: f64, b: f64, kx: f64) -> (f64, f64) {
    let dx = b - a;
    let x_mid = 0.5 * (a + b);
    let half_arg = 0.5 * kx * dx;
    let sinc = if half_arg.abs() < 1e-4 {
        1.0 - half_arg * half_arg / 6.0
    } else {
        half_arg.sin() / half_arg
    };
    let ix_mag = dx * sinc;
    (ix_mag * (kx * x_mid).cos(), ix_mag * (kx * x_mid).sin())
}

/// Per-column depth integrals `Z_i = \int_{-T}^0 f_x,i(z) exp(kz z) dz` for the station intervals
/// `[x_i, x_{i+1}]` (one per interval, `nx - 1` in all), written to `out`. Over a panel
/// `[x_i, x_{i+1}] x [z_j, z_{j+1}]` the half-breadth is bilinear, so `f_x` is constant in `x` and
/// linear in `z`, and the `z` integral has a closed form.
pub(crate) fn column_z_integrals(grid: &MichellGrid, kz: f64, out: &mut Vec<f64>) {
    column_z_terms(grid, kz, out, |v| v);
}

/// [`column_z_integrals`] with `|f_x|` in place of `f_x`: an upper bound on `|Z_i|`, used to find
/// where a hull's spectrum has decayed.
pub(crate) fn column_z_envelope(grid: &MichellGrid, kz: f64, out: &mut Vec<f64>) {
    column_z_terms(grid, kz, out, f64::abs);
}

fn column_z_terms(grid: &MichellGrid, kz: f64, out: &mut Vec<f64>, slope: impl Fn(f64) -> f64) {
    let nx = grid.xs.len();
    let nz = grid.zs.len();
    out.clear();
    for i in 0..nx - 1 {
        let dx = grid.xs[i + 1] - grid.xs[i];
        let mut z_sum = 0.0;
        for j in 0..nz - 1 {
            let dz = grid.zs[j + 1] - grid.zs[j];
            let z_j = grid.zs[j];
            let z_j1 = grid.zs[j + 1];

            let fx0 = slope((grid.half_breadths[i + 1][j] - grid.half_breadths[i][j]) / dx);
            let fx1 = slope((grid.half_breadths[i + 1][j + 1] - grid.half_breadths[i][j + 1]) / dx);

            let alpha = kz * dz;
            let (j0, j1) = if alpha < 1e-4 {
                let j0 = 0.5 + alpha / 6.0 + alpha * alpha / 24.0;
                let j1 = 0.5 + alpha / 3.0 + alpha * alpha / 8.0;
                let exp_mid = (kz * z_j).exp();
                (dz * exp_mid * j0, dz * exp_mid * j1)
            } else {
                let exp_0 = (kz * z_j).exp();
                let exp_1 = (kz * z_j1).exp();
                let j0 = (exp_1 - (1.0 + alpha) * exp_0) / (kz * kz * dz);
                let j1 = ((alpha - 1.0) * exp_1 + exp_0) / (kz * kz * dz);
                (j0, j1)
            };

            z_sum += fx0 * j0 + fx1 * j1;
        }
        out.push(z_sum);
    }
}

/// Hulls the grid stands for: two demihulls when a catamaran spacing is set.
fn hull_count(opts: &MichellOptions) -> f64 {
    if opts.catamaran_spacing.is_some() {
        2.0
    } else {
        1.0
    }
}

/// Michell's wave resistance at each Froude number on the waterline length, in order: the wave
/// drag coefficient curve `C_w(Fn)`.
pub fn michell_cw_curve(
    grid: &MichellGrid,
    froudes: &[f64],
    rho: f64,
    g: f64,
    opts: &MichellOptions,
) -> Vec<MichellResult> {
    let root = (g * grid.length_wl).sqrt();
    crate::par::par_map(froudes, |&f| {
        michell_wave_resistance_from_grid(grid, f * root, rho, g, opts)
    })
}

/// Catamaran interference at a speed: Michell's R_w of two demihulls at centreline spacing
/// `spacing_m`, over twice that of one demihull alone. Below 1 the wave systems partly cancel
/// (favourable). `None` when one demihull makes no wave drag (R_w at most 1e-9 N).
pub fn interference(
    grid: &MichellGrid,
    speed_m_s: f64,
    rho: f64,
    g: f64,
    spacing_m: f64,
) -> Option<f64> {
    let mono =
        michell_wave_resistance_from_grid(grid, speed_m_s, rho, g, &MichellOptions::default());
    if mono.rw_n <= 1e-9 {
        return None;
    }
    let cat_opts = MichellOptions {
        catamaran_spacing: Some(spacing_m),
        ..MichellOptions::default()
    };
    let cat = michell_wave_resistance_from_grid(grid, speed_m_s, rho, g, &cat_opts);
    Some(cat.rw_n / (2.0 * mono.rw_n))
}

/// The interior local maxima and minima of a curve, as indices `(maxima, minima)`. A flat top or
/// bottom is reported once, at its first point, and a flat shelf on a slope is not an extremum.
pub fn local_extrema(ys: &[f64]) -> (Vec<usize>, Vec<usize>) {
    let (mut max, mut min) = (Vec::new(), Vec::new());
    let mut i = 1;
    while i + 1 < ys.len() {
        // The plateau ys[i..=j] of equal values.
        let mut j = i;
        while j + 1 < ys.len() && ys[j + 1] == ys[i] {
            j += 1;
        }
        if j + 1 >= ys.len() {
            break;
        }
        let (before, after) = (ys[i - 1], ys[j + 1]);
        if ys[i] > before && ys[i] > after {
            max.push(i);
        } else if ys[i] < before && ys[i] < after {
            min.push(i);
        }
        i = j + 1;
    }
    (max, min)
}

/// Compute a full still-water resistance curve across speeds (knots) combining
/// Michell wave resistance with ITTC-57 frictional resistance.
pub fn michell_resistance_curve(
    grid: &MichellGrid,
    form_factor: f64,
    viscosity: f64,
    rho: f64,
    g: f64,
    speeds_kn: &[f64],
    opts: &MichellOptions,
) -> Vec<Resistance> {
    speeds_kn
        .iter()
        .map(|&kn| {
            let v = kn * resistance::KNOT;
            let re = v * grid.length_wl / viscosity;
            let cf = if re > 1.0 {
                0.075 / (re.log10() - 2.0).powi(2)
            } else {
                0.0
            };
            let q = 0.5 * rho * v * v;
            // The grid is one demihull; a catamaran drags both through the water.
            let rf = q * grid.wetted_surface_m2 * hull_count(opts) * cf;
            let michell_res = michell_wave_resistance_from_grid(grid, v, rho, g, opts);
            let rw = michell_res.rw_n;
            let rt = rf * form_factor + rw;
            Resistance {
                speed_kn: kn,
                speed_m_s: v,
                fn_: michell_res.fn_,
                reynolds: re,
                cf,
                form_factor,
                rf_n: rf,
                rapp_n: 0.0,
                rw_n: rw,
                rb_n: 0.0,
                rtr_n: 0.0,
                ra_n: 0.0,
                viscous_coefficient: form_factor * cf,
                wave_branch: WaveBranch::Michell,
                rt_n: rt,
                pe_w: rt * v,
            }
        })
        .collect()
}

/// Unified entry point accepting either a `MichellGrid` or `(&Surface, draft)`.
pub enum MichellGeometry<'a> {
    Grid(&'a MichellGrid),
    Surface {
        surface: &'a Surface,
        draft: f64,
        subdiv: SubdivisionOptions,
    },
}

impl<'a> From<&'a MichellGrid> for MichellGeometry<'a> {
    fn from(grid: &'a MichellGrid) -> Self {
        Self::Grid(grid)
    }
}

impl<'a> From<(&'a Surface, f64)> for MichellGeometry<'a> {
    fn from((surface, draft): (&'a Surface, f64)) -> Self {
        Self::Surface {
            surface,
            draft,
            subdiv: SubdivisionOptions::default(),
        }
    }
}

impl<'a> From<(&'a Surface, f64, SubdivisionOptions)> for MichellGeometry<'a> {
    fn from((surface, draft, subdiv): (&'a Surface, f64, SubdivisionOptions)) -> Self {
        Self::Surface {
            surface,
            draft,
            subdiv,
        }
    }
}

/// Calculate Michell wave-making resistance from geometry (grid or surface+draft).
pub fn michell_wave_resistance<'a>(
    geom: impl Into<MichellGeometry<'a>>,
    speed_m_s: f64,
    rho: f64,
    g: f64,
    opts: &MichellOptions,
) -> Result<MichellResult, String> {
    match geom.into() {
        MichellGeometry::Grid(grid) => {
            grid.validate()?;
            Ok(michell_wave_resistance_from_grid(
                grid, speed_m_s, rho, g, opts,
            ))
        }
        MichellGeometry::Surface {
            surface,
            draft,
            subdiv,
        } => {
            let grid = michell_grid_from_surface(surface, draft, opts.nx, opts.nz, subdiv)
                .ok_or_else(|| {
                    "cannot extract Michell offsets grid from hull surface at given draft"
                        .to_string()
                })?;
            Ok(michell_wave_resistance_from_grid(
                &grid, speed_m_s, rho, g, opts,
            ))
        }
    }
}

/// Wetted transom area, as a fraction of the largest section area, above which a Michell result
/// is flagged. The project's own threshold, not a published limit: the example hulls that close at
/// the stern (Wigley, kayık, yacht, trawler, container ship) measure below 2 % on the default 65 × 25
/// grid, the demo hull and the runabout above 80 %.
pub const MICHELL_TRANSOM_AREA_LIMIT: f64 = 0.05;

/// Thin-ship validity warnings for a Michell curve on `grid`: L/B below 6, each row whose Fn is
/// outside 0.2..=1.0, and a wetted transom larger than [`MICHELL_TRANSOM_AREA_LIMIT`] of the largest
/// section (Michell's integral assumes the hull closes at the stern).
pub fn michell_warnings(grid: &MichellGrid, rows: &[Resistance]) -> Vec<Msg> {
    let mut warnings = Vec::new();
    let lb = if grid.beam_wl > 1e-6 {
        grid.length_wl / grid.beam_wl
    } else {
        0.0
    };
    if lb < 6.0 {
        warnings.push(Msg::new(Key::WarnMichellLb).arg("lb", format!("{lb:.2}")));
    }
    let transom = grid.transom_area_ratio();
    if transom > MICHELL_TRANSOM_AREA_LIMIT {
        warnings.push(transom_warning(transom));
    }
    for r in rows {
        if !(0.20..=1.0).contains(&r.fn_) {
            warnings.push(fn_warning(r.fn_, r.speed_kn));
        }
    }
    warnings
}

/// The warning for a wetted transom `ratio` of the largest section above
/// [`MICHELL_TRANSOM_AREA_LIMIT`].
pub fn transom_warning(ratio: f64) -> Msg {
    Msg::new(Key::WarnMichellTransom)
        .arg("pct", format!("{:.0}", ratio * 100.0))
        .arg(
            "limit",
            format!("{:.0}", MICHELL_TRANSOM_AREA_LIMIT * 100.0),
        )
}

/// The warning for a Froude number outside Michell's usual 0.2 to 1.0.
pub fn fn_warning(froude: f64, speed_kn: f64) -> Msg {
    Msg::new(Key::WarnMichellFn)
        .arg("fn", format!("{froude:.3}"))
        .arg("speed", format!("{speed_kn:.2}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Michell R_w of the 10 m Wigley grid (65 x 25, default options) at Fn 0.20, 0.30, 0.35,
    /// 0.45 and as a catamaran (s = 3 m) at Fn 0.30, for [`michell_column_refactor_keeps_every_number`].
    fn frozen_case_values() -> Vec<f64> {
        let grid = wigley_grid(10.0, 1.0, 0.625, 65, 25);
        let g = 9.81;
        let speed = |fn_: f64| fn_ * (g * grid.length_wl).sqrt();
        let mono = MichellOptions::default();
        let cat = MichellOptions {
            catamaran_spacing: Some(3.0),
            ..MichellOptions::default()
        };
        let mut out: Vec<f64> = [0.20, 0.30, 0.35, 0.45]
            .iter()
            .map(|&f| michell_wave_resistance_from_grid(&grid, speed(f), 1000.0, g, &mono).rw_n)
            .collect();
        out.push(michell_wave_resistance_from_grid(&grid, speed(0.30), 1000.0, g, &cat).rw_n);
        out
    }

    /// X(a, b; k) = \int_a^b e^{i k x} dx against its antiderivative (sin kb - sin ka)/k +
    /// i (cos ka - cos kb)/k, and b - a at k = 0: calculus, no source needed.
    #[test]
    fn x_integral_matches_closed_form() {
        for (a, b, k) in [(-5.0_f64, 5.0, 0.0), (-5.0, -4.9, 3.0), (0.3, 2.1, 12.5)] {
            let (re, im) = x_integral(a, b, k);
            let (want_re, want_im) = if k == 0.0 {
                (b - a, 0.0)
            } else {
                (
                    ((k * b).sin() - (k * a).sin()) / k,
                    ((k * a).cos() - (k * b).cos()) / k,
                )
            };
            assert!(
                (re - want_re).abs() <= 1e-12,
                "re {re} vs {want_re} at {a}..{b}, k {k}"
            );
            assert!(
                (im - want_im).abs() <= 1e-12,
                "im {im} vs {want_im} at {a}..{b}, k {k}"
            );
        }
    }

    /// The envelope uses |f_x|, so it bounds |Z_i| and equals it where f_x keeps one sign in a
    /// column (every column of the Wigley hull: f_x > 0 aft of midships, < 0 forward).
    #[test]
    fn column_z_envelope_bounds_the_integrals() {
        let grid = wigley_grid(10.0, 1.0, 0.625, 65, 25);
        let (mut z, mut e) = (Vec::new(), Vec::new());
        for kz in [0.0, 0.5, 3.0, 40.0] {
            column_z_integrals(&grid, kz, &mut z);
            column_z_envelope(&grid, kz, &mut e);
            assert_eq!(z.len(), grid.xs.len() - 1);
            assert_eq!(e.len(), z.len());
            for (zi, ei) in z.iter().zip(&e) {
                assert!(
                    (zi.abs() - ei).abs() <= 1e-12 * ei.max(1e-300),
                    "|{zi}| vs {ei}"
                );
            }
        }
    }

    /// Frozen output of SeaCraft at 6c22607: catches a change, proves nothing about being right.
    #[test]
    fn michell_column_refactor_keeps_every_number() {
        const FROZEN: [f64; 5] = [
            2.5908746260076065e1,
            1.405_210_669_902_722e2,
            1.114_581_008_866_662e2,
            6.132_800_554_934_012e2,
            3.2773244253248106e2,
        ];
        let got_all = frozen_case_values();
        for (got, want) in got_all.iter().zip(FROZEN) {
            assert!(
                ((got - want) / want).abs() <= 1e-12,
                "R_w {got:e} moved from the frozen {want:e}"
            );
        }
    }

    /// Analytical solution of Michell's integral for the parabolic Wigley hull:
    /// y = (B/2) * (1 - (2x/L)^2) * (1 - (z/T)^2), -L/2 <= x <= L/2, -T <= z <= 0.
    ///
    /// f_x is odd in x, so P(theta) = 0.
    /// Q(theta) = -8 * (B / L^2) * Ix(theta) * Iz(theta)
    /// where:
    ///   Ix = \int_0^{L/2} x sin(kx x) dx = - (L/2 / kx) cos(kx L / 2) + (1 / kx^2) sin(kx L / 2)
    ///   Iz = \int_{-T}^0 (1 - z^2 / T^2) exp(kz z) dz
    ///
    /// References:
    /// 1. J.R. Shearer and J.J. Cross (1965), "The Experimental Determination of the
    ///    Components of Ship Resistance for a Mathematical Model", Trans. RINA, Vol. 107.
    /// 2. E.O. Tuck (1987), "Wave Resistance of Thin Ships and Catamarans", Research Report
    ///    T8701, Applied Mathematics Department, University of Adelaide.
    /// 3. L. Lazauskas, "Michlet: Thin-Ship Theory Code & Documentation", cyberiad.net.
    /// 4. A. Sh. Gotman (2002), "The characteristics of the wave resistance of a Wigley model",
    ///    International Shipbuilding Progress, 49(4).
    fn wigley_analytic_rw(l: f64, b: f64, t: f64, u: f64, rho: f64, g: f64, ntheta: usize) -> f64 {
        let k0 = g / (u * u);
        let dtheta = (PI * 0.5) / (ntheta as f64);
        let mut integral = 0.0;
        let half_l = l * 0.5;

        for k in 0..ntheta {
            let theta = (k as f64 + 0.5) * dtheta;
            let cos_th = theta.cos();
            let sec_th = 1.0 / cos_th;
            let sec2_th = sec_th * sec_th;
            let sec3_th = sec2_th * sec_th;

            let kx = k0 * sec_th;
            let kz = k0 * sec2_th;

            let arg = kx * half_l;
            let ix = -half_l / kx * arg.cos() + 1.0 / (kx * kx) * arg.sin();

            let kz_t = kz * t;
            let iz = if kz_t < 1e-4 {
                t * (2.0 / 3.0 - kz_t / 4.0 + kz_t * kz_t / 15.0)
            } else {
                1.0 / kz - 2.0 / (kz * kz_t * kz_t)
                    + (2.0 / (kz * kz_t) + 2.0 / (kz * kz_t * kz_t)) * (-kz_t).exp()
            };

            let q = -8.0 * b / (l * l) * ix * iz;
            integral += q * q * sec3_th * dtheta;
        }

        let factor = (4.0 * rho * g * g) / (PI * u * u);
        factor * integral
    }

    #[test]
    fn test_wigley_hull_matches_independent_quadrature_and_self_consistency() {
        // Test (a): Wigley hull (L/B = 10, B/T = 1.6, parabolic waterlines and
        // sections), Cw on the wetted surface S. The reference is an
        // independent quadrature of the same integral with different numerics
        // (λ = sec θ = cosh u, so the θ → π/2 end is regular; plain trapezoid
        // over an 801 × 201 offset grid, u ∈ [0, 6]), written for this check:
        //   Fn   0.25      0.30      0.35      0.40      0.50
        //   Cw   1.064e-3  2.142e-3  1.248e-3  2.734e-3  4.517e-3
        // It reproduces the interference hump at Fn 0.30, the hollow at 0.35
        // and the main hump at 0.50, with S = 14.88 m² (the exact Wigley wetted
        // surface, `wigley_grid_wetted_surface_matches_kajitani`); this module
        // agrees within 0.3 %. These are not published experimental or
        // tabulated values.
        let (l, b, t) = (10.0, 1.0, 0.625);
        let grid = wigley_grid(l, b, t, 65, 25);
        let rho = 1000.0;
        let g = 9.80665;
        let opts = MichellOptions {
            nx: 65,
            nz: 25,
            ntheta: 128,
            catamaran_spacing: None,
        };

        let reference_cw = [
            (0.25, 1.064e-3),
            (0.30, 2.142e-3),
            (0.35, 1.248e-3),
            (0.40, 2.734e-3),
            (0.50, 4.517e-3),
        ];

        for (fn_val, ref_cw) in reference_cw {
            let u = fn_val * (g * l).sqrt();
            let res = michell_wave_resistance_from_grid(&grid, u, rho, g, &opts);
            let diff = (res.cw - ref_cw).abs() / ref_cw;
            assert!(
                diff < 0.01,
                "at Fn = {fn_val}, computed Cw = {:.4e}, expected reference {:.4e} (diff {:.2}%)",
                res.cw,
                ref_cw,
                diff * 100.0
            );

            // Also check match against exact analytical Michell integral:
            let rw_analytic = wigley_analytic_rw(l, b, t, u, rho, g, 256);
            let diff_analytic = (res.rw_n - rw_analytic).abs() / rw_analytic;
            assert!(
                diff_analytic < 0.01,
                "at Fn = {fn_val}, grid Rw = {:.3}, analytic Rw = {:.3} (diff {:.2}%)",
                res.rw_n,
                rw_analytic,
                diff_analytic * 100.0
            );
        }

        // Self-consistency check 1: Grid refinement changes Cw < 1%
        let fine_grid = wigley_grid(l, b, t, 129, 49);
        let fine_opts = MichellOptions {
            nx: 129,
            nz: 49,
            ntheta: 256,
            catamaran_spacing: None,
        };
        for &fn_val in &[0.25, 0.35, 0.50] {
            let u = fn_val * (g * l).sqrt();
            let c_base = michell_wave_resistance_from_grid(&grid, u, rho, g, &opts).cw;
            let c_fine = michell_wave_resistance_from_grid(&fine_grid, u, rho, g, &fine_opts).cw;
            let diff = (c_fine - c_base).abs() / c_base;
            assert!(
                diff < 0.01,
                "refinement diff at Fn {fn_val} was {:.3}% (must be < 1%)",
                diff * 100.0
            );
        }

        // Self-consistency check 2: Cw is small at Fn 0.10. Converged (513 × 97
        // grid, 8192 θ points) it is 9.997e-5, about a tenth of the Fn 0.25
        // value; the bound leaves room for this grid's 0.3 % error.
        let u_low = 0.10 * (g * l).sqrt();
        let res_low = michell_wave_resistance_from_grid(&grid, u_low, rho, g, &opts);
        assert!(
            res_low.cw < 1.5e-4,
            "at Fn=0.10, Cw must be negligible, got {:.3e}",
            res_low.cw
        );

        // Self-consistency check 3: Hump near Fn ≈ 0.5 exists
        let u_035 = 0.35 * (g * l).sqrt();
        let u_050 = 0.50 * (g * l).sqrt();
        let cw_035 = michell_wave_resistance_from_grid(&grid, u_035, rho, g, &opts).cw;
        let cw_050 = michell_wave_resistance_from_grid(&grid, u_050, rho, g, &opts).cw;
        assert!(
            cw_050 > 3.0 * cw_035,
            "hump near Fn=0.5 must exist: Cw(0.5)={cw_050:.4e} vs Cw(0.35)={cw_035:.4e}"
        );

        // Self-consistency check 4: Rw scales linearly with rho
        let u_test = 0.30 * (g * l).sqrt();
        let r1 = michell_wave_resistance_from_grid(&grid, u_test, 1000.0, g, &opts).rw_n;
        let r2 = michell_wave_resistance_from_grid(&grid, u_test, 2000.0, g, &opts).rw_n;
        assert!(
            (r2 / r1 - 2.0).abs() < 1e-9,
            "Rw must scale linearly with rho: r2/r1 = {}",
            r2 / r1
        );
    }

    #[test]
    fn wigley_grid_wetted_surface_matches_kajitani() {
        // Kajitani et al. (1983), Nomenclature (report p. 6): S = Cs L (2D + B)
        // with Cs = 0.661 for the Wigley model, i.e. 14.87 m² at L = 10,
        // B = 1, D = 0.625 m. A Gauss-Legendre quadrature (400 × 200) of
        // 2 ∬ sqrt(1 + f_x² + f_z²) dx dz gives 14.8791 m² (Cs = 0.66129).
        let (l, b, t) = (10.0, 1.0, 0.625);
        let s = wigley_grid(l, b, t, 65, 25).wetted_surface_m2;
        let kajitani = 0.661 * l * (2.0 * t + b);
        assert!((s / kajitani - 1.0).abs() < 1e-3, "S {s} vs {kajitani}");
        assert!((s / 14.879_063 - 1.0).abs() < 1e-4, "S {s}");
    }

    /// Michell's Cw on the 10 m Wigley grid (`nx` × `nz` offsets, `ntheta` angles) at Froude number `fn_`.
    fn wigley_cw(fn_: f64, nx: usize, nz: usize, ntheta: usize) -> f64 {
        let (l, g) = (10.0, 9.80665);
        let grid = wigley_grid(l, 1.0, 0.625, nx, nz);
        let opts = MichellOptions {
            nx,
            nz,
            ntheta,
            catamaran_spacing: None,
        };
        michell_wave_resistance_from_grid(&grid, fn_ * (g * l).sqrt(), 1000.0, g, &opts).cw
    }

    #[test]
    #[allow(clippy::approx_constant)] // Fn 0.318 is read off a figure, not 1/π
    fn wigley_michell_against_kajitani_1983_towing_tests() {
        // Kajitani, Miyata, Ikehata, Tanaka, Adachi, Namimatsu & Ogiwara (1983), "The summary
        // of the cooperative experiment on Wigley parabolic model in Japan", Proc. 2nd DTNSRDC
        // Workshop on Ship Wave-Resistance Computations, pp. 5-35 (DTIC ADP003037): Wigley
        // geosims with B/L = 0.1, D/L = 0.0625, the proportions of `wigley_grid`. Cw =
        // C_T - (1 + k) C_F0 (Schoenherr) on S = 0.661 L (2D + B). Read from Fig. 2 (fixed
        // sinkage and trim) and Figs. 1 and 5 (free), on a calibrated grid over 300 dpi
        // renders: Cw (×1e-3) ± 0.05, Fn ± 0.005. docs/oracles.md §39 has the method.
        //           model         hump Fn, Cw    hollow Fn, Cw
        let fixed = [
            ("SRI 4.0 m FX", 0.320, 1.39, 0.350, 1.21),
            ("UT 2.5 m FX", 0.318, 1.45, 0.350, 1.30),
        ];
        let free = [
            ("IHI 6.0 m FR", 0.313, 1.81, 0.345, 1.62),
            ("SRI 4.0 m FR", 0.320, 1.61, 0.350, 1.45),
            ("UT 2.5 m FR", 0.320, 1.70, 0.345, 1.53),
            ("YNU 2.0 m FR", 0.322, 1.59, 0.347, 1.48),
        ];

        // Michell's interference hump and the hollow after it, to 0.001 in Fn.
        let curve: Vec<(f64, f64)> = (0..=90)
            .map(|i| {
                let f = 0.28 + 0.001 * i as f64;
                (f, wigley_cw(f, 65, 25, 256) * 1e3)
            })
            .collect();
        let (hump_fn, hump) = curve
            .iter()
            .filter(|(f, _)| *f < 0.33)
            .copied()
            .fold((0.0, f64::MIN), |a, b| if b.1 > a.1 { b } else { a });
        let (hollow_fn, hollow) = curve
            .iter()
            .filter(|(f, _)| *f > 0.33)
            .copied()
            .fold((0.0, f64::MAX), |a, b| if b.1 < a.1 { b } else { a });
        // Michell: hump 2.14e-3 at Fn 0.299, hollow 1.235e-3 at 0.346.
        assert!((hump_fn - 0.299).abs() < 0.0015, "hump at {hump_fn}");
        assert!((hollow_fn - 0.346).abs() < 0.0015, "hollow at {hollow_fn}");

        // What thin-ship theory gets right and wrong on this hull, as bands round
        // the measured spread widened by the reading uncertainty. Not tuned: the
        // hump is 0.014-0.023 early and 18-54 % high; the hollow is in place, and
        // in the fixed condition, which Michell models, within 5 % in size.
        for (model, h_fn, h_cw, l_fn, l_cw) in fixed.iter().chain(free.iter()) {
            let early = h_fn - hump_fn;
            assert!(
                (0.009..=0.028).contains(&early),
                "{model}: hump {early:.3} early"
            );
            assert!(
                (l_fn - hollow_fn).abs() <= 0.009,
                "{model}: hollow at {l_fn} vs {hollow_fn}"
            );
            let high = hump / h_cw;
            assert!(
                (1.14..=1.60).contains(&high),
                "{model}: hump ratio {high:.3}"
            );
            let low = hollow / l_cw;
            assert!(
                (0.73..=1.07).contains(&low),
                "{model}: hollow ratio {low:.3}"
            );
        }
        for (model, _, _, _, l_cw) in fixed {
            let r = hollow / l_cw;
            assert!(
                (0.91..=1.07).contains(&r),
                "{model}: fixed hollow ratio {r:.3}"
            );
        }
    }

    #[test]
    fn wigley_michell_matches_siren_independent_computation() {
        // Peri, D. (2026), "SIREN: A nonlinear Rankine-source boundary element solver for the
        // stationary motion of a marine vehicle in calm water", arXiv:2610.14027, Table 3,
        // column "Michell": Michell's integral for the same Wigley hull from an independent
        // Python script (λ = cosh t, Simpson quadrature), Cw on the true wetted surface. An
        // independent computation of the same theory, not experiment.
        let siren = [
            (0.250, 1.064),
            (0.267, 0.976),
            (0.289, 1.987),
            (0.316, 1.831),
            (0.354, 1.284),
            (0.408, 3.030),
            (0.450, 4.154),
        ];
        for (fn_, cw) in siren {
            let ours = wigley_cw(fn_, 129, 49, 512) * 1e3;
            let diff = (ours / cw - 1.0).abs();
            assert!(
                diff < 0.002,
                "Fn {fn_}: {ours:.4} vs SIREN {cw} ({:.3} %)",
                diff * 100.0
            );
        }
    }

    #[test]
    fn transom_ratio_is_the_aft_section_over_the_largest_one() {
        // Every Wigley station has the same z-shape, so a section's area is
        // proportional to 1 - (2x/L)^2 whatever the z-quadrature: 0 at the closed
        // stern, and 1 - 0.6^2 = 0.64 on a Wigley cut off at x = -0.3 L.
        let (l, b, t) = (10.0, 1.0, 0.625);
        let closed = wigley_grid(l, b, t, 81, 25);
        assert_eq!(closed.transom_area_ratio(), 0.0);

        let mut cut = closed.clone();
        let first = cut.xs.iter().position(|&x| x >= -0.3 * l - 1e-9).unwrap();
        assert!(
            (cut.xs[first] + 3.0).abs() < 1e-9,
            "x = -3 m is a grid station"
        );
        cut.xs.drain(..first);
        cut.half_breadths.drain(..first);
        assert!(
            (cut.transom_area_ratio() - 0.64).abs() < 1e-12,
            "{}",
            cut.transom_area_ratio()
        );

        let rows = |g: &MichellGrid| {
            michell_resistance_curve(
                g,
                1.0,
                1.19e-6,
                1000.0,
                9.80665,
                &[6.0],
                &MichellOptions::default(),
            )
        };
        let flagged = |g: &MichellGrid| {
            michell_warnings(g, &rows(g))
                .iter()
                .any(|w| w.to_string().contains("wetted transom"))
        };
        assert!(!flagged(&closed));
        assert!(flagged(&cut));
    }

    #[test]
    fn transom_warning_fires_on_the_demo_hull_and_not_on_the_wigley_example() {
        let subdiv = SubdivisionOptions {
            level: 3,
            mode: crate::subdiv::SubdivisionMode::Loop,
        };
        let ratio = |s: &Surface| {
            let draft = HydroOptions::from_surface(s, subdiv).draft;
            michell_grid_from_surface(s, draft, 65, 25, subdiv)
                .expect("grid")
                .transom_area_ratio()
        };
        // The demo hull's transom is immersed at its 0.62 m design draft.
        let demo = ratio(&crate::demo::demo_hull(true));
        assert!(demo > MICHELL_TRANSOM_AREA_LIMIT, "demo {demo}");
        // The Wigley example closes at the stern: what remains is the grid's
        // end station, cut 0.05 % of L inside the hull.
        let (wigley, _) = crate::examples::build(crate::examples::Example::Wigley).unwrap();
        let w = ratio(&wigley);
        assert!(w < 0.01, "wigley {w}");
    }

    #[test]
    fn test_symmetric_fore_aft_hull_gives_same_rw_reversed() {
        // Test (b): A symmetric fore/aft hull gives the same Rw reversed.
        let (l, b, t) = (12.0, 1.2, 0.75);
        let grid = wigley_grid(l, b, t, 65, 25);
        let rho = 1025.0;
        let g = 9.80665;
        let opts = MichellOptions {
            nx: 65,
            nz: 25,
            ntheta: 128,
            catamaran_spacing: None,
        };

        // Construct reversed grid: x -> -x, reversed stations
        let mut rev_grid = grid.clone();
        rev_grid.xs = grid.xs.iter().rev().map(|x| -x).collect();
        rev_grid.half_breadths = grid.half_breadths.iter().rev().cloned().collect();

        for &fn_val in &[0.25, 0.35, 0.45] {
            let u = fn_val * (g * l).sqrt();
            let r_fwd = michell_wave_resistance_from_grid(&grid, u, rho, g, &opts).rw_n;
            let r_rev = michell_wave_resistance_from_grid(&rev_grid, u, rho, g, &opts).rw_n;
            let rel_diff = (r_fwd - r_rev).abs() / r_fwd;
            assert!(
                rel_diff < 1e-6,
                "at Fn {fn_val}: forward Rw = {r_fwd}, reversed Rw = {r_rev} (rel diff {rel_diff})"
            );
        }
    }

    #[test]
    fn test_catamaran_large_spacing_doubles_monohull() {
        // Test (c): Catamaran at very large spacing -> 2x monohull within 2%.
        let (l, b, t) = (10.0, 1.0, 0.625);
        let grid = wigley_grid(l, b, t, 65, 25);
        let rho = 1000.0;
        let g = 9.80665;
        let u = 0.35 * (g * l).sqrt();

        let mono_opts = MichellOptions {
            nx: 65,
            nz: 25,
            ntheta: 512,
            catamaran_spacing: None,
        };
        let mono_rw = michell_wave_resistance_from_grid(&grid, u, rho, g, &mono_opts).rw_n;

        // Large separation: s = 100 m on 10 m hull (s / L = 10)
        let cat_opts = MichellOptions {
            nx: 65,
            nz: 25,
            ntheta: 512,
            catamaran_spacing: Some(100.0),
        };
        let cat_rw = michell_wave_resistance_from_grid(&grid, u, rho, g, &cat_opts).rw_n;

        let ratio = cat_rw / mono_rw;
        assert!(
            (ratio - 2.0).abs() < 0.02,
            "catamaran at large spacing should give 2x monohull within 2%, got ratio {:.4}",
            ratio
        );

        // Friction, total and Cw follow: two demihulls far apart drag twice as
        // much as one, with the same coefficient.
        let curve = |o: &MichellOptions| {
            michell_resistance_curve(&grid, 1.0, 1.19e-6, rho, g, &[8.0], o)[0].clone()
        };
        let (mono, cat) = (curve(&mono_opts), curve(&cat_opts));
        assert!((cat.rf_n / mono.rf_n - 2.0).abs() < 1e-12);
        assert!(
            (cat.rt_n / mono.rt_n - 2.0).abs() < 0.02,
            "{} vs {}",
            cat.rt_n,
            mono.rt_n
        );
    }

    #[test]
    fn test_parametric_slender_hull_michell_rw_vs_holtrop() {
        // Test (d): The parametric slender hull's Michell R_W is within a factor
        // of 2 of Holtrop's R_W at Fn 0.35 (sanity check, not accuracy).
        for kind in [
            crate::parametric::HullKind::Displacement,
            crate::parametric::HullKind::Yacht,
        ] {
            let mut spec = crate::parametric::HullSpec::default_for(kind);
            spec.lwl = 25.0;
            spec.beam = 2.5; // L/B = 10.0 (very slender)
            spec.draft = 0.8;
            spec.depth = 1.8;
            spec.transom_frac = 0.0;

            let (surface, _) = crate::parametric::generate(&spec).expect("parametric slender hull");
            let subdiv = SubdivisionOptions {
                level: 3,
                mode: crate::subdiv::SubdivisionMode::Loop,
            };
            let hull = Hull::new(&surface, subdiv);
            let mut opts = HydroOptions::from_surface(&surface, subdiv);
            opts.draft = spec.draft;

            let holtrop_in =
                resistance::HoltropInput::measure(&hull, &opts).expect("Holtrop input");
            let holtrop_rw = resistance::wave_resistance(&holtrop_in, 0.35);

            let michell_opts = MichellOptions {
                nx: 65,
                nz: 25,
                ntheta: 128,
                catamaran_spacing: None,
            };
            let grid = michell_grid_from_surface(&surface, spec.draft, 65, 25, subdiv)
                .expect("michell grid from surface");
            let u = 0.35 * (resistance::GRAVITY * holtrop_in.length_wl).sqrt();
            let michell_res = michell_wave_resistance_from_grid(
                &grid,
                u,
                holtrop_in.water_density,
                resistance::GRAVITY,
                &michell_opts,
            );

            let ratio = michell_res.rw_n / holtrop_rw;
            println!(
                "Kind {:?} Fn=0.35: Michell Rw = {:.1} N, Holtrop Rw = {:.1} N, ratio = {:.3}",
                kind, michell_res.rw_n, holtrop_rw, ratio
            );
            assert!(
                (0.5..=2.0).contains(&ratio),
                "Kind {:?} Michell Rw ({:.1} N) and Holtrop Rw ({:.1} N) must be within a factor of 2, ratio = {:.3}",
                kind,
                michell_res.rw_n,
                holtrop_rw,
                ratio
            );
        }
    }

    /// The curve is the single-speed result at each Froude number, in order.
    #[test]
    fn cw_curve_matches_single_calls() {
        let grid = wigley_grid(10.0, 1.0, 0.625, 65, 25);
        let fr = [0.2, 0.35, 0.5];
        let curve = michell_cw_curve(&grid, &fr, 1000.0, 9.81, &MichellOptions::default());
        assert_eq!(curve.len(), fr.len());
        for (r, f) in curve.iter().zip(fr) {
            let v = f * (9.81 * grid.length_wl).sqrt();
            let one = michell_wave_resistance_from_grid(
                &grid,
                v,
                1000.0,
                9.81,
                &MichellOptions::default(),
            );
            assert_eq!(r.rw_n, one.rw_n);
        }
    }

    /// Analytic limit: Michell's factor 2(1 + cos(k0 s sec²θ sinθ)) averages to 2 as s grows (module
    /// docs of wave_pattern.rs), so the catamaran tends to twice one demihull.
    #[test]
    fn interference_tends_to_one_far_apart() {
        let grid = wigley_grid(10.0, 1.0, 0.625, 65, 25);
        let v = 0.4 * (9.81 * 10.0f64).sqrt();
        let r = interference(&grid, v, 1000.0, 9.81, 500.0).unwrap();
        assert!((r - 1.0).abs() < 0.05, "{r}");
    }

    /// Property: at close spacings the interference is not neutral.
    #[test]
    fn interference_moves_with_spacing() {
        let grid = wigley_grid(10.0, 1.0, 0.625, 65, 25);
        let v = 0.4 * (9.81 * 10.0f64).sqrt();
        let rs: Vec<f64> = [2.0, 3.0, 4.0, 5.0]
            .iter()
            .map(|&s| interference(&grid, v, 1000.0, 9.81, s).unwrap())
            .collect();
        assert!(rs.iter().any(|r| (r - 1.0).abs() > 0.05), "{rs:?}");
    }

    #[test]
    fn local_extrema_find_humps_and_hollows() {
        let (max, min) = local_extrema(&[0.0, 1.0, 0.5, 0.7, 0.7, 0.2, 0.9]);
        assert_eq!(max, vec![1, 3]);
        assert_eq!(min, vec![2, 5]);
    }

    /// Property of this implementation, frozen 2026-11-19 (oracles §41): over 60 Froude numbers from
    /// 0.10 to 1.00, the Wigley C_w curve has a maximum ("hump") in 0.45..=0.55. §39's references
    /// stop at Fn 0.45, so this is not a published value.
    #[test]
    fn wigley_cw_curve_has_a_hump_near_fn_0_5() {
        let grid = wigley_grid(10.0, 1.0, 0.625, 65, 25);
        let fr: Vec<f64> = (0..60).map(|i| 0.10 + 0.90 * i as f64 / 59.0).collect();
        let curve = michell_cw_curve(&grid, &fr, 1000.0, 9.81, &MichellOptions::default());
        let cw: Vec<f64> = curve.iter().map(|r| r.cw).collect();
        let (max, _) = local_extrema(&cw);
        let humps: Vec<f64> = max.iter().map(|&i| fr[i]).collect();
        eprintln!("humps at Fn {humps:?}");
        assert!(humps.iter().any(|f| (0.45..=0.55).contains(f)), "{humps:?}");
    }
}
