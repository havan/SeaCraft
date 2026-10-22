//! SeaCraft desktop application.
//!
//! `App` owns the model and the UI state. `App::draw` takes only an
//! [`egui::Ui`], so the whole application can be driven in tests without an
//! `eframe::Frame`; the [`eframe::App`] implementation is a one-line
//! delegation.

use std::path::PathBuf;

use eframe::egui;
use seacraft_core::{demo, fef, Surface};

pub struct App {
    surface: Option<Surface>,
    path: Option<PathBuf>,
    status: String,
}

impl App {
    pub fn new(_cc: &eframe::CreationContext<'_>, open: Option<PathBuf>) -> Self {
        let mut app = Self {
            surface: None,
            path: None,
            status: "No model loaded. File > Load demo hull.".to_owned(),
        };
        if let Some(p) = open {
            app.load(p);
        }
        app
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
    }

    pub fn draw(&mut self, ui: &mut egui::Ui) {
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
                    ui.add_enabled(false, egui::Button::new("Reset cameras"));
                });
                ui.menu_button("Help", |ui| {
                    ui.label(format!("SeaCraft {}", seacraft_core::VERSION));
                    ui.label("A Rust reimplementation of FREE!ship.");
                });
            });
        });

        egui::Panel::bottom("status_bar").show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(&self.status);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(format!("v{}", seacraft_core::VERSION));
                });
            });
        });

        egui::CentralPanel::default().show(ui, |ui| match &self.surface {
            Some(surface) => {
                ui.heading(&surface.settings.name);
                ui.label(format!(
                    "{} control points · {} faces · {} layers",
                    surface.points.len(),
                    surface.faces.len(),
                    surface.layers.len()
                ));
                ui.add_space(8.0);
                ui.label("Geometry loaded. Viewports arrive in Task 3.");
            }
            None => {
                ui.centered_and_justified(|ui| {
                    ui.label("No model loaded.");
                });
            }
        });
    }

    /// Current model, if any. Public so tests and future dock panels can read it.
    pub fn surface(&self) -> Option<&Surface> {
        self.surface.as_ref()
    }

    pub fn path(&self) -> Option<&std::path::Path> {
        self.path.as_deref()
    }

    pub fn status(&self) -> &str {
        &self.status
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.draw(ui);
    }
}
