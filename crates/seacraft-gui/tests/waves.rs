//! The far-field wave pattern in the window, headless: it is computed in a job while shown, an
//! out-of-date pattern is never the current one, and a failure is reported once.
//!
//! The waits are generous guards (a debug build on a loaded machine), not speed checks.

use std::time::{Duration, Instant};

use eframe::egui;
use std::sync::Arc;

use seacraft_gui::analysis_views::AnalysisView;
use seacraft_gui::camera::{Camera, ViewKind};
use seacraft_gui::render::{self, Scene, SceneWaves};
use seacraft_gui::theme;
use seacraft_gui::waves::{compute_waves, WaveResult, WaveSettings, WaveSpeed};
use seacraft_gui::App;

const PATIENCE: Duration = Duration::from_secs(120);

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            // Tall, so the Performance tab's sections are on screen.
            egui::vec2(1600.0, 3200.0),
        )),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    // Nothing uploads the fonts in a headless pass; a dropped delta panics.
    out.textures_delta.clear();
    out
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

fn path(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The demo hull at level 2, two frames drawn.
fn demo() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&path("demo-hull.fef")));
    app.set_subdivision_level(2);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

/// Draw frames until `done` holds, failing after [`PATIENCE`].
fn frames_until(
    ctx: &egui::Context,
    app: &mut App,
    what: &str,
    done: impl Fn(&App) -> bool,
) -> egui::FullOutput {
    let t0 = Instant::now();
    loop {
        let out = frame(ctx, app, Vec::new());
        if done(app) {
            return out;
        }
        assert!(t0.elapsed() < PATIENCE, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(30));
    }
}

/// The demo hull with its pattern shown and computed.
/// The demo hull with the Wave pattern view open and its pattern computed.
fn demo_with_waves() -> (App, egui::Context) {
    let (mut app, ctx) = demo();
    app.open_analysis_view(AnalysisView::Waves);
    frames_until(&ctx, &mut app, "the wave pattern", |a| {
        a.wave_current().is_some()
    });
    (app, ctx)
}

/// The demo hull with the Plan view contours on (the view closed) and the pattern computed.
fn demo_with_plan_waves() -> (App, egui::Context) {
    let (mut app, ctx) = demo();
    app.set_wave_settings(WaveSettings {
        plan_contours: true,
        ..app.wave_settings().clone()
    });
    frames_until(&ctx, &mut app, "the wave pattern", |a| {
        a.wave_current().is_some()
    });
    (app, ctx)
}

#[test]
fn open_view_computes_by_itself() {
    let (mut app, ctx) = demo();
    app.open_analysis_view(AnalysisView::Waves);
    assert_eq!(app.analysis_view(), Some(AnalysisView::Waves));
    frames_until(&ctx, &mut app, "the pattern and its drag curve", |a| {
        a.wave_current().is_some() && a.wave_drag().is_some()
    });
    let w = app.wave_current().expect("the pattern");
    assert!((w.field.froude - 0.3).abs() < 1e-12);
    assert!(!app.wave_settings().plan_contours, "Plan contours stay off");
}

#[test]
fn plan_contours_alone_compute_the_pattern() {
    let (app, _ctx) = demo_with_plan_waves();
    assert_eq!(app.analysis_view(), None);
    assert!(app.wave_current().is_some());
}

/// Review focus 1: ten speeds one frame apart; the view settles on the last, and never shows an
/// older speed's field as current.
#[test]
fn scrubbing_settles_on_the_last_speed() {
    let (mut app, ctx) = demo_with_waves();
    for k in 0..10 {
        let fn_ = 0.30 + 0.02 * k as f64;
        app.set_wave_settings(WaveSettings {
            speed: WaveSpeed::Froude(fn_),
            ..app.wave_settings().clone()
        });
        // The demo starts at Fn 0.30, so the first step does not move the key.
        if k > 0 {
            assert!(app.wave_current().is_none(), "Fn {fn_}: the key moved");
        }
        frame(&ctx, &mut app, Vec::new());
        if let Some(w) = app.wave_current() {
            assert!(
                (w.field.froude - fn_).abs() < 1e-9,
                "{} at Fn {fn_}",
                w.field.froude
            );
        }
    }
    frames_until(&ctx, &mut app, "the last speed", |a| {
        a.wave_current().is_some()
    });
    let froude = app.wave_current().unwrap().field.froude;
    assert!((froude - 0.48).abs() < 1e-9, "Fn {froude}");
}

#[test]
fn a_settings_change_hides_the_old_pattern_at_once() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.4),
        ..app.wave_settings().clone()
    });
    assert!(
        app.wave_current().is_none(),
        "the Fn 0.3 pattern is out of date"
    );
    frames_until(&ctx, &mut app, "the Fn 0.4 pattern", |a| {
        a.wave_current().is_some()
    });
    let froude = app.wave_current().unwrap().field.froude;
    assert!((froude - 0.4).abs() < 1e-12, "Fn {froude}");
}

#[test]
fn a_model_edit_or_new_level_hides_the_old_pattern() {
    let (mut app, ctx) = demo_with_waves();
    app.set_subdivision_level(1);
    assert!(
        app.wave_current().is_none(),
        "a new level hides the pattern"
    );
    // Another model opened while the level-1 job may still run: the demo's result never returns.
    frame(&ctx, &mut app, Vec::new());
    std::thread::sleep(Duration::from_millis(350));
    frame(&ctx, &mut app, Vec::new());
    let demo_l = {
        let (a, _) = demo_with_waves();
        a.wave_current().unwrap().length_wl
    };
    app.open_model(std::path::Path::new(&path("wigley.fef")))
        .expect("open wigley.fef");
    assert!(
        app.wave_current().is_none(),
        "a new model hides the pattern"
    );
    frames_until(&ctx, &mut app, "the Wigley pattern", |a| {
        a.wave_current().is_some()
    });
    let l = app.wave_current().unwrap().length_wl;
    assert!(
        (l - demo_l).abs() > 1e-6,
        "the Wigley's L {l}, not the demo's"
    );
}

#[test]
fn a_failing_pattern_is_reported_once() {
    let (mut app, ctx) = demo();
    app.set_wave_view_open(true);
    app.hydro_options_mut().draft = 0.0;
    frames_until(&ctx, &mut app, "the failure", |a| {
        a.wave_failure().is_some()
    });
    for _ in 0..20 {
        frame(&ctx, &mut app, Vec::new());
        std::thread::sleep(Duration::from_millis(20));
    }
    let lines = app
        .messages()
        .entries()
        .filter(|m| m.text.contains("Wave pattern failed"))
        .count();
    assert_eq!(lines, 1, "one log line for one failure");
    let failure = app.wave_failure().unwrap();
    assert!(failure.contains("Michell"), "{failure}");
    assert!(!app.waves_busy(), "a failed pattern is not retried");
    assert!(app.wave_current().is_none());
}

/// Review focus 2: no waterline at draft 0 shows the failure once in the view, with no panic.
#[test]
fn draft_zero_shows_the_failure_in_the_view() {
    let (mut app, ctx) = demo();
    app.open_analysis_view(AnalysisView::Waves);
    app.hydro_options_mut().draft = 0.0;
    frames_until(&ctx, &mut app, "the failure", |a| {
        a.wave_failure().is_some()
    });
    frame(&ctx, &mut app, Vec::new());
    let texts = texts(&frame(&ctx, &mut app, Vec::new()));
    let shown = texts
        .iter()
        .filter(|t| t.starts_with("Wave pattern failed"))
        .count();
    assert_eq!(shown, 1, "{texts:?}");
    assert!(app.wave_drag().is_none());
}

#[test]
fn closed_view_computes_nothing() {
    let (mut app, ctx) = demo();
    for _ in 0..20 {
        frame(&ctx, &mut app, Vec::new());
        std::thread::sleep(Duration::from_millis(20));
        assert!(!app.waves_busy());
    }
    assert!(app.wave_current().is_none());
    assert!(!app.wave_settings().plan_contours);
}

#[test]
fn blocking_compute_makes_the_pattern_current() {
    let (mut app, _ctx) = demo();
    app.set_wave_view_open(true);
    app.compute_waves_blocking().expect("the pattern");
    assert!(app.wave_current().is_some());
}

/// Every shape, flattened.
fn flat(shapes: impl IntoIterator<Item = egui::Shape>) -> Vec<egui::Shape> {
    fn walk(shape: egui::Shape, out: &mut Vec<egui::Shape>) {
        match shape {
            egui::Shape::Vec(v) => v.into_iter().for_each(|s| walk(s, out)),
            other => out.push(other),
        }
    }
    let mut out = Vec::new();
    for s in shapes {
        walk(s, &mut out);
    }
    out
}

/// A crest contour, as a stroke or inside the tessellated contour mesh the app keeps per view.
fn is_crest_contour(shape: &egui::Shape) -> bool {
    match shape {
        egui::Shape::Mesh(m) => m
            .vertices
            .iter()
            .any(|v| v.color == theme::WAVE_CONTOUR_CREST),
        other => stroke_colour(other) == Some(theme::WAVE_CONTOUR_CREST),
    }
}

fn stroke_colour(shape: &egui::Shape) -> Option<egui::Color32> {
    match shape {
        egui::Shape::LineSegment { stroke, .. } => Some(stroke.color),
        egui::Shape::Path(p) => match p.stroke.color {
            egui::epaint::ColorMode::Solid(c) => Some(c),
            _ => None,
        },
        _ => None,
    }
}

/// The demo hull at level 2, and its pattern with `settings`.
fn demo_parts(
    settings: &WaveSettings,
) -> (
    seacraft_core::mesh::Surface,
    seacraft_core::export::RenderMesh,
    WaveResult,
) {
    let text = std::fs::read_to_string(path("demo-hull.fef")).unwrap();
    let surface = seacraft_core::fef::parse(&text).unwrap();
    let subdiv = seacraft_core::subdiv::SubdivisionOptions {
        level: 2,
        ..Default::default()
    };
    let mesh = seacraft_core::export::render_mesh(&surface, 2, Default::default());
    let draft = surface.settings.draft;
    let w = compute_waves(&surface, subdiv, draft, 1025.0, settings).unwrap();
    (surface, mesh, w)
}

fn scene<'a>(
    surface: &'a seacraft_core::mesh::Surface,
    mesh: &'a seacraft_core::export::RenderMesh,
    w: &'a WaveResult,
) -> Scene<'a> {
    let (min, max) = render::display_extents(surface);
    Scene {
        surface,
        mesh,
        curves: None,
        volumes: &[],
        windage: &[],
        min,
        max,
        draft: w.draft,
        trim: 0.0,
        heel_deg: 0.0,
        waterline: None,
        waves: Some(SceneWaves {
            result: w,
            contours: None,
        }),
        waves_status: None,
    }
}

/// The shapes one viewport draws.
fn draw_view(
    scene: &Scene,
    kind: ViewKind,
    texture: Option<&egui::TextureHandle>,
    gpu_hull: Option<egui::PaintCallback>,
) -> Vec<egui::Shape> {
    let ctx = egui::Context::default();
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 600.0));
    let camera = Camera::new(kind, (scene.min + scene.max) * 0.5, scene.max - scene.min);
    let input = egui::RawInput {
        screen_rect: Some(rect),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| {
        render::viewport_with_texture(
            ui,
            rect,
            &camera,
            Some(scene),
            &render::DisplayOptions::default(),
            true,
            &Default::default(),
            None,
            None,
            None,
            &[],
            None,
            texture,
            None,
            gpu_hull.clone(),
            None,
        )
    });
    out.textures_delta.clear();
    flat(out.shapes.into_iter().map(|c| c.shape))
}

/// The contour and wash-line strokes, as text.
fn wave_marks(shapes: &[egui::Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter(|s| {
            stroke_colour(s).is_some_and(|c| {
                c == theme::WAVE_CONTOUR_CREST
                    || c == theme::WAVE_CONTOUR_TROUGH
                    || c == theme::WAVE_CUT
            })
        })
        .map(|s| format!("{s:?}"))
        .collect()
}

#[test]
fn both_hull_paths_draw_the_same_waves() {
    let (surface, mesh, w) = demo_parts(&WaveSettings::default());
    let scene = scene(&surface, &mesh, &w);
    let painted = wave_marks(&draw_view(&scene, ViewKind::Plan, None, None));
    let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(900.0, 600.0));
    let callback = egui::PaintCallback {
        rect,
        callback: Arc::new(0u8),
    };
    let gpu = wave_marks(&draw_view(&scene, ViewKind::Plan, None, Some(callback)));
    let ctx = egui::Context::default();
    let analysis = ctx.load_texture(
        "analysis",
        egui::ColorImage::filled([2, 2], egui::Color32::GRAY),
        egui::TextureOptions::LINEAR,
    );
    let rastered = wave_marks(&draw_view(&scene, ViewKind::Plan, Some(&analysis), None));
    assert!(painted.len() > 10, "contours are drawn: {}", painted.len());
    assert_eq!(
        dashed_rows(
            &draw_view(&scene, ViewKind::Plan, None, None),
            theme::WAVE_CUT
        )
        .len(),
        2,
        "a wash line each side"
    );
    assert_eq!(painted, gpu, "the GPU hull path draws the same waves");
    assert_eq!(painted, rastered, "the analysis path draws the same waves");
}

#[test]
fn waves_only_in_plan_view() {
    let (surface, mesh, w) = demo_parts(&WaveSettings::default());
    let scene = scene(&surface, &mesh, &w);
    for kind in [ViewKind::Profile, ViewKind::Perspective, ViewKind::Body] {
        let shapes = draw_view(&scene, kind, None, None);
        assert!(wave_marks(&shapes).is_empty(), "{kind:?} draws no waves");
    }
}

#[test]
fn catamaran_pattern_lies_over_the_model() {
    let settings = WaveSettings {
        cat_spacing_m: Some(8.0),
        ..WaveSettings::default()
    };
    let (surface, mesh, w) = demo_parts(&settings);
    let scene = scene(&surface, &mesh, &w);
    let shapes = draw_view(&scene, ViewKind::Plan, None, None);
    let wave_at = shapes
        .iter()
        .position(|s| {
            stroke_colour(s)
                .is_some_and(|c| c == theme::WAVE_CONTOUR_CREST || c == theme::WAVE_CONTOUR_TROUGH)
        })
        .expect("the contours");
    let hull_at = shapes
        .iter()
        .position(|s| matches!(s, egui::Shape::Mesh(_)))
        .expect("the hull");
    assert!(hull_at < wave_at, "the contours are drawn over the hull");
    let outlines = shapes
        .iter()
        .filter(|s| stroke_colour(s) == Some(theme::WAVE_WATERLINE))
        .count();
    assert_eq!(outlines, 2, "one waterline outline per demihull");
    let caption = render::wave_caption(&w);
    assert!(caption.contains("catamaran s = 8.00 m"), "{caption}");
}

#[test]
fn stale_pattern_is_not_drawn() {
    let (mut app, ctx) = demo_with_plan_waves();
    let contoured = |out: &egui::FullOutput| {
        flat(out.shapes.iter().map(|c| c.shape.clone()))
            .iter()
            .any(is_crest_contour)
    };
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(contoured(&out), "the current pattern is drawn");
    assert!(texts(&out)
        .iter()
        .any(|t| t.starts_with("Waves · Fn 0.300")));
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.45),
        ..app.wave_settings().clone()
    });
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(
        !contoured(&out),
        "the Fn 0.3 pattern is not drawn as current"
    );
    assert!(
        texts(&out).iter().any(|t| t == "Wave contours: computing…"),
        "{:?}",
        texts(&out)
    );
}

/// The Performance tab's texts, two frames after opening it.
fn performance_texts(ctx: &egui::Context, app: &mut App) -> Vec<String> {
    app.set_dock_tab(seacraft_gui::app::DesignDock::Performance);
    frame(ctx, app, Vec::new());
    texts(&frame(ctx, app, Vec::new()))
}

#[test]
fn menu_lists_wave_pattern_after_trim_scan() {
    let all = AnalysisView::ALL;
    let trim = all
        .iter()
        .position(|v| *v == AnalysisView::TrimScan)
        .unwrap();
    assert_eq!(all[trim + 1], AnalysisView::Waves);
    assert_eq!(AnalysisView::Waves.label(), "Wave pattern");
    assert_eq!(AnalysisView::from_key("waves"), Some(AnalysisView::Waves));
    assert!(!AnalysisView::Waves.uses_slot());
}

#[test]
fn performance_tab_offers_the_view() {
    let (mut app, ctx) = demo();
    let texts = performance_texts(&ctx, &mut app);
    assert!(texts.iter().any(|t| t == "Open wave pattern"), "{texts:?}");
    assert!(!texts.iter().any(|t| t.contains("Domain")), "{texts:?}");
    assert!(!texts.iter().any(|t| t == "Show on Plan view"), "{texts:?}");
    let out = frame(&ctx, &mut app, Vec::new());
    let at = text_rect(&out, "Open wave pattern")
        .expect("the button")
        .center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&ctx, &mut app, vec![button(at, true)]);
    frame(&ctx, &mut app, vec![button(at, false)]);
    assert_eq!(app.analysis_view(), Some(AnalysisView::Waves));
    frames_until(&ctx, &mut app, "the pattern", |a| {
        a.wave_current().is_some()
    });
    let texts = performance_texts(&ctx, &mut app);
    let sentence = app.wave_current().unwrap().regime.sentence();
    assert!(texts.iter().any(|t| t == sentence), "{texts:?}");
}

#[test]
fn view_marks_a_stale_result() {
    let (mut app, ctx) = demo_with_waves();
    let chips = |t: &[String]| t.iter().filter(|t| *t == "out of date — F5").count();
    frame(&ctx, &mut app, Vec::new());
    let before = chips(&texts(&frame(&ctx, &mut app, Vec::new())));
    app.set_wave_settings(WaveSettings {
        contours: 3,
        ..app.wave_settings().clone()
    });
    let after = chips(&texts(&frame(&ctx, &mut app, Vec::new())));
    assert_eq!(after, before + 1, "the kept result carries the stale chip");
}

#[test]
fn speed_is_an_input() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Knots(8.0),
        ..app.wave_settings().clone()
    });
    frames_until(&ctx, &mut app, "the 8 kn pattern", |a| {
        a.wave_current().is_some()
    });
    let u = app.wave_current().unwrap().field.speed_m_s;
    assert!(
        (u - 8.0 * seacraft_core::resistance::KNOT).abs() < 1e-12,
        "U {u}"
    );
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-waves-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

#[test]
fn saves_png_svg_vti() {
    let (mut app, _ctx) = demo_with_waves();
    let (nx, ny) = {
        let f = &app.wave_current().unwrap().field;
        (f.xs.len() as u32, f.ys.len() as u32)
    };
    let png = scratch("waves.png");
    let svg = scratch("waves.SVG");
    let vti = scratch("waves.vti");
    for p in [&png, &svg, &vti] {
        let _ = std::fs::remove_file(p);
        app.write_wave_image(p)
            .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
        app.wait_wave_image()
            .expect("a write job")
            .unwrap_or_else(|e| panic!("{}: {e}", p.display()));
    }
    let bytes = std::fs::read(&png).unwrap();
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    let be = |i: usize| u32::from_be_bytes(bytes[i..i + 4].try_into().unwrap());
    assert_eq!(&bytes[12..16], b"IHDR");
    assert_eq!((be(16), be(20)), (nx, ny), "one pixel per grid point");
    let svg = std::fs::read_to_string(&svg).unwrap();
    assert!(svg.contains(seacraft_core::wave_pattern::FAR_FIELD_NOTE));
    let vti = std::fs::read_to_string(&vti).unwrap();
    assert!(vti.contains("<ImageData"));
    assert!(app
        .messages()
        .entries()
        .any(|m| m.text.starts_with("Saved wave pattern to ")));
}

#[test]
fn refuses_an_unknown_extension() {
    let (mut app, _ctx) = demo_with_waves();
    let jpg = scratch("waves.jpg");
    let _ = std::fs::remove_file(&jpg);
    let err = app.write_wave_image(&jpg).unwrap_err();
    assert_eq!(err, "save the wave pattern as .png, .svg or .vti");
    assert!(app.wait_wave_image().is_none(), "nothing is written");
    assert!(!jpg.exists());
}

#[test]
fn refuses_a_stale_pattern() {
    let (mut app, _ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        wash_distance_m: Some(4.0),
        ..app.wave_settings().clone()
    });
    let png = scratch("stale.png");
    let _ = std::fs::remove_file(&png);
    let err = app.write_wave_image(&png).unwrap_err();
    assert_eq!(
        err,
        "the wave pattern is out of date or not computed; wait for it to finish"
    );
    assert!(app.wait_wave_image().is_none(), "nothing is written");
    assert!(!png.exists());
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

fn text_rect(out: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
    fn walk(shape: &egui::Shape, label: &str) -> Option<egui::Rect> {
        match shape {
            egui::Shape::Vec(v) => v.iter().find_map(|s| walk(s, label)),
            egui::Shape::Text(t) if t.galley.text() == label => {
                Some(egui::Rect::from_min_size(t.pos, t.galley.size()))
            }
            _ => None,
        }
    }
    out.shapes.iter().find_map(|c| walk(&c.shape, label))
}

#[test]
fn save_button_opens_the_dialog() {
    let (mut app, ctx) = demo_with_waves();
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    let at = text_rect(&out, "Save image…").expect("the button").center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&ctx, &mut app, vec![button(at, true)]);
    frame(&ctx, &mut app, vec![button(at, false)]);
    assert!(app.dialog_open());
    let texts = texts(&frame(&ctx, &mut app, Vec::new()));
    assert!(texts.iter().any(|t| t == "Wave pattern image"), "{texts:?}");
    assert!(
        texts
            .iter()
            .any(|t| t.ends_with("demo-hull-waves-fn0.30.png")),
        "{texts:?}"
    );
}

#[test]
fn shown_pattern_goes_into_the_report() {
    let (mut app, _ctx) = demo();
    let html = scratch("report-hidden.html");
    app.write_report_blocking(&html).unwrap();
    assert!(!std::fs::read_to_string(&html)
        .unwrap()
        .contains("id=\"waves\""));
    app.set_wave_view_open(true);
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Knots(6.0),
        ..app.wave_settings().clone()
    });
    let html = scratch("report-shown.html");
    app.write_report_blocking(&html).unwrap();
    let text = std::fs::read_to_string(&html).unwrap();
    assert!(text.contains("id=\"waves\""), "the section is in");
    assert!(text.contains("<td>6.00</td>"), "at the dock's speed");
}

/// A speed change re-keys only the field: the wave-drag curve stays current.
#[test]
fn drag_curve_ignores_speed() {
    let (mut app, _ctx) = demo();
    app.set_wave_view_open(true);
    app.compute_wave_drag_blocking().unwrap();
    assert!(app.wave_drag().is_some());
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.42),
        ..app.wave_settings().clone()
    });
    assert!(app.wave_drag().is_some(), "the curve is still current");
    app.set_wave_settings(WaveSettings {
        cat_spacing_m: Some(8.0),
        ..app.wave_settings().clone()
    });
    assert!(
        app.wave_drag().is_none(),
        "a catamaran spacing re-keys the curve"
    );
}

#[test]
fn shown_pattern_computes_its_drag_curve() {
    let (mut app, ctx) = demo();
    app.set_wave_view_open(true);
    frames_until(&ctx, &mut app, "the wave drag curve", |a| {
        a.wave_drag().is_some()
    });
    assert_eq!(app.wave_drag().unwrap().mono.len(), 60);
}

/// The view's texts, once its pattern for the current settings is computed.
fn view_texts(ctx: &egui::Context, app: &mut App) -> Vec<String> {
    frames_until(ctx, app, "the current pattern", |a| {
        a.wave_current().is_some() && a.wave_drag().is_some()
    });
    frame(ctx, app, Vec::new());
    texts(&frame(ctx, app, Vec::new()))
}

#[test]
fn view_shows_plain_readouts() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.45),
        ..app.wave_settings().clone()
    });
    let texts = view_texts(&ctx, &mut app);
    for want in [
        "Around hull speed",
        "× waterline",
        "Hull speed",
        "Wash at",
        "Wave drag here",
        "Highest crest",
    ] {
        assert!(texts.iter().any(|t| t.contains(want)), "{want}: {texts:?}");
    }
    for symbol in ["λ_T", "θ_c", "|ζ|"] {
        assert!(
            !texts.iter().any(|t| t.contains(symbol)),
            "{symbol}: {texts:?}"
        );
    }
}

#[test]
fn banner_names_why_the_theory_does_not_fit() {
    let (mut app, ctx) = demo_with_waves();
    let texts = view_texts(&ctx, &mut app);
    let banner = texts
        .iter()
        .find(|t| t.starts_with("Thin-ship theory does not fit this hull (L/B"))
        .unwrap_or_else(|| panic!("{texts:?}"));
    assert!(banner.contains("wide transom"), "{banner}");
    assert!(
        banner.ends_with("read the pattern as a sketch, not a prediction."),
        "{banner}"
    );
    assert!(!texts
        .iter()
        .any(|t| t.contains("heel and trim are ignored")));
}

#[test]
fn heel_is_named_in_the_banner() {
    let (mut app, ctx) = demo_with_waves();
    app.hydro_options_mut().heel_deg = 5.0;
    let texts = view_texts(&ctx, &mut app);
    assert!(
        texts
            .iter()
            .any(|t| t == "Computed upright at the draft; heel and trim are ignored."),
        "{texts:?}"
    );
}

#[test]
fn catamaran_shows_interference() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        cat_spacing_m: Some(8.0),
        ..app.wave_settings().clone()
    });
    let texts = view_texts(&ctx, &mut app);
    assert!(texts.iter().any(|t| t == "Interference"), "{texts:?}");
    assert!(
        texts.iter().any(|t| t.starts_with("favourable (")
            || t.starts_with("unfavourable (")
            || t.starts_with("neutral (")),
        "{texts:?}"
    );
    assert!(texts.iter().any(|t| t.starts_with("8.0 m (")), "{texts:?}");
}

/// Review focus 5: a typed Fn 0.05 is kept (the slider starts at 0.10) and flagged.
#[test]
fn typed_low_speed_is_kept_and_flagged() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.05),
        ..app.wave_settings().clone()
    });
    let texts = view_texts(&ctx, &mut app);
    assert_eq!(app.wave_settings().speed, WaveSpeed::Froude(0.05));
    let banner = texts
        .iter()
        .find(|t| t.starts_with("Thin-ship theory does not fit this hull"))
        .unwrap_or_else(|| panic!("{texts:?}"));
    assert!(banner.contains("Fn 0.05 outside 0.20–1.00"), "{banner}");
}

/// Every shape of a frame, flattened.
fn frame_shapes(out: &egui::FullOutput) -> Vec<egui::Shape> {
    flat(out.shapes.iter().map(|c| c.shape.clone()))
}

/// The y of each horizontal dashed line in `colour`, rounded to a pixel, in ascending order.
fn dashed_rows(shapes: &[egui::Shape], colour: egui::Color32) -> Vec<f32> {
    let mut rows: Vec<f32> = shapes
        .iter()
        .filter_map(|s| match s {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == colour && (points[0].y - points[1].y).abs() < 0.5 =>
            {
                Some(points[0].y.round())
            }
            _ => None,
        })
        .collect();
    rows.sort_by(f32::total_cmp);
    rows.dedup();
    rows
}

/// The view's frame once its pattern is current and its texture loaded.
fn settled_view(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
    frames_until(ctx, app, "the current pattern", |a| {
        a.wave_current().is_some() && a.wave_drag().is_some()
    });
    frame(ctx, app, Vec::new());
    frame(ctx, app, Vec::new())
}

#[test]
fn map_draws_texture_hull_and_wash_lines() {
    let (mut app, ctx) = demo_with_waves();
    let out = settled_view(&ctx, &mut app);
    let shapes = frame_shapes(&out);
    let texture = app.wave_texture_id().expect("the wave texture");
    let quads = shapes
        .iter()
        .filter(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id == texture))
        .count();
    assert_eq!(quads, 1, "one wave image on the map");
    let hull = shapes
        .iter()
        .filter(|s| {
            matches!(s, egui::Shape::Mesh(m)
                if !m.vertices.is_empty() && m.vertices.iter().all(|v| v.color == theme::WAVE_HULL))
        })
        .count();
    assert!(hull >= 1, "the hull footprint is filled");
    let rows = dashed_rows(&shapes, theme::WAVE_CUT);
    assert_eq!(rows.len(), 2, "a wash line each side: {rows:?}");
    let texts = texts(&out);
    for want in ["Bow →", "far field only: not the wave profile on the hull"] {
        assert!(texts.iter().any(|t| t == want), "{want}: {texts:?}");
    }
    assert!(texts.iter().any(|t| t.starts_with("wash ")), "{texts:?}");
    assert!(texts.iter().any(|t| t.contains("· calm ·")), "{texts:?}");
}

#[test]
fn kelvin_wedge_toggles() {
    let (mut app, ctx) = demo_with_waves();
    let kelvin = |out: &egui::FullOutput| {
        frame_shapes(out)
            .iter()
            .filter(|s| matches!(s, egui::Shape::LineSegment { stroke, .. } if stroke.color == theme::WAVE_KELVIN))
            .count()
    };
    let out = settled_view(&ctx, &mut app);
    assert_eq!(kelvin(&out), 2, "two wedge lines from the bow");
    app.set_wave_settings(WaveSettings {
        kelvin_wedge: false,
        ..app.wave_settings().clone()
    });
    let out = frame(&ctx, &mut app, Vec::new());
    assert_eq!(kelvin(&out), 0);
}

/// The 2026-11-19 ruling: the window's map is dark calm water ahead of the bow.
#[test]
fn map_colours_are_dark_at_calm() {
    let (mut app, ctx) = demo_with_waves();
    settled_view(&ctx, &mut app);
    let w = app.wave_current().unwrap();
    let [nx, ny] = w.image.size;
    // The last column is ahead of the bow, where the far field is calm; the middle row is y = 0.
    let px = w.image.pixels[(ny / 2) * nx + nx - 1];
    assert_eq!(px, theme::WAVE_CALM, "{px:?}");
}

#[test]
fn dragging_the_wash_line_sets_the_distance() {
    let (mut app, ctx) = demo_with_waves();
    let out = settled_view(&ctx, &mut app);
    let d = app.wave_current().unwrap().wash_distance_m;
    let shapes = frame_shapes(&out);
    let rows = dashed_rows(&shapes, theme::WAVE_CUT);
    let [top, bottom] = rows[..] else {
        panic!("{rows:?}")
    };
    let per_m = (bottom - top) / (2.0 * d as f32);
    let x = shapes
        .iter()
        .find_map(|s| match s {
            egui::Shape::LineSegment { points, stroke }
                if stroke.color == theme::WAVE_CUT && points[0].y.round() == top =>
            {
                Some(0.5 * (points[0].x + points[1].x))
            }
            _ => None,
        })
        .unwrap();
    let from = egui::pos2(x, top);
    // Three metres closer to the hull.
    let to = egui::pos2(x, top + 3.0 * per_m);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    frame(&ctx, &mut app, vec![button(from, true)]);
    for k in 1..=6 {
        let p = from + (to - from) * (k as f32 / 6.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p)]);
    }
    frame(&ctx, &mut app, vec![button(to, false)]);
    let got = app
        .wave_settings()
        .wash_distance_m
        .expect("a wash distance");
    assert!((got - (d - 3.0)).abs() < 0.5, "{got} vs {}", d - 3.0);
}

#[test]
fn drag_plot_marks_hull_speed_and_humps() {
    let (mut app, ctx) = demo_with_waves();
    let texts = texts(&settled_view(&ctx, &mut app));
    for want in [
        "Wave drag coefficient C_w × 10³ vs Fn",
        "hull speed",
        "hump",
        "Wash at",
    ] {
        assert!(texts.iter().any(|t| t.contains(want)), "{want}: {texts:?}");
    }
    assert!(
        texts
            .iter()
            .any(|t| t.starts_with("Wash at ") && t.contains(": elevation (m) vs x (m)")),
        "{texts:?}"
    );
}

#[test]
fn spacing_mode_only_for_catamarans() {
    let (mut app, ctx) = demo_with_waves();
    let mono = texts(&settled_view(&ctx, &mut app));
    assert!(!mono.iter().any(|t| t == "vs spacing"), "{mono:?}");
    app.set_wave_settings(WaveSettings {
        cat_spacing_m: Some(8.0),
        ..app.wave_settings().clone()
    });
    let out = settled_view(&ctx, &mut app);
    let cat = texts(&out);
    assert!(cat.iter().any(|t| t == "vs spacing"), "{cat:?}");
    assert!(cat.iter().any(|t| t.contains("2 × one hull")), "{cat:?}");
    // The spacing plot: interference against spacing, with the neutral line.
    let at = text_rect(&out, "vs spacing").unwrap().center();
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&ctx, &mut app, vec![button(at, true)]);
    frame(&ctx, &mut app, vec![button(at, false)]);
    let spacing = texts(&frame(&ctx, &mut app, Vec::new()));
    assert!(
        spacing
            .iter()
            .any(|t| t.starts_with("Interference vs spacing")),
        "{spacing:?}"
    );
    assert!(
        spacing.iter().any(|t| t.contains("no interference")),
        "{spacing:?}"
    );
}

#[test]
fn dragging_the_drag_marker_sets_the_speed() {
    let (mut app, ctx) = demo_with_waves();
    let out = settled_view(&ctx, &mut app);
    let caption = text_rect(&out, "Wave drag coefficient C_w × 10³ vs Fn").expect("the plot");
    // Well inside the plot, to the right of Fn 0.30.
    let at = egui::pos2(
        caption.left() + 0.8 * caption.width().max(200.0),
        caption.bottom() + 60.0,
    );
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(at)]);
    frame(&ctx, &mut app, vec![button(at, true)]);
    frame(&ctx, &mut app, vec![button(at, false)]);
    let WaveSpeed::Froude(f) = app.wave_settings().speed else {
        panic!("{:?}", app.wave_settings().speed)
    };
    assert!(f > 0.30 && f <= 1.0, "Fn {f}");
}

/// Plan view never draws the colour image: only contours and lines, with the view closed.
#[test]
fn plan_view_has_no_wave_fill() {
    let (mut app, ctx) = demo_with_plan_waves();
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    let texture = app.wave_texture_id();
    let shapes = frame_shapes(&out);
    assert!(
        !shapes
            .iter()
            .any(|s| matches!(s, egui::Shape::Mesh(m) if Some(m.texture_id) == texture)),
        "no wave image on Plan view"
    );
    assert!(
        shapes.iter().any(is_crest_contour),
        "the contours are drawn"
    );
}

#[test]
fn plan_contours_off_by_default() {
    assert!(!WaveSettings::default().plan_contours);
    let (mut app, ctx) = demo_with_waves();
    let out = settled_view(&ctx, &mut app);
    assert!(
        !texts(&out).iter().any(|t| t.starts_with("Waves · Fn")),
        "no Plan view caption while the contours are off"
    );
}
