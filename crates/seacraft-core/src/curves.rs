//! Sectional area curve (SAC) and Bonjean curves.
//!
//! The SAC is the longitudinal distribution of submerged section area
//! `A(x)`; the Bonjean curves are the same area drawn against draft for each
//! station, `A(x, T)`. Both are cut from the **refined** mesh (see
//! [`crate::hydro::Hull`]), never from the control net.
//!
//! Conventions (see `AGENTS.md`): `x` forward, `y` starboard, `z` up, `draft`
//! measured above the keel, `trim` positive bow-up. The Bonjean curves are
//! computed upright and at even keel: they describe the hull form, not a
//! floating condition.
//!
//! Two invariants worth testing every time these curves change:
//! * `∫ A(x) dx` over the waterline equals the displacement volume, and
//! * `A(x, design draft)` equals [`crate::hydro::section_area`] at the same
//!   station, because both are the same cut with the same clipping.
//!
//! The area curve also gives a prismatic coefficient without a second
//! integration: `Cp = mean(A) / max(A)` over the measured length. It is not
//! the hydrostatic `Cp`, which divides by the *mainframe* section (the LCF by
//! default) rather than by the largest station — on the demo hull the two
//! differ by ~1.3 % because the deepest section sits forward of the LCF. Treat
//! the area-curve value as a design check, and the hydrostatic one as the
//! published coefficient.

use crate::hydro::{chain_paths, close_along_waterplane, clipped_loop_moments, plane_segments};
use crate::hydro::{Hull, SectionArea};
use crate::math::{Plane, Vec3};
use serde::Serialize;
use std::fmt::Write as _;

/// One station of the Bonjean family: the submerged section area at a list of
/// drafts, all at the same `x`.
#[derive(Debug, Clone, Serialize)]
pub struct BonjeanSection {
    pub x: f64,
    pub areas_m2: Vec<f64>,
}

/// Bonjean curves: `drafts_m[i]` pairs with `sections[j].areas_m2[i]`.
#[derive(Debug, Clone, Serialize)]
pub struct Bonjean {
    pub drafts_m: Vec<f64>,
    pub sections: Vec<BonjeanSection>,
}

impl Bonjean {
    /// Largest area anywhere in the family.
    pub fn max_area(&self) -> f64 {
        self.sections
            .iter()
            .flat_map(|s| s.areas_m2.iter())
            .fold(0.0, |m, a| m.max(*a))
    }
}

/// Cut one station of the hull once and keep the closed polygons, so every
/// draft of the Bonjean family clips the same polylines instead of re-cutting
/// the whole mesh. This is the performance-critical inner loop: a 21 × 10
/// family is 21 cuts, not 210.
pub(crate) fn station_polygons(hull: &Hull, x: f64) -> (Plane, Vec<Vec<Vec3>>) {
    let plane = Plane::new(Vec3::new(1.0, 0.0, 0.0), -x);
    let segs = plane_segments(&hull.verts, &hull.tris, &plane);
    let paths = chain_paths(&segs, 1e-7);
    (plane, close_along_waterplane(paths))
}

fn area_below(polys: &[Vec<Vec3>], plane: &Plane, wp: &Plane) -> f64 {
    clipped_loop_moments(polys, plane, wp)
        .map(|m| m.area.abs())
        .unwrap_or(0.0)
}

/// Station ordinates spanning the model length, sampled at **cell midpoints**:
/// with `n` stations the hull is divided into `n` equal slices and each station
/// sits in the middle of its slice.
///
/// Midpoint sampling is deliberate. A station that lands exactly on an end face
/// cuts a plane *coincident with that face*, and `plane_segments` then reports
/// a degenerate set of crossings — a box barge would lose a third of its end
/// ordinates and its area curve would integrate to 0.71 of the true volume. The
/// midpoints never touch a face, and the trapezoid integral of a midpoint-
/// sampled curve is the midpoint rule, which is second-order accurate.
pub fn station_xs(hull: &Hull, stations: usize) -> Vec<f64> {
    let n = stations.max(2);
    let x0 = hull.model_min.x;
    let x1 = hull.model_max.x;
    let dx = (x1 - x0) / n as f64;
    (0..n).map(|i| x0 + (i as f64 + 0.5) * dx).collect()
}

/// Bonjean curves for `stations` stations between `0` and `drafts`.
///
/// `drafts_m` must be strictly increasing and positive; the caller builds it
/// (the CLI defaults to `depth/10 .. depth` over the hull's own depth).
pub fn bonjean(hull: &Hull, stations: usize, drafts_m: &[f64]) -> Bonjean {
    let xs = station_xs(hull, stations);
    let mut sections = Vec::with_capacity(xs.len());
    for x in xs {
        let (plane, polys) = station_polygons(hull, x);
        let areas_m2 = drafts_m
            .iter()
            .map(|&d| {
                let wp = crate::hydro::waterplane_for(hull.keel, hull.length, d, 0.0, 0.0);
                area_below(&polys, &plane, &wp)
            })
            .collect();
        sections.push(BonjeanSection { x, areas_m2 });
    }
    Bonjean {
        drafts_m: drafts_m.to_vec(),
        sections,
    }
}

/// Evenly spaced drafts from `start` to `end`, inclusive. A convenience for
/// both front ends; `start` must be positive.
pub fn draft_range(start: f64, end: f64, n: usize) -> Vec<f64> {
    let n = n.max(2);
    (0..n)
        .map(|i| start + (end - start) * (i as f64 / (n - 1) as f64))
        .collect()
}

/// Prismatic coefficient read off the area curve: mean over max. Uses the
/// trapezoid mean of a curve sampled over the submerged length, and the
/// largest *sampled* ordinate as `A_max` — with few stations the maximum can be
/// under-read, so pass a fine spacing when the number matters. Returns `None`
/// when the curve is empty or flat.
pub fn prismatic_from_sac(sac: &[SectionArea]) -> Option<f64> {
    if sac.len() < 2 {
        return None;
    }
    let length = sac.last()?.x - sac.first()?.x;
    if length <= 0.0 {
        return None;
    }
    let max = sac.iter().fold(0.0_f64, |m, p| m.max(p.area));
    if max <= 1e-12 {
        return None;
    }
    // Trapezoid integral / L == mean ordinate.
    let mut integral = 0.0;
    for w in sac.windows(2) {
        integral += 0.5 * (w[0].area + w[1].area) * (w[1].x - w[0].x);
    }
    Some(integral / length / max)
}

// ---------------------------------------------------------------------------
// SVG plots
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn chart_frame(out: &mut String, title: &str, w: f64, h: f64) {
    let _ = writeln!(out, "<?xml version=\"1.0\" encoding=\"UTF-8\"?>");
    let _ = writeln!(
        out,
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{w:.0}\" height=\"{h:.0}\" viewBox=\"0 0 {w:.0} {h:.0}\">"
    );
    let _ = writeln!(out, "<title>{}</title>", esc(title));
    out.push_str("<rect width=\"100%\" height=\"100%\" fill=\"#ffffff\"/>\n");
    out.push_str("<style>\n");
    out.push_str(".axis{fill:none;stroke:#111;stroke-width:0.9}\n");
    out.push_str(".grid{fill:none;stroke:#d8d8d8;stroke-width:0.5}\n");
    out.push_str(".curve{fill:none;stroke:#1f5fa8;stroke-width:1.4}\n");
    out.push_str(".fill{fill:#1f5fa8;fill-opacity:0.14;stroke:none}\n");
    out.push_str(".station{fill:none;stroke:#1f5fa8;stroke-width:0.8;fill-opacity:0}\n");
    out.push_str(".label{font:11px ui-sans-serif,system-ui,sans-serif;fill:#333}\n");
    out.push_str(".title{font:14px ui-sans-serif,system-ui,sans-serif;fill:#111}\n");
    out.push_str("</style>\n");
}

/// Sectional area curve as a standalone SVG chart: `x` along the hull,
/// submerged section area on the y axis, with the prismatic coefficient and
/// maximum area annotated.
pub fn sac_svg(sac: &[SectionArea], title: &str) -> String {
    let w = 900.0;
    let h = 480.0;
    let (ml, mr, mt, mb) = (72.0, 24.0, 42.0, 56.0);
    let mut out = String::with_capacity(4096);
    chart_frame(&mut out, title, w, h);
    let _ = writeln!(out, "<text class=\"title\" x=\"{ml:.0}\" y=\"26\">{}</text>", esc(title));

    if sac.len() < 2 {
        let _ = writeln!(out, "<text class=\"label\" x=\"{ml:.0}\" y=\"80\">no section areas</text>");
        out.push_str("</svg>\n");
        return out;
    }

    let x0 = sac.iter().map(|p| p.x).fold(f64::INFINITY, f64::min);
    let x1 = sac.iter().map(|p| p.x).fold(f64::NEG_INFINITY, f64::max);
    let amax = sac.iter().map(|p| p.area).fold(0.0, f64::max).max(1e-6);
    let (x0, x1) = if (x1 - x0).abs() < 1e-9 { (x0 - 0.5, x0 + 0.5) } else { (x0, x1) };
    let top = amax * 1.08;

    let sx = |x: f64| ml + (x - x0) / (x1 - x0) * (w - ml - mr);
    let sy = |a: f64| mt + (1.0 - a / top) * (h - mt - mb);

    // Frame + grid lines every 10 % of the area axis.
    let _ = writeln!(
        out,
        "<rect class=\"axis\" x=\"{ml:.1}\" y=\"{mt:.1}\" width=\"{:.1}\" height=\"{:.1}\"/>",
        w - ml - mr,
        h - mt - mb
    );
    for k in 1..=4 {
        let a = amax * k as f64 / 4.0;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{ml:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            sy(a),
            w - mr,
            sy(a)
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{a:.3}</text>",
            ml - 8.0,
            sy(a) + 4.0
        );
    }

    // Filled area under the curve.
    let mut area_path = format!("M {:.2},{:.2} ", sx(sac[0].x), sy(top));
    for p in sac {
        let _ = write!(area_path, "L {:.2},{:.2} ", sx(p.x), sy(p.area));
    }
    let _ = write!(area_path, "L {:.2},{:.2} Z", sx(sac[sac.len() - 1].x), sy(top));
    let _ = writeln!(out, "<path class=\"fill\" d=\"{area_path}\"/>");

    let mut line = String::from("M ");
    for (i, p) in sac.iter().enumerate() {
        if i > 0 {
            line.push(' ');
        }
        let _ = write!(line, "{:.2},{:.2}", sx(p.x), sy(p.area));
    }
    let _ = writeln!(out, "<path class=\"curve\" d=\"{line}\"/>");

    // Axis labels: stations along x, area up the y axis.
    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">x [m]</text>",
        ml + (w - ml - mr) / 2.0,
        h - 22.0
    );
    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"18\" y=\"{:.1}\" text-anchor=\"middle\" transform=\"rotate(-90 18 {:.1})\">A(x) [m²]</text>",
        mt + (h - mt - mb) / 2.0,
        mt + (h - mt - mb) / 2.0
    );
    for k in 0..=4 {
        let xv = x0 + (x1 - x0) * k as f64 / 4.0;
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{xv:.2}</text>",
            sx(xv),
            h - mb + 18.0
        );
    }

    let mut caption = format!("A_max {amax:.3} m²");
    if let Some(cp) = prismatic_from_sac(sac) {
        let _ = write!(caption, "   Cp (area curve) {cp:.4}");
    }
    let _ = writeln!(out, "<text class=\"label\" x=\"{ml:.0}\" y=\"{:.1}\">{caption}</text>", h - 6.0);

    out.push_str("</svg>\n");
    out
}

/// Bonjean curves as a standalone SVG: draft up the vertical axis, section
/// area along the horizontal, one polyline per station.
pub fn bonjean_svg(bonjean: &Bonjean, title: &str) -> String {
    let w = 860.0;
    let h = 560.0;
    let (ml, mr, mt, mb) = (72.0, 120.0, 42.0, 56.0);
    let mut out = String::with_capacity(4096);
    chart_frame(&mut out, title, w, h);
    let _ = writeln!(out, "<text class=\"title\" x=\"{ml:.0}\" y=\"26\">{}</text>", esc(title));

    if bonjean.drafts_m.len() < 2 || bonjean.sections.is_empty() {
        let _ = writeln!(out, "<text class=\"label\" x=\"{ml:.0}\" y=\"80\">no Bonjean data</text>");
        out.push_str("</svg>\n");
        return out;
    }

    let dmax = bonjean.drafts_m.iter().cloned().fold(0.0, f64::max);
    let amax = bonjean.max_area().max(1e-6) * 1.06;
    let (dmax, amax) = (if dmax <= 0.0 { 1.0 } else { dmax }, amax);

    let sx = |a: f64| ml + a / amax * (w - ml - mr);
    let sy = |d: f64| h - mb - d / dmax * (h - mt - mb);

    let _ = writeln!(
        out,
        "<rect class=\"axis\" x=\"{ml:.1}\" y=\"{mt:.1}\" width=\"{:.1}\" height=\"{:.1}\"/>",
        w - ml - mr,
        h - mt - mb
    );
    for k in 1..=4 {
        let d = dmax * k as f64 / 4.0;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{ml:.1}\" y1=\"{:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            sy(d),
            w - mr,
            sy(d)
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"end\">{d:.2}</text>",
            ml - 8.0,
            sy(d) + 4.0
        );
    }
    for k in 0..=4 {
        let a = amax * k as f64 / 4.0;
        let _ = writeln!(
            out,
            "<line class=\"grid\" x1=\"{:.1}\" y1=\"{mt:.1}\" x2=\"{:.1}\" y2=\"{:.1}\"/>",
            sx(a),
            sx(a),
            h - mb
        );
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">{a:.2}</text>",
            sx(a),
            h - mb + 18.0
        );
    }

    for sec in &bonjean.sections {
        let mut d = String::from("M ");
        for (i, (&draft, &area)) in bonjean.drafts_m.iter().zip(sec.areas_m2.iter()).enumerate() {
            if i > 0 {
                d.push(' ');
            }
            let _ = write!(d, "{:.2},{:.2}", sx(area), sy(draft));
        }
        let _ = writeln!(out, "<path class=\"station\" d=\"{d}\"/>");
    }

    // Legend: station x, plus the area at the topmost draft (the deck area).
    let mut y = mt + 16.0;
    for sec in &bonjean.sections {
        let top = sec.areas_m2.last().copied().unwrap_or(0.0);
        let _ = writeln!(
            out,
            "<text class=\"label\" x=\"{:.1}\" y=\"{y:.1}\">x {:.2} m  A {:.3}</text>",
            w - mr + 8.0,
            sec.x,
            top
        );
        y += 14.0;
    }

    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"{:.1}\" y=\"{:.1}\" text-anchor=\"middle\">A(x, T) [m²]</text>",
        ml + (w - ml - mr) / 2.0,
        h - 22.0
    );
    let _ = writeln!(
        out,
        "<text class=\"label\" x=\"18\" y=\"{:.1}\" text-anchor=\"middle\" transform=\"rotate(-90 18 {:.1})\">draft [m]</text>",
        mt + (h - mt - mb) / 2.0,
        mt + (h - mt - mb) / 2.0
    );

    out.push_str("</svg>\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hydro::HydroOptions;
    use crate::subdiv::SubdivisionOptions;
    use crate::Surface;

    /// Closed creased box, centred on y = 0, standing on z = 0 — the same
    /// fixture as `tests/analytic.rs`, so the exact answers are known:
    /// `A(x) = B·T` everywhere inside and `A(x, d) = B·d` for `d <= depth`.
    fn barge(length: f64, beam: f64, depth: f64) -> Surface {
        use crate::mesh::VertexKind;
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
            draft: depth,
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

    const LEVEL: SubdivisionOptions = SubdivisionOptions {
        level: 2,
        mode: crate::SubdivisionMode::Loop,
    };

    #[test]
    fn bonjean_on_a_box_is_width_times_draft() {
        let s = barge(10.0, 4.0, 3.0);
        let hull = Hull::new(&s, LEVEL);
        let drafts = draft_range(0.5, 2.5, 9);
        let bj = bonjean(&hull, 11, &drafts);
        assert_eq!(bj.sections.len(), 11);
        for sec in &bj.sections {
            for (i, &d) in drafts.iter().enumerate() {
                // Interior stations only: the two end stations sit on the
                // end faces where the cut is degenerate.
                if sec.x > 0.2 && sec.x < 9.8 {
                    assert!(
                        (sec.areas_m2[i] - 4.0 * d).abs() < 1e-3,
                        "x {} d {} A {} expected {}",
                        sec.x,
                        d,
                        sec.areas_m2[i],
                        4.0 * d
                    );
                }
            }
        }
    }

    #[test]
    fn bonjean_matches_the_hydrostatic_section_area() {
        let s = crate::demo::demo_hull(false);
        let opts = HydroOptions::from_surface(&s, SubdivisionOptions { level: 3, mode: crate::SubdivisionMode::Loop });
        let hull = Hull::new(&s, opts.subdivision);
        let wp = crate::hydro::waterplane_for(hull.keel, hull.length, opts.draft, 0.0, 0.0);
        let bj = bonjean(&hull, 21, &[opts.draft]);
        for sec in &bj.sections {
            let reference = crate::hydro::section_area(&hull.verts, &hull.tris, sec.x, &wp);
            assert!(
                (sec.areas_m2[0] - reference).abs() < 1e-6,
                "x {}: {:.6} vs {:.6}",
                sec.x,
                sec.areas_m2[0],
                reference
            );
        }
    }

    #[test]
    fn prismatic_on_a_box_is_one() {
        let s = barge(10.0, 4.0, 3.0);
        let hull = Hull::new(&s, LEVEL);
        // A flat-topped area curve: A = B*T for every interior station.
        let sac: Vec<SectionArea> = (1..=9)
            .map(|i| SectionArea { x: i as f64, area: 4.0 * 1.5 })
            .collect();
        let cp = prismatic_from_sac(&sac).expect("curve is not flat");
        assert!((cp - 1.0).abs() < 1e-9, "Cp {cp}");
        // A parabolic curve halves it.
        let parabola: Vec<SectionArea> = (0..=10)
            .map(|i| {
                let t = i as f64 / 10.0;
                SectionArea { x: t * 10.0, area: 4.0 * 1.5 * t }
            })
            .collect();
        let cp = prismatic_from_sac(&parabola).expect("curve is not flat");
        assert!((cp - 0.5).abs() < 1e-9, "triangular Cp {cp}");
        // The mesh-derived curve still lands in a physically sane range.
        let bj = bonjean(&hull, 41, &[1.5]);
        let sac: Vec<SectionArea> = bj
            .sections
            .iter()
            .map(|s| SectionArea { x: s.x, area: s.areas_m2[0] })
            .collect();
        let cp = prismatic_from_sac(&sac).expect("curve is not flat");
        assert!(cp > 0.9 && cp <= 1.01, "box area-curve Cp {cp}");
    }

    #[test]
    fn svg_plots_are_well_formed() {
        let s = barge(10.0, 4.0, 3.0);
        let hull = Hull::new(&s, LEVEL);
        let bj = bonjean(&hull, 11, &draft_range(0.3, 2.4, 6));
        for svg in [
            bonjean_svg(&bj, "barge Bonjean"),
            sac_svg(
                &bj.sections
                    .iter()
                    .map(|s| SectionArea { x: s.x, area: s.areas_m2[2] })
                    .collect::<Vec<_>>(),
                "barge SAC",
            ),
        ] {
            assert!(svg.starts_with("<?xml"), "missing prolog");
            assert!(svg.trim_end().ends_with("</svg>"), "missing close tag");
            assert!(!svg.contains("NaN"), "NaN in SVG coordinates");
            assert!(!svg.contains("inf"), "infinite SVG coordinate");
            assert!(svg.contains("<path"), "no curve drawn");
        }
    }

    #[test]
    fn area_curve_integrates_to_the_displacement() {
        let s = crate::demo::demo_hull(false);
        let mut opts =
            HydroOptions::from_surface(&s, SubdivisionOptions { level: 3, mode: crate::SubdivisionMode::Loop });
        opts.stations = 0; // this test drives the stations itself
        let result = crate::hydro::compute(&s, &opts);
        // Rebuild the area curve from the same refined hull at a finer spacing
        // than the default, and integrate it by the trapezoid rule. The two
        // numbers are independent code paths through the same mesh.
        let hull = Hull::new(&s, opts.subdivision);
        let bj = bonjean(&hull, 161, &[opts.draft]);
        let mut volume = 0.0;
        for w in bj.sections.windows(2) {
            let dx = w[1].x - w[0].x;
            volume += 0.5 * (w[0].areas_m2[0] + w[1].areas_m2[0]) * dx;
        }
        assert!(
            (volume - result.volume_m3).abs() / result.volume_m3 < 0.02,
            "SAC integral {volume:.3} vs displacement {:.3}",
            result.volume_m3
        );
    }
}
