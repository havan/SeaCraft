//! `seacraft waves`, run as a user runs it, in a fresh directory per test.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

/// A fresh directory for one test, removed when dropped.
struct Dir(PathBuf);

impl Dir {
    fn new(name: &str) -> Self {
        let dir =
            std::env::temp_dir().join(format!("seacraft-waves-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("make the test directory");
        Dir(dir)
    }

    fn run(&self, args: &[&str]) -> Output {
        Command::new(env!("CARGO_BIN_EXE_seacraft"))
            .env("SEACRAFT_LANG", "en")
            .current_dir(&self.0)
            .args(args)
            .output()
            .expect("run seacraft")
    }

    /// Run and require success; (stdout, stderr).
    fn ok(&self, args: &[&str]) -> (String, String) {
        let out = self.run(args);
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "{args:?}: {err}");
        (String::from_utf8_lossy(&out.stdout).into_owned(), err)
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.join(name)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn exists(p: &Path) -> bool {
    std::fs::metadata(p).map(|m| m.len() > 0).unwrap_or(false)
}

#[test]
fn waves_writes_all_three_formats() {
    let d = Dir::new("formats");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    let (out, _) = d.ok(&[
        "waves", "w.fef", "--fn", "0.3", "-o", "p.png", "-o", "p.svg", "-o", "p.vti",
    ]);
    for name in ["p.png", "p.svg", "p.vti"] {
        assert!(exists(&d.path(name)), "{name} not written; stdout: {out}");
    }
    let png = std::fs::read(d.path("p.png")).unwrap();
    assert_eq!(&png[..8], b"\x89PNG\r\n\x1a\n");
    assert!(out.contains("Far-field thin-ship"), "{out}");
    assert!(out.contains("Fn 0.300"), "{out}");
}

/// Review focus 2: a transom hull still gets a pattern, with Michell's transom warning.
#[test]
fn waves_flags_the_demo_transom() {
    let d = Dir::new("transom");
    d.ok(&["demo", "-o", "demo.fef"]);
    let (_, err) = d.ok(&["waves", "demo.fef", "--speed", "8", "-o", "d.png"]);
    assert!(err.contains("warning: wetted transom"), "stderr: {err}");
    assert!(exists(&d.path("d.png")));
}

#[test]
fn waves_refuses_an_unknown_extension() {
    let d = Dir::new("extension");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    let out = d.run(&[
        "waves", "w.fef", "--fn", "0.3", "-o", "p.png", "-o", "p.jpg",
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success());
    assert!(err.contains("p.jpg"), "stderr: {err}");
    assert!(
        !d.path("p.png").exists(),
        "nothing is written when one output is refused"
    );
}

/// Two outputs on one file would leave only the last while both are reported.
#[test]
fn waves_refuses_two_outputs_on_one_file() {
    let d = Dir::new("collide");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    std::fs::create_dir(d.path("sub")).unwrap();
    for args in [
        &["-o", "p.png", "--cut-y", "2", "--cut-csv", "p.png"][..],
        &["-o", "p.png", "-o", "sub/../p.png"][..],
    ] {
        let mut all = vec!["waves", "w.fef", "--fn", "0.3"];
        all.extend_from_slice(args);
        let out = d.run(&all);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} was accepted");
        assert!(err.contains("same file"), "{args:?}: {err}");
        assert!(!d.path("p.png").exists(), "{args:?} wrote a file");
    }
    // Distinct files are still written.
    d.ok(&[
        "waves",
        "w.fef",
        "--fn",
        "0.3",
        "-o",
        "p.png",
        "--cut-y",
        "2",
        "--cut-csv",
        "p.csv",
    ]);
    assert!(exists(&d.path("p.png")) && exists(&d.path("p.csv")));
}

/// A dangling link to another output lands on that output once it is written.
#[cfg(unix)]
#[test]
fn waves_refuses_an_output_linked_to_another() {
    let d = Dir::new("link");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    std::os::unix::fs::symlink("p.png", d.path("q.svg")).unwrap();
    let out = d.run(&[
        "waves", "w.fef", "--fn", "0.3", "-o", "p.png", "-o", "q.svg",
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "the linked output was accepted");
    assert!(err.contains("same file"), "{err}");
    assert!(!d.path("p.png").exists(), "a refused run wrote a file");
}

/// A loop of links has no landing: the first write replaces one link and the next lands on it.
#[cfg(unix)]
#[test]
fn waves_refuses_a_loop_of_links() {
    let d = Dir::new("loop");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    for (link, target) in [("a.png", "b.svg"), ("b.svg", "c.vti"), ("c.vti", "a.png")] {
        std::os::unix::fs::symlink(target, d.path(link)).unwrap();
    }
    let out = d.run(&[
        "waves", "w.fef", "--fn", "0.3", "-o", "a.png", "-o", "b.svg",
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "a loop of links was accepted");
    assert!(err.contains("a.png") && err.contains("links"), "{err}");
    for (link, target) in [("a.png", "b.svg"), ("b.svg", "c.vti"), ("c.vti", "a.png")] {
        let now = std::fs::read_link(d.path(link)).ok();
        assert_eq!(
            now,
            Some(PathBuf::from(target)),
            "a refused run changed {link}"
        );
    }
}

/// A folder that cannot be resolved (missing, or a link to itself) is refused before any output is
/// written, not after the first one.
#[cfg(unix)]
#[test]
fn waves_refuses_an_unresolvable_folder_before_writing() {
    let d = Dir::new("loopdir");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    std::os::unix::fs::symlink("loop", d.path("loop")).unwrap();
    for folder in ["loop", "missing"] {
        let second = format!("{folder}/q.svg");
        let out = d.run(&[
            "waves", "w.fef", "--fn", "0.3", "-o", "p.png", "-o", &second,
        ]);
        assert!(!out.status.success(), "{second} was accepted");
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(err.contains(&second), "{err}");
        assert!(
            !d.path("p.png").exists(),
            "p.png was written before refusing {second}"
        );
    }
}

/// An output that cannot be written (no file name, a directory in its place, a file where its folder
/// should be) fails the run before any output is written.
#[test]
fn waves_writes_nothing_when_one_output_cannot_be_written() {
    let d = Dir::new("unwritable");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    std::fs::create_dir(d.path("q.svg")).unwrap();
    std::fs::write(d.path("blocker"), "a file").unwrap();
    for args in [
        &["--cut-y", "2", "--cut-csv", ".."][..],
        &["-o", "q.svg"][..],
        &["-o", "blocker/q.svg"][..],
    ] {
        let mut all = vec!["waves", "w.fef", "--fn", "0.3", "-o", "p.png"];
        all.extend_from_slice(args);
        let out = d.run(&all);
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(!out.status.success(), "{args:?} was accepted");
        assert!(
            !d.path("p.png").exists(),
            "{args:?}: p.png was written; {err}"
        );
    }
}

/// On a case-blind file system (Windows, macOS by default) p.png and P.PNG are one file, so names
/// that differ only in case are refused everywhere.
#[test]
fn waves_refuses_outputs_that_differ_only_in_case() {
    let d = Dir::new("case");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    let out = d.run(&[
        "waves",
        "w.fef",
        "--fn",
        "0.3",
        "-o",
        "p.png",
        "--cut-y",
        "2",
        "--cut-csv",
        "P.PNG",
    ]);
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(!out.status.success(), "p.png and P.PNG were accepted");
    assert!(err.contains("same file"), "{err}");
    for name in ["p.png", "P.PNG"] {
        assert!(!d.path(name).exists(), "a refused run wrote {name}");
    }
}

#[test]
fn waves_needs_exactly_one_of_speed_and_fn() {
    let d = Dir::new("speed");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    assert!(!d.run(&["waves", "w.fef"]).status.success());
    assert!(!d
        .run(&["waves", "w.fef", "--fn", "0.3", "--speed", "8"])
        .status
        .success());
    let out = d.run(&["waves", "w.fef", "--fn", "-0.3"]);
    assert!(!out.status.success());
    assert!(String::from_utf8_lossy(&out.stderr).contains("speed"));
}

#[test]
fn waves_json_summary() {
    let d = Dir::new("json");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    let (out, _) = d.ok(&["waves", "w.fef", "--fn", "0.3", "-o", "p.svg", "--json"]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON on stdout");
    assert!((v["froude"].as_f64().unwrap() - 0.3).abs() < 1e-9, "{v}");
    assert!(v["theta_cut_deg"].as_f64().unwrap() > 0.0);
    assert_eq!(v["files"][0], "p.svg");
    assert!(
        v.get("zeta").is_none() && v.get("xs").is_none(),
        "the grid stays out of the summary"
    );
    assert!(v["notes"][0]
        .as_str()
        .unwrap()
        .starts_with("Far-field thin-ship"));
}

#[test]
fn waves_writes_a_longitudinal_cut() {
    let d = Dir::new("cut");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    d.ok(&[
        "waves",
        "w.fef",
        "--fn",
        "0.3",
        "--cut-y",
        "2",
        "--cut-csv",
        "c.csv",
    ]);
    let csv = std::fs::read_to_string(d.path("c.csv")).expect("the cut is written");
    let mut lines = csv.lines();
    assert_eq!(lines.next(), Some("x_m,zeta_m"));
    assert_eq!(lines.count(), 2000);
}

#[test]
fn waves_reports_wash_height_and_regime() {
    let d = Dir::new("wash");
    d.ok(&["example", "wigley", "-o", "w.fef"]);
    let (out, _) = d.ok(&[
        "waves",
        "w.fef",
        "--fn",
        "0.45",
        "--wash-distance",
        "9",
        "--json",
    ]);
    let v: serde_json::Value = serde_json::from_str(&out).expect("JSON");
    assert_eq!(v["wash"]["distance_m"], 9.0, "{v}");
    assert!(v["wash"]["height_m"].as_f64().unwrap() > 0.0, "{v}");
    assert_eq!(v["regime"]["key"], "hull_speed", "{v}");
    let (text, _) = d.ok(&["waves", "w.fef", "--fn", "0.45", "--wash-distance", "9"]);
    assert!(text.contains("Around hull speed"), "{text}");
    assert!(text.contains("wash height at 9.00 m: "), "{text}");
}
