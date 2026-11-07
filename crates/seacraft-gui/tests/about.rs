//! Help > About, headless: the legal notices GPLv3 section 5(d) asks for are one click away.

use eframe::egui;
use seacraft_gui::App;

/// One headless window and what its last frame drew: label or value, rectangle.
struct Window {
    ctx: egui::Context,
    app: App,
    widgets: Vec<(String, egui::Rect)>,
}

impl Window {
    fn new() -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut w = Self {
            ctx,
            app: App::headless_for_test(None),
            widgets: Vec::new(),
        };
        w.frame(vec![]);
        w.frame(vec![]);
        w
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
        self.widgets = out
            .platform_output
            .accesskit_update
            .map(|update| {
                update
                    .nodes
                    .iter()
                    .filter_map(|(_, node)| {
                        let b = node.bounds()?;
                        let rect = egui::Rect::from_min_max(
                            egui::pos2(b.x0 as f32, b.y0 as f32),
                            egui::pos2(b.x1 as f32, b.y1 as f32),
                        );
                        let text = node.label().or(node.value()).unwrap_or("").to_owned();
                        Some((text, rect))
                    })
                    .collect()
            })
            .unwrap_or_default();
    }

    fn click(&mut self, label: &str) {
        let rect = self
            .widgets
            .iter()
            .find(|(l, _)| l == label)
            .unwrap_or_else(|| panic!("no widget `{label}`"))
            .1;
        let pos = rect.center();
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::default(),
        };
        self.frame(vec![egui::Event::PointerMoved(pos), button(true)]);
        self.frame(vec![button(false)]);
        self.frame(vec![]);
    }

    fn shows(&self, text: &str) -> bool {
        self.widgets.iter().any(|(t, _)| t.contains(text))
    }
}

#[test]
fn help_about_shows_the_notices_and_the_licence_texts_open_from_it() {
    let mut w = Window::new();
    assert!(!w.shows("ABSOLUTELY NO WARRANTY"));

    w.click("Help");
    w.click("About SeaCraft…");
    assert!(w.shows(&format!("SeaCraft {}", seacraft_core::VERSION)));
    assert!(w.shows("Copyright (c) 2026 SeaCraft contributors."));
    assert!(w.shows("ABSOLUTELY NO WARRANTY"));
    assert!(w.shows("GNU General Public License, version 3 or any later version"));
    assert!(w.shows("not validated"));

    w.click("Font licence (DejaVu)");
    assert!(w.shows("Bitstream Vera"), "the font licence opens");
}
