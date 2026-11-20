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

use crate::builder_panels;
use crate::camera::{Camera, ViewKind};
use crate::gpu_hull::geometry::{build_geometry, HullGeometry};
use crate::gpu_hull::{self, HullCallback};
use crate::history::History;
use crate::jobs::{revision_key, Job, Track};
use crate::messages::{Level, MessageLog, MessagesAction};
use crate::panels;
use crate::picking::{self, PICK_RADIUS};
use crate::recent::{RecentAction, RecentFiles};
use crate::render::{self, DisplayOptions, Scene};

/// The built-in demo hull's KG above the keel (m): part of the demo, like
/// its reference numbers. No other model gets a KG it was not given. The
/// committed `examples/demo-hull.seacraft.json` carries the same value.
pub const DEMO_KG: f64 = seacraft_core::demo::DEMO_KG;

/// What a KG-dependent result says while KG is unset.
pub const KG_UNSET_HINT: &str = "set KG first";

/// Undo steps kept.
const UNDO_LIMIT: usize = 64;

/// Cached refined mesh, invalidated by the revision counter, level or mode.
struct RenderCache {
    revision: u64,
    level: u8,
    mode: SubdivisionMode,
    mesh: RenderMesh,
    /// The ray index over `mesh`, built on first use by a view that
    /// classifies the hidden net (see `App::net_visibility_wanted`).
    bvh: Option<seacraft_core::raycast::Bvh>,
    net_lines: seacraft_core::netlines::NetLines,
    curves: Option<seacraft_core::linesplan::LinesplanCurves3d>,
    /// The section settings `curves` were cut with ([`SectionCut`]).
    curves_for: Option<SectionCut>,
    /// The curves were carried over from before a drag began: they are
    /// rebuilt on release, not on every drag frame.
    curves_stale: bool,
    /// The waterline cut from `mesh` and the plane it was cut with. It goes
    /// with the mesh, and only a different plane cuts it again: the views
    /// redraw it from here while the camera moves.
    waterline: Option<(seacraft_core::math::Plane, Vec<[Vec3; 2]>)>,
    min: Vec3,
    max: Vec3,
}

/// What the viewport section curves are cut with: the station, buttock and
/// waterline counts, whether combs are sampled, and the comb gain (bits).
/// The comb density only thins the drawing, so it is not part of the cut.
type SectionCut = (usize, usize, usize, bool, u64);

fn section_cut(d: &DisplayOptions) -> SectionCut {
    (
        d.stations,
        d.buttocks,
        d.waterlines,
        d.combs,
        d.comb_scale.to_bits(),
    )
}

/// Whether two waterplanes are exactly the same plane.
fn same_plane(a: &seacraft_core::math::Plane, b: &seacraft_core::math::Plane) -> bool {
    a.n == b.n && a.d == b.d
}

impl RenderCache {
    /// Identifies the refined mesh: the revision it was built from and the
    /// level (the drag preview refines less without a new revision).
    fn stamp(&self) -> u64 {
        (self.revision << 4) | u64::from(self.level)
    }
}

/// Hash of a [`crate::camera::ViewTransform`]'s basis and projection parameters.
fn view_key(view: &crate::camera::ViewTransform) -> u64 {
    revision_key(&[
        view.eye.x.to_bits(),
        view.eye.y.to_bits(),
        view.eye.z.to_bits(),
        view.right.x.to_bits(),
        view.right.y.to_bits(),
        view.right.z.to_bits(),
        view.up.x.to_bits(),
        view.up.y.to_bits(),
        view.up.z.to_bits(),
        view.forward.x.to_bits(),
        view.forward.y.to_bits(),
        view.forward.z.to_bits(),
        view.ortho_height.to_bits(),
        view.fit_aspect.to_bits(),
        view.fov_tan.to_bits(),
        view.perspective as u64,
    ])
}

/// Live hydrostatics: the floating condition, the last report and the hull
/// cache they were measured from.
struct HydroState {
    options: HydroOptions,
    /// KG above the keel for GM and the GZ cross curves. `None` until the
    /// user sets it (or the design requirements carry an estimate): no
    /// model is given a KG it does not have.
    kg: Option<f64>,
    /// `kg` is the design requirements' estimate, not a value the user set
    /// (the dock marks it "est.").
    kg_estimate: bool,
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
            kg: None,
            kg_estimate: false,
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

/// The app's side of the GPU hull; the wgpu resources live in egui's renderer.
struct GpuHullState {
    /// Whether views may emit a [`HullCallback`].
    routing: bool,
    /// The widest texture side the device takes; a larger cell takes the painted path.
    max_texture: u32,
    /// What every view's callback draws this frame.
    geometry: Option<Arc<HullGeometry>>,
    /// The mesh `geometry` was built from, whatever its wireframe setting
    /// ([`render::geometry_key`] without edges).
    mesh_key: u64,
    /// The mesh the GPU last failed on. While it stands the painted path draws that mesh, and every
    /// mesh during the drag in progress; a new mesh outside a drag clears it and tries the GPU again.
    failed_mesh: Option<u64>,
    /// The failure messages logged this session, each once.
    logged: std::collections::HashSet<String>,
    failure: gpu_hull::FailureSlot,
}

impl Default for GpuHullState {
    fn default() -> Self {
        Self {
            routing: false,
            max_texture: u32::MAX,
            geometry: None,
            mesh_key: 0,
            failed_mesh: None,
            logged: Default::default(),
            failure: Default::default(),
        }
    }
}

/// How one view draws its hull this frame.
enum HullRoute {
    Painted,
    Gpu(egui::PaintCallback),
    /// Routed, with nothing to draw: an empty mesh, or a cell under a pixel.
    Nothing,
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

/// Connected-editing switch, chosen preset name and optional reach override.
#[derive(Debug, Clone, PartialEq)]
pub struct ConnectedState {
    pub on: bool,
    pub preset: String,
    pub reach_m: Option<f64>,
}

impl Default for ConnectedState {
    fn default() -> Self {
        Self {
            on: false,
            preset: seacraft_core::connected::DEFAULT_PRESET.to_owned(),
            reach_m: None,
        }
    }
}

#[derive(Clone, PartialEq)]
struct HoverFollowersKey {
    hovered: usize,
    revision: u64,
    preset: seacraft_core::connected::ConnectedPreset,
    reach_bits: u64,
    seeds: Vec<usize>,
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
    /// Which points move with the drag, and where each started.
    plan: seacraft_core::netmove::MovePlan,
    /// Whether Shift+press added `point` to the selection (so a release
    /// without movement does not immediately toggle it off again).
    added_on_press: bool,
    /// Last cursor target computed by `drag_to` (or `original` before the
    /// first movement), so changing the reach mid-drag can re-apply the move.
    target: Vec3,
    /// Whether `plan` pulls neighbours along ([`App::connected_now`] when it
    /// was last built): Ctrl pressed or let go mid-drag re-plans it.
    connected: bool,
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
    /// The table of offsets as a printable PDF, in millimetres.
    OffsetsPdf,
}

impl ExportKind {
    pub const ALL: [ExportKind; 10] = [
        ExportKind::Obj,
        ExportKind::Stl,
        ExportKind::ThreeMf,
        ExportKind::Ply,
        ExportKind::HullIges,
        ExportKind::Html,
        ExportKind::Linesplan,
        ExportKind::LinesplanDxf,
        ExportKind::LinesplanIges,
        ExportKind::OffsetsPdf,
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
            ExportKind::OffsetsPdf => "Offsets table PDF…",
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
            ExportKind::OffsetsPdf => "pdf",
        }
    }

    /// Whether this is a linesplan drawing (cut with section options), not a
    /// hull mesh or surface.
    pub fn is_linesplan(self) -> bool {
        matches!(
            self,
            ExportKind::Linesplan | ExportKind::LinesplanDxf | ExportKind::LinesplanIges
        )
    }
}

/// Clipped volume meshes kept at once: more volumes than this would rebuild
/// every frame, and a drag of a volume's bounds adds one per step.
const MAX_CLIPPED_VOLUMES: usize = 64;

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
    /// A dock, Report options or Project settings step that edited the
    /// project: the project and the dock KG go back with it.
    project: Option<(Project, (Option<f64>, bool))>,
}

impl Snapshot {
    /// An edit of the model in place.
    fn edit(surface: &Surface, draft: f64) -> Self {
        Self {
            surface: Some(surface.clone()),
            draft,
            model: None,
            project: None,
        }
    }

    /// An edit of the project in place.
    fn project_edit(
        surface: &Surface,
        draft: f64,
        project: Project,
        kg: (Option<f64>, bool),
    ) -> Self {
        Self {
            surface: Some(surface.clone()),
            draft,
            model: None,
            project: Some((project, kg)),
        }
    }
}

/// What travels with a model besides its geometry.
#[derive(Debug, Clone)]
struct ModelState {
    project: Project,
    /// The dock KG and whether it is an estimate.
    kg: (Option<f64>, bool),
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
    /// The builder templates, in the format the path's extension names.
    Templates,
    /// The panel patterns, in the format the path's extension names.
    Panels,
    /// The current wave pattern, as PNG, SVG or VTI by the path's extension.
    WaveImage,
}

/// An action that may need to discard unsaved work first.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PendingAction {
    NewDemo,
    /// The New design dialog (it replaces the model and the project).
    NewDesign,
    /// The New design dialog on *Example*.
    NewExample,
    Open,
    /// File > Open recent (the path waits in `App::pending_recent`).
    OpenRecent,
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
    /// The far-field wave pattern's settings, and its last result (drawn only while current:
    /// [`App::wave_current`]).
    pub waves: crate::waves::WaveSettings,
    pub wave_result: Option<crate::waves::WaveResult>,
    /// The wave-drag curve over speed (drawn only while current: [`App::wave_drag`]).
    pub wave_drag: Option<crate::waves::WaveDrag>,
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
            waves: crate::waves::WaveSettings::default(),
            wave_result: None,
            wave_drag: None,
        }
    }
}

/// What the RAO job hands back: the RAOs and, with a sea state, the
/// irregular-sea responses computed from them.
type RaoResult = (SeakeepingReport, Option<IrregularReport>);

/// Why a model could not be opened: the open dialog and the control
/// channel decide on this, never on the status text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum OpenError {
    /// The file could not be read (missing, unreadable, not text).
    Read { path: PathBuf, reason: String },
    /// The file was read but is not a valid `.fef`.
    Parse { path: PathBuf, reason: String },
}

impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenError::Read { path, reason } => {
                write!(f, "Cannot read {}: {reason}", path.display())
            }
            OpenError::Parse { path, reason } => {
                write!(f, "Parse error in {}: {reason}", path.display())
            }
        }
    }
}

impl std::error::Error for OpenError {}

impl From<OpenError> for String {
    fn from(err: OpenError) -> Self {
        err.to_string()
    }
}

/// What the design report's wave section takes from the window: the view's settings, and the
/// waterline length and beam at the report's draft when a current result knows them.
struct ReportWaves {
    settings: crate::waves::WaveSettings,
    lengths: Option<(f64, f64)>,
}

impl ReportWaves {
    /// The section's inputs. Lengths not known are measured on `surface` at the report's draft
    /// (slow: call it off the window thread), else taken from the main particulars; with none,
    /// the section goes in on the default domain in waterline lengths and without a wash line.
    fn into_report(
        self,
        surface: &Surface,
        opts: &seacraft_core::report::ReportOptions,
    ) -> seacraft_core::report::WaveReport {
        let w = &self.settings;
        let subdivision = seacraft_core::SubdivisionOptions {
            level: opts.level,
            mode: opts.mode,
        };
        let lengths = self
            .lengths
            .or_else(|| {
                crate::waves::michell_grid(surface, subdivision, opts.draft)
                    .ok()
                    .map(|g| (g.length_wl, g.beam_wl))
            })
            .or(Some((surface.settings.length, surface.settings.beam)))
            .filter(|(l, b)| *l > 0.0 && *b >= 0.0);
        let (speed_m_s, froude) = match w.speed {
            crate::waves::WaveSpeed::Knots(kn) => (kn * seacraft_core::resistance::KNOT, None),
            crate::waves::WaveSpeed::Froude(f) => (1.0, Some(f)),
        };
        let (wash, (ahead_l, behind_l, half_width_l)) = match lengths {
            Some((l, b)) => {
                let wash = crate::waves::wash_distance(w, l, b);
                (Some(wash), crate::waves::domain_l(w, l, wash))
            }
            None => (None, (0.5, 3.0, 1.5)),
        };
        seacraft_core::report::WaveReport {
            options: seacraft_core::wave_pattern::WavePatternOptions {
                ahead_l,
                behind_l,
                half_width_l,
                catamaran_spacing: w.cat_spacing_m,
                ..seacraft_core::wave_pattern::WavePatternOptions::at_speed(speed_m_s)
            },
            froude,
            clip_m: w.clip_m,
            wash_distance_m: wash,
        }
    }
}

pub struct App {
    surface: Option<Surface>,
    path: Option<PathBuf>,
    /// Design data kept in the `<model>.seacraft.json` sidecar.
    project: Project,
    /// Why the sidecar could not be read; while set, saving leaves it alone.
    project_error: Option<String>,
    status: String,
    /// How serious the status line is: a logged warning or error keeps its
    /// colour until the next status replaces it.
    status_level: Level,
    /// Errors and warnings from file actions and jobs, with their times
    /// (View > Messages, or a click on the status text).
    messages: MessageLog,
    /// The Messages window is open.
    show_messages: bool,
    /// Where the status text was drawn last frame (test hook).
    status_rect: egui::Rect,
    /// File > Open recent, most recent first.
    recent: RecentFiles,
    /// Where the recent list is kept between runs; `None` for headless apps,
    /// which must never touch the user's list.
    storage: Option<Box<dyn eframe::Storage>>,
    /// The storage's last flush failure, already logged.
    recent_failure: Option<String>,
    /// The recent file an Open recent waiting on the unsaved prompt opens.
    pending_recent: Option<PathBuf>,
    /// Bumped by every edit; keys all derived state. Only through [`App::bump_revision`].
    revision: u64,
    /// `revision`, readable from a worker thread ([`App::revision_watch`]).
    revision_watch: std::sync::Arc<std::sync::atomic::AtomicU64>,
    level: u8,
    mode: SubdivisionMode,
    display: DisplayOptions,
    cameras: [Camera; 4],
    active: usize,
    cache: Option<RenderCache>,
    selection: crate::selection::Selection,
    hovered_point: Option<usize>,
    selected_volume: Option<VolumeRef>,
    selected_windage: Option<usize>,
    last_highlighted_tank: Option<usize>,
    last_highlighted_compartment: Option<usize>,
    history: History<Snapshot>,
    /// The widget whose continuous project edit (a drag value or a focused
    /// text field) already has an undo snapshot on the stack.
    project_edit_source: Option<egui::Id>,
    marks: Vec<(u64, Option<Surface>, Project, usize)>,
    next_mark_id: u64,
    drag: Option<DragState>,
    /// Active Shift+drag marquee selection rectangle: `(viewport, origin, end)`.
    marquee: Option<(usize, egui::Pos2, egui::Pos2)>,
    /// Set when Escape cancels a marquee so moving before release does not pan.
    marquee_cancelled: bool,
    /// Unsaved edits since the last save / load.
    dirty: bool,
    /// The model (as `.fef` text) and project as last loaded or saved, so an
    /// undo back to that state clears `dirty`. `None` for a model never saved.
    saved_state: Option<(String, Project)>,
    dialog: Option<FileDialogState>,
    /// The About window (Help menu) is showing.
    about_open: bool,
    pending: Option<PendingAction>,
    /// A quit that has been confirmed (the window closes at the end of the frame).
    close_requested: bool,
    /// A widget held the keyboard when the last frame ended. egui drops the
    /// focus on Escape before the next frame's widgets run, so this is what
    /// tells an Escape that leaves a text field from one that clears the
    /// selection.
    keyboard_taken: bool,
    show_layers: bool,
    /// The active layer (index): the one the layer panel edits, as in
    /// FREE!ship's layer box. Clamped to the model's layers when read.
    active_layer: usize,
    /// The control face under the last primary click in a viewport, for
    /// the layer panel's "Move picked face to this layer".
    picked_face: Option<usize>,
    /// The layer panel's typing and dragging state.
    layer_panel: panels::LayerPanelState,
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
    /// Where the inspector's "Move" button was drawn last frame (test hook).
    inspector_move_button_rect: egui::Rect,
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
    /// A wave-pattern image being encoded and written.
    wave_image_job: Option<(PathBuf, Job<()>)>,
    /// The File > Builder templates window is open.
    show_templates: bool,
    templates_params: crate::commands::TemplatesParams,
    /// Bumped when `templates_params` changes; part of the preview's key.
    templates_rev: u64,
    /// The preview: the PDF for the current options, its key and job.
    templates_result: Option<crate::commands::TemplatesResult>,
    templates_track: Track<crate::commands::TemplatesResult>,
    /// Templates in another format being written on a worker thread, and the path.
    templates_export_job: Option<(PathBuf, Job<()>)>,
    /// The File > Builder panels window is open.
    show_panels: bool,
    panels_params: crate::commands::PanelsParams,
    /// Panel patterns being written on a worker thread, and the path.
    panels_export_job: Option<(PathBuf, Job<usize>)>,
    /// Bumped when `panels_params` changes; part of the measurement's key.
    panels_rev: u64,
    /// The last panels measured, current or not ([`App::panels_key`] says).
    panels_result: Option<crate::commands::PanelsPreview>,
    panels_track: Track<crate::commands::PanelsPreview>,
    /// Where the window's drawing looks, and the panel picked in it or the list.
    panels_view: crate::sheet_view::SheetView,
    panels_selected: Option<usize>,
    loading_selected: usize,
    loading_result: Option<LoadingResult>,
    loading_track: Track<LoadingResult>,
    damage_selected: usize,
    damage_result: Option<DamageResult>,
    damage_track: Track<DamageResult>,
    perf_state: PerformanceState,
    resistance_track: Track<ResistanceResult>,
    waves_track: Track<crate::waves::WaveResult>,
    wave_drag_track: Track<crate::waves::WaveDrag>,
    /// The Wave pattern view's map pan and zoom.
    wave_map: crate::wave_view::MapState,
    /// Bumped each time a wave result is installed: the texture's key.
    wave_result_rev: u64,
    raos_track: Track<RaoResult>,
    /// The Heel view's own state: start, trial weights, heel shown.
    heel: crate::heel_view::HeelViewState,
    /// The Heel view's condition (sweep and equilibrium), current or not ([`App::heel_key`]).
    heel_sweep: Option<crate::heel_view::HeelSweepOut>,
    heel_sweep_track: Track<crate::heel_view::HeelSweepOut>,
    /// The exact state at the heel shown ([`App::heel_exact_key`]).
    heel_exact: Option<seacraft_core::heel::HeelState>,
    heel_exact_track: Track<seacraft_core::heel::HeelState>,
    /// The hull and its outlines for the drawings ([`App::heel_hull_key`]: the geometry alone).
    heel_hull: Option<crate::heel_view::HeelHull>,
    heel_hull_track: Track<crate::heel_view::HeelHull>,
    /// The section drawn strongly, cut on the window thread for its x (to the millimetre).
    heel_section: Option<(u64, Vec<Vec<seacraft_core::math::Vec3>>)>,
    /// Where the Heel view's drawings were last drawn (tests pick on them).
    heel_drawn: Option<crate::heel_view::DrawnViews>,
    /// Settle at the equilibrium once the condition is current (after a release or an edit).
    heel_settle_pending: bool,
    /// When the heel shown last moved: the exact solve waits for the hand to rest.
    heel_moved: Option<std::time::Instant>,
    /// The Analysis menu's views (hydrostatic table, Bonjean, offsets, trim
    /// scan, plate development): inputs, results and jobs.
    analysis: crate::analysis_views::AnalysisViews,
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
    /// Each view's fit and maximise buttons (last frame).
    view_button_rects: [[egui::Rect; 2]; 4],
    /// Rasterized frames for the analysis modes, one per viewport.
    analysis_frames: [Option<AnalysisFrame>; 4],
    /// How many analysis frames each viewport has rasterized (test hook).
    raster_counts: [u64; 4],
    /// How many times the waterline has been cut from the mesh.
    waterline_cuts: u64,
    /// Each viewport's painter-shaded hull and the [`render::hull_key`] it
    /// was built for.
    shaded_hulls: [Option<(u64, Arc<egui::Mesh>)>; 4],
    /// Each viewport's waterline, tessellated once, and the [`render::waterline_key`] it was built
    /// for: orbiting one view rebuilds its own, and the other three draw theirs again.
    waterline_meshes: [Option<(u64, Arc<egui::Mesh>)>; 4],
    /// The current wave pattern's image, under its `waves_key`.
    wave_texture: Option<(u64, egui::TextureHandle)>,
    /// Each view's wave contours, waterlines and cut as one mesh, kept like the waterline.
    wave_meshes: [Option<(u64, Arc<egui::Mesh>)>; 4],
    /// Each viewport's control-net visibility and the revision key it was computed for.
    net_visibilities: [Option<(u64, crate::visibility::NetVisibility)>; 4],
    /// The hull drawn on the GPU.
    gpu_hull: GpuHullState,
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
    /// Model > Transform…, while open.
    transform_dialog: Option<crate::model_menu::TransformDialog>,
    /// Model > Optimise…, while open.
    optimise_dialog: Option<crate::model_menu::OptimiseDialog>,
    /// The optimisation running for the Optimise window, stamped with the
    /// model revision it started from, and its level and KG.
    optimise_job: Option<OptimiseJob>,
    /// The world axis the point drag is locked to this frame (X, Y or Z held).
    axis_lock: Option<seacraft_core::transform::Axis>,
    /// Connected-editing switch, preset name and optional reach override.
    connected: ConnectedState,
    /// Ctrl (Cmd on macOS) is held over the window: connected editing is
    /// the opposite of the switch until it is let go.
    connected_flip: bool,
    /// The connected choice last written to the settings, so a drag release
    /// writes only a choice that changed (the wheel's reach).
    persisted_choice: Option<crate::recent::ConnectedChoice>,
    /// Built-in and user connected-editing presets (`connected-edit.json`).
    preset_store: crate::connected_presets::PresetStore,
    /// Cached hover follower weights, keyed by hover, revision, preset, reach and seeds.
    hover_followers: Option<(HoverFollowersKey, Vec<(usize, f64)>)>,
    /// Cache of triangulated solids for clip_to_hull compartments and tanks.
    clipped_volumes_cache: std::sync::Mutex<Vec<ClippedMeshEntry>>,
    /// Periodic autosave state machine and startup orphan recovery queue.
    autosave: crate::autosave::AutosaveState,
}

/// Build a [`MovePlan`] for `seeds`: rigid when `connected` is `None`, or
/// connected using `reach_m` if given and the preset's own reach otherwise.
fn build_move_plan(
    surface: &Surface,
    seeds: &[usize],
    connected: Option<(&seacraft_core::connected::ConnectedPreset, Option<f64>)>,
) -> seacraft_core::netmove::MovePlan {
    match connected {
        Some((preset, reach_m)) => {
            let reach = reach_m.unwrap_or_else(|| preset.reach.metres(surface));
            let lines = seacraft_core::netlines::NetLines::of(surface);
            seacraft_core::netmove::MovePlan::connected(surface, &lines, seeds, preset, reach)
        }
        None => seacraft_core::netmove::MovePlan::rigid(surface, seeds),
    }
}

/// Re-plan a drag under way: put every point back where it started, build
/// the plan again from the drag's own seeds (not the selection, which Escape
/// may have cleared since the press) and move them to the drag's target.
fn rebuild_drag_plan(
    drag: &mut DragState,
    surface: &mut Surface,
    connected: Option<(&seacraft_core::connected::ConnectedPreset, Option<f64>)>,
) {
    drag.plan.apply(surface, Vec3::ZERO);
    drag.plan = build_move_plan(surface, &drag.plan.seeds(), connected);
    drag.plan.apply_to(surface, drag.point, drag.target);
}

/// Name, model revision and box bounds (bits) of a cached clipped solid.
type ClippedMeshEntry = (String, [u64; 7], (Vec<Vec3>, Vec<[usize; 3]>));

/// An optimisation on the job layer and what installing its result needs.
struct OptimiseJob {
    job: Job<(Surface, seacraft_core::optimize::OptimizeReport)>,
    level: u8,
    kg: Option<f64>,
}

impl App {
    pub fn new(cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        crate::theme::apply(&cc.egui_ctx);
        let mut app = Self::new_detached();
        app.status =
            "No model loaded — start a new design (Ctrl+N) or open a .fef (Ctrl+O).".to_owned();
        app.preset_store =
            crate::connected_presets::PresetStore::load(crate::recent::data_dir().as_deref());
        // The recent list lives in the user's data directory. eframe's own
        // storage (`cc.storage`) exists only with its `persistence` feature;
        // a list kept there is taken over when ours is empty.
        if let Some(storage) = crate::recent::JsonFileStorage::open_default() {
            app.set_storage(Box::new(storage));
        }
        if app.recent.is_empty() {
            if let Some(storage) = cc.storage {
                app.recent = RecentFiles::load(storage);
            }
        }
        if let Some(p) = open {
            app.load(p);
        }
        if let Some(err) = app.preset_store.error.clone() {
            app.log(Level::Warning, err);
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

    pub fn new_detached() -> Self {
        Self {
            surface: None,
            path: None,
            project: Project::default(),
            project_error: None,
            status: String::new(),
            status_level: Level::Info,
            messages: MessageLog::default(),
            show_messages: false,
            status_rect: egui::Rect::NOTHING,
            recent: RecentFiles::default(),
            storage: None,
            recent_failure: None,
            pending_recent: None,
            revision: 0,
            revision_watch: Default::default(),
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
            selection: crate::selection::Selection::default(),
            hovered_point: None,
            selected_volume: None,
            selected_windage: None,
            last_highlighted_tank: None,
            last_highlighted_compartment: None,
            history: History::new(UNDO_LIMIT),
            project_edit_source: None,
            marks: Vec::new(),
            next_mark_id: 1,
            drag: None,
            marquee: None,
            marquee_cancelled: false,
            dirty: false,
            saved_state: None,
            dialog: None,
            about_open: false,
            pending: None,
            close_requested: false,
            keyboard_taken: false,
            show_layers: true,
            active_layer: 0,
            picked_face: None,
            layer_panel: panels::LayerPanelState::default(),
            show_analysis: true,
            dock_tab: DesignDock::Hydrostatics,
            dock_wide: false,
            inspector_pending: None,
            inspector_rects: [egui::Rect::NOTHING; 3],
            inspector_move_button_rect: egui::Rect::NOTHING,
            layer_list_rect: egui::Rect::NOTHING,
            inspector_rect: egui::Rect::NOTHING,
            show_report_options: false,
            new_design: None,
            project_settings: None,
            start_rects: None,
            report_job: None,
            wave_image_job: None,
            show_templates: false,
            templates_params: crate::commands::TemplatesParams::default(),
            templates_rev: 0,
            templates_result: None,
            templates_track: Track::default(),
            templates_export_job: None,
            show_panels: false,
            panels_export_job: None,
            panels_params: crate::commands::PanelsParams::default(),
            panels_rev: 0,
            panels_result: None,
            panels_track: Track::default(),
            panels_view: crate::sheet_view::SheetView::default(),
            panels_selected: None,
            loading_selected: 0,
            loading_result: None,
            loading_track: Track::default(),
            damage_selected: 0,
            damage_result: None,
            damage_track: Track::default(),
            perf_state: PerformanceState::default(),
            resistance_track: Track::default(),
            waves_track: Track::default(),
            wave_drag_track: Track::default(),
            wave_map: Default::default(),
            wave_result_rev: 0,
            raos_track: Track::default(),
            heel: Default::default(),
            heel_sweep: None,
            heel_sweep_track: Track::default(),
            heel_exact: None,
            heel_exact_track: Track::default(),
            heel_hull: None,
            heel_hull_track: Track::default(),
            heel_section: None,
            heel_drawn: None,
            heel_settle_pending: false,
            heel_moved: None,
            analysis: Default::default(),
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
            view_button_rects: [[egui::Rect::NOTHING; 2]; 4],
            analysis_frames: [None, None, None, None],
            raster_counts: [0; 4],
            waterline_cuts: 0,
            shaded_hulls: Default::default(),
            waterline_meshes: Default::default(),
            wave_texture: None,
            wave_meshes: Default::default(),
            net_visibilities: Default::default(),
            gpu_hull: GpuHullState::default(),
            control: None,
            synthetic: Vec::new(),
            control_roots: seacraft_core::control_auth::ControlRoots::default(),
            token_file: None,
            transform_dialog: None,
            optimise_dialog: None,
            optimise_job: None,
            axis_lock: None,
            connected: ConnectedState::default(),
            connected_flip: false,
            persisted_choice: None,
            preset_store: crate::connected_presets::PresetStore::load(None),
            hover_followers: None,
            clipped_volumes_cache: std::sync::Mutex::new(Vec::new()),
            autosave: crate::autosave::AutosaveState::new(),
        }
    }

    /// Load a `.fef` model, reporting failures in the status bar and the
    /// message log instead of panicking: a bad file must never take the
    /// application down.
    pub fn load(&mut self, path: PathBuf) {
        let _ = self.open_model(&path);
    }

    /// Load a `.fef` model. A failure is logged (and shown in the status
    /// bar) and leaves the current model as it was; the typed error says
    /// whether the file could not be read or not parsed.
    pub fn open_model(&mut self, path: &std::path::Path) -> Result<(), OpenError> {
        let parsed = std::fs::read_to_string(path)
            .map_err(|err| OpenError::Read {
                path: path.to_path_buf(),
                reason: err.to_string(),
            })
            .and_then(|text| {
                fef::parse(&text).map_err(|err| OpenError::Parse {
                    path: path.to_path_buf(),
                    reason: err.to_string(),
                })
            });
        let surface = match parsed {
            Ok(surface) => surface,
            Err(err) => {
                self.log(Level::Error, err.to_string());
                return Err(err);
            }
        };
        let (project, project_error) = match Project::load_for_model(path) {
            Ok(p) => (p.unwrap_or_default(), None),
            Err(e) => (Project::default(), Some(e)),
        };
        let layers_word = if surface.layers.len() == 1 {
            "layer"
        } else {
            "layers"
        };
        let mut summary = format!(
            "{} — {} points, {} faces, {} {}",
            surface.settings.name,
            surface.points.len(),
            surface.faces.len(),
            surface.layers.len(),
            layers_word
        );
        let level = if let Some(e) = &project_error {
            summary = format!("{summary} — project file not loaded: {e}");
            Level::Warning
        } else {
            if !project.is_empty() {
                summary.push_str(" + project");
            }
            Level::Info
        };
        self.log(level, format!("opened {}: {summary}", path.display()));
        // The status line keeps the model summary it always showed.
        self.status = summary;
        self.saved_state = Some((fef::write(&surface), project.clone()));
        self.surface = Some(surface);
        self.path = Some(path.to_path_buf());
        self.project = project;
        self.project_error = project_error;
        self.dirty = false;
        self.autosave.note_model_replaced(false);
        self.adopt_project_kg();
        self.adopt();
        self.apply_requirement_defaults();
        self.remember_recent(path);
        Ok(())
    }

    pub fn load_demo(&mut self) {
        self.load_demo_shape(false);
    }

    /// The built-in demo hull, full or as a symmetric half model (only
    /// `y >= 0` modelled, mirrored on evaluation).
    pub fn load_demo_shape(&mut self, symmetric: bool) {
        let surface = demo::demo_hull(symmetric);
        let half = if symmetric { " (symmetric)" } else { "" };
        self.set_status(format!(
            "{} — built-in demo hull{half} ({} points, {} faces)",
            surface.settings.name,
            surface.points.len(),
            surface.faces.len()
        ));
        self.surface = Some(surface);
        self.path = None;
        self.project = Project::default();
        self.project_error = None;
        self.dirty = true;
        self.saved_state = None;
        self.autosave.note_model_replaced(true);
        // The demo's KG is part of the demo, as its reference numbers are.
        self.hydro.kg = Some(DEMO_KG);
        self.hydro.kg_estimate = false;
        self.adopt();
    }

    /// Replace the current model with a newly generated parametric hull.
    pub fn load_generated(&mut self, surface: Surface) {
        self.set_status(format!(
            "{} — parametric hull ({} points, {} faces)",
            surface.settings.name,
            surface.points.len(),
            surface.faces.len()
        ));
        self.surface = Some(surface);
        self.path = None;
        self.project = Project::default();
        self.project_error = None;
        self.dirty = true;
        self.saved_state = None;
        self.autosave.note_model_replaced(true);
        // A generated hull has no mass distribution: no KG until one is
        // given (the New design dialog passes its estimate on).
        self.adopt_project_kg();
        self.adopt();
    }

    /// Build an example hull and load it with its project, as one undo step
    /// (the control channel's `model.example`).
    pub fn open_example(&mut self, kind: seacraft_core::examples::Example) -> Result<(), String> {
        let (surface, project) = seacraft_core::examples::build(kind)?;
        self.replace_model_undoably(|app| app.install_example(kind, surface, project));
        Ok(())
    }

    /// Replace the model and the project with an example's: unsaved and
    /// untitled, the dock KG the example's, the design waterline its draft.
    fn install_example(
        &mut self,
        kind: seacraft_core::examples::Example,
        surface: Surface,
        project: Project,
    ) {
        self.set_status(format!(
            "{} — example `{}` ({} points, {} faces, {} layers) + project",
            kind.title(),
            kind.slug(),
            surface.points.len(),
            surface.faces.len(),
            surface.layers.len()
        ));
        self.surface = Some(surface);
        self.path = None;
        self.project = project;
        self.project_error = None;
        self.dirty = true;
        self.saved_state = None;
        self.autosave.note_model_replaced(true);
        self.adopt_project_kg();
        self.adopt();
        self.apply_requirement_defaults();
    }

    /// The dock KG of the project just adopted: the KG the user set, else
    /// the requirements' estimate, else unset.
    fn adopt_project_kg(&mut self) {
        let estimate = self.project.requirements.as_ref().and_then(|r| r.kg);
        (self.hydro.kg, self.hydro.kg_estimate) = match (self.project.kg, estimate) {
            (Some(kg), _) => (Some(kg), false),
            (None, Some(kg)) => (Some(kg), true),
            (None, None) => (None, false),
        };
    }

    /// Replace the whole model as one undo step back to the current one
    /// (New design, an example). `install` starts a fresh history, as Open
    /// does; the steps before this one are kept instead, so undo can go on
    /// past the previous model's restore.
    fn replace_model_undoably(&mut self, install: impl FnOnce(&mut Self)) {
        self.project_edit_source = None;
        let before = self.current_snapshot(true);
        let history = std::mem::replace(&mut self.history, History::new(UNDO_LIMIT));
        install(self);
        self.history = history;
        self.history.snapshot(&before);
    }

    /// Reset the derived state after a model change; the undo history
    /// starts afresh (a file opened, a hull installed by a script).
    fn adopt(&mut self) {
        self.history.clear();
        self.project_edit_source = None;
        self.marks.clear();
        self.reset_derived();
    }

    /// Everything derived from the model, reset for a new one (the undo
    /// history is the caller's business).
    fn reset_derived(&mut self) {
        self.bump_revision();
        // Measured on the previous model: left in place, it would trim the
        // new one to a point that means nothing there.
        self.hydro.lcg = None;
        self.selection.clear();
        self.hovered_point = None;
        self.selected_volume = None;
        self.selected_windage = None;
        self.last_highlighted_tank = None;
        self.last_highlighted_compartment = None;
        self.drag = None;
        self.inspector_pending = None;
        self.active_layer = 0;
        self.picked_face = None;
        self.layer_panel = panels::LayerPanelState::default();
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
        self.perf_state.wave_result = None;
        self.waves_track.reset();
        self.perf_state.wave_drag = None;
        self.wave_drag_track.reset();
        // Trial weights sit at points of the old hull: they go with it.
        self.heel = Default::default();
        self.heel_sweep = None;
        self.heel_sweep_track.reset();
        self.heel_exact = None;
        self.heel_exact_track.reset();
        self.heel_hull = None;
        self.heel_hull_track.reset();
        self.heel_section = None;
        self.heel_drawn = None;
        self.heel_settle_pending = false;
        self.analysis.clear_results();
        self.panels_result = None;
        self.panels_track.reset();
        self.panels_view = crate::sheet_view::SheetView::default();
        self.panels_selected = None;
        // Keep the chosen panel layers the new model has; with none of them, go back to the
        // layers marked developable rather than fail on names the window cannot show.
        if let (Some(chosen), Some(s)) = (&mut self.panels_params.layers, &self.surface) {
            chosen.retain(|n| s.layers.iter().any(|l| &l.name == n));
        }
        if self
            .panels_params
            .layers
            .as_ref()
            .is_some_and(Vec::is_empty)
        {
            self.panels_params.layers = None;
        }
        self.last_change = Some(std::time::Instant::now());
        self.cursor_world = None;
        self.analysis_frames = [None, None, None, None];
        self.net_visibilities = [None, None, None, None];
        if let Ok(mut guard) = self.clipped_volumes_cache.lock() {
            guard.clear();
        }
        if let Some(s) = &self.surface {
            let subdiv = seacraft_core::SubdivisionOptions {
                level: self.level,
                mode: self.mode,
            };
            self.hydro.options = HydroOptions::from_surface(s, subdiv);
            self.refit_all_cameras();
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
            let net_lines = seacraft_core::netlines::NetLines::of(surface);
            let (min, max) = crate::render::display_extents(surface);
            // Section curves and combs cost more than the refinement: a drag
            // keeps the ones from before it and rebuilds them on release.
            let carried = match self.drag {
                Some(_) => self
                    .cache
                    .take()
                    .and_then(|c| c.curves.map(|curves| (curves, c.curves_for))),
                None => None,
            };
            let (curves, curves_for) = match carried {
                Some((curves, cut)) => (Some(curves), cut),
                None => (None, None),
            };
            self.cache = Some(RenderCache {
                revision: self.revision,
                level,
                mode: self.mode,
                mesh,
                bvh: None,
                net_lines,
                curves_stale: curves.is_some(),
                curves,
                curves_for,
                waterline: None,
                min,
                max,
            });
        }
        if self.display.waterplane {
            let options = &self.hydro.options;
            let plane = seacraft_core::hydro::waterplane(
                surface,
                options.draft,
                options.trim,
                options.heel_deg,
            );
            if let Some(cache) = self.cache.as_mut() {
                let current = cache
                    .waterline
                    .as_ref()
                    .is_some_and(|(cut, _)| same_plane(cut, &plane));
                if !current {
                    let segments = render::waterline_segments(&cache.mesh, &plane);
                    cache.waterline = Some((plane, segments));
                    self.waterline_cuts += 1;
                }
            }
        }
        if self.display.wants_sections() {
            let dragging = self.drag.is_some();
            let want = section_cut(&self.display);
            if let Some(cache) = self.cache.as_mut() {
                // New counts or comb gain re-cut at once; a drag's carried
                // curves wait for the release.
                if cache.curves.is_none()
                    || cache.curves_for != Some(want)
                    || (cache.curves_stale && !dragging)
                {
                    cache.curves_stale = false;
                    cache.curves_for = Some(want);
                    let opts = self.display.section_options(level, self.mode);
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
                .resizable(true)
                .default_size((window * 0.55).round())
                .size_range(384.0..=(window * 0.85).round())
        } else {
            egui::Panel::right("analysis_dock")
                .resizable(true)
                .default_size(384.0)
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
        let wave_regime = self.wave_current().map(|w| w.regime.sentence());
        let default_kg = self.effective_performance_kg_default();
        let kg_source = self.performance_kg_source();
        let kg_estimate = self.kg_is_estimate();
        let limits = self.hydro_limits();
        let keel_z = self.keel_z();
        let pre_drag = ui.ctx().dragged_id();
        let pre_focus = ui
            .ctx()
            .text_edit_focused()
            .then(|| ui.ctx().memory(|m| m.focused()))
            .flatten();
        let before = (
            self.project.clone(),
            (self.hydro.kg, self.hydro.kg_estimate),
        );
        if self.surface.is_none() {
            // Nothing to measure: the dock keeps its frame (the layout does
            // not jump when a model arrives) but not its empty controls.
            panel.show(ui, panels::empty_dock);
            return;
        }
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
                // Tight enough that the five tabs fit one row at 384 px.
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
                            kg_estimate,
                            limits,
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
                            keel_z,
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
                            wave_regime,
                        },
                    ));
                }
            });
        });
        self.dock_tab = tab;
        if toggle_wide {
            self.set_dock_wide(!self.dock_wide);
        }
        self.last_highlighted_tank = tank_highlight;
        self.last_highlighted_compartment = comp_highlight;

        match action {
            DockAction::None => {}
            DockAction::Hydro(panels::HydroAction::Changed) => self.hydro_inputs_changed(),
            DockAction::Hydro(panels::HydroAction::CrossCurves) => self.start_curves_job(),
            DockAction::Hydro(panels::HydroAction::KgSet) => {
                if let Some(kg) = self.hydro.kg {
                    self.set_kg(kg);
                }
            }
            DockAction::Hydro(panels::HydroAction::None) => {}
            DockAction::Loading(panels::LoadingAction::Changed) => {
                self.dirty = true;
                self.autosave.note_dirty();
                self.project_changed();
            }
            DockAction::Loading(panels::LoadingAction::Compute) => self.start_loading_compute(),
            DockAction::Loading(panels::LoadingAction::None) => {}
            DockAction::Damage(panels::DamageAction::Changed) => {
                self.dirty = true;
                self.autosave.note_dirty();
                self.project_changed();
            }
            DockAction::Damage(panels::DamageAction::Compute) => self.start_damage_compute(),
            DockAction::Damage(panels::DamageAction::None) => {}
            DockAction::Windage(panels::WindageAction::Changed) => {
                self.dirty = true;
                self.autosave.note_dirty();
                self.project_changed();
            }
            DockAction::Windage(panels::WindageAction::None) => {}
            DockAction::Performance(panels::PerformanceAction::Changed) => {
                self.performance_changed();
            }
            DockAction::Performance(panels::PerformanceAction::Compute) => {
                self.start_performance_compute()
            }
            DockAction::Performance(panels::PerformanceAction::OpenWaveView) => {
                self.set_wave_view_open(true);
            }
            DockAction::Performance(panels::PerformanceAction::None) => {}
        }
        self.record_project_edit(ui.ctx(), before, pre_drag, pre_focus);
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        // `App::new` themes the window's context; a headless test's context
        // gets the same type scale on its first frame.
        crate::theme::ensure(ui);
        self.shortcuts(ui);
        self.read_connected_flip(ui);
        self.menu(ui);
        egui::Window::new("About SeaCraft")
            .open(&mut self.about_open)
            .collapsible(false)
            .default_width(520.0)
            .default_height(480.0)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .show(ui.ctx(), panels::about);
        self.toolbar(ui);
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                if self.dirty {
                    ui.colored_label(crate::theme::WARN, "● unsaved")
                        .on_hover_text(self.autosave.hover_text());
                }
                // The latest message, coloured by its level; a click lists
                // them all (View > Messages).
                let status = ui
                    .add(
                        egui::Label::new(
                            egui::RichText::new(&self.status).color(self.status_level.colour()),
                        )
                        .truncate()
                        .sense(egui::Sense::click()),
                    )
                    .on_hover_text("Click for the message log");
                self.status_rect = status.rect;
                if status.clicked() {
                    self.show_messages = !self.show_messages;
                }
                let errors = self.messages.count(Level::Error);
                if errors > 0 {
                    let word = if errors == 1 { "error" } else { "errors" };
                    if ui
                        .add(
                            egui::Label::new(
                                egui::RichText::new(format!("{errors} {word}"))
                                    .color(crate::theme::FAIL),
                            )
                            .sense(egui::Sense::click()),
                        )
                        .on_hover_text("Open the message log")
                        .clicked()
                    {
                        self.show_messages = true;
                    }
                }
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
            if self.selection.active().is_some() {
                self.sync_cache();
            }
            let station_caption = match (
                self.surface.as_ref(),
                self.cache.as_ref(),
                self.selection.active(),
            ) {
                (Some(s), Some(c), Some(p)) => render::station_caption(s, &c.net_lines, p),
                _ => None,
            };
            let mut layer_action = panels::LayerAction::None;
            let mut inspector = panels::InspectorOutput::default();
            let active_layer = self.active_layer();
            egui::Panel::left("layer_panel")
                .resizable(true)
                .default_size(264.0)
                .show(ui, |ui| {
                    // The selection Inspector sits directly under the layer
                    // list, both in one scroll area, so the eye does not
                    // travel the height of the window between them.
                    if self.surface.is_none() {
                        // No layers to list and nothing to inspect.
                        panels::empty_dock(ui);
                        return;
                    }
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        let top = ui.cursor().top();
                        if let Some(surface) = self.surface.as_ref() {
                            layer_action = panels::layers(
                                ui,
                                panels::LayersPanel {
                                    surface,
                                    active: active_layer,
                                    picked_face: self.picked_face,
                                    state: &mut self.layer_panel,
                                },
                            );
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
                                selection: &self.selection,
                                station_caption: station_caption.clone(),
                                pending: &mut self.inspector_pending,
                            },
                        );
                        self.inspector_rect = egui::Rect::from_x_y_ranges(
                            ui.max_rect().x_range(),
                            top..=ui.min_rect().bottom(),
                        );
                    });
                });
            self.apply_layer_action(layer_action);
            if self.surface.is_none() {
                self.layer_list_rect = egui::Rect::NOTHING;
                self.inspector_rect = egui::Rect::NOTHING;
            }
            self.inspector_rects = inspector.fields;
            self.inspector_move_button_rect = inspector.move_button;
            self.apply_inspector_action(inspector.action);
        } else {
            self.inspector_rects = [egui::Rect::NOTHING; 3];
            self.inspector_move_button_rect = egui::Rect::NOTHING;
            self.layer_list_rect = egui::Rect::NOTHING;
            self.inspector_rect = egui::Rect::NOTHING;
        }

        if self.show_analysis {
            self.analysis_dock(ui);
        }
        self.analysis_drawer(ui);
        self.run_jobs(ui.ctx());

        if self.surface.is_some() {
            self.start_rects = None;
            egui::CentralPanel::default().show(ui, |ui| self.viewports(ui));
        } else {
            self.marquee = None;
            self.marquee_cancelled = false;
            self.start_screen(ui);
        }
        self.new_design_window(ui);
        self.model_windows(ui);
        self.project_settings_window(ui);
        self.dialog_window(ui);
        self.report_options_window(ui);
        self.templates_window(ui);
        self.panels_window(ui);
        self.finish_project_edit_frame(ui.ctx());
        self.messages_window(ui);
        self.poll_report(ui.ctx());
        self.poll_wave_image(ui.ctx());
        self.intercept_close(ui.ctx());
        self.pending_window(ui);
        self.recovery_window(ui.ctx());
        let now = std::time::Instant::now();
        self.autosave_tick(now);
        if let Some(delay) = self.autosave.repaint_after(
            now,
            self.dirty,
            self.drag.is_some(),
            (self.revision, self.project_rev),
        ) {
            ui.ctx().request_repaint_after(delay);
        }
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
        if self.new_design.is_some()
            || self.project_settings.is_some()
            || self.transform_dialog.is_some()
            || self.autosave_prompt_open()
        {
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
            if self.drag.is_some() {
                // egui ends its drag on Escape too, so the point drag is
                // cancelled rather than left without a pointer behind it.
                self.cancel_drag();
            } else if self.marquee.take().is_some() {
                self.marquee_cancelled = true;
            } else {
                self.selection.clear();
                self.selected_volume = None;
                self.selected_windage = None;
            }
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
        let step_dir: Option<bool> = ui.input(|i| {
            i.events.iter().find_map(|e| match e {
                egui::Event::Key {
                    key,
                    pressed: true,
                    modifiers,
                    ..
                } if modifiers.alt && !modifiers.ctrl && !modifiers.command && !modifiers.shift => {
                    match key {
                        egui::Key::ArrowRight => Some(true),
                        egui::Key::ArrowLeft => Some(false),
                        _ => None,
                    }
                }
                _ => None,
            })
        });
        if let Some(forward) = step_dir {
            self.step_station(forward);
        }
    }

    /// Step the active selection along its lengthwise net line (`forward` moves
    /// towards the bow).
    fn step_station(&mut self, forward: bool) {
        let Some(active) = self.selection.active() else {
            return;
        };
        self.sync_cache();
        let Some(cache) = self.cache.as_ref() else {
            return;
        };
        if let Some(next) = cache.net_lines.step(
            active,
            seacraft_core::netlines::LineKind::Lengthwise,
            forward,
        ) {
            self.select_point(Some(next));
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
                    // One entry for the three ways to start a model.
                    ui.menu_button("New", |ui| {
                        if ui
                            .add(
                                egui::Button::new("Design…")
                                    .shortcut_text(keys(egui::Modifiers::COMMAND, egui::Key::N)),
                            )
                            .on_hover_text("A hull from the main dimensions and a hull type")
                            .clicked()
                        {
                            self.request(PendingAction::NewDesign);
                            ui.close();
                        }
                        if !seacraft_core::examples::Example::all().is_empty()
                            && ui
                                .button("From an example…")
                                .on_hover_text("One of the example hulls, as a starting point")
                                .clicked()
                        {
                            self.request(PendingAction::NewExample);
                            ui.close();
                        }
                        if ui
                            .button("Demo hull")
                            .on_hover_text("The small demo hull, to try things on")
                            .clicked()
                        {
                            self.request(PendingAction::NewDemo);
                            ui.close();
                        }
                    });
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
                    let entries = self.recent.entries();
                    let mut recent = RecentAction::None;
                    ui.menu_button("Open recent", |ui| {
                        recent = crate::recent::recent_menu(ui, &entries);
                        if recent != RecentAction::None {
                            ui.close();
                        }
                    });
                    if recent != RecentAction::None {
                        self.apply_recent_action(recent);
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
                            egui::Button::new("Builder templates…"),
                        )
                        .clicked()
                    {
                        self.show_templates = true;
                        ui.close();
                    }
                    if ui
                        .add_enabled(self.surface.is_some(), egui::Button::new("Builder panels…"))
                        .clicked()
                    {
                        self.open_builder_panels();
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
                    panels::menu_heading(ui, "Show");
                    ui.checkbox(&mut self.display.shaded, "Shaded hull");
                    ui.checkbox(&mut self.display.wireframe, "Wireframe");
                    ui.checkbox(&mut self.display.control_net, "Control net");
                    ui.checkbox(&mut self.display.points, "Control points");
                    ui.menu_button("Hidden net", |ui| {
                        for mode in render::HiddenNet::ALL {
                            if ui
                                .radio(self.display.hidden_net == mode, mode.label())
                                .clicked()
                            {
                                self.display.hidden_net = mode;
                                ui.close();
                            }
                        }
                    });
                    ui.checkbox(&mut self.display.station_focus, "Station focus");
                    ui.checkbox(&mut self.display.waterplane, "Waterplane");
                    ui.checkbox(&mut self.display.volumes, "Tanks & compartments");
                    ui.checkbox(&mut self.display.windage, "Windage outlines");
                    ui.checkbox(&mut self.display.grid, "Grid");
                    ui.checkbox(&mut self.display.orientation, "Orientation");
                    ui.checkbox(&mut self.display.curves, "Intersection curves");
                    ui.checkbox(&mut self.display.combs, "Curvature combs");
                    ui.menu_button("Curves and combs", |ui| {
                        let mut display = self.display.clone();
                        if panels::section_settings(ui, &mut display) {
                            self.display = display;
                        }
                    });
                    ui.separator();
                    panels::menu_heading(ui, "Shading");
                    crate::theme::segmented(ui, |ui| {
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
                    panels::menu_heading(ui, "Panels");
                    ui.checkbox(&mut self.show_layers, "Layer panel");
                    ui.checkbox(&mut self.show_messages, "Messages");
                    // One right-hand dock with tabs: several right-hand panels
                    // leave no room for the viewports.
                    ui.checkbox(&mut self.show_analysis, "Analysis dock");
                    // The dock's tabs, indented under the dock they belong to.
                    ui.indent("dock_tabs", |ui| {
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
                    });
                    ui.separator();
                    panels::menu_heading(ui, "Precision");
                    let mut level = self.level as u32;
                    if crate::theme::slider(ui, egui::Slider::new(&mut level, 0..=5).text("level"))
                        .changed()
                    {
                        // Through the setters: shading caches and the
                        // hydrostatics key on the revision.
                        self.set_subdivision_level(level as u8);
                    }
                    // Radio buttons, not a combo box: a click inside a menu
                    // closes it, and the combo's list closed with it before
                    // a mode could be chosen.
                    let mut mode = self.mode;
                    ui.horizontal(|ui| {
                        ui.label("Scheme");
                        ui.radio_value(&mut mode, SubdivisionMode::Loop, "Loop");
                        ui.radio_value(&mut mode, SubdivisionMode::CatmullClark, "Catmull-Clark");
                    });
                    if mode != self.mode {
                        self.set_subdivision_mode(mode);
                    }
                    ui.separator();
                    panels::menu_heading(ui, "Views");
                    if ui
                        .add(egui::Button::new("Frame the model in every view").shortcut_text("F"))
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
                self.model_menu(ui);
                self.analysis_menu(ui);
                ui.menu_button("Help", |ui| {
                    ui.menu_button("Keyboard and mouse", |ui| {
                        panels::shortcut_list(ui, &panels::shortcuts(&keys));
                    });
                    if ui.button("About SeaCraft…").clicked() {
                        self.about_open = true;
                        ui.close();
                    }
                    ui.separator();
                    panels::menu_heading(
                        ui,
                        &format!(
                            "SeaCraft {} · hull design after FREE!ship",
                            seacraft_core::VERSION
                        ),
                    );
                });
            });
        });
    }

    /// The slim toolbar under the menu bar; its actions are applied here,
    /// after the panel has drawn.
    fn toolbar(&mut self, ui: &mut egui::Ui) {
        let default_reach_m = {
            let preset = self.active_connected_preset();
            self.surface
                .as_ref()
                .map_or(0.0, |s| preset.reach.metres(s))
        };
        let state = panels::ToolbarState {
            display: &self.display,
            params: self.analysis_params,
            level: self.level,
            mode: self.mode,
            has_model: self.surface.is_some(),
            connected: &self.connected,
            connected_flipped: self.connected_flip,
            presets: &self.preset_store.presets,
            default_reach_m,
            preset_error: self.preset_store.error.as_deref(),
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
        if let Some(action) = out.connected {
            match action {
                panels::ToolbarConnectedAction::Toggle(on) => self.set_connected_on(on),
                panels::ToolbarConnectedAction::SelectPreset(name) => {
                    self.set_connected_preset(name);
                }
                panels::ToolbarConnectedAction::SetReach(reach_m) => {
                    self.set_connected_reach(reach_m);
                }
            }
        }
        self.toolbar_rects = out.rects;
    }

    // ---- start screen, New design, Project settings ---------------------

    /// The central area with no model: the start card.
    fn start_screen(&mut self, ui: &mut egui::Ui) {
        let recent = self.recent.entries();
        let (action, rects) = egui::CentralPanel::default()
            .show(ui, |ui| {
                self.central_rect = ui.available_rect_before_wrap();
                crate::new_design::start_card(ui, &recent)
            })
            .inner;
        self.start_rects = Some(rects);
        match action {
            crate::new_design::StartAction::NewDesign => self.request(PendingAction::NewDesign),
            crate::new_design::StartAction::Example => self.request(PendingAction::NewExample),
            crate::new_design::StartAction::Open => self.request(PendingAction::Open),
            crate::new_design::StartAction::Demo => self.request(PendingAction::NewDemo),
            crate::new_design::StartAction::OpenRecent(i) => {
                if let Some(entry) = recent.get(i) {
                    self.request_open_recent(entry.path.clone());
                }
            }
            crate::new_design::StartAction::None => {}
        }
    }

    /// Open the New design dialog (callers go through the unsaved-changes
    /// prompt first: Ctrl+N, File > New > Design…, the start card).
    pub fn open_new_design(&mut self) {
        self.new_design = Some(crate::new_design::NewDesignState::new(
            crate::new_design::SEA_WATER_T_M3,
        ));
    }

    /// Open the New design dialog on *Example*, the first example's preview
    /// already drawn (the start card's "Open an example…").
    pub fn open_new_design_on_example(&mut self) {
        self.new_design = Some(crate::new_design::NewDesignState::on_example(
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
                self.replace_model_undoably(|app| app.load_demo_shape(half));
            }
            NewDesignRequest::Example(design) => {
                let crate::new_design::ExampleDesign {
                    kind,
                    surface,
                    project,
                } = *design;
                self.replace_model_undoably(|app| app.install_example(kind, surface, project));
            }
            NewDesignRequest::Parametric(design) => {
                let crate::new_design::ParametricDesign {
                    spec,
                    name,
                    requirements,
                    built,
                } = *design;
                let before = self.current_snapshot(true);
                // Installing a hull starts a fresh history; this one is an
                // undo step, so the steps before it stay.
                let history = std::mem::replace(&mut self.history, History::new(UNDO_LIMIT));
                let result = match built {
                    Some(p) if p.spec == spec => {
                        crate::commands::install_generated(self, p.surface, p.report)
                    }
                    _ => match crate::commands::generate(self, &spec) {
                        Ok(r) => r,
                        Err(e) => {
                            self.history = history;
                            self.log(Level::Error, format!("New design failed: {e}"));
                            return;
                        }
                    },
                };
                self.history = history;
                if let Some(s) = self.surface.as_mut() {
                    if !name.is_empty() {
                        s.settings.name = name.clone();
                    }
                }
                // The design waterline is the requested draft.
                self.hydro.options.draft = spec.draft;
                self.project.requirements = Some(requirements);
                // The requirements' KG estimate when one was given, else none.
                self.adopt_project_kg();
                self.apply_requirement_defaults();
                self.reset_cameras();
                self.history.snapshot(&before);
                self.set_status(format!(
                    "{name} — new {} design ({} points, {} faces)",
                    spec.kind, result.points, result.faces
                ));
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

    /// Model `z` of the keel: the lowest hydrostatic point, the datum KG,
    /// KB and the weather criterion's windage heights are measured from
    /// (0 without a model).
    pub fn keel_z(&self) -> f64 {
        self.surface
            .as_ref()
            .map_or(0.0, |s| s.lowest_hydrostatics_point())
    }

    /// The Windage tab's totals: area (m²) and area-weighted centroid above
    /// the keel (m) of the project's windage items, on the weather
    /// criterion's datum.
    pub fn windage_totals(&self) -> (f64, f64) {
        panels::windage_totals(&self.project.windage, self.keel_z())
    }

    /// The lightship a new loading condition starts from: the measured
    /// displacement at its LCB, at the dock KG above the keel.
    pub fn lightship_seed(&self) -> seacraft_core::loading::WeightItem {
        let keel = self.keel_z();
        // The dock KG (the user's, else the requirements' estimate). With
        // neither the seed sits at the deck: a placeholder on the safe side
        // (the highest the centre of gravity of a bare hull could be) that
        // the user replaces, not a KG the model is credited with.
        let z = match self.hydro.kg {
            Some(kg) => keel + kg,
            None => self.surface.as_ref().map_or(keel, |s| s.extents().1.z),
        };
        let (mass_t, x) = match &self.hydro.result {
            Some(h) if h.displacement_t > 0.0 => (h.displacement_t, h.lcb.x),
            _ => (10.0, 0.0),
        };
        seacraft_core::loading::WeightItem {
            name: "Lightship".into(),
            mass_t,
            x,
            y: 0.0,
            z,
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
        let requirements_changed = requirements != self.project.requirements;
        if settings_changed || requirements_changed {
            // One Apply, one undo step, whatever it changed.
            self.project_edit_source = None;
            let mut snap = Snapshot::edit(surface, self.hydro.options.draft);
            if requirements_changed {
                snap.project = Some((
                    self.project.clone(),
                    (self.hydro.kg, self.hydro.kg_estimate),
                ));
            }
            self.history.snapshot(&snap);
        }
        if settings_changed {
            if let Some(s) = self.surface.as_mut() {
                s.settings.name = name;
                s.settings.designer = designer;
                s.settings.water_density = water_density;
            }
            self.hydro.options.water_density = water_density;
            self.mark_geometry_changed();
        }
        if requirements_changed {
            self.set_requirements(requirements);
        }
        self.set_status("project settings applied".to_owned());
    }

    /// Replace the design requirements (Project settings, or its undo): the
    /// dock KG follows a changed estimate, the speed range a changed speed.
    fn set_requirements(&mut self, requirements: Option<seacraft_core::project::Requirements>) {
        if requirements == self.project.requirements {
            return;
        }
        let speed = |r: &Option<seacraft_core::project::Requirements>| {
            r.as_ref().and_then(|r| r.design_speed_kn)
        };
        let speed_changed = speed(&requirements) != speed(&self.project.requirements);
        self.project.requirements = requirements;
        self.dirty = true;
        self.autosave.note_dirty();
        if self.project.kg.is_none() && (self.hydro.kg.is_none() || self.hydro.kg_estimate) {
            // The dock follows the estimate until the user sets a KG
            // (the demo's own KG is not an estimate and stays).
            self.adopt_project_kg();
            self.hydro_inputs_changed();
        }
        if speed_changed {
            self.apply_requirement_defaults();
        }
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
            PendingAction::NewExample => self.open_new_design_on_example(),
            PendingAction::Open => self.open_dialog(DialogKind::Open),
            PendingAction::OpenRecent => {
                if let Some(path) = self.pending_recent.take() {
                    self.load(path);
                }
            }
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
            DialogKind::Templates => dir
                .join(format!("{clean}-moulds.pdf"))
                .to_string_lossy()
                .to_string(),
            DialogKind::Panels => dir
                .join(format!("{clean}-panels.pdf"))
                .to_string_lossy()
                .to_string(),
            DialogKind::WaveImage => {
                let froude = self.wave_current().map_or(0.0, |w| w.field.froude);
                dir.join(format!("{clean}-waves-fn{froude:.2}.png"))
                    .to_string_lossy()
                    .to_string()
            }
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

    /// Write the model as `.fef`. Returns whether it was written; the
    /// outcome is logged (see [`App::write_model`]).
    pub fn save_to(&mut self, path: &std::path::Path) -> bool {
        self.write_model(path).is_ok()
    }

    /// Write the model as `.fef` (and its project sidecar). A failure is
    /// logged as an error and returned; a model written without its design
    /// data (an unreadable sidecar left alone) is a logged warning.
    pub fn write_model(&mut self, path: &std::path::Path) -> Result<(), String> {
        let Some(surface) = &self.surface else {
            return Err("No model loaded.".to_owned());
        };
        let text = fef::write(surface);
        if let Err(err) = seacraft_core::io::write_atomic(path, &text) {
            let msg = format!("cannot write {}: {err}", path.display());
            self.log(Level::Error, msg.clone());
            return Err(msg);
        }
        self.path = Some(path.to_path_buf());
        self.remember_recent(path);
        let sidecar = project::sidecar_path(path);
        if let Some(e) = &self.project_error {
            let mut msg = format!(
                "saved {}; project file left as it was ({e})",
                path.display()
            );
            // The sidecar on disk could not be read, so it is not
            // overwritten; design data entered since stays unsaved.
            self.saved_state = Some((text, Project::default()));
            self.refresh_dirty();
            if self.dirty {
                msg.push_str(" — design data NOT saved: fix or remove the project file");
            }
            self.log(Level::Warning, msg);
            return Ok(());
        }
        let mut msg = format!("saved {}", path.display());
        if !self.project.is_empty() || sidecar.exists() {
            if let Err(e) = self
                .project
                .validate()
                .and_then(|()| self.project.save(&sidecar))
            {
                let msg = format!("saved {}, but {e}", path.display());
                self.log(Level::Error, msg.clone());
                return Err(msg);
            }
            msg.push_str(" + project");
        }
        self.dirty = false;
        self.saved_state = Some((text, self.project.clone()));
        self.autosave.note_clean();
        self.log(Level::Info, msg);
        Ok(())
    }

    /// Write one of the export formats. Returns whether it was written; the
    /// outcome is logged (see [`App::write_export`]).
    pub fn export_to(&mut self, kind: ExportKind, path: &std::path::Path) -> bool {
        self.write_export(kind, path).is_ok()
    }

    /// Write one of the export formats; a failure is logged and returned.
    /// Save the current wave pattern as PNG, SVG or VTI, by `path`'s extension (any case), through
    /// `write_atomic`. Only a current pattern is saved: an out-of-date one writes nothing. The
    /// checks answer at once; the encoding (a quarter of a second for SVG or VTI at a million
    /// points) and the write run in a job, which logs "Saved wave pattern to …" when done.
    pub fn write_wave_image(&mut self, path: &std::path::Path) -> Result<(), String> {
        use seacraft_core::wave_pattern::{wave_png, wave_svg, wave_vti};
        #[derive(Clone, Copy)]
        enum Format {
            Png,
            Svg,
            Vti,
        }
        let extension = path
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        let format = match extension.as_deref() {
            Some("png") => Format::Png,
            Some("svg") => Format::Svg,
            Some("vti") => Format::Vti,
            _ => return Err("save the wave pattern as .png, .svg or .vti".to_owned()),
        };
        let field = self
            .wave_current()
            .ok_or("the wave pattern is out of date or not computed; wait for it to finish")?
            .field
            .clone();
        if self.wave_image_job.is_some() {
            return Err("a wave pattern image is still being written".to_owned());
        }
        let clip = self.perf_state.waves.clip_m;
        let target = path.to_path_buf();
        let job = Job::spawn(self.revision, move || {
            let bytes = match format {
                Format::Png => wave_png(&field, clip).ok_or("the wave field is empty")?,
                Format::Svg => wave_svg(&field, clip).into_bytes(),
                Format::Vti => wave_vti(&field).into_bytes(),
            };
            seacraft_core::io::write_atomic(&target, bytes)
                .map_err(|e| format!("cannot write {}: {e}", target.display()))
        });
        self.set_status(format!("writing wave pattern {}…", path.display()));
        self.wave_image_job = Some((path.to_path_buf(), job));
        Ok(())
    }

    /// Collect a finished wave-image job, if any.
    fn poll_wave_image(&mut self, ctx: &egui::Context) {
        let Some((_, job)) = &self.wave_image_job else {
            return;
        };
        let out = match job.poll() {
            crate::jobs::Poll::Running => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            crate::jobs::Poll::Done(()) => Ok(()),
            crate::jobs::Poll::Failed(e) => Err(e),
        };
        if let Some((path, _)) = self.wave_image_job.take() {
            self.finish_wave_image(&path, out);
        }
    }

    /// Wait for the wave-image job and log its outcome; `None` when none runs (tests, scripts).
    pub fn wait_wave_image(&mut self) -> Option<Result<(), String>> {
        let (path, job) = self.wave_image_job.take()?;
        let out = job.wait();
        self.finish_wave_image(&path, out.clone());
        Some(out)
    }

    fn finish_wave_image(&mut self, path: &std::path::Path, out: Result<(), String>) {
        match out {
            Ok(()) => self.log(
                Level::Info,
                format!("Saved wave pattern to {}", path.display()),
            ),
            Err(e) => self.log(Level::Error, format!("Wave pattern image failed: {e}")),
        }
    }

    pub fn write_export(&mut self, kind: ExportKind, path: &std::path::Path) -> Result<(), String> {
        self.write_export_with(kind, path, None)
    }

    /// [`App::write_export`] with the linesplan drawn with `linesplan`
    /// instead of the viewports' section options (the control channel's
    /// `model.export` with explicit counts). Ignored for the mesh formats.
    pub fn write_export_with(
        &mut self,
        kind: ExportKind,
        path: &std::path::Path,
        linesplan: Option<seacraft_core::linesplan::LinesplanOptions>,
    ) -> Result<(), String> {
        use seacraft_core::io::write_atomic;
        let Some(surface) = &self.surface else {
            return Err("No model loaded.".to_owned());
        };
        // By default the drawing shows what the viewports show: their
        // refinement, section counts and combs, as the CLI does with its flags.
        let linesplan_opts =
            linesplan.unwrap_or_else(|| self.display.section_options(self.level, self.mode));
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
            ExportKind::OffsetsPdf => {
                let defaults = crate::commands::OffsetsParams::default();
                return self.write_offsets_pdf(path, &defaults, false).map(|_| ());
            }
        };
        self.log_export(kind.label(), path, written)
    }

    /// Write the table of offsets `params` asks for as a PDF, with `imperial` tables after the
    /// metric ones, and return its page count; the outcome is logged. The File menu writes the
    /// Offsets view's default table (21 stations, 6 waterlines, 4 buttocks).
    pub fn write_offsets_pdf(
        &mut self,
        path: &std::path::Path,
        params: &crate::commands::OffsetsParams,
        imperial: bool,
    ) -> Result<usize, String> {
        let label = ExportKind::OffsetsPdf.label();
        let name = self
            .surface
            .as_ref()
            .ok_or_else(|| "No model loaded.".to_owned())?
            .settings
            .name
            .clone();
        let made = crate::commands::offsets_in(crate::commands::ModelView::of(self), params)
            .and_then(|r| {
                let table = seacraft_core::offsets::TableOfOffsets {
                    keel_z: r.keel_z,
                    waterlines_m: r.waterlines_m,
                    buttocks_m: r.buttocks_m,
                    stations: r.stations,
                };
                let title = format!("{name} - table of offsets");
                seacraft_core::offsets::pdf(&table, &title, imperial)
            });
        match made {
            Ok(doc) => {
                let written = seacraft_core::io::write_atomic(path, doc.bytes);
                self.log_export(label, path, written).map(|()| doc.pages)
            }
            Err(err) => {
                let msg = format!("cannot export {}: {err}", path.display());
                self.log(Level::Error, msg.clone());
                Err(msg)
            }
        }
    }

    /// Log the outcome of writing an export labelled `label` to `path`.
    fn log_export(
        &mut self,
        label: &str,
        path: &std::path::Path,
        written: std::io::Result<()>,
    ) -> Result<(), String> {
        match written {
            Ok(()) => {
                self.log(
                    Level::Info,
                    format!(
                        "exported {} ({})",
                        path.display(),
                        label.trim_end_matches('…')
                    ),
                );
                Ok(())
            }
            Err(err) => {
                let msg = format!("cannot write {}: {err}", path.display());
                self.log(Level::Error, msg.clone());
                Err(msg)
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
            DialogKind::Templates => "Builder templates",
            DialogKind::Panels => "Builder panels",
            DialogKind::WaveImage => "Wave pattern image",
        };
        let mut accept = false;
        let mut cancel = false;
        // Its own id: a window that opens it may carry the same title.
        egui::Window::new(title)
            .id(egui::Id::unique("file_dialog"))
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
                    DialogKind::Templates | DialogKind::Panels => ".pdf".to_owned(),
                    DialogKind::WaveImage => ".png".to_owned(),
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
        // Each action's typed result decides: the dialog stays open on the
        // error it returned (also logged), and closes on success.
        let result = match dialog.kind {
            DialogKind::Open => self.open_model(&path).map_err(|e| e.to_string()),
            DialogKind::SaveAs => self.write_model(&path),
            DialogKind::Export(kind) => self.write_export(kind, &path),
            DialogKind::Report => self.start_report(&path),
            DialogKind::Templates => self.start_templates_export(&path),
            DialogKind::Panels => self.start_panels_export(&path),
            DialogKind::WaveImage => self.write_wave_image(&path),
        };
        match result {
            Err(err) => {
                dialog.error = Some(err);
                self.dialog = Some(dialog);
            }
            Ok(()) => {
                if let Some(action) = dialog.then {
                    if !self.dirty {
                        self.perform(action);
                    }
                }
            }
        }
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
                    if ui.add(crate::theme::primary_button("Save")).clicked() {
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
        if self.surface.is_some() {
            self.refit_all_cameras();
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
                    kg: (self.hydro.kg, self.hydro.kg_estimate),
                    project_error: self.project_error.clone(),
                    path: self.path.clone(),
                    saved_state: self.saved_state.clone(),
                })
            }),
            project: None,
        }
    }

    /// The current state with what `other` carries, so that undo and redo
    /// swap like for like.
    fn current_snapshot_like(&self, other: Option<&Snapshot>) -> Snapshot {
        let mut snap = self.current_snapshot(other.is_some_and(|s| s.model.is_some()));
        if other.is_some_and(|s| s.project.is_some()) {
            snap.project = Some((
                self.project.clone(),
                (self.hydro.kg, self.hydro.kg_estimate),
            ));
        }
        snap
    }

    /// Record a dock or Report options edit on the undo stack. Continuous
    /// edits on one widget (dragging a `DragValue` or typing in a `TextEdit`)
    /// share the snapshot taken on the first frame that changed the project.
    fn record_project_edit(
        &mut self,
        ctx: &egui::Context,
        before: (Project, (Option<f64>, bool)),
        pre_drag: Option<egui::Id>,
        pre_focus: Option<egui::Id>,
    ) {
        let current_kg = (self.hydro.kg, self.hydro.kg_estimate);
        if self.project == before.0 && current_kg == before.1 {
            return;
        }
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let post_drag = ctx.dragged_id();
        let post_focus = ctx
            .text_edit_focused()
            .then(|| ctx.memory(|m| m.focused()))
            .flatten();
        let pointer_button = ctx.input(|i| i.pointer.any_pressed() || i.pointer.any_released());
        let continuous_id = if let Some(id) = post_drag.or(pre_drag) {
            Some(id)
        } else if !pointer_button {
            post_focus.or(pre_focus)
        } else {
            None
        };
        if continuous_id.is_none() || continuous_id != self.project_edit_source {
            let snap =
                Snapshot::project_edit(surface, self.hydro.options.draft, before.0, before.1);
            self.history.snapshot(&snap);
        }
        self.project_edit_source = continuous_id;
        self.refresh_dirty();
    }

    /// Clear `project_edit_source` once the drag or text-field focus that
    /// started the continuous edit has ended.
    fn finish_project_edit_frame(&mut self, ctx: &egui::Context) {
        let Some(active) = self.project_edit_source else {
            return;
        };
        let still_dragging =
            ctx.dragged_id() == Some(active) && !ctx.input(|i| i.pointer.any_released());
        let still_typing = ctx.text_edit_focused() && ctx.memory(|m| m.focused()) == Some(active);
        if !still_dragging && !still_typing {
            self.project_edit_source = None;
            // Typed or dragged back to where it started: nothing to undo.
            let kg = (self.hydro.kg, self.hydro.kg_estimate);
            let unchanged = self
                .history
                .peek_undo()
                .and_then(|snap| snap.project.as_ref())
                .is_some_and(|(project, before)| *project == self.project && *before == kg);
            if unchanged {
                self.history.discard_last();
            }
        }
    }

    /// Put a snapshot's project and dock KG back, marking every result that
    /// depends on what changed out of date.
    fn restore_project(&mut self, project: Project, kg: (Option<f64>, bool)) {
        let kg_changed =
            (self.hydro.kg, self.hydro.kg_estimate) != kg || self.project.kg != project.kg;
        let speed = |p: &Project| p.requirements.as_ref().and_then(|r| r.design_speed_kn);
        let speed_changed = speed(&project) != speed(&self.project);
        let data_changed = self.project.loading_conditions != project.loading_conditions
            || self.project.compartments != project.compartments
            || self.project.damage_cases != project.damage_cases
            || self.project.windage != project.windage;
        self.project = project;
        (self.hydro.kg, self.hydro.kg_estimate) = kg;
        if self.project.loading_conditions.is_empty() {
            self.loading_selected = 0;
        } else {
            self.loading_selected = self
                .loading_selected
                .min(self.project.loading_conditions.len() - 1);
        }
        if self.project.damage_cases.is_empty() {
            self.damage_selected = 0;
        } else {
            self.damage_selected = self
                .damage_selected
                .min(self.project.damage_cases.len() - 1);
        }
        if self
            .selected_windage
            .is_some_and(|i| i >= self.project.windage.len())
        {
            self.selected_windage = None;
        }
        let tanks_len = self
            .project
            .loading_conditions
            .get(self.loading_selected)
            .map_or(0, |c| c.tanks.len());
        let volume_gone = match self.selected_volume {
            Some(VolumeRef::Tank(i)) => i >= tanks_len,
            Some(VolumeRef::Compartment(i)) => i >= self.project.compartments.len(),
            None => false,
        };
        if volume_gone {
            self.selected_volume = None;
        }
        if kg_changed {
            self.hydro_inputs_changed();
        }
        if speed_changed {
            self.apply_requirement_defaults();
        }
        if data_changed {
            self.project_changed();
        }
    }

    /// Put a snapshot back as the current model.
    fn restore_snapshot(&mut self, snap: Snapshot) {
        let density = |s: &Option<Surface>| s.as_ref().map(|s| s.settings.water_density);
        if let Some(d) = density(&snap.surface).filter(|d| Some(*d) != density(&self.surface)) {
            // A Project settings density change undone.
            self.hydro.options.water_density = d;
        }
        let geometry_changed = self.surface.as_ref().map(fef::write)
            != snap.surface.as_ref().map(fef::write)
            || self.hydro.options.draft != snap.draft
            || snap.model.is_some();
        self.surface = snap.surface;
        let n = self.surface.as_ref().map_or(0, |s| s.points.len());
        self.selection.retain_valid(n);
        if let Some(m) = snap.model {
            self.project = m.project;
            self.project_error = m.project_error;
            self.path = m.path;
            self.saved_state = m.saved_state;
            (self.hydro.kg, self.hydro.kg_estimate) = m.kg;
            self.reset_derived();
            self.apply_requirement_defaults();
        }
        if let Some((project, kg)) = snap.project {
            self.restore_project(project, kg);
        }
        self.hydro.options.draft = snap.draft;
        if geometry_changed {
            self.bump_revision();
            self.touch_geometry();
        }
        self.refresh_dirty();
    }

    pub fn undo(&mut self) {
        self.project_edit_source = None;
        let whole = self.history.peek_undo().is_some_and(|s| s.model.is_some());
        let current = self.current_snapshot_like(self.history.peek_undo());
        if let Some(previous) = self.history.undo(&current) {
            let moved = previous.surface.as_ref().map_or(0, |s| s.points.len());
            self.restore_snapshot(previous);
            self.set_status(if whole {
                "undo — previous model restored".to_owned()
            } else {
                format!("undo — {moved} control points restored")
            });
        }
    }

    pub fn redo(&mut self) {
        self.project_edit_source = None;
        let current = self.current_snapshot_like(self.history.peek_redo());
        if let Some(next) = self.history.redo(&current) {
            self.restore_snapshot(next);
            self.set_status("redo".to_owned());
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
        self.project_edit_source = None;
        self.history.truncate_undo(undo_len);
        self.history.clear_redo();
        let n = self.surface.as_ref().map_or(0, |s| s.points.len());
        self.selection.retain_valid(n);
        self.mark_geometry_changed();
        self.refresh_dirty();
        self.set_status(format!("restored to mark {id}"));
        Ok(())
    }

    /// Drop a mark without restoring.
    pub fn forget_mark(&mut self, id: u64) {
        if let Some(pos) = self.marks.iter().position(|(m, _, _, _)| *m == id) {
            self.marks.truncate(pos);
        }
    }

    fn viewports(&mut self, ui: &mut egui::Ui) {
        let failure = self.gpu_hull.failure.lock().ok().and_then(|mut f| f.take());
        if let Some((key, message)) = failure {
            self.report_gpu_hull_failure(key, message);
        }
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
        self.view_button_rects = [[egui::Rect::NOTHING; 2]; 4];
        let fit_key = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::NONE,
            egui::Key::F,
        ));
        let max_key = ui.ctx().format_shortcut(&egui::KeyboardShortcut::new(
            egui::Modifiers::COMMAND,
            egui::Key::Space,
        ));
        let mut toggle_maximise = false;
        let mut fresh: Vec<(usize, AnalysisFrame)> = Vec::new();
        let mut fresh_hulls: Vec<(usize, u64, Arc<egui::Mesh>)> = Vec::new();
        let mut fresh_waterlines: Vec<(usize, u64, Arc<egui::Mesh>)> = Vec::new();
        let mut fresh_wave_meshes: Vec<(usize, u64, Arc<egui::Mesh>)> = Vec::new();
        self.load_wave_texture(ui.ctx());
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
        self.hovered_point = None;

        for &(cell, index) in &cells {
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
                // Ctrl (Cmd) + wheel reaches us as egui's zoom delta,
                // exp(wheel / 200); turned back into wheel points it zooms
                // the view, or sets the reach during a Ctrl-connected drag.
                let (scroll, cursor) = ui.input(|i| {
                    let zoom = i.zoom_delta();
                    let from_zoom = if zoom > 0.0 { zoom.ln() * 200.0 } else { 0.0 };
                    (i.smooth_scroll_delta.y + from_zoom, i.pointer.hover_pos())
                });
                if scroll.abs() > 0.0 {
                    if self.drag.as_ref().is_some_and(|d| d.connected) {
                        self.scale_connected_drag_reach(scroll);
                    } else {
                        let factor = (-scroll as f64 * 0.0025).exp();
                        match cursor {
                            // The world point under the cursor stays put.
                            Some(cursor) => self.cameras[index].zoom_at(factor, cursor, cell),
                            None => self.cameras[index].zoom(factor),
                        }
                    }
                }
            }

            if response.dragged_by(egui::PointerButton::Middle) {
                // Middle-drag pans in every view, perspective included.
                let delta = response.drag_delta();
                self.cameras[index].pan(delta.x, delta.y, cell);
            }
            let secondary = ui.input(|i| i.pointer.secondary_down());
            let mut added_on_press = false;
            if response.dragged() && secondary && self.cameras[index].kind.is_perspective() {
                let delta = response.drag_delta();
                self.cameras[index].orbit(delta.x, -delta.y);
            } else {
                // Grab on *press*, not when egui decides it is a drag: the
                // pick must use the pixel the user aimed at, not where the
                // cursor has already slid to.
                // A press on a corner button is not a grab at the point
                // that happens to lie under it.
                let on_button = ui
                    .input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()))
                    .is_some_and(|p| render::view_buttons(cell).iter().any(|b| b.contains(p)));
                if response.hovered() && !on_button && ui.input(|i| i.pointer.primary_pressed()) {
                    self.marquee_cancelled = false;
                    self.sync_net_visibility(index, cell);
                    self.begin_drag(ui, &view, cell, index);
                    if self.drag.is_none()
                        && pointer_shift(ui)
                        && self.display.points
                        && self.surface.is_some()
                    {
                        if let Some(origin) =
                            ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()))
                        {
                            self.active = index;
                            self.marquee = Some((index, origin, origin));
                        }
                    }
                }
                // Nothing is hovered while the view itself is dragged (a pan
                // or an orbit): the highlight would flicker under the pointer.
                if response.hovered()
                    && !on_button
                    && !response.dragged()
                    && self.drag.is_none()
                    && self.marquee.is_none()
                {
                    self.sync_net_visibility(index, cell);
                    self.hovered_point = self.pick_at(ui, &view, cell, index);
                }
                if let Some(m) = self.marquee.as_mut().filter(|m| m.0 == index) {
                    if ui.input(|i| i.pointer.primary_down()) {
                        if let Some(pos) =
                            ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()))
                        {
                            m.2 = pos;
                        }
                    } else if !ui.input(|i| i.pointer.any_released()) {
                        self.marquee = None;
                    }
                }
                if response.dragged_by(egui::PointerButton::Primary) {
                    if self.drag.as_ref().is_some_and(|d| d.view == index) {
                        self.drag_to(ui, &view, cell);
                    } else if self.drag.is_none()
                        && self.marquee.is_none()
                        && !self.marquee_cancelled
                    {
                        let delta = response.drag_delta();
                        self.cameras[index].pan(delta.x, delta.y, cell);
                    }
                }
                if ui.input(|i| i.pointer.any_released()) {
                    self.marquee_cancelled = false;
                    if let Some((_, from, to)) = self.marquee.take_if(|m| m.0 == index) {
                        let rect = egui::Rect::from_two_pos(from, to).intersect(cell);
                        if (rect.width() >= 4.0 || rect.height() >= 4.0) && self.display.points {
                            self.sync_net_visibility(index, cell);
                            let vis = self.net_visibility(index);
                            if let Some(surface) = self.surface.as_ref() {
                                let hits = picking::points_in_rect(
                                    surface,
                                    &view,
                                    cell,
                                    rect,
                                    vis,
                                    self.display.hidden_net,
                                );
                                if !hits.is_empty() {
                                    self.selection.add_all(hits);
                                    self.selected_windage = None;
                                    self.selected_volume = None;
                                }
                            }
                        }
                    }
                    if let Some(drag) = self.drag.take_if(|d| d.view == index) {
                        added_on_press = drag.added_on_press;
                        self.axis_lock = None;
                        let moved = self.surface.as_ref().is_some_and(|s| {
                            drag.plan.weights().into_iter().any(|(p, _)| {
                                drag.plan
                                    .start(p)
                                    .zip(s.points.get(p))
                                    .is_some_and(|(from, cp)| (cp.p - from).length() > 1e-12)
                            })
                        });
                        if moved {
                            // Full refinement and measurement again; the
                            // automatic recomputes count from the release.
                            self.bump_revision();
                            self.touch_geometry();
                            let count = drag.plan.weights().len();
                            if count > 1 {
                                self.set_status(format!("{count} control points moved"));
                            } else {
                                self.set_status("control point moved".to_owned());
                            }
                            if drag.connected
                                && self.persisted_choice.as_ref() != Some(&self.connected_choice())
                            {
                                self.persist_recent();
                            }
                        } else {
                            self.history.discard_last();
                            // The wheel may have re-planned the press
                            // without moving anything.
                            self.refresh_dirty();
                        }
                    }
                }
            }
            if response.clicked() {
                self.active = index;
                self.sync_net_visibility(index, cell);
                let picked = self.pick_at(ui, &view, cell, index);
                let shift = pointer_shift(ui);
                if shift {
                    if let Some(p) = picked {
                        if !added_on_press {
                            self.selection.toggle(p);
                        }
                    }
                } else if let Some(p) = picked {
                    self.selection.set_only(p);
                } else {
                    self.selection.clear();
                }
                self.picked_face = self.pick_face_at(ui, &view, cell);
                if self.selection.is_empty() {
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
        }

        // Input first, then drawing: a point dragged in one view refines the mesh in the loop above,
        // and every view's GPU callback of a frame must share one geometry. The resources hold one,
        // so a second mesh in a frame would leave the views already rendered with nothing to show.
        self.refresh_gpu_hull_geometry();

        let drag_followers: Option<Vec<(usize, f64)>> = self
            .drag
            .as_ref()
            .filter(|d| d.connected)
            .map(|d| d.plan.followers());
        let hover_followers: Vec<(usize, f64)> =
            if drag_followers.is_none() && self.connected_now() && self.display.points {
                self.hovered_followers().to_vec()
            } else {
                Vec::new()
            };
        let followers: &[(usize, f64)] = drag_followers
            .as_deref()
            .unwrap_or(hover_followers.as_slice());

        for &(cell, index) in &cells {
            self.sync_net_visibility(index, cell);
            if let Some(mut scene) = self.scene(&volume_overlays, &windage_overlays) {
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
                let route = self.hull_route(ui.ctx(), index, cell, &scene);
                // The painter's shaded hull, kept while the view, the mesh
                // and the layer colours stay the same.
                let painted = matches!(route, HullRoute::Painted);
                let hull = (painted && texture.is_none() && self.display.shaded).then(|| {
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
                // The waterline as one mesh, kept like the hull. Only a waterline the app has cut
                // is kept: one cut on the fly is drawn as strokes.
                let waterline =
                    scene
                        .waterline
                        .filter(|_| self.display.waterplane)
                        .map(|segments| {
                            let stamp = self.cache.as_ref().map_or(0, RenderCache::stamp);
                            let camera = &self.cameras[index];
                            let key = render::waterline_key(
                                render::hull_key(stamp, camera, cell, &scene, &self.display),
                                ui.ctx().pixels_per_point(),
                                segments,
                            );
                            match &self.waterline_meshes[index] {
                                Some((k, mesh)) if *k == key => mesh.clone(),
                                _ => {
                                    let mesh = Arc::new(render::waterline_mesh(
                                        ui.ctx(),
                                        &camera.transform(),
                                        cell,
                                        segments,
                                    ));
                                    fresh_waterlines.push((index, key, mesh.clone()));
                                    mesh
                                }
                            }
                        });
                // The pattern's lines as one mesh, kept while the pattern and the view stay. Only
                // Plan view draws them, so no other view builds them (orbiting Perspective view
                // would otherwise re-tessellate every contour each frame).
                if let Some(result) = scene
                    .waves
                    .as_ref()
                    .map(|w| w.result)
                    .filter(|_| self.cameras[index].kind == ViewKind::Plan)
                {
                    let stamp = self.cache.as_ref().map_or(0, RenderCache::stamp);
                    let camera = &self.cameras[index];
                    let key = revision_key(&[
                        self.waves_key(),
                        render::hull_key(stamp, camera, cell, &scene, &self.display),
                        ui.ctx().pixels_per_point().to_bits() as u64,
                    ]);
                    let mesh = match &self.wave_meshes[index] {
                        Some((k, mesh)) if *k == key => mesh.clone(),
                        _ => {
                            let mesh = Arc::new(render::wave_mesh(
                                ui.ctx(),
                                &camera.transform(),
                                cell,
                                result,
                            ));
                            fresh_wave_meshes.push((index, key, mesh.clone()));
                            mesh
                        }
                    };
                    if let Some(waves) = scene.waves.as_mut() {
                        waves.contours = Some(mesh);
                    }
                }
                let no_hull;
                let (display, gpu_hull) = match route {
                    HullRoute::Painted => (&self.display, None),
                    HullRoute::Gpu(callback) => (&self.display, Some(callback)),
                    HullRoute::Nothing => {
                        no_hull = render::DisplayOptions {
                            shaded: false,
                            wireframe: false,
                            ..self.display.clone()
                        };
                        (&no_hull, None)
                    }
                };
                let mut sel_with_drag;
                let drawn_selection = match &self.drag {
                    Some(d) if !self.selection.contains(d.point) => {
                        sel_with_drag = self.selection.clone();
                        sel_with_drag.add_all([d.point]);
                        &sel_with_drag
                    }
                    _ => &self.selection,
                };
                let vis = self.net_visibility(index);
                let focus_point = self.hovered_point.or_else(|| drawn_selection.active());
                let net_lines = self.cache.as_ref().map(|c| &c.net_lines);
                render::viewport_with_texture(
                    ui,
                    cell,
                    &self.cameras[index],
                    Some(&scene),
                    display,
                    self.active == index,
                    drawn_selection,
                    net_lines,
                    focus_point,
                    self.hovered_point,
                    followers,
                    vis,
                    texture.as_ref(),
                    hull,
                    gpu_hull,
                    waterline,
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
            self.draw_pick_overlays(ui, cell, index);

            // Fit and maximise, in the corner of the view they act on.
            let [fit, maximise] = render::view_buttons(cell);
            self.view_button_rects[index] = [fit, maximise];
            let hud = crate::theme::IconStyle::Hud;
            if crate::theme::icon_button_at(ui, fit, crate::theme::Icon::Fit, hud, false)
                .on_hover_text(format!(
                    "Frame the model in this view ({fit_key} frames every view)"
                ))
                .clicked()
            {
                self.refit_camera(index);
            }
            let (icon, tip) = if self.maximised.is_some() {
                (crate::theme::Icon::Restore, "Restore the four views")
            } else {
                (crate::theme::Icon::Maximise, "Maximise this view")
            };
            if crate::theme::icon_button_at(ui, maximise, icon, hud, false)
                .on_hover_text(format!(
                    "{tip} ({max_key}, or double-click the view's title)"
                ))
                .clicked()
            {
                self.active = index;
                toggle_maximise = true;
            }
        }
        for (index, frame) in fresh {
            self.analysis_frames[index] = Some(frame);
            self.raster_counts[index] += 1;
        }
        for (index, key, hull) in fresh_hulls {
            self.shaded_hulls[index] = Some((key, hull));
        }
        for (index, key, mesh) in fresh_waterlines {
            self.waterline_meshes[index] = Some((key, mesh));
        }
        for (index, key, mesh) in fresh_wave_meshes {
            self.wave_meshes[index] = Some((key, mesh));
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
            waterline: self.waterline(),
            waves: self.scene_waves(),
            waves_status: self.waves_status(),
        })
    }

    /// Plan view's wave contours: only with "Contours on Plan view" on, and only a current pattern.
    fn scene_waves(&self) -> Option<render::SceneWaves<'_>> {
        if !self.perf_state.waves.plan_contours {
            return None;
        }
        let result = self.wave_current()?;
        Some(render::SceneWaves {
            result,
            contours: None,
        })
    }

    /// Plan view's note while the pattern is shown but not drawn.
    fn waves_status(&self) -> Option<&'static str> {
        if !self.perf_state.waves.plan_contours || self.wave_current().is_some() {
            None
        } else if self.wave_failure().is_some() {
            Some("Wave contours failed: see the Wave pattern view")
        } else {
            Some("Wave contours: computing…")
        }
    }

    /// Load the kept pattern's image as a texture, once per result (the image is copied only to
    /// upload it); drop it when there is none. The view draws a stale one dimmed.
    fn load_wave_texture(&mut self, ctx: &egui::Context) {
        let key = self.wave_result_rev;
        let Some(result) = self
            .waves_needed()
            .then_some(self.perf_state.wave_result.as_ref())
            .flatten()
        else {
            self.wave_texture = None;
            return;
        };
        if self.wave_texture.as_ref().is_some_and(|(k, _)| *k == key) {
            return;
        }
        let image = result.image.clone();
        let texture = ctx.load_texture("wave-pattern", image, egui::TextureOptions::LINEAR);
        self.wave_texture = Some((key, texture));
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
        index: usize,
    ) -> Option<usize> {
        // Hidden control points cannot be grabbed: a drag would move a point
        // the user cannot see.
        if !self.display.points {
            return None;
        }
        let cursor = ui.input(|i| i.pointer.interact_pos().or_else(|| i.pointer.hover_pos()))?;
        let surface = self.surface.as_ref()?;
        let pts: Vec<Vec3> = surface.points.iter().map(|p| p.p).collect();
        let vis = self.net_visibility(index);
        picking::pick_point(
            view,
            cell,
            cursor,
            &pts,
            PICK_RADIUS,
            vis,
            self.display.hidden_net,
        )
    }

    /// The control face under the cursor (see [`picking::pick_face`]).
    fn pick_face_at(
        &self,
        ui: &egui::Ui,
        view: &crate::camera::ViewTransform,
        cell: egui::Rect,
    ) -> Option<usize> {
        let cursor = ui.input(|i| i.pointer.interact_pos())?;
        let surface = self.surface.as_ref()?;
        picking::pick_face(&view.ray(cursor, cell), surface)
    }

    /// Apply what the layer panel asked for, through the typed commands (one
    /// undo step per edit). A refusal is logged as an error.
    pub fn apply_layer_action(&mut self, action: panels::LayerAction) {
        use crate::commands as c;
        use panels::LayerAction as A;
        let result = match action {
            A::None => return,
            A::Activate(layer) => c::set_active_layer(self, layer),
            A::Add => c::add_layer(self, &c::AddLayerParams::default()),
            A::Delete {
                layer,
                move_faces_to,
            } => c::delete_layer(
                self,
                &c::DeleteLayerParams {
                    layer,
                    move_faces_to,
                },
            ),
            A::Rename { layer, name } => c::rename_layer(self, layer, &name),
            A::Reorder { layer, dir } => c::move_layer(self, layer, dir),
            A::Set { layer, props } => c::set_layer(
                self,
                &c::SetLayerParams {
                    layer,
                    props,
                    reorder: None,
                },
            ),
            A::AssignFaces { faces, layer } => {
                c::assign_faces(self, &c::AssignFacesParams { faces, layer })
            }
        };
        if let Err(e) = result {
            self.log(Level::Error, e);
        }
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
        let Some(point) = self.pick_at(ui, view, cell, index) else {
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
        self.project_edit_source = None;
        self.history.snapshot(&snap);
        let mut added_on_press = false;
        if pointer_shift(ui) {
            if !self.selection.contains(point) {
                self.selection.toggle(point);
                added_on_press = true;
            }
        } else if !self.selection.contains(point) {
            self.selection.set_only(point);
        }
        let surface = self.surface.as_ref().expect("checked above");
        let connected = self
            .connected_now()
            .then(|| (self.active_connected_preset(), self.connected.reach_m));
        let plan = build_move_plan(
            surface,
            self.selection.points(),
            connected.as_ref().map(|(p, r)| (p, *r)),
        );
        self.active = index;
        if let Some((preset, reach_m)) = &connected {
            let reach = reach_m.unwrap_or_else(|| preset.reach.metres(surface));
            self.set_status(format!(
                "{} · reach {:.2} m · {} points",
                preset.name,
                reach,
                plan.weights().len()
            ));
        } else {
            self.set_status(format!(
                "dragging control point {point} · hold X, Y or Z to lock an axis"
            ));
        }
        self.drag = Some(DragState {
            point,
            plane,
            offset,
            view: index,
            original: center,
            plan,
            added_on_press,
            target: center,
            connected: connected.is_some(),
        });
    }

    /// Whether a drag now pulls neighbours along: the Connected switch,
    /// flipped while Ctrl (Cmd on macOS) is held.
    pub fn connected_now(&self) -> bool {
        self.connected.on != self.connected_flip
    }

    /// Read whether Ctrl (Cmd) is held, and re-plan a drag under way when
    /// that changes whether it pulls neighbours along. A key held for a
    /// text field is typing, never a flip.
    fn read_connected_flip(&mut self, ui: &egui::Ui) {
        let typing = self.keyboard_taken || ui.ctx().egui_wants_keyboard_input();
        self.connected_flip = !typing && ui.input(|i| i.modifiers.command);
        let now = self.connected_now();
        if !self.drag.as_ref().is_some_and(|d| d.connected != now) {
            return;
        }
        let preset = self.active_connected_preset();
        let reach_m = self.connected.reach_m;
        let (Some(drag), Some(surface)) = (self.drag.as_mut(), self.surface.as_mut()) else {
            return;
        };
        rebuild_drag_plan(drag, surface, now.then_some((&preset, reach_m)));
        drag.connected = now;
        let count = drag.plan.weights().len();
        let point = drag.point;
        if now {
            let reach = self.effective_connected_reach_m();
            self.set_status(format!(
                "{} · reach {reach:.2} m · {count} points",
                preset.name
            ));
        } else {
            self.set_status(format!(
                "dragging control point {point} · hold X, Y or Z to lock an axis"
            ));
        }
        self.bump_revision();
        self.touch_geometry();
        self.sync_cache();
    }

    /// Escape during a point drag: every point goes back where it started,
    /// the press's undo snapshot is dropped and the selection stays.
    fn cancel_drag(&mut self) {
        let Some(drag) = self.drag.take() else {
            return;
        };
        self.axis_lock = None;
        if let Some(surface) = self.surface.as_mut() {
            drag.plan.apply(surface, Vec3::ZERO);
        }
        self.history.discard_last();
        // Against the file, not the flag at the press: a save may have
        // happened since.
        self.refresh_dirty();
        self.bump_revision();
        self.touch_geometry();
        self.sync_cache();
        self.set_status("drag cancelled".to_owned());
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
        // X, Y or Z held: only that coordinate follows the cursor. A key
        // typed into a text field is text, never a lock.
        let typing = self.keyboard_taken || ui.ctx().egui_wants_keyboard_input();
        let lock = if typing {
            None
        } else {
            ui.input(crate::model_menu::held_axis)
        };
        if let Some(axis) = lock {
            target = crate::model_menu::lock_to_axis(drag.original, target, axis);
        }
        if lock != self.axis_lock {
            let connected_status = if lock.is_none() && drag.connected {
                let preset = self.active_connected_preset();
                let reach = self.effective_connected_reach_m();
                Some(format!(
                    "{} · reach {:.2} m · {} points",
                    preset.name,
                    reach,
                    drag.plan.weights().len()
                ))
            } else {
                None
            };
            self.set_status(match lock {
                Some(axis) => format!(
                    "dragging control point {point} along {} (locked)",
                    crate::model_menu::axis_name(axis)
                ),
                None => connected_status.unwrap_or_else(|| {
                    format!("dragging control point {point} · hold X, Y or Z to lock an axis")
                }),
            });
        }
        self.axis_lock = lock;
        if let (Some(drag), Some(surface)) = (self.drag.as_mut(), self.surface.as_mut()) {
            drag.target = target;
            drag.plan.apply_to(surface, point, target);
        }
        self.dirty = true;
        self.autosave.note_dirty();
        self.bump_revision();
        self.touch_geometry();
        // Refine now (at the drag level), so the dragged view shows the point
        // under the cursor this frame instead of the next.
        self.sync_cache();
    }

    fn scale_connected_drag_reach(&mut self, scroll_y: f32) {
        let preset = self.active_connected_preset();
        let (Some(drag), Some(surface)) = (self.drag.as_mut(), self.surface.as_mut()) else {
            return;
        };
        let current = self
            .connected
            .reach_m
            .unwrap_or_else(|| preset.reach.metres(surface))
            .max(1e-3);
        let next = (current * 1.1_f64.powf(scroll_y as f64 / 40.0)).clamp(1e-3, 1e4);
        self.connected.reach_m = Some(next);
        rebuild_drag_plan(drag, surface, Some((&preset, Some(next))));
        let count = drag.plan.weights().len();
        self.set_status(format!(
            "{} · reach {next:.2} m · {count} points",
            preset.name
        ));
        self.dirty = true;
        self.autosave.note_dirty();
        self.bump_revision();
        self.touch_geometry();
        self.sync_cache();
    }

    fn hovered_followers(&mut self) -> &[(usize, f64)] {
        let Some(hovered) = self.hovered_point else {
            return &[];
        };
        self.sync_cache();
        let Some(surface) = self.surface.as_ref() else {
            return &[];
        };
        let Some(cache) = self.cache.as_ref() else {
            return &[];
        };
        let preset = self.active_connected_preset();
        let reach_m = self
            .connected
            .reach_m
            .unwrap_or_else(|| preset.reach.metres(surface));
        let seeds = if self.selection.contains(hovered) {
            self.selection.points().to_vec()
        } else {
            vec![hovered]
        };
        let key = HoverFollowersKey {
            hovered,
            revision: self.revision,
            preset,
            reach_bits: reach_m.to_bits(),
            seeds,
        };
        let stale = self
            .hover_followers
            .as_ref()
            .is_none_or(|(cached, _)| *cached != key);
        if stale {
            let weights = seacraft_core::connected::follower_weights(
                surface,
                &cache.net_lines,
                &key.seeds,
                &key.preset,
                reach_m,
            );
            self.hover_followers = Some((key, weights));
        }
        self.hover_followers
            .as_ref()
            .map_or(&[], |(_, w)| w.as_slice())
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

    /// The wave pattern's inputs: the geometry and level, the floating condition (its draft) and
    /// every wave setting but `show`.
    fn waves_key(&self) -> u64 {
        revision_key(&[
            self.revision,
            self.hydro_rev,
            crate::waves::settings_key(&self.perf_state.waves),
        ])
    }

    /// The wave-drag curve's inputs: the geometry and level, the floating condition and the
    /// catamaran spacing, but not the speed.
    fn wave_drag_key(&self) -> u64 {
        revision_key(&[
            self.revision,
            self.hydro_rev,
            crate::waves::drag_key(self.perf_state.waves.cat_spacing_m),
        ])
    }

    /// The Heel view's condition: the geometry and level, the dock's floating condition and KG,
    /// the start (a loading condition's contents through the project's revision) and the trial
    /// weights.
    fn heel_key(&self) -> u64 {
        use crate::heel_view::HeelStart;
        let start = match self.heel.start {
            HeelStart::Dock => [0, 0, 0],
            HeelStart::Condition(i) => [1, i as u64, self.project_rev],
        };
        revision_key(&[
            self.revision,
            self.hydro_rev,
            start[0],
            start[1],
            start[2],
            crate::heel_view::weights_key(&self.heel.weights),
        ])
    }

    /// The drawings' hull: the geometry and its refinement.
    fn heel_hull_key(&self) -> u64 {
        revision_key(&[self.revision, 0x4845_454c])
    }

    /// The hull and outlines for the drawings, when current.
    fn heel_hull_current(&self) -> Option<&crate::heel_view::HeelHull> {
        if !self.heel_hull_track.is_current(self.heel_hull_key()) {
            return None;
        }
        self.heel_hull.as_ref()
    }

    /// The hydrostatics' hull, when it is current (a job reuses it rather than refine again).
    fn hydro_hull_current(&self) -> Option<Arc<Hull>> {
        self.hydro
            .hull
            .clone()
            .filter(|_| self.hydro.track.is_current(self.hydro_key()))
    }

    fn start_heel_hull(&mut self) {
        let Some(surface) = self.surface.clone() else {
            return;
        };
        let subdivision = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let hull = self.hydro_hull_current();
        self.heel_hull_track.start(self.heel_hull_key(), move || {
            Ok(crate::heel_view::compute_hull(&surface, subdivision, hull))
        });
    }

    /// The exact state's key: the condition's and the heel shown.
    fn heel_exact_key(&self) -> u64 {
        revision_key(&[self.heel_key(), self.heel.heel_deg.to_bits()])
    }

    /// Whether the Heel view is open (it computes only then).
    pub fn heel_needed(&self) -> bool {
        self.analysis.open == Some(crate::analysis_views::AnalysisView::Heel)
    }

    /// Open the Heel view in the drawer, or close it if it is the view shown.
    pub fn set_heel_view_open(&mut self, open: bool) {
        use crate::analysis_views::AnalysisView;
        if open {
            self.open_analysis_view(AnalysisView::Heel);
        } else if self.analysis.open == Some(AnalysisView::Heel) {
            self.close_analysis_view();
        }
    }

    /// The Heel view's state.
    pub fn heel_state(&self) -> &crate::heel_view::HeelViewState {
        &self.heel
    }

    /// Replace the Heel view's state (tests and the control channel; the panel edits it in place).
    pub fn set_heel_state(&mut self, state: crate::heel_view::HeelViewState) {
        self.heel = state;
        self.check_heel_start();
    }

    /// The Heel view's condition, `Some` only while the view is open and the result is for the
    /// current model, floating condition, start and weights. Every drawing goes through this gate.
    pub fn heel_current(&self) -> Option<&crate::heel_view::HeelSweepOut> {
        if !self.heel_needed() || !self.heel_sweep_track.is_current(self.heel_key()) {
            return None;
        }
        self.heel_sweep.as_ref()
    }

    /// The exact state at the heel shown, when current.
    pub fn heel_exact_current(&self) -> Option<&seacraft_core::heel::HeelState> {
        if self.heel_current().is_none() || !self.heel_exact_track.is_current(self.heel_exact_key())
        {
            return None;
        }
        self.heel_exact.as_ref()
    }

    /// A Heel result is kept but no longer for the current inputs (drawn dimmed).
    pub fn heel_stale(&self) -> bool {
        self.heel_sweep.is_some() && !self.heel_sweep_track.is_current(self.heel_key())
    }

    /// A Heel job is running.
    pub fn heel_busy(&self) -> bool {
        self.heel_sweep_track.busy() || self.heel_exact_track.busy() || self.heel_hull_track.busy()
    }

    /// Where the Heel view's drawings were last drawn, and how they map to the model (tests).
    pub fn heel_drawn(&self) -> Option<crate::heel_view::DrawnViews> {
        self.heel_drawn
    }

    /// Advance the settle animation, or start one when a settle is pending and the condition is
    /// current: eased over half a second between sweep samples, a picture, not a roll.
    fn heel_animate(&mut self, now: f64) -> bool {
        use crate::heel_view::Settle;
        if self.heel.hold {
            self.heel_settle_pending = false;
            self.heel.anim = None;
        }
        if self.heel_settle_pending && self.heel.grab.is_none() {
            if let Some(out) = self.heel_current() {
                let from = self.heel.heel_deg;
                let side = if from == 0.0 { out.mass.tcg() } else { from };
                let to = out.equilibrium.heel_for_side(side);
                self.heel_settle_pending = false;
                if let Some(to) = to.filter(|to| (to - from).abs() > 1e-9) {
                    self.heel.anim = Some(Settle {
                        from,
                        to,
                        started: now,
                    });
                }
            }
        }
        let Some(a) = self.heel.anim else {
            return false;
        };
        let t = ((now - a.started) / crate::heel_view::SETTLE_SECONDS).clamp(0.0, 1.0);
        if t >= 1.0 {
            self.heel.heel_deg = a.to;
            self.heel.anim = None;
        } else {
            let ease = t * t * (3.0 - 2.0 * t);
            self.heel.heel_deg = a.from + (a.to - a.from) * ease;
        }
        true
    }

    /// Append the trial weights to loading condition `i`, through undo: the only way a trial
    /// weight reaches the model. The view then starts from that condition, without the weights.
    fn heel_add_to_condition(&mut self, i: usize) {
        if self.heel.weights.is_empty() || i >= self.project.loading_conditions.len() {
            return;
        }
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let snap = Snapshot::project_edit(
            surface,
            self.hydro.options.draft,
            self.project.clone(),
            (self.hydro.kg, self.hydro.kg_estimate),
        );
        self.history.snapshot(&snap);
        self.project_edit_source = None;
        let weights = std::mem::take(&mut self.heel.weights);
        let n = weights.len();
        let cond = &mut self.project.loading_conditions[i];
        cond.items.extend(weights);
        let name = cond.name.clone();
        self.heel.selected = None;
        self.heel.entry = None;
        self.heel.start = crate::heel_view::HeelStart::Condition(i);
        self.project_changed();
        self.refresh_dirty();
        self.set_status(format!(
            "Added {n} trial weight{} to {name}",
            if n == 1 { "" } else { "s" }
        ));
    }

    /// A sweep job is running (the condition, not the drawings' hull).
    pub fn heel_sweep_busy(&self) -> bool {
        self.heel_sweep_track.busy()
    }

    /// Why the Heel view's condition failed for the current inputs, if it did.
    pub fn heel_failure(&self) -> Option<&str> {
        self.heel_sweep_track.failure(self.heel_key())
    }

    /// A start on a loading condition that is gone falls back to the bare hull, and says so.
    fn check_heel_start(&mut self) {
        if let crate::heel_view::HeelStart::Condition(i) = self.heel.start {
            if i >= self.project.loading_conditions.len() {
                self.heel.start = crate::heel_view::HeelStart::Dock;
                self.set_status("Heel view: that loading condition is gone; showing the bare hull");
            }
        }
    }

    /// The inputs of the Heel view's sweep job; `Err` when there is nothing to compute (no model,
    /// no KG for the bare hull).
    fn heel_sweep_input(&self) -> Result<crate::heel_view::SweepInput, String> {
        use crate::heel_view::{HeelStart, StartInput};
        let surface = self.surface.clone().ok_or("no model loaded")?;
        let subdivision = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let start = match self.heel.start {
            HeelStart::Dock => StartInput::Dock {
                kg: self.hydro.kg.ok_or("Set KG in the Hydrostatics dock")?,
                kg_estimate: self.kg_is_estimate(),
                lcg: if self.hydro.free_trim {
                    self.hydro.lcg
                } else {
                    None
                },
            },
            HeelStart::Condition(i) => {
                let c = self
                    .project
                    .loading_conditions
                    .get(i)
                    .ok_or("that loading condition is gone")?;
                StartInput::Condition(Box::new(self.project.with_windage(c)))
            }
        };
        let hull = self
            .heel_hull_current()
            .map(|h| h.hull.clone())
            .or_else(|| self.hydro_hull_current());
        let mut opts = self.hydro.options.clone();
        opts.subdivision = subdivision;
        Ok(crate::heel_view::SweepInput {
            surface,
            hull,
            subdivision,
            opts,
            start,
            weights: self.heel.weights.clone(),
        })
    }

    fn start_heel_sweep(&mut self) {
        let key = self.heel_key();
        match self.heel_sweep_input() {
            Ok(input) => self
                .heel_sweep_track
                .start(key, move || crate::heel_view::compute_sweep(input)),
            Err(e) => self.heel_sweep_track.fail(key, e),
        }
    }

    fn start_heel_exact(&mut self) {
        let Some(out) = self.heel_current().cloned() else {
            return;
        };
        let (key, heel) = (self.heel_exact_key(), self.heel.heel_deg);
        self.heel_exact_track
            .start(key, move || crate::heel_view::compute_exact(&out, heel));
    }

    /// Compute the Heel view's condition (and the exact state at the heel shown) on this thread
    /// (tests and scripts).
    pub fn compute_heel_blocking(&mut self) -> Result<(), String> {
        self.check_heel_start();
        if self.heel_hull_current().is_none() {
            let surface = self.surface.as_ref().ok_or("no model loaded")?;
            let subdivision = seacraft_core::subdiv::SubdivisionOptions {
                level: self.level,
                mode: self.mode,
            };
            let out =
                crate::heel_view::compute_hull(surface, subdivision, self.hydro_hull_current());
            self.heel_hull = Some(out);
            self.heel_hull_track.stamp(self.heel_hull_key());
        }
        let key = self.heel_key();
        let out = match self
            .heel_sweep_input()
            .and_then(crate::heel_view::compute_sweep)
        {
            Ok(out) => out,
            Err(e) => {
                self.heel_sweep_track.fail(key, e.clone());
                return Err(e);
            }
        };
        let exact = crate::heel_view::compute_exact(&out, self.heel.heel_deg);
        self.heel_sweep = Some(out);
        self.heel_sweep_track.stamp(key);
        let key = self.heel_exact_key();
        match exact {
            Ok(s) => {
                self.heel_exact = Some(s);
                self.heel_exact_track.stamp(key);
            }
            Err(e) => self.heel_exact_track.fail(key, e),
        }
        Ok(())
    }

    /// Whether the wave pattern and its drag curve are computed: while the Wave pattern view is
    /// open or the Plan view contours are on.
    pub fn waves_needed(&self) -> bool {
        self.analysis.open == Some(crate::analysis_views::AnalysisView::Waves)
            || self.perf_state.waves.plan_contours
    }

    /// The wave-drag curve, `Some` only while needed and computed for the current model, floating
    /// condition and catamaran spacing.
    pub fn wave_drag(&self) -> Option<&crate::waves::WaveDrag> {
        if !self.waves_needed() || !self.wave_drag_track.is_current(self.wave_drag_key()) {
            return None;
        }
        self.perf_state.wave_drag.as_ref()
    }

    /// Why the wave-drag curve failed for the current inputs, if it did.
    pub fn wave_drag_failure(&self) -> Option<&str> {
        self.wave_drag_track.failure(self.wave_drag_key())
    }

    /// Compute the wave-drag curve on this thread (tests and scripts).
    pub fn compute_wave_drag_blocking(&mut self) -> Result<(), String> {
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        let subdivision = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let key = self.wave_drag_key();
        match crate::waves::compute_wave_drag(
            surface,
            subdivision,
            self.hydro.options.draft,
            self.hydro.options.water_density,
            self.perf_state.waves.cat_spacing_m,
        ) {
            Ok(d) => {
                self.perf_state.wave_drag = Some(d);
                self.wave_drag_track.stamp(key);
                Ok(())
            }
            Err(e) => {
                self.wave_drag_track.fail(key, e.clone());
                Err(e)
            }
        }
    }

    fn start_wave_drag_compute(&mut self) {
        let Some(surface) = self.surface.clone() else {
            return;
        };
        let subdivision = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let draft = self.hydro.options.draft;
        let rho = self.hydro.options.water_density;
        let spacing = self.perf_state.waves.cat_spacing_m;
        let key = self.wave_drag_key();
        self.wave_drag_track.start(key, move || {
            crate::waves::compute_wave_drag(&surface, subdivision, draft, rho, spacing)
        });
    }

    pub fn wave_settings(&self) -> &crate::waves::WaveSettings {
        &self.perf_state.waves
    }

    /// Replace the wave settings; a changed result is recomputed after the usual quiet period.
    pub fn set_wave_settings(&mut self, settings: crate::waves::WaveSettings) {
        self.perf_state.waves = settings;
        self.last_change = Some(std::time::Instant::now());
    }

    /// Open the Wave pattern view in the drawer (it computes while open), or close it if it is the
    /// view shown.
    pub fn set_wave_view_open(&mut self, open: bool) {
        use crate::analysis_views::AnalysisView;
        if open {
            self.open_analysis_view(AnalysisView::Waves);
        } else if self.analysis.open == Some(AnalysisView::Waves) {
            self.close_analysis_view();
        }
    }

    /// The wave pattern to draw, save or report: `Some` only while it is needed
    /// ([`App::waves_needed`]) and computed for the current model, floating condition and settings.
    /// Every view goes through this gate.
    pub fn wave_current(&self) -> Option<&crate::waves::WaveResult> {
        if !self.waves_needed() || !self.waves_track.is_current(self.waves_key()) {
            return None;
        }
        self.perf_state.wave_result.as_ref()
    }

    /// Why the wave pattern failed for the current inputs, if it did.
    pub fn wave_failure(&self) -> Option<&str> {
        self.waves_track.failure(self.waves_key())
    }

    /// The wave map's texture, when one is loaded for the kept pattern (test hook).
    pub fn wave_texture_id(&self) -> Option<egui::TextureId> {
        self.wave_texture.as_ref().map(|(_, t)| t.id())
    }

    /// Whether a wave-pattern job is running (test hook).
    pub fn waves_busy(&self) -> bool {
        self.waves_track.busy()
    }

    /// A wave job is in flight for the current settings, not an older key (test hook).
    pub fn waves_running_current(&self) -> bool {
        self.waves_track.running_for() == Some(self.waves_key())
    }

    /// Compute the wave pattern on this thread (tests and scripts).
    pub fn compute_waves_blocking(&mut self) -> Result<(), String> {
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        let subdivision = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let key = self.waves_key();
        match crate::waves::compute_waves(
            surface,
            subdivision,
            self.hydro.options.draft,
            self.hydro.options.water_density,
            &self.perf_state.waves,
        ) {
            Ok(w) => {
                self.perf_state.wave_result = Some(w);
                self.wave_result_rev += 1;
                self.waves_track.stamp(key);
                Ok(())
            }
            Err(e) => {
                self.waves_track.fail(key, e.clone());
                Err(e)
            }
        }
    }

    fn start_waves_compute(&mut self) {
        let Some(surface) = self.surface.clone() else {
            return;
        };
        let subdivision = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let draft = self.hydro.options.draft;
        let rho = self.hydro.options.water_density;
        let settings = self.perf_state.waves.clone();
        let key = self.waves_key();
        self.waves_track.start(key, move || {
            crate::waves::compute_waves(&surface, subdivision, draft, rho, &settings)
        });
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
            // The wave-drag curve takes the hydrostatics' quiet: it is quick, and a drag on the
            // view's speed does not re-key it.
            if self.waves_needed() && self.wave_drag_track.due(self.wave_drag_key()) {
                if hydro_quiet {
                    self.start_wave_drag_compute();
                } else {
                    waiting = true;
                }
            }
            let loading_due = !self.project.loading_conditions.is_empty()
                && self.loading_track.due(self.loading_key());
            let resistance_due = self.resistance_track.due(self.resistance_key());
            // The wave pattern recomputes live while the speed is scrubbed: the hydrostatics'
            // quiet. One job at a time: a job in flight for an older key runs to its end (a
            // thread cannot be stopped) and its result is discarded, then the newest key starts.
            // Meanwhile the view shows the last stored result dimmed.
            let waves_key = self.waves_key();
            if self.waves_needed() && self.waves_track.due(waves_key) {
                if hydro_quiet && self.waves_track.ready(waves_key) {
                    self.start_waves_compute();
                } else {
                    waiting = true;
                }
            }
            // The Heel view: one sweep at a time, on the hydrostatics' quiet; the exact state at
            // the heel shown once the sweep is current.
            self.check_heel_start();
            if self.heel_needed() {
                let hull_key = self.heel_hull_key();
                if self.heel_hull_track.due(hull_key) {
                    if hydro_quiet && self.heel_hull_track.ready(hull_key) {
                        self.start_heel_hull();
                    } else {
                        waiting = true;
                    }
                }
                let key = self.heel_key();
                if self.heel_sweep_track.due(key) {
                    if hydro_quiet && self.heel_sweep_track.ready(key) {
                        self.start_heel_sweep();
                    } else {
                        waiting = true;
                    }
                }
                let exact = self.heel_exact_key();
                // The exact state waits for the hand to rest: no settle running, and no drag
                // or one held still for a moment (the drawing shows sweep samples meanwhile).
                let resting = self.heel.anim.is_none()
                    && (self.heel.grab.is_none()
                        || self
                            .heel_moved
                            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(150)));
                if resting && self.heel_current().is_some() && self.heel_exact_track.due(exact) {
                    if self.heel_exact_track.ready(exact) {
                        self.start_heel_exact();
                    } else {
                        waiting = true;
                    }
                }
            }
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
            || self.waves_track.busy()
            || self.wave_drag_track.busy()
            || self.heel_sweep_track.busy()
            || self.heel_exact_track.busy()
            || self.heel_hull_track.busy()
            || self.raos_track.busy()
            || self.analysis.busy();
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
            Some(Err(e)) => self.log(Level::Error, format!("Hydrostatics failed: {e}")),
            None => {}
        }
        match self.hydro.curves_track.poll(key) {
            Some(Ok(out)) => self.install_curves(out, key),
            Some(Err(e)) => self.log(Level::Error, format!("GZ curves failed: {e}")),
            None => {}
        }
        match self.loading_track.poll(self.loading_key()) {
            Some(Ok(r)) => self.install_loading(r),
            Some(Err(e)) => self.log(Level::Error, format!("Loading compute failed: {e}")),
            None => {}
        }
        match self.damage_track.poll(self.damage_key()) {
            Some(Ok(r)) => self.damage_result = Some(r),
            Some(Err(e)) => self.log(Level::Error, format!("Damage compute failed: {e}")),
            None => {}
        }
        match self.resistance_track.poll(self.resistance_key()) {
            Some(Ok(r)) => self.perf_state.resistance = Some(r),
            Some(Err(e)) => self.log(Level::Error, format!("Resistance compute failed: {e}")),
            None => {}
        }
        match self.waves_track.poll(self.waves_key()) {
            Some(Ok(w)) => {
                self.perf_state.wave_result = Some(w);
                self.wave_result_rev += 1;
            }
            Some(Err(e)) => self.log(Level::Error, format!("Wave pattern failed: {e}")),
            None => {}
        }
        match self.heel_sweep_track.poll(self.heel_key()) {
            Some(Ok(out)) => self.heel_sweep = Some(out),
            Some(Err(e)) => self.log(Level::Error, format!("Heel view failed: {e}")),
            None => {}
        }
        match self.heel_hull_track.poll(self.heel_hull_key()) {
            Some(Ok(h)) => self.heel_hull = Some(h),
            Some(Err(e)) => self.log(Level::Error, format!("Heel view hull failed: {e}")),
            None => {}
        }
        // A failed exact solve leaves the drawing on the nearest sweep sample: nothing to report.
        if let Some(Ok(s)) = self.heel_exact_track.poll(self.heel_exact_key()) {
            self.heel_exact = Some(s);
        }
        match self.wave_drag_track.poll(self.wave_drag_key()) {
            Some(Ok(d)) => self.perf_state.wave_drag = Some(d),
            Some(Err(e)) => self.log(Level::Error, format!("Wave drag curve failed: {e}")),
            None => {}
        }
        match self.raos_track.poll(self.raos_key()) {
            Some(Ok((r, irr))) => {
                self.perf_state.raos = Some(r);
                self.perf_state.sea_result = irr;
            }
            Some(Err(e)) => self.log(Level::Error, format!("Seakeeping compute failed: {e}")),
            None => {}
        }
        self.poll_analysis_views();
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
    /// one, otherwise the job measures first. `None` without a model or a KG.
    fn curves_inputs(&self) -> Option<CurvesInputs> {
        let surface = self.surface.as_ref()?;
        let kg = self.hydro.kg?;
        let measured = match (&self.hydro.hull, &self.hydro.result) {
            (Some(hull), Some(h)) if self.hydro.track.is_current(self.hydro_key()) => {
                Some((hull.clone(), h.displacement_t))
            }
            _ => None,
        };
        Some(CurvesInputs {
            hydro: self.hydro.inputs(surface, self.level, self.mode),
            kg,
            measured,
        })
    }

    /// Compute the GZ cross curves on a job ("GZ curves", F5).
    fn start_curves_job(&mut self) {
        if self.refuse_curves_without_kg() {
            return;
        }
        let Some(inputs) = self.curves_inputs() else {
            return;
        };
        let key = self.hydro_key();
        self.set_status("computing GZ cross curves…".to_owned());
        self.hydro
            .curves_track
            .start(key, move || Ok(compute_cross_curves_for(inputs)));
    }

    /// The GZ curves need a KG: without one, say so instead of computing.
    /// Returns true when the curves were refused.
    fn refuse_curves_without_kg(&mut self) -> bool {
        if self.surface.is_some() && self.hydro.kg.is_none() {
            self.set_status(format!("GZ curves: {KG_UNSET_HINT}"));
            return true;
        }
        false
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
        self.set_status(if self.hydro.curves.is_none() {
            "GZ curves: nothing is submerged at this draft".to_owned()
        } else if free {
            format!(
                "GZ cross curves for {:.2} t and ±30 %, free to trim about {:.3} m",
                out.displacement_t,
                out.lcg.unwrap_or_default()
            )
        } else {
            format!("GZ cross curves for {:.2} t and ±30 %", out.displacement_t)
        });
    }

    /// F5: recompute every stale result that is shown — the automatic ones
    /// at once, without the debounce, and the heavy ones of the open tab and of
    /// the view in the drawer.
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
        // One wave job at a time, as in `run_jobs`. A failed key is tried again: now, or, with
        // an older job in flight, by `run_jobs` when that ends.
        if self.waves_needed() {
            let key = self.waves_key();
            self.waves_track.retry(key);
            if self.waves_track.ready(key) {
                self.start_waves_compute();
            }
        }
        // The drawer's view and the dock's tab are each on screen on their
        // own: the drawer stays when the dock is hidden, and does not cover
        // the tab when both are shown.
        if let Some(view) = self.analysis.open.filter(|v| v.uses_slot()) {
            // Stale or empty.
            if self.analysis.slot(view).track.due(self.analysis_key(view)) {
                self.start_analysis_view(view);
            }
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

    /// A wave pattern is kept but no longer matches the inputs (recomputed automatically while
    /// needed).
    fn waves_stale(&self) -> bool {
        self.waves_needed()
            && self.perf_state.wave_result.is_some()
            && !self.waves_track.is_current(self.waves_key())
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
            (self.waves_stale(), "wave pattern"),
            (self.open_view_stale(), "analysis view"),
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
            DesignDock::Performance => {
                self.resistance_stale() || self.raos_stale() || self.waves_stale()
            }
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
                ("Wave pattern", self.waves_track.failure(self.waves_key())),
                (
                    "Wave drag curve",
                    self.wave_drag_track.failure(self.wave_drag_key()),
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
            || self.waves_track.busy()
            || self.wave_drag_track.busy()
            || self.raos_track.busy()
            || self.analysis.busy()
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
        self.project_edit_source = None;
        // A project that carries a KG brings it to the dock; one without
        // leaves the dock's KG as it is.
        let carries_kg = project.kg.is_some();
        self.project = project;
        if carries_kg {
            self.adopt_project_kg();
            self.hydro_inputs_changed();
        }
        self.project_error = None;
        self.dirty = true;
        self.autosave.note_dirty();
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
    fn report_options(
        &self,
    ) -> Option<(seacraft_core::report::ReportOptions, Option<ReportWaves>)> {
        let surface = self.surface.as_ref()?;
        let mut opts = self
            .project
            .report
            .apply(seacraft_core::report::ReportOptions::from_surface(surface));
        opts.level = self.level;
        opts.mode = self.mode;
        // The wave pattern goes in while it is shown, with the view's settings, at the report's
        // draft. Its domain and wash line in metres need the waterline at that draft: a current
        // pattern's or hydrostatics' when either is at that draft (a stale one may be of another
        // hull), else measured in the report job.
        let at_draft = |d: f64| (d - opts.draft).abs() <= 1e-9;
        let lengths = self
            .wave_current()
            .filter(|r| at_draft(r.draft))
            .map(|r| (r.length_wl, r.beam_wl))
            .or_else(|| {
                self.hydro
                    .result
                    .as_ref()
                    // Upright only (as measured, so a free trim counts): heeled or trimmed, the
                    // waterline lies on the tilted plane, while the report's pattern is upright.
                    .filter(|h| {
                        self.hydro.track.is_current(self.hydro_key())
                            && at_draft(h.draft)
                            && h.heel_deg == 0.0
                            && h.trim_m == 0.0
                    })
                    .map(|h| (h.lwl, h.bwl))
            });
        let waves = self.waves_needed().then(|| ReportWaves {
            settings: self.perf_state.waves.clone(),
            lengths,
        });
        Some((opts, waves))
    }

    /// Write the design report on this thread (tests and scripts).
    pub fn write_report_blocking(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (mut opts, waves) = self.report_options().ok_or("no model loaded")?;
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        opts.waves = waves.map(|w| w.into_report(surface, &opts));
        let html =
            seacraft_core::report::design_report_with_project(surface, &opts, &self.project)?;
        seacraft_core::io::write_atomic(path, html)
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        self.set_status(format!("wrote report {}", path.display()));
        Ok(())
    }

    /// Start writing the design report on a worker thread; the status bar
    /// reports when it is done (a failure is logged). Fails when nothing
    /// could be started.
    fn start_report(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (Some((mut opts, waves)), Some(surface)) =
            (self.report_options(), self.surface.clone())
        else {
            let msg = "No model loaded.".to_owned();
            self.log(Level::Error, msg.clone());
            return Err(msg);
        };
        let project = self.project.clone();
        let path = path.to_path_buf();
        self.set_status(format!("writing report {}…", path.display()));
        let target = path.clone();
        let job = Job::spawn(self.revision, move || {
            // Measuring the waterline, when it comes to that, is slow: here, off the window.
            opts.waves = waves.map(|w| w.into_report(&surface, &opts));
            seacraft_core::report::design_report_with_project(&surface, &opts, &project).and_then(
                |html| {
                    seacraft_core::io::write_atomic(&target, html)
                        .map_err(|e| format!("cannot write {}: {e}", target.display()))
                },
            )
        });
        self.report_job = Some((path, job));
        Ok(())
    }

    /// Collect a finished report job, if any.
    fn poll_report(&mut self, ctx: &egui::Context) {
        let Some((path, job)) = &self.report_job else {
            return;
        };
        let (level, msg) = match job.poll() {
            crate::jobs::Poll::Running => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            crate::jobs::Poll::Done(()) => {
                (Level::Info, format!("wrote report {}", path.display()))
            }
            crate::jobs::Poll::Failed(e) => (Level::Error, format!("report failed: {e}")),
        };
        self.log(level, msg);
        self.report_job = None;
    }

    /// View > Messages: the log, newest first, with Clear.
    fn messages_window(&mut self, ui: &mut egui::Ui) {
        if !self.show_messages {
            return;
        }
        match crate::messages::messages_window(ui.ctx(), &self.messages) {
            MessagesAction::None => {}
            MessagesAction::Clear => self.messages.clear(),
            MessagesAction::Close => self.show_messages = false,
        }
    }

    // ---- builder templates -----------------------------------------------

    /// The key of the templates preview: the model, the options, and the name and draft that
    /// the sheets print.
    fn templates_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let (name, draft) = self.surface.as_ref().map_or((0, 0), |s| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.settings.name.hash(&mut h);
            (h.finish(), s.settings.draft.to_bits())
        });
        revision_key(&[self.revision, self.templates_rev, name, draft])
    }

    /// Collect the preview job and start the next one when it is due: never while a point is
    /// dragged, and 300 ms after the geometry last moved.
    fn drive_templates_preview(&mut self, ctx: &egui::Context) {
        let key = self.templates_key();
        if let Some(Ok(result)) = self.templates_track.poll(key) {
            self.templates_result = Some(result);
        }
        let quiet = self
            .last_change
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(300));
        if self.templates_track.due(key) && self.drag.is_none() {
            match (&self.surface, quiet) {
                (Some(surface), true) => {
                    let surface = surface.clone();
                    let (level, mode) = (self.level, self.mode);
                    let params = self.templates_params.clone();
                    let format = params.pdf_format();
                    self.templates_track.start(key, move || {
                        crate::commands::templates_in(&surface, level, mode, &params, format)
                    });
                }
                (Some(_), false) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                (None, _) => {}
            }
        }
        if self.templates_track.busy() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Where the preview stands for `key`.
    fn templates_status(&self, key: u64) -> panels::TemplatesStatus<'_> {
        if let Some(reason) = self.templates_track.failure(key) {
            return panels::TemplatesStatus::Failed(reason);
        }
        match &self.templates_result {
            Some(r) if self.templates_track.is_current(key) => panels::TemplatesStatus::Ready {
                pages: r.pages.unwrap_or(0),
                sheets: r.rows.len(),
                warnings: &r.warnings,
            },
            _ => panels::TemplatesStatus::Working,
        }
    }

    /// The File > Builder templates window.
    fn templates_window(&mut self, ui: &mut egui::Ui) {
        self.poll_templates_export(ui.ctx());
        if !self.show_templates {
            return;
        }
        if self.surface.is_none() {
            self.show_templates = false;
            return;
        }
        self.drive_templates_preview(ui.ctx());
        let key = self.templates_key();
        let mut params = self.templates_params.clone();
        let mut open = true;
        let mut action = None;
        let status = self.templates_status(key);
        egui::Window::new("Builder templates")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(440.0)
            // Room for every option and Export: egui's default window height is
            // less, and the scroll area would hide the button below the fold.
            .default_height(640.0)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(640.0)
                    .show(ui, |ui| {
                        action = panels::templates_options(ui, &mut params, &status);
                    });
            });
        if params != self.templates_params {
            self.set_templates_params(params);
        }
        if action == Some(panels::TemplatesAction::Export) {
            self.open_dialog(DialogKind::Templates);
        }
        self.show_templates = open;
    }

    /// What the panels measurement depends on: the model, its name (the patterns' title) and
    /// the options.
    fn panels_key(&self) -> u64 {
        use std::hash::{Hash, Hasher};
        let name = self.surface.as_ref().map_or(0, |s| {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            s.settings.name.hash(&mut h);
            h.finish()
        });
        revision_key(&[self.revision, self.panels_rev, name])
    }

    /// Collect the measurement and start the next when it is due: one job at a time (laying
    /// panels flat is slow and a superseded job cannot be stopped), never while a point is
    /// being dragged, and only once edits have paused.
    fn drive_panels_preview(&mut self, ctx: &egui::Context) {
        let key = self.panels_key();
        if let Some(Ok(result)) = self.panels_track.poll(key) {
            // Other sheets than those drawn (other layers, another model): frame them whole,
            // and drop a pick that may no longer name the same panel.
            let drawn = self.panels_result.as_ref().map(|r| r.set.sheets.len());
            if drawn != Some(result.set.sheets.len()) {
                self.panels_view.fit_all();
                self.panels_selected = None;
            }
            self.panels_result = Some(result);
        }
        let quiet = self
            .last_change
            .is_none_or(|t| t.elapsed() >= std::time::Duration::from_millis(300));
        if self.panels_track.ready(key) && self.drag.is_none() {
            match (&self.surface, quiet) {
                (Some(surface), true) => {
                    let surface = surface.clone();
                    let (level, mode, revision) = (self.level, self.mode, self.revision);
                    let params = self.panels_params.clone();
                    self.panels_track.start(key, move || {
                        crate::commands::panels_in(&surface, level, mode, &params, revision)
                    });
                }
                (Some(_), false) => {
                    ctx.request_repaint_after(std::time::Duration::from_millis(100))
                }
                (None, _) => {}
            }
        }
        if self.panels_track.busy() || self.panels_track.due(key) {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }
    }

    /// Where the measurement stands for the current options and model.
    fn panels_status(&self) -> builder_panels::PanelsStatus<'_> {
        let key = self.panels_key();
        if let Some(reason) = self.panels_track.failure(key) {
            return builder_panels::PanelsStatus::Failed(reason);
        }
        match &self.panels_result {
            Some(r) if self.panels_track.is_current(key) => {
                builder_panels::PanelsStatus::Ready(&r.set)
            }
            Some(r) => builder_panels::PanelsStatus::Stale(&r.set),
            None => builder_panels::PanelsStatus::Working,
        }
    }

    /// The File > Builder panels window.
    fn panels_window(&mut self, ui: &mut egui::Ui) {
        self.poll_panels_export(ui.ctx());
        if !self.show_panels {
            return;
        }
        if self.surface.is_none() {
            self.show_panels = false;
            return;
        }
        self.drive_panels_preview(ui.ctx());
        let layers: Vec<String> = self
            .surface
            .as_ref()
            .map(|s| s.layers.iter().map(|l| l.name.clone()).collect())
            .unwrap_or_default();
        let mut params = self.panels_params.clone();
        let mut open = true;
        let mut action = None;
        let selected = self.panels_selected;
        let mut view = std::mem::take(&mut self.panels_view);
        let mut clicked = None;
        let mut export = None;
        let block = self.panels_export_block();
        let status = self.panels_status();
        egui::Window::new("Builder panels")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(1040.0)
            .default_height(720.0)
            .show(ui.ctx(), |ui| {
                // Options on the left; the list and the drawing, which want the room, beside them.
                egui::Panel::left("panels_options")
                    .resizable(false)
                    .default_size(300.0)
                    .show(ui, |ui| {
                        egui::ScrollArea::vertical()
                            .id_salt("panels_options_scroll")
                            .show(ui, |ui| {
                                builder_panels::panels_options(ui, &mut params, &layers);
                                ui.separator();
                                export = builder_panels::panels_export(
                                    ui,
                                    &mut params,
                                    block.as_deref(),
                                );
                            });
                    });
                egui::CentralPanel::default().show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .id_salt("panels_list_scroll")
                        .max_height(240.0)
                        .show(ui, |ui| {
                            action = builder_panels::panels_list(ui, &status, selected);
                        });
                    ui.separator();
                    let set = status.set();
                    for line in set.map(|s| s.failing()).unwrap_or_default() {
                        ui.colored_label(crate::theme::TEXT_PRIMARY, line);
                    }
                    let sheets = set.map_or(&[][..], |s| &s.sheets[..]);
                    let not_flat: Vec<bool> = set
                        .map(|s| s.rows.iter().map(|r| !r.lies_flat).collect())
                        .unwrap_or_default();
                    let stale = matches!(status, builder_panels::PanelsStatus::Stale(_));
                    ui.allocate_ui(
                        egui::vec2(ui.available_width(), ui.available_height().max(320.0)),
                        |ui| {
                            clicked =
                                crate::sheet_view::show(ui, sheets, &not_flat, stale, &mut view);
                        },
                    );
                });
            });
        self.panels_view = view;
        if clicked.is_some() {
            self.panels_selected = clicked;
        }
        if params != self.panels_params {
            self.set_panels_params(params);
        }
        if export == Some(builder_panels::PanelsAction::Export) {
            self.open_dialog(DialogKind::Panels);
        }
        match action {
            Some(builder_panels::PanelsAction::Frame(i)) => {
                self.panels_selected = Some(i);
                self.panels_view.frame(i);
            }
            Some(builder_panels::PanelsAction::Export) | None => {}
        }
        self.show_panels = open;
    }

    /// Where the Builder panels drawing looks.
    pub fn panels_view(&self) -> &crate::sheet_view::SheetView {
        &self.panels_view
    }

    /// The panel picked in the Builder panels list or drawing.
    pub fn panels_selected(&self) -> Option<usize> {
        self.panels_selected
    }

    /// Open File > Builder panels (from the File menu or Plate development).
    pub fn open_builder_panels(&mut self) {
        if self.surface.is_some() {
            self.show_panels = true;
        }
    }

    /// Is the Builder panels window open?
    pub fn show_panels(&self) -> bool {
        self.show_panels
    }

    pub fn set_show_panels(&mut self, show: bool) {
        self.show_panels = show;
    }

    pub fn panels_params(&self) -> &crate::commands::PanelsParams {
        &self.panels_params
    }

    /// Change the panels options; when a measured one changed, the measurement is stale from
    /// now ([`crate::commands::PanelsParams::measured`]).
    pub fn set_panels_params(&mut self, params: crate::commands::PanelsParams) {
        if params.measured() != self.panels_params.measured() {
            self.panels_rev += 1;
        }
        self.panels_params = params;
    }

    /// Why the patterns cannot be exported now, if they cannot: the panels for the current
    /// options are not measured yet or could not be, or a panel does not lie flat.
    pub fn panels_export_block(&self) -> Option<String> {
        match self.panels_status() {
            builder_panels::PanelsStatus::Working | builder_panels::PanelsStatus::Stale(_) => {
                Some("Wait for the panels to be measured for these options".to_owned())
            }
            builder_panels::PanelsStatus::Failed(reason) => Some(reason.to_owned()),
            builder_panels::PanelsStatus::Ready(set) if set.sheets.is_empty() => {
                Some("No panel could be drawn".to_owned())
            }
            builder_panels::PanelsStatus::Ready(set) => {
                crate::commands::panels_refusal(set, &self.panels_params)
            }
        }
    }

    /// The patterns of the measured panels, checked for export to `path`.
    fn panels_for_export(
        &self,
        path: &std::path::Path,
    ) -> Result<
        (
            std::sync::Arc<seacraft_core::panels::PanelSet>,
            seacraft_core::templates::Format,
        ),
        String,
    > {
        let format = self.panels_params.format_for(path)?;
        if let Some(reason) = self.panels_export_block() {
            return Err(reason);
        }
        let set = self
            .panels_preview()
            .ok_or("Wait for the panels to be measured")?;
        Ok((set.set.clone(), format))
    }

    /// Write the measured panels' patterns to `path` on this thread, in the format its
    /// extension names (tests and scripts; the window writes on a worker).
    pub fn export_panels_to(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (set, format) = self.panels_for_export(path)?;
        let file = crate::commands::panels_file(&set, &self.panels_params, format)?;
        let written = seacraft_core::io::write_atomic(path, &file.bytes);
        self.log_export("Builder panels", path, written)
    }

    /// Start writing the measured panels' patterns to `path` on a worker.
    fn start_panels_export(&mut self, path: &std::path::Path) -> Result<(), String> {
        let (set, format) = match self.panels_for_export(path) {
            Ok(v) => v,
            Err(err) => {
                self.log(Level::Error, err.clone());
                return Err(err);
            }
        };
        let params = self.panels_params.clone();
        let target = path.to_path_buf();
        self.set_status(format!("writing panel patterns {}…", path.display()));
        let job = Job::spawn(self.revision, move || {
            let file = crate::commands::panels_file(&set, &params, format)?;
            seacraft_core::io::write_atomic(&target, &file.bytes)
                .map_err(|e| format!("cannot write {}: {e}", target.display()))?;
            Ok(file.pages.unwrap_or(0))
        });
        self.panels_export_job = Some((path.to_path_buf(), job));
        Ok(())
    }

    /// Collect a finished panel patterns export, if any.
    fn poll_panels_export(&mut self, ctx: &egui::Context) {
        let Some((path, job)) = &self.panels_export_job else {
            return;
        };
        let (level, msg) = match job.poll() {
            crate::jobs::Poll::Running => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            crate::jobs::Poll::Done(pages) => (
                Level::Info,
                match pages {
                    0 => format!("exported {} (Builder panels)", path.display()),
                    n => format!("exported {} (Builder panels, {n} pages)", path.display()),
                },
            ),
            crate::jobs::Poll::Failed(e) => (Level::Error, format!("panel patterns failed: {e}")),
        };
        self.log(level, msg);
        self.panels_export_job = None;
    }

    /// The panels measured for the current options and model; `None` while they are being
    /// measured or after a failure.
    pub fn panels_preview(&self) -> Option<&crate::commands::PanelsPreview> {
        self.panels_result
            .as_ref()
            .filter(|_| self.panels_track.is_current(self.panels_key()))
    }

    /// Why measuring the panels for the current options and model failed, if it did.
    pub fn panels_failure(&self) -> Option<&str> {
        self.panels_track.failure(self.panels_key())
    }

    /// Is the Builder templates window open?
    pub fn show_templates(&self) -> bool {
        self.show_templates
    }

    pub fn set_show_templates(&mut self, show: bool) {
        self.show_templates = show;
    }

    pub fn templates_params(&self) -> &crate::commands::TemplatesParams {
        &self.templates_params
    }

    /// Change the templates options; the preview is stale from now.
    pub fn set_templates_params(&mut self, params: crate::commands::TemplatesParams) {
        if params != self.templates_params {
            self.templates_params = params;
            self.templates_rev += 1;
        }
    }

    /// Pages of the current preview; `None` while it is being computed or after a failure.
    pub fn templates_pages(&self) -> Option<usize> {
        match self.templates_status(self.templates_key()) {
            panels::TemplatesStatus::Ready { pages, .. } => Some(pages),
            _ => None,
        }
    }

    /// Why the current preview failed, if it did.
    pub fn templates_failure(&self) -> Option<&str> {
        self.templates_track.failure(self.templates_key())
    }

    /// The preview's bytes when they are current and in `format`.
    fn templates_preview_in(&self, format: seacraft_core::templates::Format) -> Option<&[u8]> {
        let r = self.templates_result.as_ref()?;
        (self.templates_track.is_current(self.templates_key()) && r.format == format)
            .then_some(r.bytes.as_slice())
    }

    /// Write the templates to `path` on this thread, in the format its extension names: the
    /// previewed PDF when it is current and the same format, otherwise cut afresh. For the
    /// control channel and tests; the outcome is logged.
    pub fn export_templates_to(&mut self, path: &std::path::Path) -> Result<(), String> {
        let format = self.templates_params.format_for(path)?;
        let bytes = match self.templates_preview_in(format) {
            Some(bytes) => bytes.to_vec(),
            None => {
                let surface = self.surface.as_ref().ok_or("No model loaded.")?;
                crate::commands::templates_in(
                    surface,
                    self.level,
                    self.mode,
                    &self.templates_params,
                    format,
                )?
                .bytes
            }
        };
        let written = seacraft_core::io::write_atomic(path, bytes);
        self.log_export("Builder templates", path, written)
    }

    /// The Export dialog's OK: write the previewed PDF at once, or cut another format on a
    /// worker thread; the message log reports when it is written.
    fn start_templates_export(&mut self, path: &std::path::Path) -> Result<(), String> {
        let format = match self.templates_params.format_for(path) {
            Ok(format) => format,
            Err(err) => {
                self.log(Level::Error, err.clone());
                return Err(err);
            }
        };
        if let Some(bytes) = self.templates_preview_in(format) {
            let written = seacraft_core::io::write_atomic(path, bytes);
            return self.log_export("Builder templates", path, written);
        }
        let Some(surface) = self.surface.clone() else {
            return Err("No model loaded.".to_owned());
        };
        let (level, mode) = (self.level, self.mode);
        let params = self.templates_params.clone();
        let target = path.to_path_buf();
        self.set_status(format!("writing templates {}…", path.display()));
        let job = Job::spawn(self.revision, move || {
            let r = crate::commands::templates_in(&surface, level, mode, &params, format)?;
            seacraft_core::io::write_atomic(&target, r.bytes)
                .map_err(|e| format!("cannot write {}: {e}", target.display()))
        });
        self.templates_export_job = Some((path.to_path_buf(), job));
        Ok(())
    }

    /// Collect a finished templates export, if any.
    fn poll_templates_export(&mut self, ctx: &egui::Context) {
        let Some((path, job)) = &self.templates_export_job else {
            return;
        };
        let (level, msg) = match job.poll() {
            crate::jobs::Poll::Running => {
                ctx.request_repaint_after(std::time::Duration::from_millis(200));
                return;
            }
            crate::jobs::Poll::Done(()) => (
                Level::Info,
                format!("exported {} (Builder templates)", path.display()),
            ),
            crate::jobs::Poll::Failed(e) => (Level::Error, format!("templates failed: {e}")),
        };
        self.log(level, msg);
        self.templates_export_job = None;
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
        let pre_drag = ui.ctx().dragged_id();
        let pre_focus = ui
            .ctx()
            .text_edit_focused()
            .then(|| ui.ctx().memory(|m| m.focused()))
            .flatten();
        let before = (
            self.project.clone(),
            (self.hydro.kg, self.hydro.kg_estimate),
        );
        egui::Window::new("Report options")
            .open(&mut open)
            .collapsible(false)
            .resizable(true)
            .default_width(420.0)
            .show(ui.ctx(), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(560.0)
                    .show(ui, |ui| {
                        let _ = panels::report_options(
                            ui,
                            &mut self.project.report,
                            &conditions,
                            &cases,
                        );
                    });
            });
        self.record_project_edit(ui.ctx(), before, pre_drag, pre_focus);
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

    /// Widen the Analysis dock, at first to about 55 % of the window (plots, tables); either
    /// width can be dragged and is kept.
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
                let clipped_mesh = if tank.clip_to_hull {
                    self.clipped_volume_mesh(
                        &tank.name,
                        Vec3::new(tank.x_min, tank.y_min, tank.z_min),
                        Vec3::new(tank.x_max, tank.y_max, tank.z_max),
                    )
                } else {
                    None
                };
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
                        clip_to_hull: tank.clip_to_hull,
                        clipped_mesh,
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
            let clipped_mesh = if comp.clip_to_hull {
                self.clipped_volume_mesh(
                    &comp.name,
                    Vec3::new(comp.x_min, comp.y_min, comp.z_min),
                    Vec3::new(comp.x_max, comp.y_max, comp.z_max),
                )
            } else {
                None
            };
            out.push((
                VolumeRef::Compartment(c_idx),
                render::VolumeOverlay {
                    name: comp.name.clone(),
                    min: Vec3::new(comp.x_min, comp.y_min, comp.z_min),
                    max: Vec3::new(comp.x_max, comp.y_max, comp.z_max),
                    kind: render::VolumeKind::Compartment { flooded },
                    selected,
                    clip_to_hull: comp.clip_to_hull,
                    clipped_mesh,
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
            self.set_status("No model loaded.");
            return;
        };
        if self.project.loading_conditions.is_empty() {
            self.set_status("No loading condition in project.");
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
            self.log(Level::Error, msg.clone());
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
        self.install_loading(res);
        // Supersedes any job: the result shown is this one.
        self.loading_track.stamp(self.loading_key());
        Ok(self.loading_result.as_ref().unwrap())
    }

    /// Show a finished loading computation. One that found no equilibrium
    /// is logged once, here, as a warning: its figures describe the hull
    /// at the input draft/trim, not a floating condition.
    fn install_loading(&mut self, res: LoadingResult) {
        if !res.equilibrium_found {
            let why = if res.warnings.is_empty() {
                String::new()
            } else {
                format!(" ({})", res.warnings.join("; "))
            };
            self.log(
                Level::Warning,
                format!(
                    "Loading condition \"{}\": {}{why}",
                    res.name,
                    seacraft_core::loading::NO_EQUILIBRIUM_NOTE
                ),
            );
        }
        self.loading_result = Some(res);
    }

    pub fn damage_result(&self) -> Option<&DamageResult> {
        self.damage_result.as_ref()
    }

    pub fn start_damage_compute(&mut self) {
        let Some(surface) = self.surface.clone() else {
            self.set_status("No model loaded.");
            return;
        };
        if self.project.damage_cases.is_empty() {
            self.set_status("No damage case in project.");
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

    /// The seakeeping KG: the manual value, else the default; `None` while
    /// no KG is known anywhere.
    pub fn effective_performance_kg(&self) -> Option<f64> {
        self.perf_state
            .kg
            .or_else(|| self.effective_performance_kg_default())
    }

    /// The Performance KG without a manual value: the current loading
    /// condition's fluid KG, else the hydro dock's (which may be unset).
    fn effective_performance_kg_default(&self) -> Option<f64> {
        match &self.loading_result {
            Some(r) if self.loading_current() => Some(r.kg_fluid),
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

    /// The seakeeping inputs; an error while no KG is known (the pitch
    /// restoring moment needs one).
    pub fn seakeeping_spec(&self) -> Result<SeakeepingSpec, String> {
        let draft = self.hydro.options.draft;
        let kg = self
            .effective_performance_kg()
            .ok_or_else(|| format!("seakeeping: {KG_UNSET_HINT}"))?;
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
        Ok(SeakeepingSpec {
            draft,
            speed_kn: self.perf_state.sk_speed_kn,
            kg,
            kyy,
            points: Some(self.perf_state.points),
            ..Default::default()
        })
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
        let spec = self.seakeeping_spec()?;
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
        let spec = match self.seakeeping_spec() {
            Ok(spec) => spec,
            Err(e) => {
                // A missing input (KG unset), not a failure: the open
                // Performance dock retries every frame, so a logged line
                // would flood the message log (as for the GZ curves).
                self.set_status(e);
                return;
            }
        };
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

    /// Draw the viewports' hull on the GPU, if the device can make the resources. Called by the
    /// binary unless `--no-gpu-hull`, so headless apps stay on the painted path. On an error the app
    /// stays on the painted path: it never fails to start on a machine that ran it before.
    pub fn enable_gpu_hull(&mut self, state: &eframe::egui_wgpu::RenderState) {
        match gpu_hull::create_resources(state) {
            Ok(resources) => {
                state.renderer.write().callback_resources.insert(resources);
                self.gpu_hull.max_texture = state.device.limits().max_texture_dimension_2d;
                self.gpu_hull.routing = true;
            }
            Err(message) => self.log(
                Level::Warning,
                format!("The viewport keeps the slower drawing: the GPU hull failed ({message})."),
            ),
        }
    }

    /// Route the hull through the GPU, with or without resources to draw it (test hook).
    pub fn set_gpu_hull_routing(&mut self, on: bool) {
        self.gpu_hull.routing = on;
    }

    /// Whether the viewports route their hull through the GPU.
    pub fn gpu_hull_routing(&self) -> bool {
        self.gpu_hull.routing
    }

    /// The waterline mesh view `index` keeps, tessellated for its camera and cell (test hook).
    pub fn waterline_mesh(&self, index: usize) -> Option<&Arc<egui::Mesh>> {
        self.waterline_meshes[index].as_ref().map(|(_, mesh)| mesh)
    }

    /// The geometry the viewports' callbacks share (test hook).
    pub fn gpu_hull_geometry(&self) -> Option<&Arc<HullGeometry>> {
        self.gpu_hull.geometry.as_ref()
    }

    /// The GPU could not draw the geometry with this key. That mesh, with or without its edges, and
    /// any mesh during a drag, take the old path; a new mesh outside a drag tries the GPU again. Each
    /// message is logged once a session, so a GPU that keeps failing does not flood the log.
    pub fn report_gpu_hull_failure(&mut self, key: u64, message: String) {
        let held = self
            .gpu_hull
            .geometry
            .as_ref()
            .is_some_and(|g| g.key == key);
        self.gpu_hull.failed_mesh = Some(if held { self.gpu_hull.mesh_key } else { key });
        if self.gpu_hull.logged.insert(message.clone()) {
            self.log(
                Level::Warning,
                format!("The viewport falls back to the slower drawing for this mesh: {message}"),
            );
        }
    }

    /// Whether routing is on and the display has a hull for it to draw: an analysis mode needs the
    /// rasterizer's per-pixel shading.
    fn gpu_hull_applies(&self) -> bool {
        let display = &self.display;
        self.gpu_hull.routing
            && display.analysis.is_shaded()
            && (display.shaded || display.wireframe)
    }

    /// Build the geometry the views share if the mesh drawn this frame has changed.
    fn refresh_gpu_hull_geometry(&mut self) {
        if !self.gpu_hull_applies() {
            return;
        }
        let (Some(surface), Some(cache)) = (&self.surface, &self.cache) else {
            return;
        };
        let wireframe = self.display.wireframe;
        let key = render::geometry_key(cache.stamp(), surface, wireframe);
        self.gpu_hull.mesh_key = render::geometry_key(cache.stamp(), surface, false);
        // A new mesh outside a drag tries the GPU again, so a failure does not outlive its mesh.
        if self.drag.is_none()
            && self
                .gpu_hull
                .failed_mesh
                .is_some_and(|m| m != self.gpu_hull.mesh_key)
        {
            self.gpu_hull.failed_mesh = None;
        }
        if self.gpu_hull.geometry.as_ref().is_none_or(|g| g.key != key) {
            let geometry = build_geometry(&cache.mesh, surface, key, wireframe);
            self.gpu_hull.geometry = Some(Arc::new(geometry));
        }
    }

    /// How view `index` draws its hull in `cell` this frame.
    fn hull_route(
        &self,
        ctx: &egui::Context,
        index: usize,
        cell: egui::Rect,
        scene: &Scene,
    ) -> HullRoute {
        let geometry = match &self.gpu_hull.geometry {
            Some(geometry) if self.gpu_hull_applies() => geometry,
            _ => return HullRoute::Painted,
        };
        if geometry.vertices.is_empty() {
            return HullRoute::Nothing;
        }
        let failed = self.gpu_hull.failed_mesh;
        if failed.is_some_and(|mesh| self.drag.is_some() || mesh == self.gpu_hull.mesh_key) {
            return HullRoute::Painted;
        }
        let ppp = ctx.pixels_per_point();
        let Some(size) = gpu_hull::view::target_size(cell, ppp) else {
            return HullRoute::Nothing;
        };
        if size.iter().any(|&side| side > self.gpu_hull.max_texture) {
            return HullRoute::Painted;
        }
        let display = &self.display;
        let camera = &self.cameras[index];
        let stamp = self.cache.as_ref().map_or(0, RenderCache::stamp);
        let hull_key = render::hull_key(stamp, camera, cell, scene, display);
        let callback = HullCallback {
            index,
            uniform: gpu_hull::view::view_uniform(
                &camera.transform(),
                cell,
                ppp,
                geometry.origin,
                geometry.min,
                geometry.max,
                crate::theme::WIREFRAME,
            ),
            size,
            view_key: render::gpu_view_key(hull_key, ppp, display),
            geometry: geometry.clone(),
            shaded: display.shaded,
            wire: display.wireframe,
            failure: self.gpu_hull.failure.clone(),
            context: ctx.clone(),
        };
        HullRoute::Gpu(eframe::egui_wgpu::Callback::new_paint_callback(
            cell, callback,
        ))
    }

    pub fn render_cache(&self) -> Option<&RenderMesh> {
        self.cache.as_ref().map(|c| &c.mesh)
    }

    /// Cached net lines for the current surface, once [`Self::refresh_cache`] has run.
    pub fn net_lines(&self) -> Option<&seacraft_core::netlines::NetLines> {
        self.cache.as_ref().map(|c| &c.net_lines)
    }

    /// The waterline cached for the current mesh, while it was cut with the
    /// waterplane of the current floating condition: a cut from any other
    /// plane is never shown.
    pub fn waterline(&self) -> Option<&[[Vec3; 2]]> {
        let options = &self.hydro.options;
        let plane = seacraft_core::hydro::waterplane(
            self.surface.as_ref()?,
            options.draft,
            options.trim,
            options.heel_deg,
        );
        let (cut, segments) = self.cache.as_ref()?.waterline.as_ref()?;
        same_plane(cut, &plane).then_some(segments.as_slice())
    }

    /// How many times the waterline has been cut from the mesh (test hook).
    pub fn waterline_cuts(&self) -> u64 {
        self.waterline_cuts
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

    fn net_visibility_key(&self, view: usize, cell: egui::Rect) -> Option<u64> {
        let cache = self.cache.as_ref()?;
        if cache.revision != self.revision || !cell.is_positive() {
            return None;
        }
        let transform = self.cameras.get(view)?.transform();
        Some(revision_key(&[
            cache.stamp(),
            view_key(&transform),
            u64::from(cell.width().to_bits()),
            u64::from(cell.height().to_bits()),
            self.display.shaded as u64,
        ]))
    }

    /// Whether any view classifies the control net against the hull: only a
    /// solid hull (shaded, or an analysis mode) hides it, "same" draws it as is, and with the net and its
    /// points both off there is nothing to classify. Otherwise neither the ray
    /// index nor the per-view visibility is built.
    fn net_visibility_wanted(&self) -> bool {
        self.display.hull_is_solid()
            && self.display.hidden_net != crate::render::HiddenNet::Same
            && (self.display.control_net || self.display.points)
    }

    fn sync_net_visibility(&mut self, index: usize, cell: egui::Rect) {
        let key = self
            .net_visibility_wanted()
            .then(|| self.net_visibility_key(index, cell))
            .flatten();
        let Some(key) = key else {
            if let Some(slot) = self.net_visibilities.get_mut(index) {
                *slot = None;
            }
            return;
        };
        if self.net_visibilities[index]
            .as_ref()
            .is_some_and(|(k, _)| *k == key)
        {
            return;
        }
        let (Some(surface), Some(cache)) = (self.surface.as_ref(), self.cache.as_mut()) else {
            return;
        };
        let view = self.cameras[index].transform();
        let bvh = cache
            .bvh
            .get_or_insert_with(|| seacraft_core::raycast::Bvh::of_render_mesh(&cache.mesh));
        let vis = crate::visibility::net_visibility(surface, bvh, &view, cell, 0.0);
        self.net_visibilities[index] = Some((key, vis));
    }

    /// Cached control-net visibility for viewport `view` (`0..4`), when current.
    pub fn net_visibility(&self, view: usize) -> Option<&crate::visibility::NetVisibility> {
        if !self.net_visibility_wanted() {
            return None;
        }
        let cell = *self.viewport_rects.get(view)?;
        let key = self.net_visibility_key(view, cell)?;
        self.net_visibilities
            .get(view)?
            .as_ref()
            .and_then(|(k, vis)| (*k == key).then_some(vis))
    }

    pub fn selected(&self) -> Option<usize> {
        self.selection.active()
    }

    pub fn selection(&self) -> &crate::selection::Selection {
        &self.selection
    }

    /// The control point under the pointer this frame, if any (test hook).
    pub fn hovered_point(&self) -> Option<usize> {
        self.hovered_point
    }

    /// Whether the hull's ray index is built for the current mesh (test hook):
    /// it is built only when a view classifies the net behind the hull.
    pub fn ray_index_built(&self) -> bool {
        self.cache.as_ref().is_some_and(|c| c.bvh.is_some())
    }

    pub fn selection_mut(&mut self) -> &mut crate::selection::Selection {
        &mut self.selection
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
        self.autosave.note_dirty();
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
        if self.dirty {
            self.autosave.note_dirty();
        } else {
            self.autosave.note_clean();
        }
    }

    /// The model revision: bumped by every edit.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// The model revision as a worker thread sees it: always equal to [`App::revision`].
    pub fn revision_watch(&self) -> std::sync::Arc<std::sync::atomic::AtomicU64> {
        self.revision_watch.clone()
    }

    /// Bumps the model revision; the only place it changes.
    fn bump_revision(&mut self) {
        self.revision += 1;
        self.revision_watch
            .store(self.revision, std::sync::atomic::Ordering::SeqCst);
    }

    pub fn mark_geometry_changed(&mut self) {
        let n = self.surface.as_ref().map_or(0, |s| s.points.len());
        self.selection.retain_valid(n);
        self.bump_revision();
        self.dirty = true;
        self.autosave.note_dirty();
        if let Ok(mut guard) = self.clipped_volumes_cache.lock() {
            guard.clear();
        }
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

    /// Set the dock KG as the user does: no longer an estimate, and saved
    /// in the project sidecar.
    pub fn set_kg(&mut self, kg: f64) {
        self.hydro.kg = Some(kg);
        self.hydro.kg_estimate = false;
        if self.project.kg != Some(kg) {
            self.project.kg = Some(kg);
            self.dirty = true;
            self.autosave.note_dirty();
        }
        self.hydro_inputs_changed();
    }

    /// The dock KG above the keel; `None` until set (see [`DEMO_KG`]).
    pub fn kg(&self) -> Option<f64> {
        self.hydro.kg
    }

    /// The dock KG is the design requirements' estimate.
    pub fn kg_is_estimate(&self) -> bool {
        self.hydro.kg.is_some() && self.hydro.kg_estimate
    }

    /// The largest KG and trim the dock accepts for the current hull: KG up
    /// to twice its depth, trim to a fifth of its length, never below the
    /// old fixed limits (10 m, 5 m).
    pub fn hydro_limits(&self) -> panels::HydroLimits {
        panels::HydroLimits::for_extents(self.surface.as_ref().map(|s| s.extents()))
    }

    /// Refinement level used for display and measurement. Tests drop it to keep
    /// repeated solves cheap; the UI drives the same field from the menu.
    pub fn set_subdivision_level(&mut self, level: u8) {
        self.level = level.clamp(0, 5);
        self.bump_revision();
        self.touch_geometry();
    }

    pub fn set_subdivision_mode(&mut self, mode: SubdivisionMode) {
        self.mode = mode;
        self.bump_revision();
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
        if self.refuse_curves_without_kg() {
            return;
        }
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

    /// How many station, buttock and waterline curves the viewports have
    /// cut, and how many comb spikes they carry in all (`None` before the
    /// sections are first needed).
    pub fn section_curve_counts(&self) -> Option<(usize, usize, usize, usize)> {
        let curves = self.cache.as_ref()?.curves.as_ref()?;
        let spikes = curves
            .stations
            .iter()
            .chain(&curves.buttocks)
            .chain(&curves.waterlines)
            .map(|c| c.spikes.len())
            .sum();
        Some((
            curves.stations.len(),
            curves.buttocks.len(),
            curves.waterlines.len(),
            spikes,
        ))
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

    /// Bounding extents for camera framing. When windage display is on,
    /// includes visible windage profile outlines for views that show them (Profile).
    pub fn framing_extents(&self, kind: ViewKind) -> (Vec3, Vec3) {
        let (mut min, mut max) = self
            .surface
            .as_ref()
            .map(crate::render::display_extents)
            .unwrap_or((Vec3::ZERO, Vec3::ZERO));

        if self.display.windage && kind == ViewKind::Profile {
            for item in &self.project.windage {
                if let Some(r) = &item.profile {
                    min.x = min.x.min(r.x_min);
                    max.x = max.x.max(r.x_max);
                    min.z = min.z.min(r.z_min);
                    max.z = max.z.max(r.z_max);
                }
            }
        }
        (min, max)
    }

    /// Refit camera `index` using framing extents and viewport aspect ratio.
    pub fn refit_camera(&mut self, index: usize) {
        if self.surface.is_some() {
            let (min, max) = self.framing_extents(self.cameras[index].kind);
            let cell = self.viewport_rect(index);
            let aspect = if cell.width() > 10.0 && cell.height() > 10.0 {
                Some(cell.width() as f64 / cell.height() as f64)
            } else {
                Some(1.25)
            };
            self.cameras[index].frame_aspect(min, max, aspect);
        }
    }

    /// Refit all 4 viewport cameras.
    pub fn refit_all_cameras(&mut self) {
        for i in 0..4 {
            self.refit_camera(i);
        }
    }

    /// Build or fetch cached triangulated mesh for a clip_to_hull volume.
    fn clipped_volume_mesh(
        &self,
        name: &str,
        min: Vec3,
        max: Vec3,
    ) -> Option<(Vec<Vec3>, Vec<[usize; 3]>)> {
        let bounds = [
            self.revision,
            min.x.to_bits(),
            min.y.to_bits(),
            min.z.to_bits(),
            max.x.to_bits(),
            max.y.to_bits(),
            max.z.to_bits(),
        ];
        if let Ok(guard) = self.clipped_volumes_cache.lock() {
            if let Some((_, _, mesh)) = guard.iter().find(|(n, b, _)| n == name && b == &bounds) {
                return Some(mesh.clone());
            }
        }
        let surface = self.surface.as_ref()?;
        let hull = seacraft_core::hydro::Hull::new(
            surface,
            seacraft_core::SubdivisionOptions {
                level: 2,
                mode: self.mode,
            },
        );
        let comp = seacraft_core::damage::Compartment {
            name: name.to_string(),
            x_min: min.x,
            x_max: max.x,
            y_min: min.y,
            y_max: max.y,
            z_min: min.z,
            z_max: max.z,
            permeability: 1.0,
            clip_to_hull: true,
        };
        let solid = seacraft_core::damage::CompartmentSolid::build(&comp, &hull);
        let mesh = (solid.verts, solid.tris);
        // Names need not be unique, so two volumes may share one: only the
        // revision retires an entry, and the cap bounds a drag of the bounds.
        if let Ok(mut guard) = self.clipped_volumes_cache.lock() {
            guard.retain(|(_, b, _)| b[0] == self.revision);
            if guard.len() >= MAX_CLIPPED_VOLUMES {
                guard.remove(0);
            }
            guard.push((name.to_string(), bounds, mesh.clone()));
        }
        Some(mesh)
    }

    /// Clipped volume meshes held for the viewports.
    pub fn clipped_volume_cache_len(&self) -> usize {
        self.clipped_volumes_cache.lock().map_or(0, |g| g.len())
    }

    /// Frame the model in view `index` alone.
    fn fit_view(&mut self, index: usize) {
        if self.surface.is_some() {
            self.refit_camera(index);
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

    /// View `i`'s fit and maximise buttons (last frame); `Rect::NOTHING`
    /// when hidden.
    pub fn view_button_rects(&self, i: usize) -> [egui::Rect; 2] {
        self.view_button_rects[i]
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

    /// The active layer's index, clamped to the model's layers (an undo may
    /// have taken the layer it pointed at away).
    pub fn active_layer(&self) -> usize {
        self.active_layer.min(self.layer_count().saturating_sub(1))
    }

    /// Set the active layer (the commands check the index).
    pub fn set_active_layer_index(&mut self, layer: usize) {
        self.active_layer = layer;
    }

    /// The control face under the last click in a viewport.
    pub fn picked_face(&self) -> Option<usize> {
        self.picked_face
            .filter(|&f| f < self.surface.as_ref().map_or(0, |s| s.faces.len()))
    }

    /// Set the picked face as a click would (tests, the control channel).
    pub fn set_picked_face(&mut self, face: Option<usize>) {
        self.picked_face = face;
    }

    pub fn cameras_mut(&mut self) -> &mut [Camera; 4] {
        &mut self.cameras
    }

    /// Select a control point (`None` clears the selection).
    pub fn select_point(&mut self, index: Option<usize>) {
        match index {
            Some(i) => self.selection.set_only(i),
            None => self.selection.clear(),
        }
    }

    /// Record the current model on the undo stack, as a drag does on grab.
    pub fn snapshot_undo(&mut self) {
        self.project_edit_source = None;
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
        let (to, clamped, _) = self.move_point_with(index, target, None)?;
        Ok((to, clamped))
    }

    /// Move control point `index` to `target` as one undoable edit, optionally
    /// drawing neighbours along as `connected` says. Returns the position the
    /// point took, whether it sits on the centreline of a symmetric layer, and
    /// every point that moved.
    pub fn move_point_with(
        &mut self,
        index: usize,
        target: Vec3,
        connected: Option<(&seacraft_core::connected::ConnectedPreset, Option<f64>)>,
    ) -> Result<(Vec3, bool, Vec<seacraft_core::netmove::Moved>), String> {
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        self.refuse_while_dragging()?;
        if index >= surface.points.len() {
            return Err(format!(
                "`index` must be from 0 to {}",
                surface.points.len().saturating_sub(1)
            ));
        }
        if !target.is_finite() {
            return Err("the new position must be finite".into());
        }
        let symmetric = seacraft_core::netmove::point_on_symmetric_layer(surface, index);
        let plan = build_move_plan(surface, &[index], connected);
        let moved = plan.positions_to(index, target);
        let to = moved
            .iter()
            .find(|m| m.point == index)
            .map_or(target, |m| m.to);
        self.snapshot_undo();
        if let Some(surface) = self.surface.as_mut() {
            plan.apply_to(surface, index, target);
        }
        self.select_point(Some(index));
        self.mark_geometry_changed();
        if moved.len() > 1 {
            self.status_note(format!("{} control points moved", moved.len()));
        } else {
            self.status_note(format!("control point {index} moved"));
        }
        Ok((to, symmetric && to.y == 0.0, moved))
    }

    /// A move asked for while a point is being dragged in the window would be
    /// overwritten by the drag, which recomputes its points from where they
    /// started; the hand on the mouse wins and the caller tries again.
    fn refuse_while_dragging(&self) -> Result<(), String> {
        if self.drag.is_some() {
            return Err(
                "a control point is being dragged in the window; try again once it is released"
                    .into(),
            );
        }
        Ok(())
    }

    /// Move every control point in `seeds` by `delta` as one undoable edit.
    pub fn move_points(
        &mut self,
        seeds: &[usize],
        delta: Vec3,
    ) -> Result<Vec<seacraft_core::netmove::Moved>, String> {
        self.move_points_with(seeds, delta, None)
    }

    /// Move every control point in `seeds` by `delta` as one undoable edit,
    /// optionally drawing neighbours along as `connected` says.
    pub fn move_points_with(
        &mut self,
        seeds: &[usize],
        delta: Vec3,
        connected: Option<(&seacraft_core::connected::ConnectedPreset, Option<f64>)>,
    ) -> Result<Vec<seacraft_core::netmove::Moved>, String> {
        let surface = self.surface.as_ref().ok_or("no model loaded")?;
        self.refuse_while_dragging()?;
        if seeds.is_empty() {
            return Err("`points` must name at least one control point".into());
        }
        let count = surface.points.len();
        for &index in seeds {
            if index >= count {
                return Err(format!(
                    "point index {index} is out of range (0..{})",
                    count.saturating_sub(1)
                ));
            }
        }
        if !delta.is_finite() {
            return Err("`delta` must be finite".into());
        }
        let plan = build_move_plan(surface, seeds, connected);
        let moved = plan.positions(delta);
        self.snapshot_undo();
        if let Some(surface) = self.surface.as_mut() {
            plan.apply(surface, delta);
        }
        self.selection.clear();
        self.selection.add_all(seeds.iter().copied());
        self.mark_geometry_changed();
        self.status_note(format!(
            "{} control point{} moved",
            moved.len(),
            if moved.len() == 1 { "" } else { "s" }
        ));
        Ok(moved)
    }

    /// Apply what the Inspector asked for. A value equal to the point's
    /// position changes nothing and records no undo step.
    pub fn apply_inspector_action(&mut self, action: panels::InspectorAction) {
        match action {
            panels::InspectorAction::None => {}
            panels::InspectorAction::Move { index, to } => {
                let unchanged = self
                    .surface
                    .as_ref()
                    .and_then(|s| s.points.get(index))
                    .is_some_and(|p| p.p == to);
                if unchanged {
                    return;
                }
                if let Err(e) = self.move_point(index, to) {
                    self.log(Level::Error, e);
                }
            }
            panels::InspectorAction::MoveSelection { delta } => {
                if delta == Vec3::ZERO {
                    return;
                }
                let seeds = self.selection.points().to_vec();
                if let Err(e) = self.move_points(&seeds, delta) {
                    self.log(Level::Error, e);
                }
            }
        }
    }

    /// Where the Inspector's x (0), y (1) or z (2) field was drawn last
    /// frame; `Rect::NOTHING` with no point selected (test hook).
    pub fn inspector_field_rect(&self, axis: usize) -> egui::Rect {
        self.inspector_rects[axis]
    }

    /// Where the Inspector's "Move" button was drawn last frame;
    /// `Rect::NOTHING` when fewer than two points are selected (test hook).
    pub fn inspector_move_button_rect(&self) -> egui::Rect {
        self.inspector_move_button_rect
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
        self.set_status(text);
    }

    /// Replace the status line with an informational message (not logged).
    fn set_status(&mut self, text: impl Into<String>) {
        self.status = text.into();
        self.status_level = Level::Info;
    }

    /// Log a message and show it in the status bar, coloured by its level.
    pub fn log(&mut self, level: Level, text: impl Into<String>) {
        let text = text.into();
        self.messages.push(level, text.clone());
        self.status = text;
        self.status_level = level;
    }

    /// The level of the status line: a logged warning or error, or info.
    pub fn status_level(&self) -> Level {
        self.status_level
    }

    /// The message log, oldest first.
    pub fn messages(&self) -> &MessageLog {
        &self.messages
    }

    pub fn clear_messages(&mut self) {
        self.messages.clear();
    }

    /// The Messages window is open.
    pub fn show_messages(&self) -> bool {
        self.show_messages
    }

    pub fn set_show_messages(&mut self, show: bool) {
        self.show_messages = show;
    }

    /// Where the status text was drawn last frame (a click opens Messages).
    pub fn status_rect(&self) -> egui::Rect {
        self.status_rect
    }

    // ---- recent files --------------------------------------------------

    /// Keep the recent list in `storage` from now on, starting from the list
    /// already stored there. Headless apps have none until a test gives one.
    pub fn set_storage(&mut self, storage: Box<dyn eframe::Storage>) {
        self.recent = RecentFiles::load(storage.as_ref());
        let choice = crate::recent::ConnectedChoice::load(storage.as_ref());
        self.connected = ConnectedState {
            on: choice.on,
            preset: choice.preset,
            reach_m: choice.reach_m,
        };
        self.storage = Some(storage);
        self.persisted_choice = Some(self.connected_choice());
        self.reconcile_connected_preset();
    }

    /// Take the storage back (tests: to read what was written).
    pub fn take_storage(&mut self) -> Option<Box<dyn eframe::Storage>> {
        self.storage.take()
    }

    /// The connected-editing switch, chosen preset name and reach override.
    pub fn connected(&self) -> &ConnectedState {
        &self.connected
    }

    /// Turn connected editing on or off and remember the choice.
    pub fn set_connected_on(&mut self, on: bool) {
        self.connected.on = on;
        self.hover_followers = None;
        self.persist_recent();
    }

    /// Choose a connected-editing preset by name; switching presets clears
    /// any reach override.
    pub fn set_connected_preset(&mut self, name: String) {
        if self.connected.preset != name {
            self.connected.preset = name;
            self.connected.reach_m = None;
        }
        self.hover_followers = None;
        self.persist_recent();
    }

    /// Set or clear the reach override in metres.
    pub fn set_connected_reach(&mut self, reach_m: Option<f64>) {
        self.connected.reach_m = reach_m
            .filter(|r| r.is_finite() && *r > 0.0)
            .map(|r| r.clamp(1e-3, 1e4));
        self.hover_followers = None;
        self.persist_recent();
    }

    /// Built-in and user connected-editing presets, plus any file error.
    pub fn preset_store(&self) -> &crate::connected_presets::PresetStore {
        &self.preset_store
    }

    /// Load presets from `dir/connected-edit.json` (or built-ins only when `None`).
    pub fn set_preset_dir(&mut self, dir: Option<PathBuf>) {
        self.preset_store = crate::connected_presets::PresetStore::load(dir.as_deref());
        self.hover_followers = None;
        self.reconcile_connected_preset();
        if let Some(err) = self.preset_store.error.clone() {
            self.log(Level::Warning, err);
        }
    }

    /// Re-read `connected-edit.json` and report the count or any error.
    pub fn reload_presets(&mut self) {
        self.preset_store.reload();
        self.hover_followers = None;
        let fell_back = self.reconcile_connected_preset();
        if let Some(err) = self.preset_store.error.clone() {
            self.log(Level::Warning, err);
        } else if !fell_back {
            let n = self.preset_store.presets.len();
            let msg = match &self.preset_store.path {
                Some(p) => format!("loaded {n} connected presets from {}", p.display()),
                None => format!("loaded {n} built-in connected presets"),
            };
            self.set_status(msg);
        }
        if fell_back {
            self.persist_recent();
        }
    }

    /// Write an example `connected-edit.json` without overwriting, and report
    /// the path written or why it could not be written.
    pub fn create_example_presets(&mut self) {
        match self.preset_store.write_example() {
            Ok(path) => {
                self.preset_store.reload();
                self.hover_followers = None;
                self.reconcile_connected_preset();
                self.set_status(format!("wrote example presets to {}", path.display()));
            }
            Err(err) => {
                self.log(Level::Warning, err);
            }
        }
    }

    /// The preset currently selected (falling back to the default built-in if missing).
    pub fn active_connected_preset(&self) -> seacraft_core::connected::ConnectedPreset {
        self.preset_store
            .presets
            .iter()
            .find(|p| p.name == self.connected.preset)
            .cloned()
            .unwrap_or_else(|| {
                seacraft_core::connected::builtin_presets()
                    .into_iter()
                    .next()
                    .expect("built-in presets are non-empty")
            })
    }

    /// The reach in metres used for connected editing on the current model.
    pub fn effective_connected_reach_m(&self) -> f64 {
        let preset = self.active_connected_preset();
        self.connected.reach_m.unwrap_or_else(|| {
            self.surface
                .as_ref()
                .map_or(0.0, |s| preset.reach.metres(s))
        })
    }

    fn connected_choice(&self) -> crate::recent::ConnectedChoice {
        crate::recent::ConnectedChoice {
            on: self.connected.on,
            preset: self.connected.preset.clone(),
            reach_m: self.connected.reach_m,
        }
    }

    fn reconcile_connected_preset(&mut self) -> bool {
        if self
            .preset_store
            .presets
            .iter()
            .any(|p| p.name == self.connected.preset)
        {
            return false;
        }
        let missing = std::mem::replace(
            &mut self.connected.preset,
            seacraft_core::connected::DEFAULT_PRESET.to_owned(),
        );
        self.connected.reach_m = None;
        self.hover_followers = None;
        self.log(
            Level::Warning,
            format!(
                "connected preset \"{missing}\" is no longer available; using \"{}\"",
                self.connected.preset
            ),
        );
        true
    }

    /// File > Open recent, most recent first.
    pub fn recent_files(&self) -> &RecentFiles {
        &self.recent
    }

    /// Put `path` on top of the recent list and keep the list.
    fn remember_recent(&mut self, path: &std::path::Path) {
        self.recent.push(path);
        self.persist_recent();
    }

    /// Empty the recent list (File > Open recent > Clear list).
    pub fn clear_recent(&mut self) {
        self.recent.clear();
        self.persist_recent();
    }

    /// Write the recent list to the storage now, so a crash does not lose it.
    fn persist_recent(&mut self) {
        let choice = self.connected_choice();
        let Some(storage) = self.storage.as_mut() else {
            return;
        };
        self.recent.save(storage.as_mut());
        choice.save(storage.as_mut());
        storage.flush();
        self.persisted_choice = Some(choice);
        let failure = storage.get_string(crate::recent::FLUSH_ERROR_KEY);
        // Warn once per distinct failure, not on every open.
        if failure.is_some() && failure != self.recent_failure {
            let e = failure.clone().unwrap_or_default();
            self.log(
                Level::Warning,
                format!("the recent files list is kept for this session only: {e}"),
            );
        }
        self.recent_failure = failure;
    }

    /// Open a recent file, through the unsaved-changes prompt.
    pub fn request_open_recent(&mut self, path: PathBuf) {
        self.pending_recent = Some(path);
        self.request(PendingAction::OpenRecent);
    }

    fn apply_recent_action(&mut self, action: RecentAction) {
        match action {
            RecentAction::None => {}
            RecentAction::Open(path) => self.request_open_recent(path),
            RecentAction::Clear => self.clear_recent(),
        }
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
            // `None`: a worker answers on `request.reply` when it is done.
            if let Some(reply) = crate::control::handle(self, ctx, &request) {
                let _ = request.reply.send(reply);
            }
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
            let warnings = seacraft_core::michell::michell_warnings(&grid, &core_rows);
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

/// Whether Shift is held for this frame's pointer press or release (read
/// from the button event too: a synthesised frame need not send
/// `ModifiersChanged`).
fn pointer_shift(ui: &egui::Ui) -> bool {
    ui.input(|i| {
        i.modifiers.shift
            || i.events.iter().any(
                |e| matches!(e, egui::Event::PointerButton { modifiers, .. } if modifiers.shift),
            )
    })
}

// ---- Model menu: transform, optimise, topology tools (R5-7) -------------

impl App {
    /// The Model menu, between View and Help; its action is applied after
    /// the menu has drawn.
    fn model_menu(&mut self, ui: &mut egui::Ui) {
        let mut action = crate::model_menu::ModelAction::None;
        ui.menu_button("Model", |ui| {
            let targets = self.model_targets();
            action = crate::model_menu::menu_body(ui, &targets);
        });
        self.apply_model_action(action);
    }

    /// What each Model menu item would act on now, or why it is disabled.
    pub fn model_targets(&self) -> crate::model_menu::ModelTargets {
        crate::model_menu::ModelTargets::compute(
            self.surface.as_ref(),
            self.picked_points(),
            self.picked_face(),
            self.optimise_job.is_some(),
            self.selected(),
        )
    }

    /// Apply a Model menu action: the dialogs open, the topology tools go
    /// through the typed commands (one undo step each; a refusal is logged
    /// as an error).
    pub fn apply_model_action(&mut self, action: crate::model_menu::ModelAction) {
        use crate::commands as c;
        use crate::model_menu::ModelAction as A;
        let result = match action {
            A::None => return,
            A::Transform => {
                self.open_transform_dialog();
                return;
            }
            A::Optimise => {
                self.open_optimise_dialog();
                return;
            }
            A::InsertEdge { face, a, b } => {
                c::insert_edge(self, &c::InsertEdgeParams { face, a, b }).map(|_| ())
            }
            A::SplitFace(face) => c::split_face(self, &c::SplitFaceParams { face }).map(|_| ()),
            A::Crease { edge, crease } => {
                c::set_crease(self, &c::CreaseParams { edge, crease }).map(|_| ())
            }
            A::SelectLine(kind) => c::select_line(self, kind).map(|_| ()),
            A::ReloadPresets => {
                self.reload_presets();
                return;
            }
            A::CreateExamplePresets => {
                self.create_example_presets();
                return;
            }
        };
        if let Err(e) = result {
            self.log(Level::Error, e);
        }
    }

    /// Open Model > Transform…, its fit defaults measured on the hull now.
    pub fn open_transform_dialog(&mut self) {
        let subdiv = seacraft_core::subdiv::SubdivisionOptions {
            level: self.level,
            mode: self.mode,
        };
        let current = self
            .surface
            .as_ref()
            .and_then(|s| seacraft_core::transform::MainDimensions::measure(s, subdiv));
        self.transform_dialog = Some(crate::model_menu::TransformDialog::new(current));
    }

    /// Open Model > Optimise… (a running optimisation keeps its window).
    pub fn open_optimise_dialog(&mut self) {
        if self.optimise_dialog.is_some() {
            return;
        }
        let speed = self
            .project
            .requirements
            .as_ref()
            .and_then(|r| r.design_speed_kn)
            .unwrap_or(8.0);
        self.optimise_dialog = Some(crate::model_menu::OptimiseDialog::new(
            speed,
            self.level.min(4),
            self.hydro.kg,
        ));
    }

    /// The Transform dialog's fields while it is open (tests).
    pub fn transform_dialog_mut(&mut self) -> Option<&mut crate::model_menu::TransformDialog> {
        self.transform_dialog.as_mut()
    }

    /// The Optimise window's fields while it is open (tests).
    pub fn optimise_dialog_mut(&mut self) -> Option<&mut crate::model_menu::OptimiseDialog> {
        self.optimise_dialog.as_mut()
    }

    /// An optimisation is running on the job layer.
    pub fn optimise_running(&self) -> bool {
        self.optimise_job.is_some()
    }

    /// The two picked control points, first then second (click one,
    /// Shift+click the other).
    pub fn picked_points(&self) -> Option<[usize; 2]> {
        self.selection.pair()
    }

    /// Pick two control points as a click and a Shift+click would (tests,
    /// scripts); `None` clears both.
    pub fn set_picked_points(&mut self, points: Option<[usize; 2]>) {
        self.selection.clear();
        if let Some([first, second]) = points {
            self.selection.set_only(first);
            self.selection.toggle(second);
        }
    }

    /// The world axis the point drag is locked to, while X, Y or Z is held.
    pub fn axis_lock(&self) -> Option<seacraft_core::transform::Axis> {
        self.axis_lock
    }

    /// Start an optimisation on the job layer; its result is installed by
    /// [`App::poll_optimise`] as one undo step.
    pub fn start_optimise(&mut self, spec: seacraft_core::optimize::OptimizeSpec, level: u8) {
        let kg = spec.kg;
        let params = crate::commands::OptimizeParams {
            spec,
            level: Some(level),
        };
        match crate::commands::prepare_optimize(self, &params) {
            Ok(run) => {
                let level = run.level;
                let job = Job::spawn(self.revision, move || run.run());
                self.optimise_job = Some(OptimiseJob { job, level, kg });
                if let Some(d) = self.optimise_dialog.as_mut() {
                    d.error = None;
                    d.result = None;
                }
                self.set_status("optimising the hull…".to_owned());
            }
            Err(e) => {
                if let Some(d) = self.optimise_dialog.as_mut() {
                    d.error = Some(e.clone());
                }
                self.log(Level::Error, e);
            }
        }
    }

    /// Collect a finished optimisation. A result for a model edited since
    /// the run started is discarded rather than installed over the edit.
    fn poll_optimise(&mut self, ctx: &egui::Context) {
        let Some(running) = self.optimise_job.as_ref() else {
            return;
        };
        let outcome = match running.job.poll() {
            crate::jobs::Poll::Running => {
                ctx.request_repaint_after(std::time::Duration::from_millis(100));
                return;
            }
            crate::jobs::Poll::Done(v) => Ok(v),
            crate::jobs::Poll::Failed(e) => Err(e),
        };
        let Some(OptimiseJob { job, level, kg }) = self.optimise_job.take() else {
            return;
        };
        let message = match outcome {
            Ok((surface, report)) if job.revision() == self.revision => {
                let r = crate::commands::install_optimized(self, surface, report, level, kg);
                Ok(format!(
                    "R_T {:+.1} % at {:.1} kn after {} evaluations{}; draft now {:.3} m",
                    r.report.rt_change_pct,
                    r.report.speed_kn,
                    r.report.evaluations,
                    if r.report.converged {
                        ""
                    } else {
                        " (not converged)"
                    },
                    r.draft
                ))
            }
            Ok(_) => Err("the model changed while optimising; the result was discarded".to_owned()),
            Err(e) => Err(e),
        };
        match message {
            Ok(m) => {
                if let Some(d) = self.optimise_dialog.as_mut() {
                    d.result = Some(m);
                }
            }
            Err(e) => {
                if let Some(d) = self.optimise_dialog.as_mut() {
                    d.error = Some(e.clone());
                }
                self.log(Level::Error, e);
            }
        }
    }

    /// The Transform and Optimise dialogs; what they ask for is applied
    /// here, after they have drawn.
    fn model_windows(&mut self, ui: &mut egui::Ui) {
        use crate::model_menu::{DialogAction, OptimiseAction};
        self.poll_optimise(ui.ctx());
        let transform = match self.transform_dialog.as_mut() {
            Some(state) => crate::model_menu::transform_dialog(ui.ctx(), state),
            None => DialogAction::None,
        };
        match transform {
            DialogAction::None => {}
            DialogAction::Cancel => self.transform_dialog = None,
            DialogAction::Apply(request) => match crate::commands::transform(self, &request) {
                Ok(_) => self.transform_dialog = None,
                Err(e) => {
                    if let Some(d) = self.transform_dialog.as_mut() {
                        d.error = Some(e);
                    }
                }
            },
        }
        let running = self.optimise_job.is_some();
        let optimise = match self.optimise_dialog.as_mut() {
            Some(state) => crate::model_menu::optimise_dialog(ui.ctx(), state, running),
            None => OptimiseAction::None,
        };
        match optimise {
            OptimiseAction::None => {}
            OptimiseAction::Close => {
                self.optimise_dialog = None;
                if self.optimise_job.take().is_some() {
                    self.set_status("optimisation stopped; the hull is unchanged".to_owned());
                }
            }
            OptimiseAction::Stop => {
                self.optimise_job = None;
                self.set_status("optimisation stopped; the hull is unchanged".to_owned());
            }
            OptimiseAction::Run(spec, level) => self.start_optimise(*spec, level),
        }
    }

    /// What the Model menu's tools would act on, drawn over one viewport:
    /// the picked face's outline, and the axis a locked drag follows with
    /// its caption.
    fn draw_pick_overlays(&self, ui: &egui::Ui, cell: egui::Rect, index: usize) {
        let Some(surface) = self.surface.as_ref() else {
            return;
        };
        let view = self.cameras[index].transform();
        let painter = ui.painter_at(cell);
        if let Some(face) = self.picked_face() {
            let pts: Vec<egui::Pos2> = surface.faces[face]
                .iter()
                .filter_map(|&i| view.project(surface.points[i].p, cell))
                .collect();
            if pts.len() >= 3 && pts.len() == surface.faces[face].len() {
                painter.add(egui::Shape::closed_line(
                    pts,
                    egui::Stroke::new(1.5, crate::theme::FACE_PICKED),
                ));
            }
        }
        if let Some((view_idx, from, to)) = self.marquee {
            if view_idx == index {
                let rect = egui::Rect::from_two_pos(from, to);
                if rect.width() >= 4.0 || rect.height() >= 4.0 {
                    let corners = [
                        rect.left_top(),
                        rect.right_top(),
                        rect.right_bottom(),
                        rect.left_bottom(),
                        rect.left_top(),
                    ];
                    painter.extend(egui::Shape::dashed_line(
                        &corners,
                        egui::Stroke::new(1.5, crate::theme::POINT_SELECTED),
                        6.0,
                        4.0,
                    ));
                }
            }
        }
        let (Some(axis), Some(drag)) = (self.axis_lock, self.drag.as_ref()) else {
            return;
        };
        if drag.view != index {
            return;
        }
        let span = self
            .cache
            .as_ref()
            .map_or(10.0, |c| (c.max - c.min).length().max(1.0));
        let dir = crate::model_menu::axis_vector(axis) * span;
        if let (Some(a), Some(b)) = (
            view.project(drag.original - dir, cell),
            view.project(drag.original + dir, cell),
        ) {
            painter.line_segment(
                [a, b],
                egui::Stroke::new(1.0, crate::model_menu::axis_colour(axis)),
            );
        }
        render::hud_caption(
            &painter,
            cell.center_bottom() + egui::vec2(0.0, -8.0),
            egui::Align2::CENTER_BOTTOM,
            format!(
                "Locked to {} — release the key to drag freely",
                crate::model_menu::axis_name(axis)
            ),
        );
    }
}

impl eframe::App for App {
    /// Keep the recent list in eframe's storage (called on shutdown and
    /// periodically when eframe has its `persistence` feature), and in ours.
    fn save(&mut self, storage: &mut dyn eframe::Storage) {
        self.recent.save(storage);
        self.connected_choice().save(storage);
        self.persist_recent();
    }

    /// Clean shutdown: join any in-flight autosave write and remove the
    /// session's `.json` and `.lock` files.
    fn on_exit(&mut self) {
        self.persist_recent();
        self.autosave_shutdown();
    }

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

// ---- periodic autosave and startup crash recovery -------------------------

impl App {
    /// Enable autosave in `dir` and scan `dir` for orphaned sessions.
    pub fn set_autosave_dir(&mut self, dir: PathBuf) {
        let warnings = self
            .autosave
            .set_dir(dir, self.dirty && self.surface.is_some());
        for w in warnings {
            self.log(Level::Warning, w);
        }
    }

    /// Start autosave in the platform's directory and offer what a crashed
    /// session left there. Called by the binary unless `--no-autosave`, so
    /// that flag and headless apps never touch the directory.
    pub fn enable_autosave(&mut self) {
        if let Some(dir) = crate::autosave::autosave_dir() {
            self.set_autosave_dir(dir);
        }
    }

    /// The directory where this app writes autosaves, or `None` when disabled.
    pub fn autosave_dir(&self) -> Option<&std::path::Path> {
        self.autosave.dir()
    }

    /// This app's unique autosave session identifier (`session-<pid>-<unix-nanos>`).
    pub fn autosave_session(&self) -> &str {
        self.autosave.session()
    }

    /// Whether a background autosave write thread is currently running.
    pub fn autosave_in_flight(&self) -> bool {
        self.autosave.in_flight()
    }

    /// Block until any in-flight autosave write finishes and log its outcome
    /// (test helper: joins the worker thread without sleeping).
    pub fn wait_autosave(&mut self) {
        for (level, msg) in self.autosave.join_write() {
            self.log(level, msg);
        }
    }

    /// Run one autosave state-machine tick for `now`.
    pub fn autosave_tick(&mut self, now: std::time::Instant) {
        let dirty = self.dirty && self.surface.is_some();
        let dragging = self.drag.is_some();
        let revs = (self.revision, self.project_rev);
        let surface = &self.surface;
        let project = &self.project;
        let path = &self.path;
        let report = self.autosave.tick(now, dirty, dragging, revs, || {
            let s = surface.as_ref()?;
            let fef_text = fef::write(s);
            let proj_text = project.to_json();
            let orig = path.as_deref().and_then(crate::autosave::snapshot_path);
            Some((fef_text, proj_text, orig))
        });
        for (level, msg) in report.logs {
            self.log(level, msg);
        }
    }

    /// Clean shutdown: join any in-flight write and delete both session files.
    pub fn autosave_shutdown(&mut self) {
        self.autosave.shutdown();
    }

    /// Whether the startup crash-recovery modal is open.
    pub fn autosave_prompt_open(&self) -> bool {
        !self.autosave.orphans.is_empty()
    }

    /// The orphan currently shown at the front of the recovery prompt, if any.
    pub fn current_orphan(&self) -> Option<&crate::autosave::Orphan> {
        self.autosave.orphans.first()
    }

    /// All orphans remaining in the recovery queue, newest first.
    pub fn orphans(&self) -> &[crate::autosave::Orphan] {
        &self.autosave.orphans
    }

    /// Recover the current orphan from the front of the recovery queue.
    pub fn recover_orphan(&mut self) {
        if self.autosave.orphans.is_empty() {
            return;
        }
        if self.dirty {
            self.log(
                Level::Warning,
                "cannot recover autosave while the current model has unsaved changes; save or discard first".to_owned(),
            );
            return;
        }
        let orphan = self.autosave.orphans.remove(0);
        let surface = match fef::parse(&orphan.fef) {
            Ok(s) => s,
            Err(err) => {
                self.log(
                    Level::Warning,
                    format!(
                        "cannot recover autosave {}: invalid .fef ({err}); keeping file on disk",
                        orphan.json_path.display()
                    ),
                );
                return;
            }
        };
        let (project, project_bad) = match Project::from_json(&orphan.project) {
            Ok(p) => (p, None),
            Err(e) => (Project::default(), Some(e)),
        };
        let saved_state = orphan.original_path.as_deref().and_then(|orig| {
            let text = std::fs::read_to_string(orig).ok()?;
            let parsed = fef::parse(&text).ok()?;
            let orig_proj = Project::load_for_model(orig).ok()?.unwrap_or_default();
            Some((fef::write(&parsed), orig_proj))
        });
        self.surface = Some(surface);
        self.path = orphan.original_path.clone();
        self.project = project;
        // As `load` does: an unreadable sidecar next to the file stays
        // untouched by a later save.
        self.project_error = orphan
            .original_path
            .as_deref()
            .and_then(|orig| Project::load_for_model(orig).err());
        self.saved_state = saved_state;
        self.dirty = true;
        self.autosave.note_model_replaced(true);
        self.adopt_project_kg();
        self.adopt();
        self.apply_requirement_defaults();

        let own_fef = self.surface.as_ref().map(fef::write).unwrap_or_default();
        let own_proj = self.project.to_json();
        let orig_str = self
            .path
            .as_deref()
            .and_then(crate::autosave::snapshot_path);
        let write_res = self.autosave.write_sync(
            own_fef,
            own_proj,
            orig_str,
            (self.revision, self.project_rev),
        );

        if let Some(proj_err) = project_bad {
            self.log(
                Level::Warning,
                format!(
                    "recovered geometry `{}` from autosave, but project could not be parsed ({proj_err}); using default project and keeping {} on disk",
                    orphan.model_name,
                    orphan.json_path.display()
                ),
            );
            if let Err(err) = write_res {
                self.log(
                    Level::Warning,
                    format!(
                        "could not write fresh autosave snapshot ({err}); keeping {}",
                        orphan.json_path.display()
                    ),
                );
            }
        } else {
            let target_desc = orphan
                .original_path
                .as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "untitled".to_owned());
            self.log(
                Level::Info,
                format!(
                    "recovered `{}` ({target_desc}) from autosave {}",
                    orphan.model_name,
                    orphan.json_path.display()
                ),
            );
            match write_res {
                Ok(()) => orphan.remove_files(),
                Err(err) => self.log(
                    Level::Warning,
                    format!(
                        "could not write fresh autosave snapshot ({err}); keeping orphan {}",
                        orphan.json_path.display()
                    ),
                ),
            }
        }

        if !self.autosave.orphans.is_empty() {
            let remaining = self.autosave.orphans.len();
            self.autosave.orphans.clear();
            let word = if remaining == 1 {
                "autosave"
            } else {
                "autosaves"
            };
            self.log(
                Level::Info,
                format!("{remaining} other {word} kept on disk for the next start"),
            );
        }
    }

    /// Discard the current orphan, deleting its files and advancing to the
    /// next one.
    pub fn discard_orphan(&mut self) {
        if self.autosave.orphans.is_empty() {
            return;
        }
        let orphan = self.autosave.orphans.remove(0);
        orphan.remove_files();
        self.log(
            Level::Info,
            format!("discarded autosave {}", orphan.json_path.display()),
        );
    }

    /// Keep all remaining orphans on disk and close the recovery prompt; they
    /// will be offered again at the next start.
    pub fn defer_orphans(&mut self) {
        if self.autosave.orphans.is_empty() {
            return;
        }
        let count = self.autosave.orphans.len();
        self.autosave.orphans.clear();
        let word = if count == 1 { "autosave" } else { "autosaves" };
        self.log(
            Level::Info,
            format!("deferred {count} {word} to the next start"),
        );
    }

    fn recovery_window(&mut self, ctx: &egui::Context) {
        let Some(orphan) = self.autosave.orphans.first() else {
            return;
        };
        let remaining = self.autosave.orphans.len();
        let action = crate::autosave::recovery_modal(ctx, orphan, remaining);
        match action {
            crate::autosave::RecoveryAction::None => {}
            crate::autosave::RecoveryAction::Recover => self.recover_orphan(),
            crate::autosave::RecoveryAction::Discard => self.discard_orphan(),
            crate::autosave::RecoveryAction::Defer => self.defer_orphans(),
        }
    }
}

// ---- the Analysis menu's views (R5-8) -------------------------------------
//
// Hydrostatic table, Bonjean curves, offsets, trim scan and plate
// development: each opens in the wide Analysis dock instead of the tab body
// (a tab click goes back), computes on a job from a snapshot of the model,
// and follows the recompute rule as a heavy result: manual (Compute or F5),
// stale once its key moves on.

impl App {
    /// The Analysis menu: one item per view.
    fn analysis_menu(&mut self, ui: &mut egui::Ui) {
        use crate::analysis_views::AnalysisView;
        ui.menu_button("Analysis", |ui| {
            for view in AnalysisView::ALL {
                // Tables and curves of the hull, then the studies.
                if view == AnalysisView::TrimScan {
                    ui.separator();
                }
                let item = egui::Button::selectable(self.analysis.open == Some(view), view.label());
                if ui.add_enabled(self.surface.is_some(), item).clicked() {
                    self.open_analysis_view(view);
                    ui.close();
                }
            }
        });
    }

    /// The open Analysis view, in a drawer under the viewports: a table is
    /// wide, so it takes the width of the viewports and leaves the docks and
    /// their widths alone. Its header names the view and carries its close
    /// button, so what closes is what the header is on.
    fn analysis_drawer(&mut self, ui: &mut egui::Ui) {
        let Some(view) = self.analysis.open else {
            return;
        };
        if self.surface.is_none() {
            return;
        }
        if view == crate::analysis_views::AnalysisView::Waves {
            self.wave_view_drawer(ui);
            return;
        }
        if view == crate::analysis_views::AnalysisView::Heel {
            self.heel_view_drawer(ui);
            return;
        }
        let (stale, busy, failure, layers, design_draft) = self.analysis_view_panel_inputs(view);
        let window = ui.ctx().content_rect().height();
        let mut action = crate::analysis_views::ViewAction::None;
        egui::Panel::bottom("analysis_drawer")
            .resizable(true)
            .default_size((window * 0.42).round())
            .size_range(160.0..=(window * 0.8).round())
            .show(ui, |ui| {
                // Keep the drawer's height while a view has little to show
                // (computing, or no result yet), so it does not jump.
                ui.set_min_height(ui.available_height());
                action = crate::analysis_views::panel(
                    ui,
                    crate::analysis_views::ViewPanel {
                        view,
                        state: &mut self.analysis,
                        stale,
                        busy,
                        failure,
                        layers,
                        design_draft,
                    },
                );
            });
        self.apply_view_action(action, ui.ctx());
    }

    /// The Wave pattern view in the drawer: its panel edits a copy of the wave settings and returns
    /// what else the user asked for, applied here after the panel.
    fn wave_view_drawer(&mut self, ui: &mut egui::Ui) {
        use crate::wave_view::{WaveViewAction, WaveViewPanel};
        self.load_wave_texture(ui.ctx());
        let texture = self
            .wave_texture
            .as_ref()
            .filter(|(k, _)| *k == self.wave_result_rev)
            .map(|(_, t)| t.id());
        let window = ui.ctx().content_rect().height();
        let current = self.wave_current().is_some();
        let stale = self.waves_stale();
        let busy = self.waves_track.busy() || self.wave_drag_track.busy();
        let failure = self
            .wave_failure()
            .or_else(|| self.wave_drag_failure())
            .map(str::to_owned);
        let heeled_or_trimmed =
            self.hydro.options.heel_deg != 0.0 || self.hydro.options.trim != 0.0;
        let measured = self.perf_state.wave_result.as_ref();
        let length_wl = measured
            .map(|w| w.length_wl)
            .or_else(|| self.hydro.result.as_ref().map(|h| h.lwl));
        let beam_wl = measured
            .map(|w| w.beam_wl)
            .or_else(|| self.hydro.result.as_ref().map(|h| h.bwl));
        let drag = self.wave_drag();
        let before = self.perf_state.waves.clone();
        let mut settings = before.clone();
        let mut map = self.wave_map;
        let mut action = WaveViewAction::None;
        egui::Panel::bottom("analysis_drawer")
            .resizable(true)
            .default_size((window * 0.42).round())
            .size_range(160.0..=(window * 0.8).round())
            .show(ui, |ui| {
                // Keep the drawer's height while there is little to show.
                ui.set_min_height(ui.available_height());
                action = crate::wave_view::panel(
                    ui,
                    WaveViewPanel {
                        settings: &mut settings,
                        result: measured,
                        drag,
                        stale,
                        busy,
                        current,
                        failure: failure.as_deref(),
                        heeled_or_trimmed,
                        length_wl,
                        beam_wl,
                        texture,
                        map: &mut map,
                    },
                );
            });
        self.wave_map = map;
        match action {
            WaveViewAction::None | WaveViewAction::Changed => {}
            WaveViewAction::SaveImage => self.open_dialog(DialogKind::WaveImage),
            WaveViewAction::Close => self.close_analysis_view(),
            WaveViewAction::Open(other) => self.open_analysis_view(other),
            WaveViewAction::SetSpeed(speed) => settings.speed = speed,
            WaveViewAction::SetSpacing(spacing) => settings.cat_spacing_m = Some(spacing),
            WaveViewAction::SetWashDistance(d) => settings.wash_distance_m = Some(d),
        }
        if settings != before {
            self.set_wave_settings(settings);
        }
    }

    /// The Heel view in the drawer under the viewports.
    fn heel_view_drawer(&mut self, ui: &mut egui::Ui) {
        use crate::heel_view::{HeelStart, HeelViewAction, HeelViewPanel, KgNote};
        let window = ui.ctx().content_rect().height();
        if self.heel_animate(ui.input(|i| i.time)) {
            ui.ctx().request_repaint();
        }
        let stale = self.heel_stale();
        // The condition drawn: current, or kept and dimmed while it recomputes.
        let sweep = if stale {
            self.heel_sweep.clone()
        } else {
            self.heel_current().cloned()
        };
        let exact = self.heel_exact_current().cloned();
        let shown = match (&exact, &sweep) {
            (Some(e), _) if !stale => Some(e.clone()),
            (_, Some(out)) => out.nearest(self.heel.heel_deg).cloned(),
            _ => None,
        };
        let shown_exact = exact.is_some() && !stale;
        let hull = self.heel_hull.clone();
        // The strong section, cut for its x to the millimetre.
        let section_x = self
            .heel
            .section
            .or(shown.as_ref().map(|s| s.b.x))
            .or_else(|| {
                hull.as_ref()
                    .map(|h| 0.5 * (h.outlines.min.x + h.outlines.max.x))
            });
        if let (Some(x), Some(h)) = (section_x, &hull) {
            let key = revision_key(&[self.heel_hull_key(), (x * 1000.0).round() as i64 as u64]);
            if self.heel_section.as_ref().is_none_or(|(k, _)| *k != key) {
                let cut = crate::heel_view::geometry::HullOutlines::section_at(&h.hull, x);
                self.heel_section = Some((key, cut));
            }
        }
        let section = self.heel_section.as_ref().map(|(_, s)| s.clone());
        let kg_note = match (self.heel.start, self.hydro.kg) {
            (HeelStart::Condition(_), _) => KgNote::Typed,
            (HeelStart::Dock, None) => KgNote::Unset,
            (HeelStart::Dock, Some(_)) if self.kg_is_estimate() => KgNote::Estimate,
            (HeelStart::Dock, Some(_)) => KgNote::Typed,
        };
        let dock_plane = hull.as_ref().map(|h| {
            seacraft_core::math::Plane::new(
                seacraft_core::math::Vec3::new(0.0, 0.0, 1.0),
                -(h.hull.keel + self.hydro.options.draft),
            )
        });
        let conditions: Vec<String> = self
            .project
            .loading_conditions
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let failure = self.heel_failure().map(str::to_owned);
        let busy = self.heel_busy();
        let mut state = self.heel.clone();
        let mut action = HeelViewAction::None;
        let mut drawn = None;
        egui::Panel::bottom("analysis_drawer")
            .resizable(true)
            .default_size((window * 0.42).round())
            .size_range(160.0..=(window * 0.8).round())
            .show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                action = crate::heel_view::panel(
                    ui,
                    HeelViewPanel {
                        state: &mut state,
                        shown: shown.as_ref(),
                        shown_exact,
                        sweep: sweep.as_ref(),
                        outlines: hull.as_ref().map(|h| h.outlines.as_ref()),
                        section: section.as_deref(),
                        dock_plane,
                        drawn: &mut drawn,
                        stale,
                        busy,
                        failure: failure.as_deref(),
                        kg_note,
                        conditions: &conditions,
                    },
                );
            });
        self.heel_drawn = drawn;
        if state != self.heel {
            if state.heel_deg != self.heel.heel_deg {
                self.heel_moved = Some(std::time::Instant::now());
            }
            // A new condition settles at its new equilibrium once computed.
            if state.weights != self.heel.weights || state.start != self.heel.start {
                self.heel_settle_pending = true;
            }
            self.heel = state;
            self.check_heel_start();
            ui.ctx().request_repaint();
        }
        match action {
            HeelViewAction::Close => self.close_analysis_view(),
            HeelViewAction::Open(other) => self.open_analysis_view(other),
            HeelViewAction::SettleNow => self.heel_settle_pending = true,
            HeelViewAction::AddToCondition(i) => self.heel_add_to_condition(i),
            HeelViewAction::None | HeelViewAction::Changed => {}
        }
        if self.heel.hold {
            self.heel_settle_pending = false;
        }
    }

    /// Show `view` in the drawer under the viewports. Opening a view that
    /// has no result yet computes it.
    pub fn open_analysis_view(&mut self, view: crate::analysis_views::AnalysisView) {
        self.analysis.open = Some(view);
        if !view.uses_slot() {
            // The wave pattern computes on its own jobs while the view is open.
            return;
        }
        let key = self.analysis_key(view);
        let slot = self.analysis.slot(view);
        if slot.result.is_none() && slot.track.due(key) {
            self.start_analysis_view(view);
        }
    }

    /// Close the drawer. The viewports take its height back.
    pub fn close_analysis_view(&mut self) {
        self.analysis.open = None;
    }

    /// The view shown in the Analysis dock, if any.
    pub fn analysis_view(&self) -> Option<crate::analysis_views::AnalysisView> {
        self.analysis.open
    }

    /// The views' inputs.
    pub fn analysis_inputs(&self) -> &crate::analysis_views::ViewInputs {
        &self.analysis.inputs
    }

    /// Change a view's inputs (tests; the panel edits them in place): its
    /// result goes stale.
    pub fn edit_analysis_inputs(
        &mut self,
        view: crate::analysis_views::AnalysisView,
        edit: impl FnOnce(&mut crate::analysis_views::ViewInputs),
    ) {
        edit(&mut self.analysis.inputs);
        self.analysis.slot_mut(view).inputs_rev += 1;
    }

    /// The key a view's result is computed for: the geometry and the
    /// subdivision (`revision`), the view's inputs and, for the hydrostatic
    /// table and the trim scan, the floating condition they measure at.
    fn analysis_key(&self, view: crate::analysis_views::AnalysisView) -> u64 {
        let mut parts = vec![
            view as u64,
            self.revision,
            self.analysis.slot(view).inputs_rev,
        ];
        if view.uses_floating_condition() {
            let o = &self.hydro.options;
            parts.extend([
                o.draft.to_bits(),
                o.trim.to_bits(),
                o.heel_deg.to_bits(),
                o.water_density.to_bits(),
                o.appendage_coefficient.to_bits(),
                o.mainframe_x.map_or(u64::MAX, f64::to_bits),
            ]);
        }
        revision_key(&parts)
    }

    /// Compute a view on a job (the shown result stays, dimmed, until the
    /// new one lands).
    pub fn start_analysis_view(&mut self, view: crate::analysis_views::AnalysisView) {
        if self.surface.is_none() || !view.uses_slot() {
            return;
        }
        let key = self.analysis_key(view);
        let model = crate::commands::ModelSnapshot::of(self);
        let inputs = self.analysis.inputs.clone();
        self.set_status(format!("computing {}…", view.label().to_lowercase()));
        self.analysis.slot_mut(view).track.start(key, move || {
            crate::analysis_views::compute(view, &inputs, &model)
        });
    }

    /// Take finished view jobs; a failure goes to the status line and stays
    /// on the view until its inputs change.
    fn poll_analysis_views(&mut self) {
        for view in crate::analysis_views::AnalysisView::ALL {
            let key = self.analysis_key(view);
            match self.analysis.slot_mut(view).track.poll(key) {
                Some(Ok(r)) => self.install_analysis_view(view, r),
                Some(Err(e)) => self.log(Level::Error, format!("{} failed: {e}", view.label())),
                None => {}
            }
        }
    }

    fn install_analysis_view(
        &mut self,
        view: crate::analysis_views::AnalysisView,
        result: crate::analysis_views::ViewResult,
    ) {
        let rows = crate::analysis_views::table(&result).rows.len();
        self.analysis.slot_mut(view).result = Some(result);
        self.set_status(format!("{}: {rows} rows", view.label()));
    }

    /// Block until a view's job ends and take its result (tests).
    pub fn wait_analysis_view(&mut self, view: crate::analysis_views::AnalysisView) {
        let key = self.analysis_key(view);
        match self.analysis.slot_mut(view).track.wait(key) {
            Some(Ok(r)) => self.install_analysis_view(view, r),
            Some(Err(e)) => self.log(Level::Error, format!("{} failed: {e}", view.label())),
            None => {}
        }
    }

    /// A view's result, whether current or stale.
    pub fn analysis_view_result(
        &self,
        view: crate::analysis_views::AnalysisView,
    ) -> Option<&crate::analysis_views::ViewResult> {
        self.analysis.slot(view).result.as_ref()
    }

    /// A view's result as its engineering table.
    pub fn analysis_view_table(
        &self,
        view: crate::analysis_views::AnalysisView,
    ) -> Option<crate::analysis_views::ViewTable> {
        self.analysis_view_result(view)
            .map(crate::analysis_views::table)
    }

    /// A view's table as the CSV *Copy CSV* puts on the clipboard.
    pub fn analysis_view_csv(&self, view: crate::analysis_views::AnalysisView) -> Option<String> {
        self.analysis_view_table(view)
            .map(|t| crate::analysis_views::to_csv(&t))
    }

    /// A view shows a result computed for other inputs.
    pub fn analysis_view_stale(&self, view: crate::analysis_views::AnalysisView) -> bool {
        let slot = self.analysis.slot(view);
        slot.result.is_some() && !slot.track.is_current(self.analysis_key(view))
    }

    /// A view's job is running.
    pub fn analysis_view_busy(&self, view: crate::analysis_views::AnalysisView) -> bool {
        self.analysis.slot(view).track.busy()
    }

    /// Why computing a view failed for its current inputs, if it did.
    pub fn analysis_view_failure(&self, view: crate::analysis_views::AnalysisView) -> Option<&str> {
        self.analysis
            .slot(view)
            .track
            .failure(self.analysis_key(view))
    }

    /// The open view shows a stale result (counted in the status bar).
    fn open_view_stale(&self) -> bool {
        self.analysis
            .open
            .is_some_and(|v| self.analysis_view_stale(v))
    }

    /// What the view's panel needs besides the views' own state: stale,
    /// busy, failure, the layers and the design draft.
    fn analysis_view_panel_inputs(
        &self,
        view: crate::analysis_views::AnalysisView,
    ) -> (bool, bool, Option<String>, Vec<(String, bool)>, f64) {
        let layers = self.surface.as_ref().map_or_else(Vec::new, |s| {
            s.layers
                .iter()
                .map(|l| (l.name.clone(), l.developable))
                .collect()
        });
        (
            self.analysis_view_stale(view),
            self.analysis_view_busy(view),
            self.analysis_view_failure(view).map(str::to_owned),
            layers,
            self.hydro.options.draft,
        )
    }

    /// Apply what a view's panel asked for.
    fn apply_view_action(
        &mut self,
        action: crate::analysis_views::ViewAction,
        ctx: &egui::Context,
    ) {
        use crate::analysis_views::ViewAction;
        let Some(view) = self.analysis.open else {
            return;
        };
        match action {
            ViewAction::None => {}
            ViewAction::Changed => self.analysis.slot_mut(view).inputs_rev += 1,
            ViewAction::Compute => self.start_analysis_view(view),
            ViewAction::CopyCsv => {
                if let Some(csv) = self.analysis_view_csv(view) {
                    let rows = csv.lines().count().saturating_sub(1);
                    ctx.copy_text(csv);
                    self.set_status(format!("{}: {rows} rows copied as CSV", view.label()));
                }
            }
            ViewAction::Close => self.close_analysis_view(),
            ViewAction::Open(other) => self.open_analysis_view(other),
            ViewAction::OpenBuilderPanels => self.open_builder_panels(),
        }
    }
}
