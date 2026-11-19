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
use seacraft_core::wave_pattern::{HULL_SPEED_FN, KELVIN_HALF_ANGLE_DEG};

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
    /// The pointer went down on a wash line: the drag moves the line, not the map.
    pub dragging_wash: bool,
    /// The catamaran's drag plot shows interference against spacing, not drag against speed.
    pub spacing_plot: bool,
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
        texture,
        map,
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
        // The chip sits in the header, so the map does not move when the result goes stale (as
        // it does mid-drag on the wash line).
        if stale && result.is_some() {
            crate::panels::stale_chip(ui);
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
            Some(r) => {
                let asked = ui
                    .scope(|ui| {
                        if stale {
                            ui.multiply_opacity(theme::STALE_OPACITY);
                        }
                        body_columns(ui, r, drag, settings, texture, map, body)
                    })
                    .inner;
                if asked != WaveViewAction::None {
                    action = asked;
                }
            }
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
/// Returns what the map (a dragged wash line) or the plots (a clicked speed or spacing) asked for.
fn body_columns(
    ui: &mut egui::Ui,
    r: &WaveResult,
    drag: Option<&WaveDrag>,
    settings: &WaveSettings,
    texture: Option<egui::TextureId>,
    map: &mut MapState,
    size: egui::Vec2,
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
                .show(ui, |ui| readouts(ui, r, settings));
        });
        ui.allocate_ui_with_layout(egui::vec2(map_w, size.y), top_down, |ui| {
            let map_h = if wide {
                size.y
            } else {
                (0.55 * size.y).round()
            };
            if let Some(d) = wave_map(ui, r, settings, texture, map, egui::vec2(map_w, map_h)) {
                action = WaveViewAction::SetWashDistance(d);
            }
            if !wide {
                let asked = plots(ui, r, drag, settings, map);
                if asked != WaveViewAction::None {
                    action = asked;
                }
            }
        });
        if wide {
            ui.allocate_ui_with_layout(egui::vec2(plots_w, size.y), top_down, |ui| {
                let asked = plots(ui, r, drag, settings, map);
                if asked != WaveViewAction::None {
                    action = asked;
                }
            });
        }
    });
    action
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

/// World (m, model axes) to the map's screen: bow to the right, starboard (+y) up.
#[derive(Debug, Clone, Copy)]
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
            dragging_wash: false,
            spacing_plot: map.spacing_plot,
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
    painter.text(
        t.at(x1, d) + egui::vec2(-6.0, -3.0),
        egui::Align2::RIGHT_BOTTOM,
        format!("wash {d:.1} m"),
        small.clone(),
        theme::WAVE_CUT,
    );
    if hull_bottom.is_finite() && bow.is_finite() {
        painter.text(
            egui::pos2(t.at(bow, 0.0).x, hull_bottom + 4.0),
            egui::Align2::RIGHT_TOP,
            "far field only: not the wave profile on the hull",
            small.clone(),
            theme::TEXT_MUTED,
        );
    }
    painter.text(
        rect.right_top() + egui::vec2(-8.0, 6.0),
        egui::Align2::RIGHT_TOP,
        "Bow →",
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
            format!(
                "x {x:.1} m · y {} m · elevation {} m",
                signed(y, 1),
                signed(f.zeta[k], 2)
            )
        } else {
            "hull".to_string()
        };
        painter.text(
            rect.right_bottom() + egui::vec2(-8.0, -6.0),
            egui::Align2::RIGHT_BOTTOM,
            text,
            small,
            theme::TEXT_SECONDARY,
        );
    }
    colour_bar(ui, r.clip_m, rect.width());
    wash
}

/// The window's colour map as a bar from trough to crest, labelled −clip, calm and +clip.
fn colour_bar(ui: &mut egui::Ui, clip_m: f64, width: f32) {
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
    ui.label(
        egui::RichText::new(format!(
            "{} m · calm · {} m",
            signed(-clip_m, 2),
            signed(clip_m, 2)
        ))
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
    settings: &WaveSettings,
    map: &mut MapState,
) -> WaveViewAction {
    use crate::panels::{plot_interactive, PlotMark, PlotSeries};
    let mut action = WaveViewAction::None;
    egui::ScrollArea::vertical()
        .id_salt("wave_plots")
        .show(ui, |ui| {
            let cat = r.cat_spacing_m.is_some();
            if cat {
                theme::segmented(ui, |ui| {
                    ui.selectable_value(&mut map.spacing_plot, false, "vs speed");
                    ui.selectable_value(&mut map.spacing_plot, true, "vs spacing");
                });
            }
            if cat && map.spacing_plot && r.spacing_scan.len() > 1 {
                let (s0, s1) = (
                    r.spacing_scan[0].0,
                    r.spacing_scan[r.spacing_scan.len() - 1].0,
                );
                let series = [
                    PlotSeries {
                        label: "pair over 2 × one hull".into(),
                        points: r.spacing_scan.clone(),
                        dashed: false,
                    },
                    PlotSeries {
                        label: "no interference".into(),
                        points: vec![(s0, 1.0), (s1, 1.0)],
                        dashed: true,
                    },
                ];
                let caption = format!("Interference vs spacing (m) at Fn {:.2}", r.field.froude);
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
                                    label: "catamaran".into(),
                                    points: pair.clone(),
                                    dashed: false,
                                },
                                PlotSeries {
                                    label: "2 × one hull (no interference)".into(),
                                    points: mono,
                                    dashed: true,
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
                        }],
                        mono,
                    ),
                };
                let ys: Vec<f64> = primary.iter().map(|p| p.1).collect();
                let (humps, hollows) = prominent_extrema(&ys, 0.05);
                let mut marks = vec![PlotMark {
                    x: HULL_SPEED_FN,
                    label: "hull speed",
                }];
                for (indices, label) in [(&humps, "hump"), (&hollows, "hollow")] {
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
                    "Wave drag coefficient C_w × 10³ vs Fn",
                    &series,
                    Some(fn_now),
                    &marks,
                    Some(SLIDER_FN),
                ) {
                    let knots = settings.speed.is_knots();
                    action = WaveViewAction::SetSpeed(
                        WaveSpeed::Froude((f * 1000.0).round() / 1000.0)
                            .in_unit(knots, d.length_wl),
                    );
                }
                ui.label(
                    egui::RichText::new(format!(
                        "marker: Fn {fn_now:.2} = {:.1} kn; click or drag to set the speed",
                        r.field.speed_m_s / KNOT
                    ))
                    .small()
                    .color(theme::TEXT_MUTED),
                );
            } else {
                ui.label(
                    egui::RichText::new("Computing the wave drag curve…").color(theme::TEXT_MUTED),
                );
            }
            ui.add_space(6.0);
            let wash_marks: Vec<PlotMark> = r
                .wash
                .iter()
                .map(|w| PlotMark {
                    x: w.x_m,
                    label: "largest wave",
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
            }];
            let caption = format!("Wash at {:.1} m: elevation (m) vs x (m)", r.wash_distance_m);
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
