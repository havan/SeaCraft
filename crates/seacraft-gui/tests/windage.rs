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

/// R8-3 (A04 follow-up). The Windage tab's totals refer a profiled item's
/// model-`z` centroid to the keel, as the weather criterion does: the demo
/// moved 1.5 m up (its loads and the deckhouse with it) shows the same
/// centroid as at its own keel, and both agree with the extra windage the
/// weather result was built from. Before, the totals averaged model `z`
/// (profiled) with height above the keel (plain) and rose with the model.
#[test]
fn windage_totals_follow_the_keel_and_match_the_weather_criterion() {
    let base = seacraft_core::fef::parse(&std::fs::read_to_string(demo_path()).unwrap()).unwrap();
    let cond0: LoadingCondition =
        serde_json::from_str(&std::fs::read_to_string(loadcase_path()).unwrap()).unwrap();
    let dir = std::env::temp_dir().join(format!("seacraft-r8-3-windage-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let keel0 = base.lowest_hydrostatics_point();

    let mut totals = Vec::new();
    for (i, dz) in [0.0, 1.5].into_iter().enumerate() {
        let mut s = base.clone();
        for p in &mut s.points {
            p.p.z += dz;
        }
        let path = dir.join(format!("shifted-{i}.fef"));
        std::fs::write(&path, seacraft_core::fef::write(&s)).unwrap();
        let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
        assert!(
            (app.keel_z() - (keel0 + dz)).abs() < 1e-9,
            "keel {}",
            app.keel_z()
        );

        let mut cond = cond0.clone();
        cond.lightship.z += dz;
        for item in &mut cond.items {
            item.z += dz;
        }
        for t in &mut cond.tanks {
            t.z_min += dz;
            t.z_max += dz;
        }
        let deckhouse = WindageItem::from_profile(
            "Deckhouse",
            WindageRect {
                x_min: 3.0,
                x_max: 6.0,
                z_min: keel0 + 1.6 + dz,
                z_max: keel0 + 2.4 + dz,
            },
        );
        let mast = WindageItem {
            name: "Mast".into(),
            area_m2: 1.2,
            z_centroid: 3.0,
            profile: None,
        };
        app.set_project(Project {
            loading_conditions: vec![cond],
            windage: vec![deckhouse, mast],
            ..Default::default()
        });

        let (area, z) = app.windage_totals();
        assert!((area - 3.6).abs() < 1e-9, "dz {dz}: area {area}");
        let want = (2.4 * 2.0 + 1.2 * 3.0) / 3.6;
        assert!((z - want).abs() < 1e-9, "dz {dz}: z {z} vs {want}");

        // The weather result's lever, less the hull's own windage, is the
        // extra items' centroid above the keel: the same figure.
        let res = app.compute_loading_blocking(None).expect("compute loading");
        assert!(res.equilibrium_found, "dz {dz}: {:?}", res.warnings);
        let w = res.weather.as_ref().expect("weather result");
        assert!((w.extra_windage_area_m2 - area).abs() < 1e-9);
        let windage_z = w.z_m + w.underwater_z_m;
        let extra_z = (windage_z * w.area_m2 - w.hull_windage_area_m2 * w.hull_windage_z_m)
            / w.extra_windage_area_m2;
        assert!(
            (extra_z - z).abs() < 1e-6,
            "dz {dz}: weather's extra windage at {extra_z} m vs the totals' {z} m"
        );
        totals.push(z);
    }
    assert!((totals[0] - totals[1]).abs() < 1e-12, "{totals:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
