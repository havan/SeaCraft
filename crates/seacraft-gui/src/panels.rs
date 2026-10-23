//! Side docks: the layer manager and the hydrostatics report.

use eframe::egui;
use seacraft_core::damage::{Compartment, CompartmentSpec, DamageCase, DamageResult};
use seacraft_core::hydro::{HydroOptions, Hydrostatics, SectionArea};
use seacraft_core::loading::{LoadingCondition, LoadingResult, Tank, WeightItem};
use seacraft_core::project::Project;
use seacraft_core::stability::CrossCurve;
use seacraft_core::weather::{WeatherResult, WindageItem, WindageRect};
use seacraft_core::Surface;

/// Draw one row per layer. Returns whether any flag or colour changed.
///
/// Hiding every layer is allowed: the viewports then show no hull and the
/// status bar says so, which is easier to undo than a refusal.
pub fn layers(ui: &mut egui::Ui, surface: &mut Surface) -> bool {
    let mut changed = false;
    ui.heading("Layers");
    ui.add_space(4.0);
    let mut counts = vec![0usize; surface.layers.len()];
    for &l in &surface.face_layer {
        if let Some(c) = counts.get_mut(l) {
            *c += 1;
        }
    }
    for (index, layer) in surface.layers.iter_mut().enumerate() {
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                let mut rgb = [
                    ((layer.color >> 16) & 0xff) as u8,
                    ((layer.color >> 8) & 0xff) as u8,
                    (layer.color & 0xff) as u8,
                ];
                if ui.color_edit_button_srgb(&mut rgb).changed() {
                    layer.color = ((rgb[0] as i32) << 16) | ((rgb[1] as i32) << 8) | rgb[2] as i32;
                    changed = true;
                }
                if ui
                    .add(
                        egui::TextEdit::singleline(&mut layer.name)
                            .desired_width(ui.available_width()),
                    )
                    .changed()
                {
                    changed = true;
                }
            });
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 12.0;
                changed |= ui.checkbox(&mut layer.visible, "visible").changed();
                changed |= ui.checkbox(&mut layer.symmetric, "symmetric").changed();
            });
            changed |= ui
                .checkbox(&mut layer.use_in_hydrostatics, "in hydrostatics")
                .changed();
            ui.horizontal(|ui| {
                ui.small(format!(
                    "{} faces · id {} · {:+.3} m plate",
                    counts.get(index).copied().unwrap_or(0),
                    layer.id,
                    layer.thickness
                ));
            });
        });
    }
    if counts.iter().all(|&c| c == 0) {
        ui.separator();
        ui.label("the model has no faces");
    }
    if !surface.layers.iter().any(|l| l.visible) {
        ui.separator();
        ui.colored_label(
            egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
            "no visible layers",
        );
    }
    changed
}

/// What the user asked the hydrostatics dock to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydroAction {
    None,
    Recompute,
    CrossCurves,
}

/// Everything the hydrostatics dock draws.
pub struct HydroPanel<'a> {
    pub options: &'a mut HydroOptions,
    pub kg: &'a mut f64,
    /// Solve the GZ curves free to trim about `lcg` instead of holding `trim`.
    pub free_trim: &'a mut bool,
    /// Longitudinal centre of gravity for the free-trim solve.
    pub lcg: &'a mut Option<f64>,
    pub result: Option<&'a Hydrostatics>,
    /// The geometry changed and the report has not been rebuilt yet.
    pub stale: bool,
    pub curves: Option<&'a [CrossCurve]>,
}

fn row(ui: &mut egui::Ui, label: &str, value: String) {
    ui.horizontal(|ui| {
        ui.small(label);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.monospace(value);
        });
    });
}

/// The measurement rows the dock draws, in two columns.
///
/// Kept separate from the drawing so the numbers (and their formatting) can be
/// tested without a frame: `TPC` and `MTC` are table quantities computed here
/// from the measured waterplane, with the kilogram-to-tonne factor the core's
/// kg/m³ density needs.
pub fn hydrostatic_rows(h: &Hydrostatics, kg: f64) -> [Vec<(&'static str, String)>; 2] {
    let rho_t = h.water_density / 1000.0;
    let tpc = h.waterplane_area_m2 * rho_t / 100.0;
    let mtc = if h.lwl > 1e-9 {
        rho_t * h.waterplane_inertia_longitudinal / (100.0 * h.lwl)
    } else {
        0.0
    };
    [
        vec![
            ("volume", format!("{:.3} m³", h.volume_m3)),
            ("displacement", format!("{:.2} t", h.displacement_t)),
            ("Cb", format!("{:.3}", h.block_coefficient)),
            ("Cp", format!("{:.3}", h.prismatic_coefficient)),
            ("Cm", format!("{:.3}", h.midship_coefficient)),
            ("Cwp", format!("{:.3}", h.waterplane_coefficient)),
            ("LCB", format!("{:.3} m", h.lcb.x)),
            ("LCF", format!("{:.3} m", h.lcf.x)),
            ("TPC", format!("{:.3} t/cm", tpc)),
        ],
        vec![
            ("KB", format!("{:.3} m", h.kb)),
            ("BM(t)", format!("{:.3} m", h.bm_transverse)),
            ("KM(t)", format!("{:.3} m", h.km_transverse)),
            ("GM", format!("{:+.3} m", h.gm(kg))),
            ("wetted", format!("{:.2} m²", h.wetted_surface_m2)),
            ("waterplane", format!("{:.2} m²", h.waterplane_area_m2)),
            ("LWL / BWL", format!("{:.2} / {:.2} m", h.lwl, h.bwl)),
            ("midship", format!("{:.3} m²", h.mainframe_area_m2)),
            ("MTC", format!("{:.3} t·m/cm", mtc)),
        ],
    ]
}

/// The sectional area curve as screen coordinates, inside `rect`.
///
/// Pure so the plot can be checked without a frame: the polyline must run
/// left to right, stay inside the rectangle, and put zero area on the baseline
/// and the maximum area at the top.
pub fn sac_polyline(sac: &[SectionArea], rect: egui::Rect) -> Vec<egui::Pos2> {
    let x0 = sac.first().map(|s| s.x).unwrap_or(0.0);
    let x1 = sac.last().map(|s| s.x).unwrap_or(1.0);
    let a_max = sac.iter().fold(0.0_f64, |m, s| m.max(s.area));
    let (x0, x1, a_max) = if (x1 - x0).abs() < 1e-9 || a_max <= 0.0 {
        return Vec::new();
    } else {
        (x0, x1, a_max)
    };
    sac.iter()
        .filter(|s| s.area.is_finite())
        .map(|s| {
            let tx = ((s.x - x0) / (x1 - x0)) as f32;
            let ty = (s.area / a_max).clamp(0.0, 1.0) as f32;
            egui::pos2(
                rect.left() + tx.clamp(0.0, 1.0) * rect.width(),
                rect.bottom() - ty * rect.height(),
            )
        })
        .collect()
}

/// The sectional area curve, drawn small under the readout.
fn plot_sac(ui: &mut egui::Ui, h: &Hydrostatics) {
    let height = 96.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = response.rect.shrink2(egui::vec2(2.0, 6.0));
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x0f, 0x14, 0x1b));
    let points = sac_polyline(&h.sac, rect);
    if points.len() < 2 {
        return;
    }
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x42, 0x52)),
    );
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.4, egui::Color32::from_rgb(0x5c, 0xa8, 0xf0)),
    ));
    let a_max = h.sac.iter().fold(0.0_f64, |m, s| m.max(s.area));
    painter.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        format!(
            "SAC · A max {a_max:.2} m² · x {:.2}..{:.2} m",
            h.model_min.x, h.model_max.x
        ),
        egui::FontId::monospace(10.0),
        egui::Color32::from_rgb(0x8e, 0x9e, 0xae),
    );
}

/// Draft, trim, heel and KG controls plus the measurement readout.
pub fn hydrostatics(ui: &mut egui::Ui, panel: HydroPanel<'_>) -> HydroAction {
    let mut action = HydroAction::None;
    ui.heading("Hydrostatics");
    ui.add_space(4.0);

    let mut changed = false;
    egui::Grid::new("floating_condition")
        .num_columns(2)
        .min_col_width(56.0)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Draft");
            changed |= ui
                .add_sized(
                    [96.0, 20.0],
                    egui::DragValue::new(&mut panel.options.draft)
                        .speed(0.01)
                        .range(0.0..=20.0)
                        .fixed_decimals(2)
                        .suffix(" m"),
                )
                .changed();
            ui.end_row();
            ui.label("Trim");
            changed |= ui
                .add_sized(
                    [96.0, 20.0],
                    egui::DragValue::new(&mut panel.options.trim)
                        .speed(0.01)
                        .range(-5.0..=5.0)
                        .fixed_decimals(2)
                        .suffix(" m"),
                )
                .changed();
            ui.end_row();
            ui.label("Heel");
            changed |= ui
                .add_sized(
                    [96.0, 20.0],
                    egui::DragValue::new(&mut panel.options.heel_deg)
                        .speed(0.2)
                        .range(-60.0..=60.0)
                        .fixed_decimals(1)
                        .suffix(" °"),
                )
                .changed();
            ui.end_row();
            ui.label("KG");
            changed |= ui
                .add_sized(
                    [96.0, 20.0],
                    egui::DragValue::new(panel.kg)
                        .speed(0.02)
                        .range(0.0..=10.0)
                        .fixed_decimals(2)
                        .suffix(" m"),
                )
                .changed();
            ui.end_row();
        });
    ui.add_space(4.0);
    // Free trim: the hull trims to keep the LCB on the LCG while it heels.
    // Enabling it without an LCG takes the measured one, which is the natural
    // choice for a model that carries no mass distribution.
    ui.horizontal(|ui| {
        let mut on = *panel.free_trim;
        if ui.checkbox(&mut on, "Free to trim").changed() {
            *panel.free_trim = on;
            if on && panel.lcg.is_none() {
                *panel.lcg = panel.result.map(|r| r.lcb.x);
            }
            changed = true;
        }
        if *panel.free_trim {
            let mut x = panel.lcg.unwrap_or_default();
            if ui
                .add(
                    egui::DragValue::new(&mut x)
                        .speed(0.01)
                        .range(-50.0..=50.0)
                        .fixed_decimals(3)
                        .prefix("LCG ")
                        .suffix(" m"),
                )
                .changed()
            {
                *panel.lcg = Some(x);
                changed = true;
            }
        }
    });
    if changed {
        action = HydroAction::Recompute;
    }
    if panel.stale {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.small("recomputing…");
        });
    }
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        if ui.button("Recompute").clicked() {
            action = HydroAction::Recompute;
        }
        if ui.button("GZ curves").clicked() {
            action = HydroAction::CrossCurves;
        }
    });
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);

    match panel.result {
        Some(h) if h.volume_m3 > 0.0 => {
            let columns = hydrostatic_rows(h, *panel.kg);
            ui.columns(2, |cols| {
                for (col, rows) in cols.iter_mut().zip(columns.iter()) {
                    col.spacing_mut().item_spacing.y = 4.0;
                    for (label, value) in rows {
                        row(col, label, value.clone());
                    }
                }
            });
            ui.add_space(2.0);
            plot_sac(ui, h);
            for warning in &h.warnings {
                ui.colored_label(egui::Color32::from_rgb(0xe0, 0x5c, 0x5c), &warning.message);
            }
            if h.gm(*panel.kg) < 0.0 {
                ui.colored_label(
                    egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
                    "negative GM: the hull is unstable upright",
                );
            }
        }
        Some(_) => {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                "nothing is submerged at this draft",
            );
        }
        None => {
            ui.small("no measurement yet");
        }
    }

    if let Some(curves) = panel.curves {
        ui.separator();
        plot_cross_curves(ui, curves);
        if let Some(imo) = seacraft_core::stability::evaluate_imo_criteria(curves, 2) {
            ui.add_space(4.0);
            let badge_color = if imo.passed {
                egui::Color32::from_rgb(0x5a, 0xd0, 0x6a)
            } else {
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a)
            };
            ui.horizontal(|ui| {
                ui.small("IMO A.749 (design disp.):");
                ui.colored_label(badge_color, if imo.passed { "PASS" } else { "FAIL" });
            });
            ui.columns(2, |cols| {
                cols[0].spacing_mut().item_spacing.y = 2.0;
                row(
                    &mut cols[0],
                    "A(0–30°)",
                    format!("{:.3} m·rad", imo.area_0_30_m_rad),
                );
                row(
                    &mut cols[0],
                    "A(0–40°)",
                    format!("{:.3} m·rad", imo.area_0_40_m_rad),
                );
                row(
                    &mut cols[0],
                    "A(30–40°)",
                    format!("{:.3} m·rad", imo.area_30_40_m_rad),
                );
                cols[1].spacing_mut().item_spacing.y = 2.0;
                row(
                    &mut cols[1],
                    "GZ(≥30°)",
                    format!("{:.3} m", imo.max_gz_30_plus_m),
                );
                row(
                    &mut cols[1],
                    "φ(GZmax)",
                    format!("{:.0}°", imo.angle_of_max_gz_deg),
                );
                row(&mut cols[1], "GM0", format!("{:.3} m", imo.gm0_m));
            });
        }
    }
    action
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceAction {
    None,
    Changed,
    Compute,
}

pub struct PerformancePanel<'a> {
    pub state: &'a mut crate::app::PerformanceState,
    pub default_kg: f64,
    pub kg_source: &'static str,
    pub busy_resistance: bool,
    pub busy_raos: bool,
}

pub fn performance(ui: &mut egui::Ui, panel: PerformancePanel<'_>) -> PerformanceAction {
    let mut action = PerformanceAction::None;
    ui.heading("Performance");
    ui.add_space(4.0);

    let is_busy = panel.busy_resistance || panel.busy_raos;
    let is_stale = panel.state.resistance_stale || panel.state.raos_stale;

    ui.horizontal(|ui| {
        let btn = ui.add_enabled(!is_busy, egui::Button::new("Compute"));
        if btn.clicked() {
            action = PerformanceAction::Compute;
        }
        if is_busy {
            ui.spinner();
            if panel.busy_resistance && panel.busy_raos {
                ui.label("Computing resistance & RAOs…");
            } else if panel.busy_resistance {
                ui.label("Computing resistance…");
            } else {
                ui.label("Computing RAOs…");
            }
        } else if is_stale {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                "Results out of date",
            );
        }
    });

    ui.add_space(4.0);
    ui.separator();
    ui.strong("Resistance Prediction");
    ui.add_space(2.0);

    egui::ComboBox::from_label("Method")
        .selected_text(panel.state.method.label())
        .show_ui(ui, |ui| {
            for m in [
                crate::app::ResistanceMethod::Holtrop,
                crate::app::ResistanceMethod::Dsyhs,
                crate::app::ResistanceMethod::Savitsky,
                crate::app::ResistanceMethod::Michell,
            ] {
                if ui
                    .selectable_value(&mut panel.state.method, m, m.label())
                    .clicked()
                {
                    action = PerformanceAction::Changed;
                }
            }
        });

    ui.horizontal(|ui| {
        ui.label("Speeds (kn):");
        let r1 = ui.add(
            egui::DragValue::new(&mut panel.state.speeds_kn.0)
                .speed(0.5)
                .range(0.1..=100.0)
                .prefix("min: "),
        );
        let r2 = ui.add(
            egui::DragValue::new(&mut panel.state.speeds_kn.1)
                .speed(0.5)
                .range(0.1..=100.0)
                .prefix("max: "),
        );
        let r3 = ui.add(
            egui::DragValue::new(&mut panel.state.speeds_kn.2)
                .speed(0.1)
                .range(0.1..=20.0)
                .prefix("step: "),
        );
        if r1.changed() || r2.changed() || r3.changed() {
            action = PerformanceAction::Changed;
        }
    });

    if let Some(res) = &panel.state.resistance {
        for w in &res.warnings {
            ui.colored_label(egui::Color32::from_rgb(0xe8, 0x9d, 0x3a), format!("⚠ {w}"));
        }

        let series = vec![(
            res.method.clone(),
            res.rows.iter().map(|r| (r.speed_kn, r.rt_kn)).collect(),
        )];
        plot_xy(ui, "Resistance", "Speed (kn)", &res.rt_label, &series);

        ui.add_space(2.0);
        egui::Grid::new("resistance_table")
            .striped(true)
            .min_col_width(50.0)
            .show(ui, |ui| {
                ui.label(egui::RichText::new("V (kn)").strong());
                ui.label(egui::RichText::new("Fn").strong());
                ui.label(egui::RichText::new(&res.rt_label).strong());
                ui.label(egui::RichText::new("P_E (kW)").strong());
                ui.end_row();
                for r in &res.rows {
                    ui.label(format!("{:.2}", r.speed_kn));
                    ui.label(format!("{:.3}", r.fn_));
                    ui.label(format!("{:.2}", r.rt_kn));
                    match r.pe_kw {
                        Some(pe) => ui.label(format!("{:.2}", pe)),
                        None => ui.label("-"),
                    };
                    ui.end_row();
                }
            });
    }

    ui.add_space(4.0);
    ui.separator();
    ui.strong("Seakeeping (Head Sea RAOs)");
    ui.add_space(2.0);

    let speed_resp = ui.add(
        egui::Slider::new(&mut panel.state.sk_speed_kn, 0.0..=20.0)
            .text("Speed (kn)")
            .step_by(0.5),
    );
    if speed_resp.changed() {
        action = PerformanceAction::Changed;
    }

    ui.horizontal(|ui| {
        let mut cur_kg = panel.state.kg.unwrap_or(panel.default_kg);
        let kg_resp = ui.add(
            egui::DragValue::new(&mut cur_kg)
                .speed(0.01)
                .range(0.0..=50.0)
                .prefix("KG: ")
                .suffix(" m"),
        );
        if kg_resp.changed() {
            panel.state.kg = Some(cur_kg);
            action = PerformanceAction::Changed;
        }
        if panel.state.kg.is_some() {
            if ui.small_button("Reset").clicked() {
                panel.state.kg = None;
                action = PerformanceAction::Changed;
            }
            ui.label(format!(
                "(default: {:.2} m, {})",
                panel.default_kg, panel.kg_source
            ));
        } else {
            ui.label(format!("({})", panel.kg_source));
        }
    });

    ui.horizontal(|ui| {
        let kyy_resp = ui.add(
            egui::DragValue::new(&mut panel.state.kyy_frac)
                .speed(0.01)
                .range(0.05..=1.0)
                .prefix("kyy / Lwl: "),
        );
        let pts_resp = ui.add(
            egui::DragValue::new(&mut panel.state.points)
                .range(5..=200)
                .prefix("Points: "),
        );
        if kyy_resp.changed() || pts_resp.changed() {
            action = PerformanceAction::Changed;
        }
    });

    if let Some(rep) = &panel.state.raos {
        let tn_heave = if rep.natural_heave_omega > 1e-4 {
            format!(
                "{:.2} s",
                2.0 * std::f64::consts::PI / rep.natural_heave_omega
            )
        } else {
            "-".to_string()
        };
        let tn_pitch = if rep.natural_pitch_omega > 1e-4 {
            format!(
                "{:.2} s",
                2.0 * std::f64::consts::PI / rep.natural_pitch_omega
            )
        } else {
            "-".to_string()
        };
        ui.label(format!(
            "Natural periods: Heave T_n = {}, Pitch T_n = {}",
            tn_heave, tn_pitch
        ));

        for w in &rep.warnings {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                format!("⚠ {}", w.message),
            );
        }

        // A sea-state run puts the RAOs on the spectrum's ω grid, which can reach
        // λ/L > 100; past a few hull lengths both curves are flat at 1 and only
        // squash the interesting part, so the plot stops at λ/L = 5.
        const PLOT_LAMBDA_MAX: f64 = 5.0;
        let shown = rep
            .frequencies
            .iter()
            .filter(|p| p.lambda_over_l <= PLOT_LAMBDA_MAX)
            .count();
        let in_plot = |p: &&seacraft_core::seakeeping::RaoPoint| {
            shown < 3 || p.lambda_over_l <= PLOT_LAMBDA_MAX
        };
        let heave_pts: Vec<(f64, f64)> = rep
            .frequencies
            .iter()
            .filter(in_plot)
            .map(|p| (p.lambda_over_l, p.heave_amp))
            .collect();
        let pitch_pts: Vec<(f64, f64)> = rep
            .frequencies
            .iter()
            .filter(in_plot)
            .map(|p| (p.lambda_over_l, p.pitch_amp))
            .collect();
        let series = vec![
            ("Heave z/ζ".to_string(), heave_pts),
            ("Pitch θ/kζ".to_string(), pitch_pts),
        ];
        plot_xy(ui, "Motion RAOs", "λ / L", "RAO", &series);
    }

    ui.add_space(4.0);
    ui.separator();
    if sea_state_section(ui, panel.state) {
        action = PerformanceAction::Changed;
    }

    action
}

/// The Performance tab's "Sea state" section: spectrum input, response points and
/// (after Compute) the significant-response table with MSI. Returns whether an input
/// changed (the results are then out of date).
fn sea_state_section(ui: &mut egui::Ui, state: &mut crate::app::PerformanceState) -> bool {
    use crate::app::{SeaPoint, SeaPointPos, SeaSpectrumKind, SeaStateInput};
    let mut changed = false;
    ui.strong("Sea state (irregular head seas)");
    ui.add_space(2.0);

    let input = &mut state.sea_input;
    if ui
        .checkbox(&mut input.enabled, "Compute sea-state responses")
        .on_hover_text(
            "Computes the RAOs on the spectrum's frequency grid (0.2–5 ωp) with the next \
             Compute, so the RAO plot above then covers that range.",
        )
        .changed()
    {
        changed = true;
    }

    if input.enabled {
        egui::ComboBox::from_label("Spectrum")
            .selected_text(input.kind.label())
            .show_ui(ui, |ui| {
                for k in [SeaSpectrumKind::Bretschneider, SeaSpectrumKind::Jonswap] {
                    if ui.selectable_value(&mut input.kind, k, k.label()).clicked() {
                        changed = true;
                    }
                }
            });
        ui.horizontal(|ui| {
            let hs = ui.add(
                egui::DragValue::new(&mut input.hs)
                    .speed(0.05)
                    .range(0.05..=25.0)
                    .prefix("Hs: ")
                    .suffix(" m"),
            );
            let tp = ui.add(
                egui::DragValue::new(&mut input.tp)
                    .speed(0.1)
                    .range(1.0..=30.0)
                    .prefix("Tp: ")
                    .suffix(" s"),
            );
            if hs.changed() || tp.changed() {
                changed = true;
            }
            if input.kind == SeaSpectrumKind::Jonswap
                && ui
                    .add(
                        egui::DragValue::new(&mut input.gamma)
                            .speed(0.05)
                            .range(1.0..=10.0)
                            .prefix("γ: "),
                    )
                    .changed()
            {
                changed = true;
            }
        });

        ui.label("Response points:");
        let mut remove = None;
        for (i, p) in input.points.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                if ui
                    .add(egui::TextEdit::singleline(&mut p.name).desired_width(80.0))
                    .changed()
                {
                    changed = true;
                }
                match &mut p.pos {
                    SeaPointPos::Lcg => {
                        ui.label("at the LCG");
                    }
                    SeaPointPos::Bow => {
                        ui.label("at the forward end of the waterline");
                    }
                    SeaPointPos::Model(x) => {
                        if ui
                            .add(
                                egui::DragValue::new(x)
                                    .speed(0.05)
                                    .range(-1000.0..=1000.0)
                                    .prefix("x: ")
                                    .suffix(" m"),
                            )
                            .changed()
                        {
                            changed = true;
                        }
                    }
                }
                if ui.small_button("🗑").on_hover_text("Remove point").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            input.points.remove(i);
            changed = true;
        }
        ui.horizontal(|ui| {
            if input.points.len() < 16 && ui.small_button("+ Point").clicked() {
                let n = input.points.len() + 1;
                input.points.push(SeaPoint {
                    name: format!("P{n}"),
                    pos: SeaPointPos::Model(0.0),
                });
                changed = true;
            }
            if ui.small_button("Reset points").clicked() {
                input.points = SeaStateInput::default_points();
                changed = true;
            }
        });
        if let Some(problem) = input.problem() {
            ui.colored_label(
                egui::Color32::from_rgb(0xd0, 0x40, 0x40),
                format!("⚠ {problem}"),
            );
        }
    }

    let stale = state.raos_stale;
    if let Some(r) = &state.sea_result {
        ui.add_space(4.0);
        if stale {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                "Sea-state results out of date",
            );
        }
        let sea = match r.sea {
            seacraft_core::seastate::WaveSpectrum::Bretschneider { hs, tp } => {
                format!("Bretschneider Hs {hs:.2} m, Tp {tp:.2} s")
            }
            seacraft_core::seastate::WaveSpectrum::Jonswap { hs, tp, gamma } => {
                format!("JONSWAP Hs {hs:.2} m, Tp {tp:.2} s, γ {gamma:.2}")
            }
        };
        ui.label(format!(
            "{sea} at {:.1} kn: Hs check {:.3} m, T1 {:.2} s, Tz {:.2} s",
            r.speed_kn, r.spectrum.hs_check, r.spectrum.t1_s, r.spectrum.tz_s
        ));
        for w in &r.warnings {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                format!("⚠ {}", w.message),
            );
        }
        egui::Grid::new("sea_global_table")
            .striped(true)
            .min_col_width(50.0)
            .show(ui, |ui| {
                for h in ["Response", "Significant", "RMS", "Tz (s)"] {
                    ui.label(egui::RichText::new(h).strong());
                }
                ui.end_row();
                for (name, st) in [("Heave (m)", &r.heave), ("Pitch (deg)", &r.pitch_deg)] {
                    ui.label(name);
                    ui.label(format!("{:.3}", st.significant));
                    ui.label(format!("{:.3}", st.rms));
                    ui.label(format!("{:.2}", st.tz_s));
                    ui.end_row();
                }
            });
        ui.add_space(2.0);
        egui::Grid::new("sea_points_table")
            .striped(true)
            .min_col_width(50.0)
            .show(ui, |ui| {
                for h in [
                    "Point",
                    "x (m)",
                    "Motion sig. (m)",
                    "Relative sig. (m)",
                    "Accel. RMS (g)",
                    "MSI (%)",
                ] {
                    ui.label(egui::RichText::new(h).strong());
                }
                ui.end_row();
                for p in &r.points {
                    ui.label(&p.name);
                    ui.label(format!("{:.2}", p.x));
                    ui.label(format!("{:.3}", p.vertical_motion.significant));
                    ui.label(format!("{:.3}", p.relative_motion.significant));
                    ui.label(format!("{:.4}", p.acceleration_rms_g));
                    ui.label(format!("{:.1}", p.msi_percent));
                    ui.end_row();
                }
            });
        ui.label(
            "MSI: share of unadapted people expected to vomit within a 2-hour exposure \
             (O'Hanlon & McCauley 1974).",
        );
    }
    changed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DamageAction {
    None,
    Changed,
    Compute,
}

pub struct DamagePanel<'a> {
    pub project: &'a mut Project,
    pub selected: &'a mut usize,
    pub result: Option<&'a DamageResult>,
    pub stale: bool,
    pub busy: bool,
    pub highlight: Option<usize>,
    pub scroll_to: bool,
}

fn unique_case_name(base: &str, existing: &[DamageCase]) -> String {
    let base_trimmed = base.trim();
    let base_name = if base_trimmed.is_empty() {
        "Damage case"
    } else {
        base_trimmed
    };
    if !existing.iter().any(|c| c.name == base_name) {
        return base_name.to_string();
    }
    let mut counter = 2;
    loop {
        let name = format!("{base_name} ({counter})");
        if !existing.iter().any(|c| c.name == name) {
            return name;
        }
        counter += 1;
    }
}

fn unique_compartment_name(base: &str, existing: &[Compartment]) -> String {
    let base_trimmed = base.trim();
    let base_name = if base_trimmed.is_empty() {
        "Compartment"
    } else {
        base_trimmed
    };
    if !existing.iter().any(|c| c.name == base_name) {
        return base_name.to_string();
    }
    let mut counter = 2;
    loop {
        let name = format!("{base_name} ({counter})");
        if !existing.iter().any(|c| c.name == name) {
            return name;
        }
        counter += 1;
    }
}

pub fn damage(ui: &mut egui::Ui, panel: DamagePanel<'_>) -> DamageAction {
    let mut action = DamageAction::None;
    ui.heading("Damage Stability");
    ui.add_space(4.0);

    // Compartments section
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Compartments").strong());
        if ui.button("+ Add compartment").clicked() {
            let n = panel.project.compartments.len() + 1;
            let name =
                unique_compartment_name(&format!("Compartment {n}"), &panel.project.compartments);
            panel.project.compartments.push(Compartment {
                name,
                x_min: 0.0,
                x_max: 1.0,
                y_min: -0.5,
                y_max: 0.5,
                z_min: 0.0,
                z_max: 1.0,
                permeability: 0.95,
                clip_to_hull: true,
            });
            action = DamageAction::Changed;
        }
    });

    let mut remove_comp = None;
    for (c_idx, comp) in panel.project.compartments.iter_mut().enumerate() {
        let is_highlighted = panel.highlight == Some(c_idx);
        let mut frame = egui::Frame::group(ui.style());
        if is_highlighted {
            frame = frame.stroke(egui::Stroke::new(2.0, egui::Color32::from_rgb(220, 60, 60)));
        }
        let frame_resp = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                if ui.text_edit_singleline(&mut comp.name).changed() {
                    action = DamageAction::Changed;
                }
                if ui
                    .checkbox(&mut comp.clip_to_hull, "Clip to hull")
                    .changed()
                {
                    action = DamageAction::Changed;
                }
                if ui.button("🗑").on_hover_text("Remove compartment").clicked() {
                    remove_comp = Some(c_idx);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Permeability:");
                if ui
                    .add(
                        egui::DragValue::new(&mut comp.permeability)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed()
                {
                    action = DamageAction::Changed;
                }
            });
            egui::CollapsingHeader::new("Bounds [x, y, z]")
                .id_salt(("comp_bounds_header", c_idx))
                .default_open(false)
                .show(ui, |ui| {
                    egui::Grid::new(format!("comp_bounds_{c_idx}"))
                        .num_columns(3)
                        .show(ui, |ui| {
                            ui.label("X range (m):");
                            if ui
                                .add(egui::DragValue::new(&mut comp.x_min).speed(0.01))
                                .changed()
                            {
                                action = DamageAction::Changed;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut comp.x_max).speed(0.01))
                                .changed()
                            {
                                action = DamageAction::Changed;
                            }
                            ui.end_row();

                            ui.label("Y range (m):");
                            if ui
                                .add(egui::DragValue::new(&mut comp.y_min).speed(0.01))
                                .changed()
                            {
                                action = DamageAction::Changed;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut comp.y_max).speed(0.01))
                                .changed()
                            {
                                action = DamageAction::Changed;
                            }
                            ui.end_row();

                            ui.label("Z range (m):");
                            if ui
                                .add(egui::DragValue::new(&mut comp.z_min).speed(0.01))
                                .changed()
                            {
                                action = DamageAction::Changed;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut comp.z_max).speed(0.01))
                                .changed()
                            {
                                action = DamageAction::Changed;
                            }
                            ui.end_row();
                        });
                });
        });
        if is_highlighted && panel.scroll_to {
            frame_resp.response.scroll_to_me(Some(egui::Align::Center));
        }
    }
    if let Some(i) = remove_comp {
        let removed_name = panel.project.compartments.remove(i).name;
        for case in &mut panel.project.damage_cases {
            case.compartments.retain(|spec| match spec {
                CompartmentSpec::Named(n) => n != &removed_name,
                CompartmentSpec::Inline(_) => true,
            });
        }
        action = DamageAction::Changed;
    }

    // Damage case picker and New / Duplicate / Delete buttons
    ui.separator();
    ui.heading("Damage Cases");
    ui.add_space(4.0);

    let mut delete_case = false;
    let mut duplicate_case = false;
    let mut new_case = false;

    if panel.project.damage_cases.is_empty() {
        ui.horizontal(|ui| {
            ui.label("No damage cases in project.");
            if ui.button("New").clicked() {
                new_case = true;
            }
        });
    } else {
        if *panel.selected >= panel.project.damage_cases.len() {
            *panel.selected = panel.project.damage_cases.len() - 1;
        }
        let current_name = panel.project.damage_cases[*panel.selected].name.clone();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("damage_case_picker")
                .selected_text(&current_name)
                .show_ui(ui, |ui| {
                    for (idx, case) in panel.project.damage_cases.iter().enumerate() {
                        if ui
                            .selectable_label(*panel.selected == idx, &case.name)
                            .clicked()
                        {
                            *panel.selected = idx;
                            action = DamageAction::Changed;
                        }
                    }
                });
            if ui.button("New").clicked() {
                new_case = true;
            }
            if ui.button("Duplicate").clicked() {
                duplicate_case = true;
            }
            if ui.button("Delete").clicked() {
                delete_case = true;
            }
        });
    }

    if new_case {
        let name = unique_case_name("Damage case 1", &panel.project.damage_cases);
        panel.project.damage_cases.push(DamageCase {
            name,
            compartments: Vec::new(),
            kg: Some(1.0),
            displacement_t: Some(10.0),
            lcg: Some(0.0),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: None,
        });
        *panel.selected = panel.project.damage_cases.len() - 1;
        return DamageAction::Changed;
    }

    if duplicate_case && !panel.project.damage_cases.is_empty() {
        let orig = &panel.project.damage_cases[*panel.selected];
        let mut dup = orig.clone();
        dup.name = unique_case_name(&orig.name, &panel.project.damage_cases);
        panel.project.damage_cases.push(dup);
        *panel.selected = panel.project.damage_cases.len() - 1;
        return DamageAction::Changed;
    }

    if delete_case && !panel.project.damage_cases.is_empty() {
        panel.project.damage_cases.remove(*panel.selected);
        if *panel.selected >= panel.project.damage_cases.len() && *panel.selected > 0 {
            *panel.selected -= 1;
        }
        return DamageAction::Changed;
    }

    if panel.project.damage_cases.is_empty() {
        return action;
    }

    let case = &mut panel.project.damage_cases[*panel.selected];

    // Editable name
    ui.horizontal(|ui| {
        ui.label("Name:");
        if ui.text_edit_singleline(&mut case.name).changed() {
            action = DamageAction::Changed;
        }
    });
    let name = case.name.clone();
    if panel
        .project
        .damage_cases
        .iter()
        .filter(|c| c.name == name)
        .count()
        > 1
    {
        ui.colored_label(
            egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
            "Another damage case has this name; rename it before saving.",
        );
    }
    let case = &mut panel.project.damage_cases[*panel.selected];

    // Flooded compartments checkboxes
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Flooded Compartments").strong());
    if panel.project.compartments.is_empty() {
        ui.label("No compartments defined above.");
    } else {
        for comp in &panel.project.compartments {
            let is_flooded = case.compartments.iter().any(|s| match s {
                CompartmentSpec::Named(n) => n == &comp.name,
                _ => false,
            });
            let mut checked = is_flooded;
            if ui.checkbox(&mut checked, &comp.name).changed() {
                if checked {
                    if !is_flooded {
                        case.compartments
                            .push(CompartmentSpec::Named(comp.name.clone()));
                        action = DamageAction::Changed;
                    }
                } else {
                    case.compartments.retain(|s| match s {
                        CompartmentSpec::Named(n) => n != &comp.name,
                        CompartmentSpec::Inline(_) => true,
                    });
                    action = DamageAction::Changed;
                }
            }
        }
    }

    // Keep any Inline specs read-only
    let inlines: Vec<Compartment> = case
        .compartments
        .iter()
        .filter_map(|s| match s {
            CompartmentSpec::Inline(c) => Some(c.clone()),
            _ => None,
        })
        .collect();
    if !inlines.is_empty() {
        ui.add_space(2.0);
        ui.label(egui::RichText::new("Inline Compartments (read-only):").italics());
        for inc in &inlines {
            ui.label(format!(
                "• {} [x: {:.2}..{:.2}, y: {:.2}..{:.2}, z: {:.2}..{:.2}, μ: {:.2}]",
                inc.name,
                inc.x_min,
                inc.x_max,
                inc.y_min,
                inc.y_max,
                inc.z_min,
                inc.z_max,
                inc.permeability
            ));
        }
    }

    // Intact condition
    ui.separator();
    ui.label(egui::RichText::new("Intact Condition").strong());

    let is_by_cond = case.loading_condition_name.is_some()
        || (case.loading_condition.is_some() && case.displacement_t.is_none());
    let mut mode_cond = is_by_cond;

    ui.horizontal(|ui| {
        if ui
            .radio_value(&mut mode_cond, true, "Loading condition")
            .clicked()
            && !is_by_cond
        {
            let default_name = panel
                .project
                .loading_conditions
                .first()
                .map(|c| c.name.clone());
            case.loading_condition_name = default_name;
            case.loading_condition = None;
            // Manual particulars would override the condition in
            // `damage::evaluate`, so they go when the condition takes over.
            case.displacement_t = None;
            case.kg = None;
            case.lcg = None;
            case.tcg = None;
            action = DamageAction::Changed;
        }
        if ui.radio_value(&mut mode_cond, false, "Manual").clicked() && is_by_cond {
            case.loading_condition_name = None;
            case.loading_condition = None;
            if case.displacement_t.is_none() {
                case.displacement_t = Some(10.0);
            }
            if case.kg.is_none() {
                case.kg = Some(1.0);
            }
            if case.lcg.is_none() {
                case.lcg = Some(0.0);
            }
            action = DamageAction::Changed;
        }
    });

    if mode_cond {
        if panel.project.loading_conditions.is_empty() {
            ui.label("No loading conditions in project.");
        } else {
            let current_cond_name = case
                .loading_condition_name
                .clone()
                .unwrap_or_else(|| panel.project.loading_conditions[0].name.clone());
            if case.loading_condition_name.is_none() {
                case.loading_condition_name = Some(current_cond_name.clone());
                case.loading_condition = None;
                action = DamageAction::Changed;
            }
            ui.horizontal(|ui| {
                ui.label("Condition:");
                egui::ComboBox::from_id_salt("damage_loading_condition_picker")
                    .selected_text(&current_cond_name)
                    .show_ui(ui, |ui| {
                        for cond in &panel.project.loading_conditions {
                            let is_sel = case.loading_condition_name.as_deref() == Some(&cond.name);
                            if ui.selectable_label(is_sel, &cond.name).clicked() {
                                case.loading_condition_name = Some(cond.name.clone());
                                case.loading_condition = None;
                                action = DamageAction::Changed;
                            }
                        }
                    });
            });
        }
    } else {
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.y = 2.0;
            cols[0].horizontal(|ui| {
                ui.label("Δ (t):");
                let mut disp = case.displacement_t.unwrap_or(10.0);
                if ui
                    .add(
                        egui::DragValue::new(&mut disp)
                            .speed(0.01)
                            .range(0.001..=f64::INFINITY),
                    )
                    .changed()
                {
                    case.displacement_t = Some(disp);
                    case.loading_condition_name = None;
                    action = DamageAction::Changed;
                }
            });
            cols[0].horizontal(|ui| {
                ui.label("KG (m):");
                let mut kg = case.kg.unwrap_or(1.0);
                if ui
                    .add(
                        egui::DragValue::new(&mut kg)
                            .speed(0.01)
                            .range(0.0..=f64::INFINITY),
                    )
                    .changed()
                {
                    case.kg = Some(kg);
                    case.loading_condition_name = None;
                    action = DamageAction::Changed;
                }
            });
            cols[1].spacing_mut().item_spacing.y = 2.0;
            cols[1].horizontal(|ui| {
                ui.label("LCG (m):");
                let mut lcg = case.lcg.unwrap_or(0.0);
                if ui.add(egui::DragValue::new(&mut lcg).speed(0.01)).changed() {
                    case.lcg = Some(lcg);
                    case.loading_condition_name = None;
                    action = DamageAction::Changed;
                }
            });
            cols[1].horizontal(|ui| {
                ui.label("TCG (m):");
                let mut tcg = case.tcg.unwrap_or(0.0);
                if ui.add(egui::DragValue::new(&mut tcg).speed(0.01)).changed() {
                    case.tcg = Some(tcg);
                    case.loading_condition_name = None;
                    action = DamageAction::Changed;
                }
            });
        });
    }

    // Compute button
    ui.separator();
    ui.horizontal(|ui| {
        let btn = ui.add_enabled(!panel.busy, egui::Button::new("Compute"));
        if btn.clicked() {
            action = DamageAction::Compute;
        }
        if panel.busy {
            ui.spinner();
            ui.label("computing…");
        } else if panel.stale {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                "stale — press Compute",
            );
        }
    });

    // Results
    if let Some(r) = panel.result {
        ui.separator();
        ui.heading("Damaged Equilibrium");
        if !r.equilibrium_found {
            ui.colored_label(
                egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
                "Equilibrium NOT found (lost buoyancy exceeds reserve buoyancy or capsized)",
            );
        } else {
            ui.colored_label(
                egui::Color32::from_rgb(0x5a, 0xd0, 0x6a),
                "Equilibrium found",
            );
        }

        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.y = 2.0;
            row(
                &mut cols[0],
                "Δ (mass)",
                format!("{:.3} t", r.displacement_t),
            );
            row(
                &mut cols[0],
                "Intact draft",
                format!("{:.3} m", r.intact_draft),
            );
            row(&mut cols[0], "Damaged draft", format!("{:.3} m", r.draft));
            row(&mut cols[0], "Sinkage", format!("{:.3} m", r.sinkage_m));
            row(&mut cols[0], "Damaged heel", format!("{:.1}°", r.heel_deg));
            row(
                &mut cols[0],
                "Net buoyant vol.",
                format!("{:.3} m³", r.net_volume_m3),
            );

            cols[1].spacing_mut().item_spacing.y = 2.0;
            row(&mut cols[1], "KG", format!("{:.3} m", r.kg));
            row(
                &mut cols[1],
                "Intact trim",
                format!("{:.3} m", r.intact_trim_m),
            );
            row(&mut cols[1], "Damaged trim", format!("{:.3} m", r.trim_m));
            if let Some(fb) = r.freeboard_m {
                row(&mut cols[1], "Min freeboard", format!("{:.3} m", fb));
            } else {
                row(&mut cols[1], "Min freeboard", "-".to_string());
            }
            row(
                &mut cols[1],
                "Lost buoyancy",
                format!("{:.3} m³", r.lost_volume_m3),
            );
            row(&mut cols[1], "KB", format!("{:.3} m", r.kb));
        });

        if !r.compartments.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Flooded Compartment Summary").strong());
            egui::Grid::new("result_damage_compartments_table")
                .num_columns(5)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Compartment");
                    ui.label("μ");
                    ui.label("Submerged (m³)");
                    ui.label("Lost (m³)");
                    ui.label("Centroid (x, y, z)");
                    ui.end_row();
                    for c in &r.compartments {
                        ui.label(&c.name);
                        ui.label(format!("{:.2}", c.permeability));
                        ui.label(format!("{:.3}", c.submerged_volume_m3));
                        ui.label(format!("{:.3}", c.lost_volume_m3));
                        ui.label(format!(
                            "{:.2}, {:.2}, {:.2}",
                            c.centroid.x, c.centroid.y, c.centroid.z
                        ));
                        ui.end_row();
                    }
                });
        }

        if let Some(crit) = &r.criteria {
            ui.add_space(4.0);
            let badge_color = if crit.passed {
                egui::Color32::from_rgb(0x5a, 0xd0, 0x6a)
            } else {
                egui::Color32::from_rgb(0xe0, 0x5c, 0x5c)
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Residual Stability Criteria:").strong());
                ui.colored_label(badge_color, if crit.passed { "PASS" } else { "FAIL" });
            });
            let pass_fail = |ok: bool| {
                if ok {
                    (egui::Color32::from_rgb(0x5a, 0xd0, 0x6a), "PASS")
                } else {
                    (egui::Color32::from_rgb(0xe0, 0x5c, 0x5c), "FAIL")
                }
            };
            egui::Grid::new("damage_criteria_grid")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Criterion");
                    ui.label("Value");
                    ui.label("Status");
                    ui.end_row();

                    let rows = [
                        (
                            "Positive range ≥ 15°",
                            format!("{:.1}°", crit.positive_range_deg),
                            crit.positive_range_deg >= 15.0,
                        ),
                        (
                            "Max GZ ≥ 0.10 m",
                            format!("{:.3} m", crit.max_gz_m),
                            crit.max_gz_m >= 0.10,
                        ),
                        (
                            "Area ≥ 0.015 m·rad",
                            format!("{:.4} m·rad", crit.area_m_rad),
                            crit.area_m_rad >= 0.015,
                        ),
                    ];
                    for (name, val, ok) in rows {
                        let (col, text) = pass_fail(ok);
                        ui.label(name);
                        ui.label(val);
                        ui.colored_label(col, text);
                        ui.end_row();
                    }
                });
        }

        ui.add_space(4.0);
        let pts: Vec<(f64, f64)> = r
            .residual_gz
            .iter()
            .filter_map(|p| {
                p.gz.filter(|gz| gz.is_finite())
                    .map(|gz| (p.relative_heel_deg, gz))
            })
            .collect();
        if !pts.is_empty() {
            ui.label(egui::RichText::new("Residual Righting Lever GZ").strong());
            plot_gz(ui, &[(r.case_name.clone(), pts)]);
        }

        if !r.warnings.is_empty() {
            ui.add_space(4.0);
            for w in &r.warnings {
                ui.colored_label(
                    egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                    format!("Warning: {w}"),
                );
            }
        }
    }

    action
}

/// A small GZ plot: heel across, righting lever up, one line per displacement.
fn plot_cross_curves(ui: &mut egui::Ui, curves: &[CrossCurve]) {
    let height = 150.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x0f, 0x14, 0x1b));
    let mut gz_max: f64 = 0.1;
    let mut heel_max: f64 = 1.0;
    for curve in curves {
        heel_max = heel_max.max(curve.heel_deg);
        for p in &curve.points {
            if let Some(gz) = p.gz {
                if gz.is_finite() {
                    gz_max = gz_max.max(gz.abs());
                }
            }
        }
    }
    let to_screen = |heel: f64, gz: f64| -> egui::Pos2 {
        let x = rect.left() + (heel / heel_max) as f32 * rect.width();
        let y = rect.bottom() - ((gz / gz_max) as f32 * 0.45 + 0.45) * rect.height();
        egui::pos2(x, y)
    };
    // Zero line and the axes.
    painter.line_segment(
        [to_screen(0.0, 0.0), to_screen(heel_max, 0.0)],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x42, 0x52)),
    );
    let colours = [
        egui::Color32::from_rgb(0x5c, 0xa8, 0xf0),
        egui::Color32::from_rgb(0x5a, 0xd0, 0x6a),
        egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
        egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
    ];
    for (i, curve) in curves.iter().enumerate() {
        let points: Vec<egui::Pos2> = curve
            .points
            .iter()
            .filter_map(|p| p.gz.filter(|gz| gz.is_finite()))
            .map(|gz| to_screen(curve.heel_deg, gz))
            .collect();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.4, colours[i % colours.len()]),
            ));
        }
    }
    painter.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        format!("GZ up to {gz_max:.2} m · heel to {heel_max:.0}°"),
        egui::FontId::monospace(10.0),
        egui::Color32::from_rgb(0x8e, 0x9e, 0xae),
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoadingAction {
    None,
    Changed,
    Compute,
}

pub struct LoadingPanel<'a> {
    pub project: &'a mut Project,
    pub selected: &'a mut usize,
    pub result: Option<&'a LoadingResult>,
    pub stale: bool,
    pub busy: bool,
    pub highlight: Option<usize>,
    pub scroll_to: bool,
}

fn unique_condition_name(base: &str, existing: &[LoadingCondition]) -> String {
    let base_trimmed = base.trim();
    let base_name = if base_trimmed.is_empty() {
        "Condition"
    } else {
        base_trimmed
    };
    if !existing.iter().any(|c| c.name == base_name) {
        return base_name.to_string();
    }
    let mut counter = 2;
    loop {
        let name = format!("{base_name} ({counter})");
        if !existing.iter().any(|c| c.name == name) {
            return name;
        }
        counter += 1;
    }
}

/// A general 2D XY plot with labelled series, axes, and header text.
pub fn plot_xy(
    ui: &mut egui::Ui,
    title: &str,
    x_label: &str,
    y_label: &str,
    series: &[(String, Vec<(f64, f64)>)],
) {
    let height = 150.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x0f, 0x14, 0x1b));

    let is_gz = title == "GZ";
    let mut x_min: f64 = 0.0;
    let mut x_max: f64 = if is_gz { 40.0 } else { 1.0 };
    let mut y_min: f64 = 0.0;
    let mut y_max: f64 = 0.1;

    for (_, pts) in series {
        for &(x, y) in pts {
            if x.is_finite() {
                x_max = x_max.max(x);
                if !is_gz {
                    x_min = x_min.min(x);
                }
            }
            if y.is_finite() {
                y_max = y_max.max(y);
                y_min = y_min.min(y);
            }
        }
    }
    let x_range = (x_max - x_min).max(1e-6);
    let y_range = (y_max - y_min).max(0.05) * 1.15;
    let y_base = y_min - (y_range - (y_max - y_min)) * 0.5;

    let to_screen = |x: f64, y: f64| -> egui::Pos2 {
        let sx = rect.left() + ((x - x_min) / x_range) as f32 * rect.width();
        let sy = rect.bottom() - ((y - y_base) / y_range) as f32 * rect.height();
        egui::pos2(sx, sy)
    };

    // Zero line and the axes.
    if y_base <= 0.0 && y_base + y_range >= 0.0 {
        painter.line_segment(
            [to_screen(x_min, 0.0), to_screen(x_max, 0.0)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x42, 0x52)),
        );
    }
    if x_min <= 0.0 && x_max >= 0.0 {
        painter.line_segment(
            [to_screen(0.0, y_base), to_screen(0.0, y_base + y_range)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x42, 0x52)),
        );
    }

    let colours = [
        egui::Color32::from_rgb(0x5c, 0xa8, 0xf0),
        egui::Color32::from_rgb(0x5a, 0xd0, 0x6a),
        egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
        egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
        egui::Color32::from_rgb(0xc0, 0x7c, 0xeb),
        egui::Color32::from_rgb(0x40, 0xd0, 0xd0),
    ];

    for (i, (label, pts)) in series.iter().enumerate() {
        let points: Vec<egui::Pos2> = pts
            .iter()
            .filter(|(x, y)| x.is_finite() && y.is_finite())
            .map(|&(x, y)| to_screen(x, y))
            .collect();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.4, colours[i % colours.len()]),
            ));
        }
        if series.len() > 1 && !label.is_empty() {
            let legend_pos = rect.left_top() + egui::vec2(6.0, 16.0 + i as f32 * 14.0);
            painter.text(
                legend_pos,
                egui::Align2::LEFT_TOP,
                label,
                egui::FontId::monospace(10.0),
                colours[i % colours.len()],
            );
        }
    }
    let header = if is_gz {
        format!("GZ up to {y_max:.2} m · heel to {x_max:.0}°")
    } else {
        format!("{title}: {y_label} up to {y_max:.2} · {x_label} to {x_max:.1}")
    };
    painter.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        header,
        egui::FontId::monospace(10.0),
        egui::Color32::from_rgb(0x8e, 0x9e, 0xae),
    );
}

/// A general GZ plot: heel (deg) across, righting lever GZ (m) up, with labelled curves.
pub fn plot_gz(ui: &mut egui::Ui, curves: &[(String, Vec<(f64, f64)>)]) {
    plot_xy(ui, "GZ", "heel", "GZ", curves);
}

pub fn loading(ui: &mut egui::Ui, panel: LoadingPanel<'_>) -> LoadingAction {
    let mut action = LoadingAction::None;
    ui.heading("Loading Condition");
    ui.add_space(4.0);

    // Condition picker and New / Duplicate / Delete buttons
    let mut delete_requested = false;
    let mut duplicate_requested = false;
    let mut new_requested = false;

    if panel.project.loading_conditions.is_empty() {
        ui.horizontal(|ui| {
            ui.label("No conditions in project.");
            if ui.button("New").clicked() {
                new_requested = true;
            }
        });
    } else {
        if *panel.selected >= panel.project.loading_conditions.len() {
            *panel.selected = panel.project.loading_conditions.len() - 1;
        }
        let current_name = panel.project.loading_conditions[*panel.selected]
            .name
            .clone();
        ui.horizontal(|ui| {
            egui::ComboBox::from_id_salt("loading_condition_picker")
                .selected_text(&current_name)
                .show_ui(ui, |ui| {
                    for (idx, cond) in panel.project.loading_conditions.iter().enumerate() {
                        if ui
                            .selectable_label(*panel.selected == idx, &cond.name)
                            .clicked()
                        {
                            *panel.selected = idx;
                            action = LoadingAction::Changed;
                        }
                    }
                });
            if ui.button("New").clicked() {
                new_requested = true;
            }
            if ui.button("Duplicate").clicked() {
                duplicate_requested = true;
            }
            if ui.button("Delete").clicked() {
                delete_requested = true;
            }
        });
    }

    if new_requested {
        let name = unique_condition_name("Condition 1", &panel.project.loading_conditions);
        panel.project.loading_conditions.push(LoadingCondition {
            name,
            lightship: WeightItem {
                name: "Lightship".into(),
                mass_t: 10.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            items: Vec::new(),
            tanks: Vec::new(),
            windage_extra: Vec::new(),
            bilge: seacraft_core::weather::BilgeKind::Round,
            downflooding_deg: None,
        });
        *panel.selected = panel.project.loading_conditions.len() - 1;
        return LoadingAction::Changed;
    }

    if duplicate_requested && !panel.project.loading_conditions.is_empty() {
        let orig = &panel.project.loading_conditions[*panel.selected];
        let mut dup = orig.clone();
        dup.name = unique_condition_name(&orig.name, &panel.project.loading_conditions);
        panel.project.loading_conditions.push(dup);
        *panel.selected = panel.project.loading_conditions.len() - 1;
        return LoadingAction::Changed;
    }

    if delete_requested && !panel.project.loading_conditions.is_empty() {
        panel.project.loading_conditions.remove(*panel.selected);
        if *panel.selected >= panel.project.loading_conditions.len() && *panel.selected > 0 {
            *panel.selected -= 1;
        }
        return LoadingAction::Changed;
    }

    if panel.project.loading_conditions.is_empty() {
        return action;
    }

    let cond = &mut panel.project.loading_conditions[*panel.selected];

    // Editable name
    ui.horizontal(|ui| {
        ui.label("Name:");
        if ui.text_edit_singleline(&mut cond.name).changed() {
            action = LoadingAction::Changed;
        }
    });
    let name = cond.name.clone();
    if panel
        .project
        .loading_conditions
        .iter()
        .filter(|c| c.name == name)
        .count()
        > 1
    {
        ui.colored_label(
            egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
            "Another condition has this name; rename it before saving.",
        );
    }
    let cond = &mut panel.project.loading_conditions[*panel.selected];

    // Lightship
    ui.separator();
    ui.label(egui::RichText::new("Lightship").strong());
    ui.columns(2, |cols| {
        cols[0].spacing_mut().item_spacing.y = 2.0;
        cols[0].horizontal(|ui| {
            ui.label("Mass (t):");
            if ui
                .add(
                    egui::DragValue::new(&mut cond.lightship.mass_t)
                        .speed(0.01)
                        .range(0.0..=f64::INFINITY),
                )
                .changed()
            {
                action = LoadingAction::Changed;
            }
        });
        cols[0].horizontal(|ui| {
            ui.label("LCG x (m):");
            if ui
                .add(egui::DragValue::new(&mut cond.lightship.x).speed(0.01))
                .changed()
            {
                action = LoadingAction::Changed;
            }
        });
        cols[1].spacing_mut().item_spacing.y = 2.0;
        cols[1].horizontal(|ui| {
            ui.label("TCG y (m):");
            if ui
                .add(egui::DragValue::new(&mut cond.lightship.y).speed(0.01))
                .changed()
            {
                action = LoadingAction::Changed;
            }
        });
        cols[1].horizontal(|ui| {
            ui.label("KG z (m):");
            if ui
                .add(egui::DragValue::new(&mut cond.lightship.z).speed(0.01))
                .changed()
            {
                action = LoadingAction::Changed;
            }
        });
    });

    // Weights grid
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Deadweight Items").strong());
        if ui.button("+ Add weight").clicked() {
            let n = cond.items.len() + 1;
            cond.items.push(WeightItem {
                name: format!("Item {n}"),
                mass_t: 0.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            });
            action = LoadingAction::Changed;
        }
    });

    if !cond.items.is_empty() {
        egui::Grid::new("items_weights_grid")
            .num_columns(6)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Name");
                ui.label("Mass (t)");
                ui.label("x (m)");
                ui.label("y (m)");
                ui.label("z (m)");
                ui.label("");
                ui.end_row();

                let mut remove_item = None;
                for (i, item) in cond.items.iter_mut().enumerate() {
                    if ui
                        .add_sized([140.0, 20.0], egui::TextEdit::singleline(&mut item.name))
                        .changed()
                    {
                        action = LoadingAction::Changed;
                    }
                    if ui
                        .add(
                            egui::DragValue::new(&mut item.mass_t)
                                .speed(0.01)
                                .range(0.0..=f64::INFINITY),
                        )
                        .changed()
                    {
                        action = LoadingAction::Changed;
                    }
                    if ui
                        .add(egui::DragValue::new(&mut item.x).speed(0.01))
                        .changed()
                    {
                        action = LoadingAction::Changed;
                    }
                    if ui
                        .add(egui::DragValue::new(&mut item.y).speed(0.01))
                        .changed()
                    {
                        action = LoadingAction::Changed;
                    }
                    if ui
                        .add(egui::DragValue::new(&mut item.z).speed(0.01))
                        .changed()
                    {
                        action = LoadingAction::Changed;
                    }
                    if ui.button("🗑").on_hover_text("Remove weight").clicked() {
                        remove_item = Some(i);
                    }
                    ui.end_row();
                }
                if let Some(i) = remove_item {
                    cond.items.remove(i);
                    action = LoadingAction::Changed;
                }
            });
    }

    // Tanks grid
    ui.separator();
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new("Tanks").strong());
        if ui.button("+ Add tank").clicked() {
            let n = cond.tanks.len() + 1;
            cond.tanks.push(Tank {
                name: format!("Tank {n}"),
                x_min: 0.0,
                x_max: 1.0,
                y_min: -0.5,
                y_max: 0.5,
                z_min: 0.0,
                z_max: 0.5,
                fill: 0.5,
                density_t_m3: 1.0,
                permeability: 0.98,
                clip_to_hull: false,
            });
            action = LoadingAction::Changed;
        }
    });

    let mut remove_tank = None;
    for (t_idx, tank) in cond.tanks.iter_mut().enumerate() {
        let is_highlighted = panel.highlight == Some(t_idx);
        let mut frame = egui::Frame::group(ui.style());
        if is_highlighted {
            frame = frame.stroke(egui::Stroke::new(
                2.0,
                egui::Color32::from_rgb(220, 150, 40),
            ));
        }
        let frame_resp = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                if ui.text_edit_singleline(&mut tank.name).changed() {
                    action = LoadingAction::Changed;
                }
                if ui
                    .checkbox(&mut tank.clip_to_hull, "Clip to hull")
                    .changed()
                {
                    action = LoadingAction::Changed;
                }
                if ui.button("🗑").on_hover_text("Remove tank").clicked() {
                    remove_tank = Some(t_idx);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Fill:");
                let mut fill_pct = (tank.fill * 100.0).round();
                if ui
                    .add(egui::Slider::new(&mut fill_pct, 0.0..=100.0).suffix("%"))
                    .changed()
                {
                    tank.fill = (fill_pct / 100.0).clamp(0.0, 1.0);
                    action = LoadingAction::Changed;
                }
            });
            ui.horizontal(|ui| {
                ui.label("Density (t/m³):");
                if ui
                    .add(
                        egui::DragValue::new(&mut tank.density_t_m3)
                            .speed(0.01)
                            .range(0.0..=10.0),
                    )
                    .changed()
                {
                    action = LoadingAction::Changed;
                }
                ui.label("Perm.:");
                if ui
                    .add(
                        egui::DragValue::new(&mut tank.permeability)
                            .speed(0.01)
                            .range(0.0..=1.0),
                    )
                    .changed()
                {
                    action = LoadingAction::Changed;
                }
            });
            egui::CollapsingHeader::new("Bounds [x, y, z]")
                .id_salt(("tank_bounds_header", t_idx))
                .default_open(false)
                .show(ui, |ui| {
                    egui::Grid::new(format!("tank_bounds_{t_idx}"))
                        .num_columns(3)
                        .show(ui, |ui| {
                            ui.label("X range (m):");
                            if ui
                                .add(egui::DragValue::new(&mut tank.x_min).speed(0.01))
                                .changed()
                            {
                                action = LoadingAction::Changed;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut tank.x_max).speed(0.01))
                                .changed()
                            {
                                action = LoadingAction::Changed;
                            }
                            ui.end_row();

                            ui.label("Y range (m):");
                            if ui
                                .add(egui::DragValue::new(&mut tank.y_min).speed(0.01))
                                .changed()
                            {
                                action = LoadingAction::Changed;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut tank.y_max).speed(0.01))
                                .changed()
                            {
                                action = LoadingAction::Changed;
                            }
                            ui.end_row();

                            ui.label("Z range (m):");
                            if ui
                                .add(egui::DragValue::new(&mut tank.z_min).speed(0.01))
                                .changed()
                            {
                                action = LoadingAction::Changed;
                            }
                            if ui
                                .add(egui::DragValue::new(&mut tank.z_max).speed(0.01))
                                .changed()
                            {
                                action = LoadingAction::Changed;
                            }
                            ui.end_row();
                        });
                });
        });
        if is_highlighted && panel.scroll_to {
            frame_resp.response.scroll_to_me(Some(egui::Align::Center));
        }
    }
    if let Some(i) = remove_tank {
        cond.tanks.remove(i);
        action = LoadingAction::Changed;
    }

    // Compute button
    ui.separator();
    ui.horizontal(|ui| {
        let btn = ui.add_enabled(!panel.busy, egui::Button::new("Compute"));
        if btn.clicked() {
            action = LoadingAction::Compute;
        }
        if panel.busy {
            ui.spinner();
            ui.label("computing…");
        } else if panel.stale {
            ui.colored_label(
                egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
                "stale — press Compute",
            );
        }
    });

    // Results
    if let Some(r) = panel.result {
        ui.separator();
        ui.heading("Equilibrium & Hydrostatics");
        ui.columns(2, |cols| {
            cols[0].spacing_mut().item_spacing.y = 2.0;
            row(
                &mut cols[0],
                "Δ (mass)",
                format!("{:.3} t", r.displacement_t),
            );
            row(&mut cols[0], "LCG", format!("{:.3} m", r.lcg));
            row(&mut cols[0], "TCG", format!("{:.3} m", r.tcg));
            row(&mut cols[0], "KG (solid)", format!("{:.3} m", r.kg_solid));
            row(&mut cols[0], "FSM", format!("{:.2} t·m", r.fsm_t_m));
            row(&mut cols[0], "FSC", format!("{:.3} m", r.fsc_m));

            cols[1].spacing_mut().item_spacing.y = 2.0;
            row(&mut cols[1], "KG (fluid)", format!("{:.3} m", r.kg_fluid));
            row(&mut cols[1], "Draft", format!("{:.3} m", r.draft));
            row(&mut cols[1], "Trim", format!("{:.3} m", r.trim_m));
            row(&mut cols[1], "List", format!("{:.1}°", r.list_deg));
            row(&mut cols[1], "GM (solid)", format!("{:.3} m", r.gm_solid));
            row(&mut cols[1], "GM (fluid)", format!("{:.3} m", r.gm_fluid));
        });

        if !r.tanks.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Tank Summary").strong());
            egui::Grid::new("result_tanks_table")
                .num_columns(4)
                .striped(true)
                .show(ui, |ui| {
                    ui.label("Tank");
                    ui.label("Fill");
                    ui.label("Mass (t)");
                    ui.label("FSM (t·m)");
                    ui.end_row();
                    for t in &r.tanks {
                        ui.label(&t.name);
                        ui.label(format!("{:.0}%", t.fill * 100.0));
                        ui.label(format!("{:.3}", t.mass_t));
                        ui.label(format!("{:.2}", t.fsm_t_m));
                        ui.end_row();
                    }
                });
        }

        if let Some(imo) = &r.imo_criteria {
            ui.add_space(4.0);
            let badge_color = if imo.passed {
                egui::Color32::from_rgb(0x5a, 0xd0, 0x6a)
            } else {
                egui::Color32::from_rgb(0xe0, 0x5c, 0x5c)
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("IMO A.749 Criteria:").strong());
                ui.colored_label(badge_color, if imo.passed { "PASS" } else { "FAIL" });
            });
            let pass_fail = |ok: bool| {
                if ok {
                    (egui::Color32::from_rgb(0x5a, 0xd0, 0x6a), "PASS")
                } else {
                    (egui::Color32::from_rgb(0xe0, 0x5c, 0x5c), "FAIL")
                }
            };
            egui::Grid::new("imo_criteria_table")
                .num_columns(3)
                .striped(true)
                .show(ui, |ui| {
                    let rows = [
                        (
                            "A(0–30°) ≥ 0.055",
                            format!("{:.3} m·rad", imo.area_0_30_m_rad),
                            imo.area_0_30_m_rad >= 0.055,
                        ),
                        (
                            "A(0–40°) ≥ 0.090",
                            format!("{:.3} m·rad", imo.area_0_40_m_rad),
                            imo.area_0_40_m_rad >= 0.090,
                        ),
                        (
                            "A(30–40°) ≥ 0.030",
                            format!("{:.3} m·rad", imo.area_30_40_m_rad),
                            imo.area_30_40_m_rad >= 0.030,
                        ),
                        (
                            "GZ(≥30°) ≥ 0.20",
                            format!("{:.3} m", imo.max_gz_30_plus_m),
                            imo.max_gz_30_plus_m >= 0.20,
                        ),
                        (
                            "φ(GZmax) ≥ 25°",
                            format!("{:.0}°", imo.angle_of_max_gz_deg),
                            imo.angle_of_max_gz_deg >= 25.0,
                        ),
                        (
                            "GM0 ≥ 0.15",
                            format!("{:.3} m", imo.gm0_m),
                            imo.gm0_m >= 0.15,
                        ),
                    ];
                    for (name, val, ok) in rows {
                        let (col, text) = pass_fail(ok);
                        ui.label(name);
                        ui.label(val);
                        ui.colored_label(col, text);
                        ui.end_row();
                    }
                });
        }

        if let Some(w) = &r.weather {
            ui.add_space(4.0);
            let badge_color = if w.passed {
                egui::Color32::from_rgb(0x5a, 0xd0, 0x6a)
            } else {
                egui::Color32::from_rgb(0xe0, 0x5c, 0x5c)
            };
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Weather Criterion (§2.3):").strong());
                ui.colored_label(badge_color, if w.passed { "PASS" } else { "FAIL" });
            });
            ui.columns(2, |cols| {
                row(
                    &mut cols[0],
                    "Steady heel φ0",
                    format!("{:.1}° (lim 16°)", w.phi0_deg.unwrap_or(0.0)),
                );
                row(
                    &mut cols[1],
                    "Area ratio b/a",
                    format!("{:.2} (req ≥1.0)", w.area_ratio),
                );
            });
        }

        ui.add_space(4.0);
        let pts: Vec<(f64, f64)> = r
            .cross_curves
            .iter()
            .filter_map(|c| {
                c.points
                    .first()
                    .and_then(|p| p.gz)
                    .filter(|gz| gz.is_finite())
                    .map(|gz| (c.heel_deg, gz))
            })
            .collect();
        if !pts.is_empty() {
            ui.label(egui::RichText::new("Righting Lever GZ").strong());
            plot_gz(ui, &[(r.name.clone(), pts)]);
        }
    }

    action
}

/// Action requested by the windage panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindageAction {
    None,
    Changed,
}

/// Windage panel state.
pub struct WindagePanel<'a> {
    pub project: &'a mut Project,
    pub weather: Option<&'a WeatherResult>,
    pub stale: bool,
    pub selected: &'a mut Option<usize>,
}

pub fn windage(ui: &mut egui::Ui, panel: WindagePanel<'_>) -> WindageAction {
    let mut action = WindageAction::None;
    ui.heading("Superstructure & Windage");
    ui.add_space(4.0);

    if ui.button("+ Add windage item").clicked() {
        let name = format!("Item {}", panel.project.windage.len() + 1);
        panel.project.windage.push(WindageItem {
            name,
            area_m2: 1.0,
            z_centroid: 2.0,
            profile: None,
        });
        *panel.selected = Some(panel.project.windage.len() - 1);
        return WindageAction::Changed;
    }
    ui.add_space(4.0);

    let mut remove_idx = None;
    if panel.project.windage.is_empty() {
        ui.label("No windage items defined.");
    } else {
        egui::Grid::new("windage_items_table")
            .num_columns(4)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label("");
                ui.label(egui::RichText::new("Name").strong());
                ui.label(egui::RichText::new("Geometry").strong());
                ui.label("");
                ui.end_row();

                for (i, item) in panel.project.windage.iter_mut().enumerate() {
                    let is_selected = *panel.selected == Some(i);
                    let row_start = ui.cursor().min;

                    let sel_label = if is_selected { "●" } else { "○" };
                    if ui.selectable_label(is_selected, sel_label).clicked() {
                        *panel.selected = if is_selected { None } else { Some(i) };
                    }

                    if ui
                        .add_sized([100.0, 20.0], egui::TextEdit::singleline(&mut item.name))
                        .changed()
                    {
                        action = WindageAction::Changed;
                    }

                    if let Some(mut rect) = item.profile {
                        let mut rect_changed = false;
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("x:");
                                rect_changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut rect.x_min)
                                            .speed(0.02)
                                            .prefix("min "),
                                    )
                                    .changed();
                                rect_changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut rect.x_max)
                                            .speed(0.02)
                                            .prefix("max "),
                                    )
                                    .changed();
                            });
                            ui.horizontal(|ui| {
                                ui.label("z:");
                                rect_changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut rect.z_min)
                                            .speed(0.02)
                                            .prefix("min "),
                                    )
                                    .changed();
                                rect_changed |= ui
                                    .add(
                                        egui::DragValue::new(&mut rect.z_max)
                                            .speed(0.02)
                                            .prefix("max "),
                                    )
                                    .changed();
                            });
                            ui.label(format!(
                                "Area: {:.2} m², z: {:.2} m",
                                item.area_m2, item.z_centroid
                            ));
                        });
                        if rect_changed {
                            item.profile = Some(rect);
                            item.sync_from_profile();
                            action = WindageAction::Changed;
                        }
                    } else {
                        ui.vertical(|ui| {
                            ui.horizontal(|ui| {
                                ui.label("Area:");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut item.area_m2)
                                            .speed(0.02)
                                            .range(0.0..=f64::INFINITY)
                                            .suffix(" m²"),
                                    )
                                    .changed()
                                {
                                    action = WindageAction::Changed;
                                }
                                ui.label("z:");
                                if ui
                                    .add(
                                        egui::DragValue::new(&mut item.z_centroid)
                                            .speed(0.02)
                                            .suffix(" m"),
                                    )
                                    .changed()
                                {
                                    action = WindageAction::Changed;
                                }
                            });
                            if ui.button("Draw as rectangle").clicked() {
                                let area = item.area_m2.max(0.1);
                                let w = area.sqrt();
                                let h = w;
                                let mid_x = panel
                                    .project
                                    .compartments
                                    .first()
                                    .map(|c| (c.x_min + c.x_max) * 0.5)
                                    .or_else(|| {
                                        panel.project.loading_conditions.first().and_then(|c| {
                                            c.tanks.first().map(|t| (t.x_min + t.x_max) * 0.5)
                                        })
                                    })
                                    .unwrap_or(6.0);
                                item.profile = Some(WindageRect {
                                    x_min: mid_x - w * 0.5,
                                    x_max: mid_x + w * 0.5,
                                    z_min: item.z_centroid - h * 0.5,
                                    z_max: item.z_centroid + h * 0.5,
                                });
                                item.sync_from_profile();
                                action = WindageAction::Changed;
                            }
                        });
                    }

                    if ui.button("✖").clicked() {
                        remove_idx = Some(i);
                    }

                    let row_end = ui.cursor().min;
                    if is_selected {
                        let row_rect = egui::Rect::from_min_max(
                            egui::pos2(ui.min_rect().min.x, row_start.y),
                            egui::pos2(ui.min_rect().max.x, row_end.y),
                        );
                        ui.painter().rect_stroke(
                            row_rect,
                            2.0,
                            egui::Stroke::new(1.5, egui::Color32::from_rgb(60, 200, 100)),
                            egui::StrokeKind::Inside,
                        );
                    }

                    ui.end_row();
                }
            });

        if let Some(i) = remove_idx {
            panel.project.windage.remove(i);
            if *panel.selected == Some(i) {
                *panel.selected = None;
            } else if let Some(ref mut sel) = *panel.selected {
                if *sel > i {
                    *sel -= 1;
                }
            }
            action = WindageAction::Changed;
        }
    }

    ui.add_space(8.0);
    ui.separator();
    ui.label(egui::RichText::new("Windage Totals").strong());
    let total_area: f64 = panel.project.windage.iter().map(|w| w.area_m2).sum();
    let total_az: f64 = panel
        .project
        .windage
        .iter()
        .map(|w| w.area_m2 * w.z_centroid)
        .sum();
    let z_centroid = if total_area > 1e-9 {
        total_az / total_area
    } else {
        0.0
    };
    egui::Grid::new("windage_totals_grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Total windage area (ΣA):");
            ui.label(format!("{:.2} m²", total_area));
            ui.end_row();
            ui.label("Area-weighted z centroid:");
            ui.label(format!("{:.2} m", z_centroid));
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    if let Some(w) = panel.weather {
        let badge_color = if w.passed {
            egui::Color32::from_rgb(0x5a, 0xd0, 0x6a)
        } else {
            egui::Color32::from_rgb(0xe0, 0x5c, 0x5c)
        };
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Weather Criterion Summary (§2.3):").strong());
            ui.colored_label(badge_color, if w.passed { "PASS" } else { "FAIL" });
            if panel.stale {
                ui.colored_label(egui::Color32::from_rgb(220, 150, 40), "(stale)");
            }
        });
        ui.small("from the loading panel's last Compute");
        ui.add_space(2.0);

        egui::Grid::new("windage_weather_grid")
            .num_columns(2)
            .striped(true)
            .show(ui, |ui| {
                ui.label("Hull lateral windage area:");
                ui.label(format!("{:.2} m²", w.hull_windage_area_m2));
                ui.end_row();

                ui.label("Extra windage area:");
                ui.label(format!("{:.2} m²", w.extra_windage_area_m2));
                ui.end_row();

                ui.label("Total windage area (A):");
                ui.label(format!("{:.2} m²", w.area_m2));
                ui.end_row();

                ui.label("Vertical lever arm (Z):");
                ui.label(format!("{:.2} m", w.z_m));
                ui.end_row();

                ui.label("Wind heeling lever (lw1):");
                ui.label(format!("{:.3} m", w.lw1_m));
                ui.end_row();

                ui.label("Steady heel angle (φ0):");
                ui.label(format!(
                    "{:.1}° (lim {:.1}°)",
                    w.phi0_deg.unwrap_or(0.0),
                    w.phi0_limit_deg
                ));
                ui.end_row();

                ui.label("Energy ratio (b/a):");
                ui.label(format!("{:.2} (req ≥1.0)", w.area_ratio));
                ui.end_row();
            });
    } else {
        ui.label("Compute a loading condition to check the weather criterion");
    }

    action
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::demo;
    use seacraft_core::hydro;
    use seacraft_core::subdiv::SubdivisionOptions;

    fn measured() -> Hydrostatics {
        let s = demo::demo_hull(false);
        let mut o = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 3,
                mode: Default::default(),
            },
        );
        o.draft = 0.62;
        hydro::compute(&s, &o)
    }

    #[test]
    fn the_dock_shows_both_columns_with_the_table_quantities() {
        let h = measured();
        let [left, right] = hydrostatic_rows(&h, 1.5);
        assert_eq!(left.len(), right.len(), "the two columns stay balanced");
        let labels: Vec<&str> = left.iter().chain(right.iter()).map(|(l, _)| *l).collect();
        for want in [
            "volume", "Cb", "LCB", "TPC", "KM(t)", "GM", "midship", "MTC",
        ] {
            assert!(labels.contains(&want), "{want} missing from {labels:?}");
        }
        // The table quantities are the measured waterplane in the right units.
        let tpc: f64 = left
            .iter()
            .find(|(l, _)| *l == "TPC")
            .and_then(|(_, v)| v.trim_end_matches(" t/cm").parse().ok())
            .expect("TPC parses");
        let want_tpc = h.waterplane_area_m2 * (h.water_density / 1000.0) / 100.0;
        assert!((tpc - want_tpc).abs() < 0.001, "TPC {tpc} vs {want_tpc}");
        assert!(
            (0.05..1.5).contains(&tpc),
            "TPC {tpc} t/cm is not plausible"
        );
        let mtc: f64 = right
            .iter()
            .find(|(l, _)| *l == "MTC")
            .and_then(|(_, v)| v.trim_end_matches(" t·m/cm").parse().ok())
            .expect("MTC parses");
        let want_mtc =
            (h.water_density / 1000.0) * h.waterplane_inertia_longitudinal / (100.0 * h.lwl);
        assert!((mtc - want_mtc).abs() < 0.001, "MTC {mtc} vs {want_mtc}");
        // GM follows KG: 1.5 m lighter than the measured KM by exactly that.
        let gm: f64 = right
            .iter()
            .find(|(l, _)| *l == "GM")
            .and_then(|(_, v)| v.trim_end_matches(" m").parse().ok())
            .expect("GM parses");
        assert!((gm - (h.km_transverse - 1.5)).abs() < 0.002, "GM {gm}");
    }

    #[test]
    fn the_sac_plot_runs_left_to_right_inside_the_rect() {
        let h = measured();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(120.0, 60.0));
        let pts = sac_polyline(&h.sac, rect);
        assert_eq!(pts.len(), h.sac.len(), "one point per station");
        assert!(
            pts.windows(2).all(|w| w[1].x >= w[0].x),
            "x must not go backwards"
        );
        for p in &pts {
            assert!(
                rect.x_range().contains(p.x) && rect.y_range().contains(p.y),
                "{p:?}"
            );
        }
        // The height is the area: the fullest station touches the top and every
        // other point maps linearly from its own area.
        let a_max = h.sac.iter().fold(0.0_f64, |m, s| m.max(s.area));
        for (point, station) in pts.iter().zip(h.sac.iter()) {
            let want = rect.bottom() - (station.area / a_max) as f32 * rect.height();
            assert!(
                (point.y - want).abs() < 1e-2,
                "{station:?} drew at {point:?}, wanted y {want}"
            );
        }
        let peak = pts
            .iter()
            .zip(h.sac.iter())
            .max_by(|a, b| {
                a.1.area
                    .partial_cmp(&b.1.area)
                    .unwrap_or(std::cmp::Ordering::Equal)
            })
            .map(|(p, _)| p.y)
            .expect("a peak");
        assert!((peak - rect.top()).abs() < 1e-2, "peak {peak}");
        let lowest = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        assert!(
            lowest < rect.top() + 1.0,
            "the peak reaches the top: {lowest}"
        );
        // Degenerate input draws nothing rather than dividing by zero.
        assert!(sac_polyline(&[], rect).is_empty());
        let flat = [SectionArea { x: 1.0, area: 0.0 }];
        assert!(sac_polyline(&flat, rect).is_empty());
    }

    #[test]
    fn loading_panel_and_plot_gz_run_headlessly() {
        let ctx = egui::Context::default();
        let mut project = Project::default();
        let mut selected = 0;
        let input = egui::RawInput::default();
        let mut out = ctx.run_ui(input.clone(), |ui| {
            let action = loading(
                ui,
                LoadingPanel {
                    project: &mut project,
                    selected: &mut selected,
                    result: None,
                    stale: false,
                    busy: false,
                    highlight: None,
                    scroll_to: false,
                },
            );
            assert_eq!(action, LoadingAction::None);
        });
        out.textures_delta.clear();

        project.loading_conditions.push(LoadingCondition {
            name: "Departure".into(),
            lightship: WeightItem {
                name: "Lightship".into(),
                mass_t: 10.0,
                x: 0.0,
                y: 0.0,
                z: 0.0,
            },
            items: vec![WeightItem {
                name: "Cargo".into(),
                mass_t: 2.0,
                x: 1.0,
                y: 0.0,
                z: 0.5,
            }],
            tanks: vec![Tank {
                name: "Fuel".into(),
                x_min: 0.0,
                x_max: 2.0,
                y_min: -0.5,
                y_max: 0.5,
                z_min: 0.0,
                z_max: 0.5,
                fill: 0.5,
                density_t_m3: 0.85,
                permeability: 0.98,
                clip_to_hull: false,
            }],
            windage_extra: Vec::new(),
            bilge: seacraft_core::weather::BilgeKind::Round,
            downflooding_deg: None,
        });

        let mut out = ctx.run_ui(input, |ui| {
            let action = loading(
                ui,
                LoadingPanel {
                    project: &mut project,
                    selected: &mut selected,
                    result: None,
                    stale: true,
                    busy: false,
                    highlight: None,
                    scroll_to: false,
                },
            );
            assert_eq!(action, LoadingAction::None);
            plot_gz(
                ui,
                &[("Test".into(), vec![(0.0, 0.0), (10.0, 0.2), (20.0, 0.35)])],
            );
        });
        out.textures_delta.clear();
    }

    #[test]
    fn damage_panel_and_plot_gz_run_headlessly() {
        let ctx = egui::Context::default();
        let mut project = Project::default();
        let mut selected = 0;
        let input = egui::RawInput::default();
        let mut out = ctx.run_ui(input.clone(), |ui| {
            let action = damage(
                ui,
                DamagePanel {
                    project: &mut project,
                    selected: &mut selected,
                    result: None,
                    stale: false,
                    busy: false,
                    highlight: None,
                    scroll_to: false,
                },
            );
            assert_eq!(action, DamageAction::None);
        });
        out.textures_delta.clear();

        project.compartments.push(Compartment {
            name: "Midship".into(),
            x_min: 3.5,
            x_max: 5.0,
            y_min: -1.0,
            y_max: 1.0,
            z_min: 0.0,
            z_max: 1.5,
            permeability: 0.95,
            clip_to_hull: true,
        });

        project.damage_cases.push(DamageCase {
            name: "Midship flood".into(),
            compartments: vec![CompartmentSpec::Named("Midship".into())],
            kg: Some(1.1),
            displacement_t: Some(12.0),
            lcg: Some(4.3),
            tcg: Some(0.0),
            loading_condition_name: None,
            loading_condition: None,
            water_density: None,
        });

        let mut out = ctx.run_ui(input, |ui| {
            let action = damage(
                ui,
                DamagePanel {
                    project: &mut project,
                    selected: &mut selected,
                    result: None,
                    stale: true,
                    busy: false,
                    highlight: None,
                    scroll_to: false,
                },
            );
            assert_eq!(action, DamageAction::None);
        });
        out.textures_delta.clear();
    }

    #[test]
    fn windage_panel_runs_headlessly() {
        let ctx = egui::Context::default();
        let mut project = Project::default();
        let mut selected = None;
        let input = egui::RawInput::default();
        let mut out = ctx.run_ui(input.clone(), |ui| {
            let action = windage(
                ui,
                WindagePanel {
                    project: &mut project,
                    weather: None,
                    stale: false,
                    selected: &mut selected,
                },
            );
            assert_eq!(action, WindageAction::None);
        });
        out.textures_delta.clear();

        project.windage.push(WindageItem::from_profile(
            "Deckhouse",
            WindageRect {
                x_min: 3.0,
                x_max: 6.0,
                z_min: 1.6,
                z_max: 2.4,
            },
        ));
        project.windage.push(WindageItem {
            name: "Mast".into(),
            area_m2: 1.2,
            z_centroid: 2.0,
            profile: None,
        });

        let mut out = ctx.run_ui(input, |ui| {
            let action = windage(
                ui,
                WindagePanel {
                    project: &mut project,
                    weather: None,
                    stale: true,
                    selected: &mut selected,
                },
            );
            assert_eq!(action, WindageAction::None);
        });
        out.textures_delta.clear();
    }
}
