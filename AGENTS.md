# SeaCraft: instructions for agents

SeaCraft is a Rust reimplementation of the FREE!ship hull modeller: subdivision surfaces, the `.fef` format,
panel hydrostatics, stability, lines plans, exports, a desktop GUI, a CLI and an MCP server. Humans: see
`CONTRIBUTING.md`.

## Start here

Work: `ROADMAP.md`, issues labelled `help wanted` or `good first issue`. Design: `docs/architecture.md`. Local
notes are fine in ignored folders.

```console
cargo xtask ci                       # the gate; add --full before a release
cargo run -p seacraft-gui -- --open examples/demo-hull.fef
cargo run --bin seacraft -- hydro examples/demo-hull.fef --kg 2.2
```

## Hard rules (what enforces each)

- Measure on the refined mesh; FREE!ship's subdivision weights are deliberate, so never substitute a textbook's.
  Breaking the rules in `skills/seacraft-hull-numerics/SKILL.md` gives plausible but wrong numbers: demo
  reference numbers in `cargo xtask ci`, `crates/seacraft-core/tests/analytic.rs`.
- A numeric test states where its number came from, and "published" means you read the source:
  `docs/oracles.md`, reviewed in PRs.
- Never call a result verified, validated, approved or compliant without saying against what: `DISCLAIMER.md`,
  reviewed in PRs.
- Licence `GPL-3.0-or-later`; ported code is attributed in `ATTRIBUTION.md` and its module header; dependencies stay
  compatible: `cargo deny check licenses`, `cargo xtask licences --check`.
- No local notes, absolute paths or session logs in the repository: `cargo xtask private`.
- Panels never mutate the model; they return what the user asked for, and `App::draw` applies it. Reviewed in PRs.
- Colours only from `crates/seacraft-gui/src/theme.rs`: `cargo xtask colours`.
- Slow work stays off the window thread, in `Job<T>`, and a stale result is never shown as current. Reviewed in PRs.
- Every user file goes through `write_atomic`. Reviewed in PRs.
- The version is identical in `Cargo.toml`, `Cargo.lock`, the window title and `seacraft --version`: `cargo xtask ci`.
- Run `cargo xtask ci` before every commit. `cargo xtask skills` keeps this file and `skills/` honest.

## Working on X: read the skill first

| Working on | Read |
|---|---|
| A pull request, a version, evidence for a change | `skills/seacraft-contributing/SKILL.md` |
| Licences, a dependency, ported code, wording about results | `skills/seacraft-ground-rules/SKILL.md` |
| Anything that measures a hull: subdivision, hydrostatics, stability, damage, lines plans | `skills/seacraft-hull-numerics/SKILL.md` |
| A CLI command, measurement, export, analysis shade, example hull, panel or MCP tool | `skills/seacraft-commands/SKILL.md` |
| Writing or running tests, a numeric oracle, GUI tests | `skills/seacraft-tests/SKILL.md` |
| Designing, fairing or checking a hull over MCP | `skills/README.md` |
