//! Side docks (the layer manager, the Inspector, the Analysis dock tabs) and
//! the toolbar under the menu bar.

use eframe::egui;
use seacraft_core::damage::{Compartment, CompartmentSpec, DamageCase, DamageResult};
use seacraft_core::hydro::{HydroOptions, Hydrostatics, SectionArea};
use seacraft_core::loading::{LoadingCondition, LoadingResult, Tank, WeightItem};
use seacraft_core::math::Vec3;
use seacraft_core::project::{Project, ReportSeakeeping, ReportSettings, Requirements};
use seacraft_core::seastate::WaveSpectrum;
use seacraft_core::stability::CrossCurve;
use seacraft_core::weather::{WeatherResult, WindageItem, WindageRect};
use seacraft_core::{AnalysisMode, SubdivisionMode, Surface};

use crate::commands::TemplatesParams;
use crate::render::DisplayOptions;
use crate::theme;

/// What the user asked the layer panel to do. The app applies it through
/// [`crate::commands`], one undo step per edit.
#[derive(Debug, Clone, PartialEq, Default)]
pub enum LayerAction {
    #[default]
    None,
    /// Make a layer active (the one the properties show and new faces get).
    Activate(usize),
    /// Add a layer.
    Add,
    /// Delete a layer, moving its faces to `move_faces_to`.
    Delete {
        layer: usize,
        move_faces_to: Option<usize>,
    },
    Rename {
        layer: usize,
        name: String,
    },
    Reorder {
        layer: usize,
        dir: crate::commands::LayerDirection,
    },
    /// Change one or more properties.
    Set {
        layer: usize,
        props: crate::commands::LayerProps,
    },
    /// Move faces onto a layer.
    AssignFaces {
        faces: Vec<usize>,
        layer: usize,
    },
}

/// A numeric layer property being edited in the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayerField {
    Density,
    Thickness,
}

/// The panel's own editing state, kept by the app between frames.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct LayerPanelState {
    /// The name being typed for a layer; committed when the field loses
    /// focus.
    pub name_edit: Option<(usize, String)>,
    /// A density or thickness being dragged (layer, field, value in the
    /// panel's units): committed on release, so a drag is one edit.
    pub pending: Option<(usize, LayerField, f64)>,
}

/// Everything the layer panel draws.
pub struct LayersPanel<'a> {
    pub surface: &'a Surface,
    /// The active layer (clamped by the caller).
    pub active: usize,
    /// The control face under the last click in a viewport.
    pub picked_face: Option<usize>,
    pub state: &'a mut LayerPanelState,
}

/// A small filled square in a layer's colour.
fn layer_swatch(ui: &mut egui::Ui, color: i32, size: f32) -> egui::Response {
    let (rect, response) = ui.allocate_exact_size(egui::vec2(size, size), egui::Sense::click());
    ui.painter()
        .rect_filled(rect, 2.0, theme::layer_colour(color));
    ui.painter().rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::FRAME),
        egui::StrokeKind::Inside,
    );
    response
}

/// The layer manager, after FREE!ship's layer dialog: a list (colour,
/// name, visibility, face count) with the active layer highlighted, Add /
/// Delete / ↑ / ↓, and the active layer's properties underneath, including
/// "Move picked face to this layer".
///
/// Hiding every layer is allowed: the viewports then show no hull and the
/// panel says so, which is easier to undo than a refusal.
pub fn layers(ui: &mut egui::Ui, panel: LayersPanel<'_>) -> LayerAction {
    use crate::commands::{LayerDirection, LayerProps};
    let LayersPanel {
        surface,
        active,
        picked_face,
        state,
    } = panel;
    let mut action = LayerAction::None;
    let set = |layer: usize, props: LayerProps| LayerAction::Set { layer, props };
    let counts = surface.layer_face_counts();
    let n = surface.layers.len();
    let active = active.min(n.saturating_sub(1));

    // ---- heading and buttons, on one row ------------------------------------
    ui.horizontal(|ui| {
        ui.heading("Layers");
        // Right to left: added last-first, they read Add, Delete, ↑, ↓.
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.spacing_mut().item_spacing.x = 3.0;
            ui.spacing_mut().button_padding.x = 5.0;
            if ui
                .add_enabled(active + 1 < n, egui::Button::new("↓"))
                .on_hover_text("Move the layer down the list")
                .clicked()
            {
                action = LayerAction::Reorder {
                    layer: active,
                    dir: LayerDirection::Down,
                };
            }
            if ui
                .add_enabled(active > 0, egui::Button::new("↑"))
                .on_hover_text("Move the layer up the list")
                .clicked()
            {
                action = LayerAction::Reorder {
                    layer: active,
                    dir: LayerDirection::Up,
                };
            }
            // Delete: the faces go to the neighbouring layer (the one above,
            // or below for the first), so the button never strands a face.
            let neighbour = if active > 0 { active - 1 } else { 1 };
            let owned = counts.get(active).copied().unwrap_or(0);
            let tip = if n < 2 {
                "The last layer cannot be deleted".to_owned()
            } else if owned > 0 {
                format!(
                    "Delete \"{}\"; its {owned} face{} move to \"{}\"",
                    surface.layers[active].name,
                    if owned == 1 { "" } else { "s" },
                    surface.layers[neighbour].name
                )
            } else {
                format!(
                    "Delete \"{}\" (it has no faces)",
                    surface.layers[active].name
                )
            };
            if ui
                .add_enabled(n > 1, egui::Button::new("Delete"))
                .on_hover_text(&tip)
                .on_disabled_hover_text(&tip)
                .clicked()
            {
                action = LayerAction::Delete {
                    layer: active,
                    move_faces_to: (owned > 0).then_some(neighbour),
                };
            }
            if ui
                .button("Add")
                .on_hover_text("Add an empty layer and make it active")
                .clicked()
            {
                action = LayerAction::Add;
            }
        });
    });

    // ---- the list -----------------------------------------------------------
    for (index, layer) in surface.layers.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            if layer_swatch(ui, layer.color, 14.0)
                .on_hover_text("Click to make this layer active")
                .clicked()
            {
                action = LayerAction::Activate(index);
            }
            let count = counts.get(index).copied().unwrap_or(0);
            // Right to left: the face count and the eye stay in a column.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_sized(
                    [28.0, 18.0],
                    egui::Label::new(
                        egui::RichText::new(count.to_string())
                            .monospace()
                            .color(theme::TEXT_SECONDARY),
                    ),
                )
                .on_hover_text(format!(
                    "{count} control face{}",
                    if count == 1 { "" } else { "s" }
                ));
                let eye = if layer.visible {
                    egui::RichText::new("◉").color(theme::TEXT_PRIMARY)
                } else {
                    egui::RichText::new("○").color(theme::TEXT_MUTED)
                };
                if ui
                    .add(egui::Button::new(eye).frame(false))
                    .on_hover_text(if layer.visible {
                        "Visible: click to hide"
                    } else {
                        "Hidden: click to show"
                    })
                    .clicked()
                {
                    action = set(
                        index,
                        LayerProps {
                            visible: Some(!layer.visible),
                            ..Default::default()
                        },
                    );
                }
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let name = egui::RichText::new(&layer.name).color(if layer.visible {
                        theme::TEXT_PRIMARY
                    } else {
                        theme::TEXT_MUTED
                    });
                    let r = ui.add_sized(
                        [ui.available_width(), 18.0],
                        egui::Button::selectable(index == active, (name, egui::Atom::grow())),
                    );
                    if r.on_hover_text("Make this layer active").clicked() {
                        action = LayerAction::Activate(index);
                    }
                });
            });
        });
    }
    if counts.iter().all(|&c| c == 0) {
        ui.label(egui::RichText::new("The model has no faces").color(theme::TEXT_MUTED));
    }
    if !surface.layers.iter().any(|l| l.visible) {
        ui.colored_label(theme::WARN, "No visible layers");
    }

    // ---- the active layer's properties -----------------------------------
    let Some(layer) = surface.layers.get(active) else {
        return action;
    };
    ui.separator();

    // Name: typed, committed when the field loses focus (Enter or a click
    // elsewhere). Escape abandons it.
    if state.name_edit.as_ref().is_none_or(|(i, _)| *i != active) {
        state.name_edit = Some((active, layer.name.clone()));
    }
    let mut commit_name: Option<String> = None;
    if let Some((_, buffer)) = state.name_edit.as_mut() {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Name").color(theme::TEXT_SECONDARY));
            let r = ui.add(
                egui::TextEdit::singleline(buffer)
                    .desired_width(ui.available_width())
                    .id_salt("layer_name"),
            );
            let escaped = ui.input(|i| i.key_pressed(egui::Key::Escape));
            if r.lost_focus() && !escaped && *buffer != layer.name {
                commit_name = Some(buffer.clone());
            } else if !r.has_focus() && *buffer != layer.name {
                // Undo, a refusal or a control-channel rename: show the model.
                *buffer = layer.name.clone();
            }
        });
    }
    if let Some(name) = commit_name {
        action = LayerAction::Rename {
            layer: active,
            name,
        };
    }

    // Colour: the theme's layer palette, one click one step.
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new("Colour").color(theme::TEXT_SECONDARY));
        ui.spacing_mut().item_spacing.x = 3.0;
        for c in theme::LAYER_PALETTE {
            let r = layer_swatch(ui, c, if c == layer.color { 16.0 } else { 13.0 });
            if c == layer.color {
                ui.painter().rect_stroke(
                    r.rect.expand(1.5),
                    2.0,
                    egui::Stroke::new(1.5, theme::TEXT_PRIMARY),
                    egui::StrokeKind::Outside,
                );
            }
            if r.on_hover_text(format!("#{c:06x}")).clicked() && c != layer.color {
                action = set(
                    active,
                    LayerProps {
                        color: Some(c),
                        ..Default::default()
                    },
                );
            }
        }
    });

    // Flags, in FREE!ship's order.
    type Flag = (&'static str, bool, &'static str, fn(bool) -> LayerProps);
    let flags: [Flag; 5] = [
        (
            "Symmetric",
            layer.symmetric,
            "Only y ≥ 0 is modelled; the faces are mirrored across the centreline",
            |v| LayerProps {
                symmetric: Some(v),
                ..Default::default()
            },
        ),
        (
            "Developable",
            layer.developable,
            "The plating is meant to be developable (bent, not pressed)",
            |v| LayerProps {
                developable: Some(v),
                ..Default::default()
            },
        ),
        (
            "Intersections",
            layer.use_for_intersections,
            "Use for intersections: stations, buttocks and waterlines are cut through this layer",
            |v| LayerProps {
                use_for_intersections: Some(v),
                ..Default::default()
            },
        ),
        (
            "Hydrostatics",
            layer.use_in_hydrostatics,
            "Use in hydrostatics: the faces count in displacement, stability and every measurement",
            |v| LayerProps {
                use_in_hydrostatics: Some(v),
                ..Default::default()
            },
        ),
        (
            "Lines plan",
            layer.show_in_linesplan,
            "Show in lines plan: the layer's curves are drawn in the lines plan",
            |v| LayerProps {
                show_in_linesplan: Some(v),
                ..Default::default()
            },
        ),
    ];
    egui::Grid::new("layer_flags")
        .num_columns(2)
        .spacing(egui::vec2(8.0, 2.0))
        .show(ui, |ui| {
            for (i, (label, on, tip, props)) in flags.into_iter().enumerate() {
                let mut v = on;
                if ui.checkbox(&mut v, label).on_hover_text(tip).changed() {
                    action = set(active, props(v));
                }
                if i % 2 == 1 {
                    ui.end_row();
                }
            }
        });

    // Density (t/m³) and plate thickness (mm); the model keeps kg/m³ and m.
    if state.pending.is_some_and(|(i, _, _)| i != active) {
        state.pending = None;
    }
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        ui.label(egui::RichText::new("Plating").color(theme::TEXT_SECONDARY))
            .on_hover_text("Material density and plate thickness, for a weight estimate");
        for (field, label, unit, model, speed, tip) in [
            (
                LayerField::Density,
                "Density",
                "t/m³",
                layer.density / 1000.0,
                0.01,
                "Material density of the plating, for a weight estimate",
            ),
            (
                LayerField::Thickness,
                "Thickness",
                "mm",
                layer.thickness * 1000.0,
                0.1,
                "Plate thickness, for a weight estimate",
            ),
        ] {
            let mut v = match state.pending {
                Some((_, f, v)) if f == field => v,
                _ => model,
            };
            let r = ui
                .add_sized(
                    [52.0, 20.0],
                    egui::DragValue::new(&mut v)
                        .speed(speed)
                        .range(match field {
                            LayerField::Density => 0.0..=25.0,
                            LayerField::Thickness => 0.0..=1000.0,
                        })
                        .fixed_decimals(match field {
                            LayerField::Density => 3,
                            LayerField::Thickness => 1,
                        })
                        .update_while_editing(false),
                )
                .on_hover_text(format!("{label}: {tip}"));
            let mut commit: Option<f64> = None;
            if r.changed() {
                if r.dragged() {
                    state.pending = Some((active, field, v));
                } else {
                    commit = Some(v);
                }
            }
            if r.drag_stopped() {
                commit = state
                    .pending
                    .filter(|(_, f, _)| *f == field)
                    .map(|(_, _, p)| p)
                    .or(commit);
                state.pending = None;
            }
            if let Some(v) = commit.filter(|v| (v - model).abs() > 1e-12) {
                state.pending = None;
                action = set(
                    active,
                    match field {
                        LayerField::Density => LayerProps {
                            density: Some(v * 1000.0),
                            ..Default::default()
                        },
                        LayerField::Thickness => LayerProps {
                            thickness: Some(v / 1000.0),
                            ..Default::default()
                        },
                    },
                );
            }
            ui.label(egui::RichText::new(unit).color(theme::TEXT_MUTED));
        }
    });

    // Move to layer: the face under the last click.
    let picked = picked_face.filter(|&f| f < surface.faces.len());
    let picked_layer = picked.and_then(|f| surface.face_layer.get(f).copied());
    let picked_text = match (picked, picked_layer.and_then(|l| surface.layers.get(l))) {
        (Some(f), Some(l)) => format!("picked face {f} on {}", l.name),
        _ => "no face picked".to_owned(),
    };
    ui.label(
        egui::RichText::new(format!(
            "id {} · {} face{} · {picked_text}",
            layer.id,
            counts[active],
            if counts[active] == 1 { "" } else { "s" }
        ))
        .small()
        .color(theme::TEXT_MUTED),
    );
    let can_move = picked.is_some() && picked_layer != Some(active);
    let tip = "Click a control face in any view (inside the amber control net, \
               away from the points), then move it onto this layer. One undo step.";
    if ui
        .add_enabled(
            can_move,
            egui::Button::new("Move picked face to this layer"),
        )
        .on_hover_text(tip)
        .on_disabled_hover_text(tip)
        .clicked()
    {
        if let Some(f) = picked {
            action = LayerAction::AssignFaces {
                faces: vec![f],
                layer: active,
            };
        }
    }
    action
}

/// What the user asked the selection Inspector to do.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum InspectorAction {
    #[default]
    None,
    /// Move control point `index` to `to` (one undo step; the app clamps a
    /// symmetric layer's point to `y >= 0`).
    Move { index: usize, to: Vec3 },
}

/// Everything the Inspector draws.
pub struct InspectorPanel<'a> {
    pub surface: Option<&'a Surface>,
    pub selected: Option<usize>,
    /// A value being dragged: committed on release, so a drag is one edit.
    pub pending: &'a mut Option<(usize, [f64; 3])>,
}

/// The Inspector's answer: the action and where its x/y/z fields were
/// drawn (`Rect::NOTHING` when no point is selected).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InspectorOutput {
    pub action: InspectorAction,
    pub fields: [egui::Rect; 3],
}

impl Default for InspectorOutput {
    fn default() -> Self {
        Self {
            action: InspectorAction::None,
            fields: [egui::Rect::NOTHING; 3],
        }
    }
}

/// The names of the layers whose faces use control point `point`.
fn point_layers(surface: &Surface, point: usize) -> Vec<&str> {
    let mut names: Vec<&str> = Vec::new();
    for (fi, f) in surface.faces.iter().enumerate() {
        if !f.contains(&point) {
            continue;
        }
        if let Some(layer) = surface
            .face_layer
            .get(fi)
            .and_then(|&li| surface.layers.get(li))
        {
            if !names.contains(&layer.name.as_str()) {
                names.push(&layer.name);
            }
        }
    }
    names
}

/// The selection Inspector: the selected control point's layer, editable
/// x/y/z in metres and its vertex kind and crease edges (read-only).
///
/// A typed value is committed when the field loses focus (Enter or a click
/// elsewhere), a dragged one when the drag is released, so either is one
/// undo step. Escape while typing abandons the value.
pub fn inspector(ui: &mut egui::Ui, panel: InspectorPanel<'_>) -> InspectorOutput {
    let mut out = InspectorOutput::default();
    ui.heading("Inspector");
    ui.add_space(4.0);
    let point = match (panel.surface, panel.selected) {
        (Some(surface), Some(index)) if index < surface.points.len() => Some((surface, index)),
        _ => None,
    };
    let Some((surface, index)) = point else {
        *panel.pending = None;
        ui.label(
            egui::RichText::new("Click a point to edit it — Esc clears").color(theme::TEXT_MUTED),
        );
        ui.add_space(4.0);
        return out;
    };
    if panel.pending.is_some_and(|(i, _)| i != index) {
        *panel.pending = None;
    }
    let cp = &surface.points[index];
    let layers = point_layers(surface, index);
    let layer = if layers.is_empty() {
        "no faces".to_owned()
    } else {
        layers.join(", ")
    };
    ui.label(
        egui::RichText::new(format!("Point {index}"))
            .strong()
            .color(theme::TEXT_PRIMARY),
    );
    ui.label(egui::RichText::new(layer).color(theme::TEXT_SECONDARY));
    ui.add_space(2.0);

    let mut shown = panel.pending.map_or([cp.p.x, cp.p.y, cp.p.z], |(_, v)| v);
    let mut commit: Option<[f64; 3]> = None;
    egui::Grid::new("inspector_xyz")
        .num_columns(3)
        .spacing(egui::vec2(8.0, 4.0))
        .show(ui, |ui| {
            for (axis, name) in ["x", "y", "z"].into_iter().enumerate() {
                ui.label(egui::RichText::new(name).color(theme::TEXT_SECONDARY));
                let mut v = shown[axis];
                let r = ui.add_sized(
                    [96.0, 20.0],
                    egui::DragValue::new(&mut v)
                        .speed(0.005)
                        .fixed_decimals(3)
                        .update_while_editing(false),
                );
                out.fields[axis] = r.rect;
                if r.changed() {
                    shown[axis] = v;
                    if r.dragged() {
                        *panel.pending = Some((index, shown));
                    } else {
                        commit = Some(shown);
                    }
                }
                if r.drag_stopped() {
                    commit = Some(panel.pending.map_or(shown, |(_, p)| p));
                }
                ui.label(egui::RichText::new("m").color(theme::TEXT_MUTED));
                ui.end_row();
            }
        });
    if let Some(v) = commit {
        *panel.pending = None;
        out.action = InspectorAction::Move {
            index,
            to: Vec3::new(v[0], v[1], v[2]),
        };
    }

    let creases = surface
        .edges
        .iter()
        .filter(|e| e.crease && (e.a == index || e.b == index))
        .count();
    ui.add_space(2.0);
    ui.label(
        egui::RichText::new(format!(
            "{:?} vertex · {creases} crease edge{}",
            cp.kind,
            if creases == 1 { "" } else { "s" }
        ))
        .color(theme::TEXT_MUTED),
    );
    ui.add_space(4.0);
    out
}

/// Format a signed floating-point number with explicit sign for positive values
/// (`+1.234`), minus for negative values (`-1.234`), and unsigned zero (`0.000`)
/// whenever the value rounds to zero at the given decimal precision.
pub fn format_signed(val: f64, decimals: usize) -> String {
    let p = 10f64.powi(decimals as i32);
    if (val * p).round().abs() == 0.0 {
        format!("{:.prec$}", 0.0, prec = decimals)
    } else {
        format!("{:+.*}", decimals, val)
    }
}

/// Format a dimensional or volumetric readout: 3 decimals below 10, 2 from
/// 10 up, so a group of readouts shares one precision rule.
pub fn format_form_value(val: f64) -> String {
    let abs = val.abs();
    let decimals = if abs >= 10.0 { 2 } else { 3 };
    format!("{val:.decimals$}")
}

/// GM at `kg` as a readout value: a dash while KG is unset.
pub fn gm_text(h: &Hydrostatics, kg: Option<f64>) -> String {
    match kg {
        Some(kg) => format_signed(h.gm(kg), 3),
        None => "—".to_owned(),
    }
}

/// The pinned live strip's two rows: displacement, draft and trim; GM
/// and LCB. A dash stands for a quantity not measured yet (GM: or no KG).
pub fn live_strip_rows(h: Option<&Hydrostatics>, kg: Option<f64>) -> [Vec<Readout>; 2] {
    let r = Readout::new;
    match h {
        Some(h) => [
            vec![
                r("Δ", format!("{:.2}", h.displacement_t), "t"),
                r("T", format!("{:.3}", h.draft), "m"),
                r("trim", format_signed(h.trim_m, 3), "m"),
            ],
            vec![
                r("GM", gm_text(h, kg), "m"),
                r("LCB", format!("{:.3}", h.lcb.x), "m"),
            ],
        ],
        None => [
            vec![
                r("Δ", "—".to_owned(), "t"),
                r("T", "—".to_owned(), "m"),
                r("trim", "—".to_owned(), "m"),
            ],
            vec![r("GM", "—".to_owned(), "m"), r("LCB", "—".to_owned(), "m")],
        ],
    }
}

/// The About window: the legal notices GPLv3 section 5(d) asks an interactive program to show,
/// the disclaimer, where the source is, and the licence texts of what is embedded.
pub fn about(ui: &mut egui::Ui) {
    ui.heading(format!("SeaCraft {}", seacraft_core::VERSION));
    ui.label("Copyright (c) 2026 SeaCraft contributors.");
    ui.label(
        "SeaCraft comes with ABSOLUTELY NO WARRANTY. It is free software: you may redistribute it \
         under the terms of the GNU General Public License, version 3 or any later version.",
    );
    ui.colored_label(
        theme::WARN,
        "Results are not validated and may be wrong. Do not rely on them for design or safety \
         decisions without independent checks by a qualified person.",
    );
    ui.hyperlink_to(
        "Source code of this version",
        format!(
            "{}/tree/v{}",
            env!("CARGO_PKG_REPOSITORY"),
            seacraft_core::VERSION
        ),
    );
    ui.separator();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (title, text) in [
            ("Notices", include_str!("../../../NOTICE")),
            (
                "GNU General Public License",
                include_str!("../../../LICENSE"),
            ),
            (
                "Font licence (DejaVu)",
                seacraft_core::fonts::DEJAVU_LICENCE,
            ),
        ] {
            egui::CollapsingHeader::new(title).show(ui, |ui| {
                ui.label(egui::RichText::new(text).small());
            });
        }
    });
}

/// The Analysis dock's pinned strip: the live floating condition in two
/// compact rows, greyed while the hydrostatics are being recomputed, and
/// the *↔ wide* toggle. Returns true when the toggle was clicked.
pub fn live_strip(
    ui: &mut egui::Ui,
    h: Option<&Hydrostatics>,
    kg: Option<f64>,
    stale: bool,
    wide: bool,
) -> bool {
    let mut toggle = false;
    let rows = live_strip_rows(h, kg);
    let value_colour = if stale {
        theme::STALE
    } else {
        theme::TEXT_PRIMARY
    };
    // Fixed cells rather than a `Grid`: a grid's first-frame sizing pass
    // would widen the resizable dock, and fixed cells line the two rows up.
    let cell = |ui: &mut egui::Ui, width: f32, right: bool, text: egui::RichText| {
        let layout = if right {
            egui::Layout::right_to_left(egui::Align::Center)
        } else {
            egui::Layout::left_to_right(egui::Align::Center)
        };
        ui.allocate_ui_with_layout(egui::vec2(width, 18.0), layout, |ui| {
            ui.set_min_width(width);
            ui.label(text);
        });
    };
    ui.add_space(2.0);
    for (r, row) in rows.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 4.0;
            for item in row {
                cell(
                    ui,
                    STRIP_NAME_WIDTH,
                    false,
                    egui::RichText::new(item.name)
                        .font(theme::caption_font())
                        .color(theme::TEXT_SECONDARY),
                );
                cell(
                    ui,
                    STRIP_VALUE_WIDTH,
                    true,
                    egui::RichText::new(&item.value)
                        .monospace()
                        .color(value_colour),
                );
                cell(
                    ui,
                    STRIP_UNIT_WIDTH,
                    false,
                    egui::RichText::new(item.unit)
                        .font(theme::caption_font())
                        .color(theme::TEXT_MUTED),
                );
            }
            if r == 1 {
                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                let label = if wide { "↔ narrow" } else { "↔ wide" };
                toggle = ui
                    .add(egui::Button::new(label).selected(wide))
                    .on_hover_text(
                        "Toggle dock width between standard and wide layout for plots and tables",
                    )
                    .clicked();
            }
        });
    }
    toggle
}

/// A measured value against its design target, e.g.
/// `Δ 12.15 t (12.00 +1.3 %)`.
#[derive(Debug, Clone, PartialEq)]
pub struct TargetReadout {
    pub name: &'static str,
    pub actual: f64,
    pub target: f64,
    pub decimals: usize,
    pub unit: &'static str,
}

impl TargetReadout {
    /// Deviation of the actual from the target, per cent of the target.
    pub fn deviation_pct(&self) -> f64 {
        if self.target.abs() > 1e-12 {
            100.0 * (self.actual - self.target) / self.target
        } else {
            0.0
        }
    }

    /// Within the ± [`crate::new_design::TARGET_TOLERANCE_PCT`] band.
    pub fn on_target(&self) -> bool {
        self.deviation_pct().abs() <= crate::new_design::TARGET_TOLERANCE_PCT
    }

    /// `Δ 12.15 t (12.00 +1.3 %)`.
    pub fn text(&self) -> String {
        let unit = if self.unit.is_empty() {
            String::new()
        } else {
            format!(" {}", self.unit)
        };
        format!(
            "{} {:.*}{unit} ({:.*} {} %)",
            self.name,
            self.decimals,
            self.actual,
            self.decimals,
            self.target,
            format_signed(self.deviation_pct(), 1)
        )
    }
}

/// The measured quantities that have a target in `req`: LWL, BWL, T, Δ, Cb
/// and LCB (per cent of LWL from the waterline's aft end).
pub fn target_readouts(h: &Hydrostatics, req: &Requirements) -> Vec<TargetReadout> {
    let lcb_pct = if h.lwl > 1e-9 {
        100.0 * (h.lcb.x - h.wl_aft_x) / h.lwl
    } else {
        0.0
    };
    let rows: [(&'static str, Option<f64>, f64, usize, &'static str); 6] = [
        ("Δ", req.displacement_t, h.displacement_t, 2, "t"),
        ("LWL", req.lwl, h.lwl, 2, "m"),
        ("BWL", req.bwl, h.bwl, 2, "m"),
        ("T", req.draft, h.draft, 3, "m"),
        ("Cb", req.cb, h.block_coefficient, 3, ""),
        (
            "LCB (%Lwl)",
            req.lcb_frac.map(|f| 100.0 * f),
            lcb_pct,
            1,
            "%",
        ),
    ];
    rows.into_iter()
        .filter_map(|(name, target, actual, decimals, unit)| {
            target.map(|target| TargetReadout {
                name,
                actual,
                target,
                decimals,
                unit,
            })
        })
        .collect()
}

/// The pinned strip's targets: each measured value with its target in the
/// target colour and the deviation in the ok or warn colour (grey while the
/// hydrostatics are being recomputed).
pub fn target_strip(ui: &mut egui::Ui, targets: &[TargetReadout], stale: bool) {
    if targets.is_empty() {
        return;
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        for t in targets {
            let value = if stale {
                theme::STALE
            } else {
                theme::TEXT_PRIMARY
            };
            let deviation = if stale {
                theme::STALE
            } else if t.on_target() {
                theme::OK
            } else {
                theme::WARN
            };
            let font = theme::caption_font();
            let mut job = egui::text::LayoutJob::default();
            let mut push = |text: String, colour: egui::Color32| {
                job.append(&text, 0.0, egui::TextFormat::simple(font.clone(), colour));
            };
            push(format!("{} ", t.name), theme::TEXT_SECONDARY);
            push(format!("{:.*}", t.decimals, t.actual), value);
            if !t.unit.is_empty() {
                push(format!(" {}", t.unit), theme::TEXT_MUTED);
            }
            push(format!(" ({:.*}", t.decimals, t.target), theme::TARGET);
            push(
                format!(" {} %", format_signed(t.deviation_pct(), 1)),
                deviation,
            );
            push(")".to_owned(), theme::TARGET);
            // An item never wraps inside itself: one that does not fit the
            // rest of the row starts the next one.
            ui.add(egui::Label::new(job).wrap_mode(egui::TextWrapMode::Extend))
                .on_hover_text(format!(
                    "{} — target {:.*}, ±{} % tolerance; targets never move the hull",
                    t.text(),
                    t.decimals,
                    t.target,
                    crate::new_design::TARGET_TOLERANCE_PCT
                ));
        }
    });
}

/// Widths of the pinned strip's cells: name, value (right-aligned) and unit.
const STRIP_NAME_WIDTH: f32 = 28.0;
const STRIP_VALUE_WIDTH: f32 = 54.0;
const STRIP_UNIT_WIDTH: f32 = 16.0;

/// What the user asked the hydrostatics dock to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HydroAction {
    None,
    /// The floating condition was edited: recompute after the debounce.
    Changed,
    /// Recompute now.
    Recompute,
    CrossCurves,
    /// The user set KG (it now goes into the project): recompute after the
    /// debounce, as [`HydroAction::Changed`].
    KgSet,
}

/// The largest values the floating-condition fields accept, scaled with
/// the hull so a 200 m ship is as editable as a 12 m launch.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HydroLimits {
    /// Draft, from 0.
    pub draft: f64,
    /// Trim, either way.
    pub trim: f64,
    /// KG above the keel, from 0.
    pub kg: f64,
}

impl Default for HydroLimits {
    /// The fixed limits the dock had before they scaled with the hull.
    fn default() -> Self {
        Self {
            draft: 20.0,
            trim: 5.0,
            kg: 10.0,
        }
    }
}

impl HydroLimits {
    /// The limits for a hull spanning `extents` (min, max): draft and KG up
    /// to twice its depth, trim to a fifth of its length, never below the
    /// fixed [`Default`] limits.
    pub fn for_extents(extents: Option<(Vec3, Vec3)>) -> Self {
        let base = Self::default();
        let Some((min, max)) = extents.filter(|(a, b)| (*b - *a).length().is_finite()) else {
            return base;
        };
        let depth = (max.z - min.z).max(0.0);
        let length = (max.x - min.x).max(0.0);
        Self {
            draft: base.draft.max(2.0 * depth),
            trim: base.trim.max(0.2 * length),
            kg: base.kg.max(2.0 * depth),
        }
    }
}

/// Everything the hydrostatics dock draws.
pub struct HydroPanel<'a> {
    pub options: &'a mut HydroOptions,
    /// KG above the keel; `None` until set, drawn as an empty field.
    pub kg: &'a mut Option<f64>,
    /// `kg` is the design requirements' estimate (marked "est.").
    pub kg_estimate: bool,
    /// How far the draft, trim and KG fields go.
    pub limits: HydroLimits,
    /// Solve the GZ curves free to trim about `lcg` instead of holding `trim`.
    pub free_trim: &'a mut bool,
    /// Longitudinal centre of gravity for the free-trim solve.
    pub lcg: &'a mut Option<f64>,
    pub result: Option<&'a Hydrostatics>,
    /// The report predates the last change of the model or the condition.
    pub stale: bool,
    /// A hydrostatics or GZ-curves job is running.
    pub busy: bool,
    pub curves: Option<&'a [CrossCurve]>,
    /// The GZ curves predate the last change (F5 recomputes them).
    pub curves_stale: bool,
}

/// The text and colour of a criteria badge: never colour alone. The colours
/// are the theme's semantic tokens, so a FAIL is the same red in every dock.
pub fn criteria_badge_style(passed: bool, stale: bool) -> (&'static str, egui::Color32) {
    match (passed, stale) {
        (true, false) => ("PASS", theme::OK),
        (false, false) => ("FAIL", theme::FAIL),
        (true, true) => ("PASS · stale", theme::STALE),
        (false, true) => ("FAIL · stale", theme::STALE),
    }
}

/// The line that goes with every criteria verdict: a pass is arithmetic against
/// the criteria, not approval.
pub fn not_validated_note(ui: &mut egui::Ui) {
    ui.label(
        egui::RichText::new(seacraft_core::NOT_VALIDATED_NOTE)
            .small()
            .color(theme::TEXT_SECONDARY),
    );
}

/// A criteria verdict: `PASS` green, `FAIL` red, `… · stale` grey when the
/// result predates the last edit. The only place a verdict is drawn.
pub fn criteria_badge(ui: &mut egui::Ui, passed: bool, stale: bool) -> egui::Response {
    let (text, colour) = criteria_badge_style(passed, stale);
    ui.label(egui::RichText::new(text).monospace().strong().color(colour))
}

/// The shared "out of date" chip: a result computed for inputs that have
/// changed since. F5 recomputes it.
pub fn stale_chip(ui: &mut egui::Ui) -> egui::Response {
    egui::Frame::new()
        .stroke(egui::Stroke::new(1.0, theme::STALE))
        .corner_radius(8.0)
        .inner_margin(egui::Margin::symmetric(6, 1))
        .show(ui, |ui| {
            ui.label(
                egui::RichText::new("out of date — F5")
                    .size(theme::SMALL_SIZE)
                    .color(theme::STALE),
            )
        })
        .response
}

/// Draw `add` with the stale look when `stale`: the chip first, then the
/// content dimmed to [`theme::STALE_OPACITY`]. One look for every result.
pub fn stale_section<R>(ui: &mut egui::Ui, stale: bool, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        if stale {
            stale_chip(ui);
            ui.multiply_opacity(theme::STALE_OPACITY);
        }
        add(ui)
    })
    .inner
}

/// The dot on a dock tab whose results are out of date, at the top right of
/// the tab's label.
pub fn stale_dot(ui: &egui::Ui, tab: egui::Rect) {
    ui.painter()
        .circle_filled(tab.right_top() + egui::vec2(-3.0, 3.0), 2.5, theme::WARN);
}

/// One numeric readout: a quantity's name, its value at a fixed precision
/// (signed quantities carry their sign) and its unit, kept apart so the
/// value column can align and the unit never glues onto the number.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Readout {
    pub name: &'static str,
    pub value: String,
    pub unit: &'static str,
}

impl Readout {
    pub fn new(name: &'static str, value: String, unit: &'static str) -> Self {
        Self { name, value, unit }
    }
}

/// Draw readouts as a three-column grid: name (body size, secondary colour),
/// value (monospace, right-aligned by padding to the widest value, so the
/// decimals of equal-precision rows line up) and unit (muted, fixed width).
pub fn readouts(ui: &mut egui::Ui, id: &str, rows: &[Readout]) {
    let width = rows
        .iter()
        .map(|r| r.value.chars().count())
        .max()
        .unwrap_or(0);
    egui::Grid::new(id)
        .num_columns(3)
        .spacing(egui::vec2(8.0, 4.0))
        .show(ui, |ui| readout_cells(ui, rows, width, true));
}

/// A titled group of readouts ("Volume & form", "Centres", …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadoutGroup {
    pub title: &'static str,
    pub rows: Vec<Readout>,
}

/// The measurement readouts the dock draws, in four groups: volume and
/// form, centres, stability, dimensions and areas.
///
/// Kept separate from the drawing so the numbers (and their formatting) can be
/// tested without a frame: `TPC` and `MTC` are table quantities computed here
/// from the measured waterplane, with the kilogram-to-tonne factor the core's
/// kg/m³ density needs.
pub fn hydrostatic_groups(h: &Hydrostatics, kg: Option<f64>) -> [ReadoutGroup; 4] {
    let rho_t = h.water_density / 1000.0;
    let tpc = h.waterplane_area_m2 * rho_t / 100.0;
    let mtc = if h.lwl > 1e-9 {
        rho_t * h.waterplane_inertia_longitudinal / (100.0 * h.lwl)
    } else {
        0.0
    };
    let r = Readout::new;
    [
        ReadoutGroup {
            title: "Volume & form",
            rows: vec![
                r("volume", format_form_value(h.volume_m3), "m³"),
                r("displacement", format_form_value(h.displacement_t), "t"),
                r("Cb", format!("{:.3}", h.block_coefficient), ""),
                r("Cp", format!("{:.3}", h.prismatic_coefficient), ""),
                r("Cm", format!("{:.3}", h.midship_coefficient), ""),
                r("Cwp", format!("{:.3}", h.waterplane_coefficient), ""),
            ],
        },
        ReadoutGroup {
            title: "Centres",
            rows: vec![
                r("LCB", format!("{:.3}", h.lcb.x), "m"),
                r("LCF", format!("{:.3}", h.lcf.x), "m"),
                r("KB", format!("{:.3}", h.kb), "m"),
            ],
        },
        ReadoutGroup {
            title: "Stability",
            rows: vec![
                r("BM(t)", format!("{:.3}", h.bm_transverse), "m"),
                r("KM(t)", format!("{:.3}", h.km_transverse), "m"),
                r("GM", gm_text(h, kg), "m"),
                r("TPC", format!("{:.3}", tpc), "t/cm"),
                r("MTC", format!("{:.3}", mtc), "t·m/cm"),
            ],
        },
        ReadoutGroup {
            title: "Dimensions & areas",
            rows: vec![
                r("LWL", format!("{:.2}", h.lwl), "m"),
                r("BWL", format!("{:.2}", h.bwl), "m"),
                r("wetted", format_form_value(h.wetted_surface_m2), "m²"),
                r("waterplane", format_form_value(h.waterplane_area_m2), "m²"),
                r("midship", format_form_value(h.mainframe_area_m2), "m²"),
            ],
        },
    ]
}

/// The readouts as the two columns of the wide layout: volume, form and
/// centres on the left, stability, dimensions and areas on the right.
pub fn hydrostatic_rows(h: &Hydrostatics, kg: Option<f64>) -> [Vec<Readout>; 2] {
    let [form, centres, stability, areas] = hydrostatic_groups(h, kg);
    [
        [form.rows, centres.rows].concat(),
        [stability.rows, areas.rows].concat(),
    ]
}

/// Draw readout groups as one grid, each group under a small sentence-case
/// subhead, so every value of the column shares one right edge.
pub fn grouped_readouts(ui: &mut egui::Ui, id: &str, groups: &[ReadoutGroup]) {
    grouped_grid(ui, id, groups, false);
}

/// [`grouped_readouts`], tight enough that two columns fit a 340 px dock:
/// narrower gaps and a unit column only as wide as its units.
fn grouped_grid(ui: &mut egui::Ui, id: &str, groups: &[ReadoutGroup], compact: bool) {
    let width = groups
        .iter()
        .flat_map(|g| &g.rows)
        .map(|r| r.value.chars().count())
        .max()
        .unwrap_or(0);
    let gap = if compact { 6.0 } else { 8.0 };
    egui::Grid::new(id)
        .num_columns(3)
        .min_col_width(0.0)
        .spacing(egui::vec2(gap, 4.0))
        .show(ui, |ui| {
            for g in groups {
                // Painted over a zero-width cell, so a long subhead never
                // widens the name column (the rest of its row is empty).
                let galley = ui.painter().layout_no_wrap(
                    g.title.to_owned(),
                    egui::FontId::proportional(theme::SMALL_SIZE),
                    theme::TEXT_MUTED,
                );
                let (rect, _) =
                    ui.allocate_exact_size(egui::vec2(0.0, galley.size().y), egui::Sense::hover());
                ui.painter()
                    .galley(rect.left_top(), galley, theme::TEXT_MUTED);
                ui.end_row();
                readout_cells(ui, &g.rows, width, !compact);
            }
        });
}

/// The three cells of each readout row: name, right-aligned value, unit
/// (in a column at least [`theme::UNIT_WIDTH`] wide when `unit_column`).
fn readout_cells(ui: &mut egui::Ui, rows: &[Readout], width: usize, unit_column: bool) {
    for r in rows {
        let name_label = ui.label(egui::RichText::new(r.name).color(theme::TEXT_SECONDARY));
        let val_label = ui.label(
            egui::RichText::new(format!("{:>width$}", r.value))
                .monospace()
                .color(theme::TEXT_PRIMARY),
        );
        if matches!(r.name, "Cb" | "Cp" | "Cm") {
            let note = "Calculated over total draft T including keel; canoe-body coefficients may be higher.";
            name_label.on_hover_text(note);
            val_label.on_hover_text(note);
        }
        if unit_column {
            ui.horizontal(|ui| {
                ui.set_min_width(theme::UNIT_WIDTH);
                ui.label(egui::RichText::new(r.unit).color(theme::TEXT_MUTED));
            });
        } else {
            ui.label(egui::RichText::new(r.unit).color(theme::TEXT_MUTED));
        }
        ui.end_row();
    }
}

/// The dock's inner width from which the hydrostatics readout splits into
/// two columns: what a 340 px dock leaves inside its margins and scroll bar.
pub const HYDRO_TWO_COLUMN_WIDTH: f32 = 316.0;

/// Whether the hydrostatics readout is laid out in two columns at this
/// available width.
pub fn hydro_two_columns(available_width: f32) -> bool {
    available_width >= HYDRO_TWO_COLUMN_WIDTH
}

/// The damaged-equilibrium readouts of the Damage tab. Trim, heel and GM
/// are signed and show their sign.
pub fn damage_rows(d: &DamageResult) -> Vec<Readout> {
    let r = Readout::new;
    vec![
        r("Δ (mass)", format!("{:.3}", d.displacement_t), "t"),
        r("Intact draft", format!("{:.3}", d.intact_draft), "m"),
        r("Damaged draft", format!("{:.3}", d.draft), "m"),
        r("Sinkage", format!("{:.3}", d.sinkage_m), "m"),
        r("Damaged heel", format_signed(d.heel_deg, 1), "°"),
        r("Net buoyant vol.", format!("{:.3}", d.net_volume_m3), "m³"),
        r("KG", format!("{:.3}", d.kg), "m"),
        r("Intact trim", format_signed(d.intact_trim_m, 3), "m"),
        r("Damaged trim", format_signed(d.trim_m, 3), "m"),
        match d.freeboard_m {
            Some(fb) => r("Min freeboard", format!("{fb:.3}"), "m"),
            None => r("Min freeboard", "-".to_string(), ""),
        },
        r("Lost buoyancy", format!("{:.3}", d.lost_volume_m3), "m³"),
        r("KB", format!("{:.3}", d.kb), "m"),
    ]
}

/// The equilibrium readouts of the Loading tab. TCG, trim, list and GM are
/// signed and show their sign.
pub fn loading_rows(l: &LoadingResult) -> Vec<Readout> {
    let r = Readout::new;
    vec![
        r("Δ (mass)", format!("{:.3}", l.displacement_t), "t"),
        r("LCG", format!("{:.3}", l.lcg), "m"),
        r("TCG", format_signed(l.tcg, 3), "m"),
        r("KG (solid)", format!("{:.3}", l.kg_solid), "m"),
        r("FSM", format!("{:.2}", l.fsm_t_m), "t·m"),
        r("FSC", format!("{:.3}", l.fsc_m), "m"),
        r("KG (fluid)", format!("{:.3}", l.kg_fluid), "m"),
        r("Draft", format!("{:.3}", l.draft), "m"),
        r("Trim", format_signed(l.trim_m, 3), "m"),
        r("List", format_signed(l.list_deg, 1), "°"),
        r("GM (solid)", format_signed(l.gm_solid, 3), "m"),
        r("GM (fluid)", format_signed(l.gm_fluid, 3), "m"),
    ]
}

/// The IMO severe wind and rolling readouts, each with its limit.
pub fn weather_rows(w: &WeatherResult) -> Vec<Readout> {
    vec![
        Readout::new(
            "Steady heel φ0",
            format!("{:.1}", w.phi0_deg.unwrap_or(0.0)),
            "° (lim 16°)",
        ),
        Readout::new(
            "Area ratio b/a",
            format!("{:.2}", w.area_ratio),
            "(req ≥1.0)",
        ),
    ]
}

/// The sectional area curve as a plot series: x along the hull, section
/// area up, one point per station with a finite area.
pub fn sac_series(sac: &[SectionArea]) -> Vec<(String, Vec<(f64, f64)>)> {
    vec![(
        String::new(),
        sac.iter()
            .filter(|s| s.x.is_finite() && s.area.is_finite())
            .map(|s| (s.x, s.area))
            .collect(),
    )]
}

/// Height of the SAC plot, caption band and tick labels included.
const SAC_PLOT_HEIGHT: f32 = 124.0;

/// The sectional area curve under the readout, on the same ticked axes as
/// every other plot.
fn plot_sac(ui: &mut egui::Ui, h: &Hydrostatics) {
    let a_max = h.sac.iter().fold(0.0_f64, |m, s| m.max(s.area));
    let caption = format!("SAC · A (m²) vs x (m) · A max {a_max:.2} m²");
    let rect = draw_plot(
        ui,
        &caption,
        "",
        &sac_series(&h.sac),
        false,
        SAC_PLOT_HEIGHT,
    );
    remember_plot_rect(ui.ctx(), "sac", rect);
}

/// Remember where plot `key` was drawn this frame (a test hook: see
/// [`last_plot_rect`]).
fn remember_plot_rect(ctx: &egui::Context, key: &'static str, rect: egui::Rect) {
    ctx.data_mut(|d| d.insert_temp(egui::Id::unique(("plot_rect", key)), rect));
}

/// Where plot `key` (`"sac"`, `"cross_curves"`) was last drawn, legend
/// included.
pub fn last_plot_rect(ctx: &egui::Context, key: &'static str) -> Option<egui::Rect> {
    ctx.data(|d| d.get_temp(egui::Id::unique(("plot_rect", key))))
}

/// What a KG-dependent control says while KG is unset.
pub const KG_UNSET: &str = "set KG first";

/// The dock's KG field. Set, a drag value up to `max` (marked "est." when it
/// is the requirements' estimate); unset, an empty text field with a hint,
/// so no number stands in for a KG nobody gave. Returns true when the user
/// set a value.
fn kg_field(ui: &mut egui::Ui, kg: &mut Option<f64>, estimate: bool, max: f64, width: f32) -> bool {
    match kg {
        Some(value) => ui
            .add_sized(
                [width, 20.0],
                egui::DragValue::new(value)
                    .speed(0.02)
                    .range(0.0..=max)
                    .fixed_decimals(2)
                    .suffix(if estimate { " m est." } else { " m" }),
            )
            .on_hover_text(if estimate {
                "KG estimate from the design requirements: set the real KG here"
            } else {
                "KG above the keel"
            })
            .changed(),
        None => {
            let id = ui.scope_id().with("kg_unset_text");
            let mut text: String = ui.data(|d| d.get_temp(id)).unwrap_or_default();
            let response = ui
                .add_sized(
                    [width, 20.0],
                    egui::TextEdit::singleline(&mut text).hint_text("— set KG"),
                )
                .on_hover_text("KG above the keel: GM and the GZ curves need it");
            // Enter or a click elsewhere commits (a single-line field loses
            // focus on Enter).
            let entered = response.lost_focus();
            let parsed = text
                .trim()
                .trim_end_matches('m')
                .trim()
                .parse::<f64>()
                .ok()
                .filter(|v| v.is_finite() && *v >= 0.0);
            match parsed {
                Some(v) if entered => {
                    ui.data_mut(|d| d.remove::<String>(id));
                    *kg = Some(v.min(max));
                    true
                }
                _ => {
                    ui.data_mut(|d| d.insert_temp(id, text));
                    false
                }
            }
        }
    }
}

/// Draft, trim, heel and KG controls plus the measurement readout.
pub fn hydrostatics(ui: &mut egui::Ui, panel: HydroPanel<'_>) -> HydroAction {
    let mut action = HydroAction::None;
    ui.heading("Hydrostatics");
    ui.add_space(4.0);

    // From a 340 px dock the readout runs in two columns and the floating
    // condition in two pairs per row, so the plots stay above the fold.
    let two_columns = hydro_two_columns(ui.available_width());
    let field_width = if two_columns { 84.0 } else { 96.0 };
    let mut changed = false;
    let mut kg_set = false;
    egui::Grid::new("floating_condition")
        .num_columns(if two_columns { 4 } else { 2 })
        .min_col_width(if two_columns { 36.0 } else { 56.0 })
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            ui.label("Draft");
            changed |= ui
                .add_sized(
                    [field_width, 20.0],
                    egui::DragValue::new(&mut panel.options.draft)
                        .speed(0.01)
                        .range(0.0..=panel.limits.draft)
                        .fixed_decimals(3)
                        .suffix(" m"),
                )
                .changed();
            if !two_columns {
                ui.end_row();
            }
            ui.label("Trim");
            changed |= ui
                .add_sized(
                    [field_width, 20.0],
                    egui::DragValue::new(&mut panel.options.trim)
                        .speed(0.01)
                        .range(-panel.limits.trim..=panel.limits.trim)
                        .fixed_decimals(3)
                        .suffix(" m"),
                )
                .changed();
            ui.end_row();
            ui.label("Heel");
            changed |= ui
                .add_sized(
                    [field_width, 20.0],
                    egui::DragValue::new(&mut panel.options.heel_deg)
                        .speed(0.2)
                        .range(-60.0..=60.0)
                        .fixed_decimals(1)
                        .suffix(" °"),
                )
                .changed();
            if !two_columns {
                ui.end_row();
            }
            ui.label("KG");
            kg_set |= kg_field(
                ui,
                panel.kg,
                panel.kg_estimate,
                panel.limits.kg,
                field_width,
            );
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
        action = HydroAction::Changed;
    }
    if kg_set {
        action = HydroAction::KgSet;
    }
    if panel.busy {
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
        let gz = ui.button("GZ curves");
        if gz.clicked() {
            action = HydroAction::CrossCurves;
        }
        if panel.kg.is_none() {
            ui.label(egui::RichText::new(KG_UNSET).color(theme::TEXT_MUTED));
        }
    });
    ui.add_space(2.0);
    ui.separator();
    ui.add_space(2.0);

    let kg = *panel.kg;
    stale_section(ui, panel.stale && panel.result.is_some(), |ui| {
        match panel.result {
            Some(h) if h.volume_m3 > 0.0 => {
                let [form, centres, stability, areas] = hydrostatic_groups(h, kg);
                if two_columns {
                    // Side by side, so SAC and the cross curves stay above the
                    // fold at 1600 × 1000.
                    ui.horizontal_top(|ui| {
                        ui.spacing_mut().item_spacing.x = 13.0;
                        grouped_grid(ui, "hydro_readouts", &[form, centres], true);
                        grouped_grid(ui, "hydro_readouts_2", &[stability, areas], true);
                    });
                } else {
                    grouped_readouts(ui, "hydro_readouts", &[form, centres, stability, areas]);
                }
                ui.add_space(2.0);
                plot_sac(ui, h);
                for warning in &h.warnings {
                    ui.colored_label(theme::FAIL, &warning.message);
                }
                if kg.is_some_and(|kg| h.gm(kg) < 0.0) {
                    ui.colored_label(theme::FAIL, "negative GM: the hull is unstable upright");
                }
            }
            Some(_) => {
                ui.colored_label(theme::WARN, "nothing is submerged at this draft");
            }
            None => {
                ui.small("no measurement yet");
            }
        }
    });

    if let (Some(curves), Some(kg)) = (panel.curves, kg) {
        // No criteria verdict here: these curves are the bare hull at the
        // dock KG over a synthetic ±30 % displacement range, not a condition
        // anyone defined. The per-criterion table lives in Loading.
        ui.separator();
        stale_section(ui, panel.curves_stale, |ui| {
            plot_cross_curves(ui, curves, kg);
        });
    }
    action
}

/// Draw the report options form. Returns true when any setting changed.
pub fn report_options(
    ui: &mut egui::Ui,
    report: &mut ReportSettings,
    loading_conditions: &[String],
    damage_cases: &[String],
) -> bool {
    let mut changed = false;

    ui.heading("General hydrostatics");
    egui::Grid::new("report_hydro_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            // Draft override
            ui.label("Draft override:");
            let mut has_draft = report.draft.is_some();
            if ui.checkbox(&mut has_draft, "Set").changed() {
                report.draft = if has_draft { Some(1.0) } else { None };
                changed = true;
            }
            if let Some(d) = &mut report.draft {
                changed |= ui
                    .add(
                        egui::DragValue::new(d)
                            .speed(0.01)
                            .range(0.01..=50.0)
                            .suffix(" m"),
                    )
                    .changed();
            }
            ui.end_row();

            // KG override
            ui.label("KG override:");
            let mut has_kg = report.kg.is_some();
            if ui.checkbox(&mut has_kg, "Set").changed() {
                report.kg = if has_kg { Some(1.0) } else { None };
                changed = true;
            }
            if let Some(kg) = &mut report.kg {
                changed |= ui
                    .add(
                        egui::DragValue::new(kg)
                            .speed(0.01)
                            .range(0.0..=50.0)
                            .suffix(" m"),
                    )
                    .changed();
            }
            ui.end_row();

            // LCG override (optional)
            ui.label("LCG (optional):");
            let mut has_lcg = report.lcg_x.is_some();
            if ui.checkbox(&mut has_lcg, "Set").changed() {
                report.lcg_x = if has_lcg { Some(0.0) } else { None };
                changed = true;
            }
            if let Some(lcg) = &mut report.lcg_x {
                changed |= ui
                    .add(
                        egui::DragValue::new(lcg)
                            .speed(0.01)
                            .range(-100.0..=100.0)
                            .suffix(" m"),
                    )
                    .changed();
            }
            ui.end_row();

            // Speeds (comma list)
            ui.label("Speeds (knots):");
            // While the field has focus its text lives in egui's memory:
            // re-formatting the parsed list every frame would eat a comma
            // or a trailing "." the moment it is typed.
            let buffer_id = ui.make_persistent_id("report_speeds_text");
            let mut speeds_text = ui
                .data(|d| d.get_temp::<(Option<Vec<f64>>, String)>(buffer_id))
                .filter(|(saved, _)| *saved == report.speeds_kn)
                .map(|(_, text)| text)
                .unwrap_or_else(|| {
                    report
                        .speeds_kn
                        .as_ref()
                        .map(|v| {
                            v.iter()
                                .map(|s| s.to_string())
                                .collect::<Vec<_>>()
                                .join(", ")
                        })
                        .unwrap_or_default()
                });
            let response = ui.add(
                egui::TextEdit::singleline(&mut speeds_text)
                    .hint_text("Default (e.g. 6, 8, 10, 12, 14)"),
            );
            if response.changed() {
                let parsed: Vec<f64> = speeds_text
                    .split(',')
                    .filter_map(|s| s.trim().parse::<f64>().ok())
                    .filter(|x| x.is_finite() && *x > 0.0)
                    .collect();
                report.speeds_kn = if parsed.is_empty() {
                    None
                } else {
                    Some(parsed)
                };
                changed = true;
            }
            if response.has_focus() {
                ui.data_mut(|d| {
                    d.insert_temp(buffer_id, (report.speeds_kn.clone(), speeds_text));
                });
            } else {
                ui.data_mut(|d| d.remove::<(Option<Vec<f64>>, String)>(buffer_id));
            }
            ui.end_row();
        });

    ui.separator();
    ui.heading("Resistance & powering");
    let mut inc_res = report.include_resistance.unwrap_or(true);
    if ui
        .checkbox(&mut inc_res, "Holtrop & Mennen resistance")
        .changed()
    {
        report.include_resistance = Some(inc_res);
        changed = true;
    }
    let mut inc_plan = report.include_planing.unwrap_or(false);
    if ui
        .checkbox(&mut inc_plan, "Savitsky planing analysis")
        .changed()
    {
        report.include_planing = Some(inc_plan);
        changed = true;
    }

    ui.separator();
    ui.heading("Loading conditions");
    let mut inc_w = report.include_weather.unwrap_or(true);
    if ui
        .checkbox(&mut inc_w, "Include IMO weather criterion")
        .changed()
    {
        report.include_weather = Some(inc_w);
        changed = true;
    }
    if loading_conditions.is_empty() {
        ui.label("No loading conditions in project.");
    } else {
        ui.label("Included conditions:");
        let mut cur_selected = match &report.loading_conditions {
            None => loading_conditions.to_vec(),
            Some(names) => names.clone(),
        };
        let mut lc_changed = false;
        for name in loading_conditions {
            let mut checked = cur_selected.contains(name);
            if ui.checkbox(&mut checked, name).changed() {
                lc_changed = true;
                if checked {
                    if !cur_selected.contains(name) {
                        cur_selected.push(name.clone());
                    }
                } else {
                    cur_selected.retain(|n| n != name);
                }
            }
        }
        if lc_changed {
            if cur_selected.len() == loading_conditions.len() {
                report.loading_conditions = None; // all = None
            } else {
                report.loading_conditions = Some(cur_selected);
            }
            changed = true;
        }
    }

    ui.separator();
    ui.heading("Damage cases");
    if damage_cases.is_empty() {
        ui.label("No damage cases in project.");
    } else {
        ui.label("Included damage cases:");
        let mut cur_selected = match &report.damage_cases {
            None => damage_cases.to_vec(),
            Some(names) => names.clone(),
        };
        let mut dc_changed = false;
        for name in damage_cases {
            let mut checked = cur_selected.contains(name);
            if ui.checkbox(&mut checked, name).changed() {
                dc_changed = true;
                if checked {
                    if !cur_selected.contains(name) {
                        cur_selected.push(name.clone());
                    }
                } else {
                    cur_selected.retain(|n| n != name);
                }
            }
        }
        if dc_changed {
            if cur_selected.len() == damage_cases.len() {
                report.damage_cases = None; // all = None
            } else {
                report.damage_cases = Some(cur_selected);
            }
            changed = true;
        }
    }

    ui.separator();
    ui.heading("Seakeeping");
    let mut has_seakeeping = report.seakeeping.is_some();
    if ui
        .checkbox(&mut has_seakeeping, "Include seakeeping analysis")
        .changed()
    {
        if has_seakeeping {
            report.seakeeping = Some(ReportSeakeeping {
                speed_kn: 0.0,
                kg: None,
                sea: None,
            });
        } else {
            report.seakeeping = None;
        }
        changed = true;
    }
    if let Some(sk) = &mut report.seakeeping {
        egui::Grid::new("report_sk_grid")
            .num_columns(2)
            .spacing([12.0, 4.0])
            .show(ui, |ui| {
                ui.label("Speed:");
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut sk.speed_kn)
                            .speed(0.5)
                            .range(0.0..=100.0)
                            .suffix(" kn"),
                    )
                    .changed();
                ui.end_row();

                ui.label("KG override (optional):");
                let mut has_kg = sk.kg.is_some();
                if ui.checkbox(&mut has_kg, "Set").changed() {
                    sk.kg = if has_kg { Some(1.0) } else { None };
                    changed = true;
                }
                if let Some(k) = &mut sk.kg {
                    changed |= ui
                        .add(
                            egui::DragValue::new(k)
                                .speed(0.01)
                                .range(0.0..=50.0)
                                .suffix(" m"),
                        )
                        .changed();
                }
                ui.end_row();
            });

        let mut has_sea = sk.sea.is_some();
        if ui.checkbox(&mut has_sea, "Irregular sea state").changed() {
            if has_sea {
                sk.sea = Some(WaveSpectrum::Bretschneider { hs: 2.0, tp: 7.0 });
            } else {
                sk.sea = None;
            }
            changed = true;
        }
        if let Some(sea) = &mut sk.sea {
            let is_jonswap = matches!(sea, WaveSpectrum::Jonswap { .. });
            ui.horizontal(|ui| {
                if ui.radio(!is_jonswap, "Bretschneider").clicked() && is_jonswap {
                    *sea = WaveSpectrum::Bretschneider {
                        hs: sea.hs(),
                        tp: sea.tp(),
                    };
                    changed = true;
                }
                if ui.radio(is_jonswap, "JONSWAP").clicked() && !is_jonswap {
                    *sea = WaveSpectrum::Jonswap {
                        hs: sea.hs(),
                        tp: sea.tp(),
                        gamma: 3.3,
                    };
                    changed = true;
                }
            });

            egui::Grid::new("report_sea_grid")
                .num_columns(2)
                .spacing([12.0, 4.0])
                .show(ui, |ui| {
                    ui.label("Significant wave height Hs:");
                    let mut hs = sea.hs();
                    if ui
                        .add(
                            egui::DragValue::new(&mut hs)
                                .speed(0.1)
                                .range(0.1..=30.0)
                                .suffix(" m"),
                        )
                        .changed()
                    {
                        match sea {
                            WaveSpectrum::Bretschneider { hs: h, .. } => *h = hs,
                            WaveSpectrum::Jonswap { hs: h, .. } => *h = hs,
                        }
                        changed = true;
                    }
                    ui.end_row();

                    ui.label("Peak period Tp:");
                    let mut tp = sea.tp();
                    if ui
                        .add(
                            egui::DragValue::new(&mut tp)
                                .speed(0.1)
                                .range(0.5..=30.0)
                                .suffix(" s"),
                        )
                        .changed()
                    {
                        match sea {
                            WaveSpectrum::Bretschneider { tp: t, .. } => *t = tp,
                            WaveSpectrum::Jonswap { tp: t, .. } => *t = tp,
                        }
                        changed = true;
                    }
                    ui.end_row();

                    if let WaveSpectrum::Jonswap { gamma, .. } = sea {
                        ui.label("Peak enhancement γ:");
                        changed |= ui
                            .add(egui::DragValue::new(gamma).speed(0.1).range(1.0..=20.0))
                            .changed();
                        ui.end_row();
                    }
                });
        }
    }

    changed
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PerformanceAction {
    None,
    Changed,
    Compute,
}

pub struct PerformancePanel<'a> {
    pub state: &'a mut crate::app::PerformanceState,
    /// The KG without a manual value; `None` while no KG is known.
    pub default_kg: Option<f64>,
    pub kg_source: &'static str,
    pub busy_resistance: bool,
    pub busy_raos: bool,
    /// The resistance curve predates the last change (recomputed
    /// automatically).
    pub stale_resistance: bool,
    /// The RAOs and sea-state responses predate the last change (F5).
    pub stale_raos: bool,
}

pub fn performance(ui: &mut egui::Ui, panel: PerformancePanel<'_>) -> PerformanceAction {
    let mut action = PerformanceAction::None;
    ui.heading("Performance");
    ui.add_space(4.0);

    let is_busy = panel.busy_resistance || panel.busy_raos;

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
        }
    });

    ui.add_space(4.0);
    ui.separator();
    ui.strong("Resistance prediction");
    ui.add_space(2.0);

    ui.horizontal(|ui| {
        ui.label("Method:");
        egui::ComboBox::from_id_salt("perf_method")
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
        stale_section(ui, panel.stale_resistance, |ui| resistance_results(ui, res));
    }

    ui.add_space(4.0);
    ui.separator();
    ui.strong("Seakeeping (head-sea RAOs)");
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
        ui.label("KG");
        let mut cur_kg = panel.state.kg.or(panel.default_kg);
        if kg_field(ui, &mut cur_kg, false, 50.0, 84.0) {
            panel.state.kg = cur_kg;
            action = PerformanceAction::Changed;
        }
        let default = match panel.default_kg {
            Some(kg) => format!("{kg:.2} m"),
            None => KG_UNSET.to_owned(),
        };
        if panel.state.kg.is_some() {
            if ui.small_button("Reset").clicked() {
                panel.state.kg = None;
                action = PerformanceAction::Changed;
            }
            ui.label(format!("(default: {default}, {})", panel.kg_source));
        } else if panel.default_kg.is_some() {
            ui.label(format!("({})", panel.kg_source));
        } else {
            ui.label(egui::RichText::new(format!("({default})")).color(theme::TEXT_MUTED));
        }
    });

    ui.horizontal(|ui| {
        let kyy_resp = ui.add(
            egui::DragValue::new(&mut panel.state.kyy_frac)
                .speed(0.01)
                .range(0.05..=1.0)
                .prefix("kyy / Lwl: "),
        );
        let pts_resp = ui
            .add(
                egui::DragValue::new(&mut panel.state.points)
                    .range(5..=200)
                    .prefix("Frequencies: "),
            )
            .on_hover_text("Number of encounter wave frequencies computed for the RAO");
        if kyy_resp.changed() || pts_resp.changed() {
            action = PerformanceAction::Changed;
        }
    });

    if let Some(rep) = &panel.state.raos {
        stale_section(ui, panel.stale_raos, |ui| rao_results(ui, rep));
    } else if !panel.busy_raos {
        ui.label(
            egui::RichText::new("Click Compute above for the heave and pitch RAOs.")
                .color(theme::TEXT_MUTED),
        );
    }

    ui.add_space(4.0);
    ui.separator();
    if sea_state_section(ui, panel.state, panel.stale_raos) {
        action = PerformanceAction::Changed;
    }

    action
}

/// The resistance curve and its table.
fn resistance_results(ui: &mut egui::Ui, res: &crate::app::ResistanceResult) {
    for w in &res.warnings {
        ui.colored_label(theme::WARN, format!("⚠ {w}"));
    }

    let series = vec![(
        res.method.clone(),
        res.rows.iter().map(|r| (r.speed_kn, r.rt_kn)).collect(),
    )];
    plot_xy(ui, "Resistance", "Speed (kn)", &res.rt_label, &series);

    ui.add_space(2.0);
    let (rt, rt_unit) = split_unit(&res.rt_label);
    eng_table(
        ui,
        &[
            EngColumn::number("V", "kn"),
            EngColumn::number("Fn", ""),
            EngColumn::number(rt, rt_unit),
            EngColumn::number("P_E", "kW"),
        ],
        &resistance_table(res),
    );
}

/// The natural periods and the motion RAO plot.
fn rao_results(ui: &mut egui::Ui, rep: &seacraft_core::seakeeping::SeakeepingReport) {
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
        ui.colored_label(theme::WARN, format!("⚠ {}", w.message));
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
    let in_plot =
        |p: &&seacraft_core::seakeeping::RaoPoint| shown < 3 || p.lambda_over_l <= PLOT_LAMBDA_MAX;
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

/// The Performance tab's "Sea state" section: spectrum input, response points and
/// (after Compute) the significant-response table with MSI. Returns whether an input
/// changed (the results are then out of date).
fn sea_state_section(
    ui: &mut egui::Ui,
    state: &mut crate::app::PerformanceState,
    stale: bool,
) -> bool {
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
                if ui.small_button("✕").on_hover_text("Remove point").clicked() {
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
            ui.colored_label(theme::FAIL, format!("⚠ {problem}"));
        }
    }

    if let Some(r) = &state.sea_result {
        ui.add_space(4.0);
        stale_section(ui, stale, |ui| sea_state_results(ui, r));
    }
    changed
}

/// The irregular-sea responses: spectrum, significant motions, MSI.
fn sea_state_results(ui: &mut egui::Ui, r: &seacraft_core::seastate::IrregularReport) {
    {
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
            ui.colored_label(theme::WARN, format!("⚠ {}", w.message));
        }
        let global: Vec<Vec<EngCell>> = [("Heave", "m", &r.heave), ("Pitch", "deg", &r.pitch_deg)]
            .into_iter()
            .map(|(name, unit, st)| {
                vec![
                    name.into(),
                    unit.into(),
                    format!("{:.3}", st.significant).into(),
                    format!("{:.3}", st.rms).into(),
                    format!("{:.2}", st.tz_s).into(),
                ]
            })
            .collect();
        eng_table(
            ui,
            &[
                EngColumn::text("Response"),
                EngColumn::unit(),
                EngColumn::number("Significant", ""),
                EngColumn::number("RMS", ""),
                EngColumn::number("Tz", "s"),
            ],
            &global,
        );
        ui.add_space(2.0);
        let points: Vec<Vec<EngCell>> = r
            .points
            .iter()
            .map(|p| {
                vec![
                    p.name.as_str().into(),
                    format!("{:.2}", p.x).into(),
                    format!("{:.3}", p.vertical_motion.significant).into(),
                    format!("{:.3}", p.relative_motion.significant).into(),
                    format!("{:.4}", p.acceleration_rms_g).into(),
                    format!("{:.1}", p.msi_percent).into(),
                ]
            })
            .collect();
        eng_table(
            ui,
            &[
                EngColumn::text("Point"),
                EngColumn::number("x", "m"),
                EngColumn::number("Motion sig.", "m"),
                EngColumn::number("Relative sig.", "m"),
                EngColumn::number("Accel. RMS", "g"),
                EngColumn::number("MSI", "%"),
            ],
            &points,
        );
        ui.label(
            "MSI: share of unadapted people expected to vomit within a 2-hour exposure \
             (O'Hanlon & McCauley 1974).",
        );
    }
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
    ui.heading("Damage stability");
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

    if panel.project.compartments.is_empty() {
        ui.label(
            egui::RichText::new(
                "No compartments defined. Click '+ Add compartment' to create watertight divisions.",
            )
            .color(theme::TEXT_MUTED),
        );
    }

    let mut remove_comp = None;
    let mut renamed: Vec<(String, String)> = Vec::new();
    for (c_idx, comp) in panel.project.compartments.iter_mut().enumerate() {
        let is_highlighted = panel.highlight == Some(c_idx);
        let mut frame = egui::Frame::group(ui.style());
        if is_highlighted {
            frame = frame.stroke(egui::Stroke::new(2.0, theme::HIGHLIGHT_COMPARTMENT));
        }
        let frame_resp = frame.show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label("Name:");
                let old_name = comp.name.clone();
                if ui.text_edit_singleline(&mut comp.name).changed() {
                    action = DamageAction::Changed;
                    renamed.push((old_name, comp.name.clone()));
                }
                if ui
                    .checkbox(&mut comp.clip_to_hull, "Clip to hull")
                    .changed()
                {
                    action = DamageAction::Changed;
                }
                if ui.button("✕").on_hover_text("Remove compartment").clicked() {
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
    // Damage cases name their compartments: follow a rename.
    for (old, new) in renamed {
        for case in &mut panel.project.damage_cases {
            for spec in &mut case.compartments {
                if matches!(spec, CompartmentSpec::Named(n) if *n == old) {
                    *spec = CompartmentSpec::Named(new.clone());
                }
            }
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
    ui.heading("Damage cases");
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
        ui.label(
            egui::RichText::new(
                "Click 'New' to create a damage case and evaluate damaged equilibrium.",
            )
            .color(theme::TEXT_MUTED),
        );
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
            theme::FAIL,
            "Another damage case has this name; rename it before saving.",
        );
    }
    let case = &mut panel.project.damage_cases[*panel.selected];

    // Flooded compartments checkboxes
    ui.add_space(4.0);
    ui.label(egui::RichText::new("Flooded compartments").strong());
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
        ui.label(egui::RichText::new("Inline compartments (read-only)").italics());
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
    ui.label(egui::RichText::new("Intact condition").strong());

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
        }
    });

    // Results
    if let Some(r) = panel.result {
        ui.separator();
        if panel.stale {
            stale_chip(ui);
            // The results close the panel: the dimming ends with it.
            ui.multiply_opacity(theme::STALE_OPACITY);
        }
        ui.heading("Damaged equilibrium");
        if !r.equilibrium_found {
            ui.colored_label(
                theme::FAIL,
                "Equilibrium NOT found (lost buoyancy exceeds reserve buoyancy or capsized)",
            );
        } else {
            ui.colored_label(theme::OK, "Equilibrium found");
        }

        readouts(ui, "damage_readouts", &damage_rows(r));

        if !r.compartments.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Flooded compartment summary").strong());
            eng_table(
                ui,
                &[
                    EngColumn::text("Name"),
                    EngColumn::number("μ", ""),
                    EngColumn::number("Submerged", "m³"),
                    EngColumn::number("Lost", "m³"),
                    EngColumn::number("x", "m"),
                    EngColumn::number("y", "m"),
                    EngColumn::number("z", "m"),
                ],
                &compartment_table(r),
            );
        }

        if let Some(crit) = &r.criteria {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Residual stability criteria").strong());
                criteria_badge(ui, crit.passed, panel.stale);
            });
            not_validated_note(ui);
            let stale = panel.stale;
            let rows = [
                criterion_row(
                    "Positive range ≥ 15°",
                    format!("{:.1}", crit.positive_range_deg),
                    "°",
                    crit.positive_range_deg >= 15.0,
                    stale,
                ),
                criterion_row(
                    "Max GZ ≥ 0.10 m",
                    format!("{:.3}", crit.max_gz_m),
                    "m",
                    crit.max_gz_m >= 0.10,
                    stale,
                ),
                criterion_row(
                    "Area ≥ 0.015 m·rad",
                    format!("{:.4}", crit.area_m_rad),
                    "m·rad",
                    crit.area_m_rad >= 0.015,
                    stale,
                ),
            ];
            eng_table(ui, &criteria_columns(), &rows);
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
            ui.label(egui::RichText::new("Residual righting lever GZ").strong());
            plot_gz(ui, &[(r.case_name.clone(), pts)]);
        }

        if !r.warnings.is_empty() {
            ui.add_space(4.0);
            for w in &r.warnings {
                ui.colored_label(theme::WARN, format!("Warning: {w}"));
            }
        }
    }

    action
}

// ---- engineering tables --------------------------------------------------

/// How a column of an [`eng_table`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ColumnKind {
    /// Names: proportional, left-aligned.
    Text,
    /// Numbers: monospace, right-aligned, so equal precisions align on the
    /// decimal point.
    Number,
    /// A unit per row (when the rows differ): muted, left-aligned.
    Unit,
    /// A criteria verdict ([`criteria_badge_style`]).
    Badge,
}

/// One column of an [`eng_table`]: its title, the unit every value in it
/// carries (drawn muted in the header, never glued onto a number) and how
/// it reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct EngColumn<'a> {
    pub title: &'a str,
    pub unit: &'a str,
    pub kind: ColumnKind,
}

impl<'a> EngColumn<'a> {
    pub fn text(title: &'a str) -> Self {
        Self {
            title,
            unit: "",
            kind: ColumnKind::Text,
        }
    }
    pub fn number(title: &'a str, unit: &'a str) -> Self {
        Self {
            title,
            unit,
            kind: ColumnKind::Number,
        }
    }
    pub fn unit() -> Self {
        Self {
            title: "",
            unit: "",
            kind: ColumnKind::Unit,
        }
    }
    pub fn badge(title: &'a str) -> Self {
        Self {
            title,
            unit: "",
            kind: ColumnKind::Badge,
        }
    }
}

/// One cell of an [`eng_table`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EngCell {
    /// Text, or a number already formatted at its column's precision.
    Text(String),
    /// A criteria verdict: passed, stale.
    Badge(bool, bool),
}

impl From<String> for EngCell {
    fn from(s: String) -> Self {
        EngCell::Text(s)
    }
}

impl From<&str> for EngCell {
    fn from(s: &str) -> Self {
        EngCell::Text(s.to_owned())
    }
}

/// Space between columns and inside each row.
const TABLE_GAP_X: f32 = 12.0;
const TABLE_PAD_Y: f32 = 2.0;
/// A header wraps onto a second line rather than widen its column past this
/// (or the column's widest value, if that is wider).
const TABLE_HEADER_WRAP: f32 = 64.0;

/// What [`eng_table`] drew: the whole table and each cell, header row
/// first (test hooks).
#[derive(Debug, Clone, PartialEq)]
pub struct EngTableOutput {
    pub rect: egui::Rect,
    pub cells: Vec<Vec<egui::Rect>>,
}

/// The one engineering table: a header row with the units, names on the
/// left, values right-aligned in monospace at the precision they were
/// formatted with, rows striped from the theme. Read-only: nothing in it is
/// clickable, so it is painted rather than laid out widget by widget.
pub fn eng_table(
    ui: &mut egui::Ui,
    columns: &[EngColumn<'_>],
    rows: &[Vec<EngCell>],
) -> EngTableOutput {
    let body_font = egui::FontId::proportional(theme::BODY_SIZE);
    let mono_font = egui::FontId::monospace(theme::MONO_SIZE);
    let unit_font = egui::FontId::proportional(theme::SMALL_SIZE);
    let painter = ui.painter().clone();
    let cell_galley = |kind: ColumnKind, cell: Option<&EngCell>| {
        let (text, font, colour) = match (kind, cell) {
            (_, None) => (String::new(), body_font.clone(), theme::TEXT_PRIMARY),
            (_, Some(EngCell::Badge(passed, stale))) => {
                let (text, colour) = criteria_badge_style(*passed, *stale);
                (text.to_owned(), mono_font.clone(), colour)
            }
            (ColumnKind::Number, Some(EngCell::Text(t))) => {
                (t.clone(), mono_font.clone(), theme::TEXT_PRIMARY)
            }
            (ColumnKind::Unit, Some(EngCell::Text(t))) => {
                (t.clone(), body_font.clone(), theme::TEXT_MUTED)
            }
            (_, Some(EngCell::Text(t))) => (t.clone(), body_font.clone(), theme::TEXT_PRIMARY),
        };
        painter.layout_no_wrap(text, font, colour)
    };
    let body: Vec<Vec<std::sync::Arc<egui::Galley>>> = rows
        .iter()
        .map(|row| {
            columns
                .iter()
                .enumerate()
                .map(|(c, col)| cell_galley(col.kind, row.get(c)))
                .collect()
        })
        .collect();
    let value_widths: Vec<f32> = (0..columns.len())
        .map(|c| {
            body.iter()
                .map(|row| row[c].size().x)
                .fold(0.0_f32, f32::max)
        })
        .collect();
    // Header: title in the secondary colour, unit muted beside it.
    let headers: Vec<std::sync::Arc<egui::Galley>> = columns
        .iter()
        .enumerate()
        .map(|(c, col)| {
            let mut job = egui::text::LayoutJob::default();
            job.append(
                col.title,
                0.0,
                egui::TextFormat::simple(body_font.clone(), theme::TEXT_SECONDARY),
            );
            if !col.unit.is_empty() {
                job.append(
                    &format!("({})", col.unit),
                    if col.title.is_empty() { 0.0 } else { 4.0 },
                    egui::TextFormat::simple(unit_font.clone(), theme::TEXT_MUTED),
                );
            }
            job.wrap.max_width = value_widths[c].max(TABLE_HEADER_WRAP);
            painter.layout_job(job)
        })
        .collect();
    let widths: Vec<f32> = (0..columns.len())
        .map(|c| value_widths[c].max(headers[c].size().x).ceil())
        .collect();
    let header_h = headers.iter().map(|g| g.size().y).fold(0.0_f32, f32::max) + 2.0 * TABLE_PAD_Y;
    let row_h = body
        .iter()
        .flatten()
        .map(|g| g.size().y)
        .fold(theme::MONO_SIZE, f32::max)
        + 2.0 * TABLE_PAD_Y;
    let table_w = widths.iter().sum::<f32>()
        + TABLE_GAP_X * (columns.len().saturating_sub(1) as f32)
        + 2.0 * TABLE_PAD_Y;
    let table_h = header_h + row_h * rows.len() as f32;
    let (rect, _) = ui.allocate_exact_size(egui::vec2(table_w, table_h), egui::Sense::hover());

    // Column x spans.
    let mut spans = Vec::with_capacity(columns.len());
    let mut x = rect.left() + TABLE_PAD_Y;
    for w in &widths {
        spans.push((x, x + w));
        x += w + TABLE_GAP_X;
    }
    let place = |kind: ColumnKind, (x0, x1): (f32, f32), size: egui::Vec2, y: f32| {
        let left = match kind {
            ColumnKind::Number => x1 - size.x,
            _ => x0,
        };
        egui::Rect::from_min_size(egui::pos2(left, y), size)
    };
    let mut cells = Vec::with_capacity(rows.len() + 1);
    let mut header_cells = Vec::with_capacity(columns.len());
    for (c, g) in headers.into_iter().enumerate() {
        let r = place(
            columns[c].kind,
            spans[c],
            g.size(),
            rect.top() + TABLE_PAD_Y,
        );
        header_cells.push(r);
        painter.galley(r.min, g, theme::TEXT_SECONDARY);
    }
    cells.push(header_cells);
    let rule_y = rect.top() + header_h - 0.5;
    painter.line_segment(
        [
            egui::pos2(rect.left(), rule_y),
            egui::pos2(rect.right(), rule_y),
        ],
        egui::Stroke::new(1.0, theme::FRAME),
    );
    for (i, row) in body.into_iter().enumerate() {
        let top = rect.top() + header_h + row_h * i as f32;
        if i % 2 == 1 {
            painter.rect_filled(
                egui::Rect::from_min_size(egui::pos2(rect.left(), top), egui::vec2(table_w, row_h)),
                0.0,
                theme::TABLE_STRIPE,
            );
        }
        let mut row_cells = Vec::with_capacity(columns.len());
        for (c, g) in row.into_iter().enumerate() {
            let y = top + 0.5 * (row_h - g.size().y);
            let r = place(columns[c].kind, spans[c], g.size(), y);
            row_cells.push(r);
            painter.galley(r.min, g, theme::TEXT_PRIMARY);
        }
        cells.push(row_cells);
    }
    EngTableOutput { rect, cells }
}

/// A label such as `"R_T (kN)"` split into its title and unit.
pub fn split_unit(label: &str) -> (&str, &str) {
    match label.rsplit_once(" (") {
        Some((title, rest)) if rest.ends_with(')') => (title, &rest[..rest.len() - 1]),
        _ => (label, ""),
    }
}

/// The resistance table: speed, Froude number, total resistance and
/// effective power, each at the precision the dock always printed.
pub fn resistance_table(res: &crate::app::ResistanceResult) -> Vec<Vec<EngCell>> {
    res.rows
        .iter()
        .map(|r| {
            vec![
                format!("{:.2}", r.speed_kn).into(),
                format!("{:.3}", r.fn_).into(),
                format!("{:.2}", r.rt_kn).into(),
                r.pe_kw
                    .map_or_else(|| "—".to_owned(), |pe| format!("{pe:.2}"))
                    .into(),
            ]
        })
        .collect()
}

/// The Tank summary rows of the Loading tab: name, fill in percent, mass
/// and free-surface moment.
pub fn tank_table(l: &LoadingResult) -> Vec<Vec<EngCell>> {
    l.tanks
        .iter()
        .map(|t| {
            vec![
                t.name.as_str().into(),
                format!("{:.0}", t.fill * 100.0).into(),
                format!("{:.3}", t.mass_t).into(),
                format!("{:.2}", t.fsm_t_m).into(),
            ]
        })
        .collect()
}

/// The flooded-compartment rows of the Damage tab: name, permeability,
/// submerged and lost volume, and the centroid one coordinate per column.
pub fn compartment_table(d: &DamageResult) -> Vec<Vec<EngCell>> {
    d.compartments
        .iter()
        .map(|c| {
            vec![
                c.name.as_str().into(),
                format!("{:.2}", c.permeability).into(),
                format!("{:.3}", c.submerged_volume_m3).into(),
                format!("{:.3}", c.lost_volume_m3).into(),
                format!("{:.2}", c.centroid.x).into(),
                format!("{:.2}", c.centroid.y).into(),
                format!("{:.2}", c.centroid.z).into(),
            ]
        })
        .collect()
}

/// A criteria row: the criterion, its value and unit apart, its verdict.
fn criterion_row(name: &str, value: String, unit: &str, passed: bool, stale: bool) -> Vec<EngCell> {
    vec![
        name.into(),
        value.into(),
        unit.into(),
        EngCell::Badge(passed, stale),
    ]
}

/// The criteria table's columns: criterion, value, unit, verdict.
fn criteria_columns() -> [EngColumn<'static>; 4] {
    [
        EngColumn::text("Criterion"),
        EngColumn::number("Value", ""),
        EngColumn::unit(),
        EngColumn::badge("Status"),
    ]
}

/// Height of the caption band above every plot, in points. No curve point
/// is drawn inside it, so the caption never sits on the data.
pub const PLOT_CAPTION_BAND: f32 = 20.0;
/// Plot captions, tick labels and legends: the theme's caption size.
const PLOT_FONT_SIZE: f32 = theme::CAPTION_SIZE;
/// Room left of the plot area for the y tick labels, below it for the x ones.
const PLOT_Y_LABEL_WIDTH: f32 = 42.0;
const PLOT_X_LABEL_HEIGHT: f32 = 16.0;
/// IMO A.749 thresholds drawn dashed on every GZ plot.
const GZ_THRESHOLD_HEELS_DEG: [f64; 2] = [30.0, 40.0];
const GZ_THRESHOLD_M: f64 = 0.20;

/// Split a plot rectangle into its caption band (top) and the body below.
pub fn split_caption(rect: egui::Rect) -> (egui::Rect, egui::Rect) {
    let band = PLOT_CAPTION_BAND.min(rect.height());
    let y = rect.top() + band;
    (
        egui::Rect::from_min_max(rect.min, egui::pos2(rect.right(), y)),
        egui::Rect::from_min_max(egui::pos2(rect.left(), y), rect.max),
    )
}

fn plot_caption(painter: &egui::Painter, caption: egui::Rect, text: &str) {
    painter.text(
        caption.left_center() + egui::vec2(4.0, 0.0),
        egui::Align2::LEFT_CENTER,
        text,
        theme::caption_font(),
        theme::PLOT_TEXT,
    );
}

/// Tick values on nice numbers (1, 2, 2.5 or 5 × 10ⁿ) that cover `[lo, hi]`,
/// four or five of them where the range allows (never fewer than three or
/// more than six). The first and last tick are the axis ends.
pub fn nice_ticks(lo: f64, hi: f64) -> Vec<f64> {
    let (lo, hi) = match (lo.is_finite(), hi.is_finite()) {
        (true, true) if hi > lo => (lo, hi),
        (true, _) => (lo, lo + 1.0),
        _ => (0.0, 1.0),
    };
    let base = 10f64.powf((hi - lo).log10().floor() - 1.0);
    let mut best: Option<(usize, f64, i64, i64)> = None;
    for k in 0..4 {
        for m in [1.0, 2.0, 2.5, 5.0] {
            let step = m * base * 10f64.powi(k);
            let i0 = (lo / step + 1e-9).floor() as i64;
            let i1 = (hi / step - 1e-9).ceil() as i64;
            let n = (i1 - i0 + 1).max(0) as usize;
            let score = match n {
                5 => 0,
                4 => 1,
                6 => 2,
                3 => 3,
                n => 4 + n.abs_diff(5),
            };
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, step, i0, i1));
            }
        }
    }
    let (_, step, i0, i1) = best.unwrap_or((0, 1.0, 0, 1));
    (i0..=i1).map(|i| i as f64 * step).collect()
}

/// Tick values on nice numbers (1, 2, 2.5 or 5 × 10ⁿ) that lie inside `[lo, hi]`.
pub fn nice_ticks_inside(lo: f64, hi: f64) -> Vec<f64> {
    let (lo, hi) = match (lo.is_finite(), hi.is_finite()) {
        (true, true) if hi > lo => (lo, hi),
        (true, _) => (lo, lo + 1.0),
        _ => (0.0, 1.0),
    };
    let span = (hi - lo).max(1e-9);
    let base = 10f64.powf(span.log10().floor() - 1.0);
    let mut best: Option<(usize, f64, i64, i64)> = None;
    for k in 0..4 {
        for m in [1.0, 2.0, 2.5, 5.0] {
            let step = m * base * 10f64.powi(k);
            let i0 = (lo / step - 1e-9).ceil() as i64;
            let i1 = (hi / step + 1e-9).floor() as i64;
            let n = if i1 >= i0 { (i1 - i0 + 1) as usize } else { 0 };
            let score = match n {
                5 => 0,
                4 => 1,
                6 => 2,
                3 => 3,
                n => 4 + n.abs_diff(5),
            };
            if best.is_none_or(|b| score < b.0) {
                best = Some((score, step, i0, i1));
            }
        }
    }
    let (_, step, i0, i1) = best.unwrap_or((0, 1.0, 0, 1));
    if i1 >= i0 {
        (i0..=i1).map(|i| i as f64 * step).collect()
    } else {
        vec![lo, hi]
    }
}

/// Decimals that print every tick of `ticks` exactly.
fn tick_decimals(ticks: &[f64]) -> usize {
    let step = match ticks {
        [a, b, ..] => (b - a).abs(),
        _ => return 0,
    };
    (0..6)
        .find(|&d| {
            let scaled = step * 10f64.powi(d as i32);
            (scaled - scaled.round()).abs() < 1e-6 * scaled.max(1.0)
        })
        .unwrap_or(6)
}

fn tick_label(value: f64, decimals: usize, suffix: &str) -> String {
    // Never print "-0".
    let value = if value.abs() < 1e-12 { 0.0 } else { value };
    format!("{value:.decimals$}{suffix}")
}

/// The geometry of one x/y plot: caption band, curve area, and the axis
/// ticks the data was fitted to.
/// Pure, so the layout can be tested without a frame.
#[derive(Debug, Clone)]
pub struct PlotAxes {
    pub caption: egui::Rect,
    pub area: egui::Rect,
    pub x_ticks: Vec<f64>,
    pub y_ticks: Vec<f64>,
    pub x_bounds: (f64, f64),
    pub y_bounds: (f64, f64),
    /// GZ plot: heel in degrees across, GZ in metres up, criteria dashed.
    pub gz: bool,
}

impl PlotAxes {
    /// Fit the axes to every finite point of `series` inside `rect`.
    ///
    /// The ranges always include zero; a GZ plot also spans 0–40° and
    /// 0–0.25 m so its criteria lines are in view.
    pub fn fit(rect: egui::Rect, series: &[(String, Vec<(f64, f64)>)], gz: bool) -> Self {
        let (caption, body) = split_caption(rect);
        let left = (body.left() + PLOT_Y_LABEL_WIDTH).min(body.right());
        let top = (body.top() + 8.0).min(body.bottom());
        let area = egui::Rect::from_min_max(
            egui::pos2(left, top),
            egui::pos2(
                (body.right() - 10.0).max(left),
                (body.bottom() - PLOT_X_LABEL_HEIGHT).max(top),
            ),
        );
        let (mut x_min, mut x_max): (f64, f64) = (0.0, if gz { 40.0 } else { 1.0 });
        let (mut y_min, mut y_max): (f64, f64) = (0.0, if gz { 0.25 } else { 0.1 });
        for (_, pts) in series {
            for &(x, y) in pts {
                if x.is_finite() && y.is_finite() {
                    x_min = x_min.min(x);
                    x_max = x_max.max(x);
                    y_min = y_min.min(y);
                    y_max = y_max.max(y);
                }
            }
        }
        let (x_bounds, x_ticks) = if gz {
            let t = nice_ticks(x_min, x_max);
            ((t[0], *t.last().unwrap_or(&40.0)), t)
        } else {
            let span = (x_max - x_min).max(1e-9);
            let pad = 0.02 * span;
            let lo = if x_min.abs() < 1e-9 { 0.0 } else { x_min - pad };
            let hi = x_max + pad;
            let ticks = nice_ticks_inside(lo, hi);
            let b_lo = lo.min(ticks.first().copied().unwrap_or(lo));
            let b_hi = hi.max(ticks.last().copied().unwrap_or(hi));
            ((b_lo, b_hi), ticks)
        };
        let (y_bounds, y_ticks) = {
            let t = nice_ticks(y_min, y_max);
            let b_lo = if y_min >= 0.0 && t[0] <= 0.0 {
                0.0
            } else {
                t[0]
            };
            let b_hi = *t.last().unwrap_or(&1.0);
            ((b_lo, b_hi), t)
        };
        Self {
            caption,
            area,
            x_ticks,
            y_ticks,
            x_bounds,
            y_bounds,
            gz,
        }
    }

    fn x_range(&self) -> (f64, f64) {
        self.x_bounds
    }

    fn y_range(&self) -> (f64, f64) {
        self.y_bounds
    }

    /// A data point on screen.
    pub fn to_screen(&self, x: f64, y: f64) -> egui::Pos2 {
        let (x0, x1) = self.x_range();
        let (y0, y1) = self.y_range();
        let tx = ((x - x0) / (x1 - x0).max(1e-12)) as f32;
        let ty = ((y - y0) / (y1 - y0).max(1e-12)) as f32;
        egui::pos2(
            self.area.left() + tx * self.area.width(),
            self.area.bottom() - ty * self.area.height(),
        )
    }

    /// One series on screen, split into runs wherever a value is missing
    /// (non-finite): a gap stays a gap, it is never bridged.
    pub fn polylines(&self, pts: &[(f64, f64)]) -> Vec<Vec<egui::Pos2>> {
        let mut runs = Vec::new();
        let mut run = Vec::new();
        for &(x, y) in pts {
            if x.is_finite() && y.is_finite() {
                run.push(self.to_screen(x, y));
            } else if !run.is_empty() {
                runs.push(std::mem::take(&mut run));
            }
        }
        if !run.is_empty() {
            runs.push(run);
        }
        runs
    }

    /// Grid, axes, tick labels and (on a GZ plot) the dashed criteria lines.
    fn paint_frame(&self, painter: &egui::Painter) {
        let font = theme::caption_font();
        let area = self.area;
        painter.rect_filled(area, 2.0, theme::PLOT_BG);
        let (x0, x1) = self.x_range();
        let (y0, y1) = self.y_range();
        let x_dec = tick_decimals(&self.x_ticks);
        let y_dec = tick_decimals(&self.y_ticks);
        let x_suffix = if self.gz { "°" } else { "" };
        for &x in &self.x_ticks {
            let sx = self.to_screen(x, y0).x;
            let colour = if x.abs() < 1e-12 {
                theme::PLOT_AXIS
            } else {
                theme::PLOT_GRID
            };
            painter.line_segment(
                [egui::pos2(sx, area.top()), egui::pos2(sx, area.bottom())],
                egui::Stroke::new(1.0, colour),
            );
            // The end labels sit inside the axis span: near the left edge
            // start rightward so the corner never collides; near the right
            // edge never leave the plot.
            let align = if sx - area.left() < 18.0 {
                egui::Align2::LEFT_TOP
            } else if area.right() - sx < 18.0 {
                egui::Align2::RIGHT_TOP
            } else {
                egui::Align2::CENTER_TOP
            };
            painter.text(
                egui::pos2(sx, area.bottom() + 2.0),
                align,
                tick_label(x, x_dec, x_suffix),
                font.clone(),
                theme::PLOT_TEXT,
            );
        }
        // The y labels, with the vertical span each one covers, so the
        // threshold label can keep clear of them.
        let mut y_spans: Vec<(f32, f32)> = Vec::new();
        for &y in &self.y_ticks {
            let sy = self.to_screen(x0, y).y;
            let colour = if y.abs() < 1e-12 {
                theme::PLOT_AXIS
            } else {
                theme::PLOT_GRID
            };
            painter.line_segment(
                [egui::pos2(area.left(), sy), egui::pos2(area.right(), sy)],
                egui::Stroke::new(1.0, colour),
            );
            // Centred on their lines, so evenly spaced ticks keep evenly
            // spaced labels; the corner is cleared by the x labels instead.
            let r = painter.text(
                egui::pos2(area.left() - 4.0, sy),
                egui::Align2::RIGHT_CENTER,
                tick_label(y, y_dec, ""),
                font.clone(),
                theme::PLOT_TEXT,
            );
            y_spans.push((r.top(), r.bottom()));
        }
        if !self.gz {
            return;
        }
        let dash = egui::Stroke::new(1.0, theme::PLOT_THRESHOLD);
        for heel in GZ_THRESHOLD_HEELS_DEG {
            if heel > x0 && heel < x1 {
                let sx = self.to_screen(heel, y0).x;
                painter.extend(egui::Shape::dashed_line(
                    &[egui::pos2(sx, area.top()), egui::pos2(sx, area.bottom())],
                    dash,
                    4.0,
                    3.0,
                ));
            }
        }
        if GZ_THRESHOLD_M > y0 && GZ_THRESHOLD_M < y1 {
            let sy = self.to_screen(x0, GZ_THRESHOLD_M).y;
            painter.extend(egui::Shape::dashed_line(
                &[egui::pos2(area.left(), sy), egui::pos2(area.right(), sy)],
                dash,
                4.0,
                3.0,
            ));
            // Label it in the tick column unless it would touch a tick label.
            let half = 0.5 * PLOT_FONT_SIZE + 2.0;
            let clear = y_spans
                .iter()
                .all(|&(top, bottom)| sy + half < top || sy - half > bottom);
            if clear {
                painter.text(
                    egui::pos2(area.left() - 4.0, sy),
                    egui::Align2::RIGHT_CENTER,
                    format!("{GZ_THRESHOLD_M:.2}"),
                    font,
                    theme::PLOT_THRESHOLD,
                );
            }
        }
    }
}

/// Height of a plot (caption band, curves and tick labels), legend apart.
const PLOT_HEIGHT: f32 = 172.0;
/// Space between legend entries across, and between legend rows.
const LEGEND_GAP_X: f32 = 10.0;
const LEGEND_GAP_Y: f32 = 2.0;

/// Where each legend entry of the given sizes goes: left to right from
/// `origin`, wrapping onto a further row before an entry would cross
/// `max_width`. An entry wider than the plot gets a row of its own.
pub fn legend_layout(origin: egui::Pos2, max_width: f32, sizes: &[egui::Vec2]) -> Vec<egui::Rect> {
    let mut rects = Vec::with_capacity(sizes.len());
    let mut x = 0.0_f32;
    let mut y = 0.0_f32;
    let mut row_height = 0.0_f32;
    for &size in sizes {
        if x > 0.0 && x + size.x > max_width {
            x = 0.0;
            y += row_height + LEGEND_GAP_Y;
            row_height = 0.0;
        }
        rects.push(egui::Rect::from_min_size(origin + egui::vec2(x, y), size));
        x += size.x + LEGEND_GAP_X;
        row_height = row_height.max(size.y);
    }
    rects
}

/// Draw a captioned x/y plot with ticks, and its legend underneath (outside
/// the curves) when there is more than one labelled series. The legend
/// wraps onto as many rows as it needs within the plot's width. Returns the
/// rectangle drawn, legend included.
fn draw_plot(
    ui: &mut egui::Ui,
    caption: &str,
    legend_title: &str,
    series: &[(String, Vec<(f64, f64)>)],
    gz: bool,
    height: f32,
) -> egui::Rect {
    let width = ui.available_width();
    let font = theme::caption_font();
    // The legend's entries, laid out before the space is allocated so the
    // allocation covers every row.
    let labelled = series.iter().filter(|(l, _)| !l.is_empty()).count();
    let mut entries: Vec<(String, egui::Color32)> = Vec::new();
    if series.len() > 1 && labelled > 0 {
        if !legend_title.is_empty() {
            entries.push((legend_title.to_owned(), theme::PLOT_TEXT));
        }
        for (i, (label, _)) in series.iter().enumerate() {
            if !label.is_empty() {
                entries.push((
                    format!("━ {label}"),
                    theme::PLOT_SERIES[i % theme::PLOT_SERIES.len()],
                ));
            }
        }
    }
    let galleys: Vec<_> = entries
        .into_iter()
        .map(|(text, colour)| {
            let galley = ui
                .painter()
                .layout(text, font.clone(), colour, width.max(1.0));
            (galley, colour)
        })
        .collect();
    let sizes: Vec<egui::Vec2> = galleys.iter().map(|(g, _)| g.size()).collect();
    let legend = legend_layout(egui::Pos2::ZERO, width, &sizes);
    let legend_height = legend.iter().map(|r| r.bottom()).fold(0.0_f32, f32::max);
    let legend_height = if legend.is_empty() {
        0.0
    } else {
        legend_height + 4.0
    };

    let (response, painter) = ui.allocate_painter(
        egui::vec2(width, height + legend_height),
        egui::Sense::hover(),
    );
    let plot = egui::Rect::from_min_size(response.rect.min, egui::vec2(width, height));
    let axes = PlotAxes::fit(plot, series, gz);
    axes.paint_frame(&painter);
    for (i, (_, pts)) in series.iter().enumerate() {
        let colour = theme::PLOT_SERIES[i % theme::PLOT_SERIES.len()];
        for run in axes.polylines(pts) {
            if run.len() == 1 {
                painter.circle_filled(run[0], 1.8, colour);
            } else {
                painter.add(egui::Shape::line(run, egui::Stroke::new(1.4, colour)));
            }
        }
    }
    plot_caption(&painter, axes.caption, caption);
    let origin = egui::pos2(plot.left() + 4.0, plot.bottom() + 2.0);
    for ((galley, colour), r) in galleys.into_iter().zip(&legend) {
        painter.galley(origin + r.min.to_vec2(), galley, colour);
    }
    response.rect
}

/// The cross curves as plot series: one per displacement, heel across and
/// GZ up, labelled with the displacement in tonnes.
///
/// `stability::cross_curves` returns one curve per heel, each holding the
/// displacements; this transposes it. Heels come out in increasing order. A
/// point that did not solve (`gz: None`) is NaN, so the plotter breaks the
/// line there instead of bridging the gap.
pub fn cross_curve_polylines(curves: &[CrossCurve]) -> Vec<(String, Vec<(f64, f64)>)> {
    let mut by_heel: Vec<&CrossCurve> = curves.iter().filter(|c| c.heel_deg.is_finite()).collect();
    by_heel.sort_by(|a, b| a.heel_deg.total_cmp(&b.heel_deg));
    let n = by_heel.iter().map(|c| c.points.len()).max().unwrap_or(0);
    (0..n)
        .map(|j| {
            let disp = by_heel
                .iter()
                .find_map(|c| c.points.get(j))
                .map_or(f64::NAN, |p| p.displacement_t);
            let pts = by_heel
                .iter()
                .map(|c| {
                    let gz = c
                        .points
                        .get(j)
                        .and_then(|p| p.gz)
                        .filter(|g| g.is_finite())
                        .unwrap_or(f64::NAN);
                    (c.heel_deg, gz)
                })
                .collect();
            (format!("{disp:.1} t"), pts)
        })
        .collect()
}

/// The dock's GZ plot: heel across, righting lever up, one line per
/// displacement, for the bare hull at the dock KG.
fn plot_cross_curves(ui: &mut egui::Ui, curves: &[CrossCurve], kg: f64) {
    let rect = draw_plot(
        ui,
        &format!("Cross curves — bare hull at dock KG {kg:.2} m"),
        "GZ m at Δ",
        &cross_curve_polylines(curves),
        true,
        PLOT_HEIGHT,
    );
    remember_plot_rect(ui.ctx(), "cross_curves", rect);
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
    /// The lightship a new condition starts from (the measured Δ at its
    /// LCB, at the design KG).
    pub lightship: WeightItem,
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

/// A general 2D XY plot with labelled series, ticked axes, and a caption.
///
/// The title `"GZ"` makes it a GZ plot: heel in degrees across, the IMO
/// thresholds (30°, 40°, 0.20 m) dashed.
pub fn plot_xy(
    ui: &mut egui::Ui,
    title: &str,
    x_label: &str,
    y_label: &str,
    series: &[(String, Vec<(f64, f64)>)],
) {
    let is_gz = title == "GZ";
    let caption = if is_gz {
        "GZ (m) vs heel (°)".to_string()
    } else {
        format!("{title}: {y_label} vs {x_label}")
    };
    draw_plot(ui, &caption, "", series, is_gz, PLOT_HEIGHT);
}

/// A general GZ plot: heel (deg) across, righting lever GZ (m) up, with labelled curves.
pub fn plot_gz(ui: &mut egui::Ui, curves: &[(String, Vec<(f64, f64)>)]) {
    plot_xy(ui, "GZ", "heel", "GZ", curves);
}

pub fn loading(ui: &mut egui::Ui, panel: LoadingPanel<'_>) -> LoadingAction {
    let mut action = LoadingAction::None;
    ui.heading("Loading condition");
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
            lightship: panel.lightship.clone(),
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
        let gone = panel
            .project
            .loading_conditions
            .remove(*panel.selected)
            .name;
        // A damage case that used it falls back to its own particulars,
        // rather than naming a condition that no longer exists (which would
        // make the project unsavable).
        for case in &mut panel.project.damage_cases {
            if case.loading_condition_name.as_deref() == Some(gone.as_str()) {
                case.loading_condition_name = None;
            }
        }
        if *panel.selected >= panel.project.loading_conditions.len() && *panel.selected > 0 {
            *panel.selected -= 1;
        }
        return LoadingAction::Changed;
    }

    if panel.project.loading_conditions.is_empty() {
        return action;
    }

    let cond = &mut panel.project.loading_conditions[*panel.selected];

    // Editable name; damage cases that refer to it follow the rename.
    let old_name = cond.name.clone();
    ui.horizontal(|ui| {
        ui.label("Name:");
        if ui.text_edit_singleline(&mut cond.name).changed() {
            action = LoadingAction::Changed;
        }
    });
    let name = cond.name.clone();
    if name != old_name {
        for case in &mut panel.project.damage_cases {
            if case.loading_condition_name.as_deref() == Some(old_name.as_str()) {
                case.loading_condition_name = Some(name.clone());
            }
        }
    }
    if panel
        .project
        .loading_conditions
        .iter()
        .filter(|c| c.name == name)
        .count()
        > 1
    {
        ui.colored_label(
            theme::FAIL,
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
        ui.label(egui::RichText::new("Deadweight items").strong());
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
        let name_width = (ui.available_width() - 280.0).clamp(190.0, 320.0);
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
                        .add_sized(
                            [name_width, 20.0],
                            egui::TextEdit::singleline(&mut item.name),
                        )
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
                    if ui.button("✕").on_hover_text("Remove weight").clicked() {
                        remove_item = Some(i);
                    }
                    ui.end_row();
                }
                if let Some(i) = remove_item {
                    cond.items.remove(i);
                    action = LoadingAction::Changed;
                }
            });
    } else {
        ui.label(
            egui::RichText::new(
                "No deadweight items. Click '+ Add weight' to add cargo or equipment.",
            )
            .color(theme::TEXT_MUTED),
        );
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

    if cond.tanks.is_empty() {
        ui.label(
            egui::RichText::new("No tanks defined. Click '+ Add tank' to add a liquid tank.")
                .color(theme::TEXT_MUTED),
        );
    }

    let mut remove_tank = None;
    for (t_idx, tank) in cond.tanks.iter_mut().enumerate() {
        let is_highlighted = panel.highlight == Some(t_idx);
        let mut frame = egui::Frame::group(ui.style());
        if is_highlighted {
            frame = frame.stroke(egui::Stroke::new(2.0, theme::HIGHLIGHT_TANK));
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
                if ui.button("✕").on_hover_text("Remove tank").clicked() {
                    remove_tank = Some(t_idx);
                }
            });
            ui.horizontal(|ui| {
                ui.label("Fill:");
                let mut fill_pct = (tank.fill * 100.0).round();
                if ui
                    .add(
                        egui::Slider::new(&mut fill_pct, 0.0..=100.0)
                            .fixed_decimals(0)
                            .suffix("%"),
                    )
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
        }
    });

    // Results
    if let Some(r) = panel.result {
        ui.separator();
        if panel.stale {
            stale_chip(ui);
            // The results close the panel: the dimming ends with it.
            ui.multiply_opacity(theme::STALE_OPACITY);
        }
        ui.heading("Equilibrium & hydrostatics");
        if !r.equilibrium_found {
            ui.colored_label(
                theme::WARN,
                format!("⚠ {}", seacraft_core::loading::NO_EQUILIBRIUM_NOTE),
            );
            for w in &r.warnings {
                ui.colored_label(theme::WARN, w);
            }
        }
        readouts(ui, "loading_readouts", &loading_rows(r));

        if !r.tanks.is_empty() {
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Tank summary").strong());
            eng_table(
                ui,
                &[
                    EngColumn::text("Tank"),
                    EngColumn::number("Fill", "%"),
                    EngColumn::number("Mass", "t"),
                    EngColumn::number("FSM", "t·m"),
                ],
                &tank_table(r),
            );
        }

        if let Some(imo) = &r.imo_criteria {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("IMO A.749 criteria").strong());
                criteria_badge(ui, imo.passed, panel.stale);
            });
            not_validated_note(ui);
            let stale = panel.stale;
            let rows = [
                criterion_row(
                    "A(0–30°) ≥ 0.055",
                    format!("{:.3}", imo.area_0_30_m_rad),
                    "m·rad",
                    imo.area_0_30_m_rad >= 0.055,
                    stale,
                ),
                criterion_row(
                    "A(0–40°) ≥ 0.090",
                    format!("{:.3}", imo.area_0_40_m_rad),
                    "m·rad",
                    imo.area_0_40_m_rad >= 0.090,
                    stale,
                ),
                criterion_row(
                    "A(30–40°) ≥ 0.030",
                    format!("{:.3}", imo.area_30_40_m_rad),
                    "m·rad",
                    imo.area_30_40_m_rad >= 0.030,
                    stale,
                ),
                criterion_row(
                    "GZ(≥30°) ≥ 0.20",
                    format!("{:.3}", imo.max_gz_30_plus_m),
                    "m",
                    imo.max_gz_30_plus_m >= 0.20,
                    stale,
                ),
                criterion_row(
                    "φ(GZmax) ≥ 25°",
                    format!("{:.0}", imo.angle_of_max_gz_deg),
                    "°",
                    imo.angle_of_max_gz_deg >= 25.0,
                    stale,
                ),
                criterion_row(
                    "GM0 ≥ 0.15",
                    format!("{:.3}", imo.gm0_m),
                    "m",
                    imo.gm0_m >= 0.15,
                    stale,
                ),
            ];
            eng_table(ui, &criteria_columns(), &rows);
        }

        if let Some(w) = &r.weather {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Weather criterion (§2.3)").strong());
                criteria_badge(ui, w.passed, panel.stale);
            });
            if r.imo_criteria.is_none() {
                not_validated_note(ui);
            }
            readouts(ui, "weather_readouts", &weather_rows(w));
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
            ui.label(egui::RichText::new("Righting lever GZ").strong());
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
    /// Model `z` of the keel (the lowest hydrostatic point), the datum the
    /// totals refer a profiled item's model-`z` centroid to.
    pub keel_z: f64,
}

/// Total area (m²) and area-weighted centroid above the keel (m) of the
/// windage `items`, the keel at model `z` `keel_z`: the datum the weather
/// criterion uses ([`seacraft_core::weather::extra_windage_moment`]), so
/// the totals do not move with the model.
pub fn windage_totals(items: &[WindageItem], keel_z: f64) -> (f64, f64) {
    let (area, moment) = seacraft_core::weather::extra_windage_moment(items, keel_z);
    let z = if area > 1e-9 { moment / area } else { 0.0 };
    (area, z)
}

pub fn windage(ui: &mut egui::Ui, panel: WindagePanel<'_>) -> WindageAction {
    let mut action = WindageAction::None;
    ui.heading("Superstructure & windage");
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

                    if ui
                        .button("✕")
                        .on_hover_text("Remove windage item")
                        .clicked()
                    {
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
                            egui::Stroke::new(1.5, theme::HIGHLIGHT_WINDAGE),
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
    ui.label(egui::RichText::new("Windage totals").strong());
    let (total_area, z_centroid) = windage_totals(&panel.project.windage, panel.keel_z);
    egui::Grid::new("windage_totals_grid")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("Total windage area (ΣA):");
            ui.label(format!("{:.2} m²", total_area));
            ui.end_row();
            ui.label("Area-weighted z above keel:");
            ui.label(format!("{:.2} m", z_centroid));
            ui.end_row();
        });

    ui.add_space(8.0);
    ui.separator();
    if let Some(w) = panel.weather {
        if panel.stale {
            stale_chip(ui);
            // The summary closes the panel: the dimming ends with it.
            ui.multiply_opacity(theme::STALE_OPACITY);
        }
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Weather criterion summary (§2.3)").strong());
            criteria_badge(ui, w.passed, panel.stale);
        });
        not_validated_note(ui);
        ui.small("from the loading panel's last Compute");
        ui.add_space(2.0);

        let summary_rows = [
            Readout::new(
                "Hull lateral windage area",
                format!("{:.2}", w.hull_windage_area_m2),
                "m²",
            ),
            Readout::new(
                "Extra windage area",
                format!("{:.2}", w.extra_windage_area_m2),
                "m²",
            ),
            Readout::new("Total windage area (A)", format!("{:.2}", w.area_m2), "m²"),
            Readout::new("Vertical lever arm (Z)", format!("{:.2}", w.z_m), "m"),
            Readout::new("Wind heeling lever (lw1)", format!("{:.3}", w.lw1_m), "m"),
            Readout::new(
                "Steady heel angle (φ0)",
                format!("{:.1}", w.phi0_deg.unwrap_or(0.0)),
                "° (lim 16°)",
            ),
            Readout::new(
                "Energy ratio (b/a)",
                format!("{:.2}", w.area_ratio),
                "(req ≥1.0)",
            ),
        ];
        readouts(ui, "windage_weather_grid", &summary_rows);
    } else {
        ui.label("Compute a loading condition to check the weather criterion");
    }

    action
}

// ---- toolbar -----------------------------------------------------------

/// The parameter of each surface-analysis mode, remembered while another
/// mode is shown so Zebra → Curvature → Zebra keeps the stripe count.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AnalysisParams {
    pub stripes: f64,
    /// Radius (m) at which the curvature colour scale saturates.
    pub radius: f64,
    pub angle_deg: f64,
}

impl Default for AnalysisParams {
    fn default() -> Self {
        Self {
            stripes: 14.0,
            radius: 1.0,
            angle_deg: 3.0,
        }
    }
}

impl AnalysisParams {
    /// The mode of the same kind as `kind`, carrying the remembered parameter.
    pub fn with_kind(&self, kind: AnalysisMode) -> AnalysisMode {
        match kind {
            AnalysisMode::Shaded => AnalysisMode::Shaded,
            AnalysisMode::Zebra { .. } => AnalysisMode::Zebra {
                stripes: self.stripes,
            },
            AnalysisMode::Curvature { .. } => AnalysisMode::Curvature {
                radius: self.radius,
            },
            AnalysisMode::Draft { .. } => AnalysisMode::Draft {
                angle_deg: self.angle_deg,
            },
        }
    }

    /// Remember the parameter `mode` carries.
    pub fn remember(&mut self, mode: AnalysisMode) {
        match mode {
            AnalysisMode::Shaded => {}
            AnalysisMode::Zebra { stripes } => self.stripes = stripes,
            AnalysisMode::Curvature { radius } => self.radius = radius,
            AnalysisMode::Draft { angle_deg } => self.angle_deg = angle_deg,
        }
    }
}

/// Whether `a` and `b` are the same analysis mode, whatever their parameters.
pub fn same_analysis_kind(a: AnalysisMode, b: AnalysisMode) -> bool {
    std::mem::discriminant(&a) == std::mem::discriminant(&b)
}

/// What the toolbar shows.
pub struct ToolbarState<'a> {
    pub display: &'a DisplayOptions,
    pub params: AnalysisParams,
    pub level: u8,
    pub mode: SubdivisionMode,
    pub maximised: bool,
    /// A model is loaded. Without one the view controls (analysis mode,
    /// display toggles, Fit, Maximise) are disabled; the subdivision level
    /// and scheme stay editable, as settings the next model is shown with.
    pub has_model: bool,
}

/// The one line a dock shows with no model loaded, in place of controls
/// that would have nothing to act on.
pub const NO_MODEL_NOTE: &str = "No model — start a new design or open one";

/// Draw [`NO_MODEL_NOTE`], muted and small enough for one line in the
/// layer dock, at the top of a dock. The dock keeps its width: a panel
/// would otherwise shrink to the note and widen again when a model arrives.
pub fn no_model_note(ui: &mut egui::Ui) {
    ui.set_min_width(ui.available_width());
    ui.add_space(4.0);
    ui.add(
        egui::Label::new(
            egui::RichText::new(NO_MODEL_NOTE)
                .small()
                .color(theme::TEXT_MUTED),
        )
        .truncate(),
    );
}

/// The intersection-curve counts and the comb settings, for the toolbar's
/// "…" menu and View > Curves and combs. Edits `display` and returns whether
/// anything changed; the caller applies it.
pub fn section_settings(ui: &mut egui::Ui, display: &mut DisplayOptions) -> bool {
    use crate::render::{
        BUTTOCKS_RANGE, COMB_SCALE_RANGE, COMB_STRIDE_RANGE, STATIONS_RANGE, WATERLINES_RANGE,
    };
    let mut changed = false;
    ui.label(egui::RichText::new("Intersection curves").color(theme::TEXT_SECONDARY));
    egui::Grid::new("section_counts")
        .num_columns(2)
        .show(ui, |ui| {
            for (label, value, range, tip) in [
                (
                    "stations",
                    &mut display.stations,
                    STATIONS_RANGE,
                    "Stations (x = const) from stern to bow, ends included",
                ),
                (
                    "waterlines",
                    &mut display.waterlines,
                    WATERLINES_RANGE,
                    "Waterlines (z = const) from keel to top, both included",
                ),
                (
                    "buttocks",
                    &mut display.buttocks,
                    BUTTOCKS_RANGE,
                    "Buttocks (y = const) across the beam, sides included",
                ),
            ] {
                ui.label(label);
                changed |= ui
                    .add(egui::DragValue::new(value).range(range).speed(0.1))
                    .on_hover_text(tip)
                    .changed();
                ui.end_row();
            }
        });
    ui.separator();
    ui.label(egui::RichText::new("Curvature combs").color(theme::TEXT_SECONDARY));
    egui::Grid::new("comb_settings")
        .num_columns(2)
        .show(ui, |ui| {
            ui.label("scale");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut display.comb_scale)
                        .range(COMB_SCALE_RANGE)
                        .speed(0.01)
                        .max_decimals(2),
                )
                .on_hover_text("Comb gain: metres of spike for a curvature of 1/m")
                .changed();
            ui.end_row();
            ui.label("density");
            changed |= ui
                .add(
                    egui::DragValue::new(&mut display.comb_stride)
                        .range(COMB_STRIDE_RANGE)
                        .speed(0.05)
                        .prefix("1/"),
                )
                .on_hover_text("Draw every n-th comb spike: 1/1 draws them all")
                .changed();
            ui.end_row();
        });
    changed
}

/// What the user asked the toolbar for; `App::draw` applies it.
#[derive(Debug, Clone, Default)]
pub struct ToolbarOutput {
    /// A new analysis mode, or the same mode with a new parameter.
    pub analysis: Option<AnalysisMode>,
    /// The display toggles, when one of them changed.
    pub display: Option<DisplayOptions>,
    pub level: Option<u8>,
    pub mode: Option<SubdivisionMode>,
    /// Frame the model in every view (F).
    pub fit: bool,
    /// Maximise the active view, or restore the four views.
    pub toggle_maximise: bool,
    /// Where each control was drawn, by name (a test hook): `shaded`,
    /// `zebra`, `curvature`, `draft`, `param`, `net`, `points`,
    /// `waterplane`, `curves`, `combs`, `sections` (the counts menu),
    /// `wireframe`, `level`, `level:N` (while the list is open), `mode`,
    /// `fit`, `maximise`.
    pub rects: Vec<(&'static str, egui::Rect)>,
}

const LEVEL_ITEMS: [&str; 6] = [
    "level:0", "level:1", "level:2", "level:3", "level:4", "level:5",
];

/// The slim toolbar under the menu bar: analysis mode and its parameter,
/// display toggles, subdivision level and scheme, Fit and Maximise. One row
/// at 1600 px; it wraps on a narrower window.
pub fn toolbar(ui: &mut egui::Ui, state: &ToolbarState<'_>) -> ToolbarOutput {
    let mut out = ToolbarOutput::default();
    let ctx = ui.ctx().clone();
    let keys = |modifiers: egui::Modifiers, key: egui::Key| {
        ctx.format_shortcut(&egui::KeyboardShortcut::new(modifiers, key))
    };
    let fit_key = keys(egui::Modifiers::NONE, egui::Key::F);
    let max_key = keys(egui::Modifiers::COMMAND, egui::Key::Space);

    ui.horizontal_wrapped(|ui| {
        // Analysis mode: a segmented control.
        let current = state.display.analysis;
        ui.horizontal(|ui| {
            if !state.has_model {
                ui.disable();
            }
            ui.spacing_mut().item_spacing.x = 0.0;
            for (name, kind, label, tip) in [
                (
                    "shaded",
                    AnalysisMode::Shaded,
                    "Shaded",
                    "Lit, flat-shaded hull",
                ),
                (
                    "zebra",
                    AnalysisMode::Zebra { stripes: 0.0 },
                    "Zebra",
                    "Reflection stripes: an unfair surface breaks them",
                ),
                (
                    "curvature",
                    AnalysisMode::Curvature { radius: 0.0 },
                    "Curvature",
                    "Mean curvature map",
                ),
                (
                    "draft",
                    AnalysisMode::Draft { angle_deg: 0.0 },
                    "Draft",
                    "Draft angle of each face against the vertical",
                ),
            ] {
                let response = ui
                    .selectable_label(same_analysis_kind(current, kind), label)
                    .on_hover_text(tip);
                if response.clicked() && !same_analysis_kind(current, kind) {
                    out.analysis = Some(state.params.with_kind(kind));
                }
                out.rects.push((name, response.rect));
            }
        });

        // The active mode's parameter, in a fixed-width slot so the toggles
        // to its right do not jump when the mode changes.
        let slot = egui::vec2(126.0, ui.spacing().interact_size.y);
        ui.allocate_ui_with_layout(
            slot,
            egui::Layout::left_to_right(egui::Align::Center),
            |ui| {
                if !state.has_model {
                    ui.disable();
                }
                ui.set_min_size(slot);
                ui.spacing_mut().item_spacing.x = 4.0;
                let param = match current {
                    AnalysisMode::Shaded => None,
                    AnalysisMode::Zebra { stripes } => Some((
                        "stripes",
                        stripes,
                        2.0..=64.0,
                        0.25,
                        "",
                        "Number of reflection stripes across the view",
                    )),
                    AnalysisMode::Curvature { radius } => Some((
                        "radius",
                        radius,
                        0.05..=100.0,
                        0.01,
                        " m",
                        "Radius at which the curvature colour scale saturates",
                    )),
                    AnalysisMode::Draft { angle_deg } => Some((
                        "angle",
                        angle_deg,
                        0.0..=45.0,
                        0.1,
                        "°",
                        "Draft angle that separates pullable, borderline and undercut faces",
                    )),
                };
                if let Some((name, mut value, range, speed, suffix, tip)) = param {
                    ui.label(egui::RichText::new(name).color(theme::TEXT_SECONDARY));
                    let response = ui
                        .add(
                            egui::DragValue::new(&mut value)
                                .range(range)
                                .speed(speed)
                                .max_decimals(2)
                                .suffix(suffix),
                        )
                        .on_hover_text(tip);
                    if response.changed() {
                        out.analysis = Some(match current {
                            AnalysisMode::Shaded => AnalysisMode::Shaded,
                            AnalysisMode::Zebra { .. } => AnalysisMode::Zebra { stripes: value },
                            AnalysisMode::Curvature { .. } => {
                                AnalysisMode::Curvature { radius: value }
                            }
                            AnalysisMode::Draft { .. } => {
                                AnalysisMode::Draft { angle_deg: value }
                            }
                        });
                    }
                    out.rects.push(("param", response.rect));
                }
            },
        );
        ui.separator();

        // Display toggles, the same fields as the View menu's checkboxes.
        let mut display = state.display.clone();
        let mut toggled = false;
        ui.horizontal(|ui| {
            if !state.has_model {
                ui.disable();
            }
            ui.spacing_mut().item_spacing.x = 2.0;
            for (name, label, tip, flag) in [
                (
                    "net",
                    "Net",
                    "Control net",
                    &mut display.control_net as &mut bool,
                ),
                (
                    "points",
                    "Points",
                    "Control points (hidden points cannot be picked)",
                    &mut display.points,
                ),
                (
                    "waterplane",
                    "Waterplane",
                    "Waterplane at the current draft",
                    &mut display.waterplane,
                ),
                (
                    "curves",
                    "Curves",
                    "Station, buttock and waterline intersection curves",
                    &mut display.curves,
                ),
                (
                    "combs",
                    "Combs",
                    "Curvature combs along the stations, buttocks and waterlines",
                    &mut display.combs,
                ),
                (
                    "wireframe",
                    "Wire",
                    "Wireframe of the refined mesh",
                    &mut display.wireframe,
                ),
            ] {
                let response = ui.selectable_label(*flag, label).on_hover_text(tip);
                if response.clicked() {
                    *flag = !*flag;
                    toggled = true;
                }
                out.rects.push((name, response.rect));
            }
            // The curve counts and comb settings, in a small menu.
            let menu = ui
                .menu_button("…", |ui| {
                    if section_settings(ui, &mut display) {
                        toggled = true;
                    }
                })
                .response
                .on_hover_text("Section counts (stations, waterlines, buttocks) and comb scale and density");
            out.rects.push(("sections", menu.rect));
        });
        if toggled {
            out.display = Some(display);
        }
        ui.separator();

        // Subdivision level and scheme, applied through the setters.
        let mut level = state.level;
        let combo = egui::ComboBox::from_id_salt("toolbar_level")
            .selected_text(format!("L{level}"))
            .width(44.0)
            .show_ui(ui, |ui| {
                for (n, name) in LEVEL_ITEMS.iter().enumerate() {
                    let response = ui.selectable_value(&mut level, n as u8, format!("Level {n}"));
                    out.rects.push((name, response.rect));
                }
            });
        let combo = combo
            .response
            .on_hover_text("Subdivision level, for display and measurement (0–5)");
        out.rects.push(("level", combo.rect));
        if level != state.level {
            out.level = Some(level);
        }
        let mut mode = state.mode;
        let combo = egui::ComboBox::from_id_salt("toolbar_mode")
            .selected_text(match mode {
                SubdivisionMode::Loop => "Loop",
                SubdivisionMode::CatmullClark => "Catmull-Clark",
            })
            .width(104.0)
            .show_ui(ui, |ui| {
                ui.selectable_value(&mut mode, SubdivisionMode::Loop, "Loop");
                ui.selectable_value(&mut mode, SubdivisionMode::CatmullClark, "Catmull-Clark");
            });
        let combo = combo
            .response
            .on_hover_text("Subdivision scheme: hybrid Loop or Catmull-Clark");
        out.rects.push(("mode", combo.rect));
        if mode != state.mode {
            out.mode = Some(mode);
        }
        ui.separator();

        let fit = ui
            .add_enabled(state.has_model, egui::Button::new("Fit"))
            .on_hover_text(format!("Frame the model in every view ({fit_key})"));
        out.fit = fit.clicked();
        out.rects.push(("fit", fit.rect));
        // A framed button like Fit (an action, not a display toggle),
        // highlighted while a view is maximised.
        let max = ui
            .add_enabled(
                state.has_model,
                egui::Button::new(if state.maximised {
                    "Restore"
                } else {
                    "Maximise"
                })
                .selected(state.maximised),
            )
            .on_hover_text(format!(
                "Maximise the active view, or restore the four views ({max_key}, or double-click a view's title)"
            ));
        out.toggle_maximise = max.clicked();
        out.rects.push(("maximise", max.rect));
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::demo;
    use seacraft_core::hydro;
    use seacraft_core::stability::CrossCurvePoint;
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
        let [left, right] = hydrostatic_rows(&h, Some(1.5));
        // LWL and BWL are rows of their own now (R5-5): 9 and 10 rows.
        assert!(
            left.len().abs_diff(right.len()) <= 1,
            "the two columns stay balanced: {} vs {}",
            left.len(),
            right.len()
        );
        let labels: Vec<&str> = left.iter().chain(right.iter()).map(|r| r.name).collect();
        for want in [
            "volume", "Cb", "LCB", "TPC", "KM(t)", "GM", "midship", "MTC",
        ] {
            assert!(labels.contains(&want), "{want} missing from {labels:?}");
        }
        let find = |rows: &[Readout], name: &str| -> (f64, &'static str) {
            let r = rows.iter().find(|r| r.name == name).expect(name);
            (r.value.parse().expect("value parses"), r.unit)
        };
        // The table quantities are the measured waterplane in the right units.
        let (tpc, unit) = find(&right, "TPC");
        assert_eq!(unit, "t/cm");
        let want_tpc = h.waterplane_area_m2 * (h.water_density / 1000.0) / 100.0;
        assert!((tpc - want_tpc).abs() < 0.001, "TPC {tpc} vs {want_tpc}");
        assert!(
            (0.05..1.5).contains(&tpc),
            "TPC {tpc} t/cm is not plausible"
        );
        let (mtc, unit) = find(&right, "MTC");
        assert_eq!(unit, "t·m/cm");
        let want_mtc =
            (h.water_density / 1000.0) * h.waterplane_inertia_longitudinal / (100.0 * h.lwl);
        assert!((mtc - want_mtc).abs() < 0.001, "MTC {mtc} vs {want_mtc}");
        // GM follows KG: 1.5 m lighter than the measured KM by exactly that.
        let (gm, unit) = find(&right, "GM");
        assert_eq!(unit, "m");
        assert!((gm - (h.km_transverse - 1.5)).abs() < 0.002, "GM {gm}");
    }

    /// The readouts before the split, exactly as the dock printed them: value
    /// and unit glued into one string.
    fn glued_rows(h: &Hydrostatics, kg: f64) -> Vec<(&'static str, String)> {
        let rho_t = h.water_density / 1000.0;
        let tpc = h.waterplane_area_m2 * rho_t / 100.0;
        let mtc = rho_t * h.waterplane_inertia_longitudinal / (100.0 * h.lwl);
        // In the grouped order; the old combined "LWL / BWL" row is two
        // rows at the same precision.
        vec![
            ("volume", format!("{} m³", format_form_value(h.volume_m3))),
            (
                "displacement",
                format!("{} t", format_form_value(h.displacement_t)),
            ),
            ("Cb", format!("{:.3}", h.block_coefficient)),
            ("Cp", format!("{:.3}", h.prismatic_coefficient)),
            ("Cm", format!("{:.3}", h.midship_coefficient)),
            ("Cwp", format!("{:.3}", h.waterplane_coefficient)),
            ("LCB", format!("{:.3} m", h.lcb.x)),
            ("LCF", format!("{:.3} m", h.lcf.x)),
            ("KB", format!("{:.3} m", h.kb)),
            ("BM(t)", format!("{:.3} m", h.bm_transverse)),
            ("KM(t)", format!("{:.3} m", h.km_transverse)),
            ("GM", format!("{:+.3} m", h.gm(kg))),
            ("TPC", format!("{:.3} t/cm", tpc)),
            ("MTC", format!("{:.3} t·m/cm", mtc)),
            ("LWL", format!("{:.2} m", h.lwl)),
            ("BWL", format!("{:.2} m", h.bwl)),
            (
                "wetted",
                format!("{} m²", format_form_value(h.wetted_surface_m2)),
            ),
            (
                "waterplane",
                format!("{} m²", format_form_value(h.waterplane_area_m2)),
            ),
            (
                "midship",
                format!("{} m²", format_form_value(h.mainframe_area_m2)),
            ),
        ]
    }

    #[test]
    fn readouts_split_value_and_unit_without_changing_a_number() {
        let h = measured();
        let rows = hydrostatic_rows(&h, Some(2.2)).concat();
        let before = glued_rows(&h, 2.2);
        assert_eq!(rows.len(), before.len());
        for (r, (name, glued)) in rows.iter().zip(&before) {
            assert_eq!(r.name, *name);
            // Value and unit apart: no unit text inside the value.
            assert!(
                !r.value.contains(['m', 't', '²', '³']),
                "{name}: unit glued onto {:?}",
                r.value
            );
            let rejoined = if r.unit.is_empty() {
                r.value.clone()
            } else {
                format!("{} {}", r.value, r.unit)
            };
            assert_eq!(&rejoined, glued, "{name} changed");
        }
        // The demo reference numbers are the ones on show (the hull-numerics skill).
        let value = |name: &str| -> f64 {
            rows.iter()
                .find(|r| r.name == name)
                .and_then(|r| r.value.parse().ok())
                .expect(name)
        };
        assert!((value("volume") - 11.85).abs() < 0.06);
        assert!((value("Cb") - 0.504).abs() < 0.01);
        assert!((value("KB") - 0.392).abs() < 0.01);
        // Signed quantities carry their sign either way.
        let gm = rows.iter().find(|r| r.name == "GM").unwrap();
        assert!(gm.value.starts_with('+') || gm.value.starts_with('-'));
    }

    #[test]
    fn readout_values_align_right_and_names_are_body_size() {
        let h = measured();
        let rows = hydrostatic_rows(&h, Some(2.2)).concat();
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut texts = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                readouts(ui, "test_readouts", &rows);
            });
            out.textures_delta.clear();
            texts.clear();
            fn walk(shape: &egui::Shape, out: &mut Vec<(String, f32, f32, egui::Color32)>) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                    egui::Shape::Text(t) => {
                        let section = t.galley.job.sections.first();
                        out.push((
                            t.galley.text().to_string(),
                            section.map_or(0.0, |s| s.format.font_id.size),
                            t.pos.x + t.galley.size().x,
                            section.map_or(t.fallback_color, |s| s.format.color),
                        ));
                    }
                    _ => {}
                }
            }
            for clipped in &out.shapes {
                walk(&clipped.shape, &mut texts);
            }
        }
        for r in &rows {
            let name = texts.iter().find(|t| t.0 == r.name).expect(r.name);
            assert!(name.1 >= 12.0, "{} drawn at {} pt", r.name, name.1);
            assert_eq!(name.3, theme::TEXT_SECONDARY);
        }
        // Every value is its own text, right edges on one line.
        let values: Vec<_> = rows
            .iter()
            .map(|r| {
                texts
                    .iter()
                    .find(|t| t.0.trim() == r.value)
                    .unwrap_or_else(|| panic!("value {:?} not drawn", r.value))
            })
            .collect();
        let right = values[0].2;
        for v in &values {
            assert!(
                (v.2 - right).abs() < 0.5,
                "{:?} ends at {} not {right}",
                v.0,
                v.2
            );
            assert_eq!(v.1, theme::MONO_SIZE);
        }
        // Units are separate, muted texts.
        let tpc = texts.iter().find(|t| t.0 == "t/cm").expect("unit drawn");
        assert_eq!(tpc.3, theme::TEXT_MUTED);
    }

    #[test]
    fn loading_readouts_keep_their_numbers_and_sign_signed_ones() {
        let s = demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: Default::default(),
            },
        );
        let cond: LoadingCondition = serde_json::from_value(serde_json::json!({
            "name": "readouts",
            "lightship": {"name": "Lightship", "mass_t": 10.0, "x": 5.0, "y": -0.02, "z": 1.4},
        }))
        .expect("condition");
        let l = seacraft_core::loading::evaluate(&s, &opts, &cond);
        let rows = loading_rows(&l);
        let get = |name: &str| rows.iter().find(|r| r.name == name).unwrap().clone();
        // Value and unit apart, the value printed exactly as before the split.
        assert_eq!(
            get("Δ (mass)"),
            Readout::new("Δ (mass)", format!("{:.3}", l.displacement_t), "t")
        );
        assert_eq!(
            get("FSM"),
            Readout::new("FSM", format!("{:.2}", l.fsm_t_m), "t·m")
        );
        assert_eq!(get("Draft").value, format!("{:.3}", l.draft));
        // Signed quantities show their sign.
        assert_eq!(get("TCG").value, format!("{:+.3}", l.tcg));
        assert!(get("TCG").value.starts_with('-'), "{:?}", get("TCG"));
        assert_eq!(get("List").unit, "°");
        for name in ["GM (solid)", "GM (fluid)", "Trim", "List"] {
            let v = get(name).value;
            assert!(v.starts_with('+') || v.starts_with('-'), "{name}: {v}");
        }
        for r in &rows {
            let v: f64 = r.value.parse().expect("numeric value");
            assert!(v.is_finite(), "{}", r.name);
        }
    }

    #[test]
    fn the_sac_plot_runs_left_to_right_inside_the_rect() {
        let h = measured();
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 124.0));
        let series = sac_series(&h.sac);
        assert_eq!(series.len(), 1, "one curve, no legend");
        let axes = PlotAxes::fit(rect, &series, false);
        let pts = axes.polylines(&series[0].1).concat();
        assert_eq!(pts.len(), h.sac.len(), "one point per station");
        assert!(
            pts.windows(2).all(|w| w[1].x >= w[0].x),
            "x must not go backwards"
        );
        for p in &pts {
            assert!(axes.area.expand(1e-3).contains(*p), "{p:?}");
        }
        // The height is the area, linearly, from a zero baseline: the axes
        // are ticked like every other plot's.
        assert_eq!(axes.y_ticks[0], 0.0);
        for (point, station) in pts.iter().zip(h.sac.iter()) {
            let want = axes.to_screen(station.x, station.area);
            assert!(
                (point.y - want.y).abs() < 1e-2 && (point.y - axes.area.bottom()) <= 1e-2,
                "{station:?} drew at {point:?}, wanted {want:?}"
            );
        }
        let a_max = h.sac.iter().fold(0.0_f64, |m, s| m.max(s.area));
        let top = *axes.y_ticks.last().unwrap();
        assert!(
            top >= a_max && top < 2.0 * a_max,
            "y axis to {top} for {a_max}"
        );
        let peak = pts.iter().map(|p| p.y).fold(f32::INFINITY, f32::min);
        assert!(
            peak < axes.area.top() + 0.5 * axes.area.height(),
            "the peak is in the upper half: {peak}"
        );
        // Degenerate input draws nothing rather than dividing by zero.
        assert!(sac_series(&[])[0].1.is_empty());
        let nan = [SectionArea {
            x: 1.0,
            area: f64::NAN,
        }];
        assert!(sac_series(&nan)[0].1.is_empty());
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
                    lightship: WeightItem {
                        name: "Lightship".into(),
                        mass_t: 10.0,
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
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
                    lightship: WeightItem {
                        name: "Lightship".into(),
                        mass_t: 10.0,
                        x: 0.0,
                        y: 0.0,
                        z: 0.0,
                    },
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
                    keel_z: 0.0,
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
                    keel_z: 0.0,
                },
            );
            assert_eq!(action, WindageAction::None);
        });
        out.textures_delta.clear();
    }

    /// Every piece of text a frame painted, with its colour.
    fn painted_text(shapes: &[egui::epaint::ClippedShape]) -> Vec<(String, egui::Color32)> {
        fn walk(shape: &egui::Shape, out: &mut Vec<(String, egui::Color32)>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) => {
                    let colour = t
                        .galley
                        .job
                        .sections
                        .first()
                        .map_or(t.fallback_color, |s| s.format.color);
                    out.push((t.galley.text().to_string(), colour));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// Cross curves as `stability::cross_curves` returns them: one per heel,
    /// each over the same displacements. GZ = a·sin 2φ clears IMO A.749.
    fn synthetic_cross_curves(heels: &[f64], displacements: &[f64]) -> Vec<CrossCurve> {
        heels
            .iter()
            .map(|&heel| CrossCurve {
                heel_deg: heel,
                points: displacements
                    .iter()
                    .enumerate()
                    .map(|(j, &d)| {
                        let a = 0.5 + 0.05 * j as f64;
                        let gz = a * (2.0 * heel.to_radians()).sin();
                        CrossCurvePoint {
                            displacement_t: d,
                            draft: Some(0.6),
                            trim_m: Some(0.0),
                            kn: Some(gz + 2.0 * heel.to_radians().sin()),
                            gz: Some(gz),
                        }
                    })
                    .collect(),
            })
            .collect()
    }

    const HEELS: [f64; 9] = [0.0, 10.0, 20.0, 30.0, 40.0, 50.0, 60.0, 70.0, 80.0];
    const DISPS: [f64; 5] = [8.3, 10.1, 11.9, 13.7, 15.5];

    #[test]
    fn cross_curves_plot_one_line_per_displacement_across_heel() {
        // Shuffled heels: the plot must still run left to right.
        let mut curves = synthetic_cross_curves(&HEELS, &DISPS);
        curves.swap(0, 5);
        curves.swap(2, 8);
        let series = cross_curve_polylines(&curves);
        assert_eq!(series.len(), DISPS.len(), "one polyline per displacement");
        for (j, (label, pts)) in series.iter().enumerate() {
            assert_eq!(label, &format!("{:.1} t", DISPS[j]));
            assert_eq!(pts.len(), HEELS.len(), "one point per heel");
            assert!(
                pts.windows(2).all(|w| w[1].0 > w[0].0),
                "heel strictly increasing: {pts:?}"
            );
            for (k, &(heel, gz)) in pts.iter().enumerate() {
                assert_eq!(heel, HEELS[k]);
                let want = curves
                    .iter()
                    .find(|c| c.heel_deg == heel)
                    .and_then(|c| c.points[j].gz)
                    .unwrap();
                assert_eq!(gz, want, "GZ for {label} at {heel}°");
            }
        }
        // On screen: x strictly increasing too, and no line is vertical.
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 172.0));
        let axes = PlotAxes::fit(rect, &series, true);
        for (_, pts) in &series {
            let runs = axes.polylines(pts);
            assert_eq!(runs.len(), 1);
            assert!(runs[0].windows(2).all(|w| w[1].x > w[0].x));
        }
        assert!(cross_curve_polylines(&[]).is_empty());
    }

    #[test]
    fn a_missing_gz_breaks_the_cross_curve() {
        let mut curves = synthetic_cross_curves(&HEELS, &DISPS);
        curves[4].points[1].gz = None; // 40° at the second displacement
        let series = cross_curve_polylines(&curves);
        let (_, pts) = &series[1];
        assert_eq!(pts.len(), HEELS.len());
        assert!(pts[4].1.is_nan(), "the gap is kept, not dropped");
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(320.0, 172.0));
        let axes = PlotAxes::fit(rect, &series, true);
        let runs = axes.polylines(pts);
        assert_eq!(
            runs.iter().map(Vec::len).collect::<Vec<_>>(),
            vec![4, 4],
            "split at the missing point, never bridged"
        );
        assert_eq!(axes.polylines(&series[0].1).len(), 1, "others untouched");
    }

    #[test]
    fn no_curve_point_enters_the_caption_band() {
        let rect = egui::Rect::from_min_size(egui::pos2(10.0, 40.0), egui::vec2(300.0, 172.0));

        // SAC: the curve area starts below the band.
        let h = measured();
        let sac = sac_series(&h.sac);
        let axes = PlotAxes::fit(rect, &sac, false);
        assert_eq!(axes.caption.height(), PLOT_CAPTION_BAND);
        assert_eq!(axes.caption.top(), rect.top());
        let pts = axes.polylines(&sac[0].1).concat();
        assert!(pts.len() > 2);
        for p in &pts {
            assert!(p.y >= axes.caption.bottom(), "SAC point {p:?} in the band");
        }

        // plot_xy, plain and GZ, with the data at the extremes of its range.
        let series = vec![
            (
                "a".to_string(),
                vec![(-2.0, -0.4), (0.0, 0.0), (5.0, 3.7), (12.0, 9.9)],
            ),
            ("b".to_string(), vec![(1.0, 9.99), (11.0, -0.39)]),
        ];
        let gz = cross_curve_polylines(&synthetic_cross_curves(&HEELS, &DISPS));
        for (series, is_gz) in [(&series, false), (&gz, true)] {
            let axes = PlotAxes::fit(rect, series, is_gz);
            assert_eq!(axes.caption.height(), PLOT_CAPTION_BAND);
            assert!(axes.area.top() >= axes.caption.bottom());
            for (_, s) in series {
                for p in axes.polylines(s).into_iter().flatten() {
                    assert!(p.y >= axes.caption.bottom(), "{p:?} in the band");
                    assert!(axes.area.expand(1e-3).contains(p), "{p:?} off the plot");
                }
            }
            for ticks in [&axes.x_ticks, &axes.y_ticks] {
                assert!((4..=6).contains(&ticks.len()), "{ticks:?}");
            }
        }
    }

    #[test]
    fn ticks_land_on_nice_numbers() {
        assert_eq!(nice_ticks(0.0, 80.0), vec![0.0, 20.0, 40.0, 60.0, 80.0]);
        for (lo, hi) in [
            (0.0, 1.0),
            (0.0, 0.42),
            (-0.3, 0.6),
            (0.0, 0.1),
            (0.0, 12.7),
            (-1.2, 0.05),
            (3.0, 1234.0),
            (0.0, 40.0),
            (-0.05, 0.25),
        ] {
            let t = nice_ticks(lo, hi);
            assert!((4..=5).contains(&t.len()), "{lo}..{hi}: {t:?}");
            assert!(t[0] <= lo + 1e-12 && *t.last().unwrap() >= hi - 1e-12);
            let step = t[1] - t[0];
            assert!(t.windows(2).all(|w| ((w[1] - w[0]) - step).abs() < 1e-9));
            let mantissa = step / 10f64.powf(step.log10().floor());
            assert!(
                [1.0, 2.0, 2.5, 5.0]
                    .iter()
                    .any(|m| (mantissa - m).abs() < 1e-6),
                "{step} is not a nice step"
            );
        }
        assert_eq!(tick_label(-1e-17, 1, ""), "0.0");
        assert_eq!(tick_decimals(&[0.0, 0.25, 0.5]), 2);
        assert_eq!(tick_decimals(&[0.0, 20.0]), 0);
    }

    #[test]
    fn the_hydrostatics_dock_hands_down_no_verdict() {
        let h = measured();
        let curves = synthetic_cross_curves(&HEELS, &DISPS);
        // The curves would pass A.749: the old dock painted PASS here.
        let imo = seacraft_core::stability::evaluate_imo_criteria(&curves, 2).expect("criteria");
        assert!(imo.passed, "{imo:?}");
        let ctx = egui::Context::default();
        let mut options =
            HydroOptions::from_surface(&demo::demo_hull(false), SubdivisionOptions::default());
        let (mut kg, mut free_trim, mut lcg) = (Some(1.5), false, None);
        let mut text = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                hydrostatics(
                    ui,
                    HydroPanel {
                        options: &mut options,
                        kg: &mut kg,
                        kg_estimate: false,
                        limits: HydroLimits::default(),
                        free_trim: &mut free_trim,
                        lcg: &mut lcg,
                        result: Some(&h),
                        stale: false,
                        busy: false,
                        curves: Some(&curves),
                        curves_stale: false,
                    },
                );
            });
            out.textures_delta.clear();
            text = painted_text(&out.shapes);
        }
        let all: Vec<&str> = text.iter().map(|(t, _)| t.as_str()).collect();
        assert!(
            all.iter()
                .any(|t| t.contains("bare hull at dock KG 1.50 m")),
            "the cross curves say what they are: {all:?}"
        );
        assert!(all.iter().any(|t| t.contains("11.9 t")), "legend: {all:?}");
        for t in &all {
            assert!(
                !t.contains("PASS") && !t.contains("FAIL") && !t.contains("A.749"),
                "verdict on the hydro dock: {t}"
            );
        }
    }

    /// Every text painted, with its screen rectangle, colour and font.
    fn painted_texts(
        shapes: &[egui::epaint::ClippedShape],
    ) -> Vec<(String, egui::Rect, egui::Color32, egui::FontId)> {
        fn walk(
            shape: &egui::Shape,
            out: &mut Vec<(String, egui::Rect, egui::Color32, egui::FontId)>,
        ) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, out)),
                egui::Shape::Text(t) if !t.galley.text().is_empty() => {
                    let section = t.galley.job.sections.first();
                    out.push((
                        t.galley.text().to_string(),
                        t.galley.rect.translate(t.pos.to_vec2()),
                        section.map_or(t.fallback_color, |s| s.format.color),
                        section.map_or(egui::FontId::default(), |s| s.format.font_id.clone()),
                    ));
                }
                _ => {}
            }
        }
        let mut out = Vec::new();
        for clipped in shapes {
            walk(&clipped.shape, &mut out);
        }
        out
    }

    /// Run `f` in a headless frame of `width` points (twice, so layouts
    /// settle) and return what it painted.
    fn headless(
        width: f32,
        mut f: impl FnMut(&mut egui::Ui),
    ) -> Vec<(String, egui::Rect, egui::Color32, egui::FontId)> {
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let input = || egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(width, 2000.0),
            )),
            ..Default::default()
        };
        let mut texts = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(input(), |ui| f(ui));
            out.textures_delta.clear();
            texts = painted_texts(&out.shapes);
        }
        texts
    }

    #[test]
    fn the_readout_has_separate_lwl_and_bwl_rows() {
        let h = measured();
        let groups = hydrostatic_groups(&h, Some(2.2));
        let titles: Vec<&str> = groups.iter().map(|g| g.title).collect();
        assert_eq!(
            titles,
            [
                "Volume & form",
                "Centres",
                "Stability",
                "Dimensions & areas"
            ]
        );
        let rows: Vec<&Readout> = groups.iter().flat_map(|g| &g.rows).collect();
        let get = |name: &str| *rows.iter().find(|r| r.name == name).expect(name);
        assert_eq!(
            get("LWL"),
            &Readout::new("LWL", format!("{:.2}", h.lwl), "m")
        );
        assert_eq!(
            get("BWL"),
            &Readout::new("BWL", format!("{:.2}", h.bwl), "m")
        );
        for r in &rows {
            assert!(!r.name.contains('/'), "combined row {:?}", r.name);
            assert!(!r.value.contains('/'), "combined value {:?}", r.value);
            assert!(r.value.chars().count() <= 7, "{} = {:?}", r.name, r.value);
        }
        // The columns are the groups, two by two.
        let [left, right] = hydrostatic_rows(&h, Some(2.2));
        assert_eq!(left.len(), groups[0].rows.len() + groups[1].rows.len());
        assert_eq!(right.len(), groups[2].rows.len() + groups[3].rows.len());
        // Drawn: names, subheads and both values on screen.
        let texts = headless(360.0, |ui| grouped_readouts(ui, "lwl_bwl", &groups));
        for want in ["LWL", "BWL", "Volume & form", "Dimensions & areas"] {
            assert!(texts.iter().any(|t| t.0 == want), "{want} not drawn");
        }
        // Subheads are small and muted, never a quantity's look.
        let head = texts.iter().find(|t| t.0 == "Centres").unwrap();
        assert_eq!(head.2, theme::TEXT_MUTED);
        assert_eq!(head.3.size, theme::SMALL_SIZE);
        assert!(texts.iter().any(|t| t.0.trim() == format!("{:.2}", h.bwl)));
    }

    #[test]
    fn the_readout_goes_two_columns_from_a_340_px_dock() {
        assert!(!hydro_two_columns(300.0));
        assert!(hydro_two_columns(HYDRO_TWO_COLUMN_WIDTH));
        let h = measured();
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut options =
            HydroOptions::from_surface(&demo::demo_hull(false), SubdivisionOptions::default());
        let (mut kg, mut free_trim, mut lcg) = (Some(2.2), false, None);
        // Both widths drawn the way the dock draws them: a panel of that
        // width with a vertical scroll area inside.
        let mut layout = |width: f32| {
            let mut texts = Vec::new();
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(1600.0, 3000.0),
                    )),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    egui::Panel::right(format!("dock_{width}"))
                        .resizable(false)
                        .exact_size(width)
                        .show(ui, |ui| {
                            egui::ScrollArea::vertical().show(ui, |ui| {
                                hydrostatics(
                                    ui,
                                    HydroPanel {
                                        options: &mut options,
                                        kg: &mut kg,
                                        kg_estimate: false,
                                        limits: HydroLimits::default(),
                                        free_trim: &mut free_trim,
                                        lcg: &mut lcg,
                                        result: Some(&h),
                                        stale: false,
                                        busy: false,
                                        curves: None,
                                        curves_stale: false,
                                    },
                                );
                            });
                        });
                });
                out.textures_delta.clear();
                texts = painted_texts(&out.shapes);
            }
            let at = |name: &str| texts.iter().find(|t| t.0 == name).expect(name).1;
            // Everything drawn stays inside the dock: nothing pushes it wider.
            let left = texts
                .iter()
                .map(|t| t.1.left())
                .fold(f32::INFINITY, f32::min);
            let right = texts.iter().map(|t| t.1.right()).fold(0.0, f32::max);
            assert!(
                left >= 1600.0 - width && right <= 1600.0,
                "{width} px dock: text spans {left}..{right}: {:?}",
                texts
                    .iter()
                    .filter(|t| t.1.left() < 1600.0 - width || t.1.right() > 1600.0)
                    .map(|t| &t.0)
                    .collect::<Vec<_>>()
            );
            (at("volume"), at("KM(t)"), at("midship"))
        };
        // Narrow: one column, KM(t) under volume.
        let (volume, km, _) = layout(300.0);
        assert!((km.left() - volume.left()).abs() < 1.0 && km.top() > volume.top());
        // 340 px: KM(t) heads the right column, level with the first rows.
        let (volume, km, midship) = layout(340.0);
        assert!(km.left() > volume.right() + 40.0, "{volume:?} / {km:?}");
        assert!(km.top() - volume.top() < 30.0, "{volume:?} / {km:?}");
        assert!((midship.left() - km.left()).abs() < 1.0, "{midship:?}");
        // The default 360 px dock is two columns as well.
        let (volume, km, _) = layout(360.0);
        assert!(km.left() > volume.right() + 40.0, "{volume:?} / {km:?}");
    }

    #[test]
    fn a_long_legend_wraps_onto_more_rows_inside_the_plot() {
        // Pure layout: a row breaks before an entry would cross the width.
        let sizes = vec![egui::vec2(60.0, 13.0); 9];
        let rects = legend_layout(egui::pos2(0.0, 0.0), 200.0, &sizes);
        assert_eq!(rects.len(), 9);
        let rows = {
            let mut tops: Vec<i32> = rects.iter().map(|r| r.top() as i32).collect();
            tops.dedup();
            tops.len()
        };
        assert_eq!(rows, 3, "{rects:?}");
        assert!(rects.iter().all(|r| r.right() <= 200.0), "{rects:?}");
        // An entry wider than the plot takes a row of its own.
        let rects = legend_layout(
            egui::pos2(0.0, 0.0),
            100.0,
            &[egui::vec2(40.0, 13.0), egui::vec2(140.0, 13.0)],
        );
        assert_eq!(rects[1].left(), 0.0);
        assert!(rects[1].top() > rects[0].top());

        // Drawn: twelve displacements in a 300 px plot.
        let series: Vec<(String, Vec<(f64, f64)>)> = (0..12)
            .map(|i| {
                (
                    format!("{:.1} t", 8.0 + i as f64),
                    vec![(0.0, 0.0), (30.0, 0.1 * i as f64)],
                )
            })
            .collect();
        let rect_of = |n: usize| {
            let ctx = egui::Context::default();
            theme::apply(&ctx);
            let mut rect = egui::Rect::NOTHING;
            let mut texts = Vec::new();
            for _ in 0..2 {
                let input = egui::RawInput {
                    screen_rect: Some(egui::Rect::from_min_size(
                        egui::pos2(0.0, 0.0),
                        egui::vec2(316.0, 1000.0),
                    )),
                    ..Default::default()
                };
                let mut out = ctx.run_ui(input, |ui| {
                    rect = draw_plot(ui, "GZ", "GZ m at Δ", &series[..n], true, PLOT_HEIGHT);
                });
                out.textures_delta.clear();
                texts = painted_texts(&out.shapes);
            }
            (rect, texts)
        };
        let (two, _) = rect_of(2);
        let (twelve, texts) = rect_of(12);
        assert!(
            twelve.height() > two.height() + 10.0,
            "the legend grew rows: {} vs {}",
            twelve.height(),
            two.height()
        );
        let legend: Vec<_> = texts.iter().filter(|t| t.0.starts_with('━')).collect();
        assert_eq!(legend.len(), 12, "every entry drawn");
        let mut tops: Vec<i32> = legend.iter().map(|t| t.1.top() as i32).collect();
        tops.dedup();
        assert!(tops.len() >= 2, "one row only: {tops:?}");
        for t in &texts {
            assert!(
                t.1.right() <= twelve.right() + 0.5 && t.1.left() >= twelve.left() - 0.5,
                "{:?} at {:?} leaves the plot {twelve:?}",
                t.0,
                t.1
            );
            assert!(
                t.1.bottom() <= twelve.bottom() + 0.5,
                "{:?} below the plot",
                t.0
            );
        }
    }

    #[test]
    fn plot_tick_labels_never_collide_in_the_corners() {
        // GZ going negative (the y axis starts below zero) and a plain plot.
        let gz = vec![(
            "a".to_string(),
            vec![(0.0, 0.0), (20.0, 0.4), (50.0, 0.1), (80.0, -1.3)],
        )];
        let plain = vec![("".to_string(), vec![(0.0, 0.0), (12.0, 1.6)])];
        for (series, is_gz) in [(&gz, true), (&plain, false)] {
            let texts = headless(316.0, |ui| {
                draw_plot(ui, "", "", series, is_gz, PLOT_HEIGHT);
            });
            assert!(texts.len() >= 8, "{texts:?}");
            for (i, a) in texts.iter().enumerate() {
                for b in &texts[i + 1..] {
                    assert!(
                        !a.1.shrink(0.5).intersects(b.1.shrink(0.5)),
                        "{:?} at {:?} overlaps {:?} at {:?}",
                        a.0,
                        a.1,
                        b.0,
                        b.1
                    );
                }
            }
        }
    }

    #[test]
    fn eng_table_aligns_decimals_and_keeps_the_numbers() {
        let s = demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 2,
                mode: Default::default(),
            },
        );
        let cond: LoadingCondition = serde_json::from_value(serde_json::json!({
            "name": "tanks",
            "lightship": {"name": "Lightship", "mass_t": 8.0, "x": 5.0, "y": 0.0, "z": 1.2},
            "tanks": [
                {"name": "Fuel", "x_min": 3.0, "x_max": 4.0, "y_min": -0.5, "y_max": 0.5,
                 "z_min": 0.2, "z_max": 0.6, "fill": 0.5, "density_t_m3": 0.85},
                {"name": "Fresh water", "x_min": 6.0, "x_max": 7.5, "y_min": -0.6, "y_max": 0.6,
                 "z_min": 0.2, "z_max": 0.8, "fill": 0.9, "density_t_m3": 1.0},
                {"name": "Grey", "x_min": 8.0, "x_max": 8.5, "y_min": -0.3, "y_max": 0.3,
                 "z_min": 0.3, "z_max": 0.5, "fill": 0.25, "density_t_m3": 1.0},
            ],
        }))
        .expect("condition");
        let l = seacraft_core::loading::evaluate(&s, &opts, &cond);
        let rows = tank_table(&l);
        // The numbers are the ones the old grid printed, units apart.
        assert_eq!(rows.len(), l.tanks.len());
        for (row, t) in rows.iter().zip(&l.tanks) {
            assert_eq!(
                row,
                &vec![
                    EngCell::Text(t.name.clone()),
                    EngCell::Text(format!("{:.0}", t.fill * 100.0)),
                    EngCell::Text(format!("{:.3}", t.mass_t)),
                    EngCell::Text(format!("{:.2}", t.fsm_t_m)),
                ]
            );
        }
        let columns = [
            EngColumn::text("Tank"),
            EngColumn::number("Fill", "%"),
            EngColumn::number("Mass", "t"),
            EngColumn::number("FSM", "t·m"),
        ];
        let mut table = None;
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let mut shapes = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                table = Some(eng_table(ui, &columns, &rows));
            });
            out.textures_delta.clear();
            shapes = out.shapes;
        }
        let table = table.unwrap();
        let texts = painted_texts(&shapes);
        assert_eq!(table.cells.len(), rows.len() + 1, "header + rows");
        for (r, row) in rows.iter().enumerate() {
            for (c, cell) in row.iter().enumerate() {
                let EngCell::Text(want) = cell else {
                    unreachable!()
                };
                let drawn = texts
                    .iter()
                    .find(|t| &t.0 == want && (t.1.min - table.cells[r + 1][c].min).length() < 0.5)
                    .unwrap_or_else(|| panic!("{want:?} not drawn in its cell"));
                if c == 0 {
                    // Names: proportional, left edges on one line.
                    assert_eq!(drawn.3.family, egui::FontFamily::Proportional);
                    assert!((drawn.1.left() - table.cells[1][0].left()).abs() < 0.5);
                } else {
                    // Values: monospace, right edges on one line.
                    assert_eq!(drawn.3, egui::FontId::monospace(theme::MONO_SIZE));
                    assert!(
                        (drawn.1.right() - table.cells[1][c].right()).abs() < 0.5,
                        "{want:?} ends at {} not {}",
                        drawn.1.right(),
                        table.cells[1][c].right()
                    );
                }
            }
        }
        // Units sit in the header, muted, never on a value.
        assert!(texts
            .iter()
            .any(|t| t.0.contains("Mass") && t.0.contains("(t)")));
        for t in &texts {
            if t.3.family == egui::FontFamily::Monospace {
                assert!(t.0.parse::<f64>().is_ok(), "{:?} is not a bare number", t.0);
            }
        }
        // Zebra stripes from the theme on every other row.
        fn stripes(shapes: &[egui::epaint::ClippedShape]) -> usize {
            fn walk(shape: &egui::Shape, n: &mut usize) {
                match shape {
                    egui::Shape::Vec(v) => v.iter().for_each(|s| walk(s, n)),
                    egui::Shape::Rect(r) if r.fill == theme::TABLE_STRIPE => *n += 1,
                    _ => {}
                }
            }
            let mut n = 0;
            shapes.iter().for_each(|c| walk(&c.shape, &mut n));
            n
        }
        assert_eq!(stripes(&shapes), rows.len() / 2);
        // Labels with their unit in brackets split cleanly.
        assert_eq!(split_unit("R_T (kN)"), ("R_T", "kN"));
        assert_eq!(split_unit("Fn"), ("Fn", ""));
    }

    #[test]
    fn the_resistance_table_prints_what_the_grid_printed() {
        let res = crate::app::ResistanceResult {
            method: "Holtrop".into(),
            rows: vec![
                crate::app::ResistanceRow {
                    speed_kn: 6.0,
                    fn_: 0.2846,
                    rt_kn: 1.23456,
                    pe_kw: Some(3.8),
                    note: None,
                },
                crate::app::ResistanceRow {
                    speed_kn: 7.5,
                    fn_: 0.3557,
                    rt_kn: 2.5,
                    pe_kw: None,
                    note: None,
                },
            ],
            warnings: Vec::new(),
            rt_label: "R_T (kN)".into(),
        };
        let rows = resistance_table(&res);
        let text = |r: usize, c: usize| match &rows[r][c] {
            EngCell::Text(t) => t.clone(),
            EngCell::Badge(..) => unreachable!(),
        };
        assert_eq!(
            [text(0, 0), text(0, 1), text(0, 2), text(0, 3)],
            ["6.00", "0.285", "1.23", "3.80"]
        );
        assert_eq!(text(1, 3), "—");
    }

    #[test]
    fn a_fail_badge_is_the_same_red_everywhere() {
        assert_eq!(criteria_badge_style(false, false), ("FAIL", theme::FAIL));
        assert_eq!(criteria_badge_style(true, false), ("PASS", theme::OK));
        assert_eq!(
            criteria_badge_style(false, true),
            ("FAIL · stale", theme::STALE)
        );
        assert_eq!(
            criteria_badge_style(true, true),
            ("PASS · stale", theme::STALE)
        );
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            criteria_badge(ui, false, false);
            criteria_badge(ui, true, true);
        });
        out.textures_delta.clear();
        let text = painted_text(&out.shapes);
        assert!(
            text.contains(&("FAIL".to_string(), theme::FAIL)),
            "{text:?}"
        );
        assert!(
            text.contains(&("PASS · stale".to_string(), theme::STALE)),
            "{text:?}"
        );
        // The helper is the only place a verdict is spelled out.
        let src = include_str!("panels.rs");
        let body = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(body.matches("\"FAIL").count(), 2, "FAIL outside the badge");
        assert_eq!(body.matches("\"PASS").count(), 2, "PASS outside the badge");
    }

    /// R8-3 (A08 follow-up): a loading result without an equilibrium says
    /// so in the warning colour, with the reason; one with an equilibrium
    /// carries no such line.
    #[test]
    fn the_loading_dock_warns_when_no_equilibrium_was_found() {
        use seacraft_core::subdiv::{SubdivisionMode, SubdivisionOptions};
        let (surface, mut project) =
            seacraft_core::examples::build(seacraft_core::examples::Example::Runabout6m).unwrap();
        let opts = seacraft_core::hydro::HydroOptions::from_surface(
            &surface,
            SubdivisionOptions {
                level: 1,
                mode: SubdivisionMode::Loop,
            },
        );
        let fine =
            seacraft_core::loading::evaluate(&surface, &opts, &project.loading_conditions[0]);
        let mut heavy = project.loading_conditions[0].clone();
        heavy.lightship.mass_t = 1000.0;
        let sunk = seacraft_core::loading::evaluate(&surface, &opts, &heavy);
        assert!(fine.equilibrium_found && !sunk.equilibrium_found);

        let ctx = egui::Context::default();
        let mut draw = |result: &LoadingResult| {
            let input = egui::RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(900.0, 6000.0),
                )),
                ..Default::default()
            };
            let mut selected = 0;
            let mut out = ctx.run_ui(input, |ui| {
                loading(
                    ui,
                    LoadingPanel {
                        project: &mut project,
                        selected: &mut selected,
                        result: Some(result),
                        stale: false,
                        busy: false,
                        highlight: None,
                        scroll_to: false,
                        lightship: WeightItem {
                            name: "Lightship".into(),
                            mass_t: 10.0,
                            x: 0.0,
                            y: 0.0,
                            z: 0.0,
                        },
                    },
                );
            });
            out.textures_delta.clear();
            painted_text(&out.shapes)
        };
        let note = format!("⚠ {}", seacraft_core::loading::NO_EQUILIBRIUM_NOTE);
        let text = draw(&sunk);
        assert!(text.contains(&(note.clone(), theme::WARN)), "{text:?}");
        assert!(
            text.iter()
                .any(|(t, c)| t.contains("cannot float") && *c == theme::WARN),
            "{text:?}"
        );
        let text = draw(&fine);
        assert!(!text.iter().any(|(t, _)| *t == note), "{text:?}");
    }

    /// R8-3 (A04 follow-up): the windage totals refer a profiled item's
    /// model-`z` centroid to the keel, like the weather criterion, so moving
    /// the model (and the items drawn on it) does not move them.
    #[test]
    fn windage_totals_are_above_the_keel_wherever_it_sits() {
        let items = |dz: f64| {
            vec![
                WindageItem::from_profile(
                    "Deckhouse",
                    WindageRect {
                        x_min: 3.0,
                        x_max: 7.0,
                        z_min: 3.0 + dz,
                        z_max: 5.0 + dz,
                    },
                ),
                WindageItem {
                    name: "Mast".into(),
                    area_m2: 1.0,
                    z_centroid: 8.0,
                    profile: None,
                },
            ]
        };
        let want = (9.0, (8.0 * 4.0 + 8.0) / 9.0);
        for dz in [0.0, 1.5, -2.0] {
            let (a, z) = windage_totals(&items(dz), dz);
            assert!((a - want.0).abs() < 1e-12, "dz {dz}: A {a}");
            assert!((z - want.1).abs() < 1e-12, "dz {dz}: z {z} vs {}", want.1);
        }
        // The panel prints that figure.
        let ctx = egui::Context::default();
        let mut project = Project {
            windage: items(1.5),
            ..Default::default()
        };
        let mut selected = None;
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(900.0, 3000.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            windage(
                ui,
                WindagePanel {
                    project: &mut project,
                    weather: None,
                    stale: false,
                    selected: &mut selected,
                    keel_z: 1.5,
                },
            );
        });
        out.textures_delta.clear();
        let text = painted_text(&out.shapes);
        let shown = format!("{:.2} m", want.1);
        assert!(text.iter().any(|(t, _)| *t == shown), "{shown} in {text:?}");
    }

    #[test]
    fn signed_zero_formatting_suppresses_negative_and_positive_zero() {
        assert_eq!(format_signed(0.0, 3), "0.000");
        assert_eq!(format_signed(-0.0, 3), "0.000");
        assert_eq!(format_signed(-0.00001, 3), "0.000");
        assert_eq!(format_signed(0.00001, 3), "0.000");
        assert_eq!(format_signed(-0.00049, 3), "0.000");
        assert_eq!(format_signed(0.00049, 3), "0.000");
        assert_eq!(format_signed(0.001, 3), "+0.001");
        assert_eq!(format_signed(-0.001, 3), "-0.001");
        assert_eq!(format_signed(1.234, 3), "+1.234");
        assert_eq!(format_signed(-1.234, 3), "-1.234");
        assert_eq!(format_signed(-0.02, 1), "0.0");
        assert_eq!(format_signed(0.02, 1), "0.0");
        assert_eq!(format_signed(1.5, 1), "+1.5");
        assert_eq!(format_signed(-1.5, 1), "-1.5");
    }

    #[test]
    fn free_trim_on_container_230m_keeps_lcg_at_its_lcb_without_clamp() {
        let (surface, _) =
            seacraft_core::examples::build(seacraft_core::examples::Example::Container230m)
                .unwrap();
        let mut options = seacraft_core::hydro::HydroOptions::from_surface(
            &surface,
            seacraft_core::subdiv::SubdivisionOptions {
                level: 1,
                mode: seacraft_core::subdiv::SubdivisionMode::Loop,
            },
        );
        let h = seacraft_core::hydro::compute(&surface, &options);
        assert!(h.lcb.x > 100.0, "LCB {} must be > 100 m", h.lcb.x);
        let (mut kg, mut free_trim, mut lcg) = (Some(15.0), true, Some(h.lcb.x));
        let ctx = egui::Context::default();
        theme::apply(&ctx);
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(360.0, 2000.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| {
            hydrostatics(
                ui,
                HydroPanel {
                    options: &mut options,
                    kg: &mut kg,
                    kg_estimate: false,
                    limits: HydroLimits::default(),
                    free_trim: &mut free_trim,
                    lcg: &mut lcg,
                    result: Some(&h),
                    stale: false,
                    busy: false,
                    curves: None,
                    curves_stale: false,
                },
            );
        });
        out.textures_delta.clear();
        let texts = painted_texts(&out.shapes);
        let lcg_fmt = format!("{:.3}", h.lcb.x);
        assert!(
            texts.iter().any(|t| t.0.contains(&lcg_fmt)),
            "LCB value {lcg_fmt} should be visible in texts: {:?}",
            texts.iter().map(|t| &t.0).collect::<Vec<_>>()
        );
        assert_eq!(lcg, Some(h.lcb.x), "LCG was clamped from {}", h.lcb.x);
    }

    #[test]
    fn plot_axes_fit_does_not_waste_width_on_negative_transom() {
        let rect = egui::Rect::from_min_size(egui::pos2(0.0, 0.0), egui::vec2(300.0, 150.0));
        // Container-like SAC: stations from -3.0 to 230.0
        let series = vec![(
            "SAC".to_string(),
            vec![(-3.0, 0.0), (100.0, 340.0), (230.0, 0.0)],
        )];
        let axes = PlotAxes::fit(rect, &series, false);
        let (x0, x1) = axes.x_range();
        assert!(
            x0 > -50.0,
            "axis started too far negative at {x0} (wanted near -3, not -100)"
        );
        assert!(
            !axes.x_ticks.contains(&-100.0),
            "ticks contained -100: {:?}",
            axes.x_ticks
        );
        assert!(
            axes.x_ticks.contains(&0.0),
            "ticks should include 0: {:?}",
            axes.x_ticks
        );
        // Wasted width must be <= 10 %:
        let data_width = 230.0 - (-3.0);
        let axis_width = x1 - x0;
        let wasted_ratio = (axis_width - data_width) / axis_width;
        assert!(
            wasted_ratio <= 0.10,
            "wasted width ratio {wasted_ratio:.3} exceeds 10% (axis {x0}..{x1}, data -3..230)"
        );
    }
}

// ---- builder templates ----------------------------------------------------

/// Where the Builder templates preview stands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum TemplatesStatus<'a> {
    /// The patterns for the current options and model are being cut.
    Working,
    /// The PDF for the current options and model is ready.
    Ready {
        pages: usize,
        sheets: usize,
        warnings: &'a [String],
    },
    /// The current options or model give no patterns; the reason.
    Failed(&'a str),
}

/// What the Builder templates window asks the app to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TemplatesAction {
    /// Pick a file and write the patterns.
    Export,
}

/// Shown in place of the page count while the station list on screen does not read.
pub const STATION_LIST_ERROR: &str = "Not a list of positions: write x, x, … in metres";

/// Station positions typed as `x, x, …` in metres; `None` unless every entry is a finite number
/// and there is at least one.
fn station_list(text: &str) -> Option<Vec<f64>> {
    let parsed: Option<Vec<f64>> = text
        .split(',')
        .map(str::trim)
        .filter(|t| !t.is_empty())
        .map(|t| t.parse::<f64>().ok().filter(|v| v.is_finite()))
        .collect();
    parsed.filter(|v| !v.is_empty())
}

/// The scales the window offers, paper over real size.
const TEMPLATE_SCALES: [f64; 5] = [1.0, 0.5, 0.2, 0.1, 0.05];

/// The Builder templates options, the preview's status and Export. Edits `params` in place;
/// the app compares it with what it had.
pub fn templates_options(
    ui: &mut egui::Ui,
    params: &mut TemplatesParams,
    status: &TemplatesStatus<'_>,
) -> Option<TemplatesAction> {
    use seacraft_core::templates::{StationChoice, Transom};
    // The list as typed, kept while it is being edited; forgotten in the other modes.
    let list_id = egui::Id::unique("templates_at_text");
    // Whether the list on screen reads as the stations the preview and Export use.
    let mut list_reads = true;
    egui::Grid::new("templates_grid")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            ui.label("Stations:");
            ui.horizontal(|ui| {
                let (count, spacing, list) = match &params.stations {
                    StationChoice::Count(_) => (true, false, false),
                    StationChoice::Spacing(_) => (false, true, false),
                    StationChoice::At(_) => (false, false, true),
                };
                if ui.radio(count, "Count").clicked() && !count {
                    params.stations = StationChoice::Count(11);
                }
                if ui.radio(spacing, "Spacing").clicked() && !spacing {
                    params.stations = StationChoice::Spacing(0.5);
                }
                if ui.radio(list, "At x").clicked() && !list {
                    params.stations = StationChoice::At(vec![1.0]);
                }
            });
            ui.end_row();
            ui.label("");
            match &mut params.stations {
                StationChoice::Count(n) => {
                    ui.data_mut(|d| d.remove::<String>(list_id));
                    ui.add(
                        egui::DragValue::new(n).range(2..=seacraft_core::templates::MAX_STATIONS),
                    );
                }
                StationChoice::Spacing(s) => {
                    ui.data_mut(|d| d.remove::<String>(list_id));
                    ui.add(
                        egui::DragValue::new(s)
                            .speed(0.01)
                            .range(0.05..=10.0)
                            .suffix(" m"),
                    );
                }
                StationChoice::At(xs) => {
                    let shown = xs
                        .iter()
                        .map(|x| format!("{x}"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    let mut text: String = ui.data_mut(|d| d.get_temp(list_id)).unwrap_or(shown);
                    let response = ui
                        .add(egui::TextEdit::singleline(&mut text).hint_text("x, x, … in metres"));
                    match station_list(&text) {
                        Some(parsed) if response.changed() => *xs = parsed,
                        Some(_) => {}
                        None => list_reads = false,
                    }
                    ui.data_mut(|d| d.insert_temp(list_id, text));
                }
            }
            ui.end_row();

            ui.label("Skin to deduct:");
            ui.add(
                egui::DragValue::new(&mut params.skin_mm)
                    .speed(0.1)
                    .range(0.0..=seacraft_core::templates::MAX_SKIN_M * 1000.0)
                    .suffix(" mm"),
            );
            ui.end_row();

            ui.label("Half moulds:");
            ui.checkbox(&mut params.half, "Starboard half only");
            ui.end_row();

            ui.label("Strongback:");
            ui.horizontal(|ui| {
                let mut on = params.strongback_z.is_some();
                if ui.checkbox(&mut on, "Legs up to z").changed() {
                    params.strongback_z = on.then_some(2.0);
                }
                if let Some(z) = &mut params.strongback_z {
                    ui.add(egui::DragValue::new(z).speed(0.01).suffix(" m"));
                }
            });
            ui.end_row();

            ui.label("Profile:");
            ui.checkbox(&mut params.profile, "Backbone pattern");
            ui.end_row();

            ui.label("Transom:");
            ui.horizontal(|ui| {
                let mut on = params.transom.is_some();
                if ui.checkbox(&mut on, "At x").changed() {
                    params.transom = on.then_some(Transom {
                        x: 0.0,
                        rake_deg: 0.0,
                    });
                }
                if let Some(t) = &mut params.transom {
                    ui.add(egui::DragValue::new(&mut t.x).speed(0.01).suffix(" m"));
                    ui.label("rake");
                    let max = seacraft_core::templates::MAX_TRANSOM_RAKE_DEG;
                    ui.add(
                        egui::DragValue::new(&mut t.rake_deg)
                            .speed(0.5)
                            .range(-max..=max)
                            .suffix("°"),
                    );
                }
            });
            ui.end_row();

            ui.label("Paper:");
            egui::ComboBox::from_id_salt("templates_paper")
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

            ui.label("Single page:");
            ui.checkbox(&mut params.single_page, "One page a pattern (plotter)");
            ui.end_row();

            ui.label("Margin:");
            ui.add(
                egui::DragValue::new(&mut params.margin_mm)
                    .speed(0.5)
                    .range(0.0..=50.0)
                    .suffix(" mm"),
            );
            ui.end_row();

            ui.label("Overlap:");
            ui.add(
                egui::DragValue::new(&mut params.overlap_mm)
                    .speed(0.5)
                    .range(0.0..=50.0)
                    .suffix(" mm"),
            );
            ui.end_row();

            ui.label("Scale:");
            egui::ComboBox::from_id_salt("templates_scale")
                .selected_text(seacraft_core::units::scale_label(params.scale))
                .show_ui(ui, |ui| {
                    for scale in TEMPLATE_SCALES {
                        ui.selectable_value(
                            &mut params.scale,
                            scale,
                            seacraft_core::units::scale_label(scale),
                        );
                    }
                });
            ui.end_row();

            ui.label("Imperial:");
            ui.checkbox(&mut params.imperial, "Feet and inches beside metres");
            ui.end_row();
        });

    ui.separator();
    if !list_reads {
        // The preview and Export would use the last list that read, not the one on screen.
        ui.colored_label(theme::FAIL, STATION_LIST_ERROR);
        ui.separator();
        ui.add_enabled(false, egui::Button::new("Export…"));
        return None;
    }
    match status {
        TemplatesStatus::Working => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label("Working…");
            });
        }
        TemplatesStatus::Ready {
            pages,
            sheets,
            warnings,
        } => {
            ui.label(format!("{pages} pages, {sheets} patterns"));
            if !warnings.is_empty() {
                egui::ScrollArea::vertical()
                    .max_height(120.0)
                    .show(ui, |ui| {
                        for warning in warnings.iter() {
                            ui.colored_label(theme::WARN, warning);
                        }
                    });
            }
        }
        TemplatesStatus::Failed(reason) => {
            ui.colored_label(theme::FAIL, *reason);
        }
    }
    ui.separator();
    let ready = matches!(status, TemplatesStatus::Ready { .. });
    ui.add_enabled(ready, egui::Button::new("Export…"))
        .clicked()
        .then_some(TemplatesAction::Export)
}
