//! Headless tests for superstructure and deck windage editor and profile outlines.

use eframe::egui;
use seacraft_core::loading::LoadingCondition;
use seacraft_core::math::Vec3;
use seacraft_core::project::Project;
use seacraft_core::weather::{WindageItem, WindageRect};
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

#[test]
fn test_windage_profile_and_selection() {
    let mut app = App::headless_for_test(Some(&demo_path()));

    let loadcase_str = std::fs::read_to_string(loadcase_path()).expect("read demo-loadcase");
    let loadcase: LoadingCondition =
        serde_json::from_str(&loadcase_str).expect("parse demo-loadcase");

    let deckhouse = WindageItem::from_profile(
        "Deckhouse",
        WindageRect {
            x_min: 3.0,
            x_max: 6.0,
            z_min: 1.6,
            z_max: 2.4,
        },
    );
    let mast = WindageItem {
        name: "Mast".into(),
        area_m2: 1.2,
        z_centroid: 2.0,
        profile: None,
    };

    let project = Project {
        loading_conditions: vec![loadcase],
        windage: vec![deckhouse, mast],
        ..Default::default()
    };
    app.set_project(project);

    // 1. Compute loading blocking -> weather.extra_windage_area_m2 == 2.4 + 1.2
    let res = app.compute_loading_blocking(None).expect("compute loading");
    let w = res.weather.as_ref().expect("weather result");
    assert!(
        (w.extra_windage_area_m2 - (2.4 + 1.2)).abs() < 1e-9,
        "extra_windage_area_m2: got {}, expected 3.6",
        w.extra_windage_area_m2
    );

    // 2. Assert windage_overlays() has exactly 1 item
    let overlays = app.windage_overlays();
    assert_eq!(overlays.len(), 1, "exactly 1 windage item with profile");
    assert_eq!(overlays[0].name, "Deckhouse");
    assert_eq!(overlays[0].rect.x_min, 3.0);
    assert_eq!(overlays[0].rect.x_max, 6.0);

    // 3. Draw two frames to settle layout
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    // 4. Activate Profile view (index 1)
    app.set_active_view(1);
    frame(&ctx, &mut app, Vec::new());
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

    // Center of Deckhouse rect: x=4.5, y=0.0, z=2.0
    let c = Vec3::new(4.5, 0.0, 2.0);
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

    let click_pos =
        click_screen.expect("found a point inside Deckhouse rect not near any control point");

    // Click: PointerMoved, Press, Release
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(click_pos)]);
    frame(&ctx, &mut app, vec![button(click_pos, true)]);
    frame(&ctx, &mut app, vec![button(click_pos, false)]);
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    assert_eq!(app.selected_windage(), Some(0));
    assert!(app.show_windage(), "windage tab became visible");
    assert!(!app.show_loading(), "loading panel is hidden");
    assert!(!app.show_damage(), "damage panel is hidden");
}
