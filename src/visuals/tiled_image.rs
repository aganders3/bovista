use crate::visual::{Transform, Visual};
use crate::visuals::image::SlicePlane;
use crate::visuals::chunk_visual::ChunkVisual;
use glam::Vec3;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex};
use wgpu::RenderPass;

/// Request for a chunk of data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkRequest {
    pub chunk_x: u32,
    pub chunk_y: u32,
    pub chunk_z: u32,
}

/// Response containing chunk data
pub struct ChunkData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

/// Callback for loading chunks on-demand
pub type ChunkLoaderFn = Arc<dyn Fn(ChunkRequest) -> Option<ChunkData> + Send + Sync>;

/// A single chunk of volumetric image data
struct ImageChunk {
    visual: Arc<Mutex<ChunkVisual>>,
    #[allow(dead_code)]  // Kept for potential future spatial queries
    chunk_index: (u32, u32, u32),
    #[allow(dead_code)]  // Kept for potential future spatial queries
    world_min: Vec3,
    #[allow(dead_code)]  // Kept for potential future spatial queries
    world_max: Vec3,
    last_accessed: usize,  // Frame counter for LRU
}

/// Tiled image visual that renders large volumes as separate chunks
pub struct TiledImageVisual {
    /// Grid dimensions (number of chunks in each direction)
    grid_size: (u32, u32, u32),

    /// Size of each chunk in voxels
    chunk_size: (u32, u32, u32),

    /// Total volume size in voxels
    volume_size: (u32, u32, u32),

    /// Currently loaded chunks
    chunks: HashMap<(u32, u32, u32), ImageChunk>,

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

    /// Visibility tracking
    visible_chunks: HashSet<(u32, u32, u32)>,

    /// Debug mode
    debug_mode: bool,

    /// Visual properties
    transform: Transform,
    visible: bool,
    name: String,
}

impl TiledImageVisual {
    /// Create a new tiled image visual
    pub fn new(
        volume_size: (u32, u32, u32),
        chunk_size: (u32, u32, u32),
        loader: ChunkLoaderFn,
        max_loaded_chunks: usize,
    ) -> Self {
        // Calculate grid size (number of chunks in each dimension)
        let grid_size = (
            (volume_size.0 + chunk_size.0 - 1) / chunk_size.0,
            (volume_size.1 + chunk_size.1 - 1) / chunk_size.1,
            (volume_size.2 + chunk_size.2 - 1) / chunk_size.2,
        );

        Self {
            grid_size,
            chunk_size,
            volume_size,
            chunks: HashMap::new(),
            loader,
            slice_plane: SlicePlane::default(),
            contrast_limits: (0.0, 1.0),
            max_loaded_chunks,
            frame_counter: 0,
            visible_chunks: HashSet::new(),
            debug_mode: false,
            transform: Transform::default(),
            visible: true,
            name: "TiledImage".to_string(),
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

    /// Calculate which chunks are visible based on slice plane intersection
    fn update_visible_chunks(&mut self) {
        self.visible_chunks.clear();

        // Simple brute-force approach - could be optimized with octree/BVH for very large grids
        // Note: grid_size is (z, y, x) so iterate accordingly
        // Loop variables cz/cy/cx refer to which dimension's chunks we're iterating
        for cz in 0..self.grid_size.0 {  // Z chunks (dimension 0)
            for cy in 0..self.grid_size.1 {  // Y chunks (dimension 1)
                for cx in 0..self.grid_size.2 {  // X chunks (dimension 2)
                    // CRITICAL: tuple must be (z_chunk_idx, y_chunk_idx, x_chunk_idx) = (cz, cy, cx)
                    if self.chunk_intersects_slice((cz, cy, cx)) {
                        self.visible_chunks.insert((cz, cy, cx));
                        // Debug first few visible chunks
                        if self.visible_chunks.len() <= 3 {
                            eprintln!("  update_visible_chunks: Added tuple ({},{},{}) to visible set", cz, cy, cx);
                        }
                    }
                }
            }
        }
    }

    /// Check if a chunk intersects with the current slice plane
    fn chunk_intersects_slice(&self, chunk_idx: (u32, u32, u32)) -> bool {
        // chunk_idx is (z_chunk, y_chunk, x_chunk)
        let (z_chunk, y_chunk, x_chunk) = chunk_idx;

        // Calculate world-space AABB for this chunk
        // chunk_size and volume_size are (z, y, x) dimensions
        // World space needs (x, y, z) for Vec3
        let min = Vec3::new(
            (x_chunk * self.chunk_size.2) as f32,  // X from dimension 2
            (y_chunk * self.chunk_size.1) as f32,  // Y from dimension 1
            (z_chunk * self.chunk_size.0) as f32,  // Z from dimension 0
        );

        let max = Vec3::new(
            ((x_chunk + 1) * self.chunk_size.2).min(self.volume_size.2) as f32,  // X
            ((y_chunk + 1) * self.chunk_size.1).min(self.volume_size.1) as f32,  // Y
            ((z_chunk + 1) * self.chunk_size.0).min(self.volume_size.0) as f32,  // Z
        );

        // Test if AABB intersects with slice plane
        plane_aabb_intersection(&self.slice_plane, min, max)
    }

    /// Load a chunk from the loader callback
    fn load_chunk(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        chunk_idx: (u32, u32, u32),
    ) -> bool {
        // chunk_idx is (z_chunk, y_chunk, x_chunk)
        let (z_chunk, y_chunk, x_chunk) = chunk_idx;

        // Create ChunkRequest - this will be passed to Python as (z, y, x)
        let request = ChunkRequest {
            chunk_x: x_chunk,
            chunk_y: y_chunk,
            chunk_z: z_chunk,
        };

        // Debug first few requests (always show first 5)
        static LOAD_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        let count = LOAD_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        if count < 5 {
            eprintln!("  RUST load_chunk #{}: tuple=({},{},{}) -> ChunkRequest(x={}, y={}, z={})",
                     count, z_chunk, y_chunk, x_chunk, request.chunk_x, request.chunk_y, request.chunk_z);
        }

        if let Some(chunk_data) = (self.loader)(request) {
            // Calculate world-space position
            // chunk indices are (z_chunk, y_chunk, x_chunk)
            // chunk_size is (z, y, x) dimensions
            // World space needs (x, y, z) for Vec3
            let world_min = Vec3::new(
                (x_chunk * self.chunk_size.2) as f32,  // X from dimension 2
                (y_chunk * self.chunk_size.1) as f32,  // Y from dimension 1
                (z_chunk * self.chunk_size.0) as f32,  // Z from dimension 0
            );

            let world_max = world_min
                + Vec3::new(
                    chunk_data.width as f32,   // X extent
                    chunk_data.height as f32,  // Y extent
                    chunk_data.depth as f32,   // Z extent
                );

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
                world_min,
            );

            // Apply current slice plane, contrast, and debug mode
            visual.set_slice_plane(self.slice_plane);
            visual.set_contrast_limits(self.contrast_limits.0, self.contrast_limits.1);
            visual.set_debug_mode(self.debug_mode);

            // Store the chunk
            let chunk = ImageChunk {
                visual: Arc::new(Mutex::new(visual)),
                chunk_index: chunk_idx,
                world_min,
                world_max,
                last_accessed: self.frame_counter,
            };

            self.chunks.insert(chunk_idx, chunk);
            true
        } else {
            false
        }
    }

    /// Evict chunks using LRU policy
    fn evict_chunks(&mut self) {
        if self.chunks.len() <= self.max_loaded_chunks {
            return;
        }

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
    }

    /// Prepare for rendering - load visible chunks, evict old ones
    pub fn prepare_chunks(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
    ) {
        self.frame_counter += 1;

        // Update which chunks are visible
        self.update_visible_chunks();

        // Load visible chunks that aren't loaded yet
        let visible: Vec<_> = self.visible_chunks.iter().copied().collect();
        let mut load_count = 0;
        for chunk_idx in visible {
            if !self.chunks.contains_key(&chunk_idx) {
                // Debug first few load calls
                if load_count < 3 {
                    eprintln!("  prepare_chunks: About to load chunk tuple ({},{},{})",
                             chunk_idx.0, chunk_idx.1, chunk_idx.2);
                    load_count += 1;
                }
                self.load_chunk(device, queue, surface_format, camera_bind_group_layout, chunk_idx);
            } else {
                // Update access time
                if let Some(chunk) = self.chunks.get_mut(&chunk_idx) {
                    chunk.last_accessed = self.frame_counter;
                }
            }
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

        // Render all loaded chunks that are visible
        for (chunk_idx, chunk) in &self.chunks {
            if self.visible_chunks.contains(chunk_idx) {
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
