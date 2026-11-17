//! Control-net visibility against the refined hull BVH.
//!
//! For each viewport, determines which control points and which stretches of
//! each control edge are occluded by the displayed refined hull surface.

use eframe::egui;
use seacraft_core::math::Vec3;
use seacraft_core::mesh::Surface;
use seacraft_core::raycast::Bvh;

use crate::camera::ViewTransform;

/// Per-view occlusion classification of the control net.
#[derive(Debug, Clone, PartialEq)]
pub struct NetVisibility {
    /// For each control point in `Surface::points`, `true` when visible.
    pub points: Vec<bool>,
    /// For each control edge in `Surface::edges`, parametric intervals `(t0, t1)`
    /// in `0.0..=1.0` that are occluded by the hull.
    pub hidden_spans: Vec<Vec<(f32, f32)>>,
}

/// Classify control points and edge spans of `surface` against `bvh` as seen from `view` in `cell`.
pub fn net_visibility(
    surface: &Surface,
    bvh: &Bvh,
    view: &ViewTransform,
    cell: egui::Rect,
    bias: f64,
) -> NetVisibility {
    if bvh.is_empty() {
        return NetVisibility {
            points: vec![true; surface.points.len()],
            hidden_spans: vec![Vec::new(); surface.edges.len()],
        };
    }

    let (lo, hi) = surface.extents();
    let diag = if surface.points.is_empty() {
        1.0
    } else {
        (hi - lo).length().max(1e-6)
    };
    let l = 4.0 * diag;
    let b = bias.max(1e-3 * diag);

    let is_visible = |p: Vec3| -> bool {
        let to = if view.perspective {
            view.eye
        } else {
            p - view.forward * l
        };
        let delta = to - p;
        let dist = delta.length();
        if dist <= b {
            return true;
        }
        let dir = delta * (1.0 / dist);
        let a = p + dir * b;
        !bvh.any_hit(a, to)
    };

    let points: Vec<bool> = surface.points.iter().map(|cp| is_visible(cp.p)).collect();

    let mut hidden_spans = Vec::with_capacity(surface.edges.len());
    for edge in &surface.edges {
        let (Some(wa), Some(wb)) = (
            surface.points.get(edge.a).map(|cp| cp.p),
            surface.points.get(edge.b).map(|cp| cp.p),
        ) else {
            hidden_spans.push(Vec::new());
            continue;
        };
        let n = match (view.project(wa, cell), view.project(wb, cell)) {
            (Some(pa), Some(pb)) => ((pb - pa).length() / 16.0).ceil().clamp(2.0, 64.0) as usize,
            _ => 2,
        };
        let ab = wb - wa;
        let mut spans = Vec::new();
        let mut run_start: Option<usize> = None;
        for k in 0..n {
            let t = (k as f64 + 0.5) / n as f64;
            let p = wa + ab * t;
            let hidden = !is_visible(p);
            match (run_start, hidden) {
                (None, true) => run_start = Some(k),
                (Some(k0), false) => {
                    spans.push((k0 as f32 / n as f32, k as f32 / n as f32));
                    run_start = None;
                }
                _ => {}
            }
        }
        if let Some(k0) = run_start {
            spans.push((k0 as f32 / n as f32, 1.0));
        }
        hidden_spans.push(spans);
    }

    NetVisibility {
        points,
        hidden_spans,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::camera::{Camera, ViewKind};
    use crate::App;
    use seacraft_core::mesh::ControlEdge;

    fn box_visibility_fixture() -> (Bvh, Surface, ViewTransform, egui::Rect, f64) {
        let verts = vec![
            Vec3::new(-1.0, -1.0, -1.0),
            Vec3::new(1.0, -1.0, -1.0),
            Vec3::new(1.0, 1.0, -1.0),
            Vec3::new(-1.0, 1.0, -1.0),
            Vec3::new(-1.0, -1.0, 1.0),
            Vec3::new(1.0, -1.0, 1.0),
            Vec3::new(1.0, 1.0, 1.0),
            Vec3::new(-1.0, 1.0, 1.0),
        ];
        let tris = vec![
            [0, 2, 1],
            [0, 3, 2],
            [4, 5, 6],
            [4, 6, 7],
            [0, 4, 7],
            [0, 7, 3],
            [1, 2, 6],
            [1, 6, 5],
            [0, 1, 5],
            [0, 5, 4],
            [3, 7, 6],
            [3, 6, 2],
        ];
        let bvh = Bvh::build(&verts, &tris);
        let mut surface = Surface::new();
        surface.add_point(Vec3::new(-2.0, 0.0, 0.0));
        surface.add_point(Vec3::new(2.0, 0.0, 0.0));
        surface.add_point(Vec3::new(-1.0, 0.0, 0.0));
        surface.add_point(Vec3::new(2.0, -3.0, 0.0));
        surface.add_point(Vec3::new(2.0, 3.0, 0.0));
        surface.edges.push(ControlEdge {
            a: 3,
            b: 4,
            crease: false,
        });
        let view = Camera::ortho(ViewKind::Body, Vec3::ZERO, 6.0).transform();
        let cell = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(1024.0, 1024.0));
        (bvh, surface, view, cell, 1e-3)
    }

    #[test]
    fn box_occludes_far_point_and_middle_third_of_far_edge() {
        // Closed form by hand: occluder box [-1, 1]^3, flat view looking along +x
        // (forward = (1, 0, 0)). Point at (-2, 0, 0) is in front (visible), point at
        // (2, 0, 0) is behind (hidden), point on near face (-1, 0, 0) is visible by bias.
        // Edge from (2, -3, 0) to (2, 3, 0) has length 6 and is behind [-1, 1] for
        // y in [-1, 1], which is t in [1/3, 2/3].
        let (bvh, surface, view, cell, bias) = box_visibility_fixture();
        let vis = net_visibility(&surface, &bvh, &view, cell, bias);
        assert_eq!(vis.points, vec![true, false, true, true, true]);
        let spans = &vis.hidden_spans[0];
        assert_eq!(spans.len(), 1);
        let step = 1.0 / 64.0;
        assert!((spans[0].0 - 1.0 / 3.0).abs() <= step && (spans[0].1 - 2.0 / 3.0).abs() <= step);

        let empty = net_visibility(&surface, &Bvh::build(&[], &[]), &view, cell, bias);
        assert!(empty.points.iter().all(|&v| v));
        assert!(empty.hidden_spans.iter().all(|s| s.is_empty()));
    }

    fn run_frame(ctx: &egui::Context, app: &mut App) {
        let input = egui::RawInput {
            screen_rect: Some(egui::Rect::from_min_size(
                egui::Pos2::ZERO,
                egui::vec2(1600.0, 1000.0),
            )),
            ..Default::default()
        };
        let mut out = ctx.run_ui(input, |ui| app.draw(ui));
        out.textures_delta.clear();
    }

    #[test]
    fn profile_caches_visibility_hides_port_twin_and_recomputes_after_move() {
        // Closed form by hand: on the full demo hull at midship station x = 5.20,
        // the starboard bilge point has y > 0 (on the near side of the starboard
        // camera at +y) and its port twin has y < 0 (behind the starboard hull).
        let ctx = egui::Context::default();
        let mut empty_app = App::headless_empty();
        run_frame(&ctx, &mut empty_app);
        for v in 0..4 {
            assert!(empty_app.net_visibility(v).is_none());
        }

        let mut app = App::headless_for_test(None);
        run_frame(&ctx, &mut app);
        let surface = app.surface().unwrap();
        let (sb_idx, sb_pt) = surface
            .points
            .iter()
            .enumerate()
            .find(|(_, p)| (p.p.x - 5.20).abs() < 1e-6 && p.p.y > 1.0 && p.p.z < 0.8)
            .map(|(i, p)| (i, p.p))
            .expect("starboard midship bilge point");
        let pt_idx = surface
            .points
            .iter()
            .enumerate()
            .find(|(_, p)| {
                (p.p.x - sb_pt.x).abs() < 1e-6
                    && (p.p.y + sb_pt.y).abs() < 1e-6
                    && (p.p.z - sb_pt.z).abs() < 1e-6
            })
            .map(|(i, _)| i)
            .expect("port midship bilge twin");

        let vis = app
            .net_visibility(1)
            .expect("Profile visibility cached when shaded");
        assert!(
            vis.points[sb_idx],
            "starboard point must be visible in Profile"
        );
        assert!(!vis.points[pt_idx], "port twin must be hidden in Profile");

        // Move the port point across to y = +3.0 (in front of the starboard side):
        // the key changes and the recomputed visibility marks it visible.
        app.move_point(pt_idx, Vec3::new(sb_pt.x, 3.0, sb_pt.z))
            .unwrap();
        assert!(
            app.net_visibility(1).is_none(),
            "stale visibility must not be returned before the next frame"
        );
        run_frame(&ctx, &mut app);
        let vis_after = app
            .net_visibility(1)
            .expect("Profile visibility recomputed after move");
        assert!(
            vis_after.points[pt_idx],
            "moved point at y = +3.0 must now be visible in Profile"
        );

        // With shading off nothing is hidden, so nothing is classified.
        app.display_mut().shaded = false;
        run_frame(&ctx, &mut app);
        assert!(app.net_visibility(1).is_none());
    }

    #[test]
    fn ray_index_is_built_only_when_a_view_classifies_the_hidden_net() {
        // The index costs about half a refinement on the largest example
        // (final review of feat/hull-editing), so it is built only when the
        // shaded hull hides some of the net and the net or its points are shown.
        let ctx = egui::Context::default();
        let mut app = App::headless_for_test(None);
        app.display_mut().shaded = false;
        run_frame(&ctx, &mut app);
        assert!(!app.ray_index_built(), "shading off: no index");
        assert!(app.net_visibility(1).is_none());

        app.display_mut().shaded = true;
        app.display_mut().hidden_net = crate::render::HiddenNet::Same;
        run_frame(&ctx, &mut app);
        assert!(!app.ray_index_built(), "hidden net drawn as is: no index");

        app.display_mut().hidden_net = crate::render::HiddenNet::Dimmed;
        app.display_mut().control_net = false;
        app.display_mut().points = false;
        run_frame(&ctx, &mut app);
        assert!(!app.ray_index_built(), "net and points hidden: no index");

        app.display_mut().control_net = true;
        run_frame(&ctx, &mut app);
        assert!(app.ray_index_built(), "dimmed net shown on a shaded hull");
        assert!(app.net_visibility(1).is_some());

        // A new mesh drops the index; it comes back only when wanted again.
        app.display_mut().hidden_net = crate::render::HiddenNet::Same;
        let p = app.surface().unwrap().points[0].p;
        app.move_point(0, p + Vec3::new(0.0, 0.0, 0.01)).unwrap();
        run_frame(&ctx, &mut app);
        assert!(!app.ray_index_built(), "new mesh, hidden net drawn as is");
    }

    #[test]
    #[ignore]
    fn demo_hull_visibility_timing() {
        let cell = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(900.0, 600.0));
        let demo_path = format!(
            "{}/../../examples/demo-hull.fef",
            env!("CARGO_MANIFEST_DIR")
        );
        let demo_text = std::fs::read_to_string(&demo_path).expect("read demo-hull.fef");
        let demo_surface = seacraft_core::fef::parse(&demo_text).expect("parse demo-hull.fef");
        let (container_surface, _) =
            seacraft_core::examples::build(seacraft_core::examples::Example::Container230m)
                .expect("build container-230m");

        for (label, surface) in [
            ("demo-hull.fef", &demo_surface),
            ("container-230m", &container_surface),
        ] {
            let mesh = seacraft_core::export::render_mesh(
                surface,
                3,
                seacraft_core::SubdivisionMode::Loop,
            );
            let bvh = Bvh::of_render_mesh(&mesh);
            let (min, max) = crate::render::display_extents(surface);
            let centre = (min + max) * 0.5;
            let size = max - min;
            for kind in ViewKind::ALL {
                let mut cam = Camera::new(kind, centre, size);
                cam.frame(min, max);
                let view = cam.transform();
                let mut samples = [0.0_f64; 5];
                for s in &mut samples {
                    let t0 = std::time::Instant::now();
                    let v = std::hint::black_box(net_visibility(surface, &bvh, &view, cell, 0.0));
                    std::hint::black_box(v);
                    *s = t0.elapsed().as_secs_f64() * 1000.0;
                }
                samples.sort_by(|a, b| a.total_cmp(b));
                let median = samples[2];
                println!(
                    "{label} level 3 {:?}: median {:.3} ms (runs: {:.3?})",
                    kind, median, samples
                );
            }
        }
    }
}
