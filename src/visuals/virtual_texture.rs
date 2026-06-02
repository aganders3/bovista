//! Virtual-texture rendering data: atlas, page table, LOD selection, tile streaming.

use crate::visuals::gpu_structs::{
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
    /// Effective resolution ratio relative to LOD 0 (1.0, 2.0, 4.0, ...).
    /// For isotropic downsampling this equals the per-axis ratio. For anisotropic
    /// downsampling (e.g. 2× in XY but not Z) use max(sz_i/sz_0, sy_i/sy_0, sx_i/sx_0).
    /// TODO: replace with per-axis (f32, f32, f32) and make `select_lod_for_tile`
    /// weight the axes by the projected viewing direction so LOD selection is
    /// accurate regardless of orientation.
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

    /// Visible tile spatial keys. All entries have `t = 0` — this set is
    /// "which (lod, z, y, x) tiles are visible," independent of timepoint.
    /// Pending-drain and LRU touching compare against the SPATIAL part of
    /// stored slot_map keys.
    pub visible_tile_keys: HashSet<TileKey>,
    pub lod_bias: f32,
    target_pixels_per_voxel: f32,
    pub cached_ideal_lod: usize,
    /// Timepoint currently reflected in the page table (what the shader is
    /// actually displaying).
    presentation_t: u32,
    /// Timepoint the caller wants to display. The page table flips to
    /// `desired_t` when (a) all visible spatial tiles have a slot at that t
    /// (the "clean swap" case), OR (b) `MAX_SWAP_WAIT_FRAMES` have elapsed
    /// since the swap was *first* scheduled (the "partial swap" timeout).
    /// Subsequent `desired_t` changes during the wait do NOT push the
    /// deadline further out — that's what causes the "stuck on an old
    /// frame" symptom under rapid scrubbing. They just retarget where the
    /// swap will land when the deadline fires.
    desired_t: u32,
    /// Frame counter when the current swap-pending was first scheduled.
    /// u64::MAX when no swap is pending (desired_t == presentation_t).
    swap_due_frame: u64,
}

/// Maximum frames to wait for a clean swap before forcing a partial swap.
/// At 30 fps, 6 frames = ~200 ms.
const MAX_SWAP_WAIT_FRAMES: u64 = 6;

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
            presentation_t: 0,
            desired_t: 0,
            swap_due_frame: u64::MAX,
        }
    }

    /// Caller-facing knob: request that the displayed timepoint be `t`. The
    /// switch happens when every visible spatial tile has a slot at the new
    /// t (clean swap, no flicker) OR after a short timeout (partial swap;
    /// shader falls back to coarser LODs for missing tiles). Non-temporal
    /// callers can ignore this.
    pub fn set_desired_timepoint(&mut self, t: u32) {
        if t == self.desired_t { return; }
        self.desired_t = t;
        if t == self.presentation_t {
            // The caller asked for what's already showing — cancel any
            // pending swap.
            self.swap_due_frame = u64::MAX;
        } else if self.swap_due_frame == u64::MAX {
            // First divergence from presentation_t in this swap cycle —
            // schedule the partial-swap deadline. Subsequent retargets
            // before the deadline do NOT push it back; the user will see
            // SOMETHING within MAX_SWAP_WAIT_FRAMES of their first change.
            self.swap_due_frame = self.frame_counter + MAX_SWAP_WAIT_FRAMES;
        }
    }

    pub fn presentation_t(&self) -> u32 { self.presentation_t }
    pub fn desired_t(&self) -> u32 { self.desired_t }

    // ── Tile management ──────────────────────────────────────────────────────

    /// Drop all resident tiles. Now that `TileKey` includes `t`, the
    /// preferred way to switch timepoints is `set_desired_timepoint(t)` —
    /// adjacent t's stay resident in the atlas as cache and the page table
    /// flips atomically once the new t is fully loaded (no flicker).
    /// `clear_atlas` is retained for unusual cases where the underlying
    /// data shape itself has changed.
    pub fn clear_atlas(&mut self, queue: &Queue) {
        for (key, _slot) in self.slot_map.drain() {
            // Only the currently-displayed timepoint has page-table entries.
            if key.t == self.presentation_t {
                self.page_table.clear(queue, key.lod_level, key.z, key.y, key.x);
            }
        }
        self.lru_map.clear();
        self.requested_keys.clear();
        self.pending_chunks.lock().unwrap().clear();
        self.atlas_allocator.reset();
    }

    fn write_tile_to_atlas(&self, queue: &Queue, slot: u32, data: &TileData) {
        let col   = slot % self.atlas_cols;
        let row   = (slot / self.atlas_cols) % self.atlas_rows;
        let layer = slot / (self.atlas_cols * self.atlas_rows);
        let (tile_d, tile_h, tile_w) = self.tile_size;

        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
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
            wgpu::TexelCopyBufferLayout {
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
        // Make room for the whole batch up front. Phase 2 of eviction will
        // surrender prefetched-other-t slots specifically — so the current
        // frame's tiles can always land regardless of how full the atlas
        // got with neighbor-timepoint prefetch material.
        if !pending.is_empty() {
            self.evict_tiles(queue, pending.len());
        }

        for (key, data) in pending {
            if self.slot_map.contains_key(&key) {
                continue;
            }

            // Visibility is spatial — compare the (lod, z, y, x) part only.
            // Tiles for adjacent timepoints (prefetched ahead) hit this
            // path because the camera hasn't moved, so the spatial key IS
            // visible even though the tile's `t` isn't current.
            if !self.visible_tile_keys.contains(&key.spatial()) {
                self.requested_keys.remove(&key);
                continue;
            }

            let Some(slot) = self.atlas_allocator.alloc() else {
                self.requested_keys.remove(&key);
                continue;
            };

            self.write_tile_to_atlas(queue, slot, &data);

            let TileKey { lod_level, t, z, y, x } = key;
            if log::log_enabled!(log::Level::Trace) {
                let (gx, gy, _gz) = {
                    let (gz2, gy2, gx2) = self.levels[lod_level].grid_size();
                    (gx2, gy2, gz2)
                };
                let linear = z * gy * gx + y * gx + x;
                log::trace!(
                    "VT load: lod={lod_level} t={t} ({x},{y},{z}) linear={linear} slot={slot} \
                     col={} row={} layer={}",
                    slot % self.atlas_cols,
                    (slot / self.atlas_cols) % self.atlas_rows,
                    slot / (self.atlas_cols * self.atlas_rows),
                );
            }
            // Page table only points at the currently-displayed timepoint.
            // Prefetched tiles for other t values sit in the atlas
            // unreferenced until set_desired_timepoint() flips presentation
            // to their t (via maybe_advance_presentation below).
            if t == self.presentation_t {
                self.page_table.update(queue, lod_level, z, y, x, slot);
            }

            self.slot_map.insert(key, slot);
            self.lru_map.insert(key, self.frame_counter);
        }
    }

    /// Flip the page table to `desired_t` if either (a) every visible
    /// spatial tile has a slot at `desired_t` (clean) or (b) we've waited
    /// past the timeout (partial; shader falls back to coarser LODs for
    /// missing tiles). Called at the end of `prepare()` so the swap
    /// happens within the same frame that the last needed tile arrived,
    /// or within ~200 ms of `set_desired_timepoint` if the load is slow.
    fn maybe_advance_presentation(&mut self, queue: &Queue) {
        if self.desired_t == self.presentation_t {
            self.swap_due_frame = u64::MAX;
            return;
        }
        let target = self.desired_t;
        let timed_out = self.frame_counter >= self.swap_due_frame;
        // Try for a clean swap first.
        let all_present = self.visible_tile_keys.iter().all(|spatial| {
            let key = TileKey { lod_level: spatial.lod_level, t: target,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            self.slot_map.contains_key(&key)
        });
        if !all_present && !timed_out { return; }

        // Flip: rewrite every page-table entry. Slots that don't exist at
        // `target` get cleared; the shader's LOD fallback will sample
        // coarser-LOD tiles instead, which usually ARE resident at the
        // same camera position. The page table will be refined as the
        // remaining fine tiles arrive (process_pending_chunks writes the
        // page table for tiles whose t == presentation_t).
        let visible: Vec<TileKey> = self.visible_tile_keys.iter().copied().collect();
        let mut loaded = 0usize;
        for spatial in &visible {
            let key = TileKey { lod_level: spatial.lod_level, t: target,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            if let Some(&slot) = self.slot_map.get(&key) {
                self.page_table.update(queue, spatial.lod_level, spatial.z, spatial.y, spatial.x, slot);
                loaded += 1;
            } else {
                self.page_table.clear(queue, spatial.lod_level, spatial.z, spatial.y, spatial.x);
            }
        }
        log::debug!(
            "VT presentation_t: {} → {} ({}/{} tiles loaded{})",
            self.presentation_t, target, loaded, visible.len(),
            if all_present { ", clean swap" } else { ", partial swap (timeout)" },
        );
        self.presentation_t = target;
        self.swap_due_frame = u64::MAX;
    }

    /// Evict enough slots to make room for `needed_free` new tiles.
    /// Prioritizes (most-evictable first):
    ///   1. Spatial key not in current visible set  — camera moved past it
    ///   2. Spatial visible but t is neither `presentation_t` nor `desired_t`
    ///      — prefetch material for a timepoint the user isn't on. These are
    ///      the right slots to surrender so the current frame can load.
    ///   3. Spatial visible, t ∈ {presentation, desired}, but wrong LOD.
    /// Within each phase, oldest LRU first.
    fn evict_tiles(&mut self, queue: &Queue, needed_free: usize) {
        let free_now = self.max_tiles.saturating_sub(self.slot_map.len());
        if free_now >= needed_free { return; }
        let num_to_remove = needed_free - free_now;
        let ideal_lod = self.cached_ideal_lod;
        let cur_ts = (self.presentation_t, self.desired_t);

        // Phase 1: spatial not visible.
        let mut candidates: Vec<TileKey> = self.slot_map.keys()
            .filter(|k| !self.visible_tile_keys.contains(&k.spatial()))
            .copied()
            .collect();
        candidates.sort_by_key(|k| *self.lru_map.get(k).unwrap_or(&0));

        // Phase 2: spatial visible, but t ∉ {presentation, desired}.
        // Critical for keeping the current frame snappy when the atlas
        // is full of prefetched neighbors.
        if candidates.len() < num_to_remove {
            let mut other_t: Vec<TileKey> = self.slot_map.keys()
                .filter(|k| {
                    self.visible_tile_keys.contains(&k.spatial())
                        && k.t != cur_ts.0 && k.t != cur_ts.1
                })
                .copied()
                .collect();
            other_t.sort_by(|a, b| {
                // Prefer evicting tiles farther from the current/desired t.
                let da = (a.t as i64 - cur_ts.0 as i64).abs()
                    .min((a.t as i64 - cur_ts.1 as i64).abs());
                let db = (b.t as i64 - cur_ts.0 as i64).abs()
                    .min((b.t as i64 - cur_ts.1 as i64).abs());
                db.cmp(&da).then_with(|| {
                    self.lru_map.get(a).unwrap_or(&0)
                        .cmp(self.lru_map.get(b).unwrap_or(&0))
                })
            });
            candidates.extend(other_t);
        }

        // Phase 3: visible at current/desired t but wrong LOD.
        if candidates.len() < num_to_remove {
            let mut wrong_lod: Vec<TileKey> = self.slot_map.keys()
                .filter(|k| self.visible_tile_keys.contains(&k.spatial())
                            && (k.t == cur_ts.0 || k.t == cur_ts.1)
                            && k.lod_level != ideal_lod)
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
                // Only clear the page table if the evicted tile was the one
                // currently being displayed. Tiles at other t's don't have a
                // page-table entry (they live in slot_map as prefetch).
                if key.t == self.presentation_t {
                    let TileKey { lod_level, z, y, x, .. } = key;
                    self.page_table.clear(queue, lod_level, z, y, x);
                }
            }
        }
    }

    fn request_tile(&mut self, key: TileKey) -> ChunkStatus {
        if self.requested_keys.contains(&key)
            || self.pending_chunks.lock().unwrap().contains_key(&key)
        {
            return ChunkStatus::AlreadyPending;
        }

        let TileKey { lod_level, t, z, y, x } = key;
        let request = TileRequest::from_grid_t(lod_level, t, x, y, z);
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

    fn update_visible_tiles_volume(
        &mut self,
        camera_info: &crate::visual::CameraInfo,
    ) {
        self.visible_tile_keys.clear();
        if self.levels.is_empty() { return; }

        // Seed the stack from the coarsest (single or few tile) LOD.
        let coarsest = self.levels.len() - 1;
        let coarse_grid = self.levels[coarsest].grid_size();
        let mut stack: Vec<(usize, u32, u32, u32)> = Vec::with_capacity(
            (coarse_grid.0 * coarse_grid.1 * coarse_grid.2) as usize,
        );
        for tz in 0..coarse_grid.0 {
            for ty in 0..coarse_grid.1 {
                for tx in 0..coarse_grid.2 {
                    stack.push((coarsest, tz, ty, tx));
                }
            }
        }

        // cached_ideal_lod from the volume centre — used as the shader's target_lod starting
        // point so the page-table walk is efficient on average.
        {
            let lod0 = &self.levels[0];
            let (vz, vy, vx) = lod0.volume_size;
            let (sz, sy, sx) = lod0.voxel_size;
            let (tz0, ty0, tx0) = lod0.translation;
            let vol_center = Vec3::new(
                tx0 + vx as f32 * sx * 0.5,
                ty0 + vy as f32 * sy * 0.5,
                tz0 + vz as f32 * sz * 0.5,
            );
            let typical = lod0.tile_size.2 as f32 * lod0.voxel_size.2;
            self.cached_ideal_lod = self.select_lod_for_tile(vol_center, typical, camera_info);
        }

        // Per-tile LOD selection: refine a tile only when its ideal LOD is finer than its
        // current LOD. Tiles close to the camera get refined; distant tiles stay coarse.
        while let Some((lod, tz, ty, tx)) = stack.pop() {
            let aabb = self.compute_tile_aabb(lod, (tz, ty, tx));

            // Frustum cull — skip tiles the camera cannot see.
            let margin = (aabb.max - aabb.min) * 0.05;
            if !camera_info.frustum.contains_aabb_xy(aabb.min - margin, aabb.max + margin) {
                continue;
            }

            if lod == 0 {
                // Already at finest LOD — just mark it.
                self.visible_tile_keys.insert(TileKey::new(lod, tz, ty, tx));
                continue;
            }

            // How fine does the camera need this tile? Use the tile's own centre + X-extent.
            let tile_center = (aabb.min + aabb.max) * 0.5;
            let tile_size   = aabb.max.x - aabb.min.x;
            let ideal_for_tile = self.select_lod_for_tile(tile_center, tile_size, camera_info);

            if lod <= ideal_for_tile {
                // Current LOD is fine enough (camera is far away enough) — use this tile.
                self.visible_tile_keys.insert(TileKey::new(lod, tz, ty, tx));
            } else {
                // Need finer data — push children of the next finer LOD.
                let child_lod    = lod - 1;
                let parent_grid  = self.levels[lod].grid_size();
                let child_grid   = self.levels[child_lod].grid_size();
                let cpp_z = ((child_grid.0 + parent_grid.0 - 1) / parent_grid.0).max(1);
                let cpp_y = ((child_grid.1 + parent_grid.1 - 1) / parent_grid.1).max(1);
                let cpp_x = ((child_grid.2 + parent_grid.2 - 1) / parent_grid.2).max(1);
                for dz in 0..cpp_z {
                    for dy in 0..cpp_y {
                        for dx in 0..cpp_x {
                            let (cz, cy, cx) = (tz * cpp_z + dz, ty * cpp_y + dy, tx * cpp_x + dx);
                            if cz < child_grid.0 && cy < child_grid.1 && cx < child_grid.2 {
                                stack.push((child_lod, cz, cy, cx));
                            }
                        }
                    }
                }
            }
        }
    }

    pub(crate) fn prepare_volume(
        &mut self,
        queue: &Queue,
        frame_number: u64,
        camera_info: &crate::visual::CameraInfo,
    ) {
        self.frame_counter = frame_number;
        self.update_visible_tiles_volume(camera_info);
        // Keep requests whose SPATIAL key is still visible (covers prefetched
        // future-timepoint requests where camera hasn't moved) or whose tile
        // already arrived in slot_map.
        self.requested_keys.retain(|k|
            self.visible_tile_keys.contains(&k.spatial()) || self.slot_map.contains_key(k)
        );
        // Touch LRU for the slots that are currently being displayed.
        let presentation_t = self.presentation_t;
        for spatial in &self.visible_tile_keys {
            let key = TileKey { lod_level: spatial.lod_level, t: presentation_t,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            if self.slot_map.contains_key(&key) {
                self.lru_map.insert(key, frame_number);
            }
        }
        // Eviction is now batched inside process_pending_chunks so it sizes
        // to the actual incoming work.
        self.process_pending_chunks(queue);
        // Request visible tiles at desired_t (where the user is heading).
        let target_t = self.desired_t;
        let mut visible: Vec<TileKey> = self.visible_tile_keys.iter()
            .map(|s| TileKey { lod_level: s.lod_level, t: target_t,
                               z: s.z, y: s.y, x: s.x })
            .collect();
        visible.sort_by_key(|k| Reverse(k.lod_level));
        for key in visible {
            if self.slot_map.contains_key(&key) { continue; }
            match self.request_tile(key) {
                ChunkStatus::Accepted | ChunkStatus::AlreadyPending => {}
                ChunkStatus::Rejected => break,
            }
        }
        // Once all visible-at-desired-t tiles have arrived, flip the page
        // table. Single in-frame swap, no flicker.
        self.maybe_advance_presentation(queue);
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

        // Visibility is spatial — keep requests + slots whose (lod, z, y, x)
        // is still visible regardless of their timepoint.
        self.requested_keys.retain(|k|
            self.visible_tile_keys.contains(&k.spatial()) || self.slot_map.contains_key(k)
        );

        let presentation_t = self.presentation_t;
        for spatial in &self.visible_tile_keys {
            let key = TileKey { lod_level: spatial.lod_level, t: presentation_t,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            if self.slot_map.contains_key(&key) {
                self.lru_map.insert(key, frame_number);
            }
        }

        // Eviction is now batched inside process_pending_chunks so it sizes
        // to the actual incoming work.
        self.process_pending_chunks(queue);

        // Request tiles at the timepoint the caller wants to display next.
        let target_t = self.desired_t;
        let mut visible: Vec<TileKey> = self.visible_tile_keys.iter()
            .map(|s| TileKey { lod_level: s.lod_level, t: target_t,
                               z: s.z, y: s.y, x: s.x })
            .collect();
        visible.sort_by_key(|k| Reverse(k.lod_level));
        for key in visible {
            if self.slot_map.contains_key(&key) { continue; }
            match self.request_tile(key) {
                ChunkStatus::Accepted | ChunkStatus::AlreadyPending => {}
                ChunkStatus::Rejected => break,
            }
        }
        self.maybe_advance_presentation(queue);
    }
}
