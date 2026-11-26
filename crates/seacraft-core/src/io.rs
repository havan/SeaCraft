//! Crash-safe file output.
//!
//! [`write_atomic`] writes through a temporary sibling and a rename, so an
//! interrupted or failed write leaves either the old file or the new one on
//! disk, never half of either. Every user-facing file the library, the CLI,
//! the GUI and the control channels write goes through it, or through
//! [`write_all_atomic`] when one command writes several files: that stages them
//! all before replacing any, so a bad path among them leaves every file as it was.
//!
//! Under write roots ([`within_write_roots`], which the GUI's control channel sets for each
//! request), a file must lie under a root; it is written in a folder opened from that root one
//! folder at a time, never through a symbolic link, and its temporary file and rename are made
//! in that open folder. A folder swapped for a link after the path was checked
//! ([`crate::control_auth::check_write_path`]) then fails the write instead of leading it out of
//! the root.

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;

/// Distinguishes temporary files of concurrent writes within one process.
static COUNTER: AtomicU64 = AtomicU64::new(0);
/// Serializes multi-file atomic writes in this process so concurrent exports to overlapping paths
/// cannot interleave their renames.
static WRITE_ALL_LOCK: Mutex<()> = Mutex::new(());

thread_local! {
    /// The write roots this thread's writes are held to ([`within_write_roots`]); empty for none.
    static WRITE_ROOTS: RefCell<Vec<PathBuf>> = const { RefCell::new(Vec::new()) };
}

/// Run `f` with this thread's writes held to `roots`. A file must lie under one of them, spelled
/// as it resolves (as [`crate::control_auth::check_write_path`] returns it: canonical); it is
/// written in its folder opened from that root without following a symbolic link on the way, so
/// it lands under the root or the write fails. A link at the file itself is followed only to a
/// file under a write root. Any other write fails: a file the app writes for itself, such as its
/// settings, is written inside `within_write_roots(&[], ..)`, which holds nothing. The roots
/// before come back when `f` returns or panics.
pub fn within_write_roots<T>(roots: &[PathBuf], f: impl FnOnce() -> T) -> T {
    struct Restore(Vec<PathBuf>);
    impl Drop for Restore {
        fn drop(&mut self) {
            let before = std::mem::take(&mut self.0);
            WRITE_ROOTS.with(|r| *r.borrow_mut() = before);
        }
    }
    let before = WRITE_ROOTS.with(|r| std::mem::replace(&mut *r.borrow_mut(), roots.to_vec()));
    let _restore = Restore(before);
    f()
}

/// The roots this thread's writes are held to ([`within_write_roots`]), so a worker thread can be
/// held to the same ones.
pub fn write_roots() -> Vec<PathBuf> {
    WRITE_ROOTS.with(|r| r.borrow().clone())
}

/// Write `bytes` to `path` atomically: a temporary file in the same
/// directory (`.<name>.tmp-<pid>-<n>`) is written, flushed and synced, then
/// renamed over the target. On any error the temporary file is removed and
/// an existing target is left as it was.
///
/// An existing file's permissions are carried over (best effort); a new file
/// gets the usual defaults, as [`std::fs::write`] would give it. A symbolic
/// link at `path` is followed, so the file it points to is replaced and the
/// link stays. Under write roots, see [`within_write_roots`].
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
    let _guard = WRITE_ALL_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let mut staged = Vec::with_capacity(files.len());
    for (path, bytes) in files {
        let path = path.as_ref();
        match stage(path, bytes.as_ref()) {
            Ok(s) => staged.push(s),
            Err(e) => {
                for s in &staged {
                    s.discard();
                }
                return Err(io::Error::new(e.kind(), format!("{}: {e}", path.display())));
            }
        }
    }
    let mut committed: Vec<PathBuf> = Vec::with_capacity(staged.len());
    let mut staged = staged.into_iter();
    while let Some(s) = staged.next() {
        let target = s.target.clone();
        if let Err(e) = s.commit() {
            for rest in staged {
                rest.discard();
            }
            let msg = if committed.is_empty() {
                format!("{}: {e}", target.display())
            } else {
                let done: Vec<String> = committed.iter().map(|p| p.display().to_string()).collect();
                format!(
                    "{}: {e} (partial export: already wrote {}; regenerate all outputs)",
                    target.display(),
                    done.join(", ")
                )
            };
            return Err(io::Error::new(e.kind(), msg));
        }
        committed.push(target);
    }
    Ok(())
}

/// A file written in full to its temporary sibling, waiting to be renamed over its target.
struct Staged {
    /// The temporary file's name in `dir`.
    tmp: OsString,
    /// The target's file name in `dir`.
    name: OsString,
    target: PathBuf,
    dir: PathBuf,
    /// `dir` opened from its write root ([`within_write_roots`]); `None` uses `dir` by name.
    held: Option<held::Folder>,
}

impl Staged {
    /// Write `bytes` to the temporary file, carrying the target's permissions over.
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        let mut file = match &self.held {
            Some(folder) => folder.create_new(&self.tmp)?,
            None => fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.dir.join(&self.tmp))?,
        };
        file.write_all(bytes)?;
        file.flush()?;
        // Best effort: a file we may write but not chmod keeps defaults.
        match &self.held {
            Some(folder) => folder.keep_permissions(&self.name, &file),
            None => {
                if let Ok(meta) = fs::metadata(&self.target) {
                    if meta.is_file() {
                        let _ = file.set_permissions(meta.permissions());
                    }
                }
            }
        }
        file.sync_all()
    }

    /// Remove the temporary file (best effort).
    fn discard(&self) {
        let _ = match &self.held {
            Some(folder) => folder.remove(&self.tmp),
            None => fs::remove_file(self.dir.join(&self.tmp)),
        };
    }

    /// Rename over the target; on failure the temporary file is removed.
    fn commit(self) -> io::Result<()> {
        let renamed = match &self.held {
            Some(folder) => folder.rename(&self.tmp, &self.name),
            None => fs::rename(self.dir.join(&self.tmp), &self.target),
        };
        match renamed {
            Ok(()) => {
                match &self.held {
                    Some(folder) => folder.sync(),
                    None => sync_dir(&self.dir),
                }
                Ok(())
            }
            Err(e) => {
                self.discard();
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
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "not a file path"))?
        .to_os_string();
    let held = held_folder(path, &target)?;
    #[cfg(test)]
    if held.is_some() {
        tests::after_open();
    }
    let is_dir = match &held {
        Some(folder) => folder.is_dir(&name),
        None => fs::metadata(&target).is_ok_and(|m| m.is_dir()),
    };
    if is_dir {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a folder, not a file",
        ));
    }
    let dir = match target.parent() {
        Some(d) if !d.as_os_str().is_empty() => d.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let tmp = OsString::from(format!(
        ".{}.tmp-{}-{}",
        name.to_string_lossy(),
        std::process::id(),
        COUNTER.fetch_add(1, Ordering::Relaxed)
    ));
    let staged = Staged {
        tmp,
        name,
        target,
        dir,
        held,
    };
    match staged.write(bytes) {
        Ok(()) => Ok(staged),
        Err(e) => {
            staged.discard();
            Err(e)
        }
    }
}

/// The folder `target` is written in, opened from its write root, when this thread's writes are
/// held to write roots ([`within_write_roots`]); `None` when they are not, so the write goes by
/// name as it always has. `target` is `path` with a link at the file followed ([`resolve`]).
///
/// Both must lie under a root as it resolves now, spelled as it resolves (as
/// [`crate::control_auth::check_write_path`] returns them), or the write is refused: a root
/// swapped for a link since the check, or a path spelled through a link, cannot be held to it.
fn held_folder(path: &Path, target: &Path) -> io::Result<Option<held::Folder>> {
    let roots = write_roots();
    if roots.is_empty() {
        return Ok(None);
    }
    // A root that no longer resolves holds nothing; a path under it alone is refused below.
    let roots: Vec<PathBuf> = roots.iter().filter_map(|r| r.canonicalize().ok()).collect();
    // The deepest root that holds `p`, and the rest of `p` below it.
    let under = |p: &Path| -> io::Result<Option<(&Path, PathBuf)>> {
        let p = plain(&std::path::absolute(p)?);
        Ok(roots
            .iter()
            .filter_map(|root| {
                p.strip_prefix(plain(root))
                    .ok()
                    .map(|rest| (root.as_path(), rest.to_path_buf()))
            })
            .max_by_key(|(root, _)| root.components().count()))
    };
    if under(path)?.is_none() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "not under a write root as it resolves now",
        ));
    }
    let Some((root, rest)) = under(target)? else {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "a link that leads out of the write roots",
        ));
    };
    // Below the root, only names of folders and the file: no `..`, nothing absolute.
    let mut parts = Vec::new();
    for part in rest.components() {
        match part {
            Component::Normal(name) => parts.push(name),
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("leaves the write root {}", root.display()),
                ))
            }
        }
    }
    if parts.pop().is_none() {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a folder, not a file",
        ));
    }
    held::Folder::open(root, &parts).map(Some)
}

/// `p` without a Windows verbatim prefix (`\\?\C:\` becomes `C:\`, `\\?\UNC\s\x` becomes `\\s\x`)
/// and with its drive letter in capitals, so a canonical root and a path spelled the usual way
/// compare alike. Elsewhere, `p` as it is.
fn plain(p: &Path) -> PathBuf {
    #[cfg(windows)]
    {
        use std::path::Prefix;
        let mut parts = p.components();
        if let Some(Component::Prefix(prefix)) = parts.next() {
            let head = match prefix.kind() {
                Prefix::VerbatimDisk(d) | Prefix::Disk(d) => {
                    Some(format!("{}:", char::from(d).to_ascii_uppercase()))
                }
                Prefix::VerbatimUNC(server, share) => Some(format!(
                    r"\\{}\{}",
                    server.to_string_lossy(),
                    share.to_string_lossy()
                )),
                _ => None,
            };
            if let Some(head) = head {
                let mut out = PathBuf::from(head);
                for part in parts {
                    out.push(part.as_os_str());
                }
                return out;
            }
        }
    }
    p.to_path_buf()
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
                // i18n-casing: not translated (file paths)
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

/// Make the rename itself durable (best effort; a no-op off Unix).
fn sync_dir(dir: &Path) {
    #[cfg(unix)]
    if let Ok(d) = fs::File::open(dir) {
        let _ = d.sync_all();
    }
    #[cfg(not(unix))]
    let _ = dir;
}

/// The error for a folder below a write root that could not be opened without following a link.
fn refused(root: &Path, folder: &OsStr, e: io::Error) -> io::Error {
    io::Error::new(
        e.kind(),
        format!(
            "cannot open the folder `{}` under the write root {} without following a link: {e}",
            folder.to_string_lossy(),
            root.display()
        ),
    )
}

/// A folder under a write root, opened from the root so a write in it cannot leave the root.
#[cfg(unix)]
mod held {
    use super::refused;
    use rustix::fs::{AtFlags, FileType, Mode, OFlags};
    use std::ffi::OsStr;
    use std::fs::File;
    use std::io;
    use std::os::fd::OwnedFd;
    use std::path::Path;

    /// An open folder; the temporary file, the rename and the removal are made relative to it.
    pub(super) struct Folder(OwnedFd);

    impl Folder {
        /// Open `root`, then each of `folders` in the one before, refusing a symbolic link.
        pub(super) fn open(root: &Path, folders: &[&OsStr]) -> io::Result<Folder> {
            let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
            // The root is canonical here: a link at its last name was put there since.
            let mut fd = rustix::fs::open(root, flags | OFlags::NOFOLLOW, Mode::empty())
                .map_err(|e| refused(root, root.as_os_str(), e.into()))?;
            for name in folders {
                fd = rustix::fs::openat(&fd, *name, flags | OFlags::NOFOLLOW, Mode::empty())
                    .map_err(|e| refused(root, name, e.into()))?;
            }
            Ok(Folder(fd))
        }

        fn stat(&self, name: &OsStr) -> Option<rustix::fs::Stat> {
            rustix::fs::statat(&self.0, name, AtFlags::SYMLINK_NOFOLLOW).ok()
        }

        pub(super) fn is_dir(&self, name: &OsStr) -> bool {
            self.stat(name)
                .is_some_and(|s| FileType::from_raw_mode(s.st_mode).is_dir())
        }

        /// Give `file` the permissions of the file `name`, if that is a plain file.
        pub(super) fn keep_permissions(&self, name: &OsStr, file: &File) {
            if let Some(s) = self.stat(name) {
                if FileType::from_raw_mode(s.st_mode).is_file() {
                    let _ = rustix::fs::fchmod(file, Mode::from_raw_mode(s.st_mode));
                }
            }
        }

        pub(super) fn create_new(&self, name: &OsStr) -> io::Result<File> {
            let flags =
                OFlags::WRONLY | OFlags::CREATE | OFlags::EXCL | OFlags::NOFOLLOW | OFlags::CLOEXEC;
            let fd = rustix::fs::openat(&self.0, name, flags, Mode::from_raw_mode(0o666))?;
            Ok(File::from(fd))
        }

        pub(super) fn rename(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            Ok(rustix::fs::renameat(&self.0, from, &self.0, to)?)
        }

        pub(super) fn remove(&self, name: &OsStr) -> io::Result<()> {
            Ok(rustix::fs::unlinkat(&self.0, name, AtFlags::empty())?)
        }

        /// Make the rename durable (best effort).
        pub(super) fn sync(&self) {
            let _ = rustix::fs::fsync(&self.0);
        }
    }
}

/// A folder under a write root, opened from the root so a write in it cannot leave the root.
///
/// Windows has no portable call to create a file relative to an open folder, so every folder from
/// the root down is held open without sharing deletion: while the write runs none of them can be
/// renamed, removed or replaced by a link, and the file is then written by its path.
#[cfg(not(unix))]
mod held {
    use std::ffi::OsStr;
    use std::fs::{self, File};
    use std::io;
    use std::path::{Path, PathBuf};

    pub(super) struct Folder {
        path: PathBuf,
        _held: Vec<File>,
    }

    impl Folder {
        /// Open `root`, then each of `folders` below it, refusing a symbolic link or junction.
        #[cfg(windows)]
        pub(super) fn open(root: &Path, folders: &[&OsStr]) -> io::Result<Folder> {
            use std::os::windows::fs::{MetadataExt, OpenOptionsExt};
            const FILE_SHARE_READ: u32 = 0x1;
            const FILE_SHARE_WRITE: u32 = 0x2;
            const FILE_FLAG_BACKUP_SEMANTICS: u32 = 0x0200_0000;
            const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
            const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
            let hold = |path: &Path, flags: u32| {
                fs::OpenOptions::new()
                    .read(true)
                    .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
                    .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | flags)
                    .open(path)
            };
            // The root is canonical here, so it is checked like the folders below it.
            let mut path = root.to_path_buf();
            let mut held = Vec::with_capacity(folders.len() + 1);
            for name in std::iter::once(root.as_os_str()).chain(folders.iter().copied()) {
                if !held.is_empty() {
                    path.push(name);
                }
                let folder = hold(&path, FILE_FLAG_OPEN_REPARSE_POINT)
                    .map_err(|e| super::refused(root, name, e))?;
                let meta = folder.metadata()?;
                if !meta.is_dir() || meta.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
                    return Err(super::refused(
                        root,
                        name,
                        io::Error::new(io::ErrorKind::InvalidInput, "a link, or not a folder"),
                    ));
                }
                held.push(folder);
            }
            Ok(Folder { path, _held: held })
        }

        #[cfg(not(windows))]
        pub(super) fn open(_root: &Path, _folders: &[&OsStr]) -> io::Result<Folder> {
            Err(io::Error::new(
                io::ErrorKind::Unsupported,
                "writes under write roots need Unix or Windows",
            ))
        }

        pub(super) fn is_dir(&self, name: &OsStr) -> bool {
            fs::symlink_metadata(self.path.join(name)).is_ok_and(|m| m.is_dir())
        }

        /// Give `file` the permissions of the file `name`, if that is a plain file.
        pub(super) fn keep_permissions(&self, name: &OsStr, file: &File) {
            if let Ok(meta) = fs::symlink_metadata(self.path.join(name)) {
                if meta.is_file() {
                    let _ = file.set_permissions(meta.permissions());
                }
            }
        }

        pub(super) fn create_new(&self, name: &OsStr) -> io::Result<File> {
            fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(self.path.join(name))
        }

        pub(super) fn rename(&self, from: &OsStr, to: &OsStr) -> io::Result<()> {
            fs::rename(self.path.join(from), self.path.join(to))
        }

        pub(super) fn remove(&self, name: &OsStr) -> io::Result<()> {
            fs::remove_file(self.path.join(name))
        }

        pub(super) fn sync(&self) {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    thread_local! {
        /// Runs once, on this thread's next write under a write root, just after its folder is
        /// opened: where a folder swapped by another process would meet the write.
        static AFTER_OPEN: RefCell<Option<Box<dyn FnOnce()>>> = const { RefCell::new(None) };
    }

    /// The hook [`stage`] calls after opening a folder under a write root.
    pub(super) fn after_open() {
        if let Some(hook) = AFTER_OPEN.with(|h| h.borrow_mut().take()) {
            hook();
        }
    }

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

    /// One unwritable file among several leaves every target as it was, and no temporary file:
    /// written as usual, and under a write root.
    #[test]
    fn write_all_atomic_touches_nothing_when_one_file_cannot_be_written() {
        // Canonical, so the paths lie under the write root as a checked path would.
        let dir = fs::canonicalize(scratch("all-fail")).unwrap();
        fs::write(dir.join("kept.txt"), "old").unwrap();
        fs::create_dir(dir.join("folder.txt")).unwrap();
        fs::write(dir.join("blocker"), "a file").unwrap();
        let bad_paths: [&[&str]; 4] = [
            &["missing", "x.txt"],
            &["folder.txt"],
            &["blocker", "x.txt"],
            &[".."],
        ];
        for roots in [vec![], vec![dir.clone()]] {
            for parts in bad_paths {
                let bad = parts.iter().fold(dir.clone(), |p, part| p.join(part));
                let files = [
                    (dir.join("kept.txt"), "new"),
                    (dir.join("fresh.txt"), "new"),
                    (bad.clone(), "new"),
                ];
                let Err(err) = within_write_roots(&roots, || write_all_atomic(&files)) else {
                    panic!("{} was written", bad.display());
                };
                let what = format!("{} under {roots:?}", bad.display());
                assert!(
                    err.to_string().contains(&bad.display().to_string()),
                    "{what}: {err}"
                );
                assert_eq!(
                    fs::read_to_string(dir.join("kept.txt")).unwrap(),
                    "old",
                    "{what}"
                );
                assert!(!dir.join("fresh.txt").exists(), "{what}");
                assert!(temps(&dir).is_empty(), "{what}: {:?}", temps(&dir));
            }
        }
        let _ = fs::remove_dir_all(&dir);
    }

    /// Make `link` a symbolic link to the folder `target`; false where this account may not
    /// (Windows without the privilege), so a test has nothing to show there.
    fn link_folder(target: &Path, link: &Path) -> bool {
        #[cfg(unix)]
        return std::os::unix::fs::symlink(target, link).is_ok();
        #[cfg(windows)]
        return std::os::windows::fs::symlink_dir(target, link).is_ok();
        #[cfg(not(any(unix, windows)))]
        return false;
    }

    /// A write under a write root works as one elsewhere: new files, a replaced one keeping its
    /// content until the rename, several at once, and no temporary file left.
    #[test]
    fn writes_under_a_write_root_land_in_place() {
        let dir = fs::canonicalize(scratch("held")).unwrap();
        fs::create_dir(dir.join("sub")).unwrap();
        fs::write(dir.join("sub").join("b.txt"), "old").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = fs::Permissions::from_mode(0o640);
            fs::set_permissions(dir.join("sub").join("b.txt"), mode).unwrap();
        }
        let roots = [dir.clone()];
        within_write_roots(&roots, || {
            write_atomic(dir.join("a.txt"), "one").unwrap();
            let files = [
                (dir.join("sub").join("b.txt"), "two"),
                (dir.join("sub").join("c.txt"), "three"),
            ];
            write_all_atomic(&files).unwrap();
        });
        assert_eq!(fs::read_to_string(dir.join("a.txt")).unwrap(), "one");
        assert_eq!(fs::read_to_string(dir.join("sub/b.txt")).unwrap(), "two");
        assert_eq!(fs::read_to_string(dir.join("sub/c.txt")).unwrap(), "three");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let meta = fs::metadata(dir.join("sub/b.txt")).unwrap();
            assert_eq!(meta.permissions().mode() & 0o777, 0o640, "permissions kept");
        }
        assert!(temps(&dir).is_empty(), "{:?}", temps(&dir));
        assert!(temps(&dir.join("sub")).is_empty());
        let _ = fs::remove_dir_all(&dir);
    }

    /// Issue #18, inside the writer: a folder swapped for a link out of the root after the writer
    /// opened it. The write stays in the folder it opened, or fails; it never lands outside.
    #[test]
    fn a_folder_swapped_after_it_is_opened_keeps_the_write_inside_the_root() {
        let base = fs::canonicalize(scratch("swap-open")).unwrap();
        let (root, outside) = (base.join("root"), base.join("outside"));
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir(&outside).unwrap();
        let (sub, moved) = (root.join("sub"), root.join("sub-moved"));
        let swapped = std::rc::Rc::new(std::cell::Cell::new(false));
        let hook = {
            let (sub, moved, outside, swapped) =
                (sub.clone(), moved.clone(), outside.clone(), swapped.clone());
            // Windows refuses the rename while the writer holds the folder open.
            move || swapped.set(fs::rename(&sub, &moved).is_ok() && link_folder(&outside, &sub))
        };
        AFTER_OPEN.with(|h| *h.borrow_mut() = Some(Box::new(hook)));
        let written = within_write_roots(std::slice::from_ref(&root), || {
            write_atomic(sub.join("out.fef"), "hull")
        });
        let landed: Vec<_> = fs::read_dir(&outside).unwrap().collect();
        let in_moved = fs::read_to_string(moved.join("out.fef"));
        let _ = fs::remove_dir_all(&base);
        assert!(landed.is_empty(), "{} files outside the root", landed.len());
        #[cfg(unix)]
        assert!(swapped.get(), "the hook swapped the folder");
        if swapped.get() {
            #[cfg(unix)]
            {
                written.unwrap();
                assert_eq!(
                    in_moved.unwrap(),
                    "hull",
                    "the write stays in the folder it opened"
                );
            }
            #[cfg(not(unix))]
            let _ = (written, in_moved);
        }
    }

    /// Under write roots a link at the file is followed to a file under a root, as it is
    /// elsewhere, and refused when it leads out: the link is left as it was.
    #[cfg(unix)]
    #[test]
    fn under_write_roots_a_link_at_the_file_is_followed_only_inside_them() {
        use std::os::unix::fs::symlink;
        let base = fs::canonicalize(scratch("held-link")).unwrap();
        let (root, outside) = (base.join("root"), base.join("outside"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(&outside).unwrap();
        fs::write(root.join("real.fef"), "old").unwrap();
        fs::write(outside.join("far.fef"), "far").unwrap();
        symlink(root.join("real.fef"), root.join("near.fef")).unwrap();
        symlink(outside.join("far.fef"), root.join("away.fef")).unwrap();
        let roots = [root.clone()];
        let near = within_write_roots(&roots, || write_atomic(root.join("near.fef"), "new"));
        let away = within_write_roots(&roots, || write_atomic(root.join("away.fef"), "new"));
        let real = fs::read_to_string(root.join("real.fef")).unwrap();
        let far = fs::read_to_string(outside.join("far.fef")).unwrap();
        let away_is_link = fs::symlink_metadata(root.join("away.fef"))
            .unwrap()
            .file_type()
            .is_symlink();
        let _ = fs::remove_dir_all(&base);
        near.unwrap();
        assert_eq!(real, "new");
        assert!(away.is_err(), "wrote out of the root through a link");
        assert_eq!(far, "far");
        assert!(away_is_link);
    }

    /// Under write roots a file outside all of them is refused, unless the write is taken out of
    /// the roots (`within_write_roots(&[], ..)`, as the app does for its own settings). The roots
    /// before come back when the closure returns.
    #[test]
    fn write_roots_refuse_files_outside_them_and_end_with_the_closure() {
        let base = fs::canonicalize(scratch("held-scope")).unwrap();
        let (root, elsewhere) = (base.join("root"), base.join("elsewhere"));
        fs::create_dir_all(&root).unwrap();
        fs::create_dir(&elsewhere).unwrap();
        let roots = vec![root.clone()];
        let inner = vec![elsewhere.clone()];
        within_write_roots(&roots, || {
            assert_eq!(write_roots(), roots);
            let refused = write_atomic(elsewhere.join("settings.json"), "no");
            assert_eq!(refused.unwrap_err().kind(), io::ErrorKind::PermissionDenied);
            within_write_roots(&[], || write_atomic(elsewhere.join("settings.json"), "{}"))
                .unwrap();
            within_write_roots(&inner, || assert_eq!(write_roots(), inner));
            assert_eq!(write_roots(), roots);
        });
        assert!(write_roots().is_empty());
        let caught = std::panic::catch_unwind(|| within_write_roots(&roots, || panic!("in f")));
        assert!(caught.is_err());
        assert!(write_roots().is_empty(), "a panic ends the roots too");
        assert_eq!(
            fs::read_to_string(elsewhere.join("settings.json")).unwrap(),
            "{}"
        );
        let _ = fs::remove_dir_all(&base);
    }

    /// The write root itself swapped for a link out of it after the path was checked: the path no
    /// longer lies under the root as it now resolves, and the write fails rather than going by name.
    #[test]
    fn a_write_root_swapped_for_a_link_is_not_written_through() {
        let base = fs::canonicalize(scratch("swap-root")).unwrap();
        let (root, outside) = (base.join("root"), base.join("outside"));
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::create_dir_all(outside.join("sub")).unwrap();
        // As check_write_path returned it.
        let path = root.join("sub").join("out.fef");
        fs::rename(&root, base.join("root-before")).unwrap();
        if !link_folder(&outside, &root) {
            eprintln!("skipped: this account may not make a folder link");
            let _ = fs::remove_dir_all(&base);
            return;
        }
        let written =
            within_write_roots(std::slice::from_ref(&root), || write_atomic(&path, "hull"));
        let landed = outside.join("sub").join("out.fef").exists();
        let _ = fs::remove_dir_all(&base);
        assert!(written.is_err(), "wrote through the swapped root");
        assert!(!landed, "a file landed outside the root");
    }

    /// A root spelled through a link (as `/tmp` is on macOS) and a path spelled the same way: the
    /// path cannot be held to the root as it resolves, so the write fails rather than going by name,
    /// where a folder below it swapped for a link would lead it out.
    #[test]
    fn a_path_that_cannot_be_held_to_its_root_is_refused() {
        let base = fs::canonicalize(scratch("held-alias")).unwrap();
        let (real, outside) = (base.join("real"), base.join("outside"));
        fs::create_dir_all(real.join("root").join("sub")).unwrap();
        fs::create_dir(&outside).unwrap();
        let alias = base.join("alias");
        if !link_folder(&real, &alias) {
            eprintln!("skipped: this account may not make a folder link");
            let _ = fs::remove_dir_all(&base);
            return;
        }
        let sub = real.join("root").join("sub");
        fs::rename(&sub, real.join("root").join("sub-before")).unwrap();
        assert!(link_folder(&outside, &sub));
        let root = alias.join("root");
        let written = within_write_roots(std::slice::from_ref(&root), || {
            write_atomic(root.join("sub").join("out.fef"), "hull")
        });
        let landed: Vec<_> = fs::read_dir(&outside).unwrap().collect();
        let _ = fs::remove_dir_all(&base);
        assert!(written.is_err(), "wrote by name");
        assert!(landed.is_empty(), "{} files outside the root", landed.len());
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
