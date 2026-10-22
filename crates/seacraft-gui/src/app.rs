//! SeaCraft desktop application.
//!
//! `App` owns the model and the UI state. `App::draw` takes only an
//! [`egui::Ui`], so the whole application can be driven in tests without an
//! `eframe::Frame`; the [`eframe::App`] implementation is a one-line
//! delegation.

use std::path::PathBuf;

use eframe::egui;
use seacraft_core::export::{self, RenderMesh};
use seacraft_core::math::{Plane, Vec3};
use seacraft_core::{demo, fef, SubdivisionMode, Surface};

use crate::camera::{Camera, ViewKind};
use crate::history::History;
use crate::picking::{self, PICK_RADIUS};
use crate::render::{self, DisplayOptions, Scene};

/// Cached refined mesh, invalidated by the revision counter, level or mode.
struct RenderCache {
    revision: u64,
    level: u8,
    mode: SubdivisionMode,
    mesh: RenderMesh,
    min: Vec3,
    max: Vec3,
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

pub struct App {
    surface: Option<Surface>,
    path: Option<PathBuf>,
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
    history: History,
    drag: Option<DragState>,
    /// Viewport rectangles from the last frame, so tests can aim the pointer.
    viewport_rects: [egui::Rect; 4],
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
            history: History::new(64),
            drag: None,
            viewport_rects: [egui::Rect::NOTHING; 4],
        }
    }

    /// Load a `.fef` model, reporting failures in the status bar instead of
    /// panicking: a bad file must never take the application down.
    pub fn load(&mut self, path: PathBuf) {
        match std::fs::read_to_string(&path) {
            Ok(text) => match fef::parse(&text) {
                Ok(surface) => {
                    self.status = format!(
                        "{} — {} points, {} faces, {} layers",
                        surface.settings.name,
                        surface.points.len(),
                        surface.faces.len(),
                        surface.layers.len()
                    );
                    self.surface = Some(surface);
                    self.path = Some(path);
                    self.adopt();
                }
                Err(err) => self.status = format!("Parse error in {}: {err}", path.display()),
            },
            Err(err) => self.status = format!("Cannot read {}: {err}", path.display()),
        }
    }

    pub fn load_demo(&mut self) {
        let surface = demo::demo_hull(false);
        self.status = format!(
            "{} — built-in demo hull ({} points, {} faces)",
            surface.settings.name,
            surface.points.len(),
            surface.faces.len()
        );
        self.surface = Some(surface);
        self.path = None;
        self.adopt();
    }

    /// Reset the derived state after a model change.
    fn adopt(&mut self) {
        self.revision += 1;
        self.selected = None;
        self.drag = None;
        self.cache = None;
        self.history.clear();
        if let Some(s) = &self.surface {
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
        let level = if self.drag.is_some() { self.level.min(2) } else { self.level };
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
                min,
                max,
            });
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
        self.shortcuts(ui);
        self.menu(ui);
        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("v{}", seacraft_core::VERSION));
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| self.viewports(ui));
    }

    fn shortcuts(&mut self, ui: &mut egui::Ui) {
        // Read the modifiers from the key event itself: `InputState::modifiers`
        // is only refreshed by a `ModifiersChanged` event, which a synthesised
        // test frame need not send.
        let (undo, redo, escape, frame) = ui.input(|i| {
            let mut undo = false;
            let mut redo = false;
            for event in &i.events {
                if let egui::Event::Key { key, pressed: true, modifiers, .. } = event {
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
        }
        if frame {
            self.reset_cameras();
        }
        for (i, key) in [egui::Key::Num1, egui::Key::Num2, egui::Key::Num3, egui::Key::Num4]
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
                ui.menu_button("Edit", |ui| {
                    if ui.add_enabled(self.history.can_undo(), egui::Button::new("Undo  (Ctrl+Z)")).clicked() {
                        self.undo();
                        ui.close();
                    }
                    if ui.add_enabled(self.history.can_redo(), egui::Button::new("Redo  (Ctrl+Shift+Z)")).clicked() {
                        self.redo();
                        ui.close();
                    }
                });
                ui.menu_button("File", |ui| {
                    if ui.button("Load demo hull").clicked() {
                        self.load_demo();
                        ui.close();
                    }
                    ui.separator();
                    ui.add_enabled(false, egui::Button::new("Open .fef…"));
                    ui.add_enabled(false, egui::Button::new("Save"));
                    ui.separator();
                    ui.add_enabled(false, egui::Button::new("Export…"));
                });
                ui.menu_button("View", |ui| {
                    ui.checkbox(&mut self.display.shaded, "Shaded hull");
                    ui.checkbox(&mut self.display.wireframe, "Wireframe");
                    ui.checkbox(&mut self.display.control_net, "Control net");
                    ui.checkbox(&mut self.display.points, "Control points");
                    ui.checkbox(&mut self.display.waterplane, "Waterplane");
                    ui.checkbox(&mut self.display.grid, "Grid");
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

    fn reset_cameras(&mut self) {
        if let Some(s) = &self.surface {
            let (min, max) = s.extents();
            for cam in &mut self.cameras {
                cam.frame(min, max);
            }
        }
    }

    fn undo(&mut self) {
        let Some(current) = self.surface.clone() else { return };
        if let Some(previous) = self.history.undo(&current) {
            let moved = previous.points.len();
            self.surface = Some(previous);
            self.revision += 1;
            self.status = format!("undo — {moved} control points restored");
        }
    }

    fn redo(&mut self) {
        let Some(current) = self.surface.clone() else { return };
        if let Some(next) = self.history.redo(&current) {
            self.surface = Some(next);
            self.revision += 1;
            self.status = "redo".to_owned();
        }
    }

    fn viewports(&mut self, ui: &mut egui::Ui) {
        self.sync_cache();
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let cells = render::quad(rect);

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
            }

            if let (Some(surface), Some(cache)) = (&self.surface, &self.cache) {
                let scene = Scene {
                    surface,
                    mesh: &cache.mesh,
                    min: cache.min,
                    max: cache.max,
                    draft: surface.settings.draft,
                    trim: 0.0,
                    heel_deg: 0.0,
                };
                render::viewport(
                    ui,
                    cell,
                    &self.cameras[index],
                    Some(&scene),
                    &self.display,
                    self.active == index,
                    self.selected,
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
        let Some(point) = self.pick_at(ui, view, cell) else { return };
        let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) else { return };
        let Some(surface) = self.surface.as_ref() else { return };
        let center = surface.points[point].p;
        let plane = picking::drag_plane(view, center);
        let offset = picking::ray_plane(&view.ray(cursor, cell), &plane)
            .map(|hit| center - hit)
            .unwrap_or(Vec3::ZERO);
        self.history.snapshot(surface);
        self.selected = Some(point);
        self.active = index;
        self.status = format!("dragging control point {point}");
        self.drag = Some(DragState { point, plane, offset, view: index, original: center });
    }

    fn drag_to(&mut self, ui: &egui::Ui, view: &crate::camera::ViewTransform, cell: egui::Rect) {
        let Some(cursor) = ui.input(|i| i.pointer.interact_pos()) else { return };
        let Some(drag) = self.drag.as_ref() else { return };
        let Some(hit) = picking::ray_plane(&view.ray(cursor, cell), &drag.plane) else { return };
        let target = hit + drag.offset;
        let point = drag.point;
        if let Some(surface) = self.surface.as_mut() {
            surface.points[point].p = target;
        }
        self.revision += 1;
    }

    // ---- accessors (tests and dock panels) -----------------------------

    pub fn surface(&self) -> Option<&Surface> {
        self.surface.as_ref()
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

    pub fn undo_len(&self) -> usize {
        self.history.undo_len()
    }

    pub fn display(&self) -> &DisplayOptions {
        &self.display
    }

    pub fn display_mut(&mut self) -> &mut DisplayOptions {
        &mut self.display
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
