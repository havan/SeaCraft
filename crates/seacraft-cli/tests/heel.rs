//! `seacraft heel`, run as a user runs it.

use std::process::{Command, Output};

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .env("SEACRAFT_LANG", "en")
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

    // The solved waterplane, `n . p + d = 0` in model coordinates, so a drawing can place the
    // waterline without solving for it again.
    let state = &v["equilibrium"]["state"];
    let plane = &state["waterplane"];
    let n = |k: &str| plane["n"][k].as_f64().unwrap();
    let side = |p: &serde_json::Value| {
        n("x") * p["x"].as_f64().unwrap()
            + n("y") * p["y"].as_f64().unwrap()
            + n("z") * p["z"].as_f64().unwrap()
            + plane["d"].as_f64().unwrap()
    };
    let length = (n("x").powi(2) + n("y").powi(2) + n("z").powi(2)).sqrt();
    assert!((length - 1.0).abs() < 1e-9, "unit normal: {plane}");
    assert!(
        side(&state["b"]) < 0.0,
        "B lies under the waterplane: {state}"
    );
    let tilt = (-n("y")).atan2(n("z")).to_degrees();
    assert!(
        (tilt - heel).abs() < 0.5,
        "the waterplane tilts by the heel: {tilt} vs {heel}"
    );
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

#[test]
fn heel_case_and_unreachable_lcg_report_accurately() {
    let hull = example("demo-hull.fef");
    let case = example("demo-loadcase.json");

    // Standalone loading condition without --condition works cleanly.
    let text = stdout(&["heel", &hull, "--case", &case, "--level", "1"]);
    assert!(text.contains("settles"), "{text}");

    // Named --condition with a standalone condition file is rejected.
    let out = run(&[
        "heel",
        &hull,
        "--case",
        &case,
        "--condition",
        "Loaded",
        "--level",
        "1",
    ]);
    assert!(!out.status.success());
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("--condition"), "{err}");

    // Unreachable LCG reports missing free-trim solution at requested heel, not "does not float".
    let text = stdout(&[
        "heel", &hull, "--kg", "2.2", "--lcg", "100", "--heel", "20", "--level", "1",
    ]);
    assert!(
        text.contains("no free-trim solution found at this heel"),
        "{text}"
    );
    assert!(!text.contains("does not float"), "{text}");
}
