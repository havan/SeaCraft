//! Acceptance checks shared by the example hulls' tests (the examples
//! design's acceptance list). Every check measures at level 3, Loop.

use std::collections::{HashMap, HashSet};

use super::polygon_normal;
use crate::fef;
use crate::hydro::{Hull, HydroOptions, Hydrostatics};
use crate::loading::evaluate_tank;
use crate::math::Vec3;
use crate::mesh::{ordered, Surface};
use crate::project::Project;
use crate::stability::{cross_curves, solve_floating, StabilityCondition};
use crate::subdiv::{evaluate, evaluate_full, SubdivisionMode, SubdivisionOptions};

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

/// The design targets for one example.
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
/// asserted with the design tolerances.
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

/// The Design loading condition — lightship, items and tank contents —
/// floats free to trim within 0.5 % of LWL. Returns (draft, trim).
pub fn assert_free_trim(s: &Surface, p: &Project) -> (f64, f64) {
    let c = p
        .loading_condition(Some("Design"))
        .expect("a Design condition");
    let hull = Hull::new(s, level3());
    let mut all: Vec<(f64, f64, f64)> = std::iter::once(&c.lightship)
        .chain(&c.items)
        .map(|w| (w.mass_t, w.x, w.z))
        .collect();
    for t in &c.tanks {
        let e = evaluate_tank(t, Some(&hull));
        all.push((e.mass_t, e.x, e.z));
    }
    let mass: f64 = all.iter().map(|w| w.0).sum();
    let lcg = all.iter().map(|w| w.0 * w.1).sum::<f64>() / mass;
    let vcg = all.iter().map(|w| w.0 * w.2).sum::<f64>() / mass;
    let kg = p.kg.expect("the example's KG");
    assert!(
        (vcg - kg).abs() < 5e-3,
        "Design VCG {vcg} is not the example KG {kg}"
    );
    let volume =
        mass * 1000.0 / (s.settings.water_density * s.settings.appendage_coefficient.max(1e-6));
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

/// The transom face of a half model: the largest control face whose edges
/// are all creases (its outline and the centreline) and that faces aft.
fn transom_face(s: &Surface) -> usize {
    let creased: HashSet<(usize, usize)> = s
        .edges
        .iter()
        .filter(|e| e.crease)
        .map(|e| e.key())
        .collect();
    (0..s.faces.len())
        .filter(|&fi| {
            let f = &s.faces[fi];
            (0..f.len()).all(|k| creased.contains(&ordered(f[k], f[(k + 1) % f.len()])))
        })
        .map(|fi| (fi, polygon_normal(s, &s.faces[fi])))
        .filter(|(_, n)| n.x < -0.5 * n.length())
        .max_by(|a, b| a.1.length().total_cmp(&b.1.length()))
        .map(|(fi, _)| fi)
        .expect("a transom face")
}

/// Fan triangles of `faces`.
fn fan(faces: &[Vec<usize>]) -> Vec<[usize; 3]> {
    faces
        .iter()
        .flat_map(|f| (1..f.len().saturating_sub(1)).map(move |i| [f[0], f[i], f[i + 1]]))
        .collect()
}

/// The largest `y` where the line through `(x, ·, z)` along the y axis
/// meets one of `tris` (a half model's half-breadth there), or `None`.
/// Triangles seen edge-on from the side (a flat deck, a transom whose plane
/// contains the y axis) are skipped.
fn half_breadth_at(verts: &[Vec3], tris: &[[usize; 3]], x: f64, z: f64) -> Option<f64> {
    let cross = |u: Vec3, v: Vec3| u.x * v.z - u.z * v.x;
    let mut best: Option<f64> = None;
    for t in tris {
        let a = verts[t[0]];
        let (ab, ac, ap) = (verts[t[1]] - a, verts[t[2]] - a, Vec3::new(x, 0.0, z) - a);
        let d = cross(ab, ac);
        if d.abs() <= 1e-12 * ab.length() * ac.length() {
            continue;
        }
        let (l1, l2) = (cross(ap, ac) / d, cross(ab, ap) / d);
        if l1 < -1e-9 || l2 < -1e-9 || l1 + l2 > 1.0 + 1e-9 {
            continue;
        }
        let y = a.y + l1 * ab.y + l2 * ac.y;
        best = Some(best.map_or(y, |m| m.max(y)));
    }
    best
}

/// Where the edges of `faces` cross the height `z`: the crossing with the
/// largest `y`, as `(x, y)` (the transom outline at that height).
fn outline_at(verts: &[Vec3], faces: &[Vec<usize>], z: f64) -> Option<(f64, f64)> {
    let mut best: Option<(f64, f64)> = None;
    for f in faces {
        for k in 0..f.len() {
            let (p, q) = (verts[f[k]], verts[f[(k + 1) % f.len()]]);
            if p.z == q.z || (p.z - z) * (q.z - z) > 0.0 {
                continue;
            }
            let t = (z - p.z) / (q.z - p.z);
            let (x, y) = (p.x + t * (q.x - p.x), p.y + t * (q.y - p.y));
            if best.is_none_or(|b| y > b.1) {
                best = Some((x, y));
            }
        }
    }
    best
}

/// The transom keeps its shape through refinement (level 3, half model).
/// Every corner of its outline — a control point where three or more
/// crease edges meet, the chine or the sheer on the transom — stays within
/// 5 mm of the control net; and between the transom and the station at
/// `next_x` the hull is nowhere narrower than the refined transom outline
/// at the same height by more than 5 mm, wherever neither the control net
/// nor the refined station at `next_x` is narrower than the transom there.
/// Returns the largest narrowing found (m).
pub fn assert_transom_outline(s: &Surface, next_x: f64) -> f64 {
    let ring = s.faces[transom_face(s)].clone();
    let mut creases = vec![0usize; s.points.len()];
    for e in s.edges.iter().filter(|e| e.crease) {
        creases[e.a] += 1;
        creases[e.b] += 1;
    }
    let fine = evaluate(s, level3());
    let corners: Vec<usize> = ring.iter().copied().filter(|&p| creases[p] >= 3).collect();
    assert!(
        corners.iter().any(|&p| s.points[p].p.y > 1e-9),
        "no chine or sheer corner on the transom outline"
    );
    for &p in &corners {
        let moved = (fine.verts[p] - s.points[p].p).length();
        println!("transom corner {:?} moved {moved:.2e} m", s.points[p].p);
        assert!(
            moved < 5e-3,
            "transom corner {:?} moved {moved} m on refinement",
            s.points[p].p
        );
    }

    // The refined transom is every face in the (planar) transom's plane;
    // the deck is left out of the breadths (near the sheer a ray would
    // find the deck where the refined sheer runs below the net's).
    let net: Vec<Vec3> = s.points.iter().map(|p| p.p).collect();
    let n = polygon_normal(s, &ring);
    let n = n / n.length();
    let c = ring.iter().fold(Vec3::ZERO, |c, &p| c + net[p]) / ring.len() as f64;
    let on_plane = |p: Vec3| (p - c).dot(n).abs() < 1e-4;
    let deck = |l: usize| s.layers.get(l).is_some_and(|l| l.name == "Deck");
    let mut fine_transom: Vec<Vec<usize>> = Vec::new();
    let mut fine_hull: Vec<Vec<usize>> = Vec::new();
    for (f, &l) in fine.faces.iter().zip(&fine.face_layer) {
        if f.iter().all(|&p| on_plane(fine.verts[p])) {
            fine_transom.push(f.clone());
        } else if !deck(l) {
            fine_hull.push(f.clone());
        }
    }
    assert!(
        !fine_transom.is_empty(),
        "the refined transom is not planar"
    );
    let net_hull: Vec<Vec<usize>> = s
        .faces
        .iter()
        .zip(&s.face_layer)
        .filter(|(f, &l)| **f != ring && !deck(l))
        .map(|(f, _)| f.clone())
        .collect();
    let mut net_verts = net.clone();
    let mut net_tris: Vec<[usize; 3]> = Vec::new();
    for f in &net_hull {
        if f.len() == 3 {
            net_tris.push([f[0], f[1], f[2]]);
        } else {
            let c = f.iter().fold(Vec3::ZERO, |sum, &p| sum + net_verts[p]) / f.len() as f64;
            let ci = net_verts.len();
            net_verts.push(c);
            for i in 0..f.len() {
                net_tris.push([f[i], f[(i + 1) % f.len()], ci]);
            }
        }
    }
    let fine_tris = fan(&fine_hull);
    let net_transom = [ring.clone()];

    let zs: Vec<f64> = ring.iter().map(|&p| net[p].z).collect();
    let (z0, z1) = zs
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(a, b), &z| {
            (a.min(z), b.max(z))
        });
    let mut worst = 0.0f64;
    for i in 1..40 {
        let z = z0 + (z1 - z0) * i as f64 / 40.0;
        let (Some((xa, ya)), Some((_, yn))) = (
            outline_at(&fine.verts, &fine_transom, z),
            outline_at(&net, &net_transom, z),
        ) else {
            continue;
        };
        // A pinch is a dip below both ends of the bay: compare only where
        // the refined next station is at least as wide as the transom.
        let ahead = half_breadth_at(&fine.verts, &fine_tris, next_x, z);
        if ahead.is_none_or(|y| y < ya) {
            continue;
        }
        for k in 1..8 {
            let x = xa + (next_x - xa) * k as f64 / 8.0;
            let widens =
                half_breadth_at(&net_verts, &net_tris, x, z).is_some_and(|y| y >= yn - 1e-9);
            let Some(y) = half_breadth_at(&fine.verts, &fine_tris, x, z) else {
                continue;
            };
            if !widens {
                continue;
            }
            if ya - y > worst {
                worst = ya - y;
            }
            assert!(
                ya - y <= 5e-3,
                "at x {x:.3}, z {z:.3} the hull is {:.4} m narrower than the transom ({ya:.4} m)",
                ya - y
            );
        }
    }
    println!("transom: largest narrowing forward of it {worst:.2e} m");
    worst
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
    // Draft and depth share the keel datum, so the deck is above the water.
    if let (Some(t), Some(d)) = (r.draft, r.depth) {
        assert!(d > t, "requirements depth {d} is not above the draft {t}");
    }
    p.loading_condition(Some("Design"))
        .expect("a Design condition");
}
