//! Control channel authentication and path sandboxing.
//!
//! Adapted in part from Tangency Labs' `solidcraft/crates/engine/src/control_auth.rs`
//! (MIT OR Apache-2.0): constant-time token comparison, 0600 token file permissions,
//! and RAII cleanup. See ATTRIBUTION.md.

use std::io::{Error, ErrorKind, Result, Write};
use std::path::{Path, PathBuf};

/// Compares two tokens in constant time (`a == b`), taking the same time wherever they differ.
pub fn tokens_match(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    a.len() == b.len() && a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Generate 32 random bytes from the OS and return them as a 64-character hex string.
pub fn generate_token() -> Result<String> {
    let bytes = random_32_bytes()?;
    let mut s = String::with_capacity(64);
    for b in bytes {
        use std::fmt::Write;
        let _ = write!(s, "{:02x}", b);
    }
    Ok(s)
}

#[cfg(unix)]
fn random_32_bytes() -> Result<[u8; 32]> {
    use std::fs::File;
    use std::io::Read;
    let mut file = File::open("/dev/urandom")?;
    let mut buf = [0u8; 32];
    file.read_exact(&mut buf)?;
    Ok(buf)
}

#[cfg(not(unix))]
fn random_32_bytes() -> Result<[u8; 32]> {
    let mut buf = [0u8; 32];
    getrandom::fill(&mut buf).map_err(|e| std::io::Error::other(e.to_string()))?;
    Ok(buf)
}

/// The default token file path for `port`:
/// `$XDG_RUNTIME_DIR/seacraft-control-<port>.token` if `$XDG_RUNTIME_DIR` is set and non-empty,
/// otherwise `<system temp dir>/seacraft-control-<port>.token`.
pub fn default_token_path(port: u16) -> PathBuf {
    if let Some(runtime_dir) = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty()) {
        PathBuf::from(runtime_dir).join(format!("seacraft-control-{port}.token"))
    } else {
        std::env::temp_dir().join(format!("seacraft-control-{port}.token"))
    }
}

/// Write `token` to `path` with mode 0600 on Unix.
pub fn write_token_file(path: &Path, token: &str) -> Result<()> {
    let dir = path.parent().ok_or_else(|| {
        Error::new(
            ErrorKind::InvalidInput,
            "token file path has no parent folder",
        )
    })?;
    std::fs::create_dir_all(dir)?;
    let _ = std::fs::remove_file(path);
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path)?;
    f.write_all(token.as_bytes())?;
    f.write_all(b"\n")?;
    f.sync_all()
}

/// Read the token from `path`.
pub fn read_token_file(path: &Path) -> Result<String> {
    let s = std::fs::read_to_string(path)?;
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err(Error::new(
            ErrorKind::InvalidData,
            format!("token file `{}` is empty", path.display()),
        ));
    }
    Ok(trimmed.to_string())
}

/// RAII wrapper that deletes the token file on drop (clean shutdown).
#[derive(Debug)]
pub struct TokenFile(pub PathBuf);

impl Drop for TokenFile {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Allowed root directories for file read and write sandboxing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ControlRoots {
    pub read_roots: Vec<PathBuf>,
    pub write_roots: Vec<PathBuf>,
}

impl ControlRoots {
    pub fn new(read_roots: Vec<PathBuf>, write_roots: Vec<PathBuf>) -> Self {
        Self {
            read_roots,
            write_roots,
        }
    }
}

/// Verify that `path` resolves inside at least one of `roots`.
/// If `roots` is empty, any path is allowed.
pub fn check_read_path(path: &Path, roots: &[PathBuf]) -> std::result::Result<PathBuf, String> {
    if roots.is_empty() {
        return Ok(path.to_path_buf());
    }
    let canonical = path
        .canonicalize()
        .map_err(|e| format!("cannot resolve path `{}`: {e}", path.display()))?;
    for root in roots {
        let Ok(canonical_root) = root.canonicalize() else {
            continue;
        };
        if canonical.starts_with(&canonical_root) {
            return Ok(canonical);
        }
    }
    Err(format!(
        "read-root rule violation: path `{}` is outside allowed read roots ({})",
        path.display(),
        roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

/// Verify that an output `path` (which may not exist yet) resolves inside at least one of `roots`.
/// If `roots` is empty, any path is allowed.
///
/// Parent directories and symlinks are resolved via canonicalization. The check alone does not
/// hold the write to the root: a folder on the way can be swapped for a link before the write
/// runs. Write the returned path inside [`crate::io::within_write_roots`] with the same `roots`,
/// which opens its folders from the root without following links.
pub fn check_write_path(path: &Path, roots: &[PathBuf]) -> std::result::Result<PathBuf, String> {
    if roots.is_empty() {
        return Ok(path.to_path_buf());
    }
    let canonical = canonicalize_write_target(path).map_err(|e| {
        format!(
            "cannot resolve parent directory for write path `{}`: {e}",
            path.display()
        )
    })?;
    for root in roots {
        let Ok(canonical_root) = root.canonicalize() else {
            continue;
        };
        if canonical.starts_with(&canonical_root) {
            return Ok(canonical);
        }
    }
    Err(format!(
        "write-root rule violation: path `{}` is outside allowed write roots ({})",
        path.display(),
        roots
            .iter()
            .map(|r| r.display().to_string())
            .collect::<Vec<_>>()
            .join(", ")
    ))
}

fn canonicalize_write_target(path: &Path) -> Result<PathBuf> {
    if path.exists() {
        return path.canonicalize();
    }
    let mut parts = Vec::new();
    let mut curr = path;
    while !curr.exists() {
        if let Some(file_name) = curr.file_name() {
            parts.push(file_name);
            // A bare file name's parent is "", which is the working directory.
            curr = curr
                .parent()
                .filter(|p| !p.as_os_str().is_empty())
                .unwrap_or(Path::new("."));
        } else {
            break;
        }
    }
    let mut canonical = curr.canonicalize()?;
    for part in parts.into_iter().rev() {
        canonical.push(part);
    }
    Ok(canonical)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_bare_file_name_resolves_in_the_working_directory() {
        let name = format!("seacraft-no-such-file-{}.obj", std::process::id());
        let got = canonicalize_write_target(Path::new(&name)).expect("resolves");
        let cwd = std::env::current_dir().unwrap().canonicalize().unwrap();
        assert_eq!(got, cwd.join(&name));
    }

    #[test]
    fn constant_time_tokens_match_compares_correctly() {
        assert!(tokens_match("abc123", "abc123"));
        assert!(!tokens_match("abc123", "abc124"));
        assert!(!tokens_match("abc123", "abc12"));
        assert!(!tokens_match("", "a"));
        assert!(tokens_match("", ""));
        assert!(!tokens_match("x", ""));
        let t1 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let t2 = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdeg";
        assert!(tokens_match(t1, t1));
        assert!(!tokens_match(t1, t2));
    }

    #[cfg(unix)]
    #[test]
    fn token_file_permissions_are_0600_on_unix() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("seacraft-test-auth-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let path = dir.join("control.token");
        let token = generate_token().expect("generate token");
        assert_eq!(token.len(), 64);
        write_token_file(&path, &token).expect("write token file");

        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600, "token file mode must be 0600");

        let read_back = read_token_file(&path).expect("read back");
        assert_eq!(read_back, token);

        drop(TokenFile(path.clone()));
        assert!(!path.exists(), "token file was cleaned up on drop");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn read_root_checks_and_write_root_checks() {
        let dir = std::env::temp_dir().join(format!("seacraft-test-roots-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        let allowed_file = dir.join("model.fef");
        std::fs::write(&allowed_file, "model data").unwrap();

        let roots = vec![dir.clone()];

        // Allowed read
        assert!(check_read_path(&allowed_file, &roots).is_ok());

        // Read outside root (/etc/passwd or /etc/hosts exists on unix)
        #[cfg(unix)]
        {
            let outside = Path::new("/etc/hosts");
            if outside.exists() {
                let err = check_read_path(outside, &roots).unwrap_err();
                assert!(err.contains("read-root rule violation"));
            }
        }

        // Allowed write to non-existent file in allowed root
        let new_file = dir.join("out.fef");
        assert!(check_write_path(&new_file, &roots).is_ok());

        // Write outside root via .. traversal
        let traversal = dir.join("../evil.fef");
        let err = check_write_path(&traversal, &roots).unwrap_err();
        assert!(err.contains("write-root rule violation"));

        let _ = std::fs::remove_dir_all(&dir);
    }

    use crate::io::tests::link_folder;

    /// Issue #18: a folder under the write root is swapped for a link out of it after the path
    /// was checked and before the write. The write must fail, and nothing may land outside.
    #[test]
    fn a_folder_swapped_for_a_link_after_the_check_is_not_written_through() {
        let base = std::env::temp_dir().join(format!("seacraft-test-swap-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (root, outside) = (base.join("root"), base.join("outside"));
        std::fs::create_dir_all(root.join("sub")).unwrap();
        std::fs::create_dir_all(&outside).unwrap();
        let roots = vec![root.clone()];

        let validated = check_write_path(&root.join("sub").join("out.fef"), &roots).unwrap();
        // Between the check and the write: the folder becomes a link out of the root.
        std::fs::rename(root.join("sub"), root.join("sub-before")).unwrap();
        if !link_folder(&outside, &root.join("sub")) {
            eprintln!("skipped: this account may make neither a folder link nor a junction");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let one =
            crate::io::within_write_roots(&roots, || crate::io::write_atomic(&validated, "hull"));
        let all = crate::io::within_write_roots(&roots, || {
            crate::io::write_all_atomic(&[(&validated, "hull")])
        });

        let landed: Vec<_> = std::fs::read_dir(&outside).unwrap().collect();
        let _ = std::fs::remove_dir_all(&base);
        assert!(one.is_err(), "write_atomic followed the link");
        assert!(all.is_err(), "write_all_atomic followed the link");
        assert!(landed.is_empty(), "{} files outside the root", landed.len());
    }

    /// Without write roots, writes follow folder links as they always have.
    #[test]
    fn without_write_roots_a_linked_folder_is_written_through() {
        let base =
            std::env::temp_dir().join(format!("seacraft-test-nolimit-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let (dir, elsewhere) = (base.join("dir"), base.join("elsewhere"));
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::create_dir_all(&elsewhere).unwrap();
        if !link_folder(&elsewhere, &dir.join("sub")) {
            eprintln!("skipped: this account may make neither a folder link nor a junction");
            let _ = std::fs::remove_dir_all(&base);
            return;
        }
        let path = check_write_path(&dir.join("sub").join("out.fef"), &[]).unwrap();
        let written = crate::io::within_write_roots(&[], || crate::io::write_atomic(&path, "x"));
        let landed = std::fs::read_to_string(elsewhere.join("out.fef"));
        let _ = std::fs::remove_dir_all(&base);
        written.unwrap();
        assert_eq!(landed.unwrap(), "x");
    }
}
