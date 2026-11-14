# Changelog

Versions follow [semantic versioning](https://semver.org/), with `0.x` meaning that anything may change.

## Unreleased

- Builder patterns: `seacraft templates hull.fef -o moulds.pdf --skin-mm 6` cuts full-size station moulds from the
  refined hull, moved in by the skin thickness along the surface normal, with the mould face that goes on the
  station line, the largest bevel, waterlines, the design waterline and optional strongback legs or half moulds.
  `--profile` adds the backbone and `--transom X,RAKE` a raked transom drawn true in its own plane. A PDF is tiled
  over A4, A3, Letter or any paper size with join marks, a map page and two 100 mm check rules, or put one pattern
  on each page (`--single-page`); `.svg` and `.dxf` (millimetres, layered) give one drawing. The patterns are
  checked against closed-form shapes (cylinder, cone, box, raked plane) and against poppler's rendering of the
  PDF (100 mm drew as 1000 px at 254 dpi). Nobody has printed and measured a set yet, and the PDFs have not been
  opened on Windows or macOS. [`docs/building.md`](docs/building.md) is the builder's guide.
- Plywood panels: `seacraft panels hull.fef -o panels.pdf` lays each panel of a hard-chine hull flat for
  stitch-and-glue. Panels are found on the refined half model from its layers and creases; a symmetric panel that
  meets its mirror flat across the centreline is one piece, any other is cut twice, one turned over. Each pattern
  has its cut line, station lines, a tick wherever a station crosses an edge, what each edge is stitched to, and
  the measures: seam drift (flat against 3D length, from either end), stretch, the outline shift to the next
  subdivision level and the old edge-joining gap. A panel that drifts over 2 mm or stretches over 0.1 % RMS does
  not lie flat; then nothing is written unless `--allow-undevelopable` is given, and its pattern is headed so. The
  limits are provisional project judgement (`--max-seam-drift-mm`, `--max-stretch-pct`). `--offsets-csv` and
  `--offsets-pdf` give offsets from a straight baseline, and every run counts plywood sheets end to end
  (`--stock-mm`, `--scarf-mm`): an upper bound, no nesting. The flattening is the as-rigid-as-possible method of
  Liu et al. (2008); it is checked against closed-form shapes (cylinder and cone on their rulings, a folded strip,
  planar panels, a spherical cap by Gauss–Bonnet). It is geometry, not a bending test of plywood, it allows no
  plywood thickness, and no boat has been built from these patterns yet. Core and CLI only: the GUI, control
  channel and MCP follow.
- `seacraft offsets --pdf offsets.pdf [--imperial]` prints the table of offsets in millimetres above the keel, and
  with `--imperial` the same tables in feet-inches-eighths.
- `seacraft offsets --json` keeps its standard output to the JSON table when it also writes `--pdf`, `--html` or
  `--csv`: the "wrote" lines now go to standard error.
- GUI: File ▸ Builder templates… previews the page count on a background job and exports the patterns; File ▸
  Export ▸ Offsets table PDF…. The control channel gains `build.templates`, `model.export` format `offsets_pdf`
  (with `stations`, `waterline_steps`, `buttock_steps`, `level`, `imperial`) and `ui.set` `templates`.
- MCP: `export_templates` and `export_offsets_pdf`, headless and connected, and a `seacraft-boatbuilding` skill
  that has the agent ask for the skin thickness and relay every warning.
- The DejaVu fonts moved into the core crate; PDFs embed a subset of DejaVu Sans. No change to the GUI's text.
- Fix: cutting a hull into sections lost the last point of the first closed loop and could report a later loop as
  open. This changes computed numbers where a section has closed loops. On the example hulls: the Wigley hull's
  midship section area and midship coefficient rise 9.5 % (Cm 0.6066 to 0.6639; the Wigley form's own value is
  2/3) and its prismatic coefficient falls 8.6 % (0.7286 to 0.6656, also 2/3 for the form); the demo hull's
  section area at its transom falls 1.0 %, so `seacraft sac` gives Cp 0.7097 instead of 0.7029. Every other
  quantity `hydro` and `sac` report for the seven example hulls moved by less than 0.01 %, and the demo-hull
  reference numbers checked in CI did not change.

## 0.3.0 (2026-11-13)

0.3.0-beta.1 with one fix:

- GUI: the wide Analysis dock can be dragged to any width, as the narrow one could. An Analysis menu view
  such as Offsets still opens it wide, but closing the view now gives back the narrow dock it had, instead of
  leaving it wide and fixed at 55 % of the window.

## 0.3.0-beta.1 (2026-11-07)

- GUI: the viewport hull is drawn on the GPU, so orbiting a large hull at a high subdivision level stays
  smooth. On one Linux machine with an RTX 3070 Ti, orbiting `container-230m` at level 4 went from about
  130-160 ms to about 8 ms per frame, and level 5 takes about 10 ms (`--no-gpu-hull` panics there in this build). Hull
  edges are smoother, faces are depth-tested instead of sorted, a face partly behind the eye is clipped instead of
  dropped, and wireframe lines are 1 pixel wide. Each view also keeps its waterline as one mesh, with the same
  pixels. This has not been run on Windows, macOS or an integrated GPU. `seacraft-gui --no-gpu-hull` draws the
  hull the old way, and the app falls back to it, with a message in the log, when the GPU refuses the mesh or
  a view's texture. On `container-230m` at level 5 that old drawing asks for a 403 MiB buffer and panics, so
  there it is no way out. No change to any computed number.
- GUI: the hull is drawn solid, so it looks darker and the grid no longer shows through it. No change to any
  computed number.
- A website at <https://havan.github.io/SeaCraft/>, built from `site/`. Its numbers and drawings come from
  running `seacraft` on the example hulls each time it is published. No change to any computed number.
- GUI: text is now set in DejaVu Sans and DejaVu Sans Mono, which replace egui's default fonts. Body and
  button text go from 12.5 to 12 pt to keep the same apparent size. The remove, layer visibility and
  wide dock buttons now use symbols those fonts contain, so they draw on every machine.
- GUI: Help > About SeaCraft shows the copyright, the warranty and licence notice, that results are not
  validated, a link to the source, and the licence texts. `--help` on both programs carries the notice too.
- `cargo xtask dist --target TRIPLE` builds the folder a release archive is made from, with the full
  licence text of every crate compiled in.
- Release builds for Linux (archive, AppImage, Flatpak bundle), Windows (zip) and macOS (disk images for Apple
  and Intel chips), made by a workflow that creates a draft release from a version tag. The window has a
  SeaCraft icon, and on Windows a window that cannot open says why in a message box.
- GUI: orbiting and hovering are faster with the waterplane on: the waterline is cut once per mesh and plane
  instead of once per view per frame. No change to any computed number.

## 0.2.1 (2026-11-07)

First public release. No change to any computed number.

- Licence is now `GPL-3.0-or-later`. Tags before this one say `GPL-2.0-or-later` but contain the same
  Apache-2.0-only code, so read them as `GPL-3.0-or-later` too.
- Corrected attribution, and added `NOTICE`, `DISCLAIMER.md` and a third-party licence list.
- Design reports, CLI text with stability criteria, and the GUI's criteria panels now say results are not
  validated. JSON and CSV output are unchanged.
- Added contributor documentation and CI.
- Concurrent solver calls share one thread budget instead of each starting a thread per core.
- The test gate takes about a quarter of the time: one optimised build that keeps debug checks.

## 0.2.0 (2026-11-06)

- GUI: one analysis dock, start screen, New design dialog, Model and Analysis menus, background jobs with
  stale results marked, theme tokens, layer management, recent files, message log, autosave and crash recovery.
- Example hull library: Wigley, runabout, trawler, piyade kayık, yacht and container ship.
- Stability past 90° of heel; hulls open at the deck are closed with a watertight lid; crease junctions pinned.
- Body plan convention: looking forward from aft, starboard on the right.

## 0.1.0 (2026-10-24)

First complete version: `.fef` round-trip, FREE!ship subdivision, hydrostatics, stability, IMO criteria,
loading conditions, damage stability, resistance, seakeeping, lines plans, exports, design report, command
line, control channel, MCP server and the desktop modeller.
