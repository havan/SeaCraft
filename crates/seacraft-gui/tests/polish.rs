//! Round-5 polish (R5-5), headless at 1600 × 1000: the Inspector under the
//! layer list, the cross curves above the fold, half models framed whole.

use eframe::egui;
use seacraft_gui::app::DesignDock;
use seacraft_gui::panels;
use seacraft_gui::App;

fn frame(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    // Nothing uploads the fonts in a headless pass; a dropped delta panics.
    out.textures_delta.clear();
    out
}

fn texts(out: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push((
                t.galley.text().to_string(),
                t.galley.rect.translate(t.pos.to_vec2()),
            )),
            _ => {}
        }
    }
    let mut text = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut text);
    }
    text
}

#[test]
fn the_inspector_sits_directly_under_the_layer_list() {
    let mut app = App::headless_for_test(None);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    let out = frame(&ctx, &mut app);
    let layers = app.layer_list_rect();
    let inspector = app.inspector_rect();
    assert!(layers.height() > 20.0, "layer list {layers:?}");
    assert!(inspector.height() > 20.0, "inspector {inspector:?}");
    let gap = inspector.top() - layers.bottom();
    assert!(
        (0.0..=24.0).contains(&gap),
        "the Inspector starts {gap} px below the layers ({layers:?}, {inspector:?})"
    );
    // Not pinned to the foot of the window any more.
    assert!(inspector.top() < 400.0, "{inspector:?}");
    // A sentence-case heading, as the rest of the app.
    let text = texts(&out);
    let heading = text
        .iter()
        .find(|(t, _)| t == "Inspector")
        .expect("an Inspector heading");
    assert!(inspector.contains(heading.1.center()));
    assert!(!text.iter().any(|(t, _)| t == "INSPECTOR"));
}

#[test]
fn the_cross_curves_stay_above_the_fold_at_1600_by_1000() {
    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(2);
    app.set_dock_tab(DesignDock::Hydrostatics);
    app.recompute_hydrostatics();
    app.compute_cross_curves();
    assert!(app.cross_curves().is_some(), "GZ curves computed");
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    let out = frame(&ctx, &mut app);
    let visible_bottom = app.central_rect().bottom();
    let sac = panels::last_plot_rect(&ctx, "sac").expect("the SAC plot was drawn");
    let curves = panels::last_plot_rect(&ctx, "cross_curves").expect("the GZ plot was drawn");
    assert!(sac.bottom() <= curves.top() + 1.0, "{sac:?} / {curves:?}");
    assert!(
        curves.bottom() <= visible_bottom,
        "the cross-curve plot ends at {} below the dock's visible {}",
        curves.bottom(),
        visible_bottom
    );
    // It is the two-column readout that makes the room: KM(t) heads the
    // second column, level with volume.
    let text = texts(&out);
    let at = |name: &str| {
        text.iter()
            .find(|(t, _)| t == name)
            .unwrap_or_else(|| panic!("{name} not drawn"))
            .1
    };
    let (volume, km) = (at("volume"), at("KM(t)"));
    assert!(km.left() > volume.right(), "{volume:?} / {km:?}");
    assert!(
        (km.top() - volume.top()).abs() < 30.0,
        "{volume:?} / {km:?}"
    );
    assert!(at("LWL").top() > 0.0 && at("BWL").top() > at("LWL").top());
}

#[test]
fn a_half_model_is_framed_whole_and_centred() {
    let mut app = App::headless_for_test(None);
    app.load_demo_shape(true);
    assert!(
        app.surface().unwrap().layers.iter().any(|l| l.symmetric),
        "a half model"
    );
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    app.refresh_cache();
    let verts = app.render_cache().expect("a refined hull").verts.clone();
    assert!(
        verts.iter().any(|v| v.y < -0.5),
        "the mesh is shown mirrored"
    );
    for (i, camera) in app.cameras().iter().enumerate() {
        let cell = app.viewport_rect(i);
        let view = camera.transform();
        let mut bbox = egui::Rect::NOTHING;
        for v in &verts {
            if let Some(p) = view.project(*v, cell) {
                bbox.extend_with(p);
            }
        }
        assert!(
            cell.expand(1.0).contains_rect(bbox),
            "{:?}: the hull {bbox:?} leaves the cell {cell:?}",
            camera.kind
        );
        let off = bbox.center() - cell.center();
        assert!(
            off.x.abs() <= 0.1 * cell.width() && off.y.abs() <= 0.1 * cell.height(),
            "{:?}: hull centred at {:?}, cell at {:?}",
            camera.kind,
            bbox.center(),
            cell.center()
        );
    }
}
