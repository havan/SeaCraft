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
        let fx = -4.0 * half_b * x_mid / (half_l * half_l);
        for j in 0..nz - 1 {
            let dz = zs[j + 1] - zs[j];
            let z_mid = 0.5 * (zs[j] + zs[j + 1]);
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

    for k in 0..ntheta {
        let theta = (k as f64 + 0.5) * dtheta;
        let cos_th = theta.cos();
        let sin_th = theta.sin();
        let sec_th = 1.0 / cos_th;
        let sec2_th = sec_th * sec_th;
        let sec3_th = sec2_th * sec_th;

        let kx = k0 * sec_th;
        let kz = k0 * sec2_th;

        // Panel-wise integration over (x, z):
        // Over each panel [x_i, x_{i+1}] x [z_j, z_{j+1}],
        // f(x, z) is bilinear, so f_x is constant in x and linear in z.
        // The x-integral has closed form:
        //   \int_{x_i}^{x_{i+1}} exp(i kx x) dx = dx_i * sinc(kx * dx_i / 2) * exp(i kx * x_mid)
        // The z-integral has closed form:
        //   \int_{z_j}^{z_{j+1}} (f_{x0} * (1 - w) + f_{x1} * w) exp(kz z) dz
        let mut p_val = 0.0;
        let mut q_val = 0.0;

        for i in 0..nx - 1 {
            let dx = grid.xs[i + 1] - grid.xs[i];
            let x_mid = 0.5 * (grid.xs[i] + grid.xs[i + 1]);
            let half_arg = 0.5 * kx * dx;
            let sinc = if half_arg.abs() < 1e-4 {
                1.0 - half_arg * half_arg / 6.0
            } else {
                half_arg.sin() / half_arg
            };
            let ix_mag = dx * sinc;
            let ix_cos = ix_mag * (kx * x_mid).cos();
            let ix_sin = ix_mag * (kx * x_mid).sin();

            // Sum over z panels for this station
            let mut z_sum = 0.0;
            for j in 0..nz - 1 {
                let dz = grid.zs[j + 1] - grid.zs[j];
                let z_j = grid.zs[j];
                let z_j1 = grid.zs[j + 1];

                let fx0 = (grid.half_breadths[i + 1][j] - grid.half_breadths[i][j]) / dx;
                let fx1 = (grid.half_breadths[i + 1][j + 1] - grid.half_breadths[i][j + 1]) / dx;

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

    // Normalised wave resistance coefficient Cw = Rw / (0.5 * rho * U^2 * S)
    let q_dyn = 0.5 * rho * u * u * grid.wetted_surface_m2;
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
            let rf = q * grid.wetted_surface_m2 * cf;
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

/// Check thin-ship validity warnings: Fn outside 0.2..=1.0 or L/B < 6.0.
pub fn michell_warnings(length_wl: f64, beam_wl: f64, fn_: f64) -> Vec<String> {
    let mut warnings = Vec::new();
    let lb = if beam_wl > 1e-6 {
        length_wl / beam_wl
    } else {
        0.0
    };
    if lb < 6.0 {
        warnings.push(format!(
            "L/B {lb:.2} is outside thin-ship recommended range (L/B >= 6.0)"
        ));
    }
    if !(0.20..=1.0).contains(&fn_) {
        warnings.push(format!(
            "Fn {fn_:.3} is outside typical Michell applicability range (0.20..=1.0)"
        ));
    }
    warnings
}

#[cfg(test)]
mod tests {
    use super::*;

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
        // and the main hump at 0.50; this module sits ~2 % lower throughout
        // (the reference under-resolves the x oscillation at large λ). These
        // are not published experimental or tabulated values.
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
                diff < 0.05,
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

        // Self-consistency check 2: Cw -> 0 as Fn -> 0.1
        let u_low = 0.10 * (g * l).sqrt();
        let res_low = michell_wave_resistance_from_grid(&grid, u_low, rho, g, &opts);
        assert!(
            res_low.cw < 1e-4,
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
}
