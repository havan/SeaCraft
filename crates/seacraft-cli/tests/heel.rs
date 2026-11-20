//! `seacraft heel`, run as a user runs it.

use std::process::{Command, Output};

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .args(args)
        .output()
        .expect("run seacraft")
}

fn stdout(args: &[&str]) -> String {
    let out = run(args);
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn heel_json_on_the_demo_hull() {
    let hull = example("demo-hull.fef");
    let text = stdout(&[
        "heel",
        &hull,
        "--kg",
        "2.2",
        "--weight",
        "0.5@4,0.8,1.5",
        "--heel",
        "20",
        "--level",
        "2",
        "--json",
    ]);
    let v: serde_json::Value = serde_json::from_str(&text).expect("JSON");
    assert_eq!(v["equilibrium"]["kind"], "stable", "{v}");
    let heel = v["equilibrium"]["state"]["heel_deg"].as_f64().unwrap();
    assert!(heel > 0.0, "a starboard weight heels to starboard: {heel}");
    assert_eq!(v["weights"].as_array().unwrap().len(), 1);
    assert!((v["start"]["kg_m"].as_f64().unwrap() - 2.2).abs() < 1e-12);
    let start = v["start"]["displacement_t"].as_f64().unwrap();
    let both = v["condition"]["displacement_t"].as_f64().unwrap();
    assert!((both - start - 0.5).abs() < 1e-9, "{start} + 0.5 vs {both}");
    assert_eq!(v["at_heel"]["heel_deg"], 20.0);
    assert_eq!(v["free_to_trim"], true);
}

#[test]
fn heel_text_says_where_it_settles() {
    let hull = example("demo-hull.fef");
    let text = stdout(&[
        "heel",
        &hull,
        "--kg",
        "2.2",
        "--weight",
        "0.5@4,0.8,1.5",
        "--level",
        "2",
    ]);
    assert!(text.contains("to starboard"), "{text}");
    assert!(text.contains("free to trim"), "{text}");
}

#[test]
fn heel_needs_a_kg_or_a_condition_and_a_well_formed_weight() {
    let hull = example("demo-hull.fef");
    let out = run(&["heel", &hull, "--level", "1"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("give --kg or --condition"), "{err}");

    let out = run(&[
        "heel", &hull, "--kg", "2.2", "--weight", "2@5,1", "--level", "1",
    ]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("t@x,y,z"), "{err}");

    let out = run(&["heel", &hull, "--kg", "2.2", "--lcg", "NaN", "--level", "1"]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--lcg"), "{err}");
}
