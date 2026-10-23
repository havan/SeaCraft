//! SeaCraft desktop application.
//!
//! `App` owns the model and the UI state. `App::draw` takes only an
//! [`egui::Ui`], so the whole application can be driven in tests without an
//! `eframe::Frame`; the [`eframe::App`] implementation is a one-line
//! delegation.

use std::path::PathBuf;

use eframe::egui;
use seacraft_core::damage::DamageResult;
use seacraft_core::export::{self, RenderMesh};
use seacraft_core::hydro::{Hull, HydroOptions, Hydrostatics};
use seacraft_core::loading::LoadingResult;
use seacraft_core::math::{Plane, Vec3};
use seacraft_core::project::{self, Project};
use seacraft_core::stability::{self, CrossCurve};
use seacraft_core::{demo, fef, SubdivisionMode, Surface};

use crate::camera::{Camera, ViewKind};
use crate::history::History;
use crate::panels;
use crate::picking::{self, PICK_RADIUS};
use crate::render::{self, DisplayOptions, Scene};

/// Cached refined mesh, invalidated by the revision counter, level or mode.
struct RenderCache {
    revision: u64,
    level: u8,
    mode: SubdivisionMode,
    mesh: RenderMesh,
    curves: Option<seacraft_core::linesplan::LinesplanCurves3d>,
    min: Vec3,
    max: Vec3,
}

/// Live hydrostatics: the floating condition, the last report and the hull
/// cache they were measured from.
struct HydroState {
    options: HydroOptions,
    kg: f64,
    /// Compute the GZ curves free to trim about `lcg`.
    free_trim: bool,
    /// Longitudinal centre of gravity in model x (the station the LCB is held
    /// at while free to trim). `None` until the user sets it: the dock fills it
    /// with the upright LCB, which is the natural value for a bare model.
    lcg: Option<f64>,
    result: Option<Hydrostatics>,
    curves: Option<Vec<CrossCurve>>,
    hull: Option<Hull>,
    /// Revision the cached hull was built from.
    hull_revision: u64,
    /// When the geometry last changed, for the recompute debounce.
    last_edit: Option<std::time::Instant>,
}

impl HydroState {
    fn new() -> Self {
        Self {
            options: HydroOptions {
                draft: 0.62,
                trim: 0.0,
                heel_deg: 0.0,
                water_density: 1025.0,
                appendage_coefficient: 1.0,
                mainframe_x: None,
                stations: 11,
                subdivision: seacraft_core::SubdivisionOptions::default(),
            },
            kg: 2.2,
            free_trim: false,
            lcg: None,
            result: None,
            curves: None,
            hull: None,
            hull_revision: u64::MAX,
            last_edit: None,
        }
    }
}

/// One viewport's rasterized frame, kept until the geometry or camera moves.
struct AnalysisFrame {
    key: u64,
    texture: egui::TextureHandle,
}

/// A control point being dragged in one viewport.
struct DragState {
    point: usize,
    /// Plane through the point, facing that viewport's camera.
    plane: Plane,
    /// Vector from the cursor's plane hit to the point, so it does not jump.
    offset: Vec3,
    view: usize,
    /// Position when grabbed, so a click that never moved can drop its
    /// snapshot again.
    original: Vec3,
}

/// What the File menu can produce besides a `.fef`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportKind {
    Obj,
    Stl,
    ThreeMf,
    Ply,
    HullIges,
    Html,
    Linesplan,
    LinesplanDxf,
    LinesplanIges,
}

impl ExportKind {
    pub const ALL: [ExportKind; 9] = [
        ExportKind::Obj,
        ExportKind::Stl,
        ExportKind::ThreeMf,
        ExportKind::Ply,
        ExportKind::HullIges,
        ExportKind::Html,
        ExportKind::Linesplan,
        ExportKind::LinesplanDxf,
        ExportKind::LinesplanIges,
    ];

    pub fn label(self) -> &'static str {
        match self {
            ExportKind::Obj => "Wavefront OBJ…",
            ExportKind::Stl => "STL (binary)…",
            ExportKind::ThreeMf => "3MF (welded mesh)…",
            ExportKind::Ply => "PLY (ASCII)…",
            ExportKind::HullIges => "Hull IGES (surfaces)…",
            ExportKind::Html => "HTML viewer…",
            ExportKind::Linesplan => "Linesplan SVG…",
            ExportKind::LinesplanDxf => "Linesplan DXF…",
            ExportKind::LinesplanIges => "Linesplan IGES…",
        }
    }

    pub fn extension(self) -> &'static str {
        match self {
            ExportKind::Obj => "obj",
            ExportKind::Stl => "stl",
            ExportKind::ThreeMf => "3mf",
            ExportKind::Ply => "ply",
            ExportKind::HullIges => "igs",
            ExportKind::Html => "html",
            ExportKind::Linesplan => "svg",
            ExportKind::LinesplanDxf => "dxf",
            ExportKind::LinesplanIges => "igs",
        }
    }
}

/// Which file dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    Open,
    SaveAs,
    Export(ExportKind),
}

/// An action that may need to discard unsaved work first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    NewDemo,
    Open,
    Quit,
}

/// The in-app file dialog. A native dialog needs a desktop portal, which a
/// plain X session or a test harness does not have, so the path is typed (or
/// picked from the directory listing) in a modal window.
struct FileDialogState {
    kind: DialogKind,
    path: String,
    error: Option<String>,
}

/// Reference to a selected project tank or compartment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeRef {
    Tank(usize),
    Compartment(usize),
}

/// The tab shown in the unified right-hand design dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DesignDock {
    Loading,
    Damage,
    Windage,
}

pub struct App {
    surface: Option<Surface>,
    path: Option<PathBuf>,
    /// Design data kept in the `<model>.seacraft.json` sidecar.
    project: Project,
    /// Why the sidecar could not be read; while set, saving leaves it alone.
    project_error: Option<String>,
    status: String,
    /// Bumped by every edit; keys all derived state.
    revision: u64,
    level: u8,
    mode: SubdivisionMode,
    display: DisplayOptions,
    cameras: [Camera; 4],
    active: usize,
    cache: Option<RenderCache>,
    selected: Option<usize>,
    selected_volume: Option<VolumeRef>,
    selected_windage: Option<usize>,
    last_highlighted_tank: Option<usize>,
    last_highlighted_compartment: Option<usize>,
    history: History,
    marks: Vec<(u64, Option<Surface>, usize)>,
    next_mark_id: u64,
    drag: Option<DragState>,
    /// Unsaved edits since the last save / load.
    dirty: bool,
    dialog: Option<FileDialogState>,
    pending: Option<PendingAction>,
    /// A quit that has been confirmed (the window closes at the end of the frame).
    close_requested: bool,
    show_layers: bool,
    show_hydro: bool,
    design_dock: Option<DesignDock>,
    loading_selected: usize,
    loading_result: Option<LoadingResult>,
    loading_stale: bool,
    loading_worker: Option<std::sync::mpsc::Receiver<(u64, LoadingResult)>>,
    loading_generation: u64,
    damage_selected: usize,
    damage_result: Option<DamageResult>,
    damage_stale: bool,
    damage_worker: Option<std::sync::mpsc::Receiver<(u64, Result<DamageResult, String>)>>,
    damage_generation: u64,
    /// Cursor position on the active viewport's focal plane, for the status bar.
    cursor_world: Option<Vec3>,
    /// Geometry changed since the hydrostatics were last computed.
    hydro_dirty: bool,
    hydro: HydroState,
    /// Viewport rectangles from the last frame, so tests can aim the pointer.
    viewport_rects: [egui::Rect; 4],
    /// Rasterized frames for the analysis modes, one per viewport.
    analysis_frames: [Option<AnalysisFrame>; 4],
    /// Requests from the loopback control socket (`--control`), served once
    /// per frame by [`App::process_control`].
    control: Option<std::sync::mpsc::Receiver<crate::control::ControlRequest>>,
    /// Synthetic input queued by the control socket, injected into the next
    /// frame by [`App::raw_input_hook`].
    synthetic: Vec<egui::Event>,
    /// Allowed read and write directory roots for the control channel.
    control_roots: seacraft_core::control_auth::ControlRoots,
    /// Token file removed on clean shutdown.
    token_file: Option<seacraft_core::control_auth::TokenFile>,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let mut app = Self::new_detached();
        app.status = "No model loaded. File > Load demo hull.".to_owned();
        if let Some(p) = open {
            app.load(p);
        }
        app
    }

    /// An app for tests and headless frames: loads `path` if given, otherwise
    /// the built-in demo hull.
    pub fn headless_for_test(path: Option<&str>) -> Self {
        let mut app = Self::new_detached();
        match path {
            Some(p) => app.load(PathBuf::from(p)),
            None => app.load_demo(),
        }
        app
    }

    fn new_detached() -> Self {
        Self {
            surface: None,
            path: None,
            project: Project::default(),
            project_error: None,
            status: String::new(),
            revision: 0,
            level: 3,
            mode: SubdivisionMode::Loop,
            display: DisplayOptions::default(),
            cameras: [
                Camera::new(ViewKind::Perspective, Vec3::ZERO, Vec3::new(12.0, 4.0, 2.0)),
                Camera::new(ViewKind::Profile, Vec3::ZERO, Vec3::new(12.0, 4.0, 2.0)),
                Camera::new(ViewKind::Plan, Vec3::ZERO, Vec3::new(12.0, 4.0, 2.0)),
                Camera::new(ViewKind::Body, Vec3::ZERO, Vec3::new(12.0, 4.0, 2.0)),
            ],
            active: 0,
            cache: None,
            selected: None,
            selected_volume: None,
            selected_windage: None,
            last_highlighted_tank: None,
            last_highlighted_compartment: None,
            history: History::new(64),
            marks: Vec::new(),
            next_mark_id: 1,
            drag: None,
            dirty: false,
            dialog: None,
            pending: None,
            close_requested: false,
            show_layers: true,
            show_hydro: true,
            design_dock: None,
            loading_selected: 0,
            loading_result: None,
            loading_stale: true,
            loading_worker: None,
            loading_generation: 0,
            damage_selected: 0,
            damage_result: None,
            damage_stale: true,
            damage_worker: None,
            damage_generation: 0,
            cursor_world: None,
            hydro_dirty: true,
            hydro: HydroState::new(),
            viewport_rects: [egui::Rect::NOTHING; 4],
            analysis_frames: [None, None, None, None],
            control: None,
            synthetic: Vec::new(),
            control_roots: seacraft_core::control_auth::ControlRoots::default(),
            token_file: None,
        }
    }

    /// Load a `.fef` model, reporting failures in the status bar instead of
    /// panicking: a bad file must never take the application down.
    pub fn load(&mut self, path: PathBuf) {
        let _ = self.open_model(&path);
    }

    /// Load a `.fef` model, returning the status-bar message on failure.
    pub fn open_model(&mut self, path: &std::path::Path) -> Result<(), String> {
        let parsed = std::fs::read_to_string(path)
            .map_err(|err| format!("Cannot read {}: {err}", path.display()))
            .and_then(|text| {
                fef::parse(&text).map_err(|err| format!("Parse error in {}: {err}", path.display()))
            });
        match parsed {
            Ok(surface) => {
                let (project, project_error) = match Project::load_for_model(path) {
                    Ok(p) => (p.unwrap_or_default(), None),
                    Err(e) => (Project::default(), Some(e)),
                };
                let layers_word = if surface.layers.len() == 1 {
                    "layer"
                } else {
                    "layers"
                };
                self.status = format!(
                    "{} — {} points, {} faces, {} {}",
                    surface.settings.name,
                    surface.points.len(),
                    surface.faces.len(),
                    surface.layers.len(),
                    layers_word
                );
                if let Some(e) = &project_error {
                    self.status = format!("{} — project file not loaded: {e}", self.status);
                } else if !project.is_empty() {
                    self.status.push_str(" + project");
                }
                self.surface = Some(surface);
                self.path = Some(path.to_path_buf());
                self.project = project;
                self.project_error = project_error;
                self.dirty = false;
                self.adopt();
                Ok(())
            }
            Err(err) => {
                self.status = err.clone();
                Err(err)
            }
        }
    }

    pub fn load_demo(&mut self) {
        self.load_demo_shape(false);
    }

    /// The built-in demo hull, full or as a symmetric half model (only
    /// `y >= 0` modelled, mirrored on evaluation).
    pub fn load_demo_shape(&mut self, symmetric: bool) {
        let surface = demo::demo_hull(symmetric);
        let half = if symmetric { " (symmetric)" } else { "" };
        self.status = format!(
            "{} — built-in demo hull{half} ({} points, {} faces)",
            surface.settings.name,
            surface.points.len(),
            surface.faces.len()
        );
        self.surface = Some(surface);
        self.path = None;
        self.project = Project::default();
        self.project_error = None;
        self.dirty = true;
        self.adopt();
    }

    /// Replace the current model with a newly generated parametric hull.
    pub fn load_generated(&mut self, surface: Surface) {
        self.status = format!(
            "{} — parametric hull ({} points, {} faces)",
            surface.settings.name,
            surface.points.len(),
            surface.faces.len()
        );
        self.surface = Some(surface);
        self.path = None;
        self.project = Project::default();
        self.project_error = None;
        self.dirty = true;
        self.adopt();
    }

    /// Reset the derived state after a model change.
    fn adopt(&mut self) {
        self.revision += 1;
        self.selected = None;
        self.selected_volume = None;
        self.selected_windage = None;
        self.last_highlighted_tank = None;
        self.last_highlighted_compartment = None;
        self.drag = None;
        self.cache = None;
        self.history.clear();
        self.marks.clear();
        self.hydro.result = None;
        self.hydro.curves = None;
        self.hydro.hull = None;
        self.hydro.hull_revision = u64::MAX;
        self.hydro_dirty = true;
        self.hydro.last_edit = Some(std::time::Instant::now());
        self.loading_stale = true;
        self.loading_selected = 0;
        self.loading_result = None;
        self.loading_worker = None;
        self.damage_stale = true;
        self.damage_selected = 0;
        self.damage_result = None;
        self.damage_worker = None;
        self.cursor_world = None;
        self.analysis_frames = [None, None, None, None];
        if let Some(s) = &self.surface {
            let subdiv = seacraft_core::SubdivisionOptions {
                level: self.level,
                mode: self.mode,
            };
            self.hydro.options = HydroOptions::from_surface(s, subdiv);
            let (min, max) = s.extents();
            for cam in &mut self.cameras {
                cam.frame(min, max);
            }
        }
    }

    /// Refine the hull when anything it depends on changed. While a point is
    /// being dragged the preview drops to level 2; the full level returns when
    /// the drag ends.
    fn sync_cache(&mut self) {
        let Some(surface) = &self.surface else {
            self.cache = None;
            return;
        };
        let level = if self.drag.is_some() {
            self.level.min(2)
        } else {
            self.level
        };
        let stale = match &self.cache {
            Some(c) => c.revision != self.revision || c.level != level || c.mode != self.mode,
            None => true,
        };
        if stale {
            let mesh = export::render_mesh(surface, level, self.mode);
            let (min, max) = surface.extents();
            self.cache = Some(RenderCache {
                revision: self.revision,
                level,
                mode: self.mode,
                mesh,
                curves: None,
                min,
                max,
            });
        }
        if self.display.combs {
            if let Some(cache) = self.cache.as_mut() {
                if cache.curves.is_none() {
                    let opts = seacraft_core::linesplan::LinesplanOptions {
                        level,
                        mode: self.mode,
                        combs: true,
                        ..Default::default()
                    };
                    cache.curves = Some(seacraft_core::linesplan::curves_3d(surface, opts));
                }
            }
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.shortcuts(ui);
        self.menu(ui);
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.dirty {
                    ui.colored_label(egui::Color32::from_rgb(0xe8, 0x9d, 0x3a), "● unsaved");
                }
                ui.label(&self.status);
                ui.separator();
                ui.label(format!(
                    "L{} {}",
                    self.level,
                    match self.mode {
                        SubdivisionMode::Loop => "Loop",
                        SubdivisionMode::CatmullClark => "Catmull-Clark",
                    },
                ));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("v{}", seacraft_core::VERSION));
                    if let Some(p) = self.cursor_world {
                        ui.monospace(format!("x {:.2}  y {:.2}  z {:.2}", p.x, p.y, p.z));
                    }
                });
            });
        });

        if self.show_layers {
            let mut changed = false;
            egui::Panel::left("layer_panel")
                .resizable(true)
                .default_size(250.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        if let Some(surface) = self.surface.as_mut() {
                            changed = panels::layers(ui, surface);
                        } else {
                            ui.label("No model loaded.");
                        }
                    });
                });
            if changed {
                self.revision += 1;
                self.dirty = true;
                self.touch_geometry();
            }
        }

        if self.show_hydro {
            let action = egui::Panel::right("hydro_panel")
                .resizable(true)
                .default_size(340.0)
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .show(ui, |ui| {
                            panels::hydrostatics(
                                ui,
                                panels::HydroPanel {
                                    options: &mut self.hydro.options,
                                    kg: &mut self.hydro.kg,
                                    free_trim: &mut self.hydro.free_trim,
                                    lcg: &mut self.hydro.lcg,
                                    result: self.hydro.result.as_ref(),
                                    stale: self.hydro_dirty,
                                    curves: self.hydro.curves.as_deref(),
                                },
                            )
                        })
                        .inner
                })
                .inner;
            match action {
                panels::HydroAction::Recompute => {
                    self.hydro_dirty = true;
                    self.refresh_hydro(true);
                }
                panels::HydroAction::CrossCurves => self.compute_curves(),
                panels::HydroAction::None => {}
            }
            if self.hydro_dirty {
                ui.ctx()
                    .request_repaint_after(std::time::Duration::from_millis(120));
            }
        }
        self.refresh_hydro(false);

        if let Some(mut dock) = self.design_dock {
            let (tank_highlight, scroll_to_tank) = match self.selected_volume {
                Some(VolumeRef::Tank(idx)) => {
                    let scroll = self.last_highlighted_tank != Some(idx);
                    (Some(idx), scroll)
                }
                _ => (None, false),
            };
            let (comp_highlight, scroll_to_comp) = match self.selected_volume {
                Some(VolumeRef::Compartment(idx)) => {
                    let scroll = self.last_highlighted_compartment != Some(idx);
                    (Some(idx), scroll)
                }
                _ => (None, false),
            };

            egui::Panel::right("design_dock")
                .resizable(true)
                .default_size(380.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui
                            .selectable_label(dock == DesignDock::Loading, "Loading")
                            .clicked()
                        {
                            dock = DesignDock::Loading;
                        }
                        if ui
                            .selectable_label(dock == DesignDock::Damage, "Damage")
                            .clicked()
                        {
                            dock = DesignDock::Damage;
                        }
                        if ui
                            .selectable_label(dock == DesignDock::Windage, "Windage")
                            .clicked()
                        {
                            dock = DesignDock::Windage;
                        }
                    });
                    ui.separator();

                    egui::ScrollArea::vertical().show(ui, |ui| match dock {
                        DesignDock::Loading => {
                            let action = panels::loading(
                                ui,
                                panels::LoadingPanel {
                                    project: &mut self.project,
                                    selected: &mut self.loading_selected,
                                    result: self.loading_result.as_ref(),
                                    stale: self.loading_stale,
                                    busy: self.loading_worker.is_some(),
                                    highlight: tank_highlight,
                                    scroll_to: scroll_to_tank,
                                },
                            );
                            match action {
                                panels::LoadingAction::Changed => {
                                    self.dirty = true;
                                    self.loading_stale = true;
                                }
                                panels::LoadingAction::Compute => {
                                    self.start_loading_compute();
                                }
                                panels::LoadingAction::None => {}
                            }
                        }
                        DesignDock::Damage => {
                            let action = panels::damage(
                                ui,
                                panels::DamagePanel {
                                    project: &mut self.project,
                                    selected: &mut self.damage_selected,
                                    result: self.damage_result.as_ref(),
                                    stale: self.damage_stale,
                                    busy: self.damage_worker.is_some(),
                                    highlight: comp_highlight,
                                    scroll_to: scroll_to_comp,
                                },
                            );
                            match action {
                                panels::DamageAction::Changed => {
                                    self.dirty = true;
                                    self.damage_stale = true;
                                }
                                panels::DamageAction::Compute => {
                                    self.start_damage_compute();
                                }
                                panels::DamageAction::None => {}
                            }
                        }
                        DesignDock::Windage => {
                            let action = panels::windage(
                                ui,
                                panels::WindagePanel {
                                    project: &mut self.project,
                                    weather: self
                                        .loading_result
                                        .as_ref()
                                        .and_then(|r| r.weather.as_ref()),
                                    stale: self.loading_stale,
                                    selected: &mut self.selected_windage,
                                },
                            );
                            match action {
                                panels::WindageAction::Changed => {
                                    self.dirty = true;
                                    self.loading_stale = true;
                                }
                                panels::WindageAction::None => {}
                            }
                        }
                    });
                });

            self.design_dock = Some(dock);
            self.last_highlighted_tank = tank_highlight;
            self.last_highlighted_compartment = comp_highlight;
        }

        if let Some(rx) = &self.loading_worker {
            let mut done = false;
            match rx.try_recv() {
                Ok((gen, res)) => {
                    done = true;
                    if gen == self.loading_generation {
                        self.loading_result = Some(res);
                        self.loading_stale = false;
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    done = true;
                }
            }
            if done {
                self.loading_worker = None;
            }
        }

        if let Some(rx) = &self.damage_worker {
            let mut done = false;
            match rx.try_recv() {
                Ok((gen, res)) => {
                    done = true;
                    if gen == self.damage_generation {
                        match res {
                            Ok(r) => {
                                self.damage_result = Some(r);
                                self.damage_stale = false;
                            }
                            Err(e) => {
                                self.status = format!("Damage compute failed: {e}");
                            }
                        }
                    }
                }
                Err(std::sync::mpsc::TryRecvError::Empty) => {
                    ui.ctx()
                        .request_repaint_after(std::time::Duration::from_millis(100));
                }
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    done = true;
                }
            }
            if done {
                self.damage_worker = None;
            }
        }

        egui::CentralPanel::default().show(ui, |ui| self.viewports(ui));
        self.dialog_window(ui);
        self.pending_window(ui);
        if self.close_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        // Read the modifiers from the key event itself: `InputState::modifiers`
        // is only refreshed by a `ModifiersChanged` event, which a synthesised
        // test frame need not send.
        let (undo, redo, escape, frame) = ui.input(|i| {
            let mut undo = false;
            let mut redo = false;
            for event in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } = event
                {
                    match key {
                        egui::Key::Z if modifiers.command || modifiers.ctrl => {
                            if modifiers.shift {
                                redo = true;
                            } else {
                                undo = true;
                            }
                        }
                        _ => {}
                    }
                }
            }
            (
                undo,
                redo,
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::F),
            )
        });
        if undo {
            self.undo();
        }
        if redo {
            self.redo();
        }
        if escape {
            self.selected = None;
            self.selected_volume = None;
            self.selected_windage = None;
        }
        if frame {
            self.reset_cameras();
        }
        if ui.input(|i| i.key_pressed(egui::Key::Space)) {
            self.display.shaded = !self.display.shaded;
        }
        for (i, key) in [
            egui::Key::Num1,
            egui::Key::Num2,
            egui::Key::Num3,
            egui::Key::Num4,
        ]
        .into_iter()
        .enumerate()
        {
            if ui.input(|i| i.key_pressed(key)) {
                self.active = i;
            }
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui.button("New demo hull").clicked() {
                        self.request(PendingAction::NewDemo);
                        ui.close();
                    }
                    if ui.button("Open .fef…").clicked() {
                        self.request(PendingAction::Open);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(self.surface.is_some(), egui::Button::new("Save"))
                        .clicked()
                    {
                        self.save();
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.surface.is_some(), egui::Button::new("Save as…"))
                        .clicked()
                    {
                        self.open_dialog(DialogKind::SaveAs);
                        ui.close();
                    }
                    ui.separator();
                    ui.add_enabled_ui(self.surface.is_some(), |ui| {
                        ui.menu_button("Export", |ui| {
                            for kind in ExportKind::ALL {
                                if ui.button(kind.label()).clicked() {
                                    self.open_dialog(DialogKind::Export(kind));
                                    ui.close();
                                }
                            }
                        });
                    });
                    ui.separator();
                    if ui.button("Quit").clicked() {
                        self.request(PendingAction::Quit);
                        ui.close();
                    }
                });
                ui.menu_button("Edit", |ui| {
                    if ui
                        .add_enabled(self.history.can_undo(), egui::Button::new("Undo  (Ctrl+Z)"))
                        .clicked()
                    {
                        self.undo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.history.can_redo(),
                            egui::Button::new("Redo  (Ctrl+Shift+Z)"),
                        )
                        .clicked()
                    {
                        self.redo();
                        ui.close();
                    }
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.display.shaded, "Shaded hull");
                    ui.checkbox(&mut self.display.wireframe, "Wireframe");
                    ui.checkbox(&mut self.display.control_net, "Control net");
                    ui.checkbox(&mut self.display.points, "Control points");
                    ui.checkbox(&mut self.display.waterplane, "Waterplane");
                    ui.checkbox(&mut self.display.volumes, "Tanks & compartments");
                    ui.checkbox(&mut self.display.windage, "Windage outlines");
                    ui.checkbox(&mut self.display.grid, "Grid");
                    ui.checkbox(&mut self.display.combs, "Curvature combs");
                    ui.separator();
                    ui.label("Surface analysis");
                    ui.horizontal_wrapped(|ui| {
                        let current = self.display.analysis;
                        for (mode, label) in [
                            (seacraft_core::AnalysisMode::Shaded, "Shaded"),
                            (
                                seacraft_core::AnalysisMode::Zebra { stripes: 14.0 },
                                "Zebra",
                            ),
                            (
                                seacraft_core::AnalysisMode::Curvature { radius: 1.0 },
                                "Curvature",
                            ),
                            (
                                seacraft_core::AnalysisMode::Draft { angle_deg: 3.0 },
                                "Draft",
                            ),
                        ] {
                            if ui.selectable_label(current == mode, label).clicked() {
                                self.display.analysis = mode;
                            }
                        }
                    });
                    ui.separator();
                    ui.checkbox(&mut self.show_layers, "Layer panel");
                    ui.checkbox(&mut self.show_hydro, "Hydrostatics panel");
                    // The loading, damage and windage panels share one dock:
                    // multiple right-hand panels leave no room for the viewports.
                    let mut loading = self.show_loading();
                    if ui.checkbox(&mut loading, "Loading panel").changed() {
                        self.set_show_loading(loading);
                    }
                    let mut damage = self.show_damage();
                    if ui.checkbox(&mut damage, "Damage panel").changed() {
                        self.set_show_damage(damage);
                    }
                    let mut windage = self.show_windage();
                    if ui.checkbox(&mut windage, "Windage panel").changed() {
                        self.set_show_windage(windage);
                    }
                    ui.separator();
                    let mut level = self.level as u32;
                    if ui
                        .add(egui::Slider::new(&mut level, 0..=5).text("subdivision level"))
                        .changed()
                    {
                        self.level = level as u8;
                    }
                    egui::ComboBox::from_label("mode")
                        .selected_text(match self.mode {
                            SubdivisionMode::Loop => "Loop",
                            SubdivisionMode::CatmullClark => "Catmull-Clark",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut self.mode, SubdivisionMode::Loop, "Loop");
                            ui.selectable_value(
                                &mut self.mode,
                                SubdivisionMode::CatmullClark,
                                "Catmull-Clark",
                            );
                        });
                    ui.separator();
                    if ui.button("Reset cameras (F)").clicked() {
                        self.reset_cameras();
                        ui.close();
                    }
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("SeaCraft {}", seacraft_core::VERSION));
                    ui.label("A Rust reimplementation of FREE!ship.");
                });
            });
        });
    }

    // ---- file actions --------------------------------------------------

    /// Run `action`, asking first when there are unsaved edits.
    fn request(&mut self, action: PendingAction) {
        if self.dirty {
            self.pending = Some(action);
        } else {
            self.perform(action);
        }
    }

    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::NewDemo => self.load_demo(),
            PendingAction::Open => self.open_dialog(DialogKind::Open),
            PendingAction::Quit => self.close_requested = true,
        }
    }

    /// Path to pre-fill a dialog with.
    fn suggested_path(&self, kind: DialogKind) -> String {
        let stem = self
            .path
            .as_ref()
            .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_string()))
            .or_else(|| self.surface.as_ref().map(|s| s.settings.name.clone()))
            .unwrap_or_else(|| "hull".to_owned());
        let clean: String = stem
            .chars()
            .map(|c| {
                if c.is_alphanumeric() || c == '-' || c == '_' {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        let dir = self
            .path
            .as_ref()
            .and_then(|p| p.parent().map(|d| d.to_path_buf()))
            .unwrap_or_else(|| std::path::PathBuf::from("."));
        match kind {
            DialogKind::Open => {
                let demo = std::path::PathBuf::from("examples/demo-hull.fef");
                if self.path.is_none() && demo.exists() {
                    return demo.to_string_lossy().to_string();
                }
                self.path
                    .as_ref()
                    .map(|p| p.to_string_lossy().to_string())
                    .unwrap_or_else(|| demo.to_string_lossy().to_string())
            }
            DialogKind::SaveAs => dir
                .join(format!("{clean}.fef"))
                .to_string_lossy()
                .to_string(),
            DialogKind::Export(kind) => dir
                .join(format!("{clean}.{}", kind.extension()))
                .to_string_lossy()
                .to_string(),
        }
    }

    fn open_dialog(&mut self, kind: DialogKind) {
        let path = self.suggested_path(kind);
        self.dialog = Some(FileDialogState {
            kind,
            path,
            error: None,
        });
    }

    /// Save to the model's path, or ask for one.
    pub fn save(&mut self) {
        match self.path.clone() {
            Some(path) => {
                self.save_to(&path);
            }
            None => self.open_dialog(DialogKind::SaveAs),
        }
    }

    /// Write the model as `.fef`. Returns whether it was written.
    pub fn save_to(&mut self, path: &std::path::Path) -> bool {
        let Some(surface) = &self.surface else {
            return false;
        };
        let text = fef::write(surface);
        match std::fs::write(path, text) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.status = format!("saved {}", path.display());
                let sidecar = project::sidecar_path(path);
                if let Some(e) = &self.project_error {
                    self.status = format!(
                        "saved {}; project file left as it was ({e})",
                        path.display()
                    );
                } else if !self.project.is_empty() || sidecar.exists() {
                    if let Err(e) = self
                        .project
                        .validate()
                        .and_then(|()| self.project.save(&sidecar))
                    {
                        self.status = format!("saved {}, but {e}", path.display());
                        return false;
                    }
                    self.status.push_str(" + project");
                }
                self.dirty = false;
                true
            }
            Err(err) => {
                self.status = format!("cannot write {}: {err}", path.display());
                false
            }
        }
    }

    /// Write one of the export formats. Returns whether it was written.
    pub fn export_to(&mut self, kind: ExportKind, path: &std::path::Path) -> bool {
        let Some(surface) = &self.surface else {
            return false;
        };
        let written = match kind {
            ExportKind::Obj => std::fs::write(
                path,
                export::obj(surface, self.level, self.mode, export::UpAxis::Z),
            ),
            ExportKind::Stl => std::fs::write(path, export::stl(surface, self.level, self.mode)),
            ExportKind::ThreeMf => {
                std::fs::write(path, export::threemf(surface, self.level, self.mode))
            }
            ExportKind::Ply => std::fs::write(path, export::ply(surface, self.level, self.mode)),
            ExportKind::HullIges => {
                std::fs::write(path, export::iges_surface(surface, self.level, self.mode))
            }
            ExportKind::Html => {
                std::fs::write(path, export::viewer_html(surface, self.level, self.mode))
            }
            ExportKind::Linesplan => std::fs::write(
                path,
                seacraft_core::linesplan::svg(
                    surface,
                    seacraft_core::linesplan::LinesplanOptions::default(),
                ),
            ),
            ExportKind::LinesplanDxf => std::fs::write(
                path,
                seacraft_core::linesplan::dxf(
                    surface,
                    seacraft_core::linesplan::LinesplanOptions::default(),
                ),
            ),
            ExportKind::LinesplanIges => std::fs::write(
                path,
                seacraft_core::linesplan::iges(
                    surface,
                    seacraft_core::linesplan::LinesplanOptions::default(),
                ),
            ),
        };
        match written {
            Ok(()) => {
                self.status = format!(
                    "exported {} ({})",
                    path.display(),
                    kind.label().trim_end_matches('…')
                );
                true
            }
            Err(err) => {
                self.status = format!("cannot write {}: {err}", path.display());
                false
            }
        }
    }

    fn dialog_window(&mut self, ui: &mut egui::Ui) {
        let Some(mut dialog) = self.dialog.take() else {
            return;
        };
        let title = match dialog.kind {
            DialogKind::Open => "Open .fef",
            DialogKind::SaveAs => "Save as",
            DialogKind::Export(k) => k.label(),
        };
        let mut accept = false;
        let mut cancel = false;
        egui::Window::new(title)
            .collapsible(false)
            .resizable(true)
            .default_width(520.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label("Path");
                ui.add(egui::TextEdit::singleline(&mut dialog.path).desired_width(f32::INFINITY));
                if let Some(err) = &dialog.error {
                    ui.colored_label(egui::Color32::from_rgb(0xe0, 0x5c, 0x5c), err);
                }
                ui.separator();
                let wanted = match dialog.kind {
                    DialogKind::Open | DialogKind::SaveAs => ".fef".to_owned(),
                    DialogKind::Export(k) => format!(".{}", k.extension()),
                };
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (label, path) in listing(&dialog.path, &wanted) {
                            if ui.selectable_label(false, label).clicked() {
                                dialog.path = path;
                            }
                        }
                    });
                ui.separator();
                ui.horizontal(|ui| {
                    if ui.button("Cancel").clicked() {
                        cancel = true;
                    }
                    if ui.button("OK").clicked() {
                        accept = true;
                    }
                });
            });

        if cancel {
            return;
        }
        if !accept {
            self.dialog = Some(dialog);
            return;
        }
        let path = std::path::PathBuf::from(dialog.path.trim());
        let ok = match dialog.kind {
            DialogKind::Open => {
                self.load(path);
                self.dialog_open_error()
            }
            DialogKind::SaveAs => self.save_to(&path),
            DialogKind::Export(kind) => self.export_to(kind, &path),
        };
        if !ok {
            dialog.error = Some(self.status.clone());
            self.dialog = Some(dialog);
        }
    }

    /// `load` reports failure in the status bar; turn that into a flag.
    fn dialog_open_error(&self) -> bool {
        !self.status.starts_with("Parse error") && !self.status.starts_with("Cannot read")
    }

    fn pending_window(&mut self, ui: &mut egui::Ui) {
        let Some(action) = self.pending else { return };
        let mut choice: Option<Option<PendingAction>> = None; // Some(None) = cancel
        egui::Window::new("Unsaved changes")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), |ui| {
                ui.label("There are unsaved changes. Save them first?");
                ui.horizontal(|ui| {
                    if ui.button("Save").clicked() {
                        choice = Some(Some(action));
                    }
                    if ui.button("Discard").clicked() {
                        choice = Some(None);
                        self.pending = None;
                        self.perform(action);
                    }
                    if ui.button("Cancel").clicked() {
                        choice = Some(None);
                    }
                });
            });
        match choice {
            Some(Some(action)) => {
                self.save();
                if !self.dirty {
                    self.pending = None;
                    self.perform(action);
                }
            }
            Some(None) => self.pending = None,
            None => {}
        }
    }

    pub fn reset_cameras(&mut self) {
        if let Some(s) = &self.surface {
            let (min, max) = s.extents();
            for cam in &mut self.cameras {
                cam.frame(min, max);
            }
        }
    }

    pub fn can_undo(&self) -> bool {
        self.history.can_undo()
    }

    pub fn can_redo(&self) -> bool {
        self.history.can_redo()
    }

    pub fn undo(&mut self) {
        let Some(current) = self.surface.clone() else {
            return;
        };
        if let Some(previous) = self.history.undo(&current) {
            let moved = previous.points.len();
            self.surface = Some(previous);
            self.revision += 1;
            self.dirty = true;
            self.touch_geometry();
            self.status = format!("undo — {moved} control points restored");
        }
    }

    pub fn redo(&mut self) {
        let Some(current) = self.surface.clone() else {
            return;
        };
        if let Some(next) = self.history.redo(&current) {
            self.surface = Some(next);
            self.revision += 1;
            self.dirty = true;
            self.touch_geometry();
            self.status = "redo".to_owned();
        }
    }

    /// Snapshot the current state as a mark for rollback, returning a mark ID.
    pub fn mark(&mut self) -> u64 {
        let id = self.next_mark_id;
        self.next_mark_id += 1;
        self.marks
            .push((id, self.surface.clone(), self.history.undo_len()));
        id
    }

    /// Restore the state to the given mark ID, dropping all marks taken after it.
    pub fn restore_mark(&mut self, id: u64) -> Result<(), String> {
        let pos = self
            .marks
            .iter()
            .position(|(m, _, _)| *m == id)
            .ok_or_else(|| format!("unknown mark {id}"))?;
        let (_, surface, undo_len) = self.marks[pos].clone();
        self.marks.truncate(pos);
        self.surface = surface;
        self.history.truncate_undo(undo_len);
        self.history.clear_redo();
        self.selected = None;
        self.mark_geometry_changed();
        self.status = format!("restored to mark {id}");
        Ok(())
    }

    /// Drop a mark without restoring.
    pub fn forget_mark(&mut self, id: u64) {
        if let Some(pos) = self.marks.iter().position(|(m, _, _)| *m == id) {
            self.marks.truncate(pos);
        }
    }

    fn viewports(&mut self, ui: &mut egui::Ui) {
        self.sync_cache();
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let cells = render::quad(rect);
        let mut fresh: Vec<(usize, AnalysisFrame)> = Vec::new();
        let overlays = self.volume_overlays();
        let mut volume_overlays: Vec<render::VolumeOverlay> =
            overlays.iter().map(|(_, o)| o.clone()).collect();
        let mut windage_overlays = self.windage_overlays();
        let windage_indices: Vec<usize> = self
            .project
            .windage
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| item.profile.map(|_| idx))
            .collect();

        for (cell, index) in cells {
            self.viewport_rects[index] = cell;
            let response = ui.allocate_rect(cell, egui::Sense::click_and_drag());
            let view = self.cameras[index].transform();

            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.0 {
                    self.cameras[index].zoom((-scroll as f64 * 0.0025).exp());
                }
            }

            let secondary = ui.input(|i| i.pointer.secondary_down());
            if response.dragged() && secondary && self.cameras[index].kind.is_perspective() {
                let delta = response.drag_delta();
                self.cameras[index].orbit(delta.x, -delta.y);
            } else {
                // Grab on *press*, not when egui decides it is a drag: the
                // pick must use the pixel the user aimed at, not where the
                // cursor has already slid to.
                if response.hovered() && ui.input(|i| i.pointer.primary_pressed()) {
                    self.begin_drag(ui, &view, cell, index);
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    if self.drag.as_ref().is_some_and(|d| d.view == index) {
                        self.drag_to(ui, &view, cell);
                    } else if self.drag.is_none() {
                        let delta = response.drag_delta();
                        self.cameras[index].pan(delta.x, delta.y, cell);
                    }
                }
                if ui.input(|i| i.pointer.any_released()) {
                    if let Some(drag) = self.drag.take_if(|d| d.view == index) {
                        let moved = self
                            .surface
                            .as_ref()
                            .map(|s| (s.points[drag.point].p - drag.original).length() > 1e-12)
                            .unwrap_or(false);
                        if moved {
                            // Full refinement and measurement again.
                            self.revision += 1;
                            self.status = "control point moved".to_owned();
                        } else {
                            self.history.discard_last();
                        }
                    }
                }
            }
            if response.clicked() {
                self.active = index;
                self.selected = self.pick_at(ui, &view, cell);
                if self.selected.is_none() {
                    let cursor = ui.input(|i| i.pointer.interact_pos());
                    let mut hit_windage = false;
                    if self.display.windage && self.cameras[index].kind == ViewKind::Profile {
                        if let Some(cursor) = cursor {
                            if let Some(hit) =
                                render::pick_windage(&view, cell, cursor, &windage_overlays)
                            {
                                self.selected_windage = Some(windage_indices[hit]);
                                self.selected_volume = None;
                                self.set_show_windage(true);
                                hit_windage = true;
                            }
                        }
                    }
                    if !hit_windage {
                        self.selected_windage = None;
                        if self.display.volumes {
                            if let Some(cursor) = cursor {
                                if let Some(hit) =
                                    render::pick_volume(&view, cell, cursor, &volume_overlays)
                                {
                                    let vref = overlays[hit].0;
                                    self.selected_volume = Some(vref);
                                    match vref {
                                        VolumeRef::Tank(_) => self.set_show_loading(true),
                                        VolumeRef::Compartment(_) => self.set_show_damage(true),
                                    }
                                } else {
                                    self.selected_volume = None;
                                }
                            } else {
                                self.selected_volume = None;
                            }
                        } else {
                            self.selected_volume = None;
                        }
                    }
                } else {
                    self.selected_windage = None;
                    self.selected_volume = None;
                }
                for (idx, overlay) in windage_indices.iter().zip(windage_overlays.iter_mut()) {
                    overlay.selected = self.selected_windage == Some(*idx);
                }
                for (vref, overlay) in overlays.iter().zip(volume_overlays.iter_mut()) {
                    overlay.selected = self.selected_volume == Some(vref.0);
                }
            }
            if self.active == index && response.hovered() {
                if let Some(cursor) = ui.input(|i| i.pointer.hover_pos()) {
                    let center = self
                        .cache
                        .as_ref()
                        .map_or(Vec3::ZERO, |c| (c.min + c.max) * 0.5);
                    let plane = picking::drag_plane(&view, center);
                    self.cursor_world = picking::ray_plane(&view.ray(cursor, cell), &plane);
                }
            } else if self.active == index {
                self.cursor_world = None;
            }

            if let (Some(surface), Some(cache)) = (&self.surface, &self.cache) {
                let scene = Scene {
                    surface,
                    mesh: &cache.mesh,
                    curves: cache.curves.as_ref(),
                    volumes: &volume_overlays,
                    windage: &windage_overlays,
                    min: cache.min,
                    max: cache.max,
                    draft: self.hydro.options.draft,
                    trim: self.hydro.options.trim,
                    heel_deg: self.hydro.options.heel_deg,
                };
                // An analysis mode needs per-pixel shading, which only the
                // rasterizer provides: render it into a texture and reuse it
                // until the model, camera, size or mode changes.
                let mode = self.display.analysis;
                let texture = if mode.is_shaded() {
                    None
                } else {
                    let key = render::frame_key(
                        self.revision,
                        &self.cameras[index],
                        cell,
                        &scene,
                        &self.display,
                    );
                    if let Some(frame) = self.analysis_frames[index]
                        .as_ref()
                        .filter(|f| f.key == key)
                    {
                        Some(frame.texture.clone())
                    } else {
                        let (w, h) = (
                            cell.width().round().max(1.0) as usize,
                            cell.height().round().max(1.0) as usize,
                        );
                        let (w, h, rgba) = render::raster_frame(
                            &self.cameras[index].transform(),
                            &scene,
                            mode,
                            self.display.waterplane,
                            w,
                            h,
                        );
                        let image = egui::ColorImage::from_rgba_unmultiplied([w, h], &rgba);
                        let texture = ui.ctx().load_texture(
                            format!("analysis-{index}"),
                            image,
                            egui::TextureOptions::LINEAR,
                        );
                        fresh.push((
                            index,
                            AnalysisFrame {
                                key,
                                texture: texture.clone(),
                            },
                        ));
                        Some(texture)
                    }
                };
                render::viewport_with_texture(
                    ui,
                    cell,
                    &self.cameras[index],
                    Some(&scene),
                    &self.display,
                    self.active == index,
                    self.selected,
                    texture.as_ref(),
                );
            } else {
                render::viewport(
                    ui,
                    cell,
                    &self.cameras[index],
                    None,
                    &self.display,
                    self.active == index,
                    None,
                );
            }
        }
        for (index, frame) in fresh {
            self.analysis_frames[index] = Some(frame);
        }
    }

    fn pick_at(
        &self,
        ui: &egui::Ui,
        view: &crate::camera::ViewTransform,
        cell: egui::Rect,
    ) -> Option<usize> {
        let cursor = ui.input(|i| i.pointer.interact_pos())?;
        let surface = self.surface.as_ref()?;
        let pts: Vec<Vec3> = surface.points.iter().map(|p| p.p).collect();
        picking::pick_point(view, cell, cursor, &pts, PICK_RADIUS)
    }

    /// A primary drag starts either a point drag (when it grabs a point) or a
    /// camera pan (handled by the caller when `drag` stays `None`).
    fn begin_drag(
        &mut self,
        ui: &egui::Ui,
        view: &crate::camera::ViewTransform,
        cell: egui::Rect,
        index: usize,
    ) {
        let Some(point) = self.pick_at(ui, view, cell) else {
            return;
        };
        let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let center = surface.points[point].p;
        let plane = picking::drag_plane(view, center);
        let offset = picking::ray_plane(&view.ray(cursor, cell), &plane)
            .map(|hit| center - hit)
            .unwrap_or(Vec3::ZERO);
        self.history.snapshot(surface);
        self.selected = Some(point);
        self.active = index;
        self.status = format!("dragging control point {point}");
        self.drag = Some(DragState {
            point,
            plane,
            offset,
            view: index,
            original: center,
        });
    }

    fn drag_to(&mut self, ui: &egui::Ui, view: &crate::camera::ViewTransform, cell: egui::Rect) {
        let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) else {
            return;
        };
        let Some(drag) = self.drag.as_ref() else {
            return;
        };
        let Some(hit) = picking::ray_plane(&view.ray(cursor, cell), &drag.plane) else {
            return;
        };
        let mut target = hit + drag.offset;
        let point = drag.point;
        if let Some(surface) = self.surface.as_mut() {
            let on_symmetric_layer = surface.faces.iter().enumerate().any(|(fi, f)| {
                f.contains(&point)
                    && surface
                        .face_layer
                        .get(fi)
                        .and_then(|&li| surface.layers.get(li))
                        .is_some_and(|l| l.symmetric)
            });
            if on_symmetric_layer {
                target.y = target.y.max(0.0);
            }
            surface.points[point].p = target;
        }
        self.dirty = true;
        self.revision += 1;
        self.touch_geometry();
    }

    /// Rebuild the hull and the report once the drag has ended and 120 ms have
    /// passed since the last change, so a drag does not measure every frame.
    fn refresh_hydro(&mut self, force: bool) {
        if self.drag.is_some() {
            return;
        }
        if !force {
            if !self.hydro_dirty {
                return;
            }
            if let Some(then) = self.hydro.last_edit {
                if then.elapsed() < std::time::Duration::from_millis(120) {
                    return;
                }
            }
        }
        let Some(surface) = &self.surface else { return };
        let subdiv = seacraft_core::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let hull = Hull::new(surface, subdiv);
        let result = if self.hydro.free_trim {
            let (res, fc) =
                stability::hydrostatics_free_trim(&hull, &self.hydro.options, self.hydro.lcg);
            self.hydro.options.trim = fc.trim;
            if self.hydro.lcg.is_none() {
                self.hydro.lcg = Some(res.lcb.x);
            }
            res
        } else {
            hull.hydrostatics(&self.hydro.options)
        };
        self.hydro.result = Some(result);
        self.hydro.hull = Some(hull);
        self.hydro.hull_revision = self.revision;
        self.hydro.curves = None;
        self.hydro_dirty = false;
    }

    /// GZ cross curves for the current displacement and a spread around it.
    fn compute_curves(&mut self) {
        if self.hydro.result.is_none()
            || self.hydro.hull_revision != self.revision
            || self.hydro_dirty
        {
            self.refresh_hydro(true);
        }
        let Some(hull) = self.hydro.hull.take() else {
            return;
        };
        let displacement_t = self.hydro.result.as_ref().map_or(0.0, |r| r.displacement_t);
        if displacement_t > 0.0 {
            let heels: Vec<f64> = (0..=8).map(|i| i as f64 * 10.0).collect();
            let factors = [0.7, 0.85, 1.0, 1.15, 1.3];
            let displacements: Vec<f64> = factors.iter().map(|f| displacement_t * f).collect();
            // Free trim needs a longitudinal centre of gravity. A bare model
            // carries none, so the upright LCB is the natural stand-in.
            if self.hydro.free_trim && self.hydro.lcg.is_none() {
                self.hydro.lcg = Some(stability::upright_lcb(&hull, self.hydro.options.draft));
            }
            let cond = stability::StabilityCondition {
                kg: self.hydro.kg,
                trim: self.hydro.options.trim,
                lcg_x: if self.hydro.free_trim {
                    self.hydro.lcg
                } else {
                    None
                },
            };
            self.hydro.curves = Some(stability::cross_curves(
                &hull,
                &heels,
                &displacements,
                &cond,
                &self.hydro.options,
            ));
            self.status = if cond.lcg_x.is_some() {
                format!(
                    "GZ cross curves for {:.2} t and ±30 %, free to trim about {:.3} m",
                    displacement_t,
                    cond.lcg_x.unwrap_or_default()
                )
            } else {
                format!("GZ cross curves for {:.2} t and ±30 %", displacement_t)
            };
        }
        self.hydro.hull = Some(hull);
    }

    /// Note that the geometry moved: a measurement is due after the debounce.
    fn touch_geometry(&mut self) {
        self.hydro_dirty = true;
        self.hydro.last_edit = Some(std::time::Instant::now());
        self.loading_stale = true;
        self.damage_stale = true;
    }

    // ---- accessors (tests and dock panels) -----------------------------

    pub fn surface(&self) -> Option<&Surface> {
        self.surface.as_ref()
    }

    /// The design data saved beside the model.
    pub fn project(&self) -> &Project {
        &self.project
    }

    /// Replace the design data; the model becomes unsaved.
    pub fn set_project(&mut self, project: Project) {
        self.project = project;
        self.project_error = None;
        self.dirty = true;
        self.selected_volume = None;
        self.selected_windage = None;
        self.last_highlighted_tank = None;
        self.last_highlighted_compartment = None;
        self.loading_stale = true;
        self.loading_selected = 0;
        self.damage_stale = true;
        self.damage_selected = 0;
    }

    /// Show or hide the loading panel; showing it shows only it in the design dock.
    pub fn set_show_loading(&mut self, show: bool) {
        if show {
            self.design_dock = Some(DesignDock::Loading);
        } else if self.design_dock == Some(DesignDock::Loading) {
            self.design_dock = None;
        }
    }

    pub fn show_loading(&self) -> bool {
        self.design_dock == Some(DesignDock::Loading)
    }

    /// Show or hide the damage panel; showing it shows only it in the design dock.
    pub fn set_show_damage(&mut self, show: bool) {
        if show {
            self.design_dock = Some(DesignDock::Damage);
        } else if self.design_dock == Some(DesignDock::Damage) {
            self.design_dock = None;
        }
    }

    pub fn show_damage(&self) -> bool {
        self.design_dock == Some(DesignDock::Damage)
    }

    /// Show or hide the windage panel; showing it shows only it in the design dock.
    pub fn set_show_windage(&mut self, show: bool) {
        if show {
            self.design_dock = Some(DesignDock::Windage);
        } else if self.design_dock == Some(DesignDock::Windage) {
            self.design_dock = None;
        }
    }

    pub fn show_windage(&self) -> bool {
        self.design_dock == Some(DesignDock::Windage)
    }

    pub fn design_dock(&self) -> Option<DesignDock> {
        self.design_dock
    }

    pub fn selected_windage(&self) -> Option<usize> {
        self.selected_windage
    }

    pub fn set_selected_windage(&mut self, sel: Option<usize>) {
        self.selected_windage = sel;
    }

    pub fn selected_windage_info(&self) -> Option<(usize, String)> {
        let idx = self.selected_windage?;
        let name = self.project.windage.get(idx).map(|w| w.name.clone())?;
        Some((idx, name))
    }

    pub fn windage_overlays(&self) -> Vec<render::WindageOverlay> {
        self.project
            .windage
            .iter()
            .enumerate()
            .filter_map(|(idx, item)| {
                item.profile.map(|rect| render::WindageOverlay {
                    name: item.name.clone(),
                    rect,
                    selected: self.selected_windage == Some(idx),
                })
            })
            .collect()
    }

    pub fn selected_volume(&self) -> Option<VolumeRef> {
        self.selected_volume
    }

    pub fn selected_volume_info(&self) -> Option<(&'static str, usize, String)> {
        match self.selected_volume? {
            VolumeRef::Tank(idx) => {
                let name = if !self.project.loading_conditions.is_empty() {
                    let cond_idx = self
                        .loading_selected
                        .min(self.project.loading_conditions.len() - 1);
                    self.project.loading_conditions[cond_idx]
                        .tanks
                        .get(idx)
                        .map(|t| t.name.clone())
                        .unwrap_or_default()
                } else {
                    String::new()
                };
                Some(("tank", idx, name))
            }
            VolumeRef::Compartment(idx) => {
                let name = self
                    .project
                    .compartments
                    .get(idx)
                    .map(|c| c.name.clone())
                    .unwrap_or_default();
                Some(("compartment", idx, name))
            }
        }
    }

    pub fn volume_overlays(&self) -> Vec<(VolumeRef, render::VolumeOverlay)> {
        let mut out = Vec::new();
        if !self.project.loading_conditions.is_empty() {
            let cond_idx = self
                .loading_selected
                .min(self.project.loading_conditions.len() - 1);
            let cond = &self.project.loading_conditions[cond_idx];
            for (t_idx, tank) in cond.tanks.iter().enumerate() {
                let liquid_z = if !self.loading_stale {
                    self.loading_result
                        .as_ref()
                        .and_then(|res| res.tanks.iter().find(|t| t.name == tank.name))
                        .map(|t| t.surface_z)
                } else {
                    None
                }
                .unwrap_or(tank.z_min + tank.fill * (tank.z_max - tank.z_min));

                let selected = self.selected_volume == Some(VolumeRef::Tank(t_idx));
                out.push((
                    VolumeRef::Tank(t_idx),
                    render::VolumeOverlay {
                        name: tank.name.clone(),
                        min: Vec3::new(tank.x_min, tank.y_min, tank.z_min),
                        max: Vec3::new(tank.x_max, tank.y_max, tank.z_max),
                        kind: render::VolumeKind::Tank {
                            liquid_z: Some(liquid_z),
                        },
                        selected,
                    },
                ));
            }
        }

        let selected_case = if !self.project.damage_cases.is_empty() {
            let case_idx = self
                .damage_selected
                .min(self.project.damage_cases.len() - 1);
            Some(&self.project.damage_cases[case_idx])
        } else {
            None
        };

        for (c_idx, comp) in self.project.compartments.iter().enumerate() {
            let flooded = selected_case.is_some_and(|case| {
                case.compartments.iter().any(|spec| match spec {
                    seacraft_core::damage::CompartmentSpec::Named(name) => name == &comp.name,
                    seacraft_core::damage::CompartmentSpec::Inline(c) => c.name == comp.name,
                })
            });
            let selected = self.selected_volume == Some(VolumeRef::Compartment(c_idx));
            out.push((
                VolumeRef::Compartment(c_idx),
                render::VolumeOverlay {
                    name: comp.name.clone(),
                    min: Vec3::new(comp.x_min, comp.y_min, comp.z_min),
                    max: Vec3::new(comp.x_max, comp.y_max, comp.z_max),
                    kind: render::VolumeKind::Compartment { flooded },
                    selected,
                },
            ));
        }

        out
    }

    pub fn loading_result(&self) -> Option<&LoadingResult> {
        self.loading_result.as_ref()
    }

    pub fn start_loading_compute(&mut self) {
        let Some(surface) = self.surface.clone() else {
            self.status = "No model loaded.".into();
            return;
        };
        if self.project.loading_conditions.is_empty() {
            self.status = "No loading condition in project.".into();
            return;
        }
        let sel = self
            .loading_selected
            .min(self.project.loading_conditions.len() - 1);
        let cond = &self.project.loading_conditions[sel];
        let with_windage = self.project.with_windage(cond);
        if let Err(e) = seacraft_core::loading::validate(&with_windage) {
            self.status = format!("Loading condition invalid: {e}");
            return;
        }
        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        self.loading_generation += 1;
        let gen = self.loading_generation;
        let (tx, rx) = std::sync::mpsc::channel();
        self.loading_worker = Some(rx);

        std::thread::spawn(move || {
            let res = seacraft_core::loading::evaluate(&surface, &opts, &with_windage);
            let _ = tx.send((gen, res));
        });
    }

    pub fn compute_loading_blocking(
        &mut self,
        name: Option<&str>,
    ) -> Result<&LoadingResult, String> {
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| "no model loaded".to_string())?;
        let cond = self.project.loading_condition(name)?;
        let with_windage = self.project.with_windage(cond);
        seacraft_core::loading::validate(&with_windage)?;
        // Show the condition that was computed, and drop any worker result.
        if let Some(i) = self
            .project
            .loading_conditions
            .iter()
            .position(|c| c.name == with_windage.name)
        {
            self.loading_selected = i;
        }
        self.loading_generation += 1;
        self.loading_worker = None;

        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        let res = seacraft_core::loading::evaluate(surface, &opts, &with_windage);
        self.loading_result = Some(res);
        self.loading_stale = false;
        Ok(self.loading_result.as_ref().unwrap())
    }

    pub fn damage_result(&self) -> Option<&DamageResult> {
        self.damage_result.as_ref()
    }

    pub fn start_damage_compute(&mut self) {
        let Some(surface) = self.surface.clone() else {
            self.status = "No model loaded.".into();
            return;
        };
        if self.project.damage_cases.is_empty() {
            self.status = "No damage case in project.".into();
            return;
        }
        let sel = self
            .damage_selected
            .min(self.project.damage_cases.len() - 1);
        let raw_case = self.project.damage_cases[sel].clone();
        let project = self.project.clone();

        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        self.damage_generation += 1;
        let gen = self.damage_generation;
        let (tx, rx) = std::sync::mpsc::channel();
        self.damage_worker = Some(rx);

        std::thread::spawn(move || {
            let res: Result<DamageResult, String> = (|| {
                let resolved = project.resolve_damage_case(&raw_case)?;
                seacraft_core::damage::validate(&resolved)?;
                seacraft_core::damage::evaluate(&surface, &opts, &resolved)
            })();
            let _ = tx.send((gen, res));
        });
    }

    pub fn compute_damage_blocking(&mut self, name: Option<&str>) -> Result<&DamageResult, String> {
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| "no model loaded".to_string())?;
        let raw_case = self.project.damage_case(name)?;
        let case = self.project.resolve_damage_case(raw_case)?;
        seacraft_core::damage::validate(&case)?;
        if let Some(i) = self
            .project
            .damage_cases
            .iter()
            .position(|c| c.name == case.name)
        {
            self.damage_selected = i;
        }
        self.damage_generation += 1;
        self.damage_worker = None;

        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        let res = seacraft_core::damage::evaluate(surface, &opts, &case)?;
        self.damage_result = Some(res);
        self.damage_stale = false;
        Ok(self.damage_result.as_ref().unwrap())
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn status(&self) -> &str {
        &self.status
    }

    pub fn control_point_count(&self) -> usize {
        self.surface.as_ref().map_or(0, |s| s.points.len())
    }

    pub fn face_count(&self) -> usize {
        self.surface.as_ref().map_or(0, |s| s.faces.len())
    }

    pub fn render_cache(&self) -> Option<&RenderMesh> {
        self.cache.as_ref().map(|c| &c.mesh)
    }

    pub fn cameras(&self) -> &[Camera; 4] {
        &self.cameras
    }

    pub fn active_view(&self) -> usize {
        self.active
    }

    /// Where viewport `i` was drawn last frame (test hook).
    pub fn viewport_rect(&self, i: usize) -> egui::Rect {
        self.viewport_rects[i]
    }

    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    pub fn is_dirty(&self) -> bool {
        self.dirty
    }

    pub fn dialog_open(&self) -> bool {
        self.dialog.is_some()
    }

    /// Mutable access to the model, for panels and tests. Call
    /// [`App::mark_geometry_changed`] afterwards.
    pub fn surface_mut(&mut self) -> Option<&mut Surface> {
        self.surface.as_mut()
    }

    pub fn mark_geometry_changed(&mut self) {
        self.revision += 1;
        self.dirty = true;
        self.touch_geometry();
    }

    pub fn hydrostatics(&self) -> Option<&Hydrostatics> {
        self.hydro.result.as_ref()
    }

    /// Recompute now, without waiting for the debounce (test hook).
    pub fn recompute_hydrostatics(&mut self) {
        self.refresh_hydro(true);
    }

    pub fn hydro_options_mut(&mut self) -> &mut HydroOptions {
        &mut self.hydro.options
    }

    pub fn set_kg(&mut self, kg: f64) {
        self.hydro.kg = kg;
        self.hydro_dirty = true;
    }

    pub fn kg(&self) -> f64 {
        self.hydro.kg
    }

    /// Refinement level used for display and measurement. Tests drop it to keep
    /// repeated solves cheap; the UI drives the same field from the menu.
    pub fn set_subdivision_level(&mut self, level: u8) {
        self.level = level.clamp(0, 5);
        self.revision += 1;
        self.touch_geometry();
    }

    /// Solver setting for hydrostatics and GZ curves: free to trim about `lcg`
    /// (or hold the trim when `free` is false). `None` leaves the LCG to be
    /// filled from the upright LCB on the next solve.
    pub fn set_free_trim(&mut self, free: bool, lcg: Option<f64>) {
        self.hydro.free_trim = free;
        self.hydro.lcg = lcg;
        self.hydro_dirty = true;
    }

    pub fn free_trim(&self) -> bool {
        self.hydro.free_trim
    }

    /// The GZ curves as last computed, if any.
    pub fn cross_curves(&self) -> Option<&[CrossCurve]> {
        self.hydro.curves.as_deref()
    }

    /// Rebuild the GZ curves now instead of waiting for the dock button.
    pub fn compute_cross_curves(&mut self) {
        self.compute_curves();
    }

    pub fn undo_len(&self) -> usize {
        self.history.undo_len()
    }

    pub fn display(&self) -> &DisplayOptions {
        &self.display
    }

    pub fn display_mut(&mut self) -> &mut DisplayOptions {
        &mut self.display
    }

    /// Pick the surface-analysis shading (also settable from the View menu).
    pub fn set_analysis(&mut self, mode: seacraft_core::AnalysisMode) {
        self.display.analysis = mode;
        self.analysis_frames = [None, None, None, None];
    }

    pub fn subdivision_level(&self) -> u8 {
        self.level
    }

    pub fn subdivision_mode(&self) -> SubdivisionMode {
        self.mode
    }

    pub fn set_active_view(&mut self, view: usize) {
        self.active = view.min(3);
    }

    pub fn hydro_options(&self) -> &HydroOptions {
        &self.hydro.options
    }

    /// Longitudinal centre of gravity used by the free-to-trim solve.
    pub fn lcg(&self) -> Option<f64> {
        self.hydro.lcg
    }

    pub fn layer_count(&self) -> usize {
        self.surface.as_ref().map_or(0, |s| s.layers.len())
    }

    pub fn cameras_mut(&mut self) -> &mut [Camera; 4] {
        &mut self.cameras
    }

    /// Select a control point (`None` clears the selection).
    pub fn select_point(&mut self, index: Option<usize>) {
        self.selected = index;
    }

    /// Record the current model on the undo stack, as a drag does on grab.
    pub fn snapshot_undo(&mut self) {
        if let Some(surface) = self.surface.as_ref() {
            self.history.snapshot(surface);
        }
    }

    /// Refine the hull now if anything it depends on changed (the control
    /// socket's `ui.render` needs a mesh without waiting for a frame).
    pub fn refresh_cache(&mut self) {
        self.sync_cache();
    }

    /// A status line for an action that changed nothing about the model.
    pub fn status_note(&mut self, text: impl Into<String>) {
        self.status = text.into();
    }

    /// Ask the window to close (the control socket's `app.quit`).
    pub fn request_quit(&mut self, ctx: &egui::Context) {
        self.close_requested = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    // ---- control socket ------------------------------------------------

    /// Serve `rx` from the next frame on (`--control`).
    pub fn attach_control(
        &mut self,
        rx: std::sync::mpsc::Receiver<crate::control::ControlRequest>,
    ) {
        self.control = Some(rx);
    }

    /// Whether a control socket is attached.
    pub fn has_control(&self) -> bool {
        self.control.is_some()
    }

    /// Answer up to eight pending control requests, then ask for another frame
    /// so the next ones are served even while the window is idle.
    ///
    /// Called once per frame before the UI is built: a request therefore reads
    /// and edits the state the frame is about to paint. Synthetic input it
    /// queues reaches the following frame through [`App::raw_input_hook`].
    pub fn process_control(&mut self, ctx: &egui::Context) {
        if self.control.is_none() {
            return;
        }
        for _ in 0..8 {
            let received = match self.control.as_ref() {
                Some(rx) => rx.try_recv(),
                None => break,
            };
            let request = match received {
                Ok(request) => request,
                Err(std::sync::mpsc::TryRecvError::Empty) => break,
                Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                    self.control = None;
                    break;
                }
            };
            let reply = crate::control::handle(self, ctx, &request);
            let _ = request.reply.send(reply);
        }
        // The listener thread holds no `egui::Context`, so it cannot wake the
        // window: poll instead. A request waiting at most 100 ms is the price.
        ctx.request_repaint_after(std::time::Duration::from_millis(100));
    }

    /// Move the queued synthetic input into this frame's raw input.
    pub fn raw_input_hook(&mut self, raw: &mut egui::RawInput) {
        if !self.synthetic.is_empty() {
            raw.events.append(&mut self.synthetic);
        }
    }

    /// Queue synthetic input for the next frame. Returns how many events were
    /// queued; a full queue is an error rather than unbounded memory.
    pub fn queue_events(
        &mut self,
        events: impl IntoIterator<Item = egui::Event>,
    ) -> Result<usize, String> {
        let events: Vec<egui::Event> = events.into_iter().collect();
        if self.synthetic.len() + events.len() > crate::control::MAX_SYNTHETIC {
            return Err(format!(
                "the synthetic input queue is full ({} events, at most {})",
                self.synthetic.len(),
                crate::control::MAX_SYNTHETIC
            ));
        }
        if events.is_empty() {
            return Ok(0);
        }
        let count = events.len();
        self.synthetic.extend(events);
        Ok(count)
    }

    /// How many synthetic events are waiting for the next frame.
    pub fn synthetic_len(&self) -> usize {
        self.synthetic.len()
    }

    /// Set the allowed file roots for the control channel.
    pub fn set_control_roots(&mut self, roots: seacraft_core::control_auth::ControlRoots) {
        self.control_roots = roots;
    }

    /// Allowed file roots for the control channel.
    pub fn control_roots(&self) -> &seacraft_core::control_auth::ControlRoots {
        &self.control_roots
    }

    /// Check that `path` resolves inside an allowed read root.
    pub fn check_control_read_path(
        &self,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf, String> {
        seacraft_core::control_auth::check_read_path(path, &self.control_roots.read_roots)
    }

    /// Check that `path` resolves inside an allowed write root.
    pub fn check_control_write_path(
        &self,
        path: &std::path::Path,
    ) -> Result<std::path::PathBuf, String> {
        seacraft_core::control_auth::check_write_path(path, &self.control_roots.write_roots)
    }

    /// Attach a token file to be deleted when the app is dropped (clean shutdown).
    pub fn set_token_file(&mut self, file: Option<seacraft_core::control_auth::TokenFile>) {
        self.token_file = file;
    }
}

/// Directories and files ending in `wanted`, for the in-app file dialog.
fn listing(path: &str, wanted: &str) -> Vec<(String, String)> {
    let path = std::path::Path::new(path.trim());
    let dir = if path.is_dir() {
        path.to_path_buf()
    } else {
        path.parent()
            .map(|d| d.to_path_buf())
            .unwrap_or_else(|| std::path::PathBuf::from("."))
    };
    let wanted = wanted.to_ascii_lowercase();
    let mut out = Vec::new();
    if let Ok(read) = std::fs::read_dir(&dir) {
        let mut entries: Vec<_> = read.flatten().collect();
        entries.sort_by_key(|e| e.file_name());
        for entry in entries.into_iter().take(300) {
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.') {
                continue;
            }
            if entry.path().is_dir() {
                out.push((
                    format!("{name}/"),
                    entry.path().to_string_lossy().to_string(),
                ));
            } else if name.to_ascii_lowercase().ends_with(&wanted) {
                out.push((name, entry.path().to_string_lossy().to_string()));
            }
        }
    }
    out
}

impl eframe::App for App {
    /// Serve the control socket's queue before the UI runs.
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        self.process_control(ctx);
    }

    /// Inject the synthetic input the control socket queued.
    fn raw_input_hook(&mut self, _ctx: &egui::Context, raw: &mut egui::RawInput) {
        self.raw_input_hook(raw);
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
