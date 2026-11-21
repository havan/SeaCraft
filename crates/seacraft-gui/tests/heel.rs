//! The Heel view, headless: it computes while open, goes stale (never shown as current) when the
//! model moves on, keeps its trial weights, and falls back to the bare hull when its loading
//! condition goes away.

use eframe::egui;
use seacraft_core::loading::{LoadingCondition, WeightItem};
use seacraft_core::math::Vec3;
use seacraft_gui::analysis_views::AnalysisView;
use seacraft_gui::heel_view::HeelStart;
use seacraft_gui::App;

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

/// The demo at a cheap level with the dock KG at 2.2 m, settled.
fn demo() -> (egui::Context, App) {
    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(1);
    app.set_kg(2.2);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (ctx, app)
}

fn weight(mass_t: f64, x: f64, y: f64, z: f64) -> WeightItem {
    WeightItem {
        name: "Weight 1".into(),
        mass_t,
        x,
        y,
        z,
    }
}

#[test]
fn opening_the_view_computes_the_equilibrium() {
    let (_ctx, mut app) = demo();
    assert!(
        app.heel_current().is_none(),
        "nothing before the view opens"
    );
    app.set_heel_view_open(true);
    assert_eq!(app.analysis_view(), Some(AnalysisView::Heel));
    app.compute_heel_blocking().unwrap();
    let out = app.heel_current().expect("computed for the current model");
    assert!(!out.sweep.is_empty());
    assert!(
        out.equilibrium.heel_for_side(0.0).is_some(),
        "{:?}",
        out.equilibrium
    );
    assert!(!app.heel_stale());
}

#[test]
fn the_view_computes_on_its_own_while_open() {
    let (ctx, mut app) = demo();
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.weights.push(weight(0.5, 4.0, 0.8, 1.5));
    app.set_heel_state(state);
    for _ in 0..600 {
        frame(&ctx, &mut app, Vec::new());
        if app.heel_current().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let out = app.heel_current().expect("the job finished");
    let heel = out.equilibrium.heel_for_side(0.0).unwrap();
    assert!(heel > 0.0, "a starboard weight heels to starboard: {heel}");
}

#[test]
fn editing_the_model_makes_the_view_stale() {
    let (_ctx, mut app) = demo();
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.weights.push(weight(0.5, 4.0, 0.8, 1.5));
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    assert!(!app.heel_stale());
    let p = app.surface().unwrap().points[10].p;
    app.move_point(10, p + Vec3::new(0.0, 0.0, 0.05)).unwrap();
    assert!(app.heel_stale(), "the kept result is out of date");
    assert!(app.heel_current().is_none(), "never shown as current");
    assert_eq!(
        app.heel_state().weights.len(),
        1,
        "the trial weights survive"
    );
}

#[test]
fn a_removed_condition_falls_back_to_dock() {
    let (ctx, mut app) = demo();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    let cond: LoadingCondition = serde_json::from_str(&text).unwrap();
    app.project_mut_for_test().loading_conditions.push(cond);
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(0);
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    assert!(app.heel_current().is_some());

    app.project_mut_for_test().loading_conditions.clear();
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.heel_state().start, HeelStart::Dock);
    assert!(
        app.status().contains("that loading condition is gone"),
        "{}",
        app.status()
    );
}

#[test]
fn no_kg_means_no_job() {
    // The demo hull from a file with no project sidecar: no KG anywhere.
    let dir = std::env::temp_dir().join(format!("seacraft-heel-nokg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hull.fef");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo-hull.fef"),
        &path,
    )
    .unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.set_subdivision_level(1);
    assert_eq!(app.kg(), None, "no KG to start from");
    let ctx = egui::Context::default();
    app.set_heel_view_open(true);
    for _ in 0..50 {
        frame(&ctx, &mut app, Vec::new());
        assert!(!app.heel_sweep_busy(), "no condition job without a KG");
        if app.heel_failure().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let err = app
        .heel_failure()
        .expect("the scheduler processed the missing-KG refusal after the 120 ms quiet");
    assert!(err.contains("KG"), "{err}");
    assert!(app.heel_current().is_none());
    assert!(app.compute_heel_blocking().is_err());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn ui_set_opens_the_view() {
    let (_ctx, mut app) = demo();
    app.set_heel_view_open(true);
    assert_eq!(app.analysis_view(), Some(AnalysisView::Heel));
    app.set_heel_view_open(false);
    assert_eq!(app.analysis_view(), None);
}

/// Every text drawn.
fn texts(out: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &egui::Shape, out: &mut Vec<String>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push(t.galley.text().to_string()),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut found);
    }
    found
}

fn drawn(out: &egui::FullOutput, part: &str) -> bool {
    texts(out).iter().any(|t| t.contains(part))
}

/// Open the view with `weights`, compute it, and draw two frames.
fn shown_with(weights: Vec<WeightItem>) -> (egui::Context, App, egui::FullOutput) {
    let (ctx, mut app) = demo();
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.weights = weights;
    app.set_heel_state(state);
    let _ = app.compute_heel_blocking();
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    (ctx, app, out)
}

#[test]
fn the_view_draws_the_readout_and_captions() {
    let (_ctx, _app, out) = shown_with(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let all = texts(&out);
    for part in [
        "Heel and righting arm",
        "Settles at",
        "to starboard",
        "Buoyancy",
        "Weight",
        "kN",
        "Righting lever",
        "Heeling arm",
        "moment",
        "kN·m",
        "projected onto the section",
        "From starboard",
        "not a roll simulation",
        "0.5 t",
    ] {
        assert!(drawn(&out, part), "`{part}` not drawn: {all:?}");
    }
    assert!(
        all.iter()
            .any(|t| t.starts_with("Weight ") && t.contains(" kN (") && t.contains(" t)")),
        "Weight pill must show both kN and mass: {all:?}"
    );
    assert!(
        all.iter()
            .any(|t| t.starts_with("Buoyancy ") && t.contains(" kN (") && t.contains(" t)")),
        "Buoyancy pill must show both kN and mass: {all:?}"
    );
}

#[test]
fn the_view_says_when_the_hull_cannot_float() {
    let (_ctx, _app, out) = shown_with(vec![weight(1000.0, 4.0, 0.0, 1.0)]);
    assert!(drawn(&out, "cannot float"), "{:?}", texts(&out));
    assert!(
        !drawn(&out, "Buoyancy"),
        "no picture of a hull that does not float"
    );
}

#[test]
fn no_kg_shows_the_hint() {
    let dir = std::env::temp_dir().join(format!("seacraft-heel-hint-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join("hull.fef");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo-hull.fef"),
        &path,
    )
    .unwrap();
    let mut app = App::headless_for_test(Some(path.to_str().unwrap()));
    app.set_subdivision_level(1);
    let ctx = egui::Context::default();
    app.set_heel_view_open(true);
    let mut out = frame(&ctx, &mut app, Vec::new());
    for _ in 0..300 {
        out = frame(&ctx, &mut app, Vec::new());
        if drawn(&out, "Upright") {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(
        drawn(&out, "Set KG in the Hydrostatics dock"),
        "{:?}",
        texts(&out)
    );
    assert!(
        drawn(&out, "Upright"),
        "the upright hull is drawn: {:?}",
        texts(&out)
    );
    assert!(!drawn(&out, "Buoyancy"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_stale_result_is_drawn_dimmed_and_says_so() {
    let (ctx, mut app, _) = shown_with(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let p = app.surface().unwrap().points[10].p;
    app.move_point(10, p + Vec3::new(0.0, 0.0, 0.05)).unwrap();
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(app.heel_stale());
    assert!(
        drawn(&out, "out of date — recomputing"),
        "{:?}",
        texts(&out)
    );
}

// ---- interaction -------------------------------------------------------

/// A frame at time `t` seconds.
fn frame_t(
    ctx: &egui::Context,
    app: &mut App,
    events: Vec<egui::Event>,
    t: f64,
) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        time: Some(t),
        events,
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
    out
}

fn press(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    }
}

fn key(key: egui::Key) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A clock for one test: every call is a frame 1/60 s later.
struct Clock(f64);

impl Clock {
    fn frame(&mut self, ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) {
        self.frame_out(ctx, app, events);
    }

    /// A frame, and what it drew.
    fn frame_out(
        &mut self,
        ctx: &egui::Context,
        app: &mut App,
        events: Vec<egui::Event>,
    ) -> egui::FullOutput {
        self.0 += 1.0 / 60.0;
        frame_t(ctx, app, events, self.0)
    }

    /// Click (press and release, no move) at `p`.
    fn click(&mut self, ctx: &egui::Context, app: &mut App, p: egui::Pos2) {
        self.frame(ctx, app, vec![egui::Event::PointerMoved(p)]);
        self.frame(ctx, app, vec![press(p, true)]);
        self.frame(ctx, app, vec![press(p, false)]);
        self.frame(ctx, app, Vec::new());
    }

    /// Drag from `from` by `dx` points to the right, in steps, and release.
    fn drag(&mut self, ctx: &egui::Context, app: &mut App, from: egui::Pos2, dx: f32) {
        self.frame(ctx, app, vec![egui::Event::PointerMoved(from)]);
        self.frame(ctx, app, vec![press(from, true)]);
        for k in 1..=12 {
            let p = from + egui::vec2(dx * k as f32 / 12.0, 0.0);
            self.frame(ctx, app, vec![egui::Event::PointerMoved(p)]);
        }
        self.frame(ctx, app, vec![press(from + egui::vec2(dx, 0.0), false)]);
    }

    /// Let `seconds` pass, a frame at a time.
    fn wait(&mut self, ctx: &egui::Context, app: &mut App, seconds: f64) {
        let end = self.0 + seconds;
        while self.0 < end {
            self.frame(ctx, app, Vec::new());
        }
    }
}

/// The view open on the demo with `weights`, computed, drawn on a clock.
fn interactive(weights: Vec<WeightItem>) -> (egui::Context, App, Clock) {
    let (ctx, mut app, _) = shown_with(weights);
    let mut clock = Clock(10.0);
    clock.frame(&ctx, &mut app, Vec::new());
    clock.frame(&ctx, &mut app, Vec::new());
    (ctx, app, clock)
}

/// A point in the body plan's water, off the hull and its labels: lower left.
fn body_point(app: &App) -> egui::Pos2 {
    let d = app.heel_drawn().expect("the drawings were drawn");
    d.body.left_bottom() + egui::vec2(12.0, -12.0)
}

#[test]
fn click_and_type_places_a_weight() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let d = app.heel_drawn().unwrap();
    let p = body_point(&app);
    clock.click(&ctx, &mut app, p);
    assert!(app.heel_state().entry.is_some(), "the mass field opened");
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("2".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    let w = &app.heel_state().weights;
    assert_eq!(w.len(), 1, "{:?}", app.heel_state());
    assert_eq!(w[0].mass_t, 2.0);
    assert!((w[0].x - d.section_x).abs() < 1e-9, "on the section drawn");
    // Back on the screen where it was clicked.
    let (_, c, h) = d.frame.to_earth(Vec3::new(w[0].x, w[0].y, w[0].z));
    let back = d.body_view.to_screen([c, h]);
    assert!((back - p).length() < 0.5, "{back:?} vs {p:?}");
    assert!(app.heel_state().entry.is_none());
}

#[test]
fn escape_cancels_the_weight() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let p = body_point(&app);
    clock.click(&ctx, &mut app, p);
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("2".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Escape)]);
    assert!(app.heel_state().weights.is_empty());
    assert!(app.heel_state().entry.is_none());
}

#[test]
fn a_bad_mass_is_refused_with_a_reason() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let p = body_point(&app);
    clock.click(&ctx, &mut app, p);
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("-3".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    assert!(app.heel_state().weights.is_empty());
    let entry = app.heel_state().entry.clone().expect("still open");
    assert_eq!(entry.error.as_deref(), Some("type a mass in tonnes"));
}

#[test]
fn a_weight_above_the_deck_is_allowed() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let d = app.heel_drawn().unwrap();
    let top = app
        .surface()
        .unwrap()
        .points
        .iter()
        .map(|p| p.p.z)
        .fold(f64::MIN, f64::max);
    let p = d.body.center_top() + egui::vec2(-40.0, 8.0);
    clock.click(&ctx, &mut app, p);
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("1".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    let w = &app.heel_state().weights;
    assert_eq!(w.len(), 1);
    assert!(w[0].z > top, "{} above the deck at {top}", w[0].z);
}

#[test]
fn release_settles_at_equilibrium() {
    let (ctx, mut app, mut clock) = interactive(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let eq = app
        .heel_current()
        .unwrap()
        .equilibrium
        .heel_for_side(1.0)
        .unwrap();
    let d = app.heel_drawn().unwrap();
    // 40 pt is 10°: inside the range where the righting lever brings the demo back.
    clock.drag(&ctx, &mut app, d.body.center(), 40.0);
    assert!(
        app.heel_state().heel_deg > 8.0,
        "dragging right heels to starboard: {}",
        app.heel_state().heel_deg
    );
    clock.wait(&ctx, &mut app, 0.7);
    let heel = app.heel_state().heel_deg;
    assert!((heel - eq).abs() < 0.01, "{heel} vs {eq}");
    assert!(app.heel_state().anim.is_none());
}

#[test]
fn let_go_past_the_vanishing_angle_it_does_not_come_back() {
    // The demo at KG 2.2 m rights itself only to about 20°: let go at 40° the lever turns it
    // further over, so the picture must not swing it back upright.
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let d = app.heel_drawn().unwrap();
    clock.drag(&ctx, &mut app, d.body.center(), 160.0);
    let released = app.heel_state().heel_deg;
    assert!(released > 35.0, "dragged to {released}");
    clock.wait(&ctx, &mut app, 0.7);
    assert_eq!(
        app.heel_state().heel_deg,
        released,
        "it stays where it was let go"
    );
    assert!(app.heel_state().anim.is_none());
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    assert!(drawn(&out, "would not come back"), "{:?}", texts(&out));
}

#[test]
fn hold_keeps_the_heel() {
    let (ctx, mut app, mut clock) = interactive(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let mut state = app.heel_state().clone();
    state.hold = true;
    app.set_heel_state(state);
    let d = app.heel_drawn().unwrap();
    clock.drag(&ctx, &mut app, d.body.center(), 120.0);
    let held = app.heel_state().heel_deg;
    assert!(held > 20.0, "{held}");
    clock.wait(&ctx, &mut app, 0.7);
    assert_eq!(app.heel_state().heel_deg, held);
}

#[test]
fn arrow_keys_step_the_heel() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let mut state = app.heel_state().clone();
    state.hold = true;
    app.set_heel_state(state);
    let p = body_point(&app);
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p)]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::ArrowRight)]);
    assert_eq!(app.heel_state().heel_deg, 1.0);
    clock.frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::ArrowLeft,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::SHIFT,
        }],
    );
    assert_eq!(app.heel_state().heel_deg, -4.0);
}

#[test]
fn delete_removes_the_selected_weight() {
    let (ctx, mut app, mut clock) = interactive(vec![
        weight(0.5, 4.0, 0.8, 1.5),
        weight(0.3, 5.0, -0.5, 1.2),
    ]);
    let mut state = app.heel_state().clone();
    state.selected = Some(0);
    app.set_heel_state(state);
    let p = body_point(&app);
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p)]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Delete)]);
    let w = &app.heel_state().weights;
    assert_eq!(w.len(), 1);
    assert_eq!(w[0].mass_t, 0.3);
    assert_eq!(app.heel_state().selected, None);
}

#[test]
fn add_to_condition_appends_items_and_undo_removes_them() {
    let (ctx, mut app) = demo();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    let cond: LoadingCondition = serde_json::from_str(&text).unwrap();
    let items_before = cond.items.len();
    app.project_mut_for_test().loading_conditions.push(cond);
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(0);
    state.weights = vec![weight(0.5, 4.0, 0.8, 1.5)];
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    let mut clock = Clock(10.0);
    clock.frame(&ctx, &mut app, Vec::new());
    let out = frame_t(&ctx, &mut app, Vec::new(), 10.1);
    let rect = texts_at(&out, "Add to this condition").expect("the button is drawn");
    clock.0 = 10.1;
    clock.click(&ctx, &mut app, rect.center());
    let items = &app.project().loading_conditions[0].items;
    assert_eq!(items.len(), items_before + 1);
    assert_eq!(items.last().unwrap().mass_t, 0.5);
    assert!(
        app.heel_state().weights.is_empty(),
        "the weights moved into the condition"
    );
    assert_eq!(app.heel_state().start, HeelStart::Condition(0));
    assert!(app.is_dirty());
    app.undo();
    assert_eq!(
        app.project().loading_conditions[0].items.len(),
        items_before
    );
}

/// Where the text `label` was drawn.
fn texts_at(out: &egui::FullOutput, label: &str) -> Option<egui::Rect> {
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
fn dragging_a_weight_moves_it_in_that_drawings_plane() {
    let (ctx, mut app, mut clock) = interactive(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let mut state = app.heel_state().clone();
    state.hold = true;
    app.set_heel_state(state);
    clock.frame(&ctx, &mut app, Vec::new());
    let d = app.heel_drawn().unwrap();
    let (a, _, h) = d.frame.to_earth(Vec3::new(4.0, 0.8, 1.5));
    let on = d.profile_view.to_screen([a, h]);
    let heel = app.heel_state().heel_deg;
    clock.drag(&ctx, &mut app, on, 40.0);
    let w = &app.heel_state().weights[0];
    assert_eq!(w.y, 0.8, "the profile keeps the weight's y");
    assert!(w.x > 4.5, "moved forward: x {}", w.x);
    assert_eq!(
        app.heel_state().heel_deg,
        heel,
        "a weight drag is not a heel drag"
    );
    let after = app.heel_drawn().unwrap();
    assert_eq!(
        (after.body, after.profile),
        (d.body, d.profile),
        "the drawings stay put when the result goes out of date"
    );
}

/// Each drawn text containing `part`, with the rect it may be seen in.
fn texts_clipped(out: &egui::FullOutput, part: &str) -> Vec<(egui::Rect, egui::Rect)> {
    fn walk(
        shape: &egui::Shape,
        part: &str,
        clip: egui::Rect,
        found: &mut Vec<(egui::Rect, egui::Rect)>,
    ) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, part, clip, found)),
            egui::Shape::Text(t) if t.galley.text().contains(part) => {
                found.push((egui::Rect::from_min_size(t.pos, t.galley.size()), clip));
            }
            _ => {}
        }
    }
    let mut found = Vec::new();
    for c in &out.shapes {
        walk(&c.shape, part, c.clip_rect, &mut found);
    }
    found
}

#[test]
fn the_readout_beside_the_drawings_is_not_cut_off() {
    // A long body-plan caption once widened the drawings' column and pushed the readout, the
    // equilibrium line and the weights table past the drawer's right edge, where they were cut.
    let (_, _, out) = shown_with(Vec::new());
    for part in [
        "Righting lever",
        "Buoyancy / weight",
        "Settles",
        "Trial weights",
    ] {
        let found = texts_clipped(&out, part);
        assert!(!found.is_empty(), "{part:?} was drawn");
        for (rect, clip) in found {
            assert!(
                rect.max.x <= clip.max.x + 0.5,
                "{part:?} ends at x {} past its clip at {}",
                rect.max.x,
                clip.max.x
            );
        }
    }
}

#[test]
fn upright_reads_zero_not_minus_zero() {
    // Upright the lever and the trim are rounding errors either side of zero (on the demo at
    // level 3 the lever came out a hair below): they read 0.000, not -0.000.
    let (_, app, _) = shown_with(Vec::new());
    let out = app.heel_current().expect("the sweep is current");
    let mut s = out.upright().expect("an upright state").clone();
    let tiny = -1e-9;
    (s.lever_m, s.gz_m, s.heeling_arm_m, s.trim_m) = (tiny, tiny, tiny, Some(tiny));
    let readout = seacraft_gui::heel_view::readout(&s, out, false);
    let line = seacraft_gui::heel_view::equilibrium_line(
        &seacraft_core::heel::Equilibrium::Stable { state: s.clone() },
        out.mass.displacement_t,
    );
    for text in [readout, line] {
        assert!(
            !text.contains("-0.000") && !text.contains("−0.000"),
            "{text}"
        );
    }
}

#[test]
fn the_weights_table_fits_beside_the_drawings() {
    // Each weight's row (name, mass, x, y, z and its remove button) is seen whole at 1600 points.
    let (_, _, out) = shown_with(vec![weight(0.5, 4.3, 2.79, 2.3)]);
    for part in ["Trial weights", "2.30", "✕"] {
        let found = texts_clipped(&out, part);
        assert!(!found.is_empty(), "{part:?} was drawn: {:?}", texts(&out));
        for (rect, clip) in found {
            assert!(
                rect.max.x <= clip.max.x + 0.5,
                "{part:?} ends at x {} past its clip at {}",
                rect.max.x,
                clip.max.x
            );
        }
    }
}

#[test]
fn a_capsizing_condition_is_said_once_and_upright_is_not_a_side() {
    // 0.5 t high and far out to starboard: more heeling arm than the demo has GZ.
    let (_, _, out) = shown_with(vec![weight(0.5, 4.3, 2.79, 2.3)]);
    assert!(drawn(&out, "Capsizes"), "{:?}", texts(&out));
    assert!(!drawn(&out, "would not come back"), "{:?}", texts(&out));
    assert!(!drawn(&out, "0.0° to port"), "{:?}", texts(&out));
}

#[test]
fn a_condition_changed_under_the_view_settles_again() {
    // Undo after "Add to this condition" takes the weight back out of the condition: the hull
    // settles at the condition's own equilibrium rather than staying heeled by a weight that is gone.
    let (ctx, mut app) = demo();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    let cond: LoadingCondition = serde_json::from_str(&text).unwrap();
    app.project_mut_for_test().loading_conditions.push(cond);
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(0);
    state.weights = vec![weight(0.5, 4.0, 0.8, 1.5)];
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    let mut clock = Clock(10.0);
    clock.wait(&ctx, &mut app, 0.8);
    let with = app.heel_state().heel_deg;
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    let rect = texts_at(&out, "Add to this condition").expect("the button is drawn");
    clock.click(&ctx, &mut app, rect.center());
    app.undo();
    app.compute_heel_blocking().unwrap();
    clock.wait(&ctx, &mut app, 0.8);
    let without = app
        .heel_current()
        .expect("current")
        .equilibrium
        .heel_for_side(1.0)
        .expect("the condition floats upright-ish");
    assert!(
        (with - without).abs() > 1.0,
        "the weight heeled it: {with} vs {without}"
    );
    let heel = app.heel_state().heel_deg;
    assert!(
        (heel - without).abs() < 0.01,
        "settled at {heel}, equilibrium {without}"
    );
}

#[test]
fn a_weight_the_hull_cannot_float_can_be_removed() {
    // A typo (1000 t for 1 t) must not trap the user: the weights table stays, and its ✕ removes
    // the weight, after which the view recovers.
    let (ctx, mut app, out) = shown_with(vec![weight(1000.0, 4.0, 0.0, 1.0)]);
    assert!(drawn(&out, "cannot float"), "{:?}", texts(&out));
    assert!(drawn(&out, "Trial weights (1)"), "{:?}", texts(&out));
    let mut clock = Clock(10.0);
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    let found = texts_clipped(&out, "✕");
    let remove = found
        .iter()
        .map(|(r, _)| *r)
        .find(|r| r.min.y > 560.0)
        .expect("the weight's ✕ is drawn");
    clock.click(&ctx, &mut app, remove.center());
    assert!(
        app.heel_state().weights.is_empty(),
        "the weight was removed"
    );
    app.compute_heel_blocking().unwrap();
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    assert!(!drawn(&out, "cannot float"), "{:?}", texts(&out));
    assert!(drawn(&out, "Settles"), "{:?}", texts(&out));
}

#[test]
fn a_kept_cannot_float_result_is_marked_out_of_date() {
    let (ctx, mut app, _) = shown_with(vec![weight(1000.0, 4.0, 0.0, 1.0)]);
    let mut state = app.heel_state().clone();
    state.weights[0].mass_t = 1.0;
    app.set_heel_state(state);
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(
        drawn(&out, "cannot float"),
        "the kept result: {:?}",
        texts(&out)
    );
    assert!(drawn(&out, "out of date"), "{:?}", texts(&out));
}

/// The demo with three loading conditions, A, B and C, open on the Heel view starting from `at`.
fn three_conditions(at: usize) -> (egui::Context, App) {
    let (ctx, mut app) = demo();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    for name in ["A", "B", "C"] {
        let mut cond: LoadingCondition = serde_json::from_str(&text).unwrap();
        cond.name = name.into();
        app.project_mut_for_test().loading_conditions.push(cond);
    }
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(at);
    app.set_heel_state(state);
    frame(&ctx, &mut app, Vec::new());
    (ctx, app)
}

#[test]
fn deleting_another_condition_keeps_the_start_on_its_own() {
    for at in [1, 2] {
        let (ctx, mut app) = three_conditions(at);
        let name = app.project().loading_conditions[at].name.clone();
        app.project_mut_for_test().loading_conditions.remove(0);
        frame(&ctx, &mut app, Vec::new());
        let HeelStart::Condition(i) = app.heel_state().start else {
            panic!("{name}: fell back to {:?}", app.heel_state().start);
        };
        assert_eq!(app.project().loading_conditions[i].name, name);
        assert!(!app.status().contains("gone"), "{}", app.status());
    }
}

#[test]
fn renaming_the_start_condition_falls_back_and_says_so() {
    let (ctx, mut app) = three_conditions(1);
    app.project_mut_for_test().loading_conditions[1].name = "B renamed".into();
    frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.heel_state().start, HeelStart::Dock);
    assert!(
        app.status().contains("that loading condition is gone"),
        "{}",
        app.status()
    );
}

#[test]
fn the_section_follows_a_model_edit() {
    // Raise the sheer at the section drawn: once the view is current again, its strong section is
    // the edited hull's, not one cut from the old hull while the new one was being refined.
    let (ctx, mut app, _) = shown_with(Vec::new());
    let top = |app: &App| {
        app.heel_section_drawn()
            .expect("a section is drawn")
            .iter()
            .flatten()
            .map(|p| p.z)
            .fold(f64::MIN, f64::max)
    };
    let surface = app.surface_mut().unwrap();
    let (i, p) = surface
        .points
        .iter()
        .enumerate()
        .map(|(i, p)| (i, p.p))
        .max_by(|a, b| a.1.z.total_cmp(&b.1.z))
        .unwrap();
    let mut state = app.heel_state().clone();
    state.section = Some(p.x);
    app.set_heel_state(state);
    frame(&ctx, &mut app, Vec::new());
    let before = top(&app);
    app.move_point(i, Vec3::new(p.x, p.y, p.z + 0.5)).unwrap();
    frame(&ctx, &mut app, Vec::new());
    app.compute_heel_blocking().unwrap();
    frame(&ctx, &mut app, Vec::new());
    let after = top(&app);
    // Same code path: the strong section matches the edited hull's section cut at `p.x`
    // and rose above the old hull's section (`before`).
    let expected_top = seacraft_gui::heel_view::geometry::HullOutlines::section_at(
        &app.heel_current().unwrap().hull,
        p.x,
    )
    .iter()
    .flatten()
    .map(|q| q.z)
    .fold(f64::MIN, f64::max);
    assert!(
        after > before && (after - expected_top).abs() < 1e-6,
        "the section was recut from the edited hull: {before} -> {after} (expected {expected_top})"
    );
}

#[test]
fn the_splitter_gives_the_drawings_more_height() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let d = app.heel_drawn().unwrap();
    assert!(d.splitter.is_positive(), "the splitter is drawn");
    assert!(
        d.body.width() < d.profile.width(),
        "the body plan is as wide as a section needs: {} vs {}",
        d.body.width(),
        d.profile.width()
    );
    let from = d.splitter.center();
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    clock.frame(&ctx, &mut app, vec![press(from, true)]);
    for k in 1..=10 {
        let p = from + egui::vec2(0.0, 6.0 * k as f32);
        clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p)]);
    }
    clock.frame(
        &ctx,
        &mut app,
        vec![press(from + egui::vec2(0.0, 60.0), false)],
    );
    clock.frame(&ctx, &mut app, Vec::new());
    let after = app.heel_drawn().unwrap();
    let split = app
        .heel_state()
        .split
        .expect("dragging the splitter stores a custom split");
    assert!(
        split > seacraft_gui::heel_view::DEFAULT_SPLIT,
        "custom split {split} did not grow past default"
    );
    // 60 pt downward drag grows the drawings' height, well beyond 1 pt layout rounding.
    assert!(
        after.body.height() > d.body.height() + 1.0,
        "{} -> {}",
        d.body.height(),
        after.body.height()
    );

    // Double-clicking the splitter handle restores the default split.
    let mid = after.splitter.center();
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(mid)]);
    clock.frame(&ctx, &mut app, vec![press(mid, true)]);
    clock.frame(&ctx, &mut app, vec![press(mid, false)]);
    clock.frame(&ctx, &mut app, vec![press(mid, true)]);
    clock.frame(&ctx, &mut app, vec![press(mid, false)]);
    clock.frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.heel_state().split, None);
}

#[test]
fn small_trial_mass_is_not_clamped_when_rendered_and_added() {
    let (ctx, mut app) = demo();
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    let cond: LoadingCondition = serde_json::from_str(&text).unwrap();
    app.project_mut_for_test().loading_conditions.push(cond);
    app.set_heel_view_open(true);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(0);
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    let mut clock = Clock(10.0);
    clock.frame(&ctx, &mut app, Vec::new());
    clock.frame(&ctx, &mut app, Vec::new());

    let p = body_point(&app);
    clock.click(&ctx, &mut app, p);
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("0.0005".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    assert_eq!(app.heel_state().weights[0].mass_t, 0.0005);

    // Rendering the table must not silently clamp 0.0005 t up to 0.001 t.
    for _ in 0..3 {
        clock.frame(&ctx, &mut app, Vec::new());
        assert_eq!(app.heel_state().weights[0].mass_t, 0.0005);
    }

    app.compute_heel_blocking().unwrap();
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    let rect = texts_at(&out, "Add to this condition").expect("the button is drawn");
    clock.click(&ctx, &mut app, rect.center());
    let items = &app.project().loading_conditions[0].items;
    assert_eq!(items.last().unwrap().mass_t, 0.0005);
}

#[test]
fn manual_heel_change_cancels_pending_settle() {
    let (ctx, mut app, mut clock) = interactive(Vec::new());
    let mut state = app.heel_state().clone();
    state.weights = vec![weight(0.5, 4.0, 0.8, 1.5)];
    app.set_heel_state(state);
    // First frame queues heel_settle_pending while the sweep awaits the quiet period.
    let p = body_point(&app);
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p)]);
    // Manual arrow-key heel edit before the sweep finishes must cancel the pending settle.
    clock.frame(&ctx, &mut app, vec![key(egui::Key::ArrowRight)]);
    assert_eq!(app.heel_state().heel_deg, 1.0);
    for _ in 0..300 {
        clock.frame(&ctx, &mut app, Vec::new());
        if app.heel_current().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.heel_current().is_some(), "the sweep finished");
    clock.wait(&ctx, &mut app, 0.7);
    assert_eq!(app.heel_state().heel_deg, 1.0);
    assert!(app.heel_state().anim.is_none());
}

#[test]
fn closing_or_switching_drawer_clears_transient_grab() {
    let (ctx, mut app, mut clock) = interactive(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let d = app.heel_drawn().unwrap();
    let from = d.body.center();
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    clock.frame(&ctx, &mut app, vec![press(from, true)]);
    clock.frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(from + egui::vec2(20.0, 0.0))],
    );
    assert_eq!(
        app.heel_state().grab,
        Some(seacraft_gui::heel_view::Grab::Heel)
    );
    let heeled = app.heel_state().heel_deg;
    app.open_analysis_view(AnalysisView::Offsets);
    assert_eq!(app.heel_state().grab, None);
    assert_eq!(app.heel_state().heel_deg, heeled);
    assert_eq!(app.heel_state().weights.len(), 1);
    clock.frame(
        &ctx,
        &mut app,
        vec![press(from + egui::vec2(20.0, 0.0), false)],
    );

    app.set_heel_view_open(true);
    clock.frame(&ctx, &mut app, Vec::new());
    let d = app.heel_drawn().unwrap();
    let from = d.body.center();
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    clock.frame(&ctx, &mut app, vec![press(from, true)]);
    clock.frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(from + egui::vec2(20.0, 0.0))],
    );
    assert_eq!(
        app.heel_state().grab,
        Some(seacraft_gui::heel_view::Grab::Heel)
    );
    app.set_heel_view_open(false);
    assert_eq!(app.heel_state().grab, None);
    assert_eq!(app.heel_state().weights.len(), 1);
}

#[test]
fn fast_weight_drag_picks_target_at_press_origin() {
    let (ctx, mut app, mut clock) = interactive(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let mut state = app.heel_state().clone();
    state.hold = true;
    app.set_heel_state(state);
    clock.frame(&ctx, &mut app, Vec::new());
    let d = app.heel_drawn().unwrap();
    let (_, c, h) = d.frame.to_earth(Vec3::new(4.0, 0.8, 1.5));
    let on = d.body_view.to_screen([c, h]);
    let heel_before = app.heel_state().heel_deg;
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(on)]);
    clock.frame(&ctx, &mut app, vec![press(on, true)]);
    let jumped = on + egui::vec2(25.0, 0.0);
    clock.frame(&ctx, &mut app, vec![egui::Event::PointerMoved(jumped)]);
    clock.frame(&ctx, &mut app, vec![press(jumped, false)]);
    assert_eq!(app.heel_state().heel_deg, heel_before);
    assert!(
        app.heel_state().weights[0].y > 0.9,
        "weight did not move: {}",
        app.heel_state().weights[0].y
    );
}

#[test]
fn stale_result_shows_update_failed_when_current_input_fails() {
    let (ctx, mut app, _) = shown_with(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    let text = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../examples/demo-loadcase.json"
    ))
    .unwrap();
    let mut bad: LoadingCondition = serde_json::from_str(&text).unwrap();
    bad.name = "Zero".into();
    bad.lightship.mass_t = 0.0;
    bad.items.clear();
    bad.tanks.clear();
    app.project_mut_for_test().loading_conditions.push(bad);
    let mut state = app.heel_state().clone();
    state.start = HeelStart::Condition(0);
    app.set_heel_state(state);
    for _ in 0..50 {
        frame(&ctx, &mut app, Vec::new());
        if app.heel_failure().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    assert!(app.heel_failure().is_some(), "expected condition failure");
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(
        drawn(&out, "out of date — update failed"),
        "{:?}",
        texts(&out)
    );
    assert!(
        !drawn(&out, "out of date — recomputing"),
        "{:?}",
        texts(&out)
    );
}

fn has_colour(out: &egui::FullOutput, want: egui::Color32) -> bool {
    fn walk(shape: &egui::Shape, want: egui::Color32) -> bool {
        match shape {
            egui::Shape::Vec(v) => v.iter().any(|s| walk(s, want)),
            egui::Shape::Mesh(m) => m.vertices.iter().any(|v| v.color == want),
            egui::Shape::Rect(r) => r.fill == want || r.stroke.color == want,
            _ => false,
        }
    }
    out.shapes.iter().any(|c| walk(&c.shape, want))
}

#[test]
fn capsizing_condition_paints_red_sky_and_capsize_badge() {
    // 0.5 t high and far out to starboard: the condition capsizes across the sweep.
    let (_ctx, _app, cap_out) = shown_with(vec![weight(0.5, 4.3, 2.79, 2.3)]);
    assert!(
        has_colour(&cap_out, seacraft_gui::theme::HEEL_SKY_CAPSIZE_TOP),
        "capsizing view must paint reddish sky backdrop"
    );
    assert!(drawn(&cap_out, "⚠ Capsizes"), "{:?}", texts(&cap_out));
    assert!(drawn(&cap_out, "Capsizing"), "{:?}", texts(&cap_out));
    assert!(drawn(&cap_out, "kN·m"), "{:?}", texts(&cap_out));
    assert!(drawn(&cap_out, "kN"), "{:?}", texts(&cap_out));

    // A settled condition must not paint the danger sky, unless held past its vanishing angle.
    let (ctx, mut app, safe_out) = shown_with(Vec::new());
    assert!(
        !has_colour(&safe_out, seacraft_gui::theme::HEEL_SKY_CAPSIZE_TOP),
        "stable upright hull must not paint reddish sky"
    );
    assert!(!drawn(&safe_out, "⚠ Capsizes"));

    let mut state = app.heel_state().clone();
    state.hold = true;
    state.heel_deg = 40.0;
    app.set_heel_state(state);
    app.compute_heel_blocking().unwrap();
    frame(&ctx, &mut app, Vec::new());
    let past_out = frame(&ctx, &mut app, Vec::new());
    assert!(
        has_colour(&past_out, seacraft_gui::theme::HEEL_SKY_CAPSIZE_TOP),
        "holding beyond vanishing angle paints reddish sky"
    );
    assert!(
        drawn(&past_out, "Past vanishing angle — capsizes"),
        "{:?}",
        texts(&past_out)
    );
    assert!(
        !drawn(&past_out, "Settles upright"),
        "must not display a contradicting Settles headline above the capsize card: {:?}",
        texts(&past_out)
    );
    assert!(
        drawn(&past_out, "Within righting range: settles upright"),
        "{:?}",
        texts(&past_out)
    );
}

#[test]
fn gz_plot_sits_directly_below_the_splitter_without_a_gap() {
    let (_ctx, app, out) = shown_with(Vec::new());
    let d = app.heel_drawn().unwrap();
    let plot_title = texts_at(&out, "Righting arm against heel (°), free to trim")
        .expect("GZ plot title is drawn");
    let toggle = texts_at(&out, "Moment (kN·m)").expect("plot moment toggle is drawn");
    let toggle_gap = toggle.min.y - d.splitter.max.y;
    let plot_gap = plot_title.min.y - d.splitter.max.y;
    assert!(
        toggle_gap < 30.0,
        "toggle pushed {toggle_gap} pt below splitter"
    );
    assert!(
        plot_gap < 50.0,
        "GZ plot pushed {plot_gap} pt below splitter"
    );
}

#[test]
fn manual_recalculate_waits_for_button_click() {
    let (ctx, mut app, _) = shown_with(Vec::new());
    let mut state = app.heel_state().clone();
    state.auto_recalculate = false;
    app.set_heel_state(state);

    let mut clock = Clock(10.0);
    clock.frame(&ctx, &mut app, Vec::new());

    let mut state = app.heel_state().clone();
    state.weights.push(weight(0.5, 4.0, 0.8, 1.5));
    app.set_heel_state(state);

    std::thread::sleep(std::time::Duration::from_millis(160));
    let mut out = clock.frame_out(&ctx, &mut app, Vec::new());
    for _ in 0..3 {
        out = clock.frame_out(&ctx, &mut app, Vec::new());
    }
    assert!(app.heel_stale(), "result stays stale until Recalculate");
    assert!(
        !app.heel_sweep_busy(),
        "manual mode must not auto-start sweep job"
    );
    assert!(app.heel_current().is_none());
    assert!(
        drawn(&out, "out of date — click Recalculate"),
        "{:?}",
        texts(&out)
    );
    let btn = texts_at(&out, "Recalculate").expect("Recalculate button is drawn");
    clock.click(&ctx, &mut app, btn.center());

    for _ in 0..300 {
        clock.frame(&ctx, &mut app, Vec::new());
        if app.heel_current().is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(20));
    }
    let computed = app.heel_current().expect("Recalculate completed the sweep");
    assert!(!app.heel_stale());
    assert!(computed.equilibrium.heel_for_side(0.0).unwrap() > 0.0);
}

#[test]
fn non_finite_mass_in_table_is_rejected() {
    let (ctx, mut app, mut clock) = interactive(vec![weight(1.234, 4.0, 0.0, 1.0)]);
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    let field = texts_at(&out, "1.234").expect("table mass field is drawn");
    clock.click(&ctx, &mut app, field.center());
    clock.frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    );
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("inf".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    let m = app.heel_state().weights[0].mass_t;
    assert!(m.is_finite(), "non-finite mass must never be stored: {m}");
    assert!((m - 1.234).abs() < 1e-12, "mass stayed unchanged: {m}");

    // Confirm the same click target actually opens the mass DragValue for valid edits.
    let out = clock.frame_out(&ctx, &mut app, Vec::new());
    let field = texts_at(&out, "1.234").expect("mass field still drawn");
    clock.click(&ctx, &mut app, field.center());
    clock.frame(
        &ctx,
        &mut app,
        vec![egui::Event::Key {
            key: egui::Key::A,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers: egui::Modifiers::CTRL,
        }],
    );
    clock.frame(&ctx, &mut app, vec![egui::Event::Text("2.5".into())]);
    clock.frame(&ctx, &mut app, vec![key(egui::Key::Enter)]);
    let m = app.heel_state().weights[0].mass_t;
    assert!((m - 2.5).abs() < 1e-12, "valid mass edit applied: {m}");
}

#[test]
fn plot_toggle_switches_between_gz_and_moment() {
    let (ctx, mut app, out) = shown_with(vec![weight(0.5, 4.0, 0.8, 1.5)]);
    assert!(
        drawn(&out, "Righting arm against heel"),
        "{:?}",
        texts(&out)
    );
    let mut clock = Clock(10.0);
    let toggle = texts_at(&out, "Moment (kN·m)").expect("plot moment toggle is drawn");
    clock.click(&ctx, &mut app, toggle.center());
    assert!(app.heel_state().plot_moment);
    let after = clock.frame_out(&ctx, &mut app, Vec::new());
    assert!(
        drawn(&after, "Righting moment against heel"),
        "{:?}",
        texts(&after)
    );
}
