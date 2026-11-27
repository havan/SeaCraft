//! `seacraft floodable`, run as a user runs it.

use std::process::{Command, Output};

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run_lang(lang: &str, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .env("SEACRAFT_LANG", lang)
        .args(args)
        .output()
        .expect("run seacraft")
}

fn stdout(lang: &str, args: &[&str]) -> String {
    let out = run_lang(lang, args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn floodable_json_has_three_curves_for_the_demo_hull() {
    let hull = example("demo-hull.fef");
    let text = stdout(
        "en",
        &[
            "floodable",
            &hull,
            "--level",
            "2",
            "--bulkheads",
            "3,6,9",
            "--factor",
            "0.5",
            "--json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&text).expect("json");
    let curves = v["curves"].as_array().expect("curves");
    assert_eq!(curves.len(), 3);
    let mus: Vec<f64> = curves
        .iter()
        .map(|c| c["permeability"].as_f64().unwrap())
        .collect();
    assert_eq!(mus, vec![0.60, 0.85, 0.95]);
    for c in curves {
        assert!(c["points"].as_array().unwrap().len() > 10);
        assert_eq!(c["checks"].as_array().unwrap().len(), 4);
    }
    assert_eq!(v["margin_m"].as_f64(), Some(0.076));
    assert_eq!(v["factor"].as_f64(), Some(0.5));
    assert!(
        v.get("profile").is_none(),
        "the profile is for the plot only"
    );
}

#[test]
fn floodable_takes_a_loading_and_one_permeability() {
    let hull = example("demo-hull.fef");
    let text = stdout(
        "en",
        &[
            "floodable",
            &hull,
            "--level",
            "2",
            "--displacement",
            "11.0",
            "--lcg",
            "4.5",
            "--permeability",
            "0.85",
            "--csv",
        ],
    );
    let mut lines = text.lines();
    assert_eq!(
        lines.next(),
        Some("permeability,x_mid,length,x_aft,x_fwd,trim_m,draft")
    );
    let rows: Vec<&str> = lines.collect();
    assert!(rows.len() > 10);
    assert!(rows.iter().all(|r| r.starts_with("0.85,")));
}

#[test]
fn floodable_table_in_turkish_names_the_curve() {
    let hull = example("demo-hull.fef");
    let text = stdout(
        "tr",
        &["floodable", &hull, "--level", "2", "--bulkheads", "3,6,9"],
    );
    assert!(text.contains("Su alabilir boy"), "{text}");
    assert!(text.contains("Sınır hattı"), "{text}");
}

#[test]
fn floodable_writes_an_svg() {
    let dir = std::env::temp_dir().join(format!("seacraft-floodable-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let svg = dir.join("demo-floodable.svg");
    let hull = example("demo-hull.fef");
    stdout(
        "en",
        &[
            "floodable",
            &hull,
            "--level",
            "2",
            "--svg",
            svg.to_str().unwrap(),
        ],
    );
    let text = std::fs::read_to_string(&svg).unwrap();
    assert!(text.starts_with("<?xml"));
    assert_eq!(text.matches("class=\"curve").count(), 3);
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn floodable_rejects_bad_bulkheads() {
    let hull = example("demo-hull.fef");
    let out = run_lang(
        "en",
        &["floodable", &hull, "--level", "2", "--bulkheads", "6,3"],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("bulkhead"), "{err}");
}

/// `--project` only names where `--condition` is looked up; alone, it would be ignored without a word.
#[test]
fn floodable_project_needs_a_condition() {
    let hull = example("demo-hull.fef");
    let project = example("demo-hull.seacraft.json");
    let out = run_lang(
        "en",
        &["floodable", &hull, "--level", "2", "--project", &project],
    );
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--condition"), "{err}");
}

/// A curve with no points is reported on stderr with `--csv` too, in the chosen language, and stdout stays the
/// CSV header alone: light (2 t) the demo hull would need more than its whole volume at μ 0.05.
#[test]
fn floodable_csv_reports_an_empty_curve_on_stderr() {
    let hull = example("demo-hull.fef");
    let args = [
        "floodable",
        &hull,
        "--csv",
        "--permeability",
        "0.05",
        "--displacement",
        "2",
    ];
    for (lang, warning) in [
        (
            "en",
            "no floodable length curve found for permeability 0.05",
        ),
        (
            "tr",
            "0.05 geçirgenliği için su alabilir boy eğrisi bulunamadı",
        ),
    ] {
        let out = run_lang(lang, &args);
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        let stdout = String::from_utf8(out.stdout).unwrap();
        assert_eq!(
            stdout.trim_end(),
            "permeability,x_mid,length,x_aft,x_fwd,trim_m,draft"
        );
        let stderr = String::from_utf8(out.stderr).unwrap();
        assert!(stderr.contains(warning), "{lang}: {stderr}");
    }
}
