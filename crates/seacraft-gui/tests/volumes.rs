//! Headless tests for tanks and compartments overlay rendering and selection.

use eframe::egui;
use seacraft_core::damage::{Compartment, CompartmentSpec, DamageCase};
use seacraft_core::loading::LoadingCondition;
use seacraft_core::math::Vec3;
use seacraft_core::project::Project;
use seacraft_gui::app::VolumeRef;
use seacraft_gui::render::VolumeKind;
use seacraft_gui::App;

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
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
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

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

fn damage_path() -> String {
    format!(
        "{}/../../examples/demo-damage.json",
        env!("CARGO_MANIFEST_DIR")
    )
}

#[test]
fn test_volumes_overlays_and_viewport_selection() {
    let mut app = App::headless_for_test(Some(&demo_path()));

    let loadcase_str = std::fs::read_to_string(loadcase_path()).expect("read demo-loadcase");
    let loadcase: LoadingCondition =
        serde_json::from_str(&loadcase_str).expect("parse demo-loadcase");

    let damage_str = std::fs::read_to_string(damage_path()).expect("read demo-damage");
    let damage_json: serde_json::Value =
        serde_json::from_str(&damage_str).expect("parse demo-damage");
    let comp: Compartment =
        serde_json::from_value(damage_json["compartments"][0].clone()).expect("parse comp");

    let damage_case = DamageCase {
        name: "Midship flooding".into(),
        compartments: vec![CompartmentSpec::Named("Midship compartment".into())],
        ..Default::default()
    };

    let project = Project {
        loading_conditions: vec![loadcase],
        compartments: vec![comp],
        damage_cases: vec![damage_case],
        ..Default::default()
    };
    app.set_project(project);

    // 1. Assert volume_overlays() before compute
    let overlays = app.volume_overlays();
    assert_eq!(overlays.len(), 3, "2 tanks + 1 compartment");

    assert_eq!(overlays[0].0, VolumeRef::Tank(0));
    assert_eq!(overlays[1].0, VolumeRef::Tank(1));
    assert_eq!(overlays[2].0, VolumeRef::Compartment(0));

    let box_est_1 = 0.15 + 0.68 * (0.75 - 0.15);
    match overlays[1].1.kind {
        VolumeKind::Tank { liquid_z } => {
            let lz = liquid_z.expect("tank has liquid_z");
            assert!(
                (lz - box_est_1).abs() < 1e-6,
                "liquid_z matches box estimate: {lz} vs {box_est_1}"
            );
        }
        _ => panic!("expected tank"),
    }

    match overlays[2].1.kind {
        VolumeKind::Compartment { flooded } => {
            assert!(
                flooded,
                "compartment is flooded when damage case naming it is selected"
            );
        }
        _ => panic!("expected compartment"),
    }

    // 2. Compute loading blocking
    let res = app.compute_loading_blocking(None).expect("compute loading");
    let surf_z_1 = res
        .tanks
        .iter()
        .find(|t| t.name == "Fresh water tank (forward)")
        .expect("tank 1 in results")
        .surface_z;

    let overlays_after = app.volume_overlays();
    match overlays_after[1].1.kind {
        VolumeKind::Tank { liquid_z } => {
            let lz = liquid_z.expect("tank has liquid_z after compute");
            assert!(
                (lz - surf_z_1).abs() < 1e-6,
                "liquid_z matches surface_z: {lz} vs {surf_z_1}"
            );
        }
        _ => panic!("expected tank"),
    }

    // 3. Viewport click selection
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    app.set_active_view(1); // Profile view
    frame(&ctx, &mut app, Vec::new());

    let rect = app.viewport_rect(1);
    assert!(rect.width() > 100.0, "profile viewport drawn: {rect:?}");

    let view = app.cameras()[1].transform();
    let pts: Vec<Vec3> = app
        .surface()
        .expect("surface")
        .points
        .iter()
        .map(|p| p.p)
        .collect();

    // Find a point inside tank 1 box (x: 4.9..6.9, y: -0.75..0.75, z: 0.15..0.75)
    // that is NOT within 10 px (PICK_RADIUS) of any control point
    let c = Vec3::new(5.9, 0.0, 0.45);
    let offsets = [
        Vec3::ZERO,
        Vec3::new(0.2, 0.0, 0.0),
        Vec3::new(-0.2, 0.0, 0.0),
        Vec3::new(0.0, 0.0, 0.1),
        Vec3::new(0.0, 0.0, -0.1),
        Vec3::new(0.3, 0.0, 0.1),
        Vec3::new(-0.3, 0.0, -0.1),
    ];

    let mut click_screen = None;
    for offset in offsets {
        let p3d = c + offset;
        if let Some(pos) = view.project(p3d, rect) {
            let near_cp = pts.iter().any(|&cp| {
                if let Some(cp_pos) = view.project(cp, rect) {
                    (cp_pos - pos).length() <= 10.0
                } else {
                    false
                }
            });
            if !near_cp {
                click_screen = Some(pos);
                break;
            }
        }
    }

    let click_pos = click_screen.expect("found a point inside tank 1 not near any control point");

    // Click: PointerMoved, Press, Release
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(click_pos)]);
    frame(&ctx, &mut app, vec![button(click_pos, true)]);
    frame(&ctx, &mut app, vec![button(click_pos, false)]);
    // Settle the frame after opening the dock
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    assert_eq!(app.selected_volume(), Some(VolumeRef::Tank(1)));
    assert!(app.show_loading(), "loading panel became visible");

    // A click far outside every box clears it
    let rect2 = app.viewport_rect(1);
    let view2 = app.cameras()[1].transform();
    let outside_screen = view2
        .project(Vec3::new(1.0, 0.0, 1.5), rect2)
        .expect("project outside point");

    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(outside_screen)],
    );
    frame(&ctx, &mut app, vec![button(outside_screen, true)]);
    frame(&ctx, &mut app, vec![button(outside_screen, false)]);
    frame(&ctx, &mut app, Vec::new());

    assert_eq!(
        app.selected_volume(),
        None,
        "click outside cleared selected_volume"
    );
}

#[test]
fn the_loading_and_damage_panels_share_one_dock() {
    let mut app = App::headless_for_test(None);
    app.set_show_loading(true);
    assert!(app.show_loading() && !app.show_damage());
    app.set_show_damage(true);
    assert!(app.show_damage() && !app.show_loading());
    app.set_show_damage(false);
    assert!(!app.show_damage() && !app.show_loading());
}

#[test]
fn clipped_volumes_sharing_a_name_keep_their_own_meshes() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let comp = |x_min: f64| Compartment {
        name: "Void".into(),
        x_min,
        x_max: x_min + 1.0,
        y_min: -1.0,
        y_max: 1.0,
        z_min: 0.0,
        z_max: 1.0,
        permeability: 0.9,
        clip_to_hull: true,
    };
    app.set_project(Project {
        compartments: vec![comp(3.0), comp(6.0)],
        ..Default::default()
    });
    assert_eq!(app.volume_overlays().len(), 2);
    assert_eq!(
        app.clipped_volume_cache_len(),
        2,
        "with one entry per name each frame rebuilds both meshes"
    );
}
