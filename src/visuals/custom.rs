use crate::visual::{Transform, VertexBufferLayout, Visual};
use wgpu::util::DeviceExt;
use wgpu::RenderPass;

/// A custom visual that allows users to provide their own WGSL shader
///
/// # Example
///
/// ```ignore
/// let shader = r#"
///     struct Uniforms {
///         view_proj: mat4x4<f32>,
///     }
///
///     @group(0) @binding(0)
///     var<uniform> uniforms: Uniforms;
///
///     struct VertexInput {
///         @location(0) position: vec3<f32>,
///         @location(1) color: vec4<f32>,
///     }
///
///     struct VertexOutput {
///         @builtin(position) clip_position: vec4<f32>,
///         @location(0) color: vec4<f32>,
///     }
///
///     @vertex
///     fn vs_main(input: VertexInput) -> VertexOutput {
///         var output: VertexOutput;
///         output.clip_position = uniforms.view_proj * vec4<f32>(input.position, 1.0);
///         output.color = input.color;
///         return output;
///     }
///
///     @fragment
///     fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
///         return input.color;
///     }
/// "#;
///
/// let visual = CustomVisual::new(
///     device,
///     surface_format,
///     camera_bind_group_layout,
///     shader,
///     vertex_data,
///     vertex_layout,
///     wgpu::PrimitiveTopology::TriangleList,
/// );
/// ```
///
/// # Shader Requirements
///
/// Your shader MUST:
/// - Have a camera uniform at `@group(0) @binding(0)` with structure:
///   ```wgsl
///   struct Uniforms {
///       view_proj: mat4x4<f32>,
///   }
///   ```
/// - Have a vertex entry point named `vs_main`
/// - Have a fragment entry point named `fs_main`
/// - Match the vertex layout you provide
///
/// # Cross-Platform Support
///
/// Custom shaders work on **both native and WebAssembly/browser** targets!
/// WGSL compilation happens at runtime via the WebGPU API, so your custom
/// shaders will work in the browser just as well as native applications.
pub struct CustomVisual {
    vertex_buffer: wgpu::Buffer,
    vertex_count: u32,
    render_pipeline: wgpu::RenderPipeline,
    transform: Transform,
    visible: bool,
    name: String,
}

impl CustomVisual {
    /// Create a new custom visual with a user-provided WGSL shader
    ///
    /// # Arguments
    ///
    /// * `device` - The GPU device
    /// * `surface_format` - The surface format for rendering
    /// * `camera_bind_group_layout` - The camera bind group layout (from Renderer)
    /// * `shader_source` - WGSL shader source code as a string
    /// * `vertex_data` - Raw vertex data as bytes
    /// * `vertex_layout` - Description of the vertex buffer layout
    /// * `topology` - Primitive topology (PointList, LineList, TriangleList, etc.)
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
        vertex_data: &[u8],
        vertex_layout: VertexBufferLayout,
        topology: wgpu::PrimitiveTopology,
    ) -> Result<Self, String> {
        let vertex_count = (vertex_data.len() as u64 / vertex_layout.stride) as u32;

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Custom Visual Vertex Buffer"),
            contents: vertex_data,
            usage: wgpu::BufferUsages::VERTEX,
        });

        // Compile the user-provided shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Custom Visual Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Custom Visual Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout],
            push_constant_ranges: &[],
        });

        let wgpu_vertex_layout = vertex_layout.to_wgpu();

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Custom Visual Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[wgpu_vertex_layout],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology,
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

        Ok(Self {
            vertex_buffer,
            vertex_count,
            render_pipeline,
            transform: Transform::identity(),
            visible: true,
            name: "Custom Visual".to_string(),
        })
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

impl Visual for CustomVisual {
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue, _camera_info: &crate::visual::CameraInfo) {
        // Custom visuals don't need any per-frame preparation currently
        // In the future, this could handle dynamic uniform updates
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
