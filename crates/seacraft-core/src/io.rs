//! Crash-safe file output.
//!
//! [`write_atomic`] writes through a temporary sibling and a rename, so an
//! interrupted or failed write leaves either the old file or the new one on
//! disk, never half of either. Every user-facing file the library, the CLI,
//! the GUI and the control channels write goes through it.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

/// Distinguishes temporary files of concurrent writes within one process.
static COUNTER: AtomicU64 = AtomicU64::new(0);

/// Write `bytes` to `path` atomically: a temporary file in the same
/// directory (`.<name>.tmp-<pid>-<n>`) is written, flushed and synced, then
/// renamed over the target. On any error the temporary file is removed and
/// an existing target is left as it was.
///
/// An existing file's permissions are carried over (best effort); a new file
/// gets the usual defaults, as [`std::fs::write`] would give it. A symbolic
/// link at `path` is followed, so the file it points to is replaced and the
/// link stays.
pub fn write_atomic(path: impl AsRef<Path>, bytes: impl AsRef<[u8]>) -> io::Result<()> {
    let target = resolve(path.as_ref());
    let name = target.file_name().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            format!("{} is not a file path", target.display()),
        )
    })?;
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let tmp = dir.join(format!(
        ".{}.tmp-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));

    let result = write_then_rename(&tmp, &target, bytes.as_ref());
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    } else {
        sync_dir(&dir);
    }
    result
}

/// Follow a symbolic link at `path` to the file it names; anything else
/// (including a dangling link) is written at `path` itself.
fn resolve(path: &Path) -> PathBuf {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_symlink() => {
            fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
        }
        _ => path.to_path_buf(),
    }
}

fn write_then_rename(tmp: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(tmp)?;
    file.write_all(bytes)?;
    file.flush()?;
    if let Ok(meta) = fs::metadata(target) {
        if meta.is_file() {
            // Best effort: a file we may write but not chmod keeps defaults.
            let _ = file.set_permissions(meta.permissions());
        }
    }
    file.sync_all()?;
    drop(file);
    fs::rename(tmp, target)
}

/// Make the rename itself durable (best effort; a no-op off Unix).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("seacraft-io-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// Names in `dir` that look like leftover temporary files.
    fn temps(dir: &Path) -> Vec<String> {
        fs::read_dir(dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|n| n.contains(".tmp-"))
            .collect()
    }

    #[test]
    fn writes_a_new_file_and_replaces_an_existing_one() {
        let dir = scratch("replace");
        let path = dir.join("hull.fef");
        write_atomic(&path, "first").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "first");
        write_atomic(&path, b"second, longer").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "second, longer");
        write_atomic(&path, "").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "");
        assert!(temps(&dir).is_empty(), "{:?}", temps(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_failing_rename_keeps_the_original_and_cleans_up() {
        let dir = scratch("fail");
        // The target is a directory: the temporary file is written, but the
        // rename over a directory fails.
        let target = dir.join("model.fef");
        fs::create_dir(&target).unwrap();
        fs::write(target.join("inside.txt"), "kept").unwrap();
        let err = write_atomic(&target, "new").unwrap_err();
        assert!(!err.to_string().is_empty());
        assert!(target.is_dir(), "the original survives");
        assert_eq!(
            fs::read_to_string(target.join("inside.txt")).unwrap(),
            "kept"
        );
        assert!(temps(&dir).is_empty(), "{:?}", temps(&dir));

        // A missing directory fails before anything is written.
        let missing = dir.join("no-such-dir").join("x.fef");
        assert!(write_atomic(&missing, "x").is_err());
        assert!(temps(&dir).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn permissions_and_symlinks_are_kept() {
        use std::os::unix::fs::{symlink, PermissionsExt};
        let dir = scratch("perm");
        let path = dir.join("report.html");
        fs::write(&path, "old").unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o640)).unwrap();
        write_atomic(&path, "new").unwrap();
        assert_eq!(fs::read_to_string(&path).unwrap(), "new");
        assert_eq!(
            fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o640
        );

        let link = dir.join("link.html");
        symlink(&path, &link).unwrap();
        write_atomic(&link, "through the link").unwrap();
        assert!(fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink());
        assert_eq!(fs::read_to_string(&path).unwrap(), "through the link");
        assert!(temps(&dir).is_empty(), "{:?}", temps(&dir));
        let _ = fs::remove_dir_all(&dir);
    }
}
