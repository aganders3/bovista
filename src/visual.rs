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
    /// Combined view * projection matrix. Visuals that pack camera data
    /// into their own combined uniform buffer (VolumeVisual, ImageVisual)
    /// read this directly instead of consuming the shared camera UBO.
    pub view_proj: glam::Mat4,
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
}
