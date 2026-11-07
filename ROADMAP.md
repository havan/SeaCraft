# SeaCraft roadmap

Where SeaCraft is and what comes next. Update it when a milestone task lands. Results are not validated;
see [`DISCLAIMER.md`](DISCLAIMER.md).

## Works today

- Hull modelling on a subdivision control net: FREE!ship's hybrid Loop / Catmull-Clark scheme, layers,
  creases, `.fef` read and write, parametric generator, example hulls.
- Desktop modeller: four linked viewports, control-point dragging with undo, split face and insert edge,
  docks, autosave and crash recovery.
- Hydrostatics, cross curves, GZ with free trim, IMO A.749 and IS Code weather criteria.
- Loading conditions with tanks and free-surface correction; deterministic lost-buoyancy damage stability.
- Resistance by Holtrop-Mennen, Michell thin-ship theory and Savitsky; a hull optimiser.
- Strip-theory heave and pitch motions, irregular head seas, MSI.
- Lines plans (SVG, DXF, IGES), mesh exports (OBJ, STL, 3MF, PLY, HTML), an HTML design report.
- A command line, a loopback control channel, and an MCP server for AI agents.

## Next

### M1: Validate the numbers

The most important milestone: most results have never been compared with reality.

- [ ] Compare hydrostatics and GZ with a published or independently computed hull.
- [ ] Compare Holtrop-Mennen and Savitsky with published worked examples, and Michell with published Wigley hull data.
- [ ] Compare seakeeping RAOs with published experiments.
- Done when each area has at least one oracle in `docs/oracles.md` with a checkable published source,
  and `DISCLAIMER.md` and the README say exactly what is covered.

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
takes about 140 ms to build on the window thread and 12 ms to upload.

- [x] Profile the cost per frame.
- [x] Cut the waterline once per mesh and plane instead of once per view per frame.
- [ ] Draw the hull on the GPU, so that moving the camera does not rebuild or resend its triangles. Done in
  code, and the frame target (16.7 ms at levels 4 and 5) is met above. The CPU target, 8 ms per orbit frame at
  levels 3, 4 and 5, is not: level 5 takes 8.7-10.2 ms, and levels 3 and 4 sit at the limit.
- Done when orbiting the largest example stays smooth at level 4, with the numbers recorded before and after.

### M3: Selection

- [ ] Multi-select, marquee and edge picking, and moving several points together.
- Done when headless tests cover each, and topology tools use the selection rather than picked pairs.

### M4: More naval architecture

- [ ] Floodable length curves.
- [ ] VRML import, and edge extrusion in the GUI.

### M5: Binaries

- [x] Replace egui's default fonts, whose `Ubuntu-Light.ttf` is under the Ubuntu Font Licence, with DejaVu
  (`crates/seacraft-gui/assets/fonts`, `NOTICE`).
- [x] Ship full licence texts with binaries: `cargo xtask dist` writes them from the crates that are compiled in.
- [x] Add release packaging: a version tag builds a Linux archive, AppImage and Flatpak bundle, a Windows zip
  and macOS disk images, then creates a draft release (`.github/workflows/release.yml`, `packaging/`).
- [ ] Make the first release: run the workflow, and have someone open the GUI on Windows and on a Mac.
- [x] Decide on publishing to crates.io: not now. The people a release is for download a program, and the
  library API is not meant to be stable yet. Flathub waits too, since it must build from source.

## Gap review

| Area | Today | Missing |
| --- | --- | --- |
| Validation | Closed-form cases and project scripts (`docs/oracles.md`) | Comparison with tank tests, trials and approved software |
| Intact stability | Free-trim GZ, IMO A.749, IS Code weather criterion | Other criteria sets, stability booklet output |
| Damage | Deterministic lost buoyancy | Probabilistic SOLAS, floodable length |
| Resistance | Holtrop-Mennen, Michell, Savitsky, within each method's range | Appendage detail, sailing yacht performance |
| Small craft | None | ISO 12217-2 STIX (the formula is in a paid standard) |
| Seakeeping | Strip theory, heave and pitch in head seas | 3D panel method, oblique seas, roll |
| Modelling | Point edits, split face, insert edge, crease, layers | Multi-select, marquee, edge extrusion |
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
