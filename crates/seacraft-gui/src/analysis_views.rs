//! The Analysis menu's views: hydrostatic table, Bonjean curves, table of
//! offsets, trim scan and plate development (round 5, R5-8).
//!
//! Each view opens in the wide Analysis dock with its inputs and computes
//! on a [`Job`](crate::jobs::Job) from a [`ModelSnapshot`], through the same
//! [`commands`] the control channel calls. It follows the one staleness
//! rule: a result keeps the revision key it was computed for and is shown
//! dimmed once the key has moved on, until *Compute* or F5 recomputes it.
//! The result is drawn with the engineering table ([`panels::eng_table`]),
//! and *Copy CSV* copies that table ([`to_csv`]).
//!
//! Everything here is a function of the views' state and a result; the
//! [`App`](crate::App) owns the revision keys, runs the jobs and applies the
//! [`ViewAction`] the panel returns.

use eframe::egui;
use seacraft_core::develop::Unfolded;
use seacraft_core::i18n::{Key, Locale, Tr};

use crate::commands::{
    self, BonjeanParams, BonjeanResult, HydroTableParams, HydroTableResult, ModelSnapshot,
    OffsetsParams, OffsetsResult, TrimScanParams, TrimScanResult, UnfoldParams, UnfoldResult,
};
use crate::jobs::Track;
use crate::panels::{self, ColumnKind, EngCell, EngColumn};
use crate::theme;

/// One view of the Analysis menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AnalysisView {
    HydroTable,
    Bonjean,
    Offsets,
    TrimScan,
    /// The hull heeled under trial weights: drawn by `heel_view`, computed live on the app's heel jobs.
    Heel,
    /// The far-field wave pattern: drawn by `wave_view`, computed live on the app's wave job.
    Waves,
    Plates,
}

impl AnalysisView {
    /// Every view, in menu order.
    pub const ALL: [AnalysisView; 7] = [
        AnalysisView::HydroTable,
        AnalysisView::Bonjean,
        AnalysisView::Offsets,
        AnalysisView::TrimScan,
        AnalysisView::Waves,
        AnalysisView::Heel,
        AnalysisView::Plates,
    ];

    /// The menu item and the view's heading.
    pub fn label(self) -> &'static str {
        self.label_tr(seacraft_core::i18n::Tr::new(
            seacraft_core::i18n::Locale::En,
        ))
    }

    /// Localized menu item and view heading.
    pub fn label_tr(self, tr: seacraft_core::i18n::Tr) -> &'static str {
        use seacraft_core::i18n::Key;
        match self {
            AnalysisView::HydroTable => tr.s(Key::AnalysisViewHydroTable),
            AnalysisView::Bonjean => tr.s(Key::AnalysisViewBonjean),
            AnalysisView::Offsets => tr.s(Key::AnalysisViewOffsets),
            AnalysisView::TrimScan => tr.s(Key::AnalysisViewTrimScan),
            AnalysisView::Heel => tr.s(Key::AnalysisViewHeel),
            AnalysisView::Waves => tr.s(Key::AnalysisViewWaves),
            AnalysisView::Plates => tr.s(Key::AnalysisViewPlates),
        }
    }

    /// The view's tab in the drawer's header: its label when it is the view shown, else a short
    /// name, so the row of tabs still leaves room for the header's buttons.
    pub fn tab_label(self, shown: bool) -> &'static str {
        self.tab_label_tr(
            shown,
            seacraft_core::i18n::Tr::new(seacraft_core::i18n::Locale::En),
        )
    }

    /// Localized view tab label.
    pub fn tab_label_tr(self, shown: bool, tr: seacraft_core::i18n::Tr) -> &'static str {
        match self {
            AnalysisView::Heel if !shown => tr.s(seacraft_core::i18n::Key::AnalysisViewHeelShort),
            _ => self.label_tr(tr),
        }
    }

    /// A short name (tests, status lines).
    pub fn key(self) -> &'static str {
        match self {
            AnalysisView::HydroTable => "hydro_table",
            AnalysisView::Bonjean => "bonjean",
            AnalysisView::Offsets => "offsets",
            AnalysisView::TrimScan => "trim_scan",
            AnalysisView::Heel => "heel",
            AnalysisView::Waves => "waves",
            AnalysisView::Plates => "plates",
        }
    }

    /// The view a short name stands for.
    pub fn from_key(key: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|v| v.key() == key)
    }

    fn index(self) -> usize {
        self as usize
    }

    /// The result depends on the floating condition (draft, trim, heel,
    /// water density) as well as on the geometry and the view's inputs.
    pub fn uses_floating_condition(self) -> bool {
        matches!(self, AnalysisView::HydroTable | AnalysisView::TrimScan)
    }

    /// The view is computed on its slot's job by [`compute`], on *Compute* or F5. The wave pattern
    /// and the Heel view are not: they recompute live on the app's own jobs.
    pub fn uses_slot(self) -> bool {
        !matches!(self, AnalysisView::Waves | AnalysisView::Heel)
    }
}

/// Which layers the plate development measures.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PlateLayers {
    /// Every layer of the model, one row each.
    #[default]
    All,
    /// The layers marked developable in their properties.
    Marked,
    /// One layer (index).
    One(usize),
}

/// The plate development's inputs.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct PlateParams {
    pub layers: PlateLayers,
    /// Both sides of a symmetric layer, not starboard only.
    pub both_sides: bool,
    /// Subdivision level; `None` is the app's level.
    pub level: Option<u8>,
}

/// One layer's plate.
#[derive(Debug, Clone)]
pub struct PlateRow {
    pub layer: usize,
    pub name: String,
    /// The layer is marked developable in its properties.
    pub marked: bool,
    /// The developed plate, or why the layer could not be developed (a
    /// layer without faces).
    pub result: Result<UnfoldResult, String>,
}

/// A computed view.
#[derive(Debug, Clone)]
pub enum ViewResult {
    HydroTable(HydroTableResult),
    Bonjean(BonjeanResult),
    Offsets(OffsetsResult),
    TrimScan(TrimScanResult),
    Plates(Vec<PlateRow>),
}

/// The inputs of every view, kept while another view is shown.
#[derive(Debug, Clone, PartialEq)]
pub struct ViewInputs {
    pub table: HydroTableParams,
    pub bonjean: BonjeanParams,
    pub offsets: OffsetsParams,
    pub trim: TrimScanParams,
    pub plates: PlateParams,
}

impl Default for ViewInputs {
    fn default() -> Self {
        Self {
            table: HydroTableParams::default(),
            bonjean: BonjeanParams::default(),
            offsets: OffsetsParams::default(),
            // A scan needs a speed; the Performance tab's RAO speed.
            trim: TrimScanParams {
                speed_kn: 6.0,
                ..TrimScanParams::default()
            },
            plates: PlateParams::default(),
        }
    }
}

/// One view's result, the job computing the next one and the counter its
/// inputs bump.
#[derive(Default)]
pub struct Slot {
    pub result: Option<ViewResult>,
    pub track: Track<ViewResult>,
    /// Bumped by every edit of the view's inputs (part of its key).
    pub inputs_rev: u64,
}

/// The Analysis menu's state: the open view, every view's inputs and
/// results.
#[derive(Default)]
pub struct AnalysisViews {
    /// The view shown in the dock instead of the tab body.
    pub open: Option<AnalysisView>,
    pub inputs: ViewInputs,
    slots: [Slot; AnalysisView::ALL.len()],
    /// The plate row whose flat pattern is drawn.
    pub outline: usize,
}

impl AnalysisViews {
    pub fn slot(&self, view: AnalysisView) -> &Slot {
        &self.slots[view.index()]
    }

    pub fn slot_mut(&mut self, view: AnalysisView) -> &mut Slot {
        &mut self.slots[view.index()]
    }

    /// Any view's job is running.
    pub fn busy(&self) -> bool {
        self.slots.iter().any(|s| s.track.busy())
    }

    /// A new model: no results, and no layer choice that may not exist.
    pub fn clear_results(&mut self) {
        for slot in &mut self.slots {
            slot.result = None;
            slot.track.reset();
        }
        self.inputs.plates.layers = PlateLayers::All;
        self.outline = 0;
    }
}

/// Compute `view` on a snapshot of the model (on a job).
pub fn compute(
    view: AnalysisView,
    inputs: &ViewInputs,
    model: &ModelSnapshot,
    tr: Tr,
) -> Result<ViewResult, String> {
    let m = model.view();
    Ok(match view {
        AnalysisView::HydroTable => {
            ViewResult::HydroTable(commands::hydro_table_in(m, &inputs.table)?)
        }
        AnalysisView::Bonjean => ViewResult::Bonjean(commands::bonjean_in(m, &inputs.bonjean)?),
        AnalysisView::Offsets => ViewResult::Offsets(commands::offsets_in(m, &inputs.offsets)?),
        AnalysisView::TrimScan => ViewResult::TrimScan(commands::trim_scan_in(m, &inputs.trim)?),
        AnalysisView::Plates => ViewResult::Plates(plates(model, &inputs.plates, tr)?),
        AnalysisView::Waves => {
            return Err("the wave pattern is computed by the window's wave job".into())
        }
        AnalysisView::Heel => {
            return Err("the Heel view is computed by the window's heel jobs".into())
        }
    })
}

/// Develop each chosen layer flat, one row per layer.
fn plates(model: &ModelSnapshot, params: &PlateParams, tr: Tr) -> Result<Vec<PlateRow>, String> {
    let surface = model
        .surface
        .as_ref()
        .ok_or_else(|| tr.s(Key::AnalysisErrNoModel).to_string())?;
    let chosen: Vec<usize> = match params.layers {
        PlateLayers::All => (0..surface.layers.len()).collect(),
        PlateLayers::Marked => (0..surface.layers.len())
            .filter(|&i| surface.layers[i].developable)
            .collect(),
        PlateLayers::One(i) if i < surface.layers.len() => vec![i],
        PlateLayers::One(i) => {
            return Err(tr.fmt(Key::AnalysisErrNoLayer, &[("index", &i.to_string())]))
        }
    };
    if chosen.is_empty() {
        return Err(tr.s(Key::AnalysisErrNoneMarked).to_owned());
    }
    Ok(chosen
        .into_iter()
        .map(|i| {
            let layer = &surface.layers[i];
            let result = commands::unfold_in(
                model.view(),
                &UnfoldParams {
                    level: params.level,
                    both_sides: params.both_sides,
                    layer: Some(i),
                    ..UnfoldParams::default()
                },
            );
            PlateRow {
                layer: i,
                name: layer.name.clone(),
                marked: layer.developable,
                result,
            }
        })
        .collect())
}

// ---- tables ----------------------------------------------------------------

/// A column of a [`ViewTable`], owning its title (some are built from the
/// ordinates: `T 0.25`, `WL 0.40`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TableColumn {
    pub title: String,
    pub unit: String,
    pub kind: ColumnKind,
}

impl TableColumn {
    fn number(title: impl Into<String>, unit: &str) -> Self {
        Self {
            title: title.into(),
            unit: unit.to_owned(),
            kind: ColumnKind::Number,
        }
    }

    fn text(title: &str) -> Self {
        Self {
            title: title.to_owned(),
            unit: String::new(),
            kind: ColumnKind::Text,
        }
    }
}

/// A view's result as the engineering table shows it: columns with their
/// units, and every cell already formatted at its column's precision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ViewTable {
    pub columns: Vec<TableColumn>,
    pub rows: Vec<Vec<EngCell>>,
}

impl ViewTable {
    /// The columns as [`panels::eng_table`] takes them.
    pub fn eng_columns(&self) -> Vec<EngColumn<'_>> {
        self.columns
            .iter()
            .map(|c| EngColumn {
                title: &c.title,
                unit: &c.unit,
                kind: c.kind,
            })
            .collect()
    }
}

/// The dash for a value that does not exist (a waterline above the
/// section, a layer without faces).
pub const NO_VALUE: &str = "—";

fn num(value: f64, decimals: usize) -> EngCell {
    EngCell::Text(format!("{value:.decimals$}"))
}

fn opt(value: Option<f64>, decimals: usize) -> EngCell {
    value.map_or_else(|| NO_VALUE.into(), |v| num(v, decimals))
}

/// The engineering table of a result.
pub fn table(result: &ViewResult) -> ViewTable {
    table_tr(result, Tr::new(Locale::En))
}

/// Localized engineering table of a result.
pub fn table_tr(result: &ViewResult, tr: Tr) -> ViewTable {
    match result {
        ViewResult::HydroTable(t) => hydro_table(t, tr),
        ViewResult::Bonjean(b) => bonjean_table(b),
        ViewResult::Offsets(o) => offsets_table(o, tr),
        ViewResult::TrimScan(s) => trim_table(s, tr),
        ViewResult::Plates(rows) => plate_table(rows, tr),
    }
}

fn hydro_table(t: &HydroTableResult, tr: Tr) -> ViewTable {
    let columns = vec![
        TableColumn::number("T", "m"),
        TableColumn::number(tr.s(Key::AnalysisColVolume), "m³"),
        TableColumn::number("Δ", "t"),
        TableColumn::number("KB", "m"),
        TableColumn::number("BM", "m"),
        TableColumn::number("BML", "m"),
        TableColumn::number("KM", "m"),
        TableColumn::number("LCB", "m"),
        TableColumn::number("LCF", "m"),
        TableColumn::number("Cb", ""),
        TableColumn::number("Cp", ""),
        TableColumn::number("Cwp", ""),
        TableColumn::number("Awp", "m²"),
        TableColumn::number("LWL", "m"),
        TableColumn::number("BWL", "m"),
        TableColumn::number("TPC", "t/cm"),
        TableColumn::number("MTC", "t·m/cm"),
        TableColumn::number("WSA", "m²"),
    ];
    let rows = t
        .rows
        .iter()
        .map(|r| {
            vec![
                num(r.draft, 3),
                num(r.volume_m3, 3),
                num(r.displacement_t, 3),
                num(r.kb, 3),
                num(r.bm_transverse, 3),
                num(r.bm_longitudinal, 2),
                num(r.km_transverse, 3),
                num(r.lcb_x, 3),
                num(r.lcf_x, 3),
                num(r.block_coefficient, 4),
                num(r.prismatic_coefficient, 4),
                num(r.waterplane_coefficient, 4),
                num(r.waterplane_area_m2, 3),
                num(r.lwl, 3),
                num(r.bwl, 3),
                num(r.tpc, 4),
                num(r.mtc, 4),
                num(r.wetted_surface_m2, 2),
            ]
        })
        .collect();
    ViewTable { columns, rows }
}

fn bonjean_table(b: &BonjeanResult) -> ViewTable {
    let mut columns = vec![TableColumn::number("x", "m")];
    columns.extend(
        b.drafts_m
            .iter()
            .map(|d| TableColumn::number(format!("T {d:.2}"), "m²")),
    );
    let rows = b
        .sections
        .iter()
        .map(|s| {
            let mut row = vec![num(s.x, 3)];
            row.extend(s.areas_m2.iter().map(|a| num(*a, 3)));
            row
        })
        .collect();
    ViewTable { columns, rows }
}

fn offsets_table(o: &OffsetsResult, tr: Tr) -> ViewTable {
    let mut columns = vec![
        TableColumn::number("x", "m"),
        TableColumn::number(tr.s(Key::AnalysisColKeelZ), "m"),
        TableColumn::number(tr.s(Key::AnalysisColDeckZ), "m"),
        TableColumn::number(tr.s(Key::AnalysisColHalfBeam), "m"),
    ];
    columns.extend(
        o.waterlines_m
            .iter()
            .map(|z| TableColumn::number(format!("WL {z:.2}"), "m")),
    );
    columns.extend(
        o.buttocks_m
            .iter()
            .map(|y| TableColumn::number(format!("BL {y:.2}"), "m")),
    );
    let rows = o
        .stations
        .iter()
        .map(|s| {
            let mut row = vec![
                num(s.x, 3),
                num(s.keel_z, 3),
                num(s.deck_z, 3),
                num(s.half_beam, 3),
            ];
            row.extend(s.waterline_half_breadths.iter().map(|v| opt(*v, 3)));
            row.extend(s.buttock_heights.iter().map(|v| opt(*v, 3)));
            row
        })
        .collect();
    ViewTable { columns, rows }
}

fn trim_table(s: &TrimScanResult, tr: Tr) -> ViewTable {
    let columns = vec![
        TableColumn::number(tr.s(Key::AnalysisColTrim), "m"),
        TableColumn::number("T", "m"),
        TableColumn::number(tr.s(Key::AnalysisColVolume), "m³"),
        TableColumn::number("LWL", "m"),
        TableColumn::number("Fn", ""),
        TableColumn::number("R_T", "kN"),
        TableColumn::number("P_E", "kW"),
        TableColumn::text(""),
    ];
    let best = s.best.as_ref().map(|b| b.trim_m);
    let rows = s
        .points
        .iter()
        .map(|p| {
            vec![
                num(p.trim_m, 3),
                num(p.draft_m, 3),
                num(p.volume_m3, 3),
                num(p.lwl_m, 3),
                num(p.froude, 3),
                num(p.rt_n / 1000.0, 3),
                num(p.pe_kw, 2),
                if best == Some(p.trim_m) {
                    tr.s(Key::AnalysisTrimLeastRt).into()
                } else {
                    "".into()
                },
            ]
        })
        .collect();
    ViewTable { columns, rows }
}

fn plate_table(rows: &[PlateRow], tr: Tr) -> ViewTable {
    let columns = vec![
        TableColumn::text(tr.s(Key::AnalysisColLayer)),
        TableColumn::text(tr.s(Key::AnalysisColMarked)),
        TableColumn::number(tr.s(Key::AnalysisColFaces), ""),
        TableColumn::number(tr.s(Key::AnalysisColArea), "m²"),
        TableColumn::number(tr.s(Key::AnalysisColFlatWidth), "m"),
        TableColumn::number(tr.s(Key::AnalysisColFlatHeight), "m"),
        TableColumn::number(tr.s(Key::AnalysisColMaxGap), "mm"),
        TableColumn::number(tr.s(Key::AnalysisColRmsGap), "mm"),
        TableColumn::number(tr.s(Key::AnalysisColGapRatio), "%"),
        TableColumn::text(tr.s(Key::AnalysisColDevelopable)),
    ];
    let rows = rows
        .iter()
        .map(|r| {
            let marked: EngCell = if r.marked {
                tr.s(Key::AnalysisPlatesCellYes)
            } else {
                NO_VALUE
            }
            .into();
            match &r.result {
                Ok(u) => {
                    let p = &u.plate;
                    vec![
                        r.name.as_str().into(),
                        marked,
                        EngCell::Text(u.faces.to_string()),
                        num(p.area_3d_m2, 3),
                        num(p.width_m, 3),
                        num(p.height_m, 3),
                        num(p.max_gap_m * 1000.0, 1),
                        num(p.rms_gap_m * 1000.0, 1),
                        num(p.gap_ratio * 100.0, 2),
                        if p.developable {
                            tr.s(Key::AnalysisPlatesCellYes)
                        } else {
                            tr.s(Key::AnalysisPlatesCellNo)
                        }
                        .into(),
                    ]
                }
                Err(_) => {
                    let mut row = vec![r.name.as_str().into(), marked];
                    row.extend((0..7).map(|_| EngCell::from(NO_VALUE)));
                    row.push(tr.s(Key::AnalysisPlatesCellNoFaces).into());
                    row
                }
            }
        })
        .collect();
    ViewTable { columns, rows }
}

/// The lines above a result's table: what it was computed for.
pub fn caption(result: &ViewResult) -> Vec<String> {
    caption_tr(result, Tr::new(Locale::En))
}

/// Localized caption lines above a result's table.
pub fn caption_tr(result: &ViewResult, tr: Tr) -> Vec<String> {
    match result {
        ViewResult::HydroTable(t) => {
            let drafts_s = t.drafts.to_string();
            let density_s = format!("{:.3}", t.water_density / 1000.0);
            let level_s = t.level.to_string();
            vec![tr.fmt(
                Key::AnalysisHydroCaption,
                &[
                    ("drafts", &drafts_s),
                    ("density", &density_s),
                    ("level", &level_s),
                ],
            )]
        }
        ViewResult::Bonjean(b) => {
            let depth_s = format!("{:.3}", b.depth_m);
            let draft_s = format!("{:.3}", b.design_draft_m);
            let area_s = format!("{:.3}", b.area_max_m2);
            let level_s = b.level.to_string();
            vec![tr.fmt(
                Key::AnalysisBonjeanCaption,
                &[
                    ("depth", &depth_s),
                    ("draft", &draft_s),
                    ("area", &area_s),
                    ("level", &level_s),
                ],
            )]
        }
        ViewResult::Offsets(o) => {
            let keel_s = format!("{:.3}", o.keel_z);
            let widest_s = format!("{:.3}", o.max_half_breadth_m);
            let level_s = o.level.to_string();
            vec![tr.fmt(
                Key::AnalysisOffsetsCaption,
                &[
                    ("keel", &keel_s),
                    ("widest", &widest_s),
                    ("level", &level_s),
                ],
            )]
        }
        ViewResult::TrimScan(s) => {
            let speed_s = format!("{:.2}", s.speed_kn);
            let volume_s = format!("{:.3}", s.target_volume_m3);
            let level_s = s.level.to_string();
            let mut lines = vec![tr.fmt(
                Key::AnalysisTrimCaption,
                &[
                    ("speed", &speed_s),
                    ("volume", &volume_s),
                    ("level", &level_s),
                ],
            )];
            if s.at_boundary {
                lines.push(tr.s(Key::AnalysisTrimBoundaryWarn).to_owned());
            }
            lines.extend(s.warnings.iter().map(|w| w.render(tr)));
            lines
        }
        ViewResult::Plates(_) => vec![tr.s(Key::AnalysisPlatesCaption).to_owned()],
    }
}

/// The table as CSV: a header of `title (unit)`, then one line per row. A
/// missing value is an empty field; a field with a comma, quote or line
/// break is quoted.
pub fn to_csv(table: &ViewTable) -> String {
    to_csv_tr(table, Tr::new(Locale::En))
}

/// Localized CSV formatting (localizes criteria badge strings via `tr`).
pub fn to_csv_tr(table: &ViewTable, tr: Tr) -> String {
    fn field(text: &str) -> String {
        if text.contains([',', '"', '\n', '\r']) {
            format!("\"{}\"", text.replace('"', "\"\""))
        } else {
            text.to_owned()
        }
    }
    let mut out = String::new();
    let header: Vec<String> = table
        .columns
        .iter()
        .map(|c| match (c.title.is_empty(), c.unit.is_empty()) {
            (_, true) => field(&c.title),
            (true, false) => field(&format!("({})", c.unit)),
            (false, false) => field(&format!("{} ({})", c.title, c.unit)),
        })
        .collect();
    out.push_str(&header.join(","));
    out.push('\n');
    for row in &table.rows {
        let cells: Vec<String> = (0..table.columns.len())
            .map(|c| match row.get(c) {
                None => String::new(),
                Some(EngCell::Text(t) | EngCell::Muted(t)) if t == NO_VALUE => String::new(),
                Some(EngCell::Text(t) | EngCell::Muted(t)) => field(t),
                Some(EngCell::Badge(passed, stale)) => {
                    field(panels::criteria_badge_style_tr(*passed, *stale, tr).0)
                }
            })
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
    }
    out
}

// ---- the panel -------------------------------------------------------------

/// What the view's panel asked for, applied by the App after the panel.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewAction {
    None,
    /// The inputs changed: the result is now stale.
    Changed,
    Compute,
    CopyCsv,
    Close,
    /// Show another view in the drawer.
    Open(AnalysisView),
    /// Open File > Builder panels, for patterns of the developable panels.
    OpenBuilderPanels,
}

/// The view's panel inputs.
pub struct ViewPanel<'a> {
    pub view: AnalysisView,
    pub state: &'a mut AnalysisViews,
    pub stale: bool,
    pub busy: bool,
    pub failure: Option<String>,
    /// Every layer: its name and whether it is marked developable.
    pub layers: Vec<(String, bool)>,
    /// The app's draft (the hydrostatic table's default top draft).
    pub design_draft: f64,
}

/// Draw a view: the views as tabs, *Compute*, *Copy CSV* and close, its inputs,
/// then the result (dimmed when stale): caption, table, plot.
pub fn panel(ui: &mut egui::Ui, p: ViewPanel<'_>) -> ViewAction {
    panel_tr(ui, p, Tr::new(Locale::En))
}

/// Localized form of [`panel`].
pub fn panel_tr(ui: &mut egui::Ui, p: ViewPanel<'_>, tr: Tr) -> ViewAction {
    let mut action = ViewAction::None;
    let view = p.view;
    let has_result = p.state.slot(view).result.is_some();
    // The row wraps when the drawer is too narrow for every tab and button, so none is pushed
    // off its edge.
    ui.horizontal_wrapped(|ui| {
        // The views as tabs: the one shown is the drawer's title, and the
        // others are one click away without the menu.
        for other in AnalysisView::ALL {
            let shown = other == view;
            if ui
                .selectable_label(shown, other.tab_label_tr(shown, tr))
                .clicked()
                && !shown
            {
                action = ViewAction::Open(other);
            }
        }
        ui.separator();
        if ui
            .add_enabled(!p.busy, egui::Button::new(tr.s(Key::CommonCompute)))
            .on_hover_text(tr.s(Key::AnalysisBtnComputeTip))
            .clicked()
        {
            action = ViewAction::Compute;
        }
        if ui
            .add_enabled(has_result, egui::Button::new(tr.s(Key::AnalysisBtnCopyCsv)))
            .on_hover_text(tr.s(Key::AnalysisBtnCopyCsvTip))
            .clicked()
        {
            action = ViewAction::CopyCsv;
        }
        if view == AnalysisView::Plates
            && ui
                .button(tr.s(Key::AnalysisBtnExportPatterns))
                .on_hover_text(tr.s(Key::AnalysisBtnExportPatternsTip))
                .clicked()
        {
            action = ViewAction::OpenBuilderPanels;
        }
        if p.busy {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui
                .button("✕")
                .on_hover_text(tr.fmt(Key::AnalysisBtnCloseTip, &[("view", view.label_tr(tr))]))
                .clicked()
            {
                action = ViewAction::Close;
            }
        });
    });
    ui.add_space(2.0);
    if inputs(ui, view, &mut p.state.inputs, &p.layers, p.design_draft, tr) {
        action = ViewAction::Changed;
    }
    ui.add_space(2.0);
    if let Some(msg) = &p.failure {
        ui.colored_label(theme::FAIL, format!("⚠ {msg}"));
    }
    ui.separator();
    let outline = &mut p.state.outline;
    let Some(result) = p.state.slots[view.index()].result.as_ref() else {
        if !p.busy {
            ui.label(egui::RichText::new(tr.s(Key::AnalysisEmpty)).color(theme::TEXT_MUTED));
        }
        return action;
    };
    egui::ScrollArea::vertical()
        .id_salt("analysis_view_body")
        .show(ui, |ui| {
            panels::stale_section_tr(ui, p.stale, tr, |ui| {
                for line in caption_tr(result, tr) {
                    ui.label(egui::RichText::new(line).color(theme::TEXT_SECONDARY));
                }
                ui.add_space(4.0);
                let t = table_tr(result, tr);
                egui::ScrollArea::horizontal()
                    .id_salt(("analysis_table", view.key()))
                    .show(ui, |ui| {
                        panels::eng_table_tr(ui, &t.eng_columns(), &t.rows, tr);
                    });
                ui.add_space(6.0);
                plot(ui, result, outline, tr);
            });
        });
    action
}

/// A drag value that reports a change.
fn drag_usize(
    ui: &mut egui::Ui,
    value: &mut usize,
    range: std::ops::RangeInclusive<usize>,
    prefix: &str,
) -> bool {
    ui.add(theme::num(value).range(range).prefix(prefix))
        .changed()
}

/// The view's inputs; true when one changed.
fn inputs(
    ui: &mut egui::Ui,
    view: AnalysisView,
    inputs: &mut ViewInputs,
    layers: &[(String, bool)],
    design_draft: f64,
    tr: Tr,
) -> bool {
    let mut changed = false;
    ui.horizontal_wrapped(|ui| match view {
        AnalysisView::HydroTable => {
            let p = &mut inputs.table;
            let mut to = p.to.unwrap_or(design_draft);
            ui.label(tr.s(Key::AnalysisInputDraftsUpTo));
            if ui
                .add(
                    theme::num(&mut to)
                        .speed(0.01)
                        .range(0.01..=1.0e3)
                        .fixed_decimals(3)
                        .suffix(" m"),
                )
                .changed()
            {
                p.to = Some(to);
                changed = true;
            }
            if p.to.is_some() {
                if ui
                    .small_button(tr.s(Key::AnalysisInputDesignDraft))
                    .on_hover_text(tr.s(Key::AnalysisInputDesignDraftTip))
                    .clicked()
                {
                    p.to = None;
                    changed = true;
                }
            } else {
                ui.label(
                    egui::RichText::new(tr.s(Key::AnalysisInputDesignDraftMuted))
                        .color(theme::TEXT_MUTED),
                );
            }
            changed |= drag_usize(
                ui,
                &mut p.steps,
                1..=100,
                tr.s(Key::AnalysisInputStepsPrefix),
            );
        }
        AnalysisView::Bonjean => {
            let p = &mut inputs.bonjean;
            changed |= drag_usize(
                ui,
                &mut p.stations,
                2..=200,
                tr.s(Key::AnalysisInputStationsPrefix),
            );
            changed |= drag_usize(
                ui,
                &mut p.draft_steps,
                2..=50,
                tr.s(Key::AnalysisInputDraftsPrefix),
            );
        }
        AnalysisView::Offsets => {
            let p = &mut inputs.offsets;
            changed |= drag_usize(
                ui,
                &mut p.stations,
                2..=200,
                tr.s(Key::AnalysisInputStationsPrefix),
            );
            changed |= drag_usize(
                ui,
                &mut p.waterline_steps,
                1..=40,
                tr.s(Key::AnalysisInputWaterlinesPrefix),
            );
            changed |= drag_usize(
                ui,
                &mut p.buttock_steps,
                1..=40,
                tr.s(Key::AnalysisInputButtocksPrefix),
            );
        }
        AnalysisView::TrimScan => {
            let p = &mut inputs.trim;
            changed |= ui
                .add(
                    theme::num(&mut p.speed_kn)
                        .speed(0.1)
                        .range(0.1..=100.0)
                        .prefix(tr.s(Key::AnalysisInputSpeedPrefix))
                        .suffix(" kn"),
                )
                .changed();
            changed |= ui
                .add(
                    theme::num(&mut p.from)
                        .speed(0.01)
                        .range(-20.0..=20.0)
                        .fixed_decimals(2)
                        .prefix(tr.s(Key::AnalysisInputTrimFromPrefix))
                        .suffix(" m"),
                )
                .changed();
            changed |= ui
                .add(
                    theme::num(&mut p.to)
                        .speed(0.01)
                        .range(-20.0..=20.0)
                        .fixed_decimals(2)
                        .prefix(tr.s(Key::AnalysisInputToPrefix))
                        .suffix(" m"),
                )
                .changed();
            changed |= drag_usize(
                ui,
                &mut p.steps,
                2..=60,
                tr.s(Key::AnalysisInputStepsPrefix),
            );
            changed |= ui
                .checkbox(&mut p.hold_volume, tr.s(Key::AnalysisInputHoldDisplacement))
                .changed();
        }
        // The wave and heel views draw their own inputs (`wave_view`, `heel_view`).
        AnalysisView::Waves | AnalysisView::Heel => {}
        AnalysisView::Plates => {
            let p = &mut inputs.plates;
            let name = |l: PlateLayers| match l {
                PlateLayers::All => tr.s(Key::AnalysisInputLayersAll).to_owned(),
                PlateLayers::Marked => tr.s(Key::AnalysisInputLayersMarked).to_owned(),
                PlateLayers::One(i) => layers.get(i).map_or_else(
                    || {
                        let idx_s = i.to_string();
                        tr.fmt(Key::AnalysisInputLayersOneFallback, &[("index", &idx_s)])
                    },
                    |(n, _)| n.clone(),
                ),
            };
            let before = p.layers;
            egui::ComboBox::from_id_salt("plate_layers")
                .selected_text(name(p.layers))
                .show_ui(ui, |ui| {
                    let mut options = vec![PlateLayers::All, PlateLayers::Marked];
                    options.extend((0..layers.len()).map(PlateLayers::One));
                    for o in options {
                        ui.selectable_value(&mut p.layers, o, name(o));
                    }
                });
            changed |= p.layers != before;
            changed |= ui
                .checkbox(&mut p.both_sides, tr.s(Key::AnalysisInputBothSides))
                .changed();
        }
    });
    changed
}

/// The plot under a table, where the view has one.
fn plot(ui: &mut egui::Ui, result: &ViewResult, outline: &mut usize, tr: Tr) {
    match result {
        ViewResult::Bonjean(b) => {
            // One curve per station, draft up and area across, unlabelled
            // (21 legend entries would hide the plot).
            let series: Vec<(String, Vec<(f64, f64)>)> = b
                .sections
                .iter()
                .map(|s| {
                    (
                        String::new(),
                        s.areas_m2
                            .iter()
                            .zip(&b.drafts_m)
                            .map(|(a, d)| (*a, *d))
                            .collect(),
                    )
                })
                .collect();
            panels::plot_xy_tr(
                ui,
                tr.s(Key::AnalysisViewBonjean),
                tr.s(Key::AnalysisBonjeanPlotX),
                tr.s(Key::AnalysisBonjeanPlotY),
                &series,
                tr,
            );
        }
        ViewResult::TrimScan(s) => {
            let series = vec![(
                "R_T".to_owned(),
                s.points
                    .iter()
                    .map(|p| (p.trim_m, p.rt_n / 1000.0))
                    .collect(),
            )];
            panels::plot_xy_tr(
                ui,
                tr.s(Key::AnalysisViewTrimScan),
                tr.s(Key::AnalysisTrimPlotX),
                tr.s(Key::AnalysisTrimPlotY),
                &series,
                tr,
            );
        }
        ViewResult::Plates(rows) => {
            let developed: Vec<(usize, &PlateRow, &Unfolded)> = rows
                .iter()
                .enumerate()
                .filter_map(|(i, r)| r.result.as_ref().ok().map(|u| (i, r, &u.plate)))
                .collect();
            if developed.is_empty() {
                return;
            }
            if !developed.iter().any(|(i, _, _)| i == outline) {
                *outline = developed[0].0;
            }
            if developed.len() > 1 {
                ui.horizontal(|ui| {
                    ui.label(tr.s(Key::AnalysisPlatesFlatPatternOf));
                    egui::ComboBox::from_id_salt("plate_outline")
                        .selected_text(rows[*outline].name.clone())
                        .show_ui(ui, |ui| {
                            for (i, r, _) in &developed {
                                ui.selectable_value(outline, *i, r.name.clone());
                            }
                        });
                });
            }
            if let Some((_, r, plate)) = developed.iter().find(|(i, _, _)| i == outline) {
                flat_pattern(ui, &r.name, plate, tr);
            }
        }
        ViewResult::HydroTable(_) | ViewResult::Offsets(_) => {}
    }
}

/// The developed plate drawn to scale: every flattened face outlined.
fn flat_pattern(ui: &mut egui::Ui, name: &str, plate: &Unfolded, tr: Tr) {
    let width = ui.available_width().max(120.0);
    let height = 240.0;
    let (response, painter) = ui.allocate_painter(egui::vec2(width, height), egui::Sense::hover());
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, theme::PLOT_BG);
    painter.rect_stroke(
        rect,
        2.0,
        egui::Stroke::new(1.0, theme::FRAME),
        egui::StrokeKind::Inside,
    );
    let (caption, area) = panels::split_caption(rect);
    let w_s = format!("{:.3}", plate.width_m);
    let h_s = format!("{:.3}", plate.height_m);
    painter.text(
        caption.left_center() + egui::vec2(6.0, 0.0),
        egui::Align2::LEFT_CENTER,
        tr.fmt(
            Key::AnalysisPlatesFlatPatternCaption,
            &[("name", name), ("width", &w_s), ("height", &h_s)],
        ),
        theme::caption_font(),
        theme::PLOT_TEXT,
    );
    let (mut lo, mut hi) = ((f64::MAX, f64::MAX), (f64::MIN, f64::MIN));
    for (x, y) in plate.faces.iter().flat_map(|f| f.xy.iter()) {
        lo = (lo.0.min(*x), lo.1.min(*y));
        hi = (hi.0.max(*x), hi.1.max(*y));
    }
    let (w, h) = (hi.0 - lo.0, hi.1 - lo.1);
    if !(w.is_finite() && h.is_finite()) || w <= 0.0 && h <= 0.0 {
        return;
    }
    let inner = area.shrink(10.0);
    // One scale for both axes: the plate keeps its shape.
    let scale = (inner.width() as f64 / w.max(1e-9)).min(inner.height() as f64 / h.max(1e-9));
    let offset = egui::vec2(
        (inner.width() - (w * scale) as f32) * 0.5,
        (inner.height() - (h * scale) as f32) * 0.5,
    );
    let to_screen = |(x, y): (f64, f64)| {
        inner.left_top()
            + offset
            + egui::vec2(((x - lo.0) * scale) as f32, ((hi.1 - y) * scale) as f32)
    };
    let stroke = egui::Stroke::new(0.6, theme::SECTION_CURVE);
    for f in &plate.faces {
        let mut pts: Vec<egui::Pos2> = f.xy.iter().map(|&p| to_screen(p)).collect();
        if let Some(&first) = pts.first() {
            pts.push(first);
        }
        painter.add(egui::Shape::line(pts, stroke));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> ViewTable {
        ViewTable {
            columns: vec![
                TableColumn::text("Layer"),
                TableColumn::number("Area", "m²"),
                TableColumn::number("Cb", ""),
            ],
            rows: vec![
                vec!["Bottom".into(), "1.250".into(), "0.5040".into()],
                vec!["Deck, aft".into(), NO_VALUE.into(), "0.6000".into()],
                vec![
                    "Say \"hi\"".into(),
                    "2.000".into(),
                    EngCell::Badge(true, false),
                ],
            ],
        }
    }

    #[test]
    fn csv_has_a_unit_header_and_one_line_per_row() {
        let csv = to_csv(&sample());
        let lines: Vec<&str> = csv.lines().collect();
        assert_eq!(lines.len(), 4, "{csv}");
        assert_eq!(lines[0], "Layer,Area (m²),Cb");
        assert_eq!(lines[1], "Bottom,1.250,0.5040");
        // A missing value is empty; a comma is quoted.
        assert_eq!(lines[2], "\"Deck, aft\",,0.6000");
        // Quotes are doubled; a badge reads as its verdict.
        assert_eq!(lines[3], "\"Say \"\"hi\"\"\",2.000,PASS");
        assert!(csv.ends_with('\n'));
    }

    #[test]
    fn every_view_has_a_label_and_a_key() {
        for v in AnalysisView::ALL {
            assert_eq!(AnalysisView::from_key(v.key()), Some(v));
            assert!(!v.label().is_empty());
        }
        assert_eq!(AnalysisView::from_key("nonsense"), None);
    }

    #[test]
    fn plate_errors_keep_the_row_width() {
        let rows = vec![PlateRow {
            layer: 2,
            name: "Empty".into(),
            marked: true,
            result: Err("no faces".into()),
        }];
        let t = plate_table(&rows, Tr::new(Locale::En));
        assert_eq!(t.rows[0].len(), t.columns.len());
        assert_eq!(t.rows[0][1], EngCell::from("yes"));
    }
}
