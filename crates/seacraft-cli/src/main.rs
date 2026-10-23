mod mcp;

use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use seacraft_core::hydro::{self, HydroOptions};
use seacraft_core::stability::{
    cross_curves, evaluate_imo_criteria, hydrostatics_free_trim, solve_draft, upright_lcb,
    StabilityCondition,
};
use seacraft_core::subdiv::{evaluate, SubdivisionMode, SubdivisionOptions};
use seacraft_core::{lackenby, loading, table, trimopt, 
    analysis::AnalysisMode, curves, demo, export, fef, linesplan, math::Vec3, mesh::Surface,
    develop, offsets, parametric::{self, HullKind, HullSpec}, planing, raster, resistance, topology, transform,
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
                  and stability the way the original application does."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Write the built-in demo hull to a .fef file
    Demo(DemoArgs),
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
    /// Linesplan: stations, buttocks and waterlines as SVG or DXF
    Linesplan(LinesplanArgs),
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
    /// Export the control net instead of the refined surface
    #[arg(long)]
    control: bool,
    /// Up axis for OBJ output
    #[arg(long, value_enum, default_value_t = UpArg::Z)]
    up: UpArg,
    /// Skip mirroring symmetric layers (single half only)
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
    let subdiv = SubdivisionOptions { level: a.level, mode: SubdivisionMode::Loop };
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
        if a.no_hold_volume { " (NOT held: draft fixed)" } else { "" }
    );
    println!("   trim    draft     LWL      Fn     RT [kN]   PE [kW]");
    for p in &scan.points {
        println!(
            "{:+7.3} {:7.3} {:8.3} {:6.3} {:10.3} {:10.2}",
            p.trim_m, p.draft_m, p.lwl_m, p.froude, p.rt_n / 1000.0, p.pe_kw
        );
    }
    if let Some(b) = scan.best {
        println!(
            "cheapest: trim {:+.3} m at draft {:.3} m - RT {:.3} kN, PE {:.2} kW{}",
            b.trim_m,
            b.draft_m,
            b.rt_n / 1000.0,
            b.pe_kw,
            if scan.at_boundary { " (at the edge of the range)" } else { "" }
        );
    }
    for w in &scan.warnings {
        println!("  warning: {w}");
    }
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
        SubdivisionOptions { level: a.level, mode: SubdivisionMode::Loop },
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
        std::fs::write(path, t.csv()).with_context(|| format!("cannot write {}", path.display()))?;
    }
    if let Some(path) = &a.html {
        std::fs::write(path, t.html())
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
        "   T     nabla      Delta     KB      BM     KML     LCB     LCF     Cb     Cp      TPC    MTC"
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
    /// Target LCB as a percentage of LWL, positive forward of midships
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
    let mut opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions { level: a.level, mode: SubdivisionMode::Loop },
    );
    if let Some(d) = a.draft {
        opts.draft = d;
    }
    opts.stations = opts.stations.max(41);
    let now = hydro::compute(&s, &opts);
    let lcb_now = (now.lcb.x - now.model_min.x) / now.lwl * 100.0 - 50.0;
    let target = lackenby::LackenbyTarget::new(
        a.cp.unwrap_or(now.prismatic_coefficient),
        a.lcb.unwrap_or(lcb_now),
    );
    let (out, report) = lackenby::fit(&s, target, &opts, 1e-3, 10).map_err(|e| anyhow::anyhow!(e))?;
    std::fs::write(&a.output, fef::write(&out))
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
    println!("  target  Cp {:.4}  LCB {:+.2} %", target.cp, target.lcb_pct);
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

#[derive(Args, Debug)]
struct ResistanceArgs {
    /// Input .fef file
    file: PathBuf,
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
    /// Superstructure windage item name:area:z in m2 and m above keel (repeatable)
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
    /// Input loading condition JSON file
    case: PathBuf,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Emit JSON instead of a table
    #[arg(long)]
    json: bool,
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
    #[command(flatten)]
    subdiv: SubdivArgsCli,
}

#[derive(Args, Debug)]
struct McpArgs {
    /// Forward tool calls to a running `seacraft-gui --control HOST:PORT`
    #[arg(long, value_name = "HOST:PORT")]
    connect: Option<String>,
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

fn main() {
    if let Err(e) = run() {
        eprintln!("error: {e:#}");
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    match Cli::parse().command {
        Command::Demo(a) => cmd_demo(a),
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
        Command::Resistance(a) => cmd_resistance(a),
        Command::Unfold(a) => cmd_unfold(a),
        Command::Edit(a) => cmd_edit(a),
        Command::Planing(a) => cmd_planing(a),
        Command::Stability(a) => cmd_stability(a),
        Command::Loadcase(a) => cmd_loadcase(a),
        Command::Linesplan(a) => cmd_linesplan(a),
        Command::Report(a) => cmd_report(a),
        Command::Render(a) => cmd_render(a),
        Command::Mcp(a) => cmd_mcp(a),
    }
}

fn load(path: &Path) -> Result<Surface> {
    let text = std::fs::read_to_string(path)
        .with_context(|| format!("cannot read {}", path.display()))?;
    let surface = fef::parse(&text).with_context(|| format!("cannot parse {}", path.display()))?;
    Ok(surface)
}

fn cmd_demo(a: DemoArgs) -> Result<()> {
    let surface = demo::demo_hull(a.symmetric);
    std::fs::write(&a.output, fef::write(&surface))
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!(
        "wrote {} ({} control points, {} faces, {} layer{})",
        a.output.display(),
        surface.points.len(),
        surface.faces.len(),
        surface.layers.len(),
        if surface.layers.len() == 1 { "" } else { "s" }
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
    std::fs::write(&a.output, fef::write(&surface))
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
    println!("  depth D (m)   {:8.3}     {:8.3}", rep.target_depth, rep.target_depth);
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
        s.settings.length, s.settings.beam, s.settings.draft, s.settings.water_density,
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
            if l.symmetric { "symmetric " } else { "asymmetric " },
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
        SubdivisionOptions { level: a.subdiv.level, mode: a.subdiv.mode.into() },
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
        println!("{}  -  hydrostatics at T {:.3} m", s.settings.name, result.draft);
    }
    println!();
    let row = |label: &str, value: String| println!("  {:<26} {}", label, value);
    row("volume", format!("{:.3} m3", result.volume_m3));
    row(
        "displacement",
        format!("{:.0} kg  ({:.3} t)", result.displacement_kg, result.displacement_t),
    );
    row("block coefficient Cb", format!("{:.4}", result.block_coefficient));
    row("prismatic Cp", format!("{:.4}", result.prismatic_coefficient));
    row("midship Cm", format!("{:.4}", result.midship_coefficient));
    row("waterplane Cwp", format!("{:.4}", result.waterplane_coefficient));
    row("vertical prismatic Cvp", format!("{:.4}", result.vertical_prismatic_coefficient));
    println!();
    row(
        "LCB",
        format!("x {:.3}  y {:.3}  z {:.3} m", result.lcb.x, result.lcb.y, result.lcb.z),
    );
    row("KB", format!("{:.3} m above keel", result.kb));
    row("LCF", format!("x {:.3}  y {:.3}", result.lcf.x, result.lcf.y));
    println!();
    row("LWL x BWL", format!("{:.3} x {:.3} m", result.lwl, result.bwl));
    row("wetted surface", format!("{:.3} m2", result.wetted_surface_m2));
    row("waterplane area", format!("{:.3} m2", result.waterplane_area_m2));
    row(
        "waterplane I_T / I_L",
        format!(
            "{:.1} / {:.1} m4",
            result.waterplane_inertia_transverse, result.waterplane_inertia_longitudinal
        ),
    );
    row("BM transverse", format!("{:.3} m", result.bm_transverse));
    row("BM longitudinal", format!("{:.3} m", result.bm_longitudinal));
    row("KM transverse", format!("{:.3} m", result.km_transverse));
    row("KM longitudinal", format!("{:.3} m", result.km_longitudinal));
    if let Some(kg) = a.kg {
        row(&format!("GM (KG {:.3})", kg), format!("{:.3} m", result.gm(kg)));
    }
    println!();
    row("midship section area", format!("{:.4} m2 at x {:.3}", result.mainframe_area_m2, result.mainframe_x));
    row("lateral area", format!("{:.3} m2", result.lateral_area_m2));
    if !result.sac.is_empty() {
        println!();
        println!("  section area curve ({} stations)", result.sac.len());
        for p in &result.sac {
            let bar = (p.area / result.mainframe_area_m2.max(1e-9) * 40.0).round() as usize;
            println!("  x {:>7.3}  {:>7.4} m2  {}", p.x, p.area, "#".repeat(bar.min(60)));
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

fn cmd_export(a: ExportArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mode: SubdivisionMode = a.subdiv.mode.into();
    let ext = a
        .output
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let data: Vec<u8> = match ext.as_str() {
        "obj" => {
            let mut text = if a.control {
                export::obj_control(&s, if a.up == UpArg::Y { export::UpAxis::Y } else { export::UpAxis::Z })
            } else {
                export::obj(&s, a.subdiv.level, mode, if a.up == UpArg::Y { export::UpAxis::Y } else { export::UpAxis::Z })
            };
            if a.no_mirror {
                text = obj_from_unmirrored(&s, a.subdiv.level, mode);
            }
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
    std::fs::write(&a.output, &data)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!("wrote {} ({:.1} KiB)", a.output.display(), data.len() as f64 / 1024.0);
    Ok(())
}

/// OBJ without mirroring symmetric layers, for inspecting a half model.
fn obj_from_unmirrored(s: &Surface, level: u8, mode: SubdivisionMode) -> String {
    let mesh = evaluate(s, SubdivisionOptions { level, mode });
    let mut out = String::from("# SeaCraft - unmirrored\n");
    for v in &mesh.verts {
        out.push_str(&format!("v {:.6} {:.6} {:.6}\n", v.x, v.y, v.z));
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
        SubdivisionOptions { level: a.subdiv.level, mode: a.subdiv.mode.into() },
    );
    if a.mirror {
        let sym: Vec<bool> = s.layers.iter().map(|l| l.symmetric).collect();
        mesh.mirror_symmetric(&sym);
    }
    let mut baked = s.clone();
    baked.points = mesh
        .verts
        .iter()
        .map(|p| seacraft_core::mesh::ControlPoint { p: *p, kind: seacraft_core::mesh::VertexKind::Regular })
        .collect();
    baked.faces = mesh.faces.clone();
    baked.face_layer = if mesh.face_layer.is_empty() {
        vec![0; mesh.faces.len()]
    } else {
        mesh.face_layer.clone()
    };
    baked.edges.clear();
    baked.rebuild_edges();
    std::fs::write(&a.output, fef::write(&baked))
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
        SubdivisionOptions { level: a.subdiv.level, mode: a.subdiv.mode.into() },
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
        let title = format!("{} - sectional area curve, T = {:.3} m", s.settings.name, opts.draft);
        std::fs::write(path, curves::sac_svg(&result.sac, &title))
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
            std::fs::write(&path, text)
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
        SubdivisionOptions { level: a.subdiv.level, mode: a.subdiv.mode.into() },
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
        std::fs::write(path, curves::bonjean_svg(&bonjean, &title))
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
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {} ({} stations)", path.display(), bonjean.sections.len());
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
    let s = load(&a.file)?;
    let opts = HydroOptions::from_surface(
        &s,
        SubdivisionOptions { level: a.subdiv.level, mode: a.subdiv.mode.into() },
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

    if let Some(path) = &a.html {
        std::fs::write(path, offsets::html(&table, &title))
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
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
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
        println!(
            "wrote {} ({} stations x {} waterlines)",
            path.display(),
            table.stations.len(),
            table.waterlines_m.len()
        );
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&table)?);
        return Ok(());
    }
    if a.html.is_some() && a.csv.is_none() {
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
        bail!("{key} must be three comma-separated numbers, got {}", parts.len());
    }
    Ok(Vec3::new(parts[0], parts[1], parts[2]))
}

fn cmd_transform(a: TransformArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.level,
        mode: if a.catmull_clark { SubdivisionMode::CatmullClark } else { SubdivisionMode::Loop },
    };
    let targets = transform::Targets { lwl: a.lwl, bwl: a.bwl, draft: a.draft };
    let scale = a.scale.as_deref().map(|t| parse_triple(t, "--scale")).transpose()?;
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
    let (out, report) =
        transform::run(&s, &request, subdiv).map_err(|e| anyhow::anyhow!(e))?;
    let before = report.before;
    let after = report.after;
    let fit = report.fit.clone();
    std::fs::write(&a.output, fef::write(&out))
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
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
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

    println!("{}  -  Holtrop-Mennen resistance, T {:.3} m", s.settings.name, input.draft);
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
        input.lcb_pct, input.half_entrance_deg, input.transom_area_m2, input.bulb_area_m2,
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
    let a: usize = parts[0].parse().with_context(|| format!("{key}: `{}` is not an index", parts[0]))?;
    let b: usize = parts[1].parse().with_context(|| format!("{key}: `{}` is not an index", parts[1]))?;
    Ok((a, b))
}

fn parse_triple_indices(text: &str, key: &str) -> Result<(usize, usize, usize)> {
    let parts: Vec<&str> = text.split(',').map(|t| t.trim()).collect();
    if parts.len() != 3 {
        bail!("{key} takes FACE,A,B");
    }
    let mut out = [0usize; 3];
    for (i, p) in parts.iter().enumerate() {
        out[i] = p.parse().with_context(|| format!("{key}: `{p}` is not an index"))?;
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
        let r = topology::split_face(&mut s, face).map_err(|e| anyhow::anyhow!("--split-face: {e}"))?;
        splits.push(r);
    }
    for text in &a.insert_edge {
        let (face, x, y) = parse_triple_indices(text, "--insert-edge")?;
        topology::insert_edge(&mut s, face, x, y)
            .map_err(|e| anyhow::anyhow!("--insert-edge {text}: {e}"))?;
    }
    for text in &a.crease {
        let (x, y) = parse_pair(text, "--crease")?;
        topology::set_crease(&mut s, x, y, true).map_err(|e| anyhow::anyhow!("--crease {text}: {e}"))?;
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
    s.validate().map_err(|e| anyhow::anyhow!("the edited model does not validate: {e}"))?;
    std::fs::write(&a.output, fef::write(&s))
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
    let subdiv = SubdivisionOptions { level: a.subdiv.level, mode: a.subdiv.mode.into() };
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
                p.speed_kn, p.speed_m_s, p.cv, p.c_delta, p.cl_beta, p.trim_deg, p.lambda,
                p.wetted_length_m, p.lcp_over_b, p.cf, p.resistance_n, p.pe_w
            ));
        }
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
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
    println!("{}  -  Savitsky planing, T {:.3} m", s.settings.name, opts.draft);
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
            p.speed_kn, p.cv, p.trim_deg, p.lambda, p.wetted_length_m, p.lcp_over_b,
            p.resistance_n / 1000.0, p.pe_w / 1000.0
        );
    }
    let missed: Vec<f64> = speeds
        .iter()
        .filter(|s| !curve.iter().any(|p| (p.speed_kn - **s).abs() < 1.0))
        .copied()
        .collect();
    if !missed.is_empty() {
        println!("  no planing equilibrium at {:?} kn (outside the hull's planing range)", missed);
    }
    println!("\n  effective power only; add --propeller-diameter to `resistance` for the shaft power.");
    Ok(())
}

fn cmd_unfold(a: UnfoldArgs) -> Result<()> {
    let s = load(&a.file)?;
    let subdiv = SubdivisionOptions {
        level: a.level,
        mode: if a.catmull_clark { SubdivisionMode::CatmullClark } else { SubdivisionMode::Loop },
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
        std::fs::write(path, develop::svg(&unfolded, &title))
            .with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {}", path.display());
    }
    if a.json {
        println!("{}", serde_json::to_string_pretty(&unfolded)?);
        return Ok(());
    }
    println!("{}  -  plate expansion across a {:?} band", s.settings.name, axis);
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
    // layers plus 3D copies of every curve, anything else the standalone SVG.
    let extension = a
        .output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let text = match extension.as_str() {
        "dxf" => linesplan::dxf(&s, opts),
        "igs" | "iges" => linesplan::iges(&s, opts),
        _ => linesplan::svg(&s, opts),
    };
    std::fs::write(&a.output, text)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_report(a: ReportArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mut opts = seacraft_core::report::ReportOptions::from_surface(&s);
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
    let html = seacraft_core::report::design_report(&s, &opts)
        .map_err(|e| anyhow::anyhow!("{e}"))?;
    std::fs::write(&a.output, html)
        .with_context(|| format!("cannot write {}", a.output.display()))?;
    println!("wrote {}", a.output.display());
    Ok(())
}

fn cmd_render(a: RenderArgs) -> Result<()> {
    let s = load(&a.file)?;
    let mode = if a.catmull_clark { SubdivisionMode::CatmullClark } else { SubdivisionMode::Loop };
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
    std::fs::write(&a.output, bytes)
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
        w.trim().parse().with_context(|| format!("bad width '{w}'"))?,
        h.trim().parse().with_context(|| format!("bad height '{h}'"))?,
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
    let hull = hydro::Hull::new(&s, SubdivisionOptions { level: a.subdiv.level, mode });
    let mut base = HydroOptions::from_surface(
        &s,
        SubdivisionOptions { level: a.subdiv.level, mode },
    );
    base.trim = a.trim;
    let heels = parse_list(&a.heels)?;
    let displacements = match &a.displacements {
        Some(text) => parse_list(text)?,
        None => {
            let design_volume = hull.displaced_volume(base.draft, a.trim, 0.0);
            let t = design_volume * base.water_density / 1000.0;
            vec![(t * 0.5).round(), t.round(), (t * 1.25).round()]
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
                let d = p.draft.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into());
                let t = p.trim_m.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into());
                let kn = p.kn.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into());
                let gz = p.gz.map(|v| format!("{:.4}", v)).unwrap_or_else(|| "—".into());
                text.push_str(&format!(
                    "{:.2},{:.3},{},{},{},{}\n",
                    c.heel_deg, p.displacement_t, d, t, kn, gz
                ));
            }
        }
        std::fs::write(&path, text).with_context(|| format!("cannot write {}", path.display()))?;
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

fn cmd_loadcase(a: LoadcaseArgs) -> Result<()> {
    let s = load(&a.file)?;
    let case_text = std::fs::read_to_string(&a.case)
        .with_context(|| format!("cannot read {}", a.case.display()))?;
    let cond: loading::LoadingCondition = serde_json::from_str(&case_text)
        .with_context(|| format!("cannot parse {}", a.case.display()))?;
    loading::validate(&cond).map_err(|e| anyhow::anyhow!("{}: {e}", a.case.display()))?;

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
        "TOTAL / CG",
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
        format!("{:.3} m / {:.3} m  (LWL {:.3} m)", res.lcb_x, res.lcf_x, res.lwl),
    );
    row(
        "KB / BM / KM",
        format!("{:.3} / {:.3} / {:.3} m", res.kb, res.bm_transverse, res.km_transverse),
    );
    row(
        "GM (solid / fluid)",
        format!("{:.3} m / {:.3} m", res.gm_solid, res.gm_fluid),
    );

    if let Some(c) = &res.imo_criteria {
        println!("\n  IMO A.749 / 2008 IS Code Part A intact stability criteria (KG_fluid {:.3} m):", res.kg_fluid);
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

fn print_weather_summary(w: &seacraft_core::weather::WeatherResult) {
    let opt_deg = |v: Option<f64>| match v {
        Some(x) if x.is_finite() => format!("{x:.2} deg"),
        _ => "—".to_string(),
    };
    println!("\n  IMO 2008 IS Code Part A §2.3 weather criterion:");
    println!(
        "    windage   A {:.3} m2 (hull {:.3}, extra {:.3})  Z {:.3} m  P {:.0} Pa",
        w.area_m2, w.hull_windage_area_m2, w.extra_windage_area_m2, w.z_m, w.wind_pressure_pa
    );
    println!(
        "    levers    lw1 {:.4} m   lw2 {:.4} m",
        w.lw1_m, w.lw2_m
    );
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
        match mcp::McpServer::connect(addr) {
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
