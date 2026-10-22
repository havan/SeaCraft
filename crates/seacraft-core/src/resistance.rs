//! Still-water resistance and effective power: Holtrop & Mennen (1984).
//!
//! `An approximate power prediction method`, International Shipbuilding
//! Progress 29(335), for a displacement monohull with the usual
//! `RT = RF(1+k1) + RAPP + RW + RB + RTR + RA` structure. Effective power is
//! `PE = RT · V` — no propeller, no wake fraction, no thrust deduction, so this
//! is the power the hull needs, not the power the engine delivers.
//!
//! Everything that describes the hull form is measured on the **refined mesh**
//! by [`crate::hydro`]; the method's own inputs (`iE`, `AT`, `ABT`, `LCB` as a
//! percentage of the length) are derived from those measurements here. Nothing
//! in this module looks at the control net.
//!
//! Limits worth stating in the answer rather than hiding: Holtrop–Mennen was
//! fitted to `0.55 ≤ Cp ≤ 0.85`, `3.9 ≤ L/B ≤ 14.9`, `2.1 ≤ B/T ≤ 4.0` and
//! `Fn ≤ 0.55`. [`HoltropInput::warnings`] reports the ones a given hull
//! violates instead of silently extrapolating.
//!
//! Coefficients are transcribed from the paper's equations (1)–(39); the tests
//! pin the pieces that can be checked exactly (the ITTC-57 line, the piecewise
//! coefficient switches, Froude scaling of the wave term, linearity in density)
//! rather than a golden total, which would only pin one hull.

use crate::hydro::{self, Hull, HydroOptions};
use crate::subdiv::SubdivisionOptions;
use serde::Serialize;

/// Gravity used by the method, m/s².
pub const GRAVITY: f64 = 9.80665;
/// Kinematic viscosity of sea water at 15 °C, m²/s.
pub const VISCOSITY_SEA_WATER: f64 = 1.1883e-6;
/// Kinematic viscosity of fresh water at 15 °C, m²/s.
pub const VISCOSITY_FRESH_WATER: f64 = 1.1386e-6;
/// Knots in one metre per second.
pub const KNOT: f64 = 0.514444;

/// The hull-form numbers Holtrop–Mennen needs.
#[derive(Debug, Clone, Serialize)]
pub struct HoltropInput {
    pub length_wl: f64,
    pub beam_wl: f64,
    pub draft: f64,
    pub volume_m3: f64,
    pub cb: f64,
    pub cp: f64,
    pub cm: f64,
    pub cwp: f64,
    /// Longitudinal centre of buoyancy as a percentage of the length, positive
    /// forward of midships.
    pub lcb_pct: f64,
    pub wetted_surface_m2: f64,
    /// Half-angle of entrance of the design waterline, degrees.
    pub half_entrance_deg: f64,
    /// Immersed transom area, m².
    pub transom_area_m2: f64,
    /// Immersed area of a bulbous bow, m² (0 when there is no bulb).
    pub bulb_area_m2: f64,
    /// Height of the bulb's centre above the keel, m.
    pub bulb_centre_z: f64,
    /// Holtrop's stern parameter `c_stern`, -10 (V-shaped) … +10 (U-shaped with
    /// a gondola); 0 is a normal stern.
    pub c_stern: f64,
    /// Wetted area of appendages, m², and their form factor `1+k2`.
    pub appendage_area_m2: f64,
    pub appendage_factor: f64,
    pub water_density: f64,
    pub viscosity: f64,
}

impl HoltropInput {
    /// Measure the input numbers from a hull at its design condition.
    ///
    /// `iE` is the half-angle of entrance of the design waterline: the tangent
    /// at the forward end of the DWL. A mesh only gives the waterline as a
    /// polyline, so it is measured as the chord over the first 2 % of the
    /// waterline length — the convention the method was fitted to draws the
    /// tangent at the stem, and on a fine bow the two agree closely. A very
    /// bluff waterline (a pram bow) makes the chord steeper than the tangent and
    /// therefore gives a slightly larger `iE`, i.e. a slightly larger `c1`.
    pub fn measure(hull: &Hull, opts: &HydroOptions) -> Option<Self> {
        let mut o = opts.clone();
        o.stations = 0; // the resistance model needs the integrals, not the curve
        if o.draft <= 0.0 {
            return None;
        }
        let h = hull.hydrostatics(&o);
        if h.volume_m3 <= 0.0 || h.lwl <= 0.0 || h.bwl <= 0.0 {
            return None;
        }
        let wp = hydro::waterplane_for(hull.keel, hull.length, o.draft, o.trim, o.heel_deg);
        // The submerged extent along x, taken from the waterline cut rather than
        // from the vertices: a vertex just below the plane can sit up to one
        // triangle short of the stem, which under-reads the length by about 1 %
        // and drags every percentage of L with it. The cut's crossing points are
        // exactly on the surface.
        let segs = hydro::plane_segments(&hull.verts, &hull.tris, &wp);
        let (mut x_min, mut x_max) = (f64::INFINITY, f64::NEG_INFINITY);
        for (a, b) in &segs {
            x_min = x_min.min(a.x).min(b.x);
            x_max = x_max.max(a.x).max(b.x);
        }
        if !(x_min.is_finite() && x_max.is_finite()) || x_max <= x_min {
            return None;
        }
        let lwl = x_max - x_min;
        let lcb_pct = (h.lcb.x - 0.5 * (x_min + x_max)) / lwl * 100.0;
        let half_entrance_deg = half_entrance_deg(hull, &wp, x_max, lwl);
        // Immersed transom: the section just inside the aft end. Cutting exactly
        // on the transom face is a degenerate plane, hence the 0.1 % inset.
        let transom_area_m2 = if h.lcb.x.is_finite() {
            hydro::section_area(&hull.verts, &hull.tris, x_min + 0.001 * lwl, &wp)
        } else {
            0.0
        };
        Some(Self {
            length_wl: lwl,
            beam_wl: h.bwl,
            draft: o.draft,
            volume_m3: h.volume_m3,
            cb: h.block_coefficient,
            cp: h.prismatic_coefficient,
            cm: h.midship_coefficient,
            cwp: h.waterplane_coefficient,
            lcb_pct,
            wetted_surface_m2: h.wetted_surface_m2,
            half_entrance_deg,
            transom_area_m2,
            bulb_area_m2: 0.0,
            bulb_centre_z: 0.0,
            c_stern: 0.0,
            appendage_area_m2: 0.0,
            appendage_factor: 1.5,
            water_density: o.water_density,
            viscosity: if (o.water_density - 1000.0).abs() < 25.0 {
                VISCOSITY_FRESH_WATER
            } else {
                VISCOSITY_SEA_WATER
            },
        })
    }

    /// The empirical validity ranges this hull falls outside, as short strings.
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let lb = self.length_wl / self.beam_wl;
        let bt = self.beam_wl / self.draft;
        if !(0.55..=0.85).contains(&self.cp) {
            out.push(format!("Cp {:.3} is outside 0.55-0.85", self.cp));
        }
        if !(3.9..=14.9).contains(&lb) {
            out.push(format!("L/B {lb:.2} is outside 3.9-14.9"));
        }
        if !(2.1..=4.0).contains(&bt) {
            out.push(format!("B/T {bt:.2} is outside 2.1-4.0"));
        }
        out
    }
}

/// Half-angle of entrance of the waterline at `x_bow`, in degrees.
///
/// The method defines `iE` as the tangent to the design waterline at its fore
/// end. A mesh only gives the waterline as a polyline, so the angle is taken as
/// the chord from the bow to the starboard waterline point closest to 2 % of
/// the waterline length aft of it. On a fine bow the chord and the tangent
/// agree closely; a pram bow makes the chord steeper and therefore reports a
/// larger `iE` (a larger `c1`, i.e. more wave resistance), which is the safe
/// direction for an estimate.
fn half_entrance_deg(
    hull: &Hull,
    wp: &crate::math::Plane,
    x_bow: f64,
    lwl: f64,
) -> f64 {
    let span = (0.02 * lwl).max(1e-6);
    let segs = hydro::plane_segments(&hull.verts, &hull.tris, wp);
    let mut best: Option<(f64, f64)> = None; // (chord dx, y)
    for (poly, _) in hydro::chain_paths(&segs, 1e-7) {
        for p in &poly {
            if p.y < 0.0 {
                continue; // starboard side only, y >= 0
            }
            let dx = x_bow - p.x;
            if dx <= 1e-9 || dx > span {
                continue;
            }
            if best.is_none_or(|(d, _)| (dx - span).abs() < (d - span).abs()) {
                best = Some((dx, p.y));
            }
        }
    }
    match best {
        Some((dx, y)) if dx > 1e-9 => (y / dx).atan().to_degrees(),
        _ => 0.0,
    }
}

/// One speed's resistance breakdown, all forces in newtons.
#[derive(Debug, Clone, Serialize)]
pub struct Resistance {
    pub speed_kn: f64,
    pub speed_m_s: f64,
    /// Froude number on the waterline length.
    pub fn_: f64,
    pub reynolds: f64,
    /// ITTC-57 friction coefficient.
    pub cf: f64,
    /// `1 + k1`: the form factor of the hull.
    pub form_factor: f64,
    pub rf_n: f64,
    pub rapp_n: f64,
    pub rw_n: f64,
    pub rb_n: f64,
    pub rtr_n: f64,
    pub ra_n: f64,
    /// Total still-water resistance.
    pub rt_n: f64,
    /// Effective power, `RT · V`, in watts.
    pub pe_w: f64,
}

/// Wave resistance coefficient pieces, exposed because they are the part most
/// worth checking against the paper.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct WaveCoefficients {
    pub c1: f64,
    pub c2: f64,
    pub c5: f64,
    pub c7: f64,
    pub lambda: f64,
    pub m1: f64,
    pub m2: f64,
    pub c16: f64,
}

fn c7_of(b_over_l: f64) -> f64 {
    if b_over_l < 0.11 {
        0.229577 * b_over_l.powf(1.0 / 3.0)
    } else if b_over_l <= 0.25 {
        b_over_l
    } else {
        0.5 - 0.0625 / b_over_l
    }
}

fn c16_of(cp: f64) -> f64 {
    if cp < 0.80 {
        8.07981 * cp - 13.8673 * cp * cp + 6.984388 * cp.powi(3)
    } else {
        1.73014 - 0.7067 * cp
    }
}

fn c15_of(l_over_vol13: f64) -> f64 {
    if l_over_vol13 < 8.0 {
        // L³/∇ < 512
        -1.69385
    } else if l_over_vol13 <= 12.0 {
        // 512 ≤ L³/∇ ≤ 1727
        -1.69385 + (l_over_vol13 - 8.0) / 2.36
    } else {
        0.0
    }
}

/// `λ` from equation (13).
fn lambda_of(cp: f64, l_over_b: f64) -> f64 {
    if l_over_b < 12.0 {
        1.446 * cp - 0.03 * l_over_b
    } else {
        1.446 * cp - 0.36
    }
}

/// The wave-resistance coefficient set for one Froude number.
pub fn wave_coefficients(input: &HoltropInput, fn_: f64) -> WaveCoefficients {
    let l = input.length_wl;
    let b = input.beam_wl;
    let t = input.draft;
    let vol = input.volume_m3;
    let c7 = c7_of(b / l);
    let c16 = c16_of(input.cp);
    let l_over_vol13 = l / vol.cbrt();
    let c15 = c15_of(l_over_vol13);
    let lambda = lambda_of(input.cp, l / b);
    let c1 = 2223105.0 * c7.powf(3.78613) * (t / b).powf(1.07961)
        * (90.0 - input.half_entrance_deg).max(1.0).powf(-1.37565);
    // Bulb swell term: no bulb in the model means c2 = 1.
    let c3 = if input.bulb_area_m2 > 0.0 {
        0.56
            * input.bulb_area_m2.powf(1.5)
            / (b * t
                * (0.31 * input.bulb_area_m2.sqrt() + input.draft - input.bulb_centre_z)
                    .max(1e-6))
    } else {
        0.0
    };
    let c2 = (-1.89 * c3.sqrt()).exp();
    let c5 = if input.transom_area_m2 > 0.0 {
        (1.0 - 0.8 * input.transom_area_m2 / (b * t * input.cm.max(1e-6))).max(0.0)
    } else {
        1.0
    };
    let m1 = 0.0140407 * l / t - 1.75254 * vol.cbrt() / l - 4.79323 * b / l - c16;
    let m2 = c15 * input.cp * input.cp * (-0.1 / (fn_ * fn_)).exp();
    WaveCoefficients { c1, c2, c5, c7, lambda, m1, m2, c16 }
}

/// Wave resistance in newtons at Froude number `fn_`.
pub fn wave_resistance(input: &HoltropInput, fn_: f64) -> f64 {
    if fn_ <= 0.0 {
        return 0.0;
    }
    let w = wave_coefficients(input, fn_);
    let d = -0.9;
    w.c1 * w.c2 * w.c5 * input.volume_m3 * input.water_density * GRAVITY
        * (w.m1 * fn_.powf(d) + w.m2 * (w.lambda / (fn_ * fn_)).cos()).exp()
}

/// Total still-water resistance and its breakdown at `speed_m_s`.
pub fn holtrop(input: &HoltropInput, speed_m_s: f64) -> Resistance {
    let v = speed_m_s.max(0.0);
    let l = input.length_wl;
    let re = v * l / input.viscosity;
    let cf = if re > 1.0 { 0.075 / (re.log10() - 2.0).powi(2) } else { 0.0 };
    let fn_ = v / (GRAVITY * l).sqrt();
    let q = 0.5 * input.water_density * v * v;

    // Form factor, equation (9) with LR from (10) and c14 from (11).
    let lr = l * (1.0 - input.cp + 0.06 * input.cp * input.lcb_pct / (4.0 * input.cp - 1.0));
    let c14 = 1.0 + 0.011 * input.c_stern;
    let form_factor = 0.93
        + 0.487118
            * c14
            * (input.beam_wl / l).powf(1.06806)
            * (input.draft / l).powf(0.46106)
            * (l / lr.max(1e-6)).powf(0.121563)
            * (l.powi(3) / input.volume_m3).powf(0.36486)
            * (1.0 - input.cp).max(1e-6).powf(-0.604247);

    let rf = q * input.wetted_surface_m2 * cf;
    let rapp = if input.appendage_area_m2 > 0.0 {
        q * input.appendage_area_m2 * input.appendage_factor * cf
    } else {
        0.0
    };
    let rw = wave_resistance(input, fn_);

    // Bulb, equation (31)/(32): zero without a bulb.
    let rb = if input.bulb_area_m2 > 0.0 && input.draft > input.bulb_centre_z {
        let pb = 0.56 * input.bulb_area_m2.sqrt() / (input.draft - 1.5 * input.bulb_centre_z).max(1e-6);
        let fni = v / (GRAVITY * (input.draft - input.bulb_centre_z
            - 0.25 * input.bulb_area_m2.sqrt()))
        .max(1e-6)
        .sqrt();
        0.11 * (-3.0 / (pb * pb)).exp() * fni.powi(3) * input.bulb_area_m2 * input.water_density
            * GRAVITY
            / (1.0 + fni * fni)
    } else {
        0.0
    };

    // Immersed transom, equation (33)/(34) with c6 from (35).
    let rtr = if input.transom_area_m2 > 0.0 {
        let fnt = v / (2.0 * GRAVITY * input.transom_area_m2 / input.beam_wl).max(1e-6).sqrt();
        let c6 = if fnt < 5.0 { 0.2 * (1.0 - 0.2 * fnt) } else { 0.0 };
        q * input.transom_area_m2 * c6
    } else {
        0.0
    };

    // Correlation allowance, equations (37)-(39).
    let c4 = (input.draft / l).min(0.04);
    // c2 multiplies the bulb-swell term; with no bulb c3 = 0 and c2 = 1.
    let c2 = 1.0;
    let ca = 0.006 * (l + 100.0).powf(-0.16) - 0.00205
        + 0.003 * (l / 7.5).sqrt() * input.cb.powi(4) * c2 * (0.04 - c4).max(0.0);
    let ra = q * input.wetted_surface_m2 * ca;

    let rt = rf * form_factor + rapp + rw + rb + rtr + ra;
    Resistance {
        speed_kn: v / KNOT,
        speed_m_s: v,
        fn_,
        reynolds: re,
        cf,
        form_factor,
        rf_n: rf,
        rapp_n: rapp,
        rw_n: rw,
        rb_n: rb,
        rtr_n: rtr,
        ra_n: ra,
        rt_n: rt,
        pe_w: rt * v,
    }
}

/// Resistance at each speed in knots.
pub fn curve(input: &HoltropInput, speeds_kn: &[f64]) -> Vec<Resistance> {
    speeds_kn.iter().map(|s| holtrop(input, s * KNOT)).collect()
}

/// Build the input by measuring `surface` at its design condition.
pub fn input_for(surface: &crate::Surface, subdiv: SubdivisionOptions) -> Option<HoltropInput> {
    let opts = HydroOptions::from_surface(surface, subdiv);
    let hull = Hull::new(surface, subdiv);
    HoltropInput::measure(&hull, &opts)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> HoltropInput {
        HoltropInput {
            length_wl: 100.0,
            beam_wl: 15.0,
            draft: 5.0,
            volume_m3: 6000.0,
            cb: 0.8,
            cp: 0.8,
            cm: 1.0,
            cwp: 0.8,
            lcb_pct: 0.0,
            wetted_surface_m2: 2500.0,
            half_entrance_deg: 15.0,
            transom_area_m2: 0.0,
            bulb_area_m2: 0.0,
            bulb_centre_z: 0.0,
            c_stern: 0.0,
            appendage_area_m2: 0.0,
            appendage_factor: 1.5,
            water_density: 1025.0,
            viscosity: VISCOSITY_SEA_WATER,
        }
    }

    #[test]
    fn ittc57_matches_the_closed_form() {
        // CF = 0.075/(log10 Re - 2)^2; at Re = 1e9 that is 0.075/49.
        let mut i = sample();
        // Pick the speed that gives Re = 1e9 on 100 m.
        let v = 1e9 * i.viscosity / i.length_wl;
        i.volume_m3 = 6000.0;
        let r = holtrop(&i, v);
        assert!((r.reynolds - 1e9).abs() / 1e9 < 1e-9, "Re {}", r.reynolds);
        assert!((r.cf - 0.075 / 49.0).abs() < 1e-12, "CF {}", r.cf);
        assert!((r.rf_n - 0.5 * i.water_density * v * v * 2500.0 * r.cf).abs() < 1e-6);
    }

    #[test]
    fn form_factor_is_in_a_physical_range() {
        let i = sample();
        let r = holtrop(&i, 12.0 * KNOT);
        assert!(
            r.form_factor > 1.0 && r.form_factor < 2.0,
            "1+k1 = {}",
            r.form_factor
        );
    }

    #[test]
    fn resistance_grows_with_speed_and_equals_pe_over_v() {
        let i = sample();
        let curve = curve(&i, &[6.0, 10.0, 14.0, 18.0]);
        for w in curve.windows(2) {
            assert!(w[1].rt_n > w[0].rt_n, "{:?} -> {:?}", w[0].rt_n, w[1].rt_n);
            assert!(w[1].pe_w > w[0].pe_w);
        }
        for r in &curve {
            assert!((r.pe_w - r.rt_n * r.speed_m_s).abs() < 1e-6);
            assert!(r.rt_n > 0.0 && r.rt_n.is_finite());
        }
        // A 100 m hull at 14 kn: a few hundred kN, i.e. a few MW.
        let at14 = &curve[2];
        assert!(
            (200_000.0..1_500_000.0).contains(&at14.rt_n),
            "RT at 14 kn = {} N",
            at14.rt_n
        );
    }

    #[test]
    fn resistance_is_linear_in_density() {
        let mut a = sample();
        let mut b = sample();
        b.water_density = 2.0 * a.water_density;
        let ra = holtrop(&a, 12.0 * KNOT);
        let rb = holtrop(&b, 12.0 * KNOT);
        assert!((rb.rt_n / ra.rt_n - 2.0).abs() < 1e-9, "{}", rb.rt_n / ra.rt_n);
        a.water_density = 0.0;
        assert_eq!(holtrop(&a, 10.0).rt_n, 0.0);
    }

    #[test]
    fn wave_resistance_scales_with_the_cube_at_equal_froude_number() {
        // Froude scaling: a hull k times bigger at the same Fn has k^3 the
        // wave resistance, because RW = f(Fn) * rho g volume.
        let mut big = sample();
        big.length_wl *= 2.0;
        big.beam_wl *= 2.0;
        big.draft *= 2.0;
        big.volume_m3 *= 8.0;
        big.wetted_surface_m2 *= 4.0;
        let v_small = 5.0;
        let fn_small = v_small / (GRAVITY * sample().length_wl).sqrt();
        let v_big = fn_small * (GRAVITY * big.length_wl).sqrt();
        let rw_small = wave_resistance(&sample(), fn_small);
        let rw_big = wave_resistance(&big, v_big / (GRAVITY * big.length_wl).sqrt());
        assert!(
            (rw_big / rw_small - 8.0).abs() < 1e-6,
            "ratio {}",
            rw_big / rw_small
        );
    }

    #[test]
    fn piecewise_coefficients_switch_where_the_paper_says() {
        // c7: three branches. They are continuous at B/L = 0.11 (the first form
        // reduces to B/L there), which is the only real check — the switch
        // itself is arbitrary within round-off.
        let left = 0.229577 * 0.11_f64.powf(1.0 / 3.0);
        assert!((left - 0.11).abs() < 1e-6, "{left}");
        assert!((c7_of(0.10) - 0.229577 * 0.10_f64.powf(1.0 / 3.0)).abs() < 1e-12);
        assert!((c7_of(0.20) - 0.20).abs() < 1e-12);
        assert!((c7_of(0.30) - (0.5 - 0.0625 / 0.30)).abs() < 1e-12);
        // c16 switches at Cp = 0.80.
        assert!((c16_of(0.80) - (1.73014 - 0.7067 * 0.80)).abs() < 1e-12);
        let below = 8.07981 * 0.79 - 13.8673 * 0.79_f64.powi(2) + 6.984388 * 0.79_f64.powi(3);
        assert!((c16_of(0.79) - below).abs() < 1e-12);
        // λ switches at L/B = 12.
        assert!((lambda_of(0.8, 11.9) - (1.446 * 0.8 - 0.03 * 11.9)).abs() < 1e-12);
        assert!((lambda_of(0.8, 12.5) - (1.446 * 0.8 - 0.36)).abs() < 1e-12);
        // c15 has three branches on L/∇^(1/3).
        assert_eq!(c15_of(7.0), -1.69385);
        assert!((c15_of(12.0) - (-1.69385 + 4.0 / 2.36)).abs() < 1e-12);
        assert_eq!(c15_of(13.0), 0.0);
    }

    #[test]
    fn wave_resistance_has_the_expected_shape() {
        let i = sample();
        let rw_low = wave_resistance(&i, 0.10);
        let rw_mid = wave_resistance(&i, 0.20);
        let rw_high = wave_resistance(&i, 0.30);
        assert!(rw_low < rw_mid && rw_mid < rw_high, "{rw_low} {rw_mid} {rw_high}");
        // At Fn = 0.10 the wave term is a small part of the total.
        let r = holtrop(&i, 0.10 * (GRAVITY * i.length_wl).sqrt());
        assert!(r.rw_n / r.rt_n < 0.15, "wave share {}", r.rw_n / r.rt_n);
        // By Fn = 0.30 it dominates the friction term's growth.
        let r30 = holtrop(&i, 0.30 * (GRAVITY * i.length_wl).sqrt());
        assert!(r30.rw_n > 0.3 * r30.rt_n, "wave share {}", r30.rw_n / r30.rt_n);
    }

    #[test]
    fn a_full_displacement_hull_lands_in_the_powering_ballpark() {
        // A 100 m, 6000 m3 hull (Cp 0.8, S 2500 m2) at 12 kn (Fn 0.197). The
        // components are checked elsewhere; this pins the assembled magnitude
        // so a wrong coefficient cannot slip through as "plausible".
        let r = holtrop(&sample(), 12.0 * KNOT);
        assert!(
            (100_000.0..200_000.0).contains(&r.rt_n),
            "RT at Fn 0.197 = {:.1} kN",
            r.rt_n / 1e3
        );
        assert!(
            (600_000.0..1_200_000.0).contains(&r.pe_w),
            "PE at Fn 0.197 = {:.1} kW",
            r.pe_w / 1e3
        );
        // Friction dominates at this speed: the wave term is a tenth of it.
        assert!(r.rw_n < 0.5 * r.rf_n, "RW {} vs RF {}", r.rw_n, r.rf_n);
        assert!(r.form_factor > 1.2 && r.form_factor < 1.4, "{}", r.form_factor);
    }

    #[test]
    fn validity_ranges_are_reported() {
        let mut i = sample();
        assert!(i.warnings().is_empty(), "{:?}", i.warnings());
        i.cp = 0.50;
        i.draft = 5.0;
        i.beam_wl = 2.0; // L/B 50
        let w = i.warnings();
        assert!(w.iter().any(|s| s.contains("Cp")), "{w:?}");
        assert!(w.iter().any(|s| s.contains("L/B")), "{w:?}");
        assert!(w.iter().any(|s| s.contains("B/T")), "{w:?}");
    }
}

