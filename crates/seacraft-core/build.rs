//! Stamps the build with what git says about the source it came from, so every binary can tell a
//! release from a development build. The naming rule and its table are in `src/build_version.rs`.
//!
//! A packager building from an archive, where there is no git, sets `SEACRAFT_BUILD_VERSION` to the
//! version the build should report, such as `0.3.0`; it must start with the package version.

#[path = "src/build_version.rs"]
mod build_version;

use build_version::{build_version, Git};
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let package = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let workspace = manifest.join("../..");
    println!("cargo:rerun-if-env-changed=SEACRAFT_BUILD_VERSION");

    let (version, commit) = match std::env::var("SEACRAFT_BUILD_VERSION") {
        Ok(v) if !v.trim().is_empty() => {
            let v = v.trim().to_owned();
            assert!(
                v.starts_with(&package),
                "SEACRAFT_BUILD_VERSION '{v}' does not start with the package version {package}"
            );
            (v, None)
        }
        _ if is_checkout(&workspace) => {
            watch(&workspace);
            let facts = facts(&workspace, &package);
            (
                build_version(&package, facts.as_ref()),
                git(&workspace, &["rev-parse", "HEAD"]),
            )
        }
        _ => (build_version(&package, None), None),
    };
    println!("cargo:rustc-env=SEACRAFT_VERSION={version}");
    println!(
        "cargo:rustc-env=SEACRAFT_GIT_COMMIT={}",
        commit.unwrap_or_default()
    );
}

/// What git says about the checkout, or `None` when git cannot answer.
fn facts(workspace: &Path, package: &str) -> Option<Git> {
    let tag = format!("v{package}");
    // `--exclude=*` leaves only the abbreviated hash, with `-dirty` when tracked files changed.
    let described = git(
        workspace,
        &["describe", "--always", "--dirty", "--exclude=*"],
    )?;
    let (hash, dirty) = match described.strip_suffix("-dirty") {
        Some(hash) => (hash.to_owned(), true),
        None => (described, false),
    };
    let tag_is_ancestor = Command::new("git")
        .args(["merge-base", "--is-ancestor", &tag, "HEAD"])
        .current_dir(workspace)
        .output()
        .is_ok_and(|out| out.status.success());
    let since_release = if tag_is_ancestor {
        git(workspace, &["rev-list", "--count", &format!("{tag}..HEAD")])?
            .parse()
            .ok()
    } else {
        None
    };
    let has_tags = git(workspace, &["tag", "--list", "v[0-9]*"]).is_some();
    Some(Git {
        since_release,
        has_tags,
        hash,
        dirty,
    })
}

/// Whether `workspace` is the top of a git work tree, so a SeaCraft crate unpacked inside some
/// other repository does not take that repository's history for its own.
fn is_checkout(workspace: &Path) -> bool {
    let Some(top) = git(workspace, &["rev-parse", "--show-toplevel"]) else {
        return false;
    };
    match (Path::new(&top).canonicalize(), workspace.canonicalize()) {
        (Ok(top), Ok(workspace)) => top == workspace,
        _ => false,
    }
}

/// Reruns this script when the commit, the tags or the work tree change. The sources are watched
/// as well as the index, because an unstaged edit makes the build dirty without touching the index.
fn watch(workspace: &Path) {
    let git_path = |what: &str| {
        git(workspace, &["rev-parse", "--path-format=absolute", what]).map(PathBuf::from)
    };
    let mut paths = vec![
        workspace.join("Cargo.toml"),
        workspace.join("Cargo.lock"),
        workspace.join("crates"),
    ];
    if let Some(dir) = git_path("--git-dir") {
        paths.extend([dir.join("HEAD"), dir.join("index")]);
    }
    if let Some(common) = git_path("--git-common-dir") {
        paths.extend([
            common.join("refs/heads"),
            common.join("refs/tags"),
            common.join("packed-refs"),
        ]);
    }
    // A missing path would make cargo rerun this script on every build.
    for path in paths.iter().filter(|p| p.exists()) {
        println!("cargo:rerun-if-changed={}", path.display());
    }
}

/// A git command's trimmed output, or `None` when it fails or prints nothing.
fn git(dir: &Path, args: &[&str]) -> Option<String> {
    let out = Command::new("git")
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    let text = String::from_utf8(out.stdout).ok()?;
    (out.status.success() && !text.trim().is_empty()).then(|| text.trim().to_owned())
}
