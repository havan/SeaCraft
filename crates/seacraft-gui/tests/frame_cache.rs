//! The viewports' frame cache, headless: an analysis mode rasters only the
//! views whose frame changed, a drag re-rasters only the view it happens in
//! (the others refresh once on release), the grid shows under an analysis
//! frame, maximising or restoring a view refits its camera, the waterline is
//! cut once per mesh and plane, and each view keeps its waterline as one mesh.

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

/// The app's cached waterline is the cut of the mesh it is drawing with the
/// waterplane of the current floating condition, in the same order.
fn assert_waterline_current(app: &App) {
    let options = app.hydro_options();
    let plane = seacraft_core::hydro::waterplane(
        app.surface().expect("model loaded"),
        options.draft,
        options.trim,
        options.heel_deg,
    );
    let mesh = app.render_cache().expect("mesh refined");
    let cached = app.waterline().expect("waterline cut");
    assert!(!cached.is_empty(), "the demo hull crosses its waterplane");
    let fresh = seacraft_gui::render::waterline_segments(mesh, &plane);
    assert_eq!(cached, fresh.as_slice());
}

fn shaded_demo() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    app.recompute_hydrostatics();
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

#[test]
fn orbiting_or_hovering_does_not_recut_the_waterline() {
    let (mut app, ctx) = shaded_demo();
    let cuts = app.waterline_cuts();
    assert!(cuts >= 1, "the first frames cut the waterline");
    let over = app.viewport_rect(0).center();
    for _ in 0..3 {
        app.cameras_mut()[0].orbit(3.0, 1.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over)]);
    }
    assert_eq!(app.waterline_cuts(), cuts);
}

#[test]
fn a_new_draft_recuts_the_waterline_on_the_next_frame() {
    let (mut app, ctx) = shaded_demo();
    let cuts = app.waterline_cuts();
    let before = app.waterline().expect("waterline cut").to_vec();
    app.hydro_options_mut().draft *= 0.8;
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.waterline_cuts(), cuts + 1);
    assert_ne!(app.waterline().expect("waterline cut"), before.as_slice());
    assert_waterline_current(&app);
}

#[test]
fn a_new_trim_or_heel_recuts_the_waterline() {
    let (mut app, ctx) = shaded_demo();
    app.hydro_options_mut().trim = 0.1;
    frame(&ctx, &mut app, Vec::new());
    assert_waterline_current(&app);
    app.hydro_options_mut().heel_deg = 10.0;
    frame(&ctx, &mut app, Vec::new());
    assert_waterline_current(&app);
}

#[test]
fn a_new_mesh_recuts_the_waterline() {
    let (mut app, ctx) = shaded_demo();
    app.set_subdivision_level(1);
    frame(&ctx, &mut app, Vec::new());
    assert_waterline_current(&app);
}

#[test]
fn dragging_a_point_keeps_the_waterline_on_the_previewed_mesh() {
    let (mut app, ctx) = shaded_demo();
    let index = {
        let surface = app.surface().expect("model loaded");
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .min_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .expect("a keel point")
    };
    let before = app.surface().unwrap().points[index].p;
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1]
        .transform()
        .project(before, rect)
        .expect("point visible");
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    for dy in [10.0, 20.0, 30.0] {
        let at = screen + egui::vec2(0.0, dy);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
    }
    assert_ne!(
        app.surface().unwrap().points[index].p,
        before,
        "the point moved"
    );
    assert_waterline_current(&app);
}

#[test]
fn switching_the_waterplane_off_cuts_nothing_until_it_returns() {
    let (mut app, ctx) = shaded_demo();
    app.display_mut().waterplane = false;
    app.set_subdivision_level(1);
    let cuts = app.waterline_cuts();
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.waterline_cuts(), cuts);
    app.display_mut().waterplane = true;
    frame(&ctx, &mut app, Vec::new());
    assert_waterline_current(&app);
}

/// What egui makes of the waterline's strokes in view `index`: the strokes the
/// viewport draws, tessellated here, afresh, with the context's own options.
fn tessellated_waterline(ctx: &egui::Context, app: &App, index: usize) -> egui::Mesh {
    let rect = app.viewport_rect(index);
    let shapes = seacraft_gui::render::waterline_shapes(
        &app.cameras()[index].transform(),
        rect,
        app.waterline().expect("waterline cut"),
    );
    let mut tessellator = egui::epaint::Tessellator::new(
        ctx.pixels_per_point(),
        ctx.tessellation_options(|o| *o),
        ctx.fonts(|f| f.font_image_size()),
        Vec::new(),
    );
    tessellator.set_clip_rect(rect);
    let mut mesh = egui::Mesh::default();
    for shape in shapes {
        tessellator.tessellate_shape(shape, &mut mesh);
    }
    mesh
}

fn assert_waterline_mesh_fresh(ctx: &egui::Context, app: &App, index: usize) {
    let cached = app
        .waterline_mesh(index)
        .unwrap_or_else(|| panic!("view {index} keeps a waterline mesh"));
    let fresh = tessellated_waterline(ctx, app, index);
    assert!(
        !fresh.vertices.is_empty(),
        "view {index} shows the waterline"
    );
    assert_eq!(cached.vertices, fresh.vertices, "view {index}");
    assert_eq!(cached.indices, fresh.indices, "view {index}");
}

fn drawn_meshes(shape: &egui::Shape, out: &mut Vec<std::sync::Arc<egui::Mesh>>) {
    match shape {
        egui::Shape::Vec(shapes) => shapes.iter().for_each(|s| drawn_meshes(s, out)),
        egui::Shape::Mesh(mesh) => out.push(mesh.clone()),
        _ => {}
    }
}

#[test]
fn each_view_draws_a_kept_waterline_mesh_and_only_a_moved_view_rebuilds_it() {
    use std::sync::Arc;
    let (mut app, ctx) = shaded_demo();
    let out = frame(&ctx, &mut app, Vec::new());
    let mut drawn = Vec::new();
    for clipped in &out.shapes {
        drawn_meshes(&clipped.shape, &mut drawn);
    }
    let before: Vec<Arc<egui::Mesh>> = (0..4)
        .map(|i| {
            assert_waterline_mesh_fresh(&ctx, &app, i);
            let kept = app.waterline_mesh(i).expect("kept").clone();
            assert!(
                drawn.iter().any(|m| Arc::ptr_eq(m, &kept)),
                "view {i} draws its kept mesh"
            );
            kept
        })
        .collect();

    let over = app.viewport_rect(0).center();
    app.cameras_mut()[0].orbit(3.0, 1.0);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(over)]);
    let moved = app.waterline_mesh(0).expect("kept");
    assert!(!Arc::ptr_eq(moved, &before[0]), "the orbited view rebuilt");
    assert_ne!(moved.vertices, before[0].vertices);
    assert_waterline_mesh_fresh(&ctx, &app, 0);
    for (i, kept) in before.iter().enumerate().skip(1) {
        let now = app.waterline_mesh(i).expect("kept");
        assert!(Arc::ptr_eq(now, kept), "view {i} reused its mesh");
    }

    // A new waterplane is a new waterline in every view.
    app.hydro_options_mut().draft *= 0.8;
    frame(&ctx, &mut app, Vec::new());
    for (i, kept) in before.iter().enumerate() {
        let now = app.waterline_mesh(i).expect("kept");
        assert_ne!(now.vertices, kept.vertices, "view {i} rebuilt");
        assert_waterline_mesh_fresh(&ctx, &app, i);
    }
}
