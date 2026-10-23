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

use crate::camera::{Camera, ViewTransform};

// ---- palette -----------------------------------------------------------

pub const BG: Color32 = Color32::from_rgb(0x13, 0x19, 0x21);
pub const FRAME: Color32 = Color32::from_rgb(0x2a, 0x35, 0x42);
pub const FRAME_ACTIVE: Color32 = Color32::from_rgb(0x54, 0x8c, 0xc8);
pub const GRID: Color32 = Color32::from_rgb(0x21, 0x2b, 0x36);
pub const GRID_AXIS: Color32 = Color32::from_rgb(0x33, 0x42, 0x52);
pub const HULL_EDGE: Color32 = Color32::from_rgb(0x7f, 0x9a, 0xb5);
pub const CONTROL: Color32 = Color32::from_rgb(0xe8, 0x9d, 0x3a);
pub const CREASE: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
pub const POINT: Color32 = Color32::from_rgb(0xcf, 0xd8, 0xe3);
pub const SELECTED: Color32 = Color32::from_rgb(0x5a, 0xd0, 0x6a);
pub const WATERPLANE: Color32 = Color32::from_rgba_premultiplied(0x07, 0x10, 0x1b, 0x24);
pub const WATERLINE: Color32 = Color32::from_rgb(0x5c, 0xa8, 0xf0);
pub const SECTION_CURVE: Color32 = Color32::from_rgb(0x9a, 0xb8, 0xd4);
pub const COMB_SPIKE: Color32 = Color32::from_rgb(0x4b, 0x9c, 0x62);
pub const COMB_ENV: Color32 = Color32::from_rgb(0x63, 0xd4, 0x82);
pub const TEXT: Color32 = Color32::from_rgb(0x8e, 0x9e, 0xae);

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
            grid: true,
            combs: false,
            analysis: AnalysisMode::Shaded,
        }
    }
}

/// Everything a viewport draws.
pub struct Scene<'a> {
    pub surface: &'a Surface,
    /// The refined hull, already mirrored and filtered to visible layers.
    pub mesh: &'a RenderMesh,
    /// Optional 3D stations/buttocks/waterlines and their curvature combs.
    pub curves: Option<&'a seacraft_core::linesplan::LinesplanCurves3d>,
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
    painter.rect_filled(rect, 0.0, BG);
    let Some(scene) = scene else {
        painter.text(
            rect.center(),
            Align2::CENTER_CENTER,
            "No model loaded",
            FontId::proportional(14.0),
            TEXT,
        );
        painter.rect_stroke(rect, 0.0, Stroke::new(1.0, FRAME), egui::StrokeKind::Inside);
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

    // Caption and scale.
    painter.text(
        rect.left_top() + egui::vec2(8.0, 6.0),
        Align2::LEFT_TOP,
        camera.kind.label(),
        FontId::proportional(12.0),
        TEXT,
    );
    let world_per_px = if view.perspective {
        2.0 * camera.distance * view.fov_tan / rect.height().max(1.0) as f64
    } else {
        view.ortho_height / rect.height().max(1.0) as f64
    };
    painter.text(
        rect.right_top() + egui::vec2(-8.0, 6.0),
        Align2::RIGHT_TOP,
        format!("{world_per_px:.3} m/px"),
        FontId::monospace(11.0),
        TEXT,
    );
    painter.rect_stroke(
        rect,
        0.0,
        Stroke::new(
            if active { 2.0 } else { 1.0 },
            if active { FRAME_ACTIVE } else { FRAME },
        ),
        egui::StrokeKind::Inside,
    );
}

/// A 1 m grid in the plane through the camera target, spaced adaptively so it
/// never becomes a solid wall of lines.
fn draw_grid(painter: &egui::Painter, view: &ViewTransform, rect: Rect, center: &Vec3) {
    // World height visible at the target plane: the orthographic height, or
    // the perspective frustum's height at the distance of the target.
    let height = if view.perspective {
        let depth = (*center - view.eye).dot(view.forward).max(1e-3);
        2.0 * view.fov_tan * depth
    } else {
        view.ortho_height
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

    let paint = |a: Vec3, b: Vec3, color: Color32| {
        if let (Some(p), Some(q)) = (view.project(a, rect), view.project(b, rect)) {
            painter.line_segment([p, q], Stroke::new(1.0, color));
        }
    };

    let n = (half / step).ceil() as i64;
    for i in -n..=n {
        let offset = i as f64 * step;
        let u = base + offset;
        let v = up_base;
        let color = if offset.abs() < 1e-9 { GRID_AXIS } else { GRID };
        paint(
            view.right * u + view.up * (v - half),
            view.right * u + view.up * (v + half),
            color,
        );
        let v2 = up_base + offset;
        paint(
            view.right * (base - half) + view.up * v2,
            view.right * (base + half) + view.up * v2,
            color,
        );
    }
    let _ = width;
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
                quad_mesh.colored_vertex(p, WATERPLANE);
            }
            quad_mesh.add_triangle(0, 1, 2);
            quad_mesh.add_triangle(0, 2, 3);
            painter.add(egui::Shape::mesh(quad_mesh));
        }
        let border = Color32::from_rgba_unmultiplied(0x5c, 0xa8, 0xf0, 0x80);
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
                    painter.line_segment([p0, p1], Stroke::new(1.5, WATERLINE));
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
                    painter.line_segment([p, q], Stroke::new(1.0, SECTION_CURVE));
                }
            }
            if curve.closed && curve.points.len() >= 3 {
                if let (Some(p), Some(q)) = (
                    view.project(*curve.points.last().unwrap(), rect),
                    view.project(curve.points[0], rect),
                ) {
                    painter.line_segment([p, q], Stroke::new(1.0, SECTION_CURVE));
                }
            }
            let mut tips: Vec<Pos2> = Vec::with_capacity(curve.spikes.len());
            for &(base, tip) in &curve.spikes {
                if let (Some(b), Some(t)) = (view.project(base, rect), view.project(tip, rect)) {
                    painter.line_segment([b, t], Stroke::new(0.8, COMB_SPIKE));
                    tips.push(t);
                }
            }
            for w in tips.windows(2) {
                painter.line_segment([w[0], w[1]], Stroke::new(1.1, COMB_ENV));
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
                    painter.line_segment([p, q], Stroke::new(0.7, HULL_EDGE));
                }
            }
        }
    }
}

/// A layer's colour, or the default hull grey when the index is out of range.
fn layer_color(surface: &Surface, layer: Option<usize>) -> Color32 {
    let Some(layer) = layer.and_then(|i| surface.layers.get(i)) else {
        return Color32::from_rgb(0x9f, 0xb4, 0xc7);
    };
    let c = layer.color.max(0) as u32;
    Color32::from_rgb(
        ((c >> 16) & 0xff) as u8,
        ((c >> 8) & 0xff) as u8,
        (c & 0xff) as u8,
    )
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
                    if crease { CREASE } else { CONTROL },
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
            painter.circle_filled(s, 5.0, SELECTED);
            painter.circle_stroke(s, 5.0, Stroke::new(1.0, Color32::BLACK));
        } else {
            painter.circle_filled(s, 2.0, POINT);
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
pub fn raster_view(view: &ViewTransform) -> raster::View {
    raster::View {
        eye: view.eye,
        right: view.right,
        up: view.up,
        forward: view.forward,
        ortho_height: view.ortho_height,
        fov_tan: view.fov_tan,
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
        &raster_view(view),
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
