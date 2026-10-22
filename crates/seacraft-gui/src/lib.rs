//! SeaCraft desktop GUI: state, viewports and editing.
//!
//! The crate is a thin shell over [`seacraft_core`]: it owns no geometry
//! algorithms, only interaction and drawing.

pub mod app;

pub use app::App;
