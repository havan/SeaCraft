//! User presets file (`connected-edit.json`) for connected editing.
//!
//! Built-in presets from [`seacraft_core::connected`] are always available; a
//! user's file next to `seacraft-gui.json` replaces built-ins by name and
//! appends extras. A missing, unreadable or invalid file never blocks start-up
//! or removes the built-ins.

use std::io;
use std::path::{Path, PathBuf};

use seacraft_core::connected::{
    builtin_presets, example_file, merged, parse_presets, ConnectedPreset,
};
use seacraft_core::io::write_atomic;

/// Loaded connected-editing presets, the file path they were read from, and
/// any error from reading or parsing that file.
#[derive(Debug, Clone, PartialEq)]
pub struct PresetStore {
    pub presets: Vec<ConnectedPreset>,
    pub error: Option<String>,
    pub path: Option<PathBuf>,
}

impl PresetStore {
    /// File name inside the application data directory.
    pub const FILE_NAME: &'static str = "connected-edit.json";

    /// Load presets from `dir/connected-edit.json`, or just the built-ins when
    /// `dir` is `None` or the file does not exist.
    pub fn load(dir: Option<&Path>) -> PresetStore {
        let path = dir.map(|d| d.join(Self::FILE_NAME));
        let (presets, error) = read_at(path.as_deref());
        PresetStore {
            presets,
            error,
            path,
        }
    }

    /// Re-read the presets file at `self.path`.
    pub fn reload(&mut self) {
        let (presets, error) = read_at(self.path.as_deref());
        self.presets = presets;
        self.error = error;
    }

    /// Write the example presets file if it does not already exist.
    pub fn write_example(&self) -> Result<PathBuf, String> {
        let Some(path) = self.path.as_deref() else {
            return Err("no data directory for connected-edit presets".into());
        };
        if path.exists() || path.symlink_metadata().is_ok() {
            return Err(format!(
                "{} already exists; refusing to overwrite it",
                path.display()
            ));
        }
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir)
                .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
        }
        write_atomic(path, example_file().as_bytes())
            .map_err(|e| format!("cannot write {}: {e}", path.display()))?;
        Ok(path.to_path_buf())
    }
}

fn read_at(path: Option<&Path>) -> (Vec<ConnectedPreset>, Option<String>) {
    let Some(path) = path else {
        return (builtin_presets(), None);
    };
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy())
        .unwrap_or_else(|| PresetStore::FILE_NAME.into());
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == io::ErrorKind::NotFound => {
            return (builtin_presets(), None);
        }
        Err(e) => {
            return (builtin_presets(), Some(format!("cannot read {name}: {e}")));
        }
    };
    match parse_presets(&text) {
        Ok(user) => (merged(user), None),
        Err(err) => (builtin_presets(), Some(format!("{name}: {err}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use seacraft_core::connected::{builtin_presets, parse_presets, Reach};
    use std::sync::atomic::{AtomicU64, Ordering};

    static COUNTER: AtomicU64 = AtomicU64::new(0);

    fn scratch_dir(name: &str) -> PathBuf {
        let n = COUNTER.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "seacraft-gui-presets-{name}-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_file_keeps_builtins_with_no_error() {
        let dir = scratch_dir("missing");
        let store = PresetStore::load(Some(&dir));
        assert_eq!(store.presets, builtin_presets());
        assert_eq!(store.error, None);
        assert_eq!(store.path, Some(dir.join(PresetStore::FILE_NAME)));

        let none = PresetStore::load(None);
        assert_eq!(none.presets, builtin_presets());
        assert_eq!(none.error, None);
        assert_eq!(none.path, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_file_keeps_builtins_and_names_faulty_preset() {
        let dir = scratch_dir("bad");
        let file = dir.join(PresetStore::FILE_NAME);
        std::fs::write(
            &file,
            r#"{"version": 1, "presets": [{"name": "Broken", "reach": {"m": -1.0}, "falloff": "smooth", "spread": "both"}]}"#,
        )
        .unwrap();

        let store = PresetStore::load(Some(&dir));
        assert_eq!(store.presets, builtin_presets());
        let err = store.error.as_deref().expect("error reported");
        assert!(err.contains("Broken"), "got: {err}");

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn unreadable_or_non_utf8_file_keeps_builtins_and_names_file() {
        let dir = scratch_dir("unreadable");
        let file = dir.join(PresetStore::FILE_NAME);
        std::fs::write(&file, [0xff, 0xfe, 0x00, 0x80]).unwrap();

        let store = PresetStore::load(Some(&dir));
        assert_eq!(store.presets, builtin_presets());
        let err = store.error.as_deref().expect("non-UTF-8 error reported");
        assert!(err.contains(PresetStore::FILE_NAME), "got: {err}");

        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::write(&file, "{}").unwrap();
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o000)).unwrap();
            let can_still_read = std::fs::read(&file).is_ok();
            if !can_still_read {
                let store = PresetStore::load(Some(&dir));
                assert_eq!(store.presets, builtin_presets());
                let err = store.error.as_deref().expect("permission error reported");
                assert!(err.contains(PresetStore::FILE_NAME), "got: {err}");
            }
            std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
        }

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn user_preset_replaces_builtin_and_reload_rereads_same_path() {
        let dir = scratch_dir("override");
        let file = dir.join(PresetStore::FILE_NAME);
        std::fs::write(
            &file,
            r#"{
              "version": 1,
              "presets": [
                {"name": "Nudge", "reach": {"m": 1.25}, "falloff": "linear", "spread": "both"},
                {"name": "Custom extra", "reach": {"m": 3.0}, "falloff": "smooth", "spread": "station"}
              ]
            }"#,
        )
        .unwrap();

        let mut store = PresetStore::load(Some(&dir));
        assert_eq!(store.error, None);
        let builtins = builtin_presets();
        assert_eq!(store.presets.len(), builtins.len() + 1);
        assert_eq!(store.presets[3].name, "Nudge");
        assert_eq!(store.presets[3].reach, Reach::Metres(1.25));
        assert_eq!(store.presets.last().unwrap().name, "Custom extra");

        // Break the file and reload: built-ins stay, error appears.
        std::fs::write(&file, "not json").unwrap();
        store.reload();
        assert_eq!(store.presets, builtins);
        assert!(store.error.is_some());

        // Remove the file and reload: built-ins stay, error clears.
        std::fs::remove_file(&file).unwrap();
        store.reload();
        assert_eq!(store.presets, builtins);
        assert_eq!(store.error, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn write_example_creates_valid_file_and_refuses_overwrite_or_no_dir() {
        let no_dir = PresetStore::load(None);
        let err = no_dir.write_example().expect_err("no data directory");
        assert!(!err.is_empty(), "got: {err}");

        let dir = scratch_dir("example");
        let nested = dir.join("nested");
        let store = PresetStore::load(Some(&nested));
        let written = store.write_example().expect("first write succeeds");
        assert_eq!(written, nested.join(PresetStore::FILE_NAME));
        assert!(written.is_file());

        let text = std::fs::read_to_string(&written).unwrap();
        let parsed = parse_presets(&text).expect("written example parses");
        assert!(!parsed.is_empty());

        let err = store
            .write_example()
            .expect_err("second write refuses overwrite");
        assert!(
            err.contains("exists") || err.contains("overwrite"),
            "got: {err}"
        );
        assert_eq!(std::fs::read_to_string(&written).unwrap(), text);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
