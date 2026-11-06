//! The layer panel, headless: its buttons, list entries and property
//! switches drive the layer commands (one undo step each), a click on the
//! hull picks the control face under it, and "Move picked face to this
//! layer" moves it without changing a number.

use eframe::egui;
use egui::accesskit::Role;
use seacraft_core::math::Vec3;
use seacraft_gui::camera::ViewKind;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// One headless window and the widgets its last frame drew.
struct Window {
    ctx: egui::Context,
    app: App,
    /// Role, label and rectangle of every widget drawn.
    widgets: Vec<(Role, String, egui::Rect)>,
}

impl Window {
    fn demo() -> Self {
        let mut app = App::headless_for_test(Some(&demo_path()));
        app.set_subdivision_level(2);
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
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
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(b.x0 as f32, b.y0 as f32),
                            egui::pos2(b.x1 as f32, b.y1 as f32),
                        );
                        Some((node.role(), node.label().unwrap_or("").to_owned(), rect))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn all(&self, role: Role, label: &str) -> Vec<egui::Rect> {
        self.widgets
            .iter()
            .filter(|(r, l, _)| *r == role && l == label)
            .map(|(_, _, rect)| *rect)
            .collect()
    }

    fn find(&self, role: Role, label: &str) -> egui::Rect {
        self.all(role, label).first().copied().unwrap_or_else(|| {
            panic!(
                "no {role:?} `{label}` among {:?}",
                self.widgets
                    .iter()
                    .filter(|(r, _, _)| *r == role)
                    .map(|(_, l, _)| l)
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

    fn click(&mut self, role: Role, label: &str) {
        let rect = self.find(role, label);
        self.click_at(rect.center());
    }

    fn names(&self) -> Vec<String> {
        let s = self.app.surface().unwrap();
        s.layers.iter().map(|l| l.name.clone()).collect()
    }
}

#[test]
fn the_demo_file_has_three_named_layers_that_change_no_number() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let s = app.surface().unwrap();
    assert_eq!(
        s.layers.iter().map(|l| l.name.as_str()).collect::<Vec<_>>(),
        ["Bottom", "Topsides", "Transom"]
    );
    assert!(s.layer_face_counts().iter().all(|&c| c > 0));
    // The hull-numerics skill reference numbers (level 3, Loop), unchanged.
    app.recompute_hydrostatics();
    let h = app.hydrostatics().unwrap();
    assert!((h.volume_m3 - 11.851).abs() < 0.001, "{}", h.volume_m3);
    assert!((h.block_coefficient - 0.5042).abs() < 0.0001);
    assert!((h.waterplane_coefficient - 0.8235).abs() < 0.0001);
    assert!((h.kb - 0.392).abs() < 0.001);
    assert!((h.km_transverse - 2.625).abs() < 0.001);
}

#[test]
fn add_reorder_and_delete_buttons_drive_the_layer_commands() {
    let mut w = Window::demo();
    assert_eq!(w.names(), ["Bottom", "Topsides", "Transom"]);
    // The list shows each layer as a button; the first is active.
    for name in ["Bottom", "Topsides", "Transom"] {
        w.find(Role::Button, name);
    }
    assert_eq!(w.app.active_layer(), 0);
    let undo = w.app.undo_len();

    w.click(Role::Button, "Add");
    assert_eq!(w.names(), ["Bottom", "Topsides", "Transom", "Layer 4"]);
    assert_eq!(w.app.active_layer(), 3, "the new layer is active");
    assert_eq!(w.app.undo_len(), undo + 1);
    assert!(w.app.is_dirty());

    w.click(Role::Button, "↑");
    assert_eq!(w.names(), ["Bottom", "Topsides", "Layer 4", "Transom"]);
    assert_eq!(w.app.active_layer(), 2);
    w.click(Role::Button, "↓");
    assert_eq!(w.names(), ["Bottom", "Topsides", "Transom", "Layer 4"]);
    assert_eq!(w.app.undo_len(), undo + 3);

    // The empty layer goes; the faces never move.
    let counts = w.app.surface().unwrap().layer_face_counts();
    w.click(Role::Button, "Delete");
    assert_eq!(w.names(), ["Bottom", "Topsides", "Transom"]);
    assert_eq!(w.app.surface().unwrap().layer_face_counts(), counts[..3]);
    assert_eq!(w.app.undo_len(), undo + 4);

    // Clicking a name activates it (not an edit); Delete then hands its
    // faces to the layer above, in one step.
    w.click(Role::Button, "Transom");
    assert_eq!(w.app.active_layer(), 2);
    assert_eq!(w.app.undo_len(), undo + 4);
    w.click(Role::Button, "Delete");
    assert_eq!(w.names(), ["Bottom", "Topsides"]);
    let after = w.app.surface().unwrap().layer_face_counts();
    assert_eq!(after, vec![counts[0], counts[1] + counts[2]]);
    assert_eq!(w.app.active_layer(), 1, "the receiving layer is active");
    assert_eq!(w.app.undo_len(), undo + 5);

    // Undo brings the transom back with its face.
    w.app.undo();
    w.frame(vec![]);
    assert_eq!(w.names(), ["Bottom", "Topsides", "Transom"]);
    assert_eq!(w.app.surface().unwrap().layer_face_counts(), counts[..3]);
}

#[test]
fn property_switches_and_the_eye_reach_the_model() {
    let mut w = Window::demo();
    w.click(Role::Button, "Topsides");
    assert_eq!(w.app.active_layer(), 1);
    let undo = w.app.undo_len();
    let bottom = w.app.surface().unwrap().layers[0].clone();
    let transom = w.app.surface().unwrap().layers[2].clone();
    for (label, read) in [
        (
            "Hydrostatics",
            (|l: &seacraft_core::mesh::Layer| l.use_in_hydrostatics) as fn(&_) -> bool,
        ),
        ("Symmetric", |l| l.symmetric),
        ("Developable", |l| l.developable),
        ("Intersections", |l| l.use_for_intersections),
        ("Lines plan", |l| l.show_in_linesplan),
    ] {
        let before = read(&w.app.surface().unwrap().layers[1]);
        w.click(Role::CheckBox, label);
        assert_ne!(
            read(&w.app.surface().unwrap().layers[1]),
            before,
            "{label} toggles the active layer"
        );
    }
    assert_eq!(w.app.undo_len(), undo + 5, "one step per switch");
    // The other layers keep theirs.
    assert_eq!(w.app.surface().unwrap().layers[0], bottom);
    assert_eq!(w.app.surface().unwrap().layers[2], transom);

    // The eye beside the transom hides it.
    let mut eyes = w.all(Role::Button, "👁");
    eyes.sort_by(|a, b| a.top().total_cmp(&b.top()));
    assert_eq!(eyes.len(), 3, "one eye per layer");
    w.click_at(eyes[2].center());
    assert!(!w.app.surface().unwrap().layers[2].visible);
    assert!(w.app.surface().unwrap().layers[0].visible);
    assert_eq!(w.app.undo_len(), undo + 6);
}

#[test]
fn a_click_picks_a_face_and_the_button_moves_it() {
    let mut w = Window::demo();
    w.app.recompute_hydrostatics();
    let before = format!("{:?}", w.app.hydrostatics().unwrap());

    // The button waits for a pick.
    assert_eq!(w.app.picked_face(), None);
    w.click(Role::Button, "Topsides");
    w.click(Role::Button, "Move picked face to this layer");
    assert_eq!(w.app.surface().unwrap().layer_face_counts()[1], 32);

    // Click the middle of a bottom face in the profile view.
    let s = w.app.surface().unwrap().clone();
    let face = (0..s.faces.len())
        .find(|&f| {
            s.face_layer[f] == 0 && {
                let c = centroid(&s, f);
                c.y > 0.0 && (c.x - 6.0).abs() < 1.5
            }
        })
        .expect("a starboard bottom face amidships");
    let profile = w
        .app
        .cameras()
        .iter()
        .position(|c| c.kind == ViewKind::Profile)
        .expect("a profile view");
    let rect = w.app.viewport_rect(profile);
    let view = w.app.cameras()[profile].transform();
    let at = view.project(centroid(&s, face), rect).expect("in view");
    w.click_at(at);
    let picked = w.app.picked_face().expect("a face under the click");
    assert_eq!(s.face_layer[picked], 0, "a bottom face (or its twin)");
    let c = centroid(&s, picked);
    assert!((c.x - centroid(&s, face).x).abs() < 1e-9);

    let undo = w.app.undo_len();
    w.click(Role::Button, "Move picked face to this layer");
    let after = w.app.surface().unwrap();
    assert_eq!(after.face_layer[picked], 1);
    assert_eq!(after.layer_face_counts()[1], 33);
    assert_eq!(w.app.undo_len(), undo + 1);
    // Every layer is symmetric-alike and in the hydrostatics: no number moves.
    w.app.recompute_hydrostatics();
    assert_eq!(format!("{:?}", w.app.hydrostatics().unwrap()), before);
}

#[test]
fn the_name_field_renames_on_enter() {
    let mut w = Window::demo();
    // The name field shows the active layer's name.
    let field = w
        .widgets
        .iter()
        .find(|(r, _, _)| *r == Role::TextInput)
        .map(|(_, _, rect)| *rect)
        .expect("a name field");
    w.click_at(field.center());
    let select_all = egui::Event::Key {
        key: egui::Key::A,
        physical_key: None,
        pressed: true,
        repeat: false,
        modifiers: egui::Modifiers::COMMAND,
    };
    w.frame(vec![
        select_all,
        egui::Event::Text("Underwater body".into()),
    ]);
    assert_eq!(w.names()[0], "Bottom", "nothing changes while typing");
    let undo = w.app.undo_len();
    let enter = |pressed| egui::Event::Key {
        key: egui::Key::Enter,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers: egui::Modifiers::NONE,
    };
    w.frame(vec![enter(true), enter(false)]);
    w.frame(vec![]);
    assert_eq!(w.names()[0], "Underwater body");
    assert_eq!(w.app.undo_len(), undo + 1);
    w.find(Role::Button, "Underwater body");
}

fn centroid(s: &seacraft_core::Surface, f: usize) -> Vec3 {
    let n = s.faces[f].len() as f64;
    s.faces[f]
        .iter()
        .fold(Vec3::ZERO, |a, &p| a + s.points[p].p)
        * (1.0 / n)
}
