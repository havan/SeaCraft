//! Closing the window, headless: the window manager's close (×, Alt+F4)
//! goes through the unsaved-changes prompt as File > Quit does, and the
//! prompt's "Save as" carries the quit out once the file is written.
//!
//! The prompt's buttons are found by label in egui's AccessKit tree and
//! clicked with synthetic pointer events, as a user would.

use eframe::egui;
use egui::accesskit::Role;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-close-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// One headless window: the app, its context, and what the last frame drew
/// and asked of the window.
struct Window {
    ctx: egui::Context,
    app: App,
    /// Role, label, current text and rectangle of every widget drawn.
    widgets: Vec<(Role, String, String, egui::Rect)>,
    commands: Vec<egui::ViewportCommand>,
}

impl Window {
    fn new(app: App) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut w = Self {
            ctx,
            app,
            widgets: Vec::new(),
            commands: Vec::new(),
        };
        w.frame(vec![], false);
        w.frame(vec![], false);
        w
    }

    /// Draw one frame; `close` delivers the window manager's close request.
    fn frame(&mut self, events: Vec<egui::Event>, close: bool) {
        let mut input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        };
        if close {
            input
                .viewports
                .entry(egui::ViewportId::ROOT)
                .or_default()
                .events
                .push(egui::ViewportEvent::Close);
        }
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
        self.commands = out
            .viewport_output
            .get(&egui::ViewportId::ROOT)
            .map(|v| v.commands.clone())
            .unwrap_or_default();
        self.widgets = out
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let b = node.bounds()?;
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(b.x0 as f32, b.y0 as f32),
                            egui::pos2(b.x1 as f32, b.y1 as f32),
                        );
                        Some((
                            node.role(),
                            node.label().unwrap_or("").to_owned(),
                            node.value().unwrap_or("").to_owned(),
                            rect,
                        ))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn find(&self, role: Role, label: &str) -> Option<egui::Rect> {
        self.widgets
            .iter()
            .find(|(r, l, _, _)| *r == role && l == label)
            .map(|(_, _, _, rect)| *rect)
    }

    fn click_at(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos), button(true)], false);
        self.frame(vec![button(false)], false);
    }

    /// The text field currently showing `text`.
    fn field_with(&self, text: impl Fn(&str) -> bool) -> Option<egui::Rect> {
        self.widgets
            .iter()
            .find(|(r, _, v, _)| *r == Role::TextInput && text(v))
            .map(|(_, _, _, rect)| *rect)
    }

    fn click(&mut self, label: &str) {
        // A window shown this frame is laid out on the next.
        self.frame(vec![], false);
        let rect = self.find(Role::Button, label).unwrap_or_else(|| {
            panic!(
                "no `{label}` button among {:?}",
                self.widgets
                    .iter()
                    .filter(|(r, _, _, _)| *r == Role::Button)
                    .map(|(_, l, _, _)| l)
                    .collect::<Vec<_>>()
            )
        });
        self.click_at(rect.center());
    }

    fn closing(&self) -> bool {
        self.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::Close))
    }

    fn cancelled_close(&self) -> bool {
        self.commands
            .iter()
            .any(|c| matches!(c, egui::ViewportCommand::CancelClose))
    }
}

/// The demo, opened from a scratch copy and edited so it has unsaved work.
fn dirty_model(name: &str) -> (App, std::path::PathBuf) {
    let path = scratch(name);
    std::fs::copy(demo_path(), &path).unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.snapshot_undo();
    app.surface_mut().unwrap().points[3].p.z += 0.1;
    app.mark_geometry_changed();
    assert!(app.is_dirty());
    (app, path)
}

#[test]
fn closing_a_clean_model_closes_directly() {
    let mut w = Window::new(App::headless_for_test(Some(&demo_path())));
    assert!(!w.app.is_dirty());
    w.frame(vec![], true);
    assert!(!w.cancelled_close(), "{:?}", w.commands);
    assert!(!w.app.unsaved_prompt_open());
}

#[test]
fn closing_with_unsaved_work_asks_first_and_discard_closes() {
    let (app, path) = dirty_model("discard.fef");
    let before = std::fs::read_to_string(&path).unwrap();
    let mut w = Window::new(app);

    w.frame(vec![], true);
    assert!(w.cancelled_close(), "the close is held: {:?}", w.commands);
    assert!(!w.closing());
    assert!(w.app.unsaved_prompt_open());

    // The prompt stays until answered; nothing closes meanwhile.
    w.frame(vec![], false);
    assert!(w.app.unsaved_prompt_open());
    assert!(!w.closing());

    w.click("Discard");
    assert!(w.closing(), "Discard closes the window: {:?}", w.commands);
    assert!(!w.app.unsaved_prompt_open());
    // The confirmed close comes back as a close request; it is not held.
    w.frame(vec![], true);
    assert!(!w.cancelled_close(), "{:?}", w.commands);
    assert!(!w.app.unsaved_prompt_open());
    assert_eq!(
        std::fs::read_to_string(&path).unwrap(),
        before,
        "nothing saved"
    );
    let _ = std::fs::remove_file(path);
}

/// Discard answers for the action that replaces the model. The New design
/// dialog that follows can still be cancelled, so until a model is actually
/// replaced the edits stay unsaved and Quit still asks.
#[test]
fn discard_before_a_dialog_keeps_the_edits_unsaved() {
    let (mut app, path) = dirty_model("discard-dialog.fef");
    app.request_new_design();
    let mut w = Window::new(app);
    assert!(w.app.unsaved_prompt_open());
    w.click("Discard");
    assert!(!w.app.unsaved_prompt_open());
    assert!(w.app.is_dirty(), "no model was replaced yet");
    let _ = std::fs::remove_file(path);
}

#[test]
fn cancel_keeps_the_window_and_save_writes_then_closes() {
    let (app, path) = dirty_model("save.fef");
    let before = std::fs::read_to_string(&path).unwrap();
    let mut w = Window::new(app);

    w.frame(vec![], true);
    assert!(w.app.unsaved_prompt_open());
    w.click("Cancel");
    assert!(!w.app.unsaved_prompt_open());
    assert!(!w.closing());
    assert!(w.app.is_dirty(), "the edit is still there");

    w.frame(vec![], true);
    assert!(w.cancelled_close());
    w.frame(vec![], false);
    w.click("Save");
    assert!(
        w.closing(),
        "Save on a titled model closes: {:?}",
        w.commands
    );
    assert!(!w.app.is_dirty());
    assert_ne!(std::fs::read_to_string(&path).unwrap(), before, "saved");
    let _ = std::fs::remove_file(path);
}

/// Round-3 C06: an untitled model's prompt hands over to "Save as"; once the
/// file is written, the pending action (here the quit) is carried out.
#[test]
fn save_as_from_the_prompt_writes_then_continues_the_quit() {
    let mut app = App::headless_for_test(None);
    app.load_demo();
    assert!(app.is_dirty(), "a new demo is unsaved work");
    let mut w = Window::new(app);

    w.frame(vec![], true);
    assert!(w.cancelled_close());
    w.frame(vec![], false);
    w.click("Save");
    assert!(w.app.dialog_open(), "an untitled model asks for a path");
    assert!(!w.app.unsaved_prompt_open(), "the prompt handed over");
    assert!(!w.closing(), "nothing is written yet");

    // Type the path into the dialog and accept it.
    let target = scratch("save-as.fef");
    let _ = std::fs::remove_file(&target);
    // The dialog's first frame is an invisible sizing pass.
    w.frame(vec![], false);
    w.frame(vec![], false);
    let field = w.field_with(|v| v.ends_with(".fef")).unwrap_or_else(|| {
        panic!(
            "the path field shows the suggested .fef path: {:?}",
            w.widgets
        )
    });
    w.click_at(field.center());
    let path = target.to_string_lossy().to_string();
    w.frame(
        vec![
            egui::Event::Key {
                key: egui::Key::A,
                physical_key: None,
                pressed: true,
                repeat: false,
                modifiers: egui::Modifiers::COMMAND,
            },
            egui::Event::Text(path.clone()),
        ],
        false,
    );
    w.frame(vec![], false);
    // Never accept a path other than the scratch one: the suggested path is
    // relative to the working directory.
    assert!(
        w.field_with(|v| v == path).is_some(),
        "the path was typed: {:?}",
        w.widgets
            .iter()
            .filter(|(r, _, _, _)| *r == Role::TextInput)
            .map(|(_, _, v, _)| v)
            .collect::<Vec<_>>()
    );
    w.click("OK");
    assert!(
        target.exists(),
        "Save as wrote {}: {}",
        target.display(),
        w.app.status()
    );
    assert!(!w.app.is_dirty());
    assert!(!w.app.dialog_open());
    assert!(
        w.closing(),
        "the quit continues after saving: {:?}",
        w.commands
    );
    let back = seacraft_core::fef::parse(&std::fs::read_to_string(&target).unwrap()).unwrap();
    assert_eq!(back.points.len(), w.app.control_point_count());
    let _ = std::fs::remove_file(target);
}
