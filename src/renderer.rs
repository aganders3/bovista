//! GPU renderer that manages graphics resources and coordinates scene rendering.
//!
//! The [`Renderer`] is the central component responsible for:
//! - Managing GPU device and command queue
//! - Creating and updating camera uniforms
//! - Providing shared GPU resources (bind group layouts)
//! - Coordinating render passes and frame presentation

use crate::{Camera, Scene};
use bytemuck::{Pod, Zeroable};

/// GPU-compatible camera uniform buffer structure.
///
/// This structure is uploaded to the GPU and made available to all shaders
/// via bind group 0. It contains the combined view-projection matrix for
/// transforming vertices from world space to clip space.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct CameraUniforms {
    /// Combined view-projection matrix (projection * view)
    pub view_proj: [[f32; 4]; 4],
}

/// The main renderer manages GPU resources and coordinates scene rendering.
///
/// The renderer owns the GPU device and queue, and provides shared resources
/// like camera bind groups that all visuals can access. It implements a
/// standard forward rendering pipeline with depth testing.
///
/// # Architecture
///
/// ```text
/// ┌──────────────────────────────────────────────────┐
/// │                   Renderer                       │
/// ├──────────────────────────────────────────────────┤
/// │  - WGPU Device & Queue                           │
/// │  - Camera Uniform Buffer (Bind Group 0)          │
/// │  - Surface Format Configuration                  │
/// │  - Depth Texture Management                      │
/// └──────────────────────────────────────────────────┘
///         │                       │
///         ▼                       ▼
///    ┌─────────┐           ┌──────────┐
///    │  Scene  │           │  Camera  │
///    └─────────┘           └──────────┘
///         │
///         ▼
///    ┌─────────────────┐
///    │  Visual Objects │
///    │  (Points, Lines,│
///    │   Images, etc.) │
///    └─────────────────┘
/// ```
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    camera_uniform_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
}

impl Renderer {
    pub async fn new(
        device: wgpu::Device,
        queue: wgpu::Queue,
        surface_format: wgpu::TextureFormat,
    ) -> Self {
        let camera_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Camera Uniform Buffer"),
            size: std::mem::size_of::<CameraUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let camera_bind_group_layout =
            device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some("Camera Bind Group Layout"),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });

        let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Camera Bind Group"),
            layout: &camera_bind_group_layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: camera_uniform_buffer.as_entire_binding(),
            }],
        });

        Self {
            device,
            queue,
            camera_uniform_buffer,
            camera_bind_group,
            camera_bind_group_layout,
            surface_format,
        }
    }

    pub fn update_camera(&self, camera: &Camera) {
        let uniforms = CameraUniforms {
            view_proj: camera.view_projection_matrix().to_cols_array_2d(),
        };
        self.queue.write_buffer(
            &self.camera_uniform_buffer,
            0,
            bytemuck::cast_slice(&[uniforms]),
        );
    }

    pub fn create_depth_texture(&self, width: u32, height: u32) -> wgpu::TextureView {
        let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Depth Texture"),
            size: wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24PlusStencil8,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
            view_formats: &[],
        });

        depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
    }

    pub fn render(
        &self,
        scene: &Scene,
        view: &wgpu::TextureView,
        depth_view: &wgpu::TextureView,
        clear_color: wgpu::Color,
    ) {
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("Render Encoder"),
            });

        {
            let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("Render Pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    depth_slice: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(clear_color),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: depth_view,
                    depth_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(1.0),
                        store: wgpu::StoreOp::Store,
                    }),
                    stencil_ops: Some(wgpu::Operations {
                        load: wgpu::LoadOp::Clear(0),  // Clear to 0 (lowest priority)
                        store: wgpu::StoreOp::Store,   // Save stencil state
                    }),
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
            });

            // Render all visuals. Scene re-binds the camera bind group at
            // slot 0 between each visual so that visuals like Lines / Points
            // — which expect the shared camera layout at slot 0 — keep
            // working after Volume / Image override slot 0 with their own
            // combined-UBO bind group (NVIDIA GLES UBO-aliasing workaround).
            scene.render(&mut render_pass, &self.camera_bind_group);
        }

        self.queue.submit(std::iter::once(encoder.finish()));
    }

    pub fn device(&self) -> &wgpu::Device {
        &self.device
    }

    pub fn queue(&self) -> &wgpu::Queue {
        &self.queue
    }

    pub fn surface_format(&self) -> wgpu::TextureFormat {
        self.surface_format
    }

    pub fn camera_bind_group_layout(&self) -> &wgpu::BindGroupLayout {
        &self.camera_bind_group_layout
    }
}
