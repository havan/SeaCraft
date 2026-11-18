---
name: seacraft-commands
description: Use when adding a SeaCraft CLI command, measurement, export format, analysis shade, example hull, GUI panel or MCP tool, or deciding which crate code belongs in.
---

# Adding to SeaCraft

Operations belong below the front ends: the core for anything that measures or writes files, and
`crates/seacraft-gui/src/commands.rs` for typed model and analysis operations the menus and control channel
share. A new capability should reach each interface that makes sense for it: GUI, control channel, CLI and
MCP. See `docs/architecture.md`.

* **New CLI command**: add a variant to `Command` plus an `Args` struct in
  `crates/seacraft-cli/src/main.rs`, then a `cmd_*` function; output tables by
  default and offer `--json`/`--csv` where the data is numeric.
* **New measurement**: build a `hydro::Hull` once, derive the cutting plane,
  reuse `plane_segments` + `chain_paths` + `clipped_loop_moments`, and add the
  field to `Hydrostatics` (it derives `Serialize`). Document the datum.
* **New export format**: add a function in `export.rs` taking
  `render_mesh(surface, level, mode)`; keep it dependency-free so the core stays
  light. The DXF writer (`dxf.rs`) is the model for a format that needs its own
  module: a `DxfShape` variant, a group-code branch in `write_dxf`, and a test
  that reads the output back as group-code pairs.
* **New builder output** (patterns, printable tables): build `sheet::Sheet`s
  and let `sheet.rs` write them (`pdf_tiled`, `pdf_single`, `svg`, `dxf`);
  never write PDF from a hull module. Lengths on paper go through `units.rs`,
  metric first. A table to print goes through `pdf::add_table`. A command that
  judges its output (`seacraft panels`) writes nothing when the judgement fails
  unless the user overrides it, and says which flag does.
* **New analysis shade**: extend `analysis::AnalysisMode`, add its colour in
  `analysis_color`, then give it a default in both front ends (`RenderModeArg`
  in the CLI, the View menu and `render::frame_key` in the GUI — the key must
  include the mode's parameters or the cached texture will not refresh).
* **New example hull**: follow the "Adding a hull" steps in
  `crates/seacraft-core/src/examples/mod.rs` (one file with a frozen station
  table and its acceptance tests through `examples::checks`, one line in
  `Example::builder`, then `seacraft example --all -o examples/`). `build`
  runs no solver; calibrate once, freeze the numbers. On a half model the
  centreline boundary (keel, stem, deck centreline) is pinned by the kernel:
  it stays the control polyline, so give it enough points.
* **New GUI panel**: add a function in `crates/seacraft-gui/src/panels.rs`
  taking `&mut egui::Ui` plus the state it needs, return what the user asked
  for, and let `App::draw` apply it after the closure (never mutate the model
  from inside a panel closure). Headless tests drive frames through
  `egui::Context::run_ui`; the docks settle on the second frame, so draw two
  before reading `App::viewport_rect`.

* **Control channel (`point.move`, `points.move`, `points.select`, `points.select_line`, `ui.set`) and MCP point moves (`move_point`, `move_points`)**:
  `point.move` / `move_point` move one control point to `xyz` (or by `delta` on `point.move`), and `points.move` /
  `move_points` shift a non-empty `points` list by `delta`. Both accept optional `"connected"` (one of the built-in
  names `"Blend both ways"`, `"Blend lengthwise"`, `"Blend around station"`, `"Nudge"`, `"Move station"`,
  `"Beam only"`, `"Height only"`, or an inline preset object `{name?, reach, falloff, spread, follow_axes?,
  stop_at_creases?}`, `name` defaulting to `"custom"`) and optional `"reach_m"` (a positive finite number in metres overriding the preset's reach);
  agents never read the user's `connected-edit.json` or the window's connected switch, and every point-move reply
  includes `"moved": [{point, from, to}, ...]`. In the window, `point.move` and `points.move` are refused while
  the user is dragging a point (the drag would overwrite them); retry after the release. A preset only says how
  points move: nothing about the result is checked or made fair by it. `points.select` takes `{points: [..]}` (empty clears) and replaces the selection with
  the last entry active; `points.select_line` takes `{kind: "station" | "lengthwise"}` and adds the active point's
  line(s) while keeping it active. `ui.set` toggles viewport display switches (`shaded`, `wireframe`, `control_net`,
  `points`, `waterplane`, `volumes`, `windage`, `grid`, `orientation`, `station_focus`, `curves`, `combs`), opens
  (`true`) or closes (`false`) the Wave pattern view in the Analysis drawer with `waves` (it computes in the
  background while open; `waves.compute` returns the numbers), sets
  `hidden_net` (`"dimmed"`, `"hidden"`, `"same"`) and the window's own connected editing (`connected`,
  `connected_preset`, `connected_reach_m`), which changes what a person's drag does and nothing about an agent's
  moves.

New MCP tools go in `tool_definitions()` in `crates/seacraft-cli/src/mcp.rs`. If a skill describes the area,
update it: `skills_name_only_real_tools` checks that each tool it names exists.
