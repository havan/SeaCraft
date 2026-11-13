//! File > Builder templates, headless: the window previews the PDF on a job, never shows a
//! count for old options or an old model, exports what it previewed, and shows a failure.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use seacraft_gui::app::ExportKind;
use seacraft_gui::commands::TemplatesParams;
use seacraft_gui::App;

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-templates-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// A demo model at level 2, which these tests cut quickly.
fn demo() -> App {
    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(2);
    app
}

/// One headless window whose widgets are read from egui's AccessKit tree.
struct Window {
    ctx: egui::Context,
    app: App,
    /// Role, label, whether it is disabled, and where.
    widgets: Vec<(Role, String, bool, egui::Rect)>,
    /// Every text painted in the last frame, egui's own warnings included.
    painted: Vec<String>,
}

impl Window {
    fn new(app: App) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut w = Self {
            ctx,
            app,
            widgets: Vec::new(),
            painted: Vec::new(),
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
        fn walk(shape: &egui::Shape, texts: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, texts)),
                egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
                _ => {}
            }
        }
        self.painted.clear();
        for clipped in &out.shapes {
            walk(&clipped.shape, &mut self.painted);
        }
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
                            // A button's text is its label, a plain label's its value.
                            node.label()
                                .or_else(|| node.value())
                                .unwrap_or("")
                                .to_owned(),
                            node.is_disabled(),
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

    fn button(&self, label: &str) -> Option<(bool, egui::Rect)> {
        self.widgets
            .iter()
            .find(|(r, l, _, _)| *r == Role::Button && l == label)
            .map(|(_, _, disabled, rect)| (*disabled, *rect))
    }

    fn click(&mut self, label: &str) {
        let (_, rect) = self
            .button(label)
            .unwrap_or_else(|| panic!("no `{label}` button: {:?}", self.widgets));
        let pos = rect.center();
        let press = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos), press(true)]);
        self.frame(vec![press(false)]);
        self.frame(vec![]);
    }

    /// Draw frames until `done`, for at most a minute.
    fn until(&mut self, what: &str, done: impl Fn(&App) -> bool) {
        let start = Instant::now();
        while !done(&self.app) {
            assert!(
                start.elapsed() < Duration::from_secs(60),
                "timed out: {what}"
            );
            std::thread::sleep(Duration::from_millis(10));
            self.frame(vec![]);
        }
    }
}

#[test]
fn the_file_menu_offers_builder_templates_only_with_a_model() {
    let mut empty = Window::new(App::headless_empty());
    empty.click("File");
    let (disabled, _) = empty
        .button("Builder templates…")
        .expect("in the File menu");
    assert!(disabled, "no model, nothing to cut");

    let mut w = Window::new(demo());
    w.click("File");
    let (disabled, _) = w.button("Builder templates…").expect("in the File menu");
    assert!(!disabled);
    w.click("Builder templates…");
    assert!(w.app.show_templates());
}

#[test]
fn the_window_shows_the_page_count_once_it_is_computed() {
    let mut w = Window::new(demo());
    assert_eq!(
        w.app.templates_pages(),
        None,
        "nothing computed while closed"
    );
    w.app.set_show_templates(true);
    w.until("the first count", |app| app.templates_pages().is_some());
    let shown = w.app.templates_pages().unwrap();

    // The same set, cut and counted directly.
    let surface = w.app.surface().unwrap().clone();
    let set = seacraft_core::templates::template_set(
        &surface,
        &seacraft_core::templates::SetOptions {
            subdivision: seacraft_core::subdiv::SubdivisionOptions {
                level: 2,
                mode: w.app.subdivision_mode(),
            },
            ..Default::default()
        },
    )
    .unwrap();
    let setup = seacraft_core::sheet::PageSetup::default();
    assert_eq!(shown, seacraft_core::sheet::page_count(&set.sheets, &setup));
    assert!(
        w.widgets
            .iter()
            .any(|(_, l, _, _)| *l == format!("{shown} pages, 11 patterns")),
        "the status line"
    );
}

#[test]
fn a_changed_option_or_model_is_never_shown_with_the_old_count() {
    let mut w = Window::new(demo());
    w.app.set_show_templates(true);
    w.until("the first count", |app| app.templates_pages().is_some());

    let mut params = w.app.templates_params().clone();
    params.skin_mm = 6.0;
    w.app.set_templates_params(params);
    assert_eq!(w.app.templates_pages(), None, "stale at once");
    w.frame(vec![]);
    assert_eq!(w.app.templates_pages(), None);
    w.until("the count for a 6 mm skin", |app| {
        app.templates_pages().is_some()
    });

    let p = w.app.surface().unwrap().points[5].p;
    w.app
        .move_point(5, p + seacraft_core::math::Vec3::new(0.0, 0.0, 0.05))
        .unwrap();
    assert_eq!(w.app.templates_pages(), None, "stale at once");
    w.frame(vec![]);
    assert_eq!(w.app.templates_pages(), None);
    w.until("the count for the moved point", |app| {
        app.templates_pages().is_some()
    });
}

#[test]
fn export_writes_the_previewed_document() {
    let mut w = Window::new(demo());
    w.app.set_show_templates(true);
    w.until("the preview", |app| app.templates_pages().is_some());
    let pdf = scratch("m.pdf");
    w.app.export_templates_to(&pdf).unwrap();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    assert!(w
        .app
        .messages()
        .entries()
        .any(|m| m.text.starts_with("exported") && m.text.contains("m.pdf")));
    let dxf = scratch("m.dxf");
    w.app.export_templates_to(&dxf).unwrap();
    assert!(std::fs::read_to_string(&dxf).unwrap().contains("CUT"));
    assert!(w.app.export_templates_to(&scratch("m.png")).is_err());
}

#[test]
fn export_opens_a_file_dialog_that_shares_no_widget_with_the_window() {
    let mut w = Window::new(demo());
    w.app.set_show_templates(true);
    w.until("the preview", |app| app.templates_pages().is_some());
    w.click("Export…");
    // A new window is laid out unseen on its first frame.
    w.frame(vec![]);
    assert!(w.button("OK").is_some(), "the file dialog is open");
    let clashes: Vec<&String> = w.painted.iter().filter(|t| t.contains("use of")).collect();
    assert!(clashes.is_empty(), "egui reports clashing ids: {clashes:?}");
}

#[test]
fn a_failure_is_shown_not_hidden() {
    let mut w = Window::new(demo());
    w.app.set_show_templates(true);
    w.until("the preview", |app| app.templates_pages().is_some());
    let params = TemplatesParams {
        strongback_z: Some(0.1),
        ..w.app.templates_params().clone()
    };
    w.app.set_templates_params(params);
    w.until("the failure", |app| app.templates_failure().is_some());
    let reason = w.app.templates_failure().unwrap().to_owned();
    assert!(reason.contains("strongback"), "{reason}");
    assert_eq!(w.app.templates_pages(), None);
    w.frame(vec![]);
    assert!(
        w.widgets.iter().any(|(_, l, _, _)| *l == reason),
        "the reason is on screen"
    );
    let (disabled, _) = w.button("Export…").expect("the Export button");
    assert!(disabled, "nothing to export");
}

#[test]
fn offsets_pdf_is_an_export_kind() {
    assert!(ExportKind::ALL.contains(&ExportKind::OffsetsPdf));
    let mut app = demo();
    let path = scratch("offsets.pdf");
    assert!(
        app.export_to(ExportKind::OffsetsPdf, &path),
        "{}",
        app.status()
    );
    assert!(std::fs::read(&path).unwrap().starts_with(b"%PDF-"));
}
