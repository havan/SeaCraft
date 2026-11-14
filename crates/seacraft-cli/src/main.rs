mod mcp;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use seacraft_core::hydro::{self, HydroOptions};
use seacraft_core::stability::{
    cross_curves, evaluate_imo_criteria, hydrostatics_free_trim, solve_draft, upright_lcb,
    StabilityCondition,
};
use seacraft_core::subdiv::{evaluate, SubdivisionMode, SubdivisionOptions};
use seacraft_core::{
    analysis::AnalysisMode,
    curves, damage, demo, develop, export, fef, lackenby, linesplan, loading,
    math::Vec3,
    mesh::Surface,
    michell, offsets,
    parametric::{self, HullKind, HullSpec},
    planing, project, raster, resistance, table, topology, transform, trimopt,
};
use std::path::{Path, PathBuf};

#[derive(Parser, Debug)]
#[command(
    name = "seacraft",
    version,
    about = "Subdivision-surface hull design, hydrostatics and stability (FREE!ship compatible)",
    long_about = "SeaCraft is a Rust reimplementation of the FREE!ship hull modeller.\n\
                  It reads and writes FREE!ship .fef exchange files, refines the control\n\
                  net with the same Loop/Catmull-Clark scheme and reports hydrostatics\n\
                  and stability the way the original application does.",
    after_help = "SeaCraft comes with ABSOLUTELY NO WARRANTY, and its results are not validated.\n\
                  It is free software under GPL-3.0-or-later; see LICENSE and DISCLAIMER.md."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write the built-in demo hull to a .fef file and its sidecar (KG 2.2 m)
    Demo(DemoArgs),
    /// Write a ready-made example hull and its project sidecar (--list shows them)
    Example(ExampleArgs),
    /// Generate a parametric hull control net from principal dimensions
    Generate(GenerateArgs),
    /// Summarise a model
    Info(InfoArgs),
    /// Hydrostatics for one floating condition
    Hydro(HydroArgs),
    /// Export the refined mesh to OBJ, STL, PLY or a standalone HTML viewer
    Export(ExportArgs),
    /// Bake the subdivided surface back into a control net
    Subdiv(SubdivArgs),
    /// Section area curve
    Sac(SacArgs),
    /// Bonjean curves: section area against draft at each station
    Bonjean(BonjeanArgs),
    /// Table of offsets: half-breadths at stations x waterlines, plus buttocks
    Offsets(OffsetsArgs),
    /// Scale to target main dimensions, translate or mirror the control net
    Transform(TransformArgs),
    /// Shift stations to nudge Cp and LCB (Lackenby-style, ends pinned)
    Lackenby(LackenbyArgs),
    /// Hydrostatics table: displacement, KB, BM, KM, LCB, LCF, TPC, MTC per draft
    Table(TableArgs),
    /// Scan trim at a speed and report the cheapest, at constant displacement
    Trim(TrimArgs),
    /// Reshape the hull to cut resistance at a speed, holding displacement and stability
    Optimize(OptimizeArgs),
    /// Still-water resistance and effective power (Holtrop-Mennen 1984)
    Resistance(ResistanceArgs),
    /// Developable plate expansion: flatten a strake or buttock band
    Unfold(UnfoldArgs),
    /// Edit the control net: split faces, insert edges, toggle creases
    Edit(EditArgs),
    /// Savitsky planing resistance: trim, wetted length and drag
    Planing(PlaningArgs),
    /// Cross curves of stability (KN / GZ)
    Stability(StabilityArgs),
    /// Evaluate a ship loading condition (weights, tanks, FSM/FSC, floating equilibrium and IMO A.749)
    Loadcase(LoadcaseArgs),
    /// Deterministic damage stability by the lost-buoyancy method
    Damage(DamageArgs),
    /// Strip-theory heave and pitch motion RAOs in regular head seas (Salvesen, Tuck & Faltinsen 1970)
    Seakeeping(SeakeepingArgs),
    /// Linesplan: stations, buttocks and waterlines as SVG or DXF
    Linesplan(LinesplanArgs),
    /// Full-size building patterns: station moulds, profile and transom as PDF, DXF or SVG
    Templates(TemplatesArgs),
    /// Plywood panels laid flat for stitch-and-glue: full-size patterns, offsets and a sheet count
    Panels(PanelsArgs),
    /// Design dossier: self-contained HTML report with particulars, linesplan, hydrostatics, SAC, stability and resistance
    Report(ReportArgs),
    /// Render a preview PNG, including zebra, curvature and draft analysis
    Render(RenderArgs),
    /// Serve the hull to a Model Context Protocol client over stdio
    Mcp(McpArgs),
}

#[derive(Args, Debug)]
struct RenderArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output PNG path
    #[arg(short, long, default_value = "preview.png")]
    output: PathBuf,
    /// Shading mode
    #[arg(long, value_enum, default_value_t = RenderModeArg::Shaded)]
    mode: RenderModeArg,
    /// Camera: a three-quarter perspective or one of the orthographic views
    #[arg(long, value_enum, default_value_t = RenderViewArg::Perspective)]
    view: RenderViewArg,
    /// Image size in pixels, WIDTHxHEIGHT
    #[arg(long, default_value = "1280x800")]
    size: String,
    /// Zebra stripe count
    #[arg(long, default_value_t = 14.0)]
    stripes: f64,
    /// Curvature scale: 1 / radius at which the map turns red (metres)
    #[arg(long, default_value_t = 1.0)]
    radius: f64,
    /// Draft angle in degrees
    #[arg(long, default_value_t = 3.0)]
    draft: f64,
    /// Overlay the mesh edges
    #[arg(long)]
    wireframe: bool,
    /// Supersampling factor (1 = off, 2 = four samples per pixel)
    #[arg(long, default_value_t = 2)]
    samples: usize,
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Use Catmull-Clark instead of the FREE!ship default Loop scheme
    #[arg(long)]
    catmull_clark: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum RenderModeArg {
    Shaded,
    Zebra,
    Curvature,
    Draft,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum RenderViewArg {
    Perspective,
    Profile,
    Plan,
    Body,
}

#[derive(Args, Debug)]
struct DemoArgs {
    /// Output .fef path
    #[arg(short, long, default_value = "seacraft12.fef")]
    output: PathBuf,
    /// Model only the starboard half (symmetric layer)
    #[arg(long)]
    symmetric: bool,
}

#[derive(Args, Debug)]
struct ExampleArgs {
    /// Example name, e.g. `runabout-6m` (see --list)
    #[arg(conflicts_with_all = ["list", "all"])]
    name: Option<String>,
    /// List the examples
    #[arg(long, conflicts_with = "all")]
    list: bool,
    /// Write every example into the directory given with -o
    #[arg(long)]
    all: bool,
    /// Output .fef path (default `<name>.fef`), or the directory for --all;
    /// the `.seacraft.json` sidecar is written next to each model
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// For --list: print JSON
    #[arg(long)]
    json: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum HullKindArg {
    Displacement,
    Planing,
    Yacht,
}

impl From<HullKindArg> for HullKind {
    fn from(k: HullKindArg) -> Self {
        match k {
            HullKindArg::Displacement => HullKind::Displacement,
            HullKindArg::Planing => HullKind::Planing,
            HullKindArg::Yacht => HullKind::Yacht,
        }
    }
}

#[derive(Args, Debug)]
struct GenerateArgs {
    /// Hull family
    #[arg(long, value_enum, default_value_t = HullKindArg::Displacement)]
    kind: HullKindArg,
    /// Waterline length LWL in metres
    #[arg(long)]
    lwl: Option<f64>,
    /// Waterline beam BWL in metres
    #[arg(long)]
    beam: Option<f64>,
    /// Canoe-body draft T at midship in metres
    #[arg(long)]
    draft: Option<f64>,
    /// Moulded depth from keel to sheer at midship in metres
    #[arg(long)]
    depth: Option<f64>,
    /// Optional target block coefficient Cb (0.35..=0.85)
    #[arg(long)]
    cb: Option<f64>,
    /// Optional target LCB as a fraction of LWL from the aft perpendicular (0.35..=0.60)
    #[arg(long)]
    lcb: Option<f64>,
    /// Bottom deadrise angle in degrees (planing only, default 15)
    #[arg(long)]
    deadrise: Option<f64>,
    /// Transom immersion / beam ratio (0..=1)
    #[arg(long)]
    transom: Option<f64>,
    /// Number of longitudinal stations (default 9)
    #[arg(long)]
    stations: Option<usize>,
    /// Number of control points per station (default 5)
    #[arg(long)]
    points_per_station: Option<usize>,
    /// Output .fef path
    #[arg(short, long, default_value = "generated.fef")]
    output: PathBuf,
    /// Emit the generation report as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct InfoArgs {
    /// Input .fef file
    file: PathBuf,
}

#[derive(Args, Debug, Clone)]
struct SubdivArgsCli {
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Refinement scheme
    #[arg(long, value_enum, default_value_t = ModeArg::Loop)]
    mode: ModeArg,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum ModeArg {
    /// Loop for triangles, quads for larger faces (FREE!ship default)
    Loop,
    /// Catmull-Clark: every face becomes quads
    Cc,
}

impl From<ModeArg> for SubdivisionMode {
    fn from(m: ModeArg) -> Self {
        match m {
            ModeArg::Loop => SubdivisionMode::Loop,
            ModeArg::Cc => SubdivisionMode::CatmullClark,
        }
    }
}

#[derive(Args, Debug)]
struct HydroArgs {
    /// Input .fef file
    file: PathBuf,
    /// Design draft in metres (defaults to the project setting)
    #[arg(long)]
    draft: Option<f64>,
    /// Trim in metres, positive when the bow sits higher than the stern
    #[arg(long, default_value_t = 0.0)]
    trim: f64,
    /// Heel angle in degrees, positive to starboard
    #[arg(long, default_value_t = 0.0)]
    heel: f64,
    /// Longitudinal centre of gravity in model x. Implies free trim: the hull
    /// trims to keep the LCB on this station. Defaults to the upright LCB.
    #[arg(long)]
    lcg: Option<f64>,
    /// Trim freely to longitudinal equilibrium instead of holding --trim
    #[arg(long)]
    free_trim: bool,
    /// Water density in kg/m3 (defaults to the project setting)
    #[arg(long)]
    density: Option<f64>,
    /// Appendage coefficient (defaults to the project setting)
    #[arg(long)]
    appendage: Option<f64>,
    /// Vertical centre of gravity above the keel, adds GM to the report
    #[arg(long)]
    kg: Option<f64>,
    /// Number of stations for the section area curve
    #[arg(long, default_value_t = 21)]
    stations: usize,
    /// Station x for the midship section (defaults to the LCF)
    #[arg(long)]
    mainframe: Option<f64>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct ExportArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output file; the extension selects the format (.obj/.stl/.3mf/.ply/.igs/.html)
    #[arg(short, long)]
    output: PathBuf,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Export the control net instead of the refined surface (OBJ only;
    /// refused for the other formats)
    #[arg(long)]
    control: bool,
    /// Up axis for OBJ output [default: z]. The other formats are always
    /// written z-up; `--up y` is refused for them.
    #[arg(long, value_enum)]
    up: Option<UpArg>,
    /// Skip mirroring symmetric layers, single half only (OBJ only; refused
    /// for the other formats)
    #[arg(long)]
    no_mirror: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum UpArg {
    Y,
    Z,
}

#[derive(Args, Debug)]
struct SubdivArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output .fef file
    #[arg(short, long)]
    output: PathBuf,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Mirror symmetric layers into the baked control net
    #[arg(long)]
    mirror: bool,
}

#[derive(Args, Debug)]
struct SacArgs {
    file: PathBuf,
    #[arg(long, default_value_t = 21)]
    stations: usize,
    #[arg(long)]
    draft: Option<f64>,
    /// Write CSV to this path (otherwise print a table)
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Write the plotted curve to this SVG path
    #[arg(long)]
    svg: Option<PathBuf>,
    /// Emit the curve as JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct BonjeanArgs {
    file: PathBuf,
    /// Number of stations, end points included
    #[arg(long, default_value_t = 21)]
    stations: usize,
    /// Drafts in metres, comma separated (default: depth/10 … depth)
    #[arg(long)]
    drafts: Option<String>,
    /// Number of drafts in the default range
    #[arg(long, default_value_t = 10)]
    draft_steps: usize,
    /// Design draft used for the header and coefficients
    #[arg(long)]
    draft: Option<f64>,
    /// Write the long-format table (x,draft,area_m2) to this CSV path
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Write the plotted curves to this SVG path
    #[arg(long)]
    svg: Option<PathBuf>,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct OffsetsArgs {
    file: PathBuf,
    /// Number of stations, end points excluded (slice midpoints)
    #[arg(long, default_value_t = 21)]
    stations: usize,
    /// Waterline heights above the keel in metres, comma separated
    #[arg(long)]
    waterlines: Option<String>,
    /// Number of waterlines in the default range
    #[arg(long, default_value_t = 6)]
    waterline_steps: usize,
    /// Buttock offsets from the centreline in metres, comma separated
    #[arg(long)]
    buttocks: Option<String>,
    /// Number of buttocks in the default range
    #[arg(long, default_value_t = 4)]
    buttock_steps: usize,
    /// Write CSV to this path (long format)
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Write the HTML table to this path
    #[arg(long)]
    html: Option<PathBuf>,
    /// Write the tables as a printable PDF, in millimetres, to this path
    #[arg(long)]
    pdf: Option<PathBuf>,
    /// Add the tables in feet, inches and eighths to the PDF
    #[arg(long)]
    imperial: bool,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct TrimArgs {
    /// Input .fef file
    file: PathBuf,
    /// Speed in knots
    #[arg(long)]
    speed: f64,
    /// Aft limit of the trim range in metres (negative is bow down)
    #[arg(long, default_value_t = -0.6, allow_hyphen_values = true)]
    from: f64,
    /// Forward limit of the trim range in metres
    #[arg(long, default_value_t = 0.6, allow_hyphen_values = true)]
    to: f64,
    /// Number of trims to try
    #[arg(long, default_value_t = 13)]
    steps: usize,
    /// Draft at which to hold the displacement, in metres
    #[arg(long)]
    draft: Option<f64>,
    /// Let the displacement change with trim instead of solving the draft
    #[arg(long)]
    no_hold_volume: bool,
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Emit the scan as JSON
    #[arg(long)]
    json: bool,
}

fn cmd_trim(a: TrimArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.level,
        mode: SubdivisionMode::Loop,
    };
    let mut opts = HydroOptions::from_surface(&s, subdiv);
    if let Some(d) = a.draft {
        opts.draft = d;
    }
    let trims = trimopt::trim_range(a.from, a.to, a.steps);
    let scan = trimopt::scan(&s, subdiv, &opts, a.speed, &trims, !a.no_hold_volume)
        .map_err(|e| anyhow::anyhow!(e))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&scan)?);
        return Ok(());
    }
    println!(
        "trim scan at {:.2} kn, displacement held at {:.3} m3{}",
        scan.speed_kn,
        scan.target_volume_m3,
        if a.no_hold_volume {
            " (NOT held: draft fixed)"
        } else {
            ""
        }
    );
    println!("   trim    draft     LWL      Fn     RT [kN]   PE [kW]");
    for p in &scan.points {
        println!(
            "{:+7.3} {:7.3} {:8.3} {:6.3} {:10.3} {:10.2}",
            p.trim_m,
            p.draft_m,
            p.lwl_m,
            p.froude,
            p.rt_n / 1000.0,
            p.pe_kw
        );
    }
    if let Some(b) = scan.best {
        println!(
            "cheapest: trim {:+.3} m at draft {:.3} m - RT {:.3} kN, PE {:.2} kW{}",
            b.trim_m,
            b.draft_m,
            b.rt_n / 1000.0,
            b.pe_kw,
            if scan.at_boundary {
                " (at the edge of the range)"
            } else {
                ""
            }
        );
    }
    for w in &scan.warnings {
        println!("  warning: {w}");
    }
    Ok(())
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum OptimizeMethodArg {
    /// Savitsky at Fn >= 1 when the hull planes there, Holtrop-Mennen otherwise
    Auto,
    Holtrop,
    Savitsky,
}

#[derive(Args, Debug)]
struct OptimizeArgs {
    /// Input .fef file
    file: PathBuf,
    /// Design speed in knots
    #[arg(long)]
    speed: f64,
    /// Output .fef path for the optimised hull
    #[arg(short, long)]
    output: PathBuf,
    /// Resistance method that scores a candidate
    #[arg(long, value_enum, default_value_t = OptimizeMethodArg::Auto)]
    method: OptimizeMethodArg,
    /// Variables to optimise, comma separated: cp (prismatic coefficient), lcb
    /// (longitudinal centre of buoyancy), lb (L/B, x scaled by s and y by 1/s;
    /// opt-in because it changes the length)
    #[arg(long, default_value = "cp,lcb")]
    vars: String,
    /// Override a variable's bounds, e.g. --bound cp=-0.02:0.02 (repeatable)
    #[arg(long = "bound", allow_hyphen_values = true)]
    bounds: Vec<String>,
    /// Minimum GM = KM - KG in metres (needs --kg)
    #[arg(long, allow_hyphen_values = true)]
    min_gm: Option<f64>,
    /// KG above the keel in metres (reports GM; with --min-gm also constrains it)
    #[arg(long, allow_hyphen_values = true)]
    kg: Option<f64>,
    /// Allowed LCB as a fraction of LWL from the aft end, LO,HI (0.5 is midships)
    #[arg(long)]
    lcb_band: Option<String>,
    /// Displacement may change by this fraction
    #[arg(long, default_value_t = 0.01)]
    tol: f64,
    /// Maximum objective evaluations
    #[arg(long, default_value_t = 60)]
    max_evals: usize,
    /// Draft the baseline is measured at (default: the model's design draft)
    #[arg(long)]
    draft: Option<f64>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit the report as JSON
    #[arg(long)]
    json: bool,
}

fn cmd_optimize(a: OptimizeArgs) -> Result<()> {
    use seacraft_core::optimize::{self, Method, OptimizeSpec};
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
    };
    let opts = HydroOptions::from_surface(&s, subdiv);
    let mut spec = OptimizeSpec::new(a.speed);
    spec.method = match a.method {
        OptimizeMethodArg::Auto => Method::Auto,
        OptimizeMethodArg::Holtrop => Method::Holtrop,
        OptimizeMethodArg::Savitsky => Method::Savitsky,
    };
    spec.vars = OptimizeSpec::parse_vars(&a.vars).map_err(|e| anyhow::anyhow!(e))?;
    OptimizeSpec::apply_bound_strings(&mut spec.vars, &a.bounds).map_err(|e| anyhow::anyhow!(e))?;
    spec.min_gm = a.min_gm;
    spec.kg = a.kg;
    if let Some(band) = &a.lcb_band {
        let v = parse_ordinates(band, "lcb-band")?;
        if v.len() != 2 {
            bail!("--lcb-band takes two numbers, LO,HI");
        }
        spec.lcb_band = Some((v[0], v[1]));
    }
    spec.hold_displacement_tol = a.tol;
    spec.max_evaluations = a.max_evals;
    spec.draft = a.draft;
    let (out, report) =
        optimize::optimize_hull(&s, &opts, &spec).map_err(|e| anyhow::anyhow!(e))?;
    seacraft_core::io::write_atomic(&a.output, fef::write(&out))
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    let (b, o) = (&report.baseline, &report.optimised);
    println!(
        "{}  -  resistance optimisation at {:.2} kn ({:?}, Fn {:.3})",
        s.settings.name, report.speed_kn, report.method, b.froude
    );
    println!(
        "  {:<14} {:>12} {:>12} {:>10}",
        "", "baseline", "optimised", "change"
    );
    let row = |name: &str, x: f64, y: f64, prec: usize| {
        println!(
            "  {:<14} {:>12.prec$} {:>12.prec$} {:>+10.prec$}",
            name,
            x,
            y,
            y - x,
            prec = prec
        );
    };
    row("RT [kN]", b.rt_kn, o.rt_kn, 3);
    row("PE [kW]", b.pe_kw, o.pe_kw, 2);
    row("displ. [t]", b.displacement_t, o.displacement_t, 3);
    row("volume [m3]", b.volume_m3, o.volume_m3, 3);
    row("draft [m]", b.draft_m, o.draft_m, 4);
    row("Cp", b.cp, o.cp, 4);
    row("Cb", b.cb, o.cb, 4);
    row("LCB [% LWL]", b.lcb_pct, o.lcb_pct, 2);
    row("LWL [m]", b.lwl_m, o.lwl_m, 3);
    row("BWL [m]", b.bwl_m, o.bwl_m, 3);
    row("L/B", b.lb, o.lb, 3);
    row("B/T", b.bt, o.bt, 3);
    row("KM [m]", b.km_m, o.km_m, 3);
    if let (Some(gb), Some(go)) = (b.gm_m, o.gm_m) {
        row("GM [m]", gb, go, 3);
    }
    println!(
        "  RT {:+.2} %   displacement error {:.3} %   constraints {}",
        report.rt_change_pct,
        report.displacement_error_pct,
        if report.constraints_met {
            "met"
        } else {
            "NOT met"
        }
    );
    println!("  variables:");
    for v in &report.variables {
        println!(
            "    {:<16} {:>9.5}  (range {:.4} .. {:.4}{})",
            v.name,
            v.value,
            v.lo,
            v.hi,
            if v.at_bound { ", on a bound" } else { "" }
        );
    }
    println!(
        "  {} iterations, {} evaluations ({} distinct); {}",
        report.iterations, report.evaluations, report.unique_evaluations, report.convergence_reason
    );
    for w in &report.warnings {
        println!("  warning: {w}");
    }
    println!("  wrote {}", a.output.display());
    Ok(())
}

#[derive(Args, Debug)]
struct TableArgs {
    /// Input .fef file
    file: PathBuf,
    /// Top draft in metres (default: the project draft)
    #[arg(long)]
    to: Option<f64>,
    /// Number of drafts from just above zero to the top one
    #[arg(long, default_value_t = 10)]
    steps: usize,
    /// Explicit drafts, comma separated (overrides --to/--steps)
    #[arg(long)]
    drafts: Option<String>,
    /// Write the table as CSV
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Write the table as a standalone HTML page
    #[arg(long)]
    html: Option<PathBuf>,
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Emit the table as JSON
    #[arg(long)]
    json: bool,
}

fn cmd_table(a: TableArgs) -> Result<()> {
    let text = std::fs::read_to_string(&a.file)
        .with_context(|| format!("cannot read {}", a.file.display()))?;
    let s = fef::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", a.file.display()))?;
    let opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.level,
            mode: SubdivisionMode::Loop,
        },
    );
    let drafts = match &a.drafts {
        Some(list) => list
            .split(',')
            .map(|t| t.trim().parse::<f64>())
            .collect::<Result<Vec<_>, _>>()
            .map_err(|e| anyhow::anyhow!("--drafts: {e}"))?,
        None => table::draft_range(a.to.unwrap_or(opts.draft), a.steps),
    };
    if drafts.is_empty() {
        return Err(anyhow::anyhow!("no drafts to measure"));
    }
    let t = table::table(&s, &opts, &drafts);
    if let Some(path) = &a.csv {
        seacraft_core::io::write_atomic(path, t.csv())
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    if let Some(path) = &a.html {
        seacraft_core::io::write_atomic(path, t.html())
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&t)?);
        return Ok(());
    }
    println!(
        "hydrostatics for {} (rho {:.1} kg/m3, level {})",
        t.name, t.water_density, a.level
    );
    println!(
        "   T     nabla      Delta     KB      BM     KMT     LCB     LCF     Cb     Cp      TPC    MTC"
    );
    println!("   m       m3          t      m       m       m       m       m                     t/cm  t*m/cm");
    for r in &t.rows {
        println!(
            "{:5.2} {:9.3} {:9.2} {:6.3} {:6.3} {:7.3} {:7.3} {:7.3} {:6.3} {:6.3} {:6.3} {:6.3}",
            r.draft,
            r.volume_m3,
            r.displacement_t,
            r.kb,
            r.bm_transverse,
            r.km_transverse,
            r.lcb_x,
            r.lcf_x,
            r.block_coefficient,
            r.prismatic_coefficient,
            r.tpc,
            r.mtc
        );
    }
    if let Some(p) = &a.csv {
        println!("  wrote {}", p.display());
    }
    if let Some(p) = &a.html {
        println!("  wrote {}", p.display());
    }
    Ok(())
}

#[derive(Args, Debug)]
struct LackenbyArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output .fef path
    #[arg(short, long)]
    output: PathBuf,
    /// Target prismatic coefficient (omit to keep the current one)
    #[arg(long)]
    cp: Option<f64>,
    /// Target LCB as a percentage of LWL from the aft end of the waterline,
    /// positive forward of midships (omit to keep the current one)
    #[arg(long, allow_hyphen_values = true)]
    lcb: Option<f64>,
    /// Draft at which to measure, in metres
    #[arg(long)]
    draft: Option<f64>,
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Emit the report as JSON
    #[arg(long)]
    json: bool,
}

fn cmd_lackenby(a: LackenbyArgs) -> Result<()> {
    let text = std::fs::read_to_string(&a.file)
        .with_context(|| format!("cannot read {}", a.file.display()))?;
    let s = fef::parse(&text).map_err(|e| anyhow::anyhow!("{}: {e}", a.file.display()))?;
    let (opts, target) = lackenby_setup(&s, a.level, a.draft, a.cp, a.lcb);
    let (out, report) =
        lackenby::fit(&s, target, &opts, 1e-3, 10).map_err(|e| anyhow::anyhow!(e))?;
    seacraft_core::io::write_atomic(&a.output, fef::write(&out))
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&report)?);
        return Ok(());
    }
    println!("station shift (Lackenby-style, ends pinned)");
    println!(
        "  before  Cp {:.4}  LCB {:+.2} %  V {:.3} m3  LWL {:.3} m",
        report.before.cp, report.before.lcb_pct, report.before.volume_m3, report.before.lwl
    );
    println!(
        "  target  Cp {:.4}  LCB {:+.2} %",
        target.cp, target.lcb_pct
    );
    println!(
        "  after   Cp {:.4}  LCB {:+.2} %  V {:.3} m3  LWL {:.3} m",
        report.after.cp, report.after.lcb_pct, report.after.volume_m3, report.after.lwl
    );
    println!(
        "  error   dCp {:+.5}  dLCB {:+.4} %  ({} iterations)",
        report.cp_error, report.lcb_error, report.iterations
    );
    println!(
        "  shift   a {:+.4}  b {:+.4}  midships {:.3} m",
        report.coefficients.0, report.coefficients.1, report.midship_shift_m
    );
    if !report.converged {
        println!("  ! outside this method's authority: the file holds the closest hull found");
    }
    println!("  wrote {}", a.output.display());
    Ok(())
}

/// The measuring options and the target of `seacraft lackenby`. An omitted
/// `cp` or `lcb` keeps the hull's own value, measured the way
/// `lackenby::fit` measures it: LCB as a percentage of LWL from the aft end
/// of the waterline (`wl_aft_x`), not from the model's aft extreme — so a
/// counter or raked transom overhanging the waterline does not shift the LCB
/// the user never asked to move.
fn lackenby_setup(
    s: &Surface,
    level: u8,
    draft: Option<f64>,
    cp: Option<f64>,
    lcb: Option<f64>,
) -> (HydroOptions, lackenby::LackenbyTarget) {
    let mut opts = HydroOptions::from_surface(
        s,
        SubdivisionOptions {
            level,
            mode: SubdivisionMode::Loop,
        },
    );
    if let Some(d) = draft {
        opts.draft = d;
    }
    opts.stations = opts.stations.max(41);
    let now = hydro::compute(s, &opts);
    let lcb_now = if now.lwl > 1e-9 {
        (now.lcb.x - now.wl_aft_x) / now.lwl * 100.0 - 50.0
    } else {
        0.0
    };
    let target = lackenby::LackenbyTarget::new(
        cp.unwrap_or(now.prismatic_coefficient),
        lcb.unwrap_or(lcb_now),
    );
    (opts, target)
}

#[derive(Args, Debug)]
struct TransformArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output .fef path
    #[arg(short, long)]
    output: PathBuf,
    /// Target waterline length in metres
    #[arg(long)]
    lwl: Option<f64>,
    /// Target waterline beam in metres
    #[arg(long)]
    bwl: Option<f64>,
    /// Target draft in metres (above the keel)
    #[arg(long)]
    draft: Option<f64>,
    /// Extra scale factors, x,y,z (applied after the target fit)
    #[arg(long)]
    scale: Option<String>,
    /// Translation in metres, dx,dy,dz
    #[arg(long)]
    translate: Option<String>,
    /// Reflect in x, y or z
    #[arg(long, value_enum)]
    mirror: Option<MirrorArg>,
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Use Catmull-Clark instead of the FREE!ship default Loop scheme
    #[arg(long)]
    catmull_clark: bool,
    /// Emit the report as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq, ValueEnum)]
enum MirrorArg {
    X,
    Y,
    Z,
}

#[derive(ValueEnum, Clone, Copy, Debug, PartialEq, Eq, Default)]
enum ResistanceMethodArg {
    #[default]
    Holtrop,
    Michell,
    Dsyhs,
}

#[derive(Args, Debug)]
struct ResistanceArgs {
    /// Input .fef file
    file: PathBuf,
    /// Resistance method (holtrop, michell, or dsyhs)
    #[arg(long, value_enum, default_value_t = ResistanceMethodArg::Holtrop)]
    method: ResistanceMethodArg,
    /// Catamaran demihull centreline spacing in metres (Michell method only)
    #[arg(long)]
    cat_spacing: Option<f64>,
    /// Speeds in knots, comma separated
    #[arg(long, default_value = "6,8,10,12,14")]
    speeds: String,
    /// Draft to measure at (default: the model's design draft)
    #[arg(long)]
    draft: Option<f64>,
    /// Holtrop stern parameter, -10 (V) .. +10 (U with gondola)
    #[arg(long, default_value_t = 0.0)]
    cstern: f64,
    /// Wetted area of appendages, m²
    #[arg(long, default_value_t = 0.0)]
    appendage_area: f64,
    /// Appendage form factor 1+k2
    #[arg(long, default_value_t = 1.5)]
    appendage_factor: f64,
    /// Propeller diameter in metres; enables the propulsion factors and shaft power
    #[arg(long, default_value_t = 0.0)]
    propeller_diameter: f64,
    /// Propeller expanded blade-area ratio AE/A0
    #[arg(long, default_value_t = 0.70)]
    expanded_area: f64,
    /// Open-water efficiency at the design point
    #[arg(long, default_value_t = 0.60)]
    eta_o: f64,
    /// Shafting efficiency, for brake power
    #[arg(long, default_value_t = 0.98)]
    shaft_efficiency: f64,
    /// Immersed bulbous-bow area, m²
    #[arg(long, default_value_t = 0.0)]
    bulb_area: f64,
    /// Height of the bulb's centre above the keel, m
    #[arg(long, default_value_t = 0.0)]
    bulb_z: f64,
    /// Write CSV to this path
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct UnfoldArgs {
    /// Input .fef file
    file: PathBuf,
    /// Axis the band is cut across: x (stations), y (buttocks) or z (waterlines)
    #[arg(long, default_value = "z")]
    axis: String,
    /// Lower bound of the band in metres (default: the model's minimum)
    #[arg(long)]
    from: Option<f64>,
    /// Upper bound of the band in metres (default: the model's maximum)
    #[arg(long)]
    to: Option<f64>,
    /// Start of the plate's run along x (default: the model's minimum)
    #[arg(long)]
    x_from: Option<f64>,
    /// End of the plate's run along x (default: the model's maximum)
    #[arg(long)]
    x_to: Option<f64>,
    /// Include the port side too (the default cuts only the starboard plate)
    #[arg(long)]
    both_sides: bool,
    /// Write the flat pattern as SVG
    #[arg(short, long)]
    output: Option<PathBuf>,
    /// Emit the report as JSON
    #[arg(long)]
    json: bool,
    /// Subdivision level (0 = control net, max 6)
    #[arg(long, default_value_t = 3)]
    level: u8,
    /// Use Catmull-Clark instead of the FREE!ship default Loop scheme
    #[arg(long)]
    catmull_clark: bool,
}

#[derive(Args, Debug)]
struct EditArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output .fef path
    #[arg(short, long)]
    output: PathBuf,
    /// Split this face into a triangle fan around its centroid (repeatable)
    #[arg(long = "split-face", value_name = "FACE")]
    split_face: Vec<usize>,
    /// Insert an edge between two points of a face, FACE,A,B (repeatable)
    #[arg(long = "insert-edge", value_name = "FACE,A,B")]
    insert_edge: Vec<String>,
    /// Mark the edge between two points as a crease, A,B (repeatable)
    #[arg(long = "crease", value_name = "A,B")]
    crease: Vec<String>,
    /// Clear the crease between two points, A,B (repeatable)
    #[arg(long = "clear-crease", value_name = "A,B")]
    clear_crease: Vec<String>,
    /// Emit the report as JSON
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct PlaningArgs {
    /// Input .fef file
    file: PathBuf,
    /// Speeds in knots, comma separated
    #[arg(long, default_value = "20,25,30,35,40")]
    speeds: String,
    /// Draft the displacement is taken at (default: the model's design draft)
    #[arg(long)]
    draft: Option<f64>,
    /// Planing beam in metres (default: the waterline beam)
    #[arg(long)]
    beam: Option<f64>,
    /// Longitudinal centre of gravity in model x (default: the LCB)
    #[arg(long)]
    lcg_x: Option<f64>,
    /// Deadrise at the transom in degrees (default: the tangent at the keel of
    /// the transom section, which understates it on a round-bilge hull — give
    /// the mean deadrise here for a V-bottom)
    #[arg(long)]
    deadrise: Option<f64>,
    /// Write CSV to this path
    #[arg(long)]
    csv: Option<PathBuf>,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct StabilityArgs {
    file: PathBuf,
    /// Centre of gravity height above the keel
    #[arg(long, default_value_t = 1.0)]
    kg: f64,
    /// Heel angles in degrees, comma separated
    #[arg(long, default_value = "0,10,20,30,40,50,60,75,90")]
    heels: String,
    /// Displacements in tonnes, comma separated (defaults to the design one)
    #[arg(long)]
    displacements: Option<String>,
    /// Trim held fixed while heeling, in metres (positive bow-up)
    #[arg(long, default_value_t = 0.0)]
    trim: f64,
    /// Longitudinal centre of gravity in model x. Implies free trim: the hull
    /// trims to keep the LCB on this station. Defaults to the upright LCB.
    #[arg(long)]
    lcg: Option<f64>,
    /// Trim freely to longitudinal equilibrium instead of holding --trim
    #[arg(long)]
    free_trim: bool,
    /// Evaluate the IMO 2008 IS Code §2.3 severe wind and rolling (weather) criterion
    #[arg(long)]
    weather: bool,
    /// Bilge configuration for the weather criterion: round, sharp, or keels:<Ak>
    #[arg(long, default_value = "round")]
    bilge: String,
    /// Superstructure windage item name:area:z in m2 and m above keel
    /// (repeatable; defaults to the items in the model's project file)
    #[arg(long = "windage", value_name = "NAME:AREA:Z")]
    windage: Vec<String>,
    /// Downflooding angle in degrees for the weather criterion
    #[arg(long, value_name = "DEG")]
    downflooding: Option<f64>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Write CSV to this path (otherwise print a table)
    #[arg(long)]
    csv: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct LoadcaseArgs {
    /// Input .fef file
    file: PathBuf,
    /// Loading condition JSON, or a project file (defaults to the model's
    /// `<model>.seacraft.json` sidecar)
    case: Option<PathBuf>,
    /// Which loading condition of a project file (defaults to the first)
    #[arg(long)]
    name: Option<String>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct DamageArgs {
    /// Input .fef file
    file: PathBuf,
    /// Damage case JSON, or a project file (defaults to the model's
    /// `<model>.seacraft.json` sidecar)
    case: Option<PathBuf>,
    /// Which damage case of a project file (defaults to the first)
    #[arg(long)]
    name: Option<String>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
}

#[derive(Args, Debug)]
struct SeakeepingArgs {
    /// Input .fef file
    file: PathBuf,
    /// Draft in metres (required or defaults to project settings draft)
    #[arg(long)]
    draft: Option<f64>,
    /// Ship speed in knots
    #[arg(long, aliases = ["speed-kn"], default_value_t = 0.0)]
    speed: f64,
    /// Vertical centre of gravity (KG) in metres above keel
    #[arg(long)]
    kg: Option<f64>,
    /// Pitch radius of gyration (kyy) in metres (default: 0.25 * Lwl)
    #[arg(long)]
    kyy: Option<f64>,
    /// Number of stations along the waterline
    #[arg(long, default_value_t = 21)]
    stations: usize,
    /// Minimum wavelength ratio lambda / L
    #[arg(long, default_value_t = 0.3)]
    lambda_min: f64,
    /// Maximum wavelength ratio lambda / L
    #[arg(long, default_value_t = 3.0)]
    lambda_max: f64,
    /// Significant wave height in metres: with --tp, adds irregular head-sea responses
    #[arg(long)]
    hs: Option<f64>,
    /// Peak wave period in seconds (with --hs)
    #[arg(long)]
    tp: Option<f64>,
    /// JONSWAP peak enhancement factor (default: Bretschneider spectrum)
    #[arg(long)]
    gamma: Option<f64>,
    /// Extra response point NAME:X (model x in metres); repeatable. Default: LCG and Bow
    #[arg(long = "point", value_name = "NAME:X")]
    point: Vec<String>,
    /// Number of frequency points sampled across lambda / L
    #[arg(long, default_value_t = 30)]
    points: usize,
    /// Water density in kg/m3
    #[arg(long, default_value_t = 1025.0)]
    rho: f64,
    /// Acceleration due to gravity in m/s2
    #[arg(long, default_value_t = 9.81)]
    g: f64,
    /// Leave out the STF transom boundary terms at the aft end of the strip integration
    #[arg(long)]
    no_transom: bool,
    /// Write CSV to this path or emit to stdout if no path or '-' is given
    #[arg(long, num_args = 0..=1, default_missing_value = "-")]
    csv: Option<PathBuf>,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct ReportArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output HTML path
    #[arg(short, long)]
    output: PathBuf,
    /// Draft in metres (defaults to model's design draft)
    #[arg(long)]
    draft: Option<f64>,
    /// Centre of gravity height above keel in metres
    #[arg(long)]
    kg: Option<f64>,
    /// Longitudinal centre of gravity in model x
    #[arg(long)]
    lcg: Option<f64>,
    /// Speeds in knots, comma separated
    #[arg(long)]
    speeds: Option<String>,
    /// Exclude Holtrop-Mennen resistance analysis
    #[arg(long)]
    no_resistance: bool,
    /// Include Savitsky planing hull analysis
    #[arg(long)]
    planing: bool,
    /// Ignore the model's .seacraft.json sidecar
    #[arg(long)]
    no_project: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct McpArgs {
    /// Forward tool calls to a running `seacraft-gui --control HOST:PORT`
    #[arg(long, value_name = "HOST:PORT")]
    connect: Option<String>,
    /// Token file for authenticating with the running app (defaults to $XDG_RUNTIME_DIR/seacraft-control-<port>.token)
    #[arg(long, value_name = "PATH")]
    token_file: Option<PathBuf>,
    /// Hull to serve (defaults to the built-in demo hull)
    #[arg(long, value_name = "FILE")]
    open: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct LinesplanArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output path: `.svg` (default), `.dxf` (layered AutoCAD drawing) or `.igs` (B-spline curves)
    #[arg(short, long, default_value = "linesplan.svg")]
    output: PathBuf,
    /// Number of stations in the body plan
    #[arg(long, default_value_t = 11)]
    stations: usize,
    /// Number of buttocks in the profile view
    #[arg(long, default_value_t = 5)]
    buttocks: usize,
    /// Number of waterlines in the plan view
    #[arg(long, default_value_t = 7)]
    waterlines: usize,
    /// Draw porcupine curvature combs along the curves (SVG and DXF)
    #[arg(long)]
    combs: bool,
    /// Comb gain: metres of spike per 1/m of curvature
    #[arg(long, default_value_t = 0.6)]
    comb_scale: f64,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct TemplatesArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output path: `.pdf` (tiled over paper, or one page a pattern), `.svg` or `.dxf`
    #[arg(short, long)]
    output: PathBuf,
    /// Number of stations, at the middles of equal slices of the hull [default: 11]
    #[arg(long, conflicts_with_all = ["spacing", "at"])]
    stations: Option<usize>,
    /// A station at every multiple of this many metres of x inside the hull
    #[arg(long, value_name = "METRES", conflicts_with = "at")]
    spacing: Option<f64>,
    /// Stations at these x positions in metres, comma separated
    #[arg(long, value_name = "X,X,...")]
    at: Option<String>,
    /// Skin thickness to deduct, in millimetres: plank, plywood or strip plus glass
    #[arg(
        long,
        value_name = "MM",
        default_value_t = 0.0,
        allow_negative_numbers = true
    )]
    skin_mm: f64,
    /// Draw the starboard half only, to be flipped about the centreline
    #[arg(long)]
    half: bool,
    /// Strongback level, model z in metres: open sections get legs up to it
    #[arg(long, value_name = "Z", allow_negative_numbers = true)]
    strongback: Option<f64>,
    /// Waterline heights above the keel to mark, metres, comma separated [default: 6 over the depth]
    #[arg(long, value_name = "M,M,...")]
    waterlines: Option<String>,
    /// Add the profile (backbone) pattern
    #[arg(long)]
    profile: bool,
    /// Add a transom pattern: where the plane meets the baseline and how far its top leans aft
    #[arg(long, value_name = "X,RAKE_DEG", allow_negative_numbers = true)]
    transom: Option<String>,
    /// Paper the PDF is tiled over: a4, a3, letter or WxH in millimetres [default: a4]
    #[arg(long)]
    paper: Option<String>,
    /// One page a pattern, sized to fit it, for a plotter or print shop (PDF only)
    #[arg(long)]
    single_page: bool,
    /// The strip around each page a printer cannot reach, millimetres [default: 10]
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    margin_mm: Option<f64>,
    /// How much neighbouring tiles share, millimetres [default: 10]
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    overlap_mm: Option<f64>,
    /// Drawing scale, as 1:N or one number
    #[arg(long, default_value = "1:1")]
    scale: String,
    /// Add imperial readings beside the metric ones
    #[arg(long)]
    imperial: bool,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct PanelsArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output path: `.pdf` (tiled over paper, or one page a panel), `.svg` or `.dxf`
    #[arg(short, long)]
    output: PathBuf,
    /// A layer to make panels from; repeat for more [default: the layers marked developable, or
    /// every layer used in hydrostatics when none is]
    #[arg(long = "layer", value_name = "NAME")]
    layers: Vec<String>,
    /// Number of stations to mark, at the middles of equal slices of the hull [default: 11]
    #[arg(long, conflicts_with_all = ["spacing", "at"])]
    stations: Option<usize>,
    /// A station at every multiple of this many metres of x inside the hull
    #[arg(long, value_name = "METRES", conflicts_with = "at")]
    spacing: Option<f64>,
    /// Stations at these x positions in metres, comma separated
    #[arg(long, value_name = "X,X,...")]
    at: Option<String>,
    /// Paper the PDF is tiled over: a4, a3, letter or WxH in millimetres [default: a4]
    #[arg(long)]
    paper: Option<String>,
    /// One page a panel, sized to fit it, for a plotter or print shop (PDF only)
    #[arg(long)]
    single_page: bool,
    /// The strip around each page a printer cannot reach, millimetres [default: 10]
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    margin_mm: Option<f64>,
    /// How much neighbouring tiles share, millimetres [default: 10]
    #[arg(long, value_name = "MM", allow_negative_numbers = true)]
    overlap_mm: Option<f64>,
    /// Drawing scale, as 1:N or one number
    #[arg(long, default_value = "1:1")]
    scale: String,
    /// Add imperial readings beside the metric ones
    #[arg(long)]
    imperial: bool,
    /// A panel whose flat seam length drifts from its length on the model by more than this does
    /// not lie flat, millimetres
    #[arg(
        long,
        value_name = "MM",
        default_value_t = 2.0,
        allow_negative_numbers = true
    )]
    max_seam_drift_mm: f64,
    /// A panel that must stretch more than this (RMS) to lie flat does not, per cent
    #[arg(
        long,
        value_name = "PCT",
        default_value_t = 0.1,
        allow_negative_numbers = true
    )]
    max_stretch_pct: f64,
    /// Rows of the panel offsets every this many millimetres along the baseline
    #[arg(
        long,
        value_name = "MM",
        default_value_t = 250.0,
        allow_negative_numbers = true
    )]
    offset_spacing_mm: f64,
    /// Also write the panel offsets as CSV
    #[arg(long, value_name = "FILE")]
    offsets_csv: Option<PathBuf>,
    /// Also write the panel offsets as PDF tables
    #[arg(long, value_name = "FILE")]
    offsets_pdf: Option<PathBuf>,
    /// The plywood sheet, LENGTHxWIDTH in millimetres
    #[arg(long, value_name = "LxW", default_value = "2440x1220")]
    stock_mm: String,
    /// How much joined sheets overlap at a scarf, millimetres; 0 for butt joints
    #[arg(
        long,
        value_name = "MM",
        default_value_t = 0.0,
        allow_negative_numbers = true
    )]
    scarf_mm: f64,
    /// Write the patterns even when a panel does not lie flat; each such pattern is headed so
    #[arg(long)]
    allow_undevelopable: bool,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Demo(a) => cmd_demo(a),
        Command::Example(a) => cmd_example(a),
        Command::Generate(a) => cmd_generate(a),
        Command::Info(a) => cmd_info(a),
        Command::Hydro(a) => cmd_hydro(a),
        Command::Export(a) => cmd_export(a),
        Command::Subdiv(a) => cmd_subdiv(a),
        Command::Sac(a) => cmd_sac(a),
        Command::Bonjean(a) => cmd_bonjean(a),
        Command::Offsets(a) => cmd_offsets(a),
        Command::Transform(a) => cmd_transform(a),
        Command::Lackenby(a) => cmd_lackenby(a),
        Command::Table(a) => cmd_table(a),
        Command::Trim(a) => cmd_trim(a),
        Command::Optimize(a) => cmd_optimize(a),
        Command::Resistance(a) => cmd_resistance(a),
        Command::Unfold(a) => cmd_unfold(a),
        Command::Edit(a) => cmd_edit(a),
        Command::Planing(a) => cmd_planing(a),
        Command::Stability(a) => cmd_stability(a),
        Command::Loadcase(a) => cmd_loadcase(a),
        Command::Damage(a) => cmd_damage(a),
        Command::Seakeeping(a) => cmd_seakeeping(a),
        Command::Linesplan(a) => cmd_linesplan(a),
        Command::Templates(a) => cmd_templates(a),
        Command::Panels(a) => cmd_panels(a),
        Command::Report(a) => cmd_report(a),
        Command::Render(a) => cmd_render(a),
        Command::Mcp(a) => cmd_mcp(a),
    }
}

fn load(path: &Path) -> Result<Surface> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let surface = fef::parse(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    Ok(surface)
}

fn cmd_demo(a: DemoArgs) -> Result<()> {
    let surface = demo::demo_hull(a.symmetric);
    seacraft_core::io::write_atomic(&a.output, fef::write(&surface))
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    // The demo's KG travels with it, as for the examples, so opening the
    // file gives the same stability as the built-in demo.
    let sidecar = project::sidecar_path(&a.output);
    demo::demo_project()
        .save(&sidecar)
        .map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "wrote {} and {} ({} control points, {} faces, {} layer{})",
        a.output.display(),
        sidecar.display(),
        surface.points.len(),
        surface.faces.len(),
        surface.layers.len(),
        if surface.layers.len() == 1 { "" } else { "s" }
    );
    Ok(())
}

fn cmd_example(a: ExampleArgs) -> Result<()> {
    use seacraft_core::examples::Example;
    if a.list {
        let list = Example::with_benchmarks();
        if a.json {
            let rows: Vec<serde_json::Value> = list
                .iter()
                .map(|e| {
                    serde_json::json!({
                        "name": e.slug(),
                        "title": e.title(),
                        "description": e.description(),
                        "source": e.source(),
                        "benchmark": e.is_benchmark(),
                    })
                })
                .collect();
            println!("{}", serde_json::to_string_pretty(&rows)?);
        } else {
            for e in list {
                println!("{:<16} {}: {}", e.slug(), e.title(), e.description());
            }
        }
        return Ok(());
    }
    if a.all {
        let dir = a
            .output
            .context("--all needs the output directory: -o DIR")?;
        std::fs::create_dir_all(&dir)
            .with_context(|| format!("cannot create {}", dir.display()))?;
        for e in Example::with_benchmarks() {
            write_example(e, &dir.join(format!("{}.fef", e.slug())))?;
        }
        return Ok(());
    }
    let Some(name) = a.name else {
        bail!("give an example name, --list or --all");
    };
    let kind = Example::from_slug(&name).map_err(|e| anyhow::anyhow!(e))?;
    let out = a
        .output
        .unwrap_or_else(|| PathBuf::from(format!("{}.fef", kind.slug())));
    write_example(kind, &out)
}

/// Write an example's `.fef` and its sidecar next to it.
fn write_example(kind: seacraft_core::examples::Example, out: &Path) -> Result<()> {
    let (surface, project) =
        seacraft_core::examples::build(kind).map_err(|e| anyhow::anyhow!(e))?;
    seacraft_core::io::write_atomic(out, fef::write(&surface))
        .with_context(|| format!("cannot write {}", out.display()))?;
    let sidecar = project::sidecar_path(out);
    project.save(&sidecar).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "wrote {} and {} ({}: {} control points, {} faces, {} layers)",
        out.display(),
        sidecar.display(),
        kind.title(),
        surface.points.len(),
        surface.faces.len(),
        surface.layers.len()
    );
    Ok(())
}

fn cmd_generate(a: GenerateArgs) -> Result<()> {
    let kind: HullKind = a.kind.into();
    let mut spec = HullSpec::default_for(kind);
    if let Some(lwl) = a.lwl {
        spec.lwl = lwl;
    }
    if let Some(beam) = a.beam {
        spec.beam = beam;
    }
    if let Some(draft) = a.draft {
        spec.draft = draft;
    }
    if let Some(depth) = a.depth {
        spec.depth = depth;
    }
    if let Some(cb) = a.cb {
        spec.cb = Some(cb);
    }
    if let Some(lcb) = a.lcb {
        spec.lcb_frac = Some(lcb);
    }
    if let Some(deadrise) = a.deadrise {
        spec.deadrise_deg = deadrise;
    }
    if let Some(transom) = a.transom {
        spec.transom_frac = transom;
    }
    if let Some(stations) = a.stations {
        spec.stations = stations;
    }
    if let Some(pps) = a.points_per_station {
        spec.points_per_station = pps;
    }

    let (surface, rep) = parametric::generate(&spec).map_err(|e| anyhow::anyhow!(e))?;
    seacraft_core::io::write_atomic(&a.output, fef::write(&surface))
        .with_context(|| format!("cannot write {}", a.output.display()))?;

    if a.json {
        println!("{}", serde_json::to_string_pretty(&rep)?);
        return Ok(());
    }

    println!("parametric hull ({}) -> {}", rep.kind, a.output.display());
    println!("  parameter       target     achieved");
    println!("  LWL (m)       {:8.3}     {:8.3}", rep.target_lwl, rep.lwl);
    println!("  BWL (m)       {:8.3}     {:8.3}", rep.target_bwl, rep.bwl);
    println!("  draft T (m)   {:8.3}     {:8.3}", rep.target_draft, rep.t);
    println!(
        "  depth D (m)   {:8.3}     {:8.3}",
        rep.target_depth, rep.target_depth
    );
    match rep.target_cb {
        Some(tcb) => println!("  Cb            {:8.4}     {:8.4}", tcb, rep.cb),
        None => println!("  Cb                   -     {:8.4}", rep.cb),
    }
    match rep.target_lcb_frac {
        Some(tlcb) => println!("  LCB / LWL     {:8.4}     {:8.4}", tlcb, rep.lcb_frac),
        None => println!("  LCB / LWL            -     {:8.4}", rep.lcb_frac),
    }
    println!("  volume (m3)          -     {:8.3}", rep.volume);
    println!("  disp (t)             -     {:8.3}", rep.displacement_t);
    println!("  iterations           -     {:8}", rep.iterations);
    Ok(())
}

fn cmd_info(a: InfoArgs) -> Result<()> {
    let s = load(&a.file)?;
    let (min, max) = s.extents();
    let counts = s.edge_face_counts();
    let boundary_edges = counts.iter().filter(|&&c| c == 1).count();
    let non_manifold = counts.iter().filter(|&&c| c > 2).count();
    let creases = s.edges.iter().filter(|e| e.crease).count();
    let quads = s.faces.iter().filter(|f| f.len() == 4).count();
    let tris = s.faces.iter().filter(|f| f.len() == 3).count();
    let other = s.faces.len() - quads - tris;

    println!("{}", s.settings.name);
    if !s.settings.designer.is_empty() {
        println!("  designer      {}", s.settings.designer);
    }
    println!(
        "  principal      L {:.3}  B {:.3}  T {:.3} m   rho {:.0} kg/m3  appendage {:.3}",
        s.settings.length,
        s.settings.beam,
        s.settings.draft,
        s.settings.water_density,
        s.settings.appendage_coefficient
    );
    println!(
        "  extent         x {:.3}..{:.3}  y {:.3}..{:.3}  z {:.3}..{:.3}",
        min.x, max.x, min.y, max.y, min.z, max.z
    );
    println!(
        "  control net    {} points, {} edges ({} crease, {} boundary, {} non-manifold), {} faces",
        s.points.len(),
        s.edges.len(),
        creases,
        boundary_edges,
        non_manifold,
        s.faces.len()
    );
    println!("  face mix       {quads} quads, {tris} triangles, {other} n-gons");
    println!("  control area   {:.3} m2", s.control_area());
    println!("  layers");
    for l in &s.layers {
        println!(
            "    {:<16} id {:<3} {}{}{} rho {:.0} t {:.3}",
            l.name,
            l.id,
            if l.visible { "visible " } else { "hidden  " },
            if l.symmetric {
                "symmetric "
            } else {
                "asymmetric "
            },
            if l.use_in_hydrostatics { "hydro" } else { "-" },
            l.density,
            l.thickness
        );
    }
    Ok(())
}

fn hydro_options(a: &HydroArgs, s: &Surface) -> HydroOptions {
    let mut o = HydroOptions::from_surface(
        s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    if let Some(d) = a.draft {
        o.draft = d;
    }
    o.trim = a.trim;
    o.heel_deg = a.heel;
    if let Some(d) = a.density {
        o.water_density = d;
    }
    if let Some(x) = a.appendage {
        o.appendage_coefficient = x;
    }
    // The flag overrides; without it the project setting stands (0 = the LCF).
    // Overwriting it with `None` here silently dropped a model's own mainframe.
    o.mainframe_x = a.mainframe.or(o.mainframe_x);
    o.stations = a.stations;
    o
}

fn cmd_hydro(a: HydroArgs) -> Result<()> {
    let s = load(&a.file)?;
    let opts = hydro_options(&a, &s);
    if opts.draft <= 0.0 {
        bail!("draft must be positive (use --draft, or set it in the project settings)");
    }
    let free = a.free_trim || a.lcg.is_some();
    let result = if free {
        let hull = hydro::Hull::new(&s, opts.subdivision);
        hydrostatics_free_trim(&hull, &opts, a.lcg).0
    } else {
        hydro::compute(&s, &opts)
    };
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    if free {
        println!(
            "{}  -  hydrostatics at T {:.3} m, trim {:+.3} m (free to trim)",
            s.settings.name, result.draft, result.trim_m
        );
    } else {
        println!(
            "{}  -  hydrostatics at T {:.3} m",
            s.settings.name, result.draft
        );
    }
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row("volume", format!("{:.3} m3", result.volume_m3));
    row(
        "displacement",
        format!(
            "{:.0} kg  ({:.3} t)",
            result.displacement_kg, result.displacement_t
        ),
    );
    row(
        "block coefficient Cb",
        format!("{:.4}", result.block_coefficient),
    );
    row(
        "prismatic Cp",
        format!("{:.4}", result.prismatic_coefficient),
    );
    row("midship Cm", format!("{:.4}", result.midship_coefficient));
    row(
        "waterplane Cwp",
        format!("{:.4}", result.waterplane_coefficient),
    );
    row(
        "vertical prismatic Cvp",
        format!("{:.4}", result.vertical_prismatic_coefficient),
    );
    println!();
    row(
        "LCB",
        format!(
            "x {:.3}  y {:.3}  z {:.3} m",
            result.lcb.x, result.lcb.y, result.lcb.z
        ),
    );
    row("KB", format!("{:.3} m above keel", result.kb));
    row(
        "LCF",
        format!("x {:.3}  y {:.3}", result.lcf.x, result.lcf.y),
    );
    println!();
    row(
        "LWL x BWL",
        format!("{:.3} x {:.3} m", result.lwl, result.bwl),
    );
    row(
        "wetted surface",
        format!("{:.3} m2", result.wetted_surface_m2),
    );
    row(
        "waterplane area",
        format!("{:.3} m2", result.waterplane_area_m2),
    );
    row(
        "waterplane I_T / I_L",
        format!(
            "{:.1} / {:.1} m4",
            result.waterplane_inertia_transverse, result.waterplane_inertia_longitudinal
        ),
    );
    row("BM transverse", format!("{:.3} m", result.bm_transverse));
    row(
        "BM longitudinal",
        format!("{:.3} m", result.bm_longitudinal),
    );
    row("KM transverse", format!("{:.3} m", result.km_transverse));
    row(
        "KM longitudinal",
        format!("{:.3} m", result.km_longitudinal),
    );
    if let Some(kg) = a.kg {
        row(
            &format!("GM (KG {:.3})", kg),
            format!("{:.3} m", result.gm(kg)),
        );
    }
    println!();
    row(
        "midship section area",
        format!(
            "{:.4} m2 at x {:.3}",
            result.mainframe_area_m2, result.mainframe_x
        ),
    );
    row("lateral area", format!("{:.3} m2", result.lateral_area_m2));
    if !result.sac.is_empty() {
        println!();
        println!("  section area curve ({} stations)", result.sac.len());
        for p in &result.sac {
            let bar = (p.area / result.mainframe_area_m2.max(1e-9) * 40.0).round() as usize;
            println!(
                "  x {:>7.3}  {:>7.4} m2  {}",
                p.x,
                p.area,
                "#".repeat(bar.min(60))
            );
        }
    }
    if !result.warnings.is_empty() {
        println!();
        for w in &result.warnings {
            println!("  warning [{}] {}", w.code, w.message);
        }
    }
    Ok(())
}

/// The `export` flags the format at `ext` cannot honour: `--control`,
/// `--no-mirror` and `--up y` are OBJ-only, and writing a mirrored, refined,
/// z-up STL when the user asked otherwise would be a silent lie.
fn export_flags_error(
    ext: &str,
    control: bool,
    no_mirror: bool,
    up: Option<UpArg>,
) -> Option<String> {
    // OBJ honours all three; an unknown extension gets its own error later.
    if !matches!(ext, "stl" | "3mf" | "ply" | "igs" | "iges" | "html" | "htm") {
        return None;
    }
    let mut refused = Vec::new();
    if control {
        refused.push("--control");
    }
    if no_mirror {
        refused.push("--no-mirror");
    }
    if up == Some(UpArg::Y) {
        refused.push("--up y");
    }
    if refused.is_empty() {
        return None;
    }
    Some(format!(
        "{} {} only for .obj output; .{ext} is always the refined, mirrored, z-up surface \
         (drop the flag or write .obj)",
        refused.join(", "),
        if refused.len() == 1 {
            "applies"
        } else {
            "apply"
        },
    ))
}

fn cmd_export(a: ExportArgs) -> Result<()> {
    let ext = a
        .output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    if let Some(msg) = export_flags_error(&ext, a.control, a.no_mirror, a.up) {
        bail!(msg);
    }
    let s = load(&a.file)?;
    let mode: SubdivisionMode = a.subdiv.mode.into();
    let up = if a.up == Some(UpArg::Y) {
        export::UpAxis::Y
    } else {
        export::UpAxis::Z
    };
    let data: Vec<u8> = match ext.as_str() {
        "obj" => {
            let text = if a.no_mirror {
                obj_from_unmirrored(&s, a.subdiv.level, mode, a.control, up)
            } else if a.control {
                export::obj_control(&s, up)
            } else {
                export::obj(&s, a.subdiv.level, mode, up)
            };
            text.into_bytes()
        }
        "stl" => export::stl(&s, a.subdiv.level, mode),
        "3mf" => export::threemf(&s, a.subdiv.level, mode),
        "ply" => export::ply(&s, a.subdiv.level, mode).into_bytes(),
        "igs" | "iges" => export::iges_surface(&s, a.subdiv.level, mode).into_bytes(),
        "html" | "htm" => export::viewer_html(&s, a.subdiv.level, mode).into_bytes(),
        other => bail!(
            "unsupported output format '{}' (use .obj, .stl, .3mf, .ply, .igs or .html)",
            other
        ),
    };
    seacraft_core::io::write_atomic(&a.output, &data)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!(
        "wrote {} ({:.1} KiB)",
        a.output.display(),
        data.len() as f64 / 1024.0
    );
    Ok(())
}

/// OBJ without mirroring symmetric layers, for inspecting a half model.
/// The modelled half only (symmetric layers not mirrored): the refined mesh,
/// or the control net itself with `control`, in the requested axis order.
fn obj_from_unmirrored(
    s: &Surface,
    level: u8,
    mode: SubdivisionMode,
    control: bool,
    up: export::UpAxis,
) -> String {
    let level = if control { 0 } else { level };
    let mesh = evaluate(s, SubdivisionOptions { level, mode });
    let mut out = String::from("# SeaCraft - unmirrored\n");
    for v in &mesh.verts {
        let (a, b, c) = up.map(*v);
        out.push_str(&format!("v {a:.6} {b:.6} {c:.6}\n"));
    }
    for f in &mesh.faces {
        out.push('f');
        for &i in f {
            out.push_str(&format!(" {}", i + 1));
        }
        out.push('\n');
    }
    out
}

fn cmd_subdiv(a: SubdivArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mut mesh = evaluate(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    if a.mirror {
        let sym: Vec<bool> = s.layers.iter().map(|l| l.symmetric).collect();
        mesh.mirror_symmetric(&sym);
    }
    let mut baked = s.clone();
    baked.points = mesh
        .verts
        .iter()
        .map(|p| seacraft_core::mesh::ControlPoint {
            p: *p,
            kind: seacraft_core::mesh::VertexKind::Regular,
        })
        .collect();
    baked.faces = mesh.faces.clone();
    baked.face_layer = if mesh.face_layer.is_empty() {
        vec![0; mesh.faces.len()]
    } else {
        mesh.face_layer.clone()
    };
    baked.edges.clear();
    baked.rebuild_edges();
    seacraft_core::io::write_atomic(&a.output, fef::write(&baked))
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!(
        "wrote {} ({} points, {} faces)",
        a.output.display(),
        baked.points.len(),
        baked.faces.len()
    );
    Ok(())
}

fn cmd_sac(a: SacArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mut opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    if let Some(d) = a.draft {
        opts.draft = d;
    }
    opts.stations = a.stations;
    if opts.draft <= 0.0 {
        bail!("draft must be positive (use --draft)");
    }
    let result = hydro::compute(&s, &opts);
    if let Some(path) = &a.svg {
        let title = format!(
            "{} - sectional area curve, T = {:.3} m",
            s.settings.name, opts.draft
        );
        seacraft_core::io::write_atomic(path, curves::sac_svg(&result.sac, &title))
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    if a.json {
        let cp = curves::prismatic_from_sac(&result.sac);
        let span = result
            .sac
            .last()
            .zip(result.sac.first())
            .map(|(b, a)| b.x - a.x)
            .unwrap_or(0.0);
        let body = serde_json::json!({
            "draft": opts.draft,
            "length": span,
            "area_max": result.sac.iter().map(|p| p.area).fold(0.0_f64, f64::max),
            "prismatic_coefficient": cp,
            "stations": result.sac,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    match a.csv {
        Some(path) => {
            let mut text = String::from("x,area_m2\n");
            for p in &result.sac {
                text.push_str(&format!("{:.4},{:.6}\n", p.x, p.area));
            }
            seacraft_core::io::write_atomic(&path, text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("wrote {} ({} stations)", path.display(), result.sac.len());
        }
        None => {
            println!("x,area_m2");
            for p in &result.sac {
                println!("{:.4},{:.6}", p.x, p.area);
            }
        }
    }
    Ok(())
}

fn parse_drafts(text: &str) -> Result<Vec<f64>> {
    let mut drafts: Vec<f64> = text
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("invalid draft list '{text}'"))?;
    drafts.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if drafts.len() < 2 {
        bail!("at least two drafts are needed for a Bonjean family");
    }
    if drafts[0] <= 0.0 {
        bail!("drafts must be positive");
    }
    Ok(drafts)
}

fn cmd_bonjean(a: BonjeanArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mut opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    if let Some(d) = a.draft {
        opts.draft = d;
    }
    let hull = seacraft_core::hydro::Hull::new(&s, opts.subdivision);
    let depth = hull.model_max.z - hull.keel;
    let drafts = match &a.drafts {
        Some(text) => parse_drafts(text)?,
        None => {
            if depth <= 0.0 {
                bail!("hull has no height above the keel; pass --drafts");
            }
            curves::draft_range(depth / a.draft_steps.max(2) as f64, depth, a.draft_steps)
        }
    };
    let bonjean = curves::bonjean(&hull, a.stations, &drafts);

    if let Some(path) = &a.svg {
        let title = format!(
            "{} - Bonjean curves ({} stations, design draft {:.3} m)",
            s.settings.name,
            bonjean.sections.len(),
            opts.draft
        );
        seacraft_core::io::write_atomic(path, curves::bonjean_svg(&bonjean, &title))
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    if let Some(path) = &a.csv {
        let mut text = String::from("x,draft_m,area_m2\n");
        for sec in &bonjean.sections {
            for (i, &d) in bonjean.drafts_m.iter().enumerate() {
                text.push_str(&format!("{:.4},{:.4},{:.6}\n", sec.x, d, sec.areas_m2[i]));
            }
        }
        seacraft_core::io::write_atomic(path, text)
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!(
            "wrote {} ({} stations)",
            path.display(),
            bonjean.sections.len()
        );
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&bonjean)?);
        return Ok(());
    }
    if a.csv.is_some() || a.svg.is_some() {
        // Files were requested: a short confirmation is enough.
        println!(
            "{} stations x {} drafts, max area {:.3} m², hull depth {:.3} m",
            bonjean.sections.len(),
            bonjean.drafts_m.len(),
            bonjean.max_area(),
            depth
        );
        return Ok(());
    }

    // Terminal table: one row per station, one column per draft.
    print!("{:>8}", "x [m]");
    for d in &bonjean.drafts_m {
        print!("{:>8}", format!("{d:.2}"));
    }
    println!();
    for sec in &bonjean.sections {
        print!("{:>8.3}", sec.x);
        for area in &sec.areas_m2 {
            print!("{:>8.3}", area);
        }
        println!();
    }
    println!(
        "draft [m] across the columns; max area {:.3} m² at the deepest draft",
        bonjean.max_area()
    );
    Ok(())
}

fn parse_ordinates(text: &str, key: &str) -> Result<Vec<f64>> {
    let mut out: Vec<f64> = text
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("invalid {key} list '{text}'"))?;
    out.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    if out.is_empty() {
        bail!("{key} must list at least one ordinate");
    }
    if out.iter().any(|v| *v <= 0.0 || !v.is_finite()) {
        bail!("{key} must be positive");
    }
    Ok(out)
}

fn cmd_offsets(a: OffsetsArgs) -> Result<()> {
    if a.imperial && a.pdf.is_none() {
        bail!("--imperial applies to --pdf: the other outputs are in metres");
    }
    let s = load(&a.file)?;
    let opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    let hull = seacraft_core::hydro::Hull::new(&s, opts.subdivision);
    let waterlines = match &a.waterlines {
        Some(text) => parse_ordinates(text, "waterlines")?,
        None => offsets::default_waterlines(&hull, a.waterline_steps),
    };
    let buttocks = match &a.buttocks {
        Some(text) => parse_ordinates(text, "buttocks")?,
        None => offsets::default_buttocks(&hull, a.buttock_steps),
    };
    let table = offsets::table_of_offsets(&hull, a.stations, &waterlines, &buttocks);
    let title = format!("{} - table of offsets", s.settings.name);
    // With --json the standard output is the table alone: what was written goes to stderr.
    let written = |line: String| {
        if a.json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };

    if let Some(path) = &a.pdf {
        let doc = offsets::pdf(&table, &title, a.imperial).map_err(anyhow::Error::msg)?;
        seacraft_core::io::write_atomic(path, doc.bytes)
            .with_context(|| format!("cannot write {}", path.display()))?;
        written(format!("wrote {}", path.display()));
    }

    if let Some(path) = &a.html {
        seacraft_core::io::write_atomic(path, offsets::html(&table, &title))
            .with_context(|| format!("cannot write {}", path.display()))?;
        written(format!("wrote {}", path.display()));
    }
    if let Some(path) = &a.csv {
        // Long format: one row per (station, waterline, buttock) reading would
        // be three tables; keep it to two blocks with a type column.
        let mut text = String::from("kind,station_x,ordinate,value\n");
        for st in &table.stations {
            for (i, wl) in table.waterlines_m.iter().enumerate() {
                if let Some(y) = st.waterline_half_breadths[i] {
                    text.push_str(&format!("half_breadth,{:.4},{:.4},{:.6}\n", st.x, wl, y));
                }
            }
            for (i, b) in table.buttocks_m.iter().enumerate() {
                if let Some(z) = st.buttock_heights[i] {
                    text.push_str(&format!("buttock_height,{:.4},{:.4},{:.6}\n", st.x, b, z));
                }
            }
        }
        seacraft_core::io::write_atomic(path, text)
            .with_context(|| format!("cannot write {}", path.display()))?;
        written(format!(
            "wrote {} ({} stations x {} waterlines)",
            path.display(),
            table.stations.len(),
            table.waterlines_m.len()
        ));
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&table)?);
        return Ok(());
    }
    if (a.html.is_some() || a.pdf.is_some()) && a.csv.is_none() {
        println!(
            "{} stations x {} waterlines, widest half-breadth {:.3} m",
            table.stations.len(),
            table.waterlines_m.len(),
            table.max_half_breadth()
        );
        return Ok(());
    }

    println!("{}", title);
    println!("  half-breadths [m], waterlines above the keel\n");
    print!("{:>8}", "x [m]");
    for wl in &table.waterlines_m {
        print!("{:>8}", format!("WL{wl:.2}"));
    }
    println!("{:>8}{:>8}{:>8}", "keel", "sheer", "B/2");
    for st in &table.stations {
        print!("{:>8.3}", st.x);
        for v in &st.waterline_half_breadths {
            match v {
                Some(y) => print!("{:>8.3}", y),
                None => print!("{:>8}", "-"),
            }
        }
        println!("{:>8.3}{:>8.3}{:>8.3}", st.keel_z, st.deck_z, st.half_beam);
    }
    println!("\n  buttock heights [m, model z]\n");
    print!("{:>8}", "x [m]");
    for b in &table.buttocks_m {
        print!("{:>8}", format!("B{b:.2}"));
    }
    println!();
    for st in &table.stations {
        print!("{:>8.3}", st.x);
        for v in &st.buttock_heights {
            match v {
                Some(z) => print!("{:>8.3}", z),
                None => print!("{:>8}", "-"),
            }
        }
        println!();
    }
    println!(
        "- means the section does not reach that ordinate; widest half-breadth {:.3} m",
        table.max_half_breadth()
    );
    Ok(())
}

fn parse_triple(text: &str, key: &str) -> Result<Vec3> {
    let parts: Vec<f64> = text
        .split(',')
        .map(|t| t.trim())
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>())
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("{key} must be three comma-separated numbers"))?;
    if parts.len() != 3 {
        bail!(
            "{key} must be three comma-separated numbers, got {}",
            parts.len()
        );
    }
    Ok(Vec3::new(parts[0], parts[1], parts[2]))
}

fn cmd_transform(a: TransformArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.level,
        mode: if a.catmull_clark {
            SubdivisionMode::CatmullClark
        } else {
            SubdivisionMode::Loop
        },
    };
    let targets = transform::Targets {
        lwl: a.lwl,
        bwl: a.bwl,
        draft: a.draft,
    };
    let scale = a
        .scale
        .as_deref()
        .map(|t| parse_triple(t, "--scale"))
        .transpose()?;
    let translate = a
        .translate
        .as_deref()
        .map(|t| parse_triple(t, "--translate"))
        .transpose()?
        .unwrap_or(Vec3::ZERO);
    if targets.is_empty() && scale.is_none() && translate == Vec3::ZERO && a.mirror.is_none() {
        bail!("nothing to do: give --lwl/--bwl/--draft, --scale, --translate or --mirror");
    }

    let request = transform::TransformRequest {
        lwl: a.lwl,
        bwl: a.bwl,
        draft: a.draft,
        scale,
        translate,
        mirror: a.mirror.map(|m| match m {
            MirrorArg::X => transform::Axis::X,
            MirrorArg::Y => transform::Axis::Y,
            MirrorArg::Z => transform::Axis::Z,
        }),
    };
    let (out, report) = transform::run(&s, &request, subdiv).map_err(|e| anyhow::anyhow!(e))?;
    let before = report.before;
    let after = report.after;
    let fit = report.fit.clone();
    seacraft_core::io::write_atomic(&a.output, fef::write(&out))
        .with_context(|| format!("cannot write {}", a.output.display()))?;

    if a.json {
        let body = serde_json::json!({
            "output": a.output.display().to_string(),
            "fit": fit,
            "before": before,
            "after": after,
            "settings": {
                "length": out.settings.length,
                "beam": out.settings.beam,
                "draft": out.settings.draft,
                "mainframe_location": out.settings.mainframe_location,
            },
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    let line = |label: &str, d: Option<transform::MainDimensions>| match d {
        Some(d) => println!(
            "  {label:<7} LWL {:7.3}  BWL {:6.3}  T {:5.3}  V {:8.3} m3",
            d.lwl, d.bwl, d.draft, d.volume_m3
        ),
        None => println!("  {label:<7} (no waterline: draft is zero)"),
    };
    println!("{}", s.settings.name);
    line("before", before);
    line("after", after);
    if let Some(f) = &fit {
        println!(
            "  fit      {} iteration(s), scale {:.4} x {:.4} x {:.4}, residual {:.3} %",
            f.iterations,
            f.scale.x,
            f.scale.y,
            f.scale.z,
            f.max_error * 100.0
        );
    }
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_resistance(a: ResistanceArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
    };
    let mut opts = HydroOptions::from_surface(&s, subdiv);
    if let Some(d) = a.draft {
        opts.draft = d;
    }
    let hull = seacraft_core::hydro::Hull::new(&s, subdiv);

    if a.method == ResistanceMethodArg::Michell {
        let grid = michell::michell_grid_from_surface(&s, opts.draft, 65, 25, subdiv)
            .context("cannot extract Michell offsets grid: positive draft and waterline needed")?;
        let speeds = parse_ordinates(&a.speeds, "speeds")?;
        let holtrop_in = resistance::HoltropInput::measure(&hull, &opts);
        let form_factor = if let Some(ref h_in) = holtrop_in {
            let r = resistance::holtrop(h_in, 1.0);
            if h_in.warnings().is_empty() {
                r.form_factor
            } else {
                1.0
            }
        } else {
            1.0
        };
        let viscosity = if (opts.water_density - 1000.0).abs() < 25.0 {
            resistance::VISCOSITY_FRESH_WATER
        } else {
            resistance::VISCOSITY_SEA_WATER
        };
        let mopts = michell::MichellOptions {
            nx: 65,
            nz: 25,
            ntheta: 128,
            catamaran_spacing: a.cat_spacing,
        };
        let rows = michell::michell_resistance_curve(
            &grid,
            form_factor,
            viscosity,
            opts.water_density,
            resistance::GRAVITY,
            &speeds,
            &mopts,
        );

        let mut warnings = Vec::new();
        let lb = if grid.beam_wl > 1e-6 {
            grid.length_wl / grid.beam_wl
        } else {
            0.0
        };
        if lb < 6.0 {
            warnings.push(format!(
                "L/B {lb:.2} is outside thin-ship recommended range (L/B >= 6.0)"
            ));
        }
        for r in &rows {
            if !(0.20..=1.0).contains(&r.fn_) {
                warnings.push(format!(
                    "Fn {:.3} at {:.2} kn is outside typical Michell applicability range (0.20-1.0)",
                    r.fn_, r.speed_kn
                ));
            }
        }

        if let Some(path) = &a.csv {
            let mut text = String::from(
                "speed_kn,speed_m_s,fn,re,cf,form_factor,rf_n,rapp_n,rw_n,rb_n,rtr_n,ra_n,rt_n,pe_w\n",
            );
            for r in &rows {
                text.push_str(&format!(
                    "{:.3},{:.4},{:.5},{:.0},{:.6},{:.4},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1}\n",
                    r.speed_kn, r.speed_m_s, r.fn_, r.reynolds, r.cf, r.form_factor, r.rf_n,
                    r.rapp_n, r.rw_n, r.rb_n, r.rtr_n, r.ra_n, r.rt_n, r.pe_w
                ));
            }
            seacraft_core::io::write_atomic(path, text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("wrote {} ({} speeds)", path.display(), rows.len());
        }

        if a.json {
            let body = serde_json::json!({
                "method": "michell",
                "cat_spacing": a.cat_spacing,
                "length_wl": grid.length_wl,
                "beam_wl": grid.beam_wl,
                "draft": grid.draft,
                "wetted_surface_m2": grid.wetted_surface_m2,
                "form_factor": form_factor,
                "warnings": warnings,
                "speeds": rows,
            });
            println!("{}", serde_json::to_string_pretty(&body)?);
            return Ok(());
        }
        if a.csv.is_some() {
            return Ok(());
        }

        println!(
            "{}  -  Michell wave resistance (thin-ship theory), T {:.3} m{}",
            s.settings.name,
            grid.draft,
            if let Some(sp) = a.cat_spacing {
                format!("  [Catamaran spacing: {sp:.2} m]")
            } else {
                String::new()
            }
        );
        println!(
            "  LWL {:.3}  BWL {:.3}  S {:.2} m2  L/B {:.2}  1+k {:.3}",
            grid.length_wl, grid.beam_wl, grid.wetted_surface_m2, lb, form_factor
        );
        for w in &warnings {
            println!("  warning: {w}");
        }
        println!();
        println!(
            "  {:>6}  {:>6}  {:>6}  {:>10}  {:>10}  {:>9}  {:>9}",
            "kn", "Fn", "1+k1", "RT [kN]", "PE [kW]", "RF [kN]", "RW [kN]"
        );
        for r in &rows {
            println!(
                "  {:>6.2}  {:>6.3}  {:>6.3}  {:>10.2}  {:>10.2}  {:>9.2}  {:>9.2}",
                r.speed_kn,
                r.fn_,
                r.form_factor,
                r.rt_n / 1000.0,
                r.pe_w / 1000.0,
                r.rf_n / 1000.0,
                r.rw_n / 1000.0
            );
        }
        return Ok(());
    }

    if a.method == ResistanceMethodArg::Dsyhs {
        let dsyhs_in = resistance::DsyhsInput::measure(&hull, &opts).context(
            "cannot measure the hull for DSYHS: a positive draft and waterline are needed",
        )?;
        let speeds = parse_ordinates(&a.speeds, "speeds")?;
        let rows = resistance::dsyhs_curve(&dsyhs_in, &speeds);
        let mut warnings = dsyhs_in.warnings();
        for r in &rows {
            if !(0.15..=0.75).contains(&r.fn_) {
                warnings.push(format!(
                    "Fn {:.3} at {:.2} kn is outside DSYHS valid range (0.15-0.75, clamped)",
                    r.fn_, r.speed_kn
                ));
            }
        }

        if let Some(path) = &a.csv {
            let mut text = String::from(
                "speed_kn,speed_m_s,fn,re,cf,form_factor,rf_n,rapp_n,rw_n,rb_n,rtr_n,ra_n,rt_n,pe_w\n",
            );
            for r in &rows {
                text.push_str(&format!(
                    "{:.3},{:.4},{:.5},{:.0},{:.6},{:.4},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1}\n",
                    r.speed_kn, r.speed_m_s, r.fn_, r.reynolds, r.cf, r.form_factor, r.rf_n,
                    r.rapp_n, r.rw_n, r.rb_n, r.rtr_n, r.ra_n, r.rt_n, r.pe_w
                ));
            }
            seacraft_core::io::write_atomic(path, text)
                .with_context(|| format!("cannot write {}", path.display()))?;
            println!("wrote {} ({} speeds)", path.display(), rows.len());
        }

        if a.json {
            let body = serde_json::json!({
                "method": "dsyhs",
                "input": dsyhs_in,
                "warnings": warnings,
                "speeds": rows,
            });
            println!("{}", serde_json::to_string_pretty(&body)?);
            return Ok(());
        }
        if a.csv.is_some() {
            return Ok(());
        }

        println!(
            "{}  -  Delft Systematic Yacht Hull Series (DSYHS), T {:.3} m",
            s.settings.name, dsyhs_in.draft_canoe
        );
        println!(
            "  LWL {:.3}  BWL {:.3}  Tc {:.3}  Vol {:.3} m3  Aw {:.2} m2  Cp {:.3}  Cm {:.3}  LCB/LWL {:.3}",
            dsyhs_in.length_wl,
            dsyhs_in.beam_wl,
            dsyhs_in.draft_canoe,
            dsyhs_in.volume_m3,
            dsyhs_in.waterplane_area_m2,
            dsyhs_in.cp,
            dsyhs_in.cm,
            dsyhs_in.lcb_fpp_ratio
        );
        for w in &warnings {
            println!("  warning: {w}");
        }
        println!();
        println!(
            "  {:>6}  {:>6}  {:>6}  {:>10}  {:>10}  {:>9}  {:>9}",
            "kn", "Fn", "1+k", "RT [kN]", "PE [kW]", "RF [kN]", "RR [kN]"
        );
        for r in &rows {
            println!(
                "  {:>6.2}  {:>6.3}  {:>6.3}  {:>10.2}  {:>10.2}  {:>9.2}  {:>9.2}",
                r.speed_kn,
                r.fn_,
                r.form_factor,
                r.rt_n / 1000.0,
                r.pe_w / 1000.0,
                r.rf_n / 1000.0,
                r.rw_n / 1000.0
            );
        }
        return Ok(());
    }

    let mut input = resistance::HoltropInput::measure(&hull, &opts)
        .context("cannot measure the hull: it needs a positive draft and a waterline")?;
    input.c_stern = a.cstern;
    input.appendage_area_m2 = a.appendage_area;
    input.appendage_factor = a.appendage_factor;
    input.bulb_area_m2 = a.bulb_area;
    input.bulb_centre_z = a.bulb_z;

    let speeds = parse_ordinates(&a.speeds, "speeds")?;
    let rows = resistance::curve(&input, &speeds);
    let warnings = input.warnings();

    // Propulsion factors, when a propeller is given. They depend on the speed
    // through CV, so each speed gets its own solve.
    let propeller = resistance::Propeller {
        diameter_m: a.propeller_diameter,
        expanded_area_ratio: a.expanded_area,
        open_water_efficiency: a.eta_o,
    };
    let with_propeller = a.propeller_diameter > 0.0;
    let propulsion: Vec<resistance::Propulsion> = if with_propeller {
        rows.iter()
            .map(|r| {
                let mut per_speed = input.clone();
                per_speed.viscous_coefficient = r.viscous_coefficient;
                per_speed.propulsion(&propeller)
            })
            .collect()
    } else {
        Vec::new()
    };

    if let Some(path) = &a.csv {
        let mut text = String::from(
            "speed_kn,speed_m_s,fn,re,cf,form_factor,rf_n,rapp_n,rw_n,rb_n,rtr_n,ra_n,rt_n,pe_w\n",
        );
        if with_propeller {
            text.push_str(",wake_fraction,thrust_deduction,eta_r,eta_d,pd_w,ps_w");
            text.push('\n');
        }
        for (i, r) in rows.iter().enumerate() {
            text.push_str(&format!(
                "{:.3},{:.4},{:.5},{:.0},{:.6},{:.4},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1},{:.1}",
                r.speed_kn, r.speed_m_s, r.fn_, r.reynolds, r.cf, r.form_factor, r.rf_n,
                r.rapp_n, r.rw_n, r.rb_n, r.rtr_n, r.ra_n, r.rt_n, r.pe_w
            ));
            if let Some(p) = propulsion.get(i) {
                let pd = r.pe_w / p.propulsive_coefficient.max(1e-6);
                text.push_str(&format!(
                    ",{:.4},{:.4},{:.4},{:.4},{:.1},{:.1}",
                    p.wake_fraction,
                    p.thrust_deduction,
                    p.relative_rotative,
                    p.propulsive_coefficient,
                    pd,
                    pd / a.shaft_efficiency.max(1e-6)
                ));
            }
            text.push('\n');
        }
        seacraft_core::io::write_atomic(path, text)
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {} ({} speeds)", path.display(), rows.len());
    }
    if a.json {
        let body = serde_json::json!({
            "input": input,
            "warnings": warnings,
            "speeds": rows,
            "propeller": if with_propeller { Some(propeller) } else { None },
            "propulsion": propulsion,
            "shaft_efficiency": a.shaft_efficiency,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    if a.csv.is_some() {
        return Ok(());
    }

    println!(
        "{}  -  Holtrop-Mennen resistance, T {:.3} m",
        s.settings.name, input.draft
    );
    println!(
        "  LWL {:.3}  BWL {:.3}  S {:.2} m2  nabla {:.2} m3  Cb {:.4}  Cp {:.4}  Cm {:.4}",
        input.length_wl,
        input.beam_wl,
        input.wetted_surface_m2,
        input.volume_m3,
        input.cb,
        input.cp,
        input.cm
    );
    println!(
        "  LCB {:+.2} % L  iE {:.1} deg  AT {:.3} m2  ABT {:.3} m2  c_stern {:+.1}",
        input.lcb_pct,
        input.half_entrance_deg,
        input.transom_area_m2,
        input.bulb_area_m2,
        input.c_stern
    );
    for w in &warnings {
        println!("  warning: {w} (outside the method's fitted range)");
    }
    println!();
    if with_propeller {
        println!(
            "  {:>6}  {:>6}  {:>10}  {:>10}  {:>7}  {:>7}  {:>7}  {:>10}  {:>10}",
            "kn", "Fn", "RT [kN]", "PE [kW]", "w", "t", "etaD", "PD [kW]", "PS [kW]"
        );
        for (i, r) in rows.iter().enumerate() {
            let p = &propulsion[i];
            let pd = r.pe_w / p.propulsive_coefficient.max(1e-6);
            println!(
                "  {:>6.2}  {:>6.3}  {:>10.2}  {:>10.2}  {:>7.4}  {:>7.4}  {:>7.4}  {:>10.1}  {:>10.1}",
                r.speed_kn,
                r.fn_,
                r.rt_n / 1000.0,
                r.pe_w / 1000.0,
                p.wake_fraction,
                p.thrust_deduction,
                p.propulsive_coefficient,
                pd / 1000.0,
                pd / a.shaft_efficiency.max(1e-6) / 1000.0
            );
        }
        println!(
            "  propeller D {:.2} m, AE/A0 {:.2}, etaO {:.2}, etaR {:.4}, etaH {:.4}, shaft {:.2}",
            a.propeller_diameter,
            a.expanded_area,
            a.eta_o,
            propulsion[0].relative_rotative,
            propulsion[0].hull_efficiency,
            a.shaft_efficiency
        );
        if !propulsion[0].plausible {
            println!(
                "  warning: w {:.3}, t {:.3} and LCB {:+.2} %L are outside the regressions' \
                 fitted range, so the shaft power is indicative only",
                propulsion[0].wake_fraction, propulsion[0].thrust_deduction, input.lcb_pct
            );
        }
    } else {
        println!(
            "  {:>6}  {:>6}  {:>6}  {:>10}  {:>10}  {:>9}  {:>9}",
            "kn", "Fn", "1+k1", "RT [kN]", "PE [kW]", "RF [kN]", "RW [kN]"
        );
        for r in &rows {
            println!(
                "  {:>6.2}  {:>6.3}  {:>6.3}  {:>10.2}  {:>10.2}  {:>9.2}  {:>9.2}",
                r.speed_kn,
                r.fn_,
                r.form_factor,
                r.rt_n / 1000.0,
                r.pe_w / 1000.0,
                r.rf_n / 1000.0,
                r.rw_n / 1000.0
            );
        }
    }
    if rows.iter().any(|r| r.fn_ > 0.40) {
        println!(
            "  Fn above 0.40: the wave term switches to Holtrop's c17/m3 high-speed fit at 0.55, \
             interpolated between (this run: {} low, {} interpolated, {} high).",
            rows.iter().filter(|r| r.fn_ <= 0.40).count(),
            rows.iter().filter(|r| r.fn_ > 0.40 && r.fn_ < 0.55).count(),
            rows.iter().filter(|r| r.fn_ >= 0.55).count()
        );
    }
    if with_propeller {
        println!(
            "\n  PE is the hull's effective power; PD = PE/etaD and PS = PD/etaS. etaO is an input, \
             not a prediction."
        );
    } else {
        println!("\n  effective power only: give --propeller-diameter for the power chain.");
    }
    Ok(())
}

fn parse_pair(text: &str, key: &str) -> Result<(usize, usize)> {
    let parts: Vec<&str> = text.split(',').map(|t| t.trim()).collect();
    if parts.len() != 2 {
        bail!("{key} takes two point indices, A,B");
    }
    let a: usize = parts[0]
        .parse()
        .with_context(|| format!("{key}: `{}` is not an index", parts[0]))?;
    let b: usize = parts[1]
        .parse()
        .with_context(|| format!("{key}: `{}` is not an index", parts[1]))?;
    Ok((a, b))
}

fn parse_triple_indices(text: &str, key: &str) -> Result<(usize, usize, usize)> {
    let parts: Vec<&str> = text.split(',').map(|t| t.trim()).collect();
    if parts.len() != 3 {
        bail!("{key} takes FACE,A,B");
    }
    let mut out = [0usize; 3];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p
            .parse()
            .with_context(|| format!("{key}: `{p}` is not an index"))?;
    }
    Ok((out[0], out[1], out[2]))
}

fn cmd_edit(a: EditArgs) -> Result<()> {
    let mut s = load(&a.file)?;
    let faces_before = s.faces.len();
    let points_before = s.points.len();
    let creases_before = s.edges.iter().filter(|e| e.crease).count();
    let mut splits = Vec::new();

    // Documented order: splits, then inserted edges, then crease toggles. A
    // split keeps the face's index, so an `--insert-edge` naming that face
    // afterwards refers to the first triangle of the fan.
    for &face in &a.split_face {
        let r =
            topology::split_face(&mut s, face).map_err(|e| anyhow::anyhow!("--split-face: {e}"))?;
        splits.push(r);
    }
    for text in &a.insert_edge {
        let (face, x, y) = parse_triple_indices(text, "--insert-edge")?;
        topology::insert_edge(&mut s, face, x, y)
            .map_err(|e| anyhow::anyhow!("--insert-edge {text}: {e}"))?;
    }
    for text in &a.crease {
        let (x, y) = parse_pair(text, "--crease")?;
        topology::set_crease(&mut s, x, y, true)
            .map_err(|e| anyhow::anyhow!("--crease {text}: {e}"))?;
    }
    for text in &a.clear_crease {
        let (x, y) = parse_pair(text, "--clear-crease")?;
        topology::set_crease(&mut s, x, y, false)
            .map_err(|e| anyhow::anyhow!("--clear-crease {text}: {e}"))?;
    }

    if a.split_face.is_empty()
        && a.insert_edge.is_empty()
        && a.crease.is_empty()
        && a.clear_crease.is_empty()
    {
        bail!("nothing to do: give --split-face, --insert-edge, --crease or --clear-crease");
    }
    s.validate()
        .map_err(|e| anyhow::anyhow!("the edited model does not validate: {e}"))?;
    seacraft_core::io::write_atomic(&a.output, fef::write(&s))
        .with_context(|| format!("cannot write {}", a.output.display()))?;

    let creases_after = s.edges.iter().filter(|e| e.crease).count();
    if a.json {
        let body = serde_json::json!({
            "output": a.output.display().to_string(),
            "splits": splits,
            "faces_before": faces_before,
            "faces_after": s.faces.len(),
            "points_before": points_before,
            "points_after": s.points.len(),
            "creases_before": creases_before,
            "creases_after": creases_after,
            "valid": true,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    for r in &splits {
        println!(
            "  split face {} into {} triangles around new point {}",
            r.face, r.pieces, r.point
        );
    }
    println!(
        "  faces {} -> {}, points {} -> {}, creases {} -> {}",
        faces_before,
        s.faces.len(),
        points_before,
        s.points.len(),
        creases_before,
        creases_after
    );
    println!("  validates: closed and manifold");
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_planing(a: PlaningArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
    };
    let mut opts = HydroOptions::from_surface(&s, subdiv);
    if let Some(d) = a.draft {
        opts.draft = d;
    }
    let setup = planing::setup(&s, subdiv, &opts, a.beam, a.lcg_x, a.deadrise)
        .map_err(|e| anyhow::anyhow!(e))?;
    let sail = setup.hull;
    let measured = setup.deadrise_measured;
    let lcg_x = setup.lcg_x;
    let weight_n = setup.weight_n;
    let beam = sail.beam_m;
    let deadrise = sail.deadrise_deg;
    let lcg_from_transom = sail.lcg_from_transom_m;
    let speeds = parse_ordinates(&a.speeds, "speeds")?;
    let curve = planing::planing_curve(&sail, &speeds);
    if curve.is_empty() {
        bail!(
            "no planing condition found between {:.0} and {:.0} kn: the hull is too heavy for its \
             beam, or the LCG is too far aft for the pressure centre to reach it",
            speeds.first().copied().unwrap_or(0.0),
            speeds.last().copied().unwrap_or(0.0)
        );
    }

    if let Some(path) = &a.csv {
        let mut text = String::from(
            "speed_kn,speed_m_s,cv,c_delta,cl_beta,trim_deg,lambda,wetted_length_m,lcp_over_b,cf,resistance_n,pe_w\n",
        );
        for p in &curve {
            text.push_str(&format!(
                "{:.2},{:.3},{:.4},{:.5},{:.5},{:.3},{:.4},{:.3},{:.4},{:.6},{:.1},{:.1}\n",
                p.speed_kn,
                p.speed_m_s,
                p.cv,
                p.c_delta,
                p.cl_beta,
                p.trim_deg,
                p.lambda,
                p.wetted_length_m,
                p.lcp_over_b,
                p.cf,
                p.resistance_n,
                p.pe_w
            ));
        }
        seacraft_core::io::write_atomic(path, text)
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {} ({} speeds)", path.display(), curve.len());
    }
    if a.json {
        let body = serde_json::json!({
            "hull": sail,
            "lcg_x": lcg_x,
            "deadrise_measured": measured,
            "speeds": curve,
        });
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    if a.csv.is_some() {
        return Ok(());
    }
    println!(
        "{}  -  Savitsky planing, T {:.3} m",
        s.settings.name, opts.draft
    );
    println!(
        "  weight {:.2} kN, beam {:.3} m, LCG {:.3} m from the transom ({:.3} b), deadrise {:.1} deg{}",
        weight_n / 1000.0,
        beam,
        lcg_from_transom,
        lcg_from_transom / beam,
        deadrise,
        if a.deadrise.is_none() && measured.is_some() { " (measured)" } else { "" }
    );
    println!();
    println!(
        "  {:>6}  {:>6}  {:>7}  {:>7}  {:>10}  {:>7}  {:>10}  {:>10}",
        "kn", "CV", "trim", "lambda", "wetted [m]", "LCP/b", "RT [kN]", "PE [kW]"
    );
    for p in &curve {
        println!(
            "  {:>6.1}  {:>6.3}  {:>6.2}d  {:>7.3}  {:>10.3}  {:>7.3}  {:>10.2}  {:>10.2}",
            p.speed_kn,
            p.cv,
            p.trim_deg,
            p.lambda,
            p.wetted_length_m,
            p.lcp_over_b,
            p.resistance_n / 1000.0,
            p.pe_w / 1000.0
        );
    }
    let missed: Vec<f64> = speeds
        .iter()
        .filter(|s| !curve.iter().any(|p| (p.speed_kn - **s).abs() < 1.0))
        .copied()
        .collect();
    if !missed.is_empty() {
        println!(
            "  no planing equilibrium at {:?} kn (outside the hull's planing range)",
            missed
        );
    }
    println!(
        "\n  effective power only; add --propeller-diameter to `resistance` for the shaft power."
    );
    Ok(())
}

fn cmd_unfold(a: UnfoldArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.level,
        mode: if a.catmull_clark {
            SubdivisionMode::CatmullClark
        } else {
            SubdivisionMode::Loop
        },
    };
    let axis = develop::BandAxis::parse(&a.axis)
        .with_context(|| format!("unknown --axis `{}` (use x, y or z)", a.axis))?;
    let hull = seacraft_core::hydro::Hull::new(&s, subdiv);
    let lo = a.from.unwrap_or(match axis {
        develop::BandAxis::X => hull.model_min.x,
        develop::BandAxis::Y => hull.model_min.y,
        develop::BandAxis::Z => hull.keel,
    });
    let hi = a.to.unwrap_or(match axis {
        develop::BandAxis::X => hull.model_max.x,
        develop::BandAxis::Y => hull.model_max.y,
        develop::BandAxis::Z => hull.model_max.z,
    });
    let x_lo = a.x_from.unwrap_or(hull.model_min.x);
    let x_hi = a.x_to.unwrap_or(hull.model_max.x);
    let selection = develop::PatchSelection {
        slabs: vec![(axis, lo, hi), (develop::BandAxis::X, x_lo, x_hi)],
        starboard_only: !a.both_sides,
    };
    let faces = develop::select(&hull.verts, &hull.tris, &selection);
    if faces.is_empty() {
        bail!(
            "no faces in the band {lo:.3}..{hi:.3} on {axis:?} between x {x_lo:.3} and {x_hi:.3}; \
             widen --from/--to or --x-from/--x-to"
        );
    }
    let unfolded = develop::unfold(&hull.verts, &faces);

    if let Some(path) = &a.output {
        let title = format!(
            "{} - plate between {:.2} and {:.2} m on {:?}",
            s.settings.name, lo, hi, axis
        );
        seacraft_core::io::write_atomic(path, develop::svg(&unfolded, &title))
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&unfolded)?);
        return Ok(());
    }
    println!(
        "{}  -  plate expansion across a {:?} band",
        s.settings.name, axis
    );
    println!(
        "  band {lo:.3} .. {hi:.3} m on {axis:?}, stations {x_lo:.3} .. {x_hi:.3}, {} of {} faces",
        faces.len(),
        hull.tris.len()
    );
    println!(
        "  flat pattern {:.3} x {:.3} m, area {:.4} m2 (3D {:.4} m2)",
        unfolded.width_m, unfolded.height_m, unfolded.area_2d_m2, unfolded.area_3d_m2
    );
    println!(
        "  seam gap max {:.2} mm, rms {:.2} mm, {:.3} % of the diagonal",
        unfolded.max_gap_m * 1000.0,
        unfolded.rms_gap_m * 1000.0,
        unfolded.gap_ratio * 100.0
    );
    println!(
        "  mean |K| {:.4} 1/m2  ->  {}",
        unfolded.mean_abs_gaussian,
        if unfolded.developable {
            "developable (within 0.5 % of the diagonal)"
        } else {
            "not developable: the plate needs a joint, a press or a different cut"
        }
    );
    Ok(())
}

/// How a page line names the paper: `A4`, `A3`, `Letter`, or its size.
fn paper_name(paper: seacraft_core::sheet::Paper) -> String {
    use seacraft_core::sheet::Paper;
    match paper {
        p if p == Paper::A4 => "A4".into(),
        p if p == Paper::A3 => "A3".into(),
        p if p == Paper::LETTER => "Letter".into(),
        p => format!("{} x {} mm", p.width_mm, p.height_mm),
    }
}

fn cmd_templates(a: TemplatesArgs) -> Result<()> {
    use seacraft_core::sheet::{PageSetup, Paper};
    use seacraft_core::templates::{self, Format, MouldFace, SetOptions, StationChoice, Transom};
    use seacraft_core::units;

    // Everything the user typed is checked before the hull is refined and cut.
    let extension = a
        .output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(extension.as_str(), "svg" | "dxf") {
        let given: Vec<&str> = [
            (a.paper.is_some(), "--paper"),
            (a.single_page, "--single-page"),
            (a.margin_mm.is_some(), "--margin-mm"),
            (a.overlap_mm.is_some(), "--overlap-mm"),
        ]
        .into_iter()
        .filter_map(|(given, name)| given.then_some(name))
        .collect();
        if !given.is_empty() {
            bail!(
                "{} apply to PDF only: an SVG or a DXF is one drawing at the scale asked for",
                given.join(", ")
            );
        }
    }
    let format = Format::from_path(&a.output, a.single_page).map_err(anyhow::Error::msg)?;
    let max_mm = templates::MAX_SKIN_M * 1000.0;
    if !a.skin_mm.is_finite() || !(0.0..=max_mm).contains(&a.skin_mm) {
        bail!("--skin-mm must be from 0 to {max_mm} mm, not {}", a.skin_mm);
    }
    let scale = units::parse_scale(&a.scale).map_err(|e| anyhow::anyhow!("--scale: {e}"))?;
    let paper = match &a.paper {
        Some(text) => Paper::parse(text).map_err(|e| anyhow::anyhow!("--paper: {e}"))?,
        None => Paper::A4,
    };
    let defaults = PageSetup::default();
    let setup = PageSetup {
        paper,
        margin_mm: a.margin_mm.unwrap_or(defaults.margin_mm),
        overlap_mm: a.overlap_mm.unwrap_or(defaults.overlap_mm),
        scale,
        imperial: a.imperial,
        title: String::new(),
    };
    setup.check().map_err(anyhow::Error::msg)?;
    let stations = match (&a.stations, &a.spacing, &a.at) {
        (_, Some(spacing), _) => StationChoice::Spacing(*spacing),
        (_, _, Some(at)) => StationChoice::At(parse_list(at).context("--at")?),
        (count, _, _) => StationChoice::Count(count.unwrap_or(11)),
    };
    let transom = match &a.transom {
        None => None,
        Some(text) => match parse_list(text).context("--transom")?.as_slice() {
            [x, rake_deg] => Some(Transom {
                x: *x,
                rake_deg: *rake_deg,
            }),
            _ => bail!("--transom takes X,RAKE_DEG: where the plane meets the baseline and its rake aft, not `{text}`"),
        },
    };
    let waterlines_m = match &a.waterlines {
        Some(text) => Some(parse_ordinates(text, "--waterlines")?),
        None => None,
    };

    let s = load(&a.file)?;
    let opts = SetOptions {
        subdivision: SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
        stations,
        skin_m: a.skin_mm / 1000.0,
        half: a.half,
        strongback_z: a.strongback,
        waterlines_m,
        profile: a.profile,
        transom,
        imperial: a.imperial,
    };
    let set = templates::template_set(&s, &opts).map_err(anyhow::Error::msg)?;
    let written = templates::write(&set, format, &setup).map_err(anyhow::Error::msg)?;
    seacraft_core::io::write_atomic(&a.output, &written.bytes)
        .with_context(|| format!("cannot write {}", a.output.display()))?;

    let missing: Vec<String> = written.missing.iter().map(char::to_string).collect();
    if a.json {
        let v = serde_json::json!({
            "path": a.output.display().to_string(),
            "format": format.name(),
            "pages": written.pages,
            "skin_mm": a.skin_mm,
            "scale": units::scale_label(scale),
            "sheets": set.rows,
            "warnings": set.warnings,
            "missing_characters": missing,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    println!(
        "{:<12} {:>8} {:>18} {:>8} {:>11}",
        "sheet", "x [m]", "half-breadth [mm]", "face", "bevel [deg]"
    );
    for row in &set.rows {
        let x = row.x.map_or_else(|| "-".to_owned(), |x| format!("{x:.3}"));
        let face = match row.face {
            Some(MouldFace::Forward) => "forward",
            Some(MouldFace::Aft) => "aft",
            Some(MouldFace::Either) => "either",
            None => "-",
        };
        println!(
            "{:<12} {:>8} {:>18.0} {:>8} {:>11.1}",
            row.sheet,
            x,
            row.half_breadth_m * 1000.0,
            face,
            row.max_bevel_deg
        );
    }
    let scale_label = units::scale_label(scale);
    match (format, written.pages) {
        (Format::PdfTiled, Some(pages)) => {
            println!("{pages} pages, {}, scale {scale_label}", paper_name(paper))
        }
        (_, Some(pages)) => println!("{pages} pages, one a pattern, scale {scale_label}"),
        (_, None) => println!("one drawing, scale {scale_label}"),
    }
    if a.skin_mm > 0.0 {
        println!("skin {} mm deducted", a.skin_mm);
    } else {
        println!("no skin deducted: the cut line is the hull surface");
    }
    for warning in &set.warnings {
        eprintln!("warning: {warning}");
    }
    if !missing.is_empty() {
        println!(
            "the font has no letter for {}: each is printed as ?",
            missing.join(" ")
        );
    }
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_panels(a: PanelsArgs) -> Result<()> {
    use seacraft_core::panels::{self, Limits, PanelOptions, Stock};
    use seacraft_core::sheet::{PageSetup, Paper};
    use seacraft_core::templates::{Format, StationChoice};
    use seacraft_core::units;

    // Everything the user typed is checked before the hull is refined and flattened.
    let extension = a
        .output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    if matches!(extension.as_str(), "svg" | "dxf") {
        let given: Vec<&str> = [
            (a.paper.is_some(), "--paper"),
            (a.single_page, "--single-page"),
            (a.margin_mm.is_some(), "--margin-mm"),
            (a.overlap_mm.is_some(), "--overlap-mm"),
        ]
        .into_iter()
        .filter_map(|(given, name)| given.then_some(name))
        .collect();
        if !given.is_empty() {
            bail!(
                "{} apply to PDF only: an SVG or a DXF is one drawing at the scale asked for",
                given.join(", ")
            );
        }
    }
    let format = Format::from_path(&a.output, a.single_page).map_err(anyhow::Error::msg)?;
    if let Some(p) = &a.offsets_pdf {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        if ext.as_deref() != Some("pdf") {
            bail!("--offsets-pdf writes a .pdf, not `{}`", p.display());
        }
    }
    let positive = |v: f64| v.is_finite() && v > 0.0;
    if !positive(a.max_seam_drift_mm) {
        bail!(
            "--max-seam-drift-mm must be more than zero, not {}",
            a.max_seam_drift_mm
        );
    }
    if !positive(a.max_stretch_pct) {
        bail!(
            "--max-stretch-pct: the stretch limit must be more than zero, not {}",
            a.max_stretch_pct
        );
    }
    if !positive(a.offset_spacing_mm) {
        bail!(
            "--offset-spacing-mm must be more than zero, not {}",
            a.offset_spacing_mm
        );
    }
    let stock = match a.stock_mm.split_once(['x', 'X']) {
        Some((l, w)) => match (l.trim().parse::<f64>(), w.trim().parse::<f64>()) {
            (Ok(l), Ok(w)) if positive(l) && positive(w) => (l, w),
            _ => bail!(
                "--stock-mm takes LENGTHxWIDTH in millimetres, such as 2440x1220, not `{}`",
                a.stock_mm
            ),
        },
        None => bail!(
            "--stock-mm takes LENGTHxWIDTH in millimetres, such as 2440x1220, not `{}`",
            a.stock_mm
        ),
    };
    if !a.scarf_mm.is_finite() || a.scarf_mm < 0.0 || a.scarf_mm >= stock.0 {
        bail!(
            "--scarf-mm must be from 0 to less than the sheet's length, not {}",
            a.scarf_mm
        );
    }
    let scale = units::parse_scale(&a.scale).map_err(|e| anyhow::anyhow!("--scale: {e}"))?;
    let paper = match &a.paper {
        Some(text) => Paper::parse(text).map_err(|e| anyhow::anyhow!("--paper: {e}"))?,
        None => Paper::A4,
    };
    let defaults = PageSetup::default();
    let setup = PageSetup {
        paper,
        margin_mm: a.margin_mm.unwrap_or(defaults.margin_mm),
        overlap_mm: a.overlap_mm.unwrap_or(defaults.overlap_mm),
        scale,
        imperial: a.imperial,
        title: String::new(),
    };
    setup.check().map_err(anyhow::Error::msg)?;
    let stations = match (&a.stations, &a.spacing, &a.at) {
        (_, Some(spacing), _) => StationChoice::Spacing(*spacing),
        (_, _, Some(at)) => StationChoice::At(parse_list(at).context("--at")?),
        (count, _, _) => StationChoice::Count(count.unwrap_or(11)),
    };

    let s = load(&a.file)?;
    let opts = PanelOptions {
        subdivision: SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
        layers: (!a.layers.is_empty()).then(|| a.layers.clone()),
        stations,
        limits: Limits {
            seam_drift_m: a.max_seam_drift_mm / 1000.0,
            rms_stretch: a.max_stretch_pct / 100.0,
        },
        offset_spacing_m: a.offset_spacing_mm / 1000.0,
        stock: Stock {
            length_m: stock.0 / 1000.0,
            width_m: stock.1 / 1000.0,
            scarf_m: a.scarf_mm / 1000.0,
        },
        imperial: a.imperial,
    };
    let set = panels::panel_set(&s, &opts).map_err(anyhow::Error::msg)?;

    // A panel that does not lie flat stops the export unless the user asked for it anyway.
    let failing: Vec<String> = set
        .sheets
        .iter()
        .zip(&set.rows)
        .filter(|(_, r)| !r.lies_flat)
        .map(|(sheet, r)| format!("{}: {}", r.panel, sheet.notes[0]))
        .chain(
            set.refused
                .iter()
                .map(|r| format!("{} cannot be laid flat: {}", r.panel, r.reason)),
        )
        .collect();
    if !failing.is_empty() && !a.allow_undevelopable {
        for f in &failing {
            eprintln!("{f}");
        }
        bail!(
            "{} of {} panels do not lie flat within the limits; nothing written. Pass \
             --allow-undevelopable to write the patterns anyway",
            failing.len(),
            set.rows.len() + set.refused.len()
        );
    }

    let written = panels::write(&set, format, &setup).map_err(anyhow::Error::msg)?;
    seacraft_core::io::write_atomic(&a.output, &written.bytes)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    if let Some(path) = &a.offsets_csv {
        seacraft_core::io::write_atomic(path, panels::offsets_csv(&set.offsets).as_bytes())
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    if let Some(path) = &a.offsets_pdf {
        let doc = panels::offsets_pdf(&set.offsets, &set.title, a.imperial)
            .map_err(anyhow::Error::msg)?;
        seacraft_core::io::write_atomic(path, &doc.bytes)
            .with_context(|| format!("cannot write {}", path.display()))?;
    }

    let missing: Vec<String> = written.missing.iter().map(char::to_string).collect();
    if a.json {
        let v = serde_json::json!({
            "path": a.output.display().to_string(),
            "format": format.name(),
            "pages": written.pages,
            "scale": units::scale_label(scale),
            "panels": set.rows,
            "mismatches": set.mismatches,
            "refused": set.refused,
            "total_sheets": set.total_sheets,
            "missing_characters": missing,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    println!(
        "{:<16} {:>6} {:>10} {:>7} {:>9} {:>7}",
        "panel", "copies", "lies flat", "rms %", "drift mm", "sheets"
    );
    for r in &set.rows {
        println!(
            "{:<16} {:>6} {:>10} {:>7.3} {:>9.1} {:>7}",
            r.panel,
            r.copies,
            if r.lies_flat { "yes" } else { "no" },
            r.rms_stretch_pct,
            r.max_seam_drift_mm,
            r.sheets_each
                .map_or_else(|| "-".to_owned(), |n| n.to_string())
        );
    }
    for r in &set.refused {
        println!("{:<16} refused: {}", r.panel, r.reason);
    }
    println!(
        "{} plywood sheets, end to end, no nesting: an upper bound",
        set.total_sheets
    );
    for row in &set.stock {
        if let Some(note) = &row.note {
            println!("{}: {note}", row.panel);
        }
    }
    if !failing.is_empty() {
        println!(
            "{} panels do not lie flat within the limits: their patterns are headed so",
            failing.len()
        );
    }
    let scale_label = units::scale_label(scale);
    match (format, written.pages) {
        (Format::PdfTiled, Some(pages)) => {
            println!("{pages} pages, {}, scale {scale_label}", paper_name(paper))
        }
        (_, Some(pages)) => println!("{pages} pages, one a panel, scale {scale_label}"),
        (_, None) => println!("one drawing, scale {scale_label}"),
    }
    if !missing.is_empty() {
        println!(
            "the font has no letter for {}: each is printed as ?",
            missing.join(" ")
        );
    }
    println!("wrote {}", a.output.display());
    for path in [&a.offsets_csv, &a.offsets_pdf].into_iter().flatten() {
        println!("wrote {}", path.display());
    }
    Ok(())
}

fn cmd_linesplan(a: LinesplanArgs) -> Result<()> {
    let s = load(&a.file)?;
    let opts = linesplan::LinesplanOptions {
        stations: a.stations,
        buttocks: a.buttocks,
        waterlines: a.waterlines,
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
        combs: a.combs,
        comb_scale: a.comb_scale,
    };
    // The extension picks the format: `.dxf` gives the three views on named
    // layers plus 3D copies of every curve, `.svg` (or none) the standalone
    // SVG; anything else is refused rather than written as SVG.
    let extension = a
        .output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let text = match extension.as_str() {
        "dxf" => linesplan::dxf(&s, opts),
        "igs" | "iges" => linesplan::iges(&s, opts),
        "svg" | "" => linesplan::svg(&s, opts),
        other => {
            bail!("unsupported linesplan extension `.{other}` (use .svg, .dxf, .igs or .iges)")
        }
    };
    seacraft_core::io::write_atomic(&a.output, text)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_report(a: ReportArgs) -> Result<()> {
    let s = load(&a.file)?;
    let project = if a.no_project {
        seacraft_core::project::Project::default()
    } else {
        seacraft_core::project::Project::load_for_model(&a.file)
            .map_err(|e| anyhow::anyhow!("{e}"))?
            .unwrap_or_default()
    };
    let mut opts = seacraft_core::report::ReportOptions::from_surface(&s);
    opts = project.report.apply(opts);
    opts.level = a.subdiv.level;
    opts.mode = a.subdiv.mode.into();
    if let Some(d) = a.draft {
        if d <= 0.0 {
            bail!("draft must be positive");
        }
        opts.draft = d;
    }
    if let Some(kg) = a.kg {
        if kg < 0.0 {
            bail!("kg must be non-negative");
        }
        opts.kg = kg;
    }
    if let Some(lcg) = a.lcg {
        opts.lcg_x = Some(lcg);
    }
    if let Some(speeds) = &a.speeds {
        opts.speeds_kn = parse_list(speeds)?;
    }
    if a.no_resistance {
        opts.include_resistance = false;
    }
    if a.planing {
        opts.include_planing = true;
    }
    let html = seacraft_core::report::design_report_with_project(&s, &opts, &project)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    seacraft_core::io::write_atomic(&a.output, html)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_render(a: RenderArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mode = if a.catmull_clark {
        SubdivisionMode::CatmullClark
    } else {
        SubdivisionMode::Loop
    };
    let mesh = export::render_mesh(&s, a.level, mode);

    let (min, max) = s.extents();
    let centre = (min + max) * 0.5;
    let size = (max - min).length().max(0.5);
    let view = match a.view {
        RenderViewArg::Perspective => raster::View::perspective(centre, size, 2.2, 0.30, 45.0),
        RenderViewArg::Profile => raster::View::ortho(Vec3::new(0.0, 1.0, 0.0), centre, size),
        RenderViewArg::Plan => raster::View::ortho(Vec3::new(0.0, 0.0, -1.0), centre, size),
        RenderViewArg::Body => raster::View::ortho(Vec3::new(1.0, 0.0, 0.0), centre, size),
    };
    let mode = match a.mode {
        RenderModeArg::Shaded => AnalysisMode::Shaded,
        RenderModeArg::Zebra => AnalysisMode::Zebra { stripes: a.stripes },
        RenderModeArg::Curvature => AnalysisMode::Curvature { radius: a.radius },
        RenderModeArg::Draft => AnalysisMode::Draft { angle_deg: a.draft },
    };
    let (width, height) = parse_size(&a.size)?;
    let opts = raster::RenderOptions {
        width,
        height,
        mode,
        supersample: a.samples.max(1),
        wireframe: a.wireframe,
        ..Default::default()
    };
    let canvas = raster::render(&mesh.verts, &mesh.faces, &view, &opts);
    let bytes = raster::png(canvas.w, canvas.h, &canvas.rgba)
        .ok_or_else(|| anyhow::anyhow!("cannot encode a {}x{} PNG", width, height))?;
    seacraft_core::io::write_atomic(&a.output, bytes)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!(
        "wrote {} ({}x{}, {} mode, {} faces)",
        a.output.display(),
        width,
        height,
        mode.label(),
        mesh.faces.len()
    );
    Ok(())
}

fn parse_size(text: &str) -> Result<(usize, usize)> {
    let (w, h) = text
        .split_once('x')
        .or_else(|| text.split_once('X'))
        .with_context(|| format!("image size must be WIDTHxHEIGHT, got '{text}'"))?;
    Ok((
        w.trim()
            .parse()
            .with_context(|| format!("bad width '{w}'"))?,
        h.trim()
            .parse()
            .with_context(|| format!("bad height '{h}'"))?,
    ))
}

fn parse_list(text: &str) -> Result<Vec<f64>> {
    text.split(',')
        .map(|t| {
            t.trim()
                .parse::<f64>()
                .with_context(|| format!("invalid number '{}'", t))
        })
        .collect()
}

fn cmd_stability(a: StabilityArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mode: SubdivisionMode = a.subdiv.mode.into();
    let hull = hydro::Hull::new(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode,
        },
    );
    let mut base = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode,
        },
    );
    base.trim = a.trim;
    let heels = parse_list(&a.heels)?;
    let displacements = match &a.displacements {
        Some(text) => parse_list(text)?,
        None => {
            let design_volume = hull.displaced_volume(base.draft, a.trim, 0.0);
            seacraft_core::stability::default_displacements(
                design_volume * base.water_density / 1000.0,
            )
        }
    };
    let free = a.free_trim || a.lcg.is_some();
    let lcg = match (free, a.lcg) {
        (true, Some(x)) => Some(x),
        // No mass distribution in the model: the design LCB is the natural LCG.
        (true, None) => Some(upright_lcb(&hull, base.draft)),
        (false, _) => None,
    };
    let cond = StabilityCondition {
        kg: a.kg,
        trim: a.trim,
        lcg_x: lcg,
    };
    let curves = cross_curves(&hull, &heels, &displacements, &cond, &base);

    if let Some(path) = a.csv {
        let mut text = String::from("heel_deg,displacement_t,draft_m,trim_m,kn_m,gz_m\n");
        for c in &curves {
            for p in &c.points {
                let d = p
                    .draft
                    .map(|v| format!("{:.4}", v))
                    .unwrap_or_else(|| "—".into());
                let t = p
                    .trim_m
                    .map(|v| format!("{:.4}", v))
                    .unwrap_or_else(|| "—".into());
                let kn =
                    p.kn.map(|v| format!("{:.4}", v))
                        .unwrap_or_else(|| "—".into());
                let gz =
                    p.gz.map(|v| format!("{:.4}", v))
                        .unwrap_or_else(|| "—".into());
                text.push_str(&format!(
                    "{:.2},{:.3},{},{},{},{}\n",
                    c.heel_deg, p.displacement_t, d, t, kn, gz
                ));
            }
        }
        seacraft_core::io::write_atomic(&path, text)
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
        return Ok(());
    }

    println!(
        "{}  -  cross curves, KG {:.3} m above keel",
        s.settings.name, a.kg
    );
    match lcg {
        Some(x) => println!("  free to trim about LCG x = {:.3} m", x),
        None => println!("  trim held at {:+.3} m", a.trim),
    }
    print!("  heel |");
    for d in &displacements {
        print!("  {:>8.1} t", d);
    }
    println!();
    println!("  -----+{}", "-".repeat(11 * displacements.len()));
    for c in &curves {
        print!("  {:>4.0} |", c.heel_deg);
        for p in &c.points {
            match p.gz {
                Some(gz) => print!("    {:>7.3}", gz),
                None => print!("    {:>7}", "—"),
            }
        }
        println!();
    }
    println!("\n  GZ in metres. Solved draft / trim per displacement:");
    for c in &curves {
        let solved: Vec<String> = c
            .points
            .iter()
            .map(|p| match (p.draft, p.trim_m) {
                (Some(d), Some(t)) => format!("{:.3} m / {:+.3} m", d, t),
                _ => "—".to_string(),
            })
            .collect();
        println!("    {:>4.0} deg: {}", c.heel_deg, solved.join(", "));
    }
    let criteria: Vec<_> = (0..displacements.len())
        .filter_map(|i| evaluate_imo_criteria(&curves, i))
        .collect();
    if !criteria.is_empty() {
        println!("\n  IMO A.749 / 2008 IS Code Part A intact stability criteria:");
        println!("  {}", seacraft_core::NOT_VALIDATED_NOTE);
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  status",
            "disp (t)", "A(0-30)", "A(0-40)", "A(30-40)", "GZ(>=30)", "phi_max", "GM0"
        );
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  ------",
            "min req.", ">=0.055", ">=0.090", ">=0.030", ">=0.200 m", ">=25 deg", ">=0.15"
        );
        for c in &criteria {
            println!(
                "    {:>8.2}  {:>9.4}  {:>9.4}  {:>10.4}  {:>10.3}  {:>9.1}  {:>7.3}  {}",
                c.displacement_t,
                c.area_0_30_m_rad,
                c.area_0_40_m_rad,
                c.area_30_40_m_rad,
                c.max_gz_30_plus_m,
                c.angle_of_max_gz_deg,
                c.gm0_m,
                if c.passed { "PASS" } else { "FAIL" }
            );
        }
    }
    if a.weather {
        let bilge = seacraft_core::weather::BilgeKind::parse(&a.bilge)
            .map_err(|e| anyhow::anyhow!("--bilge: {e}"))?;
        let mut windage_extra = Vec::with_capacity(a.windage.len());
        for spec in &a.windage {
            let item = seacraft_core::weather::WindageItem::parse_spec(spec)
                .map_err(|e| anyhow::anyhow!("--windage: {e}"))?;
            windage_extra.push(item);
        }
        if a.windage.is_empty() {
            if let Some(p) =
                project::Project::load_for_model(&a.file).map_err(|e| anyhow::anyhow!("{e}"))?
            {
                if !p.windage.is_empty() {
                    println!(
                        "  windage items from {}",
                        project::sidecar_path(&a.file).display()
                    );
                    windage_extra = p.windage;
                }
            }
        }
        if let Some(df) = a.downflooding {
            if !df.is_finite() || df <= 0.0 {
                bail!("--downflooding must be positive and finite");
            }
        }
        let w_input = seacraft_core::weather::WeatherInput {
            windage_extra,
            bilge,
            downflooding_deg: a.downflooding,
            ..Default::default()
        };
        let w = seacraft_core::weather::evaluate_hull_weather(&hull, &base, &cond, &w_input);
        print_weather_summary(&w);
    }
    let _ = solve_draft; // keep the public helper referenced
    let _ = Vec3::ZERO;
    Ok(())
}

enum CaseSource {
    /// A bare loading condition or damage case, still to be parsed.
    Bare(String),
    Project(Box<project::Project>),
}

/// Read `case`, or the model's sidecar when `case` is `None`. A file whose
/// `format` is `seacraft-project` is a project; anything else is a bare case.
fn read_case_or_project(model: &Path, case: Option<&Path>) -> Result<(CaseSource, PathBuf)> {
    let path = match case {
        Some(p) => p.to_path_buf(),
        None => {
            let p = project::sidecar_path(model);
            if !p.exists() {
                bail!(
                    "no case file given and no project file {} next to the model",
                    p.display()
                );
            }
            p
        }
    };
    let text = std::fs::read_to_string(&path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let is_project = serde_json::from_str::<serde_json::Value>(&text)
        .ok()
        .and_then(|v| {
            v.get("format")
                .and_then(|f| f.as_str())
                .map(|f| f == project::PROJECT_FORMAT)
        })
        .unwrap_or(false);
    if is_project {
        let p = project::Project::from_json(&text)
            .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
        Ok((CaseSource::Project(Box::new(p)), path))
    } else {
        Ok((CaseSource::Bare(text), path))
    }
}

fn cmd_loadcase(a: LoadcaseArgs) -> Result<()> {
    let s = load(&a.file)?;
    let (cond, from) = match read_case_or_project(&a.file, a.case.as_deref())? {
        (CaseSource::Bare(text), path) => (
            serde_json::from_str::<loading::LoadingCondition>(&text)
                .with_context(|| format!("cannot parse {}", path.display()))?,
            path,
        ),
        (CaseSource::Project(p), path) => {
            let c = p
                .loading_condition(a.name.as_deref())
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            (p.with_windage(c), path)
        }
    };
    loading::validate(&cond).map_err(|e| anyhow::anyhow!("{}: {e}", from.display()))?;

    let opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    let res = loading::evaluate(&s, &opts, &cond);

    if a.json {
        println!("{}", serde_json::to_string_pretty(&res)?);
        return Ok(());
    }

    println!("{}  -  loading condition: {}", s.settings.name, res.name);
    for line in loading_equilibrium_lines(&res) {
        println!("  {line}");
    }
    println!();
    println!(
        "  {:<34} {:<10} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "item", "kind", "mass [t]", "x [m]", "y [m]", "z [m]", "FSM [t*m]"
    );
    println!("  {}", "-".repeat(96));
    for r in &res.breakdown {
        println!(
            "  {:<34} {:<10} {:>9.3} {:>9.3} {:>+9.3} {:>9.3} {:>10.3}",
            r.name, r.kind, r.mass_t, r.x, r.y, r.z, r.fsm_t_m
        );
    }
    println!("  {}", "-".repeat(96));
    println!(
        "  {:<34} {:<10} {:>9.3} {:>9.3} {:>+9.3} {:>9.3} {:>10.3}",
        "TOTAL / CG", "", res.displacement_t, res.lcg, res.tcg, res.vcg_z, res.fsm_t_m
    );
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row(
        "displacement",
        format!("{:.3} t  ({:.3} m3)", res.displacement_t, res.volume_m3),
    );
    row("LCG / TCG", format!("{:.3} m / {:+.3} m", res.lcg, res.tcg));
    row("KG (solid)", format!("{:.3} m above keel", res.kg_solid));
    row(
        "FSM / FSC",
        format!("{:.3} t*m  ->  {:+.3} m", res.fsm_t_m, res.fsc_m),
    );
    row("KG (fluid)", format!("{:.3} m above keel", res.kg_fluid));
    row(
        "floating condition",
        format!(
            "draft {:.3} m, trim {:+.3} m, list {:+.2} deg",
            res.draft, res.trim_m, res.list_deg
        ),
    );
    row(
        "LCB / LCF",
        format!(
            "{:.3} m / {:.3} m  (LWL {:.3} m)",
            res.lcb_x, res.lcf_x, res.lwl
        ),
    );
    row(
        "KB / BM / KM",
        format!(
            "{:.3} / {:.3} / {:.3} m",
            res.kb, res.bm_transverse, res.km_transverse
        ),
    );
    row(
        "GM (solid / fluid)",
        format!("{:.3} m / {:.3} m", res.gm_solid, res.gm_fluid),
    );

    if let Some(c) = &res.imo_criteria {
        println!(
            "\n  IMO A.749 / 2008 IS Code Part A intact stability criteria (KG_fluid {:.3} m):",
            res.kg_fluid
        );
        println!("  {}", seacraft_core::NOT_VALIDATED_NOTE);
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  status",
            "disp (t)", "A(0-30)", "A(0-40)", "A(30-40)", "GZ(>=30)", "phi_max", "GM0"
        );
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  ------",
            "min req.", ">=0.055", ">=0.090", ">=0.030", ">=0.200 m", ">=25 deg", ">=0.15"
        );
        println!(
            "    {:>8.2}  {:>9.4}  {:>9.4}  {:>10.4}  {:>10.3}  {:>9.1}  {:>7.3}  {}",
            c.displacement_t,
            c.area_0_30_m_rad,
            c.area_0_40_m_rad,
            c.area_30_40_m_rad,
            c.max_gz_30_plus_m,
            c.angle_of_max_gz_deg,
            c.gm0_m,
            if c.passed { "PASS" } else { "FAIL" }
        );
    }
    if let Some(w) = &res.weather {
        print_weather_summary(w);
    }
    Ok(())
}

/// The warning lines a loading table opens with when no equilibrium was
/// found (none when it was): its figures are the hull at the input
/// draft/trim. `--json` carries the same as `equilibrium_found`/`warnings`.
fn loading_equilibrium_lines(res: &loading::LoadingResult) -> Vec<String> {
    if res.equilibrium_found {
        return Vec::new();
    }
    std::iter::once(format!("WARNING: {}", loading::NO_EQUILIBRIUM_NOTE))
        .chain(res.warnings.iter().map(|w| format!("  {w}")))
        .collect()
}

fn cmd_damage(a: DamageArgs) -> Result<()> {
    let s = load(&a.file)?;
    let (case, from) = match read_case_or_project(&a.file, a.case.as_deref())? {
        (CaseSource::Bare(text), path) => (
            serde_json::from_str::<damage::DamageCase>(&text)
                .with_context(|| format!("cannot parse {}", path.display()))?,
            path,
        ),
        (CaseSource::Project(p), path) => {
            let c = p
                .damage_case(a.name.as_deref())
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            let resolved = p
                .resolve_damage_case(c)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            (resolved, path)
        }
    };
    damage::validate(&case).map_err(|e| anyhow::anyhow!("{}: {e}", from.display()))?;

    let opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    let res = damage::evaluate(&s, &opts, &case).map_err(|e| anyhow::anyhow!("{e}"))?;

    if a.json {
        println!("{}", serde_json::to_string_pretty(&res)?);
        return Ok(());
    }

    println!("{}  -  damage case: {}", s.settings.name, res.case_name);
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row(
        "displacement",
        format!(
            "{:.3} t  (net {:.3} m3, rho {:.3} t/m3)",
            res.displacement_t, res.net_volume_m3, res.water_density
        ),
    );
    row(
        "LCG / TCG / KG",
        format!(
            "{:.3} m / {:+.3} m / {:.3} m above keel",
            res.lcg, res.tcg, res.kg
        ),
    );
    row(
        "intact equilibrium",
        format!(
            "draft {:.3} m, trim {:+.3} m",
            res.intact_draft, res.intact_trim_m
        ),
    );
    row(
        "damaged equilibrium",
        format!(
            "draft {:.3} m, trim {:+.3} m, heel {:+.2} deg{}",
            res.draft,
            res.trim_m,
            res.heel_deg,
            if res.equilibrium_found {
                " [FOUND]"
            } else {
                " [UNREACHED]"
            }
        ),
    );
    row("sinkage", format!("{:+.3} m", res.sinkage_m));
    row(
        "freeboard to deck edge",
        match res.freeboard_m {
            Some(fb) if fb < 0.0 => format!("{:+.3} m  [IMMERSED]", fb),
            Some(fb) => format!("{:.3} m", fb),
            None => "—".to_string(),
        },
    );
    row(
        "net buoyancy centroid",
        format!(
            "LCB {:.3} m, TCB {:+.3} m, KB {:.3} m",
            res.lcb_x, res.tcb_y, res.kb
        ),
    );
    println!();
    println!("  flooded compartments:");
    println!(
        "    {:<30} {:>6} {:>8} {:>10} {:>10}",
        "name", "perm", "clipped", "subm [m3]", "lost [m3]"
    );
    println!("    {}", "-".repeat(68));
    for c in &res.compartments {
        println!(
            "    {:<30} {:>6.2} {:>8} {:>10.3} {:>10.3}",
            c.name,
            c.permeability,
            if c.clip_to_hull { "yes" } else { "no" },
            c.submerged_volume_m3,
            c.lost_volume_m3
        );
    }
    println!("    {}", "-".repeat(68));
    println!(
        "    {:<30} {:>6} {:>8} {:>10} {:>10.3}",
        "TOTAL LOST BUOYANCY", "", "", "", res.lost_volume_m3
    );
    println!();
    println!("  residual GZ curve from damaged equilibrium (free trim):");
    println!(
        "    {:>8}  {:>8}  {:>10}  {:>10}  {:>10}",
        "rel deg", "abs deg", "draft [m]", "trim [m]", "GZ [m]"
    );
    for p in &res.residual_gz {
        let rel_int = (p.relative_heel_deg.round()) as i64;
        if rel_int % 5 == 0 || p.relative_heel_deg <= 10.0 {
            println!(
                "    {:>8.1}  {:>8.1}  {:>10}  {:>10}  {:>10}",
                p.relative_heel_deg,
                p.heel_deg,
                p.draft
                    .map(|d| format!("{:.3}", d))
                    .unwrap_or_else(|| "—".to_string()),
                p.trim_m
                    .map(|t| format!("{:+.3}", t))
                    .unwrap_or_else(|| "—".to_string()),
                p.gz.map(|g| format!("{:+.4}", g))
                    .unwrap_or_else(|| "—".to_string()),
            );
        }
    }
    if let Some(c) = &res.criteria {
        println!();
        println!("  residual stability criteria (indicative, SOLAS-style):");
        println!("  {}", seacraft_core::NOT_VALIDATED_NOTE);
        println!(
            "    {:>12}  {:>10}  {:>12}  status",
            "pos range", "max GZ", "area(range)"
        );
        println!(
            "    {:>12}  {:>10}  {:>12}  ------",
            ">=15.0 deg", ">=0.100 m", ">=0.015 m*rad"
        );
        println!(
            "    {:>10.1} deg  {:>8.3} m  {:>10.4} m*r  {}",
            c.positive_range_deg,
            c.max_gz_m,
            c.area_m_rad,
            if c.passed { "PASS" } else { "FAIL" }
        );
        println!("    note: {}", c.label);
    }
    for w in &res.warnings {
        println!("  warning: {w}");
    }
    Ok(())
}

fn cmd_seakeeping(a: SeakeepingArgs) -> Result<()> {
    let s = load(&a.file)?;
    let draft = if let Some(d) = a.draft {
        d
    } else if s.settings.draft > 0.0 {
        s.settings.draft
    } else {
        bail!("draft must be specified via --draft or set in the project settings");
    };
    if draft <= 0.0 {
        bail!("draft must be positive");
    }
    let kg = a.kg.unwrap_or(0.0);

    let spec = seacraft_core::seakeeping::SeakeepingSpec {
        draft,
        speed_kn: a.speed,
        kg,
        kyy: a.kyy,
        stations: a.stations,
        lambda_min: Some(a.lambda_min),
        lambda_max: Some(a.lambda_max),
        points: Some(a.points),
        rho: a.rho,
        g: a.g,
        transom_terms: !a.no_transom,
        ..Default::default()
    };

    let rep = seacraft_core::seakeeping::head_sea_raos(&s, &spec)
        .map_err(|e| anyhow::anyhow!("seakeeping: {e}"))?;
    let irregular = irregular_sea_for(&a, &spec, &s)?;

    if a.json {
        let mut v = serde_json::to_value(&rep)?;
        if let (Some(o), Some(irr)) = (v.as_object_mut(), &irregular) {
            o.insert("irregular".into(), serde_json::to_value(irr)?);
        }
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    if let Some(ref csv_path) = a.csv {
        let mut text = String::from(
            "lambda_over_l,wavelength_m,omega_rad_s,omega_e_rad_s,heave_amp,heave_phase_deg,pitch_amp,pitch_phase_deg\n",
        );
        for pt in &rep.frequencies {
            text.push_str(&format!(
                "{:.4},{:.3},{:.4},{:.4},{:.4},{:.2},{:.4},{:.2}\n",
                pt.lambda_over_l,
                pt.wavelength_m,
                pt.omega,
                pt.omega_e,
                pt.heave_amp,
                pt.heave_phase_deg,
                pt.pitch_amp,
                pt.pitch_phase_deg
            ));
        }
        if csv_path.as_os_str() == "-" || csv_path.as_os_str().is_empty() {
            print!("{text}");
        } else {
            seacraft_core::io::write_atomic(csv_path, text)
                .with_context(|| format!("cannot write {}", csv_path.display()))?;
            println!(
                "wrote {} ({} frequencies)",
                csv_path.display(),
                rep.frequencies.len()
            );
        }
        return Ok(());
    }

    println!(
        "{}  -  Seakeeping RAOs in regular head seas (Salvesen, Tuck & Faltinsen 1970)",
        s.settings.name
    );
    println!(
        "  draft: {:.3} m, speed: {:.1} kn, KG: {:.3} m{}",
        spec.draft,
        spec.speed_kn,
        spec.kg,
        if let Some(kyy) = spec.kyy {
            format!(", kyy: {:.3} m", kyy)
        } else {
            String::new()
        }
    );
    println!(
        "  natural frequencies: heave w_n = {:.3} rad/s, pitch w_n = {:.3} rad/s",
        rep.natural_heave_omega, rep.natural_pitch_omega
    );
    println!();
    println!(
        "  {:>8}   {:>10}   {:>13}   {:>15}   {:>10}   {:>11}   {:>18}   {:>11}",
        "lambda/L",
        "lambda (m)",
        "omega (rad/s)",
        "omega_e (rad/s)",
        "z_a/zeta_a",
        "phase (deg)",
        "theta_a/(k*zeta_a)",
        "phase (deg)"
    );
    for pt in &rep.frequencies {
        println!(
            "  {:>8.3}   {:>10.2}   {:>13.3}   {:>15.3}   {:>10.4}   {:>11.1}   {:>18.4}   {:>11.1}",
            pt.lambda_over_l,
            pt.wavelength_m,
            pt.omega,
            pt.omega_e,
            pt.heave_amp,
            pt.heave_phase_deg,
            pt.pitch_amp,
            pt.pitch_phase_deg
        );
    }
    if let Some(irr) = &irregular {
        println!();
        print_irregular_sea(irr);
    }
    if !rep.warnings.is_empty() {
        println!();
        println!("warnings:");
        for w in &rep.warnings {
            println!("  [{}] {}", w.code, w.message);
        }
    }
    Ok(())
}

/// Irregular head-sea responses when `--hs` and `--tp` are both given.
fn irregular_sea_for(
    a: &SeakeepingArgs,
    spec: &seacraft_core::seakeeping::SeakeepingSpec,
    s: &seacraft_core::Surface,
) -> Result<Option<seacraft_core::seastate::IrregularReport>> {
    use seacraft_core::seastate::{irregular_head_sea, IrregularSpec, MotionPoint, WaveSpectrum};
    let (hs, tp) = match (a.hs, a.tp) {
        (None, None) => {
            if a.gamma.is_some() || !a.point.is_empty() {
                bail!("--gamma and --point need --hs and --tp");
            }
            return Ok(None);
        }
        (Some(hs), Some(tp)) => (hs, tp),
        _ => bail!("--hs and --tp must be given together"),
    };
    let sea = match a.gamma {
        Some(gamma) => WaveSpectrum::Jonswap { hs, tp, gamma },
        None => WaveSpectrum::Bretschneider { hs, tp },
    };
    let mut points = Vec::new();
    for p in &a.point {
        let (name, x) = p
            .rsplit_once(':')
            .with_context(|| format!("--point {p:?}: expected NAME:X"))?;
        let x: f64 = x
            .trim()
            .parse()
            .with_context(|| format!("--point {p:?}: X must be a number"))?;
        if name.trim().is_empty() || !x.is_finite() {
            bail!("--point {p:?}: expected NAME:X with a name and a finite X");
        }
        points.push(MotionPoint {
            name: name.trim().to_string(),
            x,
        });
    }
    let mut irr = IrregularSpec::new(sea, spec.clone());
    irr.points = points;
    irregular_head_sea(s, &irr)
        .map(Some)
        .map_err(|e| anyhow::anyhow!("seakeeping (irregular): {e}"))
}

fn print_irregular_sea(r: &seacraft_core::seastate::IrregularReport) {
    use seacraft_core::seastate::WaveSpectrum;
    println!("Irregular head seas");
    match r.sea {
        WaveSpectrum::Bretschneider { hs, tp } => {
            println!("  Bretschneider spectrum: Hs {hs:.2} m, Tp {tp:.2} s")
        }
        WaveSpectrum::Jonswap { hs, tp, gamma } => {
            println!("  JONSWAP spectrum: Hs {hs:.2} m, Tp {tp:.2} s, gamma {gamma:.2}")
        }
    }
    println!(
        "  spectrum check: Hs = 4*sqrt(m0) = {:.3} m, T1 = {:.2} s, Tz = {:.2} s  ({} frequencies, {:.3}..{:.3} rad/s)",
        r.spectrum.hs_check,
        r.spectrum.t1_s,
        r.spectrum.tz_s,
        r.omega_count,
        r.omega_min,
        r.omega_max
    );
    println!(
        "  speed: {:.1} kn; motion origin LCG at x = {:.3} m",
        r.speed_kn, r.x_lcg
    );
    println!();
    println!(
        "  {:<22} {:>10} {:>10} {:>10}",
        "response", "sig. ampl.", "RMS", "Tz (s)"
    );
    let row = |name: String, st: &seacraft_core::seastate::ResponseStats, unit: &str| {
        println!(
            "  {:<22} {:>10.4} {:>10.4} {:>10.2}  {unit}",
            name, st.significant, st.rms, st.tz_s
        );
    };
    row("heave".into(), &r.heave, "m");
    row("pitch".into(), &r.pitch_deg, "deg");
    for p in &r.points {
        let tag = format!("{} (x {:.2})", p.name, p.x);
        row(format!("{tag} motion"), &p.vertical_motion, "m");
        row(format!("{tag} relative"), &p.relative_motion, "m");
        row(format!("{tag} accel."), &p.vertical_acceleration, "m/s2");
    }
    println!();
    println!(
        "  {:<22} {:>10} {:>12} {:>9}",
        "point", "acc RMS g", "acc sig. g", "MSI %"
    );
    for p in &r.points {
        println!(
            "  {:<22} {:>10.4} {:>12.4} {:>9.1}",
            p.name, p.acceleration_rms_g, p.acceleration_significant_g, p.msi_percent
        );
    }
    println!("  {}", r.msi_note);
    for w in &r.warnings {
        println!("  warning [{}]: {}", w.code, w.message);
    }
}

fn print_weather_summary(w: &seacraft_core::weather::WeatherResult) {
    let opt_deg = |v: Option<f64>| match v {
        Some(x) if x.is_finite() => format!("{x:.2} deg"),
        _ => "—".to_string(),
    };
    println!("\n  IMO 2008 IS Code Part A §2.3 weather criterion:");
    println!("  {}", seacraft_core::NOT_VALIDATED_NOTE);
    println!(
        "    windage   A {:.3} m2 (hull {:.3}, extra {:.3})  Z {:.3} m  P {:.0} Pa",
        w.area_m2, w.hull_windage_area_m2, w.extra_windage_area_m2, w.z_m, w.wind_pressure_pa
    );
    println!("    levers    lw1 {:.4} m   lw2 {:.4} m", w.lw1_m, w.lw2_m);
    println!(
        "    rolling   X1 {:.3}  X2 {:.3}  k {:.3}  r {:.3}  C {:.4}  T {:.2} s  s {:.4}  ->  phi1 {:.2} deg",
        w.x1, w.x2, w.k, w.r, w.c, w.roll_period_s, w.s, w.phi1_deg
    );
    println!(
        "    angles    phi0 {} (limit <= {:.1} deg: {})   phi0-phi1 {}   phic {}   phi2 {}",
        opt_deg(w.phi0_deg),
        w.phi0_limit_deg,
        if w.phi0_passed { "PASS" } else { "FAIL" },
        opt_deg(w.phi_roll_deg),
        opt_deg(w.phic_deg),
        opt_deg(w.phi2_deg)
    );
    println!(
        "    areas     a {:.4} m*rad   b {:.4} m*rad   b/a {:.2} (req b >= a: {})   status {}",
        w.area_a_m_rad,
        w.area_b_m_rad,
        w.area_ratio,
        if w.area_passed { "PASS" } else { "FAIL" },
        if w.passed { "PASS" } else { "FAIL" }
    );
    for note in &w.notes {
        println!("    note      {note}");
    }
}

/// JSON-RPC 2.0 over stdio: one MCP message per line, in and out. Diagnostics
/// go to stderr so the protocol stream stays clean.
fn cmd_mcp(a: McpArgs) -> Result<()> {
    let mut server = match &a.open {
        Some(path) => mcp::McpServer::load_file(path).map_err(anyhow::Error::msg)?,
        None => mcp::McpServer::new(),
    };
    if let Some(addr) = &a.connect {
        match mcp::McpServer::connect(addr, a.token_file.as_deref()) {
            Ok(remote) => {
                if a.open.is_some() {
                    eprintln!("seacraft mcp: --open ignored; the app at {addr} owns the model");
                }
                server = remote;
            }
            // A dead app is not a failure: the model still measures headless.
            Err(e) => eprintln!("seacraft mcp: {e}; running headless"),
        }
    }
    let stdin = std::io::stdin();
    let stdout = std::io::stdout();
    server
        .serve(stdin.lock(), stdout.lock())
        .context("MCP session failed")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// C01: omitting `--lcb` (and `--cp`) asks Lackenby for the hull it was
    /// given. The LCB is measured from the aft end of the waterline, as
    /// `lackenby::fit` measures it; the yacht's counter overhangs the
    /// waterline, where a `model_min.x` datum put the target several
    /// percent forward.
    #[test]
    fn lackenby_without_targets_keeps_the_lcb_on_a_hull_with_an_aft_overhang() {
        let kind = seacraft_core::examples::Example::Yacht10m;
        let (s, _) = seacraft_core::examples::build(kind).expect("yacht");
        let (opts, target) = lackenby_setup(&s, 3, None, None, None);
        let now = hydro::compute(&s, &opts);
        assert!(
            now.wl_aft_x > now.model_min.x + 0.2,
            "the test needs an aft overhang: wl_aft_x {} vs model_min.x {}",
            now.wl_aft_x,
            now.model_min.x
        );
        let (_, report) = lackenby::fit(&s, target, &opts, 1e-3, 10).expect("fit");
        assert!(
            (target.lcb_pct - report.before.lcb_pct).abs() < 1e-9,
            "target LCB {:+.3} % vs the hull's {:+.3} %",
            target.lcb_pct,
            report.before.lcb_pct
        );
        assert!((target.cp - report.before.cp).abs() < 1e-12);
        assert!(report.converged, "the identity target is reachable");
        assert!(
            (report.after.lcb_pct - report.before.lcb_pct).abs() < 0.05,
            "LCB moved from {:+.3} % to {:+.3} %",
            report.before.lcb_pct,
            report.after.lcb_pct
        );

        // A Cp-only request holds the LCB too.
        let (opts, target) = lackenby_setup(&s, 3, None, Some(report.before.cp + 0.01), None);
        assert!((target.lcb_pct - report.before.lcb_pct).abs() < 1e-9);
        let (_, moved) = lackenby::fit(&s, target, &opts, 1e-3, 10).expect("fit");
        assert!(
            (moved.after.lcb_pct - report.before.lcb_pct).abs() < 0.1,
            "LCB moved from {:+.3} % to {:+.3} %",
            report.before.lcb_pct,
            moved.after.lcb_pct
        );
    }

    /// R8-3 (A08 follow-up): a loading condition the hull cannot float says
    /// so in the table and in `--json`; one that floats prints no warning.
    #[test]
    fn loadcase_reports_a_missing_equilibrium_in_table_and_json() {
        let kind = seacraft_core::examples::Example::Runabout6m;
        let (s, project) = seacraft_core::examples::build(kind).expect("runabout");
        let opts = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 1,
                mode: seacraft_core::subdiv::SubdivisionMode::Loop,
            },
        );
        let cond = project.with_windage(&project.loading_conditions[0]);
        let fine = loading::evaluate(&s, &opts, &cond);
        assert!(fine.equilibrium_found);
        assert!(loading_equilibrium_lines(&fine).is_empty());
        let json = serde_json::to_value(&fine).unwrap();
        assert_eq!(json["equilibrium_found"], serde_json::json!(true));
        assert_eq!(json["warnings"], serde_json::json!([]));

        let mut heavy = cond.clone();
        heavy.lightship.mass_t = 1000.0;
        let res = loading::evaluate(&s, &opts, &heavy);
        let lines = loading_equilibrium_lines(&res);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert!(lines[0].starts_with("WARNING: no equilibrium"), "{lines:?}");
        assert!(lines[1].contains("cannot float"), "{lines:?}");
        let json = serde_json::to_value(&res).unwrap();
        assert_eq!(json["equilibrium_found"], serde_json::json!(false));
        assert!(json["warnings"][0]
            .as_str()
            .is_some_and(|w| w.contains("cannot float")));
    }

    fn parse_export(args: &[&str]) -> ExportArgs {
        let mut argv = vec!["seacraft", "export"];
        argv.extend_from_slice(args);
        match Cli::try_parse_from(argv).expect("parses").command {
            Command::Export(a) => a,
            other => panic!("not export: {other:?}"),
        }
    }

    /// C03 + C08: `--control`, `--no-mirror` and `--up y` only shape OBJ;
    /// the other formats refuse them by name instead of ignoring them.
    #[test]
    fn export_refuses_obj_only_flags_for_other_formats() {
        let dir = std::env::temp_dir().join(format!("seacraft-cli-export-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let fef_path = dir.join("demo.fef");
        std::fs::write(&fef_path, fef::write(&demo::demo_hull(true))).unwrap();
        let fef_text = fef_path.to_string_lossy().into_owned();

        for ext in ["stl", "3mf", "ply", "igs", "html"] {
            let out = dir.join(format!("hull.{ext}"));
            let out_text = out.to_string_lossy().into_owned();
            for (flags, named) in [
                (&["--control"][..], "--control"),
                (&["--no-mirror"][..], "--no-mirror"),
                (&["--up", "y"][..], "--up y"),
            ] {
                let mut args = vec![fef_text.as_str(), "-o", out_text.as_str()];
                args.extend_from_slice(flags);
                let err = cmd_export(parse_export(&args)).expect_err("refused");
                let msg = format!("{err}");
                assert!(msg.contains(named), "{ext} {flags:?}: {msg}");
                assert!(msg.contains(".obj"), "{msg}");
                assert!(!out.exists(), "nothing is written for {ext} {flags:?}");
            }
        }

        // `--up z` is what the other formats are anyway; OBJ takes them all.
        let stl = dir.join("hull.stl");
        let stl_text = stl.to_string_lossy().into_owned();
        cmd_export(parse_export(&[&fef_text, "-o", &stl_text, "--up", "z"])).expect("z-up STL");
        assert!(stl.exists());
        let obj = dir.join("hull.obj");
        let obj_text = obj.to_string_lossy().into_owned();
        cmd_export(parse_export(&[
            &fef_text,
            "-o",
            &obj_text,
            "--control",
            "--no-mirror",
            "--up",
            "y",
        ]))
        .expect("OBJ honours every flag");
        assert!(std::fs::read_to_string(&obj)
            .unwrap()
            .contains("unmirrored"));
        // An unknown format still names the format, not a flag.
        let tiff = dir.join("hull.tiff").to_string_lossy().into_owned();
        let err = cmd_export(parse_export(&[&fef_text, "-o", &tiff, "--control"])).unwrap_err();
        assert!(
            format!("{err}").contains("unsupported output format"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
