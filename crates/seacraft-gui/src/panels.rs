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

use crate::render::DisplayOptions;
use crate::theme;

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
        ui.colored_label(theme::WARN, "no visible layers");
    }
    changed
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
    ui.add_space(4.0);
    ui.label(
        egui::RichText::new("INSPECTOR")
            .font(theme::caption_font())
            .color(theme::TEXT_SECONDARY),
    );
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

/// The pinned live strip's two rows: displacement, draft and trim; GM
/// and LCB. A dash stands for a quantity not measured yet.
pub fn live_strip_rows(h: Option<&Hydrostatics>, kg: f64) -> [Vec<Readout>; 2] {
    let r = Readout::new;
    match h {
        Some(h) => [
            vec![
                r("Δ", format!("{:.2}", h.displacement_t), "t"),
                r("T", format!("{:.3}", h.draft), "m"),
                r("trim", format!("{:+.3}", h.trim_m), "m"),
            ],
            vec![
                r("GM", format!("{:+.3}", h.gm(kg)), "m"),
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

/// The Analysis dock's pinned strip: the live floating condition in two
/// compact rows, greyed while the hydrostatics are being recomputed, and
/// the *⤢ wide* toggle. Returns true when the toggle was clicked.
pub fn live_strip(
    ui: &mut egui::Ui,
    h: Option<&Hydrostatics>,
    kg: f64,
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
                let label = if wide { "⤡ narrow" } else { "⤢ wide" };
                toggle = ui
                    .selectable_label(wide, label)
                    .on_hover_text("Widen the dock for plots and tables")
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
            "{} {:.*}{unit} ({:.*} {:+.1} %)",
            self.name,
            self.decimals,
            self.actual,
            self.decimals,
            self.target,
            self.deviation_pct()
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
        ("LCB", req.lcb_frac.map(|f| 100.0 * f), lcb_pct, 1, "%"),
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
            push(format!(" {:+.1} %", t.deviation_pct()), deviation);
            push(")".to_owned(), theme::TARGET);
            ui.label(job).on_hover_text(format!(
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

/// A criteria verdict: `PASS` green, `FAIL` red, `… · stale` grey when the
/// result predates the last edit. The only place a verdict is drawn.
pub fn criteria_badge(ui: &mut egui::Ui, passed: bool, stale: bool) -> egui::Response {
    let (text, colour) = criteria_badge_style(passed, stale);
    ui.label(egui::RichText::new(text).monospace().strong().color(colour))
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
        .show(ui, |ui| {
            for r in rows {
                ui.label(egui::RichText::new(r.name).color(theme::TEXT_SECONDARY));
                ui.label(
                    egui::RichText::new(format!("{:>width$}", r.value))
                        .monospace()
                        .color(theme::TEXT_PRIMARY),
                );
                ui.horizontal(|ui| {
                    ui.set_min_width(theme::UNIT_WIDTH);
                    ui.label(egui::RichText::new(r.unit).color(theme::TEXT_MUTED));
                });
                ui.end_row();
            }
        });
}

/// The measurement rows the dock draws, in two groups (the form and the
/// stability quantities).
///
/// Kept separate from the drawing so the numbers (and their formatting) can be
/// tested without a frame: `TPC` and `MTC` are table quantities computed here
/// from the measured waterplane, with the kilogram-to-tonne factor the core's
/// kg/m³ density needs.
pub fn hydrostatic_rows(h: &Hydrostatics, kg: f64) -> [Vec<Readout>; 2] {
    let rho_t = h.water_density / 1000.0;
    let tpc = h.waterplane_area_m2 * rho_t / 100.0;
    let mtc = if h.lwl > 1e-9 {
        rho_t * h.waterplane_inertia_longitudinal / (100.0 * h.lwl)
    } else {
        0.0
    };
    let r = Readout::new;
    [
        vec![
            r("volume", format!("{:.3}", h.volume_m3), "m³"),
            r("displacement", format!("{:.2}", h.displacement_t), "t"),
            r("Cb", format!("{:.3}", h.block_coefficient), ""),
            r("Cp", format!("{:.3}", h.prismatic_coefficient), ""),
            r("Cm", format!("{:.3}", h.midship_coefficient), ""),
            r("Cwp", format!("{:.3}", h.waterplane_coefficient), ""),
            r("LCB", format!("{:.3}", h.lcb.x), "m"),
            r("LCF", format!("{:.3}", h.lcf.x), "m"),
            r("TPC", format!("{:.3}", tpc), "t/cm"),
        ],
        vec![
            r("KB", format!("{:.3}", h.kb), "m"),
            r("BM(t)", format!("{:.3}", h.bm_transverse), "m"),
            r("KM(t)", format!("{:.3}", h.km_transverse), "m"),
            r("GM", format!("{:+.3}", h.gm(kg)), "m"),
            r("wetted", format!("{:.2}", h.wetted_surface_m2), "m²"),
            r("waterplane", format!("{:.2}", h.waterplane_area_m2), "m²"),
            r("LWL / BWL", format!("{:.2} / {:.2}", h.lwl, h.bwl), "m"),
            r("midship", format!("{:.3}", h.mainframe_area_m2), "m²"),
            r("MTC", format!("{:.3}", mtc), "t·m/cm"),
        ],
    ]
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
        r("Damaged heel", format!("{:+.1}", d.heel_deg), "°"),
        r("Net buoyant vol.", format!("{:.3}", d.net_volume_m3), "m³"),
        r("KG", format!("{:.3}", d.kg), "m"),
        r("Intact trim", format!("{:+.3}", d.intact_trim_m), "m"),
        r("Damaged trim", format!("{:+.3}", d.trim_m), "m"),
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
        r("TCG", format!("{:+.3}", l.tcg), "m"),
        r("KG (solid)", format!("{:.3}", l.kg_solid), "m"),
        r("FSM", format!("{:.2}", l.fsm_t_m), "t·m"),
        r("FSC", format!("{:.3}", l.fsc_m), "m"),
        r("KG (fluid)", format!("{:.3}", l.kg_fluid), "m"),
        r("Draft", format!("{:.3}", l.draft), "m"),
        r("Trim", format!("{:+.3}", l.trim_m), "m"),
        r("List", format!("{:+.1}", l.list_deg), "°"),
        r("GM (solid)", format!("{:+.3}", l.gm_solid), "m"),
        r("GM (fluid)", format!("{:+.3}", l.gm_fluid), "m"),
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

/// Where the SAC plot draws inside its allocated rectangle: the caption band
/// on top and the curve area below it, never overlapping.
pub fn sac_plot_rects(rect: egui::Rect) -> (egui::Rect, egui::Rect) {
    let (caption, body) = split_caption(rect);
    (caption, body.shrink2(egui::vec2(2.0, 4.0)))
}

/// The sectional area curve, drawn small under the readout.
fn plot_sac(ui: &mut egui::Ui, h: &Hydrostatics) {
    let height = 96.0 + PLOT_CAPTION_BAND;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let (caption, rect) = sac_plot_rects(response.rect);
    painter.rect_filled(rect, 2.0, theme::PLOT_BG);
    let points = sac_polyline(&h.sac, rect);
    if points.len() < 2 {
        return;
    }
    painter.line_segment(
        [rect.left_bottom(), rect.right_bottom()],
        egui::Stroke::new(1.0, theme::PLOT_AXIS),
    );
    painter.add(egui::Shape::line(
        points,
        egui::Stroke::new(1.4, theme::PLOT_SERIES[0]),
    ));
    let a_max = h.sac.iter().fold(0.0_f64, |m, s| m.max(s.area));
    plot_caption(
        &painter,
        caption,
        &format!(
            "SAC · A max {a_max:.2} m² · x {:.2}..{:.2} m",
            h.model_min.x, h.model_max.x
        ),
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
            // One grid for both groups, so every value shares one column.
            let rows = hydrostatic_rows(h, *panel.kg).concat();
            readouts(ui, "hydro_readouts", &rows);
            ui.add_space(2.0);
            plot_sac(ui, h);
            for warning in &h.warnings {
                ui.colored_label(theme::FAIL, &warning.message);
            }
            if h.gm(*panel.kg) < 0.0 {
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

    if let Some(curves) = panel.curves {
        // No criteria verdict here: these curves are the bare hull at the
        // dock KG over a synthetic ±30 % displacement range, not a condition
        // anyone defined. The per-criterion table lives in Loading.
        ui.separator();
        plot_cross_curves(ui, curves, *panel.kg);
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

    ui.heading("General Hydrostatics");
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
            let mut speeds_text =
                ui.data(|d| d.get_temp::<String>(buffer_id))
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
            if response.has_focus() {
                ui.data_mut(|d| d.insert_temp(buffer_id, speeds_text.clone()));
            } else {
                ui.data_mut(|d| d.remove::<String>(buffer_id));
            }
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
            ui.end_row();
        });

    ui.separator();
    ui.heading("Resistance & Powering");
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
    ui.heading("Loading Conditions");
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
    ui.heading("Damage Cases");
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
            ui.colored_label(theme::WARN, "Results out of date");
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
            ui.colored_label(theme::WARN, format!("⚠ {w}"));
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
            ui.colored_label(theme::FAIL, format!("⚠ {problem}"));
        }
    }

    let stale = state.raos_stale;
    if let Some(r) = &state.sea_result {
        ui.add_space(4.0);
        if stale {
            ui.colored_label(theme::WARN, "Sea-state results out of date");
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
            ui.colored_label(theme::WARN, format!("⚠ {}", w.message));
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
            theme::FAIL,
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
            ui.colored_label(theme::WARN, "stale — press Compute");
        }
    });

    // Results
    if let Some(r) = panel.result {
        ui.separator();
        ui.heading("Damaged Equilibrium");
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
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Residual Stability Criteria:").strong());
                criteria_badge(ui, crit.passed, panel.stale);
            });
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
                        ui.label(name);
                        ui.label(val);
                        criteria_badge(ui, ok, panel.stale);
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
                ui.colored_label(theme::WARN, format!("Warning: {w}"));
            }
        }
    }

    action
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
/// ticks the data was fitted to (the first and last tick are the axis ends).
/// Pure, so the layout can be tested without a frame.
#[derive(Debug, Clone)]
pub struct PlotAxes {
    pub caption: egui::Rect,
    pub area: egui::Rect,
    pub x_ticks: Vec<f64>,
    pub y_ticks: Vec<f64>,
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
        Self {
            caption,
            area,
            x_ticks: nice_ticks(x_min, x_max),
            y_ticks: nice_ticks(y_min, y_max),
            gz,
        }
    }

    fn x_range(&self) -> (f64, f64) {
        (
            self.x_ticks.first().copied().unwrap_or(0.0),
            self.x_ticks.last().copied().unwrap_or(1.0),
        )
    }

    fn y_range(&self) -> (f64, f64) {
        (
            self.y_ticks.first().copied().unwrap_or(0.0),
            self.y_ticks.last().copied().unwrap_or(1.0),
        )
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
            painter.text(
                egui::pos2(sx, area.bottom() + 2.0),
                egui::Align2::CENTER_TOP,
                tick_label(x, x_dec, x_suffix),
                font.clone(),
                theme::PLOT_TEXT,
            );
        }
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
            painter.text(
                egui::pos2(area.left() - 4.0, sy),
                egui::Align2::RIGHT_CENTER,
                tick_label(y, y_dec, ""),
                font.clone(),
                theme::PLOT_TEXT,
            );
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
            // Label it in the tick column unless a tick label is already there.
            let clear = self
                .y_ticks
                .iter()
                .all(|&t| (self.to_screen(x0, t).y - sy).abs() > PLOT_FONT_SIZE);
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

/// Draw a captioned x/y plot with ticks, and its legend underneath (outside
/// the curves) when there is more than one labelled series.
fn draw_plot(
    ui: &mut egui::Ui,
    caption: &str,
    legend_title: &str,
    series: &[(String, Vec<(f64, f64)>)],
    gz: bool,
) {
    let height = 172.0;
    let (response, painter) = ui.allocate_painter(
        egui::vec2(ui.available_width(), height),
        egui::Sense::hover(),
    );
    let axes = PlotAxes::fit(response.rect, series, gz);
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

    let labelled = series.iter().filter(|(l, _)| !l.is_empty()).count();
    if series.len() > 1 && labelled > 0 {
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            if !legend_title.is_empty() {
                ui.label(
                    egui::RichText::new(legend_title)
                        .monospace()
                        .size(PLOT_FONT_SIZE)
                        .color(theme::PLOT_TEXT),
                );
            }
            for (i, (label, _)) in series.iter().enumerate() {
                if label.is_empty() {
                    continue;
                }
                ui.label(
                    egui::RichText::new(format!("━ {label}"))
                        .monospace()
                        .size(PLOT_FONT_SIZE)
                        .color(theme::PLOT_SERIES[i % theme::PLOT_SERIES.len()]),
                );
            }
        });
    }
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
    draw_plot(
        ui,
        &format!("Cross curves — bare hull at dock KG {kg:.2} m"),
        "GZ m at Δ",
        &cross_curve_polylines(curves),
        true,
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
    draw_plot(ui, &caption, "", series, is_gz);
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
            ui.colored_label(theme::WARN, "stale — press Compute");
        }
    });

    // Results
    if let Some(r) = panel.result {
        ui.separator();
        ui.heading("Equilibrium & Hydrostatics");
        readouts(ui, "loading_readouts", &loading_rows(r));

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
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("IMO A.749 Criteria:").strong());
                criteria_badge(ui, imo.passed, panel.stale);
            });
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
                        ui.label(name);
                        ui.label(val);
                        criteria_badge(ui, ok, panel.stale);
                        ui.end_row();
                    }
                });
        }

        if let Some(w) = &r.weather {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Weather Criterion (§2.3):").strong());
                criteria_badge(ui, w.passed, panel.stale);
            });
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
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Weather Criterion Summary (§2.3):").strong());
            criteria_badge(ui, w.passed, panel.stale);
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
    /// `waterplane`, `combs`, `wireframe`, `level`, `level:N` (while the
    /// list is open), `mode`, `fit`, `maximise`.
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
                    "combs",
                    "Combs",
                    "Stations, buttocks and waterlines with curvature combs",
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
            .button("Fit")
            .on_hover_text(format!("Frame the model in every view ({fit_key})"));
        out.fit = fit.clicked();
        out.rects.push(("fit", fit.rect));
        // A framed button like Fit (an action, not a display toggle),
        // highlighted while a view is maximised.
        let max = ui
            .add(
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
        let [left, right] = hydrostatic_rows(&h, 1.5);
        assert_eq!(left.len(), right.len(), "the two groups stay balanced");
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
        let (tpc, unit) = find(&left, "TPC");
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
            ("KB", format!("{:.3} m", h.kb)),
            ("BM(t)", format!("{:.3} m", h.bm_transverse)),
            ("KM(t)", format!("{:.3} m", h.km_transverse)),
            ("GM", format!("{:+.3} m", h.gm(kg))),
            ("wetted", format!("{:.2} m²", h.wetted_surface_m2)),
            ("waterplane", format!("{:.2} m²", h.waterplane_area_m2)),
            ("LWL / BWL", format!("{:.2} / {:.2} m", h.lwl, h.bwl)),
            ("midship", format!("{:.3} m²", h.mainframe_area_m2)),
            ("MTC", format!("{:.3} t·m/cm", mtc)),
        ]
    }

    #[test]
    fn readouts_split_value_and_unit_without_changing_a_number() {
        let h = measured();
        let rows = hydrostatic_rows(&h, 2.2).concat();
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
        // The demo reference numbers are the ones on show (AGENTS.md).
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
        let rows = hydrostatic_rows(&h, 2.2).concat();
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
        let (caption, area) = sac_plot_rects(rect);
        assert_eq!(caption.height(), PLOT_CAPTION_BAND);
        assert_eq!(caption.top(), rect.top());
        let pts = sac_polyline(&h.sac, area);
        assert!(pts.len() > 2);
        for p in &pts {
            assert!(p.y >= caption.bottom(), "SAC point {p:?} in the band");
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
        let (mut kg, mut free_trim, mut lcg) = (1.5, false, None);
        let mut text = Vec::new();
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                hydrostatics(
                    ui,
                    HydroPanel {
                        options: &mut options,
                        kg: &mut kg,
                        free_trim: &mut free_trim,
                        lcg: &mut lcg,
                        result: Some(&h),
                        stale: false,
                        curves: Some(&curves),
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
}
