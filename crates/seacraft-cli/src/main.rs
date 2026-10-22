use anyhow::{bail, Context, Result};
use clap::{Args, Parser, Subcommand, ValueEnum};
use seacraft_core::hydro::{self, HydroOptions};
use seacraft_core::stability::{cross_curves, solve_draft};
use seacraft_core::subdiv::{evaluate, SubdivisionMode, SubdivisionOptions};
use seacraft_core::{
    analysis::AnalysisMode, demo, export, fef, linesplan, math::Vec3, mesh::Surface, raster,
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
    /// Cross curves of stability (KN / GZ)
    Stability(StabilityArgs),
    /// Linesplan: stations, buttocks and waterlines as SVG or DXF
    Linesplan(LinesplanArgs),
    /// Render a preview PNG, including zebra, curvature and draft analysis
    Render(RenderArgs),
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
    /// Output file; the extension selects the format (.obj/.stl/.ply/.html)
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
    #[arg(long, default_value_t = 0.0)]
    trim: f64,
    #[command(flatten)]
    subdiv: SubdivArgsCli,
    /// Write CSV to this path (otherwise print a table)
    #[arg(long)]
    csv: Option<PathBuf>,
}

#[derive(Args, Debug)]
struct LinesplanArgs {
    /// Input .fef file
    file: PathBuf,
    /// Output path: `.svg` (default) or `.dxf` for a layered AutoCAD drawing
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
        Command::Stability(a) => cmd_stability(a),
        Command::Linesplan(a) => cmd_linesplan(a),
        Command::Render(a) => cmd_render(a),
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
    let result = hydro::compute(&s, &opts);
    if a.json {
        println!("{}", serde_json::to_string_pretty(&result)?);
        return Ok(());
    }
    println!("{}  -  hydrostatics at T {:.3} m", s.settings.name, result.draft);
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
        "ply" => export::ply(&s, a.subdiv.level, mode).into_bytes(),
        "html" | "htm" => export::viewer_html(&s, a.subdiv.level, mode).into_bytes(),
        other => bail!("unsupported output format '{}' (use .obj, .stl, .ply or .html)", other),
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

fn cmd_linesplan(a: LinesplanArgs) -> Result<()> {
    let s = load(&a.file)?;
    let opts = linesplan::LinesplanOptions {
        stations: a.stations,
        buttocks: a.buttocks,
        waterlines: a.waterlines,
        level: a.subdiv.level,
        mode: a.subdiv.mode.into(),
    };
    // The extension picks the format: `.dxf` gives the three views on named
    // layers plus 3D copies of every curve, anything else the standalone SVG.
    let extension = a
        .output
        .extension()
        .map(|e| e.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default();
    let text = if extension == "dxf" {
        linesplan::dxf(&s, opts)
    } else {
        linesplan::svg(&s, opts)
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
    let curves = cross_curves(&hull, &heels, &displacements, a.kg, a.trim, &base);

    if let Some(path) = a.csv {
        let mut text = String::from("heel_deg,displacement_t,draft_m,kn_m,gz_m\n");
        for c in &curves {
            for p in &c.points {
                text.push_str(&format!(
                    "{:.2},{:.3},{:.4},{:.4},{:.4}\n",
                    c.heel_deg, p.displacement_t, p.draft, p.kn, p.gz
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
    println!("\n  GZ in metres. Draft solver:");
    for c in &curves {
        let drafts: Vec<String> = c.points.iter().map(|p| format!("{:.3}", p.draft)).collect();
        println!("    {:>4.0} deg: {}", c.heel_deg, drafts.join(", "));
    }
    let _ = solve_draft; // keep the public helper referenced
    let _ = Vec3::ZERO;
    Ok(())
}
