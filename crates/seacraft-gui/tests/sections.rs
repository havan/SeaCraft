//! Intersection curves and curvature combs, headless (R5-9): two toggles
//! that draw independently, each changing the frame key; the station,
//! waterline and buttock counts re-cut the curves; the comb scale and
//! density change the combs; the toolbar and the control channel reach all
//! of it.

use eframe::egui;
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
    out.textures_delta.clear();
    out
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn click_toolbar(ctx: &egui::Context, app: &mut App, name: &str) {
    let pos = app
        .toolbar_rect(name)
        .unwrap_or_else(|| panic!("toolbar control `{name}` was not drawn"))
        .center();
    frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
    frame(ctx, app, vec![button(pos, true)]);
    frame(ctx, app, vec![button(pos, false)]);
    frame(ctx, app, Vec::new());
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn demo() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

/// How many line segments the frame drew in the section-curve colour and
/// in the comb-spike colour.
fn strokes(out: &egui::FullOutput) -> (usize, usize) {
    fn walk(shape: &egui::Shape, curves: &mut usize, spikes: &mut usize) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, curves, spikes)),
            egui::Shape::LineSegment { stroke, .. } => {
                if stroke.color == theme::SECTION_CURVE {
                    *curves += 1;
                } else if stroke.color == theme::COMB_SPIKE {
                    *spikes += 1;
                }
            }
            _ => {}
        }
    }
    let (mut curves, mut spikes) = (0, 0);
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut curves, &mut spikes);
    }
    (curves, spikes)
}

/// Draw two frames (the cache refreshes on the first) and count the last.
fn drawn(ctx: &egui::Context, app: &mut App) -> (usize, usize) {
    frame(ctx, app, Vec::new());
    strokes(&frame(ctx, app, Vec::new()))
}

fn keys(app: &App) -> [Option<u64>; 4] {
    std::array::from_fn(|i| app.analysis_frame_key(i))
}

#[test]
fn curves_and_combs_toggle_independently_in_the_drawing() {
    let (mut app, ctx) = demo();
    assert!(!app.display().curves && !app.display().combs);
    assert_eq!(drawn(&ctx, &mut app), (0, 0), "nothing by default");

    app.display_mut().curves = true;
    let (curves, spikes) = drawn(&ctx, &mut app);
    assert!(curves > 50, "the curves are drawn: {curves}");
    assert_eq!(spikes, 0, "no combs with the curves alone");

    app.display_mut().combs = true;
    let (both_curves, both_spikes) = drawn(&ctx, &mut app);
    assert_eq!(
        both_curves, curves,
        "the combs leave the curves as they were"
    );
    assert!(both_spikes > 50, "the combs are drawn: {both_spikes}");

    app.display_mut().curves = false;
    let (no_curves, comb_spikes) = drawn(&ctx, &mut app);
    assert_eq!(no_curves, 0, "the combs alone draw no curves");
    assert_eq!(comb_spikes, both_spikes, "the same combs without the lines");

    // Density 1/2 draws about half the spikes.
    app.display_mut().comb_stride = 2;
    let (_, half) = drawn(&ctx, &mut app);
    assert!(
        half < comb_spikes && half * 2 >= comb_spikes - 40,
        "density 1/2: {half} of {comb_spikes}"
    );
}

#[test]
fn each_toggle_and_count_changes_the_frame_key() {
    let (mut app, ctx) = demo();
    app.set_analysis(seacraft_core::AnalysisMode::Zebra { stripes: 14.0 });
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let base = keys(&app);
    assert!(base.iter().all(Option::is_some), "every view has a frame");

    let mut seen = vec![base];
    let mut step = |app: &mut App, what: &str, edit: &dyn Fn(&mut App)| {
        edit(app);
        frame(&ctx, app, Vec::new());
        frame(&ctx, app, Vec::new());
        let now = keys(app);
        for (i, key) in now.iter().enumerate() {
            assert!(
                seen.iter().all(|k| k[i] != *key),
                "{what}: view {i}'s frame key did not change"
            );
        }
        seen.push(now);
    };
    step(&mut app, "curves on", &|a| a.display_mut().curves = true);
    step(&mut app, "combs on", &|a| a.display_mut().combs = true);
    step(&mut app, "curves off", &|a| a.display_mut().curves = false);
    step(&mut app, "stations", &|a| a.display_mut().stations = 21);
    step(&mut app, "waterlines", &|a| a.display_mut().waterlines = 9);
    step(&mut app, "buttocks", &|a| a.display_mut().buttocks = 7);
    step(&mut app, "comb scale", &|a| {
        a.display_mut().comb_scale = 1.2
    });
    step(&mut app, "comb density", &|a| {
        a.display_mut().comb_stride = 3
    });
    // Both off again: back to the key the views started with, whatever the
    // counts, so turning the overlays off costs no re-raster of its own.
    app.display_mut().combs = false;
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(keys(&app), base, "both off: the original frame key");

    // With both off the counts do not matter: no pointless re-raster.
    let before = keys(&app);
    let rasters = app.raster_counts();
    app.display_mut().stations = 30;
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(keys(&app), before);
    assert_eq!(app.raster_counts(), rasters);
}

#[test]
fn the_counts_recut_the_curves_and_the_scale_the_combs() {
    let (mut app, ctx) = demo();
    assert_eq!(app.section_curve_counts(), None, "nothing cut while off");
    app.display_mut().curves = true;
    frame(&ctx, &mut app, Vec::new());
    let (st, bt, wl, spikes) = app.section_curve_counts().expect("curves cut");
    assert!(st >= 9 && bt >= 3 && wl >= 5, "{st} {bt} {wl}");
    assert_eq!(spikes, 0, "no comb spikes sampled while the combs are off");

    app.display_mut().stations = 21;
    app.display_mut().waterlines = 13;
    app.display_mut().buttocks = 9;
    frame(&ctx, &mut app, Vec::new());
    let (st2, bt2, wl2, _) = app.section_curve_counts().unwrap();
    assert!(st2 > st && bt2 > bt && wl2 > wl, "{st2} {bt2} {wl2}");

    // The comb scale changes the spikes' length, not their number.
    app.display_mut().combs = true;
    let tip_reach = |app: &mut App, ctx: &egui::Context| {
        let out = {
            frame(ctx, app, Vec::new());
            frame(ctx, app, Vec::new())
        };
        let mut total = 0.0f32;
        for clipped in &out.shapes {
            if let egui::Shape::LineSegment { points, stroke } = &clipped.shape {
                if stroke.color == theme::COMB_SPIKE {
                    total += points[0].distance(points[1]);
                }
            }
        }
        total
    };
    app.display_mut().comb_scale = 0.2;
    let short = tip_reach(&mut app, &ctx);
    app.display_mut().comb_scale = 1.0;
    let long = tip_reach(&mut app, &ctx);
    assert!(
        long > short * 1.5,
        "gain 1.0 reaches further than 0.2: {long} vs {short}"
    );
}

#[test]
fn the_toolbar_has_separate_curve_and_comb_toggles() {
    let (mut app, ctx) = demo();
    click_toolbar(&ctx, &mut app, "curves");
    assert!(app.display().curves && !app.display().combs);
    click_toolbar(&ctx, &mut app, "combs");
    assert!(app.display().curves && app.display().combs);
    click_toolbar(&ctx, &mut app, "curves");
    assert!(!app.display().curves && app.display().combs);
    // The settings menu opens from the toolbar.
    assert!(app
        .toolbar_rect("sections")
        .is_some_and(|r| r.is_positive()));
}

#[test]
fn the_linesplan_export_uses_the_viewport_counts() {
    let (mut app, _ctx) = demo();
    let dir = std::env::temp_dir().join(format!("seacraft-gui-sections-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let few = dir.join("few.svg");
    let many = dir.join("many.svg");
    app.display_mut().stations = 3;
    assert!(app.export_to(seacraft_gui::app::ExportKind::Linesplan, &few));
    app.display_mut().stations = 41;
    assert!(app.export_to(seacraft_gui::app::ExportKind::Linesplan, &many));
    let size = |p: &std::path::Path| std::fs::metadata(p).unwrap().len();
    assert!(size(&many) > size(&few), "more stations, more drawing");
    let _ = std::fs::remove_dir_all(dir);
}
