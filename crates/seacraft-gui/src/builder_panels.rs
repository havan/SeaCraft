//! File > Builder panels: the options, the list of panels with how far each is from lying flat,
//! and (in the window) the drawing. These functions draw and return what the user asked for;
//! `App` applies it.

use eframe::egui;
use seacraft_core::panels::PanelSet;
use seacraft_core::templates::StationChoice;

use crate::commands::PanelsParams;
use crate::theme;

/// What the user asked the window for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PanelsAction {
    /// Write the patterns to a file.
    Export,
    /// Frame panel `i` in the drawing and select its row.
    Frame(usize),
    /// Switch between the panel patterns and the nested sheets.
    ShowSheets(bool),
}

const NEST_MODES: [(&str, &str); 3] = [
    ("none", "No nesting"),
    ("blanks", "Join sheets into blanks"),
    ("split", "Split pieces to fit sheets"),
];

/// Where the measured panels stand for the current options and model.
#[derive(Clone, Copy, Debug)]
pub enum PanelsStatus<'a> {
    /// Nothing measured yet.
    Working,
    /// Measured for the current options and model.
    Ready(&'a PanelSet),
    /// Measured for older options or an older model; the new ones are being measured.
    Stale(&'a PanelSet),
    /// The current options or model give no panels; the reason.
    Failed(&'a str),
}

impl<'a> PanelsStatus<'a> {
    /// The panels to show, current or not.
    pub fn set(&self) -> Option<&'a PanelSet> {
        match self {
            PanelsStatus::Ready(s) | PanelsStatus::Stale(s) => Some(s),
            _ => None,
        }
    }
}

/// The options that change what is measured. `layers` are the model's layer names.
pub fn panels_options(ui: &mut egui::Ui, params: &mut PanelsParams, layers: &[String]) {
    egui::Grid::new("panels_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label("Layers:");
            let mut marked = params.layers.is_none();
            if ui
                .checkbox(&mut marked, "The layers marked developable")
                .changed()
            {
                params.layers = if marked { None } else { Some(Vec::new()) };
            }
            ui.end_row();
            if let Some(chosen) = &mut params.layers {
                for name in layers {
                    ui.label("");
                    let mut on = chosen.contains(name);
                    if ui.checkbox(&mut on, name).changed() {
                        if on {
                            chosen.push(name.clone());
                        } else {
                            chosen.retain(|n| n != name);
                        }
                    }
                    ui.end_row();
                }
                // A chosen name the model no longer has (a layer renamed or deleted): shown, so
                // it can be unticked.
                let gone: Vec<String> = chosen
                    .iter()
                    .filter(|n| !layers.contains(*n))
                    .cloned()
                    .collect();
                for name in gone {
                    ui.label("");
                    let mut on = true;
                    if ui
                        .checkbox(&mut on, format!("{name} (not in the model)"))
                        .changed()
                        && !on
                    {
                        chosen.retain(|n| *n != name);
                    }
                    ui.end_row();
                }
            }

            ui.label("Stations:");
            ui.horizontal(|ui| {
                let count = matches!(params.stations, StationChoice::Count(_));
                let spacing = matches!(params.stations, StationChoice::Spacing(_));
                if ui.radio(count, "Count").clicked() && !count {
                    params.stations = StationChoice::Count(11);
                }
                if ui.radio(spacing, "Spacing").clicked() && !spacing {
                    params.stations = StationChoice::Spacing(0.5);
                }
                match &mut params.stations {
                    StationChoice::Count(n) => {
                        ui.add(theme::num(n).range(2..=seacraft_core::templates::MAX_STATIONS));
                    }
                    StationChoice::Spacing(s) => {
                        ui.add(theme::num(s).speed(0.01).range(0.05..=10.0).suffix(" m"));
                    }
                    StationChoice::At(_) => {
                        ui.label("at a list of positions");
                    }
                }
            });
            ui.end_row();

            let c = &mut params.choices;
            ui.label("Seam drift limit:");
            ui.add(
                theme::num(&mut c.max_seam_drift_mm)
                    .speed(0.1)
                    .range(0.1..=100.0)
                    .suffix(" mm"),
            );
            ui.end_row();
            ui.label("Stretch limit:");
            ui.add(
                theme::num(&mut c.max_stretch_pct)
                    .speed(0.01)
                    .range(0.0001..=10.0)
                    .suffix(" % RMS"),
            );
            ui.end_row();
            ui.label("Offsets every:");
            ui.add(
                theme::num(&mut c.offset_spacing_mm)
                    .speed(5.0)
                    .range(10.0..=5000.0)
                    .suffix(" mm"),
            );
            ui.end_row();
            ui.label("Plywood sheet:");
            ui.horizontal(|ui| {
                ui.add(egui::TextEdit::singleline(&mut c.stock_mm).desired_width(90.0));
                ui.label("mm, length x width");
            });
            ui.end_row();
            ui.label("Scarf overlap:");
            ui.add(
                theme::num(&mut c.scarf_mm)
                    .speed(1.0)
                    .range(0.0..=1000.0)
                    .suffix(" mm"),
            );
            ui.end_row();
            ui.label("Nesting:");
            let current_nest = NEST_MODES
                .iter()
                .find(|(k, _)| c.nest.trim().eq_ignore_ascii_case(k))
                .map_or(c.nest.as_str(), |(_, label)| *label);
            egui::ComboBox::from_id_salt("panels_nest")
                .selected_text(current_nest)
                .show_ui(ui, |ui| {
                    for (key, label) in NEST_MODES {
                        ui.selectable_value(&mut c.nest, key.to_owned(), label);
                    }
                });
            ui.end_row();
            if !c.nest.trim().eq_ignore_ascii_case("none") {
                ui.label("Gap between pieces:");
                ui.add(
                    theme::num(&mut c.gap_mm)
                        .speed(1.0)
                        .range(1.0..=500.0)
                        .suffix(" mm"),
                );
                ui.end_row();
            }
            ui.label("Units:");
            ui.checkbox(&mut params.imperial, "Imperial beside metric");
            ui.end_row();
        });
}

/// The `Panels | Sheets` switch above the drawing, shown when a layout exists.
pub fn panels_view_switch(ui: &mut egui::Ui, show_sheets: bool) -> Option<PanelsAction> {
    let mut next = show_sheets;
    theme::segmented(ui, |ui| {
        ui.selectable_value(&mut next, false, "Panels");
        ui.selectable_value(&mut next, true, "Sheets");
    });
    (next != show_sheets).then_some(PanelsAction::ShowSheets(next))
}

/// The export options and the Export button, enabled when `block` is `None` and otherwise
/// disabled with `block` as its hover text.
pub fn panels_export(
    ui: &mut egui::Ui,
    params: &mut PanelsParams,
    block: Option<&str>,
) -> Option<PanelsAction> {
    egui::Grid::new("panels_export_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label("Paper:");
            egui::ComboBox::from_id_salt("panels_paper")
                .selected_text(params.paper.to_uppercase())
                .show_ui(ui, |ui| {
                    for paper in ["a4", "a3", "letter"] {
                        ui.selectable_value(
                            &mut params.paper,
                            paper.to_owned(),
                            paper.to_uppercase(),
                        );
                    }
                });
            ui.end_row();
            ui.label("");
            ui.checkbox(&mut params.single_page, "One page a panel (PDF)");
            ui.end_row();
            ui.label("");
            ui.checkbox(
                &mut params.offsets_pages,
                "Offsets tables after the patterns (PDF)",
            );
            ui.end_row();
            ui.label("");
            ui.checkbox(
                &mut params.allow_undevelopable,
                "Write patterns even when a panel does not lie flat",
            );
            ui.end_row();
        });
    let button = ui.add_enabled(block.is_none(), egui::Button::new("Export…"));
    let button = match block {
        Some(reason) => button.on_disabled_hover_text(reason),
        None => button.on_hover_text("Write the patterns as PDF, SVG or DXF"),
    };
    button.clicked().then_some(PanelsAction::Export)
}

/// The panel list: one row a panel, refused panels with why, then the plywood count. A list
/// measured for older options is drawn faded under "Out of date". Clicking a panel's name asks
/// to frame it.
pub fn panels_list(
    ui: &mut egui::Ui,
    status: &PanelsStatus<'_>,
    selected: Option<usize>,
) -> Option<PanelsAction> {
    let set = match status {
        PanelsStatus::Working => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Laying the panels flat…");
            });
            return None;
        }
        PanelsStatus::Failed(reason) => {
            ui.colored_label(theme::TEXT_PRIMARY, *reason);
            return None;
        }
        PanelsStatus::Ready(set) | PanelsStatus::Stale(set) => *set,
    };
    let stale = matches!(status, PanelsStatus::Stale(_));
    if stale {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.colored_label(theme::TEXT_MUTED, "Out of date: measuring again…");
        });
    }
    let text = |s: String| {
        if stale {
            egui::RichText::new(s).color(theme::TEXT_MUTED)
        } else {
            egui::RichText::new(s)
        }
    };
    let mut action = None;
    egui::Grid::new("panels_list")
        .num_columns(8)
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            for h in [
                "Panel",
                "Copies",
                "Lies flat",
                "RMS stretch",
                "Max stretch",
                "Seam drift",
                "Area",
                "Sheets each",
            ] {
                ui.label(egui::RichText::new(h).color(theme::TEXT_LABEL));
            }
            ui.end_row();
            for (i, r) in set.rows.iter().enumerate() {
                if ui
                    .selectable_label(selected == Some(i), text(r.panel.clone()))
                    .clicked()
                {
                    action = Some(PanelsAction::Frame(i));
                }
                ui.label(text(r.copies.to_string()));
                ui.label(text(if r.lies_flat {
                    "✓".to_owned()
                } else {
                    "does not lie flat".to_owned()
                }));
                ui.label(text(format!("{:.3} %", r.rms_stretch_pct)));
                ui.label(text(format!("{:.3} %", r.max_stretch_pct)));
                ui.label(text(format!("{:.1} mm", r.max_seam_drift_mm)));
                ui.label(text(format!("{:.2} m²", r.area_m2)));
                ui.label(text(
                    r.sheets_each
                        .map_or_else(|| "-".to_owned(), |n| n.to_string()),
                ));
                ui.end_row();
            }
        });
    for r in &set.refused {
        ui.label(text(format!(
            "{} cannot be laid flat: {}",
            r.panel, r.reason
        )));
    }
    ui.label(text(set.sheet_count_line()));
    ui.label(
        egui::RichText::new(
            "Measured against the model, with provisional limits; not a bending test of plywood.",
        )
        .color(theme::TEXT_MUTED),
    );
    action
}
