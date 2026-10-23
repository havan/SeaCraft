//! Irregular head-sea responses: wave spectra, spectral moments of the strip-theory
//! RAOs of [`crate::seakeeping`], significant and RMS motions, relative motion at
//! points on the hull, vertical accelerations and the motion sickness incidence.
//!
//! # Wave spectra
//!
//! * **Bretschneider** (ITTC two-parameter, $H_s$, $T_p$, $\omega_p = 2\pi/T_p$):
//!   $S(\omega) = \tfrac{5}{16} H_s^2 \omega_p^4 \omega^{-5} \exp(-\tfrac54 (\omega_p/\omega)^4)$.
//! * **JONSWAP**: $S_J = N\, S_B\, \gamma^r$ with
//!   $r = \exp(-(\omega-\omega_p)^2 / (2\sigma^2\omega_p^2))$, $\sigma = 0.07$ for
//!   $\omega \le \omega_p$ and $0.09$ above. $N$ is found *numerically* so that
//!   $\int S_J\,d\omega = H_s^2/16$ (Simpson on $0.05\,\omega_p \ldots 60\,\omega_p$).
//!   $\gamma = 1$ is exactly Bretschneider.
//!
//! # Responses
//!
//! For a response with complex transfer function $H(\omega)$ per unit wave amplitude
//! (time factor $e^{i\omega_e t}$, heave up, pitch bow up, $x$ forward) the spectral
//! moments are taken over the **incident** frequency (no Jacobian):
//! $m_n = \int |H|^2 \omega_e^n S(\omega)\,d\omega$, $\omega_e = \omega + \omega^2 U/g$.
//! Significant amplitude is $2\sqrt{m_0}$, RMS $\sqrt{m_0}$ and the mean
//! zero-crossing encounter period $2\pi\sqrt{m_0/m_2}$.
//!
//! At a point $x$ (model coordinate, $x_r = x - x_{LCG}$): vertical motion
//! $s = \eta_3 + x_r\eta_5$, wave elevation $\zeta = e^{ikx_r}$, relative motion
//! $r = s - \zeta$ and vertical acceleration $-\omega_e^2 s$.
//!
//! # Motion sickness incidence
//!
//! O'Hanlon & McCauley (1974), 2-hour exposure:
//! $\mathrm{MSI} = 100\,\Phi\big((\log_{10}(a/g) - \mu)/0.4\big)$,
//! $\mu = -0.819 + 2.32 (\log_{10}\omega)^2$ with $a$ the mean absolute vertical
//! acceleration ($0.798\,\sigma_a$) and $\omega$ the mean zero-crossing encounter
//! frequency of the acceleration. Numbers are checked against the independent
//! oracle `docs/oracles/irregular_seas.py` (see `docs/oracles.md`).

use crate::hydro::Warning;
use crate::mesh::Surface;
use crate::seakeeping::{head_sea_raos, SeakeepingReport, SeakeepingSpec};
use serde::{Deserialize, Serialize};
use std::f64::consts::PI;

/// Knots to metres per second.
const KN_TO_MS: f64 = 0.5144444444444445;

/// A parametric long-crested wave spectrum S(ω) in m²·s/rad.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WaveSpectrum {
    /// ITTC two-parameter spectrum.
    Bretschneider {
        /// Significant wave height in metres.
        hs: f64,
        /// Peak period in seconds.
        tp: f64,
    },
    /// JONSWAP spectrum normalised to the given Hs.
    Jonswap {
        /// Significant wave height in metres.
        hs: f64,
        /// Peak period in seconds.
        tp: f64,
        /// Peak enhancement factor (3.3 for the mean JONSWAP).
        gamma: f64,
    },
}

impl WaveSpectrum {
    /// Significant wave height in metres.
    pub fn hs(&self) -> f64 {
        match *self {
            WaveSpectrum::Bretschneider { hs, .. } | WaveSpectrum::Jonswap { hs, .. } => hs,
        }
    }

    /// Peak period in seconds.
    pub fn tp(&self) -> f64 {
        match *self {
            WaveSpectrum::Bretschneider { tp, .. } | WaveSpectrum::Jonswap { tp, .. } => tp,
        }
    }

    /// Peak circular frequency ωp = 2π/Tp in rad/s.
    pub fn peak_omega(&self) -> f64 {
        2.0 * PI / self.tp()
    }

    /// Check that the parameters are finite and in a physical range.
    pub fn validate(&self) -> Result<(), String> {
        let (hs, tp) = (self.hs(), self.tp());
        if !hs.is_finite() || hs <= 0.0 {
            return Err("hs must be positive and finite".into());
        }
        if !tp.is_finite() || tp <= 0.0 {
            return Err("tp must be positive and finite".into());
        }
        if let WaveSpectrum::Jonswap { gamma, .. } = *self {
            if !gamma.is_finite() || !(1.0..=20.0).contains(&gamma) {
                return Err("gamma must be between 1 and 20".into());
            }
        }
        Ok(())
    }

    /// Bretschneider shape at ω (no normalisation factor).
    fn bretschneider(hs: f64, wp: f64, w: f64) -> f64 {
        if w <= 0.0 {
            return 0.0;
        }
        5.0 / 16.0 * hs * hs * wp.powi(4) * w.powi(-5) * (-1.25 * (wp / w).powi(4)).exp()
    }

    /// Unnormalised shape (Bretschneider times γ^r).
    fn shape(&self, w: f64) -> f64 {
        let wp = self.peak_omega();
        let base = Self::bretschneider(self.hs(), wp, w);
        match *self {
            WaveSpectrum::Bretschneider { .. } => base,
            WaveSpectrum::Jonswap { gamma, .. } => {
                let sigma = if w <= wp { 0.07 } else { 0.09 };
                let r = (-(w - wp).powi(2) / (2.0 * sigma * sigma * wp * wp)).exp();
                base * gamma.powf(r)
            }
        }
    }

    /// JONSWAP normalisation N such that ∫S dω = Hs²/16; 1 for Bretschneider and γ = 1.
    ///
    /// Computed numerically (Simpson on 0.05 ωp … 60 ωp), so [`density`](Self::density)
    /// repeats that integral on every call; use [`density_grid`](Self::density_grid)
    /// for many frequencies.
    pub fn normalisation(&self) -> f64 {
        match *self {
            WaveSpectrum::Bretschneider { .. } => 1.0,
            WaveSpectrum::Jonswap { gamma, hs, .. } => {
                if (gamma - 1.0).abs() < 1e-12 {
                    return 1.0;
                }
                let wp = self.peak_omega();
                let n = 20_000usize; // intervals (even)
                let (a, b) = (0.05 * wp, 60.0 * wp);
                let h = (b - a) / n as f64;
                let mut sum = self.shape(a) + self.shape(b);
                for i in 1..n {
                    let w = a + h * i as f64;
                    sum += self.shape(w) * if i % 2 == 1 { 4.0 } else { 2.0 };
                }
                let m0 = sum * h / 3.0;
                hs * hs / 16.0 / m0
            }
        }
    }

    /// Spectral density S(ω) in m²·s/rad.
    pub fn density(&self, w: f64) -> f64 {
        self.normalisation() * self.shape(w)
    }

    /// Spectral density on a grid, normalising once.
    pub fn density_grid(&self, omegas: &[f64]) -> Vec<f64> {
        let n = self.normalisation();
        omegas.iter().map(|&w| n * self.shape(w)).collect()
    }
}

/// ∫ y dx on a possibly non-uniform grid: composite Simpson over pairs of intervals
/// (the unequal-spacing form, equal to h/3·(1,4,1) when uniform) and a trapezoid
/// for a trailing odd interval.
fn integrate(x: &[f64], y: &[f64]) -> f64 {
    let n = x.len().min(y.len());
    if n < 2 {
        return 0.0;
    }
    let mut total = 0.0;
    let mut i = 0;
    while i + 2 < n {
        let h0 = x[i + 1] - x[i];
        let h1 = x[i + 2] - x[i + 1];
        if h0 > 0.0 && h1 > 0.0 {
            let hs = h0 + h1;
            total += hs / 6.0
                * ((2.0 - h1 / h0) * y[i]
                    + hs * hs / (h0 * h1) * y[i + 1]
                    + (2.0 - h0 / h1) * y[i + 2]);
        } else {
            total += 0.5 * h0 * (y[i] + y[i + 1]) + 0.5 * h1 * (y[i + 1] + y[i + 2]);
        }
        i += 2;
    }
    if i + 1 < n {
        total += 0.5 * (x[i + 1] - x[i]) * (y[i] + y[i + 1]);
    }
    total
}

/// Spectral moments m0..m4 of S(ω): m_n = ∫ ωⁿ S dω (Simpson, non-uniform grid allowed).
pub fn spectral_moments(omegas: &[f64], s: &[f64]) -> [f64; 5] {
    let mut out = [0.0; 5];
    for (n, m) in out.iter_mut().enumerate() {
        let y: Vec<f64> = omegas
            .iter()
            .zip(s)
            .map(|(&w, &sv)| w.powi(n as i32) * sv)
            .collect();
        *m = integrate(omegas, &y);
    }
    out
}

/// Response moments m_n = ∫ |H(ω)|² ω_eⁿ S(ω) dω over the incident frequency ω
/// (no Jacobian), ω_e = ω + ω²U/g (head seas), n = 0..4.
pub fn response_moments(
    omegas: &[f64],
    rao_sq: &[f64],
    spectrum: &[f64],
    speed_ms: f64,
    g: f64,
) -> [f64; 5] {
    let mut out = [0.0; 5];
    for (n, m) in out.iter_mut().enumerate() {
        let y: Vec<f64> = omegas
            .iter()
            .zip(rao_sq.iter().zip(spectrum))
            .map(|(&w, (&h2, &sv))| {
                let we = w + w * w * speed_ms / g;
                h2 * we.powi(n as i32) * sv
            })
            .collect();
        *m = integrate(omegas, &y);
    }
    out
}

/// Complementary error function, fractional error < 1.2e-7 (Numerical Recipes `erfcc`).
fn erfc(x: f64) -> f64 {
    let z = x.abs();
    let t = 1.0 / (1.0 + 0.5 * z);
    let poly = -z * z - 1.26551223
        + t * (1.00002368
            + t * (0.37409196
                + t * (0.09678418
                    + t * (-0.18628806
                        + t * (0.27886807
                            + t * (-1.13520398
                                + t * (1.48851587 + t * (-0.82215223 + t * 0.17087277))))))));
    let ans = t * poly.exp();
    if x >= 0.0 {
        ans
    } else {
        2.0 - ans
    }
}

/// Standard normal cumulative distribution function.
fn normal_cdf(z: f64) -> f64 {
    0.5 * erfc(-z / std::f64::consts::SQRT_2)
}

/// Motion sickness incidence in percent of unadapted subjects who vomit within
/// 2 hours (O'Hanlon & McCauley 1974): `mean_abs_accel_ms2` is the mean absolute
/// vertical acceleration, `omega` the (encounter) frequency in rad/s.
pub fn msi_percent(mean_abs_accel_ms2: f64, omega: f64, g: f64) -> f64 {
    if !(mean_abs_accel_ms2 > 0.0 && omega > 0.0 && g > 0.0)
        || !mean_abs_accel_ms2.is_finite()
        || !omega.is_finite()
    {
        return 0.0;
    }
    let mu = -0.819 + 2.32 * omega.log10().powi(2);
    100.0 * normal_cdf(((mean_abs_accel_ms2 / g).log10() - mu) / 0.4)
}

/// A named longitudinal position on the hull at which motions are evaluated.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MotionPoint {
    /// Label shown in tables.
    pub name: String,
    /// Longitudinal position in model coordinates, metres.
    pub x: f64,
}

fn default_omegas() -> usize {
    60
}

/// Input of an irregular head-sea calculation.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct IrregularSpec {
    /// Wave spectrum.
    pub sea: WaveSpectrum,
    /// Hull, loading and speed. Its `omegas`/`lambda_*`/`points` fields are ignored:
    /// the frequency grid comes from the spectrum.
    pub seakeeping: SeakeepingSpec,
    /// Evaluation points; empty means "LCG" and "Bow" (forward end of the design waterline).
    #[serde(default)]
    pub points: Vec<MotionPoint>,
    /// Number of frequencies across 0.2 ωp … 5 ωp (default 60).
    #[serde(default = "default_omegas")]
    pub omegas: usize,
}

impl IrregularSpec {
    /// A spec with default points and 60 frequencies.
    pub fn new(sea: WaveSpectrum, seakeeping: SeakeepingSpec) -> Self {
        Self {
            sea,
            seakeeping,
            points: Vec::new(),
            omegas: default_omegas(),
        }
    }

    /// Build a spec from the JSON parameters shared by the MCP tool and the GUI control
    /// channel: `draft` (falls back to `default_draft` when positive), `speed_kn`,
    /// `kg`, `kyy`, `stations`, `hs`, `tp`, optional `gamma` (present = JONSWAP),
    /// `points` as `[{name, x}]` and `omegas` (a count). Wrong types are errors.
    pub fn from_params(p: &serde_json::Value, default_draft: f64) -> Result<Self, String> {
        use serde_json::Value;
        let obj = p.as_object().ok_or("parameters must be an object")?;
        let num = |key: &str| -> Result<Option<f64>, String> {
            match obj.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .filter(|x| x.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("`{key}` must be a finite number")),
            }
        };
        let count = |key: &str, lo: f64, hi: f64| -> Result<Option<usize>, String> {
            match num(key)? {
                None => Ok(None),
                Some(x) if x.fract() == 0.0 && (lo..=hi).contains(&x) => Ok(Some(x as usize)),
                Some(_) => Err(format!("`{key}` must be a whole number from {lo} to {hi}")),
            }
        };
        let draft = match num("draft")? {
            Some(d) if d > 0.0 && d <= 1.0e4 => d,
            Some(_) => return Err("draft must be positive".into()),
            None if default_draft > 0.0 => default_draft,
            None => return Err("draft must be specified or set in project settings".into()),
        };
        let hs = num("hs")?.ok_or("`hs` (significant wave height, m) is required")?;
        let tp = num("tp")?.ok_or("`tp` (peak period, s) is required")?;
        let sea = match num("gamma")? {
            Some(gamma) => WaveSpectrum::Jonswap { hs, tp, gamma },
            None => WaveSpectrum::Bretschneider { hs, tp },
        };
        let mut points = Vec::new();
        match obj.get("points") {
            None | Some(Value::Null) => {}
            Some(Value::Array(items)) => {
                if items.len() > 64 {
                    return Err("`points` has too many entries (at most 64)".into());
                }
                for (i, item) in items.iter().enumerate() {
                    let x = item
                        .get("x")
                        .and_then(Value::as_f64)
                        .filter(|x| x.is_finite())
                        .ok_or_else(|| format!("`points[{i}].x` must be a finite number"))?;
                    let name = match item.get("name") {
                        None | Some(Value::Null) => format!("P{}", i + 1),
                        Some(Value::String(s)) => s.chars().take(40).collect(),
                        Some(_) => return Err(format!("`points[{i}].name` must be a string")),
                    };
                    points.push(MotionPoint { name, x });
                }
            }
            Some(_) => return Err("`points` must be an array of {name, x}".into()),
        }
        let seakeeping = SeakeepingSpec {
            draft,
            speed_kn: num("speed_kn")?.or(num("speed")?).unwrap_or(0.0),
            kg: num("kg")?.unwrap_or(0.0),
            kyy: num("kyy")?,
            stations: count("stations", 3.0, 200.0)?.unwrap_or(21),
            ..Default::default()
        };
        let spec = Self {
            sea,
            seakeeping,
            points,
            omegas: count("omegas", 8.0, 2000.0)?.unwrap_or_else(default_omegas),
        };
        spec.validate()?;
        Ok(spec)
    }

    /// The uniform frequency grid 0.2 ωp … 5 ωp used for the RAOs and the spectrum.
    pub fn omega_grid(&self) -> Vec<f64> {
        let wp = self.sea.peak_omega();
        let n = self.omegas.max(2);
        let (a, b) = (0.2 * wp, 5.0 * wp);
        (0..n)
            .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
            .collect()
    }

    /// Check every input; messages name the offending field.
    pub fn validate(&self) -> Result<(), String> {
        self.sea.validate()?;
        if !(8..=2000).contains(&self.omegas) {
            return Err("omegas must be between 8 and 2000".into());
        }
        if self.points.iter().any(|p| !p.x.is_finite()) {
            return Err("point x must be finite".into());
        }
        Ok(())
    }
}

/// Statistics of one response from its spectral moments. Moments are in the unit
/// of the response squared (and per s² / s⁴ for m2 / m4).
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct ResponseStats {
    /// Variance m0.
    pub m0: f64,
    /// Second moment about encounter frequency.
    pub m2: f64,
    /// Fourth moment about encounter frequency.
    pub m4: f64,
    /// Root-mean-square √m0.
    pub rms: f64,
    /// Significant amplitude 2√m0.
    pub significant: f64,
    /// Mean zero-crossing encounter period 2π√(m0/m2) in s (0 if undefined).
    pub tz_s: f64,
}

impl ResponseStats {
    fn from_moments(m: [f64; 5], scale_sq: f64) -> Self {
        let (m0, m2, m4) = (m[0] * scale_sq, m[2] * scale_sq, m[4] * scale_sq);
        let m0p = m0.max(0.0);
        Self {
            m0,
            m2,
            m4,
            rms: m0p.sqrt(),
            significant: 2.0 * m0p.sqrt(),
            tz_s: if m0 > 0.0 && m2 > 0.0 {
                2.0 * PI * (m0 / m2).sqrt()
            } else {
                0.0
            },
        }
    }
}

/// Responses at one point.
#[derive(Debug, Clone, Serialize)]
pub struct PointResponse {
    /// Label.
    pub name: String,
    /// Model x in metres.
    pub x: f64,
    /// Vertical motion s = η3 + x_r η5 (m, positive up).
    pub vertical_motion: ResponseStats,
    /// Motion relative to the wave surface s − ζ (m).
    pub relative_motion: ResponseStats,
    /// Vertical acceleration −ω_e² s (m/s²).
    pub vertical_acceleration: ResponseStats,
    /// RMS vertical acceleration in g.
    pub acceleration_rms_g: f64,
    /// Significant vertical acceleration amplitude in g.
    pub acceleration_significant_g: f64,
    /// Mean absolute acceleration 0.798 σ_a (m/s²).
    pub mean_abs_acceleration: f64,
    /// Mean zero-crossing encounter frequency of the acceleration √(m2/m0), rad/s.
    pub acceleration_omega_e: f64,
    /// Motion sickness incidence, % within 2 hours of exposure.
    pub msi_percent: f64,
}

/// Properties of the input spectrum on the frequency grid used.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct SpectrumStats {
    /// Peak circular frequency in rad/s.
    pub peak_omega: f64,
    /// Zeroth moment of the discretised spectrum (Hs²/16 up to truncation).
    pub m0: f64,
    /// Hs recomputed as 4√m0 (check against the input).
    pub hs_check: f64,
    /// Mean period 2π m0/m1 in s.
    pub t1_s: f64,
    /// Zero-crossing period 2π√(m0/m2) in s.
    pub tz_s: f64,
}

/// Result of an irregular head-sea calculation.
#[derive(Debug, Clone, Serialize)]
pub struct IrregularReport {
    /// Spectrum used.
    pub sea: WaveSpectrum,
    /// Speed in knots.
    pub speed_kn: f64,
    /// Draft in metres.
    pub draft: f64,
    /// LCG (motion origin) in model x, metres.
    pub x_lcg: f64,
    /// Number of frequencies integrated.
    pub omega_count: usize,
    /// Lowest frequency, rad/s.
    pub omega_min: f64,
    /// Highest frequency, rad/s.
    pub omega_max: f64,
    /// The input spectrum's own statistics.
    pub spectrum: SpectrumStats,
    /// Heave in metres.
    pub heave: ResponseStats,
    /// Pitch in degrees (positive bow up).
    pub pitch_deg: ResponseStats,
    /// Responses at each evaluation point.
    pub points: Vec<PointResponse>,
    /// RAO-solver warnings (e.g. irregular-frequency proximity).
    pub warnings: Vec<Warning>,
    /// Statement of what the MSI means.
    pub msi_note: &'static str,
}

/// What the MSI figure represents.
pub const MSI_NOTE: &str =
    "MSI: share of unadapted subjects expected to vomit within a 2-hour exposure (O'Hanlon & McCauley 1974)";

/// Default evaluation points: the LCG and the forward end of the design waterline.
pub fn default_points(x_lcg: f64, x_bow: f64) -> Vec<MotionPoint> {
    vec![
        MotionPoint {
            name: "LCG".into(),
            x: x_lcg,
        },
        MotionPoint {
            name: "Bow".into(),
            x: x_bow,
        },
    ]
}

/// Head-sea irregular responses: compute the RAOs on the spectrum's frequency grid
/// (0.2 ωp … 5 ωp) and integrate them against the spectrum.
pub fn irregular_head_sea(
    surface: &Surface,
    spec: &IrregularSpec,
) -> Result<IrregularReport, String> {
    spec.validate()?;
    let mut sk = spec.seakeeping.clone();
    sk.omegas = spec.omega_grid();
    let rep = head_sea_raos(surface, &sk)?;
    irregular_from_raos(&rep, spec)
}

/// Integrate already-computed RAOs against `spec.sea`. `rep` must have been run
/// with `spec.seakeeping` on [`IrregularSpec::omega_grid`] (the GUI worker does so
/// to reuse one RAO run); the frequencies of `rep` define the integration grid.
pub fn irregular_from_raos(
    rep: &SeakeepingReport,
    spec: &IrregularSpec,
) -> Result<IrregularReport, String> {
    spec.validate()?;
    let g = spec.seakeeping.g;
    let speed = spec.seakeeping.speed_kn * KN_TO_MS;
    if rep.frequencies.len() < 3 {
        return Err("at least three RAO frequencies are needed".into());
    }
    let omegas: Vec<f64> = rep.frequencies.iter().map(|p| p.omega).collect();
    let s = spec.sea.density_grid(&omegas);

    let sm = spectral_moments(&omegas, &s);
    let spectrum = SpectrumStats {
        peak_omega: spec.sea.peak_omega(),
        m0: sm[0],
        hs_check: 4.0 * sm[0].max(0.0).sqrt(),
        t1_s: if sm[1] > 0.0 {
            2.0 * PI * sm[0] / sm[1]
        } else {
            0.0
        },
        tz_s: if sm[0] > 0.0 && sm[2] > 0.0 {
            2.0 * PI * (sm[0] / sm[2]).sqrt()
        } else {
            0.0
        },
    };

    let moments = |h2: &[f64]| response_moments(&omegas, h2, &s, speed, g);

    // Complex transfer functions per unit wave amplitude, (re, im).
    let eta3: Vec<(f64, f64)> = rep
        .frequencies
        .iter()
        .map(|p| {
            let ph = p.heave_phase_deg.to_radians();
            (p.heave_amp * ph.cos(), p.heave_amp * ph.sin())
        })
        .collect();
    let eta5: Vec<(f64, f64)> = rep
        .frequencies
        .iter()
        .map(|p| {
            let k = p.omega * p.omega / g;
            let ph = p.pitch_phase_deg.to_radians();
            (p.pitch_amp * k * ph.cos(), p.pitch_amp * k * ph.sin())
        })
        .collect();

    let heave_sq: Vec<f64> = eta3.iter().map(|c| c.0 * c.0 + c.1 * c.1).collect();
    let pitch_sq: Vec<f64> = eta5.iter().map(|c| c.0 * c.0 + c.1 * c.1).collect();
    let heave = ResponseStats::from_moments(moments(&heave_sq), 1.0);
    let pitch_deg = ResponseStats::from_moments(moments(&pitch_sq), (180.0 / PI).powi(2));

    let points = if spec.points.is_empty() {
        default_points(rep.x_lcg, rep.x_bow)
    } else {
        spec.points.clone()
    };

    let mut out_points = Vec::with_capacity(points.len());
    for pt in &points {
        let x_r = pt.x - rep.x_lcg;
        let mut s_sq = Vec::with_capacity(omegas.len());
        let mut r_sq = Vec::with_capacity(omegas.len());
        let mut a_sq = Vec::with_capacity(omegas.len());
        for (i, &w) in omegas.iter().enumerate() {
            let k = w * w / g;
            let we = w + w * w * speed / g;
            let sre = eta3[i].0 + x_r * eta5[i].0;
            let sim = eta3[i].1 + x_r * eta5[i].1;
            let zre = (k * x_r).cos();
            let zim = (k * x_r).sin();
            let mag_s = sre * sre + sim * sim;
            s_sq.push(mag_s);
            r_sq.push((sre - zre).powi(2) + (sim - zim).powi(2));
            a_sq.push(we.powi(4) * mag_s);
        }
        let vertical_motion = ResponseStats::from_moments(moments(&s_sq), 1.0);
        let relative_motion = ResponseStats::from_moments(moments(&r_sq), 1.0);
        let acc = ResponseStats::from_moments(moments(&a_sq), 1.0);
        let mean_abs = 0.798 * acc.rms;
        let omega_a = if acc.m0 > 0.0 && acc.m2 > 0.0 {
            (acc.m2 / acc.m0).sqrt()
        } else {
            0.0
        };
        out_points.push(PointResponse {
            name: pt.name.clone(),
            x: pt.x,
            vertical_motion,
            relative_motion,
            vertical_acceleration: acc,
            acceleration_rms_g: acc.rms / g,
            acceleration_significant_g: acc.significant / g,
            mean_abs_acceleration: mean_abs,
            acceleration_omega_e: omega_a,
            msi_percent: msi_percent(mean_abs, omega_a, g),
        });
    }

    Ok(IrregularReport {
        sea: spec.sea,
        speed_kn: spec.seakeeping.speed_kn,
        draft: spec.seakeeping.draft,
        x_lcg: rep.x_lcg,
        omega_count: omegas.len(),
        omega_min: omegas[0],
        omega_max: omegas[omegas.len() - 1],
        spectrum,
        heave,
        pitch_deg,
        points: out_points,
        warnings: rep.warnings.clone(),
        msi_note: MSI_NOTE,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn grid(a: f64, b: f64, n: usize) -> Vec<f64> {
        (0..n)
            .map(|i| a + (b - a) * i as f64 / (n - 1) as f64)
            .collect()
    }

    fn rel(a: f64, b: f64) -> f64 {
        ((a - b) / b).abs()
    }

    #[test]
    fn bretschneider_moments_match_the_oracle() {
        let sea = WaveSpectrum::Bretschneider { hs: 2.0, tp: 8.0 };
        let wp = sea.peak_omega();
        // Truncated 0.2 wp .. 5 wp grid, as the solver uses it.
        let w = grid(0.2 * wp, 5.0 * wp, 400);
        let m = spectral_moments(&w, &sea.density_grid(&w));
        assert!(rel(m[0], 0.25) < 0.005, "m0 {}", m[0]);
        // Wide grid.
        let w = grid(0.05 * wp, 60.0 * wp, 20_001);
        let m = spectral_moments(&w, &sea.density_grid(&w));
        assert!(rel(m[0], 0.250000) < 1e-3, "m0 {}", m[0]);
        assert!(rel(m[1], 0.254414) < 1e-3, "m1 {}", m[1]);
        assert!(rel(m[2], 0.305597) < 1e-3, "m2 {}", m[2]);
        let tp = 8.0;
        let t1 = 2.0 * PI * m[0] / m[1];
        let tz = 2.0 * PI * (m[0] / m[2]).sqrt();
        assert!(rel(t1 / tp, 0.7718) < 2e-3, "T1/Tp {}", t1 / tp);
        assert!(rel(tz / tp, 0.7104) < 2e-3, "Tz/Tp {}", tz / tp);
    }

    #[test]
    fn jonswap_gamma_one_is_bretschneider_and_normalised() {
        let b = WaveSpectrum::Bretschneider { hs: 2.0, tp: 8.0 };
        let j1 = WaveSpectrum::Jonswap {
            hs: 2.0,
            tp: 8.0,
            gamma: 1.0,
        };
        let wp = b.peak_omega();
        for i in 1..40 {
            let w = wp * (0.3 + 0.1 * i as f64);
            assert!(rel(j1.density(w), b.density(w)) < 1e-9, "w {w}");
        }
        let j = WaveSpectrum::Jonswap {
            hs: 2.0,
            tp: 8.0,
            gamma: 3.3,
        };
        let n = j.normalisation();
        assert!((n - 0.6558).abs() < 0.003, "N {n}");
        let w = grid(0.05 * wp, 60.0 * wp, 20_001);
        let s = j.density_grid(&w);
        let m = spectral_moments(&w, &s);
        assert!(rel(m[0], 0.25) < 1e-3, "m0 {}", m[0]);
        let step = w[1] - w[0];
        let (imax, _) = s
            .iter()
            .enumerate()
            .fold((0, f64::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a });
        assert!((w[imax] - wp).abs() <= step, "peak {} vs {wp}", w[imax]);
        // A coarse grid agrees too.
        let wc = grid(0.5 * wp, 3.0 * wp, 201);
        let peak = j.density_grid(&wc);
        let ic = peak
            .iter()
            .enumerate()
            .fold((0, f64::MIN), |a, (i, &v)| if v > a.1 { (i, v) } else { a })
            .0;
        assert!((wc[ic] - wp).abs() <= wc[1] - wc[0]);
    }

    #[test]
    fn spectrum_validation_rejects_nonsense() {
        assert!(WaveSpectrum::Bretschneider { hs: 0.0, tp: 8.0 }
            .validate()
            .is_err());
        assert!(WaveSpectrum::Bretschneider {
            hs: 1.0,
            tp: f64::NAN
        }
        .validate()
        .is_err());
        assert!(WaveSpectrum::Jonswap {
            hs: 1.0,
            tp: 8.0,
            gamma: 0.5
        }
        .validate()
        .is_err());
        assert!(WaveSpectrum::Jonswap {
            hs: 1.0,
            tp: 8.0,
            gamma: 3.3
        }
        .validate()
        .is_ok());
    }

    #[test]
    fn spectrum_serde_uses_kind_tag() {
        let v = serde_json::to_value(WaveSpectrum::Jonswap {
            hs: 1.0,
            tp: 5.0,
            gamma: 3.3,
        })
        .unwrap();
        assert_eq!(v["kind"], "jonswap");
        let back: WaveSpectrum =
            serde_json::from_str(r#"{"kind":"bretschneider","hs":2.0,"tp":8.0}"#).unwrap();
        assert_eq!(back, WaveSpectrum::Bretschneider { hs: 2.0, tp: 8.0 });
    }

    #[test]
    fn flat_rao_gives_half_hs_at_any_speed() {
        let sea = WaveSpectrum::Bretschneider { hs: 2.0, tp: 8.0 };
        let wp = sea.peak_omega();
        let w = grid(0.05 * wp, 60.0 * wp, 20_001);
        let s = sea.density_grid(&w);
        let ones = vec![1.0; w.len()];
        for u in [0.0, 3.0] {
            let m = response_moments(&w, &ones, &s, u, 9.81);
            let sig = 2.0 * m[0].sqrt();
            assert!((sig - 1.0).abs() < 1e-3, "U {u}: {sig}");
        }
    }

    #[test]
    fn resonant_synthetic_rao_matches_the_oracle() {
        let sea = WaveSpectrum::Bretschneider { hs: 2.0, tp: 8.0 };
        let wp = sea.peak_omega();
        let (u, g, wn, z) = (3.0, 9.81, 1.2, 0.2);
        let w = grid(0.05 * wp, 15.0 * wp, 30_001);
        let s = sea.density_grid(&w);
        let h2: Vec<f64> = w
            .iter()
            .map(|&wi| {
                let we = wi + wi * wi * u / g;
                let r = we / wn;
                1.0 / ((1.0 - r * r).powi(2) + (2.0 * z * r).powi(2))
            })
            .collect();
        let m = response_moments(&w, &h2, &s, u, g);
        assert!(rel(m[0], 0.825369) < 0.01, "m0 {}", m[0]);
        assert!(rel(m[2], 1.050609) < 0.01, "m2 {}", m[2]);
        assert!(rel(m[4], 1.590609) < 0.01, "m4 {}", m[4]);
    }

    #[test]
    fn nonuniform_integration_is_exact_for_quadratics() {
        let x = [0.0, 0.3, 1.0, 1.4, 2.5, 3.0, 4.2];
        let y: Vec<f64> = x
            .iter()
            .map(|v: &f64| 3.0 * v * v - 2.0 * v + 1.0)
            .collect();
        let exact = 4.2f64.powi(3) - 4.2f64.powi(2) + 4.2;
        // 6 intervals (even): all Simpson pairs, exact for quadratics.
        assert!((integrate(&x, &y) - exact).abs() < 1e-9);
        // An odd interval count ends in a trapezoid and stays close.
        let approx = integrate(&x[..6], &y[..6]);
        let exact6 = 3.0f64.powi(3) - 3.0f64.powi(2) + 3.0;
        assert!((approx - exact6).abs() < 0.2, "{approx} vs {exact6}");
    }

    #[test]
    fn msi_matches_the_oracle() {
        let g = 9.81;
        for (a, w, want) in [
            (0.10, 1.0, 32.545),
            (0.05, 1.0, 11.409),
            (0.20, 1.0, 61.794),
            (0.10, 0.5, 16.401),
            (0.10, 2.0, 16.401),
            (0.02, 1.2, 1.267),
        ] {
            let got = msi_percent(a * g, w, g);
            assert!((got - want).abs() < 0.01, "a/g {a} w {w}: {got} vs {want}");
        }
        assert_eq!(msi_percent(0.0, 1.0, g), 0.0);
        assert_eq!(msi_percent(1.0, 0.0, g), 0.0);
    }

    fn demo_spec(sea: WaveSpectrum, speed_kn: f64) -> IrregularSpec {
        IrregularSpec::new(
            sea,
            SeakeepingSpec {
                draft: 0.62,
                speed_kn,
                kg: 0.392,
                stations: 11,
                ..Default::default()
            },
        )
    }

    #[test]
    fn long_waves_are_followed_on_the_demo_hull() {
        let surface = crate::demo::demo_hull(true);
        for kn in [0.0, 6.0] {
            let sea = WaveSpectrum::Bretschneider { hs: 1.0, tp: 30.0 };
            let rep = irregular_head_sea(&surface, &demo_spec(sea, kn)).expect("irregular");
            let bow = rep.points.iter().find(|p| p.name == "Bow").unwrap();
            println!(
                "Tp=30 {kn} kn: heave sig {:.4} m, pitch sig {:.4} deg, bow rel sig {:.4} m, \
                 Hs check {:.4}",
                rep.heave.significant,
                rep.pitch_deg.significant,
                bow.relative_motion.significant,
                rep.spectrum.hs_check
            );
            assert!(
                (rep.heave.significant - 0.5).abs() < 0.03 * 0.5,
                "{kn} kn: heave {}",
                rep.heave.significant
            );
            assert!(
                rep.pitch_deg.significant < 0.5,
                "{kn} kn: pitch {}",
                rep.pitch_deg.significant
            );
            assert!(
                bow.relative_motion.significant < 0.1 * 0.5,
                "{kn} kn: bow relative {}",
                bow.relative_motion.significant
            );
        }
    }

    #[test]
    fn moderate_sea_on_the_demo_hull_is_finite_and_bow_is_worse() {
        let surface = crate::demo::demo_hull(true);
        let sea = WaveSpectrum::Bretschneider { hs: 1.0, tp: 4.0 };
        let rep = irregular_head_sea(&surface, &demo_spec(sea, 6.0)).expect("irregular");
        for st in [&rep.heave, &rep.pitch_deg] {
            assert!(st.significant.is_finite() && st.significant > 0.0);
            assert!(st.tz_s.is_finite() && st.tz_s > 0.0);
        }
        let lcg = rep.points.iter().find(|p| p.name == "LCG").unwrap();
        let bow = rep.points.iter().find(|p| p.name == "Bow").unwrap();
        for p in [lcg, bow] {
            assert!(p.vertical_acceleration.rms.is_finite() && p.vertical_acceleration.rms > 0.0);
            assert!(p.relative_motion.rms.is_finite() && p.relative_motion.rms > 0.0);
            assert!((0.0..=100.0).contains(&p.msi_percent));
            assert!(p.acceleration_omega_e > 0.0);
        }
        assert!(bow.vertical_acceleration.rms > lcg.vertical_acceleration.rms);
        assert!(bow.relative_motion.rms > lcg.relative_motion.rms);
        assert!(bow.msi_percent >= lcg.msi_percent);
        assert!((rep.spectrum.hs_check - 1.0).abs() < 0.02);
        assert_eq!(rep.omega_count, 60);
        // Custom points and the grid argument are honoured.
        let mut spec = demo_spec(sea, 6.0);
        spec.omegas = 30;
        spec.points = vec![MotionPoint {
            name: "Mid".into(),
            x: rep.x_lcg,
        }];
        let r2 = irregular_head_sea(&surface, &spec).expect("irregular");
        assert_eq!(r2.points.len(), 1);
        assert_eq!(r2.omega_count, 30);
    }

    #[test]
    fn from_params_parses_and_rejects() {
        use serde_json::json;
        let spec = IrregularSpec::from_params(
            &json!({"draft": 0.62, "speed_kn": 6, "kg": 1.0, "hs": 1.5, "tp": 6.0,
                    "gamma": 3.3, "omegas": 40,
                    "points": [{"name": "Bridge", "x": 2.5}, {"x": 1.0}]}),
            0.0,
        )
        .expect("valid");
        assert_eq!(
            spec.sea,
            WaveSpectrum::Jonswap {
                hs: 1.5,
                tp: 6.0,
                gamma: 3.3
            }
        );
        assert_eq!(spec.omegas, 40);
        assert_eq!(spec.points[0].name, "Bridge");
        assert_eq!(spec.points[1].name, "P2");
        assert_eq!(spec.seakeeping.speed_kn, 6.0);
        // The default draft fills in; bad shapes are errors.
        assert!(IrregularSpec::from_params(&json!({"hs": 1.0, "tp": 5.0}), 0.5).is_ok());
        for bad in [
            json!({"hs": 1.0, "tp": 5.0}),
            json!({"draft": 0.5, "tp": 5.0}),
            json!({"draft": 0.5, "hs": "x", "tp": 5.0}),
            json!({"draft": 0.5, "hs": 1.0, "tp": 5.0, "gamma": 0.2}),
            json!({"draft": 0.5, "hs": 1.0, "tp": 5.0, "omegas": 2}),
            json!({"draft": 0.5, "hs": 1.0, "tp": 5.0, "omegas": 20.5}),
            json!({"draft": 0.5, "hs": 1.0, "tp": 5.0, "points": 3}),
            json!({"draft": 0.5, "hs": 1.0, "tp": 5.0, "points": [{"name": "a"}]}),
            json!({"draft": -1.0, "hs": 1.0, "tp": 5.0}),
            json!([1, 2]),
        ] {
            assert!(IrregularSpec::from_params(&bad, 0.0).is_err(), "{bad}");
        }
    }

    #[test]
    fn invalid_specs_are_rejected() {
        let surface = crate::demo::demo_hull(true);
        let mut spec = demo_spec(WaveSpectrum::Bretschneider { hs: 1.0, tp: 4.0 }, 0.0);
        spec.omegas = 3;
        assert!(irregular_head_sea(&surface, &spec).is_err());
        let spec = demo_spec(WaveSpectrum::Bretschneider { hs: -1.0, tp: 4.0 }, 0.0);
        assert!(irregular_head_sea(&surface, &spec).is_err());
    }
}
