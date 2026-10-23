//! Seakeeping, 2D part: Lewis conformal sections and the heave added mass and
//! wave damping of a ship cross-section in deep water.
//!
//! The hydrodynamic coefficients come from Frank's close-fit source method on
//! the section contour (a Lewis contour or any half section), so B7b's strip
//! theory can use the real station shapes. Checked against two independent
//! Python solutions (panels, and Ursell's multipoles for the semicircle); see
//! `docs/oracles/heave_2d.py` and the tests.
//!
//! # References
//!
//! * J.M.J. Journée, "Theoretical Manual of SEAWAY" (TU Delft, Ship Hydromechanics
//!   Laboratory report 1370) — Lewis mapping coefficients and the valid range of
//!   the area coefficient ($\sigma_{\min}, \sigma_{\max}$) used for clamping.
//! * F.M. Lewis, "The Inertia of the Water Surrounding a Vibrating Ship",
//!   Transactions SNAME 37 (1929) — the two-parameter conformal mapping and the
//!   infinite-frequency added mass.
//! * W. Frank, "Oscillation of cylinders in or below the free surface of deep
//!   fluids", NSRDC report 2375 (1967) — the close-fit pulsating source method.
//! * F. Ursell, "On the heaving motion of a circular cylinder on the surface of a
//!   fluid", Q. J. Mech. Appl. Math. 2 (1949) — used for the semicircle oracle.

use serde::Serialize;
use std::f64::consts::PI;

/// A cross section represented as a two-parameter Lewis conformal form.
///
/// The physical section $(y, z)$ is mapped from the unit circle in the $\zeta$-plane by:
/// $$y(\theta) = M_s \left( (1 + a_1)\sin\theta - a_3 \sin 3\theta \right)$$
/// $$z(\theta) = -M_s \left( (1 - a_1)\cos\theta + a_3 \cos 3\theta \right)$$
/// where $\theta = 0$ is the keel and $\theta = \pi/2$ is the waterline.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct LewisForm {
    /// Half beam $b$ in metres at the waterline.
    pub half_beam: f64,
    /// Section draft $T$ in metres.
    pub draft: f64,
    /// Sectional area coefficient $\sigma = A / (2 b T)$ (clamped if necessary).
    pub area_coefficient: f64,
    /// First Lewis mapping coefficient $a_1$.
    pub a1: f64,
    /// Second Lewis mapping coefficient $a_3$.
    pub a3: f64,
    /// Scale factor $M_s$ in metres.
    pub scale: f64,
    /// Whether the sectional area coefficient $\sigma$ was clamped to the Journée valid range.
    pub clamped: bool,
}

/// Compute the Lewis form mapping parameters from half-beam, draft, and area coefficient.
///
/// Parameters follow Journée (2003, Report 1370):
/// - $H_0 = b / T$
/// - $d = (H_0 - 1) / (H_0 + 1)$
/// - If $(H_0, \sigma)$ lies outside the range of valid, conformal Lewis forms, $\sigma$
///   is clamped to $[\sigma_{\min}, \sigma_{\max}]$:
///   - For $H_0 \le 1.0$: $\sigma_{\min} = \frac{3\pi}{32} (2 - H_0)$
///   - For $H_0 > 1.0$: $\sigma_{\min} = \frac{3\pi}{32} (2 - 1/H_0)$
///   - $\sigma_{\max} = \frac{\pi}{32} (10 + H_0 + 1/H_0)$
///
/// Returns an error if any input is non-positive or non-finite.
pub fn lewis_form(half_beam: f64, draft: f64, area_coefficient: f64) -> Result<LewisForm, String> {
    if !half_beam.is_finite() || !draft.is_finite() || !area_coefficient.is_finite() {
        return Err("half_beam, draft and area_coefficient must be finite".to_string());
    }
    if half_beam <= 0.0 || draft <= 0.0 || area_coefficient <= 0.0 {
        return Err("half_beam, draft and area_coefficient must be strictly positive".to_string());
    }

    let h0 = half_beam / draft;
    let sigma_min = if h0 <= 1.0 {
        (3.0 * PI / 32.0) * (2.0 - h0)
    } else {
        (3.0 * PI / 32.0) * (2.0 - 1.0 / h0)
    };
    let sigma_max = (PI / 32.0) * (10.0 + h0 + 1.0 / h0);

    let mut clamped = false;
    let mut sigma = area_coefficient;
    if sigma < sigma_min {
        sigma = sigma_min;
        clamped = true;
    } else if sigma > sigma_max {
        sigma = sigma_max;
        clamped = true;
    }

    let d = (h0 - 1.0) / (h0 + 1.0);
    let c1 = 3.0 + (4.0 * sigma / PI) + (1.0 - 4.0 * sigma / PI) * d * d;
    let disc = (9.0 - 2.0 * c1).max(0.0);
    let a3 = (-c1 + 3.0 + disc.sqrt()) / c1;
    let a1 = d * (a3 + 1.0);
    let scale = half_beam / (1.0 + a1 + a3);

    Ok(LewisForm {
        half_beam,
        draft,
        area_coefficient: sigma,
        a1,
        a3,
        scale,
        clamped,
    })
}

/// Sample $n$ points $(y, z)$ along the half-section contour from keel to waterline.
///
/// $z$ is negative downwards; keel is at $\theta = 0$ ($(0, -T)$), and waterline is
/// at $\theta = \pi/2$ ($(b, 0)$).
pub fn lewis_contour(form: &LewisForm, n: usize) -> Vec<(f64, f64)> {
    let count = n.max(2);
    let mut pts = Vec::with_capacity(count);
    for i in 0..count {
        let theta = (PI / 2.0) * (i as f64) / ((count - 1) as f64);
        let y = form.scale * ((1.0 + form.a1) * theta.sin() - form.a3 * (3.0 * theta).sin());
        let z = -form.scale * ((1.0 - form.a1) * theta.cos() + form.a3 * (3.0 * theta).cos());
        pts.push((y, z));
    }
    pts
}

/// Closed-form high-frequency ($\omega \to \infty$) heave added mass of a Lewis section.
///
/// $$m_{a,\infty} = \frac{1}{2} \rho \pi b^2 C_0 = \frac{1}{2} \rho \pi M_s^2 \left( (1 + a_1)^2 + 3 a_3^2 \right)$$
/// For a semicircle ($a_1 = a_3 = 0, M_s = b$), this reduces to $\frac{1}{2} \rho \pi b^2$.
pub fn heave_added_mass_infinite(form: &LewisForm, rho: f64) -> f64 {
    0.5 * rho * PI * form.scale * form.scale * ((1.0 + form.a1).powi(2) + 3.0 * form.a3.powi(2))
}

/// 2D hydrodynamic heave radiation coefficients for a ship cross section.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
pub struct Heave2d {
    /// Two-dimensional heave added mass $A_{33}$ in kg/m.
    pub added_mass: f64,
    /// Two-dimensional heave radiation damping $B_{33}$ in kg/(m·s).
    pub damping: f64,
    /// Non-dimensional radiated wave amplitude ratio $\bar{A} = \eta_a / \zeta_a$.
    pub amplitude_ratio: f64,
}

/// Panels used on a Lewis contour by [`heave_2d`]: 60 gives 0.2 % against 120.
pub const LEWIS_PANELS: usize = 60;

/// 2D heave added mass and damping of a Lewis section ([`heave_2d_section`] on
/// its contour). `omega = f64::INFINITY` gives the infinite-frequency limit.
pub fn heave_2d(form: &LewisForm, omega: f64, rho: f64, g: f64) -> Heave2d {
    heave_2d_section(&lewis_contour(form, LEWIS_PANELS + 1), omega, rho, g)
}

/// 2D heave added mass and damping of a half section by Frank's close-fit
/// source method in deep water.
///
/// `contour` runs from the keel on the centreline to the waterline, `(y, z)`
/// with `y >= 0`, `z <= 0`; the port side is its mirror. Each straight panel
/// carries a constant source strength; the kinematic condition is met at the
/// panel midpoints. `omega = f64::INFINITY` solves the `phi = 0` free-surface
/// limit (damping 0).
///
/// Like every surface source method this has irregular frequencies (for a
/// semicircle the first is near `ω²b/g ≈ 2.4`, for a rectangle at
/// `K = (π/2b)·coth(πT/2b)`); results close to one are unreliable.
pub fn heave_2d_section(contour: &[(f64, f64)], omega: f64, rho: f64, g: f64) -> Heave2d {
    let k = if omega.is_finite() {
        omega * omega / g
    } else {
        f64::INFINITY
    };
    let panels: Vec<Panel> = contour
        .windows(2)
        .filter_map(|w| Panel::new(w[0], w[1]))
        .collect();
    let n = panels.len();
    if n == 0 || k.is_nan() || k <= 0.0 {
        return Heave2d {
            added_mass: f64::NAN,
            damping: f64::NAN,
            amplitude_ratio: f64::NAN,
        };
    }
    // Normal velocity (a) and potential (p) at midpoint i of unit source
    // strength on panel j and its port-side mirror.
    let mut a = vec![C::ZERO; n * n];
    let mut p = vec![C::ZERO; n * n];
    for (j, pj) in panels.iter().enumerate() {
        for mirror in [1.0, -1.0] {
            for (i, pi) in panels.iter().enumerate() {
                let own = i == j && mirror > 0.0;
                let (mut vn, mut ph) = (C::ZERO, C::ZERO);
                for (gx, gw) in GAUSS8 {
                    let s = 0.5 * (gx + 1.0);
                    let src = (mirror * (pj.p0.0 + s * pj.d.0), pj.p0.1 + s * pj.d.1);
                    let (val, grad) = green(pi.mid, src, k, !own);
                    let w = 0.5 * gw * pj.len;
                    ph = ph + val * w;
                    vn = vn + (grad.0 * pi.normal.0 + grad.1 * pi.normal.1) * w;
                }
                if own {
                    // Own panel: ∫ ln r ds at the midpoint, and the jump of the
                    // single layer's normal derivative on the fluid side.
                    ph = ph + C::real(pj.len * ((0.5 * pj.len).ln() - 1.0));
                    vn = vn + C::real(PI);
                }
                a[i * n + j] = a[i * n + j] + vn;
                p[i * n + j] = p[i * n + j] + ph;
            }
        }
    }
    // Unit heave velocity: ∂φ/∂n = n_z.
    let rhs: Vec<C> = panels.iter().map(|pi| C::real(pi.normal.1)).collect();
    let Some(sigma) = solve_complex(a, rhs, n) else {
        return Heave2d {
            added_mass: f64::NAN,
            damping: f64::NAN,
            amplitude_ratio: f64::NAN,
        };
    };
    // ∫ φ n_z ds over both sides; force = -iωρ∫φ n_z ds = iω a33 - b33.
    let mut integral = C::ZERO;
    for (i, pi) in panels.iter().enumerate() {
        let mut phi = C::ZERO;
        for (j, sj) in sigma.iter().enumerate() {
            phi = phi + p[i * n + j] * *sj;
        }
        integral = integral + phi * (2.0 * pi.normal.1 * pi.len);
    }
    let added_mass = -rho * integral.re;
    let damping = if omega.is_finite() {
        -omega * rho * integral.im
    } else {
        0.0
    };
    let amplitude_ratio = if omega.is_finite() {
        (damping.max(0.0) * omega.powi(3) / (rho * g * g)).sqrt()
    } else {
        0.0
    };
    Heave2d {
        added_mass,
        damping,
        amplitude_ratio,
    }
}

struct Panel {
    p0: (f64, f64),
    d: (f64, f64),
    len: f64,
    mid: (f64, f64),
    /// Unit normal into the fluid.
    normal: (f64, f64),
}

impl Panel {
    fn new(p0: (f64, f64), p1: (f64, f64)) -> Option<Panel> {
        let d = (p1.0 - p0.0, p1.1 - p0.1);
        let len = d.0.hypot(d.1);
        if len.is_nan() || len <= 1e-12 {
            return None;
        }
        // Keel → waterline with the hull on the left: the fluid is on the right.
        let normal = (d.1 / len, -d.0 / len);
        let mid = (p0.0 + 0.5 * d.0, p0.1 + 0.5 * d.1);
        Some(Panel {
            p0,
            d,
            len,
            mid,
            normal,
        })
    }
}

/// Deep-water pulsating source at `src` seen from `at` (both `z <= 0`), and its
/// gradient `(∂/∂y, ∂/∂z)`, for `exp(-iωt)` and outgoing waves:
///
/// `G = ln r₁ − ln r₂ − 2 PV∫₀^∞ e^{k(z+ζ)} cos k(y−η) / (k − K) dk − 2πi e^{K(z+ζ)} cos K(y−η)`
///
/// with r₂ to the image above the surface. The principal value is
/// `Re F(W)`, `F = e^{KW} E₁(KW) + iπ sgn(y−η) e^{KW}`, `W = (z+ζ) + i(y−η)`,
/// and `F' = K F − 1/W`. `with_r1 = false` leaves out `ln r₁` (the caller
/// integrates it analytically on the panel's own midpoint).
fn green(at: (f64, f64), src: (f64, f64), k: f64, with_r1: bool) -> (C, (C, C)) {
    let (dy, dz) = (at.0 - src.0, at.1 - src.1);
    let dzi = at.1 + src.1;
    let r2 = dy * dy + dzi * dzi;
    let mut val = -0.5 * r2.ln();
    let (mut gy, mut gz) = (-dy / r2, -dzi / r2);
    if with_r1 {
        let r1 = dy * dy + dz * dz;
        val += 0.5 * r1.ln();
        gy += dy / r1;
        gz += dz / r1;
    }
    let mut v = C::real(val);
    let (mut vy, mut vz) = (C::real(gy), C::real(gz));
    if k.is_finite() {
        let w = C::new(dzi, dy);
        let ekw = (w * k).exp();
        let sgn = if dy > 0.0 {
            1.0
        } else if dy < 0.0 {
            -1.0
        } else {
            0.0
        };
        let f = ekw * e1(w * k) + C::new(0.0, PI * sgn) * ekw;
        let fp = f * k - C::real(1.0) / w;
        let wave = (k * dzi).exp();
        let (c, s) = ((k * dy).cos(), (k * dy).sin());
        v = v + C::new(-2.0 * f.re, -2.0 * PI * wave * c);
        vy = vy + C::new(2.0 * fp.im, 2.0 * PI * k * wave * s);
        vz = vz + C::new(-2.0 * fp.re, -2.0 * PI * k * wave * c);
    }
    (v, (vy, vz))
}

/// Exponential integral `E₁(z)` on the principal branch, for `Re z <= 0` and
/// the right half plane alike: power series near the origin and on the left,
/// continued fraction (modified Lentz) elsewhere.
fn e1(z: C) -> C {
    let r = z.abs();
    if r == 0.0 {
        return C::real(f64::INFINITY);
    }
    if r <= 8.0 || z.re < -z.im.abs() {
        // E₁(z) = −γ − ln z − Σ (−z)ⁿ / (n·n!)
        let mut sum = C::ZERO;
        let mut term = C::real(1.0);
        for n in 1..400 {
            term = term * (-z) * (1.0 / n as f64);
            let add = term * (1.0 / n as f64);
            sum = sum + add;
            if add.abs() < 1e-17 * sum.abs().max(1e-300) {
                break;
            }
        }
        let mut ln = z.ln();
        if z.im == 0.0 && z.re < 0.0 {
            ln.im = 0.0; // on the cut: the mean of both sides (the caller adds the jump)
        }
        return C::real(-std::f64::consts::EULER_GAMMA) - ln - sum;
    }
    // E₁(z) = e^{−z} / (z + 1/(1 + 1/(z + 2/(1 + 2/(z + …)))))
    let tiny = C::real(1e-300);
    let mut f = z;
    let mut c = z;
    let mut d = C::ZERO;
    for i in 1..2000 {
        let an = ((i + 1) / 2) as f64;
        let bn = if i % 2 == 1 { C::real(1.0) } else { z };
        d = bn + d * an;
        if d.abs() == 0.0 {
            d = tiny;
        }
        c = bn + C::real(an) / c;
        if c.abs() == 0.0 {
            c = tiny;
        }
        d = C::real(1.0) / d;
        let delta = c * d;
        f = f * delta;
        if (delta - C::real(1.0)).abs() < 1e-16 {
            break;
        }
    }
    (-z).exp() / f
}

/// Gaussian elimination with partial pivoting on a dense `n × n` system.
fn solve_complex(mut a: Vec<C>, mut b: Vec<C>, n: usize) -> Option<Vec<C>> {
    for col in 0..n {
        let piv =
            (col..n).max_by(|&x, &y| a[x * n + col].abs().total_cmp(&a[y * n + col].abs()))?;
        if a[piv * n + col].abs().is_nan() || a[piv * n + col].abs() <= 1e-300 {
            return None;
        }
        if piv != col {
            for c in 0..n {
                a.swap(piv * n + c, col * n + c);
            }
            b.swap(piv, col);
        }
        let inv = C::real(1.0) / a[col * n + col];
        for row in col + 1..n {
            let factor = a[row * n + col] * inv;
            if factor.abs() == 0.0 {
                continue;
            }
            for c in col..n {
                a[row * n + c] = a[row * n + c] - factor * a[col * n + c];
            }
            b[row] = b[row] - factor * b[col];
        }
    }
    let mut x = vec![C::ZERO; n];
    for row in (0..n).rev() {
        let mut s = b[row];
        for c in row + 1..n {
            s = s - a[row * n + c] * x[c];
        }
        x[row] = s / a[row * n + row];
    }
    x.iter()
        .all(|v| v.re.is_finite() && v.im.is_finite())
        .then_some(x)
}

/// 8-point Gauss–Legendre nodes and weights on [−1, 1] (tabulated digits kept).
#[allow(clippy::excessive_precision)]
const GAUSS8: [(f64, f64); 8] = [
    (-0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
    (-0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (-0.525_532_409_916_329_0, 0.313_706_645_877_887_3),
    (-0.183_434_642_495_649_8, 0.362_683_783_378_362_0),
    (0.183_434_642_495_649_8, 0.362_683_783_378_362_0),
    (0.525_532_409_916_329_0, 0.313_706_645_877_887_3),
    (0.796_666_477_413_626_7, 0.222_381_034_453_374_5),
    (0.960_289_856_497_536_3, 0.101_228_536_290_376_3),
];

/// Minimal complex number for the panel solver (no external dependency).
#[derive(Clone, Copy, Debug, PartialEq)]
struct C {
    re: f64,
    im: f64,
}

impl C {
    const ZERO: C = C { re: 0.0, im: 0.0 };
    fn new(re: f64, im: f64) -> C {
        C { re, im }
    }
    fn real(re: f64) -> C {
        C { re, im: 0.0 }
    }
    fn abs(self) -> f64 {
        self.re.hypot(self.im)
    }
    fn exp(self) -> C {
        let m = self.re.exp();
        C::new(m * self.im.cos(), m * self.im.sin())
    }
    fn ln(self) -> C {
        C::new(self.abs().ln(), self.im.atan2(self.re))
    }
}

impl std::ops::Add for C {
    type Output = C;
    fn add(self, o: C) -> C {
        C::new(self.re + o.re, self.im + o.im)
    }
}
impl std::ops::Sub for C {
    type Output = C;
    fn sub(self, o: C) -> C {
        C::new(self.re - o.re, self.im - o.im)
    }
}
impl std::ops::Neg for C {
    type Output = C;
    fn neg(self) -> C {
        C::new(-self.re, -self.im)
    }
}
impl std::ops::Mul for C {
    type Output = C;
    fn mul(self, o: C) -> C {
        C::new(
            self.re * o.re - self.im * o.im,
            self.re * o.im + self.im * o.re,
        )
    }
}
impl std::ops::Mul<f64> for C {
    type Output = C;
    fn mul(self, s: f64) -> C {
        C::new(self.re * s, self.im * s)
    }
}
impl std::ops::Div for C {
    type Output = C;
    fn div(self, o: C) -> C {
        let d = o.re * o.re + o.im * o.im;
        C::new(
            (self.re * o.re + self.im * o.im) / d,
            (self.im * o.re - self.re * o.im) / d,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::f64::consts::PI;

    #[test]
    fn test_semicircle_coefficients_and_infinite_added_mass() {
        // Semicircle: b = 1, T = 1, sigma = pi/4
        let b = 1.0;
        let t = 1.0;
        let sigma = PI / 4.0;
        let rho = 1025.0;

        let form = lewis_form(b, t, sigma).expect("lewis_form should succeed for semicircle");
        assert!(!form.clamped);
        assert!((form.a1.abs()) < 1e-12, "a1 must be 0, got {}", form.a1);
        assert!((form.a3.abs()) < 1e-12, "a3 must be 0, got {}", form.a3);
        assert!((form.scale - b).abs() < 1e-12);

        let inf_added_mass = heave_added_mass_infinite(&form, rho);
        let expected = 0.5 * rho * PI * b * b;
        let rel_err = ((inf_added_mass - expected) / expected).abs();
        assert!(
            rel_err < 1e-9,
            "infinite added mass relative error must be < 1e-9, got {}",
            rel_err
        );
    }

    #[test]
    fn test_lewis_form_round_trip() {
        // For a grid of (a1, a3) inside the valid region, compute H0, sigma and recover (a1, a3)
        let a3_vals: [f64; 5] = [-0.15, -0.05, 0.0, 0.05, 0.1];
        let a1_vals: [f64; 5] = [-0.2, -0.1, 0.0, 0.1, 0.2];

        for &a3 in &a3_vals {
            for &a1 in &a1_vals {
                let numer_h0 = 1.0 + a1 + a3;
                let denom_h0 = 1.0 - a1 + a3;
                if numer_h0 <= 0.0 || denom_h0 <= 0.0 {
                    continue;
                }
                let h0 = numer_h0 / denom_h0;

                let numer_sigma = 1.0 - a1 * a1 - 3.0 * a3 * a3;
                let denom_sigma = (1.0 + a3).powi(2) - a1 * a1;
                if numer_sigma <= 0.0 || denom_sigma <= 0.0 {
                    continue;
                }
                let sigma = (PI / 4.0) * numer_sigma / denom_sigma;

                let b = h0;
                let t = 1.0;
                if let Ok(recovered) = lewis_form(b, t, sigma) {
                    if !recovered.clamped {
                        assert!(
                            (recovered.a1 - a1).abs() < 1e-9,
                            "a1 mismatch: expected {}, got {}",
                            a1,
                            recovered.a1
                        );
                        assert!(
                            (recovered.a3 - a3).abs() < 1e-9,
                            "a3 mismatch: expected {}, got {}",
                            a3,
                            recovered.a3
                        );
                    }
                }
            }
        }
    }

    #[test]
    fn test_contour_reproduces_beam_draft_and_area() {
        let b = 1.5;
        let t = 1.0;
        let sigma = 0.85;
        let form = lewis_form(b, t, sigma).expect("lewis_form should succeed");

        let n = 2000;
        let pts = lewis_contour(&form, n);

        // Extents check
        let keel = pts[0];
        assert!(keel.0.abs() < 1e-10, "keel y must be 0");
        assert!((keel.1 - (-t)).abs() < 1e-10, "keel z must be -T");

        let waterline = pts[pts.len() - 1];
        assert!((waterline.0 - b).abs() < 1e-10, "waterline y must be b");
        assert!(waterline.1.abs() < 1e-10, "waterline z must be 0");

        // Polygon area using shoelace formula for the closed half section
        // Polygon vertices: (0, 0) -> (0, -T) -> pts[1..] -> (b, 0) -> (0, 0)
        let mut poly = Vec::with_capacity(pts.len() + 1);
        poly.push((0.0, 0.0));
        for pt in &pts {
            poly.push(*pt);
        }

        let mut half_area = 0.0;
        let m = poly.len();
        for i in 0..m {
            let j = (i + 1) % m;
            half_area += poly[i].0 * poly[j].1 - poly[j].0 * poly[i].1;
        }
        half_area = (half_area * 0.5).abs();

        let total_area = 2.0 * half_area;
        let theoretical_area = 2.0 * b * t * form.area_coefficient;
        let area_err = ((total_area - theoretical_area) / theoretical_area).abs();

        assert!(
            area_err < 0.001,
            "polygon area error must be < 0.1%, got {:.5}%",
            area_err * 100.0
        );
    }

    #[test]
    fn infinite_frequency_added_mass_matches_lewis_closed_form() {
        let rho = 1025.0;
        for (b, t, sigma) in [(1.0, 1.0, PI / 4.0), (1.5, 1.0, 0.9), (0.8, 1.2, 0.7)] {
            let form = lewis_form(b, t, sigma).unwrap();
            let exact = heave_added_mass_infinite(&form, rho);
            let panel = heave_2d(&form, f64::INFINITY, rho, 9.81);
            let err = (panel.added_mass - exact).abs() / exact;
            assert!(
                err < 0.01,
                "b={b} T={t} σ={sigma}: {} vs {exact} ({:.2} %)",
                panel.added_mass,
                err * 100.0
            );
            assert_eq!(panel.damping, 0.0);
        }
    }

    /// Oracle: Claude's two independent Python solutions (docs/oracles/heave_2d.py):
    /// Frank close-fit panels (60 and 120 panels agree to 0.2 %) and, for the
    /// semicircle, Ursell's wave source + wave-free multipoles; the two agree to
    /// 0.3 % in added mass and 0.6 % in damping for ξ ≤ 1.5. Columns:
    /// ξ = ω²b/g, a33/(ρπb²/2), b33/(ρπb²/2·ω), Ā.
    #[test]
    fn heave_coefficients_match_the_independent_python_solutions() {
        let (rho, g) = (1025.0, 9.81);
        let semicircle = [
            (0.05, 1.9297, 2.0227, 0.0891),
            (0.25, 0.8820, 1.2490, 0.3502),
            (0.5, 0.6479, 0.8140, 0.5654),
            (0.75, 0.6000, 0.5607, 0.7038),
            (1.0, 0.6084, 0.3973, 0.7900),
            (1.5, 0.6687, 0.2120, 0.8657),
        ];
        let lewis = [
            (0.05, 2.1038, 2.0275, 0.0892),
            (0.25, 1.0506, 1.2742, 0.3537),
            (0.5, 0.8014, 0.8481, 0.5771),
            (0.75, 0.7401, 0.5940, 0.7244),
            (1.0, 0.7391, 0.4253, 0.8174),
            (1.5, 0.7912, 0.2265, 0.8947),
        ];
        for (form, table) in [
            (lewis_form(1.0, 1.0, PI / 4.0).unwrap(), &semicircle),
            (lewis_form(1.5, 1.0, 0.9).unwrap(), &lewis),
        ] {
            let b = form.half_beam;
            let norm = 0.5 * rho * PI * b * b;
            for &(xi, am, dm, abar) in table.iter() {
                let omega = (xi * g / b).sqrt();
                let h = heave_2d(&form, omega, rho, g);
                let close = |got: f64, want: f64, tol: f64| (got - want).abs() <= tol * want;
                assert!(
                    close(h.added_mass / norm, am, 0.015),
                    "b={b} ξ={xi}: a {} vs {am}",
                    h.added_mass / norm
                );
                assert!(
                    close(h.damping / (norm * omega), dm, 0.02),
                    "b={b} ξ={xi}: b {} vs {dm}",
                    h.damping / (norm * omega)
                );
                assert!(
                    close(h.amplitude_ratio, abar, 0.015),
                    "b={b} ξ={xi}: Ā {} vs {abar}",
                    h.amplitude_ratio
                );
            }
        }
    }

    #[test]
    fn panel_count_has_converged_and_damping_is_positive() {
        let (rho, g) = (1025.0, 9.81);
        let form = lewis_form(1.2, 1.0, 0.8).unwrap();
        for xi in [0.05, 0.1, 0.25, 0.5, 1.0, 1.5] {
            let omega = (xi * g / form.half_beam).sqrt();
            let coarse = heave_2d_section(&lewis_contour(&form, 61), omega, rho, g);
            let fine = heave_2d_section(&lewis_contour(&form, 121), omega, rho, g);
            assert!(coarse.damping > 0.0, "ξ={xi}");
            assert!(
                (coarse.added_mass - fine.added_mass).abs() < 0.01 * fine.added_mass,
                "ξ={xi}"
            );
            assert!(
                (coarse.damping - fine.damping).abs() < 0.01 * fine.damping,
                "ξ={xi}"
            );
        }
    }

    #[test]
    fn exponential_integral_matches_reference_values() {
        // E₁(1) = 0.219383934395520, E₁(10) = 4.15696892968532e-6 (A&S table 5.1);
        // E₁(−1 ± i0) has real part −Ei(1) = −1.89511781635594.
        let e = |re: f64, im: f64| e1(C::new(re, im));
        assert!((e(1.0, 0.0).re - 0.219_383_934_395_520).abs() < 1e-13);
        assert!((e(10.0, 0.0).re - 4.156_968_929_685_32e-6).abs() < 1e-17);
        assert!((e(-1.0, 0.0).re + 1.895_117_816_355_94).abs() < 1e-12);
        // Both evaluation branches agree where they meet (|z| = 8 on the imaginary axis).
        let (a, b) = (e(-1e-9, 8.0 - 1e-9), e(1e-9, 8.0 + 1e-9));
        assert!((a - b).abs() < 1e-8, "{a:?} vs {b:?}");
    }

    #[test]
    fn test_out_of_range_input_clamped() {
        // Very small sigma should clamp to sigma_min
        let low_sigma = lewis_form(1.0, 1.0, 0.1).unwrap();
        assert!(low_sigma.clamped, "low sigma should be clamped");
        assert!(low_sigma.a1.is_finite());
        assert!(low_sigma.a3.is_finite());
        assert!(low_sigma.scale.is_finite());

        // Very high sigma should clamp to sigma_max
        let high_sigma = lewis_form(1.0, 1.0, 1.5).unwrap();
        assert!(high_sigma.clamped, "high sigma should be clamped");
        assert!(high_sigma.a1.is_finite());
        assert!(high_sigma.a3.is_finite());
        assert!(high_sigma.scale.is_finite());

        // Non-positive input rejection
        assert!(lewis_form(0.0, 1.0, 0.8).is_err());
        assert!(lewis_form(1.0, -1.0, 0.8).is_err());
        assert!(lewis_form(1.0, 1.0, 0.0).is_err());
    }

    #[test]
    #[ignore]
    fn test_heave_benchmark_table_print() {
        let rho = 1025.0;
        let g = 9.81;
        let frequencies = [0.1, 0.25, 0.5, 0.75, 1.0, 1.5, 2.0];

        // Section (a): semicircle b = 1, T = 1, sigma = pi/4
        let semi = lewis_form(1.0, 1.0, PI / 4.0).unwrap();
        println!("\n--- Semicircle (b = 1.0 m, T = 1.0 m, sigma = pi/4) ---");
        println!(
            "{:<10} {:<24} {:<24} {:<18}",
            "w^2*b/g", "added_mass/(rho*pi*b^2/2)", "damping/(rho*pi*b^2/2*w)", "amplitude_ratio"
        );
        for &xi in &frequencies {
            let omega = (xi * g / semi.half_beam).sqrt();
            let res = heave_2d(&semi, omega, rho, g);
            let denom_m = 0.5 * rho * PI * semi.half_beam * semi.half_beam;
            let denom_d = denom_m * omega;
            let nondim_m = res.added_mass / denom_m;
            let nondim_d = res.damping / denom_d;
            println!(
                "{:<10.2} {:<24.6} {:<24.6} {:<18.6}",
                xi, nondim_m, nondim_d, res.amplitude_ratio
            );
        }

        // Section (b): b = 1.5, T = 1.0, sigma = 0.9
        let ship = lewis_form(1.5, 1.0, 0.9).unwrap();
        println!("\n--- Ship Section (b = 1.5 m, T = 1.0 m, sigma = 0.9) ---");
        println!(
            "{:<10} {:<24} {:<24} {:<18}",
            "w^2*b/g", "added_mass/(rho*pi*b^2/2)", "damping/(rho*pi*b^2/2*w)", "amplitude_ratio"
        );
        for &xi in &frequencies {
            let omega = (xi * g / ship.half_beam).sqrt();
            let res = heave_2d(&ship, omega, rho, g);
            let denom_m = 0.5 * rho * PI * ship.half_beam * ship.half_beam;
            let denom_d = denom_m * omega;
            let nondim_m = res.added_mass / denom_m;
            let nondim_d = res.damping / denom_d;
            println!(
                "{:<10.2} {:<24.6} {:<24.6} {:<18.6}",
                xi, nondim_m, nondim_d, res.amplitude_ratio
            );
        }
    }
}
