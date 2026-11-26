//! The Wave pattern view in the Analysis drawer: one speed you can scrub drives the speed regime,
//! humps and hollows of wave drag, catamaran interference, wash at a distance and the picture. It is
//! the Performance tab's speed, the one resistance and seakeeping use too.
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
use seacraft_core::i18n::{Key, Locale, Tr};
use seacraft_core::resistance::{GRAVITY, KNOT};
use seacraft_core::wave_pattern::{HULL_SPEED_FN, KELVIN_HALF_ANGLE_DEG};

/// What the wave view asked for, applied by the App after the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveViewAction {
    None,
    /// A setting was edited in place: the pattern is recomputed.
    Changed,
    /// "Save image…": PNG, SVG or VTI.
    SaveImage,
    /// A speed (knots), from a click on the drag plot.
    SetSpeed(f64),
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
    /// The pointer went down on a wash line: the drag moves the line, not the map.
    pub dragging_wash: bool,
    /// The catamaran's drag plot shows interference against spacing, not drag against speed.
    pub spacing_plot: bool,
    /// The map's frame when a wash-line drag began, kept until the release. With the Auto
    /// domain the map's sides follow the wash line, so a pattern landing mid-drag would refit
    /// the map, and the pointer, held still, would read a new distance on every refit.
    wash_frame: Option<MapTransform>,
}

/// The wave view's inputs.
pub struct WaveViewPanel<'a> {
    pub settings: &'a mut WaveSettings,
    /// The speed resistance, waves and seakeeping share (knots); the view edits it in knots or
    /// as a Froude number.
    pub speed_kn: &'a mut f64,
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
    /// The kept pattern's image, loaded as a texture.
    pub texture: Option<egui::TextureId>,
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
    panel_tr(ui, p, Tr::new(Locale::En))
}

/// Localized form of [`panel`].
pub fn panel_tr(ui: &mut egui::Ui, p: WaveViewPanel<'_>, tr: Tr) -> WaveViewAction {
    let WaveViewPanel {
        settings,
        speed_kn,
        result,
        drag,
        stale,
        busy,
        current,
        failure,
        heeled_or_trimmed,
        length_wl,
        beam_wl,
        texture,
        map,
    } = p;
    let mut action = WaveViewAction::None;
    let length_wl = length_wl.filter(|l| *l > 0.0);

    ui.horizontal_wrapped(|ui| {
        for other in AnalysisView::ALL {
            let shown = other == AnalysisView::Waves;
            if ui
                .selectable_label(shown, other.tab_label_tr(shown, tr))
                .clicked()
                && !shown
            {
                action = WaveViewAction::Open(other);
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("✕")
                .on_hover_text(tr.s(Key::WaveCloseTip))
                .clicked()
            {
                action = WaveViewAction::Close;
            }
        });
    });
    ui.horizontal(|ui| {
        // The method's name. When a second method arrives (planing, say), this becomes the
        // drop-down that picks it.
        ui.label(egui::RichText::new(tr.s(Key::WaveMethodTitle)).strong())
            .on_hover_text(tr.s(Key::WaveMethodHover));
        ui.separator();
        if speed_control(ui, settings, speed_kn, length_wl, tr) {
            action = WaveViewAction::Changed;
        }
        if let Some(r) = result {
            let notes: Vec<String> = r
                .field
                .notes
                .iter()
                .chain(&r.warnings)
                .map(|n| n.render(tr))
                .collect();
            ui.label(egui::RichText::new(tr.s(Key::WaveNotes)).color(theme::TEXT_SECONDARY))
                .on_hover_text(notes.join("\n\n"));
        }
        if busy {
            ui.spinner();
        }
        // The chip sits in the header, so the map does not move when the result goes stale (as
        // it does mid-drag on the wash line).
        if stale && result.is_some() {
            crate::panels::stale_chip_tr(ui, tr);
        }
    });

    if let Some(r) = result {
        let reasons = fit_reasons_tr(r, tr);
        if !reasons.is_empty() {
            let joined = reasons.join(", ");
            ui.colored_label(
                theme::WARN,
                tr.fmt(Key::WaveBannerFit, &[("reasons", &joined)]),
            );
        }
    }
    if heeled_or_trimmed {
        ui.colored_label(theme::WARN, tr.s(Key::WaveBannerHeeledOrTrimmed));
    }
    if let Some(failure) = failure {
        ui.colored_label(
            theme::FAIL,
            tr.fmt(Key::WaveBannerFailed, &[("failure", failure)]),
        );
    }

    let avail = ui.available_size();
    let body = egui::vec2(avail.x, (avail.y - BOTTOM_ROW).max(120.0));
    ui.allocate_ui_with_layout(body, egui::Layout::top_down(egui::Align::Min), |ui| {
        ui.set_min_size(body);
        match result {
            Some(r) => {
                let asked = ui
                    .scope(|ui| {
                        if stale {
                            ui.multiply_opacity(theme::STALE_OPACITY);
                        }
                        body_columns(ui, r, drag, settings, texture, map, body, tr)
                    })
                    .inner;
                if asked != WaveViewAction::None {
                    action = asked;
                }
            }
            None if busy => {
                ui.label(
                    egui::RichText::new(tr.s(Key::WaveComputingPattern)).color(theme::TEXT_MUTED),
                );
            }
            None => {}
        }
    });

    let row = bottom_row(ui, settings, result, current, length_wl, beam_wl, tr);
    if row != WaveViewAction::None {
        action = row;
    }
    action
}

/// The number field, the kn/Fn switch and the slider (Fn 0.10 to 1.00, shown in the field's unit).
/// The speed is the shared one, in knots; the field shows it as a Froude number when the settings
/// say so and a waterline length is known, else in knots. True when the speed or its unit changed.
fn speed_control(
    ui: &mut egui::Ui,
    s: &mut WaveSettings,
    speed_kn: &mut f64,
    length_wl: Option<f64>,
    tr: Tr,
) -> bool {
    let mut changed = false;
    ui.label(tr.s(Key::WaveCtlSpeed))
        .on_hover_text(tr.s(Key::WaveCtlSpeedTip));
    let shown = |kn: f64| shown_speed(kn, s.froude_unit, length_wl);
    let current = *speed_kn;
    let to_kn = |v: WaveSpeed| match (v, length_wl) {
        (WaveSpeed::Knots(kn), _) => kn,
        (WaveSpeed::Froude(_), Some(l)) => v.m_s(l) / KNOT,
        (WaveSpeed::Froude(_), None) => current,
    };
    let speed = shown(*speed_kn);
    let knots = speed.is_knots();
    let in_unit = |v: f64| {
        if knots {
            WaveSpeed::Knots(v)
        } else {
            WaveSpeed::Froude(v)
        }
    };
    let (mut value, range) = match speed {
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
        .on_hover_text(tr.s(Key::WaveCtlSpeedDragTip))
        .changed()
    {
        *speed_kn = to_kn(in_unit(value));
        changed = true;
    }
    // The unit is only how the speed reads: switching it does not move the speed. Without a
    // waterline length the speed reads in knots, and "Fn" cannot be picked.
    let (selected, fn_enabled) = unit_switch(s.froude_unit, length_wl);
    let mut froude = selected;
    theme::segmented(ui, |ui| {
        ui.selectable_value(&mut froude, false, "kn")
            .on_hover_text(tr.s(Key::WaveCtlKnTip));
        ui.add_enabled_ui(fn_enabled, |ui| {
            ui.selectable_value(&mut froude, true, "Fn")
                .on_hover_text(tr.s(Key::WaveCtlFnTip))
                .on_disabled_hover_text(tr.s(Key::WaveCtlFnNoLengthTip));
        });
    });
    if froude != selected {
        s.froude_unit = froude;
        changed = true;
    }
    // The slider spans Fn 0.10 to 1.00 in the field's unit. It edits a copy, so a typed value
    // outside its range is kept until the slider itself is moved.
    if let Some(l) = length_wl {
        let unit = |f: f64| {
            if knots {
                f * (GRAVITY * l).sqrt() / KNOT
            } else {
                f
            }
        };
        let (lo, hi) = (unit(SLIDER_FN.0), unit(SLIDER_FN.1));
        let mut v = value;
        let r = theme::slider(
            ui,
            egui::Slider::new(&mut v, lo..=hi)
                .show_value(false)
                .clamping(egui::SliderClamping::Edits),
        )
        .on_hover_text(tr.s(Key::WaveCtlSpeedSliderTip));
        if r.changed() && v != value {
            *speed_kn = to_kn(in_unit(v));
            changed = true;
        }
    }
    changed
}

/// The shared speed (knots) as the view shows it: a Froude number when `froude_unit` is set and a
/// waterline length (m) is known, else knots.
fn shown_speed(speed_kn: f64, froude_unit: bool, length_wl: Option<f64>) -> WaveSpeed {
    match length_wl.filter(|l| *l > 0.0) {
        Some(l) if froude_unit => WaveSpeed::Knots(speed_kn).in_unit(false, l),
        _ => WaveSpeed::Knots(speed_kn),
    }
}

/// The kn / Fn switch: whether "Fn" is the unit shown, and whether it can be picked (it needs a
/// waterline length).
fn unit_switch(froude_unit: bool, length_wl: Option<f64>) -> (bool, bool) {
    let known = length_wl.is_some_and(|l| l > 0.0);
    (froude_unit && known, known)
}

/// The short reasons thin-ship theory does not fit, for the banner: "L/B 3.1", "wide transom",
/// "Fn 0.05 outside 0.20–1.00". Empty when it fits.
pub fn fit_reasons(r: &WaveResult) -> Vec<String> {
    fit_reasons_tr(r, Tr::new(Locale::En))
}

/// Localized form of [`fit_reasons`].
pub fn fit_reasons_tr(r: &WaveResult, tr: Tr) -> Vec<String> {
    let mut reasons = Vec::new();
    if r.beam_wl > 0.0 && r.length_wl / r.beam_wl < 6.0 {
        let lb_s = format!("{:.1}", r.length_wl / r.beam_wl);
        reasons.push(tr.fmt(Key::WaveFitLb, &[("lb", &lb_s)]));
    }
    if r.warnings
        .iter()
        .any(|w| w.key() == Some(Key::WarnMichellTransom))
    {
        reasons.push(tr.s(Key::WaveFitWideTransom).to_string());
    }
    let f = r.field.froude;
    if !(0.20..=1.0).contains(&f) {
        let fn_s = format!("{f:.2}");
        reasons.push(tr.fmt(Key::WaveFitFnOutside, &[("fn", &fn_s)]));
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
    interference_words_tr(ratio, Tr::new(Locale::En))
}

/// Localized form of [`interference_words`].
pub fn interference_words_tr(ratio: f64, tr: Tr) -> String {
    let pct = (ratio - 1.0) * 100.0;
    if pct.abs() < 1.0 {
        tr.s(Key::WaveInterferenceNeutral).to_string()
    } else if pct < 0.0 {
        let pct_s = format!("{:.0}", pct.abs());
        tr.fmt(Key::WaveInterferenceFavourable, &[("pct", &pct_s)])
    } else {
        let pct_s = format!("{pct:.0}");
        tr.fmt(Key::WaveInterferenceUnfavourable, &[("pct", &pct_s)])
    }
}

/// Readouts, map and plots; the plots go under the map when the drawer is narrow.
/// Returns what the map (a dragged wash line) or the plots (a clicked speed or spacing) asked for.
#[allow(clippy::too_many_arguments)]
fn body_columns(
    ui: &mut egui::Ui,
    r: &WaveResult,
    drag: Option<&WaveDrag>,
    settings: &WaveSettings,
    texture: Option<egui::TextureId>,
    map: &mut MapState,
    size: egui::Vec2,
    tr: Tr,
) -> WaveViewAction {
    let mut action = WaveViewAction::None;
    let wide = size.x >= WIDE;
    let gap = ui.spacing().item_spacing.x;
    let plots_w = if wide { (0.34 * size.x).round() } else { 0.0 };
    let map_w = (size.x - READOUTS_WIDTH - plots_w - 2.0 * gap).max(120.0);
    let top_down = egui::Layout::top_down(egui::Align::Min);
    ui.horizontal_top(|ui| {
        ui.allocate_ui_with_layout(egui::vec2(READOUTS_WIDTH, size.y), top_down, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("wave_readouts")
                .show(ui, |ui| readouts(ui, r, settings, tr));
        });
        ui.allocate_ui_with_layout(egui::vec2(map_w, size.y), top_down, |ui| {
            let map_h = if wide {
                size.y
            } else {
                (0.55 * size.y).round()
            };
            if let Some(d) = wave_map(ui, r, settings, texture, map, egui::vec2(map_w, map_h), tr) {
                action = WaveViewAction::SetWashDistance(d);
            }
            if !wide {
                let asked = plots(ui, r, drag, map, tr);
                if asked != WaveViewAction::None {
                    action = asked;
                }
            }
        });
        if wide {
            ui.allocate_ui_with_layout(egui::vec2(plots_w, size.y), top_down, |ui| {
                let asked = plots(ui, r, drag, map, tr);
                if asked != WaveViewAction::None {
                    action = asked;
                }
            });
        }
    });
    action
}

/// The left column: the regime sentence, then one readout per question (its name over its value),
/// each with a definition on hover.
fn readouts(ui: &mut egui::Ui, r: &WaveResult, settings: &WaveSettings, tr: Tr) {
    ui.add(egui::Label::new(r.regime.sentence_tr(tr)).wrap());
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
    let len_s = format!("{:.1}", f.transverse_wavelength_m);
    let ratio_s = format!("{:.1}", f.transverse_wavelength_m / r.length_wl);
    let hull_kn_s = format!("{hull_speed_kn:.1}");
    let crest_s = signed(crest, 2);
    let trough_s = signed(trough, 2);
    let rw_s = format!("{:.2}", r.drag_here.rw_n / 1000.0);
    let cw_s = format!("{:.2}", r.drag_here.cw * 1000.0);
    let mut rows: Vec<(String, String, &'static str)> = vec![
        (
            tr.s(Key::WaveReadoutWavelength).into(),
            tr.fmt(
                Key::WaveReadoutWavelengthVal,
                &[("len", &len_s), ("ratio", &ratio_s)],
            ),
            tr.s(Key::WaveReadoutWavelengthTip),
        ),
        (
            tr.s(Key::WaveReadoutHullSpeed).into(),
            tr.fmt(Key::WaveReadoutHullSpeedVal, &[("kn", &hull_kn_s)]),
            tr.s(Key::WaveReadoutHullSpeedTip),
        ),
        (
            tr.s(Key::WaveReadoutCrest).into(),
            tr.fmt(
                Key::WaveReadoutCrestVal,
                &[("crest", &crest_s), ("trough", &trough_s)],
            ),
            tr.s(Key::WaveReadoutCrestTip),
        ),
        (
            tr.s(Key::WaveReadoutDrag).into(),
            tr.fmt(Key::WaveReadoutDragVal, &[("rw", &rw_s), ("cw", &cw_s)]),
            tr.s(Key::WaveReadoutDragTip),
        ),
    ];
    let off_map = match settings.domain {
        WaveDomain::Manual { side_m, .. } => r.wash_distance_m > side_m,
        WaveDomain::Auto => false,
    };
    let wash = match r.wash {
        Some(w) => {
            let h_s = format!("{:.2}", w.height_m);
            tr.fmt(Key::WaveReadoutWashHeight, &[("h", &h_s)])
        }
        None => tr.s(Key::WaveReadoutWashCalm).to_string(),
    };
    let d_s = format!("{:.1}", r.wash_distance_m);
    rows.push((
        tr.fmt(Key::WaveReadoutWash, &[("d", &d_s)]),
        if off_map {
            tr.fmt(Key::WaveReadoutWashOffMap, &[("wash", &wash)])
        } else {
            wash
        },
        tr.s(Key::WaveReadoutWashTip),
    ));
    if let Some(s) = r.cat_spacing_m {
        let s_s = format!("{s:.1}");
        let s_ratio = format!("{:.2}", s / r.length_wl);
        rows.push((
            tr.s(Key::WaveReadoutSpacing).into(),
            tr.fmt(
                Key::WaveReadoutSpacingVal,
                &[("s", &s_s), ("ratio", &s_ratio)],
            ),
            tr.s(Key::WaveReadoutSpacingTip),
        ));
        if let Some(i) = r.interference {
            rows.push((
                tr.s(Key::WaveReadoutInterference).into(),
                interference_words_tr(i, tr),
                tr.s(Key::WaveReadoutInterferenceTip),
            ));
        }
    }
    // Each name over its value: the column is narrow, and a value beside its name wraps into a
    // tall, cramped block.
    for (name, value, hover) in rows {
        ui.label(egui::RichText::new(name).color(theme::TEXT_SECONDARY))
            .on_hover_text(hover);
        ui.add(egui::Label::new(value).wrap()).on_hover_text(hover);
        ui.add_space(6.0);
    }
}

/// World (m, model axes) to the map's screen: bow to the right, starboard (+y) up.
#[derive(Debug, Clone, Copy, PartialEq)]
struct MapTransform {
    centre_world: (f64, f64),
    centre_screen: egui::Pos2,
    /// Points per metre.
    scale: f32,
}

impl MapTransform {
    fn at(&self, x: f64, y: f64) -> egui::Pos2 {
        let dx = (x - self.centre_world.0) as f32 * self.scale;
        let dy = (y - self.centre_world.1) as f32 * self.scale;
        self.centre_screen + egui::vec2(dx, -dy)
    }

    fn world(&self, p: egui::Pos2) -> (f64, f64) {
        let d = (p - self.centre_screen) / self.scale;
        (
            self.centre_world.0 + d.x as f64,
            self.centre_world.1 - d.y as f64,
        )
    }
}

/// A round length for the scale bar near `target` metres: 1, 2 or 5 × 10ⁿ.
fn round_length(target: f64) -> f64 {
    let p = 10f64.powf(target.max(1e-6).log10().floor());
    [5.0, 2.0, 1.0]
        .into_iter()
        .map(|m| m * p)
        .find(|v| *v <= target)
        .unwrap_or(p)
}

/// Pixels within which a press picks the wash line instead of panning.
const WASH_PICK: f32 = 6.0;
/// Height of the colour bar under the map, with its label.
const COLOUR_BAR: f32 = 26.0;

/// The wave map: the colour image, contours, the hull footprint drawn opaque, the Kelvin wedge,
/// both wash lines, a scale bar, "Bow →" and the colour bar. Scroll zooms about the cursor, a drag
/// pans (or moves the wash line when it starts on one), a double-click fits. Returns the wash
/// distance while the wash line is dragged.
fn wave_map(
    ui: &mut egui::Ui,
    r: &WaveResult,
    settings: &WaveSettings,
    texture: Option<egui::TextureId>,
    map: &mut MapState,
    size: egui::Vec2,
    tr: Tr,
) -> Option<f64> {
    let size = egui::vec2(size.x, (size.y - COLOUR_BAR).max(60.0));
    let (rect, response) = ui.allocate_exact_size(size, egui::Sense::click_and_drag());
    let f = &r.field;
    let (Some(&x0), Some(&x1), Some(&y0), Some(&y1)) =
        (f.xs.first(), f.xs.last(), f.ys.first(), f.ys.last())
    else {
        return None;
    };
    if response.double_clicked() || !map.fitted || map.zoom <= 0.0 {
        *map = MapState {
            zoom: 1.0,
            pan: egui::Vec2::ZERO,
            fitted: true,
            spacing_plot: map.spacing_plot,
            ..MapState::default()
        };
    }
    let fit = ((rect.width() - 16.0) as f64 / (x1 - x0).max(1e-9))
        .min((rect.height() - 16.0) as f64 / (y1 - y0).max(1e-9)) as f32;
    let mut t = MapTransform {
        centre_world: (0.5 * (x0 + x1), 0.5 * (y0 + y1)),
        centre_screen: rect.center() + map.pan,
        scale: fit * map.zoom,
    };

    // Zoom about the cursor: the point under it stays put.
    if let Some(hover) = response.hover_pos() {
        let scroll = ui.input(|i| i.smooth_scroll_delta.y);
        if scroll != 0.0 {
            let before = t.world(hover);
            map.zoom = (map.zoom * (scroll * 0.0015).exp()).clamp(0.2, 50.0);
            t.scale = fit * map.zoom;
            let after = t.at(before.0, before.1);
            map.pan += hover - after;
            t.centre_screen = rect.center() + map.pan;
        }
    }
    // A release the map never saw (the view closed mid-drag) still ends the drag.
    if map.dragging_wash && !ui.input(|i| i.pointer.any_down()) {
        map.dragging_wash = false;
        map.wash_frame = None;
    }
    if let Some(frame) = map.wash_frame {
        t = frame;
    }
    let d = r.wash_distance_m;
    // Picked on the frame's transform before this frame's pan.
    let pick = t;
    let near_wash = move |p: egui::Pos2| {
        [d, -d]
            .into_iter()
            .any(|y| (pick.at(x0, y).y - p.y).abs() <= WASH_PICK)
    };
    let mut wash = None;
    if response.drag_started() {
        // Where the press began: egui starts a drag only after the pointer has moved a little.
        let origin = ui.input(|i| i.pointer.press_origin());
        map.dragging_wash = origin.is_some_and(near_wash);
        map.wash_frame = map.dragging_wash.then_some(t);
    }
    if response.dragged() {
        if map.dragging_wash {
            if let Some(p) = response.interact_pointer_pos() {
                let (_, y) = t.world(p);
                wash = Some((y.abs() * 10.0).round() / 10.0).filter(|d| *d > 0.0);
            }
        } else {
            map.pan += response.drag_delta();
            t.centre_screen = rect.center() + map.pan;
        }
    }
    if response.drag_stopped() {
        map.dragging_wash = false;
        map.wash_frame = None;
    }
    if map.dragging_wash || response.hover_pos().is_some_and(near_wash) {
        ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeVertical);
    }

    let painter = ui.painter_at(rect);
    painter.rect_filled(rect, 0.0, theme::PLOT_BG);
    // The image's pixels sit on the grid points, so it reaches half a cell past the outer ones.
    let (hx, hy) = (
        0.5 * (x1 - x0) / (f.xs.len().max(2) - 1) as f64,
        0.5 * (y1 - y0) / (f.ys.len().max(2) - 1) as f64,
    );
    if let Some(texture) = texture {
        let quad = egui::Rect::from_two_pos(t.at(x0 - hx, y1 + hy), t.at(x1 + hx, y0 - hy));
        let mut mesh = egui::Mesh::with_texture(texture);
        mesh.add_rect_with_uv(
            quad,
            egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
            egui::Color32::WHITE,
        );
        painter.add(egui::Shape::mesh(mesh));
    }
    for (level, segments) in &r.contours {
        let crest = *level > 0.0;
        let stroke = egui::Stroke::new(
            1.0,
            if crest {
                theme::WAVE_CONTOUR_CREST
            } else {
                theme::WAVE_CONTOUR_TROUGH
            },
        );
        for [(ax, ay), (bx, by)] in segments {
            let (a, b) = (t.at(*ax, *ay), t.at(*bx, *by));
            if crest {
                painter.line_segment([a, b], stroke);
            } else {
                painter.extend(egui::Shape::dashed_line(&[a, b], stroke, 3.0, 2.0));
            }
        }
    }
    // The hull footprint, opaque, so nobody reads waves under the hull: a fan from the
    // outline's centroid (a waterline is star-shaped about it).
    let mut bow = f64::NEG_INFINITY;
    let mut hull_bottom = f32::NEG_INFINITY;
    for outline in &f.waterlines {
        if outline.len() < 3 {
            continue;
        }
        let n = outline.len() as f64;
        let (cx, cy) = outline
            .iter()
            .fold((0.0, 0.0), |(sx, sy), p| (sx + p[0] / n, sy + p[1] / n));
        let mut mesh = egui::Mesh::default();
        mesh.colored_vertex(t.at(cx, cy), theme::WAVE_HULL);
        let points: Vec<egui::Pos2> = outline.iter().map(|p| t.at(p[0], p[1])).collect();
        for p in &points {
            mesh.colored_vertex(*p, theme::WAVE_HULL);
        }
        let k = points.len() as u32;
        for i in 0..k {
            mesh.add_triangle(0, 1 + i, 1 + (i + 1) % k);
        }
        painter.add(egui::Shape::mesh(mesh));
        painter.add(egui::Shape::closed_line(
            points.clone(),
            egui::Stroke::new(1.5, theme::WAVE_WATERLINE),
        ));
        bow = outline.iter().map(|p| p[0]).fold(bow, f64::max);
        hull_bottom = points.iter().map(|p| p.y).fold(hull_bottom, f32::max);
    }
    if settings.kelvin_wedge && bow.is_finite() {
        let reach = (bow - x0).max(0.0);
        let half = KELVIN_HALF_ANGLE_DEG.to_radians().tan() * reach;
        let stroke = egui::Stroke::new(1.0, theme::WAVE_KELVIN);
        for side in [1.0, -1.0] {
            painter.line_segment([t.at(bow, 0.0), t.at(x0, side * half)], stroke);
        }
    }
    let cut = egui::Stroke::new(1.5, theme::WAVE_CUT);
    for y in [d, -d] {
        painter.extend(egui::Shape::dashed_line(
            &[t.at(x0, y), t.at(x1, y)],
            cut,
            8.0,
            5.0,
        ));
    }
    let small = egui::FontId::proportional(11.0);
    let d_s = format!("{d:.1}");
    painter.text(
        t.at(x1, d) + egui::vec2(-6.0, -3.0),
        egui::Align2::RIGHT_BOTTOM,
        tr.fmt(Key::WaveMapWashLabel, &[("d", &d_s)]),
        small.clone(),
        theme::WAVE_CUT,
    );
    if hull_bottom.is_finite() && bow.is_finite() {
        painter.text(
            egui::pos2(t.at(bow, 0.0).x, hull_bottom + 4.0),
            egui::Align2::RIGHT_TOP,
            tr.s(Key::WaveMapFarField),
            small.clone(),
            theme::TEXT_MUTED,
        );
    }
    painter.text(
        rect.right_top() + egui::vec2(-8.0, 6.0),
        egui::Align2::RIGHT_TOP,
        tr.s(Key::WaveMapBow),
        small.clone(),
        theme::TEXT_SECONDARY,
    );
    // Scale bar: a round length near a fifth of the map's width.
    let metres = round_length((0.2 * rect.width() / t.scale) as f64);
    let start = rect.left_bottom() + egui::vec2(10.0, -10.0);
    let end = start + egui::vec2(metres as f32 * t.scale, 0.0);
    let bar = egui::Stroke::new(1.5, theme::TEXT_SECONDARY);
    painter.line_segment([start, end], bar);
    for p in [start, end] {
        painter.line_segment([p, p - egui::vec2(0.0, 4.0)], bar);
    }
    painter.text(
        end + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        format!("{metres} m"),
        small.clone(),
        theme::TEXT_SECONDARY,
    );
    // What is under the cursor, from the nearest grid point.
    if let Some(p) = response.hover_pos() {
        let (x, y) = t.world(p);
        let nx = f.xs.len();
        let ix = (((x - x0) / (2.0 * hx)).round().max(0.0) as usize).min(nx - 1);
        let iy = (((y - y0) / (2.0 * hy)).round().max(0.0) as usize).min(f.ys.len() - 1);
        let k = iy * nx + ix;
        let text = if f.wet[k] {
            let x_s = format!("{x:.1}");
            let y_s = signed(y, 1);
            let z_s = signed(f.zeta[k], 2);
            tr.fmt(
                Key::WaveMapCursorWet,
                &[("x", &x_s), ("y", &y_s), ("z", &z_s)],
            )
        } else {
            tr.s(Key::WaveMapCursorHull).to_string()
        };
        painter.text(
            rect.right_bottom() + egui::vec2(-8.0, -6.0),
            egui::Align2::RIGHT_BOTTOM,
            text,
            small,
            theme::TEXT_SECONDARY,
        );
    }
    colour_bar(ui, r.clip_m, rect.width(), tr);
    wash
}

/// The window's colour map as a bar from trough to crest, labelled −clip, calm and +clip.
fn colour_bar(ui: &mut egui::Ui, clip_m: f64, width: f32, tr: Tr) {
    let w = width.clamp(120.0, 320.0);
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, 8.0), egui::Sense::hover());
    let mut mesh = egui::Mesh::default();
    let stops = [theme::WAVE_TROUGH, theme::WAVE_CALM, theme::WAVE_CREST];
    for (i, colour) in stops.into_iter().enumerate() {
        let x = rect.left() + rect.width() * i as f32 / 2.0;
        mesh.colored_vertex(egui::pos2(x, rect.top()), colour);
        mesh.colored_vertex(egui::pos2(x, rect.bottom()), colour);
    }
    for i in 0..2u32 {
        let k = 2 * i;
        mesh.add_triangle(k, k + 1, k + 2);
        mesh.add_triangle(k + 1, k + 3, k + 2);
    }
    ui.painter().add(egui::Shape::mesh(mesh));
    let neg_s = signed(-clip_m, 2);
    let pos_s = signed(clip_m, 2);
    ui.label(
        egui::RichText::new(tr.fmt(Key::WaveColourBar, &[("neg", &neg_s), ("pos", &pos_s)]))
            .small()
            .color(theme::TEXT_SECONDARY),
    );
}

/// Humps and hollows are marked from this Froude number up: below it C_w ripples with many small
/// extrema whose labels would crowd the plot, and wave drag is small there anyway.
const MARK_FROM_FN: f64 = 0.25;

/// The humps and hollows worth a label: `local_extrema`, with ripples dropped. Neighbouring
/// hump-hollow pairs closer than `frac` of the curve's largest |y| are removed, the closest pair
/// first (on a tie the later one), until every pair left differs by more.
fn prominent_extrema(ys: &[f64], frac: f64) -> (Vec<usize>, Vec<usize>) {
    let (max, min) = seacraft_core::michell::local_extrema(ys);
    let mut ext: Vec<(usize, bool)> = max
        .iter()
        .map(|&i| (i, true))
        .chain(min.iter().map(|&i| (i, false)))
        .collect();
    ext.sort_by_key(|e| e.0);
    let threshold = frac * ys.iter().fold(0.0f64, |m, y| m.max(y.abs()));
    loop {
        let mut closest: Option<(usize, f64)> = None;
        for k in 0..ext.len().saturating_sub(1) {
            let diff = (ys[ext[k].0] - ys[ext[k + 1].0]).abs();
            if closest.is_none_or(|(_, d)| diff <= d) {
                closest = Some((k, diff));
            }
        }
        match closest {
            Some((k, d)) if d < threshold => {
                ext.drain(k..k + 2);
            }
            _ => break,
        }
    }
    let pick = |want: bool| ext.iter().filter(|e| e.1 == want).map(|e| e.0).collect();
    (pick(true), pick(false))
}

/// The right column: wave drag against speed (or, for a catamaran, interference against spacing)
/// and the waves along the wash line. A click on the drag plot sets the speed; on the spacing plot,
/// the spacing.
fn plots(
    ui: &mut egui::Ui,
    r: &WaveResult,
    drag: Option<&WaveDrag>,
    map: &mut MapState,
    tr: Tr,
) -> WaveViewAction {
    use crate::panels::{plot_interactive, PlotMark, PlotSeries};
    let mut action = WaveViewAction::None;
    egui::ScrollArea::vertical()
        .id_salt("wave_plots")
        .show(ui, |ui| {
            let cat = r.cat_spacing_m.is_some();
            if cat {
                theme::segmented(ui, |ui| {
                    ui.selectable_value(&mut map.spacing_plot, false, tr.s(Key::WavePlotVsSpeed));
                    ui.selectable_value(&mut map.spacing_plot, true, tr.s(Key::WavePlotVsSpacing));
                });
            }
            if cat && map.spacing_plot && r.spacing_scan.len() > 1 {
                let (s0, s1) = (
                    r.spacing_scan[0].0,
                    r.spacing_scan[r.spacing_scan.len() - 1].0,
                );
                let series = [
                    PlotSeries {
                        label: tr.s(Key::WavePlotPairRatio).into(),
                        points: r.spacing_scan.clone(),
                        dashed: false,
                        muted: false,
                    },
                    PlotSeries {
                        label: tr.s(Key::WavePlotNoInterference).into(),
                        points: vec![(s0, 1.0), (s1, 1.0)],
                        dashed: true,
                        muted: false,
                    },
                ];
                let fn_s = format!("{:.2}", r.field.froude);
                let caption = tr.fmt(Key::WavePlotSpacingCaption, &[("fn", &fn_s)]);
                if let Some(s) =
                    plot_interactive(ui, &caption, &series, r.cat_spacing_m, &[], Some((s0, s1)))
                {
                    action = WaveViewAction::SetSpacing((s * 20.0).round() / 20.0);
                }
            } else if let Some(d) = drag {
                let curve = |rows: &[seacraft_core::michell::MichellResult]| -> Vec<(f64, f64)> {
                    rows.iter().map(|m| (m.fn_, m.cw * 1000.0)).collect()
                };
                let mono = curve(&d.mono);
                let (series, primary) = match &d.cat {
                    Some(c) => {
                        let pair = curve(c);
                        (
                            vec![
                                PlotSeries {
                                    label: tr.s(Key::WavePlotCatSeries).into(),
                                    points: pair.clone(),
                                    dashed: false,
                                    muted: false,
                                },
                                PlotSeries {
                                    label: tr.s(Key::WavePlotTwoMonoSeries).into(),
                                    points: mono,
                                    dashed: true,
                                    muted: false,
                                },
                            ],
                            pair,
                        )
                    }
                    None => (
                        vec![PlotSeries {
                            label: String::new(),
                            points: mono.clone(),
                            dashed: false,
                            muted: false,
                        }],
                        mono,
                    ),
                };
                let ys: Vec<f64> = primary.iter().map(|p| p.1).collect();
                let (humps, hollows) = prominent_extrema(&ys, 0.05);
                let mut marks = vec![PlotMark {
                    x: HULL_SPEED_FN,
                    label: tr.s(Key::WavePlotHullSpeed),
                }];
                for (indices, label) in [
                    (&humps, tr.s(Key::WavePlotHump)),
                    (&hollows, tr.s(Key::WavePlotHollow)),
                ] {
                    marks.extend(
                        indices
                            .iter()
                            .map(|&i| primary[i].0)
                            .filter(|f| *f >= MARK_FROM_FN)
                            .map(|x| PlotMark { x, label }),
                    );
                }
                let fn_now = r.field.froude;
                if let Some(f) = plot_interactive(
                    ui,
                    tr.s(Key::WavePlotDragCaption),
                    &series,
                    Some(fn_now),
                    &marks,
                    Some(SLIDER_FN),
                ) {
                    let fn_ = (f * 1000.0).round() / 1000.0;
                    action =
                        WaveViewAction::SetSpeed(WaveSpeed::Froude(fn_).m_s(d.length_wl) / KNOT);
                }
                let fn_s = format!("{fn_now:.2}");
                let kn_s = format!("{:.1}", r.field.speed_m_s / KNOT);
                ui.label(
                    egui::RichText::new(
                        tr.fmt(Key::WavePlotDragMarkerHint, &[("fn", &fn_s), ("kn", &kn_s)]),
                    )
                    .small()
                    .color(theme::TEXT_MUTED),
                );
            } else {
                ui.label(
                    egui::RichText::new(tr.s(Key::WaveComputingDrag)).color(theme::TEXT_MUTED),
                );
            }
            ui.add_space(6.0);
            let wash_marks: Vec<PlotMark> = r
                .wash
                .iter()
                .map(|w| PlotMark {
                    x: w.x_m,
                    label: tr.s(Key::WavePlotLargestWave),
                })
                .collect();
            let series = [PlotSeries {
                label: String::new(),
                points: r
                    .wash_cut
                    .s
                    .iter()
                    .copied()
                    .zip(r.wash_cut.zeta.iter().copied())
                    .collect(),
                dashed: false,
                muted: false,
            }];
            let d_s = format!("{:.1}", r.wash_distance_m);
            let caption = tr.fmt(Key::WavePlotWashCaption, &[("d", &d_s)]);
            let range = r
                .wash_cut
                .s
                .first()
                .zip(r.wash_cut.s.last())
                .map(|(a, b)| (*a, *b));
            plot_interactive(ui, &caption, &series, None, &wash_marks, range);
        });
    action
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
    tr: Tr,
) -> WaveViewAction {
    let mut changed = false;
    let mut save = false;
    ui.horizontal_wrapped(|ui| {
        let mut cat = s.cat_spacing_m.is_some();
        if ui
            .checkbox(&mut cat, tr.s(Key::WaveCtlCatamaran))
            .on_hover_text(tr.s(Key::WaveCtlCatamaranTip))
            .changed()
        {
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
                        .prefix(tr.s(Key::WaveCtlSpacingPrefix))
                        .suffix(" m"),
                )
                .on_hover_text(tr.s(Key::WaveCtlSpacingTip))
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
                    .prefix(tr.s(Key::WaveCtlWashLinePrefix))
                    .suffix(" m"),
            )
            .on_hover_text(tr.s(Key::WaveCtlWashLineTip))
            .changed()
        {
            s.wash_distance_m = Some(wash);
            changed = true;
        }
        ui.menu_button(tr.s(Key::WaveMapMenu), |ui| {
            let mut auto = s.domain == WaveDomain::Auto;
            if ui
                .checkbox(&mut auto, tr.s(Key::WaveMapAutoDomain))
                .on_hover_text(tr.s(Key::WaveMapAutoDomainTip))
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
                for (v, label, hover) in [
                    (
                        ahead_m,
                        tr.s(Key::WaveMapAheadPrefix),
                        tr.s(Key::WaveMapAheadTip),
                    ),
                    (
                        behind_m,
                        tr.s(Key::WaveMapBehindPrefix),
                        tr.s(Key::WaveMapBehindTip),
                    ),
                    (
                        side_m,
                        tr.s(Key::WaveMapSidePrefix),
                        tr.s(Key::WaveMapSideTip),
                    ),
                ] {
                    changed |= ui
                        .add(
                            theme::num(v)
                                .speed(0.1)
                                .range(0.0..=10_000.0)
                                .prefix(label)
                                .suffix(" m"),
                        )
                        .on_hover_text(hover)
                        .changed();
                }
            }
            ui.checkbox(&mut s.kelvin_wedge, tr.s(Key::WaveMapKelvinWedge))
                .on_hover_text(tr.s(Key::WaveMapKelvinWedgeTip));
        })
        .response
        .on_hover_text(tr.s(Key::WaveMapMenuTip));
        ui.menu_button(tr.s(Key::WaveColoursMenu), |ui| {
            let mut auto = s.clip_m.is_none();
            if ui
                .checkbox(&mut auto, tr.s(Key::WaveColoursAutoRange))
                .on_hover_text(tr.s(Key::WaveColoursAutoRangeTip))
                .changed()
            {
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
                    .on_hover_text(tr.s(Key::WaveColoursClipTip))
                    .changed();
            }
            changed |= ui
                .add(
                    theme::num(&mut s.contours)
                        .range(0..=12)
                        .prefix(tr.s(Key::WaveColoursContoursPrefix)),
                )
                .on_hover_text(tr.s(Key::WaveColoursContoursTip))
                .changed();
        })
        .response
        .on_hover_text(tr.s(Key::WaveColoursMenuTip));
        ui.checkbox(&mut s.plan_contours, tr.s(Key::WaveCtlPlanContours))
            .on_hover_text(tr.s(Key::WaveCtlPlanContoursTip));
        ui.separator();
        if ui
            .add_enabled(
                current && result.is_some(),
                egui::Button::new(tr.s(Key::WaveCtlSaveImage)),
            )
            .on_hover_text(tr.s(Key::WaveCtlSaveImageTip))
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

    /// Review I2: without a waterline length the speed reads in knots, so "kn" is the unit
    /// selected and "Fn" cannot be picked; with one, the switch follows the setting.
    #[test]
    fn the_unit_switch_shows_the_unit_drawn() {
        assert_eq!(unit_switch(true, None), (false, false));
        assert_eq!(unit_switch(false, None), (false, false));
        assert_eq!(unit_switch(true, Some(10.7)), (true, true));
        assert_eq!(unit_switch(false, Some(10.7)), (false, true));
        assert_eq!(shown_speed(6.0, true, None), WaveSpeed::Knots(6.0));
    }

    #[test]
    fn interference_is_worded() {
        assert_eq!(interference_words(0.88), "favourable (−12 %)");
        assert_eq!(interference_words(1.30), "unfavourable (+30 %)");
        assert_eq!(interference_words(1.004), "neutral (±1 %)");
    }

    /// A ripple smaller than 5 % of the peak is not marked; the main hump and hollow are.
    #[test]
    fn only_prominent_extrema_are_marked() {
        let ys = [0.0, 1.0, 10.0, 6.0, 8.0, 7.9, 8.0, 3.0];
        let (humps, hollows) = prominent_extrema(&ys, 0.05);
        assert_eq!(humps, vec![2, 4]);
        assert_eq!(hollows, vec![3]);
    }

    #[test]
    fn signed_uses_a_true_minus() {
        assert_eq!(signed(0.42, 2), "+0.42");
        assert_eq!(signed(-0.39, 2), "−0.39");
        assert_eq!(signed(-0.0001, 2), "+0.00");
    }
}
