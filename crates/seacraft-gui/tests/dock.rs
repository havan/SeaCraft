//! The one right Analysis dock (pinned strip + tabs) and the left selection
//! Inspector, headless.

use eframe::egui;
use seacraft_core::math::Vec3;
use seacraft_gui::app::DesignDock;
use seacraft_gui::panels::{self, InspectorAction};
use seacraft_gui::App;

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    // Nothing uploads the fonts in a headless pass; a dropped delta panics.
    out.textures_delta.clear();
    out
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn texts(out: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut text);
    }
    text
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A point well inside the hull on the sheer line.
fn sheer_point(app: &App) -> usize {
    app.surface()
        .expect("model loaded")
        .points
        .iter()
        .enumerate()
        .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
        .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
        .map(|(i, _)| i)
        .expect("a sheer point")
}

#[test]
fn the_loading_tab_lives_in_one_right_dock_that_leaves_the_viewports_wide() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert_eq!(app.design_dock(), Some(DesignDock::Hydrostatics));
    let ctx = egui::Context::default();
    app.set_show_loading(true);
    // The docks settle on the second frame.
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.design_dock(), Some(DesignDock::Loading));
    let profile = app.viewport_rect(1);
    assert!(
        profile.width() > 400.0,
        "profile cell {profile:?} with the Loading tab open"
    );
    // One right panel: the viewports end where that one ~360 px dock starts
    // (the old hydrostatics + design docks left them ending near x = 880).
    assert!(
        profile.right() > 1600.0 - 450.0,
        "the viewports end at {} — more than one right dock?",
        profile.right()
    );
    // Hydro and Loading share the dock: switching tabs moves nothing.
    app.set_show_loading(false);
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.design_dock(), Some(DesignDock::Hydrostatics));
    assert_eq!(app.viewport_rect(1), profile);

    // ⤢ wide takes about 55 % of the window; off again restores the dock.
    app.set_dock_wide(true);
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let wide = app.viewport_rect(1);
    assert!(
        (1600.0 - wide.right() - 0.55 * 1600.0).abs() < 30.0,
        "the wide dock starts at {}",
        wide.right()
    );
    app.set_dock_wide(false);
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.viewport_rect(1), profile);
}

#[test]
fn the_pinned_strip_shows_the_displacement_on_every_tab() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.recompute_hydrostatics();
    let h = app.hydrostatics().expect("a measurement").clone();
    let rows = panels::live_strip_rows(Some(&h), app.kg());
    assert_eq!(rows[0][0].name, "Δ");
    assert_eq!(rows[0][0].value, format!("{:.2}", h.displacement_t));
    let names: Vec<_> = rows.iter().flatten().map(|r| r.name).collect();
    assert_eq!(names, ["Δ", "T", "trim", "GM", "LCB"]);

    let ctx = egui::Context::default();
    for tab in [DesignDock::Hydrostatics, DesignDock::Damage] {
        app.set_dock_tab(tab);
        frame(&ctx, &mut app, Vec::new());
        let out = frame(&ctx, &mut app, Vec::new());
        let text = texts(&out);
        assert!(text.iter().any(|t| t == "Δ"), "{tab:?}: no Δ in {text:?}");
        let value = format!("{:.2}", h.displacement_t);
        assert!(
            text.contains(&value),
            "{tab:?}: displacement {value} not on the strip"
        );
    }
    // Before any measurement the strip shows dashes, not zeros.
    let empty = panels::live_strip_rows(None, Some(2.2));
    assert!(empty.iter().flatten().all(|r| r.value == "—"));
}

#[test]
fn the_inspector_moves_a_point_as_one_undo_step() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(
        app.inspector_field_rect(2),
        egui::Rect::NOTHING,
        "no fields with nothing selected"
    );
    let index = sheer_point(&app);
    app.select_point(Some(index));
    frame(&ctx, &mut app, Vec::new());
    assert!(app.inspector_field_rect(2).width() > 0.0, "z field drawn");

    let before = app.surface().unwrap().points[index].p;
    let undo = app.undo_len();
    let to = Vec3::new(before.x, before.y, before.z + 0.125);
    app.apply_inspector_action(InspectorAction::Move { index, to });
    assert_eq!(app.surface().unwrap().points[index].p, to);
    assert_eq!(app.undo_len(), undo + 1);
    assert!(app.is_dirty());
    // The same value again is no edit.
    app.apply_inspector_action(InspectorAction::Move { index, to });
    assert_eq!(app.undo_len(), undo + 1);
    app.undo();
    assert_eq!(app.surface().unwrap().points[index].p, before);
}

#[test]
fn the_inspector_clamps_a_symmetric_point_to_the_centreline() {
    let mut app = App::headless_for_test(None);
    app.load_demo_shape(true);
    let surface = app.surface().unwrap();
    let index = (0..surface.points.len())
        .find(|&i| {
            seacraft_gui::app::point_on_symmetric_layer(surface, i) && surface.points[i].p.y > 0.1
        })
        .expect("a point off the centreline of a symmetric layer");
    let before = surface.points[index].p;
    app.apply_inspector_action(InspectorAction::Move {
        index,
        to: Vec3::new(before.x, -0.5, before.z),
    });
    let after = app.surface().unwrap().points[index].p;
    assert_eq!(after, Vec3::new(before.x, 0.0, before.z));
    assert_eq!(app.undo_len(), 1);
}

#[test]
fn a_typed_value_is_one_undo_entry_over_several_frames() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    let index = sheer_point(&app);
    app.select_point(Some(index));
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let before = app.surface().unwrap().points[index].p;
    let field = app.inspector_field_rect(2).center();

    // Click the z field: it turns into a text edit with the value selected.
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(field)]);
    frame(&ctx, &mut app, vec![button(field, true)]);
    frame(&ctx, &mut app, vec![button(field, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert!(
        ctx.egui_wants_keyboard_input(),
        "the z field has the keyboard"
    );
    // Typing changes nothing until the value is committed.
    frame(&ctx, &mut app, vec![egui::Event::Text("1.234".into())]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.surface().unwrap().points[index].p, before);
    assert_eq!(app.undo_len(), 0);
    // Enter commits once; the frames after it add nothing.
    frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    let after = app.surface().unwrap().points[index].p;
    assert!((after.z - 1.234).abs() < 1e-12, "z is {}", after.z);
    assert_eq!((after.x, after.y), (before.x, before.y));
    assert_eq!(app.undo_len(), 1, "one edit, one undo entry");
    assert_eq!(app.selected(), Some(index), "the point stays selected");
}

#[test]
fn dragging_an_inspector_value_moves_the_point_once_on_release() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    let index = sheer_point(&app);
    app.select_point(Some(index));
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let before = app.surface().unwrap().points[index].p;
    let field = app.inspector_field_rect(0).center();

    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(field)]);
    frame(&ctx, &mut app, vec![button(field, true)]);
    for dx in [10.0, 20.0, 30.0, 40.0] {
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(field + egui::vec2(dx, 0.0))],
        );
    }
    // Mid-drag the model is untouched: the edit lands on release.
    assert_eq!(app.surface().unwrap().points[index].p, before);
    assert_eq!(app.undo_len(), 0);
    frame(
        &ctx,
        &mut app,
        vec![button(field + egui::vec2(40.0, 0.0), false)],
    );
    frame(&ctx, &mut app, Vec::new());
    let after = app.surface().unwrap().points[index].p;
    assert!(after.x > before.x + 0.05, "x {} -> {}", before.x, after.x);
    assert_eq!((after.y, after.z), (before.y, before.z));
    assert_eq!(app.undo_len(), 1, "a drag is one undo entry");
}
