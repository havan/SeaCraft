//! Damage stability dock and background worker tests.

use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_core::damage::{self, CompartmentSpec, DamageCase};
use seacraft_core::loading::LoadingCondition;
use seacraft_core::project::Project;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn damage_case_path() -> String {
    format!(
        "{}/../../examples/demo-damage.json",
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

fn setup_demo_with_damage() -> (App, egui::Context, DamageCase, DamageCase, LoadingCondition) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);

    let dc_text = std::fs::read_to_string(damage_case_path()).expect("demo-damage.json");
    let orig_case: DamageCase = serde_json::from_str(&dc_text).expect("parse demo-damage.json");

    let comp = match &orig_case.compartments[0] {
        CompartmentSpec::Inline(c) => c.clone(),
        CompartmentSpec::Named(_) => panic!("expected inline compartment in demo-damage.json"),
    };

    let case1 = DamageCase {
        compartments: vec![CompartmentSpec::Named(comp.name.clone())],
        ..orig_case.clone()
    };

    let lc_text = std::fs::read_to_string(loadcase_path()).expect("demo-loadcase.json");
    let cond: LoadingCondition = serde_json::from_str(&lc_text).expect("parse demo-loadcase.json");

    let case2 = DamageCase {
        name: "Damage with loadcase".into(),
        compartments: vec![CompartmentSpec::Named(comp.name.clone())],
        loading_condition_name: Some(cond.name.clone()),
        loading_condition: None,
        displacement_t: None,
        kg: None,
        lcg: None,
        tcg: None,
        water_density: None,
    };

    let project = Project {
        compartments: vec![comp],
        damage_cases: vec![case1.clone(), case2.clone()],
        loading_conditions: vec![cond.clone()],
        ..Default::default()
    };
    app.set_project(project);
    app.set_show_damage(true);

    let ctx = egui::Context::default();
    // Draw two frames so the docks and panels settle before reading rects.
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    let r0 = app.viewport_rect(0);
    assert!(r0.width() > 10.0, "rects settled: {r0:?}");

    (app, ctx, orig_case, case2, cond)
}

#[test]
fn damage_dock_and_blocking_compute() {
    let (mut app, _ctx, orig_case, case2, cond) = setup_demo_with_damage();

    let surface = app.surface().expect("model loaded").clone();
    let mut opts = app.hydro_options().clone();
    opts.subdivision.level = app.subdivision_level();
    opts.subdivision.mode = app.subdivision_mode();

    // (a) compute_damage_blocking(None) draft/trim/heel EXACTLY equal damage::evaluate on original demo-damage.json case
    let direct_orig = damage::evaluate(&surface, &opts, &orig_case).expect("evaluate original");
    let blocking1 = app.compute_damage_blocking(None).expect("compute case1");

    assert_eq!(blocking1.draft, direct_orig.draft);
    assert_eq!(blocking1.trim_m, direct_orig.trim_m);
    assert_eq!(blocking1.heel_deg, direct_orig.heel_deg);

    // (b) a second case with loading_condition_name = demo loadcase name gives exactly the same
    // result as damage::evaluate with that condition inline
    let with_windage = app.project().with_windage(&cond);
    let case2_inline = DamageCase {
        loading_condition: Some(with_windage),
        ..case2.clone()
    };
    let case2_inline_resolved = app
        .project()
        .resolve_damage_case(&case2_inline)
        .expect("resolve inline case2");
    let direct_case2 =
        damage::evaluate(&surface, &opts, &case2_inline_resolved).expect("evaluate case2 inline");
    let blocking2 = app
        .compute_damage_blocking(Some(&case2.name))
        .expect("compute case2");

    assert_eq!(blocking2, &direct_case2);
}

#[test]
fn damage_worker_thread_computes_in_background() {
    let (mut app, ctx, orig_case, _case2, _cond) = setup_demo_with_damage();
    assert!(app.damage_result().is_none());

    // (c) the worker path: start_damage_compute() then frames (sleep 50 ms, up to 30 s) until damage_result() is Some
    app.start_damage_compute();
    let start = Instant::now();
    while app.damage_result().is_none() {
        if start.elapsed() > Duration::from_secs(120) {
            panic!("timed out waiting for damage worker thread");
        }
        std::thread::sleep(Duration::from_millis(50));
        frame(&ctx, &mut app);
    }

    let surface = app.surface().expect("model loaded").clone();
    let mut opts = app.hydro_options().clone();
    opts.subdivision.level = app.subdivision_level();
    opts.subdivision.mode = app.subdivision_mode();
    let direct_orig = damage::evaluate(&surface, &opts, &orig_case).expect("evaluate original");

    let worker_res = app.damage_result().expect("worker result");
    assert_eq!(worker_res.draft, direct_orig.draft);
    assert_eq!(worker_res.trim_m, direct_orig.trim_m);
    assert_eq!(worker_res.heel_deg, direct_orig.heel_deg);
}
