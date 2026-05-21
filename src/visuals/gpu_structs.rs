//! Shared infrastructure for tile-based image rendering
//!
//! This module provides common types and utilities used by both simple and tiled
//! image strategies for managing and rendering image data in chunks/tiles.

use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use std::sync::Arc;
use wgpu::{BindGroup, Buffer, Texture, TextureView};

/// Unified request for a chunk/tile of image data
///
/// This structure supports both simple chunked loading (via voxel coordinates)
/// and multi-resolution tiled loading (via LOD level and chunk grid coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileRequest {
    /// LOD level (0 = highest resolution, None = no LOD hierarchy)
    pub lod_level: Option<usize>,

    /// X coordinate in chunk grid (for tiled) or voxel space (for simple chunks)
    pub x: u32,

    /// Y coordinate in chunk grid (for tiled) or voxel space (for simple chunks)
    pub y: u32,

    /// Z coordinate in chunk grid (for tiled) or voxel space (for simple chunks)
    pub z: u32,

    /// Width in voxels (optional, for simple chunks that specify size)
    pub width: Option<u32>,

    /// Height in voxels (optional, for simple chunks that specify size)
    pub height: Option<u32>,

    /// Depth in voxels (optional, for simple chunks that specify size)
    pub depth: Option<u32>,
}

impl TileRequest {
    /// Create a tile request for multiscale tiled loading
    pub fn from_grid(lod_level: usize, chunk_x: u32, chunk_y: u32, chunk_z: u32) -> Self {
        Self {
            lod_level: Some(lod_level),
            x: chunk_x,
            y: chunk_y,
            z: chunk_z,
            width: None,
            height: None,
            depth: None,
        }
    }

    /// Create a tile request for simple chunked loading with voxel coordinates
    pub fn from_voxels(x: u32, y: u32, z: u32, width: u32, height: u32, depth: u32) -> Self {
        Self {
            lod_level: None,
            x,
            y,
            z,
            width: Some(width),
            height: Some(height),
            depth: Some(depth),
        }
    }
}

/// Response containing tile/chunk data
#[derive(Debug, Clone)]
pub struct TileData {
    /// Raw voxel data as bytes (layout determined by `format`)
    pub data: Vec<u8>,

    /// Width of the data in voxels
    pub width: u32,

    /// Height of the data in voxels
    pub height: u32,

    /// Depth of the data in voxels (1 for 2D images)
    pub depth: u32,

    /// GPU texture format for this data (determines bytes-per-voxel)
    pub format: wgpu::TextureFormat,
}

impl TileData {
    /// Bytes per voxel for the stored format
    pub fn bytes_per_voxel(&self) -> u32 {
        match self.format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::R16Float => 2,
            wgpu::TextureFormat::R32Float => 4,
            _ => 1,
        }
    }
}

/// Axis-aligned bounding box in 3D space
#[derive(Debug, Clone, Copy)]
pub struct AABB {
    pub min: Vec3,
    pub max: Vec3,
}

impl AABB {
    pub fn new(min: Vec3, max: Vec3) -> Self {
        Self { min, max }
    }

    pub fn center(&self) -> Vec3 {
        (self.min + self.max) * 0.5
    }

    pub fn size(&self) -> Vec3 {
        self.max - self.min
    }
}

/// A single tile of image data with associated GPU resources
///
/// Tiles are the fundamental rendering unit. A simple image has one tile,
/// while a large tiled image has many tiles. Each tile has its own texture
/// and geometry, but shares the rendering pipeline with other tiles.
pub struct Tile {
    /// Unique identifier for this tile
    pub key: TileKey,

    /// GPU texture containing the voxel data
    pub texture: Arc<Texture>,

    /// Texture view for binding
    pub texture_view: Arc<TextureView>,

    /// World-space bounding box for this tile
    pub bounds: AABB,

    /// Size of this tile in voxels (Z, Y, X)
    pub voxel_count: (u32, u32, u32),

    /// Physical size of each voxel in world units (Z, Y, X)
    pub voxel_size: (f32, f32, f32),

    /// LOD level (0 = highest resolution, higher = coarser)
    pub lod_level: usize,

    /// Last frame this tile was accessed (for LRU eviction)
    pub last_used_frame: u64,

    /// Bind group for this tile's resources (texture, sampler, uniforms)
    /// Created on-demand during rendering
    pub bind_group: Option<Arc<BindGroup>>,

    /// Vertex buffer containing the tile's geometry (slice intersection polygon)
    /// Updated when slice plane changes
    pub vertex_buffer: Option<Buffer>,

    /// Number of vertices currently in the vertex buffer
    pub vertex_count: u32,

    /// Index buffer for rendering the tile geometry
    pub index_buffer: Option<Buffer>,

    /// Whether this tile is a placeholder (no real data loaded yet)
    pub is_placeholder: bool,

    /// Debug visualization color (for debugging LOD levels or spatial structure)
    pub debug_color: Option<[f32; 3]>,
}

impl Tile {
    /// Create a new tile with the given parameters
    pub fn new(
        key: TileKey,
        texture: Arc<Texture>,
        texture_view: Arc<TextureView>,
        bounds: AABB,
        voxel_count: (u32, u32, u32),
        voxel_size: (f32, f32, f32),
        lod_level: usize,
    ) -> Self {
        Self {
            key,
            texture,
            texture_view,
            bounds,
            voxel_count,
            voxel_size,
            lod_level,
            last_used_frame: 0,
            bind_group: None,
            vertex_buffer: None,
            vertex_count: 0,
            index_buffer: None,
            is_placeholder: false,
            debug_color: None,
        }
    }

    /// Mark this tile as accessed in the current frame
    pub fn touch(&mut self, frame: u64) {
        self.last_used_frame = frame;
    }
}

/// Unique identifier for a tile
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TileKey {
    /// LOD level (0 = highest resolution)
    pub lod_level: usize,

    /// Z coordinate in chunk grid
    pub z: u32,

    /// Y coordinate in chunk grid
    pub y: u32,

    /// X coordinate in chunk grid
    pub x: u32,
}

impl TileKey {
    pub fn new(lod_level: usize, z: u32, y: u32, x: u32) -> Self {
        Self { lod_level, z, y, x }
    }
}

/// Vertex for tile rendering (3D positioned quad with texture coordinates)
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TileVertex {
    pub position: [f32; 3],     // 3D position in world space
    pub texcoord: [f32; 3],     // 3D texture coordinates
}

impl TileVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<TileVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

// ── VT uniform types ─────────────────────────────────────────────────────────

pub const VT_MAX_LODS: usize = 16;

/// Per-LOD data in the VT uniforms block.
/// Layout must match `VTLodInfo` in `virtual_tile.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct VTLodInfo {
    /// Number of tiles in (x, y, z) at this LOD (ceil).
    pub grid_dims: [u32; 3],
    pub _pad: u32,
    /// tile_size / volume_size per axis (x, y, z).
    /// Used for exact sub-tile UV computation independent of grid rounding.
    pub tile_scale: [f32; 3],
    pub _pad2: f32,
    /// Fraction of the atlas slot actually filled with data for this LOD.
    /// = lod_tile_size / atlas_slot_size (element-wise).
    /// Coarser LODs have fewer voxels per chunk; multiplying within_tile by
    /// data_scale restricts sampling to the populated region of the slot.
    pub data_scale: [f32; 3],
    pub _pad3: f32,
}

/// Uniform block for the VT pipeline.
/// Layout must match `VTUniforms` in `virtual_tile.wgsl`.
///
/// Slot linearisation (cols-major, then rows, then layers):
///   col   = slot % atlas_cols
///   row   = (slot / atlas_cols) % atlas_rows
///   layer = slot / (atlas_cols * atlas_rows)
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct VTUniforms {
    pub atlas_cols: u32,
    pub atlas_rows: u32,
    /// UV advance per tile: 1/atlas_cols, 1/atlas_rows, 1/atlas_layers.
    pub atlas_tile_pitch_x: f32,
    pub atlas_tile_pitch_y: f32,
    pub atlas_tile_pitch_z: f32,
    pub lod_count: u32,
    pub contrast_min: f32,
    pub contrast_max: f32,
    /// Non-zero enables debug LOD tinting (green=LOD0, red=coarsest).
    pub debug_mode: u32,
    /// Width of the page table texture (for linear→2D index mapping in the shader).
    pub page_table_width: u32,
    /// CPU-computed ideal LOD for this frame (accounts for lod_bias and camera distance).
    /// The shader starts its page-table walk here rather than at LOD 0.
    pub target_lod: u32,
    pub _pad_c: u32,
    // Offset 48 — VTLodInfo has align 16, 48 mod 16 = 0 ✓
    pub lods: [VTLodInfo; VT_MAX_LODS],
}

impl Default for VTUniforms {
    fn default() -> Self {
        Self {
            atlas_cols: 1,
            atlas_rows: 1,
            atlas_tile_pitch_x: 1.0,
            atlas_tile_pitch_y: 1.0,
            atlas_tile_pitch_z: 1.0,
            lod_count: 1,
            contrast_min: 0.0,
            contrast_max: 1.0,
            debug_mode: 0,
            page_table_width: 1,
            target_lod: 0,
            _pad_c: 0,
            lods: [VTLodInfo { grid_dims: [1, 1, 1], _pad: 0, tile_scale: [1.0, 1.0, 1.0], _pad2: 0.0, data_scale: [1.0, 1.0, 1.0], _pad3: 0.0 }; VT_MAX_LODS],
        }
    }
}

/// Vertex for volume rendering — just world-space position.
/// The fragment shader derives volume UVs from the world position and AABB bounds.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct VolumeVertex {
    pub position: [f32; 3],
}

impl VolumeVertex {
    pub const ATTRIBS: [wgpu::VertexAttribute; 1] =
        wgpu::vertex_attr_array![0 => Float32x3];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<VolumeVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Uniform block for the volume ray-marching pipeline.
/// Layout must match `VolumeUniforms` in `volume_raymarch.wgsl`.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct VolumeUniforms {
    pub vol_min: [f32; 3],
    /// Step size in LOD-0 voxels. At coarser LODs the advance scales up so you always sample
    /// once per equivalent voxel. 1.0 = Nyquist at finest LOD; larger = faster but lower quality.
    /// The shader converts this to world-space units using lod0_dims.
    pub relative_step_size: f32,
    pub vol_max: [f32; 3],
    pub density_scale: f32,
    pub camera_pos: [f32; 3],
    /// Stop accumulating once front-to-back alpha exceeds this. 0.95 is a good default;
    /// lower saves steps in dense regions, higher is more accurate.
    pub early_exit_alpha: f32,
    /// Voxel dimensions of the full volume at LOD 0 (x, y, z).
    pub lod0_dims: [u32; 3],
    /// 0=normal DVR, 1=LOD tint + tile wireframe, 2=atlas direct, 3=step heatmap.
    /// Only honoured by the translucent pipeline.
    pub debug_mode: u32,
    /// Isosurface threshold in contrast-normalised raw units (0..1). Used by fs_iso.
    pub iso_threshold: f32,
    /// Attenuated-MIP falloff coefficient per world-space unit. Used by fs_attenuated_mip.
    pub attenuation: f32,
    pub _pad0: f32,
    pub _pad1: f32,
}

/// Shared uniforms for tile rendering
///
/// These uniforms are used by both simple and tiled strategies.
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct TileUniforms {
    /// Minimum value for contrast adjustment
    pub contrast_min: f32,

    /// Maximum value for contrast adjustment
    pub contrast_max: f32,

    /// Debug mode flag (0.0 = off, 1.0 = on)
    pub debug_mode: f32,

    pub _padding1: f32,

    /// Slice plane position in world space
    pub plane_position: [f32; 3],

    pub _padding2: f32,

    /// Slice plane normal vector
    pub plane_normal: [f32; 3],

    pub _padding3: f32,

    /// Volume size in voxels (Z, Y, X)
    pub volume_size: [f32; 3],

    pub _padding4: f32,

    /// Debug visualization color
    pub debug_color: [f32; 3],

    pub _padding5: f32,
}

impl Default for TileUniforms {
    fn default() -> Self {
        Self {
            contrast_min: 0.0,
            contrast_max: 1.0,
            debug_mode: 0.0,
            _padding1: 0.0,
            plane_position: [0.0, 0.0, 0.0],
            _padding2: 0.0,
            plane_normal: [0.0, 0.0, 1.0],
            _padding3: 0.0,
            volume_size: [1.0, 1.0, 1.0],
            _padding4: 0.0,
            debug_color: [1.0, 1.0, 1.0],
            _padding5: 0.0,
        }
    }
}

/// Status returned by tile loader callback
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChunkStatus {
    /// The chunk request was accepted and will be loaded asynchronously
    Accepted,
    /// The chunk is already being loaded (request is pending)
    AlreadyPending,
    /// The chunk request was rejected (e.g., at capacity)
    Rejected,
}

/// Type alias for tile loader callback
///
/// The callback receives a TileRequest and should return ChunkStatus indicating
/// whether the request was accepted, is already pending, or was rejected.
/// When the chunk is ready, the loader should call set_chunk_data() on the strategy.
///
/// IMPORTANT: The loader is responsible for implementing backpressure control
/// by returning ChunkStatus::Rejected when it cannot accept more requests.
#[cfg(not(target_arch = "wasm32"))]
pub type TileLoaderFn = Box<dyn Fn(TileRequest) -> ChunkStatus + Send + Sync>;

#[cfg(target_arch = "wasm32")]
pub type TileLoaderFn = Box<dyn Fn(TileRequest) -> ChunkStatus>;

// ============================================================================
// Geometry Utilities
// ============================================================================

/// Represents a slice plane in 3D space
#[derive(Debug, Clone, Copy)]
pub struct SlicePlane {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}

/// Compute the intersection polygon between a plane and an AABB
///
/// Returns vertices and indices for rendering the slice, or None if no intersection.
/// The vertices are sorted to form a convex polygon in counter-clockwise order.
pub fn compute_plane_aabb_intersection(
    plane: &SlicePlane,
    aabb: &AABB,
) -> Option<(Vec<TileVertex>, Vec<u16>)> {
    let plane_pos = Vec3::from(plane.position);
    let plane_normal = Vec3::from(plane.normal);
    let min = aabb.min;
    let max = aabb.max;

    // 12 edges of the AABB
    let edges = [
        // Bottom face (y = min)
        (Vec3::new(min.x, min.y, min.z), Vec3::new(max.x, min.y, min.z)),
        (Vec3::new(max.x, min.y, min.z), Vec3::new(max.x, min.y, max.z)),
        (Vec3::new(max.x, min.y, max.z), Vec3::new(min.x, min.y, max.z)),
        (Vec3::new(min.x, min.y, max.z), Vec3::new(min.x, min.y, min.z)),
        // Top face (y = max)
        (Vec3::new(min.x, max.y, min.z), Vec3::new(max.x, max.y, min.z)),
        (Vec3::new(max.x, max.y, min.z), Vec3::new(max.x, max.y, max.z)),
        (Vec3::new(max.x, max.y, max.z), Vec3::new(min.x, max.y, max.z)),
        (Vec3::new(min.x, max.y, max.z), Vec3::new(min.x, max.y, min.z)),
        // Vertical edges
        (Vec3::new(min.x, min.y, min.z), Vec3::new(min.x, max.y, min.z)),
        (Vec3::new(max.x, min.y, min.z), Vec3::new(max.x, max.y, min.z)),
        (Vec3::new(max.x, min.y, max.z), Vec3::new(max.x, max.y, max.z)),
        (Vec3::new(min.x, min.y, max.z), Vec3::new(min.x, max.y, max.z)),
    ];

    let mut intersection_points = Vec::new();

    // Find intersections with each edge
    for (p0, p1) in edges.iter() {
        if let Some(t) = line_plane_intersection(*p0, *p1, plane_pos, plane_normal) {
            if t >= 0.0 && t <= 1.0 {
                let point = p0.lerp(*p1, t);
                // Calculate 3D texture coordinate
                let texcoord = Vec3::new(
                    (point.x - min.x) / (max.x - min.x),
                    (point.y - min.y) / (max.y - min.y),
                    (point.z - min.z) / (max.z - min.z),
                );
                intersection_points.push((point, texcoord));
            }
        }
    }

    // Need at least 3 points for a polygon
    if intersection_points.len() < 3 {
        return None;
    }

    // Remove ALL duplicate points (not just consecutive ones)
    // Use epsilon relative to AABB size for scale-independence
    let aabb_diagonal = (max - min).length();
    let epsilon = aabb_diagonal * 0.001; // 0.1% of AABB diagonal
    let mut unique_points = Vec::new();
    for point in intersection_points {
        let is_duplicate = unique_points.iter().any(|(p, _): &(Vec3, Vec3)| {
            p.distance(point.0) < epsilon
        });
        if !is_duplicate {
            unique_points.push(point);
        }
    }

    if unique_points.len() < 3 {
        return None;
    }

    let intersection_points = unique_points;

    // Sort points to form a convex polygon
    let sorted_points = sort_points_on_plane(&intersection_points, plane_normal);

    // Convert to vertices
    let vertices: Vec<TileVertex> = sorted_points
        .iter()
        .map(|(pos, tex)| TileVertex {
            position: [pos.x, pos.y, pos.z],
            texcoord: [tex.x, tex.y, tex.z],
        })
        .collect();

    // Generate indices as a triangle fan (0,1,2, 0,2,3, 0,3,4, ...)
    let mut indices = Vec::new();
    for i in 1..(vertices.len() - 1) {
        indices.push(0);
        indices.push(i as u16);
        indices.push((i + 1) as u16);
    }

    Some((vertices, indices))
}

/// Find the intersection point of a line segment with a plane
///
/// Returns t value where intersection = p0 + t * (p1 - p0)
fn line_plane_intersection(p0: Vec3, p1: Vec3, plane_pos: Vec3, plane_normal: Vec3) -> Option<f32> {
    let ray_dir = p1 - p0;
    let denom = plane_normal.dot(ray_dir);

    // Line is parallel to plane
    if denom.abs() < 1e-6 {
        return None;
    }

    let t = plane_normal.dot(plane_pos - p0) / denom;
    Some(t)
}

/// Sort points to form a convex polygon in counter-clockwise order
fn sort_points_on_plane(points: &[(Vec3, Vec3)], plane_normal: Vec3) -> Vec<(Vec3, Vec3)> {
    if points.is_empty() {
        return Vec::new();
    }

    // Find centroid
    let centroid = points.iter().map(|(p, _)| *p).sum::<Vec3>() / points.len() as f32;

    // Create a coordinate system on the plane
    let up = if plane_normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };

    let tangent_unnormalized = plane_normal.cross(up);
    let tangent = if tangent_unnormalized.length() > 1e-6 {
        tangent_unnormalized.normalize()
    } else {
        // Fallback if cross product is too small
        Vec3::X
    };

    let bitangent_unnormalized = plane_normal.cross(tangent);
    let bitangent = if bitangent_unnormalized.length() > 1e-6 {
        bitangent_unnormalized.normalize()
    } else {
        Vec3::Y
    };

    // Sort by angle around centroid
    let mut sorted = points.to_vec();
    sorted.sort_by(|(a, _), (b, _)| {
        let va = *a - centroid;
        let vb = *b - centroid;

        let angle_a = va.dot(tangent).atan2(va.dot(bitangent));
        let angle_b = vb.dot(tangent).atan2(vb.dot(bitangent));

        // Handle NaN safely
        angle_a.partial_cmp(&angle_b).unwrap_or(std::cmp::Ordering::Equal)
    });

    sorted
}
