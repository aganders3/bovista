# Chunked Rendering: Virtual Texture Architecture

Both `Image` (slice rendering) and the volume visuals (`DirectVolume`, `MipVolume`, `MinipVolume`, `AverageVolume`, `IsosurfaceVolume` — all ray marching) share a single back-end for streaming large multi-resolution volumes: `VirtualTextureData`. This chapter explains how it works.

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

Because the indirection is resolved in the shader, **both `Image` and `DirectVolume` (and its sibling volume modes) issue a single draw call per frame** regardless of how many tiles are resident. There are no per-tile state changes.

## Per-Frame Streaming Loop

`VirtualTextureData::prepare()` runs every frame:

1. **LOD selection** — calculates screen-space voxel error from the camera frustum and current view geometry; selects the ideal LOD level (biased by `lod_bias`)
2. **Tile visibility** — determines which tile addresses at the selected LOD intersect the visible region (slice plane footprint for `Image`, view frustum for the volume visuals)
3. **Publish wanted tiles** — for any non-resident tile at the selected LOD, adds its key `(lod, t, z, y, x)` to the published "wanted" set with a priority (lower = more urgent; 0 = the currently-viewed timepoint)
4. **Upload arrived tiles** — drains the `PendingChunks` queue (tiles pushed by the application since last frame), uploads each into an atlas slot, updates the page table

## The Loader Model: Pull / Poll

Bovista never calls into application code on the hot path. Instead of pushing requests out through a callback, the engine **publishes** the set of tiles it currently wants, and the application **polls** that set on its own schedule (typically a worker thread or a timer), fetches the bytes, and pushes them back.

Concretely, each prepare/frame the engine recomputes the wanted set (step 3 above). The application reads it via `wanted_keys()` (Python) / `wantedKeys()` (JS), fetches whatever it doesn't already have in flight, and pushes results back with `set_chunk_data_u16()` / `setChunkDataU16()`.

```
Signature: wanted_keys() → list of (lod, t, z, y, x, priority)
                            sorted by priority (lower = more urgent, 0 = current timepoint)
```

Three consequences of this design:

- **No FFI on the hot path.** The engine never invokes user code mid-frame. It just exposes a snapshot of what it wants; the application decides when to read it.
- **Implicit cancellation.** A tile that is no longer wanted simply drops out of the published set. The loader sees it disappear and stops fetching — there is no explicit cancel call.
- **Deduplication is the loader's job.** Because the engine doesn't track requests, the loader is responsible for not re-fetching tiles it already has in flight (back-pressure). There is no `Accepted`/`AlreadyPending`/`Rejected` return value — the engine doesn't need one.

Tile keys are 4D-aware: they carry a timepoint `t` in addition to `(lod, z, y, x)`.

When data is ready, the application calls `set_chunk_data_u16(lod, t, z, y, x, data)` to push the tile into the `PendingChunks` queue. Bovista drains this queue during the next `prepare()` call and uploads the tile to the atlas.

### Python Implementation

The Python examples poll `wanted_keys()` and dispatch fetches to a `ThreadPoolExecutor`. The loader tracks `in_flight` itself for deduplication:

```python
from concurrent.futures import ThreadPoolExecutor
executor = ThreadPoolExecutor(max_workers=8)
in_flight = set()

def pump(volume):
    # Call this on a timer / each loop iteration — NOT from inside the engine.
    for lod, t, z, y, x, priority in volume.wanted_keys():
        key = (lod, t, z, y, x)
        if key in in_flight:
            continue  # dedup: already fetching this tile
        in_flight.add(key)
        executor.submit(load_and_push, volume, key)

def load_and_push(volume, key):
    lod, t, z, y, x = key
    data = fetch_tile(lod, t, z, y, x)  # returns a uint16 numpy array
    volume.set_chunk_data_u16(lod, t, z, y, x, np.asarray(data, dtype=np.uint16))
    in_flight.discard(key)
```

### JavaScript Implementation

The web examples poll `wantedKeys()` (a flat `Uint32Array` of 6 ints per key) and fetch with `fetch()`:

```javascript
const inFlight = new Set();

function pump(volume) {
    // Call this on a timer / requestAnimationFrame — NOT from inside the engine.
    const w = volume.wantedKeys();  // [lod, t, z, y, x, priority, ...]
    for (let i = 0; i < w.length; i += 6) {
        const [lod, t, z, y, x, prio] = w.slice(i, i + 6);
        const key = `${lod}_${t}_${z}_${y}_${x}`;
        if (inFlight.has(key)) continue;  // dedup: already fetching this tile
        inFlight.add(key);
        fetchTile(lod, t, z, y, x).then(arr => {
            volume.setChunkDataU16(lod, t, z, y, x, arr, zShape, yShape, xShape);
            inFlight.delete(key);
        });
    }
}
```

## Multi-Resolution (LOD) Pyramid

Volumes are described as an array of `LevelMetadata`, one per resolution level. Level 0 is the finest.

```python
levels = [
    bv.LevelMetadata(
        volume_size=(depth, height, width),    # voxels at this level, [z, y, x]
        chunk_size=(cdepth, cheight, cwidth),  # tile size, [z, y, x]
        voxel_size=(vz, vy, vx),              # physical size in µm, [z, y, x]
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

The first three arguments (`volume_size`, `chunk_size`, `voxel_size`) are `[z, y, x]` numpy-order sequences. An optional `translation` argument follows `scale_factor`.

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
image = bv.Image(viewer, levels, max_tiles=2000)
```

In the web examples, the VRAM budget selector converts a GB budget into a `max_tiles` count:

```javascript
const tileVram = tileDepth * tileHeight * tileWidth * 2; // R16Float
const maxTiles = Math.floor((budgetBytes * 0.8) / tileVram); // 80% headroom
```

## Debug Modes

Both `Image` and the volume visuals support debug visualizations:

- **LOD tint** (`set_debug_mode(true)`) — colors tiles by their loaded LOD: green = finest, red = coarsest. Useful for verifying LOD selection and tile streaming.
- **Atlas debug** (`set_atlas_debug_mode(true)`, volume only) — bypasses page-table indirection and samples the raw atlas texture directly. Shows the packing of tiles.
- **Step count heatmap** (`set_step_debug_mode(true)`, volume only) — colors pixels by ray-march step count. Blue = few steps (coarse LOD or short ray), red = many.

## Summary

| Component | Role |
|-----------|------|
| `VirtualTextureData` | Orchestrates streaming, atlas, page table, LOD selection |
| `AtlasAllocator` | Manages slot assignment and LRU eviction in the 3D atlas texture |
| `PageTable` | GPU indirection texture: maps `(lod, t, z, y, x)` → atlas slot |
| `PendingChunks` | Lock-protected queue for pushing tile data from background threads |
| Wanted set | Engine-published set of desired tiles; the loader polls it via `wanted_keys()` |
| `set_chunk_data_u16()` | Called by user when data arrives; pushes into `PendingChunks` |

The key architectural win: by resolving tile addresses in the shader, both `Image` and the volume visuals render in a single draw call per frame regardless of how many tiles are resident or loading.

---

**Next**: [Python API →](./12-python.md)
