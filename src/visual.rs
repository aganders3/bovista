//! Core visual abstraction and vertex buffer utilities.
//!
//! This module defines the [`Visual`] trait that all renderable objects must implement,
//! along with utilities for defining custom vertex layouts.

use glam::Mat4;
use std::any::Any;
use wgpu::RenderPass;

/// Describes the format of a single vertex attribute.
///
/// This enum provides a platform-independent way to describe vertex data
/// that gets mapped to WGPU vertex formats.
#[derive(Clone, Debug)]
pub enum VertexFormat {
    Float32,
    Float32x2,
    Float32x3,
    Float32x4,
    Uint32,
    Uint32x2,
    Uint32x3,
    Uint32x4,
}

impl VertexFormat {
    pub fn to_wgpu(&self) -> wgpu::VertexFormat {
        match self {
            VertexFormat::Float32 => wgpu::VertexFormat::Float32,
            VertexFormat::Float32x2 => wgpu::VertexFormat::Float32x2,
            VertexFormat::Float32x3 => wgpu::VertexFormat::Float32x3,
            VertexFormat::Float32x4 => wgpu::VertexFormat::Float32x4,
            VertexFormat::Uint32 => wgpu::VertexFormat::Uint32,
            VertexFormat::Uint32x2 => wgpu::VertexFormat::Uint32x2,
            VertexFormat::Uint32x3 => wgpu::VertexFormat::Uint32x3,
            VertexFormat::Uint32x4 => wgpu::VertexFormat::Uint32x4,
        }
    }

    pub fn size(&self) -> u64 {
        match self {
            VertexFormat::Float32 | VertexFormat::Uint32 => 4,
            VertexFormat::Float32x2 | VertexFormat::Uint32x2 => 8,
            VertexFormat::Float32x3 | VertexFormat::Uint32x3 => 12,
            VertexFormat::Float32x4 | VertexFormat::Uint32x4 => 16,
        }
    }
}

/// Describes a vertex attribute for dynamic vertex buffer layouts
#[derive(Clone, Debug)]
pub struct VertexAttribute {
    pub name: String,
    pub format: VertexFormat,
    pub location: u32,
}

/// Describes a complete vertex buffer layout
#[derive(Clone, Debug)]
pub struct VertexBufferLayout {
    pub attributes: Vec<VertexAttribute>,
    pub stride: u64,
}

impl VertexBufferLayout {
    pub fn new(attributes: Vec<VertexAttribute>) -> Self {
        let stride = attributes.iter().map(|a| a.format.size()).sum();
        Self { attributes, stride }
    }

    pub fn to_wgpu(&self) -> wgpu::VertexBufferLayout<'static> {
        let wgpu_attributes: Vec<wgpu::VertexAttribute> = self
            .attributes
            .iter()
            .enumerate()
            .map(|(i, attr)| {
                let offset = self.attributes[..i]
                    .iter()
                    .map(|a| a.format.size())
                    .sum::<u64>();
                wgpu::VertexAttribute {
                    format: attr.format.to_wgpu(),
                    offset,
                    shader_location: attr.location,
                }
            })
            .collect();

        // SAFETY: We leak the attributes vec to get a 'static lifetime
        // This is acceptable because vertex layouts are typically created once
        // and used throughout the application lifetime
        let static_attributes = Box::leak(wgpu_attributes.into_boxed_slice());

        wgpu::VertexBufferLayout {
            array_stride: self.stride,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: static_attributes,
        }
    }
}

/// Transform state for a visual
#[derive(Clone, Debug)]
pub struct Transform {
    pub matrix: Mat4,
}

impl Transform {
    pub fn identity() -> Self {
        Self {
            matrix: Mat4::IDENTITY,
        }
    }

    pub fn from_matrix(matrix: Mat4) -> Self {
        Self { matrix }
    }
}

impl Default for Transform {
    fn default() -> Self {
        Self::identity()
    }
}

/// Information about the current camera and viewport for LOD selection and culling
#[derive(Debug, Clone)]
pub struct CameraInfo {
    /// Camera position in world space
    pub position: glam::Vec3,
    /// Camera target/look-at point in world space
    pub target: glam::Vec3,
    /// Field of view in radians (vertical)
    pub fov_y: f32,
    /// Viewport width in pixels
    pub viewport_width: u32,
    /// Viewport height in pixels
    pub viewport_height: u32,
    /// View frustum planes for culling
    pub frustum: crate::camera::FrustumPlanes,
    /// Projection mode
    pub projection_mode: crate::ProjectionMode,
    /// Orthographic view height (only used when projection_mode is Orthographic)
    pub ortho_height: f32,
    /// Combined view * projection matrix. The renderer writes this to its
    /// shared camera UBO once per frame; visuals that need it (e.g. for
    /// CPU-side LOD selection from camera distance) read it here without
    /// having to inspect the bound bind group.
    pub view_proj: glam::Mat4,
}

/// How a visual's fragments combine with what's already in the framebuffer.
///
/// `Normal` is standard (premultiplied) alpha compositing — the default.
/// `Additive` sums contributions (`src + dst`), which is commutative and thus
/// order-independent; it is the basis for multi-channel compositing (render one
/// visual per channel, additively blended). Visuals that don't support blending
/// (e.g. points/lines) ignore this.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum BlendMode {
    #[default]
    Normal,
    Additive,
}

/// How the virtual-texture fragment shader turns an atlas sample into a color.
///
/// `Intensity` is the classic path: normalize the raw value by the contrast
/// limits, then look it up in the colormap LUT. `LabelHash` treats the raw
/// value as an integer segmentation ID (napari "Auto" mode): ID 0 is
/// transparent background, and any other ID is hashed to a stable colormap
/// coordinate. Contrast limits are ignored in `LabelHash` mode.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Default)]
pub enum ColorMode {
    #[default]
    Intensity,
    LabelHash,
}

impl ColorMode {
    /// Encoding shared with the shader's `color_mode` uniform.
    pub fn as_u32(self) -> u32 {
        match self {
            ColorMode::Intensity => 0,
            ColorMode::LabelHash => 1,
        }
    }
}

/// Base trait for all visual objects that can be rendered in a scene
#[cfg(not(target_arch = "wasm32"))]
pub trait Visual: Send + Sync + Any {
    /// Prepare the visual for rendering (called once per frame before render)
    /// This is where uniform buffers should be updated, etc.
    /// Camera info is provided for visuals that need LOD selection or culling
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &CameraInfo);

    /// Render the visual to the given render pass
    /// Note: This method cannot take 'self by reference with a lifetime
    /// because it needs to work with Mutex guards in the scene graph
    fn render(&self, render_pass: &mut RenderPass);

    /// Update the visual's transform matrix
    fn set_transform(&mut self, transform: Transform);

    /// Get the current transform
    fn transform(&self) -> &Transform;

    /// Check if the visual is visible
    fn is_visible(&self) -> bool;

    /// Set visibility
    fn set_visible(&mut self, visible: bool);

    /// Get a human-readable name for this visual (for debugging)
    fn name(&self) -> &str {
        "Unnamed Visual"
    }

    /// Set the blend mode for this visual. Default: no-op (Normal only).
    fn set_blend_mode(&mut self, _mode: BlendMode) {}

    /// Current blend mode. Default: `Normal`.
    fn blend_mode(&self) -> BlendMode {
        BlendMode::Normal
    }

    /// Set the per-visual opacity multiplier in `[0, 1]`. Default: no-op.
    fn set_opacity(&mut self, _opacity: f32) {}

    /// Current opacity. Default: `1.0`.
    fn opacity(&self) -> f32 {
        1.0
    }
}

/// Base trait for all visual objects that can be rendered in a scene (WASM version)
/// On WASM, wgpu types don't implement Send/Sync, so we can't require them
#[cfg(target_arch = "wasm32")]
pub trait Visual: Any {
    /// Prepare the visual for rendering (called once per frame before render)
    /// This is where uniform buffers should be updated, etc.
    /// Camera info is provided for visuals that need LOD selection or culling
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &CameraInfo);

    /// Render the visual to the given render pass
    /// Note: This method cannot take 'self by reference with a lifetime
    /// because it needs to work with Mutex guards in the scene graph
    fn render(&self, render_pass: &mut RenderPass);

    /// Update the visual's transform matrix
    fn set_transform(&mut self, transform: Transform);

    /// Get the current transform
    fn transform(&self) -> &Transform;

    /// Check if the visual is visible
    fn is_visible(&self) -> bool;

    /// Set visibility
    fn set_visible(&mut self, visible: bool);

    /// Get a human-readable name for this visual (for debugging)
    fn name(&self) -> &str {
        "Unnamed Visual"
    }

    /// Set the blend mode for this visual. Default: no-op (Normal only).
    fn set_blend_mode(&mut self, _mode: BlendMode) {}

    /// Current blend mode. Default: `Normal`.
    fn blend_mode(&self) -> BlendMode {
        BlendMode::Normal
    }

    /// Set the per-visual opacity multiplier in `[0, 1]`. Default: no-op.
    fn set_opacity(&mut self, _opacity: f32) {}

    /// Current opacity. Default: `1.0`.
    fn opacity(&self) -> f32 {
        1.0
    }
}
