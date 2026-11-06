//! Text output that shows stability criteria carries the not-validated note;
//! machine-readable output stays clean so scripts keep working.

use std::process::Command;

use seacraft_core::NOT_VALIDATED_NOTE;

fn example(name: &str) -> String {
    format!("{}/../../examples/{name}", env!("CARGO_MANIFEST_DIR"))
}

fn stdout(args: &[&str]) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_seacraft"))
        .args(args)
        .output()
        .expect("run seacraft");
    assert!(
        out.status.success(),
        "{args:?}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("utf-8")
}

#[test]
fn stability_text_carries_the_note_and_csv_does_not() {
    let hull = example("demo-hull.fef");
    let csv = std::env::temp_dir().join(format!("seacraft-note-{}.csv", std::process::id()));
    let csv_arg = csv.to_string_lossy().into_owned();
    let text = stdout(&["stability", &hull, "--kg", "2.2"]);
    assert!(text.contains(NOT_VALIDATED_NOTE), "{text}");
    stdout(&["stability", &hull, "--kg", "2.2", "--csv", &csv_arg]);
    let written = std::fs::read_to_string(&csv).expect("csv written");
    let _ = std::fs::remove_file(&csv);
    assert!(!written.contains("Not validated"), "{written}");
}

#[test]
fn loadcase_text_carries_the_note_and_json_does_not() {
    let (hull, case) = (example("demo-hull.fef"), example("demo-loadcase.json"));
    assert!(stdout(&["loadcase", &hull, &case]).contains(NOT_VALIDATED_NOTE));
    let json = stdout(&["loadcase", &hull, &case, "--json"]);
    assert!(!json.contains("Not validated"));
    serde_json::from_str::<serde_json::Value>(&json).expect("clean JSON");
}

#[test]
fn damage_text_carries_the_note_and_json_does_not() {
    let (hull, case) = (example("demo-hull.fef"), example("demo-damage.json"));
    assert!(stdout(&["damage", &hull, &case]).contains(NOT_VALIDATED_NOTE));
    let json = stdout(&["damage", &hull, &case, "--json"]);
    assert!(!json.contains("Not validated"));
    serde_json::from_str::<serde_json::Value>(&json).expect("clean JSON");
}
