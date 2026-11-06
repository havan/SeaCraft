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

New MCP tools go in `tool_definitions()` in `crates/seacraft-cli/src/mcp.rs`. If a skill describes the area,
update it: `skills_name_only_real_tools` checks that each tool it names exists.
