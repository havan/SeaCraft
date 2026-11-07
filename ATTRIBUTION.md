# Attribution

SeaCraft is `GPL-3.0-or-later` (see `LICENSE` and `README.md`). Parts of it are
adapted from the Tangency Labs CAD projects under permissive licences, which are
one-way compatible with the GPL: code may be copied *into* SeaCraft, never the
other way round. SeaCraft is not affiliated with Tangency Labs or endorsed by it.
Every port is listed here with the file it came from and what was changed.

Licence texts are in `LICENSES/`. Where a project offers `MIT OR Apache-2.0`,
SeaCraft takes it under MIT (`LICENSES/MIT.txt`). Apache-2.0-only material is
under `LICENSES/Apache-2.0.txt`, with the notices it requires in `NOTICE`.

The projects are **not** Cargo dependencies. SeaCraft has no `solidcraft-*`,
`fieldcraft-*` or `solidcore` in `Cargo.toml`: they measure in millimetres and
pull in `truck-*` and `egui 0.36`, while SeaCraft is metres and `egui 0.37`.
Only self-contained modules are adapted by hand.

---

## `crates/seacraft-core/src/dxf.rs`

* **Source:** `solidcraft/crates/io/src/dxf_out.rs`
* **Project:** [solidcraft](https://github.com/tangency-labs/solidcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 SolidCraft contributors

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
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 SolidCraft contributors

Surface-fairing analysis modes (`Zebra`, `Curvature`, `Draft`) and the
dependency-free 2× SSAA CPU z-buffer triangle rasterizer plus stored-deflate PNG
encoder, adapted to `crate::math::Vec3` and extended with depth-tested
translucent waterplane compositing.

## `crates/seacraft-core/src/threemf.rs`

* **Source:** `solidcraft/crates/io/src/threemf.rs`
* **Project:** [solidcraft](https://github.com/tangency-labs/solidcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 SolidCraft contributors

Solidcraft's bounded zip code comes from GridCraft `crates/xlsx/src/package.rs` (Copyright (c) 2026
ArtCraft Team and the GridCraft contributors, MIT OR Apache-2.0, taken under MIT).

Spatial-grid vertex welding (`weld`) and 3MF 1.3 package structure
(`[Content_Types].xml`, `_rels/.rels`, `3D/3dmodel.model`), written in metres
through a zero-dependency stored-ZIP writer with IEEE 802.3 CRC-32.

## `crates/seacraft-core/src/iges.rs` (B-spline surface fit, entity 128)

* **Source:** `solidcore/crates/solidcore/src/nurbs/srf_fit.rs`
* **Project:** [solidcore](https://github.com/tangency-labs/solidcore) (Tangency Labs)
* **Upstream of that file:** `source/SMLib/src/NL_SrfFit.cpp` in NVIDIA
  [USD BRep](https://github.com/NVIDIA-Omniverse/usd-brep), which solidcore ports to Rust
* **Licence:** Apache-2.0 only (no MIT option; this is why every SeaCraft binary needs GPLv3)
* **Copyright:** Copyright (c) 2026 The solidcore contributors; Copyright (c) 2020-2026 NVIDIA
  CORPORATION & AFFILIATES. See `NOTICE`.
* **Modified:** yes. Reduced to the interpolation routines and adapted to `crate::math::Vec3`.

Tensor-product B-spline surface interpolation (`fit_surface` / `interpolate_grid`:
averaged chord-length parameters, averaging knot vectors, and sequential `u`/`v`
collocation solves) for exporting lofted hull patches as IGES 5.3 entity 128.

## `crates/seacraft-gui/src/control.rs` and `crates/seacraft-cli/src/mcp.rs`

* **Source:** `fieldcraft/crates/ui-egui/src/control.rs` and
  `fieldcraft/crates/mcp/src/`
* **Project:** [fieldcraft](https://github.com/tangency-labs/fieldcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 FieldCraft contributors

Upstream of those files: fieldcraft adapted them from solidcraft, whose own attribution traces
them to CADCraft (Copyright (c) 2026 ArtCraft Team and the CADCraft contributors) and, for the tool
schema helpers, GridCraft (Copyright (c) 2026 ArtCraft Team and the GridCraft contributors). Both are
MIT OR Apache-2.0, taken under MIT.

Loopback TCP JSON-lines GUI control channel (`--control HOST:PORT`, executed on
the next `egui` frame with input clamping) and the stdio JSON-RPC 2.0 Model
Context Protocol (`seacraft mcp`) server supporting both in-process headless
evaluation and remote `--connect` to a live GUI window.

## `crates/seacraft-core/src/control_auth.rs`

* **Source:** `solidcraft/crates/engine/src/control_auth.rs`
* **Project:** [solidcraft](https://github.com/tangency-labs/solidcraft) (Tangency Labs)
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 SolidCraft contributors

Control channel token generation, constant-time token comparison, 0600 token
file creation and RAII token file cleanup. The path checks (`check_read_path` and
`check_write_path`) are SeaCraft's own, after the read and write roots in fieldcraft's
`crates/io/src/roots.rs`.

## `DISCLAIMER.md`

* **Source:** `DISCLAIMER.md` of [solidcraft](https://github.com/tangency-labs/solidcraft)
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 SolidCraft contributors

Structure and the warranty, liability and indemnity wording. The sections on validation and approval
are SeaCraft's own.

## Contributor documents

* **Source:** `AGENTS.md`, `CONTRIBUTING.md`, `skills/README.md`, `skills/solidcraft-contributing/SKILL.md` and
  the pull request template of [solidcraft](https://github.com/tangency-labs/solidcraft); `CODE_OF_CONDUCT.md`,
  `SECURITY.md`, `CONTRIBUTING.md` and the issue templates of
  [fieldcraft](https://github.com/tangency-labs/fieldcraft)
* **Licence:** MIT OR Apache-2.0, taken under MIT
* **Copyright:** Copyright (c) 2026 SolidCraft contributors; Copyright (c) 2026 FieldCraft contributors

The structure and some wording of SeaCraft's `AGENTS.md`, `CONTRIBUTING.md`, `SECURITY.md`,
`CODE_OF_CONDUCT.md`, `skills/seacraft-contributing/`, `skills/seacraft-ground-rules/`, and `.github/` templates
are adapted from these. The ideas behind `deny.toml`, `.github/workflows/ci.yml` and the `cargo xtask` repository
checks come from them too; that code is SeaCraft's own.

---

## `site/fonts/`

The website serves these fonts unchanged. They are not linked into any program, and each licence text is next
to its font.

* **Big Shoulders Stencil:** Copyright 2019 The Big Shoulders Project Authors, SIL Open Font License 1.1
* **Atkinson Hyperlegible Next:** Copyright 2020-2024 The Atkinson Hyperlegible Next Project Authors, SIL Open
  Font License 1.1
* **Atkinson Hyperlegible Mono:** Copyright 2020-2024 The Atkinson Hyperlegible Mono Project Authors, SIL Open
  Font License 1.1
