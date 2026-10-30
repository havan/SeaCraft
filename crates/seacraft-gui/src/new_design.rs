//! Starting a design: the start card shown with no model, the *New design*
//! dialog and the *Project settings* dialog.
//!
//! The dialogs keep their own editing state and return what the user asked
//! for; [`crate::App`] applies it after the dialog has been drawn, so
//! *Cancel* leaves the model and the project exactly as they were.
//!
//! The New design preview runs the parametric generator on a worker thread
//! (a spec with an LCB target takes seconds; a plain one about 50 ms), 250 ms
//! after the last edit, and draws the generated control net in profile and
//! body plan with the painter. An example hull needs no worker: its net is a
//! frozen table, so the preview is built the moment it is picked.

use std::sync::mpsc::{Receiver, TryRecvError};
use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_core::examples::Example;
use seacraft_core::parametric::{GenerateReport, HullKind, HullSpec};
use seacraft_core::project::{Project, Requirements};
use seacraft_core::Surface;

use crate::theme;

/// Edits settle this long before the preview is regenerated.
pub const PREVIEW_DEBOUNCE: Duration = Duration::from_millis(250);
/// Sea water, t/m³, for the Δ ⇄ Cb link of a design that has no project yet.
pub const SEA_WATER_T_M3: f64 = 1.025;
/// Deviation from a target shown as on target (± per cent).
pub const TARGET_TOLERANCE_PCT: f64 = 2.0;

// ---- arithmetic ---------------------------------------------------------

/// `Cb = Δ / (ρ · L · B · T)`, with Δ in tonnes and ρ in t/m³.
pub fn cb_from_displacement(displacement_t: f64, rho_t_m3: f64, l: f64, b: f64, t: f64) -> f64 {
    let box_t = rho_t_m3 * l * b * t;
    if box_t > 0.0 {
        displacement_t / box_t
    } else {
        0.0
    }
}

/// `Δ = Cb · ρ · L · B · T`, in tonnes for ρ in t/m³.
pub fn displacement_from_cb(cb: f64, rho_t_m3: f64, l: f64, b: f64, t: f64) -> f64 {
    cb * rho_t_m3 * l * b * t
}

/// Froude number `V / √(g L)` of a speed in knots on a length in metres.
pub fn froude_number(speed_kn: f64, lwl: f64) -> f64 {
    if lwl <= 0.0 {
        return 0.0;
    }
    speed_kn * 1852.0 / 3600.0 / (9.81 * lwl).sqrt()
}

/// The Performance tab's speed range for a design speed: half to one and a
/// half times it, in a round step that gives about ten speeds.
pub fn speed_range(design_kn: f64) -> (f64, f64, f64) {
    let step = [0.1, 0.2, 0.25, 0.5, 1.0, 2.0, 2.5, 5.0, 10.0]
        .into_iter()
        .find(|s| *s >= design_kn / 10.0)
        .unwrap_or(10.0);
    (0.5 * design_kn, 1.5 * design_kn, step)
}

// ---- start card -----------------------------------------------------------

/// What the start card asked for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartAction {
    None,
    NewDesign,
    /// The New design dialog, on *Example*.
    Example,
    Open,
    Demo,
}

/// The start card's buttons, from the last frame (test hook).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StartRects {
    pub new_design: egui::Rect,
    pub example: egui::Rect,
    pub open: egui::Rect,
    pub demo: egui::Rect,
}

impl Default for StartRects {
    fn default() -> Self {
        Self {
            new_design: egui::Rect::NOTHING,
            example: egui::Rect::NOTHING,
            open: egui::Rect::NOTHING,
            demo: egui::Rect::NOTHING,
        }
    }
}

/// The central area with no model: a card with the ways to start.
pub fn start_card(ui: &mut egui::Ui) -> (StartAction, StartRects) {
    let mut action = StartAction::None;
    let mut rects = StartRects::default();
    let full = ui.available_rect_before_wrap();
    ui.painter().rect_filled(full, 0.0, theme::CANVAS);
    let size = egui::vec2(340.0, 268.0);
    let card = egui::Rect::from_center_size(full.center(), size).intersect(full);
    let ctx = ui.ctx().clone();
    let keys = |key: egui::Key| {
        ctx.format_shortcut(&egui::KeyboardShortcut::new(egui::Modifiers::COMMAND, key))
    };
    ui.scope_builder(egui::UiBuilder::new().max_rect(card), |ui| {
        egui::Frame::new()
            .fill(theme::PANEL)
            .stroke(egui::Stroke::new(1.0, theme::FRAME))
            .corner_radius(6.0)
            .inner_margin(egui::Margin::same(20))
            .show(ui, |ui| {
                ui.set_width(size.x - 40.0);
                ui.label(
                    egui::RichText::new("SeaCraft")
                        .heading()
                        .color(theme::TEXT_PRIMARY),
                );
                ui.label(egui::RichText::new("Start a hull design").color(theme::TEXT_SECONDARY));
                ui.add_space(12.0);
                let button = |ui: &mut egui::Ui, text: &str, shortcut: Option<String>| {
                    let mut b = egui::Button::new(text).min_size(egui::vec2(size.x - 40.0, 30.0));
                    if let Some(s) = shortcut {
                        b = b.shortcut_text(s);
                    }
                    ui.add(b)
                };
                let r = button(ui, "New design…", Some(keys(egui::Key::N)));
                rects.new_design = r.rect;
                if r.clicked() {
                    action = StartAction::NewDesign;
                }
                if !Example::all().is_empty() {
                    let r = button(ui, "Open an example…", None);
                    rects.example = r.rect;
                    if r.clicked() {
                        action = StartAction::Example;
                    }
                }
                let r = button(ui, "Open…", Some(keys(egui::Key::O)));
                rects.open = r.rect;
                if r.clicked() {
                    action = StartAction::Open;
                }
                let r = button(ui, "Open demo hull", None);
                rects.demo = r.rect;
                if r.clicked() {
                    action = StartAction::Demo;
                }
                ui.add_space(8.0);
                ui.label(
                    egui::RichText::new(format!("v{}", seacraft_core::VERSION))
                        .small()
                        .color(theme::TEXT_MUTED),
                );
            });
    });
    (action, rects)
}

// ---- New design dialog --------------------------------------------------

/// Where a new design starts from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartFrom {
    Parametric,
    /// One of the example hulls, with its project.
    Example,
    Demo,
    Import,
}

/// A generated hull the preview already built (reused by *Create* when the
/// spec has not changed since).
#[derive(Debug, Clone)]
pub struct Preview {
    pub spec: HullSpec,
    pub surface: Surface,
    pub report: GenerateReport,
}

/// What the dialog asked for.
#[derive(Debug, Clone)]
pub enum NewDesignAction {
    None,
    Cancel,
    Create(NewDesignRequest),
}

/// A confirmed *Create*.
#[derive(Debug, Clone)]
pub enum NewDesignRequest {
    Parametric(Box<ParametricDesign>),
    /// An example hull and its sidecar, as the preview built them.
    Example(Box<ExampleDesign>),
    Demo {
        half: bool,
    },
    Import,
}

/// An example to create: the hull and the project
/// [`seacraft_core::examples::build`] made.
#[derive(Debug, Clone)]
pub struct ExampleDesign {
    pub kind: Example,
    pub surface: Surface,
    pub project: Project,
}

/// A parametric hull to create, with the requirements it was asked for.
#[derive(Debug, Clone)]
pub struct ParametricDesign {
    pub spec: HullSpec,
    pub name: String,
    pub requirements: Requirements,
    /// The preview's hull when it was built from exactly `spec`.
    pub built: Option<Preview>,
}

/// Which principal fields the user has edited (a kind switch re-fills the
/// others from [`HullSpec::default_for`]).
#[derive(Debug, Clone, Copy, Default)]
struct Touched {
    name: bool,
    lwl: bool,
    beam: bool,
    draft: bool,
    depth: bool,
    deadrise: bool,
    transom: bool,
}

/// Where the dialog's buttons were drawn last frame (test hook).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DialogRects {
    pub create: egui::Rect,
    pub cancel: egui::Rect,
}

impl Default for DialogRects {
    fn default() -> Self {
        Self {
            create: egui::Rect::NOTHING,
            cancel: egui::Rect::NOTHING,
        }
    }
}

type PreviewOutcome = (u64, Result<Preview, String>);

/// The New design dialog's editing state.
pub struct NewDesignState {
    pub start: StartFrom,
    pub kind: HullKind,
    /// The example picked under *Example*.
    pub example: Example,
    /// The picked example, built (the preview and what *Create* loads).
    example_built: Option<ExampleDesign>,
    example_error: Option<String>,
    pub half_model: bool,
    pub name: String,
    pub lwl: f64,
    pub beam: f64,
    pub draft: f64,
    pub depth: f64,
    /// Target block coefficient; Δ is shown from it (`None` = auto).
    pub cb: Option<f64>,
    /// Target LCB, per cent of LWL from its aft end.
    pub lcb_pct: Option<f64>,
    pub speed_kn: Option<f64>,
    pub kg: Option<f64>,
    pub deadrise_deg: f64,
    pub transom_frac: f64,
    pub stations: usize,
    pub points_per_station: usize,
    /// Water density for the Δ ⇄ Cb link, t/m³.
    pub rho_t_m3: f64,
    touched: Touched,
    /// The spec drawn last frame, to notice edits.
    last_spec: Option<HullSpec>,
    last_edit: Option<Instant>,
    preview: Option<Preview>,
    preview_error: Option<String>,
    worker: Option<Receiver<PreviewOutcome>>,
    generation: u64,
    /// Generate the preview on the calling thread (headless tests).
    pub inline_preview: bool,
    pub rects: DialogRects,
}

impl NewDesignState {
    pub fn new(rho_t_m3: f64) -> Self {
        let kind = HullKind::Displacement;
        let d = HullSpec::default_for(kind);
        Self {
            start: StartFrom::Parametric,
            kind,
            example: Example::all()
                .first()
                .copied()
                .unwrap_or(Example::Runabout6m),
            example_built: None,
            example_error: None,
            half_model: false,
            name: default_name(kind, d.lwl),
            lwl: d.lwl,
            beam: d.beam,
            draft: d.draft,
            depth: d.depth,
            cb: None,
            lcb_pct: None,
            speed_kn: None,
            kg: None,
            deadrise_deg: d.deadrise_deg,
            transom_frac: d.transom_frac,
            stations: d.stations,
            points_per_station: d.points_per_station,
            rho_t_m3,
            touched: Touched::default(),
            last_spec: None,
            last_edit: None,
            preview: None,
            preview_error: None,
            worker: None,
            generation: 0,
            inline_preview: false,
            rects: DialogRects::default(),
        }
    }

    /// Switch the hull type, re-filling every principal field the user has
    /// not edited from that type's defaults.
    pub fn set_kind(&mut self, kind: HullKind) {
        self.kind = kind;
        let d = HullSpec::default_for(kind);
        let t = self.touched;
        if !t.lwl {
            self.lwl = d.lwl;
        }
        if !t.beam {
            self.beam = d.beam;
        }
        if !t.draft {
            self.draft = d.draft;
        }
        if !t.depth {
            self.depth = d.depth;
        }
        if !t.deadrise {
            self.deadrise_deg = d.deadrise_deg;
        }
        if !t.transom {
            self.transom_frac = d.transom_frac;
        }
        if !t.name {
            self.name = default_name(kind, self.lwl);
        }
    }

    /// The generator's input.
    pub fn spec(&self) -> HullSpec {
        HullSpec {
            kind: self.kind,
            lwl: self.lwl,
            beam: self.beam,
            draft: self.draft,
            depth: self.depth,
            cb: self.cb,
            lcb_frac: self.lcb_pct.map(|p| p / 100.0),
            deadrise_deg: self.deadrise_deg,
            transom_frac: self.transom_frac,
            stations: self.stations,
            points_per_station: self.points_per_station,
        }
    }

    /// The dialog opened on *Example* (the start card's "Open an
    /// example…"), its preview already built.
    pub fn on_example(rho_t_m3: f64) -> Self {
        let mut s = Self::new(rho_t_m3);
        s.start = StartFrom::Example;
        s.refresh_example();
        s
    }

    /// Pick an example: its preview is built at once.
    pub fn set_example(&mut self, kind: Example) {
        self.example = kind;
        self.refresh_example();
    }

    /// The picked example as built for the preview.
    pub fn example_preview(&self) -> Option<&ExampleDesign> {
        self.example_built
            .as_ref()
            .filter(|b| b.kind == self.example)
    }

    /// Build the picked example unless it is the one already built.
    fn refresh_example(&mut self) {
        if self.example_preview().is_some() {
            return;
        }
        match seacraft_core::examples::build(self.example) {
            Ok((surface, project)) => {
                self.example_built = Some(ExampleDesign {
                    kind: self.example,
                    surface,
                    project,
                });
                self.example_error = None;
            }
            Err(e) => {
                self.example_built = None;
                self.example_error = Some(e);
            }
        }
    }

    /// Why *Create* is disabled, if it is.
    pub fn problem(&self) -> Option<String> {
        if self.start == StartFrom::Example {
            if !self.example.is_available() {
                return Some(format!("the {} example is not available", self.example));
            }
            return self.example_error.clone();
        }
        if self.start != StartFrom::Parametric {
            return None;
        }
        if let Err(e) = self.spec().validate() {
            return Some(e);
        }
        if self.speed_kn.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
            return Some("design speed must be > 0".into());
        }
        if self.kg.is_some_and(|v| !(v.is_finite() && v > 0.0)) {
            return Some("KG estimate must be > 0".into());
        }
        None
    }

    pub fn can_create(&self) -> bool {
        self.problem().is_none()
    }

    /// The target displacement, from the target Cb.
    pub fn displacement_t(&self) -> Option<f64> {
        self.cb
            .map(|cb| displacement_from_cb(cb, self.rho_t_m3, self.lwl, self.beam, self.draft))
    }

    /// Set the target displacement: the target Cb follows.
    pub fn set_displacement_t(&mut self, displacement_t: f64) {
        self.cb = Some(cb_from_displacement(
            displacement_t,
            self.rho_t_m3,
            self.lwl,
            self.beam,
            self.draft,
        ));
    }

    /// The requirements a *Create* stores in the project.
    pub fn requirements(&self) -> Requirements {
        Requirements {
            name: self.name.trim().to_owned(),
            lwl: Some(self.lwl),
            bwl: Some(self.beam),
            draft: Some(self.draft),
            depth: Some(self.depth),
            displacement_t: self.displacement_t(),
            cb: self.cb,
            lcb_frac: self.lcb_pct.map(|p| p / 100.0),
            design_speed_kn: self.speed_kn,
            kg: self.kg,
        }
    }

    /// The last finished preview.
    pub fn preview(&self) -> Option<&Preview> {
        self.preview.as_ref()
    }

    /// A preview is being generated.
    pub fn preview_busy(&self) -> bool {
        self.worker.is_some()
    }

    /// Notice edits, collect a finished preview and start a new one once
    /// the edits have settled. Returns how long to wait for the next check.
    fn tick(&mut self, now: Instant) -> Option<Duration> {
        if self.start == StartFrom::Example {
            self.refresh_example();
        }
        if !self.touched.name {
            // An untyped name follows the type and the length.
            self.name = default_name(self.kind, self.lwl);
        }
        let spec = self.spec();
        if self.last_spec.as_ref() != Some(&spec) {
            // The first frame generates at once; later edits wait.
            if self.last_spec.is_some() {
                self.last_edit = Some(now);
            }
            self.last_spec = Some(spec.clone());
        }
        if let Some(rx) = &self.worker {
            match rx.try_recv() {
                Ok((gen, outcome)) => {
                    self.worker = None;
                    if gen == self.generation {
                        match outcome {
                            Ok(p) => {
                                self.preview = Some(p);
                                self.preview_error = None;
                            }
                            Err(e) => self.preview_error = Some(e),
                        }
                    }
                }
                Err(TryRecvError::Empty) => return Some(Duration::from_millis(60)),
                Err(TryRecvError::Disconnected) => self.worker = None,
            }
        }
        if self.start != StartFrom::Parametric || spec.validate().is_err() {
            return None;
        }
        let current = self.preview.as_ref().is_some_and(|p| p.spec == spec);
        if current || self.worker.is_some() {
            return None;
        }
        if let Some(t) = self.last_edit {
            let since = now.saturating_duration_since(t);
            if since < PREVIEW_DEBOUNCE {
                return Some(PREVIEW_DEBOUNCE - since);
            }
        }
        if self.preview_error.is_some() && self.last_edit.is_none() {
            return None;
        }
        self.last_edit = None;
        self.generation += 1;
        let gen = self.generation;
        let build = move |spec: HullSpec| -> Result<Preview, String> {
            let (surface, report) = seacraft_core::parametric::generate(&spec)?;
            Ok(Preview {
                spec,
                surface,
                report,
            })
        };
        if self.inline_preview {
            match build(spec) {
                Ok(p) => {
                    self.preview = Some(p);
                    self.preview_error = None;
                }
                Err(e) => self.preview_error = Some(e),
            }
            return None;
        }
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send((gen, build(spec)));
        });
        self.worker = Some(rx);
        Some(Duration::from_millis(60))
    }

    fn request(&mut self) -> NewDesignRequest {
        match self.start {
            StartFrom::Parametric => {
                let spec = self.spec();
                let built = self.preview.as_ref().filter(|p| p.spec == spec).cloned();
                NewDesignRequest::Parametric(Box::new(ParametricDesign {
                    spec,
                    name: self.name.trim().to_owned(),
                    requirements: self.requirements(),
                    built,
                }))
            }
            StartFrom::Example => {
                self.refresh_example();
                match self.example_preview() {
                    Some(b) => NewDesignRequest::Example(Box::new(b.clone())),
                    // `problem` keeps Create disabled without a built example.
                    None => NewDesignRequest::Import,
                }
            }
            StartFrom::Demo => NewDesignRequest::Demo {
                half: self.half_model,
            },
            StartFrom::Import => NewDesignRequest::Import,
        }
    }
}

fn default_name(kind: HullKind, lwl: f64) -> String {
    let what = match kind {
        HullKind::Displacement => "Displacement hull",
        HullKind::Planing => "Planing hull",
        HullKind::Yacht => "Yacht",
    };
    format!("{what} {lwl:.0} m")
}

/// A name / value / unit row in a dialog grid.
pub(crate) fn field(
    ui: &mut egui::Ui,
    name: &str,
    widget: impl egui::Widget,
    unit: &str,
) -> egui::Response {
    ui.label(egui::RichText::new(name).color(theme::TEXT_SECONDARY));
    let r = ui.add(widget);
    ui.label(egui::RichText::new(unit).color(theme::TEXT_MUTED));
    r
}

/// An optional target: a checkbox that turns it on (at `default`) and its
/// value. Returns whether anything changed.
pub(crate) fn optional(
    ui: &mut egui::Ui,
    name: &str,
    value: &mut Option<f64>,
    default: f64,
    speed: f64,
    decimals: usize,
    unit: &str,
) -> bool {
    let mut on = value.is_some();
    let mut changed = false;
    if ui
        .checkbox(
            &mut on,
            egui::RichText::new(name).color(theme::TEXT_SECONDARY),
        )
        .changed()
    {
        *value = on.then_some(default);
        changed = true;
    }
    match value {
        Some(v) => {
            changed |= ui
                .add(
                    egui::DragValue::new(v)
                        .speed(speed)
                        .range(0.0..=1.0e5)
                        .fixed_decimals(decimals),
                )
                .changed();
        }
        None => {
            ui.label(egui::RichText::new("auto").color(theme::TEXT_MUTED));
        }
    }
    ui.label(egui::RichText::new(unit).color(theme::TEXT_MUTED));
    changed
}

/// Draw the New design dialog.
pub fn new_design_dialog(ctx: &egui::Context, state: &mut NewDesignState) -> NewDesignAction {
    if let Some(wait) = state.tick(Instant::now()) {
        ctx.request_repaint_after(wait);
    }
    let mut action = NewDesignAction::None;
    let modal = egui::Modal::new(egui::Id::unique("new_design_dialog")).show(ctx, |ui| {
        ui.set_width(600.0);
        ui.label(
            egui::RichText::new("New design")
                .heading()
                .color(theme::TEXT_PRIMARY),
        );
        ui.add_space(4.0);
        start_from(ui, state);
        ui.separator();
        let parametric = state.start == StartFrom::Parametric;
        if state.start == StartFrom::Example {
            example_preview(ui, state);
        } else {
            ui.add_enabled_ui(parametric, |ui| principal(ui, state));
        }
        if parametric {
            egui::CollapsingHeader::new("Targets (optional)")
                .id_salt("new_design_targets")
                .default_open(false)
                .show(ui, |ui| targets(ui, state));
            egui::CollapsingHeader::new("Shape (optional)")
                .id_salt("new_design_shape")
                .default_open(false)
                .show(ui, |ui| shape(ui, state));
            ui.separator();
            preview(ui, state);
        }
        ui.separator();
        ui.horizontal(|ui| {
            if let Some(p) = state.problem() {
                ui.colored_label(theme::FAIL, format!("⚠ {p}"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let label = match state.start {
                    StartFrom::Import => "Open…",
                    _ => "Create",
                };
                let create = ui.add_enabled(state.can_create(), egui::Button::new(label));
                state.rects.create = create.rect;
                if create.clicked() {
                    action = NewDesignAction::Create(state.request());
                }
                let cancel = ui.button("Cancel");
                state.rects.cancel = cancel.rect;
                if cancel.clicked() {
                    action = NewDesignAction::Cancel;
                }
            });
        });
    });
    // Escape cancels; a click on the backdrop does not (it would lose the
    // typed values to a slip of the mouse).
    if matches!(action, NewDesignAction::None)
        && modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        action = NewDesignAction::Cancel;
    }
    action
}

fn start_from(ui: &mut egui::Ui, state: &mut NewDesignState) {
    egui::Grid::new("new_design_start")
        .num_columns(2)
        .spacing(egui::vec2(12.0, 6.0))
        .show(ui, |ui| {
            ui.label(egui::RichText::new("Start from").color(theme::TEXT_SECONDARY));
            ui.horizontal(|ui| {
                ui.radio_value(&mut state.start, StartFrom::Parametric, "Parametric");
                ui.add_enabled_ui(state.start == StartFrom::Parametric, |ui| {
                    for kind in [HullKind::Displacement, HullKind::Planing, HullKind::Yacht] {
                        let label = match kind {
                            HullKind::Displacement => "Displacement",
                            HullKind::Planing => "Planing",
                            HullKind::Yacht => "Yacht",
                        };
                        if ui.selectable_label(state.kind == kind, label).clicked()
                            && state.kind != kind
                        {
                            state.set_kind(kind);
                        }
                    }
                });
            });
            ui.end_row();
            let examples = Example::all();
            if !examples.is_empty() {
                ui.label("");
                ui.horizontal(|ui| {
                    ui.radio_value(&mut state.start, StartFrom::Example, "Example");
                    ui.add_enabled_ui(state.start == StartFrom::Example, |ui| {
                        egui::ComboBox::from_id_salt("new_design_example")
                            .selected_text(state.example.title())
                            .width(260.0)
                            .show_ui(ui, |ui| {
                                for e in examples {
                                    let r = ui
                                        .selectable_label(state.example == e, e.title())
                                        .on_hover_text(e.description());
                                    if r.clicked() && state.example != e {
                                        state.set_example(e);
                                    }
                                }
                            });
                    });
                });
                ui.end_row();
            }
            ui.label("");
            ui.horizontal(|ui| {
                ui.radio_value(&mut state.start, StartFrom::Demo, "Demo hull");
                ui.add_enabled(
                    state.start == StartFrom::Demo,
                    egui::Checkbox::new(&mut state.half_model, "half model"),
                );
            });
            ui.end_row();
            ui.label("");
            ui.radio_value(&mut state.start, StartFrom::Import, "Import .fef…");
            ui.end_row();
        });
}

fn principal(ui: &mut egui::Ui, state: &mut NewDesignState) {
    ui.label(egui::RichText::new("Principal dimensions").color(theme::TEXT_PRIMARY));
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Name").color(theme::TEXT_SECONDARY));
        if ui
            .add(egui::TextEdit::singleline(&mut state.name).desired_width(240.0))
            .changed()
        {
            state.touched.name = true;
        }
    });
    egui::Grid::new("new_design_principal")
        .num_columns(6)
        .spacing(egui::vec2(8.0, 6.0))
        .show(ui, |ui| {
            fn dim(v: &mut f64) -> egui::DragValue<'_> {
                egui::DragValue::new(v)
                    .speed(0.01)
                    .range(0.0..=1000.0)
                    .fixed_decimals(2)
            }
            if field(ui, "LWL", dim(&mut state.lwl), "m").changed() {
                state.touched.lwl = true;
            }
            if field(ui, "BWL", dim(&mut state.beam), "m").changed() {
                state.touched.beam = true;
            }
            ui.end_row();
            if field(ui, "T", dim(&mut state.draft), "m").changed() {
                state.touched.draft = true;
            }
            if field(ui, "Depth", dim(&mut state.depth), "m").changed() {
                state.touched.depth = true;
            }
            ui.end_row();
        });
}

fn targets(ui: &mut egui::Ui, state: &mut NewDesignState) {
    let actual = state.preview.as_ref().map(|p| p.report.clone());
    let actual = actual.as_ref();
    egui::Grid::new("new_design_targets")
        .num_columns(3)
        .spacing(egui::vec2(8.0, 6.0))
        .show(ui, |ui| {
            // Δ ⇄ Cb: one target, shown both ways. Cb is what the generator
            // takes; Δ follows the principal dimensions.
            let mut on = state.cb.is_some();
            if ui
                .checkbox(
                    &mut on,
                    egui::RichText::new("Δ ⇄ Cb").color(theme::TEXT_SECONDARY),
                )
                .changed()
            {
                state.cb = on.then(|| actual.map_or(0.5, |r| (r.cb * 1000.0).round() / 1000.0));
            }
            ui.horizontal(|ui| match state.cb {
                Some(_) => {
                    let mut disp = state.displacement_t().unwrap_or(0.0);
                    if ui
                        .add(
                            egui::DragValue::new(&mut disp)
                                .speed(0.05)
                                .range(0.0..=1.0e6)
                                .fixed_decimals(2)
                                .suffix(" t"),
                        )
                        .changed()
                    {
                        state.set_displacement_t(disp);
                    }
                    ui.label(egui::RichText::new("⇄").color(theme::TEXT_MUTED));
                    if let Some(cb) = state.cb.as_mut() {
                        ui.add(
                            egui::DragValue::new(cb)
                                .speed(0.002)
                                .range(0.0..=1.0)
                                .fixed_decimals(3)
                                .prefix("Cb "),
                        );
                    }
                }
                None => {
                    let text = match actual {
                        Some(r) => format!("auto ({:.2} t, Cb {:.3})", r.displacement_t, r.cb),
                        None => "auto".to_owned(),
                    };
                    ui.label(egui::RichText::new(text).color(theme::TEXT_MUTED));
                }
            });
            ui.label("");
            ui.end_row();

            optional(
                ui,
                "LCB",
                &mut state.lcb_pct,
                actual.map_or(50.0, |r| (r.lcb_frac * 1000.0).round() / 10.0),
                0.1,
                1,
                "% LWL from aft end",
            );
            ui.end_row();
            let fn_text = |v: Option<f64>, lwl: f64| {
                v.map(|v| format!("kn  (Fn {:.2})", froude_number(v, lwl)))
                    .unwrap_or_else(|| "kn".to_owned())
            };
            let unit = fn_text(state.speed_kn, state.lwl);
            optional(ui, "Design speed", &mut state.speed_kn, 8.0, 0.1, 1, &unit);
            ui.end_row();
            let kg_default = (0.6 * state.depth * 100.0).round() / 100.0;
            optional(ui, "KG estimate", &mut state.kg, kg_default, 0.01, 2, "m");
            ui.end_row();
        });
}

fn shape(ui: &mut egui::Ui, state: &mut NewDesignState) {
    egui::Grid::new("new_design_shape")
        .num_columns(3)
        .spacing(egui::vec2(8.0, 6.0))
        .show(ui, |ui| {
            if field(
                ui,
                "Deadrise",
                egui::DragValue::new(&mut state.deadrise_deg)
                    .speed(0.5)
                    .range(0.0..=90.0)
                    .fixed_decimals(1),
                "°",
            )
            .changed()
            {
                state.touched.deadrise = true;
            }
            ui.end_row();
            if field(
                ui,
                "Transom",
                egui::DragValue::new(&mut state.transom_frac)
                    .speed(0.01)
                    .range(0.0..=2.0)
                    .fixed_decimals(2),
                "of beam",
            )
            .changed()
            {
                state.touched.transom = true;
            }
            ui.end_row();
            ui.label(egui::RichText::new("Control net").color(theme::TEXT_SECONDARY));
            ui.horizontal(|ui| {
                ui.add(egui::DragValue::new(&mut state.stations).range(1..=99));
                ui.label(egui::RichText::new("×").color(theme::TEXT_MUTED));
                ui.add(egui::DragValue::new(&mut state.points_per_station).range(1..=99));
            });
            ui.label(egui::RichText::new("stations × points").color(theme::TEXT_MUTED));
            ui.end_row();
        });
}

/// One row of the target / actual table.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareRow {
    pub name: &'static str,
    pub target: Option<f64>,
    pub actual: f64,
    pub decimals: usize,
    pub unit: &'static str,
}

/// The preview table: what was asked for against what the generator built.
pub fn compare_rows(report: &GenerateReport) -> Vec<CompareRow> {
    let row = |name, target, actual, decimals, unit| CompareRow {
        name,
        target,
        actual,
        decimals,
        unit,
    };
    let disp_target = report.target_cb.map(|cb| {
        displacement_from_cb(
            cb,
            SEA_WATER_T_M3,
            report.target_lwl,
            report.target_bwl,
            report.target_draft,
        )
    });
    vec![
        row("LWL", Some(report.target_lwl), report.lwl, 2, "m"),
        row("BWL", Some(report.target_bwl), report.bwl, 2, "m"),
        row("T", Some(report.target_draft), report.t, 3, "m"),
        row("Δ", disp_target, report.displacement_t, 2, "t"),
        row("Cb", report.target_cb, report.cb, 3, ""),
        row(
            "LCB",
            report.target_lcb_frac.map(|f| 100.0 * f),
            100.0 * report.lcb_frac,
            1,
            "% LWL",
        ),
    ]
}

fn preview(ui: &mut egui::Ui, state: &NewDesignState) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Preview").color(theme::TEXT_PRIMARY));
        if state.preview_busy() {
            ui.spinner();
        }
        if state.preview.is_some()
            && state
                .preview
                .as_ref()
                .is_some_and(|p| Some(&p.spec) != state.last_spec.as_ref())
        {
            ui.label(egui::RichText::new("out of date").color(theme::STALE));
        }
    });
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 190.0), egui::Sense::hover());
        draw_outlines(
            ui.painter(),
            rect,
            state.preview.as_ref().map(|p| (&p.surface, p.spec.draft)),
        );
        ui.add_space(8.0);
        ui.vertical(|ui| {
            if let Some(e) = &state.preview_error {
                ui.colored_label(theme::FAIL, e);
            }
            let Some(p) = &state.preview else {
                ui.label(egui::RichText::new("generating…").color(theme::TEXT_MUTED));
                return;
            };
            egui::Grid::new("new_design_compare")
                .num_columns(4)
                .spacing(egui::vec2(10.0, 4.0))
                .show(ui, |ui| {
                    ui.label("");
                    ui.label(egui::RichText::new("target").color(theme::TARGET));
                    ui.label(egui::RichText::new("actual").color(theme::TEXT_SECONDARY));
                    ui.label("");
                    ui.end_row();
                    for r in compare_rows(&p.report) {
                        ui.label(egui::RichText::new(r.name).color(theme::TEXT_SECONDARY));
                        let target = r
                            .target
                            .map(|t| format!("{t:.*}", r.decimals))
                            .unwrap_or_else(|| "auto".to_owned());
                        ui.label(egui::RichText::new(target).monospace().color(theme::TARGET));
                        ui.label(
                            egui::RichText::new(format!("{:.*}", r.decimals, r.actual))
                                .monospace()
                                .color(theme::TEXT_PRIMARY),
                        );
                        ui.label(egui::RichText::new(r.unit).color(theme::TEXT_MUTED));
                        ui.end_row();
                    }
                });
        });
    });
}

/// One row of the example preview's particulars.
#[derive(Debug, Clone, PartialEq)]
pub struct ExampleRow {
    pub name: &'static str,
    pub value: String,
}

/// The particulars an example preview lists: its design requirements and
/// its KG.
pub fn example_rows(project: &Project) -> Vec<ExampleRow> {
    let mut rows = Vec::new();
    let mut push = |name, v: Option<f64>, decimals: usize, unit: &str| {
        if let Some(v) = v {
            rows.push(ExampleRow {
                name,
                value: format!("{v:.decimals$} {unit}").trim_end().to_owned(),
            });
        }
    };
    if let Some(r) = &project.requirements {
        push("LWL", r.lwl, 2, "m");
        push("BWL", r.bwl, 2, "m");
        push("T", r.draft, 3, "m");
        push("Depth", r.depth, 2, "m");
        push("Δ", r.displacement_t, 2, "t");
        push("Cb", r.cb, 3, "");
        push("Speed", r.design_speed_kn, 1, "kn");
    }
    push("KG", project.kg, 2, "m");
    rows
}

/// The example under *Example*: what it is, its outlines and particulars.
fn example_preview(ui: &mut egui::Ui, state: &NewDesignState) {
    let e = state.example;
    ui.label(egui::RichText::new(e.title()).color(theme::TEXT_PRIMARY));
    ui.label(egui::RichText::new(e.description()).color(theme::TEXT_SECONDARY));
    ui.label(
        egui::RichText::new(format!("Particulars: {}", e.source()))
            .small()
            .color(theme::TEXT_MUTED),
    );
    ui.separator();
    ui.horizontal_top(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(300.0, 190.0), egui::Sense::hover());
        let built = state.example_preview();
        draw_outlines(
            ui.painter(),
            rect,
            built.map(|b| (&b.surface, b.surface.settings.draft)),
        );
        ui.add_space(8.0);
        ui.vertical(|ui| {
            if let Some(err) = &state.example_error {
                ui.colored_label(theme::FAIL, err);
            }
            let Some(b) = built else { return };
            egui::Grid::new("new_design_example_rows")
                .num_columns(2)
                .spacing(egui::vec2(10.0, 4.0))
                .show(ui, |ui| {
                    for r in example_rows(&b.project) {
                        ui.label(egui::RichText::new(r.name).color(theme::TEXT_SECONDARY));
                        ui.label(
                            egui::RichText::new(r.value)
                                .monospace()
                                .color(theme::TEXT_PRIMARY),
                        );
                        ui.end_row();
                    }
                });
        });
    });
}

/// Profile (top) and body plan (bottom) of a control net, with its design
/// waterline `draft` above the keel.
fn draw_outlines(painter: &egui::Painter, rect: egui::Rect, net: Option<(&Surface, f64)>) {
    painter.rect_filled(rect, 4.0, theme::CANVAS);
    painter.rect_stroke(
        rect,
        4.0,
        egui::Stroke::new(1.0, theme::FRAME),
        egui::StrokeKind::Inside,
    );
    let Some((s, draft)) = net else { return };
    if s.points.is_empty() {
        return;
    }
    let (min, max) = s.extents();
    let keel = min.z;
    let dwl = keel + draft;
    let caption = |r: egui::Rect, text: &str| {
        painter.text(
            r.left_top() + egui::vec2(4.0, 2.0),
            egui::Align2::LEFT_TOP,
            text,
            theme::caption_font(),
            theme::TEXT_SECONDARY,
        );
    };
    let net = egui::Stroke::new(1.0, theme::CONTROL_NET);
    let waterline = egui::Stroke::new(1.0, theme::DWL);
    let inner = rect.shrink(6.0);
    let (top, bottom) = inner.split_top_bottom_at_fraction(0.45);
    let top = top.shrink2(egui::vec2(0.0, 4.0));
    let bottom = bottom.shrink2(egui::vec2(0.0, 4.0));

    // Profile: x across, z up.
    let map = fit(top.shrink(10.0), (min.x, min.z), (max.x, max.z));
    caption(top, "profile");
    for e in &s.edges {
        let (a, b) = (s.points[e.a].p, s.points[e.b].p);
        painter.line_segment([map(a.x, a.z), map(b.x, b.z)], net);
    }
    painter.line_segment([map(min.x, dwl), map(max.x, dwl)], waterline);

    // Body plan: y across (mirrored, the model is a half), z up.
    let half = max.y.abs().max(min.y.abs()).max(1e-6);
    let map = fit(bottom.shrink(10.0), (-half, min.z), (half, max.z));
    caption(bottom, "body plan");
    for e in &s.edges {
        let (a, b) = (s.points[e.a].p, s.points[e.b].p);
        painter.line_segment([map(a.y, a.z), map(b.y, b.z)], net);
        painter.line_segment([map(-a.y, a.z), map(-b.y, b.z)], net);
    }
    painter.line_segment([map(-half, dwl), map(half, dwl)], waterline);
}

/// A uniform-scale map from model `(u, v)` (v up) into `rect`, centred.
fn fit(rect: egui::Rect, min: (f64, f64), max: (f64, f64)) -> impl Fn(f64, f64) -> egui::Pos2 {
    let w = (max.0 - min.0).max(1e-6);
    let h = (max.1 - min.1).max(1e-6);
    let scale = (rect.width() as f64 / w).min(rect.height() as f64 / h);
    let cx = 0.5 * (min.0 + max.0);
    let cy = 0.5 * (min.1 + max.1);
    let c = rect.center();
    move |u, v| {
        egui::pos2(
            c.x + ((u - cx) * scale) as f32,
            c.y - ((v - cy) * scale) as f32,
        )
    }
}

// ---- Project settings dialog ---------------------------------------------

/// The Project settings dialog's editing copy.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSettingsState {
    pub name: String,
    pub designer: String,
    /// kg/m³, as the `.fef` stores it.
    pub water_density: f64,
    pub requirements: Requirements,
    pub rects: DialogRects,
}

/// What the Project settings dialog asked for.
#[derive(Debug, Clone, PartialEq)]
pub enum ProjectSettingsAction {
    None,
    Cancel,
    Apply(Box<ProjectSettingsEdit>),
}

/// The values a Project settings *Apply* sets.
#[derive(Debug, Clone, PartialEq)]
pub struct ProjectSettingsEdit {
    pub name: String,
    pub designer: String,
    pub water_density: f64,
    /// `None` when every target is cleared.
    pub requirements: Option<Requirements>,
}

impl ProjectSettingsState {
    pub fn new(surface: &Surface, requirements: Option<&Requirements>) -> Self {
        Self {
            name: surface.settings.name.clone(),
            designer: surface.settings.designer.clone(),
            water_density: surface.settings.water_density,
            requirements: requirements.cloned().unwrap_or_default(),
            rects: DialogRects::default(),
        }
    }

    pub fn problem(&self) -> Option<String> {
        if !(self.water_density.is_finite() && self.water_density > 0.0) {
            return Some("water density must be > 0".into());
        }
        self.requirements.validate().err()
    }

    fn apply(&self) -> ProjectSettingsAction {
        let empty = self.requirements == Requirements::default();
        ProjectSettingsAction::Apply(Box::new(ProjectSettingsEdit {
            name: self.name.trim().to_owned(),
            designer: self.designer.trim().to_owned(),
            water_density: self.water_density,
            requirements: (!empty).then(|| self.requirements.clone()),
        }))
    }
}

/// Draw the Project settings dialog.
pub fn project_settings_dialog(
    ctx: &egui::Context,
    state: &mut ProjectSettingsState,
) -> ProjectSettingsAction {
    let mut action = ProjectSettingsAction::None;
    let modal = egui::Modal::new(egui::Id::unique("project_settings_dialog")).show(ctx, |ui| {
        ui.set_width(440.0);
        ui.label(
            egui::RichText::new("Project settings")
                .heading()
                .color(theme::TEXT_PRIMARY),
        );
        ui.add_space(4.0);
        egui::Grid::new("project_settings_main")
            .num_columns(3)
            .spacing(egui::vec2(8.0, 6.0))
            .show(ui, |ui| {
                field(
                    ui,
                    "Name",
                    egui::TextEdit::singleline(&mut state.name).desired_width(220.0),
                    "",
                );
                ui.end_row();
                field(
                    ui,
                    "Designer",
                    egui::TextEdit::singleline(&mut state.designer).desired_width(220.0),
                    "",
                );
                ui.end_row();
                field(
                    ui,
                    "Water density",
                    egui::DragValue::new(&mut state.water_density)
                        .speed(0.5)
                        .range(0.0..=2000.0)
                        .fixed_decimals(1),
                    "kg/m³",
                );
                ui.end_row();
            });
        ui.separator();
        ui.label(egui::RichText::new("Design requirements").color(theme::TEXT_PRIMARY));
        ui.label(
            egui::RichText::new("Targets are shown against the measured hull; they never move it.")
                .small()
                .color(theme::TEXT_MUTED),
        );
        let r = &mut state.requirements;
        egui::Grid::new("project_settings_requirements")
            .num_columns(3)
            .spacing(egui::vec2(8.0, 6.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Design name").color(theme::TEXT_SECONDARY));
                ui.add(egui::TextEdit::singleline(&mut r.name).desired_width(200.0));
                ui.label("");
                ui.end_row();
                optional(ui, "LWL", &mut r.lwl, 10.0, 0.01, 2, "m");
                ui.end_row();
                optional(ui, "BWL", &mut r.bwl, 3.0, 0.01, 2, "m");
                ui.end_row();
                optional(ui, "T", &mut r.draft, 0.6, 0.01, 3, "m");
                ui.end_row();
                optional(ui, "Depth", &mut r.depth, 1.5, 0.01, 2, "m");
                ui.end_row();
                optional(ui, "Δ", &mut r.displacement_t, 10.0, 0.05, 2, "t");
                ui.end_row();
                optional(ui, "Cb", &mut r.cb, 0.5, 0.002, 3, "");
                ui.end_row();
                let mut pct = r.lcb_frac.map(|f| 100.0 * f);
                if optional(ui, "LCB", &mut pct, 50.0, 0.1, 1, "% LWL from aft end") {
                    r.lcb_frac = pct.map(|p| p / 100.0);
                }
                ui.end_row();
                optional(
                    ui,
                    "Design speed",
                    &mut r.design_speed_kn,
                    8.0,
                    0.1,
                    1,
                    "kn",
                );
                ui.end_row();
                optional(ui, "KG estimate", &mut r.kg, 1.0, 0.01, 2, "m");
                ui.end_row();
            });
        ui.separator();
        ui.horizontal(|ui| {
            if let Some(p) = state.problem() {
                ui.colored_label(theme::FAIL, format!("⚠ {p}"));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let ok = ui.add_enabled(state.problem().is_none(), egui::Button::new("Apply"));
                state.rects.create = ok.rect;
                if ok.clicked() {
                    action = state.apply();
                }
                let cancel = ui.button("Cancel");
                state.rects.cancel = cancel.rect;
                if cancel.clicked() {
                    action = ProjectSettingsAction::Cancel;
                }
            });
        });
    });
    if action == ProjectSettingsAction::None
        && modal.is_top_modal
        && !modal.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape))
    {
        action = ProjectSettingsAction::Cancel;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn displacement_and_cb_are_one_target() {
        // 12 × 3.6 × 0.75 m box of sea water is 33.21 t.
        let d = displacement_from_cb(0.5, 1.025, 12.0, 3.6, 0.75);
        assert!((d - 16.605).abs() < 1e-9, "{d}");
        let cb = cb_from_displacement(d, 1.025, 12.0, 3.6, 0.75);
        assert!((cb - 0.5).abs() < 1e-12);
        assert_eq!(cb_from_displacement(1.0, 1.025, 0.0, 3.6, 0.75), 0.0);

        let mut s = NewDesignState::new(SEA_WATER_T_M3);
        assert_eq!(s.displacement_t(), None);
        s.set_displacement_t(12.0);
        let cb = s.cb.unwrap();
        assert!((cb - 12.0 / (1.025 * 12.0 * 3.6 * 0.75)).abs() < 1e-12);
        assert!((s.displacement_t().unwrap() - 12.0).abs() < 1e-9);
        // Δ follows the dimensions; Cb stays.
        s.lwl = 24.0;
        assert!((s.displacement_t().unwrap() - 24.0).abs() < 1e-9);
        assert_eq!(s.requirements().cb, Some(cb));
    }

    #[test]
    fn froude_number_and_speed_range() {
        // 8 kn on 12 m: 4.1156 m/s / √(9.81 · 12) = 0.379.
        assert!((froude_number(8.0, 12.0) - 0.3793).abs() < 1e-3);
        assert_eq!(froude_number(8.0, 0.0), 0.0);
        assert_eq!(speed_range(8.0), (4.0, 12.0, 1.0));
        assert_eq!(speed_range(25.0), (12.5, 37.5, 2.5));
    }

    #[test]
    fn a_kind_switch_refills_only_untouched_fields() {
        let mut s = NewDesignState::new(SEA_WATER_T_M3);
        s.beam = 4.0;
        s.touched.beam = true;
        s.set_kind(HullKind::Planing);
        let d = HullSpec::default_for(HullKind::Planing);
        assert_eq!(s.lwl, d.lwl);
        assert_eq!(s.draft, d.draft);
        assert_eq!(s.transom_frac, d.transom_frac);
        assert_eq!(s.beam, 4.0, "an edited field is kept");
        assert_eq!(s.name, "Planing hull 10 m");
    }

    #[test]
    fn validation_gates_create() {
        let mut s = NewDesignState::new(SEA_WATER_T_M3);
        assert!(s.can_create(), "the defaults create without typing");
        s.cb = Some(0.95);
        assert!(s.problem().unwrap().contains("cb"));
        s.cb = Some(0.5);
        s.depth = 0.5;
        assert!(!s.can_create());
        // Demo and import need no dimensions.
        s.start = StartFrom::Demo;
        assert!(s.can_create());
    }

    #[test]
    fn the_preview_waits_for_edits_to_settle() {
        let mut s = NewDesignState::new(SEA_WATER_T_M3);
        s.inline_preview = true;
        let t0 = Instant::now();
        // First look: generated at once.
        assert_eq!(s.tick(t0), None);
        let first = s.preview().expect("preview").report.clone();
        assert!((first.lwl - 12.0).abs() < 0.12);
        // An edit: nothing until 250 ms have passed.
        s.lwl = 14.0;
        let wait = s.tick(t0).expect("debounced");
        assert!(wait <= PREVIEW_DEBOUNCE);
        assert_eq!(s.preview().unwrap().spec.lwl, 12.0);
        assert_eq!(
            s.tick(t0 + PREVIEW_DEBOUNCE + Duration::from_millis(1)),
            None
        );
        assert_eq!(s.preview().unwrap().spec.lwl, 14.0);
        let rows = compare_rows(&s.preview().unwrap().report);
        assert_eq!(rows[0].name, "LWL");
        assert_eq!(rows[0].target, Some(14.0));
        assert_eq!(rows[3].target, None, "no Δ target without a Cb target");
    }
}
