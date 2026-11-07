//! The hull on a real GPU: what `GpuHullResources` renders and composites, read back from the
//! texture. Every expected colour comes from `render::face_shade` and `theme::shaded`, the painted
//! path's own functions, and every expected place from `ViewTransform::project`.
//!
//! These need an adapter: `cargo test -p seacraft-gui --test gpu_hull -- --ignored --nocapture`.
//! A test with no adapter prints `skipped: no adapter` and returns. CI has no GPU.

use std::sync::OnceLock;

use eframe::egui::{pos2, Color32, Pos2, Rect};
use eframe::wgpu;
use seacraft_core::export::{self, RenderMesh};
use seacraft_core::math::Vec3;
use seacraft_core::mesh::{Layer, Surface};
use seacraft_gui::camera::{Camera, ViewKind, ViewTransform};
use seacraft_gui::gpu_hull::geometry::{build_geometry, chunk_ranges, HullGeometry, HullVertex};
use seacraft_gui::gpu_hull::pipeline::GpuHullResources;
use seacraft_gui::gpu_hull::view::{view_uniform, ViewUniform};
use seacraft_gui::{render, theme};

const COLOUR_A: i32 = 0x80c040;
const COLOUR_B: i32 = 0x4060e0;
const SIZE: [u32; 2] = [400, 300];
const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// What `paint` draws over: a colour no hull in these tests has.
const BACKGROUND: [u8; 4] = [0, 255, 0, 255];

struct Gpu {
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// One device for the whole binary, as the app has one. Tests that each created their own instance
/// hung in the driver when run in parallel with no `DISPLAY`.
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    let gpu = GPU.get_or_init(create_gpu).as_ref();
    if gpu.is_none() {
        println!("skipped: no adapter");
    }
    gpu
}

/// eframe's defaults: no feature and no limit beyond them.
fn create_gpu() -> Option<Gpu> {
    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
    let options = wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        ..Default::default()
    };
    let adapter = pollster::block_on(instance.request_adapter(&options)).ok()?;
    println!("adapter: {}", adapter.get_info().name);
    let (device, queue) =
        pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor::default()))
            .expect("a device with the default limits");
    Some(Gpu {
        adapter,
        device,
        queue,
    })
}

impl Gpu {
    fn resources(&self) -> GpuHullResources {
        GpuHullResources::new(&self.device, &self.adapter, FORMAT)
    }

    fn upload(&self, res: &mut GpuHullResources, geometry: &HullGeometry) {
        res.upload(&self.device, &self.queue, geometry)
            .expect("the upload succeeds");
    }

    /// One `render_view`, submitted.
    fn render_at(
        &self,
        res: &mut GpuHullResources,
        index: usize,
        uniform: &ViewUniform,
        (size, view_key): ([u32; 2], u64),
        (shaded, wire): (bool, bool),
    ) -> Result<(), String> {
        let mut encoder = self.device.create_command_encoder(&Default::default());
        let result = res.render_view(
            &self.device,
            &self.queue,
            &mut encoder,
            index,
            uniform,
            size,
            view_key,
            shaded,
            wire,
        );
        self.queue.submit([encoder.finish()]);
        result
    }

    fn render(&self, res: &mut GpuHullResources, uniform: &ViewUniform, view_key: u64) {
        self.render_at(res, 0, uniform, (SIZE, view_key), (true, false))
            .expect("the render succeeds");
    }

    fn read_view(&self, res: &GpuHullResources, index: usize) -> Image {
        self.read(res.resolved_texture(index).expect("a current texture"))
    }

    /// `upload`, render and read back, in one view.
    fn draw(&self, geometry: &HullGeometry, view: &ViewTransform) -> Image {
        let mut res = self.resources();
        self.upload(&mut res, geometry);
        self.render(&mut res, &uniform(view, geometry), 1);
        self.read_view(&res, 0)
    }

    fn read(&self, texture: &wgpu::Texture) -> Image {
        let (w, h) = (texture.width(), texture.height());
        let padded = (4 * w).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: u64::from(padded * h),
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self.device.create_command_encoder(&Default::default());
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(h),
                },
            },
            texture.size(),
        );
        self.queue.submit([encoder.finish()]);
        let slice = buffer.slice(..);
        slice.map_async(wgpu::MapMode::Read, |r| r.expect("the buffer maps"));
        self.device
            .poll(wgpu::PollType::wait_indefinitely())
            .expect("the GPU finishes");
        let mapped = slice.get_mapped_range().expect("the buffer is mapped");
        let mut rgba = Vec::with_capacity((4 * w * h) as usize);
        for row in mapped.chunks(padded as usize) {
            rgba.extend_from_slice(&row[..(4 * w) as usize]);
        }
        Image { w, h, rgba }
    }

    /// `paint` view `index` over [`BACKGROUND`], as egui's callback does, and read the target.
    fn paint(&self, res: &GpuHullResources, index: usize, format: wgpu::TextureFormat) -> Image {
        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("window stand-in"),
            size: wgpu::Extent3d {
                width: SIZE[0],
                height: SIZE[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&Default::default());
        let mut encoder = self.device.create_command_encoder(&Default::default());
        {
            let mut pass = encoder
                .begin_render_pass(&wgpu::RenderPassDescriptor {
                    label: Some("window stand-in"),
                    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                        view: &view,
                        depth_slice: None,
                        resolve_target: None,
                        ops: wgpu::Operations {
                            load: wgpu::LoadOp::Clear(wgpu::Color {
                                r: 0.0,
                                g: 1.0,
                                b: 0.0,
                                a: 1.0,
                            }),
                            store: wgpu::StoreOp::Store,
                        },
                    })],
                    depth_stencil_attachment: None,
                    timestamp_writes: None,
                    occlusion_query_set: None,
                    multiview_mask: None,
                })
                .forget_lifetime();
            res.paint(&mut pass, index);
        }
        self.queue.submit([encoder.finish()]);
        self.read(&target)
    }
}

struct Image {
    w: u32,
    h: u32,
    rgba: Vec<u8>,
}

impl Image {
    fn at(&self, x: u32, y: u32) -> [u8; 4] {
        assert!(x < self.w && y < self.h, "({x}, {y}) is off the image");
        let i = (4 * (y * self.w + x)) as usize;
        self.rgba[i..i + 4].try_into().unwrap()
    }

    /// The pixel holding the point `p`, in the pixels of a rect at the origin.
    fn at_point(&self, p: Pos2) -> [u8; 4] {
        self.at(p.x as u32, p.y as u32)
    }

    fn pixels(&self) -> impl Iterator<Item = [u8; 4]> + '_ {
        self.rgba.chunks(4).map(|c| c.try_into().unwrap())
    }

    /// Whether a pixel within 1 px of `p` passes `test`: where a one-pixel line lands.
    fn near(&self, p: Pos2, test: impl Fn([u8; 4]) -> bool) -> bool {
        (-1..=1).any(|dy| {
            (-1..=1).any(|dx| test(self.at((p.x as i32 + dx) as u32, (p.y as i32 + dy) as u32)))
        })
    }
}

#[track_caller]
fn assert_colour(got: [u8; 4], want: Color32, what: &str) {
    let close = |a: u8, b: u8| a.abs_diff(b) <= 2;
    assert!(
        close(got[0], want.r()) && close(got[1], want.g()) && close(got[2], want.b()),
        "{what}: got {got:?}, want {want:?}"
    );
    assert_eq!(got[3], 255, "{what}: not opaque, {got:?}");
}

fn rect() -> Rect {
    Rect::from_min_max(pos2(0.0, 0.0), pos2(SIZE[0] as f32, SIZE[1] as f32))
}

fn surface(colours: &[i32]) -> Surface {
    let layer = |&color: &i32| Layer {
        name: "l".into(),
        id: 0,
        color,
        visible: true,
        developable: false,
        symmetric: false,
        use_for_intersections: true,
        use_in_hydrostatics: true,
        show_in_linesplan: true,
        density: 0.0,
        thickness: 0.0,
    };
    Surface {
        layers: colours.iter().map(layer).collect(),
        ..Surface::default()
    }
}

/// A closed box with outward normals, on layer 0.
fn box_mesh(min: Vec3, max: Vec3) -> RenderMesh {
    let verts = (0..8)
        .map(|i| {
            Vec3::new(
                if i & 1 == 0 { min.x } else { max.x },
                if i & 2 == 0 { min.y } else { max.y },
                if i & 4 == 0 { min.z } else { max.z },
            )
        })
        .collect();
    let faces = vec![
        vec![0, 4, 6, 2],
        vec![1, 3, 7, 5],
        vec![0, 1, 5, 4],
        vec![2, 6, 7, 3],
        vec![0, 2, 3, 1],
        vec![4, 5, 7, 6],
    ];
    RenderMesh {
        verts,
        faces,
        face_layer: vec![0; 6],
    }
}

fn unit_cube() -> (Vec3, Vec3) {
    (Vec3::new(-0.5, -0.5, -0.5), Vec3::new(0.5, 0.5, 0.5))
}

/// A camera of `kind` framing `min..max` in [`rect`].
fn framed(kind: ViewKind, min: Vec3, max: Vec3) -> ViewTransform {
    let mut camera = Camera::new(kind, (min + max) * 0.5, max - min);
    camera.frame_aspect(min, max, Some(f64::from(SIZE[0]) / f64::from(SIZE[1])));
    camera.transform()
}

fn uniform(view: &ViewTransform, g: &HullGeometry) -> ViewUniform {
    view_uniform(view, rect(), 1.0, g.origin, g.min, g.max, theme::WIREFRAME)
}

/// What the painted path draws for a face with `normal` at `point`.
fn painted(view: &ViewTransform, base: i32, normal: Vec3, point: Vec3) -> Color32 {
    let to_eye = if view.perspective {
        (view.eye - point).norm()
    } else {
        -view.forward
    };
    theme::shaded(
        theme::layer_colour(base),
        render::face_shade(normal, render::key_light(view), to_eye),
    )
}

fn project(view: &ViewTransform, p: Vec3) -> Pos2 {
    view.project(p, rect()).expect("in front of the camera")
}

/// A unit cube and a perspective view of it, for tests that only need a hull.
fn cube_geometry(key: u64, wire: bool) -> (HullGeometry, ViewTransform) {
    let (min, max) = unit_cube();
    let g = build_geometry(&box_mesh(min, max), &surface(&[COLOUR_A]), key, wire);
    (g, framed(ViewKind::Perspective, min, max))
}

/// A quad in the plane `y`, spanning `x` and `z`, with its normal along +y.
fn quad(verts: &mut Vec<Vec3>, y: f64, half: f64) -> Vec<usize> {
    let n = verts.len();
    verts.extend([
        Vec3::new(-half, y, -half),
        Vec3::new(-half, y, half),
        Vec3::new(half, y, half),
        Vec3::new(half, y, -half),
    ]);
    vec![n, n + 1, n + 2, n + 3]
}

/// The screen's convex hull of `points`, in order, without collinear points.
fn convex_hull(mut points: Vec<Pos2>) -> Vec<Pos2> {
    points.sort_by(|a, b| a.x.total_cmp(&b.x).then(a.y.total_cmp(&b.y)));
    points.dedup();
    let cross = |o: Pos2, a: Pos2, b: Pos2| (a - o).x * (b - o).y - (a - o).y * (b - o).x;
    let half = |points: &mut dyn Iterator<Item = Pos2>| {
        let mut chain: Vec<Pos2> = Vec::new();
        for p in points {
            while chain.len() >= 2
                && cross(chain[chain.len() - 2], chain[chain.len() - 1], p) <= 0.0
            {
                chain.pop();
            }
            chain.push(p);
        }
        chain.pop();
        chain
    };
    let mut hull = half(&mut points.iter().copied());
    hull.extend(half(&mut points.iter().rev().copied()));
    hull
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_cube_face_has_the_painted_paths_colour() {
    let Some(gpu) = gpu() else { return };
    let (min, max) = unit_cube();
    let mesh = box_mesh(min, max);
    let g = build_geometry(&mesh, &surface(&[COLOUR_A]), 1, false);
    for kind in [ViewKind::Profile, ViewKind::Perspective] {
        let view = framed(kind, min, max);
        let image = gpu.draw(&g, &view);
        let mut seen = 0;
        for face in &mesh.faces {
            let poly: Vec<Vec3> = face.iter().map(|&i| mesh.verts[i]).collect();
            let normal = export::newell_normal(&poly).norm();
            let centre = poly.iter().fold(Vec3::ZERO, |a, &b| a + b) * 0.25;
            let to_eye = if view.perspective {
                (view.eye - centre).norm()
            } else {
                -view.forward
            };
            // Only a face turned well towards the eye has a centre clear of its edges.
            if normal.dot(to_eye) < 0.2 {
                continue;
            }
            seen += 1;
            assert_colour(
                image.at_point(project(&view, centre)),
                painted(&view, COLOUR_A, normal, centre),
                &format!("{kind:?}, face with normal {normal:?}"),
            );
        }
        assert!(seen >= 1, "{kind:?}: no visible face was checked");
    }
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn around_the_hull_is_transparent() {
    let Some(gpu) = gpu() else { return };
    let (min, max) = unit_cube();
    let g = build_geometry(&box_mesh(min, max), &surface(&[COLOUR_A]), 1, false);
    let image = gpu.draw(&g, &framed(ViewKind::Profile, min, max));
    let (r, b) = (SIZE[0] - 1, SIZE[1] - 1);
    for (x, y) in [(0, 0), (r, 0), (0, b), (r, b)] {
        assert_eq!(image.at(x, y)[3], 0, "corner ({x}, {y})");
    }
    assert!(image.pixels().any(|p| p[3] == 255), "no hull was drawn");
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn the_silhouette_is_where_project_says() {
    let Some(gpu) = gpu() else { return };
    // Off the camera's target on every axis, so a mirrored or shifted view cannot pass.
    let (min, max) = (Vec3::new(-6.0, 0.5, -1.0), Vec3::new(4.0, 5.0, 2.5));
    let frame = (Vec3::new(-8.0, -8.0, -8.0), Vec3::new(8.0, 8.0, 8.0));
    let g = build_geometry(&box_mesh(min, max), &surface(&[COLOUR_A]), 1, false);
    for kind in ViewKind::ALL {
        let view = framed(kind, frame.0, frame.1);
        let image = gpu.draw(&g, &view);
        let corners = (0..8)
            .map(|i| {
                Vec3::new(
                    if i & 1 == 0 { min.x } else { max.x },
                    if i & 2 == 0 { min.y } else { max.y },
                    if i & 4 == 0 { min.z } else { max.z },
                )
            })
            .map(|c| project(&view, c))
            .collect();
        let hull = convex_hull(corners);
        let middle = hull.iter().fold(pos2(0.0, 0.0), |a, p| a + p.to_vec2()) / hull.len() as f32;
        let mut checked = 0;
        for (i, &a) in hull.iter().enumerate() {
            let b = hull[(i + 1) % hull.len()];
            if (b - a).length() < 20.0 {
                continue;
            }
            let along = (b - a).normalized();
            let mut out = along.rot90();
            if out.dot(a - middle) < 0.0 {
                out = -out;
            }
            for t in [0.25, 0.5, 0.75] {
                let p = a + (b - a) * t;
                let (inside, outside) =
                    (image.at_point(p - out * 3.0), image.at_point(p + out * 3.0));
                assert_eq!(inside[3], 255, "{kind:?}: 3 px inside {p:?}: {inside:?}");
                assert_eq!(outside[3], 0, "{kind:?}: 3 px outside {p:?}: {outside:?}");
                checked += 1;
            }
        }
        assert!(checked >= 12, "{kind:?}: only {checked} points checked");
    }
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn the_nearer_face_wins_whatever_the_order() {
    let Some(gpu) = gpu() else { return };
    let s = surface(&[COLOUR_A, COLOUR_B]);
    let normal = Vec3::new(0.0, 1.0, 0.0);
    for kind in [ViewKind::Profile, ViewKind::Perspective] {
        for layers in [[0usize, 1], [1, 0]] {
            let mut verts = Vec::new();
            let faces = vec![quad(&mut verts, -0.5, 3.0), quad(&mut verts, 0.5, 3.0)];
            let mesh = RenderMesh {
                verts,
                faces,
                face_layer: layers.to_vec(),
            };
            let (min, max) = (Vec3::new(-3.0, -0.5, -3.0), Vec3::new(3.0, 0.5, 3.0));
            let view = framed(kind, min, max);
            let image = gpu.draw(&build_geometry(&mesh, &s, 1, false), &view);
            let plane = [Vec3::new(0.0, -0.5, 0.0), Vec3::new(0.0, 0.5, 0.0)];
            let depth = |p: Vec3| (p - view.eye).dot(view.forward);
            let near = usize::from(depth(plane[1]) < depth(plane[0]));
            let colour = [COLOUR_A, COLOUR_B][layers[near]];
            assert_colour(
                image.at_point(project(&view, plane[near])),
                painted(&view, colour, normal, plane[near]),
                &format!("{kind:?}, layers {layers:?}"),
            );
        }
    }
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_new_mesh_replaces_the_old_one() {
    let Some(gpu) = gpu() else { return };
    let frame = (Vec3::new(-4.0, -4.0, -4.0), Vec3::new(4.0, 4.0, 4.0));
    let view = framed(ViewKind::Profile, frame.0, frame.1);
    let at = |x: f64| (Vec3::new(x - 0.5, -0.5, -0.5), Vec3::new(x + 0.5, 0.5, 0.5));
    let (a_min, a_max) = at(-2.0);
    let (b_min, b_max) = at(2.0);
    let a = build_geometry(&box_mesh(a_min, a_max), &surface(&[COLOUR_A]), 1, false);
    let b = build_geometry(&box_mesh(b_min, b_max), &surface(&[COLOUR_B]), 2, false);
    let (pa, pb) = (
        project(&view, (a_min + a_max) * 0.5),
        project(&view, (b_min + b_max) * 0.5),
    );

    let mut res = gpu.resources();
    gpu.upload(&mut res, &a);
    gpu.render(&mut res, &uniform(&view, &a), 10);
    let first = gpu.read_view(&res, 0);
    assert_eq!(first.at_point(pa)[3], 255);
    assert_eq!(first.at_point(pb)[3], 0);

    gpu.upload(&mut res, &b);
    gpu.render(&mut res, &uniform(&view, &b), 20);
    let second = gpu.read_view(&res, 0);
    assert_eq!(second.at_point(pa)[3], 0, "the old mesh is still there");
    let normal = (-view.forward).norm();
    assert_colour(
        second.at_point(pb),
        painted(&view, COLOUR_B, normal, Vec3::new(2.0, 0.0, 0.0)),
        "the new mesh",
    );

    let empty = build_geometry(
        &RenderMesh {
            verts: vec![],
            faces: vec![],
            face_layer: vec![],
        },
        &surface(&[]),
        3,
        true,
    );
    gpu.upload(&mut res, &empty);
    assert_eq!(
        res.buffer_counts(),
        (0, 0),
        "empty geometry keeps no buffers"
    );
    gpu.render_at(
        &mut res,
        0,
        &uniform(&view, &empty),
        (SIZE, 30),
        (true, true),
    )
    .expect("the render succeeds");
    let third = gpu.read_view(&res, 0);
    assert!(third.pixels().all(|p| p == [0, 0, 0, 0]), "not fully clear");
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_new_mesh_is_not_painted_until_it_is_rendered() {
    let Some(gpu) = gpu() else { return };
    let (a, view) = cube_geometry(1, false);
    let (b, _) = cube_geometry(2, false);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &a);
    gpu.render(&mut res, &uniform(&view, &a), 10);
    assert!(gpu.paint(&res, 0, FORMAT).pixels().any(|p| p != BACKGROUND));

    gpu.upload(&mut res, &b);
    assert!(res.resolved_texture(0).is_none(), "the texture is stale");
    assert!(gpu.paint(&res, 0, FORMAT).pixels().all(|p| p == BACKGROUND));

    // The new mesh renders even under the view key the old one held.
    gpu.render(&mut res, &uniform(&view, &b), 10);
    assert!(gpu.paint(&res, 0, FORMAT).pixels().any(|p| p != BACKGROUND));
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn an_unchanged_view_key_does_not_render_again() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    let (min, max) = unit_cube();
    let other = framed(ViewKind::Profile, min, max);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &g);
    gpu.render(&mut res, &uniform(&view, &g), 10);
    let first = gpu.read_view(&res, 0).rgba;

    gpu.upload(&mut res, &g);
    gpu.render(&mut res, &uniform(&other, &g), 10);
    assert!(gpu.read_view(&res, 0).rgba == first, "it rendered again");

    gpu.render(&mut res, &uniform(&other, &g), 11);
    assert!(
        gpu.read_view(&res, 0).rgba != first,
        "a new key must render"
    );
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_new_size_renders_again_under_the_same_key() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    let u = uniform(&view, &g);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &g);
    gpu.render(&mut res, &u, 10);
    gpu.render_at(&mut res, 0, &u, ([200, 150], 10), (true, false))
        .expect("the render succeeds");
    let image = gpu.read_view(&res, 0);
    assert_eq!((image.w, image.h), (200, 150));
    assert_eq!(
        image.at(100, 75)[3],
        255,
        "the new texture was not rendered"
    );
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_tall_mesh_splits_and_still_draws() {
    let Some(gpu) = gpu() else { return };
    // An 8 x 8 grid of quads, facing the Profile camera.
    let side = 8usize;
    let mut verts = Vec::new();
    let mut faces = Vec::new();
    for j in 0..side {
        for i in 0..side {
            let (x, z) = (i as f64, j as f64);
            let n = verts.len();
            verts.extend([
                Vec3::new(x, 0.0, z),
                Vec3::new(x + 1.0, 0.0, z),
                Vec3::new(x + 1.0, 0.0, z + 1.0),
                Vec3::new(x, 0.0, z + 1.0),
            ]);
            faces.push(vec![n, n + 1, n + 2, n + 3]);
        }
    }
    let face_layer = vec![0; faces.len()];
    let mesh = RenderMesh {
        verts,
        faces,
        face_layer,
    };
    let g = build_geometry(&mesh, &surface(&[COLOUR_A]), 1, true);
    assert_eq!(g.vertices.len(), 6 * side * side);
    let (min, max) = (g.min, g.max);
    let view = framed(ViewKind::Profile, min, max);

    // Nine triangles to a buffer.
    let cap = 9 * 3 * std::mem::size_of::<HullVertex>() as u64;
    let mut res = GpuHullResources::with_buffer_cap(&gpu.device, &gpu.adapter, FORMAT, cap);
    gpu.upload(&mut res, &g);
    let (triangles, lines) = res.buffer_counts();
    assert_eq!(triangles, chunk_ranges(g.vertices.len(), 28, 3, cap).len());
    assert_eq!(lines, chunk_ranges(g.edges.len(), 12, 2, cap).len());
    assert!(
        triangles > 1 && lines > 1,
        "{triangles} and {lines} buffers"
    );

    let point = |i: usize, j: usize, u: f64, v: f64| Vec3::new(i as f64 + u, 0.0, j as f64 + v);
    gpu.render_at(&mut res, 0, &uniform(&view, &g), (SIZE, 1), (true, false))
        .expect("the render succeeds");
    let solid = gpu.read_view(&res, 0);
    gpu.render_at(&mut res, 0, &uniform(&view, &g), (SIZE, 2), (false, true))
        .expect("the render succeeds");
    let wire = gpu.read_view(&res, 0);
    for j in 0..side {
        for i in 0..side {
            // Each side of the quad's fan diagonal, one triangle each.
            for (u, v) in [(0.75, 0.25), (0.25, 0.75)] {
                let p = project(&view, point(i, j, u, v));
                assert_eq!(solid.at_point(p)[3], 255, "triangle at {i}, {j}, {u}");
            }
            // The middle of each edge of the quad.
            for (u, v) in [(0.5, 0.0), (1.0, 0.5), (0.5, 1.0), (0.0, 0.5)] {
                let p = project(&view, point(i, j, u, v));
                assert!(wire.near(p, |c| c[3] > 0), "edge at {i}, {j}, {u}, {v}");
            }
        }
    }
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn the_wire_shows_through_the_hull() {
    let Some(gpu) = gpu() else { return };
    // A small far quad behind a large near one: its outline falls inside the near quad.
    let (min, max) = (Vec3::new(-3.0, -0.5, -3.0), Vec3::new(3.0, 0.5, 3.0));
    let view = framed(ViewKind::Profile, min, max);
    let (near_y, far_y) = if (view.forward).y > 0.0 {
        (-0.5, 0.5)
    } else {
        (0.5, -0.5)
    };
    let mut verts = Vec::new();
    let faces = vec![quad(&mut verts, near_y, 3.0), quad(&mut verts, far_y, 1.0)];
    let mesh = RenderMesh {
        verts,
        faces,
        face_layer: vec![0, 1],
    };
    let g = build_geometry(&mesh, &surface(&[COLOUR_A, COLOUR_B]), 1, true);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &g);
    let u = uniform(&view, &g);
    let hull = painted(
        &view,
        COLOUR_A,
        Vec3::new(0.0, 1.0, 0.0),
        Vec3::new(0.0, near_y, 0.0),
    );
    let is_hull = |c: [u8; 4]| {
        c[0].abs_diff(hull.r()) <= 2 && c[1].abs_diff(hull.g()) <= 2 && c[2].abs_diff(hull.b()) <= 2
    };
    let far_edge = project(&view, Vec3::new(1.0, far_y, 0.0));
    let open = project(&view, Vec3::new(0.0, near_y, 0.0));

    gpu.render_at(&mut res, 0, &u, (SIZE, 1), (true, false))
        .expect("the render succeeds");
    let shaded = gpu.read_view(&res, 0);
    assert!(
        !shaded.near(far_edge, |c| !is_hull(c)),
        "no wire was asked for"
    );

    gpu.render_at(&mut res, 0, &u, (SIZE, 2), (true, true))
        .expect("the render succeeds");
    let both = gpu.read_view(&res, 0);
    assert!(
        both.near(far_edge, |c| !is_hull(c)),
        "the far edge is hidden"
    );
    assert!(is_hull(both.at_point(open)), "away from the wire");

    gpu.render_at(&mut res, 0, &u, (SIZE, 3), (false, true))
        .expect("the render succeeds");
    let wire = gpu.read_view(&res, 0);
    assert!(wire.near(far_edge, |c| c[3] > 0), "no line");
    assert_eq!(wire.at_point(open)[3], 0, "the inside of a wire is clear");
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn the_composite_shows_the_texture() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    // A gamma target and an sRGB one read back the same bytes: `fs_linear` undoes what the target
    // encodes.
    for format in [
        wgpu::TextureFormat::Rgba8Unorm,
        wgpu::TextureFormat::Rgba8UnormSrgb,
    ] {
        let mut res = GpuHullResources::new(&gpu.device, &gpu.adapter, format);
        gpu.upload(&mut res, &g);
        gpu.render(&mut res, &uniform(&view, &g), 1);
        let texture = gpu.read_view(&res, 0);
        let painted = gpu.paint(&res, 0, format);
        let (mut hull, mut around) = (0, 0);
        for (t, p) in texture.pixels().zip(painted.pixels()) {
            match t[3] {
                255 => {
                    hull += 1;
                    let close = (0..3).all(|c| t[c].abs_diff(p[c]) <= 2);
                    assert!(close && p[3] == 255, "{format:?}: {t:?} became {p:?}");
                }
                0 => {
                    around += 1;
                    assert_eq!(p, BACKGROUND, "{format:?}: a clear pixel covered");
                }
                _ => {} // an edge, covered in part
            }
        }
        assert!(
            hull > 1000 && around > 1000,
            "{format:?}: {hull} and {around}"
        );
    }
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn the_composite_blends_a_partly_covered_pixel() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &g);
    gpu.render(&mut res, &uniform(&view, &g), 1);
    let texture = gpu.read_view(&res, 0);
    let painted = gpu.paint(&res, 0, FORMAT);
    let mut seen = 0;
    for (t, p) in texture.pixels().zip(painted.pixels()) {
        if t[3] > 0 && t[3] < 255 {
            seen += 1;
            // Premultiplied over the background's green.
            let green = u16::from(t[1]) + (255 - u16::from(t[3]));
            assert!(u16::from(p[1]).abs_diff(green) <= 2, "{t:?} became {p:?}");
            assert!(p[0].abs_diff(t[0]) <= 2 && p[2].abs_diff(t[2]) <= 2);
        }
    }
    // Only a multisampled texture has such pixels.
    let flags = gpu.adapter.get_texture_format_features(FORMAT).flags;
    let multisampled = flags.contains(wgpu::TextureFormatFeatureFlags::MULTISAMPLE_X4);
    assert!(
        seen > 0 || !multisampled,
        "no edge pixel was partly covered"
    );
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_zero_sized_view_allocates_nothing_and_paints_nothing() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    let u = uniform(&view, &g);
    let mut res = gpu.resources();
    assert!(gpu.paint(&res, 0, FORMAT).pixels().all(|p| p == BACKGROUND));
    gpu.upload(&mut res, &g);
    for size in [[0, 10], [10, 0], [0, 0]] {
        gpu.render_at(&mut res, 0, &u, (size, 10), (true, false))
            .expect("a zero-sized view is not an error");
        assert!(res.resolved_texture(0).is_none(), "{size:?}");
        assert!(gpu.paint(&res, 0, FORMAT).pixels().all(|p| p == BACKGROUND));
    }
    // A view that goes to zero and comes back renders, under the same key.
    gpu.render(&mut res, &u, 10);
    assert!(gpu.read_view(&res, 0).pixels().any(|p| p[3] == 255));
    gpu.render_at(&mut res, 0, &u, ([0, 0], 10), (true, false))
        .expect("a zero-sized view is not an error");
    assert!(gpu.paint(&res, 0, FORMAT).pixels().all(|p| p == BACKGROUND));
    gpu.render(&mut res, &u, 10);
    assert!(gpu.read_view(&res, 0).pixels().any(|p| p[3] == 255));
}

#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_view_out_of_range_is_an_error_and_paints_nothing() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &g);
    assert!(gpu
        .render_at(&mut res, 4, &uniform(&view, &g), (SIZE, 1), (true, false))
        .is_err());
    assert!(res.resolved_texture(4).is_none());
    assert!(gpu.paint(&res, 4, FORMAT).pixels().all(|p| p == BACKGROUND));
    assert_eq!(res.buffer_counts().0, 1, "a wrong index frees nothing");
}

/// The real failure a large window meets: a texture the device refuses, without an out-of-memory.
/// Without the error scope this panics in wgpu's default handler.
#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_failed_texture_forgets_everything_and_repeats_its_error() {
    let Some(gpu) = gpu() else { return };
    let (g, view) = cube_geometry(1, false);
    let u = uniform(&view, &g);
    let mut res = gpu.resources();
    gpu.upload(&mut res, &g);
    gpu.render_at(&mut res, 0, &u, (SIZE, 10), (true, false))
        .unwrap();
    gpu.render_at(&mut res, 1, &u, (SIZE, 10), (true, false))
        .unwrap();
    assert!(gpu.paint(&res, 1, FORMAT).pixels().any(|p| p != BACKGROUND));

    let too_wide = [gpu.device.limits().max_texture_dimension_2d + 1, 16];
    let error = gpu
        .render_at(&mut res, 0, &u, (too_wide, 11), (true, false))
        .expect_err("the device refuses this texture");

    // Nothing is left to draw from, in any view.
    assert_eq!(res.buffer_counts(), (0, 0));
    for index in 0..2 {
        assert!(res.resolved_texture(index).is_none(), "view {index}");
        assert!(gpu
            .paint(&res, index, FORMAT)
            .pixels()
            .all(|p| p == BACKGROUND));
    }

    // The same geometry would fit, so only the remembered failure can refuse it.
    assert_eq!(res.upload(&gpu.device, &gpu.queue, &g), Err(error.clone()));
    assert_eq!(res.buffer_counts(), (0, 0));
    assert_eq!(
        gpu.render_at(&mut res, 1, &u, (SIZE, 10), (true, false)),
        Err(error),
        "a later view of the frame fails the same way"
    );
    assert!(res.resolved_texture(1).is_none());

    // A new mesh starts again.
    let (next, _) = cube_geometry(2, false);
    gpu.upload(&mut res, &next);
    gpu.render(&mut res, &uniform(&view, &next), 12);
    assert!(gpu.paint(&res, 0, FORMAT).pixels().any(|p| p != BACKGROUND));
}

/// A cap below one triangle stands in for a buffer that cannot be had. Lines still fit, so the
/// resources hold something drawn before the failure.
#[test]
#[ignore = "needs a GPU: run with --ignored"]
fn a_failed_upload_forgets_everything_and_repeats_its_error() {
    let Some(gpu) = gpu() else { return };
    let (cube, view) = cube_geometry(2, true);
    let outline = HullGeometry {
        key: 1,
        origin: cube.origin,
        min: cube.min,
        max: cube.max,
        vertices: Vec::new(),
        edges: cube.edges.clone(),
    };
    let cap = 2 * 12;
    let mut res = GpuHullResources::with_buffer_cap(&gpu.device, &gpu.adapter, FORMAT, cap);
    gpu.upload(&mut res, &outline);
    let u = uniform(&view, &outline);
    gpu.render_at(&mut res, 0, &u, (SIZE, 10), (false, true))
        .unwrap();
    assert!(gpu.paint(&res, 0, FORMAT).pixels().any(|p| p != BACKGROUND));

    let error = res
        .upload(&gpu.device, &gpu.queue, &cube)
        .expect_err("a triangle does not fit the cap");
    assert_eq!(res.buffer_counts(), (0, 0), "nothing half uploaded");
    assert!(res.resolved_texture(0).is_none());
    assert!(gpu.paint(&res, 0, FORMAT).pixels().all(|p| p == BACKGROUND));
    assert_eq!(
        res.upload(&gpu.device, &gpu.queue, &cube),
        Err(error.clone())
    );
    assert_eq!(
        gpu.render_at(&mut res, 0, &u, (SIZE, 11), (false, true)),
        Err(error)
    );

    // Geometry that fits starts again, and the failure is forgotten with it.
    gpu.upload(&mut res, &outline);
    gpu.render_at(&mut res, 0, &u, (SIZE, 12), (false, true))
        .unwrap();
    assert!(gpu.paint(&res, 0, FORMAT).pixels().any(|p| p != BACKGROUND));
}
