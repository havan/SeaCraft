//! File > Open recent, headless (R5-9): opening and saving put the file on
//! top of the list (deduplicated, at most eight), a missing file is marked,
//! Clear list empties it, the list survives a restart through the storage,
//! and a headless app has no storage at all.

use eframe::egui;
use seacraft_gui::recent::{JsonFileStorage, RecentFiles, RECENT_CAP, STORAGE_KEY};
use seacraft_gui::App;

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

fn scratch_dir(name: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("seacraft-gui-recent-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// `n` copies of the demo model in `dir`.
fn models(dir: &std::path::Path, n: usize) -> Vec<std::path::PathBuf> {
    let text = std::fs::read_to_string(demo_path()).unwrap();
    (0..n)
        .map(|i| {
            let p = dir.join(format!("hull{i}.fef"));
            std::fs::write(&p, &text).unwrap();
            p
        })
        .collect()
}

#[test]
fn opening_files_orders_dedupes_caps_and_marks_missing_ones() {
    let dir = scratch_dir("order");
    let files = models(&dir, 10);
    let mut app = App::headless_empty();
    assert!(app.recent_files().is_empty());
    for f in &files {
        app.open_model(f).unwrap();
    }
    let list = app.recent_files().paths().to_vec();
    assert_eq!(list.len(), RECENT_CAP, "capped at eight");
    assert_eq!(list[0], files[9], "most recent first");
    assert_eq!(list[7], files[2]);
    // Reopening an older file moves it up, once.
    app.open_model(&files[4]).unwrap();
    let list = app.recent_files().paths().to_vec();
    assert_eq!(list[0], files[4]);
    assert_eq!(list.iter().filter(|p| **p == files[4]).count(), 1);
    assert_eq!(list.len(), RECENT_CAP);
    // A failed open is not remembered.
    let _ = app.open_model(&dir.join("nope.fef"));
    assert_eq!(app.recent_files().paths()[0], files[4]);
    // Saving under a new name remembers it.
    let saved = dir.join("saved-as.fef");
    assert!(app.save_to(&saved));
    assert_eq!(app.recent_files().paths()[0], saved);
    // A deleted file stays listed, marked missing.
    std::fs::remove_file(&files[9]).unwrap();
    let entries = app.recent_files().entries();
    let gone = entries.iter().find(|e| e.path == files[9]).unwrap();
    assert!(!gone.exists, "a deleted file is marked missing");
    assert!(entries
        .iter()
        .filter(|e| e.path != files[9])
        .all(|e| e.exists));
    app.clear_recent();
    assert!(app.recent_files().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn the_list_survives_a_restart_through_the_storage() {
    let dir = scratch_dir("persist");
    let files = models(&dir, 3);
    let store = dir.join("state").join(JsonFileStorage::FILE_NAME);

    let mut first = App::headless_empty();
    first.set_storage(Box::new(JsonFileStorage::open(&store)));
    for f in &files {
        first.open_model(f).unwrap();
    }
    // Written as soon as it changed, not only at a clean exit.
    assert!(store.exists(), "the storage file was written");
    drop(first);

    let mut second = App::headless_empty();
    second.set_storage(Box::new(JsonFileStorage::open(&store)));
    let want: Vec<_> = files.iter().rev().cloned().collect();
    assert_eq!(second.recent_files().paths(), want.as_slice());

    // Clear list is kept too.
    second.clear_recent();
    let third = {
        let mut a = App::headless_empty();
        a.set_storage(Box::new(JsonFileStorage::open(&store)));
        a
    };
    assert!(third.recent_files().is_empty());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn eframe_save_writes_the_list_to_the_given_storage() {
    let dir = scratch_dir("eframe");
    let files = models(&dir, 2);
    let mut app = App::headless_empty();
    for f in &files {
        app.open_model(f).unwrap();
    }
    let mut storage = JsonFileStorage::open(dir.join("eframe.json"));
    eframe::App::save(&mut app, &mut storage);
    let text = eframe::Storage::get_string(&storage, STORAGE_KEY).expect("stored");
    let back = RecentFiles::from_json(&text);
    assert_eq!(back.paths(), [files[1].clone(), files[0].clone()]);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn headless_apps_have_no_storage() {
    let mut app = App::headless_for_test(Some(&demo_path()));
    assert!(
        app.take_storage().is_none(),
        "tests never touch the user's list"
    );
    let mut empty = App::headless_empty();
    assert!(empty.take_storage().is_none());
}

#[test]
fn the_start_card_lists_recent_files_and_opens_one() {
    let dir = scratch_dir("card");
    let files = models(&dir, 2);
    let mut app = App::headless_empty();
    app.set_storage(Box::new(JsonFileStorage::open(dir.join("s.json"))));
    for f in &files {
        app.open_model(f).unwrap();
    }
    // Back to the start card with the list: a fresh app on the same storage.
    let mut app = App::headless_empty();
    app.set_storage(Box::new(JsonFileStorage::open(dir.join("s.json"))));
    let ctx = egui::Context::default();
    let frame = |app: &mut App, events: Vec<egui::Event>| {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::pos2(0.0, 0.0),
                egui::vec2(1600.0, 1000.0),
            )),
            events,
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
    };
    frame(&mut app, vec![]);
    frame(&mut app, vec![]);
    let rects = app.start_rects().expect("the start card");
    assert!(rects.recent[0].is_positive() && rects.recent[1].is_positive());
    assert!(!rects.recent[2].is_positive(), "only two files listed");
    let pos = rects.recent[1].center();
    let button = |pressed| egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::NONE,
    };
    frame(&mut app, vec![egui::Event::PointerMoved(pos)]);
    frame(&mut app, vec![button(true)]);
    frame(&mut app, vec![button(false)]);
    assert_eq!(
        app.path(),
        Some(files[0].as_path()),
        "the older file opened"
    );
    assert_eq!(
        app.recent_files().paths()[0],
        files[0],
        "and moved to the top"
    );
    let _ = std::fs::remove_dir_all(dir);
}
