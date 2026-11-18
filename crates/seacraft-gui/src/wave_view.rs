//! The Wave pattern view in the Analysis drawer: one speed you can scrub drives the speed regime,
//! humps and hollows of wave drag, catamaran interference, wash at a distance and the picture.
//!
//! The panel never changes the model or the app: it edits the wave settings it is given (view
//! inputs, like the other Analysis views' inputs) and returns what the user asked for as a
//! [`WaveViewAction`], which `App::wave_view_drawer` applies. The map's pan and zoom are view state
//! ([`MapState`]).

use crate::analysis_views::AnalysisView;
use crate::theme;
use crate::waves::{WaveDrag, WaveResult, WaveSettings, WaveSpeed};
use eframe::egui;

/// What the wave view asked for, applied by the App after the panel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum WaveViewAction {
    None,
    /// A setting was edited in place: the pattern is recomputed.
    Changed,
    /// "Save image…": PNG, SVG or VTI.
    SaveImage,
    SetSpeed(WaveSpeed),
    /// A catamaran spacing (m), from the spacing plot.
    SetSpacing(f64),
    /// A wash distance (m), from dragging the wash line.
    SetWashDistance(f64),
    Close,
    /// Show another Analysis view in the drawer.
    Open(AnalysisView),
}

/// The map's pan and zoom: view state, kept by the App between frames.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct MapState {
    /// Scale over the fitted view (0 until fitted).
    pub zoom: f32,
    /// Offset of the field's centre from the map's centre (points).
    pub pan: egui::Vec2,
    pub fitted: bool,
}

/// The wave view's inputs.
pub struct WaveViewPanel<'a> {
    pub settings: &'a mut WaveSettings,
    /// The last pattern computed, current or not (`current`, `stale`).
    pub result: Option<&'a WaveResult>,
    /// The wave-drag curve, when current.
    pub drag: Option<&'a WaveDrag>,
    /// The result kept predates the last change.
    pub stale: bool,
    pub busy: bool,
    /// The result is for the current inputs: it may be saved.
    pub current: bool,
    /// Why the pattern (or the drag curve) failed for the current inputs.
    pub failure: Option<&'a str>,
    /// The floating condition is heeled or trimmed, which the pattern ignores.
    pub heeled_or_trimmed: bool,
    /// The waterline length and beam the speed's units and the catamaran guess convert by.
    pub length_wl: Option<f64>,
    pub beam_wl: Option<f64>,
    pub map: &'a mut MapState,
}

/// Draw the view: header (the views as tabs, Save image, close), the failure line, then the body.
pub fn panel(ui: &mut egui::Ui, p: WaveViewPanel<'_>) -> WaveViewAction {
    let mut action = WaveViewAction::None;
    ui.horizontal(|ui| {
        for other in AnalysisView::ALL {
            let shown = other == AnalysisView::Waves;
            if ui.selectable_label(shown, other.label()).clicked() && !shown {
                action = WaveViewAction::Open(other);
            }
        }
        ui.separator();
        if p.busy {
            ui.spinner();
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✕").on_hover_text("Close Wave pattern").clicked() {
                action = WaveViewAction::Close;
            }
            let can_save = p.current && p.result.is_some();
            if ui
                .add_enabled(can_save, egui::Button::new("Save image…"))
                .on_hover_text("PNG, SVG or VTI, by the file's extension")
                .clicked()
            {
                action = WaveViewAction::SaveImage;
            }
        });
    });
    if let Some(failure) = p.failure {
        ui.colored_label(theme::FAIL, format!("Wave pattern failed: {failure}"));
    }
    if let Some(r) = p.result {
        crate::panels::stale_section(ui, p.stale, |ui| {
            ui.label(r.regime.sentence());
        });
    } else if p.busy {
        ui.label(egui::RichText::new("Computing the wave pattern…").color(theme::TEXT_MUTED));
    }
    action
}
