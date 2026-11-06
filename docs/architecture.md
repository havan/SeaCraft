# Architecture

How SeaCraft is put together today. `README.md` is for users, `ROADMAP.md` covers what comes next, and
the skills in `skills/` carry the detailed rules for each kind of work.

## Crates

```
seacraft-core     geometry, measurement, file formats; no GUI, few dependencies
  ^       ^
  |       |
seacraft-cli      clap front end and the MCP server (crates/seacraft-cli/src/mcp.rs)
seacraft-gui      egui desktop modeller and the control channel (crates/seacraft-gui/src/control.rs)

xtask             cargo xtask ci, and the repository checks
```

The MCP server runs headless on the core, or connects to a running GUI over the control channel
(`seacraft mcp --connect`). The CLI and GUI do not depend on each other.

## From control net to numbers

```
.fef file --fef.rs--> Surface (mesh.rs) --subdiv.rs--> refined mesh --hydro.rs Hull--> measurements
                                                                         |
       stability.rs, loading.rs, damage.rs, weather.rs, resistance.rs, michell.rs, planing.rs, seakeeping.rs
```

- `crates/seacraft-core/src/mesh.rs`: `Surface` is the control net, with layers, edges and faces.
- `crates/seacraft-core/src/subdiv.rs`: FREE!ship's refinement. Its weights are deliberate.
- `crates/seacraft-core/src/hydro.rs`: `Hull` holds the refined triangles and closes open boundaries with
  lids. Everything measured goes through it, never the control net.
- `crates/seacraft-core/src/project.rs`: `<model>.seacraft.json` holds what `.fef` has no place for, such as
  loading conditions, compartments, damage cases, windage and report settings.
- `docs/oracles.md` records each test's reference number and where it came from.

## Commands

Operations live below the front ends, so the menus, control channel, CLI and MCP tools share them.

- `crates/seacraft-gui/src/commands.rs` holds typed model and analysis operations shared by the menus
  (`crates/seacraft-gui/src/model_menu.rs`) and the control channel.
- `crates/seacraft-cli/src/main.rs` and `crates/seacraft-cli/src/mcp.rs` call the core directly.

A new capability should reach each interface that makes sense for it. See `skills/seacraft-commands/SKILL.md`.

## GUI rules

- **Layout.** Menu bar and toolbar on top, status bar below, a left dock for layers and the inspector, a
  right analysis dock, and four linked viewports in the middle (`crates/seacraft-gui/src/app.rs`).
- **Panels never mutate the model.** A function in `crates/seacraft-gui/src/panels.rs` takes `&mut egui::Ui`
  plus the state it needs and returns what the user asked for. `App::draw` applies it after the closure.
- **Colours come from `crates/seacraft-gui/src/theme.rs`.** `cargo xtask colours` enforces it. A verdict is
  never colour alone.
- **Slow work runs off the window thread.** `Job<T>` in `crates/seacraft-gui/src/jobs.rs` runs it on a
  background thread, stamped with the revision it started from, and reports a panic as a failure.
- **One staleness rule.** A result carries the revision it was computed for, and a stale one is marked as
  such. A green `PASS` is never shown on stale data.
- **Undo** is snapshots (`crates/seacraft-gui/src/history.rs`), up to 64, shared by model and project edits.

## Files on disk

Every user file goes through `write_atomic` in `crates/seacraft-core/src/io.rs` (temporary sibling, sync,
rename), so a crash cannot truncate it. Autosave keeps a locked session pair in the state directory
(`crates/seacraft-gui/src/autosave.rs`), with a recovery prompt at the next start.

## Control channel

`seacraft-gui --control HOST:PORT` opens a loopback JSON-lines channel, executed on the next frame.
Authentication is on by default: a random token in a file only the user can read, compared in constant time
(`crates/seacraft-core/src/control_auth.rs`), with optional read and write roots for file access.

## Decisions worth keeping

- **Determinism.** Parallel cross curves must equal the serial result bit for bit, not within a tolerance.
- **Measure on the refined mesh.** A control-polygon shortcut gives plausible but wrong numbers.
- **No fabricated inputs.** A model loaded without a KG shows GM as `—` or a marked estimate. The demo hull
  states its KG explicitly.
- **Ported code stays self-contained.** Modules from other projects are adapted by hand into the core rather
  than pulled in as dependencies, and each is listed in `ATTRIBUTION.md`.
