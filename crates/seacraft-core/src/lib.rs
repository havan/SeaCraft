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

/// Shown next to stability criteria and on design reports. Results of this
/// program are not validated; `DISCLAIMER.md` says what that means.
pub const NOT_VALIDATED_NOTE: &str =
    "Not validated: results may be wrong and are not class, flag or statutory approval; see DISCLAIMER.md.";

/// Localized form of [`NOT_VALIDATED_NOTE`].
pub fn not_validated_note(locale: i18n::Locale) -> &'static str {
    i18n::Tr::new(locale).s(i18n::Key::DisclaimerNotValidated)
}

pub mod analysis;
/// Connected editing: presets and the weights of the points that follow a move.
pub mod connected;
/// Contour lines of a gridded scalar (marching squares).
pub mod contour;
pub mod control_auth;
pub mod curves;
pub mod damage;
pub mod demo;
pub mod develop;
pub mod dxf;
/// Ready-made example hulls with their design sidecars.
pub mod examples;
pub mod export;
pub mod fef;
pub mod flatten;
/// The embedded DejaVu fonts and their licence.
pub mod fonts;
pub mod heel;
pub mod hydro;
/// Compile-time checked multi-language catalogs and locale detection.
pub mod i18n;
pub mod iges;
/// Crash-safe file output: temporary sibling, sync, rename.
pub mod io;
pub mod lackenby;
pub mod linesplan;
pub mod loading;
pub mod math;
pub mod mesh;
pub mod michell;
/// Outline packing on rectangular sheets or blanks.
pub mod nest;
/// The control net read as lengthwise lines and stations.
pub mod netlines;
/// Moving control points: one rule for the window and the agents.
pub mod netmove;
pub mod offsets;
pub mod optimize;
pub mod panel_choices;
pub mod panel_dims;
pub mod panel_layout;
pub mod panels;
mod par;
pub mod parametric;
/// A small PDF writer: pages, reusable drawings, stroked lines, Unicode text.
pub mod pdf;
pub mod planing;
pub mod project;
pub mod raster;
/// Whether a segment is blocked by a triangle mesh.
pub mod raycast;
pub mod report;
pub mod resistance;
/// Lewis conformal mapping and Ursell–Tasai 2D heave radiation.
pub mod seakeeping;
/// Irregular head-sea responses: spectra, significant motions, accelerations, MSI.
pub mod seastate;
/// A 2D drawing in metres and its writers: tiled PDF, single-page PDF, SVG, DXF.
pub mod sheet;
pub mod stability;
pub mod subdiv;
pub mod table;
/// Full-size patterns from a hull: station moulds, the profile, the transom.
pub mod templates;
pub mod threemf;
pub mod topology;
pub mod transform;
pub mod trimopt;
/// Read a TrueType font and write a subset of it.
pub mod ttf;
/// Lengths as text: metric first, imperial beside it when asked.
pub mod units;
/// Far-field thin-ship wave pattern behind a hull: Michell's spectrum, the Kelvin wedge.
pub mod wave_pattern;
pub mod weather;

pub use analysis::AnalysisMode;
pub use hydro::{HydroOptions, Hydrostatics};
pub use mesh::{Layer, ProjectSettings, Surface};
pub use report::{design_report, ReportOptions};
pub use seakeeping::{
    heave_2d, heave_added_mass_infinite, lewis_contour, lewis_form, Heave2d, LewisForm,
};
pub use subdiv::{SubdivisionMode, SubdivisionOptions};

#[cfg(test)]
mod build_version;

/// The version of this build: the release version alone for a clean checkout of its tag, otherwise
/// naming the commit, such as `0.3.0+67-ga4b51c2-dirty` or `0.3.1-dev+gabc1234`. The rule is in
/// `build_version.rs`.
pub const VERSION: &str = env!("SEACRAFT_VERSION");

/// The release version from `Cargo.toml`, without build metadata.
pub const RELEASE: &str = env!("CARGO_PKG_VERSION");

/// The full commit this was built from, or `None` when the source was not a git checkout.
pub const GIT_COMMIT: Option<&str> = match env!("SEACRAFT_GIT_COMMIT").as_bytes() {
    [] => None,
    _ => Some(env!("SEACRAFT_GIT_COMMIT")),
};

/// Where the source of this build can be browsed: the commit when known, otherwise the release tag.
pub fn source_url() -> String {
    let repo = env!("CARGO_PKG_REPOSITORY");
    match GIT_COMMIT {
        Some(commit) => format!("{repo}/tree/{commit}"),
        None => format!("{repo}/tree/v{RELEASE}"),
    }
}
