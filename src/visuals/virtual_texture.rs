//! Virtual-texture rendering data: atlas, page table, LOD selection, tile streaming.

use crate::visuals::gpu_structs::{TileData, TileKey, AABB};
use crate::visuals::image::SlicePlane;
use crate::visuals::atlas::AtlasAllocator;
use crate::visuals::page_table::PageTable;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use wgpu::{Device, Queue};

/// Type alias for pending chunks queue (can be shared across threads)
pub type PendingChunks = Arc<Mutex<HashMap<TileKey, TileData>>>;

/// Set of tiles bovista currently wants in the atlas, keyed by full
/// TileKey, value = priority (lower = more urgent). Rebuilt from
/// scratch at the end of every prepare; loaders read it to decide
/// what to fetch.
pub type Wanted = Arc<Mutex<HashMap<TileKey, i32>>>;

/// Per-prepare timing + counts. Refreshed by every `prepare` /
/// `prepare_volume` call, exposed via `VolumeVisual::stats()` /
/// `ImageVisual::stats()` for performance diagnostics. Read it from
/// the render loop after `scene.prepare` returns; it reflects the
/// most recent prepare.
#[derive(Default, Clone, Debug)]
pub struct PrepareStats {
    /// Frame number this snapshot covers.
    pub frame: u64,
    /// Tiles drained from `pending_chunks` and successfully written.
    pub tiles_installed: usize,
    /// Tiles drained from `pending_chunks` but dropped (slot_map
    /// already had them, no longer visible, or `alloc_or_evict` returned
    /// None because no resident slot was lower-priority).
    pub tiles_dropped: usize,
    /// Wallclock time spent in `process_pending_chunks`.
    pub process_pending_ns: u64,
    /// Wallclock time spent in `refresh_page_table`.
    pub refresh_page_table_ns: u64,
    /// Wallclock time spent in `publish_wanted`.
    pub publish_wanted_ns: u64,
    /// `queue.write_texture` calls into the atlas this prepare.
    pub atlas_writes: usize,
    /// `queue.write_texture` calls into the page table this prepare.
    pub page_table_writes: usize,
    /// `slot_map` size at end of prepare (atlas occupancy).
    pub slot_map_len: usize,
    /// `pending_chunks` size at end of prepare (tiles waiting after
    /// any per-frame cap).
    pub pending_len: usize,
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
    // SPATIAL key (t forced to 0) → atlas slot the page table currently
    // points at for that spatial. Two uses, both about gating GPU writes:
    //   1. `refresh_page_table` skips `queue.write_texture` calls for
    //      spatials whose entry already points at the right slot.
    //   2. On eviction, lets us detect when the slot a page-table entry
    //      references is about to be recycled, so we can clear the
    //      entry instead of letting the shader read garbage.
    // NOT consulted by the eviction filter — that's the deadlock trap.
    last_written_slot: HashMap<TileKey, u32>,
    // TileKey → last frame the tile was accessed
    lru_map: HashMap<TileKey, u64>,

    /// Tile bytes that have been fetched (by whoever the consumer
    /// wired up) and are waiting to be uploaded to the atlas. Drained
    /// at the top of every prepare via `process_pending_chunks`.
    pub pending_chunks: PendingChunks,
    /// Set of tiles bovista currently wants resident in the atlas,
    /// keyed by full TileKey. Value is priority (lower = more urgent;
    /// 0 means "user is looking at this t now"). Rebuilt atomically
    /// from scratch at the end of every prepare; loaders read it and
    /// decide what to fetch and in what order. No callbacks.
    pub wanted: Wanted,

    /// Diagnostic snapshot of the most recent prepare. Read by the
    /// example's render loop to print [perf] lines.
    pub stats: PrepareStats,

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
    /// Timepoint the caller wants to display. Missing tiles at this t
    /// fall back to coarser LODs via the shader's existing LOD-walk;
    /// the shader's `desired_t` uniform compare also rejects any stale
    /// entries left in the page table from a previous t. As fine tiles
    /// arrive via `process_pending_chunks` the page table refines.
    desired_t: u32,
    /// Prefetch ±N timepoints around `desired_t` every prepare. Runs
    /// through the SAME request path as current-t (slot_map check +
    /// request_tile dedupe), so the atlas is the single source of
    /// truth — already-resident tiles cost a HashMap lookup, never a
    /// re-decode. 0 disables.
    prefetch_lookahead: u32,
    /// Total number of timepoints in the dataset. Used to clamp the
    /// prefetch range; otherwise the strategy has no way to know it.
    t_count: u32,
}

impl VirtualTextureData {
    pub fn new(
        device: &Device,
        lod_levels: Vec<LodLevelConfig>,
        max_tiles: usize,
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
            last_written_slot: HashMap::new(),
            lru_map: HashMap::new(),
            pending_chunks: Arc::new(Mutex::new(HashMap::new())),
            wanted: Arc::new(Mutex::new(HashMap::new())),
            stats: PrepareStats::default(),
            max_tiles,
            frame_counter: 0,
            visible_tile_keys: HashSet::new(),
            lod_bias: 0.0,
            target_pixels_per_voxel: 1.0,
            cached_ideal_lod: 0,
            levels: lod_levels,
            desired_t: 0,
            prefetch_lookahead: 0,
            t_count: 1,
        }
    }

    /// Configure background prefetching of neighboring timepoints.
    /// `t_count` is the total number of timepoints in the dataset
    /// (used to clamp range). Pass `lookahead = 0` to disable.
    pub fn set_prefetch(&mut self, lookahead: u32, t_count: u32) {
        self.prefetch_lookahead = lookahead;
        self.t_count = t_count.max(1);
    }

    /// Caller-facing knob: request that the displayed timepoint be `t`.
    /// `refresh_page_table` on the next `prepare()` call points the
    /// page table at any resident tiles at `t`. Spatials with no
    /// resident `t` tile fall back through the shader's LOD chain;
    /// they snap in as fine tiles arrive in `process_pending_chunks`.
    pub fn set_desired_timepoint(&mut self, t: u32) {
        self.desired_t = t;
    }

    pub fn desired_t(&self) -> u32 { self.desired_t }

    /// (resident, visible) — how many of the visible spatial tiles at
    /// `desired_t` are actually in the atlas right now. Useful for
    /// progress reporting; returns (0, 0) before the first prepare.
    pub fn current_t_load_status(&self) -> (usize, usize) {
        let target = self.desired_t;
        let resident = self.visible_tile_keys.iter()
            .filter(|s| {
                let key = TileKey { lod_level: s.lod_level, t: target,
                                    z: s.z, y: s.y, x: s.x };
                self.slot_map.contains_key(&key)
            })
            .count();
        (resident, self.visible_tile_keys.len())
    }

    // ── Tile management ──────────────────────────────────────────────────────

    /// Drop all resident tiles. Now that `TileKey` includes `t`, the
    /// preferred way to switch timepoints is `set_desired_timepoint(t)` —
    /// adjacent t's stay resident in the atlas as cache and the page table
    /// flips atomically once the new t is fully loaded (no flicker).
    /// `clear_atlas` is retained for unusual cases where the underlying
    /// data shape itself has changed.
    pub fn clear_atlas(&mut self, queue: &Queue) {
        // Clear every page-table entry that's been written.
        for spatial in self.last_written_slot.keys() {
            self.page_table.clear(queue, spatial.lod_level, spatial.z, spatial.y, spatial.x);
        }
        self.last_written_slot.clear();
        self.slot_map.clear();
        self.lru_map.clear();
        self.wanted.lock().unwrap().clear();
        self.pending_chunks.lock().unwrap().clear();
        self.atlas_allocator.reset();
    }

    fn process_pending_chunks(&mut self, queue: &Queue) {
        let started = std::time::Instant::now();
        let pending: Vec<(TileKey, TileData)> = {
            let mut guard = self.pending_chunks.lock().unwrap();
            guard.drain().collect()
        };
        if pending.is_empty() {
            self.stats.process_pending_ns = started.elapsed().as_nanos() as u64;
            return;
        }

        // Cap how many tiles we install per frame. Each
        // queue.write_texture on the GL backend is ~3 ms; 16 tiles is
        // ~48 ms — enough headroom inside a 33 ms frame budget for
        // render-thread work to stay responsive. Excess pending tiles
        // get pushed back into `pending` (HashMap, no dedup needed
        // since publish_wanted skips pending entries so workers won't
        // re-fetch them).
        const MAX_TILES_PER_FRAME: usize = 16;

        // Build the eviction candidate list ONCE
        // (single O(N log N) sort vs O(N) scan per tile).
        let current_t = self.desired_t;
        let frame_counter = self.frame_counter;
        let mut candidates: Vec<(u64, u64, TileKey, u32)> = self.slot_map.iter()
            .map(|(k, &slot)| {
                let eff = if self.visible_tile_keys.contains(&k.spatial()) {
                    (k.t as i64 - current_t as i64).unsigned_abs()
                } else {
                    u64::MAX
                };
                let age = frame_counter
                    .saturating_sub(*self.lru_map.get(k).unwrap_or(&0));
                (eff, age, *k, slot)
            })
            .collect();
        candidates.sort_by_key(|&(eff, age, _, _)| (eff, age));

        let mut installed = 0usize;
        let mut dropped = 0usize;
        let mut page_table_writes = 0usize;
        let mut deferred: Vec<(TileKey, TileData)> = Vec::new();

        for (key, data) in pending {
            if installed >= MAX_TILES_PER_FRAME {
                // Frame budget spent — defer the rest. Order doesn't
                // matter; whichever this worker decoded next frame
                // will get processed first.
                deferred.push((key, data));
                continue;
            }
            if self.slot_map.contains_key(&key) {
                dropped += 1;
                continue;
            }
            if !self.visible_tile_keys.contains(&key.spatial()) {
                dropped += 1;
                continue;
            }
            let Some(slot) = self.alloc_or_evict(queue, key.t, &mut candidates) else {
                dropped += 1;
                continue;
            };

            self.write_tile_to_atlas(queue, slot, &data);

            let TileKey { lod_level, t, z, y, x } = key;
            if t == self.desired_t {
                self.page_table.update(queue, lod_level, z, y, x, t, slot);
                self.last_written_slot.insert(key.spatial(), slot);
                page_table_writes += 1;
            }

            self.slot_map.insert(key, slot);
            self.lru_map.insert(key, self.frame_counter);
            installed += 1;
        }
        // Re-queue deferred entries into pending for next frame.
        if !deferred.is_empty() {
            let mut guard = self.pending_chunks.lock().unwrap();
            for (key, data) in deferred {
                guard.insert(key, data);
            }
        }
        self.stats.process_pending_ns = started.elapsed().as_nanos() as u64;
        self.stats.tiles_installed = installed;
        self.stats.tiles_dropped = dropped;
        self.stats.atlas_writes = installed;
        self.stats.page_table_writes = page_table_writes;
    }

    fn write_tile_to_atlas(&self, queue: &Queue, slot: u32, data: &TileData) {
        let col   = slot % self.atlas_cols;
        let row   = (slot / self.atlas_cols) % self.atlas_rows;
        let layer = slot / (self.atlas_cols * self.atlas_rows);
        let (tile_d, tile_h, tile_w) = self.tile_size;
        let origin = wgpu::Origin3d {
            x: col   * tile_w,
            y: row   * tile_h,
            z: layer * tile_d,
        };
        let bpv = data.bytes_per_voxel() as usize;

        // Boundary tiles (z/y/x edge of the volume at this LOD) have
        // smaller extent than the slot. Without zero-padding, the
        // remainder of the slot would still hold whatever a previous
        // tile left there, and the shader's per-LOD data_scale would
        // sample into that stale data.
        let is_full =
            data.width == tile_w && data.height == tile_h && data.depth == tile_d;
        if is_full {
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                &data.data,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(data.width * bpv as u32),
                    rows_per_image: Some(data.height),
                },
                wgpu::Extent3d {
                    width: data.width,
                    height: data.height,
                    depth_or_array_layers: data.depth,
                },
            );
        } else {
            let stride_row_dst = tile_w as usize * bpv;
            let stride_slice_dst = stride_row_dst * tile_h as usize;
            let stride_row_src = data.width as usize * bpv;
            let mut padded = vec![0u8; stride_slice_dst * tile_d as usize];
            for z in 0..data.depth as usize {
                for y in 0..data.height as usize {
                    let dst = z * stride_slice_dst + y * stride_row_dst;
                    let src = z * (data.height as usize) * stride_row_src
                            + y * stride_row_src;
                    padded[dst..dst + stride_row_src]
                        .copy_from_slice(&data.data[src..src + stride_row_src]);
                }
            }
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.atlas_texture,
                    mip_level: 0,
                    origin,
                    aspect: wgpu::TextureAspect::All,
                },
                &padded,
                wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(tile_w * bpv as u32),
                    rows_per_image: Some(tile_h),
                },
                wgpu::Extent3d {
                    width: tile_w,
                    height: tile_h,
                    depth_or_array_layers: tile_d,
                },
            );
        }
    }

    /// For every visible spatial, point the page table at the slot
    /// holding the `desired_t` tile if resident. Tiles at other
    /// timepoints sit in slot_map silently — the shader's `desired_t`
    /// compare rejects any page-table entries left at other t's, so no
    /// explicit clear is needed when desired_t changes.
    ///
    /// Two cheap gates keep the GL traffic bounded:
    ///   - skip spatials with no resident `desired_t` tile (slot_map miss),
    ///   - skip spatials where `last_written_slot` already shows this slot.
    /// In steady state (page table fully wired for desired_t), this runs
    /// zero `queue.write_texture` calls.
    ///
    /// Rebuild `wanted` atomically from current state: every visible
    /// spatial at `desired_t` (priority 0), plus the next N prefetch
    /// timepoints (priority = offset), skipping anything already in
    /// `slot_map`. Loaders read this map and decide what to fetch and
    /// in what order.
    fn publish_wanted(&mut self) {
        let t0 = std::time::Instant::now();
        let lookahead = self.prefetch_lookahead as i32;
        // Snapshot pending keys so a worker doesn't re-fetch a tile
        // that's already decoded and waiting in line to install.
        let pending_keys: HashSet<TileKey> = {
            let guard = self.pending_chunks.lock().unwrap();
            guard.keys().copied().collect()
        };
        let mut next: HashMap<TileKey, i32> = HashMap::with_capacity(
            self.visible_tile_keys.len() * (1 + lookahead.max(0) as usize),
        );
        for spatial in &self.visible_tile_keys {
            for offset in 0..=lookahead {
                let t_i = self.desired_t as i32 + offset;
                if t_i < 0 || (self.t_count > 1 && t_i >= self.t_count as i32) {
                    continue;
                }
                let key = TileKey {
                    lod_level: spatial.lod_level,
                    t: t_i as u32,
                    z: spatial.z, y: spatial.y, x: spatial.x,
                };
                if self.slot_map.contains_key(&key) { continue; }
                if pending_keys.contains(&key) { continue; }
                next.insert(key, offset);
            }
        }
        *self.wanted.lock().unwrap() = next;
        self.stats.publish_wanted_ns = t0.elapsed().as_nanos() as u64;
    }

    /// O(visible) per call; cheap enough to run every prepare.
    fn refresh_page_table(&mut self, queue: &Queue) {
        let t0 = std::time::Instant::now();
        let target = self.desired_t;
        let mut writes = 0usize;
        let visible: Vec<TileKey> =
            self.visible_tile_keys.iter().copied().collect();
        for spatial in &visible {
            let key = TileKey { lod_level: spatial.lod_level, t: target,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            let Some(&slot) = self.slot_map.get(&key) else { continue; };
            if self.last_written_slot.get(spatial) == Some(&slot) { continue; }
            self.page_table.update(queue,
                spatial.lod_level, spatial.z, spatial.y, spatial.x, target, slot);
            self.last_written_slot.insert(*spatial, slot);
            writes += 1;
        }
        self.stats.refresh_page_table_ns = t0.elapsed().as_nanos() as u64;
        // Add to atlas-vs-page-table breakdown so it's clear how many
        // page-table writes came from this path vs process_pending.
        self.stats.page_table_writes += writes;
    }

    /// Return an atlas slot suitable for storing a tile at `tile_t`. If the
    /// allocator has a free slot, use it. Otherwise pick the worst-priority
    /// resident slot and evict it — but only if it really is lower-priority
    /// than the incoming tile.
    ///
    /// Effective distance per slot:
    ///   non-visible spatial → u64::MAX (always evictable)
    ///   visible spatial     → |slot.t - desired_t|
    /// Within a tier, oldest LRU loses. The incoming tile claims the slot
    /// iff `eff > |tile_t - desired_t|`, so a current-t arrival (dist 0)
    /// can claim any non-current slot; a prefetch at distance D can only
    /// claim slots at distance > D.
    /// Return a slot for a tile at `tile_t`. Free-list first; if
    /// exhausted, evict the worst-priority entry from `candidates`
    /// (a sorted-ascending list built once per `process_pending`
    /// batch — see there for the priority rule).
    fn alloc_or_evict(
        &mut self,
        queue: &Queue,
        tile_t: u32,
        candidates: &mut Vec<(u64, u64, TileKey, u32)>,
    ) -> Option<u32> {
        if let Some(slot) = self.atlas_allocator.alloc() {
            return Some(slot);
        }
        let current = self.desired_t;
        let incoming_dist = (tile_t as i64 - current as i64).unsigned_abs();
        // Pop from the prebuilt sorted list (tail is most-evictable).
        // Skip entries whose slot already changed hands earlier in
        // this same batch (e.g. another iteration already evicted
        // them).
        while let Some((eff, age, vkey, vslot)) = candidates.pop() {
            if eff <= incoming_dist {
                // Push back: a later iteration with smaller
                // incoming_dist may still be able to claim this.
                candidates.push((eff, age, vkey, vslot));
                return None;
            }
            if self.slot_map.get(&vkey) != Some(&vslot) {
                continue;
            }
            self.slot_map.remove(&vkey);
            self.lru_map.remove(&vkey);
            self.atlas_allocator.free(vslot);
            let spatial = vkey.spatial();
            if self.last_written_slot.get(&spatial) == Some(&vslot) {
                let TileKey { lod_level, z, y, x, .. } = vkey;
                self.page_table.clear(queue, lod_level, z, y, x);
                self.last_written_slot.remove(&spatial);
            }
            return self.atlas_allocator.alloc();
        }
        None
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
        self.stats = PrepareStats { frame: frame_number, ..Default::default() };
        self.frame_counter = frame_number;
        self.update_visible_tiles_volume(camera_info);
        // Touch LRU for displayed slots (at desired_t — anything else
        // is shader-filtered out by the page-table t-compare).
        let target = self.desired_t;
        for spatial in &self.visible_tile_keys {
            let key = TileKey { lod_level: spatial.lod_level, t: target,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            if self.slot_map.contains_key(&key) {
                self.lru_map.insert(key, frame_number);
            }
        }
        self.process_pending_chunks(queue);
        self.refresh_page_table(queue);
        // Publish what we want; the loader (whoever wired one up) picks
        // it up on its next poll.
        self.publish_wanted();
        self.stats.slot_map_len = self.slot_map.len();
        self.stats.pending_len = self.pending_chunks.lock().unwrap().len();
    }

    // ── Main prepare ─────────────────────────────────────────────────────────

    pub(crate) fn prepare(
        &mut self,
        queue: &Queue,
        slice_plane: &SlicePlane,
        frame_number: u64,
        camera_info: &crate::visual::CameraInfo,
    ) {
        self.stats = PrepareStats { frame: frame_number, ..Default::default() };
        self.frame_counter = frame_number;
        self.update_visible_tiles(slice_plane, camera_info);

        // Touch LRU for displayed slots (at desired_t — anything else
        // is shader-filtered).
        let target = self.desired_t;
        for spatial in &self.visible_tile_keys {
            let key = TileKey { lod_level: spatial.lod_level, t: target,
                                z: spatial.z, y: spatial.y, x: spatial.x };
            if self.slot_map.contains_key(&key) {
                self.lru_map.insert(key, frame_number);
            }
        }

        self.process_pending_chunks(queue);
        self.refresh_page_table(queue);
        self.publish_wanted();
        self.stats.slot_map_len = self.slot_map.len();
        self.stats.pending_len = self.pending_chunks.lock().unwrap().len();
    }
}
