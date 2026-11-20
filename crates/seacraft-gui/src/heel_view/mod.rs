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

/// The number of stations the drawings cut.
pub const OUTLINE_STATIONS: usize = 25;

/// The hull refined for the drawings, and its station outlines: keyed by the geometry alone, so
/// the hull is drawn even with no KG.
#[derive(Clone)]
pub struct HeelHull {
    pub hull: Arc<Hull>,
    pub outlines: Arc<geometry::HullOutlines>,
}

/// Refine the hull (or take the one given) and cut its outlines (the outline job).
pub fn compute_hull(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    hull: Option<Arc<Hull>>,
) -> HeelHull {
    let hull = hull.unwrap_or_else(|| Arc::new(Hull::new(surface, subdivision)));
    let outlines = Arc::new(geometry::HullOutlines::new(&hull, OUTLINE_STATIONS));
    HeelHull { hull, outlines }
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

/// The readout of the state drawn, in plain words.
pub fn readout(s: &HeelState, out: &HeelSweepOut, approx: bool) -> String {
    let keel = out.hull.keel;
    let mut parts = vec![
        heel_words(s.heel_deg, approx),
        format!(
            "righting lever {:+.3} m = GZ {:+.3} m − heeling arm {:+.3} m",
            s.lever_m, s.gz_m, s.heeling_arm_m
        ),
    ];
    if let (Some(d), Some(t)) = (s.draft, s.trim_m) {
        parts.push(format!("draft {d:.3} m"));
        parts.push(format!("trim {t:+.3} m"));
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
            Some(t) => format!("Settles upright, trim {t:+.3} m"),
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
                words.push_str(&format!(", trim {t:+.3} m"));
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
        stale,
        busy,
        failure,
        kg_note,
        conditions,
    } = p;
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
        let slider = egui::Slider::new(&mut state.heel_deg, -90.0..=90.0)
            .suffix("°")
            .fixed_decimals(1);
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
            drawings(
                ui,
                state,
                o,
                &frame,
                section,
                None,
                "Upright at the dock draft",
            );
        }
        return action;
    }
    let Some(out) = sweep else {
        return action;
    };
    if let Equilibrium::CannotFloat = out.equilibrium {
        ui.colored_label(
            theme::FAIL,
            equilibrium_line(&out.equilibrium, out.mass.displacement_t),
        );
        return action;
    }
    egui::ScrollArea::vertical()
        .id_salt("heel_view_body")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            if stale {
                stale_chip(ui);
                ui.multiply_opacity(theme::STALE_OPACITY);
            }
            for w in &out.warnings {
                ui.colored_label(theme::WARN, format!("⚠ {w}"));
            }
            if let (Some(s), Some(o)) = (shown, outlines) {
                let frame = geometry::EarthFrame::from_plane(&s.plane);
                let trim = s
                    .trim_m
                    .map_or(String::new(), |t| format!(" · trim {t:+.3} m"));
                drawings(ui, state, o, &frame, section, Some(s), &trim);
                ui.label(egui::RichText::new(readout(s, out, !shown_exact)).strong());
            }
            ui.label(equilibrium_line(&out.equilibrium, out.mass.displacement_t));
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
fn drawings(
    ui: &mut egui::Ui,
    state: &HeelViewState,
    outlines: &geometry::HullOutlines,
    frame: &geometry::EarthFrame,
    section: Option<&[Vec<Vec3>]>,
    shown: Option<&HeelState>,
    profile_note: &str,
) {
    let width = ui.available_width();
    let gap = 8.0;
    let half = ((width - gap) * 0.5).max(40.0);
    let height = DRAWING_HEIGHT.min(half);
    let (body_rect, profile_rect) = ui
        .horizontal(|ui| {
            let (b, _) = ui.allocate_exact_size(egui::vec2(half, height), egui::Sense::hover());
            ui.add_space(gap);
            let (p, _) = ui.allocate_exact_size(egui::vec2(half, height), egui::Sense::hover());
            (b, p)
        })
        .inner;
    let (body_view, profile_view) = draw::fit_views(body_rect, profile_rect, outlines, frame);
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
    let section_x = state.section.or(shown.map(|s| s.b.x));
    let small = |t: String| {
        egui::RichText::new(t)
            .size(theme::SMALL_SIZE)
            .color(theme::TEXT_SECONDARY)
    };
    ui.horizontal(|ui| {
        let body_caption = match (shown, section_x) {
            (Some(_), Some(x)) => {
                format!("Looking forward · B and G projected onto the section at x = {x:.2} m")
            }
            (None, Some(x)) => format!("Looking forward · section at x = {x:.2} m"),
            _ => "Looking forward".to_string(),
        };
        ui.add_sized([half, 14.0], egui::Label::new(small(body_caption)));
        ui.add_space(gap);
        ui.add_sized(
            [half, 14.0],
            egui::Label::new(small(format!("From starboard{profile_note}"))),
        );
    });
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
