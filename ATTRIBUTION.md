# Attribution

SeaCraft is `GPL-2.0-or-later` (see `README.md`). Parts of it are adapted from
the Tangency Labs CAD projects under permissive licences, which are one-way
compatible with the GPL: code may be copied *into* SeaCraft, never the other
way round. Every port is listed here with the file it came from and what was
changed.

The projects are **not** Cargo dependencies. SeaCraft has no `solidcraft-*`,
`fieldcraft-*` or `solidcore` in `Cargo.toml`: they measure in millimetres and
pull in `truck-*` and `egui 0.36`, while SeaCraft is metres and `egui 0.37`.
Only self-contained modules are adapted by hand.

---

## `crates/seacraft-core/src/dxf.rs`

* **Source:** `solidcraft/crates/io/src/dxf_out.rs`
* **Project:** [solidcraft](https://github.com/tangency-labs/solidcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0
* **Copyright:** Tangency Labs and the solidcraft contributors

An AutoCAD 2000 (`AC1015`) DXF writer. What was kept:

* the file skeleton — HEADER with `$ACADVER`/`$HANDSEED`/`$INSUNITS`/
  `$MEASUREMENT`/`$EXTMIN`/`$EXTMAX`, the LTYPE/LAYER/STYLE/BLOCK_RECORD
  tables, empty model and paper space blocks, the ENTITIES section and the
  root dictionary;
* the `Out` writer with its three-column group codes, twelve-decimal real
  formatting with trailing zeros trimmed, and monotonically increasing
  hexadecimal handles;
* `DxfLinetype` (with the DASHED 12.7/−6.35 mm pattern and HIDDEN 2/−1 mm),
  `DxfLayer`, `DxfEntity`, and the LINE / LWPOLYLINE / CIRCLE / ARC / TEXT
  encodings, including PLINEGEN (group 70, bit 128) and centred TEXT.

What changed:

* `solidcraft_geom::Vec2` → a local two-field `Vec2`, and the geometry type is
  `crate::math::Vec3` where height is needed;
* the exact-conic machinery (`Seg2`, `Loop2`, ELLIPSE arcs, SPLINE entities)
  is dropped: SeaCraft's linesplan is polylines;
* `$INSUNITS` is 6 (metres) instead of 4 (millimetres), matching the core's
  units;
* added `DxfShape::Polyline3` (a `POLYLINE` with `VERTEX … 70 = 32` and
  `SEQEND`) so hull curves can be exported in place in 3D;
* the tests were rewritten against SeaCraft's own output rather than the
  original reader.

## `crates/seacraft-core/src/linesplan.rs` (DXF layout)

* **Source:** `solidcraft/crates/io/src/dxf_out.rs` — layer/colour/linetype
  conventions only (`DxfLayer::new(name, aci).dashed()`).
* The three-view layout, curve extraction and layer names are SeaCraft's own.

---

## `crates/seacraft-core/src/analysis.rs` and `crates/seacraft-core/src/raster.rs`

* **Source:** `solidcraft/crates/render/src/analysis.rs` and
  `solidcraft/crates/render/src/raster.rs`
* **Project:** [solidcraft](https://github.com/tangency-labs/solidcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0
* **Copyright:** Tangency Labs and the solidcraft contributors

Surface-fairing analysis modes (`Zebra`, `Curvature`, `Draft`) and the
dependency-free 2× SSAA CPU z-buffer triangle rasterizer plus stored-deflate PNG
encoder, adapted to `crate::math::Vec3` and extended with depth-tested
translucent waterplane compositing.

## `crates/seacraft-core/src/threemf.rs`

* **Source:** `solidcraft/crates/io/src/threemf.rs`
* **Project:** [solidcraft](https://github.com/tangency-labs/solidcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0
* **Copyright:** Tangency Labs and the solidcraft contributors

Spatial-grid vertex welding (`weld`) and 3MF 1.3 package structure
(`[Content_Types].xml`, `_rels/.rels`, `3D/3dmodel.model`), written in metres
through a zero-dependency stored-ZIP writer with IEEE 802.3 CRC-32.

## `crates/seacraft-core/src/iges.rs` (B-spline surface fit, entity 128)

* **Source:** `solidcore/crates/solidcore/src/nurbs/srf_fit.rs`
* **Project:** [solidcore](https://github.com/tangency-labs/solidcore) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0
* **Copyright:** Tangency Labs and the solidcore contributors

Tensor-product B-spline surface interpolation (`fit_surface` / `interpolate_grid`:
averaged chord-length parameters, averaging knot vectors, and sequential `u`/`v`
collocation solves) for exporting lofted hull patches as IGES 5.3 entity 128.

## `crates/seacraft-gui/src/control.rs` and `crates/seacraft-cli/src/mcp.rs`

* **Source:** `fieldcraft/crates/ui-egui/src/control.rs` and
  `fieldcraft/crates/mcp/src/`
* **Project:** [fieldcraft](https://github.com/tangency-labs/fieldcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0
* **Copyright:** Tangency Labs and the fieldcraft contributors

Loopback TCP JSON-lines GUI control channel (`--control HOST:PORT`, executed on
the next `egui` frame with input clamping) and the stdio JSON-RPC 2.0 Model
Context Protocol (`seacraft mcp`) server supporting both in-process headless
evaluation and remote `--connect` to a live GUI window.
