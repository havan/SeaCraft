# SeaCraft roadmap

Where SeaCraft is and what comes next. Update it when a milestone task lands. Results are not validated;
see [`DISCLAIMER.md`](DISCLAIMER.md).

## Works today

- Hull modelling on a subdivision control net: FREE!ship's hybrid Loop / Catmull-Clark scheme, layers,
  creases, `.fef` read and write, parametric generator, example hulls.
- Desktop modeller: four linked viewports, control-point dragging with undo, split face and insert edge,
  docks, autosave and crash recovery.
- Hydrostatics, cross curves, GZ with free trim, IMO A.749 and IS Code weather criteria.
- Heel under trial weights: the equilibrium heel (or loll, or capsize), free to trim, in the window's Heel and
  righting arm view with B, G and the righting lever drawn live, on the command line (`seacraft heel`) and over MCP.
- Loading conditions with tanks and free-surface correction; deterministic lost-buoyancy damage stability.
- Resistance by Holtrop-Mennen, Michell thin-ship theory and Savitsky; a hull optimiser.
- The far-field thin-ship wave pattern behind a hull or catamaran (PNG, SVG contours, VTK), with the speed
  regime, wash height and catamaran interference: command line, MCP, the window's Wave pattern view, and the
  design report.
- Strip-theory heave and pitch motions, irregular head seas, MSI.
- Lines plans (SVG, DXF, IGES), mesh exports (OBJ, STL, 3MF, PLY, HTML), an HTML design report.
- Full-size building patterns: station moulds with the skin deducted, profile and raked transom, as a PDF
  tiled over ordinary paper, one page a pattern, SVG or DXF; the table of offsets as a PDF.
- A command line, a loopback control channel, and an MCP server for AI agents.

## Next

### M1: Validate the numbers

The most important milestone: most results have never been compared with reality.

- [x] Compare hydrostatics and GZ with a published or independently computed hull: the Wigley hull's published
  equation, by closed forms over drafts and an independent GZ integration to 80° (`docs/oracles.md` §44, §45).
- [x] Compare Holtrop-Mennen and Savitsky with published worked examples: Holtrop & Mennen (1982), Holtrop (1984),
  Molland et al. (2011) (`docs/oracles.md` §42, §43).
- [x] Compare Michell with published Wigley hull data: Kajitani et al. (1983) towing tests and an independent
  computation (`docs/oracles.md` §39).
- [x] Compare seakeeping RAOs with published experiments: strip theory on Wigley I and III in head waves at
  Fn 0.2 to 0.4, coefficients, wave loads and motions each against Journée (1992) towing tests
  (`docs/oracles.md` §46).
- Done when each area has at least one oracle in `docs/oracles.md` with a checkable published source,
  and `DISCLAIMER.md` and the README say exactly what is covered.

M1 is done (2026-11-20): every area above has an oracle with a checkable published source, and `DISCLAIMER.md`
and the README say what each covers. Most results are still not compared with real vessels; see the gap review.

### M2: Viewport performance

The viewport is smooth on small hulls at the default subdivision level, and slow on larger hulls and at higher
levels. Orbiting in a real window, release build, 1600 x 1000, vsync off, RTX 3070 Ti, on a machine also running
other work, median frame time and fps with the waterline cached:

| Hull | Level 3 | Level 4 |
|---|---|---|
| `demo-hull` | 5 ms, about 200 fps | 18 ms, about 55 fps |
| `wigley` | 22 ms, about 45 fps | 80 ms, about 12 fps |
| `container-230m` | 37 ms, about 27 fps | 125-165 ms, 6-8 fps |

`cargo run --release -p seacraft-gui --example orbit_bench -- HULL.fef LEVEL` takes these numbers, and
`crates/seacraft-gui/tests/perf.rs` splits the cost per frame. The CPU is the limit, not the GPU.

The hull is now drawn on the GPU, and each view keeps its waterline as one tessellated mesh. These numbers
describe one machine: Linux, RTX 3070 Ti on Vulkan, release build, 1600 x 1000, vsync off, load average 2 to 12
from other work. No integrated GPU was available. The GPU path has not been run on Windows or macOS, and
`--no-gpu-hull` draws the hull the old way. Before is `--no-gpu-hull` in the same build, alternated with the GPU
path; each cell is the range of the medians of separate runs, frame time then CPU time per frame:

| Hull | Level | Before: frame, CPU | After: frame, CPU |
|---|---|---|---|
| `demo-hull` | 3 | 4.1-5.3 ms, 3.9-5.0 ms | 2.0-2.3 ms, 1.7-2.0 ms |
| `wigley` | 4 | 85-91 ms, 85-91 ms | 3.5-4.0 ms, 3.1-3.4 ms |
| `container-230m` | 3 | 31-40 ms, 31-40 ms | 7.0-8.7 ms, 6.7-8.2 ms |
| `container-230m` | 4 | 127-160 ms, 127-160 ms | 7.5-8.4 ms, 7.1-8.1 ms |
| `container-230m` | 5 | the old path panics | 9.0-10.6 ms, 8.7-10.2 ms |

At level 5 the old path asks wgpu for a 403 MiB vertex buffer, over its 256 MiB limit, and panics, so
`--no-gpu-hull` is no way out there. A new level 5 mesh of `container-230m` (1.29 million triangles, 103 MiB)
takes about 140 ms to build and 12 ms to upload. Since 2026-11-26 a mesh over about 24 000 refined faces is built in
a background job while the window draws the highest level under that at once (level 3 for `container-230m`), so an
edit no longer waits for it; orbiting is unchanged (alternating runs against 0.7.0 on a loaded machine). A pause of
about half a second after an edit to `container-230m` remains, at level 3 as well as 5, from something other than
the mesh; it is not yet profiled.

- [x] Profile the cost per frame.
- [x] Cut the waterline once per mesh and plane instead of once per view per frame.
- [ ] Draw the hull on the GPU, so that moving the camera does not rebuild or resend its triangles. Done in
  code, and the frame target (16.7 ms at levels 4 and 5) is met above. The CPU target, 8 ms per orbit frame at
  levels 3, 4 and 5, is not: level 5 takes 8.7-10.2 ms, and levels 3 and 4 sit at the limit.
- Done when orbiting the largest example stays smooth at level 4, with the numbers recorded before and after.

### M3: Selection

- [ ] Edge picking. Multi-select (Shift+click, a Shift+drag rectangle, a whole station or lengthwise line) and
  moving several points together are done, with connected editing on top.
- Done when headless tests cover each, and topology tools use the selection rather than picked pairs.

### M4: More naval architecture

- [ ] Floodable length curves.
- [ ] VRML import, and edge extrusion in the GUI.
- [x] One speed shared across the Performance tab and the Wave pattern view (resistance, waves, seakeeping).
- [x] Grey out the resistance curve where the method does not apply: all of the Holtrop curve for a hull outside
  its Cp, L/B or B/T range, and DSYHS speeds outside Fn 0.15 to 0.75. Neither Holtrop paper prints a top Froude
  number, so no Holtrop speed is greyed on that account.
- [ ] A 3D wave surface in Perspective view.
- [ ] A wave pattern method for planing hulls; the Wave pattern view's title, "Thin-ship theory (Michell)",
  then becomes a drop-down that picks the method.

### M5: Binaries

- [x] Replace egui's default fonts, whose `Ubuntu-Light.ttf` is under the Ubuntu Font Licence, with DejaVu
  (`crates/seacraft-core/assets/fonts`, `NOTICE`).
- [x] Ship full licence texts with binaries: `cargo xtask dist` writes them from the crates that are compiled in.
- [x] Add release packaging: a version tag builds a Linux archive, AppImage and Flatpak bundle, a Windows zip
  and macOS disk images, then creates a draft release (`.github/workflows/release.yml`, `packaging/`).
- [ ] Make the first release: run the workflow, and have someone open the GUI on Windows and on a Mac.
- [x] Decide on publishing to crates.io: not now. The people a release is for download a program, and the
  library API is not meant to be stable yet. Flathub waits too, since it must build from source.

### M6: Builder's toolkit

- [x] Station moulds from the refined hull, with the skin deducted along the surface normal, mould face and
  bevel on each sheet, strongback legs, half moulds (`crates/seacraft-core/src/templates.rs`).
- [x] Profile (backbone) and raked transom patterns.
- [x] A dependency-free PDF writer with an embedded DejaVu subset, tiled over A4, A3 or Letter with join marks,
  a map page and 100 mm check rules, or one page a pattern; SVG and DXF in millimetres (`sheet.rs`, `pdf.rs`).
- [x] The table of offsets as a PDF, metric with feet-inches-eighths beside it.
- [x] The same in the CLI (`seacraft templates`, `offsets --pdf`), the GUI (File ▸ Builder templates…), the
  control channel (`build.templates`) and MCP (`export_templates`, `export_offsets_pdf`), with a boatbuilding
  skill.
- [ ] Print the calibration page and one station and measure them.
- [ ] Open the PDFs on Windows and macOS viewers.
- [x] Plywood panels in the core and CLI: panels found from layers and creases, laid flat by least stretch, seam
  drift and stretch against provisional limits, station marks and seam ticks, offsets from a baseline, plywood
  sheets end to end (`flatten.rs`, `panels.rs`, `seacraft panels`).
- [x] Plywood panels in the GUI, the control channel and MCP; the offsets tables inside the pattern PDF
  (File ▸ Builder panels…, `build.panels`, `export_panels`).
- [x] A plywood example whose every panel lies flat: `skiff-4m`, a 4.3 m flat-bottom skiff (bottom, sides,
  transom).
- [x] Measurements on every panel pattern (overall length and width, baseline offsets, and seam lengths as reference
  lines and text) and automatic plywood nesting onto joined blanks or split parts, with a full-size sheet view in
  Builder panels, per-sheet SVG/DXF cutting files, and scaled layout overview pages at the front of the PDF
  (`nest.rs`, `panel_layout.rs`, `panel_dims.rs`).
- [ ] Build a skiff from the panel patterns and recalibrate the 2 mm / 0.1 % limits on it (`skiff-4m` is the
  candidate).
- [ ] Materials and weight: skin area, plywood sheets, glass and resin, and the weight they add.
- [ ] A build pack: patterns, offsets, panels and a cutting list in one document.

## Possible later work

These have no milestone and no date; the roadmap is not worked in order, and they are here so they are not lost.

- DTMB 5415 against the SIMMAN 2008 particulars (∇ 8424.4 m³, S 2972.6 m², CB 0.507, CM 0.821, LCB −0.683 %
  Lpp). It needs a new example fitted to the published hull lines.
- A published GZ or KN table for a hull with printed offsets (for example a textbook worked example), once a
  source is in hand.
- The `wigley` example's fit below half draft (5.5 % thin at a quarter draft, `docs/oracles.md` §44), if a use
  for the example at light drafts turns up.
- More plywood examples beside `skiff-4m`: a two-chine skiff (bottom, a lower and an upper strake each side) and a
  multi-chine sea kayak, each with every panel lying flat.
- Interactive panel nesting: hand placement (dragging pieces on sheets), puzzle joints in place of straight split
  joints, and optional quarter-turn placement across the grain.
- A command to make one control point a corner by hand (GUI, CLI and MCP); today a point is a corner only where
  three creases meet, or when a `.fef` says so.
- Seakeeping on DTMB 5415 (a transom, a real warship) against Irvine, Longo & Stern (2008), *Journal of Ship
  Research* 52(2), heave and pitch in head waves.
- Seakeeping on Wigley II and IV (L/B 5) from the same Journée (1992) report, with the same test code: II was run
  at Fn 0.2 only and IV not at Fn 0.4.

## Gap review

| Area | Today | Missing |
| --- | --- | --- |
| Validation | Closed-form cases and project scripts (`docs/oracles.md`) | Comparison with tank tests, trials and approved software |
| Intact stability | Free-trim GZ, IMO A.749, IS Code weather criterion | Other criteria sets, stability booklet output |
| Damage | Deterministic lost buoyancy | Probabilistic SOLAS, floodable length |
| Resistance | Holtrop-Mennen, Michell, Savitsky, within each method's range | Appendage detail, sailing yacht performance |
| Wave pattern | Far-field thin-ship (Michell) waves, checked against Kelvin's asymptotics | A measured wave cut to compare with, the wave profile on the hull, heel and trim |
| Small craft | None | ISO 12217-2 STIX (the formula is in a paid standard) |
| Seakeeping | Strip theory, heave and pitch in head seas; compared with Wigley tank tests (`docs/oracles.md` §46) | 3D panel method, oblique seas, roll |
| Modelling | Point edits alone, in groups or with followers; split face, insert edge, crease, layers | Edge picking, edge extrusion |
| File formats | `.fef`, SVG, DXF, IGES, OBJ, STL, 3MF, PLY | FREE!ship's binary `.fbm`, VRML import |
| GUI | Dark theme, four viewports, docks | Light theme, command palette, smooth 3D on large models |

## Known limits

- `project.set` over the control channel is not undoable.
- The design report names no tanks or windage items, though the weather criterion counts them.
- The crash recovery prompt shows its time in UTC, and undo history is not recovered.
- Clipped tank overlays are drawn at subdivision level 2.
- A one-sided model keeps whole stations; a catamaran's stations show one demihull a side.
- The Windows programs are unsigned and have no icon in Explorer, the macOS app is not notarised, and `.fef`
  files do not open on double-click on any system.
