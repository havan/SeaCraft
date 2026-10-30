//! The grid's scale at a glance (R6-1), headless at 1600 × 1000: every view
//! names its grid step in metres and follows the adaptive step as it zooms,
//! and each orthographic view carries a scale bar one minor square long.

use eframe::egui;
use seacraft_gui::render::{format_length, grid_steps};
use seacraft_gui::{theme, App};

/// The profile, plan and body-plan views (the perspective view is 0).
const ORTHO: [usize; 3] = [1, 2, 3];

fn frame(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
    out
}

/// Two frames, so the docks settle; the second frame's shapes.
fn settled(ctx: &egui::Context, app: &mut App) -> Vec<egui::Shape> {
    frame(ctx, app);
    let out = frame(ctx, app);
    fn walk(shape: &egui::Shape, acc: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, acc)),
            s => acc.push(s.clone()),
        }
    }
    let mut acc = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut acc);
    }
    acc
}

/// Texts painted inside `rect`.
fn texts_in(shapes: &[egui::Shape], rect: egui::Rect) -> Vec<String> {
    shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::Text(t) if rect.contains(t.pos) => Some(t.galley.text().to_owned()),
            _ => None,
        })
        .collect()
}

/// The grid caption of view `i`.
fn caption(shapes: &[egui::Shape], app: &App, i: usize) -> String {
    let rect = app.viewport_rect(i);
    texts_in(shapes, rect)
        .into_iter()
        .find(|t| t.contains("grid "))
        .unwrap_or_else(|| panic!("view {i} has no grid caption"))
}

/// The minor grid step view `i` draws now.
fn minor_step(app: &App, i: usize) -> (f64, f64) {
    let rect = app.viewport_rect(i);
    let world_per_px = app.cameras()[i].transform().world_per_px(rect, 1.0);
    (grid_steps(1.0 / world_per_px).0, world_per_px)
}

#[test]
fn every_view_names_its_grid_step_and_follows_the_zoom() {
    let mut app = App::headless_for_test(None);
    let ctx = egui::Context::default();
    let shapes = settled(&ctx, &mut app);
    for i in ORTHO {
        let text = caption(&shapes, &app, i);
        let (minor, _) = minor_step(&app, i);
        assert!(
            text.starts_with(&format!("grid {} · major ", format_length(minor))),
            "view {i}: {text}"
        );
        // The bare m/px figure is gone.
        assert!(
            !texts_in(&shapes, app.viewport_rect(i))
                .iter()
                .any(|t| t.contains("m/px")),
            "view {i}"
        );
    }
    let profile = caption(&shapes, &app, 1);
    assert_eq!(profile, "grid 1 m · major 5 m", "the usual zoom");
    let perspective = caption(&shapes, &app, 0);
    assert!(
        perspective.starts_with("ground grid "),
        "the perspective view names its ground grid: {perspective}"
    );

    // Zoomed far out, the step coarsens and the caption says so.
    app.cameras_mut()[1].zoom(12.0);
    let shapes = settled(&ctx, &mut app);
    let zoomed = caption(&shapes, &app, 1);
    let (minor, _) = minor_step(&app, 1);
    assert!(minor > 1.0, "the adaptive step coarsened: {minor}");
    assert_ne!(zoomed, profile);
    assert!(
        zoomed.starts_with(&format!("grid {} · major ", format_length(minor))),
        "{zoomed}"
    );
    // Zoomed in on a detail, sub-metre steps.
    // (One call zooms by at most 20×.)
    app.cameras_mut()[1].zoom(1.0 / 12.0);
    app.cameras_mut()[1].zoom(1.0 / 10.0);
    let shapes = settled(&ctx, &mut app);
    let (minor, _) = minor_step(&app, 1);
    assert!(minor < 1.0, "{minor}");
    assert_eq!(
        caption(&shapes, &app, 1),
        format!(
            "grid {} · major {}",
            format_length(minor),
            format_length(grid_steps(1.0 / minor_step(&app, 1).1).1)
        )
    );
}

#[test]
fn each_orthographic_view_has_a_scale_bar_one_minor_square_long() {
    let mut app = App::headless_for_test(None);
    let ctx = egui::Context::default();
    for zoom in [1.0, 6.0] {
        app.cameras_mut()[2].zoom(zoom);
        let shapes = settled(&ctx, &mut app);
        for i in ORTHO {
            let rect = app.viewport_rect(i);
            let (minor, world_per_px) = minor_step(&app, i);
            // The bar: the longest horizontal stroke in the text colour near
            // the bottom-left corner.
            let corner = egui::Rect::from_min_max(
                rect.left_bottom() + egui::vec2(0.0, -40.0),
                rect.left_bottom() + egui::vec2(rect.width() * 0.5, 0.0),
            );
            let bar = shapes
                .iter()
                .filter_map(|s| match s {
                    egui::Shape::LineSegment { points, stroke }
                        if stroke.color == theme::TEXT_SECONDARY
                            && points[0].y == points[1].y
                            && corner.contains(points[0]) =>
                    {
                        Some((points[1].x - points[0].x).abs())
                    }
                    _ => None,
                })
                .fold(0.0_f32, f32::max);
            let want = (minor / world_per_px) as f32;
            assert!(
                (bar - want).abs() <= 1.0,
                "view {i} at zoom {zoom}: bar {bar} px, one {minor} m step is {want} px"
            );
            // Labelled with its length, on a backing.
            let label = format_length(minor);
            let label_pos = shapes
                .iter()
                .find_map(|s| match s {
                    egui::Shape::Text(t) if corner.contains(t.pos) && t.galley.text() == label => {
                        Some(t.pos)
                    }
                    _ => None,
                })
                .unwrap_or_else(|| panic!("view {i}: no {label} label"));
            assert!(
                shapes.iter().any(|s| matches!(
                    s,
                    egui::Shape::Rect(r) if r.fill == theme::HUD_BACKING && r.rect.contains(label_pos)
                )),
                "view {i}: the scale bar has no backing"
            );
        }
        // The perspective view has none (its scale changes with depth).
        let persp = app.viewport_rect(0);
        let corner = egui::Rect::from_min_max(
            persp.left_bottom() + egui::vec2(0.0, -40.0),
            persp.left_bottom() + egui::vec2(persp.width() * 0.5, 0.0),
        );
        assert!(!shapes.iter().any(|s| matches!(
            s,
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == theme::TEXT_SECONDARY && corner.contains(points[0])
        )));
    }
}
