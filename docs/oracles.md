# SeaCraft Numeric Oracles

This document records every numeric oracle used across the SeaCraft test suite:
the mathematical formula, the exact test function (`file::test_name`), the numbers
and parameters evaluated, and the provenance describing how the reference value
was produced.

Only facts stated in the codebase, code comments or `ATTRIBUTION.md` are documented.

Passing an oracle shows that the code agrees with that case, not that its results are right for a real
vessel; see [`DISCLAIMER.md`](../DISCLAIMER.md). Many of these derivations and scripts were produced with
AI coding assistants and then checked as described in each section. A value marked "not published" has
no published source behind it.

---

## 1. Analytic Box Displacement and Hydrostatics

### Formula
For an upright, creased rectangular cuboid barge of length $L$, beam $B$, depth $D$, floating at draft $T < D$ in water of density $\rho$:
- **Displaced volume**: $\nabla = L \cdot B \cdot T$
- **Displacement mass**: $\Delta = \rho \cdot \nabla$
- **Form coefficients**: $C_b = C_{wp} = C_m = C_p = 1.0$
- **Centre of buoyancy**: $KB = T / 2$, $LCB_x = L / 2$, $LCB_y = 0$
- **Centre of flotation**: $LCF_x = L / 2$, $LCF_y = 0$
- **Waterplane area**: $A_{wp} = L \cdot B$
- **Metacentric radii**:
  - Transverse: $BM_T = \frac{B^2}{12 T}$, $KM_T = KB + BM_T = \frac{T}{2} + \frac{B^2}{12 T}$
  - Longitudinal: $BM_L = \frac{L^2}{12 T}$
- **Wetted surface area**: $S = L \cdot B + 2 L T + 2 B T$
- **Midship section area**: $A_m = B \cdot T$

### Test Location
- [`crates/seacraft-core/tests/analytic.rs::a_closed_barge_has_exact_hydrostatics`](../crates/seacraft-core/tests/analytic.rs)

### Numbers Used
- $L = 10.0\text{ m}$, $B = 4.0\text{ m}$, $D = 4.0\text{ m}$, draft $T = 2.0\text{ m}$, $\rho = 1000.0\text{ kg/m}^3$.
- Displaced volume: $\nabla = 80.0\text{ m}^3$, $\Delta = 80.0\text{ t}$.
- Coefficients: $C_b = C_{wp} = C_m = C_p = 1.0000$.
- Buoyancy & Metacentre: $KB = 1.000\text{ m}$, $A_{wp} = 40.0\text{ m}^2$, $BM_T = 0.6667\text{ m}$, $BM_L = 4.1667\text{ m}$, $KM_T = 1.6667\text{ m}$.
- Centroids: $LCB = (5.0, 0.0, 1.0)\text{ m}$, $LCF = (5.0, 0.0)\text{ m}$.
- Areas: $S = 96.0\text{ m}^2$, $A_m = 8.0\text{ m}^2$.

### Provenance
Exact closed-form geometric integration for a rectangular cuboid. Edges are creased and corner points pinned so subdivision does not round corners.

---

## 2. Upright Stability of a Symmetric Box Barge

### Formula
For a box barge with transverse symmetry about the centreline $y = 0$:
- At upright condition ($\phi = 0^\circ$), righting lever $GZ = 0$ and cross-curve lever $KN = 0$.
- At positive heel $\phi > 0^\circ$, center of buoyancy shifts to leeward, producing positive righting lever $GZ > 0$.

### Test Location
- [`crates/seacraft-core/tests/analytic.rs::a_box_has_no_righting_lever_upright`](../crates/seacraft-core/tests/analytic.rs)

### Numbers Used
- Box $10.0 \times 4.0 \times 4.0\text{ m}$, $T = 2.0\text{ m}$, target volume $\nabla = 80.0\text{ m}^3$, $KG = 1.0\text{ m}$.
- Heel $0.0^\circ$: $GZ = 0.0\text{ m}$, $KN = 0.0\text{ m}$.
- Heel $10.0^\circ$: $GZ > 0.0\text{ m}$.

### Provenance
Transverse geometric symmetry of an upright rectangular barge.

---

## 3. Box-Barge Trim to LCB Equilibrium

### Formula
A rectangular box barge of length $L$, beam $B$, draft $d$ floating with bow-up trim has submerged local depth $h(x) = d + \text{trim}(1/2 - x/L)$.
Displaced volume is invariant to trim: $\nabla = L \cdot B \cdot d$.
Centroid of submerged volume along $x$:
$$\bar{x} = \frac{L}{2} - \text{trim} \cdot \frac{L}{12 d}$$
Inverting to solve for the equilibrium trim required to position $LCB$ at target $LCG = \bar{x}$:
$$\text{trim} = \left(\frac{L}{2} - LCG\right) \cdot \frac{12 d}{L}$$

### Test Location
- [`crates/seacraft-core/tests/analytic.rs::free_trim_moves_the_lcb_to_the_requested_station`](../crates/seacraft-core/tests/analytic.rs)

### Numbers Used
- Box $L = 10.0\text{ m}$, $B = 4.0\text{ m}$, $D = 4.0\text{ m}$, $d = 2.0\text{ m}$.
- Target $LCG = 4.0\text{ m}$.
- Analytic trim: $(5.0 - 4.0) \cdot 12 \cdot 2.0 / 10.0 = 2.400\text{ m}$ bow-up.
- Equilibrium draft: $2.000\text{ m}$.
- Symmetry check: at heel $20^\circ$ and $LCG = 5.0\text{ m}$, trim must be $0.0\text{ m}$.

### Provenance
Analytic 1D moment integration of a trapezoidal submerged waterplane on a prismatic rectangular geometry.

---

## 4. Free-to-Trim Stability Cross Curves Invariance

### Formula
Non-linear two-unknown solve for $(T, \text{trim})$ ensuring displaced volume matches target displacement and $LCB_x = LCG$.
For a non-prismatic hull with a wide transom and fine bow, heeling moves the centre of buoyancy aft, causing trim. Holding trim fixed leaves $LCB$ away from $LCG$ and overstates $GZ$; allowing the hull to trim freely yields the true, lower equilibrium righting lever $GZ_{\text{free}} < GZ_{\text{fixed}}$.

### Test Location
- [`crates/seacraft-core/tests/analytic.rs::free_trim_cross_curves_hold_the_lcb_on_the_lcg`](../crates/seacraft-core/tests/analytic.rs)

### Numbers Used
- Demo hull at $T = 0.62\text{ m}$, heels $[0^\circ, 10^\circ, 20^\circ, 30^\circ]$, $LCG = 4.297\text{ m}$ (upright LCB).
- At heel $\ge 20^\circ$: $|\text{trim}| > 0.005\text{ m}$ and $GZ_{\text{free}} < GZ_{\text{fixed}}$.

### Provenance
Multi-dimensional root finding equilibrium invariant on refined surface mesh.

---

## 5. Closed Box High Heel Stability (Singularity Avoidance)

### Formula
Deriving cutting waterplane orientation from $(\sin\phi, \cos\phi)$ avoids the $\tan\phi$ singularity at $\phi = 90^\circ$. For a completely closed box, righting lever $GZ$ and cross curve $KN$ remain finite and continuous at high heel angles.

### Test Location
- [`crates/seacraft-core/tests/analytic.rs::closed_box_cross_curve_at_75_and_90_degrees_is_finite`](../crates/seacraft-core/tests/analytic.rs)

### Numbers Used
- Box $10.0 \times 4.0 \times 4.0\text{ m}$ at draft $2.0\text{ m}$.
- Heel angles: $75.0^\circ$ and $90.0^\circ$. Both yield finite $GZ$ and $KN$.

### Provenance
Trigonometric formulation verification for extreme angles.

---

## 6. Demo Hull Reference Hydrostatics (Project Regression Oracle)

### Formula
Panel divergence divergence integration over triangular panels of the refined limit mesh (Loop subdivision mode, level 3) at design draft $T = 0.62\text{ m}$, water density $\rho = 1025.0\text{ kg/m}^3$, appendage coefficient $1.0$, $11$ stations.

### Test Locations
- [`crates/seacraft-core/tests/analytic.rs::demo_hull_matches_the_reference_numbers`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-gui/tests/hydro.rs::the_dock_reports_the_cli_reference_numbers`](../crates/seacraft-gui/tests/hydro.rs)
- [`crates/seacraft-core/src/table.rs::a_row_matches_the_single_point_measurement`](../crates/seacraft-core/src/table.rs)
- `xtask/src/main.rs` (CI gate)

### Numbers Used
- **Volume $\nabla$**: $11.851\text{ m}^3$ (tolerance $1.0\%$)
- **Block coefficient $C_b$**: $0.5042$ (tolerance $1.0\%$)
- **Prismatic coefficient $C_p$**: $0.7548$ (tolerance $1.0\%$)
- **Midship coefficient $C_m$**: $0.6680$ (tolerance $1.0\%$)
- **Waterplane coefficient $C_{wp}$**: $0.8235$ (tolerance $1.0\%$)
- **Vertical centre of buoyancy $KB$**: $0.392\text{ m}$ (tolerance $1.0\%$)
- **Transverse metacentre $KM$**: $2.625\text{ m}$ (tolerance $1.0\%$)
- **Midship area $A_m$**: $1.467\text{ m}^2$ (tolerance $1.0\%$)

### Provenance
SeaCraft's own output for the built-in demo hull, frozen as a regression baseline (not an independent or FREE!ship-computed value; provenance of the original numbers: not recorded). Recorded in `xtask/src/main.rs` as the project's primary regression gate: any change altering these metrics by $> 1\%$ is a regression.

---

## 7. Wigley Michell Wave Resistance Quadrature Reference

### Formula
Michell thin-ship wave resistance integral:
$$R_w = \frac{4 \rho g^2}{\pi U^2} \int_0^{\pi/2} (P^2 + Q^2) \sec^3\theta \, d\theta$$
For the parabolic Wigley hull $y = \pm \frac{B}{2} (1 - (2x/L)^2)(1 - (z/T)^2)$:
$P = 0$ due to fore-aft symmetry, while $Q$ evaluates by exact separable quadrature in $x$ and $z$:
$$I_x = -\frac{L/2}{k_x} \cos\left(k_x \frac{L}{2}\right) + \frac{1}{k_x^2} \sin\left(k_x \frac{L}{2}\right)$$
$$I_z = \frac{1}{k_z} - \frac{2}{k_z (k_z T)^2} + \left(\frac{2}{k_z (k_z T)} + \frac{2}{k_z (k_z T)^2}\right) e^{-k_z T}$$
with $k_x = k_0 \sec\theta$, $k_z = k_0 \sec^2\theta$, $k_0 = g / U^2$.

### Test Location
- [`crates/seacraft-core/src/michell.rs::test_wigley_hull_matches_independent_quadrature_and_self_consistency`](../crates/seacraft-core/src/michell.rs)
- [`crates/seacraft-core/src/michell.rs::wigley_analytic_rw`](../crates/seacraft-core/src/michell.rs)

### Numbers Used
- Hull: $L = 10.0\text{ m}$, $B = 1.0\text{ m}$, $T = 0.625\text{ m}$ ($L/B = 10$, $B/T = 1.6$).
- Physical constants: $\rho = 1000.0\text{ kg/m}^3$, $g = 9.80665\text{ m/s}^2$.
- Numerical grid: $65 \times 25$ offsets, $n_\theta = 128$.
- Independent reference $C_w = R_w / (\frac{1}{2}\rho U^2 S)$:
  - $Fn = 0.25$: $C_w = 1.064 \times 10^{-3}$
  - $Fn = 0.30$: $C_w = 2.142 \times 10^{-3}$ (interference hump)
  - $Fn = 0.35$: $C_w = 1.248 \times 10^{-3}$ (hollow)
  - $Fn = 0.40$: $C_w = 2.734 \times 10^{-3}$
  - $Fn = 0.50$: $C_w = 4.517 \times 10^{-3}$ (main hump)
- The module's $C_w$ agrees with these within $1\%$ (0.3 % in fact), with $S = 14.879\text{ m}^2$ from
  `wigley_grid`, the exact Wigley wetted surface (see `wigley_grid_wetted_surface_matches_kajitani` below).
  Until 2026-11-18 `wigley_grid` integrated $S$ with a wrong slope $f_x$ (twice too large, without its
  $(1 - z^2/T^2)$ factor), giving $15.14\text{ m}^2$ and $C_w$ 1.8 % low; $R_w$ was unaffected.
- Grid result matches 1D analytical quadrature `wigley_analytic_rw` within $1\%$.
- $C_w(Fn = 0.10)$ converges to $9.997 \times 10^{-5}$ (513 × 97 grid, 8192 $\theta$ points); the test bounds it
  by $1.5 \times 10^{-4}$.
- `wigley_grid_wetted_surface_matches_kajitani`: $S$ within 0.1 % of $0.661\,L(2T + B) = 14.87\text{ m}^2$, the
  definition printed in Kajitani et al. (1983, Nomenclature), and within 0.01 % of a $400 \times 200$
  Gauss-Legendre quadrature of $2\iint\sqrt{1 + f_x^2 + f_z^2}\,dx\,dz$ ($14.8791\text{ m}^2$, $C_s = 0.66129$).

### Provenance
Independent numerical quadrature using variable substitution $\lambda = \sec\theta = \cosh u$ ($u \in [0, 6]$) over an $801 \times 201$ offset grid using the trapezoidal rule, computed specifically for this regression check. As documented in the module comments, these are not published experimental or tabulated values.

The comparison with published experiment (Kajitani et al., 1983) and with a published independent computation
(Peri, 2026) is §39. The far-field wave pattern built on the same $P + iQ$ is checked in §40.

---

## 8. DSYHS Yacht Residuary Resistance Polynomial Cross-Check

### Formula
Delft Systematic Yacht Hull Series (DSYHS Keuning & Katgert 2008, Table 2):
$$R_r = \Delta \cdot g \cdot \frac{\nabla^{1/3}}{L_{WL}} \cdot P(x_1, \dots, x_7)$$
where $P = a_0 + \sum_{i=1}^7 a_i x_i$, with dimensionless parameters:
- $x_1 = LCB / LWL$ (from FPP, fraction of $LWL$, positive aft)
- $x_2 = C_p$
- $x_3 = \nabla^{2/3} / A_w$
- $x_4 = BWL / LWL$
- $x_5 = LCB / LCF$
- $x_6 = BWL / T_c$
- $x_7 = C_m$

### Test Locations
- [`crates/seacraft-core/src/resistance.rs::dsyhs_polynomial_at_tabulated_fn_matches_hand_arithmetic`](../crates/seacraft-core/src/resistance.rs)
- [`crates/seacraft-core/src/resistance.rs::dsyhs_interpolation_midway_between_fn_rows`](../crates/seacraft-core/src/resistance.rs)
- Coefficient table comments in [`crates/seacraft-core/src/resistance.rs#L792-L809`](../crates/seacraft-core/src/resistance.rs)

### Numbers Used
- Sample hull: $L_{WL} = 10.0\text{ m}$, $B_{WL} = 3.0\text{ m}$, $T_c = 0.50\text{ m}$, $\nabla = 5.0\text{ m}^3$, $A_w = 18.0\text{ m}^2$, $S_w = 25.0\text{ m}^2$, $C_p = 0.55$, $C_m = 0.70$, $LCB / LWL = 0.52$, $LCF / LWL = 0.56$, $\rho = 1025.0\text{ kg/m}^3$, $g = 9.80665\text{ m/s}^2$.
- At $Fn = 0.40$:
  - Dimensionless inputs: $x_1 = 0.52$, $x_2 = 0.55$, $x_3 \approx 0.162445$, $x_4 = 0.30$, $x_5 = 13/14 \approx 0.928571$, $x_6 = 6.0$, $x_7 = 0.70$.
  - Coefficients: $a = [-0.0064, -0.4034, -0.1250, 0.0273, -0.1341, 0.3578, 0.0045, 0.1115]$.
  - Polynomial value $P = 0.116579612$.
  - Scaling factors: $\nabla^{1/3}/L_{WL} = 0.17099759$, $\Delta g = 50259.08125\text{ N}$.
  - Expected residuary resistance $R_r = 1001.90645\text{ N}$.

### Provenance
Coefficients from Table 2 of J. A. Keuning and M. Katgert (2008), "A bare hull resistance prediction method derived from the results of the Delft Systematic Yacht Hull Series extended to higher speeds", InnovSail '08.
Cross-checked against Pedro Teixeira (2014) MSc thesis (IST Lisbon), Appendix IV, Table IV-1: 103 of 104 coefficients match identically; $a_3$ at $Fn = 0.55$ is $0.8762$ in the thesis versus $0.8726$ in Keuning & Katgert (2008). Test verifies polynomial evaluation against step-by-step arithmetic hand calculation.

---

## 9. Deterministic Damage Stability: Box Barge Sinkage

### Formula
For a box barge $L \times B \times D$ at initial draft $T$, damaged by flooding a full-breadth midship compartment of length $l$ and permeability $\mu$ open to the sea and extending above the waterline:
- Intact displaced volume: $V_0 = L \cdot B \cdot T$
- Lost buoyancy volume: $\mu \cdot l \cdot B \cdot T'$
- Equilibrium buoyant volume: $(L - \mu \cdot l) \cdot B \cdot T' = V_0 = L \cdot B \cdot T$
- Damaged equilibrium draft:
  $$T' = T \cdot \frac{L}{L - \mu \cdot l}$$
- With midship and symmetric flooding: $\text{trim} = 0.0\text{ m}$, $\text{heel} = 0.0^\circ$.

### Test Location
- [`crates/seacraft-core/src/damage.rs::box_barge_lost_buoyancy_sinkage_matches_analytic_derived_oracle`](../crates/seacraft-core/src/damage.rs)

### Numbers Used
- $L = 10.0\text{ m}$, $B = 4.0\text{ m}$, $D = 2.0\text{ m}$, initial draft $T = 0.80\text{ m}$.
- Compartment: length $l = 2.0\text{ m}$, permeability $\mu = 0.95$.
- Analytic damaged draft: $T' = 0.80 \cdot 10.0 / (10.0 - 0.95 \cdot 2.0) = 8.0 / 8.1 \approx 0.987654\text{ m}$.
- Asserted within $0.5\%$ relative tolerance; trim $< 10^{-3}\text{ m}$, heel $< 10^{-3\circ}$.

### Provenance
Direct algebraic derivation from the lost-buoyancy principle for prismatic rectangular box hulls.

---

## 10. Tank Free-Surface Moment and Virtual KG Correction

### Formula
For an upright rectangular tank of length $l$ and breadth $b$:
- **Transverse waterplane second moment**: $I_T = \frac{l \cdot b^3}{12}$
- **Free-surface moment**: $\text{FSM} = \rho_{\text{fluid}} \cdot I_T = \rho_{\text{fluid}} \cdot \frac{l \cdot b^3}{12}$
- **Free-surface correction**: $\text{FSC} = \frac{\sum \text{FSM}}{\Delta}$
- **Virtual KG**: $KG_{\text{fluid}} = KG_{\text{solid}} + \text{FSC}$
- For empty ($\text{fill} = 0$) or pressed-up ($\text{fill} = 1$) tanks: $\text{FSM} = 0$.
*(Rule note: FSM is not reduced by permeability, maintaining a conservative margin).*

### Test Locations
- [`crates/seacraft-core/src/loading.rs::box_tank_volume_and_centroid_at_half_fill`](../crates/seacraft-core/src/loading.rs)
- [`crates/seacraft-core/src/loading.rs::rectangular_tank_fsm_is_exact_and_zero_when_empty_or_pressed_up`](../crates/seacraft-core/src/loading.rs)
- [`crates/seacraft-core/src/loading.rs::kg_of_items_is_mass_weighted_mean_and_fsc_is_fsm_over_displacement`](../crates/seacraft-core/src/loading.rs)

### Numbers Used
- Tank: $l = 4.0\text{ m}$, $b = 2.4\text{ m}$, $h = 1.5\text{ m}$, $\rho = 1.025\text{ t/m}^3$, permeability $0.98$.
  - $I_T = 4.0 \cdot (2.4)^3 / 12 = 4.608\text{ m}^4$.
  - $\text{FSM} = 1.025 \cdot 4.608 = 4.7232\text{ t}\cdot\text{m}$ for slack fills ($0.1, 0.5, 0.9$).
  - At fill $0.0$ and $1.0$: $\text{FSM} = 0.0$.
- In multi-item test: tank $l = 2.0\text{ m}$, $b = 1.2\text{ m}$, $\rho = 1.0\text{ t/m}^3 \implies \text{FSM} = 0.288\text{ t}\cdot\text{m}$.

### Provenance
Classical naval architecture formula for second moment of area of a rectangular free water surface.

---

## 11. IMO Weather Criterion (IS Code Part A §2.3)

### Formula
IMO 2008 Intact Stability Code Part A §2.3:
- **Wind heeling levers**:
  $$l_{w1} = \frac{P \cdot A \cdot Z}{1000 \cdot g \cdot \Delta}\text{ (m)}, \quad l_{w2} = 1.5 \cdot l_{w1}$$
  where $P = 504\text{ Pa}$, $A$ is above-water lateral projected area, $Z$ is lever between centroids of windage and underwater lateral area.
- **Roll period**:
  $$T_{\text{roll}} = \frac{2 \cdot C \cdot B}{\sqrt{GM}}$$
  with $C = 0.373 + 0.023(B/d) - 0.043(L_{WL}/100)$.
- **Rollback angle**:
  $$\phi_1 = 109 \cdot k \cdot X_1 \cdot X_2 \cdot \sqrt{r \cdot s}\text{ (deg)}$$
  with $r = 0.73 + 0.6(OG/d)$, $OG = KG - d$, and empirical table factors $X_1(B/d)$, $X_2(C_b)$, $s(T_{\text{roll}})$, $k(\text{bilge})$.
- **Area integration**:
  - $\text{Area } a$: integral of $(l_{w2} - GZ)$ from $\phi_{\text{start}} = \phi_0 - \phi_1$ to equilibrium $\phi_c$.
  - $\text{Area } b$: integral of $(GZ - l_{w2})$ from $\phi_c$ to downflooding/deck angle $\phi_2$.
  - Criterion: $\text{Area } b \ge \text{Area } a$.

### Test Locations
- [`crates/seacraft-core/src/weather.rs::roll_period_and_phi1_hand_computed`](../crates/seacraft-core/src/weather.rs)
- [`crates/seacraft-core/src/weather.rs::box_barge_windage_and_lw1_match_analytic`](../crates/seacraft-core/src/weather.rs)
- [`crates/seacraft-core/src/weather.rs::synthetic_sine_gz_areas_match_analytic_within_one_percent`](../crates/seacraft-core/src/weather.rs)

### Numbers Used
1. Roll period & $\phi_1$ case:
   - $B = 10.0\text{ m}$, $d = 4.0\text{ m}$ ($B/d = 2.5 \implies X_1 = 0.98$), $L_{WL} = 50.0\text{ m}$, $C_b = 0.55 \implies X_2 = 0.89$.
   - $KG = 4.4\text{ m} \implies OG = 0.4\text{ m} \implies r = 0.79$, $GM = 1.00\text{ m}$.
   - $C = 0.409$, $T = 8.18\text{ s}$, $s(8.18) = 0.09174$, $k = 1.0$.
   - $\phi_1 = 109 \cdot 1.0 \cdot 0.98 \cdot 0.89 \cdot \sqrt{0.79 \cdot 0.09174} \approx 25.59^\circ$.
2. Box barge windage case:
   - Box $10.0 \times 4.0 \times 3.0\text{ m}$, draft $d = 1.2\text{ m}$.
   - Underwater lateral area: $10 \times 1.2 = 12.0\text{ m}^2$, $z = 0.6\text{ m}$.
   - Above-water windage: $10 \times 1.8 = 18.0\text{ m}^2$, $z = 2.1\text{ m}$.
   - Lever $Z = 2.1 - 0.6 = 1.5\text{ m}$.
   - Displacement $\Delta = 49.2\text{ t} \implies l_{w1} = 0.02816\text{ m}$, $l_{w2} = 0.04224\text{ m}$.
3. Synthetic sine $GZ$ areas:
   - $GZ(\phi) = GM \sin\phi$ ($GM = 1.0\text{ m}$).
   - $l_{w1} = 0.08\text{ m}$, $l_{w2} = 0.12\text{ m}$.
   - Exact trigonometric integrals evaluated analytically:
     $$\text{Area } a = l_{w2}(\phi_c - \phi_{\text{start}}) + GM(\cos\phi_c - \cos\phi_{\text{start}})$$
     $$\text{Area } b = GM(\cos\phi_c - \cos\phi_2) - l_{w2}(\phi_2 - \phi_c)$$
   - Numerical integration agrees with analytic solution within $1\%$.

### Provenance
Formulas from IMO 2008 IS Code Part A §2.3 / Resolution MSC.267(85). Hand calculation and exact trigonometric integration.

---

## 12. Linear GZ Curve IMO Area Integration

### Formula
For a linear righting arm curve $GZ(\phi) = k \cdot \phi$ (with $\phi$ in degrees):
$$\text{Area}_{a \to b} = \int_{\theta_a}^{\theta_b} k \left(\frac{180}{\pi}\theta\right) d\theta = \frac{k}{2} (\phi_b^2 - \phi_a^2) \cdot \frac{\pi}{180^\circ}$$
Because $GZ(\phi)$ is linear, trapezoidal quadrature is exact.
Initial metacentric height: $GM_0 = k / (\pi / 180)$.

### Test Location
- [`crates/seacraft-core/src/stability.rs::imo_areas_are_exact_for_a_linear_gz_curve`](../crates/seacraft-core/src/stability.rs)

### Numbers Used
- $k = 0.01\text{ m/deg}$, heels from $0^\circ$ to $90^\circ$ in $10^\circ$ steps.
- $\text{Area}_{0 \to 30} = 0.07854\text{ m}\cdot\text{rad}$.
- $\text{Area}_{0 \to 40} = 0.13963\text{ m}\cdot\text{rad}$.
- $\text{Area}_{30 \to 40} = 0.06109\text{ m}\cdot\text{rad}$.
- Max $GZ = 0.90\text{ m}$ at $90^\circ$.
- $GM_0 = 0.572958\text{ m}$.

### Provenance
Exact algebraic integration of a linear function.

---

## 13. Three-Point Parabolic GZ Peak Interpolation

### Formula
Fitting a second-order parabola through 3 consecutive points around the discrete maximum:
$$GZ(\phi) = a \phi^2 + b \phi + c$$
yields vertex location $\phi_{\text{max}} = -b / (2a)$ and interpolated maximum $GZ_{\text{max}} = c - b^2 / (4a)$.

### Test Location
- [`crates/seacraft-core/tests/analytic.rs::parabola_interpolated_angle_of_max_gz_on_synthetic_curve`](../crates/seacraft-core/tests/analytic.rs)

### Numbers Used
- Synthetic test parabola: $GZ(\phi) = 0.55 - 0.001 \cdot (\phi - 33.5)^2$.
- Sampled at $10^\circ$ intervals: $\phi \in [0^\circ, 80^\circ]$.
- Analytical peak: $\phi = 33.5^\circ$, $GZ_{\text{max}} = 0.55\text{ m}$.

### Provenance
Synthetic algebraic test function.

---

## 14. Box Barge Bonjean Section Areas

### Formula
For a rectangular box barge of constant breadth $B$ and depth $D$, underwater cross-sectional area at longitudinal station $x$ and draft $d \le D$:
$$A(x, d) = B \cdot d$$
Prismatic coefficient $C_p = \nabla / (L \cdot A_m) = 1.0$.

### Test Locations
- [`crates/seacraft-core/src/curves.rs::bonjean_on_a_box_is_width_times_draft`](../crates/seacraft-core/src/curves.rs)
- [`crates/seacraft-core/src/curves.rs::prismatic_on_a_box_is_one`](../crates/seacraft-core/src/curves.rs)

### Numbers Used
- $L = 10.0\text{ m}$, $B = 4.0\text{ m}$, $D = 3.0\text{ m}$.
- Drafts: $[0.5, 1.0, 1.5, 2.0, 2.5, 3.0]\text{ m}$.
- Section area $A = 4.0 \cdot d\text{ m}^2$ at all internal stations.

### Provenance
Exact cross-sectional geometry of rectangular prisms.

---

## 15. 2D Heave Added Mass and Damping of Lewis Sections (B7a)

### Formula
Deep-water radiation problem for a heaving section, `exp(-iωt)`, solved with Frank's close-fit source method:
force = −iωρ∮φ n_z ds = iω a₃₃ − b₃₃, and b₃₃ = ρg²Ā²/ω³. Infinite-frequency limit (Lewis 1929):
a₃₃∞ = ½ρπM_s²((1+a₁)² + 3a₃²).

### Test Locations
- [`crates/seacraft-core/src/seakeeping.rs::heave_coefficients_match_the_independent_python_solutions`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::infinite_frequency_added_mass_matches_lewis_closed_form`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::panel_count_has_converged_and_damping_is_positive`](../crates/seacraft-core/src/seakeeping.rs)

### Numbers Used
Semicircle (b = T = 1 m) and a Lewis section b = 1.5 m, T = 1 m, σ = 0.9, at ξ = ω²b/g ∈ {0.05, 0.25, 0.5, 0.75, 1.0, 1.5};
tolerances 1.5 % (added mass, Ā) and 2 % (damping).

### Provenance
Computed with [`docs/oracles/heave_2d.py`](oracles/heave_2d.py) by two independent methods: Frank panels
(60 vs 120 panels agree to 0.2 %; ∞-frequency semicircle 1.006 × ρπb²/2) and, for the semicircle, Ursell's wave
source + wave-free multipoles. The two agree to 0.3 % in added mass and 0.6 % in damping for ξ ≤ 1.5. The wave-source
kernel was checked against direct numerical principal-value integration. ξ = 2 is not used: it is near the panel
method's first irregular frequency. Not published values.

---

## 16. Strip-Theory Heave/Pitch RAOs: Physical Limits (B7b)

### Formula
Coupled heave–pitch in head seas (STF 1970, pitch bow up). Checks that need no published data:
very long waves are followed (heave → 1; pitch → ∫2b·x² / (∫2b·x² + ∇(KB − KG)), = 1 when KG = KB) at any speed;
short waves give RAOs → 0; a box barge has C33 = ρgLB and C55 = ρgBL³/12 + ρg∇(KB − KG); a fore–aft symmetric box
at U = 0 has no heave–pitch coupling; the speed terms satisfy A35 − A53 = 2U/ωe²·B33 and B35 − B53 = −2U·A33.

### Test Locations
- [`crates/seacraft-core/src/seakeeping.rs::very_long_waves_are_followed_at_any_speed`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::test_box_barge_hydrostatic_stiffness`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::test_stf_speed_structure`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::test_symmetric_box_cross_coupling_vanishes`](../crates/seacraft-core/src/seakeeping.rs)

### Provenance
Analytic limits. The speed-term signs were derived from the relative-motion sectional force
−D/Dt[(a + b/iω_e)·V], D/Dt = ∂t − U∂x, integrated by parts; the long-wave test caught the original swapped signs.
The demo hull's KG = 2.2 m limit (≈ 1.10) is BM_L/GM_L from `seacraft hydro`. Not published values.

---

## 17. Irregular Head Seas: Spectra, Significant Responses, MSI (E2)

### Formula
Bretschneider (ITTC two-parameter) S(ω) = 5/16·Hs²·ωp⁴·ω⁻⁵·exp(−5/4·(ωp/ω)⁴); its moments are
m_n = (A/4)·B^((n−4)/4)·Γ(1 − n/4) with A = 5/16·Hs²·ωp⁴, B = 5/4·ωp⁴, so m0 = Hs²/16, T1 = 0.7718·Tp,
Tz = 0.7104·Tp. JONSWAP multiplies by γ^r (σ = 0.07/0.09) and is rescaled numerically to m0 = Hs²/16.
Responses: m_n = ∫|H(ω)|²·ω_eⁿ·S(ω) dω over the incident frequency, ω_e = ω + ω²U/g; significant amplitude 2√m0.
Point motion s = η3 + x_r·η5 (pitch bow up), relative motion s − e^{ikx_r}, acceleration −ω_e²·s.
MSI (O'Hanlon & McCauley 1974, 2 h): 100·Φ((log10(a/g) − μ)/0.4), μ = −0.819 + 2.32·(log10 ω)², a = 0.798·σ_a.

### Test Locations
- [`crates/seacraft-core/src/seastate.rs`](../crates/seacraft-core/src/seastate.rs): `bretschneider_moments_match_the_oracle`,
  `jonswap_gamma_one_is_bretschneider_and_normalised`, `flat_rao_gives_half_hs_at_any_speed`,
  `resonant_synthetic_rao_matches_the_oracle`, `msi_matches_the_oracle`, `long_waves_are_followed_on_the_demo_hull`,
  `moderate_sea_on_the_demo_hull_is_finite_and_bow_is_worse`.

### Oracle values
Recipe: [`docs/oracles/irregular_seas.py`](oracles/irregular_seas.py) — run with `python3 -I irregular_seas.py`
(scipy quadrature, independent of the Rust grid integration).

| Quantity (Bretschneider Hs = 2 m, Tp = 8 s) | Value |
|---|---|
| m0 / m1 / m2 | 0.250000 / 0.254414 / 0.305597 |
| JONSWAP γ = 3.3 normalisation (Goda: 1 − 0.287 ln γ) | 0.6558 (0.6573) |
| Resonant RAO (ωn 1.2, ζ 0.2, U 3 m/s): m0 / m2 / m4 | 0.825369 / 1.050609 / 1.590609 |
| MSI (a/g, ω) → %: (0.10, 1.0) (0.05, 1.0) (0.20, 1.0) (0.10, 0.5) (0.10, 2.0) (0.02, 1.2) | 32.545, 11.409, 61.794, 16.401, 16.401, 1.267 |

### Provenance
Analytic moments and a hand-checkable normal-CDF formula, computed with scipy. The relative-motion
sign rests on the RAO phases: in long waves pitch leads heave by ≈ +90° (λ/L = 8: +92.8°), i.e. the hull
follows the wave slope, so s ≈ e^{ikx_r}. Not published values.

---

## 18. Irregular-frequency lid and transom terms (E3)

### Formula
**Lid.** The Frank source method on the hull alone has irregular frequencies (the eigenfrequencies of the interior
problem below the free surface). Ohmatsu (1975) closes the waterline from inside with source panels on z = 0 between
the waterline and the centreline (`nlid = max(4, ⌈b_wl / mean hull panel length⌉)`), with ∂φ/∂z = 0 on the lid; forces
are integrated over the hull only. At z = 0 the Rankine part ln r₁ − ln r₂ cancels; the lid self panel uses
Σ w(G − 2 ln|Δy|) + 2L(ln(L/2) − 1) for the potential and −2π + Σ w ∂G/∂z for the normal derivative. A `φ = 0` lid
(Dirichlet) was tried and diverges with refinement. K = ∞ has none, so the lid is skipped there.

**Transom terms** (pitch bow up, x forward from the LCG, `e^{iω_e t}`, m = a − i b/ω_e; x_A, a_A, b_A the aft end
and the 2-D coefficients of the aftmost station, zero for a closed stern). The transom part of −D/Dt[m(w − v)]
integrated over x is U·m_A·(v_A − w_A), w = iω_e(η₃ + xη₅) − Uη₅, v_A = iω κ_A e^{ik x_A}. Radiation (w_A):

| | A | B |
|---|---|---|
| 33 | −U b_A/ω_e² | U a_A |
| 35 | (−U x_A b_A + U² a_A)/ω_e² | U x_A a_A + U² b_A/ω_e² |
| 53 | −U x_A b_A/ω_e² | U x_A a_A |
| 55 | (−U x_A² b_A + U² x_A a_A)/ω_e² | U x_A² a_A + U² x_A b_A/ω_e² |

Excitation (v_A): ΔF₃ = U m_A v_A, ΔF₅ = x_A ΔF₃. In very long waves the hull follows the wave (w → v), so the two
terms cancel: `very_long_waves_are_followed_at_any_speed` (and the version with the terms explicitly on at 0/3/6 kn)
is the sign check. A closed stern (a_A = b_A = 0) is unchanged by the terms.

### Test Locations
- [`crates/seacraft-core/src/seakeeping.rs::lid_removes_the_irregular_frequencies_of_the_semicircle`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::transom_terms_vanish_for_a_closed_stern`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::transom_coefficient_changes_match_the_formulas`](../crates/seacraft-core/src/seakeeping.rs)
- [`crates/seacraft-core/src/seakeeping.rs::long_waves_are_followed_with_transom_terms_at_any_speed`](../crates/seacraft-core/src/seakeeping.rs)
- `very_long_waves_are_followed_at_any_speed` (§16) and `test_stf_speed_structure` (run with the transom terms off:
  the structure A35 − A53 = 2U/ω_e²·B33 holds for the strip integrals, not for the boundary terms).

### Oracle values
Recipe: [`docs/oracles/heave_2d_lid.py`](oracles/heave_2d_lid.py) — run with `python3 -I docs/oracles/heave_2d_lid.py`
(numpy + scipy). Semicircle R = 1, 60 hull panels, columns a/(ρπR²/2) and b/(ρπR²/2·ω):

| ξ = KR | Ursell (a, b) | lid (a, b) | plain (a, b) |
|---|---|---|---|
| 0.50 | 0.6479 0.8140 | 0.6487 0.8129 | 0.6480 0.8120 |
| 1.00 | 0.6084 0.3973 | 0.6093 0.3974 | 0.6085 0.3952 |
| 1.50 | 0.6687 0.2120 | 0.6692 0.2128 | 0.6679 0.2080 |
| 1.82 | 0.7096 0.1470 | 0.7098 0.1479 | 0.2895 0.3839 |
| 2.50 | 0.7801 0.0729 | 0.7798 0.0739 | 0.7816 0.0742 |
| 3.00 | 0.8178 0.0461 | 0.8173 0.0471 | 0.8190 0.0467 |
| 4.00 | 0.8684 0.0206 | 0.8676 0.0214 | 0.8695 0.0208 |
| 4.78 | 0.8937 0.0120 | 0.8928 0.0127 | 0.8780 0.0284 |

The Rust lid result is asserted within 2 % (a) and 3 % or 0.002 absolute (b) of the Ursell columns at every ξ; the
plain method (`lid = false`) at ξ = 1.82 must be more than 30 % off in a.

### Provenance
Lid oracle and the transom formulas derived and checked for this project (2026-10-23); not published values. Ohmatsu, "On
the irregular frequencies in the theory of oscillating bodies in a free surface", Papers Ship Res. Inst. 48 (1975).

## 19. Round-3 fixes: silhouette windage, Savitsky CL0, catamaran friction, loop signs

### Formulas
- Lateral windage is the side silhouette: at each station `x` the hull spans `[z_lo(x), z_hi(x)]`;
  above-water area `A = ∫ (z_hi − max(z_lo, z_wl(x)))⁺ dx`, centroid `∫ ½(top² − bottom²) dx / A`, and the same
  below the waterline with `min(z_hi, z_wl)`. Trapezoidal rule over 400 stations.
- Savitsky (1964): `C_Lβ = C_L0 − 0.0065 β C_L0^0.6`, solved for `C_L0` by `C_L0 ← C_Lβ + 0.0065 β C_L0^0.6`.
- Michell with a catamaran spacing: `R_W` is for both demihulls, so `R_F = ½ρV²·(2S)·C_F` and `C_W` uses `2S`.
- Waterplane/section loops: sign `(−1)^depth` from nesting (even-odd containment of each loop's first point).

### Tests
- [`weather.rs::windage_is_the_side_silhouette_not_the_centreline_cut`](../crates/seacraft-core/src/weather.rs) — a
  10 m × 4 m barge with a deck dipping from z = 3 (sides) to 2.5 (centreline), unrefined net, d = 1.2 m: A = 18 m²,
  z = 2.1 m, underwater 12 m² at 0.6 m (the centreline cut would give 13 m²). Trimmed 1 m by the bow: underwater
  centroid `10 (1.7³ − 0.7³) / 6 / 12` = 0.63472 m.
- [`planing.rs::flat_plate_lift_matches_a_hand_solution`](../crates/seacraft-core/src/planing.rs) — β = 10°,
  C_Lβ = 0.05 → C_L0 = 0.062290 (checked in Python: 0.06229 − 0.065 · 0.06229^0.6 = 0.05000).
- [`michell.rs::test_catamaran_large_spacing_doubles_monohull`](../crates/seacraft-core/src/michell.rs) — at
  s/L = 10, R_F doubles exactly and R_T within 2 %.
- [`math.rs::loop_signs_count_separate_loops_and_subtract_holes`](../crates/seacraft-core/src/math.rs) — two unit
  squares walked in opposite directions sum to 2; a 2 × 2 square with a unit hole, both counter-clockwise, gives 3.
- [`hydro.rs::the_lcf_lies_on_the_trimmed_waterplane`](../crates/seacraft-core/src/hydro.rs) — demo at T 0.62,
  trim 0.3: `|n·LCF + d| < 1e-9`.

### Provenance
Derived for these fixes (2026-10-24); hand and Python checks, not published values. Savitsky,
"Hydrodynamic design of planing hulls", Marine Technology 1(1), 1964 (not read; §43 checks the Savitsky equations
against the worked example of Molland et al. 2011, which states them).

---

## 20. Large-Angle GZ of a Box: Past 90° and at Exactly 90° (R8-1a)

### Formula
Closed box section, beam $b$, depth $d$, floating half full ($T = d/2$) with $G$ at the centre $C$ of the section
($KG = d/2$), heel $\phi$:

- The section is point-symmetric about $C$, so every line through $C$ halves its area: the waterline passes through
  $C$ at every heel.
- Wall-sided, $\tan\phi \le d/b$: $GZ = \sin\phi\,(GM + \tfrac12 BM \tan^2\phi)$, $BM = b^2/(6d)$, $GM = b^2/(6d) - d/4$.
- Deck and bottom cut, $\tan\phi \ge d/b$: the body turned 90° is a box of beam $d$, depth $b$ heeled by
  $\psi = \phi - 90^\circ$, and the righting moment does not depend on how the body axes are labelled, so
  $GZ = -\cos\phi\,(GM' + \tfrac12 BM'\cot^2\phi)$ with $BM' = d^2/(6b)$, $GM' = d^2/(6b) - b/4$. Both forms equal
  $\sin\phi\,(b^2 - d^2)/(6d)$ at $\tan\phi = d/b$, so the curve is continuous ($\sin\phi$ for $b = 4$, $d = 2$).
- Turning any box 180° about its axis maps it and a mid-depth $G$ onto themselves: $GZ(\phi + 180^\circ) = GZ(\phi)$ at
  every displacement; with $GZ(-\phi) = -GZ(\phi)$, $GZ(180^\circ - \phi) = -GZ(\phi)$ and $GZ(90^\circ) = GZ(180^\circ) = 0$.
- On its side (90°) the half-full box floats at $T' = b/2$; free to trim about an $LCG$ off $L/2$ it needs the level
  trim (bow over stern, against the water) $(L/2 - LCG)\cdot 12\,T'/L$ (§3 with $T'$), and every slice is a rectangle
  centred across the water surface, so $GZ = 0$, $KN = KG$.
- $GZ$ is measured along $t = n \times \hat x$ (across the hull, in the water surface), positive towards smaller heel at
  every heel; $KN = GZ + KG \sin\phi$.

### Test Locations
- [`crates/seacraft-core/tests/analytic.rs::a_half_full_box_has_the_analytic_gz_from_0_to_180_degrees`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/tests/analytic.rs::a_light_box_with_g_at_mid_depth_rights_symmetrically_past_90_degrees`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/tests/analytic.rs::a_box_on_its_side_trims_freely_at_exactly_90_degrees`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/tests/analytic.rs::the_half_full_box_oracle_is_continuous_and_symmetric`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/src/examples/yacht.rs::yacht_vanishing_stability_is_past_110_degrees`](../crates/seacraft-core/src/examples/yacht.rs)
  (cross curves to 180° against the hull turned upside down, $GZ(h) = -GZ_{inv}(180^\circ - h)$, within $10^{-4}$ m)

### Numbers Used
- Box $10 \times 4 \times 2$ m, $T = 1$ m, $KG = 1$ m, 40 t at $\rho = 1000$: heels 0°–180° every 5°, the knee
  $\arctan(1/2) = 26.565^\circ$ and $153.435^\circ$, 89°, 90°, 91°, 135.001°; fixed and free to trim ($LCG = 5$ m);
  $|GZ - GZ_{\text{analytic}}| < 10^{-6}$ m. Samples: $GZ(45^\circ) = 0.530330$, $GZ(60^\circ) = 0.402778$,
  $GZ(91^\circ) = -0.014543$, $GZ(120^\circ) = -0.402778$ m.
- Light box $T = 0.6$ m, $KG = 1$ m: $|GZ(\phi) + GZ(180^\circ - \phi)| < 10^{-6}$ for $\phi = 0, 5, \dots, 180^\circ$,
  wall-sided values at 5°, 10°, 15° ($\tan\phi \le 2T/b$) and their mirrors.
- On its side, $LCG = 4$ m: level trim $2.4$ m, $|LCB - 4| < 10^{-5}$, $GZ = 0$, $KN = 1$ m.

### Provenance
Derived for this test (2026-10-31). The closed form and both symmetries were checked against an independent
Python script that clips the rectangular section by the heeled waterline and bisects the offset for the area
(agreement $2 \times 10^{-16}$ at 18 heels from 0° to 180°; light-box symmetry $6 \times 10^{-16}$). Not published values.

## 21. Damaged Residual GZ of a Light Barge with the Bilge Out (R8-2a)

### Formula
A box barge $L \times B \times D$ floating $\nabla = LBT$, heeled until the bilge leaves the water and before the
deck edge immerses, floats on a triangle at the low corner: legs $a$ along the bottom and $a \tan\phi$ up the side,

$$\tfrac12 a^2 \tan\phi = BT, \qquad GZ = \left(\tfrac{B}{2} - \tfrac{a}{3}\right)\cos\phi + \left(\tfrac{a\tan\phi}{3} - KG\right)\sin\phi,$$

valid while $a \le B$ and $a\tan\phi \le D$. Past $\tan\phi = 8T/B$ the plane through the keel on the centreline
already floats more than $\nabla$, so the waterplane cuts the hull's vertical below the keel: a draft solve clamped
to zero there reports the larger section. A compartment of zero permeability floods nothing, so the damaged
residual curve must equal it.

### Tests
- [`crates/seacraft-core/src/damage.rs::residual_gz_of_a_light_barge_follows_the_bilge_out_triangle`](../crates/seacraft-core/src/damage.rs)

### Numbers Used
- Box $10 \times 4 \times 2$ m, $T = 0.3$ m, $KG = 0.5$ m, 12 t at $\rho = 1.0$ t/m³; residual heels 16°–58° every
  2° (validity 8.53°…59.0°, clamp from 30.96°); $|GZ - GZ_{\text{analytic}}| < 10^{-6}$ m. Samples:
  $GZ(16^\circ) = 0.933929$, $GZ(32^\circ) = 1.093452$, $GZ(40^\circ) = 1.082906$, $GZ(58^\circ) = 0.973499$ m.

### Provenance
Derived for this test (2026-10-31); checked against an independent Python script that clips the
rectangular section by the heeled waterline and bisects its offset for the area (agreement $\le 5 \times 10^{-16}$
at 16°, 30°, 32°, 40°, 50°, 58°). Not published values.

## 22. A Hull Open at the Deck: Watertight-Deck Lid, Deck-Edge Immersion and Overload (R8-4)

### Formula
Model: an open boundary loop of the hull (the sheer of a hull without deck faces, as most FREE!ship models are)
is closed by a lid through the loop's own vertices (`hydro::boundary_lids`, ear-clipped in the plane of the loop's
Newell normal; planar for a planar sheer). The deck is taken as watertight, the usual intact-stability
assumption, so an open hull measures exactly like the same hull with a deck:

- Sheer dry: the lid is out of the water and contributes nothing; every number equals the closed hull's.
- Box $b \times d$, waterplane $z \le T + y\tan\phi$ (draft $T$, heel $\phi$, no trim): the submerged section is
  the rectangle clipped by that half-plane, $\nabla = L \cdot A$, centre at its centroid. The waterline runs from
  the bottom to the deck when $T/\tan\phi \le b/2$ and $(d - T)/\tan\phi \le b/2$: $A_{wp} = L\,d/(\tan\phi\cos\phi)$.
- Half full with $G$ at the centre: the §20 closed-form $GZ$ from 0° to 180°.
- Fully submerged: $\nabla = L b d$ at every deeper waterplane, so a larger target has no waterline.

Without the lid the divergence integral about the waterplane origin $O$ misses the submerged part of the opening,
$\tfrac13 \int_{\text{lid}} (p - O)\cdot n \, dA$: overloaded upright it reports $Lbd + A_{deck}\,h/3$ ($h$ the
waterplane height over the deck), a volume that grows without bound, and heeled past deck-edge immersion it is
wrong in either direction.

### Tests
- [`crates/seacraft-core/tests/analytic.rs::an_open_box_measures_like_the_closed_one_while_its_sheer_is_dry`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/tests/analytic.rs::an_open_box_heeled_past_deck_edge_immersion_keeps_its_deck_watertight`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/tests/analytic.rs::a_half_full_open_box_has_the_analytic_gz_from_0_to_180_degrees`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/tests/analytic.rs::an_overloaded_open_box_has_no_waterline`](../crates/seacraft-core/tests/analytic.rs)
- [`crates/seacraft-core/src/hydro.rs::the_demo_hulls_open_deck_is_closed_by_consistently_wound_lids`](../crates/seacraft-core/src/hydro.rs)
- [`crates/seacraft-core/src/loading.rs::an_overloaded_open_deck_hull_is_flagged_not_floated_at_a_huge_draft`](../crates/seacraft-core/src/loading.rs)

### Numbers Used
- Open box $10 \times 4 \times 4$ m at $T = 2$ m: volume, KB, BM, $A_{wp}$, wetted surface, $A_m$, lateral area equal
  the closed box's within $10^{-9}$, no warning.
- Open and closed box $10 \times 4 \times 2$ m, $T = 1.5$ m, $\phi = 40^\circ$ (deck edge 0.18 m under):
  $\nabla = 51.917536$ m³, centre $y = 0.610874$, $z = 0.846968$ m (rel. $10^{-9}$), $A_{wp} = 31.114477$ m²
  ($10^{-6}$); the open box warns `open_hull_lid_submerged`. Without the lid: 46.676581 m³ (the missing deck strip,
  $14.04$ m² at $1.1198$ m above $O$, is $5.2410$ m³).
- Half-full open box ($T = KG = 1$ m): §20 heels, fixed and free to trim, $|GZ - GZ_{\text{analytic}}| < 10^{-6}$ m.
- Overload: fully submerged at drafts 2.5, 10, 75 m the open box floats $80$ m³ with VCB 1 m (lid-less: 86.667 m³
  at 2.5 m); $1.01 \cdot 80$ m³ has no draft or waterline at 0°, 30°, 120°, fixed or free; $0.99 \cdot 80$ m³ floats
  at $1.98$ m. Demo hull, 1000 t lightship: `equilibrium_found = false`, "cannot float" (was 74.6 m, `true`).

### Provenance
Derived for this test (2026-10-31). The clipped-section numbers, the waterplane run and the lid-less deficit
were recomputed with an independent Python script (shoelace on the clipped rectangle); the lid-less values are what
the code produced before the fix and match $Lbd + A\,h/3$ and the deficit formula to $10^{-12}$. FREE!ship's source
was not available locally; this follows the README's description of `TFreeHydrostaticCalc` (waterplane-only lid),
which assumes a hull closed below the waterline. Not published values.

---

## 23. Viewport Waterline of a Unit Cube

### Formula
A plane parallel to two faces of a cube of side $a$ cuts a square section of perimeter $4a$.

### Tests and Values
`seacraft-gui render::tests::a_unit_cube_cut_at_half_height_has_a_perimeter_of_four`: $a = 1$, plane $z = 0.5$:
segment lengths sum to $4$ ($10^{-12}$) and every end lies on the plane ($10^{-12}$).

### Provenance
Elementary geometry, derived for this test (2026-11-07). Not a published value. It covers the line the viewports
draw, not a measurement: hydrostatics integrate the mesh separately.

---

## 24. Moulds: the Skin Deducted from a Section

### Formula
A station mould is the hull's section moved inward by the skin thickness $t$. Each straight piece of the section
moves along its own normal in the drawing plane by $t / \max(|n_p|, 0.2)$, where $n_p$ is the part of the surface
normal lying in the plane, so a surface meeting the plane at an angle $\alpha$ off square is deducted by
$t / \cos\alpha$. Neighbouring pieces meet where their moved lines meet (a mitre), however sharp the corner: at an
interior angle $\theta$ the corner moves $t / \sin\tfrac{\theta}{2}$ along its bisector. A piece the move consumes, one
shorter than its two ends' shortening $t\,(\cot\tfrac{\theta_1}{2} + \cot\tfrac{\theta_2}{2})$ at interior angles
$\theta_1, \theta_2$, is dropped and its neighbours' moved lines meet instead.
- Cylinder of radius $R$: the mould is a circle of radius $R - t$.
- Cone of half-angle $a$: at radius $R$ the mould radius is $R - t / \cos a$, and the bevel is $a$.
- Rectangle $W \times H$: the mould is $(W - 2t) \times (H - 2t)$ with square corners.

### Tests
`crates/seacraft-core/src/templates.rs`:
- `a_cylinder_mould_is_smaller_by_the_skin`: 720 flat strips, which lie within $R(1 - \cos(\pi / 720)) =
  9.5 \times 10^{-6}$ m of the cylinder; the test asserts each mould point within $5 \times 10^{-5}$ m of
  $R - t = 0.988$ m.
- `a_cone_mould_is_deducted_along_the_slope`: $a = \arctan(0.5 / 4) = 7.125^\circ$; at $x = 2$, $R = 0.75$, $t =
  0.012$: mould radius $0.75 - 0.012 \times 1.007782 = 0.737907$ m, bevel $a$.
- `a_box_mould_has_mitred_corners`: a $10 \times 4 \times 2$ m box with a deck and $t = 10$ mm gives a
  $3.98 \times 1.98$ m mould with square corners.
- `slivers_in_line_with_a_side_stay_inside_the_corner`: $4 \times 2$ moved in by 0.01 is $3.98 \times 1.98$, with
  0.2 and 0.4 mm pieces beside a corner in line with the sides.
- `a_segment_shorter_than_the_skin_at_a_corner_is_consumed_not_a_crossing`: $2 \times 1$ with one corner cut by a
  4 mm chamfer at $45^\circ$, $t = 6$ mm: each end of the chamfer shortens by $0.006 \tan 22.5^\circ = 2.49$ mm, more
  than half its length, so the mould is $1.988 \times 0.988$ with the rectangle's own corner ($10^{-9}$).
- `a_sharp_v_keeps_its_whole_mitre`: a $20^\circ$ V with $t = 0.01$: its point moves $0.01 / \sin 10^\circ =
  0.0575877$ m ($10^{-9}$), where a mitre held at $4t$ would leave both sides nearer the hull than $t$.
- `a_section_the_skin_closes_to_nothing_is_reported`: a 20 mm square with $t = 10$ mm closes to a point (area
  0), which is reported as the skin being thicker than the hull's radius.
- `a_section_narrowing_both_ways_says_where_to_bevel`: a box whose half breadth falls from 1 to 0.5 m and whose
  deck rises from 1 to 1.5 m over 4 m. At $x = 2$ the sides (2.5 m of section) narrow forward and the deck (1.5 m)
  aft, both at $\arctan 0.125 = 7.125^\circ$: the station line goes on the forward face and the aft face is to be
  bevelled up to $7.1^\circ$. A cone, narrowing one way only, gets no such note.
- `a_hair_of_a_segment_beside_a_bend_is_consumed_quietly`: a 0.15 mm piece beside a $3^\circ$ bend; with
  $t = 0.06$ the mould corner is $0.06$ from both long lines ($10^{-9}$).
- `a_surface_nearly_in_the_plane_is_clamped_and_counted`: a cone 1 m in radius and 0.1 m long meets the plane at
  $5.7^\circ$; $|n_p| = 0.0995 < 0.2$, so the deduction is $0.01 / 0.2 = 0.05$ and 64 pieces are counted.
- `a_skin_thicker_than_the_radius_is_warned_about`: a tube 5 mm in radius under a 12 mm skin is consumed whole and
  warned about; at 50 mm it is not.

### Provenance
Closed forms derived by hand for these tests (2026-11-12 and 2026-11-13). Not published values. The deduction
rule is this project's own; the spec for it is in the module header of `templates.rs`.

---

## 25. Profile and Raked Transom Patterns

### Formula
A $L \times B \times D$ box cut by its centre plane is an $L \times D$ rectangle. A plane through $(x_0, 0, 0)$
whose top leans aft by $r$ cuts it in a rectangle $B$ wide and $D / \cos r$ high, drawn in that plane. The sides
are square to the plane and deducted by $t$; the bottom and deck meet it at $r$ off square and are deducted by
$t / \cos r$.

### Tests
`crates/seacraft-core/src/templates.rs`:
- `a_box_profile_is_its_side_view`: $10 \times 4 \times 2$ m, $t = 10$ mm: the cut spans
  $[0.01, 0.01, 9.99, 1.99]$ ($10^{-9}$); station marks at $u = 2$ and $5$.
- `a_raked_transom_is_drawn_in_its_own_plane`: $x_0 = 1$, $r = 20^\circ$: hull outline $4 \times 2.128356$, cut
  spans $[-1.99, 0.010642, 1.99, 2.117714]$ ($10^{-6}$), bevel $20^\circ$. The cut is taken 1 mm forward of the
  plane, which leaves this rectangle unchanged; at $x_0 = 0$, $r = 0$ (the box's own end face) it gives
  $[-1.99, 0.01, 1.99, 1.99]$.

### Provenance
Closed forms derived by hand for these tests (2026-11-13). Not published values.

---

## 26. Printed Size: the PDF Point and the Rendered Millimetre

### Formula
$1\ \text{pt} = 1/72$ in (PDF reference) and $1\ \text{in} = 25.4$ mm exactly (1959 international yard and
pound), so $1\ \text{mm} = 72 / 25.4 = 2.834646$ pt.

### Tests
- `crates/seacraft-core/src/pdf.rs::an_a4_page_has_the_a4_media_box`: $210 \times 297$ mm is
  $595.2756 \times 841.8898$ pt.
- `crates/seacraft-core/src/pdf.rs::a_100_mm_line_spans_283_465_pt`: 10 mm $= 28.346$ pt, 110 mm $= 311.811$ pt.
- `crates/seacraft-core/src/sheet.rs::a_single_page_is_the_drawing_the_calibration_column_and_the_margins`:
  $(1000 + 120 + 20)$ mm $= 3231.496$ pt by $(500 + 20)$ mm $= 1474.016$ pt; 100 mm $= 283.465$ pt.
- `crates/seacraft-core/src/sheet.rs::imperial_adds_the_inch_rule_and_changes_nothing_else`: 4 in $= 288$ pt.
- `crates/seacraft-core/tests/pdf_render.rs::poppler_draws_100_mm_as_1000_pixels_and_reads_turkish_text`
  (ignored by default; needs poppler): renders with poppler, an implementation independent of this writer, at
  254 dpi, where 1 px = 0.1 mm. A 100 mm line must cover 1000 px ($\pm 2$) and `pdftotext` must read back
  "Kayık Şile". Run on 2026-11-13 with poppler 22.02.0 (`pdftoppm -v`), it printed: `100 mm drew as 1000 px at
  254 dpi; text read back as "Kayık Şile"`.

### Provenance
Definitions, not measurements. The poppler run checks the writer against another program's reading of the file;
nobody has yet measured a printed page, and the PDFs have not been opened in a Windows or macOS viewer.

---

## 27. Imperial Readings and the Offsets Table in Millimetres

### Formula
$1\ \text{in} = 25.4$ mm exactly. A length is rounded to the nearest sixteenth of an inch and written as feet,
inches and eighths (`7-10-4` is 7 ft 10 4/8 in; a trailing `+` is one sixteenth more) or as feet, inches and a
fraction in lowest terms.

### Tests
- `crates/seacraft-core/src/units.rs::imperial_readings`: 0.3048 m → `1-0-0`; 1.0 m (39.370 in → 630/16) →
  `3-3-3`, `3' 3 3/8"`; 2.4 m (1512/16) → `7-10-4`, `7' 10 1/2"`; 0.3 m (189/16) → `0-11-6+`, `11 13/16"`;
  0.30475 m (191.97/16) carries to `1-0-0`; −0.05 m (31/16) → `-0-1-7+`; 0.01 m (6/16) → `0-0-3`, `3/8"`;
  0.3148 m (198/16) → `1-0-3`; −0.0001 m → `0-0-0` with no sign.
- `crates/seacraft-core/src/offsets.rs::imperial_adds_its_tables_after_the_metric_ones`: 2.0 m = 78.740 in =
  1260/16 in = 6 ft 6 12/16 in, written `6-6-6`.
- `crates/seacraft-core/src/offsets.rs::a_box_table_reads_in_millimetres`: a box 4 m wide has every half-breadth
  below the deck equal to 2000 mm.

### Provenance
Hand-computed from 1 in = 25.4 mm (2026-11-12, 2026-11-13). Not published values.

---

## 28. Tiling a Pattern over Paper

### Formula
This project's own layout rule: the printable area is the paper less two margins; neighbouring tiles share the
overlap, so they are one step apart, step = printable − overlap; the drawing is padded by 5 mm a side and needs
$\lceil w / \text{step}_w \rceil \times \lceil h / \text{step}_h \rceil$ tiles; the orientation with fewer tiles
wins; a tile nothing crosses is not printed.

### Tests
`crates/seacraft-core/src/sheet.rs`, A4 with 10 mm margin and overlap (printable $190 \times 277$, step
$180 \times 267$ mm):
- `a_small_sheet_is_one_tile_and_has_no_map`: $150 \times 250$ mm → $1 \times 1$.
- `the_orientation_with_fewer_tiles_wins`: $400 \times 250$ mm: portrait $3 \times 1$, landscape $2 \times 2$;
  $300 \times 300$ is $2 \times 2$ either way; $250 \times 400$: portrait $2 \times 2$, landscape $1 \times 3$.
- `a_tile_nothing_crosses_is_not_printed`: bounds $520 \times 620$ mm → $3 \times 3$ portrait, grid
  $550 \times 811$ mm with the drawing's corner at $(15, 95.5)$.
- `the_drawing_and_the_join_grid_are_each_written_once`: a 700 mm line takes $\lceil 700 / 267 \rceil = 3$
  landscape tiles.
- `a_join_cross_is_at_the_same_sheet_position_on_both_pages`: neighbours differ by one step, 267 mm $= 756.850$ pt.
- `a_ship_at_full_size_is_refused_with_the_count`: $100 \times 30$ m at full size on A4 landscape is $375 \times 167$
  tiles and is refused.

### Provenance
Worked by hand from the rule above (2026-11-12). The rule is this project's, so these check the code against its
own specification, not against an outside source.

---

## 29. Builder Patterns: Internal Consistency, Not Oracles

These compare one part of the program with another. They catch a part drifting from the rest; they do not show
either part is right.
- `crates/seacraft-core/src/templates.rs::the_demo_hull_s_moulds_agree_with_its_offsets_table`: each demo-hull
  mould is no wider than the offsets table's section at that station, and narrower by no more than the skin over
  the 0.2 floor.
- `crates/seacraft-core/src/templates.rs::the_demo_hull_s_profile_is_as_long_as_the_hull`: the profile spans the
  hull's length within 1 %.
- `crates/seacraft-core/src/templates.rs::the_runabout_s_moulds_keep_the_skin_from_the_hull`: every point of each
  `runabout-6m` station mould is at least $0.99\,t$ from the section ($t = 6$ mm), and none crosses itself.
- `crates/seacraft-gui/tests/templates.rs::the_window_shows_the_page_count_once_it_is_computed`: the count the
  window shows equals `sheet::page_count` of the same set.

---

## 30. The Flattening's Linear Solver

### Formula
A symmetric positive-definite $A$ factors as $A = L L^\top$ (Cholesky); after a reverse Cuthill–McKee reordering
the factor stays inside the matrix's envelope.
- $A = \begin{pmatrix}4&1&0\\1&3&1\\0&1&2\end{pmatrix}$, $b = (1, 2, 3)$: the answer by Cramer's rule,
  $x_k = \det A_k / \det A$, computed in the test.
- A path of $n$ nodes joined by unit springs, node 0 held at 0, a unit load on every other node: node $i$
  carries the load of nodes $i..n-1$, so $x_i - x_{i-1} = n - i$ and $x_i = i(2n - 1 - i)/2$ ($n = 3$: 2, 3).
- $\begin{pmatrix}1&2\\2&1\end{pmatrix}$ has eigenvalues 3 and $-1$: no Cholesky factor.

### Tests
`crates/seacraft-core/src/flatten.rs`: `cholesky_solves_a_small_spd_system` ($10^{-12}$),
`cholesky_on_a_path_laplacian_matches_closed_form` ($n = 50$, $10^{-9}$), `cholesky_refuses_an_indefinite_matrix`.

### Provenance
Closed forms worked by hand (2026-11-14). Not published values.

---

## 31. Laying a Panel Flat

### Formula
`flatten` is the as-rigid-as-possible (ARAP) local/global method of Liu, Zhang, Xu, Gotsman and Gortler, "A
Local/Global Approach to Mesh Parameterization", Symposium on Geometry Processing 2008 (Computer Graphics Forum
27(5)), sections 4.3–4.4, equations 1 and 4: least squares over every triangle edge, each against its true length
turned by the triangle's best rotation, weighted by the cotangent of the opposite angle. It is exact (every edge its
3D length) on a mesh that is developable as meshed:
- A cylinder of radius $r$ faceted on its rulings, $N$ facets over $\Phi$: a rectangle of length $\ell$ and width
  $N \cdot 2r\sin(\Phi / 2N)$.
- A cone of half-angle $\alpha$ faceted on its rulings, $N$ facets over $\Phi$: two rulings $\Phi/N$ apart meet at the
  apex at $2\arcsin(\sin\alpha \sin(\Phi / 2N))$, so the pattern is a sector of $N$ times that, its radii the slant
  distances.
- A strip folded along a straight line: the strip.
- A planar panel: itself.

A surface with Gaussian curvature cannot lie flat. Gauss–Bonnet, discrete and exact for any faceted disc: the angle
defects at interior vertices plus the turning $\pi - \theta$ at boundary vertices sum to $2\pi$. Smooth: a cap of
polar angle $\varphi$ on a unit sphere holds $2\pi(1 - \cos\varphi)$.

### Tests
`crates/seacraft-core/src/flatten.rs`:
- `a_cylinder_meshed_on_its_rulings_flattens_to_its_rectangle`: $r = 0.5$, $\ell = 2$, 24 facets over $120°$; every
  edge and the rectangle's sides and diagonal to $10^{-9}$.
- `a_cone_meshed_on_its_rulings_flattens_to_its_sector`: $\alpha = 30°$, slant 0.3 to 1, 36 facets over $180°$;
  sector angle and both radii to $10^{-9}$.
- `a_cone_to_its_apex_flattens_after_cleaning`: the same cone from slant 0, a pole of 37 coincident vertices and 36
  collapsed triangles that `clean` welds and drops; sector and radius to $10^{-9}$.
- `a_folded_strip_flattens_exactly`: $0.2 \times 1$ m folded $90°$; edges to $10^{-9}$, area 0.2 m² to $10^{-12}$.
- `a_spherical_cap_has_its_gauss_bonnet_defect_and_stretches`: $\varphi = 40°$, 10 rings of 48: discrete
  Gauss–Bonnet to $10^{-9}$; interior defects 1.3333 against $2\pi(1 - \cos 38°) = 1.3320$ (each interior vertex
  stands for the surface half way to its neighbours, so they hold the cap to half a ring short of the edge,
  $\varphi' = 40° \times 9.5/10$), within 2 %; the flattening stretches it (RMS 2.40 %, above 0.1 %).
- `crates/seacraft-core/src/panels.rs::a_planar_transom_with_obtuse_triangles_flattens_exactly`: the demo hull's
  transom, one planar 11-sided face, whose subdivision has edges with cotangent sums below zero; largest edge strain
  under $10^{-9}$.

Convergence, stated as such: `skewed_diagonals_leave_a_residual_that_shrinks`: a cylinder whose alternate rings are
turned half a facet is not developable as meshed; its RMS strain at 12, 24 and 48 facets is $8.4 \times 10^{-6}$,
$4.9 \times 10^{-7}$, $2.9 \times 10^{-8}$ (about $N^{-4}$), asserted falling and under $10^{-4}$.

Refusals, by construction: `a_ring_is_not_a_disc` (two edge loops), `a_fold_is_refused` (a triangle turned over),
`a_self_crossing_outline_is_refused` (a strip wound 1.2 times round a spiral, whose end line passes through a
vertex of the first turn), `pieces_of_one_straight_edge_do_not_cross` (two pieces of one line with orientations of
$\pm 9 \times 10^{-19}$, from the pram's transom read back from its file, are apart).

When to stop, a numerical judgement, not an oracle: the local/global loop converges linearly, so once the largest
step shrinks by a steady factor $\rho$ per iteration (measured over 10 iterations) about $\text{step} \cdot \rho /
(1 - \rho)$ is left to move. The loop stops when that is under $10^{-6}$ of the mesh's size (0.01 mm on a 10 m
panel) or a step is under $10^{-10}$ of it (rounding), and a pattern stopped by the 10 000-iteration limit carries
the estimate, is not judged to lie flat and says so on its sheet. Measured (release build): the demo hull's
Topsides, which stopped unsettled at the earlier 1000-iteration limit 2.9 mm from the same mesh renumbered,
settles in about 3500 iterations within 0.016 mm of it; the planar panels of the pram read back from its file
settle at once instead of running to the limit.
- `the_iteration_limit_is_reported_not_hidden`: a cap stopped after 3 iterations reports how far it is from
  settled; with the full limit it settles.
- `crates/seacraft-core/src/panels.rs::a_planar_panel_read_back_from_its_file_settles_at_once`: every panel of the
  flat pram read back from its `.fef` settles in under 50 iterations.

Internal consistency, not an oracle: `the_result_does_not_depend_on_face_order_or_placement` (faces scrambled and
the mesh turned and moved: outlines agree after the best rigid fit within twice the settling tolerance,
$2 \times 10^{-6}$ of the mesh's size; measured $1.3 \times 10^{-6}$ m on the 2.2 m mesh).

### Provenance
The method was read in the paper itself (2026-11-14); the closed forms were worked by hand for these tests. Two
choices differ from the paper: the start is SeaCraft's edge-joining development, and a turned-over triangle is
refused instead of repaired. Cotangents that are not positive are raised to $10^{-6}$ of the mean positive one,
each on its own; this keeps the exact cases exact (every weight positive, zero energy at an isometry). The least
stretch is geometry: nothing here models plywood bending.

---

## 32. Panels of a Hard-Chine Pram: Measures

### Formula
The test pram (`panels::pram_fixture`, test code, not an example hull) has straight chines and planar sides, bottom
and transoms, so every panel lies flat exactly: zero stretch, seams their 3D length, flat area the 3D area. Seam
drift is the largest $|s_\text{flat} - s_\text{3D}|$ of the cumulative lengths at the station ticks and the seam's
end, measured from the first tick and again from the last.

### Tests
`crates/seacraft-core/src/panels.rs`:
- `a_flat_pram_lies_flat_with_true_seams`: four panels; RMS stretch under $10^{-9}$, every seam drift under
  $10^{-6}$ m, areas equal to $10^{-9}$ m². Measured on the file round trip: drift $\le 2.1 \times 10^{-11}$ m.
- `drift_is_measured_from_both_ends`: ticks $(0, 1, 2)$ against $(0, 1.003, 2.003)$ and against
  $(0, 1, 2.003)$ both drift 3 mm; against $(0, 1.004, 2)$, 4 mm. Arithmetic.
- `mating_seams_are_compared`: the chine as the bottom and the side lay it flat differs by under $10^{-6}$ m; five
  seam pairs.
- `a_pram_read_back_from_its_file_still_flattens`: flat, V and bowed prams written to a `.fef` and read back (points
  rounded by up to $4.6 \times 10^{-6}$ m) give four panels each.
- `a_joined_panel_measures_its_seams_across_the_centreline_and_marks_it`: the flat bottom, joined to its mirror,
  measures its edges to the transoms whole, $2 \times 0.35$ and $2 \times 0.45$ m to $10^{-6}$ m, as do the joined
  transoms; its pattern carries the centreline, 2.4 m long, and both chines are labelled.

Convergence, stated as such: `a_curved_side_reports_its_level_shift`: with the sides bowed 80 mm out amidships, the
side's outline moves 1.04 mm from level 1 to 2 and 0.057 mm from 3 to 4 (largest distance between the two outlines,
either way round, after the best rigid fit of their corners). `the_level_shift_is_the_outline_not_the_corners`
pins the 0.057 mm (between 0.04 and 0.08 mm); the corners alone move 0.0099 mm, which is why the corners are only
used to place the outlines. `the_outline_shift_sees_a_bulge_between_corners` is the closed form: two unit squares
with the same corners, one with an edge's middle pushed out 10 mm and the square turned and moved, differ by
10 mm to $10^{-12}$.

Internal consistency: `the_edge_join_gap_is_kept` (the `develop::unfold` gap is reported as is), `limits_decide_
lies_flat` (a cap of measured RMS $r$ fails at a limit of $0.99r$ and passes at $1.01r$).

### Limits
"Does not lie flat" above 2 mm seam drift or 0.1 % RMS stretch. **Project judgement with no published source**,
chosen by the maintainer (2026-11-14) as provisional, to be recalibrated when a skiff has been built from the
patterns. `--max-seam-drift-mm` and `--max-stretch-pct` change them.

### Provenance
Closed forms and arithmetic worked by hand (2026-11-14). Not published values.

---

## 33. Panel Offsets

### Formula
Offsets are measured square off a straight baseline along the pattern's lowest edge, at rows every spacing from its
aft end and one at its forward end.
- A $2.4 \times 0.6$ m rectangle: rows at 0, 250, …, 2250 and 2400 mm, every row crossing at 0 and 600 mm.
- The development of a cone of half-angle $30°$ taken $180°$ round, slant 0.3 to 1, with 1440 facets: a sector of
  angle $S = 1440 \cdot 2\arcsin(\sin 30° \sin(\pi / 2880))$; a row at $x$ crosses the arcs at $\pm\sqrt{r^2 - x^2}$
  (where within the sector) and the end rulings at $\pm x\tan(S/2)$ (where $0.3 \le x / \cos(S/2) \le 1$). The
  chords lie within $1 - \cos(\delta/2) \approx 1.5 \times 10^{-7}$ m of the arcs.

### Tests
`crates/seacraft-core/src/panels.rs`: `a_rectangle_has_constant_ordinates` ($10^{-6}$ m),
`the_cone_sector_ordinates_follow_its_arcs` (three rows, $10^{-6}$ m), `csv_and_pdf_carry_every_row` (one CSV line
per crossing), `a_name_with_a_comma_is_quoted_in_the_csv`.

### Provenance
Closed forms worked by hand (2026-11-14). Not published values.

---

## 34. Plywood Sheets End to End

### Formula
The first sheet reaches its length $L$; each further one adds $L - s$ for a scarf $s$. A panel wider than the sheet
is not laid out. Totals are sheets per copy times copies: no nesting, an upper bound.

### Tests
`crates/seacraft-core/src/panels.rs`: `a_panel_shorter_than_a_sheet_takes_one` (2.0 m: 1);
`butt_and_scarf_counts` (5.0 m on 2440 mm: 3 sheets, joints at 2.44 and 4.88 m; with a 200 mm scarf: 3 sheets,
joints at 2.44 and 4.68 m); `a_panel_wider_than_the_stock_is_reported` (1300 mm on 1220 mm: not laid out, not
counted); `total_counts_copies` (a pair of one-sheet panels and a two-sheet panel: 4).

### Provenance
Arithmetic (2026-11-14). Not published values.

---

## 35. Segments Blocked by a Triangle Mesh

### Formula
A segment $a \to b$ meets a triangle where the Möller–Trumbore solution has $0 < t < 1$ and barycentrics inside
the triangle, from either side. The index only decides which triangles are tried; its answer must be the brute
force one.

### Tests
`crates/seacraft-core/src/raycast.rs`: `box_segment_hits_at_quarter_and_beside_misses` (box $[-1, 1]^3$, segment
along an axis from 2 outside to 2 beyond the far face: first hit at $t = 1/4$; a parallel segment beside the box:
none); `lcg_random_triangles_and_segments_match_brute_force` (fixed-seed triangles and segments: same hit or miss
and same nearest $t$ as testing every triangle); `of_render_mesh_triangulates_quad_and_hits` (a quad is fanned
into two triangles); `a_lopsided_mesh_does_not_outgrow_the_traversal_stack` (600 000 triangles crowded as
$u^{4.596}$: tree depth at most 64, nearest hits at $t = 1/3$ and $(2 - x_\text{last})/3$).

`crates/seacraft-gui/src/visibility.rs`: `box_occludes_far_point_and_middle_third_of_far_edge` (looking along
$+x$ at the same box: a point in front is visible, one behind is hidden, one on the near face is visible; the
edge $(2, -3, 0) \to (2, 3, 0)$ is hidden for $y \in [-1, 1]$, that is $t \in [1/3, 2/3]$, to one sample).

`crates/seacraft-gui/src/render.rs`: `a_hidden_span_ends_where_its_world_point_projects_in_perspective` (pinhole
camera at the origin looking along $+z$; edge $(0, 0, 2) \to (6, 0, 6)$ hidden from $t = 1/2$: the world point
$(3, 0, 4)$ projects to $x/z = 3/4$, between $0$ and $1$ at the ends, so the dimmed stretch starts three quarters
along the screen segment, not halfway).

### Provenance
Geometry worked by hand (2026-11-17). Not published values.

---

## 36. Net Lines and Stations

### Formula
A line continues through a point to the edge two steps away in the fan of quads round it: opposite across four
quads, along the rim across two, straight on at a three-quad corner; anywhere else it ends. Neighbouring sides of
a quad are of different families; the family running further along $x$ is lengthwise.

### Tests
`crates/seacraft-core/src/netlines.rs`: `grid_5x4_has_five_transverse_and_four_lengthwise_lines` (a $5 \times 4$
grid: 5 stations of 4 points at $x = i$, 4 lengthwise lines of 5 points; its outline is four lines);
`a_line_lists_its_points_in_order_with_the_edges_between_them`; `a_ring_of_a_tube_is_one_closed_line` (4 rings of
6 points: 4 closed stations, 6 open lengthwise lines); `demo_hull_stations_follow_its_station_table` (the half
demo hull: 8 stations of 6 points at the $x$ of its station table, the transom rim one line of 6 at $x = 0$, 6
lengthwise lines of 9, 17 lines in all with the two apex edges); `the_full_demo_hull_has_whole_stations_port_to_starboard`
(11 points a station); `a_hole_inside_the_net_leaves_the_grid_lines_whole` (a $5 \times 5$ grid with one quad
split into a fan: still 5 and 5 lines of 5 points, plus 4 spokes); `a_pole_ends_the_lines_that_reach_it`;
`a_boundary_point_with_four_quads_ends_its_lines`; `every_shipped_example_classifies_every_edge`.

`crates/seacraft-gui/src/render.rs`: `station_caption_formats_single_x_and_leaning_range` (stations 2.5 m apart
on a 10 m net: "x ≈ 5.00 m"; one point moved 0.20 m, more than 0.5 % of the length: "x 5.00–5.20 m").

### Provenance
Counted by hand on nets built for the purpose, and from the demo hull's own station table in `demo.rs`
(2026-11-17). Not published values.

---

## 37. Moving Points Together

### Formula
A seed moves by the whole displacement $\Delta$; a follower at distance $d$ along the allowed edges from the
nearest seed moves by $w(d / r)\,\Delta$ in the followed axes, with reach $r$ and $w(t)$: linear $1 - t$, smooth
$1 - 3t^2 + 2t^3$, sharp $(1 - t)^2$, constant $1$, or straight lines through given points; $w = 0$ for $t > 1$.
On a symmetric layer no point goes below $y = 0$, and a point on the centreline stays on it unless it is the
only seed. Followers take their share of the move the seeds could make: a move towards the centreline is cut
to the room $\min y_{\text{seed}}$ of the symmetric seeds, so a seed stopped at $y = 0$ stops its followers too.

### Tests
`crates/seacraft-core/src/connected.rs`, on a unit grid: `falloff_and_grid_dijkstra_weights_match_closed_forms`
(reach 3, linear: $2/3$ and $1/3$ at one and two edges; lengthwise leaves the station neighbours out; both ways
the distance is the Manhattan distance; smooth and sharp at $t = 0.5$: $0.5$ and $0.25$);
`a_curve_is_read_in_order_of_t_and_holds_its_end_values`; `stop_at_creases_holds_a_move_on_one_side` (a creased
row: a seed below reaches the crease points and nothing beyond, 14 followers; a seed on the crease reaches all
24); `two_seeds_use_the_nearer_and_constant_reaches_the_rim`; `a_centreline_follower_stays_on_the_centreline`
(seed moved $(0, 0.6, 0.3)$: the centreline neighbour goes to $z = 0.2$ and keeps $y = 0$);
`a_split_face_fan_still_yields_follower_weights` (the fan's centre, $\sqrt{0.5}$ away: $1 - \sqrt{0.5}/3$);
`move_station_on_the_demo_hull_moves_the_station_and_nothing_else` (the station's other 5 points, weight 1);
`the_built_in_presets_are_these_seven`; `parse_presets_rejects_each_invalid_input_naming_the_preset`.

`crates/seacraft-core/src/netmove.rs`: `centreline_and_symmetric_layer_rules_hold_for_single_and_multi_seed_moves`;
`followers_move_by_their_weight_in_the_masked_axes_and_seeds_by_all_of_it` (weight 0.5 in "yz" of
$(0.2, 0.4, 0.6)$: $(0, 0.2, 0.3)$); `a_zero_move_puts_every_point_back_exactly`;
`moving_the_anchor_to_a_position_lands_on_it_exactly` ($0.3 + (0.9 - 0.3) \ne 0.9$ in floating point);
`followers_move_by_what_the_seeds_could_move_when_the_centreline_stops_them` (seed at $y = 1$ moved by
$-3$ stops at $0$; a weight-0.5 follower at $y = 2$ goes to $2 - 0.5 = 1.5$, and to $1.8$ for a move of
$-0.4$; with a second seed at $y = 2$ the room is still $1$).

### Provenance
Arithmetic by hand (2026-11-17). Not published values. A preset says how points move; nothing here measures or
claims fairness.

---

## 38. Michell Transom Area Ratio

### Formula
The ratio is the immersed area of the aftmost grid station over the largest immersed station area, each
$A(x_i) = 2\int_{-T}^{0} f(x_i, z)\,dz$ by the trapezoid rule. On the Wigley grid every station has the same
$z$-shape, so $A(x) \propto 1 - (2x/L)^2$ whatever the quadrature: $0$ at the closed stern $x = -L/2$, and
$1 - 0.6^2 = 0.64$ on a Wigley grid cut off at $x = -0.3L$.

### Test Location
- `crates/seacraft-core/src/michell.rs::transom_ratio_is_the_aft_section_over_the_largest_one`: $L = 10$,
  $B = 1$, $T = 0.625$ m, $81 \times 25$ grid ($x = -3$ m is a station): exactly $0$ closed, $0.64$ within
  $10^{-12}$ cut; the warning fires on the cut grid and not on the closed one.
- `crates/seacraft-core/src/michell.rs::transom_warning_fires_on_the_demo_hull_and_not_on_the_wigley_example`:
  surface grids at design draft (65 × 25, level 3 Loop). The demo hull measures 0.84, above the 5 % limit; the
  Wigley example below 0.01 (its end station is cut 0.05 % of $L$ inside the hull, so not exactly 0).

### Provenance
Closed form by hand (2026-11-18). The 5 % limit (`MICHELL_TRANSOM_AREA_LIMIT`) is the project's own threshold,
not a published limit: the example hulls that close at the stern measure below 2 % (yacht 1.4 %, trawler 1.8 %,
container ship 0.5 %, kayık 0.01 %), the demo hull 84 % and the runabout 100 %.

---

## 39. Wigley Michell Wave Resistance against Kajitani et al. (1983)

### Sources
- **Primary, experiment.** Kajitani, H., Miyata, H., Ikehata, M., Tanaka, H., Adachi, H., Namimatsu, M. &
  Ogiwara, S. (1983). "The summary of the cooperative experiment on Wigley parabolic model in Japan." *Proc. 2nd
  DTNSRDC Workshop on Ship Wave-Resistance Computations*, Bethesda, MD, pp. 5–35. DTIC ADP003037. Read
  2026-11-18 from the DTIC scan: geosim Wigley models of 6.0 m (IHI), 4.0 m (SRI), 2.5 m (UT) and 2.0 m (YNU),
  $y = (B/2)[1 - (2x/L)^2][1 - (z/D)^2]$, $B/L = 0.1$, $D/L = 0.0625$ (report p. 8), the proportions of
  `wigley_grid`. $C_w = R_w / (\tfrac12 \rho U^2 S)$ is "derived from towing test" using a form factor $k$ on the
  Schoenherr line, i.e. $C_T - (1 + k) C_{F0}$, and $C_{wp}$ from wave-pattern analysis (Newman–Sharma), both on
  $S = 0.661\,L(2D + B)$ (Nomenclature, p. 6). Fig. 1: free to sink and trim (FR), 6, 4 and 2.5 m; Fig. 2:
  fixed (FX), 4 and 2.5 m; Fig. 5: 2.0 m, FR.
- **Independent computation, not experiment.** Peri, D. (2026). "SIREN: A nonlinear Rankine-source boundary
  element solver for the stationary motion of a marine vehicle in calm water." arXiv:2610.14027, Table 3,
  column "Michell": the same hull from an independent Python script ($\lambda = \cosh t$, Simpson), $C_w$ on the
  true wetted surface.

### Formula
Michell's integral as in §7, on `wigley_grid` ($L = 10$ m, $B = 1$ m, $T = 0.625$ m); $C_w$ on the grid's
$S = 14.879\text{ m}^2$, which is Kajitani's $S$ to 0.05 % (§7). Michell models a hull held at its rest
waterline, so the fixed (FX) runs are the like-for-like case; the free (FR) runs sink and trim, which raises
$C_w$.

### Test Location
- `crates/seacraft-core/src/michell.rs::wigley_michell_against_kajitani_1983_towing_tests`
- `crates/seacraft-core/src/michell.rs::wigley_michell_matches_siren_independent_computation`
- Readings and crops: [`docs/oracles/kajitani_1983_wigley.py`](oracles/kajitani_1983_wigley.py)

### Numbers Used
$C_w$ is plotted, not tabulated, so it was digitised: pages rendered at 300 dpi, axes calibrated on the printed
scale ticks (the graph paper is 0.01 Fn × 0.2e-3 per cell; the typed ticks sit on its half-rows), and markers and
faired curves read against a 0.005 Fn × 0.05e-3 grid over enlarged crops. Uncertainty of a reading: Fn ± 0.005,
$C_w$ ± 0.05e-3. $C_w \times 10^3$:

| Model | Condition | Figure | Hump Fn | Hump $C_w$ | Hollow Fn | Hollow $C_w$ |
|---|---|---|---|---|---|---|
| SRI 4.0 m | FX | 2 | 0.320 | 1.39 | 0.350 | 1.21 |
| UT 2.5 m | FX | 2 | 0.318 | 1.45 | 0.350 | 1.30 |
| IHI 6.0 m | FR | 1 | 0.313 | 1.81 | 0.345 | 1.62 |
| SRI 4.0 m | FR | 1 | 0.320 | 1.61 | 0.350 | 1.45 |
| UT 2.5 m | FR | 1 | 0.320 | 1.70 | 0.345 | 1.53 |
| YNU 2.0 m | FR | 5 | 0.322 | 1.59 | 0.347 | 1.48 |

Michell (129 × 49 grid, 1024 angles): hump $2.14 \times 10^{-3}$ at Fn 0.299, hollow $1.235 \times 10^{-3}$ at
Fn 0.346 (65 × 25, 256 angles in the test: the same to 0.1 %). So Michell's hump comes 0.014–0.023 in Fn early
and is 18–54 % high (FX: 48–54 %); its hollow is within 0.004 of the measured Fn, 0.95–1.02 of the FX values and
0.76–0.85 of the FR values. The test asserts, for every row: hump 0.009–0.028 early, hump ratio 1.14–1.60, hollow
Fn within 0.009, hollow ratio 0.73–1.07, and for the FX rows a hollow ratio of 0.91–1.07: the measured ranges
widened by the reading uncertainty. These bands record what thin-ship theory gets right and wrong on this hull;
they were not tuned, and the overprediction of the hump is the known behaviour of Michell's theory on the Wigley
hull, not something to remove.

$C_{wp}$ (wave-pattern analysis) is the quantity Michell's far-field integral corresponds to, but scatters more:
hump $\times 10^3$ SRI FX 1.37, UT FX 1.48, IHI FR 1.57, SRI FR 1.45, YNU FR 1.32 (Fn 0.313–0.322), against which
Michell is 1.36–1.62 times high. Recorded, not tested.

SIREN Table 3 (Michell, $\times 10^3$) at Fn 0.250, 0.267, 0.289, 0.316, 0.354, 0.408, 0.450: 1.064, 0.976, 1.987,
1.831, 1.284, 3.030, 4.154. SeaCraft on a 129 × 49 grid with 512 angles agrees within 0.06 %; the test allows
0.2 %. SIREN's value at Fn 0.25 equals the §7 reference.

### Provenance
Read from the primary source (2026-11-18) as described above; the figure pages,
calibrations and every reading are in `docs/oracles/kajitani_1983_wigley.py`, which redraws the crops from a
render of the DTIC PDF so each reading can be checked. Consistency of the reading: Fig. 3 replots the UT 2.5 m
model and, read at half that resolution, gives a hump of about 1.65 (FR) and 1.46 (FX), against 1.70 and 1.45
from Figs. 1 and 2.

Not used: the report's typed pressure-resistance table (p. 23, $C_{PR}$ by integrating hull pressure at Fn 0.250,
0.267, 0.289 and 0.316) includes viscous pressure drag, so it is not a wave resistance. SIREN's Table 5 lists
$C_{t,exp}$ attributed to the SRI 4 m model that does not match Fig. 1 as read here (3.94 against 4.23 at
Fn 0.25), so it is not used as a reproduction. Wu et al. (2019, *Phys. Fluids* 31, 047102) and Weymouth & Yue
(2014, arXiv:1401.3816) reproduce Wigley wave profiles and waterline elevations, not wave resistance, so they
could not check these readings; thin-ship theory gives no hull wave profile, and those profiles are not an
oracle here.

What this covers: Michell's $R_w$ for the Wigley hull ($L/B = 10$, $B/T = 1.6$) at Fn 0.28–0.37, in the bands
above. It says nothing about other hulls, about transom sterns (flagged, §38), or about catamaran interference.
The wave pattern from the same spectrum is §40; no measured Wigley wave cut was found to compare it with.

## 40. Far-field Wave Pattern: Kelvin's Far Field and the Wedge

### Sources
- Wu, H., Wu, J., He, J., Zhu, R., Yang, C.-J. & Noblesse, F. (2019). "Wave profile along a ship hull, short
  far-field waves, and broad inner Kelvin wake sans divergent waves." *Physics of Fluids* 31, 047102. Read
  2026-11-18: elementary waves eqs. (9b), (10a); amplitude function eq. (12a) with its Hogner (thin-ship) limit
  $A_\pm = n_x$ (eq. 12d without the potential terms) and eqs. (12b), (12d); far-field elevation eq. (14a);
  stationary-phase far field eqs. (13a), (13b), (15a)–(15d); the cusp ray eq. (18).
- Michell's $P + iQ$ and its grid as in §7.

### Formula
On a hull $y = f(x, z)$, $n_x\,da = -f_x\,dx\,dz$, and the thin-ship limit puts the amplitude integral on the
centreplane, where it is Michell's $P + iQ$. With $1/F^2 = k_0 L$, $A = -(2k_0/L)(P - iQ)$, and eq. (14a) in metres
is
$$\zeta(x, y) = -\frac{2k_0}{\pi} \int_0^{\theta_c} \sec^3\theta\,
\mathrm{Re}\!\left[\overline{S(x;\theta)}\left(e^{i\psi_+} + e^{i\psi_-}\right)\right] d\theta,\qquad
\psi_\pm = k_0 \sec^2\theta\,(x\cos\theta \pm y\sin\theta),$$
$k_0 = g/U^2$, model axes ($x$ forward). $S(x;\theta)$ is $P + iQ$ of the part of the hull forward of $x$: the whole
hull behind the stern (Wu's eq. 14a as written), none ahead of the bow, and the forward part of the hull alongside
it (SeaCraft's extension, not Wu's). A catamaran at $y = \pm s/2$ multiplies the integrand by
$2\cos(k_0\sec^2\theta\sin\theta\,s/2)$, whose square is Michell's interference factor. $\theta_c$ leaves out waves
shorter than two grid spacings.

Kelvin's far field (eqs. 13a, 13b, 15a–15d), the check of the evaluation: with $h = k_0\sqrt{X^2 + Y^2}$,
$\tan\psi = Y/(-X)$, $q_T = 2\tan\psi/(1 + \sqrt{1 - 8\tan^2\psi})$, $q_D = 1/(2q_T)$, and for $q \in \{q_T, q_D\}$:
$q^* = \sqrt{1 + q^2}$, $\varphi = -(1 + q^2)/\sqrt{1 + 4q^2}$,
$\varphi'' = (2q^2 - 1)/((1 + q^2)\sqrt{1 + 4q^2})$, $A/F^2 = -2k_0^2 (P - iQ)(\theta = \arctan q)$,
$$\zeta \approx \frac{1}{k_0}\,\mathrm{Re}\!\left[a_T e^{i(h\varphi_T - \pi/4)} + a_D e^{i(h\varphi_D + \pi/4)}\right],\qquad
a = \sqrt{2/\pi}\;\frac{q^* A/F^2}{\sqrt{|\varphi''|}\sqrt{h}}.$$
On the track $q_T = 0$ and the divergent term drops. The cusp ray is $\tan\psi_K = 1/\sqrt 8$ (19.47°), eq. (18); the
transverse wavelength on the track is $2\pi U^2/g$, eq. (10a) at $q = 0$. A stationary-phase evaluation of the
integral above gives the same $q_T$, $q_D$, $\varphi$, $\varphi''$ and amplitude: a second check on their
transcription from the paper.

A remark, not a source: with $\mathcal A(\theta) = -(2k_0/\pi)\sec^3\theta\,(P - iQ)$ over $\theta \in (-\pi/2,
\pi/2)$, Havelock's $R_w = (\pi\rho U^2/2)\int|\mathcal A|^2\cos^3\theta\,d\theta$ equals Michell's §7 integral term
by term. Havelock's formula is quoted for that remark only; the scale of the field rests on Wu's equations above.

### Test Location
- `crates/seacraft-core/src/wave_pattern.rs::kelvin_far_field_on_the_track`
- `crates/seacraft-core/src/wave_pattern.rs::kelvin_far_field_off_the_track`
- `crates/seacraft-core/src/wave_pattern.rs::transverse_wavelength_on_the_track`
- `crates/seacraft-core/src/wave_pattern.rs::kelvin_wedge_cusp`
- Structure: `nothing_ahead_of_the_bow`, `field_is_symmetric_port_and_starboard`,
  `behind_the_stern_every_row_sees_the_whole_hull`, `field_matches_point_evaluation`,
  `field_is_converged_in_theta`, `catamaran_is_two_shifted_demihulls`, `cuts_match_point_evaluation`, in the
  same file; `michell.rs::michell_column_refactor_keeps_every_number` freezes Michell's $R_w$ across the refactor
  that made its column terms reusable (frozen output, not a reference).

### Numbers Used
Wigley hull of §7 ($L = 10$ m, $B = 1$ m, $T = 0.625$ m, 65 × 25 grid) at Fn 0.30, $g = 9.80665$ m/s², phase step
π/6 between wave-angle samples.

- On the track, 200 points from 40 L to 20 L behind the origin (Nyquist spacing 0.25 m): $|\zeta - \zeta_K| \le
  0.03\,|a_T|/k_0$ at every point; worst found 2.21 %.
- Along the ray ψ = 15°, $h$ from 250 to 450, both wave systems: $|\zeta - \zeta_K| \le 0.03\,(|a_T| + |a_D|)/k_0$;
  worst found 2.84 %.
- The 3 % bound is the asymptote's own error, not quadrature: the worst error falls with distance (on the track
  4.0, 2.1, 1.5, 0.94 % at $h \approx$ 125, 275, 425, 925; at 15° 6.0, 2.8, 1.8, 0.89 %), and is unchanged when the
  phase step is cut to π/24. Its size is that of the hull's length against the distance ($k_0 L / h$), which the
  asymptote treats as a point.
- Transverse wavelength: mean spacing of the up-crossings on the track over 40 L to 20 L behind, 5.65547 m against
  $2\pi U^2/g = 5.65487$ m (0.011 %); the test allows 0.5 %.
- Cusp: a transverse cut 40 L behind the bow, $y$ from 0 to 25 L, 4000 samples; the envelope (largest $|\zeta|$ in
  each 0.25 $\lambda_T$ window) peaks at ψ = 18.76° against ψ_K = 19.47°, and beyond ψ_K + 3° stays below 0.02 % of
  the peak. The test allows a peak from ψ_K − 2° to ψ_K + 0.5° (the Airy peak of a cusp lies inside the caustic,
  closer as $h^{-2/3}$) and 10 % beyond ψ_K + 3° (no stationary wave angle outside the wedge).
- Structure: the grid equals point evaluation to $10^{-10}$ of the largest $|\zeta|$; halving the phase step moves
  the default field by 0.0013 % of its largest $|\zeta|$ (the test allows 1 %); the catamaran equals two shifted
  demihulls to $10^{-10}$.
- Bite checks run while writing the tests: dropping the $1/\pi$, or reversing the sign of the $x$ phase, fails
  both Kelvin tests; scaling the $y$ wavenumber by 1.2 moves the cusp peak to 15.75° and fails the cusp test.

### Provenance
The equations were read from the paper (2026-11-18) and checked against a stationary-phase evaluation of the field
integral; the measured errors above are from the tests' output on 2026-11-18.

**No measured wave pattern.** No measured off-hull wave cut of the ITTC Wigley hull was found to compare with.
Kajitani et al. (1983, §39) report wave-pattern resistance $C_{wp}$ from longitudinal cuts but no cut traces (read
2026-11-18, all 31 pages; the contour plot on p. 16 is wake head loss, not wave elevation). A literature search by
the maintainer found none either. Leads for later: Inui, T., Kajitani, H., Narita, H. & Mori, K. (1969), *J. Soc.
Naval Architects Japan* 126, pp. 35–48 (measured cuts on other mathematical hulls); Insel, M. (1990), PhD thesis,
University of Southampton (Wigley III catamaran cuts, not checked); McCarthy, J. H. (ed., 1985), DTNSRDC-85/011.
Measured off-hull cuts exist for KCS, Series 60 and DTMB 5415, which are outside thin-ship scope or not yet
modelled here.

Not used: the transverse wave cut of Peri, D. (2026), SIREN, arXiv:2610.14027, §5.5, eq. (22). Its author reports
the cut low by a roughly constant factor (cut/pressure 0.38–0.58, Table 7 and the text after it) from a
normalisation constant that was not located, so it cannot supply an independent constant.

What this covers: that the evaluation of the far-field integral agrees with its own published far-field
asymptotics and with Kelvin's geometry (wedge angle, transverse wavelength) for the Wigley hull at Fn 0.30. It is a
check of the method, not a comparison with measurement, and says nothing about the elevation near the hull (free
waves only, no local disturbance), about transom sterns (computed and flagged, §38) or about real catamarans.

## 41. Wave Pattern Readouts: Wash Height, Regime Bands, Kelvin Half-angle

### Sources
- Calculus, no published source: the wash height of a sine, and the Kelvin half-angle from the deep-water
  dispersion relation.
- SeaCraft's own wording bands for the speed regime: a project choice, not published thresholds.

### Formula
- **Wash height** on a longitudinal cut $y = y_c$: the extrema are where the sign of the first difference of the
  samples changes (flat runs skipped); the wash height is the largest $|\zeta_a - \zeta_b|$ over neighbouring
  extrema, at $x = (x_a + x_b)/2$ and distance $|y_c|$. For $\zeta = a\sin(2\pi x/\lambda)$ it is $2a$.
- **Regime** from the Froude number on the waterline length: below 0.30 "low", below 0.40 "rising", up to 0.50
  "around hull speed", above that "past hull speed". Hull speed is shown as Fn 0.40 and labelled a rule of thumb.
  The bands only choose a sentence; no number is computed from them.
- **Kelvin half-angle** in deep water at any speed: $\arcsin(1/3) = 19.4712°$. It is the same ray as §40's cusp,
  $\tan\psi_K = 1/\sqrt 8$.
- **Default grid spacing** of the field: $\min(\lambda_T/20,\ L_{WL}/60)$, coarsened to stay within 250 000 points
  (an explicit spacing keeps the 1 000 000 cap). A resolution choice, not a reference number.

### Test Location
- `crates/seacraft-core/src/wave_pattern.rs::wash_height_of_a_sine_is_twice_its_amplitude`
- `crates/seacraft-core/src/wave_pattern.rs::wash_height_picks_the_largest_neighbouring_pair` (hand-worked)
- `crates/seacraft-core/src/wave_pattern.rs::wash_height_is_none_for_calm_or_transverse`
- `crates/seacraft-core/src/wave_pattern.rs::regime_bands_are_the_projects`
- `crates/seacraft-core/src/wave_pattern.rs::kelvin_half_angle_is_asin_one_third`
- `crates/seacraft-core/src/wave_pattern.rs::default_grid_is_finer_than_a_twentieth_wavelength_at_speed`,
  `default_grid_stays_under_its_cap_at_low_speed`

### Numbers Used
- Sine: $a = 0.2$ m, $\lambda = 5$ m, 2001 samples over 50 m; wash height 0.4 m within $10^{-4}$ m (the sampling
  misses the exact peak by at most $a(1 - \cos(\pi \cdot 0.025/5 \cdot 2))$).
- Hand-worked cut: $\zeta$ = 0, 0.1, −0.05, 0.3, −0.25, 0.02, 0 at $x$ = 0…6 m; 0.55 m at $x$ = 3.5 m.

- **Catamaran interference** $r = R_w(\text{two demihulls at } s) / (2\,R_w(\text{one}))$ from Michell's integral
  (§7). Analytic limit: Michell's factor $2(1 + \cos(k_0 s \sec^2\theta \sin\theta))$ averages to 2 as $s$ grows,
  so $r \to 1$; at Fn 0.40 and $s = 500$ m on the Wigley hull $|r - 1| < 0.05$. At $s$ = 2–5 m some $|r - 1| > 0.05$
  (a property, not a reference). No published catamaran Michell result was read, so the sign of $r$ at a given
  spacing is not checked against one.
- **Wigley $C_w$ curve** over 60 Froude numbers from 0.10 to 1.00 (65 × 25 grid, 128 angles): maxima at Fn 0.176,
  0.207, 0.237, 0.298 and 0.497. The one at 0.298 is §39's hump (0.299 on the finer grid); the one at 0.497 is a
  frozen output of SeaCraft (2026-11-19), pinned to 0.45–0.55, since §39's references stop at Fn 0.45.
  `local_extrema` reports a flat top once and ignores a shelf on a slope.

Tests: `crates/seacraft-core/src/michell.rs::cw_curve_matches_single_calls`, `interference_tends_to_one_far_apart`,
`interference_moves_with_spacing`, `local_extrema_find_humps_and_hollows`,
`wigley_cw_curve_has_a_hump_near_fn_0_5`.

### Provenance
Calculus and project choices, written 2026-11-19. The wash height is a thin-ship estimate of the free waves in deep
water: not a wash assessment against any rule, and not checked against measurement.

## 42. Holtrop-Mennen Against Both Papers' Worked Examples

### Sources
- Holtrop, J. & Mennen, G. G. J. (1982), "An approximate power prediction method", *International Shipbuilding
  Progress* 29(335), 166–170. Worked example on p. 170: a 205 m single-screw ship at 25 kn. Read 2026-11-19.
- Holtrop, J. (1984), "A statistical re-analysis of resistance and propulsion data", *International Shipbuilding
  Progress* 31(363), 272–276. Formulas on pp. 272–274, worked example on p. 275: a 50 m twin-screw ship at
  25–35 kn. Read 2026-11-19, in a retyped course copy.

### Formula
SeaCraft is Holtrop 1984 throughout: the 1984 form factor `1+k1`, the high-speed fit `c17`, `m3` above Fn 0.55, and
the 1984 hump coefficient in **both** wave fits,
$$m_4 = 0.4\,c_{15}\,\exp(-0.034\,F_n^{-3.29}),$$
in $R_{W-A} = c_1 c_2 c_5 \nabla\rho g \exp\{m_1 F_n^d + m_4\cos(\lambda F_n^{-2})\}$ (Fn ≤ 0.40) and
$R_{W-B} = c_{17} c_2 c_5 \nabla\rho g \exp\{m_3 F_n^d + m_4\cos(\lambda F_n^{-2})\}$ (Fn ≥ 0.55), with the paper's
straight-line interpolation between them. Until 2026-11-19 SeaCraft used the 1982
$m_2 = c_{15} C_p^2 \exp(-0.1 F_n^{-2})$ in both fits; with it, $R_W$ in the 1984 example is off by −6 % to +5 %.
Friction (ITTC-57), appendages, bulb, transom and the correlation allowance are the 1982 formulas, which 1984 kept.
Two limits as printed (2026-11-20; `piecewise_coefficients_switch_where_the_paper_says`,
`a_bulb_centre_above_six_tenths_of_the_draft_counts_as_six_tenths`): $c_{15}$ switches on $L^3/\nabla$ at 512 and
1726.91 (1984, p. 273; 1727 in 1982), where its middle branch reaches zero, so it is never positive (the code
switched at $L/\nabla^{1/3} = 12$, i.e. 1728, and gave up to +0.001065); and the bulb centre height $h_B$ is capped
at $0.6\,T$ in $c_3$ and $R_B$, as 1984 p. 273 says it "should not exceed the upper limit of 0.6 $T_F$", with a
warning when the input is higher.

Neither paper prints an upper Froude number (re-read 2026-11-26): 1982 gives none, and 1984 derives the high-speed
fit "for the speed range Fn > 0.55" (p. 273) without a top. The Performance tab therefore greys the Holtrop curve
only for a hull outside the Cp, L/B or B/T range (`HoltropInput::outside_fitted_range`,
`outside_fitted_range_follows_the_form_ranges_not_the_bulb_cap`), never by speed. Those three ranges are not
printed in either paper either: 1982 says only that "the application is limited to hull forms resembling the average
ship described by the main dimensions and form coefficients used in the method" (p. 166, re-read 2026-11-26, no
table of limits on that page). Where SeaCraft's figures come from is not yet cited.

The correlation allowance is taken on $S$, by the 1982 formula. The 1984 example's totals take it on
$S + S_{APP}$; the test adds the appendages' share back and says so.

### Test Location
- `crates/seacraft-core/src/resistance.rs::published_1984_example_reproduces_the_paper`
- `crates/seacraft-core/src/resistance.rs::published_example_reproduces_the_paper` (the 1982 example)
- `crates/seacraft-core/src/resistance.rs::wave_resistance_vanishes_smoothly_at_low_froude_numbers` (a guard:
  $|m_4| < 10^{-12}$ at Fn 0.02–0.1, which the 1982 $m_2$ fails, and no NaN at Fn $10^{-160}$, where $\lambda/F_n^2$
  overflows)
- `crates/seacraft-core/src/resistance.rs::relative_rotative_efficiency_matches_two_printed_examples`
- `crates/seacraft-core/src/resistance.rs::wake_fraction_follows_holtrop_1984_single_screw` (hand calculation)

### Numbers Used
SeaCraft's constants throughout (g 9.80665 m/s², knot 0.514444 m/s, ν 1.1883e-6 m²/s); the papers use 9.81 and
0.5144, a difference under 0.1 % that the tolerances absorb. Expected values were computed in Python on 2026-11-19.

**1984 example.** L 50 m, B 12 m, T 3.2 m (mean of TF 3.1, TA 3.3), ∇ 900 m³, CM 0.78, CWP 0.80, lcb 4.5 % of L
aft of ½L, iE 25°, AT 10 m², no bulb, Cstern 0, S 584.9 m², SAPP 50 m², 1+k2 3. Printed and asserted: LR 14.1728 m
(rel 1e-4); 1+k1 1.297, c17 1.4133, m3 −2.0298, c5 0.7329, λ 0.7440 (rel 1e-3). The printed table lost its minus
signs (on c15, on every $m_3 F_n^d$, and on $m_4\cos$ from 31 kn); the test restores them.

| kn | $m_4\cos$ | $m_3 F_n^d$ | $R_W$ printed / expected | $R_{APP}$ | $R_{TR}$ | $R$ printed / expected (+ RA on SAPP) |
|---|---|---|---|---|---|---|
| 25 | 0.3279 | −3.3100 | 475 / 474.98 | 21 | 25 | 662 / 661.74 |
| 27 | 0.1820 | −3.0883 | 512 / 512.33 | 24 | 16 | 715 / 715.19 |
| 29 | 0.0409 | −2.8962 | 539 / 539.18 | 28 | 2 | 756 / 756.09 |
| 31 | −0.0834 | −2.7274 | 564 / 563.75 | 31 | 0 | 807 / 807.43 |
| 33 | −0.1876 | −2.5780 | 590 / 589.73 | 35 | 0 | 864 / 864.15 |
| 35 | −0.2730 | −2.4453 | 618 / 618.42 | 39 | 0 | 925 / 925.32 |

Forces in kN. Tolerances: the two products 1e-3 absolute; $R_W$ and $R$ 1.0 kN; $R_{APP}$ and $R_{TR}$ 0.6 kN (printed
to the kN). Every speed is on the high-speed fit (Fn 0.58–0.81), which the test also asserts. Without the RA share on
SAPP, $R$ comes out 3–5 kN low.

**1982 example.** L 205 m, B 32 m, T 10 m, ∇ 37 500 m³, Cp 0.5833, CM 0.98, CWP 0.75, lcb 0.75 % of L aft of ½L,
iE 12.08°, AT 16 m², ABT 20 m², hB 4.0 m, Cstern 10, S 7381.45 m², SAPP 50 m² with 1+k2 1.5, 25 kn. Every term the
re-analysis kept reproduces as before: Fn 0.2868, CF 0.001390, RF 869.63 kN, c7 0.15610, c1 1.398, c2 0.7595,
c5 0.9592, λ 0.6513, m1 −2.1274 (rel 1e-3 or tighter), RAPP 8.83 kN, RA 221.98 kN (1 %), RTR 0 and RB below 0.1 kN
as printed. LR 81.385 m (rel 1e-4). Two numbers differ by the formula choice, not by transcription:
- 1+k1: 1.1851 from the 1984 formula (asserted, rel 1e-3); the 1982 formula
  $c_{13}\{0.93 + c_{12}(B/L_R)^{0.92497}(0.95 - C_p)^{-0.521448}(1 - C_p + 0.0225\,lcb)^{0.6906}\}$ gives 1.1564
  here against 1.156 printed.
- $R_W$: 554.02 kN with $m_4$ against 557.11 printed (−0.55 %; the 1982 $m_2$ gives 556.93). The test allows 1 %
  against the print, and 1e-4 against the Python expectation with $m_4$ ($m_4$ −0.085549 at Fn 0.286841, also
  asserted).
- The total follows: 1814.06 kN against 1793.26 printed (+1.16 %); the test allows 1.5 %.

Bite check (2026-11-19): with $m_2$ put back, the 1984 test fails at 25 kn ($m_4\cos$ 0.2697 against 0.3279).

**Propulsion factors** (added 2026-11-20). $\eta_R = 0.9922 - 0.05908\,A_E/A_0 + 0.07424\,(C_p - 0.0225\,lcb)$,
the 1982 formula that 1984 kept, against two printed values: the 1982 example (Cp 0.5833, lcb −0.75 %, $A_E/A_0$
0.7393 as printed) gives 0.99308 against 0.9931 printed (test 1e-4); Molland, Turnock & Hudson (2011) §17.2.6.2
(BAR 0.700, Cp 0.663, LCB −1.5 %) gives 1.00257 against 1.003 printed (test 1e-3). An earlier test back-solved
$A_E/A_0$ 0.775 from the printed ηR with the wrong lcb −2.02 and checked nothing.

The single-screw wake fraction is Holtrop (1984), pp. 273–274, as Molland et al. (2011) eq. 8.17 also print it:
$w = c_9 c_{20} C_V (L/T_A)(0.050776 + 0.93405\,c_{11} C_V/(1 - C_{P1})) + 0.27915\,c_{20}\sqrt{B/(L(1 - C_{P1}))}
+ c_{19} c_{20}$, with $c_{19} = 0.12997/(0.95 - C_B) - 0.11056/(0.95 - C_p)$ for $C_p \le 0.7$ and
$0.18567/(1.3571 - C_M) - 0.71276 + 0.38648\,C_p$ above. Until 2026-11-20 the code had
$c_9 C_V\sqrt{C_V L/T_A}(\ldots)$ without $c_{20}$, and switched $c_{19}$ on $B/T_A$ with a different first form.
No printed example uses these formulas (the 1982 example prints $w$ 0.2584 from the 1982 formula; the 1984 example
is twin-screw), so the test checks a hand calculation in Python from the printed formulas, not a published
number: the 1982 ship with $C_V$ 0.001963 and D 8 m gives $w$ 0.274120 and $t$ 0.198376; a full hull on the
$C_p > 0.7$ branch (L 150, B 25, T 9 m, S 5500 m², D 6.5 m, Cp 0.80, CM 0.99, CB 0.792, lcb +2 %, $C_V$ 0.0021)
gives $w$ 0.392322. Tolerance 1e-5. With the old wake the test fails ($w$ 0.230213).

### Provenance
Both papers read 2026-11-19; the expected values recomputed in Python the same day and reproduced by the tests.
Earlier the 1982 test fed lcb 2.02 % aft and called the 1+k1 gap a misprint in the paper's equation (9); both were
wrong, and it is the 1984 against the 1982 form factor.

What this covers: the transcription of the resistance equations, checked against the examples their authors
printed. It does not show that the method predicts any real vessel's resistance. Of the propulsion factors, ηR
is checked against two printed values; the wake fraction and thrust deduction only against the printed formulas.

## 43. Savitsky Against Molland et al. (2011), Table 10.6

### Sources
- Molland, A. F., Turnock, S. R. & Hudson, D. A. (2011), *Ship Resistance and Propulsion*, Cambridge University
  Press, §10.3.3.2, pp. 216–219: the Savitsky equations (eqs. 10.43–10.48) and a worked example (Table 10.6). Read
  2026-11-19.
- Savitsky (1964), Molland's source, was not read. The oracle rests on Molland.

### Formula
As in `planing.rs`: $C_{L\beta} = 2C_\Delta/C_V^2$, $C_{L\beta} = C_{L0} - 0.0065\,\beta\,C_{L0}^{0.6}$,
$C_{L0} = \tau^{1.1}(0.0120\lambda^{0.5} + 0.0055\lambda^{2.5}/C_V^2)$,
$l_p/b = \lambda(0.75 - 1/(5.21 C_V^2/\lambda^2 + 2.39))$, $D_F = \tfrac12\rho V^2 (\lambda b^2/\cos\beta)\,C_F$,
$R = \Delta\tan\tau + D_F$ (plus the roughness allowance $\Delta C_F$ 0.0004 in production).

Change on 2026-11-19: $C_F$ (ITTC-57) now takes the Reynolds number on the mean wetted length $\lambda b$, as
Molland does. It was taken on $\lambda b/\cos\beta$, which gave $C_F$ 0.8 % low in this example. Change on
2026-11-20: `wetted_length_m` reports $\lambda b$ (it reported $\lambda b/\cos\beta$, which is no length: it is
the wetted area over the beam), and `wetted_area_m2` reports $S = \lambda b^2/\cos\beta$, on which the friction
acts.

Two model differences remain, stated rather than removed:
- **Moment balance.** SeaCraft solves LCP = LCG. Molland solves $N l_p = \Delta\,LCG$ with $N = \Delta\cos\tau$.
  Here that moves the trim from 3.31° to 3.318°, and λb by −0.4 %.
- **Drag resolution.** SeaCraft has $R = \Delta\tan\tau + D_F$; Molland resolves
  $R_T = (\Delta\sin\tau + D_F)\cos\tau$. The two differ by 0.2 % here.

### Test Location
- `crates/seacraft-core/src/planing.rs::molland_savitsky_example_reproduces_table_10_6`

### Numbers Used
70 t craft (Δ = 686 700 N, 70 t × 9.81), chine beam 6.5 m, deadrise 20°, LCG 10.0 m forward of the transom, 45 kn
(V 23.15 m/s), ρ 1025 kg/m³, ν 1.19e-6 m²/s. SeaCraft's g is 9.80665 (Molland 9.81). Molland takes no roughness
allowance, so the test subtracts SeaCraft's from the drag.

| Quantity | Molland | SeaCraft | Tolerance |
|---|---|---|---|
| $C_{L0}$ | 0.0898 | 0.08979 | 0.2 % |
| λ at τ 2.0° | 3.8083 | 3.8053 | 0.2 % |
| λ at τ 3.0° | 2.6310 | 2.6315 | 0.2 % |
| λ at τ 4.0° | 1.8701 | 1.8703 | 0.2 % |
| λ at τ 3.5° | 2.2138 | 2.2146 | 0.2 % |
| λ at τ 3.31° | 2.3658 | 2.3634 | 0.2 % |
| equilibrium τ | 3.31° | 3.3177° | 0.02° |
| λb (`wetted_length_m`) | 15.378 m | 15.322 m | 0.5 % |
| S (`wetted_area_m2`) | 106.37 m² | 105.98 m² | 0.5 % |
| Re | 2.9916e8 (V λb/ν from the printed λb) | 2.9807e8 | 0.5 % |
| $C_F$ | 1.7884e-3 | 1.7893e-3 | 0.5 % |
| $D_F$ | 52.24 kN | 52.08 kN | 0.5 % |
| $R_T$ | 91.74 kN | 91.89 kN | 0.5 % |

Bite check (2026-11-19): with the Reynolds number on λb/cos β, as before the change, the test fails on $C_F$
(1.774e-3 against 1.7884e-3).

### Provenance
Molland read 2026-11-19; Python recomputation the same day reproduced the λ rows to 0.11 %; the SeaCraft column is
the test's output on 2026-11-19.

What this covers: the transcription of the Savitsky equations as Molland states them, for one craft at one speed.
It does not show that the method predicts any real craft's resistance, and it does not cover spray or whisker
drag, appendages, or Savitsky's mean bottom velocity (Molland uses V in the Reynolds number, and so does SeaCraft).

## 44. Wigley Hydrostatics over Drafts against the Closed Forms

### Sources
- **Hull equation, published.** Kajitani et al. (1983), p. 8, as cited in §39:
  $y = \pm(B/2)(1 - \xi^2)(1 - \zeta^2)$, $\xi = 2x/L - 1$, $\zeta = (T - z)/T$ (the report's $D$ is the
  draft, here $T$). SeaCraft takes it wall-sided above $T$ to a flat deck at $D$, as `examples/wigley.fef` does.
  $L$ 10 m, $B$ 1 m, $T$ 0.625 m, $D$ 1 m, $\rho$ 1025 kg/m³.
- **Wetted surface, published.** $S = 0.661\,L(2T + B) = 14.8725$ m² (Nomenclature, p. 6). The coefficient is
  printed to three figures, so half a unit in its last place is ±0.076 %.
- **Closed forms, not published.** Our own integration of the equation, written 2026-11-20.

### Formula
With $\tau = t/T$ at draft $t \le T$:

- $\nabla = \tfrac23 LBT(\tau^2 - \tau^3/3)$; $KB = T(\tfrac23\tau^3 - \tfrac14\tau^4)/(\tau^2 - \tau^3/3)$
- $A_{wp} = \tfrac23 LB(2\tau - \tau^2)$; $I_T = \tfrac23 (B/2)^3 (2\tau - \tau^2)^3 \cdot \tfrac{L}{2} \cdot \tfrac{32}{35}$;
  $I_L = B(2\tau - \tau^2)(L/2)^3 \cdot \tfrac{4}{15}$; $BM = I/\nabla$
- $A_m = BT(\tau^2 - \tau^3/3)$; $LCB = LCF = L/2$

Above $T$ the sides are vertical: $\nabla = \tfrac49 LBT + \tfrac23 LB(t - T)$, the moment about the keel gains
$\tfrac23 LB(t^2 - T^2)/2$ over $\tfrac49 LBT \cdot \tfrac58 T$, $A_{wp}$, $I_T$, $I_L$ stay at their $\tau = 1$
values, and $A_m = \tfrac23 BT + B(t - T)$. At $T$ these give $\nabla = \tfrac49 LBT$, $KB = 0.625\,T$,
$BM_T = 9B^2/(105\,T)$, the forms in the header of `examples/wigley.rs`.

Two levels are measured, so a failure says which part is wrong:

- **Exact mesh**: the equation faceted directly (400 columns, 120 rows to $T$, rows at 0.8 m and $D$, diagonals
  mirrored at midship), no subdivision, measured by the production `Hull`. This checks the measurement.
- **The example**: `examples/wigley.fef` refined as the CLI measures it (Loop, level 3). Its control rows were
  scaled to match the closed forms at $T$ only, so off $T$ this measures the model's fit, not the code.

### Test Location
- `crates/seacraft-core/tests/wigley_hydro.rs::the_closed_forms_reduce_to_the_design_draft_values`
- `crates/seacraft-core/tests/wigley_hydro.rs::exact_mesh_matches_the_closed_forms_over_drafts`
- `crates/seacraft-core/tests/wigley_hydro.rs::exact_mesh_wetted_surface_matches_kajitani`
- `crates/seacraft-core/tests/wigley_hydro.rs::example_hull_follows_the_closed_forms_off_design`
- `crates/seacraft-core/tests/wigley_hydro.rs::example_wetted_surface_matches_kajitani`

### Numbers Used
Drafts 0.25T, 0.5T, 0.75T, T and 0.8 m (on the wall-sided topside); ∇, KB, $A_{wp}$, $BM_T$, $BM_L$, $A_m$,
LCB, LCF.

- **Exact mesh**, within 0.01 %, twice the worst measured gap: at most 0.0051 % ($A_m$ at 0.25T, where the keel's
  facets are coarsest against the section); volume within 0.002 %, KB 0.001 %. LCB and LCF at $L/2$ within
  $10^{-6}L$. Wetted surface at $T$: 14.879 m², +0.044 % on Kajitani's 14.8725 (0.1 % allowed, which covers the
  coefficient's ±0.076 % rounding).
- **The example**, gap in %, measured 2026-11-20. Each band is the gap rounded up to the next 0.1 % plus 0.1 %:

  | draft | volume |  KB   |  $A_{wp}$ | $BM_T$ | $BM_L$ | $A_m$ |
  |-------|--------|-------|-------|-------|-------|-------|
  | 0.25T | −5.55  | +1.03 | −2.11 | −0.39 | +3.35 | −5.40 |
  | 0.5T  | −1.86  | +0.92 | −0.01 | +2.16 | +1.59 | −1.71 |
  | 0.75T | −0.83  | +0.64 | +0.31 | +2.06 | +0.85 | −0.67 |
  | T     | −0.48  | +0.44 | −0.07 | +0.56 | +0.13 | −0.33 |
  | 0.8 m | −0.34  | +0.32 | −0.01 | +0.61 | +0.04 | −0.18 |

  Wetted surface at $T$: +0.041 % on Kajitani's value (0.2 % allowed). LCB and LCF at $L/2$ within $10^{-6}L$.

  At 0.25T the gaps pass 3 % (volume −5.5 %). They are recorded as bands rather than refitting the example
  (project decision, 2026-11-20). Loop refinement pulls the limit surface in near the keel, so the
  example is a design-draft fit that runs thin below about 0.5T.

### Provenance
The equation and $S$ are published (read 2026-11-18, §39); the closed forms and the bands are our own, written
2026-11-20. This covers one hull form (parabolic, L/B 10, B/T 1.6), upright and level, at drafts 0.16–0.8 m, with
no appendages. It is not a comparison with a real hull or with approved software.

## 45. Wigley GZ to 80° against an Independent Integration

### Sources
- **Hull equation, published.** Kajitani et al. (1983), p. 8, as in §44.
- **KN table, independent computation, not published.** `docs/oracles/wigley_gz.py`, standard-library Python
  that shares no code with SeaCraft, run 2026-11-20. No published KN or GZ for this hull was found.

### Formula
Body axes x forward, y to starboard, z up from the keel. Heeled $\varphi$ to starboard, a point is under water when
$z\cos\varphi - y\sin\varphi \le c$. Each section (2000 intervals a side, the deck at $D$) is clipped by that
half-plane (Sutherland–Hodgman) and integrated for area and centroid; Simpson's rule over 400 intervals in $x$
sums them (the 201 stations of the aft half, doubled, since the hull is symmetric fore and aft).
$c$ is solved for $\nabla = \tfrac49 LBT$ to $10^{-10}\nabla$. Then

$$KN = y_B\cos\varphi + z_B\sin\varphi, \qquad GZ = KN - KG\sin\varphi,$$

with K on the keel at the centreline, as `stability.rs` reports it. The hull is symmetric fore and aft, so it
stays level in trim at every heel and no trim is solved for. The script asserts its own checks: upright ∇ and KB
match §44 to $10^{-6}$; KN(0) = 0; KN(5°) within 1 % of $KM\sin 5°$; halving both resolutions moves no KN by
more than $10^{-5}$ m (measured $9 \times 10^{-8}$ m).

### Test Location
- `crates/seacraft-core/tests/wigley_hydro.rs::exact_mesh_gz_matches_the_independent_script`
- `crates/seacraft-core/tests/wigley_hydro.rs::example_gz_follows_the_independent_script`
- `crates/seacraft-core/tests/wigley_hydro.rs::example_free_trim_stays_level_and_gz_is_odd`
- `crates/seacraft-core/tests/wigley_hydro.rs::small_angle_gz_is_gm_sin_phi`

### Numbers Used
Δ = 2.8472 t (∇ at $T$, ρ 1025), KG 0.35 m (the example's). The midship deck edge goes under at 37.7°, so the
rows from 40° on are taken past deck immersion.

| heel (°) | 0 | 5 | 10 | 20 | 30 | 40 | 50 | 60 | 70 | 80 |
|---|---|---|---|---|---|---|---|---|---|---|
| KN (m) | 0 | 0.04602 | 0.09182 | 0.18201 | 0.26937 | 0.35368 | 0.42846 | 0.48813 | 0.53297 | 0.56450 |
| GZ (m) | 0 | 0.01552 | 0.03105 | 0.06230 | 0.09437 | 0.12870 | 0.16035 | 0.18502 | 0.20408 | 0.21982 |

- **Exact mesh** (§44), heeled by `cross_curves` at fixed zero trim: within $5 \times 10^{-5}$ m at every heel;
  measured at most $4.7 \times 10^{-6}$ m, the table's rounding.
- **The example**: KN above the script by +0.59, +0.63, +0.69, +0.71, +0.70, +0.62, +0.55, +0.46 and +0.33 % of KN
  from 5° to 80° (at most 2.7 mm), the same sign as its $BM_T$ gap at $T$ (§44). Band 0.9 % of KN, the largest
  rounded up to the next 0.1 % plus 0.1 %.
- **Free trim**, on the example floating at LCG $L/2$, the solve started 0.05 m out of trim so that it iterates:
  trim within $10^{-5}$ m of level at every heel and KN within $10^{-6}$ m of the fixed-trim KN (the solver stops
  at $|LCB - LCG| < 10^{-6}$ m; measured trim at most $6 \times 10^{-12}$ m). **Heel sign**: GZ at −10°, −30°,
  −50° is minus GZ at +10°, +30°, +50° within $10^{-6}$ m (the mirrored half model; measured about $10^{-15}$ m).
- **Small angle**, exact mesh: GZ(5°) = 0.01552 m within 0.5 % of $GM\sin 5° = 0.01549$ m (+0.19 %),
  $GM = KM - KG$ from §44. The term $GM\sin\varphi$ drops is $BM/(2\,GM)\tan^2\varphi = 0.29$ % at 5° for a
  wall-sided hull.

### Provenance
An independent integration of the published equation, written and run 2026-11-20: not an experiment, not a
published table and not approved software. It covers one hull, at one displacement, heeled to starboard up to 80°
(to port at 10°, 30° and 50°) with no trim, KG 0.35 m; it says nothing about hulls with a transom, a chine or a skeg, or about
trimmed conditions.

---

## 46. Strip-Theory Heave and Pitch against Journée (1992), Wigley I and III

### Source
- J.M.J. Journée, *Experiments and Calculations on 4 Wigley Hull Forms in Head Waves*, Delft University of
  Technology, Ship Hydromechanics Laboratory, Report 0909, May 1992 (reprinted 2003). Read 2026-11-20. The TU
  Delft repository record is closed; the copy read was the Internet Archive's capture of the author's site
  (`shipmotions.nl`, `0909-DUT-92.pdf`).
- Used: §2 (pp. 2 to 3, particulars and the hull formula), §4.1 to §4.3 (pp. 5 to 9, how each quantity was reduced
  and made non-dimensional), Appendix I (axes), and Tables 2-I, 2-III (pp. 33, 35), 3-I, 3-III (pp. 37, 39), 6-I,
  6-III (pp. 45, 47), 8-I, 8-III (pp. 51, 53). Every number in the tests is a table value as printed. The figures
  are scanned and no test or band uses a number read from them.

### What is compared
Fresh water (ρ 1000 kg/m³, g 9.81 m/s²), L 3 m, draft 0.1875 m, Fn 0.2, 0.3, 0.4 (U 1.085, 1.627, 2.170 m/s), head
seas, models not free to surge. Wigley I (α 1, ∇ 0.0946 m³) and Wigley III (α 0, ∇ 0.0780 m³), both B 0.3 m,
KG 0.17 m, kyy 0.75 m; the only two of the four models run at all three speeds. Each part is checked on its own, so
that a gap in the motions can be traced to the radiation side or the excitation side:

1. **Radiation coefficients** (forced heave at 0.025 and 0.050 m, forced pitch at 1.5° and 3.0°; Tables 2, 3):
   A33, B33, A53, B53, A55, B55, A35, B35 at the encounter frequencies 3 to 12 rad/s (also 1 and 2 rad/s on
   Wigley III at Fn 0.2), against `GlobalCoefficients` at the wave frequency that meets the hull at that ω_e
   (the positive root of ω + ω²U/g = ω_e).
2. **Wave loads on the restrained model** (Table 6, wave amplitudes about 2.5 and 4.5 cm, λ/L 0.5 to 2.0): the
   heave force and pitch moment, against the new `WaveLoads` that `head_sea_raos_on_hull` returns (the
   excitation the motions are solved with: Froude–Krylov, diffraction, the speed term).
3. **Motions of the free model** (Table 8, λ/L 0.5 to 2.37): heave and pitch amplitude and phase.

Non-dimensional forms, divided by Journée's printed ∇, C33 6119 N/m and C55 2874 N·m (never SeaCraft's):
A33″ = A33/(ρ∇), B33″ = B33/(ρ∇√(g/L)); A35″, A53″ = A/(ρ∇L), B35″, B53″ = B/(ρ∇L√(g/L)); A55″ = A55/(ρ∇L²),
B55″ = B55/(ρ∇L²√(g/L)); Xw3″ = |F3|/(ζa C33), Xw5″ = |F5|/(k ζa C55); z″ = z/ζa; θ″ = θa L/(2π ζa), which is
SeaCraft's `pitch_amp` × L/λ. Phases are relative to the wave crest at the centre of gravity in both.

**Sign map.** Journée's axes (Appendix I) make pitch positive bow down; SeaCraft's is bow up. So pitch and
pitch-moment phases differ by 180° and A35, B35, A53, B53 change sign; A33, B33, A55, B55 and the heave phases do
not. Proved, not assumed: in a λ/L = 20 wave on Wigley I at Fn 0.2 SeaCraft's heave phase is −0.0° and its pitch
phase plus 180° is 270.4° (bow down on the wave slope); at λ/L 2 they are 358° and 261.6° against the measured 360°
and 265°; and B53″, B35″ on Wigley I at Fn 0.3 have the measured signs at every frequency of 5 rad/s and above.

**C55 ruling.** §4.1 reduces A55 = −Xo5a cos ε/(θa ω²) + C55/ω² − M55*, with C55 = 2ρg∫y_w x² dx (2874 N·m)
alone, the model turning about KR = 0.1875 m (the waterline). It says nothing of compensating the buoyancy and
weight moments about KR, and does not give the forced model's mass or KG. If the model weighed ρ∇ with G at the
printed KG, his A55 holds an offset ρg∇(KB − KG)/ω² (−58.6 N·m on Wigley I, −40.4 N·m on Wigley III). A55 is
compared **as printed**; the test prints the corrected value beside it. The correction does not explain the gap:
it makes the low-frequency gaps larger, and the tank's excess over strip theory is roughly constant in ω (about
0.008 in A55″ on Wigley I at Fn 0.3, 6 to 10 rad/s), not 1/ω².

### The exact hulls
`tests/journee_1992/mod.rs` meshes the formula directly (no control net, no subdivision): 200 columns over L,
40 rows below the waterline, wall-sided above it (the ζ = 0 waterline carried straight up), run through
`head_sea_raos_on_hull`, the same station cutting, close-fit sections and strip integration as
`head_sea_raos`. Against the formula integrated by hand (not published): volume 4.2e-4 (I) and 1.4e-4 (III) low,
KB within 2.9e-4, waterplane area 2.9e-5, I_L 5.8e-5; ρg·Awp and ρg·I_L are within 0.1 % of the printed C33 and
C55, and the volumes within the printed four decimals. Convergence on Wigley I at Fn 0.3 (motions at λ/L 0.75 to
2.0, B33″ and B55″ at 4 to 12 rad/s): 21 → 41 stations moves the values 1.9 % and 2.4°, 41 → 81 0.48 % and 0.59°,
81 → 161 0.12 % and 0.14°; mesh (100, 20) → (200, 40) 0.62 %, (200, 40) → (400, 80) 0.15 % and 0.10°. The tests use
81 stations on the (200, 40) mesh, and `stations_and_mesh_have_converged` holds both steps beyond it to 0.5 %
and 0.5°.

### Gaps and bands
A gap is |SeaCraft − tank| / max(|tank|, floor), the floor being 10 % of the largest measured |value| of the group
(model, Fn, quantity); a phase gap is in degrees, compared only where both the tank's and SeaCraft's amplitudes
reach the floor. Each band is the largest gap of its group over every row and both amplitudes, rounded up to the
next 0.01 (phases: the next 2°), measured 2026-11-20. The bands guard against change; they are not a statement
that strip theory is that accurate on any other hull.

**Radiation coefficients**, ω ≥ 4 rad/s:

| | A33 | B33 | A53 | B53 | A55 | B55 | A35 | B35 |
|---|---|---|---|---|---|---|---|---|
| Wigley I, Fn 0.2 | 0.11 | 0.61 | 1.41 | 1.32 | 0.33 | 0.53 | 2.32 | 2.91 |
| Wigley I, Fn 0.3 | 0.08 | 0.65 | 2.89 | 0.90 | 0.35 | 1.03 | 2.13 | 1.19 |
| Wigley I, Fn 0.4 | 0.13 | 0.70 | 4.62 | 0.93 | 0.37 | 1.78 | 1.43 | 0.36 |
| Wigley III, Fn 0.2 | 0.17 | 0.30 | 1.08 | 2.00 | 0.22 | 0.57 | 1.02 | 1.82 |
| Wigley III, Fn 0.3 | 0.16 | 0.29 | 2.83 | 1.52 | 0.33 | 0.91 | 2.00 | 1.73 |
| Wigley III, Fn 0.4 | 0.19 | 0.45 | 3.98 | 1.50 | 0.35 | 1.50 | 1.60 | 1.15 |

At 4 to 12 rad/s A33 is within 8 to 19 % (at 3 rad/s, unbanded, up to 60 % off). The large gaps follow strip
theory's known pattern, and Journée's own strip-theory curves show it too (his Figures 2-I, 5-I, 6-I and 6-III,
read by hand for this purpose only): B55 1.5 to 2.7 times the tank at 4 to 7 rad/s, growing with speed (the
U²/ω_e²·B33 term), and about half the tank at 12 rad/s; B33 too low at 10 and 12 rad/s (Wigley I, Fn 0.3: 0.37 and
0.17 against 0.57 and 0.45); A55 about 30 % low from about 6 rad/s (0.0157 against 0.0235; at 4 rad/s it is up to
about 30 % high instead). His curves were not read at every point: at ω_e ≈ 4 rad/s and Fn 0.3 to 0.4 SeaCraft's B55 lies
above them as well as above the tank. The couplings: on these fore-and-aft symmetric hulls without a transom
SeaCraft's A53 is exactly U·B33/ω_e² and A35 its negative, while the tank's A53 is about half of U·B33/ω_e²
formed from the tank's own B33 even where it is positive (0.028 against 0.064 on Wigley I, Fn 0.2, 4 rad/s). That
speed-coupling gap, measured against a 10 % floor, gives bands above 1. The rows at ω ≈ 3
rad/s (and 1, 2 rad/s on Wigley III at Fn 0.2) are printed but not banded: there τ = ω_e U/g is 0.33 to 0.66,
around strip theory's limit of 1/4, the measured coefficients jump between neighbouring frequencies, and their
gaps (up to 23 times the floor) would loosen the bands past catching a doubled coefficient. The one stop rule kept
is the sign of B35 and B53 above their floor (they carry ±U·A33, so a wrong pitch sign shows at every frequency).
A35 and A53 are banded only: the tank's A53 on Wigley I at Fn 0.4 turns negative (−0.009 to −0.015) from about
7 rad/s, where strip theory's U·B33/ω_e² cannot (it also dips just below zero, under the floor, at Fn 0.2 and 0.3
from 6 to 7 rad/s).

**Wave loads on the restrained model:**

| | Xw3″ | ε3 | Xw5″ | ε5 |
|---|---|---|---|---|
| Wigley I, Fn 0.2 | 0.24 | 30° | 0.13 | 44° |
| Wigley I, Fn 0.3 | 0.52 | 46° | 0.31 | 48° |
| Wigley I, Fn 0.4 | 0.69 | 24° | 0.57 | 82° |
| Wigley III, Fn 0.2 | 0.59 | 22° | 0.13 | 28° |
| Wigley III, Fn 0.3 | 0.73 | 30° | 0.25 | 22° |
| Wigley III, Fn 0.4 | 0.71 | 22° | 0.43 | 14° |

From λ/L 1.25 up, at Fn 0.2 and 0.3 the heave force is within 15 % and the pitch moment within 24 %; at Fn 0.4
within 32 % and 43 % (both worst at λ/L 1.25, where on Wigley III the tank's two wave amplitudes give Xw5″ 0.397
and 0.328). The large amplitude
gaps are in short waves, measured against the floor: on Wigley III at λ/L 0.5 and 0.75 SeaCraft's heave force
falls into the cancellation of the Froude–Krylov and diffraction parts (Xw3″ 0.025 to 0.040 against the tank's
0.051 to 0.12), as Journée's strip theory does at Fn 0.4, λ/L 0.75 (his Figure 13-III). There the computed phase
swings fast, so the ε3 rows where SeaCraft's amplitude is below the floor (λ/L 0.75 at every speed, λ/L 0.5 at
Fn 0.3 and 0.4) are not compared (the SeaCraft side of the floor rule; at Fn 0.4, λ/L 0.75 it is 104° from the tank). The
82° on Wigley I at Fn 0.4 is at λ/L 0.75, where the tank's two wave amplitudes themselves differ by 48°.

**Motions:**

| | z″ | ε_z | θ″ | ε_θ |
|---|---|---|---|---|
| Wigley I, Fn 0.2 | 0.49 | 16° | 0.35 | 24° |
| Wigley I, Fn 0.3 | 0.19 | 34° | 0.54 | 32° |
| Wigley I, Fn 0.4 | 0.33 | 40° | 0.81 | 40° |
| Wigley III, Fn 0.2 | 0.47 | 48° | 0.32 | 22° |
| Wigley III, Fn 0.3 | 0.87 | 42° | 0.68 | 44° |
| Wigley III, Fn 0.4 | 0.47 | 22° | 1.15 | 58° |

In waves of λ/L ≥ 1.25 at Fn 0.2 and 0.3, away from resonance (more than 0.15 in λ/L from the tank's peak), heave
is within 30 % and 30° on both hulls and pitch on Wigley III, and the test keeps that as a stop rule. Shorter
waves away from resonance are not that close (Wigley III, Fn 0.3, λ/L 1.0: θ″ 1.24 against 0.74). The rest follows
strip theory's known behaviour, and Journée's own strip-theory curves (his Figures 16-III, 17-I, 17-III, read by
hand) show the same: the pitch resonance comes at shorter waves than the tank's (Wigley I, Fn 0.3: λ/L 1.25 with
θ″ 1.83, against the tank's 2.00 at λ/L 1.5; Wigley III, Fn 0.4: at λ/L 1.25 SeaCraft gives 1.80 where the tank
gives 0.84, the tank's own peak being 1.80 at λ/L 1.5), traced to the A55 about 30 % low at the same encounter
frequencies; Wigley I's pitch peak at Fn 0.4 is overpredicted (2.27 at λ/L 1.4, against the tank's highest 1.35),
and so is Wigley III's heave peak at Fn 0.3 and 0.4 (Fn 0.4, λ/L 1.25: z″ 2.51 against 1.86). Three rows are named and left out of the stop rule
(banded only): Wigley I pitch at Fn 0.3, λ/L 1.25 and 1.75 (the resonance shift), and at Fn 0.2, λ/L 1.75, where the
tank's θ″ of 1.09 stands above both neighbours (0.95 and 0.68). Short-wave rows (λ/L ≤ 0.75) have the largest
relative gaps against the floor (Wigley III, Fn 0.2, λ/L 0.75: z″ 0.13 against 0.25).

**The tank's own spread.** Each table with two amplitudes measures one linear quantity twice. At 4 rad/s and above
the direct coefficients A33, B33, A55, B55 of the two amplitudes differ by a median of 1 to 6 % per group and at
most 19 % (Wigley III, Fn 0.4, B33). The wave loads differ by a median of 2 to 5 %, but up to 91 % on Wigley I
(Xw3″ at Fn 0.4, λ/L 1.0: 0.108 and 0.206) and 31 % in Xw5″ (Fn 0.3, λ/L 1.5: 0.464 and 0.537). A band takes the
larger of the two gaps, so one scattered point can set it (Wigley I, Fn 0.2, B35″ at 4 rad/s: 0.101 and 0.015).

### Test locations
- `crates/seacraft-core/tests/wigley_seakeeping.rs`: `helpers_handle_roots_and_wraps`,
  `exact_hulls_match_the_formula_and_the_printed_particulars`, `tables_are_consistent`,
  `signs_and_phases_follow_journee`, `coupling_signs_match_after_mapping`, `stations_and_mesh_have_converged`,
  `radiation_coefficients_against_forced_oscillation`, `wave_loads_against_the_restrained_model`,
  `motions_against_the_free_model`; the tables and helpers in `tests/journee_1992/`.
- `crates/seacraft-core/src/seakeeping.rs`: `hull_entry_point_matches_the_surface_path` (the new entry point gives
  bit for bit what `head_sea_raos` gives, transom terms included), `hull_entry_point_solves_upright_and_level` (a
  trim or heel in the caller's options is ignored, as the strips are cut level) and
  `wave_loads_reproduce_the_motions` (the 2×2 system re-solved from the returned coefficients and loads gives the
  RAOs back to 1e-9).

### Provenance
The tables were transcribed page by page from the text layer, the blanks checked against the page images, and
re-extracted and compared number by number (377 rows, 0 differences), 2026-11-20. Bite checks, 2026-11-20: SeaCraft's
A33 scaled by 1.05 fails the Wigley I Fn 0.4 A33 band; the coupling sign map flipped fails 183 checks; the pitch-moment
phase without its 180° shift fails ε5 in every group; kyy 0.80 m instead of 0.75 fails eight heave bands.

What this covers: SeaCraft's strip theory on two slender Wigley hulls (L/B 10, B/T 1.6) in regular head waves at
Fn 0.2 to 0.4, compared with Journée's (1992) towing-tank measurements, part by part. It shows where strip theory
departs from the tank on these hulls and freezes those gaps; it does not show the method is accurate for other
hulls, oblique seas, other speeds, roll, or any hull with a transom, flare or bulb.

---

## 47. Box Barge Heel under an Off-Centre Weight against the Wall-Sided Formula

### Source
**Derived, not published.** The wall-sided formula is worked from first principles for this test (2026-11-20): heel a
wall-sided section by φ about the upright waterline's centre; the immersed and emerged wedges are equal triangles of
half-breadth `B/2`, and their shift moves B by `(B²/12T) tan φ` across and `(B²/24T) tan² φ` up in the upright frame.
Resolving along the heeled water surface gives

- `GZ = sin φ (GM + ½ BM tan² φ)`, with `GM = KB + BM − KG`, `KB = T/2`, `BM = B²/(12T)`;
- an off-centre centre of gravity at `TCG` takes `TCG cos φ` off the lever (the heeling arm);
- equilibrium, `GZ = TCG cos φ`, divided by `cos φ`: `(BM/2) tan³ φ + GM tan φ − TCG = 0`, a cubic in `tan φ` with one
  real root when `GM > 0` (Cardano);
- with `TCG = 0` and `GM < 0`, the loll angle `tan φ = √(−2 GM / BM)`, to either side.

The formula holds while the sides stay wall-sided at the waterline: the deck edge dry (`|tan φ| < 2(D − T)/B`) and the
bilge wet (`|tan φ| < 2T/B`). The off-centre weight test asserts both limits, the 22.7° loll test asserts the bilge limit,
and the 1.2° and 0.4° small-loll tests assert the resulting angles directly.

### What is compared
The 10 × 4 × 3 m creased box of §1 in fresh water (ρ 1000 kg/m³, appendage coefficient 1, subdivision level 2),
floating at 1.0 m (Δ 40 t), G at mid-length on the centreline. The equilibrium from `heel::equilibrium` (always
free to trim, large-angle, refined by bisection on the lever) is compared with the formula.

| Case | Inputs | Formula | Tolerance |
|---|---|---|---|
| Off-centre weight | KG 1.6 m, + 2 t at (5, 1.0, 1.6): Δ 42 t, T 1.05 m, GM 0.195 m, TCG 0.0476 m | heel 12.021° | 0.05°; trim within 1e-4 m of 0 |
| Mirror | the same weight at y = −1.0 | −(the starboard heel) | 1e-3° |
| Centreline weight at LCB | + 2 t at (5, 0, 1.6) | heel 0, draft 1.05 m | 1e-6° and 1e-6 m |
| Loll | KG 1.95 m, no weight: GM −0.117 m | ±22.7° (`tan φ` 0.418) | 0.05°; port = −starboard to 1e-3° |
| Capsize | + 30 t at (5, 1.5, 6.0) | no stable root within ±90° | outcome `Capsizes` |
| Cannot float | + 200 t (the box holds 120 t) | no floating state | outcome `CannotFloat` |

The small-angle answer `tan φ = TCG / GM` would give 13.7° for the first case: the test tells the two apart.

### Test locations
- `crates/seacraft-core/tests/analytic.rs`: `a_weight_off_centre_heels_a_box_to_the_wall_sided_angle`,
  `a_starboard_weight_heels_to_starboard`, `a_weight_at_the_centreline_and_lcb_changes_only_the_draft`,
  `negative_gm_lolls_to_both_sides`, `a_small_loll_is_found_on_both_sides` (KG solved from the loll formula for
  1.2°, inside the first sweep step), `a_very_small_loll_below_half_a_degree_is_found_on_both_sides` (0.4°, below
  0.5°), `a_high_heavy_weight_capsizes_the_box`, `heavier_than_the_hull_cannot_float`.
- `crates/seacraft-core/src/heel.rs`: `centreline_lever_is_the_cross_curve_gz` (same code path: with G on the
  centreline the lever equals the cross curves' GZ to 1e-12 on the demo hull, the 10 m yacht and the 6.5 m runabout),
  `off_centre_g_splits_into_gz_and_heeling_arm` (definition), `weights_combine_by_mass` (arithmetic),
  `bare_condition_uses_the_dock_draft` (the dock's displacement to 1e-9 relative).

### Provenance
Derived for this test (2026-11-20); the cubic is solved in the test itself, so the reference is the formula, not a
stored number. Bite check, 2026-11-20: the heeling arm scaled by 1.01 moves the first case to 12.113° and fails it.

What this covers: the equilibrium search and the lever split on a wall-sided box inside the formula's range. It does
not show that the heel of a real hull under a real weight is right; past the deck-edge or bilge limits only the
same-code-path checks against the GZ curves apply.

---

## 48. The Plywood Skiff Example: Every Panel Lies Flat

### Formula
A ruled surface between a chine $c(u)$ and a sheer $s(u)$, with rulings $s(u) - c(u)$, is developable when its
tangent plane does not turn along a ruling, that is when $c'(u)$, $s'(u)$ and $s(u) - c(u)$ are coplanar:
$\det[c'(u),\ s'(u),\ s(u) - c(u)] = 0$ (worked by hand, 2026-11-20: the surface normal along the ruling
$c + v(s - c)$ is $(c' + v(s' - c')) \times (s - c)$, whose direction does not depend on $v$ exactly when the
determinant vanishes). `crates/seacraft-core/src/examples/skiff.rs` solves it for the sheer point of every chine station
(`sheer_partner`); where no root exists, at the transom and at the stem, the rungs fan from one corner, a cone.
The bottom is flat across (both rows of a station at one height), a cylinder, and straight over its last 0.6 m
so that the narrow forefoot stays flat across through refinement.

### Tests
`crates/seacraft-core/src/examples/skiff.rs::skiff_panels_all_lie_flat`: at levels 3 and 4, Loop, the panels are
Bottom (one copy, joined across the centreline), Side (a mirrored pair) and Transom (one copy), none refused, and
every one lies flat within the default limits of §32 (2 mm seam drift, 0.1 % RMS stretch). Measured at authoring
(2026-11-20), level 3: Bottom RMS 0.0002 %, drift 0.01 mm; Side RMS 0.0018 %, drift 0.23 mm; Transom 0. The test
holds the limits, not these numbers.

### Provenance
The limits are §32's project judgement, not a published rule. The skiff's lines and design targets are
SeaCraft's own, representative of home-built skiffs of this size; Δ 0.30 t is the design target and the other
coefficients in `skiff_meets_the_board_targets` were measured on the net and frozen. Lying flat is measured
against the modelled surface with no plywood thickness: not a bending test of real plywood.

---

## 49. Measurements on Panel Patterns

### Formula
None new. The numbers written on each pattern are ones already measured: the overall length and width are the
pattern's extent (its lowest corner is at the origin, `panels::orient`); the offsets are the rows of the panel's
offsets table (`panels::panel_offsets`, every "Offsets every" from the baseline along the pattern's lowest edge),
one label per crossing; each seam's length is its flat length (`SeamMeasure::length_flat_m`), the same number
the seam drift is measured from.

### Tests
`crates/seacraft-core/src/panel_dims.rs`: `the_overall_size_is_the_extent` (the flat pram's Side, and a hand-built
3 m "C" shape, whose size is 3000 by 3000 mm by construction), `offsets_are_written_at_every_crossing` (one label
per ordinate of every row on the pram), `a_notched_outline_writes_every_ordinate` (the "C" is crossed four times at
x = 2 m, at 0, 1000, 2000 and 3000 mm by construction), `seam_lengths_are_labelled` (every seam of the skiff's
Side), `measurements_are_never_cut_lines` (every measurement line is a reference line).

### Provenance
By construction for the "C" shape; otherwise the measurements of §32 and §48, written on the drawing in whole
millimetres. Nothing here is a new measurement.

---

## 50. Panel Nesting: Outline Packing, Blanks and Split Parts

### Formula
`nest::pack` places closed 2D outlines in decreasing area on rectangular bins of length $L$ and width $W$, keeping a
gap $g$ between outlines on the same bin ($\mathrm{dist}(A, B) \ge g$) and from every bin edge
($[g, L - g] \times [g, W - g]$). Each item may be turned end for end (180° in its plane) or turned over (mirrored
across its length axis, $y \mapsto -y$, when `may_turn_over`), never a quarter turn across the grain. Candidate placements step along $x$ by 10 mm (always including $x_{\max}$) and locate the lowest valid $y$ by 10 mm
steps (always including $y_{\max}$) followed by bisection to 1 mm; the search simplifies outlines by Douglas–Peucker
to at most 0.25 mm and widens the inter-item search gap by 0.5 mm (twice that tolerance) so the requested gap $g$
holds on the true outlines. Bin-edge clearance uses each pose's true bounding box. Negative, non-finite, or oversized
gaps ($2g \ge W$ or $2g \ge L$), as well as non-finite or non-positive bin sizes, are refused without placing any
piece.

`panel_layout::nest_panels` turns each panel into its cut copies (a mirrored pair's second copy is the outline
mirrored $y \mapsto -y$ before posing):
- In `NestMode::Blanks`, `new_bin(need)` opens a blank of the fewest whole stock sheets $k \le 100$ with
  $L + (k - 1)(L - s) - 2g \ge \text{need}$ for scarf $s$, placing joints at $L, 2L - s, \dots$ from the aft end.
- In `NestMode::Split`, a panel longer than $L - 2g$ is divided square to the baseline into the fewest equal parts
  fitting $L - 2g$. A joint at an $x$ where the outline is crossed more than twice moves aft, then forward, in 10 mm
  steps to the nearest $x$ crossed twice; parts are clipped against the vertical slabs by Sutherland–Hodgman, and
  each joint gets matching `Mark` ticks at a quarter and three quarters of its length, lettered consecutively across
  the panel set.

### Tests
- `crates/seacraft-core/src/nest.rs`:
  - `rectangles_fill_a_bin_without_overlap`: twelve 0.5 × 0.3 m rectangles in one 2.44 × 1.22 m bin, gap 0.01 m;
    every point inside $[g, L - g] \times [g, W - g]$ and every pair at least $g - 10^{-9}$ m apart.
  - `two_arches_share_a_width`: two 3 m × 0.5 m bounding-box parabolic arches (whose boxes need 1.03 m > 0.85 m with
    gaps) both pack into one 3.2 × 0.85 m bin, holding the gap to $10^{-9}$ m, by construction.
  - `poses_keep_the_shape_and_anchor_the_box`: all four pose flags place the bounding box's low corner at $(x, y)$
    and preserve area and edge lengths to $10^{-12}$; `turned_over` reverses winding.
  - `too_wide_is_unplaced_by_name`, `no_bin_long_enough_is_unplaced`, `a_bad_gap_places_nothing_and_says_why`,
    `packing_is_deterministic`, `the_gap_is_kept_from_bin_edges`, `tight_fit_within_one_step_of_bin_edges`,
    `non_finite_bin_and_zero_usable_width_are_unplaced`.
- `crates/seacraft-core/src/panel_layout.rs`:
  - `blanks_hold_the_skiff_without_overlap`: all 4 pieces of `skiff-4m` (`Bottom`, `Side`, `Side, turned over`,
    `Transom`) placed on at most 3 blanks of 3.0 × 1.5 m, gap 0.01 m held to $10^{-9}$ m.
  - `a_blank_is_as_long_as_its_sheets_less_the_scarfs`: 4.4 m piece on 3.0 m stock with 0.15 m scarf gives a 2-sheet
    blank of 5.85 m with its joint at 3.0 m, by construction.
  - `split_parts_add_up_and_fit`: `skiff-4m` `Side` on 2440 mm stock splits into `"Side 1 of 2"` and `"Side 2 of 2"`,
    part lengths sum to the panel's length within $10^{-9}$ m, and every part fits $L - 2g$.
  - `joint_marks_match`: split parts carry `Mark` ticks at matching joint positions and matching letters across the
    set (`A`/`B`, `C`/`D`).
  - `a_joint_moves_off_a_triple_crossing`: a 4 × 1 m slotted slab (crossed 4 times for $x > 1.5$ m) moves its joint
    below $x = 1.5$ m where crossed twice; part lengths sum to 4.0 m and part areas sum to $4.0 - 2.5 \times 0.2 =
    3.5$ m² within $10^{-9}$.
  - `the_turned_over_copy_is_the_mirror`, `layout_drawings_cut_only_the_pieces`,
    `tolerance_boundary_pieces_still_place`, `three_way_split_and_mirrored_label_orientation`.

### Provenance
Synthetic shapes by construction and geometric invariants (no published benchmark). `nest::pack` is a deterministic
largest-first heuristic, not minimum-sheet packing, and is not checked for plywood grain defects, kerf variation, or
real shop tolerances.

---

## 51. Hull Materials: Layer Areas and Shell Weights

### Formula
On the refined half mesh (`subdiv::evaluate`), a layer's area $A$ is the sum of its face areas (`polygon_area3`), each
counted twice when the layer is symmetric (the mirror lies at $-y$) unless every vertex of the face has $|y| < 10^{-9}$.
The layer centre is the area centroid of the faces and their mirrors; a face's centroid weighs each triangle of its fan
by its area signed along the face normal, so a triangle folded back over a concave face subtracts. Shell mass is
$m = A\,t\,\rho$ when $t > 0$ and $\rho > 0$, and 0 otherwise, with a `materials.warn.no_plating` warning; glass is
$A \sum g_i / 1000$ kg for plies of $g_i$ g/m²; resin is glass mass times `resin_per_glass`. The mass-weighted mean of
the layer centres is the centre of shell and laminate; the total centre also weighs in the seams' tape and fillets at
their own centres (§52).

### Tests
- `crates/seacraft-core/src/materials.rs`, `half_box_*`: a box open on $y = 0$ and $z = 1$, $x \in [0, 2]$, $y, z \in [0, 1]$,
  with every edge creased and every point a corner, so subdivision leaves it exact. Hand-derived values at level 3:
  - `Bottom` ($z = 0$): half area $2$ m², so $A = 4.0$ m²; centre $(1, 0, 0)$; 10 mm at 500 kg/m³ gives 20.0 kg.
  - `Side` ($y = 1$): half area $2$, $A = 4.0$ m²; centre $(1, 0, 0.5)$; 6 mm at 500 gives 12.0 kg.
  - `End` ($x = 0$ and $x = 2$, 1 m² each): half area $2$, $A = 4.0$ m²; centre $(1, 0, 0.5)$; 12 mm at 500 gives 24.0 kg.
  - Total 56.0 kg (0.056 t), centre $z = (20 \cdot 0 + 12 \cdot 0.5 + 24 \cdot 0.5)/56 = 18/56$ m. Tolerance $10^{-9}$.
  - `plies_add_glass_and_resin`: `Bottom` plies 200 + 200 g/m² give $4 \cdot 0.4 = 1.6$ kg glass and, at 1.2 resin per glass,
    $1.6 \cdot 1.2 = 1.92$ kg resin. The glass and resin figures are the test's own inputs, not published values.
- `crates/seacraft-core/src/materials.rs`, the other layer tests, hand-derived:
  - `a_face_on_the_centreline_counts_once`: a 1 m × 1 m face in the plane $y = 0$ on a symmetric layer has every vertex
    on the centreline, so it is not mirrored: $A = 1.0$ m², not 2.0.
  - `zero_plating_warns_and_weighs_nothing`: the box with `Bottom` at density 0 gives `Bottom` a shell of exactly 0 kg
    ($A\,t \cdot 0$) and one `materials.warn.no_plating` warning.
  - `negative_plating_warns_and_weighs_nothing`: `Bottom` at density $-500$ or thickness $-0.01$ m weighs 0 kg with one
    `materials.warn.no_plating`; the total is `Side` 12.0 + `End` 24.0 = 36.0 kg at $(1, 0, 0.5)$.
  - `a_concave_face_is_centred_by_signed_area`: the square $[0, 2]^2$ less the notch $(2,2), (1,0.5), (0,2)$ has
    area $4 - 1.5 = 2.5$ m² and centroid $y = (4 \cdot 1 - 1.5 \cdot 1.5)/2.5 = 0.7$, $x = 1$; the unit square's
    centroid is $(0.5, 0.5)$.
  - `a_layer_that_is_not_symmetric_counts_once_off_the_centreline`: the box with `Side` not symmetric has no mirror, so
    $A = 2.0$ m², centre $(1, 1, 0.5)$ (the face itself, not the mean of it and its mirror), shell
    $2 \cdot 0.006 \cdot 500 = 6.0$ kg.
  - `a_layer_with_no_faces_has_no_area`: a fourth layer with no faces has $A = 0$ and 0 kg; the total stays 56.0 kg
    at the same centre.
- `crates/seacraft-core/src/examples/mod.rs`, `materials_shell_matches_the_render_mesh_sum`: every example's per-layer shell
  mass equals the sum over `export::render_mesh` (the full mirrored mesh), within $10^{-9}$ relative. A consistency check
  between two ways of summing the same mesh, not an oracle.

### Provenance
Hand-derived for the box (areas, centres, masses). Nothing here is published; the figures are the builder's numbers applied
to the modelled surface, with no laminate thickness offset.

## 52. Hull Materials: Seams, Tape and Fillets

### Formula
Seams are found on the refined half mesh (`subdiv::evaluate`). An edge with two faces is a seam candidate when it is
creased or its faces are on different layers; a one-face edge with both ends on $y = 0$ on a symmetric layer is folded
against its own mirror, whose outward normal is $(n_x, -n_y, n_z)$. That edge needs no crease: the refinement treats
every boundary edge as hard, so a crease would decide nothing at level 1 and above, and the fold angle alone decides at
every level. A face whose every vertex is on $y = 0$ is its own mirror; its free edges are plate edges, not seams.
An edge with three or more faces, no length, or a face of no area is not measured and raises one
`materials.warn.seam_degenerate` per layer pair. With unit outward normals $n_1, n_2$, $e$
along the edge as the first face winds it, and $\varphi = \angle(n_1, n_2)$, the edge is convex when
$(n_1 \times n_2) \cdot e > 0$, and the inside angle is $\theta = \pi - \varphi$ (convex) or $\pi + \varphi$ (concave).
$|\theta - \pi| \le 5^\circ$ is flat: no seam. A two-face edge between symmetric layers counts twice (port and
starboard), even with both ends on $y = 0$: a bottom meeting a web in the centreline plane meets each face of the web,
two joints. A one-face edge folded against its own mirror is one joint and counts once.

Tape per metre of seam is $w \cdot n \cdot g \cdot s / 10^6$ kg for width $w$ mm, $n$ plies of $g$ g/m², on $s$
faces (1 or 2); its resin is that times `resin_per_glass`. The fillet, only on a convex edge with $\theta > 5^\circ$, has
the cross-section of a cove of radius $r$ in a corner of inside angle $\theta$:

$$A(\theta) = r^2\left(\cot\frac{\theta}{2} - \frac{\pi - \theta}{2}\right).$$

Derivation: the circle of radius $r$ tangent to both faces has its centre on the bisector, at $r / \sin(\theta/2)$
from the corner; each tangent point is $r \cot(\theta/2)$ from the corner. The kite corner–tangent–centre–tangent is
two right triangles of legs $r$ and $r\cot(\theta/2)$, area $r^2 \cot(\theta/2)$. Its angle at the circle's centre is
$\pi - \theta$, so the sector of the circle inside the kite is $r^2 (\pi - \theta)/2$. The cove is the kite less that
sector. Closed forms: $A(\pi/2) = r^2(1 - \pi/4)$ (kite $r^2$, quarter circle $\pi r^2/4$), and
$A(2\pi/3) = r^2(1/\sqrt{3} - \pi/6)$ ($\cot(\pi/3) = 1/\sqrt{3}$, sector angle $\pi/3$). Fillet mass is
$\sum$ edge length $\times A(\theta) \times$ mix density.

### Tests
All in `crates/seacraft-core/src/materials.rs`, at level 3, tolerance $10^{-9}$ unless stated; the tape, resin and
fillet figures are the tests' own inputs, not published values.
- `fillet_area_closed_forms`: the two closed forms above at $r = 1$ (tolerance $10^{-12}$), and $A(\pi/2)$ at
  $r = 0.012$.
- `half_box_seams`, the box of §51: three groups, each 4.0 m and convex with $\theta = \pi/2$ on every edge.
  `Bottom / End`: $x = 0$ and $x = 2$, $y \in [0, 1]$, $z = 0$, so $2 \cdot 1$ m on the half, $\times 2$ for the mirror;
  centre $(1, 0, 0)$. `Bottom / Side`: $y = 1$, $z = 0$, $x \in [0, 2]$, 2 m $\times 2$; centre $(1, 0, 0)$.
  `End / Side`: $x = 0$ and $2$, $y = 1$, $z \in [0, 1]$, 2 m $\times 2$; centre $(1, 0, 0.5)$. No `Bottom / Bottom`
  and no `End / End`: the bottom and the ends meet their mirrors flat across $y = 0$ ($\varphi = 0$).
  `flat_bottom_across_centreline_has_no_keel_seam` checks the same with a schedule.
- `half_box_tape_and_fillet`: tape 75 mm, 1 ply of 200 g/m², both faces, resin 1.0 per glass; fillet $r = 12$ mm at
  1000 kg/m³. Per group: tape $4.0 \cdot 0.075 \cdot 0.2 \cdot 2 = 0.12$ kg, resin 0.12 kg, fillet
  $4.0 \cdot 0.012^2 \cdot (1 - \pi/4) \cdot 1000$ kg. The total adds the three groups to the 56.0 kg shell; its centre
  $z$ is $(18 + 0.5\,m_g)/(56 + 3\,m_g)$ for a group mass $m_g$ (two groups at $z = 0$, one at $z = 0.5$).
- `v_bottom_keel_is_one_seam_counted_once`: half a V bottom from $(x, 0, 0)$ to $(x, 1, 0.5)$, $x \in [0, 2]$, creased
  on the centreline. Outward normal $(0, 1, -2)/\sqrt{5}$, mirror $(0, -1, -2)/\sqrt{5}$, $\cos\varphi = 3/5$, so
  $\varphi = 2\arctan(0.5)$ and $\theta = \pi - 2\arctan(0.5)$ (each side rises at $\arctan 0.5$). One
  `Bottom / Bottom` group of 2.0 m, counted once, centre $(1, 0, 0)$; fillet $2.0 \cdot A(\theta) \cdot 1000$ at
  $r = 12$ mm.
- `a_web_on_the_centreline_has_a_seam_each_side`: a flat bottom in $z = 0$, $y \in [0, 1]$, outward $-z$, meets a web
  in $y = 0$, $z \in [0, 0.5]$, normal $-y$, along a creased 2 m edge on the centreline. Both layers are symmetric, so
  the port bottom meets the web's port face: one `Bottom / Web` group of $2 \cdot 2.0 = 4.0$ m at $(1, 0, 0)$, convex at
  $\theta = \pi/2$, tape $4.0 \cdot 0.075 \cdot 0.2 \cdot 2$ kg and fillet $4.0 \cdot A(\pi/2) \cdot 1000$ at
  $r = 12$ mm.
- `colour_only_boundary_is_not_a_seam`: a flat, uncreased 2 m × 1 m plate split into two layers at $x = 1$: the
  boundary is flat ($\varphi = 0$), so no seam.
- `concave_crease_gets_tape_but_no_fillet`: face A in $z = 0$, $y \in [0, 1]$, outward $-z$; face B in $y = 1$,
  $z \in [-1, 0]$, outward $-y$; creased shared edge along $x$. $(n_1 \times n_2) \cdot e = -1$, so concave,
  $\theta = \pi + \pi/2 = 3\pi/2$: tape $2.0 \cdot 0.075 \cdot 0.2 \cdot 2$ kg, no fillet.
- `a_closed_seam_warns_and_has_no_fillet`: B folded back over A to $(y = 0, z = 0.05)$: convex, $\theta = \arctan 0.05$
  (under $5^\circ$), so a `materials.warn.seam_closed` warning, tape, and no fillet.
- `two_faces_winding_an_edge_the_same_way_warn_and_are_not_a_seam`: the concave pair with B's winding reversed: one
  `materials.warn.seam_winding` warning and no seam.
- `an_uncreased_v_keel_is_still_a_seam_at_every_level`: the V bottom above without the centreline crease gives the same
  `Bottom / Bottom` group of 2.0 m at $\theta = \pi - 2\arctan(0.5)$ at level 0 and at level 3.
- `a_plate_in_the_centreline_plane_has_no_seams`: the 1 m × 1 m plate in $y = 0$ of §51 with tape and fillet: its mirror
  normal is $-n$ ($\varphi = \pi$), but it lies in the plane, so no seam and no warning.
- `an_edge_of_three_faces_warns_once_and_is_not_a_seam`: three faces of one layer on one creased edge: no seam, exactly
  one `materials.warn.seam_degenerate` warning for `Hull / Hull`.
- `a_seam_on_a_face_of_no_area_warns_once`: the fold with B collinear with the shared edge (zero normal): no seam, one
  `materials.warn.seam_degenerate` warning.
- `a_seam_with_a_layer_that_is_not_symmetric_counts_once`: the box with `Side` not symmetric: `Bottom / Side` 2.0 m at
  $(1, 1, 0)$ and `End / Side` 2.0 m (no mirror), `Bottom / End` still 4.0 m.
- `crates/seacraft-core/src/examples/mod.rs`, `materials_shell_matches_the_render_mesh_sum` also asserts that no
  example raises `materials.warn.seam_winding` or `materials.warn.seam_degenerate` (its half mesh winds consistently
  and has no degenerate or non-manifold seam edge). A consistency check, not an oracle.

### Provenance
Hand-derived: the fillet cross-section (plane geometry above), the box seams and the V bottom's angle. Nothing here is
published; the figures are the builder's numbers applied to the modelled surface, not validated (`DISCLAIMER.md`), with
no laminate thickness offset.

---

## 52. Floodable Length: Box Barge Closed Form and Lost-Buoyancy Balance

### Formula
The floodable length at a point is the length of a compartment, with that point as centre, that can be flooded
without immersing any part of the margin line when the ship has no list; the margin line is at least 76 mm (3 in)
below the bulkhead deck at side. Each trial waterline $W$ is tangent to the margin line; with $V_W$ and $x_B$ the
volume and centroid under it, $\nabla$ the floating volume and $\mu$ the permeability, the lost volume and its
centroid are
$$v = V_W - \nabla, \qquad x_c = \frac{V_W x_B - \nabla \cdot LCG}{v},$$
and the compartment $[a, b]$ holds $v / \mu$ under $W$ with its centroid at $x_c$.

For a box barge $L \times B \times D$ floating at $T$ with $LCG = L/2$ and margin line height $z_m = D - m$, the
even-keel point is amidships with
$$l = \frac{L (z_m - T)}{\mu \, z_m}.$$
At a trimmed waterline the height above the keel is linear, $h(x) = c - s x$ (clamped to $[0, D]$), and the balance
$\mu B \int_a^b h\,dx = B \int_0^L h\,dx - \nabla$, $\mu B \int_a^b x h\,dx = B \int_0^L x h\,dx - \nabla \cdot L/2$
is evaluated with exact piecewise integrals.

### Tests
`crates/seacraft-core/src/floodable.rs`:
- `box_barge_even_keel_point_matches_closed_form`: $L = 10$, $B = 4$, $D = 2$ m, $T = 0.80$ m, $\rho = 1$ t/m³,
  $m = 0.076$ m ($z_m = 1.924$ m), $\mu \in \{0.60, 0.85, 0.95\}$: $x_{mid} = 5.0$ m within $10^{-3}$ m and
  $l = 5.842 / \mu$ m within $10^{-3}$ relative (9.737, 6.873, 6.149 m).
- `box_barge_trimmed_points_satisfy_the_analytic_balance`: every point of the three curves satisfies both balance
  equations within $2 \times 10^{-3}$ relative.
- `a_dried_end_keeps_the_solve_finite`: the same at $T = 0.20$ m, where some trial waterlines lift one end of the
  bottom out of the water.
- `tangent_waterline_touches_the_margin_line_and_stays_below_it`, `curve_covers_the_hull_and_stops_inside_its_ends`,
  `bulkheads_split_the_hull_and_are_judged_at_their_midpoints` (factor 0.5: amidships 6.87 m floodable, 3.44 m
  permissible, a 2 m compartment passes), `a_compartment_outside_the_curve_is_not_judged`,
  `hull_ends_ignore_vertices_outside_the_panels`, and the error tests (`unphysical_particulars_are_errors` among them).
- `a_curve_does_not_depend_on_the_other_permeabilities` (demo hull, level 2, 8 t): the $\mu = 0.60$ curve asked
  alone agrees with the same curve asked with 0.85 and 0.95 within 1 % at mid-range. SeaCraft against itself, not a
  source.

### Provenance
Both books give the definitions: the margin line 76 mm (3 in) below the bulkhead deck at side, the floodable length
centred at the point, and permissible length = floodable length × factor of subdivision (Rawson & Tupper, *Basic
Ship Theory*, Vol. 1, 5th ed., ch. 5, pp. 155-156; Gillmer & Johnson, *Introduction to Naval Architecture* (1982),
§10-1, pp. 180-182). Rawson & Tupper alone give the triangle check of base angle $\tan^{-1} 2$ and the forward and
after terminals at the ends. Gillmer & Johnson alone give the method: trial waterlines tangent to the margin line,
sectional areas to each waterline from Bonjean curves, the flooding water's position from a moment balance and the
compartment volume from it (Eq. 10-1 and 10-2). Neither book prints worked numbers. The box barge values are derived here from the lost-buoyancy balance, not published.
SeaCraft measures the margin line from the moulded deck edge (no deck thickness).

---

## 53. Floodable Length: Cross-Check with the Damage Solver

### Formula
A compartment from a floodable length curve, flooded as an ordinary damage case (lost buoyancy, compartment clipped
to the hull, `damage::evaluate`), must float the hull upright with the margin line awash: minimum freeboard to the
deck edge equal to the margin depth $m$, at the curve point's trim.

### Tests
`crates/seacraft-core/src/floodable.rs::curve_points_flood_the_demo_hull_to_its_margin_line`: demo hull, level 3,
design-draft loading, $\mu = 0.85$, the first, middle and last points of the curve. Asserted: heel below 0.05°,
freeboard within 5 mm of 0.076 m, trim within 20 mm of the point's. Flooding the same compartments at $\mu = 0.95$
instead fails the test (freeboard 0.016 m at the aft point).

### Provenance
Agreement between two SeaCraft methods (sectional areas under trial waterlines against a clipped solid in the
damage equilibrium solver), not an external source.

---

## 54. Floodable Length: Demo Hull, Recorded

### Tests
`crates/seacraft-core/src/floodable.rs::demo_hull_curve_recorded`: demo hull, level 3, design draft, $\mu = 0.85$:
$\Delta = 12.1$ t, $LCG = 4.30$ m, least floodable length 1.39 m at $x = 0.701$ m, 3.37 m at $x = 6.0$ m, each
within 0.5 %.

### Provenance
Not published: recorded from SeaCraft on 2026-11-26 to catch a change. The curve's shape (least near the aft end,
highest amidships, a second low near the forward quarter) agrees with Gillmer & Johnson's remark (glossary, p. 304)
that the floodable length "is usually greatest amidships and least near the quarter length".
