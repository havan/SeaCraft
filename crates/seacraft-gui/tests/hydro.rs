//! The hydrostatics dock must agree with the CLI for the same condition.

use eframe::egui;
use seacraft_gui::App;

/// One headless frame, so the render cache and the docks settle.
fn frame(app: &mut App) {
    let ctx = egui::Context::default();
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(1600.0, 1000.0))),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
}

fn demo_path() -> String {
    format!("{}/../../examples/demo-hull.fef", env!("CARGO_MANIFEST_DIR"))
}

#[test]
fn the_dock_reports_the_cli_reference_numbers() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_kg(2.2);
    app.recompute_hydrostatics();
    let h = app.hydrostatics().expect("a measurement");
    assert!((h.volume_m3 - 11.85).abs() < 0.06, "volume {}", h.volume_m3);
    assert!((h.block_coefficient - 0.504).abs() < 0.01, "Cb {}", h.block_coefficient);
    assert!((h.waterplane_coefficient - 0.824).abs() < 0.01, "Cwp {}", h.waterplane_coefficient);
    assert!((h.kb - 0.392).abs() < 0.01, "KB {}", h.kb);
    assert!((h.km_transverse - 2.63).abs() < 0.05, "KM {}", h.km_transverse);
    assert!((h.mainframe_area_m2 - 1.47).abs() < 0.03, "midship {}", h.mainframe_area_m2);
    assert!((h.gm(2.2) - (h.km_transverse - 2.2)).abs() < 1e-12);
    assert!((h.draft - 0.62).abs() < 1e-12);
}

#[test]
fn clearing_the_hydrostatics_flag_empties_the_displacement() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.recompute_hydrostatics();
    let full = app.hydrostatics().unwrap().volume_m3;
    assert!(full > 10.0);

    {
        let surface = app.surface_mut().expect("model");
        surface.layers[0].use_in_hydrostatics = false;
    }
    app.mark_geometry_changed();
    app.recompute_hydrostatics();
    let none = app.hydrostatics().unwrap().volume_m3;
    assert!(none < 1e-9, "no hydrostatic layer means no displacement, got {none}");

    {
        let surface = app.surface_mut().expect("model");
        surface.layers[0].use_in_hydrostatics = true;
    }
    app.mark_geometry_changed();
    app.recompute_hydrostatics();
    assert!((app.hydrostatics().unwrap().volume_m3 - full).abs() < 1e-9);
}

#[test]
fn a_hidden_layer_leaves_the_measurement_alone() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.recompute_hydrostatics();
    let before = app.hydrostatics().unwrap().volume_m3;
    {
        let surface = app.surface_mut().expect("model");
        surface.layers[0].visible = false;
    }
    app.mark_geometry_changed();
    app.recompute_hydrostatics();
    assert!((app.hydrostatics().unwrap().volume_m3 - before).abs() < 1e-9);
    frame(&mut app);
    assert!(app.render_cache().is_some_and(|m| m.faces.is_empty()), "the hidden layer is not drawn");
}
