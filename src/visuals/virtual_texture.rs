//! Virtual-texture rendering data: atlas, page table, LOD selection, tile streaming.

use crate::visuals::tile::{
    ChunkStatus, TileData, TileKey, TileLoaderFn, TileRequest, AABB,
};
use crate::visuals::image::SlicePlane;
use crate::visuals::atlas::AtlasAllocator;
use crate::visuals::page_table::PageTable;
use glam::Vec3;
use std::cmp::Reverse;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use wgpu::{Device, Queue};

/// Type alias for pending chunks queue (can be shared across threads)
pub type PendingChunks = Arc<Mutex<HashMap<TileKey, TileData>>>;

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

/// Virtual-texture rendering strategy.
///
/// Maintains a physical atlas (one large 3D texture) and a page table (small
/// 2D-array texture) so the entire volume can be rendered with a single draw call.
/// The fragment shader walks the LOD hierarchy in-flight for each fragment.
pub struct VirtualTextureData {
    pub levels: Vec<LodLevelConfig>,

    // Tile size assumed uniform across all LODs (validated in new()).
    pub tile_size: (u32, u32, u32),  // (z, y, x)

    // Physical atlas resources (owned; views are referenced by the ImageVisual bind group)
    pub atlas_allocator: AtlasAllocator,
    pub atlas_texture: wgpu::Texture,
    pub atlas_texture_view: wgpu::TextureView,
    pub atlas_cols: u32,
    pub atlas_rows: u32,
    pub atlas_layers: u32,

    // Page table (owns its own texture + view)
    pub page_table: PageTable,

    // TileKey → atlas slot index
    pub slot_map: HashMap<TileKey, u32>,
    // TileKey → last frame the tile was accessed
    lru_map: HashMap<TileKey, u64>,

    // Loader / pending-chunk infrastructure
    pub pending_chunks: PendingChunks,
    requested_keys: HashSet<TileKey>,
    loader: TileLoaderFn,

    pub max_tiles: usize,
    frame_counter: u64,

    pub visible_tile_keys: HashSet<TileKey>,
    pub lod_bias: f32,
    target_pixels_per_voxel: f32,
    pub cached_ideal_lod: usize,
}

impl VirtualTextureData {
    pub fn new(
        device: &Device,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
        loader: TileLoaderFn,
    ) -> Self {
        assert!(!lod_levels.is_empty(), "VirtualTextureData requires at least one LOD level");

        let tile_size = lod_levels[0].tile_size; // (z, y, x)

        let (tile_d, tile_h, tile_w) = tile_size;

        // Clamp atlas grid to the device's 3D texture dimension limits.
        // The atlas uses all three dimensions for slot stacking:
        //   atlas width  = tile_w * cols
        //   atlas height = tile_h * rows
        //   atlas depth  = tile_d * layers
        let max_dim = device.limits().max_texture_dimension_3d;
        let max_cols   = (max_dim / tile_w).max(1);
        let max_rows   = (max_dim / tile_h).max(1);
        let max_layers = (max_dim / tile_d).max(1);
        let effective_capacity =
            max_tiles.min((max_cols * max_rows * max_layers) as usize);

        // Distribute slots as evenly as possible across the three axes.
        let cbrt_cap = (effective_capacity as f64).cbrt().ceil() as u32;
        let cols   = cbrt_cap.min(max_cols).max(1);
        let rows   = cbrt_cap.min(max_rows).max(1);
        let layers = ((effective_capacity as u32 + cols * rows - 1) / (cols * rows))
            .min(max_layers)
            .max(1);

        // cols*rows*layers may exceed effective_capacity due to grid rounding.
        let atlas_capacity = effective_capacity.min((cols * rows * layers) as usize);

        let atlas_texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("VT Atlas"),
            size: wgpu::Extent3d {
                width:                tile_w * cols,
                height:               tile_h * rows,
                depth_or_array_layers: tile_d * layers,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let atlas_texture_view = atlas_texture.create_view(&wgpu::TextureViewDescriptor::default());
        let atlas_allocator = AtlasAllocator::new(atlas_capacity);
        let page_table = PageTable::new(device, &lod_levels);

        Self {
            tile_size,
            atlas_allocator,
            atlas_texture,
            atlas_texture_view,
            atlas_cols: cols,
            atlas_rows: rows,
            atlas_layers: layers,
            page_table,
            slot_map: HashMap::new(),
            lru_map: HashMap::new(),
            pending_chunks: Arc::new(Mutex::new(HashMap::new())),
            requested_keys: HashSet::new(),
            loader,
            max_tiles,
            frame_counter: 0,
            visible_tile_keys: HashSet::new(),
            lod_bias: 0.0,
            target_pixels_per_voxel: 1.0,
            cached_ideal_lod: 0,
            levels: lod_levels,
        }
    }

    // ── Tile management ──────────────────────────────────────────────────────

    fn write_tile_to_atlas(&self, queue: &Queue, slot: u32, data: &TileData) {
        let col   = slot % self.atlas_cols;
        let row   = (slot / self.atlas_cols) % self.atlas_rows;
        let layer = slot / (self.atlas_cols * self.atlas_rows);
        let (tile_d, tile_h, tile_w) = self.tile_size;

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &self.atlas_texture,
                mip_level: 0,
                origin: wgpu::Origin3d {
                    x: col   * tile_w,
                    y: row   * tile_h,
                    z: layer * tile_d,
                },
                aspect: wgpu::TextureAspect::All,
            },
            &data.data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(data.width * data.bytes_per_voxel()),
                rows_per_image: Some(data.height),
            },
            wgpu::Extent3d {
                width: data.width,
                height: data.height,
                depth_or_array_layers: data.depth,
            },
        );
    }

    fn process_pending_chunks(&mut self, queue: &Queue) {
        let pending: Vec<(TileKey, TileData)> = {
            let mut guard = self.pending_chunks.lock().unwrap();
            guard.drain().collect()
        };

        for (key, data) in pending {
            if self.slot_map.contains_key(&key) {
                continue;
            }

            // Discard tiles that are no longer needed (camera moved past that LOD).
            // Remove from requested_keys so the tile can be re-requested if it becomes
            // visible again.
            if !self.visible_tile_keys.contains(&key) {
                self.requested_keys.remove(&key);
                continue;
            }

            let Some(slot) = self.atlas_allocator.alloc() else {
                // Eviction ran before this, but visible tiles may have consumed all slots.
                // Release the key so it can be re-requested once space frees up.
                self.requested_keys.remove(&key);
                continue;
            };

            self.write_tile_to_atlas(queue, slot, &data);

            let TileKey { lod_level, z, y, x } = key;
            let (gx, gy, _gz) = {
                let (gz2, gy2, gx2) = self.levels[lod_level].grid_size();
                (gx2, gy2, gz2)
            };
            let linear = z * gy * gx + y * gx + x;
            eprintln!("VT load: lod={lod_level} ({x},{y},{z}) linear={linear} slot={slot} \
                col={} row={} layer={}", slot % self.atlas_cols,
                (slot / self.atlas_cols) % self.atlas_rows,
                slot / (self.atlas_cols * self.atlas_rows));
            self.page_table.update(queue, lod_level, z, y, x, slot);

            self.slot_map.insert(key, slot);
            self.lru_map.insert(key, self.frame_counter);
        }
    }

    fn evict_tiles(&mut self, queue: &Queue) {
        if self.slot_map.len() < self.max_tiles {
            return;
        }

        // Evict enough to bring us back below max_tiles and open one free slot
        // for the next arriving chunk.
        let num_to_remove = (self.slot_map.len() + 1).saturating_sub(self.max_tiles);
        let ideal_lod = self.cached_ideal_lod;

        // Phase 1: tiles not currently visible — pure LRU.
        let mut candidates: Vec<TileKey> = self.slot_map.keys()
            .filter(|k| !self.visible_tile_keys.contains(k))
            .copied()
            .collect();
        candidates.sort_by_key(|k| *self.lru_map.get(k).unwrap_or(&0));

        // Phase 2: if still short, add visible tiles at the wrong LOD.
        // Sort by distance from ideal_lod descending so the most off-target tiles go first.
        if candidates.len() < num_to_remove {
            let mut wrong_lod: Vec<TileKey> = self.slot_map.keys()
                .filter(|k| self.visible_tile_keys.contains(k) && k.lod_level != ideal_lod)
                .copied()
                .collect();
            wrong_lod.sort_by(|a, b| {
                let da = a.lod_level.abs_diff(ideal_lod);
                let db = b.lod_level.abs_diff(ideal_lod);
                // Farthest-from-ideal first; break ties by LRU (oldest first).
                db.cmp(&da).then_with(|| {
                    self.lru_map.get(a).unwrap_or(&0)
                        .cmp(self.lru_map.get(b).unwrap_or(&0))
                })
            });
            candidates.extend(wrong_lod);
        }

        for key in candidates.into_iter().take(num_to_remove) {
            if let Some(slot) = self.slot_map.remove(&key) {
                self.atlas_allocator.free(slot);
                self.lru_map.remove(&key);
                self.requested_keys.remove(&key);
                let TileKey { lod_level, z, y, x } = key;
                self.page_table.clear(queue, lod_level, z, y, x);
            }
        }
    }

    fn request_tile(&mut self, key: TileKey) -> ChunkStatus {
        if self.requested_keys.contains(&key)
            || self.pending_chunks.lock().unwrap().contains_key(&key)
        {
            return ChunkStatus::AlreadyPending;
        }

        let TileKey { lod_level, z, y, x } = key;
        let request = TileRequest::from_grid(lod_level, x, y, z);
        let status = (self.loader)(request);

        if status == ChunkStatus::Accepted {
            self.requested_keys.insert(key);
        }
        status
    }

    // ── Visibility / LOD selection ────────────────────────────────────────────

    fn select_lod_for_tile(
        &self,
        tile_center: Vec3,
        tile_world_size: f32,
        camera_info: &crate::visual::CameraInfo,
    ) -> usize {
        let screen_pixels = match camera_info.projection_mode {
            crate::ProjectionMode::Perspective => {
                let distance = (tile_center - camera_info.position).length();
                let min_distance = tile_world_size * 0.1;
                if distance < min_distance {
                    return 0;
                }
                let focal_length = camera_info.viewport_height as f32
                    / (2.0 * (camera_info.fov_y / 2.0).tan());
                (tile_world_size / distance) * focal_length
            }
            crate::ProjectionMode::Orthographic => {
                (tile_world_size / camera_info.ortho_height) * camera_info.viewport_height as f32
            }
        };

        let lod0 = &self.levels[0];
        let voxels_per_tile = lod0.tile_size.2 as f32;
        let pixels_per_voxel = screen_pixels / voxels_per_tile;
        let target = self.target_pixels_per_voxel * 2.0_f32.powf(-self.lod_bias);

        for (lod, level) in self.levels.iter().enumerate() {
            if pixels_per_voxel * level.scale_factor >= target {
                return lod;
            }
        }
        self.levels.len() - 1
    }

    fn compute_tile_aabb(&self, lod_level: usize, tile_pos: (u32, u32, u32)) -> AABB {
        let level = &self.levels[lod_level];
        let (tz, ty, tx) = tile_pos;
        let (tile_size_z, tile_size_y, tile_size_x) = level.tile_size;
        let (voxel_z, voxel_y, voxel_x) = level.voxel_size;
        let (trans_z, trans_y, trans_x) = level.translation;

        let min = Vec3::new(
            tx as f32 * tile_size_x as f32 * voxel_x + trans_x,
            ty as f32 * tile_size_y as f32 * voxel_y + trans_y,
            tz as f32 * tile_size_z as f32 * voxel_z + trans_z,
        );

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

    fn plane_aabb_intersection(plane: &SlicePlane, aabb: &AABB) -> bool {
        let plane_pos = Vec3::from(plane.position);
        let plane_normal = Vec3::from(plane.normal);

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

        let mut min_dist = f32::MAX;
        let mut max_dist = f32::MIN;
        for corner in &corners {
            let dist = plane_normal.dot(*corner - plane_pos);
            min_dist = min_dist.min(dist);
            max_dist = max_dist.max(dist);
        }

        let aabb_size = (aabb.max - aabb.min).length();
        let epsilon = aabb_size * 0.001;
        min_dist <= epsilon && max_dist >= -epsilon
    }

    fn update_visible_tiles(
        &mut self,
        slice_plane: &SlicePlane,
        camera_info: &crate::visual::CameraInfo,
    ) {
        self.visible_tile_keys.clear();

        let lod0 = &self.levels[0];
        let typical_tile_size = lod0.tile_size.2 as f32 * lod0.voxel_size.2;

        let plane_normal = Vec3::from_array(slice_plane.normal);
        let plane_point = Vec3::from_array(slice_plane.position);
        // Signed distance: positive = camera on the normal-facing side.
        let signed_dist = (camera_info.position - plane_point).dot(plane_normal);

        // Project the camera position onto the slice plane.
        let center_point = camera_info.position - plane_normal * signed_dist;
        let ideal_lod = self.select_lod_for_tile(center_point, typical_tile_size, camera_info);
        self.cached_ideal_lod = ideal_lod;

        // Hierarchical traversal from a manageable coarse LOD down to ideal_lod.
        let mut start_lod = ideal_lod;
        for lod in ideal_lod..self.levels.len() {
            let level = &self.levels[lod];
            let grid = level.grid_size();
            if (grid.0 as usize) * (grid.1 as usize) * (grid.2 as usize) <= 1000 {
                start_lod = lod;
                break;
            }
        }

        let start_level = &self.levels[start_lod];
        let start_grid = start_level.grid_size();
        let mut active_regions: Vec<(usize, u32, u32, u32)> = Vec::new();

        for tz in 0..start_grid.0 {
            for ty in 0..start_grid.1 {
                for tx in 0..start_grid.2 {
                    let aabb = self.compute_tile_aabb(start_lod, (tz, ty, tx));
                    let margin = (aabb.max - aabb.min) * 0.05;
                    if !camera_info.frustum.contains_aabb_xy(aabb.min - margin, aabb.max + margin) {
                        continue;
                    }
                    if Self::plane_aabb_intersection(slice_plane, &aabb) {
                        active_regions.push((start_lod, tz, ty, tx));
                    }
                }
            }
        }

        // Add start_lod tiles as fallback coverage.  Only start_lod and ideal_lod
        // go into visible_tile_keys — adding every intermediate LOD was inflating the
        // visible count far above max_tiles and causing constant eviction/reload churn.
        for &(lod, tz, ty, tx) in &active_regions {
            self.visible_tile_keys.insert(TileKey::new(lod, tz, ty, tx));
        }

        for current_lod in (ideal_lod..start_lod).rev() {
            let mut next_regions = Vec::new();
            for (parent_lod, pz, py, px) in &active_regions {
                if *parent_lod != current_lod + 1 {
                    next_regions.push((*parent_lod, *pz, *py, *px));
                    continue;
                }
                let parent_grid_dims = self.levels[*parent_lod].grid_size();
                let child_grid = self.levels[current_lod].grid_size();
                // Children-per-parent per axis: derived from grid sizes so floating-point
                // scale_factor imprecision can't cause tiles to be silently skipped.
                let cpp_z = ((child_grid.0 + parent_grid_dims.0 - 1) / parent_grid_dims.0).max(1);
                let cpp_y = ((child_grid.1 + parent_grid_dims.1 - 1) / parent_grid_dims.1).max(1);
                let cpp_x = ((child_grid.2 + parent_grid_dims.2 - 1) / parent_grid_dims.2).max(1);

                for dz in 0..cpp_z {
                    for dy in 0..cpp_y {
                        for dx in 0..cpp_x {
                            let (cz, cy, cx) = (pz * cpp_z + dz, py * cpp_y + dy, px * cpp_x + dx);
                            if cz >= child_grid.0 || cy >= child_grid.1 || cx >= child_grid.2 {
                                continue;
                            }
                            let aabb = self.compute_tile_aabb(current_lod, (cz, cy, cx));
                            let margin = (aabb.max - aabb.min) * 0.05;
                            if !camera_info.frustum.contains_aabb_xy(aabb.min - margin, aabb.max + margin) {
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

        // Add the ideal_lod tiles — these are the ones we actually want to display.
        for &(lod, tz, ty, tx) in &active_regions {
            self.visible_tile_keys.insert(TileKey::new(lod, tz, ty, tx));
        }
    }

    // ── Main prepare ─────────────────────────────────────────────────────────

    pub(crate) fn prepare(
        &mut self,
        queue: &Queue,
        slice_plane: &SlicePlane,
        frame_number: u64,
        camera_info: &crate::visual::CameraInfo,
    ) {
        self.frame_counter = frame_number;

        self.update_visible_tiles(slice_plane, camera_info);

        // Cancel in-flight requests for tiles that are no longer visible.
        self.requested_keys.retain(|k| self.visible_tile_keys.contains(k) || self.slot_map.contains_key(k));

        // Touch LRU timestamps for visible loaded tiles.
        for key in &self.visible_tile_keys {
            self.lru_map.insert(*key, frame_number);
        }

        // Evict before placing arriving chunks so there's always a free slot available.
        self.evict_tiles(queue);

        // Place any chunks that finished loading.
        self.process_pending_chunks(queue);

        // Request any visible tiles that aren't loaded.
        // Sort coarsest-first so fallback LODs are resident while fine tiles stream in.
        let mut visible: Vec<TileKey> = self.visible_tile_keys.iter().copied().collect();
        visible.sort_by_key(|k| Reverse(k.lod_level));
        for key in visible {
            if self.slot_map.contains_key(&key) {
                continue;
            }
            match self.request_tile(key) {
                ChunkStatus::Accepted | ChunkStatus::AlreadyPending => {}
                ChunkStatus::Rejected => break,
            }
        }
    }
}
