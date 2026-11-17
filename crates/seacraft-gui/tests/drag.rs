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

    // A starboard point on the sheer line, well inside the hull so nothing else is near.
    let index = {
        let surface = app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0 && p.p.y > 0.0)
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
    assert!(moved.x > 0.0, "dragging right increases x: {moved:?}");
    let projected_after = app.cameras()[1]
        .transform()
        .project(after, rect)
        .expect("moved point still visible");
    let expected_screen = screen + egui::vec2(50.0, 0.0);
    assert!(
        (projected_after - expected_screen).length() < 1.0,
        "moved point projected to {projected_after:?}, expected {expected_screen:?}"
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
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0 && p.p.y > 0.0)
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
        .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0 && p.p.y > 0.0)
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
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0 && p.p.y > 0.0)
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

#[test]
fn stale_selection_indices_are_dropped_after_undo_redo_and_topology_edit() {
    use seacraft_gui::commands::{self, SplitFaceParams};
    let mut app = App::headless_for_test(Some(&demo_path()));
    let split = commands::split_face(&mut app, &SplitFaceParams { face: 0 }).unwrap();
    let n = app.control_point_count();
    assert_eq!(split.point, n - 1);
    app.select_point(Some(n - 1));
    assert_eq!(app.selected(), Some(n - 1));
    assert!(app.selection().contains(n - 1));
    app.undo();
    assert!(!app.selection().contains(n - 1));
    assert!(app.selected().is_none());
}

#[test]
fn shift_click_adds_a_point_and_a_second_shift_click_removes_it() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    let sheer: Vec<usize> = {
        let s = app.surface().expect("model loaded");
        let mut pts: Vec<usize> = (0..s.points.len())
            .filter(|&i| s.points[i].p.x > 2.0 && s.points[i].p.x < 10.0 && s.points[i].p.y > 0.0)
            .collect();
        pts.sort_by(|&a, &b| s.points[b].p.z.total_cmp(&s.points[a].p.z));
        let mut out: Vec<usize> = Vec::new();
        for i in pts {
            if out
                .iter()
                .all(|&o| (s.points[o].p.x - s.points[i].p.x).abs() > 2.0)
            {
                out.push(i);
            }
        }
        out
    };
    let rect = app.viewport_rect(1);
    let view = app.cameras()[1].transform();
    let pos_a = view
        .project(app.surface().unwrap().points[sheer[0]].p, rect)
        .unwrap();
    let pos_b = view
        .project(app.surface().unwrap().points[sheer[1]].p, rect)
        .unwrap();
    let click = |ctx: &egui::Context, app: &mut App, pos: egui::Pos2, shift: bool| {
        let modifiers = if shift {
            egui::Modifiers::SHIFT
        } else {
            egui::Modifiers::NONE
        };
        let btn = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        frame(ctx, app, vec![egui::Event::PointerMoved(pos), btn(true)]);
        frame(ctx, app, vec![btn(false)]);
        frame(ctx, app, Vec::new());
    };

    click(&ctx, &mut app, pos_a, false);
    let first = app.selected().expect("first point selected");
    assert_eq!(app.selection().points(), &[first]);

    // Shift+click adds the second point and makes it active.
    click(&ctx, &mut app, pos_b, true);
    let second = app.selected().expect("second point active");
    assert_ne!(first, second);
    assert_eq!(app.selection().points(), &[first, second]);
    assert_eq!(app.picked_points(), Some([first, second]));

    // A second Shift+click on the same point removes it and restores `first` as active.
    click(&ctx, &mut app, pos_b, true);
    assert_eq!(app.selection().points(), &[first]);
    assert_eq!(app.selected(), Some(first));
    assert_eq!(app.picked_points(), None);

    // Re-add `second`, then plain-click `first` to collapse the selection to `first`.
    click(&ctx, &mut app, pos_b, true);
    assert_eq!(app.selection().points(), &[first, second]);
    click(&ctx, &mut app, pos_a, false);
    assert_eq!(app.selection().points(), &[first]);
    assert_eq!(app.selected(), Some(first));
    assert_eq!(app.picked_points(), None);
}

#[test]
fn shift_selecting_two_points_and_dragging_moves_both_in_one_undo_step() {
    let mut app = App::headless_for_test(None);
    {
        let surface = app.surface_mut().expect("model");
        *surface = seacraft_core::demo::demo_hull(true);
    }
    app.mark_geometry_changed();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    // Pick two starboard sheer points well separated in x in the Profile viewport.
    let sheer: Vec<usize> = {
        let s = app.surface().expect("model loaded");
        let mut pts: Vec<usize> = (0..s.points.len())
            .filter(|&i| s.points[i].p.x > 2.0 && s.points[i].p.x < 10.0 && s.points[i].p.y > 0.0)
            .collect();
        pts.sort_by(|&a, &b| s.points[b].p.z.total_cmp(&s.points[a].p.z));
        let mut out: Vec<usize> = Vec::new();
        for i in pts {
            if out
                .iter()
                .all(|&o| (s.points[o].p.x - s.points[i].p.x).abs() > 2.0)
            {
                out.push(i);
            }
        }
        out
    };
    let (a, b) = (sheer[0], sheer[1]);
    let before_a = app.surface().unwrap().points[a].p;
    let before_b = app.surface().unwrap().points[b].p;
    let rect = app.viewport_rect(1); // profile
    let view = app.cameras()[1].transform();
    let pos_a = view.project(before_a, rect).unwrap();
    let pos_b = view.project(before_b, rect).unwrap();

    // Click `a`, Shift+click `b`, then drag `a` 40 px right (+x).
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(pos_a), button(pos_a, true)],
    );
    frame(&ctx, &mut app, vec![button(pos_a, false)]);
    frame(&ctx, &mut app, Vec::new());
    let shift_btn = |pos, pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::SHIFT,
    };
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(pos_b), shift_btn(pos_b, true)],
    );
    frame(&ctx, &mut app, vec![shift_btn(pos_b, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.selection().points(), &[a, b]);
    assert_eq!(app.undo_len(), 0);

    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos_a)]);
    frame(&ctx, &mut app, vec![button(pos_a, true)]);
    for dx in [10.0, 40.0] {
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(pos_a + egui::vec2(dx, 0.0))],
        );
    }
    frame(
        &ctx,
        &mut app,
        vec![button(pos_a + egui::vec2(40.0, 0.0), false)],
    );
    frame(&ctx, &mut app, Vec::new());

    let after_a = app.surface().unwrap().points[a].p;
    let after_b = app.surface().unwrap().points[b].p;
    let delta_a = after_a - before_a;
    let delta_b = after_b - before_b;
    assert!(delta_a.x > 0.05, "point a moved in +x: {delta_a:?}");
    assert!(
        (delta_a - delta_b).length() < 1e-9,
        "both selected points move by the same displacement: {delta_a:?} vs {delta_b:?}"
    );
    assert_eq!(app.undo_len(), 1, "one group drag is one undo step");
    app.undo();
    assert!((app.surface().unwrap().points[a].p - before_a).length() < 1e-9);
    assert!((app.surface().unwrap().points[b].p - before_b).length() < 1e-9);

    // Centreline rule in a group drag on the symmetric half-hull:
    // dragging a group that includes a centreline point in Plan view (+x right, +y up)
    // moves both in x while keeping the centreline point at y = 0.
    let cl = (0..app.surface().unwrap().points.len())
        .find(|&i| {
            let p = app.surface().unwrap().points[i].p;
            p.x > 3.0 && p.x < 9.0 && p.y.abs() <= 1e-9
        })
        .expect("a centreline point");
    let before_cl = app.surface().unwrap().points[cl].p;
    assert_eq!(before_cl.y, 0.0);
    app.selection_mut().clear();
    app.selection_mut().add_all([cl, a]);
    frame(&ctx, &mut app, Vec::new());

    let plan_rect = app.viewport_rect(2);
    let plan_pos_a = app.cameras()[2]
        .transform()
        .project(before_a, plan_rect)
        .expect("projected in plan");
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(plan_pos_a)]);
    frame(&ctx, &mut app, vec![button(plan_pos_a, true)]);
    for step in [egui::vec2(10.0, -10.0), egui::vec2(40.0, -30.0)] {
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(plan_pos_a + step)],
        );
    }
    frame(
        &ctx,
        &mut app,
        vec![button(plan_pos_a + egui::vec2(40.0, -30.0), false)],
    );
    frame(&ctx, &mut app, Vec::new());

    let after_a = app.surface().unwrap().points[a].p;
    let after_cl = app.surface().unwrap().points[cl].p;
    assert!(after_a.x - before_a.x > 0.05 && after_a.y - before_a.y > 0.05);
    assert!(
        ((after_cl.x - before_cl.x) - (after_a.x - before_a.x)).abs() < 1e-9,
        "centreline point moves by the same dx"
    );
    assert_eq!(
        after_cl.y, 0.0,
        "centreline point in a group drag stays at y = 0"
    );
    assert_eq!(app.undo_len(), 1);
    app.undo();
    assert!((app.surface().unwrap().points[cl].p - before_cl).length() < 1e-9);
    assert!((app.surface().unwrap().points[a].p - before_a).length() < 1e-9);
}

#[test]
fn model_with_no_symmetric_layer_moves_across_y_zero_freely() {
    // Review Focus 5: on an unsymmetric model (`demo_hull(false)`), neither the centreline lock
    // nor the y >= 0 floor applies; by hand, shifting by (0.1, -0.5, 0.2) subtracts 0.5 from y.
    use seacraft_core::math::Vec3;
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert!(app.surface().unwrap().layers.iter().all(|l| !l.symmetric));
    let (cl, stbd) = {
        let s = app.surface().unwrap();
        let cl = (0..s.points.len())
            .find(|&i| s.points[i].p.y.abs() <= 1e-9)
            .expect("a point at y = 0");
        let stbd = (0..s.points.len())
            .find(|&i| s.points[i].p.y > 0.05 && s.points[i].p.y < 0.4)
            .expect("a point with 0.05 < y < 0.4");
        (cl, stbd)
    };
    let before_cl = app.surface().unwrap().points[cl].p;
    let before_stbd = app.surface().unwrap().points[stbd].p;
    let delta = Vec3::new(0.1, -0.5, 0.2);

    let moved = app.move_points(&[cl, stbd], delta).expect("move_points");
    assert_eq!(moved.len(), 2);
    let after_cl = app.surface().unwrap().points[cl].p;
    let after_stbd = app.surface().unwrap().points[stbd].p;
    assert!((after_cl - (before_cl + delta)).length() < 1e-12);
    assert!((after_stbd - (before_stbd + delta)).length() < 1e-12);
    assert!(after_cl.y < 0.0 && after_stbd.y < 0.0);
    assert_eq!(app.undo_len(), 1);
    assert_eq!(app.selection().points(), &[cl, stbd]);
    assert_eq!(app.selected(), Some(stbd));

    // Empty and out-of-range calls fail without changing the model or undo stack.
    assert!(app.move_points(&[], delta).is_err());
    assert!(app.move_points(&[cl, 999_999], delta).is_err());
    assert_eq!(app.undo_len(), 1);
    assert_eq!(app.surface().unwrap().points[cl].p, after_cl);
}

fn shift_button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::SHIFT,
    }
}

fn marquee_drag(ctx: &egui::Context, app: &mut App, from: egui::Pos2, to: egui::Pos2) {
    let mid = egui::pos2((from.x + to.x) * 0.5, (from.y + to.y) * 0.5);
    frame(ctx, app, vec![egui::Event::PointerMoved(from)]);
    frame(ctx, app, vec![shift_button(from, true)]);
    frame(ctx, app, vec![egui::Event::PointerMoved(mid)]);
    frame(ctx, app, vec![egui::Event::PointerMoved(to)]);
    frame(ctx, app, vec![shift_button(to, false)]);
    frame(ctx, app, Vec::new());
}

#[test]
fn shift_drag_marquee_selects_visible_points_in_dimmed_and_all_in_same_without_panning() {
    let mut app = App::headless_for_test(None);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    let rect = app.viewport_rect(1); // Profile
    let view = app.cameras()[1].transform();
    let target_before = app.cameras()[1].target;

    // Midship station x = 5.20 on the full demo hull: starboard bilge point is visible,
    // port bilge twin is hidden behind the hull in Profile.
    let (sb_idx, pt_idx, from, to) = {
        let s = app.surface().expect("model loaded");
        let sb_idx = s
            .points
            .iter()
            .enumerate()
            .find(|(_, p)| (p.p.x - 5.20).abs() < 1e-6 && p.p.y > 1.0 && p.p.z < 0.8)
            .map(|(i, _)| i)
            .expect("starboard midship bilge");
        let sb_pt = s.points[sb_idx].p;
        let pt_idx = s
            .points
            .iter()
            .enumerate()
            .find(|(_, p)| {
                (p.p.x - sb_pt.x).abs() < 1e-6
                    && (p.p.y + sb_pt.y).abs() < 1e-6
                    && (p.p.z - sb_pt.z).abs() < 1e-6
            })
            .map(|(i, _)| i)
            .expect("port midship bilge twin");

        let station_screen: Vec<egui::Pos2> = s
            .points
            .iter()
            .filter(|p| (p.p.x - 5.20).abs() < 1e-6)
            .map(|p| view.project(p.p, rect).unwrap())
            .collect();
        let x = station_screen[0].x;
        let top = station_screen
            .iter()
            .map(|p| p.y)
            .fold(f32::INFINITY, f32::min);
        let bottom = station_screen
            .iter()
            .map(|p| p.y)
            .fold(f32::NEG_INFINITY, f32::max);
        // Start well above every point in empty sky and sweep past the keel.
        let from = egui::pos2(x - 14.0, (top - 30.0).max(rect.top() + 4.0));
        let to = egui::pos2(x + 14.0, (bottom + 15.0).min(rect.bottom() - 4.0));
        (sb_idx, pt_idx, from, to)
    };
    assert!(rect.contains(from) && rect.contains(to));

    // In Dimmed mode (default) with hull shaded: visible points inside are taken,
    // hidden port twin is skipped, and the camera does not pan.
    marquee_drag(&ctx, &mut app, from, to);
    assert!(
        (app.cameras()[1].target - target_before).length() < 1e-12,
        "Shift+drag marquee must not pan the camera"
    );
    assert!(app.selection().contains(sb_idx));
    assert!(!app.selection().contains(pt_idx));
    assert_eq!(app.selected(), app.selection().points().last().copied());

    // In Hidden mode with hull shaded: hidden port twin is also skipped.
    app.selection_mut().clear();
    app.display_mut().hidden_net = seacraft_gui::render::HiddenNet::Hidden;
    frame(&ctx, &mut app, Vec::new());
    marquee_drag(&ctx, &mut app, from, to);
    assert!(app.selection().contains(sb_idx));
    assert!(!app.selection().contains(pt_idx));

    // In Dimmed mode with shading off: all points inside are taken.
    app.selection_mut().clear();
    app.display_mut().hidden_net = seacraft_gui::render::HiddenNet::Dimmed;
    app.display_mut().shaded = false;
    frame(&ctx, &mut app, Vec::new());
    marquee_drag(&ctx, &mut app, from, to);
    assert!(app.selection().contains(sb_idx));
    assert!(app.selection().contains(pt_idx));

    // In As-drawn (Same) mode with shading on: hidden port twin is also taken.
    app.selection_mut().clear();
    app.display_mut().shaded = true;
    app.display_mut().hidden_net = seacraft_gui::render::HiddenNet::Same;
    frame(&ctx, &mut app, Vec::new());
    marquee_drag(&ctx, &mut app, from, to);
    assert!(app.selection().contains(sb_idx));
    assert!(app.selection().contains(pt_idx));

    // Mid-marquee: the dashed marquee rectangle is drawn only in Profile, clipped to its cell,
    // even when the drag extends outside the Profile cell.
    app.display_mut().orientation = false;
    frame(&ctx, &mut app, Vec::new());
    let outside = egui::pos2(rect.right() + 40.0, to.y);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    frame(&ctx, &mut app, vec![shift_button(from, true)]);
    let mut out = ctx.run_ui(
        egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1600.0, 1000.0),
            )),
            events: vec![egui::Event::PointerMoved(outside)],
            ..Default::default()
        },
        |ui| app.draw(ui),
    );
    out.textures_delta.clear();
    let marquee_clips: Vec<egui::Rect> = out
        .shapes
        .iter()
        .filter(|cs| {
            matches!(
                &cs.shape,
                egui::epaint::Shape::LineSegment { stroke, .. }
                    if stroke.color == seacraft_gui::theme::POINT_SELECTED
                        && (stroke.width - 1.5).abs() < 1e-4
            )
        })
        .map(|cs| cs.clip_rect)
        .collect();
    assert!(!marquee_clips.is_empty(), "marquee dashed lines drawn");
    for clip in &marquee_clips {
        assert_eq!(*clip, rect, "marquee clipped strictly to Profile cell");
    }
    frame(&ctx, &mut app, vec![shift_button(outside, false)]);
    frame(&ctx, &mut app, Vec::new());

    // With control points switched off (`display.points = false`): no marquee, nothing selected.
    app.selection_mut().clear();
    app.display_mut().points = false;
    frame(&ctx, &mut app, Vec::new());
    marquee_drag(&ctx, &mut app, from, to);
    assert!(app.selection().is_empty());
}

#[test]
fn shift_drag_marquee_adds_to_selection_and_tiny_drag_or_escape_does_not() {
    let mut app = App::headless_for_test(None);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    let rect = app.viewport_rect(1); // Profile
    let view = app.cameras()[1].transform();

    // Seed an existing selection at the bow (x = 12.0).
    let bow_pt = app
        .surface()
        .unwrap()
        .points
        .iter()
        .position(|p| (p.p.x - 12.0).abs() < 1e-6)
        .expect("bow point");
    app.select_point(Some(bow_pt));

    // Marquee around midship station x = 5.20 adds to `bow_pt` instead of replacing it.
    let (sb_idx, from, to) = {
        let s = app.surface().unwrap();
        let sb_idx = s
            .points
            .iter()
            .position(|p| (p.p.x - 5.20).abs() < 1e-6 && p.p.y > 1.0 && p.p.z < 0.8)
            .expect("starboard midship bilge");
        let p_screen = view.project(s.points[sb_idx].p, rect).unwrap();
        let top = s
            .points
            .iter()
            .filter(|p| (p.p.x - 5.20).abs() < 1e-6)
            .map(|p| view.project(p.p, rect).unwrap().y)
            .fold(f32::INFINITY, f32::min);
        (
            sb_idx,
            egui::pos2(p_screen.x - 14.0, (top - 30.0).max(rect.top() + 4.0)),
            egui::pos2(p_screen.x + 14.0, p_screen.y + 20.0),
        )
    };
    marquee_drag(&ctx, &mut app, from, to);
    assert!(app.selection().contains(bow_pt));
    assert!(app.selection().contains(sb_idx));
    let before_tiny = app.selection().points().to_vec();
    let active_before_tiny = app.selected();

    // A tiny Shift+drag (< 4 px on both sides) over a different point is a click and selects nothing.
    let stern_pt = app
        .surface()
        .unwrap()
        .points
        .iter()
        .position(|p| p.p.x.abs() < 1e-6 && p.p.y > 0.5)
        .expect("stern starboard point");
    let stern_screen = view
        .project(app.surface().unwrap().points[stern_pt].p, rect)
        .unwrap();
    let tiny_from = stern_screen - egui::vec2(1.5, 1.5);
    let tiny_to = stern_screen + egui::vec2(1.5, 1.5);
    // Start slightly away from `stern_pt` (> PICK_RADIUS) so it's empty space, 2x2 px box.
    let sky_tiny_from = egui::pos2(rect.left() + 12.0, rect.top() + 12.0);
    let sky_tiny_to = sky_tiny_from + egui::vec2(2.0, 2.0);
    marquee_drag(&ctx, &mut app, sky_tiny_from, sky_tiny_to);
    assert_eq!(app.selection().points(), before_tiny.as_slice());
    assert_eq!(app.selected(), active_before_tiny);
    let _ = (tiny_from, tiny_to);

    // Escape during a marquee cancels it without changing the selection.
    let target_before = app.cameras()[1].target;
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    frame(&ctx, &mut app, vec![shift_button(from, true)]);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(to)]);
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut app, vec![shift_button(to, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.selection().points(), before_tiny.as_slice());
    assert_eq!(app.selected(), active_before_tiny);
    assert!((app.cameras()[1].target - target_before).length() < 1e-12);
}

#[test]
fn select_line_adds_station_and_lengthwise_points_keeping_active_and_rejects_invalid() {
    use seacraft_core::netlines::LineKind;
    use seacraft_gui::commands;

    let mut app = App::headless_empty();
    // With no model loaded, select_line and select_points return Err without panicking.
    assert!(commands::select_line(&mut app, LineKind::Transverse).is_err());
    assert!(commands::select_points(&mut app, &[]).is_err());

    let mut app = App::headless_for_test(None);
    {
        let s = app.surface_mut().unwrap();
        *s = seacraft_core::demo::demo_hull(true);
    }
    app.mark_geometry_changed();

    // With no active point, select_line returns Err.
    assert!(commands::select_line(&mut app, LineKind::Transverse).is_err());

    // Pick a point on midship station x = 5.20 (half demo hull: 6 points per station, 9 per lengthwise line).
    let active = app
        .surface()
        .unwrap()
        .points
        .iter()
        .position(|p| (p.p.x - 5.20).abs() < 1e-6 && p.p.y > 0.1 && p.p.z < 0.8)
        .expect("midship point on half demo hull");
    app.select_point(Some(active));

    commands::select_line(&mut app, LineKind::Transverse).expect("select station");
    assert_eq!(app.selection().len(), 6);
    assert_eq!(app.selected(), Some(active));

    app.select_point(Some(active));
    commands::select_line(&mut app, LineKind::Lengthwise).expect("select lengthwise line");
    assert_eq!(app.selection().len(), 9);
    assert_eq!(app.selected(), Some(active));

    // A point on no line of that kind returns Err and leaves selection unchanged.
    {
        let s = app.surface_mut().unwrap();
        *s = seacraft_core::Surface::new();
        s.add_point(seacraft_core::math::Vec3::ZERO);
    }
    app.mark_geometry_changed();
    app.select_point(Some(0));
    assert!(commands::select_line(&mut app, LineKind::Transverse).is_err());
    assert!(commands::select_line(&mut app, LineKind::Lengthwise).is_err());
    assert_eq!(app.selection().points(), &[0]);
    assert_eq!(app.selected(), Some(0));
}

#[test]
fn marquee_with_no_model_is_a_noop() {
    let mut app = App::headless_empty();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    marquee_drag(
        &ctx,
        &mut app,
        egui::pos2(100.0, 100.0),
        egui::pos2(300.0, 300.0),
    );
    assert!(app.selection().is_empty());
}
