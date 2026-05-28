// Force recompilation when any shader file changes.
const _SHADER_HASH: &str = env!("SHADER_HASH");

use crate::visual::{Transform, Visual};
use crate::visuals::virtual_texture::{LodLevelConfig, PendingChunks, VirtualTextureData};
use crate::visuals::gpu_structs::{VolumeVertex, VolumeUniforms, VTUniforms, VTLodInfo, VT_MAX_LODS};
use crate::visuals::gpu_structs::TileLoaderFn;
use wgpu::RenderPass;

/// Visual for direct volume rendering via ray marching.
///
/// Reuses the same `VirtualTextureData` (atlas, page table, LOD selection, tile streaming)
/// as `ImageVisual`. The fragment shader fires a ray per pixel and composites voxel
/// contributions front-to-back using the same colormap/contrast as the slice visual.
pub struct VolumeVisual {
    strategy: VirtualTextureData,

    // Colormap LUT (group 2, binding 1) — 256-entry 1D RGBA texture
    colormap_texture: wgpu::Texture,
    #[allow(dead_code)]
    colormap_bind_group_layout: wgpu::BindGroupLayout,
    vol_bind_group: wgpu::BindGroup,
    vol_uniform_buffer: wgpu::Buffer,
    pending_colormap: Option<Vec<u8>>,

    // VT bind group (group 1)
    vt_bind_group: wgpu::BindGroup,
    vt_uniform_buffer: wgpu::Buffer,

    // Box geometry (8 vertices, 36 indices)
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,

    render_pipeline: wgpu::RenderPipeline,

    contrast_limits: (f32, f32),
    relative_step_size: f32,
    density_scale: f32,
    early_exit_alpha: f32,
    debug_mode: u32,
    frame_number: u64,

    transform: Transform,
    visible: bool,
    name: String,
}

/// Unit-cube vertices (positions only).
#[rustfmt::skip]
const BOX_VERTICES: &[[f32; 3]] = &[
    [0.0, 0.0, 0.0], // 0
    [1.0, 0.0, 0.0], // 1
    [1.0, 1.0, 0.0], // 2
    [0.0, 1.0, 0.0], // 3
    [0.0, 0.0, 1.0], // 4
    [1.0, 0.0, 1.0], // 5
    [1.0, 1.0, 1.0], // 6
    [0.0, 1.0, 1.0], // 7
];

/// 12 triangles (36 indices), CW winding viewed from outside (inward-wound).
/// Combined with `cull_mode: Front`, this renders the back/far faces of the box —
/// the exit faces for rays travelling from the camera into the volume.
#[rustfmt::skip]
const BOX_INDICES: &[u16] = &[
    // -Z face
    0, 1, 2,  0, 2, 3,
    // +Z face
    4, 6, 5,  4, 7, 6,
    // -Y face
    0, 5, 1,  0, 4, 5,
    // +Y face
    3, 2, 6,  3, 6, 7,
    // -X face
    0, 7, 4,  0, 3, 7,
    // +X face
    1, 6, 2,  1, 5, 6,
];

impl VolumeVisual {
    /// Create a `VolumeVisual` with virtual-texture multiscale streaming.
    ///
    /// # Arguments
    /// * `lod_levels` - Configuration for each LOD level (finest to coarsest)
    /// * `max_tiles`  - Atlas capacity (number of simultaneously resident tiles)
    /// * `loader`     - Callback invoked when a tile is needed
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        let strategy = VirtualTextureData::new(device, lod_levels, max_tiles, loader);

        // Atlas sampler
        let atlas_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("Volume Atlas Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // ── VT bind group (group 1) ───────────────────────────────────────────
        let vt_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Volume VT Bind Group Layout"),
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
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Uint,
                        view_dimension: wgpu::TextureViewDimension::D2Array,
                        multisampled: false,
                    },
                    count: None,
                },
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
            label: Some("Volume VT Uniform Buffer"),
            size: std::mem::size_of::<VTUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let vt_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Volume VT Bind Group"),
            layout: &vt_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&strategy.atlas_texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&atlas_sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::TextureView(&strategy.page_table.texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 3,
                    resource: vt_uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // ── Volume bind group (group 2) ───────────────────────────────────────
        let colormap_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Volume Colormap Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // Visible to vertex stage too: vs_main uses vol_min/vol_max to position the box.
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
                        view_dimension: wgpu::TextureViewDimension::D1,
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

        let vol_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Volume Uniforms Buffer"),
            size: std::mem::size_of::<VolumeUniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Default colormap: grayscale with proportional alpha (matches kiln's grayscale TF).
        // Color ramps with density so low-density regions are dark gray, high-density
        // regions are white. Alpha also ramps so background/air stays transparent.
        let colormap_data: Vec<u8> = (0u32..256)
            .flat_map(|i| { let v = i as u8; [v, v, v, v] })
            .collect();
        let colormap_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Volume Colormap Texture"),
            size: wgpu::Extent3d { width: 256, height: 1, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D1,
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
            label: Some("Volume Colormap Sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let colormap_view = colormap_texture.create_view(&wgpu::TextureViewDescriptor::default());

        let vol_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Volume Bind Group"),
            layout: &colormap_bgl,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: vol_uniform_buffer.as_entire_binding(),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::TextureView(&colormap_view),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: wgpu::BindingResource::Sampler(&colormap_sampler),
                },
            ],
        });

        // ── Box geometry ──────────────────────────────────────────────────────
        use wgpu::util::DeviceExt;
        let box_verts: Vec<VolumeVertex> = BOX_VERTICES.iter()
            .map(|p| VolumeVertex { position: *p })
            .collect();
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Volume Box Vertex Buffer"),
            contents: bytemuck::cast_slice(&box_verts),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Volume Box Index Buffer"),
            contents: bytemuck::cast_slice(BOX_INDICES),
            usage: wgpu::BufferUsages::INDEX,
        });

        // ── Render pipeline ───────────────────────────────────────────────────
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Volume Ray March Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/volume_raymarch.wgsl").into(),
            ),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Volume Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &vt_bgl, &colormap_bgl],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Volume Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[VolumeVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                // TEMP DIAGNOSTIC: cull disabled to isolate whether HPC GLES
                // path is rejecting triangles by winding. Was: Some(Face::Front).
                cull_mode: None,
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,  // transparent volume; don't occlude other visuals
                // TEMP DIAGNOSTIC: Always lets every fragment pass the depth
                // test. Was: LessEqual.
                depth_compare: wgpu::CompareFunction::Always,
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
            strategy,
            colormap_texture,
            colormap_bind_group_layout: colormap_bgl,
            vol_bind_group,
            vol_uniform_buffer,
            pending_colormap: None,
            vt_bind_group,
            vt_uniform_buffer,
            vertex_buffer,
            index_buffer,
            render_pipeline,
            contrast_limits: (0.0, 1.0),
            relative_step_size: 1.0,
            density_scale: 0.01,
            early_exit_alpha: 0.95,
            debug_mode: 0,
            frame_number: 0,
            transform: Transform::identity(),
            visible: true,
            name: "Volume".to_string(),
        }
    }

    pub fn set_name(&mut self, name: String) {
        self.name = name;
    }

    /// Step size in LOD-0 voxels. Coarser-LOD regions take proportionally larger world-space
    /// steps so you always sample once per equivalent voxel. 1.0 = Nyquist at finest LOD.
    /// Smaller values = higher quality and slower; larger = faster but blockier.
    pub fn set_relative_step_size(&mut self, step: f32) {
        self.relative_step_size = step;
    }

    /// Overall opacity multiplier applied per step.
    pub fn set_density_scale(&mut self, scale: f32) {
        self.density_scale = scale;
    }

    /// Front-to-back accumulation cutoff. Default 0.95.
    /// Lower (e.g. 0.85) terminates rays sooner in dense regions for a small accuracy cost;
    /// raise toward 1.0 for higher accuracy at the cost of more steps.
    pub fn set_early_exit_alpha(&mut self, alpha: f32) {
        self.early_exit_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Set display contrast window [min, max] in normalised voxel units.
    pub fn set_contrast_limits(&mut self, min: f32, max: f32) {
        self.contrast_limits = (min, max);
    }

    /// Upload a 256-entry RGBA colormap (1024 bytes, values 0–255).
    /// Resets to grayscale if an empty or wrong-length slice is passed.
    pub fn set_colormap(&mut self, rgba: &[u8]) {
        self.pending_colormap = Some(if rgba.len() == 1024 {
            rgba.to_vec()
        } else {
            (0u32..256).flat_map(|i| { let v = i as u8; [v, v, v, v] }).collect()
        });
    }

    /// Enable debug LOD tinting + tile wireframes (mode 1). False restores normal DVR (mode 0).
    pub fn set_debug_mode(&mut self, debug: bool) {
        self.debug_mode = if debug { 1 } else { 0 };
    }

    /// Set the debug mode by exact value. See volume_raymarch.wgsl for the
    /// mode→behaviour mapping. Intended for offscreen diagnostics; the
    /// boolean setters above remain the supported public API.
    pub fn set_debug_mode_raw(&mut self, mode: u32) {
        self.debug_mode = mode;
    }

    /// Enable atlas-direct debug mode (mode 2): bypasses the page table and samples the
    /// raw packed atlas texture. Useful for verifying atlas allocation and loaded data.
    /// False restores normal DVR (mode 0).
    pub fn set_atlas_debug_mode(&mut self, debug: bool) {
        self.debug_mode = if debug { 2 } else { 0 };
    }

    /// Enable step-count heatmap (mode 3): colours each pixel by the number of ray-march
    /// steps taken. Blue = few steps (coarse LOD or short ray), red = many (fine LOD).
    /// False restores normal DVR (mode 0).
    pub fn set_step_debug_mode(&mut self, debug: bool) {
        self.debug_mode = if debug { 3 } else { 0 };
    }

    /// LOD bias. Positive = finer (higher resolution), negative = coarser.
    pub fn set_lod_bias(&mut self, bias: f32) {
        self.strategy.lod_bias = bias;
    }

    /// Returns `(loaded_tiles, visible_tiles)`.
    pub fn get_stats(&self) -> (usize, usize) {
        (self.strategy.slot_map.len(), self.strategy.visible_tile_keys.len())
    }

    /// Access the pending chunks queue for pushing tile data from other threads.
    pub fn pending_chunks(&self) -> Option<PendingChunks> {
        Some(self.strategy.pending_chunks.clone())
    }
}

impl Visual for VolumeVisual {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &crate::visual::CameraInfo) {
        let _ = device; // box geometry is static; no per-frame rebuild needed

        // Upload pending colormap
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
        self.strategy.prepare_volume(queue, self.frame_number, camera_info);

        // Upload VT uniforms (identical logic to ImageVisual::prepare)
        let vt = &self.strategy;
        let mut lods = [VTLodInfo {
            grid_dims: [1, 1, 1], _pad: 0,
            tile_scale: [1.0, 1.0, 1.0], _pad2: 0.0,
            data_scale: [1.0, 1.0, 1.0], _pad3: 0.0,
        }; VT_MAX_LODS];
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
                    // Use (tile - 0.5) / atlas_tile so the maximum within-tile UV
                    // lands on the last texel's centre rather than the boundary
                    // between this tile and the next slot in the atlas.
                    (tile_w as f32 - 0.5) / atlas_tile_w as f32,
                    (tile_h as f32 - 0.5) / atlas_tile_h as f32,
                    (tile_d as f32 - 0.5) / atlas_tile_d as f32,
                ],
                _pad3: 0.0,
            };
        }
        let vt_uniforms = VTUniforms {
            atlas_cols: vt.atlas_cols,
            atlas_rows: vt.atlas_rows,
            atlas_tile_pitch_x: 1.0 / vt.atlas_cols as f32,
            atlas_tile_pitch_y: 1.0 / vt.atlas_rows as f32,
            atlas_tile_pitch_z: 1.0 / vt.atlas_layers as f32,
            lod_count: vt.levels.len().min(VT_MAX_LODS) as u32,
            contrast_min: self.contrast_limits.0,
            contrast_max: self.contrast_limits.1,
            debug_mode: self.debug_mode,
            page_table_width: vt.page_table.width,
            target_lod: vt.cached_ideal_lod as u32,
            _pad_c: 0,
            lods,
        };
        queue.write_buffer(&self.vt_uniform_buffer, 0, bytemuck::cast_slice(&[vt_uniforms]));

        // Upload volume uniforms: AABB and voxel dimensions from LOD 0.
        let lod0 = &vt.levels[0];
        let vol_min = [
            lod0.translation.2,
            lod0.translation.1,
            lod0.translation.0,
        ];
        let vol_max = [
            vol_min[0] + lod0.volume_size.2 as f32 * lod0.voxel_size.2,
            vol_min[1] + lod0.volume_size.1 as f32 * lod0.voxel_size.1,
            vol_min[2] + lod0.volume_size.0 as f32 * lod0.voxel_size.0,
        ];
        // LOD-0 voxel dimensions used by the shader as the base unit; advance is then
        // scaled up by the actual LOD ratio so step size = relative_step_size voxels at that LOD.
        let lod0_dims = [lod0.volume_size.2, lod0.volume_size.1, lod0.volume_size.0];
        let vol_uniforms = VolumeUniforms {
            vol_min,
            relative_step_size: self.relative_step_size,
            vol_max,
            density_scale: self.density_scale,
            camera_pos: camera_info.position.into(),
            early_exit_alpha: self.early_exit_alpha,
            lod0_dims,
            debug_mode: self.debug_mode,
        };
        queue.write_buffer(&self.vol_uniform_buffer, 0, bytemuck::cast_slice(&[vol_uniforms]));
    }

    fn render(&self, render_pass: &mut RenderPass) {
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(1, &self.vt_bind_group, &[]);
        render_pass.set_bind_group(2, &self.vol_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..BOX_INDICES.len() as u32, 0, 0..1);
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
