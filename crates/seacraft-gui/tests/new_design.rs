//! Starting a design, headless: the start card with no model, the New design
//! dialog (Ctrl+N, Create as one undo step, Cancel changing nothing, Create
//! disabled while invalid) and the design requirements it leaves behind (the
//! sidecar, the pinned strip's targets, a new loading condition's lightship,
//! the Performance speed range).

use eframe::egui;
use seacraft_core::fef;
use seacraft_core::parametric::HullKind;
use seacraft_core::project::{self, Project};
use seacraft_gui::new_design::{speed_range, StartFrom};
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

fn key(k: egui::Key, modifiers: egui::Modifiers) -> Vec<egui::Event> {
    let ev = |pressed| egui::Event::Key {
        key: k,
        physical_key: None,
        pressed,
        repeat: false,
        modifiers,
    };
    vec![ev(true), ev(false)]
}

fn ctrl_n() -> Vec<egui::Event> {
    key(
        egui::Key::N,
        egui::Modifiers::CTRL | egui::Modifiers::COMMAND,
    )
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-new-design-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// Every text the frame painted, with where it was painted.
fn texts(out: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, acc: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Text(t) => {
                acc.push((t.galley.text().to_owned(), shape.visual_bounding_rect()))
            }
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, acc)),
            _ => {}
        }
    }
    let mut acc = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut acc);
    }
    acc
}

/// An empty app (a launch without `--open`), drawn until it settles.
fn empty() -> (App, egui::Context) {
    let mut app = App::headless_empty();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    (app, ctx)
}

/// Open the dialog with Ctrl+N; the preview generates on this thread.
fn open_dialog(ctx: &egui::Context, app: &mut App) {
    frame(ctx, app, ctrl_n());
    assert!(app.new_design_open(), "Ctrl+N opens the New design dialog");
    app.new_design_mut().unwrap().inline_preview = true;
    frame(ctx, app, Vec::new());
    frame(ctx, app, Vec::new());
}

fn click_create(ctx: &egui::Context, app: &mut App) {
    // The modal re-centres a frame after its size changes.
    frame(ctx, app, Vec::new());
    frame(ctx, app, Vec::new());
    let rect = app.new_design_mut().expect("dialog open").rects.create;
    assert!(rect.is_positive(), "Create was drawn: {rect:?}");
    click(ctx, app, rect.center());
}

/// A planing hull with a Cb, a speed and a KG target, created through the
/// dialog.
fn created() -> (App, egui::Context) {
    let (mut app, ctx) = empty();
    open_dialog(&ctx, &mut app);
    {
        let d = app.new_design_mut().unwrap();
        d.set_kind(HullKind::Planing);
        d.lwl = 11.0;
        d.cb = Some(0.45);
        d.speed_kn = Some(20.0);
        d.kg = Some(1.1);
    }
    frame(&ctx, &mut app, Vec::new());
    click_create(&ctx, &mut app);
    assert!(!app.new_design_open(), "Create closes the dialog");
    (app, ctx)
}

#[test]
fn with_no_model_the_start_card_is_shown() {
    let (mut app, ctx) = empty();
    let rects = app.start_rects().expect("the start card is drawn");
    for r in [rects.new_design, rects.open, rects.demo] {
        assert!(r.is_positive(), "{r:?}");
    }
    assert!(app.surface().is_none());
    let out = frame(&ctx, &mut app, Vec::new());
    let painted: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
    for want in ["New design…", "Open…", "Open demo hull"] {
        assert!(painted.iter().any(|t| t == want), "{want} in {painted:?}");
    }
    // Its first button opens the dialog.
    click(&ctx, &mut app, rects.new_design.center());
    assert!(app.new_design_open());

    // `--open FILE` skips the card.
    let mut opened = App::headless_for_test(Some(&demo_path()));
    frame(&ctx, &mut opened, Vec::new());
    assert!(opened.start_rects().is_none());
}

#[test]
fn ctrl_n_asks_about_unsaved_work_first() {
    let mut app = App::headless_for_test(None); // the demo is unsaved work
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, ctrl_n());
    assert!(app.unsaved_prompt_open());
    assert!(!app.new_design_open());
}

#[test]
fn create_builds_the_requested_hull_as_one_undo_step() {
    let (mut app, ctx) = created();
    assert!(app.face_count() > 0);
    assert_eq!(app.undo_len(), 1, "one undo step");
    assert!(app.is_dirty());
    let s = app.surface().unwrap();
    assert_eq!(s.settings.name, "Planing hull 11 m");

    let req = app.project().requirements.clone().expect("requirements");
    assert_eq!(req.lwl, Some(11.0));
    assert_eq!(req.cb, Some(0.45));
    assert_eq!(req.design_speed_kn, Some(20.0));
    assert_eq!(req.kg, Some(1.1));
    let d = req.displacement_t.unwrap();
    assert!((d - 0.45 * 1.025 * 11.0 * 3.2 * 0.6).abs() < 1e-9, "{d}");

    // The design waterline is T, and the hull measures as asked there.
    assert_eq!(app.hydro_options().draft, 0.6);
    assert_eq!(app.kg(), 1.1);
    app.recompute_hydrostatics();
    let h = app.hydrostatics().expect("hydrostatics");
    assert!((h.lwl - 11.0).abs() / 11.0 < 0.01, "LWL {}", h.lwl);
    assert!((h.bwl - 3.2).abs() / 3.2 < 0.01, "BWL {}", h.bwl);

    // The Performance tab centres on the design speed.
    assert_eq!(app.performance_state().speeds_kn, speed_range(20.0));
    assert_eq!(app.performance_state().speeds_kn, (10.0, 30.0, 2.0));

    // Undo goes back to the start screen; redo brings the design back.
    app.undo();
    assert!(app.surface().is_none());
    assert!(app.project().requirements.is_none());
    assert!(!app.is_dirty());
    frame(&ctx, &mut app, Vec::new());
    assert!(app.start_rects().is_some());
    app.redo();
    assert!(app.face_count() > 0);
    assert_eq!(app.project().requirements, Some(req));
}

#[test]
fn a_new_design_over_a_saved_model_undoes_back_to_it() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let before = fef::write(app.surface().unwrap());
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    open_dialog(&ctx, &mut app);
    click_create(&ctx, &mut app);
    assert_ne!(fef::write(app.surface().unwrap()), before);
    assert!(app.path().is_none(), "a new design is untitled");
    app.undo();
    assert_eq!(fef::write(app.surface().unwrap()), before);
    assert!(app.path().is_some(), "the file's path comes back");
    assert!(!app.is_dirty(), "and it is the saved state again");
}

#[test]
fn cancel_changes_nothing_and_leaves_no_undo_step() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    let before = fef::write(app.surface().unwrap());
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    open_dialog(&ctx, &mut app);
    app.new_design_mut().unwrap().lwl = 20.0;
    frame(&ctx, &mut app, Vec::new());
    let cancel = app.new_design_mut().unwrap().rects.cancel;
    click(&ctx, &mut app, cancel.center());
    assert!(!app.new_design_open());
    assert_eq!(fef::write(app.surface().unwrap()), before);
    assert_eq!(app.undo_len(), 0);
    assert!(!app.is_dirty());
    assert!(app.project().requirements.is_none());

    // Escape cancels too.
    open_dialog(&ctx, &mut app);
    frame(
        &ctx,
        &mut app,
        key(egui::Key::Escape, egui::Modifiers::NONE),
    );
    assert!(!app.new_design_open());
    assert_eq!(fef::write(app.surface().unwrap()), before);
}

#[test]
fn an_invalid_cb_disables_create() {
    let (mut app, ctx) = empty();
    open_dialog(&ctx, &mut app);
    app.new_design_mut().unwrap().cb = Some(0.95);
    let out = frame(&ctx, &mut app, Vec::new());
    let d = app.new_design_mut().unwrap();
    assert!(!d.can_create());
    let problem = d.problem().unwrap();
    assert!(problem.contains("cb"), "{problem}");
    assert!(
        texts(&out).iter().any(|(t, _)| t.contains(&problem)),
        "the error is shown inline"
    );
    click_create(&ctx, &mut app);
    assert!(app.new_design_open(), "a disabled Create does nothing");
    assert!(app.surface().is_none());
}

#[test]
fn the_demo_and_import_choices() {
    let (mut app, ctx) = empty();
    open_dialog(&ctx, &mut app);
    {
        let d = app.new_design_mut().unwrap();
        d.start = StartFrom::Demo;
        d.half_model = true;
    }
    frame(&ctx, &mut app, Vec::new());
    click_create(&ctx, &mut app);
    let s = app.surface().expect("demo loaded");
    assert!(s.layers.iter().any(|l| l.symmetric), "the half model");
    assert_eq!(app.undo_len(), 1);
    assert!(app.project().requirements.is_none());

    // Import hands over to the Open dialog.
    let mut app = App::headless_empty();
    open_dialog(&ctx, &mut app);
    app.new_design_mut().unwrap().start = StartFrom::Import;
    frame(&ctx, &mut app, Vec::new());
    click_create(&ctx, &mut app);
    assert!(app.dialog_open(), "the Open dialog");
    assert!(app.surface().is_none());
}

#[test]
fn requirements_travel_in_the_sidecar() {
    let (mut app, _ctx) = created();
    let path = scratch("designed.fef");
    let sidecar = project::sidecar_path(&path);
    let _ = std::fs::remove_file(&sidecar);
    assert!(app.save_to(&path), "{}", app.status());
    assert!(sidecar.exists(), "requirements alone write the sidecar");
    let want = app.project().requirements.clone();

    let back = App::headless_for_test(Some(path.to_str().unwrap()));
    assert_eq!(back.project().requirements, want);
    // Opening it centres the speed range again.
    assert_eq!(back.performance_state().speeds_kn, speed_range(20.0));
    assert!(!back.is_dirty());

    // An old sidecar without requirements still loads.
    let old = scratch("old.fef");
    std::fs::copy(demo_path(), &old).unwrap();
    std::fs::write(
        project::sidecar_path(&old),
        r#"{ "format": "seacraft-project", "version": 1,
             "windage": [ { "name": "Cabin", "area_m2": 2.0, "z_centroid": 1.5 } ] }"#,
    )
    .unwrap();
    let app = App::headless_for_test(Some(old.to_str().unwrap()));
    assert_eq!(app.project().windage.len(), 1, "{}", app.status());
    assert_eq!(app.project().requirements, None);
    assert_eq!(
        Project::load_for_model(&old).unwrap().unwrap().requirements,
        None
    );
}

#[test]
fn the_pinned_strip_shows_actual_against_target() {
    let (mut app, ctx) = created();
    app.recompute_hydrostatics();
    let h = app.hydrostatics().unwrap().clone();
    let targets = app.target_readouts();
    let lwl = targets
        .iter()
        .find(|t| t.name == "LWL")
        .expect("LWL target");
    assert_eq!(lwl.target, 11.0);
    assert!(lwl.on_target(), "{}", lwl.text());
    let disp = targets.iter().find(|t| t.name == "Δ").expect("Δ target");
    assert_eq!(disp.actual, h.displacement_t);
    let text = disp.text();
    assert!(
        text.starts_with(&format!("Δ {:.2} t ({:.2} ", h.displacement_t, disp.target)),
        "{text}"
    );
    assert!(text.ends_with(" %)"), "{text}");

    // And the strip paints it.
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    let painted: Vec<String> = texts(&out).into_iter().map(|(t, _)| t).collect();
    assert!(
        painted.iter().any(|t| t == &lwl.text()),
        "{} not in {painted:?}",
        lwl.text()
    );

    // No requirements, no targets.
    let demo = App::headless_for_test(Some(&demo_path()));
    assert!(demo.target_readouts().is_empty());
}

#[test]
fn a_new_loading_condition_starts_from_the_measured_hull() {
    let (mut app, ctx) = created();
    app.recompute_hydrostatics();
    let h = app.hydrostatics().unwrap().clone();
    let keel = app.surface().unwrap().lowest_hydrostatics_point();
    app.set_dock_tab(seacraft_gui::app::DesignDock::Loading);
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    let (_, new) = texts(&out)
        .into_iter()
        .find(|(t, _)| t == "New")
        .expect("the loading tab's New button");
    click(&ctx, &mut app, new.center());
    let conds = &app.project().loading_conditions;
    assert_eq!(conds.len(), 1);
    let ls = &conds[0].lightship;
    assert_eq!(ls.mass_t, h.displacement_t, "mass = Δ");
    assert_eq!(ls.x, h.lcb.x);
    assert!(
        (ls.z - (keel + 1.1)).abs() < 1e-12,
        "KG estimate above the keel"
    );
}

#[test]
fn project_settings_edit_the_requirements() {
    let (mut app, ctx) = created();
    let undo_before = app.undo_len();
    let mut req = app.project().requirements.clone().unwrap();
    req.design_speed_kn = Some(8.0);
    app.apply_project_settings("Launch".into(), "A. Designer".into(), 1000.0, Some(req));
    let s = app.surface().unwrap();
    assert_eq!(s.settings.name, "Launch");
    assert_eq!(s.settings.designer, "A. Designer");
    assert_eq!(app.hydro_options().water_density, 1000.0);
    assert_eq!(
        app.undo_len(),
        undo_before + 1,
        "the .fef settings are one step"
    );
    assert_eq!(app.performance_state().speeds_kn, (4.0, 12.0, 1.0));

    // The dialog opens on the current values and applies through its button.
    frame(&ctx, &mut app, Vec::new());
    app.open_project_settings();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let state = app.project_settings_mut().expect("dialog open");
    assert_eq!(state.name, "Launch");
    state.requirements.kg = Some(1.4);
    let apply = state.rects.create;
    click(&ctx, &mut app, apply.center());
    assert!(app.project_settings_mut().is_none());
    assert_eq!(app.project().requirements.as_ref().unwrap().kg, Some(1.4));
    app.undo();
    assert_eq!(app.surface().unwrap().settings.name, "Planing hull 11 m");
    assert_eq!(app.hydro_options().water_density, 1025.0);
}
