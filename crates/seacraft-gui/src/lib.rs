//! SeaCraft desktop GUI: state, viewports and editing.
//!
//! The crate is a thin shell over [`seacraft_core`]: it owns no geometry
//! algorithms, only interaction and drawing.

pub mod app;
pub mod camera;
pub mod commands;
pub mod control;
pub mod history;
pub mod new_design;
pub mod panels;
pub mod picking;
pub mod render;
pub mod theme;

pub use app::App;
