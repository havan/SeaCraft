//! The Wave pattern view in the Analysis drawer: one speed you can scrub drives the speed regime,
//! humps and hollows of wave drag, catamaran interference, wash at a distance and the picture.
//!
//! The panel never changes the model or the app: it edits the wave settings it is given (view
//! inputs, like the other Analysis views' inputs) and returns what the user asked for as a
//! [`WaveViewAction`], which `App::wave_view_drawer` applies. The map's pan and zoom are view state
//! ([`MapState`]).
//!
//! Every readout is in plain words with units: no bare symbols. The numbers are thin-ship (Michell)
//! estimates in deep water, and the banner says first when the theory does not fit the hull.

use crate::analysis_views::AnalysisView;
use crate::theme;
use crate::waves::{WaveDomain, WaveDrag, WaveResult, WaveSettings, WaveSpeed};
use eframe::egui;
use seacraft_core::resistance::{GRAVITY, KNOT};
use seacraft_core::wave_pattern::HULL_SPEED_FN;

/// What the wave view asked for, applied by the App after the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveViewAction {
    None,
    /// A setting was edited in place: the pattern is recomputed.
    Changed,
    /// "Save image…": PNG, SVG or VTI.
    SaveImage,
    SetSpeed(WaveSpeed),
    /// A catamaran spacing (m), from the spacing plot.
    SetSpacing(f64),
    /// A wash distance (m), from dragging the wash line.
    SetWashDistance(f64),
    Close,
    /// Show another Analysis view in the drawer.
    Open(AnalysisView),
}

/// The map's pan and zoom: view state, kept by the App between frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MapState {
    /// Scale over the fitted view (0 until fitted).
    pub zoom: f32,
    /// Offset of the field's centre from the map's centre (points).
    pub pan: egui::Vec2,
    pub fitted: bool,
}

/// The wave view's inputs.
pub struct WaveViewPanel<'a> {
    pub settings: &'a mut WaveSettings,
    /// The last pattern computed, current or not (`current`, `stale`).
    pub result: Option<&'a WaveResult>,
    /// The wave-drag curve, when current.
    pub drag: Option<&'a WaveDrag>,
    /// The result kept predates the last change.
    pub stale: bool,
    pub busy: bool,
    /// The result is for the current inputs: it may be saved.
    pub current: bool,
    /// Why the pattern (or the drag curve) failed for the current inputs.
    pub failure: Option<&'a str>,
    /// The floating condition is heeled or trimmed, which the pattern ignores.
    pub heeled_or_trimmed: bool,
    /// The waterline length and beam the speed's units and the catamaran guess convert by.
    pub length_wl: Option<f64>,
    pub beam_wl: Option<f64>,
    pub map: &'a mut MapState,
}

/// The slider's Froude range; a typed value outside it is kept.
const SLIDER_FN: (f64, f64) = (0.10, 1.00);
/// The readouts column's width (points).
const READOUTS_WIDTH: f32 = 240.0;
/// Below this drawer width the plots go under the map.
const WIDE: f32 = 900.0;
/// Height kept for the bottom row of inputs.
const BOTTOM_ROW: f32 = 34.0;

/// Draw the view: header (the views as tabs, the speed, notes, close), the fit banner, the body
/// (readouts, map, plots) and the inputs used less often.
pub fn panel(ui: &mut egui::Ui, p: WaveViewPanel<'_>) -> WaveViewAction {
    let WaveViewPanel {
        settings,
        result,
        drag,
        stale,
        busy,
        current,
        failure,
        heeled_or_trimmed,
        length_wl,
        beam_wl,
        map: _,
    } = p;
    let mut action = WaveViewAction::None;
    let length_wl = length_wl.filter(|l| *l > 0.0);

    ui.horizontal(|ui| {
        for other in AnalysisView::ALL {
            let shown = other == AnalysisView::Waves;
            if ui.selectable_label(shown, other.label()).clicked() && !shown {
                action = WaveViewAction::Open(other);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕").on_hover_text("Close Wave pattern").clicked() {
                action = WaveViewAction::Close;
            }
        });
    });
    ui.horizontal(|ui| {
        if speed_control(ui, settings, length_wl) {
            action = WaveViewAction::Changed;
        }
        if let Some(r) = result {
            let mut notes: Vec<&str> = r.field.notes.iter().map(String::as_str).collect();
            notes.extend(r.warnings.iter().map(String::as_str));
            ui.label(egui::RichText::new("Notes").color(theme::TEXT_SECONDARY))
                .on_hover_text(notes.join("\n\n"));
        }
        if busy {
            ui.spinner();
        }
    });

    if let Some(r) = result {
        let reasons = fit_reasons(r);
        if !reasons.is_empty() {
            ui.colored_label(
                theme::WARN,
                format!(
                    "Thin-ship theory does not fit this hull ({}): read the pattern as a sketch, \
                     not a prediction.",
                    reasons.join(", ")
                ),
            );
        }
    }
    if heeled_or_trimmed {
        ui.colored_label(
            theme::WARN,
            "Computed upright at the draft; heel and trim are ignored.",
        );
    }
    if let Some(failure) = failure {
        ui.colored_label(theme::FAIL, format!("Wave pattern failed: {failure}"));
    }

    let avail = ui.available_size();
    let body = egui::vec2(avail.x, (avail.y - BOTTOM_ROW).max(120.0));
    ui.allocate_ui_with_layout(body, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_size(body);
        match result {
            Some(r) => crate::panels::stale_section(ui, stale, |ui| {
                body_columns(ui, r, drag, settings, body);
            }),
            None if busy => {
                ui.label(
                    egui::RichText::new("Computing the wave pattern…").color(theme::TEXT_MUTED),
                );
            }
            None => {}
        }
    });

    let row = bottom_row(ui, settings, result, current, length_wl, beam_wl);
    if row != WaveViewAction::None {
        action = row;
    }
    action
}

/// The number field, the kn/Fn switch and the slider (Fn 0.10 to 1.00, shown in the field's unit).
/// True when the speed changed.
fn speed_control(ui: &mut egui::Ui, s: &mut WaveSettings, length_wl: Option<f64>) -> bool {
    let mut changed = false;
    ui.label("Speed");
    let knots = s.speed.is_knots();
    let (mut value, range) = match s.speed {
        WaveSpeed::Knots(v) => (v, 0.1..=100.0),
        WaveSpeed::Froude(v) => (v, 0.01..=2.0),
    };
    if ui
        .add(
            theme::num(&mut value)
                .speed(if knots { 0.05 } else { 0.002 })
                .range(range)
                .max_decimals(3),
        )
        .changed()
    {
        s.speed = if knots {
            WaveSpeed::Knots(value)
        } else {
            WaveSpeed::Froude(value)
        };
        changed = true;
    }
    let mut to_knots = knots;
    theme::segmented(ui, |ui| {
        ui.selectable_value(&mut to_knots, true, "kn");
        ui.selectable_value(&mut to_knots, false, "Fn");
    });
    if to_knots != knots {
        // The speed itself does not jump: the value converts by the waterline length.
        s.speed = match length_wl {
            Some(l) => s.speed.in_unit(to_knots, l),
            None if to_knots => WaveSpeed::Knots(value),
            None => WaveSpeed::Froude(value),
        };
        changed = true;
    }
    // The slider spans Fn 0.10 to 1.00 in the field's unit. It edits a copy, so a typed value
    // outside its range is kept until the slider itself is moved.
    let unit = |f: f64| match (knots, length_wl) {
        (true, Some(l)) => f * (GRAVITY * l).sqrt() / KNOT,
        _ => f,
    };
    if !knots || length_wl.is_some() {
        let (lo, hi) = (unit(SLIDER_FN.0), unit(SLIDER_FN.1));
        let mut v = value;
        let r = theme::slider(
            ui,
            egui::Slider::new(&mut v, lo..=hi)
                .show_value(false)
                .clamping(egui::SliderClamping::Edits),
        )
        .on_hover_text("Scrub the speed: everything in the view follows it");
        if r.changed() && v != value {
            s.speed = if knots {
                WaveSpeed::Knots(v)
            } else {
                WaveSpeed::Froude(v)
            };
            changed = true;
        }
    }
    changed
}

/// The short reasons thin-ship theory does not fit, for the banner: "L/B 3.1", "wide transom",
/// "Fn 0.05 outside 0.20–1.00". Empty when it fits.
pub fn fit_reasons(r: &WaveResult) -> Vec<String> {
    let mut reasons = Vec::new();
    if r.beam_wl > 0.0 && r.length_wl / r.beam_wl < 6.0 {
        reasons.push(format!("L/B {:.1}", r.length_wl / r.beam_wl));
    }
    if r.warnings.iter().any(|w| w.starts_with("wetted transom")) {
        reasons.push("wide transom".to_string());
    }
    let f = r.field.froude;
    if !(0.20..=1.0).contains(&f) {
        reasons.push(format!("Fn {f:.2} outside 0.20–1.00"));
    }
    reasons
}

/// A signed number with a true minus sign: "+0.42", "−0.39".
fn signed(x: f64, decimals: usize) -> String {
    let text = format!("{:.*}", decimals, x.abs());
    if x < 0.0 && !text.trim_start_matches(['0', '.']).is_empty() {
        format!("−{text}")
    } else {
        format!("+{text}")
    }
}

/// The catamaran interference in words: within 1 % is neutral.
pub fn interference_words(ratio: f64) -> String {
    let pct = (ratio - 1.0) * 100.0;
    if pct.abs() < 1.0 {
        "neutral (±1 %)".to_string()
    } else if pct < 0.0 {
        format!("favourable (−{:.0} %)", pct.abs())
    } else {
        format!("unfavourable (+{pct:.0} %)")
    }
}

/// Readouts, map and plots; the plots go under the map when the drawer is narrow.
fn body_columns(
    ui: &mut egui::Ui,
    r: &WaveResult,
    drag: Option<&WaveDrag>,
    settings: &WaveSettings,
    size: egui::Vec2,
) {
    let wide = size.x >= WIDE;
    let gap = ui.spacing().item_spacing.x;
    let plots_w = if wide { (0.34 * size.x).round() } else { 0.0 };
    let map_w = (size.x - READOUTS_WIDTH - plots_w - 2.0 * gap).max(120.0);
    let top_down = egui::Layout::top_down(egui::Align::Min);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(READOUTS_WIDTH, size.y), top_down, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("wave_readouts")
                .show(ui, |ui| readouts(ui, r, settings));
        });
        ui.allocate_ui_with_layout(egui::vec2(map_w, size.y), top_down, |ui| {
            let map_h = if wide {
                size.y
            } else {
                (0.55 * size.y).round()
            };
            map_placeholder(ui, egui::vec2(map_w, map_h));
            if !wide {
                plots_placeholder(ui, r, drag);
            }
        });
        if wide {
            ui.allocate_ui_with_layout(egui::vec2(plots_w, size.y), top_down, |ui| {
                plots_placeholder(ui, r, drag);
            });
        }
    });
}

/// The left column: the regime sentence, then one row per question, each with a definition on
/// hover.
fn readouts(ui: &mut egui::Ui, r: &WaveResult, settings: &WaveSettings) {
    ui.add(egui::Label::new(r.regime.sentence()).wrap());
    ui.add_space(6.0);
    let f = &r.field;
    let (mut crest, mut trough) = (0.0f64, 0.0f64);
    for (z, wet) in f.zeta.iter().zip(&f.wet) {
        if *wet {
            crest = crest.max(*z);
            trough = trough.min(*z);
        }
    }
    let hull_speed_kn = HULL_SPEED_FN * (GRAVITY * r.length_wl).sqrt() / KNOT;
    let mut rows: Vec<(String, String, &str)> =
        vec![
        (
            "Wave length".into(),
            format!(
                "{:.1} m = {:.1} × waterline",
                f.transverse_wavelength_m,
                f.transverse_wavelength_m / r.length_wl
            ),
            "The length of the transverse waves behind the hull, crest to crest, against the \
             waterline length.",
        ),
        (
            "Hull speed".into(),
            format!("{hull_speed_kn:.1} kn (Fn 0.40, rule of thumb)"),
            "The speed at which the transverse wave is about as long as the waterline: a rule of \
             thumb, not a limit.",
        ),
        (
            "Highest crest".into(),
            format!("{} m · deepest trough {} m", signed(crest, 2), signed(trough, 2)),
            "The highest and lowest water on the map, away from the hull (far field only).",
        ),
        (
            "Wave drag here".into(),
            format!(
                "{:.2} kN (C_w {:.2} × 10⁻³)",
                r.drag_here.rw_n / 1000.0,
                r.drag_here.cw * 1000.0
            ),
            "Michell's thin-ship estimate of the wave-making resistance at this speed, and its \
             coefficient on the wetted surface.",
        ),
    ];
    let off_map = match settings.domain {
        WaveDomain::Manual { side_m, .. } => r.wash_distance_m > side_m,
        WaveDomain::Auto => false,
    };
    let wash = match r.wash {
        Some(w) => format!("{:.2} m crest to trough", w.height_m),
        None => "calm".to_string(),
    };
    rows.push((
        format!("Wash at {:.1} m", r.wash_distance_m),
        if off_map {
            format!("{wash} · wash line off the map")
        } else {
            wash
        },
        "The largest crest-to-trough height along a line this far to the side: a thin-ship \
         estimate in deep water, not a wash assessment against any rule.",
    ));
    if let Some(s) = r.cat_spacing_m {
        rows.push((
            "Spacing".into(),
            format!("{s:.1} m ({:.2} L)", s / r.length_wl),
            "The distance between the demihulls' centrelines.",
        ));
        if let Some(i) = r.interference {
            rows.push((
                "Interference".into(),
                interference_words(i),
                "The pair's wave drag against twice one demihull's alone at this speed: below \
                 that (favourable) the wave systems partly cancel.",
            ));
        }
    }
    egui::Grid::new("wave_readouts_grid")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            for (name, value, hover) in rows {
                ui.label(egui::RichText::new(name).color(theme::TEXT_SECONDARY))
                    .on_hover_text(hover);
                ui.add(egui::Label::new(value).wrap());
                ui.end_row();
            }
        });
}

/// The map's place until the map is drawn (Task 8 of the plan).
fn map_placeholder(ui: &mut egui::Ui, size: egui::Vec2) {
    let (rect, _) = ui.allocate_exact_size(size, egui::Sense::hover());
    ui.painter().rect_filled(rect, 0.0, theme::PLOT_BG);
}

/// The plots' place until they are drawn (Task 9 of the plan).
fn plots_placeholder(ui: &mut egui::Ui, _r: &WaveResult, _drag: Option<&WaveDrag>) {
    ui.label(egui::RichText::new("Wave drag").color(theme::TEXT_SECONDARY));
}

/// Catamaran and spacing, the wash line, the Map and Colours menus, the Plan view contours and
/// "Save image…".
fn bottom_row(
    ui: &mut egui::Ui,
    s: &mut WaveSettings,
    result: Option<&WaveResult>,
    current: bool,
    length_wl: Option<f64>,
    beam_wl: Option<f64>,
) -> WaveViewAction {
    let mut changed = false;
    let mut save = false;
    ui.horizontal_wrapped(|ui| {
        let mut cat = s.cat_spacing_m.is_some();
        if ui.checkbox(&mut cat, "Catamaran").changed() {
            let guess = crate::waves::catamaran_guess(length_wl, beam_wl);
            s.cat_spacing_m = cat.then_some(guess);
            changed = true;
        }
        if let Some(spacing) = s.cat_spacing_m.as_mut() {
            changed |= ui
                .add(
                    theme::num(spacing)
                        .speed(0.05)
                        .range(0.1..=1000.0)
                        .prefix("spacing ")
                        .suffix(" m"),
                )
                .on_hover_text("Between the demihulls' centrelines")
                .changed();
        }
        ui.separator();
        let mut wash = s
            .wash_distance_m
            .or(result.map(|r| r.wash_distance_m))
            .unwrap_or(10.0);
        if ui
            .add(
                theme::num(&mut wash)
                    .speed(0.1)
                    .range(0.1..=10_000.0)
                    .prefix("Wash line ")
                    .suffix(" m"),
            )
            .on_hover_text("Distance to the side where the wash height is read")
            .changed()
        {
            s.wash_distance_m = Some(wash);
            changed = true;
        }
        ui.menu_button("Map ▾", |ui| {
            let mut auto = s.domain == WaveDomain::Auto;
            if ui
                .checkbox(&mut auto, "Auto domain")
                .on_hover_text("Ahead 0.5 L, behind 3 L, each side to clear the wash line")
                .changed()
            {
                let l = length_wl.unwrap_or(10.0);
                let side = result.map_or(1.5 * l, |r| r.field.ys[r.field.ys.len() - 1]);
                s.domain = if auto {
                    WaveDomain::Auto
                } else {
                    WaveDomain::Manual {
                        ahead_m: 0.5 * l,
                        behind_m: 3.0 * l,
                        side_m: side,
                    }
                };
                changed = true;
            }
            if let WaveDomain::Manual {
                ahead_m,
                behind_m,
                side_m,
            } = &mut s.domain
            {
                for (v, label) in [
                    (ahead_m, "ahead of the bow "),
                    (behind_m, "behind the stern "),
                    (side_m, "each side "),
                ] {
                    changed |= ui
                        .add(
                            theme::num(v)
                                .speed(0.1)
                                .range(0.0..=10_000.0)
                                .prefix(label)
                                .suffix(" m"),
                        )
                        .changed();
                }
            }
            ui.checkbox(&mut s.kelvin_wedge, "Kelvin wedge");
        });
        ui.menu_button("Colours ▾", |ui| {
            let mut auto = s.clip_m.is_none();
            if ui.checkbox(&mut auto, "Auto range").changed() {
                s.clip_m = if auto {
                    None
                } else {
                    Some(result.map_or(0.1, |r| r.clip_m).max(0.001))
                };
                changed = true;
            }
            if let Some(clip) = s.clip_m.as_mut() {
                changed |= ui
                    .add(
                        theme::num(clip)
                            .speed(0.002)
                            .range(0.001..=10.0)
                            .prefix("± ")
                            .suffix(" m"),
                    )
                    .changed();
            }
            changed |= ui
                .add(
                    theme::num(&mut s.contours)
                        .range(0..=12)
                        .prefix("Contours each side: "),
                )
                .changed();
        });
        ui.checkbox(&mut s.plan_contours, "Contours on Plan view")
            .on_hover_text("Faint contours and the wash line over Plan view");
        ui.separator();
        if ui
            .add_enabled(
                current && result.is_some(),
                egui::Button::new("Save image…"),
            )
            .on_hover_text("PNG, SVG or VTI, by the file's extension")
            .clicked()
        {
            save = true;
        }
    });
    if save {
        WaveViewAction::SaveImage
    } else if changed {
        WaveViewAction::Changed
    } else {
        WaveViewAction::None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn interference_is_worded() {
        assert_eq!(interference_words(0.88), "favourable (−12 %)");
        assert_eq!(interference_words(1.30), "unfavourable (+30 %)");
        assert_eq!(interference_words(1.004), "neutral (±1 %)");
    }

    #[test]
    fn signed_uses_a_true_minus() {
        assert_eq!(signed(0.42, 2), "+0.42");
        assert_eq!(signed(-0.39, 2), "−0.39");
        assert_eq!(signed(-0.0001, 2), "+0.00");
    }
}
