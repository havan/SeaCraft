---
name: seacraft-boatbuilding
description: Use when a builder wants to build a SeaCraft hull: full-size station moulds, profile or transom patterns, a table of offsets to loft from, or the skin thickness to take off the moulds.
---

# SeaCraft boatbuilding

## Overview

Patterns are cut from the refined hull and moved in by the skin, so the planking, plywood or strip-plus-glass lands on the designed surface. A wrong skin thickness, a pattern printed at the wrong size or a coarse mesh gives a mould that looks right and is wrong. Ask, read, export, relay what the program warned about, and leave the final check to the builder with a ruler.

## Before the first call

Ask the builder for the skin thickness in millimetres: plank, plywood, or strips plus glass and fairing compound on the outside. Never guess it. Zero means no deduction, and the program says so on every sheet: "Mould line is the hull surface: no skin deducted". Ask too whether they build on a strongback (its height, model z, at or above the sheer) and whether they want half moulds.
Conventions: x forward, y to starboard, z up, metres. Heights on patterns and in the offsets table are above the keel.
Read the hull first: `hull_info {}` for extents and `validation.ok` (a leaking net cuts nothing useful), then `table_of_offsets {"stations": 11}` for half-breadths, keel and sheer heights at the stations, so you can say what the moulds will look like before writing them.

## Export

1. Patterns: `export_templates {"path": "/tmp/moulds.pdf", "skin_mm": 6}` writes one station mould for each of 11 stations, tiled over A4 at full size. Give `stations`, `spacing` (metres) or `at` (a list of x) to place them; only one of the three.
2. Backbone and transom: `export_templates {"path": "/tmp/moulds.pdf", "skin_mm": 6, "profile": true, "transom_x": 0.0, "transom_rake_deg": 12}`. The transom is drawn in its own raked plane, so its shape is true; heights on it run up the slope.
3. A plotter or a CNC shop: `"single_page": true` gives one page a pattern; a `.dxf` path gives one drawing in millimetres with CUT, MARK and REFERENCE layers.
4. The demo hull at 1:1 on A4 is 822 pages for 11 moulds. A large hull at full size can pass the 1000-page limit; offer larger `paper`, `single_page`, or a DXF before a smaller `scale`, since a scaled pattern cannot be cut from.
5. The table of offsets for lofting by hand: `export_offsets_pdf {"path": "/tmp/offsets.pdf", "imperial": true}`.

## Read the reply

- `sheets` has a row a pattern: `x`, `half_breadth_m`, `face` and `max_bevel_deg`. `face` says which side of the mould goes on its station line so the mould needs no bevel to clear the skin: `forward` means the hull narrows going forward and the mould stands aft of its line; `aft` the other way; `either` where the hull is parallel. Where parts of a section narrow opposite ways, `face` follows the greater length and the sheet's notes say which face to bevel, and by up to how much, where the rest stands proud; relay that note.
- Relay every entry of `warnings`, word for word, with the sheet it names. "skin is thicker than the hull's local radius here" means that mould cannot be made at that skin as drawn; a station that "cuts nothing and was skipped" is missing from the set.
- `missing_characters`: letters the PDF font lacks, printed as `?`. Mention them if any.
- Each sheet's notes include "Longest straight segment … mm". If it is more than a few tens of millimetres on a curved section, the mesh is coarse: export again with a higher `level` (4 or 5) and compare.

## Tell the builder

- Print at 100 %, never "fit to page", and measure both 100 mm rules on every page before cutting. If either is not 100 mm, do not cut. The PDF prints these instructions beside its rules too.
- Join tiles on their alignment marks; the map page shows where each tile goes.
- The bevel figure is the largest angle along the section, not the bevel for the whole edge; the builder finds the local bevel with a batten across several moulds and planes each part to it before planking.
- Results are not validated (see `DISCLAIMER.md`): the patterns are what the program computed from this hull, not a checked set.

## Red flags

- Writing a pattern with `skin_mm` you did not get from the builder.
- Calling a pattern accurate, checked or ready to build. Say what it was computed from and what the builder must measure.
- Dropping or summarising away a warning.
- Changing `scale` to get under the page limit for a pattern the builder will cut from.
- Exporting before `hull_info` shows `validation.ok`.
- A strongback level below the sheer: the tool refuses it, because moulds stand on it upside down.
