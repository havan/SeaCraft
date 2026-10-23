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
        subdivision: SubdivisionOptions { level: 2, mode: Default::default() },
    }
}

#[test]
fn a_closed_barge_has_exact_hydrostatics() {
    let (l, b, depth, t) = (10.0, 4.0, 4.0, 2.0);
    let surface = barge(l, b, depth);
    let opts = options(&surface, t);
    let h = hydro::compute(&surface, &opts);

    let volume = l * b * t;
    assert!((h.volume_m3 - volume).abs() < 1e-6 * volume, "volume {} vs {volume}", h.volume_m3);
    assert!((h.displacement_t - volume).abs() < 1e-6 * volume, "rho 1000: {} t", h.displacement_t);

    // Coefficients are exactly one for a box at its own depth.
    assert!((h.block_coefficient - 1.0).abs() < 1e-9, "Cb {}", h.block_coefficient);
    assert!((h.waterplane_coefficient - 1.0).abs() < 1e-9, "Cwp {}", h.waterplane_coefficient);
    assert!((h.midship_coefficient - 1.0).abs() < 1e-9, "Cm {}", h.midship_coefficient);
    assert!((h.prismatic_coefficient - 1.0).abs() < 1e-9, "Cp {}", h.prismatic_coefficient);

    // Buoyancy at half depth, metacentres from the waterplane second moments.
    assert!((h.kb - t / 2.0).abs() < 1e-9, "KB {}", h.kb);
    assert!((h.waterplane_area_m2 - l * b).abs() < 1e-9, "Awp {}", h.waterplane_area_m2);
    assert!((h.bm_transverse - b * b / (12.0 * t)).abs() < 1e-9, "BMt {}", h.bm_transverse);
    assert!((h.bm_longitudinal - l * l / (12.0 * t)).abs() < 1e-6, "BMl {}", h.bm_longitudinal);
    assert!((h.km_transverse - (t / 2.0 + b * b / (12.0 * t))).abs() < 1e-9, "KM {}", h.km_transverse);

    // Centre of buoyancy and flotation sit amidships, on the centreline.
    assert!((h.lcb.x - l / 2.0).abs() < 1e-9, "LCB x {}", h.lcb.x);
    assert!(h.lcb.y.abs() < 1e-9, "LCB y {}", h.lcb.y);
    assert!((h.lcf.x - l / 2.0).abs() < 1e-9, "LCF x {}", h.lcf.x);

    // Half the box is wetted: bottom + four sides up to the waterline.
    let wetted = l * b + 2.0 * l * t + 2.0 * b * t;
    assert!((h.wetted_surface_m2 - wetted).abs() < 1e-6 * wetted, "wetted {}", h.wetted_surface_m2);
    assert!((h.mainframe_area_m2 - b * t).abs() < 1e-9, "midship {}", h.mainframe_area_m2);
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
        subdivision: SubdivisionOptions { level: 3, mode: Default::default() },
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
    assert!(fc.trim.abs() < 1e-3, "a symmetric box should not trim: {}", fc.trim);
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
        subdivision: SubdivisionOptions { level: 3, mode: Default::default() },
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
        assert!(gz.is_finite(), "heel {} GZ must be finite: {gz}", curve.heel_deg);
        assert!(kn.is_finite(), "heel {} KN must be finite: {kn}", curve.heel_deg);
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
