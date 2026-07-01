//! Labels visual — 2D segmentation-mask rendering (napari "Labels" layer).
//!
//! Labels reuse the entire virtual-texture slice path of [`Image`]: atlas,
//! page table, slice geometry, LOD walk, nearest sampling, blend modes,
//! timepoints, prefetch. The *only* differences are:
//!
//!   * the fragment shader runs in `ColorMode::LabelHash` — it reads the atlas
//!     sample as an integer ID, treats 0 as transparent background, and hashes
//!     every other ID to a stable colormap coordinate (see `virtual_tile.wgsl`);
//!   * label data is packed into the R16Float atlas *by magnitude* (exact to
//!     2048), not normalized — done in the bindings via `pack_*_label_tile`.
//!
//! So `Labels` is a thin newtype over `Image` that presets the color mode and a
//! categorical default palette, and forwards everything else through `Deref`.

use std::ops::{Deref, DerefMut};

use crate::visual::{BlendMode, ColorMode, Transform, Visual};
use crate::visuals::image::Image;
use crate::visuals::virtual_texture::LodLevelConfig;

/// Build a 256-entry RGBA colormap of visually-distinct categorical colors by
/// stepping hue with the golden-ratio conjugate (low-discrepancy → adjacent
/// LUT indices are far apart in hue). The label hash maps IDs into this LUT.
fn categorical_colormap() -> Vec<u8> {
    const GOLDEN: f32 = 0.618_034;
    let mut out = Vec::with_capacity(256 * 4);
    let mut h = 0.0f32;
    for _ in 0..256 {
        h = (h + GOLDEN).fract();
        // Fixed high saturation/value for vivid, well-separated colors.
        let (r, g, b) = hsv_to_rgb(h, 0.7, 0.95);
        out.extend_from_slice(&[r, g, b, 255]);
    }
    out
}

/// HSV → RGB (all inputs in [0, 1]); returns 8-bit channels.
fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    (
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}

/// Segmentation-mask visual. See the module docs; behaves like [`Image`] but
/// renders integer label IDs as flat hashed colors (ID 0 = transparent).
pub struct Labels {
    inner: Image,
}

impl Labels {
    /// Same signature as [`Image::new`] so the binding macros can construct it
    /// uniformly. Presets `LabelHash` color mode and a categorical palette.
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        atlas_count: usize,
    ) -> Self {
        let mut inner = Image::new(
            device,
            queue,
            surface_format,
            camera_bind_group_layout,
            lod_levels,
            max_tiles,
            atlas_count,
        );
        inner.set_color_mode(ColorMode::LabelHash);
        inner.set_colormap(&categorical_colormap());
        inner.set_name("Labels".to_string());
        Self { inner }
    }
}

// Forward all of Image's inherent methods (slice setters, set_colormap,
// set_label_seed, pending_chunks, wanted_handle, get_stats, timepoints, …).
impl Deref for Labels {
    type Target = Image;
    fn deref(&self) -> &Image {
        &self.inner
    }
}
impl DerefMut for Labels {
    fn deref_mut(&mut self) -> &mut Image {
        &mut self.inner
    }
}

// Trait objects don't see through Deref, so forward the Visual surface explicitly.
impl Visual for Labels {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &crate::visual::CameraInfo) {
        self.inner.prepare(device, queue, camera_info);
    }
    fn render(&self, render_pass: &mut wgpu::RenderPass) {
        self.inner.render(render_pass);
    }
    fn set_transform(&mut self, transform: Transform) {
        self.inner.set_transform(transform);
    }
    fn transform(&self) -> &Transform {
        self.inner.transform()
    }
    fn is_visible(&self) -> bool {
        self.inner.is_visible()
    }
    fn set_visible(&mut self, visible: bool) {
        self.inner.set_visible(visible);
    }
    fn set_blend_mode(&mut self, mode: BlendMode) {
        self.inner.set_blend_mode(mode);
    }
    fn blend_mode(&self) -> BlendMode {
        self.inner.blend_mode()
    }
    fn set_opacity(&mut self, opacity: f32) {
        self.inner.set_opacity(opacity);
    }
    fn opacity(&self) -> f32 {
        self.inner.opacity()
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
}
