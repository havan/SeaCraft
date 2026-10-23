//! Loading condition dock and background worker tests.

use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn loadcase_path() -> String {
    format!(
        "{}/../../examples/demo-loadcase.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn frame(ctx: &egui::Context, app: &mut App) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
}

fn setup_demo_with_loadcase() -> (App, egui::Context, seacraft_core::loading::LoadingCondition) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let text = std::fs::read_to_string(loadcase_path()).expect("demo-loadcase.json");
    let cond: seacraft_core::loading::LoadingCondition =
        serde_json::from_str(&text).expect("parse demo-loadcase.json");
    let project = seacraft_core::project::Project {
        loading_conditions: vec![cond.clone()],
        ..Default::default()
    };
    app.set_project(project);
    app.set_show_loading(true);

    let ctx = egui::Context::default();
    // Draw two frames so the docks and panels settle.
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);

    (app, ctx, cond)
}

#[test]
fn loading_dock_and_blocking_compute() {
    let (mut app, _ctx, cond) = setup_demo_with_loadcase();

    let surface = app.surface().expect("model loaded");
    let with_windage = app.project().with_windage(&cond);
    let mut opts = app.hydro_options().clone();
    opts.subdivision.level = app.subdivision_level();
    opts.subdivision.mode = app.subdivision_mode();

    let direct = seacraft_core::loading::evaluate(surface, &opts, &with_windage);
    let blocking = app.compute_loading_blocking(None).expect("compute");

    assert_eq!(blocking.displacement_t, direct.displacement_t);
    assert_eq!(blocking.kg_fluid, direct.kg_fluid);
    assert_eq!(blocking.floating.draft, direct.floating.draft);

    assert!(
        (blocking.displacement_t - 12.147).abs() < 0.01,
        "displacement {} != 12.147",
        blocking.displacement_t
    );
    assert!(
        (blocking.floating.draft - 0.620).abs() < 0.005,
        "draft {} != 0.620",
        blocking.floating.draft
    );
}

#[test]
fn loading_worker_thread_computes_in_background() {
    let (mut app, ctx, _cond) = setup_demo_with_loadcase();

    assert!(app.loading_result().is_none());
    app.start_loading_compute();

    let start = Instant::now();
    while app.loading_result().is_none() {
        if start.elapsed() > Duration::from_secs(20) {
            panic!("timed out waiting for loading worker thread");
        }
        std::thread::sleep(Duration::from_millis(50));
        frame(&ctx, &mut app);
    }

    let res = app.loading_result().expect("worker result");
    assert!(
        (res.displacement_t - 12.147).abs() < 0.01,
        "worker displacement {} != 12.147",
        res.displacement_t
    );
    assert!(
        (res.floating.draft - 0.620).abs() < 0.005,
        "worker draft {} != 0.620",
        res.floating.draft
    );
}

#[test]
fn a_result_for_an_edited_hull_is_not_taken_as_fresh() {
    let (mut app, ctx, _) = setup_demo_with_loadcase();
    app.start_loading_compute();
    assert!(app.loading_busy());
    // Edit the hull while the worker measures the old one.
    app.snapshot_undo();
    app.surface_mut().unwrap().points[3].p.z += 0.05;
    app.mark_geometry_changed();
    let t0 = Instant::now();
    while app.loading_busy() && t0.elapsed() < Duration::from_secs(60) {
        frame(&ctx, &mut app);
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(!app.loading_busy(), "the worker finished");
    assert!(
        app.loading_result().is_none(),
        "a result for the hull before the edit was shown as current"
    );
}
