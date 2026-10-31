//! Regression tests for two round-3 findings that were fixed but never
//! tested (`.hermes/research/2026-10-24_bughunt-C-frontends.md`):
//!
//! * **C07** — the View menu's subdivision level slider and mode combo set
//!   the fields directly, so the revision never moved: analysis frames and
//!   the hydrostatics stayed those of the old mesh. They now go through the
//!   setters.
//! * **C13** — renaming or deleting a loading condition (or renaming a
//!   compartment) left damage cases naming something that no longer exists,
//!   and `Project::validate` then refused every save. The docks now cascade
//!   the change.
//!
//! Both are driven through the real widgets (found by their accessibility
//! nodes), not through the setters the fix calls.

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use seacraft_core::damage::{CompartmentSpec, DamageCase};
use seacraft_core::loading::LoadingCondition;
use seacraft_core::project::Project;
use seacraft_core::SubdivisionMode;
use seacraft_gui::App;

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

#[derive(Debug, Clone)]
struct Widget {
    role: Role,
    label: String,
    value: String,
    rect: egui::Rect,
}

struct Window {
    ctx: egui::Context,
    app: App,
    widgets: Vec<Widget>,
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
                        Some(Widget {
                            role: node.role(),
                            label: node.label().unwrap_or("").to_owned(),
                            value: node.value().unwrap_or("").to_owned(),
                            rect: egui::Rect::from_min_max(
                                egui::pos2(b.x0 as f32, b.y0 as f32),
                                egui::pos2(b.x1 as f32, b.y1 as f32),
                            ),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn find(&self, pred: impl Fn(&Widget) -> bool, what: &str) -> Widget {
        self.widgets
            .iter()
            .find(|w| pred(w))
            .cloned()
            .unwrap_or_else(|| {
                panic!(
                    "no {what} among {:?}",
                    self.widgets
                        .iter()
                        .map(|w| (w.role, w.label.as_str(), w.value.as_str()))
                        .collect::<Vec<_>>()
                )
            })
    }

    fn label(&self, label: &str) -> Widget {
        self.find(|w| w.label == label, &format!("`{label}`"))
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

    /// Replace the text of the field showing `old` with `new`, as typed.
    fn retype(&mut self, old: &str, new: &str) {
        let field = self.find(
            |w| w.role == Role::TextInput && w.value == old,
            &format!("a text field showing `{old}`"),
        );
        self.click_at(field.rect.center());
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        self.frame(vec![select_all, egui::Event::Text(new.into())]);
        self.frame(vec![]);
    }

    /// Replace the text of the field showing `old` with `new`, typing one
    /// character per frame so continuous-edit coalescing is exercised.
    fn retype_char_by_char(&mut self, old: &str, new: &str) {
        let field = self.find(
            |w| w.role == Role::TextInput && w.value == old,
            &format!("a text field showing `{old}`"),
        );
        self.click_at(field.rect.center());
        let select_all = egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::COMMAND,
        };
        let mut chars = new.chars();
        if let Some(first) = chars.next() {
            self.frame(vec![select_all, egui::Event::Text(first.to_string())]);
        }
        for ch in chars {
            self.frame(vec![egui::Event::Text(ch.to_string())]);
        }
        self.frame(vec![]);
    }

    /// Drag horizontally from `start` across several frames before releasing.
    fn drag_horizontal(&mut self, start: egui::Pos2, steps: &[f32]) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(start), button(start, true)]);
        let mut last = start;
        for &dx in steps {
            last = start + egui::vec2(dx, 0.0);
            self.frame(vec![egui::Event::PointerMoved(last)]);
        }
        self.frame(vec![button(last, false)]);
        self.frame(vec![]);
    }

    /// Let the hydrostatics job for the current inputs finish. Before the
    /// first result nothing counts as stale and the debounced job may not
    /// have started yet, so wait for a result too.
    fn settle_hydro(&mut self) {
        let t0 = Instant::now();
        while self.app.hydrostatics().is_none()
            || self.app.stale_results().contains(&"hydrostatics")
            || self.app.hydro_busy()
        {
            assert!(
                t0.elapsed() < Duration::from_secs(60),
                "hydro never settled"
            );
            std::thread::sleep(Duration::from_millis(20));
            self.frame(vec![]);
        }
    }
}

// ---- C07 -------------------------------------------------------------------

#[test]
fn c07_view_menu_level_and_mode_refresh_frames_and_hydrostatics() {
    let mut app = App::headless_for_test(Some(&example("demo-hull.fef")));
    app.set_subdivision_level(2);
    app.set_analysis(seacraft_core::AnalysisMode::Zebra { stripes: 14.0 });
    let mut w = Window::new(app);
    w.settle_hydro();
    let key0 = w.app.analysis_frame_key(0).expect("a zebra frame");
    let volume0 = w.app.hydrostatics().expect("hydrostatics").volume_m3;

    // View > mode > Catmull-Clark. (It was a combo box, whose list closed
    // with the menu on the click that opened it: radio buttons now.)
    let view = w.label("View").rect.center();
    w.click_at(view);
    let cc = w
        .find(
            |x| x.role == Role::RadioButton && x.label == "Catmull-Clark",
            "the View menu's Catmull-Clark",
        )
        .rect
        .center();
    w.click_at(cc);
    assert_eq!(w.app.subdivision_mode(), SubdivisionMode::CatmullClark);
    assert_ne!(
        w.app.analysis_frame_key(0),
        Some(key0),
        "the zebra frame was redrawn for the new mesh"
    );
    // The demo's net is all quads, which both schemes refine alike, so the
    // volume cannot tell the meshes apart; the level below can.
    w.settle_hydro();
    let volume1 = w.app.hydrostatics().unwrap().volume_m3;
    assert!((volume1 - volume0).abs() < 1e-9 * volume0);
    let key1 = w.app.analysis_frame_key(0);

    // View > subdivision level, dragged to its low end.
    if !w.widgets.iter().any(|x| x.role == Role::Slider) {
        w.click_at(view);
    }
    let slider = w.find(|x| x.role == Role::Slider, "the level slider");
    w.click_at(egui::pos2(slider.rect.left() + 1.0, slider.rect.center().y));
    assert_eq!(w.app.subdivision_level(), 0);
    assert_ne!(w.app.analysis_frame_key(0), key1, "redrawn at level 0");
    w.settle_hydro();
    assert_ne!(w.app.hydrostatics().unwrap().volume_m3, volume1);
}

// ---- C13 -------------------------------------------------------------------

/// The demo with one loading condition, one compartment and two damage
/// cases naming them.
fn demo_with_cases() -> (App, LoadingCondition, String) {
    let mut app = App::headless_for_test(Some(&example("demo-hull.fef")));
    app.set_subdivision_level(1);
    let cond: LoadingCondition =
        serde_json::from_str(&std::fs::read_to_string(example("demo-loadcase.json")).unwrap())
            .unwrap();
    let case: DamageCase =
        serde_json::from_str(&std::fs::read_to_string(example("demo-damage.json")).unwrap())
            .unwrap();
    let comp = match &case.compartments[0] {
        CompartmentSpec::Inline(c) => c.clone(),
        CompartmentSpec::Named(_) => panic!("an inline compartment"),
    };
    let named = DamageCase {
        compartments: vec![CompartmentSpec::Named(comp.name.clone())],
        ..case.clone()
    };
    let by_condition = DamageCase {
        name: "Damage with loadcase".into(),
        loading_condition_name: Some(cond.name.clone()),
        loading_condition: None,
        displacement_t: None,
        kg: None,
        lcg: None,
        tcg: None,
        ..named.clone()
    };
    let name = comp.name.clone();
    app.set_project(Project {
        compartments: vec![comp],
        damage_cases: vec![named, by_condition],
        loading_conditions: vec![cond.clone()],
        ..Default::default()
    });
    assert!(app.project().validate().is_ok());
    (app, cond, name)
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-regressions-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn c13_renaming_or_deleting_a_loading_condition_follows_into_damage_cases() {
    let (mut app, cond, _) = demo_with_cases();
    app.set_show_loading(true);
    let mut w = Window::new(app);

    w.retype(&cond.name, "Arrival");
    let p = w.app.project();
    assert_eq!(p.loading_conditions[0].name, "Arrival");
    assert_eq!(
        p.damage_cases[1].loading_condition_name.as_deref(),
        Some("Arrival"),
        "the damage case follows the rename"
    );
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    assert!(w.app.save_to(&scratch("renamed.fef")), "the project saves");

    // The dock's Delete (the layer panel on the left has one too).
    let middle = w.app.central_rect().center().x;
    let delete = w.find(
        |x| x.role == Role::Button && x.label == "Delete" && x.rect.center().x > middle,
        "the loading dock's Delete",
    );
    w.click_at(delete.rect.center());
    let p = w.app.project();
    assert!(p.loading_conditions.is_empty(), "the condition is gone");
    assert_eq!(
        p.damage_cases[1].loading_condition_name, None,
        "the damage case no longer names it"
    );
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    assert!(w.app.save_to(&scratch("deleted.fef")), "the project saves");
}

#[test]
fn c13_renaming_a_compartment_follows_into_damage_cases() {
    let (mut app, _, comp) = demo_with_cases();
    app.set_show_damage(true);
    let mut w = Window::new(app);

    w.retype(&comp, "Engine room");
    let p = w.app.project();
    assert_eq!(p.compartments[0].name, "Engine room");
    for case in &p.damage_cases {
        assert!(
            matches!(&case.compartments[0], CompartmentSpec::Named(n) if n == "Engine room"),
            "{:?}",
            case.compartments
        );
    }
    assert!(p.validate().is_ok(), "{:?}", p.validate());
    assert!(
        w.app.save_to(&scratch("compartment.fef")),
        "the project saves"
    );
}

// ---- Dock undo/redo --------------------------------------------------------

/// Save `demo_with_cases` and reopen it so history is empty and `dirty` is false.
fn saved_demo_with_cases(tag: &str) -> (App, LoadingCondition, String) {
    let (mut app, cond, comp) = demo_with_cases();
    let path = scratch(tag);
    assert!(app.save_to(&path));
    let mut reopened = App::headless_for_test(Some(path.to_str().unwrap()));
    reopened.set_subdivision_level(1);
    assert!(!reopened.is_dirty());
    assert_eq!(reopened.undo_len(), 0);
    (reopened, cond, comp)
}

#[test]
fn dock_rename_and_delete_condition_undo_and_redo_in_one_step() {
    let (mut app, cond, comp) = saved_demo_with_cases("dock-undo-rename.fef");
    app.set_show_loading(true);
    let mut w = Window::new(app);
    w.app.compute_loading_blocking(None).unwrap();
    w.app.compute_damage_blocking(None).unwrap();
    assert!(!w.app.stale_results().contains(&"loading"));
    assert!(!w.app.stale_results().contains(&"damage"));

    // Typing "Arrival" character by character across frames is coalesced into
    // a single undo step, and cascades into the damage case reference.
    w.retype_char_by_char(&cond.name, "Arrival");
    assert_eq!(w.app.undo_len(), 1, "multi-frame typing is one undo step");
    assert!(w.app.is_dirty());
    assert_eq!(w.app.project().loading_conditions[0].name, "Arrival");
    assert_eq!(
        w.app.project().damage_cases[1]
            .loading_condition_name
            .as_deref(),
        Some("Arrival")
    );
    assert!(w.app.stale_results().contains(&"loading"));
    assert!(w.app.stale_results().contains(&"damage"));

    // Recompute at the renamed state so we can verify undo marks results stale.
    w.app.compute_loading_blocking(None).unwrap();
    w.app.compute_damage_blocking(None).unwrap();
    assert!(!w.app.stale_results().contains(&"loading"));
    assert!(!w.app.stale_results().contains(&"damage"));

    // Undo restores both the condition name and the damage case reference in one step.
    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 0);
    assert!(w.app.can_redo());
    assert!(!w.app.is_dirty(), "back at saved state");
    assert_eq!(w.app.project().loading_conditions[0].name, cond.name);
    assert_eq!(
        w.app.project().damage_cases[1]
            .loading_condition_name
            .as_deref(),
        Some(cond.name.as_str())
    );
    assert!(w.app.stale_results().contains(&"loading"));
    assert!(w.app.stale_results().contains(&"damage"));

    // Redo reapplies both the condition name and the damage case reference in one step.
    w.app.compute_loading_blocking(None).unwrap();
    w.app.compute_damage_blocking(None).unwrap();
    w.app.redo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 1);
    assert!(w.app.is_dirty());
    assert_eq!(w.app.project().loading_conditions[0].name, "Arrival");
    assert_eq!(
        w.app.project().damage_cases[1]
            .loading_condition_name
            .as_deref(),
        Some("Arrival")
    );
    assert!(w.app.stale_results().contains(&"loading"));
    assert!(w.app.stale_results().contains(&"damage"));

    // Deleting the condition cascades to the damage case and undoes in one step.
    let middle = w.app.central_rect().center().x;
    let delete = w.find(
        |x| x.role == Role::Button && x.label == "Delete" && x.rect.center().x > middle,
        "the loading dock's Delete",
    );
    w.click_at(delete.rect.center());
    assert_eq!(w.app.undo_len(), 2);
    assert!(w.app.project().loading_conditions.is_empty());
    assert_eq!(w.app.project().damage_cases[1].loading_condition_name, None);

    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 1);
    assert_eq!(w.app.project().loading_conditions.len(), 1);
    assert_eq!(w.app.project().loading_conditions[0].name, "Arrival");
    assert_eq!(
        w.app.project().damage_cases[1]
            .loading_condition_name
            .as_deref(),
        Some("Arrival")
    );

    // Renaming a compartment in the Damage tab cascades to damage cases and
    // undoes in one step too.
    w.app.set_show_damage(true);
    w.frame(vec![]);
    w.retype_char_by_char(&comp, "Engine room");
    assert_eq!(w.app.undo_len(), 2);
    assert_eq!(w.app.project().compartments[0].name, "Engine room");
    assert!(
        matches!(&w.app.project().damage_cases[0].compartments[0], CompartmentSpec::Named(n) if n == "Engine room")
    );
    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 1);
    assert_eq!(w.app.project().compartments[0].name, comp);
    assert!(
        matches!(&w.app.project().damage_cases[0].compartments[0], CompartmentSpec::Named(n) if n == &comp)
    );
}

#[test]
fn dock_numeric_field_drag_is_one_undo_step() {
    let (mut app, cond, _) = saved_demo_with_cases("dock-undo-drag.fef");
    app.set_show_loading(true);
    let mut w = Window::new(app);
    let mass0 = cond.lightship.mass_t;
    let shown0 = format!("{mass0:.2}");

    // Find the Lightship mass DragValue (SpinButton) in the Loading dock and
    // drag it across several frames.
    let middle = w.app.central_rect().center().x;
    let spin = w.find(
        |x| {
            x.role == Role::SpinButton && x.value.starts_with(&shown0) && x.rect.center().x > middle
        },
        "the lightship mass DragValue",
    );
    w.drag_horizontal(spin.rect.center(), &[8.0, 20.0, 35.0, 50.0]);

    let mass1 = w.app.project().loading_conditions[0].lightship.mass_t;
    assert!(
        (mass1 - mass0).abs() > 1e-6,
        "drag changed mass from {mass0} to {mass1}"
    );
    assert_eq!(
        w.app.undo_len(),
        1,
        "a multi-frame DragValue drag is one undo step"
    );
    assert!(w.app.is_dirty());

    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 0);
    assert!(!w.app.is_dirty());
    assert!(
        (w.app.project().loading_conditions[0].lightship.mass_t - mass0).abs() < 1e-9,
        "undo restored the original mass"
    );

    w.app.redo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 1);
    assert!(w.app.is_dirty());
    assert!(
        (w.app.project().loading_conditions[0].lightship.mass_t - mass1).abs() < 1e-9,
        "redo reapplied the dragged mass"
    );
}

#[test]
fn control_point_drag_followed_by_dock_edit_undoes_in_reverse_order() {
    let (mut app, cond, _) = saved_demo_with_cases("dock-undo-order.fef");
    app.set_show_loading(true);
    let mut w = Window::new(app);

    // Step 1: drag a control point in the profile viewport (view 1).
    let index = {
        let surface = w.app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .expect("a sheer point")
    };
    let pt_before = w.app.surface().unwrap().points[index].p;
    let rect = w.app.viewport_rect(1);
    let screen = w.app.cameras()[1]
        .transform()
        .project(pt_before, rect)
        .expect("point visible");
    w.drag_horizontal(screen, &[10.0, 45.0]);
    let pt_after = w.app.surface().unwrap().points[index].p;
    assert!(
        (pt_after - pt_before).length() > 1e-3,
        "control point moved"
    );
    assert_eq!(w.app.undo_len(), 1);

    // Step 2: rename the loading condition in the dock.
    w.retype(&cond.name, "Ballast");
    assert_eq!(w.app.project().loading_conditions[0].name, "Ballast");
    assert_eq!(w.app.undo_len(), 2);

    // First undo: reverses the dock edit, leaving the control point moved.
    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 1);
    assert_eq!(w.app.project().loading_conditions[0].name, cond.name);
    assert!(
        (w.app.surface().unwrap().points[index].p - pt_after).length() < 1e-9,
        "the control-point move is still in place after undoing the dock edit"
    );

    // Second undo: reverses the control-point drag.
    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.app.undo_len(), 0);
    assert!(!w.app.is_dirty());
    assert!(
        (w.app.surface().unwrap().points[index].p - pt_before).length() < 1e-9,
        "the control point is back where it started"
    );

    // Redo in forward order: first the control point, then the dock edit.
    w.app.redo();
    w.frame(vec![]);
    assert!(
        (w.app.surface().unwrap().points[index].p - pt_after).length() < 1e-9,
        "first redo restores the control-point move"
    );
    assert_eq!(w.app.project().loading_conditions[0].name, cond.name);

    w.app.redo();
    w.frame(vec![]);
    assert_eq!(w.app.project().loading_conditions[0].name, "Ballast");

    // Opening/creating a model clears the history as before.
    w.app.load_demo();
    assert_eq!(w.app.undo_len(), 0);
    assert!(!w.app.can_undo());
    assert!(!w.app.can_redo());
}
