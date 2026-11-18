//! The far-field wave pattern in the window, headless: it is computed in a job while shown, an
//! out-of-date pattern is never the current one, and a failure is reported once.
//!
//! The waits are generous guards (a debug build on a loaded machine), not speed checks.

use std::time::{Duration, Instant};

use eframe::egui;
use std::sync::Arc;

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
fn demo_with_waves() -> (App, egui::Context) {
    let (mut app, ctx) = demo();
    app.set_waves_shown(true);
    frames_until(&ctx, &mut app, "the wave pattern", |a| {
        a.wave_overlay().is_some()
    });
    (app, ctx)
}

#[test]
fn shown_pattern_computes_by_itself() {
    let (app, _ctx) = demo_with_waves();
    let w = app.wave_overlay().expect("the pattern");
    assert!((w.field.froude - 0.3).abs() < 1e-12);
    assert!(!app.waves_busy());
}

#[test]
fn a_settings_change_hides_the_old_pattern_at_once() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.4),
        ..app.wave_settings().clone()
    });
    assert!(
        app.wave_overlay().is_none(),
        "the Fn 0.3 pattern is out of date"
    );
    frames_until(&ctx, &mut app, "the Fn 0.4 pattern", |a| {
        a.wave_overlay().is_some()
    });
    let froude = app.wave_overlay().unwrap().field.froude;
    assert!((froude - 0.4).abs() < 1e-12, "Fn {froude}");
}

#[test]
fn a_model_edit_or_new_level_hides_the_old_pattern() {
    let (mut app, ctx) = demo_with_waves();
    app.set_subdivision_level(1);
    assert!(
        app.wave_overlay().is_none(),
        "a new level hides the pattern"
    );
    // Another model opened while the level-1 job may still run: the demo's result never returns.
    frame(&ctx, &mut app, Vec::new());
    std::thread::sleep(Duration::from_millis(350));
    frame(&ctx, &mut app, Vec::new());
    let demo_l = {
        let (a, _) = demo_with_waves();
        a.wave_overlay().unwrap().length_wl
    };
    app.open_model(std::path::Path::new(&path("wigley.fef")))
        .expect("open wigley.fef");
    assert!(
        app.wave_overlay().is_none(),
        "a new model hides the pattern"
    );
    frames_until(&ctx, &mut app, "the Wigley pattern", |a| {
        a.wave_overlay().is_some()
    });
    let l = app.wave_overlay().unwrap().length_wl;
    assert!(
        (l - demo_l).abs() > 1e-6,
        "the Wigley's L {l}, not the demo's"
    );
}

#[test]
fn a_failing_pattern_is_reported_once() {
    let (mut app, ctx) = demo();
    app.set_waves_shown(true);
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
    assert!(app.wave_overlay().is_none());
}

#[test]
fn hidden_pattern_computes_nothing() {
    let (mut app, ctx) = demo();
    for _ in 0..20 {
        frame(&ctx, &mut app, Vec::new());
        std::thread::sleep(Duration::from_millis(20));
        assert!(!app.waves_busy());
    }
    assert!(app.wave_overlay().is_none());
    assert!(!app.wave_settings().show);
}

#[test]
fn blocking_compute_makes_the_pattern_current() {
    let (mut app, _ctx) = demo();
    app.set_waves_shown(true);
    app.compute_waves_blocking().expect("the pattern");
    assert!(app.wave_overlay().is_some());
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

fn is_wave_mesh(shape: &egui::Shape, texture: egui::TextureId) -> bool {
    matches!(shape, egui::Shape::Mesh(m) if m.texture_id == texture)
}

const WAVE_TEXTURE: egui::TextureId = egui::TextureId::User(4242);

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
    let w = compute_waves(&surface, subdiv, draft, settings).unwrap();
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
            texture: WAVE_TEXTURE,
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

/// The wave texture's meshes and the contour strokes, as text.
fn wave_marks(shapes: &[egui::Shape]) -> Vec<String> {
    shapes
        .iter()
        .filter(|s| {
            is_wave_mesh(s, WAVE_TEXTURE)
                || stroke_colour(s).is_some_and(|c| {
                    c == theme::WAVE_CONTOUR_CREST || c == theme::WAVE_CONTOUR_TROUGH
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
    assert!(
        painted.iter().any(|s| s.starts_with("Mesh")),
        "the wave quad is drawn"
    );
    assert!(painted.len() > 10, "contours are drawn: {}", painted.len());
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
        .position(|s| is_wave_mesh(s, WAVE_TEXTURE))
        .expect("the wave quad");
    let hull_at = shapes
        .iter()
        .position(|s| matches!(s, egui::Shape::Mesh(m) if m.texture_id != WAVE_TEXTURE))
        .expect("the hull");
    assert!(hull_at < wave_at, "the pattern is drawn over the hull");
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
    let (mut app, ctx) = demo_with_waves();
    let textured = |out: &egui::FullOutput| {
        flat(out.shapes.iter().map(|c| c.shape.clone())).iter().any(
            |s| matches!(s, egui::Shape::Mesh(m) if m.texture_id != egui::TextureId::default()),
        )
    };
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(textured(&out), "the current pattern is drawn");
    assert!(texts(&out)
        .iter()
        .any(|t| t.starts_with("Waves · Fn 0.300")));
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Froude(0.45),
        ..app.wave_settings().clone()
    });
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(
        !textured(&out),
        "the Fn 0.3 pattern is not drawn as current"
    );
    assert!(
        texts(&out).iter().any(|t| t == "Wave pattern: computing…"),
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
fn waves_section_shows_its_results() {
    let (mut app, ctx) = demo_with_waves();
    let texts = performance_texts(&ctx, &mut app);
    assert!(texts.iter().any(|t| t == "Wave pattern"), "{texts:?}");
    let note = seacraft_core::wave_pattern::FAR_FIELD_NOTE;
    assert!(
        texts.iter().any(|t| t == note),
        "the far-field note, verbatim"
    );
    assert!(
        texts.iter().any(|t| t == "Wave cut: ζ (m) vs x (m)"),
        "{texts:?}"
    );
    assert!(!texts
        .iter()
        .any(|t| t.contains("heel and trim are ignored")));
}

#[test]
fn heel_is_named_as_ignored() {
    let (mut app, ctx) = demo_with_waves();
    app.hydro_options_mut().heel_deg = 5.0;
    let texts = performance_texts(&ctx, &mut app);
    assert!(
        texts
            .iter()
            .any(|t| t.contains("computed upright at the draft; heel and trim are ignored")),
        "{texts:?}"
    );
}

#[test]
fn waves_section_marks_a_stale_result() {
    let (mut app, ctx) = demo_with_waves();
    let chips = |t: &[String]| t.iter().filter(|t| *t == "out of date — F5").count();
    let before = chips(&performance_texts(&ctx, &mut app));
    app.set_wave_settings(WaveSettings {
        contours: 3,
        ..app.wave_settings().clone()
    });
    let after = chips(&texts(&frame(&ctx, &mut app, Vec::new())));
    assert_eq!(after, before + 1, "the wave results carry the stale chip");
}

#[test]
fn speed_is_an_input() {
    let (mut app, ctx) = demo_with_waves();
    app.set_wave_settings(WaveSettings {
        speed: WaveSpeed::Knots(8.0),
        ..app.wave_settings().clone()
    });
    frames_until(&ctx, &mut app, "the 8 kn pattern", |a| {
        a.wave_overlay().is_some()
    });
    let u = app.wave_overlay().unwrap().field.speed_m_s;
    assert!(
        (u - 8.0 * seacraft_core::resistance::KNOT).abs() < 1e-12,
        "U {u}"
    );
    let texts = performance_texts(&ctx, &mut app);
    // The section's summary line, not the Plan view caption.
    assert!(
        texts.iter().any(|t| t.starts_with("U 8.00 kn · Fn ")),
        "{texts:?}"
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
        let f = &app.wave_overlay().unwrap().field;
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
        cut_y_l: 0.5,
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
    app.set_dock_tab(seacraft_gui::app::DesignDock::Performance);
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
