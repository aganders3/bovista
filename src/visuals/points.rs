use crate::visual::{Transform, Visual};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use wgpu::RenderPass;

/// A vertex in a point cloud with position and color
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PointVertex {
    pub position: [f32; 3],
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

/// Visual for rendering point clouds
pub struct PointsVisual {
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    render_pipeline: wgpu::RenderPipeline,
    transform: Transform,
    visible: bool,
    name: String,
}

impl PointsVisual {
    /// Create a new PointsVisual from vertex data
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
                format: wgpu::TextureFormat::Depth32Float,
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

    /// Create a simple test point cloud (a cube of points)
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
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {
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
