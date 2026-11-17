//! `seacraft resistance`, run as a user runs it, in a fresh directory holding the demo hull.

use std::process::Command;

#[test]
fn a_csv_only_michell_run_still_gives_its_warnings() {
    let dir = std::env::temp_dir().join(format!("seacraft-resistance-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("make the test directory");
    let run = |args: &[&str]| {
        let out = Command::new(env!("CARGO_BIN_EXE_seacraft"))
            .current_dir(&dir)
            .args(args)
            .output()
            .expect("run seacraft");
        let err = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(out.status.success(), "{args:?}: {err}");
        (String::from_utf8_lossy(&out.stdout).into_owned(), err)
    };
    run(&["demo", "-o", "demo.fef"]);

    // The demo hull's transom is immersed at its design draft (84 % of the largest section).
    let (_, err) = run(&[
        "resistance",
        "demo.fef",
        "--method",
        "michell",
        "--csv",
        "r.csv",
    ]);
    assert!(err.contains("warning: wetted transom"), "stderr: {err}");
    let csv = std::fs::read_to_string(dir.join("r.csv")).expect("the CSV is written");
    assert!(csv.starts_with("speed_kn,"), "{csv}");
    assert!(!csv.contains("warning"), "{csv}");
    let _ = std::fs::remove_dir_all(&dir);
}
