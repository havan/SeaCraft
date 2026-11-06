//! Design dossier: a self-contained HTML report of a hull.
//!
//! Produces a comprehensive naval architecture report: principal particulars,
//! inlined linesplan, hydrostatics table over a draft range, sectional area
//! curve, intact stability (GZ curve and IMO A.749 criteria), Holtrop–Mennen
//! still-water resistance, and optional Savitsky planing analysis.

use std::fmt::Write as _;

use crate::curves;
use crate::damage;
use crate::hydro::{Hull, HydroOptions};
use crate::linesplan::{self, LinesplanOptions};
use crate::loading;
use crate::mesh::Surface;
use crate::planing::{self, PlaningPoint};
use crate::project::Project;
use crate::resistance::{self, HoltropInput, Resistance};
use crate::seakeeping::{self, SeakeepingSpec};
use crate::seastate::{self, IrregularSpec, WaveSpectrum};
use crate::stability::{self, upright_lcb, ImoCriteria, StabilityCondition};
use crate::subdiv::{SubdivisionMode, SubdivisionOptions};
use crate::table;

/// Options configuring the generated design report.
#[derive(Debug, Clone)]
pub struct ReportOptions {
    /// Design draft in metres. Defaults to the model's project setting if positive,
    /// otherwise 50 % of the hull depth.
    pub draft: f64,
    /// Vertical centre of gravity above keel in metres. Defaults to 75 % of the hull depth.
    pub kg: f64,
    /// Longitudinal centre of gravity in model `x`. `None` trims freely about the upright LCB.
    pub lcg_x: Option<f64>,
    /// Heel angles to evaluate in degrees. Defaults to 0° to 90°.
    pub heels: Vec<f64>,
    /// Speeds in knots to evaluate for resistance/planing.
    pub speeds_kn: Vec<f64>,
    /// Displacements in tonnes for stability evaluation. `None` evaluates at the design displacement.
    pub displacements_t: Option<Vec<f64>>,
    /// Subdivision level (0–6). Defaults to 3.
    pub level: u8,
    /// Subdivision mode. Defaults to Loop.
    pub mode: SubdivisionMode,
    /// Whether to include Holtrop–Mennen resistance analysis. Defaults to true.
    pub include_resistance: bool,
    /// Whether to include Savitsky planing hull analysis. Defaults to false.
    pub include_planing: bool,
}

impl ReportOptions {
    /// Create options with defaults derived from the surface's project settings and extents.
    ///
    /// Defaults:
    /// - `draft`: `surface.settings.draft` if positive, else 0.5 × depth (`max.z - min.z`).
    /// - `kg`: 0.75 × depth (`max.z - min.z`) above keel.
    /// - `lcg_x`: `None` (free to trim about upright LCB).
    /// - `heels`: `[0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0]`.
    /// - `speeds_kn`: `[6.0, 8.0, 10.0, 12.0, 14.0]`.
    /// - `displacements_t`: `None`.
    /// - `level`: 3.
    /// - `mode`: `SubdivisionMode::Loop`.
    /// - `include_resistance`: `true`.
    /// - `include_planing`: `false`.
    pub fn from_surface(surface: &Surface) -> Self {
        let (min, max) = surface.extents();
        let depth = (max.z - min.z).max(0.1);
        let draft = if surface.settings.draft > 0.0 {
            surface.settings.draft
        } else {
            0.5 * depth
        };
        let kg = 0.75 * depth;
        Self {
            draft,
            kg,
            lcg_x: None,
            heels: vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0],
            speeds_kn: vec![6.0, 8.0, 10.0, 12.0, 14.0],
            displacements_t: None,
            level: 3,
            mode: SubdivisionMode::Loop,
            include_resistance: true,
            include_planing: false,
        }
    }
}

impl Default for ReportOptions {
    fn default() -> Self {
        Self {
            draft: 1.0,
            kg: 1.0,
            lcg_x: None,
            heels: vec![0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 75.0, 90.0],
            speeds_kn: vec![6.0, 8.0, 10.0, 12.0, 14.0],
            displacements_t: None,
            level: 3,
            mode: SubdivisionMode::Loop,
            include_resistance: true,
            include_planing: false,
        }
    }
}

/// Names of the HTML `<section>` elements that this configuration will produce.
pub fn report_sections(opts: &ReportOptions) -> Vec<&'static str> {
    report_sections_with_project(opts, &Project::default())
}

/// Names of the HTML `<section>` elements that this configuration and project will produce.
pub fn report_sections_with_project(opts: &ReportOptions, project: &Project) -> Vec<&'static str> {
    let mut sections = vec![
        "particulars",
        "lines",
        "hydrostatics",
        "sac",
        "stability",
        "resistance",
    ];
    if opts.include_planing {
        sections.push("planing");
    }
    let has_loading = match &project.report.loading_conditions {
        Some(names) => !names.is_empty(),
        None => !project.loading_conditions.is_empty(),
    };
    if has_loading {
        sections.push("loading");
    }
    let has_damage = match &project.report.damage_cases {
        Some(names) => !names.is_empty(),
        None => !project.damage_cases.is_empty(),
    };
    if has_damage {
        sections.push("damage");
    }
    if project.report.seakeeping.is_some() {
        sections.push("seakeeping");
    }
    sections
}

/// Produce a standalone self-contained HTML5 design report of the hull.
///
/// Output contains inline CSS, inline SVG, no external resources or JavaScript,
/// print-friendly formatting, and sections with IDs:
/// - `particulars`
/// - `lines`
/// - `hydrostatics`
/// - `sac`
/// - `stability`
/// - `resistance`
/// - `planing` (when `opts.include_planing` is true)
pub fn design_report(surface: &Surface, opts: &ReportOptions) -> Result<String, String> {
    design_report_with_project(surface, opts, &Project::default())
}

/// Produce a standalone self-contained HTML5 design report of the hull and project sidecar data.
pub fn design_report_with_project(
    surface: &Surface,
    opts: &ReportOptions,
    project: &Project,
) -> Result<String, String> {
    if !opts.draft.is_finite() || opts.draft <= 0.0 {
        return Err("design_report: draft must be positive and finite".into());
    }
    if !opts.kg.is_finite() || opts.kg < 0.0 {
        return Err("design_report: kg must be non-negative and finite".into());
    }

    let subdiv = SubdivisionOptions {
        level: opts.level,
        mode: opts.mode,
    };
    let hull = Hull::new(surface, subdiv);
    let mut h_opts = HydroOptions::from_surface(surface, subdiv);
    h_opts.draft = opts.draft;
    h_opts.trim = 0.0;
    h_opts.stations = 21;

    let h = if let Some(lcg) = opts.lcg_x {
        stability::hydrostatics_free_trim(&hull, &h_opts, Some(lcg)).0
    } else {
        hull.hydrostatics(&h_opts)
    };

    let esc_name = html_escape(&surface.settings.name);
    let esc_designer = html_escape(&surface.settings.designer);
    let esc_created = html_escape(&surface.settings.created_by);
    let esc_comment = html_escape(&surface.settings.comment);

    let mut out = String::with_capacity(65536);

    out.push_str("<!DOCTYPE html>\n<html lang=\"en\">\n<head>\n");
    out.push_str("<meta charset=\"utf-8\">\n");
    out.push_str("<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\n");
    let _ = writeln!(out, "<title>{} - Design Dossier</title>", esc_name);
    out.push_str("<style>\n");
    out.push_str(":root {\n");
    out.push_str("  --bg: #ffffff;\n");
    out.push_str("  --text: #1a1e24;\n");
    out.push_str("  --text-muted: #57606a;\n");
    out.push_str("  --border: #d0d7de;\n");
    out.push_str("  --card-bg: #f6f8fa;\n");
    out.push_str("  --primary: #0969da;\n");
    out.push_str("  --pass-bg: #dafbe1;\n");
    out.push_str("  --pass-text: #1a7f37;\n");
    out.push_str("  --fail-bg: #ffebe9;\n");
    out.push_str("  --fail-text: #cf222e;\n");
    out.push_str("}\n");
    out.push_str("* { box-sizing: border-box; }\n");
    out.push_str("body {\n");
    out.push_str("  font-family: -apple-system, BlinkMacSystemFont, \"Segoe UI\", Roboto, Helvetica, Arial, sans-serif;\n");
    out.push_str("  color: var(--text);\n");
    out.push_str("  background: var(--bg);\n");
    out.push_str("  line-height: 1.5;\n");
    out.push_str("  margin: 0 auto;\n");
    out.push_str("  padding: 2.5rem;\n");
    out.push_str("  max-width: 1100px;\n");
    out.push_str("}\n");
    out.push_str("header {\n");
    out.push_str("  border-bottom: 2px solid var(--border);\n");
    out.push_str("  padding-bottom: 1.25rem;\n");
    out.push_str("  margin-bottom: 2.5rem;\n");
    out.push_str("}\n");
    out.push_str("header h1 { margin: 0 0 0.4rem 0; font-size: 2.2rem; color: #111; }\n");
    out.push_str(".subtitle { color: var(--text-muted); font-size: 1.05rem; margin: 0; }\n");
    out.push_str("section {\n");
    out.push_str("  margin-bottom: 3rem;\n");
    out.push_str("  page-break-inside: avoid;\n");
    out.push_str("}\n");
    out.push_str("section h2 {\n");
    out.push_str("  font-size: 1.45rem;\n");
    out.push_str("  border-bottom: 1px solid var(--border);\n");
    out.push_str("  padding-bottom: 0.4rem;\n");
    out.push_str("  margin-top: 0;\n");
    out.push_str("  margin-bottom: 1.2rem;\n");
    out.push_str("  color: #111;\n");
    out.push_str("}\n");
    out.push_str(".cards-grid {\n");
    out.push_str("  display: grid;\n");
    out.push_str("  grid-template-columns: repeat(auto-fit, minmax(240px, 1fr));\n");
    out.push_str("  gap: 1rem;\n");
    out.push_str("  margin-bottom: 1.5rem;\n");
    out.push_str("}\n");
    out.push_str(".card {\n");
    out.push_str("  background: var(--card-bg);\n");
    out.push_str("  border: 1px solid var(--border);\n");
    out.push_str("  border-radius: 6px;\n");
    out.push_str("  padding: 1rem 1.2rem;\n");
    out.push_str("}\n");
    out.push_str(".card h3 { margin: 0 0 0.6rem 0; font-size: 1rem; color: var(--primary); }\n");
    out.push_str(".card dl {\n");
    out.push_str("  margin: 0;\n");
    out.push_str("  display: grid;\n");
    out.push_str("  grid-template-columns: 1fr auto;\n");
    out.push_str("  row-gap: 0.35rem;\n");
    out.push_str("  font-size: 0.9rem;\n");
    out.push_str("}\n");
    out.push_str(".card dt { color: var(--text-muted); }\n");
    out.push_str(".card dd { margin: 0; font-weight: 500; text-align: right; font-variant-numeric: tabular-nums; }\n");
    out.push_str("table {\n");
    out.push_str("  width: 100%;\n");
    out.push_str("  border-collapse: collapse;\n");
    out.push_str("  font-variant-numeric: tabular-nums;\n");
    out.push_str("  margin-bottom: 1.5rem;\n");
    out.push_str("  font-size: 0.88rem;\n");
    out.push_str("}\n");
    out.push_str("th, td {\n");
    out.push_str("  padding: 0.45rem 0.65rem;\n");
    out.push_str("  text-align: right;\n");
    out.push_str("  border-bottom: 1px solid var(--border);\n");
    out.push_str("}\n");
    out.push_str("th { background: var(--card-bg); font-weight: 600; color: var(--text); }\n");
    out.push_str("th:first-child, td:first-child { text-align: left; }\n");
    out.push_str(".text-left { text-align: left; }\n");
    out.push_str(".text-center { text-align: center; }\n");
    out.push_str("tr.active-row td { background: #eef6ff; font-weight: 600; }\n");
    out.push_str(".badge-pass {\n");
    out.push_str("  background: var(--pass-bg);\n");
    out.push_str("  color: var(--pass-text);\n");
    out.push_str("  font-weight: bold;\n");
    out.push_str("  padding: 0.2rem 0.55rem;\n");
    out.push_str("  border-radius: 4px;\n");
    out.push_str("  display: inline-block;\n");
    out.push_str("}\n");
    out.push_str(".badge-fail {\n");
    out.push_str("  background: var(--fail-bg);\n");
    out.push_str("  color: var(--fail-text);\n");
    out.push_str("  font-weight: bold;\n");
    out.push_str("  padding: 0.2rem 0.55rem;\n");
    out.push_str("  border-radius: 4px;\n");
    out.push_str("  display: inline-block;\n");
    out.push_str("}\n");
    out.push_str(".svg-frame {\n");
    out.push_str("  width: 100%;\n");
    out.push_str("  overflow-x: auto;\n");
    out.push_str("  background: #ffffff;\n");
    out.push_str("  border: 1px solid var(--border);\n");
    out.push_str("  border-radius: 6px;\n");
    out.push_str("  margin-bottom: 1.5rem;\n");
    out.push_str("}\n");
    out.push_str(".svg-frame svg { width: 100%; height: auto; display: block; }\n");
    out.push_str(".note-box {\n");
    out.push_str("  background: var(--card-bg);\n");
    out.push_str("  border-left: 4px solid var(--primary);\n");
    out.push_str("  padding: 0.75rem 1rem;\n");
    out.push_str("  margin-bottom: 1rem;\n");
    out.push_str("  font-size: 0.9rem;\n");
    out.push_str("}\n");
    out.push_str(".warn-box {\n");
    out.push_str("  background: #fff8c5;\n");
    out.push_str("  border-left: 4px solid #9a6700;\n");
    out.push_str("  padding: 0.75rem 1rem;\n");
    out.push_str("  margin-bottom: 1rem;\n");
    out.push_str("  font-size: 0.9rem;\n");
    out.push_str("  color: #573c00;\n");
    out.push_str("}\n");
    out.push_str(".not-validated {\n");
    out.push_str("  background: var(--fail-bg);\n");
    out.push_str("  color: var(--fail-text);\n");
    out.push_str("  padding: 0.5rem 1rem;\n");
    out.push_str("  border-radius: 4px;\n");
    out.push_str("}\n\n");
    out.push_str("footer {\n");
    out.push_str("  border-top: 1px solid var(--border);\n");
    out.push_str("  padding-top: 1.5rem;\n");
    out.push_str("  margin-top: 3.5rem;\n");
    out.push_str("  font-size: 0.85rem;\n");
    out.push_str("  color: var(--text-muted);\n");
    out.push_str("  text-align: center;\n");
    out.push_str("}\n");
    out.push_str("@media print {\n");
    out.push_str("  body { padding: 0; font-size: 10pt; max-width: 100%; }\n");
    out.push_str("  section { page-break-inside: avoid; margin-bottom: 2rem; }\n");
    out.push_str("  .svg-frame { border: none; }\n");
    out.push_str("  @page { margin: 1.5cm; }\n");
    out.push_str("}\n");
    out.push_str("</style>\n</head>\n<body>\n");

    let _ = writeln!(
        out,
        "<p class=\"not-validated\"><strong>{}</strong></p>",
        crate::NOT_VALIDATED_NOTE
    );
    out.push_str("<header>\n");
    let _ = writeln!(out, "  <h1>{}</h1>", esc_name);
    let designer_str = if !esc_designer.is_empty() {
        format!(" &bull; Designer: {}", esc_designer)
    } else {
        String::new()
    };
    let _ = writeln!(
        out,
        "  <p class=\"subtitle\">Design Dossier{}</p>",
        designer_str
    );
    out.push_str("</header>\n\n");

    // ------------------------------------------------------------- 1. Particulars
    out.push_str("<section id=\"particulars\">\n");
    out.push_str("  <h2>Principal Particulars</h2>\n");

    let loa = (hull.model_max.x - hull.model_min.x).max(0.0);
    let boa = (hull.model_max.y - hull.model_min.y).max(0.0);
    let depth = (hull.model_max.z - hull.model_min.z).max(0.0);

    let rho_t = h.water_density / 1000.0;
    let tpc = h.waterplane_area_m2 * rho_t / 100.0;
    let mtc = if h.lwl > 1e-9 {
        rho_t * h.waterplane_inertia_longitudinal / (100.0 * h.lwl)
    } else {
        0.0
    };
    let gmt = h.km_transverse - opts.kg;

    out.push_str("  <div class=\"cards-grid\">\n");

    out.push_str("    <div class=\"card\">\n");
    out.push_str("      <h3>Dimensions</h3>\n");
    out.push_str("      <dl>\n");
    let _ = writeln!(out, "        <dt>LOA</dt><dd>{} m</dd>", fmt_num(loa, 3));
    let _ = writeln!(out, "        <dt>LWL</dt><dd>{} m</dd>", fmt_num(h.lwl, 3));
    let _ = writeln!(out, "        <dt>BWL</dt><dd>{} m</dd>", fmt_num(h.bwl, 3));
    let _ = writeln!(
        out,
        "        <dt>Beam Max</dt><dd>{} m</dd>",
        fmt_num(boa, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>Draft T</dt><dd>{} m</dd>",
        fmt_num(h.draft, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>Depth</dt><dd>{} m</dd>",
        fmt_num(depth, 3)
    );
    out.push_str("      </dl>\n");
    out.push_str("    </div>\n");

    out.push_str("    <div class=\"card\">\n");
    out.push_str("      <h3>Displacement & Areas</h3>\n");
    out.push_str("      <dl>\n");
    let _ = writeln!(
        out,
        "        <dt>Volume &nabla;</dt><dd>{} m&sup3;</dd>",
        fmt_num(h.volume_m3, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>Displacement &Delta;</dt><dd>{} t</dd>",
        fmt_num(h.displacement_t, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>Mass</dt><dd>{} kg</dd>",
        fmt_num(h.displacement_kg, 0)
    );
    let _ = writeln!(
        out,
        "        <dt>Wetted Surface</dt><dd>{} m&sup2;</dd>",
        fmt_num(h.wetted_surface_m2, 2)
    );
    let _ = writeln!(
        out,
        "        <dt>Waterplane Area</dt><dd>{} m&sup2;</dd>",
        fmt_num(h.waterplane_area_m2, 2)
    );
    let _ = writeln!(
        out,
        "        <dt>Density &rho;</dt><dd>{} kg/m&sup3;</dd>",
        fmt_num(h.water_density, 1)
    );
    out.push_str("      </dl>\n");
    out.push_str("    </div>\n");

    out.push_str("    <div class=\"card\">\n");
    out.push_str("      <h3>Form Coefficients</h3>\n");
    out.push_str("      <dl>\n");
    let _ = writeln!(
        out,
        "        <dt>Block Cb</dt><dd>{}</dd>",
        fmt_num(h.block_coefficient, 4)
    );
    let _ = writeln!(
        out,
        "        <dt>Prismatic Cp</dt><dd>{}</dd>",
        fmt_num(h.prismatic_coefficient, 4)
    );
    let _ = writeln!(
        out,
        "        <dt>Midship Cm</dt><dd>{}</dd>",
        fmt_num(h.midship_coefficient, 4)
    );
    let _ = writeln!(
        out,
        "        <dt>Waterplane Cwp</dt><dd>{}</dd>",
        fmt_num(h.waterplane_coefficient, 4)
    );
    let _ = writeln!(
        out,
        "        <dt>Vertical Prismatic Cvp</dt><dd>{}</dd>",
        fmt_num(h.vertical_prismatic_coefficient, 4)
    );
    out.push_str("      </dl>\n");
    out.push_str("    </div>\n");

    out.push_str("    <div class=\"card\">\n");
    out.push_str("      <h3>Centroids & Stability</h3>\n");
    out.push_str("      <dl>\n");
    let _ = writeln!(
        out,
        "        <dt>LCB</dt><dd>{} m</dd>",
        fmt_num(h.lcb.x, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>LCF</dt><dd>{} m</dd>",
        fmt_num(h.lcf.x, 3)
    );
    let _ = writeln!(out, "        <dt>KB</dt><dd>{} m</dd>", fmt_num(h.kb, 3));
    let _ = writeln!(out, "        <dt>KG</dt><dd>{} m</dd>", fmt_num(opts.kg, 3));
    let _ = writeln!(
        out,
        "        <dt>BM Transverse</dt><dd>{} m</dd>",
        fmt_num(h.bm_transverse, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>KM Transverse</dt><dd>{} m</dd>",
        fmt_num(h.km_transverse, 3)
    );
    let _ = writeln!(
        out,
        "        <dt>GM Transverse</dt><dd>{} m</dd>",
        fmt_num(gmt, 3)
    );
    let _ = writeln!(out, "        <dt>TPC</dt><dd>{} t/cm</dd>", fmt_num(tpc, 3));
    let _ = writeln!(
        out,
        "        <dt>MTC</dt><dd>{} t&middot;m/cm</dd>",
        fmt_num(mtc, 3)
    );
    out.push_str("      </dl>\n");
    out.push_str("    </div>\n");

    out.push_str("  </div>\n");

    if !esc_comment.is_empty() || !esc_created.is_empty() {
        out.push_str("  <div class=\"note-box\">\n");
        if !esc_created.is_empty() {
            let _ = writeln!(
                out,
                "    <p><strong>Created by:</strong> {}</p>",
                esc_created
            );
        }
        if !esc_comment.is_empty() {
            let _ = writeln!(out, "    <p><strong>Comments:</strong> {}</p>", esc_comment);
        }
        out.push_str("  </div>\n");
    }

    out.push_str("</section>\n\n");

    // ------------------------------------------------------------- 2. Linesplan
    out.push_str("<section id=\"lines\">\n");
    out.push_str("  <h2>Linesplan</h2>\n");
    let lines_opts = LinesplanOptions {
        level: opts.level,
        mode: opts.mode,
        ..Default::default()
    };
    let lines_svg = linesplan::svg(surface, lines_opts);
    out.push_str("  <div class=\"svg-frame\">\n");
    out.push_str(strip_xml_header(&lines_svg));
    out.push_str("  </div>\n");
    out.push_str("</section>\n\n");

    // ------------------------------------------------------------- 3. Hydrostatics Table
    out.push_str("<section id=\"hydrostatics\">\n");
    out.push_str("  <h2>Hydrostatics Table</h2>\n");
    let draft_max = (opts.draft * 1.25).max(opts.draft);
    let drafts = table::draft_range(draft_max, 10);
    let hyd_table = table::table(surface, &h_opts, &drafts);

    out.push_str("  <table>\n");
    out.push_str("    <thead>\n");
    out.push_str("      <tr>\n");
    out.push_str("        <th>Draft [m]</th><th>Volume [m&sup3;]</th><th>Displ [&Delta; t]</th>\n");
    out.push_str(
        "        <th>KB [m]</th><th>BM [m]</th><th>KM [m]</th><th>LCB [m]</th><th>LCF [m]</th>\n",
    );
    out.push_str("        <th>Cb</th><th>Cp</th><th>Cwp</th><th>Aw [m&sup2;]</th><th>TPC [t/cm]</th><th>MTC [t&middot;m]</th>\n");
    out.push_str("      </tr>\n");
    out.push_str("    </thead>\n");
    out.push_str("    <tbody>\n");

    // Find closest row to design draft
    let mut closest_idx = 0;
    let mut min_diff = f64::INFINITY;
    for (i, r) in hyd_table.rows.iter().enumerate() {
        let diff = (r.draft - opts.draft).abs();
        if diff < min_diff {
            min_diff = diff;
            closest_idx = i;
        }
    }

    for (i, r) in hyd_table.rows.iter().enumerate() {
        let row_class = if i == closest_idx {
            " class=\"active-row\""
        } else {
            ""
        };
        let _ = writeln!(
            out,
            "      <tr{}><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
            row_class,
            fmt_num(r.draft, 3),
            fmt_num(r.volume_m3, 3),
            fmt_num(r.displacement_t, 3),
            fmt_num(r.kb, 3),
            fmt_num(r.bm_transverse, 3),
            fmt_num(r.km_transverse, 3),
            fmt_num(r.lcb_x, 3),
            fmt_num(r.lcf_x, 3),
            fmt_num(r.block_coefficient, 4),
            fmt_num(r.prismatic_coefficient, 4),
            fmt_num(r.waterplane_coefficient, 4),
            fmt_num(r.waterplane_area_m2, 2),
            fmt_num(r.tpc, 3),
            fmt_num(r.mtc, 3),
        );
    }
    out.push_str("    </tbody>\n");
    out.push_str("  </table>\n");
    out.push_str("</section>\n\n");

    // ------------------------------------------------------------- 4. Sectional Area Curve
    out.push_str("<section id=\"sac\">\n");
    out.push_str("  <h2>Sectional Area Curve</h2>\n");
    let sac_title = format!(
        "{} - Sectional Area Curve (T = {} m)",
        esc_name,
        fmt_num(opts.draft, 3)
    );
    let sac_svg = curves::sac_svg(&h.sac, &sac_title);
    out.push_str("  <div class=\"svg-frame\">\n");
    out.push_str(strip_xml_header(&sac_svg));
    out.push_str("  </div>\n");
    out.push_str("</section>\n\n");

    // ------------------------------------------------------------- 5. Stability & IMO Criteria
    out.push_str("<section id=\"stability\">\n");
    out.push_str("  <h2>Intact Stability (GZ Cross Curves &amp; IMO A.749)</h2>\n");

    let design_disp = h.displacement_t;
    let displacements = opts
        .displacements_t
        .clone()
        .unwrap_or_else(|| vec![design_disp]);

    let target_lcg = match opts.lcg_x {
        Some(x) => Some(x),
        None => Some(upright_lcb(&hull, opts.draft)),
    };
    let cond = StabilityCondition {
        kg: opts.kg,
        trim: 0.0,
        lcg_x: target_lcg,
    };
    let curves = stability::cross_curves(&hull, &opts.heels, &displacements, &cond, &h_opts);
    let imo = stability::evaluate_imo_criteria(&curves, 0);

    // Collect (heel, gz) points for displacement index 0
    let mut gz_points: Vec<(f64, Option<f64>)> = Vec::with_capacity(curves.len());
    for c in &curves {
        let gz = c.points.first().and_then(|p| p.gz);
        gz_points.push((c.heel_deg, gz));
    }

    let gz_title = format!(
        "{} - Righting Lever GZ (Displ. {} t, KG {} m)",
        esc_name,
        fmt_num(design_disp, 3),
        fmt_num(opts.kg, 3)
    );
    let gz_svg = gz_chart_svg(&gz_points, imo.as_ref(), &gz_title);
    out.push_str("  <div class=\"svg-frame\">\n");
    out.push_str(&gz_svg);
    out.push_str("  </div>\n");

    // Righting Levers Table
    out.push_str("  <h3>Righting Levers (Cross Curves)</h3>\n");
    out.push_str("  <table>\n");
    out.push_str("    <thead>\n");
    out.push_str("      <tr><th>Heel [&deg;]</th><th>Draft [m]</th><th>Trim [m]</th><th>KN [m]</th><th>GZ [m]</th></tr>\n");
    out.push_str("    </thead>\n");
    out.push_str("    <tbody>\n");
    for c in &curves {
        if let Some(p) = c.points.first() {
            let draft_s = fmt_opt(p.draft, 3);
            let trim_s = fmt_opt(p.trim_m, 3);
            let kn_s = fmt_opt(p.kn, 3);
            let gz_s = fmt_opt(p.gz, 3);
            let _ = writeln!(
                out,
                "      <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                fmt_num(c.heel_deg, 1),
                draft_s,
                trim_s,
                kn_s,
                gz_s
            );
        }
    }
    out.push_str("    </tbody>\n");
    out.push_str("  </table>\n");

    // IMO A.749 Criteria Table
    out.push_str("  <h3>IMO A.749 / 2008 IS Code Intact Stability Criteria</h3>\n");
    out.push_str("  <table>\n");
    out.push_str("    <thead>\n");
    out.push_str("      <tr><th class=\"text-left\">Criterion</th><th>Required</th><th>Actual</th><th class=\"text-center\">Status</th></tr>\n");
    out.push_str("    </thead>\n");
    out.push_str("    <tbody>\n");

    if let Some(ref imo) = imo {
        let p_0_30 = imo.area_0_30_m_rad >= 0.055;
        let p_0_40 = imo.area_0_40_m_rad >= 0.090;
        let p_30_40 = imo.area_30_40_m_rad >= 0.030;
        let p_max_30 = imo.max_gz_30_plus_m >= 0.20;
        let p_angle = imo.angle_of_max_gz_deg >= 25.0;
        let p_gm0 = imo.gm0_m >= 0.15;

        let badge = |pass: bool| {
            if pass {
                "<span class=\"badge-pass\">PASS</span>"
            } else {
                "<span class=\"badge-fail\">FAIL</span>"
            }
        };

        let _ = writeln!(
            out,
            "      <tr><td class=\"text-left\">Area under GZ curve 0&deg; to 30&deg;</td><td>&ge; 0.055 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.area_0_30_m_rad, 4),
            badge(p_0_30)
        );
        let _ = writeln!(
            out,
            "      <tr><td class=\"text-left\">Area under GZ curve 0&deg; to 40&deg;</td><td>&ge; 0.090 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.area_0_40_m_rad, 4),
            badge(p_0_40)
        );
        let _ = writeln!(
            out,
            "      <tr><td class=\"text-left\">Area under GZ curve 30&deg; to 40&deg;</td><td>&ge; 0.030 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.area_30_40_m_rad, 4),
            badge(p_30_40)
        );
        let _ = writeln!(
            out,
            "      <tr><td class=\"text-left\">Maximum GZ at heel &ge; 30&deg;</td><td>&ge; 0.200 m</td><td>{} m</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.max_gz_30_plus_m, 3),
            badge(p_max_30)
        );
        let _ = writeln!(
            out,
            "      <tr><td class=\"text-left\">Angle of maximum GZ</td><td>&ge; 25.0&deg;</td><td>{}&deg;</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.angle_of_max_gz_deg, 1),
            badge(p_angle)
        );
        let _ = writeln!(
            out,
            "      <tr><td class=\"text-left\">Initial metacentric height GM0</td><td>&ge; 0.150 m</td><td>{} m</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.gm0_m, 3),
            badge(p_gm0)
        );
        let _ = writeln!(
            out,
            "      <tr class=\"active-row\"><td class=\"text-left\"><strong>Overall Status</strong></td><td>All criteria met</td><td>Displ. {} t</td><td class=\"text-center\">{}</td></tr>",
            fmt_num(imo.displacement_t, 3),
            badge(imo.passed)
        );
    } else {
        out.push_str("      <tr><td class=\"text-left\">IMO Criteria Evaluation</td><td>&mdash;</td><td>Insufficient heel range or equilibrium data</td><td class=\"text-center\"><span class=\"badge-fail\">FAIL</span></td></tr>\n");
    }

    out.push_str("    </tbody>\n");
    out.push_str("  </table>\n");

    // IMO 2008 IS Code §2.3 Severe Wind and Rolling (Weather) Criterion Table
    let weather_res = stability::evaluate_hull_weather(
        &hull,
        &h_opts,
        &cond,
        &stability::WeatherInput::default(),
    );
    let badge = |pass: bool| {
        if pass {
            "<span class=\"badge-pass\">PASS</span>"
        } else {
            "<span class=\"badge-fail\">FAIL</span>"
        }
    };
    out.push_str("  <h3>IMO 2008 IS Code Part A &sect;2.3 Severe Wind and Rolling (Weather) Criterion</h3>\n");
    out.push_str("  <table>\n");
    out.push_str("    <thead>\n");
    out.push_str("      <tr><th class=\"text-left\">Parameter / Criterion</th><th>Requirement / Formula</th><th>Value</th><th class=\"text-center\">Status</th></tr>\n");
    out.push_str("    </thead>\n");
    out.push_str("    <tbody>\n");
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Lateral windage area A &amp; lever Z</td><td>P = {} Pa</td><td>A = {} m&sup2;, Z = {} m</td><td class=\"text-center\">&mdash;</td></tr>",
        fmt_num(weather_res.wind_pressure_pa, 0),
        fmt_num(weather_res.area_m2, 3),
        fmt_num(weather_res.z_m, 3)
    );
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Wind heeling levers l<sub>w1</sub> / l<sub>w2</sub></td><td>l<sub>w2</sub> = 1.5 &middot; l<sub>w1</sub></td><td>{} m / {} m</td><td class=\"text-center\">&mdash;</td></tr>",
        fmt_num(weather_res.lw1_m, 4),
        fmt_num(weather_res.lw2_m, 4)
    );
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Roll factors (X<sub>1</sub>, X<sub>2</sub>, k, r, s) &amp; period T</td><td>T = 2&middot;C&middot;B / &radic;GM</td><td>({}, {}, {}, {}, {}), T = {} s</td><td class=\"text-center\">&mdash;</td></tr>",
        fmt_num(weather_res.x1, 3),
        fmt_num(weather_res.x2, 3),
        fmt_num(weather_res.k, 3),
        fmt_num(weather_res.r, 3),
        fmt_num(weather_res.s, 4),
        fmt_num(weather_res.roll_period_s, 2)
    );
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Roll-back angle &phi;<sub>1</sub></td><td>109&middot;k&middot;X<sub>1</sub>&middot;X<sub>2</sub>&middot;&radic;(r&middot;s)</td><td>{}&deg;</td><td class=\"text-center\">&mdash;</td></tr>",
        fmt_num(weather_res.phi1_deg, 2)
    );
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Steady wind heel angle &phi;<sub>0</sub></td><td>&le; {}&deg;</td><td>{}&deg;</td><td class=\"text-center\">{}</td></tr>",
        fmt_num(weather_res.phi0_limit_deg, 1),
        fmt_opt(weather_res.phi0_deg, 2),
        badge(weather_res.phi0_passed)
    );
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Integration limits (&phi;<sub>0</sub> &minus; &phi;<sub>1</sub>, &phi;<sub>c</sub>, &phi;<sub>2</sub>)</td><td>&phi;<sub>2</sub> = min(50&deg;, &phi;<sub>f</sub>, &phi;<sub>c2</sub>)</td><td>({}&deg;, {}&deg;, {}&deg;)</td><td class=\"text-center\">&mdash;</td></tr>",
        fmt_opt(weather_res.phi_roll_deg, 2),
        fmt_opt(weather_res.phic_deg, 2),
        fmt_opt(weather_res.phi2_deg, 2)
    );
    let _ = writeln!(
        out,
        "      <tr><td class=\"text-left\">Energy balance (area b vs area a)</td><td>b &ge; a</td><td>a = {} m&middot;rad, b = {} m&middot;rad (b/a = {})</td><td class=\"text-center\">{}</td></tr>",
        fmt_num(weather_res.area_a_m_rad, 4),
        fmt_num(weather_res.area_b_m_rad, 4),
        fmt_num(weather_res.area_ratio, 2),
        badge(weather_res.area_passed)
    );
    let _ = writeln!(
        out,
        "      <tr class=\"active-row\"><td class=\"text-left\"><strong>Weather Criterion Overall</strong></td><td>&phi;<sub>0</sub> &le; 16&deg; and b &ge; a</td><td>l<sub>w1</sub> = {} m, l<sub>w2</sub> = {} m</td><td class=\"text-center\">{}</td></tr>",
        fmt_num(weather_res.lw1_m, 4),
        fmt_num(weather_res.lw2_m, 4),
        badge(weather_res.passed)
    );
    out.push_str("    </tbody>\n");
    out.push_str("  </table>\n");
    out.push_str("</section>\n\n");

    // ------------------------------------------------------------- 6. Resistance (Holtrop-Mennen)
    out.push_str("<section id=\"resistance\">\n");
    out.push_str("  <h2>Resistance &amp; Powering (Holtrop–Mennen)</h2>\n");

    if !opts.include_resistance {
        out.push_str("  <div class=\"note-box\">Resistance analysis skipped (include_resistance is false).</div>\n");
    } else {
        match HoltropInput::measure(&hull, &h_opts) {
            None => {
                out.push_str("  <div class=\"note-box\">Resistance analysis skipped: hull parameters cannot be measured at this condition (hull may displace zero volume or lack a valid waterline).</div>\n");
            }
            Some(input) => {
                let warnings = input.warnings();
                if !warnings.is_empty() {
                    out.push_str("  <div class=\"warn-box\">\n");
                    out.push_str("    <strong>Validity range warnings:</strong>\n    <ul>\n");
                    for w in &warnings {
                        let _ = writeln!(out, "      <li>{}</li>", html_escape(w));
                    }
                    out.push_str("    </ul>\n");
                    out.push_str("  </div>\n");
                }

                let curve = resistance::curve(&input, &opts.speeds_kn);
                let res_title =
                    format!("{} - Total Resistance RT and Effective Power PE", esc_name);
                let res_svg = resistance_chart_svg(&curve, &res_title);
                out.push_str("  <div class=\"svg-frame\">\n");
                out.push_str(&res_svg);
                out.push_str("  </div>\n");

                out.push_str("  <table>\n");
                out.push_str("    <thead>\n");
                out.push_str("      <tr><th>Speed [kn]</th><th>Speed [m/s]</th><th>Fn</th><th>1+k1</th><th>RF [kN]</th><th>RW [kN]</th><th>RT [kN]</th><th>PE [kW]</th></tr>\n");
                out.push_str("    </thead>\n");
                out.push_str("    <tbody>\n");
                for r in &curve {
                    let _ = writeln!(
                        out,
                        "      <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                        fmt_num(r.speed_kn, 2),
                        fmt_num(r.speed_m_s, 2),
                        fmt_num(r.fn_, 3),
                        fmt_num(r.form_factor, 3),
                        fmt_num(r.rf_n / 1000.0, 2),
                        fmt_num(r.rw_n / 1000.0, 2),
                        fmt_num(r.rt_n / 1000.0, 2),
                        fmt_num(r.pe_w / 1000.0, 2),
                    );
                }
                out.push_str("    </tbody>\n");
                out.push_str("  </table>\n");
            }
        }
    }
    out.push_str("</section>\n\n");

    // ------------------------------------------------------------- 7. Planing (Savitsky, only if include_planing)
    if opts.include_planing {
        out.push_str("<section id=\"planing\">\n");
        out.push_str("  <h2>Planing Performance (Savitsky)</h2>\n");

        match planing::setup(surface, subdiv, &h_opts, None, opts.lcg_x, None) {
            Err(e) => {
                let _ = writeln!(
                    out,
                    "  <div class=\"note-box\">Planing prediction unavailable: {}</div>",
                    html_escape(&e)
                );
            }
            Ok(setup) => {
                if let Some(dead) = setup.deadrise_measured {
                    if dead < 5.0 {
                        let _ = writeln!(
                            out,
                            "  <div class=\"warn-box\">Measured deadrise ({}&deg;) is low. Savitsky's method applies to prismatic planing craft with deadrise 10&ndash;30&deg;.</div>",
                            fmt_num(dead, 1)
                        );
                    }
                }
                let pts = planing::planing_curve(&setup.hull, &opts.speeds_kn);
                if pts.is_empty() {
                    out.push_str("  <div class=\"note-box\">No planing equilibrium converged in the given speed range.</div>\n");
                } else {
                    let plan_title = format!("{} - Planing Resistance &amp; Trim", esc_name);
                    let plan_svg = planing_chart_svg(&pts, &plan_title);
                    out.push_str("  <div class=\"svg-frame\">\n");
                    out.push_str(&plan_svg);
                    out.push_str("  </div>\n");

                    out.push_str("  <table>\n");
                    out.push_str("    <thead>\n");
                    out.push_str("      <tr><th>Speed [kn]</th><th>Trim [&deg;]</th><th>Wetted Length [m]</th><th>LCP/b</th><th>Resistance [kN]</th><th>PE [kW]</th></tr>\n");
                    out.push_str("    </thead>\n");
                    out.push_str("    <tbody>\n");
                    for p in &pts {
                        let _ = writeln!(
                            out,
                            "      <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                            fmt_num(p.speed_kn, 1),
                            fmt_num(p.trim_deg, 2),
                            fmt_num(p.wetted_length_m, 2),
                            fmt_num(p.lcp_over_b, 3),
                            fmt_num(p.resistance_n / 1000.0, 2),
                            fmt_num(p.pe_w / 1000.0, 2),
                        );
                    }
                    out.push_str("    </tbody>\n");
                    out.push_str("  </table>\n");
                }
            }
        }
        out.push_str("</section>\n\n");
    }

    // ------------------------------------------------------------- 8. Loading Conditions
    let has_loading = match &project.report.loading_conditions {
        Some(names) => !names.is_empty(),
        None => !project.loading_conditions.is_empty(),
    };
    if has_loading {
        out.push_str("<section id=\"loading\">\n");
        out.push_str("  <h2>Loading Conditions</h2>\n");

        let cond_names: Vec<&str> = match &project.report.loading_conditions {
            Some(names) => names.iter().map(|s| s.as_str()).collect(),
            None => project
                .loading_conditions
                .iter()
                .map(|c| c.name.as_str())
                .collect(),
        };

        let inc_weather = project.report.include_weather.unwrap_or(true);

        for name in cond_names {
            let Some(orig_cond) = project.loading_conditions.iter().find(|c| c.name == name) else {
                let _ = writeln!(
                    out,
                    "  <div class=\"warn-box\">Loading condition <strong>{}</strong> not found in project.</div>",
                    html_escape(name)
                );
                continue;
            };

            let cond = project.with_windage(orig_cond);
            let esc_cond = html_escape(&cond.name);
            let _ = writeln!(out, "  <h3>{}</h3>", esc_cond);

            let res = loading::evaluate(surface, &h_opts, &cond);

            if !res.equilibrium_found {
                out.push_str("  <div class=\"warn-box\">\n");
                let _ = writeln!(
                    out,
                    "    <strong>Warning: {}.</strong>",
                    html_escape(loading::NO_EQUILIBRIUM_NOTE)
                );
                if !res.warnings.is_empty() {
                    out.push_str("    <ul>\n");
                    for w in &res.warnings {
                        let _ = writeln!(out, "      <li>{}</li>", html_escape(w));
                    }
                    out.push_str("    </ul>\n");
                }
                out.push_str("  </div>\n");
            }

            out.push_str("  <table>\n");
            out.push_str("    <thead>\n");
            out.push_str("      <tr><th>&Delta; [t]</th><th>LCG [m]</th><th>TCG [m]</th><th>KG solid [m]</th><th>FSC [m]</th><th>KG fluid [m]</th><th>Draft [m]</th><th>Trim [m]</th><th>List [&deg;]</th><th>GM fluid [m]</th></tr>\n");
            out.push_str("    </thead>\n");
            out.push_str("    <tbody>\n");
            let _ = writeln!(
                out,
                "      <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                fmt_num(res.displacement_t, 3),
                fmt_num(res.lcg, 3),
                fmt_num(res.tcg, 3),
                fmt_num(res.kg_solid, 3),
                fmt_num(res.fsc_m, 3),
                fmt_num(res.kg_fluid, 3),
                fmt_num(res.draft, 3),
                fmt_num(res.trim_m, 3),
                fmt_num(res.list_deg, 2),
                fmt_num(res.gm_fluid, 3),
            );
            out.push_str("    </tbody>\n");
            out.push_str("  </table>\n");

            // IMO A.749 Criteria
            out.push_str("  <h4>IMO A.749 / 2008 IS Code Intact Stability Criteria</h4>\n");
            out.push_str("  <table>\n");
            out.push_str("    <thead>\n");
            out.push_str("      <tr><th class=\"text-left\">Criterion</th><th>Required</th><th>Actual</th><th class=\"text-center\">Status</th></tr>\n");
            out.push_str("    </thead>\n");
            out.push_str("    <tbody>\n");
            if let Some(ref imo) = res.imo_criteria {
                let p_0_30 = imo.area_0_30_m_rad >= 0.055;
                let p_0_40 = imo.area_0_40_m_rad >= 0.090;
                let p_30_40 = imo.area_30_40_m_rad >= 0.030;
                let p_max_30 = imo.max_gz_30_plus_m >= 0.20;
                let p_angle = imo.angle_of_max_gz_deg >= 25.0;
                let p_gm0 = imo.gm0_m >= 0.15;

                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Area under GZ curve 0&deg; to 30&deg;</td><td>&ge; 0.055 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.area_0_30_m_rad, 4),
                    badge(p_0_30)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Area under GZ curve 0&deg; to 40&deg;</td><td>&ge; 0.090 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.area_0_40_m_rad, 4),
                    badge(p_0_40)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Area under GZ curve 30&deg; to 40&deg;</td><td>&ge; 0.030 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.area_30_40_m_rad, 4),
                    badge(p_30_40)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Maximum GZ at heel &ge; 30&deg;</td><td>&ge; 0.200 m</td><td>{} m</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.max_gz_30_plus_m, 3),
                    badge(p_max_30)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Angle of maximum GZ</td><td>&ge; 25.0&deg;</td><td>{}&deg;</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.angle_of_max_gz_deg, 1),
                    badge(p_angle)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Initial metacentric height GM0</td><td>&ge; 0.150 m</td><td>{} m</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.gm0_m, 3),
                    badge(p_gm0)
                );
                let _ = writeln!(
                    out,
                    "      <tr class=\"active-row\"><td class=\"text-left\"><strong>Overall Status</strong></td><td>All criteria met</td><td>Displ. {} t</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(imo.displacement_t, 3),
                    badge(imo.passed)
                );
            } else {
                out.push_str("      <tr><td class=\"text-left\">IMO Criteria Evaluation</td><td>&mdash;</td><td>Insufficient heel range or equilibrium data</td><td class=\"text-center\"><span class=\"badge-fail\">FAIL</span></td></tr>\n");
            }
            out.push_str("    </tbody>\n");
            out.push_str("  </table>\n");

            // Weather criterion
            if inc_weather {
                if let Some(ref weather_res) = res.weather {
                    out.push_str("  <h4>IMO 2008 IS Code Part A &sect;2.3 Severe Wind and Rolling (Weather) Criterion</h4>\n");
                    out.push_str("  <table>\n");
                    out.push_str("    <thead>\n");
                    out.push_str("      <tr><th class=\"text-left\">Parameter / Criterion</th><th>Requirement / Formula</th><th>Value</th><th class=\"text-center\">Status</th></tr>\n");
                    out.push_str("    </thead>\n");
                    out.push_str("    <tbody>\n");
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Lateral windage area A &amp; lever Z</td><td>P = {} Pa</td><td>A = {} m&sup2;, Z = {} m</td><td class=\"text-center\">&mdash;</td></tr>",
                        fmt_num(weather_res.wind_pressure_pa, 0),
                        fmt_num(weather_res.area_m2, 3),
                        fmt_num(weather_res.z_m, 3)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Wind heeling levers l<sub>w1</sub> / l<sub>w2</sub></td><td>l<sub>w2</sub> = 1.5 &middot; l<sub>w1</sub></td><td>{} m / {} m</td><td class=\"text-center\">&mdash;</td></tr>",
                        fmt_num(weather_res.lw1_m, 4),
                        fmt_num(weather_res.lw2_m, 4)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Roll factors (X<sub>1</sub>, X<sub>2</sub>, k, r, s) &amp; period T</td><td>T = 2&middot;C&middot;B / &radic;GM</td><td>({}, {}, {}, {}, {}), T = {} s</td><td class=\"text-center\">&mdash;</td></tr>",
                        fmt_num(weather_res.x1, 3),
                        fmt_num(weather_res.x2, 3),
                        fmt_num(weather_res.k, 3),
                        fmt_num(weather_res.r, 3),
                        fmt_num(weather_res.s, 4),
                        fmt_num(weather_res.roll_period_s, 2)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Roll-back angle &phi;<sub>1</sub></td><td>109&middot;k&middot;X<sub>1</sub>&middot;X<sub>2</sub>&middot;&radic;(r&middot;s)</td><td>{}&deg;</td><td class=\"text-center\">&mdash;</td></tr>",
                        fmt_num(weather_res.phi1_deg, 2)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Steady wind heel angle &phi;<sub>0</sub></td><td>&le; {}&deg;</td><td>{}&deg;</td><td class=\"text-center\">{}</td></tr>",
                        fmt_num(weather_res.phi0_limit_deg, 1),
                        fmt_opt(weather_res.phi0_deg, 2),
                        badge(weather_res.phi0_passed)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Integration limits (&phi;<sub>0</sub> &minus; &phi;<sub>1</sub>, &phi;<sub>c</sub>, &phi;<sub>2</sub>)</td><td>&phi;<sub>2</sub> = min(50&deg;, &phi;<sub>f</sub>, &phi;<sub>c2</sub>)</td><td>({}&deg;, {}&deg;, {}&deg;)</td><td class=\"text-center\">&mdash;</td></tr>",
                        fmt_opt(weather_res.phi_roll_deg, 2),
                        fmt_opt(weather_res.phic_deg, 2),
                        fmt_opt(weather_res.phi2_deg, 2)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">Energy balance (area b vs area a)</td><td>b &ge; a</td><td>a = {} m&middot;rad, b = {} m&middot;rad (b/a = {})</td><td class=\"text-center\">{}</td></tr>",
                        fmt_num(weather_res.area_a_m_rad, 4),
                        fmt_num(weather_res.area_b_m_rad, 4),
                        fmt_num(weather_res.area_ratio, 2),
                        badge(weather_res.area_passed)
                    );
                    let _ = writeln!(
                        out,
                        "      <tr class=\"active-row\"><td class=\"text-left\"><strong>Weather Criterion Overall</strong></td><td>&phi;<sub>0</sub> &le; 16&deg; and b &ge; a</td><td>l<sub>w1</sub> = {} m, l<sub>w2</sub> = {} m</td><td class=\"text-center\">{}</td></tr>",
                        fmt_num(weather_res.lw1_m, 4),
                        fmt_num(weather_res.lw2_m, 4),
                        badge(weather_res.passed)
                    );
                    out.push_str("    </tbody>\n");
                    out.push_str("  </table>\n");
                }
            }

            // GZ curve via gz_chart_svg
            let mut gz_points: Vec<(f64, Option<f64>)> = Vec::with_capacity(res.cross_curves.len());
            for c in &res.cross_curves {
                let gz = c.points.first().and_then(|p| p.gz);
                gz_points.push((c.heel_deg, gz));
            }
            let gz_title = format!(
                "{} - Righting Lever GZ (Displ. {} t, KG fluid {} m)",
                esc_cond,
                fmt_num(res.displacement_t, 3),
                fmt_num(res.kg_fluid, 3)
            );
            let gz_svg = gz_chart_svg(&gz_points, res.imo_criteria.as_ref(), &gz_title);
            out.push_str("  <div class=\"svg-frame\">\n");
            out.push_str(&gz_svg);
            out.push_str("  </div>\n");
        }
        out.push_str("</section>\n\n");
    }

    // ------------------------------------------------------------- 9. Damage Cases
    let has_damage = match &project.report.damage_cases {
        Some(names) => !names.is_empty(),
        None => !project.damage_cases.is_empty(),
    };
    if has_damage {
        out.push_str("<section id=\"damage\">\n");
        out.push_str("  <h2>Damage Cases</h2>\n");

        let case_names: Vec<&str> = match &project.report.damage_cases {
            Some(names) => names.iter().map(|s| s.as_str()).collect(),
            None => project
                .damage_cases
                .iter()
                .map(|c| c.name.as_str())
                .collect(),
        };

        for name in case_names {
            let Some(raw_case) = project.damage_cases.iter().find(|c| c.name == name) else {
                let _ = writeln!(
                    out,
                    "  <div class=\"warn-box\">Damage case <strong>{}</strong> not found in project.</div>",
                    html_escape(name)
                );
                continue;
            };

            let esc_case = html_escape(&raw_case.name);
            let _ = writeln!(out, "  <h3>{}</h3>", esc_case);

            let resolved = match project.resolve_damage_case(raw_case) {
                Ok(r) => r,
                Err(e) => {
                    let _ = writeln!(
                        out,
                        "  <div class=\"warn-box\">Cannot resolve damage case: {}</div>",
                        html_escape(&e)
                    );
                    continue;
                }
            };

            let res = match damage::evaluate(surface, &h_opts, &resolved) {
                Ok(r) => r,
                Err(e) => {
                    let _ = writeln!(
                        out,
                        "  <div class=\"warn-box\">Damage evaluation error: {}</div>",
                        html_escape(&e)
                    );
                    continue;
                }
            };

            for w in &res.warnings {
                let _ = writeln!(out, "  <div class=\"warn-box\">{}</div>", html_escape(w));
            }

            // Intact vs damaged draft/trim/heel, sinkage, freeboard
            out.push_str("  <h4>Equilibrium Floating Condition</h4>\n");
            out.push_str("  <table>\n");
            out.push_str("    <thead>\n");
            out.push_str("      <tr><th class=\"text-left\">Condition</th><th>Draft [m]</th><th>Trim [m]</th><th>Heel [&deg;]</th><th>Sinkage [m]</th><th>Freeboard [m]</th></tr>\n");
            out.push_str("    </thead>\n");
            out.push_str("    <tbody>\n");
            let _ = writeln!(
                out,
                "      <tr><td class=\"text-left\">Intact</td><td>{}</td><td>{}</td><td>0.00</td><td>&mdash;</td><td>&mdash;</td></tr>",
                fmt_num(res.intact_draft, 3),
                fmt_num(res.intact_trim_m, 3)
            );
            let _ = writeln!(
                out,
                "      <tr class=\"active-row\"><td class=\"text-left\">Damaged</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                fmt_num(res.draft, 3),
                fmt_num(res.trim_m, 3),
                fmt_num(res.heel_deg, 2),
                fmt_num(res.sinkage_m, 3),
                fmt_opt(res.freeboard_m, 3)
            );
            out.push_str("    </tbody>\n");
            out.push_str("  </table>\n");

            // Flooded compartments table
            out.push_str("  <h4>Flooded Compartments</h4>\n");
            out.push_str("  <table>\n");
            out.push_str("    <thead>\n");
            out.push_str("      <tr><th class=\"text-left\">Compartment</th><th>Permeability &mu;</th><th>Submerged Vol [m&sup3;]</th><th>Lost Buoyancy [m&sup3;]</th><th>Centroid X [m]</th><th>Centroid Y [m]</th><th>Centroid Z [m]</th></tr>\n");
            out.push_str("    </thead>\n");
            out.push_str("    <tbody>\n");
            if res.compartments.is_empty() {
                out.push_str("      <tr><td colspan=\"7\" class=\"text-center\">No compartments flooded</td></tr>\n");
            } else {
                for c in &res.compartments {
                    let _ = writeln!(
                        out,
                        "      <tr><td class=\"text-left\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                        html_escape(&c.name),
                        fmt_num(c.permeability, 2),
                        fmt_num(c.submerged_volume_m3, 3),
                        fmt_num(c.lost_volume_m3, 3),
                        fmt_num(c.centroid.x, 3),
                        fmt_num(c.centroid.y, 3),
                        fmt_num(c.centroid.z, 3),
                    );
                }
            }
            out.push_str("    </tbody>\n");
            out.push_str("  </table>\n");

            // Residual criteria PASS/FAIL
            out.push_str("  <h4>Residual Stability Criteria (SOLAS II-1 Reg. 7-2)</h4>\n");
            out.push_str("  <table>\n");
            out.push_str("    <thead>\n");
            out.push_str("      <tr><th class=\"text-left\">Criterion</th><th>Required</th><th>Actual</th><th class=\"text-center\">Status</th></tr>\n");
            out.push_str("    </thead>\n");
            out.push_str("    <tbody>\n");
            if let Some(ref crit) = res.criteria {
                let p_range = crit.positive_range_deg >= 15.0;
                let p_max = crit.max_gz_m >= 0.10;
                let p_area = crit.area_m_rad >= 0.015;
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Positive righting lever range</td><td>&ge; 15.0&deg;</td><td>{}&deg;</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(crit.positive_range_deg, 1),
                    badge(p_range)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Maximum residual righting lever GZ</td><td>&ge; 0.100 m</td><td>{} m</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(crit.max_gz_m, 3),
                    badge(p_max)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Angle of maximum residual GZ</td><td>&mdash;</td><td>{}&deg;</td><td class=\"text-center\">&mdash;</td></tr>",
                    fmt_num(crit.angle_of_max_gz_deg, 1)
                );
                let _ = writeln!(
                    out,
                    "      <tr><td class=\"text-left\">Area under residual GZ curve</td><td>&ge; 0.0150 m&middot;rad</td><td>{} m&middot;rad</td><td class=\"text-center\">{}</td></tr>",
                    fmt_num(crit.area_m_rad, 4),
                    badge(p_area)
                );
                let _ = writeln!(
                    out,
                    "      <tr class=\"active-row\"><td class=\"text-left\"><strong>Overall Residual Status</strong></td><td>All criteria met</td><td>{}</td><td class=\"text-center\">{}</td></tr>",
                    html_escape(&crit.label),
                    badge(crit.passed)
                );
            } else {
                out.push_str("      <tr><td class=\"text-left\">Residual Criteria Evaluation</td><td>&mdash;</td><td>Damaged equilibrium not found or criteria not evaluated</td><td class=\"text-center\"><span class=\"badge-fail\">FAIL</span></td></tr>\n");
            }
            out.push_str("    </tbody>\n");
            out.push_str("  </table>\n");

            // Residual GZ chart
            let gz_pts: Vec<(f64, Option<f64>)> = res
                .residual_gz
                .iter()
                .map(|p| (p.relative_heel_deg, p.gz))
                .collect();
            let gz_title = format!(
                "{} - Residual Righting Levers (Damaged Equilibrium)",
                esc_case
            );
            let gz_svg = gz_chart_svg(&gz_pts, None, &gz_title);
            out.push_str("  <div class=\"svg-frame\">\n");
            out.push_str(&gz_svg);
            out.push_str("  </div>\n");
        }
        out.push_str("</section>\n\n");
    }

    // ------------------------------------------------------------- 10. Seakeeping
    if let Some(ref sk_conf) = project.report.seakeeping {
        out.push_str("<section id=\"seakeeping\">\n");
        out.push_str("  <h2>Seakeeping</h2>\n");

        let kg = sk_conf.kg.unwrap_or(opts.kg);
        let mut spec = SeakeepingSpec {
            draft: opts.draft,
            speed_kn: sk_conf.speed_kn,
            kg,
            stations: 21,
            ..Default::default()
        };
        let irr_spec = sk_conf.sea.map(|sea| IrregularSpec::new(sea, spec.clone()));
        if let Some(ref irr) = irr_spec {
            spec.omegas = irr.omega_grid();
        }

        match seakeeping::head_sea_raos(surface, &spec) {
            Err(e) => {
                let _ = writeln!(
                    out,
                    "  <div class=\"warn-box\">Seakeeping calculation error: {}</div>",
                    html_escape(&e)
                );
            }
            Ok(rep) => {
                let _ = writeln!(
                    out,
                    "  <h3>Head Sea Motion RAOs (Speed: {} kn, Draft: {} m, KG: {} m)</h3>",
                    fmt_num(sk_conf.speed_kn, 1),
                    fmt_num(opts.draft, 3),
                    fmt_num(kg, 3)
                );

                out.push_str("  <table>\n");
                out.push_str("    <thead>\n");
                out.push_str("      <tr><th>&lambda;/L</th><th>Wave &omega; [rad/s]</th><th>&lambda; [m]</th><th>Heave Amp [m/m]</th><th>Heave Phase [&deg;]</th><th>Pitch Amp [rad/rad]</th><th>Pitch Phase [&deg;]</th></tr>\n");
                out.push_str("    </thead>\n");
                out.push_str("    <tbody>\n");
                for p in &rep.frequencies {
                    let _ = writeln!(
                        out,
                        "      <tr><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>",
                        fmt_num(p.lambda_over_l, 3),
                        fmt_num(p.omega, 3),
                        fmt_num(p.wavelength_m, 2),
                        fmt_num(p.heave_amp, 4),
                        fmt_num(p.heave_phase_deg, 1),
                        fmt_num(p.pitch_amp, 4),
                        fmt_num(p.pitch_phase_deg, 1),
                    );
                }
                out.push_str("    </tbody>\n");
                out.push_str("  </table>\n");

                if let Some(ref irr) = irr_spec {
                    let sea = irr.sea;
                    match seastate::irregular_from_raos(&rep, irr) {
                        Err(e) => {
                            let _ = writeln!(
                                out,
                                "  <div class=\"warn-box\">Irregular sea response error: {}</div>",
                                html_escape(&e)
                            );
                        }
                        Ok(irr) => {
                            out.push_str("  <h3>Irregular Head Sea Responses</h3>\n");
                            out.push_str("  <div class=\"cards-grid\">\n");

                            out.push_str("    <div class=\"card\">\n");
                            out.push_str("      <h3>Wave Spectrum</h3>\n");
                            out.push_str("      <dl>\n");
                            let spec_type = match sea {
                                WaveSpectrum::Bretschneider { .. } => "Bretschneider",
                                WaveSpectrum::Jonswap { .. } => "JONSWAP",
                            };
                            let _ =
                                writeln!(out, "        <dt>Spectrum</dt><dd>{}</dd>", spec_type);
                            let _ = writeln!(
                                out,
                                "        <dt>Target Hs</dt><dd>{} m</dd>",
                                fmt_num(sea.hs(), 2)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Peak Period Tp</dt><dd>{} s</dd>",
                                fmt_num(sea.tp(), 2)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Spectrum Hs Check</dt><dd>{} m</dd>",
                                fmt_num(irr.spectrum.hs_check, 2)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Peak &omega;p</dt><dd>{} rad/s</dd>",
                                fmt_num(irr.spectrum.peak_omega, 3)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Zero-Crossing Tz</dt><dd>{} s</dd>",
                                fmt_num(irr.spectrum.tz_s, 2)
                            );
                            out.push_str("      </dl>\n");
                            out.push_str("    </div>\n");

                            out.push_str("    <div class=\"card\">\n");
                            out.push_str("      <h3>Ship Responses</h3>\n");
                            out.push_str("      <dl>\n");
                            let _ = writeln!(
                                out,
                                "        <dt>Heave Significant</dt><dd>{} m</dd>",
                                fmt_num(irr.heave.significant, 3)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Heave RMS</dt><dd>{} m</dd>",
                                fmt_num(irr.heave.rms, 3)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Pitch Significant</dt><dd>{}&deg;</dd>",
                                fmt_num(irr.pitch_deg.significant, 2)
                            );
                            let _ = writeln!(
                                out,
                                "        <dt>Pitch RMS</dt><dd>{}&deg;</dd>",
                                fmt_num(irr.pitch_deg.rms, 2)
                            );
                            out.push_str("      </dl>\n");
                            out.push_str("    </div>\n");

                            out.push_str("  </div>\n");

                            out.push_str("  <h4>Point Responses &amp; Motion Sickness</h4>\n");
                            out.push_str("  <table>\n");
                            out.push_str("    <thead>\n");
                            out.push_str("      <tr><th class=\"text-left\">Point</th><th>Position X [m]</th><th>Vertical Motion Sig. [m]</th><th>Relative Motion Sig. [m]</th><th>Vert. Accel. RMS [g]</th><th>MSI [% (2 h)]</th></tr>\n");
                            out.push_str("    </thead>\n");
                            out.push_str("    <tbody>\n");
                            for pt in &irr.points {
                                let _ = writeln!(
                                    out,
                                    "      <tr><td class=\"text-left\">{}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td><td>{} %</td></tr>",
                                    html_escape(&pt.name),
                                    fmt_num(pt.x, 3),
                                    fmt_num(pt.vertical_motion.significant, 3),
                                    fmt_num(pt.relative_motion.significant, 3),
                                    fmt_num(pt.acceleration_rms_g, 3),
                                    fmt_num(pt.msi_percent, 1),
                                );
                            }
                            out.push_str("    </tbody>\n");
                            out.push_str("  </table>\n");

                            let _ = writeln!(
                                out,
                                "  <div class=\"note-box\">{}</div>\n",
                                html_escape(irr.msi_note)
                            );
                        }
                    }
                }
            }
        }
        out.push_str("</section>\n\n");
    }

    // ------------------------------------------------------------- Footer
    let version = env!("CARGO_PKG_VERSION");
    out.push_str("<footer>\n");
    let _ = writeln!(
        out,
        "  <p>SeaCraft v{} &bull; Design Dossier &bull; Subdivision Level {}, Mode {:?}</p>",
        version, opts.level, opts.mode
    );
    out.push_str("  <p>Units: Metres (m), Tonnes (t = 1000 kg), Kilograms per cubic metre (kg/m&sup3;), Degrees (&deg;), Knots (kn), Kilonewtons (kN), Kilowatts (kW).</p>\n");
    out.push_str("</footer>\n\n");

    out.push_str("</body>\n</html>\n");
    Ok(out)
}

/// Escape text safely for embedding in HTML.
pub fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

/// Format a finite float with fixed precision, or emit "—" if not finite.
fn fmt_num(v: f64, decimals: usize) -> String {
    if v.is_finite() {
        let text = format!("{:.prec$}", v, prec = decimals);
        // A tiny negative value rounds to "-0.000"; print it as zero.
        if text.starts_with('-') && text[1..].chars().all(|c| c == '0' || c == '.') {
            return text[1..].to_string();
        }
        text
    } else {
        "—".to_string()
    }
}

/// Format an optional finite float with fixed precision, or emit "—".
fn fmt_opt(v: Option<f64>, decimals: usize) -> String {
    match v {
        Some(x) => fmt_num(x, decimals),
        None => "—".to_string(),
    }
}

/// Strip `<?xml ... ?>` and any leading bytes before the first `<svg`.
fn strip_xml_header(svg: &str) -> &str {
    if let Some(pos) = svg.find("<svg") {
        &svg[pos..]
    } else {
        svg
    }
}

/// Generate an inline SVG chart of the GZ righting arm versus heel angle.
fn gz_chart_svg(points: &[(f64, Option<f64>)], imo: Option<&ImoCriteria>, title: &str) -> String {
    let w = 900.0;
    let h = 450.0;
    let (ml, mr, mt, mb) = (70.0, 30.0, 40.0, 50.0);

    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">"
    );
    let _ = writeln!(out, "<title>{}</title>", html_escape(title));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<style>\n");
    out.push_str(".axis { stroke: #111; stroke-width: 0.9; fill: none; }\n");
    out.push_str(
        ".grid { stroke: #e0e0e0; stroke-width: 0.6; stroke-dasharray: 4 4; fill: none; }\n",
    );
    out.push_str(".zero-line { stroke: #666; stroke-width: 1.0; fill: none; }\n");
    out.push_str(
        ".crit-line { stroke: #cf222e; stroke-width: 0.8; stroke-dasharray: 2 3; fill: none; }\n",
    );
    out.push_str(".curve { fill: none; stroke: #1f5fa8; stroke-width: 2.2; }\n");
    out.push_str(".curve-fill { fill: #1f5fa8; fill-opacity: 0.12; stroke: none; }\n");
    out.push_str(".label { font: 11px ui-sans-serif, system-ui, sans-serif; fill: #444; }\n");
    out.push_str(".title { font: 14px ui-sans-serif, system-ui, sans-serif; font-weight: 600; fill: #111; }\n");
    out.push_str("</style>\n");

    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{ml:.0}\" y=\"24\">{}</text>",
        html_escape(title)
    );

    let valid: Vec<(f64, f64)> = points
        .iter()
        .filter_map(|&(heel, gz)| gz.filter(|g| g.is_finite()).map(|g| (heel, g)))
        .collect();

    if valid.len() < 2 {
        let _ = writeln!(out, "<text class=\"label\" x=\"{ml:.0}\" y=\"80\">Insufficient stability points to draw curve</text></svg>\n");
        return out;
    }

    let x_max = points
        .iter()
        .map(|&(h, _)| h)
        .fold(60.0_f64, f64::max)
        .max(60.0);
    let max_gz = valid.iter().map(|&(_, g)| g).fold(0.0_f64, f64::max);
    let min_gz = valid.iter().map(|&(_, g)| g).fold(0.0_f64, f64::min);

    let y_max = (max_gz * 1.15).max(0.25);
    let y_min = if min_gz < -0.02 { min_gz * 1.15 } else { 0.0 };

    let sx = |h: f64| ml + (h / x_max) * (w - ml - mr);
    let sy = |g: f64| mt + (1.0 - (g - y_min) / (y_max - y_min)) * (h - mt - mb);

    // Frame
    let _ = writeln!(
        out,
        "<rect class=\"axis\" x=\"{ml:.1}\" y=\"{mt:.1}\" width=\"{:.1}\" height=\"{:.1}\"/>",
        w - ml - mr,
        h - mt - mb
    );

    // Horizontal grid & ticks
    let y_ticks = 5;
    for k in 0..=y_ticks {
        let val = y_min + (y_max - y_min) * k as f64 / y_ticks as f64;
        let y = sy(val);
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{ml:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            w - mr
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{val:.2}</text>",
            ml - 8.0,
            y + 4.0
        );
    }

    // Baseline GZ = 0
    if (y_min..=y_max).contains(&0.0) {
        let y0 = sy(0.0);
        let _ = writeln!(
            out,
            "<line class=\"zero-line\" x1=\"{ml:.1}\" y1=\"{y0:.1}\" x2=\"{:.1}\" y2=\"{y0:.1}\"/>",
            w - mr
        );
    }

    // Vertical grid & ticks (every 15 deg)
    let mut heel_tick = 0.0;
    while heel_tick <= x_max + 1e-6 {
        let x = sx(heel_tick);
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{mt:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            h - mb
        );
        let _ = writeln!(out, "<text class=\"label\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{:.0}&deg;</text>", h - mb + 18.0, heel_tick);
        heel_tick += 15.0;
    }

    // Mark IMO critical angles: 30 deg and 40 deg
    if x_max >= 40.0 {
        let x30 = sx(30.0);
        let x40 = sx(40.0);
        let _ = writeln!(out, "<line class=\"crit-line\" x1=\"{x30:.1}\" y1=\"{mt:.1}\" x2=\"{x30:.1}\" y2=\"{:.1}\"/>", h - mb);
        let _ = writeln!(out, "<line class=\"crit-line\" x1=\"{x40:.1}\" y1=\"{mt:.1}\" x2=\"{x40:.1}\" y2=\"{:.1}\"/>", h - mb);
        let _ = writeln!(out, "<text class=\"label\" x=\"{x30:.1}\" y=\"{:.1}\" text-anchor=\"middle\" fill=\"#cf222e\">30&deg;</text>", mt - 6.0);
        let _ = writeln!(out, "<text class=\"label\" x=\"{x40:.1}\" y=\"{:.1}\" text-anchor=\"middle\" fill=\"#cf222e\">40&deg;</text>", mt - 6.0);
    }

    // Shaded fill under the positive curve. Where GZ changes sign the fill
    // meets the axis at the interpolated crossing, not at the next sample.
    let y0 = sy(0.0_f64.max(y_min));
    let mut fill_path = format!("M {:.1},{:.1} ", sx(valid[0].0), y0);
    for (i, &(h, g)) in valid.iter().enumerate() {
        if i > 0 {
            let (hp, gp) = valid[i - 1];
            if (gp > 0.0) != (g > 0.0) && gp != g {
                let hz = hp + (h - hp) * gp / (gp - g);
                let _ = write!(fill_path, "L {:.1},{:.1} ", sx(hz), y0);
            }
        }
        let _ = write!(fill_path, "L {:.1},{:.1} ", sx(h), sy(g.max(0.0)));
    }
    let _ = write!(
        fill_path,
        "L {:.1},{:.1} Z",
        sx(valid.last().unwrap().0),
        y0
    );
    let _ = writeln!(out, "<path class=\"curve-fill\" d=\"{}\"/>", fill_path);

    // GZ curve polyline
    let mut curve_path = format!("M {:.1},{:.1} ", sx(valid[0].0), sy(valid[0].1));
    for &(h, g) in &valid[1..] {
        let _ = write!(curve_path, "L {:.1},{:.1} ", sx(h), sy(g));
    }
    let _ = writeln!(out, "<path class=\"curve\" d=\"{}\"/>", curve_path);

    // Points
    for &(h, g) in &valid {
        let _ = writeln!(
            out,
            "<circle cx=\"{:.1}\" cy=\"{:.1}\" r=\"4\" fill=\"#1f5fa8\" stroke=\"#ffffff\" stroke-width=\"1.5\"><title>Heel: {:.1}&deg;, GZ: {} m</title></circle>",
            sx(h), sy(g), h, fmt_num(g, 3)
        );
    }

    // Annotate angle of maximum GZ if IMO is present
    if let Some(im) = imo {
        if im.angle_of_max_gz_deg <= x_max && im.max_gz_30_plus_m.is_finite() {
            let x_peak = sx(im.angle_of_max_gz_deg);
            let y_peak = sy(valid.iter().map(|&(_, g)| g).fold(0.0_f64, f64::max));
            let _ = writeln!(
                out,
                "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-weight=\"bold\" fill=\"#1f5fa8\">Peak {:.3} m @ {:.1}&deg;</text>",
                x_peak, y_peak - 10.0, valid.iter().map(|&(_, g)| g).fold(0.0_f64, f64::max), im.angle_of_max_gz_deg
            );
        }
    }

    // Axis labels
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-weight=\"bold\">Heel Angle [&deg;]</text>", ml + (w - ml - mr) / 2.0, h - 12.0);
    let _ = writeln!(
        out,
        "<text class=\"label\" transform=\"rotate(-90)\" x=\"{:.1}\" y=\"20\" text-anchor=\"middle\" font-weight=\"bold\">Righting Lever GZ [m]</text>",
        -(mt + (h - mt - mb) / 2.0)
    );

    out.push_str("</svg>\n");
    out
}

/// Lowest and highest finite speed, or `(0, fallback)` when there is none.
fn speed_range(speeds: impl Iterator<Item = f64>, fallback: f64) -> (f64, f64) {
    let (lo, hi) = speeds
        .filter(|v| v.is_finite())
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| {
            (lo.min(v), hi.max(v))
        });
    if lo <= hi {
        (lo, hi)
    } else {
        (0.0, fallback)
    }
}

/// Generate an inline SVG dual-axis chart of Resistance (RT) and Effective Power (PE) vs Speed.
fn resistance_chart_svg(res: &[Resistance], title: &str) -> String {
    let w = 900.0;
    let h = 450.0;
    let (ml, mr, mt, mb) = (70.0, 70.0, 40.0, 50.0);

    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">"
    );
    let _ = writeln!(out, "<title>{}</title>", html_escape(title));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<style>\n");
    out.push_str(".axis { stroke: #111; stroke-width: 0.9; fill: none; }\n");
    out.push_str(
        ".grid { stroke: #e0e0e0; stroke-width: 0.6; stroke-dasharray: 4 4; fill: none; }\n",
    );
    out.push_str(".rt-curve { fill: none; stroke: #0969da; stroke-width: 2.2; }\n");
    out.push_str(
        ".pe-curve { fill: none; stroke: #cf222e; stroke-width: 2.2; stroke-dasharray: 5 3; }\n",
    );
    out.push_str(".label { font: 11px ui-sans-serif, system-ui, sans-serif; fill: #444; }\n");
    out.push_str(".title { font: 14px ui-sans-serif, system-ui, sans-serif; font-weight: 600; fill: #111; }\n");
    out.push_str("</style>\n");

    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{ml:.0}\" y=\"24\">{}</text>",
        html_escape(title)
    );

    if res.len() < 2 {
        let _ = writeln!(out, "<text class=\"label\" x=\"{ml:.0}\" y=\"80\">Insufficient speed points to draw curve</text></svg>\n");
        return out;
    }

    // Speeds may come in any order: the axis spans their range.
    let (x0, x1) = speed_range(res.iter().map(|r| r.speed_kn), 10.0);
    let x_span = (x1 - x0).max(1.0);

    let rt_max = res.iter().map(|r| r.rt_n / 1000.0).fold(0.0_f64, f64::max) * 1.15;
    let rt_max = rt_max.max(0.1);
    let pe_max = res.iter().map(|r| r.pe_w / 1000.0).fold(0.0_f64, f64::max) * 1.15;
    let pe_max = pe_max.max(0.1);

    let sx = |kn: f64| ml + ((kn - x0) / x_span) * (w - ml - mr);
    let sy_rt = |rt: f64| mt + (1.0 - rt / rt_max) * (h - mt - mb);
    let sy_pe = |pe: f64| mt + (1.0 - pe / pe_max) * (h - mt - mb);

    // Frame
    let _ = writeln!(
        out,
        "<rect class=\"axis\" x=\"{ml:.1}\" y=\"{mt:.1}\" width=\"{:.1}\" height=\"{:.1}\"/>",
        w - ml - mr,
        h - mt - mb
    );

    // Ticks & Grid
    let n_ticks = 5;
    for k in 0..=n_ticks {
        let frac = k as f64 / n_ticks as f64;
        let rt_val = rt_max * frac;
        let pe_val = pe_max * frac;
        let y = mt + (1.0 - frac) * (h - mt - mb);

        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{ml:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            w - mr
        );
        let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#0969da\">{rt_val:.1}</text>", ml - 8.0, y + 4.0);
        let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"start\" fill=\"#cf222e\">{pe_val:.1}</text>", w - mr + 8.0, y + 4.0);
    }

    // X Ticks
    for r in res {
        let x = sx(r.speed_kn);
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{mt:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            h - mb
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{:.1}</text>",
            h - mb + 18.0,
            r.speed_kn
        );
    }

    // RT polyline
    let mut rt_path = format!(
        "M {:.1},{:.1} ",
        sx(res[0].speed_kn),
        sy_rt(res[0].rt_n / 1000.0)
    );
    for r in &res[1..] {
        let _ = write!(
            rt_path,
            "L {:.1},{:.1} ",
            sx(r.speed_kn),
            sy_rt(r.rt_n / 1000.0)
        );
    }
    let _ = writeln!(out, "<path class=\"rt-curve\" d=\"{}\"/>", rt_path);

    // PE polyline
    let mut pe_path = format!(
        "M {:.1},{:.1} ",
        sx(res[0].speed_kn),
        sy_pe(res[0].pe_w / 1000.0)
    );
    for r in &res[1..] {
        let _ = write!(
            pe_path,
            "L {:.1},{:.1} ",
            sx(r.speed_kn),
            sy_pe(r.pe_w / 1000.0)
        );
    }
    let _ = writeln!(out, "<path class=\"pe-curve\" d=\"{}\"/>", pe_path);

    // Points
    for r in res {
        let x = sx(r.speed_kn);
        let y_rt = sy_rt(r.rt_n / 1000.0);
        let y_pe = sy_pe(r.pe_w / 1000.0);
        let _ = writeln!(
            out,
            "<circle cx=\"{x:.1}\" cy=\"{y_rt:.1}\" r=\"4\" fill=\"#0969da\" stroke=\"#fff\" stroke-width=\"1.5\"><title>{:.1} kn: RT = {:.2} kN</title></circle>",
            r.speed_kn, r.rt_n / 1000.0
        );
        let _ = writeln!(
            out,
            "<circle cx=\"{x:.1}\" cy=\"{y_pe:.1}\" r=\"4\" fill=\"#cf222e\" stroke=\"#fff\" stroke-width=\"1.5\"><title>{:.1} kn: PE = {:.2} kW</title></circle>",
            r.speed_kn, r.pe_w / 1000.0
        );
    }

    // Legend
    let leg_x = w - mr - 200.0;
    let leg_y = mt + 15.0;
    let _ = writeln!(out, "<line x1=\"{leg_x:.1}\" y1=\"{leg_y:.1}\" x2=\"{:.1}\" y2=\"{leg_y:.1}\" stroke=\"#0969da\" stroke-width=\"2.5\"/>", leg_x + 24.0);
    let _ = writeln!(
        out,
        "<circle cx=\"{:.1}\" cy=\"{leg_y:.1}\" r=\"3.5\" fill=\"#0969da\"/>",
        leg_x + 12.0
    );
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" fill=\"#0969da\" font-weight=\"bold\">RT Total Resistance [kN]</text>", leg_x + 30.0, leg_y + 4.0);

    let leg_y2 = leg_y + 20.0;
    let _ = writeln!(out, "<line x1=\"{leg_x:.1}\" y1=\"{leg_y2:.1}\" x2=\"{:.1}\" y2=\"{leg_y2:.1}\" stroke=\"#cf222e\" stroke-width=\"2.5\" stroke-dasharray=\"5 3\"/>", leg_x + 24.0);
    let _ = writeln!(
        out,
        "<circle cx=\"{:.1}\" cy=\"{leg_y2:.1}\" r=\"3.5\" fill=\"#cf222e\"/>",
        leg_x + 12.0
    );
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" fill=\"#cf222e\" font-weight=\"bold\">PE Effective Power [kW]</text>", leg_x + 30.0, leg_y2 + 4.0);

    // Axis labels
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-weight=\"bold\">Speed [knots]</text>", ml + (w - ml - mr) / 2.0, h - 12.0);
    let _ = writeln!(
        out,
        "<text class=\"label\" transform=\"rotate(-90)\" x=\"{:.1}\" y=\"20\" text-anchor=\"middle\" font-weight=\"bold\" fill=\"#0969da\">Total Resistance RT [kN]</text>",
        -(mt + (h - mt - mb) / 2.0)
    );
    let _ = writeln!(
        out,
        "<text class=\"label\" transform=\"rotate(90)\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-weight=\"bold\" fill=\"#cf222e\">Effective Power PE [kW]</text>",
        mt + (h - mt - mb) / 2.0,
        -(w - 20.0)
    );

    out.push_str("</svg>\n");
    out
}

/// Generate an inline SVG chart for Savitsky planing results.
fn planing_chart_svg(points: &[PlaningPoint], title: &str) -> String {
    let w = 900.0;
    let h = 450.0;
    let (ml, mr, mt, mb) = (70.0, 70.0, 40.0, 50.0);

    let mut out = String::with_capacity(4096);
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">"
    );
    let _ = writeln!(out, "<title>{}</title>", html_escape(title));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<style>\n");
    out.push_str(".axis { stroke: #111; stroke-width: 0.9; fill: none; }\n");
    out.push_str(
        ".grid { stroke: #e0e0e0; stroke-width: 0.6; stroke-dasharray: 4 4; fill: none; }\n",
    );
    out.push_str(".res-curve { fill: none; stroke: #0969da; stroke-width: 2.2; }\n");
    out.push_str(
        ".trim-curve { fill: none; stroke: #d97706; stroke-width: 2.2; stroke-dasharray: 5 3; }\n",
    );
    out.push_str(".label { font: 11px ui-sans-serif, system-ui, sans-serif; fill: #444; }\n");
    out.push_str(".title { font: 14px ui-sans-serif, system-ui, sans-serif; font-weight: 600; fill: #111; }\n");
    out.push_str("</style>\n");

    let _ = writeln!(
        out,
        "<text class=\"title\" x=\"{ml:.0}\" y=\"24\">{}</text>",
        html_escape(title)
    );

    if points.len() < 2 {
        let _ = writeln!(out, "<text class=\"label\" x=\"{ml:.0}\" y=\"80\">Insufficient speed points to draw curve</text></svg>\n");
        return out;
    }

    let (x0, x1) = speed_range(points.iter().map(|p| p.speed_kn), 30.0);
    let x_span = (x1 - x0).max(1.0);

    let r_max = points
        .iter()
        .map(|p| p.resistance_n / 1000.0)
        .fold(0.0_f64, f64::max)
        * 1.15;
    let r_max = r_max.max(0.1);
    let trim_max = points.iter().map(|p| p.trim_deg).fold(0.0_f64, f64::max) * 1.25;
    let trim_max = trim_max.max(1.0);

    let sx = |kn: f64| ml + ((kn - x0) / x_span) * (w - ml - mr);
    let sy_r = |r: f64| mt + (1.0 - r / r_max) * (h - mt - mb);
    let sy_t = |t: f64| mt + (1.0 - t / trim_max) * (h - mt - mb);

    let _ = writeln!(
        out,
        "<rect class=\"axis\" x=\"{ml:.1}\" y=\"{mt:.1}\" width=\"{:.1}\" height=\"{:.1}\"/>",
        w - ml - mr,
        h - mt - mb
    );

    let n_ticks = 5;
    for k in 0..=n_ticks {
        let frac = k as f64 / n_ticks as f64;
        let r_val = r_max * frac;
        let t_val = trim_max * frac;
        let y = mt + (1.0 - frac) * (h - mt - mb);

        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{ml:.1}\" y1=\"{y:.1}\" x2=\"{:.1}\" y2=\"{y:.1}\"/>",
            w - mr
        );
        let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\" fill=\"#0969da\">{r_val:.1}</text>", ml - 8.0, y + 4.0);
        let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"start\" fill=\"#d97706\">{t_val:.1}&deg;</text>", w - mr + 8.0, y + 4.0);
    }

    for p in points {
        let x = sx(p.speed_kn);
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{x:.1}\" y1=\"{mt:.1}\" x2=\"{x:.1}\" y2=\"{:.1}\"/>",
            h - mb
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{x:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{:.0}</text>",
            h - mb + 18.0,
            p.speed_kn
        );
    }

    // Resistance path
    let mut r_path = format!(
        "M {:.1},{:.1} ",
        sx(points[0].speed_kn),
        sy_r(points[0].resistance_n / 1000.0)
    );
    for p in &points[1..] {
        let _ = write!(
            r_path,
            "L {:.1},{:.1} ",
            sx(p.speed_kn),
            sy_r(p.resistance_n / 1000.0)
        );
    }
    let _ = writeln!(out, "<path class=\"res-curve\" d=\"{}\"/>", r_path);

    // Trim path
    let mut t_path = format!(
        "M {:.1},{:.1} ",
        sx(points[0].speed_kn),
        sy_t(points[0].trim_deg)
    );
    for p in &points[1..] {
        let _ = write!(t_path, "L {:.1},{:.1} ", sx(p.speed_kn), sy_t(p.trim_deg));
    }
    let _ = writeln!(out, "<path class=\"trim-curve\" d=\"{}\"/>", t_path);

    // Points
    for p in points {
        let x = sx(p.speed_kn);
        let y_r = sy_r(p.resistance_n / 1000.0);
        let y_t = sy_t(p.trim_deg);
        let _ = writeln!(out, "<circle cx=\"{x:.1}\" cy=\"{y_r:.1}\" r=\"4\" fill=\"#0969da\" stroke=\"#fff\" stroke-width=\"1.5\"/>");
        let _ = writeln!(out, "<circle cx=\"{x:.1}\" cy=\"{y_t:.1}\" r=\"4\" fill=\"#d97706\" stroke=\"#fff\" stroke-width=\"1.5\"/>");
    }

    // Legend
    let leg_x = w - mr - 200.0;
    let leg_y = mt + 15.0;
    let _ = writeln!(out, "<line x1=\"{leg_x:.1}\" y1=\"{leg_y:.1}\" x2=\"{:.1}\" y2=\"{leg_y:.1}\" stroke=\"#0969da\" stroke-width=\"2.5\"/>", leg_x + 24.0);
    let _ = writeln!(
        out,
        "<circle cx=\"{:.1}\" cy=\"{leg_y:.1}\" r=\"3.5\" fill=\"#0969da\"/>",
        leg_x + 12.0
    );
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" fill=\"#0969da\" font-weight=\"bold\">Resistance [kN]</text>", leg_x + 30.0, leg_y + 4.0);

    let leg_y2 = leg_y + 20.0;
    let _ = writeln!(out, "<line x1=\"{leg_x:.1}\" y1=\"{leg_y2:.1}\" x2=\"{:.1}\" y2=\"{leg_y2:.1}\" stroke=\"#d97706\" stroke-width=\"2.5\" stroke-dasharray=\"5 3\"/>", leg_x + 24.0);
    let _ = writeln!(
        out,
        "<circle cx=\"{:.1}\" cy=\"{leg_y2:.1}\" r=\"3.5\" fill=\"#d97706\"/>",
        leg_x + 12.0
    );
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" fill=\"#d97706\" font-weight=\"bold\">Trim [&deg;]</text>", leg_x + 30.0, leg_y2 + 4.0);

    // Axis labels
    let _ = writeln!(out, "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-weight=\"bold\">Speed [knots]</text>", ml + (w - ml - mr) / 2.0, h - 12.0);
    let _ = writeln!(
        out,
        "<text class=\"label\" transform=\"rotate(-90)\" x=\"{:.1}\" y=\"20\" text-anchor=\"middle\" font-weight=\"bold\" fill=\"#0969da\">Resistance [kN]</text>",
        -(mt + (h - mt - mb) / 2.0)
    );
    let _ = writeln!(
        out,
        "<text class=\"label\" transform=\"rotate(90)\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\" font-weight=\"bold\" fill=\"#d97706\">Trim [&deg;]</text>",
        mt + (h - mt - mb) / 2.0,
        -(w - 20.0)
    );

    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::demo;

    #[test]
    fn report_shows_not_validated_note() {
        let surface = demo::demo_hull(false);
        let mut opts = ReportOptions::from_surface(&surface);
        let html = design_report(&surface, &opts).expect("report");
        let at = html
            .find(crate::NOT_VALIDATED_NOTE)
            .expect("note on the report");
        assert!(
            at < html.find("<section").expect("sections"),
            "note is above the sections"
        );

        opts.include_resistance = false;
        opts.include_planing = false;
        let html = design_report(&surface, &opts).expect("report");
        assert!(html.contains(crate::NOT_VALIDATED_NOTE));
    }

    #[test]
    fn chart_axes_span_speeds_given_in_any_order() {
        assert_eq!(
            speed_range([14.0, 12.0, 8.0].into_iter(), 10.0),
            (8.0, 14.0)
        );
        assert_eq!(speed_range([f64::NAN].into_iter(), 10.0), (0.0, 10.0));
    }

    #[test]
    fn tiny_negative_values_print_as_zero() {
        assert_eq!(fmt_num(-0.0000004, 3), "0.000");
        assert_eq!(fmt_num(-0.0, 2), "0.00");
        assert_eq!(fmt_num(-0.0006, 3), "-0.001");
        assert_eq!(fmt_num(-12.5, 1), "-12.5");
        assert_eq!(fmt_num(f64::NAN, 1), "—");
        assert_eq!(fmt_opt(Some(-0.0001), 3), "0.000");
    }

    #[test]
    fn report_on_demo_hull_contains_every_section_id_and_is_well_formed() {
        let surface = demo::demo_hull(false);
        let mut opts = ReportOptions::from_surface(&surface);
        opts.include_planing = true;
        let html = design_report(&surface, &opts).expect("report succeeds");

        let expected_sections = [
            "particulars",
            "lines",
            "hydrostatics",
            "sac",
            "stability",
            "resistance",
            "planing",
        ];
        for id in expected_sections {
            assert!(
                html.contains(&format!("id=\"{id}\"")),
                "report missing section id `{id}`"
            );
        }

        let svg_open = html.matches("<svg").count();
        let svg_close = html.matches("</svg>").count();
        assert_eq!(
            svg_open, svg_close,
            "unbalanced <svg> tags: {svg_open} vs {svg_close}"
        );
        assert!(
            svg_open >= 4,
            "expected at least 4 SVGs (lines, sac, stability, resistance, planing)"
        );

        let sec_open = html.matches("<section").count();
        let sec_close = html.matches("</section>").count();
        assert_eq!(
            sec_open, sec_close,
            "unbalanced <section> tags: {sec_open} vs {sec_close}"
        );
        assert_eq!(sec_open, 7, "expected 7 sections with planing enabled");

        assert!(
            !html.contains("NaN"),
            "HTML report must contain no NaN values"
        );
        assert!(
            html.contains("Severe Wind and Rolling (Weather) Criterion"),
            "HTML report must include the IMO weather criterion table"
        );
    }

    #[test]
    fn report_escapes_special_characters_in_name() {
        let mut surface = demo::demo_hull(false);
        surface.settings.name = "<script>alert('xss')</script>&\"Special\"".into();
        surface.settings.designer = "O'Connor & Sons <designer>".into();

        let opts = ReportOptions::from_surface(&surface);
        let html = design_report(&surface, &opts).expect("report succeeds");

        assert!(!html.contains("<script>"), "raw <script> must be escaped");
        assert!(html
            .contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;&amp;&quot;Special&quot;"));
        assert!(html.contains("O&#39;Connor &amp; Sons &lt;designer&gt;"));
    }

    #[test]
    fn report_is_deterministic() {
        let surface = demo::demo_hull(false);
        let opts = ReportOptions::from_surface(&surface);

        let h1 = design_report(&surface, &opts).expect("first call");
        let h2 = design_report(&surface, &opts).expect("second call");
        assert_eq!(
            h1, h2,
            "two calls on identical inputs must produce identical output"
        );
    }

    #[test]
    fn design_report_matches_design_report_with_empty_project() {
        let surface = demo::demo_hull(true);
        let mut opts = ReportOptions::from_surface(&surface);
        opts.level = 2;
        let r1 = design_report(&surface, &opts).expect("design_report succeeds");
        let r2 = design_report_with_project(&surface, &opts, &Project::default())
            .expect("design_report_with_project succeeds");
        assert_eq!(
            r1, r2,
            "empty project report must be byte-identical to design_report"
        );
    }

    fn demo_hull_project() -> (Surface, ReportOptions, Project) {
        use crate::damage::{CompartmentSpec, DamageCase};
        use crate::loading::LoadingCondition;
        use crate::project::ReportSeakeeping;

        let surface = demo::demo_hull(true);
        let mut opts = ReportOptions::from_surface(&surface);
        opts.level = 2;

        let orig_case: DamageCase =
            serde_json::from_str(include_str!("../../../examples/demo-damage.json"))
                .expect("demo-damage.json");
        let comp = match &orig_case.compartments[0] {
            CompartmentSpec::Inline(c) => c.clone(),
            CompartmentSpec::Named(_) => panic!("expected inline compartment"),
        };
        let case = DamageCase {
            compartments: vec![CompartmentSpec::Named(comp.name.clone())],
            ..orig_case
        };
        let lc: LoadingCondition =
            serde_json::from_str(include_str!("../../../examples/demo-loadcase.json"))
                .expect("demo-loadcase.json");

        let project = Project {
            compartments: vec![comp],
            damage_cases: vec![case],
            loading_conditions: vec![lc],
            report: crate::project::ReportSettings {
                loading_conditions: None,
                damage_cases: None,
                seakeeping: Some(ReportSeakeeping {
                    speed_kn: 6.0,
                    kg: None,
                    sea: Some(WaveSpectrum::Bretschneider { hs: 1.2, tp: 5.5 }),
                }),
                ..Default::default()
            },
            ..Default::default()
        };
        (surface, opts, project)
    }

    #[test]
    fn report_sections_appear_exactly_when_selected() {
        let (surface, opts, mut project) = demo_hull_project();

        // 1. All selected
        let html_all = design_report_with_project(&surface, &opts, &project).unwrap();
        assert!(html_all.contains("Loading Conditions"));
        assert!(html_all.contains("Damage Cases"));
        assert!(html_all.contains("Seakeeping"));
        let secs = report_sections_with_project(&opts, &project);
        assert!(secs.contains(&"loading"));
        assert!(secs.contains(&"damage"));
        assert!(secs.contains(&"seakeeping"));

        // 2. None selected
        project.report.loading_conditions = Some(vec![]);
        project.report.damage_cases = Some(vec![]);
        project.report.seakeeping = None;
        let html_none = design_report_with_project(&surface, &opts, &project).unwrap();
        assert!(!html_none.contains("Loading Conditions"));
        assert!(!html_none.contains("Damage Cases"));
        assert!(!html_none.contains("Seakeeping"));
        let secs_none = report_sections_with_project(&opts, &project);
        assert!(!secs_none.contains(&"loading"));
        assert!(!secs_none.contains(&"damage"));
        assert!(!secs_none.contains(&"seakeeping"));

        // 3. Only Loading Conditions
        project.report.loading_conditions = None; // all in project
        let html_lc = design_report_with_project(&surface, &opts, &project).unwrap();
        assert!(html_lc.contains("Loading Conditions"));
        assert!(!html_lc.contains("Damage Cases"));
        assert!(!html_lc.contains("Seakeeping"));

        // 4. Only Damage Cases
        project.report.loading_conditions = Some(vec![]);
        project.report.damage_cases = None;
        let html_dc = design_report_with_project(&surface, &opts, &project).unwrap();
        assert!(!html_dc.contains("Loading Conditions"));
        assert!(html_dc.contains("Damage Cases"));
        assert!(!html_dc.contains("Seakeeping"));

        // 5. Only Seakeeping
        project.report.damage_cases = Some(vec![]);
        project.report.seakeeping = Some(crate::project::ReportSeakeeping {
            speed_kn: 5.0,
            kg: Some(1.2),
            sea: None,
        });
        let html_sk = design_report_with_project(&surface, &opts, &project).unwrap();
        assert!(!html_sk.contains("Loading Conditions"));
        assert!(!html_sk.contains("Damage Cases"));
        assert!(html_sk.contains("Seakeeping"));
    }

    /// R8-3 (A08 follow-up). A loading condition the hull cannot float is
    /// reported with a warning box, not as if its figures were an
    /// equilibrium; a condition that floats carries no such box. The
    /// runabout is closed at the deck, so 1000 t sinks it.
    #[test]
    fn report_warns_when_a_loading_condition_has_no_equilibrium() {
        let (surface, mut project) =
            crate::examples::build(crate::examples::Example::Runabout6m).unwrap();
        let mut opts = ReportOptions::from_surface(&surface);
        opts.level = 1;
        opts.include_resistance = false;
        project.report.damage_cases = Some(vec![]);
        project.report.seakeeping = None;
        let fine = design_report_with_project(&surface, &opts, &project).unwrap();
        assert!(fine.contains("Loading Conditions"));
        assert!(!fine.contains(loading::NO_EQUILIBRIUM_NOTE));

        project.loading_conditions[0].lightship.mass_t = 1000.0;
        let html = design_report_with_project(&surface, &opts, &project).unwrap();
        let note = format!(
            "<strong>Warning: {}.</strong>",
            loading::NO_EQUILIBRIUM_NOTE
        );
        let at = html.find(&note).expect("the warning box");
        let open = html[..at].rfind("<section").expect("a section");
        let rest = &html[at..];
        let close = rest.find("</section>").expect("closed");
        assert!(
            html[open..].starts_with("<section id=\"loading\">")
                && rest.find("<section").is_none_or(|n| n > close),
            "the warning sits in the loading section"
        );
        assert!(html[at..].contains("cannot float"), "the specific warning");
    }

    #[test]
    fn report_escapes_special_characters_in_condition_name_and_reports_missing() {
        let (surface, opts, mut project) = demo_hull_project();
        project.loading_conditions[0].name = "<b>&\"x\"".into();
        project.report.loading_conditions =
            Some(vec!["<b>&\"x\"".into(), "nonexistent_condition".into()]);
        project.report.damage_cases = Some(vec!["nonexistent_case".into()]);

        let html = design_report_with_project(&surface, &opts, &project).unwrap();

        // Must escape special characters in condition name
        assert!(
            !html.contains("<b>&\"x\""),
            "raw <b>&\"x\" must not appear in HTML"
        );
        assert!(
            html.contains("&lt;b&gt;&amp;&quot;x&quot;"),
            "escaped name must appear"
        );

        // Missing names produce warning paragraph, not panic
        assert!(html.contains("Loading condition <strong>nonexistent_condition</strong> not found"));
        assert!(html.contains("Damage case <strong>nonexistent_case</strong> not found"));

        // No NaN or inf anywhere in output
        assert!(!html.contains("NaN"), "report must contain no NaN");
        assert!(!html.contains("inf"), "report must contain no inf");
        assert!(
            !html.contains("Infinity"),
            "report must contain no Infinity"
        );

        // Deterministic on same input
        let html2 = design_report_with_project(&surface, &opts, &project).unwrap();
        assert_eq!(html, html2, "repeated calls must be identical");
    }
}
