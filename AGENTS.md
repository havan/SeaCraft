---
summary: >
  SeaCraft is a Rust reimplementation of the open-source FREE!ship hull
  modeller: a subdivision-surface geometry kernel, the .fef exchange format,
  panel-based hydrostatics, stability cross curves, a linesplan generator and
  mesh export. Occasionally refines to the same shape as FREE!ship, so the
  weighting rules in subdiv.rs are load-bearing and must not be "tidied".
section_index:
  - "What this is": overview and status
  - "Layout": where every module lives
  - "Build and test": the only commands you need
  - "Domain conventions": axes, units, signs, datum
  - "Invariants": rules that silently corrupt results if broken
  - "Pitfalls already hit": bugs that were fixed; do not reintroduce
  - "Extending": recipes for a new CLI command / measurement
  - "Licensing and provenance": GPL-2.0-or-later
agent_instructions: >
  Read "Invariants" and "Pitfalls already hit" before touching geometry or
  hydrostatics. Measure on the refined mesh, never on the control net. After
  any change to subdiv.rs, hydro.rs or linesplan.rs run `cargo test` and the
  smoke sequence in "Build and test". Never reorder or re-derive the
  subdivision weights from a textbook: FREE!ship's averaging pass is a
  deliberate variant, and matching it is the point of the project.
---

# AGENTS.md

## What this is

SeaCraft reproduces the modelling and measurement pipeline of
[FREE!ship](https://sourceforge.net/projects/freeship/) (GPLv2, Delphi) in
Rust. It reads and writes FREE!ship's text `.fef` exchange format, refines the
control net with the same hybrid Loop / Catmull-Clark scheme, and measures the
submerged hull.

**Status.** Library, CLI, GUI, tests, `.fef` round-trip, hydrostatics,
stability, linesplan SVG and mesh export are implemented and verified. The
desktop modeller (`seacraft-gui`) has four linked viewports, control-point
dragging with undo, a layer dock, a live hydrostatics dock with GZ cross
curves, and file/export actions.

The workspace is a **local-only** git repository: no remotes, no push.

## Layout

```
Cargo.toml                  workspace: core + cli + gui, one shared version
crates/seacraft-core/
  src/lib.rs                public API and module map
  src/math.rs               Vec3, Plane, 2D polygon moments (area/mx/my/ix/iy)
  src/mesh.rs               Surface = control points/edges/faces/layers, extents,
                            edge incidence, boundary/crease initialisation
  src/subdiv.rs             refinement kernel: refine_once, average, mirroring
  src/fef.rs                .fef parse/write (format documented in file header)
  src/hydro.rs              Hull, waterplane, clipping, integration, sections,
                            waterline chaining, coefficients
  src/stability.rs          draft/trim solving + KN/GZ cross curves + IMO A.749
  src/weather.rs            IMO 2008 IS Code Part A §2.3 severe wind and rolling (weather) criterion
  src/loading.rs            loading conditions: weights, tanks (box/hull-clipped), FSM/FSC, equilibrium + IMO
  src/damage.rs             deterministic damage stability: lost buoyancy, equilibrium draft/trim/heel, residual GZ & SOLAS criteria
  src/linesplan.rs          stations/buttocks/waterlines -> SVG/DXF/IGES + combs
  src/dxf.rs                AutoCAD 2000 (AC1015) writer (ported, see ATTRIBUTION)
  src/iges.rs               IGES 5.3 B-spline curve (126) + surface (128) writer
  src/threemf.rs            spatial-grid vertex welding + 3MF 1.3 archive writer
  src/analysis.rs           vertex normals, discrete curvature, zebra/draft/curvature
  src/raster.rs             CPU z-buffer rasterizer, 2x SSAA, waterplane depth, PNG
  src/report.rs             design dossier: self-contained HTML5 report (particulars, lines, hydro, SAC, stability, resistance)
  src/export.rs             OBJ (y- or z-up), binary STL, 3MF, PLY, IGES, HTML viewer
  src/resistance.rs         still-water resistance and effective power (Holtrop & Mennen 1984)
  src/michell.rs            Michell thin-ship wave resistance (Michlet-style) for slender monohulls and catamarans
  src/demo.rs               parametric 12 m demo hull, full and half model
  src/parametric.rs         parametric hull generator (displacement/planing/yacht) from principal dims
  src/optimize.rs           Nelder–Mead + optimize_hull: reshape Cp/LCB/L-B for least R_T at a speed, displacement held
  src/control_auth.rs       control-channel token (0600 file, constant-time compare) + read/write roots (ported, see ATTRIBUTION)
crates/seacraft-cli/
  src/main.rs               clap front end, table/JSON/CSV output
  src/mcp.rs                JSON-RPC 2.0 MCP server over stdio (headless + --connect)
crates/seacraft-gui/
  src/main.rs               eframe bootstrap, `--open FILE`, `--control HOST:PORT`
  src/app.rs                App state, menus, file actions, dock wiring
  src/control.rs            loopback TCP JSON-lines control channel
  src/camera.rs             ViewKind, Camera, ViewTransform, Ray
  src/render.rs             viewport painter: grid, shading, control net
  src/picking.rs            screen-space picking, drag plane
  src/history.rs            snapshot undo/redo (64)
  src/panels.rs             layer dock, hydrostatics dock, GZ plot
  tests/                    drag, files, hydro, smoke (all headless)
xtask/                      cargo xtask ci: clippy, tests, version check, demo numbers
docs/oracles.md             formulae, tests and numbers for all numeric test oracles
skills/                     agent skills for the MCP server (CI-checked tool names)
examples/demo-hull.fef      generated demo model, safe to regenerate
README.md                   user-facing docs + FREE!ship investigation notes
ATTRIBUTION.md              provenance of code ported from ../resource/*
```

## Build and test

```console
cargo build --release
cargo xtask ci   # the full gate: clippy, tests, versions, demo numbers, no remotes
cargo test                       # core + gui unit + 5 integration suites
cargo clippy --all-targets       # must stay silent
cargo run --release --bin seacraft -- demo -o /tmp/demo.fef
cargo run --release --bin seacraft -- hydro /tmp/demo.fef --kg 2.2 --stations 11
cargo run --release --bin seacraft -- linesplan /tmp/demo.fef -o /tmp/lines.svg
cargo run --release --bin seacraft -- linesplan /tmp/demo.fef -o /tmp/lines.dxf
cargo run --release --bin seacraft -- render /tmp/demo.fef -o /tmp/zebra.png --mode zebra
cargo run --release --bin seacraft -- export /tmp/demo.fef -o /tmp/hull.html --level 2
cargo run -p seacraft-gui -- --open examples/demo-hull.fef
```

The GUI runs headlessly for tests: `egui::Context::run_ui` with synthetic
pointer and key events (`crates/seacraft-gui/tests/`). A real check needs the
display: `DISPLAY=:1 import -window root /tmp/gui.png` while the app runs.

Reference numbers for the demo hull at T = 0.62 m (level 3, Loop mode):
volume ≈ 11.85 m³, Cb ≈ 0.504, Cwp ≈ 0.824, KB ≈ 0.392 m, KM ≈ 2.63 m,
midship area ≈ 1.47 m². A change that moves these by more than ~1 % is a
regression, not an improvement.

## Domain conventions

* **Axes**: `x` forward, `y` to starboard, `z` up; keel at low `z`.
* **Units**: metres and kg inside the core. `.fef` carries the project's
  units flag; imperial is only a project setting at present.
* **Layers** may be `symmetric` (only `y >= 0` modelled, mirrored on evaluation)
  and `use_in_hydrostatics` (filter for measurement, without affecting display).
* **Heel** is positive to starboard. **Trim** is a length in metres, positive
  when the bow is higher than the stern.
* **Datum**: `KB`, `BM`, `KM`, `KG` are above the keel (lowest hydrostatic
  point). `LCB`, `LCF` are in model coordinates.
* **Waterplane** is built in `hydro::waterplane_for` from draft, trim and heel;
  the normal points up, and points with `side <= 0` are submerged.

## Invariants

Breaking any of these produces plausible-looking but wrong numbers.

1. **Measure on the refined mesh.** All hydrostatics, sections and linesplan
   curves use `subdiv::evaluate` output, never the control polygon.
2. **Outward normals.** The volume integral is a divergence integral over the
   hull panels; reversed winding flips the sign or corrupts the volume.
3. **Closed below the waterline.** Edges must have two adjacent faces except
   deliberate model boundaries. A submerged boundary edge is a leak. A
   symmetric half model also needs a transom face, otherwise its waterplane
   never closes.
4. **Integration origin lives in the waterplane.** `integrate` projects the
   world origin onto the plane so the waterplane lid contributes zero; the
   centroid is shifted back by that origin afterwards.
5. **Subdivision weights are FREE!ship's, not a textbook's.** Edge points are
   midpoints, face points are centroids, and `average` applies
   `P' = P + a (C - P)` with `a = 4/n` for all-quad, the Loop beta formula for
   all-triangle, and `12/(3Nq + 2Nt)` mixed, then crease `0.5P + 0.25(N1+N2)`
   for crease vertices with **exactly two** crease neighbours (others are
   pinned, so the weights can never sum past 1).
   Do not substitute standard Catmull-Clark.

## Pitfalls already hit

These were real failures in this codebase; the fixes are load-bearing.

* **Curves fragment into many short paths.** Two causes, both fixed in
  `hydro::chain_paths`: walking an open chain from an edge in its middle (walk
  forward, then backward from the other endpoint and concatenate), and
  duplicate segments from quad diagonals (dedupe by unordered endpoint pair).
* **Closure detection.** A closed ring returns to the start edge's *other*
  vertex, not to the first vertex walked. Comparing against the wrong one makes
  every waterline look open.
* **Endpoint snapping.** the same crossing computed from the two adjacent
  triangles differs in the last bits; grid rounding splits it at cell
  boundaries. Snap by proximity with a neighbour search.
* **Open station cuts need closing.** A hull open at the deck gives an open
  station curve; close it along the waterplane before computing area. The
  centreline (lateral) cut needs the same.
* **Mirroring reads `face_layer` per face.** Clear it only after
  `mirror_symmetric`, or it panics with an index-out-of-bounds.
* **Crease junctions must not average.** A vertex with three or more incident
  crease edges (chine + transom + sheer) summed `0.5 + 0.25k > 1` and drifted
  outward by 25 % per level. The rule is `nb.len() == 2`; write the weights as
  a single expression so the sum is visible.
* **A heeled hull is free to trim.** Holding `trim` while heeling leaves `LCB`
  off `LCG`, and the usual hull (wide transom, fine bow) then reports a `KN`/
  `GZ` that is too high — the error grows with heel and is invisible upright.
  `StabilityCondition::lcg_x: Some(x)` runs the two-unknown solve; `None` keeps
  the old fixed-trim behaviour for speed and compatibility. The check is the
  analytic box: at 2 m draft, asking a 10 m box for `LCB` 4.0 m must trim 2.4 m
  bow-up (see `tests/analytic.rs`).
* **`PolygonMoments::from_polygon` returns signed values.** Areas/moments may
  come back negative depending on winding; callers take `abs()` where the
  orientation is not guaranteed.

## Extending

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
* **New test**: prefer analytic or invariant checks (box displacement, barge
  waterplane, GZ ≈ 0 at zero heel, creased box volume preserved, mirror
  symmetry, `.fef` round-trip) over golden files.
* **New GUI panel**: add a function in `crates/seacraft-gui/src/panels.rs`
  taking `&mut egui::Ui` plus the state it needs, return what the user asked
  for, and let `App::draw` apply it after the closure (never mutate the model
  from inside a panel closure). Headless tests drive frames through
  `egui::Context::run_ui`; the docks settle on the second frame, so draw two
  before reading `App::viewport_rect`.
* **Ported code**: keep the source file, project and licence in the module
  header *and* in `ATTRIBUTION.md`. Adapt the vector type to
  `seacraft_core::math::Vec3`; never add `solidcraft-*`, `fieldcraft-*` or
  `solidcore` as Cargo dependencies (millimetres, `truck`, `egui 0.36`).

## Versioning and history

* Semver with a prerelease counter, shared through
  `[workspace.package] version`. History: `0.1.0-alpha.1` at import time,
  `alpha.2` … `alpha.10` per completed GUI task, `0.1.0-beta.1` when the
  acceptance checklist passed, then `beta.N` per completed task. `0.1.0` only
  when a user can open, edit and measure a hull without a known blocking
  defect.
* The version must be identical in `Cargo.toml`, `Cargo.lock`, the window title
  and `seacraft --version`.
* **Local-only Git.** `git remote -v` must stay empty: no push, no publish, no
  origin. One linear commit per task.
* `.hermes/NEXT.md` is the session handoff: current version and commit, what was
  just done and how it was verified, the next task's first three actions, open
  decisions. A fresh session starts there, not from chat history.

## Licensing and provenance

SeaCraft is GPL-2.0-or-later, matching FREE!ship. FREE!ship is Copyright its
authors; this is an independent reimplementation written from documented
behaviour and the GPL source, not a translation. The modelling ideas
(subdivision scheme, `.fef` layout, panel hydrostatics) come from FREE!ship;
keep attribution intact when adding derived documentation.
