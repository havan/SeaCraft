//! Crash-safe file output.
//!
//! [`write_atomic`] writes through a temporary sibling and a rename, so an
//! interrupted or failed write leaves either the old file or the new one on
//! disk, never half of either. Every user-facing file the library, the CLI,
//! the GUI and the control channels write goes through it, or through
//! [`write_all_atomic`] when one command writes several files: that stages them
//! all before replacing any, so a bad path among them leaves every file as it was.

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
    stage(path.as_ref(), bytes.as_ref())?.commit()
}

/// Write several files as one: each is staged in its temporary sibling (written, flushed, synced)
/// as [`write_atomic`] does, and only when every one is staged are they renamed over their targets,
/// in order. So a path that cannot be written (a missing folder, a file where a folder should be, a
/// directory where the file should be, no file name) fails before any target is touched, and the
/// error names it. A rename can still fail after earlier ones, but staging has shown every folder
/// writable, so that takes a change on disk between the two steps.
pub fn write_all_atomic<P: AsRef<Path>, B: AsRef<[u8]>>(files: &[(P, B)]) -> io::Result<()> {
    let mut staged = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        let path = path.as_ref();
        match stage(path, bytes.as_ref()) {
            Ok(s) => staged.push(s),
            Err(e) => {
                for s in &staged {
                    let _ = fs::remove_file(&s.tmp);
                }
                return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display())));
            }
        }
    }
    let mut staged = staged.into_iter();
    while let Some(s) = staged.next() {
        if let Err(e) = s.commit() {
            for rest in staged {
                let _ = fs::remove_file(&rest.tmp);
            }
            return Err(e);
        }
    }
    Ok(())
}

/// A file written in full to its temporary sibling, waiting to be renamed over its target.
struct Staged {
    tmp: PathBuf,
    target: PathBuf,
    dir: PathBuf,
}

impl Staged {
    /// Rename over the target; on failure the temporary file is removed.
    fn commit(self) -> io::Result<()> {
        match fs::rename(&self.tmp, &self.target) {
            Ok(()) => {
                sync_dir(&self.dir);
                Ok(())
            }
            Err(e) => {
                let _ = fs::remove_file(&self.tmp);
                Err(e)
            }
        }
    }
}

/// Write `bytes` to a temporary sibling of `path`'s target; on failure nothing is left behind.
fn stage(path: &Path, bytes: &[u8]) -> io::Result<Staged> {
    let target = resolve(path);
    let name = target
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?;
    if fs::metadata(&target).is_ok_and(|m| m.is_dir()) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a folder, not a file",
        ));
    }
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
    match write_temp(&tmp, &target, bytes) {
        Ok(()) => Ok(Staged { tmp, target, dir }),
        Err(e) => {
            let _ = fs::remove_file(&tmp);
            Err(e)
        }
    }
}

/// Where a write to `path` lands, to tell whether two outputs name one file. Symbolic links are
/// followed, a dangling one too: once another output creates its target, [`write_atomic`] writes
/// through it. The directory is made canonical; the file need not exist. A loop of links has no
/// landing and is an error: the first write would replace one of its links and the next land there.
/// So is a directory that is missing or cannot be resolved.
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
    let (Some(dir), Some(name)) = (p.parent(), p.file_name()) else {
        return Ok(p);
    };
    // A folder that is missing or cannot be resolved (a loop of links, no permission) would fail
    // only its own write, after the outputs before it were written: refuse it before any.
    match fs::canonicalize(dir) {
        Ok(dir) => Ok(dir.join(name)),
        Err(e) => Err(io::Error::new(
            e.kind(),
            format!("{}: cannot resolve its folder: {e}", path.display()),
        )),
    }
}

/// The first pair of `outputs`, as `(earlier, later)` indices, that would land in one file (see
/// [`landing`]): written in turn, the later would replace the earlier while both are reported.
/// Landings are compared without regard to case, on every platform: on a case-blind file system
/// (Windows, macOS by default) `p.png` and `P.PNG` are one file, and telling which a folder is
/// costs more than refusing two names a user would hardly mean as two files. A path that is not
/// UTF-8 is compared as it is: it cannot equal a UTF-8 one, and folding it would merge names.
pub fn shared_output(outputs: &[&Path]) -> io::Result<Option<(usize, usize)>> {
    let landed = outputs
        .iter()
        .map(|p| {
            let land = landing(p)?;
            Ok(match land.to_str() {
                Some(text) => PathBuf::from(text.to_lowercase()),
                None => land,
            })
        })
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

fn write_temp(tmp: &Path, target: &Path, bytes: &[u8]) -> io::Result<()> {
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
    file.sync_all()
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
        // The target is a directory: refused before a temporary file is
        // written.
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
        assert_eq!(
            shared_output(&[&b, &a, &dir.join("A.PNG")]).unwrap(),
            Some((1, 2))
        );
        // A folder no file system can find under any case or spelling (not `SUB/..`, which a
        // case-blind disk resolves to `sub` and Windows drops before looking).
        assert!(
            shared_output(&[&a, &dir.join("missing/a.png")]).is_err(),
            "a missing folder has no landing"
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

    #[test]
    fn write_all_atomic_writes_every_file() {
        let dir = scratch("all");
        fs::write(dir.join("b.txt"), "old").unwrap();
        let files = [(dir.join("a.txt"), "one"), (dir.join("b.txt"), "two")];
        write_all_atomic(&files).unwrap();
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "one");
        assert_eq!(fs::read_to_string(dir.join("b.txt")).unwrap(), "two");
        assert!(temps(&dir).is_empty(), "{:?}", temps(&dir));
        let _ = fs::remove_dir_all(&dir);
    }

    /// One unwritable file among several leaves every target as it was, and no temporary file.
    #[test]
    fn write_all_atomic_touches_nothing_when_one_file_cannot_be_written() {
        let dir = scratch("all-fail");
        fs::write(dir.join("kept.txt"), "old").unwrap();
        fs::create_dir(dir.join("folder.txt")).unwrap();
        fs::write(dir.join("blocker"), "a file").unwrap();
        for bad in ["missing/x.txt", "folder.txt", "blocker/x.txt", ".."] {
            let files = [
                (dir.join("kept.txt"), "new"),
                (dir.join("fresh.txt"), "new"),
                (dir.join(bad), "new"),
            ];
            let Err(err) = write_all_atomic(&files) else {
                panic!("{bad} was written");
            };
            assert!(err.to_string().contains(bad), "{bad}: {err}");
            assert_eq!(
                fs::read_to_string(dir.join("kept.txt")).unwrap(),
                "old",
                "{bad}"
            );
            assert!(!dir.join("fresh.txt").exists(), "{bad}");
            assert!(temps(&dir).is_empty(), "{bad}: {:?}", temps(&dir));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Names that are not UTF-8 are compared byte for byte: two such names are not one file because
    /// both would print with the same replacement character.
    #[cfg(unix)]
    #[test]
    fn names_that_are_not_utf8_stay_distinct() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        let dir = scratch("bytes");
        let a = dir.join(OsStr::from_bytes(b"a\x80.png"));
        let b = dir.join(OsStr::from_bytes(b"a\x81.png"));
        assert_eq!(shared_output(&[&a, &b]).unwrap(), None);
        assert_eq!(shared_output(&[&a, &b, &a]).unwrap(), Some((0, 2)));
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
