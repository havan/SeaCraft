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

use eframe::egui::{self, Color32, FontFamily, FontId, TextStyle};

// ---- surfaces ----------------------------------------------------------

/// Viewport background: dark slate.
pub const CANVAS: Color32 = Color32::from_rgb(0x13, 0x19, 0x21);
/// Dock and window background (egui's dark panel grey, made explicit).
pub const PANEL: Color32 = Color32::from_gray(27);
/// Every other row of an engineering table, a shade above the panel.
pub const TABLE_STRIPE: Color32 = Color32::from_gray(33);
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
/// Control points.
pub const POINT: Color32 = Color32::from_rgb(0xcf, 0xd8, 0xe3);
/// The picked control point and its outline.
pub const POINT_SELECTED: Color32 = Color32::from_rgb(0x5a, 0xd0, 0x6a);
pub const POINT_SELECTED_OUTLINE: Color32 = Color32::BLACK;
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
/// Body text, labels and quantity names.
pub const BODY_SIZE: f32 = 12.5;
/// Buttons.
pub const BUTTON_SIZE: f32 = 12.5;
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

    let visuals = &mut style.visuals;
    visuals.panel_fill = PANEL;
    visuals.window_fill = PANEL;
    visuals.warn_fg_color = WARN;
    visuals.error_fg_color = FAIL;
}

/// Apply the theme to every style of `ctx` (dark and light).
pub fn apply(ctx: &egui::Context) {
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Every GUI source except this one. A new module must be added here.
    const SOURCES: [(&str, &str); 12] = [
        ("app.rs", include_str!("app.rs")),
        ("camera.rs", include_str!("camera.rs")),
        ("commands.rs", include_str!("commands.rs")),
        ("control.rs", include_str!("control.rs")),
        ("history.rs", include_str!("history.rs")),
        ("jobs.rs", include_str!("jobs.rs")),
        ("lib.rs", include_str!("lib.rs")),
        ("main.rs", include_str!("main.rs")),
        ("new_design.rs", include_str!("new_design.rs")),
        ("panels.rs", include_str!("panels.rs")),
        ("picking.rs", include_str!("picking.rs")),
        ("render.rs", include_str!("render.rs")),
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
        assert_eq!(
            modules,
            SOURCES.len() - 2 + 1,
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

    #[test]
    fn layer_colours_unpack_rgb() {
        assert_eq!(
            layer_colour(0x12_34_56),
            Color32::from_rgb(0x12, 0x34, 0x56)
        );
        assert_eq!(layer_colour(-1), Color32::from_rgb(0, 0, 0));
    }
}
