//! The message log, headless (R5-9): a failed open logs a typed error and
//! leaves the model untouched; the open dialog decides failure from the
//! typed result and stays open on the error; save, export and job failures
//! are logged; a click on the status text opens the Messages window, whose
//! Clear empties the log.

use eframe::egui;
use egui::accesskit::Role;
use seacraft_gui::app::{ExportKind, OpenError};
use seacraft_gui::messages::Level;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-messages-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// One headless window whose widgets are read from egui's AccessKit tree.
struct Window {
    ctx: egui::Context,
    app: App,
    widgets: Vec<(Role, String, String, egui::Rect)>,
}

impl Window {
    fn new(app: App) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut w = Self {
            ctx,
            app,
            widgets: Vec::new(),
        };
        w.frame(vec![]);
        w.frame(vec![]);
        w
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
        self.widgets = out
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let b = node.bounds()?;
                        Some((
                            node.role(),
                            node.label().unwrap_or("").to_owned(),
                            node.value().unwrap_or("").to_owned(),
                            egui::Rect::from_min_max(
                                egui::pos2(b.x0 as f32, b.y0 as f32),
                                egui::pos2(b.x1 as f32, b.y1 as f32),
                            ),
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn click_at(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
        self.frame(vec![button(false)]);
        self.frame(vec![]);
    }

    fn click(&mut self, label: &str) {
        let rect = self
            .widgets
            .iter()
            .find(|(r, l, _, _)| *r == Role::Button && l == label)
            .map(|(_, _, _, rect)| *rect)
            .unwrap_or_else(|| panic!("no `{label}` button: {:?}", self.widgets));
        self.click_at(rect.center());
    }

    fn field_with(&self, text: impl Fn(&str) -> bool) -> Option<egui::Rect> {
        self.widgets
            .iter()
            .find(|(r, _, v, _)| *r == Role::TextInput && text(v))
            .map(|(_, _, _, rect)| *rect)
    }

    fn key(&mut self, key: egui::Key, modifiers: egui::Modifiers) {
        let ev = |pressed| egui::Event::Key {
            key,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers,
        };
        self.frame(vec![ev(true), ev(false)]);
    }
}

#[test]
fn a_failed_open_logs_a_typed_error_and_leaves_the_model_untouched() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let points = app.control_point_count();
    let path = app.path().map(|p| p.to_path_buf());

    // A file that is not there: a read error.
    let missing = scratch("not-there.fef");
    let _ = std::fs::remove_file(&missing);
    let err = app.open_model(&missing).unwrap_err();
    assert!(matches!(err, OpenError::Read { .. }), "{err:?}");
    // A file that is there but not a model: a parse error.
    let junk = scratch("junk.fef");
    std::fs::write(&junk, "this is not a FREE!ship file").unwrap();
    let err = app.open_model(&junk).unwrap_err();
    assert!(matches!(err, OpenError::Parse { .. }), "{err:?}");

    assert_eq!(app.control_point_count(), points, "the model is untouched");
    assert_eq!(app.path().map(|p| p.to_path_buf()), path);
    assert!(!app.is_dirty());
    let errors: Vec<_> = app
        .messages()
        .entries()
        .filter(|m| m.level == Level::Error)
        .collect();
    assert_eq!(errors.len(), 2, "both failures logged");
    assert!(errors[1].text.contains("junk.fef"), "{}", errors[1].text);
    assert_eq!(app.status_level(), Level::Error, "the status bar shows it");
    assert_eq!(app.status(), errors[1].text);
    // A successful open is logged too, and the status bar is back to info.
    app.open_model(std::path::Path::new(&demo_path())).unwrap();
    assert_eq!(app.status_level(), Level::Info);
    assert_eq!(app.messages().last().unwrap().level, Level::Info);
    let _ = std::fs::remove_file(junk);
}

#[test]
fn the_open_dialog_stays_open_on_the_typed_error() {
    let app = App::headless_for_test(Some(&demo_path()));
    let points = app.control_point_count();
    let mut w = Window::new(app);
    w.key(
        egui::Key::O,
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
    );
    assert!(w.app.dialog_open(), "Ctrl+O opens the file dialog");
    w.frame(vec![]);
    w.frame(vec![]);
    let field = w
        .field_with(|v| v.ends_with(".fef"))
        .unwrap_or_else(|| panic!("the path field: {:?}", w.widgets));
    w.click_at(field.center());
    let junk = scratch("dialog-junk.fef");
    std::fs::write(&junk, "garbage").unwrap();
    let typed = junk.to_string_lossy().to_string();
    w.frame(vec![
        egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        },
        egui::Event::Text(typed.clone()),
    ]);
    w.frame(vec![]);
    assert!(w.field_with(|v| v == typed).is_some(), "the path was typed");
    w.click("OK");
    assert!(w.app.dialog_open(), "a failed open keeps the dialog open");
    assert_eq!(w.app.control_point_count(), points, "model untouched");
    let last = w.app.messages().last().expect("logged");
    assert_eq!(last.level, Level::Error);
    assert!(last.text.starts_with("Parse error in"), "{}", last.text);
    // The dialog shows the same error.
    assert!(
        w.widgets
            .iter()
            .any(|(_, l, v, _)| l == &last.text || v == &last.text),
        "the dialog shows the error"
    );
    let _ = std::fs::remove_file(junk);
}

#[test]
fn save_and_export_failures_are_logged_as_errors() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let bad = std::path::Path::new("/nonexistent-dir-seacraft/x.fef");
    assert!(app.write_model(bad).is_err());
    assert_eq!(app.messages().last().unwrap().level, Level::Error);
    assert!(app
        .write_export(
            ExportKind::Stl,
            std::path::Path::new("/nonexistent-dir-seacraft/x.stl")
        )
        .is_err());
    assert_eq!(app.messages().count(Level::Error), 2);
    // A good save is an info line.
    let ok = scratch("ok.fef");
    assert!(app.save_to(&ok));
    assert_eq!(app.messages().last().unwrap().level, Level::Info);
    assert_eq!(app.status_level(), Level::Info);
    let _ = std::fs::remove_file(ok);
}

#[test]
fn a_failed_job_is_logged() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.start_loading_job_for_test(|| Err("tank table corrupt".to_owned()));
    let ctx = egui::Context::default();
    for _ in 0..200 {
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| app.draw(ui));
        out.textures_delta.clear();
        if app.messages().count(Level::Error) > 0 {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(5));
    }
    let last = app.messages().last().expect("the failure was logged");
    assert_eq!(last.level, Level::Error);
    assert!(last.text.contains("tank table corrupt"), "{}", last.text);
}

#[test]
fn clicking_the_status_text_opens_the_log_and_clear_empties_it() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let _ = app.open_model(&scratch("missing-for-log.fef"));
    let mut w = Window::new(app);
    assert!(!w.app.show_messages());
    let status = w.app.status_rect();
    assert!(status.is_positive(), "the status text was drawn");
    w.click_at(status.center());
    assert!(w.app.show_messages(), "a click on the status opens the log");
    w.frame(vec![]);
    w.click("Clear");
    assert!(w.app.messages().is_empty(), "Clear empties the log");
    // The View menu has the same switch.
    w.app.set_show_messages(false);
    w.frame(vec![]);
    assert!(!w.app.show_messages());
}
