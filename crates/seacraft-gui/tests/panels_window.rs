//! File > Builder panels, headless: the window measures the panels on a job, lists them, never
//! shows a list for old options or an old model as current, and says why when it cannot.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use seacraft_gui::App;

/// The test pram (a hard-chine fixture, not an example hull) at level 2.
fn pram(test: &str, bow_m: f64) -> App {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-panels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // One file a test: tests run in parallel, and one must not read another's half-written file.
    let path = dir.join(format!("{test}.fef"));
    let surface = seacraft_core::panels::pram_fixture(0.0, bow_m);
    std::fs::write(&path, seacraft_core::fef::write(&surface)).unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
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
        self.click_at(rect.center());
    }

    fn click_at(&mut self, pos: egui::Pos2) {
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

impl Window {
    fn shows(&self, text: &str) -> bool {
        self.painted.iter().any(|t| t.contains(text))
            || self.widgets.iter().any(|(_, l, _, _)| l.contains(text))
    }
}

#[test]
fn the_file_menu_offers_builder_panels_only_with_a_model() {
    let mut empty = Window::new(App::headless_empty());
    empty.click("File");
    let (disabled, _) = empty.button("Builder panels…").expect("in the File menu");
    assert!(disabled, "no model, no panels");

    let mut w = Window::new(pram(
        "the_file_menu_offers_builder_panels_only_with_a_model",
        0.0,
    ));
    w.click("File");
    let (disabled, _) = w.button("Builder panels…").expect("in the File menu");
    assert!(!disabled);
    w.click("Builder panels…");
    assert!(w.app.show_panels());
}

#[test]
fn the_list_appears_once_measured() {
    let mut w = Window::new(pram("the_list_appears_once_measured", 0.0));
    assert!(
        w.app.panels_preview().is_none(),
        "nothing measured while closed"
    );
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    let set = w.app.panels_preview().unwrap().set.clone();
    let names: Vec<&str> = set.rows.iter().map(|r| r.panel.as_str()).collect();
    assert_eq!(names, ["Bottom", "Side"]);
    for name in names {
        assert!(w.shows(name), "{name} listed");
    }
    assert!(w.shows(&set.sheet_count_line()), "the plywood count");
    assert!(!w.shows("Out of date"));
}

#[test]
fn a_changed_option_or_model_is_never_shown_as_current() {
    let mut w = Window::new(pram(
        "a_changed_option_or_model_is_never_shown_as_current",
        0.0,
    ));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());

    let mut params = w.app.panels_params().clone();
    params.choices.max_stretch_pct = 0.0001;
    w.app.set_panels_params(params);
    assert!(w.app.panels_preview().is_none(), "stale at once");
    w.frame(vec![]);
    assert!(w.shows("Out of date"), "the old list is marked");
    w.until("the panels for the new limit", |app| {
        app.panels_preview().is_some()
    });
    w.frame(vec![]);
    assert!(!w.shows("Out of date"));

    let p = w.app.surface().unwrap().points[5].p;
    w.app
        .move_point(5, p + seacraft_core::math::Vec3::new(0.0, 0.0, 0.01))
        .unwrap();
    assert!(w.app.panels_preview().is_none(), "stale at once");
    w.until("the panels for the moved point", |app| {
        app.panels_preview().is_some()
    });
}

#[test]
fn a_panel_that_does_not_lie_flat_says_so_in_the_list() {
    let mut w = Window::new(pram(
        "a_panel_that_does_not_lie_flat_says_so_in_the_list",
        0.08,
    ));
    let mut params = w.app.panels_params().clone();
    params.choices.max_stretch_pct = 0.0001;
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    assert!(w.shows("does not lie flat"), "{:?}", w.painted);
}

#[test]
fn a_failure_is_shown_not_hidden() {
    let mut w = Window::new(pram("a_failure_is_shown_not_hidden", 0.0));
    let mut params = w.app.panels_params().clone();
    params.layers = Some(vec!["No such layer".into()]);
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the failure", |app| app.panels_failure().is_some());
    w.frame(vec![]);
    let reason = w.app.panels_failure().unwrap().to_owned();
    assert!(reason.contains("No such layer"), "{reason}");
    assert!(w.shows(&reason), "{:?}", w.painted);
}
