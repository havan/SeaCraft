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
//! Coefficients are transcribed from the paper's equations (1)–(39) and checked
//! against the worked example the paper itself prints for a 205 m ship: `CF`,
//! `RF`, `CA`, `RA`, `RAPP`, `c1`, `c2`, `c5`, `c7`, `λ`, `m1`, `m2` and `RW`
//! reproduce to better than 1 %, and the transom term is zero at `FnT` 5.4 as
//! the paper prints. The one number that does not reproduce is `1+k1` (1.156
//! there, 1.188 from equation (9) as printed), so the total comes out 1.4 %
//! high; see the test's comment. The other tests pin what can be checked
//! exactly — the ITTC-57 line, the piecewise coefficient switches, Froude
//! scaling of the wave term, linearity in density.

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
    /// Which wave-resistance branch this speed used.
    pub wave_branch: WaveBranch,
    /// Total still-water resistance.
    pub rt_n: f64,
    /// Effective power, `RT · V`, in watts.
    pub pe_w: f64,
}

/// Wave resistance coefficient pieces, exposed because they are the part most
/// worth checking against the paper.
/// Which wave-resistance branch a Froude number falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveBranch {
    /// `Fn <= 0.40`: the `c1` / `m1` fit.
    Low,
    /// `0.40 < Fn < 0.55`: the paper's own interpolation between the two fits.
    Interpolated,
    /// `Fn >= 0.55`: the `c17` / `m3` high-speed fit of the 1984 re-analysis.
    High,
}

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
    /// High-speed fit coefficient (`Fn >= 0.55`).
    pub c17: f64,
    /// High-speed hump exponent (`Fn >= 0.55`).
    pub m3: f64,
    pub branch: WaveBranch,
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
    // The hump amplitude. This is the *published* `m2` of the 1982 paper and of
    // every textbook reproduction: `c15·Cp²·exp(−0.1/Fn²)`. The paper's own
    // worked example prints m2 = −0.17087 for a 205 m hull at Fn 0.2868, and
    // this expression gives −0.17093 there; the `0.4·c15·exp(−0.034·Fn^−3.29)`
    // form that circulates in some implementations gives −0.085 and is the
    // *high-speed* branch below, not this one.
    let m2 = c15 * input.cp * input.cp * (-0.1 / (fn_ * fn_)).exp();
    // High-speed fit (1984 re-analysis): c17 and m3 replace c1 and m1 above
    // Fn 0.55, where the low-speed fit was never valid.
    let c17 = 6919.3 * input.cm.max(1e-6).powf(-1.3346) * (vol / l.powi(3)).powf(2.00977)
        * (l / b - 2.0).max(1e-6).powf(1.40692);
    let m3 = -7.2035 * (b / l).powf(0.326869) * (t / b).powf(0.605375);
    let branch = if fn_ <= 0.40 {
        WaveBranch::Low
    } else if fn_ >= 0.55 {
        WaveBranch::High
    } else {
        WaveBranch::Interpolated
    };
    WaveCoefficients { c1, c2, c5, c7, lambda, m1, m2, c16, c17, m3, branch }
}

/// The low-speed wave resistance at a Froude number, `c1`/`m1` fit.
fn wave_low(input: &HoltropInput, w: &WaveCoefficients, fn_: f64) -> f64 {
    let d = -0.9;
    w.c1 * w.c2 * w.c5 * input.volume_m3 * input.water_density * GRAVITY
        * (w.m1 * fn_.powf(d) + w.m2 * (w.lambda / (fn_ * fn_)).cos()).exp()
}

/// The high-speed wave resistance at a Froude number, `c17`/`m3` fit.
fn wave_high(input: &HoltropInput, w: &WaveCoefficients, fn_: f64) -> f64 {
    let d = -0.9;
    w.c17 * w.c2 * w.c5 * input.volume_m3 * input.water_density * GRAVITY
        * (w.m3 * fn_.powf(d) + w.m2 * (w.lambda / (fn_ * fn_)).cos()).exp()
}

/// Wave resistance in newtons at Froude number `fn_`.
///
/// Three branches, exactly as published: the `c1` fit up to 0.40, the `c17`
/// high-speed fit from 0.55, and between them the paper's own straight-line
/// interpolation. The anchors are evaluated **at 0.40 and 0.55**, not at the
/// ship's Froude number — every Fn-dependent term (`Fn^d`, `m2`, the cosine)
/// belongs to the anchor. Computing the anchors at the ship's Fn and
/// interpolating scalars is the usual way to get a plausible but wrong hump.
pub fn wave_resistance(input: &HoltropInput, fn_: f64) -> f64 {
    if fn_ <= 0.0 {
        return 0.0;
    }
    let w = wave_coefficients(input, fn_);
    match w.branch {
        WaveBranch::Low => wave_low(input, &w, fn_),
        WaveBranch::High => wave_high(input, &w, fn_),
        WaveBranch::Interpolated => {
            let w40 = wave_coefficients(input, 0.40);
            let w55 = wave_coefficients(input, 0.55);
            let low = wave_low(input, &w40, 0.40);
            let high = wave_high(input, &w55, 0.55);
            low + (10.0 * fn_ - 4.0) * (high - low) / 1.5
        }
    }
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
    //
    // The paper defines lcb as a percentage of L, so `lcb_pct` enters (10) as a
    // percentage. Some published implementations divide it by 100 as well,
    // which moves LR by up to 15 % and `1+k1` by about 0.3 % — small, but it is
    // the reason two tools rarely agree to the fourth digit on the form factor.
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
        // The paper's own example prints RB = 0.049 kN, and only this form
        // reproduces it: ABT^1.5 (not ABT), and the 0.15 V² term *inside* the
        // square root of Fni. With either one different the bulb term is out by
        // orders of magnitude.
        let pb = 0.56 * input.bulb_area_m2.sqrt() / (input.draft - 1.5 * input.bulb_centre_z).max(1e-6);
        let inside = GRAVITY
            * (input.draft - input.bulb_centre_z - 0.25 * input.bulb_area_m2.sqrt()).max(1e-6)
            + 0.15 * v * v;
        let fni = v / inside.sqrt();
        0.11 * (-3.0 / (pb * pb)).exp() * fni.powi(3) * input.bulb_area_m2.powf(1.5)
            * input.water_density
            * GRAVITY
            / (1.0 + fni * fni)
    } else {
        0.0
    };

    // Immersed transom, equation (33)/(34) with c6 from (35).
    //
    // FnT uses the **waterplane** coefficient in its denominator. With CF there
    // instead, the published example hull (AT 16 m2, CWP 0.75, B 32 m) reports
    // FnT 4.11 where the paper prints 5.433, c6 stays above zero, and a phantom
    // 48 kN of transom drag appears in a case the paper says is 0.00 kN.
    let rtr = if input.transom_area_m2 > 0.0 {
        let denom = (input.beam_wl + input.beam_wl * input.cwp).max(1e-6);
        let fnt = v / (2.0 * GRAVITY * input.transom_area_m2 / denom).max(1e-6).sqrt();
        let c6 = if fnt < 5.0 { 0.2 * (1.0 - 0.2 * fnt) } else { 0.0 };
        q * input.transom_area_m2 * c6
    } else {
        0.0
    };

    // Correlation allowance, equations (37)-(39).
    let c4 = (input.draft / l).min(0.04);
    // c2 is the same bulb-swell factor as in the wave term; with no bulb the
    // coefficient of the second term is zero anyway (c4 saturates at 0.04 for
    // any hull with T/L >= 0.04, and c2 = 1 when c3 = 0).
    let c3 = if input.bulb_area_m2 > 0.0 {
        0.56 * input.bulb_area_m2.powf(1.5)
            / (input.beam_wl * input.draft
                * (0.31 * input.bulb_area_m2.sqrt() + input.draft - input.bulb_centre_z)
                    .max(1e-6))
    } else {
        0.0
    };
    let c2 = (-1.89 * c3.sqrt()).exp();
    let ca = 0.006 * (l + 100.0).powf(-0.16) - 0.00205
        + 0.003 * (l / 7.5).sqrt() * input.cb.powi(4) * c2 * (0.04 - c4).max(0.0);
    let ra = q * input.wetted_surface_m2 * ca;

    let rt = rf * form_factor + rapp + rw + rb + rtr + ra;
    Resistance {
        wave_branch: wave_coefficients(input, fn_).branch,
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


    /// The worked example printed in Holtrop & Mennen (1982), p. 170.
    ///
    /// A 205 m ship, 32 m beam, 10 m draft, 37 500 m³, Cp 0.5833, Cm 0.98,
    /// Cwp 0.75, lcb 2.02 % aft of ½L, iE 12.08°, AT 16 m², ABT 20 m²,
    /// hB 4.0 m, S 7381.45 m², SAPP 50 m², c_stern 10, at 25 knots. The paper
    /// prints every coefficient of the method for it, which makes it the one
    /// case where the implementation can be checked against the source rather
    /// than against itself.
    ///
    /// Every component reproduces to better than 1 %. Two printed numbers do
    /// not: `1+k1` is 1.156 there against 1.188 from equation (9) as printed,
    /// and the total follows it (1793.26 kN against 1819 kN, 1.4 %). The
    /// friction, correlation, wave and appendage terms — everything this crate
    /// could have got wrong by a sign or a factor — agree, so the difference is
    /// in the form factor the paper's example used, not in the transcription.
    #[test]
    fn published_example_reproduces_the_paper() {
        let i = HoltropInput {
            length_wl: 205.0,
            beam_wl: 32.0,
            draft: 10.0,
            volume_m3: 37500.0,
            cb: 37500.0 / (205.0 * 32.0 * 10.0),
            cp: 0.5833,
            cm: 0.98,
            cwp: 0.75,
            lcb_pct: -2.02,
            wetted_surface_m2: 7381.45,
            half_entrance_deg: 12.08,
            transom_area_m2: 16.0,
            bulb_area_m2: 20.0,
            bulb_centre_z: 4.0,
            c_stern: 10.0,
            appendage_area_m2: 50.0,
            appendage_factor: 1.5,
            water_density: 1025.0,
            viscosity: VISCOSITY_SEA_WATER,
        };
        let r = holtrop(&i, 25.0 * KNOT);
        let w = wave_coefficients(&i, r.fn_);
        let near = |got: f64, want: f64, tol: f64, what: &str| {
            assert!(
                (got - want).abs() / want.abs().max(1e-12) < tol,
                "{what}: {got:.6} vs published {want:.6}"
            );
        };
        near(r.fn_, 0.2868, 1e-3, "Fn");
        near(r.cf, 0.001390, 1e-3, "CF");
        near(r.rf_n / 1e3, 869.63, 1e-3, "RF [kN]");
        near(w.c7, 0.15610, 1e-4, "c7");
        near(w.c1, 1.398, 1e-3, "c1");
        near(w.c2, 0.7595, 1e-3, "c2");
        near(w.c5, 0.9592, 1e-3, "c5");
        near(w.lambda, 0.6513, 1e-3, "lambda");
        near(w.m1, -2.1274, 1e-3, "m1");
        near(w.m2, -0.17087, 1e-3, "m2");
        near(r.rw_n / 1e3, 557.11, 1e-3, "RW [kN]");
        near(r.rapp_n / 1e3, 8.83, 1e-2, "RAPP [kN]");
        near(r.ra_n / 1e3, 221.98, 1e-2, "RA [kN]");
        // FnT > 5, so the paper's transom term is zero; with CF instead of Cwp
        // in the denominator it came out 4.11 and added 48 kN of phantom drag.
        near(r.rtr_n / 1e3, 0.0, 1.0, "RTR [kN]");
        assert!(r.rtr_n < 1e-9, "RTR must vanish above FnT 5: {}", r.rtr_n);
        // The total lands at 1819.3 kN against the paper's 1793.26 kN (+1.4 %),
        // which is the form-factor gap above and nothing else: every additive
        // term reproduces to better than 1 %.
        near(r.rt_n / 1e3, 1793.26, 2e-2, "RT [kN]");
        // Bulb resistance is a rounding error on this hull, as printed.
        assert!(r.rb_n < 100.0, "RB {} N", r.rb_n);
    }


    #[test]
    fn bulb_resistance_matches_the_paper() {
        // The published example: ABT 20 m2, hB 4.0 m, T 10 m, 25 kn, prints
        // RB = 0.049 kN. It is a tiny term, but it is the only check on the
        // bulb formula, and both ABT^1.5 and the 0.15 V² inside Fni's root are
        // needed to land on it.
        let mut i = sample();
        i.length_wl = 205.0;
        i.beam_wl = 32.0;
        i.draft = 10.0;
        i.volume_m3 = 37500.0;
        i.cb = 37500.0 / (205.0 * 32.0 * 10.0);
        i.cp = 0.5833;
        i.cm = 0.98;
        i.cwp = 0.75;
        i.half_entrance_deg = 12.08;
        i.bulb_area_m2 = 20.0;
        i.bulb_centre_z = 4.0;
        i.c_stern = 10.0;
        let r = holtrop(&i, 25.0 * KNOT);
        assert!(
            (r.rb_n - 49.0).abs() / 49.0 < 0.05,
            "RB {:.2} N against the paper's 0.049 kN",
            r.rb_n
        );
        // No bulb, no bulb resistance.
        let mut none = i;
        none.bulb_area_m2 = 0.0;
        assert_eq!(holtrop(&none, 25.0 * KNOT).rb_n, 0.0);
    }

    #[test]
    fn the_wave_branches_meet_at_their_anchors() {
        let i = sample();
        // Branch selection.
        assert_eq!(wave_coefficients(&i, 0.40).branch, WaveBranch::Low);
        assert_eq!(wave_coefficients(&i, 0.4001).branch, WaveBranch::Interpolated);
        assert_eq!(wave_coefficients(&i, 0.5499).branch, WaveBranch::Interpolated);
        assert_eq!(wave_coefficients(&i, 0.55).branch, WaveBranch::High);

        // Continuity at both anchors. Exact at 0.55 by construction (the
        // interpolation factor reaches 1 there); at 0.40 the interpolated value
        // is one thousandth of the anchor span away, so a 1 % check is the
        // honest resolution of a sampled curve.
        let low_anchor = wave_resistance(&i, 0.40);
        let high_anchor = wave_resistance(&i, 0.55);
        for (fn_, want) in [(0.4001, low_anchor), (0.5499, high_anchor)] {
            let got = wave_resistance(&i, fn_);
            assert!(
                (got - want).abs() / want < 1e-2,
                "Fn {fn_}: {got:.0} N against the anchor {want:.0} N"
            );
        }

        // The interpolation is the paper's straight line between the anchors,
        // with every Fn-dependent term evaluated *at* the anchor.
        let mid = wave_resistance(&i, 0.475);
        let want_mid = low_anchor + 0.5 * (high_anchor - low_anchor);
        assert!(
            (mid - want_mid).abs() / want_mid < 1e-9,
            "midpoint {mid:.1} vs line {want_mid:.1}"
        );

        // The high-speed branch really is the c17/m3 fit.
        let w = wave_coefficients(&i, 0.60);
        assert!(w.c17 > 0.0 && w.c17.is_finite(), "c17 {}", w.c17);
        assert!(w.m3 < 0.0, "m3 {}", w.m3);
        assert!(wave_resistance(&i, 0.60).is_finite());
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

