//! The Model menu, headless (R5-7): Transform… and Optimise… dialogs (the
//! optimiser on the job layer), Insert edge / Split face / Crease on the
//! picked points or face, each one undo step; disabled items say why; and
//! the axis-locked point drag (X, Y or Z held).

use std::time::{Duration, Instant};

use eframe::egui;
use egui::accesskit::Role;
use seacraft_core::math::Vec3;
use seacraft_core::transform::Axis;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// One widget of the last frame: role, label, rectangle, disabled.
#[derive(Debug, Clone)]
struct Widget {
    role: Role,
    label: String,
    /// A label's text, a text field's contents.
    value: String,
    rect: egui::Rect,
    disabled: bool,
}

/// One headless window and the widgets its last frame drew.
struct Window {
    ctx: egui::Context,
    app: App,
    widgets: Vec<Widget>,
}

impl Window {
    fn demo() -> Self {
        let mut app = App::headless_for_test(Some(&demo_path()));
        app.set_subdivision_level(2);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        // Tooltips show at once, so a hover test needs no waiting.
        ctx.global_style_mut(|s| {
            s.interaction.tooltip_delay = 0.0;
            s.interaction.show_tooltips_only_when_still = false;
        });
        let mut w = Self {
            ctx,
            app,
            widgets: Vec::new(),
        };
        w.frame(vec![]);
        w.frame(vec![]);
        w
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
        self.widgets = out
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let b = node.bounds()?;
                        Some(Widget {
                            role: node.role(),
                            label: node.label().unwrap_or("").to_owned(),
                            value: node.value().unwrap_or("").to_owned(),
                            rect: egui::Rect::from_min_max(
                                egui::pos2(b.x0 as f32, b.y0 as f32),
                                egui::pos2(b.x1 as f32, b.y1 as f32),
                            ),
                            disabled: node.is_disabled(),
                        })
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn get(&self, label: &str) -> Option<&Widget> {
        self.widgets.iter().find(|w| w.label == label)
    }

    fn find(&self, label: &str) -> Widget {
        self.get(label).cloned().unwrap_or_else(|| {
            panic!(
                "no widget `{label}` among {:?}",
                self.widgets
                    .iter()
                    .map(|w| (w.role, w.label.as_str()))
                    .collect::<Vec<_>>()
            )
        })
    }

    fn click_at(&mut self, pos: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
        self.frame(vec![button(false)]);
        self.frame(vec![]);
    }

    fn click(&mut self, label: &str) {
        let w = self.find(label);
        assert!(!w.disabled, "`{label}` is disabled");
        self.click_at(w.rect.center());
    }

    /// Open the Model menu and click one of its items.
    fn model(&mut self, item: &str) {
        self.click("Model");
        self.click(item);
    }

    fn settle_optimise(&mut self) {
        let t0 = Instant::now();
        while self.app.optimise_running() {
            assert!(
                t0.elapsed() < Duration::from_secs(120),
                "the optimisation never finished"
            );
            std::thread::sleep(Duration::from_millis(10));
            self.frame(vec![]);
        }
        self.frame(vec![]);
    }
}

fn key(key: egui::Key, pressed: bool) -> egui::Event {
    egui::Event::Key {
        key,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    }
}

/// A face of the demo with four corners (it splits along a diagonal).
fn quad_face(app: &App) -> usize {
    let s = app.surface().unwrap();
    (0..s.faces.len())
        .find(|&f| s.faces[f].len() == 4)
        .expect("a quad face")
}

#[test]
fn disabled_items_say_why_until_their_target_is_picked() {
    let mut w = Window::demo();
    w.click("Model");
    for item in ["Transform…", "Optimise…"] {
        assert!(!w.find(item).disabled, "{item} works on any model");
    }
    for item in ["Insert edge", "Split face", "Crease edge", "Remove crease"] {
        assert!(w.find(item).disabled, "{item} needs a pick");
    }
    // Hovering a disabled item shows the reason.
    let split = w.find("Split face").rect.center();
    w.frame(vec![egui::Event::PointerMoved(split)]);
    // The pointer rests (a frame is 1/60 s) until the tooltip shows.
    for _ in 0..30 {
        w.frame(vec![]);
    }
    let reason = "Click a face of the hull in a viewport first.";
    assert!(
        w.widgets.iter().any(|x| x.value == reason),
        "no tooltip `{reason}` among {:?}",
        w.widgets.iter().map(|x| &x.value).collect::<Vec<_>>()
    );
    // The same reasons, as the app reports them.
    let t = w.app.model_targets();
    assert_eq!(t.split_face.unwrap_err(), reason);
    assert!(t.insert_edge.unwrap_err().contains("Shift+click"));

    // Without a model every item is disabled.
    let mut empty = Window::demo();
    empty.app = App::headless_empty();
    empty.frame(vec![]);
    empty.frame(vec![]);
    empty.click("Model");
    for item in ["Transform…", "Optimise…", "Insert edge", "Split face"] {
        assert!(empty.find(item).disabled, "{item} without a model");
    }
}

fn points(app: &App) -> Vec<Vec3> {
    app.surface().unwrap().points.iter().map(|p| p.p).collect()
}

fn press(pos: egui::Pos2, pressed: bool, modifiers: egui::Modifiers) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers,
    }
}

/// Sheer points of the demo between 2 and 10 m, highest first, at least
/// `gap` apart in x: well clear of other points in the profile view.
fn sheer_points(app: &App, gap: f64) -> Vec<usize> {
    let s = app.surface().unwrap();
    let mut pts: Vec<usize> = (0..s.points.len())
        .filter(|&i| s.points[i].p.x > 2.0 && s.points[i].p.x < 10.0)
        .collect();
    pts.sort_by(|&a, &b| s.points[b].p.z.total_cmp(&s.points[a].p.z));
    let mut out: Vec<usize> = Vec::new();
    for i in pts {
        if out
            .iter()
            .all(|&o| (s.points[o].p.x - s.points[i].p.x).abs() > gap)
        {
            out.push(i);
        }
    }
    out
}

/// Where control point `index` is drawn in viewport `view`.
fn on_screen(app: &App, view: usize, index: usize) -> egui::Pos2 {
    let rect = app.viewport_rect(view);
    let p = app.surface().unwrap().points[index].p;
    let s = app.cameras()[view]
        .transform()
        .project(p, rect)
        .expect("point visible");
    assert!(rect.contains(s), "point {index} inside view {view}");
    s
}

#[test]
fn transform_moves_the_hull_as_one_undo_step() {
    let mut w = Window::demo();
    let before = points(&w.app);
    let undo = w.app.undo_len();

    // Cancel changes nothing.
    w.model("Transform…");
    assert!(w.app.transform_dialog_mut().is_some(), "the dialog is open");
    w.click("Cancel");
    assert!(w.app.transform_dialog_mut().is_none());
    assert_eq!(w.app.undo_len(), undo);

    w.model("Transform…");
    let d = w.app.transform_dialog_mut().unwrap();
    let lwl = d.current.expect("the fit defaults are measured").lwl;
    assert!(lwl > 5.0, "{lwl}");
    // Nothing to do yet: Apply is disabled and says why.
    w.frame(vec![]);
    assert!(w.find("Apply").disabled);
    assert!(w.widgets.iter().any(|x| x.value.contains("nothing to do")));

    w.app.transform_dialog_mut().unwrap().translate = [0.0, 0.0, 0.25];
    w.frame(vec![]);
    w.click("Apply");
    assert!(w.app.transform_dialog_mut().is_none(), "Apply closes it");
    assert_eq!(w.app.undo_len(), undo + 1, "one undo step");
    assert!(w.app.is_dirty());
    for (a, b) in before.iter().zip(points(&w.app)) {
        assert!((b - (*a + Vec3::new(0.0, 0.0, 0.25))).length() < 1e-12);
    }

    // A fit to a longer waterline, through the same dialog.
    w.model("Transform…");
    w.app.transform_dialog_mut().unwrap().lwl = Some(lwl * 1.1);
    w.frame(vec![]);
    w.click("Apply");
    assert_eq!(w.app.undo_len(), undo + 2);
    w.app.recompute_hydrostatics();
    let fitted = w.app.hydrostatics().unwrap().lwl;
    assert!((fitted / (lwl * 1.1) - 1.0).abs() < 2e-3, "{fitted}");

    w.app.undo();
    w.app.undo();
    for (a, b) in before.iter().zip(points(&w.app)) {
        assert!((b - *a).length() < 1e-12, "undo restores the hull");
    }
}

#[test]
fn optimise_runs_as_a_job_and_installs_one_undo_step() {
    let mut w = Window::demo();
    let before = points(&w.app);
    let undo = w.app.undo_len();
    w.model("Optimise…");
    {
        let d = w.app.optimise_dialog_mut().expect("the window is open");
        d.speed_kn = 8.0;
        d.vars = [false, false, true];
        d.max_evaluations = 6;
        d.level = 1;
    }
    w.frame(vec![]);
    let run = w.find("Run").rect.center();
    w.frame(vec![
        egui::Event::PointerMoved(run),
        press(run, true, egui::Modifiers::NONE),
    ]);
    w.frame(vec![press(run, false, egui::Modifiers::NONE)]);
    // The click returned with the run still going: it is a job, polled by
    // the frames, and the model is untouched until it ends.
    assert!(
        w.app.optimise_running(),
        "the optimiser runs off the UI thread"
    );
    assert_eq!(w.app.undo_len(), undo);
    assert!(
        w.app.model_targets().optimise.is_err(),
        "one optimisation at a time"
    );

    w.settle_optimise();
    assert_eq!(w.app.undo_len(), undo + 1, "the result is one undo step");
    assert_eq!(w.app.control_point_count(), before.len(), "topology kept");
    assert_ne!(points(&w.app), before, "the hull changed");
    let d = w.app.optimise_dialog_mut().unwrap();
    assert!(d.error.is_none(), "{:?}", d.error);
    let summary = d.result.clone().expect("a summary");
    assert!(
        summary.contains("R_T") && summary.contains("draft"),
        "{summary}"
    );
    assert!(w.app.model_targets().optimise.is_ok());

    w.app.undo();
    assert_eq!(points(&w.app), before, "undo restores the hull");

    w.click("Close");
    assert!(w.app.optimise_dialog_mut().is_none());
}

#[test]
fn an_optimisation_for_an_edited_hull_is_discarded() {
    use seacraft_core::optimize::{DesignVar, OptimizeSpec, VarSpec};
    let mut w = Window::demo();
    w.app.open_optimise_dialog();
    let mut spec = OptimizeSpec::new(8.0);
    spec.vars = vec![VarSpec::new(DesignVar::LbRatioScale)];
    spec.max_evaluations = 4;
    w.app.start_optimise(spec, 1);
    // An edit lands before the next frame collects the result.
    let p = w.app.surface().unwrap().points[5].p + Vec3::new(0.0, 0.0, 0.01);
    w.app.move_point(5, p).unwrap();
    let edited = points(&w.app);
    let undo = w.app.undo_len();
    w.settle_optimise();
    assert_eq!(w.app.undo_len(), undo, "nothing was installed");
    assert_eq!(points(&w.app), edited, "the edit stands");
    let err = w.app.optimise_dialog_mut().unwrap().error.clone();
    assert!(err.is_some_and(|e| e.contains("discarded")));
}

#[test]
fn insert_edge_crease_and_split_face_act_on_the_picks() {
    let mut w = Window::demo();
    let f = quad_face(&w.app);
    let c = w.app.surface().unwrap().faces[f].clone();
    let (faces, pts, undo) = (
        w.app.face_count(),
        w.app.control_point_count(),
        w.app.undo_len(),
    );
    let edge = |app: &App| {
        let key = seacraft_core::mesh::ordered(c[0], c[2]);
        app.surface()
            .unwrap()
            .edges
            .iter()
            .find(|e| e.key() == key)
            .map(|e| e.crease)
    };
    assert_eq!(edge(&w.app), None);

    // Two opposite corners of a quad: the diagonal.
    w.app.set_picked_points(Some([c[0], c[2]]));
    w.app.set_picked_face(Some(f));
    w.frame(vec![]);
    assert_eq!(w.app.model_targets().insert_edge, Ok((f, c[0], c[2])));
    w.model("Insert edge");
    assert_eq!(w.app.face_count(), faces + 1);
    assert_eq!(edge(&w.app), Some(false), "the new edge joins the picks");
    assert_eq!(w.app.undo_len(), undo + 1);

    w.model("Crease edge");
    assert_eq!(edge(&w.app), Some(true));
    assert_eq!(w.app.undo_len(), undo + 2);
    assert!(w.app.model_targets().crease.is_err(), "already a crease");
    w.model("Remove crease");
    assert_eq!(edge(&w.app), Some(false));
    assert_eq!(w.app.undo_len(), undo + 3);

    // The picked face is now a triangle: three triangles about a new point.
    w.model("Split face");
    assert_eq!(w.app.face_count(), faces + 3);
    assert_eq!(w.app.control_point_count(), pts + 1);
    assert_eq!(w.app.undo_len(), undo + 4);
    assert!(w.app.is_dirty());

    for _ in 0..4 {
        w.app.undo();
    }
    assert_eq!(w.app.face_count(), faces);
    assert_eq!(w.app.control_point_count(), pts);
    assert_eq!(edge(&w.app), None);
}

/// A click at `pos` with `modifiers` held on press and release.
fn click_with(w: &mut Window, pos: egui::Pos2, modifiers: egui::Modifiers) {
    w.frame(vec![
        egui::Event::PointerMoved(pos),
        press(pos, true, modifiers),
    ]);
    w.frame(vec![press(pos, false, modifiers)]);
    w.frame(vec![]);
}

#[test]
fn shift_click_picks_a_second_point_and_a_click_on_the_hull_picks_a_face() {
    let mut w = Window::demo();
    let sheer = sheer_points(&w.app, 2.0);
    let (a, b) = (
        on_screen(&w.app, 1, sheer[0]),
        on_screen(&w.app, 1, sheer[1]),
    );
    click_with(&mut w, a, egui::Modifiers::NONE);
    let first = w.app.selected().expect("a point is selected");
    assert_eq!(w.app.picked_points(), None, "one point is not a pair");
    click_with(&mut w, b, egui::Modifiers::SHIFT);
    let second = w.app.selected().expect("the second point is selected");
    assert_ne!(first, second);
    assert_eq!(w.app.picked_points(), Some([first, second]));
    assert_eq!(w.app.undo_len(), 0, "picking is not an edit");
    // A plain click starts over with one point.
    click_with(&mut w, a, egui::Modifiers::NONE);
    assert_eq!(w.app.selected(), Some(first));
    assert_eq!(w.app.picked_points(), None);
    // Escape drops the picks.
    click_with(&mut w, b, egui::Modifiers::SHIFT);
    assert!(w.app.picked_points().is_some());
    w.frame(vec![
        key(egui::Key::Escape, true),
        key(egui::Key::Escape, false),
    ]);
    assert_eq!(w.app.picked_points(), None);

    // A click on the hull below the sheer picks the face under it, and
    // Split face then names it.
    w.app.set_picked_face(None);
    assert!(w.app.model_targets().split_face.is_err());
    let mid = egui::pos2((a.x + b.x) * 0.5, (a.y + b.y) * 0.5 + 25.0);
    assert!(w.app.viewport_rect(1).contains(mid));
    click_with(&mut w, mid, egui::Modifiers::NONE);
    let face = w.app.picked_face().expect("a face under the click");
    assert_eq!(w.app.model_targets().split_face, Ok(face));
}

/// Press on control point `index` in the profile view and drag it by
/// `delta` pixels with `held` keys down (still pressed at the end);
/// returns the grabbed point's displacement (a point behind `index` in the
/// profile may be the one grabbed).
fn drag_with(w: &mut Window, index: usize, delta: egui::Vec2, held: &[egui::Key]) -> Vec3 {
    let s = on_screen(&w.app, 1, index);
    w.frame(vec![egui::Event::PointerMoved(s)]);
    w.frame(vec![press(s, true, egui::Modifiers::NONE)]);
    let index = w.app.selected().expect("the press grabbed a point");
    let before = w.app.surface().unwrap().points[index].p;
    let mut events: Vec<egui::Event> = held.iter().map(|&k| key(k, true)).collect();
    events.push(egui::Event::PointerMoved(s + delta * 0.25));
    w.frame(events);
    w.frame(vec![egui::Event::PointerMoved(s + delta)]);
    w.frame(vec![egui::Event::PointerMoved(s + delta)]);
    w.app.surface().unwrap().points[index].p - before
}

/// Release the drag [`drag_with`] started, then the keys.
fn release_drag(w: &mut Window, held: &[egui::Key]) {
    let at = w.ctx.input(|i| i.pointer.hover_pos()).expect("the pointer");
    w.frame(vec![press(at, false, egui::Modifiers::NONE)]);
    w.frame(held.iter().map(|&k| key(k, false)).collect());
}

#[test]
fn holding_x_or_z_locks_a_drag_to_that_world_axis() {
    for (k, axis, name) in [
        (egui::Key::X, Axis::X, "along x"),
        (egui::Key::Z, Axis::Z, "along z"),
    ] {
        let mut w = Window::demo();
        let index = sheer_points(&w.app, 2.0)[0];
        let per_px = {
            let rect = w.app.viewport_rect(1);
            w.app.cameras()[1].transform().world_per_px(rect, 1.0)
        };
        // Right 40 px and up 30 px: a free drag would move x and z.
        let moved = drag_with(&mut w, index, egui::vec2(40.0, -30.0), &[k]);
        assert_eq!(w.app.axis_lock(), Some(axis), "the lock shows while held");
        assert!(w.app.status().contains(name), "{}", w.app.status());
        if axis == Axis::X {
            assert!((moved.x - 40.0 * per_px).abs() < 0.05, "{moved:?}");
            assert!(moved.y == 0.0 && moved.z == 0.0, "{moved:?}");
        } else {
            assert!((moved.z - 30.0 * per_px).abs() < 0.05, "{moved:?}");
            assert!(moved.x == 0.0 && moved.y == 0.0, "{moved:?}");
        }
        release_drag(&mut w, &[k]);
        assert_eq!(w.app.axis_lock(), None, "released");
        assert_eq!(w.app.undo_len(), 1, "the drag is one undo step");
    }
    // Without a key the same drag moves both.
    let mut w = Window::demo();
    let index = sheer_points(&w.app, 2.0)[0];
    let moved = drag_with(&mut w, index, egui::vec2(40.0, -30.0), &[]);
    assert_eq!(w.app.axis_lock(), None);
    assert!(moved.x > 0.0 && moved.z > 0.0, "{moved:?}");
}

#[test]
fn axis_keys_are_text_while_a_field_has_focus() {
    let mut w = Window::demo();
    // The layer name field has the keyboard.
    let field = w
        .widgets
        .iter()
        .find(|x| x.role == Role::TextInput)
        .map(|x| x.rect)
        .expect("a text field");
    w.click_at(field.center());
    assert!(w.ctx.text_edit_focused(), "the field has the keyboard");
    let index = sheer_points(&w.app, 2.0)[0];
    let moved = drag_with(&mut w, index, egui::vec2(40.0, -30.0), &[egui::Key::X]);
    assert!(
        w.ctx.text_edit_focused(),
        "the field kept the keyboard through the drag"
    );
    assert_eq!(w.app.axis_lock(), None, "X typed into a field is no lock");
    assert!(moved.x > 0.0 && moved.z > 0.0, "a free drag: {moved:?}");
}
