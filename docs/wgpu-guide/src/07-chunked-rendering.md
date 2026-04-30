# Chunked Rendering: Virtual Texture Architecture

Both `ImageVisual` (slice rendering) and `VolumeVisual` (ray marching) share a single back-end for streaming large multi-resolution volumes: `VirtualTextureData`. This chapter explains how it works.

<!-- toc -->

## The Problem

A typical OME-Zarr dataset might be 32,000³ voxels at the finest resolution — tens of terabytes uncompressed. Even at the coarsest level, it won't fit in GPU memory. The system needs to:

- Keep only the currently-visible tiles resident in VRAM
- Load tiles asynchronously from a remote source (S3, HTTP) without blocking the render loop
- Fall back gracefully to coarser-resolution tiles when fine-resolution ones haven't arrived yet
- Issue as few GPU draw calls as possible (ideally one per visual per frame)

## Virtual Texture Architecture

Bovista uses a virtual texture approach: the full volume is logically divided into a grid of uniformly-sized tiles at each LOD level, but only resident tiles are stored on the GPU. Two GPU textures implement the indirection:

### Atlas (`AtlasAllocator` + 3D `wgpu::Texture`)

A fixed-size 3D texture that holds all physically resident tiles, packed into a uniform grid of slots. The atlas dimensions are chosen at creation time based on `max_tiles` (user-configurable) and the tile size for the finest LOD level.

When a new tile arrives and the atlas is full, the least-recently-used slot is evicted.

### Page Table (`PageTable` + 2D-array `wgpu::Texture`)

A small integer texture with one layer per LOD level. Each texel stores either an atlas slot index for that tile (if resident) or a sentinel value (if absent). The shader looks up this indirection table at runtime to find where a tile lives in the atlas — or learns it's absent and falls back to a coarser LOD.

```
Virtual address:  (lod, z_tile, y_tile, x_tile)
           ↓
     page table lookup
           ↓
Atlas slot index → 3D atlas sample
```

Because the indirection is resolved in the shader, **both `ImageVisual` and `VolumeVisual` issue a single draw call per frame** regardless of how many tiles are resident. There are no per-tile state changes.

## Per-Frame Streaming Loop

`VirtualTextureData::prepare()` runs every frame:

1. **LOD selection** — calculates screen-space voxel error from the camera frustum and current view geometry; selects the ideal LOD level (biased by `lod_bias`)
2. **Tile visibility** — determines which tile addresses at the selected LOD intersect the visible region (slice plane footprint for `ImageVisual`, view frustum for `VolumeVisual`)
3. **Request missing tiles** — for any tile not resident, calls the user-supplied loader callback with `(lod, z, y, x)`
4. **Upload arrived tiles** — drains the `PendingChunks` queue (tiles pushed by the loader since last frame), uploads each into an atlas slot, updates the page table

## The Loader Callback

The loader is a synchronous callback that Bovista calls when it needs a tile. The loader's job is to **initiate** the load (kick off a background thread or async task) and return a status immediately. It must not block.

```
Signature: (lod: u32, z: u32, y: u32, x: u32) → ChunkStatus
```

`ChunkStatus` values:
- `Accepted` — the request was accepted; data will arrive via `set_chunk_data_u16()` soon
- `AlreadyPending` — already loading this tile; skip it
- `Rejected` — unable to accept (e.g. at capacity); try again next frame

When the data is ready, the loader (or its background task) calls `set_chunk_data_u16(lod, z, y, x, data)` to push the tile into the `PendingChunks` queue. Bovista drains this queue during the next `prepare()` call and uploads the tile to the atlas.

### Python Implementation

The Python examples use a `ThreadPoolExecutor` for background loading:

```python
from concurrent.futures import ThreadPoolExecutor
executor = ThreadPoolExecutor(max_workers=8)
pending = {}
cache = {}

def request_tile(lod, z, y, x):
    key = (lod, z, y, x)
    if key in cache:
        # Already loaded — push data again (Bovista may have evicted it)
        image.set_chunk_data_u16(lod, z, y, x, cache[key])
        return bv.ChunkStatus.Accepted
    if key in pending:
        return bv.ChunkStatus.AlreadyPending
    pending.add(key)
    executor.submit(load_and_push, key)
    return bv.ChunkStatus.Accepted

def load_and_push(key):
    lod, z, y, x = key
    data = zarr_array[lod][z*cz:(z+1)*cz, y*cy:(y+1)*cy, x*cx:(x+1)*cx]
    image.set_chunk_data_u16(lod, z, y, x, np.asarray(data, dtype=np.uint16))
    pending.discard(key)
    cache[key] = data  # optional local cache
```

### JavaScript Implementation

The web examples use `fetch()` and an `async` pipeline:

```javascript
const pending = new Set();
const cache = new Map();

function requestTile(lod, z, y, x) {
    const key = `${lod}_${z}_${y}_${x}`;
    if (cache.has(key)) {
        image.setChunkDataU16(lod, z, y, x, cache.get(key), width, height, depth);
        return ChunkStatus.Accepted;
    }
    if (pending.has(key)) return ChunkStatus.AlreadyPending;
    pending.add(key);
    fetchTile(lod, z, y, x).then(data => {
        image.setChunkDataU16(lod, z, y, x, data, width, height, depth);
        cache.set(key, data);
        pending.delete(key);
    });
    return ChunkStatus.Accepted;
}
```

## Multi-Resolution (LOD) Pyramid

Volumes are described as an array of `LevelMetadata`, one per resolution level. Level 0 is the finest.

```python
levels = [
    bv.LevelMetadata(
        volume_size=(depth, height, width),    # voxels at this level
        chunk_size=(cdepth, cheight, cwidth),  # tile size
        voxel_size=(vz, vy, vx),              # physical size in µm
        scale_factor=1.0,                      # LOD 0 is the reference
    ),
    bv.LevelMetadata(
        volume_size=(depth//2, height//2, width//2),
        chunk_size=(cdepth, cheight, cwidth),
        voxel_size=(vz, vy*2, vx*2),
        scale_factor=2.0,                      # voxels 2× larger than LOD 0
    ),
    # ... more levels (4.0, 8.0, ...)
]
```

LOD selection is screen-space: Bovista calculates how many voxels map to a single pixel at the current zoom, and selects the finest LOD where that ratio is close to 1:1.

## LOD Fallback in the Shader

The page table stores the actual LOD at which a tile was loaded, not just whether it's present. When the fragment shader samples a tile that's absent at the ideal LOD, it climbs up the page table to find the coarsest available ancestor:

```wgsl
// Simplified virtual texture lookup (virtual_tile.wgsl)
var lod = ideal_lod;
var slot = page_table_lookup(lod, tile_addr);
while slot == ABSENT && lod < max_lod {
    lod += 1u;
    slot = page_table_lookup(lod, tile_addr >> lod_shift);
}
// sample atlas at slot
```

This means coarse tiles fill in immediately while finer tiles stream in, giving smooth progressive refinement.

## Memory Budget

The atlas size (and therefore VRAM use) is controlled by `max_tiles` at visual construction time:

```python
# Each tile is tile_depth × tile_height × tile_width × 2 bytes (R16Float)
# e.g. 64³ tile = 524 KB; 2000 tiles ≈ 1 GB
image = bv.Image(viewer, levels, max_tiles=2000, loader=request_tile)
```

In the web examples, the VRAM budget selector converts a GB budget into a `max_tiles` count:

```javascript
const tileVram = tileDepth * tileHeight * tileWidth * 2; // R16Float
const maxTiles = Math.floor((budgetBytes * 0.8) / tileVram); // 80% headroom
```

## Debug Modes

Both `ImageVisual` and `VolumeVisual` support debug visualizations:

- **LOD tint** (`set_debug_mode(true)`) — colors tiles by their loaded LOD: green = finest, red = coarsest. Useful for verifying LOD selection and tile streaming.
- **Atlas debug** (`set_atlas_debug_mode(true)`, volume only) — bypasses page-table indirection and samples the raw atlas texture directly. Shows the packing of tiles.
- **Step count heatmap** (`set_step_debug_mode(true)`, volume only) — colors pixels by ray-march step count. Blue = few steps (coarse LOD or short ray), red = many.

## Summary

| Component | Role |
|-----------|------|
| `VirtualTextureData` | Orchestrates streaming, atlas, page table, LOD selection |
| `AtlasAllocator` | Manages slot assignment and LRU eviction in the 3D atlas texture |
| `PageTable` | GPU indirection texture: maps `(lod, z, y, x)` → atlas slot |
| `PendingChunks` | Lock-protected queue for pushing tile data from background threads |
| Loader callback | User code; kicks off async loads and returns `ChunkStatus` |
| `set_chunk_data_u16()` | Called by user when data arrives; pushes into `PendingChunks` |

The key architectural win: by resolving tile addresses in the shader, both `ImageVisual` and `VolumeVisual` render in a single draw call per frame regardless of how many tiles are resident or loading.

---

**Next**: [Python API →](./12-python.md)
