//! Starting from an example hull, headless: the start card's "Open an
//! example…", the New design dialog's *Example* choice with a preview built
//! at once, and *Create* loading the hull and its project as one undo step
//! with the example's KG in the dock.

use eframe::egui;
use seacraft_core::examples::Example;
use seacraft_core::project;
use seacraft_gui::new_design::{example_rows, StartFrom};
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

/// Every text the frame painted.
fn texts(out: &egui::FullOutput) -> Vec<String> {
    fn walk(shape: &egui::Shape, acc: &mut Vec<String>) {
        match shape {
            egui::Shape::Text(t) => acc.push(t.galley.text().to_owned()),
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

fn scratch(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-examples-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    dir.join(name)
}

/// An empty app, its start card clicked on "Open an example…".
fn dialog_on_example() -> (App, egui::Context) {
    let mut app = App::headless_empty();
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let rect = app.start_rects().expect("the start card").example;
    assert!(rect.is_positive(), "Open an example… was drawn: {rect:?}");
    click(&ctx, &mut app, rect.center());
    assert!(app.new_design_open(), "the New design dialog opened");
    (app, ctx)
}

/// The dialog on the runabout, whatever the first example is.
fn dialog_on_runabout() -> (App, egui::Context) {
    let (mut app, ctx) = dialog_on_example();
    app.new_design_mut()
        .unwrap()
        .set_example(Example::Runabout6m);
    (app, ctx)
}

fn click_create(ctx: &egui::Context, app: &mut App) {
    frame(ctx, app, Vec::new());
    frame(ctx, app, Vec::new());
    let rect = app.new_design_mut().expect("dialog open").rects.create;
    assert!(rect.is_positive(), "Create was drawn: {rect:?}");
    click(ctx, app, rect.center());
}

#[test]
fn the_start_card_opens_the_dialog_on_an_example_with_its_preview() {
    let (mut app, ctx) = dialog_on_example();
    let d = app.new_design_mut().unwrap();
    assert_eq!(d.start, StartFrom::Example);
    let first = Example::all()[0];
    assert_eq!(d.example, first);
    // Built at once: no worker, no debounce.
    let built = d.example_preview().expect("the preview is built at once");
    assert_eq!(built.kind, first);
    assert!(!built.surface.faces.is_empty());
    assert!(d.can_create());
    let rows = example_rows(&built.project);
    assert!(rows.iter().any(|r| r.name == "KG"), "{rows:?}");

    // The dialog paints the example's title, description and particulars.
    let out = frame(&ctx, &mut app, Vec::new());
    let painted = texts(&out);
    for want in [first.title(), first.description()] {
        assert!(painted.iter().any(|t| t == want), "{want} in {painted:?}");
    }
    assert!(painted.iter().any(|t| t == "Example"), "{painted:?}");
    let kg = rows.iter().find(|r| r.name == "KG").unwrap();
    assert!(
        painted.contains(&kg.value),
        "the example's KG row: {painted:?}"
    );
}

#[test]
fn the_picker_offers_the_gui_examples_only() {
    let all = Example::all();
    assert!(all.contains(&Example::Runabout6m));
    assert!(
        !all.contains(&Example::Wigley),
        "the benchmark stays in the CLI"
    );
    assert!(all.iter().all(|e| e.is_available()));

    let (mut app, _ctx) = dialog_on_example();
    let d = app.new_design_mut().unwrap();
    for e in all {
        d.set_example(e);
        let built = d.example_preview().expect("built");
        assert_eq!(built.kind, e);
        assert_eq!(built.surface.settings.name, e.title());
    }
}

#[test]
fn create_loads_the_example_and_its_project_as_one_undo_step() {
    let (mut app, ctx) = dialog_on_runabout();
    click_create(&ctx, &mut app);
    assert!(!app.new_design_open(), "Create closes the dialog");
    let (want_surface, want_project) = seacraft_core::examples::build(Example::Runabout6m).unwrap();

    let s = app.surface().expect("a model");
    assert_eq!(
        seacraft_core::fef::write(s),
        seacraft_core::fef::write(&want_surface)
    );
    assert_eq!(app.project(), &want_project);
    assert_eq!(app.undo_len(), 1, "one undo step");
    assert!(app.is_dirty());
    assert!(app.path().is_none(), "an example opens untitled");
    assert!(app.status().contains("runabout-6m"), "{}", app.status());

    // The dock KG is the project's, the user's own (not an estimate), and
    // the design waterline is the example's draft.
    assert_eq!(app.kg(), Some(0.65));
    assert!(!app.kg_is_estimate());
    assert_eq!(app.hydro_options().draft, 0.38);
    app.recompute_hydrostatics();
    let h = app.hydrostatics().expect("hydrostatics").clone();
    assert!(
        (h.displacement_t - 1.922).abs() < 2e-3,
        "Δ {}",
        h.displacement_t
    );
    // The Performance tab centres on the design speed (30 kn).
    assert_eq!(
        app.performance_state().speeds_kn,
        seacraft_gui::new_design::speed_range(30.0)
    );
    // Its GZ curves run without asking for a KG.
    app.compute_cross_curves();
    assert!(app.cross_curves().is_some(), "{}", app.status());

    // Undo goes back to the start screen, redo brings the example back.
    app.undo();
    assert!(app.surface().is_none());
    assert!(app.project().is_empty());
    frame(&ctx, &mut app, Vec::new());
    assert!(app.start_rects().is_some());
    app.redo();
    assert_eq!(app.project(), &want_project);
    assert!(app.surface().is_some());
}

#[test]
fn an_example_saves_with_its_sidecar_and_reopens_with_its_kg() {
    let (mut app, ctx) = dialog_on_runabout();
    click_create(&ctx, &mut app);
    let path = scratch("runabout.fef");
    let _ = std::fs::remove_file(project::sidecar_path(&path));
    assert!(app.save_to(&path), "{}", app.status());
    assert!(project::sidecar_path(&path).exists());
    let back = App::headless_for_test(Some(path.to_str().unwrap()));
    assert_eq!(back.kg(), Some(0.65));
    assert!(!back.kg_is_estimate());
    assert_eq!(
        back.project()
            .loading_condition(Some("Design"))
            .unwrap()
            .name,
        "Design"
    );
    assert!(!back.is_dirty());
}

#[test]
fn cancel_on_the_example_changes_nothing() {
    let (mut app, ctx) = dialog_on_example();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
    let cancel = app.new_design_mut().unwrap().rects.cancel;
    click(&ctx, &mut app, cancel.center());
    assert!(!app.new_design_open());
    assert!(app.surface().is_none());
    assert_eq!(app.undo_len(), 0);
}
