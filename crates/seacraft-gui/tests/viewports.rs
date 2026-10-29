//! Viewport framing and navigation, headless: a narrowed cell shows a smaller
//! hull rather than a clipped one, the wheel zooms toward the cursor and the
//! middle button pans.

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
    out.textures_delta.clear();
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn assert_every_point_inside(app: &App, why: &str) {
    let points: Vec<_> = app.surface().unwrap().points.iter().map(|p| p.p).collect();
    for view in 0..4 {
        let rect = app.viewport_rect(view);
        let transform = app.cameras()[view].transform();
        for (i, p) in points.iter().enumerate() {
            let screen = transform
                .project(*p, rect)
                .unwrap_or_else(|| panic!("{why}: view {view}, point {i} behind the camera"));
            assert!(
                rect.contains(screen),
                "{why}: view {view}, point {i} at {screen:?} outside {rect:?}"
            );
        }
    }
}

#[test]
fn every_control_point_stays_inside_its_viewport_with_the_design_dock_open() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_every_point_inside(&app, "docks closed");
    let wide = app.viewport_rect(1);

    app.set_show_loading(true);
    // The docks settle on the second frame.
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert!(
        app.viewport_rect(1).width() < wide.width(),
        "the design dock narrows the viewports"
    );
    assert_every_point_inside(&app, "design dock open");

    // F refits to the narrower cells without leaving any point outside.
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::F,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut app, Vec::new());
    assert_every_point_inside(&app, "after F");
}

#[test]
fn the_wheel_zooms_toward_the_cursor() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    for view in [0, 1] {
        let rect = app.viewport_rect(view);
        let cursor = rect.min + rect.size() * egui::vec2(0.75, 0.3);
        let camera = app.cameras()[view].clone();
        let transform = camera.transform();
        let ray = transform.ray(cursor, rect);
        let t =
            (camera.target - ray.origin).dot(transform.forward) / ray.dir.dot(transform.forward);
        let world = ray.point_at(t);

        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(cursor)]);
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::MouseWheel {
                unit: egui::MouseWheelUnit::Line,
                delta: egui::vec2(0.0, 3.0),
                phase: egui::TouchPhase::Move,
                source: egui::MouseWheelSource::Wheel,
                modifiers: egui::Modifiers::NONE,
            }],
        );
        // egui spreads a wheel step over several frames.
        for _ in 0..30 {
            frame(&ctx, &mut app, Vec::new());
        }
        let after = &app.cameras()[view];
        let zoomed = if view == 0 {
            after.distance < camera.distance * 0.99
        } else {
            after.ortho_height < camera.ortho_height * 0.99
        };
        assert!(zoomed, "view {view}: the wheel zoomed in");
        let p = after.transform().project(world, rect).expect("visible");
        assert!(
            (p - cursor).length() < 0.5,
            "view {view}: the point under the cursor moved to {p:?}, cursor {cursor:?}"
        );
    }
}

#[test]
fn a_middle_drag_pans_every_view() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let points: Vec<_> = app.surface().unwrap().points.iter().map(|p| p.p).collect();
    for view in 0..4 {
        let rect = app.viewport_rect(view);
        let start = rect.center();
        let target = app.cameras()[view].target;
        let middle = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Middle,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start)]);
        frame(&ctx, &mut app, vec![middle(start, true)]);
        for dx in [10.0, 40.0, 80.0] {
            frame(
                &ctx,
                &mut app,
                vec![egui::Event::PointerMoved(start + egui::vec2(dx, 0.0))],
            );
        }
        frame(
            &ctx,
            &mut app,
            vec![middle(start + egui::vec2(80.0, 0.0), false)],
        );
        frame(&ctx, &mut app, Vec::new());
        let moved = app.cameras()[view].target - target;
        let right = app.cameras()[view].transform().right;
        assert!(
            moved.dot(right) < -1e-3,
            "view {view}: dragging right moves the target left, moved {moved:?}"
        );
    }
    let after: Vec<_> = app.surface().unwrap().points.iter().map(|p| p.p).collect();
    assert_eq!(points, after, "a pan is not an edit");
    assert_eq!(app.undo_len(), 0);
}
