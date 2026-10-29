//! The toolbar and the maximised view, headless: the analysis mode and its
//! parameter refresh the cached frames, the level goes through the setter,
//! and Ctrl+Space, the toolbar button and a double-click on a view's title
//! maximise the active view.

use eframe::egui;
use seacraft_core::AnalysisMode;
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

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) {
    frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
    frame(ctx, app, vec![button(pos, true)]);
    frame(ctx, app, vec![button(pos, false)]);
    frame(ctx, app, Vec::new());
}

fn click_toolbar(ctx: &egui::Context, app: &mut App, name: &str) {
    let rect = app
        .toolbar_rect(name)
        .unwrap_or_else(|| panic!("toolbar control `{name}` was not drawn"));
    click(ctx, app, rect.center());
}

/// Press and release Ctrl+Space (egui marks a press of a key still held
/// as a repeat, which does not toggle again).
fn ctrl_space() -> Vec<egui::Event> {
    let key = |pressed| egui::Event::Key {
        key: egui::Key::Space,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
    };
    vec![key(true), key(false)]
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The demo hull at a cheap level, drawn until the docks settle.
fn demo() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

fn quad_rects(app: &App) -> [egui::Rect; 4] {
    std::array::from_fn(|i| app.viewport_rect(i))
}

#[test]
fn the_toolbar_sets_the_analysis_mode_and_its_parameter() {
    let (mut app, ctx) = demo();
    assert_eq!(app.analysis_frame_key(0), None, "shaded needs no raster");

    click_toolbar(&ctx, &mut app, "zebra");
    assert_eq!(
        app.display().analysis,
        AnalysisMode::Zebra { stripes: 14.0 },
        "the default stripe count"
    );
    let zebra = app.analysis_frame_key(0).expect("a zebra frame");

    // Drag the stripe count up.
    let param = app.toolbar_rect("param").expect("the parameter field");
    let start = param.center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(start)]);
    frame(&ctx, &mut app, vec![button(start, true)]);
    for dx in [5.0, 15.0, 30.0] {
        frame(
            &ctx,
            &mut app,
            vec![egui::Event::PointerMoved(start + egui::vec2(dx, 0.0))],
        );
    }
    frame(
        &ctx,
        &mut app,
        vec![button(start + egui::vec2(30.0, 0.0), false)],
    );
    frame(&ctx, &mut app, Vec::new());
    let AnalysisMode::Zebra { stripes } = app.display().analysis else {
        panic!("still zebra: {:?}", app.display().analysis);
    };
    assert!(stripes > 14.0, "dragging right adds stripes: {stripes}");
    let more = app.analysis_frame_key(0).expect("a zebra frame");
    assert_ne!(zebra, more, "a new parameter is a new frame");

    // The parameter survives a trip through another mode.
    click_toolbar(&ctx, &mut app, "curvature");
    assert_eq!(
        app.display().analysis,
        AnalysisMode::Curvature { radius: 1.0 }
    );
    assert_ne!(app.analysis_frame_key(0), Some(more));
    click_toolbar(&ctx, &mut app, "zebra");
    assert_eq!(app.display().analysis, AnalysisMode::Zebra { stripes });
    click_toolbar(&ctx, &mut app, "draft");
    assert_eq!(
        app.display().analysis,
        AnalysisMode::Draft { angle_deg: 3.0 }
    );
    click_toolbar(&ctx, &mut app, "shaded");
    assert_eq!(app.display().analysis, AnalysisMode::Shaded);
}

#[test]
fn the_toolbar_toggles_display_options_and_sets_the_level() {
    let (mut app, ctx) = demo();
    let net = app.display().control_net;
    click_toolbar(&ctx, &mut app, "net");
    assert_eq!(app.display().control_net, !net);
    let combs = app.display().combs;
    click_toolbar(&ctx, &mut app, "combs");
    assert_eq!(app.display().combs, !combs);
    let wire = app.display().wireframe;
    click_toolbar(&ctx, &mut app, "wireframe");
    assert_eq!(app.display().wireframe, !wire);

    // The level list: open it, pick level 1.
    let faces = app.render_cache().expect("a refined mesh").faces.len();
    click_toolbar(&ctx, &mut app, "level");
    click_toolbar(&ctx, &mut app, "level:1");
    assert_eq!(app.subdivision_level(), 1);
    frame(&ctx, &mut app, Vec::new());
    let after = app.render_cache().expect("a refined mesh").faces.len();
    assert!(
        after < faces,
        "the setter refreshed the geometry: {faces} -> {after} faces"
    );

    // Fit refits a camera that was moved off.
    let before = format!("{:?}", app.cameras()[1]);
    app.cameras_mut()[1].zoom(3.0);
    click_toolbar(&ctx, &mut app, "fit");
    assert_eq!(format!("{:?}", app.cameras()[1]), before);
}

#[test]
fn ctrl_space_maximises_the_active_view_and_restores_four() {
    let (mut app, ctx) = demo();
    let four = quad_rects(&app);
    let shaded = app.display().shaded;
    app.set_active_view(1);
    frame(&ctx, &mut app, ctrl_space());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.maximised(), Some(1));
    assert_eq!(app.display().shaded, shaded, "Ctrl+Space is not Space");
    assert_eq!(app.viewport_rect(1), app.central_rect());
    for hidden in [0, 2, 3] {
        assert_eq!(app.viewport_rect(hidden), egui::Rect::NOTHING);
    }
    // 1-4 switch the maximised view.
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::Num3,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        }],
    );
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.maximised(), Some(2));
    assert_eq!(app.viewport_rect(2), app.central_rect());

    frame(&ctx, &mut app, ctrl_space());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.maximised(), None);
    assert_eq!(quad_rects(&app), four, "four cells again");

    // The toolbar button does the same.
    click_toolbar(&ctx, &mut app, "maximise");
    assert_eq!(app.maximised(), Some(2));
    assert_eq!(app.viewport_rect(2), app.central_rect());
    click_toolbar(&ctx, &mut app, "maximise");
    assert_eq!(quad_rects(&app), four);
}

#[test]
fn a_double_click_on_a_title_maximises_that_view() {
    let (mut app, ctx) = demo();
    let four = quad_rects(&app);
    let title = app.title_rect(3);
    assert!(four[3].contains(title.center()));
    let pos = title.center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
    frame(&ctx, &mut app, vec![button(pos, true)]);
    frame(&ctx, &mut app, vec![button(pos, false)]);
    frame(&ctx, &mut app, vec![button(pos, true)]);
    frame(&ctx, &mut app, vec![button(pos, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.maximised(), Some(3));
    assert_eq!(app.active_view(), 3);
    assert_eq!(app.viewport_rect(3), app.central_rect());

    // Double-clicking the title again restores the four views (after the
    // double-click delay, or egui counts a triple click).
    for _ in 0..30 {
        frame(&ctx, &mut app, Vec::new());
    }
    let pos = app.title_rect(3).center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
    frame(&ctx, &mut app, vec![button(pos, true)]);
    frame(&ctx, &mut app, vec![button(pos, false)]);
    frame(&ctx, &mut app, vec![button(pos, true)]);
    frame(&ctx, &mut app, vec![button(pos, false)]);
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.maximised(), None);
    assert_eq!(quad_rects(&app), four);
}

#[test]
fn a_hidden_view_is_not_picked() {
    let (mut app, ctx) = demo();
    let profile = app.viewport_rect(1);
    let profile_view = app.cameras()[1].transform();
    let points: Vec<_> = app.surface().unwrap().points.iter().map(|p| p.p).collect();

    app.set_active_view(0);
    app.set_maximised(true);
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let full = app.viewport_rect(0);
    assert_eq!(full, app.central_rect());
    let perspective = app.cameras()[0].transform();

    // A point the profile view showed, where the perspective view shows no
    // point near the cursor.
    let (index, cursor) = points
        .iter()
        .enumerate()
        .filter_map(|(i, p)| Some((i, profile_view.project(*p, profile)?)))
        .find(|(_, s)| {
            profile.shrink(20.0).contains(*s)
                && points.iter().all(|q| {
                    perspective
                        .project(*q, full)
                        .is_none_or(|t| (t - *s).length() > 30.0)
                })
        })
        .expect("a profile point clear of every perspective point");

    click(&ctx, &mut app, cursor);
    assert_eq!(
        app.active_view(),
        0,
        "the click landed in the maximised view"
    );
    assert_eq!(app.maximised(), Some(0));
    assert_ne!(
        app.selected(),
        Some(index),
        "the hidden profile view picked"
    );
    assert_eq!(app.selected(), None);
}
