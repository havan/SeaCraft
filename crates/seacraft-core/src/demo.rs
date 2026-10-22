//! A parametric demo hull: "SeaCraft 12", a 12 m round-bilge motor cruiser.
//!
//! The control net is a station x buttock grid with a shared keel line, a
//! transom plate and a hard transom edge, which exercises every part of the
//! kernel (quads, large polygons, creases, model boundaries, subdivision and
//! hydrostatics).

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

/// Build the demo hull. When `symmetric` is true only the starboard half is
/// modelled and the layer is flagged symmetric, which is how FREE!ship models
/// are usually drawn.
pub fn demo_hull(symmetric: bool) -> Surface {
    let mut s = Surface::new();
    s.layers = vec![Layer {
        name: "Hull".into(),
        id: 1,
        color: 0x2f7fd0,
        visible: true,
        developable: false,
        symmetric,
        use_for_intersections: true,
        use_in_hydrostatics: true,
        show_in_linesplan: true,
        density: 2400.0,
        thickness: 0.012,
    }];
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
        mainframe_location: 5.2,
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
            s.add_face(
                vec![sb[i][j], sb[i][j + 1], sb[i + 1][j + 1], sb[i + 1][j]],
                0,
            );
            if !symmetric {
                s.add_face(
                    vec![pt[i + 1][j], pt[i + 1][j + 1], pt[i][j + 1], pt[i][j]],
                    0,
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
        s.add_face(transom.clone(), 0);
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
        s.add_face(half.clone(), 0);
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
