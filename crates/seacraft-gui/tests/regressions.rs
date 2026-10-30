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

    /// Let the hydrostatics job for the current inputs finish.
    fn settle_hydro(&mut self) {
        let t0 = Instant::now();
        while self.app.stale_results().contains(&"hydrostatics") || self.app.hydro_busy() {
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
