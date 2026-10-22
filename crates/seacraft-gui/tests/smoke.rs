//! The acceptance gate for the GUI: a real model renders and measures with no
//! window, and degenerate models do not panic.

use eframe::egui;
use seacraft_gui::App;

fn demo_path() -> String {
    format!("{}/../../examples/demo-hull.fef", env!("CARGO_MANIFEST_DIR"))
}

/// Run `n` headless frames. Textures are never uploaded in a test, so their
/// deltas are cleared instead of dropped.
fn frames(app: &mut App, n: usize) {
    let ctx = egui::Context::default();
    for _ in 0..n {
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
}

#[test]
fn demo_model_renders_and_measures_without_a_window() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    frames(&mut app, 3);
    app.recompute_hydrostatics();
    let h = app.hydrostatics().expect("hydrostatics computed");
    assert!((h.volume_m3 - 11.85).abs() < 0.05, "volume {}", h.volume_m3);
    assert!(app.control_point_count() > 50);
    assert!(app.render_cache().is_some_and(|m| !m.faces.is_empty()));
}

#[test]
fn a_missing_file_leaves_the_app_empty_and_drawable() {
    let mut app = App::headless_for_test(Some("does/not/exist.fef"));
    assert!(app.surface().is_none());
    frames(&mut app, 2);
    assert!(app.hydrostatics().is_none());
}

#[test]
fn one_point_and_no_faces_do_not_panic() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    {
        let surface = app.surface_mut().expect("model");
        surface.points.truncate(1);
        surface.faces.clear();
        surface.face_layer.clear();
        surface.edges.clear();
    }
    app.mark_geometry_changed();
    frames(&mut app, 2);
    app.recompute_hydrostatics();
    let h = app.hydrostatics().expect("a report, even if empty");
    assert!(h.volume_m3 < 1e-12);
}

#[test]
fn all_layers_hidden_still_draws_and_measures() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.recompute_hydrostatics();
    let volume = app.hydrostatics().unwrap().volume_m3;
    {
        let surface = app.surface_mut().expect("model");
        for layer in &mut surface.layers {
            layer.visible = false;
        }
    }
    app.mark_geometry_changed();
    frames(&mut app, 2);
    assert!(app.render_cache().is_some_and(|m| m.faces.is_empty()));
    app.recompute_hydrostatics();
    assert!((app.hydrostatics().unwrap().volume_m3 - volume).abs() < 1e-9);
}

#[test]
fn every_viewport_has_a_usable_rectangle() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    frames(&mut app, 2);
    for i in 0..4 {
        let rect = app.viewport_rect(i);
        assert!(rect.width() > 100.0 && rect.height() > 100.0, "viewport {i}: {rect:?}");
    }
}
