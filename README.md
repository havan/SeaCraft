# SeaCraft — a Rust clone of FREE!ship

SeaCraft is an independent Rust reimplementation of the open-source
[FREE!ship](https://sourceforge.net/projects/freeship/) hull-design program.
It reproduces FREE!ship's modelling kernel and its naval-architecture
measurement pipeline, and it reads and writes FREE!ship's `.fef` exchange
format so control nets can move between the two applications.

FREE!ship itself is a Delphi (Object Pascal) application released under the
GNU GPL v2. Its descendant **FREE!ship Plus** (Victor Timoshenko) and its
Lazarus port **FreeShip Plus in Lazarus** (Mark Malakanov) continue the same
codebase. SeaCraft is a clean-room Rust implementation written from the
documented behaviour and the GPL source, not a line-by-line translation.

---

## 1. How FREE!ship works (investigation notes)

### 1.1 Modelling: subdivision surfaces instead of NURBS

FREE!ship's core idea is that a hull is a **subdivision surface** — a coarse
*control net* of polygons that the program repeatedly refines and smooths.
The designer moves control points; the program shows the smooth limit surface.
This gives more freedom than NURBS because the control grid need not be
rectangular: holes, knuckles and local detail are just extra polygons.

The refinement is a hybrid of **Loop** and **Catmull–Clark** subdivision
("quad-triangle subdivision"). In the FREE!ship sources this is
`TFreeSubdivisionSurface.SubDivide` (`Units/FreeGeometry.pas`):

1. Every face is split by inserting an **edge point** in the middle of each
   edge and, for faces with more than three corners, a **face point** at the
   centroid. Triangles split into four triangles; larger faces connect each
   edge point to the face point.
2. Every point is then moved by an **averaging** step that combines the new
   face/edge points with the original vertex position. The averaging weights
   depend on the vertex type — regular, crease, corner, boundary — so that
   creases (knuckle lines) and corners are preserved instead of being
   smoothed away.
3. The resulting mesh replaces the old one; repeating the two steps `n` times
   is "subdivision level `n`".

Key practical consequences that SeaCraft mirrors:

* Faces must be consistently wound and normals must point **outward** —
  hydrostatics integrates the volume "behind" the faces.
* Edges should have exactly two adjacent faces except at deliberate boundary
  edges. A submerged boundary edge makes the volume integral meaningless.
* Crease edges keep a knuckle sharp; corner points are pinned.
* Layers group faces and carry per-layer flags (visible, symmetric,
  use-in-hydrostatics, material density, plate thickness).

### 1.2 Exchange format: `.fef`

`TFreeSubdivisionSurface.ExportFEFFile` / `ImportFEFFile` define a plain-text
format: layer count and layer records, then control points, control edges and
control faces. It is the format SeaCraft parses and writes, which makes it a
practical bridge to the original program (see `crates/seacraft-core/src/fef.rs`).

### 1.3 Hydrostatics: panel integration

FREE!ship does **not** use ordinate tables. `TFreeHydrostaticCalc.Calculate`
(`Units/FreeShipUnit.pas`) integrates the refined surface directly:

* The hull is refined to the current subdivision level.
* Every triangle is clipped against the waterplane.
* Volume and the centre of buoyancy come from the divergence theorem over the
  submerged panels; the waterplane closes the volume, so the projection origin
  is placed *in* the waterplane to cancel the lid's contribution.
* Wetted surface is the sum of the submerged panel areas.
* The waterline is the intersection of the mesh with the waterplane; chaining
  those segments gives waterplane area, centre of flotation and the transverse
  and longitudinal second moments, hence `BM = I / ∇` and `KM = KB + BM`.
* Section areas along the hull give the section-area curve; the centreline cut
  gives the lateral (profile) area.
* Coefficients follow from these: `Cb = ∇/(L·B·T)`, `Cp = ∇/(Am·L)`,
  `Cm = Am/(B·T)`, `Cwp = Awp/(L·B)`.
* Heel and trim are handled by tilting the waterplane, so the same panel
  machinery yields cross curves (`KN`) and, with `KG`, righting levers (`GZ`).

### 1.4 Other FREE!ship features

* **Intersections / linesplan** — stations, buttocks, waterlines and diagonals
  are computed by cutting the refined surface with planes and are drawn as a
  conventional three-view linesplan.
* **Unfolding (developability)** — flat-plate development of (near-)developable
  surface regions, with a check of the 3D area against the unfolded area.
* **Exports** — OBJ, STL, DXF, IGES (NURBS), VRML, GHS and its own `.fef`.
* **Hullform transformation** — Lackenby transformation to hit a target `Cb`,
  `Cp` and `LCB`.

---

## 2. What SeaCraft implements

| FREE!ship concept | SeaCraft module |
| --- | --- |
| Control net, layers, creases | `crates/seacraft-core/src/mesh.rs` |
| Loop / Catmull–Clark refinement | `subdiv.rs` |
| `.fef` import/export | `fef.rs` |
| Panel hydrostatics, coefficients, SAC | `hydro.rs` |
| Cross curves, `KN`/`GZ`, draft/trim solving, IMO A.749 | `stability.rs` |
| Stations / buttocks / waterlines (SVG, DXF, IGES, 3D combs) | `linesplan.rs` |
| AutoCAD 2000 DXF writer | `dxf.rs` |
| IGES 5.3 B-spline curve (126) & surface (128) writer | `iges.rs` |
| Watertight vertex welding + 3MF 1.3 archive writer | `threemf.rs` |
| Curvature, zebra and draft analysis | `analysis.rs` |
| CPU z-buffer rasterizer + waterplane depth + PNG | `raster.rs` |
| OBJ / STL / 3MF / PLY / IGES / HTML viewer | `export.rs` |
| Parametric demo hull | `demo.rs` |
| Command-line front end + MCP stdio server (`seacraft mcp`) | `crates/seacraft-cli` |
| Desktop modeller (4 viewports, drag, docks, `--control`) | `crates/seacraft-gui` |

### Modelling kernel

`subdiv::evaluate` refines a `Surface` to a given level. `SubdivisionMode`
selects the FREE!ship default (`Loop`: triangles become four triangles, larger
faces become quads) or pure Catmull–Clark. Creases, corners and boundary
vertices keep their FREE!ship weights. `evaluate_full` additionally mirrors
symmetric layers so the whole hull is produced.

### Hydrostatics

`hydro::Hull` refines once and can then be measured for many floating
conditions, which is what the stability solver does. `hydro::compute` returns
volume, displacement, `LCB`, `KB`, waterplane area/inertia, `LCF`, `BM`, `KM`,
all the coefficients, the section-area curve and the lateral area. The results
are serde-serialisable and available as JSON from the CLI.

`curves::bonjean` builds the Bonjean family: the submerged section area at each
station against a range of drafts (`seacraft bonjean`), plus an area-curve
prismatic coefficient from the section-area curve. Both curves plot to
standalone SVG with no external dependency, and both are cut from the same
refined mesh and clipping path as the hydrostatics.

`transform::fit_to_targets` rescales the control net to a target waterline
length, beam and draft (`seacraft transform`), and `transform::apply` /
`transform::mirror` translate or reflect it. The scale is anchored at the aft
end, the centreline and the keel, so a stretched hull keeps its transom, its
symmetry and its ground line; a mirror reverses every face's winding, because a
reflection would otherwise turn the outward normals inward and measure the
displacement negative.

`resistance` implements Holtrop–Mennen (1984) still-water resistance:
`RT = RF(1+k1) + RAPP + RW + RB + RTR + RA` on the ITTC-57 friction line, with
`PE = RT·V` and no propeller model, so it reports the power the hull needs
rather than the power the engine delivers. Everything the method takes from the
hull form — `LWL`, `BWL`, `CB`, `Cp`, `Cm`, `LCB` as a percentage of the length,
the wetted surface, the immersed transom and the half-angle of entrance `iE` —
is measured on the refined mesh. `seacraft resistance` prints the breakdown and
warns when the hull falls outside the ranges the method was fitted to.

`offsets::table_of_offsets` produces the drawing-office table — half-breadths
at stations x waterlines and buttock heights at stations x buttocks — as a
terminal table, long-format CSV, JSON or a standalone HTML page
(`seacraft offsets`). A cell the section cannot reach is empty (`-`), not zero.

### Stability

`stability::cross_curves` floats the hull at a constant displacement for each
heel angle, then reports `KN` and `GZ = KN − KG·sin φ`. Two conditions are
supported. *Fixed trim* holds the trim and bisects on the draft only — fast,
but the `LCB` drifts away from the `LCG` as the hull heels. *Free to trim*
(`--free-trim`, `--lcg x`) solves `(draft, trim)` so the hull is in
longitudinal equilibrium at every heel; it costs a few more iterations and is
what a real boat does. With no `--lcg`, the upright `LCB` at the design draft
is used.

### Fairing analysis

Fairing a hull means looking at how light reflects off it. `analysis` computes
per-vertex normals and discrete curvature (cotangent Laplacian for the mean
curvature, angle deficit for the Gaussian one) and provides three analysis
shades:

* **Zebra** — reflection stripes; an unfair bump shows up as a kink in a band.
* **Curvature** — false colour from flat (blue) through medium (green) to tight
  (red), with `--radius` the curvature at which it saturates.
* **Draft** — green where the surface can be pulled from +z at more than the
  draft angle, red on undercuts.

`raster` is a dependency-free CPU z-buffer renderer with 2× supersampling and a
small PNG encoder, so those modes work per pixel with no GPU:

```console
seacraft render hull.fef -o zebra.png --mode zebra --stripes 16
seacraft render hull.fef -o curvature.png --mode curvature --radius 1.5 --view body
```

The desktop app draws the same modes in all four viewports (View ▸ Surface
analysis, or `--analysis zebra`), rendering each viewport through the
rasterizer into a texture and caching it until the model or camera changes.

### Linesplan and export

`linesplan::svg` produces a standalone three-view drawing (profile, plan, body
plan) with stations, buttocks and waterlines. `export` writes OBJ, binary STL,
ASCII PLY and a self-contained three.js HTML viewer.

Add `--combs` to either drawing and every curve grows a **porcupine comb**:
spikes normal to the curve whose length is the local curvature times a gain
(`--comb-scale`, 0.6 m per 1/m by default, clamped to 12 % of the view). The
curvature is read at the comb's own sampling interval (~2 % of the view, not at
every refined vertex 5 mm apart, which would measure the mesh and not the
hull), and the line through the spike tips is the curve a fair hull keeps
smooth — so `--combs` turns a linesplan into a fairing check. In DXF the combs
go on their own `Combs` layer.

`linesplan::dxf` writes the same three views as an AutoCAD 2000 (`AC1015`)
drawing in metres, laid out conventionally (plan below the profile, body plan
to the right) on named layers — `Stations`, `Buttocks`, `Waterlines`,
`Waterplane`, `Centerline`, `Grid`, each with its own ACI colour, the frame and
centrelines dashed. Every curve is repeated as a 3D polyline on
`Stations 3D` / `Buttocks 3D` / `Waterlines 3D` at its true coordinates, so the
hull can be reopened in Rhino or ShipConstructor without rebuilding it from the
views. The writer is dependency-free; see `ATTRIBUTION.md`.

`linesplan::iges` writes the same station, buttock and waterline curves as IGES
5.3 rational B-spline curves (entity 126) in metres, so a receiving CAD package
gets exact curves rather than polyline approximations. It has been verified with
`gmsh`/Open Cascade: all 61 curves of the demo hull transfer, with non-zero
lengths and the correct 1000× millimetre scale. `export::iges_surface` (`seacraft
export hull.fef -o hull.igs`) additionally fits bicubic tensor-product B-spline
surfaces (entity 128) across the resampled starboard and port station curves,
and `export::threemf` (`-o hull.3mf`) writes a welded, watertight 3MF 1.3
archive in metres.

---

## 3. Command line

```text
seacraft demo      -o hull.fef [--symmetric]
seacraft info      hull.fef
seacraft hydro     hull.fef [--draft T] [--trim m | --free-trim [--lcg x]] [--heel deg] [--kg m] [--json]
seacraft sac       hull.fef [--stations N] [--csv out.csv] [--svg sac.svg] [--json]
seacraft bonjean   hull.fef [--stations N] [--drafts T1,T2,...] [--draft-steps N]
                              [--csv out.csv] [--svg bonjean.svg] [--json]
seacraft offsets   hull.fef [--stations N] [--waterlines h1,h2,...] [--buttocks y1,y2,...]
                              [--csv out.csv] [--html offsets.html] [--json]
seacraft transform hull.fef -o scaled.fef [--lwl L] [--bwl B] [--draft T]
                              [--scale sx,sy,sz] [--translate dx,dy,dz] [--mirror x|y|z] [--json]
seacraft resistance hull.fef [--speeds 6,8,10,12] [--draft T] [--cstern X]
                              [--appendage-area m2] [--bulb-area m2] [--csv out.csv] [--json]
seacraft stability hull.fef [--kg m] [--heels 0,10,20,...] [--displacements t,...]
                            [--trim m | --free-trim [--lcg x]] [--csv out.csv]
seacraft linesplan hull.fef -o linesplan.svg|.dxf|.igs [--stations N] [--buttocks N] [--waterlines N] [--combs [--comb-scale m]]
seacraft export    hull.fef -o hull.obj|.stl|.3mf|.ply|.igs|.html [--level N] [--control]
seacraft render    hull.fef -o preview.png [--mode shaded|zebra|curvature|draft] [--view perspective|profile|plan|body] [--size WxH] [--wireframe]
seacraft subdiv    hull.fef -o baked.fef [--level N] [--mirror]
seacraft mcp       [--open hull.fef] [--connect 127.0.0.1:PORT]
```

Subdivision level defaults to 3; `--mode cc` selects pure Catmull–Clark.

### Example

```console
$ cargo run --release -- demo -o /tmp/demo.fef
$ cargo run --release -- hydro /tmp/demo.fef --kg 2.2
$ cargo run --release -- linesplan /tmp/demo.fef -o /tmp/demo.svg
```

---

## 4. Desktop application

`seacraft-gui` is an `eframe`/`egui` front end with no GPU pipeline: every
viewport is CPU-projected into an `egui::Painter`, so it runs on a plain X
display or a software renderer.

```console
cargo run --release -p seacraft-gui -- --open examples/demo-hull.fef
```

```
Perspective | Profile
------------+------------
Plan        | Body plan
```

* Drag a control point to deform the hull; the preview refines at level 2
  while dragging and at the full level on release. **Ctrl+Z** / **Ctrl+Shift+Z**
  undo and redo (64 snapshots), **Esc** clears the selection, **F** frames the
  model, **1**–**4** activate a viewport, **Space** toggles shading.
* Primary drag on empty space pans, secondary drag in the perspective view
  orbits, the wheel zooms.
* The left dock manages layers (colour, name, visible, symmetric, use in
  hydrostatics). The right dock is the live hydrostatics report: draft, trim,
  heel and KG, the full coefficient set, and GZ cross curves drawn in place.
  It recomputes 120 ms after the last geometry change, never mid-drag.
* File menu: new demo hull, open/save `.fef` (with an in-app path dialog, so
  no desktop portal is needed), export OBJ / STL / PLY / HTML viewer /
  linesplan SVG. Unsaved work is flagged in the status bar and confirmed
  before it is discarded.

The status bar carries the model summary, the subdivision level and mode, the
cursor position on the active viewport's focal plane and the crate version.

---

## 5. Coordinate system and units

* `x` forward, `y` to starboard, `z` up, keel at low `z`.
* Internal units are metres; `.fef` carries the project settings (length,
  beam, draft, water density, appendage coefficient, imperial flag).
* Heel is positive to starboard; trim is a length in metres, positive when the
  bow sits higher than the stern.
* Hydrostatics report `KB`/`KM`/`BM` above the keel, `LCB`/`LCF` in model
  coordinates.

---

## 6. Status and limitations

* The kernel, `.fef` round-trip, hydrostatics, stability, linesplan and mesh
  exports are implemented and tested.
* The desktop modeller (four viewports, control-point dragging with undo, layer
  and hydrostatics docks, file actions) is implemented and tested headlessly.
* DXF export of the linesplan (2D views plus 3D curve copies) is implemented
  and audit-clean under `ezdxf`.
* Zebra / curvature / draft analysis and a CPU z-buffer renderer with PNG
  output are implemented, in the CLI and in all four GUI viewports. Curvature
  combs are drawn both on the linesplan (`--combs`, SVG and DXF) and in the four
  GUI viewports (View ▸ Curvature combs, or `seacraft-gui --combs`).
* IGES export is implemented both for the linesplan as entity-126 B-spline
  curves and for the lofted hull as entity-128 bicubic B-spline surface patches;
  3MF export (`threemf.rs`) welds coincident vertices into a watertight mesh.
* Programmatic automation is supported via `seacraft-gui --control 127.0.0.1:PORT`
  (JSON lines over TCP, executed on the next frame) and `seacraft mcp` (JSON-RPC
  2.0 Model Context Protocol over stdio, either headless or connected via
  `--connect 127.0.0.1:PORT`). Eleven tools: `hull_info`, `load_hull`,
  `save_hull`, `move_point`, `set_crease`, `hydro_compute`,
  `stability_cross_curves`, `bonjean_curves`, `table_of_offsets`,
  `export_linesplan` and `render_png`.
* Not yet implemented: developability/unfolding, Lackenby
  transformation, propulsion (propeller, wake fraction) and the Savitsky
  planing method,
  VRML import, and interactive face/edge splitting or extrusion in the GUI.
* FREE!ship's own sample models are binary `.fbm` files; SeaCraft targets the
  documented text `.fef` exchange format instead.

## 7. Licence

SeaCraft is distributed under the GNU GPL v2 or later, matching FREE!ship's
licence. FREE!ship itself is Copyright (C) its authors; this project is an
independent reimplementation and is not affiliated with it.
