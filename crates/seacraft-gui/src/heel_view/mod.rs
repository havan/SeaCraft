//! The Heel view in the Analysis drawer: the real hull heeled, water level, with the waterline,
//! the centres of buoyancy and gravity, their arrows and the righting lever between them, and
//! trial weights placed with the mouse.
//!
//! The numbers are the GZ curve's: the same solver and lever as the cross curves
//! (`seacraft_core::heel`), always free to trim. The panel never changes the model or the app: it
//! edits its own view state ([`HeelViewState`]: the start, the trial weights, the heel shown) and
//! returns what the user asked for as a [`HeelViewAction`], which `App::heel_view_drawer` applies.
//! Trial weights are session state; "Add to this condition" is the only way one reaches the model.

use std::sync::Arc;

use crate::analysis_views::AnalysisView;
use crate::theme;
use eframe::egui;
use seacraft_core::heel::{self, Equilibrium, HeelState, MassCondition};
use seacraft_core::hydro::{Hull, HydroOptions};
use seacraft_core::i18n::{Key, Locale, Tr};
use seacraft_core::loading::{LoadingCondition, WeightItem};
use seacraft_core::math::{Plane, Vec3};
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_core::Surface;

pub mod draw;
pub mod geometry;

/// What the view starts from, before the trial weights.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum HeelStart {
    /// The bare hull at the dock's draft and KG.
    #[default]
    Dock,
    /// A loading condition of the project, by index.
    Condition(usize),
}

/// Which drawing a point was picked in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Drawing {
    /// Looking forward: the section, heeled.
    BodyPlan,
    /// From starboard, trimmed.
    Profile,
}

/// A weight being placed: the point picked, and the mass being typed.
#[derive(Debug, Clone, PartialEq)]
pub struct PendingWeight {
    /// Model coordinates of the point picked.
    pub at: Vec3,
    /// The drawing it was picked in (where the field opens).
    pub drawing: Drawing,
    /// The mass as typed, tonnes.
    pub text: String,
    /// Which weight is being edited, when it is an existing one (a double click).
    pub editing: Option<usize>,
    /// Why the last Enter was refused.
    pub error: Option<String>,
    /// The field has been given the keyboard (once, when it opens).
    pub focused: bool,
}

impl PendingWeight {
    pub fn new(at: Vec3, drawing: Drawing, editing: Option<usize>, text: String) -> Self {
        Self {
            at,
            drawing,
            text,
            editing,
            error: None,
            focused: false,
        }
    }
}

/// The eased swing back to the equilibrium heel after a release: a picture between static states,
/// not a roll simulation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Settle {
    pub from: f64,
    pub to: f64,
    /// When it started, in the egui clock's seconds.
    pub started: f64,
}

/// The view's own state, kept by the App between frames.
#[derive(Debug, Clone, PartialEq)]
pub struct HeelViewState {
    pub start: HeelStart,
    /// Trial weights, model coordinates; kept when the start changes.
    pub weights: Vec<WeightItem>,
    pub selected: Option<usize>,
    /// The heel shown, degrees, + to starboard.
    pub heel_deg: f64,
    /// Keep the heel where the hand leaves it instead of settling.
    pub hold: bool,
    /// Automatically recompute the sweep and equilibrium when inputs change.
    pub auto_recalculate: bool,
    /// Plot turning moment (kN·m) instead of righting lever (m).
    pub plot_moment: bool,
    /// The x of the section drawn in the body plan; `None` is at the centre of buoyancy.
    pub section: Option<f64>,
    pub entry: Option<PendingWeight>,
    pub anim: Option<Settle>,
    /// What the pointer holds while it drags in a drawing.
    pub grab: Option<Grab>,
    /// The share of the drawer's height the drawings take; the GZ plot has the rest. `None` is
    /// [`DEFAULT_SPLIT`]; the splitter between them sets it.
    pub split: Option<f32>,
}

impl Default for HeelViewState {
    fn default() -> Self {
        Self {
            start: HeelStart::default(),
            weights: Vec::new(),
            selected: None,
            heel_deg: 0.0,
            hold: false,
            auto_recalculate: true,
            plot_moment: false,
            section: None,
            entry: None,
            anim: None,
            grab: None,
            split: None,
        }
    }
}

/// The drawings' share of the drawer's height until the splitter is dragged.
pub const DEFAULT_SPLIT: f32 = 0.6;

/// What a drag in a drawing moves.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Grab {
    /// The heel (a drag in the body plan off the weights).
    Heel,
    /// A trial weight, in that drawing's plane.
    Weight(usize, Drawing),
}

/// What the Heel view asked for, applied by the App after the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum HeelViewAction {
    None,
    /// The state was edited in place (heel, weights, start): the App re-keys the jobs.
    Changed,
    /// The user explicitly asked to recalculate the sweep for the current inputs.
    Recalculate,
    /// The user explicitly chose a heel via the slider, arrow keys, or GZ plot.
    ManualHeel,
    /// The hand left the heel: settle to the equilibrium (unless Hold is on).
    SettleNow,
    /// Append the trial weights to this loading condition.
    AddToCondition(usize),
    Close,
    /// Show another Analysis view in the drawer.
    Open(AnalysisView),
}

/// The start as the job needs it.
#[derive(Debug, Clone)]
pub enum StartInput {
    /// The bare hull at the dock draft: KG above the keel (and whether it is the estimate), LCG
    /// (`None`: the upright LCB).
    Dock {
        kg: f64,
        kg_estimate: bool,
        lcg: Option<f64>,
    },
    /// A loading condition, with the project's windage, still to be evaluated.
    Condition(Box<LoadingCondition>),
}

/// Everything a sweep job needs, cloned off the window thread.
#[derive(Clone)]
pub struct SweepInput {
    pub surface: Surface,
    /// The hull the hydrostatics measured, when it is current; otherwise the job refines one.
    pub hull: Option<Arc<Hull>>,
    pub subdivision: SubdivisionOptions,
    /// The dock's floating condition (draft, density, appendage coefficient).
    pub opts: HydroOptions,
    pub start: StartInput,
    pub weights: Vec<WeightItem>,
}

/// One computed condition: the sweep the curve and the drag preview use, and the equilibrium.
#[derive(Clone)]
pub struct HeelSweepOut {
    /// The start before the trial weights.
    pub start: MassCondition,
    /// The start plus the trial weights.
    pub mass: MassCondition,
    /// [`heel::heel_state`] on [`heel::sweep_heels`], in order (`None` where it does not float).
    pub sweep: Vec<Option<HeelState>>,
    pub equilibrium: Equilibrium,
    /// The free-trim upright GM (`d(lever)/dφ` at 0° including heel–trim coupling through
    /// `I_TL`), when it floats upright.
    pub upright_gm: Option<f64>,
    /// Why the start is doubtful (a loading condition not in equilibrium).
    pub warnings: Vec<String>,
    /// The hull it was measured on.
    pub hull: Arc<Hull>,
    pub opts: HydroOptions,
}

impl HeelSweepOut {
    /// The sweep sample nearest `heel_deg` that floats.
    pub fn nearest(&self, heel_deg: f64) -> Option<&HeelState> {
        self.sweep.iter().flatten().min_by(|a, b| {
            (a.heel_deg - heel_deg)
                .abs()
                .total_cmp(&(b.heel_deg - heel_deg).abs())
        })
    }

    /// The upright state, when it floats.
    pub fn upright(&self) -> Option<&HeelState> {
        self.sweep.iter().flatten().find(|s| s.heel_deg == 0.0)
    }
}

/// Remove trial weight `i`. The selection, the mass field and a drag stay on the weights they
/// were on (their indices shift down past `i`); one that was on `i` itself ends.
pub fn remove_weight(state: &mut HeelViewState, i: usize) {
    if i >= state.weights.len() {
        return;
    }
    state.weights.remove(i);
    let shift = |j: usize| match j.cmp(&i) {
        std::cmp::Ordering::Less => Some(j),
        std::cmp::Ordering::Equal => None,
        std::cmp::Ordering::Greater => Some(j - 1),
    };
    state.selected = state.selected.and_then(shift);
    if let Some(j) = state.entry.as_ref().and_then(|e| e.editing) {
        match shift(j) {
            Some(k) => {
                if let Some(e) = state.entry.as_mut() {
                    e.editing = Some(k);
                }
            }
            None => state.entry = None,
        }
    }
    if let Some(Grab::Weight(j, d)) = state.grab {
        state.grab = shift(j).map(|k| Grab::Weight(k, d));
    }
}

/// What the hull does when let go at a heel: a reading of the lever curve, not a roll.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum LetGo {
    /// Nothing turns it there: its lever is zero, or the heel is outside the sweep.
    Stays,
    /// It turns until its lever changes sign, a stable equilibrium, at this heel.
    Settles(f64),
    /// Nothing stops it before the end of the sweep: it turns over.
    Capsizes,
}

/// The sweep's (heel, righting lever) pairs in heel order, keeping failed samples as `(NaN, NaN)`
/// breaks so gaps are never bridged.
pub fn lever_curve(out: &HeelSweepOut) -> Vec<(f64, f64)> {
    out.sweep
        .iter()
        .map(|o| match o {
            Some(s) => (s.heel_deg, s.lever_m),
            None => (f64::NAN, f64::NAN),
        })
        .collect()
}

fn finite_point(p: (f64, f64)) -> bool {
    p.0.is_finite() && p.1.is_finite()
}

/// Where the hull goes when let go at `from` on `curve` ((heel, lever) in heel order, with
/// non-finite entries marking gaps): it turns the way its lever pushes it (a positive lever to
/// port) until the lever changes sign. Past the heel where the lever vanishes it turns over
/// rather than coming back; across an unsolved gap it stops (`Stays`).
pub fn let_go(curve: &[(f64, f64)], from: f64) -> LetGo {
    let Some(i) = curve
        .windows(2)
        .position(|w| finite_point(w[0]) && finite_point(w[1]) && w[0].0 <= from && from <= w[1].0)
    else {
        return LetGo::Stays;
    };
    let (a, b) = (curve[i], curve[i + 1]);
    let lever = if b.0 == a.0 {
        a.1
    } else {
        a.1 + (b.1 - a.1) * (from - a.0) / (b.0 - a.0)
    };
    if lever.abs() < 1e-12 {
        return LetGo::Stays;
    }
    let ahead: Vec<(f64, f64)> = if lever > 0.0 {
        curve[..=i].iter().rev().copied().collect()
    } else {
        curve[i + 1..].to_vec()
    };
    let mut prev = (from, lever);
    for (h, l) in ahead {
        if !finite_point((h, l)) {
            return LetGo::Stays;
        }
        if h == prev.0 {
            continue;
        }
        if l == 0.0 || l.signum() != lever.signum() {
            return LetGo::Settles(prev.0 + (h - prev.0) * prev.1 / (prev.1 - l));
        }
        prev = (h, l);
    }
    LetGo::Capsizes
}

fn splice_root(curve: &mut Vec<(f64, f64)>, root: f64) {
    if let Some(i) = curve
        .windows(2)
        .position(|w| finite_point(w[0]) && finite_point(w[1]) && w[0].0 < root && root < w[1].0)
    {
        curve.insert(i + 1, (root, 0.0));
    }
}

/// Where the hull goes when let go at `from` for a computed sweep and equilibrium: splices the
/// solved equilibrium root(s) into the sweep so sub-step loll angles (within ±2°) choose the
/// right side, while keeping the outer sweep check (including capsizes) intact.
pub fn settle_target(out: &HeelSweepOut, from: f64) -> LetGo {
    let mut curve = lever_curve(out);
    match &out.equilibrium {
        Equilibrium::Loll { port, starboard } => {
            if from == 0.0 || (from > 0.0 && from < starboard.heel_deg) {
                return LetGo::Settles(starboard.heel_deg);
            }
            if from < 0.0 && from > port.heel_deg {
                return LetGo::Settles(port.heel_deg);
            }
            splice_root(&mut curve, port.heel_deg);
            splice_root(&mut curve, starboard.heel_deg);
        }
        Equilibrium::Stable { state } => {
            splice_root(&mut curve, state.heel_deg);
        }
        Equilibrium::Capsizes { .. }
        | Equilibrium::CannotFloat
        | Equilibrium::Unresolved { .. } => {}
    }
    let_go(&curve, from)
}

/// The number of stations the drawings cut.
pub const OUTLINE_STATIONS: usize = 25;

/// The hull refined for the drawings, and its station outlines: keyed by the geometry alone, so
/// the hull is drawn even with no KG.
#[derive(Clone)]
pub struct HeelHull {
    pub hull: Arc<Hull>,
    pub outlines: Arc<geometry::HullOutlines>,
    /// The key of the model it was built from: what a section cut from it is cached under.
    pub key: u64,
}

/// Refine the hull (or take the one given) and cut its outlines (the outline job).
pub fn compute_hull(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    hull: Option<Arc<Hull>>,
    key: u64,
) -> HeelHull {
    let hull = hull.unwrap_or_else(|| Arc::new(Hull::new(surface, subdivision)));
    let outlines = Arc::new(geometry::HullOutlines::new(&hull, OUTLINE_STATIONS));
    HeelHull {
        hull,
        outlines,
        key,
    }
}

/// Compute one condition: the start's mass, the sweep, the equilibrium (the sweep job).
pub fn compute_sweep(input: SweepInput) -> Result<HeelSweepOut, String> {
    let SweepInput {
        surface,
        hull,
        subdivision,
        opts,
        start,
        weights,
    } = input;
    let hull = hull.unwrap_or_else(|| Arc::new(Hull::new(&surface, subdivision)));
    let mut warnings = Vec::new();
    let start = match start {
        StartInput::Dock {
            kg,
            kg_estimate,
            lcg,
        } => {
            let mut m = MassCondition::bare(&hull, &opts, kg, lcg).ok_or_else(|| {
                format!(
                    "the hull displaces nothing at the dock draft {:.3} m",
                    opts.draft
                )
            })?;
            m.kg_estimate = kg_estimate;
            m
        }
        StartInput::Condition(cond) => {
            seacraft_core::loading::validate(&cond)
                .map_err(|e| format!("loading condition invalid: {e}"))?;
            let r = seacraft_core::loading::evaluate(&surface, &opts, &cond);
            if !r.equilibrium_found {
                warnings.extend(r.warnings.iter().cloned());
            }
            MassCondition::from_loading(&r, hull.keel)
        }
    };
    let mass = start.with_weights(&weights);
    let sweep = heel::heel_sweep(&hull, &mass, &heel::sweep_heels(), &opts);
    let equilibrium = heel::equilibrium(&hull, &mass, &opts, Some(&sweep));
    let upright_gm = sweep
        .iter()
        .flatten()
        .find(|s| s.heel_deg == 0.0)
        .and_then(|s| heel::upright_free_trim_gm(&hull, &mass, &opts, s));
    Ok(HeelSweepOut {
        start,
        mass,
        sweep,
        equilibrium,
        upright_gm,
        warnings,
        hull,
        opts,
    })
}

/// The exact state at `heel_deg` for a computed condition (the exact job).
pub fn compute_exact(out: &HeelSweepOut, heel_deg: f64) -> Result<HeelState, String> {
    let hint = out.nearest(heel_deg).and_then(|s| s.trim_m).unwrap_or(0.0);
    heel::heel_state(&out.hull, &out.mass, heel_deg, &out.opts, hint)
        .ok_or_else(|| format!("the hull does not float at {heel_deg:.1}°"))
}

/// A key part for the trial weights.
pub fn weights_key(weights: &[WeightItem]) -> u64 {
    let bits: Vec<u64> = weights
        .iter()
        .flat_map(|w| [w.mass_t, w.x, w.y, w.z].map(f64::to_bits))
        .collect();
    crate::jobs::revision_key(&bits)
}

/// Where the KG of the bare hull comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KgNote {
    /// Typed in the dock (or the loading condition's own).
    Typed,
    /// The design requirements' estimate: every readout says "est.".
    Estimate,
    /// No KG: the view says where to set it and draws the hull upright.
    Unset,
}

/// The hint shown with no KG.
pub const NO_KG_HINT: &str = "Set KG in the Hydrostatics dock";

/// Where the two drawings were drawn, and how they map to the model.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DrawnViews {
    pub body: egui::Rect,
    pub body_view: draw::View2,
    pub profile: egui::Rect,
    pub profile_view: draw::View2,
    pub frame: geometry::EarthFrame,
    /// The x of the section the body plan picks on.
    pub section_x: f64,
    /// The splitter between the drawings and the GZ plot (empty until drawn).
    pub splitter: egui::Rect,
}

/// How long the eased swing back to the equilibrium takes, seconds.
pub const SETTLE_SECONDS: f64 = 0.5;

/// Heel per point of horizontal drag in the body plan, degrees.
const DEG_PER_POINT: f64 = 0.25;
/// How near (points) a pick must be to take a weight rather than the water.
const WEIGHT_PICK: f32 = 9.0;

/// What the panel draws, and the view state it edits.
pub struct HeelViewPanel<'a> {
    pub state: &'a mut HeelViewState,
    /// The state drawn: the exact one at the heel shown, or the nearest sweep sample.
    pub shown: Option<&'a HeelState>,
    /// `shown` is the exact state at `state.heel_deg`, not a sweep sample near it.
    pub shown_exact: bool,
    /// The condition's sweep and equilibrium (current, or kept and `stale`).
    pub sweep: Option<&'a HeelSweepOut>,
    pub outlines: Option<&'a geometry::HullOutlines>,
    /// The section drawn strongly, at `state.section` or the shown B.
    pub section: Option<&'a [Vec<Vec3>]>,
    /// The upright waterplane at the dock draft (drawn when there is no condition).
    pub dock_plane: Option<Plane>,
    /// Where the drawings were drawn this frame (the App keeps it for tests).
    pub drawn: &'a mut Option<DrawnViews>,
    pub stale: bool,
    pub busy: bool,
    pub failure: Option<&'a str>,
    pub kg_note: KgNote,
    /// The project's loading conditions, by name.
    pub conditions: &'a [String],
}

/// "Heel 12.4° to starboard", "Heel ≈ 12° to port" (a sweep sample), "Upright".
pub fn heel_words(heel_deg: f64, approx: bool) -> String {
    heel_words_tr(heel_deg, approx, Tr::new(Locale::En))
}

/// Localized form of [`heel_words`].
pub fn heel_words_tr(heel_deg: f64, approx: bool, tr: Tr) -> String {
    let stbd = heel_deg > 0.0;
    match (approx, heel_deg.abs()) {
        (true, a) if a < 0.5 => tr.s(Key::HeelWordsApproxZero).to_string(),
        (true, a) => {
            let deg_s = format!("{a:.0}");
            let key = if stbd {
                Key::HeelWordsApproxStbd
            } else {
                Key::HeelWordsApproxPort
            };
            tr.fmt(key, &[("deg", &deg_s)])
        }
        (false, a) if a < 0.05 => tr.s(Key::HeelWordsUpright).to_string(),
        (false, a) => {
            let deg_s = format!("{a:.1}");
            let key = if stbd {
                Key::HeelWordsExactStbd
            } else {
                Key::HeelWordsExactPort
            };
            tr.fmt(key, &[("deg", &deg_s)])
        }
    }
}

/// A signed length to the millimetre, reading 0.000 (not -0.000) when it rounds to zero.
pub(crate) fn signed(x: f64) -> String {
    crate::panels::format_signed(x, 3)
}

/// The readout of the state drawn, in plain words.
pub fn readout(s: &HeelState, out: &HeelSweepOut, approx: bool) -> String {
    readout_tr(s, out, approx, Tr::new(Locale::En))
}

/// Localized form of [`readout`].
pub fn readout_tr(s: &HeelState, out: &HeelSweepOut, approx: bool, tr: Tr) -> String {
    let keel = out.hull.keel;
    let lever_s = signed(s.lever_m);
    let gz_s = signed(s.gz_m);
    let heel_arm_s = signed(s.heeling_arm_m);
    let mut parts = vec![
        heel_words_tr(s.heel_deg, approx, tr),
        tr.fmt(
            Key::HeelReadoutLineLever,
            &[
                ("lever", &lever_s),
                ("gz", &gz_s),
                ("heel_arm", &heel_arm_s),
            ],
        ),
    ];
    if let (Some(d), Some(t)) = (s.draft, s.trim_m) {
        let draft_s = format!("{d:.3}");
        let trim_s = signed(t);
        parts.push(tr.fmt(Key::HeelReadoutLineDraft, &[("draft", &draft_s)]));
        parts.push(tr.fmt(Key::HeelReadoutLineTrim, &[("trim", &trim_s)]));
    }
    parts.push(format!("Δ {:.2} t", s.displacement_t));
    let est = if out.mass.kg_estimate {
        tr.s(Key::HeelReadoutLineEst)
    } else {
        ""
    };
    parts.push(format!("KG {:.3} m{est}", out.mass.kg(keel)));
    if let Some(gm) = out.upright_gm {
        let gm_s = format!("{gm:.3}");
        parts.push(tr.fmt(Key::HeelReadoutLineUprightGm, &[("gm", &gm_s)]));
    }
    parts.join(" · ")
}

/// Where the condition settles, in plain words.
pub fn equilibrium_line(eq: &Equilibrium, displacement_t: f64) -> String {
    equilibrium_line_tr(eq, displacement_t, Tr::new(Locale::En))
}

/// Localized form of [`equilibrium_line`].
pub fn equilibrium_line_tr(eq: &Equilibrium, displacement_t: f64, tr: Tr) -> String {
    match eq {
        Equilibrium::Stable { state } if state.heel_deg.abs() < 0.05 => match state.trim_m {
            Some(t) => {
                let trim_s = signed(t);
                tr.fmt(Key::HeelEqSettlesUprightTrim, &[("trim", &trim_s)])
            }
            None => tr.s(Key::HeelEqSettlesUpright).to_string(),
        },
        Equilibrium::Stable { state } => {
            let deg_s = format!("{:.1}", state.heel_deg.abs());
            let stbd = state.heel_deg > 0.0;
            match state.trim_m {
                Some(t) => {
                    let trim_s = signed(t);
                    let key = if stbd {
                        Key::HeelEqSettlesStbdTrim
                    } else {
                        Key::HeelEqSettlesPortTrim
                    };
                    tr.fmt(key, &[("deg", &deg_s), ("trim", &trim_s)])
                }
                None => {
                    let key = if stbd {
                        Key::HeelEqSettlesStbd
                    } else {
                        Key::HeelEqSettlesPort
                    };
                    tr.fmt(key, &[("deg", &deg_s)])
                }
            }
        }
        Equilibrium::Loll { port, starboard } => {
            let p = format!("{:.1}", port.heel_deg.abs());
            let s = format!("{:.1}", starboard.heel_deg.abs());
            if p == s {
                tr.fmt(Key::HeelEqLollSym, &[("deg", &s)])
            } else {
                tr.fmt(Key::HeelEqLollAsym, &[("port", &p), ("stbd", &s)])
            }
        }
        Equilibrium::Capsizes { .. } => tr.s(Key::HeelEqCapsizes).to_string(),
        Equilibrium::CannotFloat => {
            let disp_s = format!("{displacement_t:.2}");
            tr.fmt(Key::HeelBannerCannotFloatBody, &[("disp", &disp_s)])
        }
        Equilibrium::Unresolved { reason } => tr.fmt(Key::HeelEqUnresolved, &[("reason", reason)]),
    }
}

/// The caption under the drawings: what the picture is, and is not.
pub const STATIC_CAPTION: &str = "A static picture, free to trim: the swing back is not a roll \
    simulation. Liquids in tanks do not shift; their free surface is in KG.";

/// Height of the two drawings, points.
/// The least height of the drawings and of the GZ plot, points.
const MIN_DRAWING_HEIGHT: f32 = 120.0;
const MIN_PLOT_HEIGHT: f32 = 90.0;
/// Height under the drawings taken by their captions, the static caption, the splitter and the
/// plot's caption and legend, points.
const LAYOUT_OVERHEAD: f32 = 80.0;
/// The body plan's width over its height: a section turned through any heel fits a square, and a
/// little more leaves room for the labels.
const BODY_ASPECT: f32 = 1.15;

/// The heights of the drawings and of the GZ plot in a drawer `total` points high: the drawings
/// take `split` of it, each keeps its least height, and a short drawer scrolls.
pub fn layout_heights(total: f32, split: f32) -> (f32, f32) {
    let usable = (total - LAYOUT_OVERHEAD).max(MIN_DRAWING_HEIGHT + MIN_PLOT_HEIGHT);
    let drawings = (usable * split).clamp(MIN_DRAWING_HEIGHT, usable - MIN_PLOT_HEIGHT);
    (drawings, usable - drawings)
}

/// The body plan's and the profile's widths in a row `width` wide with the drawings `height`
/// high: the body plan as wide as a section needs, the profile the rest.
pub fn layout_widths(width: f32, height: f32, gap: f32) -> (f32, f32) {
    let body = (height * BODY_ASPECT).clamp(120.0, ((width - gap) * 0.5).max(120.0));
    (body, (width - gap - body).max(120.0))
}

/// Combine panel actions so higher-precedence button actions (Recalculate, Close, Open) are
/// never overwritten by secondary state edits in the same frame.
fn combine_action(cur: HeelViewAction, next: HeelViewAction) -> HeelViewAction {
    let rank = |a: HeelViewAction| match a {
        HeelViewAction::None => 0u8,
        HeelViewAction::Changed | HeelViewAction::ManualHeel | HeelViewAction::SettleNow => 1,
        HeelViewAction::AddToCondition(_) | HeelViewAction::Recalculate => 2,
        HeelViewAction::Open(_) | HeelViewAction::Close => 3,
    };
    if next != HeelViewAction::None && rank(next) >= rank(cur) {
        next
    } else {
        cur
    }
}

/// Capsize alert shown across the viewports for the condition and currently displayed heel.
pub fn capsize_alert(out: &HeelSweepOut, shown: Option<&HeelState>) -> draw::CapsizeAlert {
    if matches!(out.equilibrium, Equilibrium::Capsizes { .. }) {
        return draw::CapsizeAlert::ConditionCapsizes;
    }
    if !matches!(
        out.equilibrium,
        Equilibrium::CannotFloat | Equilibrium::Unresolved { .. }
    ) && shown.is_some_and(|s| settle_target(out, s.heel_deg) == LetGo::Capsizes)
    {
        return draw::CapsizeAlert::PastVanishingAngle;
    }
    draw::CapsizeAlert::None
}

/// A single row in the right-hand Heel readout table.
#[derive(Debug, Clone, PartialEq)]
pub struct ReadoutRow {
    pub name: &'static str,
    pub value: String,
    pub unit: &'static str,
    pub colour: egui::Color32,
    pub strong: bool,
    pub hint: &'static str,
}

/// Structured rows for the right-hand readout table: (turning-couple rows, condition rows).
pub fn readout_rows(s: &HeelState, out: &HeelSweepOut) -> (Vec<ReadoutRow>, Vec<ReadoutRow>) {
    readout_rows_tr(s, out, Tr::new(Locale::En))
}

/// Localized form of [`readout_rows`].
pub fn readout_rows_tr(
    s: &HeelState,
    out: &HeelSweepOut,
    tr: Tr,
) -> (Vec<ReadoutRow>, Vec<ReadoutRow>) {
    let force_kn = draw::weight_force_kn(s.displacement_t);
    let moment_knm = draw::righting_moment_knm(s.displacement_t, s.lever_m);
    let righting = draw::is_righting(s.heel_deg, s.lever_m);
    let lever_col = draw::lever_colour(s.heel_deg, s.lever_m);
    let moment_name = if righting || moment_knm.abs() < 0.005 {
        tr.s(Key::HeelReadoutRightingMoment)
    } else if capsize_alert(out, Some(s)) != draw::CapsizeAlert::None {
        tr.s(Key::HeelReadoutCapsizingMoment)
    } else {
        tr.s(Key::HeelReadoutHeelingMoment)
    };
    let couple = vec![
        ReadoutRow {
            name: tr.s(Key::HeelReadoutLever),
            value: signed(s.lever_m),
            unit: "m",
            colour: lever_col,
            strong: true,
            hint: tr.s(Key::HeelReadoutLeverTip),
        },
        ReadoutRow {
            name: tr.s(Key::HeelReadoutGzCl),
            value: signed(s.gz_m),
            unit: "m",
            colour: theme::TEXT_PRIMARY,
            strong: false,
            hint: tr.s(Key::HeelReadoutGzClTip),
        },
        ReadoutRow {
            name: tr.s(Key::HeelReadoutHeelingArm),
            value: signed(s.heeling_arm_m),
            unit: "m",
            colour: theme::TEXT_PRIMARY,
            strong: false,
            hint: tr.s(Key::HeelReadoutHeelingArmTip),
        },
        ReadoutRow {
            name: moment_name,
            value: draw::format_moment_knm(moment_knm),
            unit: "kN·m",
            colour: lever_col,
            strong: true,
            hint: tr.s(Key::HeelReadoutMomentTip),
        },
        ReadoutRow {
            name: tr.s(Key::HeelReadoutBuoyancyWeight),
            value: draw::format_force_kn(force_kn),
            unit: "kN",
            colour: theme::TEXT_PRIMARY,
            strong: false,
            hint: tr.s(Key::HeelReadoutBuoyancyWeightTip),
        },
    ];

    let mut condition = Vec::new();
    if let Some(d) = s.draft {
        condition.push(ReadoutRow {
            name: tr.s(Key::HydroDraft),
            value: format!("{d:.3}"),
            unit: "m",
            colour: theme::TEXT_PRIMARY,
            strong: false,
            hint: tr.s(Key::HeelReadoutDraftTip),
        });
    }
    if let Some(t) = s.trim_m {
        condition.push(ReadoutRow {
            name: tr.s(Key::HydroTrim),
            value: signed(t),
            unit: "m",
            colour: theme::TEXT_PRIMARY,
            strong: false,
            hint: tr.s(Key::HeelReadoutTrimTip),
        });
    }
    let (disp_val, disp_unit) = draw::format_mass_parts(s.displacement_t);
    condition.push(ReadoutRow {
        name: tr.s(Key::HeelReadoutDisp),
        value: disp_val,
        unit: disp_unit,
        colour: theme::TEXT_PRIMARY,
        strong: false,
        hint: tr.s(Key::HeelReadoutDispTip),
    });
    condition.push(ReadoutRow {
        name: "KG",
        value: format!("{:.3}", out.mass.kg(out.hull.keel)),
        unit: if out.mass.kg_estimate {
            tr.s(Key::HeelReadoutKgEstUnit)
        } else {
            "m"
        },
        colour: theme::TEXT_PRIMARY,
        strong: false,
        hint: tr.s(Key::HeelReadoutKgTip),
    });
    if let Some(gm) = out.upright_gm {
        condition.push(ReadoutRow {
            name: tr.s(Key::HeelReadoutUprightGm),
            value: format!("{gm:.3}"),
            unit: "m",
            colour: if gm < 0.0 {
                theme::FAIL
            } else {
                theme::TEXT_PRIMARY
            },
            strong: gm < 0.0,
            hint: tr.s(Key::HeelReadoutUprightGmTip),
        });
    }
    (couple, condition)
}

/// Draw the view: header, inputs, the two drawings, the readout and the GZ curve.
pub fn panel(ui: &mut egui::Ui, p: HeelViewPanel<'_>) -> HeelViewAction {
    panel_tr(ui, p, Tr::new(Locale::En))
}

/// Localized form of [`panel`].
pub fn panel_tr(ui: &mut egui::Ui, p: HeelViewPanel<'_>, tr: Tr) -> HeelViewAction {
    let HeelViewPanel {
        state,
        shown,
        shown_exact,
        sweep,
        outlines,
        section,
        dock_plane,
        drawn,
        stale,
        busy,
        failure,
        kg_note,
        conditions,
    } = p;
    *drawn = None;
    let mut action = header_tr(ui, tr);
    ui.horizontal_wrapped(|ui| {
        let start_name = |s: HeelStart| match s {
            HeelStart::Dock => tr.s(Key::HeelCtlBareHull).to_string(),
            HeelStart::Condition(i) => conditions.get(i).cloned().unwrap_or_else(|| {
                let n_s = (i + 1).to_string();
                tr.fmt(Key::HeelCtlConditionN, &[("n", &n_s)])
            }),
        };
        ui.label(tr.s(Key::HeelCtlStart));
        egui::ComboBox::from_id_salt("heel_start")
            .selected_text(start_name(state.start))
            .show_ui(ui, |ui| {
                let options = std::iter::once(HeelStart::Dock)
                    .chain((0..conditions.len()).map(HeelStart::Condition));
                for option in options {
                    if ui
                        .selectable_label(state.start == option, start_name(option))
                        .clicked()
                        && state.start != option
                    {
                        state.start = option;
                        action = combine_action(action, HeelViewAction::Changed);
                    }
                }
            });
        ui.separator();
        ui.label(tr.s(Key::HeelCtlHeel));
        // One decimal shown, but not rounded: a settle moves the heel in small steps, and a
        // rounding slider would report them as the user's edit and stop it.
        let slider = egui::Slider::new(&mut state.heel_deg, -90.0..=90.0)
            .suffix("°")
            .custom_formatter(|v, _| format!("{v:.1}"));
        if ui
            .add(slider)
            .on_hover_text(tr.s(Key::HeelCtlHeelTip))
            .changed()
        {
            state.anim = None;
            action = combine_action(action, HeelViewAction::ManualHeel);
        }
        ui.checkbox(&mut state.hold, tr.s(Key::HeelCtlHoldHeel))
            .on_hover_text(tr.s(Key::HeelCtlHoldHeelTip));
        ui.separator();
        let at_b = shown.map(|s| s.b.x);
        ui.label(tr.s(Key::HeelCtlSectionX));
        let mut x = state.section.or(at_b).unwrap_or(0.0);
        if ui
            .add(
                egui::DragValue::new(&mut x)
                    .speed(0.05)
                    .suffix(" m")
                    .fixed_decimals(2),
            )
            .on_hover_text(tr.s(Key::HeelCtlSectionXTip))
            .changed()
        {
            state.section = Some(x);
        }
        if ui
            .add_enabled(
                state.section.is_some(),
                egui::Button::new(tr.s(Key::HeelCtlAtB)),
            )
            .on_hover_text(tr.s(Key::HeelCtlAtBTip))
            .clicked()
        {
            state.section = None;
        }
        ui.separator();
        if ui
            .checkbox(&mut state.auto_recalculate, tr.s(Key::HeelCtlAutoRecalc))
            .on_hover_text(tr.s(Key::HeelCtlAutoRecalcTip))
            .changed()
        {
            action = combine_action(action, HeelViewAction::Changed);
        }
        if !state.auto_recalculate {
            let no_kg_dock = kg_note == KgNote::Unset && state.start == HeelStart::Dock;
            let needs_recalc = stale || (sweep.is_none() && failure.is_none() && !no_kg_dock);
            let btn = if needs_recalc && !busy {
                theme::primary_button(tr.s(Key::HeelCtlRecalculate))
            } else {
                egui::Button::new(tr.s(Key::HeelCtlRecalculate))
            };
            if ui
                .add_enabled(!busy, btn)
                .on_hover_text(tr.s(Key::HeelCtlRecalculateTip))
                .clicked()
            {
                action = combine_action(action, HeelViewAction::Recalculate);
            }
        }
        if busy {
            ui.spinner();
        }
    });
    if let Some(msg) = failure {
        ui.colored_label(theme::FAIL, format!("⚠ {msg}"));
    }
    if kg_note == KgNote::Unset && state.start == HeelStart::Dock {
        ui.colored_label(theme::WARN, tr.s(Key::HeelNoKgHint));
        if let (Some(o), Some(plane)) = (outlines, dock_plane) {
            let frame = geometry::EarthFrame::from_plane(&plane);
            let caption = tr.s(Key::HeelCaptionProfileUpright);
            let height = layout_heights(ui.available_height(), DEFAULT_SPLIT).0;
            let (_, views) = drawings(
                ui,
                state,
                o,
                &frame,
                section,
                None,
                caption,
                draw::CapsizeAlert::None,
                false,
                height,
                tr,
            );
            *drawn = Some(views);
        }
        return action;
    }
    let waiting_manual = !state.auto_recalculate && !busy;
    let Some(out) = sweep else {
        if waiting_manual && failure.is_none() {
            ui.colored_label(theme::WARN, tr.s(Key::HeelCtlManualWait));
            if let (Some(o), Some(plane)) = (outlines, dock_plane) {
                let frame = geometry::EarthFrame::from_plane(&plane);
                let caption = tr.s(Key::HeelCaptionProfileUpright);
                let height = layout_heights(ui.available_height(), DEFAULT_SPLIT).0;
                let (_, views) = drawings(
                    ui,
                    state,
                    o,
                    &frame,
                    section,
                    None,
                    caption,
                    draw::CapsizeAlert::None,
                    false,
                    height,
                    tr,
                );
                *drawn = Some(views);
            }
        }
        return action;
    };
    let no_floating = matches!(out.equilibrium, Equilibrium::CannotFloat)
        || (matches!(out.equilibrium, Equilibrium::Unresolved { .. })
            && out.sweep.iter().all(Option::is_none));
    if no_floating {
        // No picture of a hull that does not float, but the weights stay editable: an overweight
        // typo must not be a dead end.
        ui.scope(|ui| {
            if stale {
                stale_chip(ui, failure.is_some(), waiting_manual, tr);
                ui.multiply_opacity(theme::STALE_OPACITY);
            }
            status_banner(ui, out, None, tr);
        });
        let a = weights_table(ui, state, conditions, tr);
        action = combine_action(action, a);
        return action;
    }
    // The drawings and the GZ plot share the drawer's height; the splitter between them moves
    // the share, and a drawer too short for both scrolls.
    let (draw_height, plot_height) =
        layout_heights(ui.available_height(), state.split.unwrap_or(DEFAULT_SPLIT));
    let total = draw_height + plot_height;
    let capsize = capsize_alert(out, shown);
    egui::ScrollArea::vertical()
        .id_salt("heel_view_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The drawings on the left; what they say, and the weights, beside them. Nothing
            // above the drawings comes and goes, so they hold still under a drag that makes the
            // result out of date.
            let width = ui.available_width();
            let side = (0.45 * width).clamp(280.0, 480.0);
            // The row puts item spacing on each side of the 12 pt gap.
            let spacing = 2.0 * ui.spacing().item_spacing.x;
            let draw_width = (width - side - 12.0 - spacing).max(200.0);
            ui.horizontal_top(|ui| {
                ui.vertical(|ui| {
                    ui.set_width(draw_width);
                    if stale {
                        ui.multiply_opacity(theme::STALE_OPACITY);
                    }
                    if let (Some(s), Some(o)) = (shown, outlines) {
                        let frame = geometry::EarthFrame::from_plane(&s.plane);
                        let trim = s.trim_m.map_or(String::new(), |t| {
                            let trim_s = signed(t);
                            tr.fmt(Key::HeelCaptionProfileTrim, &[("trim", &trim_s)])
                        });
                        // Interactive while stale too: moving a weight re-keys the sweep on the
                        // first frame of the drag, and the drag must carry on over the old result.
                        let (a, views) = drawings(
                            ui,
                            state,
                            o,
                            &frame,
                            section,
                            Some(s),
                            &trim,
                            capsize,
                            true,
                            draw_height,
                            tr,
                        );
                        *drawn = Some(views);
                        action = combine_action(action, a);
                    }
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(side);
                    if stale {
                        stale_chip(ui, failure.is_some(), waiting_manual, tr);
                        ui.multiply_opacity(theme::STALE_OPACITY);
                    }
                    status_banner(ui, out, shown, tr);
                    for w in &out.warnings {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!("⚠ {w}")).color(theme::WARN),
                            )
                            .wrap(),
                        );
                    }
                    if shown.is_some_and(|s| s.lid_submerged) {
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(format!(
                                    "⚠ {}",
                                    tr.s(Key::HeelWarnLidSubmerged)
                                ))
                                .color(theme::WARN),
                            )
                            .wrap(),
                        );
                    }
                    if let Some(s) = shown {
                        ui.add_space(3.0);
                        readout_table(ui, s, out, !shown_exact, tr);
                    }
                    ui.add_space(3.0);
                    let a = weights_table(ui, state, conditions, tr);
                    action = combine_action(action, a);
                });
            });
            // The splitter: drag it to give the drawings or the plot more of the drawer.
            let (bar, _) =
                ui.allocate_exact_size(egui::vec2(ui.available_width(), 8.0), egui::Sense::hover());
            let split = ui
                .interact(
                    bar,
                    egui::Id::unique("heel_view_splitter"),
                    egui::Sense::click_and_drag(),
                )
                .on_hover_cursor(egui::CursorIcon::ResizeVertical)
                .on_hover_text(tr.s(Key::HeelSplitterTip));
            let stroke = if split.hovered() || split.dragged() {
                theme::OUTLINE_HOVER
            } else {
                theme::GRID_MAJOR
            };
            ui.painter().hline(
                bar.x_range(),
                bar.center().y,
                egui::Stroke::new(if split.dragged() { 2.0 } else { 1.0 }, stroke),
            );
            if split.dragged() && total > 0.0 {
                let share = state.split.unwrap_or(draw_height / total);
                state.split = Some((share + split.drag_delta().y / total).clamp(0.15, 0.9));
            }
            if split.double_clicked() {
                state.split = None;
            }
            if let Some(views) = drawn.as_mut() {
                views.splitter = bar;
            }
            if stale {
                ui.multiply_opacity(theme::STALE_OPACITY);
            }
            let caption_w = (ui.available_width() - 200.0).max(160.0);
            ui.horizontal_top(|ui| {
                ui.allocate_ui_with_layout(
                    egui::vec2(caption_w, 0.0),
                    egui::Layout::top_down(egui::Align::Min),
                    |ui| {
                        ui.set_width(caption_w);
                        ui.add(
                            egui::Label::new(
                                egui::RichText::new(tr.s(Key::HeelStaticCaption))
                                    .size(theme::SMALL_SIZE)
                                    .color(theme::TEXT_SECONDARY),
                            )
                            .wrap(),
                        );
                    },
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::TOP), |ui| {
                    theme::segmented(ui, |ui| {
                        ui.selectable_value(
                            &mut state.plot_moment,
                            true,
                            tr.s(Key::HeelPlotMomentBtn),
                        )
                        .on_hover_text(tr.s(Key::HeelPlotMomentBtnTip));
                        ui.selectable_value(
                            &mut state.plot_moment,
                            false,
                            tr.s(Key::HeelPlotLeverBtn),
                        )
                        .on_hover_text(tr.s(Key::HeelPlotLeverBtnTip));
                    });
                });
            });
            if let Some(heel) = gz_plot(
                ui,
                out,
                shown.map(|s| s.heel_deg),
                plot_height,
                state.plot_moment,
                tr,
            ) {
                state.heel_deg = (heel * 10.0).round() / 10.0;
                state.anim = None;
                action = combine_action(action, HeelViewAction::ManualHeel);
            }
        });
    if let Some(views) = *drawn {
        for a in [mass_field(ui, state, &views, tr), keys(ui, state, &views)] {
            action = combine_action(action, a);
        }
    }
    action
}

/// Draw a single left-accented status banner for the condition and currently displayed heel:
/// when held past the vanishing angle, leads with the immediate capsize hazard and tucks the
/// small-angle equilibrium inside as secondary context so the two never contradict each other.
fn status_banner(ui: &mut egui::Ui, out: &HeelSweepOut, shown: Option<&HeelState>, tr: Tr) {
    let disp = out.mass.displacement_t;
    if capsize_alert(out, shown) == draw::CapsizeAlert::PastVanishingAngle {
        if let Some(s) = shown {
            let body = if s.heel_deg.abs() < 0.05 {
                tr.s(Key::HeelBannerPastVanishingBodyUpright).to_string()
            } else {
                let deg_s = format!("{:.1}", s.heel_deg.abs());
                let key = if s.heel_deg > 0.0 {
                    Key::HeelBannerPastVanishingBodyStbd
                } else {
                    Key::HeelBannerPastVanishingBodyPort
                };
                tr.fmt(key, &[("deg", &deg_s)])
            };
            let righting_note = match &out.equilibrium {
                Equilibrium::Stable { state } if state.heel_deg.abs() < 0.05 => {
                    Some(tr.s(Key::HeelBannerWithinRangeUpright).to_string())
                }
                Equilibrium::Stable { state } => {
                    let deg_s = format!("{:.1}", state.heel_deg.abs());
                    let key = if state.heel_deg > 0.0 {
                        Key::HeelBannerWithinRangeStbd
                    } else {
                        Key::HeelBannerWithinRangePort
                    };
                    Some(tr.fmt(key, &[("deg", &deg_s)]))
                }
                _ => {
                    let desc = if tr.locale() == Locale::En {
                        out.equilibrium.describe(disp)
                    } else {
                        equilibrium_line_tr(&out.equilibrium, disp, tr)
                    };
                    Some(tr.fmt(Key::HeelBannerWithinRangeOther, &[("desc", &desc)]))
                }
            };
            paint_status_card(
                ui,
                theme::FAIL,
                theme::HEEL_STATUS_FAIL_BG,
                tr.s(Key::HeelBannerPastVanishing),
                &body,
                righting_note.as_deref(),
            );
            return;
        }
    }

    let (accent, fill, title, detail) = match &out.equilibrium {
        Equilibrium::Stable { state } => {
            let headline = if state.heel_deg.abs() < 0.05 {
                tr.s(Key::HeelEqSettlesUpright).to_string()
            } else {
                let deg_s = format!("{:.1}", state.heel_deg.abs());
                let key = if state.heel_deg > 0.0 {
                    Key::HeelEqSettlesStbd
                } else {
                    Key::HeelEqSettlesPort
                };
                tr.fmt(key, &[("deg", &deg_s)])
            };
            let detail = match (state.trim_m, state.draft) {
                (Some(t), Some(d)) => {
                    let trim_s = signed(t);
                    let draft_s = format!("{d:.3}");
                    tr.fmt(
                        Key::HeelBannerTrimAndDraft,
                        &[("trim", &trim_s), ("draft", &draft_s)],
                    )
                }
                (Some(t), None) => {
                    let trim_s = signed(t);
                    tr.fmt(Key::HeelBannerTrimOnly, &[("trim", &trim_s)])
                }
                _ => String::new(),
            };
            (theme::OK, theme::HEEL_STATUS_OK_BG, headline, detail)
        }
        Equilibrium::Loll { .. } => (
            theme::WARN,
            theme::HEEL_STATUS_WARN_BG,
            equilibrium_line_tr(&out.equilibrium, disp, tr),
            tr.s(Key::HeelBannerLollGmNeg).to_string(),
        ),
        Equilibrium::Capsizes { .. } => (
            theme::FAIL,
            theme::HEEL_STATUS_FAIL_BG,
            tr.s(Key::HeelBadgeCapsizes).to_string(),
            tr.s(Key::HeelBannerCapsizesBody).to_string(),
        ),
        Equilibrium::CannotFloat => {
            let disp_s = format!("{disp:.2}");
            (
                theme::FAIL,
                theme::HEEL_STATUS_FAIL_BG,
                tr.s(Key::HeelBannerCannotFloat).to_string(),
                tr.fmt(Key::HeelBannerCannotFloatBody, &[("disp", &disp_s)]),
            )
        }
        Equilibrium::Unresolved { reason } => (
            theme::WARN,
            theme::HEEL_STATUS_WARN_BG,
            tr.s(Key::HeelBannerNoEq).to_string(),
            reason.clone(),
        ),
    };
    paint_status_card(ui, accent, fill, &title, &detail, None);
}

fn paint_status_card(
    ui: &mut egui::Ui,
    accent: egui::Color32,
    fill: egui::Color32,
    title: &str,
    detail: &str,
    note: Option<&str>,
) {
    let resp = egui::Frame::new()
        .fill(fill)
        .stroke(egui::Stroke::new(1.0, theme::FRAME))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin {
            left: 10,
            right: 6,
            top: 3,
            bottom: 3,
        })
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.add(egui::Label::new(egui::RichText::new(title).strong().color(accent)).wrap());
            if !detail.is_empty() {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(detail)
                            .size(theme::SMALL_SIZE)
                            .color(theme::TEXT_LABEL),
                    )
                    .wrap(),
                );
            }
            if let Some(n) = note.filter(|s| !s.is_empty()) {
                ui.add(
                    egui::Label::new(
                        egui::RichText::new(n)
                            .size(theme::SMALL_SIZE)
                            .color(theme::TEXT_MUTED),
                    )
                    .wrap(),
                );
            }
        });
    let r = resp.response.rect;
    let strip = egui::Rect::from_min_max(r.left_top(), egui::pos2(r.left() + 3.0, r.bottom()));
    ui.painter().rect_filled(strip, 1.0, accent);
}

/// Width at which the right-hand readout splits Turning couple and Floating condition side-by-side
/// so Trial weights and the GZ plot stay above the fold.
const READOUT_TWO_COL_WIDTH: f32 = 360.0;

/// Structured three-column readout card for the currently displayed heel angle.
fn readout_table(ui: &mut egui::Ui, s: &HeelState, out: &HeelSweepOut, approx: bool, tr: Tr) {
    let (couple, condition) = readout_rows_tr(s, out, tr);
    let moment_knm = draw::righting_moment_knm(s.displacement_t, s.lever_m);
    let righting = draw::is_righting(s.heel_deg, s.lever_m);
    let peak_knm = out
        .sweep
        .iter()
        .flatten()
        .map(|st| draw::righting_moment_knm(st.displacement_t, st.lever_m).abs())
        .fold(moment_knm.abs(), f64::max)
        .max(1e-6);

    let max_width = |rows: &[ReadoutRow]| {
        rows.iter()
            .map(|r| r.value.chars().count())
            .max()
            .unwrap_or(0)
    };

    let two_cols = ui.available_width() >= READOUT_TWO_COL_WIDTH;
    egui::Frame::new()
        .fill(theme::PLOT_BG)
        .stroke(egui::Stroke::new(1.0, theme::FRAME))
        .corner_radius(theme::RADIUS)
        .inner_margin(egui::Margin::symmetric(6, 4))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 1.0;
            ui.label(
                egui::RichText::new(heel_words_tr(s.heel_deg, approx, tr))
                    .strong()
                    .color(theme::TEXT_PRIMARY),
            );
            if two_cols {
                let avail = ui.available_width();
                let gap = 8.0;
                let left_w = ((avail - gap) * 0.54).round();
                let right_w = (avail - gap - left_w).max(120.0);
                let (w_c, w_k) = (max_width(&couple), max_width(&condition));
                ui.horizontal_top(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(left_w);
                        paint_readout_grid(
                            ui,
                            "heel_couple_grid",
                            tr.s(Key::HeelReadoutCoupleHead),
                            &couple,
                            w_c,
                            true,
                        );
                        paint_moment_bar(ui, moment_knm.abs(), peak_knm, righting, tr);
                    });
                    ui.add_space(gap);
                    ui.vertical(|ui| {
                        ui.set_width(right_w);
                        paint_readout_grid(
                            ui,
                            "heel_condition_grid",
                            tr.s(Key::HeelReadoutConditionHead),
                            &condition,
                            w_k,
                            true,
                        );
                    });
                });
            } else {
                let w = max_width(&couple).max(max_width(&condition));
                paint_readout_grid(
                    ui,
                    "heel_couple_grid",
                    tr.s(Key::HeelReadoutCoupleHead),
                    &couple,
                    w,
                    false,
                );
                paint_moment_bar(ui, moment_knm.abs(), peak_knm, righting, tr);
                ui.separator();
                paint_readout_grid(
                    ui,
                    "heel_condition_grid",
                    tr.s(Key::HeelReadoutConditionHead),
                    &condition,
                    w,
                    false,
                );
            }
        });
}

fn paint_readout_grid(
    ui: &mut egui::Ui,
    id: &str,
    subhead: &str,
    rows: &[ReadoutRow],
    width: usize,
    compact: bool,
) {
    ui.label(
        egui::RichText::new(subhead)
            .size(theme::SMALL_SIZE)
            .color(theme::TEXT_MUTED),
    );
    let col_gap = if compact { 4.0 } else { 10.0 };
    let unit_min_w = if compact { 20.0 } else { theme::UNIT_WIDTH };
    egui::Grid::new(id)
        .num_columns(3)
        .striped(true)
        .min_col_width(0.0)
        .spacing(egui::vec2(col_gap, 1.0))
        .show(ui, |ui| {
            for r in rows {
                let name = ui.label(egui::RichText::new(r.name).color(theme::TEXT_SECONDARY));
                let mut val_text = egui::RichText::new(format!("{:>width$}", r.value))
                    .monospace()
                    .color(r.colour);
                if r.strong {
                    val_text = val_text.strong();
                }
                let val = ui.label(val_text);
                ui.horizontal(|ui| {
                    ui.set_min_width(unit_min_w);
                    ui.label(
                        egui::RichText::new(r.unit)
                            .size(theme::SMALL_SIZE)
                            .color(theme::TEXT_MUTED),
                    );
                });
                name.on_hover_text(r.hint);
                val.on_hover_text(r.hint);
                ui.end_row();
            }
        });
}

/// Zero-centred horizontal bar comparing the current turning moment against peak sweep moment:
/// green to the right when righting, coral-red to the left when capsizing.
fn paint_moment_bar(ui: &mut egui::Ui, abs_knm: f64, peak_knm: f64, righting: bool, tr: Tr) {
    let w = ui.available_width().max(80.0);
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 8.0), egui::Sense::hover());
    let p = ui.painter();
    p.rect(
        rect,
        2.0,
        theme::FIELD,
        egui::Stroke::new(1.0, theme::FRAME),
        egui::StrokeKind::Inside,
    );
    let mid_x = rect.center().x;
    let frac = (abs_knm / peak_knm.max(1e-6)).clamp(0.0, 1.0) as f32;
    let half = 0.5 * (rect.width() - 2.0);
    if frac > 0.005 {
        let fill_col = if righting {
            theme::LEVER_RIGHTING
        } else {
            theme::LEVER_CAPSIZING
        };
        let fill = if righting {
            egui::Rect::from_min_max(
                egui::pos2(mid_x, rect.top() + 1.0),
                egui::pos2(mid_x + half * frac, rect.bottom() - 1.0),
            )
        } else {
            egui::Rect::from_min_max(
                egui::pos2(mid_x - half * frac, rect.top() + 1.0),
                egui::pos2(mid_x, rect.bottom() - 1.0),
            )
        };
        p.rect_filled(fill, 1.0, fill_col);
    }
    p.line_segment(
        [
            egui::pos2(mid_x, rect.top()),
            egui::pos2(mid_x, rect.bottom()),
        ],
        egui::Stroke::new(1.0, theme::GRID_DATUM),
    );
    let pct_s = format!("{:.0}", frac * 100.0);
    let peak_s = draw::format_moment_knm(peak_knm);
    resp.on_hover_text(tr.fmt(Key::HeelMomentBarTip, &[("pct", &pct_s), ("peak", &peak_s)]));
}

/// Minimum accepted trial-weight mass (tonnes), shared by both mass editors.
const MIN_TRIAL_MASS_T: f64 = 1e-6;
const TRIAL_MASS_RANGE: std::ops::RangeInclusive<f64> = MIN_TRIAL_MASS_T..=f64::INFINITY;

fn valid_trial_mass(m: f64) -> bool {
    m.is_finite() && TRIAL_MASS_RANGE.contains(&m)
}

/// The small field where a weight's mass is typed, at the point picked. Enter places (or edits)
/// the weight; Esc cancels.
fn mass_field(
    ui: &mut egui::Ui,
    state: &mut HeelViewState,
    views: &DrawnViews,
    tr: Tr,
) -> HeelViewAction {
    let Some(mut entry) = state.entry.clone() else {
        return HeelViewAction::None;
    };
    let (a, c, h) = views.frame.to_earth(entry.at);
    let at = match entry.drawing {
        Drawing::BodyPlan => views.body_view.to_screen([c, h]),
        Drawing::Profile => views.profile_view.to_screen([a, h]),
    };
    let mut action = HeelViewAction::None;
    let mut close = false;
    egui::Area::new(egui::Id::unique("heel_weight_entry"))
        .order(egui::Order::Foreground)
        .fixed_pos(at + egui::vec2(10.0, -10.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.horizontal(|ui| {
                    let edit = ui.add(
                        egui::TextEdit::singleline(&mut entry.text)
                            .desired_width(56.0)
                            .hint_text(tr.s(Key::HeelMassHint)),
                    );
                    ui.label("t");
                    if !entry.focused {
                        edit.request_focus();
                        entry.focused = true;
                    }
                    let (enter, escape) = ui.input(|i| {
                        (
                            i.key_pressed(egui::Key::Enter),
                            i.key_pressed(egui::Key::Escape),
                        )
                    });
                    if escape {
                        close = true;
                    } else if enter && (edit.lost_focus() || edit.has_focus()) {
                        match entry.text.trim().parse::<f64>() {
                            Ok(m) if valid_trial_mass(m) => {
                                place(state, &entry, m, tr);
                                close = true;
                                action = HeelViewAction::Changed;
                            }
                            _ => {
                                entry.error = Some(tr.s(Key::HeelMassError).into());
                                entry.focused = false;
                            }
                        }
                    }
                });
                if let Some(e) = &entry.error {
                    ui.colored_label(theme::FAIL, e);
                }
            });
        });
    state.entry = if close { None } else { Some(entry) };
    action
}

/// Place a new weight at the entry's point, or set the mass of the one it edits.
fn place(state: &mut HeelViewState, entry: &PendingWeight, mass_t: f64, tr: Tr) {
    match entry.editing.and_then(|i| state.weights.get_mut(i)) {
        Some(w) => w.mass_t = mass_t,
        None => {
            let n = state.weights.len() + 1;
            let n_s = n.to_string();
            state.weights.push(WeightItem {
                name: tr.fmt(Key::HeelWeightsNameN, &[("n", &n_s)]),
                mass_t,
                x: entry.at.x,
                y: entry.at.y,
                z: entry.at.z,
            });
            state.selected = Some(n - 1);
        }
    }
}

/// ← → step the heel (1°, Shift 5°) and Delete removes the selected weight, while the pointer is
/// over a drawing and no field has the keyboard.
fn keys(ui: &mut egui::Ui, state: &mut HeelViewState, views: &DrawnViews) -> HeelViewAction {
    let over = ui
        .ctx()
        .pointer_hover_pos()
        .is_some_and(|p| views.body.contains(p) || views.profile.contains(p));
    if !over || ui.ctx().egui_wants_keyboard_input() {
        return HeelViewAction::None;
    }
    let mut action = HeelViewAction::None;
    // The step is read from each key event's own modifiers: Shift for 5°. Alt+arrows belong to
    // the station stepping in the modelling views.
    let (delta, delete) = ui.input_mut(|i| {
        let mut delta = 0.0;
        let mut delete = false;
        i.events.retain(|e| match e {
            egui::Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } if !modifiers.alt && !modifiers.command && !modifiers.ctrl => {
                let step = if modifiers.shift { 5.0 } else { 1.0 };
                match key {
                    egui::Key::ArrowRight => delta += step,
                    egui::Key::ArrowLeft => delta -= step,
                    egui::Key::Delete | egui::Key::Backspace => delete = true,
                    _ => return true,
                }
                false
            }
            _ => true,
        });
        (delta, delete)
    });
    if delete {
        if let Some(i) = state.selected {
            if i < state.weights.len() {
                remove_weight(state, i);
                action = HeelViewAction::Changed;
            }
        }
        state.selected = None;
    }
    if delta != 0.0 {
        state.heel_deg = (state.heel_deg + delta).clamp(-90.0, 90.0);
        state.anim = None;
        action = HeelViewAction::ManualHeel;
    }
    action
}

/// The trial weights as a table (typed edits) and the way into the model: "Add to this condition".
fn weights_table(
    ui: &mut egui::Ui,
    state: &mut HeelViewState,
    conditions: &[String],
    tr: Tr,
) -> HeelViewAction {
    let mut action = HeelViewAction::None;
    let count_s = state.weights.len().to_string();
    let title = tr.fmt(Key::HeelWeightsTitle, &[("count", &count_s)]);
    egui::CollapsingHeader::new(title)
        .id_salt("heel_weights")
        .default_open(true)
        .show(ui, |ui| {
            if state.weights.is_empty() {
                ui.label(
                    egui::RichText::new(tr.s(Key::HeelWeightsEmptyHint))
                        .color(theme::TEXT_SECONDARY),
                );
            }
            let mut remove = None;
            // Units in the headings keep each row narrow; a narrow window scrolls it sideways
            // rather than cutting off the last columns.
            egui::ScrollArea::horizontal()
                .id_salt("heel_weights_scroll")
                .auto_shrink([false, true])
                .show(ui, |ui| {
                    egui::Grid::new("heel_weights_grid")
                        .striped(true)
                        .show(ui, |ui| {
                            if !state.weights.is_empty() {
                                for h in
                                    ["", tr.s(Key::HeelWeightsColMass), "x m", "y m", "z m", ""]
                                {
                                    ui.label(egui::RichText::new(h).color(theme::TEXT_SECONDARY));
                                }
                                ui.end_row();
                            }
                            let selected = state.selected;
                            for (i, w) in state.weights.iter_mut().enumerate() {
                                let mut edited = false;
                                if ui.selectable_label(selected == Some(i), &w.name).clicked() {
                                    state.selected = Some(i);
                                }
                                let mut mass = w.mass_t;
                                let resp = ui.add(
                                    egui::DragValue::new(&mut mass)
                                        .speed(0.05)
                                        .max_decimals(6)
                                        .range(TRIAL_MASS_RANGE)
                                        .clamp_existing_to_range(false),
                                );
                                if resp.changed() && valid_trial_mass(mass) {
                                    w.mass_t = mass;
                                    edited = true;
                                }
                                for v in [&mut w.x, &mut w.y, &mut w.z] {
                                    let mut coord = *v;
                                    let resp = ui.add(
                                        egui::DragValue::new(&mut coord)
                                            .speed(0.02)
                                            .fixed_decimals(2),
                                    );
                                    if resp.changed() && coord.is_finite() {
                                        *v = coord;
                                        edited = true;
                                    }
                                }
                                if ui
                                    .small_button("✕")
                                    .on_hover_text(tr.s(Key::HeelWeightsRemoveTip))
                                    .clicked()
                                {
                                    remove = Some(i);
                                }
                                if edited {
                                    action = HeelViewAction::Changed;
                                }
                                ui.end_row();
                            }
                        });
                });
            if let Some(i) = remove {
                remove_weight(state, i);
                action = HeelViewAction::Changed;
            }
            let none = state.weights.is_empty();
            match state.start {
                HeelStart::Condition(i) => {
                    let button = ui
                        .add_enabled(
                            !none,
                            egui::Button::new(tr.s(Key::HeelWeightsAddToCondition)),
                        )
                        .on_hover_text(tr.s(Key::HeelWeightsAddToConditionTip))
                        .on_disabled_hover_text(tr.s(Key::HeelWeightsPlaceFirst));
                    if button.clicked() {
                        action = HeelViewAction::AddToCondition(i);
                    }
                }
                HeelStart::Dock => {
                    let enabled = !none && !conditions.is_empty();
                    let reason = if none {
                        tr.s(Key::HeelWeightsPlaceFirst)
                    } else {
                        tr.s(Key::HeelWeightsNoConditions)
                    };
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.menu_button(tr.s(Key::HeelWeightsAddToLoading), |ui| {
                            for (i, name) in conditions.iter().enumerate() {
                                if ui.button(name).clicked() {
                                    action = HeelViewAction::AddToCondition(i);
                                    ui.close();
                                }
                            }
                        })
                        .response
                        .on_disabled_hover_text(reason);
                    });
                }
            }
        });
    action
}

/// The weight under `p` in `drawing`, if one is within reach.
fn weight_at(
    state: &HeelViewState,
    views: &DrawnViews,
    drawing: Drawing,
    p: egui::Pos2,
) -> Option<usize> {
    state
        .weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let (a, c, h) = views.frame.to_earth(Vec3::new(w.x, w.y, w.z));
            let s = match drawing {
                Drawing::BodyPlan => views.body_view.to_screen([c, h]),
                Drawing::Profile => views.profile_view.to_screen([a, h]),
            };
            (i, (s - p).length())
        })
        .filter(|&(_, d)| d <= WEIGHT_PICK)
        .min_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(i, _)| i)
}

/// The model point under `p` in `drawing`: on the section drawn (body plan) or the centreline
/// plane (profile), or in the plane through `keep` when a weight is moved.
fn model_at(
    views: &DrawnViews,
    drawing: Drawing,
    p: egui::Pos2,
    keep: Option<Vec3>,
) -> Option<Vec3> {
    match drawing {
        Drawing::BodyPlan => {
            let [c, h] = views.body_view.from_screen(p);
            views
                .frame
                .model_at_x(c, h, keep.map_or(views.section_x, |k| k.x))
        }
        Drawing::Profile => {
            let [a, h] = views.profile_view.from_screen(p);
            views.frame.model_at_y(a, h, keep.map_or(0.0, |k| k.y))
        }
    }
}

/// A click places a weight (or selects one), a double click edits a weight's mass, a drag moves
/// a weight or, in the body plan, the heel.
fn pointer(
    state: &mut HeelViewState,
    resp: &egui::Response,
    drawing: Drawing,
    views: &DrawnViews,
) -> HeelViewAction {
    let mut action = HeelViewAction::None;
    let at = resp.interact_pointer_pos();
    if resp.drag_started() {
        let origin = resp.ctx.input(|i| i.pointer.press_origin()).or(at);
        let hit = origin.and_then(|p| weight_at(state, views, drawing, p));
        state.grab = match (hit, drawing) {
            (Some(i), _) => {
                state.selected = Some(i);
                Some(Grab::Weight(i, drawing))
            }
            (None, Drawing::BodyPlan) => Some(Grab::Heel),
            (None, Drawing::Profile) => None,
        };
        state.anim = None;
        state.entry = None;
    }
    if resp.dragged() {
        match state.grab {
            Some(Grab::Heel) if drawing == Drawing::BodyPlan => {
                let dx = f64::from(resp.drag_delta().x);
                if dx != 0.0 {
                    state.heel_deg = (state.heel_deg + dx * DEG_PER_POINT).clamp(-90.0, 90.0);
                    action = HeelViewAction::Changed;
                }
            }
            Some(Grab::Weight(i, d)) if d == drawing => {
                let keep = state.weights.get(i).map(|w| Vec3::new(w.x, w.y, w.z));
                if let (Some(p), Some(k)) = (at, keep) {
                    if let Some(m) = model_at(views, drawing, p, Some(k)) {
                        let w = &mut state.weights[i];
                        (w.x, w.y, w.z) = (m.x, m.y, m.z);
                        action = HeelViewAction::Changed;
                    }
                }
            }
            _ => {}
        }
    }
    if resp.drag_stopped() {
        if state.grab == Some(Grab::Heel) {
            action = HeelViewAction::SettleNow;
        }
        state.grab = None;
    }
    if resp.double_clicked() {
        if let Some(i) = at.and_then(|p| weight_at(state, views, drawing, p)) {
            let w = &state.weights[i];
            let text = format!("{}", w.mass_t);
            let at = Vec3::new(w.x, w.y, w.z);
            state.entry = Some(PendingWeight::new(at, drawing, Some(i), text));
        }
    } else if resp.clicked() {
        match at.map(|p| (p, weight_at(state, views, drawing, p))) {
            Some((_, Some(i))) => {
                state.selected = Some(i);
                state.entry = None;
            }
            Some((p, None)) => {
                state.selected = None;
                state.entry = model_at(views, drawing, p, None)
                    .map(|m| PendingWeight::new(m, drawing, None, String::new()));
            }
            None => {}
        }
    }
    action
}

/// The view's out-of-date chip: says whether the update failed, waits for manual
/// recalculation, or is recomputing.
fn stale_chip(ui: &mut egui::Ui, failed: bool, waiting_manual: bool, tr: Tr) {
    let (text, colour) = if failed {
        (tr.s(Key::HeelStaleFailed), theme::STALE)
    } else if waiting_manual {
        (tr.s(Key::HeelStaleManual), theme::WARN)
    } else {
        (tr.s(Key::HeelStaleRecomputing), theme::STALE)
    };
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, colour))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new(text)
                    .size(theme::SMALL_SIZE)
                    .color(colour),
            )
        });
}

/// The body plan and the profile side by side, with their captions. `shown` carries B, G and the
/// lever; without it the hull is drawn alone. `profile_note` follows "From starboard".
#[allow(clippy::too_many_arguments)]
fn drawings(
    ui: &mut egui::Ui,
    state: &mut HeelViewState,
    outlines: &geometry::HullOutlines,
    frame: &geometry::EarthFrame,
    section: Option<&[Vec<Vec3>]>,
    shown: Option<&HeelState>,
    profile_note: &str,
    capsize: draw::CapsizeAlert,
    interactive: bool,
    height: f32,
    tr: Tr,
) -> (HeelViewAction, DrawnViews) {
    let width = ui.available_width();
    let gap = 8.0;
    let (body_width, profile_width) = layout_widths(width, height, gap);
    let sense = if interactive {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    // Fixed ids: an auto id moves when anything before it changes, and a drag on it ends.
    let (body_resp, profile_resp) = ui
        .horizontal(|ui| {
            let (b, _) =
                ui.allocate_exact_size(egui::vec2(body_width, height), egui::Sense::hover());
            ui.add_space(gap);
            let (p, _) =
                ui.allocate_exact_size(egui::vec2(profile_width, height), egui::Sense::hover());
            (
                ui.interact(b, egui::Id::unique("heel_view_body_plan"), sense),
                ui.interact(p, egui::Id::unique("heel_view_profile"), sense),
            )
        })
        .inner;
    let (body_rect, profile_rect) = (body_resp.rect, profile_resp.rect);
    let (body_view, profile_view) = draw::fit_views(body_rect, profile_rect, outlines, frame);
    let section_x = state
        .section
        .or(shown.map(|s| s.b.x))
        .unwrap_or(0.5 * (outlines.min.x + outlines.max.x));
    let views = DrawnViews {
        body: body_rect,
        body_view,
        profile: profile_rect,
        profile_view,
        frame: *frame,
        section_x,
        splitter: egui::Rect::NOTHING,
    };
    let mut action = HeelViewAction::None;
    if interactive {
        for (resp, drawing) in [
            (&body_resp, Drawing::BodyPlan),
            (&profile_resp, Drawing::Profile),
        ] {
            let a = pointer(state, resp, drawing, &views);
            if a != HeelViewAction::None {
                action = a;
            }
        }
    }
    let stations = geometry::body_plan(outlines, frame);
    let section = section
        .map(|s| geometry::project_section(s, frame))
        .unwrap_or_default();
    let earth = |p: Vec3| frame.to_earth(p);
    let weights_body: Vec<(geometry::Point2, String, bool)> = state
        .weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let (_, c, h) = earth(Vec3::new(w.x, w.y, w.z));
            (
                [c, h],
                format!("{:.1} t", w.mass_t),
                state.selected == Some(i),
            )
        })
        .collect();
    let weights_profile: Vec<(geometry::Point2, String, bool)> = state
        .weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            let (a, _, h) = earth(Vec3::new(w.x, w.y, w.z));
            (
                [a, h],
                format!("{:.1} t", w.mass_t),
                state.selected == Some(i),
            )
        })
        .collect();
    let forces = shown.map(|s| {
        let (_, bc, bh) = earth(s.b);
        let (_, gc, gh) = earth(s.g);
        draw::Forces {
            b: [bc, bh],
            g: [gc, gh],
            heel_deg: s.heel_deg,
            lever_m: s.lever_m,
            displacement_t: s.displacement_t,
        }
    });
    let painter = ui.painter_at(body_rect);
    draw::paint_body_plan_tr(
        &painter,
        body_rect,
        &body_view,
        &draw::BodyPlanScene {
            stations: &stations,
            section: &section,
            forces,
            weights: &weights_body,
            capsize,
        },
        tr,
    );
    let (top, bottom) = geometry::profile_silhouette(outlines, frame);
    let painter = ui.painter_at(profile_rect);
    draw::paint_profile(
        &painter,
        profile_rect,
        &profile_view,
        &draw::ProfileScene {
            top: &top,
            bottom: &bottom,
            b: shown.map(|s| {
                let (a, _, h) = earth(s.b);
                [a, h]
            }),
            g: shown.map(|s| {
                let (a, _, h) = earth(s.g);
                [a, h]
            }),
            weights: &weights_profile,
            capsize,
        },
    );
    let section_x = (shown.is_some() || state.section.is_some()).then_some(section_x);
    let small = |t: String| {
        egui::RichText::new(t)
            .size(theme::SMALL_SIZE)
            .color(theme::TEXT_SECONDARY)
    };
    // Each caption wraps within its drawing's width: a wider one would widen the column and push
    // what sits beside the drawings out of sight.
    let caption = |ui: &mut egui::Ui, text: String, w: f32| {
        ui.allocate_ui_with_layout(
            egui::vec2(w, 0.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.set_width(w);
                ui.add(egui::Label::new(small(text)).wrap());
            },
        );
    };
    ui.horizontal_top(|ui| {
        let body_caption = match (shown, section_x) {
            (Some(_), Some(x)) => {
                let x_s = format!("{x:.2}");
                tr.fmt(Key::HeelCaptionBodyProjected, &[("x", &x_s)])
            }
            (None, Some(x)) => {
                let x_s = format!("{x:.2}");
                tr.fmt(Key::HeelCaptionBodySection, &[("x", &x_s)])
            }
            _ => tr.s(Key::HeelCaptionBodyOnly).to_string(),
        };
        caption(ui, body_caption, body_width);
        ui.add_space(gap);
        caption(
            ui,
            tr.fmt(Key::HeelCaptionProfile, &[("note", profile_note)]),
            profile_width,
        );
    });
    (action, views)
}

/// The condition's GZ curve (G on the centreline) and heeling arm against heel — as lever (m) or
/// turning moment (kN·m) — with the heel shown and the equilibrium marked; returns the heel clicked.
fn gz_plot(
    ui: &mut egui::Ui,
    out: &HeelSweepOut,
    shown_heel: Option<f64>,
    height: f32,
    as_moment: bool,
    tr: Tr,
) -> Option<f64> {
    use crate::panels::{plot_interactive_sized, PlotMark, PlotSeries};
    let pts = |f: fn(&HeelState) -> f64| -> Vec<(f64, f64)> {
        out.sweep
            .iter()
            .map(|o| match o {
                Some(s) => (s.heel_deg, f(s)),
                None => (f64::NAN, f64::NAN),
            })
            .collect()
    };
    let series = if as_moment {
        [
            PlotSeries {
                label: tr.s(Key::HeelPlotGzMomentSeries).into(),
                points: pts(|s| draw::righting_moment_knm(s.displacement_t, s.gz_m)),
                dashed: false,
            },
            PlotSeries {
                label: tr.s(Key::HeelPlotHeelingMomentSeries).into(),
                points: pts(|s| draw::righting_moment_knm(s.displacement_t, s.heeling_arm_m)),
                dashed: true,
            },
        ]
    } else {
        [
            PlotSeries {
                label: tr.s(Key::HeelPlotGzLeverSeries).into(),
                points: pts(|s| s.gz_m),
                dashed: false,
            },
            PlotSeries {
                label: tr.s(Key::HeelPlotHeelingArmSeries).into(),
                points: pts(|s| s.heeling_arm_m),
                dashed: true,
            },
        ]
    };
    let title = if as_moment {
        tr.s(Key::HeelPlotTitleMoment)
    } else {
        tr.s(Key::HeelPlotTitleLever)
    };
    let marks: Vec<PlotMark> = match &out.equilibrium {
        Equilibrium::Stable { state } => vec![PlotMark {
            x: state.heel_deg,
            label: tr.s(Key::HeelPlotMarkSettles),
        }],
        Equilibrium::Loll { port, starboard } => vec![
            PlotMark {
                x: port.heel_deg,
                label: tr.s(Key::HeelPlotMarkLoll),
            },
            PlotMark {
                x: starboard.heel_deg,
                label: tr.s(Key::HeelPlotMarkLoll),
            },
        ],
        _ => Vec::new(),
    };
    plot_interactive_sized(
        ui,
        title,
        &series,
        shown_heel,
        &marks,
        Some((-90.0, 90.0)),
        height,
    )
    .1
}

/// Draw the view's header: the views as tabs and the close button.
pub fn header(ui: &mut egui::Ui) -> HeelViewAction {
    header_tr(ui, Tr::new(Locale::En))
}

/// Localized form of [`header`].
pub fn header_tr(ui: &mut egui::Ui, tr: Tr) -> HeelViewAction {
    let mut action = HeelViewAction::None;
    ui.horizontal_wrapped(|ui| {
        for other in AnalysisView::ALL {
            let shown = other == AnalysisView::Heel;
            if ui
                .selectable_label(shown, other.tab_label_tr(shown, tr))
                .clicked()
                && !shown
            {
                action = HeelViewAction::Open(other);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("✕")
                .on_hover_text(tr.s(Key::HeelCloseTip))
                .clicked()
            {
                action = HeelViewAction::Close;
            }
        });
    });
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    /// (heel, lever) every 2° from −90° to 90° for `lever(h)`.
    fn curve(lever: impl Fn(f64) -> f64) -> Vec<(f64, f64)> {
        (-45..=45)
            .map(|i| f64::from(i) * 2.0)
            .map(|h| (h, lever(h)))
            .collect()
    }

    // Hand-made lever curves (positive turns the hull to port), not hull numbers.

    #[test]
    fn let_go_inside_the_righting_range_it_comes_back() {
        // Rights itself between ±20°, capsizes beyond.
        let c = curve(|h| h * (400.0 - h * h));
        for from in [10.0, -10.0, 19.0] {
            match let_go(&c, from) {
                LetGo::Settles(to) => assert!(to.abs() < 1e-9, "from {from} to {to}"),
                other => panic!("from {from}: {other:?}"),
            }
        }
    }

    #[test]
    fn let_go_past_the_vanishing_angle_it_capsizes() {
        let c = curve(|h| h * (400.0 - h * h));
        assert_eq!(let_go(&c, 30.0), LetGo::Capsizes);
        assert_eq!(let_go(&c, -21.0), LetGo::Capsizes);
    }

    #[test]
    fn let_go_on_a_lolling_hull_it_finds_the_loll_on_its_side() {
        // Upright is unstable; it lolls at ±10° and capsizes past ±20°.
        let c = curve(|h| h * (h * h - 100.0) * (400.0 - h * h));
        let to = |from| match let_go(&c, from) {
            LetGo::Settles(to) => to,
            other => panic!("from {from}: {other:?}"),
        };
        assert!((to(5.0) - 10.0).abs() < 1e-9);
        assert!((to(15.0) - 10.0).abs() < 1e-9);
        assert!((to(-5.0) + 10.0).abs() < 1e-9);
    }

    fn trial(x: f64) -> WeightItem {
        WeightItem {
            name: format!("at {x}"),
            mass_t: 1.0,
            x,
            y: 0.0,
            z: 1.0,
        }
    }

    /// Removing a weight keeps the mass field and a drag on the weights they were on.
    #[test]
    fn removing_a_weight_keeps_edits_and_drags_on_their_own_weight() {
        let mut state = HeelViewState {
            weights: vec![trial(1.0), trial(2.0), trial(3.0)],
            entry: Some(PendingWeight::new(
                Vec3::ZERO,
                Drawing::Profile,
                Some(2),
                "1".into(),
            )),
            grab: Some(Grab::Weight(2, Drawing::Profile)),
            selected: Some(2),
            ..HeelViewState::default()
        };
        remove_weight(&mut state, 0);
        assert_eq!(state.weights[1].x, 3.0);
        assert_eq!(state.entry.as_ref().unwrap().editing, Some(1));
        assert_eq!(state.grab, Some(Grab::Weight(1, Drawing::Profile)));
        assert_eq!(state.selected, Some(1));
        remove_weight(&mut state, 1);
        assert_eq!(state.entry, None, "its weight is gone");
        assert_eq!(state.grab, None);
        assert_eq!(state.selected, None);
    }

    #[test]
    fn the_drawings_and_the_plot_share_the_drawer() {
        let (d, p) = layout_heights(1000.0, 0.6);
        assert!(
            (d + p - (1000.0 - LAYOUT_OVERHEAD)).abs() < 1e-3,
            "{d} + {p}"
        );
        assert!((d / (d + p) - 0.6).abs() < 1e-3);
        // A short drawer keeps both at their least height (and scrolls).
        assert_eq!(
            layout_heights(150.0, 0.6),
            (MIN_DRAWING_HEIGHT, MIN_PLOT_HEIGHT)
        );
        // The splitter cannot squeeze either away.
        assert_eq!(layout_heights(1000.0, 0.99).1, MIN_PLOT_HEIGHT);
        assert_eq!(layout_heights(1000.0, 0.01).0, MIN_DRAWING_HEIGHT);
    }

    #[test]
    fn the_body_plan_is_as_wide_as_a_section_needs() {
        let (b, p) = layout_widths(1100.0, 300.0, 8.0);
        assert!((b - 300.0 * BODY_ASPECT).abs() < 1e-3, "{b}");
        assert!((b + p - 1092.0).abs() < 1e-3);
        // In a narrow row the body plan takes at most half.
        let (b, p) = layout_widths(400.0, 300.0, 8.0);
        assert!(
            (b - 196.0).abs() < 1e-3 && (p - 196.0).abs() < 1e-3,
            "{b} {p}"
        );
    }

    #[test]
    fn let_go_where_nothing_turns_it_it_stays() {
        let c = curve(|h| h * (400.0 - h * h));
        assert_eq!(let_go(&c, 0.0), LetGo::Stays);
        assert_eq!(let_go(&c, 95.0), LetGo::Stays, "outside the sweep");
    }

    #[test]
    fn let_go_stops_at_failed_sweep_gaps() {
        // An unsolved gap at +4° must neither allow release inside [2°, 6°] nor bridge
        // a sign change from +2° (< 0) to +6° (> 0) across +4°.
        let mut c = curve(|h| h - 4.0);
        c[47] = (f64::NAN, f64::NAN); // +4°
        assert_eq!(let_go(&c, 3.0), LetGo::Stays);
        assert_eq!(let_go(&c, 5.0), LetGo::Stays);
        // Released at +1° (lever < 0, walking right towards +4°), it hits the gap before +6° and stops.
        assert_eq!(let_go(&c, 1.0), LetGo::Stays);
    }

    #[test]
    fn settle_target_chooses_own_side_for_sub_step_loll() {
        use seacraft_core::heel::{equilibrium, heel_sweep, sweep_heels, MassCondition};
        use seacraft_core::hydro::{Hull, HydroOptions};
        use seacraft_core::subdiv::SubdivisionOptions;
        use seacraft_core::{Layer, ProjectSettings, Surface};

        let (l, b, d, t) = (10.0, 4.0, 3.0, 1.0);
        let mut s = Surface::new();
        s.layers = vec![Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Layer::default()
        }];
        s.settings = ProjectSettings {
            name: "box".into(),
            length: l,
            beam: b,
            draft: t,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..ProjectSettings::default()
        };
        for (x, y, z) in [
            (0.0, -2.0, 0.0),
            (l, -2.0, 0.0),
            (l, 2.0, 0.0),
            (0.0, 2.0, 0.0),
            (0.0, -2.0, d),
            (l, -2.0, d),
            (l, 2.0, d),
            (0.0, 2.0, d),
        ] {
            s.add_point(Vec3::new(x, y, z));
        }
        for f in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            s.add_face(f, 0);
        }
        s.initialize_trace();
        s.edges.iter_mut().for_each(|e| e.crease = true);
        s.points
            .iter_mut()
            .for_each(|p| p.kind = seacraft_core::mesh::VertexKind::Corner);
        let sub = SubdivisionOptions {
            level: 1,
            mode: Default::default(),
        };
        let opts = HydroOptions::from_surface(&s, sub);
        let hull = Arc::new(Hull::new(&s, sub));

        let bm = b * b / (12.0 * t);
        for loll_deg in [1.2_f64, 0.4_f64] {
            let tan = loll_deg.to_radians().tan();
            let kg = t / 2.0 + bm + bm * tan * tan / 2.0;
            let mass = MassCondition::bare(&hull, &opts, kg, Some(5.0)).unwrap();
            let sweep = heel_sweep(&hull, &mass, &sweep_heels(), &opts);
            let eq = equilibrium(&hull, &mass, &opts, Some(&sweep));
            let out = HeelSweepOut {
                start: mass.clone(),
                mass,
                sweep,
                equilibrium: eq,
                upright_gm: None,
                warnings: Vec::new(),
                hull: hull.clone(),
                opts: opts.clone(),
            };
            match settle_target(&out, 0.75) {
                LetGo::Settles(to) => {
                    assert!((to - loll_deg).abs() < 0.05, "{loll_deg}: +0.75 -> {to}")
                }
                other => panic!("{loll_deg}: +0.75 -> {other:?}"),
            }
            match settle_target(&out, -0.75) {
                LetGo::Settles(to) => {
                    assert!((to + loll_deg).abs() < 0.05, "{loll_deg}: -0.75 -> {to}")
                }
                other => panic!("{loll_deg}: -0.75 -> {other:?}"),
            }
            match settle_target(&out, 5.0) {
                LetGo::Settles(to) => {
                    assert!((to - loll_deg).abs() < 0.05, "{loll_deg}: +5 -> {to}")
                }
                other => panic!("{loll_deg}: +5 -> {other:?}"),
            }
            assert_eq!(
                settle_target(&out, 80.0),
                LetGo::Capsizes,
                "{loll_deg}: +80"
            );
        }
    }

    #[test]
    fn asymmetric_loll_line_names_both_sides_when_different() {
        let mut p = HeelState {
            heel_deg: -12.3,
            b: Vec3::ZERO,
            g: Vec3::ZERO,
            lever_m: 0.0,
            gz_m: 0.0,
            heeling_arm_m: 0.0,
            draft: Some(1.0),
            trim_m: Some(0.0),
            displacement_t: 40.0,
            lid_submerged: false,
            plane: Plane::new(Vec3::new(0.0, 0.0, 1.0), -1.0),
        };
        let mut s = p.clone();
        s.heel_deg = 12.3;
        assert_eq!(
            equilibrium_line(
                &Equilibrium::Loll {
                    port: p.clone(),
                    starboard: s.clone(),
                },
                40.0
            ),
            "Lolls to 12.3° either side: the upright GM is negative"
        );
        p.heel_deg = -11.4;
        s.heel_deg = 13.8;
        assert_eq!(
            equilibrium_line(
                &Equilibrium::Loll {
                    port: p,
                    starboard: s,
                },
                40.0
            ),
            "Lolls to 11.4° port or 13.8° starboard: the upright GM is negative"
        );
    }

    #[test]
    fn readout_rows_include_forces_in_kn_and_moments_in_knm() {
        let mut s = HeelState {
            heel_deg: 20.0,
            b: Vec3::new(5.0, 0.1, 0.4),
            g: Vec3::new(5.0, 0.0, 1.2),
            lever_m: -0.2,
            gz_m: -0.2,
            heeling_arm_m: 0.0,
            draft: Some(0.9),
            trim_m: Some(0.02),
            displacement_t: 10.0,
            lid_submerged: false,
            plane: Plane::new(Vec3::new(0.0, 0.0, 1.0), -0.9),
        };
        let out = HeelSweepOut {
            start: MassCondition {
                displacement_t: 10.0,
                g: s.g,
                kg_estimate: false,
            },
            mass: MassCondition {
                displacement_t: 10.0,
                g: s.g,
                kg_estimate: false,
            },
            sweep: vec![Some(s.clone())],
            equilibrium: Equilibrium::Capsizes {
                last_heel_deg: 90.0,
            },
            upright_gm: Some(-0.15),
            warnings: Vec::new(),
            hull: Arc::new(Hull::new(&Surface::new(), SubdivisionOptions::default())),
            opts: HydroOptions::from_surface(&Surface::new(), SubdivisionOptions::default()),
        };
        let (couple, cond) = readout_rows(&s, &out);
        let m_row = couple
            .iter()
            .find(|r| r.name == "Capsizing moment")
            .unwrap();
        assert_eq!(m_row.unit, "kN·m");
        assert_eq!(m_row.value, "19.61");
        let f_row = couple
            .iter()
            .find(|r| r.name == "Buoyancy / weight")
            .unwrap();
        assert_eq!(f_row.unit, "kN");
        assert_eq!(f_row.value, "98.1");
        let gm_row = cond.iter().find(|r| r.name == "Upright GM").unwrap();
        assert_eq!(gm_row.colour, theme::FAIL);
        let disp_row = cond.iter().find(|r| r.name == "Displacement").unwrap();
        assert_eq!((disp_row.value.as_str(), disp_row.unit), ("10.0", "t"));

        let mut light = s.clone();
        light.displacement_t = 0.45;
        let (_, light_cond) = readout_rows(&light, &out);
        let light_disp = light_cond
            .iter()
            .find(|r| r.name == "Displacement")
            .unwrap();
        assert_eq!((light_disp.value.as_str(), light_disp.unit), ("450", "kg"));

        let mut stable = s.clone();
        stable.lever_m = -0.1;
        let out_stable = HeelSweepOut {
            equilibrium: Equilibrium::Stable {
                state: {
                    let mut eq = s.clone();
                    eq.heel_deg = 25.0;
                    eq.lever_m = 0.0;
                    eq
                },
            },
            sweep: vec![
                Some(stable.clone()),
                Some({
                    let mut mid = s.clone();
                    mid.heel_deg = 25.0;
                    mid.lever_m = 0.0;
                    mid
                }),
                Some({
                    let mut end = s.clone();
                    end.heel_deg = 40.0;
                    end.lever_m = 0.2;
                    end
                }),
            ],
            ..out.clone()
        };
        let (couple_heel, _) = readout_rows(&stable, &out_stable);
        assert!(couple_heel.iter().any(|r| r.name == "Heeling moment"));

        s.lever_m = 0.2;
        let (couple_ok, _) = readout_rows(&s, &out);
        assert!(couple_ok.iter().any(|r| r.name == "Righting moment"));
        assert!(!valid_trial_mass(f64::INFINITY));
        assert!(valid_trial_mass(0.0005));
    }

    #[test]
    fn sheared_box_compute_sweep_and_readout_report_negative_free_trim_gm() {
        use seacraft_core::{Layer, ProjectSettings, Surface};
        let (l, b, d, t) = (10.0, 4.0, 3.0, 1.0);
        let mut s = Surface::new();
        s.layers = vec![Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..Layer::default()
        }];
        s.settings = ProjectSettings {
            name: "sheared-box".into(),
            length: l,
            beam: b,
            draft: t,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..ProjectSettings::default()
        };
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ] {
            let y_sheared = (y - b * 0.5) + 0.4 * (x - 5.0);
            s.add_point(Vec3::new(x, y_sheared, z));
        }
        for f in [
            vec![0, 3, 2, 1],
            vec![4, 5, 6, 7],
            vec![0, 1, 5, 4],
            vec![1, 2, 6, 5],
            vec![2, 3, 7, 6],
            vec![3, 0, 4, 7],
        ] {
            s.add_face(f, 0);
        }
        s.initialize_trace();
        s.edges.iter_mut().for_each(|e| e.crease = true);
        s.points
            .iter_mut()
            .for_each(|p| p.kind = seacraft_core::mesh::VertexKind::Corner);
        let sub = SubdivisionOptions {
            level: 1,
            mode: Default::default(),
        };
        let opts = HydroOptions::from_surface(&s, sub);
        let out = compute_sweep(SweepInput {
            surface: s,
            hull: None,
            subdivision: sub,
            opts,
            start: StartInput::Dock {
                kg: 1.90,
                kg_estimate: false,
                lcg: Some(5.0),
            },
            weights: Vec::new(),
        })
        .unwrap();
        let gm = out.upright_gm.expect("upright GM is computed");
        assert!((gm + 1.0 / 15.0).abs() < 1e-3, "got {gm}");
        let u = out.upright().unwrap();
        let (_, cond) = readout_rows(u, &out);
        let row = cond.iter().find(|r| r.name == "Upright GM").unwrap();
        assert_eq!(row.value, "-0.067");
        assert_eq!(row.colour, theme::FAIL);
        assert!(row.strong);
        assert!(row.hint.contains("Free-trim"), "{}", row.hint);
        assert!(readout(u, &out, false).contains("upright GM -0.067 m"));
    }
}
