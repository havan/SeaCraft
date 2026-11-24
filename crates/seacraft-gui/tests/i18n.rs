//! Headless i18n tests: deterministic English startup, live switching via Help > Language,
//! Phase 2a Turkish chrome/menus/dialogs/start card, min-size rendering, and persistence.

use eframe::egui;
use seacraft_core::i18n::{LanguagePref, Locale};
use seacraft_gui::recent::JsonFileStorage;
use seacraft_gui::App;

struct Window {
    ctx: egui::Context,
    app: App,
    size: egui::Vec2,
    widgets: Vec<(String, egui::Rect)>,
    painted_texts: Vec<String>,
}

impl Window {
    fn new() -> Self {
        Self::with_app(App::headless_for_test(None), egui::vec2(1600.0, 1000.0))
    }

    fn with_app(app: App, size: egui::Vec2) -> Self {
        let ctx = egui::Context::default();
        ctx.enable_accesskit();
        let mut w = Self {
            ctx,
            app,
            size,
            widgets: Vec::new(),
            painted_texts: Vec::new(),
        };
        w.frame(vec![]);
        w.frame(vec![]);
        w
    }

    fn frame(&mut self, events: Vec<egui::Event>) {
        fn collect_texts(shape: &egui::Shape, out: &mut Vec<String>) {
            match shape {
                egui::Shape::Vec(v) => v.iter().for_each(|s| collect_texts(s, out)),
                egui::Shape::Text(t) => out.push(t.galley.text().to_owned()),
                _ => {}
            }
        }
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, self.size)),
            events,
            ..Default::default()
        };
        let app = &mut self.app;
        let mut out = self.ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
        self.painted_texts.clear();
        for clipped in &out.shapes {
            collect_texts(&clipped.shape, &mut self.painted_texts);
        }
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
            .unwrap_or_else(|| {
                panic!(
                    "no widget `{label}` among {:?}",
                    self.widgets
                        .iter()
                        .map(|(l, _)| l.as_str())
                        .collect::<Vec<_>>()
                )
            })
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

    fn press_esc(&mut self) {
        let key = |pressed| egui::Event::Key {
            key: egui::Key::Escape,
            physical_key: None,
            pressed,
            repeat: false,
            modifiers: egui::Modifiers::NONE,
        };
        self.frame(vec![key(true)]);
        self.frame(vec![key(false)]);
    }

    fn shows(&self, text: &str) -> bool {
        self.widgets.iter().any(|(t, _)| t == text)
    }

    fn shows_substring(&self, text: &str) -> bool {
        self.widgets.iter().any(|(t, _)| t.contains(text))
    }

    fn paints(&self, text: &str) -> bool {
        self.painted_texts.iter().any(|t| t == text)
    }

    fn paints_substring(&self, text: &str) -> bool {
        self.painted_texts.iter().any(|t| t.contains(text))
    }
}

#[test]
fn headless_defaults_to_english_and_switches_live_via_help_language_menu() {
    let mut w = Window::new();
    assert_eq!(w.app.locale(), Locale::En);
    assert!(w.shows("Help"));
    assert!(!w.shows("Yardım"));

    w.click("Help");
    w.click("Language");
    w.click("Türkçe");

    assert_eq!(w.app.locale(), Locale::Tr);
    for menu in ["Dosya", "Düzenle", "Görünüm", "Model", "Analiz", "Yardım"] {
        assert!(w.shows(menu), "expected Turkish menu `{menu}`");
    }
    assert!(!w.shows("Help"));

    w.click("Dosya");
    for item in ["Yeni", "Kaydet", "Dışa aktar", "Çıkış"] {
        assert!(
            w.shows_substring(item),
            "expected Turkish File item `{item}`"
        );
    }
    w.press_esc();

    w.click("Analiz");
    for item in [
        "Hidrostatik tablo",
        "Dalga deseni",
        "Meyil ve doğrultucu kol",
    ] {
        assert!(w.shows(item), "expected Turkish Analysis item `{item}`");
    }
    w.press_esc();

    w.click("Yardım");
    assert!(w.shows("Dil"));
    assert!(w.shows("SeaCraft Hakkında…"));
    w.click("SeaCraft Hakkında…");
    assert!(w.shows("SeaCraft Hakkında"));
    assert!(w.shows("Yazı tipi lisansı (DejaVu)"));
    w.click("Yazı tipi lisansı (DejaVu)");
    assert!(w.shows_substring("Bitstream Vera"));
    w.press_esc();

    w.click("Yardım");
    w.click("Dil");
    w.click("English");

    assert_eq!(w.app.locale(), Locale::En);
    assert!(w.shows("Help"));
}

#[test]
fn language_preference_persists_across_storage_reload() {
    let dir = std::env::temp_dir().join(format!("seacraft-gui-i18n-test-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(JsonFileStorage::FILE_NAME);

    {
        let mut app = App::headless_for_test(None);
        app.set_storage(Box::new(JsonFileStorage::open(&path)));
        assert_eq!(app.language_pref(), LanguagePref::Auto);
        assert_eq!(app.locale(), Locale::En);

        app.set_language_pref(LanguagePref::Fixed(Locale::Tr));
        assert_eq!(app.locale(), Locale::Tr);
    }

    {
        let mut reloaded = App::headless_for_test(None);
        reloaded.set_storage(Box::new(JsonFileStorage::open(&path)));
        assert_eq!(reloaded.language_pref(), LanguagePref::Fixed(Locale::Tr));
        assert_eq!(reloaded.locale(), Locale::Tr);
    }

    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn start_card_and_min_window_render_in_turkish() {
    let mut empty = App::headless_empty();
    empty.set_locale(Locale::Tr);
    let w_empty = Window::with_app(empty, egui::vec2(900.0, 600.0));
    for btn in ["Yeni tasarım…", "Örnek aç…", "Demo tekneyi aç"] {
        assert!(
            w_empty.shows_substring(btn),
            "expected Turkish start card button `{btn}`"
        );
    }

    let mut loaded = App::headless_for_test(None);
    loaded.set_locale(Locale::Tr);
    let w_loaded = Window::with_app(loaded, egui::vec2(900.0, 600.0));
    for menu in ["Dosya", "Düzenle", "Görünüm", "Model", "Analiz", "Yardım"] {
        assert!(
            w_loaded.shows(menu),
            "expected Turkish menu `{menu}` at 900x600"
        );
    }
}

#[test]
fn viewports_orientation_model_menu_and_dialogs_render_in_turkish() {
    let mut app = App::headless_for_test(None);
    app.set_locale(Locale::Tr);
    let mut w = Window::with_app(app, egui::vec2(1600.0, 1000.0));

    for title in [
        "Perspektif",
        "Profil · sancaktan",
        "Plan · yarı genişlikler, sancak yukarı",
        "Posta planı · başa bakış",
    ] {
        assert!(
            w.paints(title),
            "expected Turkish viewport title `{title}` in {:?}",
            w.painted_texts
        );
    }
    for orient in ["Baş", "Kıç", "İsk.", "Snc.", "Yuk."] {
        assert!(
            w.paints(orient),
            "expected Turkish orientation label `{orient}`"
        );
    }
    assert!(
        w.paints_substring("ızgara"),
        "expected Turkish grid caption"
    );

    w.click("Model");
    for item in [
        "Dönüştür…",
        "Eniyile…",
        "Kenar ekle",
        "Yüzeyi böl",
        "Keskin kenar yap",
        "Postayı seç",
        "Boyuna hattı seç",
    ] {
        assert!(w.shows(item), "expected Turkish Model menu item `{item}`");
    }

    w.click("Dönüştür…");
    for text in ["Tekneyi dönüştür", "Uygula", "İptal"] {
        assert!(
            w.shows(text),
            "expected Turkish Transform dialog text `{text}`"
        );
    }
    w.press_esc();

    w.click("Model");
    w.click("Eniyile…");
    for text in ["Tekneyi eniyile", "Çalıştır", "Kapat"] {
        assert!(
            w.shows(text),
            "expected Turkish Optimise dialog text `{text}`"
        );
    }
    w.click("Kapat");

    w.click("Dosya");
    w.click("Proje ayarları…");
    for text in ["Proje ayarları", "Tasarımcı", "Su yoğunluğu", "Uygula"] {
        assert!(
            w.shows(text),
            "expected Turkish Project settings text `{text}`"
        );
    }
    w.press_esc();

    w.click("Dosya");
    w.click("Yeni");
    w.click("Tasarım… Ctrl+N");
    assert!(w.shows("Kaydedilmemiş değişiklikler"));
    w.click("Kaydetmeden devam et");
    if let Some(d) = w.app.new_design_mut() {
        d.inline_preview = true;
    }
    w.frame(vec![]);
    for text in ["Yeni tasarım", "Parametrik", "Ana boyutlar", "Oluştur"] {
        assert!(
            w.shows(text),
            "expected Turkish New design dialog text `{text}`"
        );
    }
}
