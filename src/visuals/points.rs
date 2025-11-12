//! Point cloud rendering for 3D scatter plots and particle systems.
//!
//! This module provides [`PointsVisual`] for rendering collections of colored points in 3D space.
//! Each point has a position and RGB color, making it ideal for:
//! - Scientific data visualization (scatter plots)
//! - Particle systems
//! - Point cloud data from sensors (LiDAR, photogrammetry)
//!
//! # Examples
//!
//! ## Creating a Point Cloud from Vertices
//!
//! ```rust,no_run
//! use bovista::{PointsVisual, Renderer};
//! use bovista::visuals::points::PointVertex;
//! # async fn example(renderer: Renderer) {
//!
//! // Create colored vertices
//! let vertices = vec![
//!     PointVertex { position: [0.0, 0.0, 0.0], color: [1.0, 0.0, 0.0] }, // Red
//!     PointVertex { position: [1.0, 0.0, 0.0], color: [0.0, 1.0, 0.0] }, // Green
//!     PointVertex { position: [0.0, 1.0, 0.0], color: [0.0, 0.0, 1.0] }, // Blue
//! ];
//!
//! let points = PointsVisual::new(
//!     renderer.device(),
//!     renderer.surface_format(),
//!     renderer.camera_bind_group_layout(),
//!     vertices
//! );
//! # }
//! ```
//!
//! ## Live Demo
//!
//! **[🌐 Try it in your browser →](https://yourdomain.com/bovista/examples/point_cloud.html)**
//!
//! Runs directly in any browser with WebGPU support (Chrome 113+, Safari 18+).
//! No installation required!
//!
//! ## Python Usage
//!
//! ```python
//! import bovista as bv
//! import numpy as np
//!
//! viewer = bv.PyViewer(800, 600)
//! viewer.initialize()
//!
//! # Generate random point cloud
//! n_points = 10000
//! positions = np.random.randn(n_points, 1, 3).astype(np.float32) * 2.0
//! colors = np.random.rand(n_points, 1, 3).astype(np.float32)
//!
//! points = bv.PyPointsVisual.from_numpy(viewer, positions, colors)
//! viewer.add_points(points)
//! viewer.run()  # Opens interactive window
//! ```
//!
//! ## Local Examples
//!
//! ### Desktop (Rust)
//!
//! ```bash
//! cargo run --example point_cloud
//! ```
//!
//! ### Browser (WASM)
//!
//! ```bash
//! ./build_wasm.sh wasm_point_cloud
//! python3 -m http.server 8000 --directory web
//! # Open http://localhost:8000
//! ```
//!
//! ### Controls
//!
//! - **Left-drag**: Orbit camera
//! - **Scroll**: Zoom in/out
//! - **Close**: Exit

use crate::visual::{Transform, Visual};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use wgpu::RenderPass;

/// A vertex in a point cloud with position and RGB color.
///
/// Each vertex is 24 bytes (2×[f32; 3]) and can be directly uploaded to GPU memory.
/// Marked with [`Pod`] and [`Zeroable`] for safe zero-copy transfer.
///
/// # Example
///
/// ```rust
/// use bovista::visuals::points::PointVertex;
///
/// let vertex = PointVertex {
///     position: [1.0, 2.0, 3.0],  // World-space position
///     color: [1.0, 0.5, 0.0],     // RGB in 0-1 range (orange)
/// };
/// ```
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PointVertex {
    /// World-space position (X, Y, Z)
    pub position: [f32; 3],
    /// RGB color (each component 0.0 to 1.0)
    pub color: [f32; 3],
}

impl PointVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PointVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Visual for rendering point clouds in 3D space.
///
/// `PointsVisual` renders a collection of colored points using GPU acceleration.
/// Each point is rendered as a screen-space circle with configurable size.
///
/// # GPU Resources
///
/// - **Vertex Buffer**: Stores all point positions and colors (immutable after creation)
/// - **Render Pipeline**: Point rendering shader with depth testing
/// - **Memory**: ~24 bytes per point (position + color)
///
/// # Performance
///
/// Can efficiently render millions of points per frame on modern GPUs.
/// Rendering cost is O(n) where n is the number of visible points.
///
/// # Examples
///
/// ## Basic Usage
///
/// ```rust,no_run
/// # use bovista::{PointsVisual, Renderer, Scene};
/// # use bovista::visuals::points::PointVertex;
/// # use std::sync::{Arc, Mutex};
/// # async fn example(renderer: Renderer) {
/// let vertices = vec![
///     PointVertex { position: [0.0, 0.0, 0.0], color: [1.0, 0.0, 0.0] },
///     PointVertex { position: [1.0, 1.0, 1.0], color: [0.0, 1.0, 0.0] },
/// ];
///
/// let points = PointsVisual::new(
///     renderer.device(),
///     renderer.surface_format(),
///     renderer.camera_bind_group_layout(),
///     vertices
/// );
///
/// let mut scene = Scene::new();
/// scene.add(Arc::new(Mutex::new(points)));
/// # }
/// ```
///
/// ## Try it yourself
///
/// Run the complete interactive example:
///
/// ```bash
/// cargo run --example point_cloud
/// ```
pub struct PointsVisual {
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    render_pipeline: wgpu::RenderPipeline,
    transform: Transform,
    visible: bool,
    name: String,
}

impl PointsVisual {
    /// Creates a new point cloud visual from vertex data.
    ///
    /// # Arguments
    ///
    /// * `device` - GPU device for creating buffers and pipelines
    /// * `surface_format` - Target surface format for rendering
    /// * `camera_bind_group_layout` - Shared camera uniform layout
    /// * `vertices` - Vector of point vertices (position + color)
    ///
    /// # Note
    ///
    /// Vertex data is uploaded to GPU immediately and cannot be modified after creation.
    /// To update points, create a new `PointsVisual` instance.
    ///
    /// # Example
    ///
    /// ```rust,no_run
    /// # use bovista::{PointsVisual, Renderer};
    /// # use bovista::visuals::points::PointVertex;
    /// # async fn example(renderer: Renderer) {
    /// let vertices = vec![
    ///     PointVertex { position: [0.0, 0.0, 0.0], color: [1.0, 1.0, 1.0] },
    ///     // ... more vertices
    /// ];
    ///
    /// let points = PointsVisual::new(
    ///     renderer.device(),
    ///     renderer.surface_format(),
    ///     renderer.camera_bind_group_layout(),
    ///     vertices
    /// );
    /// # }
    /// ```
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        vertices: Vec<PointVertex>,
    ) -> Self {
        Self::with_shader(
            device,
            surface_format,
            camera_bind_group_layout,
            vertices,
            include_str!("../shaders/point_cloud.wgsl"),
        )
    }

    /// Create a new PointsVisual with a custom shader
    pub fn with_shader(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        vertices: Vec<PointVertex>,
        shader_source: &str,
    ) -> Self {
        let vertex_count = vertices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Points Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Points Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Points Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Points Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[PointVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::REPLACE),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::PointList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::Less,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        Self {
            vertex_buffer,
            vertex_count,
            render_pipeline,
            transform: Transform::identity(),
            visible: true,
            name: "Points".to_string(),
        }
    }

    /// Creates a test cube of colored points for demos and testing.
    ///
    /// Generates a `size × size × size` cube of points centered at the origin,
    /// spanning from -1 to +1 in each dimension. Points are colored based on
    /// their position (red = X, green = Y, blue = Z), creating a rainbow gradient.
    ///
    /// # Arguments
    ///
    /// * `size` - Number of points along each axis (total points = size³)
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// # use bovista::{PointsVisual, Renderer};
    /// # async fn example(renderer: Renderer) {
    /// // Create 20×20×20 = 8,000 colored points
    /// let cube = PointsVisual::test_cube(
    ///     renderer.device(),
    ///     renderer.surface_format(),
    ///     renderer.camera_bind_group_layout(),
    ///     20
    /// );
    /// # }
    /// ```
    ///
    /// # Performance
    ///
    /// - `size = 10`: 1,000 points (~24 KB)
    /// - `size = 50`: 125,000 points (~3 MB)
    /// - `size = 100`: 1,000,000 points (~24 MB)
    ///
    /// # Try it Live
    ///
    /// **[🌐 Interactive Demo](https://yourdomain.com/bovista/examples/point_cloud.html)**
    pub fn test_cube(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        size: u32,
    ) -> Self {
        let mut vertices = Vec::new();
        let step = 2.0 / size as f32;

        for i in 0..size {
            for j in 0..size {
                for k in 0..size {
                    let x = -1.0 + i as f32 * step;
                    let y = -1.0 + j as f32 * step;
                    let z = -1.0 + k as f32 * step;

                    // Color based on position (normalized to 0-1)
                    let color = [(x + 1.0) / 2.0, (y + 1.0) / 2.0, (z + 1.0) / 2.0];

                    vertices.push(PointVertex {
                        position: [x, y, z],
                        color,
                    });
                }
            }
        }

        Self::new(device, surface_format, camera_bind_group_layout, vertices)
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

impl Visual for PointsVisual {
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, _camera_info: &crate::visual::CameraInfo) {
        // Points don't need any per-frame preparation currently
        // In the future, this could handle dynamic updates to vertex data
    }

    fn render(&self, render_pass: &mut RenderPass) {
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..1);
    }

    fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }

    fn transform(&self) -> &Transform {
        &self.transform
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn name(&self) -> &str {
        &self.name
    }
}
