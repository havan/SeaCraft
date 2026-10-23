//! Side docks: the layer manager and the hydrostatics report.

use eframe::egui;
use seacraft_core::hydro::{HydroOptions, Hydrostatics, SectionArea};
use seacraft_core::loading::{LoadingCondition, LoadingResult, Tank, WeightItem};
use seacraft_core::project::Project;
use seacraft_core::stability::CrossCurve;
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

/// A general GZ plot: heel (deg) across, righting lever GZ (m) up, with labelled curves.
pub fn plot_gz(ui: &mut egui::Ui, curves: &[(String, Vec<(f64, f64)>)]) {
    let height = 150.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, egui::Color32::from_rgb(0x0f, 0x14, 0x1b));

    let mut heel_max: f64 = 40.0;
    let mut gz_max: f64 = 0.1;
    let mut gz_min: f64 = 0.0;

    for (_, pts) in curves {
        for &(h, g) in pts {
            if h.is_finite() {
                heel_max = heel_max.max(h);
            }
            if g.is_finite() {
                gz_max = gz_max.max(g);
                gz_min = gz_min.min(g);
            }
        }
    }
    let gz_range = (gz_max - gz_min).max(0.05) * 1.15;
    let gz_base = gz_min - (gz_range - (gz_max - gz_min)) * 0.5;

    let to_screen = |heel: f64, gz: f64| -> egui::Pos2 {
        let x = rect.left() + (heel / heel_max) as f32 * rect.width();
        let y = rect.bottom() - ((gz - gz_base) / gz_range) as f32 * rect.height();
        egui::pos2(x, y)
    };

    // Zero line and the axes.
    if gz_base <= 0.0 && gz_base + gz_range >= 0.0 {
        painter.line_segment(
            [to_screen(0.0, 0.0), to_screen(heel_max, 0.0)],
            egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x42, 0x52)),
        );
    }
    painter.line_segment(
        [to_screen(0.0, gz_base), to_screen(0.0, gz_base + gz_range)],
        egui::Stroke::new(1.0, egui::Color32::from_rgb(0x33, 0x42, 0x52)),
    );

    let colours = [
        egui::Color32::from_rgb(0x5c, 0xa8, 0xf0),
        egui::Color32::from_rgb(0x5a, 0xd0, 0x6a),
        egui::Color32::from_rgb(0xe8, 0x9d, 0x3a),
        egui::Color32::from_rgb(0xe0, 0x5c, 0x5c),
        egui::Color32::from_rgb(0xc0, 0x7c, 0xeb),
        egui::Color32::from_rgb(0x40, 0xd0, 0xd0),
    ];

    for (i, (label, pts)) in curves.iter().enumerate() {
        let points: Vec<egui::Pos2> = pts
            .iter()
            .filter(|(h, g)| h.is_finite() && g.is_finite())
            .map(|&(h, g)| to_screen(h, g))
            .collect();
        if points.len() >= 2 {
            painter.add(egui::Shape::line(
                points,
                egui::Stroke::new(1.4, colours[i % colours.len()]),
            ));
        }
        if curves.len() > 1 && !label.is_empty() {
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
    painter.text(
        rect.left_top() + egui::vec2(4.0, 2.0),
        egui::Align2::LEFT_TOP,
        format!("GZ up to {gz_max:.2} m · heel to {heel_max:.0}°"),
        egui::FontId::monospace(10.0),
        egui::Color32::from_rgb(0x8e, 0x9e, 0xae),
    );
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
        egui::Frame::group(ui.style()).show(ui, |ui| {
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
}
