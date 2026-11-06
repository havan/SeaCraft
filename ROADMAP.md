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

The UI is slow with 3D models. Measured on the demo hull in a release build: one frame of four viewports
takes roughly 60 ms, while the idle app repaints nothing.

- [ ] Profile the cost per frame, and cache what does not change when only the camera moves.
- Done when orbiting the largest example stays smooth, with the numbers recorded before and after.

### M3: Selection

- [ ] Multi-select, marquee and edge picking, and moving several points together.
- Done when headless tests cover each, and topology tools use the selection rather than picked pairs.

### M4: More naval architecture

- [ ] Floodable length curves.
- [ ] VRML import, and edge extrusion in the GUI.

### M5: Binaries

- [ ] Before anyone distributes binaries: egui's default fonts include `Ubuntu-Light.ttf` under the Ubuntu
  Font Licence. Confirm it may ship in a GPLv3 binary or replace it (`deny.toml`, `NOTICE`).
- [ ] Ship full licence texts with binaries, and add release packaging.
- [ ] Decide on publishing to crates.io.

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
