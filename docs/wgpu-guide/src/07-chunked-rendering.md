# Chunked Rendering Architecture

The most sophisticated part of Bovista is its chunked rendering system for massive datasets. This enables viewing terabyte-scale volumes interactively.

## The Problem

**Goal**: Visualize a 32,000³ voxel volume (~32 TB uncompressed)

**Constraints**:
- GPU memory: 8-16 GB
- System RAM: 32-64 GB
- Network bandwidth: Limited (remote storage)

**Solution**: Chunk the volume and load on-demand with LRU caching.

## Architecture Overview

```
┌─────────────────────────────────────────────────┐
│          TiledImageVisual                       │
├─────────────────────────────────────────────────┤
│  Volume: 32000×32000×32000 voxels               │
│  Chunk size: 1024×1024×512 voxels               │
│  Grid: 31×31×63 = 60,453 chunks                 │
│  Max loaded: 150 chunks (~1 GB)                 │
│  Current slice: Z = 16000                       │
└─────────────────────────────────────────────────┘
         │
         ├── update_visible_chunks()
         │   └→ Test each chunk's AABB against slice plane
         │       Result: ~10-20 visible chunks
         │
         ├── load_chunk()
         │   └→ Call loader(chunk_z, chunk_y, chunk_x)
         │       ├─ Python: Fetch from Zarr/S3
         │       ├─ WASM: Fetch from browser cache/network
         │       └─ Return: ChunkData { data: Vec<u8>, width, height, depth }
         │
         ├── evict_chunks()
         │   └→ LRU eviction if > max_loaded_chunks
         │       ├─ Sort by last_accessed timestamp
         │       └─ Drop least recently used
         │
         └── prepare_chunks()
             └→ For each loaded chunk:
                 ├─ Compute plane-AABB intersection
                 ├─ Generate geometry (vertices, indices)
                 └─ Upload to GPU buffers
```

## Data Structures

From `src/visuals/tiled_image.rs:108-149`:

```rust
pub struct TiledImageVisual {
    /// Grid dimensions (number of chunks in each direction)
    grid_size: (u32, u32, u32),  // e.g., (31, 31, 63)

    /// Size of each chunk in voxels (Z, Y, X)
    chunk_size: (u32, u32, u32),  // e.g., (512, 1024, 1024)

    /// Total volume size in voxels
    volume_size: (u32, u32, u32),  // e.g., (32000, 32000, 32000)

    /// Physical size of each voxel in world space
    voxel_size: (f32, f32, f32),  // e.g., (0.5, 0.5, 0.5) microns

    /// Currently loaded chunks
    chunks: HashMap<(u32, u32, u32), ImageChunk>,  // Key: (z, y, x) chunk index

    /// Chunk loader callback
    loader: ChunkLoaderFn,  // Arc<dyn Fn(ChunkRequest) -> Option<ChunkData>>

    /// Slice plane for rendering
    slice_plane: SlicePlane,

    /// Maximum number of chunks to keep loaded
    max_loaded_chunks: usize,  // e.g., 150

    /// Frame counter for LRU tracking
    frame_counter: usize,

    /// Visibility tracking
    visible_chunks: HashSet<(u32, u32, u32)>,

    // ... other fields ...
}
```

### ImageChunk

```rust
struct ImageChunk {
    visual: Arc<Mutex<ChunkVisual>>,  // The actual renderable chunk
    chunk_index: (u32, u32, u32),     // Position in grid
    world_min: Vec3,                  // AABB min in world space
    world_max: Vec3,                  // AABB max in world space
    last_accessed: usize,             // Frame number (for LRU)
}
```

## The Loading Pipeline

### Step 1: Visibility Culling

Test each chunk in the grid against the slice plane:

```rust
fn update_visible_chunks(&mut self) {
    self.visible_chunks.clear();

    for cz in 0..self.grid_size.0 {
        for cy in 0..self.grid_size.1 {
            for cx in 0..self.grid_size.2 {
                if self.chunk_intersects_slice((cz, cy, cx)) {
                    self.visible_chunks.insert((cz, cy, cx));
                }
            }
        }
    }
}

fn chunk_intersects_slice(&self, chunk_idx: (u32, u32, u32)) -> bool {
    // Compute chunk AABB in world space
    let min = /* ... */;
    let max = /* ... */;

    // Test plane-AABB intersection
    plane_aabb_intersection(&self.slice_plane, min, max)
}
```

**Result**: ~10-20 visible chunks out of 60,000 total.

### Step 2: Load Missing Chunks

```rust
fn load_missing_chunks(&mut self, device: &Device, queue: &Queue) {
    for &chunk_idx in &self.visible_chunks {
        if !self.chunks.contains_key(&chunk_idx) {
            // Not loaded yet—request it
            if let Some(chunk_data) = (self.loader)(ChunkRequest {
                chunk_z: chunk_idx.0,
                chunk_y: chunk_idx.1,
                chunk_x: chunk_idx.2,
            }) {
                // Create ChunkVisual
                let visual = ChunkVisual::new(
                    device,
                    queue,
                    // ... parameters ...
                    &chunk_data.data,
                    chunk_data.width,
                    chunk_data.height,
                    chunk_data.depth,
                    // ...
                );

                // Store in map
                self.chunks.insert(chunk_idx, ImageChunk {
                    visual: Arc::new(Mutex::new(visual)),
                    chunk_index: chunk_idx,
                    world_min,
                    world_max,
                    last_accessed: self.frame_counter,
                });
            }
        }
    }
}
```

### Step 3: Update Access Times

```rust
fn update_access_times(&mut self) {
    for &chunk_idx in &self.visible_chunks {
        if let Some(chunk) = self.chunks.get_mut(&chunk_idx) {
            chunk.last_accessed = self.frame_counter;
        }
    }
    self.frame_counter += 1;
}
```

### Step 4: LRU Eviction

```rust
fn evict_excess_chunks(&mut self) {
    if self.chunks.len() <= self.max_loaded_chunks {
        return;  // Under limit
    }

    // Sort chunks by access time
    let mut chunks_by_access: Vec<_> = self.chunks.iter()
        .map(|(idx, chunk)| (*idx, chunk.last_accessed))
        .collect();

    chunks_by_access.sort_by_key(|(_, access)| *access);

    // Remove oldest until under limit
    let to_remove = self.chunks.len() - self.max_loaded_chunks;
    for (chunk_idx, _) in chunks_by_access.iter().take(to_remove) {
        self.chunks.remove(chunk_idx);
        // ChunkVisual dropped here, GPU resources freed
    }
}
```

## Loader Callback Pattern

### Python Example

```python
import zarr

# Open remote Zarr array
store = zarr.storage.FSStore('s3://bucket/dataset.zarr')
array = zarr.open(store, mode='r')

def load_chunk(z, y, x):
    # Zarr uses (Z, Y, X) indexing
    cz, cy, cx = chunk_size
    chunk_data = array[
        z * cz:(z + 1) * cz,
        y * cy:(y + 1) * cy,
        x * cx:(x + 1) * cx,
    ]
    return np.asarray(chunk_data, dtype=np.uint8)

# Create tiled visual with loader
tiled = bv.TiledImage.from_loader(
    viewer,
    volume_size=(32000, 32000, 32000),
    chunk_size=(512, 1024, 1024),
    loader=load_chunk,
    max_loaded_chunks=150
)
```

### WASM/Browser Example

```rust
// Cache chunks fetched from server
static mut CHUNK_CACHE: Option<Rc<RefCell<HashMap<(u32, u32, u32), Vec<u8>>>>> = None;

let loader: ChunkLoaderFn = Arc::new(|req| {
    unsafe {
        if let Some(cache) = &CHUNK_CACHE {
            if let Some(data) = cache.borrow().get(&(req.chunk_z, req.chunk_y, req.chunk_x)) {
                return Some(ChunkData {
                    data: data.clone(),
                    width, height, depth,
                });
            }
        }

        // Not in cache—request from server
        request_chunk_load(req.chunk_z, req.chunk_y, req.chunk_x);
        None  // Return None for now, will be available next frame
    }
});
```

## Memory Management

### Memory Calculation

**Target: ~1 MB chunks for optimal performance**

```
Small chunks (< 1 MB, very thin Z slices):
  1 × 275 × 271 = 74,525 bytes = ~73 KB per chunk

  With 500 chunks → 36.6 MB total
  Pros: Minimal memory, very fast loading
  Cons: Many chunks needed per slice
  Use case: Highly anisotropic microscopy data

Target size (~1 MB, optimal):
  128 × 128 × 64 = 1,048,576 bytes = 1 MB per chunk
  OR
  16 × 256 × 256 = 1,048,576 bytes = 1 MB per chunk (anisotropic)

  With 500 chunks → 500 MB total
  With 1000 chunks → 1 GB total

  Pros: Fast network loading, good spatial coverage, reasonable memory
  Cons: None - this is the sweet spot!
  Use case: Most volumetric datasets

Larger chunks (4-8 MB, when fewer chunks needed):
  256 × 256 × 64 = 4,194,304 bytes = ~4 MB per chunk

  With 150 chunks → 600 MB total
  With 250 chunks → 1 GB total

  Pros: Fewer chunks to cover large areas
  Cons: Slower network loading, more wasted data when only partial chunk visible
  Use case: Lower resolution levels, overview rendering
```

**Why ~1 MB is optimal:**
- ✅ **Network speed**: ~100-200 ms to load from S3 (vs. seconds for 64 MB chunks)
- ✅ **Memory efficiency**: 500 chunks = only 500 MB (vs. gigabytes)
- ✅ **Spatial coverage**: Good balance between chunk count and area covered
- ✅ **GPU upload**: Fast texture creation (<10 ms)

**Key insight**: Bovista's chunk size **must match your data's native chunking** (Zarr `.chunks`):
- Optimal I/O: one data chunk = one network request = one visual chunk
- No resampling or rechunking needed
- Data should be written with ~1 MB chunks for best performance

### Automatic Cleanup

When chunks are dropped:

```rust
impl Drop for ImageChunk {
    fn drop(&mut self) {
        // ChunkVisual dropped
        //   → texture dropped
        //   → vertex_buffer dropped
        //   → GPU memory freed
    }
}
```

WGPU automatically frees GPU resources when Rust drops them.

## Performance Characteristics

### Chunk Loading Latency

**Local disk**:
- Read 73 KB chunk: ~1 ms (SSD)
- Read 1 MB chunk: ~5 ms (SSD)
- Read 4 MB chunk: ~15 ms (SSD)
- Upload to GPU: ~2-10 ms (scales with size)

**Remote (S3/cloud storage)**:
- 73 KB chunk: ~50-100 ms network fetch ✅
- 1 MB chunk: ~100-200 ms network fetch ✅ (optimal!)
- 4 MB chunk: ~300-500 ms network fetch
- 32 MB chunk: ~1000-2000 ms network fetch ❌ (too slow!)
- Upload to GPU: ~2-10 ms

**Why ~1 MB is the sweet spot:**
- Fast enough for interactive navigation
- Small enough to minimize wasted data
- Large enough for reasonable spatial coverage
- Balances network latency vs. bandwidth

**Optimization**: Prefetch neighboring chunks in background.

### Frame Rates

**Typical workload**:
- 15 visible chunks per frame
- Each chunk: 6-8 triangles (triangle fan)
- **Total**: ~100 triangles per frame

**Result**: 60+ FPS on integrated GPUs.

**Bottleneck**: Network bandwidth, not rendering.

## Advanced Optimizations

### 1. Level of Detail (LOD)

```rust
struct MultiResolutionVolume {
    levels: Vec<TiledImageVisual>,  // [full-res, half-res, quarter-res, ...]
}

impl MultiResolutionVolume {
    fn get_appropriate_level(&self, camera_distance: f32) -> &TiledImageVisual {
        // Use lower res for distant views
        let lod_index = (camera_distance / 100.0).log2().floor() as usize;
        &self.levels[lod_index.min(self.levels.len() - 1)]
    }
}
```

### 2. Chunk Prefetching

```rust
fn prefetch_neighbors(&mut self, chunk_idx: (u32, u32, u32)) {
    for dz in -1..=1 {
        for dy in -1..=1 {
            for dx in -1..=1 {
                let neighbor = (
                    chunk_idx.0 + dz,
                    chunk_idx.1 + dy,
                    chunk_idx.2 + dx,
                );
                // Request in background
                self.request_load(neighbor);
            }
        }
    }
}
```

### 3. Compression

```python
# Zarr supports compression
array = zarr.open(store, mode='r', compressor=zarr.Blosc(cname='zstd'))
# Chunks stored compressed, decompressed on read
```

**Benefit**: Significant size reduction (e.g., 1 MB → 100-300 KB compressed, 4 MB → 500 KB - 1 MB compressed).

### 4. Caching Layer

```
Browser Cache
    ↓
IndexedDB
    ↓
Service Worker
    ↓
Network (S3)
```

**Result**: Instant load for recently viewed slices.

## Debugging

### Statistics Display

```rust
impl TiledImageVisual {
    pub fn get_stats(&self) -> (usize, usize, usize) {
        (
            self.chunks.len(),              // Loaded
            self.visible_chunks.len(),      // Visible
            self.grid_size.0 * self.grid_size.1 * self.grid_size.2,  // Total
        )
    }
}
```

```python
# Display in UI
loaded, visible, total = tiled.get_stats()
print(f"Chunks: {loaded} loaded, {visible} visible, {total} total")
```

### Debug Visualization

Color-code chunks by Z-layer:

```wgsl
// In chunk shader
if (chunk.debug_mode > 0.5) {
    let tinted = mix(vec3<f32>(value), chunk.debug_color, 0.3);
    return vec4<f32>(tinted, 1.0);
}
```

**Result**: Different colors per Z-layer, easy to see chunk boundaries.

## Summary

**Key Techniques**:
1. **Spatial partitioning**: Divide volume into manageable chunks
2. **Visibility culling**: Only load chunks intersecting slice
3. **LRU caching**: Automatic eviction of old chunks
4. **Async loading**: Callback pattern for flexible backends
5. **Dynamic geometry**: Compute slice intersection per chunk

**Performance**:
- Interactive framerates (60+ FPS)
- Memory-bounded (150 chunks ≈ 10 GB)
- Works with terabyte-scale data

**Flexibility**:
- Any chunk size
- Any data source (Zarr, HDF5, S3, local files)
- Any platform (native, web)

---

**Next**: [Bind Groups Deep Dive →](./10-bind-groups.md)
