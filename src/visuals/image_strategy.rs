//! Image loading strategies using the tile-based architecture
//!
//! This module provides different strategies for loading and managing image data:
//! - SimpleStrategy: For small images that fit in memory
//! - TiledStrategy: For large images with on-demand loading and LOD support

use crate::visuals::tile::{
    ChunkStatus, Tile, TileData, TileKey, TileLoaderFn, TileRequest, AABB,
    compute_plane_aabb_intersection,
};
use crate::visuals::image::SlicePlane;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use wgpu::{Device, Queue};

/// Type alias for pending chunks queue (can be shared across threads)
pub type PendingChunks = Arc<Mutex<HashMap<TileKey, TileData>>>;

/// Image loading and rendering strategy
///
/// This enum provides different strategies for loading and managing image data:
/// - Simple: For small images that fit in GPU memory
/// - Tiled: For large images with on-demand loading and LOD support
pub enum ImageStrategy {
    Simple(SimpleData),
    Tiled(TiledData),
}

/// Data for simple strategy
pub struct SimpleData {
    tile: Tile,
    dimensions: (u32, u32, u32),
}

impl SimpleData {
    pub fn new(
        device: &Device,
        queue: &Queue,
        data: &[u8],
        width: u32,
        height: u32,
        depth: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Simple Image Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,  // Always use 3D for consistency with tile shader
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Calculate bytes per pixel based on format
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Rg8Unorm => 2,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => 4,
            wgpu::TextureFormat::R16Float => 2,
            wgpu::TextureFormat::R32Float => 4,
            _ => 4, // Default fallback
        };

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes_per_pixel),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create a single tile covering the entire image
        let bounds = AABB::new(
            Vec3::ZERO,
            Vec3::new(width as f32, height as f32, depth as f32),
        );

        let tile = Tile::new(
            TileKey::new(0, 0, 0, 0), // LOD 0, single tile at origin
            Arc::new(texture),
            Arc::new(texture_view),
            bounds,
            (depth, height, width), // (Z, Y, X)
            (1.0, 1.0, 1.0),        // voxel size
            0,                       // LOD level
        );

        Self {
            tile,
            dimensions: (depth, height, width),
        }
    }
}

impl SimpleData {
    fn prepare(
        &mut self,
        device: &Device,
        _queue: &Queue,
        slice_plane: &SlicePlane,
        _frame_number: u64,
        _camera_info: &crate::visual::CameraInfo,
    ) {
        // For now, create a simple quad geometry for the entire volume
        // TODO: Implement proper plane-AABB intersection for arbitrary slice planes

        use crate::visuals::tile::TileVertex;
        use wgpu::util::DeviceExt;

        // For simplicity, assume slice plane is axis-aligned (XY, XZ, or YZ)
        // Create a quad that spans the full volume at the slice position
        let bounds = &self.tile.bounds;
        let (depth, height, width) = self.tile.voxel_count;

        // Determine which axis the slice is along based on the normal
        let normal = slice_plane.normal;
        let position = slice_plane.position;

        // Simple heuristic: check which component of normal is largest
        let abs_x = normal[0].abs();
        let abs_y = normal[1].abs();
        let abs_z = normal[2].abs();

        let vertices = if abs_z > abs_x && abs_z > abs_y {
            // XY plane (normal along Z)
            let z = position[2].clamp(bounds.min.z, bounds.max.z);
            let tz = (z / depth as f32).clamp(0.0, 1.0);

            vec![
                TileVertex { position: [bounds.min.x, bounds.min.y, z], texcoord: [0.0, 0.0, tz] },
                TileVertex { position: [bounds.max.x, bounds.min.y, z], texcoord: [1.0, 0.0, tz] },
                TileVertex { position: [bounds.max.x, bounds.max.y, z], texcoord: [1.0, 1.0, tz] },
                TileVertex { position: [bounds.min.x, bounds.max.y, z], texcoord: [0.0, 1.0, tz] },
            ]
        } else if abs_y > abs_x && abs_y > abs_z {
            // XZ plane (normal along Y)
            let y = position[1].clamp(bounds.min.y, bounds.max.y);
            let ty = (y / height as f32).clamp(0.0, 1.0);

            vec![
                TileVertex { position: [bounds.min.x, y, bounds.min.z], texcoord: [0.0, ty, 0.0] },
                TileVertex { position: [bounds.max.x, y, bounds.min.z], texcoord: [1.0, ty, 0.0] },
                TileVertex { position: [bounds.max.x, y, bounds.max.z], texcoord: [1.0, ty, 1.0] },
                TileVertex { position: [bounds.min.x, y, bounds.max.z], texcoord: [0.0, ty, 1.0] },
            ]
        } else {
            // YZ plane (normal along X)
            let x = position[0].clamp(bounds.min.x, bounds.max.x);
            let tx = (x / width as f32).clamp(0.0, 1.0);

            vec![
                TileVertex { position: [x, bounds.min.y, bounds.min.z], texcoord: [tx, 0.0, 0.0] },
                TileVertex { position: [x, bounds.max.y, bounds.min.z], texcoord: [tx, 1.0, 0.0] },
                TileVertex { position: [x, bounds.max.y, bounds.max.z], texcoord: [tx, 1.0, 1.0] },
                TileVertex { position: [x, bounds.min.y, bounds.max.z], texcoord: [tx, 0.0, 1.0] },
            ]
        };

        let indices: [u16; 6] = [0, 1, 2, 0, 2, 3];

        // Create or update vertex buffer
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Simple Strategy Vertex Buffer"),
            contents: bytemuck::cast_slice(&vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });

        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Simple Strategy Index Buffer"),
            contents: bytemuck::cast_slice(&indices),
            usage: wgpu::BufferUsages::INDEX,
        });

        self.tile.vertex_buffer = Some(vertex_buffer);
        self.tile.index_buffer = Some(index_buffer);
        self.tile.vertex_count = 6; // 6 indices for 2 triangles
    }
}

/// Configuration for a single LOD level in the pyramid
#[derive(Debug, Clone)]
pub struct LodLevelConfig {
    /// Volume size at this LOD level (Z, Y, X) in voxels
    pub volume_size: (u32, u32, u32),
    /// Tile size at this LOD level (Z, Y, X) in voxels
    pub tile_size: (u32, u32, u32),
    /// Physical size of each voxel at this LOD level (Z, Y, X) in world units
    pub voxel_size: (f32, f32, f32),
    /// Scale factor relative to LOD 0 (1.0, 2.0, 4.0, ...)
    pub scale_factor: f32,
    /// Translation offset (Z, Y, X) in world units - for mapping data space to world space
    pub translation: (f32, f32, f32),
}

impl LodLevelConfig {
    /// Calculate grid size (number of tiles in each dimension)
    pub fn grid_size(&self) -> (u32, u32, u32) {
        let (vz, vy, vx) = self.volume_size;
        let (tz, ty, tx) = self.tile_size;
        (
            (vz + tz - 1) / tz,
            (vy + ty - 1) / ty,
            (vx + tx - 1) / tx,
        )
    }
}

/// Tiled strategy for large images with on-demand loading
///
/// This strategy:
/// - Divides the image into a spatial grid of tiles
/// - Loads tiles on-demand based on visibility
/// - Supports multiple LOD levels (multi-resolution pyramid)
/// - Uses LRU caching to manage memory
pub struct TiledData {
    /// All LOD levels (from high-res to low-res)
    levels: Vec<LodLevelConfig>,

    /// Currently loaded tiles
    tiles: Vec<Tile>,

    /// Map from TileKey to index in tiles vec
    tile_cache: HashMap<TileKey, usize>,

    /// Pending chunks waiting to have textures created
    /// Map from TileKey -> TileData (raw data, not yet uploaded to GPU)
    /// Uses separate mutex so background threads can submit without blocking on visual lock
    pending_chunks: PendingChunks,

    /// Tracks which chunks we've requested (to return AlreadyPending)
    requested_keys: HashSet<TileKey>,

    /// Tile loader callback
    loader: TileLoaderFn,

    /// Maximum number of tiles to keep loaded
    max_tiles: usize,

    /// Frame counter for LRU tracking
    frame_counter: u64,

    /// Set of visible tile keys (updated each frame)
    visible_tile_keys: HashSet<TileKey>,

    /// LOD selection parameters
    lod_bias: f32,  // User adjustment: negative = prefer high-res
    target_pixels_per_voxel: f32,  // Target screen-space density

    /// Cached ideal LOD from last update
    cached_ideal_lod: usize,

    /// Debug mode
    debug_mode: bool,
}

impl TiledData {
    pub fn new(
        _device: &Device,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        assert!(!lod_levels.is_empty(), "Must have at least one LOD level");

        Self {
            levels: lod_levels,
            tiles: Vec::new(),
            tile_cache: HashMap::new(),
            pending_chunks: Arc::new(Mutex::new(HashMap::new())),
            requested_keys: HashSet::new(),
            loader,
            max_tiles,
            frame_counter: 0,
            visible_tile_keys: HashSet::new(),
            lod_bias: 0.0,  // Neutral - match TiledImageVisual default
            target_pixels_per_voxel: 1.0,
            cached_ideal_lod: 0,
            debug_mode: false,
        }
    }

    /// Compute AABB for a tile at given LOD and grid position
    fn compute_tile_aabb(&self, lod_level: usize, tile_pos: (u32, u32, u32)) -> AABB {
        let level = &self.levels[lod_level];
        let (tz, ty, tx) = tile_pos;
        let (tile_size_z, tile_size_y, tile_size_x) = level.tile_size;
        let (voxel_z, voxel_y, voxel_x) = level.voxel_size;
        let (trans_z, trans_y, trans_x) = level.translation;

        // Calculate world-space position (data space -> world space via translation)
        let min = Vec3::new(
            tx as f32 * tile_size_x as f32 * voxel_x + trans_x,
            ty as f32 * tile_size_y as f32 * voxel_y + trans_y,
            tz as f32 * tile_size_z as f32 * voxel_z + trans_z,
        );

        // Calculate actual size of this tile (may be smaller at edges)
        let (vol_z, vol_y, vol_x) = level.volume_size;
        let actual_size_x = tile_size_x.min(vol_x - tx * tile_size_x);
        let actual_size_y = tile_size_y.min(vol_y - ty * tile_size_y);
        let actual_size_z = tile_size_z.min(vol_z - tz * tile_size_z);

        let max = min + Vec3::new(
            actual_size_x as f32 * voxel_x,
            actual_size_y as f32 * voxel_y,
            actual_size_z as f32 * voxel_z,
        );

        AABB::new(min, max)
    }

    /// Check if plane intersects AABB
    fn plane_aabb_intersection(plane: &SlicePlane, aabb: &AABB) -> bool {
        let plane_pos = Vec3::from(plane.position);
        let plane_normal = Vec3::from(plane.normal);

        // Check all 8 corners of AABB
        let corners = [
            aabb.min,
            Vec3::new(aabb.max.x, aabb.min.y, aabb.min.z),
            Vec3::new(aabb.min.x, aabb.max.y, aabb.min.z),
            Vec3::new(aabb.max.x, aabb.max.y, aabb.min.z),
            Vec3::new(aabb.min.x, aabb.min.y, aabb.max.z),
            Vec3::new(aabb.max.x, aabb.min.y, aabb.max.z),
            Vec3::new(aabb.min.x, aabb.max.y, aabb.max.z),
            aabb.max,
        ];

        // Check if corners are on both sides of plane
        let mut min_dist = f32::MAX;
        let mut max_dist = f32::MIN;

        for corner in &corners {
            let dist = plane_normal.dot(*corner - plane_pos);
            min_dist = min_dist.min(dist);
            max_dist = max_dist.max(dist);
        }

        // AABB intersects plane if min and max have opposite signs
        // Use a small epsilon relative to AABB size for numerical tolerance
        let aabb_size = (aabb.max - aabb.min).length();
        let epsilon = aabb_size * 0.001;  // 0.1% of AABB diagonal
        min_dist <= epsilon && max_dist >= -epsilon
    }

    /// Select LOD for a tile based on camera distance and screen-space size
    fn select_lod_for_tile(
        &self,
        tile_center: Vec3,
        tile_world_size: f32,
        camera_info: &crate::visual::CameraInfo,
    ) -> usize {
        let distance = (tile_center - camera_info.position).length();
        // Use adaptive threshold: very close relative to tile size
        let min_distance = tile_world_size * 0.1;
        if distance < min_distance {
            return 0; // Very close, use highest resolution
        }

        // Screen-space size calculation
        // focal_length = viewport_height / (2 * tan(fov_y / 2))
        let focal_length = camera_info.viewport_height as f32 / (2.0 * (camera_info.fov_y / 2.0).tan());
        let screen_pixels = (tile_world_size / distance) * focal_length;

        // Pixels per voxel at LOD 0
        let lod0 = &self.levels[0];
        let voxels_per_tile = lod0.tile_size.2 as f32; // Use X dimension
        let pixels_per_voxel = screen_pixels / voxels_per_tile;

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

    /// Update visible tiles based on slice plane and camera
    fn update_visible_tiles(&mut self, slice_plane: &SlicePlane, camera_info: &crate::visual::CameraInfo) {
        self.visible_tile_keys.clear();

        // Estimate typical tile size for LOD selection
        let lod0 = &self.levels[0];
        let typical_tile_size = lod0.tile_size.2 as f32 * lod0.voxel_size.2;

        // Compute distance from camera to slice plane for LOD estimation
        let plane_normal = Vec3::from_array(slice_plane.normal);
        let plane_point = Vec3::from_array(slice_plane.position);
        // Use adaptive minimum distance based on volume size instead of hardcoded 1.0
        let min_distance = typical_tile_size * 0.5;
        let distance_to_plane = (camera_info.position - plane_point).dot(plane_normal).abs().max(min_distance);

        // Select LOD for center of view
        let center_point = camera_info.position + plane_normal * distance_to_plane;
        let ideal_lod = self.select_lod_for_tile(center_point, typical_tile_size, camera_info);

        // Cache ideal LOD for render ordering
        self.cached_ideal_lod = ideal_lod;

        // STRATEGY: First pass - mark all already-loaded tiles as visible if they pass
        // frustum/slice tests. Render order (via stencil) will handle LOD priority.
        // This ensures smooth transitions - no tiles disappear when loading new LODs.
        for tile_key in self.tile_cache.keys() {
            let aabb = self.compute_tile_aabb(tile_key.lod_level, (tile_key.z, tile_key.y, tile_key.x));

            // Frustum culling with 5% margin to be conservative
            let margin = (aabb.max - aabb.min) * 0.05;
            let expanded_min = aabb.min - margin;
            let expanded_max = aabb.max + margin;
            if !camera_info.frustum.contains_aabb_xy(expanded_min, expanded_max) {
                continue;
            }

            // Slice plane intersection
            if Self::plane_aabb_intersection(slice_plane, &aabb) {
                self.visible_tile_keys.insert(*tile_key);
            }
        }

        // Second pass: Hierarchical traversal to find ideal-LOD tiles for loading
        // Pick a starting LOD that has a manageable number of tiles (~1000)
        let mut start_lod = ideal_lod;
        for lod in ideal_lod..self.levels.len() {
            let level = &self.levels[lod];
            let grid = level.grid_size();
            let total_tiles = grid.0 as usize * grid.1 as usize * grid.2 as usize;
            if total_tiles <= 1000 {
                start_lod = lod;
                break;
            }
        }

        // If no suitable start found, use coarsest LOD
        if start_lod == ideal_lod {
            let level = &self.levels[ideal_lod];
            let grid = level.grid_size();
            let total = grid.0 as usize * grid.1 as usize * grid.2 as usize;
            if total > 1000 {
                start_lod = self.levels.len() - 1;
            }
        }

        // Hierarchical refinement: test tiles from start_lod down to ideal_lod
        let mut active_regions: Vec<(usize, u32, u32, u32)> = Vec::new(); // (lod, z, y, x)

        // Start with all tiles at coarse LOD
        let start_level = &self.levels[start_lod];
        let start_grid = start_level.grid_size();

        for tz in 0..start_grid.0 {
            for ty in 0..start_grid.1 {
                for tx in 0..start_grid.2 {
                    let aabb = self.compute_tile_aabb(start_lod, (tz, ty, tx));

                    // Frustum culling with 5% margin
                    let margin = (aabb.max - aabb.min) * 0.05;
                    let expanded_min = aabb.min - margin;
                    let expanded_max = aabb.max + margin;
                    if !camera_info.frustum.contains_aabb_xy(expanded_min, expanded_max) {
                        continue;
                    }

                    // Slice plane intersection
                    if Self::plane_aabb_intersection(slice_plane, &aabb) {
                        active_regions.push((start_lod, tz, ty, tx));
                    }
                }
            }
        }

        // Refine active regions down to ideal LOD
        for current_lod in (ideal_lod..start_lod).rev() {
            let mut next_regions = Vec::new();

            for (parent_lod, pz, py, px) in &active_regions {
                if *parent_lod != current_lod + 1 {
                    // Keep this region as-is
                    next_regions.push((*parent_lod, *pz, *py, *px));
                    continue;
                }

                // Refine this region: find child tiles at current_lod
                // Each parent tile maps to 2x2x2 child tiles (or fewer at boundaries)
                let parent_level = &self.levels[*parent_lod];
                let child_level = &self.levels[current_lod];

                let scale_ratio = parent_level.scale_factor / child_level.scale_factor;
                let children_per_parent = (scale_ratio as u32).max(1);

                for dz in 0..children_per_parent {
                    for dy in 0..children_per_parent {
                        for dx in 0..children_per_parent {
                            let cz = pz * children_per_parent + dz;
                            let cy = py * children_per_parent + dy;
                            let cx = px * children_per_parent + dx;

                            let child_grid = child_level.grid_size();
                            if cz >= child_grid.0 || cy >= child_grid.1 || cx >= child_grid.2 {
                                continue;
                            }

                            let aabb = self.compute_tile_aabb(current_lod, (cz, cy, cx));

                            // Test visibility with 5% margin
                            let margin = (aabb.max - aabb.min) * 0.05;
                            let expanded_min = aabb.min - margin;
                            let expanded_max = aabb.max + margin;
                            if !camera_info.frustum.contains_aabb_xy(expanded_min, expanded_max) {
                                continue;
                            }

                            if Self::plane_aabb_intersection(slice_plane, &aabb) {
                                next_regions.push((current_lod, cz, cy, cx));
                            }
                        }
                    }
                }
            }

            active_regions = next_regions;
        }

        // Add all active regions to visible set
        for (lod, tz, ty, tx) in active_regions {
            let key = TileKey::new(lod, tz, ty, tx);
            self.visible_tile_keys.insert(key);
        }
    }

    /// Request a tile to be loaded
    ///
    /// Calls the loader callback which returns ChunkStatus.
    /// If accepted, marks the tile as requested. The actual data will arrive
    /// later via set_chunk_data().
    fn request_tile(&mut self, key: TileKey) -> ChunkStatus {
        let TileKey { lod_level, z, y, x } = key;

        // Check if already requested or pending
        if self.requested_keys.contains(&key) || self.pending_chunks.lock().unwrap().contains_key(&key) {
            return ChunkStatus::AlreadyPending;
        }

        // Create tile request
        let request = TileRequest::from_grid(lod_level, x, y, z);

        // Call loader
        let status = (self.loader)(request);

        // Track if accepted
        if status == ChunkStatus::Accepted {
            self.requested_keys.insert(key);
        }

        status
    }

    /// Create a tile from loaded data
    ///
    /// This is called when processing pending chunks to create GPU textures.
    fn create_tile_from_data(&mut self, device: &Device, queue: &Queue, key: TileKey, data: TileData) {
        let TileKey { lod_level, z, y, x } = key;
        let level = &self.levels[lod_level];

        // Create texture
        let texture_size = wgpu::Extent3d {
            width: data.width,
            height: data.height,
            depth_or_array_layers: data.depth,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Tile Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Upload data
        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &data.data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(data.width),
                rows_per_image: Some(data.height),
            },
            texture_size,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Compute AABB
        let aabb = self.compute_tile_aabb(lod_level, (z, y, x));

        // Create tile
        let mut tile = Tile::new(
            key,
            Arc::new(texture),
            Arc::new(texture_view),
            aabb,
            (data.depth, data.height, data.width),
            level.voxel_size,
            lod_level,
        );

        tile.last_used_frame = self.frame_counter;

        // Add debug color if enabled
        if self.debug_mode {
            let color_index = (z as f32 / level.grid_size().0 as f32 * 6.0) as u32;
            let colors = [
                [1.0, 0.0, 0.0], // Red
                [0.0, 1.0, 0.0], // Green
                [0.0, 0.0, 1.0], // Blue
                [1.0, 1.0, 0.0], // Yellow
                [1.0, 0.0, 1.0], // Magenta
                [0.0, 1.0, 1.0], // Cyan
            ];
            tile.debug_color = Some(colors[(color_index % 6) as usize]);
        }

        // Check if this tile already exists in cache (shouldn't happen, but defensive)
        if let Some(&existing_index) = self.tile_cache.get(&key) {
            // Replace the existing tile in-place rather than creating a duplicate
            self.tiles[existing_index] = tile;
        } else {
            // Add new tile to cache
            let index = self.tiles.len();
            self.tiles.push(tile);
            self.tile_cache.insert(key, index);
        }
    }

    /// Evict tiles using LRU policy
    fn evict_tiles(&mut self) {
        if self.tiles.len() <= self.max_tiles {
            return;
        }

        // Sort by last_used_frame (oldest first)
        let mut tiles_by_access: Vec<_> = self.tiles
            .iter()
            .enumerate()
            .map(|(i, tile)| (i, tile.last_used_frame, tile.key))
            .collect();

        tiles_by_access.sort_by_key(|(_, frame, _)| *frame);

        // Remove oldest tiles, but NEVER evict visible tiles
        let num_to_remove = self.tiles.len() - self.max_tiles;
        let mut to_remove: Vec<_> = tiles_by_access
            .iter()
            .filter(|(_, _, key)| !self.visible_tile_keys.contains(key))  // Skip visible tiles
            .take(num_to_remove)
            .map(|(_, _, key)| *key)
            .collect();

        // Remove from back to front to maintain indices
        // Filter out any keys that aren't actually in the cache (shouldn't happen)
        to_remove.retain(|key| self.tile_cache.contains_key(key));

        to_remove.sort_by_key(|key| {
            self.tile_cache.get(key).copied().expect("tile should be in cache")
        });
        to_remove.reverse();

        for key in &to_remove {
            if let Some(&index) = self.tile_cache.get(key) {
                self.tile_cache.remove(key);
                self.tiles.remove(index);

                // Clean up requested_keys, but NOT pending_chunks
                // pending_chunks contains incoming data that might have just arrived for re-requested tiles
                self.requested_keys.remove(key);

                // Update indices in cache for all tiles after the removed one
                for (_other_key, idx) in self.tile_cache.iter_mut() {
                    if *idx > index {
                        *idx -= 1;
                    }
                }
            }
        }
    }

    /// Update geometry for all loaded tiles based on the current slice plane
    fn update_tile_geometry(&mut self, device: &Device, queue: &Queue, slice_plane: &SlicePlane) {
        use wgpu::util::DeviceExt;

        // Convert image::SlicePlane to tile::SlicePlane
        let tile_plane = crate::visuals::tile::SlicePlane {
            position: slice_plane.position,
            normal: slice_plane.normal,
        };

        for tile in &mut self.tiles {
            // Compute plane-AABB intersection
            if let Some((vertices, indices)) = compute_plane_aabb_intersection(&tile_plane, &tile.bounds) {
                let vertex_count = indices.len() as u32;

                // Try to reuse existing buffers if they exist and are the right size
                let needs_new_vertex_buffer = tile.vertex_buffer.is_none() ||
                    tile.vertex_count != vertex_count;

                if needs_new_vertex_buffer {
                    // Create new vertex buffer
                    let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Tile Vertex Buffer"),
                        contents: bytemuck::cast_slice(&vertices),
                        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    });

                    // Create new index buffer
                    let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                        label: Some("Tile Index Buffer"),
                        contents: bytemuck::cast_slice(&indices),
                        usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
                    });

                    tile.vertex_buffer = Some(vertex_buffer);
                    tile.index_buffer = Some(index_buffer);
                    tile.vertex_count = vertex_count;
                } else if let (Some(ref vertex_buffer), Some(ref index_buffer)) =
                    (&tile.vertex_buffer, &tile.index_buffer) {
                    // Reuse existing buffers - just update the data
                    // Note: WebGPU requires 4-byte alignment for write_buffer
                    let vertex_bytes = bytemuck::cast_slice(&vertices);
                    let index_bytes = bytemuck::cast_slice(&indices);

                    // Pad index data to 4-byte alignment if needed
                    if index_bytes.len() % 4 == 0 {
                        queue.write_buffer(vertex_buffer, 0, vertex_bytes);
                        queue.write_buffer(index_buffer, 0, index_bytes);
                    } else {
                        // Need to pad indices to 4-byte boundary
                        let padded_len = (index_bytes.len() + 3) & !3;  // Round up to multiple of 4
                        let mut padded_indices = vec![0u8; padded_len];
                        padded_indices[..index_bytes.len()].copy_from_slice(index_bytes);

                        queue.write_buffer(vertex_buffer, 0, vertex_bytes);
                        queue.write_buffer(index_buffer, 0, &padded_indices);
                    }
                }
            } else {
                // No intersection - tile doesn't intersect the slice plane
                tile.vertex_count = 0;
                tile.vertex_buffer = None;
                tile.index_buffer = None;
            }
        }
    }
}

impl TiledData {
    fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        slice_plane: &SlicePlane,
        frame_number: u64,
        camera_info: &crate::visual::CameraInfo,
    ) {
        self.frame_counter = frame_number;

        // Process any pending chunks first (create textures for arrived data)
        // Lock once, drain all chunks, then release
        let pending_chunks: Vec<(TileKey, TileData)> = {
            let mut chunks = self.pending_chunks.lock().unwrap();
            chunks.drain().collect()
        };
        for (key, data) in pending_chunks {
            self.create_tile_from_data(device, queue, key, data);
        }

        // Update visible tiles based on slice plane and camera
        self.update_visible_tiles(slice_plane, camera_info);

        // Request visible tiles that aren't loaded yet
        // We rely on the loader to return Rejected when at capacity
        let mut visible: Vec<_> = self.visible_tile_keys.iter().copied().collect();
        visible.sort_by_key(|k| k.lod_level);  // High-res first

        for key in visible {
            if let Some(&index) = self.tile_cache.get(&key) {
                // Tile already loaded - just update access time
                if index < self.tiles.len() && self.tiles[index].key == key {
                    self.tiles[index].last_used_frame = frame_number;
                }
            } else {
                // Tile not loaded - request it
                match self.request_tile(key) {
                    ChunkStatus::Accepted => {
                        // Request accepted, continue with next tile
                    }
                    ChunkStatus::AlreadyPending => {
                        // Already requested, continue
                    }
                    ChunkStatus::Rejected => {
                        // Loader at capacity, stop requesting
                        break;
                    }
                }
            }
        }

        // Evict old tiles if over limit
        self.evict_tiles();

        // Update tile geometry for visible tiles
        self.update_tile_geometry(device, queue, slice_plane);
    }
}

/// Implementation of image strategy methods with dispatch to concrete types
impl ImageStrategy {
    /// Prepare for rendering (update textures, load tiles, manage visibility)
    pub fn prepare(
        &mut self,
        device: &Device,
        queue: &Queue,
        slice_plane: &SlicePlane,
        frame_number: u64,
        camera_info: &crate::visual::CameraInfo,
    ) {
        match self {
            ImageStrategy::Simple(s) => s.prepare(device, queue, slice_plane, frame_number, camera_info),
            ImageStrategy::Tiled(t) => t.prepare(device, queue, slice_plane, frame_number, camera_info),
        }
    }

    /// Get the tiles that should be rendered
    pub fn tiles(&self) -> &[Tile] {
        match self {
            ImageStrategy::Simple(s) => std::slice::from_ref(&s.tile),
            ImageStrategy::Tiled(t) => &t.tiles,
        }
    }

    /// Get the dimensions of the full image (Z, Y, X)
    pub fn dimensions(&self) -> (u32, u32, u32) {
        match self {
            ImageStrategy::Simple(s) => s.dimensions,
            ImageStrategy::Tiled(t) => t.levels[0].volume_size,
        }
    }

    /// Check if data is ready to render
    pub fn is_ready(&self) -> bool {
        match self {
            ImageStrategy::Simple(_) => true,
            ImageStrategy::Tiled(t) => !t.tiles.is_empty(),
        }
    }

    /// Set LOD bias for automatic LOD selection (only for Tiled strategy)
    pub fn set_lod_bias(&mut self, bias: f32) {
        if let ImageStrategy::Tiled(t) = self {
            t.lod_bias = bias;
        }
        // No-op for Simple strategy
    }

    /// Set debug mode
    pub fn set_debug_mode(&mut self, enabled: bool) {
        if let ImageStrategy::Tiled(t) = self {
            t.debug_mode = enabled;
        }
        // No-op for Simple strategy
    }

    /// Get statistics (loaded tiles, visible tiles)
    pub fn get_stats(&self) -> (usize, usize) {
        match self {
            ImageStrategy::Simple(_) => (0, 0),
            ImageStrategy::Tiled(t) => (t.tiles.len(), t.visible_tile_keys.len()),
        }
    }

    /// Provide chunk data for a requested tile (only for Tiled strategy)
    ///
    /// This should be called when chunk data is ready for a tile that was requested.
    pub fn set_chunk_data(&mut self, lod_level: usize, z: u32, y: u32, x: u32, data: crate::visuals::tile::TileData) {
        if let ImageStrategy::Tiled(t) = self {
            let key = TileKey { lod_level, z, y, x };
            t.requested_keys.remove(&key);
            t.pending_chunks.lock().unwrap().insert(key, data);
        }
        // No-op for Simple strategy
    }

    /// Get the ideal/target LOD level for the current view
    pub fn get_ideal_lod(&self) -> usize {
        match self {
            ImageStrategy::Simple(_) => 0,
            ImageStrategy::Tiled(t) => t.cached_ideal_lod,
        }
    }

    /// Get the pending chunks queue (only for Tiled strategy)
    ///
    /// Returns None for Simple strategy.
    pub fn pending_chunks(&self) -> Option<PendingChunks> {
        match self {
            ImageStrategy::Simple(_) => None,
            ImageStrategy::Tiled(t) => Some(t.pending_chunks.clone()),
        }
    }
}
