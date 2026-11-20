//! The Heel view, headless: it computes while open, goes stale (never shown as current) when the
//! model moves on, keeps its trial weights, and falls back to the bare hull when its loading
//! condition goes away.

use eframe::egui;
use seacraft_core::loading::{LoadingCondition, WeightItem};
use seacraft_core::math::Vec3;
use seacraft_gui::analysis_views::AnalysisView;
use seacraft_gui::heel_view::HeelStart;
use seacraft_gui::App;

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
    out
}

/// The demo at a cheap level with the dock KG at 2.2 m, settled.
fn demo() -> (egui::Context, App) {
    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(1);
    app.set_kg(2.2);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (ctx, app)
}

fn weight(mass_t: f64, x: f64, y: f64, z: f64) -> WeightItem {
    WeightItem {
        name: "Weight 1".into(),
        mass_t,
        x,
        y,
        z,
    }
}

#[test]
fn opening_the_view_computes_the_equilibrium() {
    let (_ctx, mut app) = demo();
    assert!(
        app.heel_current().is_none(),
        "nothing before the view opens"
    );
    app.set_heel_view_open(true);
    assert_eq!(app.analysis_view(), Some(AnalysisView::Heel));
    app.compute_heel_blocking().unwrap();
    let out = app.heel_current().expect("computed for the current model");
    assert!(!out.sweep.is_empty());
    assert!(
        out.equilibrium.heel_for_side(0.0).is_some(),
        "{:?}",
        out.equilibrium
    );
    assert!(!app.heel_stale());
}

#[test]
fn the_view_computes_on_its_own_while_open() {
    let (ctx, mut app) = demo();
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.weights.push(weight(0.5, 4.0, 0.8, 1.5));
    app.set_heel_state(state);
    for _ in 0..600 {
        frame(&ctx, &mut app, Vec::new());
        if app.heel_current().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = app.heel_current().expect("the job finished");
    let heel = out.equilibrium.heel_for_side(0.0).unwrap();
    assert!(heel > 0.0, "a starboard weight heels to starboard: {heel}");
}

#[test]
fn editing_the_model_makes_the_view_stale() {
    let (_ctx, mut app) = demo();
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.weights.push(weight(0.5, 4.0, 0.8, 1.5));
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    assert!(!app.heel_stale());
    let p = app.surface().unwrap().points[10].p;
    app.move_point(10, p + Vec3::new(0.0, 0.0, 0.05)).unwrap();
    assert!(app.heel_stale(), "the kept result is out of date");
    assert!(app.heel_current().is_none(), "never shown as current");
    assert_eq!(
        app.heel_state().weights.len(),
        1,
        "the trial weights survive"
    );
}

#[test]
fn a_removed_condition_falls_back_to_dock() {
    let (ctx, mut app) = demo();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    let cond: LoadingCondition = serde_json::from_str(&text).unwrap();
    app.project_mut_for_test().loading_conditions.push(cond);
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(0);
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    assert!(app.heel_current().is_some());

    app.project_mut_for_test().loading_conditions.clear();
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.heel_state().start, HeelStart::Dock);
    assert!(
        app.status().contains("that loading condition is gone"),
        "{}",
        app.status()
    );
}

#[test]
fn no_kg_means_no_job() {
    // The demo hull from a file with no project sidecar: no KG anywhere.
    let dir = std::env::temp_dir().join(format!("seacraft-heel-nokg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hull.fef");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo-hull.fef"),
        &path,
    )
    .unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.set_subdivision_level(1);
    assert_eq!(app.kg(), None, "no KG to start from");
    let ctx = egui::Context::default();
    app.set_heel_view_open(true);
    for _ in 0..5 {
        frame(&ctx, &mut app, Vec::new());
    }
    assert!(!app.heel_busy(), "no job without a KG");
    assert!(app.heel_current().is_none());
    assert!(app.compute_heel_blocking().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_set_opens_the_view() {
    let (_ctx, mut app) = demo();
    app.set_heel_view_open(true);
    assert_eq!(app.analysis_view(), Some(AnalysisView::Heel));
    app.set_heel_view_open(false);
    assert_eq!(app.analysis_view(), None);
}
