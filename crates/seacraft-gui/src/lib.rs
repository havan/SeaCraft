//! SeaCraft desktop GUI: state, viewports and editing.
//!
//! The crate is a thin shell over [`seacraft_core`]: it owns no geometry
//! algorithms, only interaction and drawing.

/// The desktop identity: the window's app id, which matches the desktop entry in `packaging/linux`.
pub const APP_ID: &str = "io.github.havan.SeaCraft";

/// The window and taskbar icon, drawn from `packaging/linux/io.github.havan.SeaCraft.svg`.
pub fn window_icon() -> eframe::egui::IconData {
    eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon-256.png"))
        .expect("the embedded icon is a valid PNG")
}

pub mod analysis_views;
pub mod app;
pub mod autosave;
pub mod camera;
pub mod commands;
pub mod control;
pub mod gpu_hull;
pub mod history;
pub mod jobs;
pub mod messages;
pub mod model_menu;
pub mod new_design;
pub mod panels;
pub mod picking;
pub mod recent;
pub mod render;
pub mod theme;

pub use app::App;
