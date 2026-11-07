//! File > Open recent: the last [`RECENT_CAP`] models opened or saved, most
//! recent first, kept across runs in an [`eframe::Storage`].
//!
//! eframe only hands an app its storage with the `persistence` feature,
//! which pulls in crates SeaCraft does not otherwise use; [`JsonFileStorage`]
//! is the same trait over one small JSON file in the folder eframe would
//! use ([`data_dir`]), written atomically. `App::save` writes the list
//! to whichever storage it is given, so either works. Headless apps (tests)
//! have no storage and never touch the user's list.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use eframe::egui;

use crate::theme;

/// How many recent files are kept.
pub const RECENT_CAP: usize = 8;

/// The storage key the list is kept under (a JSON array of paths).
pub const STORAGE_KEY: &str = "recent_files";

/// One entry of the list as the menu shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RecentEntry {
    pub path: PathBuf,
    /// The file is there now; a missing one is greyed out.
    pub exists: bool,
}

impl RecentEntry {
    /// The file name, for the menu (the full path is the tooltip).
    pub fn label(&self) -> String {
        self.path
            .file_name()
            .map(|n| n.to_string_lossy().to_string())
            .unwrap_or_else(|| self.path.to_string_lossy().to_string())
    }
}

/// The recent-files list, most recent first, without duplicates, at most
/// [`RECENT_CAP`] long.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RecentFiles {
    paths: Vec<PathBuf>,
}

/// `path` made absolute (against the working directory) without resolving
/// links or requiring the file to exist, so the same file opened by a
/// relative and an absolute path is one entry, and a deleted one stays listed.
pub fn normalise(path: &Path) -> PathBuf {
    std::path::absolute(path).unwrap_or_else(|_| path.to_path_buf())
}

impl RecentFiles {
    /// A list from paths, most recent first (normalised, deduplicated, capped).
    pub fn from_paths(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        let mut list = Self::default();
        let paths: Vec<PathBuf> = paths.into_iter().collect();
        // Pushed oldest first, so the first path ends up on top.
        for p in paths.iter().rev() {
            list.push(p);
        }
        list
    }

    /// Put `path` on top, removing an older entry for the same file and the
    /// oldest entry past the cap.
    pub fn push(&mut self, path: &Path) {
        let path = normalise(path);
        self.paths.retain(|p| *p != path);
        self.paths.insert(0, path);
        self.paths.truncate(RECENT_CAP);
    }

    /// Most recent first.
    pub fn paths(&self) -> &[PathBuf] {
        &self.paths
    }

    /// The entries with whether each file exists now.
    pub fn entries(&self) -> Vec<RecentEntry> {
        self.paths
            .iter()
            .map(|p| RecentEntry {
                path: p.clone(),
                exists: p.is_file(),
            })
            .collect()
    }

    pub fn is_empty(&self) -> bool {
        self.paths.is_empty()
    }

    pub fn clear(&mut self) {
        self.paths.clear();
    }

    /// The list as stored: a JSON array of path strings.
    pub fn to_json(&self) -> String {
        let strings: Vec<String> = self
            .paths
            .iter()
            .map(|p| p.to_string_lossy().to_string())
            .collect();
        serde_json::to_string(&strings).unwrap_or_else(|_| "[]".to_owned())
    }

    /// The list from [`RecentFiles::to_json`]'s text; anything unreadable
    /// gives an empty list rather than an error (it is a convenience, and a
    /// bad file must not stop the app from starting).
    pub fn from_json(text: &str) -> Self {
        let strings: Vec<String> = serde_json::from_str(text).unwrap_or_default();
        Self::from_paths(
            strings
                .into_iter()
                .filter(|s| !s.trim().is_empty())
                .map(PathBuf::from),
        )
    }

    /// The list kept in `storage`, or an empty one.
    pub fn load(storage: &dyn eframe::Storage) -> Self {
        storage
            .get_string(STORAGE_KEY)
            .map(|text| Self::from_json(&text))
            .unwrap_or_default()
    }

    /// Keep the list in `storage` (the caller flushes).
    pub fn save(&self, storage: &mut dyn eframe::Storage) {
        storage.set_string(STORAGE_KEY, self.to_json());
    }
}

/// An [`eframe::Storage`] in one JSON object file (key → string value).
pub struct JsonFileStorage {
    path: PathBuf,
    values: BTreeMap<String, String>,
    dirty: bool,
    /// Why the last flush could not write the file (read back through
    /// [`FLUSH_ERROR_KEY`]).
    flush_error: Option<String>,
}

/// The key under which a storage reports why its last flush failed: not a
/// stored value, but readable through `dyn eframe::Storage`, so the app can
/// log the failure instead of losing the list silently.
pub const FLUSH_ERROR_KEY: &str = "seacraft.flush_error";

impl JsonFileStorage {
    /// The file name inside [`eframe::storage_dir`].
    pub const FILE_NAME: &'static str = "seacraft-gui.json";

    /// Open (or start) the storage at `path`. A missing or unreadable file
    /// starts empty; it is replaced on the next flush.
    pub fn open(path: impl Into<PathBuf>) -> Self {
        let path = path.into();
        let values = std::fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str(&text).ok())
            .unwrap_or_default();
        Self {
            path,
            values,
            dirty: false,
            flush_error: None,
        }
    }

    /// The storage in the user's data directory
    /// (`~/.local/share/seacraft/seacraft-gui.json` on Linux), if there is one.
    pub fn open_default() -> Option<Self> {
        data_dir().map(|dir| Self::open(dir.join(Self::FILE_NAME)))
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

/// The folder eframe's own storage uses for an app called "SeaCraft"
/// (`eframe::storage_dir`, which is only built with its `persistence`
/// feature): `$XDG_DATA_HOME/seacraft` or `~/.local/share/seacraft` on
/// Linux, `~/Library/Application Support/SeaCraft` on macOS,
/// `%APPDATA%\SeaCraft\data` on Windows.
pub fn data_dir() -> Option<PathBuf> {
    let var = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
    };
    if cfg!(target_os = "windows") {
        var("APPDATA").map(|p| p.join("SeaCraft").join("data"))
    } else if cfg!(target_os = "macos") {
        var("HOME").map(|p| {
            p.join("Library")
                .join("Application Support")
                .join("SeaCraft")
        })
    } else {
        var("XDG_DATA_HOME")
            .or_else(|| var("HOME").map(|h| h.join(".local").join("share")))
            .map(|p| p.join("seacraft"))
    }
}

impl eframe::Storage for JsonFileStorage {
    fn get_string(&self, key: &str) -> Option<String> {
        if key == FLUSH_ERROR_KEY {
            return self.flush_error.clone();
        }
        self.values.get(key).cloned()
    }

    fn set_string(&mut self, key: &str, value: String) {
        if self.values.get(key) != Some(&value) {
            self.values.insert(key.to_owned(), value);
            self.dirty = true;
        }
    }

    fn remove_string(&mut self, key: &str) {
        if self.values.remove(key).is_some() {
            self.dirty = true;
        }
    }

    /// Write the file if anything changed. A failure is not fatal: the
    /// list is a convenience, and it stays in memory for this run; the
    /// reason is kept under [`FLUSH_ERROR_KEY`] for the app to log.
    fn flush(&mut self) {
        if !self.dirty {
            return;
        }
        if let Some(dir) = self.path.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let text = serde_json::to_string_pretty(&self.values).unwrap_or_else(|_| "{}".to_owned());
        match seacraft_core::io::write_atomic(&self.path, text) {
            Ok(()) => {
                self.dirty = false;
                self.flush_error = None;
            }
            Err(e) => {
                self.flush_error = Some(format!("cannot write {}: {e}", self.path.display()));
            }
        }
    }
}

/// What the Open recent menu asked for; `App` applies it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecentAction {
    None,
    Open(PathBuf),
    Clear,
}

/// The items of File > Open recent (inside the submenu): the files, most
/// recent first, missing ones greyed with a tooltip, and Clear list.
pub fn recent_menu(ui: &mut egui::Ui, entries: &[RecentEntry]) -> RecentAction {
    let mut action = RecentAction::None;
    if entries.is_empty() {
        ui.label(egui::RichText::new("No recent files").color(theme::TEXT_MUTED));
    }
    for (i, entry) in entries.iter().enumerate() {
        let text = format!("{}  {}", i + 1, entry.label());
        let full = entry.path.display().to_string();
        let response = ui.add_enabled(entry.exists, egui::Button::new(text));
        if entry.exists {
            if response.on_hover_text(full).clicked() {
                action = RecentAction::Open(entry.path.clone());
            }
        } else {
            response.on_disabled_hover_text(format!("{full}\nThe file is missing."));
        }
    }
    ui.separator();
    if ui
        .add_enabled(!entries.is_empty(), egui::Button::new("Clear list"))
        .clicked()
    {
        action = RecentAction::Clear;
    }
    action
}

#[cfg(test)]
mod tests {
    use super::*;

    // `/tmp/x` is not absolute on Windows, where it gains the current drive.
    fn abs(name: &str) -> PathBuf {
        normalise(Path::new(name))
    }

    #[test]
    fn push_puts_the_newest_first_dedupes_and_caps() {
        let mut list = RecentFiles::default();
        for i in 0..10 {
            list.push(Path::new(&format!("/tmp/r{i}.fef")));
        }
        assert_eq!(list.paths().len(), RECENT_CAP);
        assert_eq!(list.paths()[0], abs("/tmp/r9.fef"));
        assert_eq!(list.paths()[7], abs("/tmp/r2.fef"));
        // Opening an older one again moves it to the top, once.
        list.push(Path::new("/tmp/r5.fef"));
        assert_eq!(list.paths()[0], abs("/tmp/r5.fef"));
        assert_eq!(list.paths().len(), RECENT_CAP);
        assert_eq!(
            list.paths()
                .iter()
                .filter(|p| **p == abs("/tmp/r5.fef"))
                .count(),
            1
        );
    }

    #[test]
    fn a_relative_and_an_absolute_path_to_one_file_are_one_entry() {
        let mut list = RecentFiles::default();
        list.push(Path::new("hull.fef"));
        list.push(&abs("hull.fef"));
        assert_eq!(list.paths(), [abs("hull.fef")]);
    }

    #[test]
    fn json_round_trip_and_garbage_gives_an_empty_list() {
        let list = RecentFiles::from_paths([
            PathBuf::from("/a/one.fef"),
            PathBuf::from("/b/two.fef"),
            PathBuf::from("/a/one.fef"),
        ]);
        assert_eq!(list.paths(), [abs("/a/one.fef"), abs("/b/two.fef")]);
        assert_eq!(RecentFiles::from_json(&list.to_json()), list);
        assert!(RecentFiles::from_json("not json").is_empty());
        assert!(RecentFiles::from_json("{\"a\": 1}").is_empty());
    }
}
