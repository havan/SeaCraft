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
    // Best-effort non-unix entropy fallback using standard library only.
    // Combines high-resolution timestamps, process ID, thread ID, and memory addresses
    // mixed with a splitmix64 permutation.
    use std::time::{SystemTime, UNIX_EPOCH};
    let mut buf = [0u8; 32];
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let pid = std::process::id() as u128;
    let stack_addr = &buf as *const _ as usize as u128;
    let mut state = now ^ (pid << 32) ^ (stack_addr << 64);
    for chunk in buf.chunks_exact_mut(8) {
        state = state.wrapping_add(0x9E3779B97F4A7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58476D1CE4E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D049BB133111EB);
        z ^= z >> 31;
        chunk.copy_from_slice(&(z as u64).to_le_bytes());
    }
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
/// Parent directories and symlinks are resolved via canonicalization.
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
}
