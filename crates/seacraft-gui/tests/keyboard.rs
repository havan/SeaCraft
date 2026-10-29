//! Keyboard shortcuts, headless: single keys never fire while a text field
//! has the keyboard, and the Ctrl shortcuts reach the file and edit actions.

use eframe::egui;
use seacraft_gui::App;

/// One frame of the app, with a text field drawn over it when `field` is
/// given: `Some(true)` focuses the field, `Some(false)` lets it go.
fn frame(
    ctx: &egui::Context,
    app: &mut App,
    text: &mut String,
    field: Option<bool>,
    events: Vec<egui::Event>,
) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        app.draw(ui);
        if let Some(focus) = field {
            egui::Window::new("field").show(ui.ctx(), |ui| {
                let response = ui.text_edit_singleline(text);
                if focus {
                    response.request_focus();
                } else {
                    response.surrender_focus();
                }
            });
        }
    });
    out.textures_delta.clear();
}

fn key(key: egui::Key, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers,
    }
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn cameras(app: &App) -> String {
    format!("{:?}", app.cameras())
}

fn typed(c: &str, k: egui::Key) -> Vec<egui::Event> {
    vec![
        key(k, egui::Modifiers::NONE),
        egui::Event::Text(c.to_owned()),
    ]
}

#[test]
fn single_key_shortcuts_do_not_fire_while_typing() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    let mut text = String::new();
    frame(&ctx, &mut app, &mut text, Some(true), Vec::new());
    frame(&ctx, &mut app, &mut text, Some(true), Vec::new());
    assert!(
        ctx.egui_wants_keyboard_input(),
        "the field has the keyboard"
    );

    // Move a camera off its framing so F would visibly refit it.
    app.cameras_mut()[1].target.x += 3.0;
    app.select_point(Some(0));
    let (before, shaded, active) = (cameras(&app), app.display().shaded, app.active_view());

    for (c, k) in [
        ("f", egui::Key::F),
        (" ", egui::Key::Space),
        ("2", egui::Key::Num2),
    ] {
        frame(&ctx, &mut app, &mut text, Some(true), typed(c, k));
    }
    frame(
        &ctx,
        &mut app,
        &mut text,
        Some(true),
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert_eq!(cameras(&app), before, "F refitted the cameras while typing");
    assert_eq!(app.display().shaded, shaded, "Space toggled shading");
    assert_eq!(app.active_view(), active, "2 switched the view");
    assert_eq!(app.selected(), Some(0), "Escape cleared the selection");
    assert!(text.contains("f 2"), "the field got the text: {text:?}");

    // With the field let go, the same keys act again.
    frame(&ctx, &mut app, &mut text, Some(false), Vec::new());
    frame(&ctx, &mut app, &mut text, None, Vec::new());
    assert!(!ctx.egui_wants_keyboard_input());
    for (c, k) in [
        ("f", egui::Key::F),
        (" ", egui::Key::Space),
        ("2", egui::Key::Num2),
    ] {
        frame(&ctx, &mut app, &mut text, None, typed(c, k));
    }
    assert_ne!(cameras(&app), before, "F refits the cameras");
    assert_eq!(app.display().shaded, !shaded, "Space toggles shading");
    assert_eq!(app.active_view(), 1, "2 picks the profile view");
    frame(
        &ctx,
        &mut app,
        &mut text,
        None,
        vec![key(egui::Key::Escape, egui::Modifiers::NONE)],
    );
    assert_eq!(app.selected(), None, "Escape clears the selection");
}

#[test]
fn ctrl_y_redoes_like_ctrl_shift_z() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    let mut text = String::new();
    frame(&ctx, &mut app, &mut text, None, Vec::new());
    let before = app.surface().unwrap().points[5].p;

    for redo in [
        key(egui::Key::Y, egui::Modifiers::CTRL),
        key(egui::Key::Z, egui::Modifiers::CTRL | egui::Modifiers::SHIFT),
    ] {
        app.snapshot_undo();
        app.surface_mut().unwrap().points[5].p.z += 0.25;
        app.mark_geometry_changed();
        let moved = app.surface().unwrap().points[5].p;

        frame(
            &ctx,
            &mut app,
            &mut text,
            None,
            vec![key(egui::Key::Z, egui::Modifiers::CTRL)],
        );
        assert!((app.surface().unwrap().points[5].p - before).length() < 1e-12);
        assert!(app.can_redo());

        frame(&ctx, &mut app, &mut text, None, vec![redo.clone()]);
        assert!(
            (app.surface().unwrap().points[5].p - moved).length() < 1e-12,
            "{redo:?} redid the edit"
        );
        app.undo();
    }
}

#[test]
fn ctrl_z_in_a_text_field_does_not_undo_the_model() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    let mut text = String::new();
    frame(&ctx, &mut app, &mut text, Some(true), Vec::new());
    app.snapshot_undo();
    app.surface_mut().unwrap().points[5].p.z += 0.25;
    app.mark_geometry_changed();
    let moved = app.surface().unwrap().points[5].p;
    frame(
        &ctx,
        &mut app,
        &mut text,
        Some(true),
        vec![key(egui::Key::Z, egui::Modifiers::CTRL)],
    );
    assert_eq!(app.surface().unwrap().points[5].p, moved);
}

#[test]
fn ctrl_s_saves_to_the_path_and_ctrl_o_and_ctrl_shift_s_open_dialogs() {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-keys-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("keys.fef");
    std::fs::copy(demo_path(), &path).unwrap();

    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    let ctx = egui::Context::default();
    let mut text = String::new();
    frame(&ctx, &mut app, &mut text, None, Vec::new());
    app.surface_mut().unwrap().points[5].p.z += 0.25;
    app.mark_geometry_changed();
    assert!(app.is_dirty());

    frame(
        &ctx,
        &mut app,
        &mut text,
        None,
        vec![key(egui::Key::S, egui::Modifiers::CTRL)],
    );
    assert!(!app.is_dirty(), "Ctrl+S saved: {}", app.status());
    assert!(
        !app.dialog_open(),
        "a model with a path saves without asking"
    );
    let back = seacraft_core::fef::parse(&std::fs::read_to_string(&path).unwrap()).unwrap();
    assert!((back.points[5].p.z - app.surface().unwrap().points[5].p.z).abs() < 1e-9);

    frame(
        &ctx,
        &mut app,
        &mut text,
        None,
        vec![key(
            egui::Key::S,
            egui::Modifiers::CTRL | egui::Modifiers::SHIFT,
        )],
    );
    assert!(app.dialog_open(), "Ctrl+Shift+S asks for a path");

    // The built-in demo has no path yet.
    let mut fresh = App::headless_for_test(None);
    assert!(fresh.path().is_none());
    frame(
        &egui::Context::default(),
        &mut fresh,
        &mut text,
        None,
        vec![key(egui::Key::S, egui::Modifiers::CTRL)],
    );
    assert!(fresh.dialog_open(), "Ctrl+S without a path acts as Save as");

    // A saved model opens straight away (unsaved edits would ask first, as
    // the File menu does).
    let mut saved = App::headless_for_test(Some(path.to_str().unwrap()));
    assert!(!saved.is_dirty());
    frame(
        &egui::Context::default(),
        &mut saved,
        &mut text,
        None,
        vec![key(egui::Key::O, egui::Modifiers::CTRL)],
    );
    assert!(saved.dialog_open(), "Ctrl+O opens the file dialog");
}
