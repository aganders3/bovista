# Virtual Textures

Volumetric datasets routinely dwarf VRAM — light-sheet and whole-brain imaging reach tens of terabytes, and 4D time-series go further. Bovista renders them by streaming through a **virtual texture**: the volume is divided into tiles at multiple LOD levels, and only the visible ones — typically tens resident out of millions virtual — ever reach the GPU. VRAM stays fixed (a bounded atlas with LRU eviction) regardless of dataset size, so terabyte-scale data renders interactively straight from remote storage (S3 / Zarr / HTTP).

This back-end (`src/visuals/virtual_texture.rs`) is shared by `Image` (slice rendering) and the volume visuals (`DirectVolume`, `MipVolume`, `MinipVolume`, `AverageVolume`, `IsosurfaceVolume`). `Points`, `Lines`, and `Custom` visuals don't use it.

## Atlas and Page Table

The full volume is logically a grid of uniform tiles at each LOD, but only resident tiles live on the GPU. Two textures implement the indirection:

- **Atlas** (`AtlasAllocator` + 3D `wgpu::Texture`) — a fixed-size 3D texture holding all resident tiles in a uniform grid of slots, sized at creation from `max_tiles` and the finest LOD's tile size. When it fills, the least-recently-used slot is evicted.
- **Page table** (`PageTable` + 2D-array `wgpu::Texture`; `src/visuals/page_table.rs`) — a small integer texture with one layer per LOD. Each texel holds an atlas slot index (resident) or a sentinel (absent). The shader reads it at runtime to locate a tile — or learn it's absent and fall back to a coarser LOD.

```
Virtual address:  (lod, z_tile, y_tile, x_tile)
           ↓
     page table lookup
           ↓
Atlas slot index → 3D atlas sample
```

Because the indirection is resolved in the shader, both `Image` and the volume visuals issue a **single draw call per frame** regardless of resident tile count — no per-tile state changes.

## Per-Frame Streaming Loop

`VirtualTextureData::prepare()` runs every frame:

1. **LOD selection** — screen-space voxel error from the camera selects the ideal LOD (biased by `lod_bias`).
2. **Tile visibility** — finds which tile addresses at that LOD intersect the view (slice-plane footprint for `Image`, frustum for volumes).
3. **Publish wanted tiles** — adds each non-resident tile's key `(lod, t, z, y, x)` to the published "wanted" set with a priority (lower = more urgent).
4. **Upload arrived tiles** — drains `PendingChunks` (tiles pushed since the last frame) into atlas slots and updates the page table.

Loading is **pull-based**: the engine only publishes what it wants; the app polls that set and pushes tiles back, with no engine callback on the hot path and implicit cancellation (a tile that leaves the view simply drops out of the set). The poll-loop API — `wanted_keys()` / `wantedKeys()` and `set_chunk_data_u16()` / `setChunkDataU16()` — is covered in [Python Bindings](./12-python.md) and [JavaScript Bindings](./13-wasm.md).

## LOD Selection and Fallback

A volume is described as an array of `LevelMetadata` (one per resolution level, level 0 finest; see the Bindings pages for the constructor). LOD selection is screen-space: Bovista picks the finest level whose voxel-to-pixel ratio at the current zoom is near 1:1.

The page table records the LOD each tile was loaded at, so when a tile is absent at the ideal LOD the fragment shader climbs to the coarsest available ancestor:

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

Coarse tiles fill in immediately while finer ones stream, giving smooth progressive refinement.

## Memory Budget

`max_tiles` at construction sets the atlas size, and therefore VRAM use. Each tile is `tile_d × tile_h × tile_w × 2` bytes (R16Float) — a 64³ tile is 524 KB, so ~2000 tiles ≈ 1 GB:

```python
image = bv.Image(viewer, levels, max_tiles=2000)
```

The web examples convert a GB budget into a tile count (`floor(budgetBytes * 0.8 / tileVram)`).

## Debug Modes

Both `Image` and the volume visuals support debug visualizations:

- **LOD tint** (`set_debug_mode(true)`) — colors tiles by loaded LOD (green = finest, red = coarsest).
- **Atlas debug** (`set_atlas_debug_mode(true)`, volume only) — bypasses the page table to sample the raw atlas, showing tile packing.
- **Step count heatmap** (`set_step_debug_mode(true)`, volume only) — colors pixels by ray-march step count.

## Summary

| Component | Role |
|-----------|------|
| `VirtualTextureData` | Orchestrates streaming, atlas, page table, LOD selection |
| `AtlasAllocator` | Slot assignment and LRU eviction in the 3D atlas texture |
| `PageTable` | GPU indirection: maps `(lod, t, z, y, x)` → atlas slot |
| `PendingChunks` | Lock-protected queue for tile data pushed from background threads |
| Wanted set | Engine-published set of desired tiles; the loader polls it via `wanted_keys()` |
| `set_chunk_data_u16()` | Pushes arrived tile data into `PendingChunks` |

## Limits

Because tiles stream on demand, dataset size is bounded by addressing, not VRAM. The ceilings and where they come from:

| Limit | Value | Source |
|-------|-------|--------|
| Voxels per axis | 2³² ≈ 4.3 billion | `volume_size` is `u32` per axis |
| Tiles in the finest LOD | ≤ `max_texture_dimension_2d²` (~67M in-browser, ~268M desktop) | the page table is a single 2D texture; exceeding it fails at texture creation |
| Resident tiles (working set) | 4 atlases × 65,536 = 262,144 | page-table texel packs a 16-bit slot index + 2-bit atlas id |
| Timepoints (4D) | 8,192 | 13-bit `t` field in the page-table texel |
| LOD levels | 16 | `VT_MAX_LODS` |

With the example loaders' 64³ tiles, the finest-LOD cap works out to roughly a 26,000³ volume in the browser and ~41,000³ on desktop GPUs (~18–70 teravoxels).

Two practical conditions matter more than the raw ceilings:

- **Multiscale pyramid.** bovista renders large volumes by streaming coarse LODs and refining toward the view. Without coarse levels it's forced onto the full-resolution grid, where the resident-tile budget runs out almost immediately. The "handles a ton of data" story is really "handles a ton of *well-pyramided* data."
- **Tile/chunk size.** Bigger tiles raise the per-image ceiling (fewer tiles for the same voxels) but shrink the resident slot count (bigger atlas slots) — and the atlas is itself a 3D texture bounded by `max_texture_dimension_3d`, which is why up to four atlases exist.
