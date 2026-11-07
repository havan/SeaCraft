//! The hull drawn by the GPU: vertex data built on the CPU, uploaded once per mesh, the view uniforms that
//! reproduce the viewport's projection, and the pipelines that render each view offscreen and composite it.

pub mod geometry;
pub mod pipeline;
pub mod view;

use std::sync::{Arc, Mutex};

use eframe::egui_wgpu::{self, CallbackResources, ScreenDescriptor};
use eframe::{egui, wgpu};

use geometry::HullGeometry;
use pipeline::GpuHullResources;
use view::ViewUniform;

/// Where a frame's callbacks report a failure, as the geometry key and the message.
pub type FailureSlot = Arc<Mutex<Option<(u64, String)>>>;

/// One view's hull in egui's paint list: `prepare` renders it offscreen if its key changed, and
/// `paint` composites the finished texture.
pub struct HullCallback {
    pub index: usize,
    pub uniform: ViewUniform,
    pub size: [u32; 2],
    pub view_key: u64,
    pub geometry: Arc<HullGeometry>,
    pub shaded: bool,
    pub wire: bool,
    pub failure: FailureSlot,
    /// A failure is only seen at the start of a frame, and an idle window has no next frame unless
    /// it is asked for.
    pub context: egui::Context,
}

impl HullCallback {
    fn render(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Result<(), String> {
        let resources = resources
            .get_mut::<GpuHullResources>()
            .ok_or("the GPU hull has no resources")?;
        resources.upload(device, queue, &self.geometry)?;
        resources.render_view(
            device,
            queue,
            encoder,
            self.index,
            &self.uniform,
            self.size,
            self.view_key,
            self.shaded,
            self.wire,
        )
    }
}

impl egui_wgpu::CallbackTrait for HullCallback {
    fn prepare(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        _screen: &ScreenDescriptor,
        encoder: &mut wgpu::CommandEncoder,
        resources: &mut CallbackResources,
    ) -> Vec<wgpu::CommandBuffer> {
        if let Err(message) = self.render(device, queue, encoder, resources) {
            if let Ok(mut slot) = self.failure.lock() {
                slot.get_or_insert((self.geometry.key, message));
            }
            self.context.request_repaint();
        }
        Vec::new()
    }

    fn paint(
        &self,
        info: egui::PaintCallbackInfo,
        pass: &mut wgpu::RenderPass<'static>,
        resources: &CallbackResources,
    ) {
        if let Some(resources) = resources.get::<GpuHullResources>() {
            if viewport_fits(&info, self.size) {
                resources.paint(pass, self.index);
            }
        }
    }
}

/// Whether the viewport egui set for a callback is the size of its texture. The window can be resized
/// between laying a frame out and painting it, and a texture composited into another size would be
/// stretched off the overlays; the view is then left empty for that frame.
fn viewport_fits(info: &egui::PaintCallbackInfo, size: [u32; 2]) -> bool {
    let viewport = info.viewport_in_pixels();
    viewport.width_px as u32 == size[0] && viewport.height_px as u32 == size[1]
}

/// [`GpuHullResources::new`], with what it raises on the device caught rather than sent to wgpu's
/// default handler, which panics.
pub fn create_resources(state: &egui_wgpu::RenderState) -> Result<GpuHullResources, String> {
    let device = &state.device;
    let out_of_memory = device.push_error_scope(wgpu::ErrorFilter::OutOfMemory);
    let validation = device.push_error_scope(wgpu::ErrorFilter::Validation);
    let resources = GpuHullResources::new(device, &state.adapter, state.target_format);
    let invalid = pollster::block_on(validation.pop());
    let no_memory = pollster::block_on(out_of_memory.pop());
    match no_memory.or(invalid) {
        Some(error) => Err(error.to_string()),
        None => Ok(resources),
    }
}

#[cfg(test)]
mod tests {
    use eframe::egui::{pos2, Rect};

    use super::*;

    fn info(left: f32, right: f32, screen_width: u32) -> egui::PaintCallbackInfo {
        let viewport = Rect::from_min_max(pos2(left, 0.0), pos2(right, 50.0));
        egui::PaintCallbackInfo {
            viewport,
            clip_rect: viewport,
            pixels_per_point: 1.0,
            screen_size_px: [screen_width, 100],
        }
    }

    #[test]
    fn a_viewport_clamped_to_the_screen_does_not_fit_its_texture() {
        assert!(viewport_fits(&info(100.0, 200.0, 300), [100, 50]));
        // The window shrank between layout and paint.
        assert!(!viewport_fits(&info(100.0, 200.0, 150), [100, 50]));
        assert!(!viewport_fits(&info(100.0, 200.0, 300), [101, 50]));
    }
}
