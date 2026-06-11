// Force recompilation when any shader file changes.
const _SHADER_HASH: &str = env!("SHADER_HASH");

use crate::visual::{Transform, Visual};
use crate::visuals::virtual_texture::{LodLevelConfig, PendingChunks, VirtualTextureData};
use crate::visuals::gpu_structs::{TileVertex, VTUniforms, VTLodInfo, VT_MAX_LODS};
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

/// Visual for rendering volume slices via the virtual-texture pipeline.
pub struct Image {
    vt: VirtualTextureData,

    // Colormap LUT (group 2) — 256-entry 1D RGBA texture
    colormap_texture: wgpu::Texture,
    colormap_bind_group: wgpu::BindGroup,
    #[allow(dead_code)]
    colormap_bind_group_layout: wgpu::BindGroupLayout,
    pending_colormap: Option<Vec<u8>>,

    // Virtual-texture pipeline resources
    vt_render_pipeline: wgpu::RenderPipeline,
    vt_bind_group: wgpu::BindGroup,
    vt_uniform_buffer: wgpu::Buffer,
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

impl Image {
    /// Create an Image with virtual-texture multiscale rendering.
    ///
    /// # Arguments
    /// * `lod_levels` - Configuration for each LOD level (high-res to low-res)
    /// * `max_tiles`  - Total atlas capacity across all atlases
    /// * `atlas_count` - Number of physical atlas textures (1..=MAX_ATLAS_COUNT)
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        atlas_count: usize,
    ) -> Self {
        let (depth, _height, _width) = if !lod_levels.is_empty() {
            lod_levels[0].volume_size
        } else {
            (1, 1, 1)
        };

        let vt = VirtualTextureData::new(device, lod_levels, max_tiles, atlas_count);

        // Atlas sampler (linear for smooth interpolation across tile boundaries)
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("VT Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Colormap LUT bind group layout (group 2)
        let colormap_bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Colormap Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
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

        // Build default grayscale colormap (256 RGBA entries)
        let colormap_data: Vec<u8> = (0u32..256)
            .flat_map(|i| { let v = i as u8; [v, v, v, 255] })
            .collect();
        let colormap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Colormap Texture"),
            size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            // 2D (not 1D) — see volume.rs for the naga/GL 1D-Rgba8Unorm bug.
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &colormap_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &colormap_data,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: None },
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

        // VT bind group layout (group 1): atlas[0..=3], atlas_sampler, page_table, vt_uniforms.
        // All 4 atlas textures are always bound; entries past atlas_count are
        // 1×1×1 dummies created inside VirtualTextureData.
        use crate::visuals::virtual_texture::MAX_ATLAS_COUNT;
        let atlas_3d_entry = |binding: u32| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
                view_dimension: wgpu::TextureViewDimension::D3,
                multisampled: false,
            },
            count: None,
        };
        let vt_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("VT Bind Group Layout"),
            entries: &[
                atlas_3d_entry(0),
                atlas_3d_entry(1),
                atlas_3d_entry(2),
                atlas_3d_entry(3),
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 6,
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

        assert_eq!(vt.atlas_views.len(), MAX_ATLAS_COUNT);
        let vt_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("VT Bind Group"),
            layout: &vt_bgl,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&vt.atlas_views[0]) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&vt.atlas_views[1]) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&vt.atlas_views[2]) },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(&vt.atlas_views[3]) },
                wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(&sampler) },
                wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(&vt.page_table.texture_view) },
                wgpu::BindGroupEntry { binding: 6, resource: vt_uniform_buffer.as_entire_binding() },
            ],
        });

        let vt_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("VT Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/virtual_tile.wgsl").into(),
            ),
        });

        let vt_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("VT Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &vt_bgl, &colormap_bind_group_layout],
            push_constant_ranges: &[],
        });

        let vt_render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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

        let slice_plane = SlicePlane::xy((depth as f32) / 2.0);

        Self {
            vt,
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
            contrast_limits: (0.0, 1.0),
            debug_mode: false,
            frame_number: 0,
            transform: Transform::identity(),
            visible: true,
            name: "Image".to_string(),
        }
    }

    /// Convenience alias kept for compatibility with existing bindings.
    pub fn from_virtual_tiled(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        atlas_count: usize,
    ) -> Self {
        Self::new(device, queue, surface_format, camera_bind_group_layout, lod_levels, max_tiles, atlas_count)
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

    /// Enable or disable debug LOD tinting (green=LOD0, red=coarsest)
    pub fn set_debug_mode(&mut self, debug: bool) {
        self.debug_mode = debug;
    }

    /// Set LOD bias. Positive = prefer higher resolution (finer), negative = prefer lower (coarser).
    pub fn set_lod_bias(&mut self, bias: f32) {
        self.vt.lod_bias = bias;
    }

    /// Returns (loaded_tiles, visible_tiles).
    pub fn get_stats(&self) -> (usize, usize) {
        (self.vt.slot_map.len(), self.vt.visible_tile_keys.len())
    }

    /// Get the pending chunks queue so callers can push tile data from other threads.
    pub fn pending_chunks(&self) -> Option<PendingChunks> {
        Some(self.vt.pending_chunks.clone())
    }

    /// Shared handle to the "tiles bovista wants" map. Loaders read
    /// this to decide what to fetch and in what order.
    pub fn wanted_handle(&self) -> crate::visuals::virtual_texture::Wanted {
        self.vt.wanted.clone()
    }

    /// Snapshot of the most recent prepare's timing/counts.
    pub fn stats(&self) -> crate::visuals::virtual_texture::PrepareStats {
        self.vt.stats.clone()
    }

    /// Request that the image display timepoint `t`. Identical semantics
    /// to `VolumeVisual::set_desired_timepoint` — page table flips once
    /// the visible tiles for `t` have arrived.
    pub fn set_desired_timepoint(&mut self, t: u32) {
        self.vt.set_desired_timepoint(t);
    }

    pub fn desired_t(&self) -> u32 { self.vt.desired_t() }

    /// Enable look-ahead prefetching of the next `lookahead` timepoints.
    /// Look-ahead only by design — the LRU pool keeps recently-displayed
    /// past frames resident, so explicit look-behind is redundant.
    pub fn set_prefetch(&mut self, lookahead: u32, t_count: u32) {
        self.vt.set_prefetch(lookahead, t_count);
    }
}

impl Visual for Image {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &crate::visual::CameraInfo) {
        // Upload pending colormap if set
        if let Some(data) = self.pending_colormap.take() {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.colormap_texture,
                    mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &data,
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(256 * 4), rows_per_image: None },
                wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            );
        }

        self.frame_number += 1;

        let _ = device;
        self.vt.prepare(queue, &self.slice_plane, self.frame_number, camera_info);

        // Update VT uniform buffer.
        let vt = &self.vt;
        let mut lods = [VTLodInfo {
            grid_dims: [1, 1, 1], _pad: 0,
            tile_scale: [1.0, 1.0, 1.0], _pad2: 0.0,
            data_scale: [1.0, 1.0, 1.0], _pad3: 0.0,
            data_offset: [0.0, 0.0, 0.0], _pad4: 0.0,
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
                    // See volume.rs for the math: (tile - 1) / atlas
                    // paired with 0.5/atlas data_offset maps voxel_frac
                    // [0, 1] to the centres of the first and last
                    // texels in the slot, avoiding Nearest tie-breaks.
                    (tile_w as f32 - 1.0) / atlas_tile_w as f32,
                    (tile_h as f32 - 1.0) / atlas_tile_h as f32,
                    (tile_d as f32 - 1.0) / atlas_tile_d as f32,
                ],
                _pad3: 0.0,
                data_offset: [
                    0.5 / atlas_tile_w as f32,
                    0.5 / atlas_tile_h as f32,
                    0.5 / atlas_tile_d as f32,
                ],
                _pad4: 0.0,
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
            desired_t: vt.desired_t(),
            lods,
        };
        queue.write_buffer(&self.vt_uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));

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
        let vol_aabb = crate::visuals::gpu_structs::AABB::new(vol_min, vol_max);
        let tile_plane = crate::visuals::gpu_structs::SlicePlane {
            position: self.slice_plane.position,
            normal: self.slice_plane.normal,
        };

        if let Some((vertices, indices)) =
            crate::visuals::gpu_structs::compute_plane_aabb_intersection(&tile_plane, &vol_aabb)
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
    }

    fn render(&self, render_pass: &mut RenderPass) {
        if self.vt_index_count == 0 {
            return;
        }
        if let (Some(vb), Some(ib)) = (&self.vt_vertex_buffer, &self.vt_index_buffer) {
            render_pass.set_pipeline(&self.vt_render_pipeline);
            render_pass.set_bind_group(1, &self.vt_bind_group, &[]);
            render_pass.set_bind_group(2, &self.colormap_bind_group, &[]);
            render_pass.set_vertex_buffer(0, vb.slice(..));
            render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint16);
            render_pass.draw_indexed(0..self.vt_index_count, 0, 0..1);
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
