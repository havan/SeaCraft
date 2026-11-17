//! The visual system: every colour the GUI paints, the type scale and the
//! spacing, in one place.
//!
//! The palette keeps the app's identity (dark slate canvas, sea-blue hull,
//! amber control net, red creases) and fixes contrast and consistency: a FAIL
//! is the same red in every dock, and the grid has three tiers: quiet minor
//! lines (≈ 1.25:1 against the canvas, so they never compete with the hull),
//! stronger major lines (≈ 1.7:1) every 5 m, and the datums strongest of all.
//!
//! No other module writes a colour literal (`Color32::from_rgb` and friends);
//! the test at the bottom of this file reads the sources and enforces it.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

// ---- surfaces ----------------------------------------------------------

/// Viewport background: dark slate.
pub const CANVAS: Color32 = Color32::from_rgb(0x13, 0x19, 0x21);
/// Dock and window background: the canvas slate, one step lighter, so the
/// docks and the viewports read as one surface family.
pub const PANEL: Color32 = Color32::from_rgb(0x1a, 0x21, 0x2b);
/// Every other row of an engineering table, a shade above the panel.
pub const TABLE_STRIPE: Color32 = Color32::from_rgb(0x1f, 0x27, 0x32);
/// Floating windows and menus, a step above the panel they cover.
pub const WINDOW: Color32 = Color32::from_rgb(0x1e, 0x26, 0x31);
/// A button at rest: raised above the panel. Something that does a thing.
pub const RAISED: Color32 = Color32::from_rgb(0x2a, 0x34, 0x42);
/// A button under the pointer, and while it is pressed.
pub const RAISED_HOVER: Color32 = Color32::from_rgb(0x34, 0x40, 0x50);
pub const RAISED_ACTIVE: Color32 = Color32::from_rgb(0x3e, 0x4c, 0x5f);
/// A field that takes a value: sunk below the panel, with an outline.
/// Something that holds a thing.
pub const FIELD: Color32 = Color32::from_rgb(0x10, 0x15, 0x1c);
pub const FIELD_HOVER: Color32 = Color32::from_rgb(0x14, 0x1b, 0x24);
/// Outlines: of a field and a button at rest, and under the pointer.
pub const OUTLINE: Color32 = Color32::from_rgb(0x36, 0x43, 0x53);
pub const OUTLINE_HOVER: Color32 = Color32::from_rgb(0x52, 0x65, 0x7c);
/// The fill of the selected thing: the active layer, the open tab, the
/// chosen one of several. Never a switch that is merely on.
pub const SELECTED: Color32 = Color32::from_rgb(0x24, 0x56, 0x8a);
/// A display switch that is on: brighter than a button, and not the
/// selection colour, so "on" and "selected" are told apart.
pub const SWITCH_ON: Color32 = Color32::from_rgb(0x45, 0x55, 0x6b);
/// Text on [`SELECTED`] and on the primary button.
pub const TEXT_ON_ACCENT: Color32 = Color32::from_rgb(0xf2, 0xf6, 0xfa);
/// Adds `slider` with its value drawn as a field (see [`as_field`]): the
/// rail and the handle keep the raised fill, the number beside them is sunk.
pub fn slider(ui: &mut egui::Ui, slider: egui::Slider<'_>) -> egui::Response {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        w.inactive.weak_bg_fill = FIELD;
        w.hovered.weak_bg_fill = FIELD_HOVER;
        w.active.weak_bg_fill = FIELD_HOVER;
        ui.add(slider)
    })
    .inner
}

/// The primary button of a dialog: the one action it exists for.
pub const PRIMARY: Color32 = Color32::from_rgb(0x2f, 0x6f, 0xb0);
/// Plot area background, a shade below the canvas.
pub const PLOT_BG: Color32 = Color32::from_rgb(0x0f, 0x14, 0x1b);
/// Viewport frame, inactive and active.
pub const FRAME: Color32 = Color32::from_rgb(0x2a, 0x35, 0x42);
pub const FRAME_ACTIVE: Color32 = Color32::from_rgb(0x54, 0x8c, 0xc8);
/// Semi-opaque canvas behind viewport captions, so geometry under them never
/// makes the text unreadable.
pub const HUD_BACKING: Color32 = Color32::from_rgba_unmultiplied_const(0x13, 0x19, 0x21, 0xc8);

// ---- text --------------------------------------------------------------

/// Values and anything the eye should land on first.
pub const TEXT_PRIMARY: Color32 = Color32::from_rgb(0xdc, 0xe3, 0xea);
/// Quantity names, captions on the canvas.
pub const TEXT_SECONDARY: Color32 = Color32::from_rgb(0x8e, 0x9e, 0xae);
/// A plain label: between a value and a quantity's name.
pub const TEXT_LABEL: Color32 = Color32::from_rgb(0xb6, 0xc2, 0xce);
/// Units and other supporting text.
pub const TEXT_MUTED: Color32 = Color32::from_rgb(0x6f, 0x7f, 0x90);

// ---- viewport ----------------------------------------------------------

/// Minor grid lines (1 m at the usual zoom): about 1.25:1 against
/// [`CANVAS`] (WCAG relative luminance), there when looked for and quiet
/// otherwise.
pub const GRID_MINOR: Color32 = Color32::from_rgb(0x22, 0x2c, 0x38);
/// Major grid lines (every 5 m at the usual zoom): about 1.7:1.
pub const GRID_MAJOR: Color32 = Color32::from_rgb(0x34, 0x41, 0x52);
/// Datum lines: centreline (y = 0), baseline (z = 0), x = 0, midship and the
/// design waterline. Above 3:1.
pub const GRID_DATUM: Color32 = Color32::from_rgb(0x5f, 0x74, 0x8a);
/// Hull shade when a face's layer is missing; layers carry their own colour.
pub const HULL_DEFAULT: Color32 = Color32::from_rgb(0x9f, 0xb4, 0xc7);
/// Refined-mesh wireframe.
pub const WIREFRAME: Color32 = Color32::from_rgb(0x7f, 0x9a, 0xb5);
/// Control net edges.
pub const CONTROL_NET: Color32 = Color32::from_rgb(0xe8, 0x9d, 0x3a);
/// Crease edges of the control net.
pub const CREASE: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
/// The control net and its creases while no point is selected: the same
/// hues, quiet enough that the hull under them is what the eye reads.
pub const CONTROL_NET_IDLE: Color32 = Color32::from_rgba_unmultiplied_const(0xe8, 0x9d, 0x3a, 0x66);
pub const CREASE_IDLE: Color32 = Color32::from_rgba_unmultiplied_const(0xe0, 0x5c, 0x5c, 0x80);
/// Control net edges, creases and points hidden behind the shaded hull.
pub const CONTROL_NET_HIDDEN: Color32 =
    Color32::from_rgba_unmultiplied_const(0xe8, 0x9d, 0x3a, 0x30);
pub const CREASE_HIDDEN: Color32 = Color32::from_rgba_unmultiplied_const(0xe0, 0x5c, 0x5c, 0x40);
pub const POINT_HIDDEN: Color32 = Color32::from_rgba_unmultiplied_const(0xcf, 0xd8, 0xe3, 0x48);
/// The active or hovered station highlighted across all views, and the rest
/// of the control net in the Body plan while station focus is on.
pub const STATION_HIGHLIGHT: Color32 = Color32::from_rgb(0x7d, 0xc6, 0xff);
pub const CONTROL_NET_FAINT: Color32 =
    Color32::from_rgba_unmultiplied_const(0xe8, 0x9d, 0x3a, 0x20);
/// Control points.
pub const POINT: Color32 = Color32::from_rgb(0xcf, 0xd8, 0xe3);
/// The picked control point and its outline.
pub const POINT_SELECTED: Color32 = Color32::from_rgb(0x5a, 0xd0, 0x6a);
pub const POINT_SELECTED_OUTLINE: Color32 = Color32::BLACK;
/// The first of two picked control points (Shift+click), ringed for the
/// Model menu's edge tools.
pub const POINT_PICKED: Color32 = POINT_SELECTED;
/// Outline of the control face the last click picked (Model > Split face).
pub const FACE_PICKED: Color32 = Color32::from_rgba_unmultiplied_const(0x5a, 0xd0, 0x6a, 0x90);
/// The world axis a locked drag follows: x red, y green, z blue.
pub const AXIS_X: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
pub const AXIS_Y: Color32 = Color32::from_rgb(0x5a, 0xd0, 0x6a);
pub const AXIS_Z: Color32 = Color32::from_rgb(0x5c, 0xa8, 0xf0);
/// Waterplane quad fill (premultiplied) and its border.
pub const WATERPLANE_FILL: Color32 = Color32::from_rgba_premultiplied(0x07, 0x10, 0x1b, 0x24);
pub const WATERPLANE_EDGE: Color32 = Color32::from_rgba_unmultiplied_const(0x5c, 0xa8, 0xf0, 0x80);
/// The design waterline traced on the hull.
pub const DWL: Color32 = Color32::from_rgb(0x5c, 0xa8, 0xf0);
/// Stations, buttocks and waterlines, and their curvature combs.
pub const SECTION_CURVE: Color32 = Color32::from_rgb(0x9a, 0xb8, 0xd4);
pub const COMB_SPIKE: Color32 = Color32::from_rgb(0x4b, 0x9c, 0x62);
pub const COMB_ENVELOPE: Color32 = Color32::from_rgb(0x63, 0xd4, 0x82);

// ---- tanks, compartments, windage --------------------------------------

pub const TANK_FILL: Color32 = Color32::from_rgba_unmultiplied_const(235, 160, 40, 25);
pub const TANK_LIQUID: Color32 = Color32::from_rgba_unmultiplied_const(60, 140, 240, 70);
pub const TANK_EDGE: Color32 = Color32::from_rgb(220, 150, 40);
pub const TANK_EDGE_SELECTED: Color32 = Color32::from_rgb(255, 200, 70);
pub const COMPARTMENT_FILL: Color32 = Color32::from_rgba_unmultiplied_const(120, 160, 200, 25);
pub const COMPARTMENT_EDGE: Color32 = Color32::from_rgb(120, 155, 190);
pub const COMPARTMENT_EDGE_SELECTED: Color32 = Color32::from_rgb(170, 210, 250);
pub const FLOODED_FILL: Color32 = Color32::from_rgba_unmultiplied_const(230, 60, 60, 70);
pub const FLOODED_EDGE: Color32 = Color32::from_rgb(220, 60, 60);
pub const FLOODED_EDGE_SELECTED: Color32 = Color32::from_rgb(255, 90, 90);
pub const WINDAGE_FILL: Color32 = Color32::from_rgba_unmultiplied_const(60, 200, 100, 25);
pub const WINDAGE_FILL_SELECTED: Color32 = Color32::from_rgba_unmultiplied_const(60, 200, 100, 50);
pub const WINDAGE_EDGE: Color32 = Color32::from_rgba_unmultiplied_const(70, 190, 110, 180);
pub const WINDAGE_EDGE_SELECTED: Color32 = Color32::from_rgb(120, 255, 160);
pub const WINDAGE_TEXT: Color32 = Color32::from_rgba_unmultiplied_const(180, 230, 190, 200);
pub const WINDAGE_TEXT_SELECTED: Color32 = Color32::from_rgb(220, 255, 230);
/// Dock highlights of the row selected in a viewport.
pub const HIGHLIGHT_TANK: Color32 = TANK_EDGE;
pub const HIGHLIGHT_COMPARTMENT: Color32 = FLOODED_EDGE;
pub const HIGHLIGHT_WINDAGE: Color32 = Color32::from_rgb(60, 200, 100);

// ---- plots -------------------------------------------------------------

pub const PLOT_AXIS: Color32 = Color32::from_rgb(0x4a, 0x5c, 0x70);
pub const PLOT_GRID: Color32 = Color32::from_rgb(0x1f, 0x29, 0x35);
pub const PLOT_TEXT: Color32 = TEXT_SECONDARY;
/// Criteria thresholds drawn dashed on GZ plots.
pub const PLOT_THRESHOLD: Color32 = Color32::from_rgb(0xa8, 0x84, 0x48);
/// Series colours, in the order curves are drawn.
pub const PLOT_SERIES: [Color32; 6] = [
    Color32::from_rgb(0x5c, 0xa8, 0xf0),
    Color32::from_rgb(0x5a, 0xd0, 0x6a),
    Color32::from_rgb(0xe8, 0x9d, 0x3a),
    Color32::from_rgb(0xe0, 0x5c, 0x5c),
    Color32::from_rgb(0xc0, 0x7c, 0xeb),
    Color32::from_rgb(0x40, 0xd0, 0xd0),
];

// ---- semantics ---------------------------------------------------------

/// A criterion met, an equilibrium found.
pub const OK: Color32 = Color32::from_rgb(0x5a, 0xd0, 0x6a);
/// A criterion failed, an error: the same red in every dock.
pub const FAIL: Color32 = Color32::from_rgb(0xe0, 0x5c, 0x5c);
/// Something to look at before trusting a result.
pub const WARN: Color32 = Color32::from_rgb(0xe8, 0x9d, 0x3a);
/// A result older than the last edit.
pub const STALE: Color32 = Color32::from_rgb(0x8e, 0x9e, 0xae);
/// The opacity a stale result is drawn at (with the "out of date" chip).
pub const STALE_OPACITY: f32 = 0.55;
/// A design target (requirements), as opposed to an actual.
pub const TARGET: Color32 = Color32::from_rgb(0x40, 0xd0, 0xd0);
/// Focus and the active viewport.
pub const ACCENT: Color32 = FRAME_ACTIVE;

/// Colours a new layer takes (`.fef` `0xRRGGBB`), in order: the first one no
/// layer uses yet. Mid-tone and well apart, so neighbouring layers read as
/// different under the viewport's shading.
pub const LAYER_PALETTE: [i32; 8] = [
    0x2f_7f_d0, // sea blue
    0xb0_4a_3c, // antifouling red
    0xc8_8f_3a, // teak
    0x4f_a8_6a, // green
    0x8a_6c_c8, // violet
    0x3a_a8_b0, // teal
    0xc8_6a_9a, // rose
    0x9a_a0_a8, // grey
];

/// The palette colour for a new layer: the first [`LAYER_PALETTE`] entry
/// none of `used` has, or the next one round when every entry is taken.
pub fn next_layer_colour(used: &[i32]) -> i32 {
    LAYER_PALETTE
        .iter()
        .copied()
        .find(|c| !used.contains(c))
        .unwrap_or(LAYER_PALETTE[used.len() % LAYER_PALETTE.len()])
}

/// A `.fef` layer colour (`0xRRGGBB`) as a paint colour.
pub fn layer_colour(rgb: i32) -> Color32 {
    let c = rgb.max(0) as u32;
    Color32::from_rgb(
        ((c >> 16) & 0xff) as u8,
        ((c >> 8) & 0xff) as u8,
        (c & 0xff) as u8,
    )
}

// ---- type scale and spacing --------------------------------------------

/// Headings.
pub const HEADING_SIZE: f32 = 15.0;
/// Corner radius of buttons and fields, and of windows and menus.
pub const RADIUS: u8 = 4;
pub const WINDOW_RADIUS: u8 = 8;
/// Body text, labels and quantity names.
pub const BODY_SIZE: f32 = 13.0;
/// Buttons.
pub const BUTTON_SIZE: f32 = 13.0;
/// Small print. Never a quantity's name.
pub const SMALL_SIZE: f32 = 11.0;
/// Numeric values.
pub const MONO_SIZE: f32 = 13.0;
/// Plot and viewport captions, tick labels and legends.
pub const CAPTION_SIZE: f32 = 11.0;
/// Width of the unit column in numeric readouts.
pub const UNIT_WIDTH: f32 = 32.0;

/// The font of plot and viewport captions: 11 pt monospace.
pub fn caption_font() -> FontId {
    FontId::monospace(CAPTION_SIZE)
}

/// The fonts compiled into the program, unmodified from DejaVu 2.37 (`seacraft_core::fonts`, with
/// their licence). Each family falls back on the other, so a symbol only one of them has still draws.
pub fn fonts() -> egui::FontDefinitions {
    const SANS: &str = "DejaVu Sans";
    const MONO: &str = "DejaVu Sans Mono";
    let mut fonts = egui::FontDefinitions::empty();
    for (name, bytes) in [
        (SANS, seacraft_core::fonts::DEJAVU_SANS),
        (MONO, seacraft_core::fonts::DEJAVU_SANS_MONO),
    ] {
        fonts.font_data.insert(
            name.to_owned(),
            std::sync::Arc::new(egui::FontData::from_static(bytes)),
        );
    }
    fonts.families.insert(
        FontFamily::Proportional,
        vec![SANS.to_owned(), MONO.to_owned()],
    );
    fonts.families.insert(
        FontFamily::Monospace,
        vec![MONO.to_owned(), SANS.to_owned()],
    );
    fonts
}

/// Apply the type scale, spacing and surface colours to `style`.
pub fn style(style: &mut egui::Style) {
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(HEADING_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Body,
            FontId::new(BODY_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Button,
            FontId::new(BUTTON_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(SMALL_SIZE, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(MONO_SIZE, FontFamily::Monospace),
        ),
    ]
    .into();
    // Spacing on a 4 px grid.
    let spacing = &mut style.spacing;
    spacing.item_spacing = egui::vec2(8.0, 4.0);
    spacing.indent = 16.0;
    spacing.window_margin = egui::Margin::same(8);
    spacing.menu_margin = egui::Margin::same(4);

    spacing.button_padding = egui::vec2(8.0, 3.0);
    spacing.interact_size.y = 22.0;

    let visuals = &mut style.visuals;
    visuals.panel_fill = PANEL;
    visuals.window_fill = WINDOW;
    visuals.window_stroke = Stroke::new(1.0, OUTLINE);
    visuals.window_corner_radius = CornerRadius::same(WINDOW_RADIUS);
    visuals.menu_corner_radius = CornerRadius::same(WINDOW_RADIUS - 2);
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = FAIL;
    visuals.hyperlink_color = AXIS_Z;
    visuals.faint_bg_color = TABLE_STRIPE;
    // Text fields: sunk, with the resting outline (egui draws it from
    // `widgets.inactive.bg_stroke`) and the accent when focused.
    visuals.extreme_bg_color = FIELD;
    visuals.selection.bg_fill = SELECTED;
    visuals.selection.stroke = Stroke::new(1.0, TEXT_ON_ACCENT);

    let radius = CornerRadius::same(RADIUS);
    let w = &mut visuals.widgets;
    // Labels, separators and frames. Headings and values ask for
    // TEXT_PRIMARY themselves; a plain label is a quantity's name.
    w.noninteractive.bg_fill = PANEL;
    w.noninteractive.weak_bg_fill = PANEL;
    w.noninteractive.bg_stroke = Stroke::new(1.0, FRAME);
    w.noninteractive.fg_stroke = Stroke::new(1.0, TEXT_LABEL);
    w.noninteractive.corner_radius = radius;
    for (v, fill, outline, text) in [
        (&mut w.inactive, RAISED, OUTLINE, TEXT_PRIMARY),
        (&mut w.hovered, RAISED_HOVER, OUTLINE_HOVER, TEXT_ON_ACCENT),
        (&mut w.active, RAISED_ACTIVE, ACCENT, TEXT_ON_ACCENT),
        (&mut w.open, RAISED_HOVER, OUTLINE_HOVER, TEXT_PRIMARY),
    ] {
        v.bg_fill = fill;
        v.weak_bg_fill = fill;
        v.bg_stroke = Stroke::new(1.0, outline);
        v.fg_stroke = Stroke::new(1.0, text);
        v.corner_radius = radius;
        v.expansion = 0.0;
    }
}

/// A section heading: the brightest text in a dock, so the eye finds the
/// section before its contents.
pub fn heading(ui: &mut egui::Ui, text: impl Into<String>) -> egui::Response {
    ui.label(egui::RichText::new(text).heading().color(TEXT_PRIMARY))
}

/// Runs `add` with the visuals of a value field: sunk and outlined, where a
/// button is raised. egui draws a `DragValue` as a button, so without this
/// a number to edit and an action to press look the same.
pub fn as_field<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let w = &mut ui.visuals_mut().widgets;
        for (v, fill) in [
            (&mut w.inactive, FIELD),
            (&mut w.hovered, FIELD_HOVER),
            (&mut w.active, FIELD_HOVER),
        ] {
            v.bg_fill = fill;
            v.weak_bg_fill = fill;
        }
        add(ui)
    })
    .inner
}

/// A numeric field: an `egui::DragValue` drawn as a field (see [`as_field`]).
/// It forwards the builder methods the app uses.
#[must_use = "add it with `ui.add`"]
pub struct Num<'a>(egui::DragValue<'a>);

/// A numeric field on `value`.
pub fn num<N: egui::emath::Numeric>(value: &mut N) -> Num<'_> {
    Num(egui::DragValue::new(value))
}

impl<'a> Num<'a> {
    pub fn speed(self, speed: impl Into<f64>) -> Self {
        Self(self.0.speed(speed))
    }
    pub fn range<N: egui::emath::Numeric>(self, range: std::ops::RangeInclusive<N>) -> Self {
        Self(self.0.range(range))
    }
    pub fn prefix(self, prefix: impl egui::IntoAtoms<'a>) -> Self {
        Self(self.0.prefix(prefix))
    }
    pub fn suffix(self, suffix: impl egui::IntoAtoms<'a>) -> Self {
        Self(self.0.suffix(suffix))
    }
    pub fn fixed_decimals(self, n: usize) -> Self {
        Self(self.0.fixed_decimals(n))
    }
    pub fn max_decimals(self, n: usize) -> Self {
        Self(self.0.max_decimals(n))
    }
    pub fn update_while_editing(self, update: bool) -> Self {
        Self(self.0.update_while_editing(update))
    }
}

impl egui::Widget for Num<'_> {
    fn ui(self, ui: &mut egui::Ui) -> egui::Response {
        as_field(ui, |ui| ui.add(self.0))
    }
}

/// The primary button of a dialog: filled with the accent, one per dialog.
pub fn primary_button<'a>(text: impl Into<String>) -> egui::Button<'a> {
    egui::Button::new(egui::RichText::new(text).color(TEXT_ON_ACCENT)).fill(PRIMARY)
}

/// Runs `add` with a switch's visuals: a `selectable_label` that is on is
/// drawn raised and bright, not in the selection colour.
pub fn as_switch<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.scope(|ui| {
        let v = ui.visuals_mut();
        v.selection.bg_fill = SWITCH_ON;
        v.selection.stroke = Stroke::new(1.0, TEXT_ON_ACCENT);
        add(ui)
    })
    .inner
}

/// Runs `add` inside the track of a segmented control: one sunk, outlined
/// box whose `selectable_label`s are the choices, so "one of these" does
/// not read as a row of separate buttons.
pub fn segmented<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(FIELD)
        .stroke(Stroke::new(1.0, OUTLINE))
        .corner_radius(CornerRadius::same(RADIUS))
        .inner_margin(egui::Margin::same(1))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.spacing_mut().button_padding = egui::vec2(7.0, 2.0);
                add(ui)
            })
            .inner
        })
        .inner
}

/// The side of an icon button, in points.
pub const ICON_BUTTON: f32 = 20.0;

/// The icons the app draws itself: the embedded fonts carry no symbol set,
/// and a stroked icon stays sharp at any scale.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Icon {
    /// Frame the model: four corner brackets.
    Fit,
    /// Two arrows pointing out to opposite corners.
    Maximise,
    /// Two arrows pointing in from opposite corners.
    Restore,
    /// A double chevron pointing left: the right dock grows.
    Widen,
    /// A double chevron pointing right: the right dock shrinks.
    Narrow,
}

impl Icon {
    /// Stroke the icon centred in `rect`.
    fn paint(self, painter: &egui::Painter, rect: egui::Rect, colour: Color32) {
        let c = rect.center();
        let stroke = Stroke::new(1.5, colour);
        let at = |x: f32, y: f32| c + egui::vec2(x, y);
        let line = |a: egui::Pos2, b: egui::Pos2| painter.line_segment([a, b], stroke);
        match self {
            Icon::Fit => {
                for (sx, sy) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                    line(at(5.0 * sx, 5.0 * sy), at(1.5 * sx, 5.0 * sy));
                    line(at(5.0 * sx, 5.0 * sy), at(5.0 * sx, 1.5 * sy));
                }
            }
            Icon::Maximise | Icon::Restore => {
                // Each arrow runs along the diagonal; its head sits at the
                // corner end (maximise) or at the centre end (restore).
                for s in [-1.0_f32, 1.0] {
                    let (tip, tail) = if self == Icon::Maximise {
                        (5.0 * s, 1.5 * s)
                    } else {
                        (1.5 * s, 5.0 * s)
                    };
                    let back = if self == Icon::Maximise { -s } else { s };
                    line(at(tail, -tail), at(tip, -tip));
                    line(at(tip, -tip), at(tip + 3.5 * back, -tip));
                    line(at(tip, -tip), at(tip, -tip - 3.5 * back));
                }
            }
            Icon::Widen | Icon::Narrow => {
                let s = if self == Icon::Widen { 1.0 } else { -1.0 };
                for x in [-2.5_f32, 2.5] {
                    line(at(x + 2.0 * s, -4.0), at(x - 2.0 * s, 0.0));
                    line(at(x - 2.0 * s, 0.0), at(x + 2.0 * s, 4.0));
                }
            }
        }
    }
}

/// How an icon button sits on what is behind it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum IconStyle {
    /// In a dock or toolbar: a raised button.
    Raised,
    /// Over a viewport: the caption backing until the pointer is on it.
    Hud,
}

/// An icon button in `rect`. `on` draws it as a switch that is on.
pub fn icon_button_at(
    ui: &mut egui::Ui,
    rect: egui::Rect,
    icon: Icon,
    style: IconStyle,
    on: bool,
) -> egui::Response {
    let response = ui.allocate_rect(rect, egui::Sense::click());
    if ui.is_rect_visible(rect) {
        let v = ui.style().interact(&response);
        // Keyboard focus is not idle: the focused button must show which
        // action Space or Enter will press.
        let idle =
            !(response.hovered() || response.is_pointer_button_down_on() || response.has_focus());
        let (fill, outline, colour) = match (style, idle, on) {
            (_, true, true) => (SWITCH_ON, Stroke::NONE, TEXT_ON_ACCENT),
            (IconStyle::Hud, true, false) => (HUD_BACKING, Stroke::NONE, TEXT_SECONDARY),
            _ => (v.bg_fill, v.bg_stroke, v.fg_stroke.color),
        };
        ui.painter().rect(
            rect,
            CornerRadius::same(RADIUS),
            fill,
            outline,
            egui::StrokeKind::Inside,
        );
        icon.paint(ui.painter(), rect, colour);
    }
    response
}

/// An icon button laid out like any other widget.
pub fn icon_button(ui: &mut egui::Ui, icon: Icon, on: bool) -> egui::Response {
    let (rect, _) = ui.allocate_exact_size(egui::Vec2::splat(ICON_BUTTON), egui::Sense::hover());
    icon_button_at(ui, rect, icon, IconStyle::Raised, on)
}

/// Apply the theme to every style of `ctx` (dark and light).
pub fn apply(ctx: &egui::Context) {
    ctx.set_fonts(fonts());
    ctx.all_styles_mut(style);
}

/// Whether `style` already carries the theme's type scale.
pub fn is_themed(style: &egui::Style) -> bool {
    let size = |s: TextStyle| style.text_styles.get(&s).map(|f| f.size);
    size(TextStyle::Heading) == Some(HEADING_SIZE) && size(TextStyle::Monospace) == Some(MONO_SIZE)
}

/// Theme the context (once) and the current `ui`, so a context the app did
/// not create (a headless test's) still gets the type scale on its first
/// frame.
pub fn ensure(ui: &mut egui::Ui) {
    if !is_themed(&ui.ctx().global_style()) {
        apply(ui.ctx());
    }
    if !is_themed(ui.style()) {
        style(ui.style_mut());
    }
}

/// WCAG 2 relative luminance of an opaque colour.
pub fn relative_luminance(c: Color32) -> f64 {
    let lin = |v: u8| {
        let s = v as f64 / 255.0;
        if s <= 0.04045 {
            s / 12.92
        } else {
            ((s + 0.055) / 1.055).powf(2.4)
        }
    };
    0.2126 * lin(c.r()) + 0.7152 * lin(c.g()) + 0.0722 * lin(c.b())
}

/// WCAG 2 contrast ratio between two opaque colours (≥ 1).
pub fn contrast_ratio(a: Color32, b: Color32) -> f64 {
    let (la, lb) = (relative_luminance(a), relative_luminance(b));
    (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
}

/// `base` scaled by `shade` in red, green and blue, and always opaque.
/// `Color32::gamma_multiply` scales alpha too, which would make a shaded
/// surface see-through.
pub fn shaded(base: Color32, shade: f32) -> Color32 {
    let scale = |c: u8| (c as f32 * shade + 0.5) as u8;
    Color32::from_rgb(scale(base.r()), scale(base.g()), scale(base.b()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shading_scales_rgb_and_keeps_alpha_opaque() {
        let c = Color32::from_rgb(100, 200, 50);
        assert_eq!(shaded(c, 0.5), Color32::from_rgb(50, 100, 25));
        assert_eq!(shaded(c, 0.5).a(), 255);
        assert_eq!(shaded(c, 1.0), c);
    }

    /// The fill of a HUD icon button drawn in a frame of its own, with or
    /// without the keyboard focus.
    fn hud_button_fill(focused: bool) -> Color32 {
        let ctx = egui::Context::default();
        let rect =
            egui::Rect::from_min_size(egui::pos2(20.0, 20.0), egui::Vec2::splat(ICON_BUTTON));
        let mut fill = None;
        for _ in 0..2 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                let response = icon_button_at(ui, rect, Icon::Fit, IconStyle::Hud, false);
                if focused {
                    response.request_focus();
                }
            });
            // Nothing uploads the fonts in a headless pass; a dropped delta panics.
            out.textures_delta.clear();
            fill = out.shapes.iter().find_map(|c| match &c.shape {
                egui::Shape::Rect(r) if r.rect == rect => Some(r.fill),
                _ => None,
            });
        }
        fill.expect("the button's backing")
    }

    #[test]
    fn a_focused_icon_button_does_not_look_idle() {
        assert_eq!(hud_button_fill(false), HUD_BACKING);
        assert_ne!(
            hud_button_fill(true),
            HUD_BACKING,
            "the keyboard focus must show on the button"
        );
    }

    /// Every GUI source except this one. A new module must be added here.
    const SOURCES: [(&str, &str); 25] = [
        ("analysis_views.rs", include_str!("analysis_views.rs")),
        ("app.rs", include_str!("app.rs")),
        ("autosave.rs", include_str!("autosave.rs")),
        ("camera.rs", include_str!("camera.rs")),
        ("commands.rs", include_str!("commands.rs")),
        ("connected_presets.rs", include_str!("connected_presets.rs")),
        ("control.rs", include_str!("control.rs")),
        ("gpu_hull/mod.rs", include_str!("gpu_hull/mod.rs")),
        ("gpu_hull/geometry.rs", include_str!("gpu_hull/geometry.rs")),
        ("gpu_hull/pipeline.rs", include_str!("gpu_hull/pipeline.rs")),
        ("gpu_hull/view.rs", include_str!("gpu_hull/view.rs")),
        ("history.rs", include_str!("history.rs")),
        ("jobs.rs", include_str!("jobs.rs")),
        ("lib.rs", include_str!("lib.rs")),
        ("main.rs", include_str!("main.rs")),
        ("messages.rs", include_str!("messages.rs")),
        ("model_menu.rs", include_str!("model_menu.rs")),
        ("new_design.rs", include_str!("new_design.rs")),
        ("orientation.rs", include_str!("orientation.rs")),
        ("panels.rs", include_str!("panels.rs")),
        ("picking.rs", include_str!("picking.rs")),
        ("recent.rs", include_str!("recent.rs")),
        ("render.rs", include_str!("render.rs")),
        ("selection.rs", include_str!("selection.rs")),
        ("visibility.rs", include_str!("visibility.rs")),
    ];

    #[test]
    fn colour_literals_live_only_in_the_theme() {
        // Built from pieces so this file's own test does not match itself.
        let needles = [
            "from_rgb(",
            "from_rgba_unmultiplied(",
            "from_rgba_unmultiplied_const(",
            "from_rgb_additive(",
            "from_additive_luminance(",
            "from_rgba_premultiplied(",
            "from_gray(",
            "from_black_alpha(",
            "from_white_alpha(",
            "from_hex(",
        ]
        .map(|n| format!("Color32::{n}"));
        let mut found = Vec::new();
        for (file, src) in SOURCES {
            for (i, line) in src.lines().enumerate() {
                if needles.iter().any(|n| line.contains(n.as_str())) {
                    found.push(format!("{file}:{}: {}", i + 1, line.trim()));
                }
            }
        }
        assert!(
            found.is_empty(),
            "colour literals outside theme.rs:\n{}",
            found.join("\n")
        );
        // The module list above is complete.
        let lib = include_str!("lib.rs");
        let modules = lib.lines().filter(|l| l.starts_with("pub mod ")).count();
        // Files inside a module directory, beside its `mod.rs`.
        let nested = SOURCES.iter().filter(|(f, _)| f.contains('/')).count()
            - SOURCES
                .iter()
                .filter(|(f, _)| f.ends_with("/mod.rs"))
                .count();
        assert_eq!(
            modules,
            SOURCES.len() - nested - 2 + 1,
            "a module is missing from SOURCES"
        );
    }

    #[test]
    fn the_type_scale_reaches_the_style() {
        let ctx = egui::Context::default();
        apply(&ctx);
        let style = ctx.global_style();
        let size = |s: TextStyle| style.text_styles[&s].size;
        assert!(
            size(TextStyle::Body) >= 12.0,
            "body {}",
            size(TextStyle::Body)
        );
        assert!(size(TextStyle::Button) >= 12.0);
        assert!(
            size(TextStyle::Small) >= 11.0,
            "small {}",
            size(TextStyle::Small)
        );
        assert_eq!(size(TextStyle::Heading), 15.0);
        assert_eq!(size(TextStyle::Monospace), 13.0);
        assert!(is_themed(&style));
        assert_eq!(caption_font(), FontId::monospace(11.0));
        // Spacing on the 4 px grid.
        let s = &style.spacing;
        assert_eq!(s.item_spacing.x % 4.0, 0.0);
        assert_eq!(s.item_spacing.y % 4.0, 0.0);
        // Both themes, so a system light/dark switch does not lose it.
        assert!(is_themed(&ctx.style_of(egui::Theme::Light)));
        assert!(is_themed(&ctx.style_of(egui::Theme::Dark)));
    }

    #[test]
    fn the_grid_reads_against_the_canvas() {
        // R5-5: the minor lines went quiet (≥ 2:1 before; then ≈ 1.4:1) so
        // they stop competing with the hull; majors and datums carry the
        // structure. R6-1, "a little quieter": one notch down for both,
        // minor ≈ 1.25:1 and major ≈ 1.7:1, still 0.4 apart; datums as they
        // were.
        let minor = contrast_ratio(GRID_MINOR, CANVAS);
        let major = contrast_ratio(GRID_MAJOR, CANVAS);
        let datum = contrast_ratio(GRID_DATUM, CANVAS);
        assert!((1.2..=1.3).contains(&minor), "minor grid {minor:.2}:1");
        assert!(
            (1.6..=1.8).contains(&major) && major > minor + 0.4,
            "major {major:.2}:1 vs minor {minor:.2}:1"
        );
        assert!(
            datum >= 3.0 && datum > major + 1.0,
            "datum {datum:.2}:1 vs major {major:.2}:1"
        );
        // Every grid tier stays below the wireframe, so the hull reads first.
        assert!(datum < contrast_ratio(WIREFRAME, CANVAS));
        // The WCAG formula itself: white on black is 21:1.
        assert!((contrast_ratio(Color32::WHITE, Color32::BLACK) - 21.0).abs() < 1e-9);
    }

    #[test]
    fn text_tokens_are_ordered_and_legible_on_the_panel() {
        let p = contrast_ratio(TEXT_PRIMARY, PANEL);
        let s = contrast_ratio(TEXT_SECONDARY, PANEL);
        let m = contrast_ratio(TEXT_MUTED, PANEL);
        assert!(p > s && s > m, "{p:.2} > {s:.2} > {m:.2}");
        assert!(m >= 3.0, "muted text {m:.2}:1");
        assert!(s >= 4.5, "secondary text {s:.2}:1");
    }

    #[test]
    fn a_new_layer_takes_the_first_unused_palette_colour() {
        let p = LAYER_PALETTE;
        assert_eq!(next_layer_colour(&[]), p[0]);
        assert_eq!(next_layer_colour(&[p[0], p[1], 0x123456]), p[2]);
        // The palette is distinct and every entry a valid 0xRRGGBB.
        for (i, a) in p.iter().enumerate() {
            assert!((0..=0xff_ff_ff).contains(a));
            assert!(p[i + 1..].iter().all(|b| b != a));
        }
        // All taken: round the palette again rather than fail.
        assert!(p.contains(&next_layer_colour(&p)));
    }

    /// Every non-ASCII character outside a comment in `dir`, recursively, with where it is.
    fn characters_in(dir: &std::path::Path, found: &mut Vec<(char, String)>) {
        for entry in std::fs::read_dir(dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                characters_in(&path, found);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let text = std::fs::read_to_string(&path).unwrap();
                for (i, line) in text.lines().enumerate() {
                    let code = line.split("//").next().unwrap_or("");
                    for c in code.chars().filter(|c| !c.is_ascii()) {
                        found.push((c, format!("{}:{}", path.display(), i + 1)));
                    }
                }
            }
        }
    }

    /// A machine without a suitable system font must still draw all of SeaCraft's own text.
    #[test]
    fn the_embedded_fonts_draw_every_character_the_program_writes() {
        assert!(
            egui::FontDefinitions::builtin_font_names().is_empty(),
            "egui's default fonts are back in the build"
        );
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
        let mut found = Vec::new();
        characters_in(&root.join("src"), &mut found);
        characters_in(&root.join("../seacraft-core/src"), &mut found);
        assert!(found.iter().any(|(c, _)| *c == '°'), "the walk found text");

        let ctx = egui::Context::default();
        apply(&ctx);
        // New fonts take effect at the start of the next pass. A headless context has no system
        // font provider, so only the embedded fonts can answer below.
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let missing: Vec<_> = ctx.fonts_mut(|fonts| {
            found
                .iter()
                .filter(|(c, _)| {
                    !fonts.has_glyph(&FontId::proportional(BODY_SIZE), *c)
                        || !fonts.has_glyph(&FontId::monospace(MONO_SIZE), *c)
                })
                .cloned()
                .collect()
        });
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    #[test]
    fn layer_colours_unpack_rgb() {
        assert_eq!(
            layer_colour(0x12_34_56),
            Color32::from_rgb(0x12, 0x34, 0x56)
        );
        assert_eq!(layer_colour(-1), Color32::from_rgb(0, 0, 0));
    }

    #[test]
    fn hidden_net_tokens_are_quieter_than_idle_and_distinct_from_canvas() {
        for (hidden, idle, full) in [
            (CONTROL_NET_HIDDEN, CONTROL_NET_IDLE, CONTROL_NET),
            (CREASE_HIDDEN, CREASE_IDLE, CREASE),
            (POINT_HIDDEN, POINT, POINT),
        ] {
            assert_ne!(hidden, CANVAS);
            assert_ne!(hidden, idle);
            assert_ne!(hidden, full);
            assert!(hidden.a() > 0 && hidden.a() < idle.a());
        }
    }
}
