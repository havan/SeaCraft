//! Viewport drawing.
//!
//! Everything is CPU-projected into an [`egui::Painter`]: there is no GPU
//! pipeline and the crate stays testable on a plain `egui::Context`. Each
//! viewport draws, in order, a background, a 1 m grid, the refined hull, the
//! control net, the control points, the waterplane and a caption.

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
    /// Draw station/buttock/waterline section curves and curvature combs.
    pub combs: bool,
    /// Shading mode. Anything but `Shaded` renders the viewport through the
    /// CPU rasterizer, which is the only way to get per-pixel zebra stripes
    /// and curvature maps.
    pub analysis: AnalysisMode,
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            shaded: true,
            wireframe: false,
            control_net: true,
            points: true,
            waterplane: true,
            volumes: true,
            windage: true,
            grid: true,
            combs: false,
            analysis: AnalysisMode::Shaded,
        }
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
    viewport_with_texture(ui, rect, camera, scene, opts, active, selected, None)
}

/// Draw a viewport, optionally using `texture` (a rasterizer frame) in place
/// of the painter's shaded hull.
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
        draw_grid(&painter, &view, rect, &scene.center());
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
                draw_hull(&painter, &view, rect, scene, opts);
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
    if opts.combs {
        if let Some(curves) = scene.curves {
            draw_combs(&painter, &view, camera.kind, rect, curves);
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
    let world_per_px = view.world_per_px(rect, camera.distance);
    hud_caption(
        &painter,
        rect.right_top() + egui::vec2(-8.0, 6.0),
        Align2::RIGHT_TOP,
        format!("{world_per_px:.3} m/px"),
    );
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

/// A 1 m grid in the plane through the camera target, spaced adaptively so it
/// never becomes a solid wall of lines. In the orthographic views the datum
/// lines (centreline, baseline, x = 0) and midship are drawn stronger.
fn draw_grid(painter: &egui::Painter, view: &ViewTransform, rect: Rect, center: &Vec3) {
    // World height visible at the target plane: the orthographic height, or
    // the perspective frustum's height at the distance of the target.
    let (w, h) = (rect.width() as f64, rect.height() as f64);
    let height = if view.perspective {
        let depth = (*center - view.eye).dot(view.forward).max(1e-3);
        2.0 * view.visible_fov_tan_wh(w, h) * depth
    } else {
        view.visible_height_wh(w, h)
    };
    let width = height * (rect.width().max(1.0) / rect.height().max(1.0)) as f64;
    let half = (height.max(width) * 0.75).max(1.0);

    let base = (*center).dot(view.right);
    let up_base = (*center).dot(view.up);
    let step = [
        0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 500.0,
    ]
    .into_iter()
    .find(|s| 2.0 * half / s <= 30.0)
    .unwrap_or(1000.0);

    // Minor lines first, datum lines on top of them.
    let mut lines: Vec<([Vec3; 2], bool)> = Vec::new();
    let datum_right = is_world_axis(view.right);
    let datum_up = is_world_axis(view.up);
    for (u, zero) in grid_lines(base, half, step) {
        lines.push((
            [
                view.right * u + view.up * (up_base - half),
                view.right * u + view.up * (up_base + half),
            ],
            zero && datum_right,
        ));
    }
    for (v, zero) in grid_lines(up_base, half, step) {
        lines.push((
            [
                view.right * (base - half) + view.up * v,
                view.right * (base + half) + view.up * v,
            ],
            zero && datum_up,
        ));
    }
    // Midship: the middle of the model's length, where a view runs along x.
    if datum_right && view.right.x.abs() > 0.999 {
        lines.push((
            [
                view.right * base + view.up * (up_base - half),
                view.right * base + view.up * (up_base + half),
            ],
            true,
        ));
    }
    if datum_up && view.up.x.abs() > 0.999 {
        lines.push((
            [
                view.right * (base - half) + view.up * up_base,
                view.right * (base + half) + view.up * up_base,
            ],
            true,
        ));
    }
    lines.sort_by_key(|(_, major)| *major);
    for ([a, b], major) in lines {
        if let (Some(p), Some(q)) = (view.project(a, rect), view.project(b, rect)) {
            let stroke = if major {
                Stroke::new(1.2, theme::GRID_MAJOR)
            } else {
                Stroke::new(1.0, theme::GRID_MINOR)
            };
            painter.line_segment([p, q], stroke);
        }
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

fn draw_combs(
    painter: &egui::Painter,
    view: &ViewTransform,
    kind: crate::camera::ViewKind,
    rect: Rect,
    curves: &seacraft_core::linesplan::LinesplanCurves3d,
) {
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
            for w in curve.points.windows(2) {
                if let (Some(p), Some(q)) = (view.project(w[0], rect), view.project(w[1], rect)) {
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
            let mut tips: Vec<Pos2> = Vec::with_capacity(curve.spikes.len());
            for &(base, tip) in &curve.spikes {
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

fn draw_hull(
    painter: &egui::Painter,
    view: &ViewTransform,
    rect: Rect,
    scene: &Scene,
    opts: &DisplayOptions,
) {
    let mesh = scene.mesh;

    // ---- flat-shaded faces ------------------------------------------------
    if opts.shaded && !mesh.faces.is_empty() {
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
        let mut tri_mesh = egui::Mesh::default();
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
        if !tri_mesh.is_empty() {
            painter.add(egui::Shape::mesh(tri_mesh));
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

/// A hash of everything a rasterized frame depends on, so the texture is only
/// rebuilt when the model, the camera, the viewport size or the mode changes.
pub fn frame_key(
    revision: u64,
    camera: &Camera,
    rect: Rect,
    scene: &Scene,
    opts: &DisplayOptions,
) -> u64 {
    fn mix(h: &mut u64, x: f64) {
        *h ^= x.to_bits();
        *h = h.wrapping_mul(0x0100_0000_01b3);
    }
    let view = camera.transform();
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    mix(&mut h, revision as f64);
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
        background: (
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
        ),
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

        // 1. Translucent fill of the projected 2-D convex hull of the 8 corners (alpha ~25)
        let hull = convex_hull_2d(&valid_pts);
        if hull.len() >= 3 {
            let base_fill = match vol.kind {
                VolumeKind::Tank { .. } => theme::TANK_FILL,
                VolumeKind::Compartment { flooded: false } => theme::COMPARTMENT_FILL,
                VolumeKind::Compartment { flooded: true } => theme::FLOODED_FILL,
            };
            painter.add(egui::Shape::convex_polygon(hull, base_fill, Stroke::NONE));
        }

        // 2. Translucent fill for liquid level in tanks (alpha ~70, liquid blue)
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

        // 3. 12 box edges
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
        for &(i, j) in &BOX_EDGES {
            if let (Some(p), Some(q)) = (projected[i], projected[j]) {
                painter.line_segment([p, q], stroke);
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
                        assert!(stroke.width > 1.0);
                    }
                }
                egui::Shape::Rect(r) if r.fill == theme::HUD_BACKING => backings.push(r.rect),
                egui::Shape::Text(t) => captions.push((t.galley.text().to_string(), t.pos)),
                _ => {}
            });
        }
        assert!(minor > 4, "{minor} minor lines");
        // Profile: baseline z = 0, x = 0 and midship.
        assert_eq!(major, 3, "datum lines in the profile view");
        assert_eq!(backings.len(), 2, "view name and scale each get a backing");
        for (text, pos) in &captions {
            assert!(
                backings.iter().any(|b| b.contains(*pos)),
                "caption {text:?} has no backing"
            );
        }
        assert!(captions.iter().any(|(t, _)| t == "Profile"));
        assert!(captions.iter().any(|(t, _)| t.ends_with("m/px")));
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
        };
        let inner = VolumeOverlay {
            name: "Inner".into(),
            min: Vec3::new(-2.0, -2.0, -2.0),
            max: Vec3::new(2.0, 2.0, 2.0),
            kind: VolumeKind::Tank { liquid_z: None },
            selected: false,
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
