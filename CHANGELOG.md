# Changelog

Versions follow [semantic versioning](https://semver.org/), with `0.x` meaning that anything may change.

## Unreleased

- Michell resistance (CLI, MCP, the Performance tab and the control channel) now warns when the hull has a wetted
  transom larger than 5 % of its largest section, since Michell's integral assumes the hull closes at the stern.
  The demo hull and the runabout example are flagged at their design drafts. The four front ends now share one
  set of Michell warnings (`michell::michell_warnings`); the L/B and Fn messages are unchanged.
- `michell::wigley_grid` integrated the Wigley hull's wetted surface with a wrong slope and gave 15.14 m² for
  the 10 m hull instead of 14.88 m², so Cw on that grid read 1.8 % low (R_w was right). Hull-file Michell results
  take S from the hydrostatics and are unchanged.

## 0.5.0 (2026-11-17)

Seeing and shaping the control net. All of this is covered by headless tests; the look of it (colours, the
triad, the markers) has been reviewed by eye on Linux only.

- The Profile view now looks from starboard, with the bow still on the right, so the half of a hull that is
  modelled is the half you see. A symmetric hull looks as it did. Each view's title says which way it faces
  ("Profile · from starboard", "Plan · half-breadths, starboard up", "Body plan · looking forward"), and an axis
  triad in the corner and edge labels (Aft, Fwd, Port, Stbd) say the same; View ▸ Orientation turns them off.
  `seacraft render --view profile` is unchanged and still looks from port.
- Control net behind the shaded hull is drawn dimmed. View ▸ Hidden net, or the toolbar's choice beside Net and
  Points, says whether it is dimmed, hidden or shown; an analysis mode (Zebra, Curvature, Draft) hides it too, and
  with no solid hull nothing is hidden. A click prefers the point in front; a hidden point can still be picked
  when nothing visible is in reach, except in the hidden mode. The point under the pointer is ringed.
- The station of the control point under the pointer, or of the selected one, is highlighted in all four views.
  In the Body plan the rest of the net is drawn fainter (View ▸ Station focus) and a caption gives the station's
  `x` and point count; the Inspector shows the same. **Alt+Left** and **Alt+Right** step the selection aft and
  forward along the point's lengthwise line. Stations are read from the net's own lines, so a station whose
  points lean is still one station.
- Several control points can be selected: **Shift+click** adds or removes one, **Shift+drag** on empty space
  draws a rectangle that adds the points in it (hidden ones are left out unless the hidden net is drawn as
  normal), Model ▸ Select station and Select lengthwise line add a whole line. Dragging one selected point drags
  them all as one undo step, and with several selected the Inspector moves them by a typed displacement. A plain
  drag on empty space still pans. **Esc** during a drag cancels it and puts the points back.
- **Ctrl+wheel** and a trackpad pinch zoom the views like the wheel; before they did nothing.
- Connected editing: with the **Connected** switch on, a dragged point draws its neighbours along, less the
  further they are along the net. A preset says how: which neighbours follow (lengthwise, round the station, or
  both), how the pull fades, how far it reaches, and which of x, y, z the followers move in. Seven presets are
  built in ("Blend both ways", "Blend lengthwise", "Blend around station", "Nudge", "Move station", "Beam only",
  "Height only"). Hovering shows which points would follow and how strongly; the wheel changes the reach during a
  drag. Holding **Ctrl** (**Cmd** on macOS) flips the switch for as long as it is held, also mid-drag. It is off
  by default and the choice is remembered. Your own presets go in `connected-edit.json` beside
  `seacraft-gui.json` (Model ▸ Create example presets file writes a commented example, Reload presets re-reads
  it); a mistake in that file is reported in the window and leaves the built-in presets working. A preset says
  how points move; it does not check or make anything fair.
- On a half model, a point on the centreline that is moved as part of a group or as a follower stays on the
  centreline, and no point of a symmetric layer goes below it. A centreline point moved on its own behaves as
  before.
- Control channel: `points.move`, `points.select`, `points.select_line`; `point.move` and `points.move` take an
  optional `"connected"` (a built-in preset's name or a preset object) and `"reach_m"`, and report every point
  moved; `ui.set` gains `orientation`, `hidden_net`, `station_focus`, `connected`, `connected_preset` and
  `connected_reach_m`; `ui.inspect` reports them and `selected_points`. MCP: `move_points`, and `"connected"` on
  `move_point` and `move_points`. An agent's move is connected only when the call says so, and never reads the
  user's presets file. While a point is being dragged in the window, an agent's point move is refused with an
  error, so the drag cannot silently undo it.
- Changed: the headless MCP server's `move_point` now holds a point at `y >= 0` when any face it belongs to is
  on a symmetric layer, as the window always did. It used to require all of them, so a point shared between a
  symmetric and an unsymmetric layer could be moved below the centreline headless and not in the window.

## 0.4.0 (2026-11-16)

- Every program says which source it was built from. `seacraft --version`, the window title, About and the MCP
  server show `0.3.0` only for a clean build of the tag `v0.3.0`; any other build names its commit, such as
  `0.3.0+67-ga4b51c2-dirty` after the release or `0.3.1-dev+gabc1234` before it, and reports name it too. About links to the exact commit.
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
- GUI look: the docks, windows and menus take the viewports' slate instead of neutral grey; a field that holds a
  value is sunk and outlined where a button is raised; the selection blue marks only what is selected (the active
  layer, the open tab, the shading mode), and a display switch that is on is drawn raised instead; section headings
  are the brightest text in a dock; body text is 13 px; Create and the unsaved-changes Save are filled primary
  buttons. The analysis dock opens 384 px wide and the layer panel 264 px, and the hydrostatics readout goes two
  columns from a 364 px dock. The Builder templates window opens tall enough to show Export.
- GUI: an Analysis menu view (hydrostatic table, Bonjean curves, offsets, trim scan, plate development) opens in a
  drawer under the viewports instead of taking over and widening the Analysis dock. The drawer's header has the
  five views as tabs, Compute, Copy CSV and ✕; closing it gives the viewports their height back, and the dock
  keeps its tab and its width throughout. F5 recomputes the drawer's view and the dock's tab, each when it is out
  of date, and reaches the drawer with the dock hidden.
- GUI toolbar: the shading mode is one segmented control, and the subdivision level is labelled Precision and
  reads "Level 3". Fit and Maximise left the toolbar: each view has the two as icon buttons in its top right
  corner, where fit frames that view alone (F still frames all four) and maximise acts on that view, active or
  not.
- GUI Analysis dock: the pinned strip is one row of five tiles (name and unit over the value) with the dock's
  width control as an icon at its end, in place of two cramped rows and a "↔ wide" button. The Hydro tab loses
  its Recompute button: hydrostatics already recompute 120 ms after a change, and F5 still forces it. A slider's
  number is drawn as a field.
- GUI menus: File > New holds the three ways to start a model (Design…, From an example…, Demo hull). The View
  menu is grouped under Show, Shading, Panels, Precision and Views, with the dock's tabs indented under the dock.
  Help > Keyboard and mouse lists what the keys and the mouse do.
- GUI viewports: the control net and its creases are drawn dimmed until a control point is selected, so the hull
  is what the eye reads when nothing is being edited.
- GUI layer panel: the five switches are grouped under Shape and Used for; density and thickness are named
  fields that carry their units; the tooltip of "Move picked face to this layer" says what is missing while the
  button is disabled.
- GUI start screen: each recent file shows the folder it is in, on the start card and in File > Open recent, so
  two files of one name can be told apart. With no model the docks are empty: only the status bar and the start
  card say so, where both docks used to repeat it.
- GUI Builder templates: a tiled set of more than 40 pages gets a warning under its page count that names the two
  ways to need fewer, and Export is the window's filled primary button. The Builder templates and Report options
  windows lost their collapse arrow, like every other window.
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
