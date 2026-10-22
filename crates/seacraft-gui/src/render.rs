//! Viewport drawing.
//!
//! Everything is CPU-projected into an [`egui::Painter`]: there is no GPU
//! pipeline and the crate stays testable on a plain `egui::Context`. Each
//! viewport draws, in order, a background, a 1 m grid, the refined hull, the
//! control net, the control points, the waterplane and a caption.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, Stroke};
use seacraft_core::export::RenderMesh;
use seacraft_core::math::Vec3;
use seacraft_core::mesh::Surface;

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
pub const WATERPLANE: Color32 = Color32::from_rgba_premultiplied(0x30, 0x6c, 0xb8, 0x38);
pub const WATERLINE: Color32 = Color32::from_rgb(0x5c, 0xa8, 0xf0);
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
}

impl Default for DisplayOptions {
    fn default() -> Self {
        Self {
            shaded: false,
            wireframe: true,
            control_net: true,
            points: true,
            waterplane: true,
            grid: true,
        }
    }
}

/// Everything a viewport draws.
pub struct Scene<'a> {
    pub surface: &'a Surface,
    /// The refined hull, already mirrored and filtered to visible layers.
    pub mesh: &'a RenderMesh,
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

    fn extent(&self) -> f64 {
        (self.max - self.min).length().max(1e-6)
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
    if opts.waterplane {
        draw_waterplane(&painter, &view, rect, scene);
    }
    if opts.shaded || opts.wireframe {
        draw_hull(&painter, &view, rect, scene, opts);
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
        Stroke::new(if active { 2.0 } else { 1.0 }, if active { FRAME_ACTIVE } else { FRAME }),
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
    let step = [0.1, 0.25, 0.5, 1.0, 2.0, 5.0, 10.0, 25.0, 50.0, 100.0, 500.0]
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
        paint(view.right * u + view.up * (v - half), view.right * u + view.up * (v + half), color);
        let v2 = up_base + offset;
        paint(view.right * (base - half) + view.up * v2, view.right * (base + half) + view.up * v2, color);
    }
    let _ = width;
}

fn draw_waterplane(painter: &egui::Painter, view: &ViewTransform, rect: Rect, scene: &Scene) {
    let plane = seacraft_core::hydro::waterplane(
        scene.surface,
        scene.draft,
        scene.trim,
        scene.heel_deg,
    );
    let (u, v) = plane.basis();
    let center = plane.project(scene.center());
    let half = scene.extent();
    let corners = [
        center + u * -half + v * -half,
        center + u * half + v * -half,
        center + u * half + v * half,
        center + u * -half + v * half,
    ];
    let pts: Vec<Pos2> = corners.iter().filter_map(|p| view.project(*p, rect)).collect();
    if pts.len() == 4 {
        painter.add(egui::Shape::convex_polygon(pts.clone(), WATERPLANE, Stroke::new(1.0, WATERLINE)));
        painter.line_segment([pts[0], pts[2]], Stroke::new(1.0, WATERLINE));
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
    if opts.wireframe {
        for f in &mesh.faces {
            for i in 0..f.len() {
                let a = f[i];
                let b = f[(i + 1) % f.len()];
                if let (Some(p), Some(q)) = (view.project(mesh.verts[a], rect), view.project(mesh.verts[b], rect)) {
                    painter.line_segment([p, q], Stroke::new(0.7, HULL_EDGE));
                }
            }
        }
    }
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
            let (Some(p), Some(q)) = (view.project(surface.points[a].p, rect), view.project(surface.points[b].p, rect)) else {
                continue;
            };
            let crease = creases.contains(&seacraft_core::mesh::ordered(a, b));
            painter.line_segment(
                [p, q],
                Stroke::new(if crease { 1.6 } else { 1.0 }, if crease { CREASE } else { CONTROL }),
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
        let Some(s) = view.project(p.p, rect) else { continue };
        if i == selected.unwrap_or(usize::MAX) {
            painter.circle_filled(s, 5.0, SELECTED);
            painter.circle_stroke(s, 5.0, Stroke::new(1.0, Color32::BLACK));
        } else {
            painter.circle_filled(s, 2.0, POINT);
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
