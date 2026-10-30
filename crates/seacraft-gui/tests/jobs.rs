//! Background jobs and the recompute rule (R5-2, U11), headless: failures
//! reach the user, superseded results are dropped, a drag makes results
//! stale, the cheap ones come back by themselves and F5 brings back the
//! rest, and no PASS is ever drawn on stale data.
//!
//! The waits are generous guards (a debug build on a loaded machine), not
//! speed checks.

use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_gui::app::DesignDock;
use seacraft_gui::App;

const PATIENCE: Duration = Duration::from_secs(120);

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            // Tall, so the Loading tab's criteria badges are on screen.
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

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

fn f5() -> egui::Event {
    egui::Event::Key {
        key: egui::Key::F5,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

fn path(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

/// The demo hull with the demo loading condition, at level 2 for speed.
fn demo_with_loadcase() -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&path("demo-hull.fef")));
    app.set_subdivision_level(2);
    let text = std::fs::read_to_string(path("demo-loadcase.json")).expect("demo-loadcase.json");
    let cond: seacraft_core::loading::LoadingCondition =
        serde_json::from_str(&text).expect("parse demo-loadcase.json");
    app.set_project(seacraft_core::project::Project {
        loading_conditions: vec![cond],
        ..Default::default()
    });
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

#[test]
fn a_panicking_job_reaches_the_status_line_and_the_tab() {
    let (mut app, ctx) = demo_with_loadcase();
    app.set_show_loading(true);
    app.start_loading_job_for_test(|| panic!("tank table corrupt at row 7"));
    let out = frames_until(&ctx, &mut app, "the failing job", |a| !a.loading_busy());
    assert!(
        app.status().contains("tank table corrupt at row 7"),
        "status: {}",
        app.status()
    );
    assert!(
        app.status().contains("Loading compute failed"),
        "status: {}",
        app.status()
    );
    let failure = app
        .tab_failure(DesignDock::Loading)
        .expect("the tab's failure");
    assert!(failure.contains("tank table corrupt at row 7"), "{failure}");
    // The failure is drawn on the tab (the frame after it landed).
    let out = if texts(&out).iter().any(|t| t.contains("row 7")) {
        out
    } else {
        frame(&ctx, &mut app, Vec::new())
    };
    assert!(
        texts(&out)
            .iter()
            .any(|t| t.contains("tank table corrupt at row 7")),
        "the Loading tab shows the failure"
    );
    // The same inputs are not retried automatically.
    std::thread::sleep(Duration::from_millis(400));
    frame(&ctx, &mut app, Vec::new());
    assert!(!app.loading_busy(), "a failed revision is not retried");
    assert!(app.loading_result().is_none());
}

#[test]
fn a_superseded_job_is_discarded() {
    let (mut app, ctx) = demo_with_loadcase();
    let (release, gate) = std::sync::mpsc::channel::<()>();
    // The first job answers only after the second has started.
    app.start_loading_job_for_test(move || {
        let _ = gate.recv();
        Err("the superseded job answered".into())
    });
    app.start_loading_compute();
    let _ = release.send(());
    frames_until(&ctx, &mut app, "the loading job", |a| !a.loading_busy());
    assert!(app.loading_result().is_some(), "the newer job's result");
    assert!(
        !app.status().contains("superseded job answered"),
        "status: {}",
        app.status()
    );
    assert!(app.tab_failure(DesignDock::Loading).is_none());
    assert!(!app.stale_results().contains(&"loading"));
}

#[test]
fn a_drag_makes_results_stale_auto_recomputes_cheap_ones_and_f5_the_rest() {
    let (mut app, ctx) = demo_with_loadcase();
    // Everything current: hydrostatics, GZ curves, loading, resistance.
    app.recompute_hydrostatics();
    app.compute_cross_curves();
    app.compute_loading_blocking(None).expect("loading");
    app.compute_resistance_blocking().expect("resistance");
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.stale_results(), Vec::<&str>::new());
    assert_eq!(app.stale_count(), 0);

    // The demo condition passes its criteria: a green PASS while current.
    app.set_show_loading(true);
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(
        texts(&out).iter().any(|t| t == "PASS"),
        "a current loading result shows PASS"
    );

    // Drag a sheer point in the profile view.
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
        .expect("point visible");
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![button(screen, true)]);
    let to = screen + egui::vec2(10.0, 0.0);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(to)]);
    let to = screen + egui::vec2(40.0, 0.0);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(to)]);
    frame(&ctx, &mut app, vec![button(to, false)]);
    let out = frame(&ctx, &mut app, Vec::new());
    let stale = app.stale_results();
    for name in ["GZ curves", "loading", "resistance"] {
        assert!(
            stale.contains(&name),
            "{name} stale after the drag: {stale:?}"
        );
    }
    assert!(app.stale_count() > 0);
    // No green verdict on stale data: the badge turns grey.
    let shown = texts(&out);
    assert!(
        !shown.iter().any(|t| t == "PASS"),
        "no PASS on a stale loading result"
    );
    assert!(shown.iter().any(|t| t == "PASS · stale"), "{shown:?}");
    assert!(shown.iter().any(|t| t == "out of date — F5"), "the chip");
    assert!(
        shown.iter().any(|t| t.ends_with("out of date · F5")),
        "the status bar counts stale results"
    );

    // The cheap results come back by themselves after the debounce.
    frames_until(&ctx, &mut app, "the automatic recompute", |a| {
        let stale = a.stale_results();
        !stale.contains(&"hydrostatics")
            && !stale.contains(&"loading")
            && !stale.contains(&"resistance")
    });
    assert_eq!(
        app.stale_results(),
        vec!["GZ curves"],
        "only the manual one"
    );
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(
        texts(&out).iter().any(|t| t == "1 result out of date · F5"),
        "the status bar: {:?}",
        texts(&out)
            .into_iter()
            .filter(|t| t.contains("out of date"))
            .collect::<Vec<_>>()
    );

    // F5 recomputes what is shown and stale: the GZ curves on the
    // Hydrostatics tab.
    app.set_show_loading(false);
    assert_eq!(app.design_dock(), Some(DesignDock::Hydrostatics));
    frame(&ctx, &mut app, vec![f5()]);
    frames_until(&ctx, &mut app, "F5", |a| {
        a.stale_count() == 0 && !a.jobs_busy()
    });
    assert!(app.cross_curves().is_some());
}

#[test]
fn a_hydrostatics_refresh_does_not_hold_the_frame() {
    let mut app = App::headless_for_test(Some(&path("demo-hull.fef")));
    let ctx = egui::Context::default();
    app.set_subdivision_level(4);
    app.recompute_hydrostatics();
    let before = app.hydrostatics().expect("a measurement").clone();
    frame(&ctx, &mut app, Vec::new());

    // A change, then the debounce: the next frame starts the job and
    // returns while it runs.
    app.snapshot_undo();
    app.surface_mut().unwrap().points[3].p.z += 0.05;
    app.mark_geometry_changed();
    std::thread::sleep(Duration::from_millis(150));
    frame(&ctx, &mut app, Vec::new());
    assert!(app.hydro_busy(), "the frame returned while the job ran");
    // The previous report stays shown, out of date, meanwhile.
    assert_eq!(
        app.hydrostatics().map(|h| h.volume_m3.to_bits()),
        Some(before.volume_m3.to_bits())
    );
    assert!(app.stale_results().contains(&"hydrostatics"));

    frames_until(&ctx, &mut app, "the hydrostatics job", |a| !a.hydro_busy());
    assert!(!app.stale_results().contains(&"hydrostatics"));
    // The job measures exactly what the synchronous path does.
    let from_job = app.hydrostatics().unwrap().clone();
    app.recompute_hydrostatics();
    assert_eq!(
        app.hydrostatics().unwrap().volume_m3.to_bits(),
        from_job.volume_m3.to_bits()
    );
    assert_eq!(
        app.hydrostatics().unwrap().kb.to_bits(),
        from_job.kb.to_bits()
    );
}

#[test]
fn the_gz_curves_button_runs_a_job() {
    let (mut app, ctx) = demo_with_loadcase();
    app.recompute_hydrostatics();
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    // Find the button by its text and click it.
    let pos = out
        .shapes
        .iter()
        .find_map(|c| match &c.shape {
            egui::Shape::Text(t) if t.galley.text() == "GZ curves" => {
                Some(t.pos + t.galley.rect.center().to_vec2())
            }
            _ => None,
        })
        .expect("the GZ curves button");
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(pos)]);
    frame(&ctx, &mut app, vec![button(pos, true)]);
    frame(&ctx, &mut app, vec![button(pos, false)]);
    assert!(
        app.curves_busy() || app.cross_curves().is_some(),
        "the click started the curves"
    );
    frames_until(&ctx, &mut app, "the GZ curves job", |a| {
        !a.curves_busy() && a.cross_curves().is_some()
    });
    // The job's curves are the synchronous ones, bit for bit.
    let from_job = app.cross_curves().unwrap().to_vec();
    app.compute_cross_curves();
    let direct = app.cross_curves().unwrap();
    for (a, b) in from_job.iter().zip(direct) {
        for (p, q) in a.points.iter().zip(&b.points) {
            assert_eq!(p.gz.map(f64::to_bits), q.gz.map(f64::to_bits));
            assert_eq!(p.kn.map(f64::to_bits), q.kn.map(f64::to_bits));
        }
    }
}
