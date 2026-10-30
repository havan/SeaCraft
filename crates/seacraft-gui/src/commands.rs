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
}
