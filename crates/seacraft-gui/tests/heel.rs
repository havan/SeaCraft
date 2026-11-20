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
    for _ in 0..5 {
        frame(&ctx, &mut app, Vec::new());
    }
    assert!(!app.heel_sweep_busy(), "no condition job without a KG");
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
        "righting lever",
        "heeling arm",
        "projected onto the section",
        "From starboard",
        "not a roll simulation",
        "0.5 t",
    ] {
        assert!(drawn(&out, part), "`{part}` not drawn: {all:?}");
    }
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
    for part in ["righting lever", "Settles", "Trial weights"] {
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
