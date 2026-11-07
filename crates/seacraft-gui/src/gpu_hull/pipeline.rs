//! The hull's wgpu resources: each view renders offscreen, and only when its key changes, then a fullscreen triangle
//! composites the finished texture into egui's pass every frame.
//!
//! wgpu's default handler panics on an uncaptured error, and neither eframe nor egui-wgpu installs another, so every
//! allocation here runs inside error scopes. On a failure the resources forget all they hold and remember the error, so
//! a failed frame shows no hull rather than a stale one and the caller can fall back. A mesh with a new key starts over.

use std::mem::size_of;

use eframe::wgpu;

use super::geometry::{chunk_ranges, HullGeometry, HullVertex, MAX_BUFFER_BYTES};
use super::view::ViewUniform;

/// The hull's shaders.
pub const SHADER: &str = include_str!("hull.wgsl");

const COLOUR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
const DEPTH_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const VERTEX_BYTES: u64 = size_of::<HullVertex>() as u64;
const EDGE_POINT_BYTES: u64 = size_of::<[f32; 3]>() as u64;
const VIEWS: usize = 4;

/// One vertex buffer, holding whole primitives.
struct Chunk {
    buffer: wgpu::Buffer,
    vertices: u32,
}

/// One view's offscreen images. Never built with a zero-sized side.
struct Target {
    size: [u32; 2],
    /// The multisampled colour image; `None` at one sample, when `resolved` is drawn into directly.
    multisampled: Option<wgpu::TextureView>,
    resolved: wgpu::Texture,
    resolved_view: wgpu::TextureView,
    depth: wgpu::TextureView,
    composite: wgpu::BindGroup,
}

struct View {
    uniform: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    target: Option<Target>,
    /// The key of what `target` holds; `None` when that is nothing current.
    key: Option<u64>,
}

/// Why the resources are empty: what failed, and the geometry it failed with, if any was uploaded.
struct Failure {
    geometry_key: Option<u64>,
    message: String,
}

/// The pipelines, per-view targets and uploaded buffers of the GPU hull.
pub struct GpuHullResources {
    sample_count: u32,
    buffer_cap: u64,
    shaded_pipeline: wgpu::RenderPipeline,
    wire_pipeline: wgpu::RenderPipeline,
    composite_pipeline: wgpu::RenderPipeline,
    composite_layout: wgpu::BindGroupLayout,
    sampler: wgpu::Sampler,
    views: [View; VIEWS],
    triangles: Vec<Chunk>,
    lines: Vec<Chunk>,
    uploaded: Option<u64>,
    failure: Option<Failure>,
}

/// Run `allocate` in out-of-memory and validation scopes, and return what they caught.
fn scoped<T>(device: &wgpu::Device, allocate: impl FnOnce() -> T) -> Result<T, String> {
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let value = allocate();
    let invalid = pollster::block_on(validation.pop());
    let no_memory = pollster::block_on(out_of_memory.pop());
    match no_memory.or(invalid) {
        Some(error) => Err(error.to_string()),
        None => Ok(value),
    }
}

/// 4 when both the colour and the depth format offer it, else 1.
fn sample_count(adapter: &wgpu::Adapter) -> u32 {
    use wgpu::TextureFormatFeatureFlags as Flags;
    let colour = adapter.get_texture_format_features(COLOUR_FORMAT).flags;
    let depth = adapter.get_texture_format_features(DEPTH_FORMAT).flags;
    if colour.contains(Flags::MULTISAMPLE_X4 | Flags::MULTISAMPLE_RESOLVE)
        && depth.contains(Flags::MULTISAMPLE_X4)
    {
        4
    } else {
        1
    }
}

fn create_target(
    device: &wgpu::Device,
    sample_count: u32,
    composite_layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    size: [u32; 2],
) -> Target {
    let image = |label, format, sample_count, usage| {
        device.create_texture(&wgpu::TextureDescriptor {
            label: Some(label),
            size: wgpu::Extent3d {
                width: size[0],
                height: size[1],
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count,
            dimension: wgpu::TextureDimension::D2,
            format,
            usage,
            view_formats: &[],
        })
    };
    let view = |texture: &wgpu::Texture| texture.create_view(&Default::default());
    let resolved = image(
        "gpu hull resolved",
        COLOUR_FORMAT,
        1,
        wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::TEXTURE_BINDING
            | wgpu::TextureUsages::COPY_SRC,
    );
    let resolved_view = view(&resolved);
    let multisampled = (sample_count > 1).then(|| {
        view(&image(
            "gpu hull colour",
            COLOUR_FORMAT,
            sample_count,
            wgpu::TextureUsages::RENDER_ATTACHMENT,
        ))
    });
    let depth = view(&image(
        "gpu hull depth",
        DEPTH_FORMAT,
        sample_count,
        wgpu::TextureUsages::RENDER_ATTACHMENT,
    ));
    let composite = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("gpu hull composite"),
        layout: composite_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::TextureView(&resolved_view),
            },
            wgpu::BindGroupEntry {
                binding: 2,
                resource: wgpu::BindingResource::Sampler(sampler),
            },
        ],
    });
    Target {
        size,
        multisampled,
        resolved,
        resolved_view,
        depth,
        composite,
    }
}

impl GpuHullResources {
    /// Everything but the targets and the geometry, which come later. It can raise device errors of
    /// its own, so a caller that must not panic wraps it in an error scope.
    pub fn new(
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        target_format: wgpu::TextureFormat,
    ) -> Self {
        Self::with_buffer_cap(device, adapter, target_format, MAX_BUFFER_BYTES)
    }

    /// [`Self::new`] with vertex buffers of at most `min(device.limits().max_buffer_size, cap)`
    /// bytes. A test hook: it reaches splitting, and failure, without gigabytes of vertices.
    pub fn with_buffer_cap(
        device: &wgpu::Device,
        adapter: &wgpu::Adapter,
        target_format: wgpu::TextureFormat,
        cap: u64,
    ) -> Self {
        let sample_count = sample_count(adapter);
        let module = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("gpu hull"),
            source: wgpu::ShaderSource::Wgsl(SHADER.into()),
        });

        let view_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu hull view"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: wgpu::BufferSize::new(size_of::<ViewUniform>() as u64),
                },
                count: None,
            }],
        });
        let composite_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("gpu hull composite"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let layout_of = |bind_group: &wgpu::BindGroupLayout| {
            device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                label: Some("gpu hull"),
                bind_group_layouts: &[Some(bind_group)],
                immediate_size: 0,
            })
        };
        let view_pipeline_layout = layout_of(&view_layout);

        let depth_state = |write: bool, compare| wgpu::DepthStencilState {
            format: DEPTH_FORMAT,
            depth_write_enabled: Some(write),
            depth_compare: Some(compare),
            stencil: wgpu::StencilState::default(),
            bias: wgpu::DepthBiasState::default(),
        };
        let multisample = wgpu::MultisampleState {
            count: sample_count,
            mask: !0,
            alpha_to_coverage_enabled: false,
        };

        let shaded_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpu hull shaded"),
            layout: Some(&view_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_hull"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: VERTEX_BYTES,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    // `Unorm8x4` applies no sRGB conversion, so the colour stays in gamma space.
                    attributes: &wgpu::vertex_attr_array![
                        0 => Float32x3,
                        1 => Float32x3,
                        2 => Unorm8x4,
                    ],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                // The Body view flips winding, and open shells show both sides.
                cull_mode: None,
                ..Default::default()
            },
            depth_stencil: Some(depth_state(true, wgpu::CompareFunction::Less)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_hull"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOUR_FORMAT,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let wire_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpu hull wire"),
            layout: Some(&view_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_wire"),
                buffers: &[Some(wgpu::VertexBufferLayout {
                    array_stride: EDGE_POINT_BYTES,
                    step_mode: wgpu::VertexStepMode::Vertex,
                    attributes: &wgpu::vertex_attr_array![0 => Float32x3],
                })],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::LineList,
                ..Default::default()
            },
            // See-through, as the painted wireframe is.
            depth_stencil: Some(depth_state(false, wgpu::CompareFunction::Always)),
            multisample,
            fragment: Some(wgpu::FragmentState {
                module: &module,
                entry_point: Some("fs_wire"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: COLOUR_FORMAT,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let composite_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("gpu hull composite"),
            layout: Some(&layout_of(&composite_layout)),
            vertex: wgpu::VertexState {
                module: &module,
                entry_point: Some("vs_composite"),
                buffers: &[],
                compilation_options: Default::default(),
            },
            primitive: wgpu::PrimitiveState::default(),
            // eframe's pass: one sample, no depth (`multisampling: 0`, the default).
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            fragment: Some(wgpu::FragmentState {
                module: &module,
                // The same choice egui's renderer makes.
                entry_point: Some(if target_format.is_srgb() {
                    "fs_linear"
                } else {
                    "fs_gamma"
                }),
                targets: &[Some(wgpu::ColorTargetState {
                    format: target_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            multiview_mask: None,
            cache: None,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("gpu hull composite"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let views = std::array::from_fn(|_| {
            let uniform = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu hull view"),
                size: size_of::<ViewUniform>() as u64,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("gpu hull view"),
                layout: &view_layout,
                entries: &[wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform.as_entire_binding(),
                }],
            });
            View {
                uniform,
                bind_group,
                target: None,
                key: None,
            }
        });

        Self {
            sample_count,
            buffer_cap: device.limits().max_buffer_size.min(cap),
            shaded_pipeline,
            wire_pipeline,
            composite_pipeline,
            composite_layout,
            sampler,
            views,
            triangles: Vec::new(),
            lines: Vec::new(),
            uploaded: None,
            failure: None,
        }
    }

    /// Forget everything held, and remember why.
    fn fail(&mut self, geometry_key: Option<u64>, message: String) -> Result<(), String> {
        self.triangles.clear();
        self.lines.clear();
        self.uploaded = None;
        for view in &mut self.views {
            view.target = None;
            view.key = None;
        }
        self.failure = Some(Failure {
            geometry_key,
            message: message.clone(),
        });
        Err(message)
    }

    /// Give the GPU `geometry`. A no-op when its key is already uploaded. On an out-of-memory or
    /// validation error this keeps nothing half uploaded, and the same key fails again at once
    /// without allocating. A new key leaves every view without a current texture until it renders.
    pub fn upload(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        geometry: &HullGeometry,
    ) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            if failure.geometry_key == Some(geometry.key) {
                return Err(failure.message.clone());
            }
        }
        if self.uploaded == Some(geometry.key) {
            return Ok(());
        }
        // Free the old mesh first: the new one may need the memory.
        self.triangles.clear();
        self.lines.clear();
        self.uploaded = None;
        for view in &mut self.views {
            view.key = None;
        }

        let cap = self.buffer_cap;
        let uploaded = upload_chunks(device, queue, &geometry.vertices, VERTEX_BYTES, 3, cap)
            .and_then(|triangles| {
                let points = &geometry.edges;
                let lines = upload_chunks(device, queue, points, EDGE_POINT_BYTES, 2, cap)?;
                Ok((triangles, lines))
            });
        match uploaded {
            Ok((triangles, lines)) => {
                self.triangles = triangles;
                self.lines = lines;
                self.uploaded = Some(geometry.key);
                self.failure = None;
                Ok(())
            }
            Err(message) => self.fail(Some(geometry.key), message),
        }
    }

    /// Render view `index` (0..4) unless `view_key` is what its texture already holds at `size`. It
    /// clears to transparent, then draws the triangles if `shaded` and the lines if `wire`. A size
    /// with a zero side leaves the view with no target. A failed allocation forgets everything, as
    /// a failed [`Self::upload`] does, and this fails at once until a mesh with a new key uploads.
    #[allow(clippy::too_many_arguments)]
    pub fn render_view(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        index: usize,
        uniform: &ViewUniform,
        size: [u32; 2],
        view_key: u64,
        shaded: bool,
        wire: bool,
    ) -> Result<(), String> {
        if let Some(failure) = &self.failure {
            return Err(failure.message.clone());
        }
        let Some(view) = self.views.get_mut(index) else {
            return Err(format!("no view {index}"));
        };
        if size[0] == 0 || size[1] == 0 {
            view.target = None;
            view.key = None;
            return Ok(());
        }
        let held_size = view.target.as_ref().map(|target| target.size);
        if held_size == Some(size) && view.key == Some(view_key) {
            return Ok(());
        }
        // A key is only true of what was drawn after this, whatever happens below.
        view.key = None;
        let written = scoped(device, || {
            if held_size != Some(size) {
                // The old images go first: the new ones may need the memory.
                drop(view.target.take());
                view.target = Some(create_target(
                    device,
                    self.sample_count,
                    &self.composite_layout,
                    &self.sampler,
                    size,
                ));
            }
            queue.write_buffer(&view.uniform, 0, bytemuck::bytes_of(uniform));
        });
        if let Err(message) = written {
            return self.fail(self.uploaded, message);
        }
        let view = &mut self.views[index];
        let Some(target) = &view.target else {
            return Err(format!("view {index} has no target"));
        };

        let (attachment, resolve_target, store) = match &target.multisampled {
            Some(multisampled) => (
                multisampled,
                Some(&target.resolved_view),
                wgpu::StoreOp::Discard,
            ),
            None => (&target.resolved_view, None, wgpu::StoreOp::Store),
        };
        let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("gpu hull"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: attachment,
                depth_slice: None,
                resolve_target,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(wgpu::Color::TRANSPARENT),
                    store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: &target.depth,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Discard,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_bind_group(0, &view.bind_group, &[]);
        let mut draw = |pipeline, chunks: &[Chunk]| {
            pass.set_pipeline(pipeline);
            for chunk in chunks {
                pass.set_vertex_buffer(0, chunk.buffer.slice(..));
                pass.draw(0..chunk.vertices, 0..1);
            }
        };
        if shaded {
            draw(&self.shaded_pipeline, &self.triangles);
        }
        if wire {
            draw(&self.wire_pipeline, &self.lines);
        }
        drop(pass);
        view.key = Some(view_key);
        Ok(())
    }

    /// The view's images, if they hold what was last rendered for the mesh now uploaded.
    fn current(&self, index: usize) -> Option<&Target> {
        let view = self.views.get(index)?;
        view.key.and(view.target.as_ref())
    }

    /// Composite view `index` into `pass`; nothing when its texture is not current.
    pub fn paint(&self, pass: &mut wgpu::RenderPass<'static>, index: usize) {
        if let Some(target) = self.current(index) {
            pass.set_pipeline(&self.composite_pipeline);
            pass.set_bind_group(0, &target.composite, &[]);
            pass.draw(0..3, 0..1);
        }
    }

    /// View `index`'s finished image, if current. It has `COPY_SRC`, for reading back in a test.
    pub fn resolved_texture(&self, index: usize) -> Option<&wgpu::Texture> {
        self.current(index).map(|target| &target.resolved)
    }

    /// How many buffers hold the triangles and the lines, so a test can see the geometry split.
    pub fn buffer_counts(&self) -> (usize, usize) {
        (self.triangles.len(), self.lines.len())
    }
}

/// Upload `items` into buffers of at most `cap` bytes, whole primitives of `unit` items each.
fn upload_chunks<T: bytemuck::Pod>(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    items: &[T],
    item_bytes: u64,
    unit: usize,
    cap: u64,
) -> Result<Vec<Chunk>, String> {
    if !items.is_empty() && cap < item_bytes * unit as u64 {
        return Err(format!(
            "a vertex buffer of {cap} bytes cannot hold one primitive"
        ));
    }
    let mut chunks = Vec::new();
    for range in chunk_ranges(items.len(), item_bytes, unit, cap) {
        let items = &items[range];
        let chunk = scoped(device, || {
            let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some("gpu hull vertices"),
                size: items.len() as u64 * item_bytes,
                usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buffer, 0, bytemuck::cast_slice(items));
            Chunk {
                buffer,
                vertices: items.len() as u32,
            }
        })?;
        chunks.push(chunk);
    }
    Ok(chunks)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wgpu::naga;

    #[test]
    fn the_shader_parses_and_validates() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("the shader parses");
        naga::valid::Validator::new(
            naga::valid::ValidationFlags::all(),
            naga::valid::Capabilities::default(),
        )
        .validate(&module)
        .expect("the shader validates");
        let entry = |name: &str| module.entry_points.iter().any(|e| e.name == name);
        for name in [
            "vs_hull",
            "fs_hull",
            "vs_wire",
            "fs_wire",
            "vs_composite",
            "fs_gamma",
            "fs_linear",
        ] {
            assert!(entry(name), "no entry point {name}");
        }
    }

    #[test]
    fn the_wgsl_view_struct_mirrors_the_uniform() {
        let module = naga::front::wgsl::parse_str(SHADER).expect("the shader parses");
        let span = module
            .types
            .iter()
            .find_map(|(_, t)| match (&t.name, &t.inner) {
                (Some(name), naga::TypeInner::Struct { span, .. }) if name == "View" => Some(*span),
                _ => None,
            })
            .expect("a struct named View");
        assert_eq!(span as usize, std::mem::size_of::<ViewUniform>());
        assert_eq!(std::mem::size_of::<ViewUniform>(), 128);
    }
}
