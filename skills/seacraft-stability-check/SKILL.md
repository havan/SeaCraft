---
name: seacraft-stability-check
description: Use when you must report SeaCraft hydrostatics at a loading condition, a hydrostatic table, KN/GZ cross curves with free trim, IMO A.749 intact-stability results, or still-water resistance from Holtrop-Mennen or Savitsky planing.
---

# SeaCraft stability check

## Overview

Every figure here is measured on the refined mesh at one stated condition: draft, trim, heel, KG, density. A report without its condition is worthless. Fix the condition, measure, cross-check against a hand calculation, and report numbers with units and datum.

## Before the first call

Conventions: x forward, y to starboard, z up, metres, tonnes. Heel + to starboard. Trim is a length in metres, + bow up. KB, BM, KM, KG are above the keel; LCB, LCF are model x. Density defaults to 1025 kg/m3.
Demo hull at T = 0.62 m, KG 2.2 m: volume 11.85 m3, displacement 12.147 t, Cb 0.504, Cp 0.755, Cwp 0.823, KB 0.392 m, KM 2.62 m, GM 0.42 m, LCB x 4.297 m. `load_hull {"path": "hull.fef"}` if the server was not started with `--open`, then `hull_info {}` and check `validation.ok`, the layers with `use_in_hydrostatics`, and that a half model has its transom face.
Ask for, or state as an assumption, KG, the draft or displacement, the LCG, and the density.

## Hydrostatics at one condition

1. `hydro_compute {"draft": 0.62, "kg": 2.2}` for upright, even keel. Read `volume_m3`, `displacement_t`, `kb`, `bm_transverse`, `km_transverse`, `gm_m`, `lcb`, `lcf`, the coefficients, `waterplane_area_m2`, `wetted_surface_m2`, and the `warnings` list.
2. Heeled: add `"heel_deg": 10`. Free to trim, so LCB meets LCG: `"free_trim": true, "lcg": 4.3`. A heeled hull is free to trim; holding trim while heeling leaves LCB off LCG and over-reports KN and GZ.
3. `hydro_table {"steps": 5, "to": 1.0}` gives displacement, KB, KM, LCB, LCF, TPC and MTC per draft. Use it to pick the draft that matches a displacement.
4. Cross-check: GM = KM - KG (demo: 2.62 - 2.2 = 0.42). Displacement = volume x density / 1000.

## Cross curves and IMO A.749

`stability_cross_curves {"draft": 0.62, "kg": 2.2, "free_trim": true, "heels": [0, 10, 20, 30, 40, 60, 80], "displacements": [6, 12, 15]}`
- For a full loaded ship (lightship, deadweight items, and slack tanks with free-surface correction `FSC = FSM / Δ`), run `loading_condition {"case_path": "examples/demo-loadcase.json"}` (or pass `"case"` inline) to evaluate `kg_fluid`, `gm_fluid`, the free-trim floating condition and IMO A.749 criteria directly. If `equilibrium_found` is false, report its `warnings`: the figures are measured at the input draft/trim, not at an equilibrium.
- For the IMO 2008 IS Code Part A §2.3 severe wind and rolling (weather) criterion, run `weather_criterion {"draft": 0.62, "kg": 1.0, "free_trim": true, "bilge": "round"}` (optionally with `windage_extra` and `downflooding_deg`) to check steady-wind heel `phi0_deg <= 16` and gust energy balance `area_b_m_rad >= area_a_m_rad`.
- For deterministic damage stability by the lost-buoyancy method, run `damage_stability {"case_path": "examples/demo-damage.json"}` (or pass `"case"` inline) to evaluate damaged equilibrium (draft, trim, heel/list), check deck-edge freeboard, and assess residual GZ curves and indicative SOLAS-style criteria.
- `condition.lcg_x` is the LCG used (defaults to the LCB at the given draft, 4.297 m on the demo).
- `curves[].points[]` hold `kn`, `gz`, `draft` and `trim_m` per displacement. GZ at 0 degrees must be about 0 (1e-16 on the demo). If not, the condition is wrong.
- `imo_criteria[]` per displacement: `area_0_30_m_rad` (need >= 0.055), `area_0_40_m_rad` (>= 0.090), `area_30_40_m_rad` (>= 0.030), `max_gz_30_plus_m` (>= 0.20 m), `angle_of_max_gz_deg` (>= 25), `gm0_m` (>= 0.15), and `passed`.
- Demo hull with KG 2.2: GM0 is 0.61, 0.35 and 0.21 m at 6, 12 and 15 t, but every row fails (`passed` false, `max_gz_30_plus_m` negative): the hull has little freeboard and range at that KG. Report that as a finding, with the heel where GZ goes negative, not as an error.
- Name the weakest criterion per displacement. Change KG, never the criteria.

## Resistance

- `resistance_curve {"draft": 0.62, "speeds": [6, 8]}` (knots): `rt_n`, `pe_w`, `rf_n`, `rw_n`, `fn_`, `wave_branch`. Supports `method: "michell"` (with optional `cat_spacing` for catamarans) and DSYHS sailing yacht resistance (`method: "dsyhs"`) as well as Holtrop-Mennen (`method: "holtrop"`). Demo at 8 kn: Fn 0.40, RT 3627 N, Pe 14.9 kW. Read `warnings`: Holtrop-Mennen is for displacement hulls, Fn below about 0.5; Michell is for slender hulls (L/B >= 6, Fn 0.2-1.0); DSYHS is for sailing yacht bare hulls (Fn 0.15-0.75). Add `propeller_diameter` for wake and thrust deduction.
- `trim_scan {"speed_kn": 8, "from": -0.4, "to": 0.4, "steps": 9}` for the cheapest trim at constant displacement; `at_boundary: true` means widen the range.
- `planing_curve {"speeds": [20], "draft": 0.62}` (Savitsky): trim, wetted length, resistance. The demo is not a planing hull (deadrise measured 0.9 degrees); on it the 20 kn row gives 14.5 kN at 5.2 degrees, and means little. Use planing only for a hull with deadrise.

## Verify

- GZ(0) about 0, GM = KM - KG, displacement = volume x density / 1000.
- Same condition twice (`level` the same) gives the same numbers; changing `level` by one should move volume by well under 1 %.
- On the unedited demo the reference numbers above reproduce within 1 %.

## Report

State the condition (draft, trim, heel, KG, LCG, density, `level`), then displacement, KM, GM, LCB, the IMO table with margins, and warnings. Datum above keel for KB, KM, KG. Finish by producing `design_report {"path": "/tmp/report.html"}`.

## Red flags

- `free_trim` omitted for heeled cases, or `lcg` left implicit when the load is not at the LCB.
- KG measured from the deck or the waterline instead of the keel.
- A pass quoted without the heel range and the weakest margin.
- Using an open hull's stability beyond the deck-edge immersion angle, or quoting resistance with a warning ignored.

Written against SeaCraft 0.1.0-beta.36.
