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
  src/templates.rs          builder patterns: station moulds, profile, transom; skin deduction
  src/flatten.rs            least-stretch flattening of a triangle mesh (ARAP, Liu et al. 2008), strain, folds
  src/panels.rs             plywood panels: found from layers and creases, flattened, measured, drawn, offsets, stock
  src/sheet.rs              a drawing sheet in metres -> tiled/single-page PDF, SVG, DXF (mm)
  src/pdf.rs                dependency-free PDF writer: pages, reusable forms, text, tables
  src/ttf.rs                TrueType reader and subsetter for the PDF font
  src/fonts.rs              the embedded DejaVu fonts and their licence
  src/units.rs              lengths as text: metres, millimetres, feet-inches-eighths, scales
  src/iges.rs               IGES 5.3 B-spline curve (126) + surface (128) writer
  src/threemf.rs            spatial-grid vertex welding + 3MF 1.3 archive writer
  src/analysis.rs           vertex normals, discrete curvature, zebra/draft/curvature
  src/raster.rs             CPU z-buffer rasterizer, 2x SSAA, waterplane depth, PNG
  src/report.rs             design dossier: self-contained HTML5 report (particulars, lines, hydro, SAC, stability, resistance)
  src/export.rs             OBJ (y- or z-up), binary STL, 3MF, PLY, IGES, HTML viewer
  src/resistance.rs         still-water resistance and effective power (Holtrop & Mennen 1984)
  src/michell.rs            Michell thin-ship wave resistance (Michlet-style) for slender monohulls and catamarans
  src/demo.rs               parametric 12 m demo hull, full and half model
  src/examples/             example-hull library: Example enum, build() -> (Surface, Project),
                            net primitives (mod.rs), shared acceptance checks (checks.rs),
                            one file per hull (runabout.rs, wigley.rs, ...)
  src/parametric.rs         parametric hull generator (displacement/planing/yacht) from principal dims
  src/optimize.rs           Nelder–Mead + optimize_hull: reshape Cp/LCB/L-B for least R_T at a speed, displacement held
  src/seakeeping.rs         Lewis sections; 2D heave added mass/damping by Frank close-fit panels (B7a); STF strip-theory heave & pitch RAOs (B7b)
  src/project.rs            <model>.seacraft.json sidecar: loading conditions, compartments, damage cases, windage, report settings
  src/seastate.rs           irregular head seas: Bretschneider/JONSWAP spectra, significant motions, MSI (E2)
  src/io.rs                 write_atomic: temp sibling + sync + rename for every user file
  src/control_auth.rs       control-channel token (0600 file, constant-time compare) + read/write roots (ported, see ATTRIBUTION)
crates/seacraft-cli/
  src/main.rs               clap front end, table/JSON/CSV output
  src/mcp.rs                JSON-RPC 2.0 MCP server over stdio (headless + --connect)
crates/seacraft-gui/
  src/main.rs               eframe bootstrap, `--open FILE`, `--control HOST:PORT`
  src/app.rs                App state, menus, file actions, dock wiring
  src/commands.rs           typed model/analysis operations (transform, optimise, topology,
                            crease, generate, hydro table, trim scan, Bonjean, offsets,
                            unfold) shared by the control channel and the menus
  src/model_menu.rs         Model menu items + disabled reasons, Transform/Optimise dialogs,
                            topology-tool targets (Shift+click pair, picked face), axis lock
  src/control.rs            loopback TCP JSON-lines control channel
  src/camera.rs             ViewKind, Camera, ViewTransform, Ray
  src/render.rs             viewport painter: grid, shading, control net
  src/gpu_hull/             the viewport hull on the GPU: vertex data, view uniforms, offscreen pipelines
  src/theme.rs              colour tokens (the only colour literals), type scale, spacing
  src/picking.rs            screen-space picking, drag plane
  src/history.rs            snapshot undo/redo (64)
  src/messages.rs           typed message log (level, text, time) + Messages window
  src/recent.rs             File > Open recent list, JSON eframe::Storage in the data dir
  src/autosave.rs           periodic autosave (30 s, locked session pair) + startup crash recovery modal
  src/panels.rs             layer dock, hydrostatics dock, design-dock tabs (loading, damage, windage, performance), report options, plot_xy/plot_gz
  examples/orbit_bench.rs   real-window, real-GPU frame times while orbiting
  tests/                    integration tests, headless except those in gpu_hull.rs and gpu_hull_routing.rs that need a GPU adapter (ignored by default)
xtask/                      cargo xtask ci: fmt, clippy, tests, versions, demo numbers, repository checks
docs/oracles.md             formulae, tests and numbers for all numeric test oracles
docs/oracles/heave_2d.py    independent Python oracle (Frank panels + Ursell multipoles) for seakeeping.rs
docs/oracles/heave_2d_lid.py  Frank panels with the irregular-frequency lid vs Ursell (E3)
docs/oracles/irregular_seas.py  spectrum moments, response moments, MSI oracle for seastate.rs (E2)
skills/                     contributor skills and MCP design skills
examples/demo-hull.fef      generated demo model + .seacraft.json (KG 2.2); `seacraft demo -o` writes both
README.md                   user-facing docs + FREE!ship notes
ROADMAP.md, CHANGELOG.md    what comes next, what changed
ATTRIBUTION.md              provenance of ported code
NOTICE, LICENSE, LICENSES/  licence texts and required notices
```

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

Builder output goes one way, from the hull to paper:

```
Hull --templates.rs--> Sheet (paths, labels, notes in metres) --sheet.rs--> pdf.rs / SVG / dxf.rs
offsets.rs TableOfOffsets --pdf::add_table--> PDF
half mesh --panels.rs (flatten.rs)--> FlatPanel --> Sheet --sheet.rs--> PDF / SVG / DXF; PanelOffsets --> CSV / PDF
```

`templates.rs` and `panels.rs` know hulls and nothing about paper; `sheet.rs` knows paper and nothing about
hulls; `flatten.rs` knows triangles and nothing about hulls.

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
- **The viewport hull has two paths.** By default each view emits one paint callback
  (`crates/seacraft-gui/src/gpu_hull/`): the vertex data is built once per mesh next to the refinement and
  uploaded once, and a view renders offscreen only when its camera, cell or mesh changes, then composites the
  texture where the painted hull would be. `--no-gpu-hull`, a failed allocation, and headless tests take the
  painted path, `render::shaded_hull`, a depth-sorted egui mesh. Both shade with the same formula and colours,
  and every overlay stays an egui shape projected by `ViewTransform::project` on either path.

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
- **One thread budget.** `crates/seacraft-core/src/par.rs` allows one worker per core across the process, so
  concurrent calls share the cores rather than multiplying threads.
- **Measure on the refined mesh.** A control-polygon shortcut gives plausible but wrong numbers.
- **No fabricated inputs.** A model loaded without a KG shows GM as `—` or a marked estimate. The demo hull
  states its KG explicitly.
- **Ported code stays self-contained.** Modules from other projects are adapted by hand into the core rather
  than pulled in as dependencies, and each is listed in `ATTRIBUTION.md`.
