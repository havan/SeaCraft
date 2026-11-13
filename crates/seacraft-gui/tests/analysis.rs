//! The Analysis menu's views, headless: each menu item opens its view in
//! the wide dock, a computed table matches the command behind it, Copy CSV
//! puts the table on the clipboard, and a geometry change makes the result
//! stale until F5.

use eframe::egui;
use seacraft_core::math::Vec3;
use seacraft_gui::analysis_views::{AnalysisView, PlateLayers, ViewResult, NO_VALUE};
use seacraft_gui::commands;
use seacraft_gui::panels::EngCell;
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

/// Every text drawn, with where.
fn texts(out: &egui::FullOutput) -> Vec<(String, egui::Rect)> {
    fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Rect)>) {
        match shape {
            egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
            egui::Shape::Text(t) => out.push((
                t.galley.text().to_string(),
                egui::Rect::from_min_size(t.pos, t.galley.size()),
            )),
            _ => {}
        }
    }
    let mut found = Vec::new();
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut found);
    }
    found
}

fn find(out: &egui::FullOutput, text: &str) -> Option<egui::Rect> {
    texts(out)
        .into_iter()
        .find(|(t, _)| t == text)
        .map(|(_, r)| r)
}

/// Click at `pos` and return the frame after the release.
fn click(ctx: &egui::Context, app: &mut App, pos: egui::Pos2) -> egui::FullOutput {
    frame(ctx, app, vec![egui::Event::PointerMoved(pos)]);
    frame(ctx, app, vec![button(pos, true)]);
    frame(ctx, app, vec![button(pos, false)])
}

/// Click the text `label` drawn in the last frame `out`.
fn click_text(
    ctx: &egui::Context,
    app: &mut App,
    out: &egui::FullOutput,
    label: &str,
) -> egui::FullOutput {
    let rect = find(out, label).unwrap_or_else(|| {
        let all: Vec<String> = texts(out).into_iter().map(|(t, _)| t).collect();
        panic!("`{label}` was not drawn; drawn: {all:?}")
    });
    click(ctx, app, rect.center())
}

/// The demo app with its docks settled.
fn settled() -> (egui::Context, App, egui::FullOutput) {
    let mut app = App::headless_for_test(None);
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    (ctx, app, out)
}

/// Open a view through the menu bar: Analysis, then its item.
fn open_from_menu(
    ctx: &egui::Context,
    app: &mut App,
    out: &egui::FullOutput,
    view: AnalysisView,
) -> egui::FullOutput {
    click_text(ctx, app, out, "Analysis");
    let out = frame(
        ctx,
        app,
        vec![egui::Event::PointerMoved(egui::pos2(1.0, 500.0))],
    );
    click_text(ctx, app, &out, view.label())
}

fn cell(row: &[EngCell], c: usize) -> &str {
    match &row[c] {
        EngCell::Text(t) => t,
        EngCell::Badge(..) => panic!("a badge in column {c}"),
    }
}

#[test]
fn each_analysis_menu_item_opens_its_view_in_the_wide_dock() {
    let (ctx, mut app, mut out) = settled();
    assert_eq!(app.analysis_view(), None);
    assert!(!app.dock_wide());
    for view in AnalysisView::ALL {
        open_from_menu(&ctx, &mut app, &out, view);
        assert_eq!(app.analysis_view(), Some(view), "{}", view.label());
        assert!(app.dock_wide(), "a view opens in the wide dock");
        assert!(app.show_analysis());
        // Opening a view with no result computes it.
        assert!(
            app.analysis_view_busy(view) || app.analysis_view_result(view).is_some(),
            "{} did not start computing",
            view.label()
        );
        app.wait_analysis_view(view);
        assert!(
            app.analysis_view_result(view).is_some(),
            "{}: {}",
            view.label(),
            app.status()
        );
        frame(&ctx, &mut app, Vec::new());
        out = frame(&ctx, &mut app, Vec::new());
        assert!(
            find(&out, view.label()).is_some(),
            "the {} heading is drawn",
            view.label()
        );
        assert!(find(&out, "Copy CSV").is_some());
    }
    // A tab click goes back to the tabs.
    click_text(&ctx, &mut app, &out, "Loading");
    assert_eq!(app.analysis_view(), None);
    assert_eq!(
        app.design_dock(),
        Some(seacraft_gui::app::DesignDock::Loading)
    );
    // Reopening keeps the result (nothing to recompute); Close goes back.
    app.open_analysis_view(AnalysisView::Bonjean);
    assert!(!app.analysis_view_busy(AnalysisView::Bonjean));
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    click_text(&ctx, &mut app, &out, "Close");
    assert_eq!(app.analysis_view(), None);
    assert_eq!(
        app.design_dock(),
        Some(seacraft_gui::app::DesignDock::Loading)
    );
}

#[test]
fn the_hydrostatic_table_matches_the_command() {
    let mut app = App::headless_for_test(None);
    app.open_analysis_view(AnalysisView::HydroTable);
    app.wait_analysis_view(AnalysisView::HydroTable);
    let table = app
        .analysis_view_table(AnalysisView::HydroTable)
        .expect("a table");
    let expected = commands::hydro_table(&app, &app.analysis_inputs().table.clone()).unwrap();
    assert_eq!(table.rows.len(), expected.rows.len());
    assert_eq!(table.rows.len(), 10, "ten steps by default");
    assert_eq!(table.columns.len(), 18);
    assert_eq!(table.columns[2].title, "Δ");
    assert_eq!(table.columns[2].unit, "t");
    for (row, r) in table.rows.iter().zip(&expected.rows) {
        assert_eq!(cell(row, 0), format!("{:.3}", r.draft));
        assert_eq!(cell(row, 1), format!("{:.3}", r.volume_m3));
        assert_eq!(cell(row, 2), format!("{:.3}", r.displacement_t));
        assert_eq!(cell(row, 6), format!("{:.3}", r.km_transverse));
        assert_eq!(cell(row, 9), format!("{:.4}", r.block_coefficient));
    }
    // Up to the design draft: the last row is the dock's draft.
    let last = table.rows.last().unwrap();
    assert_eq!(cell(last, 0), format!("{:.3}", app.hydro_options().draft));
}

#[test]
fn bonjean_offsets_trim_and_plates_match_their_commands() {
    let mut app = App::headless_for_test(None);
    for view in [
        AnalysisView::Bonjean,
        AnalysisView::Offsets,
        AnalysisView::TrimScan,
        AnalysisView::Plates,
    ] {
        app.open_analysis_view(view);
        app.wait_analysis_view(view);
        assert!(
            app.analysis_view_failure(view).is_none(),
            "{}: {:?}",
            view.label(),
            app.analysis_view_failure(view)
        );
    }
    let inputs = app.analysis_inputs().clone();

    let bonjean = commands::bonjean(&app, &inputs.bonjean).unwrap();
    let t = app.analysis_view_table(AnalysisView::Bonjean).unwrap();
    assert_eq!(t.rows.len(), bonjean.sections.len());
    assert_eq!(t.columns.len(), 1 + bonjean.drafts_m.len());
    assert_eq!(t.columns[1].unit, "m²");
    for (row, s) in t.rows.iter().zip(&bonjean.sections) {
        assert_eq!(cell(row, 0), format!("{:.3}", s.x));
        for (c, a) in s.areas_m2.iter().enumerate() {
            assert_eq!(cell(row, c + 1), format!("{a:.3}"));
        }
    }

    let offsets = commands::offsets(&app, &inputs.offsets).unwrap();
    let t = app.analysis_view_table(AnalysisView::Offsets).unwrap();
    assert_eq!(t.rows.len(), offsets.stations.len());
    assert_eq!(
        t.columns.len(),
        4 + offsets.waterlines_m.len() + offsets.buttocks_m.len()
    );
    for (row, s) in t.rows.iter().zip(&offsets.stations) {
        assert_eq!(cell(row, 3), format!("{:.3}", s.half_beam));
        for (c, hb) in s.waterline_half_breadths.iter().enumerate() {
            let want = hb.map_or_else(|| NO_VALUE.to_owned(), |v| format!("{v:.3}"));
            assert_eq!(cell(row, 4 + c), want);
        }
    }

    let scan = commands::trim_scan(&app, &inputs.trim).unwrap();
    let t = app.analysis_view_table(AnalysisView::TrimScan).unwrap();
    assert_eq!(t.rows.len(), scan.points.len());
    for (row, p) in t.rows.iter().zip(&scan.points) {
        assert_eq!(cell(row, 0), format!("{:.3}", p.trim_m));
        assert_eq!(cell(row, 5), format!("{:.3}", p.rt_n / 1000.0));
    }

    // Plate development: one row per layer, each the layer developed alone.
    let surface = app.surface().unwrap().clone();
    let t = app.analysis_view_table(AnalysisView::Plates).unwrap();
    assert_eq!(t.rows.len(), surface.layers.len());
    for (i, row) in t.rows.iter().enumerate() {
        assert_eq!(cell(row, 0), surface.layers[i].name);
        let plate = commands::unfold(
            &app,
            &commands::UnfoldParams {
                layer: Some(i),
                ..Default::default()
            },
        )
        .unwrap();
        assert_eq!(cell(row, 2), plate.faces.to_string());
        assert_eq!(cell(row, 3), format!("{:.3}", plate.plate.area_3d_m2));
    }
    // The layers' areas add up to the whole hull's (starboard) area.
    let whole = commands::unfold(&app, &commands::UnfoldParams::default()).unwrap();
    let Some(ViewResult::Plates(rows)) = app.analysis_view_result(AnalysisView::Plates) else {
        panic!("plate rows");
    };
    let sum: f64 = rows
        .iter()
        .map(|r| r.result.as_ref().unwrap().plate.area_3d_m2)
        .sum();
    assert!(
        (sum - whole.plate.area_3d_m2).abs() < 1e-6 * whole.plate.area_3d_m2,
        "layers {sum} vs whole {}",
        whole.plate.area_3d_m2
    );

    // One layer only.
    app.edit_analysis_inputs(AnalysisView::Plates, |i| {
        i.plates.layers = PlateLayers::One(1)
    });
    app.start_analysis_view(AnalysisView::Plates);
    app.wait_analysis_view(AnalysisView::Plates);
    let t = app.analysis_view_table(AnalysisView::Plates).unwrap();
    assert_eq!(t.rows.len(), 1);
    assert_eq!(cell(&t.rows[0], 0), surface.layers[1].name);
    // No layer of the demo is marked developable: said so, not an empty table.
    app.edit_analysis_inputs(AnalysisView::Plates, |i| {
        i.plates.layers = PlateLayers::Marked
    });
    app.start_analysis_view(AnalysisView::Plates);
    app.wait_analysis_view(AnalysisView::Plates);
    assert!(app
        .analysis_view_failure(AnalysisView::Plates)
        .is_some_and(|e| e.contains("marked developable")));
}

#[test]
fn copy_csv_puts_the_table_on_the_clipboard() {
    let (ctx, mut app, _) = settled();
    app.open_analysis_view(AnalysisView::HydroTable);
    app.wait_analysis_view(AnalysisView::HydroTable);
    let csv = app.analysis_view_csv(AnalysisView::HydroTable).unwrap();
    let lines: Vec<&str> = csv.lines().collect();
    assert_eq!(lines.len(), 1 + 10, "a header and one line per draft");
    assert!(
        lines[0].starts_with("T (m),Volume (m³),Δ (t),KB (m)"),
        "{}",
        lines[0]
    );
    assert!(
        lines[0].ends_with("TPC (t/cm),MTC (t·m/cm),WSA (m²)"),
        "{}",
        lines[0]
    );
    assert!(lines.iter().all(|l| l.split(',').count() == 18), "{csv}");

    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    let out = click_text(&ctx, &mut app, &out, "Copy CSV");
    let copied: Vec<&String> = out
        .platform_output
        .commands
        .iter()
        .filter_map(|c| match c {
            egui::OutputCommand::CopyText(t) => Some(t),
            _ => None,
        })
        .collect();
    assert_eq!(copied, vec![&csv], "Copy CSV copies the table");
    assert!(app.status().contains("10 rows copied"), "{}", app.status());

    // The offsets' missing values are empty fields.
    app.open_analysis_view(AnalysisView::Offsets);
    app.wait_analysis_view(AnalysisView::Offsets);
    let csv = app.analysis_view_csv(AnalysisView::Offsets).unwrap();
    let stations = app.analysis_inputs().offsets.stations;
    assert_eq!(csv.lines().count(), 1 + stations);
    assert!(!csv.contains(NO_VALUE));
    assert!(csv.lines().next().unwrap().contains("WL "));
}

#[test]
fn a_geometry_change_makes_a_view_stale_until_f5() {
    let (ctx, mut app, _) = settled();
    app.open_analysis_view(AnalysisView::Offsets);
    app.wait_analysis_view(AnalysisView::Offsets);
    app.open_analysis_view(AnalysisView::HydroTable);
    app.wait_analysis_view(AnalysisView::HydroTable);
    assert!(!app.analysis_view_stale(AnalysisView::HydroTable));
    assert!(!app.stale_results().contains(&"analysis view"));

    // A draft change: the hydrostatic table is stale, the offsets are not
    // (they do not depend on the floating condition).
    app.hydro_options_mut().draft += 0.05;
    assert!(app.analysis_view_stale(AnalysisView::HydroTable));
    assert!(!app.analysis_view_stale(AnalysisView::Offsets));
    app.hydro_options_mut().draft -= 0.05;
    assert!(!app.analysis_view_stale(AnalysisView::HydroTable));

    // Move a point: every computed view is stale, the open one counts in
    // the status bar and is drawn with the stale chip.
    let p = app.surface().unwrap().points[10].p;
    app.move_point(10, p + Vec3::new(0.0, 0.0, 0.05)).unwrap();
    assert!(app.analysis_view_stale(AnalysisView::HydroTable));
    assert!(app.analysis_view_stale(AnalysisView::Offsets));
    assert!(app.stale_results().contains(&"analysis view"));
    let out = frame(&ctx, &mut app, Vec::new());
    assert!(find(&out, "out of date — F5").is_some());
    // Heavy results wait: no frame recomputes them by itself.
    assert!(!app.analysis_view_busy(AnalysisView::HydroTable));

    // F5 recomputes the open view (only).
    app.recompute_stale();
    assert!(app.analysis_view_busy(AnalysisView::HydroTable));
    assert!(!app.analysis_view_busy(AnalysisView::Offsets));
    app.wait_analysis_view(AnalysisView::HydroTable);
    assert!(!app.analysis_view_stale(AnalysisView::HydroTable));
    assert!(app.analysis_view_stale(AnalysisView::Offsets));
    assert!(!app.stale_results().contains(&"analysis view"));

    // Changing a view's inputs makes it stale too.
    app.edit_analysis_inputs(AnalysisView::HydroTable, |i| i.table.steps = 4);
    assert!(app.analysis_view_stale(AnalysisView::HydroTable));
    app.recompute_stale();
    app.wait_analysis_view(AnalysisView::HydroTable);
    assert_eq!(
        app.analysis_view_table(AnalysisView::HydroTable)
            .unwrap()
            .rows
            .len(),
        4
    );
}

#[test]
fn the_analysis_menu_needs_a_model() {
    let mut app = App::headless_empty();
    app.open_analysis_view(AnalysisView::HydroTable);
    assert!(!app.analysis_view_busy(AnalysisView::HydroTable));
    assert!(app.analysis_view_result(AnalysisView::HydroTable).is_none());
    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());
}

#[test]
fn a_view_gives_the_dock_back_at_the_width_it_had() {
    let (ctx, mut app, out) = settled();
    let narrow = app.viewport_rect(1);
    assert!(!app.dock_wide());

    // Opened from the narrow dock: Close and a tab both give the narrow dock back.
    open_from_menu(&ctx, &mut app, &out, AnalysisView::Offsets);
    assert!(app.dock_wide());
    let out = frame(&ctx, &mut app, Vec::new());
    click_text(&ctx, &mut app, &out, "Close");
    assert_eq!(app.analysis_view(), None);
    assert!(!app.dock_wide(), "Close gives the narrow dock back");
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    assert_eq!(app.viewport_rect(1), narrow);

    open_from_menu(&ctx, &mut app, &out, AnalysisView::Offsets);
    let out = frame(&ctx, &mut app, Vec::new());
    click_text(&ctx, &mut app, &out, "Hydro");
    assert_eq!(app.analysis_view(), None);
    assert!(!app.dock_wide(), "a tab gives the narrow dock back");

    // Widened by hand first: the dock stays wide.
    app.set_dock_wide(true);
    app.open_analysis_view(AnalysisView::Offsets);
    app.close_analysis_view();
    assert!(app.dock_wide(), "the hand-set width is kept");
}

/// The x of the resize handle on the right dock's left edge.
fn dock_edge(out: &egui::FullOutput) -> f32 {
    let update = out
        .platform_output
        .accesskit_update
        .as_ref()
        .expect("AccessKit on");
    update
        .nodes
        .iter()
        .filter(|(_, n)| {
            n.role() == egui::accesskit::Role::Splitter && n.label() == Some("Resize panel")
        })
        .filter_map(|(_, n)| n.bounds())
        .map(|b| ((b.x0 + b.x1) / 2.0) as f32)
        .fold(f32::NEG_INFINITY, f32::max)
}

#[test]
fn the_wide_dock_can_be_dragged_narrower_with_a_wide_table_in_it() {
    let (ctx, mut app, out) = settled();
    ctx.enable_accesskit();
    open_from_menu(&ctx, &mut app, &out, AnalysisView::Offsets);
    app.wait_analysis_view(AnalysisView::Offsets);
    frame(&ctx, &mut app, Vec::new());
    let out = frame(&ctx, &mut app, Vec::new());
    let edge = dock_edge(&out);
    assert!(
        (1600.0 - edge - 0.55 * 1600.0).abs() < 30.0,
        "the wide dock opens at 55 %: its edge is at {edge}"
    );

    let from = egui::pos2(edge, 600.0);
    let to = egui::pos2(edge + 300.0, 600.0);
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(from)]);
    frame(&ctx, &mut app, vec![button(from, true)]);
    for step in 1..=10 {
        let p = from.lerp(to, step as f32 / 10.0);
        frame(&ctx, &mut app, vec![egui::Event::PointerMoved(p)]);
    }
    frame(&ctx, &mut app, vec![button(to, false)]);
    for _ in 0..3 {
        frame(&ctx, &mut app, Vec::new());
    }
    let out = frame(&ctx, &mut app, Vec::new());
    let dragged = dock_edge(&out);
    assert!(
        (dragged - (edge + 300.0)).abs() < 10.0,
        "dragged from {edge} to {}, the edge is at {dragged}",
        edge + 300.0
    );
}
