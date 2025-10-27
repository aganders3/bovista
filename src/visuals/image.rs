use crate::visual::{Transform, Visual};
use crate::visuals::image_strategy::{ChunkLoaderFn, ChunkedImageStrategy, ImageStrategy, SimpleImageStrategy};
use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;
use wgpu::RenderPass;

/// Slice plane defined by position and normal vector
#[derive(Debug, Clone, Copy)]
pub struct SlicePlane {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

impl SlicePlane {
    /// Create a plane perpendicular to Z axis (XY slice)
    pub fn xy(z: f32) -> Self {
        Self {
            position: [0.0, 0.0, z],
            normal: [0.0, 0.0, 1.0],
        }
    }

    /// Create a plane perpendicular to Y axis (XZ slice)
    pub fn xz(y: f32) -> Self {
        Self {
            position: [0.0, y, 0.0],
            normal: [0.0, 1.0, 0.0],
        }
    }

    /// Create a plane perpendicular to X axis (YZ slice)
    pub fn yz(x: f32) -> Self {
        Self {
            position: [x, 0.0, 0.0],
            normal: [1.0, 0.0, 0.0],
        }
    }

    /// Create an arbitrary slice plane with given position and normal
    /// The normal will be automatically normalized
    pub fn new(position: [f32; 3], normal: [f32; 3]) -> Self {
        let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
        Self {
            position,
            normal: [normal[0] / len, normal[1] / len, normal[2] / len],
        }
    }
}

impl Default for SlicePlane {
    fn default() -> Self {
        Self::xy(0.0)
    }
}

/// Standard slice orientations
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SliceOrientation {
    XY,
    XZ,
    YZ,
}

/// Vertex for image rendering (simple quad)
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ImageVertex {
    position: [f32; 2],
    texcoord: [f32; 2],
}

impl ImageVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x2, 1 => Float32x2];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ImageVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Uniforms for image rendering parameters
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ImageUniforms {
    contrast_min: f32,
    contrast_max: f32,
    _padding1: f32,
    _padding2: f32,
    plane_position: [f32; 3],
    _padding3: f32,
    plane_normal: [f32; 3],
    _padding4: f32,
    volume_size: [f32; 3],
    _padding5: f32,
}

/// Visual for rendering 2D images and 3D volume slices
pub struct ImageVisual {
    strategy: Box<dyn ImageStrategy>,

    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,
    _sampler: wgpu::Sampler,

    slice_plane: SlicePlane,
    contrast_limits: (f32, f32),

    transform: Transform,
    visible: bool,
    name: String,
}

impl ImageVisual {
    /// Create a new ImageVisual from in-memory 2D image data
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        data: &[u8],
        width: u32,
        height: u32,
    ) -> Self {
        Self::new_3d(device, queue, surface_format, camera_bind_group_layout, data, width, height, 1)
    }

    /// Create a new ImageVisual from in-memory 3D volume data
    pub fn new_3d(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        data: &[u8],
        width: u32,
        height: u32,
        depth: u32,
    ) -> Self {
        let strategy = Box::new(SimpleImageStrategy::new(
            device,
            queue,
            data,
            width,
            height,
            depth,
            wgpu::TextureFormat::R8Unorm,
        ));

        Self::from_strategy(
            device,
            queue,
            surface_format,
            camera_bind_group_layout,
            strategy,
            width,
            height,
            depth,
        )
    }

    /// Create an ImageVisual with a chunked/remote data loader
    pub fn with_loader(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        width: u32,
        height: u32,
        depth: u32,
        chunk_size: (u32, u32, u32),
        loader: ChunkLoaderFn,
    ) -> Self {
        let strategy = Box::new(ChunkedImageStrategy::new(
            device,
            width,
            height,
            depth,
            chunk_size,
            wgpu::TextureFormat::R8Unorm,
            loader,
        ));

        Self::from_strategy(
            device,
            queue,
            surface_format,
            camera_bind_group_layout,
            strategy,
            width,
            height,
            depth,
        )
    }

    /// Internal constructor from a strategy
    fn from_strategy(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        strategy: Box<dyn ImageStrategy>,
        width: u32,
        height: u32,
        depth: u32,
    ) -> Self {
        // Create a unit quad
        let vertices = [
            ImageVertex { position: [0.0, 0.0], texcoord: [0.0, 1.0] },
            ImageVertex { position: [1.0, 0.0], texcoord: [1.0, 1.0] },
            ImageVertex { position: [1.0, 1.0], texcoord: [1.0, 0.0] },
            ImageVertex { position: [0.0, 1.0], texcoord: [0.0, 0.0] },
        ];

        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Image Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Image Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Image Uniform Buffer"),
            size: std::mem::size_of::<ImageUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Image Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Determine texture view dimension based on depth
        let view_dimension = if depth > 1 {
            wgpu::TextureViewDimension::D3
        } else {
            wgpu::TextureViewDimension::D2
        };

        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Image Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension,
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

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Image Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(strategy.texture_view()),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
            ],
        });

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Image Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/image.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Image Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Image Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[ImageVertex::desc()],
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
                topology: wgpu::PrimitiveTopology::TriangleList,
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

        // Initialize with centered slice
        let slice_plane = SlicePlane::xy((depth as f32) / 2.0);

        // Auto-detect reasonable contrast limits (for now, use full range)
        let contrast_limits = (0.0, 1.0);

        // Initial uniform update
        let uniforms = ImageUniforms {
            contrast_min: contrast_limits.0,
            contrast_max: contrast_limits.1,
            _padding1: 0.0,
            _padding2: 0.0,
            plane_position: slice_plane.position,
            _padding3: 0.0,
            plane_normal: slice_plane.normal,
            _padding4: 0.0,
            volume_size: [width as f32, height as f32, depth as f32],
            _padding5: 0.0,
        };
        queue.write_buffer(&uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

        Self {
            strategy,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            render_pipeline,
            _sampler: sampler,
            slice_plane,
            contrast_limits,
            transform: Transform::identity(),
            visible: true,
            name: "Image".to_string(),
        }
    }

    /// Set the slice plane to a specific Z coordinate (XY plane)
    pub fn set_slice_z(&mut self, z: f32) {
        self.slice_plane = SlicePlane::xy(z);
    }

    /// Set the slice plane to a specific Y coordinate (XZ plane)
    pub fn set_slice_y(&mut self, y: f32) {
        self.slice_plane = SlicePlane::xz(y);
    }

    /// Set the slice plane to a specific X coordinate (YZ plane)
    pub fn set_slice_x(&mut self, x: f32) {
        self.slice_plane = SlicePlane::yz(x);
    }

    /// Set an arbitrary slice plane
    pub fn set_slice_plane(&mut self, plane: SlicePlane) {
        self.slice_plane = plane;
    }

    /// Set the slice orientation (convenience method)
    pub fn set_slice_orientation(&mut self, orientation: SliceOrientation, value: f32) {
        self.slice_plane = match orientation {
            SliceOrientation::XY => SlicePlane::xy(value),
            SliceOrientation::XZ => SlicePlane::xz(value),
            SliceOrientation::YZ => SlicePlane::yz(value),
        };
    }

    /// Get the current slice plane
    pub fn slice_plane(&self) -> SlicePlane {
        self.slice_plane
    }

    /// Set contrast limits for display
    pub fn set_contrast_limits(&mut self, min: f32, max: f32) {
        self.contrast_limits = (min, max);
    }

    /// Get the current contrast limits
    pub fn contrast_limits(&self) -> (f32, f32) {
        self.contrast_limits
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }
}

impl Visual for ImageVisual {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Update strategy (load chunks if needed)
        self.strategy.prepare(device, queue, None);

        // Update uniforms
        let dims = self.strategy.dimensions();
        let uniforms = ImageUniforms {
            contrast_min: self.contrast_limits.0,
            contrast_max: self.contrast_limits.1,
            _padding1: 0.0,
            _padding2: 0.0,
            plane_position: self.slice_plane.position,
            _padding3: 0.0,
            plane_normal: self.slice_plane.normal,
            _padding4: 0.0,
            volume_size: [dims.0 as f32, dims.1 as f32, dims.2 as f32],
            _padding5: 0.0,
        };

        // Debug output (only print occasionally to avoid spam)
        static mut FRAME_COUNT: u32 = 0;
        unsafe {
            if FRAME_COUNT % 60 == 0 {
                println!("ImageVisual uniforms: pos={:?}, normal={:?}, size={:?}, contrast=[{}, {}]",
                         uniforms.plane_position, uniforms.plane_normal, uniforms.volume_size,
                         uniforms.contrast_min, uniforms.contrast_max);
            }
            FRAME_COUNT += 1;
        }

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    fn render(&self, render_pass: &mut RenderPass) {
        if !self.strategy.is_ready() {
            return; // Don't render if data isn't loaded yet
        }

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..6, 0, 0..1);
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
