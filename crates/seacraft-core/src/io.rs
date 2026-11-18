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

/// Where a write to `path` lands, to tell whether two outputs name one file. Symbolic links are
/// followed, a dangling one too: once another output creates its target, [`write_atomic`] writes
/// through it. The directory is made canonical; the file need not exist. A loop of links has no
/// landing and is an error: the first write would replace one of its links and the next land there.
pub fn landing(path: &Path) -> io::Result<PathBuf> {
    let mut p = std::path::absolute(path)?;
    let mut hops = 0;
    while let Ok(target) = fs::read_link(&p) {
        hops += 1;
        if hops > 40 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("{}: too many symbolic links (a loop?)", path.display()),
            ));
        }
        // A relative target is read from the link's directory; `join` keeps an absolute one.
        p = match p.parent() {
            Some(dir) => dir.join(target),
            None => target,
        };
    }
    Ok(match (p.parent(), p.file_name()) {
        (Some(dir), Some(name)) => fs::canonicalize(dir).map(|d| d.join(name)).unwrap_or(p),
        _ => p,
    })
}

/// The first pair of `outputs`, as `(earlier, later)` indices, that would land in one file (see
/// [`landing`]): written in turn, the later would replace the earlier while both are reported.
/// Landings are compared without regard to case, on every platform: on a case-blind file system
/// (Windows, macOS by default) `p.png` and `P.PNG` are one file, and telling which a folder is
/// costs more than refusing two names a user would hardly mean as two files.
pub fn shared_output(outputs: &[&Path]) -> io::Result<Option<(usize, usize)>> {
    let landed = outputs
        .iter()
        .map(|p| Ok(landing(p)?.to_string_lossy().to_lowercase()))
        .collect::<io::Result<Vec<_>>>()?;
    Ok((0..landed.len()).find_map(|i| {
        landed[..i]
            .iter()
            .position(|q| *q == landed[i])
            .map(|j| (j, i))
    }))
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

    #[test]
    fn landing_names_where_a_write_goes() {
        let dir = scratch("landing");
        fs::create_dir(dir.join("sub")).unwrap();
        let here = fs::canonicalize(&dir).unwrap();
        let land = |name: &str| landing(&dir.join(name)).unwrap();
        assert_eq!(land("a.png"), here.join("a.png"));
        assert_eq!(land("sub/../a.png"), here.join("a.png"));
        assert_ne!(land("a.png"), land("a.svg"));
        fs::write(dir.join("b.png"), "x").unwrap();
        assert_eq!(land("b.png"), here.join("b.png"));
        let (a, b) = (dir.join("a.png"), dir.join("b.png"));
        let up = dir.join("sub/../a.png");
        assert_eq!(shared_output(&[&a, &b]).unwrap(), None);
        assert_eq!(shared_output(&[&a, &b, &up]).unwrap(), Some((0, 2)));
        let shout = dir.join("SUB/../A.PNG");
        assert_eq!(
            shared_output(&[&b, &a, &dir.join("A.PNG")]).unwrap(),
            Some((1, 2))
        );
        assert!(
            shared_output(&[&a, &shout]).is_ok(),
            "a missing folder is not an error"
        );
        let _ = fs::remove_dir_all(&dir);
    }

    /// A dangling link lands where it points: after its target is written, writes go through it.
    #[cfg(unix)]
    #[test]
    fn landing_follows_links_even_dangling() {
        use std::os::unix::fs::symlink;
        let dir = scratch("landing-link");
        fs::create_dir(dir.join("sub")).unwrap();
        let here = fs::canonicalize(&dir).unwrap();
        symlink("a.png", dir.join("dangling.svg")).unwrap();
        symlink("../a.png", dir.join("sub/up.svg")).unwrap();
        symlink(dir.join("dangling.svg"), dir.join("chain.svg")).unwrap();
        for link in ["dangling.svg", "sub/up.svg", "chain.svg"] {
            assert_eq!(
                landing(&dir.join(link)).unwrap(),
                here.join("a.png"),
                "{link}"
            );
        }
        // And the write goes there once the target exists.
        write_atomic(dir.join("a.png"), "png").unwrap();
        write_atomic(dir.join("dangling.svg"), "svg").unwrap();
        assert_eq!(fs::read_to_string(dir.join("a.png")).unwrap(), "svg");
        let _ = fs::remove_dir_all(&dir);
    }

    /// A loop of links has no landing: writing its first link replaces it, and the rest then lead
    /// to that file.
    #[cfg(unix)]
    #[test]
    fn a_loop_of_links_has_no_landing() {
        use std::os::unix::fs::symlink;
        let dir = scratch("landing-loop");
        for (link, target) in [("a.png", "b.svg"), ("b.svg", "c.vti"), ("c.vti", "a.png")] {
            symlink(target, dir.join(link)).unwrap();
        }
        let (a, b) = (dir.join("a.png"), dir.join("b.svg"));
        let err = landing(&a).unwrap_err();
        assert!(err.to_string().contains("a.png"), "{err}");
        assert!(shared_output(&[&a, &b]).is_err());
        let _ = fs::remove_dir_all(&dir);
    }
}
