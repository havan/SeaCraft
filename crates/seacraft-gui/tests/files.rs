//! Save and export round-trips, driven through the app's public file API.

use seacraft_core::fef;
use seacraft_gui::app::ExportKind;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn saving_a_fef_round_trips() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let (points, faces) = (app.control_point_count(), app.face_count());
    let path = scratch("rt.fef");
    assert!(app.save_to(&path), "save failed: {}", app.status());
    assert!(!app.is_dirty(), "a save clears the dirty flag");

    let text = std::fs::read_to_string(&path).unwrap();
    let back = fef::parse(&text).unwrap();
    assert_eq!(back.points.len(), points);
    assert_eq!(back.faces.len(), faces);
    assert_eq!(app.surface().unwrap().settings.name, back.settings.name);
}

#[test]
fn every_export_format_writes_a_plausible_file() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    for kind in ExportKind::ALL {
        let path = scratch(&format!("hull.{}", kind.extension()));
        assert!(app.export_to(kind, &path), "{kind:?}: {}", app.status());
        let bytes = std::fs::read(&path).unwrap_or_else(|e| panic!("{kind:?}: {e}"));
        assert!(!bytes.is_empty(), "{kind:?} wrote nothing");
        let head = String::from_utf8_lossy(&bytes[..bytes.len().min(96)]);
        match kind {
            ExportKind::Obj => assert!(head.starts_with("# SeaCraft"), "{head}"),
            ExportKind::Ply => assert!(head.starts_with("ply\n"), "{head}"),
            ExportKind::Html => {
                assert!(head.to_lowercase().starts_with("<!doctype html"), "{head}")
            }
            ExportKind::Linesplan => assert!(head.starts_with("<?xml"), "{head}"),
            ExportKind::LinesplanDxf => assert!(head.starts_with("  0\nSECTION"), "{head}"),
            // The Start section is free text; the Global section follows.
            ExportKind::LinesplanIges | ExportKind::HullIges => {
                assert!(head.starts_with("SeaCraft"), "{head}");
                assert!(bytes.windows(2).any(|w| w == b"1H"), "no Global section");
            }
            ExportKind::Stl => {
                // 80-byte header, then a u32 triangle count that matches the size.
                assert!(bytes.len() > 84);
                let tris =
                    u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
                assert_eq!(bytes.len(), 84 + tris * 50, "binary STL size");
                assert!(tris > 1000, "the refined hull has faces: {tris}");
            }
            ExportKind::ThreeMf => {
                assert!(
                    bytes.starts_with(b"PK\x03\x04"),
                    "3MF must be a ZIP archive"
                );
                assert!(
                    bytes.windows(16).any(|w| w == b"3D/3dmodel.model"),
                    "3MF must contain 3D/3dmodel.model"
                );
            }
        }
    }
}

#[test]
fn editing_marks_the_model_dirty_and_a_new_demo_does_too() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert!(!app.is_dirty());
    // A saved path is known, so `save` writes without a dialog.
    let path = scratch("dirty.fef");
    assert!(app.save_to(&path));
    assert!(!app.is_dirty());
    app.load_demo();
    assert!(app.is_dirty(), "a new demo model is unsaved work");
}

#[test]
fn undo_back_to_the_saved_state_is_clean_again() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert!(!app.is_dirty());
    app.snapshot_undo();
    app.surface_mut().unwrap().points[3].p.z += 0.1;
    app.mark_geometry_changed();
    assert!(app.is_dirty());
    app.undo();
    assert!(!app.is_dirty(), "undo returned to the opened model");
    app.redo();
    assert!(app.is_dirty(), "redo re-applies the edit");

    // After a save the saved state moves: undo to the opened model is now
    // an unsaved change, and redo back to the saved one is clean.
    let path = scratch("undo-clean.fef");
    assert!(app.save_to(&path), "{}", app.status());
    app.undo();
    assert!(app.is_dirty(), "the undone edit is no longer on disk");
    app.redo();
    assert!(!app.is_dirty(), "redo returned to the saved model");

    // A project edit is not undone by the model history.
    let mut project = app.project().clone();
    project.compartments.clear();
    project.windage.push(seacraft_core::weather::WindageItem {
        name: "Mast".into(),
        area_m2: 1.0,
        z_centroid: 2.0,
        profile: None,
    });
    app.set_project(project);
    app.undo();
    app.redo();
    assert!(app.is_dirty(), "the project differs from the saved one");
}

#[test]
fn the_project_sidecar_travels_with_the_model() {
    use seacraft_core::project::{sidecar_path, Project};
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert!(app.project().is_empty(), "the demo has no sidecar");

    // An empty project writes no sidecar.
    let bare = scratch("bare.fef");
    assert!(app.save_to(&bare), "{}", app.status());
    assert!(!sidecar_path(&bare).exists());

    let case = format!(
        "{}/../../examples/demo-loadcase.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut project = Project::default();
    project
        .loading_conditions
        .push(serde_json::from_str(&std::fs::read_to_string(case).unwrap()).unwrap());
    app.set_project(project.clone());
    assert!(app.is_dirty(), "a project edit is unsaved work");

    let path = scratch("with-project.fef");
    assert!(app.save_to(&path), "{}", app.status());
    assert_eq!(Project::load(&sidecar_path(&path)).unwrap(), project);

    let mut again = App::headless_for_test(None);
    again.open_model(&path).unwrap();
    assert_eq!(again.project(), &project);
    assert!(again.status().ends_with("+ project"), "{}", again.status());

    // A broken sidecar does not stop the model opening, and a save leaves it alone.
    std::fs::write(sidecar_path(&path), "{ not json").unwrap();
    again.open_model(&path).unwrap();
    assert!(again.project().is_empty());
    assert!(
        again.status().contains("project file not loaded"),
        "{}",
        again.status()
    );
    assert!(again.save_to(&path), "{}", again.status());
    assert_eq!(
        std::fs::read_to_string(sidecar_path(&path)).unwrap(),
        "{ not json"
    );

    // A new demo starts with an empty project.
    app.load_demo();
    assert!(app.project().is_empty());
}

#[test]
fn a_project_with_duplicate_names_is_not_saved() {
    use seacraft_core::project::{sidecar_path, Project};
    let mut app = App::headless_for_test(Some(&demo_path()));
    let case = format!(
        "{}/../../examples/demo-loadcase.json",
        env!("CARGO_MANIFEST_DIR")
    );
    let cond: seacraft_core::loading::LoadingCondition =
        serde_json::from_str(&std::fs::read_to_string(case).unwrap()).unwrap();
    app.set_project(Project {
        loading_conditions: vec![cond.clone(), cond],
        ..Default::default()
    });
    let path = scratch("dup.fef");
    assert!(
        !app.save_to(&path),
        "a sidecar that would not load back must not be written"
    );
    assert!(app.status().contains("named"), "{}", app.status());
    assert!(!sidecar_path(&path).exists());
}

#[test]
fn design_data_is_not_marked_saved_when_the_sidecar_was_unreadable() {
    use seacraft_core::project::sidecar_path;
    let path = scratch("broken-sidecar.fef");
    std::fs::copy(demo_path(), &path).unwrap();
    std::fs::write(sidecar_path(&path), "{ not json").unwrap();
    let mut app = App::headless_for_test(None);
    app.open_model(&path).unwrap();
    // Design data entered in a panel (which edits the project in place).
    app.project_mut_for_test()
        .windage
        .push(seacraft_core::weather::WindageItem {
            name: "Mast".into(),
            area_m2: 1.0,
            z_centroid: 2.0,
            profile: None,
        });
    assert!(app.save_to(&path), "{}", app.status());
    assert!(
        app.is_dirty(),
        "the windage item is not on disk: {}",
        app.status()
    );
    assert!(app.status().contains("NOT saved"), "{}", app.status());
    assert_eq!(
        std::fs::read_to_string(sidecar_path(&path)).unwrap(),
        "{ not json"
    );
}

#[test]
fn the_linesplan_export_uses_the_viewport_settings() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.display_mut().combs = true;
    let path = scratch("combs.svg");
    assert!(
        app.export_to(ExportKind::Linesplan, &path),
        "{}",
        app.status()
    );
    let svg = std::fs::read_to_string(&path).unwrap();
    assert!(
        svg.contains("class=\"comb\""),
        "combs drawn as in the viewports"
    );
}
