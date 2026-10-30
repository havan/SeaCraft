//! Typed model and analysis operations on the [`App`].
//!
//! Each operation is a plain params struct and one function that returns a
//! typed, serialisable result, so the control channel ([`crate::control`]) and
//! the GUI menus share one implementation. The control channel keeps the JSON
//! side: it parses and range-checks the request into the params struct and
//! turns the result into its reply.
//!
//! Mutating commands follow the round-3 rule: the edit is made on a copy, so a
//! refused edit leaves no undo entry; an accepted one is exactly one undo
//! snapshot and marks the geometry changed. Read-only commands (tables, curves,
//! plate development) never snapshot and never dirty the model.

use std::path::PathBuf;

use seacraft_core::curves::{Bonjean, BonjeanSection};
use seacraft_core::develop::{BandAxis, PatchSelection, Unfolded};
use seacraft_core::offsets::{OffsetStation, TableOfOffsets};
use seacraft_core::optimize::{OptimizeReport, OptimizeSpec};
use seacraft_core::parametric::{GenerateReport, HullSpec};
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_core::table::HydrostaticRow;
use seacraft_core::transform::{TransformReport, TransformRequest};
use seacraft_core::trimopt::{TrimPoint, TrimScan};
use serde::Serialize;

use crate::App;

/// `level`, or the app's own subdivision level when `None`.
fn level_or_app(app: &App, level: Option<u8>) -> u8 {
    level.unwrap_or_else(|| app.subdivision_level())
}

/// The app's subdivision mode at `level`.
fn subdivision_at(app: &App, level: u8) -> SubdivisionOptions {
    SubdivisionOptions {
        level,
        mode: app.subdivision_mode(),
    }
}

// ---- model.generate ------------------------------------------------------

/// The result of [`generate`]: the generator's report plus the new model's size.
#[derive(Debug, Clone, Serialize)]
pub struct GenerateResult {
    #[serde(flatten)]
    pub report: GenerateReport,
    pub points: usize,
    pub faces: usize,
    pub layers: usize,
}

/// Replace the model with a parametric hull built from `spec`.
pub fn generate(app: &mut App, spec: &HullSpec) -> Result<GenerateResult, String> {
    let (surface, report) = seacraft_core::parametric::generate(spec)?;
    Ok(install_generated(app, surface, report))
}

/// Replace the model with a hull [`seacraft_core::parametric::generate`]
/// already built (the New design dialog's preview), as [`generate`] does.
pub fn install_generated(
    app: &mut App,
    surface: seacraft_core::Surface,
    report: GenerateReport,
) -> GenerateResult {
    app.load_generated(surface);
    GenerateResult {
        report,
        points: app.control_point_count(),
        faces: app.face_count(),
        layers: app.layer_count(),
    }
}

// ---- edge.crease ---------------------------------------------------------

/// Set or clear the crease flag on the control edge between two points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CreaseParams {
    /// The edge's two control point indices, in either order.
    pub edge: [usize; 2],
    /// `true` creases the edge, `false` smooths it.
    pub crease: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct CreaseResult {
    /// The edge, lower index first.
    pub edge: [usize; 2],
    pub crease: bool,
    pub undo_len: usize,
}

/// Crease (or smooth) one control edge: one undo step, geometry changed.
pub fn set_crease(app: &mut App, params: &CreaseParams) -> Result<CreaseResult, String> {
    let surface = app.surface().ok_or("no model loaded")?;
    let count = surface.points.len();
    if params.edge.iter().any(|&i| i >= count) {
        return Err(format!(
            "an `edge` index must be from 0 to {}",
            count.saturating_sub(1)
        ));
    }
    let edge = seacraft_core::mesh::ordered(params.edge[0], params.edge[1]);
    if !surface.edges.iter().any(|e| e.key() == edge) {
        return Err(format!("no control edge between {} and {}", edge.0, edge.1));
    }
    let crease = params.crease;
    app.snapshot_undo();
    if let Some(surface) = app.surface_mut() {
        for e in &mut surface.edges {
            if e.key() == edge {
                e.crease = crease;
            }
        }
    }
    app.mark_geometry_changed();
    app.status_note(format!("edge {}-{} crease {crease}", edge.0, edge.1));
    Ok(CreaseResult {
        edge: [edge.0, edge.1],
        crease,
        undo_len: app.undo_len(),
    })
}

// ---- model.split_face ----------------------------------------------------

/// Fan a face into triangles around a new centroid point.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SplitFaceParams {
    pub face: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct SplitFaceResult {
    /// The face that was split (it keeps its index).
    pub face: usize,
    /// The new centroid control point.
    pub point: usize,
    /// How many faces the original became.
    pub pieces: usize,
    /// Faces and points in the model after the split.
    pub faces: usize,
    pub points: usize,
    pub undo_len: usize,
}

/// Split one face. A refused split leaves the model and the undo stack as
/// they were.
pub fn split_face(app: &mut App, params: &SplitFaceParams) -> Result<SplitFaceResult, String> {
    let face = params.face;
    // Edit a copy: a refused split must leave no undo entry behind.
    let mut edited = app.surface().ok_or("no model loaded")?.clone();
    let report = seacraft_core::topology::split_face(&mut edited, face)
        .map_err(|e| format!("model.split_face: {e}"))?;
    app.snapshot_undo();
    if let Some(surface) = app.surface_mut() {
        *surface = edited;
    }
    app.mark_geometry_changed();
    app.status_note(format!(
        "face {face} split into {} triangles",
        report.pieces
    ));
    Ok(SplitFaceResult {
        face: report.face,
        point: report.point,
        pieces: report.pieces,
        faces: report.faces_after,
        points: report.points_after,
        undo_len: app.undo_len(),
    })
}

// ---- model.insert_edge ---------------------------------------------------

/// Split `face` along a new edge between two of its control points.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InsertEdgeParams {
    pub face: usize,
    pub a: usize,
    pub b: usize,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct InsertEdgeResult {
    pub face: usize,
    pub a: usize,
    pub b: usize,
    /// Faces in the model after the edit.
    pub faces: usize,
    pub undo_len: usize,
}

/// Insert one edge. A refused insertion leaves no undo entry.
pub fn insert_edge(app: &mut App, params: &InsertEdgeParams) -> Result<InsertEdgeResult, String> {
    let InsertEdgeParams { face, a, b } = *params;
    let mut edited = app.surface().ok_or("no model loaded")?.clone();
    seacraft_core::topology::insert_edge(&mut edited, face, a, b)
        .map_err(|e| format!("model.insert_edge: {e}"))?;
    app.snapshot_undo();
    if let Some(surface) = app.surface_mut() {
        *surface = edited;
    }
    app.mark_geometry_changed();
    app.status_note(format!("edge {a}-{b} inserted on face {face}"));
    Ok(InsertEdgeResult {
        face,
        a,
        b,
        faces: app.surface().map(|s| s.faces.len()).unwrap_or(0),
        undo_len: app.undo_len(),
    })
}

// ---- model.transform -----------------------------------------------------

/// The transform request is the core's own type: target LWL/BWL/draft, extra
/// scale, translation and mirror.
pub type TransformParams = TransformRequest;

#[derive(Debug, Clone, Serialize)]
pub struct TransformResult {
    pub report: TransformReport,
    pub faces: usize,
    pub points: usize,
    /// The draft the app measures at after the edit.
    pub draft: f64,
    pub undo_len: usize,
}

/// Fit, scale, move and/or mirror the hull: one undo step, and the app's
/// measuring draft follows the model's rescaled design draft.
pub fn transform(app: &mut App, request: &TransformParams) -> Result<TransformResult, String> {
    let subdiv = subdivision_at(app, app.subdivision_level());
    let surface = app
        .surface()
        .ok_or_else(|| "model.transform: no model is loaded".to_string())?;
    let (out, report) = seacraft_core::transform::run(surface, request, subdiv)
        .map_err(|e| format!("model.transform: {e}"))?;
    app.snapshot_undo();
    if let Some(surface) = app.surface_mut() {
        *surface = out;
    }
    app.select_point(None);
    // The fit rescales the model's own design draft; the app measures at
    // its hydro option, so carry the new draft across or every number
    // after this would be read at the old waterline.
    let new_draft = app.surface().map(|s| s.settings.draft).unwrap_or(0.0);
    if new_draft > 0.0 {
        app.hydro_options_mut().draft = new_draft;
    }
    app.mark_geometry_changed();
    app.status_note(format!(
        "hull transformed: scale {:.3} x {:.3} x {:.3}",
        report.scale.x, report.scale.y, report.scale.z
    ));
    Ok(TransformResult {
        report,
        faces: app.face_count(),
        points: app.control_point_count(),
        draft: app.hydro_options().draft,
        undo_len: app.undo_len(),
    })
}

// ---- model.optimize ------------------------------------------------------

/// Reshape the hull for lower resistance at a speed.
#[derive(Debug, Clone)]
pub struct OptimizeParams {
    pub spec: OptimizeSpec,
    /// Subdivision level to measure at; `None` is the app's level.
    pub level: Option<u8>,
}

#[derive(Debug, Clone, Serialize)]
pub struct OptimizeResult {
    pub level: u8,
    pub kg: Option<f64>,
    pub report: OptimizeReport,
    pub faces: usize,
    pub points: usize,
    /// The draft the app measures at after the edit (the one that holds the
    /// original displacement).
    pub draft: f64,
    pub undo_len: usize,
}

/// Run the optimiser and replace the hull with its result: one undo step.
/// Synchronous; a long run blocks the caller.
pub fn optimize(app: &mut App, params: &OptimizeParams) -> Result<OptimizeResult, String> {
    let level = level_or_app(app, params.level);
    let mut opts = app.hydro_options().clone();
    opts.subdivision = subdivision_at(app, level);
    let surface = app
        .surface()
        .ok_or_else(|| "model.optimize: no model is loaded".to_string())?;
    let (out, report) = seacraft_core::optimize::optimize_hull(surface, &opts, &params.spec)
        .map_err(|e| format!("model.optimize: {e}"))?;
    app.snapshot_undo();
    if let Some(surface) = app.surface_mut() {
        *surface = out;
    }
    app.select_point(None);
    // The optimised hull is measured at the draft that holds the
    // original displacement; the app must read it there too.
    app.hydro_options_mut().draft = report.optimised.draft_m;
    app.mark_geometry_changed();
    app.status_note(format!(
        "hull optimised at {:.1} kn: RT {:+.1} %",
        report.speed_kn, report.rt_change_pct
    ));
    Ok(OptimizeResult {
        level,
        kg: params.spec.kg,
        report,
        faces: app.face_count(),
        points: app.control_point_count(),
        draft: app.hydro_options().draft,
        undo_len: app.undo_len(),
    })
}

// ---- hydro.table ---------------------------------------------------------

/// A hydrostatic table over a range of drafts.
#[derive(Debug, Clone, PartialEq)]
pub struct HydroTableParams {
    /// Top of the default draft range; `None` is the app's draft.
    pub to: Option<f64>,
    /// Subdivision level; `None` is the app's level.
    pub level: Option<u8>,
    /// Explicit drafts; when `None`, `steps` equal steps up to `to`.
    pub drafts: Option<Vec<f64>>,
    pub steps: usize,
}

impl Default for HydroTableParams {
    fn default() -> Self {
        Self {
            to: None,
            level: None,
            drafts: None,
            steps: 10,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct HydroTableResult {
    pub level: u8,
    pub name: String,
    pub water_density: f64,
    /// Number of rows.
    pub drafts: usize,
    pub rows: Vec<HydrostaticRow>,
}

/// Build the hydrostatic table. Read-only.
pub fn hydro_table(app: &App, params: &HydroTableParams) -> Result<HydroTableResult, String> {
    let mut opts = app.hydro_options().clone();
    if let Some(to) = params.to {
        opts.draft = to;
    }
    let level = level_or_app(app, params.level);
    opts.subdivision = subdivision_at(app, level);
    let drafts = match &params.drafts {
        Some(list) => {
            if list.is_empty() {
                return Err("hydro.table: `drafts` is empty".into());
            }
            if list.iter().any(|d| *d <= 0.0) {
                return Err("hydro.table: `drafts` must be positive".into());
            }
            list.clone()
        }
        None => seacraft_core::table::draft_range(opts.draft, params.steps),
    };
    let surface = app
        .surface()
        .ok_or_else(|| "hydro.table: no model is loaded".to_string())?;
    let t = seacraft_core::table::table(surface, &opts, &drafts);
    Ok(HydroTableResult {
        level,
        name: t.name,
        water_density: t.water_density,
        drafts: t.rows.len(),
        rows: t.rows,
    })
}

// ---- hydro.trim ----------------------------------------------------------

/// A resistance scan over trim at one speed.
#[derive(Debug, Clone, PartialEq)]
pub struct TrimScanParams {
    /// Draft; `None` is the app's draft.
    pub draft: Option<f64>,
    /// Subdivision level; `None` is the app's level.
    pub level: Option<u8>,
    pub speed_kn: f64,
    pub from: f64,
    pub to: f64,
    pub steps: usize,
    /// Hold the displacement (re-solve the draft at each trim).
    pub hold_volume: bool,
}

impl Default for TrimScanParams {
    fn default() -> Self {
        Self {
            draft: None,
            level: None,
            speed_kn: 0.0,
            from: -0.6,
            to: 0.6,
            steps: 13,
            hold_volume: true,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct TrimScanResult {
    pub level: u8,
    pub speed_kn: f64,
    pub target_volume_m3: f64,
    pub at_boundary: bool,
    pub best: Option<TrimPoint>,
    pub warnings: Vec<String>,
    pub points: Vec<TrimPoint>,
}

/// Scan resistance over trim. Read-only.
pub fn trim_scan(app: &App, params: &TrimScanParams) -> Result<TrimScanResult, String> {
    let mut opts = app.hydro_options().clone();
    if let Some(d) = params.draft {
        opts.draft = d;
    }
    let level = level_or_app(app, params.level);
    let subdiv = subdivision_at(app, level);
    if params.to <= params.from {
        return Err("hydro.trim: `to` must be greater than `from`".into());
    }
    let surface = app
        .surface()
        .ok_or_else(|| "hydro.trim: no model is loaded".to_string())?;
    let trims = seacraft_core::trimopt::trim_range(params.from, params.to, params.steps);
    let TrimScan {
        speed_kn,
        target_volume_m3,
        points,
        best,
        at_boundary,
        warnings,
    } = seacraft_core::trimopt::scan(
        surface,
        subdiv,
        &opts,
        params.speed_kn,
        &trims,
        params.hold_volume,
    )
    .map_err(|e| format!("hydro.trim: {e}"))?;
    Ok(TrimScanResult {
        level,
        speed_kn,
        target_volume_m3,
        at_boundary,
        best,
        warnings,
        points,
    })
}

// ---- curves.bonjean ------------------------------------------------------

/// Bonjean curves: section area against draft at each station.
#[derive(Debug, Clone, PartialEq)]
pub struct BonjeanParams {
    /// Subdivision level; `None` is the app's level.
    pub level: Option<u8>,
    pub stations: usize,
    /// Number of default drafts, keel to the top of the hull.
    pub draft_steps: usize,
    /// Explicit drafts (at least two, positive; sorted here).
    pub drafts: Option<Vec<f64>>,
}

impl Default for BonjeanParams {
    fn default() -> Self {
        Self {
            level: None,
            stations: 21,
            draft_steps: 10,
            drafts: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct BonjeanResult {
    pub level: u8,
    pub depth_m: f64,
    pub design_draft_m: f64,
    pub drafts_m: Vec<f64>,
    pub sections: Vec<BonjeanSection>,
    pub area_max_m2: f64,
}

/// Measure the Bonjean curves of the app's model. Read-only.
pub fn bonjean(app: &App, params: &BonjeanParams) -> Result<BonjeanResult, String> {
    let level = level_or_app(app, params.level);
    let surface = app
        .surface()
        .ok_or_else(|| "curves.bonjean: no model is loaded".to_string())?;
    let hull = seacraft_core::hydro::Hull::new(surface, subdivision_at(app, level));
    let depth = hull.model_max.z - hull.keel;
    if depth <= 0.0 {
        return Err("curves.bonjean: the hull has no height above the keel".into());
    }
    let drafts = match &params.drafts {
        Some(given) => {
            if given.len() < 2 {
                return Err("curves.bonjean: `drafts` needs at least two ordinates".into());
            }
            if given.iter().any(|d| *d <= 0.0) {
                return Err("curves.bonjean: `drafts` must be positive".into());
            }
            let mut given = given.clone();
            given.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
            given
        }
        None => seacraft_core::curves::draft_range(
            depth / params.draft_steps as f64,
            depth,
            params.draft_steps,
        ),
    };
    let bj: Bonjean = seacraft_core::curves::bonjean(&hull, params.stations, &drafts);
    let area_max_m2 = bj.max_area();
    Ok(BonjeanResult {
        level,
        depth_m: depth,
        design_draft_m: app.hydro_options().draft,
        drafts_m: bj.drafts_m,
        sections: bj.sections,
        area_max_m2,
    })
}

// ---- curves.offsets ------------------------------------------------------

/// A table of offsets: half-breadths and heights at stations.
#[derive(Debug, Clone, PartialEq)]
pub struct OffsetsParams {
    /// Subdivision level; `None` is the app's level.
    pub level: Option<u8>,
    pub stations: usize,
    /// Default waterline count when `waterlines` is `None`.
    pub waterline_steps: usize,
    /// Default buttock count when `buttocks` is `None`.
    pub buttock_steps: usize,
    /// Explicit waterline heights above the keel (positive; sorted here).
    pub waterlines: Option<Vec<f64>>,
    /// Explicit buttock offsets (positive; sorted here).
    pub buttocks: Option<Vec<f64>>,
}

impl Default for OffsetsParams {
    fn default() -> Self {
        Self {
            level: None,
            stations: 21,
            waterline_steps: 6,
            buttock_steps: 4,
            waterlines: None,
            buttocks: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct OffsetsResult {
    pub level: u8,
    pub keel_z: f64,
    pub waterlines_m: Vec<f64>,
    pub buttocks_m: Vec<f64>,
    pub stations: Vec<OffsetStation>,
    pub max_half_breadth_m: f64,
}

/// A non-empty list of positive ordinates, sorted ascending.
fn positive_ordinates(given: &[f64], key: &str) -> Result<Vec<f64>, String> {
    if given.is_empty() {
        return Err(format!(
            "curves.offsets: `{key}` needs at least one ordinate"
        ));
    }
    if given.iter().any(|x| *x <= 0.0) {
        return Err(format!("curves.offsets: `{key}` must be positive"));
    }
    let mut v = given.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    Ok(v)
}

/// Build the table of offsets of the app's model. Read-only.
pub fn offsets(app: &App, params: &OffsetsParams) -> Result<OffsetsResult, String> {
    let level = level_or_app(app, params.level);
    let surface = app
        .surface()
        .ok_or_else(|| "curves.offsets: no model is loaded".to_string())?;
    let hull = seacraft_core::hydro::Hull::new(surface, subdivision_at(app, level));
    let waterlines = match &params.waterlines {
        Some(v) => positive_ordinates(v, "waterlines")?,
        None => seacraft_core::offsets::default_waterlines(&hull, params.waterline_steps),
    };
    let buttocks = match &params.buttocks {
        Some(v) => positive_ordinates(v, "buttocks")?,
        None => seacraft_core::offsets::default_buttocks(&hull, params.buttock_steps),
    };
    let table: TableOfOffsets =
        seacraft_core::offsets::table_of_offsets(&hull, params.stations, &waterlines, &buttocks);
    let max_half_breadth_m = table.max_half_breadth();
    Ok(OffsetsResult {
        level,
        keel_z: table.keel_z,
        waterlines_m: table.waterlines_m,
        buttocks_m: table.buttocks_m,
        stations: table.stations,
        max_half_breadth_m,
    })
}

// ---- curves.unfold -------------------------------------------------------

/// Plate development of a band of the hull.
#[derive(Debug, Clone, PartialEq)]
pub struct UnfoldParams {
    /// The axis the band is cut across.
    pub axis: BandAxis,
    /// Subdivision level; `None` is the app's level.
    pub level: Option<u8>,
    /// Band limits along `axis`; `None` is the hull's extent.
    pub from: Option<f64>,
    pub to: Option<f64>,
    /// Limits along `x`; `None` is the hull's extent.
    pub x_from: Option<f64>,
    pub x_to: Option<f64>,
    /// Both sides of a symmetric hull, not starboard only.
    pub both_sides: bool,
    /// Where to write the plate as SVG. The caller has already checked the
    /// path (the control channel against its write roots).
    pub path: Option<PathBuf>,
}

impl Default for UnfoldParams {
    fn default() -> Self {
        Self {
            axis: BandAxis::Z,
            level: None,
            from: None,
            to: None,
            x_from: None,
            x_to: None,
            both_sides: false,
            path: None,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct UnfoldResult {
    pub level: u8,
    /// Number of refined triangles in the plate.
    pub faces: usize,
    pub plate: Unfolded,
    /// The SVG written, if a path was given.
    pub written: Option<String>,
}

/// Develop a band of the hull flat. Read-only on the model; writes the SVG
/// when `path` is given.
pub fn unfold(app: &App, params: &UnfoldParams) -> Result<UnfoldResult, String> {
    let axis = params.axis;
    let level = level_or_app(app, params.level);
    let surface = app
        .surface()
        .ok_or_else(|| "curves.unfold: no model is loaded".to_string())?;
    let hull = seacraft_core::hydro::Hull::new(surface, subdivision_at(app, level));
    let pick = |axis: BandAxis, given: Option<f64>, low: bool| {
        given.unwrap_or(match (axis, low) {
            (BandAxis::X, true) => hull.model_min.x,
            (BandAxis::X, false) => hull.model_max.x,
            (BandAxis::Y, true) => hull.model_min.y,
            (BandAxis::Y, false) => hull.model_max.y,
            (BandAxis::Z, true) => hull.keel,
            (BandAxis::Z, false) => hull.model_max.z,
        })
    };
    let selection = PatchSelection {
        slabs: vec![
            (
                axis,
                pick(axis, params.from, true),
                pick(axis, params.to, false),
            ),
            (
                BandAxis::X,
                pick(BandAxis::X, params.x_from, true),
                pick(BandAxis::X, params.x_to, false),
            ),
        ],
        starboard_only: !params.both_sides,
    };
    let faces = seacraft_core::develop::select(&hull.verts, &hull.tris, &selection);
    if faces.is_empty() {
        return Err("curves.unfold: the band selected no faces; widen the range".into());
    }
    let plate = seacraft_core::develop::unfold(&hull.verts, &faces);
    let mut written = None;
    if let Some(path) = &params.path {
        let title = format!(
            "{} - plate on {:?} {:.2}..{:.2} m",
            surface.settings.name, axis, selection.slabs[0].1, selection.slabs[0].2
        );
        seacraft_core::io::write_atomic(path, seacraft_core::develop::svg(&plate, &title))
            .map_err(|e| format!("curves.unfold: cannot write {}: {e}", path.display()))?;
        written = Some(path.to_string_lossy().to_string());
    }
    Ok(UnfoldResult {
        level,
        faces: faces.len(),
        plate,
        written,
    })
}

// ---- layers --------------------------------------------------------------
//
// FREE!ship's layer dialog: add, delete, rename, reorder, the per-layer flags
// and plating properties, the active layer, and "Move to layer" for faces.
// Layers are addressed by index (their place in the list, which is what a
// face's layer refers to); ids are the `.fef` identity and stay unique.

/// One layer as the control channel and the tests see it. `density_kg_m3`
/// and `thickness_m` are the core's units (the panel shows t/m³ and mm).
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayerInfo {
    pub index: usize,
    pub id: i32,
    pub name: String,
    /// `0xRRGGBB`.
    pub color: i32,
    pub visible: bool,
    pub developable: bool,
    pub symmetric: bool,
    pub use_for_intersections: bool,
    pub use_in_hydrostatics: bool,
    pub show_in_linesplan: bool,
    pub density_kg_m3: f64,
    pub thickness_m: f64,
    /// Control faces on the layer.
    pub faces: usize,
}

/// Every layer, the active one and the face total.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct LayerList {
    pub layers: Vec<LayerInfo>,
    pub active: usize,
    pub faces: usize,
}

/// The layers of the app's model.
pub fn list_layers(app: &App) -> Result<LayerList, String> {
    let surface = app.surface().ok_or("no model loaded")?;
    let counts = surface.layer_face_counts();
    Ok(LayerList {
        layers: surface
            .layers
            .iter()
            .enumerate()
            .map(|(index, l)| LayerInfo {
                index,
                id: l.id,
                name: l.name.clone(),
                color: l.color,
                visible: l.visible,
                developable: l.developable,
                symmetric: l.symmetric,
                use_for_intersections: l.use_for_intersections,
                use_in_hydrostatics: l.use_in_hydrostatics,
                show_in_linesplan: l.show_in_linesplan,
                density_kg_m3: l.density,
                thickness_m: l.thickness,
                faces: counts[index],
            })
            .collect(),
        active: app.active_layer(),
        faces: surface.faces.len(),
    })
}

/// The answer of every layer edit.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct LayerEditResult {
    /// The layer the edit was about, at its index after the edit (the
    /// receiving layer for a delete).
    pub layer: usize,
    pub layers: usize,
    pub active: usize,
    /// Faces whose layer changed.
    pub moved_faces: usize,
    /// `false` when the request asked for what the model already had: no
    /// undo entry, nothing marked changed.
    pub changed: bool,
    pub undo_len: usize,
}

/// Run `edit` on a copy of the model and keep it as one undo step when it
/// succeeds and changed the layers or the face assignment. A refused or
/// empty edit leaves the model and the undo stack as they were.
fn edit_layers<T>(
    app: &mut App,
    what: &str,
    edit: impl FnOnce(&mut seacraft_core::Surface) -> Result<T, String>,
) -> Result<(T, bool), String> {
    let current = app.surface().ok_or("no model loaded")?;
    let mut edited = current.clone();
    let out = edit(&mut edited).map_err(|e| format!("{what}: {e}"))?;
    let changed = edited.layers != current.layers || edited.face_layer != current.face_layer;
    if changed {
        app.snapshot_undo();
        if let Some(surface) = app.surface_mut() {
            *surface = edited;
        }
        app.mark_geometry_changed();
    }
    Ok((out, changed))
}

fn layer_index(surface: &seacraft_core::Surface, layer: usize) -> Result<usize, String> {
    if layer < surface.layers.len() {
        Ok(layer)
    } else {
        Err(format!(
            "`layer` must be from 0 to {}",
            surface.layers.len().saturating_sub(1)
        ))
    }
}

/// A layer name as it will be stored: one line, trimmed, not empty and not
/// another layer's name.
fn clean_layer_name(
    surface: &seacraft_core::Surface,
    layer: Option<usize>,
    name: &str,
) -> Result<String, String> {
    let name = name.replace(['\r', '\n', '\t'], " ").trim().to_owned();
    if name.is_empty() {
        return Err("a layer name cannot be empty".into());
    }
    if name.chars().count() > 64 {
        return Err("a layer name has at most 64 characters".into());
    }
    if surface
        .layers
        .iter()
        .enumerate()
        .any(|(i, l)| Some(i) != layer && l.name == name)
    {
        return Err(format!("a layer named \"{name}\" already exists"));
    }
    Ok(name)
}

/// Add a layer.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AddLayerParams {
    /// The new layer's name; `None` gives the first free "Layer N".
    pub name: Option<String>,
}

/// Add an empty layer after the others and make it active: the first free
/// "Layer N" name, the next free id, the first palette colour no layer
/// uses, and the active layer's symmetry (a symmetric layer in a full model
/// would mirror its faces onto the other half). One undo step.
pub fn add_layer(app: &mut App, params: &AddLayerParams) -> Result<LayerEditResult, String> {
    let template = app
        .surface()
        .and_then(|s| s.layers.get(app.active_layer()).cloned());
    let ((index, name), _) = edit_layers(app, "layer.add", |s| {
        let name = match &params.name {
            Some(n) => clean_layer_name(s, None, n)?,
            None => (s.layers.len() + 1..)
                .map(|n| format!("Layer {n}"))
                .find(|n| !s.layers.iter().any(|l| &l.name == n))
                .expect("an unused name"),
        };
        let id = s.layers.iter().map(|l| l.id).max().unwrap_or(0).max(0) + 1;
        let used: Vec<i32> = s.layers.iter().map(|l| l.color).collect();
        let base = template.unwrap_or_default();
        s.layers.push(seacraft_core::mesh::Layer {
            name: name.clone(),
            id,
            color: crate::theme::next_layer_colour(&used),
            visible: true,
            symmetric: base.symmetric,
            ..Default::default()
        });
        Ok((s.layers.len() - 1, name))
    })?;
    app.set_active_layer_index(index);
    app.status_note(format!("layer \"{name}\" added"));
    Ok(layer_result(app, index, 0, true))
}

fn layer_result(app: &App, layer: usize, moved_faces: usize, changed: bool) -> LayerEditResult {
    LayerEditResult {
        layer,
        layers: app.layer_count(),
        active: app.active_layer(),
        moved_faces,
        changed,
        undo_len: app.undo_len(),
    }
}

/// Delete a layer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeleteLayerParams {
    pub layer: usize,
    /// Where the layer's faces go. A layer that still owns faces is not
    /// deleted without one.
    pub move_faces_to: Option<usize>,
}

/// Delete a layer, moving its faces to `move_faces_to` first. The last
/// layer is never deleted, and a layer with faces needs somewhere to put
/// them. One undo step.
pub fn delete_layer(app: &mut App, params: &DeleteLayerParams) -> Result<LayerEditResult, String> {
    let active = app.active_layer();
    let ((target, moved, name), _) = edit_layers(app, "layer.delete", |s| {
        let k = layer_index(s, params.layer)?;
        if s.layers.len() == 1 {
            return Err("the last layer cannot be deleted".into());
        }
        let owned = s.layer_face_counts()[k];
        let target = match params.move_faces_to {
            Some(t) if t == k => return Err("`move_faces_to` is the layer being deleted".into()),
            Some(t) => Some(layer_index(s, t).map_err(|_| {
                format!(
                    "`move_faces_to` must be from 0 to {}",
                    s.layers.len().saturating_sub(1)
                )
            })?),
            None if owned > 0 => {
                return Err(format!(
                    "\"{}\" still owns {owned} face{}; give `move_faces_to`",
                    s.layers[k].name,
                    if owned == 1 { "" } else { "s" }
                ))
            }
            None => None,
        };
        if let Some(t) = target {
            for l in s.face_layer.iter_mut().filter(|l| **l == k) {
                *l = t;
            }
        }
        let name = s.layers.remove(k).name;
        for l in s.face_layer.iter_mut().filter(|l| **l > k) {
            *l -= 1;
        }
        let shift = |i: usize| if i > k { i - 1 } else { i };
        Ok((target.map(shift), owned, name))
    })?;
    let k = params.layer;
    // The active layer follows its own entry; deleting it activates the
    // layer that received its faces (or the one now in its place).
    let next_active = if active == k {
        target.unwrap_or(k.min(app.layer_count().saturating_sub(1)))
    } else if active > k {
        active - 1
    } else {
        active
    };
    app.set_active_layer_index(next_active);
    app.status_note(match target {
        Some(_) if moved > 0 => format!("layer \"{name}\" deleted, {moved} faces moved"),
        _ => format!("layer \"{name}\" deleted"),
    });
    Ok(layer_result(
        app,
        target.unwrap_or(next_active),
        moved,
        true,
    ))
}

/// Up or down the layer list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerDirection {
    Up,
    Down,
}

/// Every property of a layer; `None` leaves it as it is.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerProps {
    pub name: Option<String>,
    pub id: Option<i32>,
    /// `0xRRGGBB`.
    pub color: Option<i32>,
    pub visible: Option<bool>,
    pub developable: Option<bool>,
    pub symmetric: Option<bool>,
    pub use_for_intersections: Option<bool>,
    pub use_in_hydrostatics: Option<bool>,
    pub show_in_linesplan: Option<bool>,
    /// kg/m³, from 0 to 25 000.
    pub density: Option<f64>,
    /// Metres, from 0 to 1.
    pub thickness: Option<f64>,
}

/// Change a layer's properties and/or its place in the list.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SetLayerParams {
    pub layer: usize,
    pub props: LayerProps,
    /// Move the layer one place up or down the list.
    pub reorder: Option<LayerDirection>,
}

fn apply_layer_props(
    s: &mut seacraft_core::Surface,
    k: usize,
    p: &LayerProps,
) -> Result<(), String> {
    let name = match &p.name {
        Some(n) => Some(clean_layer_name(s, Some(k), n)?),
        None => None,
    };
    if let Some(id) = p.id {
        if id < 1 {
            return Err("`id` must be a whole number from 1".into());
        }
        if s.layers
            .iter()
            .enumerate()
            .any(|(i, l)| i != k && l.id == id)
        {
            return Err(format!("another layer already has id {id}"));
        }
    }
    if p.color.is_some_and(|c| !(0..=0xff_ff_ff).contains(&c)) {
        return Err("`color` must be 0xRRGGBB, from 0 to 16777215".into());
    }
    if p.density
        .is_some_and(|d| !d.is_finite() || !(0.0..=25_000.0).contains(&d))
    {
        return Err("`density` must be from 0 to 25000 kg/m³".into());
    }
    if p.thickness
        .is_some_and(|t| !t.is_finite() || !(0.0..=1.0).contains(&t))
    {
        return Err("`thickness` must be from 0 to 1 m".into());
    }
    let l = &mut s.layers[k];
    if let Some(name) = name {
        l.name = name;
    }
    if let Some(v) = p.id {
        l.id = v;
    }
    if let Some(v) = p.color {
        l.color = v;
    }
    if let Some(v) = p.visible {
        l.visible = v;
    }
    if let Some(v) = p.developable {
        l.developable = v;
    }
    if let Some(v) = p.symmetric {
        l.symmetric = v;
    }
    if let Some(v) = p.use_for_intersections {
        l.use_for_intersections = v;
    }
    if let Some(v) = p.use_in_hydrostatics {
        l.use_in_hydrostatics = v;
    }
    if let Some(v) = p.show_in_linesplan {
        l.show_in_linesplan = v;
    }
    if let Some(v) = p.density {
        l.density = v;
    }
    if let Some(v) = p.thickness {
        l.thickness = v;
    }
    Ok(())
}

/// Swap layer `k` with its neighbour, carrying the faces of both along.
/// Returns the layer's new index.
fn reorder_layer(
    s: &mut seacraft_core::Surface,
    k: usize,
    dir: LayerDirection,
) -> Result<usize, String> {
    let j = match dir {
        LayerDirection::Up if k > 0 => k - 1,
        LayerDirection::Down if k + 1 < s.layers.len() => k + 1,
        LayerDirection::Up => return Err("the layer is already first".into()),
        LayerDirection::Down => return Err("the layer is already last".into()),
    };
    s.layers.swap(k, j);
    for l in &mut s.face_layer {
        if *l == k {
            *l = j;
        } else if *l == j {
            *l = k;
        }
    }
    Ok(j)
}

/// Set any of a layer's properties and/or move it up or down the list: one
/// undo step for the whole request, none when nothing changes.
pub fn set_layer(app: &mut App, params: &SetLayerParams) -> Result<LayerEditResult, String> {
    let active = app.active_layer();
    let (index, changed) = edit_layers(app, "layer.set", |s| {
        let k = layer_index(s, params.layer)?;
        apply_layer_props(s, k, &params.props)?;
        match params.reorder {
            Some(dir) => reorder_layer(s, k, dir),
            None => Ok(k),
        }
    })?;
    // The active layer stays the same layer, wherever it now sits.
    let k = params.layer;
    if index != k {
        if active == k {
            app.set_active_layer_index(index);
        } else if active == index {
            app.set_active_layer_index(k);
        }
    }
    if changed {
        let name = app
            .surface()
            .and_then(|s| s.layers.get(index))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        app.status_note(format!("layer \"{name}\" changed"));
    }
    Ok(layer_result(app, index, 0, changed))
}

/// Rename a layer (a [`set_layer`] with only the name).
pub fn rename_layer(app: &mut App, layer: usize, name: &str) -> Result<LayerEditResult, String> {
    set_layer(
        app,
        &SetLayerParams {
            layer,
            props: LayerProps {
                name: Some(name.to_owned()),
                ..Default::default()
            },
            reorder: None,
        },
    )
}

/// Move a layer one place up or down the list (a [`set_layer`] with only
/// the reorder).
pub fn move_layer(
    app: &mut App,
    layer: usize,
    dir: LayerDirection,
) -> Result<LayerEditResult, String> {
    set_layer(
        app,
        &SetLayerParams {
            layer,
            reorder: Some(dir),
            ..Default::default()
        },
    )
}

/// Make `layer` the active layer: the one the panel edits and new faces go
/// to. A view setting, not a model edit: no undo step.
pub fn set_active_layer(app: &mut App, layer: usize) -> Result<LayerEditResult, String> {
    layer_index(app.surface().ok_or("no model loaded")?, layer)?;
    app.set_active_layer_index(layer);
    Ok(layer_result(app, layer, 0, false))
}

/// Move control faces to a layer ("Move to layer").
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AssignFacesParams {
    pub faces: Vec<usize>,
    pub layer: usize,
}

/// Put `faces` on `layer`: one undo step, none when they are all there
/// already. Moving faces between layers with the same symmetry and
/// hydrostatics flags changes no number.
pub fn assign_faces(app: &mut App, params: &AssignFacesParams) -> Result<LayerEditResult, String> {
    let (moved, changed) = edit_layers(app, "layer.assign_faces", |s| {
        let k = layer_index(s, params.layer)?;
        if params.faces.is_empty() {
            return Err("give at least one face".into());
        }
        if let Some(&bad) = params.faces.iter().find(|&&f| f >= s.faces.len()) {
            return Err(format!(
                "face {bad} does not exist (the model has {} faces)",
                s.faces.len()
            ));
        }
        let mut moved = 0;
        for &f in &params.faces {
            if s.face_layer[f] != k {
                s.face_layer[f] = k;
                moved += 1;
            }
        }
        Ok(moved)
    })?;
    if changed {
        let name = app
            .surface()
            .and_then(|s| s.layers.get(params.layer))
            .map(|l| l.name.clone())
            .unwrap_or_default();
        app.status_note(format!(
            "{moved} face{} moved to layer \"{name}\"",
            if moved == 1 { "" } else { "s" }
        ));
    }
    Ok(layer_result(app, params.layer, moved, changed))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The demo model at a cheap level: these tests drive the commands, not
    /// the refinement.
    fn demo() -> App {
        let mut app = App::headless_for_test(None);
        app.set_subdivision_level(2);
        app
    }

    #[test]
    fn transform_scales_lwl_in_one_undo_step() {
        let mut app = demo();
        let undo = app.undo_len();
        let r = transform(
            &mut app,
            &TransformParams {
                lwl: Some(13.0),
                ..Default::default()
            },
        )
        .expect("transform");
        let after = r.report.after.as_ref().expect("dimensions after");
        assert!((after.lwl - 13.0).abs() < 0.05, "{after:?}");
        assert_eq!(app.undo_len(), undo + 1);
        assert_eq!(r.undo_len, app.undo_len());
        assert!(app.is_dirty());
        // An empty request is refused and leaves nothing behind.
        assert!(transform(&mut app, &TransformParams::default()).is_err());
        assert_eq!(app.undo_len(), undo + 1);
    }

    #[test]
    fn crease_toggles_an_edge_in_one_undo_step() {
        let mut app = demo();
        let key = app.surface().unwrap().edges[0].key();
        let was = app.surface().unwrap().edges[0].crease;
        let undo = app.undo_len();
        let r = set_crease(
            &mut app,
            &CreaseParams {
                edge: [key.1, key.0],
                crease: !was,
            },
        )
        .expect("crease");
        assert_eq!(r.edge, [key.0, key.1], "the edge comes back ordered");
        assert_eq!(app.surface().unwrap().edges[0].crease, !was);
        assert_eq!(app.undo_len(), undo + 1);
        // A missing edge and an out-of-range point are refused, no snapshot.
        let n = app.control_point_count();
        assert!(set_crease(
            &mut app,
            &CreaseParams {
                edge: [0, n],
                crease: true
            }
        )
        .is_err());
        assert_eq!(app.undo_len(), undo + 1);
    }

    #[test]
    fn refused_split_leaves_no_undo_entry() {
        let mut app = demo();
        let faces = app.face_count();
        let undo = app.undo_len();
        let err = split_face(&mut app, &SplitFaceParams { face: faces }).unwrap_err();
        assert!(err.starts_with("model.split_face: "), "{err}");
        assert_eq!(app.undo_len(), undo);
        assert_eq!(app.face_count(), faces);
        let r = split_face(&mut app, &SplitFaceParams { face: 0 }).expect("split");
        assert_eq!(r.faces, faces + r.pieces - 1);
        assert_eq!(app.undo_len(), undo + 1);
        // An edge across a face from a point to itself is refused too.
        let undo = app.undo_len();
        assert!(insert_edge(
            &mut app,
            &InsertEdgeParams {
                face: 0,
                a: 0,
                b: 0
            }
        )
        .is_err());
        assert_eq!(app.undo_len(), undo);
    }

    #[test]
    fn hydro_table_has_one_row_per_draft() {
        let app = demo();
        let undo = app.undo_len();
        let t = hydro_table(
            &app,
            &HydroTableParams {
                steps: 4,
                ..Default::default()
            },
        )
        .expect("table");
        assert_eq!(t.drafts, 4);
        assert_eq!(t.rows.len(), 4);
        let top = t.rows.last().unwrap();
        assert!((top.draft - app.hydro_options().draft).abs() < 1e-9);
        let t = hydro_table(
            &app,
            &HydroTableParams {
                drafts: Some(vec![0.3, 0.5, 0.62]),
                ..Default::default()
            },
        )
        .expect("table");
        assert_eq!(t.rows.len(), 3);
        assert!(hydro_table(
            &app,
            &HydroTableParams {
                drafts: Some(vec![]),
                ..Default::default()
            }
        )
        .is_err());
        // Read-only by signature (`&App`): no snapshot, no dirty flag.
        assert_eq!(app.undo_len(), undo, "a table is not an edit");
    }

    #[test]
    fn offsets_and_bonjean_are_read_only_reports() {
        let app = demo();
        let o = offsets(
            &app,
            &OffsetsParams {
                stations: 5,
                waterlines: Some(vec![0.62, 0.3]),
                ..Default::default()
            },
        )
        .expect("offsets");
        assert_eq!(o.stations.len(), 5);
        assert_eq!(o.waterlines_m, vec![0.3, 0.62], "sorted ascending");
        let b = bonjean(
            &app,
            &BonjeanParams {
                stations: 5,
                draft_steps: 3,
                ..Default::default()
            },
        )
        .expect("bonjean");
        assert_eq!(b.drafts_m.len(), 3);
        assert!((b.drafts_m[2] - b.depth_m).abs() < 1e-9);
        assert!(b.area_max_m2 > 0.0);
    }

    #[test]
    fn generate_replaces_the_model() {
        let mut app = demo();
        let spec = HullSpec::default_for(seacraft_core::parametric::HullKind::Displacement);
        let r = generate(&mut app, &spec).expect("generate");
        assert_eq!(r.points, app.control_point_count());
        assert!(r.faces > 0);
    }

    // ---- layers ----------------------------------------------------------

    fn names(app: &App) -> Vec<String> {
        app.surface()
            .unwrap()
            .layers
            .iter()
            .map(|l| l.name.clone())
            .collect()
    }

    #[test]
    fn add_layer_is_one_undo_step_with_a_fresh_name_id_and_colour() {
        let mut app = demo();
        assert_eq!(names(&app), ["Bottom", "Topsides", "Transom"]);
        let undo = app.undo_len();
        let r = add_layer(&mut app, &AddLayerParams::default()).expect("add");
        assert_eq!(r.layers, 4);
        assert_eq!(r.layer, 3);
        assert_eq!(r.active, 3, "the new layer is active");
        assert_eq!(app.undo_len(), undo + 1);
        let s = app.surface().unwrap();
        let new = &s.layers[3];
        assert_eq!(new.name, "Layer 4");
        assert_eq!(new.id, 4);
        assert!(s.layers[..3].iter().all(|l| l.color != new.color));
        assert!(crate::theme::LAYER_PALETTE.contains(&new.color));
        assert_eq!(new.symmetric, s.layers[0].symmetric, "the model's symmetry");
        assert!(new.use_in_hydrostatics && new.visible);
        assert_eq!(s.layer_face_counts()[3], 0);
        // A second one gets the next name and colour; a taken name is refused.
        add_layer(&mut app, &AddLayerParams::default()).expect("add");
        assert_eq!(names(&app)[4], "Layer 5");
        let undo = app.undo_len();
        let err = add_layer(
            &mut app,
            &AddLayerParams {
                name: Some("Bottom".into()),
            },
        )
        .unwrap_err();
        assert!(err.contains("already exists"), "{err}");
        assert_eq!(app.undo_len(), undo);
        // Undo takes the layer away again.
        app.undo();
        assert_eq!(app.layer_count(), 4);
    }

    #[test]
    fn a_layer_with_faces_is_deleted_only_with_somewhere_to_put_them() {
        let mut app = demo();
        let faces = app.face_count();
        let undo = app.undo_len();
        let err = delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: 1,
                move_faces_to: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("still owns"), "{err}");
        assert_eq!(app.undo_len(), undo, "a refusal leaves no undo entry");
        assert_eq!(app.layer_count(), 3);
        // Moving them onto the layer itself is refused too.
        assert!(delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: 1,
                move_faces_to: Some(1),
            },
        )
        .is_err());
        let before = app.surface().unwrap().layer_face_counts();
        let r = delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: 1,
                move_faces_to: Some(2),
            },
        )
        .expect("delete with a move");
        assert_eq!(app.undo_len(), undo + 1);
        assert_eq!(names(&app), ["Bottom", "Transom"]);
        assert_eq!(r.layer, 1, "Transom, now at index 1, took the faces");
        assert_eq!(r.moved_faces, before[1]);
        let s = app.surface().unwrap();
        s.validate().expect("every face points at a layer");
        assert_eq!(s.faces.len(), faces);
        assert_eq!(
            s.layer_face_counts(),
            vec![before[0], before[1] + before[2]]
        );
        // An empty layer goes without a move; the last one never goes.
        add_layer(&mut app, &AddLayerParams::default()).unwrap();
        let empty = app.layer_count() - 1;
        delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: empty,
                move_faces_to: None,
            },
        )
        .expect("an empty layer");
        delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: 0,
                move_faces_to: Some(1),
            },
        )
        .expect("down to one layer");
        let undo = app.undo_len();
        let err = delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: 0,
                move_faces_to: None,
            },
        )
        .unwrap_err();
        assert!(err.contains("last layer"), "{err}");
        assert_eq!(app.undo_len(), undo);
        assert_eq!(app.surface().unwrap().layer_face_counts(), vec![faces]);
    }

    #[test]
    fn rename_and_reorder_keep_the_faces_with_their_layer() {
        let mut app = demo();
        let undo = app.undo_len();
        rename_layer(&mut app, 0, "  Underwater\nbody ").expect("rename");
        assert_eq!(names(&app)[0], "Underwater body", "one trimmed line");
        assert_eq!(app.undo_len(), undo + 1);
        assert!(rename_layer(&mut app, 0, "   ").is_err());
        assert!(rename_layer(&mut app, 0, "Transom").is_err(), "duplicate");
        assert_eq!(app.undo_len(), undo + 1);
        // Renaming to the same name changes nothing and records nothing.
        let r = rename_layer(&mut app, 0, "Underwater body").unwrap();
        assert!(!r.changed);
        assert_eq!(app.undo_len(), undo + 1);

        let transom_faces: Vec<usize> = (0..app.face_count())
            .filter(|&f| app.surface().unwrap().face_layer[f] == 2)
            .collect();
        set_active_layer(&mut app, 2).unwrap();
        let r = move_layer(&mut app, 2, LayerDirection::Up).expect("up");
        assert_eq!(r.layer, 1);
        assert_eq!(r.active, 1, "the active layer moves with it");
        assert_eq!(names(&app), ["Underwater body", "Transom", "Topsides"]);
        let s = app.surface().unwrap();
        assert!(transom_faces.iter().all(|&f| s.face_layer[f] == 1));
        assert_eq!(app.undo_len(), undo + 2);
        let undo = app.undo_len();
        assert!(move_layer(&mut app, 0, LayerDirection::Up).is_err());
        assert!(move_layer(&mut app, 2, LayerDirection::Down).is_err());
        assert_eq!(app.undo_len(), undo);
        move_layer(&mut app, 1, LayerDirection::Down).expect("down");
        assert_eq!(names(&app), ["Underwater body", "Topsides", "Transom"]);
        assert_eq!(app.active_layer(), 2);
    }

    #[test]
    fn layer_properties_reach_the_model_and_the_measurement() {
        let mut app = demo();
        app.recompute_hydrostatics();
        let full = app.hydrostatics().unwrap().volume_m3;
        let undo = app.undo_len();
        let r = set_layer(
            &mut app,
            &SetLayerParams {
                layer: 0,
                props: LayerProps {
                    id: Some(42),
                    color: Some(0x10_20_30),
                    visible: Some(false),
                    developable: Some(true),
                    use_for_intersections: Some(false),
                    use_in_hydrostatics: Some(false),
                    show_in_linesplan: Some(false),
                    density: Some(1500.0),
                    thickness: Some(0.008),
                    ..Default::default()
                },
                reorder: None,
            },
        )
        .expect("set");
        assert!(r.changed);
        assert_eq!(app.undo_len(), undo + 1, "the whole set is one step");
        let l = &app.surface().unwrap().layers[0];
        assert_eq!(
            (l.id, l.color, l.visible, l.developable),
            (42, 0x10_20_30, false, true)
        );
        assert!(!l.use_for_intersections && !l.use_in_hydrostatics && !l.show_in_linesplan);
        assert_eq!((l.density, l.thickness), (1500.0, 0.008));
        // Taking the bottom out of the hydrostatics drops the displacement.
        app.recompute_hydrostatics();
        assert!(app.hydrostatics().unwrap().volume_m3 < full - 1.0);
        // Bad values are refused whole: nothing of the request is applied.
        for bad in [
            LayerProps {
                id: Some(2),
                visible: Some(true),
                ..Default::default()
            },
            LayerProps {
                color: Some(0x1_00_00_00),
                ..Default::default()
            },
            LayerProps {
                density: Some(f64::NAN),
                ..Default::default()
            },
            LayerProps {
                thickness: Some(-0.001),
                ..Default::default()
            },
        ] {
            let undo = app.undo_len();
            assert!(set_layer(
                &mut app,
                &SetLayerParams {
                    layer: 0,
                    props: bad.clone(),
                    reorder: None
                }
            )
            .is_err());
            assert_eq!(app.undo_len(), undo, "{bad:?}");
            assert!(!app.surface().unwrap().layers[0].visible);
        }
        assert!(set_layer(
            &mut app,
            &SetLayerParams {
                layer: 9,
                ..Default::default()
            }
        )
        .is_err());
        app.undo();
        app.recompute_hydrostatics();
        assert!((app.hydrostatics().unwrap().volume_m3 - full).abs() < 1e-12);
    }

    #[test]
    fn assigning_faces_moves_them_and_leaves_the_numbers_alone() {
        let mut app = demo();
        app.recompute_hydrostatics();
        let before = app.hydrostatics().unwrap().clone();
        let undo = app.undo_len();
        add_layer(&mut app, &AddLayerParams::default()).unwrap();
        let r = assign_faces(
            &mut app,
            &AssignFacesParams {
                faces: vec![0, 1, 1, 7],
                layer: 3,
            },
        )
        .expect("assign");
        assert_eq!(r.moved_faces, 3, "a repeated face moves once");
        assert_eq!(app.undo_len(), undo + 2);
        let s = app.surface().unwrap();
        assert_eq!(s.face_layer[0], 3);
        assert_eq!(s.face_layer[7], 3);
        assert_eq!(s.layer_face_counts()[3], 3);
        app.recompute_hydrostatics();
        let after = app.hydrostatics().unwrap();
        assert_eq!(format!("{before:?}"), format!("{after:?}"));
        // Already there: no step. Out of range, or no faces: refused.
        let undo = app.undo_len();
        let r = assign_faces(
            &mut app,
            &AssignFacesParams {
                faces: vec![0],
                layer: 3,
            },
        )
        .unwrap();
        assert!(!r.changed);
        let n = app.face_count();
        for bad in [
            AssignFacesParams {
                faces: vec![n],
                layer: 0,
            },
            AssignFacesParams {
                faces: vec![],
                layer: 0,
            },
            AssignFacesParams {
                faces: vec![0],
                layer: 4,
            },
        ] {
            assert!(assign_faces(&mut app, &bad).is_err(), "{bad:?}");
        }
        assert_eq!(app.undo_len(), undo);
    }

    #[test]
    fn the_active_layer_is_not_an_edit() {
        let mut app = demo();
        let undo = app.undo_len();
        set_active_layer(&mut app, 2).unwrap();
        assert_eq!(app.active_layer(), 2);
        assert!(set_active_layer(&mut app, 3).is_err());
        assert_eq!(app.active_layer(), 2);
        assert_eq!(app.undo_len(), undo);
        let list = list_layers(&app).unwrap();
        assert_eq!(list.active, 2);
        assert_eq!(list.layers.len(), 3);
        assert_eq!(
            list.layers.iter().map(|l| l.faces).sum::<usize>(),
            list.faces
        );
        // Deleting a layer above the active one keeps the same layer active.
        add_layer(&mut app, &AddLayerParams::default()).unwrap();
        set_active_layer(&mut app, 2).unwrap();
        delete_layer(
            &mut app,
            &DeleteLayerParams {
                layer: 0,
                move_faces_to: Some(1),
            },
        )
        .unwrap();
        assert_eq!(names(&app)[app.active_layer()], "Transom");
    }
}
