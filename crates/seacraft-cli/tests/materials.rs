//! `seacraft materials`, run as a user runs it, on copies of the example hulls in a fresh directory.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const ITEM: &str = "Hull materials (estimate)";

/// A fresh directory holding `hull.fef`, and `hull.seacraft.json` when `sidecar` is set, both
/// copied from the example `name`.
fn workdir(test: &str, name: &str, sidecar: bool) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("seacraft-materials-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the test directory");
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    std::fs::copy(examples.join(format!("{name}.fef")), dir.join("hull.fef")).expect("copy hull");
    if sidecar {
        std::fs::copy(
            examples.join(format!("{name}.seacraft.json")),
            dir.join("hull.seacraft.json"),
        )
        .expect("copy sidecar");
    }
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .env("SEACRAFT_LANG", "en")
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run seacraft")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ok(dir: &Path, args: &[&str]) -> Output {
    let out = run(dir, args);
    assert!(out.status.success(), "{args:?}: {}", text(&out.stderr));
    out
}

fn fails(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(
        !out.status.success(),
        "{args:?} should fail: {}",
        text(&out.stdout)
    );
    text(&out.stderr)
}

fn project(dir: &Path) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(dir.join("hull.seacraft.json")).unwrap()).unwrap()
}

fn first_condition(dir: &Path) -> String {
    project(dir)["loading_conditions"][0]["name"]
        .as_str()
        .unwrap()
        .to_owned()
}

#[test]
fn materials_reports_shell_without_a_schedule() {
    let dir = workdir("noschedule", "demo-hull", false);
    let out = text(&ok(&dir, &["materials", "hull.fef"]).stdout);
    assert!(out.contains("no glass or resin entered"), "{out}");
    assert!(out.contains("Total"), "{out}");
}

#[test]
fn materials_json_has_layers_and_total() {
    let dir = workdir("json", "skiff-4m", true);
    let out = text(&ok(&dir, &["materials", "hull.fef", "--json"]).stdout);
    let v: serde_json::Value = serde_json::from_str(&out).expect("json");
    assert_eq!(v["layers"].as_array().unwrap().len(), 3);
    assert!(v["total"]["mass_kg"].as_f64().unwrap() > 0.0);
    assert_eq!(v["plywood"].as_array().unwrap().len(), 3);
}

#[test]
fn add_to_writes_once_and_replaces_on_second_run() {
    let dir = workdir("addto", "skiff-4m", true);
    let cond = first_condition(&dir);
    for _ in 0..2 {
        let out = ok(&dir, &["materials", "hull.fef", "--add-to", &cond]);
        let so = text(&out.stdout);
        assert!(so.contains("wrote") && so.contains(ITEM), "{so}");
    }
    let p = project(&dir);
    let n = p["loading_conditions"][0]["items"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|i| i["name"] == ITEM)
        .count();
    assert_eq!(n, 1);
}

#[test]
fn add_to_with_json_sends_wrote_to_stderr() {
    let dir = workdir("addtojson", "skiff-4m", true);
    let cond = first_condition(&dir);
    let out = ok(
        &dir,
        &["materials", "hull.fef", "--json", "--add-to", &cond],
    );
    assert!(text(&out.stderr).contains("wrote"));
    serde_json::from_str::<serde_json::Value>(&text(&out.stdout)).expect("stdout stays json");
}

#[test]
fn add_to_unknown_condition_lists_names() {
    let dir = workdir("unknown", "skiff-4m", true);
    let cond = first_condition(&dir);
    let err = fails(&dir, &["materials", "hull.fef", "--add-to", "Nope"]);
    assert!(err.contains(&cond), "{err}");
}

#[test]
fn add_to_without_project_file_refuses() {
    let dir = workdir("nosidecar", "skiff-4m", false);
    let err = fails(&dir, &["materials", "hull.fef", "--add-to", "Design"]);
    assert!(err.contains("hull.seacraft.json"), "{err}");
}

#[test]
fn add_to_refuses_a_bare_loading_condition() {
    let dir = workdir("bare", "demo-hull", false);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    std::fs::copy(examples.join("demo-loadcase.json"), dir.join("case.json")).unwrap();
    fails(
        &dir,
        &["materials", "hull.fef", "case.json", "--add-to", "Design"],
    );
    ok(&dir, &["materials", "hull.fef", "case.json"]);
}

#[test]
fn csv_has_kind_column() {
    let dir = workdir("csv", "skiff-4m", true);
    let out = text(&ok(&dir, &["materials", "hull.fef", "--csv"]).stdout);
    assert_eq!(
        out.lines().next().unwrap(),
        "kind,name,length_m,area_m2,shell_kg,glass_kg,resin_kg,tape_kg,fillet_kg,total_kg,x,y,z,sheets"
    );
    assert!(out.lines().any(|l| l.starts_with("layer,")));
    assert!(out.lines().any(|l| l.starts_with("plywood,")));
}
