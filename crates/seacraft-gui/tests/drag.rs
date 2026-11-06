//! Headless interaction tests: the app is driven through `egui::Context` with
//! synthetic pointer events, so picking and dragging are verified without a
//! window, a display or a screenshot tool.

use eframe::egui;
use seacraft_gui::App;

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
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
}

/// The demo model, addressed from the crate root so `cargo test` works from
/// any directory.
fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

#[test]
fn dragging_a_control_point_moves_it_and_undo_restores_it() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    // A point on the sheer line, well inside the hull so nothing else is near.
    let index = {
        let surface = app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .expect("a sheer point")
    };
    let before = app.surface().unwrap().points[index].p;
    let rect = app.viewport_rect(1); // profile
    assert!(rect.width() > 100.0, "profile viewport was drawn: {rect:?}");
    let screen = app.cameras()[1]
        .transform()
        .project(before, rect)
        .expect("point visible");
    assert!(rect.contains(screen), "point projects inside the viewport");

    // Press on the point, move 40 px right, release.
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    // Past egui's click threshold: this frame starts the drag.
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(10.0, 0.0))],
    );
    // Every further frame moves the grabbed point.
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(50.0, 0.0))],
    );
    frame(
        &ctx,
        &mut app,
        vec![button(screen + egui::vec2(50.0, 0.0), false)],
    );
    frame(&ctx, &mut app, Vec::new());

    let after = app.surface().unwrap().points[index].p;
    let moved = after - before;
    // The scale is aspect-aware: the smaller of the height and width fits.
    let per_px = app.cameras()[1].transform().world_per_px(rect, 1.0);
    assert!(
        (moved.x - 50.0 * per_px).abs() < 0.05,
        "point moved {moved:?}, expected about {} in x",
        50.0 * per_px
    );
    assert!(moved.z.abs() < 1e-6, "the drag plane keeps z: {moved:?}");
    assert!(app.undo_len() >= 1, "the drag is undoable");
    assert_eq!(app.selected(), Some(index));

    // Ctrl+Z restores the geometry.
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Z,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    );
    let undone = app.surface().unwrap().points[index].p;
    assert!(
        (undone - before).length() < 1e-9,
        "undo restored {undone:?} vs {before:?}"
    );
}

#[test]
fn a_click_without_movement_leaves_no_undo_entry() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    // Two frames: the resizable docks settle on the second.
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let index = {
        let surface = app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .expect("a sheer point")
    };
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1]
        .transform()
        .project(app.surface().unwrap().points[index].p, rect)
        .unwrap();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    frame(&ctx, &mut app, vec![button(screen, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.selected(), Some(index), "the click selected the point");
    assert_eq!(app.undo_len(), 0, "a click is not an edit");
}

#[test]
fn selecting_a_point_keeps_the_redo_stack() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let surface = app.surface().expect("model loaded");
    let index = surface
        .points
        .iter()
        .enumerate()
        .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
        .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
        .map(|(i, _)| i)
        .expect("a sheer point");
    let p = surface.points[index].p;
    app.move_point(index, p + seacraft_core::math::Vec3::new(0.0, 0.0, 0.01))
        .unwrap();
    app.undo();
    assert!(app.can_redo());
    frame(&ctx, &mut app, Vec::new());
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1]
        .transform()
        .project(app.surface().unwrap().points[index].p, rect)
        .unwrap();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    frame(&ctx, &mut app, vec![button(screen, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.selected(), Some(index), "the click selected the point");
    assert!(app.can_redo(), "selecting a point is not an edit");
}

#[test]
fn a_drag_that_misses_every_point_pans_the_camera_instead() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    let rect = app.viewport_rect(2); // plan
    let points_before: Vec<_> = app.surface().unwrap().points.iter().map(|p| p.p).collect();
    let target = app.cameras()[2].target;

    // The top-left corner of the plan view is empty sky.
    let empty = rect.left_top() + egui::vec2(12.0, 12.0);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(empty)]);
    frame(&ctx, &mut app, vec![button(empty, true)]);
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(empty - egui::vec2(10.0, 0.0))],
    );
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(empty - egui::vec2(60.0, 0.0))],
    );
    frame(
        &ctx,
        &mut app,
        vec![button(empty - egui::vec2(60.0, 0.0), false)],
    );

    assert!(
        (app.cameras()[2].target - target).length() > 1e-6,
        "the camera panned instead of moving a point"
    );
    let points_after: Vec<_> = app.surface().unwrap().points.iter().map(|p| p.p).collect();
    assert_eq!(points_before.len(), points_after.len());
    assert!(points_before
        .iter()
        .zip(&points_after)
        .all(|(a, b)| (*a - *b).length() < 1e-12));
    assert_eq!(app.undo_len(), 0, "a pan is not an edit");
}

#[test]
fn symmetric_layer_clamps_dragged_point_at_the_centreline() {
    let mut app = App::headless_for_test(None);
    {
        let surface = app.surface_mut().expect("model");
        *surface = seacraft_core::demo::demo_hull(true);
    }
    app.mark_geometry_changed();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    // Pick a starboard point on the symmetric half-hull in the Plan viewport (+x right, +y up).
    let index = {
        let surface = app.surface().expect("model");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 3.0 && p.p.x < 9.0 && p.p.y > 0.2)
            .min_by(|a, b| a.1.p.y.total_cmp(&b.1.p.y))
            .map(|(i, _)| i)
            .expect("starboard point")
    };
    let before = app.surface().unwrap().points[index].p;
    assert!(before.y > 0.0);
    let rect = app.viewport_rect(2); // plan: +y is up on screen, so dragging down moves toward -y
    let screen = app.cameras()[2]
        .transform()
        .project(before, rect)
        .expect("visible");

    // Drag 250 px down (well past the y = 0 centreline into -y).
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(0.0, 20.0))],
    );
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(0.0, 250.0))],
    );
    frame(
        &ctx,
        &mut app,
        vec![button(screen + egui::vec2(0.0, 250.0), false)],
    );

    let after = app.surface().unwrap().points[index].p;
    assert_eq!(
        after.y, 0.0,
        "symmetric layer must clamp y >= 0, got {after:?}"
    );
}

#[test]
fn hidden_control_points_cannot_be_dragged() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.display_mut().points = false;
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let index = {
        let surface = app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .expect("a sheer point")
    };
    let before = app.surface().unwrap().points[index].p;
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1]
        .transform()
        .project(before, rect)
        .expect("point visible");
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    for dx in [10.0, 50.0] {
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(screen + egui::vec2(dx, 0.0))],
        );
    }
    frame(
        &ctx,
        &mut app,
        vec![button(screen + egui::vec2(50.0, 0.0), false)],
    );
    assert_eq!(app.surface().unwrap().points[index].p, before);
    assert_eq!(app.undo_len(), 0);
}
