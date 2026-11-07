# Changelog

Versions follow [semantic versioning](https://semver.org/), with `0.x` meaning that anything may change.

## Unreleased

- GUI: the viewport hull is drawn on the GPU, so orbiting a large hull at a high subdivision level stays
  smooth. On one Linux machine with an RTX 3070 Ti, orbiting `container-230m` at level 4 went from about
  130-160 ms to about 8 ms per frame, and level 5, where the old drawing panics, takes about 10 ms. Hull edges
  are smoother, faces are depth-tested instead of sorted, a face partly behind the eye is clipped instead of
  dropped, and wireframe lines are 1 pixel wide. Each view also keeps its waterline as one mesh, with the same
  pixels. This has not been run on Windows, macOS or an integrated GPU. `seacraft-gui --no-gpu-hull` draws the
  hull the old way, and the app falls back to it, with a message in the log, when the GPU refuses the mesh or
  a view's texture. No change to any computed number.
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
