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
pub mod control_auth;
pub mod curves;
pub mod damage;
pub mod demo;
pub mod develop;
pub mod dxf;
pub mod export;
pub mod fef;
pub mod hydro;
pub mod iges;
/// Crash-safe file output: temporary sibling, sync, rename.
pub mod io;
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
pub mod project;
pub mod raster;
pub mod report;
pub mod resistance;
/// Lewis conformal mapping and Ursell–Tasai 2D heave radiation.
pub mod seakeeping;
/// Irregular head-sea responses: spectra, significant motions, accelerations, MSI.
pub mod seastate;
pub mod stability;
pub mod subdiv;
pub mod table;
pub mod threemf;
pub mod topology;
pub mod transform;
pub mod trimopt;
pub mod weather;

pub use analysis::AnalysisMode;
pub use hydro::{HydroOptions, Hydrostatics};
pub use mesh::{Layer, ProjectSettings, Surface};
pub use report::{design_report, ReportOptions};
pub use seakeeping::{
    heave_2d, heave_added_mass_infinite, lewis_contour, lewis_form, Heave2d, LewisForm,
};
pub use subdiv::{SubdivisionMode, SubdivisionOptions};

/// Crate version.
pub const VERSION: &str = env!("CARGO_PKG_VERSION");
