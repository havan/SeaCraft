//! The hull drawn by the GPU: vertex data built on the CPU, uploaded once per mesh, the view uniforms that
//! reproduce the viewport's projection, and the pipelines that render each view offscreen and composite it.

pub mod geometry;
pub mod pipeline;
pub mod view;
