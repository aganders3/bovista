// Force recompilation when any shader file changes.
const _SHADER_HASH: &str = env!("SHADER_HASH");

use crate::visual::{Transform, Visual};
use crate::visuals::virtual_texture::{LodLevelConfig, PendingChunks, VirtualTextureData};
use crate::visuals::gpu_structs::{VolumeVertex, VolumeUniforms, VTUniforms, VTLodInfo, VT_MAX_LODS};
use crate::visuals::gpu_structs::TileLoaderFn;
use wgpu::RenderPass;

// ── Internal: rendering modes (one pipeline per mode) ───────────────────────

/// Which fragment-shader entry point to use. Each concrete public visual fixes
/// a single variant at construction; users never see this enum.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum VolumeRenderMode {
    Translucent,
    Additive,
    /// `MipVolume` always uses this. With attenuation = 0 it degenerates to plain
    /// MIP; the `exp()` per step is cheap enough to amortise over a single pipeline.
    AttenuatedMip,
    Minip,
    Average,
    Iso,
}

impl VolumeRenderMode {
    fn fragment_entry(self) -> &'static str {
        match self {
            Self::Translucent   => "fs_translucent",
            Self::Additive      => "fs_additive",
            Self::AttenuatedMip => "fs_attenuated_mip",
            Self::Minip         => "fs_minip",
            Self::Average       => "fs_average",
            Self::Iso           => "fs_iso",
        }
    }
}

/// Mode-specific values that go into the volume uniform buffer each frame.
/// Each concrete visual fills only the fields its shader actually reads.
struct VolumeUniformExtras {
    density_scale: f32,
    early_exit_alpha: f32,
    iso_threshold: f32,
    attenuation: f32,
    debug_mode: u32,
}

impl Default for VolumeUniformExtras {
    fn default() -> Self {
        Self {
            density_scale: 0.01,
            early_exit_alpha: 0.95,
            iso_threshold: 0.5,
            attenuation: 0.0,
            debug_mode: 0,
        }
    }
}

// ── Box geometry shared by all volume types ─────────────────────────────────

#[rustfmt::skip]
const BOX_VERTICES: &[[f32; 3]] = &[
    [0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [1.0, 1.0, 0.0], [0.0, 1.0, 0.0],
    [0.0, 0.0, 1.0], [1.0, 0.0, 1.0], [1.0, 1.0, 1.0], [0.0, 1.0, 1.0],
];

/// 12 triangles (36 indices), CW winding viewed from outside (inward-wound).
/// Combined with `cull_mode: Front`, this renders only the far/exit faces.
#[rustfmt::skip]
const BOX_INDICES: &[u16] = &[
    0, 1, 2,  0, 2, 3,    // -Z
    4, 6, 5,  4, 7, 6,    // +Z
    0, 5, 1,  0, 4, 5,    // -Y
    3, 2, 6,  3, 6, 7,    // +Y
    0, 7, 4,  0, 3, 7,    // -X
    1, 6, 2,  1, 5, 6,    // +X
];

// ── VolumeCore: shared GPU state for all volume visual types ────────────────

/// Shared GPU state for every volume rendering visual. Each concrete public
/// type (`DirectVolume`, `MipVolume`, `IsosurfaceVolume`, …) owns one of these
/// plus its mode-specific parameters. The pipeline inside the core is fixed
/// at construction to the right mode's fragment entry point.
pub struct VolumeCore {
    strategy: VirtualTextureData,

    // Group 2 — colormap LUT + volume uniforms.
    colormap_texture: wgpu::Texture,
    vol_bind_group: wgpu::BindGroup,
    vol_uniform_buffer: wgpu::Buffer,
    pending_colormap: Option<Vec<u8>>,

    // Group 1 — VT bind group.
    vt_bind_group: wgpu::BindGroup,
    vt_uniform_buffer: wgpu::Buffer,

    // Geometry.
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,

    // Pipeline for this visual's mode.
    pipeline: wgpu::RenderPipeline,

    // Common state.
    contrast_limits: (f32, f32),
    relative_step_size: f32,
    frame_number: u64,

    transform: Transform,
    visible: bool,
    name: String,
}

impl VolumeCore {
    fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
        mode: VolumeRenderMode,
    ) -> Self {
        let strategy = VirtualTextureData::new(device, lod_levels, max_tiles, loader);

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

        // ── Group 1: VT resources ───────────────────────────────────────────
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
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&strategy.atlas_texture_view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&atlas_sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&strategy.page_table.texture_view) },
                wgpu::BindGroupEntry { binding: 3, resource: vt_uniform_buffer.as_entire_binding() },
            ],
        });

        // ── Group 2: colormap + volume uniforms ─────────────────────────────
        let colormap_bgl = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Volume Colormap Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // Visible to vertex stage too: vs_main uses vol_min/vol_max.
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

        // Default colormap: grayscale ramp with proportional alpha.
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
                wgpu::BindGroupEntry { binding: 0, resource: vol_uniform_buffer.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&colormap_view) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&colormap_sampler) },
            ],
        });

        // ── Box geometry ────────────────────────────────────────────────────
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

        // ── Pipeline for this mode ──────────────────────────────────────────
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
        let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
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
                entry_point: Some(mode.fragment_entry()),
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
                cull_mode: Some(wgpu::Face::Front),
                unclipped_depth: false,
                polygon_mode: wgpu::PolygonMode::Fill,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1, mask: !0, alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        let default_name = match mode {
            VolumeRenderMode::Translucent   => "DirectVolume",
            VolumeRenderMode::Additive      => "AdditiveVolume",
            VolumeRenderMode::AttenuatedMip => "MipVolume",
            VolumeRenderMode::Minip         => "MinipVolume",
            VolumeRenderMode::Average       => "AverageVolume",
            VolumeRenderMode::Iso           => "IsosurfaceVolume",
        };

        Self {
            strategy,
            colormap_texture,
            vol_bind_group,
            vol_uniform_buffer,
            pending_colormap: None,
            vt_bind_group,
            vt_uniform_buffer,
            vertex_buffer,
            index_buffer,
            pipeline,
            contrast_limits: (0.0, 1.0),
            relative_step_size: 1.0,
            frame_number: 0,
            transform: Transform::identity(),
            visible: true,
            name: default_name.to_string(),
        }
    }

    /// Upload all uniforms for this frame. `extras` carries the mode-specific
    /// fields; the caller passes zeros/defaults for fields its shader doesn't read.
    fn upload_uniforms(
        &mut self,
        queue: &wgpu::Queue,
        camera_info: &crate::visual::CameraInfo,
        extras: VolumeUniformExtras,
    ) {
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

        self.frame_number += 1;
        self.strategy.prepare_volume(queue, self.frame_number, camera_info);

        // VT uniforms (identical to ImageVisual).
        let vt = &self.strategy;
        let mut lods = [VTLodInfo {
            grid_dims: [1, 1, 1], _pad: 0,
            tile_scale: [1.0, 1.0, 1.0], _pad2: 0.0,
            data_scale: [1.0, 1.0, 1.0], _pad3: 0.0,
        }; VT_MAX_LODS];
        // Atlas slots are sized to fit the largest chunk across all LODs, not
        // just LOD 0's chunk size — `vt.tile_size` is that max.
        let (atlas_tile_d, atlas_tile_h, atlas_tile_w) = vt.tile_size;
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
            debug_mode: extras.debug_mode,
            page_table_width: vt.page_table.width,
            target_lod: vt.cached_ideal_lod as u32,
            _pad_c: 0,
            lods,
        };
        queue.write_buffer(&self.vt_uniform_buffer, 0, bytemuck::cast_slice(&[vt_uniforms]));

        // Volume uniforms.
        let lod0 = &vt.levels[0];
        let vol_min = [lod0.translation.2, lod0.translation.1, lod0.translation.0];
        let vol_max = [
            vol_min[0] + lod0.volume_size.2 as f32 * lod0.voxel_size.2,
            vol_min[1] + lod0.volume_size.1 as f32 * lod0.voxel_size.1,
            vol_min[2] + lod0.volume_size.0 as f32 * lod0.voxel_size.0,
        ];
        let lod0_dims = [lod0.volume_size.2, lod0.volume_size.1, lod0.volume_size.0];
        let vol_uniforms = VolumeUniforms {
            vol_min,
            relative_step_size: self.relative_step_size,
            vol_max,
            density_scale: extras.density_scale,
            camera_pos: camera_info.position.into(),
            early_exit_alpha: extras.early_exit_alpha,
            lod0_dims,
            debug_mode: extras.debug_mode,
            iso_threshold: extras.iso_threshold,
            attenuation: extras.attenuation,
            _pad0: 0.0,
            _pad1: 0.0,
        };
        queue.write_buffer(&self.vol_uniform_buffer, 0, bytemuck::cast_slice(&[vol_uniforms]));
    }

    fn render(&self, render_pass: &mut RenderPass) {
        render_pass.set_pipeline(&self.pipeline);
        render_pass.set_bind_group(1, &self.vt_bind_group, &[]);
        render_pass.set_bind_group(2, &self.vol_bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..BOX_INDICES.len() as u32, 0, 0..1);
    }

    // ── Common setters / getters (shared API across all volume types) ───────

    pub fn set_name(&mut self, name: String) { self.name = name; }

    /// Step size in LOD-0 voxels. 1.0 = Nyquist at finest LOD.
    pub fn set_relative_step_size(&mut self, step: f32) { self.relative_step_size = step; }

    pub fn set_contrast_limits(&mut self, min: f32, max: f32) {
        self.contrast_limits = (min, max);
    }

    /// Upload a 256-entry RGBA colormap (1024 bytes, values 0–255).
    /// An empty or wrong-length slice resets to the default grayscale.
    pub fn set_colormap(&mut self, rgba: &[u8]) {
        self.pending_colormap = Some(if rgba.len() == 1024 {
            rgba.to_vec()
        } else {
            (0u32..256).flat_map(|i| { let v = i as u8; [v, v, v, v] }).collect()
        });
    }

    pub fn set_lod_bias(&mut self, bias: f32) { self.strategy.lod_bias = bias; }

    /// `(loaded_tiles, visible_tiles)`.
    pub fn get_stats(&self) -> (usize, usize) {
        (self.strategy.slot_map.len(), self.strategy.visible_tile_keys.len())
    }

    pub fn pending_chunks(&self) -> Option<PendingChunks> {
        Some(self.strategy.pending_chunks.clone())
    }
}

// ── Macro: identical Visual trait impl for each volume type ─────────────────

macro_rules! impl_volume_visual {
    ($ty:ty, |$me:ident| $extras:expr) => {
        impl Visual for $ty {
            fn prepare(
                &mut self,
                _device: &wgpu::Device,
                queue: &wgpu::Queue,
                camera_info: &crate::visual::CameraInfo,
            ) {
                let $me: &$ty = self;
                let extras = $extras;
                self.core.upload_uniforms(queue, camera_info, extras);
            }
            fn render(&self, render_pass: &mut RenderPass) { self.core.render(render_pass); }
            fn set_transform(&mut self, transform: Transform) { self.core.transform = transform; }
            fn transform(&self) -> &Transform { &self.core.transform }
            fn is_visible(&self) -> bool { self.core.visible }
            fn set_visible(&mut self, visible: bool) { self.core.visible = visible; }
            fn name(&self) -> &str { &self.core.name }
        }
    };
}

/// Forwards common setters/getters from the wrapping public type to its `core` field.
/// Saves writing the same six pass-throughs on every concrete visual.
macro_rules! impl_common_volume_methods {
    ($ty:ty) => {
        impl $ty {
            pub fn set_name(&mut self, name: String) { self.core.set_name(name); }
            pub fn set_relative_step_size(&mut self, step: f32) { self.core.set_relative_step_size(step); }
            pub fn set_contrast_limits(&mut self, min: f32, max: f32) { self.core.set_contrast_limits(min, max); }
            pub fn set_colormap(&mut self, rgba: &[u8]) { self.core.set_colormap(rgba); }
            pub fn set_lod_bias(&mut self, bias: f32) { self.core.set_lod_bias(bias); }
            pub fn get_stats(&self) -> (usize, usize) { self.core.get_stats() }
            pub fn pending_chunks(&self) -> Option<PendingChunks> { self.core.pending_chunks() }
        }
    };
}

// ── Concrete volume visual types ────────────────────────────────────────────

/// Direct volume rendering with front-to-back alpha compositing (Beer-Lambert).
/// Equivalent to napari's `Image.rendering = "translucent"`.
pub struct DirectVolume {
    core: VolumeCore,
    density_scale: f32,
    early_exit_alpha: f32,
    debug_mode: u32,
}

impl DirectVolume {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        let core = VolumeCore::new(
            device, queue, surface_format, camera_bind_group_layout,
            lod_levels, max_tiles, loader, VolumeRenderMode::Translucent,
        );
        Self { core, density_scale: 0.01, early_exit_alpha: 0.95, debug_mode: 0 }
    }

    /// Per-step extinction multiplier. Higher = denser/more opaque volume.
    pub fn set_density_scale(&mut self, scale: f32) { self.density_scale = scale; }

    /// Front-to-back accumulation cutoff. Default 0.95.
    pub fn set_early_exit_alpha(&mut self, alpha: f32) {
        self.early_exit_alpha = alpha.clamp(0.0, 1.0);
    }

    /// Enable LOD tinting + tile wireframes (mode 1). False restores normal DVR.
    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = if enabled { 1 } else { 0 };
    }

    /// Bypass page table and sample raw atlas (mode 2). False restores normal DVR.
    pub fn set_atlas_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = if enabled { 2 } else { 0 };
    }

    /// Step-count heatmap (mode 3). False restores normal DVR.
    pub fn set_step_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = if enabled { 3 } else { 0 };
    }
}

impl_common_volume_methods!(DirectVolume);
impl_volume_visual!(DirectVolume, |me| VolumeUniformExtras {
    density_scale: me.density_scale,
    early_exit_alpha: me.early_exit_alpha,
    debug_mode: me.debug_mode,
    ..VolumeUniformExtras::default()
});

// ────────────────────────────────────────────────────────────────────────────

/// Additive accumulation along the ray. No occlusion — every voxel contributes
/// regardless of what's in front of it. Useful for multi-channel composition
/// (stack N AdditiveVolumes with different colormaps).
pub struct AdditiveVolume {
    core: VolumeCore,
    density_scale: f32,
}

impl AdditiveVolume {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        let core = VolumeCore::new(
            device, queue, surface_format, camera_bind_group_layout,
            lod_levels, max_tiles, loader, VolumeRenderMode::Additive,
        );
        Self { core, density_scale: 0.01 }
    }

    pub fn set_density_scale(&mut self, scale: f32) { self.density_scale = scale; }
}

impl_common_volume_methods!(AdditiveVolume);
impl_volume_visual!(AdditiveVolume, |me| VolumeUniformExtras {
    density_scale: me.density_scale,
    ..VolumeUniformExtras::default()
});

// ────────────────────────────────────────────────────────────────────────────

/// Maximum Intensity Projection. With `attenuation == 0` this is plain MIP;
/// with `attenuation > 0` each sample is multiplied by `exp(-attenuation * t)`
/// before max-tracking, weighting near-camera regions more heavily
/// ("attenuated MIP" in napari).
pub struct MipVolume {
    core: VolumeCore,
    attenuation: f32,
}

impl MipVolume {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        // The attenuated-MIP shader degenerates to plain MIP when attenuation == 0,
        // so we always use that pipeline. (The exp() per step is cheap enough to
        // amortise across a single pipeline rather than maintaining two.)
        let core = VolumeCore::new(
            device, queue, surface_format, camera_bind_group_layout,
            lod_levels, max_tiles, loader, VolumeRenderMode::AttenuatedMip,
        );
        Self { core, attenuation: 0.0 }
    }

    /// Falloff coefficient per world-space unit along the ray.
    /// 0.0 = plain MIP. Larger = stronger near-camera emphasis.
    pub fn set_attenuation(&mut self, attenuation: f32) { self.attenuation = attenuation.max(0.0); }
}

impl_common_volume_methods!(MipVolume);
impl_volume_visual!(MipVolume, |me| VolumeUniformExtras {
    attenuation: me.attenuation,
    ..VolumeUniformExtras::default()
});

// ────────────────────────────────────────────────────────────────────────────

/// Minimum Intensity Projection.
pub struct MinipVolume { core: VolumeCore }

impl MinipVolume {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        let core = VolumeCore::new(
            device, queue, surface_format, camera_bind_group_layout,
            lod_levels, max_tiles, loader, VolumeRenderMode::Minip,
        );
        Self { core }
    }
}

impl_common_volume_methods!(MinipVolume);
impl_volume_visual!(MinipVolume, |_me| VolumeUniformExtras::default());

// ────────────────────────────────────────────────────────────────────────────

/// Mean intensity along the ray.
pub struct AverageVolume { core: VolumeCore }

impl AverageVolume {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        let core = VolumeCore::new(
            device, queue, surface_format, camera_bind_group_layout,
            lod_levels, max_tiles, loader, VolumeRenderMode::Average,
        );
        Self { core }
    }
}

impl_common_volume_methods!(AverageVolume);
impl_volume_visual!(AverageVolume, |_me| VolumeUniformExtras::default());

// ────────────────────────────────────────────────────────────────────────────

/// First-hit isosurface raymarching with Phong shading.
/// `iso_threshold` is in contrast-normalised raw-data units, 0..1.
pub struct IsosurfaceVolume {
    core: VolumeCore,
    iso_threshold: f32,
}

impl IsosurfaceVolume {
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        let core = VolumeCore::new(
            device, queue, surface_format, camera_bind_group_layout,
            lod_levels, max_tiles, loader, VolumeRenderMode::Iso,
        );
        Self { core, iso_threshold: 0.5 }
    }

    pub fn set_iso_threshold(&mut self, threshold: f32) {
        self.iso_threshold = threshold.clamp(0.0, 1.0);
    }
}

impl_common_volume_methods!(IsosurfaceVolume);
impl_volume_visual!(IsosurfaceVolume, |me| VolumeUniformExtras {
    iso_threshold: me.iso_threshold,
    ..VolumeUniformExtras::default()
});
