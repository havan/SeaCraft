//! The Model menu: its items, the Transform and Optimise dialogs, the
//! targets of the topology tools and the axis lock of a point drag.
//!
//! Everything here draws or decides; nothing mutates the model. The menu and
//! the dialogs return what the user asked for and [`crate::App`] applies it
//! through [`crate::commands`] (one undo step per edit).
//!
//! Until a real selection model exists (round 6), the topology tools act on
//! what a click picks: the selected control point plus the one before it
//! (Shift+click picks a second point), and the control face under the last
//! click.

use eframe::egui;
use seacraft_core::i18n::{Key, Locale, Tr};
use seacraft_core::math::Vec3;
use seacraft_core::optimize::{DesignVar, Method, OptimizeSpec, VarSpec};
use seacraft_core::transform::{Axis, MainDimensions, TransformRequest};
use seacraft_core::Surface;

use crate::new_design::{field, optional};
use crate::theme;

/// What one menu item would act on, or why it cannot (the disabled item's
/// hover text).
pub type Target<T> = Result<T, String>;

/// The Model menu's items and what each would do now.
#[derive(Debug, Clone, PartialEq)]
pub struct ModelTargets {
    pub transform: Target<()>,
    pub optimise: Target<()>,
    /// Face, first point, second point.
    pub insert_edge: Target<(usize, usize, usize)>,
    pub split_face: Target<usize>,
    /// The edge to crease, and the creased edge to smooth.
    pub crease: Target<[usize; 2]>,
    pub smooth: Target<[usize; 2]>,
    /// Select the active point's station or lengthwise line.
    pub select_station: Target<usize>,
    pub select_lengthwise: Target<usize>,
}

impl ModelTargets {
    /// The targets for a model (if any), the two picked points (first, then
    /// second), the picked face, whether an optimisation is running, and the
    /// active control point.
    pub fn compute(
        surface: Option<&Surface>,
        picked: Option<[usize; 2]>,
        face: Option<usize>,
        optimising: bool,
        active: Option<usize>,
    ) -> Self {
        Self::compute_tr(
            surface,
            picked,
            face,
            optimising,
            active,
            Tr::new(Locale::En),
        )
    }

    /// Localized variant of [`ModelTargets::compute`].
    pub fn compute_tr(
        surface: Option<&Surface>,
        picked: Option<[usize; 2]>,
        face: Option<usize>,
        optimising: bool,
        active: Option<usize>,
        tr: Tr,
    ) -> Self {
        let Some(surface) = surface else {
            let no_model = tr.s(Key::ModelErrNoModel);
            return Self {
                transform: Err(no_model.to_owned()),
                optimise: Err(no_model.to_owned()),
                insert_edge: Err(no_model.to_owned()),
                split_face: Err(no_model.to_owned()),
                crease: Err(no_model.to_owned()),
                smooth: Err(no_model.to_owned()),
                select_station: Err(no_model.to_owned()),
                select_lengthwise: Err(no_model.to_owned()),
            };
        };
        let select_target = active.ok_or_else(|| tr.s(Key::ModelErrPickPoint).to_owned());
        Self {
            transform: Ok(()),
            optimise: if optimising {
                Err(tr.s(Key::ModelErrOptimising).to_owned())
            } else {
                Ok(())
            },
            insert_edge: insert_edge_target_tr(surface, picked, face, tr),
            split_face: split_face_target_tr(surface, face, tr),
            crease: edge_target_tr(surface, picked, true, tr),
            smooth: edge_target_tr(surface, picked, false, tr),
            select_station: select_target.clone(),
            select_lengthwise: select_target,
        }
    }
}

/// The face an edge between the two picked points would split: the picked
/// face when both points are corners of it (and not neighbours), otherwise
/// the first face that qualifies.
pub fn insert_edge_target(
    surface: &Surface,
    picked: Option<[usize; 2]>,
    face: Option<usize>,
) -> Target<(usize, usize, usize)> {
    insert_edge_target_tr(surface, picked, face, Tr::new(Locale::En))
}

pub fn insert_edge_target_tr(
    surface: &Surface,
    picked: Option<[usize; 2]>,
    face: Option<usize>,
    tr: Tr,
) -> Target<(usize, usize, usize)> {
    let [a, b] = picked.ok_or_else(|| tr.s(Key::ModelErrPickTwo).to_owned())?;
    let a_s = a.to_string();
    let b_s = b.to_string();
    let key = seacraft_core::mesh::ordered(a, b);
    if surface.edges.iter().any(|e| e.key() == key) {
        return Err(tr.fmt(Key::ModelErrAlreadyEdge, &[("a", &a_s), ("b", &b_s)]));
    }
    let splits = |f: &Vec<usize>| {
        let n = f.len();
        let (Some(i), Some(j)) = (
            f.iter().position(|&v| v == a),
            f.iter().position(|&v| v == b),
        ) else {
            return false;
        };
        let steps = (j + n - i) % n;
        n >= 4 && steps != 1 && steps != n - 1
    };
    let picked_face = face.filter(|&f| surface.faces.get(f).is_some_and(splits));
    picked_face
        .or_else(|| surface.faces.iter().position(splits))
        .map(|f| (f, a, b))
        .ok_or_else(|| tr.fmt(Key::ModelErrNotCorners, &[("a", &a_s), ("b", &b_s)]))
}

/// The picked face, if it can be split.
pub fn split_face_target(surface: &Surface, face: Option<usize>) -> Target<usize> {
    split_face_target_tr(surface, face, Tr::new(Locale::En))
}

pub fn split_face_target_tr(surface: &Surface, face: Option<usize>, tr: Tr) -> Target<usize> {
    let face = face.ok_or_else(|| tr.s(Key::ModelErrClickFace).to_owned())?;
    match surface.faces.get(face) {
        Some(f) if f.len() >= 3 => Ok(face),
        Some(_) => {
            let f_s = face.to_string();
            Err(tr.fmt(Key::ModelErrFaceFewPoints, &[("face", &f_s)]))
        }
        None => Err(tr.s(Key::ModelErrFaceGone).to_owned()),
    }
}

/// The control edge between the two picked points, when creasing it
/// (`crease`) or smoothing it would change it.
pub fn edge_target(
    surface: &Surface,
    picked: Option<[usize; 2]>,
    crease: bool,
) -> Target<[usize; 2]> {
    edge_target_tr(surface, picked, crease, Tr::new(Locale::En))
}

pub fn edge_target_tr(
    surface: &Surface,
    picked: Option<[usize; 2]>,
    crease: bool,
    tr: Tr,
) -> Target<[usize; 2]> {
    let [a, b] = picked.ok_or_else(|| tr.s(Key::ModelErrPickTwo).to_owned())?;
    let a_s = a.to_string();
    let b_s = b.to_string();
    let key = seacraft_core::mesh::ordered(a, b);
    let edge = surface
        .edges
        .iter()
        .find(|e| e.key() == key)
        .ok_or_else(|| tr.fmt(Key::ModelErrNotJoined, &[("a", &a_s), ("b", &b_s)]))?;
    match (edge.crease, crease) {
        (true, true) => Err(tr.fmt(Key::ModelErrAlreadyCrease, &[("a", &a_s), ("b", &b_s)])),
        (false, false) => Err(tr.fmt(Key::ModelErrNotCrease, &[("a", &a_s), ("b", &b_s)])),
        _ => Ok([a, b]),
    }
}

/// What the Model menu asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ModelAction {
    None,
    Transform,
    Optimise,
    InsertEdge { face: usize, a: usize, b: usize },
    SplitFace(usize),
    Crease { edge: [usize; 2], crease: bool },
    SelectLine(seacraft_core::netlines::LineKind),
    ReloadPresets,
    CreateExamplePresets,
}

/// One menu item: enabled with a hover description, or disabled with the
/// reason. Returns whether it was clicked.
fn item<T>(
    ui: &mut egui::Ui,
    label: &str,
    target: &Target<T>,
    hover: impl FnOnce(&T) -> String,
) -> bool {
    match target {
        Ok(t) => {
            let clicked = ui.button(label).on_hover_text(hover(t)).clicked();
            if clicked {
                ui.close();
            }
            clicked
        }
        Err(reason) => {
            ui.add_enabled(false, egui::Button::new(label))
                .on_disabled_hover_text(reason);
            false
        }
    }
}

/// The Model menu's contents.
pub fn menu_body(ui: &mut egui::Ui, t: &ModelTargets, tr: Tr) -> ModelAction {
    let mut action = ModelAction::None;
    if item(ui, tr.s(Key::ModelMenuTransform), &t.transform, |_| {
        tr.s(Key::ModelMenuTransformTip).to_owned()
    }) {
        action = ModelAction::Transform;
    }
    if item(ui, tr.s(Key::ModelMenuOptimise), &t.optimise, |_| {
        tr.s(Key::ModelMenuOptimiseTip).to_owned()
    }) {
        action = ModelAction::Optimise;
    }
    ui.separator();
    if item(
        ui,
        tr.s(Key::ModelMenuInsertEdge),
        &t.insert_edge,
        |&(f, a, b)| {
            let (f_s, a_s, b_s) = (f.to_string(), a.to_string(), b.to_string());
            tr.fmt(
                Key::ModelMenuInsertEdgeTip,
                &[("face", &f_s), ("a", &a_s), ("b", &b_s)],
            )
        },
    ) {
        if let Ok((face, a, b)) = t.insert_edge {
            action = ModelAction::InsertEdge { face, a, b };
        }
    }
    if item(ui, tr.s(Key::ModelMenuSplitFace), &t.split_face, |f| {
        let f_s = f.to_string();
        tr.fmt(Key::ModelMenuSplitFaceTip, &[("face", &f_s)])
    }) {
        if let Ok(face) = t.split_face {
            action = ModelAction::SplitFace(face);
        }
    }
    if item(ui, tr.s(Key::ModelMenuCrease), &t.crease, |[a, b]| {
        let (a_s, b_s) = (a.to_string(), b.to_string());
        tr.fmt(Key::ModelMenuCreaseTip, &[("a", &a_s), ("b", &b_s)])
    }) {
        if let Ok(edge) = t.crease {
            action = ModelAction::Crease { edge, crease: true };
        }
    }
    if item(ui, tr.s(Key::ModelMenuRemoveCrease), &t.smooth, |[a, b]| {
        let (a_s, b_s) = (a.to_string(), b.to_string());
        tr.fmt(Key::ModelMenuRemoveCreaseTip, &[("a", &a_s), ("b", &b_s)])
    }) {
        if let Ok(edge) = t.smooth {
            action = ModelAction::Crease {
                edge,
                crease: false,
            };
        }
    }
    ui.separator();
    if item(
        ui,
        tr.s(Key::ModelMenuSelectStation),
        &t.select_station,
        |p| {
            let p_s = p.to_string();
            tr.fmt(Key::ModelMenuSelectStationTip, &[("point", &p_s)])
        },
    ) {
        action = ModelAction::SelectLine(seacraft_core::netlines::LineKind::Transverse);
    }
    if item(
        ui,
        tr.s(Key::ModelMenuSelectLengthwise),
        &t.select_lengthwise,
        |p| {
            let p_s = p.to_string();
            tr.fmt(Key::ModelMenuSelectLengthwiseTip, &[("point", &p_s)])
        },
    ) {
        action = ModelAction::SelectLine(seacraft_core::netlines::LineKind::Lengthwise);
    }
    ui.separator();
    if ui
        .button(tr.s(Key::ModelMenuReloadPresets))
        .on_hover_text(tr.s(Key::ModelMenuReloadPresetsTip))
        .clicked()
    {
        action = ModelAction::ReloadPresets;
        ui.close();
    }
    if ui
        .button(tr.s(Key::ModelMenuCreatePresets))
        .on_hover_text(tr.s(Key::ModelMenuCreatePresetsTip))
        .clicked()
    {
        action = ModelAction::CreateExamplePresets;
        ui.close();
    }
    ui.separator();
    ui.label(
        egui::RichText::new(tr.s(Key::ModelMenuHint))
            .small()
            .color(theme::TEXT_MUTED),
    );
    action
}

// ---- axis-locked drag ------------------------------------------------------

/// The world axis held down (X, Y or Z, no Ctrl/Cmd) in this frame's input.
pub fn held_axis(input: &egui::InputState) -> Option<Axis> {
    if input.modifiers.command || input.modifiers.ctrl {
        return None;
    }
    [
        (egui::Key::X, Axis::X),
        (egui::Key::Y, Axis::Y),
        (egui::Key::Z, Axis::Z),
    ]
    .into_iter()
    .find(|(key, _)| input.key_down(*key))
    .map(|(_, axis)| axis)
}

/// The unit vector of a world axis.
pub fn axis_vector(axis: Axis) -> Vec3 {
    match axis {
        Axis::X => Vec3::new(1.0, 0.0, 0.0),
        Axis::Y => Vec3::new(0.0, 1.0, 0.0),
        Axis::Z => Vec3::new(0.0, 0.0, 1.0),
    }
}

/// A free drag target held to the line through `original` along `axis`:
/// only that coordinate follows the cursor, the other two stay where the
/// point was grabbed.
pub fn lock_to_axis(original: Vec3, free: Vec3, axis: Axis) -> Vec3 {
    let mut out = original;
    match axis {
        Axis::X => out.x = free.x,
        Axis::Y => out.y = free.y,
        Axis::Z => out.z = free.z,
    }
    out
}

/// The axis's name in captions.
pub fn axis_name(axis: Axis) -> &'static str {
    match axis {
        Axis::X => "x",
        Axis::Y => "y",
        Axis::Z => "z",
    }
}

/// The axis's guide colour.
pub fn axis_colour(axis: Axis) -> egui::Color32 {
    match axis {
        Axis::X => theme::AXIS_X,
        Axis::Y => theme::AXIS_Y,
        Axis::Z => theme::AXIS_Z,
    }
}

// ---- dialogs ---------------------------------------------------------------

/// What a dialog asked for.
#[derive(Debug, Clone)]
pub enum DialogAction<T> {
    None,
    Cancel,
    Apply(T),
}

/// The Transform dialog's fields, kept while it is open.
#[derive(Debug, Clone)]
pub struct TransformDialog {
    /// Fit targets; `None` leaves that dimension alone.
    pub lwl: Option<f64>,
    pub bwl: Option<f64>,
    pub draft: Option<f64>,
    /// Extra scale per axis after the fit; `None` is no scale.
    pub scale: Option<[f64; 3]>,
    pub translate: [f64; 3],
    pub mirror: Option<Axis>,
    /// The hull's dimensions when the dialog opened (the fit defaults).
    pub current: Option<MainDimensions>,
    /// Why the last Apply was refused.
    pub error: Option<String>,
}

impl TransformDialog {
    pub fn new(current: Option<MainDimensions>) -> Self {
        Self {
            lwl: None,
            bwl: None,
            draft: None,
            scale: None,
            translate: [0.0; 3],
            mirror: None,
            current,
            error: None,
        }
    }

    /// The request the fields describe.
    pub fn request(&self) -> TransformRequest {
        TransformRequest {
            lwl: self.lwl,
            bwl: self.bwl,
            draft: self.draft,
            scale: self.scale.map(|[x, y, z]| Vec3::new(x, y, z)),
            translate: Vec3::new(self.translate[0], self.translate[1], self.translate[2]),
            mirror: self.mirror,
        }
    }

    /// Why the fields cannot be applied yet.
    pub fn problem(&self) -> Option<String> {
        self.problem_tr(Tr::new(Locale::En))
    }

    /// Localized validation error for the Transform dialog.
    pub fn problem_tr(&self, tr: Tr) -> Option<String> {
        let positive = |v: Option<f64>| v.is_none_or(|v| v > 0.0 && v.is_finite());
        if !(positive(self.lwl) && positive(self.bwl) && positive(self.draft)) {
            return Some(tr.s(Key::ModelTransformErrDimsPositive).to_owned());
        }
        if self
            .scale
            .is_some_and(|s| s.iter().any(|v| !(*v > 0.0 && v.is_finite())))
        {
            return Some(tr.s(Key::ModelTransformErrScalePositive).to_owned());
        }
        if self.request().is_empty() {
            return Some(tr.s(Key::ModelTransformErrEmpty).to_owned());
        }
        None
    }
}

/// Draw the Transform dialog (modal).
pub fn transform_dialog(
    ctx: &egui::Context,
    state: &mut TransformDialog,
    tr: Tr,
) -> DialogAction<TransformRequest> {
    let mut action = DialogAction::None;
    let modal = egui::Modal::new(egui::Id::unique("transform_dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(
            egui::RichText::new(tr.s(Key::ModelTransformTitle))
                .heading()
                .color(theme::TEXT_PRIMARY),
        );
        ui.label(
            egui::RichText::new(tr.s(Key::ModelTransformSubtitle))
                .small()
                .color(theme::TEXT_MUTED),
        );
        ui.add_space(4.0);
        let cur = state.current;
        egui::Grid::new("transform_fit")
            .num_columns(3)
            .spacing(egui::vec2(8.0, 6.0))
            .show(ui, |ui| {
                optional(
                    ui,
                    tr.s(Key::ModelTransformFitLwl),
                    &mut state.lwl,
                    cur.map_or(10.0, |c| c.lwl),
                    0.01,
                    3,
                    "m",
                    tr,
                );
                ui.end_row();
                optional(
                    ui,
                    tr.s(Key::ModelTransformFitBwl),
                    &mut state.bwl,
                    cur.map_or(3.0, |c| c.bwl),
                    0.01,
                    3,
                    "m",
                    tr,
                );
                ui.end_row();
                optional(
                    ui,
                    tr.s(Key::ModelTransformFitDraft),
                    &mut state.draft,
                    cur.map_or(0.5, |c| c.draft),
                    0.005,
                    3,
                    "m",
                    tr,
                );
                ui.end_row();
            });
        if let Some(c) = cur {
            let lwl = format!("{:.3}", c.lwl);
            let bwl = format!("{:.3}", c.bwl);
            let draft = format!("{:.3}", c.draft);
            ui.label(
                egui::RichText::new(tr.fmt(
                    Key::ModelTransformNow,
                    &[("lwl", &lwl), ("bwl", &bwl), ("draft", &draft)],
                ))
                .small()
                .color(theme::TEXT_SECONDARY),
            );
        }
        ui.separator();
        egui::Grid::new("transform_move")
            .num_columns(4)
            .spacing(egui::vec2(8.0, 6.0))
            .show(ui, |ui| {
                let mut on = state.scale.is_some();
                if ui
                    .checkbox(
                        &mut on,
                        egui::RichText::new(tr.s(Key::ModelTransformScale))
                            .color(theme::TEXT_SECONDARY),
                    )
                    .changed()
                {
                    state.scale = on.then_some([1.0; 3]);
                }
                match state.scale.as_mut() {
                    Some(s) => {
                        for v in s.iter_mut() {
                            ui.add(
                                theme::num(v)
                                    .speed(0.005)
                                    .range(0.001..=1000.0)
                                    .fixed_decimals(3),
                            );
                        }
                    }
                    None => {
                        ui.label(egui::RichText::new("×1").color(theme::TEXT_MUTED));
                        ui.label("");
                        ui.label("");
                    }
                }
                ui.end_row();
                ui.label(
                    egui::RichText::new(tr.s(Key::ModelTransformMove)).color(theme::TEXT_SECONDARY),
                );
                for v in state.translate.iter_mut() {
                    ui.add(theme::num(v).speed(0.01).fixed_decimals(3));
                }
                ui.end_row();
                ui.label("");
                for name in ["x", "y", "z"] {
                    ui.label(egui::RichText::new(name).small().color(theme::TEXT_MUTED));
                }
                ui.end_row();
            });
        ui.horizontal(|ui| {
            ui.label(
                egui::RichText::new(tr.s(Key::ModelTransformMirror)).color(theme::TEXT_SECONDARY),
            );
            ui.radio_value(&mut state.mirror, None, tr.s(Key::CommonNone));
            ui.radio_value(
                &mut state.mirror,
                Some(Axis::X),
                tr.s(Key::ModelTransformMirrorX),
            );
            ui.radio_value(&mut state.mirror, Some(Axis::Y), "y");
            ui.radio_value(&mut state.mirror, Some(Axis::Z), "z");
        });
        ui.separator();
        ui.horizontal(|ui| {
            let problem = state.problem_tr(tr);
            if let Some(e) = state.error.as_ref().or(problem.as_ref()) {
                ui.colored_label(theme::FAIL, format!("⚠ {e}"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(problem.is_none(), egui::Button::new(tr.s(Key::CommonApply)))
                    .clicked()
                {
                    action = DialogAction::Apply(state.request());
                }
                if ui.button(tr.s(Key::CommonCancel)).clicked() {
                    action = DialogAction::Cancel;
                }
            });
        });
    });
    if matches!(action, DialogAction::None)
        && modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        action = DialogAction::Cancel;
    }
    action
}

/// The Optimise dialog's fields and the last run's outcome.
#[derive(Debug, Clone)]
pub struct OptimiseDialog {
    pub speed_kn: f64,
    /// Cp, LCB and L/B, in [`DesignVar::ALL`] order.
    pub vars: [bool; 3],
    pub method: Method,
    /// Allowed displacement change, percent.
    pub tolerance_pct: f64,
    pub max_evaluations: usize,
    /// Subdivision level each candidate is measured at.
    pub level: u8,
    pub kg: Option<f64>,
    pub min_gm: Option<f64>,
    /// Why the last run failed.
    pub error: Option<String>,
    /// The last run's summary.
    pub result: Option<String>,
}

impl OptimiseDialog {
    /// Defaults: the design speed, Cp and LCB free, Holtrop or Savitsky by
    /// speed, 1 % on displacement, 60 evaluations.
    pub fn new(speed_kn: f64, level: u8, kg: Option<f64>) -> Self {
        Self {
            speed_kn,
            vars: [true, true, false],
            method: Method::Auto,
            tolerance_pct: 1.0,
            max_evaluations: 60,
            level,
            kg,
            min_gm: None,
            error: None,
            result: None,
        }
    }

    /// The spec the fields describe, or why they do not make one.
    pub fn spec(&self) -> Result<OptimizeSpec, String> {
        self.spec_tr(Tr::new(Locale::En))
    }

    /// Localized validation of the Optimise dialog specification.
    pub fn spec_tr(&self, tr: Tr) -> Result<OptimizeSpec, String> {
        if !(self.speed_kn > 0.0 && self.speed_kn.is_finite()) {
            return Err(tr.s(Key::ModelOptimiseErrSpeedPositive).to_owned());
        }
        let vars: Vec<VarSpec> = DesignVar::ALL
            .iter()
            .zip(self.vars)
            .filter(|(_, on)| *on)
            .map(|(v, _)| VarSpec::new(*v))
            .collect();
        if vars.is_empty() {
            return Err(tr.s(Key::ModelOptimiseErrChooseVar).to_owned());
        }
        if self.min_gm.is_some() && self.kg.is_none() {
            return Err(tr.s(Key::ModelOptimiseErrGmNeedsKg).to_owned());
        }
        let mut spec = OptimizeSpec::new(self.speed_kn);
        spec.vars = vars;
        spec.method = self.method;
        spec.hold_displacement_tol = self.tolerance_pct / 100.0;
        spec.max_evaluations = self.max_evaluations.max(1);
        spec.kg = self.kg;
        spec.min_gm = self.min_gm;
        Ok(spec)
    }
}

/// What the Optimise window asked for.
#[derive(Debug, Clone)]
pub enum OptimiseAction {
    None,
    /// Close the window (stopping a run).
    Close,
    /// Start a run with this spec at this level.
    Run(Box<OptimizeSpec>, u8),
    /// Stop the run; its result is discarded.
    Stop,
}

/// Draw the Optimise window. `running` shows the run's progress instead of
/// the Run button.
pub fn optimise_dialog(
    ctx: &egui::Context,
    state: &mut OptimiseDialog,
    running: bool,
    tr: Tr,
) -> OptimiseAction {
    let mut action = OptimiseAction::None;
    let mut open = true;
    egui::Window::new(tr.s(Key::ModelOptimiseTitle))
        .id(egui::Id::unique("optimise_dialog"))
        .open(&mut open)
        .collapsible(false)
        .resizable(false)
        .default_width(400.0)
        .show(ctx, |ui| {
            ui.label(
                egui::RichText::new(tr.s(Key::ModelOptimiseSubtitle))
                    .small()
                    .color(theme::TEXT_MUTED),
            );
            ui.add_enabled_ui(!running, |ui| {
                egui::Grid::new("optimise_fields")
                    .num_columns(3)
                    .spacing(egui::vec2(8.0, 6.0))
                    .show(ui, |ui| {
                        field(
                            ui,
                            tr.s(Key::ModelOptimiseSpeed),
                            theme::num(&mut state.speed_kn)
                                .speed(0.1)
                                .range(0.1..=80.0)
                                .fixed_decimals(1),
                            "kn",
                        );
                        ui.end_row();
                        ui.label(
                            egui::RichText::new(tr.s(Key::ModelOptimiseVary))
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.horizontal(|ui| {
                            ui.checkbox(&mut state.vars[0], "Cp");
                            ui.checkbox(&mut state.vars[1], "LCB");
                            ui.checkbox(&mut state.vars[2], "L/B");
                        });
                        ui.label("");
                        ui.end_row();
                        ui.label(
                            egui::RichText::new(tr.s(Key::ModelOptimiseMethod))
                                .color(theme::TEXT_SECONDARY),
                        );
                        ui.horizontal(|ui| {
                            ui.radio_value(&mut state.method, Method::Auto, tr.s(Key::CommonAuto));
                            ui.radio_value(&mut state.method, Method::Holtrop, "Holtrop");
                            ui.radio_value(&mut state.method, Method::Savitsky, "Savitsky");
                        });
                        ui.label("");
                        ui.end_row();
                        field(
                            ui,
                            tr.s(Key::ModelOptimiseDisplacementWithin),
                            theme::num(&mut state.tolerance_pct)
                                .speed(0.05)
                                .range(0.05..=10.0)
                                .fixed_decimals(2),
                            "%",
                        );
                        ui.end_row();
                        field(
                            ui,
                            tr.s(Key::ModelOptimiseEvaluations),
                            theme::num(&mut state.max_evaluations)
                                .speed(1.0)
                                .range(1..=500),
                            tr.s(Key::ModelOptimiseMax),
                        );
                        ui.end_row();
                        field(
                            ui,
                            tr.s(Key::ModelOptimiseMeasureLevel),
                            theme::num(&mut state.level).speed(0.05).range(0..=4),
                            "",
                        );
                        ui.end_row();
                        optional(ui, "KG", &mut state.kg, 1.0, 0.01, 2, "m", tr);
                        ui.end_row();
                        optional(
                            ui,
                            tr.s(Key::ModelOptimiseMinGm),
                            &mut state.min_gm,
                            0.35,
                            0.01,
                            2,
                            "m",
                            tr,
                        );
                        ui.end_row();
                    });
            });
            ui.separator();
            if let Some(r) = &state.result {
                ui.colored_label(theme::OK, r);
            }
            let problem = state.spec_tr(tr).err();
            if let Some(e) = state.error.as_ref().or(problem.as_ref()) {
                ui.colored_label(theme::FAIL, format!("⚠ {e}"));
            }
            ui.horizontal(|ui| {
                if running {
                    ui.spinner();
                    ui.label(
                        egui::RichText::new(tr.s(Key::ModelOptimiseRunning))
                            .color(theme::TEXT_SECONDARY),
                    );
                    if ui.button(tr.s(Key::CommonStop)).clicked() {
                        action = OptimiseAction::Stop;
                    }
                } else {
                    let spec = state.spec_tr(tr);
                    if ui
                        .add_enabled(spec.is_ok(), egui::Button::new(tr.s(Key::CommonRun)))
                        .clicked()
                    {
                        if let Ok(spec) = spec {
                            action = OptimiseAction::Run(Box::new(spec), state.level);
                        }
                    }
                    if ui.button(tr.s(Key::CommonClose)).clicked() {
                        action = OptimiseAction::Close;
                    }
                }
            });
        });
    if !open {
        action = OptimiseAction::Close;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    fn quad() -> Surface {
        let mut s = Surface::default();
        for (x, y) in [(0.0, 0.0), (1.0, 0.0), (1.0, 1.0), (0.0, 1.0)] {
            s.add_point(Vec3::new(x, y, 0.0));
        }
        if s.layers.is_empty() {
            s.layers.push(seacraft_core::mesh::Layer::default());
        }
        s.add_face(vec![0, 1, 2, 3], 0);
        s.rebuild_edges();
        s
    }

    #[test]
    fn targets_name_what_they_act_on_or_why_not() {
        let s = quad();
        let t = ModelTargets::compute(Some(&s), Some([0, 2]), Some(0), false, Some(2));
        assert_eq!(t.insert_edge, Ok((0, 0, 2)));
        assert_eq!(t.split_face, Ok(0));
        assert!(t.crease.as_ref().unwrap_err().contains("not joined"));
        assert_eq!(t.select_station, Ok(2));
        assert_eq!(t.select_lengthwise, Ok(2));
        let t = ModelTargets::compute(Some(&s), Some([0, 1]), None, true, None);
        assert!(t.insert_edge.unwrap_err().contains("already share an edge"));
        assert_eq!(t.crease, Ok([0, 1]));
        assert!(t.smooth.unwrap_err().contains("not a crease"));
        assert!(t.split_face.unwrap_err().contains("Click a face"));
        assert!(t.optimise.is_err() && t.transform.is_ok());
        assert!(t
            .select_station
            .unwrap_err()
            .contains("Select a control point"));
        assert!(t
            .select_lengthwise
            .unwrap_err()
            .contains("Select a control point"));
        let t = ModelTargets::compute(Some(&s), None, None, false, None);
        assert!(t.insert_edge.unwrap_err().contains("Shift+click"));
        let t = ModelTargets::compute(None, Some([0, 2]), Some(0), false, Some(2));
        assert!(t.transform.unwrap_err().contains("No model"));
        assert!(t.select_station.unwrap_err().contains("No model"));
    }

    #[test]
    fn a_locked_drag_keeps_the_other_coordinates() {
        let o = Vec3::new(1.0, 2.0, 3.0);
        let free = Vec3::new(4.0, 5.0, 6.0);
        assert_eq!(lock_to_axis(o, free, Axis::X), Vec3::new(4.0, 2.0, 3.0));
        assert_eq!(lock_to_axis(o, free, Axis::Y), Vec3::new(1.0, 5.0, 3.0));
        assert_eq!(lock_to_axis(o, free, Axis::Z), Vec3::new(1.0, 2.0, 6.0));
    }

    #[test]
    fn dialog_fields_make_requests() {
        let mut d = TransformDialog::new(None);
        assert!(d.problem().unwrap().contains("nothing to do"));
        d.translate = [0.0, 0.0, 0.5];
        assert!(d.problem().is_none());
        assert_eq!(d.request().translate, Vec3::new(0.0, 0.0, 0.5));
        d.scale = Some([1.0, 0.0, 1.0]);
        assert!(d.problem().unwrap().contains("scale"));

        let mut o = OptimiseDialog::new(8.0, 2, None);
        let spec = o.spec().unwrap();
        assert_eq!(spec.vars.len(), 2);
        assert_eq!(spec.max_evaluations, 60);
        o.vars = [false; 3];
        assert!(o.spec().is_err());
        o.vars = [false, false, true];
        o.min_gm = Some(0.3);
        assert!(o.spec().unwrap_err().contains("KG"));
        o.kg = Some(1.0);
        assert_eq!(o.spec().unwrap().vars[0].var, DesignVar::LbRatioScale);
    }
}
