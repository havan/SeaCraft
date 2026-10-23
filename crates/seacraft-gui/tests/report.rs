//! The design report from the GUI: project report settings, the options
//! window, and a written file with the project's sections.

use eframe::egui;
use seacraft_core::damage::{CompartmentSpec, DamageCase};
use seacraft_core::loading::LoadingCondition;
use seacraft_core::project::{Project, ReportSettings};
use seacraft_gui::App;

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn frame(ctx: &egui::Context, app: &mut App) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 1000.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
}

#[test]
fn the_report_carries_the_projects_conditions_and_cases() {
    let mut app = App::headless_for_test(Some(&example("demo-hull.fef")));
    app.set_subdivision_level(2);
    let cond: LoadingCondition =
        serde_json::from_str(&std::fs::read_to_string(example("demo-loadcase.json")).unwrap())
            .unwrap();
    let mut case: DamageCase =
        serde_json::from_str(&std::fs::read_to_string(example("demo-damage.json")).unwrap())
            .unwrap();
    let compartments = case
        .compartments
        .iter()
        .filter_map(|c| match c {
            CompartmentSpec::Inline(c) => Some(c.clone()),
            CompartmentSpec::Named(_) => None,
        })
        .collect::<Vec<_>>();
    case.compartments = compartments
        .iter()
        .map(|c| CompartmentSpec::Named(c.name.clone()))
        .collect();
    app.set_project(Project {
        loading_conditions: vec![cond],
        compartments,
        damage_cases: vec![case],
        report: ReportSettings {
            include_resistance: Some(false),
            ..Default::default()
        },
        ..Default::default()
    });

    // The options window opens and draws without disturbing the model.
    let ctx = egui::Context::default();
    app.set_show_report_options(true);
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    assert!(app.show_report_options());

    let path =
        std::env::temp_dir().join(format!("seacraft-gui-report-{}.html", std::process::id()));
    app.write_report_blocking(&path).unwrap();
    let html = std::fs::read_to_string(&path).unwrap();
    assert!(html.contains("Loading Conditions"), "no loading section");
    assert!(html.contains("Damage Cases"), "no damage section");
    assert!(!html.contains("NaN"));
    assert!(app.status().starts_with("wrote report"), "{}", app.status());
    let _ = std::fs::remove_file(path);
}
