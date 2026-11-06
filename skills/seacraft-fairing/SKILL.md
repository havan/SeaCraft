---
name: seacraft-fairing
description: Use when a SeaCraft hull looks lumpy, kinked or has uneven curvature, or you must fair it by nudging control points, check zebra stripes or curvature renders, or draw linesplan curvature combs.
---

# SeaCraft fairing

## Overview

Fairness is judged on the refined surface, not the control net. Look first (zebra, curvature, combs), name the offending control points, move them in small steps, look again, then prove the hydrostatics did not drift. Fairing must not change the design, so every step is checked against a baseline.

## Before the first call

Conventions: x forward, y to starboard, z up, metres. A symmetric layer models only y >= 0; port is mirrored, so fair the starboard half and never push y below 0. Creases are deliberate hard edges (chine, transom, sheer): a stripe break on a crease is correct, not a fault.
Demo hull at T = 0.62 m: volume 11.85 m3, Cb 0.504, KM 2.62 m. Take a baseline now: `hydro_compute {"draft": 0.62, "kg": 2.2}` and write down volume, `lcb.x`, `kb`, `block_coefficient`, `waterplane_area_m2`. Tolerance for a fairing pass: volume and LCB within 0.5 %.
Run `hull_info {}` and check `validation.ok`; fairing a leaking net measures nothing.

## Look

1. Zebra from several views, one file each:
   - `render_png {"path": "/tmp/zebra-persp.png", "mode": "zebra", "view": "perspective"}`
   - `render_png {"path": "/tmp/zebra-prof.png", "mode": "zebra", "view": "profile"}`
   - `render_png {"path": "/tmp/zebra-body.png", "mode": "zebra", "view": "body"}`
   Smooth, parallel, evenly spaced stripes are fair. Stripes that kink, pinch or change spacing abruptly mark a bump; a stripe that jumps sideways across a face marks a tangent break.
2. `render_png {"path": "/tmp/curv.png", "mode": "curvature", "view": "plan"}` colours the discrete curvature. Look for isolated spots and for sign changes inside a panel. Use `"mode": "draft"` only for mould draught angle.
3. Combs: `export_linesplan {"path": "/tmp/lines.svg", "combs": true, "stations": 11, "waterlines": 7, "buttocks": 5}`. Each station, waterline and buttock gets a curvature comb; `comb_scale` (default 0.6) sets its gain. A fair curve has a comb that changes smoothly with no spikes; a spike or a comb that flips sides marks the nearest control point. A flat comb on a straight run is fine.
4. Judge a bump by two views. A feature seen only in one is likely a render artefact of that view.

## Fix in small steps

1. Pick the control point nearest the defect. `move_point {"index": 40, "xyz": [x, y, z]}` returns `before` and `after`, so the first call tells you where the point is.
2. Move it by at most 1 to 2 % of the local beam or depth, toward the average of its neighbours. One point per step.
3. Re-render the same view under the same file name pattern and compare the stripes. Repeat, up to about 10 moves, then re-baseline.
4. Do not fair across a crease: use `set_crease {"a": 4, "b": 5, "crease": false}` only when the edge was never meant to be sharp, and say so in the report.
5. A defect that persists across moves needs more control detail: `split_face {"face": 12}` (a triangle fan, the face keeps its index) or `insert_edge {"face": 12, "a": 3, "b": 7}` (non-adjacent points of that face).

## Verify

- `hydro_compute {"draft": 0.62, "kg": 2.2}` again. Volume and `lcb.x` within 0.5 % of the baseline; KB and `block_coefficient` within 1 %. If they moved more, undo the last moves by moving the points back.
- `hull_info {}`: `validation.ok` true, boundary and non-manifold edge counts unchanged.
- Final zebra and curvature renders in at least two views, and a comb linesplan, all showing no new defect.
- `save_hull {"path": "faired.fef"}` under a new name; keep the original.

## Red flags

- Judging fairness from the shaded render alone: shading hides curvature breaks that zebra shows.
- Rendering with a lower `level` than before; stripes at level 1 are coarse by construction. Keep `level` the same through a pass.
- Volume drifting steadily across moves: each was small, the sum was not. Re-baseline every ten moves.
- Moving a point on a symmetric layer to y < 0: it is clamped to 0 and the reply says `clamped`.
- Chasing a comb spike at a crease or a transom corner.
