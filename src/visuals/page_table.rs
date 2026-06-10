// Virtual-texture page table backed by a wgpu R32Uint 2D-array texture.
//
// Layout:
//   - One array layer per LOD level.
//   - Width  = min(max_linear, max_texture_dimension_2d).
//   - Height = ceil(max_linear / width).
//   - Each texel encodes a single tile:
//       bit 31     : resident
//       bits 29-30 : atlas_id (2 bits, 0..=3) — which of up to MAX_ATLAS_COUNT
//                    physical atlas textures holds this tile's voxel data
//       bits 16-28 : timepoint `t` (13 bits, 0..=8191)
//       bits  0-15 : atlas slot index (16 bits, 0..=65535)
//     The shader compares the texel's t against a `desired_t` uniform; a
//     mismatch is treated identically to non-resident, so the LOD fallback
//     walks to a coarser tile rather than displaying stale-t data.
//
// Coordinate mapping (must match the shader):
//   linear_index = tz * gy * gx + ty * gx + tx
//   texel_x = linear_index % width
//   texel_y = linear_index / width
//
// The page table is sampled via `textureLoad` (exact-index, no filtering)
// while the atlas is read via `textureSampleLevel` (Nearest filter for
// data fidelity, but with linear blending available if we ever want it).

use crate::visuals::virtual_texture::LodLevelConfig;

pub struct PageTable {
    pub texture: wgpu::Texture,
    pub texture_view: wgpu::TextureView,
    /// Width of the 2D page table texture (used for linear→2D coord mapping).
    pub width: u32,
    // (gx, gy, gz) for each LOD
    lod_grids: Vec<(u32, u32, u32)>,
}

impl PageTable {
    pub fn new(device: &wgpu::Device, levels: &[LodLevelConfig]) -> Self {
        assert!(!levels.is_empty(), "PageTable requires at least one LOD level");

        let mut lod_grids = Vec::with_capacity(levels.len());
        let mut max_linear = 0u32;

        for level in levels {
            let (gz, gy, gx) = level.grid_size();
            lod_grids.push((gx, gy, gz));
            max_linear = max_linear.max(gz * gy * gx);
        }

        let max_dim = device.limits().max_texture_dimension_2d;
        let pt_width  = max_linear.min(max_dim);
        let pt_height = max_linear.div_ceil(pt_width);
        let num_lods  = levels.len() as u32;

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("VT Page Table"),
            size: wgpu::Extent3d {
                width: pt_width,
                height: pt_height,
                // Pad to >= 2 layers so wgpu-hal-GL creates a real
                // GL_TEXTURE_2D_ARRAY object — its heuristic maps a
                // single-layer descriptor to plain GL_TEXTURE_2D, which
                // can't be sampled through a D2Array binding. The
                // shader's lod_count uniform bounds reads to the real
                // LOD count, so the padding layer is never read.
                depth_or_array_layers: num_lods.max(2),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        Self { texture, texture_view, width: pt_width, lod_grids }
    }

    /// Mark a tile as resident at the given atlas slot, tagged with the
    /// timepoint `t` whose data the slot currently holds. The shader
    /// compares `t` to its `desired_t` uniform; mismatched-t reads are
    /// treated as non-resident, so stale-t entries left behind after a
    /// scrub gracefully fall through to coarser LODs without needing
    /// explicit clearing on every desired_t change.
    pub fn update(&self, queue: &wgpu::Queue, lod: usize, tz: u32, ty: u32, tx: u32,
                  atlas_id: u32, t: u32, slot: u32) {
        let (gx, gy, _gz) = self.lod_grids[lod];
        let linear = tz * gy * gx + ty * gx + tx;
        let value: u32 = (1u32 << 31)
                       | ((atlas_id & 0x3) << 29)
                       | ((t & 0x1FFF) << 16)
                       | (slot & 0xFFFF);
        self.write_texel(queue, lod as u32, linear, value);
    }

    /// Mark a tile as non-resident (clear its entry).
    pub fn clear(&self, queue: &wgpu::Queue, lod: usize, tz: u32, ty: u32, tx: u32) {
        let (gx, gy, _gz) = self.lod_grids[lod];
        let linear = tz * gy * gx + ty * gx + tx;
        self.write_texel(queue, lod as u32, linear, 0u32);
    }

    fn write_texel(&self, queue: &wgpu::Queue, layer: u32, linear: u32, value: u32) {
        let px = linear % self.width;
        let py = linear / self.width;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: px, y: py, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            &value.to_ne_bytes(),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
    }
}
