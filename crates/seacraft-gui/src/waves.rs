//! The far-field wave pattern in the window: the settings the Performance tab edits, the computation
//! a `Job` runs off the window thread, and the image Plan view lays on the water.
//!
//! The colour map is the core's (`seacraft_core::wave_pattern`), so the window and the command line
//! draw the same colours; dry cells are left clear so the model shows through them.

use eframe::egui;
use seacraft_core::wave_pattern::{wave_rgba, WaveField};

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
    use seacraft_core::wave_pattern::DIVERGING_STOPS;

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
}
