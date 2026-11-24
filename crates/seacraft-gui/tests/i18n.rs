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

#[test]
fn docks_and_options_windows_render_in_turkish() {
    use seacraft_core::i18n::Tr;
    use seacraft_gui::panels;

    let tr_tr = Tr::new(Locale::Tr);
    assert_eq!(
        panels::criteria_badge_style_tr(true, false, tr_tr).0,
        "GEÇTİ"
    );
    assert_eq!(
        panels::criteria_badge_style_tr(true, true, tr_tr).0,
        "GEÇTİ · güncel değil"
    );
    assert_eq!(
        panels::criteria_badge_style_tr(false, false, tr_tr).0,
        "KALDI"
    );
    assert_eq!(
        panels::criteria_badge_style_tr(false, true, tr_tr).0,
        "KALDI · güncel değil"
    );

    let mut app = App::headless_for_test(None);
    app.set_locale(Locale::Tr);
    app.recompute_hydrostatics();
    let mut w = Window::with_app(app, egui::vec2(1600.0, 1000.0));

    // Left dock: Layers & empty Inspector.
    for text in [
        "Katmanlar",
        "Ekle",
        "Sil",
        "Simetrik",
        "Hidrostatik",
        "Endaze resmi",
        "Seçilen yüzeyi bu katmana taşı",
        "Denetçi",
        "Düzenlemek için bir noktaya tıklayın — Esc temizler",
    ] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish left dock text `{text}`"
        );
    }

    // Inspector with a selected point.
    w.app.select_point(Some(0));
    w.frame(vec![]);
    assert!(
        w.shows("Nokta 0"),
        "expected Turkish selected point heading"
    );
    assert!(
        w.shows_substring("keskin kenar"),
        "expected Turkish vertex crease footer"
    );

    // Right Analysis dock tab bar + Hydrostatics tab.
    for tab in ["Hidro", "Yükleme", "Yaralı", "Rüzgâr", "Performans"] {
        assert!(w.shows(tab), "expected Turkish dock tab `{tab}`");
    }
    for text in [
        "Hidrostatik",
        "Su çekimi",
        "Meyil",
        "Serbest trim",
        "GZ eğrileri",
        "Hacim ve form",
        "Stabilite (Denge)",
    ] {
        assert!(
            w.shows(text) || w.paints(text),
            "expected Turkish Hydrostatics text `{text}`"
        );
    }

    // Loading tab.
    w.click("Yükleme");
    for text in ["Yükleme durumu", "Yeni"] {
        assert!(w.shows(text), "expected Turkish Loading text `{text}`");
    }
    w.click("Yeni");
    for text in ["Boş gemi", "Ölü ağırlık kalemleri", "Tanklar", "Hesapla"] {
        assert!(w.shows(text), "expected Turkish Loading form text `{text}`");
    }

    // Damage tab.
    w.click("Yaralı");
    for text in ["Yaralı stabilite", "Bölmeler", "Yara durumları"] {
        assert!(w.shows(text), "expected Turkish Damage text `{text}`");
    }
    w.click("Yeni");
    for text in ["Hasarsız durum", "Yükleme durumu", "Elle"] {
        assert!(w.shows(text), "expected Turkish Damage case text `{text}`");
    }

    // Windage tab.
    w.click("Rüzgâr");
    for text in [
        "Üst yapı ve rüzgâr alanı",
        "+ Rüzgâr öğesi ekle",
        "Rüzgâr alanı toplamları",
    ] {
        assert!(w.shows(text), "expected Turkish Windage text `{text}`");
    }

    // Performance tab.
    w.click("Performans");
    for text in [
        "Performans",
        "Direnç tahmini",
        "Dalga deseni",
        "Denizcilik (baş dalgası RAO)",
        "Deniz durumu (düzensiz baş dalgaları)",
    ] {
        assert!(w.shows(text), "expected Turkish Performance text `{text}`");
    }

    // Report options window.
    w.click("Dosya");
    w.click("Rapor seçenekleri…");
    for text in [
        "Rapor seçenekleri",
        "Genel hidrostatik",
        "Direnç ve güç",
        "Yükleme durumları",
        "Yara durumları",
        "Denizcilik (Deniz tutma)",
    ] {
        assert!(
            w.shows(text),
            "expected Turkish Report options text `{text}`"
        );
    }
    w.app.set_show_report_options(false);
    w.frame(vec![]);

    // Builder templates window.
    w.click("Dosya");
    w.click("İnşa şablonları…");
    for text in [
        "İnşa şablonları",
        "Postalar:",
        "Sayı",
        "Aralık",
        "Kâğıt:",
        "Ölçek:",
        "Dışa aktar…",
    ] {
        assert!(
            w.shows(text),
            "expected Turkish Builder templates text `{text}`"
        );
    }
    w.app.set_show_templates(false);
    w.frame(vec![]);

    // Render all 5 dock tabs cleanly at the minimum supported window size (900x600).
    let mut small = App::headless_for_test(None);
    small.set_locale(Locale::Tr);
    small.recompute_hydrostatics();
    let mut w_small = Window::with_app(small, egui::vec2(900.0, 600.0));
    for tab in ["Hidro", "Yükleme", "Yaralı", "Rüzgâr", "Performans"] {
        w_small.click(tab);
    }
}

#[test]
fn analysis_views_and_builder_panels_render_in_turkish() {
    use seacraft_core::heel::Equilibrium;
    use seacraft_core::i18n::Tr;
    use seacraft_gui::analysis_views::{self, AnalysisView, TableColumn, ViewTable};
    use seacraft_gui::panels::{ColumnKind, EngCell};
    use seacraft_gui::{heel_view, wave_view};

    let tr_tr = Tr::new(Locale::Tr);

    // Pure helper localization checks.
    let sample_table = ViewTable {
        columns: vec![
            TableColumn {
                title: "Katman".into(),
                unit: String::new(),
                kind: ColumnKind::Text,
            },
            TableColumn {
                title: "Alan".into(),
                unit: "m²".into(),
                kind: ColumnKind::Number,
            },
            TableColumn {
                title: "Cb".into(),
                unit: String::new(),
                kind: ColumnKind::Number,
            },
        ],
        rows: vec![vec![
            "Dip".into(),
            "1.250".into(),
            EngCell::Badge(true, false),
        ]],
    };
    let csv = analysis_views::to_csv_tr(&sample_table, tr_tr);
    assert_eq!(csv, "Katman,Alan (m²),Cb\nDip,1.250,GEÇTİ\n");

    assert_eq!(
        wave_view::interference_words_tr(0.88, tr_tr),
        "olumlu (−12 %)"
    );
    assert_eq!(
        wave_view::interference_words_tr(1.004, tr_tr),
        "nötr (±1 %)"
    );
    assert_eq!(
        heel_view::equilibrium_line_tr(
            &Equilibrium::Capsizes {
                last_heel_deg: 90.0
            },
            10.0,
            tr_tr
        ),
        "Alabora olur: ±90° içinde kararlı denge yok"
    );

    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(1);
    app.set_locale(Locale::Tr);
    app.recompute_hydrostatics();
    let mut w = Window::with_app(app, egui::vec2(1600.0, 1000.0));

    // Hydrostatic table.
    w.click("Analiz");
    w.click("Hidrostatik tablo");
    w.app.wait_analysis_view(AnalysisView::HydroTable);
    w.frame(vec![]);
    for text in ["Hesapla", "CSV kopyala", "Üst su çekimi", "Hacim"] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Hydrostatic table text `{text}`"
        );
    }

    // Bonjean curves.
    w.app.open_analysis_view(AnalysisView::Bonjean);
    w.app.wait_analysis_view(AnalysisView::Bonjean);
    w.frame(vec![]);
    for text in ["Bonjean eğrileri", "posta:", "su çekimi:"] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Bonjean text `{text}`"
        );
    }

    // Offsets table.
    w.app.open_analysis_view(AnalysisView::Offsets);
    w.app.wait_analysis_view(AnalysisView::Offsets);
    w.frame(vec![]);
    for text in ["Ofset tablosu", "Omurga z", "Güverte z", "Yarı genişlik"] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Offsets text `{text}`"
        );
    }

    // Trim scan.
    w.app.open_analysis_view(AnalysisView::TrimScan);
    w.app.wait_analysis_view(AnalysisView::TrimScan);
    w.frame(vec![]);
    for text in ["Trim taraması", "deplasmanı sabit tut"] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Trim scan text `{text}`"
        );
    }

    // Plate development.
    w.app.open_analysis_view(AnalysisView::Plates);
    w.app.wait_analysis_view(AnalysisView::Plates);
    w.frame(vec![]);
    for text in [
        "Şablonları dışa aktar…",
        "iki taraf",
        "Katman",
        "Açınabilir",
    ] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Plates text `{text}`"
        );
    }

    // Wave pattern view.
    w.app.open_analysis_view(AnalysisView::Waves);
    w.app.compute_waves_blocking().unwrap();
    w.app.compute_wave_drag_blocking().unwrap();
    w.frame(vec![]);
    w.frame(vec![]);
    for text in [
        "İnce gemi kuramı (Michell)",
        "Hız",
        "Katamaran",
        "Harita ▾",
        "Renkler ▾",
        "Görüntüyü kaydet…",
        "Dalga boyu",
        "Gövde hızı",
        "Baş →",
    ] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Wave view text `{text}`"
        );
    }

    // Heel and righting arm view: with KG set.
    w.app.open_analysis_view(AnalysisView::Heel);
    w.app.set_kg(2.2);
    w.app.compute_heel_blocking().unwrap();
    w.frame(vec![]);
    for text in [
        "Başlangıç",
        "Çıplak gövde (panel)",
        "Meyli tut",
        "Otomatik yeniden hesapla",
        "Deneme ağırlıkları (0)",
        "Doğrultucu kol",
        "Yüzme durumu",
        "Döndürme çifti",
        "Başa bakış",
    ] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Heel view text `{text}`"
        );
    }
    w.app.set_heel_view_open(false);
    w.frame(vec![]);

    // Heel view without KG (copy demo-hull.fef without sidecar): shows Turkish missing-KG hint.
    let no_kg_dir =
        std::env::temp_dir().join(format!("seacraft-gui-i18n-no-kg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&no_kg_dir);
    std::fs::create_dir_all(&no_kg_dir).unwrap();
    let no_kg_path = no_kg_dir.join("hull.fef");
    std::fs::copy(
        concat!(env!("CARGO_MANIFEST_DIR"), "/../../examples/demo-hull.fef"),
        &no_kg_path,
    )
    .unwrap();
    let mut no_kg_app = App::headless_for_test(Some(no_kg_path.to_str().unwrap()));
    no_kg_app.set_subdivision_level(1);
    no_kg_app.set_locale(Locale::Tr);
    no_kg_app.set_heel_view_open(true);
    let mut w_no_kg = Window::with_app(no_kg_app, egui::vec2(1600.0, 1000.0));
    assert_eq!(w_no_kg.app.kg(), None);
    w_no_kg.frame(vec![]);
    assert!(
        w_no_kg.paints_substring("Hidrostatik panelinde KG değerini girin")
            || w_no_kg.shows_substring("Hidrostatik panelinde KG değerini girin"),
        "expected Turkish missing-KG hint"
    );
    let _ = std::fs::remove_dir_all(&no_kg_dir);

    // Builder panels window via File menu.
    w.click("Dosya");
    w.click("Kontrplak paneller…");
    w.frame(vec![]);
    for text in [
        "Kontrplak paneller",
        "Katmanlar:",
        "Açınabilir işaretli katmanlar",
        "Yerleşim (Nesting):",
        "Kontrplak plaka:",
        "Dışa aktar…",
    ] {
        assert!(
            w.shows_substring(text) || w.paints_substring(text),
            "expected Turkish Builder panels text `{text}`"
        );
    }
    w.app.set_show_panels(false);
    w.frame(vec![]);

    // Minimum supported window size check with open Analysis views.
    let mut small = App::headless_for_test(None);
    small.set_subdivision_level(1);
    small.set_locale(Locale::Tr);
    small.set_kg(2.2);
    small.open_analysis_view(AnalysisView::Heel);
    small.compute_heel_blocking().unwrap();
    let mut w_small = Window::with_app(small, egui::vec2(900.0, 600.0));
    w_small.app.open_analysis_view(AnalysisView::Waves);
    w_small.app.compute_waves_blocking().unwrap();
    w_small.frame(vec![]);
}

#[test]
fn turkish_example_picker_and_design_report_export() {
    use seacraft_gui::app::ExportKind;

    let mut empty = App::headless_empty();
    empty.set_locale(Locale::Tr);
    let mut w_empty = Window::with_app(empty, egui::vec2(1600.0, 1000.0));
    w_empty.click("Yeni tasarım… Ctrl+N");
    w_empty.click("Örnek");
    for expected in [
        "10 m gezi yatı",
        "Fin salmalı, pala dümenli ve güverteli gezi yatı",
        "bu boydaki seri üretim tekneleri temsil eder",
    ] {
        assert!(
            w_empty.shows_substring(expected) || w_empty.paints_substring(expected),
            "expected Turkish example picker text `{expected}`"
        );
    }

    let dir = std::env::temp_dir().join(format!("seacraft-gui-report-i18n-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(2);
    app.set_locale(Locale::Tr);

    let html_path = dir.join("report-tr.html");
    app.write_report_blocking(&html_path).unwrap();
    let html = std::fs::read_to_string(&html_path).unwrap();
    assert!(html.contains("<html lang=\"tr\">"));
    assert!(html.contains(seacraft_core::not_validated_note(Locale::Tr)));
    assert!(html.contains("Ana Boyutlar ve Özellikler"));
    assert!(html.contains("Posta planı - postalar"));

    let svg_path = dir.join("lines-tr.svg");
    app.write_export(ExportKind::Linesplan, &svg_path).unwrap();
    let svg = std::fs::read_to_string(&svg_path).unwrap();
    assert!(svg.contains("Posta planı - postalar"), "{svg}");

    let _ = std::fs::remove_dir_all(&dir);
}

/// Decode every `Tj` string written by `seacraft_core::pdf` using the PDF's own ToUnicode CMap.
fn pdf_shown_text(bytes: &[u8]) -> Vec<String> {
    fn object_bytes(bytes: &[u8], n: usize) -> &[u8] {
        let s = String::from_utf8_lossy(bytes);
        let at = s.rfind("startxref\n").expect("startxref") + 10;
        let xref: usize = s[at..].lines().next().unwrap().parse().unwrap();
        let entries = bytes[xref + 5..].iter().position(|&b| b == b'\n').unwrap() + xref + 6;
        let offset: usize = std::str::from_utf8(&bytes[entries + 20 * n..entries + 20 * n + 10])
            .unwrap()
            .parse()
            .unwrap();
        let start = bytes[offset..]
            .windows(4)
            .position(|w| w == b"obj\n")
            .unwrap()
            + offset
            + 4;
        let end = bytes[start..]
            .windows(6)
            .position(|w| w == b"endobj")
            .unwrap()
            + start;
        &bytes[start..end]
    }
    fn stream_slice(obj: &[u8]) -> Option<&[u8]> {
        let rest = obj.strip_prefix(b"<< /Length ")?;
        let sp = rest.iter().position(|&b| b == b' ')?;
        let len: usize = std::str::from_utf8(&rest[..sp]).ok()?.trim().parse().ok()?;
        let start = obj.windows(10).position(|w| w == b">>\nstream\n")? + 10;
        obj.get(start..start + len)
    }

    let s = String::from_utf8_lossy(bytes);
    let mut map = std::collections::BTreeMap::new();
    if let Some(at) = s.find("/ToUnicode ") {
        let n: usize = s[at + 11..]
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap();
        if let Some(cmap) = stream_slice(object_bytes(bytes, n)) {
            for line in String::from_utf8_lossy(cmap).lines() {
                let Some((glyph, text)) = line
                    .strip_prefix('<')
                    .and_then(|l| l.split_once("> <"))
                    .and_then(|(g, t)| Some((g, t.strip_suffix('>')?)))
                else {
                    continue;
                };
                if glyph.len() != 4 || text.len() % 4 != 0 || (glyph == "0000" && text == "FFFF") {
                    continue;
                }
                let units: Vec<u16> = (0..text.len())
                    .step_by(4)
                    .map(|i| u16::from_str_radix(&text[i..i + 4], 16).unwrap())
                    .collect();
                map.insert(
                    u16::from_str_radix(glyph, 16).unwrap(),
                    String::from_utf16(&units).unwrap(),
                );
            }
        }
    }

    let size: usize = s[s.rfind("/Size ").unwrap() + 6..]
        .split_whitespace()
        .next()
        .unwrap()
        .parse()
        .unwrap();
    let mut out = Vec::new();
    for n in 1..size {
        let obj = object_bytes(bytes, n);
        if String::from_utf8_lossy(&obj[..obj.len().min(200)]).contains("/Length1") {
            continue;
        }
        let Some(stream) = stream_slice(obj) else {
            continue;
        };
        for line in String::from_utf8_lossy(stream).lines() {
            if let (Some(a), Some(b)) = (line.find('<'), line.find("> Tj")) {
                let hex = &line[a + 1..b];
                let decoded: String = (0..hex.len())
                    .step_by(4)
                    .map(|i| map[&u16::from_str_radix(&hex[i..i + 4], 16).unwrap()].as_str())
                    .collect();
                out.push(decoded);
            }
        }
    }
    out
}

#[test]
fn turkish_builder_exports_localize_offsets_templates_and_panels() {
    let dir =
        std::env::temp_dir().join(format!("seacraft-gui-builder-i18n-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();

    // Offsets PDF & Templates PDF/SVG on demo hull in Turkish.
    let mut app = App::headless_for_test(None);
    app.set_subdivision_level(2);
    app.set_locale(Locale::Tr);

    let offsets_path = dir.join("offsets-tr.pdf");
    app.write_offsets_pdf(
        &offsets_path,
        &seacraft_gui::commands::OffsetsParams::default(),
        false,
    )
    .unwrap();
    let offsets_bytes = std::fs::read(&offsets_path).unwrap();
    let offsets_text = pdf_shown_text(&offsets_bytes);
    let offsets_joined = offsets_text.join(" ");
    for want in [
        "ofset tablosu",
        "Posta",
        seacraft_core::not_validated_note(Locale::Tr),
    ] {
        assert!(
            offsets_text.iter().any(|t| t.contains(want)) || offsets_joined.contains(want),
            "missing `{want}` in offsets PDF: {offsets_text:?}"
        );
    }

    let mut tp = app.templates_params().clone();
    tp.scale = 0.1;
    app.set_templates_params(tp);

    let tpl_pdf_path = dir.join("templates-tr.pdf");
    app.export_templates_to(&tpl_pdf_path).unwrap();
    let tpl_pdf_bytes = std::fs::read(&tpl_pdf_path).unwrap();
    let tpl_pdf_text = pdf_shown_text(&tpl_pdf_bytes);
    let tpl_joined = tpl_pdf_text.join(" ");
    for want in ["Posta 1", "Ölçek 1:10"] {
        assert!(
            tpl_pdf_text.iter().any(|t| t.contains(want)) || tpl_joined.contains(want),
            "missing `{want}` in templates PDF"
        );
    }

    let tpl_svg_path = dir.join("templates-tr.svg");
    app.export_templates_to(&tpl_svg_path).unwrap();
    let tpl_svg = std::fs::read_to_string(&tpl_svg_path).unwrap();
    assert!(tpl_svg.contains("Posta 1"), "{tpl_svg}");

    // Builder panels window & export on Skiff4m in Turkish.
    let (skiff_surface, _) =
        seacraft_core::examples::build(seacraft_core::examples::Example::Skiff4m).expect("skiff");
    let skiff_path = dir.join("skiff.fef");
    std::fs::write(&skiff_path, seacraft_core::fef::write(&skiff_surface)).unwrap();
    let mut skiff_app = App::headless_for_test(Some(skiff_path.to_str().unwrap()));
    skiff_app.set_subdivision_level(2);
    skiff_app.set_locale(Locale::Tr);
    let mut w = Window::with_app(skiff_app, egui::vec2(1400.0, 900.0));
    w.app.open_builder_panels();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while w.app.panels_preview().is_none() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for Builder panels preview (failure={:?})",
            w.app.panels_failure()
        );
        std::thread::sleep(std::time::Duration::from_millis(20));
        w.frame(vec![]);
    }
    w.frame(vec![]);
    assert!(
        w.shows_substring("kontrplak plaka") || w.paints_substring("kontrplak plaka"),
        "expected Turkish plywood summary in Builder panels window"
    );

    let panels_pdf_path = dir.join("panels-tr.pdf");
    w.app.export_panels_to(&panels_pdf_path).unwrap();
    let panels_pdf_bytes = std::fs::read(&panels_pdf_path).unwrap();
    let panels_text = pdf_shown_text(&panels_pdf_bytes);
    let panels_joined = panels_text.join(" ");
    for want in [
        seacraft_core::panels::thickness_note(Locale::Tr),
        seacraft_core::not_validated_note(Locale::Tr),
        "Boyunca",
        "Ofsetler",
    ] {
        assert!(
            panels_text.iter().any(|t| t.contains(want)) || panels_joined.contains(want),
            "missing `{want}` in panels PDF"
        );
    }

    let _ = std::fs::remove_dir_all(&dir);
}
