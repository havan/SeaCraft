//! File > Builder panels, headless: the window measures the panels on a job, lists them, never
//! shows a list for old options or an old model as current, and says why when it cannot.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use seacraft_gui::App;

/// The test pram (a hard-chine fixture, not an example hull) at level 2.
fn pram(test: &str, bow_m: f64) -> App {
    let path = pram_file(test, bow_m);
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.set_subdivision_level(2);
    app
}

/// The test pram written to a file of its own.
fn pram_file(test: &str, bow_m: f64) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-panels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // One file a test: tests run in parallel, and one must not read another's half-written file.
    let path = dir.join(format!("{test}.fef"));
    let surface = seacraft_core::panels::pram_fixture(0.0, bow_m);
    std::fs::write(&path, seacraft_core::fef::write(&surface)).unwrap();
    path
}

/// The skiff-4m example (bottom, sides and transom all lie flat) at level 3.
fn skiff(test: &str) -> App {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-panels-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(format!("{test}.fef"));
    let (surface, _) =
        seacraft_core::examples::build(seacraft_core::examples::Example::Skiff4m).expect("skiff");
    std::fs::write(&path, seacraft_core::fef::write(&surface)).unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.set_subdivision_level(3);
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
    /// Whether the stale-drawing veil was painted in the last frame.
    has_stale_veil: bool,
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
            has_stale_veil: false,
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
        fn walk(shape: &egui::Shape, texts: &mut Vec<String>, stale_veil: &mut bool) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, texts, stale_veil)),
                egui::Shape::Text(t) => texts.push(t.galley.text().to_string()),
                egui::Shape::Rect(r) if r.fill == seacraft_gui::theme::PATTERN_STALE_VEIL => {
                    *stale_veil = true;
                }
                _ => {}
            }
        }
        self.painted.clear();
        self.has_stale_veil = false;
        for clipped in &out.shapes {
            walk(&clipped.shape, &mut self.painted, &mut self.has_stale_veil);
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

    fn widget(&self, role: Role, label: &str) -> Option<egui::Rect> {
        self.widgets
            .iter()
            .find(|(r, l, _, _)| *r == role && l == label)
            .map(|(_, _, _, rect)| *rect)
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

#[test]
fn the_drawing_shows_every_panel() {
    let mut w = Window::new(pram("the_drawing_shows_every_panel", 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    assert!(w.shows("Drawing: 2 sheets"), "{:?}", w.widgets);
    assert!(!w.shows("Nothing to draw"));
}

#[test]
fn clicking_a_row_frames_that_panel() {
    let mut w = Window::new(pram("clicking_a_row_frames_that_panel", 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    let fitted = w.app.panels_view().clone();
    // The list's row, not the layer panel's button of the same name: the one inside the window.
    let window = w
        .widgets
        .iter()
        .find(|(_, l, _, _)| l.starts_with("Drawing:"))
        .map(|(_, _, _, r)| *r)
        .expect("the drawing");
    let rect = w
        .widgets
        .iter()
        .filter(|(r, l, _, _)| *r == Role::Button && l == "Side")
        .map(|(_, _, _, r)| *r)
        .find(|r| r.min.x >= window.min.x - 1.0)
        .expect("the Side row");
    w.click_at(rect.center());
    assert_eq!(w.app.panels_selected(), Some(1));
    assert_ne!(
        w.app.panels_view(),
        &fitted,
        "the view moved to the Side panel"
    );
}

#[test]
fn a_panel_that_does_not_lie_flat_is_named_over_the_drawing() {
    let mut w = Window::new(pram(
        "a_panel_that_does_not_lie_flat_is_named_over_the_drawing",
        0.08,
    ));
    let mut params = w.app.panels_params().clone();
    params.choices.max_stretch_pct = 0.0001;
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    let failing = w.app.panels_preview().unwrap().set.failing();
    assert_eq!(failing.len(), 1);
    assert!(w.shows(&failing[0]), "{:?}", w.painted);
}

#[test]
fn no_panels_draws_nothing_to_draw() {
    let mut w = Window::new(pram("no_panels_draws_nothing_to_draw", 0.0));
    let mut params = w.app.panels_params().clone();
    params.layers = Some(vec!["No such layer".into()]);
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the failure", |app| app.panels_failure().is_some());
    w.frame(vec![]);
    assert!(w.shows("Nothing to draw"), "{:?}", w.painted);
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-panels-out-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn export_writes_the_measured_patterns() {
    let mut w = Window::new(pram("export_writes_the_measured_patterns", 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    let set = w.app.panels_preview().unwrap().set.clone();
    let params = w.app.panels_params().clone();
    let pdf = scratch("p.pdf");
    w.app.export_panels_to(&pdf).unwrap();
    let expected = seacraft_gui::commands::panels_file(
        &set,
        &params,
        seacraft_core::templates::Format::PdfTiled,
    )
    .unwrap();
    assert!(expected.offsets_pages >= 1);
    assert_eq!(std::fs::read(&pdf).unwrap(), expected.bytes);
    assert!(w
        .app
        .messages()
        .entries()
        .any(|m| m.text.starts_with("exported") && m.text.contains("p.pdf")));
    let svg = scratch("p.svg");
    w.app.export_panels_to(&svg).unwrap();
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
    assert!(w.app.export_panels_to(&scratch("p.png")).is_err());
}

#[test]
fn export_options_do_not_measure_again() {
    let mut w = Window::new(pram("export_options_do_not_measure_again", 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    let mut params = w.app.panels_params().clone();
    params.offsets_pages = false;
    params.allow_undevelopable = true;
    params.paper = "a3".into();
    w.app.set_panels_params(params);
    assert!(w.app.panels_preview().is_some(), "still current");
}

#[test]
fn export_is_disabled_while_a_panel_does_not_lie_flat() {
    let mut w = Window::new(pram(
        "export_is_disabled_while_a_panel_does_not_lie_flat",
        0.08,
    ));
    let mut params = w.app.panels_params().clone();
    params.choices.max_stretch_pct = 0.0001;
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    let (disabled, _) = w.button("Export…").expect("the Export button");
    assert!(disabled);
    let reason = w.app.panels_export_block().expect("a reason");
    assert!(reason.contains("Side"), "{reason}");
    assert!(w.app.export_panels_to(&scratch("bowed.pdf")).is_err());

    let mut params = w.app.panels_params().clone();
    params.allow_undevelopable = true;
    w.app.set_panels_params(params);
    w.frame(vec![]);
    let (disabled, _) = w.button("Export…").unwrap();
    assert!(!disabled);
}

#[test]
fn export_waits_for_the_current_panels() {
    let mut w = Window::new(pram("export_waits_for_the_current_panels", 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    let mut params = w.app.panels_params().clone();
    params.choices.max_seam_drift_mm = 3.0;
    w.app.set_panels_params(params);
    w.frame(vec![]);
    let (disabled, _) = w.button("Export…").expect("the Export button");
    assert!(disabled, "the list is out of date");
    w.until("the new panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    assert!(!w.button("Export…").unwrap().0);
}

#[test]
fn export_opens_a_file_dialog() {
    let mut w = Window::new(pram("export_opens_a_file_dialog", 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.click("Export…");
    w.frame(vec![]);
    assert!(w.button("OK").is_some(), "the file dialog is open");
    let clashes: Vec<&String> = w.painted.iter().filter(|t| t.contains("use of")).collect();
    assert!(clashes.is_empty(), "egui reports clashing ids: {clashes:?}");
}

#[test]
fn plate_development_opens_builder_panels() {
    let mut w = Window::new(pram("plate_development_opens_builder_panels", 0.0));
    w.app
        .open_analysis_view(seacraft_gui::analysis_views::AnalysisView::Plates);
    w.frame(vec![]);
    w.click("Export patterns…");
    assert!(w.app.show_panels());
}

/// With the window's measurement current for the same options, `build.panels` without a path
/// answers at once from it, with no worker; other options start a worker.
#[test]
fn build_panels_answers_from_the_window_when_it_can() {
    use seacraft_gui::control::{handle, ControlRequest};
    let mut w = Window::new(pram(
        "build_panels_answers_from_the_window_when_it_can",
        0.0,
    ));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    let (tx, _rx) = std::sync::mpsc::channel();
    let req = ControlRequest::new("build.panels", serde_json::json!({}), tx);
    let r = handle(&mut w.app, &w.ctx, &req).expect("answered at once");
    assert_eq!(r["ok"], true, "{r}");
    assert_eq!(
        r["result"]["plywood"],
        w.app.panels_preview().unwrap().set.sheet_count_line()
    );
    let (tx, rx) = std::sync::mpsc::channel();
    let req = ControlRequest::new(
        "build.panels",
        serde_json::json!({ "max_seam_drift_mm": 5 }),
        tx,
    );
    assert!(
        handle(&mut w.app, &w.ctx, &req).is_none(),
        "other options: a worker"
    );
    let r = rx.recv_timeout(Duration::from_secs(60)).unwrap();
    assert_eq!(r["ok"], true, "{r}");
}

/// The options are one line apart: no row of the options grid stretches to another's height.
#[test]
fn the_options_are_one_line_apart() {
    let mut w = Window::new(pram("the_options_are_one_line_apart", 0.0));
    w.app.set_show_panels(true);
    // A new window is laid out unseen on its first frame.
    w.frame(vec![]);
    w.frame(vec![]);
    let y = |label: &str| {
        w.widgets
            .iter()
            .find(|(_, l, _, _)| l == label)
            .map(|(_, _, _, r)| r.min.y)
            .unwrap_or_else(|| panic!("no `{label}`"))
    };
    let rows = [
        "Layers:",
        "Stations:",
        "Seam drift limit:",
        "Stretch limit:",
        "Offsets every:",
    ];
    for pair in rows.windows(2) {
        let gap = y(pair[1]) - y(pair[0]);
        assert!(
            gap > 0.0 && gap < 40.0,
            "{} to {}: {gap} px",
            pair[0],
            pair[1]
        );
    }
}

/// The window measured and shown, then `Side` picked in the drawing's list.
fn measured_with_side_picked(test: &str) -> Window {
    let mut w = Window::new(pram(test, 0.0));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    let window = w
        .widgets
        .iter()
        .find(|(_, l, _, _)| l.starts_with("Drawing:"))
        .map(|(_, _, _, r)| *r)
        .expect("the drawing");
    let row = w
        .widgets
        .iter()
        .filter(|(r, l, _, _)| *r == Role::Button && l == "Side")
        .map(|(_, _, _, r)| *r)
        .find(|r| r.min.x >= window.min.x - 1.0)
        .expect("the Side row");
    w.click_at(row.center());
    assert_eq!(w.app.panels_selected(), Some(1));
    w.frame(vec![]);
    w
}

/// The view a fresh window settles on for `app`'s model and options.
fn fresh_view(mut app: App, params: seacraft_gui::commands::PanelsParams) -> (f32, (f64, f64)) {
    app.set_panels_params(params);
    let mut w = Window::new(app);
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    let v = w.app.panels_view();
    (v.zoom, v.centre)
}

fn assert_same_view(w: &Window, (zoom, centre): (f32, (f64, f64))) {
    let v = w.app.panels_view();
    assert!(
        (v.zoom - zoom).abs() < 1e-3 * zoom
            && (v.centre.0 - centre.0).abs() < 1e-6
            && (v.centre.1 - centre.1).abs() < 1e-6,
        "framed whole: {v:?} against a fresh window's {zoom}, {centre:?}"
    );
}

#[test]
fn opening_another_hull_shows_none_of_the_old_panels() {
    let mut w = measured_with_side_picked("opening_another_hull_shows_none_of_the_old_panels");
    let other = pram_file(
        "opening_another_hull_shows_none_of_the_old_panels-other",
        0.08,
    );
    w.app.open_model(&other).unwrap();
    w.frame(vec![]);
    assert_eq!(w.app.panels_selected(), None, "the old hull's pick is gone");
    assert!(!w.shows("Out of date"), "{:?}", w.painted);
    assert!(!w.shows("Drawing: 2 sheets"), "{:?}", w.widgets);
    assert!(w.shows("Laying the panels flat"), "{:?}", w.painted);

    w.until("the new hull's panels", |app| {
        app.panels_preview().is_some()
    });
    w.frame(vec![]);
    w.frame(vec![]);
    let mut app = App::headless_for_test(Some(other.to_str().unwrap()));
    app.set_subdivision_level(2);
    let params = w.app.panels_params().clone();
    assert_same_view(&w, fresh_view(app, params));
}

#[test]
fn a_new_set_of_sheets_is_framed_whole() {
    let test = "a_new_set_of_sheets_is_framed_whole";
    let mut w = measured_with_side_picked(test);
    let mut params = w.app.panels_params().clone();
    params.layers = Some(vec!["Bottom".into()]);
    w.app.set_panels_params(params.clone());
    w.until("the bottom alone", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert_eq!(w.app.panels_preview().unwrap().set.sheets.len(), 1);
    assert_eq!(
        w.app.panels_selected(),
        None,
        "the pick was the second of two sheets"
    );
    assert_same_view(&w, fresh_view(pram(test, 0.0), params));
}

#[test]
fn a_chosen_layer_renamed_away_can_be_unticked() {
    let mut w = Window::new(pram("a_chosen_layer_renamed_away_can_be_unticked", 0.0));
    let mut params = w.app.panels_params().clone();
    params.layers = Some(vec!["Bottom".into(), "Side".into()]);
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    let bottom = w
        .app
        .surface()
        .unwrap()
        .layers
        .iter()
        .position(|l| l.name == "Bottom")
        .unwrap();
    w.app
        .apply_layer_action(seacraft_gui::panels::LayerAction::Rename {
            layer: bottom,
            name: "Floor".into(),
        });
    // A window's first frame only sizes it.
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(
        w.widget(Role::CheckBox, "Floor").is_some(),
        "{:?}",
        w.widgets
    );
    let gone = w
        .widget(Role::CheckBox, "Bottom (not in the model)")
        .unwrap_or_else(|| panic!("the old name can be unticked: {:?}", w.widgets));
    w.click_at(gone.center());
    assert_eq!(w.app.panels_params().layers, Some(vec!["Side".to_owned()]));
}

#[test]
fn opening_another_hull_forgets_layers_it_does_not_have() {
    let mut w = Window::new(pram(
        "opening_another_hull_forgets_layers_it_does_not_have",
        0.0,
    ));
    let mut params = w.app.panels_params().clone();
    params.layers = Some(vec!["Bottom".into()]);
    w.app.set_panels_params(params);
    let same_layers = pram_file(
        "opening_another_hull_forgets_layers_it_does_not_have-pram",
        0.08,
    );
    w.app.open_model(&same_layers).unwrap();
    assert_eq!(
        w.app.panels_params().layers,
        Some(vec!["Bottom".to_owned()]),
        "kept where the new hull has it"
    );
    let wigley = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/wigley.fef");
    w.app.open_model(std::path::Path::new(wigley)).unwrap();
    assert_eq!(
        w.app.panels_params().layers,
        None,
        "none of the chosen layers is in the Wigley hull: back to the layers marked developable"
    );
}

#[test]
fn the_paper_does_not_stop_the_measurement() {
    let mut w = Window::new(pram("the_paper_does_not_stop_the_measurement", 0.0));
    let mut params = w.app.panels_params().clone();
    params.paper = "no such paper".into();
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the panels or a failure", |app| {
        app.panels_preview().is_some() || app.panels_failure().is_some()
    });
    assert_eq!(
        w.app.panels_failure(),
        None,
        "the paper is used when writing only"
    );
}

#[test]
fn a_list_of_stations_can_be_switched_to_spacing() {
    let mut w = Window::new(pram("a_list_of_stations_can_be_switched_to_spacing", 0.0));
    let mut params = w.app.panels_params().clone();
    params.stations = seacraft_core::templates::StationChoice::At(vec![0.5, 1.0]);
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.frame(vec![]);
    w.frame(vec![]);
    let spacing = w
        .widget(Role::RadioButton, "Spacing")
        .unwrap_or_else(|| panic!("the Spacing radio: {:?}", w.widgets));
    w.click_at(spacing.center());
    assert!(
        matches!(
            w.app.panels_params().stations,
            seacraft_core::templates::StationChoice::Spacing(_)
        ),
        "{:?}",
        w.app.panels_params().stations
    );
}

#[test]
fn choosing_blanks_shows_the_sheets() {
    let mut w = Window::new(skiff("choosing_blanks_shows_the_sheets"));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(w.shows("Nesting:"), "{:?}", w.painted);
    assert!(w.shows("No nesting"), "{:?}", w.painted);

    let mut params = w.app.panels_params().clone();
    params.choices.nest = "blanks".into();
    w.app.set_panels_params(params);
    w.until("the nested blanks", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);

    assert!(w.button("Panels").is_some(), "{:?}", w.widgets);
    assert!(w.button("Sheets").is_some(), "{:?}", w.widgets);
    assert!(w.app.panels_showing_sheets());
    assert!(w.shows("Join sheets into blanks"), "{:?}", w.painted);
    assert!(w.shows("Gap between pieces:"), "{:?}", w.painted);

    let n = w
        .app
        .panels_preview()
        .unwrap()
        .set
        .layout
        .as_ref()
        .unwrap()
        .drawings
        .len();
    assert!(n >= 1);
    assert!(w.shows(&format!("Drawing: {n} sheets")), "{:?}", w.widgets);
    assert!(w.shows("nested"), "{:?}", w.painted);
    assert!(w.app.panels_sheets_view().zoom > 0.0);

    w.click("Panels");
    assert!(!w.app.panels_showing_sheets());
    assert!(w.shows("Drawing: 3 sheets"), "{:?}", w.widgets);

    w.click("Sheets");
    assert!(w.app.panels_showing_sheets());
    assert!(w.shows(&format!("Drawing: {n} sheets")), "{:?}", w.widgets);
}

#[test]
fn a_stale_layout_is_faded_and_export_waits() {
    let mut w = Window::new(skiff("a_stale_layout_is_faded_and_export_waits"));
    let mut params = w.app.panels_params().clone();
    params.choices.nest = "split".into();
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the nested layout", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(w.app.panels_showing_sheets());
    assert!(!w.has_stale_veil);
    assert!(!w.button("Export…").unwrap().0);

    let mut params = w.app.panels_params().clone();
    params.choices.gap_mm = 20.0;
    w.app.set_panels_params(params);
    w.frame(vec![]);

    assert!(w.app.panels_preview().is_none(), "stale at once");
    assert!(w.shows("Out of date"), "{:?}", w.painted);
    assert!(w.has_stale_veil, "sheet view drawn under the stale veil");
    assert!(w.button("Export…").unwrap().0, "Export waits");
    assert!(w.app.export_panels_to(&scratch("stale.svg")).is_err());

    w.until("the new layout", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(!w.has_stale_veil);
    assert!(!w.shows("Out of date"));
    assert!(!w.button("Export…").unwrap().0);
}

#[test]
fn export_writes_a_file_per_sheet() {
    let mut w = Window::new(skiff("export_writes_a_file_per_sheet"));
    let mut params = w.app.panels_params().clone();
    params.choices.nest = "split".into();
    w.app.set_panels_params(params);
    w.app.set_show_panels(true);
    w.until("the nested layout", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);

    let n = w
        .app
        .panels_preview()
        .unwrap()
        .set
        .layout
        .as_ref()
        .unwrap()
        .sheets
        .len();
    assert!(n >= 1);

    let svg = scratch("nested-gui.svg");
    let _ = std::fs::remove_file(&svg);
    for i in 0..n {
        let _ = std::fs::remove_file(seacraft_core::panels::sheet_file_name(&svg, i));
    }
    w.app.export_panels_to(&svg).unwrap();
    assert!(std::fs::read_to_string(&svg).unwrap().contains("<svg"));
    for i in 0..n {
        let p = seacraft_core::panels::sheet_file_name(&svg, i);
        assert!(
            std::fs::read_to_string(&p).unwrap().contains("<svg"),
            "{p:?}"
        );
    }

    let pdf = scratch("nested-gui.pdf");
    let _ = std::fs::remove_file(&pdf);
    let _ = std::fs::remove_file(seacraft_core::panels::sheet_file_name(&pdf, 0));
    w.app.export_panels_to(&pdf).unwrap();
    assert!(std::fs::read(&pdf).unwrap().starts_with(b"%PDF-"));
    assert!(!seacraft_core::panels::sheet_file_name(&pdf, 0).exists());
}

#[test]
fn no_nesting_hides_the_switch() {
    let mut w = Window::new(skiff("no_nesting_hides_the_switch"));
    w.app.set_show_panels(true);
    w.until("the panels", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(w.button("Panels").is_none());
    assert!(w.button("Sheets").is_none());
    assert!(!w.app.panels_showing_sheets());
    assert!(!w.shows("Gap between pieces:"));

    let mut params = w.app.panels_params().clone();
    params.choices.nest = "blanks".into();
    w.app.set_panels_params(params);
    w.until("the blanks layout", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(w.button("Panels").is_some());
    assert!(w.button("Sheets").is_some());

    let mut params = w.app.panels_params().clone();
    params.choices.nest = "none".into();
    w.app.set_panels_params(params);
    w.until("no nesting", |app| app.panels_preview().is_some());
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(w.button("Panels").is_none());
    assert!(w.button("Sheets").is_none());
    assert!(!w.app.panels_showing_sheets());
}
