//! Control-net topology edits: split a face, insert an edge, toggle a crease.
//!
//! These are the operations a modeller needs but the `.fef` importer never
//! produces: change the control net's *topology* without moving the surface.
//! Both splits are closed under manifoldness — the new edge or fan is interior
//! to a face, so every new edge has exactly two adjacent faces — and neither
//! touches the geometry: [`split_face`] adds the face's centroid, which lies in
//! the face's own plane when the face is planar, and [`insert_edge`] only
//! re-partitions the polygon's vertices.
//!
//! On a *non-planar* quad a fan through the centroid does move the limit
//! surface slightly, because the four triangles and the quad refine to different
//! surfaces. That is a property of subdivision, not of the edit: the control net
//! area is unchanged, and the measured displacement moves by well under a
//! percent (see the tests).
//!
//! After a successful edit the edge list is rebuilt with
//! [`Surface::rebuild_edges`], which preserves crease flags on the edges that
//! survive and leaves the new interior edges uncreased.

use crate::math::Vec3;
use crate::mesh::{ordered, Surface, VertexKind};
use serde::Serialize;

/// What [`split_face`] did.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct SplitFace {
    /// The face that was split (it keeps this index and becomes one triangle).
    pub face: usize,
    /// Index of the new centroid control point.
    pub point: usize,
    /// Faces the original one became.
    pub pieces: usize,
    pub faces_before: usize,
    pub faces_after: usize,
    pub points_before: usize,
    pub points_after: usize,
}

/// Split `face` into a fan of triangles around its centroid ("poke").
///
/// The face keeps its index and its layer; the remaining triangles are appended.
/// A three-sided face is already a fan of one triangle, so splitting it adds the
/// centroid and three triangles that refine differently — allowed, and useful
/// for adding resolution where a crease or a plate boundary will go.
pub fn split_face(surface: &mut Surface, face: usize) -> Result<SplitFace, String> {
    let faces_before = surface.faces.len();
    let points_before = surface.points.len();
    let verts = surface
        .faces
        .get(face)
        .cloned()
        .ok_or_else(|| format!("no face {face}: the model has {faces_before}"))?;
    if verts.len() < 3 {
        return Err(format!(
            "face {face} has {} points, a face needs at least 3",
            verts.len()
        ));
    }
    let layer = surface.face_layer.get(face).copied().unwrap_or(0);
    if layer >= surface.layers.len() {
        return Err(format!("face {face} references missing layer {layer}"));
    }
    let mut c = Vec3::ZERO;
    for &v in &verts {
        c += surface.points[v].p;
    }
    c = c / verts.len() as f64;
    let centre = surface.add_point(c);
    surface.points[centre].kind = VertexKind::Regular;

    let n = verts.len();
    // An n-gon needs n triangles to fan it: the loop must include the edge from
    // the last vertex back to the first, or the face loses that boundary edge
    // and the neighbouring face is left with a one-sided edge (a leak).
    // The first triangle takes the original index, so a caller that held `face`
    // still refers to something sensible afterwards.
    surface.faces[face] = vec![verts[0], verts[1], centre];
    for i in 1..n {
        let f = vec![verts[i], verts[(i + 1) % n], centre];
        surface.add_face(f, layer);
    }
    surface.rebuild_edges();
    Ok(SplitFace {
        face,
        point: centre,
        pieces: n,
        faces_before,
        faces_after: surface.faces.len(),
        points_before,
        points_after: surface.points.len(),
    })
}

/// Insert an edge between two non-adjacent points of `face`, splitting it in two.
///
/// Both points must belong to the face and must not already be neighbours along
/// it; otherwise the "edge" is already there and the split would produce a
/// degenerate face. Both halves keep the original face's layer.
pub fn insert_edge(surface: &mut Surface, face: usize, a: usize, b: usize) -> Result<(), String> {
    let faces_before = surface.faces.len();
    let verts = surface
        .faces
        .get(face)
        .cloned()
        .ok_or_else(|| format!("no face {face}: the model has {faces_before}"))?;
    let layer = surface.face_layer.get(face).copied().unwrap_or(0);
    let n = verts.len();
    let i = verts
        .iter()
        .position(|&v| v == a)
        .ok_or_else(|| format!("point {a} is not part of face {face}"))?;
    let j = verts
        .iter()
        .position(|&v| v == b)
        .ok_or_else(|| format!("point {b} is not part of face {face}"))?;
    if a == b {
        return Err("an edge needs two different points".to_string());
    }
    let steps = (j + n - i) % n;
    if steps == 1 || steps == n - 1 {
        return Err(format!(
            "points {a} and {b} are already neighbours on face {face}"
        ));
    }
    // Half one: a .. b going forward. Half two: b .. a going forward.
    let mut first: Vec<usize> = Vec::with_capacity(steps + 1);
    for k in 0..=steps {
        first.push(verts[(i + k) % n]);
    }
    let mut second: Vec<usize> = Vec::with_capacity(n - steps + 1);
    for k in 0..=(n - steps) {
        second.push(verts[(j + k) % n]);
    }
    if first.len() < 3 || second.len() < 3 {
        return Err("the split would leave a face with fewer than 3 points".to_string());
    }
    surface.faces[face] = first;
    surface.add_face(second, layer);
    surface.rebuild_edges();
    Ok(())
}

/// Mark (or clear) the control edge between `a` and `b` as a crease.
///
/// The edge must exist. A crease keeps the edge sharp through subdivision —
/// `subdiv.rs` averages a crease vertex only with exactly two crease neighbours
/// — so this is how a chine, a knuckle or a transom edge is made hard.
pub fn set_crease(surface: &mut Surface, a: usize, b: usize, crease: bool) -> Result<(), String> {
    if a == b {
        return Err("an edge needs two different points".to_string());
    }
    let key = ordered(a, b);
    let edge = surface
        .edges
        .iter_mut()
        .find(|e| e.key() == key)
        .ok_or_else(|| format!("no control edge between {a} and {b}"))?;
    edge.crease = crease;
    Ok(())
}

/// Whether a control edge exists between `a` and `b`.
pub fn has_edge(surface: &Surface, a: usize, b: usize) -> bool {
    let key = ordered(a, b);
    surface.edges.iter().any(|e| e.key() == key)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::{Hull, HydroOptions};
    use crate::subdiv::{SubdivisionMode, SubdivisionOptions};

    const LEVEL: SubdivisionOptions = SubdivisionOptions {
        level: 2,
        mode: SubdivisionMode::Loop,
    };

    fn barge() -> Surface {
        let (l, b, d) = (10.0, 4.0, 3.0);
        let mut s = Surface::new();
        s.layers = vec![crate::Layer {
            name: "Hull".into(),
            symmetric: false,
            use_in_hydrostatics: true,
            visible: true,
            ..crate::Layer::default()
        }];
        s.settings = crate::ProjectSettings {
            name: "barge".into(),
            length: l,
            beam: b,
            draft: d * 0.5,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..crate::ProjectSettings::default()
        };
        for (x, y, z) in [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ] {
            s.add_point(Vec3::new(x, y - b * 0.5, z));
        }
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
        for e in &mut s.edges {
            e.crease = true;
        }
        for p in &mut s.points {
            p.kind = VertexKind::Corner;
        }
        s
    }

    fn volume(s: &Surface) -> f64 {
        let opts = HydroOptions::from_surface(s, LEVEL);
        Hull::new(s, LEVEL).hydrostatics(&opts).volume_m3
    }

    #[test]
    fn splitting_every_face_keeps_the_box_a_box() {
        let mut s = barge();
        let before = volume(&s);
        let n = s.faces.len();
        for f in 0..n {
            let r = split_face(&mut s, f).expect("split");
            assert_eq!(r.pieces, 4, "a quad becomes four triangles");
        }
        assert_eq!(s.faces.len(), n * 4);
        assert_eq!(s.points.len(), 8 + n);
        assert!(s.validate().is_ok(), "{:?}", s.validate());
        // Every new edge is interior: exactly two faces, no leaks.
        let counts = s.edge_face_counts();
        assert!(
            counts.iter().all(|c| *c == 2),
            "non-manifold edge: {counts:?}"
        );
        // The box stays the box: the poke point is in the face plane.
        let after = volume(&s);
        assert!(
            (after - before).abs() / before < 1e-9,
            "volume {before} -> {after}"
        );
    }

    #[test]
    fn inserting_a_diagonal_splits_a_quad_without_moving_it() {
        let mut s = barge();
        let before = volume(&s);
        let n = s.faces.len();
        // Face 0 is the bottom quad [0, 3, 2, 1]; 0 and 2 are diagonal.
        insert_edge(&mut s, 0, 0, 2).expect("insert");
        assert_eq!(s.faces.len(), n + 1);
        assert_eq!(s.faces[0].len(), 3);
        assert!(s.validate().is_ok());
        let counts = s.edge_face_counts();
        assert!(counts.iter().all(|c| *c == 2), "{counts:?}");
        assert!(has_edge(&s, 0, 2), "the diagonal is now an edge");
        assert!((volume(&s) - before).abs() / before < 1e-12);
    }

    #[test]
    fn a_hexagon_can_be_split_into_two_quads() {
        let mut s = Surface::new();
        let pts: Vec<usize> = (0..6)
            .map(|i| {
                let a = std::f64::consts::TAU * i as f64 / 6.0;
                s.add_point(Vec3::new(a.cos(), a.sin(), 0.0))
            })
            .collect();
        s.add_face(pts.clone(), 0);
        s.rebuild_edges();
        insert_edge(&mut s, 0, pts[0], pts[3]).expect("split a hexagon across its far points");
        assert_eq!(s.faces.len(), 2);
        assert_eq!(s.faces[0].len(), 4);
        assert_eq!(s.faces[1].len(), 4);
        assert!(s.validate().is_ok());
    }

    #[test]
    fn bad_edits_are_refused() {
        let mut s = barge();
        assert!(split_face(&mut s, 99).is_err());
        // A point that is not on the face.
        let p = s.add_point(Vec3::new(5.0, 5.0, 5.0));
        assert!(insert_edge(&mut s, 0, 0, p).is_err());
        // Neighbours on the face are already joined.
        assert!(insert_edge(&mut s, 0, 0, 3).is_err());
        assert!(insert_edge(&mut s, 0, 0, 0).is_err());
        // A crease needs an edge to sit on.
        assert!(set_crease(&mut s, 0, 7, true).is_err());
        assert!(set_crease(&mut s, 0, 0, true).is_err());
        // Nothing was changed by the refusals.
        assert_eq!(s.faces.len(), 6);
    }

    #[test]
    fn creases_can_be_set_and_cleared() {
        let mut s = barge();
        // Start from a clean net: no creases at all.
        for e in &mut s.edges {
            e.crease = false;
        }
        set_crease(&mut s, 0, 3, true).expect("set");
        assert!(s.edges.iter().any(|e| e.key() == ordered(0, 3) && e.crease));
        set_crease(&mut s, 3, 0, false).expect("clear, argument order irrelevant");
        assert!(s
            .edges
            .iter()
            .all(|e| !e.crease || e.key() != ordered(0, 3)));
    }

    #[test]
    fn a_split_on_a_curved_hull_barely_moves_the_displacement() {
        let s = crate::demo::demo_hull(false);
        let opts = HydroOptions::from_surface(
            &s,
            SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
        );
        let before = Hull::new(&s, opts.subdivision)
            .hydrostatics(&opts)
            .volume_m3;
        let mut edited = s.clone();
        // Split a face on the bilge, where the quad is not planar.
        let mut best = 0usize;
        let mut best_y = -1.0;
        for (i, f) in edited.faces.iter().enumerate() {
            let y = f.iter().map(|&p| edited.points[p].p.y).sum::<f64>() / f.len() as f64;
            if y > best_y {
                best_y = y;
                best = i;
            }
        }
        split_face(&mut edited, best).expect("split");
        let after = Hull::new(&edited, opts.subdivision)
            .hydrostatics(&opts)
            .volume_m3;
        assert!(
            (after - before).abs() / before < 5e-3,
            "a fan through a curved quad moved the displacement {before:.4} -> {after:.4}"
        );
        assert!(edited.validate().is_ok());
    }
}
