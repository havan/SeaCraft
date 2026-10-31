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
use serde::{Deserialize, Serialize};

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
    /// `CV = (1+k1)·CF + CA` at the design speed, needed by the wake-fraction
    /// regression. [`HoltropInput::measure`] leaves it at 0; set it from the
    /// resistance result at the speed the propeller is designed for
    /// ([`Resistance::viscous_coefficient`]).
    pub viscous_coefficient: f64,
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
            viscous_coefficient: 0.0,
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
fn half_entrance_deg(hull: &Hull, wp: &crate::math::Plane, x_bow: f64, lwl: f64) -> f64 {
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

// ---------------------------------------------------------------------------
// Propulsion: wake fraction, thrust deduction, and the power chain
// ---------------------------------------------------------------------------

/// The propeller a power estimate is made for.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Propeller {
    /// Diameter in metres.
    pub diameter_m: f64,
    /// Expanded blade-area ratio `AE/A0`.
    pub expanded_area_ratio: f64,
    /// Open-water efficiency at the design point, from an open-water chart or a
    /// B-series polynomial. Holtrop's method does not predict it.
    pub open_water_efficiency: f64,
}

impl Default for Propeller {
    fn default() -> Self {
        Self {
            diameter_m: 0.0,
            expanded_area_ratio: 0.70,
            open_water_efficiency: 0.60,
        }
    }
}

/// Holtrop's propulsion factors for a single screw.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct Propulsion {
    /// Taylor wake fraction `w`.
    pub wake_fraction: f64,
    /// Thrust deduction `t`.
    pub thrust_deduction: f64,
    /// Relative rotative efficiency `ηR`.
    pub relative_rotative: f64,
    /// Hull efficiency `ηH = (1 − t)/(1 − w)`.
    pub hull_efficiency: f64,
    /// Open-water efficiency, as supplied.
    pub open_water_efficiency: f64,
    /// Quasi-propulsive coefficient `ηD = ηH · ηR · ηO`.
    pub propulsive_coefficient: f64,
    /// The viscous coefficient `CV` the wake formula is built on.
    pub cv: f64,
    /// False when the answer came out outside the range the regressions were
    /// fitted to (a wake fraction below 2 % or above 60 %, a thrust deduction
    /// outside 0–60 %, or an LCB further than 6 % of L from midships). The
    /// numbers are still returned — they are the formula's answer — but a
    /// designer should not trust them.
    pub plausible: bool,
}

impl HoltropInput {
    /// Wake fraction, thrust deduction and the propulsive coefficient.
    ///
    /// Equations (42)–(53) of Holtrop & Mennen: single-screw, twin-screw
    /// neglected, and the propeller's own open-water efficiency has to come
    /// from outside — the method predicts the hull's side of the bargain, not
    /// the propeller's.
    pub fn propulsion(&self, propeller: &Propeller) -> Propulsion {
        let d = propeller.diameter_m.max(1e-3);
        let (b, ta) = (self.beam_wl, self.draft);
        let s = self.wetted_surface_m2;
        let cp = self.cp;
        let lcb = self.lcb_pct;
        // CV = (1+k1)CF + CA, evaluated at a representative Froude number: the
        // wake and thrust-deduction regressions were fitted on trial data with
        // CF taken at the ship's speed, so the caller passes the speed in.
        let cv = self.viscous_coefficient;
        let bta = b / ta;
        let c8 = if bta < 5.0 {
            b * s.max(1e-6) / (self.length_wl * d * ta)
        } else {
            s.max(1e-6) * (7.0 * bta - 25.0) / (self.length_wl * d * (bta - 3.0))
        };
        let c9 = if c8 < 28.0 {
            c8
        } else {
            32.0 - 16.0 / (c8 - 24.0)
        };
        let tadover = ta / d;
        let c11 = if tadover < 2.0 {
            tadover
        } else {
            0.0833333 * tadover.powi(3) + 1.33333
        };
        let c19 = if bta < 5.0 {
            0.12997 / (bta - 0.95) - 0.11056 / (bta - 0.95).powi(2)
        } else {
            0.18567 / (1.3571 - self.cm) - 0.71276 + 0.38648 * cp
        };
        let c20 = 1.0 + 0.015 * self.c_stern;
        // `1 − Cp1` divides, so it is floored: a hull with a large |lcb| or a
        // high Cp pushes Cp1 towards 1 and the unguarded formula to absurd
        // wake fractions (the demo hull, lcb −9.9 %, returned w = 525).
        let cp1 = (1.45 * cp - 0.315 - 0.0225 * lcb).clamp(-1.0, 0.95);
        let wake = (c9
            * cv
            * (cv * self.length_wl / ta).sqrt()
            * (0.050776 + 0.93405 * c11 * cv / (1.0 - cp1)).max(0.0)
            + 0.27915 * c20 * (b / (self.length_wl * (1.0 - cp1)).max(1e-6)).sqrt()
            + c19 * c20)
            .max(0.0);
        let thrust =
            0.25014 * (b / self.length_wl).powf(0.28956) * ((b * ta).sqrt() / d).powf(0.2624)
                / (1.0 - cp + 0.0225 * lcb).max(1e-6).powf(0.01762)
                + 0.0015 * self.c_stern;
        let etar = 0.9922 - 0.05908 * propeller.expanded_area_ratio + 0.07424 * (cp - 0.0225 * lcb);
        let eta_h = (1.0 - thrust) / (1.0 - wake).max(1e-6);
        let plausible =
            (0.02..=0.60).contains(&wake) && (0.0..=0.60).contains(&thrust) && lcb.abs() <= 6.0;
        Propulsion {
            plausible,
            wake_fraction: wake,
            thrust_deduction: thrust,
            relative_rotative: etar,
            hull_efficiency: eta_h,
            open_water_efficiency: propeller.open_water_efficiency,
            propulsive_coefficient: eta_h * etar * propeller.open_water_efficiency,
            cv,
        }
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
    /// `CV = (1+k1)·CF + CA`, the viscous coefficient a wake-fraction estimate
    /// is built on.
    pub viscous_coefficient: f64,
    /// Which wave-resistance branch this speed used.
    pub wave_branch: WaveBranch,
    /// Total still-water resistance.
    pub rt_n: f64,
    /// Effective power, `RT · V`, in watts.
    pub pe_w: f64,
}

/// Still-water resistance prediction method.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    /// Holtrop & Mennen (1984) approximate power prediction method for displacement ships.
    #[default]
    Holtrop,
    /// Michell thin-ship wave resistance theory (1898) for slender hulls and catamarans.
    Michell,
    /// Delft Systematic Yacht Hull Series (DSYHS) bare-hull resistance for sailing yachts (Keuning & Katgert 2008).
    Dsyhs,
}

/// Wave resistance coefficient pieces, exposed because they are the part most
/// worth checking against the paper.
/// Which wave-resistance branch a Froude number falls in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WaveBranch {
    /// `Fn <= 0.40`: the `c1` / `m1` fit.
    Low,
    /// `0.40 < Fn < 0.55`: the paper's own interpolation between the two fits.
    Interpolated,
    /// `Fn >= 0.55`: the `c17` / `m3` high-speed fit of the 1984 re-analysis.
    High,
    /// Michell thin-ship wave resistance theory (1898).
    Michell,
    /// Delft Systematic Yacht Hull Series (DSYHS) residuary resistance (Keuning & Katgert 2008).
    Dsyhs,
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
    let c1 = 2223105.0
        * c7.powf(3.78613)
        * (t / b).powf(1.07961)
        * (90.0 - input.half_entrance_deg).max(1.0).powf(-1.37565);
    // Bulb swell term: no bulb in the model means c2 = 1.
    let c3 = if input.bulb_area_m2 > 0.0 {
        0.56 * input.bulb_area_m2.powf(1.5)
            / (b * t
                * (0.31 * input.bulb_area_m2.sqrt() + input.draft - input.bulb_centre_z).max(1e-6))
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
    let c17 = 6919.3
        * input.cm.max(1e-6).powf(-1.3346)
        * (vol / l.powi(3)).powf(2.00977)
        * (l / b - 2.0).max(1e-6).powf(1.40692);
    let m3 = -7.2035 * (b / l).powf(0.326869) * (t / b).powf(0.605375);
    let branch = if fn_ <= 0.40 {
        WaveBranch::Low
    } else if fn_ >= 0.55 {
        WaveBranch::High
    } else {
        WaveBranch::Interpolated
    };
    WaveCoefficients {
        c1,
        c2,
        c5,
        c7,
        lambda,
        m1,
        m2,
        c16,
        c17,
        m3,
        branch,
    }
}

/// The low-speed wave resistance at a Froude number, `c1`/`m1` fit.
fn wave_low(input: &HoltropInput, w: &WaveCoefficients, fn_: f64) -> f64 {
    let d = -0.9;
    w.c1 * w.c2
        * w.c5
        * input.volume_m3
        * input.water_density
        * GRAVITY
        * (w.m1 * fn_.powf(d) + w.m2 * (w.lambda / (fn_ * fn_)).cos()).exp()
}

/// The high-speed wave resistance at a Froude number, `c17`/`m3` fit.
fn wave_high(input: &HoltropInput, w: &WaveCoefficients, fn_: f64) -> f64 {
    let d = -0.9;
    w.c17
        * w.c2
        * w.c5
        * input.volume_m3
        * input.water_density
        * GRAVITY
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
        WaveBranch::Michell | WaveBranch::Dsyhs => 0.0,
    }
}

/// Total still-water resistance and its breakdown at `speed_m_s`.
pub fn holtrop(input: &HoltropInput, speed_m_s: f64) -> Resistance {
    let v = speed_m_s.max(0.0);
    let l = input.length_wl;
    let re = v * l / input.viscosity;
    let cf = if re > 1.0 {
        0.075 / (re.log10() - 2.0).powi(2)
    } else {
        0.0
    };
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
        let pb =
            0.56 * input.bulb_area_m2.sqrt() / (input.draft - 1.5 * input.bulb_centre_z).max(1e-6);
        let inside = GRAVITY
            * (input.draft - input.bulb_centre_z - 0.25 * input.bulb_area_m2.sqrt()).max(1e-6)
            + 0.15 * v * v;
        let fni = v / inside.sqrt();
        0.11 * (-3.0 / (pb * pb)).exp()
            * fni.powi(3)
            * input.bulb_area_m2.powf(1.5)
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
        let fnt = v
            / (2.0 * GRAVITY * input.transom_area_m2 / denom)
                .max(1e-6)
                .sqrt();
        let c6 = if fnt < 5.0 {
            0.2 * (1.0 - 0.2 * fnt)
        } else {
            0.0
        };
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
            / (input.beam_wl
                * input.draft
                * (0.31 * input.bulb_area_m2.sqrt() + input.draft - input.bulb_centre_z).max(1e-6))
    } else {
        0.0
    };
    let c2 = (-1.89 * c3.sqrt()).exp();
    let ca = 0.006 * (l + 100.0).powf(-0.16) - 0.00205
        + 0.003 * (l / 7.5).sqrt() * input.cb.powi(4) * c2 * (0.04 - c4).max(0.0);
    let ra = q * input.wetted_surface_m2 * ca;

    let rt = rf * form_factor + rapp + rw + rb + rtr + ra;
    Resistance {
        viscous_coefficient: form_factor * cf + ca,
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

// ---------------------------------------------------------------------------
// Delft Systematic Yacht Hull Series (DSYHS)
// ---------------------------------------------------------------------------

/// Geometric and hydrostatic parameters needed by the DSYHS upright bare-hull resistance model.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DsyhsInput {
    /// Waterline length LWL, m.
    pub length_wl: f64,
    /// Waterline maximum beam BWL, m.
    pub beam_wl: f64,
    /// Canoe-body draft Tc at midship, m.
    pub draft_canoe: f64,
    /// Underwater canoe-body volume of displacement ∇, m³.
    pub volume_m3: f64,
    /// Waterplane area Aw at zero speed, m².
    pub waterplane_area_m2: f64,
    /// Wetted surface area of canoe body Sw, m².
    pub wetted_surface_m2: f64,
    /// Prismatic coefficient Cp.
    pub cp: f64,
    /// Midship section area coefficient Cm.
    pub cm: f64,
    /// Longitudinal centre of buoyancy (LCB) measured from the forward perpendicular
    /// (FPP, fore end of DWL) as a fraction of LWL (positive aft towards stern, ~0.50-0.56).
    pub lcb_fpp_ratio: f64,
    /// Longitudinal centre of flotation (LCF) measured from the forward perpendicular
    /// (FPP, fore end of DWL) as a fraction of LWL (positive aft towards stern, ~0.50-0.60).
    pub lcf_fpp_ratio: f64,
    /// Form factor (1 + k). The DSYHS bare-hull regression was derived using Froude's
    /// hypothesis with 1+k = 1.0 on the ITTC-57 friction line.
    pub form_factor: f64,
    /// Water density, kg/m³.
    pub water_density: f64,
    /// Kinematic viscosity, m²/s.
    pub viscosity: f64,
}

/// Share of the largest piece's submerged volume below which a closed piece
/// of the hull is an appendage, not a hull of its own (a catamaran's two
/// demi-hulls both stay).
const APPENDAGE_VOLUME_SHARE: f64 = 0.15;

/// The hull without its appendages, or `None` when it has none.
///
/// The refined hull is split into its connected pieces, vertices welded by
/// position (a mirrored half joins its other half on the centreline). A
/// piece is an appendage when it is closed — every edge between two of its
/// triangles — and floats less than [`APPENDAGE_VOLUME_SHARE`] of the
/// largest piece below `wp`. Open pieces are never dropped: a hull built of
/// layers that do not share their boundary vertices is open piece by piece,
/// and their volumes mean nothing alone.
pub fn canoe_body(hull: &Hull, wp: &crate::math::Plane) -> Option<Hull> {
    use std::collections::HashMap;
    let key = |v: crate::math::Vec3| {
        let q = |c: f64| (c * 1e6).round() as i64;
        (q(v.x), q(v.y), q(v.z))
    };
    let mut ids: HashMap<(i64, i64, i64), usize> = HashMap::new();
    let welded: Vec<usize> = hull
        .verts
        .iter()
        .map(|&v| {
            let n = ids.len();
            *ids.entry(key(v)).or_insert(n)
        })
        .collect();
    let mut parent: Vec<usize> = (0..ids.len()).collect();
    fn find(parent: &mut [usize], mut i: usize) -> usize {
        while parent[i] != i {
            parent[i] = parent[parent[i]];
            i = parent[i];
        }
        i
    }
    for t in &hull.tris {
        let a = find(&mut parent, welded[t[0]]);
        for &v in &t[1..] {
            let b = find(&mut parent, welded[v]);
            parent[b] = a;
        }
    }
    let mut pieces: HashMap<usize, Vec<[usize; 3]>> = HashMap::new();
    for t in &hull.tris {
        let root = find(&mut parent, welded[t[0]]);
        pieces.entry(root).or_default().push(*t);
    }
    if pieces.len() < 2 {
        return None;
    }
    let closed = |tris: &[[usize; 3]]| {
        let mut edges: HashMap<(usize, usize), u32> = HashMap::new();
        for t in tris {
            for i in 0..3 {
                let (a, b) = (welded[t[i]], welded[t[(i + 1) % 3]]);
                *edges.entry((a.min(b), a.max(b))).or_default() += 1;
            }
        }
        edges.values().all(|&n| n == 2)
    };
    let volume = |tris: &[[usize; 3]]| hydro::integrate(&hull.verts, tris, wp).0;
    let sized: Vec<(f64, bool, Vec<[usize; 3]>)> = pieces
        .into_values()
        .map(|tris| (volume(&tris), closed(&tris), tris))
        .collect();
    let largest = sized.iter().map(|p| p.0).fold(0.0, f64::max);
    let mut tris = Vec::new();
    let mut dropped = false;
    for (v, closed, piece) in sized {
        if closed && v < APPENDAGE_VOLUME_SHARE * largest {
            dropped = true;
        } else {
            tris.extend(piece);
        }
    }
    if !dropped || tris.is_empty() {
        return None;
    }
    let keel = tris
        .iter()
        .flatten()
        .map(|&i| hull.verts[i].z)
        .fold(f64::INFINITY, f64::min);
    Some(Hull::from_panels(
        hull.verts.clone(),
        tris,
        keel,
        hull.length,
        hull.model_min,
        hull.model_max,
    ))
}

impl DsyhsInput {
    /// Measure DSYHS input numbers from a hull at its design condition.
    ///
    /// In SeaCraft's coordinate convention: x points forward (bow is at highest x),
    /// y points to starboard, and z points up.
    /// The Forward Perpendicular (FPP) is at `x_max` of the design waterline cut.
    /// Distances aft of FPP are therefore `x_max - x`.
    ///
    /// The series regresses on the *canoe body*: a fin keel, a rudder or a
    /// skeg modelled as a closed shell of its own (no vertex shared with the
    /// hull, as FREE!ship appendage layers usually are) is measured out —
    /// see [`canoe_body`] — and `draft_canoe` is the canoe body's draft, not
    /// the total draft to the fin's tip. A hull without such shells is
    /// measured as it is.
    pub fn measure(hull: &Hull, opts: &HydroOptions) -> Option<Self> {
        if opts.draft <= 0.0 {
            return None;
        }
        let wp =
            hydro::waterplane_for(hull.keel, hull.length, opts.draft, opts.trim, opts.heel_deg);
        if let Some(canoe) = canoe_body(hull, &wp) {
            // The same waterplane: `waterplane_for` places it at keel + draft.
            let mut o = opts.clone();
            o.draft = opts.draft + hull.keel - canoe.keel;
            return Self::measure_hull(&canoe, &o);
        }
        Self::measure_hull(hull, opts)
    }

    fn measure_hull(hull: &Hull, opts: &HydroOptions) -> Option<Self> {
        let mut o = opts.clone();
        o.stations = 0;
        if o.draft <= 0.0 {
            return None;
        }
        let h = hull.hydrostatics(&o);
        if h.volume_m3 <= 0.0 || h.lwl <= 0.0 || h.bwl <= 0.0 {
            return None;
        }
        let wp = hydro::waterplane_for(hull.keel, hull.length, o.draft, o.trim, o.heel_deg);
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

        // FPP is at x_max (the forward end of the waterline).
        // Longitudinal positions measured aft of FPP:
        let lcb_fpp = (x_max - h.lcb.x).max(0.0);
        let lcf_fpp = (x_max - h.lcf.x).max(1e-6);
        let lcb_fpp_ratio = lcb_fpp / lwl;
        let lcf_fpp_ratio = lcf_fpp / lwl;

        Some(Self {
            length_wl: lwl,
            beam_wl: h.bwl,
            draft_canoe: o.draft,
            volume_m3: h.volume_m3,
            waterplane_area_m2: h.waterplane_area_m2,
            wetted_surface_m2: h.wetted_surface_m2,
            cp: h.prismatic_coefficient,
            cm: h.midship_coefficient,
            lcb_fpp_ratio,
            lcf_fpp_ratio,
            form_factor: 1.0,
            water_density: o.water_density,
            viscosity: if (o.water_density - 1000.0).abs() < 25.0 {
                VISCOSITY_FRESH_WATER
            } else {
                VISCOSITY_SEA_WATER
            },
        })
    }

    /// Check hull parameters against published DSYHS series validity boundaries.
    ///
    /// Ranges as published in Keuning & Katgert (2008), Table 1 & text:
    /// - Cp: 0.52 to 0.60
    /// - Cm: 0.65 to 0.78
    /// - LWL / BWL: 2.73 to 5.00
    /// - BWL / Tc: 2.46 to 19.38
    /// - LWL / ∇^(1/3): 4.34 to 8.50
    /// - LCB from FPP: 0.50 to 0.56 · LWL
    pub fn warnings(&self) -> Vec<String> {
        let mut out = Vec::new();
        let lb = if self.beam_wl > 1e-6 {
            self.length_wl / self.beam_wl
        } else {
            0.0
        };
        let bt = if self.draft_canoe > 1e-6 {
            self.beam_wl / self.draft_canoe
        } else {
            0.0
        };
        let l_vol = if self.volume_m3 > 1e-6 {
            self.length_wl / self.volume_m3.cbrt()
        } else {
            0.0
        };
        if !(0.52..=0.60).contains(&self.cp) {
            out.push(format!(
                "Cp {:.3} is outside DSYHS valid range 0.52-0.60",
                self.cp
            ));
        }
        if !(0.65..=0.78).contains(&self.cm) {
            out.push(format!(
                "Cm {:.3} is outside DSYHS valid range 0.65-0.78",
                self.cm
            ));
        }
        if !(2.73..=5.00).contains(&lb) {
            out.push(format!(
                "L/B {lb:.2} is outside DSYHS valid range 2.73-5.00"
            ));
        }
        if !(2.46..=19.38).contains(&bt) {
            out.push(format!(
                "B/T {bt:.2} is outside DSYHS valid range 2.46-19.38"
            ));
        }
        if !(4.34..=8.50).contains(&l_vol) {
            out.push(format!(
                "L/∇^(1/3) {l_vol:.2} is outside DSYHS valid range 4.34-8.50"
            ));
        }
        if !(0.50..=0.56).contains(&self.lcb_fpp_ratio) {
            out.push(format!(
                "LCB from FPP {:.3}·LWL is outside DSYHS valid range 0.50-0.56",
                self.lcb_fpp_ratio
            ));
        }
        out
    }
}

#[rustfmt::skip]
const DSYHS_FN_GRID: [f64; 13] = [
    0.15, 0.20, 0.25, 0.30, 0.35, 0.40, 0.45, 0.50, 0.55, 0.60, 0.65, 0.70, 0.75,
];

// Source:
// Authors: J. A. Keuning and M. Katgert
// Title: "A bare hull resistance prediction method derived from the results of the Delft Systematic Yacht Hull Series extended to higher speeds"
// Year: 2008
// Conference: Proceedings of the International Conference on Innovation in High Performance Sailing Yachts (InnovSail '08), Lorient, France, 29-30 May 2008.
// URL: https://www.scribd.com/document/346520042/Keuning-2008-PDF (also indexed on boatdesign.net)
// Table: Table 2 ("The coefficients for the polynomial for the untrimmed upright residuary resistance of the bare hull")
//
// Cross-check (2026-10-23, against an independent reproduction, not the paper
// itself): P. Teixeira, "Development of a Velocity Prediction Program for
// Sailing Yachts", MSc thesis, IST Lisbon, Appendix IV, Table IV-1
// (https://fenix.tecnico.ulisboa.pt/downloadFile/3378094857519920/Thesis_Pedro_Teixeira_104081_V3.pdf).
// All 104 coefficients agree except a3 at Fn 0.55, printed there as 0.8762
// (0.8726 here, which is also the value another open VPP project quotes from
// the paper's Table 2). The thesis labels its 13 columns Fn 0.10-0.70; the
// paper extends the series to Fn 0.75 and the 0.55 quote above fixes the
// labelling used here, 0.15-0.75.
//
// Columns: [a0, a1, a2, a3, a4, a5, a6, a7]
// Variables:
//   a0: constant
//   a1: LCB / LWL (measured from FPP, fraction of LWL, positive aft)
//   a2: Cp (prismatic coefficient)
//   a3: ∇^(2/3) / Aw (displacement volume to waterplane area ratio)
//   a4: BWL / LWL (waterline beam to length ratio)
//   a5: LCB / LCF (ratio of centers from FPP)
//   a6: BWL / Tc (beam to canoe-body draft ratio)
//   a7: Cm (midship section area coefficient)
#[rustfmt::skip]
const DSYHS_COEFFICIENTS: [[f64; 8]; 13] = [
    // Fn = 0.15
    [-0.0005,  0.0023, -0.0086, -0.0015,  0.0061,  0.0010,  0.0001,  0.0052],
    // Fn = 0.20
    [-0.0003,  0.0059, -0.0064,  0.0070,  0.0014,  0.0013,  0.0005, -0.0020],
    // Fn = 0.25
    [-0.0002, -0.0156,  0.0031, -0.0021, -0.0070,  0.0148,  0.0010, -0.0043],
    // Fn = 0.30
    [-0.0009,  0.0016,  0.0337, -0.0285, -0.0367,  0.0218,  0.0015, -0.0172],
    // Fn = 0.35
    [-0.0026, -0.0567,  0.0446, -0.1091, -0.0707,  0.0914,  0.0021, -0.0078],
    // Fn = 0.40
    [-0.0064, -0.4034, -0.1250,  0.0273, -0.1341,  0.3578,  0.0045,  0.1115],
    // Fn = 0.45
    [-0.0218, -0.5261, -0.2945,  0.2485, -0.2428,  0.6293,  0.0081,  0.2086],
    // Fn = 0.50
    [-0.0388, -0.5986, -0.3038,  0.6033, -0.0430,  0.8332,  0.0106,  0.1336],
    // Fn = 0.55
    [-0.0347, -0.4764, -0.2361,  0.8726,  0.4219,  0.8990,  0.0096, -0.2272],
    // Fn = 0.60
    [-0.0361,  0.0037, -0.2960,  0.9661,  0.6123,  0.7534,  0.0100, -0.3352],
    // Fn = 0.65
    [ 0.0008,  0.3728, -0.3667,  1.3957,  1.0343,  0.3230,  0.0072, -0.4632],
    // Fn = 0.70
    [ 0.0108, -0.1238, -0.2026,  1.1282,  1.1836,  0.4973,  0.0038, -0.4477],
    // Fn = 0.75
    [ 0.1023,  0.7726,  0.5040,  1.7867,  2.1934, -1.5479, -0.0115, -0.0977],
];

/// Residuary resistance in newtons by the DSYHS polynomial (Keuning & Katgert 2008).
///
/// Interpolates linearly between tabulated Froude numbers without extrapolation
/// (clamped to 0.15..=0.75).
pub fn dsyhs_residuary_resistance(input: &DsyhsInput, fn_: f64) -> f64 {
    if fn_ <= 0.0 || input.volume_m3 <= 0.0 || input.length_wl <= 0.0 {
        return 0.0;
    }
    let fn_clamped = fn_.clamp(0.15, 0.75);
    let idx_f = (fn_clamped - 0.15) / 0.05;
    let idx = (idx_f.floor() as usize).min(11);
    let t = (fn_clamped - DSYHS_FN_GRID[idx]) / 0.05;

    let x1 = input.lcb_fpp_ratio;
    let x2 = input.cp;
    let x3 = input.volume_m3.powf(2.0 / 3.0) / input.waterplane_area_m2.max(1e-6);
    let x4 = input.beam_wl / input.length_wl.max(1e-6);
    let x5 = input.lcb_fpp_ratio / input.lcf_fpp_ratio.max(1e-6);
    let x6 = input.beam_wl / input.draft_canoe.max(1e-6);
    let x7 = input.cm;

    let eval_poly = |coeffs: &[f64; 8]| -> f64 {
        coeffs[0]
            + coeffs[1] * x1
            + coeffs[2] * x2
            + coeffs[3] * x3
            + coeffs[4] * x4
            + coeffs[5] * x5
            + coeffs[6] * x6
            + coeffs[7] * x7
    };

    let p0 = eval_poly(&DSYHS_COEFFICIENTS[idx]);
    let p1 = eval_poly(&DSYHS_COEFFICIENTS[idx + 1]);
    let p = (1.0 - t) * p0 + t * p1;

    let displacement_weight = input.volume_m3 * input.water_density * GRAVITY;
    let l_disp_factor = input.volume_m3.cbrt() / input.length_wl.max(1e-6);
    let rr = displacement_weight * l_disp_factor * p;
    rr.max(0.0)
}

/// Total still-water bare-hull resistance for sailing yachts (DSYHS Keuning & Katgert 2008).
///
/// Frictional resistance is computed with ITTC-57 line on canoe-body wetted surface
/// at Reynolds length `0.7·LWL`, as defined by the Delft series.
pub fn dsyhs(input: &DsyhsInput, speed_m_s: f64) -> Resistance {
    let v = speed_m_s.max(0.0);
    let l = input.length_wl;
    let fn_ = if l > 1e-6 {
        v / (GRAVITY * l).sqrt()
    } else {
        0.0
    };

    // Frictional resistance: ITTC-57 with L_ref = 0.7 * LWL per Delft series definition.
    let l_ref = 0.7 * l;
    let re = if input.viscosity > 1e-9 {
        v * l_ref / input.viscosity
    } else {
        0.0
    };
    let cf = if re > 1.0 {
        0.075 / (re.log10() - 2.0).powi(2)
    } else {
        0.0
    };
    let q = 0.5 * input.water_density * v * v;
    let rf = q * input.wetted_surface_m2 * cf;

    let rr = if v > 1e-6 {
        dsyhs_residuary_resistance(input, fn_)
    } else {
        0.0
    };

    let form_factor = input.form_factor;
    let rt = rf * form_factor + rr;

    Resistance {
        speed_kn: v / KNOT,
        speed_m_s: v,
        fn_,
        reynolds: re,
        cf,
        form_factor,
        rf_n: rf,
        rapp_n: 0.0,
        rw_n: rr,
        rb_n: 0.0,
        rtr_n: 0.0,
        ra_n: 0.0,
        viscous_coefficient: form_factor * cf,
        wave_branch: WaveBranch::Dsyhs,
        rt_n: rt,
        pe_w: rt * v,
    }
}

/// DSYHS resistance curve at speeds in knots.
pub fn dsyhs_curve(input: &DsyhsInput, speeds_kn: &[f64]) -> Vec<Resistance> {
    speeds_kn.iter().map(|&s| dsyhs(input, s * KNOT)).collect()
}

/// Measure DSYHS input from a Surface at its design condition.
pub fn dsyhs_input_for(surface: &crate::Surface, subdiv: SubdivisionOptions) -> Option<DsyhsInput> {
    let opts = HydroOptions::from_surface(surface, subdiv);
    let hull = Hull::new(surface, subdiv);
    DsyhsInput::measure(&hull, &opts)
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
            viscous_coefficient: 0.0020,
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
        assert!(
            (rb.rt_n / ra.rt_n - 2.0).abs() < 1e-9,
            "{}",
            rb.rt_n / ra.rt_n
        );
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
        assert!(
            rw_low < rw_mid && rw_mid < rw_high,
            "{rw_low} {rw_mid} {rw_high}"
        );
        // At Fn = 0.10 the wave term is a small part of the total.
        let r = holtrop(&i, 0.10 * (GRAVITY * i.length_wl).sqrt());
        assert!(r.rw_n / r.rt_n < 0.15, "wave share {}", r.rw_n / r.rt_n);
        // By Fn = 0.30 it dominates the friction term's growth.
        let r30 = holtrop(&i, 0.30 * (GRAVITY * i.length_wl).sqrt());
        assert!(
            r30.rw_n > 0.3 * r30.rt_n,
            "wave share {}",
            r30.rw_n / r30.rt_n
        );
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
        assert!(
            r.form_factor > 1.2 && r.form_factor < 1.4,
            "{}",
            r.form_factor
        );
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
            viscous_coefficient: 0.0,
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
        assert_eq!(
            wave_coefficients(&i, 0.4001).branch,
            WaveBranch::Interpolated
        );
        assert_eq!(
            wave_coefficients(&i, 0.5499).branch,
            WaveBranch::Interpolated
        );
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
    fn relative_rotative_efficiency_matches_the_paper() {
        // The paper's example prints ηR = 0.9931 for a 4-blade propeller; its
        // expanded area ratio is not printed, but inverting the formula gives
        // AE/A0 = 0.775, and that value reproduces the printed number exactly.
        let mut i = sample();
        i.length_wl = 205.0;
        i.beam_wl = 32.0;
        i.draft = 10.0;
        i.volume_m3 = 37500.0;
        i.cp = 0.5833;
        i.cm = 0.98;
        i.lcb_pct = -2.02;
        let prop = Propeller {
            diameter_m: 8.0,
            expanded_area_ratio: 0.775,
            open_water_efficiency: 0.60,
        };
        let p = i.propulsion(&prop);
        assert!(
            (p.relative_rotative - 0.9931).abs() < 5e-4,
            "etaR {:.4} against the paper's 0.9931",
            p.relative_rotative
        );
        // And the chain closes: ηD = ηH · ηR · ηO with ηH = (1−t)/(1−w).
        assert!(
            (p.hull_efficiency - (1.0 - p.thrust_deduction) / (1.0 - p.wake_fraction)).abs()
                < 1e-12
        );
        assert!(
            (p.propulsive_coefficient - p.hull_efficiency * p.relative_rotative * 0.60).abs()
                < 1e-12
        );
    }

    #[test]
    fn propulsion_factors_stay_physical_and_move_the_right_way() {
        let mut i = sample();
        i.viscous_coefficient = 0.0021;
        let prop = Propeller {
            diameter_m: 5.0,
            expanded_area_ratio: 0.70,
            open_water_efficiency: 0.55,
        };
        let base = i.propulsion(&prop);
        assert!(
            base.wake_fraction > 0.10 && base.wake_fraction < 0.45,
            "w {}",
            base.wake_fraction
        );
        assert!(
            base.thrust_deduction > 0.05 && base.thrust_deduction < 0.40,
            "t {}",
            base.thrust_deduction
        );
        assert!(
            base.hull_efficiency > 0.8 && base.hull_efficiency < 1.5,
            "ηH {}",
            base.hull_efficiency
        );

        // A beamier hull at the same length drags more water: more wake.
        let mut wide = i.clone();
        wide.beam_wl *= 1.3;
        assert!(
            wide.propulsion(&prop).wake_fraction > base.wake_fraction,
            "wake must grow with beam"
        );
        // A fuller hull (higher Cp) has *more* thrust deduction: the coefficient
        // divides by (1 − Cp + 0.0225·lcb)^0.01762, so the exponent is tiny but
        // the direction is the physical one.
        let mut full = i.clone();
        full.cp += 0.06;
        assert!(
            full.propulsion(&prop).thrust_deduction > base.thrust_deduction,
            "t must grow with Cp: {} vs {}",
            full.propulsion(&prop).thrust_deduction,
            base.thrust_deduction
        );
        // A degenerate propeller must not produce infinities.
        let tiny = Propeller {
            diameter_m: 0.0,
            expanded_area_ratio: 0.0,
            open_water_efficiency: 0.0,
        };
        let p = i.propulsion(&tiny);
        for v in [
            p.wake_fraction,
            p.thrust_deduction,
            p.hull_efficiency,
            p.propulsive_coefficient,
        ] {
            assert!(v.is_finite(), "{p:?}");
        }
        assert_eq!(
            p.propulsive_coefficient, 0.0,
            "no open-water efficiency, no power"
        );
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

    /// R8-2 A07. DSYHS regresses on the canoe body. The 10 m yacht floats
    /// at T = 1.85 m on its fin; its canoe body draws Tc = 0.45 m. The fin
    /// and rudder are closed shells of their own, so they are measured out:
    /// Tc, not T, and B/Tc ≈ 6.4 inside the series' range. A hull without
    /// separate appendages (the demo) is measured as it is.
    #[test]
    fn dsyhs_measures_the_canoe_body_without_the_fin_and_rudder() {
        let level3 = SubdivisionOptions {
            level: 3,
            mode: crate::subdiv::SubdivisionMode::Loop,
        };
        let (s, _) = crate::examples::build(crate::examples::Example::Yacht10m).unwrap();
        let hull = Hull::new(&s, level3);
        let mut opts = HydroOptions::from_surface(&s, level3);
        opts.draft = 1.85;
        let whole = hull.hydrostatics(&opts);
        let d = DsyhsInput::measure(&hull, &opts).expect("measures");
        assert!((d.draft_canoe - 0.45).abs() < 1e-6, "Tc {}", d.draft_canoe);
        assert!(
            d.volume_m3 < whole.volume_m3,
            "{} vs {}",
            d.volume_m3,
            whole.volume_m3
        );
        assert!(
            !d.warnings().iter().any(|w| w.contains("B/Tc")),
            "{:?}",
            d.warnings()
        );

        let demo = crate::demo::demo_hull(false);
        let hull = Hull::new(&demo, level3);
        let mut opts = HydroOptions::from_surface(&demo, level3);
        opts.draft = 0.62;
        let d = DsyhsInput::measure(&hull, &opts).expect("measures");
        assert_eq!(d.draft_canoe, 0.62);
        assert_eq!(d.volume_m3, hull.hydrostatics(&opts).volume_m3);
    }

    fn sample_dsyhs() -> DsyhsInput {
        DsyhsInput {
            length_wl: 10.0,
            beam_wl: 3.0,
            draft_canoe: 0.50,
            volume_m3: 5.0,
            waterplane_area_m2: 18.0,
            wetted_surface_m2: 25.0,
            cp: 0.55,
            cm: 0.70,
            lcb_fpp_ratio: 0.52,
            lcf_fpp_ratio: 0.56,
            form_factor: 1.0,
            water_density: 1025.0,
            viscosity: VISCOSITY_SEA_WATER,
        }
    }

    #[test]
    fn dsyhs_polynomial_at_tabulated_fn_matches_hand_arithmetic() {
        let input = sample_dsyhs();
        let rr = dsyhs_residuary_resistance(&input, 0.40);

        // Arithmetic hand computation at Fn = 0.40:
        // Hull parameters:
        // LWL = 10.0 m, BWL = 3.0 m, Tc = 0.50 m, volume = 5.0 m³,
        // Aw = 18.0 m², Sw = 25.0 m², Cp = 0.55, Cm = 0.70,
        // LCB from FPP = 5.2 m (LCB/LWL = 0.52),
        // LCF from FPP = 5.6 m (LCF/LWL = 0.56, LCB/LCF = 5.2 / 5.6 = 13/14),
        // rho = 1025.0 kg/m³, g = 9.80665 m/s².
        //
        // Dimensionless terms:
        // x1 = LCB / LWL = 0.52
        // x2 = Cp = 0.55
        // x3 = ∇^(2/3) / Aw = 5.0^(2/3) / 18.0 = 2.924017738212866 / 18.0 = 0.16244542990071478
        // x4 = BWL / LWL = 3.0 / 10.0 = 0.30
        // x5 = LCB / LCF = 5.2 / 5.6 = 13.0 / 14.0 = 0.9285714285714286
        // x6 = BWL / Tc = 3.0 / 0.50 = 6.0
        // x7 = Cm = 0.70
        //
        // Coefficients from Keuning & Katgert (2008), Table 2 at Fn = 0.40:
        // a0 = -0.0064
        // a1 = -0.4034
        // a2 = -0.1250
        // a3 =  0.0273
        // a4 = -0.1341
        // a5 =  0.3578
        // a6 =  0.0045
        // a7 =  0.1115
        //
        // Individual polynomial terms:
        // term0 = a0                          = -0.0064
        // term1 = a1 * x1 = -0.4034 * 0.52    = -0.209768
        // term2 = a2 * x2 = -0.1250 * 0.55    = -0.068750
        // term3 = a3 * x3 =  0.0273 * (5^(2/3)/18) = 0.004434755236289513
        // term4 = a4 * x4 = -0.1341 * 0.30    = -0.040230
        // term5 = a5 * x5 =  0.3578 * (13/14) =  0.33224285714285715
        // term6 = a6 * x6 =  0.0045 * 6.0     =  0.027000
        // term7 = a7 * x7 =  0.1115 * 0.70    =  0.078050
        //
        // Polynomial sum P:
        // P = term0 + term1 + term2 + term3 + term4 + term5 + term6 + term7
        //   = 0.11657961237914666
        //
        // Scale factors:
        // ∇^(1/3) / LWL = 5.0^(1/3) / 10.0 = 0.1709975946676697
        // ∇ * rho * g = 5.0 * 1025.0 * 9.80665 = 50259.08125 N
        //
        // Expected residuary resistance Rr:
        // Rr = (∇ * rho * g) * (∇^(1/3) / LWL) * P
        //    = 50259.08125 * 0.1709975946676697 * 0.11657955394017726
        //    = 1001.9064497080522 N
        let expected_rr = 1001.9064497080522;
        assert!(
            (rr - expected_rr).abs() < 1e-4,
            "hand arithmetic: got {rr}, expected {expected_rr}"
        );
    }

    #[test]
    fn dsyhs_interpolation_midway_between_fn_rows() {
        let input = sample_dsyhs();
        let r_35 = dsyhs_residuary_resistance(&input, 0.35);
        let r_40 = dsyhs_residuary_resistance(&input, 0.40);
        let r_mid = dsyhs_residuary_resistance(&input, 0.375);
        let expected = 0.5 * (r_35 + r_40);
        assert!(
            (r_mid - expected).abs() < 1e-9,
            "midway interpolation: got {r_mid}, expected {expected}"
        );
    }

    #[test]
    fn dsyhs_worked_example_source_paper_note() {
        // The source paper Keuning & Katgert (2008) "A bare hull resistance prediction method
        // derived from the results of the Delft Systematic Yacht Hull Series extended to higher speeds"
        // lists model geometric particulars in Table 3 (page 8) and plots calculated vs measured
        // resistance curves in Figures 5-9 (pages 9-11), but does not print numeric force values in a
        // worked-example table. Test (1) verifies the exact formula against hand arithmetic from Table 2.
    }

    #[test]
    fn dsyhs_parametric_yacht_resistance_finite_positive_increasing() {
        use crate::parametric::{self, HullKind, HullSpec};
        use crate::subdiv::SubdivisionOptions;

        let spec = HullSpec::default_for(HullKind::Yacht);
        let subdiv = SubdivisionOptions {
            level: 2,
            ..Default::default()
        };
        let (surface, _report) = parametric::generate(&spec).expect("generate yacht surface");
        let dsyhs_in = dsyhs_input_for(&surface, subdiv).expect("dsyhs input for yacht");

        let speeds = [4.0, 5.0, 6.0, 7.0, 8.0];
        let curve = dsyhs_curve(&dsyhs_in, &speeds);
        assert_eq!(curve.len(), 5);

        for w in curve.windows(2) {
            assert!(
                w[1].rt_n > w[0].rt_n,
                "Rt must increase with speed: {} kn ({:.1} N) vs {} kn ({:.1} N)",
                w[0].speed_kn,
                w[0].rt_n,
                w[1].speed_kn,
                w[1].rt_n
            );
            assert!(w[1].pe_w > w[0].pe_w, "Pe must increase with speed");
        }

        for r in &curve {
            assert!(
                r.rt_n > 0.0 && r.rt_n.is_finite(),
                "Rt must be finite and positive: {}",
                r.rt_n
            );
            assert!(
                r.rf_n > 0.0 && r.rf_n.is_finite(),
                "Rf must be finite and positive: {}",
                r.rf_n
            );
            assert!(
                r.rw_n >= 0.0 && r.rw_n.is_finite(),
                "Rr must be finite and non-negative: {}",
                r.rw_n
            );
            assert_eq!(r.wave_branch, WaveBranch::Dsyhs);
        }

        let warnings = dsyhs_in.warnings();
        assert!(
            !warnings.is_empty(),
            "expected warnings for out-of-range yacht parameters, got none"
        );
    }
}
