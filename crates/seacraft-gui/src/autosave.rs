//! Periodic autosave and startup crash recovery.
//!
//! Each GUI session owns a pair of files in [`autosave_dir`], named
//! `session-<pid>-<unix-nanos>.lock` and `session-<pid>-<unix-nanos>.json`.
//!
//! * The `.lock` file is created at the first snapshot and held open and
//!   exclusively locked with [`std::fs::File::try_lock`] for the life of the
//!   process. When the process exits — cleanly, by a signal or by a crash —
//!   the operating system drops the lock, so any snapshot whose lock file can
//!   be locked (or has no lock file) belongs to a dead session.
//! * The `.json` file is written atomically through
//!   [`seacraft_core::io::write_atomic`] on a background thread so `fsync`
//!   never stalls a frame. Its project payload is kept as an embedded JSON
//!   string so a schema error in the project cannot make the geometry
//!   unreadable.

use std::fs::{self, File, OpenOptions};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread::JoinHandle;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use eframe::egui;
use serde::{Deserialize, Serialize};

use crate::messages::clock;
use crate::theme;

/// How long unsaved work sits before an autosave snapshot is written.
pub const AUTOSAVE_INTERVAL: Duration = Duration::from_secs(30);

/// Schema version of `<session>.json`.
pub const SNAPSHOT_SCHEMA: u32 = 1;

/// Monotonic nanosecond counter so two sessions started in the same clock tick
/// in one process never share a session name.
static LAST_SESSION_NANOS: AtomicU64 = AtomicU64::new(0);

/// Generate a unique session identifier `session-<pid>-<unix-nanos>`.
pub fn new_session_id() -> String {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_nanos() as u64);
    let mut prev = LAST_SESSION_NANOS.load(Ordering::Relaxed);
    let nanos = loop {
        let next = now.max(prev.saturating_add(1));
        match LAST_SESSION_NANOS.compare_exchange_weak(
            prev,
            next,
            Ordering::Relaxed,
            Ordering::Relaxed,
        ) {
            Ok(_) => break next,
            Err(actual) => prev = actual,
        }
    };
    format!("session-{}-{nanos}", std::process::id())
}

/// Default autosave directory:
/// * Linux / Unix: `$XDG_STATE_HOME/seacraft/autosave` (absolute values only),
///   else `$HOME/.local/state/seacraft/autosave`
/// * Windows: `%LOCALAPPDATA%\SeaCraft\autosave`
/// * macOS: `~/Library/Application Support/SeaCraft/autosave`
pub fn autosave_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if cfg!(target_os = "windows") {
        var("LOCALAPPDATA").map(|p| p.join("SeaCraft").join("autosave"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|p| {
            p.join("Library")
                .join("Application Support")
                .join("SeaCraft")
                .join("autosave")
        })
    } else {
        var("XDG_STATE_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".local").join("state")))
            .map(|p| p.join("seacraft").join("autosave"))
    }
}

/// On-disk JSON payload of `<session>.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFile {
    pub schema: u32,
    pub app_version: String,
    #[serde(default)]
    pub original_path: Option<String>,
    pub saved_at_unix: u64,
    pub fef: String,
    pub project: String,
}

impl SnapshotFile {
    /// Parse a snapshot JSON string, checking `schema == 1` before reading
    /// the remaining fields.
    pub fn from_json(text: &str) -> Result<Self, String> {
        let value: serde_json::Value =
            serde_json::from_str(text).map_err(|e| format!("not valid JSON: {e}"))?;
        match value.get("schema").and_then(|v| v.as_u64()) {
            Some(s) if s == u64::from(SNAPSHOT_SCHEMA) => {}
            Some(other) => {
                return Err(format!(
                    "unsupported autosave schema {other} (expected {SNAPSHOT_SCHEMA})"
                ));
            }
            None => {
                return Err(format!(
                    "missing `schema` field (expected {SNAPSHOT_SCHEMA})"
                ));
            }
        }
        serde_json::from_value(value).map_err(|e| format!("invalid snapshot fields: {e}"))
    }

    pub fn to_json(&self) -> String {
        let mut text = serde_json::to_string_pretty(self).expect("snapshot serialises");
        text.push('\n');
        text
    }
}

/// `path` as a snapshot records it: absolute, because the session that
/// recovers it may start in another directory, and a relative path would then
/// name another file. `None` for a path JSON cannot carry (not UTF-8): the
/// recovered model then asks where to save rather than writing to an altered
/// name.
pub fn snapshot_path(path: &Path) -> Option<String> {
    std::path::absolute(path).ok()?.to_str().map(str::to_owned)
}

/// An orphaned session snapshot discovered at startup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Orphan {
    pub session: String,
    pub json_path: PathBuf,
    pub lock_path: PathBuf,
    pub app_version: String,
    pub model_name: String,
    pub original_path: Option<PathBuf>,
    pub saved_at_unix: u64,
    /// True when `original_path` exists on disk and its modification time is
    /// newer than the autosave snapshot.
    pub original_newer: bool,
    pub fef: String,
    pub project: String,
}

impl Orphan {
    /// Whether the original `.fef` file on disk is newer than this autosave.
    pub fn original_newer(&self) -> bool {
        self.original_newer
    }

    /// Delete both `<session>.json` and `<session>.lock` for this orphan.
    pub fn remove_files(&self) {
        let _ = fs::remove_file(&self.json_path);
        let _ = fs::remove_file(&self.lock_path);
    }
}

/// Find every orphaned `session-*.json` in `dir`, newest first.
///
/// A snapshot is an orphan when its `.lock` file does not exist or can be
/// exclusively locked with [`File::try_lock`]. Unreadable or wrong-schema
/// snapshots are skipped and left on disk.
pub fn find_orphans(dir: &Path) -> Vec<Orphan> {
    scan_orphans(dir, None).0
}

/// Like [`find_orphans`], also skipping `own_session` and returning warnings
/// for unreadable or wrong-schema snapshots so the caller can log them.
pub fn scan_orphans(dir: &Path, own_session: Option<&str>) -> (Vec<Orphan>, Vec<String>) {
    let mut found: Vec<(Orphan, u64, Option<SystemTime>)> = Vec::new();
    let mut warnings = Vec::new();

    let Ok(read_dir) = fs::read_dir(dir) else {
        return (Vec::new(), warnings);
    };

    let mut entries: Vec<_> = read_dir.filter_map(|e| e.ok()).collect();
    entries.sort_by_key(|e| e.file_name());

    for entry in entries {
        let file_name = entry.file_name();
        let Some(name) = file_name.to_str() else {
            continue;
        };
        let Some(session) = name
            .strip_prefix("session-")
            .and_then(|s| s.strip_suffix(".json"))
            .map(|_| name.trim_end_matches(".json"))
        else {
            continue;
        };
        if own_session == Some(session) {
            continue;
        }
        let json_path = entry.path();
        let lock_path = dir.join(format!("{session}.lock"));
        if !is_lock_obtainable(&lock_path) {
            continue;
        }
        let text = match fs::read_to_string(&json_path) {
            Ok(t) => t,
            Err(e) => {
                warnings.push(format!(
                    "unreadable autosave snapshot {}: {e}",
                    json_path.display()
                ));
                continue;
            }
        };
        let snap = match SnapshotFile::from_json(&text) {
            Ok(s) => s,
            Err(e) => {
                warnings.push(format!(
                    "skipping autosave snapshot {}: {e}",
                    json_path.display()
                ));
                continue;
            }
        };
        // Nothing a Recover could use: leave the file on disk and say so,
        // rather than offer it on every start and fail every time.
        if let Err(e) = seacraft_core::fef::parse(&snap.fef) {
            warnings.push(format!(
                "skipping autosave snapshot {}: invalid .fef ({e})",
                json_path.display()
            ));
            continue;
        }
        let original_path = snap
            .original_path
            .as_deref()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        let json_mtime = fs::metadata(&json_path).and_then(|m| m.modified()).ok();
        let original_newer = original_path
            .as_deref()
            .is_some_and(|orig| is_original_newer(orig, snap.saved_at_unix, json_mtime));
        let model_name = seacraft_core::fef::parse(&snap.fef)
            .ok()
            .map(|s| s.settings.name)
            .filter(|n| !n.trim().is_empty())
            .or_else(|| {
                original_path
                    .as_ref()
                    .and_then(|p| p.file_stem())
                    .map(|s| s.to_string_lossy().to_string())
            })
            .unwrap_or_else(|| "untitled".to_owned());
        let nanos = session_nanos(session);
        found.push((
            Orphan {
                session: session.to_owned(),
                json_path,
                lock_path,
                app_version: snap.app_version,
                model_name,
                original_path,
                saved_at_unix: snap.saved_at_unix,
                original_newer,
                fef: snap.fef,
                project: snap.project,
            },
            nanos,
            json_mtime,
        ));
    }

    // Newest first: by saved_at_unix, then by session nanosecond timestamp,
    // then by file mtime.
    found.sort_by(|a, b| {
        b.0.saved_at_unix
            .cmp(&a.0.saved_at_unix)
            .then_with(|| b.1.cmp(&a.1))
            .then_with(|| b.2.cmp(&a.2))
            .then_with(|| b.0.session.cmp(&a.0.session))
    });

    let orphans = found.into_iter().map(|(o, _, _)| o).collect();
    (orphans, warnings)
}

fn session_nanos(session: &str) -> u64 {
    session
        .rsplit('-')
        .next()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(0)
}

/// True when `lock_path` does not exist or can be locked with `File::try_lock`.
fn is_lock_obtainable(lock_path: &Path) -> bool {
    if !lock_path.exists() {
        return true;
    }
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(lock_path)
        .or_else(|_| File::open(lock_path));
    let Ok(file) = file else {
        return !lock_path.exists();
    };
    match file.try_lock() {
        Ok(()) => {
            let _ = file.unlock();
            true
        }
        Err(_) => false,
    }
}

/// Remove lock files of dead sessions that left no snapshot (one that crashed
/// while its model was clean): nothing else would ever delete them.
fn sweep_stale_locks(dir: &Path) {
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };
    for entry in read_dir.filter_map(|e| e.ok()) {
        let path = entry.path();
        let is_lock = path.extension().is_some_and(|e| e == "lock")
            && entry.file_name().to_string_lossy().starts_with("session-");
        if is_lock && !path.with_extension("json").exists() && is_lock_obtainable(&path) {
            let _ = fs::remove_file(&path);
        }
    }
}

/// Check whether `orig` has been modified after the snapshot at `saved_at_unix`.
fn is_original_newer(orig: &Path, saved_at_unix: u64, json_mtime: Option<SystemTime>) -> bool {
    let Ok(orig_mtime) = fs::metadata(orig).and_then(|m| m.modified()) else {
        return false;
    };
    let orig_secs = orig_mtime
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    if orig_secs > saved_at_unix {
        return true;
    }
    // When both writes happened within the same wall-clock second (e.g. in a
    // fast test), fall back to comparing the two files' sub-second mtimes.
    if orig_secs == saved_at_unix {
        if let Some(jm) = json_mtime {
            return orig_mtime > jm;
        }
    }
    false
}

/// Open (or create) `lock_path` and acquire an exclusive non-blocking lock.
fn open_and_lock(lock_path: &Path) -> Result<File, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
        .map_err(|e| format!("cannot open lock {}: {e}", lock_path.display()))?;
    file.try_lock()
        .map_err(|e| format!("cannot lock {}: {e}", lock_path.display()))?;
    Ok(file)
}

/// Result of a background snapshot write.
struct WrittenSnapshot {
    fef: String,
    project: String,
    saved_at_unix: u64,
    revs: (u64, u64),
    model_gen: u64,
    lock_file: Option<File>,
}

/// Outcome reported to `App` when `AutosaveState` polls or runs a tick.
pub struct TickReport {
    /// Log messages `(level, text)` produced during the tick.
    pub logs: Vec<(crate::messages::Level, String)>,
}

/// Per-app autosave state machine and recovery queue.
pub struct AutosaveState {
    session: String,
    dir: Option<PathBuf>,
    lock_file: Option<File>,
    has_snapshot: bool,
    /// When the current dirty spell began or the last due check/snapshot ran.
    timer_anchor: Option<Instant>,
    /// The `(fef, project)` strings of the snapshot currently on disk.
    last_snapshot_text: Option<(String, String)>,
    /// `(revision, project_rev)` of the snapshot currently on disk.
    last_saved_revs: Option<(u64, u64)>,
    /// An edit since the last payload was built. Not every project edit moves
    /// the revisions (a KG or a report option does not, or the loading results
    /// would go stale), so the revisions alone cannot end the idle wake-up.
    edited_since_payload: bool,
    /// Unix timestamp (seconds) of the last completed snapshot.
    last_saved_at: Option<u64>,
    /// Background write thread, if one is running.
    in_flight: Option<JoinHandle<Result<WrittenSnapshot, String>>>,
    /// True once a write failure has been logged; cleared on the next success.
    write_failed: bool,
    /// Bumped whenever the whole model is replaced (Open, New demo, Discard)
    /// so an older model's snapshot is removed on the next tick.
    model_gen: u64,
    /// True when the model was replaced while a previous snapshot was on disk.
    obsolete_snapshot: bool,
    /// Orphans waiting for the user's decision at startup, newest first.
    pub orphans: Vec<Orphan>,
}

impl Default for AutosaveState {
    fn default() -> Self {
        Self::new()
    }
}

impl AutosaveState {
    pub fn new() -> Self {
        Self {
            session: new_session_id(),
            dir: None,
            lock_file: None,
            has_snapshot: false,
            timer_anchor: None,
            last_snapshot_text: None,
            last_saved_revs: None,
            edited_since_payload: false,
            last_saved_at: None,
            in_flight: None,
            write_failed: false,
            model_gen: 0,
            obsolete_snapshot: false,
            orphans: Vec::new(),
        }
    }

    pub fn session(&self) -> &str {
        &self.session
    }

    pub fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    pub fn json_path(&self) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(format!("{}.json", self.session)))
    }

    pub fn lock_path(&self) -> Option<PathBuf> {
        self.dir
            .as_ref()
            .map(|d| d.join(format!("{}.lock", self.session)))
    }

    /// Enable autosave in `dir` and scan `dir` for orphaned sessions.
    /// Returns any warnings from unreadable or wrong-schema snapshots.
    pub fn set_dir(&mut self, dir: PathBuf, dirty: bool) -> Vec<String> {
        let (orphans, warnings) = scan_orphans(&dir, Some(&self.session));
        sweep_stale_locks(&dir);
        self.dir = Some(dir);
        self.orphans = orphans;
        if dirty && self.timer_anchor.is_none() {
            self.note_dirty();
        }
        warnings
    }

    /// Whether a background write is currently running.
    pub fn in_flight(&self) -> bool {
        self.in_flight.is_some()
    }

    /// Note that the model became dirty.
    pub fn note_dirty(&mut self) {
        self.edited_since_payload = true;
        if self.timer_anchor.is_none() {
            self.timer_anchor = Some(Instant::now());
        }
    }

    /// Note that the model is now clean (saved, opened, or undone to saved).
    pub fn note_clean(&mut self) {
        self.timer_anchor = None;
    }

    /// Note that the whole model was replaced (New demo, New design, Open,
    /// Discard). Any snapshot from the previous model is marked for removal
    /// on the next tick, and the timer restarts if the new model is dirty.
    pub fn note_model_replaced(&mut self, dirty: bool) {
        self.model_gen += 1;
        if self.has_snapshot || self.in_flight.is_some() {
            self.obsolete_snapshot = true;
        }
        self.last_snapshot_text = None;
        self.last_saved_revs = None;
        self.last_saved_at = None;
        self.timer_anchor = None;
        if dirty {
            self.note_dirty();
        }
    }

    /// Hover text for the status bar's `● unsaved` indicator.
    pub fn hover_text(&self) -> String {
        match self.last_saved_at {
            Some(secs) => {
                let time = UNIX_EPOCH + Duration::from_secs(secs);
                format!("Last autosave at {} UTC", clock(time))
            }
            None => "No autosave has happened yet".to_owned(),
        }
    }

    /// Poll a finished background write without blocking.
    pub fn poll_write(&mut self) -> Vec<(crate::messages::Level, String)> {
        let finished = self.in_flight.as_ref().is_some_and(|h| h.is_finished());
        if !finished {
            return Vec::new();
        }
        self.join_write()
    }

    /// Block until any in-flight write finishes and apply its outcome.
    pub fn join_write(&mut self) -> Vec<(crate::messages::Level, String)> {
        let Some(handle) = self.in_flight.take() else {
            return Vec::new();
        };
        let mut logs = Vec::new();
        match handle.join() {
            Ok(Ok(written)) => {
                if let Some(lock) = written.lock_file {
                    self.lock_file = Some(lock);
                }
                self.has_snapshot = true;
                if written.model_gen == self.model_gen {
                    self.last_snapshot_text = Some((written.fef, written.project));
                    self.last_saved_at = Some(written.saved_at_unix);
                    self.last_saved_revs = Some(written.revs);
                } else {
                    self.obsolete_snapshot = true;
                }
                if self.write_failed {
                    self.write_failed = false;
                    logs.push((
                        crate::messages::Level::Info,
                        "autosave is working again".to_owned(),
                    ));
                }
            }
            Ok(Err(err)) => {
                if !self.write_failed {
                    self.write_failed = true;
                    logs.push((
                        crate::messages::Level::Warning,
                        format!("autosave failed: {err}"),
                    ));
                }
            }
            Err(_) => {
                if !self.write_failed {
                    self.write_failed = true;
                    logs.push((
                        crate::messages::Level::Warning,
                        "autosave worker panicked".to_owned(),
                    ));
                }
            }
        }
        logs
    }

    /// Remove `<session>.json` when not dirty or when a previous model's
    /// snapshot was superseded. Called only from `tick` while no write is in
    /// flight.
    fn remove_snapshot_file(&mut self) {
        if let Some(json_path) = self.json_path() {
            if self.has_snapshot || json_path.exists() {
                let _ = fs::remove_file(&json_path);
            }
        }
        self.has_snapshot = false;
        self.obsolete_snapshot = false;
        self.last_snapshot_text = None;
        self.last_saved_revs = None;
        self.last_saved_at = None;
    }

    /// Run one autosave state-machine tick.
    pub fn tick(
        &mut self,
        now: Instant,
        dirty: bool,
        dragging: bool,
        revs: (u64, u64),
        build_payload: impl FnOnce() -> Option<(String, String, Option<String>)>,
    ) -> TickReport {
        let logs = self.poll_write();

        let Some(dir) = self.dir.clone() else {
            return TickReport { logs };
        };

        if !dirty {
            self.timer_anchor = None;
            if self.in_flight.is_none() && (self.has_snapshot || self.obsolete_snapshot) {
                self.remove_snapshot_file();
            }
            return TickReport { logs };
        }

        if self.obsolete_snapshot && self.in_flight.is_none() {
            self.remove_snapshot_file();
        }

        let anchor = match self.timer_anchor {
            Some(a) if now < a => {
                self.timer_anchor = Some(now);
                now
            }
            Some(a) => a,
            None => {
                self.timer_anchor = Some(now);
                now
            }
        };

        if dragging || self.in_flight.is_some() {
            return TickReport { logs };
        }

        if now.saturating_duration_since(anchor) < AUTOSAVE_INTERVAL {
            return TickReport { logs };
        }

        self.timer_anchor = Some(now);
        self.edited_since_payload = false;
        let Some((fef, project, original_path)) = build_payload() else {
            return TickReport { logs };
        };

        if self
            .last_snapshot_text
            .as_ref()
            .is_some_and(|(last_fef, last_proj)| *last_fef == fef && *last_proj == project)
        {
            self.last_saved_revs = Some(revs);
            return TickReport { logs };
        }

        let session = self.session.clone();
        let need_lock = self.lock_file.is_none();
        let model_gen = self.model_gen;
        let saved_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let snapshot = SnapshotFile {
            schema: SNAPSHOT_SCHEMA,
            app_version: seacraft_core::VERSION.to_owned(),
            original_path,
            saved_at_unix,
            fef: fef.clone(),
            project: project.clone(),
        };
        let bytes = snapshot.to_json();

        self.in_flight = Some(std::thread::spawn(move || {
            write_snapshot_files(&dir, &session, need_lock, &bytes).map(|lock_file| {
                WrittenSnapshot {
                    fef,
                    project,
                    saved_at_unix,
                    revs,
                    model_gen,
                    lock_file,
                }
            })
        }));

        TickReport { logs }
    }

    /// Write our own session's snapshot synchronously (used by `Recover`
    /// before deleting an orphan).
    pub fn write_sync(
        &mut self,
        fef: String,
        project: String,
        original_path: Option<String>,
        revs: (u64, u64),
    ) -> Result<(), String> {
        let _ = self.join_write();
        let Some(dir) = self.dir.clone() else {
            return Ok(());
        };
        let need_lock = self.lock_file.is_none();
        let saved_at_unix = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |d| d.as_secs());
        let snapshot = SnapshotFile {
            schema: SNAPSHOT_SCHEMA,
            app_version: seacraft_core::VERSION.to_owned(),
            original_path,
            saved_at_unix,
            fef: fef.clone(),
            project: project.clone(),
        };
        let bytes = snapshot.to_json();
        let lock = write_snapshot_files(&dir, &self.session, need_lock, &bytes)?;
        if let Some(lock) = lock {
            self.lock_file = Some(lock);
        }
        self.has_snapshot = true;
        self.obsolete_snapshot = false;
        self.last_snapshot_text = Some((fef, project));
        self.last_saved_at = Some(saved_at_unix);
        self.last_saved_revs = Some(revs);
        self.timer_anchor = Some(Instant::now());
        self.write_failed = false;
        Ok(())
    }

    /// How long until `App::draw` should wake an idle window for autosave.
    ///
    /// Returns `None` when disabled, clean, or when the current revision has
    /// already been snapshotted and no write is running (so an idle window
    /// never repaints continuously).
    pub fn repaint_after(
        &self,
        now: Instant,
        dirty: bool,
        dragging: bool,
        revs: (u64, u64),
    ) -> Option<Duration> {
        self.dir.as_ref()?;
        if self.in_flight.is_some() {
            return Some(Duration::from_millis(50));
        }
        if !dirty || dragging {
            return None;
        }
        if self.last_saved_revs == Some(revs) && !self.write_failed && !self.edited_since_payload {
            return None;
        }
        let anchor = self.timer_anchor?;
        let elapsed = now.saturating_duration_since(anchor);
        Some(
            AUTOSAVE_INTERVAL
                .saturating_sub(elapsed)
                .max(Duration::from_millis(50)),
        )
    }

    /// Clean shutdown: join any in-flight write, remove both session files
    /// and drop the lock file handle.
    pub fn shutdown(&mut self) {
        let _ = self.join_write();
        if let Some(json_path) = self.json_path() {
            let _ = fs::remove_file(json_path);
        }
        if let Some(lock_path) = self.lock_path() {
            let _ = fs::remove_file(lock_path);
        }
        self.lock_file = None;
        self.has_snapshot = false;
        self.obsolete_snapshot = false;
        self.last_snapshot_text = None;
        self.last_saved_revs = None;
        self.last_saved_at = None;
        self.timer_anchor = None;
    }
}

fn write_snapshot_files(
    dir: &Path,
    session: &str,
    need_lock: bool,
    bytes: &str,
) -> Result<Option<File>, String> {
    fs::create_dir_all(dir).map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    let lock_path = dir.join(format!("{session}.lock"));
    let lock_file = if need_lock {
        Some(open_and_lock(&lock_path)?)
    } else {
        None
    };
    let json_path = dir.join(format!("{session}.json"));
    if let Err(e) = seacraft_core::io::write_atomic(&json_path, bytes) {
        if lock_file.is_some() {
            drop(lock_file);
            let _ = fs::remove_file(&lock_path);
        }
        return Err(format!("cannot write {}: {e}", json_path.display()));
    }
    Ok(lock_file)
}

/// User choice in the startup crash-recovery modal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecoveryAction {
    None,
    Recover,
    Discard,
    Defer,
}

/// Draw the crash-recovery modal for `orphan` (`remaining` is the total count
/// of orphans still queued, including `orphan`).
pub fn recovery_modal(ctx: &egui::Context, orphan: &Orphan, remaining: usize) -> RecoveryAction {
    let mut action = RecoveryAction::None;
    egui::Modal::new(egui::Id::unique("autosave_recovery_modal")).show(ctx, |ui| {
        ui.set_width(460.0);
        let title = if remaining > 1 {
            format!("Recover unsaved work? (1 of {remaining})")
        } else {
            "Recover unsaved work?".to_owned()
        };
        ui.label(
            egui::RichText::new(title)
                .heading()
                .color(theme::TEXT_PRIMARY),
        );
        ui.add_space(4.0);
        egui::Grid::new("autosave_recovery_grid")
            .num_columns(2)
            .spacing(egui::vec2(12.0, 4.0))
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Model").color(theme::TEXT_SECONDARY));
                ui.label(egui::RichText::new(&orphan.model_name).color(theme::TEXT_PRIMARY));
                ui.end_row();

                ui.label(egui::RichText::new("Original file").color(theme::TEXT_SECONDARY));
                let path_text = orphan
                    .original_path
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "untitled".to_owned());
                ui.label(egui::RichText::new(path_text).color(theme::TEXT_PRIMARY));
                ui.end_row();

                ui.label(egui::RichText::new("Autosaved at").color(theme::TEXT_SECONDARY));
                let saved_time = UNIX_EPOCH + Duration::from_secs(orphan.saved_at_unix);
                ui.monospace(format!("{} UTC", clock(saved_time)));
                ui.end_row();
            });
        if orphan.original_newer {
            ui.add_space(4.0);
            ui.colored_label(
                theme::WARN,
                "⚠ The file on disk is newer than this autosave.",
            );
        }
        ui.add_space(4.0);
        ui.label(
            egui::RichText::new("Undo history is not recovered.")
                .small()
                .color(theme::TEXT_MUTED),
        );
        ui.separator();
        ui.horizontal(|ui| {
            if ui.button("Recover").clicked() {
                action = RecoveryAction::Recover;
            }
            if ui.button("Discard").clicked() {
                action = RecoveryAction::Discard;
            }
            if ui.button("Decide later").clicked() {
                action = RecoveryAction::Defer;
            }
        });
    });
    action
}
