//! Headless acceptance tests for periodic autosave and crash recovery.

use std::fs::{self, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};

use eframe::egui;
use seacraft_core::fef;
use seacraft_core::project::Project;
use seacraft_gui::autosave::{
    find_orphans, snapshot_path, SnapshotFile, AUTOSAVE_INTERVAL, SNAPSHOT_SCHEMA,
};
use seacraft_gui::messages::Level;
use seacraft_gui::App;

/// Safely past the interval: an edit starts its timer after a test takes `t0`.
const DUE: Duration = AUTOSAVE_INTERVAL.saturating_add(Duration::from_secs(1));

static NEXT_DIR: AtomicU64 = AtomicU64::new(1);

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// Unique temporary directory per test so parallel tests never collide.
struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> Self {
        let id = NEXT_DIR.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "seacraft-autosave-test-{}-{tag}-{id}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Ok(meta) = fs::metadata(&self.0) {
                let mut perms = meta.permissions();
                perms.set_mode(0o755);
                let _ = fs::set_permissions(&self.0, perms);
            }
        }
        let _ = fs::remove_dir_all(&self.0);
    }
}

/// Join any in-flight autosave background write without sleeping, then run a
/// tick to apply the joined result.
fn wait_for_write(app: &mut App, now: Instant) {
    app.wait_autosave();
    app.autosave_tick(now);
}

fn json_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("json"))
        .collect();
    out.sort();
    out
}

fn lock_files(dir: &Path) -> Vec<PathBuf> {
    let Ok(rd) = fs::read_dir(dir) else {
        return Vec::new();
    };
    let mut out: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|s| s.to_str()) == Some("lock"))
        .collect();
    out.sort();
    out
}

fn frame(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
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
    out
}

// 1. An edit plus a tick before the interval writes nothing; after the
//    interval a snapshot appears whose `fef` equals `fef::write(surface)` and
//    whose project equals `project.to_json()`.
#[test]
fn test_1_edit_ticks_before_and_after_interval_write_snapshot() {
    let tmp = TempDir::new("t1");
    let autosave_dir = tmp.path().join("autosave");
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_autosave_dir(autosave_dir.clone());
    assert!(!app.is_dirty());

    let t0 = Instant::now();
    app.autosave_tick(t0);
    assert!(json_files(&autosave_dir).is_empty());

    // Edit both geometry and project.
    app.snapshot_undo();
    app.surface_mut().unwrap().points[0].p.z += 0.25;
    app.mark_geometry_changed();
    app.set_kg(2.45);
    assert!(app.is_dirty());

    // Tick before the 30 s interval writes nothing.
    let before_due = t0 + Duration::from_secs(15);
    app.autosave_tick(before_due);
    wait_for_write(&mut app, before_due);
    assert!(
        json_files(&autosave_dir).is_empty(),
        "no snapshot should be written before AUTOSAVE_INTERVAL"
    );

    // Tick at/after the interval writes the snapshot.
    let after_due = t0 + DUE;
    app.autosave_tick(after_due);
    wait_for_write(&mut app, after_due);

    let jsons = json_files(&autosave_dir);
    assert_eq!(jsons.len(), 1, "expected 1 snapshot file, got {jsons:?}");
    assert_eq!(lock_files(&autosave_dir).len(), 1, "expected 1 lock file");

    let raw = fs::read_to_string(&jsons[0]).unwrap();
    let snap = SnapshotFile::from_json(&raw).expect("valid snapshot JSON");
    assert_eq!(snap.schema, SNAPSHOT_SCHEMA);
    assert_eq!(snap.app_version, seacraft_core::VERSION);
    assert_eq!(snap.fef, fef::write(app.surface().unwrap()));
    assert_eq!(snap.project, app.project().to_json());
    assert_eq!(
        snap.original_path.as_deref(),
        Some(demo_path().as_str()),
        "original_path recorded"
    );
}

// 2. No write during a drag; no second write when nothing changed since the
//    last snapshot.
#[test]
fn test_2_no_write_during_drag_and_no_second_write_when_unchanged() {
    let tmp = TempDir::new("t2");
    let autosave_dir = tmp.path().join("autosave");
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_autosave_dir(autosave_dir.clone());

    let ctx = egui::Context::default();
    frame(&ctx, &mut app, Vec::new());
    frame(&ctx, &mut app, Vec::new());

    // Locate a sheer control point in the profile viewport and start dragging it
    // without releasing the pointer button.
    let index = {
        let surface = app.surface().unwrap();
        surface
            .points
            .iter()
            .enumerate()
            .filter(|(_, p)| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.1.p.z.total_cmp(&b.1.p.z))
            .map(|(i, _)| i)
            .unwrap()
    };
    let pt = app.surface().unwrap().points[index].p;
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1].transform().project(pt, rect).unwrap();
    let press = egui::Event::PointerButton {
        pos: screen,
        button: egui::PointerButton::Primary,
        pressed: true,
        modifiers: egui::Modifiers::default(),
    };
    frame(&ctx, &mut app, vec![egui::Event::PointerMoved(screen)]);
    frame(&ctx, &mut app, vec![press]);
    frame(
        &ctx,
        &mut app,
        vec![egui::Event::PointerMoved(screen + egui::vec2(25.0, 0.0))],
    );
    assert!(app.is_dirty(), "drag dirtied the model");

    // Even well past the interval, no snapshot may be written while a drag is in progress.
    let t_drag = Instant::now() + DUE * 2;
    app.autosave_tick(t_drag);
    wait_for_write(&mut app, t_drag);
    assert!(
        json_files(&autosave_dir).is_empty(),
        "must not autosave while a drag is in progress"
    );

    // Release the drag.
    let release = egui::Event::PointerButton {
        pos: screen + egui::vec2(25.0, 0.0),
        button: egui::PointerButton::Primary,
        pressed: false,
        modifiers: egui::Modifiers::default(),
    };
    frame(&ctx, &mut app, vec![release]);

    // Tick after release writes the snapshot once.
    let t1 = t_drag + DUE;
    app.autosave_tick(t1);
    wait_for_write(&mut app, t1);
    let jsons = json_files(&autosave_dir);
    assert_eq!(jsons.len(), 1, "snapshot written after drag release");

    // Replace the on-disk file with a marker or check that a subsequent due tick
    // without further edits does not spawn a second write.
    fs::write(&jsons[0], "MARKER_UNCHANGED").unwrap();
    let t2 = t1 + DUE * 2;
    app.autosave_tick(t2);
    assert!(
        !app.autosave_in_flight(),
        "no second write thread should be spawned when nothing changed"
    );
    wait_for_write(&mut app, t2);
    assert_eq!(
        fs::read_to_string(&jsons[0]).unwrap(),
        "MARKER_UNCHANGED",
        "snapshot on disk was not rewritten when model text was unchanged"
    );
}

// 3. A save removes the snapshot; undo back to the saved state removes it too.
#[test]
fn test_3_save_and_undo_to_saved_state_remove_snapshot() {
    let tmp = TempDir::new("t3");
    let autosave_dir = tmp.path().join("autosave");
    let saved_fef = tmp.path().join("saved.fef");
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_autosave_dir(autosave_dir.clone());

    // Edit and produce an autosave snapshot.
    let t0 = Instant::now();
    app.snapshot_undo();
    app.surface_mut().unwrap().points[1].p.z += 0.15;
    app.mark_geometry_changed();
    let t1 = t0 + DUE;
    app.autosave_tick(t1);
    wait_for_write(&mut app, t1);
    assert_eq!(json_files(&autosave_dir).len(), 1);
    assert_eq!(lock_files(&autosave_dir).len(), 1);

    // Saving clears dirty; the next tick removes <session>.json while keeping <session>.lock.
    assert!(app.save_to(&saved_fef));
    assert!(!app.is_dirty());
    app.autosave_tick(t1 + Duration::from_secs(1));
    assert!(
        json_files(&autosave_dir).is_empty(),
        "saving removes <session>.json in the tick"
    );
    assert_eq!(
        lock_files(&autosave_dir).len(),
        1,
        "<session>.lock stays until shutdown"
    );

    // Edit again -> snapshot reappears after interval.
    let t2 = t1 + Duration::from_secs(2);
    app.snapshot_undo();
    app.surface_mut().unwrap().points[2].p.z += 0.2;
    app.mark_geometry_changed();
    assert!(app.is_dirty());
    let t3 = t2 + DUE;
    app.autosave_tick(t3);
    wait_for_write(&mut app, t3);
    assert_eq!(json_files(&autosave_dir).len(), 1);

    // Undo back to the saved state -> clean again -> tick removes <session>.json.
    app.undo();
    assert!(!app.is_dirty(), "undo back to saved state clears dirty");
    app.autosave_tick(t3 + Duration::from_secs(1));
    assert!(
        json_files(&autosave_dir).is_empty(),
        "undo back to saved state removes <session>.json"
    );
    assert_eq!(
        lock_files(&autosave_dir).len(),
        1,
        "<session>.lock still kept until shutdown"
    );
}

// 4. `autosave_shutdown` removes both files. Dropping an app without shutdown
//    leaves the snapshot, and a new app with the same dir offers it (the lock
//    is released on drop).
#[test]
fn test_4_shutdown_removes_both_files_and_drop_without_shutdown_offers_orphan() {
    let tmp = TempDir::new("t4");
    let autosave_dir = tmp.path().join("autosave");

    // Part A: clean shutdown removes both .json and .lock.
    {
        let mut app = App::headless_for_test(Some(&demo_path()));
        app.set_autosave_dir(autosave_dir.clone());
        let t0 = Instant::now();
        app.surface_mut().unwrap().points[0].p.z += 0.1;
        app.mark_geometry_changed();
        let t1 = t0 + DUE;
        app.autosave_tick(t1);
        wait_for_write(&mut app, t1);
        assert_eq!(json_files(&autosave_dir).len(), 1);
        assert_eq!(lock_files(&autosave_dir).len(), 1);

        app.autosave_shutdown();
        assert!(
            json_files(&autosave_dir).is_empty(),
            "shutdown removes .json"
        );
        assert!(
            lock_files(&autosave_dir).is_empty(),
            "shutdown removes .lock"
        );
    }

    // Part B: dropping an app without autosave_shutdown leaves the snapshot and
    // releases the OS file lock so a new app offers it.
    let session_name = {
        let mut crashed = App::headless_for_test(Some(&demo_path()));
        crashed.set_autosave_dir(autosave_dir.clone());
        let t0 = Instant::now();
        crashed.surface_mut().unwrap().points[0].p.z += 0.3;
        crashed.mark_geometry_changed();
        let t1 = t0 + DUE;
        crashed.autosave_tick(t1);
        wait_for_write(&mut crashed, t1);
        assert_eq!(json_files(&autosave_dir).len(), 1);
        assert_eq!(lock_files(&autosave_dir).len(), 1);
        crashed.autosave_session().to_owned()
        // `crashed` is dropped here without `autosave_shutdown`.
    };

    assert_eq!(json_files(&autosave_dir).len(), 1);
    assert_eq!(lock_files(&autosave_dir).len(), 1);

    let mut next_app = App::headless_empty();
    next_app.set_autosave_dir(autosave_dir.clone());
    assert!(
        next_app.autosave_prompt_open(),
        "new app with the same dir offers the orphaned snapshot"
    );
    assert_eq!(
        next_app.current_orphan().unwrap().session,
        session_name,
        "offered orphan matches dropped session"
    );
}

// 5. A live session's snapshot (its lock held by another open file handle) is
//    NOT offered.
#[test]
fn test_5_live_session_snapshot_is_not_offered() {
    let tmp = TempDir::new("t5");
    let autosave_dir = tmp.path().join("autosave");

    let mut live_app = App::headless_for_test(Some(&demo_path()));
    live_app.set_autosave_dir(autosave_dir.clone());
    let t0 = Instant::now();
    live_app.surface_mut().unwrap().points[0].p.z += 0.2;
    live_app.mark_geometry_changed();
    let t1 = t0 + DUE;
    live_app.autosave_tick(t1);
    wait_for_write(&mut live_app, t1);
    assert_eq!(json_files(&autosave_dir).len(), 1);
    assert_eq!(lock_files(&autosave_dir).len(), 1);

    // Also create a second live session manually holding an exclusive lock on its .lock file.
    let other_lock_path = autosave_dir.join("session-99999-1.lock");
    let other_json_path = autosave_dir.join("session-99999-1.json");
    let other_lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(&other_lock_path)
        .unwrap();
    other_lock.try_lock().unwrap();
    let snap = SnapshotFile {
        schema: SNAPSHOT_SCHEMA,
        app_version: seacraft_core::VERSION.to_owned(),
        original_path: None,
        saved_at_unix: 100,
        fef: fef::write(live_app.surface().unwrap()),
        project: Project::default().to_json(),
    };
    fs::write(&other_json_path, snap.to_json()).unwrap();

    // While both locks are held, neither session is offered as an orphan.
    assert!(find_orphans(&autosave_dir).is_empty());
    let mut second_app = App::headless_empty();
    second_app.set_autosave_dir(autosave_dir.clone());
    assert!(
        !second_app.autosave_prompt_open(),
        "live sessions with held locks must not be offered"
    );

    drop(other_lock);
    live_app.autosave_shutdown();
}

// 6. Recover restores geometry, project and path with `dirty == true`, leaves a
//    fresh snapshot of our session, and removes the orphan; a following `save()`
//    writes to the original path. The same with no original path: `path` is `None`.
#[test]
fn test_6_recover_with_and_without_original_path() {
    let tmp = TempDir::new("t6");
    let autosave_dir = tmp.path().join("autosave");
    let orig_fef = tmp.path().join("vessel.fef");
    fs::copy(demo_path(), &orig_fef).unwrap();

    // Case A: model with an original_path.
    let (expected_fef, expected_project, crashed_session) = {
        let mut crashed = App::headless_for_test(Some(orig_fef.to_str().unwrap()));
        crashed.set_autosave_dir(autosave_dir.clone());
        crashed.surface_mut().unwrap().points[4].p.z += 0.42;
        crashed.mark_geometry_changed();
        crashed.set_kg(3.15);
        let t1 = Instant::now() + DUE;
        crashed.autosave_tick(t1);
        wait_for_write(&mut crashed, t1);
        (
            fef::write(crashed.surface().unwrap()),
            crashed.project().clone(),
            crashed.autosave_session().to_owned(),
        )
    };

    let orphan_json = autosave_dir.join(format!("{crashed_session}.json"));
    let orphan_lock = autosave_dir.join(format!("{crashed_session}.lock"));
    assert!(orphan_json.exists());

    let mut app = App::headless_empty();
    app.set_autosave_dir(autosave_dir.clone());
    assert!(app.autosave_prompt_open());
    app.recover_orphan();
    assert!(!app.autosave_prompt_open());
    assert!(app.is_dirty(), "recovered model must be dirty");
    assert_eq!(app.path(), Some(orig_fef.as_path()));
    assert_eq!(fef::write(app.surface().unwrap()), expected_fef);
    assert_eq!(app.project(), &expected_project);
    assert_eq!(app.kg(), Some(3.15));

    // Orphan files were removed and replaced by a fresh snapshot of `app`'s own session.
    assert!(!orphan_json.exists(), "orphan .json removed after recover");
    assert!(!orphan_lock.exists(), "orphan .lock removed after recover");
    let own_json = autosave_dir.join(format!("{}.json", app.autosave_session()));
    assert!(
        own_json.exists(),
        "recover wrote a fresh snapshot for our session"
    );

    // A following `save()` writes to the original path without opening a dialog.
    app.save();
    assert!(!app.dialog_open());
    assert!(!app.is_dirty());
    assert_eq!(fs::read_to_string(&orig_fef).unwrap(), expected_fef);
    app.autosave_shutdown();

    // Case B: untitled model with no original_path (`path` is `None`).
    let (untitled_fef, untitled_proj, untitled_session) = {
        let mut crashed = App::headless_for_test(None);
        crashed.set_autosave_dir(autosave_dir.clone());
        crashed.surface_mut().unwrap().points[2].p.z += 0.33;
        crashed.mark_geometry_changed();
        crashed.set_kg(1.95);
        let t1 = Instant::now() + DUE;
        crashed.autosave_tick(t1);
        wait_for_write(&mut crashed, t1);
        (
            fef::write(crashed.surface().unwrap()),
            crashed.project().clone(),
            crashed.autosave_session().to_owned(),
        )
    };
    let untitled_json = autosave_dir.join(format!("{untitled_session}.json"));
    assert!(untitled_json.exists());

    let mut app2 = App::headless_empty();
    app2.set_autosave_dir(autosave_dir.clone());
    assert!(app2.autosave_prompt_open());
    assert!(app2.current_orphan().unwrap().original_path.is_none());
    app2.recover_orphan();
    assert!(!app2.autosave_prompt_open());
    assert!(app2.is_dirty());
    assert!(app2.path().is_none(), "untitled recovery keeps path None");
    assert_eq!(fef::write(app2.surface().unwrap()), untitled_fef);
    assert_eq!(app2.project(), &untitled_proj);
    assert!(!untitled_json.exists());
    let own_json2 = autosave_dir.join(format!("{}.json", app2.autosave_session()));
    assert!(own_json2.exists());
    app2.autosave_shutdown();
}

// 7. Discard deletes; Decide later keeps the files and a new app offers them again.
#[test]
fn test_7_discard_deletes_and_decide_later_keeps_for_next_start() {
    let tmp = TempDir::new("t7");
    let autosave_dir = tmp.path().join("autosave");

    // Create two orphaned sessions so we can Discard the newest and Decide later on the second.
    let session_older = {
        let mut a = App::headless_for_test(None);
        a.set_autosave_dir(autosave_dir.clone());
        let t = Instant::now() + DUE;
        a.autosave_tick(t);
        wait_for_write(&mut a, t);
        a.autosave_session().to_owned()
    };
    let session_newer = {
        let mut b = App::headless_for_test(Some(&demo_path()));
        b.set_autosave_dir(autosave_dir.clone());
        b.surface_mut().unwrap().points[0].p.z += 0.1;
        b.mark_geometry_changed();
        let t = Instant::now() + DUE;
        b.autosave_tick(t);
        wait_for_write(&mut b, t);
        b.autosave_session().to_owned()
    };

    let older_json = autosave_dir.join(format!("{session_older}.json"));
    let newer_json = autosave_dir.join(format!("{session_newer}.json"));
    assert!(older_json.exists() && newer_json.exists());

    let mut app = App::headless_empty();
    app.set_autosave_dir(autosave_dir.clone());
    assert_eq!(app.orphans().len(), 2);
    assert_eq!(
        app.current_orphan().unwrap().session,
        session_newer,
        "newest orphan offered first"
    );

    // Discard deletes the first orphan and advances to the second.
    app.discard_orphan();
    assert!(!newer_json.exists(), "discarded orphan deleted from disk");
    assert!(
        app.autosave_prompt_open(),
        "prompt stays open for the next orphan"
    );
    assert_eq!(app.current_orphan().unwrap().session, session_older);

    // Decide later closes the prompt and keeps the remaining orphan on disk.
    app.defer_orphans();
    assert!(!app.autosave_prompt_open());
    assert!(older_json.exists(), "deferred orphan kept on disk");

    // A new app offers the kept orphan again.
    let mut next_app = App::headless_empty();
    next_app.set_autosave_dir(autosave_dir);
    assert!(next_app.autosave_prompt_open());
    assert_eq!(next_app.current_orphan().unwrap().session, session_older);
}

// 8. A corrupt snapshot is skipped with a warning and left on disk. A bad
//    project string recovers the geometry and keeps the orphan. Original file
//    newer than the snapshot is reported (expose it on the orphan's struct).
#[test]
fn test_8_corrupt_snapshot_bad_project_and_original_newer() {
    let tmp = TempDir::new("t8");
    let autosave_dir = tmp.path().join("autosave");
    fs::create_dir_all(&autosave_dir).unwrap();

    // 8a. Corrupt JSON and wrong-schema snapshots are skipped with a warning and left on disk.
    let corrupt_json = autosave_dir.join("session-100-1.json");
    fs::write(&corrupt_json, "{ not valid json").unwrap();
    let bad_schema_json = autosave_dir.join("session-100-2.json");
    fs::write(
        &bad_schema_json,
        r#"{"schema": 99, "app_version": "0.1.0", "saved_at_unix": 1, "fef": "", "project": ""}"#,
    )
    .unwrap();

    // Valid JSON around a .fef that cannot parse: Recover could only ever fail.
    let bad_fef_json = autosave_dir.join("session-100-3.json");
    let bad_fef = SnapshotFile {
        schema: SNAPSHOT_SCHEMA,
        app_version: seacraft_core::VERSION.to_owned(),
        original_path: None,
        saved_at_unix: 1,
        fef: "not a fef".to_owned(),
        project: Project::default().to_json(),
    };
    fs::write(&bad_fef_json, bad_fef.to_json()).unwrap();

    let mut app = App::headless_empty();
    app.set_autosave_dir(autosave_dir.clone());
    assert!(
        !app.autosave_prompt_open(),
        "corrupt, wrong-schema and unparseable snapshots must not be offered"
    );
    assert!(bad_fef_json.exists(), "unparseable snapshot left on disk");
    fs::remove_file(&bad_fef_json).unwrap();
    assert!(corrupt_json.exists(), "corrupt snapshot left on disk");
    assert!(
        bad_schema_json.exists(),
        "wrong-schema snapshot left on disk"
    );
    assert!(
        app.messages().count(Level::Warning) >= 2,
        "warnings logged for unreadable/wrong-schema snapshots"
    );
    fs::remove_file(&corrupt_json).unwrap();
    fs::remove_file(&bad_schema_json).unwrap();

    // 8b. A bad project string recovers the geometry, uses a default project,
    // logs Level::Warning, and keeps the orphan file on disk.
    let orig_file = tmp.path().join("ship.fef");
    fs::copy(demo_path(), &orig_file).unwrap();
    let mut edited_surface = seacraft_core::demo::demo_hull(false);
    edited_surface.points[0].p.z += 0.55;
    let edited_fef = fef::write(&edited_surface);

    let bad_proj_json = autosave_dir.join("session-200-1.json");
    let snap = SnapshotFile {
        schema: SNAPSHOT_SCHEMA,
        app_version: seacraft_core::VERSION.to_owned(),
        original_path: Some(orig_file.to_string_lossy().to_string()),
        saved_at_unix: 1_000, // older than orig_file's mtime on disk
        fef: edited_fef.clone(),
        project: "{ broken project json".to_owned(),
    };
    fs::write(&bad_proj_json, snap.to_json()).unwrap();

    // 8c. Original file on disk is newer than `saved_at_unix = 1_000`.
    let mut app2 = App::headless_empty();
    app2.set_autosave_dir(autosave_dir.clone());
    assert!(app2.autosave_prompt_open());
    let orphan = app2.current_orphan().unwrap();
    assert!(
        orphan.original_newer && orphan.original_newer(),
        "original file newer than snapshot is reported on the orphan struct"
    );

    let warn_before = app2.messages().count(Level::Warning);
    app2.recover_orphan();
    assert!(!app2.autosave_prompt_open());
    assert_eq!(
        fef::write(app2.surface().unwrap()),
        edited_fef,
        "geometry recovered despite bad project string"
    );
    assert_eq!(app2.project(), &Project::default(), "default project used");
    assert!(
        app2.messages().count(Level::Warning) > warn_before,
        "Level::Warning logged for bad project"
    );
    assert!(
        bad_proj_json.exists(),
        "orphan with bad project string is kept on disk"
    );
    app2.autosave_shutdown();
}

// 9. `App::new_detached` with edits and ticks creates nothing in any directory
//    (no dir set means no writes).
#[test]
fn test_9_new_detached_writes_nothing_anywhere() {
    let mut app = App::new_detached();
    assert!(app.autosave_dir().is_none());
    app.load_demo();
    app.surface_mut().unwrap().points[0].p.z += 0.2;
    app.mark_geometry_changed();
    app.set_kg(2.4);
    assert!(app.is_dirty());

    let t0 = Instant::now();
    app.autosave_tick(t0);
    app.autosave_tick(t0 + DUE);
    app.autosave_tick(t0 + DUE * 2);
    assert!(!app.autosave_in_flight());
    assert!(app.autosave_dir().is_none());
    assert_eq!(app.messages().count(Level::Warning), 0);
}

// 10. An unwritable directory logs exactly one warning over several due ticks,
//     and the app keeps working.
#[test]
fn test_10_unwritable_directory_logs_one_warning_and_keeps_working() {
    let tmp = TempDir::new("t10");
    // Place a regular file where the autosave directory path points so
    // `create_dir_all` fails deterministically on every platform (even under root).
    let blocked_path = tmp.path().join("not-a-directory");
    fs::write(&blocked_path, "file blocking directory creation").unwrap();
    let unwritable_dir = blocked_path.join("autosave");

    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_autosave_dir(unwritable_dir);

    let t0 = Instant::now();
    app.surface_mut().unwrap().points[0].p.z += 0.1;
    app.mark_geometry_changed();

    // Run several due ticks; each attempts a write that fails, but only the
    // first logs a warning.
    for step in 1..=4 {
        let t = t0 + DUE * step;
        app.surface_mut().unwrap().points[0].p.z += 0.05;
        app.mark_geometry_changed();
        app.autosave_tick(t);
        wait_for_write(&mut app, t);
    }

    assert_eq!(
        app.messages().count(Level::Warning),
        1,
        "unwritable directory must log exactly one warning across multiple due ticks: {:?}",
        app.messages().entries().collect::<Vec<_>>()
    );

    // The app keeps working: hydrostatics still compute and edits still apply.
    app.recompute_hydrostatics();
    assert!(app.hydrostatics().is_some());

    // When a writable directory is configured, the next due tick succeeds and
    // logs that autosave works again.
    let working_dir = tmp.path().join("working-autosave");
    app.set_autosave_dir(working_dir.clone());
    let t_recover = t0 + DUE * 6;
    app.surface_mut().unwrap().points[0].p.z += 0.05;
    app.mark_geometry_changed();
    app.autosave_tick(t_recover);
    wait_for_write(&mut app, t_recover);
    assert_eq!(json_files(&working_dir).len(), 1);
    assert!(
        app.messages()
            .entries()
            .any(|m| m.text.contains("working again")),
        "recovery from write failure logged: {:?}",
        app.messages().entries().collect::<Vec<_>>()
    );
    app.autosave_shutdown();
}

// 11. A crashed session whose sidecar could not be read: recovering must not
//     let a later save overwrite that sidecar, which `load` also protects.
#[test]
fn test_11_recover_leaves_an_unreadable_sidecar_alone() {
    let tmp = TempDir::new("sidecar");
    let autosave_dir = tmp.path().join("autosave");
    let model = tmp.path().join("hull.fef");
    fs::copy(demo_path(), &model).unwrap();
    let sidecar = seacraft_core::project::sidecar_path(&model);
    fs::write(&sidecar, "{ not json").unwrap();
    {
        let mut crashed = App::headless_for_test(Some(model.to_str().unwrap()));
        crashed.set_autosave_dir(autosave_dir.clone());
        let t0 = Instant::now();
        // Design data entered since the failed load, which a save would write.
        crashed.project_mut_for_test().kg = Some(2.0);
        crashed.surface_mut().unwrap().points[0].p.z += 0.3;
        crashed.mark_geometry_changed();
        crashed.autosave_tick(t0 + DUE);
        wait_for_write(&mut crashed, t0 + DUE);
    }
    let before = fs::read_to_string(&model).unwrap();
    let mut app = App::new_detached();
    app.set_autosave_dir(autosave_dir);
    assert!(app.autosave_prompt_open());
    app.recover_orphan();
    assert_eq!(app.project().kg, Some(2.0), "the design data is recovered");
    app.save();
    assert_ne!(
        fs::read_to_string(&model).unwrap(),
        before,
        "the model saved"
    );
    assert_eq!(fs::read_to_string(&sidecar).unwrap(), "{ not json");
}

// 12. A dead session that left only its lock file is swept at startup; a live
//     session's lock stays.
#[test]
fn test_12_stale_lock_files_are_swept_and_live_ones_kept() {
    let tmp = TempDir::new("sweep");
    let dir = tmp.path().join("autosave");
    fs::create_dir_all(&dir).unwrap();
    let dead = dir.join("session-1-1.lock");
    let live = dir.join("session-2-2.lock");
    fs::write(&dead, "").unwrap();
    let holder = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&live)
        .unwrap();
    holder.try_lock().unwrap();
    let mut app = App::new_detached();
    app.set_autosave_dir(dir);
    assert!(!dead.exists(), "the dead session's lock is removed");
    assert!(live.exists(), "the live session's lock stays");
}

#[test]
fn snapshot_paths_are_absolute() {
    // A session started elsewhere must find the same file, not its namesake.
    let recorded = snapshot_path(Path::new("hull.fef")).unwrap();
    assert_eq!(
        PathBuf::from(recorded),
        std::env::current_dir().unwrap().join("hull.fef")
    );
    assert_eq!(snapshot_path(Path::new("/a/b.fef")).unwrap(), "/a/b.fef");
}

#[cfg(unix)]
#[test]
fn a_path_json_cannot_carry_is_not_recorded_altered() {
    use std::os::unix::ffi::OsStrExt;
    let path = Path::new(std::ffi::OsStr::from_bytes(b"/tmp/boat_\xff.fef"));
    assert_eq!(snapshot_path(path), None);
}

#[test]
fn an_edit_that_moves_no_revision_still_wakes_the_idle_app() {
    use seacraft_gui::autosave::AutosaveState;
    let tmp = TempDir::new("idle");
    let mut state = AutosaveState::new();
    state.set_dir(tmp.path().join("autosave"), false);
    let revs = (1, 1);
    let t0 = Instant::now();
    state.note_dirty();
    let payload = || Some(("fef".to_owned(), "project".to_owned(), None));
    state.tick(t0 + DUE, true, false, revs, payload);
    state.join_write();
    state.tick(t0 + DUE, true, false, revs, payload);
    assert_eq!(
        state.repaint_after(t0 + DUE, true, false, revs),
        None,
        "written and unchanged: nothing to wake for"
    );

    // A KG or a report option edit: dirty already, the revisions do not move.
    state.note_dirty();
    assert!(
        state.repaint_after(t0 + DUE, true, false, revs).is_some(),
        "the edit must reach a snapshot without further input"
    );
    state.tick(t0 + DUE + DUE, true, false, revs, payload);
    state.join_write();
    assert_eq!(state.repaint_after(t0 + DUE + DUE, true, false, revs), None);
}
