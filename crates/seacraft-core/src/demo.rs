//! A parametric demo hull: "SeaCraft 12", a 12 m round-bilge motor cruiser.
//!
//! The control net is a station x buttock grid with a shared keel line, a
//! transom plate and a hard transom edge, which exercises every part of the
//! kernel (quads, large polygons, creases, model boundaries, subdivision and
//! hydrostatics).
//!
//! The faces sit on three layers, the way a designer would split this hull
//! for plating and paint: **Bottom** (keel to the turn of the bilge, the
//! first three girth strips), **Topsides** (turn of the bilge to the sheer)
//! and **Transom**. Every layer counts in the hydrostatics and shares the
//! model's symmetry, so the layers change no number: subdivision does not
//! look at layers, and measurement only reads those two flags. There is no
//! deck: the hull is open at the sheer, as FREE!ship hulls usually are, so
//! there is no lid to keep out of the hydrostatics.

use crate::math::Vec3;
use crate::mesh::{Layer, ProjectSettings, Surface};

/// Station table: `(x, half beam, keel z, sheer z)`.
const STATIONS: [(f64, f64, f64, f64); 9] = [
    (0.00, 1.62, 0.00, 0.98),
    (1.60, 1.86, 0.00, 1.00),
    (3.40, 2.00, 0.00, 1.04),
    (5.20, 2.00, 0.00, 1.08),
    (7.00, 1.90, 0.04, 1.14),
    (8.60, 1.58, 0.18, 1.22),
    (10.00, 1.12, 0.42, 1.36),
    (11.20, 0.58, 0.72, 1.52),
    (12.00, 0.06, 1.02, 1.66),
];

/// Section ordinates `t` from the keel (0) to the sheer (1).
const SECTION_T: [f64; 6] = [0.0, 0.18, 0.36, 0.55, 0.75, 1.0];

fn section_offsets(half_beam: f64, keel: f64, sheer: f64, t: f64) -> (f64, f64) {
    // Round bilge: the section opens up quickly at the bottom, then rises to
    // the sheer line.
    let y = half_beam * (std::f64::consts::FRAC_PI_2 * t).sin().powf(0.9);
    let z = keel + (sheer - keel) * t.powf(1.55);
    (y, z)
}

/// Girth strips (counted from the keel) that belong to the bottom; the rest
/// are topsides. Strip 2 ends at `t = 0.55`, where the section turns from
/// the flat of the bottom into the side.
const BOTTOM_STRIPS: usize = 3;

/// Layer indices of the demo hull.
pub const LAYER_BOTTOM: usize = 0;
pub const LAYER_TOPSIDES: usize = 1;
pub const LAYER_TRANSOM: usize = 2;

fn demo_layer(name: &str, id: i32, color: i32, symmetric: bool, thickness: f64) -> Layer {
    Layer {
        name: name.into(),
        id,
        color,
        visible: true,
        developable: false,
        symmetric,
        use_for_intersections: true,
        use_in_hydrostatics: true,
        show_in_linesplan: true,
        density: 2400.0,
        thickness,
    }
}

/// The demo hull's KG above the keel (m): part of the demo, like its
/// reference numbers. No other model gets a KG it was not given.
pub const DEMO_KG: f64 = 2.2;

/// The demo's design sidecar: its KG and nothing else, so it changes no
/// number but GM and the GZ curves, which need a KG. `seacraft demo` writes
/// it next to the `.fef` (`examples/demo-hull.seacraft.json`).
pub fn demo_project() -> crate::project::Project {
    crate::project::Project {
        kg: Some(DEMO_KG),
        ..Default::default()
    }
}

/// Build the demo hull. When `symmetric` is true only the starboard half is
/// modelled and the layers are flagged symmetric, which is how FREE!ship
/// models are usually drawn.
pub fn demo_hull(symmetric: bool) -> Surface {
    let mut s = Surface::new();
    s.layers = vec![
        // Antifouling red, sea blue, varnished teak.
        demo_layer("Bottom", 1, 0xb0_4a_3c, symmetric, 0.014),
        demo_layer("Topsides", 2, 0x2f_7f_d0, symmetric, 0.010),
        demo_layer("Transom", 3, 0xc8_8f_3a, symmetric, 0.016),
    ];
    s.settings = ProjectSettings {
        name: "SeaCraft 12".into(),
        designer: "SeaCraft".into(),
        created_by: format!("SeaCraft {} demo", crate::VERSION),
        comment: "Parametric round-bilge motor cruiser, 12 m".into(),
        length: 12.0,
        beam: 4.0,
        draft: 0.62,
        water_density: 1025.0,
        appendage_coefficient: 1.0,
        imperial: false,
        main_particulars_set: true,
        // 0 means "measure the midship section at the LCF". The `.fef` format
        // carries no mainframe station (see `fef.rs`), so a demo that set one
        // here would measure at that station in memory and at the LCF after a
        // save/load round trip — two different `Cm` and `Cp` from the same
        // model. Pass `--mainframe 5.2` (or `mainframe_x` over MCP) to measure
        // the section at midships instead.
        mainframe_location: 0.0,
    };

    let ns = STATIONS.len();
    let np = SECTION_T.len() - 1;

    // Indices: keel[i], sb[i][j], pt[i][j] (j = 1..np)
    let mut keel = Vec::with_capacity(ns);
    let mut sb: Vec<Vec<usize>> = Vec::with_capacity(ns);
    let mut pt: Vec<Vec<usize>> = Vec::with_capacity(ns);

    for &(x, hb, k, sh) in STATIONS.iter() {
        keel.push(s.add_point(Vec3::new(x, 0.0, k)));
        let mut row_sb = vec![0usize; np + 1];
        let mut row_pt = vec![0usize; np + 1];
        row_sb[0] = *keel.last().unwrap();
        row_pt[0] = *keel.last().unwrap();
        for j in 1..=np {
            let (y, z) = section_offsets(hb, k, sh, SECTION_T[j]);
            row_sb[j] = s.add_point(Vec3::new(x, y, z));
            if !symmetric {
                row_pt[j] = s.add_point(Vec3::new(x, -y, z));
            }
        }
        sb.push(row_sb);
        pt.push(row_pt);
    }

    // Hull plating.
    for i in 0..ns - 1 {
        for j in 0..np {
            let layer = if j < BOTTOM_STRIPS {
                LAYER_BOTTOM
            } else {
                LAYER_TOPSIDES
            };
            s.add_face(
                vec![sb[i][j], sb[i][j + 1], sb[i + 1][j + 1], sb[i + 1][j]],
                layer,
            );
            if !symmetric {
                s.add_face(
                    vec![pt[i + 1][j], pt[i + 1][j + 1], pt[i][j + 1], pt[i][j]],
                    layer,
                );
            }
        }
    }

    // Transom: a single polygon closing the stern, oriented towards -x.
    let mut transom: Vec<usize> = Vec::new();
    for j in (1..=np).rev() {
        transom.push(sb[0][j]);
    }
    transom.push(keel[0]);
    transom.extend(pt[0].iter().skip(1).take(np));
    if !symmetric {
        let normal = polygon_normal(&s, &transom);
        if normal.x > 0.0 {
            transom.reverse();
        }
        s.add_face(transom.clone(), LAYER_TRANSOM);
        crease_loop(&mut s, &transom);
    } else {
        // Half model: close the stern from the keel up the starboard side and
        // back down the centreline, so the mirrored half forms a watertight
        // transom and the waterplane closes into a ring.
        let apex = s.add_point(Vec3::new(0.0, 0.0, STATIONS[0].3));
        let mut half: Vec<usize> = Vec::new();
        for j in (1..=np).rev() {
            half.push(sb[0][j]);
        }
        half.push(keel[0]);
        half.push(apex);
        let normal = polygon_normal(&s, &half);
        if normal.x > 0.0 {
            half.reverse();
        }
        s.add_face(half.clone(), LAYER_TRANSOM);
        crease_loop(&mut s, &half);
    }

    s.initialize_trace();
    s
}

fn crease_loop(s: &mut Surface, idx: &[usize]) {
    s.rebuild_edges();
    for k in 0..idx.len() {
        let a = idx[k];
        let b = idx[(k + 1) % idx.len()];
        if let Some(e) = s
            .edges
            .iter_mut()
            .find(|e| e.key() == crate::mesh::ordered(a, b))
        {
            e.crease = true;
        }
    }
}

fn polygon_normal(s: &Surface, idx: &[usize]) -> Vec3 {
    // Newell's method.
    let mut n = Vec3::ZERO;
    for i in 0..idx.len() {
        let a = s.points[idx[i]].p;
        let b = s.points[idx[(i + 1) % idx.len()]].p;
        n.x += (a.y - b.y) * (a.z + b.z);
        n.y += (a.z - b.z) * (a.x + b.x);
        n.z += (a.x - b.x) * (a.y + b.y);
    }
    n
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::{compute, HydroOptions};
    use crate::subdiv::SubdivisionOptions;

    /// Lowest and highest control point z over the faces of `layer`.
    fn z_range(s: &Surface, layer: usize) -> (f64, f64) {
        s.faces
            .iter()
            .zip(&s.face_layer)
            .filter(|(_, &l)| l == layer)
            .flat_map(|(f, _)| f.iter().map(|&p| s.points[p].p.z))
            .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), z| {
                (lo.min(z), hi.max(z))
            })
    }

    #[test]
    fn the_demo_has_named_layers_that_each_own_faces() {
        for symmetric in [false, true] {
            let s = demo_hull(symmetric);
            s.validate().expect("valid");
            let names: Vec<&str> = s.layers.iter().map(|l| l.name.as_str()).collect();
            assert_eq!(names, ["Bottom", "Topsides", "Transom"]);
            let counts = s.layer_face_counts();
            assert!(counts.iter().all(|&c| c > 0), "{counts:?}");
            assert_eq!(counts[LAYER_TRANSOM], 1, "one transom face");
            assert_eq!(counts.iter().sum::<usize>(), s.faces.len());
            let ids: std::collections::HashSet<i32> = s.layers.iter().map(|l| l.id).collect();
            assert_eq!(ids.len(), 3, "unique ids");
            let colours: std::collections::HashSet<i32> =
                s.layers.iter().map(|l| l.color).collect();
            assert_eq!(colours.len(), 3, "distinct colours");
            assert!(s.layers.iter().all(|l| l.symmetric == symmetric));
            assert!(s.layers.iter().all(|l| l.use_in_hydrostatics && l.visible));
            // The bottom is below the topsides.
            let (bottom_lo, bottom_hi) = z_range(&s, LAYER_BOTTOM);
            let (top_lo, top_hi) = z_range(&s, LAYER_TOPSIDES);
            assert!(bottom_lo < top_lo && bottom_hi < top_hi);
        }
    }

    #[test]
    fn the_layers_change_no_number() {
        // The same hull on one layer measures bit for bit the same: layers
        // only decide which faces are mirrored and measured, and all of them
        // are, with the model's symmetry.
        for symmetric in [false, true] {
            let layered = demo_hull(symmetric);
            let mut single = layered.clone();
            single.layers.truncate(1);
            single.face_layer.iter_mut().for_each(|l| *l = 0);
            let opts = HydroOptions::from_surface(&layered, SubdivisionOptions::default());
            let a = compute(&layered, &opts);
            let b = compute(&single, &opts);
            assert_eq!(format!("{a:?}"), format!("{b:?}"));
            if !symmetric {
                // The AGENTS.md reference (the full model, as in examples/).
                assert!((a.volume_m3 - 11.851).abs() < 0.001, "{}", a.volume_m3);
            }
        }
    }

    /// The committed `examples/demo-hull.fef` is the full demo hull (up to
    /// the version in its "created by" line) and its sidecar carries the
    /// demo's KG and nothing else: `seacraft demo -o examples/demo-hull.fef`
    /// writes both.
    #[test]
    fn committed_demo_files_match_the_demo() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../examples");
        let fef_path = dir.join("demo-hull.fef");
        let text = std::fs::read_to_string(&fef_path)
            .unwrap_or_else(|e| panic!("{}: {e}", fef_path.display()));
        let mut on_disk = crate::fef::parse(&text).expect("demo-hull.fef parses");
        let built = demo_hull(false);
        on_disk.settings.created_by = built.settings.created_by.clone();
        assert!(
            crate::fef::write(&on_disk) == crate::fef::write(&built),
            "{} differs from demo_hull(false); run `seacraft demo -o examples/demo-hull.fef`",
            fef_path.display()
        );
        let sidecar = crate::project::sidecar_path(&fef_path);
        let json = std::fs::read_to_string(&sidecar)
            .unwrap_or_else(|e| panic!("{}: {e}", sidecar.display()));
        assert_eq!(json, demo_project().to_json(), "{}", sidecar.display());
        let project = crate::project::Project::from_json(&json).unwrap();
        assert_eq!(project.kg, Some(DEMO_KG));
        assert_eq!(
            crate::project::Project {
                kg: None,
                ..project
            },
            crate::project::Project::default(),
            "the demo sidecar carries only KG"
        );
    }
}
