//! Drag frame cost, as an ignored release benchmark:
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
