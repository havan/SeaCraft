//! Stamps the build with what git says about the source it came from, so every binary can tell a
//! release from a development build. The naming rule and its table are in `src/build_version.rs`.
//!
//! A packager building from an archive, where there is no git, sets `SEACRAFT_BUILD_VERSION` to the
//! version the build should report, such as `0.3.0`; it must start with the package version.

#[path = "src/build_version.rs"]
mod build_version;

use build_version::{build_version, Git};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn main() {
    let package = std::env::var("CARGO_PKG_VERSION").expect("cargo sets CARGO_PKG_VERSION");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("cargo sets it"));
    let workspace = manifest.join("../..");
    println!("cargo:rerun-if-env-changed=SEACRAFT_BUILD_VERSION");
    generate_i18n(&workspace);

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

/// Convert a flat dot/underscore key like `"menu.help.about"` into `MenuHelpAbout`.
fn key_variant(key: &str) -> String {
    assert!(!key.is_empty(), "empty i18n key in locales/en.json");
    let mut out = String::with_capacity(key.len());
    for seg in key.split(['.', '_']) {
        assert!(!seg.is_empty(), "invalid i18n key '{key}': empty segment");
        let mut chars = seg.chars();
        let first = chars.next().expect("non-empty segment");
        assert!(
            first.is_ascii_lowercase() || first.is_ascii_digit(),
            "invalid i18n key '{key}': segment '{seg}' must start with [a-z0-9]"
        );
        out.push(first.to_ascii_uppercase());
        for ch in chars {
            assert!(
                ch.is_ascii_lowercase() || ch.is_ascii_digit(),
                "invalid i18n key '{key}': only [a-z0-9._] allowed"
            );
            out.push(ch);
        }
    }
    if out.chars().next().is_some_and(|c| c.is_ascii_digit()) {
        out.insert(0, 'K');
    }
    out
}

fn read_catalog(path: &Path) -> BTreeMap<String, String> {
    let text = fs::read_to_string(path)
        .unwrap_or_else(|e| panic!("cannot read locale catalog {}: {e}", path.display()));
    serde_json::from_str(&text)
        .unwrap_or_else(|e| panic!("invalid JSON in locale catalog {}: {e}", path.display()))
}

fn generate_i18n(workspace: &Path) {
    let locales_dir = workspace.join("locales");
    let en_path = locales_dir.join("en.json");
    let tr_path = locales_dir.join("tr.json");
    if locales_dir.exists() {
        println!("cargo:rerun-if-changed={}", locales_dir.display());
    }
    println!("cargo:rerun-if-changed={}", en_path.display());
    if tr_path.exists() {
        println!("cargo:rerun-if-changed={}", tr_path.display());
    }

    let en = read_catalog(&en_path);
    let tr = if tr_path.exists() {
        read_catalog(&tr_path)
    } else {
        BTreeMap::new()
    };

    let mut variants = Vec::with_capacity(en.len());
    let mut seen_variants = BTreeSet::new();
    for key in en.keys() {
        let variant = key_variant(key);
        assert!(
            seen_variants.insert(variant.clone()),
            "i18n key '{key}' collides on Rust variant name '{variant}'"
        );
        variants.push((key.as_str(), variant));
    }

    let mut code = String::from("// Generated by build.rs from locales/*.json; do not edit.\n\n");
    code.push_str("/// Compile-time checked message key from `locales/en.json`.\n");
    code.push_str("#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]\n");
    code.push_str("pub enum Key {\n");
    for (_, variant) in &variants {
        code.push_str(&format!("    {variant},\n"));
    }
    code.push_str("}\n\nimpl Key {\n");
    code.push_str(&format!(
        "    pub const COUNT: usize = {};\n",
        variants.len()
    ));
    code.push_str("    pub const ALL: &'static [Key] = &[\n");
    for (_, variant) in &variants {
        code.push_str(&format!("        Key::{variant},\n"));
    }
    code.push_str("    ];\n\n");
    code.push_str("    /// Original dot-separated key in `locales/*.json`.\n");
    code.push_str("    pub const fn id(self) -> &'static str {\n");
    code.push_str("        match self {\n");
    for (key, variant) in &variants {
        code.push_str(&format!("            Key::{variant} => {key:?},\n"));
    }
    code.push_str("        }\n    }\n}\n\n");

    code.push_str("static STRINGS_EN: [&str; Key::COUNT] = [\n");
    for value in en.values() {
        code.push_str(&format!("    {value:?},\n"));
    }
    code.push_str("];\n\n");

    code.push_str("static STRINGS_TR: [&str; Key::COUNT] = [\n");
    for (key, en_value) in &en {
        let val = tr.get(key).unwrap_or(en_value);
        code.push_str(&format!("    {val:?},\n"));
    }
    code.push_str("];\n");

    let out_dir = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    fs::write(out_dir.join("i18n_generated.rs"), code)
        .expect("cannot write generated i18n catalog");
}
