// Force recompilation when any shader file changes.
const _SHADER_HASH: &str = env!("SHADER_HASH");

use crate::visual::{Transform, Visual};
use crate::visuals::image_strategy::{ImageStrategy, SimpleData, TiledData, VirtualTextureData};
use crate::visuals::tile::{TileUniforms, TileVertex, VTUniforms, VTLodInfo, VT_MAX_LODS};
use crate::visuals::image_strategy::LodLevelConfig;
use std::collections::HashMap;
use std::sync::Arc;
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

/// Visual for rendering 2D images and 3D volume slices
pub struct ImageVisual {
    strategy: ImageStrategy,

    // Shared resources for tile rendering (Simple / Tiled strategies)
    bind_group_layout: wgpu::BindGroupLayout,
    render_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,

    // Cache for per-tile bind groups (keyed by texture pointer)
    bind_group_cache: HashMap<usize, Arc<wgpu::BindGroup>>,

    // Uniform buffers (one per tile, created on demand)
    uniform_buffer_cache: HashMap<usize, wgpu::Buffer>,

    // Colormap LUT (group 2) — 256-entry 1D RGBA texture (shared across strategies)
    colormap_texture: wgpu::Texture,
    colormap_bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    colormap_bind_group_layout: wgpu::BindGroupLayout,
    pending_colormap: Option<Vec<u8>>,

    // Virtual-texture pipeline resources (VirtualTexture strategy only)
    vt_render_pipeline: Option<wgpu::RenderPipeline>,
    vt_bind_group: Option<wgpu::BindGroup>,
    vt_uniform_buffer: Option<wgpu::Buffer>,
    vt_vertex_buffer: Option<wgpu::Buffer>,
    vt_index_buffer: Option<wgpu::Buffer>,
    vt_index_count: u32,

    slice_plane: SlicePlane,
    contrast_limits: (f32, f32),
    debug_mode: bool,
    frame_number: u64,

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
        let strategy = ImageStrategy::Simple(SimpleData::new(
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

    /// Create an ImageVisual with virtual-texture multiscale rendering.
    ///
    /// Like `from_chunked` but renders with a single draw call via a physical atlas +
    /// page table, with shader-side LOD fallback.  The tile format is R16Float.
    ///
    /// # Arguments
    /// * `lod_levels` - Configuration for each LOD level (high-res to low-res)
    /// * `max_tiles` - Atlas capacity (number of simultaneously resident tiles)
    /// * `loader` - Callback invoked when a tile is needed
    pub fn from_virtual_tiled(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: crate::visuals::tile::TileLoaderFn,
    ) -> Self {
        let (depth, height, width) = if !lod_levels.is_empty() {
            lod_levels[0].volume_size
        } else {
            (1, 1, 1)
        };

        let strategy = ImageStrategy::VirtualTexture(VirtualTextureData::new(
            device,
            lod_levels,
            max_tiles,
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

    /// Create an ImageVisual with multi-resolution chunked loading
    ///
    /// This creates an image visual that loads chunks on-demand with automatic LOD selection.
    ///
    /// # Arguments
    /// * `device` - GPU device
    /// * `queue` - GPU queue
    /// * `surface_format` - Surface format for rendering
    /// * `camera_bind_group_layout` - Camera bind group layout
    /// * `lod_levels` - Configuration for each LOD level (high-res to low-res)
    /// * `max_tiles` - Maximum number of chunks to keep in memory
    /// * `loader` - Callback function to load chunk data on-demand
    pub fn from_chunked(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<crate::visuals::image_strategy::LodLevelConfig>,
        max_tiles: usize,
        loader: crate::visuals::tile::TileLoaderFn,
    ) -> Self {
        let strategy = ImageStrategy::Tiled(TiledData::new(
            device,
            lod_levels.clone(),
            max_tiles,
            loader,
        ));

        // Get dimensions from first LOD level
        let (depth, height, width) = if !lod_levels.is_empty() {
            lod_levels[0].volume_size
        } else {
            (1, 1, 1)
        };

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
        strategy: ImageStrategy,
        _width: u32,
        _height: u32,
        depth: u32,
    ) -> Self {
        // Create shared sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Tile Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,  // Avoid edge artifacts between chunks
            min_filter: wgpu::FilterMode::Nearest,  // Avoid edge artifacts between chunks
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create bind group layout for tiles
        // Layout matches tile.wgsl:
        //   @group(1) @binding(0) var tile_texture: texture_3d<f32>;
        //   @group(1) @binding(1) var tile_sampler: sampler;
        //   @group(1) @binding(2) var<uniform> tile: TileUniforms;
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Tile Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        // Load tile shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Tile Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tile.wgsl").into()),
        });

        // Colormap LUT bind group layout (group 2) — shared across all pipelines
        let colormap_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Colormap Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D1,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Tile Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout, &colormap_bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Tile Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[TileVertex::desc()],
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
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: true,  // Write depth for proper occlusion
                depth_compare: wgpu::CompareFunction::LessEqual,  // Allow coplanar tiles, stencil handles priority
                stencil: wgpu::StencilState {
                    front: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::NotEqual,  // Only draw where unpainted
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Replace,  // Mark as painted
                    },
                    back: wgpu::StencilFaceState {
                        compare: wgpu::CompareFunction::NotEqual,
                        fail_op: wgpu::StencilOperation::Keep,
                        depth_fail_op: wgpu::StencilOperation::Keep,
                        pass_op: wgpu::StencilOperation::Replace,
                    },
                    read_mask: 0xff,
                    write_mask: 0xff,
                },
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

        // Build default grayscale colormap (256 RGBA entries: r=g=b=i, a=255)
        let colormap_data: Vec<u8> = (0u32..256)
            .flat_map(|i| { let v = i as u8; [v, v, v, 255] })
            .collect();
        let colormap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Colormap Texture"),
            size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &colormap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &colormap_data,
            wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: None },
            wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
        );
        let colormap_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Colormap Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let colormap_view = colormap_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let colormap_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Colormap Bind Group"),
            layout: &colormap_bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&colormap_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&colormap_sampler) },
            ],
        });

        // Initialize with centered slice
        let slice_plane = SlicePlane::xy((depth as f32) / 2.0);

        let contrast_limits = (0.0, 1.0);

        // ── Virtual-texture pipeline (only for VirtualTexture strategy) ──────
        let (vt_render_pipeline, vt_bind_group, vt_uniform_buffer) =
            if let ImageStrategy::VirtualTexture(ref vt) = strategy {
                let vt_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                    label: Some("VT Bind Group Layout"),
                    entries: &[
                        // binding 0: physical atlas
                        wgpu::BindGroupLayoutEntry {
                            binding: 0,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                                view_dimension: wgpu::TextureViewDimension::D3,
                                multisampled: false,
                            },
                            count: None,
                        },
                        // binding 1: atlas sampler
                        wgpu::BindGroupLayoutEntry {
                            binding: 1,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                            count: None,
                        },
                        // binding 2: page table (integer texture array)
                        wgpu::BindGroupLayoutEntry {
                            binding: 2,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Texture {
                                sample_type: wgpu::TextureSampleType::Uint,
                                view_dimension: wgpu::TextureViewDimension::D2Array,
                                multisampled: false,
                            },
                            count: None,
                        },
                        // binding 3: VT uniforms
                        wgpu::BindGroupLayoutEntry {
                            binding: 3,
                            visibility: wgpu::ShaderStages::FRAGMENT,
                            ty: wgpu::BindingType::Buffer {
                                ty: wgpu::BufferBindingType::Uniform,
                                has_dynamic_offset: false,
                                min_binding_size: None,
                            },
                            count: None,
                        },
                    ],
                });

                let vt_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("VT Uniform Buffer"),
                    size: std::mem::size_of::<VTUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });

                let vt_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                    label: Some("VT Bind Group"),
                    layout: &vt_bgl,
                    entries: &[
                        wgpu::BindGroupEntry {
                            binding: 0,
                            resource: wgpu::BindingResource::TextureView(&vt.atlas_texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 1,
                            resource: wgpu::BindingResource::Sampler(&sampler),
                        },
                        wgpu::BindGroupEntry {
                            binding: 2,
                            resource: wgpu::BindingResource::TextureView(&vt.page_table.texture_view),
                        },
                        wgpu::BindGroupEntry {
                            binding: 3,
                            resource: vt_uniform_buffer.as_entire_binding(),
                        },
                    ],
                });

                let vt_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
                    label: Some("VT Shader"),
                    source: wgpu::ShaderSource::Wgsl(
                        include_str!("../shaders/virtual_tile.wgsl").into(),
                    ),
                });

                let vt_pipeline_layout =
                    device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
                        label: Some("VT Pipeline Layout"),
                        bind_group_layouts: &[
                            camera_bind_group_layout,
                            &vt_bgl,
                            &colormap_bind_group_layout,
                        ],
                        push_constant_ranges: &[],
                    });

                let vt_pipeline =
                    device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                        label: Some("VT Render Pipeline"),
                        layout: Some(&vt_pipeline_layout),
                        vertex: wgpu::VertexState {
                            module: &vt_shader,
                            entry_point: Some("vs_main"),
                            buffers: &[TileVertex::desc()],
                            compilation_options: Default::default(),
                        },
                        fragment: Some(wgpu::FragmentState {
                            module: &vt_shader,
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
                            format: wgpu::TextureFormat::Depth24PlusStencil8,
                            depth_write_enabled: true,
                            depth_compare: wgpu::CompareFunction::LessEqual,
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

                (Some(vt_pipeline), Some(vt_bind_group), Some(vt_uniform_buffer))
            } else {
                (None, None, None)
            };

        Self {
            strategy,
            bind_group_layout,
            render_pipeline,
            sampler,
            bind_group_cache: HashMap::new(),
            uniform_buffer_cache: HashMap::new(),
            colormap_texture,
            colormap_bind_group,
            colormap_bind_group_layout,
            pending_colormap: None,
            vt_render_pipeline,
            vt_bind_group,
            vt_uniform_buffer,
            vt_vertex_buffer: None,
            vt_index_buffer: None,
            vt_index_count: 0,
            slice_plane,
            contrast_limits,
            debug_mode: false,
            frame_number: 0,
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

    /// Set a colormap LUT from 256 RGBA entries (1024 bytes, values in 0..=255).
    /// Pass an empty slice to reset to the default grayscale map.
    /// The upload is deferred to the next `prepare()` call.
    pub fn set_colormap(&mut self, rgba: &[u8]) {
        self.pending_colormap = Some(if rgba.len() == 1024 {
            rgba.to_vec()
        } else {
            (0u32..256).flat_map(|i| { let v = i as u8; [v, v, v, 255] }).collect()
        });
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Enable or disable debug visualization mode
    pub fn set_debug_mode(&mut self, debug: bool) {
        self.debug_mode = debug;
        self.strategy.set_debug_mode(debug);
    }

    /// Set LOD bias for automatic LOD selection
    ///
    /// Negative values prefer higher resolution, positive prefer lower resolution.
    /// Only has effect when using TiledStrategy (no-op for SimpleStrategy).
    pub fn set_lod_bias(&mut self, bias: f32) {
        self.strategy.set_lod_bias(bias);
    }

    /// Get statistics (loaded tiles, visible tiles)
    pub fn get_stats(&self) -> (usize, usize) {
        self.strategy.get_stats()
    }

    /// Provide chunk data for a requested tile (used with from_chunked)
    ///
    /// This method should be called when chunk data is ready for a tile that was requested.
    /// Only has effect when using TiledStrategy (no-op for SimpleStrategy).
    pub fn set_chunk_data(&mut self, lod_level: usize, z: u32, y: u32, x: u32, data: crate::visuals::tile::TileData) {
        self.strategy.set_chunk_data(lod_level, z, y, x, data);
    }

    /// Get the pending chunks queue (for PyChunkedImageStrategy)
    ///
    /// Returns None if not using TiledStrategy.
    pub fn pending_chunks(&self) -> Option<crate::visuals::image_strategy::PendingChunks> {
        self.strategy.pending_chunks()
    }
}

impl Visual for ImageVisual {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &crate::visual::CameraInfo) {
        // Upload pending colormap if set
        if let Some(data) = self.pending_colormap.take() {
            queue.write_texture(
                wgpu::ImageCopyTexture {
                    texture: &self.colormap_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::ImageDataLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: None },
                wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            );
        }

        // Update frame counter
        self.frame_number += 1;

        // Update strategy (determine visible tiles, load data if needed)
        self.strategy.prepare(
            device,
            queue,
            &self.slice_plane,
            self.frame_number,
            camera_info,
        );

        // ── Virtual-texture path: update uniforms and slice geometry ──────────
        if let ImageStrategy::VirtualTexture(ref vt) = self.strategy {
            // Update VT uniform buffer.
            if let Some(ref ub) = self.vt_uniform_buffer {
                let mut lods = [VTLodInfo {
                    grid_dims: [1, 1, 1], _pad: 0,
                    tile_scale: [1.0, 1.0, 1.0], _pad2: 0.0,
                    data_scale: [1.0, 1.0, 1.0], _pad3: 0.0,
                }; VT_MAX_LODS];
                // Atlas slot size is fixed at LOD 0's tile dimensions.
                let (atlas_tile_d, atlas_tile_h, atlas_tile_w) = vt.levels[0].tile_size;
                for (i, level) in vt.levels.iter().take(VT_MAX_LODS).enumerate() {
                    let (gz, gy, gx) = level.grid_size();
                    let (vol_z, vol_y, vol_x) = level.volume_size;
                    let (tile_d, tile_h, tile_w) = level.tile_size;
                    lods[i] = VTLodInfo {
                        grid_dims: [gx, gy, gz],
                        _pad: 0,
                        tile_scale: [
                            tile_w as f32 / vol_x as f32,
                            tile_h as f32 / vol_y as f32,
                            tile_d as f32 / vol_z as f32,
                        ],
                        _pad2: 0.0,
                        data_scale: [
                            tile_w as f32 / atlas_tile_w as f32,
                            tile_h as f32 / atlas_tile_h as f32,
                            tile_d as f32 / atlas_tile_d as f32,
                        ],
                        _pad3: 0.0,
                    };
                }
                let uniforms = VTUniforms {
                    atlas_cols: vt.atlas_cols,
                    atlas_rows: vt.atlas_rows,
                    atlas_tile_pitch_x: 1.0 / vt.atlas_cols as f32,
                    atlas_tile_pitch_y: 1.0 / vt.atlas_rows as f32,
                    atlas_tile_pitch_z: 1.0 / vt.atlas_layers as f32,
                    lod_count: vt.levels.len().min(VT_MAX_LODS) as u32,
                    contrast_min: self.contrast_limits.0,
                    contrast_max: self.contrast_limits.1,
                    debug_mode: self.debug_mode as u32,
                    page_table_width: vt.page_table.width,
                    target_lod: vt.cached_ideal_lod as u32,
                    _pad_c: 0,
                    lods,
                };
                queue.write_buffer(ub, 0, bytemuck::cast_slice(&[uniforms]));
            }

            // Rebuild slice-plane intersection geometry for the full volume.
            let lod0 = &vt.levels[0];
            let vol_min = glam::Vec3::new(
                lod0.translation.2,
                lod0.translation.1,
                lod0.translation.0,
            );
            let vol_max = vol_min + glam::Vec3::new(
                lod0.volume_size.2 as f32 * lod0.voxel_size.2,
                lod0.volume_size.1 as f32 * lod0.voxel_size.1,
                lod0.volume_size.0 as f32 * lod0.voxel_size.0,
            );
            let vol_aabb = crate::visuals::tile::AABB::new(vol_min, vol_max);
            let tile_plane = crate::visuals::tile::SlicePlane {
                position: self.slice_plane.position,
                normal: self.slice_plane.normal,
            };

            if let Some((vertices, indices)) =
                crate::visuals::tile::compute_plane_aabb_intersection(&tile_plane, &vol_aabb)
            {
                use wgpu::util::DeviceExt;
                self.vt_vertex_buffer =
                    Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("VT Vertex Buffer"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX,
                    }));
                self.vt_index_buffer =
                    Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("VT Index Buffer"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX,
                    }));
                self.vt_index_count = indices.len() as u32;
            } else {
                self.vt_vertex_buffer = None;
                self.vt_index_buffer = None;
                self.vt_index_count = 0;
            }
            return; // VT prepare is done; skip the per-tile bind-group path below.
        }

        // ── Tiled / Simple path: per-tile bind groups ────────────────────────
        let dims = self.strategy.dimensions();
        let contrast_min = self.contrast_limits.0;
        let contrast_max = self.contrast_limits.1;
        let debug_mode = if self.debug_mode { 1.0 } else { 0.0 };
        let plane_position = self.slice_plane.position;
        let plane_normal = self.slice_plane.normal;

        let tile_info: Vec<_> = self.strategy.tiles().iter()
            .map(|tile| (tile.debug_color.unwrap_or([1.0, 1.0, 1.0]), Arc::clone(&tile.texture), Arc::clone(&tile.texture_view)))
            .collect();

        // First pass: ensure all uniform buffers exist
        for idx in 0..tile_info.len() {
            if !self.uniform_buffer_cache.contains_key(&idx) {
                let buffer = device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("Tile Uniform Buffer"),
                    size: std::mem::size_of::<TileUniforms>() as u64,
                    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                });
                self.uniform_buffer_cache.insert(idx, buffer);
            }
        }

        // Second pass: update uniforms and create bind groups
        for (idx, (debug_color, texture, texture_view)) in tile_info.iter().enumerate() {
            let uniform_buffer = self.uniform_buffer_cache.get(&idx).unwrap();

            let uniforms = TileUniforms {
                contrast_min,
                contrast_max,
                debug_mode,
                _padding1: 0.0,
                plane_position,
                _padding2: 0.0,
                plane_normal,
                _padding3: 0.0,
                volume_size: [dims.2 as f32, dims.1 as f32, dims.0 as f32],
                _padding4: 0.0,
                debug_color: *debug_color,
                _padding5: 0.0,
            };

            queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

            let texture_ptr = Arc::as_ptr(texture) as usize;
            let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("Tile Bind Group"),
                layout: &self.bind_group_layout,
                entries: &[
                    wgpu::BindGroupEntry {
                        binding: 0,
                        resource: wgpu::BindingResource::TextureView(texture_view),
                    },
                    wgpu::BindGroupEntry {
                        binding: 1,
                        resource: wgpu::BindingResource::Sampler(&self.sampler),
                    },
                    wgpu::BindGroupEntry {
                        binding: 2,
                        resource: uniform_buffer.as_entire_binding(),
                    },
                ],
            });
            self.bind_group_cache.insert(texture_ptr, Arc::new(bind_group));
        }
    }

    fn render(&self, render_pass: &mut RenderPass) {
        if !self.strategy.is_ready() {
            return;
        }

        // ── Virtual-texture path: single draw call ────────────────────────────
        if matches!(self.strategy, ImageStrategy::VirtualTexture(_)) {
            if let (Some(pipeline), Some(bg), Some(vb), Some(ib)) = (
                &self.vt_render_pipeline,
                &self.vt_bind_group,
                &self.vt_vertex_buffer,
                &self.vt_index_buffer,
            ) {
                if self.vt_index_count == 0 {
                    return;
                }
                render_pass.set_pipeline(pipeline);
                render_pass.set_bind_group(1, bg, &[]);
                render_pass.set_bind_group(2, &self.colormap_bind_group, &[]);
                render_pass.set_vertex_buffer(0, vb.slice(..));
                render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint16);
                render_pass.draw_indexed(0..self.vt_index_count, 0, 0..1);
            }
            return;
        }

        // ── Tiled / Simple path ───────────────────────────────────────────────
        render_pass.set_pipeline(&self.render_pipeline);

        // Get ideal LOD for sorting
        let ideal_lod = self.strategy.get_ideal_lod();

        // Collect and sort tiles by distance from ideal LOD
        // Stencil uses NotEqual with ref=1: first tile painted wins
        let mut tiles_with_distance: Vec<_> = self.strategy.tiles().iter()
            .filter(|tile| tile.vertex_count > 0)
            .map(|tile| {
                let distance = (tile.lod_level as i32 - ideal_lod as i32).abs();
                let prefer_higher_res = if tile.lod_level < ideal_lod { 0 } else { 1 };
                (tile, distance, prefer_higher_res)
            })
            .collect();

        // Sort by: distance first, then prefer higher-res (lower LOD) for ties
        tiles_with_distance.sort_by_key(|(_, dist, pref)| (*dist, *pref));

        // Stencil reference = 1 (buffer cleared to 0, NotEqual ensures first painted wins)
        render_pass.set_stencil_reference(1);

        // Set colormap LUT once for all tiles
        render_pass.set_bind_group(2, &self.colormap_bind_group, &[]);

        for (tile, _, _) in tiles_with_distance {
            // Note: uniform buffer is already bound in the bind group created during prepare()

            // Get or create bind group (uses cache)
            let texture_ptr = Arc::as_ptr(&tile.texture) as usize;

            // Check if we have a cached bind group
            if !self.bind_group_cache.contains_key(&texture_ptr) {
                // This shouldn't happen if prepare() was called, but handle gracefully
                continue;
            }

            let bind_group = self.bind_group_cache.get(&texture_ptr).unwrap();

            render_pass.set_bind_group(1, bind_group.as_ref(), &[]);

            // Set vertex buffer if present
            if let Some(ref vertex_buffer) = tile.vertex_buffer {
                render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));

                // Draw the tile geometry
                if let Some(ref index_buffer) = tile.index_buffer {
                    // Use indexed rendering if available
                    render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
                    render_pass.draw_indexed(0..tile.vertex_count, 0, 0..1);
                } else {
                    // Use non-indexed rendering
                    render_pass.draw(0..tile.vertex_count, 0..1);
                }
            }
        }
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
