//! The far-field wave pattern in the window, headless: it is computed in a job while shown, an
//! out-of-date pattern is never the current one, and a failure is reported once.
//!
//! The waits are generous guards (a debug build on a loaded machine), not speed checks.

use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_gui::waves::{WaveSettings, WaveSpeed};
use seacraft_gui::App;

const PATIENCE: Duration = Duration::from_secs(120);

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            // Tall, so the Performance tab's sections are on screen.
            egui::vec2(1600.0, 3200.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    // Nothing uploads the fonts in a headless pass; a dropped delta panics.
    out.textures_delta.clear();
    out
}

#[allow(dead_code)] // used by the drawing tests
fn texts(out: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut text);
    }
    text
}

fn path(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The demo hull at level 2, two frames drawn.
fn demo() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&path("demo-hull.fef")));
    app.set_subdivision_level(2);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

/// Draw frames until `done` holds, failing after [`PATIENCE`].
fn frames_until(
    ctx: &egui::Context,
    app: &mut App,
    what: &str,
    done: impl Fn(&App) -> bool,
) -> egui::FullOutput {
    let t0 = Instant::now();
    loop {
        let out = frame(ctx, app, Vec::new());
        if done(app) {
            return out;
        }
        assert!(t0.elapsed() < PATIENCE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// The demo hull with its pattern shown and computed.
fn demo_with_waves() -> (App, egui::Context) {
    let (mut app, ctx) = demo();
    app.set_waves_shown(true);
    frames_until(&ctx, &mut app, "the wave pattern", |a| {
        a.wave_overlay().is_some()
    });
    (app, ctx)
}

#[test]
fn shown_pattern_computes_by_itself() {
    let (app, _ctx) = demo_with_waves();
    let w = app.wave_overlay().expect("the pattern");
    assert!((w.field.froude - 0.3).abs() < 1e-12);
    assert!(!app.waves_busy());
}

#[test]
fn a_settings_change_hides_the_old_pattern_at_once() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.4),
        ..app.wave_settings().clone()
    });
    assert!(
        app.wave_overlay().is_none(),
        "the Fn 0.3 pattern is out of date"
    );
    frames_until(&ctx, &mut app, "the Fn 0.4 pattern", |a| {
        a.wave_overlay().is_some()
    });
    let froude = app.wave_overlay().unwrap().field.froude;
    assert!((froude - 0.4).abs() < 1e-12, "Fn {froude}");
}

#[test]
fn a_model_edit_or_new_level_hides_the_old_pattern() {
    let (mut app, ctx) = demo_with_waves();
    app.set_subdivision_level(1);
    assert!(
        app.wave_overlay().is_none(),
        "a new level hides the pattern"
    );
    // Another model opened while the level-1 job may still run: the demo's result never returns.
    frame(&ctx, &mut app, Vec::new());
    std::thread::sleep(Duration::from_millis(350));
    frame(&ctx, &mut app, Vec::new());
    let demo_l = {
        let (a, _) = demo_with_waves();
        a.wave_overlay().unwrap().length_wl
    };
    app.open_model(std::path::Path::new(&path("wigley.fef")))
        .expect("open wigley.fef");
    assert!(
        app.wave_overlay().is_none(),
        "a new model hides the pattern"
    );
    frames_until(&ctx, &mut app, "the Wigley pattern", |a| {
        a.wave_overlay().is_some()
    });
    let l = app.wave_overlay().unwrap().length_wl;
    assert!(
        (l - demo_l).abs() > 1e-6,
        "the Wigley's L {l}, not the demo's"
    );
}

#[test]
fn a_failing_pattern_is_reported_once() {
    let (mut app, ctx) = demo();
    app.set_waves_shown(true);
    app.hydro_options_mut().draft = 0.0;
    frames_until(&ctx, &mut app, "the failure", |a| {
        a.wave_failure().is_some()
    });
    for _ in 0..20 {
        frame(&ctx, &mut app, Vec::new());
        std::thread::sleep(Duration::from_millis(20));
    }
    let lines = app
        .messages()
        .entries()
        .filter(|m| m.text.contains("Wave pattern failed"))
        .count();
    assert_eq!(lines, 1, "one log line for one failure");
    let failure = app.wave_failure().unwrap();
    assert!(failure.contains("Michell"), "{failure}");
    assert!(!app.waves_busy(), "a failed pattern is not retried");
    assert!(app.wave_overlay().is_none());
}

#[test]
fn hidden_pattern_computes_nothing() {
    let (mut app, ctx) = demo();
    for _ in 0..20 {
        frame(&ctx, &mut app, Vec::new());
        std::thread::sleep(Duration::from_millis(20));
        assert!(!app.waves_busy());
    }
    assert!(app.wave_overlay().is_none());
    assert!(!app.wave_settings().show);
}

#[test]
fn blocking_compute_makes_the_pattern_current() {
    let (mut app, _ctx) = demo();
    app.set_waves_shown(true);
    app.compute_waves_blocking().expect("the pattern");
    assert!(app.wave_overlay().is_some());
}
