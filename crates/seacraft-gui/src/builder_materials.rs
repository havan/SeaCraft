//! File > Builder materials: the laminate and seam schedule, what it weighs on the hull, and the
//! total added to a loading condition. [`show`] draws and returns what the user asked for; `App`
//! applies it as an undoable project edit.
//!
//! The figures are the builder's own: nothing here offers a glass or resin value. A new ply, tape
//! or fillet starts at zero, which the schedule refuses until a number is entered.

use eframe::egui;
use seacraft_core::i18n::{Key, Tr};
use seacraft_core::materials::{
    Estimate, Fillet, Materials, Ply, PlywoodStock, Side, Tape, TapeSides,
};

use crate::theme;

/// What the user asked the window for.
#[derive(Clone, Debug, PartialEq)]
pub enum MaterialsAction {
    /// Make this the project's schedule (it has passed [`Materials::validate`]).
    SetSchedule(Materials),
    /// Put the shown total in the loading condition called `condition` as an item called
    /// `item_name` (the localized default when empty).
    AddToCondition {
        condition: String,
        item_name: String,
    },
}

/// Where the estimate stands for the current model and schedule.
#[derive(Clone, Copy, Debug)]
pub enum MaterialsStatus<'a> {
    /// Nothing weighed yet.
    Computing,
    /// Weighed for the current model and schedule.
    Ready(&'a Estimate),
    /// Weighed for an older model or schedule; the current one is being weighed.
    Stale(&'a Estimate),
    /// The current model and schedule give no estimate; the reason.
    Failed(&'a str),
}

impl<'a> MaterialsStatus<'a> {
    /// The estimate to show, current or not.
    pub fn estimate(&self) -> Option<&'a Estimate> {
        match self {
            MaterialsStatus::Ready(e) | MaterialsStatus::Stale(e) => Some(e),
            _ => None,
        }
    }
}

/// The dialog's own state: the schedule being edited, which becomes the project's once it is
/// complete, and the add-to-condition choices.
#[derive(Clone, Debug, Default)]
pub struct MaterialsDialogState {
    /// The schedule as edited, perhaps incomplete.
    draft: Materials,
    /// The project's schedule the draft last matched: the project changing away from it (undo,
    /// open, the control channel) starts the draft again from the project's.
    applied: Materials,
    /// The hull's layer names, each once.
    layers: Vec<String>,
    /// The loading condition to add to.
    pub condition: String,
    /// The item's name; empty means the localized default.
    pub item_name: String,
}

impl MaterialsDialogState {
    /// Follow the project's schedule (`None` is an empty one) and the hull's layer names. An
    /// unchanged schedule keeps the draft, finished or not.
    pub fn sync(&mut self, schedule: Option<&Materials>, layers: &[String]) {
        let current = schedule.cloned().unwrap_or_default();
        if current != self.applied {
            self.draft = current.clone();
            self.applied = current;
        }
        self.layers.clear();
        for name in layers {
            if !self.layers.contains(name) {
                self.layers.push(name.clone());
            }
        }
    }

    /// The schedule as edited.
    pub fn draft(&self) -> &Materials {
        &self.draft
    }
}

/// The schedule editor, the estimate and the add button. Returns what the user asked for.
pub fn show(
    ui: &mut egui::Ui,
    state: &mut MaterialsDialogState,
    status: MaterialsStatus<'_>,
    conditions: &[String],
    tr: Tr,
) -> Option<MaterialsAction> {
    let mut action = None;
    let mut invalid = None;
    egui::Panel::left("materials_schedule")
        .resizable(false)
        .default_size(400.0)
        .show(ui, |ui| {
            egui::ScrollArea::vertical()
                .id_salt("materials_schedule_scroll")
                .show(ui, |ui| {
                    schedule_editor(ui, state, tr);
                    invalid = state.draft.validate().err();
                    if let Some(reason) = &invalid {
                        ui.add_space(6.0);
                        ui.colored_label(
                            theme::WARN,
                            tr.fmt(Key::MaterialsInvalid, &[("reason", reason)]),
                        );
                    }
                });
        });
    if invalid.is_none() && state.draft != state.applied {
        state.applied = state.draft.clone();
        action = Some(MaterialsAction::SetSchedule(state.draft.clone()));
    }
    let unapplied = state.draft != state.applied;
    // The add button below the results, always in view however long they are.
    egui::Panel::bottom("materials_add_panel")
        .resizable(false)
        .show(ui, |ui| {
            ui.add_space(4.0);
            let add = add_section(ui, state, &status, conditions, unapplied, tr);
            if action.is_none() {
                action = add;
            }
            ui.add_space(4.0);
        });
    egui::CentralPanel::default().show(ui, |ui| {
        egui::ScrollArea::vertical()
            .id_salt("materials_results_scroll")
            .show(ui, |ui| results(ui, &status, tr));
    });
    action
}

fn side_label(side: Side, tr: Tr) -> &'static str {
    match side {
        Side::Outside => tr.s(Key::MaterialsSideOutside),
        Side::Inside => tr.s(Key::MaterialsSideInside),
    }
}

fn tape_sides_label(sides: TapeSides, tr: Tr) -> &'static str {
    match sides {
        TapeSides::Outside => tr.s(Key::MaterialsSideOutside),
        TapeSides::Inside => tr.s(Key::MaterialsSideInside),
        TapeSides::Both => tr.s(Key::MaterialsSideBoth),
    }
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).color(theme::TEXT_LABEL).strong());
}

/// Resin ratio, plies by layer, tape, fillet and plywood stock, edited in `state.draft`.
fn schedule_editor(ui: &mut egui::Ui, state: &mut MaterialsDialogState, tr: Tr) {
    let draft = &mut state.draft;
    ui.horizontal(|ui| {
        ui.label(tr.s(Key::MaterialsResinColon));
        let mut resin = draft.resin_per_glass.unwrap_or(0.0);
        if ui
            .add(theme::num(&mut resin).speed(0.01).range(0.0..=20.0))
            .changed()
        {
            draft.resin_per_glass = (resin > 0.0).then_some(resin);
        }
    });
    ui.label(egui::RichText::new(tr.s(Key::MaterialsResinHint)).color(theme::TEXT_MUTED));
    ui.add_space(6.0);

    heading(ui, tr.s(Key::MaterialsGlassHeading));
    // The hull's layers, then any the schedule names that the hull does not have, so they can
    // be emptied.
    let mut names: Vec<(String, bool)> = state.layers.iter().map(|n| (n.clone(), true)).collect();
    for name in draft.layers.keys() {
        if !state.layers.contains(name) {
            names.push((name.clone(), false));
        }
    }
    let mut remove = None;
    let mut add = None;
    for (name, on_hull) in &names {
        ui.add_space(2.0);
        if *on_hull {
            ui.label(name.as_str());
        } else {
            ui.colored_label(
                theme::WARN,
                tr.fmt(Key::MaterialsLayerMissing, &[("name", name)]),
            );
        }
        ui.indent(("materials_layer", name.as_str()), |ui| {
            match draft.layers.get_mut(name) {
                Some(plies) if !plies.is_empty() => {
                    for (i, ply) in plies.iter_mut().enumerate() {
                        ui.horizontal(|ui| {
                            ui.add(
                                theme::num(&mut ply.g_m2)
                                    .speed(1.0)
                                    .range(0.0..=10000.0)
                                    .suffix(" g/m²"),
                            )
                            .on_hover_text(tr.s(Key::MaterialsPlyGlassTip));
                            egui::ComboBox::from_id_salt(("materials_ply_side", name.as_str(), i))
                                .selected_text(side_label(ply.side, tr))
                                .show_ui(ui, |ui| {
                                    for side in [Side::Outside, Side::Inside] {
                                        ui.selectable_value(
                                            &mut ply.side,
                                            side,
                                            side_label(side, tr),
                                        );
                                    }
                                });
                            if ui.button(tr.s(Key::MaterialsRemovePly)).clicked() {
                                remove = Some((name.clone(), i));
                            }
                        });
                    }
                }
                _ => {
                    ui.label(
                        egui::RichText::new(tr.s(Key::MaterialsNoPlies)).color(theme::TEXT_MUTED),
                    );
                }
            }
            if *on_hull && ui.button(tr.s(Key::MaterialsAddPly)).clicked() {
                add = Some(name.clone());
            }
        });
    }
    if let Some((name, i)) = remove {
        if let Some(plies) = draft.layers.get_mut(&name) {
            plies.remove(i);
            if plies.is_empty() {
                draft.layers.remove(&name);
            }
        }
    }
    if let Some(name) = add {
        draft.layers.entry(name).or_default().push(Ply {
            g_m2: 0.0,
            side: Side::Outside,
        });
    }
    ui.add_space(6.0);
    ui.separator();

    let mut tape_on = draft.seams.tape.is_some();
    if ui
        .checkbox(&mut tape_on, tr.s(Key::MaterialsTapeCheck))
        .changed()
    {
        draft.seams.tape = tape_on.then_some(Tape {
            width_mm: 0.0,
            g_m2: 0.0,
            plies: 1,
            sides: TapeSides::Both,
        });
    }
    if let Some(tape) = &mut draft.seams.tape {
        egui::Grid::new("materials_tape")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(tr.s(Key::MaterialsTapeWidthColon));
                ui.add(
                    theme::num(&mut tape.width_mm)
                        .speed(1.0)
                        .range(0.0..=2000.0)
                        .suffix(" mm"),
                );
                ui.end_row();
                ui.label(tr.s(Key::MaterialsTapeGlassColon));
                ui.add(
                    theme::num(&mut tape.g_m2)
                        .speed(1.0)
                        .range(0.0..=10000.0)
                        .suffix(" g/m²"),
                );
                ui.end_row();
                ui.label(tr.s(Key::MaterialsTapePliesColon));
                ui.add(theme::num(&mut tape.plies).range(1..=20));
                ui.end_row();
                ui.label(tr.s(Key::MaterialsTapeSidesColon));
                egui::ComboBox::from_id_salt("materials_tape_sides")
                    .selected_text(tape_sides_label(tape.sides, tr))
                    .show_ui(ui, |ui| {
                        for sides in [TapeSides::Outside, TapeSides::Inside, TapeSides::Both] {
                            ui.selectable_value(
                                &mut tape.sides,
                                sides,
                                tape_sides_label(sides, tr),
                            );
                        }
                    });
                ui.end_row();
            });
    }

    let mut fillet_on = draft.seams.fillet.is_some();
    if ui
        .checkbox(&mut fillet_on, tr.s(Key::MaterialsFilletCheck))
        .changed()
    {
        draft.seams.fillet = fillet_on.then_some(Fillet {
            radius_mm: 0.0,
            density_kg_m3: 0.0,
        });
    }
    if let Some(fillet) = &mut draft.seams.fillet {
        egui::Grid::new("materials_fillet")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(tr.s(Key::MaterialsFilletRadiusColon));
                ui.add(
                    theme::num(&mut fillet.radius_mm)
                        .speed(0.5)
                        .range(0.0..=500.0)
                        .suffix(" mm"),
                );
                ui.end_row();
                ui.label(tr.s(Key::MaterialsFilletDensityColon));
                ui.add(
                    theme::num(&mut fillet.density_kg_m3)
                        .speed(5.0)
                        .range(0.0..=5000.0)
                        .suffix(" kg/m³"),
                );
                ui.end_row();
            });
    }

    // Unticked, the sheets are counted on the panel layout's default stock.
    let default_stock = seacraft_core::panel_choices::Choices::default().stock_mm;
    let mut stock_on = draft.plywood.is_some();
    if ui
        .checkbox(
            &mut stock_on,
            tr.fmt(Key::MaterialsPlywoodCheck, &[("stock", &default_stock)]),
        )
        .changed()
    {
        draft.plywood = stock_on.then(|| PlywoodStock {
            stock_mm: default_stock.clone(),
            scarf_mm: 0.0,
        });
    }
    if let Some(stock) = &mut draft.plywood {
        egui::Grid::new("materials_plywood")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(tr.s(Key::PanelsPlywoodSheetColon));
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut stock.stock_mm).desired_width(90.0));
                    ui.label(tr.s(Key::PanelsPlywoodHint));
                });
                ui.end_row();
                ui.label(tr.s(Key::PanelsScarfColon));
                ui.add(
                    theme::num(&mut stock.scarf_mm)
                        .speed(1.0)
                        .range(0.0..=1000.0)
                        .suffix(" mm"),
                );
                ui.end_row();
            });
    }
}

/// The estimate's tables, total, warnings and notes; faded under "Out of date" when stale.
fn results(ui: &mut egui::Ui, status: &MaterialsStatus<'_>, tr: Tr) {
    let est = match status {
        MaterialsStatus::Computing => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr.s(Key::MaterialsComputing));
            });
            return;
        }
        MaterialsStatus::Failed(reason) => {
            ui.colored_label(theme::FAIL, *reason);
            return;
        }
        MaterialsStatus::Ready(est) | MaterialsStatus::Stale(est) => *est,
    };
    let stale = matches!(status, MaterialsStatus::Stale(_));
    if stale {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.colored_label(theme::TEXT_MUTED, tr.s(Key::MaterialsStale));
        });
    }
    let text = |s: String| {
        if stale {
            egui::RichText::new(s).color(theme::TEXT_MUTED)
        } else {
            egui::RichText::new(s)
        }
    };
    let header = |ui: &mut egui::Ui, keys: &[Key]| {
        for k in keys {
            ui.label(egui::RichText::new(tr.s(*k)).color(theme::TEXT_LABEL));
        }
        ui.end_row();
    };

    egui::Grid::new("materials_layers")
        .num_columns(6)
        .striped(true)
        .spacing([10.0, 3.0])
        .show(ui, |ui| {
            header(
                ui,
                &[
                    Key::ReportMatColLayer,
                    Key::ReportMatColArea,
                    Key::ReportMatColShell,
                    Key::ReportMatColGlass,
                    Key::ReportMatColResin,
                    Key::ReportMatColTotal,
                ],
            );
            for r in &est.layers {
                ui.label(text(r.layer.clone()));
                for v in [r.area_m2, r.shell_kg, r.glass_kg, r.resin_kg, r.total_kg] {
                    ui.label(text(format!("{v:.2}")));
                }
                ui.end_row();
            }
        });

    if !est.seams.is_empty() {
        ui.add_space(6.0);
        ui.label(text(tr.s(Key::ReportMatSeamsHeading).to_owned()).strong());
        egui::Grid::new("materials_seams")
            .num_columns(5)
            .striped(true)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                header(
                    ui,
                    &[
                        Key::ReportMatColBetween,
                        Key::ReportMatColLength,
                        Key::ReportMatColTape,
                        Key::ReportMatColFillet,
                        Key::ReportMatColTotal,
                    ],
                );
                for r in &est.seams {
                    ui.label(text(format!("{} / {}", r.between[0], r.between[1])));
                    for v in [
                        r.length_m,
                        r.tape_kg + r.tape_resin_kg,
                        r.fillet_kg,
                        r.total_kg,
                    ] {
                        ui.label(text(format!("{v:.2}")));
                    }
                    ui.end_row();
                }
            });
    }

    if !est.plywood.is_empty() {
        ui.add_space(6.0);
        ui.label(text(tr.s(Key::ReportMatPlywoodHeading).to_owned()).strong());
        egui::Grid::new("materials_plywood_rows")
            .num_columns(4)
            .striped(true)
            .spacing([10.0, 3.0])
            .show(ui, |ui| {
                header(
                    ui,
                    &[
                        Key::ReportMatColLayer,
                        Key::ReportMatColThickness,
                        Key::ReportMatColSheets,
                        Key::ReportMatColStatus,
                    ],
                );
                for r in &est.plywood {
                    let status = if !r.not_flat.is_empty() {
                        tr.fmt(
                            Key::ReportMatPlywoodNotFlat,
                            &[("panels", &r.not_flat.join(", "))],
                        )
                    } else if !r.refused.is_empty() {
                        tr.fmt(
                            Key::ReportMatPlywoodRefused,
                            &[("panels", &r.refused.join(", "))],
                        )
                    } else if r.all_lie_flat {
                        tr.s(Key::ReportMatPlywoodFlat).to_owned()
                    } else {
                        String::new()
                    };
                    ui.label(text(r.layer.clone()));
                    ui.label(text(format!("{:.1}", r.thickness_mm)));
                    ui.label(text(r.sheets.to_string()));
                    ui.label(text(status));
                    ui.end_row();
                }
            });
    }

    ui.add_space(6.0);
    let [x, y, z] = est.total.centre;
    let total = tr.fmt(
        Key::ReportMatTotalLine,
        &[
            ("kg", &format!("{:.1}", est.total.mass_kg)),
            ("t", &format!("{:.3}", est.total.mass_t)),
            ("x", &format!("{x:.3}")),
            ("y", &format!("{y:.3}")),
            ("z", &format!("{z:.3}")),
        ],
    );
    ui.label(if stale {
        egui::RichText::new(total).color(theme::TEXT_MUTED)
    } else {
        egui::RichText::new(total)
            .color(theme::TEXT_PRIMARY)
            .strong()
    });
    if !est.has_glass {
        ui.label(text(tr.s(Key::ReportMatNoGlass).to_owned()));
    }
    for w in &est.warnings {
        let line = tr.fmt(Key::CommonWarningPrefix, &[("msg", &w.render(tr))]);
        ui.colored_label(
            if stale {
                theme::TEXT_MUTED
            } else {
                theme::WARN
            },
            line,
        );
    }
    ui.add_space(4.0);
    ui.label(egui::RichText::new(tr.s(Key::ReportMatNote)).color(theme::TEXT_MUTED));
    ui.label(
        egui::RichText::new(seacraft_core::not_validated_note(tr.locale()))
            .color(theme::TEXT_MUTED),
    );
}

/// The condition picker, the item name and the add button, enabled only for a current estimate
/// of the schedule as shown.
fn add_section(
    ui: &mut egui::Ui,
    state: &mut MaterialsDialogState,
    status: &MaterialsStatus<'_>,
    conditions: &[String],
    unapplied: bool,
    tr: Tr,
) -> Option<MaterialsAction> {
    heading(ui, tr.s(Key::MaterialsAddHeading));
    if conditions.is_empty() {
        ui.label(egui::RichText::new(tr.s(Key::MaterialsAddNoConditions)).color(theme::TEXT_MUTED));
    } else {
        if !conditions.contains(&state.condition) {
            state.condition = conditions[0].clone();
        }
        egui::Grid::new("materials_add")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label(tr.s(Key::MaterialsConditionColon));
                egui::ComboBox::from_id_salt("materials_condition")
                    .selected_text(state.condition.as_str())
                    .show_ui(ui, |ui| {
                        for name in conditions {
                            ui.selectable_value(&mut state.condition, name.clone(), name);
                        }
                    });
                ui.end_row();
                ui.label(tr.s(Key::MaterialsItemColon));
                ui.add(egui::TextEdit::singleline(&mut state.item_name).desired_width(240.0));
                ui.end_row();
            });
    }
    let block = if conditions.is_empty() {
        Some(tr.s(Key::MaterialsAddNoConditions))
    } else if unapplied {
        Some(tr.s(Key::MaterialsAddWaitSchedule))
    } else if !matches!(status, MaterialsStatus::Ready(_)) {
        Some(tr.s(Key::MaterialsAddWait))
    } else {
        None
    };
    let button = ui.add_enabled(
        block.is_none(),
        egui::Button::new(tr.s(Key::MaterialsAddButton)),
    );
    let button = match block {
        Some(reason) => button.on_disabled_hover_text(reason),
        None => button.on_hover_text(tr.s(Key::MaterialsAddTip)),
    };
    button.clicked().then(|| MaterialsAction::AddToCondition {
        condition: state.condition.clone(),
        item_name: state.item_name.trim().to_owned(),
    })
}
