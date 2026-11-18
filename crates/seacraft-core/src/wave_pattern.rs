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

    /// `ζ(x, j dy)` for `j` in `0..count`, stepping the y-phase by a rotation per wave angle.
    fn elevations_along_y(&self, x: f64, dy: f64, count: usize) -> Vec<f64> {
        let xs = &self.grid.xs;
        let mut acc = vec![0.0; count];
        if x >= xs[xs.len() - 1] {
            return acc;
        }
        let partial = (x > xs[0]).then(|| xs.partition_point(|&s| s <= x) - 1);
        for m in 0..self.u.len() {
            let (p, q) = self.cumulative(m, x, partial);
            let (sin_a, cos_a) = (self.kx[m] * x).sin_cos();
            let c = self.weight[m] * (p * cos_a + q * sin_a);
            let (sin_b, cos_b) = (self.beta[m] * dy).sin_cos();
            let (mut re, mut im) = (1.0, 0.0);
            for a in acc.iter_mut() {
                *a += c * re;
                (re, im) = (re * cos_b - im * sin_b, re * sin_b + im * cos_b);
            }
        }
        let scale = -(4.0 * self.k0 / PI);
        acc.iter_mut().for_each(|a| *a *= scale);
        acc
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

/// The most grid points [`wave_field`] computes; a finer request is coarsened to fit.
pub const MAX_FIELD_POINTS: usize = 1_000_000;

/// The wave elevation on a plan-view grid behind (and beside) a hull.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveField {
    /// Grid x (m, model axes, ascending).
    pub xs: Vec<f64>,
    /// Grid y (m, ascending, symmetric about 0).
    pub ys: Vec<f64>,
    /// Elevation (m), row-major: `zeta[iy * xs.len() + ix]`; 0 where not wet.
    pub zeta: Vec<f64>,
    /// False inside a waterline.
    pub wet: Vec<bool>,
    /// Ship speed U (m/s).
    pub speed_m_s: f64,
    /// Froude number on the waterline length.
    pub froude: f64,
    /// `k0 = g / U²` (1/m).
    pub k0: f64,
    /// Transverse wavelength `2π U² / g` (m).
    pub transverse_wavelength_m: f64,
    /// Largest wave angle kept (degrees).
    pub theta_cut_deg: f64,
    /// Shortest wave kept (m).
    pub shortest_wavelength_m: f64,
    /// Wave-angle samples in the quadrature.
    pub theta_samples: usize,
    /// Largest |ζ| over wet points (m).
    pub max_abs_zeta_m: f64,
    /// What the field is and what was left out; the first is [`FAR_FIELD_NOTE`].
    pub notes: Vec<String>,
    /// Where the hull or speed is outside thin-ship theory.
    pub warnings: Vec<String>,
}

/// Where a field is computed: its axes, and the reach and spacing its spectrum is built for.
#[derive(Debug, Clone)]
struct FieldLayout {
    xs: Vec<f64>,
    /// `y >= 0` rows: `j * dy` for `j` in `0..=half_rows`.
    dy: f64,
    half_rows: usize,
    reach_x: f64,
    reach_y: f64,
    nyquist: f64,
    /// Set when the spacing was coarsened to stay within [`MAX_FIELD_POINTS`].
    coarsened: Option<(f64, f64)>,
}

fn field_layout(grid: &MichellGrid, opts: &WavePatternOptions) -> Result<FieldLayout, String> {
    grid.validate()?;
    opts.validate()?;
    let l = grid.length_wl;
    let (stern, bow) = (grid.xs[0], grid.xs[grid.xs.len() - 1]);
    let (x0, x1) = (stern - opts.behind_l * l, bow + opts.ahead_l * l);
    let half_width = opts.half_width_l * l;
    let wanted = opts
        .spacing_m
        .unwrap_or(2.0 * PI * opts.speed_m_s * opts.speed_m_s / opts.g / 20.0);
    let counts = |h: f64| {
        let nx = ((x1 - x0) / h).round().max(1.0) as usize + 1;
        let half_rows = (half_width / h).round() as usize;
        (nx, half_rows)
    };
    let mut h = wanted;
    let (mut nx, mut half_rows) = counts(h);
    while nx.saturating_mul(2 * half_rows + 1) > MAX_FIELD_POINTS {
        h *= ((nx * (2 * half_rows + 1)) as f64 / MAX_FIELD_POINTS as f64).sqrt() * 1.001;
        (nx, half_rows) = counts(h);
    }
    let dx = (x1 - x0) / (nx - 1) as f64;
    let dy = if half_rows > 0 {
        half_width / half_rows as f64
    } else {
        0.0
    };
    let xs = (0..nx)
        .map(|i| if i + 1 == nx { x1 } else { x0 + i as f64 * dx })
        .collect();
    Ok(FieldLayout {
        xs,
        dy,
        half_rows,
        reach_x: (x1 - stern).max(bow - x0),
        reach_y: half_width,
        nyquist: dx.max(dy),
        coarsened: (h != wanted).then_some((wanted, dx.max(dy))),
    })
}

/// The half-breadth of the calm waterline (`z = 0`) at `x`, 0 beyond the ends.
fn waterline_half_breadth(grid: &MichellGrid, x: f64) -> f64 {
    let xs = &grid.xs;
    if x < xs[0] || x > xs[xs.len() - 1] {
        return 0.0;
    }
    let top = grid.zs.len() - 1;
    let i = xs.partition_point(|&s| s <= x).clamp(1, xs.len() - 1) - 1;
    let t = (x - xs[i]) / (xs[i + 1] - xs[i]);
    grid.half_breadths[i][top] * (1.0 - t) + grid.half_breadths[i + 1][top] * t
}

/// The wave elevation of `grid` at `opts` on a plan-view grid: `behind_l` waterline lengths
/// behind the stern to `ahead_l` ahead of the bow, `half_width_l` each side.
pub fn wave_field(grid: &MichellGrid, opts: &WavePatternOptions) -> Result<WaveField, String> {
    let lay = field_layout(grid, opts)?;
    let sp = wave_spectrum(grid, opts, lay.reach_x, lay.reach_y, lay.nyquist)?;
    let nx = lay.xs.len();
    let rows = lay.half_rows;
    let columns = par_map(&lay.xs, |&x| sp.elevations_along_y(x, lay.dy, rows + 1));
    let ys: Vec<f64> = (0..=2 * rows)
        .map(|k| {
            let j = k.abs_diff(rows) as f64 * lay.dy;
            if k < rows {
                -j
            } else {
                j
            }
        })
        .collect();
    let ny = ys.len();
    let mut zeta = vec![0.0; nx * ny];
    let mut wet = vec![true; nx * ny];
    let mut max_abs = 0.0_f64;
    for (ix, &x) in lay.xs.iter().enumerate() {
        let half_breadth = waterline_half_breadth(grid, x);
        for (iy, &y) in ys.iter().enumerate() {
            let k = iy * nx + ix;
            if y.abs() < half_breadth {
                wet[k] = false;
            } else {
                zeta[k] = columns[ix][iy.abs_diff(rows)];
                max_abs = max_abs.max(zeta[k].abs());
            }
        }
    }

    let k0 = sp.k0();
    let theta_cut = sp.theta_cut();
    let shortest = 2.0 * PI * theta_cut.cos().powi(2) / k0;
    let theta_nyquist = (PI / (lay.nyquist * k0)).sqrt().recip().acos();
    let mut notes = vec![FAR_FIELD_NOTE.to_string()];
    notes.push(if theta_cut < theta_nyquist - 1e-9 {
        format!(
            "waves shorter than {shortest:.3} m (θ > {:.1}°) are left out: the hull makes almost none \
             (below {:e} of its spectrum's peak)",
            theta_cut.to_degrees(),
            opts.spectrum_floor
        )
    } else {
        format!(
            "waves shorter than {shortest:.3} m (θ > {:.1}°) are left out: the grid cannot show them",
            theta_cut.to_degrees()
        )
    });
    if let Some((from, to)) = lay.coarsened {
        notes.push(format!(
            "spacing coarsened from {from:.4} m to {to:.4} m to stay within 1 000 000 points"
        ));
    }
    let froude = opts.speed_m_s / (opts.g * grid.length_wl).sqrt();
    let mut warnings = crate::michell::michell_warnings(grid, &[]);
    if !(0.20..=1.0).contains(&froude) {
        warnings.push(format!(
            "Fn {froude:.3} at {:.2} kn is outside typical Michell applicability range (0.20-1.0)",
            opts.speed_m_s / crate::resistance::KNOT
        ));
    }
    Ok(WaveField {
        xs: lay.xs,
        ys,
        zeta,
        wet,
        speed_m_s: opts.speed_m_s,
        froude,
        k0,
        transverse_wavelength_m: 2.0 * PI / k0,
        theta_cut_deg: theta_cut.to_degrees(),
        shortest_wavelength_m: shortest,
        theta_samples: sp.samples(),
        max_abs_zeta_m: max_abs,
        notes,
        warnings,
    })
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

    /// Index of the axis value nearest `v`.
    fn nearest(axis: &[f64], v: f64) -> usize {
        (0..axis.len())
            .min_by(|&a, &b| (axis[a] - v).abs().total_cmp(&(axis[b] - v).abs()))
            .unwrap()
    }

    #[test]
    fn field_axes_and_mask() {
        let f = wave_field(&wigley(), &opts_at_fn(0.3)).unwrap();
        let (nx, ny) = (f.xs.len(), f.ys.len());
        assert!((f.xs[0] + 35.0).abs() < 1e-9 && (f.xs[nx - 1] - 15.0).abs() < 1e-9);
        assert!((f.ys[0] + 15.0).abs() < 1e-9 && (f.ys[ny - 1] - 15.0).abs() < 1e-9);
        for k in 0..ny {
            assert_eq!(f.ys[k], -f.ys[ny - 1 - k], "ys not symmetric at {k}");
        }
        assert_eq!(f.zeta.len(), nx * ny);
        assert_eq!(f.wet.len(), nx * ny);
        let (i0, j0, j1) = (
            nearest(&f.xs, 0.0),
            nearest(&f.ys, 0.0),
            nearest(&f.ys, 1.0),
        );
        assert!(!f.wet[j0 * nx + i0], "(0, 0) is inside the waterline");
        assert!(f.wet[j1 * nx + i0], "(0, 1) is outside the waterline");
        for (k, z) in f.zeta.iter().enumerate() {
            if f.xs[k % nx] > 5.0 {
                assert_eq!(*z, 0.0, "ζ ahead of the bow at x {}", f.xs[k % nx]);
            }
        }
        assert!(f.notes[0] == FAR_FIELD_NOTE);
        assert!((f.froude - 0.3).abs() < 1e-12);
    }

    #[test]
    fn field_matches_point_evaluation() {
        let g = wigley();
        let o = opts_at_fn(0.3);
        let f = wave_field(&g, &o).unwrap();
        let lay = field_layout(&g, &o).unwrap();
        let sp = wave_spectrum(&g, &o, lay.reach_x, lay.reach_y, lay.nyquist).unwrap();
        assert_eq!(sp.samples(), f.theta_samples);
        let (nx, ny) = (f.xs.len(), f.ys.len());
        let mut seed: u64 = 12345;
        let mut checked = 0;
        while checked < 10 {
            seed = seed
                .wrapping_mul(6364136223846793005)
                .wrapping_add(1442695040888963407);
            let k = (seed >> 33) as usize % (nx * ny);
            if !f.wet[k] {
                continue;
            }
            let want = sp.elevation(f.xs[k % nx], f.ys[k / nx]);
            assert!(
                (f.zeta[k] - want).abs() <= 1e-10 * f.max_abs_zeta_m,
                "grid {} vs point {want}",
                f.zeta[k]
            );
            checked += 1;
        }
    }

    #[test]
    fn field_is_converged_in_theta() {
        let g = wigley();
        let o = opts_at_fn(0.3);
        let a = wave_field(&g, &o).unwrap();
        let fine = WavePatternOptions {
            phase_step: o.phase_step / 2.0,
            ..o
        };
        let b = wave_field(&g, &fine).unwrap();
        assert!(b.theta_samples > a.theta_samples);
        let worst = a
            .zeta
            .iter()
            .zip(&b.zeta)
            .map(|(p, q)| (p - q).abs())
            .fold(0.0, f64::max);
        eprintln!(
            "halving the phase step moves ζ by {:.4} % of max",
            100.0 * worst / a.max_abs_zeta_m
        );
        assert!(worst <= 0.01 * a.max_abs_zeta_m);
    }

    /// Review focus 1: at Fn 0.10 the default spacing (λ_T / 20) would need about 1.5 million points.
    #[test]
    fn low_froude_field_is_capped() {
        let f = wave_field(&wigley(), &opts_at_fn(0.10)).unwrap();
        assert!(f.xs.len() * f.ys.len() <= MAX_FIELD_POINTS);
        assert!(
            f.notes.iter().any(|n| n.contains("coarsened")),
            "{:?}",
            f.notes
        );
        assert!(
            f.warnings.iter().any(|w| w.contains("Fn 0.100")),
            "{:?}",
            f.warnings
        );
    }

    /// Review focus 4.
    #[test]
    fn bad_inputs_are_refused() {
        let g = wigley();
        let with = |edit: &dyn Fn(&mut WavePatternOptions)| {
            let mut o = opts_at_fn(0.3);
            edit(&mut o);
            wave_field(&g, &o).unwrap_err()
        };
        assert!(with(&|o| o.speed_m_s = 0.0).contains("speed"));
        assert!(with(&|o| o.speed_m_s = f64::NAN).contains("speed"));
        assert!(with(&|o| o.behind_l = -1.0).contains("behind"));
        assert!(with(&|o| o.spacing_m = Some(0.0)).contains("spacing"));
    }
}
