//! Acceptance checks shared by the example hulls' tests (the examples
//! board's "Acceptance" list). Every check measures at level 3, Loop.

use std::collections::HashMap;

use crate::fef;
use crate::hydro::{Hull, HydroOptions, Hydrostatics};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::project::Project;
use crate::stability::{cross_curves, solve_floating, StabilityCondition};
use crate::subdiv::{evaluate_full, SubdivisionMode, SubdivisionOptions};

pub fn level3() -> SubdivisionOptions {
    SubdivisionOptions {
        level: 3,
        mode: SubdivisionMode::Loop,
    }
}

/// Hydrostatics at the design draft (`settings.draft`), upright, with the
/// midship section at the middle of the waterline length.
pub fn measure(s: &Surface) -> Hydrostatics {
    let hull = Hull::new(s, level3());
    let mut opts = HydroOptions::from_surface(s, level3());
    opts.mainframe_x = None;
    let first = hull.hydrostatics(&opts);
    opts.mainframe_x = Some(first.wl_aft_x + 0.5 * first.lwl);
    hull.hydrostatics(&opts)
}

/// LCB as a fraction of LWL from the waterline's aft end.
pub fn lcb_frac(h: &Hydrostatics) -> f64 {
    (h.lcb.x - h.wl_aft_x) / h.lwl
}

/// The board's targets for one example.
#[derive(Debug, Clone, Copy)]
pub struct Targets {
    pub lwl: f64,
    pub bwl: f64,
    pub displacement_t: f64,
    pub cb: f64,
    pub cp: f64,
    pub cm: f64,
    pub cwp: f64,
    /// LCB, fraction of LWL from its aft end.
    pub lcb_frac: f64,
}

/// Measured against target, printed (run with `--nocapture` to see it) and
/// asserted with the board's tolerances.
pub fn assert_targets(name: &str, h: &Hydrostatics, t: &Targets) {
    let rows = [
        ("Δ (t)", t.displacement_t, h.displacement_t),
        ("LWL (m)", t.lwl, h.lwl),
        ("BWL (m)", t.bwl, h.bwl),
        ("Cb", t.cb, h.block_coefficient),
        ("Cp", t.cp, h.prismatic_coefficient),
        ("Cm", t.cm, h.midship_coefficient),
        ("Cwp", t.cwp, h.waterplane_coefficient),
        ("LCB (%LWL)", 100.0 * t.lcb_frac, 100.0 * lcb_frac(h)),
    ];
    println!("{name}: quantity  target  measured");
    for (q, target, got) in rows {
        println!("{name}: {q:<10} {target:8.4} {got:8.4}");
    }
    let rel = |got: f64, want: f64| ((got - want) / want).abs();
    let mut fails = Vec::new();
    if rel(h.displacement_t, t.displacement_t) > 0.02 {
        fails.push(format!("Δ {} vs {}", h.displacement_t, t.displacement_t));
    }
    if rel(h.lwl, t.lwl) > 0.01 {
        fails.push(format!("LWL {} vs {}", h.lwl, t.lwl));
    }
    if rel(h.bwl, t.bwl) > 0.01 {
        fails.push(format!("BWL {} vs {}", h.bwl, t.bwl));
    }
    if (h.block_coefficient - t.cb).abs() > 0.02 {
        fails.push(format!("Cb {} vs {}", h.block_coefficient, t.cb));
    }
    if (h.prismatic_coefficient - t.cp).abs() > 0.02 {
        fails.push(format!("Cp {} vs {}", h.prismatic_coefficient, t.cp));
    }
    if (h.midship_coefficient - t.cm).abs() > 0.03 {
        fails.push(format!("Cm {} vs {}", h.midship_coefficient, t.cm));
    }
    if (h.waterplane_coefficient - t.cwp).abs() > 0.03 {
        fails.push(format!("Cwp {} vs {}", h.waterplane_coefficient, t.cwp));
    }
    if (lcb_frac(h) - t.lcb_frac).abs() > 0.005 {
        fails.push(format!("LCB {} vs {}", lcb_frac(h), t.lcb_frac));
    }
    assert!(fails.is_empty(), "{name} misses its targets: {fails:?}");
    assert!(h.warnings.is_empty(), "{name}: {:?}", h.warnings);
}

/// No leak below the waterline: on the control net no boundary edge below
/// DWL + 0.05 T except on the centreline and no edge with more than two
/// faces; every interior edge is walked in opposite directions by its two
/// faces (consistent winding); on the mirrored level-3 mesh every edge
/// touching the submerged part has exactly two triangles; and the raw
/// divergence volume is positive (outward normals).
pub fn assert_closed_below(s: &Surface) {
    let keel = s.lowest_hydrostatics_point();
    let draft = s.settings.draft;
    let limit = keel + 1.05 * draft;
    let counts = s.edge_face_counts();
    for (e, &c) in s.edges.iter().zip(&counts) {
        let (a, b) = (s.points[e.a].p, s.points[e.b].p);
        assert!(c <= 2, "non-manifold control edge {a:?}-{b:?}");
        if c == 1 && a.z.min(b.z) < limit {
            assert!(
                a.y.abs() < 1e-9 && b.y.abs() < 1e-9,
                "boundary edge below DWL + 0.05 T off the centreline: {a:?}-{b:?}"
            );
        }
    }

    let mut directed: HashMap<(usize, usize), usize> = HashMap::new();
    for f in &s.faces {
        for k in 0..f.len() {
            *directed.entry((f[k], f[(k + 1) % f.len()])).or_default() += 1;
        }
    }
    for (&(a, b), &n) in &directed {
        assert_eq!(n, 1, "edge {a}->{b} walked twice the same way");
    }

    let full = evaluate_full(s, level3());
    let tris: Vec<[u32; 3]> = full
        .triangles()
        .into_iter()
        .map(|t| [t[0] as u32, t[1] as u32, t[2] as u32])
        .collect();
    let (verts, tris) = crate::threemf::weld(&full.verts, &tris, 1e-6);
    let mut edges: HashMap<(u32, u32), usize> = HashMap::new();
    for t in &tris {
        for k in 0..3 {
            let (u, v) = (t[k], t[(k + 1) % 3]);
            *edges.entry((u.min(v), u.max(v))).or_default() += 1;
        }
    }
    let dwl = keel + draft;
    for (&(u, v), &c) in &edges {
        let (pu, pv) = (verts[u as usize], verts[v as usize]);
        if pu.z <= dwl + 1e-4 || pv.z <= dwl + 1e-4 {
            assert_eq!(c, 2, "leak on the refined hull at {pu:?}-{pv:?}");
        }
    }

    let wp = crate::hydro::waterplane(s, draft, 0.0, 0.0);
    let origin = wp.project(Vec3::ZERO);
    let mut raw = 0.0;
    for t in &full.triangles() {
        let p = [full.verts[t[0]], full.verts[t[1]], full.verts[t[2]]];
        if p.iter().all(|q| wp.side(*q) <= 0.0) {
            raw += (p[0] - origin).dot((p[1] - origin).cross(p[2] - origin)) / 6.0;
        }
    }
    assert!(raw > 0.0, "inward normals: raw volume {raw}");
}

/// GM at `kg` (upright, design draft) and GZ at 0° free to trim at the
/// design displacement and `lcg`. Asserts GM ≥ 0.15 m and |GZ(0)| < 1 mm.
pub fn assert_stability(s: &Surface, kg: f64, lcg: f64) -> (f64, f64) {
    let h = measure(s);
    let gm = h.gm(kg);
    let hull = Hull::new(s, level3());
    let base = HydroOptions::from_surface(s, level3());
    let curves = cross_curves(
        &hull,
        &[0.0],
        &[h.displacement_t],
        &StabilityCondition::free_to_trim(kg, lcg),
        &base,
    );
    let gz0 = curves[0].points[0].gz.expect("GZ at 0°");
    println!(
        "stability: KM {:.4} KG {kg:.3} GM {gm:.4} GZ(0) {gz0:.2e}",
        h.km_transverse
    );
    assert!(gm >= 0.15, "GM {gm} below 0.15 m at KG {kg}");
    assert!(gz0.abs() < 1e-3, "GZ(0°) = {gz0}");
    (gm, gz0)
}

/// The Design loading condition floats free to trim within 0.5 % of LWL.
/// Returns (draft, trim).
pub fn assert_free_trim(s: &Surface, p: &Project) -> (f64, f64) {
    let c = p
        .loading_condition(Some("Design"))
        .expect("a Design condition");
    let all: Vec<_> = std::iter::once(&c.lightship).chain(&c.items).collect();
    let mass: f64 = all.iter().map(|w| w.mass_t).sum();
    let lcg = all.iter().map(|w| w.mass_t * w.x).sum::<f64>() / mass;
    let vcg = all.iter().map(|w| w.mass_t * w.z).sum::<f64>() / mass;
    let kg = p.kg.expect("the example's KG");
    assert!(
        (vcg - kg).abs() < 5e-3,
        "Design VCG {vcg} is not the example KG {kg}"
    );
    let hull = Hull::new(s, level3());
    let volume = mass * 1000.0 / s.settings.water_density;
    let fc = solve_floating(&hull, volume, 0.0, lcg, 0.0).expect("floats");
    let lwl = s.settings.length;
    println!(
        "free trim: Δ {mass:.3} t LCG {lcg:.3} -> T {:.4} trim {:.4} ({:.2} % LWL)",
        fc.draft,
        fc.trim,
        100.0 * fc.trim / lwl
    );
    assert!(
        fc.trim.abs() < 0.005 * lwl,
        "trim {} exceeds 0.5 % of LWL",
        fc.trim
    );
    (fc.draft, fc.trim)
}

/// `.fef` written, read and written again is the same text, and the read
/// model measures the same.
pub fn assert_fef_round_trip(s: &Surface) {
    let text = fef::write(s);
    let back = fef::parse(&text).expect("parses");
    assert_eq!(
        fef::write(&back),
        text,
        "the .fef round trip changed the file"
    );
    assert_eq!(back.points.len(), s.points.len());
    for (a, b) in back.points.iter().zip(&s.points) {
        assert_eq!(a.p, b.p, "a coordinate changed on the round trip");
        assert_eq!(a.kind, b.kind);
    }
    let (h0, h1) = (measure(s), measure(&back));
    assert_eq!(h0.volume_m3, h1.volume_m3);
}

/// The sidecar validates and survives its JSON round trip.
pub fn assert_project_valid(p: &Project) {
    p.validate().expect("the sidecar validates");
    assert_eq!(&Project::from_json(&p.to_json()).expect("loads"), p);
    assert!(p.kg.is_some(), "an example carries its KG");
    let r = p.requirements.as_ref().expect("requirements");
    r.validate().expect("requirements validate");
    p.loading_condition(Some("Design"))
        .expect("a Design condition");
}
