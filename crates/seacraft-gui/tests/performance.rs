//! Headless tests for the Performance tab (resistance curves and seakeeping RAOs).

use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_core::i18n::{Locale, Tr};
use seacraft_core::resistance;
use seacraft_core::seakeeping;
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_gui::app::{DesignDock, ResistanceMethod, ResistanceResult, ResistanceRow};
use seacraft_gui::panels::resistance_series;
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn frame(ctx: &egui::Context, app: &mut App) {
    let input = egui::RawInput {
        screen_rect: Some(egui::Rect::from_min_size(
            egui::pos2(0.0, 0.0),
            egui::vec2(1600.0, 1000.0),
        )),
        ..Default::default()
    };
    let mut out = ctx.run_ui(input, |ui| app.draw(ui));
    out.textures_delta.clear();
}

#[test]
fn performance_tab_and_computations() {
    // R5-3: a .fef opened without a project KG has none, and no KG means no
    // seakeeping (a copy of the demo without its sidecar).
    let dir = std::env::temp_dir().join(format!("seacraft-performance-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let bare = dir.join("bare-demo.fef");
    std::fs::copy(demo_path(), &bare).unwrap();
    let _ = std::fs::remove_file(seacraft_core::project::sidecar_path(&bare));
    let mut no_kg = App::headless_for_test(Some(bare.to_str().unwrap()));
    no_kg.set_subdivision_level(2);
    assert!(no_kg.seakeeping_spec().is_err(), "no KG, no seakeeping");

    // The committed demo brings the demo's KG in its sidecar; the
    // seakeeping below needs it.
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    assert_eq!(app.kg(), Some(seacraft_gui::app::DEMO_KG));
    let ctx = egui::Context::default();

    // Settle docks with two frames
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);

    // 1. Opening the Performance tab hides the Loading tab
    app.set_show_loading(true);
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    assert!(app.show_loading());
    assert!(!app.show_performance());
    assert_eq!(app.design_dock(), Some(DesignDock::Loading));

    app.set_show_performance(true);
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
    assert!(app.show_performance());
    assert!(!app.show_loading());
    assert_eq!(app.design_dock(), Some(DesignDock::Performance));

    // 2. compute_resistance_blocking for Holtrop equals resistance::curve(&resistance::input_for(..), &speeds) exactly (same speeds)
    let res = app
        .compute_resistance_blocking()
        .expect("compute resistance")
        .clone();
    let surface = app.surface().expect("surface").clone();
    let subdiv = SubdivisionOptions {
        level: app.subdivision_level(),
        mode: app.subdivision_mode(),
    };
    let speeds = app.performance_speeds();
    let holtrop_in = resistance::input_for(&surface, subdiv).expect("holtrop input");
    let expected_core = resistance::curve(&holtrop_in, &speeds);
    let expected_res = ResistanceResult::from_core(
        "Holtrop & Mennen",
        &expected_core,
        holtrop_in.warnings(),
        "R_T (kN)",
    )
    .with_in_range(|_| !holtrop_in.outside_fitted_range());
    assert_eq!(res, expected_res);
    assert_eq!(app.resistance_result(), Some(&expected_res));

    // 3. compute_raos_blocking frequencies equal seakeeping::head_sea_raos with the same SeakeepingSpec exactly
    let rep = app.compute_raos_blocking().expect("compute raos").clone();
    let spec = app.seakeeping_spec().expect("the demo has a KG");
    let expected_rep = seakeeping::head_sea_raos(&surface, &spec).expect("head sea raos");
    assert_eq!(rep.frequencies, expected_rep.frequencies);
    assert_eq!(
        app.rao_result().map(|r| &r.frequencies),
        Some(&expected_rep.frequencies)
    );

    // 4. The worker path: start_performance_compute(), then frames (sleep 50 ms, up to 60 s) until both results are Some
    app.performance_state_mut().resistance = None;
    app.performance_state_mut().raos = None;
    assert!(app.resistance_result().is_none());
    assert!(app.rao_result().is_none());

    app.start_performance_compute();
    let start = Instant::now();
    while app.resistance_result().is_none() || app.rao_result().is_none() {
        if start.elapsed() > Duration::from_secs(60) {
            panic!("timed out waiting for performance worker threads");
        }
        std::thread::sleep(Duration::from_millis(50));
        frame(&ctx, &mut app);
    }
    assert!(app.resistance_result().is_some());
    assert!(app.rao_result().is_some());
    assert_eq!(app.resistance_result(), Some(&expected_res));
    assert_eq!(
        app.rao_result().map(|r| &r.frequencies),
        Some(&expected_rep.frequencies)
    );
}

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}.fef", env!("CARGO_MANIFEST_DIR"))
}

fn resistance_of(
    name: &str,
    method: ResistanceMethod,
    speeds: (f64, f64, f64),
) -> ResistanceResult {
    let mut app = App::headless_for_test(Some(&example(name)));
    app.set_subdivision_level(2);
    app.performance_state_mut().method = method;
    app.performance_state_mut().speeds_kn = speeds;
    app.compute_resistance_blocking()
        .expect("compute resistance")
        .clone()
}

/// The demo hull's L/B (3.02) and B/T (5.71) lie outside Holtrop's ranges, so every speed is
/// outside the fit; the container ship lies inside them, so none is.
#[test]
fn holtrop_rows_outside_the_fitted_form_are_out_of_range() {
    let demo = resistance_of("demo-hull", ResistanceMethod::Holtrop, (4.0, 14.0, 2.0));
    assert!(!demo.rows.is_empty());
    assert!(demo.rows.iter().all(|r| !r.in_range), "{:?}", demo.rows);
    let ship = resistance_of(
        "container-230m",
        ResistanceMethod::Holtrop,
        (10.0, 24.0, 2.0),
    );
    assert!(!ship.rows.is_empty());
    assert!(ship.rows.iter().all(|r| r.in_range), "{:?}", ship.rows);
}

/// DSYHS is fitted for Fn 0.15 to 0.75: the speeds outside are out of range, the rest in.
#[test]
fn dsyhs_rows_outside_its_froude_range_are_out_of_range() {
    let res = resistance_of("yacht-10m", ResistanceMethod::Dsyhs, (1.0, 15.0, 1.0));
    assert!(res.rows.iter().any(|r| r.in_range));
    assert!(res.rows.iter().any(|r| !r.in_range));
    for r in &res.rows {
        assert_eq!(r.in_range, (0.15..=0.75).contains(&r.fn_), "{r:?}");
    }
}

fn row(speed_kn: f64, in_range: bool) -> ResistanceRow {
    ResistanceRow {
        speed_kn,
        fn_: 0.0,
        rt_kn: speed_kn * 2.0,
        pe_kw: None,
        note: None,
        in_range,
    }
}

fn result(rows: Vec<ResistanceRow>) -> ResistanceResult {
    ResistanceResult {
        method: "Holtrop & Mennen".into(),
        rows,
        warnings: Vec::new(),
        rt_label: "R_T (kN)".into(),
    }
}

/// The whole curve in the method's colour, and the segments touching an out-of-range speed drawn
/// again on top in grey, broken (NaN) between separate stretches.
#[test]
fn resistance_series_grey_the_segments_out_of_range() {
    let tr = Tr::new(Locale::En);
    let flags = [true, true, false, true, true, false];
    let res = result(
        flags
            .iter()
            .enumerate()
            .map(|(i, f)| row(i as f64, *f))
            .collect(),
    );
    let series = resistance_series(&res, tr);
    assert_eq!(series.len(), 2);
    assert!(!series[0].muted);
    assert_eq!(series[0].label, "Holtrop & Mennen");
    assert_eq!(series[0].points.len(), 6);
    assert!(series[1].muted);
    assert!(series[1].label.contains("outside"), "{}", series[1].label);
    let xs: Vec<f64> = series[1].points.iter().map(|p| p.0).collect();
    assert_eq!(xs.len(), 6, "{xs:?}");
    assert_eq!(&xs[..3], &[1.0, 2.0, 3.0]);
    assert!(xs[3].is_nan());
    assert_eq!(&xs[4..], &[4.0, 5.0]);
}

#[test]
fn a_curve_all_in_range_has_no_grey_series() {
    let res = result((0..4).map(|i| row(i as f64, true)).collect());
    let series = resistance_series(&res, Tr::new(Locale::En));
    assert_eq!(series.len(), 1);
    assert!(!series[0].muted);
}

#[test]
fn a_curve_all_out_of_range_is_grey_end_to_end() {
    let res = result((0..4).map(|i| row(i as f64, false)).collect());
    let series = resistance_series(&res, Tr::new(Locale::En));
    assert_eq!(series.len(), 2);
    assert_eq!(series[1].points, series[0].points);
    // One speed alone is a grey point, not an empty series.
    let one = result(vec![row(3.0, false)]);
    let series = resistance_series(&one, Tr::new(Locale::En));
    assert_eq!(series[1].points, vec![(3.0, 6.0)]);
}

/// The Performance tab draws the grey curve without panicking.
#[test]
fn the_performance_tab_draws_a_grey_curve() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    let ctx = egui::Context::default();
    app.set_show_performance(true);
    app.compute_resistance_blocking().expect("resistance");
    frame(&ctx, &mut app);
    frame(&ctx, &mut app);
}

/// The table keeps every number, and greys the rows outside the method's range.
#[test]
fn resistance_table_greys_rows_out_of_range() {
    use seacraft_gui::panels::{resistance_table, EngCell};
    let rows = resistance_table(&result(vec![row(1.0, true), row(2.0, false)]));
    assert!(rows[0].iter().all(|c| matches!(c, EngCell::Text(_))));
    assert!(rows[1].iter().all(|c| matches!(c, EngCell::Muted(_))));
    assert_eq!(rows[1][0], EngCell::Muted("2.00".into()));
}
