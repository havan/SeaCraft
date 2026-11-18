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

/// The most memory a stored [`WaveSpectrum`] may take; a larger reach or a finer spacing is refused.
pub const MAX_SPECTRUM_BYTES: usize = 256 << 20;

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
    theta_nyquist: f64,
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
    let quad = Quadrature::new(grid, opts, reach_x_m, reach_y_m, nyquist_spacing_m)?;
    let columns = grid.xs.len() - 1;
    // Per sample: Z_i (8 bytes a column) and the suffix sums (16 bytes, one more than columns).
    let bytes = quad.len().saturating_mul(columns * 8 + (columns + 1) * 16);
    if bytes > MAX_SPECTRUM_BYTES {
        return Err(format!(
            "the spectrum needs {} wave-angle samples ({} MB, more than {} MB): make the domain or the \
             cut smaller, or its spacing coarser",
            quad.len(),
            bytes >> 20,
            MAX_SPECTRUM_BYTES >> 20
        ));
    }
    let rows = par_map(&(0..quad.len()).collect::<Vec<_>>(), |&m| {
        let smp = quad.sample(m);
        let mut z = Vec::with_capacity(columns);
        column_z_integrals(grid, smp.kz, &mut z);
        let mut suffix = vec![(0.0, 0.0); columns + 1];
        for i in (0..columns).rev() {
            let (re, im) = x_integral(grid.xs[i], grid.xs[i + 1], smp.kx);
            suffix[i] = (suffix[i + 1].0 + z[i] * re, suffix[i + 1].1 + z[i] * im);
        }
        (smp, z, suffix)
    });

    let n = rows.len();
    let mut sp = WaveSpectrum {
        k0: quad.k0,
        grid: grid.clone(),
        u: Vec::with_capacity(n),
        weight: Vec::with_capacity(n),
        kx: Vec::with_capacity(n),
        beta: Vec::with_capacity(n),
        z: Vec::with_capacity(n * columns),
        suffix: Vec::with_capacity(n * (columns + 1)),
        theta_cut: quad.theta_cut(),
        theta_nyquist: quad.theta_nyquist,
    };
    for (smp, z, suffix) in rows {
        sp.u.push(smp.u);
        sp.weight.push(smp.weight);
        sp.kx.push(smp.kx);
        sp.beta.push(smp.beta);
        sp.z.extend(z);
        sp.suffix.extend(suffix);
    }
    Ok(sp)
}

/// The wave-angle quadrature for field points within a reach, keeping waves no shorter than two
/// spacings: trapezoid samples of `u` on `[0, u_c]` (`sec θ = cosh u`).
#[derive(Debug, Clone, Copy)]
struct Quadrature {
    k0: f64,
    du: f64,
    intervals: usize,
    theta_nyquist: f64,
    catamaran: Option<f64>,
}

/// One sample of the quadrature.
#[derive(Debug, Clone, Copy)]
struct Sample {
    u: f64,
    /// Trapezoid weight times `cosh²u`, times the catamaran factor.
    weight: f64,
    /// `k0 sec θ`.
    kx: f64,
    /// `k0 sec²θ`: the decay rate with depth.
    kz: f64,
    /// `k0 sec θ tan θ`.
    beta: f64,
}

impl Quadrature {
    fn new(
        grid: &MichellGrid,
        opts: &WavePatternOptions,
        reach_x_m: f64,
        reach_y_m: f64,
        nyquist_spacing_m: f64,
    ) -> Result<Self, String> {
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
        if let Some(s) = opts.catamaran_spacing.filter(|&s| s < grid.beam_wl) {
            return Err(format!(
                "catamaran spacing {s} m is less than the demihull beam {} m: the demihulls would overlap",
                grid.beam_wl
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
        Ok(Self {
            k0,
            du: u_c / intervals as f64,
            intervals,
            theta_nyquist: u_nyquist.sinh().atan(),
            catamaran: opts.catamaran_spacing,
        })
    }

    fn len(&self) -> usize {
        self.intervals + 1
    }

    fn theta_cut(&self) -> f64 {
        (self.du * self.intervals as f64).sinh().atan()
    }

    fn sample(&self, m: usize) -> Sample {
        let u = m as f64 * self.du;
        let (ch, sh) = (u.cosh(), u.sinh());
        let beta = self.k0 * ch * sh;
        let end = if m == 0 || m == self.intervals {
            0.5
        } else {
            1.0
        };
        let mut weight = end * self.du * ch * ch;
        if let Some(s) = self.catamaran {
            weight *= 2.0 * (beta * s / 2.0).cos();
        }
        Sample {
            u,
            weight,
            kx: self.k0 * ch,
            kz: self.k0 * ch * ch,
            beta,
        }
    }
}

/// Add `c cos(beta y)` at `y = y0 + j dy` to `acc[j]`, stepping the phase by a rotation.
fn accumulate_along_y(acc: &mut [f64], c: f64, beta: f64, y0: f64, dy: f64) {
    let (sin_b, cos_b) = (beta * dy).sin_cos();
    let (mut im, mut re) = (beta * y0).sin_cos();
    for a in acc.iter_mut() {
        *a += c * re;
        (re, im) = (re * cos_b - im * sin_b, re * sin_b + im * cos_b);
    }
}

/// The column of `xs` holding `x` when `x` is alongside the hull (`None` behind the stern).
fn partial_column(xs: &[f64], x: f64) -> Option<usize> {
    (x > xs[0]).then(|| xs.partition_point(|&s| s <= x) - 1)
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
        let partial = partial_column(xs, x);
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
        let partial = partial_column(xs, x);
        for m in 0..self.u.len() {
            let (p, q) = self.cumulative(m, x, partial);
            let (sin_a, cos_a) = (self.kx[m] * x).sin_cos();
            let c = self.weight[m] * (p * cos_a + q * sin_a);
            accumulate_along_y(&mut acc, c, self.beta[m], 0.0, dy);
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
    /// The calm waterline of each hull (one, or two demihulls) as a closed outline of `[x, y]` points
    /// (m): starboard side from stern to bow, then port side back.
    pub waterlines: Vec<Vec<[f64; 2]>>,
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
    if !(wanted.is_finite() && wanted > 0.0) {
        return Err(format!(
            "speed {} m/s is too low for a wave grid: its waves are shorter than a float can space",
            opts.speed_m_s
        ));
    }
    // Counts are clamped before the cast so that a tiny spacing cannot overflow; the loop below
    // then coarsens it to the point cap.
    const MAX_COUNT: f64 = 1e9;
    let counts = |h: f64| {
        let nx = ((x1 - x0) / h).round().clamp(1.0, MAX_COUNT) as usize + 1;
        let half_rows = (half_width / h).round().clamp(0.0, MAX_COUNT) as usize;
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

/// The calm waterline (`z = 0`) of the hull, or of both demihulls at `±s/2`, as closed outlines.
fn waterline_outlines(grid: &MichellGrid, catamaran: Option<f64>) -> Vec<Vec<[f64; 2]>> {
    let top = grid.zs.len() - 1;
    let side: Vec<(f64, f64)> = grid
        .xs
        .iter()
        .zip(&grid.half_breadths)
        .map(|(&x, col)| (x, col[top]))
        .collect();
    let centres = match catamaran {
        Some(s) => vec![s / 2.0, -s / 2.0],
        None => vec![0.0],
    };
    centres
        .into_iter()
        .map(|c| {
            let starboard = side.iter().map(|&(x, h)| [x, c + h]);
            let port = side.iter().rev().map(|&(x, h)| [x, c - h]);
            starboard.chain(port).collect()
        })
        .collect()
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

/// The shortest wave kept when wave angles above `theta_cut` are left out (m).
fn shortest_wavelength(k0: f64, theta_cut: f64) -> f64 {
    2.0 * PI * theta_cut.cos().powi(2) / k0
}

/// What the wave-angle cut of `sp` left out, and why.
fn cut_off_note(sp: &WaveSpectrum, floor: f64) -> String {
    cut_off_note_for(sp.k0, sp.theta_cut, sp.theta_nyquist, floor)
}

fn cut_off_note_for(k0: f64, theta: f64, theta_nyquist: f64, floor: f64) -> String {
    let shortest = shortest_wavelength(k0, theta);
    if theta < theta_nyquist - 1e-9 {
        format!(
            "waves shorter than {shortest:.3} m (θ > {:.1}°) are left out: the hull makes almost none \
             (below {floor:e} of its spectrum's peak)",
            theta.to_degrees()
        )
    } else {
        format!(
            "waves shorter than {shortest:.3} m (θ > {:.1}°) are left out: the grid cannot show them",
            theta.to_degrees()
        )
    }
}

/// Whether `y` is inside a waterline of the given half-breadth: one hull on the centreline, or
/// two demihulls at `±s/2`.
fn inside_waterline(half_breadth: f64, catamaran: Option<f64>, y: f64) -> bool {
    let off = catamaran.map_or(y.abs(), |s| (y.abs() - s / 2.0).abs());
    off < half_breadth
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
            if inside_waterline(half_breadth, opts.catamaran_spacing, y) {
                wet[k] = false;
            } else {
                zeta[k] = columns[ix][iy.abs_diff(rows)];
                max_abs = max_abs.max(zeta[k].abs());
            }
        }
    }

    let k0 = sp.k0();
    let theta_cut = sp.theta_cut();
    let shortest = shortest_wavelength(k0, theta_cut);
    let mut notes = vec![
        FAR_FIELD_NOTE.to_string(),
        cut_off_note(&sp, opts.spectrum_floor),
    ];
    if opts.catamaran_spacing.is_some() {
        notes.push(
            "catamaran: each demihull is taken as symmetric about its own centreplane, as in \
             Michell's resistance"
                .to_string(),
        );
    }
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
        waterlines: waterline_outlines(grid, opts.catamaran_spacing),
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

/// A straight line in plan view along which to sample the waves.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum CutLine {
    /// Along x at a fixed y (m).
    Longitudinal { y: f64 },
    /// Along y at a fixed x (m).
    Transverse { x: f64 },
}

/// The wave elevation along a cut, sampled from the spectrum (not read off a field grid).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WaveCut {
    pub line: CutLine,
    /// The coordinate along the line (m): x for a longitudinal cut, y for a transverse one.
    pub s: Vec<f64>,
    /// Elevation (m); 0 inside a waterline.
    pub zeta: Vec<f64>,
    /// Largest wave angle kept (degrees).
    pub theta_cut_deg: f64,
    /// What the cut is and what was left out; the first is [`FAR_FIELD_NOTE`].
    pub notes: Vec<String>,
}

/// The waves along `line` from `from` to `to` (m, `from < to`), at `samples` evenly spaced points.
/// The spectrum is built for this cut alone: its reach from the cut's end points, its short-wave cut
/// from the sample spacing.
pub fn wave_cut(
    grid: &MichellGrid,
    opts: &WavePatternOptions,
    line: CutLine,
    from: f64,
    to: f64,
    samples: usize,
) -> Result<WaveCut, String> {
    grid.validate()?;
    if !(from.is_finite() && to.is_finite() && from < to) {
        return Err(format!(
            "a cut runs from a smaller to a larger finite coordinate, got {from} to {to}"
        ));
    }
    if samples < 2 {
        return Err(format!("a cut needs at least 2 samples, got {samples}"));
    }
    let ds = (to - from) / (samples - 1) as f64;
    let s: Vec<f64> = (0..samples)
        .map(|k| {
            if k + 1 == samples {
                to
            } else {
                from + k as f64 * ds
            }
        })
        .collect();
    let (stern, bow) = (grid.xs[0], grid.xs[grid.xs.len() - 1]);
    let reach_from = |x: f64| (x - stern).abs().max((x - bow).abs());
    let (zeta, theta_cut, note) = match line {
        CutLine::Longitudinal { y } => {
            if !y.is_finite() {
                return Err(format!("the cut's y must be a finite number, got {y}"));
            }
            let reach_x = reach_from(from).max(reach_from(to));
            let sp = wave_spectrum(grid, opts, reach_x, y.abs(), ds)?;
            let zeta = par_map(&s, |&x| {
                let dry =
                    inside_waterline(waterline_half_breadth(grid, x), opts.catamaran_spacing, y);
                if dry {
                    0.0
                } else {
                    sp.elevation(x, y)
                }
            });
            (zeta, sp.theta_cut, cut_off_note(&sp, opts.spectrum_floor))
        }
        CutLine::Transverse { x } => {
            if !x.is_finite() {
                return Err(format!("the cut's x must be a finite number, got {x}"));
            }
            let quad = Quadrature::new(grid, opts, reach_from(x), from.abs().max(to.abs()), ds)?;
            let mut zeta = transverse_values(grid, &quad, x, from, ds, samples);
            let half_breadth = waterline_half_breadth(grid, x);
            for (z, &y) in zeta.iter_mut().zip(&s) {
                if inside_waterline(half_breadth, opts.catamaran_spacing, y) {
                    *z = 0.0;
                }
            }
            let theta_cut = quad.theta_cut();
            let note =
                cut_off_note_for(quad.k0, theta_cut, quad.theta_nyquist, opts.spectrum_floor);
            (zeta, theta_cut, note)
        }
    };
    Ok(WaveCut {
        line,
        s,
        zeta,
        theta_cut_deg: theta_cut.to_degrees(),
        notes: vec![FAR_FIELD_NOTE.to_string(), note],
    })
}

/// `ζ(x, y0 + j dy)` for `j` in `0..count`, summed sample by sample without storing the spectrum:
/// a transverse cut sees one partial column, and a fine cut needs many wave-angle samples.
fn transverse_values(
    grid: &MichellGrid,
    quad: &Quadrature,
    x: f64,
    y0: f64,
    dy: f64,
    count: usize,
) -> Vec<f64> {
    let xs = &grid.xs;
    let columns = xs.len() - 1;
    if x >= xs[columns] {
        return vec![0.0; count];
    }
    let partial = partial_column(xs, x);
    let first = partial.map_or(0, |j| j + 1);
    let n = quad.len();
    let chunk = n.div_ceil(256).max(1);
    let starts: Vec<usize> = (0..n).step_by(chunk).collect();
    let parts = par_map(&starts, |&a| {
        let mut acc = vec![0.0; count];
        let mut z = Vec::with_capacity(columns);
        for m in a..(a + chunk).min(n) {
            let smp = quad.sample(m);
            column_z_integrals(grid, smp.kz, &mut z);
            // Bow to stern, the order of WaveSpectrum's suffix sums.
            let (mut p, mut q) = (0.0, 0.0);
            for i in (first..columns).rev() {
                let (re, im) = x_integral(xs[i], xs[i + 1], smp.kx);
                (p, q) = (p + z[i] * re, q + z[i] * im);
            }
            if let Some(j) = partial {
                let (re, im) = x_integral(x, xs[j + 1], smp.kx);
                (p, q) = (p + z[j] * re, q + z[j] * im);
            }
            let (sin_a, cos_a) = (smp.kx * x).sin_cos();
            accumulate_along_y(
                &mut acc,
                smp.weight * (p * cos_a + q * sin_a),
                smp.beta,
                y0,
                dy,
            );
        }
        acc
    });
    let scale = -(4.0 * quad.k0 / PI);
    let mut out = vec![0.0; count];
    for part in parts {
        for (o, v) in out.iter_mut().zip(part) {
            *o += v;
        }
    }
    out.iter_mut().for_each(|o| *o *= scale);
    out
}

/// The diverging colour map: trough, calm, crest. The GUI builds its theme tokens from these, so
/// the command line's images and the window match.
pub const DIVERGING_STOPS: [[u8; 3]; 3] = [[33, 102, 172], [247, 247, 247], [178, 24, 43]];

/// Inside a waterline.
pub const HULL_RGB: [u8; 3] = [90, 90, 90];

/// Contour levels each side of calm in [`wave_svg`].
const SVG_LEVELS: usize = 6;

/// The colour of `t` in `[-1, 1]` (clamped; NaN is calm), linear between [`DIVERGING_STOPS`].
pub fn diverging_rgb(t: f64) -> [u8; 3] {
    if t.is_nan() {
        return DIVERGING_STOPS[1];
    }
    let t = t.clamp(-1.0, 1.0);
    let (from, to, w) = if t < 0.0 {
        (DIVERGING_STOPS[1], DIVERGING_STOPS[0], -t)
    } else {
        (DIVERGING_STOPS[1], DIVERGING_STOPS[2], t)
    };
    std::array::from_fn(|i| (from[i] as f64 + (to[i] as f64 - from[i] as f64) * w).round() as u8)
}

/// `count` levels each side of calm: `±k clip / count` for `k` in `1..=count`, ascending, no 0.
pub fn contour_levels(clip_m: f64, count: usize) -> Vec<f64> {
    let step = clip_m / count as f64;
    let mut levels: Vec<f64> = (1..=count).rev().map(|k| -(k as f64) * step).collect();
    levels.extend((1..=count).map(|k| k as f64 * step));
    levels
}

/// The colour clip: `clip_m` when given and positive, else the field's largest |ζ|.
fn clip_of(field: &WaveField, clip_m: Option<f64>) -> f64 {
    clip_m
        .filter(|c| c.is_finite() && *c > 0.0)
        .unwrap_or(field.max_abs_zeta_m)
}

/// The field as RGBA, one pixel per grid point, row 0 at the largest y (starboard up, bow right).
pub fn wave_rgba(field: &WaveField, clip_m: Option<f64>) -> Vec<u8> {
    let (nx, ny) = (field.xs.len(), field.ys.len());
    let clip = clip_of(field, clip_m);
    let mut rgba = Vec::with_capacity(nx * ny * 4);
    for row in 0..ny {
        let iy = ny - 1 - row;
        for ix in 0..nx {
            let k = iy * nx + ix;
            let rgb = if !field.wet[k] {
                HULL_RGB
            } else if clip > 0.0 {
                diverging_rgb(field.zeta[k] / clip)
            } else {
                DIVERGING_STOPS[1]
            };
            rgba.extend_from_slice(&rgb);
            rgba.push(255);
        }
    }
    rgba
}

/// The field as a PNG, one pixel per grid point; `None` for an empty field.
pub fn wave_png(field: &WaveField, clip_m: Option<f64>) -> Option<Vec<u8>> {
    crate::raster::png(field.xs.len(), field.ys.len(), &wave_rgba(field, clip_m))
}

/// The field as an SVG contour map in metres (bow to the right, starboard up): crests solid,
/// troughs dashed, the hull filled, with the speed, Froude number, clip and notes.
pub fn wave_svg(field: &WaveField, clip_m: Option<f64>) -> String {
    use std::fmt::Write;
    let (nx, ny) = (field.xs.len(), field.ys.len());
    let mut out = String::new();
    if nx == 0 || ny == 0 {
        return out;
    }
    let clip = clip_of(field, clip_m);
    let (x0, x1) = (field.xs[0], field.xs[nx - 1]);
    let (y0, y1) = (field.ys[0], field.ys[ny - 1]);
    let (w, h) = (x1 - x0, y1 - y0);
    // The drawing is in metres: strokes, dashes and text are sized from the domain.
    let stroke = 0.0015 * w.max(h);
    let font = 0.018 * w;
    let rgb = |c: [u8; 3]| format!("rgb({},{},{})", c[0], c[1], c[2]);
    let title = format!(
        "Wave pattern at {:.3} m/s ({:.2} kn), Fn {:.3}, colour clip ±{:.4} m",
        field.speed_m_s,
        field.speed_m_s / crate::resistance::KNOT,
        field.froude,
        clip
    );
    let notes: Vec<&String> = field.notes.iter().chain(&field.warnings).collect();
    let max_chars = ((w / (0.6 * font)) as usize).saturating_sub(2).max(20);
    let lines: Vec<String> = std::iter::once(&title)
        .chain(notes.iter().copied())
        .flat_map(|t| wrap(t, max_chars))
        .collect();
    let caption = font * 1.5 * (lines.len() + 1) as f64;
    let _ = writeln!(
        out,
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<svg xmlns=\"http://www.w3.org/2000/svg\" \
         viewBox=\"{x0:.3} {:.3} {w:.3} {:.3}\">",
        -y1,
        h + caption
    );
    let _ = writeln!(out, "<title>{}</title>", crate::sheet::xml(&title));
    let desc: Vec<String> = notes.iter().map(|n| crate::sheet::xml(n)).collect();
    let _ = writeln!(out, "<desc>{}</desc>", desc.join("\n"));
    let _ = writeln!(
        out,
        "<rect x=\"{x0:.3}\" y=\"{:.3}\" width=\"{w:.3}\" height=\"{h:.3}\" fill=\"{}\"/>",
        -y1,
        rgb(DIVERGING_STOPS[1])
    );
    // SVG's y runs down: a model point (x, y) is drawn at (x, -y).
    for level in contour_levels(clip, SVG_LEVELS) {
        let segs = crate::contour::segments(&field.xs, &field.ys, &field.zeta, &field.wet, level);
        if segs.is_empty() {
            continue;
        }
        let mut d = String::new();
        for [a, b] in segs {
            let _ = write!(d, "M{:.3} {:.3}L{:.3} {:.3}", a.0, -a.1, b.0, -b.1);
        }
        let style = if level > 0.0 {
            format!("stroke=\"{}\"", rgb(DIVERGING_STOPS[2]))
        } else {
            format!(
                "stroke=\"{}\" stroke-dasharray=\"{:.4} {:.4}\"",
                rgb(DIVERGING_STOPS[0]),
                6.0 * stroke,
                4.0 * stroke
            )
        };
        let _ = writeln!(
            out,
            "<path d=\"{d}\" fill=\"none\" {style} stroke-width=\"{stroke:.4}\">\
             <title>ζ = {level:.4} m</title></path>"
        );
    }
    // The hull: each calm waterline as a filled outline.
    let mut hull = String::new();
    for outline in &field.waterlines {
        for (k, [x, y]) in outline.iter().enumerate() {
            let _ = write!(hull, "{}{x:.3} {:.3}", if k == 0 { "M" } else { "L" }, -y);
        }
        hull.push('Z');
    }
    if !hull.is_empty() {
        let _ = writeln!(out, "<path d=\"{hull}\" fill=\"{}\"/>", rgb(HULL_RGB));
    }
    let _ = writeln!(
        out,
        "<g font-family=\"DejaVu Sans, Verdana, sans-serif\" font-size=\"{font:.3}\" fill=\"black\">"
    );
    let mut y = -y0 + 1.5 * font;
    for line in &lines {
        let _ = writeln!(
            out,
            "<text x=\"{x0:.3}\" y=\"{y:.3}\">{}</text>",
            crate::sheet::xml(line)
        );
        y += 1.5 * font;
    }
    out.push_str("</g>\n</svg>\n");
    out
}

/// `text` broken at spaces into lines of at most `max` characters (a longer word stays whole).
fn wrap(text: &str, max: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > max {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

/// The field as VTK XML image data (`.vti`): point data `zeta` (m) and `wet` (1 or 0), x fastest.
pub fn wave_vti(field: &WaveField) -> String {
    use std::fmt::Write;
    let (nx, ny) = (field.xs.len(), field.ys.len());
    let step = |a: &[f64]| {
        if a.len() > 1 {
            (a[a.len() - 1] - a[0]) / (a.len() - 1) as f64
        } else {
            1.0
        }
    };
    let extent = format!("0 {} 0 {} 0 0", nx.saturating_sub(1), ny.saturating_sub(1));
    let mut out = String::new();
    let _ = writeln!(
        out,
        "<?xml version=\"1.0\"?>\n<VTKFile type=\"ImageData\" version=\"0.1\" byte_order=\"LittleEndian\">\n\
         <ImageData WholeExtent=\"{extent}\" Origin=\"{} {} 0\" Spacing=\"{} {} 1\">\n\
         <Piece Extent=\"{extent}\">\n<PointData Scalars=\"zeta\">\n\
         <DataArray type=\"Float64\" Name=\"zeta\" format=\"ascii\">",
        field.xs.first().copied().unwrap_or(0.0),
        field.ys.first().copied().unwrap_or(0.0),
        step(&field.xs),
        step(&field.ys)
    );
    for row in field.zeta.chunks(nx.max(1)) {
        let line: Vec<String> = row.iter().map(|z| format!("{z:e}")).collect();
        let _ = writeln!(out, "{}", line.join(" "));
    }
    out.push_str("</DataArray>\n<DataArray type=\"UInt8\" Name=\"wet\" format=\"ascii\">\n");
    for row in field.wet.chunks(nx.max(1)) {
        let line: Vec<&str> = row.iter().map(|&w| if w { "1" } else { "0" }).collect();
        let _ = writeln!(out, "{}", line.join(" "));
    }
    out.push_str("</DataArray>\n</PointData>\n</Piece>\n</ImageData>\n</VTKFile>\n");
    out
}

/// A field's numbers without its grid, for a JSON reply: every field of [`WaveField`] except `xs`,
/// `ys`, `zeta` and `wet`, plus `grid_points` `[nx, ny]`.
pub fn summary_json(field: &WaveField) -> serde_json::Value {
    let mut v = serde_json::to_value(field).unwrap_or_default();
    if let Some(map) = v.as_object_mut() {
        for key in ["xs", "ys", "zeta", "wet", "waterlines"] {
            map.remove(key);
        }
        map.insert(
            "grid_points".into(),
            serde_json::json!([field.xs.len(), field.ys.len()]),
        );
    }
    v
}

/// The wave pattern as the MCP tool `wave_pattern` and the window's `waves.compute` take it:
/// `speed` (knots) or `fn` (exactly one), `draft`, `cat_spacing`, `ahead`, `behind`, `half_width`
/// (waterline lengths), `spacing`, `clip` (metres), `png_path`, `svg_path`, `vti_path`. Every
/// argument and path is checked before any work, and each path passes `check_path` (the window's
/// write roots) before anything is written. Returns [`summary_json`] with `draft`, `cat_spacing` and
/// `files`; errors name the argument.
pub fn wave_tool(
    surface: &crate::mesh::Surface,
    subdiv: crate::subdiv::SubdivisionOptions,
    design_draft: f64,
    args: &serde_json::Map<String, serde_json::Value>,
    check_path: &dyn Fn(&std::path::Path) -> Result<std::path::PathBuf, String>,
) -> Result<serde_json::Value, String> {
    use serde_json::Value;
    let number = |key: &str| -> Result<Option<f64>, String> {
        match args.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .filter(|x| x.is_finite())
                .map(Some)
                .ok_or_else(|| format!("`{key}` must be a number")),
        }
    };
    let positive = |key: &str| -> Result<Option<f64>, String> {
        match number(key)? {
            Some(x) if x <= 0.0 => Err(format!("`{key}` must be above 0, got {x}")),
            other => Ok(other),
        }
    };
    let not_negative = |key: &str, default: f64| -> Result<f64, String> {
        match number(key)? {
            Some(x) if x < 0.0 => Err(format!("`{key}` must be 0 or more, got {x}")),
            other => Ok(other.unwrap_or(default)),
        }
    };
    let speed_kn = positive("speed")?;
    let froude = positive("fn")?;
    let draft = positive("draft")?.unwrap_or(design_draft);
    let defaults = WavePatternOptions::at_speed(1.0);
    let ahead = not_negative("ahead", defaults.ahead_l)?;
    let behind = not_negative("behind", defaults.behind_l)?;
    let half_width = not_negative("half_width", defaults.half_width_l)?;
    let spacing = positive("spacing")?;
    let cat_spacing = positive("cat_spacing")?;
    let clip = positive("clip")?;
    let mut outputs = Vec::new();
    for key in ["png_path", "svg_path", "vti_path"] {
        match args.get(key) {
            None | Some(Value::Null) => {}
            Some(Value::String(p)) if !p.trim().is_empty() => {
                let path = check_path(std::path::Path::new(p.trim()))?;
                outputs.push((key, path));
            }
            Some(_) => return Err(format!("`{key}` must be a file path")),
        }
    }
    // Two outputs written to one file would leave only the last, while both are reported.
    let landed: Vec<std::path::PathBuf> = outputs
        .iter()
        .map(|(_, p)| {
            crate::control_auth::canonicalize_write_target(p).unwrap_or_else(|_| p.clone())
        })
        .collect();
    for (i, (key, path)) in outputs.iter().enumerate() {
        if let Some(j) = landed[..i].iter().position(|q| *q == landed[i]) {
            return Err(format!(
                "`{}` and `{key}` name the same file, `{}`: each output needs a file of its own",
                outputs[j].0,
                path.display()
            ));
        }
    }
    if speed_kn.is_some() == froude.is_some() {
        return Err("give exactly one of `speed` (knots) and `fn`".into());
    }

    let grid = crate::michell::michell_grid_from_surface(surface, draft, 65, 25, subdiv).ok_or(
        "cannot extract Michell offsets grid from the hull (positive draft and waterline needed)",
    )?;
    let speed = match (speed_kn, froude) {
        (Some(kn), _) => kn * crate::resistance::KNOT,
        (None, Some(f)) => f * (crate::resistance::GRAVITY * grid.length_wl).sqrt(),
        (None, None) => unreachable!("checked above"),
    };
    let opts = WavePatternOptions {
        ahead_l: ahead,
        behind_l: behind,
        half_width_l: half_width,
        spacing_m: spacing,
        catamaran_spacing: cat_spacing,
        ..WavePatternOptions::at_speed(speed)
    };
    let field = wave_field(&grid, &opts)?;
    let mut files = Vec::new();
    for (key, path) in outputs {
        let bytes = match key {
            "png_path" => wave_png(&field, clip).ok_or("the wave field is empty")?,
            "svg_path" => wave_svg(&field, clip).into_bytes(),
            _ => wave_vti(&field).into_bytes(),
        };
        crate::io::write_atomic(&path, bytes)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        files.push(path.display().to_string());
    }
    let mut reply = summary_json(&field);
    if let Some(map) = reply.as_object_mut() {
        map.insert("draft".into(), serde_json::json!(grid.draft));
        map.insert("cat_spacing".into(), serde_json::json!(cat_spacing));
        map.insert("files".into(), serde_json::json!(files));
    }
    Ok(reply)
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
            let Err(err) = wave_field(&g, &o) else {
                panic!("{o:?} was accepted");
            };
            err
        };
        assert!(with(&|o| o.speed_m_s = 0.0).contains("speed"));
        assert!(with(&|o| o.speed_m_s = f64::NAN).contains("speed"));
        assert!(with(&|o| o.behind_l = -1.0).contains("behind"));
        assert!(with(&|o| o.spacing_m = Some(0.0)).contains("spacing"));
    }

    fn cat_opts(fn_: f64, s: f64) -> WavePatternOptions {
        WavePatternOptions {
            catamaran_spacing: Some(s),
            ..opts_at_fn(fn_)
        }
    }

    #[test]
    fn catamaran_is_two_shifted_demihulls() {
        let (g, s) = (wigley(), 3.0);
        let cat = wave_spectrum(&g, &cat_opts(0.3, s), 30.0, 10.0, 0.5).unwrap();
        let mono = wave_spectrum(&g, &opts_at_fn(0.3), 30.0, 10.0 + s / 2.0, 0.5).unwrap();
        assert_eq!(cat.samples(), mono.samples());
        let pts: Vec<(f64, f64)> = (0..20)
            .map(|k| (-5.5 - 1.1 * k as f64, -4.0 + 0.45 * k as f64))
            .collect();
        let max = pts
            .iter()
            .map(|&(x, y)| cat.elevation(x, y).abs())
            .fold(0.0, f64::max);
        assert!(max > 0.0);
        for (x, y) in pts {
            let two = mono.elevation(x, y - s / 2.0) + mono.elevation(x, y + s / 2.0);
            let d = (cat.elevation(x, y) - two).abs();
            assert!(
                d <= 1e-10 * max,
                "({x}, {y}): catamaran vs two demihulls differ by {d:e}"
            );
        }
    }

    #[test]
    fn catamaran_masks_both_demihulls() {
        let f = wave_field(&wigley(), &cat_opts(0.3, 3.0)).unwrap();
        let nx = f.xs.len();
        let i0 = nearest(&f.xs, 0.0);
        for (y, wet) in [(-1.5, false), (1.5, false), (0.0, true)] {
            assert_eq!(f.wet[nearest(&f.ys, y) * nx + i0], wet, "wet at (0, {y})");
        }
        assert!(
            f.notes.iter().any(|n| n.starts_with("catamaran:")),
            "{:?}",
            f.notes
        );
    }

    /// Review focus 5: demihulls 0.8 m apart with a 1 m beam overlap.
    #[test]
    fn overlapping_demihulls_are_refused() {
        let Err(err) = wave_field(&wigley(), &cat_opts(0.3, 0.8)) else {
            panic!("overlapping demihulls were accepted");
        };
        assert!(err.contains("spacing"), "{err}");
    }

    /// Oracle §40: Wu et al. (2019) eqs. 15a and 18 put the cusp of the Kelvin wedge on the ray
    /// tan ψ_K = 1/√8 (19.47°). Across a transverse cut 40 L behind the bow, the envelope of |ζ| peaks
    /// inside the wedge, close to ψ_K: the Airy-function peak of a cusp lies inside the caustic, at an
    /// offset that shrinks as h^(-2/3), hence the band ψ_K - 2° to ψ_K + 0.5°. Outside the wedge no wave
    /// angle is stationary, so beyond ψ_K + 3° the envelope stays below 10 % of the peak.
    #[test]
    fn kelvin_wedge_cusp() {
        let o = opts_at_fn(0.3);
        let bow = 5.0;
        let x = bow - 400.0;
        let cut = wave_cut(&wigley(), &o, CutLine::Transverse { x }, 0.0, 250.0, 4000).unwrap();
        let window = 0.25 * 2.0 * PI * o.speed_m_s * o.speed_m_s / o.g;
        let psi_k = (1.0 / 8.0_f64.sqrt()).atan().to_degrees();
        let mut env: Vec<(f64, f64)> = Vec::new(); // (ψ of the window centre in degrees, max |ζ|)
        let mut start = 0;
        while start < cut.s.len() {
            let end = cut.s[start] + window;
            let stop = cut.s.partition_point(|&s| s < end).max(start + 1);
            let peak = cut.zeta[start..stop]
                .iter()
                .map(|z| z.abs())
                .fold(0.0, f64::max);
            let mid = 0.5 * (cut.s[start] + cut.s[stop - 1]);
            env.push(((mid / (bow - x)).atan().to_degrees(), peak));
            start = stop;
        }
        let &(psi_peak, peak) = env.iter().max_by(|a, b| a.1.total_cmp(&b.1)).unwrap();
        let beyond = env
            .iter()
            .filter(|e| e.0 > psi_k + 3.0)
            .map(|e| e.1)
            .fold(0.0, f64::max);
        eprintln!(
            "cusp: envelope peak at ψ {psi_peak:.2}° (ψ_K {psi_k:.2}°), beyond ψ_K + 3°: {:.2} % of peak",
            100.0 * beyond / peak
        );
        assert!(
            (psi_k - 2.0..=psi_k + 0.5).contains(&psi_peak),
            "peak at {psi_peak}°"
        );
        assert!(
            beyond < 0.1 * peak,
            "{beyond} beyond the wedge vs peak {peak}"
        );
    }

    #[test]
    fn cuts_match_point_evaluation() {
        let (g, o) = (wigley(), opts_at_fn(0.3));
        let t = wave_cut(&g, &o, CutLine::Transverse { x: -20.0 }, 0.0, 10.0, 101).unwrap();
        let sp = wave_spectrum(&g, &o, 25.0, 10.0, 0.1).unwrap();
        let max = t.zeta.iter().map(|z| z.abs()).fold(0.0, f64::max);
        assert!(max > 0.0);
        for (y, z) in t.s.iter().zip(&t.zeta) {
            assert!(
                (z - sp.elevation(-20.0, *y)).abs() <= 1e-10 * max,
                "transverse at y {y}"
            );
        }
        let l = wave_cut(&g, &o, CutLine::Longitudinal { y: 3.0 }, -30.0, 10.0, 201).unwrap();
        let sp = wave_spectrum(&g, &o, 35.0, 3.0, 0.2).unwrap();
        let max = l.zeta.iter().map(|z| z.abs()).fold(0.0, f64::max);
        for (x, z) in l.s.iter().zip(&l.zeta) {
            assert!(
                (z - sp.elevation(*x, 3.0)).abs() <= 1e-10 * max,
                "longitudinal at x {x}"
            );
        }
        assert_eq!(l.notes[0], FAR_FIELD_NOTE);
    }

    #[test]
    fn bad_cuts_are_refused() {
        let (g, o) = (wigley(), opts_at_fn(0.3));
        let line = CutLine::Longitudinal { y: 2.0 };
        for (from, to, n) in [(5.0, -5.0, 100), (-5.0, 5.0, 1), (f64::NAN, 5.0, 100)] {
            let Err(err) = wave_cut(&g, &o, line, from, to, n) else {
                panic!("cut {from}..{to} with {n} samples was accepted");
            };
            assert!(err.contains("cut"), "{err}");
        }
    }

    /// A coarse Wigley field (1 m spacing, 51 x 31) for the exports.
    fn small_field() -> WaveField {
        let o = WavePatternOptions {
            spacing_m: Some(1.0),
            ..opts_at_fn(0.3)
        };
        wave_field(&wigley(), &o).unwrap()
    }

    #[test]
    fn diverging_rgb_ends_and_middle() {
        assert_eq!(diverging_rgb(-1.0), DIVERGING_STOPS[0]);
        assert_eq!(diverging_rgb(0.0), DIVERGING_STOPS[1]);
        assert_eq!(diverging_rgb(1.0), DIVERGING_STOPS[2]);
        assert_eq!(diverging_rgb(-7.0), DIVERGING_STOPS[0]);
        assert_eq!(diverging_rgb(f64::NAN), DIVERGING_STOPS[1]);
    }

    #[test]
    fn png_has_the_grid_size_and_hull_pixels() {
        let f = small_field();
        let (nx, ny) = (f.xs.len(), f.ys.len());
        let png = wave_png(&f, None).unwrap();
        assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
        let be = |b: &[u8]| u32::from_be_bytes([b[0], b[1], b[2], b[3]]) as usize;
        assert_eq!((be(&png[16..20]), be(&png[20..24])), (nx, ny));
        let rgba = wave_rgba(&f, None);
        let (ix, iy) = (nearest(&f.xs, 0.0), nearest(&f.ys, 0.0));
        let px = ((ny - 1 - iy) * nx + ix) * 4; // row 0 is the largest y
        assert_eq!(rgba[px..px + 3], HULL_RGB);
        // The crest colour appears at the largest ζ and nowhere does a wet pixel take the hull colour.
        let top = (0..f.zeta.len())
            .max_by(|&a, &b| f.zeta[a].total_cmp(&f.zeta[b]))
            .unwrap();
        let px = ((ny - 1 - top / nx) * nx + top % nx) * 4;
        assert_eq!(rgba[px..px + 3], DIVERGING_STOPS[2]);
    }

    #[test]
    fn svg_has_crests_troughs_and_the_note() {
        let svg = wave_svg(&small_field(), None);
        let [r, g, b] = DIVERGING_STOPS[2];
        assert!(
            svg.contains(&format!("stroke=\"rgb({r},{g},{b})\"")),
            "no crest contours"
        );
        let [r, g, b] = DIVERGING_STOPS[0];
        assert!(svg.contains(&format!("stroke=\"rgb({r},{g},{b})\" stroke-dasharray")));
        let [r, g, b] = HULL_RGB;
        assert!(
            svg.contains(&format!("fill=\"rgb({r},{g},{b})\"")),
            "no hull"
        );
        assert!(svg.contains(&format!("<desc>{FAR_FIELD_NOTE}")));
        assert!(svg.contains("Fn 0.300"));
        assert!(svg.trim_end().ends_with("</svg>"));
    }

    #[test]
    fn vti_extent_and_value_count_match_the_grid() {
        let f = small_field();
        let (nx, ny) = (f.xs.len(), f.ys.len());
        let vti = wave_vti(&f);
        assert!(vti.contains(&format!("WholeExtent=\"0 {} 0 {} 0 0\"", nx - 1, ny - 1)));
        let array = |name: &str| -> usize {
            let start = vti.find(&format!("Name=\"{name}\"")).unwrap();
            let body = &vti[start..];
            let open = body.find('>').unwrap() + 1;
            let close = body.find("</DataArray>").unwrap();
            body[open..close].split_whitespace().count()
        };
        assert_eq!(array("zeta"), nx * ny);
        assert_eq!(array("wet"), nx * ny);
    }

    #[test]
    fn contour_levels_are_symmetric_without_zero() {
        let levels = contour_levels(5.0, 5);
        assert_eq!(
            levels,
            vec![-5.0, -4.0, -3.0, -2.0, -1.0, 1.0, 2.0, 3.0, 4.0, 5.0]
        );
    }

    /// Attribute values of `name` in `svg`, as numbers (each whitespace-separated part).
    fn attr_values(svg: &str, name: &str) -> Vec<f64> {
        let key = format!(" {name}=\"");
        svg.match_indices(&key)
            .flat_map(|(i, _)| {
                let rest = &svg[i + key.len()..];
                rest[..rest.find('"').unwrap()]
                    .split_whitespace()
                    .map(|v| v.parse::<f64>().unwrap())
                    .collect::<Vec<_>>()
            })
            .collect()
    }

    /// The SVG is in metres: a line 1 unit wide would be 1 m wide, and `vector-effect` is not
    /// honoured by every renderer, so strokes and dashes are set as a small part of the domain.
    #[test]
    fn svg_strokes_are_thin_in_metres() {
        let f = small_field();
        let width = f.xs[f.xs.len() - 1] - f.xs[0];
        let svg = wave_svg(&f, None);
        assert!(!svg.contains("vector-effect"));
        let strokes = attr_values(&svg, "stroke-width");
        assert!(!strokes.is_empty());
        assert!(strokes.iter().all(|&w| w <= 0.005 * width), "{strokes:?}");
        let dashes = attr_values(&svg, "stroke-dasharray");
        assert!(
            !dashes.is_empty() && dashes.iter().all(|&d| d <= 0.02 * width),
            "{dashes:?}"
        );
    }

    #[test]
    fn svg_caption_fits_the_width() {
        let f = small_field();
        let width = f.xs[f.xs.len() - 1] - f.xs[0];
        let svg = wave_svg(&f, None);
        let font = attr_values(&svg, "font-size")[0];
        let lines: Vec<&str> = svg
            .split("<text ")
            .skip(1)
            .map(|t| &t[t.find('>').unwrap() + 1..t.find("</text>").unwrap()])
            .collect();
        assert!(lines.len() > f.notes.len() + f.warnings.len(), "{lines:?}");
        for line in lines {
            assert!(
                line.chars().count() as f64 * 0.6 * font <= width,
                "too long: {line}"
            );
        }
    }

    #[test]
    fn field_carries_the_waterline_outlines() {
        let f = small_field();
        assert_eq!(f.waterlines.len(), 1);
        let widest = f.waterlines[0]
            .iter()
            .map(|p| p[1])
            .fold(f64::MIN, f64::max);
        assert!((widest - 0.5).abs() < 1e-9, "half-breadth {widest}");
        let cat = wave_field(
            &wigley(),
            &WavePatternOptions {
                spacing_m: Some(1.0),
                ..cat_opts(0.3, 3.0)
            },
        )
        .unwrap();
        assert_eq!(cat.waterlines.len(), 2);
        for (outline, centre) in cat.waterlines.iter().zip([1.5, -1.5]) {
            let mean = outline.iter().map(|p| p[1]).sum::<f64>() / outline.len() as f64;
            assert!((mean - centre).abs() < 1e-9, "{mean} vs {centre}");
        }
        assert!(summary_json(&f).get("waterlines").is_none());
    }

    /// A speed so low that the default spacing underflows to 0 m must be refused, not loop forever.
    #[test]
    fn a_vanishing_speed_is_refused() {
        let o = WavePatternOptions::at_speed(1e-170);
        let Err(err) = wave_field(&wigley(), &o) else {
            panic!("a speed of 1e-170 m/s was accepted");
        };
        assert!(err.contains("speed"), "{err}");
    }

    /// A positive but tiny spacing is coarsened to the point cap, not overflowed.
    #[test]
    fn a_tiny_spacing_is_coarsened() {
        let o = WavePatternOptions {
            spacing_m: Some(1e-300),
            ..opts_at_fn(0.3)
        };
        // The layout is where the count overflowed; the field itself is low_froude_field_is_capped.
        let lay = field_layout(&wigley(), &o).unwrap();
        assert!(lay.xs.len() * (2 * lay.half_rows + 1) <= MAX_FIELD_POINTS);
        assert!(lay.coarsened.is_some());
    }

    /// About 250 000 wave-angle samples (a reach of 170 m sideways at 0.025 m) would take about
    /// 400 MB of stored spectrum: refused with a way out, not allocated.
    #[test]
    fn spectrum_memory_is_bounded() {
        let Err(err) = wave_spectrum(&wigley(), &opts_at_fn(0.3), 405.0, 170.0, 0.025) else {
            panic!("a 400 MB spectrum was built");
        };
        assert!(err.contains("smaller"), "{err}");
    }
}
