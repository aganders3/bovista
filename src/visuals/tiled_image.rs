//! Tiled image rendering with on-demand chunk loading for massive volumes.
//!
//! This module implements a sophisticated system for rendering volumetric image data
//! that is too large to fit in memory. Key features:
//!
//! - **Spatial Partitioning**: Divides volume into a 3D grid of chunks
//! - **Visibility Culling**: Only loads chunks intersecting the current slice plane
//! - **LRU Caching**: Automatically evicts least-recently-used chunks when memory limit reached
//! - **Async Loading**: Chunks loaded via callback function (typically calling Python/Zarr)
//!
//! # Architecture
//!
//! ```text
//! ┌─────────────────────────────────────────────────┐
//! │          TiledImageVisual                        │
//! │  - Volume grid: 10×8×12 chunks                   │
//! │  - Max loaded: 150 chunks                        │
//! │  - Current slice: Z=5.2                          │
//! └─────────────────────────────────────────────────┘
//!          │
//!          ├─── update_visible_chunks()
//!          │    └─→ Find chunks intersecting slice plane
//!          │
//!          ├─── load_chunk()
//!          │    └─→ Call Python loader callback
//!          │         ├─ Fetch from remote Zarr/S3
//!          │         └─ Return uint8 array
//!          │
//!          ├─── evict_chunks()
//!          │    └─→ Remove LRU chunks if over limit
//!          │
//!          └─── render()
//!               └─→ Render all visible chunks
//! ```
//!
//! # Coordinate System
//!
//! Throughout this module, dimensions follow the pattern `(Z, Y, X)`:
//! - Z: depth (slice axis)
//! - Y: height (rows)
//! - X: width (columns)
//!
//! This matches NumPy/Zarr array indexing conventions.

use crate::camera::FrustumPlanes;
use crate::visual::{Transform, Visual};
use crate::visuals::image::SlicePlane;
use crate::visuals::chunk_visual::ChunkVisual;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use wgpu::RenderPass;

/// Metadata for a single LOD level in a multi-resolution pyramid
///
/// Each level represents the same volume at a different resolution.
/// Level 0 is the highest resolution, with each subsequent level typically
/// being downsampled by a factor of 2 in each dimension.
#[derive(Debug, Clone)]
pub struct LevelMetadata {
    /// Volume size at this LOD level (Z, Y, X) in voxels
    pub volume_size: (u32, u32, u32),

    /// Chunk size at this LOD level (Z, Y, X) in voxels
    pub chunk_size: (u32, u32, u32),

    /// Physical size of each voxel at this LOD level (Z, Y, X) in world units
    pub voxel_size: (f32, f32, f32),

    /// Grid dimensions (number of chunks in each direction)
    pub grid_size: (u32, u32, u32),

    /// Scale factor relative to LOD 0 (1.0, 2.0, 4.0, ...)
    /// This is how much larger voxels are at this level compared to LOD 0
    pub scale_factor: f32,
}

impl LevelMetadata {
    /// Create a new level metadata
    pub fn new(
        volume_size: (u32, u32, u32),
        chunk_size: (u32, u32, u32),
        voxel_size: (f32, f32, f32),
        scale_factor: f32,
    ) -> Self {
        // Calculate grid size
        let grid_size = (
            (volume_size.0 + chunk_size.0 - 1) / chunk_size.0,
            (volume_size.1 + chunk_size.1 - 1) / chunk_size.1,
            (volume_size.2 + chunk_size.2 - 1) / chunk_size.2,
        );

        Self {
            volume_size,
            chunk_size,
            voxel_size,
            grid_size,
            scale_factor,
        }
    }
}

/// Request for a chunk of data to be loaded.
///
/// Chunks are identified by their LOD level and integer grid coordinates in the volume.
/// The loader callback receives this request and should return the chunk data
/// or None if the chunk doesn't exist or can't be loaded.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkRequest {
    /// LOD level (0 = highest resolution)
    pub lod_level: usize,
    /// X coordinate in chunk grid at this LOD level
    pub chunk_x: u32,
    /// Y coordinate in chunk grid at this LOD level
    pub chunk_y: u32,
    /// Z coordinate in chunk grid at this LOD level
    pub chunk_z: u32,
}

/// Response containing loaded chunk data.
///
/// The data should be in row-major order (Z,Y,X) with uint8 values.
/// Width/height/depth specify the actual size of this chunk, which may be
/// smaller than the chunk grid size for edge chunks.
pub struct ChunkData {
    /// Raw uint8 voxel data in (Z,Y,X) order
    pub data: Vec<u8>,
    /// Actual width of this chunk in voxels
    pub width: u32,
    /// Actual height of this chunk in voxels
    pub height: u32,
    /// Actual depth of this chunk in voxels
    pub depth: u32,
}

/// Callback for loading chunks on-demand.
///
/// This function is called when a chunk needs to be loaded. It should:
/// 1. Fetch the chunk data (e.g., from Zarr array, network, disk)
/// 2. Convert to uint8 format if needed
/// 3. Return Some(ChunkData) on success or None if chunk unavailable
///
/// The callback may be called from the render thread, so it should be fast.
/// For slow I/O operations (like network fetches), consider using async loading
/// in the Python layer and returning None until data is ready.
pub type ChunkLoaderFn = Arc<dyn Fn(ChunkRequest) -> Option<ChunkData> + Send + Sync>;

/// Internal representation of a loaded chunk with metadata.
struct ImageChunk {
    visual: Arc<Mutex<ChunkVisual>>,
    #[allow(dead_code)]  // Kept for potential future spatial queries
    lod_level: usize,
    #[allow(dead_code)]  // Kept for potential future spatial queries
    chunk_index: (u32, u32, u32),
    #[allow(dead_code)]  // Kept for potential future spatial queries
    world_min: Vec3,
    #[allow(dead_code)]  // Kept for potential future spatial queries
    world_max: Vec3,
    last_accessed: usize,  // Frame counter for LRU
    is_placeholder: bool,  // True if this is a placeholder waiting for real data
}

/// Type alias for chunk cache key: (lod_level, z_chunk, y_chunk, x_chunk)
type ChunkKey = (usize, u32, u32, u32);

/// Tiled image visual that renders large volumes as separate chunks
///
/// Supports both single-LOD and multi-LOD rendering:
/// - **Single-LOD**: fixed_lod = Some(0), levels.len() == 1
/// - **Multi-LOD**: fixed_lod = None, spatial LOD selection based on camera
pub struct TiledImageVisual {
    /// All LOD levels (single-LOD has 1 element, multi-LOD has multiple)
    levels: Vec<LevelMetadata>,

    /// If Some(n), use only LOD level n (backward compatible single-LOD mode)
    /// If None, use spatial LOD selection based on camera distance
    fixed_lod: Option<usize>,

    /// Currently loaded chunks (key: lod_level, z, y, x)
    chunks: HashMap<ChunkKey, ImageChunk>,

    /// Chunk loader callback
    loader: ChunkLoaderFn,

    /// Slice plane for rendering
    slice_plane: SlicePlane,

    /// Contrast limits
    contrast_limits: (f32, f32),

    /// Maximum number of chunks to keep loaded
    max_loaded_chunks: usize,

    /// Frame counter for LRU tracking
    frame_counter: usize,

    /// Visibility tracking (key: lod_level, z, y, x)
    visible_chunks: HashSet<ChunkKey>,

    /// Debug mode
    debug_mode: bool,

    /// LOD selection parameters (only used when fixed_lod is None)
    lod_bias: f32,                  // User adjustment: negative = prefer high-res
    target_pixels_per_voxel: f32,   // Target screen-space density

    /// Visual properties
    transform: Transform,
    visible: bool,
    name: String,
}

impl TiledImageVisual {
    /// Create a new single-LOD tiled image visual (backward compatible)
    ///
    /// This creates a visual with only one resolution level.
    /// For multi-resolution rendering, use `from_levels` instead.
    pub fn new(
        volume_size: (u32, u32, u32),
        chunk_size: (u32, u32, u32),
        voxel_size: (f32, f32, f32),
        loader: ChunkLoaderFn,
        max_loaded_chunks: usize,
    ) -> Self {
        // Create a single LOD level with scale_factor = 1.0
        let level = LevelMetadata::new(volume_size, chunk_size, voxel_size, 1.0);

        Self {
            levels: vec![level],
            fixed_lod: Some(0),  // Fixed to LOD 0 (single-LOD mode)
            chunks: HashMap::new(),
            loader,
            slice_plane: SlicePlane::default(),
            contrast_limits: (0.0, 1.0),
            max_loaded_chunks,
            frame_counter: 0,
            visible_chunks: HashSet::new(),
            debug_mode: false,
            lod_bias: 0.0,
            target_pixels_per_voxel: 1.0,
            transform: Transform::default(),
            visible: true,
            name: "TiledImage".to_string(),
        }
    }

    /// Create a new multi-LOD tiled image visual
    ///
    /// This creates a visual that can render chunks from multiple resolution levels
    /// simultaneously, selecting the appropriate LOD based on camera distance and
    /// screen-space resolution.
    ///
    /// # Arguments
    ///
    /// * `levels` - Metadata for each LOD level (LOD 0 = highest resolution)
    /// * `loader` - Callback for loading chunks (receives ChunkRequest with lod_level)
    /// * `max_loaded_chunks` - Maximum chunks to keep in memory across all LODs
    /// * `lod_bias` - User preference for LOD selection (-2.0 = prefer high-res, +2.0 = prefer low-res)
    /// * `target_pixels_per_voxel` - Target screen-space density (typically 0.5-2.0)
    pub fn from_levels(
        levels: Vec<LevelMetadata>,
        loader: ChunkLoaderFn,
        max_loaded_chunks: usize,
        lod_bias: f32,
        target_pixels_per_voxel: f32,
    ) -> Self {
        assert!(!levels.is_empty(), "Must provide at least one LOD level");

        Self {
            levels,
            fixed_lod: None,  // Use spatial LOD selection
            chunks: HashMap::new(),
            loader,
            slice_plane: SlicePlane::default(),
            contrast_limits: (0.0, 1.0),
            max_loaded_chunks,
            frame_counter: 0,
            visible_chunks: HashSet::new(),
            debug_mode: false,
            lod_bias,
            target_pixels_per_voxel,
            transform: Transform::default(),
            visible: true,
            name: "TiledImage (Multi-LOD)".to_string(),
        }
    }

    /// Set the slice plane
    pub fn set_slice_plane(&mut self, plane: SlicePlane) {
        self.slice_plane = plane;

        // Update slice plane for all loaded chunks
        for chunk in self.chunks.values() {
            if let Ok(mut visual) = chunk.visual.lock() {
                visual.set_slice_plane(plane);
            }
        }
    }

    /// Set contrast limits
    pub fn set_contrast_limits(&mut self, min: f32, max: f32) {
        self.contrast_limits = (min, max);

        // Update contrast for all loaded chunks
        for chunk in self.chunks.values() {
            if let Ok(mut visual) = chunk.visual.lock() {
                visual.set_contrast_limits(min, max);
            }
        }
    }

    /// Set LOD bias for spatial LOD selection
    ///
    /// Only affects multi-LOD visuals (fixed_lod = None).
    /// Negative values prefer higher resolution, positive prefer lower resolution.
    pub fn set_lod_bias(&mut self, bias: f32) {
        self.lod_bias = bias;
    }

    /// Compute world-space AABB for a chunk at a specific LOD level
    fn compute_chunk_aabb(&self, lod_level: usize, chunk_idx: (u32, u32, u32)) -> (Vec3, Vec3) {
        let level = &self.levels[lod_level];
        let (z_chunk, y_chunk, x_chunk) = chunk_idx;

        // Min corner in world space
        let min = Vec3::new(
            (x_chunk * level.chunk_size.2) as f32 * level.voxel_size.2,  // X
            (y_chunk * level.chunk_size.1) as f32 * level.voxel_size.1,  // Y
            (z_chunk * level.chunk_size.0) as f32 * level.voxel_size.0,  // Z
        );

        // Max corner in world space
        let max = Vec3::new(
            ((x_chunk + 1) * level.chunk_size.2).min(level.volume_size.2) as f32 * level.voxel_size.2,
            ((y_chunk + 1) * level.chunk_size.1).min(level.volume_size.1) as f32 * level.voxel_size.1,
            ((z_chunk + 1) * level.chunk_size.0).min(level.volume_size.0) as f32 * level.voxel_size.0,
        );

        (min, max)
    }

    /// Compute which chunks at fine_lod overlap the given world-space AABB
    /// This is used for hierarchical traversal - instead of testing all fine chunks,
    /// we only test those that spatially overlap a coarse visible region.
    fn get_overlapping_chunks(
        &self,
        fine_lod: usize,
        coarse_aabb: (Vec3, Vec3),
    ) -> Vec<(u32, u32, u32)> {
        let level = &self.levels[fine_lod];
        let (min_world, max_world) = coarse_aabb;

        // Convert world coordinates to voxel coordinates at this LOD
        let min_voxel_x = (min_world.x / level.voxel_size.2).floor().max(0.0) as u32;
        let min_voxel_y = (min_world.y / level.voxel_size.1).floor().max(0.0) as u32;
        let min_voxel_z = (min_world.z / level.voxel_size.0).floor().max(0.0) as u32;

        let max_voxel_x = (max_world.x / level.voxel_size.2).ceil() as u32;
        let max_voxel_y = (max_world.y / level.voxel_size.1).ceil() as u32;
        let max_voxel_z = (max_world.z / level.voxel_size.0).ceil() as u32;

        // Convert voxel coordinates to chunk indices
        let min_cx = (min_voxel_x / level.chunk_size.2).min(level.grid_size.2.saturating_sub(1));
        let min_cy = (min_voxel_y / level.chunk_size.1).min(level.grid_size.1.saturating_sub(1));
        let min_cz = (min_voxel_z / level.chunk_size.0).min(level.grid_size.0.saturating_sub(1));

        let max_cx = (max_voxel_x / level.chunk_size.2).min(level.grid_size.2.saturating_sub(1));
        let max_cy = (max_voxel_y / level.chunk_size.1).min(level.grid_size.1.saturating_sub(1));
        let max_cz = (max_voxel_z / level.chunk_size.0).min(level.grid_size.0.saturating_sub(1));

        // Generate all chunk indices in this range
        let mut chunks = Vec::new();
        for cz in min_cz..=max_cz {
            for cy in min_cy..=max_cy {
                for cx in min_cx..=max_cx {
                    chunks.push((cz, cy, cx));
                }
            }
        }
        chunks
    }

    /// Select appropriate LOD level for a chunk based on screen-space resolution
    ///
    /// Returns the LOD level index (0 = highest resolution) that should be used
    /// for rendering a chunk at the given world-space position.
    ///
    /// # Arguments
    ///
    /// * `chunk_center` - Center point of the chunk in world space
    /// * `chunk_world_size` - Physical size of the chunk in world units
    /// * `camera_position` - Position of the camera in world space
    /// * `fov_y` - Vertical field of view in radians
    /// * `viewport_height` - Height of the viewport in pixels
    fn select_lod_for_chunk(
        &self,
        chunk_center: Vec3,
        chunk_world_size: f32,
        camera_position: Vec3,
        fov_y: f32,
        viewport_height: u32,
    ) -> usize {
        if self.fixed_lod.is_some() {
            return self.fixed_lod.unwrap();
        }

        let distance = (chunk_center - camera_position).length();
        if distance < 0.01 {
            return 0; // Very close, use highest resolution
        }

        // Screen-space size calculation
        // focal_length = viewport_height / (2 * tan(fov_y / 2))
        let focal_length = viewport_height as f32 / (2.0 * (fov_y / 2.0).tan());
        let screen_pixels = (chunk_world_size / distance) * focal_length;

        // Pixels per voxel at LOD 0
        let lod0 = &self.levels[0];
        let voxels_per_chunk = lod0.chunk_size.2 as f32; // Use X dimension
        let pixels_per_voxel = screen_pixels / voxels_per_chunk;

        // Select LOD: target is target_pixels_per_voxel, adjusted by lod_bias
        let target = self.target_pixels_per_voxel * 2.0_f32.powf(-self.lod_bias);

        for (lod, level) in self.levels.iter().enumerate() {
            let effective_ppv = pixels_per_voxel * level.scale_factor;
            if effective_ppv >= target {
                return lod;
            }
        }

        // Default to lowest resolution
        self.levels.len() - 1
    }

    /// Determine which LOD levels are relevant for the current view
    ///
    /// Returns (min_lod, max_lod) where min_lod is highest resolution needed
    /// and max_lod is lowest resolution needed.
    fn compute_relevant_lod_range(&self, camera_position: Vec3, fov_y: f32, viewport_height: u32) -> (usize, usize) {
        if self.fixed_lod.is_some() {
            let lod = self.fixed_lod.unwrap();
            return (lod, lod);
        }

        // Compute distance from camera to slice plane
        let plane_normal = Vec3::from_array(self.slice_plane.normal);
        let plane_point = Vec3::from_array(self.slice_plane.position);

        // Distance from camera to plane
        let distance_to_plane = (camera_position - plane_point).dot(plane_normal).abs();

        // Estimate the extent of the visible region on the slice
        // Use the volume diagonal as a conservative estimate
        let lod0 = &self.levels[0];
        let volume_diagonal = Vec3::new(
            lod0.volume_size.2 as f32 * lod0.voxel_size.2,
            lod0.volume_size.1 as f32 * lod0.voxel_size.1,
            lod0.volume_size.0 as f32 * lod0.voxel_size.0,
        ).length();

        // Closest point: on the plane near camera
        let min_distance = distance_to_plane.max(1.0);

        // Farthest point: diagonal distance across volume
        let max_distance = (distance_to_plane + volume_diagonal).max(min_distance + 1.0);

        // Use typical chunk size for LOD selection
        let typical_chunk_size = lod0.chunk_size.2 as f32 * lod0.voxel_size.2;

        // Select LOD for closest and farthest points
        let min_lod = self.select_lod_for_chunk(
            camera_position,
            typical_chunk_size,
            camera_position + plane_normal * min_distance,
            fov_y,
            viewport_height,
        );

        let max_lod = self.select_lod_for_chunk(
            camera_position,
            typical_chunk_size,
            camera_position + plane_normal * max_distance,
            fov_y,
            viewport_height,
        );

        // Add a buffer of ±1 LOD to be conservative
        let min_lod = min_lod.saturating_sub(1);
        let max_lod = (max_lod + 1).min(self.levels.len() - 1);

        (min_lod, max_lod)
    }

    /// Calculate which chunks are visible based on frustum culling and slice plane intersection
    ///
    /// For multi-LOD visuals, this also performs spatial LOD selection to determine
    /// which LOD level to use for each spatial region.
    fn update_visible_chunks(&mut self, frustum: Option<&FrustumPlanes>, camera_position: Vec3, fov_y: f32, viewport_height: u32) {
        self.visible_chunks.clear();

        // For single-LOD mode, just test the single level
        if let Some(fixed) = self.fixed_lod {
            let level = &self.levels[fixed];
            for cz in 0..level.grid_size.0 {
                for cy in 0..level.grid_size.1 {
                    for cx in 0..level.grid_size.2 {
                        let (min, max) = self.compute_chunk_aabb(fixed, (cz, cy, cx));

                        // Frustum culling (skip near/far planes for slice visualization)
                        // Add a safety margin to avoid incorrectly culling visible chunks
                        if let Some(f) = frustum {
                            // Expand AABB by 20% to be more conservative
                            let margin = (max - min) * 0.2;
                            let expanded_min = min - margin;
                            let expanded_max = max + margin;

                            if !f.contains_aabb_xy(expanded_min, expanded_max) {
                                continue;
                            }
                        }

                        // Slice plane intersection
                        if plane_aabb_intersection(&self.slice_plane, min, max) {
                            self.visible_chunks.insert((fixed, cz, cy, cx));
                        }
                    }
                }
            }
            return;
        }

        // Multi-LOD mode: Determine relevant LOD range based on camera distance
        let (min_lod, max_lod) = self.compute_relevant_lod_range(camera_position, fov_y, viewport_height);

        // Print LOD configuration on first frame
        if self.frame_counter == 1 {
            eprintln!("[TILED] Multi-LOD configuration:");
            for (i, level) in self.levels.iter().enumerate() {
                let total = level.grid_size.0 as usize * level.grid_size.1 as usize * level.grid_size.2 as usize;
                eprintln!("  LOD{}: {}×{}×{} = {} chunks (scale factor: {}x)",
                    i,
                    level.grid_size.0,
                    level.grid_size.1,
                    level.grid_size.2,
                    total,
                    level.scale_factor);
            }
        }

        // STRATEGY: Test the ideal LOD, but also keep any already-loaded chunks from other LODs visible
        // This ensures smooth transitions - we show "wrong" LOD data until "right" LOD loads
        let ideal_lod = min_lod;

        // Print selected LOD
        if self.frame_counter % 60 == 0 {
            eprintln!("[TILED] Relevant LOD range: LOD{} to LOD{}, ideal is LOD{}",
                min_lod, max_lod, ideal_lod);
        }

        let start_time = std::time::Instant::now();
        let mut tested_total = 0;
        let mut found_total = 0;

        // First pass: Mark already-loaded chunks from ANY LOD as visible if they intersect
        // This prevents data from disappearing during LOD transitions
        // Depth test will naturally hide low-res chunks behind high-res ones
        for (chunk_key, _chunk) in &self.chunks {
            let (lod_level, cz, cy, cx) = *chunk_key;
            let (min, max) = self.compute_chunk_aabb(lod_level, (cz, cy, cx));

            // Basic visibility test
            if let Some(f) = frustum {
                let margin = (max - min) * 0.2;
                if !f.contains_aabb_xy(min - margin, max + margin) {
                    continue;
                }
            }

            if plane_aabb_intersection(&self.slice_plane, min, max) {
                self.visible_chunks.insert(*chunk_key);
                found_total += 1;
            }
        }

        // Second pass: Test the ideal LOD to identify what SHOULD be loaded
        // This determines load priority but doesn't hide already-loaded chunks
        let test_level = &self.levels[ideal_lod];
        let grid_size = test_level.grid_size.0 as usize * test_level.grid_size.1 as usize * test_level.grid_size.2 as usize;

        if self.frame_counter % 60 == 0 {
            eprintln!("[TILED] Testing ideal LOD{} ({} total chunks, will be culled)", ideal_lod, grid_size);
        }

        for cz in 0..test_level.grid_size.0 {
            for cy in 0..test_level.grid_size.1 {
                for cx in 0..test_level.grid_size.2 {
                    tested_total += 1;
                    let (min, max) = self.compute_chunk_aabb(ideal_lod, (cz, cy, cx));

                    // Frustum culling
                    if let Some(f) = frustum {
                        let margin = (max - min) * 0.2;
                        if !f.contains_aabb_xy(min - margin, max + margin) {
                            continue;
                        }
                    }

                    // Slice plane intersection
                    if !plane_aabb_intersection(&self.slice_plane, min, max) {
                        continue;
                    }

                    // Mark this ideal-LOD chunk as visible
                    let chunk_key = (ideal_lod, cz, cy, cx);
                    if !self.visible_chunks.contains(&chunk_key) {
                        self.visible_chunks.insert(chunk_key);
                        found_total += 1;
                    }
                }
            }
        }

        let elapsed = start_time.elapsed();
        if self.frame_counter % 60 == 0 {
            eprintln!("[TILED] Visibility testing took {}ms, found {} total visible chunks",
                elapsed.as_millis(), found_total);
        }

        // Emergency brake: if testing took too long, warn user
        if elapsed.as_millis() > 16 {
            eprintln!("[TILED] WARNING: Visibility testing took {}ms (>16ms frame budget)! Consider implementing hierarchical traversal.",
                elapsed.as_millis());
        }
    }

    /// Check if a chunk at a specific LOD intersects with the current slice plane
    fn chunk_intersects_slice(&self, lod_level: usize, chunk_idx: (u32, u32, u32)) -> bool {
        let (min, max) = self.compute_chunk_aabb(lod_level, chunk_idx);
        plane_aabb_intersection(&self.slice_plane, min, max)
    }

    /// Old signature for backward compatibility (assumes LOD 0)
    #[allow(dead_code)]
    fn chunk_intersects_slice_compat(&self, chunk_idx: (u32, u32, u32)) -> bool {
        let lod = self.fixed_lod.unwrap_or(0);
        self.chunk_intersects_slice(lod, chunk_idx)
    }

    /// Load a chunk from the loader callback
    fn load_chunk(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        chunk_key: ChunkKey,
    ) -> bool {
        // chunk_key is (lod_level, z_chunk, y_chunk, x_chunk)
        let (lod_level, z_chunk, y_chunk, x_chunk) = chunk_key;
        let level = &self.levels[lod_level];

        // Validate chunk indices are within grid bounds for this LOD
        if z_chunk >= level.grid_size.0 || y_chunk >= level.grid_size.1 || x_chunk >= level.grid_size.2 {
            eprintln!(
                "WARNING: Attempted to load out-of-bounds chunk LOD {} ({}, {}, {}) but grid size is ({}, {}, {})",
                lod_level, z_chunk, y_chunk, x_chunk,
                level.grid_size.0, level.grid_size.1, level.grid_size.2
            );
            return false;
        }

        // Create ChunkRequest - this will be passed to Python
        let request = ChunkRequest {
            lod_level,
            chunk_x: x_chunk,
            chunk_y: y_chunk,
            chunk_z: z_chunk,
        };

        if let Some(chunk_data) = (self.loader)(request) {
            // Calculate world-space position using this LOD's voxel_size
            let (world_min, world_max) = self.compute_chunk_aabb(lod_level, (z_chunk, y_chunk, x_chunk));

            // Create ChunkVisual for this chunk with 3D positioning
            let mut visual = ChunkVisual::new(
                device,
                queue,
                surface_format,
                camera_bind_group_layout,
                &chunk_data.data,
                chunk_data.width,
                chunk_data.height,
                chunk_data.depth,
                level.voxel_size,
                world_min,
            );

            // Apply current slice plane, contrast, and debug mode
            visual.set_slice_plane(self.slice_plane);
            visual.set_contrast_limits(self.contrast_limits.0, self.contrast_limits.1);
            visual.set_debug_mode(self.debug_mode);

            // Store the chunk (this may replace a placeholder)
            let was_placeholder = self.chunks.get(&chunk_key).map(|c| c.is_placeholder).unwrap_or(false);

            let chunk = ImageChunk {
                visual: Arc::new(Mutex::new(visual)),
                lod_level,
                chunk_index: (z_chunk, y_chunk, x_chunk),
                world_min,
                world_max,
                last_accessed: self.frame_counter,
                is_placeholder: false,
            };

            self.chunks.insert(chunk_key, chunk);

            if was_placeholder {
                eprintln!("[TILED] Replaced placeholder with real data for LOD{} chunk ({},{},{})",
                    lod_level, z_chunk, y_chunk, x_chunk);
            }

            true
        } else {
            // Data not available yet - create placeholder if in debug mode
            if self.debug_mode {
                eprintln!("[TILED] Creating placeholder for LOD{} chunk ({},{},{})", lod_level, z_chunk, y_chunk, x_chunk);
                self.create_placeholder_chunk(device, queue, surface_format, camera_bind_group_layout, chunk_key);
            }
            false
        }
    }

    /// Create a placeholder chunk with a solid color for debugging
    fn create_placeholder_chunk(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        chunk_key: ChunkKey,
    ) {
        let (lod_level, z_chunk, y_chunk, x_chunk) = chunk_key;
        let level = &self.levels[lod_level];

        // Don't create placeholder if already exists
        if self.chunks.contains_key(&chunk_key) {
            eprintln!("[TILED] Skipping placeholder - chunk already exists");
            return;
        }

        // Use the actual chunk size from the level, not a small placeholder
        let size = level.chunk_size;
        // Create bright white placeholder
        let placeholder_data: Vec<u8> = vec![255; (size.0 * size.1 * size.2) as usize];

        let (world_min, world_max) = self.compute_chunk_aabb(lod_level, (z_chunk, y_chunk, x_chunk));

        eprintln!("[TILED] Creating placeholder with size {:?} at world pos {:?} to {:?}", size, world_min, world_max);

        // Create ChunkVisual with placeholder data
        let mut visual = ChunkVisual::new(
            device,
            queue,
            surface_format,
            camera_bind_group_layout,
            &placeholder_data,
            size.2,  // width (X)
            size.1,  // height (Y)
            size.0,  // depth (Z)
            level.voxel_size,
            world_min,
        );

        visual.set_slice_plane(self.slice_plane);
        // Set high contrast to make placeholder very visible
        visual.set_contrast_limits(0.0, 1.0);
        visual.set_debug_mode(self.debug_mode);

        let chunk = ImageChunk {
            visual: Arc::new(Mutex::new(visual)),
            lod_level,
            chunk_index: (z_chunk, y_chunk, x_chunk),
            world_min,
            world_max,
            last_accessed: self.frame_counter,
            is_placeholder: true,
        };

        self.chunks.insert(chunk_key, chunk);
        eprintln!("[TILED] Placeholder created and inserted into chunks map");
    }

    /// Evict chunks using LRU policy
    fn evict_chunks(&mut self) {
        // Only evict when we're significantly over capacity to avoid running every frame
        // Use a threshold of 10% over capacity
        let threshold = self.max_loaded_chunks + (self.max_loaded_chunks / 10).max(10);
        if self.chunks.len() <= threshold {
            return;
        }

        let start_time = std::time::Instant::now();

        // Collect chunks with their access times
        let mut chunks_by_access: Vec<_> = self
            .chunks
            .iter()
            .map(|(idx, chunk)| (*idx, chunk.last_accessed))
            .collect();

        // Sort by last accessed (oldest first)
        chunks_by_access.sort_by_key(|(_, accessed)| *accessed);

        // Remove oldest chunks until we're under the limit
        let num_to_remove = self.chunks.len() - self.max_loaded_chunks;
        for (chunk_idx, _) in chunks_by_access.iter().take(num_to_remove) {
            self.chunks.remove(chunk_idx);
        }

        let elapsed = start_time.elapsed();
        if elapsed.as_millis() > 5 {
            eprintln!("[TILED] Eviction took {}ms to remove {} chunks from {} total",
                elapsed.as_millis(), num_to_remove, chunks_by_access.len());
        }
    }

    /// Prepare for rendering - load visible chunks, evict old ones
    ///
    /// For multi-LOD visuals, you must provide camera and viewport information
    /// for spatial LOD selection. For single-LOD visuals, these can be default values.
    pub fn prepare_chunks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        frustum: Option<&FrustumPlanes>,
        camera_position: Vec3,
        fov_y: f32,
        viewport_height: u32,
    ) {
        self.frame_counter += 1;

        // Update which chunks are visible (with optional frustum culling and LOD selection)
        self.update_visible_chunks(frustum, camera_position, fov_y, viewport_height);

        // Debug: Print LOD distribution and coverage (only occasionally to avoid spam)
        if self.frame_counter % 60 == 0 {
            if self.fixed_lod.is_none() {
                let mut lod_counts = vec![0; self.levels.len()];
                for (lod, _, _, _) in &self.visible_chunks {
                    lod_counts[*lod] += 1;
                }
                let lod_str: Vec<String> = lod_counts.iter().enumerate()
                    .filter(|(_, &count)| count > 0)
                    .map(|(lod, count)| format!("LOD{}: {}", lod, count))
                    .collect();
                if !lod_str.is_empty() {
                    eprintln!("[TILED] Visible chunks: {} ({})", self.visible_chunks.len(), lod_str.join(", "));
                }
            }

            // Print loading progress
            let loaded = self.chunks.len();
            let visible = self.visible_chunks.len();
            let unloaded = visible.saturating_sub(loaded);
            if unloaded > 0 {
                eprintln!("[TILED] Loading progress: {}/{} loaded ({} pending)", loaded, visible, unloaded);
            }
        }

        // Load visible chunks that aren't loaded yet
        // IMPORTANT: Limit how many we load per frame to avoid blocking main thread
        const MAX_LOADS_PER_FRAME: usize = 4;

        // Sort visible chunks by LOD (high-res first) for load prioritization
        let mut visible: Vec<_> = self.visible_chunks.iter().copied().collect();
        visible.sort_by_key(|(lod, _, _, _)| *lod); // LOD 0 first, then 1, 2, etc.

        let mut loads_this_frame = 0;
        let mut skipped = 0;

        for chunk_key in visible {
            let needs_load = if let Some(chunk) = self.chunks.get(&chunk_key) {
                // Chunk exists - check if it's a placeholder that needs replacing
                chunk.is_placeholder
            } else {
                // Chunk doesn't exist - needs to be loaded
                true
            };

            if needs_load {
                // Limit concurrent loads to avoid blocking
                if loads_this_frame < MAX_LOADS_PER_FRAME {
                    self.load_chunk(device, queue, surface_format, camera_bind_group_layout, chunk_key);
                    loads_this_frame += 1;
                } else {
                    skipped += 1;
                }
            } else {
                // Chunk is loaded (not a placeholder) - just update access time
                if let Some(chunk) = self.chunks.get_mut(&chunk_key) {
                    chunk.last_accessed = self.frame_counter;
                }
            }
        }

        if self.frame_counter % 60 == 0 && (loads_this_frame > 0 || skipped > 0) {
            eprintln!("[TILED] Frame {}: Loaded {} chunks, skipped {} (rate limited)",
                self.frame_counter, loads_this_frame, skipped);

            // Show what LODs are loaded vs visible
            let mut loaded_by_lod = vec![0; self.levels.len()];
            let mut visible_by_lod = vec![0; self.levels.len()];
            for (chunk_key, _) in &self.chunks {
                loaded_by_lod[chunk_key.0] += 1;
            }
            for (lod, _, _, _) in &self.visible_chunks {
                visible_by_lod[*lod] += 1;
            }
            eprintln!("[TILED] Loaded by LOD: {:?}", loaded_by_lod);
            eprintln!("[TILED] Visible by LOD: {:?}", visible_by_lod);
        }

        // Evict excess chunks
        self.evict_chunks();

        // Prepare all loaded chunks
        for chunk in self.chunks.values() {
            if let Ok(mut visual) = chunk.visual.lock() {
                visual.prepare(device, queue);
            }
        }
    }

    /// Get the number of currently loaded chunks
    pub fn loaded_chunk_count(&self) -> usize {
        self.chunks.len()
    }

    /// Get the number of visible chunks
    pub fn visible_chunk_count(&self) -> usize {
        self.visible_chunks.len()
    }

    /// Set debug mode for all chunks
    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = enabled;
        for chunk in self.chunks.values() {
            if let Ok(mut visual) = chunk.visual.lock() {
                visual.set_debug_mode(enabled);
            }
        }
    }
}

impl Visual for TiledImageVisual {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        // Note: This doesn't have access to surface_format or camera_bind_group_layout
        // Those need to be provided separately via prepare_chunks()
        // For now, just prepare existing chunks
        for chunk in self.chunks.values() {
            if let Ok(mut visual) = chunk.visual.lock() {
                visual.prepare(device, queue);
            }
        }
    }

    fn render(&self, render_pass: &mut RenderPass) {
        if !self.visible {
            return;
        }

        // Collect visible chunks and sort by LOD (low-res first, then high-res)
        // With LessEqual depth test, last-drawn wins at equal depth
        // For coplanar slices, we want high-res to overwrite low-res, so:
        // 1. Draw LOD 9 (lowest res) first
        // 2. Draw LOD 0 (highest res) last - overwrites everything
        let mut chunks_by_lod: Vec<Vec<_>> = vec![Vec::new(); self.levels.len()];
        for (chunk_key, chunk) in &self.chunks {
            if self.visible_chunks.contains(chunk_key) {
                chunks_by_lod[chunk_key.0].push((chunk_key, chunk));
            }
        }

        // Render low-res to high-res (reverse LOD order)
        for lod in (0..self.levels.len()).rev() {
            for (_, chunk) in &chunks_by_lod[lod] {
                if let Ok(visual) = chunk.visual.lock() {
                    visual.render(render_pass);
                }
            }
        }
    }

    fn transform(&self) -> &Transform {
        &self.transform
    }

    fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
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

/// Test if an AABB intersects with a plane
fn plane_aabb_intersection(plane: &SlicePlane, min: Vec3, max: Vec3) -> bool {
    let plane_pos = Vec3::from(plane.position);
    let plane_normal = Vec3::from(plane.normal);

    // Get the positive and negative vertices relative to plane normal
    let mut p_vertex = min;
    let mut n_vertex = max;

    if plane_normal.x >= 0.0 {
        p_vertex.x = max.x;
        n_vertex.x = min.x;
    }
    if plane_normal.y >= 0.0 {
        p_vertex.y = max.y;
        n_vertex.y = min.y;
    }
    if plane_normal.z >= 0.0 {
        p_vertex.z = max.z;
        n_vertex.z = min.z;
    }

    // Calculate signed distances
    let d_pos = plane_normal.dot(p_vertex - plane_pos);
    let d_neg = plane_normal.dot(n_vertex - plane_pos);

    // AABB intersects the plane if it straddles it (different signs)
    // The plane must be between the negative and positive vertices
    d_neg <= 0.0 && d_pos >= 0.0
}
