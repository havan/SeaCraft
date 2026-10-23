//! Design loop: reshape a hull to cut calm-water resistance at a speed while
//! holding displacement and stability.
//!
//! Two parts, deliberately separate:
//!
//! * [`nelder_mead`] — a dependency-free simplex minimiser with the standard
//!   coefficients (reflection 1, expansion 2, contraction ½, shrink ½). Bounds
//!   are enforced by clamping every trial point, convergence needs both a small
//!   simplex *and* a small spread of function values, and a cap on evaluations
//!   always ends the run. It knows nothing about hulls.
//! * [`optimize_hull`] — maps a design vector to a candidate hull, measures it
//!   on the **refined mesh** like every other measurement here, and hands the
//!   scalar objective to the minimiser.
//!
//! # Design vector
//!
//! Up to three variables, each a [`DesignVar`] with its own bounds. A missing
//! variable keeps its neutral value.
//!
//! | variable | neutral | effect |
//! |---|---|---|
//! | `cp_delta` | 0 | change in prismatic coefficient, by [`crate::lackenby`] station shift |
//! | `lcb_delta_frac` | 0 | change in `LCB` as a *fraction of `LWL`* (0.01 = 1 % of the length), same shift |
//! | `lb_ratio_scale` | 1 | `x` scaled by `s`, `y` by `1/s`: `L/B` becomes `s²` times what it was, and the displaced volume is unchanged at a given draft |
//!
//! The order is fixed: Lackenby first (its targets are the baseline `Cp`/`LCB`
//! plus the deltas, clamped to the range that method accepts), then the
//! `x`/`y` scale. The draft is then **re-solved** with the same root finder the
//! stability code uses so that the hull floats the original displacement, kept
//! within 0.7–1.3 of the starting draft; if the displacement still cannot be
//! met inside that window the error shows up in the objective as a penalty.
//! Trim is zero and heel is zero throughout.
//!
//! # Objective
//!
//! `f = R_T(speed)` in kN from [`Method`] — Holtrop–Mennen, or Savitsky for a
//! planing hull, `Auto` picking Savitsky at `Fn_L >= 1` when the hull actually
//! planes there — plus quadratic penalties, each scaled by the baseline `R_T`
//! so the weights mean the same thing on a 12 m launch and a 200 m ship:
//!
//! | penalty | grows with |
//! |---|---|
//! | displacement | relative error beyond `hold_displacement_tol` |
//! | stability | shortfall of `GM = KM - KG` below `min_gm`, relative to `min_gm` |
//! | `LCB` band | distance of `LCB` (fraction of `LWL` from aft) outside `lcb_band` |
//! | validity | each resistance-method warning the baseline did not already have |
//!
//! A candidate the geometry or the method cannot evaluate (no waterline, no
//! planing equilibrium) costs 100 times the baseline resistance. Quadratic
//! penalties alone let the minimiser sit slightly inside a constraint, so the
//! answer is chosen differently from the search: among every evaluated hull
//! that satisfies the hard constraints (displacement, `GM`, `LCB` band), the one
//! with the lowest objective. The baseline is always evaluated, so when it is
//! itself feasible the result is never worse than the starting hull.
//!
//! # What it does not claim
//!
//! It is a local search over a few smooth shape parameters, not a hull-form
//! optimiser: it cannot change the sections, the ends or the topology, and the
//! Lackenby shift has limited authority (see [`crate::lackenby`]). Holtrop–
//! Mennen is a regression, so a minimum it finds is a minimum of the
//! regression; the report lists the validity warnings of the final hull.

use std::collections::HashMap;

use serde::Serialize;
use serde_json::{Map, Value};

use crate::hydro::{Hull, HydroOptions};
use crate::lackenby::{self, LackenbyTarget};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::planing;
use crate::resistance::{self, HoltropInput, GRAVITY, KNOT};
use crate::stability::solve_draft;
use crate::subdiv::SubdivisionOptions;
use crate::transform;

// ---------------------------------------------------------------------------
// Nelder–Mead
// ---------------------------------------------------------------------------

/// Settings for [`nelder_mead`].
#[derive(Debug, Clone)]
pub struct NmOptions {
    /// Lower bounds, one per variable; empty means unbounded below.
    pub lower: Vec<f64>,
    /// Upper bounds, one per variable; empty means unbounded above.
    pub upper: Vec<f64>,
    /// Hard cap on calls to the objective.
    pub max_evaluations: usize,
    /// Converged when every vertex is within `x_tol` of the best one, measured
    /// in units of the initial step of that variable.
    pub x_tol: f64,
    /// ... and the worst vertex is within `f_tol` (absolute) of the best one.
    pub f_tol: f64,
}

impl Default for NmOptions {
    fn default() -> Self {
        Self {
            lower: Vec::new(),
            upper: Vec::new(),
            max_evaluations: 200,
            x_tol: 1e-6,
            f_tol: 1e-9,
        }
    }
}

/// Why [`nelder_mead`] stopped.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum NmStop {
    /// Simplex size and function spread are both under their tolerances.
    Converged,
    /// The evaluation cap was reached first.
    MaxEvaluations,
}

/// One call of the objective.
#[derive(Debug, Clone, Serialize)]
pub struct TrajectoryPoint {
    /// 1-based index of the evaluation.
    pub eval: usize,
    /// The (clamped) point that was evaluated.
    pub x: Vec<f64>,
    pub f: f64,
}

/// The outcome of [`nelder_mead`].
#[derive(Debug, Clone, Serialize)]
pub struct NmResult {
    /// The best point evaluated.
    pub x: Vec<f64>,
    pub f: f64,
    pub iterations: usize,
    pub evaluations: usize,
    pub stop: NmStop,
    /// Every evaluation, in order.
    pub trajectory: Vec<TrajectoryPoint>,
}

struct Evaluator<F> {
    f: F,
    lower: Vec<f64>,
    upper: Vec<f64>,
    max: usize,
    trajectory: Vec<TrajectoryPoint>,
}

impl<F: FnMut(&[f64]) -> f64> Evaluator<F> {
    fn exhausted(&self) -> bool {
        self.trajectory.len() >= self.max
    }

    fn clamp(&self, x: &mut [f64]) {
        for (i, v) in x.iter_mut().enumerate() {
            if let Some(lo) = self.lower.get(i) {
                *v = v.max(*lo);
            }
            if let Some(hi) = self.upper.get(i) {
                *v = v.min(*hi);
            }
        }
    }

    /// Clamp `x` into the bounds, evaluate it, and record it. `None` once the
    /// evaluation budget is spent.
    fn eval(&mut self, x: &mut [f64]) -> Option<f64> {
        if self.exhausted() {
            return None;
        }
        self.clamp(x);
        let mut fx = (self.f)(x);
        if !fx.is_finite() {
            fx = 1e300; // keeps the ordering total
        }
        self.trajectory.push(TrajectoryPoint {
            eval: self.trajectory.len() + 1,
            x: x.to_vec(),
            f: fx,
        });
        Some(fx)
    }
}

/// `c + t (x - c)`.
fn along(c: &[f64], x: &[f64], t: f64) -> Vec<f64> {
    c.iter().zip(x).map(|(c, x)| c + t * (x - c)).collect()
}

/// Minimise `f` from `x0` with the Nelder–Mead simplex method.
///
/// `step` gives the initial offset of the simplex along each axis (and the unit
/// in which `x_tol` is measured). Trial points are clamped into
/// `opts.lower`/`opts.upper` before they are evaluated, so the minimiser never
/// asks `f` about a point outside the bounds and a minimum on a bound is
/// reached exactly. Coefficients are the standard ones: reflection 1,
/// expansion 2, contraction 0.5, shrink 0.5.
pub fn nelder_mead<F: FnMut(&[f64]) -> f64>(
    f: F,
    x0: &[f64],
    step: &[f64],
    opts: &NmOptions,
) -> NmResult {
    const ALPHA: f64 = 1.0;
    const GAMMA: f64 = 2.0;
    const RHO: f64 = 0.5;
    const SIGMA: f64 = 0.5;

    let n = x0.len();
    let mut ev = Evaluator {
        f,
        lower: opts.lower.clone(),
        upper: opts.upper.clone(),
        max: opts.max_evaluations.max(1),
        trajectory: Vec::new(),
    };
    let scale: Vec<f64> = (0..n)
        .map(|i| step.get(i).copied().unwrap_or(1.0).abs().max(1e-12))
        .collect();

    let mut stop = NmStop::MaxEvaluations;
    let mut iterations = 0;
    let mut simplex: Vec<(Vec<f64>, f64)> = Vec::with_capacity(n + 1);

    // Initial simplex: x0 and one vertex per axis. A step that a bound turns
    // into no move at all is tried the other way.
    let mut start = x0.to_vec();
    ev.clamp(&mut start);
    'build: {
        let Some(f0) = ev.eval(&mut start) else {
            break 'build;
        };
        simplex.push((start.clone(), f0));
        for i in 0..n {
            let mut v = start.clone();
            v[i] += scale[i] * step.get(i).map_or(1.0, |s| s.signum());
            ev.clamp(&mut v);
            if (v[i] - start[i]).abs() < 1e-15 {
                v[i] = start[i] - scale[i] * step.get(i).map_or(1.0, |s| s.signum());
                ev.clamp(&mut v);
            }
            let Some(fv) = ev.eval(&mut v) else {
                break 'build;
            };
            simplex.push((v, fv));
        }
    }

    if simplex.len() == n + 1 && n > 0 {
        'outer: loop {
            simplex.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal));
            let size = simplex[1..]
                .iter()
                .map(|(v, _)| {
                    (0..n)
                        .map(|i| (v[i] - simplex[0].0[i]).abs() / scale[i])
                        .fold(0.0_f64, f64::max)
                })
                .fold(0.0_f64, f64::max);
            let spread = simplex[n].1 - simplex[0].1;
            if size <= opts.x_tol && spread <= opts.f_tol {
                stop = NmStop::Converged;
                break;
            }
            if ev.exhausted() {
                break;
            }
            iterations += 1;

            let mut c = vec![0.0; n];
            for (v, _) in &simplex[..n] {
                for (ci, vi) in c.iter_mut().zip(v) {
                    *ci += vi / n as f64;
                }
            }
            let (f_best, f_second_worst, f_worst) = (simplex[0].1, simplex[n - 1].1, simplex[n].1);
            let worst = simplex[n].0.clone();

            // Reflection.
            let mut xr = along(&c, &worst, -ALPHA);
            let Some(fr) = ev.eval(&mut xr) else { break };
            if fr >= f_best && fr < f_second_worst {
                simplex[n] = (xr, fr);
                continue;
            }
            if fr < f_best {
                // Expansion.
                let mut xe = along(&c, &xr, GAMMA);
                let Some(fe) = ev.eval(&mut xe) else { break };
                simplex[n] = if fe < fr { (xe, fe) } else { (xr, fr) };
                continue;
            }
            // Contraction: outside when the reflection beat the worst vertex,
            // inside when it did not.
            let (mut xc, outside) = if fr < f_worst {
                (along(&c, &xr, RHO), true)
            } else {
                (along(&c, &worst, RHO), false)
            };
            let Some(fc) = ev.eval(&mut xc) else { break };
            let accepted = if outside { fc <= fr } else { fc < f_worst };
            if accepted {
                simplex[n] = (xc, fc);
                continue;
            }
            // Shrink every vertex towards the best.
            let best = simplex[0].0.clone();
            for slot in simplex.iter_mut().skip(1) {
                let mut v = along(&best, &slot.0, SIGMA);
                let Some(fv) = ev.eval(&mut v) else {
                    break 'outer;
                };
                *slot = (v, fv);
            }
        }
    }

    // The best point *evaluated*, which can be a trial the budget cut short.
    let (x, fx) = ev
        .trajectory
        .iter()
        .min_by(|a, b| a.f.partial_cmp(&b.f).unwrap_or(std::cmp::Ordering::Equal))
        .map(|p| (p.x.clone(), p.f))
        .unwrap_or_else(|| (x0.to_vec(), f64::INFINITY));
    NmResult {
        x,
        f: fx,
        iterations,
        evaluations: ev.trajectory.len(),
        stop,
        trajectory: ev.trajectory,
    }
}

// ---------------------------------------------------------------------------
// The design problem
// ---------------------------------------------------------------------------

/// One of the shape variables.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum DesignVar {
    /// Change in the prismatic coefficient (Lackenby station shift).
    CpDelta,
    /// Change in `LCB`, as a fraction of `LWL` (Lackenby station shift).
    LcbDeltaFrac,
    /// `x` scaled by `s` and `y` by `1/s`, so `L/B` is multiplied by `s²`.
    LbRatioScale,
}

impl DesignVar {
    pub const ALL: [DesignVar; 3] = [Self::CpDelta, Self::LcbDeltaFrac, Self::LbRatioScale];

    pub fn name(self) -> &'static str {
        match self {
            Self::CpDelta => "cp_delta",
            Self::LcbDeltaFrac => "lcb_delta_frac",
            Self::LbRatioScale => "lb_ratio_scale",
        }
    }

    /// `cp`, `lcb`, `lb` or the full names.
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "cp" | "cp_delta" => Some(Self::CpDelta),
            "lcb" | "lcb_delta_frac" | "lcb_delta" => Some(Self::LcbDeltaFrac),
            "lb" | "lb_ratio_scale" | "lb_ratio" => Some(Self::LbRatioScale),
            _ => None,
        }
    }

    /// The value that leaves the hull alone.
    pub fn neutral(self) -> f64 {
        match self {
            Self::LbRatioScale => 1.0,
            _ => 0.0,
        }
    }

    /// Bounds used when none are given: inside what the station shift can
    /// reach on a typical hull, and a ±8 % stretch (±17 % in `L/B`).
    pub fn default_bounds(self) -> (f64, f64) {
        match self {
            Self::CpDelta => (-0.03, 0.03),
            Self::LcbDeltaFrac => (-0.02, 0.02),
            Self::LbRatioScale => (0.92, 1.08),
        }
    }
}

/// A variable and its bounds.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct VarSpec {
    pub var: DesignVar,
    pub lo: f64,
    pub hi: f64,
}

impl VarSpec {
    pub fn new(var: DesignVar) -> Self {
        let (lo, hi) = var.default_bounds();
        Self { var, lo, hi }
    }

    pub fn bounded(var: DesignVar, lo: f64, hi: f64) -> Self {
        Self { var, lo, hi }
    }
}

/// Which resistance method scores a candidate.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Method {
    Holtrop,
    Savitsky,
    /// Savitsky when `Fn_L >= 1` and the baseline hull planes at the speed,
    /// Holtrop–Mennen otherwise.
    Auto,
}

impl Method {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "holtrop" | "holtrop-mennen" | "displacement" => Some(Self::Holtrop),
            "savitsky" | "planing" => Some(Self::Savitsky),
            "auto" => Some(Self::Auto),
            _ => None,
        }
    }
}

/// What to optimise.
#[derive(Debug, Clone)]
pub struct OptimizeSpec {
    /// Design speed in knots.
    pub speed_kn: f64,
    pub method: Method,
    pub vars: Vec<VarSpec>,
    /// Displacement may differ from the baseline by this fraction (default 1 %).
    pub hold_displacement_tol: f64,
    /// Minimum `GM = KM - KG` in metres; needs `kg`.
    pub min_gm: Option<f64>,
    /// `KG` above the keel, metres. Without `min_gm` it only reports `GM`.
    pub kg: Option<f64>,
    /// Allowed `LCB`, as a fraction of `LWL` from the aft end of the waterline
    /// (0.5 is midships).
    pub lcb_band: Option<(f64, f64)>,
    /// Cap on objective evaluations (default 60).
    pub max_evaluations: usize,
    /// Draft the baseline is measured at (default: the options' draft).
    pub draft: Option<f64>,
}

impl OptimizeSpec {
    /// Cp and LCB with their default bounds (L/B is opt-in: it changes the
    /// length, which is usually fixed by berth, rule or cost), Holtrop/Savitsky
    /// chosen automatically, 1 % displacement tolerance, 60 evaluations.
    pub fn new(speed_kn: f64) -> Self {
        Self {
            speed_kn,
            method: Method::Auto,
            vars: [DesignVar::CpDelta, DesignVar::LcbDeltaFrac]
                .iter()
                .map(|v| VarSpec::new(*v))
                .collect(),
            hold_displacement_tol: 0.01,
            min_gm: None,
            kg: None,
            lcb_band: None,
            max_evaluations: 60,
            draft: None,
        }
    }

    /// Variables from a comma-separated list (`cp,lcb,lb`), each with its
    /// default bounds.
    pub fn parse_vars(list: &str) -> Result<Vec<VarSpec>, String> {
        let mut out: Vec<VarSpec> = Vec::new();
        for name in list.split(',').map(str::trim).filter(|s| !s.is_empty()) {
            let var = DesignVar::parse(name)
                .ok_or_else(|| format!("unknown variable `{name}` (use cp, lcb or lb)"))?;
            if out.iter().all(|v| v.var != var) {
                out.push(VarSpec::new(var));
            }
        }
        if out.is_empty() {
            return Err("give at least one variable (cp, lcb, lb)".into());
        }
        Ok(out)
    }

    /// Override bounds from `name=lo:hi` strings (`cp=-0.02:0.02`).
    pub fn apply_bound_strings(vars: &mut [VarSpec], bounds: &[String]) -> Result<(), String> {
        for b in bounds {
            let (name, range) = b
                .split_once('=')
                .ok_or_else(|| format!("bound `{b}` must look like cp=-0.02:0.02"))?;
            let var = DesignVar::parse(name)
                .ok_or_else(|| format!("unknown variable `{}` in bound `{b}`", name.trim()))?;
            let (lo, hi) = range
                .split_once(':')
                .ok_or_else(|| format!("bound `{b}` must look like cp=-0.02:0.02"))?;
            let parse = |s: &str| {
                s.trim()
                    .parse::<f64>()
                    .map_err(|_| format!("bound `{b}`: `{s}` is not a number"))
            };
            let (lo, hi) = (parse(lo)?, parse(hi)?);
            let slot = vars.iter_mut().find(|v| v.var == var).ok_or_else(|| {
                format!(
                    "bound given for `{}`, which is not being optimised",
                    var.name()
                )
            })?;
            slot.lo = lo;
            slot.hi = hi;
        }
        Ok(())
    }

    /// Build a spec from tool parameters (`speed_kn`, `method`, `vars`,
    /// `bounds`, `hold_displacement_tol`, `min_gm`, `kg`, `lcb_band`,
    /// `max_evaluations`, `draft`). One parser for the MCP server and the
    /// app's control channel, so they cannot disagree. `default_kg` stands in
    /// when `kg` is absent.
    pub fn from_params(p: &Map<String, Value>, default_kg: Option<f64>) -> Result<Self, String> {
        let num = |key: &str| -> Result<Option<f64>, String> {
            match p.get(key) {
                None | Some(Value::Null) => Ok(None),
                Some(v) => v
                    .as_f64()
                    .filter(|n| n.is_finite())
                    .map(Some)
                    .ok_or_else(|| format!("`{key}` must be a finite number")),
            }
        };
        let speed = num("speed_kn")?.ok_or("give `speed_kn`")?;
        let mut spec = Self::new(speed);
        if let Some(Value::String(m)) = p.get("method") {
            spec.method = Method::parse(m)
                .ok_or_else(|| format!("`method` must be auto, holtrop or savitsky (got `{m}`)"))?;
        }
        match p.get("vars") {
            None | Some(Value::Null) => {}
            Some(Value::Array(items)) => {
                let mut names = Vec::new();
                for item in items {
                    names.push(
                        item.as_str()
                            .ok_or("`vars` must be an array of names")?
                            .to_string(),
                    );
                }
                spec.vars = Self::parse_vars(&names.join(","))?;
            }
            Some(Value::String(list)) => spec.vars = Self::parse_vars(list)?,
            Some(_) => return Err("`vars` must be an array of names (cp, lcb, lb)".into()),
        }
        match p.get("bounds") {
            None | Some(Value::Null) => {}
            Some(Value::Object(o)) => {
                for (name, range) in o {
                    let var = DesignVar::parse(name)
                        .ok_or_else(|| format!("unknown variable `{name}` in `bounds`"))?;
                    let pair = range
                        .as_array()
                        .filter(|a| a.len() == 2)
                        .and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)))
                        .filter(|(lo, hi)| lo.is_finite() && hi.is_finite())
                        .ok_or_else(|| format!("`bounds.{name}` must be [lo, hi]"))?;
                    let slot = spec.vars.iter_mut().find(|v| v.var == var).ok_or_else(|| {
                        format!(
                            "`bounds.{name}` given, but `{}` is not being optimised",
                            var.name()
                        )
                    })?;
                    slot.lo = pair.0;
                    slot.hi = pair.1;
                }
            }
            Some(_) => return Err("`bounds` must be an object like {\"cp\": [-0.02, 0.02]}".into()),
        }
        if let Some(t) = num("hold_displacement_tol")? {
            spec.hold_displacement_tol = t;
        }
        spec.min_gm = num("min_gm")?;
        spec.kg = num("kg")?.or(default_kg);
        match p.get("lcb_band") {
            None | Some(Value::Null) => {}
            Some(v) => {
                let pair = v
                    .as_array()
                    .filter(|a| a.len() == 2)
                    .and_then(|a| Some((a[0].as_f64()?, a[1].as_f64()?)))
                    .ok_or("`lcb_band` must be [lo, hi] as fractions of LWL")?;
                spec.lcb_band = Some(pair);
            }
        }
        if let Some(n) = num("max_evaluations")? {
            if n < 1.0 || n.fract() != 0.0 {
                return Err("`max_evaluations` must be a positive whole number".into());
            }
            spec.max_evaluations = n as usize;
        }
        spec.draft = num("draft")?;
        Ok(spec)
    }

    fn validate(&self) -> Result<(), String> {
        if !(self.speed_kn.is_finite() && self.speed_kn > 0.0) {
            return Err(format!(
                "optimize: a speed of {} kn is not positive",
                self.speed_kn
            ));
        }
        if self.vars.is_empty() {
            return Err("optimize: give at least one variable".into());
        }
        for (i, v) in self.vars.iter().enumerate() {
            if self.vars[..i].iter().any(|u| u.var == v.var) {
                return Err(format!("optimize: `{}` listed twice", v.var.name()));
            }
            if !(v.lo.is_finite() && v.hi.is_finite() && v.lo < v.hi) {
                return Err(format!(
                    "optimize: the bounds of `{}` must satisfy lo < hi (got {}..{})",
                    v.var.name(),
                    v.lo,
                    v.hi
                ));
            }
            let n = v.var.neutral();
            if !(v.lo..=v.hi).contains(&n) {
                return Err(format!(
                    "optimize: the bounds of `{}` must contain {n} (the unchanged hull)",
                    v.var.name()
                ));
            }
            if v.var == DesignVar::LbRatioScale && v.lo <= 0.0 {
                return Err("optimize: `lb_ratio_scale` bounds must be positive".into());
            }
        }
        if !(self.hold_displacement_tol.is_finite() && self.hold_displacement_tol >= 0.0) {
            return Err("optimize: `hold_displacement_tol` must be zero or positive".into());
        }
        if let Some(gm) = self.min_gm {
            if !gm.is_finite() {
                return Err("optimize: `min_gm` must be finite".into());
            }
            if self.kg.is_none() {
                return Err("optimize: `min_gm` needs `kg` (GM = KM - KG)".into());
            }
        }
        if self.kg.is_some_and(|k| !k.is_finite()) {
            return Err("optimize: `kg` must be finite".into());
        }
        if let Some((lo, hi)) = self.lcb_band {
            if !(lo.is_finite() && hi.is_finite() && 0.0 < lo && lo < hi && hi < 1.0) {
                return Err(format!(
                    "optimize: `lcb_band` must satisfy 0 < lo < hi < 1 (got {lo}..{hi})"
                ));
            }
        }
        if self.max_evaluations < self.vars.len() + 2 {
            return Err(format!(
                "optimize: `max_evaluations` must be at least {} for {} variable(s)",
                self.vars.len() + 2,
                self.vars.len()
            ));
        }
        if self.draft.is_some_and(|d| !(d.is_finite() && d > 0.0)) {
            return Err("optimize: `draft` must be positive".into());
        }
        Ok(())
    }
}

/// A hull's measured state at the design speed.
#[derive(Debug, Clone, Serialize)]
pub struct DesignState {
    /// Total still-water resistance, kN.
    pub rt_kn: f64,
    /// Effective power `R_T · V`, kW.
    pub pe_kw: f64,
    pub draft_m: f64,
    pub volume_m3: f64,
    /// Displacement, tonnes.
    pub displacement_t: f64,
    pub cp: f64,
    pub cb: f64,
    /// `LCB` as a percentage of `LWL`, positive forward of midships.
    pub lcb_pct: f64,
    /// `LCB` as a fraction of `LWL` from the aft end of the waterline.
    pub lcb_frac: f64,
    pub lwl_m: f64,
    pub bwl_m: f64,
    pub lb: f64,
    pub bt: f64,
    pub froude: f64,
    pub km_m: f64,
    /// `KM - KG`; `None` without a `kg`.
    pub gm_m: Option<f64>,
    /// Running trim at equilibrium (Savitsky only), degrees.
    pub planing_trim_deg: Option<f64>,
}

/// One optimised variable.
#[derive(Debug, Clone, Serialize)]
pub struct VariableReport {
    pub name: &'static str,
    pub lo: f64,
    pub hi: f64,
    pub start: f64,
    pub value: f64,
    /// The optimum sits on a bound: a wider range might do better.
    pub at_bound: bool,
}

/// The outcome of [`optimize_hull`].
#[derive(Debug, Clone, Serialize)]
pub struct OptimizeReport {
    pub speed_kn: f64,
    pub method_requested: Method,
    /// The method actually used (never `auto`).
    pub method: Method,
    pub baseline: DesignState,
    pub optimised: DesignState,
    /// `(optimised - baseline) / baseline` of `R_T`, percent (negative is better).
    pub rt_change_pct: f64,
    pub variables: Vec<VariableReport>,
    pub iterations: usize,
    /// Objective evaluations (including repeats answered from the cache).
    pub evaluations: usize,
    /// Distinct design vectors that were actually built and measured.
    pub unique_evaluations: usize,
    pub converged: bool,
    pub convergence_reason: String,
    /// Relative displacement error of the optimised hull, percent.
    pub displacement_error_pct: f64,
    /// The hard constraints (displacement, `GM`, `LCB` band) hold.
    pub constraints_met: bool,
    pub warnings: Vec<String>,
    /// Every evaluation: index, design vector (in `variables` order), objective.
    pub trajectory: Vec<TrajectoryPoint>,
}

// Penalty weights, each multiplied by the baseline R_T (see the module docs).
const DISPLACEMENT_WEIGHT: f64 = 1e5;
const GM_WEIGHT: f64 = 100.0;
const LCB_WEIGHT: f64 = 1e4;
const WARNING_WEIGHT: f64 = 0.02;
const FAILURE_FACTOR: f64 = 100.0;
/// The draft may move to between these multiples of the starting draft.
const DRAFT_RANGE: (f64, f64) = (0.7, 1.3);

struct Ctx<'a> {
    base: &'a Surface,
    subdiv: SubdivisionOptions,
    opts: HydroOptions,
    spec: &'a OptimizeSpec,
    speed_m_s: f64,
    draft0: f64,
    /// Raw displaced volume of the baseline (what the draft solver targets).
    target_volume: f64,
    /// Baseline volume as `Hydrostatics::volume_m3` reports it.
    base_volume_m3: f64,
    lackenby_cp0: f64,
    lackenby_lcb0: f64,
    rt0_kn: f64,
    warnings0: usize,
}

struct Measured {
    state: DesignState,
    warnings: Vec<String>,
}

fn measure(
    base_ctx: &MeasureCtx,
    cand: &Surface,
    method: Method,
    fixed_draft: Option<f64>,
) -> Result<Measured, String> {
    let hull = Hull::new(cand, base_ctx.subdiv);
    let mut o = base_ctx.opts.clone();
    o.mainframe_x = HydroOptions::from_surface(cand, base_ctx.subdiv).mainframe_x;
    o.trim = 0.0;
    o.heel_deg = 0.0;
    o.stations = 0;
    let draft = match fixed_draft {
        Some(d) => d,
        None => {
            let (lo, hi) = (
                DRAFT_RANGE.0 * base_ctx.draft0,
                DRAFT_RANGE.1 * base_ctx.draft0,
            );
            solve_draft(&hull, base_ctx.target_volume, 0.0, 0.0)
                .unwrap_or(hi)
                .clamp(lo, hi)
        }
    };
    o.draft = draft;
    let h = hull.hydrostatics(&o);
    let input = HoltropInput::measure(&hull, &o)
        .ok_or_else(|| "the hull has no waterline at this draft".to_string())?;
    let v = base_ctx.speed_m_s;
    let mut warnings = input.warnings();
    let (rt_n, pe_w, froude, trim) = match method {
        Method::Savitsky => {
            let setup = planing::setup(cand, base_ctx.subdiv, &o, None, None, None)?;
            if setup.deadrise_measured.is_none() {
                warnings.push(
                    "deadrise could not be measured at the transom; 12 degrees assumed".into(),
                );
            }
            let p = planing::planing_point(&setup.hull, v)
                .ok_or_else(|| "the hull does not plane at this speed".to_string())?;
            (
                p.resistance_n,
                p.pe_w,
                v / (GRAVITY * input.length_wl).sqrt(),
                Some(p.trim_deg),
            )
        }
        _ => {
            let r = resistance::holtrop(&input, v);
            if r.fn_ > 0.55 {
                warnings.push(format!(
                    "Fn {:.2} is above the 0.55 Holtrop-Mennen was fitted to",
                    r.fn_
                ));
            }
            (r.rt_n, r.pe_w, r.fn_, None)
        }
    };
    if !(rt_n.is_finite() && rt_n > 0.0) {
        return Err("the resistance estimate is not a positive number".into());
    }
    Ok(Measured {
        state: DesignState {
            rt_kn: rt_n / 1000.0,
            pe_kw: pe_w / 1000.0,
            draft_m: draft,
            volume_m3: h.volume_m3,
            displacement_t: h.displacement_t,
            cp: input.cp,
            cb: input.cb,
            lcb_pct: input.lcb_pct,
            lcb_frac: 0.5 + input.lcb_pct / 100.0,
            lwl_m: input.length_wl,
            bwl_m: input.beam_wl,
            lb: input.length_wl / input.beam_wl,
            bt: input.beam_wl / draft,
            froude,
            km_m: h.km_transverse,
            gm_m: base_ctx.kg.map(|kg| h.km_transverse - kg),
            planing_trim_deg: trim,
        },
        warnings,
    })
}

/// The slice of [`Ctx`] a measurement needs.
struct MeasureCtx {
    subdiv: SubdivisionOptions,
    opts: HydroOptions,
    speed_m_s: f64,
    draft0: f64,
    target_volume: f64,
    kg: Option<f64>,
}

struct Judged {
    f: f64,
    feasible: bool,
}

/// Objective value and feasibility of a measured candidate.
fn judge(ctx: &Ctx, m: &Measured) -> Judged {
    let s = &m.state;
    let spec = ctx.spec;
    let rt0 = ctx.rt0_kn;
    let mut penalty = 0.0;
    let mut feasible = true;

    let err = (s.volume_m3 - ctx.base_volume_m3).abs() / ctx.base_volume_m3;
    let excess = (err - spec.hold_displacement_tol).max(0.0);
    penalty += rt0 * DISPLACEMENT_WEIGHT * excess * excess;
    feasible &= excess <= 1e-9;

    if let (Some(min), Some(gm)) = (spec.min_gm, s.gm_m) {
        let short = (min - gm).max(0.0);
        let rel = short / min.abs().max(0.1);
        penalty += rt0 * GM_WEIGHT * rel * rel;
        feasible &= short <= 1e-9;
    }
    if let Some((lo, hi)) = spec.lcb_band {
        let out = (lo - s.lcb_frac).max(s.lcb_frac - hi).max(0.0);
        penalty += rt0 * LCB_WEIGHT * out * out;
        feasible &= out <= 1e-9;
    }
    let extra = m.warnings.len().saturating_sub(ctx.warnings0);
    penalty += rt0 * WARNING_WEIGHT * extra as f64;
    Judged {
        f: s.rt_kn + penalty,
        feasible,
    }
}

fn decode(vars: &[VarSpec], x: &[f64]) -> (f64, f64, f64) {
    let (mut cp, mut lcb, mut s) = (0.0, 0.0, 1.0);
    for (v, xv) in vars.iter().zip(x) {
        match v.var {
            DesignVar::CpDelta => cp = *xv,
            DesignVar::LcbDeltaFrac => lcb = *xv,
            DesignVar::LbRatioScale => s = *xv,
        }
    }
    (cp, lcb, s)
}

struct Built {
    surface: Surface,
    /// Whether the station shift reached its targets (`None` when not used).
    shift_converged: Option<bool>,
}

/// The hull a design vector describes.
fn build(
    ctx: &Ctx,
    x: &[f64],
    shifted: &mut HashMap<(i64, i64), (Surface, bool)>,
) -> Result<Built, String> {
    let (d_cp, d_lcb, s) = decode(&ctx.spec.vars, x);
    let mut shift_converged = None;
    let mut surface = if d_cp.abs() > 1e-12 || d_lcb.abs() > 1e-12 {
        let key = ((d_cp * 1e6).round() as i64, (d_lcb * 1e6).round() as i64);
        if let std::collections::hash_map::Entry::Vacant(slot) = shifted.entry(key) {
            // Lackenby accepts 0.30..0.95 and +/-10 % LWL; stay inside.
            let target = LackenbyTarget::new(
                (ctx.lackenby_cp0 + d_cp).clamp(0.31, 0.94),
                (ctx.lackenby_lcb0 + 100.0 * d_lcb).clamp(-9.9, 9.9),
            );
            let (out, report) = lackenby::fit(ctx.base, target, &ctx.opts, 1e-3, 6)?;
            slot.insert((out, report.converged));
        }
        let (out, converged) = &shifted[&key];
        shift_converged = Some(*converged);
        out.clone()
    } else {
        ctx.base.clone()
    };
    if (s - 1.0).abs() > 1e-12 {
        transform::apply(&mut surface, Vec3::new(s, 1.0 / s, 1.0), Vec3::ZERO)?;
    }
    Ok(Built {
        surface,
        shift_converged,
    })
}

fn measure_ctx(ctx: &Ctx) -> MeasureCtx {
    MeasureCtx {
        subdiv: ctx.subdiv,
        opts: ctx.opts.clone(),
        speed_m_s: ctx.speed_m_s,
        draft0: ctx.draft0,
        target_volume: ctx.target_volume,
        kg: ctx.spec.kg,
    }
}

struct Record {
    x: Vec<f64>,
    f: f64,
    feasible: bool,
}

/// Reshape `surface` to minimise the resistance at `spec.speed_kn`.
///
/// `opts` supplies the subdivision, water density and (unless `spec.draft`
/// overrides it) the draft the baseline is measured at. The input surface is
/// not modified: the optimised hull is returned, with its design draft set to
/// the draft that holds the original displacement, together with the report.
pub fn optimize_hull(
    surface: &Surface,
    opts: &HydroOptions,
    spec: &OptimizeSpec,
) -> Result<(Surface, OptimizeReport), String> {
    spec.validate()?;
    let subdiv = opts.subdivision;
    let mut base_opts = opts.clone();
    base_opts.trim = 0.0;
    base_opts.heel_deg = 0.0;
    base_opts.stations = 0;
    let draft0 = spec.draft.unwrap_or(opts.draft);
    if !(draft0.is_finite() && draft0 > 0.0) {
        return Err("optimize: the hull needs a positive draft to be measured at".into());
    }
    base_opts.draft = draft0;

    let hull0 = Hull::new(surface, subdiv);
    let target_volume = hull0.displaced_volume(draft0, 0.0, 0.0);
    if !(target_volume.is_finite() && target_volume > 1e-9) {
        return Err(format!(
            "optimize: the hull displaces nothing at T = {draft0:.3} m"
        ));
    }
    // The shift's own bookkeeping of Cp and LCB (it measures from the model's
    // aft end, with 21+ stations), so its targets are consistent with it.
    let mut lack_opts = base_opts.clone();
    lack_opts.stations = lack_opts.stations.max(21);
    let h0 = crate::hydro::compute(surface, &lack_opts);
    if h0.lwl <= 1e-9 {
        return Err("optimize: the hull has no waterline length at this draft".into());
    }
    let lackenby_cp0 = h0.prismatic_coefficient;
    let lackenby_lcb0 = (h0.lcb.x - h0.model_min.x) / h0.lwl * 100.0 - 50.0;

    let mut ctx = Ctx {
        base: surface,
        subdiv,
        opts: base_opts.clone(),
        spec,
        speed_m_s: spec.speed_kn * KNOT,
        draft0,
        target_volume,
        base_volume_m3: target_volume * base_opts.appendage_coefficient,
        lackenby_cp0,
        lackenby_lcb0,
        rt0_kn: 1.0,
        warnings0: 0,
    };

    // Baseline, and which method to use.
    let mc = measure_ctx(&ctx);
    let (method, baseline) = match spec.method {
        Method::Holtrop => (
            Method::Holtrop,
            measure(&mc, surface, Method::Holtrop, Some(draft0))
                .map_err(|e| format!("optimize: baseline: {e}"))?,
        ),
        Method::Savitsky => (
            Method::Savitsky,
            measure(&mc, surface, Method::Savitsky, Some(draft0))
                .map_err(|e| format!("optimize: Savitsky baseline: {e}"))?,
        ),
        Method::Auto => {
            let hol = measure(&mc, surface, Method::Holtrop, Some(draft0))
                .map_err(|e| format!("optimize: baseline: {e}"))?;
            if hol.state.froude >= 1.0 {
                match measure(&mc, surface, Method::Savitsky, Some(draft0)) {
                    Ok(sav) => (Method::Savitsky, sav),
                    Err(_) => (Method::Holtrop, hol),
                }
            } else {
                (Method::Holtrop, hol)
            }
        }
    };
    ctx.rt0_kn = baseline.state.rt_kn;
    ctx.warnings0 = baseline.warnings.len();

    // The search.
    let x0: Vec<f64> = spec.vars.iter().map(|v| v.var.neutral()).collect();
    let step: Vec<f64> = spec.vars.iter().map(|v| 0.125 * (v.hi - v.lo)).collect();
    let nm_opts = NmOptions {
        lower: spec.vars.iter().map(|v| v.lo).collect(),
        upper: spec.vars.iter().map(|v| v.hi).collect(),
        max_evaluations: spec.max_evaluations,
        x_tol: 0.02,
        f_tol: 2e-4 * ctx.rt0_kn,
    };
    let mut shifted: HashMap<(i64, i64), (Surface, bool)> = HashMap::new();
    let mut seen: HashMap<Vec<i64>, usize> = HashMap::new();
    let mut records: Vec<Record> = Vec::new();
    let failure = ctx.rt0_kn * FAILURE_FACTOR;
    let result = nelder_mead(
        |x: &[f64]| {
            let key: Vec<i64> = x.iter().map(|v| (v * 1e6).round() as i64).collect();
            if let Some(&i) = seen.get(&key) {
                return records[i].f;
            }
            let outcome = build(&ctx, x, &mut shifted)
                .and_then(|b| measure(&mc, &b.surface, method, None))
                .map(|m| judge(&ctx, &m));
            let (f, feasible) = match outcome {
                Ok(j) => (j.f, j.feasible),
                Err(_) => (failure, false),
            };
            seen.insert(key, records.len());
            records.push(Record {
                x: x.to_vec(),
                f,
                feasible,
            });
            f
        },
        &x0,
        &step,
        &nm_opts,
    );

    // The answer: the cheapest hull that meets the hard constraints, else the
    // cheapest overall (and say so).
    let cheapest = |only_feasible: bool| {
        records
            .iter()
            .filter(|r| r.feasible || !only_feasible)
            .min_by(|a, b| a.f.partial_cmp(&b.f).unwrap_or(std::cmp::Ordering::Equal))
    };
    let chosen = cheapest(true)
        .or_else(|| cheapest(false))
        .ok_or("optimize: nothing could be evaluated")?;
    let best_x = chosen.x.clone();

    let built = build(&ctx, &best_x, &mut shifted)?;
    let mut out = built.surface;
    let finish = measure(&mc, &out, method, None)?;
    let judged = judge(&ctx, &finish);
    out.settings.draft = finish.state.draft_m;

    let disp_err = (finish.state.volume_m3 - ctx.base_volume_m3).abs() / ctx.base_volume_m3;
    let mut warnings: Vec<String> = finish.warnings.clone();
    if spec.min_gm.is_none() {
        warnings.push(
            "no stability constraint: give kg and min_gm to hold GM while the hull is reshaped"
                .into(),
        );
    }
    if !judged.feasible {
        warnings.push(
            "no evaluated hull met every constraint; this is the least-bad one, check displacement, GM and LCB"
                .into(),
        );
    }
    if built.shift_converged == Some(false) {
        warnings.push(
            "the station shift did not reach its Cp/LCB targets (outside its authority); the numbers are for the hull that was built"
                .into(),
        );
    }
    if baseline.warnings.len() > finish.warnings.len() {
        warnings.push(format!(
            "the baseline had {} validity warning(s); the optimised hull has {}",
            baseline.warnings.len(),
            finish.warnings.len()
        ));
    }
    let variables: Vec<VariableReport> = spec
        .vars
        .iter()
        .zip(&best_x)
        .map(|(v, x)| {
            let at_bound = (x - v.lo).abs() < 1e-9 || (x - v.hi).abs() < 1e-9;
            VariableReport {
                name: v.var.name(),
                lo: v.lo,
                hi: v.hi,
                start: v.var.neutral(),
                value: *x,
                at_bound,
            }
        })
        .collect();
    if variables.iter().any(|v| v.at_bound) {
        let names: Vec<&str> = variables
            .iter()
            .filter(|v| v.at_bound)
            .map(|v| v.name)
            .collect();
        warnings.push(format!(
            "{} ended on a bound; a wider range may find a lower resistance",
            names.join(", ")
        ));
    }
    let converged = result.stop == NmStop::Converged;
    let report = OptimizeReport {
        speed_kn: spec.speed_kn,
        method_requested: spec.method,
        method,
        rt_change_pct: (finish.state.rt_kn - baseline.state.rt_kn) / baseline.state.rt_kn * 100.0,
        baseline: baseline.state,
        optimised: finish.state,
        variables,
        iterations: result.iterations,
        evaluations: result.evaluations,
        unique_evaluations: records.len(),
        converged,
        convergence_reason: if converged {
            "simplex size and objective spread under tolerance".to_string()
        } else {
            format!("evaluation limit of {} reached", spec.max_evaluations)
        },
        displacement_error_pct: disp_err * 100.0,
        constraints_met: judged.feasible,
        warnings,
        trajectory: result.trajectory,
    };
    Ok((out, report))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;
    use crate::fef;
    use crate::subdiv::SubdivisionMode;

    fn rosenbrock(x: &[f64]) -> f64 {
        (1.0 - x[0]).powi(2) + 100.0 * (x[1] - x[0] * x[0]).powi(2)
    }

    #[test]
    fn nelder_mead_finds_the_rosenbrock_minimum() {
        let opts = NmOptions {
            max_evaluations: 2000,
            x_tol: 1e-9,
            f_tol: 1e-14,
            ..NmOptions::default()
        };
        let r = nelder_mead(rosenbrock, &[-1.2, 1.0], &[0.5, 0.5], &opts);
        assert!(
            (r.x[0] - 1.0).abs() < 1e-3 && (r.x[1] - 1.0).abs() < 1e-3,
            "{:?} f {}",
            r.x,
            r.f
        );
        assert_eq!(r.stop, NmStop::Converged, "{r:?}");
        assert!(r.f < 1e-6, "f {}", r.f);
        assert_eq!(r.evaluations, r.trajectory.len());
        assert!(r.iterations > 10 && r.evaluations >= r.iterations);
    }

    #[test]
    fn a_bounded_quadratic_converges_to_the_bound() {
        // The unconstrained minimum, 3, is outside [0, 1]: the answer is the bound.
        let opts = NmOptions {
            lower: vec![0.0],
            upper: vec![1.0],
            max_evaluations: 500,
            x_tol: 1e-9,
            f_tol: 1e-12,
        };
        let r = nelder_mead(|x| (x[0] - 3.0).powi(2), &[0.2], &[0.1], &opts);
        assert!((r.x[0] - 1.0).abs() < 1e-9, "{:?}", r.x);
        assert!((r.f - 4.0).abs() < 1e-9, "f {}", r.f);
        // Never asked about a point outside the bounds.
        assert!(r.trajectory.iter().all(|p| (0.0..=1.0).contains(&p.x[0])));
    }

    #[test]
    fn the_evaluation_cap_ends_the_run() {
        let opts = NmOptions {
            max_evaluations: 10,
            x_tol: 1e-12,
            f_tol: 1e-14,
            ..NmOptions::default()
        };
        let r = nelder_mead(rosenbrock, &[-1.2, 1.0], &[0.5, 0.5], &opts);
        assert_eq!(r.evaluations, 10);
        assert_eq!(r.stop, NmStop::MaxEvaluations);
    }

    #[test]
    fn spec_parsing_and_validation() {
        let vars = OptimizeSpec::parse_vars("cp, lb").expect("parses");
        assert_eq!(vars.len(), 2);
        assert!(OptimizeSpec::parse_vars("cp,wibble").is_err());
        let mut v = OptimizeSpec::parse_vars("cp").expect("parses");
        OptimizeSpec::apply_bound_strings(&mut v, &["cp=-0.01:0.02".into()]).expect("bounds");
        assert_eq!((v[0].lo, v[0].hi), (-0.01, 0.02));
        assert!(OptimizeSpec::apply_bound_strings(&mut v, &["lb=0.9:1.1".into()]).is_err());
        let mut spec = OptimizeSpec::new(8.0);
        spec.min_gm = Some(0.5);
        assert!(spec.validate().unwrap_err().contains("kg"));
        spec.kg = Some(1.0);
        assert!(spec.validate().is_ok());
        spec.vars[0].lo = 0.01; // the unchanged hull is no longer reachable
        assert!(spec.validate().is_err());
        let p: Map<String, Value> = serde_json::from_str(
            r#"{"speed_kn": 8, "vars": ["cp", "lb"], "bounds": {"lb": [0.95, 1.05]}}"#,
        )
        .expect("json");
        let spec = OptimizeSpec::from_params(&p, Some(1.5)).expect("params");
        assert_eq!(spec.vars.len(), 2);
        assert_eq!(spec.kg, Some(1.5));
        assert_eq!(spec.vars[1].hi, 1.05);
    }

    fn demo_setup() -> (Surface, HydroOptions) {
        let s = demo::demo_hull(false);
        let o = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: SubdivisionMode::Loop,
            },
        );
        (s, o)
    }

    #[test]
    fn the_demo_hull_gets_cheaper_at_8_knots_and_the_report_is_honest() {
        let (s, o) = demo_setup();
        let before = fef::write(&s);
        let mut spec = OptimizeSpec::new(8.0);
        spec.kg = Some(1.0);
        spec.min_gm = Some(1.0);
        spec.max_evaluations = 24;
        let (out, rep) = optimize_hull(&s, &o, &spec).expect("optimises");

        // The optimiser works on a copy.
        assert_eq!(fef::write(&s), before, "the input surface was modified");

        assert!(
            rep.optimised.rt_kn <= rep.baseline.rt_kn + 1e-12,
            "{rep:#?}"
        );
        assert!(rep.rt_change_pct <= 1e-9, "{}", rep.rt_change_pct);
        assert!(
            rep.displacement_error_pct <= 100.0 * spec.hold_displacement_tol + 1e-6,
            "{rep:#?}"
        );
        assert!(rep.constraints_met, "{rep:#?}");
        assert!(rep.optimised.gm_m.expect("gm") >= 1.0 - 1e-9, "{rep:#?}");
        assert!(rep.evaluations <= 24 && rep.evaluations == rep.trajectory.len());
        assert_eq!(
            rep.method,
            Method::Holtrop,
            "8 kn on 12 m is a displacement speed"
        );
        assert_eq!(
            rep.variables.len(),
            2,
            "default is Cp and LCB; L/B is opt-in"
        );

        // Re-measure the returned surface from scratch with the same public
        // tools a user would reach for.
        let o2 = HydroOptions::from_surface(&out, o.subdivision);
        assert!(
            (o2.draft - rep.optimised.draft_m).abs() < 1e-12,
            "design draft was not carried"
        );
        let hull = Hull::new(&out, o.subdivision);
        let h = hull.hydrostatics(&o2);
        let input = HoltropInput::measure(&hull, &o2).expect("measures");
        let r = resistance::holtrop(&input, 8.0 * KNOT);
        let opt = &rep.optimised;
        assert!(
            (r.rt_n / 1000.0 - opt.rt_kn).abs() < 1e-9 * opt.rt_kn.max(1.0),
            "{} vs {}",
            r.rt_n / 1000.0,
            opt.rt_kn
        );
        assert!((r.pe_w / 1000.0 - opt.pe_kw).abs() < 1e-9 * opt.pe_kw.max(1.0));
        assert!(
            (h.volume_m3 - opt.volume_m3).abs() < 1e-9,
            "{} vs {}",
            h.volume_m3,
            opt.volume_m3
        );
        assert!((h.displacement_t - opt.displacement_t).abs() < 1e-9);
        assert!((input.cp - opt.cp).abs() < 1e-12 && (input.cb - opt.cb).abs() < 1e-12);
        assert!((input.lcb_pct - opt.lcb_pct).abs() < 1e-9);
        assert!((input.length_wl / input.beam_wl - opt.lb).abs() < 1e-9);
        assert!((h.km_transverse - 1.0 - opt.gm_m.expect("gm")).abs() < 1e-9);
        // And the baseline is the unchanged hull measured the ordinary way.
        let input0 = resistance::input_for(&s, o.subdivision).expect("baseline");
        let r0 = resistance::holtrop(&input0, 8.0 * KNOT);
        assert!(
            (r0.rt_n / 1000.0 - rep.baseline.rt_kn).abs() < 1e-9,
            "{} vs {}",
            r0.rt_n / 1000.0,
            rep.baseline.rt_kn
        );
        // The displacement is the original one.
        let v0 = Hull::new(&s, o.subdivision).hydrostatics(&o).volume_m3;
        assert!(
            (h.volume_m3 - v0).abs() / v0 <= spec.hold_displacement_tol + 1e-9,
            "{} vs {v0}",
            h.volume_m3
        );
    }

    #[test]
    fn a_single_variable_run_leaves_the_others_alone() {
        let (s, o) = demo_setup();
        let mut spec = OptimizeSpec::new(8.0);
        spec.vars = OptimizeSpec::parse_vars("lb").expect("vars");
        spec.max_evaluations = 12;
        let (_, rep) = optimize_hull(&s, &o, &spec).expect("optimises");
        assert_eq!(rep.variables.len(), 1);
        assert!(
            (rep.optimised.cp - rep.baseline.cp).abs() < 1e-9,
            "Cp moved without being asked"
        );
        assert!((rep.optimised.lcb_pct - rep.baseline.lcb_pct).abs() < 1e-9);
        // L/B x s^2, displacement and draft unchanged by a pure stretch.
        let s_val = rep.variables[0].value;
        assert!(
            (rep.optimised.lb / rep.baseline.lb - s_val * s_val).abs() < 2e-3,
            "{rep:#?}"
        );
        assert!((rep.optimised.draft_m - rep.baseline.draft_m).abs() < 1e-4);
        assert!(rep
            .warnings
            .iter()
            .any(|w| w.contains("no stability constraint")));
    }

    #[test]
    fn bad_specs_are_refused_before_any_work() {
        let (s, o) = demo_setup();
        assert!(optimize_hull(&s, &o, &OptimizeSpec::new(0.0)).is_err());
        let mut spec = OptimizeSpec::new(8.0);
        spec.min_gm = Some(0.5);
        assert!(optimize_hull(&s, &o, &spec).is_err(), "min_gm without kg");
        let mut spec = OptimizeSpec::new(8.0);
        spec.max_evaluations = 2;
        assert!(optimize_hull(&s, &o, &spec).is_err());
    }
}
