//! `seacraft templates` and `seacraft offsets --pdf`, run as a user runs them: in a fresh
//! directory holding the demo hull.

use std::path::PathBuf;
use std::process::{Command, Output};

/// A fresh directory for one test, with `demo.fef` written into it.
fn workdir(test: &str) -> PathBuf {
    let dir =
        std::env::temp_dir().join(format!("seacraft-templates-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the test directory");
    let out = run(&dir, &["demo", "-o", "demo.fef"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    dir
}

fn run(dir: &PathBuf, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run seacraft")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

/// Run and expect success; the standard output.
fn ok(dir: &PathBuf, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(out.status.success(), "{args:?}: {}", text(&out.stderr));
    text(&out.stdout)
}

/// Run and expect failure; the standard error.
fn fails(dir: &PathBuf, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(
        !out.status.success(),
        "{args:?} should fail: {}",
        text(&out.stdout)
    );
    text(&out.stderr)
}

#[test]
fn templates_writes_a_pdf_and_says_how_many_pages() {
    let dir = workdir("pdf");
    let out = ok(&dir, &["templates", "demo.fef", "-o", "moulds.pdf"]);
    let bytes = std::fs::read(dir.join("moulds.pdf")).expect("written");
    assert!(bytes.starts_with(b"%PDF-"));
    for row in ["Station 1 ", "Station 11 "] {
        assert!(out.lines().any(|l| l.starts_with(row)), "{row} in {out}");
    }
    let pages = out
        .lines()
        .find(|l| l.ends_with(" pages, A4, scale 1:1"))
        .unwrap_or_else(|| panic!("a page line in {out}"));
    assert!(pages.split(' ').next().unwrap().parse::<usize>().unwrap() > 11);
    assert!(out.contains("no skin deducted"), "{out}");
    assert!(out.contains("wrote moulds.pdf"), "{out}");
}

#[test]
fn json_reports_the_set() {
    let dir = workdir("json");
    let out = ok(
        &dir,
        &[
            "templates",
            "demo.fef",
            "-o",
            "m.pdf",
            "--json",
            "--skin-mm",
            "6",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("only JSON on stdout");
    assert_eq!(v["format"], "pdf");
    assert!(v["pages"].is_u64(), "{v}");
    assert_eq!(v["skin_mm"], 6.0);
    assert_eq!(v["scale"], "1:1");
    assert!(v["warnings"].is_array());
    assert!(v["missing_characters"].is_array());
    let sheets = v["sheets"].as_array().unwrap();
    assert_eq!(sheets.len(), 11);
    for key in ["sheet", "x", "half_breadth_m", "face", "max_bevel_deg"] {
        assert!(sheets[0].get(key).is_some(), "{key} in {}", sheets[0]);
    }
}

#[test]
fn dxf_and_svg_refuse_paper_options() {
    let dir = workdir("dxf");
    for option in [
        &["--paper", "a3"][..],
        &["--single-page"],
        &["--overlap-mm", "5"],
        &["--margin-mm", "5"],
    ] {
        let mut args = vec!["templates", "demo.fef", "-o", "m.dxf"];
        args.extend_from_slice(option);
        let err = fails(&dir, &args);
        assert!(err.contains("apply to PDF"), "{option:?}: {err}");
    }
    ok(&dir, &["templates", "demo.fef", "-o", "m.dxf"]);
    let dxf = std::fs::read_to_string(dir.join("m.dxf")).unwrap();
    assert!(dxf.contains("CUT"));
    ok(
        &dir,
        &["templates", "demo.fef", "-o", "m.svg", "--scale", "1:10"],
    );
    assert!(std::fs::read_to_string(dir.join("m.svg"))
        .unwrap()
        .contains("<svg"));
}

#[test]
fn station_choices_conflict() {
    let dir = workdir("conflict");
    fails(
        &dir,
        &[
            "templates",
            "demo.fef",
            "-o",
            "m.pdf",
            "--stations",
            "5",
            "--spacing",
            "0.5",
        ],
    );
    fails(
        &dir,
        &[
            "templates",
            "demo.fef",
            "-o",
            "m.pdf",
            "--spacing",
            "0.5",
            "--at",
            "1,2",
        ],
    );
}

#[test]
fn bad_values_fail_with_a_reason() {
    let dir = workdir("bad");
    for (args, named) in [
        (&["--skin-mm", "-1"][..], "--skin-mm"),
        (&["--scale", "1:0"], "--scale"),
        (&["--paper", "a5"], "--paper"),
        (&["--transom", "1"], "--transom"),
        (&["--at", "999"], "no station cuts the hull"),
        (&["--waterlines", "0.5,x"], "--waterlines"),
        (&["--strongback", "0.1"], "strongback"),
    ] {
        let mut all = vec!["templates", "demo.fef", "-o", "m.pdf"];
        all.extend_from_slice(args);
        let err = fails(&dir, &all);
        assert!(err.contains(named), "{args:?}: {err}");
    }
    let err = fails(&dir, &["templates", "demo.fef", "-o", "m.png"]);
    assert!(err.contains("m.png") && err.contains(".dxf"), "{err}");
    assert!(!dir.join("m.pdf").exists() && !dir.join("m.png").exists());
}

#[test]
fn a_station_that_misses_is_reported_and_the_rest_written() {
    let dir = workdir("miss");
    let out = run(
        &dir,
        &["templates", "demo.fef", "-o", "m.pdf", "--at", "2,999"],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let err = text(&out.stderr);
    assert!(
        err.lines()
            .any(|l| l.starts_with("warning:") && l.contains("skipped")),
        "{err}"
    );
    assert!(dir.join("m.pdf").exists());
}

#[test]
fn offsets_writes_a_pdf() {
    let dir = workdir("offsets");
    let out = ok(&dir, &["offsets", "demo.fef", "--pdf", "o.pdf"]);
    assert!(out.contains("wrote o.pdf"), "{out}");
    let metric = std::fs::read(dir.join("o.pdf")).unwrap();
    assert!(metric.starts_with(b"%PDF-"));
    ok(
        &dir,
        &["offsets", "demo.fef", "--pdf", "i.pdf", "--imperial"],
    );
    let both = std::fs::read(dir.join("i.pdf")).unwrap();
    assert!(both.len() > metric.len());
    let err = fails(&dir, &["offsets", "demo.fef", "--imperial"]);
    assert!(err.contains("--imperial applies to --pdf"), "{err}");
}

/// With `--json` the standard output is the table alone, whatever files are written beside it;
/// what was written goes to the standard error.
#[test]
fn offsets_json_stays_json_beside_written_files() {
    let dir = workdir("offsets-json");
    let out = run(
        &dir,
        &[
            "offsets", "demo.fef", "--json", "--pdf", "o.pdf", "--html", "o.html", "--csv", "o.csv",
        ],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let table: serde_json::Value = serde_json::from_slice(&out.stdout)
        .unwrap_or_else(|e| panic!("{e}: {}", text(&out.stdout)));
    assert!(table["stations"].is_array());
    let err = text(&out.stderr);
    for file in ["o.pdf", "o.html", "o.csv"] {
        assert!(dir.join(file).exists(), "{file}");
        assert!(err.contains(&format!("wrote {file}")), "{err}");
    }
}
