//! Far-field thin-ship (Michell) wave pattern: the elevation `ζ(x, y)` of the free waves behind a
//! monohull or a catamaran, from the same centreplane grid as Michell's wave resistance
//! ([`crate::michell`]).
//!
//! # Representation
//!
//! Wu, H., Wu, J., He, J., Zhu, R., Yang, C.-J. & Noblesse, F. (2019), "Wave profile along a ship
//! hull, short far-field waves, and broad inner Kelvin wake sans divergent waves", *Physics of
//! Fluids* 31, 047102 (read 2026-11-18): the far-field waves aft of a ship are a superposition of
//! elementary waves (eqs. 9b, 10a) weighted by an amplitude function (eq. 12a) over the mean wetted
//! hull, eq. 14a. In the Hogner (thin-ship) limit the amplitude densities are `A± = n_x` (eq. 12d
//! without the potential terms). On a hull `y = f(x, z)`, `n_x da = -f_x dx dz`; the thin-ship
//! limit puts that integral on the centreplane, where it is Michell's `P + iQ`. With their
//! `1/F² = k0 L` this gives `A = -(2 k0 / L) (P - iQ)` and, in metres,
//!
//! ```text
//! ζ(x, y) = -(2 k0 / π) ∫_0^θc sec³θ Re[ conj(S(x; θ)) (e^{iψ+} + e^{iψ-}) ] dθ
//! ψ± = k0 sec²θ (x cos θ ± y sin θ),   k0 = g / U²,
//! ```
//!
//! model axes: x forward (bow at the largest x), y to starboard, z up. Michell's grid gives
//! `P + iQ = Σ_i Z_i(θ) X(x_i, x_{i+1}; k0 sec θ)`, with `Z_i` the closed-form depth integral of
//! column `i` and `X(a, b; k) = ∫_a^b e^{ikξ} dξ` (see `michell::column_z_integrals`).
//!
//! # Downstream only
//!
//! A field point at `x` sees the part of the hull forward of it: `S(x; θ)` sums the columns wholly
//! forward of `x` and the forward part `[x, x_{j+1}]` of the column `j` that contains `x`. Behind the
//! stern `S = P + iQ`, the whole hull, and the field is Wu's eq. 14a. Alongside the hull this
//! cumulative form is SeaCraft's extension, not Wu's. Ahead of the bow `S = 0` and so is `ζ`.
//! Per wave angle the suffix sums of `Z_i X_i` are built once, so a point costs one partial column.
//!
//! # Catamaran
//!
//! Two demihulls at `y = ±s/2` multiply the integrand by `2 cos(k0 sec²θ sin θ · s/2)`, whose square
//! is Michell's interference factor `2 (1 + cos(k0 s sec θ tan θ))`.
//!
//! # Quadrature and the short-wave cut
//!
//! With `sec θ = cosh u` (so `sec³θ dθ = cosh²u du`) the trapezoid rule on `[0, u_c]` is the
//! trapezoid rule on `(-u_c, u_c)` for `e^{iψ+}` alone, since `ψ-(θ) = ψ+(-θ)`. Samples are set so the
//! phase moves at most `phase_step` between them anywhere in the domain:
//! `n = ⌈k0 (R_x sinh u_c + R_y cosh 2u_c) u_c / phase_step⌉`, at least 64, with `R_x` the largest
//! `|x - x_station|` and `R_y` the largest `|y|`. The cut `θc` is the smaller of the Nyquist angle of
//! the output spacing (`k0 sec²θ = π / spacing`) and the angle above which the spectrum's envelope
//! `sec³θ Σ_i Δx_i ∫|f_x,i| e^{k z} dz` stays below `spectrum_floor` of its peak. Shorter waves are
//! left out, not drawn aliased.
//!
//! # A consistency remark, not a source
//!
//! With `𝒜(θ) = -(2k0/π) sec³θ (P - iQ)` over `θ ∈ (-π/2, π/2)`, Havelock's
//! `R_w = (π ρ U² / 2) ∫ |𝒜|² cos³θ dθ` equals Michell's `(4 ρ g² / (π U²)) ∫_0^{π/2} (P² + Q²) sec³θ dθ`
//! term by term. Havelock's formula is quoted here as that remark only; the scale of the field rests
//! on Wu's equations above, and `docs/oracles.md` §40 checks the evaluation against Kelvin's far-field
//! asymptotics (Wu et al. 2019 eqs. 15a-15d).
//!
//! # What this is not
//!
//! Free waves only: the local disturbance is left out, so the elevation near the hull is not the
//! hull wave profile. Thin-ship theory, deep water, no sinkage or trim, no nonlinearity. Michell's
//! own limits apply (`michell_warnings`): slender hulls, Fn 0.2 to 1.0, a hull closing at the stern.

use crate::michell::{column_z_envelope, column_z_integrals, x_integral, MichellGrid};
use crate::par::par_map;
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Printed with every wave-pattern output.
pub const FAR_FIELD_NOTE: &str =
    "Far-field thin-ship (Michell) waves: free waves only, without the local \
    disturbance, so the elevation near the hull is not the hull wave profile.";

/// Wave-angle samples beyond which [`wave_spectrum`] refuses rather than run for minutes.
pub const MAX_THETA_SAMPLES: usize = 1_000_000;

/// Samples of the envelope when finding the spectral cut.
const ENVELOPE_SAMPLES: usize = 512;

/// What to compute and where.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WavePatternOptions {
    /// Ship speed U (m/s).
    pub speed_m_s: f64,
    /// Gravitational acceleration (m/s²).
    pub g: f64,
    /// Domain ahead of the bow, in waterline lengths.
    pub ahead_l: f64,
    /// Domain behind the stern, in waterline lengths.
    pub behind_l: f64,
    /// Half-width of the domain each side of the centreline, in waterline lengths.
    pub half_width_l: f64,
    /// Grid spacing (m); `None` takes a twentieth of the transverse wavelength.
    pub spacing_m: Option<f64>,
    /// Demihull centreline separation (m) for a catamaran; `None` for a monohull.
    pub catamaran_spacing: Option<f64>,
    /// Largest phase change between wave-angle samples (radians).
    pub phase_step: f64,
    /// Fraction of its peak below which the spectrum's envelope ends the wave-angle range.
    pub spectrum_floor: f64,
}

impl WavePatternOptions {
    /// Defaults at `speed_m_s`: 1 L ahead, 3 L behind, ±1.5 L wide, standard gravity.
    pub fn at_speed(speed_m_s: f64) -> Self {
        Self {
            speed_m_s,
            g: crate::resistance::GRAVITY,
            ahead_l: 1.0,
            behind_l: 3.0,
            half_width_l: 1.5,
            spacing_m: None,
            catamaran_spacing: None,
            phase_step: PI / 6.0,
            spectrum_floor: 1e-4,
        }
    }

    fn validate(&self) -> Result<(), String> {
        let positive = |name: &str, v: f64| {
            if v.is_finite() && v > 0.0 {
                Ok(())
            } else {
                Err(format!("{name} must be a finite number above 0, got {v}"))
            }
        };
        let not_negative = |name: &str, v: f64| {
            if v.is_finite() && v >= 0.0 {
                Ok(())
            } else {
                Err(format!(
                    "{name} must be a finite number of 0 or more, got {v}"
                ))
            }
        };
        positive("speed (m/s)", self.speed_m_s)?;
        positive("g (m/s²)", self.g)?;
        not_negative("ahead (L)", self.ahead_l)?;
        not_negative("behind (L)", self.behind_l)?;
        not_negative("half-width (L)", self.half_width_l)?;
        if let Some(h) = self.spacing_m {
            positive("grid spacing (m)", h)?;
        }
        if let Some(s) = self.catamaran_spacing {
            positive("catamaran spacing (m)", s)?;
        }
        if !(self.phase_step.is_finite() && self.phase_step > 0.0 && self.phase_step <= PI / 2.0) {
            return Err(format!(
                "phase step must be above 0 and at most π/2, got {}",
                self.phase_step
            ));
        }
        if !(self.spectrum_floor > 0.0 && self.spectrum_floor < 1.0) {
            return Err(format!(
                "spectrum floor must be between 0 and 1, got {}",
                self.spectrum_floor
            ));
        }
        Ok(())
    }
}

/// The hull's wave spectrum sampled in wave angle, ready to evaluate `ζ` anywhere in the reach it
/// was built for. Owns a copy of the grid, for [`WaveSpectrum::amplitude`].
#[derive(Debug, Clone)]
pub struct WaveSpectrum {
    k0: f64,
    grid: MichellGrid,
    /// Samples of `u` (`sec θ = cosh u`).
    u: Vec<f64>,
    /// Trapezoid weight times `cosh²u` (`sec³θ dθ = cosh²u du`), times the catamaran factor.
    weight: Vec<f64>,
    /// `k0 sec θ`: the wavenumber along x.
    kx: Vec<f64>,
    /// `k0 sec θ tan θ`: the wavenumber along y.
    beta: Vec<f64>,
    /// `Z_i` per sample, `columns` per row.
    z: Vec<f64>,
    /// Suffix sums of `Z_i X_i` per sample, `columns + 1` per row (the last is 0).
    suffix: Vec<(f64, f64)>,
    theta_cut: f64,
}

/// Build the spectrum of `grid` at `opts` for field points whose `|x - x_station|` is at most
/// `reach_x_m` and `|y|` at most `reach_y_m`, keeping waves no shorter than two `nyquist_spacing_m`.
pub fn wave_spectrum(
    grid: &MichellGrid,
    opts: &WavePatternOptions,
    reach_x_m: f64,
    reach_y_m: f64,
    nyquist_spacing_m: f64,
) -> Result<WaveSpectrum, String> {
    grid.validate()?;
    opts.validate()?;
    for (name, v) in [("reach in x (m)", reach_x_m), ("reach in y (m)", reach_y_m)] {
        if !(v.is_finite() && v >= 0.0) {
            return Err(format!(
                "{name} must be a finite number of 0 or more, got {v}"
            ));
        }
    }
    if !(nyquist_spacing_m.is_finite() && nyquist_spacing_m > 0.0) {
        return Err(format!(
            "grid spacing must be a finite number above 0 m, got {nyquist_spacing_m}"
        ));
    }
    let k0 = opts.g / (opts.speed_m_s * opts.speed_m_s);
    let sec2_nyquist = PI / (nyquist_spacing_m * k0);
    if sec2_nyquist <= 1.0 + 1e-9 {
        let lambda = 2.0 * PI / k0;
        return Err(format!(
            "grid spacing {nyquist_spacing_m} m is too coarse for transverse waves {lambda:.3} m long: \
             use less than {:.3} m",
            lambda / 2.0
        ));
    }
    let u_nyquist = sec2_nyquist.sqrt().acosh();
    let u_c = spectral_cut(grid, k0, u_nyquist, opts.spectrum_floor);

    let reach_y = reach_y_m + opts.catamaran_spacing.unwrap_or(0.0) / 2.0;
    let phase_rate = k0 * (reach_x_m * u_c.sinh() + reach_y * (2.0 * u_c).cosh());
    let intervals = ((phase_rate * u_c / opts.phase_step).ceil() as usize).max(64);
    if intervals >= MAX_THETA_SAMPLES {
        return Err(format!(
            "the domain needs {intervals} wave-angle samples (more than {MAX_THETA_SAMPLES}): \
             make it smaller or the grid spacing larger"
        ));
    }
    let du = u_c / intervals as f64;
    let columns = grid.xs.len() - 1;
    let catamaran = opts.catamaran_spacing;

    let rows = par_map(&(0..=intervals).collect::<Vec<_>>(), |&m| {
        let u = m as f64 * du;
        let (ch, sh) = (u.cosh(), u.sinh());
        let kx = k0 * ch;
        let beta = k0 * ch * sh;
        let end = if m == 0 || m == intervals { 0.5 } else { 1.0 };
        let mut weight = end * du * ch * ch;
        if let Some(s) = catamaran {
            weight *= 2.0 * (beta * s / 2.0).cos();
        }
        let mut z = Vec::with_capacity(columns);
        column_z_integrals(grid, k0 * ch * ch, &mut z);
        let mut suffix = vec![(0.0, 0.0); columns + 1];
        for i in (0..columns).rev() {
            let (re, im) = x_integral(grid.xs[i], grid.xs[i + 1], kx);
            suffix[i] = (suffix[i + 1].0 + z[i] * re, suffix[i + 1].1 + z[i] * im);
        }
        (u, weight, kx, beta, z, suffix)
    });

    let n = rows.len();
    let mut sp = WaveSpectrum {
        k0,
        grid: grid.clone(),
        u: Vec::with_capacity(n),
        weight: Vec::with_capacity(n),
        kx: Vec::with_capacity(n),
        beta: Vec::with_capacity(n),
        z: Vec::with_capacity(n * columns),
        suffix: Vec::with_capacity(n * (columns + 1)),
        theta_cut: u_c.sinh().atan(),
    };
    for (u, weight, kx, beta, z, suffix) in rows {
        sp.u.push(u);
        sp.weight.push(weight);
        sp.kx.push(kx);
        sp.beta.push(beta);
        sp.z.extend(z);
        sp.suffix.extend(suffix);
    }
    Ok(sp)
}

/// The `u` above which the envelope `sec³θ Σ_i Δx_i ∫|f_x,i| e^{k z} dz` stays below `floor` of its
/// peak over `[0, u_max]`, or `u_max` when it never falls that low.
fn spectral_cut(grid: &MichellGrid, k0: f64, u_max: f64, floor: f64) -> f64 {
    let mut env = Vec::new();
    let values: Vec<f64> = (0..=ENVELOPE_SAMPLES)
        .map(|k| {
            let u = u_max * k as f64 / ENVELOPE_SAMPLES as f64;
            let ch = u.cosh();
            column_z_envelope(grid, k0 * ch * ch, &mut env);
            let sum: f64 = env
                .iter()
                .zip(grid.xs.windows(2))
                .map(|(e, x)| e * (x[1] - x[0]))
                .sum();
            sum * ch * ch * ch
        })
        .collect();
    let peak = values.iter().copied().fold(0.0, f64::max);
    match values.iter().rposition(|&v| v >= floor * peak) {
        Some(k) if k < ENVELOPE_SAMPLES => u_max * (k + 1) as f64 / ENVELOPE_SAMPLES as f64,
        _ => u_max,
    }
}

impl WaveSpectrum {
    /// The wave elevation `ζ(x, y)` (m) at a point in model axes; exactly 0 ahead of the bow.
    pub fn elevation(&self, x: f64, y: f64) -> f64 {
        let xs = &self.grid.xs;
        let columns = xs.len() - 1;
        if x >= xs[columns] {
            return 0.0;
        }
        // The column holding x, when x is alongside the hull.
        let partial = (x > xs[0]).then(|| xs.partition_point(|&s| s <= x) - 1);
        let mut sum = 0.0;
        for m in 0..self.u.len() {
            let kx = self.kx[m];
            let (p, q) = self.cumulative(m, x, partial);
            let (sin_a, cos_a) = (kx * x).sin_cos();
            sum += self.weight[m] * (self.beta[m] * y).cos() * (p * cos_a + q * sin_a);
        }
        // 2 cos(β y) Re[conj(S) e^{i kx x}] = Re[conj(S) (e^{iψ+} + e^{iψ-})]
        -(4.0 * self.k0 / PI) * sum
    }

    /// `S(x; θ_m)` for sample `m`: the columns forward of `x`, and the forward part of `partial`.
    fn cumulative(&self, m: usize, x: f64, partial: Option<usize>) -> (f64, f64) {
        let columns = self.grid.xs.len() - 1;
        let row = &self.suffix[m * (columns + 1)..(m + 1) * (columns + 1)];
        match partial {
            None => row[0],
            Some(j) => {
                let zj = self.z[m * columns + j];
                let (re, im) = x_integral(x, self.grid.xs[j + 1], self.kx[m]);
                (row[j + 1].0 + zj * re, row[j + 1].1 + zj * im)
            }
        }
    }

    /// The largest wave angle kept (radians).
    pub fn theta_cut(&self) -> f64 {
        self.theta_cut
    }

    /// Wave-angle samples in the quadrature.
    pub fn samples(&self) -> usize {
        self.u.len()
    }

    /// `k0 = g / U²` (1/m).
    pub fn k0(&self) -> f64 {
        self.k0
    }

    /// Michell's `(P, Q)` of the whole hull (one demihull) at wave angle `theta` (radians).
    pub fn amplitude(&self, theta: f64) -> (f64, f64) {
        let sec = 1.0 / theta.cos();
        let mut z = Vec::new();
        column_z_integrals(&self.grid, self.k0 * sec * sec, &mut z);
        z.iter()
            .zip(self.grid.xs.windows(2))
            .map(|(zi, x)| {
                let (re, im) = x_integral(x[0], x[1], self.k0 * sec);
                (zi * re, zi * im)
            })
            .fold((0.0, 0.0), |a, b| (a.0 + b.0, a.1 + b.1))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::michell::wigley_grid;

    /// The ITTC Wigley hull at 10 m (B 1 m, T 0.625 m) on Michell's default 65 x 25 grid.
    fn wigley() -> MichellGrid {
        wigley_grid(10.0, 1.0, 0.625, 65, 25)
    }

    fn opts_at_fn(fn_: f64) -> WavePatternOptions {
        let g = crate::resistance::GRAVITY;
        WavePatternOptions::at_speed(fn_ * (g * 10.0).sqrt())
    }

    /// ζ by the same quadrature as [`WaveSpectrum::elevation`] but with the whole hull's
    /// P + iQ from [`WaveSpectrum::amplitude`] at every sample: Wu et al. (2019) eq. 14a as written.
    fn elevation_from_whole_hull(sp: &WaveSpectrum, x: f64, y: f64) -> f64 {
        let mut sum = 0.0;
        for m in 0..sp.samples() {
            let theta = sp.u[m].sinh().atan();
            let (p, q) = sp.amplitude(theta);
            let (sin_a, cos_a) = (sp.kx[m] * x).sin_cos();
            sum += sp.weight[m] * 2.0 * (sp.beta[m] * y).cos() * (p * cos_a + q * sin_a);
        }
        -(2.0 * sp.k0() / PI) * sum
    }

    /// Kelvin's far field, Wu et al. (2019) eqs. 13a, 13b, 15a-15d, at (x, y) behind the hull
    /// (x < 0, y >= 0, model x). Returns (ζ, |a_T| / k0, |a_D| / k0); a_D is 0 on the track.
    fn kelvin(sp: &WaveSpectrum, x: f64, y: f64) -> (f64, f64, f64) {
        let k0 = sp.k0();
        let h = k0 * x.hypot(y);
        let tan_psi = y / -x;
        let root = (1.0 - 8.0 * tan_psi * tan_psi).sqrt();
        let q_t = 2.0 * tan_psi / (1.0 + root);
        let term = |q: f64, quarter: f64| -> (f64, f64) {
            let q2 = q * q;
            let q_star = (1.0 + q2).sqrt();
            let phi = -(1.0 + q2) / (1.0 + 4.0 * q2).sqrt();
            let phi2 = (2.0 * q2 - 1.0) / ((1.0 + q2) * (1.0 + 4.0 * q2).sqrt());
            let (p, qq) = sp.amplitude(q.atan());
            // A/F^2 = -2 k0^2 (P - iQ)
            let (a_re, a_im) = (-2.0 * k0 * k0 * p, 2.0 * k0 * k0 * qq);
            let scale = (2.0 / PI).sqrt() * q_star / (phi2.abs().sqrt() * h.sqrt());
            let (s, c) = (h * phi + quarter).sin_cos();
            let re = scale * (a_re * c - a_im * s);
            (re / k0, scale * a_re.hypot(a_im) / k0)
        };
        let (zt, at) = term(q_t, -PI / 4.0);
        if tan_psi == 0.0 {
            return (zt, at, 0.0);
        }
        let (zd, ad) = term(1.0 / (2.0 * q_t), PI / 4.0);
        (zt + zd, at, ad)
    }

    #[test]
    fn nothing_ahead_of_the_bow() {
        let sp = wave_spectrum(&wigley(), &opts_at_fn(0.3), 20.0, 5.0, 0.5).unwrap();
        for x in [5.01, 6.0, 9.0] {
            for y in [0.0, 2.0, -3.0] {
                assert_eq!(sp.elevation(x, y), 0.0, "ζ({x}, {y}) ahead of the bow");
            }
        }
    }

    #[test]
    fn field_is_symmetric_port_and_starboard() {
        let sp = wave_spectrum(&wigley(), &opts_at_fn(0.3), 40.0, 15.0, 0.5).unwrap();
        let pts: Vec<(f64, f64)> = (0..20)
            .map(|k| (-5.5 - 1.37 * k as f64, 0.3 + 0.71 * k as f64))
            .collect();
        let max = pts
            .iter()
            .map(|&(x, y)| sp.elevation(x, y).abs())
            .fold(0.0, f64::max);
        assert!(max > 0.0);
        for (x, y) in pts {
            let d = (sp.elevation(x, y) - sp.elevation(x, -y)).abs();
            assert!(d <= 1e-12 * max, "ζ({x}, ±{y}) differ by {d}");
        }
    }

    #[test]
    fn behind_the_stern_every_row_sees_the_whole_hull() {
        let sp = wave_spectrum(&wigley(), &opts_at_fn(0.3), 20.0, 6.0, 0.5).unwrap();
        let pts = [-5.0 - 1e-9, -12.0];
        let ys = [0.0, 0.8, 2.5, 5.0];
        let mut max = 0.0_f64;
        let mut worst = 0.0_f64;
        for x in pts {
            for y in ys {
                let a = sp.elevation(x, y);
                let b = elevation_from_whole_hull(&sp, x, y);
                max = max.max(a.abs());
                worst = worst.max((a - b).abs());
            }
        }
        assert!(max > 0.0);
        assert!(
            worst <= 1e-10 * max,
            "cumulative vs whole hull: {worst:e} of {max:e}"
        );
    }

    /// Oracle §40: Wu et al. (2019) eq. 15 on the track (q_T = 0), 20 to 40 L behind.
    #[test]
    fn kelvin_far_field_on_the_track() {
        let sp = wave_spectrum(&wigley(), &opts_at_fn(0.3), 405.0, 0.0, 0.25).unwrap();
        let mut worst = 0.0_f64;
        for k in 0..200 {
            let x = -400.0 + 200.0 * k as f64 / 199.0;
            let z = sp.elevation(x, 0.0);
            let (zk, at, _) = kelvin(&sp, x, 0.0);
            let err = (z - zk).abs() / at;
            worst = worst.max(err);
            assert!(
                err <= 0.03,
                "x {x}: ζ {z:e} vs Kelvin {zk:e} ({:.2} % of |a_T|/k0)",
                err * 100.0
            );
        }
        eprintln!(
            "kelvin on the track: worst {:.3} % of |a_T|/k0",
            worst * 100.0
        );
    }

    /// Oracle §40: Wu et al. (2019) eqs. 15a-15d along the ray psi = 15 deg, both wave systems.
    #[test]
    fn kelvin_far_field_off_the_track() {
        let psi = 15.0_f64.to_radians();
        let k0 = opts_at_fn(0.3).g / opts_at_fn(0.3).speed_m_s.powi(2);
        let reach = 450.0 / k0;
        let sp = wave_spectrum(
            &wigley(),
            &opts_at_fn(0.3),
            reach * psi.cos() + 5.0,
            reach * psi.sin(),
            0.25,
        )
        .unwrap();
        let mut worst = 0.0_f64;
        for k in 0..200 {
            let h = 250.0 + 200.0 * k as f64 / 199.0;
            let (x, y) = (-h * psi.cos() / k0, h * psi.sin() / k0);
            let z = sp.elevation(x, y);
            let (zk, at, ad) = kelvin(&sp, x, y);
            let err = (z - zk).abs() / (at + ad);
            worst = worst.max(err);
            assert!(
                err <= 0.03,
                "h {h}: ζ {z:e} vs Kelvin {zk:e} ({:.2} %)",
                err * 100.0
            );
        }
        eprintln!(
            "kelvin at 15 deg: worst {:.3} % of (|a_T| + |a_D|)/k0",
            worst * 100.0
        );
    }

    /// Wu et al. (2019) eq. 10a at q = 0: transverse waves on the track are 2 pi U^2 / g long.
    #[test]
    fn transverse_wavelength_on_the_track() {
        let o = opts_at_fn(0.3);
        let sp = wave_spectrum(&wigley(), &o, 405.0, 0.0, 0.25).unwrap();
        let n = 4000;
        let xs: Vec<f64> = (0..=n)
            .map(|k| -400.0 + 200.0 * k as f64 / n as f64)
            .collect();
        let zs: Vec<f64> = xs.iter().map(|&x| sp.elevation(x, 0.0)).collect();
        let ups: Vec<f64> = (0..n)
            .filter(|&k| zs[k] < 0.0 && zs[k + 1] >= 0.0)
            .map(|k| xs[k] + (xs[k + 1] - xs[k]) * zs[k] / (zs[k] - zs[k + 1]))
            .collect();
        assert!(ups.len() > 20, "{} up-crossings", ups.len());
        let mean = (ups[ups.len() - 1] - ups[0]) / (ups.len() - 1) as f64;
        let lambda = 2.0 * PI * o.speed_m_s * o.speed_m_s / o.g;
        let rel = (mean - lambda).abs() / lambda;
        eprintln!(
            "transverse wavelength {mean:.5} m vs {lambda:.5} m ({:.4} %)",
            rel * 100.0
        );
        assert!(rel <= 0.005, "wavelength {mean} vs {lambda}");
    }

    #[test]
    fn bad_spectrum_inputs_are_refused() {
        let g = wigley();
        let mut o = opts_at_fn(0.3);
        o.speed_m_s = 0.0;
        assert!(wave_spectrum(&g, &o, 10.0, 1.0, 0.5)
            .unwrap_err()
            .contains("speed"));
        o.speed_m_s = f64::NAN;
        assert!(wave_spectrum(&g, &o, 10.0, 1.0, 0.5)
            .unwrap_err()
            .contains("speed"));
        let o = opts_at_fn(0.3);
        assert!(wave_spectrum(&g, &o, 10.0, 1.0, 0.0)
            .unwrap_err()
            .contains("spacing"));
        assert!(wave_spectrum(&g, &o, 10.0, 1.0, 50.0)
            .unwrap_err()
            .contains("spacing"));
        assert!(wave_spectrum(&g, &o, -1.0, 1.0, 0.5).is_err());
    }
}
