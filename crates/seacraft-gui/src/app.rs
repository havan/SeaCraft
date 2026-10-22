//! SeaCraft desktop application.
//!
//! `App` owns the model and the UI state. `App::draw` takes only an
//! [`egui::Ui`], so the whole application can be driven in tests without an
//! `eframe::Frame`; the [`eframe::App`] implementation is a one-line
//! delegation.

use std::path::PathBuf;

use eframe::egui;
use seacraft_core::export::{self, RenderMesh};
use seacraft_core::math::Vec3;
use seacraft_core::{demo, fef, SubdivisionMode, Surface};

use crate::camera::{Camera, ViewKind};
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
        self.cache = None;
        if let Some(s) = &self.surface {
            let (min, max) = s.extents();
            for cam in &mut self.cameras {
                cam.frame(min, max);
            }
        }
    }

    /// Refine the hull when anything it depends on changed.
    fn sync_cache(&mut self) {
        let Some(surface) = &self.surface else {
            self.cache = None;
            return;
        };
        let stale = match &self.cache {
            Some(c) => c.revision != self.revision || c.level != self.level || c.mode != self.mode,
            None => true,
        };
        if stale {
            let mesh = export::render_mesh(surface, self.level, self.mode);
            let (min, max) = surface.extents();
            self.cache = Some(RenderCache {
                revision: self.revision,
                level: self.level,
                mode: self.mode,
                mesh,
                min,
                max,
            });
        }
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
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

    fn menu(&mut self, ui: &mut egui::Ui) {
        egui::Panel::top("menu_bar").show(ui, |ui| {
            egui::MenuBar::new().ui(ui, |ui| {
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

    fn viewports(&mut self, ui: &mut egui::Ui) {
        self.sync_cache();
        let rect = ui.available_rect_before_wrap();
        if rect.width() < 8.0 || rect.height() < 8.0 {
            return;
        }
        let cells = render::quad(rect);

        for (cell, index) in cells {
            let response = ui.allocate_rect(cell, egui::Sense::click_and_drag());
            if response.hovered() {
                let scroll = ui.input(|i| i.smooth_scroll_delta.y);
                if scroll.abs() > 0.0 {
                    self.cameras[index].zoom((-scroll as f64 * 0.0025).exp());
                }
            }
            if response.dragged() {
                let delta = response.drag_delta();
                let secondary = ui.input(|i| i.pointer.secondary_down());
                if secondary && self.cameras[index].kind.is_perspective() {
                    self.cameras[index].orbit(delta.x, -delta.y);
                } else {
                    self.cameras[index].pan(delta.x, delta.y, cell);
                }
            }
            if response.clicked() {
                self.active = index;
            }

            // The scene borrows the surface, so build it here (the cache is
            // already synced; nothing below mutates `self`).
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
                render::viewport(ui, cell, &self.cameras[index], Some(&scene), &self.display, self.active == index, self.selected);
            } else {
                render::viewport(ui, cell, &self.cameras[index], None, &self.display, self.active == index, None);
            }
        }
    }

    // ---- accessors (tests and future dock panels) ----------------------

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
