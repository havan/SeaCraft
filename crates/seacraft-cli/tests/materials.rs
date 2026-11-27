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
        "kind,name,length_m,area_m2,shell_kg,glass_kg,resin_kg,tape_kg,fillet_kg,total_kg,x_m,y_m,z_m,sheets,thickness_mm,not_counted"
    );
    assert!(out.lines().any(|l| l.starts_with("layer,")));
    assert!(
        out.lines().any(|l| l == "plywood,Transom,,,,,,,,,,,,1,12,"),
        "{out}"
    );
}

#[test]
fn plywood_rows_name_the_panels_they_do_not_count() {
    // The runabout's bottom is wider than the default stock: 0 sheets, and the row says why.
    let dir = workdir("notcounted", "runabout-6m", true);
    let out = text(&ok(&dir, &["materials", "hull.fef"]).stdout);
    let bottom = out
        .lines()
        .find(|l| l.trim_start().starts_with("Bottom ") && l.contains("mm)"))
        .unwrap_or_else(|| panic!("no plywood Bottom row: {out}"));
    assert!(
        bottom.contains("not counted: Bottom (wider than the stock)"),
        "{bottom}"
    );
    let csv = text(&ok(&dir, &["materials", "hull.fef", "--csv"]).stdout);
    assert!(
        csv.lines()
            .any(|l| l == "plywood,Bottom,,,,,,,,,,,,0,10,Bottom (wider than the stock)"),
        "{csv}"
    );
    let width = csv_columns(csv.lines().next().unwrap());
    for l in csv.lines() {
        assert_eq!(csv_columns(l), width, "{l}");
    }
}

#[test]
fn one_sheet_is_one_sheet() {
    let dir = workdir("onesheet", "skiff-4m", true);
    let out = text(&ok(&dir, &["materials", "hull.fef"]).stdout);
    assert!(out.contains("1 sheet (12.0 mm)"), "{out}");
    assert!(out.contains("2 sheets (9.0 mm)"), "{out}");
    assert!(!out.contains("1 sheets"), "{out}");
}

#[test]
fn the_total_centre_prints_no_negative_zero() {
    // The demo hull's centre lies a rounding step to port of the centreline.
    let dir = workdir("negzero", "demo-hull", false);
    let out = text(&ok(&dir, &["materials", "hull.fef"]).stdout);
    assert!(out.contains(" y 0.000 z "), "{out}");
    assert!(!out.contains("-0.000"), "{out}");
}

fn read(dir: &Path, name: &str) -> Vec<u8> {
    std::fs::read(dir.join(name)).unwrap()
}

/// Counts the columns of one CSV line, honouring double quotes.
fn csv_columns(line: &str) -> usize {
    let (mut n, mut quoted) = (1, false);
    for c in line.chars() {
        match c {
            '"' => quoted = !quoted,
            ',' if !quoted => n += 1,
            _ => {}
        }
    }
    n
}

#[test]
fn csv_rows_keep_the_header_width_when_a_layer_name_has_a_comma() {
    let dir = workdir("csvcomma", "skiff-4m", true);
    let fef = std::fs::read_to_string(dir.join("hull.fef")).unwrap();
    assert!(fef.contains("\nTransom\n"));
    std::fs::write(
        dir.join("hull.fef"),
        fef.replace("\nTransom\n", "\nTran,som\n"),
    )
    .unwrap();
    let out = text(&ok(&dir, &["materials", "hull.fef", "--csv"]).stdout);
    assert!(out.contains("\"Tran,som\""), "{out}");
    let width = csv_columns(out.lines().next().unwrap());
    for l in out.lines() {
        assert_eq!(csv_columns(l), width, "{l}");
    }
}

#[test]
fn an_invalid_schedule_is_refused_naming_the_field() {
    let dir = workdir("invalid", "skiff-4m", true);
    let mut p = project(&dir);
    p["materials"] = serde_json::json!({
        "resin_per_glass": 1.0,
        "layers": {"Bottom": [{"g_m2": 0.0, "side": "outside"}]}
    });
    std::fs::write(dir.join("hull.seacraft.json"), p.to_string()).unwrap();
    let err = fails(&dir, &["materials", "hull.fef"]);
    assert!(err.contains("layers.Bottom[0].g_m2"), "{err}");
}

#[test]
fn bare_refusal_names_the_file() {
    let dir = workdir("barename", "demo-hull", false);
    let examples = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
    std::fs::copy(examples.join("demo-loadcase.json"), dir.join("case.json")).unwrap();
    let err = fails(
        &dir,
        &["materials", "hull.fef", "case.json", "--add-to", "Design"],
    );
    assert!(err.contains("case.json"), "{err}");
}

#[test]
fn refusals_leave_the_project_file_unchanged() {
    let dir = workdir("unchanged", "skiff-4m", true);
    let before = read(&dir, "hull.seacraft.json");
    fails(&dir, &["materials", "hull.fef", "--add-to", "Nope"]);
    assert_eq!(read(&dir, "hull.seacraft.json"), before);
    let dir = workdir("unchanged-nosidecar", "skiff-4m", false);
    fails(&dir, &["materials", "hull.fef", "--add-to", "Design"]);
    assert!(!dir.join("hull.seacraft.json").exists());
}

#[test]
fn item_names_the_written_weight_item() {
    let dir = workdir("item", "skiff-4m", true);
    let cond = first_condition(&dir);
    ok(
        &dir,
        &[
            "materials",
            "hull.fef",
            "--add-to",
            &cond,
            "--item",
            "My hull",
        ],
    );
    let items = project(&dir)["loading_conditions"][0]["items"].clone();
    assert!(items
        .as_array()
        .unwrap()
        .iter()
        .any(|i| i["name"] == "My hull"));
}

#[test]
fn default_table_prints_the_not_validated_note() {
    let dir = workdir("note", "skiff-4m", true);
    let out = text(&ok(&dir, &["materials", "hull.fef"]).stdout);
    let note = seacraft_core::not_validated_note(seacraft_core::i18n::Locale::En);
    assert!(out.contains(note), "{out}");
}

#[test]
fn written_mass_equals_the_estimate_total() {
    let dir = workdir("mass", "skiff-4m", true);
    let cond = first_condition(&dir);
    let json = text(&ok(&dir, &["materials", "hull.fef", "--json"]).stdout);
    let v: serde_json::Value = serde_json::from_str(&json).unwrap();
    let total = v["total"]["mass_t"].as_f64().unwrap();
    ok(&dir, &["materials", "hull.fef", "--add-to", &cond]);
    let items = project(&dir)["loading_conditions"][0]["items"].clone();
    let item = items
        .as_array()
        .unwrap()
        .iter()
        .find(|i| i["name"] == ITEM)
        .unwrap()
        .clone();
    assert!((item["mass_t"].as_f64().unwrap() - total).abs() < 1e-12);
}
