---
name: seacraft-tests
description: Use when writing or running SeaCraft tests, adding a numeric oracle, or testing the GUI headlessly.
---

# Tests

`cargo xtask ci` is the gate. Run it before every commit. It runs fmt, clippy with warnings denied, the tests, the
version check, demo reference numbers and the repository checks. The dev profile is optimised, and it keeps debug
assertions and overflow checks on. `cargo xtask ci --full` also runs the release build's tests: use it before a
release. CI runs it weekly.

* **New test**: prefer analytic or invariant checks (box displacement, barge
  waterplane, GZ ≈ 0 at zero heel, creased box volume preserved, mirror
  symmetry, `.fef` round-trip) over golden files.

## Smoke sequence

```console
cargo build --release
cargo xtask ci   # the gate: fmt, clippy, tests, versions, demo numbers, repository checks
cargo test
cargo clippy --all-targets       # must stay silent
cargo run --release --bin seacraft -- demo -o /tmp/demo.fef
cargo run --release --bin seacraft -- hydro /tmp/demo.fef --kg 2.2 --stations 11
cargo run --release --bin seacraft -- linesplan /tmp/demo.fef -o /tmp/lines.svg
cargo run --release --bin seacraft -- linesplan /tmp/demo.fef -o /tmp/lines.dxf
cargo run --release --bin seacraft -- render /tmp/demo.fef -o /tmp/zebra.png --mode zebra
cargo run --release --bin seacraft -- export /tmp/demo.fef -o /tmp/hull.html --level 2
cargo run -p seacraft-gui -- --open examples/demo-hull.fef
```

## Numeric oracles

A number that a test holds the code to is an oracle. Record it in `docs/oracles.md`: formula, test name,
values, and where it came from.

- Prefer a closed form. Otherwise compute it a second, different way, in `docs/oracles/` if it is a script.
- Say "not published" unless you read the published source. Agreement to 0.1 % with "the literature" is a
  warning sign: check where the number came from.
- A frozen output of SeaCraft catches changes but proves nothing about being right. Label it so.
- Check that a new test bites: undo the fix and watch it fail.

## GUI

The GUI runs headlessly for tests: `egui::Context::run_ui` with synthetic
pointer and key events (`crates/seacraft-gui/tests/`). A real check needs the
display: `DISPLAY=:1 import -window root /tmp/gui.png` while the app runs.

Scroll areas skip what is out of view, so make the test's screen tall when asserting on dock text.
