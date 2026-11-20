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
use eframe::egui;
use seacraft_core::heel::{self, Equilibrium, HeelState, MassCondition};
use seacraft_core::hydro::{Hull, HydroOptions};
use seacraft_core::loading::{LoadingCondition, WeightItem};
use seacraft_core::math::Vec3;
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_core::Surface;

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
    /// "Add to loading condition…" with the bare hull as start: the menu is open.
    PickConditionForAdd,
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
    /// The hull it was measured on, and its station outlines for the drawings.
    pub hull: Arc<Hull>,
    pub outlines: Arc<geometry::HullOutlines>,
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

/// The number of stations the drawings cut.
pub const OUTLINE_STATIONS: usize = 25;

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
    let outlines = Arc::new(geometry::HullOutlines::new(&hull, OUTLINE_STATIONS));
    Ok(HeelSweepOut {
        start,
        mass,
        sweep,
        equilibrium,
        upright_gm,
        warnings,
        hull,
        outlines,
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
