//! The far-field wave pattern in the window: the settings the Wave pattern view edits, the
//! computation a `Job` runs off the window thread (the field and every readout at one speed), and the
//! image the view draws.
//!
//! The window draws dark calm water with crest and trough tints (`window_rgb`, from `theme.rs`), by
//! the maintainer's ruling of 2026-11-19; the PNG and SVG exports and the command line keep the core's
//! light map (`seacraft_core::wave_pattern::diverging_rgb`), which suits paper. This reverses PR #10's
//! "the window and the command line draw the same colours". Dry cells are left clear.

use crate::theme;
use eframe::egui;
use seacraft_core::contour::{self, Segment};
use seacraft_core::mesh::Surface;
use seacraft_core::michell::{
    michell_wave_resistance_from_grid, MichellGrid, MichellOptions, MichellResult,
};
use seacraft_core::resistance::{GRAVITY, KNOT};
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_core::wave_pattern::{
    contour_levels, regime, wash_cut, wave_field, Regime, WashHeight, WaveCut, WaveField,
    WavePatternOptions,
};
use std::hash::{Hash, Hasher};

/// The ship speed the pattern is computed at.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveSpeed {
    Knots(f64),
    /// Froude number on the waterline length.
    Froude(f64),
}

impl WaveSpeed {
    /// The speed in m/s on a hull of waterline length `length_wl` (m).
    pub fn m_s(self, length_wl: f64) -> f64 {
        match self {
            WaveSpeed::Knots(kn) => kn * KNOT,
            WaveSpeed::Froude(f) => f * (GRAVITY * length_wl).sqrt(),
        }
    }

    pub fn is_knots(self) -> bool {
        matches!(self, WaveSpeed::Knots(_))
    }

    /// The same speed in knots (`knots`) or as a Froude number, on a hull of waterline length
    /// `length_wl` (m).
    pub fn in_unit(self, knots: bool, length_wl: f64) -> WaveSpeed {
        match (self, knots) {
            (WaveSpeed::Froude(_), true) => WaveSpeed::Knots(self.m_s(length_wl) / KNOT),
            (WaveSpeed::Knots(_), false) => {
                WaveSpeed::Froude(self.m_s(length_wl) / (GRAVITY * length_wl).sqrt())
            }
            _ => self,
        }
    }
}

/// Where the field is computed around the hull.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveDomain {
    /// Ahead 0.5 L, behind 3 L, each side the larger of 1.5 L and 1.2 × the wash distance, so the
    /// wash line is always on the map.
    Auto,
    /// Metres ahead of the bow, behind the stern and each side of the centreline.
    Manual {
        ahead_m: f64,
        behind_m: f64,
        side_m: f64,
    },
}

/// What the Wave pattern view asks of the wave pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveSettings {
    pub speed: WaveSpeed,
    pub domain: WaveDomain,
    /// Demihull centreline separation (m); `None` for a monohull.
    pub cat_spacing_m: Option<f64>,
    /// Colour clip (m); `None` takes the field's largest |ζ|.
    pub clip_m: Option<f64>,
    /// Contour levels each side of calm; 0 draws none.
    pub contours: usize,
    /// Lateral distance of the wash line (m); `None` takes one waterline length, rounded to 0.5 m.
    pub wash_distance_m: Option<f64>,
    /// Draw the Kelvin wedge on the map (drawing only, not in the key).
    pub kelvin_wedge: bool,
    /// Draw faint contours on Plan view (drawing only, not in the key).
    pub plan_contours: bool,
}

impl Default for WaveSettings {
    fn default() -> Self {
        Self {
            speed: WaveSpeed::Froude(0.30),
            domain: WaveDomain::Auto,
            cat_spacing_m: None,
            clip_m: None,
            contours: 6,
            wash_distance_m: None,
            kelvin_wedge: true,
            plan_contours: false,
        }
    }
}

/// The domain `(ahead, behind, side)` in waterline lengths of `length_wl` (m), for a wash line at
/// `wash_m` (m).
pub fn domain_l(settings: &WaveSettings, length_wl: f64, wash_m: f64) -> (f64, f64, f64) {
    match settings.domain {
        WaveDomain::Auto => (0.5, 3.0, (1.2 * wash_m / length_wl).max(1.5)),
        WaveDomain::Manual {
            ahead_m,
            behind_m,
            side_m,
        } => (
            ahead_m / length_wl,
            behind_m / length_wl,
            side_m / length_wl,
        ),
    }
}

/// The wash line's distance (m): the setting, or one waterline length rounded to the nearest 0.5 m,
/// and never inside the hull (at least `B_WL / 2 + 0.05 L_WL`).
pub fn wash_distance(settings: &WaveSettings, length_wl: f64, beam_wl: f64) -> f64 {
    let d = settings
        .wash_distance_m
        .filter(|d| d.is_finite())
        .unwrap_or_else(|| (2.0 * length_wl).round() / 2.0);
    d.max(0.5 * beam_wl + 0.05 * length_wl)
}

/// The window's colour map at `t` = ζ / clip in `[-1, 1]`: dark calm water, blending to the trough
/// tint below and the crest tint above.
pub fn window_rgb(t: f64) -> [u8; 3] {
    let rgb = |c: egui::Color32| [c.r(), c.g(), c.b()];
    if t.is_nan() {
        return rgb(theme::WAVE_CALM);
    }
    let t = t.clamp(-1.0, 1.0);
    let (from, to, w) = if t < 0.0 {
        (rgb(theme::WAVE_CALM), rgb(theme::WAVE_TROUGH), -t)
    } else {
        (rgb(theme::WAVE_CALM), rgb(theme::WAVE_CREST), t)
    };
    std::array::from_fn(|i| (from[i] as f64 + (to[i] as f64 - from[i] as f64) * w).round() as u8)
}

/// The demihull spacing offered when "Catamaran" is ticked: the usual 0.3 L, but never less than
/// twice the waterline beam, so the demihulls do not overlap; 3 m before the hull is measured.
pub fn catamaran_guess(length_wl: Option<f64>, beam_wl: Option<f64>) -> f64 {
    let l = length_wl.filter(|l| *l > 0.0).map(|l| 0.3 * l);
    let b = beam_wl.filter(|b| *b > 0.0).map(|b| 2.0 * b);
    match (l, b) {
        (None, None) => 3.0,
        (l, b) => l.unwrap_or(0.0).max(b.unwrap_or(0.0)).max(0.1),
    }
}

/// A computed pattern at one speed and everything the view draws and reads out from it.
#[derive(Debug, Clone)]
pub struct WaveResult {
    pub field: WaveField,
    pub image: egui::ColorImage,
    /// The colour clip used (m).
    pub clip_m: f64,
    /// `(level, segments)` in ascending level, none at calm.
    pub contours: Vec<(f64, Vec<Segment>)>,
    /// The wash line: a longitudinal cut at `y = -wash_distance_m` over the field's x range, 401
    /// samples or more (four to a transverse wave at low speed).
    pub wash_cut: WaveCut,
    pub wash: Option<WashHeight>,
    /// The wash distance used (m), after clamping clear of the hull.
    pub wash_distance_m: f64,
    pub regime: Regime,
    /// Michell's wave resistance at this speed (of the pair when a catamaran spacing is set).
    pub drag_here: MichellResult,
    /// The pair's wave resistance over twice one demihull's at this speed; catamaran only.
    pub interference: Option<f64>,
    /// `(spacing m, interference)` over the spacing range at this speed; empty for a monohull.
    pub spacing_scan: Vec<(f64, f64)>,
    pub length_wl: f64,
    pub beam_wl: f64,
    pub draft: f64,
    /// The catamaran spacing it was computed for (m).
    pub cat_spacing_m: Option<f64>,
    /// The field's warnings, each once.
    pub warnings: Vec<String>,
}

/// The fewest samples along the wash line (more at a slow speed: `wash_samples`).
const WASH_LINE_SAMPLES: usize = 401;
/// Spacings in the catamaran spacing scan.
const SPACING_SCAN_POINTS: usize = 40;

/// The spacing scan's range (m): from the clearance `max(2 B, 0.05 L)` to `max(L, 2 B + 0.1 L)`.
pub fn spacing_range(length_wl: f64, beam_wl: f64) -> (f64, f64) {
    (
        (2.0 * beam_wl).max(0.05 * length_wl),
        length_wl.max(2.0 * beam_wl + 0.1 * length_wl),
    )
}

/// Michell's offsets grid at `draft`, as the core's `wave_tool` builds it (65 × 25).
pub(crate) fn michell_grid(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    draft: f64,
) -> Result<MichellGrid, String> {
    seacraft_core::michell::michell_grid_from_surface(surface, draft, 65, 25, subdivision).ok_or_else(
        || {
            "cannot extract Michell offsets grid from the hull (positive draft and waterline needed)"
                .to_string()
        },
    )
}

/// The pattern at `draft`, upright, with its readouts, in water of density `rho` (kg/m³; only the
/// wave resistance in newtons depends on it). Heel and trim are not inputs: thin-ship theory takes
/// an upright hull.
pub fn compute_waves(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    draft: f64,
    rho: f64,
    settings: &WaveSettings,
) -> Result<WaveResult, String> {
    let grid = michell_grid(surface, subdivision, draft)?;
    let (length_wl, beam_wl) = (grid.length_wl, grid.beam_wl);
    let wash_distance_m = wash_distance(settings, length_wl, beam_wl);
    let (ahead_l, behind_l, half_width_l) = domain_l(settings, length_wl, wash_distance_m);
    let speed_m_s = settings.speed.m_s(length_wl);
    let opts = WavePatternOptions {
        ahead_l,
        behind_l,
        half_width_l,
        catamaran_spacing: settings.cat_spacing_m,
        ..WavePatternOptions::at_speed(speed_m_s)
    };
    let field = wave_field(&grid, &opts)?;
    let clip_m = settings
        .clip_m
        .filter(|c| c.is_finite() && *c > 0.0)
        .unwrap_or(field.max_abs_zeta_m);
    let contours = if clip_m > 0.0 && settings.contours > 0 {
        contour_levels(clip_m, settings.contours)
            .into_iter()
            .map(|level| {
                let segs = contour::segments(&field.xs, &field.ys, &field.zeta, &field.wet, level);
                (level, segs)
            })
            .collect()
    } else {
        Vec::new()
    };
    let samples = seacraft_core::wave_pattern::wash_samples(&field, WASH_LINE_SAMPLES)?;
    let (wash_cut, wash) = wash_cut(&grid, &opts, &field, wash_distance_m, samples)?;
    let image = wave_image(&field, clip_m);

    let michell = |spacing: Option<f64>| {
        let o = MichellOptions {
            catamaran_spacing: spacing,
            ..MichellOptions::default()
        };
        michell_wave_resistance_from_grid(&grid, speed_m_s, rho, GRAVITY, &o)
    };
    let drag_here = michell(settings.cat_spacing_m);
    let (interference, spacing_scan) = match settings.cat_spacing_m {
        None => (None, Vec::new()),
        Some(_) => {
            // The same ratio as `michell::interference`, with one demihull computed once.
            let mono = michell(None).rw_n;
            let ratio = |cat_rw: f64| (mono > 1e-9).then(|| cat_rw / (2.0 * mono));
            let (s0, s1) = spacing_range(length_wl, beam_wl);
            let n = SPACING_SCAN_POINTS;
            let scan = (0..n)
                .filter_map(|i| {
                    let sp = s0 + (s1 - s0) * i as f64 / (n - 1) as f64;
                    ratio(michell(Some(sp)).rw_n).map(|r| (sp, r))
                })
                .collect();
            (ratio(drag_here.rw_n), scan)
        }
    };
    let mut warnings: Vec<String> = Vec::new();
    for w in &field.warnings {
        if !warnings.contains(w) {
            warnings.push(w.clone());
        }
    }
    Ok(WaveResult {
        regime: regime(field.froude),
        field,
        image,
        clip_m,
        contours,
        wash_cut,
        wash,
        wash_distance_m,
        drag_here,
        interference,
        spacing_scan,
        length_wl,
        beam_wl,
        draft: grid.draft,
        cat_spacing_m: settings.cat_spacing_m,
        warnings,
    })
}

/// Froude numbers of the wave-drag curve: 60, evenly from 0.10 to 1.00.
pub const DRAG_CURVE_POINTS: usize = 60;

/// Michell's wave drag over speed, for the view's wave-drag plot. Not keyed on the speed, so
/// scrubbing the speed does not recompute it.
#[derive(Debug, Clone)]
pub struct WaveDrag {
    /// One demihull (or the monohull) at each Froude number.
    pub mono: Vec<MichellResult>,
    /// The pair at the catamaran spacing, when one is set.
    pub cat: Option<Vec<MichellResult>>,
    /// The catamaran spacing it was computed for (m).
    pub cat_spacing_m: Option<f64>,
    pub length_wl: f64,
    /// Michell's warnings for the hull (L/B, wetted transom).
    pub warnings: Vec<String>,
}

/// The wave-drag curve at `draft`, upright, in water of density `rho` (kg/m³).
pub fn compute_wave_drag(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    draft: f64,
    rho: f64,
    cat_spacing_m: Option<f64>,
) -> Result<WaveDrag, String> {
    let grid = michell_grid(surface, subdivision, draft)?;
    let n = DRAG_CURVE_POINTS;
    let froudes: Vec<f64> = (0..n)
        .map(|i| 0.10 + 0.90 * i as f64 / (n - 1) as f64)
        .collect();
    let curve = |spacing: Option<f64>| {
        let o = MichellOptions {
            catamaran_spacing: spacing,
            ..MichellOptions::default()
        };
        seacraft_core::michell::michell_cw_curve(&grid, &froudes, rho, GRAVITY, &o)
    };
    Ok(WaveDrag {
        mono: curve(None),
        cat: cat_spacing_m.map(|s| curve(Some(s))),
        cat_spacing_m,
        length_wl: grid.length_wl,
        warnings: seacraft_core::michell::michell_warnings(&grid, &[]),
    })
}

/// A key over the settings the wave-drag curve depends on: the catamaran spacing only.
pub fn drag_key(cat_spacing_m: Option<f64>) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    cat_spacing_m.map(f64::to_bits).hash(&mut h);
    h.finish()
}

/// A key over every setting that changes the result: all but `kelvin_wedge` and `plan_contours`.
pub fn settings_key(settings: &WaveSettings) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match settings.speed {
        WaveSpeed::Knots(v) => (0u8, v.to_bits()).hash(&mut h),
        WaveSpeed::Froude(v) => (1u8, v.to_bits()).hash(&mut h),
    }
    match settings.domain {
        WaveDomain::Auto => 0u8.hash(&mut h),
        WaveDomain::Manual {
            ahead_m,
            behind_m,
            side_m,
        } => (1u8, ahead_m.to_bits(), behind_m.to_bits(), side_m.to_bits()).hash(&mut h),
    }
    for v in [
        settings.cat_spacing_m,
        settings.clip_m,
        settings.wash_distance_m,
    ] {
        v.map(f64::to_bits).hash(&mut h);
    }
    settings.contours.hash(&mut h);
    h.finish()
}

/// The field as an image, one pixel per grid point, row 0 at the largest y (starboard up, bow
/// right): the window's dark map (`window_rgb`) clipped at `clip_m`, with dry cells clear so the
/// view's hull footprint shows through them.
pub fn wave_image(field: &WaveField, clip_m: f64) -> egui::ColorImage {
    let (nx, ny) = (field.xs.len(), field.ys.len());
    let inv = if clip_m > 0.0 { 1.0 / clip_m } else { 0.0 };
    let mut rgba = vec![0u8; 4 * nx * ny];
    for (k, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (row, ix) = (k / nx, k % nx);
        let cell = (ny - 1 - row) * nx + ix;
        if field.wet[cell] {
            let [r, g, b] = window_rgb(field.zeta[cell] * inv);
            *px = [r, g, b, 255];
        }
    }
    egui::ColorImage::from_rgba_unmultiplied([nx, ny], &rgba)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme;
    use eframe::egui::Color32;
    use seacraft_core::subdiv::SubdivisionOptions;
    use seacraft_core::wave_pattern::{Regime, DIVERGING_STOPS, FAR_FIELD_NOTE};

    /// Sea water (kg/m³).
    const RHO: f64 = 1025.0;

    fn rgb(c: Color32) -> [u8; 3] {
        [c.r(), c.g(), c.b()]
    }

    /// The 2026-11-19 ruling: dark calm water in the window; the core's exports keep the light map.
    #[test]
    fn window_map_is_dark_at_calm_and_exports_stay_light() {
        let calm = window_rgb(0.0);
        let lum = |c: [u8; 3]| 0.2126 * c[0] as f64 + 0.7152 * c[1] as f64 + 0.0722 * c[2] as f64;
        assert!(lum(calm) < 40.0, "{calm:?}");
        assert!(lum(window_rgb(1.0)) > 90.0 && lum(window_rgb(-1.0)) > 90.0);
        assert_eq!(rgb(theme::WAVE_CALM), calm);
        assert_eq!(rgb(theme::WAVE_CREST), window_rgb(1.0));
        assert_eq!(rgb(theme::WAVE_TROUGH), window_rgb(-1.0));
        assert_eq!(DIVERGING_STOPS[1], [247, 247, 247]);
    }

    /// A 3 × 2 field with one dry cell.
    fn small_field() -> WaveField {
        let mut wet = vec![true; 6];
        wet[4] = false;
        WaveField {
            xs: vec![0.0, 1.0, 2.0],
            ys: vec![-1.0, 1.0],
            zeta: vec![-1.0, 0.0, 1.0, 0.5, 0.0, -0.5],
            wet,
            waterlines: Vec::new(),
            speed_m_s: 1.0,
            froude: 0.3,
            k0: 1.0,
            transverse_wavelength_m: 1.0,
            theta_cut_deg: 80.0,
            shortest_wavelength_m: 0.1,
            theta_samples: 10,
            max_abs_zeta_m: 1.0,
            notes: Vec::new(),
            warnings: Vec::new(),
        }
    }

    #[test]
    fn image_is_the_window_map_with_dry_cells_clear() {
        let f = small_field();
        let image = wave_image(&f, 1.0);
        assert_eq!(image.size, [3, 2]);
        for row in 0..2 {
            let iy = 1 - row; // row 0 is the largest y
            for ix in 0..3 {
                let px = image.pixels[row * 3 + ix];
                let k = iy * 3 + ix;
                if f.wet[k] {
                    let want = window_rgb(f.zeta[k]);
                    assert_eq!(
                        [px.r(), px.g(), px.b(), px.a()],
                        [want[0], want[1], want[2], 255],
                        "pixel ({ix}, {row})"
                    );
                } else {
                    assert_eq!(px.a(), 0, "dry pixel ({ix}, {row}) is clear");
                }
            }
        }
        // Row 0 is y = +1: its first pixel is ζ = 0.5, a crest tint, not the trough of ζ = -1.
        assert!(image.pixels[0].r() > image.pixels[0].b());
    }

    fn demo() -> (seacraft_core::mesh::Surface, SubdivisionOptions, f64) {
        let path = concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo-hull.fef");
        let text = std::fs::read_to_string(path).expect("read demo-hull.fef");
        let surface = seacraft_core::fef::parse(&text).expect("parse demo-hull.fef");
        let draft = surface.settings.draft;
        let subdiv = SubdivisionOptions {
            level: 2,
            ..SubdivisionOptions::default()
        };
        (surface, subdiv, draft)
    }

    #[test]
    fn demo_pattern_is_calm_ahead_and_has_contours() {
        let (surface, subdiv, draft) = demo();
        let w = compute_waves(&surface, subdiv, draft, RHO, &WaveSettings::default()).unwrap();
        let f = &w.field;
        let (nx, ny) = (f.xs.len(), f.ys.len());
        assert!((f.froude - 0.3).abs() < 1e-12, "Fn {}", f.froude);
        let bow = f
            .waterlines
            .iter()
            .flatten()
            .map(|p| p[0])
            .fold(f64::NEG_INFINITY, f64::max);
        let mut ahead = 0;
        for iy in 0..ny {
            for ix in 0..nx {
                let k = iy * nx + ix;
                if f.wet[k] && f.xs[ix] > bow + 0.05 * w.length_wl {
                    assert_eq!(f.zeta[k], 0.0, "ζ at x {} y {}", f.xs[ix], f.ys[iy]);
                    ahead += 1;
                }
            }
        }
        assert!(ahead > 0, "the domain reaches ahead of the bow");
        assert_eq!(w.image.size, [nx, ny]);
        assert!(!w.contours.is_empty());
        assert!(w.contours.iter().any(|(_, s)| !s.is_empty()));
        assert!(
            w.contours.windows(2).all(|p| p[0].0 < p[1].0),
            "ascending levels"
        );
        assert_eq!(w.wash_cut.s.len(), 401);
        assert_eq!(w.wash_cut.s[0], f.xs[0]);
        assert!((w.wash_cut.s[400] - f.xs[nx - 1]).abs() < 1e-9);
        assert_eq!(f.notes[0], FAR_FIELD_NOTE);
        assert!((w.clip_m - f.max_abs_zeta_m).abs() < 1e-15);
        assert!((w.draft - draft).abs() < 1e-12);
    }

    #[test]
    fn catamaran_has_two_dry_bands() {
        let (surface, subdiv, draft) = demo();
        let settings = WaveSettings {
            cat_spacing_m: Some(8.0),
            ..WaveSettings::default()
        };
        let w = compute_waves(&surface, subdiv, draft, RHO, &settings).unwrap();
        let f = &w.field;
        let nx = f.xs.len();
        let nearest = |y: f64| {
            (0..f.ys.len())
                .min_by(|&a, &b| (f.ys[a] - y).abs().total_cmp(&(f.ys[b] - y).abs()))
                .unwrap()
        };
        let row_dry = |iy: usize| (0..nx).filter(|&ix| !f.wet[iy * nx + ix]).count();
        assert_eq!(row_dry(nearest(0.0)), 0, "the centreline is open water");
        assert!(row_dry(nearest(4.0)) > 0, "starboard demihull");
        assert!(row_dry(nearest(-4.0)) > 0, "port demihull");
        assert_eq!(f.waterlines.len(), 2);
    }

    #[test]
    fn empty_domain_does_not_panic() {
        let (surface, subdiv, draft) = demo();
        for settings in [
            WaveSettings {
                domain: WaveDomain::Manual {
                    ahead_m: 5.0,
                    behind_m: 30.0,
                    side_m: 0.0,
                },
                ..WaveSettings::default()
            },
            WaveSettings {
                domain: WaveDomain::Manual {
                    ahead_m: 0.0,
                    behind_m: 0.0,
                    side_m: 15.0,
                },
                ..WaveSettings::default()
            },
        ] {
            match compute_waves(&surface, subdiv, draft, RHO, &settings) {
                Ok(w) => assert_eq!(w.image.size, [w.field.xs.len(), w.field.ys.len()]),
                Err(e) => assert!(!e.is_empty()),
            }
        }
    }

    #[test]
    fn no_waterline_is_an_error() {
        let (surface, subdiv, _) = demo();
        let err = compute_waves(&surface, subdiv, 0.0, RHO, &WaveSettings::default())
            .expect_err("no waterline at draft 0");
        assert!(err.contains("Michell"), "{err}");
    }

    #[test]
    fn catamaran_guess_clears_the_demihull() {
        // The demo hull: L_wl about 10.7 m, B_wl 3.54 m; 0.3 L alone would overlap.
        let s = catamaran_guess(Some(10.7), Some(3.54));
        assert!(s >= 2.0 * 3.54, "{s}");
        // A slender hull keeps the usual 0.3 L.
        assert!((catamaran_guess(Some(20.0), Some(1.0)) - 6.0).abs() < 1e-12);
        assert_eq!(catamaran_guess(None, None), 3.0);
    }

    #[test]
    fn switching_the_unit_keeps_the_speed() {
        let l = 9.7;
        let kn = WaveSpeed::Froude(0.3).in_unit(true, l);
        let WaveSpeed::Knots(v) = kn else {
            panic!("{kn:?}")
        };
        assert!((v * KNOT - 0.3 * (GRAVITY * l).sqrt()).abs() < 1e-12);
        let WaveSpeed::Froude(f) = kn.in_unit(false, l) else {
            panic!("back to Fn")
        };
        assert!((f - 0.3).abs() < 1e-12, "{f}");
        assert_eq!(kn.in_unit(true, l), kn, "already in knots");
    }

    #[test]
    fn key_moves_with_every_result_setting_only() {
        let base = WaveSettings::default();
        let k0 = settings_key(&base);
        for same in [
            WaveSettings {
                kelvin_wedge: !base.kelvin_wedge,
                ..base.clone()
            },
            WaveSettings {
                plan_contours: !base.plan_contours,
                ..base.clone()
            },
        ] {
            assert_eq!(settings_key(&same), k0);
        }
        let changed = [
            WaveSettings {
                speed: WaveSpeed::Froude(0.31),
                ..base.clone()
            },
            WaveSettings {
                speed: WaveSpeed::Knots(0.30),
                ..base.clone()
            },
            WaveSettings {
                domain: WaveDomain::Manual {
                    ahead_m: 5.0,
                    behind_m: 30.0,
                    side_m: 15.0,
                },
                ..base.clone()
            },
            WaveSettings {
                domain: WaveDomain::Manual {
                    ahead_m: 5.0,
                    behind_m: 30.0,
                    side_m: 16.0,
                },
                ..base.clone()
            },
            WaveSettings {
                cat_spacing_m: Some(8.0),
                ..base.clone()
            },
            WaveSettings {
                clip_m: Some(0.1),
                ..base.clone()
            },
            WaveSettings {
                contours: 3,
                ..base.clone()
            },
            WaveSettings {
                wash_distance_m: Some(12.0),
                ..base.clone()
            },
        ];
        for (i, s) in changed.iter().enumerate() {
            assert_ne!(settings_key(s), k0, "setting {i}");
        }
        assert_ne!(settings_key(&changed[2]), settings_key(&changed[3]));
    }

    #[test]
    fn auto_domain_keeps_the_wash_line_on_the_map() {
        let s = WaveSettings {
            wash_distance_m: Some(40.0),
            ..WaveSettings::default()
        };
        let (ahead, behind, side) = domain_l(&s, 10.0, 40.0);
        assert_eq!((ahead, behind), (0.5, 3.0));
        assert!((side - 4.8).abs() < 1e-12);
        let (_, _, side) = domain_l(&WaveSettings::default(), 10.0, 10.0);
        assert_eq!(side, 1.5);
        let manual = WaveSettings {
            domain: WaveDomain::Manual {
                ahead_m: 5.0,
                behind_m: 30.0,
                side_m: 20.0,
            },
            ..WaveSettings::default()
        };
        assert_eq!(domain_l(&manual, 10.0, 40.0), (0.5, 3.0, 2.0));
    }

    /// Review focus 4: the wash line never lies inside the hull.
    #[test]
    fn wash_distance_clears_the_hull() {
        let s = WaveSettings {
            wash_distance_m: Some(0.1),
            ..WaveSettings::default()
        };
        assert!((wash_distance(&s, 10.0, 3.0) - (1.5 + 0.5)).abs() < 1e-12);
        assert_eq!(wash_distance(&WaveSettings::default(), 10.7, 3.5), 10.5);
    }

    #[test]
    fn demo_result_has_every_readout() {
        let (surface, subdiv, draft) = demo();
        let settings = WaveSettings {
            speed: WaveSpeed::Froude(0.45),
            ..WaveSettings::default()
        };
        let w = compute_waves(&surface, subdiv, draft, RHO, &settings).unwrap();
        assert_eq!(w.regime, Regime::HullSpeed);
        assert!(w.wash.is_some());
        assert!(w.drag_here.rw_n > 0.0);
        assert!(w.interference.is_none());
        assert!(w.spacing_scan.is_empty());
        assert!(w.beam_wl > 0.0 && w.length_wl > w.beam_wl);
        assert_eq!(
            w.wash_distance_m,
            wash_distance(&settings, w.length_wl, w.beam_wl)
        );
        let mut sorted = w.warnings.clone();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), w.warnings.len(), "deduplicated");
    }

    /// Review focus 3: the scan starts at the clearance between the demihulls.
    #[test]
    fn catamaran_result_scans_spacing_from_the_clearance() {
        let (surface, subdiv, draft) = demo();
        let settings = WaveSettings {
            cat_spacing_m: Some(8.0),
            ..WaveSettings::default()
        };
        let w = compute_waves(&surface, subdiv, draft, RHO, &settings).unwrap();
        assert_eq!(w.spacing_scan.len(), 40);
        assert!(
            w.spacing_scan[0].0 >= 2.0 * w.beam_wl - 1e-12,
            "{:?}",
            w.spacing_scan[0]
        );
        let last = w.spacing_scan[39].0;
        assert!((last - w.length_wl.max(2.0 * w.beam_wl + 0.1 * w.length_wl)).abs() < 1e-9);
        assert!(w.interference.is_some());
        assert_eq!(w.cat_spacing_m, Some(8.0));
    }

    /// Review focus 4: the wash cut comes from the spectrum, so a narrow manual map still gives it.
    #[test]
    fn manual_domain_narrower_than_wash_still_reports_wash() {
        let (surface, subdiv, draft) = demo();
        let settings = WaveSettings {
            domain: WaveDomain::Manual {
                ahead_m: 5.0,
                behind_m: 30.0,
                side_m: 2.0,
            },
            wash_distance_m: Some(9.0),
            ..WaveSettings::default()
        };
        let w = compute_waves(&surface, subdiv, draft, RHO, &settings).unwrap();
        assert!(w.wash.is_some());
        assert_eq!(w.wash_distance_m, 9.0);
    }

    #[test]
    fn drag_curve_has_sixty_points_and_a_cat_curve_when_set() {
        let (surface, subdiv, draft) = demo();
        let mono = compute_wave_drag(&surface, subdiv, draft, RHO, None).unwrap();
        assert_eq!(mono.mono.len(), 60);
        assert!((mono.mono[0].fn_ - 0.10).abs() < 1e-9);
        assert!((mono.mono[59].fn_ - 1.00).abs() < 1e-9);
        assert!(mono.cat.is_none());
        assert!(mono.length_wl > 0.0);
        assert!(
            mono.warnings.iter().any(|w| w.contains("transom")),
            "{:?}",
            mono.warnings
        );
        let cat = compute_wave_drag(&surface, subdiv, draft, RHO, Some(8.0)).unwrap();
        assert_eq!(cat.cat.as_ref().map(Vec::len), Some(60));
        assert_eq!(cat.cat_spacing_m, Some(8.0));
        assert_ne!(drag_key(None), drag_key(Some(8.0)));
    }

    /// Review focus 2: no waterline fails the curve as it fails the pattern.
    #[test]
    fn no_waterline_fails_the_drag_curve_too() {
        let (surface, subdiv, _) = demo();
        let err = compute_wave_drag(&surface, subdiv, 0.0, RHO, None).expect_err("draft 0");
        assert!(err.contains("Michell"), "{err}");
    }

    /// Review focus 5: at Fn 0.05 the waves are 0.17 m long; the wash line samples finely enough
    /// for them, so the pattern still computes.
    #[test]
    fn very_low_speed_still_computes_with_its_wash() {
        let (surface, subdiv, draft) = demo();
        let settings = WaveSettings {
            speed: WaveSpeed::Froude(0.05),
            ..WaveSettings::default()
        };
        let w = compute_waves(&surface, subdiv, draft, RHO, &settings).unwrap();
        let ds = w.wash_cut.s[1] - w.wash_cut.s[0];
        assert!(ds < 0.5 * w.field.transverse_wavelength_m, "ds {ds}");
        assert!(w.field.xs.len() * w.field.ys.len() <= 250_000);
    }
}
