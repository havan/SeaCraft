//! File > Builder materials, headless: the dialog returns what the user asked for and the app
//! applies it as an undoable project edit; the estimate runs on a job, a stale one is faded and
//! cannot be added to a loading condition; adding one is undoable.
//!
//! The schedules here are the tests' own inputs, not glass or resin figures to build with.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use seacraft_core::materials::{Materials, Ply, Side};
use seacraft_gui::builder_materials::{
    self, MaterialsAction, MaterialsDialogState, MaterialsStatus,
};
use seacraft_gui::App;

/// The skiff-4m example written to a file of the test's own and opened at level 2.
fn skiff(test: &str) -> App {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-materials-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    // One file a test: tests run in parallel, and one must not read another's half-written file.
    let path = dir.join(format!("{test}.fef"));
    let (surface, _) =
        seacraft_core::examples::build(seacraft_core::examples::Example::Skiff4m).expect("skiff");
    std::fs::write(&path, seacraft_core::fef::write(&surface)).unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.set_subdivision_level(2);
    app
}

/// Two plies on the skiff's bottom and a resin ratio: the test's own numbers.
fn two_plies() -> Materials {
    let mut m = Materials {
        resin_per_glass: Some(1.5),
        ..Materials::default()
    };
    m.layers.insert(
        "Bottom".into(),
        vec![
            Ply {
                g_m2: 300.0,
                side: Side::Outside,
            },
            Ply {
                g_m2: 200.0,
                side: Side::Inside,
            },
        ],
    );
    m
}

/// A loading condition called `Design` holding only a lightship: the test's own.
fn design_condition() -> seacraft_core::loading::LoadingCondition {
    serde_json::from_value(serde_json::json!({
        "name": "Design",
        "lightship": { "name": "Lightship", "mass_t": 0.1, "x": 2.0, "y": 0.0, "z": 0.3 }
    }))
    .unwrap()
}

/// Role, label, whether it is disabled, and where.
type Widget = (Role, String, bool, egui::Rect);

/// Every widget of the frame, from egui's AccessKit tree.
fn widgets(out: &egui::FullOutput) -> Vec<Widget> {
    out.platform_output
        .accesskit_update
        .as_ref()
        .map(|update| {
            update
                .nodes
                .iter()
                .filter_map(|(_, node)| {
                    let b = node.bounds()?;
                    Some((
                        node.role(),
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
        .unwrap_or_default()
}

/// Every text painted in the frame and the colour of its first section.
fn painted(out: &egui::FullOutput) -> Vec<(String, egui::Color32)> {
    fn walk(shape: &egui::Shape, texts: &mut Vec<(String, egui::Color32)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, texts)),
            egui::Shape::Text(t) => {
                let colour = t
                    .galley
                    .job
                    .sections
                    .first()
                    .map_or(t.fallback_color, |s| s.format.color);
                texts.push((t.galley.text().to_string(), colour));
            }
            _ => {}
        }
    }
    let mut texts = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut texts);
    }
    texts
}

fn input(events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        // Tall, so the dialog's scroll areas draw everything.
        screen_rect: Some(egui::Rect::from_min_size(
            egui::Pos2::ZERO,
            egui::vec2(1600.0, 1400.0),
        )),
        events,
        ..Default::default()
    }
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// One headless window drawing the whole app.
struct Window {
    ctx: egui::Context,
    app: App,
    widgets: Vec<Widget>,
    painted: Vec<(String, egui::Color32)>,
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
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input(events), |ui| app.draw(ui));
        out.textures_delta.clear();
        self.widgets = widgets(&out);
        self.painted = painted(&out);
    }

    fn buttons(&self, label: &str) -> Vec<(bool, egui::Rect)> {
        self.widgets
            .iter()
            .filter(|(r, l, _, _)| *r == Role::Button && l == label)
            .map(|(_, _, disabled, rect)| (*disabled, *rect))
            .collect()
    }

    fn button(&self, label: &str) -> (bool, egui::Rect) {
        *self
            .buttons(label)
            .first()
            .unwrap_or_else(|| panic!("no `{label}` button: {:?}", self.widgets))
    }

    fn click_at(&mut self, pos: egui::Pos2) {
        self.frame(vec![egui::Event::PointerMoved(pos), press(pos, true)]);
        self.frame(vec![press(pos, false)]);
        self.frame(vec![]);
    }

    fn click(&mut self, label: &str) {
        let (_, rect) = self.button(label);
        self.click_at(rect.center());
    }

    fn shows(&self, text: &str) -> bool {
        self.painted.iter().any(|(t, _)| t.contains(text))
            || self.widgets.iter().any(|(_, l, _, _)| l.contains(text))
    }

    /// The colour the first painted text starting with `text` was drawn in.
    fn colour_of(&self, text: &str) -> egui::Color32 {
        self.painted
            .iter()
            .find(|(t, _)| t.starts_with(text))
            .map(|(_, c)| *c)
            .unwrap_or_else(|| panic!("`{text}` not painted: {:?}", self.painted))
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
fn dialog_returns_actions_without_mutating() {
    // The dialog alone: it edits its own draft and returns the schedule asked for.
    let ctx = egui::Context::default();
    ctx.enable_accesskit();
    let tr = seacraft_core::i18n::Tr::new(seacraft_core::i18n::Locale::En);
    let mut state = MaterialsDialogState::default();
    state.sync(
        Some(&two_plies()),
        &["Bottom".to_owned(), "Side".to_owned()],
    );
    let conditions = vec!["Design".to_owned()];
    let run = |state: &mut MaterialsDialogState, events| {
        let mut action = None;
        let mut out = ctx.run_ui(input(events), |ui| {
            action =
                builder_materials::show(ui, state, MaterialsStatus::Computing, &conditions, tr);
        });
        out.textures_delta.clear();
        (action, widgets(&out))
    };
    let (first, _) = run(&mut state, vec![]);
    let (second, ws) = run(&mut state, vec![]);
    assert_eq!(first, None, "nothing asked for yet");
    assert_eq!(second, None);
    let remove: Vec<egui::Rect> = ws
        .iter()
        .filter(|(r, l, _, _)| *r == Role::Button && l == "Remove ply")
        .map(|(_, _, _, rect)| *rect)
        .collect();
    assert_eq!(remove.len(), 2, "one a ply: {ws:?}");
    let pos = remove[0].center();
    let (a, _) = run(
        &mut state,
        vec![egui::Event::PointerMoved(pos), press(pos, true)],
    );
    let (b, _) = run(&mut state, vec![press(pos, false)]);
    let asked = a.or(b).expect("removing a ply asks for the schedule");
    let MaterialsAction::SetSchedule(m) = asked else {
        panic!("not a schedule: {asked:?}");
    };
    assert_eq!(m.layers["Bottom"].len(), 1);
    assert_eq!(m.layers["Bottom"][0].g_m2, 200.0, "the first ply went");

    // In the app: the project changes only when `App::draw` applies the request, and undo
    // takes it back, the dialog following.
    let mut app = skiff("dialog_returns_actions_without_mutating");
    app.set_project(seacraft_core::project::Project {
        materials: Some(two_plies()),
        ..Default::default()
    });
    app.open_builder_materials();
    assert!(app.show_materials());
    let mut w = Window::new(app);
    assert_eq!(
        w.app.project().materials,
        Some(two_plies()),
        "drawing changes nothing"
    );
    let undo_before = w.app.undo_len();
    w.click("Remove ply");
    let after = w.app.project().materials.clone().expect("a schedule");
    assert_eq!(after.layers["Bottom"].len(), 1);
    assert_eq!(w.app.undo_len(), undo_before + 1, "one undo step");
    w.app.undo();
    w.frame(vec![]);
    w.frame(vec![]);
    assert_eq!(w.app.project().materials, Some(two_plies()));
    assert_eq!(
        w.buttons("Remove ply").len(),
        2,
        "the dialog shows the schedule undone"
    );
}

#[test]
fn stale_estimate_is_faded_and_add_disabled() {
    let mut app = skiff("stale_estimate_is_faded_and_add_disabled");
    app.project_mut_for_test()
        .loading_conditions
        .push(design_condition());
    app.open_builder_materials();
    let mut w = Window::new(app);
    w.until("the estimate", |app| app.materials_estimate().is_some());
    w.frame(vec![]);
    let (disabled, _) = w.button("Add to loading condition");
    assert!(!disabled, "current: the total can be added");
    assert!(w.shows("no glass or resin entered") || w.shows("No glass or resin entered"));
    assert!(w.shows("Not validated"), "the note");
    let current = w.colour_of("Total:");

    let p = w.app.surface().unwrap().points[5].p;
    w.app
        .move_point(5, p + seacraft_core::math::Vec3::new(0.0, 0.0, 0.01))
        .unwrap();
    assert!(w.app.materials_estimate().is_none(), "stale at once");
    w.frame(vec![]);
    let (disabled, _) = w.button("Add to loading condition");
    assert!(disabled, "an out-of-date total cannot be added");
    assert!(w.shows("Out of date"), "the old estimate is marked");
    let stale = w.colour_of("Total:");
    assert_eq!(stale, seacraft_gui::theme::TEXT_MUTED, "faded");
    assert_ne!(stale, current);

    w.until("the estimate for the moved point", |app| {
        app.materials_estimate().is_some()
    });
    w.frame(vec![]);
    assert!(!w.button("Add to loading condition").0);
}

#[test]
fn add_to_condition_then_undo() {
    let mut app = skiff("add_to_condition_then_undo");
    app.project_mut_for_test()
        .loading_conditions
        .push(design_condition());
    app.open_builder_materials();
    let mut w = Window::new(app);
    w.until("the estimate", |app| app.materials_estimate().is_some());
    w.frame(vec![]);
    let total = w.app.materials_estimate().unwrap().total.clone();
    let undo_before = w.app.undo_len();
    w.click("Add to loading condition");
    let items = &w.app.project().loading_conditions[0].items;
    assert_eq!(items.len(), 1, "{items:?}");
    let item = &items[0];
    assert_eq!(item.name, "Hull materials (estimate)", "the default name");
    assert_eq!(item.mass_t, total.mass_t);
    assert_eq!([item.x, item.y, item.z], total.centre);
    assert_eq!(w.app.undo_len(), undo_before + 1, "one undo step");
    assert!(
        w.app.status().contains("Hull materials (estimate)") && w.app.status().contains("Design"),
        "{}",
        w.app.status()
    );
    assert!(w.app.is_dirty());

    w.app.undo();
    assert!(w.app.project().loading_conditions[0].items.is_empty());
}

#[test]
fn the_file_menu_offers_builder_materials_only_with_a_model() {
    let mut empty = Window::new(App::headless_empty());
    empty.click("File");
    assert!(
        empty.button("Builder materials…").0,
        "no model, no materials"
    );

    let mut w = Window::new(skiff(
        "the_file_menu_offers_builder_materials_only_with_a_model",
    ));
    w.click("File");
    assert!(!w.button("Builder materials…").0);
    w.click("Builder materials…");
    assert!(w.app.show_materials());
}

/// The painted "not applied" lines, in Turkish.
fn not_applied_tr(w: &Window) -> Vec<String> {
    w.painted
        .iter()
        .map(|(t, _)| t.clone())
        .filter(|t| t.starts_with("Henüz uygulanmadı:"))
        .collect()
}

#[test]
fn an_unfinished_schedule_is_explained_in_the_users_language() {
    let mut app = skiff("an_unfinished_schedule_is_explained_in_the_users_language");
    app.set_locale(seacraft_core::i18n::Locale::Tr);
    app.open_builder_materials();
    let mut w = Window::new(app);
    // A ply with no resin ratio yet.
    w.click("Kat ekle");
    let lines = not_applied_tr(&w);
    assert_eq!(lines.len(), 1, "{:?}", w.painted);
    assert!(lines[0].contains("Cam elyaf başına reçine"), "{lines:?}");

    // With a resin ratio, the new ply's zero is what is missing.
    w.app.set_project(seacraft_core::project::Project {
        materials: Some(two_plies()),
        ..Default::default()
    });
    w.frame(vec![]);
    w.frame(vec![]);
    w.click("Kat ekle");
    let lines = not_applied_tr(&w);
    assert_eq!(lines.len(), 1, "{:?}", w.painted);
    // Whichever layer's button came first: the ply is named by layer and number.
    assert!(
        lines[0].contains(" katmanındaki ") && lines[0].contains(". kat: "),
        "{lines:?}"
    );
    for path in ["g_m2", "resin_per_glass", "layers.", "["] {
        assert!(!lines[0].contains(path), "{path} in {lines:?}");
    }
    assert_eq!(w.app.project().materials, Some(two_plies()), "not applied");
}

#[test]
fn a_schedule_edit_makes_the_estimate_stale() {
    let mut app = skiff("a_schedule_edit_makes_the_estimate_stale");
    app.set_project(seacraft_core::project::Project {
        materials: Some(two_plies()),
        loading_conditions: vec![design_condition()],
        ..Default::default()
    });
    app.open_builder_materials();
    let mut w = Window::new(app);
    w.until("the estimate", |app| app.materials_estimate().is_some());
    w.frame(vec![]);
    assert!(!w.button("Add to loading condition").0);
    w.click("Remove ply");
    assert_eq!(
        w.app.project().materials.as_ref().unwrap().layers["Bottom"].len(),
        1
    );
    assert!(w.app.materials_estimate().is_none(), "stale at once");
    assert!(w.button("Add to loading condition").0, "add disabled");
    assert!(w.shows("Out of date"));
    w.until("the estimate for the new schedule", |app| {
        app.materials_estimate().is_some()
    });
    let est = w.app.materials_estimate().unwrap();
    assert_eq!(
        est.layers
            .iter()
            .find(|r| r.layer == "Bottom")
            .unwrap()
            .glass_g_m2,
        200.0
    );
}

#[test]
fn an_unfinished_draft_does_not_follow_another_model() {
    let mut app = skiff("an_unfinished_draft_does_not_follow_another_model");
    app.open_builder_materials();
    let mut w = Window::new(app);
    w.click("Add ply");
    assert_eq!(w.buttons("Remove ply").len(), 1, "the unfinished ply");
    assert!(w.shows("Not applied yet"));
    assert_eq!(w.app.project().materials, None);

    // Another hull whose project has no schedule either: the draft goes with the old hull.
    w.app.load_demo();
    w.frame(vec![]);
    w.frame(vec![]);
    assert!(w.app.show_materials());
    assert!(w.buttons("Remove ply").is_empty(), "{:?}", w.widgets);
    assert!(!w.shows("Not applied yet"));
}

#[test]
fn the_results_name_uncounted_panels_and_print_no_negative_zero() {
    // The runabout's bottom is wider than the default stock, so its row counts no sheets and
    // says why; the total's centre lies a rounding step to port of the centreline.
    let (surface, _) = seacraft_core::examples::build(seacraft_core::examples::Example::Runabout6m)
        .expect("runabout");
    let mut est = seacraft_core::materials::estimate(
        &surface,
        &Materials::default(),
        seacraft_core::subdiv::SubdivisionOptions {
            level: 2,
            mode: seacraft_core::subdiv::SubdivisionMode::Loop,
        },
    );
    est.total.centre[1] = -1e-9;
    let ctx = egui::Context::default();
    let tr = seacraft_core::i18n::Tr::new(seacraft_core::i18n::Locale::En);
    let mut state = MaterialsDialogState::default();
    state.sync(None, &["Bottom".to_owned(), "Topsides".to_owned()]);
    let mut texts = Vec::new();
    for _ in 0..2 {
        let mut out = ctx.run_ui(input(vec![]), |ui| {
            builder_materials::show(ui, &mut state, MaterialsStatus::Ready(&est), &[], tr);
        });
        out.textures_delta.clear();
        texts = painted(&out);
    }
    let shows = |s: &str| texts.iter().any(|(t, _)| t.contains(s));
    assert!(
        shows("not flat: Bottom; not counted: Bottom (wider than the stock)"),
        "{texts:?}"
    );
    assert!(shows(" y 0.000 z "), "{texts:?}");
    assert!(!shows("-0.000"), "{texts:?}");
}
