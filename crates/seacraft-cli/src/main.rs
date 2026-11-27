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
    curves, damage, demo, develop, export, fef,
    i18n::{resolve_locale, Key, LanguagePref, Locale, Tr},
    lackenby, linesplan, loading,
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
    version = seacraft_core::VERSION,
    about = "Subdivision-surface hull design, hydrostatics and stability (FREE!ship compatible)",
    long_about = "SeaCraft is a Rust reimplementation of the FREE!ship hull modeller.\n\
                  It reads and writes FREE!ship .fef exchange files, refines the control\n\
                  net with the same Loop/Catmull-Clark scheme and reports hydrostatics\n\
                  and stability the way the original application does.",
    after_help = "SeaCraft comes with ABSOLUTELY NO WARRANTY, and its results are not validated.\n\
                  It is free software under GPL-3.0-or-later; see LICENSE and DISCLAIMER.md."
)]
struct Cli {
    /// Display language for human-readable output and generated documents (en, tr)
    #[arg(long, global = true, value_name = "CODE")]
    lang: Option<String>,
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
    /// Still-water resistance and effective power (Holtrop-Mennen 1982/1984)
    Resistance(ResistanceArgs),
    /// Far-field thin-ship (Michell) wave pattern behind the hull: PNG, SVG contours, VTK
    Waves(WavesArgs),
    /// Developable plate expansion: flatten a strake or buttock band
    Unfold(UnfoldArgs),
    /// Edit the control net: split faces, insert edges, toggle creases
    Edit(EditArgs),
    /// Savitsky planing resistance: trim, mean wetted length, wetted area and drag
    Planing(PlaningArgs),
    /// Cross curves of stability (KN / GZ)
    Stability(StabilityArgs),
    /// Evaluate a ship loading condition (weights, tanks, FSM/FSC, floating equilibrium and IMO A.749)
    Loadcase(LoadcaseArgs),
    /// Builder's weight estimate: shell, glass, resin, tape, fillets and plywood sheets
    Materials(MaterialsArgs),
    /// Where the hull settles under trial weights: large-angle heel, trim and draft, free to trim
    Heel(HeelArgs),
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

fn cmd_trim(a: TrimArgs, tr: Tr) -> Result<()> {
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
    let not_held = if a.no_hold_volume {
        tr.s(Key::CliTrimNotHeld)
    } else {
        ""
    };
    println!(
        "{}",
        tr.fmt(
            Key::CliTrimHeading,
            &[
                ("speed", &format!("{:.2}", scan.speed_kn)),
                ("volume", &format!("{:.3}", scan.target_volume_m3)),
                ("not_held", not_held),
            ]
        )
    );
    println!(
        "   {:>4}  {:>7}     LWL      Fn     RT [kN]   PE [kW]",
        tr.s(Key::CliTrimColTrim),
        tr.s(Key::CliTrimColDraft)
    );
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
        let boundary = if scan.at_boundary {
            tr.s(Key::CliTrimAtBoundary)
        } else {
            ""
        };
        println!(
            "{}",
            tr.fmt(
                Key::CliTrimCheapest,
                &[
                    ("trim", &format!("{:+.3}", b.trim_m)),
                    ("draft", &format!("{:.3}", b.draft_m)),
                    ("rt", &format!("{:.3}", b.rt_n / 1000.0)),
                    ("pe", &format!("{:.2}", b.pe_kw)),
                    ("boundary", boundary),
                ]
            )
        );
    }
    for w in &scan.warnings {
        println!(
            "{}",
            tr.fmt(Key::CliCommonWarningIndent, &[("message", &w.render(tr))])
        );
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

fn cmd_optimize(a: OptimizeArgs, tr: Tr) -> Result<()> {
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
        "{}",
        tr.fmt(
            Key::CliOptimizeHeading,
            &[
                ("name", &s.settings.name),
                ("speed", &format!("{:.2}", report.speed_kn)),
                ("method", &format!("{:?}", report.method)),
                ("fn", &format!("{:.3}", b.froude)),
            ]
        )
    );
    println!(
        "  {:<14} {:>12} {:>12} {:>10}",
        "",
        tr.s(Key::CliOptimizeColBaseline),
        tr.s(Key::CliOptimizeColOptimised),
        tr.s(Key::CliOptimizeColChange)
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
    row(
        tr.s(Key::CliOptimizeRowDispl),
        b.displacement_t,
        o.displacement_t,
        3,
    );
    row(tr.s(Key::CliOptimizeRowVolume), b.volume_m3, o.volume_m3, 3);
    row(tr.s(Key::CliOptimizeRowDraft), b.draft_m, o.draft_m, 4);
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
    let status = if report.constraints_met {
        tr.s(Key::CliOptimizeMet)
    } else {
        tr.s(Key::CliOptimizeNotMet)
    };
    println!(
        "{}",
        tr.fmt(
            Key::CliOptimizeSummary,
            &[
                ("rt_pct", &format!("{:+.2}", report.rt_change_pct)),
                ("disp_err", &format!("{:.3}", report.displacement_error_pct)),
                ("status", status),
            ]
        )
    );
    println!("{}", tr.s(Key::CliOptimizeVariables));
    for v in &report.variables {
        println!(
            "    {:<16} {:>9.5}  (range {:.4} .. {:.4}{})",
            v.name,
            v.value,
            v.lo,
            v.hi,
            if v.at_bound {
                tr.s(Key::CliOptimizeAtBound)
            } else {
                ""
            }
        );
    }
    println!(
        "{}",
        tr.fmt(
            Key::CliOptimizeEvals,
            &[
                ("iters", &report.iterations.to_string()),
                ("evals", &report.evaluations.to_string()),
                ("unique", &report.unique_evaluations.to_string()),
                ("reason", &report.convergence_reason),
            ]
        )
    );
    for w in &report.warnings {
        println!(
            "{}",
            tr.fmt(Key::CliCommonWarningIndent, &[("message", &w.render(tr))])
        );
    }
    let out_str = a.output.display().to_string();
    println!(
        "{}",
        tr.fmt(Key::CliCommonWroteIndent, &[("path", &out_str)])
    );
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

fn cmd_table(a: TableArgs, tr: Tr) -> Result<()> {
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
        seacraft_core::io::write_atomic(path, t.html_tr(tr))
            .with_context(|| format!("cannot write {}", path.display()))?;
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&t)?);
        return Ok(());
    }
    println!(
        "{}",
        tr.fmt(
            Key::CliTableHeading,
            &[
                ("name", &t.name),
                ("density", &format!("{:.1}", t.water_density)),
                ("level", &a.level.to_string()),
            ]
        )
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
        let path_str = p.display().to_string();
        println!(
            "{}",
            tr.fmt(Key::CliCommonWroteIndent, &[("path", &path_str)])
        );
    }
    if let Some(p) = &a.html {
        let path_str = p.display().to_string();
        println!(
            "{}",
            tr.fmt(Key::CliCommonWroteIndent, &[("path", &path_str)])
        );
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

fn cmd_lackenby(a: LackenbyArgs, tr: Tr) -> Result<()> {
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
    println!("{}", tr.s(Key::CliLackenbyHeading));
    println!(
        "{}",
        tr.fmt(
            Key::CliLackenbyBefore,
            &[
                ("cp", &format!("{:.4}", report.before.cp)),
                ("lcb", &format!("{:+.2}", report.before.lcb_pct)),
                ("v", &format!("{:.3}", report.before.volume_m3)),
                ("lwl", &format!("{:.3}", report.before.lwl)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliLackenbyTarget,
            &[
                ("cp", &format!("{:.4}", target.cp)),
                ("lcb", &format!("{:+.2}", target.lcb_pct)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliLackenbyAfter,
            &[
                ("cp", &format!("{:.4}", report.after.cp)),
                ("lcb", &format!("{:+.2}", report.after.lcb_pct)),
                ("v", &format!("{:.3}", report.after.volume_m3)),
                ("lwl", &format!("{:.3}", report.after.lwl)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliLackenbyError,
            &[
                ("dcp", &format!("{:+.5}", report.cp_error)),
                ("dlcb", &format!("{:+.4}", report.lcb_error)),
                ("iters", &report.iterations.to_string()),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliLackenbyShift,
            &[
                ("a", &format!("{:+.4}", report.coefficients.0)),
                ("b", &format!("{:+.4}", report.coefficients.1)),
                ("mid", &format!("{:.3}", report.midship_shift_m)),
            ]
        )
    );
    if !report.converged {
        println!("{}", tr.s(Key::CliLackenbyNotConverged));
    }
    let out_str = a.output.display().to_string();
    println!(
        "{}",
        tr.fmt(Key::CliCommonWroteIndent, &[("path", &out_str)])
    );
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
struct WavesArgs {
    /// Input .fef file
    file: PathBuf,
    /// Ship speed in knots
    #[arg(
        long,
        allow_negative_numbers = true,
        required_unless_present = "fn_",
        conflicts_with = "fn_"
    )]
    speed: Option<f64>,
    /// Froude number on the waterline length, instead of --speed
    #[arg(long = "fn", allow_negative_numbers = true)]
    fn_: Option<f64>,
    /// Draft to measure at (default: the model's design draft)
    #[arg(long)]
    draft: Option<f64>,
    /// Catamaran demihull centreline spacing in metres (the model is one demihull)
    #[arg(long, allow_negative_numbers = true)]
    cat_spacing: Option<f64>,
    /// Domain ahead of the bow, in waterline lengths
    #[arg(long, default_value_t = 1.0, allow_negative_numbers = true)]
    ahead: f64,
    /// Domain behind the stern, in waterline lengths
    #[arg(long, default_value_t = 3.0, allow_negative_numbers = true)]
    behind: f64,
    /// Domain half-width each side of the centreline, in waterline lengths
    #[arg(long, default_value_t = 1.5, allow_negative_numbers = true)]
    half_width: f64,
    /// Grid spacing in metres (default: the smaller of a twentieth of the transverse wavelength and
    /// L_WL/60)
    #[arg(long, allow_negative_numbers = true)]
    spacing: Option<f64>,
    /// Lateral distance in metres of a wash line: prints the wash height there (a thin-ship estimate
    /// in deep water)
    #[arg(long, allow_negative_numbers = true)]
    wash_distance: Option<f64>,
    /// Write the pattern here: .png, .svg or .vti (repeat for several)
    #[arg(short, long)]
    output: Vec<PathBuf>,
    /// Colour and contour clip in metres (default: the largest |elevation|)
    #[arg(long, allow_negative_numbers = true)]
    clip: Option<f64>,
    /// y in metres of a longitudinal cut over the field's length, written with --cut-csv
    #[arg(long, allow_negative_numbers = true, requires = "cut_csv")]
    cut_y: Option<f64>,
    /// Write the longitudinal cut (2000 samples, x_m,zeta_m) to this CSV
    #[arg(long, requires = "cut_y")]
    cut_csv: Option<PathBuf>,
    /// Emit a JSON summary instead of text
    #[arg(long)]
    json: bool,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

/// What `seacraft waves -o` can write.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum WaveFormat {
    Png,
    Svg,
    Vti,
}

fn wave_format(path: &Path) -> Result<WaveFormat> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    match ext.as_deref() {
        Some("png") => Ok(WaveFormat::Png),
        Some("svg") => Ok(WaveFormat::Svg),
        Some("vti") => Ok(WaveFormat::Vti),
        _ => bail!(
            "{}: unknown output format, use .png, .svg or .vti",
            path.display()
        ),
    }
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
struct MaterialsArgs {
    /// Input .fef file
    file: PathBuf,
    /// Project file (defaults to the model's `<model>.seacraft.json` sidecar; none means an empty
    /// schedule)
    case: Option<PathBuf>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit JSON instead of a table
    #[arg(long, conflicts_with = "csv")]
    json: bool,
    /// Emit CSV, one row per layer, seam or plywood line
    #[arg(long)]
    csv: bool,
    /// Put the estimate into this loading condition of the project file
    #[arg(long = "add-to", value_name = "CONDITION")]
    add_to: Option<String>,
    /// Name of the weight item (defaults to "Hull materials (estimate)")
    #[arg(long = "item", requires = "add_to")]
    item: Option<String>,
}

#[derive(Args, Debug)]
struct HeelArgs {
    /// Input .fef file
    file: PathBuf,
    /// KG above the keel in metres for the bare hull (or give --condition)
    #[arg(long, conflicts_with_all = ["condition", "case"])]
    kg: Option<f64>,
    /// Draft the bare hull floats at before the weights (defaults to the model's)
    #[arg(long, conflicts_with_all = ["condition", "case"])]
    draft: Option<f64>,
    /// LCG of the bare hull in model x (defaults to the upright LCB)
    #[arg(long, conflicts_with_all = ["condition", "case"])]
    lcg: Option<f64>,
    /// Start from this loading condition of the project file
    #[arg(long)]
    condition: Option<String>,
    /// Loading condition JSON or project file (defaults to the model's
    /// `<model>.seacraft.json` sidecar when --condition is given)
    #[arg(long)]
    case: Option<PathBuf>,
    /// A trial weight, tonnes at model x, y, z in metres (repeatable)
    #[arg(long = "weight", value_name = "T@X,Y,Z")]
    weights: Vec<String>,
    /// Also report the lever at this heel, degrees (+ to starboard)
    #[arg(long, allow_negative_numbers = true)]
    heel: Option<f64>,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit JSON instead of text
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
    /// Automatically nest pieces onto plywood sheets: none, blanks or split
    #[arg(long, value_name = "MODE", default_value = "none")]
    nest: String,
    /// Gap between nested pieces and from the sheet edges, millimetres
    #[arg(
        long,
        value_name = "MM",
        default_value_t = 10.0,
        allow_negative_numbers = true
    )]
    gap_mm: f64,
    /// Write the patterns even when a panel does not lie flat; each such pattern is headed so
    #[arg(long)]
    allow_undevelopable: bool,
    /// Leave the panel offsets tables out of the pattern PDF
    #[arg(long)]
    no_offsets_pages: bool,
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
    let cli = Cli::parse();
    let cli_locale = match cli.lang.as_deref() {
        Some(code) => Some(
            Locale::from_code(code)
                .ok_or_else(|| anyhow::anyhow!("unknown language: {code} (en, tr)"))?,
        ),
        None => None,
    };
    let env_locale = Locale::from_env_checked().unwrap_or_else(|bad| {
        eprintln!("warning: SEACRAFT_LANG={bad} is not a supported language (en, tr); ignored");
        None
    });
    let locale = resolve_locale(
        cli_locale,
        env_locale,
        LanguagePref::Auto,
        Locale::detect_system(),
    );
    let tr = Tr::new(locale);
    match cli.command {
        Command::Demo(a) => cmd_demo(a, tr),
        Command::Example(a) => cmd_example(a, tr),
        Command::Generate(a) => cmd_generate(a, tr),
        Command::Info(a) => cmd_info(a, tr),
        Command::Hydro(a) => cmd_hydro(a, tr),
        Command::Export(a) => cmd_export(a, tr),
        Command::Subdiv(a) => cmd_subdiv(a, tr),
        Command::Sac(a) => cmd_sac(a, tr),
        Command::Bonjean(a) => cmd_bonjean(a, tr),
        Command::Offsets(a) => cmd_offsets(a, tr),
        Command::Transform(a) => cmd_transform(a, tr),
        Command::Lackenby(a) => cmd_lackenby(a, tr),
        Command::Table(a) => cmd_table(a, tr),
        Command::Trim(a) => cmd_trim(a, tr),
        Command::Optimize(a) => cmd_optimize(a, tr),
        Command::Resistance(a) => cmd_resistance(a, tr),
        Command::Waves(a) => cmd_waves(a, tr),
        Command::Unfold(a) => cmd_unfold(a, tr),
        Command::Edit(a) => cmd_edit(a, tr),
        Command::Planing(a) => cmd_planing(a, tr),
        Command::Stability(a) => cmd_stability(a, tr),
        Command::Loadcase(a) => cmd_loadcase(a, tr),
        Command::Materials(a) => cmd_materials(a, tr),
        Command::Heel(a) => cmd_heel(a, tr),
        Command::Damage(a) => cmd_damage(a, tr),
        Command::Seakeeping(a) => cmd_seakeeping(a, tr),
        Command::Linesplan(a) => cmd_linesplan(a, tr),
        Command::Templates(a) => cmd_templates(a, tr),
        Command::Panels(a) => cmd_panels(a, tr),
        Command::Report(a) => cmd_report(a, tr),
        Command::Render(a) => cmd_render(a, tr),
        Command::Mcp(a) => cmd_mcp(a),
    }
}

fn load(path: &Path) -> Result<Surface> {
    let text =
        std::fs::read_to_string(path).with_context(|| format!("cannot read {}", path.display()))?;
    let surface = fef::parse(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    Ok(surface)
}

fn cmd_demo(a: DemoArgs, tr: Tr) -> Result<()> {
    let surface = demo::demo_hull(a.symmetric);
    seacraft_core::io::write_atomic(&a.output, fef::write(&surface))
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    // The demo's KG travels with it, as for the examples, so opening the
    // file gives the same stability as the built-in demo.
    let sidecar = project::sidecar_path(&a.output);
    demo::demo_project()
        .save(&sidecar)
        .map_err(|e| anyhow::anyhow!(e))?;
    let key = if surface.layers.len() == 1 {
        Key::CliDemoWroteOne
    } else {
        Key::CliDemoWroteOther
    };
    println!(
        "{}",
        tr.fmt(
            key,
            &[
                ("fef", &a.output.display().to_string()),
                ("sidecar", &sidecar.display().to_string()),
                ("points", &surface.points.len().to_string()),
                ("faces", &surface.faces.len().to_string()),
                ("layers", &surface.layers.len().to_string()),
            ]
        )
    );
    Ok(())
}

fn cmd_example(a: ExampleArgs, tr: Tr) -> Result<()> {
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
                println!(
                    "{:<16} {}: {}",
                    e.slug(),
                    e.title_tr(tr),
                    e.description_tr(tr)
                );
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
            write_example(e, &dir.join(format!("{}.fef", e.slug())), tr)?;
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
    write_example(kind, &out, tr)
}

/// Write an example's `.fef` and its sidecar next to it.
fn write_example(kind: seacraft_core::examples::Example, out: &Path, tr: Tr) -> Result<()> {
    let (surface, project) =
        seacraft_core::examples::build(kind).map_err(|e| anyhow::anyhow!(e))?;
    seacraft_core::io::write_atomic(out, fef::write(&surface))
        .with_context(|| format!("cannot write {}", out.display()))?;
    let sidecar = project::sidecar_path(out);
    project.save(&sidecar).map_err(|e| anyhow::anyhow!(e))?;
    println!(
        "{}",
        tr.fmt(
            Key::CliExampleWrote,
            &[
                ("fef", &out.display().to_string()),
                ("sidecar", &sidecar.display().to_string()),
                ("title", kind.title_tr(tr)),
                ("points", &surface.points.len().to_string()),
                ("faces", &surface.faces.len().to_string()),
                ("layers", &surface.layers.len().to_string()),
            ]
        )
    );
    Ok(())
}

fn cmd_generate(a: GenerateArgs, tr: Tr) -> Result<()> {
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

    println!(
        "{}",
        tr.fmt(
            Key::CliGenerateHeading,
            &[
                ("kind", &rep.kind.to_string()),
                ("path", &a.output.display().to_string()),
            ]
        )
    );
    println!("{}", tr.s(Key::CliGenerateCols));
    println!(
        "  {:<14}{:8.3}     {:8.3}",
        tr.s(Key::CliGenerateLwl),
        rep.target_lwl,
        rep.lwl
    );
    println!(
        "  {:<14}{:8.3}     {:8.3}",
        tr.s(Key::CliGenerateBwl),
        rep.target_bwl,
        rep.bwl
    );
    println!(
        "  {:<14}{:8.3}     {:8.3}",
        tr.s(Key::CliGenerateDraft),
        rep.target_draft,
        rep.t
    );
    println!(
        "  {:<14}{:8.3}     {:8.3}",
        tr.s(Key::CliGenerateDepth),
        rep.target_depth,
        rep.target_depth
    );
    match rep.target_cb {
        Some(tcb) => println!(
            "  {:<14}{:8.4}     {:8.4}",
            tr.s(Key::CliGenerateCb),
            tcb,
            rep.cb
        ),
        None => println!(
            "  {:<14}       -     {:8.4}",
            tr.s(Key::CliGenerateCb),
            rep.cb
        ),
    }
    match rep.target_lcb_frac {
        Some(tlcb) => println!(
            "  {:<14}{:8.4}     {:8.4}",
            tr.s(Key::CliGenerateLcbLwl),
            tlcb,
            rep.lcb_frac
        ),
        None => println!(
            "  {:<14}       -     {:8.4}",
            tr.s(Key::CliGenerateLcbLwl),
            rep.lcb_frac
        ),
    }
    println!(
        "  {:<14}       -     {:8.3}",
        tr.s(Key::CliGenerateVolume),
        rep.volume
    );
    println!(
        "  {:<14}       -     {:8.3}",
        tr.s(Key::CliGenerateDisp),
        rep.displacement_t
    );
    println!(
        "  {:<14}       -     {:8}",
        tr.s(Key::CliGenerateIterations),
        rep.iterations
    );
    Ok(())
}

fn cmd_info(a: InfoArgs, tr: Tr) -> Result<()> {
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
        println!(
            "{}",
            tr.fmt(Key::CliInfoDesigner, &[("designer", &s.settings.designer)])
        );
    }
    println!(
        "{}",
        tr.fmt(
            Key::CliInfoPrincipal,
            &[
                ("l", &format!("{:.3}", s.settings.length)),
                ("b", &format!("{:.3}", s.settings.beam)),
                ("t", &format!("{:.3}", s.settings.draft)),
                ("rho", &format!("{:.0}", s.settings.water_density)),
                ("app", &format!("{:.3}", s.settings.appendage_coefficient)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliInfoExtent,
            &[
                ("xmin", &format!("{:.3}", min.x)),
                ("xmax", &format!("{:.3}", max.x)),
                ("ymin", &format!("{:.3}", min.y)),
                ("ymax", &format!("{:.3}", max.y)),
                ("zmin", &format!("{:.3}", min.z)),
                ("zmax", &format!("{:.3}", max.z)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliInfoControlNet,
            &[
                ("points", &s.points.len().to_string()),
                ("edges", &s.edges.len().to_string()),
                ("creases", &creases.to_string()),
                ("boundary", &boundary_edges.to_string()),
                ("non_manifold", &non_manifold.to_string()),
                ("faces", &s.faces.len().to_string()),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliInfoFaceMix,
            &[
                ("quads", &quads.to_string()),
                ("tris", &tris.to_string()),
                ("other", &other.to_string()),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliInfoControlArea,
            &[("area", &format!("{:.3}", s.control_area()))]
        )
    );
    println!("{}", tr.s(Key::CliInfoLayers));
    for l in &s.layers {
        println!(
            "    {:<16} id {:<3} {}{}{} rho {:.0} t {:.3}",
            l.name,
            l.id,
            if l.visible {
                tr.s(Key::CliInfoVisible)
            } else {
                tr.s(Key::CliInfoHidden)
            },
            if l.symmetric {
                tr.s(Key::CliInfoSymmetric)
            } else {
                tr.s(Key::CliInfoAsymmetric)
            },
            if l.use_in_hydrostatics {
                tr.s(Key::CliInfoHydro)
            } else {
                "-"
            },
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

fn cmd_hydro(a: HydroArgs, tr: Tr) -> Result<()> {
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
            "{}",
            tr.fmt(
                Key::CliHydroHeadingFree,
                &[
                    ("name", &s.settings.name),
                    ("draft", &format!("{:.3}", result.draft)),
                    ("trim", &format!("{:+.3}", result.trim_m)),
                ]
            )
        );
    } else {
        println!(
            "{}",
            tr.fmt(
                Key::CliHydroHeading,
                &[
                    ("name", &s.settings.name),
                    ("draft", &format!("{:.3}", result.draft)),
                ]
            )
        );
    }
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row(
        tr.s(Key::CliHydroVolume),
        format!("{:.3} m3", result.volume_m3),
    );
    row(
        tr.s(Key::CliHydroDisplacement),
        format!(
            "{:.0} kg  ({:.3} t)",
            result.displacement_kg, result.displacement_t
        ),
    );
    row(
        tr.s(Key::CliHydroCb),
        format!("{:.4}", result.block_coefficient),
    );
    row(
        tr.s(Key::CliHydroCp),
        format!("{:.4}", result.prismatic_coefficient),
    );
    row(
        tr.s(Key::CliHydroCm),
        format!("{:.4}", result.midship_coefficient),
    );
    row(
        tr.s(Key::CliHydroCwp),
        format!("{:.4}", result.waterplane_coefficient),
    );
    row(
        tr.s(Key::CliHydroCvp),
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
    row(
        "KB",
        tr.fmt(
            Key::CliHydroKbAboveKeel,
            &[("kb", &format!("{:.3}", result.kb))],
        ),
    );
    row(
        "LCF",
        format!("x {:.3}  y {:.3}", result.lcf.x, result.lcf.y),
    );
    println!();
    row(
        tr.s(Key::CliHydroLwlXBwl),
        format!("{:.3} x {:.3} m", result.lwl, result.bwl),
    );
    row(
        tr.s(Key::CliHydroWettedSurface),
        format!("{:.3} m2", result.wetted_surface_m2),
    );
    row(
        tr.s(Key::CliHydroWaterplaneArea),
        format!("{:.3} m2", result.waterplane_area_m2),
    );
    row(
        tr.s(Key::CliHydroWaterplaneInertia),
        format!(
            "{:.1} / {:.1} m4",
            result.waterplane_inertia_transverse, result.waterplane_inertia_longitudinal
        ),
    );
    row(
        tr.s(Key::CliHydroBmTransverse),
        format!("{:.3} m", result.bm_transverse),
    );
    row(
        tr.s(Key::CliHydroBmLongitudinal),
        format!("{:.3} m", result.bm_longitudinal),
    );
    row(
        tr.s(Key::CliHydroKmTransverse),
        format!("{:.3} m", result.km_transverse),
    );
    row(
        tr.s(Key::CliHydroKmLongitudinal),
        format!("{:.3} m", result.km_longitudinal),
    );
    if let Some(kg) = a.kg {
        row(
            &tr.fmt(Key::CliHydroGmLabel, &[("kg", &format!("{:.3}", kg))]),
            format!("{:.3} m", result.gm(kg)),
        );
    }
    println!();
    row(
        tr.s(Key::CliHydroMidshipArea),
        tr.fmt(
            Key::CliHydroMidshipVal,
            &[
                ("area", &format!("{:.4}", result.mainframe_area_m2)),
                ("x", &format!("{:.3}", result.mainframe_x)),
            ],
        ),
    );
    row(
        tr.s(Key::CliHydroLateralArea),
        format!("{:.3} m2", result.lateral_area_m2),
    );
    if !result.sac.is_empty() {
        println!();
        println!(
            "{}",
            tr.fmt(
                Key::CliHydroSacHeading,
                &[("count", &result.sac.len().to_string())]
            )
        );
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
            println!(
                "{}",
                tr.fmt(
                    Key::CliHydroWarning,
                    &[("code", w.code), ("message", &w.message.render(tr))]
                )
            );
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

fn cmd_export(a: ExportArgs, tr: Tr) -> Result<()> {
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
        "{}",
        tr.fmt(
            Key::CliCommonWroteKib,
            &[
                ("path", &a.output.display().to_string()),
                ("kib", &format!("{:.1}", data.len() as f64 / 1024.0)),
            ]
        )
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

fn cmd_subdiv(a: SubdivArgs, tr: Tr) -> Result<()> {
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
        "{}",
        tr.fmt(
            Key::CliSubdivWrote,
            &[
                ("path", &a.output.display().to_string()),
                ("points", &baked.points.len().to_string()),
                ("faces", &baked.faces.len().to_string()),
            ]
        )
    );
    Ok(())
}

fn cmd_sac(a: SacArgs, tr: Tr) -> Result<()> {
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
        let title = tr.fmt(
            Key::CliSacSvgTitle,
            &[
                ("name", &s.settings.name),
                ("draft", &format!("{:.3}", opts.draft)),
            ],
        );
        seacraft_core::io::write_atomic(path, curves::sac_svg_tr(&result.sac, &title, tr))
            .with_context(|| format!("cannot write {}", path.display()))?;
        let path_str = path.display().to_string();
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
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
            println!(
                "{}",
                tr.fmt(
                    Key::CliCommonWroteStations,
                    &[
                        ("path", &path.display().to_string()),
                        ("count", &result.sac.len().to_string()),
                    ]
                )
            );
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

fn cmd_bonjean(a: BonjeanArgs, tr: Tr) -> Result<()> {
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
        let title = tr.fmt(
            Key::CliBonjeanSvgTitle,
            &[
                ("name", &s.settings.name),
                ("count", &bonjean.sections.len().to_string()),
                ("draft", &format!("{:.3}", opts.draft)),
            ],
        );
        seacraft_core::io::write_atomic(path, curves::bonjean_svg_tr(&bonjean, &title, tr))
            .with_context(|| format!("cannot write {}", path.display()))?;
        let path_str = path.display().to_string();
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
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
            "{}",
            tr.fmt(
                Key::CliCommonWroteStations,
                &[
                    ("path", &path.display().to_string()),
                    ("count", &bonjean.sections.len().to_string()),
                ]
            )
        );
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&bonjean)?);
        return Ok(());
    }
    if a.csv.is_some() || a.svg.is_some() {
        // Files were requested: a short confirmation is enough.
        println!(
            "{}",
            tr.fmt(
                Key::CliBonjeanSummary,
                &[
                    ("stations", &bonjean.sections.len().to_string()),
                    ("drafts", &bonjean.drafts_m.len().to_string()),
                    ("max_area", &format!("{:.3}", bonjean.max_area())),
                    ("depth", &format!("{:.3}", depth)),
                ]
            )
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
        "{}",
        tr.fmt(
            Key::CliBonjeanCaption,
            &[("max_area", &format!("{:.3}", bonjean.max_area()))]
        )
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

fn cmd_offsets(a: OffsetsArgs, tr: Tr) -> Result<()> {
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
    let title = tr.fmt(Key::OffsetsPdfDocTitle, &[("name", &s.settings.name)]);
    // With --json the standard output is the table alone: what was written goes to stderr.
    let written = |line: String| {
        if a.json {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    };

    if let Some(path) = &a.pdf {
        let doc = offsets::pdf_tr(&table, &title, a.imperial, tr).map_err(anyhow::Error::msg)?;
        seacraft_core::io::write_atomic(path, doc.bytes)
            .with_context(|| format!("cannot write {}", path.display()))?;
        let path_str = path.display().to_string();
        written(tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
    }

    if let Some(path) = &a.html {
        seacraft_core::io::write_atomic(path, offsets::html_tr(&table, &title, tr))
            .with_context(|| format!("cannot write {}", path.display()))?;
        let path_str = path.display().to_string();
        written(tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
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
            "{} ({} x {})",
            tr.fmt(
                Key::CliCommonWrote,
                &[("path", &path.display().to_string())]
            ),
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
            "{}",
            tr.fmt(
                Key::CliOffsetsMaxHbSummary,
                &[
                    ("stations", &table.stations.len().to_string()),
                    ("waterlines", &table.waterlines_m.len().to_string()),
                    ("max_hb", &format!("{:.3}", table.max_half_breadth())),
                ]
            )
        );
        return Ok(());
    }

    println!("{}", title);
    println!("{}", tr.s(Key::CliOffsetsHalfBreadthsSub));
    print!("{:>8}", "x [m]");
    for wl in &table.waterlines_m {
        print!("{:>8}", format!("WL{wl:.2}"));
    }
    println!(
        "{:>8}{:>8}{:>8}",
        tr.s(Key::CliOffsetsColKeel),
        tr.s(Key::CliOffsetsColSheer),
        "B/2"
    );
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
    println!("{}", tr.s(Key::CliOffsetsButtocksSub));
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
        "{}",
        tr.fmt(
            Key::CliOffsetsFooter,
            &[("max_hb", &format!("{:.3}", table.max_half_breadth()))]
        )
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

fn cmd_transform(a: TransformArgs, tr: Tr) -> Result<()> {
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
        None => println!("  {label:<7} {}", tr.s(Key::CliTransformNoWaterline)),
    };
    println!("{}", s.settings.name);
    line(tr.s(Key::CliTransformBefore), before);
    line(tr.s(Key::CliTransformAfter), after);
    if let Some(f) = &fit {
        println!(
            "{}",
            tr.fmt(
                Key::CliTransformFit,
                &[
                    ("iters", &f.iterations.to_string()),
                    ("sx", &format!("{:.4}", f.scale.x)),
                    ("sy", &format!("{:.4}", f.scale.y)),
                    ("sz", &format!("{:.4}", f.scale.z)),
                    ("err", &format!("{:.3}", f.max_error * 100.0)),
                ]
            )
        );
    }
    let out_str = a.output.display().to_string();
    println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &out_str)]));
    Ok(())
}

fn cmd_resistance(a: ResistanceArgs, tr: Tr) -> Result<()> {
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

        let warnings = michell::michell_warnings(&grid, &rows);
        // Catamaran interference at each speed: R_w of the pair over twice one demihull.
        let interference: Vec<Option<f64>> = match a.cat_spacing {
            Some(sp) => rows
                .iter()
                .map(|r| {
                    michell::interference(
                        &grid,
                        r.speed_m_s,
                        opts.water_density,
                        resistance::GRAVITY,
                        sp,
                    )
                })
                .collect(),
            None => Vec::new(),
        };

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
            println!(
                "{}",
                tr.fmt(
                    Key::CliCommonWroteSpeeds,
                    &[
                        ("path", &path.display().to_string()),
                        ("count", &rows.len().to_string()),
                    ]
                )
            );
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
                "interference": a.cat_spacing.map(|_| &interference),
            });
            println!("{}", serde_json::to_string_pretty(&body)?);
            return Ok(());
        }
        if a.csv.is_some() {
            // The CSV has no place for warnings; say them on stderr.
            for w in &warnings {
                eprintln!(
                    "{}",
                    tr.fmt(Key::CliCommonWarning, &[("message", &w.render(tr))])
                );
            }
            return Ok(());
        }

        let cat_note = if let Some(sp) = a.cat_spacing {
            tr.fmt(
                Key::CliResistanceCatSpacing,
                &[("spacing", &format!("{sp:.2}"))],
            )
        } else {
            String::new()
        };
        println!(
            "{}",
            tr.fmt(
                Key::CliResistanceMichellHeading,
                &[
                    ("name", &s.settings.name),
                    ("draft", &format!("{:.3}", grid.draft)),
                    ("cat", &cat_note),
                ]
            )
        );
        println!(
            "  LWL {:.3}  BWL {:.3}  S {:.2} m2  L/B {:.2}  1+k {:.3}",
            grid.length_wl,
            grid.beam_wl,
            grid.wetted_surface_m2,
            grid.length_wl / grid.beam_wl,
            form_factor
        );
        for w in &warnings {
            println!(
                "{}",
                tr.fmt(Key::CliCommonWarningIndent, &[("message", &w.render(tr))])
            );
        }
        println!();
        let cat = a.cat_spacing.is_some();
        println!(
            "  {:>6}  {:>6}  {:>6}  {:>10}  {:>10}  {:>9}  {:>9}{}",
            "kn",
            "Fn",
            "1+k1",
            "RT [kN]",
            "PE [kW]",
            "RF [kN]",
            "RW [kN]",
            if cat {
                format!("  {:>7}", tr.s(Key::CliResistanceColInterf))
            } else {
                String::new()
            }
        );
        for (i, r) in rows.iter().enumerate() {
            let interf = match interference.get(i) {
                Some(Some(x)) => format!("  {x:>7.3}"),
                Some(None) => format!("  {:>7}", "-"),
                None => String::new(),
            };
            println!(
                "  {:>6.2}  {:>6.3}  {:>6.3}  {:>10.2}  {:>10.2}  {:>9.2}  {:>9.2}{}",
                r.speed_kn,
                r.fn_,
                r.form_factor,
                r.rt_n / 1000.0,
                r.pe_w / 1000.0,
                r.rf_n / 1000.0,
                r.rw_n / 1000.0,
                interf
            );
        }
        if cat {
            println!("{}", tr.s(Key::CliResistanceInterfNote));
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
                warnings.push(resistance::dsyhs_fn_warning(r.fn_, r.speed_kn));
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
            println!(
                "{}",
                tr.fmt(
                    Key::CliCommonWroteSpeeds,
                    &[
                        ("path", &path.display().to_string()),
                        ("count", &rows.len().to_string()),
                    ]
                )
            );
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
            // The CSV has no place for warnings; say them on stderr.
            for w in &warnings {
                eprintln!(
                    "{}",
                    tr.fmt(Key::CliCommonWarning, &[("message", &w.render(tr))])
                );
            }
            return Ok(());
        }

        println!(
            "{}",
            tr.fmt(
                Key::CliResistanceDsyhsHeading,
                &[
                    ("name", &s.settings.name),
                    ("draft", &format!("{:.3}", dsyhs_in.draft_canoe)),
                ]
            )
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
            println!(
                "{}",
                tr.fmt(Key::CliCommonWarningIndent, &[("message", &w.render(tr))])
            );
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
        println!(
            "{}",
            tr.fmt(
                Key::CliCommonWroteSpeeds,
                &[
                    ("path", &path.display().to_string()),
                    ("count", &rows.len().to_string()),
                ]
            )
        );
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
        // The CSV has no place for warnings; say them on stderr.
        for w in &warnings {
            let msg = tr.fmt(
                Key::CliResistanceWarningRange,
                &[("message", &w.render(tr))],
            );
            eprintln!("{}", tr.fmt(Key::CliCommonWarning, &[("message", &msg)]));
        }
        return Ok(());
    }

    println!(
        "{}",
        tr.fmt(
            Key::CliResistanceHoltropHeading,
            &[
                ("name", &s.settings.name),
                ("draft", &format!("{:.3}", input.draft)),
            ]
        )
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
        let msg = tr.fmt(
            Key::CliResistanceWarningRange,
            &[("message", &w.render(tr))],
        );
        println!(
            "{}",
            tr.fmt(Key::CliCommonWarningIndent, &[("message", &msg)])
        );
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
            "{}",
            tr.fmt(
                Key::CliResistancePropellerLine,
                &[
                    ("d", &format!("{:.2}", a.propeller_diameter)),
                    ("aea0", &format!("{:.2}", a.expanded_area)),
                    ("eta_o", &format!("{:.2}", a.eta_o)),
                    ("eta_r", &format!("{:.4}", propulsion[0].relative_rotative)),
                    ("eta_h", &format!("{:.4}", propulsion[0].hull_efficiency)),
                    ("shaft", &format!("{:.2}", a.shaft_efficiency)),
                ]
            )
        );
        if !propulsion[0].plausible {
            println!(
                "{}",
                tr.fmt(
                    Key::CliResistancePropulsionWarn,
                    &[
                        ("w", &format!("{:.3}", propulsion[0].wake_fraction)),
                        ("t", &format!("{:.3}", propulsion[0].thrust_deduction)),
                        ("lcb", &format!("{:+.2}", input.lcb_pct)),
                    ]
                )
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
            "{}",
            tr.fmt(
                Key::CliResistanceHighFnNote,
                &[
                    (
                        "low",
                        &rows.iter().filter(|r| r.fn_ <= 0.40).count().to_string()
                    ),
                    (
                        "mid",
                        &rows
                            .iter()
                            .filter(|r| r.fn_ > 0.40 && r.fn_ < 0.55)
                            .count()
                            .to_string()
                    ),
                    (
                        "high",
                        &rows.iter().filter(|r| r.fn_ >= 0.55).count().to_string()
                    ),
                ]
            )
        );
    }
    if with_propeller {
        println!("{}", tr.s(Key::CliResistancePowerChainNote));
    } else {
        println!("{}", tr.s(Key::CliResistancePeOnlyNote));
    }
    Ok(())
}

fn cmd_waves(a: WavesArgs, tr: Tr) -> Result<()> {
    use seacraft_core::wave_pattern::{self, CutLine, WavePatternOptions};
    // Refuse a bad output before any work, so that nothing is half written.
    let formats = a
        .output
        .iter()
        .map(|p| wave_format(p))
        .collect::<Result<Vec<_>>>()?;
    // Two outputs written to one file would leave only the last, while both are reported.
    let outputs: Vec<(&str, &Path)> = a
        .output
        .iter()
        .map(|p| ("-o", p.as_path()))
        .chain(a.cut_csv.as_deref().map(|p| ("--cut-csv", p)))
        .collect();
    let paths: Vec<&Path> = outputs.iter().map(|(_, p)| *p).collect();
    if let Some((j, i)) = seacraft_core::io::shared_output(&paths)? {
        bail!(
            "{} and {} name the same file, `{}`: each output needs a file of its own",
            outputs[i].0,
            outputs[j].0,
            outputs[i].1.display()
        );
    }
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
    };
    let draft = a
        .draft
        .unwrap_or_else(|| HydroOptions::from_surface(&s, subdiv).draft);
    let grid = michell::michell_grid_from_surface(&s, draft, 65, 25, subdiv)
        .context("cannot extract Michell offsets grid: positive draft and waterline needed")?;
    let speed = match (a.speed, a.fn_) {
        (Some(kn), _) => kn * resistance::KNOT,
        (None, Some(f)) => f * (resistance::GRAVITY * grid.length_wl).sqrt(),
        (None, None) => bail!("give --speed or --fn"),
    };
    let opts = WavePatternOptions {
        ahead_l: a.ahead,
        behind_l: a.behind,
        half_width_l: a.half_width,
        spacing_m: a.spacing,
        catamaran_spacing: a.cat_spacing,
        ..WavePatternOptions::at_speed(speed)
    };
    let field = wave_pattern::wave_field(&grid, &opts).map_err(anyhow::Error::msg)?;
    let cut = match a.cut_y {
        Some(y) => {
            let (x0, x1) = (field.xs[0], field.xs[field.xs.len() - 1]);
            let line = CutLine::Longitudinal { y };
            Some(
                wave_pattern::wave_cut(&grid, &opts, line, x0, x1, 2000)
                    .map_err(anyhow::Error::msg)?,
            )
        }
        None => None,
    };
    let wash = match a.wash_distance {
        Some(d) => {
            wave_pattern::wash_samples(&field, wave_pattern::WASH_SAMPLES)
                .and_then(|samples| wave_pattern::wash_cut(&grid, &opts, &field, d, samples))
                .map_err(|e| anyhow::anyhow!("--wash-distance: {e}"))?
                .1
        }
        None => None,
    };

    // Every output is made first and written together, so one that cannot be written leaves none.
    let mut writes: Vec<(&Path, Vec<u8>)> = Vec::new();
    for (path, format) in a.output.iter().zip(formats) {
        let bytes = match format {
            WaveFormat::Png => {
                wave_pattern::wave_png(&field, a.clip).context("empty wave field")?
            }
            WaveFormat::Svg => wave_pattern::wave_svg_tr(&field, a.clip, tr).into_bytes(),
            WaveFormat::Vti => wave_pattern::wave_vti(&field).into_bytes(),
        };
        writes.push((path, bytes));
    }
    if let (Some(cut), Some(path)) = (&cut, &a.cut_csv) {
        let mut text = String::from("x_m,zeta_m\n");
        for (x, z) in cut.s.iter().zip(&cut.zeta) {
            text.push_str(&format!("{x:.4},{z:.6e}\n"));
        }
        writes.push((path, text.into_bytes()));
    }
    seacraft_core::io::write_all_atomic(&writes).context("cannot write the outputs")?;
    let files: Vec<String> = writes
        .iter()
        .map(|(p, _)| p.display().to_string())
        .collect();

    for w in &field.warnings {
        eprintln!(
            "{}",
            tr.fmt(Key::CliCommonWarning, &[("message", &w.render(tr))])
        );
    }
    if a.json {
        let mut body = wave_pattern::summary_json(&field);
        if let Some(map) = body.as_object_mut() {
            map.insert("draft".into(), serde_json::json!(grid.draft));
            map.insert("cat_spacing".into(), serde_json::json!(a.cat_spacing));
            map.insert("files".into(), serde_json::json!(files));
            if a.wash_distance.is_some() {
                map.insert("wash".into(), serde_json::json!(wash));
            }
        }
        println!("{}", serde_json::to_string_pretty(&body)?);
        return Ok(());
    }
    let cat_note = a
        .cat_spacing
        .map(|sp| tr.fmt(Key::CliWavesCatSpacing, &[("spacing", &format!("{sp:.2}"))]))
        .unwrap_or_default();
    println!(
        "{}",
        tr.fmt(
            Key::CliWavesHeading,
            &[
                ("name", &s.settings.name),
                ("draft", &format!("{:.3}", grid.draft)),
                ("cat", &cat_note),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliWavesStatsLine,
            &[
                ("u", &format!("{:.3}", field.speed_m_s)),
                ("kn", &format!("{:.2}", field.speed_m_s / resistance::KNOT)),
                ("froude", &format!("{:.3}", field.froude)),
                ("k0", &format!("{:.4}", field.k0)),
                ("lambda_t", &format!("{:.3}", field.transverse_wavelength_m)),
            ]
        )
    );
    println!("  {}", wave_pattern::regime(field.froude).sentence_tr(tr));
    println!(
        "{}",
        tr.fmt(
            Key::CliWavesAnglesLine,
            &[
                ("theta", &format!("{:.1}", field.theta_cut_deg)),
                ("shortest", &format!("{:.3}", field.shortest_wavelength_m)),
                ("samples", &field.theta_samples.to_string()),
                ("nx", &field.xs.len().to_string()),
                ("ny", &field.ys.len().to_string()),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliWavesMaxElev,
            &[("val", &format!("{:.4}", field.max_abs_zeta_m))]
        )
    );
    if let Some(d) = a.wash_distance {
        match wash {
            Some(w) => println!(
                "{}",
                tr.fmt(
                    Key::CliWavesWashLine,
                    &[
                        ("d", &format!("{d:.2}")),
                        ("h", &format!("{:.3}", w.height_m)),
                        ("x", &format!("{:.2}", w.x_m)),
                    ]
                )
            ),
            None => println!(
                "{}",
                tr.fmt(Key::CliWavesWashCalm, &[("d", &format!("{d:.2}"))])
            ),
        }
    }
    for n in &field.notes {
        println!(
            "{}",
            tr.fmt(Key::CliWavesNoteLine, &[("note", &n.render(tr))])
        );
    }
    for f in &files {
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", f)]));
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

fn cmd_edit(a: EditArgs, tr: Tr) -> Result<()> {
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
            "{}",
            tr.fmt(
                Key::CliEditSplitLine,
                &[
                    ("face", &r.face.to_string()),
                    ("pieces", &r.pieces.to_string()),
                    ("point", &r.point.to_string()),
                ]
            )
        );
    }
    println!(
        "{}",
        tr.fmt(
            Key::CliEditSummaryLine,
            &[
                ("fb", &faces_before.to_string()),
                ("fa", &s.faces.len().to_string()),
                ("pb", &points_before.to_string()),
                ("pa", &s.points.len().to_string()),
                ("cb", &creases_before.to_string()),
                ("ca", &creases_after.to_string()),
            ]
        )
    );
    println!("{}", tr.s(Key::CliEditValidates));
    let out_str = a.output.display().to_string();
    println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &out_str)]));
    Ok(())
}

fn cmd_planing(a: PlaningArgs, tr: Tr) -> Result<()> {
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
            "speed_kn,speed_m_s,cv,c_delta,cl_beta,trim_deg,lambda,wetted_length_m,wetted_area_m2,lcp_over_b,cf,resistance_n,pe_w\n",
        );
        for p in &curve {
            text.push_str(&format!(
                "{:.2},{:.3},{:.4},{:.5},{:.5},{:.3},{:.4},{:.3},{:.3},{:.4},{:.6},{:.1},{:.1}\n",
                p.speed_kn,
                p.speed_m_s,
                p.cv,
                p.c_delta,
                p.cl_beta,
                p.trim_deg,
                p.lambda,
                p.wetted_length_m,
                p.wetted_area_m2,
                p.lcp_over_b,
                p.cf,
                p.resistance_n,
                p.pe_w
            ));
        }
        seacraft_core::io::write_atomic(path, text)
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!(
            "{}",
            tr.fmt(
                Key::CliCommonWroteSpeeds,
                &[
                    ("path", &path.display().to_string()),
                    ("count", &curve.len().to_string()),
                ]
            )
        );
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
        "{}",
        tr.fmt(
            Key::CliPlaningHeading,
            &[
                ("name", &s.settings.name),
                ("draft", &format!("{:.3}", opts.draft)),
            ]
        )
    );
    let measured_tag = if a.deadrise.is_none() && measured.is_some() {
        tr.s(Key::CliPlaningMeasuredTag)
    } else {
        ""
    };
    println!(
        "{}",
        tr.fmt(
            Key::CliPlaningSetupLine,
            &[
                ("weight", &format!("{:.2}", weight_n / 1000.0)),
                ("beam", &format!("{:.3}", beam)),
                ("lcg", &format!("{:.3}", lcg_from_transom)),
                ("lcg_b", &format!("{:.3}", lcg_from_transom / beam)),
                ("deadrise", &format!("{:.1}", deadrise)),
                ("measured", measured_tag),
            ]
        )
    );
    println!();
    println!(
        "  {:>6}  {:>6}  {:>7}  {:>7}  {:>10}  {:>10}  {:>7}  {:>10}  {:>10}",
        "kn",
        "CV",
        tr.s(Key::CliPlaningColTrim),
        "lambda",
        tr.s(Key::CliPlaningColWetted),
        tr.s(Key::CliPlaningColArea),
        "LCP/b",
        "RT [kN]",
        "PE [kW]"
    );
    for p in &curve {
        println!(
            "  {:>6.1}  {:>6.3}  {:>6.2}d  {:>7.3}  {:>10.3}  {:>10.2}  {:>7.3}  {:>10.2}  {:>10.2}",
            p.speed_kn,
            p.cv,
            p.trim_deg,
            p.lambda,
            p.wetted_length_m,
            p.wetted_area_m2,
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
            "{}",
            tr.fmt(Key::CliPlaningMissed, &[("speeds", &format!("{missed:?}"))])
        );
    }
    println!("{}", tr.s(Key::CliPlaningPeOnlyNote));
    Ok(())
}

fn cmd_unfold(a: UnfoldArgs, tr: Tr) -> Result<()> {
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
        let title = tr.fmt(
            Key::CliUnfoldSvgTitle,
            &[
                ("name", &s.settings.name),
                ("lo", &format!("{lo:.2}")),
                ("hi", &format!("{hi:.2}")),
                ("axis", &format!("{axis:?}")),
            ],
        );
        seacraft_core::io::write_atomic(path, develop::svg_tr(&unfolded, &title, tr))
            .with_context(|| format!("cannot write {}", path.display()))?;
        let path_str = path.display().to_string();
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&unfolded)?);
        return Ok(());
    }
    let axis_str = format!("{axis:?}");
    println!(
        "{}",
        tr.fmt(
            Key::CliUnfoldHeading,
            &[("name", &s.settings.name), ("axis", &axis_str)]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliUnfoldBandLine,
            &[
                ("lo", &format!("{lo:.3}")),
                ("hi", &format!("{hi:.3}")),
                ("axis", &axis_str),
                ("x_lo", &format!("{x_lo:.3}")),
                ("x_hi", &format!("{x_hi:.3}")),
                ("picked", &faces.len().to_string()),
                ("total", &hull.tris.len().to_string()),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliUnfoldPatternLine,
            &[
                ("w", &format!("{:.3}", unfolded.width_m)),
                ("h", &format!("{:.3}", unfolded.height_m)),
                ("a2d", &format!("{:.4}", unfolded.area_2d_m2)),
                ("a3d", &format!("{:.4}", unfolded.area_3d_m2)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliUnfoldGapLine,
            &[
                ("max_mm", &format!("{:.2}", unfolded.max_gap_m * 1000.0)),
                ("rms_mm", &format!("{:.2}", unfolded.rms_gap_m * 1000.0)),
                ("pct", &format!("{:.3}", unfolded.gap_ratio * 100.0)),
            ]
        )
    );
    let verdict = if unfolded.developable {
        tr.s(Key::CliUnfoldDevelopableVerdict)
    } else {
        tr.s(Key::CliUnfoldNotDevelopableVerdict)
    };
    println!(
        "{}",
        tr.fmt(
            Key::CliUnfoldMeanKLine,
            &[
                ("k", &format!("{:.4}", unfolded.mean_abs_gaussian)),
                ("verdict", verdict),
            ],
        )
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

fn cmd_templates(a: TemplatesArgs, tr: Tr) -> Result<()> {
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
        locale: tr.locale(),
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
        locale: tr.locale(),
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
        tr.s(Key::CliTemplatesColSheet),
        "x [m]",
        tr.s(Key::CliTemplatesColHb),
        tr.s(Key::CliTemplatesColFace),
        tr.s(Key::CliTemplatesColBevel)
    );
    for row in &set.rows {
        let x = row.x.map_or_else(|| "-".to_owned(), |x| format!("{x:.3}"));
        let face = match row.face {
            Some(MouldFace::Forward) => tr.s(Key::CliTemplatesFaceForward),
            Some(MouldFace::Aft) => tr.s(Key::CliTemplatesFaceAft),
            Some(MouldFace::Either) => tr.s(Key::CliTemplatesFaceEither),
            None => "-",
        };
        println!(
            "{:<12} {:>8} {:>18.0} {:>8} {:>11.1}",
            row.sheet.render(tr),
            x,
            row.half_breadth_m * 1000.0,
            face,
            row.max_bevel_deg
        );
    }
    let scale_label = units::scale_label(scale);
    match (format, written.pages) {
        (Format::PdfTiled, Some(pages)) => println!(
            "{}",
            tr.fmt(
                Key::CliTemplatesPagesTiled,
                &[
                    ("pages", &pages.to_string()),
                    ("paper", &paper_name(paper)),
                    ("scale", &scale_label),
                ]
            )
        ),
        (_, Some(pages)) => println!(
            "{}",
            tr.fmt(
                Key::CliTemplatesPagesSingle,
                &[("pages", &pages.to_string()), ("scale", &scale_label)]
            )
        ),
        (_, None) => println!(
            "{}",
            tr.fmt(Key::CliTemplatesOneDrawing, &[("scale", &scale_label)])
        ),
    }
    if a.skin_mm > 0.0 {
        println!(
            "{}",
            tr.fmt(
                Key::CliTemplatesSkinDeducted,
                &[("mm", &a.skin_mm.to_string())]
            )
        );
    } else {
        println!("{}", tr.s(Key::CliTemplatesNoSkin));
    }
    for warning in &set.warnings {
        eprintln!(
            "{}",
            tr.fmt(Key::CliCommonWarning, &[("message", &warning.render(tr))])
        );
    }
    if !missing.is_empty() {
        println!(
            "{}",
            tr.fmt(
                Key::CliTemplatesMissingChars,
                &[("chars", &missing.join(" "))]
            )
        );
    }
    let out_str = a.output.display().to_string();
    println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &out_str)]));
    Ok(())
}

fn cmd_panels(a: PanelsArgs, tr: Tr) -> Result<()> {
    use seacraft_core::panel_choices;
    use seacraft_core::panels::{self, PanelOptions};
    use seacraft_core::sheet::{PageSetup, Paper};
    use seacraft_core::templates::{Format, StationChoice};
    use seacraft_core::units;

    // Everything the user typed is checked before the hull is refined and flattened.
    let extension = a
        .output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let given: Vec<&str> = [
        (a.paper.is_some(), "--paper"),
        (a.single_page, "--single-page"),
        (a.margin_mm.is_some(), "--margin-mm"),
        (a.overlap_mm.is_some(), "--overlap-mm"),
    ]
    .into_iter()
    .filter_map(|(given, name)| given.then_some(name))
    .collect();
    panel_choices::pdf_only(&extension, &given).map_err(anyhow::Error::msg)?;
    let format = Format::from_path(&a.output, a.single_page).map_err(anyhow::Error::msg)?;
    if let Some(p) = &a.offsets_pdf {
        let ext = p
            .extension()
            .map(|e| e.to_string_lossy().to_ascii_lowercase());
        if ext.as_deref() != Some("pdf") {
            bail!("--offsets-pdf writes a .pdf, not `{}`", p.display());
        }
    }
    // Two outputs written to one file would leave only the last, while both are reported.
    let outputs: Vec<(&str, &Path)> = [
        ("-o", Some(a.output.as_path())),
        ("--offsets-csv", a.offsets_csv.as_deref()),
        ("--offsets-pdf", a.offsets_pdf.as_deref()),
    ]
    .into_iter()
    .filter_map(|(flag, path)| path.map(|p| (flag, p)))
    .collect();
    let paths: Vec<&Path> = outputs.iter().map(|(_, p)| *p).collect();
    if let Some((j, i)) = seacraft_core::io::shared_output(&paths)? {
        bail!(
            "{} and {} name the same file, `{}`: each output needs a file of its own",
            outputs[i].0,
            outputs[j].0,
            outputs[i].1.display()
        );
    }
    let checked = panel_choices::check(&panel_choices::Choices {
        max_seam_drift_mm: a.max_seam_drift_mm,
        max_stretch_pct: a.max_stretch_pct,
        offset_spacing_mm: a.offset_spacing_mm,
        stock_mm: a.stock_mm.clone(),
        scarf_mm: a.scarf_mm,
        nest: a.nest.clone(),
        gap_mm: a.gap_mm,
    })
    .map_err(|e| {
        let flag = match e.field {
            panel_choices::Field::MaxSeamDrift => "--max-seam-drift-mm",
            panel_choices::Field::MaxStretch => "--max-stretch-pct",
            panel_choices::Field::OffsetSpacing => "--offset-spacing-mm",
            panel_choices::Field::Stock => "--stock-mm",
            panel_choices::Field::Scarf => "--scarf-mm",
            panel_choices::Field::Nest => "--nest",
            panel_choices::Field::Gap => "--gap-mm",
        };
        anyhow::anyhow!("{flag} {}", e.message)
    })?;
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
        locale: tr.locale(),
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
        limits: checked.limits,
        offset_spacing_m: checked.offset_spacing_m,
        stock: checked.stock,
        imperial: a.imperial,
        nesting: checked.nesting,
        locale: tr.locale(),
    };
    let set = panels::panel_set(&s, &opts).map_err(anyhow::Error::msg)?;

    // A panel that does not lie flat stops the export unless the user asked for it anyway.
    if let (Some(summary), false) = (set.not_flat_summary_tr(tr), a.allow_undevelopable) {
        for f in set.failing_tr(tr) {
            eprintln!("{f}");
        }
        bail!("{summary}; nothing written. Pass --allow-undevelopable to write the others anyway");
    }

    // Every output is made first and written together, so one that cannot be written leaves none.
    let panels::PanelsWritten {
        written,
        offsets_pages,
        layout_pages,
        per_sheet,
    } = panels::write(&set, format, &setup, !a.no_offsets_pages).map_err(anyhow::Error::msg)?;
    let sheet_paths: Vec<PathBuf> = (0..per_sheet.len())
        .map(|i| panels::sheet_file_name(&a.output, i))
        .collect();
    let mut named_outputs: Vec<(String, &Path)> = vec![("-o".to_owned(), a.output.as_path())];
    let mut writes: Vec<(&Path, Vec<u8>)> = vec![(&a.output, written.bytes)];
    for (i, (path, sheet)) in sheet_paths.iter().zip(per_sheet).enumerate() {
        named_outputs.push((format!("sheet {}", i + 1), path.as_path()));
        writes.push((path.as_path(), sheet.bytes));
    }
    if let Some(path) = &a.offsets_csv {
        named_outputs.push(("--offsets-csv".to_owned(), path.as_path()));
        writes.push((path, panels::offsets_csv(&set.offsets).into_bytes()));
    }
    if let Some(path) = &a.offsets_pdf {
        named_outputs.push(("--offsets-pdf".to_owned(), path.as_path()));
        let doc = panels::offsets_pdf_tr(&set.offsets, &set.title, a.imperial, tr)
            .map_err(anyhow::Error::msg)?;
        writes.push((path, doc.bytes));
    }
    let write_paths: Vec<&Path> = named_outputs.iter().map(|(_, p)| *p).collect();
    if let Some((j, i)) = seacraft_core::io::shared_output(&write_paths)? {
        bail!(
            "{} and {} name the same file, `{}`: each output needs a file of its own",
            named_outputs[i].0,
            named_outputs[j].0,
            named_outputs[i].1.display()
        );
    }
    seacraft_core::io::write_all_atomic(&writes).context("cannot write the outputs")?;
    // Sheet files an earlier, larger layout left beside these: named, never deleted.
    let stale = panels::stale_sheet_files(&a.output, sheet_paths.len());
    if let Some(note) = panels::stale_sheet_note_tr(&stale, tr) {
        eprintln!("{}", tr.fmt(Key::CliCommonWarning, &[("message", &note)]));
    }

    let missing: Vec<String> = written.missing.iter().map(char::to_string).collect();
    if a.json {
        let stale_files: Vec<String> = stale.iter().map(|p| p.display().to_string()).collect();
        let sheet_files: Vec<String> = sheet_paths
            .iter()
            .map(|p| p.display().to_string())
            .collect();
        let v = serde_json::json!({
            "path": a.output.display().to_string(),
            "format": format.name(),
            "pages": written.pages,
            "layout_pages": layout_pages,
            "offsets_pages": offsets_pages,
            "scale": units::scale_label(scale),
            "panels": set.rows,
            "mismatches": set.mismatches,
            "refused": set.refused,
            "total_sheets": set.total_sheets,
            "layout": set.layout,
            "sheet_files": sheet_files,
            "stale_sheet_files": stale_files,
            "missing_characters": missing,
        });
        println!("{}", serde_json::to_string_pretty(&v)?);
        return Ok(());
    }

    println!(
        "{:<16} {:>6} {:>10} {:>7} {:>9} {:>7}",
        tr.s(Key::CliPanelsColPanel),
        tr.s(Key::CliPanelsColCopies),
        tr.s(Key::CliPanelsColLiesFlat),
        tr.s(Key::CliPanelsColRms),
        tr.s(Key::CliPanelsColDrift),
        tr.s(Key::CliPanelsColSheets)
    );
    for r in &set.rows {
        println!(
            "{:<16} {:>6} {:>10} {:>7.3} {:>9.1} {:>7}",
            r.panel,
            r.copies,
            if r.lies_flat {
                tr.s(Key::CliCommonYes)
            } else {
                tr.s(Key::CliCommonNo)
            },
            r.rms_stretch_pct,
            r.max_seam_drift_mm,
            r.sheets_each
                .map_or_else(|| "-".to_owned(), |n| n.to_string())
        );
    }
    for r in &set.rows {
        if let Some(mm) = r.unsettled_mm {
            println!(
                "{}: {}",
                r.panel,
                panels::unsettled_note_tr(mm / 1000.0, tr)
            );
        }
    }
    for r in &set.refused {
        println!(
            "{}",
            tr.fmt(
                Key::CliPanelsRefusedLine,
                &[
                    ("panel", &format!("{:<16}", r.panel)),
                    ("reason", &r.reason.render(tr)),
                ]
            )
        );
    }
    for m in &set.mismatches {
        println!(
            "{}",
            tr.fmt(
                Key::CliPanelsMismatchLine,
                &[
                    ("a", &m.a),
                    ("b", &m.b),
                    ("mm", &format!("{:.1}", m.max_m * 1000.0)),
                ]
            )
        );
    }
    println!("{}", set.sheet_count_line_tr(tr));
    for row in &set.stock {
        if let Some(note) = &row.note {
            println!("{}: {}", row.panel, note.render(tr));
        }
    }
    let headed = set.rows.iter().filter(|r| !r.lies_flat).count();
    match headed {
        0 => {}
        1 => println!("{}", tr.s(Key::CliPanelsHeadedOne)),
        n => println!(
            "{}",
            tr.fmt(Key::CliPanelsHeadedOther, &[("count", &n.to_string())])
        ),
    }
    let refused: Vec<&str> = set.refused.iter().map(|r| r.panel.as_str()).collect();
    match refused.len() {
        0 => {}
        1 => println!(
            "{}",
            tr.fmt(Key::CliPanelsNotLaidOne, &[("names", refused[0])])
        ),
        n => println!(
            "{}",
            tr.fmt(
                Key::CliPanelsNotLaidOther,
                &[("count", &n.to_string()), ("names", &refused.join(", ")),]
            )
        ),
    }
    let scale_label = units::scale_label(scale);
    let mut page_parts = Vec::new();
    match layout_pages {
        0 => {}
        1 => page_parts.push(tr.s(Key::CliPanelsLayoutOne).to_owned()),
        n => page_parts.push(tr.fmt(Key::CliPanelsLayoutOther, &[("count", &n.to_string())])),
    }
    match offsets_pages {
        0 => {}
        1 => page_parts.push(tr.s(Key::CliPanelsOffsetsOne).to_owned()),
        n => page_parts.push(tr.fmt(Key::CliPanelsOffsetsOther, &[("count", &n.to_string())])),
    }
    let tables = if page_parts.is_empty() {
        String::new()
    } else {
        format!(" ({})", page_parts.join(", "))
    };
    match (format, written.pages) {
        (Format::PdfTiled, Some(pages)) => println!(
            "{}",
            tr.fmt(
                Key::CliPanelsPagesTiled,
                &[
                    ("pages", &pages.to_string()),
                    ("tables", &tables),
                    ("paper", &paper_name(paper)),
                    ("scale", &scale_label),
                ]
            )
        ),
        (_, Some(pages)) => println!(
            "{}",
            tr.fmt(
                Key::CliPanelsPagesSingle,
                &[
                    ("pages", &pages.to_string()),
                    ("tables", &tables),
                    ("scale", &scale_label),
                ]
            )
        ),
        (_, None) if sheet_paths.is_empty() => println!(
            "{}",
            tr.fmt(Key::CliPanelsOneDrawing, &[("scale", &scale_label)])
        ),
        (_, None) => {
            let key = if sheet_paths.len() == 1 {
                Key::CliPanelsOneDrawingSheetsOne
            } else {
                Key::CliPanelsOneDrawingSheetsOther
            };
            println!(
                "{}",
                tr.fmt(
                    key,
                    &[
                        ("scale", &scale_label),
                        ("count", &sheet_paths.len().to_string()),
                    ]
                )
            );
        }
    }
    if !missing.is_empty() {
        println!(
            "{}",
            tr.fmt(
                Key::CliTemplatesMissingChars,
                &[("chars", &missing.join(" "))]
            )
        );
    }
    let out_str = a.output.display().to_string();
    println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &out_str)]));
    for path in &sheet_paths {
        let path_str = path.display().to_string();
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
    }
    for path in [&a.offsets_csv, &a.offsets_pdf].into_iter().flatten() {
        let path_str = path.display().to_string();
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
    }
    Ok(())
}

fn cmd_linesplan(a: LinesplanArgs, tr: Tr) -> Result<()> {
    let s = load(&a.file)?;
    let opts = linesplan::LinesplanOptions {
        stations: a.stations,
        buttocks: a.buttocks,
        waterlines: a.waterlines,
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
        combs: a.combs,
        comb_scale: a.comb_scale,
        locale: tr.locale(),
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
    let out_str = a.output.display().to_string();
    println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &out_str)]));
    Ok(())
}

fn cmd_report(a: ReportArgs, tr: Tr) -> Result<()> {
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
    opts.locale = tr.locale();
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
    let out_str = a.output.display().to_string();
    println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &out_str)]));
    Ok(())
}

fn cmd_render(a: RenderArgs, tr: Tr) -> Result<()> {
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
        "{}",
        tr.fmt(
            Key::CliRenderWrote,
            &[
                ("path", &a.output.display().to_string()),
                ("w", &width.to_string()),
                ("h", &height.to_string()),
                ("mode", mode.label()),
                ("faces", &mesh.faces.len().to_string()),
            ]
        )
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

fn cmd_stability(a: StabilityArgs, tr: Tr) -> Result<()> {
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
        let path_str = path.display().to_string();
        println!("{}", tr.fmt(Key::CliCommonWrote, &[("path", &path_str)]));
        return Ok(());
    }

    println!(
        "{}",
        tr.fmt(
            Key::CliStabilityHeading,
            &[("name", &s.settings.name), ("kg", &format!("{:.3}", a.kg))]
        )
    );
    match lcg {
        Some(x) => println!(
            "{}",
            tr.fmt(Key::CliStabilityFreeTrim, &[("lcg", &format!("{:.3}", x))])
        ),
        None => println!(
            "{}",
            tr.fmt(
                Key::CliStabilityFixedTrim,
                &[("trim", &format!("{:+.3}", a.trim))]
            )
        ),
    }
    print!("  {:>4} |", tr.s(Key::CliStabilityColHeel));
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
    println!("{}", tr.s(Key::CliStabilitySolvedHeading));
    let deg_unit = tr.s(Key::CliStabilityDeg);
    for c in &curves {
        let solved: Vec<String> = c
            .points
            .iter()
            .map(|p| match (p.draft, p.trim_m) {
                (Some(d), Some(t)) => format!("{:.3} m / {:+.3} m", d, t),
                _ => "—".to_string(),
            })
            .collect();
        println!(
            "    {:>4.0} {}: {}",
            c.heel_deg,
            deg_unit,
            solved.join(", ")
        );
    }
    let criteria: Vec<_> = (0..displacements.len())
        .filter_map(|i| evaluate_imo_criteria(&curves, i))
        .collect();
    if !criteria.is_empty() {
        println!("{}", tr.s(Key::CliStabilityImoHeading));
        println!("  {}", seacraft_core::not_validated_note(tr.locale()));
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  {}",
            tr.s(Key::CliStabilityColDisp),
            "A(0-30)",
            "A(0-40)",
            "A(30-40)",
            "GZ(>=30)",
            "phi_max",
            "GM0",
            tr.s(Key::CliStabilityColStatus)
        );
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  ------",
            tr.s(Key::CliStabilityMinReq),
            ">=0.055",
            ">=0.090",
            ">=0.030",
            ">=0.200 m",
            ">=25 deg",
            ">=0.15"
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
                if c.passed {
                    tr.s(Key::ReportBadgePass)
                } else {
                    tr.s(Key::ReportBadgeFail)
                }
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
                        "{}",
                        tr.fmt(
                            Key::CliStabilityWindageFrom,
                            &[(
                                "path",
                                &project::sidecar_path(&a.file).display().to_string()
                            )]
                        )
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
        print_weather_summary(&w, tr);
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

/// The loading condition `case` (or the model's sidecar) holds: a bare
/// condition, or the one called `name` in a project (the first when `None`),
/// validated. When `reject_bare_name` is true, passing `name` for a bare case
/// file returns an error rather than ignoring it.
fn read_loading_condition_with_mode(
    model: &Path,
    case: Option<&Path>,
    name: Option<&str>,
    reject_bare_name: bool,
) -> Result<loading::LoadingCondition> {
    let (cond, from) = match read_case_or_project(model, case)? {
        (CaseSource::Bare(text), path) => {
            if reject_bare_name && name.is_some() {
                bail!(
                    "{}: --condition cannot be used with a standalone loading condition file",
                    path.display()
                );
            }
            (
                serde_json::from_str::<loading::LoadingCondition>(&text)
                    .with_context(|| format!("cannot parse {}", path.display()))?,
                path,
            )
        }
        (CaseSource::Project(p), path) => {
            let c = p
                .loading_condition(name)
                .map_err(|e| anyhow::anyhow!("{}: {e}", path.display()))?;
            (p.with_windage(c), path)
        }
    };
    loading::validate(&cond).map_err(|e| anyhow::anyhow!("{}: {e}", from.display()))?;
    Ok(cond)
}

fn read_loading_condition(
    model: &Path,
    case: Option<&Path>,
    name: Option<&str>,
) -> Result<loading::LoadingCondition> {
    read_loading_condition_with_mode(model, case, name, false)
}

fn cmd_heel(a: HeelArgs, tr: Tr) -> Result<()> {
    use seacraft_core::heel;
    let s = load(&a.file)?;
    let mut opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );
    let weights = a
        .weights
        .iter()
        .enumerate()
        .map(|(i, w)| {
            heel::parse_weight(w).map(|mut w| {
                // English in `--json` whatever `--lang`; the rows below print it translated.
                w.name = format!("Weight {}", i + 1);
                w
            })
        })
        .collect::<Result<Vec<_>, _>>()
        .map_err(|e| anyhow::anyhow!("--weight: {e}"))?;
    if let Some(h) = a.heel {
        if !(h.is_finite() && h.abs() <= 90.0) {
            bail!("--heel must be within -90..90 degrees");
        }
    }
    let hull = hydro::Hull::new(&s, opts.subdivision);
    let mut warnings = Vec::new();
    let start = if a.condition.is_some() || a.case.is_some() {
        let cond = read_loading_condition_with_mode(
            &a.file,
            a.case.as_deref(),
            a.condition.as_deref(),
            true,
        )?;
        let res = loading::evaluate(&s, &opts, &cond);
        if !res.equilibrium_found {
            warnings.extend(res.warnings.iter().cloned());
        }
        heel::MassCondition::from_loading(&res, hull.keel)
    } else {
        let Some(kg) = a.kg else {
            bail!("give --kg or --condition (a loading condition in the project file)");
        };
        if !(kg.is_finite() && kg >= 0.0) {
            bail!("--kg must be a non-negative number of metres");
        }
        if let Some(d) = a.draft {
            if !(d.is_finite() && d > 0.0) {
                bail!("--draft must be positive");
            }
            opts.draft = d;
        }
        if let Some(x) = a.lcg {
            if !x.is_finite() {
                bail!("--lcg must be a finite number");
            }
        }
        heel::MassCondition::bare(&hull, &opts, kg, a.lcg)
            .with_context(|| format!("the hull displaces nothing at draft {:.3} m", opts.draft))?
    };
    let mut r = heel::report(&hull, &opts, &start, &weights, a.heel);
    warnings.extend(r.warnings);
    r.warnings = warnings;
    if a.json {
        println!("{}", serde_json::to_string_pretty(&r)?);
        return Ok(());
    }
    println!(
        "{}",
        tr.fmt(Key::CliHeelHeading, &[("name", &s.settings.name)])
    );
    for w in &r.warnings {
        println!(
            "{}",
            tr.fmt(Key::CliHeelWarning, &[("warning", &w.render(tr))])
        );
    }
    let cond = |c: &heel::ConditionSummary| {
        tr.fmt(
            Key::CliHeelCondFmt,
            &[
                ("disp", &format!("{:.3}", c.displacement_t)),
                ("kg", &format!("{:.3}", c.kg_m)),
                ("lcg", &format!("{:.3}", c.lcg_m)),
                ("tcg", &format!("{:+.3}", c.tcg_m)),
            ],
        )
    };
    let row = |label: &str, value: String| println!("  {label:<14} {value}");
    row(tr.s(Key::CliHeelRowStart), cond(&r.start));
    for (i, w) in r.weights.iter().enumerate() {
        let name = tr.fmt(Key::CliHeelWeightName, &[("n", &(i + 1).to_string())]);
        row(
            tr.s(Key::CliHeelRowWeight),
            tr.fmt(
                Key::CliHeelWeightVal,
                &[
                    ("name", &name),
                    ("mass", &format!("{:.3}", w.mass_t)),
                    ("x", &format!("{:.3}", w.x)),
                    ("y", &format!("{:+.3}", w.y)),
                    ("z", &format!("{:.3}", w.z)),
                ],
            ),
        );
    }
    if !r.weights.is_empty() {
        row(tr.s(Key::CliHeelRowWithWeights), cond(&r.condition));
    }
    row(
        tr.s(Key::CliHeelRowSettles),
        r.equilibrium.describe_tr(r.condition.displacement_t, tr),
    );
    if let heel::Equilibrium::Loll { port, starboard } = &r.equilibrium {
        row(
            "",
            tr.fmt(
                Key::CliHeelLollNegGm,
                &[
                    ("port", &format!("{:+.1}", port.heel_deg)),
                    ("stbd", &format!("{:+.1}", starboard.heel_deg)),
                ],
            ),
        );
    }
    if let Some(h) = a.heel {
        let label = tr.fmt(Key::CliHeelAtHeelLabel, &[("deg", &format!("{h:+.1}"))]);
        match &r.at_heel {
            Some(st) => {
                let extra = match (st.draft, st.trim_m) {
                    (Some(d), Some(t)) => tr.fmt(
                        Key::CliHeelAtHeelDraftTrim,
                        &[("draft", &format!("{d:.3}")), ("trim", &format!("{t:+.3}"))],
                    ),
                    _ => String::new(),
                };
                row(
                    &label,
                    tr.fmt(
                        Key::CliHeelAtHeelLever,
                        &[
                            ("lever", &format!("{:+.3}", st.lever_m)),
                            ("gz", &format!("{:+.3}", st.gz_m)),
                            ("ha", &format!("{:+.3}", st.heeling_arm_m)),
                            ("extra", &extra),
                        ],
                    ),
                );
            }
            None => row(
                &label,
                if matches!(r.equilibrium, heel::Equilibrium::CannotFloat) {
                    tr.s(Key::CliHeelDoesNotFloat).into()
                } else {
                    tr.s(Key::CliHeelNoFreeTrim).into()
                },
            ),
        }
    }
    Ok(())
}

fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_owned()
    }
}

/// One CSV row per layer, seam and plywood line. Tape resin rides with the tape column.
fn materials_csv(est: &seacraft_core::materials::Estimate) -> String {
    let mut out = String::from(
        "kind,name,length_m,area_m2,shell_kg,glass_kg,resin_kg,tape_kg,fillet_kg,total_kg,x_m,y_m,z_m,sheets\n",
    );
    for r in &est.layers {
        out.push_str(&format!(
            "layer,{},,{},{},{},{},,,{},{},{},{},\n",
            csv_field(&r.layer),
            r.area_m2,
            r.shell_kg,
            r.glass_kg,
            r.resin_kg,
            r.total_kg,
            r.centre[0],
            r.centre[1],
            r.centre[2]
        ));
    }
    for r in &est.seams {
        out.push_str(&format!(
            "seam,{},{},,,,,{},{},{},{},{},{},\n",
            csv_field(&format!("{} / {}", r.between[0], r.between[1])),
            r.length_m,
            r.tape_kg + r.tape_resin_kg,
            r.fillet_kg,
            r.total_kg,
            r.centre[0],
            r.centre[1],
            r.centre[2]
        ));
    }
    for r in &est.plywood {
        out.push_str(&format!(
            "plywood,{},,,,,,,,,,,,{}\n",
            csv_field(&r.layer),
            r.sheets
        ));
    }
    out
}

fn print_materials_table(name: &str, est: &seacraft_core::materials::Estimate, tr: Tr) {
    println!("{}", tr.fmt(Key::CliMaterialsHeading, &[("name", name)]));
    println!();
    println!(
        "  {:<18} {:>11} {:>11} {:>11} {:>11} {:>11}",
        tr.s(Key::CliMaterialsColLayer),
        tr.s(Key::CliMaterialsColArea),
        tr.s(Key::CliMaterialsColShell),
        tr.s(Key::CliMaterialsColGlass),
        tr.s(Key::CliMaterialsColResin),
        tr.s(Key::CliMaterialsColTotal)
    );
    for r in &est.layers {
        println!(
            "  {:<18} {:>11.3} {:>11.2} {:>11.2} {:>11.2} {:>11.2}",
            r.layer, r.area_m2, r.shell_kg, r.glass_kg, r.resin_kg, r.total_kg
        );
    }
    if !est.seams.is_empty() {
        println!();
        println!("{}", tr.s(Key::CliMaterialsSeamsHeading));
        println!(
            "  {:<24} {:>11} {:>11} {:>11} {:>11}",
            tr.s(Key::CliMaterialsColBetween),
            tr.s(Key::CliMaterialsColLength),
            tr.s(Key::CliMaterialsColTape),
            tr.s(Key::CliMaterialsColFillet),
            tr.s(Key::CliMaterialsColTotal)
        );
        for r in &est.seams {
            println!(
                "  {:<24} {:>11.2} {:>11.2} {:>11.2} {:>11.2}",
                format!("{} / {}", r.between[0], r.between[1]),
                r.length_m,
                r.tape_kg + r.tape_resin_kg,
                r.fillet_kg,
                r.total_kg
            );
        }
    }
    if !est.plywood.is_empty() {
        println!();
        println!("{}", tr.s(Key::CliMaterialsPlywoodHeading));
        for r in &est.plywood {
            let sheets = tr.fmt(
                Key::CliMaterialsPlywoodSheets,
                &[
                    ("sheets", &r.sheets.to_string()),
                    ("thickness", &format!("{:.1}", r.thickness_mm)),
                ],
            );
            let note = if !r.not_flat.is_empty() {
                tr.fmt(
                    Key::CliMaterialsPlywoodNotFlat,
                    &[("panels", &r.not_flat.join(", "))],
                )
            } else if !r.refused.is_empty() {
                tr.fmt(
                    Key::CliMaterialsPlywoodRefused,
                    &[("panels", &r.refused.join(", "))],
                )
            } else if r.all_lie_flat {
                tr.s(Key::CliMaterialsPlywoodFlat).to_owned()
            } else {
                String::new()
            };
            println!("  {:<18} {}  {}", r.layer, sheets, note);
        }
    }
    println!();
    let [x, y, z] = est.total.centre;
    println!(
        "{}: {}",
        tr.s(Key::CliMaterialsTotal),
        tr.fmt(
            Key::CliMaterialsTotalLine,
            &[
                ("kg", &format!("{:.1}", est.total.mass_kg)),
                ("t", &format!("{:.3}", est.total.mass_t)),
                ("x", &format!("{x:.3}")),
                ("y", &format!("{y:.3}")),
                ("z", &format!("{z:.3}")),
            ]
        )
    );
    if !est.has_glass {
        println!("{}", tr.s(Key::CliMaterialsNoGlass));
    }
    for w in &est.warnings {
        println!(
            "{}",
            tr.fmt(Key::CliCommonWarning, &[("message", &w.render(tr))])
        );
    }
    println!("{}", tr.s(Key::CliMaterialsNote));
    println!("{}", seacraft_core::not_validated_note(tr.locale()));
}

fn cmd_materials(a: MaterialsArgs, tr: Tr) -> Result<()> {
    let s = load(&a.file)?;
    let sidecar = project::sidecar_path(&a.file);
    let mut loaded: Option<(project::Project, PathBuf)> = None;
    if a.case.is_some() || sidecar.exists() {
        match read_case_or_project(&a.file, a.case.as_deref())? {
            (CaseSource::Project(p), path) => loaded = Some((*p, path)),
            (CaseSource::Bare(_), path) => {
                if a.add_to.is_some() {
                    bail!(
                        "{}",
                        tr.fmt(
                            Key::CliMaterialsAddToBare,
                            &[("path", &path.display().to_string())]
                        )
                    );
                }
            }
        }
    } else if a.add_to.is_some() {
        bail!(
            "{}",
            tr.fmt(
                Key::CliMaterialsAddToNoProject,
                &[("path", &sidecar.display().to_string())]
            )
        );
    }
    let schedule = loaded
        .as_ref()
        .and_then(|(p, _)| p.materials.clone())
        .unwrap_or_default();
    schedule.validate().map_err(|e| {
        anyhow::anyhow!(tr.fmt(Key::CliMaterialsScheduleInvalid, &[("reason", e.as_str())]))
    })?;
    let est = seacraft_core::materials::estimate(
        &s,
        &schedule,
        SubdivisionOptions {
            level: a.subdiv.level,
            mode: a.subdiv.mode.into(),
        },
    );

    // The project is written before anything prints, so an unknown condition leaves no output.
    let mut wrote = None;
    if let Some(cond_name) = &a.add_to {
        let (mut proj, path) = loaded.expect("add-to without a project was refused above");
        let names: Vec<String> = proj
            .loading_conditions
            .iter()
            .map(|c| c.name.clone())
            .collect();
        let Some(cond) = proj
            .loading_conditions
            .iter_mut()
            .find(|c| &c.name == cond_name)
        else {
            bail!(
                "{}",
                tr.fmt(
                    Key::CliMaterialsAddToUnknown,
                    &[
                        ("name", cond_name),
                        ("path", &path.display().to_string()),
                        ("names", &names.join(", ")),
                    ]
                )
            );
        };
        let item_name = a
            .item
            .clone()
            .unwrap_or_else(|| seacraft_core::materials::default_item_name(tr.locale()).to_owned());
        let also = seacraft_core::materials::replaced_names(&item_name);
        loading::put_item(cond, est.weight_item(&item_name), &also);
        proj.save(&path).map_err(|e| anyhow::anyhow!(e))?;
        wrote = Some(tr.fmt(
            Key::CliMaterialsWrote,
            &[
                ("path", &path.display().to_string()),
                ("item", &item_name),
                ("mass", &format!("{:.3}", est.total.mass_t)),
                ("condition", cond_name),
            ],
        ));
    }

    if a.json {
        println!("{}", serde_json::to_string_pretty(&est)?);
    } else if a.csv {
        print!("{}", materials_csv(&est));
    } else {
        print_materials_table(&s.settings.name, &est, tr);
    }
    if let Some(line) = wrote {
        if a.json || a.csv {
            eprintln!("{line}");
        } else {
            println!("{line}");
        }
    }
    Ok(())
}

fn cmd_loadcase(a: LoadcaseArgs, tr: Tr) -> Result<()> {
    let s = load(&a.file)?;
    let cond = read_loading_condition(&a.file, a.case.as_deref(), a.name.as_deref())?;

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

    println!(
        "{}",
        tr.fmt(
            Key::CliLoadcaseHeading,
            &[("name", &s.settings.name), ("cond", &res.name)]
        )
    );
    for line in loading_equilibrium_lines_tr(&res, tr) {
        println!("  {line}");
    }
    println!();
    println!(
        "  {:<34} {:<10} {:>9} {:>9} {:>9} {:>9} {:>10}",
        tr.s(Key::CliLoadcaseColItem),
        tr.s(Key::CliLoadcaseColKind),
        tr.s(Key::CliLoadcaseColMass),
        "x [m]",
        "y [m]",
        "z [m]",
        "FSM [t*m]"
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
        tr.s(Key::CliLoadcaseTotalCg),
        "",
        res.displacement_t,
        res.lcg,
        res.tcg,
        res.vcg_z,
        res.fsm_t_m
    );
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row(
        tr.s(Key::CliLoadcaseDisplacement),
        format!("{:.3} t  ({:.3} m3)", res.displacement_t, res.volume_m3),
    );
    row("LCG / TCG", format!("{:.3} m / {:+.3} m", res.lcg, res.tcg));
    row(
        tr.s(Key::CliLoadcaseKgSolid),
        tr.fmt(
            Key::CliLoadcaseAboveKeel,
            &[("kg", &format!("{:.3}", res.kg_solid))],
        ),
    );
    row(
        "FSM / FSC",
        format!("{:.3} t*m  ->  {:+.3} m", res.fsm_t_m, res.fsc_m),
    );
    row(
        tr.s(Key::CliLoadcaseKgFluid),
        tr.fmt(
            Key::CliLoadcaseAboveKeel,
            &[("kg", &format!("{:.3}", res.kg_fluid))],
        ),
    );
    row(
        tr.s(Key::CliLoadcaseFloating),
        tr.fmt(
            Key::CliLoadcaseFloatingVal,
            &[
                ("draft", &format!("{:.3}", res.draft)),
                ("trim", &format!("{:+.3}", res.trim_m)),
                ("list", &format!("{:+.2}", res.list_deg)),
            ],
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
        tr.s(Key::CliLoadcaseGmSolidFluid),
        format!("{:.3} m / {:.3} m", res.gm_solid, res.gm_fluid),
    );

    if let Some(c) = &res.imo_criteria {
        println!(
            "{}",
            tr.fmt(
                Key::CliLoadcaseImoHeading,
                &[("kg", &format!("{:.3}", res.kg_fluid))]
            )
        );
        println!("  {}", seacraft_core::not_validated_note(tr.locale()));
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  {}",
            tr.s(Key::CliStabilityColDisp),
            "A(0-30)",
            "A(0-40)",
            "A(30-40)",
            "GZ(>=30)",
            "phi_max",
            "GM0",
            tr.s(Key::CliStabilityColStatus)
        );
        println!(
            "    {:>8}  {:>9}  {:>9}  {:>10}  {:>10}  {:>9}  {:>7}  ------",
            tr.s(Key::CliStabilityMinReq),
            ">=0.055",
            ">=0.090",
            ">=0.030",
            ">=0.200 m",
            ">=25 deg",
            ">=0.15"
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
            if c.passed {
                tr.s(Key::ReportBadgePass)
            } else {
                tr.s(Key::ReportBadgeFail)
            }
        );
    }
    if let Some(w) = &res.weather {
        print_weather_summary(w, tr);
    }
    Ok(())
}

/// The warning lines a loading table opens with when no equilibrium was
/// found (none when it was): its figures are the hull at the input
/// draft/trim. `--json` carries the same as `equilibrium_found`/`warnings`.
#[cfg(test)]
fn loading_equilibrium_lines(res: &loading::LoadingResult) -> Vec<String> {
    loading_equilibrium_lines_tr(res, Tr::new(Locale::En))
}

fn loading_equilibrium_lines_tr(res: &loading::LoadingResult, tr: Tr) -> Vec<String> {
    if res.equilibrium_found {
        return Vec::new();
    }
    std::iter::once(tr.fmt(
        Key::CliLoadcaseWarningNoEq,
        &[("note", tr.s(Key::LoadingNoEquilibrium))],
    ))
    .chain(res.warnings.iter().map(|w| format!("  {}", w.render(tr))))
    .collect()
}

fn cmd_damage(a: DamageArgs, tr: Tr) -> Result<()> {
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

    println!(
        "{}",
        tr.fmt(
            Key::CliDamageHeading,
            &[("name", &s.settings.name), ("case", &res.case_name)]
        )
    );
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row(
        tr.s(Key::CliDamageDisplacement),
        tr.fmt(
            Key::CliDamageDispVal,
            &[
                ("disp", &format!("{:.3}", res.displacement_t)),
                ("vol", &format!("{:.3}", res.net_volume_m3)),
                ("rho", &format!("{:.3}", res.water_density)),
            ],
        ),
    );
    row(
        "LCG / TCG / KG",
        format!(
            "{:.3} m / {:+.3} m / {}",
            res.lcg,
            res.tcg,
            tr.fmt(
                Key::CliLoadcaseAboveKeel,
                &[("kg", &format!("{:.3}", res.kg))]
            )
        ),
    );
    row(
        tr.s(Key::CliDamageIntactEq),
        tr.fmt(
            Key::CliDamageIntactVal,
            &[
                ("draft", &format!("{:.3}", res.intact_draft)),
                ("trim", &format!("{:+.3}", res.intact_trim_m)),
            ],
        ),
    );
    row(
        tr.s(Key::CliDamageDamagedEq),
        tr.fmt(
            Key::CliDamageDamagedVal,
            &[
                ("draft", &format!("{:.3}", res.draft)),
                ("trim", &format!("{:+.3}", res.trim_m)),
                ("heel", &format!("{:+.2}", res.heel_deg)),
                (
                    "status",
                    if res.equilibrium_found {
                        tr.s(Key::CliDamageFoundBadge)
                    } else {
                        tr.s(Key::CliDamageUnreachedBadge)
                    },
                ),
            ],
        ),
    );
    row(
        tr.s(Key::CliDamageSinkage),
        format!("{:+.3} m", res.sinkage_m),
    );
    row(
        tr.s(Key::CliDamageFreeboard),
        match res.freeboard_m {
            Some(fb) if fb < 0.0 => tr.fmt(
                Key::CliDamageImmersedBadge,
                &[("fb", &format!("{:+.3}", fb))],
            ),
            Some(fb) => format!("{:.3} m", fb),
            None => "—".to_string(),
        },
    );
    row(
        tr.s(Key::CliDamageNetCentroid),
        format!(
            "LCB {:.3} m, TCB {:+.3} m, KB {:.3} m",
            res.lcb_x, res.tcb_y, res.kb
        ),
    );
    println!();
    println!("{}", tr.s(Key::CliDamageCompHeading));
    println!(
        "    {:<30} {:>6} {:>8} {:>10} {:>10}",
        tr.s(Key::CliDamageColName),
        tr.s(Key::CliDamageColPerm),
        tr.s(Key::CliDamageColClipped),
        tr.s(Key::CliDamageColSubm),
        tr.s(Key::CliDamageColLost)
    );
    println!("    {}", "-".repeat(68));
    for c in &res.compartments {
        println!(
            "    {:<30} {:>6.2} {:>8} {:>10.3} {:>10.3}",
            c.name,
            c.permeability,
            if c.clip_to_hull {
                tr.s(Key::CliCommonYes)
            } else {
                tr.s(Key::CliCommonNo)
            },
            c.submerged_volume_m3,
            c.lost_volume_m3
        );
    }
    println!("    {}", "-".repeat(68));
    println!(
        "    {:<30} {:>6} {:>8} {:>10} {:>10.3}",
        tr.s(Key::CliDamageTotalLost),
        "",
        "",
        "",
        res.lost_volume_m3
    );
    println!();
    println!("{}", tr.s(Key::CliDamageResidualHeading));
    println!(
        "    {:>8}  {:>8}  {:>10}  {:>10}  {:>10}",
        tr.s(Key::CliDamageColRelDeg),
        tr.s(Key::CliDamageColAbsDeg),
        tr.s(Key::CliDamageColDraft),
        tr.s(Key::CliDamageColTrim),
        "GZ [m]"
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
        println!("{}", tr.s(Key::CliDamageCriteriaHeading));
        println!("  {}", seacraft_core::not_validated_note(tr.locale()));
        println!(
            "    {:>12}  {:>10}  {:>12}  {}",
            tr.s(Key::CliDamageColPosRange),
            tr.s(Key::CliDamageColMaxGz),
            tr.s(Key::CliDamageColAreaRange),
            tr.s(Key::CliStabilityColStatus)
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
            if c.passed {
                tr.s(Key::ReportBadgePass)
            } else {
                tr.s(Key::ReportBadgeFail)
            }
        );
        println!(
            "{}",
            tr.fmt(Key::CliDamageNoteLine, &[("note", &c.label.render(tr))])
        );
    }
    for w in &res.warnings {
        println!(
            "{}",
            tr.fmt(Key::CliCommonWarningIndent, &[("message", &w.render(tr))])
        );
    }
    Ok(())
}

fn cmd_seakeeping(a: SeakeepingArgs, tr: Tr) -> Result<()> {
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
                "{}",
                tr.fmt(
                    Key::CliCommonWroteStations,
                    &[
                        ("path", &csv_path.display().to_string()),
                        ("count", &rep.frequencies.len().to_string()),
                    ]
                )
            );
        }
        return Ok(());
    }

    println!(
        "{}",
        tr.fmt(Key::CliSeakeepingHeading, &[("name", &s.settings.name)])
    );
    let kyy_str = if let Some(kyy) = spec.kyy {
        format!(", kyy: {:.3} m", kyy)
    } else {
        String::new()
    };
    println!(
        "{}",
        tr.fmt(
            Key::CliSeakeepingParams,
            &[
                ("draft", &format!("{:.3}", spec.draft)),
                ("speed", &format!("{:.1}", spec.speed_kn)),
                ("kg", &format!("{:.3}", spec.kg)),
                ("kyy", &kyy_str),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliSeakeepingNatFreq,
            &[
                ("heave", &format!("{:.3}", rep.natural_heave_omega)),
                ("pitch", &format!("{:.3}", rep.natural_pitch_omega)),
            ]
        )
    );
    println!();
    let phase = tr.s(Key::CliSeakeepingColPhase);
    println!(
        "  {:>8}   {:>10}   {:>13}   {:>15}   {:>10}   {:>11}   {:>18}   {:>11}",
        "lambda/L",
        "lambda (m)",
        "omega (rad/s)",
        "omega_e (rad/s)",
        "z_a/zeta_a",
        phase,
        "theta_a/(k*zeta_a)",
        phase
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
        print_irregular_sea(irr, tr);
    }
    if !rep.warnings.is_empty() {
        println!();
        println!("{}", tr.s(Key::CliSeakeepingWarnings));
        for w in &rep.warnings {
            println!("  [{}] {}", w.code, w.message.render(tr));
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

fn print_irregular_sea(r: &seacraft_core::seastate::IrregularReport, tr: Tr) {
    use seacraft_core::seastate::WaveSpectrum;
    println!("{}", tr.s(Key::CliSeakeepingIrrHeading));
    match r.sea {
        WaveSpectrum::Bretschneider { hs, tp } => println!(
            "{}",
            tr.fmt(
                Key::CliSeakeepingBretschneider,
                &[("hs", &format!("{hs:.2}")), ("tp", &format!("{tp:.2}"))]
            )
        ),
        WaveSpectrum::Jonswap { hs, tp, gamma } => println!(
            "{}",
            tr.fmt(
                Key::CliSeakeepingJonswap,
                &[
                    ("hs", &format!("{hs:.2}")),
                    ("tp", &format!("{tp:.2}")),
                    ("gamma", &format!("{gamma:.2}")),
                ]
            )
        ),
    }
    println!(
        "{}",
        tr.fmt(
            Key::CliSeakeepingSpectrumCheck,
            &[
                ("hs", &format!("{:.3}", r.spectrum.hs_check)),
                ("t1", &format!("{:.2}", r.spectrum.t1_s)),
                ("tz", &format!("{:.2}", r.spectrum.tz_s)),
                ("count", &r.omega_count.to_string()),
                ("wmin", &format!("{:.3}", r.omega_min)),
                ("wmax", &format!("{:.3}", r.omega_max)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliSeakeepingSpeedLcg,
            &[
                ("speed", &format!("{:.1}", r.speed_kn)),
                ("lcg", &format!("{:.3}", r.x_lcg)),
            ]
        )
    );
    println!();
    println!(
        "  {:<22} {:>10} {:>10} {:>10}",
        tr.s(Key::CliSeakeepingColResponse),
        tr.s(Key::CliSeakeepingColSigAmpl),
        "RMS",
        "Tz (s)"
    );
    let row = |name: String, st: &seacraft_core::seastate::ResponseStats, unit: &str| {
        println!(
            "  {:<22} {:>10.4} {:>10.4} {:>10.2}  {unit}",
            name, st.significant, st.rms, st.tz_s
        );
    };
    row(tr.s(Key::CliSeakeepingHeave).into(), &r.heave, "m");
    row(tr.s(Key::CliSeakeepingPitch).into(), &r.pitch_deg, "deg");
    for p in &r.points {
        let tag = format!("{} (x {:.2})", p.name, p.x);
        row(
            tr.fmt(Key::CliSeakeepingMotionSuffix, &[("tag", &tag)]),
            &p.vertical_motion,
            "m",
        );
        row(
            tr.fmt(Key::CliSeakeepingRelativeSuffix, &[("tag", &tag)]),
            &p.relative_motion,
            "m",
        );
        row(
            tr.fmt(Key::CliSeakeepingAccelSuffix, &[("tag", &tag)]),
            &p.vertical_acceleration,
            "m/s2",
        );
    }
    println!();
    println!(
        "  {:<22} {:>10} {:>12} {:>9}",
        tr.s(Key::CliSeakeepingColPoint),
        tr.s(Key::CliSeakeepingColAccRms),
        tr.s(Key::CliSeakeepingColAccSig),
        "MSI %"
    );
    for p in &r.points {
        println!(
            "  {:<22} {:>10.4} {:>12.4} {:>9.1}",
            p.name, p.acceleration_rms_g, p.acceleration_significant_g, p.msi_percent
        );
    }
    println!("  {}", seacraft_core::seastate::msi_note(tr));
    for w in &r.warnings {
        println!(
            "{}",
            tr.fmt(
                Key::CliCommonWarningIndent,
                &[(
                    "message",
                    &format!("[{}]: {}", w.code, w.message.render(tr))
                )]
            )
        );
    }
}

fn print_weather_summary(w: &seacraft_core::weather::WeatherResult, tr: Tr) {
    let opt_deg = |v: Option<f64>| match v {
        Some(x) if x.is_finite() => format!("{x:.2} deg"),
        _ => "—".to_string(),
    };
    let badge = |passed: bool| {
        if passed {
            tr.s(Key::ReportBadgePass)
        } else {
            tr.s(Key::ReportBadgeFail)
        }
    };
    println!("{}", tr.s(Key::CliWeatherHeading));
    println!("  {}", seacraft_core::not_validated_note(tr.locale()));
    println!(
        "{}",
        tr.fmt(
            Key::CliWeatherWindage,
            &[
                ("a", &format!("{:.3}", w.area_m2)),
                ("hull", &format!("{:.3}", w.hull_windage_area_m2)),
                ("extra", &format!("{:.3}", w.extra_windage_area_m2)),
                ("z", &format!("{:.3}", w.z_m)),
                ("p", &format!("{:.0}", w.wind_pressure_pa)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliWeatherLevers,
            &[
                ("lw1", &format!("{:.4}", w.lw1_m)),
                ("lw2", &format!("{:.4}", w.lw2_m)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliWeatherRolling,
            &[
                ("x1", &format!("{:.3}", w.x1)),
                ("x2", &format!("{:.3}", w.x2)),
                ("k", &format!("{:.3}", w.k)),
                ("r", &format!("{:.3}", w.r)),
                ("c", &format!("{:.4}", w.c)),
                ("t", &format!("{:.2}", w.roll_period_s)),
                ("s", &format!("{:.4}", w.s)),
                ("phi1", &format!("{:.2}", w.phi1_deg)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliWeatherAngles,
            &[
                ("phi0", &opt_deg(w.phi0_deg)),
                ("lim", &format!("{:.1}", w.phi0_limit_deg)),
                ("phi0_st", badge(w.phi0_passed)),
                ("phi_roll", &opt_deg(w.phi_roll_deg)),
                ("phic", &opt_deg(w.phic_deg)),
                ("phi2", &opt_deg(w.phi2_deg)),
            ]
        )
    );
    println!(
        "{}",
        tr.fmt(
            Key::CliWeatherAreas,
            &[
                ("a", &format!("{:.4}", w.area_a_m_rad)),
                ("b", &format!("{:.4}", w.area_b_m_rad)),
                ("ratio", &format!("{:.2}", w.area_ratio)),
                ("area_st", badge(w.area_passed)),
                ("status", badge(w.passed)),
            ]
        )
    );
    for note in &w.notes {
        println!(
            "{}",
            tr.fmt(Key::CliWeatherNote, &[("note", &note.render(tr))])
        );
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
                let err =
                    cmd_export(parse_export(&args), Tr::new(Locale::En)).expect_err("refused");
                let msg = format!("{err}");
                assert!(msg.contains(named), "{ext} {flags:?}: {msg}");
                assert!(msg.contains(".obj"), "{msg}");
                assert!(!out.exists(), "nothing is written for {ext} {flags:?}");
            }
        }

        // `--up z` is what the other formats are anyway; OBJ takes them all.
        let stl = dir.join("hull.stl");
        let stl_text = stl.to_string_lossy().into_owned();
        cmd_export(
            parse_export(&[&fef_text, "-o", &stl_text, "--up", "z"]),
            Tr::new(Locale::En),
        )
        .expect("z-up STL");
        assert!(stl.exists());
        let obj = dir.join("hull.obj");
        let obj_text = obj.to_string_lossy().into_owned();
        cmd_export(
            parse_export(&[
                &fef_text,
                "-o",
                &obj_text,
                "--control",
                "--no-mirror",
                "--up",
                "y",
            ]),
            Tr::new(Locale::En),
        )
        .expect("OBJ honours every flag");
        assert!(std::fs::read_to_string(&obj)
            .unwrap()
            .contains("unmirrored"));
        // An unknown format still names the format, not a flag.
        let tiff = dir.join("hull.tiff").to_string_lossy().into_owned();
        let err = cmd_export(
            parse_export(&[&fef_text, "-o", &tiff, "--control"]),
            Tr::new(Locale::En),
        )
        .unwrap_err();
        assert!(
            format!("{err}").contains("unsupported output format"),
            "{err}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
