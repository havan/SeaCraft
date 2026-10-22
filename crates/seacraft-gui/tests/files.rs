//! Save and export round-trips, driven through the app's public file API.

use seacraft_core::fef;
use seacraft_gui::app::ExportKind;
use seacraft_gui::App;

fn demo_path() -> String {
    format!("{}/../../examples/demo-hull.fef", env!("CARGO_MANIFEST_DIR"))
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
            ExportKind::Html => assert!(head.to_lowercase().starts_with("<!doctype html"), "{head}"),
            ExportKind::Linesplan => assert!(head.starts_with("<?xml"), "{head}"),
            ExportKind::LinesplanDxf => assert!(head.starts_with("  0\nSECTION"), "{head}"),
            // The Start section is free text; the Global section follows.
            ExportKind::LinesplanIges => {
                assert!(head.starts_with("SeaCraft"), "{head}");
                assert!(bytes.windows(2).any(|w| w == b"1H"), "no Global section");
            }
            ExportKind::Stl => {
                // 80-byte header, then a u32 triangle count that matches the size.
                assert!(bytes.len() > 84);
                let tris = u32::from_le_bytes([bytes[80], bytes[81], bytes[82], bytes[83]]) as usize;
                assert_eq!(bytes.len(), 84 + tris * 50, "binary STL size");
                assert!(tris > 1000, "the refined hull has faces: {tris}");
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
