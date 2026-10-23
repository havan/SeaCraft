//! Seakeeping: Lewis conformal sections, 2D heave added mass/damping by Frank close-fit panels,
//! and linear strip-theory heave and pitch motion RAOs in regular head seas.
//!
//! # 2D Radiation Problem (B7a)
//!
//! The 2D hydrodynamic coefficients come from Frank's close-fit source method on
//! the section contour (a Lewis contour or any half section), so B7b's strip
//! theory can use the real station shapes. Checked against two independent
//! Python solutions (panels, and Ursell's multipoles for the semicircle); see
//! `docs/oracles/heave_2d.py` and the tests.
//!
//! ## Irregular frequencies and the interior lid
//!
//! A source distribution on the hull alone fails at the irregular frequencies,
//! the eigenfrequencies of the interior (sloshing) problem of the body below
//! the free surface (for the semicircle `K R ≈ 1.82` and `4.78`, where the
//! added mass is off by up to 60 %). Following Ohmatsu (1975) the waterline is
//! closed from inside by a lid of source panels on `z = 0` between the
//! waterline and the centreline, carrying `∂φ/∂z = 0`. The interior problem
//! with a rigid lid has only the trivial solution, so the irregular
//! frequencies disappear; the forces are still integrated over the hull only.
//! Imposing `φ = 0` on the lid instead (the Dirichlet variant) does *not*
//! work: it diverges with panel refinement at the waterline corner.
//! [`heave_2d_section_with`] with `lid = false` is the plain method.
//! Checked against Ursell's multipoles for the semicircle in
//! `docs/oracles/heave_2d_lid.py` (docs/oracles.md §18).
//!
//! # Strip Theory Heave and Pitch Motions (B7b)
//!
//! Linear strip-theory heave and pitch motion Response Amplitude Operators (RAOs)
//! in regular head seas, following:
//!
//! * N. Salvesen, E.O. Tuck, and O.M. Faltinsen, "Ship motions and sea loads",
//!   Transactions SNAME 78, pp. 250–287 (1970).
//!
//! The transom boundary terms (see below) are included unless switched off with
//! [`SeakeepingSpec::transom_terms`].
//!
//! Speed terms in A35/A53/B35/B53 and the U/(iω_e)·H₃ moment term follow from the
//! relative-motion sectional force −D/Dt[(a + b/iω_e)·V], D/Dt = ∂t − U∂x, by
//! parts, for pitch positive bow up. Restoring C33/C35/C55 are integrated over the
//! same stations as the excitation (plus ρg∇(KB − KG) in C55), so very long waves
//! give heave → 1 and pitch → ∫2b·x² / (∫2b·x² + ∇(KB − KG)) at any speed
//! (= 1 when KG = KB); see `very_long_waves_are_followed_at_any_speed`.
//!
//! ## Transom terms
//!
//! Time factor `e^{iω_e t}`, complex sectional mass `m = a − i·b/ω_e`, relative
//! vertical velocity `w − v` with `w = iω_e(η₃ + xη₅) − Uη₅` the section's
//! velocity and `v = iω κ e^{ikx}` (`κ = e^{−kT*}`) the wave's. The sectional
//! force is `−D/Dt[m·(w − v)]`; integrating `U ∂ₓ[m(w − v)]` by parts over the
//! hull gives, besides the speed terms above, the boundary term at the aft end
//! `x_A` (the forward end has no section, `m = 0`):
//!
//! `∫ −D/Dt[m(w − v)] dx ∋ U·m_A·(v_A − w_A)`.
//!
//! The `w_A` part is the radiation transom term; with `m_A = a_A − i b_A/ω_e`,
//! `a_A, b_A` the 2-D coefficients of the aftmost station (zero for a closed
//! stern), it adds
//!
//! * `ΔA33 = −U b_A/ω_e²`, `ΔB33 = U a_A`
//! * `ΔA35 = (−U x_A b_A + U² a_A)/ω_e²`, `ΔB35 = U x_A a_A + U² b_A/ω_e²`
//! * `ΔA53 = −U x_A b_A/ω_e²`, `ΔB53 = U x_A a_A`
//! * `ΔA55 = (−U x_A² b_A + U² x_A a_A)/ω_e²`, `ΔB55 = U x_A² a_A + U² x_A b_A/ω_e²`
//!
//! (that is `ΔK = −ω_e²ΔA + iω_eΔB = U m_A·(iω_e, iω_e x_A − U)` times 1 and
//! `x_A`), and the `v_A` part is the diffraction term
//! `ΔF₃ = U m_A v_A`, `ΔF₅ = x_A ΔF₃` with `v_A = iω κ_A e^{ik x_A}`. In very
//! long waves the hull follows the wave (`w → v`), so the radiation and the
//! excitation transom terms cancel and the long-wave limit above is unchanged
//! at any speed: that test is the sign check of these terms.
//!
//! # Sign Conventions
//!
//! * **Axes**: $x$ forward from the longitudinal centre of gravity (LCG = LCB at equilibrium),
//!   $y$ to starboard, $z$ upwards.
//! * **Heave $\eta_3$ (or $z$)**: positive upwards.
//! * **Pitch $\eta_5$ (or $\theta$)**: positive **bow up** (SeaCraft's trim convention).
//! * **Incident wave**: regular deep-water wave in head seas ($\beta = 180^\circ$) propagating
//!   towards $-x$ (bow to stern), with elevation at LCG given by $\zeta(t) = \text{Re}\{\zeta_a e^{i \omega_e t}\}$.
//! * **Encounter frequency**: $\omega_e = \omega + \omega^2 U / g$ where $U$ is forward speed.
//!
//! # Exciting Force Formulation
//!
//! Wave exciting heave force and pitch moment are evaluated using the STF strip-theory
//! formulation combining Froude–Krylov restoring pressure with diffraction forces
//! derived from the 2D sectional radiation coefficients $a_{33}(x)$ and $b_{33}(x)$,
//! attenuated exponentially with mean sectional draft $T^*(x) = A(x) / (2 b(x))$
//! through the depth attenuation factor $e^{-k T^*(x)}$ where $k = \omega^2 / g$.
//!
//! # References
//!
//! * N. Salvesen, E.O. Tuck, and O.M. Faltinsen, "Ship motions and sea loads",
//!   Transactions SNAME 78, pp. 250–287 (1970).
//! * J.M.J. Journée, "Theoretical Manual of SEAWAY" (TU Delft, Ship Hydromechanics
//!   Laboratory report 1370) — Lewis mapping coefficients and the valid range of
//!   the area coefficient ($\sigma_{\min}, \sigma_{\max}$) used for clamping.
//! * F.M. Lewis, "The Inertia of the Water Surrounding a Vibrating Ship",
//!   Transactions SNAME 37 (1929) — the two-parameter conformal mapping and the
//!   infinite-frequency added mass.
//! * W. Frank, "Oscillation of cylinders in or below the free surface of deep
//!   fluids", NSRDC report 2375 (1967) — the close-fit pulsating source method.
//! * M. Ohmatsu, "On the irregular frequencies in the theory of oscillating
//!   bodies in a free surface", Papers of Ship Research Institute 48 (1975) —
//!   the interior free-surface lid that removes them.
//! * F. Ursell, "On the heaving motion of a circular cylinder on the surface of a
//!   fluid", Q. J. Mech. Appl. Math. 2 (1949) — used for the semicircle oracle.

use crate::curves;
use crate::hydro::{self, Hull, HydroOptions, Warning};
use crate::mesh::Surface;
use crate::subdiv::SubdivisionOptions;
use serde::{Deserialize, Serialize};
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
/// The plain surface source method has irregular frequencies (for a
/// semicircle near `K R ≈ 1.82` and `4.78`, for a rectangle at
/// `K = (π/2b)·coth(πT/2b)`); this entry point removes them with an interior
/// free-surface lid, see [`heave_2d_section_with`].
pub fn heave_2d_section(contour: &[(f64, f64)], omega: f64, rho: f64, g: f64) -> Heave2d {
    heave_2d_section_with(contour, omega, rho, g, true)
}

/// [`heave_2d_section`] with the interior-lid irregular-frequency removal
/// switched by `lid`. `lid = false` is the plain Frank method, which is
/// unreliable near its irregular frequencies (kept for the oracle test that
/// documents them).
///
/// The lid is `nlid = max(4, ⌈b_wl / mean hull panel length⌉)` source panels on
/// `z = 0` from the waterline end of the contour to the centreline, carrying
/// the interior condition `∂φ/∂z = 0` (lid normal `+z`). It is skipped for a
/// zero waterline half-beam, for a contour that does not end on `z = 0`, and
/// for `omega = ∞` (`K = ∞` has no irregular frequencies). Forces are
/// integrated over the hull panels only.
pub fn heave_2d_section_with(
    contour: &[(f64, f64)],
    omega: f64,
    rho: f64,
    g: f64,
    lid: bool,
) -> Heave2d {
    let k = if omega.is_finite() {
        omega * omega / g
    } else {
        f64::INFINITY
    };
    let mut panels: Vec<Panel> = contour
        .windows(2)
        .filter_map(|w| Panel::new(w[0], w[1]))
        .collect();
    let nh = panels.len();
    if nh == 0 || k.is_nan() || k <= 0.0 {
        return Heave2d {
            added_mass: f64::NAN,
            damping: f64::NAN,
            amplitude_ratio: f64::NAN,
        };
    }
    // Interior free-surface lid on z = 0, waterline → centreline (Ohmatsu 1975).
    if lid && k.is_finite() {
        let (b_wl, z_wl) = *contour.last().unwrap_or(&(0.0, 0.0));
        if b_wl >= 1e-9 && z_wl.abs() <= 1e-6 * (1.0 + b_wl) {
            let mean_len = panels.iter().map(|p| p.len).sum::<f64>() / nh as f64;
            let nlid = ((b_wl / mean_len).ceil() as usize).max(4);
            for l in 0..nlid {
                let y0 = b_wl * (1.0 - l as f64 / nlid as f64);
                let y1 = b_wl * (1.0 - (l + 1) as f64 / nlid as f64);
                if let Some(p) = Panel::new((y0, 0.0), (y1, 0.0)) {
                    panels.push(p);
                }
            }
        }
    }
    let n = panels.len();
    // Normal velocity (a) and potential (p) at midpoint i of unit source
    // strength on panel j and its port-side mirror.
    let mut a = vec![C::ZERO; n * n];
    let mut p = vec![C::ZERO; n * n];
    for (j, pj) in panels.iter().enumerate() {
        for mirror in [1.0, -1.0] {
            for (i, pi) in panels.iter().enumerate() {
                let own = i == j && mirror > 0.0;
                let hull_own = own && j < nh;
                let lid_own = own && j >= nh;
                let (mut vn, mut ph) = (C::ZERO, C::ZERO);
                for (gx, gw) in GAUSS8 {
                    let s = 0.5 * (gx + 1.0);
                    let src = (mirror * (pj.p0.0 + s * pj.d.0), pj.p0.1 + s * pj.d.1);
                    let (val, grad) = green(pi.mid, src, k, !hull_own);
                    let w = 0.5 * gw * pj.len;
                    if lid_own {
                        // The wave part carries 2 ln|Δy| (the Rankine terms
                        // cancel on z = 0); integrate that analytically below.
                        ph = ph + (val - C::real(2.0 * (pi.mid.0 - src.0).abs().ln())) * w;
                    } else {
                        ph = ph + val * w;
                    }
                    vn = vn + (grad.0 * pi.normal.0 + grad.1 * pi.normal.1) * w;
                }
                if hull_own {
                    // Own panel: ∫ ln r ds at the midpoint, and the jump of the
                    // single layer's normal derivative on the fluid side.
                    ph = ph + C::real(pj.len * ((0.5 * pj.len).ln() - 1.0));
                    vn = vn + C::real(PI);
                }
                if lid_own {
                    // ∫ 2 ln|s| ds over the panel, and the jump −2π seen from
                    // below (a source sheet on z = 0 radiates 2π σ each way).
                    ph = ph + C::real(2.0 * pj.len * ((0.5 * pj.len).ln() - 1.0));
                    vn = vn + C::real(-2.0 * PI);
                }
                a[i * n + j] = a[i * n + j] + vn;
                p[i * n + j] = p[i * n + j] + ph;
            }
        }
    }
    // Unit heave velocity on the hull: ∂φ/∂n = n_z; ∂φ/∂z = 0 on the lid.
    let rhs: Vec<C> = panels
        .iter()
        .enumerate()
        .map(|(i, pi)| {
            if i < nh {
                C::real(pi.normal.1)
            } else {
                C::ZERO
            }
        })
        .collect();
    let Some(sigma) = solve_complex(a, rhs, n) else {
        return Heave2d {
            added_mass: f64::NAN,
            damping: f64::NAN,
            amplitude_ratio: f64::NAN,
        };
    };
    // ∫ φ n_z ds over both sides of the hull; force = -iωρ∫φ n_z ds = iω a33 - b33.
    let mut integral = C::ZERO;
    for (i, pi) in panels.iter().take(nh).enumerate() {
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

// ============================================================================
// B7b: Linear Strip-Theory Heave & Pitch Motions in Regular Head Seas
// ============================================================================

/// Input specification for strip-theory heave and pitch motion calculations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SeakeepingSpec {
    /// Design draft in metres.
    pub draft: f64,
    /// Forward speed in knots (default 0.0).
    #[serde(default)]
    pub speed_kn: f64,
    /// Vertical centre of gravity KG above the keel datum in metres (default 0.0).
    #[serde(default)]
    pub kg: f64,
    /// Pitch radius of gyration in metres. Defaults to 0.25 * Lwl if None or <= 0.
    #[serde(default)]
    pub kyy: Option<f64>,
    /// Number of transverse stations along the waterline (default 21).
    #[serde(default = "default_stations")]
    pub stations: usize,
    /// Wave circular frequencies in rad/s. If empty, auto-generated from lambda/L range.
    #[serde(default)]
    pub omegas: Vec<f64>,
    /// Minimum wavelength ratio lambda / Lwl (default 0.3) when omegas is empty.
    #[serde(default)]
    pub lambda_min: Option<f64>,
    /// Maximum wavelength ratio lambda / Lwl (default 3.0) when omegas is empty.
    #[serde(default)]
    pub lambda_max: Option<f64>,
    /// Number of points across lambda/L range (default 30) when omegas is empty.
    #[serde(default)]
    pub points: Option<usize>,
    /// Water density in kg/m^3 (default 1025.0).
    #[serde(default = "default_rho")]
    pub rho: f64,
    /// Acceleration of gravity in m/s^2 (default 9.81).
    #[serde(default = "default_g")]
    pub g: f64,
    /// Apply the STF transom boundary terms at the aft end of the strip
    /// integration (default true; see the module documentation).
    #[serde(default = "default_transom_terms")]
    pub transom_terms: bool,
}

fn default_transom_terms() -> bool {
    true
}

fn default_stations() -> usize {
    21
}

fn default_rho() -> f64 {
    1025.0
}

fn default_g() -> f64 {
    9.81
}

impl Default for SeakeepingSpec {
    fn default() -> Self {
        Self {
            draft: 0.0,
            speed_kn: 0.0,
            kg: 0.0,
            kyy: None,
            stations: 21,
            omegas: Vec::new(),
            lambda_min: None,
            lambda_max: None,
            points: None,
            rho: 1025.0,
            g: 9.81,
            transom_terms: true,
        }
    }
}

/// Motion response amplitude operator (RAO) and phase at one wave frequency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RaoPoint {
    /// Incident wave frequency in rad/s.
    pub omega: f64,
    /// Encounter frequency in rad/s: omega_e = omega + omega^2 * U / g.
    pub omega_e: f64,
    /// Deep-water incident wavelength in metres: lambda = 2 * pi * g / omega^2.
    pub wavelength_m: f64,
    /// Non-dimensional wavelength ratio lambda / Lwl.
    pub lambda_over_l: f64,
    /// Non-dimensional heave amplitude z_a / zeta_a (m/m).
    pub heave_amp: f64,
    /// Heave phase angle in degrees.
    pub heave_phase_deg: f64,
    /// Non-dimensional pitch amplitude theta_a / (k * zeta_a) (rad/rad).
    pub pitch_amp: f64,
    /// Pitch phase angle in degrees.
    pub pitch_phase_deg: f64,
}

/// Global 3D hydrodynamic and hydrostatic coefficients at one encounter frequency.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GlobalCoefficients {
    pub omega: f64,
    pub omega_e: f64,
    pub a33: f64,
    pub b33: f64,
    pub a35: f64,
    pub b35: f64,
    pub a53: f64,
    pub b53: f64,
    pub a55: f64,
    pub b55: f64,
    pub c33: f64,
    pub c35: f64,
    pub c53: f64,
    pub c55: f64,
}

/// Report containing heave and pitch motion RAOs, natural frequencies and warnings.
#[derive(Debug, Clone, Serialize)]
pub struct SeakeepingReport {
    pub frequencies: Vec<RaoPoint>,
    pub natural_heave_omega: f64,
    pub natural_pitch_omega: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub coefficients: Option<Vec<GlobalCoefficients>>,
    pub warnings: Vec<Warning>,
    /// Longitudinal centre of gravity (= LCB at equilibrium) in model x, metres.
    /// This is the origin of the motion coordinates used by the RAOs.
    pub x_lcg: f64,
    /// Forward end of the design waterline in model x, metres.
    pub x_bow: f64,
}

struct ExtractedSection {
    contour: Vec<(f64, f64)>,
    half_beam: f64,
    draft: f64,
    area: f64,
    valid: bool,
}

fn clip_halfplane(pts: &[(f64, f64)], nx: f64, ny: f64, d: f64) -> Vec<(f64, f64)> {
    let mut out = Vec::new();
    let n = pts.len();
    if n < 3 {
        return out;
    }
    for i in 0..n {
        let j = (i + 1) % n;
        let p1 = pts[i];
        let p2 = pts[j];
        let s1 = nx * p1.0 + ny * p1.1 - d;
        let s2 = nx * p2.0 + ny * p2.1 - d;
        if s1 <= 1e-9 {
            out.push(p1);
        }
        if (s1 < -1e-9 && s2 > 1e-9) || (s1 > 1e-9 && s2 < -1e-9) {
            let t = s1 / (s1 - s2);
            out.push((p1.0 + t * (p2.0 - p1.0), p1.1 + t * (p2.1 - p1.1)));
        }
    }
    out
}

fn polygon_area_2d(pts: &[(f64, f64)]) -> f64 {
    let mut a = 0.0;
    let n = pts.len();
    for i in 0..n {
        let j = (i + 1) % n;
        a += pts[i].0 * pts[j].1 - pts[j].0 * pts[i].1;
    }
    0.5 * a
}

fn extract_submerged_starboard(polys: &[Vec<crate::math::Vec3>], zwl: f64) -> ExtractedSection {
    let mut best_area = 0.0;
    let mut best_poly: Vec<(f64, f64)> = Vec::new();
    let mut best_b = 0.0;
    let mut best_t = 0.0;

    for poly in polys {
        if poly.len() < 3 {
            continue;
        }
        let pts2d: Vec<(f64, f64)> = poly.iter().map(|p| (p.y, p.z)).collect();
        // Clip against z <= zwl
        let c1 = clip_halfplane(&pts2d, 0.0, 1.0, zwl);
        // Clip against y >= 0
        let c2 = clip_halfplane(&c1, -1.0, 0.0, 0.0);
        let area = polygon_area_2d(&c2).abs();
        if area > best_area {
            best_area = area;
            let mut max_y = 0.0_f64;
            let mut min_z = zwl;
            for pt in &c2 {
                max_y = max_y.max(pt.0);
                min_z = min_z.min(pt.1);
            }
            best_poly = c2;
            best_b = max_y;
            best_t = (zwl - min_z).max(0.0);
        }
    }

    if best_area < 1e-6 || best_b < 1e-4 || best_t < 1e-4 || best_poly.len() < 3 {
        return ExtractedSection {
            contour: Vec::new(),
            half_beam: best_b,
            draft: best_t,
            area: 2.0 * best_area,
            valid: false,
        };
    }

    let n = best_poly.len();
    let min_z = zwl - best_t;

    let mut keel_idx = 0;
    let mut min_keel_dist = f64::INFINITY;
    for (i, p) in best_poly.iter().enumerate() {
        let d = p.0.hypot(p.1 - min_z);
        if d < min_keel_dist {
            min_keel_dist = d;
            keel_idx = i;
        }
    }

    let mut wl_idx = 0;
    let mut min_wl_dist = f64::INFINITY;
    for (i, p) in best_poly.iter().enumerate() {
        let d = (p.0 - best_b).hypot(p.1 - zwl);
        if d < min_wl_dist {
            min_wl_dist = d;
            wl_idx = i;
        }
    }

    if keel_idx == wl_idx {
        return ExtractedSection {
            contour: Vec::new(),
            half_beam: best_b,
            draft: best_t,
            area: 2.0 * best_area,
            valid: false,
        };
    }

    let mut path_fwd = Vec::new();
    let mut idx = keel_idx;
    while idx != wl_idx {
        path_fwd.push(best_poly[idx]);
        idx = (idx + 1) % n;
    }
    path_fwd.push(best_poly[wl_idx]);

    let mut path_bwd = Vec::new();
    let mut idx2 = keel_idx;
    while idx2 != wl_idx {
        path_bwd.push(best_poly[idx2]);
        idx2 = (idx2 + n - 1) % n;
    }
    path_bwd.push(best_poly[wl_idx]);

    let dist_corner = |path: &[(f64, f64)]| -> f64 {
        path.iter()
            .map(|p| p.0.hypot(p.1 - zwl))
            .fold(f64::INFINITY, |m, v| m.min(v))
    };
    let d_fwd = dist_corner(&path_fwd);
    let d_bwd = dist_corner(&path_bwd);

    let hull_path = if d_fwd > d_bwd { path_fwd } else { path_bwd };

    if hull_path.len() < 2 {
        return ExtractedSection {
            contour: Vec::new(),
            half_beam: best_b,
            draft: best_t,
            area: 2.0 * best_area,
            valid: false,
        };
    }

    let mut contour: Vec<(f64, f64)> = hull_path
        .iter()
        .map(|&(y, z)| (y.max(0.0), (z - zwl).min(0.0)))
        .collect();
    if let Some(first) = contour.first_mut() {
        first.0 = 0.0;
        first.1 = -best_t;
    }
    if let Some(last) = contour.last_mut() {
        last.0 = best_b;
        last.1 = 0.0;
    }

    let mut clean_contour: Vec<(f64, f64)> = Vec::with_capacity(contour.len());
    for p in contour {
        if let Some(prev) = clean_contour.last() {
            if (p.0 - prev.0).hypot(p.1 - prev.1) > 1e-6 {
                clean_contour.push(p);
            }
        } else {
            clean_contour.push(p);
        }
    }

    let valid = clean_contour.len() >= 2;
    let final_contour = if valid {
        resample_contour(&clean_contour, 31)
    } else {
        clean_contour
    };
    ExtractedSection {
        contour: final_contour,
        half_beam: best_b,
        draft: best_t,
        area: 2.0 * best_area,
        valid,
    }
}

fn resample_contour(pts: &[(f64, f64)], count: usize) -> Vec<(f64, f64)> {
    if pts.len() <= 2 || count <= 2 {
        return pts.to_vec();
    }
    let mut dists = Vec::with_capacity(pts.len());
    dists.push(0.0);
    let mut total_len = 0.0;
    for w in pts.windows(2) {
        total_len += (w[1].0 - w[0].0).hypot(w[1].1 - w[0].1);
        dists.push(total_len);
    }
    if total_len < 1e-9 {
        return pts.to_vec();
    }

    let mut resampled = Vec::with_capacity(count);
    resampled.push(pts[0]);

    let mut seg = 0;
    for k in 1..count - 1 {
        let target_s = total_len * (k as f64) / ((count - 1) as f64);
        while seg + 1 < dists.len() && dists[seg + 1] < target_s {
            seg += 1;
        }
        let s0 = dists[seg];
        let s1 = dists[seg + 1];
        let ds = s1 - s0;
        let t = if ds > 1e-12 {
            (target_s - s0) / ds
        } else {
            0.0
        };
        let y = pts[seg].0 + t * (pts[seg + 1].0 - pts[seg].0);
        let z = pts[seg].1 + t * (pts[seg + 1].1 - pts[seg].1);
        resampled.push((y, z));
    }
    resampled.push(*pts.last().unwrap());
    resampled
}

fn trapz(xs: &[f64], ys: &[f64]) -> f64 {
    let mut sum = 0.0;
    for i in 0..xs.len().saturating_sub(1) {
        let dx = xs[i + 1] - xs[i];
        sum += 0.5 * (ys[i] + ys[i + 1]) * dx;
    }
    sum
}

fn trapz_complex(xs: &[f64], ys: &[C]) -> C {
    let mut sum = C::ZERO;
    for i in 0..xs.len().saturating_sub(1) {
        let dx = xs[i + 1] - xs[i];
        sum = sum + (ys[i] + ys[i + 1]) * (0.5 * dx);
    }
    sum
}

/// Compute linear strip-theory heave and pitch motion RAOs in regular head seas.
///
/// Implements Salvesen, Tuck & Faltinsen (1970) strip theory including the
/// transom boundary terms ([`SeakeepingSpec::transom_terms`]).
///
/// # Sign Conventions
///
/// * Coordinate system: $x$ forward from LCG (where LCG = LCB at equilibrium), $y$ starboard, $z$ upwards.
/// * Heave $z$ is positive upwards.
/// * Pitch $\theta$ is positive **bow up** (SeaCraft's trim convention).
///
/// # References
///
/// * N. Salvesen, E.O. Tuck, and O.M. Faltinsen, "Ship motions and sea loads",
///   Transactions SNAME 78, pp. 250–287 (1970).
pub fn head_sea_raos(surface: &Surface, spec: &SeakeepingSpec) -> Result<SeakeepingReport, String> {
    head_sea_raos_detailed(surface, spec).map(|(report, _)| report)
}

/// The aft end of the strip integration at one wave frequency: the data the
/// transom boundary terms are built from (recorded whether or not
/// [`SeakeepingSpec::transom_terms`] applies them).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct AftStation {
    /// Incident wave frequency in rad/s.
    pub omega: f64,
    /// Encounter frequency in rad/s.
    pub omega_e: f64,
    /// Station position `x_A` from the LCG in metres (negative aft).
    pub x_a: f64,
    /// 2-D heave added mass `a_A` of the aftmost station (0 for a closed stern).
    pub a_a: f64,
    /// 2-D heave damping `b_A` of the aftmost station (0 for a closed stern).
    pub b_a: f64,
}

/// [`head_sea_raos`] that also returns the [`AftStation`] of every frequency.
pub(crate) fn head_sea_raos_detailed(
    surface: &Surface,
    spec: &SeakeepingSpec,
) -> Result<(SeakeepingReport, Vec<AftStation>), String> {
    if spec.draft <= 0.0 || !spec.draft.is_finite() {
        return Err("draft must be positive and finite".into());
    }
    if spec.rho <= 0.0 || !spec.rho.is_finite() {
        return Err("rho must be positive and finite".into());
    }
    if spec.g <= 0.0 || !spec.g.is_finite() {
        return Err("g must be positive and finite".into());
    }

    let u_speed = spec.speed_kn * 0.5144444444444445;
    let subdiv = SubdivisionOptions::default();
    let hull = Hull::new(surface, subdiv);
    let mut hydro_opts = HydroOptions::from_surface(surface, subdiv);
    hydro_opts.draft = spec.draft;
    hydro_opts.water_density = spec.rho;
    let hydro = hull.hydrostatics(&hydro_opts);

    if hydro.volume_m3 <= 1e-6 || !hydro.volume_m3.is_finite() {
        return Err(format!(
            "hull displaces no volume at draft {:.3} m",
            spec.draft
        ));
    }

    let zwl = hull.keel + spec.draft;
    let wp = hydro::waterplane_for(hull.keel, hull.length, spec.draft, 0.0, 0.0);
    let segs = hydro::plane_segments(&hull.verts, &hull.tris, &wp);
    let (mut wl_min, mut wl_max) = (f64::INFINITY, f64::NEG_INFINITY);
    for (a, b) in &segs {
        wl_min = wl_min.min(a.x).min(b.x);
        wl_max = wl_max.max(a.x).max(b.x);
    }
    if !wl_min.is_finite() || !wl_max.is_finite() || wl_max <= wl_min {
        wl_min = hull.model_min.x;
        wl_max = hull.model_max.x;
    }
    let lwl = if hydro.lwl > 0.0 {
        hydro.lwl
    } else {
        wl_max - wl_min
    };
    let x_lcg = hydro.lcb.x;

    let n_stations = spec.stations.max(5);
    let mut station_xs = Vec::with_capacity(n_stations);
    let mut station_x_models = Vec::with_capacity(n_stations);
    for i in 0..n_stations {
        let x_m = wl_min + (wl_max - wl_min) * (i as f64) / ((n_stations - 1) as f64);
        station_x_models.push(x_m);
        station_xs.push(x_m - x_lcg);
    }

    let mut warnings = Vec::new();
    let mut station_contours = Vec::with_capacity(n_stations);

    for &cut_x in &station_x_models {
        let clamped_x = cut_x.clamp(wl_min + 1e-4 * lwl, wl_max - 1e-4 * lwl);
        let (_, polys) = curves::station_polygons(&hull, clamped_x);
        let sec = extract_submerged_starboard(&polys, zwl);
        station_contours.push((cut_x, sec));
    }

    // Determine wave frequencies
    let mut omegas = spec.omegas.clone();
    if omegas.is_empty() {
        let l_min = spec.lambda_min.unwrap_or(0.3).max(0.01);
        let l_max = spec.lambda_max.unwrap_or(3.0).max(l_min);
        let pts = spec.points.unwrap_or(30).max(1);
        if pts <= 1 || (l_max - l_min).abs() < 1e-9 {
            let wavelength = l_min * lwl;
            if wavelength > 0.0 {
                let w = (2.0 * PI * spec.g / wavelength).sqrt();
                omegas.push(w);
            }
        } else {
            for i in 0..pts {
                let ratio = l_min + (l_max - l_min) * (i as f64) / ((pts - 1) as f64);
                let wavelength = ratio * lwl;
                if wavelength > 0.0 {
                    let w = (2.0 * PI * spec.g / wavelength).sqrt();
                    omegas.push(w);
                }
            }
        }
        omegas.sort_by(|a, b| a.total_cmp(b));
    }

    // Mass properties
    let mass = spec.rho * hydro.volume_m3;
    let kyy = match spec.kyy {
        Some(k) if k > 0.0 => k,
        _ => 0.25 * lwl,
    };
    let i55 = mass * kyy * kyy;

    // Hydrostatic stiffness, integrated over the same stations as the wave
    // excitation so that very long waves give heave → 1 and pitch → the
    // wave slope (up to the ρg∇(KB − KG) term, which the 2-DOF model with
    // surge held has no matching excitation for): C33 = ρg∫2b, C35 = C53 =
    // ρg∫2b·x, C55 = ρg∫2b·x² + ρg∇(KB − KG), x from the LCG.
    let bwl: Vec<f64> = station_contours
        .iter()
        .map(|(_, s)| 2.0 * s.half_beam)
        .collect();
    let moment = |p: i32| {
        let ys: Vec<f64> = station_xs
            .iter()
            .zip(&bwl)
            .map(|(x, b)| b * x.powi(p))
            .collect();
        spec.rho * spec.g * trapz(&station_xs, &ys)
    };
    let c33 = moment(0);
    let c55 = moment(2) + spec.rho * spec.g * hydro.volume_m3 * (hydro.kb - spec.kg);
    let c35 = moment(1);
    let c53 = c35;

    let mut raos = Vec::with_capacity(omegas.len());
    let mut global_coeffs = Vec::with_capacity(omegas.len());
    let mut aft_stations = Vec::with_capacity(omegas.len());

    for &w in &omegas {
        if w <= 0.0 {
            continue;
        }
        let k_wave = w * w / spec.g;
        let w_e = w + w * w * u_speed / spec.g;
        let lambda_m = 2.0 * PI * spec.g / (w * w);
        let lambda_over_l = lambda_m / lwl;

        let mut a33_secs = Vec::with_capacity(n_stations);
        let mut b33_secs = Vec::with_capacity(n_stations);
        let mut xa33_secs = Vec::with_capacity(n_stations);
        let mut xb33_secs = Vec::with_capacity(n_stations);
        let mut x2a33_secs = Vec::with_capacity(n_stations);
        let mut x2b33_secs = Vec::with_capacity(n_stations);
        let mut f3_secs = Vec::with_capacity(n_stations);
        let mut h3_secs = Vec::with_capacity(n_stations);
        // Depth attenuation κ_A = e^{−k T*_A} of the aftmost station (0 if it is closed).
        let mut kappa_aft = 0.0;

        for (i, (cut_x, sec)) in station_contours.iter().enumerate() {
            let x_rel = station_xs[i];
            let b = sec.half_beam;
            let t = sec.draft;
            let a_sec = sec.area;

            if a_sec < 1e-6 || b < 1e-4 || t < 1e-4 {
                a33_secs.push(0.0);
                b33_secs.push(0.0);
                xa33_secs.push(0.0);
                xb33_secs.push(0.0);
                x2a33_secs.push(0.0);
                x2b33_secs.push(0.0);
                f3_secs.push(C::ZERO);
                h3_secs.push(C::ZERO);
                continue;
            }

            let mut h_res = if sec.valid {
                heave_2d_section(&sec.contour, w_e, spec.rho, spec.g)
            } else {
                Heave2d {
                    added_mass: f64::NAN,
                    damping: f64::NAN,
                    amplitude_ratio: f64::NAN,
                }
            };

            if !h_res.added_mass.is_finite() || !h_res.damping.is_finite() {
                let sigma = a_sec / (2.0 * b * t);
                if let Ok(form) = lewis_form(b, t, sigma) {
                    h_res = heave_2d(&form, w_e, spec.rho, spec.g);
                    warnings.push(Warning {
                        code: "station_lewis_fallback",
                        message: format!(
                            "station at x = {:.3} m could not be converted to polygon contour; fell back to Lewis form",
                            cut_x
                        ),
                    });
                }
            }

            let a33_i = if h_res.added_mass.is_finite() {
                h_res.added_mass
            } else {
                0.0
            };
            let b33_i = if h_res.damping.is_finite() {
                h_res.damping
            } else {
                0.0
            };

            a33_secs.push(a33_i);
            b33_secs.push(b33_i);
            xa33_secs.push(x_rel * a33_i);
            xb33_secs.push(x_rel * b33_i);
            x2a33_secs.push(x_rel * x_rel * a33_i);
            x2b33_secs.push(x_rel * x_rel * b33_i);

            // Exciting force per section (Froude-Krylov + Diffraction with depth attenuation e^{-k T*})
            let t_star = a_sec / (2.0 * b);
            let kappa = (-k_wave * t_star).exp();
            if i == 0 {
                kappa_aft = kappa;
            }
            let phase = C::new((k_wave * x_rel).cos(), (k_wave * x_rel).sin());

            // Froude–Krylov ρg·2b plus diffraction h₃ = −ω(ω_e a₃₃ − i b₃₃) (STF).
            let diffraction = C::new(-w * w_e * a33_i, w * b33_i) * C::real(kappa) * phase;
            let froude_krylov = C::real(2.0 * spec.rho * spec.g * b * kappa) * phase;
            f3_secs.push(froude_krylov + diffraction);
            h3_secs.push(diffraction);
        }

        // Global integration
        let a33_0 = trapz(&station_xs, &a33_secs);
        let b33_0 = trapz(&station_xs, &b33_secs);
        let a35_0 = trapz(&station_xs, &xa33_secs);
        let b35_0 = trapz(&station_xs, &xb33_secs);
        let a55_0 = trapz(&station_xs, &x2a33_secs);
        let b55_0 = trapz(&station_xs, &x2b33_secs);

        let mut a33 = a33_0;
        let mut b33 = b33_0;
        // STF speed terms for pitch positive bow up (x forward): derived from the
        // relative-motion force −D/Dt[(a + b/iω_e)·V], D/Dt = ∂t − U∂x, by parts.
        let mut a35 = a35_0 + (u_speed / (w_e * w_e)) * b33_0;
        let mut b35 = b35_0 - u_speed * a33_0;
        let mut a53 = a35_0 - (u_speed / (w_e * w_e)) * b33_0;
        let mut b53 = b35_0 + u_speed * a33_0;
        let mut a55 = a55_0 + (u_speed * u_speed / (w_e * w_e)) * a33_0;
        let mut b55 = b55_0 + (u_speed * u_speed / (w_e * w_e)) * b33_0;

        // STF transom terms: the boundary terms of the same integration by parts
        // at the aft end x_A (zero for a closed stern, a_A = b_A = 0).
        let x_a = station_xs[0];
        let (a_a, b_a) = (a33_secs[0], b33_secs[0]);
        aft_stations.push(AftStation {
            omega: w,
            omega_e: w_e,
            x_a,
            a_a,
            b_a,
        });
        let mut transom_f3 = C::ZERO;
        let mut transom_f5 = C::ZERO;
        if spec.transom_terms {
            let (u, we2) = (u_speed, w_e * w_e);
            a33 += -u * b_a / we2;
            b33 += u * a_a;
            a35 += (-u * x_a * b_a + u * u * a_a) / we2;
            b35 += u * x_a * a_a + u * u * b_a / we2;
            a53 += -u * x_a * b_a / we2;
            b53 += u * x_a * a_a;
            a55 += (-u * x_a * x_a * b_a + u * u * x_a * a_a) / we2;
            b55 += u * x_a * x_a * a_a + u * u * x_a * b_a / we2;
            // ΔF3 = U m_A v_A, ΔF5 = x_A ΔF3, m_A = a_A − i b_A/ω_e, v_A = iω κ_A e^{ik x_A}.
            let m_a = C::new(a_a, -b_a / w_e);
            let v_a =
                C::new(0.0, w * kappa_aft) * C::new((k_wave * x_a).cos(), (k_wave * x_a).sin());
            transom_f3 = m_a * v_a * u;
            transom_f5 = transom_f3 * x_a;
        }

        global_coeffs.push(GlobalCoefficients {
            omega: w,
            omega_e: w_e,
            a33,
            b33,
            a35,
            b35,
            a53,
            b53,
            a55,
            b55,
            c33,
            c35,
            c53,
            c55,
        });

        // Wave excitation forces
        let f3_total = trapz_complex(&station_xs, &f3_secs);
        let mut x_f3_secs = Vec::with_capacity(n_stations);
        for (i, &x_rel) in station_xs.iter().enumerate() {
            x_f3_secs.push(f3_secs[i] * x_rel);
        }
        let static_m5 = trapz_complex(&station_xs, &x_f3_secs);
        // STF: the speed term U/(iω_e) applies to the diffraction part only.
        let h3_total = trapz_complex(&station_xs, &h3_secs);
        let f5_total = static_m5 + C::new(0.0, -u_speed / w_e) * h3_total + transom_f5;
        let f3_total = f3_total + transom_f3;

        // 2x2 complex motion equations
        let k11 = C::new(c33 - w_e * w_e * (mass + a33), w_e * b33);
        let k12 = C::new(c35 - w_e * w_e * a35, w_e * b35);
        let k21 = C::new(c53 - w_e * w_e * a53, w_e * b53);
        let k22 = C::new(c55 - w_e * w_e * (i55 + a55), w_e * b55);

        let det = k11 * k22 - k12 * k21;
        let (eta3, eta5) = if det.abs() > 1e-30 {
            let e3 = (k22 * f3_total - k12 * f5_total) / det;
            let e5 = (-k21 * f3_total + k11 * f5_total) / det;
            (e3, e5)
        } else {
            (C::ZERO, C::ZERO)
        };

        let heave_amp = eta3.abs();
        let heave_phase_deg = eta3.im.atan2(eta3.re) * 180.0 / PI;
        let theta_amp = eta5.abs();
        let pitch_amp = if k_wave > 1e-12 {
            theta_amp / k_wave
        } else {
            0.0
        };
        let pitch_phase_deg = eta5.im.atan2(eta5.re) * 180.0 / PI;

        raos.push(RaoPoint {
            omega: w,
            omega_e: w_e,
            wavelength_m: lambda_m,
            lambda_over_l,
            heave_amp,
            heave_phase_deg,
            pitch_amp,
            pitch_phase_deg,
        });
    }

    let natural_heave_omega = find_natural_frequency(&global_coeffs, mass, c33, true);
    let natural_pitch_omega = find_natural_frequency(&global_coeffs, i55, c55, false);

    Ok((
        SeakeepingReport {
            frequencies: raos,
            natural_heave_omega,
            natural_pitch_omega,
            coefficients: Some(global_coeffs),
            warnings,
            x_lcg,
            x_bow: wl_max,
        },
        aft_stations,
    ))
}

fn find_natural_frequency(
    coeffs: &[GlobalCoefficients],
    inertia: f64,
    stiffness: f64,
    is_heave: bool,
) -> f64 {
    if coeffs.is_empty() {
        return 0.0;
    }
    for i in 0..coeffs.len().saturating_sub(1) {
        let w1 = coeffs[i].omega;
        let a1 = if is_heave {
            coeffs[i].a33
        } else {
            coeffs[i].a55
        };
        let g1 = stiffness - w1 * w1 * (inertia + a1);

        let w2 = coeffs[i + 1].omega;
        let a2 = if is_heave {
            coeffs[i + 1].a33
        } else {
            coeffs[i + 1].a55
        };
        let g2 = stiffness - w2 * w2 * (inertia + a2);

        if (g1 >= 0.0 && g2 <= 0.0) || (g1 <= 0.0 && g2 >= 0.0) {
            let denom = g1 - g2;
            if denom.abs() > 1e-15 {
                let frac = g1 / denom;
                return w1 + frac * (w2 - w1);
            }
        }
    }
    let mid = coeffs.len() / 2;
    let a_mid = if is_heave {
        coeffs[mid].a33
    } else {
        coeffs[mid].a55
    };
    let total_m = inertia + a_mid;
    if total_m > 0.0 && stiffness > 0.0 {
        (stiffness / total_m).sqrt()
    } else {
        0.0
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

    fn test_box_barge(l: f64, b: f64, depth: f64) -> Surface {
        let mut s = Surface::new();
        s.layers = vec![crate::mesh::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Default::default()
        }];
        s.settings = crate::mesh::ProjectSettings {
            name: "box_barge".into(),
            length: l,
            beam: b,
            draft: depth,
            water_density: 1025.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..Default::default()
        };
        let corners = [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, depth),
            (l, 0.0, depth),
            (l, b, depth),
            (0.0, b, depth),
        ];
        for (x, y, z) in corners {
            s.add_point(crate::math::Vec3::new(x, y - b * 0.5, z));
        }
        for face in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            s.add_face(face, 0);
        }
        s.initialize_trace();
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = crate::mesh::VertexKind::Corner;
        }
        s
    }

    /// Test 7.a: long waves (λ/L = 20, U = 0): heave_amp -> 1 within 3 %, pitch_amp -> 1 within 5 %
    #[test]
    fn test_raos_long_waves_approach_unity() {
        let (l, b, depth, t) = (10.0, 2.0, 2.0, 0.5);
        let surface = test_box_barge(l, b, depth);
        let spec = SeakeepingSpec {
            draft: t,
            speed_kn: 0.0,
            kg: 0.25,
            stations: 21,
            lambda_min: Some(20.0),
            lambda_max: Some(20.0),
            points: Some(1),
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        assert_eq!(rep.frequencies.len(), 1);
        let pt = &rep.frequencies[0];
        assert!(
            (pt.heave_amp - 1.0).abs() < 0.03,
            "long wave heave amp must be within 3% of 1.0, got {:.4}",
            pt.heave_amp
        );
        assert!(
            (pt.pitch_amp - 1.0).abs() < 0.05,
            "long wave pitch amp must be within 5% of 1.0, got {:.4}",
            pt.pitch_amp
        );
    }

    /// Test 7.b: short waves (λ/L = 0.2): heave_amp and pitch_amp < 0.1
    #[test]
    fn test_raos_short_waves_decay() {
        let surface = crate::demo::demo_hull(true);
        let spec = SeakeepingSpec {
            draft: 0.62,
            speed_kn: 0.0,
            kg: 2.2,
            stations: 21,
            lambda_min: Some(0.2),
            lambda_max: Some(0.2),
            points: Some(1),
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        assert_eq!(rep.frequencies.len(), 1);
        let pt = &rep.frequencies[0];
        assert!(
            pt.heave_amp < 0.1,
            "short wave heave amp must be < 0.1, got {:.4}",
            pt.heave_amp
        );
        assert!(
            pt.pitch_amp < 0.1,
            "short wave pitch amp must be < 0.1, got {:.4}",
            pt.pitch_amp
        );
    }

    /// Test 7.c: box barge (L = 10, B = 2, T = 0.5):
    /// C33 = ρg·L·B within 1 %, C55 = ρg·B·L³/12 (+ the ρg∇(KB−KG) term) within 2 %
    ///
    /// Mathematical derivation:
    /// 1. Submerged volume \nabla = L * B * T.
    /// 2. Waterplane area Awp = L * B, so C33 = \rho * g * Awp = \rho * g * L * B.
    /// 3. Waterplane second moment of area I_L = B * L^3 / 12.
    /// 4. Metacentric radius BM_L = I_L / \nabla = (B * L^3 / 12) / \nabla.
    /// 5. Vertical center of buoyancy KB = T / 2.
    /// 6. Longitudinal metacentric height GM_L = KB + BM_L - KG = (B * L^3 / 12) / \nabla + (KB - KG).
    /// 7. Restoring stiffness C55 = \rho * g * \nabla * GM_L
    ///    = \rho * g * (B * L^3 / 12) + \rho * g * \nabla * (KB - KG).
    #[test]
    fn test_box_barge_hydrostatic_stiffness() {
        let (l, b, depth, t) = (10.0, 2.0, 2.0, 0.5);
        let surface = test_box_barge(l, b, depth);
        let rho = 1025.0;
        let g = 9.81;
        let kg = 0.4;

        let spec = SeakeepingSpec {
            draft: t,
            speed_kn: 0.0,
            kg,
            stations: 21,
            rho,
            g,
            points: Some(2),
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        let coeffs = rep.coefficients.expect("coefficients must be present");
        let c = &coeffs[0];

        let expected_c33 = rho * g * l * b;
        let c33_err = (c.c33 - expected_c33).abs() / expected_c33;
        assert!(
            c33_err < 0.01,
            "C33 error must be < 1%, got {:.4} (got {}, expected {})",
            c33_err,
            c.c33,
            expected_c33
        );

        let nabla = l * b * t;
        let kb = t * 0.5;
        let expected_c55 = rho * g * (b * l.powi(3) / 12.0) + rho * g * nabla * (kb - kg);
        let c55_err = (c.c55 - expected_c55).abs() / expected_c55;
        assert!(
            c55_err < 0.02,
            "C55 error must be < 2%, got {:.4} (got {}, expected {})",
            c55_err,
            c.c55,
            expected_c55
        );
    }

    /// Test 7.d: fore–aft symmetric box at U = 0 with LCG at midship:
    /// |A35|, |B35|, |C35| < 1e-3 of corresponding sqrt(A33*A55), sqrt(B33*B55), sqrt(C33*C55)
    #[test]
    fn test_symmetric_box_cross_coupling_vanishes() {
        let (l, b, depth, t) = (10.0, 2.0, 2.0, 0.5);
        let surface = test_box_barge(l, b, depth);
        let spec = SeakeepingSpec {
            draft: t,
            speed_kn: 0.0,
            kg: 0.25,
            stations: 21,
            points: Some(3),
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        let coeffs = rep.coefficients.expect("coefficients must be present");
        for c in coeffs {
            let ref_a = (c.a33 * c.a55).sqrt();
            let ref_b = (c.b33 * c.b55).sqrt();
            let ref_c = (c.c33 * c.c55).sqrt();

            assert!(
                c.a35.abs() < 1e-3 * ref_a,
                "|A35| must be < 1e-3 * sqrt(A33*A55), got {} vs 1e-3*{}",
                c.a35.abs(),
                ref_a
            );
            assert!(
                c.b35.abs() < 1e-3 * ref_b,
                "|B35| must be < 1e-3 * sqrt(B33*B55), got {} vs 1e-3*{}",
                c.b35.abs(),
                ref_b
            );
            assert!(
                c.c35.abs() < 1e-3 * ref_c,
                "|C35| must be < 1e-3 * sqrt(C33*C55), got {} vs 1e-3*{}",
                c.c35.abs(),
                ref_c
            );
        }
    }

    /// In very long waves the hull follows the surface whatever its speed. This
    /// caught swapped speed terms in A35/A53, B35/B53 (pitch 9.9 at 6 kn instead
    /// of 1.1). With KG = KB pitch must tend to the wave slope exactly; with G
    /// above B it tends to ∫2b·x² / (∫2b·x² + ∇(KB − KG)), the 2-DOF model's
    /// own limit (surge held, so no excitation matches the ∇(KB − KG) term).
    #[test]
    fn very_long_waves_are_followed_at_any_speed() {
        let surface = crate::demo::demo_hull(true);
        let run = |kg: f64, speed_kn: f64| {
            let spec = SeakeepingSpec {
                draft: 0.62,
                speed_kn,
                kg,
                lambda_min: Some(100.0),
                lambda_max: Some(100.0),
                points: Some(1),
                ..Default::default()
            };
            let rep = head_sea_raos(&surface, &spec).expect("raos");
            (rep.frequencies[0].heave_amp, rep.frequencies[0].pitch_amp)
        };
        let kb = 0.392; // demo hull at T = 0.62 m (AGENTS.md reference)
        for speed in [0.0, 3.0, 6.0] {
            let (heave, pitch) = run(kb, speed);
            assert!((heave - 1.0).abs() < 0.01, "{speed} kn: heave {heave}");
            assert!((pitch - 1.0).abs() < 0.02, "{speed} kn: pitch {pitch}");
        }
        let (_, still) = run(2.2, 0.0);
        let (_, moving) = run(2.2, 6.0);
        assert!(
            (still - 1.10).abs() < 0.02,
            "KG 2.2 m: {still} (BM_L/GM_L ≈ 1.10)"
        );
        assert!((moving - still).abs() < 0.03 * still, "{moving} vs {still}");
    }

    /// STF speed structure for pitch positive bow up:
    /// A35 − A53 = 2U/ωe²·B33 and B35 − B53 = −2U·A33 to 1e-9 relative
    #[test]
    fn test_stf_speed_structure() {
        let surface = crate::demo::demo_hull(true);
        let speed_kn = 6.0;
        let u_speed = speed_kn * 0.5144444444444445;
        let spec = SeakeepingSpec {
            draft: 0.62,
            speed_kn,
            kg: 2.2,
            stations: 21,
            points: Some(5),
            // The speed structure below is that of the strip integrals; the
            // transom terms have their own structure (see the transom tests).
            transom_terms: false,
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        let coeffs = rep.coefficients.expect("coefficients must be present");
        for c in coeffs {
            let w_e = c.omega_e;
            let diff_a = c.a35 - c.a53;
            let expected_diff_a = 2.0 * u_speed / (w_e * w_e) * c.b33;
            let scale_a = c.a35.abs().max(c.a53.abs()).max(1.0);
            let rel_err_a = (diff_a - expected_diff_a).abs() / scale_a;
            assert!(
                rel_err_a < 1e-9,
                "A35 - A53 error must be < 1e-9 relative, got {}",
                rel_err_a
            );

            let diff_b = c.b35 - c.b53;
            let expected_diff_b = -2.0 * u_speed * c.a33;
            let scale_b = c.b35.abs().max(c.b53.abs()).max(1.0);
            let rel_err_b = (diff_b - expected_diff_b).abs() / scale_b;
            assert!(
                rel_err_b < 1e-9,
                "B35 - B53 error must be < 1e-9 relative, got {}",
                rel_err_b
            );
        }
    }

    /// Test 7.f: heave RAO peak (U = 0, demo hull, fine ω grid) lies within 5 % of natural_heave_omega
    #[test]
    fn test_heave_rao_peak_near_natural_frequency() {
        let surface = crate::demo::demo_hull(true);
        // Fine omega grid from 1.5 to 3.5 rad/s with 60 points
        // Fine omega grid around resonance (2.5 to 3.2 rad/s)
        let omegas: Vec<f64> = (0..50)
            .map(|i| 2.5 + (3.2 - 2.5) * (i as f64) / 49.0)
            .collect();
        let spec = SeakeepingSpec {
            draft: 0.62,
            speed_kn: 0.0,
            kg: 2.2,
            stations: 21,
            omegas,
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        let mut max_amp = 0.0;
        let mut peak_omega = 0.0;
        for pt in &rep.frequencies {
            if pt.heave_amp > max_amp {
                max_amp = pt.heave_amp;
                peak_omega = pt.omega;
            }
        }
        let nat = rep.natural_heave_omega;
        assert!(nat > 0.0, "natural heave omega must be positive");
        let err = (peak_omega - nat).abs() / nat;
        assert!(
            err < 0.05,
            "peak heave frequency {:.3} must be within 5% of natural_heave_omega {:.3} (got {:.2}%)",
            peak_omega, nat, err * 100.0
        );
    }

    /// E3 oracle (docs/oracles/heave_2d_lid.py, §18): semicircle R = 1 with 60 hull
    /// panels against Ursell's multipoles. Columns: ξ = KR, a/(ρπR²/2),
    /// b/(ρπR²/2·ω). The plain method is off by 60 % at the irregular
    /// frequency ξ ≈ 1.82; the lid removes it.
    #[test]
    fn lid_removes_the_irregular_frequencies_of_the_semicircle() {
        let (rho, g) = (1025.0, 9.81);
        let ursell: [(f64, f64, f64); 8] = [
            (0.50, 0.6479, 0.8140),
            (1.00, 0.6084, 0.3973),
            (1.50, 0.6687, 0.2120),
            (1.82, 0.7096, 0.1470),
            (2.50, 0.7801, 0.0729),
            (3.00, 0.8178, 0.0461),
            (4.00, 0.8684, 0.0206),
            (4.78, 0.8937, 0.0120),
        ];
        let form = lewis_form(1.0, 1.0, PI / 4.0).unwrap();
        let contour = lewis_contour(&form, 61);
        let norm = 0.5 * rho * PI;
        for (xi, am, dm) in ursell {
            let omega = (xi * g).sqrt();
            let lid = heave_2d_section(&contour, omega, rho, g);
            let (a, b) = (lid.added_mass / norm, lid.damping / (norm * omega));
            println!("ξ={xi:4.2}: lid a={a:.4} b={b:.4}");
            assert!(
                (a - am).abs() <= 0.02 * am,
                "ξ={xi}: lid a {a} vs Ursell {am}"
            );
            assert!(
                (b - dm).abs() <= (0.03 * dm).max(0.002),
                "ξ={xi}: lid b {b} vs Ursell {dm}"
            );
        }
        let omega = (1.82_f64 * g).sqrt();
        let plain = heave_2d_section_with(&contour, omega, rho, g, false);
        let a = plain.added_mass / norm;
        assert!(
            (a - 0.7096).abs() > 0.3 * 0.7096,
            "plain Frank at ξ = 1.82 must show the irregular frequency, got a = {a}"
        );
        // The lid is a no-op in the infinite-frequency limit (K = ∞).
        let inf = heave_2d_section(&contour, f64::INFINITY, rho, g);
        let inf_plain = heave_2d_section_with(&contour, f64::INFINITY, rho, g, false);
        assert_eq!(inf.added_mass, inf_plain.added_mass);
    }

    /// An octahedron hull (pointed at bow and stern): the aftmost station is closed.
    fn test_diamond(l: f64, b: f64, d: f64) -> Surface {
        let mut s = test_box_barge(l, b, d);
        s.points.clear();
        s.edges.clear();
        s.faces.clear();
        s.face_layer.clear();
        let pts = [
            (0.0, 0.0, 0.5 * d),
            (l, 0.0, 0.5 * d),
            (0.5 * l, 0.5 * b, 0.5 * d),
            (0.5 * l, -0.5 * b, 0.5 * d),
            (0.5 * l, 0.0, 0.0),
            (0.5 * l, 0.0, d),
        ];
        for (x, y, z) in pts {
            s.add_point(crate::math::Vec3::new(x, y, z));
        }
        // 0 stern, 1 bow, 2 starboard, 3 port, 4 keel, 5 top; outward winding.
        for face in [
            vec![1, 2, 5],
            vec![1, 4, 2],
            vec![1, 5, 3],
            vec![1, 3, 4],
            vec![0, 5, 2],
            vec![0, 2, 4],
            vec![0, 3, 5],
            vec![0, 4, 3],
        ] {
            s.add_face(face, 0);
        }
        s.initialize_trace();
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = crate::mesh::VertexKind::Corner;
        }
        s
    }

    /// With a closed stern (a_A = b_A = 0) the transom terms vanish identically.
    #[test]
    fn transom_terms_vanish_for_a_closed_stern() {
        let surface = test_diamond(10.0, 2.0, 2.0);
        let run = |transom_terms: bool| {
            let spec = SeakeepingSpec {
                draft: 0.8,
                speed_kn: 6.0,
                kg: 0.6,
                omegas: vec![1.0, 1.6, 2.4, 3.2],
                transom_terms,
                ..Default::default()
            };
            head_sea_raos_detailed(&surface, &spec).expect("raos")
        };
        let (on, aft) = run(true);
        let (off, _) = run(false);
        for s in &aft {
            assert_eq!(s.a_a, 0.0);
            assert_eq!(s.b_a, 0.0);
        }
        for (p, q) in on.frequencies.iter().zip(&off.frequencies) {
            assert_eq!(p.heave_amp, q.heave_amp);
            assert_eq!(p.pitch_amp, q.pitch_amp);
            assert_eq!(p.heave_phase_deg, q.heave_phase_deg);
            assert_eq!(p.pitch_phase_deg, q.pitch_phase_deg);
            assert!(p.heave_amp.is_finite() && p.heave_amp > 0.0);
        }
    }

    /// For the demo hull (wide transom) at 6 kn the coefficients change by
    /// exactly the STF transom formulas built from the aft station's a_A, b_A.
    #[test]
    fn transom_coefficient_changes_match_the_formulas() {
        let surface = crate::demo::demo_hull(true);
        let u = 6.0 * 0.5144444444444445;
        let run = |transom_terms: bool| {
            let spec = SeakeepingSpec {
                draft: 0.62,
                speed_kn: 6.0,
                kg: 2.2,
                omegas: vec![2.0, 3.0, 4.5],
                transom_terms,
                ..Default::default()
            };
            head_sea_raos_detailed(&surface, &spec).expect("raos")
        };
        let (on, aft) = run(true);
        let (off, aft_off) = run(false);
        assert_eq!(aft, aft_off);
        let (c_on, c_off) = (on.coefficients.unwrap(), off.coefficients.unwrap());
        for ((on, off), s) in c_on.iter().zip(&c_off).zip(&aft) {
            let (x, a, b, we2) = (s.x_a, s.a_a, s.b_a, s.omega_e * s.omega_e);
            assert!(
                a > 0.0 && b > 0.0,
                "the demo hull has a transom: a_A {a}, b_A {b}"
            );
            let checks = [
                ("A33", on.a33 - off.a33, -u * b / we2),
                ("B33", on.b33 - off.b33, u * a),
                ("A35", on.a35 - off.a35, (-u * x * b + u * u * a) / we2),
                ("B35", on.b35 - off.b35, u * x * a + u * u * b / we2),
                ("A53", on.a53 - off.a53, -u * x * b / we2),
                ("B53", on.b53 - off.b53, u * x * a),
                (
                    "A55",
                    on.a55 - off.a55,
                    (-u * x * x * b + u * u * x * a) / we2,
                ),
                ("B55", on.b55 - off.b55, u * x * x * a + u * u * x * b / we2),
            ];
            for (name, got, want) in checks {
                assert!(
                    (got - want).abs() <= 1e-9 * want.abs().max(1.0),
                    "ω={}: Δ{name} {got} vs {want}",
                    s.omega
                );
            }
        }
        // The terms act on the motions.
        let (p_on, p_off) = (&on.frequencies[1], &off.frequencies[1]);
        assert!((p_on.heave_amp - p_off.heave_amp).abs() > 1e-6);
    }

    /// The transom terms are on by default and keep the long-wave limit
    /// (radiation and excitation terms cancel) at 0, 3 and 6 kn.
    #[test]
    fn long_waves_are_followed_with_transom_terms_at_any_speed() {
        assert!(SeakeepingSpec::default().transom_terms);
        let surface = crate::demo::demo_hull(true);
        for speed_kn in [0.0, 3.0, 6.0] {
            let spec = SeakeepingSpec {
                draft: 0.62,
                speed_kn,
                kg: 0.392,
                lambda_min: Some(100.0),
                lambda_max: Some(100.0),
                points: Some(1),
                transom_terms: true,
                ..Default::default()
            };
            let rep = head_sea_raos(&surface, &spec).expect("raos");
            let p = &rep.frequencies[0];
            assert!(
                (p.heave_amp - 1.0).abs() < 0.01,
                "{speed_kn} kn: heave {}",
                p.heave_amp
            );
            assert!(
                (p.pitch_amp - 1.0).abs() < 0.02,
                "{speed_kn} kn: pitch {}",
                p.pitch_amp
            );
        }
    }

    /// Demo hull at 6 kn: the effect of the transom terms (printed for the E3 report).
    #[test]
    fn transom_effect_on_the_demo_hull_rao_is_moderate() {
        let surface = crate::demo::demo_hull(true);
        for ratio in [1.0, 1.5, 2.0] {
            let run = |transom_terms: bool| {
                let spec = SeakeepingSpec {
                    draft: 0.62,
                    speed_kn: 6.0,
                    kg: 2.2,
                    lambda_min: Some(ratio),
                    lambda_max: Some(ratio),
                    points: Some(1),
                    transom_terms,
                    ..Default::default()
                };
                let rep = head_sea_raos(&surface, &spec).expect("raos");
                (rep.frequencies[0].heave_amp, rep.frequencies[0].pitch_amp)
            };
            let (h1, p1) = run(true);
            let (h0, p0) = run(false);
            println!("λ/L={ratio}: heave on {h1:.4} off {h0:.4}; pitch on {p1:.4} off {p0:.4}");
            assert!(h1.is_finite() && p1.is_finite());
            assert!((h1 - h0).abs() < 0.5 * h0.max(0.2), "heave {h1} vs {h0}");
            assert!((p1 - p0).abs() < 0.5 * p0.max(0.2), "pitch {p1} vs {p0}");
        }
    }

    /// Test 7.g: demo hull at 6 kn gives finite RAOs at all points, no panics
    #[test]
    fn test_demo_hull_at_6kn_finite_raos() {
        let surface = crate::demo::demo_hull(true);
        let spec = SeakeepingSpec {
            draft: 0.62,
            speed_kn: 6.0,
            kg: 2.2,
            stations: 21,
            points: Some(20),
            ..Default::default()
        };
        let rep = head_sea_raos(&surface, &spec).expect("head_sea_raos failed");
        assert_eq!(rep.frequencies.len(), 20);
        for pt in &rep.frequencies {
            assert!(pt.omega.is_finite());
            assert!(pt.omega_e.is_finite());
            assert!(pt.wavelength_m.is_finite());
            assert!(pt.lambda_over_l.is_finite());
            assert!(pt.heave_amp.is_finite(), "heave amp must be finite");
            assert!(pt.heave_phase_deg.is_finite(), "heave phase must be finite");
            assert!(pt.pitch_amp.is_finite(), "pitch amp must be finite");
            assert!(pt.pitch_phase_deg.is_finite(), "pitch phase must be finite");
        }
    }
}
