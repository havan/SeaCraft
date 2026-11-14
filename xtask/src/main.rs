//! `cargo xtask ci`: the local gate (fmt, clippy, tests with debug checks, versions, demo
//! reference numbers, licences, colour literals, private files, skills).
//! `cargo xtask private`, `skills`, `colours` and `licences [--check]` run one check.
//! `cargo xtask dist --target TRIPLE` builds the folder a release archive is made from.

mod gates;

use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};

/// Reference numbers for the demo hull at T = 0.62 m (level 3, Loop mode).
/// Also quoted in `skills/seacraft-hull-numerics/SKILL.md`.
/// Reference values:
///   volume 11.851 m3, Cb 0.5042, Cp 0.7548, Cm 0.6680,
///   Cwp 0.8235, KB 0.392 m, KM 2.625 m, midship area 1.467 m2.
/// Any metric deviating by more than 1.0 % is considered a regression.
struct DemoReferenceMetric {
    name: &'static str,
    key: &'static str,
    reference: f64,
    unit: &'static str,
}

const DEMO_REFERENCE_TABLE: &[DemoReferenceMetric] = &[
    DemoReferenceMetric {
        name: "Volume",
        key: "volume_m3",
        reference: 11.851,
        unit: "m3",
    },
    DemoReferenceMetric {
        name: "Block coeff (Cb)",
        key: "block_coefficient",
        reference: 0.5042,
        unit: "",
    },
    DemoReferenceMetric {
        name: "Prismatic coeff (Cp)",
        key: "prismatic_coefficient",
        reference: 0.7548,
        unit: "",
    },
    DemoReferenceMetric {
        name: "Midship coeff (Cm)",
        key: "midship_coefficient",
        reference: 0.6680,
        unit: "",
    },
    DemoReferenceMetric {
        name: "Waterplane coeff (Cwp)",
        key: "waterplane_coefficient",
        reference: 0.8235,
        unit: "",
    },
    DemoReferenceMetric {
        name: "KB",
        key: "kb",
        reference: 0.392,
        unit: "m",
    },
    DemoReferenceMetric {
        name: "KM (transverse)",
        key: "km_transverse",
        reference: 2.625,
        unit: "m",
    },
    DemoReferenceMetric {
        name: "Midship area",
        key: "mainframe_area_m2",
        reference: 1.467,
        unit: "m2",
    },
];

const USAGE: &str =
    "Usage: cargo xtask ci [--skip-tests] [--full] | private | skills | colours | licences [--check] | dist --target TRIPLE";
const COMMANDS: &[&str] = &["ci", "private", "skills", "colours", "licences", "dist"];

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("{USAGE}");
        std::process::exit(1);
    }

    match args[1].as_str() {
        "ci" => {
            let skip_tests = args.iter().any(|a| a == "--skip-tests");
            let full = args.iter().any(|a| a == "--full");
            run_ci(skip_tests, full);
        }
        "private" => step_private(&workspace_root()),
        "skills" => step_skills(&workspace_root()),
        "colours" => step_colours(&workspace_root()),
        "licences" => {
            let check = args.iter().any(|a| a == "--check");
            step_licences(&workspace_root(), check);
        }
        "dist" => {
            let target = args
                .iter()
                .position(|a| a == "--target")
                .and_then(|i| args.get(i + 1))
                .unwrap_or_else(|| fail("dist", USAGE));
            step_dist(&workspace_root(), target);
        }
        "--help" | "-h" => {
            println!("{USAGE}");
        }
        other => {
            eprintln!("unknown command: '{other}'\n{USAGE}");
            std::process::exit(1);
        }
    }
}

fn run_ci(skip_tests: bool, full: bool) {
    let root = workspace_root();

    step_fmt(&root);

    // Step a: cargo clippy --all-targets --release -- -D warnings
    step_clippy(&root);

    // Step b: cargo test (or skipped). The dev profile is optimised but keeps debug_assert!s
    // and overflow checks, and egui has debug_assert!s of its own that a headless test can
    // trip. `--full` also runs the release build's tests, for a release or a scheduled run.
    if skip_tests {
        println!("[ci] tests: skipped (--skip-tests)");
    } else {
        step_test(&root, &["test"], "tests");
        if full {
            step_test(&root, &["test", "--release"], "tests (release)");
        }
    }

    // Step c: version check
    step_versions(&root);

    // Step d: demo reference check
    step_demo_reference(&root);

    // Step e: repository checks that need no build
    step_licences(&root, true);
    step_colours(&root);
    step_private(&root);
    step_skills(&root);

    println!("[ci] all CI checks passed successfully");
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .expect("xtask directory must have a parent")
        .to_path_buf()
}

fn fail(step: &str, msg: &str) -> ! {
    eprintln!("[ci] {step} FAILED: {msg}");
    std::process::exit(1);
}

fn step_fmt(root: &Path) {
    let status = Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(root)
        .status()
        .expect("failed to execute cargo fmt");
    if !status.success() {
        fail(
            "fmt",
            "cargo fmt --check found unformatted code; run `cargo fmt`",
        );
    }
    println!("[ci] fmt: pass");
}

fn step_clippy(root: &Path) {
    let status = Command::new("cargo")
        .args(["clippy", "--all-targets", "--", "-D", "warnings"])
        .current_dir(root)
        .status()
        .expect("failed to execute cargo clippy");

    if !status.success() {
        fail("clippy", "cargo clippy reported warnings or errors");
    }
    println!("[ci] clippy: pass");
}

fn step_test(root: &Path, args: &[&str], step: &str) {
    let output = Command::new("cargo")
        .args(args)
        .current_dir(root)
        .output()
        .expect("failed to execute cargo test");

    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).ok();
        std::io::stderr().write_all(&output.stderr).ok();
        fail(step, "cargo test exited with failure");
    }

    let mut total_passed = 0;
    let stdout_str = String::from_utf8_lossy(&output.stdout);
    let stderr_str = String::from_utf8_lossy(&output.stderr);

    for line in stdout_str.lines().chain(stderr_str.lines()) {
        if line.contains("test result:") && line.contains("passed;") {
            if let Some(pos) = line.find(" passed;") {
                let prefix = &line[..pos];
                if let Some(count_str) = prefix.split_whitespace().last() {
                    if let Ok(count) = count_str.parse::<usize>() {
                        total_passed += count;
                    }
                }
            }
        }
    }

    println!("[ci] {step}: pass ({total_passed} passed)");
}

fn step_versions(root: &Path) {
    // 1. Cargo.toml [workspace.package] version
    let cargo_toml_path = root.join("Cargo.toml");
    let cargo_toml = fs::read_to_string(&cargo_toml_path)
        .unwrap_or_else(|e| fail("versions", &format!("cannot read Cargo.toml: {e}")));

    let ws_version = parse_workspace_version(&cargo_toml).unwrap_or_else(|| {
        fail(
            "versions",
            "cannot find [workspace.package] version in Cargo.toml",
        )
    });

    // Ensure release binaries are built and Cargo.lock is updated
    let build_status = Command::new("cargo")
        .args(["build", "--bins"])
        .current_dir(root)
        .status()
        .unwrap_or_else(|e| {
            fail(
                "versions",
                &format!("cannot run cargo build --release --bins: {e}"),
            )
        });
    if !build_status.success() {
        fail("versions", "cargo build --release --bins failed");
    }

    // 2. Cargo.lock package versions
    let cargo_lock_path = root.join("Cargo.lock");
    let cargo_lock = fs::read_to_string(&cargo_lock_path)
        .unwrap_or_else(|e| fail("versions", &format!("cannot read Cargo.lock: {e}")));

    for pkg in &["seacraft-core", "seacraft-cli", "seacraft-gui"] {
        let lock_ver = parse_lock_package_version(&cargo_lock, pkg).unwrap_or_else(|| {
            fail(
                "versions",
                &format!("cannot find package '{pkg}' in Cargo.lock"),
            )
        });
        if lock_ver != ws_version {
            fail(
                "versions",
                &format!(
                    "Cargo.lock '{pkg}' version '{lock_ver}' != workspace version '{ws_version}'"
                ),
            );
        }
    }

    // 3. target/debug/seacraft --version
    let seacraft_bin = root.join("target/debug/seacraft");
    if !seacraft_bin.exists() {
        fail(
            "versions",
            &format!(
                "binary not found at {}: run cargo build --release first",
                seacraft_bin.display()
            ),
        );
    }
    let sc_out = Command::new(&seacraft_bin)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| fail("versions", &format!("cannot run seacraft --version: {e}")));
    if !sc_out.status.success() {
        fail("versions", "seacraft --version failed");
    }
    let sc_ver_str = String::from_utf8_lossy(&sc_out.stdout).trim().to_string();
    let sc_ver = sc_ver_str.split_whitespace().last().unwrap_or("");
    if sc_ver != ws_version {
        fail(
            "versions",
            &format!("seacraft binary version '{sc_ver}' != workspace version '{ws_version}'"),
        );
    }

    // 4. target/debug/seacraft-gui --version
    let seacraft_gui_bin = root.join("target/debug/seacraft-gui");
    if !seacraft_gui_bin.exists() {
        fail(
            "versions",
            &format!(
                "binary not found at {}: run cargo build --release first",
                seacraft_gui_bin.display()
            ),
        );
    }
    let sc_gui_out = Command::new(&seacraft_gui_bin)
        .arg("--version")
        .output()
        .unwrap_or_else(|e| {
            fail(
                "versions",
                &format!("cannot run seacraft-gui --version: {e}"),
            )
        });
    if !sc_gui_out.status.success() {
        fail("versions", "seacraft-gui --version failed");
    }
    let sc_gui_ver_str = String::from_utf8_lossy(&sc_gui_out.stdout)
        .trim()
        .to_string();
    let sc_gui_ver = sc_gui_ver_str.split_whitespace().last().unwrap_or("");
    if sc_gui_ver != ws_version {
        fail(
            "versions",
            &format!(
                "seacraft-gui binary version '{sc_gui_ver}' != workspace version '{ws_version}'"
            ),
        );
    }

    println!(
        "[ci] versions: pass ({ws_version} in Cargo.toml, Cargo.lock, seacraft, seacraft-gui)"
    );
}

fn parse_workspace_version(toml: &str) -> Option<String> {
    let mut in_workspace_package = false;
    for line in toml.lines() {
        let trimmed = line.trim();
        if trimmed == "[workspace.package]" {
            in_workspace_package = true;
        } else if trimmed.starts_with('[') {
            in_workspace_package = false;
        } else if in_workspace_package && trimmed.starts_with("version") {
            if let Some((_, v)) = trimmed.split_once('=') {
                let v = v.trim().trim_matches('"');
                return Some(v.to_string());
            }
        }
    }
    None
}

fn parse_lock_package_version(lock: &str, pkg_name: &str) -> Option<String> {
    let mut in_pkg = false;
    for line in lock.lines() {
        let trimmed = line.trim();
        if trimmed == "[[package]]" {
            in_pkg = false;
        } else if trimmed == format!("name = \"{pkg_name}\"") {
            in_pkg = true;
        } else if in_pkg && trimmed.starts_with("version = \"") {
            let ver = trimmed.strip_prefix("version = \"")?.strip_suffix('"')?;
            return Some(ver.to_string());
        }
    }
    None
}

fn step_demo_reference(root: &Path) {
    let seacraft_bin = root.join("target/debug/seacraft");
    let tmp_demo =
        std::env::temp_dir().join(format!("seacraft-ci-demo-{}.fef", std::process::id()));

    // 1. target/debug/seacraft demo -o <tmp>/demo.fef
    let demo_status: ExitStatus = Command::new(&seacraft_bin)
        .args(["demo", "-o"])
        .arg(&tmp_demo)
        .status()
        .unwrap_or_else(|e| fail("demo reference", &format!("cannot run seacraft demo: {e}")));

    if !demo_status.success() {
        fail("demo reference", "seacraft demo command failed");
    }

    // 2. target/debug/seacraft hydro <tmp>/demo.fef --draft 0.62 --json
    let hydro_out = Command::new(&seacraft_bin)
        .args(["hydro"])
        .arg(&tmp_demo)
        .args(["--draft", "0.62", "--json"])
        .output()
        .unwrap_or_else(|e| fail("demo reference", &format!("cannot run seacraft hydro: {e}")));

    // Clean up the temporary model and the sidecar `demo` writes beside it
    let _ = fs::remove_file(&tmp_demo);
    let _ = fs::remove_file(tmp_demo.with_extension("seacraft.json"));

    if !hydro_out.status.success() {
        std::io::stderr().write_all(&hydro_out.stderr).ok();
        fail("demo reference", "seacraft hydro command failed");
    }

    let json_val: serde_json::Value =
        serde_json::from_slice(&hydro_out.stdout).unwrap_or_else(|e| {
            fail(
                "demo reference",
                &format!("failed to parse hydro JSON: {e}"),
            )
        });

    println!(
        "  {:<24} {:>14} {:>14} {:>12}",
        "Metric", "Actual", "Reference", "Deviation"
    );
    println!("  {}", "-".repeat(68));

    let mut any_deviation_exceeded = false;
    for metric in DEMO_REFERENCE_TABLE {
        let actual = json_val
            .get(metric.key)
            .and_then(|v| v.as_f64())
            .unwrap_or_else(|| {
                fail(
                    "demo reference",
                    &format!("key '{}' missing in hydro JSON", metric.key),
                )
            });

        let diff_rel = (actual - metric.reference).abs() / metric.reference;
        let pct = (actual - metric.reference) / metric.reference * 100.0;
        let sign = if pct >= 0.0 { "+" } else { "" };

        let actual_fmt = if metric.unit.is_empty() {
            format!("{:.4}", actual)
        } else {
            format!("{:.3} {}", actual, metric.unit)
        };
        let ref_fmt = if metric.unit.is_empty() {
            format!("{:.4}", metric.reference)
        } else {
            format!("{:.3} {}", metric.reference, metric.unit)
        };
        let dev_fmt = format!("{sign}{:.2} %", pct);

        println!(
            "  {:<24} {:>14} {:>14} {:>12}",
            metric.name, actual_fmt, ref_fmt, dev_fmt
        );

        if diff_rel > 0.01 {
            any_deviation_exceeded = true;
            eprintln!(
                "  -> Metric '{}' deviated by {:.3}% (> 1.0% tolerance)",
                metric.name,
                diff_rel * 100.0
            );
        }
    }

    if any_deviation_exceeded {
        fail(
            "demo reference",
            "one or more demo metrics deviated by > 1.0%",
        );
    }

    println!("[ci] demo reference: pass (all metrics within 1.0 %)");
}

/// Every tracked file, repository-relative.
fn tracked_files(root: &Path) -> Vec<String> {
    let out = Command::new("git")
        .args(["ls-files", "-z"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| fail("git", &format!("cannot run git ls-files: {e}")));
    if !out.status.success() {
        fail("git", "git ls-files returned non-zero exit status");
    }
    String::from_utf8_lossy(&out.stdout)
        .split('\0')
        .filter(|f| !f.is_empty())
        .map(str::to_string)
        .collect()
}

/// Text of a tracked file; `None` for a binary file or one this checkout deleted.
fn read_text(root: &Path, file: &str) -> Option<String> {
    let bytes = fs::read(root.join(file)).ok()?;
    if bytes.contains(&0) {
        return None;
    }
    Some(String::from_utf8_lossy(&bytes).into_owned())
}

fn report(step: &str, violations: Vec<String>, passed: &str) {
    if !violations.is_empty() {
        fail(step, &format!("\n  {}", violations.join("\n  ")));
    }
    println!("[ci] {step}: pass ({passed})");
}

fn step_private(root: &Path) {
    let files = tracked_files(root);
    let mut machine = vec![root.to_string_lossy().into_owned()];
    if let Ok(home) = std::env::var("HOME") {
        if Path::new(&home).components().count() > 2 {
            machine.push(home);
        }
    }
    let mut bad = gates::private_path_violations(&files);
    for file in &files {
        if let Some(text) = read_text(root, file) {
            bad.extend(gates::private_text_violations(file, &text, &machine));
        }
    }
    report("private", bad, "no private files or paths tracked");
}

fn step_skills(root: &Path) {
    let read = |p: &str| read_text(root, p).unwrap_or_default();
    let mut dirs: Vec<String> = fs::read_dir(root.join("skills"))
        .unwrap_or_else(|e| fail("skills", &format!("cannot read skills/: {e}")))
        .filter_map(Result::ok)
        .filter(|e| e.path().is_dir())
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    dirs.sort();
    let bad = gates::skills_violations(
        &read("AGENTS.md"),
        &read("skills/README.md"),
        &dirs,
        &|p: &str| root.join(p).exists(),
        COMMANDS,
    );
    report("skills", bad, "AGENTS.md and skills/ agree");
}

fn step_colours(root: &Path) {
    let mut bad = Vec::new();
    for file in tracked_files(root) {
        if file.starts_with("crates/seacraft-gui/src/") && file.ends_with(".rs") {
            if let Some(text) = read_text(root, &file) {
                bad.extend(gates::colour_violations(&file, &text));
            }
        }
    }
    report("colours", bad, "colour literals only in theme.rs");
}

fn step_licences(root: &Path, check: bool) {
    let out = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| fail("licences", &format!("cannot run cargo metadata: {e}")));
    if !out.status.success() {
        fail("licences", &String::from_utf8_lossy(&out.stderr));
    }
    let meta: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| fail("licences", &format!("bad cargo metadata: {e}")));
    let members = meta["workspace_members"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let field = |p: &serde_json::Value, k: &str| p[k].as_str().unwrap_or("not stated").to_string();
    let deps: Vec<gates::Dep> = meta["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|p| !members.contains(&p["id"]))
        .map(|p| gates::Dep {
            name: field(p, "name"),
            version: field(p, "version"),
            license: field(p, "license"),
            repository: field(p, "repository"),
        })
        .collect();
    let text = gates::licences_render(&deps);
    let path = root.join("THIRD-PARTY-LICENSES.txt");
    if check {
        if fs::read_to_string(&path).ok().as_deref() != Some(text.as_str()) {
            fail(
                "licences",
                "THIRD-PARTY-LICENSES.txt is out of date; run `cargo xtask licences`",
            );
        }
        println!("[ci] licences: pass ({} dependencies)", deps.len());
    } else {
        fs::write(&path, text)
            .unwrap_or_else(|e| fail("licences", &format!("cannot write {path:?}: {e}")));
        println!("[ci] licences: wrote {} dependencies", deps.len());
    }
}

/// The crates compiled into the two programs on `target`, with their licence files.
fn shipped_crates(root: &Path, target: &str) -> Vec<gates::ShippedCrate> {
    let tree = Command::new("cargo")
        .args(["tree", "--locked", "-e", "normal", "--prefix", "none"])
        .args([
            "-p",
            "seacraft-gui",
            "-p",
            "seacraft-cli",
            "--target",
            target,
        ])
        .args(["--format", "{p}"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| fail("dist", &format!("cannot run cargo tree: {e}")));
    if !tree.status.success() {
        fail("dist", &String::from_utf8_lossy(&tree.stderr));
    }
    let wanted: std::collections::BTreeSet<(String, String)> =
        String::from_utf8_lossy(&tree.stdout)
            .lines()
            .filter_map(|line| {
                let mut words = line.split_whitespace();
                let name = words.next()?;
                let version = words.next()?.strip_prefix('v')?;
                Some((name.to_owned(), version.to_owned()))
            })
            .collect();

    let meta = Command::new("cargo")
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .unwrap_or_else(|e| fail("dist", &format!("cannot run cargo metadata: {e}")));
    if !meta.status.success() {
        fail("dist", &String::from_utf8_lossy(&meta.stderr));
    }
    let meta: serde_json::Value = serde_json::from_slice(&meta.stdout)
        .unwrap_or_else(|e| fail("dist", &format!("bad cargo metadata: {e}")));
    let members = meta["workspace_members"]
        .as_array()
        .cloned()
        .unwrap_or_default();
    let field = |p: &serde_json::Value, k: &str| p[k].as_str().unwrap_or("not stated").to_string();
    meta["packages"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or_default()
        .iter()
        .filter(|p| !members.contains(&p["id"]))
        .filter(|p| wanted.contains(&(field(p, "name"), field(p, "version"))))
        .map(|p| {
            let manifest = PathBuf::from(field(p, "manifest_path"));
            let dir = manifest
                .parent()
                .unwrap_or_else(|| fail("dist", "a manifest path has no parent"));
            gates::ShippedCrate {
                dep: gates::Dep {
                    name: field(p, "name"),
                    version: field(p, "version"),
                    license: field(p, "license"),
                    repository: field(p, "repository"),
                },
                authors: p["authors"]
                    .as_array()
                    .map(Vec::as_slice)
                    .unwrap_or_default()
                    .iter()
                    .filter_map(|a| a.as_str().map(str::to_owned))
                    .collect(),
                files: licence_files(dir),
            }
        })
        .collect()
}

/// The licence files at the top of a crate and in a `LICENSES` style folder, in a stable order.
fn licence_files(dir: &Path) -> Vec<(String, String)> {
    let mut files = Vec::new();
    let Ok(entries) = fs::read_dir(dir) else {
        return files;
    };
    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().into_owned();
        if !gates::is_licence_file(&name) {
            continue;
        }
        let path = entry.path();
        if path.is_dir() {
            for inner in fs::read_dir(&path).into_iter().flatten().flatten() {
                if let Ok(bytes) = fs::read(inner.path()) {
                    let file = format!("{name}/{}", inner.file_name().to_string_lossy());
                    files.push((file, String::from_utf8_lossy(&bytes).into_owned()));
                }
            }
        } else if let Ok(bytes) = fs::read(&path) {
            files.push((name, String::from_utf8_lossy(&bytes).into_owned()));
        }
    }
    files.sort();
    files
}

/// Standard licence texts, for crates that declare a licence but publish no file for it.
fn standard_licence_texts(root: &Path) -> Vec<(&'static str, String)> {
    let read = |name: &str| {
        fs::read_to_string(root.join("LICENSES").join(name))
            .unwrap_or_else(|e| fail("dist", &format!("cannot read LICENSES/{name}: {e}")))
    };
    // LICENSES/MIT.txt names the projects SeaCraft adapts code from; those lines are not the
    // copyright of a dependency, whose authors are listed with it instead.
    let mit: String = read("MIT.txt")
        .lines()
        .filter(|line| !line.starts_with("Copyright"))
        .map(|line| format!("{line}\n"))
        .collect();
    vec![
        ("MIT", mit),
        ("Apache-2.0", read("Apache-2.0.txt")),
        ("BSL-1.0", read("BSL-1.0.txt")),
        ("Zlib", read("Zlib.txt")),
    ]
}

/// Build the two programs for `target` and collect what a release archive holds.
fn step_dist(root: &Path, target: &str) {
    let version = env!("CARGO_PKG_VERSION");
    let status = Command::new("cargo")
        .args(["build", "--locked", "--profile", "dist", "--target", target])
        .args(["-p", "seacraft-gui", "-p", "seacraft-cli"])
        .current_dir(root)
        .status()
        .unwrap_or_else(|e| fail("dist", &format!("cannot run cargo build: {e}")));
    if !status.success() {
        fail("dist", "the build failed");
    }

    let folder = format!("seacraft-{version}-{target}");
    let stage = root.join("target").join("seacraft-dist").join(&folder);
    let copy = |from: PathBuf, to: PathBuf| {
        fs::copy(&from, &to).unwrap_or_else(|e| fail("dist", &format!("copy {from:?}: {e}")));
    };
    let copy_folder = |from: &str, to: &Path| {
        fs::create_dir_all(to).unwrap_or_else(|e| fail("dist", &format!("create {to:?}: {e}")));
        let entries = fs::read_dir(root.join(from))
            .unwrap_or_else(|e| fail("dist", &format!("read {from}: {e}")));
        for entry in entries.flatten() {
            copy(entry.path(), to.join(entry.file_name()));
        }
    };
    if stage.exists() {
        fs::remove_dir_all(&stage)
            .unwrap_or_else(|e| fail("dist", &format!("clear {stage:?}: {e}")));
    }
    fs::create_dir_all(&stage).unwrap_or_else(|e| fail("dist", &format!("create {stage:?}: {e}")));

    let exe = if target.contains("windows") {
        ".exe"
    } else {
        ""
    };
    let built = root.join("target").join(target).join("dist");
    for program in ["seacraft-gui", "seacraft"] {
        let file = format!("{program}{exe}");
        copy(built.join(&file), stage.join(&file));
    }
    for file in ["LICENSE", "NOTICE", "ATTRIBUTION.md", "DISCLAIMER.md"] {
        copy(root.join(file), stage.join(file));
    }
    copy(root.join("packaging/README.txt"), stage.join("README.txt"));
    copy_folder("LICENSES", &stage.join("LICENSES"));
    copy(
        root.join("crates/seacraft-core/assets/fonts/LICENSE-DejaVu.txt"),
        stage.join("LICENSES/DejaVu.txt"),
    );
    copy_folder("examples", &stage.join("examples"));

    let crates = shipped_crates(root, target);
    let heading = format!("Third-party licences of SeaCraft {version} for {target}.");
    let text = gates::licence_texts_render(&heading, &crates, &standard_licence_texts(root))
        .unwrap_or_else(|e| fail("dist", &e));
    fs::write(stage.join("THIRD-PARTY-LICENSES.txt"), &text)
        .unwrap_or_else(|e| fail("dist", &format!("cannot write the licences: {e}")));

    println!(
        "[ci] dist: {} ({} crates, {} KiB of licence text)",
        stage.display(),
        crates.len(),
        text.len() / 1024
    );
}
