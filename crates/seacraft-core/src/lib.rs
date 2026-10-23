//! # SeaCraft
//!
//! A Rust reimplementation of the open source [FREE!ship] hull modelling
//! program: subdivision surface hull design, the `.fef` exchange format,
//! hydrostatics and stability.
//!
//! The geometry kernel reproduces FREE!ship's hybrid Loop / Catmull-Clark
//! refinement (including its averaging pass, crease and corner rules), so a
//! control net imported from the original application refines to the same
//! shape.
//!
//! ```no_run
//! use seacraft_core::{fef, subdiv::SubdivisionOptions, hydro};
//!
//! let text = std::fs::read_to_string("hull.fef").unwrap();
//! let surface = fef::parse(&text).unwrap();
//! let opts = hydro::HydroOptions::from_surface(&surface, SubdivisionOptions { level: 3, mode: Default::default() });
//! let result = hydro::compute(&surface, &opts);
//! println!("{} m3, LCB at {:.3}", result.volume_m3, result.lcb.x);
//! ```
//!
//! [FREE!ship]: https://sourceforge.net/projects/freeship

pub mod analysis;
pub mod curves;
pub mod demo;
pub mod develop;
pub mod dxf;
pub mod export;
pub mod fef;
pub mod hydro;
pub mod iges;
pub mod lackenby;
pub mod linesplan;
pub mod loading;
pub mod math;
pub mod mesh;
pub mod michell;
pub mod offsets;
pub mod optimize;
pub mod parametric;
pub mod planing;
pub mod raster;
pub mod report;
pub mod resistance;
pub mod stability;
pub mod weather;
pub mod subdiv;
pub mod table;
pub mod threemf;
pub mod topology;
pub mod trimopt;
pub mod transform;

pub use analysis::AnalysisMode;
pub use hydro::{HydroOptions, Hydrostatics};
pub use mesh::{Layer, ProjectSettings, Surface};
pub use report::{design_report, ReportOptions};
pub use subdiv::{SubdivisionMode, SubdivisionOptions};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
