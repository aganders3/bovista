// Virtual-texture page table backed by a wgpu 2D-array texture.
//
// Layout:
//   - One array layer per LOD level.
//   - Width  = min(max_linear, max_texture_dimension_2d).
//   - Height = ceil(max_linear / width).
//   - Format: Rgba8Unorm. Each texel packs:
//       R = slot's low byte  (slot & 0xFF)
//       G = slot's high byte ((slot >> 8) & 0xFF)
//       B = resident byte    (255 if resident, 0 otherwise)
//       A = 0  (reserved)
//     This avoids `usampler*` in the shader — a combination of `usampler*`
//     (integer textures) and regular `sampler*` (float textures) in the
//     same fragment shader trips a long-standing NVIDIA Cg-compiler bug
//     on the GL 3.30 compat profile that surfaces as silently empty
//     fragments. Float-format textures read via `textureLoad` (no
//     filtering, no sampler binding) sidestep the bug while keeping the
//     same access semantics.
//
// Coordinate mapping (must match the shader):
//   linear_index = tz * gy * gx + ty * gx + tx
//   texel_x = linear_index % width
//   texel_y = linear_index / width

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
                // GL_TEXTURE_2D_ARRAY object (its heuristic maps
                // depth_or_array_layers==1 to plain GL_TEXTURE_2D, which
                // can't be sampled through a D2Array binding on GLES).
                // The shader's lod_count uniform still bounds reads to
                // the real LOD count, so the extra layer is never read.
                depth_or_array_layers: num_lods.max(2),
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        Self { texture, texture_view, width: pt_width, lod_grids }
    }

    /// Mark a tile as resident at the given atlas slot.
    pub fn update(&self, queue: &wgpu::Queue, lod: usize, tz: u32, ty: u32, tx: u32, slot: u32) {
        let (gx, gy, _gz) = self.lod_grids[lod];
        let linear = tz * gy * gx + ty * gx + tx;
        // Pack [slot_lo, slot_hi, resident_byte, reserved] as Rgba8Unorm.
        let bytes: [u8; 4] = [
            (slot & 0xff) as u8,
            ((slot >> 8) & 0xff) as u8,
            0xff,  // resident
            0,
        ];
        self.write_texel(queue, lod as u32, linear, bytes);
    }

    /// Mark a tile as non-resident (clear its entry).
    pub fn clear(&self, queue: &wgpu::Queue, lod: usize, tz: u32, ty: u32, tx: u32) {
        let (gx, gy, _gz) = self.lod_grids[lod];
        let linear = tz * gy * gx + ty * gx + tx;
        self.write_texel(queue, lod as u32, linear, [0, 0, 0, 0]);
    }

    fn write_texel(&self, queue: &wgpu::Queue, layer: u32, linear: u32, bytes: [u8; 4]) {
        let px = linear % self.width;
        let py = linear / self.width;
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.texture,
                mip_level: 0,
                origin: wgpu::Origin3d { x: px, y: py, z: layer },
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(self.width * 4),
                rows_per_image: None,
            },
            wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
        );
    }
}
