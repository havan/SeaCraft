---
name: seacraft-hull-numerics
description: Use when changing or reviewing anything that measures a hull in SeaCraft (subdivision, hydrostatics, stability, damage, sections, lines plans), before touching the code, because breaking these rules gives plausible but wrong numbers.
---

# Hull numerics

Read this before you touch geometry or measurement. After a change to `subdiv.rs`, `hydro.rs` or
`linesplan.rs`, run `cargo test` and check the reference numbers below.

Reference numbers for the demo hull at T = 0.62 m (level 3, Loop mode):
volume ≈ 11.85 m³, Cb ≈ 0.504, Cwp ≈ 0.824, KB ≈ 0.392 m, KM ≈ 2.63 m,
midship area ≈ 1.47 m². A change that moves these by more than ~1 % is a
regression, not an improvement. `cargo xtask ci` checks them.

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
* **Body plan** looks forward from aft with starboard on the right; stations
  aft of amidships on the left half, forward of it on the right, amidships on
  both (`linesplan::body_plan_stations`, `camera.rs`). Same in the GUI, SVG and
  DXF; 3D curves (DXF `Stations 3D`, IGES, Perspective) stay whole.
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
   never closes. `Hull` closes every open boundary loop (the sheer of a hull
   without deck faces) with `Hull::lids` (`hydro::boundary_lids`); measure
   with `Hull::integrate` / `closed_tris`, not `integrate(&hull.tris)`.
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

* **Patterns are cut as the offsets table is cut.** `templates::cut` uses the
  same refined `Hull`, `plane_segments` and `chain_paths` as the offsets table,
  so a mould and the table agree at a station. The skin comes off by
  `t / |n_p|` (the surface normal's part in the drawing plane, held at 0.2), not
  by `t`: a surface meeting the plane off square needs more. A piece the move
  consumes at a corner (shorter than its ends' shortening) is dropped and its
  neighbours' moved lines meet; only a line consumed whole, one turned inside
  out, or moved pieces that cross are a "skin thicker than the radius" warning.
* **A plane lying in a face has no section.** Cutting exactly on a transom or
  an end face gives degenerate segments; the transom is cut 1 mm forward of the
  plane given, and triangles lying in the plane are left out of a cut.
* **`chain_paths` lost points of closed loops.** It dropped the last point of
  the first closed loop and compared a vertex with an edge index to decide
  whether a later loop had closed. Fixed with
  `chained_loops_keep_every_point_and_are_all_closed`; decked sections and
  multi-loop waterplanes depend on it.

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
* **A draft above the keel cannot place every waterplane.** Past 90° heel the
  submerged volume *falls* as that draft grows (the solve returned draft 0 and
  a plausible GZ); at 90° the draft does not move the plane (no solution); a
  light hull at a steep heel needs the plane below the keel on the centreline
  (clamped to 0: the demo at 30 % of its displacement lost 0.15 m of GZ at
  55–60°); and
  `waterplane_for`'s trim stops tilting the plane at 90°. `stability` falls
  back to the plane offset along its normal, bracketed by
  `Hull::extent_along`, with the free-trim loop on the *level* trim
  (`hydro::waterplane_normal`). The check is the half-full box with `G` at
  its centre: closed-form GZ, `GZ(180° − φ) = −GZ(φ)` (`docs/oracles.md` §20).
  `damage::solve_damaged_draft` had the same clamp and uses the same offset
  fallback (`damaged_buoyancy_in`); its check is the light barge's
  bilge-out triangle (`damage.rs` tests).
* **An open deck is a leak once it is wet.** The demo hull and most
  FREE!ship models end at the sheer. With the origin in the waterplane the
  divergence integral over an open surface counts the cone from that origin
  to the opening, so past deck-edge immersion the volume was wrong in either
  direction (open 10×4×2 box at 40°: 46.68 m³ for 51.92), and overloaded it
  grew without bound (`L·B·D + A_deck·h/3`): 1000 t on the demo floated at
  74.6 m with `equilibrium_found = true`. `Hull::new` closes each boundary
  loop with lid triangles (deck taken as watertight; never wetted surface),
  hydrostatics warns `open_hull_lid_submerged` when one is under water, and
  the draft searches stop expanding once the hull is fully submerged. The
  check is the open box against the half-full-box GZ oracle and its
  overload (`docs/oracles.md` §22).
* **A crease junction left `Regular` was smoothed away.** Where three or
  more crease edges meet (chine or sheer meeting a transom outline) the
  point must be pinned as a `Corner`; previously a `Regular` one was averaged
  as a smooth point and rounded the runabout's transom chine corner off by
  0.2 m, rolling the sides in towards the transom. `subdiv.rs` now automatically
  treats any vertex with three or more incident crease edges as a `Corner`
  during refinement, so user-creased models and `.fef` imports preserve sharp
  corners regardless of stored vertex kind. The checks are
  `subdiv::tests::creased_box_with_regular_vertices_preserves_corners_at_level_3`
  and `examples::checks::assert_transom_outline`.
* **A closed hull has no boundary to read the sheer from.** The damaged
  freeboard came from the open boundary alone, so on a hull with a deck, most
  of the examples, it read "-" and the deck-immersed warning could not fire.
  `damage::closed_hull_deck_edge` takes the perimeter of the large
  upward-facing regions; going by region keeps a spray rail from passing for
  the deck edge.
* **The free-trim residual falls as the trim grows.** Bow up moves the LCB
  aft, so ordering the secant bracket by trim mixed its signs.
  `stability::secant_on_trim` (shared by intact and damage) keeps the bracket
  by residual sign; do not reintroduce a `lo`/`hi` swap.
* **Chained loops come out in arbitrary directions.** Two pontoons walked
  opposite ways cancelled each other's waterplane. `hydro::loop_moments` and
  `clipped_loop_moments` take each loop's sign from its nesting
  (`math::loop_signs`), never from the winding the chain walk produced.
* **A point in plane coordinates needs the plane's offset.** `u·cx + v·cy`
  lies in the plane through the origin; add `-n·d` to land on the real plane
  (the LCF was at z ≈ 0 and shifted in x under trim).
* **Savitsky's deadrise relation is `C_Lβ = C_L0 − 0.0065 β C_L0^0.6`.** It
  gives `C_Lβ` from `C_L0`; solve it for `C_L0` (`planing::flat_plate_lift`),
  do not substitute `C_Lβ` on the right.
* **`PolygonMoments::from_polygon` returns signed values.** Areas/moments may
  come back negative depending on winding; callers take `abs()` where the
  orientation is not guaranteed.
