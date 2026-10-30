//! The viewports' frame cache, headless: an analysis mode rasters only the
//! views whose frame changed, a drag re-rasters only the view it happens in
//! (the others refresh once on release), the grid shows under an analysis
//! frame, and maximising or restoring a view refits its camera.

use eframe::egui;
use seacraft_core::AnalysisMode;
use seacraft_gui::{theme, App};

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

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// The demo hull at a cheap level in zebra mode, drawn until the docks and
/// the frames settle.
fn zebra_demo() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    app.set_analysis(AnalysisMode::Zebra { stripes: 14.0 });
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    // Measure now so the debounced hydrostatics stay out of the way.
    app.recompute_hydrostatics();
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

fn advanced(before: [u64; 4], after: [u64; 4]) -> [u64; 4] {
    std::array::from_fn(|i| after[i] - before[i])
}

#[test]
fn a_frame_with_nothing_changed_rasters_no_view() {
    let (mut app, ctx) = zebra_demo();
    let counts = app.raster_counts();
    assert!(
        counts.iter().all(|&c| c >= 1),
        "every view rastered its zebra frame: {counts:?}"
    );
    // Hovering over a view repaints but changes nothing.
    let over = app.viewport_rect(2).center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(advanced(counts, app.raster_counts()), [0; 4]);
}

#[test]
fn a_zebra_drag_rasters_only_the_dragged_view_until_release() {
    let (mut app, ctx) = zebra_demo();

    // A sheer point well inside the hull, dragged in the profile view.
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
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(10.0, 0.0))],
    );
    for dx in [14.0, 18.0, 22.0] {
        let counts = app.raster_counts();
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(screen + egui::vec2(dx, 0.0))],
        );
        assert_eq!(
            advanced(counts, app.raster_counts()),
            [0, 1, 0, 0],
            "only the profile view rasters while its point is dragged"
        );
    }
    let moved = app.surface().unwrap().points[index].p;
    assert!((moved - before).length() > 1e-3, "the point moved");

    let counts = app.raster_counts();
    frame(
        &ctx,
        &mut app,
        vec![button(screen + egui::vec2(22.0, 0.0), false)],
    );
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(
        advanced(counts, app.raster_counts()),
        [1; 4],
        "release refreshes every view once"
    );
    let counts = app.raster_counts();
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(advanced(counts, app.raster_counts()), [0; 4]);
}

#[test]
fn the_grid_is_drawn_under_an_analysis_frame() {
    let (mut app, ctx) = zebra_demo();
    let out = frame(&ctx, &mut app, Vec::new());
    // Paint order: the grid's line segments, then the frame's textured mesh.
    let mut order: Vec<&'static str> = Vec::new();
    fn walk(shape: &egui::Shape, order: &mut Vec<&'static str>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, order)),
            egui::Shape::LineSegment { stroke, .. }
                if stroke.color == theme::GRID_MINOR || stroke.color == theme::GRID_MAJOR =>
            {
                order.push("grid")
            }
            egui::Shape::Mesh(mesh) if matches!(mesh.texture_id, egui::TextureId::Managed(id) if id > 0) => {
                order.push("frame")
            }
            _ => {}
        }
    }
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut order);
    }
    let frames = order.iter().filter(|s| **s == "frame").count();
    assert_eq!(frames, 4, "one analysis frame per view: {order:?}");
    let grid = order.iter().filter(|s| **s == "grid").count();
    assert!(grid > 16, "every view draws its grid in zebra mode: {grid}");
    let first_frame = order.iter().position(|s| *s == "frame").unwrap();
    assert_eq!(order[0], "grid", "the grid comes first");
    assert!(first_frame > 0);
}

/// The fraction of `cell` the demo hull's projected bounding box spans,
/// across and down.
fn fill(app: &App, view: usize) -> (f32, f32) {
    let cell = app.viewport_rect(view);
    let (min, max) = app.surface().unwrap().extents();
    let transform = app.cameras()[view].transform();
    let pts: Vec<egui::Pos2> = (0..8)
        .filter_map(|i| {
            let p = seacraft_core::math::Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            );
            transform.project(p, cell)
        })
        .collect();
    let bbox = egui::Rect::from_points(&pts);
    (bbox.width() / cell.width(), bbox.height() / cell.height())
}

#[test]
fn maximising_and_restoring_a_view_refits_its_camera() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(1);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    for view in [0, 1] {
        app.set_active_view(view);

        // Zoomed far out, the hull is a speck in its cell.
        app.cameras_mut()[view].zoom(10.0);
        frame(&ctx, &mut app, Vec::new());
        let (w, h) = fill(&app, view);
        assert!(w < 0.2 && h < 0.2, "view {view} zoomed out: {w} x {h}");

        app.toggle_maximised();
        frame(&ctx, &mut app, Vec::new());
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.maximised(), Some(view));
        let (w, h) = fill(&app, view);
        assert!(
            w >= 0.5 || h >= 0.5,
            "maximised view {view} is refitted: {w} x {h}"
        );

        app.cameras_mut()[view].zoom(10.0);
        app.toggle_maximised();
        frame(&ctx, &mut app, Vec::new());
        frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.maximised(), None);
        let (w, h) = fill(&app, view);
        assert!(
            w >= 0.5 || h >= 0.5,
            "restored view {view} is refitted: {w} x {h}"
        );
    }
}
