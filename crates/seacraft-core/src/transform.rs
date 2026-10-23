//! Hull transformation: scale, translate and mirror the control net.
//!
//! A design rarely changes proportion by redrawing it: it is scaled to a new
//! length, beam or draft and the offsets follow. These functions move the
//! **control points** — the subdivision weights in [`crate::subdiv`] are not
//! touched, so the transformed net refines to the transformed shape, and every
//! measurement afterwards is still taken on a refined mesh.
//!
//! Anchor points, chosen so a scaled model stays where the designer left it:
//!
//! * `x` scales about the aft end (`x_min`), so the bow moves and the transom
//!   stays put — the usual way a hull is stretched;
//! * `y` scales about the centreline (`y = 0`), which is what keeps a symmetric
//!   half model symmetric;
//! * `z` scales about the keel, so `draft` above the keel scales by the same
//!   factor and the hull does not sink through the ground plane.
//!
//! Mirroring reverses the winding of every face. A reflection in one axis flips
//! the orientation of a closed mesh, and a hull with reversed normals measures
//! a negative volume — the divergence integral does not care that the shape
//! looks right. [`mirror`] therefore reverses each face's point order, and the
//! test for it asserts the volume is unchanged rather than merely that the
//! vertices moved.

use crate::hydro::{self, HydroOptions};
use crate::math::Vec3;
use crate::mesh::Surface;
use crate::subdiv::SubdivisionOptions;
use serde::Serialize;

/// Which axis to reflect in.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Axis {
    X,
    Y,
    Z,
}

impl Axis {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_ascii_lowercase().as_str() {
            "x" | "longitudinal" => Some(Self::X),
            "y" | "transverse" => Some(Self::Y),
            "z" | "vertical" => Some(Self::Z),
            _ => None,
        }
    }
}

/// Main dimensions as measured on the refined mesh at the model's design draft.
#[derive(Debug, Clone, Copy, Serialize)]
pub struct MainDimensions {
    pub lwl: f64,
    pub bwl: f64,
    pub draft: f64,
    pub volume_m3: f64,
}

impl MainDimensions {
    pub fn measure(surface: &Surface, subdiv: SubdivisionOptions) -> Option<Self> {
        let mut opts = HydroOptions::from_surface(surface, subdiv);
        if opts.draft <= 0.0 {
            return None;
        }
        opts.stations = 0; // a fit only needs the integrals, not the curve
        let h = hydro::compute(surface, &opts);
        (h.lwl > 0.0 && h.bwl > 0.0).then_some(Self {
            lwl: h.lwl,
            bwl: h.bwl,
            draft: h.draft,
            volume_m3: h.volume_m3,
        })
    }
}

/// Target main dimensions; `None` leaves that dimension alone.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct Targets {
    pub lwl: Option<f64>,
    pub bwl: Option<f64>,
    pub draft: Option<f64>,
}

impl Targets {
    pub fn is_empty(&self) -> bool {
        self.lwl.is_none() && self.bwl.is_none() && self.draft.is_none()
    }
}

/// What a caller wants done to the model, in one request.
#[derive(Debug, Clone, Copy, Default, Serialize)]
pub struct TransformRequest {
    pub lwl: Option<f64>,
    pub bwl: Option<f64>,
    pub draft: Option<f64>,
    /// Extra axis scale applied *after* the target fit.
    pub scale: Option<Vec3>,
    pub translate: Vec3,
    pub mirror: Option<Axis>,
}

impl TransformRequest {
    pub fn is_empty(&self) -> bool {
        self.lwl.is_none()
            && self.bwl.is_none()
            && self.draft.is_none()
            && self.scale.is_none()
            && self.translate == Vec3::ZERO
            && self.mirror.is_none()
    }
}

/// The outcome of [`run`].
#[derive(Debug, Clone, Serialize)]
pub struct TransformReport {
    pub fit: Option<FitReport>,
    pub before: Option<MainDimensions>,
    pub after: Option<MainDimensions>,
    pub scale: Vec3,
    pub translate: Vec3,
    pub mirror: Option<Axis>,
}

/// Apply a whole [`TransformRequest`] to a copy of `surface`.
///
/// The order is fixed and documented: fit to the target dimensions, then the
/// extra scale, then the translation, then the mirror. One implementation so the
/// CLI, the control socket and the MCP server cannot drift apart in what
/// "transform" means.
pub fn run(
    surface: &Surface,
    request: &TransformRequest,
    subdiv: SubdivisionOptions,
) -> Result<(Surface, TransformReport), String> {
    if request.is_empty() {
        return Err(
            "nothing to do: give target dimensions, a scale, a translation or a mirror".to_string(),
        );
    }
    let before = MainDimensions::measure(surface, subdiv);
    let targets = Targets {
        lwl: request.lwl,
        bwl: request.bwl,
        draft: request.draft,
    };
    let (mut out, fit) = if targets.is_empty() {
        (surface.clone(), None)
    } else {
        let (fitted, report) = fit_to_targets(surface, targets, subdiv, 1e-3, 4)?;
        (fitted, Some(report))
    };
    let mut cumulative_scale = fit
        .as_ref()
        .map(|f| f.scale)
        .unwrap_or(Vec3::new(1.0, 1.0, 1.0));
    if let Some(scale) = request.scale {
        apply(&mut out, scale, Vec3::ZERO)?;
        cumulative_scale = Vec3::new(
            cumulative_scale.x * scale.x,
            cumulative_scale.y * scale.y,
            cumulative_scale.z * scale.z,
        );
    }
    if request.translate != Vec3::ZERO {
        apply(&mut out, Vec3::new(1.0, 1.0, 1.0), request.translate)?;
    }
    if let Some(axis) = request.mirror {
        mirror(&mut out, axis)?;
    }
    let after = MainDimensions::measure(&out, subdiv);
    Ok((
        out,
        TransformReport {
            fit,
            before,
            after,
            scale: cumulative_scale,
            translate: request.translate,
            mirror: request.mirror,
        },
    ))
}

/// What [`fit_to_targets`] did.
#[derive(Debug, Clone, Serialize)]
pub struct FitReport {
    /// Cumulative scale factor applied to the input model.
    pub scale: Vec3,
    /// Iterations the residual solve needed (1 is exact for length and draft).
    pub iterations: usize,
    pub before: MainDimensions,
    pub after: MainDimensions,
    /// Residual relative error per target, largest of the three.
    pub max_error: f64,
}

fn anchor(surface: &Surface) -> Vec3 {
    let (min, _) = surface.extents();
    Vec3::new(min.x, 0.0, surface.lowest_hydrostatics_point())
}

/// Scale the net about its anchors (see the module docs) and translate it.
///
/// The project's main particulars are scaled with it: `length` and `beam` by the
/// `x`/`y` factors, `draft` by the `z` factor, and `mainframe_location` follows
/// the `x` scale from the same aft anchor. A factor must be finite and positive.
pub fn apply(surface: &mut Surface, scale: Vec3, translate: Vec3) -> Result<(), String> {
    for (name, v) in [("x", scale.x), ("y", scale.y), ("z", scale.z)] {
        if !v.is_finite() || v <= 0.0 {
            return Err(format!("scale.{name} must be a positive finite number"));
        }
    }
    if !(translate.x.is_finite() && translate.y.is_finite() && translate.z.is_finite()) {
        return Err("translate must be finite".into());
    }
    let a = anchor(surface);
    for p in &mut surface.points {
        p.p = Vec3::new(
            a.x + (p.p.x - a.x) * scale.x + translate.x,
            a.y + (p.p.y - a.y) * scale.y + translate.y,
            a.z + (p.p.z - a.z) * scale.z + translate.z,
        );
    }
    let s = &mut surface.settings;
    s.length *= scale.x;
    s.beam *= scale.y;
    s.draft *= scale.z;
    if s.mainframe_location != 0.0 {
        s.mainframe_location = a.x + (s.mainframe_location - a.x) * scale.x + translate.x;
    }
    Ok(())
}

/// Reflect the model in `axis`, in place, keeping the extents where they are.
///
/// Every face's point order is reversed, because a reflection flips the
/// handedness of the mesh; without that the outward normals point inward and
/// the hydrostatics come out negative. Reflecting `y` is refused on a model
/// with a symmetric layer: the port half is generated by the symmetry, not
/// stored, so mirroring the stored half would model the same side twice.
pub fn mirror(surface: &mut Surface, axis: Axis) -> Result<(), String> {
    if axis == Axis::Y && surface.layers.iter().any(|l| l.symmetric) {
        return Err(
            "cannot mirror y: a symmetric layer generates the port half, so the stored half \
             already covers both sides"
                .to_string(),
        );
    }
    let (min, max) = surface.extents();
    let centre = match axis {
        Axis::X => 0.5 * (min.x + max.x),
        Axis::Y => 0.0,
        Axis::Z => {
            surface.lowest_hydrostatics_point()
                + 0.5 * (max.z - surface.lowest_hydrostatics_point())
        }
    };
    for p in &mut surface.points {
        match axis {
            Axis::X => p.p.x = 2.0 * centre - p.p.x,
            Axis::Y => p.p.y = -p.p.y,
            Axis::Z => p.p.z = 2.0 * centre - p.p.z,
        }
    }
    for face in &mut surface.faces {
        face.reverse();
    }
    let s = &mut surface.settings;
    match axis {
        Axis::X => {
            if s.mainframe_location != 0.0 {
                s.mainframe_location = 2.0 * centre - s.mainframe_location;
            }
        }
        Axis::Y => {}
        Axis::Z => {
            // The keel is the new datum: the draft above it is unchanged,
            // because the reflection kept the extents.
        }
    }
    Ok(())
}

/// Scale the model so that the measured `LWL`, `BWL` and draft hit the targets.
///
/// Length and draft are exact after one pass (both scale linearly about the
/// anchors). Beam is not: widening the hull *and* deepening it moves the
/// waterline up a flared topside, so the beam measured at the new draft is
/// wider than the scaled one. The solve therefore re-measures and corrects on
/// the same model until the residual is below `tolerance` (relative) or
/// `max_iterations` passes have run — two or three are enough for a real hull.
///
/// The input surface is not modified; the fitted model is returned with the
/// report.
pub fn fit_to_targets(
    surface: &Surface,
    targets: Targets,
    subdiv: SubdivisionOptions,
    tolerance: f64,
    max_iterations: usize,
) -> Result<(Surface, FitReport), String> {
    if targets.is_empty() {
        return Err("no target dimensions given".into());
    }
    for (name, v) in [
        ("lwl", targets.lwl),
        ("bwl", targets.bwl),
        ("draft", targets.draft),
    ] {
        if let Some(v) = v {
            if !v.is_finite() || v <= 0.0 {
                return Err(format!("target {name} must be a positive finite number"));
            }
        }
    }
    let before = MainDimensions::measure(surface, subdiv).ok_or_else(|| {
        "cannot measure the model: it has no waterline (draft must be positive)".to_string()
    })?;

    let mut out = surface.clone();
    let mut cumulative = Vec3::new(1.0, 1.0, 1.0);
    let mut current = before;
    let mut iterations = 0;
    let mut max_error = f64::INFINITY;
    for _ in 0..max_iterations.max(1) {
        iterations += 1;
        let sx = targets.lwl.map_or(1.0, |t| t / current.lwl);
        let sy = targets.bwl.map_or(1.0, |t| t / current.bwl);
        let sz = targets.draft.map_or(1.0, |t| t / current.draft);
        apply(&mut out, Vec3::new(sx, sy, sz), Vec3::ZERO)?;
        cumulative = Vec3::new(cumulative.x * sx, cumulative.y * sy, cumulative.z * sz);
        current = MainDimensions::measure(&out, subdiv)
            .ok_or_else(|| "the scaled model could not be measured".to_string())?;
        max_error = [
            targets.lwl.map(|t| (current.lwl - t).abs() / t),
            targets.bwl.map(|t| (current.bwl - t).abs() / t),
            targets.draft.map(|t| (current.draft - t).abs() / t),
        ]
        .into_iter()
        .flatten()
        .fold(0.0_f64, f64::max);
        if max_error <= tolerance {
            break;
        }
    }
    Ok((
        out,
        FitReport {
            scale: cumulative,
            iterations,
            before,
            after: current,
            max_error,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesh::VertexKind;
    use crate::subdiv::SubdivisionMode;

    const LEVEL: SubdivisionOptions = SubdivisionOptions {
        level: 2,
        mode: SubdivisionMode::Loop,
    };

    fn barge(length: f64, beam: f64, depth: f64) -> Surface {
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
            length,
            beam,
            draft: depth * 0.5,
            water_density: 1000.0,
            appendage_coefficient: 1.0,
            main_particulars_set: true,
            ..crate::ProjectSettings::default()
        };
        let (l, b, d) = (length, beam, depth);
        let corners = [
            (0.0, 0.0, 0.0),
            (l, 0.0, 0.0),
            (l, b, 0.0),
            (0.0, b, 0.0),
            (0.0, 0.0, d),
            (l, 0.0, d),
            (l, b, d),
            (0.0, b, d),
        ];
        for (x, y, z) in corners {
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
        for edge in &mut s.edges {
            edge.crease = true;
        }
        for point in &mut s.points {
            point.kind = VertexKind::Corner;
        }
        s
    }

    fn hydro(surface: &Surface, level: u8) -> hydro::Hydrostatics {
        let mut opts = HydroOptions::from_surface(
            surface,
            SubdivisionOptions {
                level,
                mode: SubdivisionMode::Loop,
            },
        );
        opts.stations = 0;
        hydro::compute(surface, &opts)
    }

    #[test]
    fn scaling_a_box_scales_its_displacement_cubically() {
        let mut s = barge(10.0, 4.0, 3.0);
        let before = hydro(&s, 2);
        apply(&mut s, Vec3::new(2.0, 2.0, 2.0), Vec3::ZERO).expect("scale");
        let after = hydro(&s, 2);
        assert!(
            (after.volume_m3 / before.volume_m3 - 8.0).abs() < 1e-6,
            "volume ratio {}",
            after.volume_m3 / before.volume_m3
        );
        assert!((after.lwl - 2.0 * before.lwl).abs() < 1e-6);
        assert!((after.bwl - 2.0 * before.bwl).abs() < 1e-6);
        assert!(
            (after.draft - 2.0 * before.draft).abs() < 1e-9,
            "settings draft scales"
        );
        assert!((after.block_coefficient - before.block_coefficient).abs() < 1e-9);
        // The anchor: the aft end and the keel stay put.
        let (min, _) = s.extents();
        assert!(min.x.abs() < 1e-9, "x_min moved to {}", min.x);
        assert!(min.z.abs() < 1e-9, "keel moved to {}", min.z);
    }

    #[test]
    fn fit_hits_length_beam_and_draft_on_a_real_hull() {
        let s = crate::demo::demo_hull(false);
        let targets = Targets {
            lwl: Some(15.0),
            bwl: Some(4.6),
            draft: Some(0.8),
        };
        let (fitted, report) = fit_to_targets(
            &s,
            targets,
            SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
            1e-3,
            4,
        )
        .expect("fit");
        assert!(
            report.max_error < 1e-3,
            "residual {}: {report:?}",
            report.max_error
        );
        assert!(report.iterations <= 4);
        assert!((report.after.lwl - 15.0).abs() / 15.0 < 1e-3, "{report:?}");
        assert!((report.after.bwl - 4.6).abs() / 4.6 < 1e-3, "{report:?}");
        assert!((report.after.draft - 0.8).abs() < 1e-3, "{report:?}");
        assert!(report.after.volume_m3 > report.before.volume_m3);
        // The original is untouched: a fit works on a copy.
        let original = MainDimensions::measure(
            &s,
            SubdivisionOptions {
                level: 3,
                mode: SubdivisionMode::Loop,
            },
        )
        .expect("measure");
        assert!((original.lwl - report.before.lwl).abs() < 1e-9);
        assert!(
            (fitted.settings.draft - 0.8).abs() < 1e-9,
            "{}",
            fitted.settings.draft
        );
    }

    #[test]
    fn translating_moves_the_centre_of_buoyancy_and_nothing_else() {
        let mut s = barge(10.0, 4.0, 3.0);
        let before = hydro(&s, 2);
        apply(&mut s, Vec3::new(1.0, 1.0, 1.0), Vec3::new(5.0, 0.0, 0.0)).expect("translate");
        let after = hydro(&s, 2);
        assert!((after.volume_m3 - before.volume_m3).abs() < 1e-9);
        assert!(
            (after.lcb.x - (before.lcb.x + 5.0)).abs() < 1e-9,
            "lcb {}",
            after.lcb.x
        );
        assert!((after.lcb.z - before.lcb.z).abs() < 1e-9);
        assert!((after.block_coefficient - before.block_coefficient).abs() < 1e-9);
    }

    #[test]
    fn mirroring_keeps_the_volume_and_flips_the_ends() {
        let s = crate::demo::demo_hull(false);
        let before = hydro(&s, 2);

        let mut flipped = s.clone();
        mirror(&mut flipped, Axis::X).expect("mirror x");
        let after = hydro(&flipped, 2);
        assert!(
            (after.volume_m3 - before.volume_m3).abs() / before.volume_m3 < 1e-6,
            "mirroring must not change the volume: {:.6} vs {:.6}",
            after.volume_m3,
            before.volume_m3
        );
        let (min, max) = s.extents();
        assert!(
            (after.lcb.x - (min.x + max.x - before.lcb.x)).abs() < 1e-6,
            "lcb did not mirror: {} vs {}",
            after.lcb.x,
            min.x + max.x - before.lcb.x
        );
        // Normals still point out: a reversed mesh integrates a negative volume.
        assert!(after.volume_m3 > 0.0);

        // Port/starboard: same hull by symmetry, so the same volume, and the
        // y coordinate of the centre of buoyancy flips sign.
        let mut stbd = s.clone();
        mirror(&mut stbd, Axis::Y).expect("mirror y");
        let mirrored = hydro(&stbd, 2);
        assert!((mirrored.volume_m3 - before.volume_m3).abs() / before.volume_m3 < 1e-9);
        assert!((mirrored.lcb.y + before.lcb.y).abs() < 1e-9);
    }

    #[test]
    fn mirroring_y_a_symmetric_model_is_refused() {
        let mut s = crate::demo::demo_hull(true);
        let err = mirror(&mut s, Axis::Y).expect_err("a half model cannot be mirrored");
        assert!(err.contains("symmetric layer"), "{err}");
        // The other axes are fine.
        assert!(mirror(&mut s, Axis::X).is_ok());
    }

    #[test]
    fn bad_factors_are_rejected() {
        let mut s = barge(10.0, 4.0, 3.0);
        for bad in [
            Vec3::new(0.0, 1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
            Vec3::new(1.0, f64::NAN, 1.0),
            Vec3::new(f64::INFINITY, 1.0, 1.0),
        ] {
            assert!(apply(&mut s, bad, Vec3::ZERO).is_err(), "{bad:?}");
        }
        assert!(apply(
            &mut s,
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(f64::NAN, 0.0, 0.0)
        )
        .is_err());
        assert!(fit_to_targets(&s, Targets::default(), LEVEL, 1e-3, 3).is_err());
        assert!(fit_to_targets(
            &s,
            Targets {
                lwl: Some(-1.0),
                ..Targets::default()
            },
            LEVEL,
            1e-3,
            3
        )
        .is_err());
    }
}
