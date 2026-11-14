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
- Grid result matches 1D analytical quadrature `wigley_analytic_rw` within $1\%$.

### Provenance
Independent numerical quadrature using variable substitution $\lambda = \sec\theta = \cosh u$ ($u \in [0, 6]$) over an $801 \times 201$ offset grid using the trapezoidal rule, computed specifically for this regression check. As documented in the module comments, these are not published experimental or tabulated values.

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
"Hydrodynamic design of planing hulls", Marine Technology 1(1), 1964.

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
