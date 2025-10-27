use crate::visual::{Transform, Visual};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use wgpu::RenderPass;

/// A vertex for line rendering with position and color
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl LineVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Visual for rendering lines
pub struct LinesVisual {
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    render_pipeline: wgpu::RenderPipeline,
    transform: Transform,
    visible: bool,
    name: String,
}

impl LinesVisual {
    /// Create a new LinesVisual from vertex pairs
    /// Each pair of vertices defines a line segment
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        vertices: Vec<LineVertex>,
    ) -> Self {
        Self::with_shader(
            device,
            surface_format,
            camera_bind_group_layout,
            vertices,
            include_str!("../shaders/lines.wgsl"),
        )
    }

    /// Create a new LinesVisual with a custom shader
    pub fn with_shader(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        vertices: Vec<LineVertex>,
        shader_source: &str,
    ) -> Self {
        let vertex_count = vertices.len() as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Lines Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lines Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Lines Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Lines Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[LineVertex::desc()],
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
                topology: wgpu::PrimitiveTopology::LineList,
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
            name: "Lines".to_string(),
        }
    }

    /// Create a test wireframe cube
    pub fn test_cube(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let color = [1.0, 1.0, 1.0]; // White lines

        // Define the 8 corners of a cube
        let corners = [
            [-1.0, -1.0, -1.0], // 0
            [1.0, -1.0, -1.0],  // 1
            [1.0, 1.0, -1.0],   // 2
            [-1.0, 1.0, -1.0],  // 3
            [-1.0, -1.0, 1.0],  // 4
            [1.0, -1.0, 1.0],   // 5
            [1.0, 1.0, 1.0],    // 6
            [-1.0, 1.0, 1.0],   // 7
        ];

        // Define the 12 edges of a cube (as pairs of corner indices)
        let edges = [
            (0, 1), (1, 2), (2, 3), (3, 0), // Bottom face
            (4, 5), (5, 6), (6, 7), (7, 4), // Top face
            (0, 4), (1, 5), (2, 6), (3, 7), // Vertical edges
        ];

        let mut vertices = Vec::new();
        for (start, end) in edges.iter() {
            vertices.push(LineVertex {
                position: corners[*start],
                color,
            });
            vertices.push(LineVertex {
                position: corners[*end],
                color,
            });
        }

        Self::new(device, surface_format, camera_bind_group_layout, vertices)
    }

    /// Create a 3D axis helper (XYZ axes)
    pub fn axis_helper(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        length: f32,
    ) -> Self {
        let vertices = vec![
            // X axis (red)
            LineVertex {
                position: [0.0, 0.0, 0.0],
                color: [1.0, 0.0, 0.0],
            },
            LineVertex {
                position: [length, 0.0, 0.0],
                color: [1.0, 0.0, 0.0],
            },
            // Y axis (green)
            LineVertex {
                position: [0.0, 0.0, 0.0],
                color: [0.0, 1.0, 0.0],
            },
            LineVertex {
                position: [0.0, length, 0.0],
                color: [0.0, 1.0, 0.0],
            },
            // Z axis (blue)
            LineVertex {
                position: [0.0, 0.0, 0.0],
                color: [0.0, 0.0, 1.0],
            },
            LineVertex {
                position: [0.0, 0.0, length],
                color: [0.0, 0.0, 1.0],
            },
        ];

        Self::new(device, surface_format, camera_bind_group_layout, vertices)
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

impl Visual for LinesVisual {
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {
        // Lines don't need any per-frame preparation currently
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
