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
    analysis::AnalysisMode, curves, demo, export, fef, linesplan, math::Vec3, mesh::Surface,
    develop, offsets, raster, resistance, transform,
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
    /// Still-water resistance and effective power (Holtrop-Mennen 1984)
    Resistance(ResistanceArgs),
    /// Developable plate expansion: flatten a strake or buttock band
    Unfold(UnfoldArgs),
    /// Cross curves of stability (KN / GZ)
    Stability(StabilityArgs),
    /// Linesplan: stations, buttocks and waterlines as SVG or DXF
    Linesplan(LinesplanArgs),
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
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Write CSV to this path (otherwise print a table)
    #[arg(long)]
    csv: Option<PathBuf>,
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
        Command::Info(a) => cmd_info(a),
        Command::Hydro(a) => cmd_hydro(a),
        Command::Export(a) => cmd_export(a),
        Command::Subdiv(a) => cmd_subdiv(a),
        Command::Sac(a) => cmd_sac(a),
        Command::Bonjean(a) => cmd_bonjean(a),
        Command::Offsets(a) => cmd_offsets(a),
        Command::Transform(a) => cmd_transform(a),
        Command::Resistance(a) => cmd_resistance(a),
        Command::Unfold(a) => cmd_unfold(a),
        Command::Stability(a) => cmd_stability(a),
        Command::Linesplan(a) => cmd_linesplan(a),
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
    o.mainframe_x = a.mainframe;
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

    // Measure the input once; every later number is compared against it.
    let before = transform::MainDimensions::measure(&s, subdiv);

    let (mut out, fit) = if targets.is_empty() {
        (s.clone(), None)
    } else {
        let (fitted, report) = transform::fit_to_targets(&s, targets, subdiv, 1e-3, 4)
            .map_err(|e| anyhow::anyhow!(e))?;
        (fitted, Some(report))
    };
    if let Some(scale) = scale {
        transform::apply(&mut out, scale, Vec3::ZERO)
            .map_err(|e| anyhow::anyhow!("--scale: {e}"))?;
    }
    if translate != Vec3::ZERO {
        transform::apply(&mut out, Vec3::new(1.0, 1.0, 1.0), translate)
            .map_err(|e| anyhow::anyhow!("--translate: {e}"))?;
    }
    if let Some(axis) = a.mirror {
        let axis = match axis {
            MirrorArg::X => transform::Axis::X,
            MirrorArg::Y => transform::Axis::Y,
            MirrorArg::Z => transform::Axis::Z,
        };
        transform::mirror(&mut out, axis).map_err(|e| anyhow::anyhow!("--mirror: {e}"))?;
    }

    let after = transform::MainDimensions::measure(&out, subdiv);
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
        std::fs::write(path, text).with_context(|| format!("cannot write {}", path.display()))?;
        println!("wrote {} ({} speeds)", path.display(), rows.len());
    }
    if a.json {
        let body = serde_json::json!({
            "input": input,
            "warnings": warnings,
            "speeds": rows,
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
    println!("\n  effective power only: no propeller, wake fraction or thrust deduction.");
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
                text.push_str(&format!(
                    "{:.2},{:.3},{:.4},{:.4},{:.4},{:.4}\n",
                    c.heel_deg, p.displacement_t, p.draft, p.trim_m, p.kn, p.gz
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
            print!("    {:>7.3}", p.gz);
        }
        println!();
    }
    println!("\n  GZ in metres. Solved draft / trim per displacement:");
    for c in &curves {
        let solved: Vec<String> = c
            .points
            .iter()
            .map(|p| format!("{:.3} m / {:+.3} m", p.draft, p.trim_m))
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
    let _ = solve_draft; // keep the public helper referenced
    let _ = Vec3::ZERO;
    Ok(())
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
