//! Regression tests for the round-8 GUI bug hunt
//! (`.hermes/research/2026-10-31_bughunt-B-gui.md`, verdicts in
//! `.hermes/plans/2026-10-31_round8-bughunt.md`):
//!
//! * **B03** — a New design (demo, example, parametric) is one undo step back
//!   to the previous model, but it also wiped every undo step before it.
//!   Opening a file still starts a fresh history.
//! * **B05** — a refused layer, Model menu or Inspector action only replaced
//!   the status line at info level; it is now an error in the message log.
//! * **B06** — a Project settings Apply that changed only the design
//!   requirements set the model dirty but left no undo step, so Ctrl+Z undid
//!   an older geometry edit instead.
//! * **B07** — a recent-files storage that cannot be written failed silently;
//!   the failure is now a warning in the message log (once).

use seacraft_core::math::Vec3;
use seacraft_core::project::Requirements;
use seacraft_gui::messages::Level;
use seacraft_gui::new_design::NewDesignRequest;
use seacraft_gui::recent::JsonFileStorage;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn points(app: &App) -> Vec<Vec3> {
    app.surface().unwrap().points.iter().map(|p| p.p).collect()
}

/// The demo model with three point edits, each one undo step. Returns the
/// app and the points before each edit, oldest first, plus the final ones.
fn edited_demo() -> (App, Vec<Vec<Vec3>>) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert_eq!(app.undo_len(), 0);
    let mut states = vec![points(&app)];
    for i in 0..3 {
        let p = app.surface().unwrap().points[5 + i].p + Vec3::new(0.0, 0.0, 0.05);
        app.move_point(5 + i, p).unwrap();
        states.push(points(&app));
    }
    assert_eq!(app.undo_len(), 3);
    (app, states)
}

/// Undo back through the three edits after the new model was undone.
fn assert_old_history_survives(mut app: App, states: &[Vec<Vec3>], what: &str) {
    assert_eq!(
        app.undo_len(),
        4,
        "{what}: the three edits and the new design"
    );
    app.undo();
    assert_eq!(
        points(&app),
        states[3],
        "{what}: undo restores the edited model"
    );
    for i in (0..3).rev() {
        assert!(app.can_undo(), "{what}: the edit before is still there");
        app.undo();
        assert_eq!(points(&app), states[i], "{what}: undo edit {i}");
    }
    assert!(!app.can_undo());
    assert!(!app.is_dirty(), "{what}: back to the opened file");
}

#[test]
fn b03_a_new_design_keeps_the_undo_history_before_it() {
    let (mut app, states) = edited_demo();
    app.apply_new_design(NewDesignRequest::Demo { half: true });
    assert_old_history_survives(app, &states, "demo");

    let (mut app, states) = edited_demo();
    app.open_example(seacraft_core::examples::Example::Runabout6m)
        .unwrap();
    assert_old_history_survives(app, &states, "example");

    let (mut app, states) = edited_demo();
    let dialog = seacraft_gui::new_design::NewDesignState::new(1.025);
    let design = seacraft_gui::new_design::ParametricDesign {
        spec: dialog.spec(),
        name: "Parametric".into(),
        requirements: dialog.requirements(),
        built: None,
    };
    app.apply_new_design(NewDesignRequest::Parametric(Box::new(design)));
    assert!(app.surface().unwrap().points.len() != states[0].len());
    assert_old_history_survives(app, &states, "parametric");
}

#[test]
fn b03_opening_a_file_still_starts_a_fresh_history() {
    let (mut app, _) = edited_demo();
    app.open_model(std::path::Path::new(&demo_path())).unwrap();
    assert_eq!(app.undo_len(), 0, "Open is not undoable: the history goes");
}

#[test]
fn b05_refused_actions_are_logged_as_errors() {
    use seacraft_gui::model_menu::ModelAction;
    use seacraft_gui::panels::{InspectorAction, LayerAction};
    let mut app = App::headless_for_test(Some(&demo_path()));
    let errors = |app: &App| app.messages().count(Level::Error);
    assert_eq!(errors(&app), 0);

    app.apply_model_action(ModelAction::SplitFace(usize::MAX));
    assert_eq!(errors(&app), 1, "Model menu: {}", app.status());
    assert_eq!(app.status_level(), Level::Error);

    app.apply_inspector_action(InspectorAction::Move {
        index: usize::MAX,
        to: Vec3::new(0.0, 0.0, 0.0),
    });
    assert_eq!(errors(&app), 2, "Inspector: {}", app.status());
    assert!(app.status().contains("index"), "{}", app.status());

    app.apply_layer_action(LayerAction::Delete {
        layer: 0,
        move_faces_to: Some(0),
    });
    assert_eq!(errors(&app), 3, "Layers: {}", app.status());
    assert_eq!(app.status_level(), Level::Error);
    assert_eq!(app.undo_len(), 0, "nothing was changed");
}

#[test]
fn b06_a_requirements_only_apply_is_one_undo_step() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    // An older geometry edit, which must not be what Ctrl+Z undoes next.
    let p = app.surface().unwrap().points[5].p + Vec3::new(0.0, 0.0, 0.05);
    app.move_point(5, p).unwrap();
    let edited = points(&app);
    let undo = app.undo_len();
    let (name, designer, density) = {
        let st = &app.surface().unwrap().settings;
        (st.name.clone(), st.designer.clone(), st.water_density)
    };
    let before = app.project().requirements.clone();
    let req = Requirements {
        design_speed_kn: Some(14.0),
        ..Default::default()
    };
    app.apply_project_settings(name.clone(), designer.clone(), density, Some(req.clone()));
    assert_eq!(app.project().requirements, Some(req.clone()));
    assert!(app.is_dirty());
    assert_eq!(app.undo_len(), undo + 1, "the Apply is one undo step");

    app.undo();
    assert_eq!(app.project().requirements, before, "undo restores them");
    assert_eq!(points(&app), edited, "and leaves the geometry edit alone");
    app.redo();
    assert_eq!(app.project().requirements, Some(req.clone()));
    assert_eq!(points(&app), edited);

    // Settings and requirements in one Apply: one step undoes both.
    app.apply_project_settings("Renamed".into(), designer, density, None);
    assert_eq!(app.undo_len(), undo + 2);
    app.undo();
    assert_eq!(app.surface().unwrap().settings.name, name);
    assert_eq!(app.project().requirements, Some(req));
}

#[test]
fn b07_an_unwritable_recent_list_is_a_logged_warning_once() {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-b07-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    // The storage's folder is a plain file: nothing can be written under it.
    let blocker = dir.join("not-a-dir");
    std::fs::write(&blocker, "x").unwrap();
    let model = dir.join("hull.fef");
    std::fs::copy(demo_path(), &model).unwrap();

    let mut app = App::headless_empty();
    app.set_storage(Box::new(JsonFileStorage::open(blocker.join("s.json"))));
    app.open_model(&model).unwrap();
    let warnings: Vec<String> = app
        .messages()
        .entries()
        .filter(|m| m.level == Level::Warning)
        .map(|m| m.text.clone())
        .collect();
    assert_eq!(warnings.len(), 1, "{warnings:?}");
    assert!(warnings[0].contains("recent"), "{}", warnings[0]);
    // The list still works for this run, and the same failure is not
    // repeated on every open.
    assert_eq!(app.recent_files().paths()[0], model);
    app.open_model(&model).unwrap();
    assert_eq!(app.messages().count(Level::Warning), 1);
    let _ = std::fs::remove_dir_all(&dir);
}
