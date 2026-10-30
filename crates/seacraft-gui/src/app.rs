//! SeaCraft desktop application.
//!
//! `App` owns the model and the UI state. `App::draw` takes only an
//! [`egui::Ui`], so the whole application can be driven in tests without an
//! `eframe::Frame`; the [`eframe::App`] implementation is a one-line
//! delegation.

use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui;
use seacraft_core::damage::DamageResult;
use seacraft_core::export::{self, RenderMesh};
use seacraft_core::hydro::{Hull, HydroOptions, Hydrostatics};
use seacraft_core::loading::LoadingResult;
use seacraft_core::math::{Plane, Vec3};
use seacraft_core::project::{self, Project};
use seacraft_core::resistance::{self, HoltropInput};
use seacraft_core::seakeeping::{SeakeepingReport, SeakeepingSpec};
use seacraft_core::seastate::{IrregularReport, IrregularSpec};
use seacraft_core::stability::{self, CrossCurve};
use seacraft_core::{demo, fef, SubdivisionMode, Surface};

use crate::camera::{Camera, ViewKind};
use crate::history::History;
use crate::jobs::{revision_key, Job, Track};
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
    /// The curves were carried over from before a drag began: they are
    /// rebuilt on release, not on every drag frame.
    curves_stale: bool,
    min: Vec3,
    max: Vec3,
}

impl RenderCache {
    /// Identifies the refined mesh: the revision it was built from and the
    /// level (the drag preview refines less without a new revision).
    fn stamp(&self) -> u64 {
        (self.revision << 4) | u64::from(self.level)
    }
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
    /// The hull `result` was measured on (the GZ curves reuse it while it
    /// is current).
    hull: Option<Arc<Hull>>,
    /// Revision stamp and job of `result` (and `hull`).
    track: Track<HydroOut>,
    /// Revision stamp and job of `curves`.
    curves_track: Track<CurvesOut>,
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
            track: Track::default(),
            curves_track: Track::default(),
        }
    }

    /// Everything a measurement needs, cloned off the app for a job.
    fn inputs(&self, surface: &Surface, level: u8, mode: SubdivisionMode) -> HydroInputs {
        HydroInputs {
            surface: surface.clone(),
            subdiv: seacraft_core::SubdivisionOptions { level, mode },
            options: self.options.clone(),
            free_trim: self.free_trim,
            lcg: self.lcg,
        }
    }
}

/// The inputs of a hydrostatics measurement.
struct HydroInputs {
    surface: Surface,
    subdiv: seacraft_core::SubdivisionOptions,
    options: HydroOptions,
    free_trim: bool,
    lcg: Option<f64>,
}

/// A hydrostatics measurement: the report, the hull it was measured on and,
/// free to trim, the solved trim (written back to the floating condition).
struct HydroOut {
    result: Hydrostatics,
    hull: Arc<Hull>,
    trim: Option<f64>,
}

/// Refine and measure (the hydrostatics job, and its synchronous twin).
fn measure_hydro(i: &HydroInputs) -> HydroOut {
    let hull = Hull::new(&i.surface, i.subdiv);
    let (result, trim) = if i.free_trim {
        let (res, fc) = stability::hydrostatics_free_trim(&hull, &i.options, i.lcg);
        (res, Some(fc.trim))
    } else {
        (hull.hydrostatics(&i.options), None)
    };
    HydroOut {
        result,
        hull: Arc::new(hull),
        trim,
    }
}

/// The inputs of the GZ cross curves: the measurement's inputs, KG and,
/// when the measurement is current, its hull and displacement.
struct CurvesInputs {
    hydro: HydroInputs,
    kg: f64,
    measured: Option<(Arc<Hull>, f64)>,
}

/// The GZ cross curves (`None` when nothing floats), the LCG they were
/// solved about and, when the job had to measure first, that measurement.
struct CurvesOut {
    curves: Option<Vec<CrossCurve>>,
    displacement_t: f64,
    lcg: Option<f64>,
    hydro: Option<HydroOut>,
}

/// GZ cross curves for the displacement and a ±30 % spread around it.
fn compute_cross_curves_for(i: CurvesInputs) -> CurvesOut {
    let (hull, displacement_t, trim, lcg, hydro) = match i.measured {
        Some((hull, d)) => (hull, d, i.hydro.options.trim, i.hydro.lcg, None),
        None => {
            let out = measure_hydro(&i.hydro);
            let trim = out.trim.unwrap_or(i.hydro.options.trim);
            // As the dock's measurement: free to trim without an LCG, the
            // measured LCB is the LCG.
            let lcg = match (i.hydro.free_trim, i.hydro.lcg) {
                (true, None) => Some(out.result.lcb.x),
                (_, lcg) => lcg,
            };
            (
                out.hull.clone(),
                out.result.displacement_t,
                trim,
                lcg,
                Some(out),
            )
        }
    };
    if displacement_t <= 0.0 {
        return CurvesOut {
            curves: None,
            displacement_t,
            lcg,
            hydro,
        };
    }
    let heels: Vec<f64> = (0..=8).map(|i| i as f64 * 10.0).collect();
    let factors = [0.7, 0.85, 1.0, 1.15, 1.3];
    let displacements: Vec<f64> = factors.iter().map(|f| displacement_t * f).collect();
    // Free trim needs a longitudinal centre of gravity. A bare model
    // carries none, so the upright LCB is the natural stand-in.
    let lcg = match (i.hydro.free_trim, lcg) {
        (true, None) => Some(stability::upright_lcb(&hull, i.hydro.options.draft)),
        (_, lcg) => lcg,
    };
    let cond = stability::StabilityCondition {
        kg: i.kg,
        trim,
        lcg_x: if i.hydro.free_trim { lcg } else { None },
    };
    let curves = stability::cross_curves(&hull, &heels, &displacements, &cond, &i.hydro.options);
    CurvesOut {
        curves: Some(curves),
        displacement_t,
        lcg,
        hydro,
    }
}

/// One viewport's rasterized frame, kept until the geometry or camera moves.
struct AnalysisFrame {
    /// [`render::frame_key`]: the view and the mesh it was rastered from.
    key: u64,
    /// [`render::view_key`]: the view alone. While a point is dragged in
    /// another view, a frame whose view is unchanged is kept until release.
    view_key: u64,
    texture: egui::TextureHandle,
}

/// A rasterized analysis image waiting to be uploaded as a texture.
struct RasterImage {
    /// The [`render::frame_key`] it was rastered for.
    key: u64,
    size: [usize; 2],
    rgba: Vec<u8>,
}

impl RasterImage {
    /// Rasterize `camera`'s analysis image at `cell`'s size.
    fn render(
        camera: &Camera,
        cell: egui::Rect,
        scene: &Scene,
        display: &DisplayOptions,
        key: u64,
    ) -> Self {
        let (w, h, rgba) = render::analysis_image(
            &camera.transform(),
            scene,
            display.analysis,
            display.waterplane,
            cell.width().round().max(1.0) as usize,
            cell.height().round().max(1.0) as usize,
        );
        Self {
            key,
            size: [w, h],
            rgba,
        }
    }
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

/// One undo step: the model and the draft it was measured at, since a
/// transform or an optimisation moves both.
#[derive(Debug, Clone)]
struct Snapshot {
    /// `None` before the first model: undoing a New design made from the
    /// start screen goes back to it.
    surface: Option<Surface>,
    draft: f64,
    /// A step that replaced the whole model (New design, demo from the
    /// dialog): the project, path and saved state go back with it.
    model: Option<Box<ModelState>>,
}

impl Snapshot {
    /// An edit of the model in place.
    fn edit(surface: &Surface, draft: f64) -> Self {
        Self {
            surface: Some(surface.clone()),
            draft,
            model: None,
        }
    }
}

/// What travels with a model besides its geometry.
#[derive(Debug, Clone)]
struct ModelState {
    project: Project,
    project_error: Option<String>,
    path: Option<PathBuf>,
    saved_state: Option<(String, Project)>,
}

/// Which file dialog is open.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DialogKind {
    Open,
    SaveAs,
    Export(ExportKind),
    /// The HTML design report, written with the project's report settings.
    Report,
}

/// An action that may need to discard unsaved work first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    NewDemo,
    /// The New design dialog (it replaces the model and the project).
    NewDesign,
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
    /// What to do once a "Save as" opened from the unsaved-changes prompt
    /// has written the model (new, open or quit).
    then: Option<PendingAction>,
}

/// Reference to a selected project tank or compartment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VolumeRef {
    Tank(usize),
    Compartment(usize),
}

/// The tab shown in the right-hand Analysis dock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DesignDock {
    /// The hydrostatics report (the default tab).
    #[default]
    Hydrostatics,
    Loading,
    Damage,
    Windage,
    Performance,
}

impl DesignDock {
    /// Every tab, in tab-bar order.
    pub const ALL: [DesignDock; 5] = [
        DesignDock::Hydrostatics,
        DesignDock::Loading,
        DesignDock::Damage,
        DesignDock::Windage,
        DesignDock::Performance,
    ];

    /// The tab-bar label.
    pub fn label(self) -> &'static str {
        match self {
            DesignDock::Hydrostatics => "Hydro",
            DesignDock::Loading => "Loading",
            DesignDock::Damage => "Damage",
            DesignDock::Windage => "Windage",
            DesignDock::Performance => "Performance",
        }
    }

    /// The control channel's name for the tab (`ui.set {"dock_tab": …}`).
    pub fn key(self) -> &'static str {
        match self {
            DesignDock::Hydrostatics => "hydro",
            DesignDock::Loading => "loading",
            DesignDock::Damage => "damage",
            DesignDock::Windage => "windage",
            DesignDock::Performance => "performance",
        }
    }

    /// The tab a control-channel name stands for.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|t| t.key() == key)
    }
}

/// What the Analysis dock's active tab asked for, applied after the panel.
enum DockAction {
    None,
    Hydro(panels::HydroAction),
    Loading(panels::LoadingAction),
    Damage(panels::DamageAction),
    Windage(panels::WindageAction),
    Performance(panels::PerformanceAction),
}

/// Method used for resistance prediction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ResistanceMethod {
    #[default]
    Holtrop,
    Dsyhs,
    Savitsky,
    Michell,
}

impl ResistanceMethod {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Holtrop => "Holtrop & Mennen",
            Self::Dsyhs => "Delft (DSYHS)",
            Self::Savitsky => "Savitsky (Planing)",
            Self::Michell => "Michell (Thin ship)",
        }
    }
}

/// A row in the GUI resistance prediction table.
#[derive(Debug, Clone, PartialEq)]
pub struct ResistanceRow {
    pub speed_kn: f64,
    pub fn_: f64,
    pub rt_kn: f64,
    pub pe_kw: Option<f64>,
    pub note: Option<String>,
}

/// Consolidated resistance prediction result for GUI display.
#[derive(Debug, Clone, PartialEq)]
pub struct ResistanceResult {
    pub method: String,
    pub rows: Vec<ResistanceRow>,
    pub warnings: Vec<String>,
    pub rt_label: String,
}

impl ResistanceResult {
    pub fn from_core(
        method: &str,
        rows: &[seacraft_core::resistance::Resistance],
        warnings: Vec<String>,
        rt_label: &str,
    ) -> Self {
        Self {
            method: method.to_string(),
            rows: rows
                .iter()
                .map(|r| ResistanceRow {
                    speed_kn: r.speed_kn,
                    fn_: r.fn_,
                    rt_kn: r.rt_n / 1000.0,
                    pe_kw: Some(r.pe_w / 1000.0),
                    note: None,
                })
                .collect(),
            warnings,
            rt_label: rt_label.to_string(),
        }
    }
}

/// Which wave spectrum the sea-state section uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeaSpectrumKind {
    Bretschneider,
    Jonswap,
}

impl SeaSpectrumKind {
    pub fn label(self) -> &'static str {
        match self {
            SeaSpectrumKind::Bretschneider => "Bretschneider",
            SeaSpectrumKind::Jonswap => "JONSWAP",
        }
    }
}

/// Where a sea-state response point sits along the hull.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SeaPointPos {
    /// The longitudinal centre of gravity.
    Lcg,
    /// The forward end of the design waterline.
    Bow,
    /// A model x coordinate in metres.
    Model(f64),
}

/// A named response point of the sea-state section.
#[derive(Debug, Clone, PartialEq)]
pub struct SeaPoint {
    pub name: String,
    pub pos: SeaPointPos,
}

/// Input of the Performance tab's "Sea state" section (irregular head seas).
#[derive(Debug, Clone, PartialEq)]
pub struct SeaStateInput {
    /// Compute the sea-state responses with the RAOs (RAOs then use the spectrum's ω grid).
    pub enabled: bool,
    pub kind: SeaSpectrumKind,
    pub hs: f64,
    pub tp: f64,
    pub gamma: f64,
    pub points: Vec<SeaPoint>,
}

impl Default for SeaStateInput {
    fn default() -> Self {
        Self {
            enabled: false,
            kind: SeaSpectrumKind::Bretschneider,
            hs: 1.0,
            tp: 6.0,
            gamma: 3.3,
            points: Self::default_points(),
        }
    }
}

impl SeaStateInput {
    /// The default response points: LCG and Bow.
    pub fn default_points() -> Vec<SeaPoint> {
        vec![
            SeaPoint {
                name: "LCG".into(),
                pos: SeaPointPos::Lcg,
            },
            SeaPoint {
                name: "Bow".into(),
                pos: SeaPointPos::Bow,
            },
        ]
    }

    pub fn spectrum(&self) -> seacraft_core::seastate::WaveSpectrum {
        match self.kind {
            SeaSpectrumKind::Bretschneider => {
                seacraft_core::seastate::WaveSpectrum::Bretschneider {
                    hs: self.hs,
                    tp: self.tp,
                }
            }
            SeaSpectrumKind::Jonswap => seacraft_core::seastate::WaveSpectrum::Jonswap {
                hs: self.hs,
                tp: self.tp,
                gamma: self.gamma,
            },
        }
    }

    /// Why the input cannot be computed, if so.
    pub fn problem(&self) -> Option<String> {
        self.spectrum().validate().err()
    }

    /// The points with LCG and Bow resolved to model x.
    pub fn resolve_points(
        &self,
        x_lcg: f64,
        x_bow: f64,
    ) -> Vec<seacraft_core::seastate::MotionPoint> {
        self.points
            .iter()
            .map(|p| seacraft_core::seastate::MotionPoint {
                name: p.name.clone(),
                x: match p.pos {
                    SeaPointPos::Lcg => x_lcg,
                    SeaPointPos::Bow => x_bow,
                    SeaPointPos::Model(x) => x,
                },
            })
            .collect()
    }
}

/// RAOs for the Performance tab. With a sea state, the RAOs are computed once on the
/// spectrum's ω grid (0.2 ωp … 5 ωp) and integrated against the spectrum.
pub fn compute_raos_with_sea(
    surface: &Surface,
    spec: &SeakeepingSpec,
    sea: Option<&SeaStateInput>,
) -> Result<(SeakeepingReport, Option<IrregularReport>), String> {
    let Some(input) = sea else {
        let rep = seacraft_core::seakeeping::head_sea_raos(surface, spec)
            .map_err(|e| format!("seakeeping compute: {e}"))?;
        return Ok((rep, None));
    };
    let mut ispec = IrregularSpec::new(input.spectrum(), spec.clone());
    ispec.validate().map_err(|e| format!("sea state: {e}"))?;
    let mut sk = spec.clone();
    sk.omegas = ispec.omega_grid();
    let rep = seacraft_core::seakeeping::head_sea_raos(surface, &sk)
        .map_err(|e| format!("seakeeping compute: {e}"))?;
    ispec.points = input.resolve_points(rep.x_lcg, rep.x_bow);
    let irr = seacraft_core::seastate::irregular_from_raos(&rep, &ispec)
        .map_err(|e| format!("sea state: {e}"))?;
    Ok((rep, Some(irr)))
}

/// State for the performance tab (resistance prediction and seakeeping RAOs).
#[derive(Debug, Clone)]
pub struct PerformanceState {
    pub method: ResistanceMethod,
    pub speeds_kn: (f64, f64, f64),
    pub sk_speed_kn: f64,
    pub kg: Option<f64>,
    pub kyy_frac: f64,
    pub points: usize,
    pub resistance: Option<ResistanceResult>,
    pub raos: Option<SeakeepingReport>,
    /// Input of the sea-state section.
    pub sea_input: SeaStateInput,
    /// Irregular-sea responses from the same run as `raos` (None unless a sea state was set).
    pub sea_result: Option<IrregularReport>,
}

impl Default for PerformanceState {
    fn default() -> Self {
        Self {
            method: ResistanceMethod::Holtrop,
            speeds_kn: (2.0, 12.0, 1.0),
            sk_speed_kn: 6.0,
            kg: None,
            kyy_frac: 0.25,
            points: 30,
            resistance: None,
            raos: None,
            sea_input: SeaStateInput::default(),
            sea_result: None,
        }
    }
}

/// What the RAO job hands back: the RAOs and, with a sea state, the
/// irregular-sea responses computed from them.
type RaoResult = (SeakeepingReport, Option<IrregularReport>);

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
    history: History<Snapshot>,
    marks: Vec<(u64, Option<Surface>, Project, usize)>,
    next_mark_id: u64,
    drag: Option<DragState>,
    /// Unsaved edits since the last save / load.
    dirty: bool,
    /// The model (as `.fef` text) and project as last loaded or saved, so an
    /// undo back to that state clears `dirty`. `None` for a model never saved.
    saved_state: Option<(String, Project)>,
    dialog: Option<FileDialogState>,
    pending: Option<PendingAction>,
    /// A quit that has been confirmed (the window closes at the end of the frame).
    close_requested: bool,
    /// A widget held the keyboard when the last frame ended. egui drops the
    /// focus on Escape before the next frame's widgets run, so this is what
    /// tells an Escape that leaves a text field from one that clears the
    /// selection.
    keyboard_taken: bool,
    show_layers: bool,
    /// The right-hand Analysis dock (pinned strip + tabs) is shown.
    show_analysis: bool,
    /// The Analysis dock's tab.
    dock_tab: DesignDock,
    /// The Analysis dock is widened for plots and tables.
    dock_wide: bool,
    /// An inspector value being dragged (point index and x/y/z): the point
    /// moves when the drag is released, as one undo step.
    inspector_pending: Option<(usize, [f64; 3])>,
    /// Where the inspector's x/y/z fields were drawn last frame (test hook).
    inspector_rects: [egui::Rect; 3],
    /// Where the layer list and the Inspector under it were drawn last
    /// frame (test hooks).
    layer_list_rect: egui::Rect,
    inspector_rect: egui::Rect,
    /// The File > Report options window is open.
    show_report_options: bool,
    /// The New design dialog, while it is open.
    new_design: Option<crate::new_design::NewDesignState>,
    /// The File > Project settings dialog, while it is open.
    project_settings: Option<crate::new_design::ProjectSettingsState>,
    /// The start card's buttons from the last frame; `None` when a model is
    /// shown instead (test hook).
    start_rects: Option<crate::new_design::StartRects>,
    /// A design report being written on a worker thread, and its path.
    report_job: Option<(PathBuf, Job<()>)>,
    loading_selected: usize,
    loading_result: Option<LoadingResult>,
    loading_track: Track<LoadingResult>,
    damage_selected: usize,
    damage_result: Option<DamageResult>,
    damage_track: Track<DamageResult>,
    perf_state: PerformanceState,
    resistance_track: Track<ResistanceResult>,
    raos_track: Track<RaoResult>,
    /// Bumped by every change of the floating condition (draft, trim, heel,
    /// KG, free trim, LCG). With `revision` and the counters below it keys
    /// every result: a result is current while its key is unchanged.
    hydro_rev: u64,
    /// Bumped by every edit of the project (loading, damage, windage).
    project_rev: u64,
    /// Bumped by every edit of the Performance inputs.
    perf_rev: u64,
    /// When an input last changed, for the automatic recompute's debounce.
    last_change: Option<std::time::Instant>,
    /// Cursor position on the active viewport's focal plane, for the status bar.
    cursor_world: Option<Vec3>,
    hydro: HydroState,
    /// Viewport rectangles from the last frame, so tests can aim the pointer.
    /// A view hidden by a maximised one has `Rect::NOTHING`.
    viewport_rects: [egui::Rect; 4],
    /// The viewports' title captions from the last frame (double-click
    /// maximises), `Rect::NOTHING` for a hidden view.
    title_rects: [egui::Rect; 4],
    /// The whole central area the viewports share, from the last frame.
    central_rect: egui::Rect,
    /// The view that fills the whole central area, if one is maximised.
    maximised: Option<usize>,
    /// Each analysis mode's parameter, kept while another mode is shown.
    analysis_params: panels::AnalysisParams,
    /// Where the toolbar's controls were drawn last frame (test hook).
    toolbar_rects: Vec<(&'static str, egui::Rect)>,
    /// Rasterized frames for the analysis modes, one per viewport.
    analysis_frames: [Option<AnalysisFrame>; 4],
    /// How many analysis frames each viewport has rasterized (test hook).
    raster_counts: [u64; 4],
    /// Each viewport's painter-shaded hull and the [`render::hull_key`] it
    /// was built for.
    shaded_hulls: [Option<(u64, Arc<egui::Mesh>)>; 4],
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

/// Whether control point `point` belongs to a face on a symmetric layer
/// (only `y >= 0` modelled), so it must not cross the centreline.
pub fn point_on_symmetric_layer(surface: &Surface, point: usize) -> bool {
    surface.faces.iter().enumerate().any(|(fi, f)| {
        f.contains(&point)
            && surface
                .face_layer
                .get(fi)
                .and_then(|&li| surface.layers.get(li))
                .is_some_and(|l| l.symmetric)
    })
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        crate::theme::apply(&cc.egui_ctx);
        let mut app = Self::new_detached();
        app.status =
            "No model loaded — start a new design (Ctrl+N) or open a .fef (Ctrl+O).".to_owned();
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

    /// An app with no model, as a launch without `--open` starts (tests).
    pub fn headless_empty() -> Self {
        let mut app = Self::new_detached();
        app.status = "No model loaded.".to_owned();
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
            saved_state: None,
            dialog: None,
            pending: None,
            close_requested: false,
            keyboard_taken: false,
            show_layers: true,
            show_analysis: true,
            dock_tab: DesignDock::Hydrostatics,
            dock_wide: false,
            inspector_pending: None,
            inspector_rects: [egui::Rect::NOTHING; 3],
            layer_list_rect: egui::Rect::NOTHING,
            inspector_rect: egui::Rect::NOTHING,
            show_report_options: false,
            new_design: None,
            project_settings: None,
            start_rects: None,
            report_job: None,
            loading_selected: 0,
            loading_result: None,
            loading_track: Track::default(),
            damage_selected: 0,
            damage_result: None,
            damage_track: Track::default(),
            perf_state: PerformanceState::default(),
            resistance_track: Track::default(),
            raos_track: Track::default(),
            hydro_rev: 0,
            project_rev: 0,
            perf_rev: 0,
            last_change: None,
            cursor_world: None,
            hydro: HydroState::new(),
            viewport_rects: [egui::Rect::NOTHING; 4],
            title_rects: [egui::Rect::NOTHING; 4],
            central_rect: egui::Rect::NOTHING,
            maximised: None,
            analysis_params: panels::AnalysisParams::default(),
            toolbar_rects: Vec::new(),
            analysis_frames: [None, None, None, None],
            raster_counts: [0; 4],
            shaded_hulls: Default::default(),
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
                self.saved_state = Some((fef::write(&surface), project.clone()));
                self.surface = Some(surface);
                self.path = Some(path.to_path_buf());
                self.project = project;
                self.project_error = project_error;
                self.dirty = false;
                self.adopt();
                self.apply_requirement_defaults();
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
        self.saved_state = None;
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
        self.saved_state = None;
        self.adopt();
    }

    /// Reset the derived state after a model change.
    fn adopt(&mut self) {
        self.history.clear();
        self.marks.clear();
        self.reset_derived();
    }

    /// Everything derived from the model, reset for a new one (the undo
    /// history is the caller's business).
    fn reset_derived(&mut self) {
        self.revision += 1;
        self.selected = None;
        self.selected_volume = None;
        self.selected_windage = None;
        self.last_highlighted_tank = None;
        self.last_highlighted_compartment = None;
        self.drag = None;
        self.inspector_pending = None;
        self.cache = None;
        self.hydro.result = None;
        self.hydro.curves = None;
        self.hydro.hull = None;
        self.hydro.track.reset();
        self.hydro.curves_track.reset();
        self.loading_selected = 0;
        self.loading_result = None;
        self.loading_track.reset();
        self.damage_selected = 0;
        self.damage_result = None;
        self.damage_track.reset();
        self.perf_state.resistance = None;
        self.resistance_track.reset();
        self.perf_state.raos = None;
        self.perf_state.sea_result = None;
        self.raos_track.reset();
        self.last_change = Some(std::time::Instant::now());
        self.cursor_world = None;
        self.analysis_frames = [None, None, None, None];
        if let Some(s) = &self.surface {
            let subdiv = seacraft_core::SubdivisionOptions {
                level: self.level,
                mode: self.mode,
            };
            self.hydro.options = HydroOptions::from_surface(s, subdiv);
            let (min, max) = crate::render::display_extents(s);
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
            let (min, max) = crate::render::display_extents(surface);
            // Section curves and combs cost more than the refinement: a drag
            // keeps the ones from before it and rebuilds them on release.
            let carried = match self.drag {
                Some(_) => self.cache.take().and_then(|c| c.curves),
                None => None,
            };
            self.cache = Some(RenderCache {
                revision: self.revision,
                level,
                mode: self.mode,
                mesh,
                curves_stale: carried.is_some(),
                curves: carried,
                min,
                max,
            });
        }
        if self.display.combs {
            let dragging = self.drag.is_some();
            if let Some(cache) = self.cache.as_mut() {
                if cache.curves.is_none() || (cache.curves_stale && !dragging) {
                    cache.curves_stale = false;
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

    /// The right-hand Analysis dock: the pinned live strip, the tab bar and
    /// the active tab's body. The panels return what the user asked for and
    /// it is applied after the panel has been drawn.
    fn analysis_dock(&mut self, ui: &mut egui::Ui) {
        let (tank_highlight, scroll_to_tank) = match self.selected_volume {
            Some(VolumeRef::Tank(idx)) => (Some(idx), self.last_highlighted_tank != Some(idx)),
            _ => (None, false),
        };
        let (comp_highlight, scroll_to_comp) = match self.selected_volume {
            Some(VolumeRef::Compartment(idx)) => {
                (Some(idx), self.last_highlighted_compartment != Some(idx))
            }
            _ => (None, false),
        };
        // Wide and normal are separate panels so each keeps its own width.
        let panel = if self.dock_wide {
            let window = ui.ctx().content_rect().width();
            egui::Panel::right("analysis_dock_wide")
                .resizable(false)
                .exact_size((window * 0.55).round())
        } else {
            egui::Panel::right("analysis_dock")
                .resizable(true)
                .default_size(360.0)
        };
        let mut tab = self.dock_tab;
        let mut toggle_wide = false;
        let mut action = DockAction::None;
        let targets = self.target_readouts();
        let lightship = self.lightship_seed();
        let tab_stale = DesignDock::ALL.map(|t| self.tab_stale(t));
        let failure = self.tab_failure(tab);
        let hydro_stale = self.hydro_stale();
        let curves_stale = self.curves_stale();
        let hydro_busy = self.hydro.track.busy() || self.hydro.curves_track.busy();
        let loading_stale = self.loading_stale();
        let loading_busy = self.loading_track.busy();
        let damage_stale = self.damage_stale();
        let damage_busy = self.damage_track.busy();
        let resistance_stale = self.resistance_stale();
        let raos_stale = self.raos_stale();
        let busy_resistance = self.resistance_track.busy();
        let busy_raos = self.raos_track.busy();
        let default_kg = self.effective_performance_kg_default();
        let kg_source = self.performance_kg_source();
        panel.show(ui, |ui| {
            toggle_wide = panels::live_strip(
                ui,
                self.hydro.result.as_ref(),
                self.hydro.kg,
                hydro_stale,
                self.dock_wide,
            );
            panels::target_strip(ui, &targets, hydro_stale);
            ui.add_space(2.0);
            ui.horizontal_wrapped(|ui| {
                // Tight enough that the five tabs fit one row at 360 px.
                ui.spacing_mut().item_spacing.x = 2.0;
                ui.spacing_mut().button_padding.x = 5.0;
                for (i, t) in DesignDock::ALL.into_iter().enumerate() {
                    let response = ui.selectable_label(tab == t, t.label());
                    if tab_stale[i] {
                        panels::stale_dot(ui, response.rect);
                    }
                    if response.clicked() {
                        tab = t;
                    }
                }
            });
            ui.separator();
            if let Some(msg) = &failure {
                ui.colored_label(crate::theme::FAIL, format!("⚠ {msg}"));
            }
            egui::ScrollArea::vertical().show(ui, |ui| match tab {
                DesignDock::Hydrostatics => {
                    action = DockAction::Hydro(panels::hydrostatics(
                        ui,
                        panels::HydroPanel {
                            options: &mut self.hydro.options,
                            kg: &mut self.hydro.kg,
                            free_trim: &mut self.hydro.free_trim,
                            lcg: &mut self.hydro.lcg,
                            result: self.hydro.result.as_ref(),
                            stale: hydro_stale,
                            busy: hydro_busy,
                            curves: self.hydro.curves.as_deref(),
                            curves_stale,
                        },
                    ));
                }
                DesignDock::Loading => {
                    action = DockAction::Loading(panels::loading(
                        ui,
                        panels::LoadingPanel {
                            project: &mut self.project,
                            selected: &mut self.loading_selected,
                            result: self.loading_result.as_ref(),
                            stale: loading_stale,
                            busy: loading_busy,
                            highlight: tank_highlight,
                            scroll_to: scroll_to_tank,
                            lightship: lightship.clone(),
                        },
                    ));
                }
                DesignDock::Damage => {
                    action = DockAction::Damage(panels::damage(
                        ui,
                        panels::DamagePanel {
                            project: &mut self.project,
                            selected: &mut self.damage_selected,
                            result: self.damage_result.as_ref(),
                            stale: damage_stale,
                            busy: damage_busy,
                            highlight: comp_highlight,
                            scroll_to: scroll_to_comp,
                        },
                    ));
                }
                DesignDock::Windage => {
                    action = DockAction::Windage(panels::windage(
                        ui,
                        panels::WindagePanel {
                            project: &mut self.project,
                            weather: self
                                .loading_result
                                .as_ref()
                                .and_then(|r| r.weather.as_ref()),
                            stale: loading_stale,
                            selected: &mut self.selected_windage,
                        },
                    ));
                }
                DesignDock::Performance => {
                    action = DockAction::Performance(panels::performance(
                        ui,
                        panels::PerformancePanel {
                            state: &mut self.perf_state,
                            default_kg,
                            kg_source,
                            busy_resistance,
                            busy_raos,
                            stale_resistance: resistance_stale,
                            stale_raos: raos_stale,
                        },
                    ));
                }
            });
        });
        self.dock_tab = tab;
        if toggle_wide {
            self.dock_wide = !self.dock_wide;
        }
        self.last_highlighted_tank = tank_highlight;
        self.last_highlighted_compartment = comp_highlight;

        match action {
            DockAction::None => {}
            DockAction::Hydro(panels::HydroAction::Changed) => self.hydro_inputs_changed(),
            DockAction::Hydro(panels::HydroAction::Recompute) => self.start_hydro_job(),
            DockAction::Hydro(panels::HydroAction::CrossCurves) => self.start_curves_job(),
            DockAction::Hydro(panels::HydroAction::None) => {}
            DockAction::Loading(panels::LoadingAction::Changed) => {
                self.dirty = true;
                self.project_changed();
            }
            DockAction::Loading(panels::LoadingAction::Compute) => self.start_loading_compute(),
            DockAction::Loading(panels::LoadingAction::None) => {}
            DockAction::Damage(panels::DamageAction::Changed) => {
                self.dirty = true;
                self.project_changed();
            }
            DockAction::Damage(panels::DamageAction::Compute) => self.start_damage_compute(),
            DockAction::Damage(panels::DamageAction::None) => {}
            DockAction::Windage(panels::WindageAction::Changed) => {
                self.dirty = true;
                self.project_changed();
            }
            DockAction::Windage(panels::WindageAction::None) => {}
            DockAction::Performance(panels::PerformanceAction::Changed) => {
                self.performance_changed();
            }
            DockAction::Performance(panels::PerformanceAction::Compute) => {
                self.start_performance_compute()
            }
            DockAction::Performance(panels::PerformanceAction::None) => {}
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // `App::new` themes the window's context; a headless test's context
        // gets the same type scale on its first frame.
        crate::theme::ensure(ui);
        self.shortcuts(ui);
        self.menu(ui);
        self.toolbar(ui);
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.dirty {
                    ui.colored_label(crate::theme::WARN, "● unsaved");
                }
                ui.label(&self.status);
                let stale = self.stale_count();
                if stale > 0 {
                    ui.separator();
                    let word = if stale == 1 { "result" } else { "results" };
                    ui.colored_label(
                        crate::theme::STALE,
                        format!("{stale} {word} out of date · F5"),
                    );
                }
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
            let mut inspector = panels::InspectorOutput::default();
            egui::Panel::left("layer_panel")
                .resizable(true)
                .default_size(250.0)
                .show(ui, |ui| {
                    // The selection Inspector sits directly under the layer
                    // list, both in one scroll area, so the eye does not
                    // travel the height of the window between them.
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let top = ui.cursor().top();
                        if let Some(surface) = self.surface.as_mut() {
                            changed = panels::layers(ui, surface);
                        } else {
                            ui.label("No model loaded.");
                        }
                        self.layer_list_rect = egui::Rect::from_x_y_ranges(
                            ui.max_rect().x_range(),
                            top..=ui.min_rect().bottom(),
                        );
                        ui.add_space(4.0);
                        ui.separator();
                        let top = ui.cursor().top();
                        inspector = panels::inspector(
                            ui,
                            panels::InspectorPanel {
                                surface: self.surface.as_ref(),
                                selected: self.selected,
                                pending: &mut self.inspector_pending,
                            },
                        );
                        self.inspector_rect = egui::Rect::from_x_y_ranges(
                            ui.max_rect().x_range(),
                            top..=ui.min_rect().bottom(),
                        );
                    });
                });
            if changed {
                self.revision += 1;
                self.dirty = true;
                self.touch_geometry();
            }
            self.inspector_rects = inspector.fields;
            self.apply_inspector_action(inspector.action);
        } else {
            self.inspector_rects = [egui::Rect::NOTHING; 3];
            self.layer_list_rect = egui::Rect::NOTHING;
            self.inspector_rect = egui::Rect::NOTHING;
        }

        if self.show_analysis {
            self.analysis_dock(ui);
        }
        self.run_jobs(ui.ctx());

        if self.surface.is_some() {
            self.start_rects = None;
            egui::CentralPanel::default().show(ui, |ui| self.viewports(ui));
        } else {
            self.start_screen(ui);
        }
        self.new_design_window(ui);
        self.project_settings_window(ui);
        self.dialog_window(ui);
        self.report_options_window(ui);
        self.poll_report(ui.ctx());
        self.intercept_close(ui.ctx());
        self.pending_window(ui);
        if self.close_requested {
            ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
        }
        self.keyboard_taken = ui.ctx().egui_wants_keyboard_input();
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        #[derive(Default)]
        struct Pressed {
            undo: bool,
            redo: bool,
            open: bool,
            save: bool,
            save_as: bool,
            maximise: bool,
            new_design: bool,
            recompute: bool,
        }
        // A modal dialog owns the keyboard: nothing reaches the model
        // behind it.
        if self.new_design.is_some() || self.project_settings.is_some() {
            return;
        }
        // Read the modifiers from the key event itself: `InputState::modifiers`
        // is only refreshed by a `ModifiersChanged` event, which a synthesised
        // test frame need not send.
        let pressed = ui.input(|i| {
            let mut pressed = Pressed::default();
            for event in &i.events {
                if let egui::Event::Key {
                    key,
                    pressed: true,
                    repeat,
                    modifiers,
                    ..
                } = event
                {
                    if *key == egui::Key::F5 && !repeat && modifiers.is_none() {
                        pressed.recompute = true;
                    }
                    if !(modifiers.command || modifiers.ctrl) {
                        continue;
                    }
                    match key {
                        egui::Key::Z if modifiers.shift => pressed.redo = true,
                        egui::Key::Z => pressed.undo = true,
                        egui::Key::Y => pressed.redo = true,
                        egui::Key::O => pressed.open = true,
                        egui::Key::S if modifiers.shift => pressed.save_as = true,
                        egui::Key::S => pressed.save = true,
                        // Not Tab (egui's focus key), not plain Space
                        // (shading); a held key toggles once.
                        egui::Key::Space if !repeat => pressed.maximise = true,
                        egui::Key::N => pressed.new_design = true,
                        _ => {}
                    }
                }
            }
            pressed
        });
        // A text field has its own undo: Ctrl+Z there must not undo the model.
        let typing = ui.ctx().text_edit_focused();
        if pressed.undo && !typing {
            self.undo();
        }
        if pressed.redo && !typing {
            self.redo();
        }
        if pressed.open {
            self.request(PendingAction::Open);
        }
        if pressed.new_design {
            self.request(PendingAction::NewDesign);
        }
        if pressed.save_as && self.surface.is_some() {
            self.open_dialog(DialogKind::SaveAs);
        } else if pressed.save && self.surface.is_some() {
            // No path yet: `save` opens Save as.
            self.save();
        }
        if pressed.maximise && !typing {
            self.toggle_maximised();
        }
        if pressed.recompute {
            self.recompute_stale();
        }

        // Single keys are text while a field (or any focused widget) has the
        // keyboard: typing "1" in a number field must not switch the view.
        if self.keyboard_taken || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let (escape, frame, shade) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Escape),
                i.key_pressed(egui::Key::F),
                // Ctrl+Space maximises; only a plain Space toggles shading.
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::Key {
                            key: egui::Key::Space,
                            pressed: true,
                            modifiers,
                            ..
                        } if !(modifiers.command || modifiers.ctrl)
                    )
                }),
            )
        });
        if escape {
            self.selected = None;
            self.selected_volume = None;
            self.selected_windage = None;
        }
        if frame {
            self.reset_cameras();
        }
        if shade {
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
                self.set_active_view(i);
            }
        }
    }

    fn menu(&mut self, ui: &mut egui::Ui) {
        // "Ctrl+S" here, "⌘S" on a Mac: the same text egui uses elsewhere.
        let ctx = ui.ctx().clone();
        let keys = |modifiers: egui::Modifiers, key: egui::Key| {
            ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
        };
        let command_shift = egui::Modifiers::COMMAND | egui::Modifiers::SHIFT;
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
                ui.menu_button("File", |ui| {
                    if ui
                        .add(
                            egui::Button::new("New design…")
                                .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::N)),
                        )
                        .clicked()
                    {
                        self.request(PendingAction::NewDesign);
                        ui.close();
                    }
                    if ui.button("New demo hull").clicked() {
                        self.request(PendingAction::NewDemo);
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::new("Open .fef…")
                                .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::O)),
                        )
                        .clicked()
                    {
                        self.request(PendingAction::Open);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.surface.is_some(),
                            egui::Button::new("Save")
                                .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::S)),
                        )
                        .clicked()
                    {
                        self.save();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.surface.is_some(),
                            egui::Button::new("Save as…")
                                .shortcut_text(keys(command_shift, egui::Key::S)),
                        )
                        .clicked()
                    {
                        self.open_dialog(DialogKind::SaveAs);
                        ui.close();
                    }
                    ui.separator();
                    if ui
                        .add_enabled(
                            self.surface.is_some() && self.report_job.is_none(),
                            egui::Button::new("Design report…"),
                        )
                        .clicked()
                    {
                        self.open_dialog(DialogKind::Report);
                        ui.close();
                    }
                    if ui.button("Report options…").clicked() {
                        self.show_report_options = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.surface.is_some(),
                            egui::Button::new("Project settings…"),
                        )
                        .clicked()
                    {
                        self.open_project_settings();
                        ui.close();
                    }
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
                        .add_enabled(
                            self.history.can_undo(),
                            egui::Button::new("Undo")
                                .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::Z)),
                        )
                        .clicked()
                    {
                        self.undo();
                        ui.close();
                    }
                    if ui
                        .add_enabled(
                            self.history.can_redo(),
                            egui::Button::new("Redo")
                                .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::Y)),
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
                        // Each mode with its remembered parameter (the
                        // toolbar edits it; 14 stripes, 1 m, 3° by default).
                        let params = self.analysis_params;
                        for (kind, label) in [
                            (seacraft_core::AnalysisMode::Shaded, "Shaded"),
                            (seacraft_core::AnalysisMode::Zebra { stripes: 0.0 }, "Zebra"),
                            (
                                seacraft_core::AnalysisMode::Curvature { radius: 0.0 },
                                "Curvature",
                            ),
                            (
                                seacraft_core::AnalysisMode::Draft { angle_deg: 0.0 },
                                "Draft",
                            ),
                        ] {
                            let on = panels::same_analysis_kind(current, kind);
                            if ui.selectable_label(on, label).clicked() && !on {
                                self.set_analysis(params.with_kind(kind));
                            }
                        }
                    });
                    ui.separator();
                    ui.checkbox(&mut self.show_layers, "Layer panel");
                    // One right-hand dock with tabs: several right-hand panels
                    // leave no room for the viewports.
                    ui.checkbox(&mut self.show_analysis, "Analysis dock");
                    for tab in DesignDock::ALL {
                        let name = match tab {
                            DesignDock::Hydrostatics => "Hydrostatics",
                            other => other.label(),
                        };
                        if ui
                            .radio(self.show_analysis && self.dock_tab == tab, name)
                            .clicked()
                        {
                            self.set_dock_tab(tab);
                        }
                    }
                    ui.separator();
                    let mut level = self.level as u32;
                    if ui
                        .add(egui::Slider::new(&mut level, 0..=5).text("subdivision level"))
                        .changed()
                    {
                        // Through the setters: shading caches and the
                        // hydrostatics key on the revision.
                        self.set_subdivision_level(level as u8);
                    }
                    let mut mode = self.mode;
                    egui::ComboBox::from_label("mode")
                        .selected_text(match mode {
                            SubdivisionMode::Loop => "Loop",
                            SubdivisionMode::CatmullClark => "Catmull-Clark",
                        })
                        .show_ui(ui, |ui| {
                            ui.selectable_value(&mut mode, SubdivisionMode::Loop, "Loop");
                            ui.selectable_value(
                                &mut mode,
                                SubdivisionMode::CatmullClark,
                                "Catmull-Clark",
                            );
                        });
                    if mode != self.mode {
                        self.set_subdivision_mode(mode);
                    }
                    ui.separator();
                    if ui
                        .add(egui::Button::new("Reset cameras").shortcut_text("F"))
                        .clicked()
                    {
                        self.reset_cameras();
                        ui.close();
                    }
                    if ui
                        .add(
                            egui::Button::selectable(
                                self.maximised.is_some(),
                                "Maximise active view",
                            )
                            .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::Space)),
                        )
                        .clicked()
                    {
                        self.toggle_maximised();
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

    /// The slim toolbar under the menu bar; its actions are applied here,
    /// after the panel has drawn.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let state = panels::ToolbarState {
            display: &self.display,
            params: self.analysis_params,
            level: self.level,
            mode: self.mode,
            maximised: self.maximised.is_some(),
        };
        let out = egui::Panel::top("toolbar")
            .frame(
                egui::Frame::side_top_panel(ui.style()).inner_margin(egui::Margin::symmetric(8, 3)),
            )
            .show(ui, |ui| panels::toolbar(ui, &state))
            .inner;
        if let Some(display) = out.display {
            self.display = display;
        }
        if let Some(mode) = out.analysis {
            self.set_analysis(mode);
        }
        if let Some(level) = out.level {
            self.set_subdivision_level(level);
        }
        if let Some(mode) = out.mode {
            self.set_subdivision_mode(mode);
        }
        if out.fit {
            self.reset_cameras();
        }
        if out.toggle_maximise {
            self.toggle_maximised();
        }
        self.toolbar_rects = out.rects;
    }

    // ---- start screen, New design, Project settings ---------------------

    /// The central area with no model: the start card.
    fn start_screen(&mut self, ui: &mut egui::Ui) {
        let (action, rects) = egui::CentralPanel::default()
            .show(ui, crate::new_design::start_card)
            .inner;
        self.start_rects = Some(rects);
        match action {
            crate::new_design::StartAction::NewDesign => self.request(PendingAction::NewDesign),
            crate::new_design::StartAction::Open => self.request(PendingAction::Open),
            crate::new_design::StartAction::Demo => self.request(PendingAction::NewDemo),
            crate::new_design::StartAction::None => {}
        }
    }

    /// Open the New design dialog (callers go through the unsaved-changes
    /// prompt first: Ctrl+N, File > New design…, the start card).
    pub fn open_new_design(&mut self) {
        self.new_design = Some(crate::new_design::NewDesignState::new(
            crate::new_design::SEA_WATER_T_M3,
        ));
    }

    /// Ask for the New design dialog as Ctrl+N does: through the
    /// unsaved-changes prompt when there are unsaved edits.
    pub fn request_new_design(&mut self) {
        self.request(PendingAction::NewDesign);
    }

    fn new_design_window(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.new_design.as_mut() else {
            return;
        };
        match crate::new_design::new_design_dialog(ui.ctx(), state) {
            crate::new_design::NewDesignAction::None => {}
            crate::new_design::NewDesignAction::Cancel => self.new_design = None,
            crate::new_design::NewDesignAction::Create(request) => {
                self.new_design = None;
                self.apply_new_design(request);
            }
        }
    }

    /// Carry out the New design dialog's *Create*: a new model and project
    /// as one undo step (an import is a plain Open).
    pub fn apply_new_design(&mut self, request: crate::new_design::NewDesignRequest) {
        use crate::new_design::NewDesignRequest;
        match request {
            NewDesignRequest::Import => self.open_dialog(DialogKind::Open),
            NewDesignRequest::Demo { half } => {
                let before = self.current_snapshot(true);
                self.load_demo_shape(half);
                self.history.snapshot(&before);
            }
            NewDesignRequest::Parametric(design) => {
                let crate::new_design::ParametricDesign {
                    spec,
                    name,
                    requirements,
                    built,
                } = *design;
                let before = self.current_snapshot(true);
                let result = match built {
                    Some(p) if p.spec == spec => {
                        crate::commands::install_generated(self, p.surface, p.report)
                    }
                    _ => match crate::commands::generate(self, &spec) {
                        Ok(r) => r,
                        Err(e) => {
                            self.status = format!("New design failed: {e}");
                            return;
                        }
                    },
                };
                if let Some(s) = self.surface.as_mut() {
                    if !name.is_empty() {
                        s.settings.name = name.clone();
                    }
                }
                // The design waterline is the requested draft.
                self.hydro.options.draft = spec.draft;
                if let Some(kg) = requirements.kg {
                    self.hydro.kg = kg;
                }
                self.project.requirements = Some(requirements);
                self.apply_requirement_defaults();
                self.reset_cameras();
                self.history.snapshot(&before);
                self.status = format!(
                    "{name} — new {} design ({} points, {} faces)",
                    spec.kind, result.points, result.faces
                );
            }
        }
    }

    /// Inputs that follow the design requirements: the Performance speed
    /// range centres on the design speed.
    fn apply_requirement_defaults(&mut self) {
        let speed = self
            .project
            .requirements
            .as_ref()
            .and_then(|r| r.design_speed_kn)
            .filter(|v| v.is_finite() && *v > 0.0);
        if let Some(v) = speed {
            self.perf_state.speeds_kn = crate::new_design::speed_range(v);
            self.perf_state.sk_speed_kn = v;
            self.performance_changed();
        }
    }

    /// The lightship a new loading condition starts from: the measured
    /// displacement at its LCB, at the requirements' KG estimate (the dock's
    /// KG without one) above the keel.
    pub fn lightship_seed(&self) -> seacraft_core::loading::WeightItem {
        let keel = self
            .surface
            .as_ref()
            .map_or(0.0, |s| s.lowest_hydrostatics_point());
        let kg = self
            .project
            .requirements
            .as_ref()
            .and_then(|r| r.kg)
            .unwrap_or(self.hydro.kg);
        let (mass_t, x) = match &self.hydro.result {
            Some(h) if h.displacement_t > 0.0 => (h.displacement_t, h.lcb.x),
            _ => (10.0, 0.0),
        };
        seacraft_core::loading::WeightItem {
            name: "Lightship".into(),
            mass_t,
            x,
            y: 0.0,
            z: keel + kg,
        }
    }

    /// The measured values the pinned strip shows against the design
    /// requirements (empty without requirements or hydrostatics).
    pub fn target_readouts(&self) -> Vec<panels::TargetReadout> {
        match (&self.hydro.result, &self.project.requirements) {
            (Some(h), Some(r)) => panels::target_readouts(h, r),
            _ => Vec::new(),
        }
    }

    /// Open File > Project settings for the current model.
    pub fn open_project_settings(&mut self) {
        if let Some(s) = &self.surface {
            self.project_settings = Some(crate::new_design::ProjectSettingsState::new(
                s,
                self.project.requirements.as_ref(),
            ));
        }
    }

    fn project_settings_window(&mut self, ui: &mut egui::Ui) {
        let Some(state) = self.project_settings.as_mut() else {
            return;
        };
        match crate::new_design::project_settings_dialog(ui.ctx(), state) {
            crate::new_design::ProjectSettingsAction::None => {}
            crate::new_design::ProjectSettingsAction::Cancel => self.project_settings = None,
            crate::new_design::ProjectSettingsAction::Apply(edit) => {
                self.project_settings = None;
                let e = *edit;
                self.apply_project_settings(e.name, e.designer, e.water_density, e.requirements);
            }
        }
    }

    /// Apply File > Project settings. The name, designer and density live in
    /// the `.fef` and are one undo step; the requirements live in the
    /// project and, like every dock edit of it, mark the model unsaved.
    pub fn apply_project_settings(
        &mut self,
        name: String,
        designer: String,
        water_density: f64,
        requirements: Option<seacraft_core::project::Requirements>,
    ) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let st = &surface.settings;
        let settings_changed =
            st.name != name || st.designer != designer || st.water_density != water_density;
        if settings_changed {
            self.snapshot_undo();
            if let Some(s) = self.surface.as_mut() {
                s.settings.name = name;
                s.settings.designer = designer;
                s.settings.water_density = water_density;
            }
            self.hydro.options.water_density = water_density;
            self.mark_geometry_changed();
        }
        if requirements != self.project.requirements {
            let speed = |r: &Option<seacraft_core::project::Requirements>| {
                r.as_ref().and_then(|r| r.design_speed_kn)
            };
            let speed_changed = speed(&requirements) != speed(&self.project.requirements);
            self.project.requirements = requirements;
            self.dirty = true;
            if speed_changed {
                self.apply_requirement_defaults();
            }
        }
        self.status = "project settings applied".to_owned();
    }

    /// The New design dialog is open.
    pub fn new_design_open(&self) -> bool {
        self.new_design.is_some()
    }

    /// The New design dialog's state (tests and the control channel).
    pub fn new_design_mut(&mut self) -> Option<&mut crate::new_design::NewDesignState> {
        self.new_design.as_mut()
    }

    /// The Project settings dialog's state, while it is open.
    pub fn project_settings_mut(&mut self) -> Option<&mut crate::new_design::ProjectSettingsState> {
        self.project_settings.as_mut()
    }

    /// The start card's buttons from the last frame; `None` while a model
    /// is shown (test hook).
    pub fn start_rects(&self) -> Option<crate::new_design::StartRects> {
        self.start_rects
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

    /// The window manager's close (×, Alt+F4) asks about unsaved work, as
    /// File > Quit does. A close already confirmed (`close_requested`: the
    /// prompt's Discard or Save, Quit on a clean model, `app.quit`) goes
    /// through, so the `Close` command it sends does not prompt again.
    fn intercept_close(&mut self, ctx: &egui::Context) {
        if self.close_requested || !ctx.input(|i| i.viewport().close_requested()) {
            return;
        }
        if self.dirty {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            self.pending = Some(PendingAction::Quit);
        }
    }

    /// The unsaved-changes prompt is open.
    pub fn unsaved_prompt_open(&self) -> bool {
        self.pending.is_some()
    }

    fn perform(&mut self, action: PendingAction) {
        match action {
            PendingAction::NewDemo => self.load_demo(),
            PendingAction::NewDesign => self.open_new_design(),
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
            DialogKind::Report => dir
                .join(format!("{clean}-report.html"))
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
            then: None,
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
        match seacraft_core::io::write_atomic(path, &text) {
            Ok(()) => {
                self.path = Some(path.to_path_buf());
                self.status = format!("saved {}", path.display());
                let sidecar = project::sidecar_path(path);
                if let Some(e) = &self.project_error {
                    self.status = format!(
                        "saved {}; project file left as it was ({e})",
                        path.display()
                    );
                    // The sidecar on disk could not be read, so it is not
                    // overwritten; design data entered since stays unsaved.
                    self.saved_state = Some((text, Project::default()));
                    self.refresh_dirty();
                    if self.dirty {
                        self.status
                            .push_str(" — design data NOT saved: fix or remove the project file");
                    }
                    return true;
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
                self.saved_state = Some((text, self.project.clone()));
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
        use seacraft_core::io::write_atomic;
        let Some(surface) = &self.surface else {
            return false;
        };
        // The drawing shows what the viewports show: their refinement and
        // combs, as the CLI does with its flags.
        let linesplan_opts = seacraft_core::linesplan::LinesplanOptions {
            level: self.level,
            mode: self.mode,
            combs: self.display.combs,
            ..Default::default()
        };
        let written = match kind {
            ExportKind::Obj => write_atomic(
                path,
                export::obj(surface, self.level, self.mode, export::UpAxis::Z),
            ),
            ExportKind::Stl => write_atomic(path, export::stl(surface, self.level, self.mode)),
            ExportKind::ThreeMf => {
                write_atomic(path, export::threemf(surface, self.level, self.mode))
            }
            ExportKind::Ply => write_atomic(path, export::ply(surface, self.level, self.mode)),
            ExportKind::HullIges => {
                write_atomic(path, export::iges_surface(surface, self.level, self.mode))
            }
            ExportKind::Html => {
                write_atomic(path, export::viewer_html(surface, self.level, self.mode))
            }
            ExportKind::Linesplan => {
                write_atomic(path, seacraft_core::linesplan::svg(surface, linesplan_opts))
            }
            ExportKind::LinesplanDxf => {
                write_atomic(path, seacraft_core::linesplan::dxf(surface, linesplan_opts))
            }
            ExportKind::LinesplanIges => write_atomic(
                path,
                seacraft_core::linesplan::iges(surface, linesplan_opts),
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
            DialogKind::Report => "Design report",
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
                    ui.colored_label(crate::theme::FAIL, err);
                }
                ui.separator();
                let wanted = match dialog.kind {
                    DialogKind::Open | DialogKind::SaveAs => ".fef".to_owned(),
                    DialogKind::Export(k) => format!(".{}", k.extension()),
                    DialogKind::Report => ".html".to_owned(),
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
            DialogKind::Report => self.start_report(&path),
        };
        if !ok {
            dialog.error = Some(self.status.clone());
            self.dialog = Some(dialog);
        } else if let Some(action) = dialog.then {
            if !self.dirty {
                self.perform(action);
            }
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
                if let Some(dialog) = self
                    .dialog
                    .as_mut()
                    .filter(|d| d.kind == DialogKind::SaveAs)
                {
                    // An untitled model: the prompt hands over to "Save as",
                    // which carries the action out once the file is written.
                    dialog.then = Some(action);
                    self.pending = None;
                } else if !self.dirty {
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
            let (min, max) = crate::render::display_extents(s);
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

    /// The model and its measurement draft, as the history stores them;
    /// with `whole`, also the project and path (the other side of a step
    /// that replaced the whole model).
    fn current_snapshot(&self, whole: bool) -> Snapshot {
        Snapshot {
            surface: self.surface.clone(),
            draft: self.hydro.options.draft,
            model: whole.then(|| {
                Box::new(ModelState {
                    project: self.project.clone(),
                    project_error: self.project_error.clone(),
                    path: self.path.clone(),
                    saved_state: self.saved_state.clone(),
                })
            }),
        }
    }

    /// Put a snapshot back as the current model.
    fn restore_snapshot(&mut self, snap: Snapshot) {
        let density = |s: &Option<Surface>| s.as_ref().map(|s| s.settings.water_density);
        if let Some(d) = density(&snap.surface).filter(|d| Some(*d) != density(&self.surface)) {
            // A Project settings density change undone.
            self.hydro.options.water_density = d;
        }
        self.surface = snap.surface;
        if let Some(m) = snap.model {
            self.project = m.project;
            self.project_error = m.project_error;
            self.path = m.path;
            self.saved_state = m.saved_state;
            self.reset_derived();
            self.apply_requirement_defaults();
        }
        self.hydro.options.draft = snap.draft;
        self.revision += 1;
        self.refresh_dirty();
        self.touch_geometry();
    }

    pub fn undo(&mut self) {
        let whole = self.history.peek_undo().is_some_and(|s| s.model.is_some());
        let current = self.current_snapshot(whole);
        if let Some(previous) = self.history.undo(&current) {
            let moved = previous.surface.as_ref().map_or(0, |s| s.points.len());
            self.restore_snapshot(previous);
            self.status = if whole {
                "undo — previous model restored".to_owned()
            } else {
                format!("undo — {moved} control points restored")
            };
        }
    }

    pub fn redo(&mut self) {
        let whole = self.history.peek_redo().is_some_and(|s| s.model.is_some());
        let current = self.current_snapshot(whole);
        if let Some(next) = self.history.redo(&current) {
            self.restore_snapshot(next);
            self.status = "redo".to_owned();
        }
    }

    /// Snapshot the current state as a mark for rollback, returning a mark ID.
    pub fn mark(&mut self) -> u64 {
        let id = self.next_mark_id;
        self.next_mark_id += 1;
        self.marks.push((
            id,
            self.surface.clone(),
            self.project.clone(),
            self.history.undo_len(),
        ));
        id
    }

    /// Restore the state to the given mark ID, dropping all marks taken after it.
    pub fn restore_mark(&mut self, id: u64) -> Result<(), String> {
        let pos = self
            .marks
            .iter()
            .position(|(m, _, _, _)| *m == id)
            .ok_or_else(|| format!("unknown mark {id}"))?;
        let (_, surface, project, undo_len) = self.marks[pos].clone();
        self.marks.truncate(pos);
        // A batch may have edited the project as well as the model: roll
        // both back, as the headless server does.
        if project != self.project {
            self.set_project(project);
        }
        self.surface = surface;
        self.history.truncate_undo(undo_len);
        self.history.clear_redo();
        self.selected = None;
        self.mark_geometry_changed();
        self.refresh_dirty();
        self.status = format!("restored to mark {id}");
        Ok(())
    }

    /// Drop a mark without restoring.
    pub fn forget_mark(&mut self, id: u64) {
        if let Some(pos) = self.marks.iter().position(|(m, _, _, _)| *m == id) {
            self.marks.truncate(pos);
        }
    }

    fn viewports(&mut self, ui: &mut egui::Ui) {
        self.sync_cache();
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        self.central_rect = rect;
        // A maximised view takes the whole area; the others are neither
        // drawn nor picked.
        let cells: Vec<(egui::Rect, usize)> = match self.maximised {
            Some(view) => vec![(rect, view)],
            None => render::quad(rect).to_vec(),
        };
        self.viewport_rects = [egui::Rect::NOTHING; 4];
        self.title_rects = [egui::Rect::NOTHING; 4];
        let mut toggle_maximise = false;
        let mut fresh: Vec<(usize, AnalysisFrame)> = Vec::new();
        let mut fresh_hulls: Vec<(usize, u64, Arc<egui::Mesh>)> = Vec::new();
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
        // Whether a point drag was under way when the frame began: views
        // other than the dragged one keep their analysis frame until the
        // frame after release, which refreshes all of them once.
        let drag_view = self.drag.as_ref().map(|d| d.view);
        let mut ready = self.raster_stale_views(&cells, &volume_overlays, &windage_overlays);

        for (cell, index) in cells {
            self.viewport_rects[index] = cell;
            let response = ui.allocate_rect(cell, egui::Sense::click_and_drag());
            let view = self.cameras[index].transform();
            let title = render::title_rect(ui.painter(), cell, &self.cameras[index]);
            self.title_rects[index] = title;
            if response.double_clicked()
                && response
                    .interact_pointer_pos()
                    .is_some_and(|p| title.contains(p))
            {
                self.active = index;
                toggle_maximise = true;
            }

            if response.hovered() {
                let (scroll, cursor) =
                    ui.input(|i| (i.smooth_scroll_delta.y, i.pointer.hover_pos()));
                if scroll.abs() > 0.0 {
                    let factor = (-scroll as f64 * 0.0025).exp();
                    match cursor {
                        // The world point under the cursor stays put.
                        Some(cursor) => self.cameras[index].zoom_at(factor, cursor, cell),
                        None => self.cameras[index].zoom(factor),
                    }
                }
            }

            if response.dragged_by(egui::PointerButton::Middle) {
                // Middle-drag pans in every view, perspective included.
                let delta = response.drag_delta();
                self.cameras[index].pan(delta.x, delta.y, cell);
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
                            // Full refinement and measurement again; the
                            // automatic recomputes count from the release.
                            self.revision += 1;
                            self.touch_geometry();
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

            if let Some(scene) = self.scene(&volume_overlays, &windage_overlays) {
                // An analysis mode needs per-pixel shading, which only the
                // rasterizer provides: render it into a texture and reuse it
                // until the model, camera, size or mode changes.
                let texture = if self.display.analysis.is_shaded() {
                    None
                } else {
                    let (key, view_key) = self.analysis_keys(index, cell, &scene);
                    if self.frame_current(index, key, view_key, drag_view) {
                        self.analysis_frames[index]
                            .as_ref()
                            .map(|f| f.texture.clone())
                    } else {
                        // Rastered before the loop unless this view's camera
                        // or mesh changed since.
                        let image =
                            ready[index]
                                .take()
                                .filter(|r| r.key == key)
                                .unwrap_or_else(|| {
                                    RasterImage::render(
                                        &self.cameras[index],
                                        cell,
                                        &scene,
                                        &self.display,
                                        key,
                                    )
                                });
                        let texture = ui.ctx().load_texture(
                            format!("analysis-{index}"),
                            egui::ColorImage::from_rgba_unmultiplied(image.size, &image.rgba),
                            egui::TextureOptions::LINEAR,
                        );
                        fresh.push((
                            index,
                            AnalysisFrame {
                                key,
                                view_key,
                                texture: texture.clone(),
                            },
                        ));
                        Some(texture)
                    }
                };
                // The painter's shaded hull, kept while the view, the mesh
                // and the layer colours stay the same.
                let hull = (texture.is_none() && self.display.shaded).then(|| {
                    let stamp = self.cache.as_ref().map_or(0, RenderCache::stamp);
                    let camera = &self.cameras[index];
                    let key = render::hull_key(stamp, camera, cell, &scene, &self.display);
                    match &self.shaded_hulls[index] {
                        Some((k, hull)) if *k == key => hull.clone(),
                        _ => {
                            let hull =
                                Arc::new(render::shaded_hull(&camera.transform(), cell, &scene));
                            fresh_hulls.push((index, key, hull.clone()));
                            hull
                        }
                    }
                });
                render::viewport_with_texture(
                    ui,
                    cell,
                    &self.cameras[index],
                    Some(&scene),
                    &self.display,
                    self.active == index,
                    self.selected,
                    texture.as_ref(),
                    hull,
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
            self.raster_counts[index] += 1;
        }
        for (index, key, hull) in fresh_hulls {
            self.shaded_hulls[index] = Some((key, hull));
        }
        if toggle_maximise {
            self.toggle_maximised();
        }
    }

    /// The scene the viewports draw, once a model is loaded and refined.
    fn scene<'a>(
        &'a self,
        volumes: &'a [render::VolumeOverlay],
        windage: &'a [render::WindageOverlay],
    ) -> Option<Scene<'a>> {
        let (surface, cache) = (self.surface.as_ref()?, self.cache.as_ref()?);
        Some(Scene {
            surface,
            mesh: &cache.mesh,
            curves: cache.curves.as_ref(),
            volumes,
            windage,
            min: cache.min,
            max: cache.max,
            draft: self.hydro.options.draft,
            trim: self.hydro.options.trim,
            heel_deg: self.hydro.options.heel_deg,
        })
    }

    /// View `index`'s analysis frame key and view key for `cell`.
    fn analysis_keys(&self, index: usize, cell: egui::Rect, scene: &Scene) -> (u64, u64) {
        let stamp = self.cache.as_ref().map_or(0, RenderCache::stamp);
        let camera = &self.cameras[index];
        (
            render::frame_key(stamp, camera, cell, scene, &self.display),
            render::view_key(camera, cell, scene, &self.display),
        )
    }

    /// Whether view `index`'s analysis frame can be shown as it is: it was
    /// rastered for this view and mesh, or a point is being dragged in
    /// another view (`drag_view`) and this view's camera has not moved.
    fn frame_current(
        &self,
        index: usize,
        key: u64,
        view_key: u64,
        drag_view: Option<usize>,
    ) -> bool {
        let keep = drag_view.is_some_and(|v| v != index);
        self.analysis_frames[index]
            .as_ref()
            .is_some_and(|f| f.key == key || (keep && f.view_key == view_key))
    }

    /// Rasterize, in parallel, the analysis frames that are out of date as
    /// the frame begins (a new mode, a resize, the refresh after a drag).
    /// The view being dragged in is left to the loop, where its mesh
    /// changes; a view whose camera moves this frame is rastered there too.
    fn raster_stale_views(
        &self,
        cells: &[(egui::Rect, usize)],
        volumes: &[render::VolumeOverlay],
        windage: &[render::WindageOverlay],
    ) -> [Option<RasterImage>; 4] {
        let mut ready: [Option<RasterImage>; 4] = Default::default();
        if self.display.analysis.is_shaded() {
            return ready;
        }
        let Some(scene) = self.scene(volumes, windage) else {
            return ready;
        };
        let drag_view = self.drag.as_ref().map(|d| d.view);
        let stale: Vec<(egui::Rect, usize, u64)> = cells
            .iter()
            .filter(|&&(_, index)| drag_view != Some(index))
            .filter_map(|&(cell, index)| {
                let (key, view_key) = self.analysis_keys(index, cell, &scene);
                (!self.frame_current(index, key, view_key, drag_view)).then_some((cell, index, key))
            })
            .collect();
        let (cameras, display) = (&self.cameras, &self.display);
        let images: Vec<(usize, RasterImage)> = if stale.len() < 2 {
            stale
                .iter()
                .map(|&(cell, index, key)| {
                    let image = RasterImage::render(&cameras[index], cell, &scene, display, key);
                    (index, image)
                })
                .collect()
        } else {
            std::thread::scope(|s| {
                let scene = &scene;
                let jobs: Vec<_> = stale
                    .iter()
                    .map(|&(cell, index, key)| {
                        s.spawn(move || {
                            let camera = &cameras[index];
                            (
                                index,
                                RasterImage::render(camera, cell, scene, display, key),
                            )
                        })
                    })
                    .collect();
                jobs.into_iter()
                    .map(|job| job.join().expect("analysis raster thread"))
                    .collect()
            })
        };
        for (index, image) in images {
            ready[index] = Some(image);
        }
        ready
    }

    fn pick_at(
        &self,
        ui: &egui::Ui,
        view: &crate::camera::ViewTransform,
        cell: egui::Rect,
    ) -> Option<usize> {
        // Hidden control points cannot be grabbed: a drag would move a point
        // the user cannot see.
        if !self.display.points {
            return None;
        }
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
        let snap = Snapshot::edit(surface, self.hydro.options.draft);
        self.history.snapshot(&snap);
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
            if point_on_symmetric_layer(surface, point) {
                target.y = target.y.max(0.0);
            }
            surface.points[point].p = target;
        }
        self.dirty = true;
        self.revision += 1;
        self.touch_geometry();
        // Refine now (at the drag level), so the dragged view shows the point
        // under the cursor this frame instead of the next.
        self.sync_cache();
    }

    // ---- background jobs and the recompute rule -----------------------
    //
    // One rule for every result (round-4 U11): each carries the revision
    // key it was computed for, and is stale while the key has moved on.
    // Hydrostatics recompute automatically 120 ms after the last change,
    // the loading condition and resistance 300 ms after it, never while a
    // point is dragged. The heavier results (GZ curves, damage, RAOs and sea
    // state) wait for F5 or their button. Every computation runs as a
    // [`Job`]; a result for an old key is discarded.

    /// The key of the hydrostatics and the GZ curves.
    fn hydro_key(&self) -> u64 {
        revision_key(&[self.revision, self.hydro_rev])
    }

    fn loading_key(&self) -> u64 {
        revision_key(&[
            self.revision,
            self.hydro_rev,
            self.project_rev,
            self.loading_selected as u64,
        ])
    }

    fn damage_key(&self) -> u64 {
        revision_key(&[
            self.revision,
            self.hydro_rev,
            self.project_rev,
            self.damage_selected as u64,
        ])
    }

    fn resistance_key(&self) -> u64 {
        revision_key(&[self.revision, self.hydro_rev, self.perf_rev])
    }

    fn raos_key(&self) -> u64 {
        // The KG may come from the loading condition.
        revision_key(&[
            self.revision,
            self.hydro_rev,
            self.perf_rev,
            self.project_rev,
            self.loading_selected as u64,
        ])
    }

    /// The floating condition changed (draft, trim, heel, KG, free trim).
    fn hydro_inputs_changed(&mut self) {
        self.hydro_rev += 1;
        self.last_change = Some(std::time::Instant::now());
    }

    /// The project's loading conditions, damage cases or windage changed.
    fn project_changed(&mut self) {
        self.project_rev += 1;
        self.last_change = Some(std::time::Instant::now());
    }

    /// The Performance tab's inputs changed.
    fn performance_changed(&mut self) {
        self.perf_rev += 1;
        self.last_change = Some(std::time::Instant::now());
    }

    /// Note that the geometry moved (the caller bumped `revision`): the
    /// automatic recomputes wait for the debounce from now.
    fn touch_geometry(&mut self) {
        self.last_change = Some(std::time::Instant::now());
    }

    /// Collect finished jobs, then start the automatic recomputes that are
    /// due. Called once per frame; never blocks.
    fn run_jobs(&mut self, ctx: &egui::Context) {
        self.poll_jobs();
        let quiet = |ms: u64| {
            self.last_change
                .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(ms))
        };
        let (hydro_quiet, quiet) = (quiet(120), quiet(300));
        let mut waiting = false;
        if self.drag.is_none() && self.surface.is_some() {
            if self.hydro.track.due(self.hydro_key()) {
                if hydro_quiet {
                    self.start_hydro_job();
                } else {
                    waiting = true;
                }
            }
            let loading_due = !self.project.loading_conditions.is_empty()
                && self.loading_track.due(self.loading_key());
            let resistance_due = self.resistance_track.due(self.resistance_key());
            if loading_due || resistance_due {
                if !quiet {
                    waiting = true;
                } else {
                    if loading_due {
                        self.start_loading_compute();
                    }
                    if resistance_due {
                        self.start_resistance_compute();
                    }
                }
            }
        }
        let busy = self.hydro.track.busy()
            || self.hydro.curves_track.busy()
            || self.loading_track.busy()
            || self.damage_track.busy()
            || self.resistance_track.busy()
            || self.raos_track.busy();
        if busy || waiting {
            ctx.request_repaint_after(std::time::Duration::from_millis(50));
        }
    }

    /// Take the results of finished jobs. A failure goes to the status line
    /// (and stays on its tab until the inputs change).
    fn poll_jobs(&mut self) {
        let key = self.hydro_key();
        match self.hydro.track.poll(key) {
            Some(Ok(out)) => self.install_hydro(out),
            Some(Err(e)) => self.status = format!("Hydrostatics failed: {e}"),
            None => {}
        }
        match self.hydro.curves_track.poll(key) {
            Some(Ok(out)) => self.install_curves(out, key),
            Some(Err(e)) => self.status = format!("GZ curves failed: {e}"),
            None => {}
        }
        match self.loading_track.poll(self.loading_key()) {
            Some(Ok(r)) => self.loading_result = Some(r),
            Some(Err(e)) => self.status = format!("Loading compute failed: {e}"),
            None => {}
        }
        match self.damage_track.poll(self.damage_key()) {
            Some(Ok(r)) => self.damage_result = Some(r),
            Some(Err(e)) => self.status = format!("Damage compute failed: {e}"),
            None => {}
        }
        match self.resistance_track.poll(self.resistance_key()) {
            Some(Ok(r)) => self.perf_state.resistance = Some(r),
            Some(Err(e)) => self.status = format!("Resistance compute failed: {e}"),
            None => {}
        }
        match self.raos_track.poll(self.raos_key()) {
            Some(Ok((r, irr))) => {
                self.perf_state.raos = Some(r);
                self.perf_state.sea_result = irr;
            }
            Some(Err(e)) => self.status = format!("Seakeeping compute failed: {e}"),
            None => {}
        }
    }

    /// Measure the hydrostatics on a job (the previous report stays shown,
    /// dimmed, until it lands).
    fn start_hydro_job(&mut self) {
        let Some(surface) = &self.surface else {
            return;
        };
        let inputs = self.hydro.inputs(surface, self.level, self.mode);
        let key = self.hydro_key();
        self.hydro
            .track
            .start(key, move || Ok(measure_hydro(&inputs)));
    }

    fn install_hydro(&mut self, out: HydroOut) {
        if let Some(trim) = out.trim {
            self.hydro.options.trim = trim;
            if self.hydro.lcg.is_none() {
                self.hydro.lcg = Some(out.result.lcb.x);
            }
        }
        self.hydro.result = Some(out.result);
        self.hydro.hull = Some(out.hull);
    }

    /// The GZ curves' inputs: the current measurement's hull when there is
    /// one, otherwise the job measures first.
    fn curves_inputs(&self) -> Option<CurvesInputs> {
        let surface = self.surface.as_ref()?;
        let measured = match (&self.hydro.hull, &self.hydro.result) {
            (Some(hull), Some(h)) if self.hydro.track.is_current(self.hydro_key()) => {
                Some((hull.clone(), h.displacement_t))
            }
            _ => None,
        };
        Some(CurvesInputs {
            hydro: self.hydro.inputs(surface, self.level, self.mode),
            kg: self.hydro.kg,
            measured,
        })
    }

    /// Compute the GZ cross curves on a job ("GZ curves", F5).
    fn start_curves_job(&mut self) {
        let Some(inputs) = self.curves_inputs() else {
            return;
        };
        let key = self.hydro_key();
        self.status = "computing GZ cross curves…".to_owned();
        self.hydro
            .curves_track
            .start(key, move || Ok(compute_cross_curves_for(inputs)));
    }

    fn install_curves(&mut self, out: CurvesOut, key: u64) {
        if let Some(h) = out.hydro {
            self.install_hydro(h);
            self.hydro.track.stamp(key);
        }
        if self.hydro.free_trim && self.hydro.lcg.is_none() {
            self.hydro.lcg = out.lcg;
        }
        let free = self.hydro.free_trim;
        self.hydro.curves = out.curves;
        self.status = if self.hydro.curves.is_none() {
            "GZ curves: nothing is submerged at this draft".to_owned()
        } else if free {
            format!(
                "GZ cross curves for {:.2} t and ±30 %, free to trim about {:.3} m",
                out.displacement_t,
                out.lcg.unwrap_or_default()
            )
        } else {
            format!("GZ cross curves for {:.2} t and ±30 %", out.displacement_t)
        };
    }

    /// F5: recompute every stale result that is shown — the automatic ones
    /// at once, without the debounce, and the heavy ones of the open tab.
    pub fn recompute_stale(&mut self) {
        if self.drag.is_some() || self.surface.is_none() {
            return;
        }
        let wanted =
            |current: bool, running: Option<u64>, key: u64| !current && running != Some(key);
        let key = self.hydro_key();
        if wanted(
            self.hydro.track.is_current(key),
            self.hydro.track.running_for(),
            key,
        ) {
            self.start_hydro_job();
        }
        let key = self.loading_key();
        if !self.project.loading_conditions.is_empty()
            && wanted(
                self.loading_track.is_current(key),
                self.loading_track.running_for(),
                key,
            )
        {
            self.start_loading_compute();
        }
        let key = self.resistance_key();
        if wanted(
            self.resistance_track.is_current(key),
            self.resistance_track.running_for(),
            key,
        ) {
            self.start_resistance_compute();
        }
        if !self.show_analysis {
            return;
        }
        match self.dock_tab {
            DesignDock::Hydrostatics => {
                let key = self.hydro_key();
                if self.curves_stale() && self.hydro.curves_track.running_for() != Some(key) {
                    self.start_curves_job();
                }
            }
            DesignDock::Damage => {
                if self.damage_stale() && self.damage_track.running_for() != Some(self.damage_key())
                {
                    self.start_damage_compute();
                }
            }
            DesignDock::Performance => {
                if self.raos_stale() && self.raos_track.running_for() != Some(self.raos_key()) {
                    self.start_raos_compute();
                }
            }
            DesignDock::Loading | DesignDock::Windage => {}
        }
    }

    fn hydro_stale(&self) -> bool {
        self.hydro.result.is_some() && !self.hydro.track.is_current(self.hydro_key())
    }

    fn curves_stale(&self) -> bool {
        self.hydro.curves.is_some() && !self.hydro.curves_track.is_current(self.hydro_key())
    }

    fn loading_stale(&self) -> bool {
        self.loading_result.is_some() && !self.loading_track.is_current(self.loading_key())
    }

    /// A loading result for the current inputs is shown.
    fn loading_current(&self) -> bool {
        self.loading_result.is_some() && self.loading_track.is_current(self.loading_key())
    }

    fn damage_stale(&self) -> bool {
        self.damage_result.is_some() && !self.damage_track.is_current(self.damage_key())
    }

    fn resistance_stale(&self) -> bool {
        self.perf_state.resistance.is_some()
            && !self.resistance_track.is_current(self.resistance_key())
    }

    fn raos_stale(&self) -> bool {
        (self.perf_state.raos.is_some() || self.perf_state.sea_result.is_some())
            && !self.raos_track.is_current(self.raos_key())
    }

    /// The results shown out of date, by name (status bar, tests).
    pub fn stale_results(&self) -> Vec<&'static str> {
        [
            (self.hydro_stale(), "hydrostatics"),
            (self.curves_stale(), "GZ curves"),
            (self.loading_stale(), "loading"),
            (self.damage_stale(), "damage"),
            (self.resistance_stale(), "resistance"),
            (self.raos_stale(), "RAOs"),
        ]
        .into_iter()
        .filter_map(|(stale, name)| stale.then_some(name))
        .collect()
    }

    /// How many results are shown out of date (the status bar's count).
    pub fn stale_count(&self) -> usize {
        self.stale_results().len()
    }

    /// A tab shows a stale result (its tab gets a dot).
    pub fn tab_stale(&self, tab: DesignDock) -> bool {
        match tab {
            DesignDock::Hydrostatics => self.hydro_stale() || self.curves_stale(),
            DesignDock::Loading => self.loading_stale(),
            DesignDock::Damage => self.damage_stale(),
            DesignDock::Windage => {
                self.loading_stale()
                    && self
                        .loading_result
                        .as_ref()
                        .is_some_and(|r| r.weather.is_some())
            }
            DesignDock::Performance => self.resistance_stale() || self.raos_stale(),
        }
    }

    /// Why a tab's computation failed for the current inputs, if it did.
    pub fn tab_failure(&self, tab: DesignDock) -> Option<String> {
        let key = self.hydro_key();
        let failures: Vec<(&str, Option<&str>)> = match tab {
            DesignDock::Hydrostatics => vec![
                ("Hydrostatics", self.hydro.track.failure(key)),
                ("GZ curves", self.hydro.curves_track.failure(key)),
            ],
            DesignDock::Loading | DesignDock::Windage => vec![(
                "Loading compute",
                self.loading_track.failure(self.loading_key()),
            )],
            DesignDock::Damage => vec![(
                "Damage compute",
                self.damage_track.failure(self.damage_key()),
            )],
            DesignDock::Performance => vec![
                (
                    "Resistance compute",
                    self.resistance_track.failure(self.resistance_key()),
                ),
                (
                    "Seakeeping compute",
                    self.raos_track.failure(self.raos_key()),
                ),
            ],
        };
        failures
            .into_iter()
            .find_map(|(what, e)| e.map(|e| format!("{what} failed: {e}")))
    }

    /// Whether a hydrostatics job is running (test hook).
    pub fn hydro_busy(&self) -> bool {
        self.hydro.track.busy()
    }

    /// Whether a GZ-curves job is running (test hook).
    pub fn curves_busy(&self) -> bool {
        self.hydro.curves_track.busy()
    }

    /// Whether a resistance job is running (test hook).
    pub fn resistance_busy(&self) -> bool {
        self.resistance_track.busy()
    }

    /// Whether any analysis job is running (test hook).
    pub fn jobs_busy(&self) -> bool {
        self.hydro.track.busy()
            || self.hydro.curves_track.busy()
            || self.loading_track.busy()
            || self.damage_track.busy()
            || self.resistance_track.busy()
            || self.raos_track.busy()
    }

    /// Run `work` as the loading job, as a worker that fails would (test
    /// hook for the failure path: a panic must reach the user).
    #[doc(hidden)]
    pub fn start_loading_job_for_test(
        &mut self,
        work: impl FnOnce() -> Result<LoadingResult, String> + Send + 'static,
    ) {
        let key = self.loading_key();
        self.loading_track.start(key, work);
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
        self.loading_selected = 0;
        self.damage_selected = 0;
        self.perf_state.resistance = None;
        self.resistance_track.reset();
        self.perf_state.raos = None;
        self.perf_state.sea_result = None;
        self.raos_track.reset();
        self.project_changed();
    }

    /// Show or hide the loading panel; showing it shows only it in the design dock.
    /// Report options for the current model: defaults from the surface, the
    /// project's saved settings, then the viewports' subdivision.
    fn report_options(&self) -> Option<seacraft_core::report::ReportOptions> {
        let surface = self.surface.as_ref()?;
        let mut opts = self
            .project
            .report
            .apply(seacraft_core::report::ReportOptions::from_surface(surface));
        opts.level = self.level;
        opts.mode = self.mode;
        Some(opts)
    }

    /// Write the design report on this thread (tests and scripts).
    pub fn write_report_blocking(&mut self, path: &std::path::Path) -> Result<(), String> {
        let opts = self.report_options().ok_or("no model loaded")?;
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        let html =
            seacraft_core::report::design_report_with_project(surface, &opts, &self.project)?;
        seacraft_core::io::write_atomic(path, html)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        self.status = format!("wrote report {}", path.display());
        Ok(())
    }

    /// Start writing the design report on a worker thread; the status bar
    /// reports when it is done. Returns false when nothing could be started.
    fn start_report(&mut self, path: &std::path::Path) -> bool {
        let (Some(opts), Some(surface)) = (self.report_options(), self.surface.clone()) else {
            self.status = "No model loaded.".into();
            return false;
        };
        let project = self.project.clone();
        let path = path.to_path_buf();
        self.status = format!("writing report {}…", path.display());
        let target = path.clone();
        let job = Job::spawn(self.revision, move || {
            seacraft_core::report::design_report_with_project(&surface, &opts, &project).and_then(
                |html| {
                    seacraft_core::io::write_atomic(&target, html)
                        .map_err(|e| format!("cannot write {}: {e}", target.display()))
                },
            )
        });
        self.report_job = Some((path, job));
        true
    }

    /// Collect a finished report job, if any.
    fn poll_report(&mut self, ctx: &egui::Context) {
        let Some((path, job)) = &self.report_job else {
            return;
        };
        self.status = match job.poll() {
            crate::jobs::Poll::Running => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            crate::jobs::Poll::Done(()) => format!("wrote report {}", path.display()),
            crate::jobs::Poll::Failed(e) => format!("report failed: {e}"),
        };
        self.report_job = None;
    }

    /// The File > Report options window, editing the project's report settings.
    fn report_options_window(&mut self, ui: &mut egui::Ui) {
        if !self.show_report_options {
            return;
        }
        let conditions: Vec<String> = self
            .project
            .loading_conditions
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let cases: Vec<String> = self
            .project
            .damage_cases
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let mut open = true;
        let mut changed = false;
        egui::Window::new("Report options")
            .open(&mut open)
            .resizable(true)
            .default_width(420.0)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(560.0)
                    .show(ui, |ui| {
                        changed = panels::report_options(
                            ui,
                            &mut self.project.report,
                            &conditions,
                            &cases,
                        );
                    });
            });
        if changed {
            self.dirty = true;
        }
        self.show_report_options = open;
    }

    /// Is the report options window open?
    pub fn show_report_options(&self) -> bool {
        self.show_report_options
    }

    pub fn set_show_report_options(&mut self, show: bool) {
        self.show_report_options = show;
    }

    /// Show `tab` in the Analysis dock (and show the dock).
    pub fn set_dock_tab(&mut self, tab: DesignDock) {
        self.dock_tab = tab;
        self.show_analysis = true;
    }

    /// The Analysis dock's tab (also when the dock is hidden).
    pub fn dock_tab(&self) -> DesignDock {
        self.dock_tab
    }

    /// Show or hide the whole Analysis dock; its tab is kept.
    pub fn set_show_analysis(&mut self, show: bool) {
        self.show_analysis = show;
    }

    pub fn show_analysis(&self) -> bool {
        self.show_analysis
    }

    /// Widen the Analysis dock to about 55 % of the window (plots, tables).
    pub fn set_dock_wide(&mut self, wide: bool) {
        self.dock_wide = wide;
    }

    pub fn dock_wide(&self) -> bool {
        self.dock_wide
    }

    /// Showing a tab selects it and shows the dock; hiding the tab that is
    /// showing returns the dock to Hydrostatics. Hiding another tab is a no-op.
    fn set_show_tab(&mut self, tab: DesignDock, show: bool) {
        if show {
            self.set_dock_tab(tab);
        } else if self.dock_tab == tab {
            self.dock_tab = DesignDock::Hydrostatics;
        }
    }

    fn shows_tab(&self, tab: DesignDock) -> bool {
        self.show_analysis && self.dock_tab == tab
    }

    /// Show the Loading tab (or leave it, back to Hydrostatics).
    pub fn set_show_loading(&mut self, show: bool) {
        self.set_show_tab(DesignDock::Loading, show);
    }

    pub fn show_loading(&self) -> bool {
        self.shows_tab(DesignDock::Loading)
    }

    /// Show the Damage tab (or leave it, back to Hydrostatics).
    pub fn set_show_damage(&mut self, show: bool) {
        self.set_show_tab(DesignDock::Damage, show);
    }

    pub fn show_damage(&self) -> bool {
        self.shows_tab(DesignDock::Damage)
    }

    /// Show the Windage tab (or leave it, back to Hydrostatics).
    pub fn set_show_windage(&mut self, show: bool) {
        self.set_show_tab(DesignDock::Windage, show);
    }

    pub fn show_windage(&self) -> bool {
        self.shows_tab(DesignDock::Windage)
    }

    /// Show the Performance tab (or leave it, back to Hydrostatics).
    pub fn set_show_performance(&mut self, show: bool) {
        self.set_show_tab(DesignDock::Performance, show);
    }

    pub fn show_performance(&self) -> bool {
        self.shows_tab(DesignDock::Performance)
    }

    /// The tab the Analysis dock shows, `None` while the dock is hidden.
    pub fn design_dock(&self) -> Option<DesignDock> {
        self.show_analysis.then_some(self.dock_tab)
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
                let liquid_z = if self.loading_current() {
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

    /// Whether a loading-condition job is still running.
    pub fn loading_busy(&self) -> bool {
        self.loading_track.busy()
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
        let key = self.loading_key();
        if let Err(e) = seacraft_core::loading::validate(&with_windage) {
            let msg = format!("Loading condition invalid: {e}");
            self.status = msg.clone();
            // Remembered, so the automatic recompute does not retry it.
            self.loading_track.fail(key, msg);
            return;
        }
        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;
        self.loading_track.start(key, move || {
            Ok(seacraft_core::loading::evaluate(
                &surface,
                &opts,
                &with_windage,
            ))
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
        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        let res = seacraft_core::loading::evaluate(surface, &opts, &with_windage);
        self.loading_result = Some(res);
        // Supersedes any job: the result shown is this one.
        self.loading_track.stamp(self.loading_key());
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

        let key = self.damage_key();
        self.damage_track.start(key, move || {
            let resolved = project.resolve_damage_case(&raw_case)?;
            seacraft_core::damage::validate(&resolved)?;
            seacraft_core::damage::evaluate(&surface, &opts, &resolved)
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
        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        let res = seacraft_core::damage::evaluate(surface, &opts, &case)?;
        self.damage_result = Some(res);
        self.damage_track.stamp(self.damage_key());
        Ok(self.damage_result.as_ref().unwrap())
    }

    pub fn performance_state(&self) -> &PerformanceState {
        &self.perf_state
    }

    /// The Performance inputs and results, edited in place (tests): the
    /// results become out of date.
    pub fn performance_state_mut(&mut self) -> &mut PerformanceState {
        self.performance_changed();
        &mut self.perf_state
    }

    pub fn performance_speeds(&self) -> Vec<f64> {
        let (min, max, step) = self.perf_state.speeds_kn;
        let (min, max) = if min <= max { (min, max) } else { (max, min) };
        let step = if step > 0.05 { step } else { 1.0 };
        let mut speeds = Vec::new();
        let mut s = min;
        while s <= max + 1e-6 {
            speeds.push(s);
            s += step;
        }
        if speeds.is_empty() {
            speeds.push(min);
        }
        speeds
    }

    pub fn effective_performance_kg(&self) -> f64 {
        self.perf_state
            .kg
            .unwrap_or_else(|| self.effective_performance_kg_default())
    }

    /// The Performance KG without a manual value: the current loading
    /// condition's fluid KG, else the hydro dock's.
    fn effective_performance_kg_default(&self) -> f64 {
        match &self.loading_result {
            Some(r) if self.loading_current() => r.kg_fluid,
            _ => self.hydro.kg,
        }
    }

    pub fn performance_kg_source(&self) -> &'static str {
        if self.perf_state.kg.is_some() {
            "Manual"
        } else if self.loading_current() {
            "Loading (KG_fluid)"
        } else {
            "Hydro dock"
        }
    }

    pub fn seakeeping_spec(&self) -> SeakeepingSpec {
        let draft = self.hydro.options.draft;
        let kg = self.effective_performance_kg();
        let kyy = if (self.perf_state.kyy_frac - 0.25).abs() < 1e-6 {
            None
        } else {
            let lwl = self
                .surface
                .as_ref()
                .map(|s| {
                    let hull = Hull::new(s, seacraft_core::SubdivisionOptions::default());
                    let mut opts =
                        HydroOptions::from_surface(s, seacraft_core::SubdivisionOptions::default());
                    opts.draft = draft;
                    hull.hydrostatics(&opts).lwl
                })
                .unwrap_or(0.0);
            Some(self.perf_state.kyy_frac * lwl)
        };
        SeakeepingSpec {
            draft,
            speed_kn: self.perf_state.sk_speed_kn,
            kg,
            kyy,
            points: Some(self.perf_state.points),
            ..Default::default()
        }
    }

    pub fn compute_resistance_for(
        &self,
        method: ResistanceMethod,
        speeds: &[f64],
        opts: &HydroOptions,
    ) -> Result<ResistanceResult, String> {
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| "no model loaded".to_string())?;
        compute_resistance_impl(surface, method, speeds, opts)
    }

    pub fn compute_resistance_blocking(&mut self) -> Result<&ResistanceResult, String> {
        let speeds = self.performance_speeds();
        let method = self.perf_state.method;
        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;
        let res = self.compute_resistance_for(method, &speeds, &opts)?;
        self.perf_state.resistance = Some(res);
        self.resistance_track.stamp(self.resistance_key());
        Ok(self.perf_state.resistance.as_ref().unwrap())
    }

    pub fn compute_raos_blocking(&mut self) -> Result<&SeakeepingReport, String> {
        let surface = self
            .surface
            .as_ref()
            .ok_or_else(|| "no model loaded".to_string())?;
        let spec = self.seakeeping_spec();
        let sea = self.sea_for_job();
        let (rep, irr) = compute_raos_with_sea(surface, &spec, sea.as_ref())?;
        self.perf_state.raos = Some(rep);
        self.perf_state.sea_result = irr;
        self.raos_track.stamp(self.raos_key());
        Ok(self.perf_state.raos.as_ref().unwrap())
    }

    /// The sea-state input when the section is switched on.
    fn sea_for_job(&self) -> Option<SeaStateInput> {
        self.perf_state
            .sea_input
            .enabled
            .then(|| self.perf_state.sea_input.clone())
    }

    pub fn start_resistance_compute(&mut self) {
        let surface = match &self.surface {
            Some(s) => s.clone(),
            None => return,
        };
        let speeds = self.performance_speeds();
        let method = self.perf_state.method;
        let mut opts = self.hydro.options.clone();
        opts.subdivision.level = self.level;
        opts.subdivision.mode = self.mode;

        let key = self.resistance_key();
        self.resistance_track.start(key, move || {
            compute_resistance_impl(&surface, method, &speeds, &opts)
        });
    }

    pub fn start_raos_compute(&mut self) {
        let surface = match &self.surface {
            Some(s) => s.clone(),
            None => return,
        };
        let spec = self.seakeeping_spec();
        let sea = self.sea_for_job();

        let key = self.raos_key();
        self.raos_track.start(key, move || {
            compute_raos_with_sea(&surface, &spec, sea.as_ref())
        });
    }

    pub fn start_performance_compute(&mut self) {
        self.start_resistance_compute();
        self.start_raos_compute();
    }

    pub fn resistance_result(&self) -> Option<&ResistanceResult> {
        self.perf_state.resistance.as_ref()
    }

    pub fn rao_result(&self) -> Option<&SeakeepingReport> {
        self.perf_state.raos.as_ref()
    }

    pub fn set_performance_results(&mut self, res: ResistanceResult, raos: SeakeepingReport) {
        self.perf_state.resistance = Some(res);
        self.resistance_track.stamp(self.resistance_key());
        self.perf_state.raos = Some(raos);
        self.perf_state.sea_result = None;
        self.raos_track.stamp(self.raos_key());
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
    /// The project edited in place, as the dock panels do (tests).
    #[doc(hidden)]
    pub fn project_mut_for_test(&mut self) -> &mut Project {
        self.dirty = true;
        self.project_changed();
        &mut self.project
    }

    pub fn surface_mut(&mut self) -> Option<&mut Surface> {
        self.surface.as_mut()
    }

    /// Recompute `dirty` against the last loaded or saved state: unsaved
    /// unless both the model and the project are equal to it.
    fn refresh_dirty(&mut self) {
        self.dirty = match (&self.saved_state, &self.surface) {
            (Some((fef_text, project)), Some(surface)) => {
                *project != self.project || fef::write(surface) != *fef_text
            }
            // Back on the start screen: nothing to save.
            (_, None) => false,
            _ => true,
        };
    }

    pub fn mark_geometry_changed(&mut self) {
        self.revision += 1;
        self.dirty = true;
        self.touch_geometry();
    }

    pub fn hydrostatics(&self) -> Option<&Hydrostatics> {
        self.hydro.result.as_ref()
    }

    /// Recompute now on this thread, without waiting for the debounce or a
    /// job (tests and the control channel). A running job is superseded.
    pub fn recompute_hydrostatics(&mut self) {
        let Some(surface) = &self.surface else {
            return;
        };
        let out = measure_hydro(&self.hydro.inputs(surface, self.level, self.mode));
        self.install_hydro(out);
        self.hydro.track.stamp(self.hydro_key());
    }

    /// The floating condition, edited in place: the hydrostatics become out
    /// of date.
    pub fn hydro_options_mut(&mut self) -> &mut HydroOptions {
        self.hydro_inputs_changed();
        &mut self.hydro.options
    }

    pub fn set_kg(&mut self, kg: f64) {
        self.hydro.kg = kg;
        self.hydro_inputs_changed();
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

    pub fn set_subdivision_mode(&mut self, mode: SubdivisionMode) {
        self.mode = mode;
        self.revision += 1;
        self.touch_geometry();
    }

    /// Solver setting for hydrostatics and GZ curves: free to trim about `lcg`
    /// (or hold the trim when `free` is false). `None` leaves the LCG to be
    /// filled from the upright LCB on the next solve.
    pub fn set_free_trim(&mut self, free: bool, lcg: Option<f64>) {
        self.hydro.free_trim = free;
        self.hydro.lcg = lcg;
        self.hydro_inputs_changed();
    }

    pub fn free_trim(&self) -> bool {
        self.hydro.free_trim
    }

    /// The GZ curves as last computed, if any.
    pub fn cross_curves(&self) -> Option<&[CrossCurve]> {
        self.hydro.curves.as_deref()
    }

    /// Rebuild the GZ curves now, on this thread, instead of waiting for
    /// the dock button's job (tests and the control channel).
    pub fn compute_cross_curves(&mut self) {
        let Some(inputs) = self.curves_inputs() else {
            return;
        };
        let key = self.hydro_key();
        let out = compute_cross_curves_for(inputs);
        if out.curves.is_none() {
            // Nothing floats: as before, the curves are left alone.
            if let Some(h) = out.hydro {
                self.install_hydro(h);
                self.hydro.track.stamp(key);
            }
            return;
        }
        self.install_curves(out, key);
        self.hydro.curves_track.stamp(key);
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
        self.analysis_params.remember(mode);
        self.analysis_frames = [None, None, None, None];
    }

    /// The parameter each analysis mode uses when it is chosen.
    pub fn analysis_params(&self) -> panels::AnalysisParams {
        self.analysis_params
    }

    /// The key of the analysis frame cached for `view`, if one was rendered
    /// (see [`render::frame_key`]).
    pub fn analysis_frame_key(&self, view: usize) -> Option<u64> {
        self.analysis_frames[view].as_ref().map(|f| f.key)
    }

    /// How many analysis frames each view has rasterized so far (test hook).
    pub fn raster_counts(&self) -> [u64; 4] {
        self.raster_counts
    }

    pub fn subdivision_level(&self) -> u8 {
        self.level
    }

    pub fn subdivision_mode(&self) -> SubdivisionMode {
        self.mode
    }

    /// Make `view` the active one. A maximised view follows it.
    pub fn set_active_view(&mut self, view: usize) {
        self.active = view.min(3);
        if self.maximised.is_some() {
            self.maximised = Some(self.active);
        }
    }

    /// The view filling the whole central area, if one is maximised.
    pub fn maximised(&self) -> Option<usize> {
        self.maximised
    }

    /// Maximise the active view (`true`) or restore the four views. The view
    /// that grows or shrinks back is refitted to its new cell, as F does.
    pub fn set_maximised(&mut self, maximised: bool) {
        let before = self.maximised;
        self.maximised = maximised.then_some(self.active);
        if self.maximised != before {
            if let Some(view) = self.maximised.or(before) {
                self.fit_view(view);
            }
        }
    }

    /// Frame the model in view `index` alone.
    fn fit_view(&mut self, index: usize) {
        if let Some(s) = &self.surface {
            let (min, max) = crate::render::display_extents(s);
            self.cameras[index].frame(min, max);
        }
    }

    /// Toolbar button, Ctrl+Space and a double-click on a view's title.
    pub fn toggle_maximised(&mut self) {
        self.set_maximised(self.maximised.is_none());
    }

    /// The whole central area the viewports share (last frame).
    pub fn central_rect(&self) -> egui::Rect {
        self.central_rect
    }

    /// View `i`'s title caption (last frame); `Rect::NOTHING` when hidden.
    pub fn title_rect(&self, i: usize) -> egui::Rect {
        self.title_rects[i]
    }

    /// Where the toolbar control `name` was drawn last frame (see
    /// [`panels::ToolbarOutput::rects`]).
    pub fn toolbar_rect(&self, name: &str) -> Option<egui::Rect> {
        self.toolbar_rects
            .iter()
            .find(|(n, _)| *n == name)
            .map(|(_, r)| *r)
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
            let snap = Snapshot::edit(surface, self.hydro.options.draft);
            self.history.snapshot(&snap);
        }
    }

    /// Move control point `index` to `target` as one undoable edit: the
    /// shared path of the Inspector and the control channel's `point.move`.
    /// A point on a symmetric layer is clamped to `y >= 0` (the mirror
    /// supplies the other half). Returns the position the point took and
    /// whether it sits on the centreline of a symmetric layer.
    pub fn move_point(&mut self, index: usize, target: Vec3) -> Result<(Vec3, bool), String> {
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        if index >= surface.points.len() {
            return Err(format!(
                "`index` must be from 0 to {}",
                surface.points.len().saturating_sub(1)
            ));
        }
        if !target.is_finite() {
            return Err("the new position must be finite".into());
        }
        let symmetric = point_on_symmetric_layer(surface, index);
        let target = if symmetric && target.y < 0.0 {
            Vec3::new(target.x, 0.0, target.z)
        } else {
            target
        };
        self.snapshot_undo();
        if let Some(surface) = self.surface.as_mut() {
            surface.points[index].p = target;
        }
        self.select_point(Some(index));
        self.mark_geometry_changed();
        self.status_note(format!("control point {index} moved"));
        Ok((target, symmetric && target.y == 0.0))
    }

    /// Apply what the Inspector asked for. A value equal to the point's
    /// position changes nothing and records no undo step.
    pub fn apply_inspector_action(&mut self, action: panels::InspectorAction) {
        let panels::InspectorAction::Move { index, to } = action else {
            return;
        };
        let unchanged = self
            .surface
            .as_ref()
            .and_then(|s| s.points.get(index))
            .is_some_and(|p| p.p == to);
        if unchanged {
            return;
        }
        if let Err(e) = self.move_point(index, to) {
            self.status = e;
        }
    }

    /// Where the Inspector's x (0), y (1) or z (2) field was drawn last
    /// frame; `Rect::NOTHING` with no point selected (test hook).
    pub fn inspector_field_rect(&self, axis: usize) -> egui::Rect {
        self.inspector_rects[axis]
    }

    /// Where the left dock's layer list was drawn last frame (test hook).
    pub fn layer_list_rect(&self) -> egui::Rect {
        self.layer_list_rect
    }

    /// Where the Inspector was drawn last frame, under the layer list
    /// (test hook).
    pub fn inspector_rect(&self) -> egui::Rect {
        self.inspector_rect
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

fn compute_resistance_impl(
    surface: &Surface,
    method: ResistanceMethod,
    speeds: &[f64],
    opts: &HydroOptions,
) -> Result<ResistanceResult, String> {
    let hull = Hull::new(surface, opts.subdivision);
    match method {
        ResistanceMethod::Holtrop => {
            let input = HoltropInput::measure(&hull, opts)
                .ok_or_else(|| "cannot measure the hull for Holtrop".to_string())?;
            let core_rows = resistance::curve(&input, speeds);
            let warnings = input.warnings();
            let rows = core_rows
                .into_iter()
                .map(|r| ResistanceRow {
                    speed_kn: r.speed_kn,
                    fn_: r.fn_,
                    rt_kn: r.rt_n / 1000.0,
                    pe_kw: Some(r.pe_w / 1000.0),
                    note: None,
                })
                .collect();
            Ok(ResistanceResult {
                method: "Holtrop & Mennen".to_string(),
                rows,
                warnings,
                rt_label: "R_T (kN)".to_string(),
            })
        }
        ResistanceMethod::Dsyhs => {
            let dsyhs_in = resistance::DsyhsInput::measure(&hull, opts)
                .ok_or_else(|| "cannot measure the hull for DSYHS".to_string())?;
            let mut warnings = dsyhs_in.warnings();
            let core_rows = resistance::dsyhs_curve(&dsyhs_in, speeds);
            for r in &core_rows {
                if !(0.15..=0.75).contains(&r.fn_) {
                    warnings.push(format!(
                        "Fn {:.3} at {:.2} kn is outside DSYHS valid range (0.15-0.75, clamped)",
                        r.fn_, r.speed_kn
                    ));
                }
            }
            let rows = core_rows
                .into_iter()
                .map(|r| ResistanceRow {
                    speed_kn: r.speed_kn,
                    fn_: r.fn_,
                    rt_kn: r.rt_n / 1000.0,
                    pe_kw: Some(r.pe_w / 1000.0),
                    note: None,
                })
                .collect();
            Ok(ResistanceResult {
                method: "Delft (DSYHS)".to_string(),
                rows,
                warnings,
                rt_label: "R_T (kN)".to_string(),
            })
        }
        ResistanceMethod::Savitsky => {
            let setup =
                seacraft_core::planing::setup(surface, opts.subdivision, opts, None, None, None)
                    .map_err(|e| format!("planing setup: {e}"))?;
            let core_rows = seacraft_core::planing::planing_curve(&setup.hull, speeds);
            let mut warnings = Vec::new();
            if core_rows.is_empty() {
                warnings.push("No planing equilibrium found at these speeds".to_string());
            }
            let rows = core_rows
                .into_iter()
                .map(|p| ResistanceRow {
                    speed_kn: p.speed_kn,
                    fn_: p.cv,
                    rt_kn: p.resistance_n / 1000.0,
                    pe_kw: Some(p.pe_w / 1000.0),
                    note: Some(format!("trim {:.1}°", p.trim_deg)),
                })
                .collect();
            Ok(ResistanceResult {
                method: "Savitsky (Planing)".to_string(),
                rows,
                warnings,
                rt_label: "R_T (kN)".to_string(),
            })
        }
        ResistanceMethod::Michell => {
            let grid = seacraft_core::michell::michell_grid_from_surface(
                surface,
                opts.draft,
                65,
                25,
                opts.subdivision,
            )
            .ok_or_else(|| "cannot extract Michell offsets grid from hull surface".to_string())?;
            let holtrop_in = resistance::HoltropInput::measure(&hull, opts);
            let form_factor = if let Some(ref h_in) = holtrop_in {
                let r = resistance::holtrop(h_in, 1.0);
                if h_in.warnings().is_empty() {
                    r.form_factor
                } else {
                    1.0
                }
            } else {
                1.0
            };
            let viscosity = if (opts.water_density - 1000.0).abs() < 25.0 {
                resistance::VISCOSITY_FRESH_WATER
            } else {
                resistance::VISCOSITY_SEA_WATER
            };
            let mopts = seacraft_core::michell::MichellOptions {
                nx: 65,
                nz: 25,
                ntheta: 128,
                catamaran_spacing: None,
            };
            let core_rows = seacraft_core::michell::michell_resistance_curve(
                &grid,
                form_factor,
                viscosity,
                opts.water_density,
                resistance::GRAVITY,
                speeds,
                &mopts,
            );
            let mut warnings = Vec::new();
            let lb = if grid.beam_wl > 1e-6 {
                grid.length_wl / grid.beam_wl
            } else {
                0.0
            };
            if lb < 6.0 {
                warnings.push(format!(
                    "L/B {lb:.2} is outside thin-ship recommended range (L/B >= 6.0)"
                ));
            }
            for r in &core_rows {
                if !(0.20..=1.0).contains(&r.fn_) {
                    warnings.push(format!(
                        "Fn {:.3} at {:.2} kn is outside typical Michell applicability range (0.20-1.0)",
                        r.fn_, r.speed_kn
                    ));
                }
            }
            let rows = core_rows
                .into_iter()
                .map(|r| ResistanceRow {
                    speed_kn: r.speed_kn,
                    fn_: r.fn_,
                    rt_kn: r.rt_n / 1000.0,
                    pe_kw: Some(r.pe_w / 1000.0),
                    note: Some(format!("Rw {:.2} kN", r.rw_n / 1000.0)),
                })
                .collect();
            Ok(ResistanceResult {
                method: "Michell (Thin ship)".to_string(),
                rows,
                warnings,
                rt_label: "R_T (kN)".to_string(),
            })
        }
    }
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
