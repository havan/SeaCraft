//! The far-field wave pattern in the window: the settings the Performance tab edits, the computation
//! a `Job` runs off the window thread, and the image Plan view lays on the water.
//!
//! The colour map is the core's (`seacraft_core::wave_pattern`), so the window and the command line
//! draw the same colours; dry cells are left clear so the model shows through them.

use eframe::egui;
use seacraft_core::contour::{self, Segment};
use seacraft_core::mesh::Surface;
use seacraft_core::resistance::{GRAVITY, KNOT};
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_core::wave_pattern::{
    contour_levels, wave_cut, wave_field, wave_rgba, CutLine, WaveCut, WaveField,
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

/// What the Performance tab asks of the wave pattern.
#[derive(Debug, Clone, PartialEq)]
pub struct WaveSettings {
    /// Draw it on Plan view; it is computed only while shown.
    pub show: bool,
    pub speed: WaveSpeed,
    /// Domain ahead of the bow, behind the stern and each side, in waterline lengths.
    pub ahead_l: f64,
    pub behind_l: f64,
    pub half_width_l: f64,
    /// Demihull centreline separation (m); `None` for a monohull.
    pub cat_spacing_m: Option<f64>,
    /// Colour clip (m); `None` takes the field's largest |ζ|.
    pub clip_m: Option<f64>,
    /// Contour levels each side of calm; 0 draws none.
    pub contours: usize,
    /// The longitudinal cut's offset from the centreline, in waterline lengths.
    pub cut_y_l: f64,
}

impl Default for WaveSettings {
    fn default() -> Self {
        let d = WavePatternOptions::at_speed(1.0);
        Self {
            show: false,
            speed: WaveSpeed::Froude(0.30),
            ahead_l: d.ahead_l,
            behind_l: d.behind_l,
            half_width_l: d.half_width_l,
            cat_spacing_m: None,
            clip_m: None,
            contours: 6,
            cut_y_l: 0.25,
        }
    }
}

/// A computed pattern and everything the window draws from it.
#[derive(Debug, Clone)]
pub struct WaveResult {
    pub field: WaveField,
    pub image: egui::ColorImage,
    /// The colour clip used (m).
    pub clip_m: f64,
    /// `(level, segments)` in ascending level, none at calm.
    pub contours: Vec<(f64, Vec<Segment>)>,
    /// The longitudinal cut at the settings' offset, across the field's x range.
    pub cut: WaveCut,
    pub length_wl: f64,
    pub draft: f64,
    /// The catamaran spacing it was computed for (m).
    pub cat_spacing_m: Option<f64>,
}

/// Samples along the longitudinal cut.
const CUT_SAMPLES: usize = 401;

/// The pattern at `draft`, upright, built the way the core's `wave_tool` builds it (Michell's
/// 65 × 25 offsets grid). Heel and trim are not inputs: thin-ship theory takes an upright hull.
pub fn compute_waves(
    surface: &Surface,
    subdivision: SubdivisionOptions,
    draft: f64,
    settings: &WaveSettings,
) -> Result<WaveResult, String> {
    let grid = seacraft_core::michell::michell_grid_from_surface(
        surface,
        draft,
        65,
        25,
        subdivision,
    )
    .ok_or(
        "cannot extract Michell offsets grid from the hull (positive draft and waterline needed)",
    )?;
    let length_wl = grid.length_wl;
    let opts = WavePatternOptions {
        ahead_l: settings.ahead_l,
        behind_l: settings.behind_l,
        half_width_l: settings.half_width_l,
        catamaran_spacing: settings.cat_spacing_m,
        ..WavePatternOptions::at_speed(settings.speed.m_s(length_wl))
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
    let (x0, x1) = (field.xs[0], field.xs[field.xs.len() - 1]);
    let line = CutLine::Longitudinal {
        y: settings.cut_y_l * length_wl,
    };
    let cut = wave_cut(&grid, &opts, line, x0, x1, CUT_SAMPLES)?;
    let image = wave_image(&field, clip_m);
    Ok(WaveResult {
        field,
        image,
        clip_m,
        contours,
        cut,
        length_wl,
        draft: grid.draft,
        cat_spacing_m: settings.cat_spacing_m,
    })
}

/// A key over every setting that changes the result: all but `show`.
pub fn settings_key(settings: &WaveSettings) -> u64 {
    let mut h = std::collections::hash_map::DefaultHasher::new();
    match settings.speed {
        WaveSpeed::Knots(v) => (0u8, v.to_bits()).hash(&mut h),
        WaveSpeed::Froude(v) => (1u8, v.to_bits()).hash(&mut h),
    }
    for v in [
        settings.ahead_l,
        settings.behind_l,
        settings.half_width_l,
        settings.cut_y_l,
    ] {
        v.to_bits().hash(&mut h);
    }
    for v in [settings.cat_spacing_m, settings.clip_m] {
        v.map(f64::to_bits).hash(&mut h);
    }
    settings.contours.hash(&mut h);
    h.finish()
}

/// The field as an image, one pixel per grid point, row 0 at the largest y (starboard up, bow
/// right): the core's colour map clipped at `clip_m`, with dry cells clear. The command line's PNG
/// paints dry cells grey; the window shows the model through them instead.
pub fn wave_image(field: &WaveField, clip_m: f64) -> egui::ColorImage {
    let (nx, ny) = (field.xs.len(), field.ys.len());
    let mut rgba = wave_rgba(field, Some(clip_m));
    for (k, px) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let (row, ix) = (k / nx, k % nx);
        if !field.wet[(ny - 1 - row) * nx + ix] {
            px.fill(0);
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
    use seacraft_core::wave_pattern::{DIVERGING_STOPS, FAR_FIELD_NOTE};

    fn rgb(c: Color32) -> [u8; 3] {
        [c.r(), c.g(), c.b()]
    }

    #[test]
    fn theme_stops_are_the_core_stops() {
        assert_eq!(rgb(theme::WAVE_TROUGH), DIVERGING_STOPS[0]);
        assert_eq!(rgb(theme::WAVE_CALM), DIVERGING_STOPS[1]);
        assert_eq!(rgb(theme::WAVE_CREST), DIVERGING_STOPS[2]);
        for c in [theme::WAVE_TROUGH, theme::WAVE_CALM, theme::WAVE_CREST] {
            assert_eq!(c.a(), 255);
        }
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
    fn image_is_the_core_colour_map_with_dry_cells_clear() {
        let f = small_field();
        let image = wave_image(&f, 1.0);
        assert_eq!(image.size, [3, 2]);
        let core = wave_rgba(&f, Some(1.0));
        for row in 0..2 {
            let iy = 1 - row; // row 0 is the largest y
            for ix in 0..3 {
                let px = image.pixels[row * 3 + ix];
                let k = row * 3 + ix;
                if f.wet[iy * 3 + ix] {
                    assert_eq!(
                        [px.r(), px.g(), px.b(), px.a()],
                        [core[4 * k], core[4 * k + 1], core[4 * k + 2], 255],
                        "pixel ({ix}, {row})"
                    );
                } else {
                    assert_eq!(px.a(), 0, "dry pixel ({ix}, {row}) is clear");
                }
            }
        }
        // Row 0 is y = +1: its first pixel is ζ = 0.5, a pale crest, not the trough of ζ = -1.
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
        let w = compute_waves(&surface, subdiv, draft, &WaveSettings::default()).unwrap();
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
        assert_eq!(w.cut.s.len(), 401);
        assert_eq!(w.cut.s[0], f.xs[0]);
        assert!((w.cut.s[400] - f.xs[nx - 1]).abs() < 1e-9);
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
        let w = compute_waves(&surface, subdiv, draft, &settings).unwrap();
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
                half_width_l: 0.0,
                ..WaveSettings::default()
            },
            WaveSettings {
                ahead_l: 0.0,
                behind_l: 0.0,
                ..WaveSettings::default()
            },
        ] {
            match compute_waves(&surface, subdiv, draft, &settings) {
                Ok(w) => assert_eq!(w.image.size, [w.field.xs.len(), w.field.ys.len()]),
                Err(e) => assert!(!e.is_empty()),
            }
        }
    }

    #[test]
    fn no_waterline_is_an_error() {
        let (surface, subdiv, _) = demo();
        let err = compute_waves(&surface, subdiv, 0.0, &WaveSettings::default())
            .expect_err("no waterline at draft 0");
        assert!(err.contains("Michell"), "{err}");
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
    fn key_moves_with_every_setting_but_show() {
        let base = WaveSettings::default();
        let k0 = settings_key(&base);
        let shown = WaveSettings {
            show: !base.show,
            ..base.clone()
        };
        assert_eq!(settings_key(&shown), k0);
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
                ahead_l: 0.5,
                ..base.clone()
            },
            WaveSettings {
                behind_l: 2.0,
                ..base.clone()
            },
            WaveSettings {
                half_width_l: 1.0,
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
                cut_y_l: 0.5,
                ..base.clone()
            },
        ];
        for (i, s) in changed.iter().enumerate() {
            assert_ne!(settings_key(s), k0, "setting {i}");
        }
    }
}
