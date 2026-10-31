//! Viewport drawing.
//!
//! Everything is CPU-projected into an [`egui::Painter`]: there is no GPU
//! pipeline and the crate stays testable on a plain `egui::Context`. Each
//! viewport draws, in order, a background, a 1 m grid, the refined hull, the
//! control net, the control points, the waterplane and a caption.

use std::sync::Arc;

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};
use seacraft_core::analysis::AnalysisMode;
use seacraft_core::export::{self, RenderMesh};
use seacraft_core::math::Vec3;
use seacraft_core::mesh::Surface;
use seacraft_core::raster;
use seacraft_core::weather::WindageRect;

use crate::camera::{Camera, ViewTransform};
use crate::theme;

/// What the viewports may draw.
#[derive(Debug, Clone)]
pub struct DisplayOptions {
    /// Flat-shade the hull (otherwise only its wireframe is drawn).
    pub shaded: bool,
    /// Draw the refined mesh's edges.
    pub wireframe: bool,
    /// Draw the editable control net.
    pub control_net: bool,
    /// Draw the control points.
    pub points: bool,
    /// Draw the waterplane at the current draft.
    pub waterplane: bool,
    /// Draw project tanks and compartments.
    pub volumes: bool,
    /// Draw superstructure and deck windage profile outlines.
    pub windage: bool,
    /// Draw the ground grid.
    pub grid: bool,
    /// Draw the station, buttock and waterline intersection curves.
    pub curves: bool,
    /// Draw curvature combs (porcupines) along those curves. Independent of
    /// `curves`: the combs alone show the curvature without the lines.
    pub combs: bool,
    /// Stations (x = const cuts) across the length, ends included.
    pub stations: usize,
    /// Buttocks (y = const cuts) across the beam, sides included.
    pub buttocks: usize,
    /// Waterlines (z = const cuts) across the depth, keel and top included.
    pub waterlines: usize,
    /// Comb gain: metres of spike a curvature of 1/m draws.
    pub comb_scale: f64,
    /// Comb density: draw every `comb_stride`-th spike (1 draws them all).
    pub comb_stride: usize,
    /// Shading mode. Anything but `Shaded` renders the viewport through the
    /// CPU rasterizer, which is the only way to get per-pixel zebra stripes
    /// and curvature maps.
    pub analysis: AnalysisMode,
}

impl Default for DisplayOptions {
    /// The section counts and comb gain start at the linesplan's defaults
    /// (11 stations, 5 buttocks, 7 waterlines, gain 0.6): the viewports show
    /// what the drawing draws.
    fn default() -> Self {
        let lines = seacraft_core::linesplan::LinesplanOptions::default();
        Self {
            shaded: true,
            wireframe: false,
            control_net: true,
            points: true,
            waterplane: true,
            volumes: true,
            windage: true,
            grid: true,
            curves: false,
            combs: false,
            stations: lines.stations,
            buttocks: lines.buttocks,
            waterlines: lines.waterlines,
            comb_scale: lines.comb_scale,
            comb_stride: 1,
            analysis: AnalysisMode::Shaded,
        }
    }
}

/// The ranges the section counts and comb settings are kept in (the
/// toolbar, the View menu and the control channel all clamp to them).
pub const STATIONS_RANGE: std::ops::RangeInclusive<usize> = 2..=60;
pub const BUTTOCKS_RANGE: std::ops::RangeInclusive<usize> = 2..=30;
pub const WATERLINES_RANGE: std::ops::RangeInclusive<usize> = 2..=40;
pub const COMB_SCALE_RANGE: std::ops::RangeInclusive<f64> = 0.05..=10.0;
pub const COMB_STRIDE_RANGE: std::ops::RangeInclusive<usize> = 1..=8;

impl DisplayOptions {
    /// Whether the section curves have to be cut at all (for the lines or
    /// for their combs).
    pub fn wants_sections(&self) -> bool {
        self.curves || self.combs
    }

    /// The linesplan options the viewport curves are cut with (and the
    /// linesplan export draws with): these counts and gain at `level`.
    pub fn section_options(
        &self,
        level: u8,
        mode: seacraft_core::subdiv::SubdivisionMode,
    ) -> seacraft_core::linesplan::LinesplanOptions {
        seacraft_core::linesplan::LinesplanOptions {
            stations: self.stations,
            buttocks: self.buttocks,
            waterlines: self.waterlines,
            level,
            mode,
            combs: self.combs,
            comb_scale: self.comb_scale,
        }
    }

    /// The same options with every count and the comb settings clamped to
    /// their ranges.
    pub fn clamped(mut self) -> Self {
        let clamp = |v: usize, r: &std::ops::RangeInclusive<usize>| v.clamp(*r.start(), *r.end());
        self.stations = clamp(self.stations, &STATIONS_RANGE);
        self.buttocks = clamp(self.buttocks, &BUTTOCKS_RANGE);
        self.waterlines = clamp(self.waterlines, &WATERLINES_RANGE);
        self.comb_stride = clamp(self.comb_stride, &COMB_STRIDE_RANGE);
        self.comb_scale = if self.comb_scale.is_finite() {
            self.comb_scale
                .clamp(*COMB_SCALE_RANGE.start(), *COMB_SCALE_RANGE.end())
        } else {
            seacraft_core::linesplan::LinesplanOptions::default().comb_scale
        };
        self
    }
}

/// The kind of volume overlay to render.
#[derive(Debug, Clone, PartialEq)]
pub enum VolumeKind {
    Tank { liquid_z: Option<f64> },
    Compartment { flooded: bool },
}

/// An axis-aligned 3D box overlay representing a tank or compartment.
#[derive(Debug, Clone, PartialEq)]
pub struct VolumeOverlay {
    pub name: String,
    pub min: Vec3,
    pub max: Vec3,
    pub kind: VolumeKind,
    pub selected: bool,
    pub clip_to_hull: bool,
    pub clipped_mesh: Option<(Vec<Vec3>, Vec<[usize; 3]>)>,
}

/// A 2D lateral windage profile outline in the model's y = 0 plane.
#[derive(Debug, Clone, PartialEq)]
pub struct WindageOverlay {
    pub name: String,
    pub rect: WindageRect,
    pub selected: bool,
}

/// Everything a viewport draws.
pub struct Scene<'a> {
    pub surface: &'a Surface,
    /// The refined hull, already mirrored and filtered to visible layers.
    pub mesh: &'a RenderMesh,
    /// Optional 3D stations/buttocks/waterlines and their curvature combs.
    pub curves: Option<&'a seacraft_core::linesplan::LinesplanCurves3d>,
    /// Project tanks and compartments.
    pub volumes: &'a [VolumeOverlay],
    /// Superstructure and deck windage profile outlines.
    pub windage: &'a [WindageOverlay],
    pub min: Vec3,
    pub max: Vec3,
    /// Current floating condition, for the waterplane.
    pub draft: f64,
    pub trim: f64,
    pub heel_deg: f64,
}

impl Scene<'_> {
    fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }
}

/// The box the viewports frame and draw around: the control net's extent,
/// mirrored across the centreline when a symmetric layer has faces, since
/// the refined hull is always shown mirrored ([`export::render_mesh`]). A
/// half model framed on its modelled `y >= 0` half alone sits off-centre
/// with its mirror image cut off.
pub fn display_extents(surface: &Surface) -> (Vec3, Vec3) {
    let (mut min, mut max) = surface.extents();
    let mirrored = surface
        .face_layer
        .iter()
        .any(|&l| surface.layers.get(l).is_some_and(|l| l.symmetric));
    if mirrored {
        let reach = max.y.abs().max(min.y.abs());
        min.y = -reach;
        max.y = reach;
    }
    (min, max)
}

/// Project every vertex once; `None` for points behind a perspective camera.
pub fn project_verts(view: &ViewTransform, verts: &[Vec3], rect: Rect) -> Vec<Option<Pos2>> {
    verts.iter().map(|p| view.project(*p, rect)).collect()
}

/// Draw one viewport. `active` marks the viewport that receives picking.
pub fn viewport(
    ui: &mut egui::Ui,
    rect: Rect,
    camera: &Camera,
    scene: Option<&Scene>,
    opts: &DisplayOptions,
    active: bool,
    selected: Option<usize>,
) {
    viewport_with_texture(ui, rect, camera, scene, opts, active, selected, None, None)
}

/// Draw a viewport, optionally using `texture` (a rasterizer frame) in place
/// of the painter's shaded hull, or `hull`, the [`shaded_hull`] mesh built
/// for this view on an earlier frame.
#[allow(clippy::too_many_arguments)]
pub fn viewport_with_texture(
    ui: &mut egui::Ui,
    rect: Rect,
    camera: &Camera,
    scene: Option<&Scene>,
    opts: &DisplayOptions,
    active: bool,
    selected: Option<usize>,
    texture: Option<&egui::TextureHandle>,
    hull: Option<Arc<egui::Mesh>>,
) {
    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::CANVAS);
    let Some(scene) = scene else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No model loaded",
            FontId::proportional(14.0),
            theme::TEXT_SECONDARY,
        );
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, theme::FRAME),
            egui::StrokeKind::Inside,
        );
        return;
    };

    let view = camera.transform();
    if opts.grid {
        draw_grid(&painter, &view, rect, scene);
    }
    match texture {
        Some(texture) => {
            // A rasterizer frame: per-pixel shading, already depth-tested.
            painter.image(
                texture.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::WHITE,
            );
        }
        None => {
            if opts.shaded || opts.wireframe {
                draw_hull(&painter, &view, rect, scene, opts, hull);
            }
        }
    }
    if opts.waterplane {
        draw_waterplane(&painter, &view, rect, scene, texture.is_none());
    }
    if opts.volumes {
        draw_volumes(&painter, &view, rect, scene.volumes);
    }
    if opts.windage && camera.kind == crate::camera::ViewKind::Profile {
        draw_windage(&painter, &view, rect, scene.windage);
    }
    if opts.wants_sections() {
        if let Some(curves) = scene.curves {
            draw_sections(&painter, &view, camera.kind, rect, curves, opts);
        }
    }
    if opts.control_net {
        draw_control_net(&painter, &view, rect, scene.surface);
    }
    if opts.points {
        draw_points(&painter, &view, rect, scene.surface, selected);
    }

    // Caption and scale, on a backing so the hull behind stays readable.
    hud_caption(
        &painter,
        title_pos(rect),
        Align2::LEFT_TOP,
        camera.kind.label().to_owned(),
    );
    // The scale in metres rather than m/px: the side of a grid square is
    // what the eye judges a length against.
    let (minor, major) = view_grid_steps(&view, rect, scene.min, scene.max);
    if let Some(text) = grid_caption(view.perspective, opts.grid, minor, major) {
        hud_caption(
            &painter,
            rect.right_top() + egui::vec2(-8.0, 6.0),
            Align2::RIGHT_TOP,
            text,
        );
    }
    if !view.perspective {
        draw_scale_bar(
            &painter,
            &scale_bar(rect, view.world_per_px(rect, 1.0), minor),
        );
    }
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(
            if active { 2.0 } else { 1.0 },
            if active {
                theme::FRAME_ACTIVE
            } else {
                theme::FRAME
            },
        ),
        egui::StrokeKind::Inside,
    );
}

/// Where a viewport's title caption is anchored (its top-left corner).
fn title_pos(rect: Rect) -> Pos2 {
    rect.left_top() + egui::vec2(8.0, 6.0)
}

/// The backing rectangle of the title caption of a viewport drawn in `rect`
/// with `camera`: the caption a double-click maximises.
pub fn title_rect(painter: &egui::Painter, rect: Rect, camera: &Camera) -> Rect {
    let galley = painter.layout_no_wrap(
        camera.kind.label().to_owned(),
        theme::caption_font(),
        theme::TEXT_SECONDARY,
    );
    caption_backing(Align2::LEFT_TOP.anchor_size(title_pos(rect), galley.size()))
}

fn caption_backing(text_rect: Rect) -> Rect {
    text_rect.expand2(egui::vec2(4.0, 2.0))
}

/// A viewport caption in the caption font on a semi-opaque canvas backing.
/// Returns the backing rectangle.
pub fn hud_caption(painter: &egui::Painter, pos: Pos2, align: Align2, text: String) -> Rect {
    let galley = painter.layout_no_wrap(text, theme::caption_font(), theme::TEXT_SECONDARY);
    let text_rect = align.anchor_size(pos, galley.size());
    let backing = caption_backing(text_rect);
    painter.rect_filled(backing, 2.0, theme::HUD_BACKING);
    painter.galley(text_rect.min, galley, theme::TEXT_SECONDARY);
    backing
}

/// A length in metres as a grid label: `0.5 m`, `1 m`, `50 m`, `1 km`.
pub fn format_length(m: f64) -> String {
    let (value, unit) = if m >= 1000.0 {
        (m / 1000.0, "km")
    } else {
        (m, "m")
    };
    // The ladder's steps are round numbers: no trailing zeros.
    let text = format!("{value:.2}");
    let text = text.trim_end_matches('0').trim_end_matches('.');
    format!("{text} {unit}")
}

/// The minor and major grid steps a view draws (see [`grid_segments`]): an
/// orthographic view's at its own scale, the perspective view's ground grid
/// at the depth of the model's centre.
pub fn view_grid_steps(view: &ViewTransform, rect: Rect, min: Vec3, max: Vec3) -> (f64, f64) {
    let world_per_px = if view.perspective {
        let centre = (min + max) * 0.5;
        let depth = (centre - view.eye).dot(view.forward).max(1e-3);
        view.world_per_px(rect, depth)
    } else {
        view.world_per_px(rect, 1.0)
    };
    grid_steps(1.0 / world_per_px)
}

/// The scale caption of a view: `grid 1 m · major 5 m` (the perspective
/// view: `ground grid …`). Without the grid an orthographic view's scale bar
/// carries the scale alone and the perspective view shows none: `None`.
pub fn grid_caption(perspective: bool, grid: bool, minor: f64, major: f64) -> Option<String> {
    grid.then(|| {
        format!(
            "{}grid {} · major {}",
            if perspective { "ground " } else { "" },
            format_length(minor),
            format_length(major)
        )
    })
}

/// The scale bar in an orthographic view's bottom-left corner: one minor
/// grid step long, `├──┤ 1 m`, on a caption backing.
#[derive(Debug, Clone, PartialEq)]
pub struct ScaleBar {
    /// The bar's two ends on screen (`b.x - a.x` is the step in pixels).
    pub a: Pos2,
    pub b: Pos2,
    /// Its length, as [`format_length`] writes it.
    pub label: String,
    /// The label's left edge, level with the bar.
    pub label_pos: Pos2,
}

/// Height of the scale bar's end ticks.
const SCALE_BAR_TICK: f32 = 6.0;

/// The scale bar of a view drawn in `rect` at `world_per_px`, one `step`
/// (the minor grid step) long.
pub fn scale_bar(rect: Rect, world_per_px: f64, step: f64) -> ScaleBar {
    let length = (step / world_per_px.max(1e-12)) as f32;
    let a = rect.left_bottom() + egui::vec2(14.0, -14.0);
    let b = a + egui::vec2(length, 0.0);
    ScaleBar {
        a,
        b,
        label: format_length(step),
        label_pos: b + egui::vec2(6.0, 0.0),
    }
}

/// Draw a [`ScaleBar`]: the bar with an end tick each side and its label,
/// on one semi-opaque backing.
fn draw_scale_bar(painter: &egui::Painter, bar: &ScaleBar) {
    let galley = painter.layout_no_wrap(
        bar.label.clone(),
        theme::caption_font(),
        theme::TEXT_SECONDARY,
    );
    let text_rect = Align2::LEFT_CENTER.anchor_size(bar.label_pos, galley.size());
    let tick = egui::vec2(0.0, 0.5 * SCALE_BAR_TICK);
    let bar_rect = Rect::from_min_max(bar.a - tick, bar.b + tick);
    let backing = caption_backing(bar_rect.union(text_rect)).expand2(egui::vec2(2.0, 1.0));
    painter.rect_filled(backing, 2.0, theme::HUD_BACKING);
    let stroke = Stroke::new(1.0, theme::TEXT_SECONDARY);
    painter.line_segment([bar.a, bar.b], stroke);
    painter.line_segment([bar.a - tick, bar.a + tick], stroke);
    painter.line_segment([bar.b - tick, bar.b + tick], stroke);
    painter.galley(text_rect.min, galley, theme::TEXT_SECONDARY);
}

/// The three tiers of the viewport grid, weakest first (the drawing order).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum GridTier {
    /// Every minor step (1 m at the usual zoom).
    Minor,
    /// Every major step (5 m at the usual zoom).
    Major,
    /// Centreline, baseline, x = 0, midship and the design waterline.
    Datum,
}

/// One grid line on screen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct GridLine {
    pub a: Pos2,
    pub b: Pos2,
    pub tier: GridTier,
}

/// Minor grid lines never sit closer than this on screen.
pub const GRID_MIN_SPACING_PX: f64 = 12.0;

/// Sub-metre minor lines only once they are this far apart: zoomed in on a
/// detail, not at the usual whole-hull zoom, where 1 m is the grid.
const GRID_SUBMETRE_SPACING_PX: f64 = 40.0;

/// The minor and major grid steps, in metres, at `px_per_m` pixels per
/// metre: the finest minor step at least [`GRID_MIN_SPACING_PX`] apart on
/// screen (sub-metre steps at least 40 px). 1 m minor lines come with 5 m
/// majors; zoomed out, 5 m minors come with 10 m majors, 10 m with 50 m, and
/// so on.
pub fn grid_steps(px_per_m: f64) -> (f64, f64) {
    const LADDER: [(f64, f64); 11] = [
        (0.1, 0.5),
        (0.5, 1.0),
        (1.0, 5.0),
        (5.0, 10.0),
        (10.0, 50.0),
        (50.0, 100.0),
        (100.0, 500.0),
        (500.0, 1000.0),
        (1000.0, 5000.0),
        (5000.0, 10000.0),
        (10000.0, 50000.0),
    ];
    let px = if px_per_m.is_finite() && px_per_m > 0.0 {
        px_per_m
    } else {
        1e-9
    };
    LADDER
        .into_iter()
        .find(|(minor, _)| {
            let need = if *minor < 1.0 {
                GRID_SUBMETRE_SPACING_PX
            } else {
                GRID_MIN_SPACING_PX
            };
            minor * px >= need
        })
        .unwrap_or(LADDER[LADDER.len() - 1])
}

/// Grid lines on multiples of `minor` covering `lo..=hi`, each with its
/// tier: [`GridTier::Major`] on multiples of `major`, [`GridTier::Datum`] at
/// zero when `zero_is_datum`.
fn tiered_lines(
    lo: f64,
    hi: f64,
    minor: f64,
    major: f64,
    zero_is_datum: bool,
) -> Vec<(f64, GridTier)> {
    let ratio = (major / minor).round().max(1.0) as i64;
    let i0 = (lo / minor).floor() as i64;
    let i1 = (hi / minor).ceil() as i64;
    (i0..=i1)
        .map(|i| {
            let tier = if i == 0 && zero_is_datum {
                GridTier::Datum
            } else if i.rem_euclid(ratio) == 0 {
                GridTier::Major
            } else {
                GridTier::Minor
            };
            (i as f64 * minor, tier)
        })
        .collect()
}

/// The viewport grid on screen, weakest tier first.
///
/// Orthographic views draw a grid in the view plane, world-anchored (a line
/// at 3 m is at 3 m in every view) and covering the whole cell, with the
/// datums where the view runs along world axes: centreline, baseline, x = 0,
/// midship (the middle of `min..max` in x) and, in the views with z up, the
/// design waterline at `dwl_z`.
///
/// The perspective view draws a ground grid on the baseline plane z = 0
/// around the model instead of a screen-space wallpaper: it foreshortens
/// with the hull, so it says which way is down and how far away things are,
/// and its lines are the same metres as the orthographic views'.
pub fn grid_segments(
    view: &ViewTransform,
    rect: Rect,
    min: Vec3,
    max: Vec3,
    dwl_z: Option<f64>,
) -> Vec<GridLine> {
    let mut lines = if view.perspective {
        ground_grid(view, rect, min, max)
    } else {
        ortho_grid(view, rect, min, max, dwl_z)
    };
    lines.sort_by_key(|l| l.tier);
    lines
}

fn ortho_grid(
    view: &ViewTransform,
    rect: Rect,
    min: Vec3,
    max: Vec3,
    dwl_z: Option<f64>,
) -> Vec<GridLine> {
    let mut out = Vec::new();
    let world_per_px = view.world_per_px(rect, 1.0);
    let (minor, major) = view_grid_steps(view, rect, min, max);
    // The screen centre is the camera target; the eye sits on its axis.
    let (base, up_base) = (view.eye.dot(view.right), view.eye.dot(view.up));
    let half_w = 0.5 * rect.width() as f64 * world_per_px + minor;
    let half_h = 0.5 * rect.height() as f64 * world_per_px + minor;
    let datum_right = is_world_axis(view.right);
    let datum_up = is_world_axis(view.up);
    let mut push = |a: Vec3, b: Vec3, tier: GridTier| {
        if let (Some(a), Some(b)) = (view.project(a, rect), view.project(b, rect)) {
            out.push(GridLine { a, b, tier });
        }
    };
    let vertical = |u: f64| {
        (
            view.right * u + view.up * (up_base - half_h),
            view.right * u + view.up * (up_base + half_h),
        )
    };
    let horizontal = |v: f64| {
        (
            view.right * (base - half_w) + view.up * v,
            view.right * (base + half_w) + view.up * v,
        )
    };
    for (u, tier) in tiered_lines(base - half_w, base + half_w, minor, major, datum_right) {
        let (a, b) = vertical(u);
        push(a, b, tier);
    }
    for (v, tier) in tiered_lines(up_base - half_h, up_base + half_h, minor, major, datum_up) {
        let (a, b) = horizontal(v);
        push(a, b, tier);
    }
    // Midship: the middle of the model's length, where a view runs along x.
    let mid_x = 0.5 * (min.x + max.x);
    if datum_right && view.right.x.abs() > 0.999 {
        let (a, b) = vertical(mid_x * view.right.x.signum());
        push(a, b, GridTier::Datum);
    }
    if datum_up && view.up.x.abs() > 0.999 {
        let (a, b) = horizontal(mid_x * view.up.x.signum());
        push(a, b, GridTier::Datum);
    }
    // The design waterline, in the views with z up (profile, body plan).
    if let Some(z) = dwl_z.filter(|z| z.is_finite()) {
        if view.up.z > 0.999 {
            let (a, b) = horizontal(z);
            push(a, b, GridTier::Datum);
        }
    }
    out
}

/// The perspective view's ground grid on z = 0, around the model's plan.
fn ground_grid(view: &ViewTransform, rect: Rect, min: Vec3, max: Vec3) -> Vec<GridLine> {
    let mut out = Vec::new();
    // Scale at the depth of the model's centre, where the eye looks.
    let (minor, major) = view_grid_steps(view, rect, min, max);
    // The model's plan plus half its length all round, so the grid frames
    // the hull and stops well before the horizon.
    let size = (max.x - min.x).max(max.y - min.y).max(1.0);
    let pad = 0.5 * size;
    let (x0, x1) = (min.x - pad, max.x + pad);
    let (y0, y1) = (min.y - pad, max.y + pad);
    let x_lines = tiered_lines(x0, x1, minor, major, true);
    let y_lines = tiered_lines(y0, y1, minor, major, true);
    let (xs, xe) = (x_lines[0].0, x_lines[x_lines.len() - 1].0);
    let (ys, ye) = (y_lines[0].0, y_lines[y_lines.len() - 1].0);
    let mut push = |a: Vec3, b: Vec3, tier: GridTier| {
        if let Some((a, b)) = clip_near(view, a, b) {
            if let (Some(a), Some(b)) = (view.project(a, rect), view.project(b, rect)) {
                out.push(GridLine { a, b, tier });
            }
        }
    };
    for &(x, tier) in &x_lines {
        push(Vec3::new(x, ys, 0.0), Vec3::new(x, ye, 0.0), tier);
    }
    for &(y, tier) in &y_lines {
        push(Vec3::new(xs, y, 0.0), Vec3::new(xe, y, 0.0), tier);
    }
    let mid_x = 0.5 * (min.x + max.x);
    push(
        Vec3::new(mid_x, ys, 0.0),
        Vec3::new(mid_x, ye, 0.0),
        GridTier::Datum,
    );
    out
}

/// The part of segment `a`–`b` in front of a perspective camera's near
/// plane, or `None` when all of it is behind.
fn clip_near(view: &ViewTransform, a: Vec3, b: Vec3) -> Option<(Vec3, Vec3)> {
    const NEAR: f64 = 0.1;
    let (da, db) = (
        (a - view.eye).dot(view.forward),
        (b - view.eye).dot(view.forward),
    );
    match (da >= NEAR, db >= NEAR) {
        (true, true) => Some((a, b)),
        (false, false) => None,
        (true, false) => Some((a, a + (b - a) * ((da - NEAR) / (da - db)))),
        (false, true) => Some((b + (a - b) * ((db - NEAR) / (db - da)), b)),
    }
}

/// Draw [`grid_segments`]: minor lines thin and quiet, major lines a little
/// stronger, datums strongest.
fn draw_grid(painter: &egui::Painter, view: &ViewTransform, rect: Rect, scene: &Scene) {
    let dwl_z = scene.surface.lowest_hydrostatics_point() + scene.draft;
    for line in grid_segments(view, rect, scene.min, scene.max, Some(dwl_z)) {
        let stroke = match line.tier {
            GridTier::Minor => Stroke::new(1.0, theme::GRID_MINOR),
            GridTier::Major => Stroke::new(1.0, theme::GRID_MAJOR),
            GridTier::Datum => Stroke::new(1.2, theme::GRID_DATUM),
        };
        painter.line_segment([line.a, line.b], stroke);
    }
}

/// Grid line coordinates on multiples of `step` covering `base ± half`,
/// each flagged when it is the world datum (coordinate zero). Anchored to the
/// world, not to the model, so a line at 3 m is at 3 m in every view.
pub fn grid_lines(base: f64, half: f64, step: f64) -> Vec<(f64, bool)> {
    let i0 = ((base - half) / step).floor() as i64;
    let i1 = ((base + half) / step).ceil() as i64;
    (i0..=i1).map(|i| (i as f64 * step, i == 0)).collect()
}

/// Whether a view axis runs along a world axis (the orthographic views), so
/// its zero line is a datum: the centreline, the baseline or x = 0.
fn is_world_axis(axis: Vec3) -> bool {
    axis.x.abs().max(axis.y.abs()).max(axis.z.abs()) > 0.999
}

fn draw_waterplane(
    painter: &egui::Painter,
    view: &ViewTransform,
    rect: Rect,
    scene: &Scene,
    fill_quad: bool,
) {
    let plane =
        seacraft_core::hydro::waterplane(scene.surface, scene.draft, scene.trim, scene.heel_deg);
    let (u, v) = plane.basis();
    let center = plane.project(scene.center());
    let mut u_min = f64::INFINITY;
    let mut u_max = f64::NEG_INFINITY;
    let mut v_min = f64::INFINITY;
    let mut v_max = f64::NEG_INFINITY;
    for x in [scene.min.x, scene.max.x] {
        for y in [scene.min.y, scene.max.y] {
            for z in [scene.min.z, scene.max.z] {
                let d = plane.project(Vec3::new(x, y, z)) - center;
                let du = d.dot(u);
                let dv = d.dot(v);
                u_min = u_min.min(du);
                u_max = u_max.max(du);
                v_min = v_min.min(dv);
                v_max = v_max.max(dv);
            }
        }
    }
    let pad_u = ((u_max - u_min) * 0.12).max(0.2);
    let pad_v = ((v_max - v_min) * 0.12).max(0.2);
    let corners = [
        center + u * (u_min - pad_u) + v * (v_min - pad_v),
        center + u * (u_max + pad_u) + v * (v_min - pad_v),
        center + u * (u_max + pad_u) + v * (v_max + pad_v),
        center + u * (u_min - pad_u) + v * (v_max + pad_v),
    ];
    let pts: Vec<Pos2> = corners
        .iter()
        .filter_map(|p| view.project(*p, rect))
        .collect();
    if pts.len() == 4 {
        if view.perspective && fill_quad {
            let mut quad_mesh = egui::Mesh::default();
            for &p in &pts {
                quad_mesh.colored_vertex(p, theme::WATERPLANE_FILL);
            }
            quad_mesh.add_triangle(0, 1, 2);
            quad_mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(quad_mesh));
        }
        let border = theme::WATERPLANE_EDGE;
        for i in 0..4 {
            painter.line_segment([pts[i], pts[(i + 1) % 4]], Stroke::new(1.0, border));
        }
    }

    // Draw the true waterline intersection curve on the hull surface.
    for f in &scene.mesh.faces {
        if f.len() < 3 {
            continue;
        }
        for k in 1..f.len() - 1 {
            let tri = [
                scene.mesh.verts[f[0]],
                scene.mesh.verts[f[k]],
                scene.mesh.verts[f[k + 1]],
            ];
            let s = [plane.side(tri[0]), plane.side(tri[1]), plane.side(tri[2])];
            let mut hits: Vec<Vec3> = Vec::with_capacity(2);
            for (i, j) in [(0usize, 1usize), (1, 2), (2, 0)] {
                if (s[i] <= 0.0) != (s[j] <= 0.0) {
                    let denom = s[i] - s[j];
                    let t = if denom.abs() > 1e-15 {
                        (s[i] / denom).clamp(0.0, 1.0)
                    } else {
                        0.5
                    };
                    hits.push(tri[i] + (tri[j] - tri[i]) * t);
                }
            }
            if hits.len() == 2 {
                if let (Some(p0), Some(p1)) =
                    (view.project(hits[0], rect), view.project(hits[1], rect))
                {
                    painter.line_segment([p0, p1], Stroke::new(1.5, theme::DWL));
                }
            }
        }
    }
}

/// The intersection curves (`opts.curves`) and their curvature combs
/// (`opts.combs`, every `comb_stride`-th spike) each view shows: the body
/// plan its stations, the profile its buttocks, the plan its waterlines and
/// the perspective all three.
fn draw_sections(
    painter: &egui::Painter,
    view: &ViewTransform,
    kind: crate::camera::ViewKind,
    rect: Rect,
    curves: &seacraft_core::linesplan::LinesplanCurves3d,
    opts: &DisplayOptions,
) {
    let stride = opts.comb_stride.max(1);
    let groups: &[&[seacraft_core::linesplan::CurveWithCombs]] = match kind {
        crate::camera::ViewKind::Profile => &[&curves.buttocks],
        crate::camera::ViewKind::Plan => &[&curves.waterlines],
        crate::camera::ViewKind::Body => &[&curves.stations],
        crate::camera::ViewKind::Perspective => {
            &[&curves.stations, &curves.buttocks, &curves.waterlines]
        }
    };
    for group in groups {
        for curve in *group {
            if opts.curves {
                for w in curve.points.windows(2) {
                    if let (Some(p), Some(q)) = (view.project(w[0], rect), view.project(w[1], rect))
                    {
                        painter.line_segment([p, q], Stroke::new(1.0, theme::SECTION_CURVE));
                    }
                }
                if curve.closed && curve.points.len() >= 3 {
                    if let (Some(p), Some(q)) = (
                        view.project(*curve.points.last().unwrap(), rect),
                        view.project(curve.points[0], rect),
                    ) {
                        painter.line_segment([p, q], Stroke::new(1.0, theme::SECTION_CURVE));
                    }
                }
            }
            if !opts.combs {
                continue;
            }
            let mut tips: Vec<Pos2> = Vec::with_capacity(curve.spikes.len() / stride + 1);
            for &(base, tip) in curve.spikes.iter().step_by(stride) {
                if let (Some(b), Some(t)) = (view.project(base, rect), view.project(tip, rect)) {
                    painter.line_segment([b, t], Stroke::new(0.8, theme::COMB_SPIKE));
                    tips.push(t);
                }
            }
            for w in tips.windows(2) {
                painter.line_segment([w[0], w[1]], Stroke::new(1.1, theme::COMB_ENVELOPE));
            }
        }
    }
}

/// The flat-shaded hull as one triangle mesh, farthest face first, for the
/// painter. It depends only on the refined mesh, the layer colours and the
/// view, so a viewport keeps it while none of them change ([`hull_key`]).
pub fn shaded_hull(view: &ViewTransform, rect: Rect, scene: &Scene) -> egui::Mesh {
    let mesh = scene.mesh;
    let mut tri_mesh = egui::Mesh::default();
    if mesh.faces.is_empty() {
        return tri_mesh;
    }
    // A key light from over the viewer's shoulder: the hull reads as a
    // solid form instead of a silhouette.
    let light = (-view.forward + view.up * 0.45 + view.right * 0.35).norm();
    let mut items: Vec<(f64, Vec<Pos2>, Color32)> = Vec::with_capacity(mesh.faces.len());
    let mut poly: Vec<Vec3> = Vec::new();
    for (fi, f) in mesh.faces.iter().enumerate() {
        if f.len() < 3 {
            continue;
        }
        let mut pts: Vec<Pos2> = Vec::with_capacity(f.len());
        let mut center = Vec3::ZERO;
        let mut depth = 0.0;
        let mut complete = true;
        for &i in f {
            match view.project(mesh.verts[i], rect) {
                Some(p) => pts.push(p),
                None => {
                    complete = false;
                    break;
                }
            }
            center += mesh.verts[i];
            depth += (mesh.verts[i] - view.eye).dot(view.forward);
        }
        if !complete {
            continue;
        }
        let n = f.len() as f64;
        center = center / n;
        depth /= n;

        poly.clear();
        poly.extend(f.iter().map(|&i| mesh.verts[i]));
        let normal = export::newell_normal(&poly);
        if normal.length() < 1e-12 {
            continue;
        }
        let normal = normal.norm();
        let to_eye = if view.perspective {
            (view.eye - center).norm()
        } else {
            -view.forward
        };
        // Render both sides of open hull shells (such as the top-down Plan view),
        // dimming the interior slightly so the outside remains distinct.
        let facing = normal.dot(to_eye);
        let side_factor = if facing >= 0.0 { 1.0 } else { 0.82 };

        let base = layer_color(scene.surface, mesh.face_layer.get(fi).copied());
        let shade = ((0.35 + 0.65 * normal.dot(light).abs()) * side_factor) as f32;
        items.push((depth, pts, base.gamma_multiply(shade)));
    }
    // Painter's algorithm: farthest face first, emitted into one triangle
    // mesh so epaint's path miter-join extruder never shoots spikes on
    // near-silhouette faces.
    items.sort_by(|a, b| b.0.total_cmp(&a.0));
    for (_, pts, color) in items {
        if pts.len() < 3 {
            continue;
        }
        let base_idx = tri_mesh.vertices.len() as u32;
        for &p in &pts {
            tri_mesh.colored_vertex(p, color);
        }
        for k in 1..(pts.len() as u32 - 1) {
            tri_mesh.add_triangle(base_idx, base_idx + k, base_idx + k + 1);
        }
    }
    tri_mesh
}

/// The key a [`shaded_hull`] mesh is kept under: the view and the mesh
/// `stamp` ([`frame_key`]), where the cell sits on screen (the mesh is in
/// screen coordinates) and the layer colours.
pub fn hull_key(
    stamp: u64,
    camera: &Camera,
    rect: Rect,
    scene: &Scene,
    opts: &DisplayOptions,
) -> u64 {
    let mut h = frame_key(stamp, camera, rect, scene, opts);
    mix(&mut h, rect.min.x as f64);
    mix(&mut h, rect.min.y as f64);
    for layer in &scene.surface.layers {
        mix(&mut h, layer.color as f64);
    }
    h
}

fn draw_hull(
    painter: &egui::Painter,
    view: &ViewTransform,
    rect: Rect,
    scene: &Scene,
    opts: &DisplayOptions,
    hull: Option<Arc<egui::Mesh>>,
) {
    let mesh = scene.mesh;

    // ---- flat-shaded faces ------------------------------------------------
    if opts.shaded {
        let hull = hull.unwrap_or_else(|| Arc::new(shaded_hull(view, rect, scene)));
        if !hull.is_empty() {
            painter.add(egui::Shape::Mesh(hull));
        }
    }

    // ---- wireframe --------------------------------------------------------
    if opts.wireframe {
        for f in &mesh.faces {
            for i in 0..f.len() {
                let a = f[i];
                let b = f[(i + 1) % f.len()];
                if let (Some(p), Some(q)) = (
                    view.project(mesh.verts[a], rect),
                    view.project(mesh.verts[b], rect),
                ) {
                    painter.line_segment([p, q], Stroke::new(0.7, theme::WIREFRAME));
                }
            }
        }
    }
}

/// A layer's colour, or the default hull grey when the index is out of range.
fn layer_color(surface: &Surface, layer: Option<usize>) -> Color32 {
    let Some(layer) = layer.and_then(|i| surface.layers.get(i)) else {
        return theme::HULL_DEFAULT;
    };
    theme::layer_colour(layer.color)
}

fn draw_control_net(painter: &egui::Painter, view: &ViewTransform, rect: Rect, surface: &Surface) {
    let mut creases: std::collections::HashSet<(usize, usize)> = std::collections::HashSet::new();
    for e in &surface.edges {
        if e.crease {
            creases.insert(e.key());
        }
    }
    for f in &surface.faces {
        for i in 0..f.len() {
            let a = f[i];
            let b = f[(i + 1) % f.len()];
            let (Some(p), Some(q)) = (
                view.project(surface.points[a].p, rect),
                view.project(surface.points[b].p, rect),
            ) else {
                continue;
            };
            let crease = creases.contains(&seacraft_core::mesh::ordered(a, b));
            painter.line_segment(
                [p, q],
                Stroke::new(
                    if crease { 1.6 } else { 1.0 },
                    if crease {
                        theme::CREASE
                    } else {
                        theme::CONTROL_NET
                    },
                ),
            );
        }
    }
}

fn draw_points(
    painter: &egui::Painter,
    view: &ViewTransform,
    rect: Rect,
    surface: &Surface,
    selected: Option<usize>,
) {
    for (i, p) in surface.points.iter().enumerate() {
        let Some(s) = view.project(p.p, rect) else {
            continue;
        };
        if i == selected.unwrap_or(usize::MAX) {
            painter.circle_filled(s, 5.0, theme::POINT_SELECTED);
            painter.circle_stroke(s, 5.0, Stroke::new(1.0, theme::POINT_SELECTED_OUTLINE));
        } else {
            painter.circle_filled(s, 2.0, theme::POINT);
        }
    }
}

fn mix(h: &mut u64, x: f64) {
    *h ^= x.to_bits();
    *h = h.wrapping_mul(0x0100_0000_01b3);
}

/// A hash of what one view's rasterized frame looks like from: the camera,
/// the viewport size, the analysis mode and the waterplane. It leaves out the
/// model, so a view whose camera did not move can keep its frame while the
/// geometry is being dragged in another view.
pub fn view_key(camera: &Camera, rect: Rect, scene: &Scene, opts: &DisplayOptions) -> u64 {
    let view = camera.transform();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    mix(&mut h, rect.width() as f64);
    mix(&mut h, rect.height() as f64);
    for p in [view.eye, view.right, view.up, view.forward] {
        mix(&mut h, p.x);
        mix(&mut h, p.y);
        mix(&mut h, p.z);
    }
    mix(&mut h, view.ortho_height);
    mix(&mut h, view.fov_tan);
    mix(&mut h, view.fit_aspect);
    mix(&mut h, if view.perspective { 1.0 } else { 0.0 });
    let (a, b) = match opts.analysis {
        AnalysisMode::Shaded => (0.0, 0.0),
        AnalysisMode::Zebra { stripes } => (1.0, stripes),
        AnalysisMode::Curvature { radius } => (2.0, radius),
        AnalysisMode::Draft { angle_deg } => (3.0, angle_deg),
    };
    mix(&mut h, a);
    mix(&mut h, b);
    mix(&mut h, if opts.waterplane { 1.0 } else { 0.0 });
    if opts.waterplane {
        mix(&mut h, scene.draft);
        mix(&mut h, scene.trim);
        mix(&mut h, scene.heel_deg);
    }
    h
}

/// A hash of everything a rasterized frame depends on, so the texture is only
/// rebuilt when the model, the camera, the viewport size or the mode changes:
/// the [`view_key`] plus `revision`, the stamp of the refined mesh drawn.
pub fn frame_key(
    revision: u64,
    camera: &Camera,
    rect: Rect,
    scene: &Scene,
    opts: &DisplayOptions,
) -> u64 {
    let mut h = view_key(camera, rect, scene, opts);
    mix(&mut h, revision as f64);
    mix(&mut h, sections_key(opts) as f64);
    h
}

/// A hash of the section overlay settings: the curve and comb toggles and,
/// while either is on, the counts; while the combs are on, their gain and
/// density. Part of [`frame_key`], so a view's cached frame never outlives
/// the overlay it was drawn with, and keys the viewport curve cache.
pub fn sections_key(opts: &DisplayOptions) -> u64 {
    let mut h: u64 = 0x6c62_272e_07bb_0142;
    mix(&mut h, if opts.curves { 1.0 } else { 0.0 });
    mix(&mut h, if opts.combs { 1.0 } else { 0.0 });
    if opts.wants_sections() {
        mix(&mut h, opts.stations as f64);
        mix(&mut h, opts.buttocks as f64);
        mix(&mut h, opts.waterlines as f64);
    }
    if opts.combs {
        mix(&mut h, opts.comb_scale);
        mix(&mut h, opts.comb_stride as f64);
    }
    h
}

/// A title for the analysis mode, for the viewport caption and the menu.
pub fn mode_label(mode: AnalysisMode) -> &'static str {
    match mode {
        AnalysisMode::Shaded => "Shaded",
        AnalysisMode::Zebra { .. } => "Zebra",
        AnalysisMode::Curvature { .. } => "Gaussian curvature",
        AnalysisMode::Draft { .. } => "Draft angle",
    }
}

/// The rasterizer's camera for this view transform, at `width × height` pixels.
/// The rasterizer scales by the image height, so it gets the height the
/// aspect-aware transform actually shows over the whole cell.
pub fn raster_view(view: &ViewTransform, width: usize, height: usize) -> raster::View {
    let (w, h) = (width as f64, height as f64);
    raster::View {
        eye: view.eye,
        right: view.right,
        up: view.up,
        forward: view.forward,
        ortho_height: view.visible_height_wh(w, h),
        fov_tan: view.visible_fov_tan_wh(w, h),
        perspective: view.perspective,
    }
}

/// The rasterizer's background gradient, top and bottom.
const RASTER_BACKGROUND: ([f64; 3], [f64; 3]) = (
    [
        0x13 as f64 / 255.0,
        0x19 as f64 / 255.0,
        0x21 as f64 / 255.0,
    ],
    [
        0x0d as f64 / 255.0,
        0x12 as f64 / 255.0,
        0x18 as f64 / 255.0,
    ],
);

/// The waterplane tint the rasterizer lays over the background (away from
/// the hull) in a perspective view, and its opacity (`raster::render`).
const RASTER_WATERPLANE_TINT: ([f64; 3], f64) = ([48.0, 108.0, 184.0], 0.22);

/// Render one viewport with the CPU rasterizer. Returns the RGBA pixels.
pub fn raster_frame(
    view: &ViewTransform,
    scene: &Scene,
    mode: AnalysisMode,
    waterplane: bool,
    width: usize,
    height: usize,
) -> (usize, usize, Vec<u8>) {
    let opts = raster::RenderOptions {
        width,
        height,
        mode,
        background: RASTER_BACKGROUND,
        supersample: 1,
        wireframe: false,
        two_sided: true,
        waterplane: (waterplane && view.perspective).then(|| {
            seacraft_core::hydro::waterplane(scene.surface, scene.draft, scene.trim, scene.heel_deg)
        }),
    };
    let canvas = raster::render(
        &scene.mesh.verts,
        &scene.mesh.faces,
        &raster_view(view, width, height),
        &opts,
    );
    (canvas.w, canvas.h, canvas.rgba)
}

/// A viewport's analysis frame: [`raster_frame`] with the background made
/// see-through, so the grid drawn under it shows around the hull as it does
/// in the shaded mode.
pub fn analysis_image(
    view: &ViewTransform,
    scene: &Scene,
    mode: AnalysisMode,
    waterplane: bool,
    width: usize,
    height: usize,
) -> (usize, usize, Vec<u8>) {
    let (w, h, mut rgba) = raster_frame(view, scene, mode, waterplane, width, height);
    clear_background(w, h, &mut rgba);
    (w, h, rgba)
}

/// Clear the rasterizer's background. A pixel is background when it carries
/// exactly its row's gradient colour (the rasterizer's own formula, one
/// sample per pixel); where the perspective waterplane tinted the background,
/// the tint keeps its own opacity. A hull pixel would have to match the
/// gradient in all three channels to be lost, and then shows the canvas.
fn clear_background(w: usize, h: usize, rgba: &mut [u8]) {
    let (top, bottom) = RASTER_BACKGROUND;
    let (tint, alpha) = RASTER_WATERPLANE_TINT;
    let tint_rgba = [
        tint[0] as u8,
        tint[1] as u8,
        tint[2] as u8,
        (alpha * 255.0).round() as u8,
    ];
    for (y, row) in rgba.chunks_exact_mut(w.max(1) * 4).take(h).enumerate() {
        let t = y as f64 / h.max(1) as f64;
        let bg: [u8; 3] = std::array::from_fn(|k| {
            ((top[k] + (bottom[k] - top[k]) * t).clamp(0.0, 1.0) * 255.0).round() as u8
        });
        let tinted: [u8; 3] =
            std::array::from_fn(|k| (bg[k] as f64 * (1.0 - alpha) + tint[k] * alpha).round() as u8);
        for px in row.as_chunks_mut::<4>().0 {
            if px[..3] == bg {
                px[3] = 0;
            } else if px[..3] == tinted {
                px.copy_from_slice(&tint_rgba);
            }
        }
    }
}

/// Screen-space rectangle of a viewport inside `rect`, split 2×2.
pub fn quad(rect: Rect) -> [(Rect, usize); 4] {
    let (top, bottom) = rect.split_top_bottom_at_fraction(0.5);
    let (tl, tr) = top.split_left_right_at_fraction(0.5);
    let (bl, br) = bottom.split_left_right_at_fraction(0.5);
    [(tl, 0), (tr, 1), (bl, 2), (br, 3)]
}

const BOX_EDGES: [(usize, usize); 12] = [
    (0, 1),
    (1, 3),
    (3, 2),
    (2, 0),
    (4, 5),
    (5, 7),
    (7, 6),
    (6, 4),
    (0, 4),
    (1, 5),
    (2, 6),
    (3, 7),
];

fn box_corners(min: Vec3, max: Vec3) -> [Vec3; 8] {
    [
        Vec3::new(min.x, min.y, min.z),
        Vec3::new(max.x, min.y, min.z),
        Vec3::new(min.x, max.y, min.z),
        Vec3::new(max.x, max.y, min.z),
        Vec3::new(min.x, min.y, max.z),
        Vec3::new(max.x, min.y, max.z),
        Vec3::new(min.x, max.y, max.z),
        Vec3::new(max.x, max.y, max.z),
    ]
}

/// Compute the 2-D convex hull of a slice of points using Andrew's monotone chain algorithm.
pub fn convex_hull_2d(points: &[Pos2]) -> Vec<Pos2> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut pts = points.to_vec();
    pts.sort_by(|a, b| a.x.total_cmp(&b.x).then_with(|| a.y.total_cmp(&b.y)));
    pts.dedup_by(|a, b| (a.x - b.x).abs() < 1e-6 && (a.y - b.y).abs() < 1e-6);
    if pts.len() <= 2 {
        return pts;
    }

    fn cross(o: Pos2, a: Pos2, b: Pos2) -> f32 {
        (a.x - o.x) * (b.y - o.y) - (a.y - o.y) * (b.x - o.x)
    }

    let mut lower = Vec::new();
    for &p in &pts {
        while lower.len() >= 2 && cross(lower[lower.len() - 2], lower[lower.len() - 1], p) <= 0.0 {
            lower.pop();
        }
        lower.push(p);
    }

    let mut upper = Vec::new();
    for &p in pts.iter().rev() {
        while upper.len() >= 2 && cross(upper[upper.len() - 2], upper[upper.len() - 1], p) <= 0.0 {
            upper.pop();
        }
        upper.push(p);
    }

    lower.pop();
    upper.pop();
    lower.extend(upper);
    lower
}

/// Test whether a 2-D point lies inside a polygon using ray casting.
pub fn point_in_polygon(pt: Pos2, poly: &[Pos2]) -> bool {
    if poly.len() < 3 {
        return false;
    }
    let mut inside = false;
    let mut j = poly.len() - 1;
    for i in 0..poly.len() {
        let pi = poly[i];
        let pj = poly[j];
        if ((pi.y > pt.y) != (pj.y > pt.y))
            && (pt.x < (pj.x - pi.x) * (pt.y - pi.y) / (pj.y - pi.y) + pi.x)
        {
            inside = !inside;
        }
        j = i;
    }
    inside
}

/// Projected area of a 2-D polygon using the shoelace formula.
pub fn polygon_area(poly: &[Pos2]) -> f32 {
    if poly.len() < 3 {
        return 0.0;
    }
    let mut a = 0.0f32;
    for i in 0..poly.len() {
        let j = (i + 1) % poly.len();
        a += poly[i].x * poly[j].y - poly[j].x * poly[i].y;
    }
    (a * 0.5).abs()
}

/// Pick the volume under `cursor` with the smallest projected area.
pub fn pick_volume(
    view: &ViewTransform,
    rect: Rect,
    cursor: Pos2,
    volumes: &[VolumeOverlay],
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, vol) in volumes.iter().enumerate() {
        let corners = box_corners(vol.min, vol.max);
        let projected: Vec<Pos2> = corners
            .iter()
            .filter_map(|c| view.project(*c, rect))
            .collect();
        if projected.len() < 3 {
            continue;
        }
        let hull = convex_hull_2d(&projected);
        if hull.len() >= 3 && point_in_polygon(cursor, &hull) {
            let area = polygon_area(&hull);
            if best.as_ref().is_none_or(|(_, best_area)| area < *best_area) {
                best = Some((i, area));
            }
        }
    }
    best.map(|(i, _)| i)
}

fn draw_volumes(
    painter: &egui::Painter,
    view: &ViewTransform,
    rect: Rect,
    volumes: &[VolumeOverlay],
) {
    for vol in volumes {
        let corners = box_corners(vol.min, vol.max);
        let projected: Vec<Option<Pos2>> = corners.iter().map(|c| view.project(*c, rect)).collect();
        let valid_pts: Vec<Pos2> = projected.iter().filter_map(|p| *p).collect();
        if valid_pts.len() < 3 {
            continue;
        }

        let base_fill = match vol.kind {
            VolumeKind::Tank { .. } => theme::TANK_FILL,
            VolumeKind::Compartment { flooded: false } => theme::COMPARTMENT_FILL,
            VolumeKind::Compartment { flooded: true } => theme::FLOODED_FILL,
        };

        let (edge_color, edge_width) = match (&vol.kind, vol.selected) {
            (VolumeKind::Tank { .. }, false) => (theme::TANK_EDGE, 1.0),
            (VolumeKind::Tank { .. }, true) => (theme::TANK_EDGE_SELECTED, 2.0),
            (VolumeKind::Compartment { flooded: false }, false) => (theme::COMPARTMENT_EDGE, 1.0),
            (VolumeKind::Compartment { flooded: false }, true) => {
                (theme::COMPARTMENT_EDGE_SELECTED, 2.0)
            }
            (VolumeKind::Compartment { flooded: true }, false) => (theme::FLOODED_EDGE, 1.0),
            (VolumeKind::Compartment { flooded: true }, true) => {
                (theme::FLOODED_EDGE_SELECTED, 2.0)
            }
        };
        let stroke = Stroke::new(edge_width, edge_color);

        if vol.clip_to_hull {
            // Draw filled clipped solid and liquid fill if available
            if let Some((verts, tris)) = &vol.clipped_mesh {
                // One side of the closed solid only: front and back faces
                // both drawn stack the translucent fill until it is opaque.
                let facing = |a: Pos2, b: Pos2, c: Pos2| {
                    let (u, v) = (b - a, c - a);
                    u.x * v.y - u.y * v.x > 0.0
                };
                // The solid is a stack of slabs along x: the caps between
                // neighbouring slabs are inside it and would stack the fill
                // once per slab in the Body plan. Keep the outer surface.
                let (x_lo, x_hi) = verts
                    .iter()
                    .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), v| {
                        (a.min(v.x), b.max(v.x))
                    });
                let tol = 1e-9 * (x_hi - x_lo).abs().max(1.0);
                let inner_cap = |t: &[usize; 3]| {
                    let x = verts[t[0]].x;
                    (verts[t[1]].x - x).abs() < tol
                        && (verts[t[2]].x - x).abs() < tol
                        && x > x_lo + tol
                        && x < x_hi - tol
                };
                let proj_verts: Vec<Option<Pos2>> =
                    verts.iter().map(|v| view.project(*v, rect)).collect();
                let mut mesh = egui::Mesh::default();
                for t in tris {
                    if let (Some(p0), Some(p1), Some(p2)) =
                        (proj_verts[t[0]], proj_verts[t[1]], proj_verts[t[2]])
                    {
                        if inner_cap(t) || !facing(p0, p1, p2) {
                            continue;
                        }
                        let idx = mesh.vertices.len() as u32;
                        mesh.colored_vertex(p0, base_fill);
                        mesh.colored_vertex(p1, base_fill);
                        mesh.colored_vertex(p2, base_fill);
                        mesh.add_triangle(idx, idx + 1, idx + 2);
                    }
                }
                if !mesh.is_empty() {
                    painter.add(egui::Shape::mesh(mesh));
                }

                if let VolumeKind::Tank { liquid_z: Some(lz) } = vol.kind {
                    let top_z = lz.clamp(vol.min.z, vol.max.z);
                    if top_z > vol.min.z + 1e-6 {
                        let liquid_blue = theme::TANK_LIQUID;
                        let mut liq_mesh = egui::Mesh::default();
                        for t in tris {
                            let z_avg = (verts[t[0]].z + verts[t[1]].z + verts[t[2]].z) / 3.0;
                            if z_avg <= top_z {
                                if let (Some(p0), Some(p1), Some(p2)) =
                                    (proj_verts[t[0]], proj_verts[t[1]], proj_verts[t[2]])
                                {
                                    if inner_cap(t) || !facing(p0, p1, p2) {
                                        continue;
                                    }
                                    let idx = liq_mesh.vertices.len() as u32;
                                    liq_mesh.colored_vertex(p0, liquid_blue);
                                    liq_mesh.colored_vertex(p1, liquid_blue);
                                    liq_mesh.colored_vertex(p2, liquid_blue);
                                    liq_mesh.add_triangle(idx, idx + 1, idx + 2);
                                }
                            }
                        }
                        if !liq_mesh.is_empty() {
                            painter.add(egui::Shape::mesh(liq_mesh));
                        }
                    }
                }
            }

            // Outside part of bounding box drawn as dashed outline
            for &(i, j) in &BOX_EDGES {
                if let (Some(p), Some(q)) = (projected[i], projected[j]) {
                    painter.extend(egui::Shape::dashed_line(&[p, q], stroke, 6.0, 4.0));
                }
            }
        } else {
            // Unclipped volume: translucent fill of projected convex hull
            let hull = convex_hull_2d(&valid_pts);
            if hull.len() >= 3 {
                painter.add(egui::Shape::convex_polygon(hull, base_fill, Stroke::NONE));
            }

            // Translucent fill for liquid level in tanks
            if let VolumeKind::Tank { liquid_z: Some(lz) } = vol.kind {
                let top_z = lz.clamp(vol.min.z, vol.max.z);
                if top_z > vol.min.z + 1e-6 {
                    let sub_corners = box_corners(vol.min, Vec3::new(vol.max.x, vol.max.y, top_z));
                    let sub_valid: Vec<Pos2> = sub_corners
                        .iter()
                        .filter_map(|c| view.project(*c, rect))
                        .collect();
                    if sub_valid.len() >= 3 {
                        let sub_hull = convex_hull_2d(&sub_valid);
                        if sub_hull.len() >= 3 {
                            let liquid_blue = theme::TANK_LIQUID;
                            painter.add(egui::Shape::convex_polygon(
                                sub_hull,
                                liquid_blue,
                                Stroke::NONE,
                            ));
                        }
                    }
                }
            }

            // 12 solid box edges
            for &(i, j) in &BOX_EDGES {
                if let (Some(p), Some(q)) = (projected[i], projected[j]) {
                    painter.line_segment([p, q], stroke);
                }
            }
        }

        // 4. Show name next to selected volume only
        if vol.selected {
            let mut label_pos = valid_pts[0];
            for &p in &valid_pts[1..] {
                if p.y < label_pos.y {
                    label_pos = p;
                }
            }
            painter.text(
                label_pos + egui::vec2(4.0, -4.0),
                Align2::LEFT_BOTTOM,
                &vol.name,
                FontId::proportional(12.0),
                edge_color,
            );
        }
    }
}

/// Pick the windage item under `cursor` with the smallest projected area.
pub fn pick_windage(
    view: &ViewTransform,
    rect: Rect,
    cursor: Pos2,
    windage: &[WindageOverlay],
) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, item) in windage.iter().enumerate() {
        let p0 = Vec3::new(item.rect.x_min, 0.0, item.rect.z_min);
        let p1 = Vec3::new(item.rect.x_max, 0.0, item.rect.z_min);
        let p2 = Vec3::new(item.rect.x_max, 0.0, item.rect.z_max);
        let p3 = Vec3::new(item.rect.x_min, 0.0, item.rect.z_max);

        if let (Some(q0), Some(q1), Some(q2), Some(q3)) = (
            view.project(p0, rect),
            view.project(p1, rect),
            view.project(p2, rect),
            view.project(p3, rect),
        ) {
            let hull = convex_hull_2d(&[q0, q1, q2, q3]);
            if hull.len() >= 3 && point_in_polygon(cursor, &hull) {
                let area = polygon_area(&hull);
                if best.as_ref().is_none_or(|(_, best_area)| area < *best_area) {
                    best = Some((i, area));
                }
            }
        }
    }
    best.map(|(i, _)| i)
}

fn draw_windage(
    painter: &egui::Painter,
    view: &ViewTransform,
    rect: Rect,
    windage: &[WindageOverlay],
) {
    for item in windage {
        let p0 = Vec3::new(item.rect.x_min, 0.0, item.rect.z_min);
        let p1 = Vec3::new(item.rect.x_max, 0.0, item.rect.z_min);
        let p2 = Vec3::new(item.rect.x_max, 0.0, item.rect.z_max);
        let p3 = Vec3::new(item.rect.x_min, 0.0, item.rect.z_max);

        if let (Some(q0), Some(q1), Some(q2), Some(q3)) = (
            view.project(p0, rect),
            view.project(p1, rect),
            view.project(p2, rect),
            view.project(p3, rect),
        ) {
            let hull = convex_hull_2d(&[q0, q1, q2, q3]);
            if hull.len() >= 3 {
                let (fill, stroke) = if item.selected {
                    (
                        theme::WINDAGE_FILL_SELECTED,
                        Stroke::new(2.0, theme::WINDAGE_EDGE_SELECTED),
                    )
                } else {
                    (theme::WINDAGE_FILL, Stroke::new(1.0, theme::WINDAGE_EDGE))
                };
                painter.add(egui::Shape::convex_polygon(hull, fill, stroke));

                let min_x = q0.x.min(q1.x).min(q2.x).min(q3.x);
                let max_x = q0.x.max(q1.x).max(q2.x).max(q3.x);
                let min_y = q0.y.min(q1.y).min(q2.y).min(q3.y);
                let max_y = q0.y.max(q1.y).max(q2.y).max(q3.y);
                let w_px = max_x - min_x;
                let h_px = max_y - min_y;

                if w_px >= 30.0 && h_px >= 14.0 && !item.name.is_empty() {
                    let center = Pos2::new((min_x + max_x) * 0.5, (min_y + max_y) * 0.5);
                    let text_color = if item.selected {
                        theme::WINDAGE_TEXT_SELECTED
                    } else {
                        theme::WINDAGE_TEXT
                    };
                    painter.text(
                        center,
                        Align2::CENTER_CENTER,
                        &item.name,
                        FontId::proportional(11.0),
                        text_color,
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, ViewKind};
    use seacraft_core::math::Vec3;

    #[test]
    fn grid_lines_are_world_anchored_with_one_datum() {
        let lines = grid_lines(3.3, 10.0, 1.0);
        assert!(lines.first().unwrap().0 <= 3.3 - 10.0);
        assert!(lines.last().unwrap().0 >= 3.3 + 10.0);
        for (c, _) in &lines {
            assert!((c - c.round()).abs() < 1e-12, "{c} is not on the 1 m grid");
        }
        let datums: Vec<_> = lines.iter().filter(|l| l.1).collect();
        assert_eq!(datums.len(), 1);
        assert_eq!(datums[0].0, 0.0);
        // Far from the origin there is no datum to draw.
        assert!(grid_lines(100.0, 5.0, 1.0).iter().all(|l| !l.1));
    }

    #[test]
    fn grid_captions_name_the_steps_in_metres() {
        assert_eq!(format_length(0.1), "0.1 m");
        assert_eq!(format_length(0.5), "0.5 m");
        assert_eq!(format_length(1.0), "1 m");
        assert_eq!(format_length(10.0), "10 m");
        assert_eq!(format_length(100.0), "100 m");
        assert_eq!(format_length(5000.0), "5 km");
        assert_eq!(
            grid_caption(false, true, 1.0, 5.0).as_deref(),
            Some("grid 1 m · major 5 m")
        );
        assert_eq!(
            grid_caption(true, true, 5.0, 10.0).as_deref(),
            Some("ground grid 5 m · major 10 m")
        );
        assert_eq!(grid_caption(false, false, 1.0, 5.0), None);
    }

    #[test]
    fn the_scale_bar_is_one_minor_step_long() {
        let rect = Rect::from_min_size(Pos2::new(100.0, 50.0), egui::vec2(600.0, 400.0));
        for world_per_px in [0.002, 0.005, 0.03, 0.07, 0.2, 0.5] {
            let (minor, _) = grid_steps(1.0 / world_per_px);
            let bar = scale_bar(rect, world_per_px, minor);
            let want = (minor / world_per_px) as f32;
            assert!(
                ((bar.b.x - bar.a.x) - want).abs() <= 1.0,
                "{world_per_px}: {} px vs {want}",
                bar.b.x - bar.a.x
            );
            assert_eq!(bar.a.y, bar.b.y, "horizontal");
            assert_eq!(bar.label, format_length(minor));
            // Bottom-left corner, inside the view.
            assert!(rect.contains(bar.a) && rect.contains(bar.b));
            assert!(bar.a.x - rect.left() < 20.0 && rect.bottom() - bar.a.y < 20.0);
        }
    }

    #[test]
    fn grid_steps_keep_minor_lines_apart_and_majors_every_5_m() {
        // The usual zoom (~0.03 m/px): 1 m minors, 5 m majors.
        assert_eq!(grid_steps(1.0 / 0.03), (1.0, 5.0));
        // Zoomed out, the minors coarsen to 5 m (10 m majors), then 10 m.
        assert_eq!(grid_steps(1.0 / 0.2), (5.0, 10.0));
        assert_eq!(grid_steps(1.0 / 0.5), (10.0, 50.0));
        // Zoomed in on a detail, half-metre and then 0.1 m minors.
        assert_eq!(grid_steps(1.0 / 0.005), (0.5, 1.0));
        assert_eq!(grid_steps(1.0 / 0.002), (0.1, 0.5));
        for px_per_m in [0.01, 0.3, 2.0, 11.9, 12.0, 40.0, 250.0, 5000.0] {
            let (minor, major) = grid_steps(px_per_m);
            assert!(major > minor, "{px_per_m}: {minor} / {major}");
            if px_per_m > 0.01 {
                assert!(
                    minor * px_per_m >= GRID_MIN_SPACING_PX,
                    "{px_per_m} px/m: {minor} m minors are {} px apart",
                    minor * px_per_m
                );
            }
        }
    }

    /// Screen positions of the minor and major lines of an orthographic
    /// grid, across (vertical lines) and up (horizontal lines).
    fn grid_positions(lines: &[GridLine]) -> (Vec<f32>, Vec<f32>) {
        let mut xs = Vec::new();
        let mut ys = Vec::new();
        for l in lines.iter().filter(|l| l.tier != GridTier::Datum) {
            if (l.a.x - l.b.x).abs() < 0.01 {
                xs.push(l.a.x);
            } else {
                ys.push(l.a.y);
            }
        }
        xs.sort_by(f32::total_cmp);
        ys.sort_by(f32::total_cmp);
        (xs, ys)
    }

    #[test]
    fn no_two_minor_grid_lines_sit_closer_than_12_px_at_any_zoom() {
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(486.0, 452.0));
        let (min, max) = (Vec3::new(0.0, -1.8, 0.0), Vec3::new(12.0, 1.8, 1.7));
        for kind in [ViewKind::Profile, ViewKind::Plan, ViewKind::Body] {
            for height in [0.3, 2.0, 7.5, 14.0, 40.0, 130.0, 900.0] {
                let camera = Camera::ortho(kind, (min + max) * 0.5, height);
                let lines = grid_segments(&camera.transform(), rect, min, max, Some(0.62));
                let (xs, ys) = grid_positions(&lines);
                assert!(xs.len() >= 2 && ys.len() >= 2, "{kind:?} at {height} m");
                for w in xs.windows(2).chain(ys.windows(2)) {
                    assert!(
                        w[1] - w[0] >= GRID_MIN_SPACING_PX as f32 - 0.01,
                        "{kind:?} at {height} m: lines {} px apart",
                        w[1] - w[0]
                    );
                }
                // The grid covers the cell, so a panned view still has one.
                assert!(xs[0] <= rect.left() + 1.0 && *xs.last().unwrap() >= rect.right() - 1.0);
            }
        }
        // Body plan datums: centreline, baseline and the waterline.
        let camera = Camera::ortho(ViewKind::Body, (min + max) * 0.5, 4.0);
        let view = camera.transform();
        let datums: Vec<_> = grid_segments(&view, rect, min, max, Some(0.62))
            .into_iter()
            .filter(|l| l.tier == GridTier::Datum)
            .collect();
        assert_eq!(datums.len(), 3, "{datums:?}");
        let dwl = view.project(Vec3::new(0.0, 0.0, 0.62), rect).unwrap().y;
        assert!(datums
            .iter()
            .any(|l| (l.a.y - dwl).abs() < 0.5 && (l.b.y - dwl).abs() < 0.5));
    }

    #[test]
    fn the_perspective_grid_lies_on_the_baseline_plane() {
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(486.0, 452.0));
        let (min, max) = (Vec3::new(0.0, -1.8, 0.0), Vec3::new(12.0, 1.8, 1.7));
        let mut camera = Camera::new(ViewKind::Perspective, (min + max) * 0.5, max - min);
        camera.frame(min, max);
        let view = camera.transform();
        let lines = grid_segments(&view, rect, min, max, Some(0.62));
        assert!(lines.len() > 10, "{} lines", lines.len());
        // Every end point is the projection of a point on z = 0: cast the
        // ray back and it meets the plane where the line was drawn from.
        for l in &lines {
            for p in [l.a, l.b] {
                let ray = view.ray(p, rect);
                assert!(
                    ray.dir.z.abs() > 1e-6,
                    "the line is not on a plane under the eye"
                );
                let t = -ray.origin.z / ray.dir.z;
                assert!(t > 0.0, "z = 0 is behind the camera at {p:?}");
                let hit = ray.point_at(t);
                assert!(
                    hit.x > min.x - 7.0 && hit.x < max.x + 7.0 && hit.y.abs() < 9.0,
                    "{hit:?} is not around the model"
                );
            }
        }
        // The centreline and midship are datums; the rest are 1 m / 5 m.
        let datums = lines.iter().filter(|l| l.tier == GridTier::Datum).count();
        assert!((2..=3).contains(&datums), "{datums} datums");
        assert!(lines.iter().any(|l| l.tier == GridTier::Major));
        // Weakest first, so a datum is never painted over.
        assert!(lines.windows(2).all(|w| w[0].tier <= w[1].tier));
    }

    #[test]
    fn major_grid_lines_are_stronger_and_captions_sit_on_a_backing() {
        let s = seacraft_core::demo::demo_hull(false);
        let mesh = export::render_mesh(&s, 1, Default::default());
        let (min, max) = s.extents();
        let scene = Scene {
            surface: &s,
            mesh: &mesh,
            curves: None,
            volumes: &[],
            windage: &[],
            min,
            max,
            draft: 0.62,
            trim: 0.0,
            heel_deg: 0.0,
        };
        let opts = DisplayOptions {
            shaded: false,
            control_net: false,
            points: false,
            waterplane: false,
            ..Default::default()
        };
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(600.0, 400.0));
            let camera = Camera::new(ViewKind::Profile, (min + max) * 0.5, max - min);
            viewport(ui, rect, &camera, Some(&scene), &opts, true, None);
        });
        out.textures_delta.clear();
        let mut minor = 0;
        let mut major = 0;
        let mut datum = 0;
        let mut backings = Vec::new();
        let mut captions = Vec::new();
        fn walk(shape: &egui::Shape, f: &mut dyn FnMut(&egui::Shape)) {
            if let egui::Shape::Vec(v) = shape {
                v.iter().for_each(|s| walk(s, f));
            } else {
                f(shape);
            }
        }
        for clipped in &out.shapes {
            walk(&clipped.shape, &mut |shape| match shape {
                egui::Shape::LineSegment { stroke, .. } => {
                    if stroke.color == theme::GRID_MINOR {
                        minor += 1;
                        assert!(stroke.width <= 1.0);
                    } else if stroke.color == theme::GRID_MAJOR {
                        major += 1;
                        assert!(stroke.width <= 1.0);
                    } else if stroke.color == theme::GRID_DATUM {
                        datum += 1;
                        assert!(stroke.width > 1.0);
                    }
                }
                egui::Shape::Rect(r) if r.fill == theme::HUD_BACKING => backings.push(r.rect),
                egui::Shape::Text(t) => captions.push((t.galley.text().to_string(), t.pos)),
                _ => {}
            });
        }
        assert!(minor > 4, "{minor} minor lines");
        // 5 m majors: at least x = 5 and x = 10 along the 12 m hull.
        assert!(major >= 2, "{major} major lines");
        // Profile: baseline z = 0, x = 0, midship and the design waterline.
        assert_eq!(datum, 4, "datum lines in the profile view");
        // R6-1: the m/px caption became the grid step, and the scale bar
        // brought a third backing.
        assert_eq!(
            backings.len(),
            3,
            "view name, grid caption and scale bar each get a backing"
        );
        for (text, pos) in &captions {
            assert!(
                backings.iter().any(|b| b.contains(*pos)),
                "caption {text:?} has no backing"
            );
        }
        assert!(captions.iter().any(|(t, _)| t == "Profile"));
        assert!(
            captions.iter().any(|(t, _)| t.starts_with("grid ")),
            "{captions:?}"
        );
    }

    #[test]
    fn an_analysis_image_is_see_through_around_the_hull_only() {
        let s = seacraft_core::demo::demo_hull(false);
        let mesh = export::render_mesh(&s, 1, Default::default());
        let (min, max) = s.extents();
        let scene = Scene {
            surface: &s,
            mesh: &mesh,
            curves: None,
            volumes: &[],
            windage: &[],
            min,
            max,
            draft: 0.62,
            trim: 0.0,
            heel_deg: 0.0,
        };
        let zebra = AnalysisMode::Zebra { stripes: 14.0 };
        let alpha = |rgba: &[u8], w: usize, x: usize, y: usize| rgba[(y * w + x) * 4 + 3];

        let mut camera = Camera::new(ViewKind::Profile, (min + max) * 0.5, max - min);
        camera.frame(min, max);
        let (w, h, rgba) = analysis_image(&camera.transform(), &scene, zebra, false, 160, 100);
        let (_, _, opaque) = raster_frame(&camera.transform(), &scene, zebra, false, 160, 100);
        assert_eq!(alpha(&rgba, w, 1, 1), 0, "the background is cleared");
        assert_eq!(alpha(&rgba, w, w - 2, h - 2), 0);
        assert_eq!(alpha(&rgba, w, w / 2, h / 2), 255, "the hull is opaque");
        // The colours are the rasterizer's own wherever a pixel stays opaque.
        for (a, b) in rgba
            .as_chunks::<4>()
            .0
            .iter()
            .zip(opaque.as_chunks::<4>().0)
        {
            if a[3] == 255 {
                assert_eq!(a, b);
            }
        }

        // In perspective the waterplane around the hull keeps its tint, at
        // its own opacity, over the grid.
        let mut camera = Camera::new(ViewKind::Perspective, (min + max) * 0.5, max - min);
        camera.frame(min, max);
        let (_, _, rgba) = analysis_image(&camera.transform(), &scene, zebra, true, 160, 100);
        let tinted = rgba
            .as_chunks::<4>()
            .0
            .iter()
            .filter(|p| p[3] == 56)
            .count();
        assert!(
            tinted > 100,
            "{tinted} waterplane pixels over the background"
        );
    }

    #[test]
    fn convex_hull_2d_square_and_interior_points() {
        let pts = vec![
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
            Pos2::new(0.0, 10.0),
            Pos2::new(5.0, 5.0),
            Pos2::new(2.0, 3.0),
        ];
        let hull = convex_hull_2d(&pts);
        assert_eq!(hull.len(), 4);
        for p in [
            Pos2::new(0.0, 0.0),
            Pos2::new(10.0, 0.0),
            Pos2::new(10.0, 10.0),
            Pos2::new(0.0, 10.0),
        ] {
            assert!(hull.contains(&p));
        }
    }

    #[test]
    fn pick_volume_nested_boxes_and_outside() {
        let camera = Camera::ortho(ViewKind::Plan, Vec3::ZERO, 30.0);
        let view = camera.transform();
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(200.0, 200.0));

        let outer = VolumeOverlay {
            name: "Outer".into(),
            min: Vec3::new(-10.0, -10.0, -5.0),
            max: Vec3::new(10.0, 10.0, 5.0),
            kind: VolumeKind::Compartment { flooded: false },
            selected: false,
            clip_to_hull: false,
            clipped_mesh: None,
        };
        let inner = VolumeOverlay {
            name: "Inner".into(),
            min: Vec3::new(-2.0, -2.0, -2.0),
            max: Vec3::new(2.0, 2.0, 2.0),
            kind: VolumeKind::Tank { liquid_z: None },
            selected: false,
            clip_to_hull: false,
            clipped_mesh: None,
        };
        let volumes = vec![outer, inner];

        // Cursor at the projected center (0, 0, 0)
        let center_screen = view.project(Vec3::ZERO, rect).expect("project center");
        let hit = pick_volume(&view, rect, center_screen, &volumes);
        assert_eq!(hit, Some(1), "the inner box with smaller area is picked");

        // Cursor outside both
        let outside = Pos2::new(5.0, 5.0);
        let miss = pick_volume(&view, rect, outside, &volumes);
        assert_eq!(miss, None, "a cursor outside both yields None");
    }

    #[test]
    fn pick_windage_hits_smallest_enclosing_rect_and_misses_outside() {
        let camera = Camera::ortho(ViewKind::Profile, Vec3::ZERO, 30.0);
        let view = camera.transform();
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), egui::vec2(200.0, 200.0));

        let outer = WindageOverlay {
            name: "Outer".into(),
            rect: WindageRect {
                x_min: -10.0,
                x_max: 10.0,
                z_min: -5.0,
                z_max: 5.0,
            },
            selected: false,
        };
        let inner = WindageOverlay {
            name: "Inner".into(),
            rect: WindageRect {
                x_min: -2.0,
                x_max: 2.0,
                z_min: -2.0,
                z_max: 2.0,
            },
            selected: false,
        };
        let windage = vec![outer, inner];

        let center_screen = view.project(Vec3::ZERO, rect).expect("project center");
        let hit = pick_windage(&view, rect, center_screen, &windage);
        assert_eq!(hit, Some(1), "inner rect with smaller area is picked");

        let outside = Pos2::new(5.0, 5.0);
        let miss = pick_windage(&view, rect, outside, &windage);
        assert_eq!(miss, None, "outside yields None");
    }
}
