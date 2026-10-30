//! Headless tests for the Performance tab (resistance curves and seakeeping RAOs).

use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_core::resistance;
use seacraft_core::seakeeping;
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_gui::app::{DesignDock, ResistanceResult};
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
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

#[test]
fn performance_tab_and_computations() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    // R5-3: a .fef opened without a project KG has none; the seakeeping
    // below needs the demo's.
    assert!(app.seakeeping_spec().is_err(), "no KG, no seakeeping");
    app.set_kg(seacraft_gui::app::DEMO_KG);
    let ctx = egui::Context::default();

    // Settle docks with two frames
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);

    // 1. Opening the Performance tab hides the Loading tab
    app.set_show_loading(true);
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    assert!(app.show_loading());
    assert!(!app.show_performance());
    assert_eq!(app.design_dock(), Some(DesignDock::Loading));

    app.set_show_performance(true);
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    assert!(app.show_performance());
    assert!(!app.show_loading());
    assert_eq!(app.design_dock(), Some(DesignDock::Performance));

    // 2. compute_resistance_blocking for Holtrop equals resistance::curve(&resistance::input_for(..), &speeds) exactly (same speeds)
    let res = app
        .compute_resistance_blocking()
        .expect("compute resistance")
        .clone();
    let surface = app.surface().expect("surface").clone();
    let subdiv = SubdivisionOptions {
        level: app.subdivision_level(),
        mode: app.subdivision_mode(),
    };
    let speeds = app.performance_speeds();
    let holtrop_in = resistance::input_for(&surface, subdiv).expect("holtrop input");
    let expected_core = resistance::curve(&holtrop_in, &speeds);
    let expected_res = ResistanceResult::from_core(
        "Holtrop & Mennen",
        &expected_core,
        holtrop_in.warnings(),
        "R_T (kN)",
    );
    assert_eq!(res, expected_res);
    assert_eq!(app.resistance_result(), Some(&expected_res));

    // 3. compute_raos_blocking frequencies equal seakeeping::head_sea_raos with the same SeakeepingSpec exactly
    let rep = app.compute_raos_blocking().expect("compute raos").clone();
    let spec = app.seakeeping_spec().expect("the demo has a KG");
    let expected_rep = seakeeping::head_sea_raos(&surface, &spec).expect("head sea raos");
    assert_eq!(rep.frequencies, expected_rep.frequencies);
    assert_eq!(
        app.rao_result().map(|r| &r.frequencies),
        Some(&expected_rep.frequencies)
    );

    // 4. The worker path: start_performance_compute(), then frames (sleep 50 ms, up to 60 s) until both results are Some
    app.performance_state_mut().resistance = None;
    app.performance_state_mut().raos = None;
    assert!(app.resistance_result().is_none());
    assert!(app.rao_result().is_none());

    app.start_performance_compute();
    let start = Instant::now();
    while app.resistance_result().is_none() || app.rao_result().is_none() {
        if start.elapsed() > Duration::from_secs(60) {
            panic!("timed out waiting for performance worker threads");
        }
        std::thread::sleep(Duration::from_millis(50));
        frame(&ctx, &mut app);
    }
    assert!(app.resistance_result().is_some());
    assert!(app.rao_result().is_some());
    assert_eq!(app.resistance_result(), Some(&expected_res));
    assert_eq!(
        app.rao_result().map(|r| &r.frequencies),
        Some(&expected_rep.frequencies)
    );
}
