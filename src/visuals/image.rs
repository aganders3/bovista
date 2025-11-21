use crate::visual::{Transform, Visual};
use crate::visuals::image_strategy::{ImageStrategy, SimpleData, TiledData};
use crate::visuals::tile::{TileUniforms, TileVertex};
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

    // Shared resources for tile rendering
    bind_group_layout: wgpu::BindGroupLayout,
    render_pipeline: wgpu::RenderPipeline,
    sampler: wgpu::Sampler,

    // Cache for per-tile bind groups (keyed by texture pointer)
    bind_group_cache: HashMap<usize, Arc<wgpu::BindGroup>>,

    // Uniform buffers (one per tile, created on demand)
    uniform_buffer_cache: HashMap<usize, wgpu::Buffer>,

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
        _queue: &wgpu::Queue,
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

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Tile Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout],
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

        // Initialize with centered slice
        let slice_plane = SlicePlane::xy((depth as f32) / 2.0);

        // Auto-detect reasonable contrast limits (for now, use full range)
        let contrast_limits = (0.0, 1.0);

        Self {
            strategy,
            bind_group_layout,
            render_pipeline,
            sampler,
            bind_group_cache: HashMap::new(),
            uniform_buffer_cache: HashMap::new(),
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

        // Get dimensions and extract values to avoid borrow issues
        let dims = self.strategy.dimensions();
        let contrast_min = self.contrast_limits.0;
        let contrast_max = self.contrast_limits.1;
        let debug_mode = if self.debug_mode { 1.0 } else { 0.0 };
        let plane_position = self.slice_plane.position;
        let plane_normal = self.slice_plane.normal;

        // Collect tile information (to avoid holding immutable borrow of self.strategy)
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
            // Get uniform buffer (we know it exists from first pass)
            let uniform_buffer = self.uniform_buffer_cache.get(&idx).unwrap();

            // Create uniforms for this tile
            let uniforms = TileUniforms {
                contrast_min,
                contrast_max,
                debug_mode,
                _padding1: 0.0,
                plane_position,
                _padding2: 0.0,
                plane_normal,
                _padding3: 0.0,
                volume_size: [dims.2 as f32, dims.1 as f32, dims.0 as f32], // ZYX order
                _padding4: 0.0,
                debug_color: *debug_color,
                _padding5: 0.0,
            };

            queue.write_buffer(uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

            // Always recreate bind group to avoid stale cache entries when textures are reused
            // (Arc may reuse same memory address after tile eviction)
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
            return; // Don't render if data isn't loaded yet
        }

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
