//! Analytic and regression checks against the panel hydrostatics.
//!
//! A closed rectangular barge has exact hydrostatics: `∇ = L·B·T`,
//! `Cb = Cwp = Cm = 1`, `KB = T/2`, `BM = B²/(12T)` (transverse) and
//! `L²/(12T)` (longitudinal), and no righting lever at zero heel. If the
//! clipping, the divergence integral or the waterline chaining drifts, these
//! numbers move and nothing else in the suite necessarily notices.

use seacraft_core::hydro::{self, Hull, HydroOptions};
use seacraft_core::math::Vec3;
use seacraft_core::stability;
use seacraft_core::subdiv::SubdivisionOptions;
use seacraft_core::{Layer, ProjectSettings, Surface};

/// A closed box: `L` along x, `B` along y, standing on z = 0, with every edge
/// creased so refinement cannot round the corners away. The box is taller than
/// the draft in every test: floating it exactly at its own depth would put the
/// waterplane in the top face, which is a degenerate cut and says nothing.
fn barge(length: f64, beam: f64, depth: f64) -> Surface {
    let mut s = Surface::new();
    s.layers = vec![Layer {
        name: "Hull".into(),
        symmetric: false,
        use_in_hydrostatics: true,
        visible: true,
        ..Layer::default()
    }];
    s.settings = ProjectSettings {
        name: "barge".into(),
        length,
        beam,
        draft: depth,
        water_density: 1000.0,
        appendage_coefficient: 1.0,
        main_particulars_set: true,
        ..ProjectSettings::default()
    };
    let (l, b) = (length, beam);
    // 0..3 bottom, 4..7 top, matching the cube test in subdiv.rs.
    let corners = [
        (0.0, 0.0, 0.0),
        (l, 0.0, 0.0),
        (l, b, 0.0),
        (0.0, b, 0.0),
        (0.0, 0.0, depth),
        (l, 0.0, depth),
        (l, b, depth),
        (0.0, b, depth),
    ];
    for (x, y, z) in corners {
        // Centred on the centreline: a translation keeps the winding, unlike
        // a mirror, so the normals still point out.
        s.add_point(Vec3::new(x, y - b * 0.5, z));
    }
    // Outward normals: bottom down, top up, sides out.
    for face in [
        vec![0, 3, 2, 1],
        vec![4, 5, 6, 7],
        vec![0, 1, 5, 4],
        vec![1, 2, 6, 5],
        vec![2, 3, 7, 6],
        vec![3, 0, 4, 7],
    ] {
        s.add_face(face, 0);
    }
    s.initialize_trace();
    // Everything creased and every point a corner: the box stays a box.
    for edge in &mut s.edges {
        edge.crease = true;
    }
    for point in &mut s.points {
        point.kind = seacraft_core::mesh::VertexKind::Corner;
    }
    s
}

fn options(surface: &Surface, draft: f64) -> HydroOptions {
    HydroOptions {
        draft,
        trim: 0.0,
        heel_deg: 0.0,
        water_density: 1000.0,
        appendage_coefficient: 1.0,
        mainframe_x: Some(surface.settings.length * 0.5),
        stations: 5,
        subdivision: SubdivisionOptions {
            level: 2,
            mode: Default::default(),
        },
    }
}

#[test]
fn a_closed_barge_has_exact_hydrostatics() {
    let (l, b, depth, t) = (10.0, 4.0, 4.0, 2.0);
    let surface = barge(l, b, depth);
    let opts = options(&surface, t);
    let h = hydro::compute(&surface, &opts);

    let volume = l * b * t;
    assert!(
        (h.volume_m3 - volume).abs() < 1e-6 * volume,
        "volume {} vs {volume}",
        h.volume_m3
    );
    assert!(
        (h.displacement_t - volume).abs() < 1e-6 * volume,
        "rho 1000: {} t",
        h.displacement_t
    );

    // Coefficients are exactly one for a box at its own depth.
    assert!(
        (h.block_coefficient - 1.0).abs() < 1e-9,
        "Cb {}",
        h.block_coefficient
    );
    assert!(
        (h.waterplane_coefficient - 1.0).abs() < 1e-9,
        "Cwp {}",
        h.waterplane_coefficient
    );
    assert!(
        (h.midship_coefficient - 1.0).abs() < 1e-9,
        "Cm {}",
        h.midship_coefficient
    );
    assert!(
        (h.prismatic_coefficient - 1.0).abs() < 1e-9,
        "Cp {}",
        h.prismatic_coefficient
    );

    // Buoyancy at half depth, metacentres from the waterplane second moments.
    assert!((h.kb - t / 2.0).abs() < 1e-9, "KB {}", h.kb);
    assert!(
        (h.waterplane_area_m2 - l * b).abs() < 1e-9,
        "Awp {}",
        h.waterplane_area_m2
    );
    assert!(
        (h.bm_transverse - b * b / (12.0 * t)).abs() < 1e-9,
        "BMt {}",
        h.bm_transverse
    );
    assert!(
        (h.bm_longitudinal - l * l / (12.0 * t)).abs() < 1e-6,
        "BMl {}",
        h.bm_longitudinal
    );
    assert!(
        (h.km_transverse - (t / 2.0 + b * b / (12.0 * t))).abs() < 1e-9,
        "KM {}",
        h.km_transverse
    );

    // Centre of buoyancy and flotation sit amidships, on the centreline.
    assert!((h.lcb.x - l / 2.0).abs() < 1e-9, "LCB x {}", h.lcb.x);
    assert!(h.lcb.y.abs() < 1e-9, "LCB y {}", h.lcb.y);
    assert!((h.lcf.x - l / 2.0).abs() < 1e-9, "LCF x {}", h.lcf.x);

    // Half the box is wetted: bottom + four sides up to the waterline.
    let wetted = l * b + 2.0 * l * t + 2.0 * b * t;
    assert!(
        (h.wetted_surface_m2 - wetted).abs() < 1e-6 * wetted,
        "wetted {}",
        h.wetted_surface_m2
    );
    assert!(
        (h.mainframe_area_m2 - b * t).abs() < 1e-9,
        "midship {}",
        h.mainframe_area_m2
    );
}

#[test]
fn a_box_has_no_righting_lever_upright() {
    let (l, b, depth, t) = (10.0, 4.0, 4.0, 2.0);
    let surface = barge(l, b, depth);
    let base = options(&surface, t);
    let hull = Hull::new(&surface, base.subdivision);
    // Float the barge at its own displacement.
    let target = l * b * t;
    let draft = stability::solve_draft(&hull, target, 0.0, 0.0).expect("the barge floats");
    assert!((draft - t).abs() < 1e-6, "solved draft {draft}");

    let curves = stability::cross_curves(
        &hull,
        &[0.0, 10.0],
        &[target],
        &stability::StabilityCondition::fixed(1.0, 0.0),
        &base,
    );
    let upright = &curves[0].points[0];
    let gz0 = upright.gz.expect("gz upright");
    let kn0 = upright.kn.expect("kn upright");
    assert!(gz0.abs() < 1e-9, "GZ at zero heel {gz0}");
    assert!(kn0.abs() < 1e-9, "KN at zero heel {kn0}");
    // Heeling a box puts the centre of buoyancy to leeward: positive GZ.
    let heeled = &curves[1].points[0];
    let gz10 = heeled.gz.expect("gz 10 deg");
    assert!(gz10 > 0.0, "GZ at 10 degrees {gz10}");
}

/// The numbers `AGENTS.md` quotes for the demo hull. They are the project's
/// regression oracle: a change that moves them by more than ~1 % is a bug.
#[test]
fn demo_hull_matches_the_reference_numbers() {
    let surface = seacraft_core::demo::demo_hull(false);
    let opts = HydroOptions {
        draft: 0.62,
        trim: 0.0,
        heel_deg: 0.0,
        water_density: 1025.0,
        appendage_coefficient: 1.0,
        mainframe_x: None,
        stations: 11,
        subdivision: SubdivisionOptions {
            level: 3,
            mode: Default::default(),
        },
    };
    let h = hydro::compute(&surface, &opts);
    let check = |label: &str, got: f64, want: f64, tol: f64| {
        assert!(
            (got - want).abs() <= tol,
            "{label}: {got} differs from the reference {want} by more than {tol}"
        );
    };
    check("volume", h.volume_m3, 11.85, 0.12);
    check("Cb", h.block_coefficient, 0.504, 0.005);
    check("Cwp", h.waterplane_coefficient, 0.824, 0.008);
    check("KB", h.kb, 0.392, 0.004);
    check("KM", h.km_transverse, 2.63, 0.03);
    // The reference was measured at the centre of flotation, which is what a
    // `.fef` without a main-frame location falls back to.
    check("midship", h.mainframe_area_m2, 1.47, 0.02);
}

/// Free trim against an exact answer. A rectangular box of length `L`, beam
/// `B` and draft `d` that floats with the bow up by `trim` submerges
/// `h(x) = d + trim (1/2 - x/L)`, so the displacement stays `L·B·d` for every
/// trim while the centre of buoyancy moves aft:
///
///     x̄ = L/2 - trim · L / (12 d)
///
/// Asking for `x̄ = 4 m` on a 10 m box at 2 m draft therefore has to trim
/// `2.4 m` bow-up. Nothing else in the solver has a closed form to check.
#[test]
fn free_trim_moves_the_lcb_to_the_requested_station() {
    let (l, b, depth, t) = (10.0, 4.0, 4.0, 2.0);
    let surface = barge(l, b, depth);
    let base = options(&surface, t);
    let hull = Hull::new(&surface, base.subdivision);
    let target = l * b * t;
    let lcg = 4.0;

    let fc = stability::solve_floating(&hull, target, 0.0, lcg, 0.0).expect("the box floats");

    let want_trim = (l * 0.5 - lcg) * 12.0 * t / l;
    assert!(
        (fc.trim - want_trim).abs() < 0.02 * want_trim,
        "trim {} vs the analytic {want_trim}",
        fc.trim
    );
    assert!((fc.draft - t).abs() < 1e-3, "draft {}", fc.draft);

    // The solve really is in equilibrium, not just near the analytic answer.
    let (v, cb) = hull.buoyancy(fc.draft, fc.trim, 0.0);
    assert!((v - target).abs() < 1e-6 * target, "volume {v} vs {target}");
    assert!(
        (cb.x - lcg).abs() < 1e-4,
        "LCB {} did not reach the LCG {lcg}",
        cb.x
    );

    // A box is symmetric fore and aft, so heeling must not make it trim.
    let fc = stability::solve_floating(&hull, target, 20.0, l / 2.0, 0.0).expect("heeled box");
    assert!(
        fc.trim.abs() < 1e-3,
        "a symmetric box should not trim: {}",
        fc.trim
    );
}

/// Free-to-trim cross curves must stay in longitudinal equilibrium at every
/// heel, and the demo hull — wide transom, fine bow — really does trim: a heel
/// is worth several centimetres of trim, and letting it happen costs some
/// righting lever relative to holding the trim fixed.
#[test]
fn free_trim_cross_curves_hold_the_lcb_on_the_lcg() {
    let surface = seacraft_core::demo::demo_hull(false);
    let base = HydroOptions {
        draft: 0.62,
        trim: 0.0,
        heel_deg: 0.0,
        water_density: 1025.0,
        appendage_coefficient: 1.0,
        mainframe_x: None,
        stations: 0,
        subdivision: SubdivisionOptions {
            level: 3,
            mode: Default::default(),
        },
    };
    let hull = Hull::new(&surface, base.subdivision);
    let target = hull.buoyancy(base.draft, 0.0, 0.0).0;
    let lcg = stability::upright_lcb(&hull, base.draft);
    let heels = [0.0, 10.0, 20.0, 30.0];

    let free = stability::cross_curves(
        &hull,
        &heels,
        &[target * base.water_density / 1000.0],
        &stability::StabilityCondition::free_to_trim(2.0, lcg),
        &base,
    );
    let fixed = stability::cross_curves(
        &hull,
        &heels,
        &[target * base.water_density / 1000.0],
        &stability::StabilityCondition::fixed(2.0, 0.0),
        &base,
    );

    for (i, curve) in free.iter().enumerate() {
        let p = &curve.points[0];
        let gz = p.gz.expect("gz must solve");
        let draft = p.draft.expect("draft must solve");
        let trim_m = p.trim_m.expect("trim must solve");
        assert!(gz.is_finite(), "heel {} did not solve", curve.heel_deg);
        let (v, cb) = hull.buoyancy(draft, trim_m, curve.heel_deg);
        assert!(
            (v - target).abs() < 1e-4 * target,
            "heel {}: volume {v} vs {target}",
            curve.heel_deg
        );
        assert!(
            (cb.x - lcg).abs() < 2e-3,
            "heel {}: LCB {} vs LCG {lcg}",
            curve.heel_deg,
            cb.x
        );
        if curve.heel_deg >= 20.0 {
            assert!(
                trim_m.abs() > 0.005,
                "heel {}: a hull with a wide transom must trim, got {}",
                curve.heel_deg,
                trim_m
            );
            // Holding the trim fixed leaves the hull out of equilibrium and
            // overstates the righting lever.
            let fixed_gz = fixed[i].points[0].gz.expect("fixed gz");
            assert!(
                gz < fixed_gz,
                "heel {}: free-trim GZ {} should not exceed the fixed-trim {fixed_gz}",
                curve.heel_deg,
                gz
            );
        }
    }
}

#[test]
fn closed_box_cross_curve_at_75_and_90_degrees_is_finite() {
    let surface = barge(10.0, 4.0, 4.0);
    let base = options(&surface, 2.0);
    let hull = Hull::new(&surface, base.subdivision);
    let target = hull.buoyancy(base.draft, 0.0, 0.0).0;
    let heels = [75.0, 90.0];
    let curves = stability::cross_curves(
        &hull,
        &heels,
        &[target * base.water_density / 1000.0],
        &stability::StabilityCondition::fixed(1.0, 0.0),
        &base,
    );
    for curve in &curves {
        let p = &curve.points[0];
        let gz = p.gz.expect("GZ must solve for closed box");
        let kn = p.kn.expect("KN must solve for closed box");
        assert!(
            gz.is_finite(),
            "heel {} GZ must be finite: {gz}",
            curve.heel_deg
        );
        assert!(
            kn.is_finite(),
            "heel {} KN must be finite: {kn}",
            curve.heel_deg
        );
    }
}

#[test]
fn parabola_interpolated_angle_of_max_gz_on_synthetic_curve() {
    // Parabola GZ(h) = 0.55 - 0.001 * (h - 33.5)^2
    // Vertex is at angle = 33.5 deg, max GZ = 0.55 m.
    let f = |h: f64| 0.55 - 0.001 * (h - 33.5).powi(2);
    let heels: Vec<f64> = (0..=8).map(|i| i as f64 * 10.0).collect();
    let curves: Vec<stability::CrossCurve> = heels
        .iter()
        .map(|&h| stability::CrossCurve {
            heel_deg: h,
            points: vec![stability::CrossCurvePoint {
                displacement_t: 20.0,
                draft: Some(0.5),
                trim_m: Some(0.0),
                kn: Some(f(h) + 1.0 * seacraft_core::math::deg_to_rad(h).sin()),
                gz: Some(f(h)),
            }],
        })
        .collect();
    let c = stability::evaluate_imo_criteria(&curves, 0).expect("criteria");
    assert!(
        (c.angle_of_max_gz_deg - 33.5).abs() < 1e-9,
        "angle of max GZ {}, expected 33.5",
        c.angle_of_max_gz_deg
    );
    assert!(
        (c.max_gz_30_plus_m - 0.55).abs() < 1e-9,
        "max GZ {}, expected 0.55",
        c.max_gz_30_plus_m
    );
}

/// Analytic `GZ` of a closed box section (beam `b`, depth `d`) floating half
/// full (`T = d/2`) with `G` at the centre of the section, `KG = d/2`, at any
/// heel `φ` (degrees, either sign, any turn). See `docs/oracles.md` §20.
///
/// * The section is point-symmetric about its centre `C`, so every line
///   through `C` halves its area: half full, the waterline passes through `C`
///   at every heel.
/// * Wall-sided regime, `tan φ ≤ d/b` (the waterline meets both sides):
///   `GZ = sin φ (GM + ½ BM tan² φ)`, with `BM = b²/(12T) = b²/(6d)`,
///   `KB = T/2 = d/4` and `GM = KB + BM − KG = b²/(6d) − d/4`.
/// * Past it, `tan φ ≥ d/b`, the waterline meets the deck and the bottom.
///   Turned 90° the same body is a box of beam `d` and depth `b` heeled by
///   `ψ = φ − 90°`, and the physical righting moment does not depend on how
///   the body axes are labelled: `GZ(φ) = GZ'(φ − 90°)`, the primed box in its
///   own wall-sided regime with `GM' = d²/(6b) − b/4`, `BM' = d²/(6b)`:
///   `GZ = −cos φ (GM' + ½ BM' cot² φ)`. The two forms agree at
///   `tan φ = d/b`, so the curve is continuous.
/// * Turning the box 180° about its axis maps it and `G` onto themselves, so
///   `GZ(φ + 180°) = GZ(φ)`; with the mirror symmetry `GZ(−φ) = −GZ(φ)` that
///   is `GZ(180° − φ) = −GZ(φ)`, hence `GZ(90°) = GZ(180°) = 0`.
fn half_full_box_gz(b: f64, d: f64, heel_deg: f64) -> f64 {
    // Reduce to [0°, 90°] with the two symmetries.
    let mut phi = heel_deg.rem_euclid(360.0);
    let mut sign = 1.0;
    if phi > 180.0 {
        phi -= 180.0; // GZ(φ + 180°) = GZ(φ)
    }
    if phi > 90.0 {
        phi = 180.0 - phi; // GZ(180° − φ) = −GZ(φ)
        sign = -1.0;
    }
    let r = phi.to_radians();
    let gz = if r.tan() <= d / b {
        let bm = b * b / (6.0 * d);
        let gm = bm - d / 4.0;
        r.sin() * (gm + 0.5 * bm * r.tan().powi(2))
    } else {
        let bm = d * d / (6.0 * b);
        let gm = bm - b / 4.0;
        let cot = r.cos() / r.sin();
        -r.cos() * (gm + 0.5 * bm * cot * cot)
    };
    sign * gz
}

#[test]
fn the_half_full_box_oracle_is_continuous_and_symmetric() {
    let (b, d): (f64, f64) = (4.0, 2.0);
    let knee = (d / b).atan().to_degrees();
    let below = half_full_box_gz(b, d, knee - 1e-9);
    let above = half_full_box_gz(b, d, knee + 1e-9);
    assert!((below - above).abs() < 1e-6, "{below} vs {above}");
    // At the knee both forms give sin φ · (GM + ½ BM tan² φ) = sin φ here.
    assert!((below - knee.to_radians().sin()).abs() < 1e-9);
    for h in [0.0, 10.0, 40.0, 75.0, 90.0] {
        let (a, m) = (half_full_box_gz(b, d, h), half_full_box_gz(b, d, 180.0 - h));
        assert!((a + m).abs() < 1e-12, "{h}°");
        assert!((half_full_box_gz(b, d, -h) + a).abs() < 1e-12);
    }
    assert!(half_full_box_gz(b, d, 90.0).abs() < 1e-12);
}

/// Heels 0° … 180° every 5°, with the regime change at `atan(d/b)` and its
/// mirror, either side of 90°, and exactly 90°.
fn full_circle_heels(b: f64, d: f64) -> Vec<f64> {
    let knee = (d / b).atan().to_degrees();
    let mut heels: Vec<f64> = (0..=36).map(|k| 5.0 * k as f64).collect();
    heels.extend([knee, 180.0 - knee, 89.0, 91.0, 135.001]);
    heels.sort_by(|a, b| a.partial_cmp(b).unwrap());
    heels
}

/// The large-angle oracle: a half-full box with `G` at its centre, heeled
/// from 0° to 180°, fixed and free to trim, must give the closed-form `GZ`
/// everywhere — past 90° and at exactly 90° included. A solver that places
/// the waterplane by a draft above the keel cannot: past 90° the volume falls
/// as that draft grows, and at 90° the draft does not move the plane at all.
#[test]
fn a_half_full_box_has_the_analytic_gz_from_0_to_180_degrees() {
    let (l, b, depth) = (10.0, 4.0, 2.0);
    let (t, kg) = (depth / 2.0, depth / 2.0);
    let surface = barge(l, b, depth);
    let base = options(&surface, t);
    let hull = Hull::new(&surface, base.subdivision);
    let target = l * b * t;
    let heels = full_circle_heels(b, depth);
    for cond in [
        stability::StabilityCondition::fixed(kg, 0.0),
        stability::StabilityCondition::free_to_trim(kg, l / 2.0),
    ] {
        let curves = stability::cross_curves(&hull, &heels, &[target], &cond, &base);
        for c in &curves {
            let p = &c.points[0];
            let want = half_full_box_gz(b, depth, c.heel_deg);
            let gz = p.gz.unwrap_or_else(|| panic!("no GZ at {}°", c.heel_deg));
            let kn = p.kn.expect("KN with GZ");
            assert!(
                (gz - want).abs() < 1e-6,
                "{}° ({}): GZ {gz} vs the analytic {want}",
                c.heel_deg,
                if cond.is_free() { "free" } else { "fixed" }
            );
            let want_kn = want + kg * c.heel_deg.to_radians().sin();
            assert!((kn - want_kn).abs() < 1e-6, "{}°: KN {kn}", c.heel_deg);
            // Where a draft exists it reproduces the displacement.
            if let (Some(d), Some(tr)) = (p.draft, p.trim_m) {
                let (v, _) = hull.buoyancy(d, tr, c.heel_deg);
                assert!(
                    (v - target).abs() < 1e-6 * target,
                    "{}°: draft {d} trim {tr} floats {v}, not {target}",
                    c.heel_deg
                );
            }
        }
    }
}

/// Free trim at exactly 90°. On its side the half-full box (`L` 10, `b` 4,
/// `d` 2) is a box of beam `d` and depth `b` floating at `T' = b/2 = 2 m`, so
/// the free-trim oracle of `free_trim_moves_the_lcb_to_the_requested_station`
/// applies with `T'`: an `LCG` of 4 m needs a level trim (bow over stern,
/// against the water) of `(L/2 − LCG)·12 T'/L = 2.4 m`. Every slice is then a
/// rectangle centred across the water surface, so `GZ = 0` and `KN = KG`.
/// `waterplane_for`'s trim, measured along the hull's own vertical, cannot
/// tilt the plane at 90° at all, so there is no draft or trim to report.
#[test]
fn a_box_on_its_side_trims_freely_at_exactly_90_degrees() {
    let (l, b, depth) = (10.0, 4.0, 2.0);
    let kg = depth / 2.0;
    let surface = barge(l, b, depth);
    let base = options(&surface, depth / 2.0);
    let hull = Hull::new(&surface, base.subdivision);
    let target = l * b * depth / 2.0;
    let (lcg, t_side) = (4.0, b / 2.0);
    let cond = stability::StabilityCondition::free_to_trim(kg, lcg);

    let wl = stability::solve_waterline(&hull, target, 90.0, &cond).expect("floats at 90°");
    assert!(wl.floating.is_none(), "no waterplane_for draft at 90°");
    let (v, cb) = hull.buoyancy_in(&wl.plane);
    assert!((v - target).abs() < 1e-6 * target, "volume {v}");
    assert!((cb.x - lcg).abs() < 1e-5, "LCB {} vs LCG {lcg}", cb.x);
    let n = wl.plane.n;
    let level_trim = l * n.x / n.y.hypot(n.z);
    let want = (l / 2.0 - lcg) * 12.0 * t_side / l;
    assert!(
        (level_trim - want).abs() < 1e-4,
        "level trim {level_trim} vs the analytic {want}"
    );

    let curves = stability::cross_curves(&hull, &[90.0], &[target], &cond, &base);
    let p = &curves[0].points[0];
    assert!(p.gz.expect("GZ at 90°").abs() < 1e-6, "GZ {:?}", p.gz);
    assert!(
        (p.kn.expect("KN at 90°") - kg).abs() < 1e-6,
        "KN {:?}",
        p.kn
    );
    assert!(p.draft.is_none() && p.trim_m.is_none());
}

/// The same symmetry without the half-full shortcut: a light box (`T` well
/// under half the depth) with `G` at mid-depth still maps onto itself when
/// turned 180°, so `GZ(180° − φ) = −GZ(φ)` and `GZ(90°) = GZ(180°) = 0` at
/// any displacement. The wall-sided formula covers the first and last
/// degrees, and at steep heels the waterline crosses the centreline below
/// the keel (a negative draft), which a draft bracket starting at zero
/// clamps away.
#[test]
fn a_light_box_with_g_at_mid_depth_rights_symmetrically_past_90_degrees() {
    let (l, b, depth, t) = (10.0, 4.0, 2.0, 0.6);
    let kg = depth / 2.0;
    let surface = barge(l, b, depth);
    let base = options(&surface, t);
    let hull = Hull::new(&surface, base.subdivision);
    let target = l * b * t;
    let heels: Vec<f64> = (0..=36).map(|k| 5.0 * k as f64).collect();
    let curves = stability::cross_curves(
        &hull,
        &heels,
        &[target],
        &stability::StabilityCondition::fixed(kg, 0.0),
        &base,
    );
    let gz = |h: f64| -> f64 {
        let c = curves.iter().find(|c| c.heel_deg == h).unwrap();
        c.points[0]
            .gz
            .unwrap_or_else(|| panic!("no GZ at {h}° for the light box"))
    };
    for &h in &heels {
        let (a, m) = (gz(h), gz(180.0 - h));
        assert!(
            (a + m).abs() < 1e-6,
            "GZ({h}°) = {a} but GZ({}°) = {m}",
            180.0 - h
        );
    }
    assert!(gz(90.0).abs() < 1e-6 && gz(180.0).abs() < 1e-6);
    // Wall-sided while the deck edge stays dry and the bilge wet:
    // tan φ ≤ 2T/b, here φ ≤ 16.7°.
    let bm = b * b / (12.0 * t);
    let gm = t / 2.0 + bm - kg;
    for h in [5.0, 10.0, 15.0] {
        let r = f64::to_radians(h);
        let want = r.sin() * (gm + 0.5 * bm * r.tan().powi(2));
        assert!((gz(h) - want).abs() < 1e-6, "{h}°: {} vs {want}", gz(h));
        assert!((gz(180.0 - h) + want).abs() < 1e-6, "{}°", 180.0 - h);
    }
    // Every solved waterplane floats the displacement, the negative drafts of
    // the steep heels included.
    for c in &curves {
        let p = &c.points[0];
        if let (Some(d), Some(tr)) = (p.draft, p.trim_m) {
            let (v, _) = hull.buoyancy(d, tr, c.heel_deg);
            assert!(
                (v - target).abs() < 1e-6 * target,
                "{}°: draft {d} floats {v}, not {target}",
                c.heel_deg
            );
        }
    }
}
