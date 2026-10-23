//! Side docks: the layer manager and the hydrostatics report.

use eframe::egui;
use seacraft_core::hydro::{HydroOptions, Hydrostatics};
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
        ui.colored_label(egui::Color32::from_rgb(0xe8, 0x9d, 0x3a), "no visible layers");
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
                row(&mut cols[0], "A(0–30°)", format!("{:.3} m·rad", imo.area_0_30_m_rad));
                row(&mut cols[0], "A(0–40°)", format!("{:.3} m·rad", imo.area_0_40_m_rad));
                row(&mut cols[0], "A(30–40°)", format!("{:.3} m·rad", imo.area_30_40_m_rad));
                cols[1].spacing_mut().item_spacing.y = 2.0;
                row(&mut cols[1], "GZ(≥30°)", format!("{:.3} m", imo.max_gz_30_plus_m));
                row(&mut cols[1], "φ(GZmax)", format!("{:.0}°", imo.angle_of_max_gz_deg));
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
            if p.gz.is_finite() {
                gz_max = gz_max.max(p.gz.abs());
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
            .filter(|p| p.gz.is_finite())
            .map(|p| to_screen(curve.heel_deg, p.gz))
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

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::demo;
    use seacraft_core::hydro;
    use seacraft_core::subdiv::SubdivisionOptions;

    fn measured() -> Hydrostatics {
        let s = demo::demo_hull(false);
        let mut o = HydroOptions::from_surface(&s, SubdivisionOptions { level: 3, mode: Default::default() });
        o.draft = 0.62;
        hydro::compute(&s, &o)
    }

    #[test]
    fn the_dock_shows_both_columns_with_the_table_quantities() {
        let h = measured();
        let [left, right] = hydrostatic_rows(&h, 1.5);
        assert_eq!(left.len(), right.len(), "the two columns stay balanced");
        let labels: Vec<&str> = left.iter().chain(right.iter()).map(|(l, _)| *l).collect();
        for want in ["volume", "Cb", "LCB", "TPC", "KM(t)", "GM", "midship", "MTC"] {
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
        assert!((0.05..1.5).contains(&tpc), "TPC {tpc} t/cm is not plausible");
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
}
