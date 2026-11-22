//! `seacraft panels`, run as a user runs it: in a fresh directory holding the test pram (a
//! hard-chine fixture from `seacraft_core::panels`, not an example hull) and the demo hull.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test, with `pram.fef` (flat bottom, planar sides, every layer
/// marked developable) and `demo.fef` written into it.
fn workdir(test: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("seacraft-panels-{}-{test}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the test directory");
    let mut pram = seacraft_core::panels::pram_fixture(0.0, 0.0);
    for layer in &mut pram.layers {
        layer.developable = true;
    }
    std::fs::write(dir.join("pram.fef"), seacraft_core::fef::write(&pram)).expect("write the pram");
    let out = run(&dir, &["demo", "-o", "demo.fef"]);
    assert!(out.status.success(), "{}", text(&out.stderr));
    dir
}

fn run(dir: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .current_dir(dir)
        .args(args)
        .output()
        .expect("run seacraft")
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn ok(dir: &Path, args: &[&str]) -> String {
    let out = run(dir, args);
    assert!(out.status.success(), "{args:?}: {}", text(&out.stderr));
    text(&out.stdout)
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

#[test]
fn panels_writes_a_tiled_pdf_and_offsets() {
    let dir = workdir("pdf");
    let out = ok(
        &dir,
        &[
            "panels",
            "pram.fef",
            "-o",
            "p.pdf",
            "--offsets-csv",
            "o.csv",
            "--offsets-pdf",
            "o.pdf",
            "--json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON on standard output");
    for f in ["p.pdf", "o.csv", "o.pdf"] {
        assert!(dir.join(f).exists(), "{f}");
    }
    assert!(std::fs::read(dir.join("p.pdf"))
        .unwrap()
        .starts_with(b"%PDF-"));
    assert!(std::fs::read_to_string(dir.join("o.csv"))
        .unwrap()
        .starts_with("panel,along_mm,crossing,ordinate_mm\n"));
    assert!(v["total_sheets"].as_u64().unwrap() >= 3, "{v}");
    let panels = v["panels"].as_array().unwrap();
    assert_eq!(panels.len(), 4, "{v}");
    assert!(panels.iter().all(|p| p["lies_flat"] == true), "{v}");
    assert_eq!(v["format"], "pdf");
}

#[test]
fn panels_prints_a_table_and_the_sheet_count() {
    let dir = workdir("table");
    let out = ok(&dir, &["panels", "pram.fef", "-o", "p.svg"]);
    assert!(out.contains("Side"), "{out}");
    assert!(
        out.contains("Bottom against Side: stitched edges differ by up to 0.0 mm at the ticks"),
        "{out}"
    );
    assert!(
        out.contains("plywood sheets, end to end, no nesting: an upper bound"),
        "{out}"
    );
    assert!(out.contains("wrote p.svg"), "{out}");
}

#[test]
fn a_hull_that_does_not_lie_flat_writes_nothing() {
    let dir = workdir("demo");
    let err = fails(&dir, &["panels", "demo.fef", "-o", "p.pdf"]);
    assert!(err.contains("--allow-undevelopable"), "{err}");
    assert!(
        err.contains("Bottom: Does not lie flat: must stretch") && err.contains("nothing written"),
        "{err}"
    );
    assert!(!dir.join("p.pdf").exists());
}

#[test]
fn allow_undevelopable_writes_it_headed() {
    let dir = workdir("allow");
    let out = ok(
        &dir,
        &["panels", "demo.fef", "-o", "p.pdf", "--allow-undevelopable"],
    );
    assert!(dir.join("p.pdf").exists());
    assert!(
        out.contains("do not lie flat within the limits: their patterns are headed so"),
        "{out}"
    );
}

#[test]
fn an_unknown_layer_is_an_error() {
    let dir = workdir("layer");
    let err = fails(
        &dir,
        &["panels", "pram.fef", "-o", "p.pdf", "--layer", "Keel"],
    );
    for w in ["Keel", "Bottom", "Side", "Transoms"] {
        assert!(err.contains(w), "{err}");
    }
    assert!(!dir.join("p.pdf").exists());
}

#[test]
fn svg_refuses_paper_options() {
    let dir = workdir("svg");
    let err = fails(
        &dir,
        &["panels", "pram.fef", "-o", "p.svg", "--paper", "a3"],
    );
    assert!(err.contains("--paper") && err.contains("PDF only"), "{err}");
}

#[test]
fn bad_limits_and_stock_are_refused_before_any_work() {
    let dir = workdir("limits");
    let err = fails(
        &dir,
        &["panels", "pram.fef", "-o", "p.pdf", "--stock-mm", "2440"],
    );
    assert!(err.contains("--stock-mm"), "{err}");
    let err = fails(
        &dir,
        &[
            "panels",
            "pram.fef",
            "-o",
            "p.pdf",
            "--max-stretch-pct",
            "0",
        ],
    );
    assert!(err.contains("stretch"), "{err}");
}

#[test]
fn a_refused_panel_is_named_and_not_counted() {
    let dir = workdir("refused");
    let mut pram = seacraft_core::panels::pram_fixture(0.0, 0.0);
    for layer in &mut pram.layers {
        layer.developable = true;
    }
    let fi = pram.face_layer.iter().position(|&l| l == 1).unwrap() + 3;
    pram.faces[fi].reverse();
    pram.rebuild_edges();
    std::fs::write(dir.join("flipped.fef"), seacraft_core::fef::write(&pram)).unwrap();
    let err = fails(&dir, &["panels", "flipped.fef", "-o", "p.pdf"]);
    assert!(err.contains("Side cannot be laid flat"), "{err}");
    let out = ok(
        &dir,
        &[
            "panels",
            "flipped.fef",
            "-o",
            "p.pdf",
            "--allow-undevelopable",
        ],
    );
    assert!(
        out.contains("plywood sheets for 3 of 4 panels")
            && out.contains("Side (could not be laid flat)"),
        "{out}"
    );
    assert!(
        out.contains("1 panel could not be laid flat and is not in the file: Side"),
        "{out}"
    );
    assert!(!out.contains("their patterns are headed so"), "{out}");
}

#[test]
fn two_outputs_to_one_file_are_refused_before_any_is_written() {
    let dir = workdir("same");
    let absolute = dir.join("p.pdf");
    let absolute = absolute.to_str().unwrap();
    for (flag, other) in [
        ("--offsets-pdf", "./p.pdf"),
        ("--offsets-pdf", absolute),
        ("--offsets-csv", "p.pdf"),
    ] {
        let err = fails(&dir, &["panels", "pram.fef", "-o", "p.pdf", flag, other]);
        assert!(
            err.contains(flag) && err.contains("same file") && err.contains("-o"),
            "{err}"
        );
        assert!(!dir.join("p.pdf").exists(), "{flag} {other}");
    }
    std::fs::create_dir_all(dir.join("sub")).unwrap();
    let err = fails(
        &dir,
        &[
            "panels",
            "pram.fef",
            "-o",
            "sub/../p.pdf",
            "--offsets-pdf",
            "p.pdf",
        ],
    );
    assert!(err.contains("same file"), "{err}");
    assert!(!dir.join("p.pdf").exists());
    // Three different files are written.
    ok(
        &dir,
        &[
            "panels",
            "pram.fef",
            "-o",
            "p.pdf",
            "--offsets-csv",
            "sub/../o.csv",
            "--offsets-pdf",
            "o.pdf",
        ],
    );
    for f in ["p.pdf", "o.csv", "o.pdf"] {
        assert!(dir.join(f).exists(), "{f}");
    }
}

#[test]
fn no_offsets_pages_leaves_them_out_of_the_pdf() {
    let dir = workdir("offsets-pages");
    let json = |extra: &[&str]| -> serde_json::Value {
        let mut args = vec!["panels", "pram.fef", "-o", "p.pdf", "--json"];
        args.extend_from_slice(extra);
        serde_json::from_str(&ok(&dir, &args)).expect("JSON on standard output")
    };
    let with = json(&[]);
    let without = json(&["--no-offsets-pages"]);
    let tables = with["offsets_pages"].as_u64().expect("offsets_pages");
    assert!(tables >= 1, "{with}");
    assert_eq!(without["offsets_pages"], 0, "{without}");
    assert_eq!(
        with["pages"].as_u64().unwrap(),
        without["pages"].as_u64().unwrap() + tables
    );
}

#[test]
fn panels_nest_writes_a_file_per_sheet() {
    let dir = workdir("nest");
    let ex = run(&dir, &["example", "skiff-4m", "-o", "skiff.fef"]);
    assert!(ex.status.success(), "{}", text(&ex.stderr));

    let out = ok(
        &dir,
        &["panels", "skiff.fef", "-o", "x.svg", "--nest", "split"],
    );
    assert!(out.contains("nested"), "{out}");
    assert!(out.contains("wrote x.svg"), "{out}");
    assert!(out.contains("wrote x-sheet-1.svg"), "{out}");
    assert!(std::fs::read_to_string(dir.join("x.svg"))
        .unwrap()
        .contains("<svg"));
    assert!(std::fs::read_to_string(dir.join("x-sheet-1.svg"))
        .unwrap()
        .contains("<svg"));

    let json_out = ok(
        &dir,
        &[
            "panels",
            "skiff.fef",
            "-o",
            "j.svg",
            "--nest",
            "split",
            "--json",
        ],
    );
    let v: serde_json::Value = serde_json::from_str(&json_out).expect("JSON");
    assert_eq!(v["layout"]["mode"], "split", "{v}");
    let sheets = v["layout"]["sheets"].as_array().expect("layout.sheets");
    assert!(!sheets.is_empty(), "{v}");
    let files = v["sheet_files"].as_array().expect("sheet_files");
    assert_eq!(files.len(), sheets.len(), "{v}");
    for (i, f) in files.iter().enumerate() {
        let name = f.as_str().expect("string path");
        assert_eq!(name, format!("j-sheet-{}.svg", i + 1), "{v}");
        let body = std::fs::read_to_string(dir.join(name)).expect("sheet file exists");
        assert!(body.contains("<svg"), "{name}");
    }

    assert_eq!(v["stale_sheet_files"], serde_json::json!([]), "{v}");

    // A later export with fewer sheets names the older files it leaves and touches none.
    let extra = format!("j-sheet-{}.svg", files.len() + 1);
    std::fs::write(dir.join(&extra), "hand edited").unwrap();
    let out = run(
        &dir,
        &["panels", "skiff.fef", "-o", "j.svg", "--nest", "split"],
    );
    assert!(out.status.success(), "{}", text(&out.stderr));
    let err = text(&out.stderr);
    assert!(err.contains("warning: 1 older sheet file"), "{err}");
    assert!(err.contains(&extra), "{err}");
    assert_eq!(
        std::fs::read_to_string(dir.join(&extra)).unwrap(),
        "hand edited"
    );
    let again: serde_json::Value = serde_json::from_str(&ok(
        &dir,
        &[
            "panels",
            "skiff.fef",
            "-o",
            "j.svg",
            "--nest",
            "split",
            "--json",
        ],
    ))
    .expect("JSON");
    assert_eq!(
        again["stale_sheet_files"],
        serde_json::json!([extra.clone()]),
        "{again}"
    );

    let plain_json = ok(&dir, &["panels", "skiff.fef", "-o", "plain.svg", "--json"]);
    let pv: serde_json::Value = serde_json::from_str(&plain_json).expect("JSON");
    assert!(pv["layout"].is_null(), "{pv}");
    assert_eq!(pv["sheet_files"], serde_json::json!([]), "{pv}");

    for bad_gap in ["0", "-5"] {
        let err = fails(
            &dir,
            &[
                "panels",
                "skiff.fef",
                "-o",
                "bad.svg",
                "--nest",
                "split",
                "--gap-mm",
                bad_gap,
            ],
        );
        assert!(err.contains("--gap-mm"), "{err}");
    }
    let err = fails(
        &dir,
        &["panels", "skiff.fef", "-o", "bad.svg", "--nest", "tetris"],
    );
    assert!(
        err.contains("--nest") && err.contains("none, blanks or split"),
        "{err}"
    );

    let err = fails(
        &dir,
        &[
            "panels",
            "skiff.fef",
            "-o",
            "clash.svg",
            "--nest",
            "split",
            "--offsets-csv",
            "clash-sheet-1.svg",
        ],
    );
    assert!(err.contains("same file"), "{err}");
    assert!(!dir.join("clash.svg").exists());
    assert!(!dir.join("clash-sheet-1.svg").exists());

    let _ = std::fs::remove_dir_all(&dir);
}
