<figure>
  <img src="./images/golden_puffball_banner.jpg" alt="Golden puffballs (Bovista colorata) on a pine-needle forest floor" style="width:100%;border-radius:6px">
  <figcaption style="font-size:0.85em;color:#888;margin-top:0.4em">
    <em>Golden Puffball</em> (<em>Bovista colorata</em>) by
    <a href="https://www.flickr.com/photos/7147684@N03/1034884099">Jason Hollinger</a> —
    cropped from the <a href="https://commons.wikimedia.org/wiki/File:Golden_Puffball.jpg">original</a>,
    licensed under <a href="https://creativecommons.org/licenses/by/2.0/">CC BY 2.0</a>.
  </figcaption>
</figure>

# Introduction

Bovista is a visualization library for large 3D imaging datasets. The thesis: write the rendering engine once in Rust and run it anywhere — as a Python extension, a WebAssembly module in the browser, or a native binary.

It provides:

- **Slice rendering** — arbitrary-orientation plane intersections through 3D volumes, via 3D texture sampling
- **Volume rendering** — different modes (`DirectVolume` / `MipVolume` / `MinipVolume` / `AverageVolume` / `IsosurfaceVolume`) via GPU ray casting
- **Virtual texture streaming** — keeps only the visible tiles resident in VRAM, used in both slice
  and volume rendering
- **Multi-resolution LOD** — coarse-to-fine pyramid; LOD selection by screen-space voxel error
- **Remote OME-Zarr** — pull-based loading; Zarr chunking maps directly to atlas tiles
- **Points, lines, custom visuals** — simple annotations and visuals with user-defined shaders


## Key Concepts

### One Codebase, Multiple Targets

The core library has no Python or browser dependencies. Target-specific code lives in `src/python.rs` and `src/wasm.rs`, which are intended to be thin wrappers.

### Pull-Based Loading

Loading is decoupled from rendering. Bovista visuals publish a **wanted** set — the tiles it needs, sorted by priority. The app polls that set and pushes arrived tiles back:

```python
# poll what's wanted, then load and push back.
for lod, t, z, y, x, priority in volume.wanted_keys():
    # priority: lower = more urgent, 0 = currently viewed
    data = load_tile(lod, t, z, y, x)          # however you like
    volume.set_chunk_data_u16(lod, t, z, y, x, data)
```

`wanted_keys()` returns `(lod, t, z, y, x, priority)` tuples sorted by priority — tile keys carry a timepoint `t`. Load them however you like (thread pools, `asyncio`, JS `fetch()`) and push results back via `set_chunk_data_u16()`, which queues the GPU upload for the next `prepare()`. Cancellation is implicit: tiles no longer needed simply drop out of the `wanted` set.

In WASM the same set is `volume.wantedKeys()`, returning a flat `Uint32Array` of `[lod, t, z, y, x, priority, ...]` (6 ints per key); tiles go back via `volume.setChunkDataU16(lod, t, z, y, x, u16, zShape, yShape, xShape)`.

This relies an assumption of TCZYX dimensions, though only single-channel data is currently supported.

### Virtual Texture Streaming

The `Image` and volume visuals share one streaming back-end. The volume is divided into uniform tiles. A 3D texture **atlas** holds the resident tiles; a **page table** (a small 2D-array texture indexed by LOD + tile grid coordinates) maps each virtual tile address to its atlas slot. The fragment shader looks up the page table at runtime; if a tile is missing it falls back to the nearest loaded coarser LOD. The entire volume renders in a **single draw call** — no per-tile state changes.

Because only the visible tiles are ever resident, the addressable dataset is bounded by indexing rather than memory: up to ~4 billion voxels per axis and many teravoxels overall — ballpark a ~26,000³ volume in the browser and larger on desktop GPUs. This assumes the dataset ships a proper multiscale pyramid and sane chunking — bovista renders large volumes by streaming coarse LODs, so the coarse levels are what make it tractable. In practice you'll hit your storage and network long before bovista's limits; see [Virtual Textures](./01-philosophy.md#limits) for the specifics.
