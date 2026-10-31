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

#[test]
fn perspective_view_framing_fills_and_centres_bbox() {
    // Check both demo hull and long slender container ship
    for model_path in [
        None,
        Some(format!(
            "{}/../../examples/container-230m.fef",
            env!("CARGO_MANIFEST_DIR")
        )),
    ] {
        let mut app = App::headless_for_test(model_path.as_deref());
        let ctx = egui::Context::default();
        frame(&ctx, &mut app);
        frame(&ctx, &mut app);
        app.refresh_cache();

        let verts = app.render_cache().expect("refined hull").verts.clone();
        let cell = app.viewport_rect(0);
        let view = app.cameras()[0].transform();
        let mut bbox = egui::Rect::NOTHING;
        for v in &verts {
            if let Some(p) = view.project(*v, cell) {
                bbox.extend_with(p);
            }
        }

        assert!(
            cell.expand(1.0).contains_rect(bbox),
            "perspective hull bbox {bbox:?} leaves viewport {cell:?}"
        );

        let smaller_dim = cell.width().min(cell.height());
        let max_dim = bbox.width().max(bbox.height());
        let fill_frac = max_dim / smaller_dim;
        assert!(
            fill_frac >= 0.60,
            "perspective hull bbox fills {fill_frac:.2} of smaller viewport dim (expected >= 0.60)"
        );

        let off = bbox.center() - cell.center();
        assert!(
            off.x.abs() <= 0.10 * cell.width() && off.y.abs() <= 0.10 * cell.height(),
            "perspective hull bbox centred at {:?}, cell at {:?}, offset {:?}",
            bbox.center(),
            cell.center(),
            off
        );
    }
}

#[test]
fn profile_view_framing_includes_windage_profile_mast() {
    let yacht_path = format!(
        "{}/../../examples/yacht-10m.fef",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut app = App::headless_for_test(Some(&yacht_path));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);

    // Mast tip at z = 16.36 m
    let mast_tip = seacraft_core::math::Vec3::new(5.64, 0.0, 16.36);
    let cell_profile = app.viewport_rect(1);
    let view_profile = app.cameras()[1].transform();

    let p = view_profile
        .project(mast_tip, cell_profile)
        .expect("mast tip projects in Profile view");
    assert!(
        cell_profile.contains(p),
        "mast tip at {p:?} should be visible inside Profile viewport {cell_profile:?}"
    );

    // When windage display is toggled off, refit only frames canoe body (z <= 3.5 m)
    app.display_mut().windage = false;
    app.reset_cameras();
    frame(&ctx, &mut app);
    let view_profile_nowindage = app.cameras()[1].transform();
    let p_nowindage = view_profile_nowindage
        .project(mast_tip, cell_profile)
        .expect("project");
    assert!(
        p_nowindage.y < cell_profile.top(),
        "without windage framing, mast tip {p_nowindage:?} shoots off top of viewport ({cell_profile:?})"
    );
}

#[test]
fn compartment_with_clip_to_hull_renders_clipped_mesh_and_dashed_edges() {
    let container_path = format!(
        "{}/../../examples/container-230m.fef",
        env!("CARGO_MANIFEST_DIR")
    );
    let mut app = App::headless_for_test(Some(&container_path));
    let ctx = egui::Context::default();
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);

    let overlays = app.volume_overlays();
    let hold1 = overlays
        .iter()
        .find(|(_, o)| o.name == "No. 1 hold")
        .map(|(_, o)| o)
        .expect("No. 1 hold compartment overlay found");

    assert!(
        hold1.clip_to_hull,
        "No. 1 hold must have clip_to_hull: true"
    );
    let (clipped_verts, clipped_tris) = hold1
        .clipped_mesh
        .as_ref()
        .expect("clipped mesh generated for No. 1 hold");
    assert!(!clipped_verts.is_empty(), "clipped mesh has vertices");
    assert!(!clipped_tris.is_empty(), "clipped mesh has triangles");

    // The unclipped raw box reaches y = +/- 16.1 m.
    // In the forward flared bow at x in [190, 213], hull half-beam is clipped
    // inside the box (at the fwd boundary x >= 210 m, half-beam is < 10.0 m,
    // and overall max_y is 14.82 m < 16.1 m).
    let max_y = clipped_verts
        .iter()
        .map(|v| v.y.abs())
        .fold(0.0f64, f64::max);
    assert!(
        max_y < 15.5,
        "clipped compartment solid half-beam is {max_y:.2} m (must be clipped inside 16.1 m box)"
    );

    let fwd_max_y = clipped_verts
        .iter()
        .filter(|v| v.x >= 210.0)
        .map(|v| v.y.abs())
        .fold(0.0f64, f64::max);
    assert!(
        fwd_max_y < 12.0,
        "at forward end of hold (x >= 210), clipped half-beam is {fwd_max_y:.2} m (well inside 16.1 m box)"
    );
}
