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
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HeelViewState {
    pub start: HeelStart,
    /// Trial weights, model coordinates; kept when the start changes.
    pub weights: Vec<WeightItem>,
    pub selected: Option<usize>,
    /// The heel shown, degrees, + to starboard.
    pub heel_deg: f64,
    /// Keep the heel where the hand leaves it instead of settling.
    pub hold: bool,
    /// The x of the section drawn in the body plan; `None` is at the centre of buoyancy.
    pub section: Option<f64>,
    pub entry: Option<PendingWeight>,
    pub anim: Option<Settle>,
    /// What the pointer holds while it drags in a drawing.
    pub grab: Option<Grab>,
}

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
    /// The upright GM of the condition (KM at its upright waterline − KG), when it floats upright.
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

/// The sweep's (heel, righting lever) pairs that float, in heel order.
pub fn lever_curve(out: &HeelSweepOut) -> Vec<(f64, f64)> {
    out.sweep
        .iter()
        .flatten()
        .map(|s| (s.heel_deg, s.lever_m))
        .collect()
}

/// Where the hull goes when let go at `from` on `curve` ((heel, lever) in heel order): it turns
/// the way its lever pushes it (a positive lever to port) until the lever changes sign. Past the
/// heel where the lever vanishes it turns over rather than coming back.
pub fn let_go(curve: &[(f64, f64)], from: f64) -> LetGo {
    let Some(i) = curve
        .windows(2)
        .position(|w| w[0].0 <= from && from <= w[1].0)
    else {
        return LetGo::Stays;
    };
    let (a, b) = (curve[i], curve[i + 1]);
    let lever = a.1 + (b.1 - a.1) * (from - a.0) / (b.0 - a.0);
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
        .and_then(|s| {
            let (draft, trim) = (s.draft?, s.trim_m?);
            let mut o = opts.clone();
            o.draft = draft;
            o.trim = trim;
            o.heel_deg = 0.0;
            Some(hull.hydrostatics(&o).km_transverse - mass.kg(hull.keel))
        });
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
    let side = if heel_deg > 0.0 { "starboard" } else { "port" };
    match (approx, heel_deg.abs()) {
        (true, a) if a < 0.5 => "Heel ≈ 0°".to_string(),
        (true, a) => format!("Heel ≈ {a:.0}° to {side}"),
        (false, a) if a < 0.05 => "Upright".to_string(),
        (false, a) => format!("Heel {a:.1}° to {side}"),
    }
}

/// A signed length to the millimetre, reading 0.000 (not -0.000) when it rounds to zero.
pub(crate) fn signed(x: f64) -> String {
    crate::panels::format_signed(x, 3)
}

/// The readout of the state drawn, in plain words.
pub fn readout(s: &HeelState, out: &HeelSweepOut, approx: bool) -> String {
    let keel = out.hull.keel;
    let mut parts = vec![
        heel_words(s.heel_deg, approx),
        format!(
            "righting lever {} m = GZ {} m − heeling arm {} m",
            signed(s.lever_m),
            signed(s.gz_m),
            signed(s.heeling_arm_m)
        ),
    ];
    if let (Some(d), Some(t)) = (s.draft, s.trim_m) {
        parts.push(format!("draft {d:.3} m"));
        parts.push(format!("trim {} m", signed(t)));
    }
    parts.push(format!("Δ {:.2} t", s.displacement_t));
    let est = if out.mass.kg_estimate { " (est.)" } else { "" };
    parts.push(format!("KG {:.3} m{est}", out.mass.kg(keel)));
    if let Some(gm) = out.upright_gm {
        parts.push(format!("upright GM {gm:.3} m"));
    }
    parts.join(" · ")
}

/// Where the condition settles, in plain words.
pub fn equilibrium_line(eq: &Equilibrium, displacement_t: f64) -> String {
    match eq {
        Equilibrium::Stable { state } if state.heel_deg.abs() < 0.05 => match state.trim_m {
            Some(t) => format!("Settles upright, trim {} m", signed(t)),
            None => "Settles upright".to_string(),
        },
        Equilibrium::Stable { state } => {
            let side = if state.heel_deg > 0.0 {
                "starboard"
            } else {
                "port"
            };
            let mut words = format!("Settles at {:.1}° to {side}", state.heel_deg.abs());
            if let Some(t) = state.trim_m {
                words.push_str(&format!(", trim {} m", signed(t)));
            }
            words
        }
        Equilibrium::Loll { starboard, .. } => format!(
            "Lolls to {:.1}° either side: the upright GM is negative",
            starboard.heel_deg.abs()
        ),
        Equilibrium::Capsizes { .. } => "Capsizes: no stable equilibrium within ±90°".to_string(),
        Equilibrium::CannotFloat => format!("The hull cannot float {displacement_t:.2} t"),
    }
}

/// The caption under the drawings: what the picture is, and is not.
pub const STATIC_CAPTION: &str = "A static picture, free to trim: the swing back is not a roll \
    simulation. Liquids in tanks do not shift; their free surface is in KG.";

/// Height of the two drawings, points.
const DRAWING_HEIGHT: f32 = 230.0;

/// Draw the view: header, inputs, the two drawings, the readout and the GZ curve.
pub fn panel(ui: &mut egui::Ui, p: HeelViewPanel<'_>) -> HeelViewAction {
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
    let mut action = header(ui);
    ui.horizontal_wrapped(|ui| {
        let start_name = |s: HeelStart| match s {
            HeelStart::Dock => "Bare hull (dock)".to_string(),
            HeelStart::Condition(i) => conditions
                .get(i)
                .cloned()
                .unwrap_or_else(|| format!("Condition {}", i + 1)),
        };
        ui.label("Start");
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
                        action = HeelViewAction::Changed;
                    }
                }
            });
        ui.separator();
        ui.label("Heel");
        // One decimal shown, but not rounded: a settle moves the heel in small steps, and a
        // rounding slider would report them as the user's edit and stop it.
        let slider = egui::Slider::new(&mut state.heel_deg, -90.0..=90.0)
            .suffix("°")
            .custom_formatter(|v, _| format!("{v:.1}"));
        if ui.add(slider).on_hover_text("+ to starboard").changed() {
            state.anim = None;
            action = HeelViewAction::Changed;
        }
        ui.checkbox(&mut state.hold, "Hold heel")
            .on_hover_text("Keep the heel where you leave it instead of settling");
        ui.separator();
        let at_b = shown.map(|s| s.b.x);
        ui.label("Section x");
        let mut x = state.section.or(at_b).unwrap_or(0.0);
        if ui
            .add(
                egui::DragValue::new(&mut x)
                    .speed(0.05)
                    .suffix(" m")
                    .fixed_decimals(2),
            )
            .on_hover_text("The station drawn strongly in the body plan")
            .changed()
        {
            state.section = Some(x);
        }
        if ui
            .add_enabled(state.section.is_some(), egui::Button::new("at B"))
            .on_hover_text("Draw the section through the centre of buoyancy")
            .clicked()
        {
            state.section = None;
        }
        if busy {
            ui.spinner();
        }
    });
    if let Some(msg) = failure {
        ui.colored_label(theme::FAIL, format!("⚠ {msg}"));
    }
    if kg_note == KgNote::Unset && state.start == HeelStart::Dock {
        ui.colored_label(theme::WARN, NO_KG_HINT);
        if let (Some(o), Some(plane)) = (outlines, dock_plane) {
            let frame = geometry::EarthFrame::from_plane(&plane);
            let caption = " · Upright at the dock draft";
            let (_, views) = drawings(ui, state, o, &frame, section, None, caption, false);
            *drawn = Some(views);
        }
        return action;
    }
    let Some(out) = sweep else {
        return action;
    };
    if let Equilibrium::CannotFloat = out.equilibrium {
        // No picture of a hull that does not float, but the weights stay editable: an overweight
        // typo must not be a dead end.
        ui.scope(|ui| {
            if stale {
                stale_chip(ui);
                ui.multiply_opacity(theme::STALE_OPACITY);
            }
            ui.colored_label(
                theme::FAIL,
                equilibrium_line(&out.equilibrium, out.mass.displacement_t),
            );
        });
        let a = weights_table(ui, state, conditions);
        if a != HeelViewAction::None {
            action = a;
        }
        return action;
    }
    egui::ScrollArea::vertical()
        .id_salt("heel_view_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            // The drawings on the left; what they say, and the weights, beside them. Nothing
            // above the drawings comes and goes, so they hold still under a drag that makes the
            // result out of date.
            let width = ui.available_width();
            let side = (0.36 * width).clamp(280.0, 460.0);
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
                        let trim = s
                            .trim_m
                            .map_or(String::new(), |t| format!(" · trim {} m", signed(t)));
                        // Interactive while stale too: moving a weight re-keys the sweep on the
                        // first frame of the drag, and the drag must carry on over the old result.
                        let (a, views) =
                            drawings(ui, state, o, &frame, section, Some(s), &trim, true);
                        *drawn = Some(views);
                        if a != HeelViewAction::None {
                            action = a;
                        }
                    }
                });
                ui.add_space(12.0);
                ui.vertical(|ui| {
                    ui.set_width(side);
                    if stale {
                        stale_chip(ui);
                        ui.multiply_opacity(theme::STALE_OPACITY);
                    }
                    for w in &out.warnings {
                        ui.colored_label(theme::WARN, format!("⚠ {w}"));
                    }
                    if let Some(s) = shown {
                        ui.label(egui::RichText::new(readout(s, out, !shown_exact)).strong());
                    }
                    ui.label(equilibrium_line(&out.equilibrium, out.mass.displacement_t));
                    // When the condition capsizes anyway, the line above has said so.
                    let resting = state.grab.is_none() && state.anim.is_none();
                    let capsizes = matches!(out.equilibrium, Equilibrium::Capsizes { .. });
                    if let (true, false, Some(s)) = (resting, capsizes, shown) {
                        if let_go(&lever_curve(out), s.heel_deg) == LetGo::Capsizes {
                            let at = if s.heel_deg.abs() < 0.05 {
                                "upright".to_string()
                            } else {
                                let side = if s.heel_deg > 0.0 {
                                    "starboard"
                                } else {
                                    "port"
                                };
                                format!("at {:.1}° to {side}", s.heel_deg.abs())
                            };
                            ui.colored_label(
                                theme::WARN,
                                format!(
                                    "Let go {at}, it would not come back: the righting lever \
                                     turns it further over."
                                ),
                            );
                        }
                    }
                    ui.add_space(4.0);
                    let a = weights_table(ui, state, conditions);
                    if a != HeelViewAction::None {
                        action = a;
                    }
                });
            });
            if stale {
                ui.multiply_opacity(theme::STALE_OPACITY);
            }
            ui.label(
                egui::RichText::new(STATIC_CAPTION)
                    .size(theme::SMALL_SIZE)
                    .color(theme::TEXT_SECONDARY),
            );
            if let Some(heel) = gz_plot(ui, out, shown.map(|s| s.heel_deg)) {
                state.heel_deg = (heel * 10.0).round() / 10.0;
                state.anim = None;
                action = HeelViewAction::Changed;
            }
        });
    if let Some(views) = *drawn {
        for a in [mass_field(ui, state, &views), keys(ui, state, &views)] {
            if a != HeelViewAction::None {
                action = a;
            }
        }
    }
    action
}

/// The small field where a weight's mass is typed, at the point picked. Enter places (or edits)
/// the weight; Esc cancels.
fn mass_field(ui: &mut egui::Ui, state: &mut HeelViewState, views: &DrawnViews) -> HeelViewAction {
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
                            .hint_text("mass"),
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
                            Ok(m) if m.is_finite() && m > 0.0 => {
                                place(state, &entry, m);
                                close = true;
                                action = HeelViewAction::Changed;
                            }
                            _ => {
                                entry.error = Some("type a mass in tonnes".into());
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
fn place(state: &mut HeelViewState, entry: &PendingWeight, mass_t: f64) {
    match entry.editing.and_then(|i| state.weights.get_mut(i)) {
        Some(w) => w.mass_t = mass_t,
        None => {
            let n = state.weights.len() + 1;
            state.weights.push(WeightItem {
                name: format!("Weight {n}"),
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
    if delta != 0.0 {
        state.heel_deg = (state.heel_deg + delta).clamp(-90.0, 90.0);
        state.anim = None;
        action = HeelViewAction::Changed;
    }
    if delete {
        if let Some(i) = state.selected {
            if i < state.weights.len() {
                remove_weight(state, i);
                action = HeelViewAction::Changed;
            }
        }
        state.selected = None;
    }
    action
}

/// The trial weights as a table (typed edits) and the way into the model: "Add to this condition".
fn weights_table(
    ui: &mut egui::Ui,
    state: &mut HeelViewState,
    conditions: &[String],
) -> HeelViewAction {
    let mut action = HeelViewAction::None;
    let title = format!("Trial weights ({})", state.weights.len());
    egui::CollapsingHeader::new(title)
        .id_salt("heel_weights")
        .default_open(true)
        .show(ui, |ui| {
            if state.weights.is_empty() {
                ui.label(
                    egui::RichText::new("Click a drawing to place a weight, then type its mass.")
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
                        for h in ["", "mass t", "x m", "y m", "z m", ""] {
                            ui.label(egui::RichText::new(h).color(theme::TEXT_SECONDARY));
                        }
                        ui.end_row();
                    }
                    let selected = state.selected;
                    for (i, w) in state.weights.iter_mut().enumerate() {
                        if ui.selectable_label(selected == Some(i), &w.name).clicked() {
                            state.selected = Some(i);
                        }
                        let mut edited = ui
                            .add(
                                egui::DragValue::new(&mut w.mass_t)
                                    .speed(0.05)
                                    .range(0.001..=1.0e6),
                            )
                            .changed();
                        for v in [&mut w.x, &mut w.y, &mut w.z] {
                            edited |= ui
                                .add(
                                    egui::DragValue::new(v).speed(0.02).fixed_decimals(2),
                                )
                                .changed();
                        }
                        if ui.small_button("✕").on_hover_text("Remove this weight").clicked() {
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
                        .add_enabled(!none, egui::Button::new("Add to this condition"))
                        .on_hover_text(
                            "Append the trial weights to this loading condition (Undo takes them out)",
                        )
                        .on_disabled_hover_text("Place a weight first");
                    if button.clicked() {
                        action = HeelViewAction::AddToCondition(i);
                    }
                }
                HeelStart::Dock => {
                    let enabled = !none && !conditions.is_empty();
                    let reason = if none {
                        "Place a weight first"
                    } else {
                        "The project has no loading condition yet (Loading tab)"
                    };
                    ui.add_enabled_ui(enabled, |ui| {
                        ui.menu_button("Add to loading condition…", |ui| {
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
        let hit = at.and_then(|p| weight_at(state, views, drawing, p));
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

/// The view's out-of-date chip: it recomputes on its own, so no F5.
fn stale_chip(ui: &mut egui::Ui) {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::STALE))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("out of date — recomputing")
                    .size(theme::SMALL_SIZE)
                    .color(theme::STALE),
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
    interactive: bool,
) -> (HeelViewAction, DrawnViews) {
    let width = ui.available_width();
    let gap = 8.0;
    let half = ((width - gap) * 0.5).max(40.0);
    let height = DRAWING_HEIGHT.min(half);
    let sense = if interactive {
        egui::Sense::click_and_drag()
    } else {
        egui::Sense::hover()
    };
    // Fixed ids: an auto id moves when anything before it changes, and a drag on it ends.
    let (body_resp, profile_resp) = ui
        .horizontal(|ui| {
            let size = egui::vec2(half, height);
            let (b, _) = ui.allocate_exact_size(size, egui::Sense::hover());
            ui.add_space(gap);
            let (p, _) = ui.allocate_exact_size(size, egui::Sense::hover());
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
            lever_m: s.lever_m,
            displacement_t: s.displacement_t,
        }
    });
    let painter = ui.painter_at(body_rect);
    draw::paint_body_plan(
        &painter,
        body_rect,
        &body_view,
        &draw::BodyPlanScene {
            stations: &stations,
            section: &section,
            forces,
            weights: &weights_body,
        },
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
    let caption = |ui: &mut egui::Ui, text: String| {
        ui.allocate_ui_with_layout(
            egui::vec2(half, 0.0),
            egui::Layout::top_down(egui::Align::Center),
            |ui| {
                ui.set_width(half);
                ui.add(egui::Label::new(small(text)).wrap());
            },
        );
    };
    ui.horizontal_top(|ui| {
        let body_caption = match (shown, section_x) {
            (Some(_), Some(x)) => {
                format!("Looking forward · B and G projected onto the section at x = {x:.2} m")
            }
            (None, Some(x)) => format!("Looking forward · section at x = {x:.2} m"),
            _ => "Looking forward".to_string(),
        };
        caption(ui, body_caption);
        ui.add_space(gap);
        caption(ui, format!("From starboard{profile_note}"));
    });
    (action, views)
}

/// The condition's GZ curve (G on the centreline) and heeling arm against heel, the heel shown and
/// the equilibrium marked; returns the heel clicked.
fn gz_plot(ui: &mut egui::Ui, out: &HeelSweepOut, shown_heel: Option<f64>) -> Option<f64> {
    use crate::panels::{plot_interactive, PlotMark, PlotSeries};
    let pts = |f: fn(&HeelState) -> f64| -> Vec<(f64, f64)> {
        out.sweep
            .iter()
            .flatten()
            .map(|s| (s.heel_deg, f(s)))
            .collect()
    };
    let series = [
        PlotSeries {
            label: "GZ, G on the centreline (m)".into(),
            points: pts(|s| s.gz_m),
            dashed: false,
        },
        PlotSeries {
            label: "Heeling arm (m)".into(),
            points: pts(|s| s.heeling_arm_m),
            dashed: true,
        },
    ];
    let marks: Vec<PlotMark> = match &out.equilibrium {
        Equilibrium::Stable { state } => vec![PlotMark {
            x: state.heel_deg,
            label: "settles",
        }],
        Equilibrium::Loll { port, starboard } => vec![
            PlotMark {
                x: port.heel_deg,
                label: "loll",
            },
            PlotMark {
                x: starboard.heel_deg,
                label: "loll",
            },
        ],
        _ => Vec::new(),
    };
    plot_interactive(
        ui,
        "Righting arm against heel (°), free to trim",
        &series,
        shown_heel,
        &marks,
        Some((-90.0, 90.0)),
    )
}

/// Draw the view's header: the views as tabs and the close button.
pub fn header(ui: &mut egui::Ui) -> HeelViewAction {
    let mut action = HeelViewAction::None;
    ui.horizontal(|ui| {
        for other in AnalysisView::ALL {
            let shown = other == AnalysisView::Heel;
            if ui.selectable_label(shown, other.tab_label(shown)).clicked() && !shown {
                action = HeelViewAction::Open(other);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("✕")
                .on_hover_text("Close Heel and righting arm")
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
    fn let_go_where_nothing_turns_it_it_stays() {
        let c = curve(|h| h * (400.0 - h * h));
        assert_eq!(let_go(&c, 0.0), LetGo::Stays);
        assert_eq!(let_go(&c, 95.0), LetGo::Stays, "outside the sweep");
    }
}
