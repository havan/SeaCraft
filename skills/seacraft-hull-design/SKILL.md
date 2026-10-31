---
name: seacraft-hull-design
description: Use when you load a SeaCraft hull (.fef or the demo) and must reach a target displacement, draft, LCB, prismatic coefficient or main dimensions, edit control points, compare trims, or save the result.
---

# SeaCraft hull design

## Overview

SeaCraft holds a hull as a subdivision control net and measures the refined surface. Design is a loop: measure, change one thing, measure again. Edits are in memory until `save_hull`. There is no direct Lackenby or prismatic-shift tool: `transform_hull` scales the whole model, shape changes are `move_point` moves you plan as numbers, and `optimize_hull` searches Cp, LCB and L/B for you when the goal is lower resistance at a speed.

## Before the first call

Conventions: x forward, y to starboard, z up, metres. Heel + to starboard. Trim is a length in metres, + bow up. KB, BM, KM, KG are above the keel; LCB and LCF are model x.
Write the targets down with units and tolerances (for example 1 % on volume) before editing. Demo hull at T = 0.62 m, level 3: volume 11.85 m3, displacement 12.147 t, Cb 0.504, Cp 0.755, Cwp 0.823, KB 0.392 m, KM 2.62 m, LCB x 4.297 m, LWL 10.70 m, midship area 1.467 m2. Anything that differs on the unedited demo is a bug, not a design change.

## Load and inspect

1. `load_hull {"path": "hull.fef"}` or `load_hull {"demo": true}`, or start from a specification with `generate_hull {"kind": "displacement", "lwl": 10, "beam": 3, "draft": 0.6, "depth": 1.6}`, or from a ready-made hull with its project with `example {"name": "runabout-6m"}` (the tool's `name` enum lists the examples; each brings its KG, its requirements and a `Design` loading condition, and the session KG becomes the example's). `load_hull {"path": ...}` likewise takes the KG of the model's `.seacraft.json` sidecar; without one the session KG is the 1.0 m default (`hull_info` says which in `session_kg_from`), so still pass `kg`. A server started with `--open` already has it loaded.
2. `hull_info {}`. Read `validation.ok`, `edges.boundary` and `edges.non_manifold`, `extents` (demo: length 12, beam 4, depth 1.66, keel 0) and each layer's `symmetric` and `use_in_hydrostatics`. A symmetric layer holds only y >= 0.
3. Stop and fix the model if validation fails: a submerged boundary edge leaks and every volume is wrong. A half model needs a transom face.

## Measure a baseline

`hydro_compute {"draft": 0.62, "kg": 2.2}` returns volume, `displacement_t`, `block_coefficient`, `prismatic_coefficient`, `lcb`, `kb`, `bm_transverse`, `km_transverse`, `gm_m`, `lwl`, `waterplane_coefficient`. Always pass `draft` and `kg`. Add `"free_trim": true` when the hull should float with LCB under LCG, with `"lcg": 4.3` for the LCG. Pass `"mainframe_x"` to fix the Cp reference section: it changes Cp and Cm (on a rescaled demo, x = 6 instead of the LCF moved Cp from 0.760 to 0.902), not the volume. Keep it the same across a comparison.

## Reach the targets

1. Main dimensions: `transform_hull {"lwl": 12.0, "draft": 0.7}` scales x and z so the waterline length and draft hit the targets, anchored at the aft end, centreline and keel. It scales the whole model (the demo went from 11.85 to 15.00 m3 and its overall length from 12.0 to 13.45 m), so re-run `hydro_compute` and expect every coefficient to shift. `"bwl"`, `"scale": [sx, sy, sz]`, `"translate"` and `"mirror": "y"` also exist.
2. Displacement: change draft first. `hydro_table {"steps": 5}` lists TPC and volume per draft so you can interpolate instead of guessing (demo: 1.77 t at 0.207 m, 6.14 t at 0.413 m, 12.15 t at 0.62 m).
3. LCB and Cp: move control points of the affected stations fore or aft with `move_point {"index": 50, "xyz": [7.9, 1.5, 0.54]}`. The reply gives `before` and `after`: read `before` first with a harmless move to the same place, or take coordinates from the `.fef`. Move one point by at most 2 to 3 % of length per step, then re-measure. On a symmetric layer y below 0 is clamped, and `clamped` in the reply says so. Multiple actions can be chained atomically with `batch {"calls": [{"name": "move_point", "arguments": {"index": 50, "xyz": [7.9, 1.5, 0.54]}}, {"name": "hydro_compute", "arguments": {"draft": 0.62, "kg": 2.2}}]}`: any failure rolls back earlier steps.
4. Hard edges: `set_crease {"a": 4, "b": 5, "crease": true}` for chines and transoms. `split_face` and `insert_edge` add control detail where a face is too coarse.
5. Trim at a speed: `trim_scan {"speed_kn": 8, "from": -0.4, "to": 0.4, "steps": 9}` holds displacement by solving the draft at each trim and reports the cheapest. `at_boundary: true` means widen the range.
6. Cut resistance at a design speed: `optimize_hull {"speed_kn": 8}` runs a bounded search over Cp, LCB and L/B (station shift and an x/y stretch), re-solves the draft so displacement holds, and scores each candidate with Holtrop-Mennen (Savitsky for a planing hull). Add `"kg": 1.0, "min_gm": 0.5` to hold stability, `"vars": ["cp", "lcb"]` to leave the beam alone, `"lcb_band": [0.4, 0.55]` to keep LCB (fraction of LWL from aft) in range. **It edits the model**: the session hull becomes the optimised one and the draft moves to the one that holds the displacement. Save the original first, or wrap the call in `batch` so a later failure rolls it back. Read `report.baseline` against `report.optimised`, `constraints_met`, `variables[].at_bound` (a bound hit means widen it with `bounds`) and `warnings`; it is a local search of a few smooth parameters, not a form optimiser.
7. Look: `render_png {"path": "/tmp/shaded.png", "view": "perspective"}`, then `"view": "body"` for sections.
8. Save: `save_hull {"path": "design-v2.fef"}`. Without `path` it overwrites the file it came from, so save under a new name first.

## Verify

- Re-run `hydro_compute` with the baseline arguments and compare each target to its tolerance. Report before and after.
- `hull_info {}` still shows `validation.ok` true and the same face count unless you split faces.
- Reload the saved file with `load_hull` and check the volume matches.

## Red flags

- Volume or Cb unchanged after an edit: the point is on a layer with `use_in_hydrostatics` false, or the index is wrong.
- A tool error such as `index 9999 is out of range` comes back as a normal result with `isError` true. Read it, do not retry blindly.
- Cb jumps by more than 5 % from one `move_point`: you moved a crease or transom corner. Undo by moving it back.
- Coefficients compared across calls with different `draft`, `level` or `mainframe_x`.
- KM or KB quoted from model z: they are above the keel, which is z = 0 only when `hull_info` shows keel 0.

Written against SeaCraft 0.1.0-beta.36.
