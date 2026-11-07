//! Frame cost, as ignored release benchmarks: dragging a point, orbiting each
//! example hull, and orbiting with one drawn layer off at a time. The orbit
//! benchmarks time `run_ui` and tessellation, and not the GPU, which
//! `examples/orbit_bench.rs` includes in a real window.
//!
//!
//! ```console
//! cargo test --release -p seacraft-gui --test perf -- --ignored --nocapture
//! ```
//!
//! The demo hull is loaded at level 3 and 4, put into each analysis mode with
//! and without curvature combs, and one control point is dragged across 30
//! frames in the profile view through the same synthetic-pointer path the
//! drag tests use (`tests/drag.rs`). Each `run_ui` frame is timed and the
//! table prints the median and 95th percentile of the drag frames, the frame
//! that releases the point, the frame after it (full refinement and the
//! views' refresh) and the median of five idle frames afterwards.

use std::time::Instant;

use eframe::egui;
use seacraft_core::AnalysisMode;
use seacraft_gui::App;

const DRAG_FRAMES: usize = 30;

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> f64 {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let start = Instant::now();
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    let ms = start.elapsed().as_secs_f64() * 1e3;
    out.textures_delta.clear();
    ms
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    let i = ((sorted.len() - 1) as f64 * p).round() as usize;
    sorted[i]
}

struct Row {
    median: f64,
    p95: f64,
    release: f64,
    refresh: f64,
    idle: f64,
}

fn measure(level: u8, mode: AnalysisMode, combs: bool) -> Row {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(level);
    app.set_analysis(mode);
    app.display_mut().combs = combs;
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    app.recompute_hydrostatics();
    frame(&ctx, &mut app, Vec::new());

    // A sheer point well inside the hull, as in `tests/drag.rs`.
    let index = {
        let surface = app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .expect("a sheer point")
    };
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1]
        .transform()
        .project(app.surface().unwrap().points[index].p, rect)
        .expect("point visible");

    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    // Past egui's click threshold: this frame starts the drag.
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(10.0, 0.0))],
    );
    let mut times: Vec<f64> = (0..DRAG_FRAMES)
        .map(|i| {
            let at = screen + egui::vec2(11.0 + i as f32, 0.0);
            frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)])
        })
        .collect();
    let end = screen + egui::vec2(11.0 + DRAG_FRAMES as f32, 0.0);
    let release = frame(&ctx, &mut app, vec![button(end, false)]);
    // The release raised the revision mid-frame: the next frame refines at
    // the full level and refreshes the views.
    let refresh = frame(&ctx, &mut app, Vec::new());
    // Measure now, so the debounced hydrostatics do not land in an idle
    // frame; then nothing has changed.
    app.recompute_hydrostatics();
    let mut idle: Vec<f64> = (0..5).map(|_| frame(&ctx, &mut app, Vec::new())).collect();
    idle.sort_by(f64::total_cmp);
    times.sort_by(f64::total_cmp);
    Row {
        median: percentile(&times, 0.5),
        p95: percentile(&times, 0.95),
        release,
        refresh,
        idle: percentile(&idle, 0.5),
    }
}

#[test]
#[ignore = "release benchmark: run with --release -- --ignored --nocapture"]
fn drag_frame_cost_per_mode() {
    let modes = [
        ("shaded", AnalysisMode::Shaded),
        ("zebra", AnalysisMode::Zebra { stripes: 14.0 }),
        ("curvature", AnalysisMode::Curvature { radius: 1.0 }),
        ("draft", AnalysisMode::Draft { angle_deg: 3.0 }),
    ];
    println!();
    println!("drag frame cost, {DRAG_FRAMES} frames, 1600 x 1000 window (ms)");
    println!(
        "{:>5} {:>10} {:>6} {:>8} {:>8} {:>8} {:>8} {:>8}",
        "level", "mode", "combs", "median", "p95", "release", "refresh", "idle"
    );
    for level in [3u8, 4] {
        for (name, mode) in modes {
            for combs in [false, true] {
                let row = measure(level, mode, combs);
                println!(
                    "{:>5} {:>10} {:>6} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>8.1}",
                    level,
                    name,
                    if combs { "on" } else { "off" },
                    row.median,
                    row.p95,
                    row.release,
                    row.refresh,
                    row.idle
                );
            }
        }
    }
}

/// One frame's cost split the way the real app spends it: building the shapes
/// (`run_ui`) and turning them into triangles (`tessellate`). The GPU upload
/// and draw are not included: a headless pass has no GPU.
struct OrbitFrame {
    ui: f64,
    tessellate: f64,
    vertices: usize,
    indices: usize,
    meshes: usize,
}

fn orbit_frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> OrbitFrame {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let start = Instant::now();
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    let ui = start.elapsed().as_secs_f64() * 1e3;
    out.textures_delta.clear();
    let start = Instant::now();
    let meshes = ctx.tessellate(out.shapes, out.pixels_per_point);
    let tessellate = start.elapsed().as_secs_f64() * 1e3;
    let (mut vertices, mut indices) = (0, 0);
    for clipped in &meshes {
        if let egui::epaint::Primitive::Mesh(mesh) = &clipped.primitive {
            vertices += mesh.vertices.len();
            indices += mesh.indices.len();
        }
    }
    OrbitFrame {
        ui,
        tessellate,
        vertices,
        indices,
        meshes: meshes.len(),
    }
}

/// Orbit one view of the four through the real pointer path and report the
/// median frame, once per example hull at the levels a user would pick.
#[test]
#[ignore = "release benchmark: run with --release -- --ignored --nocapture"]
fn orbit_frame_cost() {
    const FRAMES: usize = 60;
    let dir = format!("{}/../../examples", env!("CARGO_MANIFEST_DIR"));
    println!();
    println!("orbit frame cost, {FRAMES} frames, 1600 x 1000 window, four views (ms)");
    println!(
        "{:>16} {:>5} {:>8} {:>8} {:>8} {:>10} {:>10} {:>7}",
        "hull", "level", "ui", "tess", "total", "vertices", "tris", "meshes"
    );
    for hull in [
        "runabout-6m",
        "demo-hull",
        "yacht-10m",
        "wigley",
        "container-230m",
    ] {
        for level in [2u8, 3, 4] {
            let mut app = App::headless_for_test(Some(&format!("{dir}/{hull}.fef")));
            app.set_subdivision_level(level);
            let ctx = egui::Context::default();
            for _ in 0..3 {
                orbit_frame(&ctx, &mut app, Vec::new());
            }
            app.recompute_hydrostatics();
            orbit_frame(&ctx, &mut app, Vec::new());
            let index = app
                .cameras()
                .iter()
                .position(|c| c.kind == seacraft_gui::camera::ViewKind::Perspective)
                .expect("a perspective view");
            let at = app.viewport_rect(index).center();
            let mut frames: Vec<OrbitFrame> = (0..FRAMES)
                .map(|_| {
                    app.cameras_mut()[index].orbit(3.0, 1.0);
                    orbit_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)])
                })
                .collect();
            let mut ui: Vec<f64> = frames.iter().map(|f| f.ui).collect();
            let mut tess: Vec<f64> = frames.iter().map(|f| f.tessellate).collect();
            let mut total: Vec<f64> = frames.iter().map(|f| f.ui + f.tessellate).collect();
            ui.sort_by(f64::total_cmp);
            tess.sort_by(f64::total_cmp);
            total.sort_by(f64::total_cmp);
            let last = frames.pop().expect("frames ran");
            println!(
                "{:>16} {:>5} {:>8.1} {:>8.1} {:>8.1} {:>10} {:>10} {:>7}",
                hull,
                level,
                percentile(&ui, 0.5),
                percentile(&tess, 0.5),
                percentile(&total, 0.5),
                last.vertices,
                last.indices / 3,
                last.meshes,
            );
        }
    }
}

/// What each drawn layer costs while orbiting: the same orbit with one layer
/// switched off at a time, on the largest example.
#[test]
#[ignore = "release benchmark: run with --release -- --ignored --nocapture"]
fn orbit_layer_cost() {
    const FRAMES: usize = 60;
    type Toggle = fn(&mut seacraft_gui::render::DisplayOptions);
    let layers: [(&str, Toggle); 8] = [
        ("all on", |_| {}),
        ("no shade/water", |d| {
            d.shaded = false;
            d.waterplane = false;
        }),
        ("no shading", |d| d.shaded = false),
        ("no net", |d| d.control_net = false),
        ("no points", |d| d.points = false),
        ("no grid", |d| d.grid = false),
        ("no waterplane", |d| d.waterplane = false),
        ("no volumes", |d| d.volumes = false),
    ];
    let path = format!(
        "{}/../../examples/container-230m.fef",
        env!("CARGO_MANIFEST_DIR")
    );
    println!();
    println!("container-230m, orbit, {FRAMES} frames, one layer off at a time (ms)");
    println!(
        "{:>5} {:>14} {:>8} {:>8} {:>8} {:>8} {:>10} {:>10}",
        "level", "layers", "ui", "tess", "total", "hover", "vertices", "tris"
    );
    for level in [3u8, 4, 5] {
        for (name, toggle) in layers {
            // Level 5 is too slow to repeat for every layer: what remains
            // without the hull is the row that matters there.
            if level == 5 && !name.starts_with("no shad") {
                continue;
            }
            let mut app = App::headless_for_test(Some(&path));
            app.set_subdivision_level(level);
            toggle(app.display_mut());
            let ctx = egui::Context::default();
            for _ in 0..3 {
                orbit_frame(&ctx, &mut app, Vec::new());
            }
            app.recompute_hydrostatics();
            orbit_frame(&ctx, &mut app, Vec::new());
            let index = app
                .cameras()
                .iter()
                .position(|c| c.kind == seacraft_gui::camera::ViewKind::Perspective)
                .expect("a perspective view");
            let at = app.viewport_rect(index).center();
            let frames: Vec<OrbitFrame> = (0..FRAMES)
                .map(|_| {
                    app.cameras_mut()[index].orbit(3.0, 1.0);
                    orbit_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)])
                })
                .collect();
            let mut ui: Vec<f64> = frames.iter().map(|f| f.ui).collect();
            let mut tess: Vec<f64> = frames.iter().map(|f| f.tessellate).collect();
            let mut total: Vec<f64> = frames.iter().map(|f| f.ui + f.tessellate).collect();
            ui.sort_by(f64::total_cmp);
            tess.sort_by(f64::total_cmp);
            total.sort_by(f64::total_cmp);
            let last = frames.last().expect("frames ran");
            // The mouse moves over a view and no camera does.
            let mut hover: Vec<f64> = (0..FRAMES)
                .map(|i| {
                    let moved = at + egui::vec2((i % 7) as f32, (i % 5) as f32);
                    let f = orbit_frame(&ctx, &mut app, vec![egui::Event::PointerMoved(moved)]);
                    f.ui + f.tessellate
                })
                .collect();
            hover.sort_by(f64::total_cmp);
            println!(
                "{:>5} {:>14} {:>8.1} {:>8.1} {:>8.1} {:>8.1} {:>10} {:>10}",
                level,
                name,
                percentile(&ui, 0.5),
                percentile(&tess, 0.5),
                percentile(&total, 0.5),
                percentile(&hover, 0.5),
                last.vertices,
                last.indices / 3,
            );
        }
    }
}
