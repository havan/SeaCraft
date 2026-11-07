//! Which path draws each viewport's hull, headless: with routing on, a view emits one paint callback
//! where the painted hull used to be, the geometry follows the mesh within the frame, and whatever
//! cannot go through the GPU takes the old path or draws nothing.
//!
//! The last tests need an adapter and drive the callback through egui's renderer:
//! `cargo test -p seacraft-gui --test gpu_hull_routing -- --ignored --nocapture`. With no adapter
//! they print `skipped: no adapter` and return. CI has no GPU.

use std::sync::{Arc, OnceLock};

use eframe::egui::{self, pos2, vec2, Rect};
use eframe::egui_wgpu::{self, CallbackTrait, ScreenDescriptor};
use eframe::wgpu;
use seacraft_core::AnalysisMode;
use seacraft_gui::commands::{self, LayerProps, SetLayerParams};
use seacraft_gui::gpu_hull::geometry::{build_geometry, HullGeometry};
use seacraft_gui::gpu_hull::pipeline::GpuHullResources;
use seacraft_gui::gpu_hull::view::target_size;
use seacraft_gui::gpu_hull::{FailureSlot, HullCallback};
use seacraft_gui::messages::Level;
use seacraft_gui::{theme, App};

const SCREEN: egui::Vec2 = vec2(1600.0, 1000.0);

fn input(screen: egui::Vec2, events: Vec<egui::Event>) -> egui::RawInput {
    egui::RawInput {
        screen_rect: Some(Rect::from_min_size(pos2(0.0, 0.0), screen)),
        events,
        ..Default::default()
    }
}

fn frame_sized(
    ctx: &egui::Context,
    app: &mut App,
    screen: egui::Vec2,
    events: Vec<egui::Event>,
) -> egui::FullOutput {
    let mut out = ctx.run_ui(input(screen, events), |ui| app.draw(ui));
    // Nothing uploads the fonts in a headless pass; a dropped delta panics.
    out.textures_delta.clear();
    out
}

fn frame(ctx: &egui::Context, app: &mut App) -> egui::FullOutput {
    frame_sized(ctx, app, SCREEN, Vec::new())
}

fn frame_with(ctx: &egui::Context, app: &mut App, events: Vec<egui::Event>) -> egui::FullOutput {
    frame_sized(ctx, app, SCREEN, events)
}

fn demo_path() -> String {
    format!(
        "{}/../../examples/demo-hull.fef",
        env!("CARGO_MANIFEST_DIR")
    )
}

/// The demo hull at a cheap level, drawn until the docks settle, with hydrostatics out of the way.
fn settled(routing: bool) -> (App, egui::Context) {
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    app.set_gpu_hull_routing(routing);
    let ctx = egui::Context::default();
    for _ in 0..3 {
        frame(&ctx, &mut app);
    }
    app.recompute_hydrostatics();
    frame(&ctx, &mut app);
    (app, ctx)
}

fn routed() -> (App, egui::Context) {
    settled(true)
}

fn walk(shape: &egui::Shape, visit: &mut impl FnMut(&egui::Shape)) {
    if let egui::Shape::Vec(shapes) = shape {
        shapes.iter().for_each(|s| walk(s, visit));
    } else {
        visit(shape);
    }
}

fn each_shape(out: &egui::FullOutput, mut visit: impl FnMut(&egui::Shape)) {
    for clipped in &out.shapes {
        walk(&clipped.shape, &mut visit);
    }
}

fn callbacks(out: &egui::FullOutput) -> usize {
    let mut count = 0;
    each_shape(out, |shape| {
        if matches!(shape, egui::Shape::Callback(_)) {
            count += 1;
        }
    });
    count
}

/// The most vertices in any one mesh of the frame. Only a painted hull has thousands.
fn largest_mesh(out: &egui::FullOutput) -> usize {
    let mut largest = 0;
    each_shape(out, |shape| {
        if let egui::Shape::Mesh(mesh) = shape {
            largest = largest.max(mesh.vertices.len());
        }
    });
    largest
}

fn assert_no_painted_hull(out: &egui::FullOutput) {
    let largest = largest_mesh(out);
    assert!(largest < 1000, "a painted hull of {largest} vertices");
}

fn assert_painted_hull(out: &egui::FullOutput) {
    let largest = largest_mesh(out);
    assert!(largest >= 1000, "no painted hull, largest mesh {largest}");
}

/// What the app holds now against a build from the mesh it is drawing.
fn assert_geometry_current(app: &App) {
    let held = app.gpu_hull_geometry().expect("a geometry");
    let fresh: HullGeometry = build_geometry(
        app.render_cache().expect("a mesh"),
        app.surface().expect("a model"),
        held.key,
        false,
    );
    assert_eq!(held.vertices.len(), fresh.vertices.len());
    assert!(held.vertices == fresh.vertices, "stale vertices");
    assert_eq!(held.origin, fresh.origin);
}

fn set_layer(app: &mut App, layer: usize, props: LayerProps) {
    let params = SetLayerParams {
        layer,
        props,
        reorder: None,
    };
    commands::set_layer(app, &params).expect("the layer changes");
}

fn hide_layer(app: &mut App, layer: usize) {
    let props = LayerProps {
        visible: Some(false),
        ..Default::default()
    };
    set_layer(app, layer, props);
}

fn button(pos: egui::Pos2, pressed: bool) -> egui::Event {
    egui::Event::PointerButton {
        pos,
        button: egui::PointerButton::Primary,
        pressed,
        modifiers: egui::Modifiers::default(),
    }
}

/// Press on a sheer point in the profile view and move past egui's drag threshold, leaving the drag
/// under way. Returns where the pointer is.
fn start_drag(app: &mut App, ctx: &egui::Context) -> egui::Pos2 {
    let point = {
        let surface = app.surface().expect("a model");
        let sheer = surface
            .points
            .iter()
            .filter(|p| p.p.x > 2.0 && p.p.x < 10.0)
            .max_by(|a, b| a.p.z.total_cmp(&b.p.z))
            .expect("a sheer point");
        sheer.p
    };
    let rect = app.viewport_rect(1);
    let screen = app.cameras()[1]
        .transform()
        .project(point, rect)
        .expect("point visible");
    frame_with(ctx, app, vec![egui::Event::PointerMoved(screen)]);
    frame_with(ctx, app, vec![button(screen, true)]);
    let moved = screen + vec2(12.0, 0.0);
    frame_with(ctx, app, vec![egui::Event::PointerMoved(moved)]);
    moved
}

#[test]
fn each_view_emits_one_callback_between_grid_and_overlays() {
    let (mut app, ctx) = routed();
    let out = frame(&ctx, &mut app);
    let mut order: Vec<&'static str> = Vec::new();
    each_shape(&out, |shape| match shape {
        egui::Shape::Callback(_) => order.push("callback"),
        egui::Shape::LineSegment { stroke, .. }
            if stroke.color == theme::GRID_MINOR || stroke.color == theme::GRID_MAJOR =>
        {
            order.push("grid")
        }
        egui::Shape::Circle(circle) if circle.fill == theme::POINT => order.push("point"),
        _ => {}
    });
    let at: Vec<usize> = (0..order.len())
        .filter(|&i| order[i] == "callback")
        .collect();
    assert_eq!(at.len(), 4, "one callback per view");
    let mut start = 0;
    for (view, &callback) in at.iter().enumerate() {
        let end = at.get(view + 1).copied().unwrap_or(order.len());
        let before = &order[start..callback];
        let after = &order[callback + 1..end];
        assert!(before.contains(&"grid"), "view {view}: grid under the hull");
        assert!(
            after.contains(&"point"),
            "view {view}: points over the hull"
        );
        // Each view's points are drawn before the next view's grid.
        let last_point = after.iter().rposition(|s| *s == "point").unwrap();
        let first_grid = after.iter().position(|s| *s == "grid");
        if view + 1 == at.len() {
            assert_eq!(first_grid, None, "the last view's grid is under its hull");
        } else {
            assert!(first_grid.is_some_and(|g| g > last_point));
        }
        start = callback + 1;
    }
    let pixels_per_point = out.pixels_per_point;
    let primitives = ctx.tessellate(out.shapes, pixels_per_point);
    let tessellated = primitives
        .iter()
        .filter(|p| matches!(p.primitive, egui::epaint::Primitive::Callback(_)))
        .count();
    assert_eq!(tessellated, 4);
}

#[test]
fn routed_views_build_no_painted_hull() {
    let (mut app, ctx) = routed();
    assert_no_painted_hull(&frame(&ctx, &mut app));
}

#[test]
fn without_routing_the_shapes_are_todays() {
    let (mut app, ctx) = settled(false);
    let out = frame(&ctx, &mut app);
    assert_eq!(callbacks(&out), 0);
    assert_painted_hull(&out);
    assert!(app.gpu_hull_geometry().is_none(), "nothing is built for it");
}

#[test]
fn orbiting_keeps_the_same_geometry() {
    let (mut app, ctx) = routed();
    let held = app.gpu_hull_geometry().expect("a geometry").clone();
    for _ in 0..3 {
        app.cameras_mut()[0].orbit(3.0, 1.0);
        let out = frame(&ctx, &mut app);
        assert_eq!(callbacks(&out), 4);
        let now = app.gpu_hull_geometry().expect("a geometry");
        assert!(Arc::ptr_eq(&held, now), "orbiting rebuilt the geometry");
    }
}

#[test]
fn a_new_level_gives_new_geometry_the_same_frame() {
    let (mut app, ctx) = routed();
    let before = app.gpu_hull_geometry().expect("a geometry").clone();
    app.set_subdivision_level(1);
    frame(&ctx, &mut app);
    assert_ne!(app.gpu_hull_geometry().unwrap().key, before.key);
    assert!(app.gpu_hull_geometry().unwrap().vertices.len() < before.vertices.len());
    assert_geometry_current(&app);
}

#[test]
fn a_layer_colour_change_gives_new_geometry() {
    let (mut app, ctx) = routed();
    let colour = 0x123456;
    let props = LayerProps {
        color: Some(colour),
        ..Default::default()
    };
    for layer in 0..app.layer_count() {
        set_layer(&mut app, layer, props.clone());
    }
    frame(&ctx, &mut app);
    let held = app.gpu_hull_geometry().expect("a geometry");
    assert!(held
        .vertices
        .iter()
        .all(|v| v.colour == [0x12, 0x34, 0x56, 255]));
}

#[test]
fn a_layer_colour_alone_changes_the_geometry_key() {
    // Written directly, so the revision does not move and only the key's colours can notice.
    let (mut app, ctx) = routed();
    let before = app.gpu_hull_geometry().expect("a geometry").clone();
    for layer in &mut app.surface_mut().expect("a model").layers {
        layer.color = 0x123456;
    }
    frame(&ctx, &mut app);
    let held = app.gpu_hull_geometry().expect("a geometry");
    assert_ne!(held.key, before.key);
    assert_geometry_current(&app);
}

#[test]
fn a_hidden_layer_and_an_undo_give_new_geometry() {
    let (mut app, ctx) = routed();
    let whole = app.gpu_hull_geometry().expect("a geometry").clone();

    // Hiding a layer moves the mesh's stamp, so the key changes with no visibility of its own.
    hide_layer(&mut app, 0);
    frame(&ctx, &mut app);
    let hidden = app.gpu_hull_geometry().expect("a geometry").clone();
    assert_ne!(hidden.key, whole.key);
    assert!(hidden.vertices.len() < whole.vertices.len(), "faces went");
    assert_geometry_current(&app);

    // Its undo brings them back.
    app.undo();
    frame(&ctx, &mut app);
    let back = app.gpu_hull_geometry().expect("a geometry").clone();
    assert_ne!(back.key, hidden.key);
    assert!(back.vertices == whole.vertices, "the whole hull again");
    assert_geometry_current(&app);

    // A drag, released, and its undo.
    let at = start_drag(&mut app, &ctx);
    frame_with(&ctx, &mut app, vec![button(at, false)]);
    frame(&ctx, &mut app);
    let dragged = app.gpu_hull_geometry().expect("a geometry").clone();
    assert_ne!(dragged.key, back.key);
    assert!(dragged.vertices != back.vertices, "the point moved");
    assert_geometry_current(&app);
    app.undo();
    frame(&ctx, &mut app);
    let undone = app.gpu_hull_geometry().expect("a geometry");
    assert_ne!(undone.key, dragged.key);
    assert!(undone.vertices == back.vertices, "the drag is undone");
    assert_geometry_current(&app);
}

#[test]
fn a_drag_frame_holds_the_mesh_it_moved_to() {
    // The dragged view refines in the middle of the frame. The geometry the frame's callbacks share
    // must be that mesh, or the views would upload two meshes in one frame and the first would blank.
    let (mut app, ctx) = routed();
    let at = start_drag(&mut app, &ctx);
    for dx in [4.0, 8.0, 12.0] {
        let moved = at + vec2(dx, 0.0);
        let before = app.gpu_hull_geometry().expect("a geometry").clone();
        let out = frame_with(&ctx, &mut app, vec![egui::Event::PointerMoved(moved)]);
        assert_eq!(callbacks(&out), 4);
        assert_ne!(app.gpu_hull_geometry().unwrap().key, before.key);
        assert_geometry_current(&app);
    }
}

#[test]
fn hiding_every_layer_emits_no_callback() {
    let (mut app, ctx) = routed();
    for layer in 0..app.layer_count() {
        hide_layer(&mut app, layer);
    }
    let out = frame(&ctx, &mut app);
    assert_eq!(callbacks(&out), 0);
    assert_no_painted_hull(&out);
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
    assert!(!primitives.is_empty());

    let mut empty = App::headless_empty();
    empty.set_gpu_hull_routing(true);
    let ctx = egui::Context::default();
    assert_eq!(callbacks(&frame(&ctx, &mut empty)), 0);
}

#[test]
fn a_mesh_of_only_degenerate_faces_draws_nothing() {
    let (mut app, ctx) = routed();
    app.display_mut().wireframe = true;
    let surface = app.surface_mut().expect("a model");
    let first = surface.points[0].p;
    surface.points.iter_mut().for_each(|p| p.p = first);
    app.mark_geometry_changed();
    let out = frame(&ctx, &mut app);
    assert_eq!(callbacks(&out), 0);
    assert_no_painted_hull(&out);
    let mut wire = 0;
    each_shape(&out, |shape| {
        if matches!(shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == theme::WIREFRAME)
        {
            wire += 1;
        }
    });
    assert_eq!(
        wire, 0,
        "the old wireframe would outline the collapsed faces"
    );
}

#[test]
fn an_analysis_mode_emits_no_callback() {
    let (mut app, ctx) = routed();
    app.set_analysis(AnalysisMode::Zebra { stripes: 14.0 });
    assert_eq!(callbacks(&frame(&ctx, &mut app)), 0);
    app.set_analysis(AnalysisMode::Shaded);
    assert_eq!(callbacks(&frame(&ctx, &mut app)), 4);
}

#[test]
fn shading_and_wireframe_off_emits_no_callback() {
    let (mut app, ctx) = routed();
    app.display_mut().shaded = false;
    app.display_mut().wireframe = false;
    let out = frame(&ctx, &mut app);
    assert_eq!(callbacks(&out), 0);
    assert_no_painted_hull(&out);
}

#[test]
fn wireframe_alone_emits_a_callback() {
    let (mut app, ctx) = routed();
    app.display_mut().shaded = false;
    app.display_mut().wireframe = true;
    let out = frame(&ctx, &mut app);
    assert_eq!(callbacks(&out), 4);
    assert!(!app.gpu_hull_geometry().unwrap().edges.is_empty());
    let mut wire = 0;
    each_shape(&out, |shape| {
        if matches!(shape, egui::Shape::LineSegment { stroke, .. } if stroke.color == theme::WIREFRAME)
        {
            wire += 1;
        }
    });
    assert_eq!(wire, 0, "the old wireframe does not draw too");
}

#[test]
fn a_tiny_window_emits_no_callback_for_an_empty_cell() {
    let (mut app, ctx) = routed();
    let out = frame_sized(&ctx, &mut app, vec2(1.0, 1.0), Vec::new());
    assert_eq!(callbacks(&out), 0);
    let primitives = ctx.tessellate(out.shapes, out.pixels_per_point);
    drop(primitives);
}

/// Every cell drawn in the frame: its canvas is filled over exactly its rectangle. `viewport_rect`
/// keeps last frame's when the central area is too small to draw in.
fn drawn_cells(out: &egui::FullOutput) -> Vec<Rect> {
    let mut cells = Vec::new();
    each_shape(out, |shape| {
        if let egui::Shape::Rect(rect) = shape {
            if rect.fill == theme::CANVAS {
                cells.push(rect.rect);
            }
        }
    });
    cells
}

#[test]
fn a_cell_under_a_pixel_wide_draws_no_hull() {
    let (mut app, ctx) = routed();
    // Search for the width that leaves the central area about ten points wide, which at this scale
    // is under a pixel for every cell.
    ctx.set_zoom_factor(0.02);
    frame(&ctx, &mut app);
    let found = (100..1600).find_map(|width| {
        let screen = vec2(width as f32, 1000.0);
        let out = frame_sized(&ctx, &mut app, screen, Vec::new());
        let ppp = ctx.pixels_per_point();
        let cells = drawn_cells(&out);
        let empty = cells.len() == 4 && cells.iter().all(|c| target_size(*c, ppp).is_none());
        empty.then_some(out)
    });
    let out = found.expect("a width that leaves every cell under a pixel");
    assert_eq!(callbacks(&out), 0);
    assert_no_painted_hull(&out);
}

#[test]
fn a_callback_viewport_has_the_size_of_its_texture_at_any_window_size() {
    // egui clamps a callback's viewport to the screen and `target_size` does not, so this holds only
    // while a cell stays on the screen: the panels take their room from the screen, the central area
    // has what is left, and it is drawn only when at least 8 points remain.
    let (mut app, ctx) = routed();
    let mut seen = 0;
    for zoom in [1.0, 1.25, 1.5] {
        ctx.set_zoom_factor(zoom);
        frame(&ctx, &mut app);
        for width in (100..1700).step_by(160) {
            for height in (100..1100).step_by(125) {
                let screen = vec2(width as f32, height as f32);
                let out = frame_sized(&ctx, &mut app, screen, Vec::new());
                let ppp = ctx.pixels_per_point();
                let screen_px = [
                    (screen.x * ppp).round() as u32,
                    (screen.y * ppp).round() as u32,
                ];
                each_shape(&out, |shape| {
                    let egui::Shape::Callback(callback) = shape else {
                        return;
                    };
                    seen += 1;
                    let viewport =
                        egui::epaint::ViewportInPixels::from_points(&callback.rect, ppp, screen_px);
                    assert_eq!(
                        target_size(callback.rect, ppp),
                        Some([viewport.width_px as u32, viewport.height_px as u32]),
                        "{:?} at {screen:?} and scale {ppp}",
                        callback.rect
                    );
                });
            }
        }
    }
    assert!(seen > 100, "only {seen} callbacks were checked");
}

#[test]
fn a_failed_mesh_falls_back_and_logs_once() {
    let (mut app, ctx) = routed();
    let warnings = app.messages().count(Level::Warning);
    let key = app.gpu_hull_geometry().expect("a geometry").key;
    app.report_gpu_hull_failure(key, "out of memory".to_owned());
    // Reported again, as the frame's other views would.
    app.report_gpu_hull_failure(key, "out of memory".to_owned());
    for _ in 0..3 {
        let out = frame(&ctx, &mut app);
        assert_eq!(callbacks(&out), 0);
        assert_painted_hull(&out);
    }
    assert_eq!(app.messages().count(Level::Warning), warnings + 1);
    let text = &app.messages().last().expect("the warning").text;
    assert!(text.contains("out of memory"), "{text}");

    // A new mesh tries again.
    app.set_subdivision_level(1);
    let out = frame(&ctx, &mut app);
    assert_eq!(callbacks(&out), 4);
    assert_eq!(app.messages().count(Level::Warning), warnings + 1);
}

// ---- through egui's renderer, on a device ----------------------------------

struct Gpu {
    instance: wgpu::Instance,
    adapter: wgpu::Adapter,
    device: wgpu::Device,
    queue: wgpu::Queue,
}

/// One device for the whole binary, as the app has one: see `tests/gpu_hull.rs`.
fn gpu() -> Option<&'static Gpu> {
    static GPU: OnceLock<Option<Gpu>> = OnceLock::new();
    let gpu = GPU
        .get_or_init(|| {
            let instance =
                wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle());
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
                instance,
                adapter,
                device,
                queue,
            })
        })
        .as_ref();
    if gpu.is_none() {
        println!("skipped: no adapter");
    }
    gpu
}

const FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;

fn render_state(gpu: &Gpu) -> egui_wgpu::RenderState {
    let renderer = egui_wgpu::Renderer::new(&gpu.device, FORMAT, Default::default());
    egui_wgpu::RenderState {
        adapter: gpu.adapter.clone(),
        available_adapters: Vec::new(),
        instance: gpu.instance.clone(),
        device: gpu.device.clone(),
        queue: gpu.queue.clone(),
        target_format: FORMAT,
        renderer: Arc::new(egui::mutex::RwLock::new(renderer)),
    }
}

/// Run a frame through the renderer as eframe does, painting into `target` when given.
fn render_frame(
    state: &egui_wgpu::RenderState,
    ctx: &egui::Context,
    app: &mut App,
    screen: egui::Vec2,
    target: Option<&wgpu::Texture>,
) {
    let mut out = ctx.run_ui(input(screen, Vec::new()), |ui| app.draw(ui));
    let descriptor = ScreenDescriptor {
        size_in_pixels: [screen.x as u32, screen.y as u32],
        pixels_per_point: out.pixels_per_point,
    };
    let primitives = ctx.tessellate(std::mem::take(&mut out.shapes), out.pixels_per_point);
    let (device, queue) = (&state.device, &state.queue);
    let mut renderer = state.renderer.write();
    for (id, deltas) in &out.textures_delta.set {
        for delta in deltas {
            renderer.update_texture(device, queue, *id, delta);
        }
    }
    out.textures_delta.clear();
    let mut encoder = device.create_command_encoder(&Default::default());
    let commands = renderer.update_buffers(device, queue, &mut encoder, &primitives, &descriptor);
    if let Some(target) = target {
        let view = target.create_view(&Default::default());
        let mut pass = encoder
            .begin_render_pass(&wgpu::RenderPassDescriptor {
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                ..Default::default()
            })
            .forget_lifetime();
        renderer.render(&mut pass, &primitives, &descriptor);
    }
    queue.submit(commands.into_iter().chain([encoder.finish()]));
}

fn read_pixels(gpu: &Gpu, texture: &wgpu::Texture) -> Vec<u8> {
    let (width, height) = (texture.width(), texture.height());
    let buffer = gpu.device.create_buffer(&wgpu::BufferDescriptor {
        label: None,
        size: u64::from(width * height * 4),
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    encoder.copy_texture_to_buffer(
        texture.as_image_copy(),
        wgpu::TexelCopyBufferInfo {
            buffer: &buffer,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(width * 4),
                rows_per_image: Some(height),
            },
        },
        texture.size(),
    );
    gpu.queue.submit([encoder.finish()]);
    buffer
        .slice(..)
        .map_async(wgpu::MapMode::Read, |r| r.expect("mapped"));
    gpu.device
        .poll(wgpu::PollType::wait_indefinitely())
        .expect("the copy finishes");
    let pixels = buffer
        .slice(..)
        .get_mapped_range()
        .expect("mapped")
        .to_vec();
    pixels
}

#[test]
#[ignore = "needs an adapter"]
fn a_failed_callback_reports_and_asks_for_the_next_frame() {
    let Some(gpu) = gpu() else { return };
    let context = egui::Context::default();
    for _ in 0..5 {
        let mut out = context.run_ui(input(SCREEN, Vec::new()), |_| {});
        out.textures_delta.clear();
    }
    assert!(!context.has_requested_repaint(), "an idle window");
    let geometry = HullGeometry {
        key: 42,
        origin: Default::default(),
        min: Default::default(),
        max: Default::default(),
        vertices: Vec::new(),
        edges: Vec::new(),
    };
    let failure = FailureSlot::default();
    let callback = HullCallback {
        index: 0,
        uniform: bytemuck::Zeroable::zeroed(),
        size: [8, 8],
        view_key: 1,
        geometry: Arc::new(geometry),
        shaded: true,
        wire: false,
        failure: failure.clone(),
        context: context.clone(),
    };
    let screen = ScreenDescriptor {
        size_in_pixels: [8, 8],
        pixels_per_point: 1.0,
    };
    let mut encoder = gpu.device.create_command_encoder(&Default::default());
    // No resources in the renderer: a failure with no device error in it.
    let mut resources = egui_wgpu::CallbackResources::default();
    callback.prepare(
        &gpu.device,
        &gpu.queue,
        &screen,
        &mut encoder,
        &mut resources,
    );
    let reported = failure.lock().unwrap().take();
    assert_eq!(reported.map(|(key, _)| key), Some(42));
    assert!(context.has_requested_repaint());
}

#[test]
#[ignore = "needs an adapter"]
fn the_renderer_draws_every_view_through_the_callback() {
    let Some(gpu) = gpu() else { return };
    let state = render_state(gpu);
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    app.enable_gpu_hull(&state);
    // Nothing over the hull where the test looks.
    let display = app.display_mut();
    display.grid = false;
    display.waterplane = false;
    display.points = false;
    display.control_net = false;
    let ctx = egui::Context::default();
    let target = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d {
            width: SCREEN.x as u32,
            height: SCREEN.y as u32,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: FORMAT,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    for _ in 0..3 {
        render_frame(&state, &ctx, &mut app, SCREEN, Some(&target));
    }
    {
        let renderer = state.renderer.read();
        let resources = renderer
            .callback_resources
            .get::<GpuHullResources>()
            .expect("enable_gpu_hull inserted the resources");
        for view in 0..4 {
            assert!(resources.resolved_texture(view).is_some(), "view {view}");
        }
    }
    let warnings = app.messages().count(Level::Warning);
    assert_eq!(warnings, 0, "no failure was reported");

    // The middle of the hull's box, in the profile view, on the frame eframe would present.
    let (min, max) = app.surface().expect("a model").extents();
    let centre = app.cameras()[1]
        .transform()
        .project((min + max) * 0.5, app.viewport_rect(1))
        .expect("visible");
    let pixels = read_pixels(gpu, &target);
    let at = (centre.y as usize * SCREEN.x as usize + centre.x as usize) * 4;
    let pixel = [pixels[at], pixels[at + 1], pixels[at + 2], pixels[at + 3]];
    assert_ne!(pixel, theme::CANVAS.to_array(), "the hull is composited");
    assert_eq!(pixel[3], 255);
}

#[test]
#[ignore = "needs an adapter"]
fn a_view_the_device_refuses_falls_back_and_logs_once() {
    let Some(gpu) = gpu() else { return };
    let state = render_state(gpu);
    let mut app = App::headless_for_test(Some(&demo_path()));
    app.set_subdivision_level(2);
    app.enable_gpu_hull(&state);
    let ctx = egui::Context::default();

    // Cells wider than the default limit's 8192 pixels: a real refusal, on the path a real one takes.
    let huge = vec2(20000.0, 1000.0);
    render_frame(&state, &ctx, &mut app, huge, None);
    assert!(app.viewport_rect(0).width() > 8192.0);
    {
        let renderer = state.renderer.read();
        let resources = renderer.callback_resources.get::<GpuHullResources>();
        let resources = resources.expect("the resources");
        assert!(
            resources.resolved_texture(0).is_none(),
            "nothing is current"
        );
    }
    let warnings = app.messages().count(Level::Warning);
    assert_eq!(warnings, 0, "not reported until the next frame");

    for _ in 0..3 {
        let out = frame_sized(&ctx, &mut app, huge, Vec::new());
        assert_eq!(callbacks(&out), 0);
        assert_painted_hull(&out);
    }
    assert_eq!(app.messages().count(Level::Warning), 1);

    // A new mesh at a size the device takes routes again, and draws.
    app.set_subdivision_level(1);
    render_frame(&state, &ctx, &mut app, SCREEN, None);
    render_frame(&state, &ctx, &mut app, SCREEN, None);
    let renderer = state.renderer.read();
    let resources = renderer.callback_resources.get::<GpuHullResources>();
    assert!(resources
        .expect("the resources")
        .resolved_texture(0)
        .is_some());
    assert_eq!(app.messages().count(Level::Warning), 1);
}
