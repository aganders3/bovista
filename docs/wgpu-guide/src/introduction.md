# Introduction

Welcome to the **Bovista Architecture Guide**. This guide explains how Bovista works under the hood: how it renders massive volumetric datasets interactively, how the virtual texture system streams tiles on demand, and how the same Rust core runs in Python and in a browser.

## What is Bovista?

Bovista is a visualization library for large 3D imaging datasets. Its thesis is that the core rendering engine should be written once in Rust and run anywhere — as a Python extension, as a WebAssembly module in the browser, or as a native binary.

Concretely, it provides:

- **Slice rendering** — arbitrary-orientation plane intersections through 3D volumes, rendered via 3D texture sampling
- **Volume ray marching** — direct volume rendering (DVR) with per-pixel ray casting and transfer function compositing
- **Virtual texture streaming** — an atlas + page table architecture that keeps only the visible tiles resident in VRAM, regardless of dataset size
- **Multi-resolution LOD** — coarse-to-fine pyramid; LOD selection is based on screen-space voxel error
- **Remote OME-Zarr** — tiles are fetched from S3/HTTP by a user-supplied async callback; Zarr chunking maps directly to atlas tiles
- **Points, lines, custom visuals** — for annotations and user-defined shaders

### Real-World Use Cases

- **Microscopy**: Exploring 32,000³ voxel images from light-sheet or electron microscopes
- **Medical Imaging**: Navigating CT/MRI volumes without loading the full dataset
- **Scientific Visualization**: Analyzing simulation data stored in remote Zarr stores
- **Browser Sharing**: Viewing multi-gigabyte datasets directly in Chrome or Safari without installing software

## Architecture at a Glance

```
┌─────────────────────────────────────────────────────────┐
│                     Bovista Library                      │
├─────────────────────────────────────────────────────────┤
│                                                          │
│  Bindings                                                │
│  ├─ Python (PyO3 — Viewer, Image, Volume, Lines, ...)   │
│  └─ WebAssembly (wasm-bindgen — JsViewer, ...)          │
│                                                          │
│  Rendering Engine                                        │
│  ├─ Renderer: wgpu device/queue, render pass            │
│  ├─ Scene: visual list, per-frame prepare + render       │
│  └─ Camera: orbit controls, projection, frustum          │
│                                                          │
│  Visual Types                                            │
│  ├─ ImageVisual: slice-plane rendering                  │
│  ├─ VolumeVisual: ray marching DVR                      │
│  ├─ PointsVisual / LinesVisual                          │
│  └─ CustomVisual: user-defined WGSL shaders             │
│                                                          │
│  Virtual Texture System (shared by Image + Volume)       │
│  ├─ VirtualTextureData: LOD selection, tile requests    │
│  ├─ AtlasAllocator: 3D texture atlas (LRU eviction)     │
│  └─ PageTable: 2D-array indirection texture             │
│                                                          │
└─────────────────────────────────────────────────────────┘
                          │
                          ▼
┌─────────────────────────────────────────────────────────┐
│                    wgpu-rs                               │
├─────────────────────────────────────────────────────────┤
│  Cross-platform GPU abstraction                         │
│  ├─ Native: Vulkan, Metal, DirectX 12                  │
│  └─ Web: WebGPU                                         │
└─────────────────────────────────────────────────────────┘
```

## Key Concepts

### Virtual Texture Streaming

Both `ImageVisual` and `VolumeVisual` share the same streaming back-end. The volume is divided into uniformly-sized tiles. A 3D texture **atlas** holds the physically resident tiles. A **page table** (a small 2D-array texture indexed by LOD + tile grid coordinates) maps each virtual tile address to its atlas slot. The fragment shader looks up the page table at runtime to find each tile's atlas location; if a tile is missing it falls back to the nearest loaded coarser LOD.

This means the entire volume can be rendered in a **single draw call** — no per-tile state changes.

### Callback-Based Loading

Data loading is decoupled from rendering via a callback:

```python
def request(lod, z, y, x):
    # Submit async load; call volume.set_chunk_data_u16() when done
    return bv.ChunkStatus.Accepted
```

The rendering engine calls `request()` each frame for tiles it needs. The implementation can use Python thread pools, `asyncio`, JS `fetch()`, or any other mechanism. Arrived data is pushed back via `set_chunk_data_u16()`, which queues it for upload on the next `prepare()` pass.

### One Codebase, Multiple Targets

The core library has no Python or browser dependencies. Target-specific code lives in `src/python.rs` and `src/wasm.rs`, which are thin wrappers around `src/bindings_common.rs`. Conditional compilation (`#[cfg(feature = "python")]`, `#[cfg(target_arch = "wasm32")]`) keeps them isolated.

## Project Structure

```
src/
  lib.rs                    — crate root, public API
  renderer.rs               — wgpu device/queue, render pass
  camera.rs                 — orbit controls, frustum, projection
  scene.rs                  — visual list
  visual.rs                 — Visual trait
  bindings_common.rs        — shared Python/WASM logic
  visuals/
    image.rs                — ImageVisual (slice rendering)
    volume.rs               — VolumeVisual (ray marching)
    virtual_texture.rs      — VirtualTextureData: atlas, page table, LOD
    atlas.rs                — AtlasAllocator
    page_table.rs           — PageTable
    tile.rs                 — TileKey, TileData, uniforms
    points.rs               — PointsVisual
    lines.rs                — LinesVisual
    custom.rs               — CustomVisual
  shaders/
    virtual_tile.wgsl       — slice shader
    volume_raymarch.wgsl    — ray marching shader
    point_cloud.wgsl
    lines.wgsl
  python.rs                 — PyO3 bindings
  wasm.rs                   — wasm-bindgen bindings
examples/
  slice_viewer/
    python/                 — PyQt6 + ThreadPoolExecutor
    web/                    — browser + WebAssembly
    rust/                   — stub
  volume_renderer/
    python/                 — PyQt6 + ThreadPoolExecutor
    web/                    — browser + WebAssembly
    rust/                   — stub
  pkg/                      — WASM build output (generated, not committed)
kiln-render/                — standalone WebGPU browser DVR renderer (TypeScript)
```

## Example: Volume Ray Marching

```python
import bovista as bv
import zarr
import numpy as np
from concurrent.futures import ThreadPoolExecutor
from threading import Lock

store = zarr.open_group("https://s3.../dataset.ome.zarr", mode="r")

# Build LevelMetadata for each LOD in the OME-Zarr pyramid
lod_levels = [
    bv.LevelMetadata(
        volume_size=(depth, height, width),
        chunk_size=(cz, cy, cx),
        voxel_size=(sz, sy, sx),
        scale_factor=2.0**i,
        translation=(0.0, 0.0, 0.0),
    )
    for i, (depth, height, width, cz, cy, cx, sz, sy, sx) in enumerate(...)
]

executor = ThreadPoolExecutor(max_workers=8)
pending = {}
lock = Lock()

def request(lod, z, y, x):
    key = (lod, z, y, x)
    with lock:
        if key in pending:
            return bv.ChunkStatus.AlreadyPending
        future = executor.submit(load_tile, lod, z, y, x)
        future.add_done_callback(lambda f: volume.set_chunk_data_u16(lod, z, y, x, f.result()))
        pending[key] = future
    return bv.ChunkStatus.Accepted

viewer = bv.Viewer(800, 600)
viewer.initialize_with_window(window_handle, 800, 600)

volume = bv.Volume(viewer, lod_levels, max_tiles=2000, loader=request)
volume.set_contrast(0.0, 1.0)
volume.set_density_scale(0.01)
viewer.add(volume)

# In render loop:
viewer.render_frame()
```

---

**Start here**: [Python API →](./12-python.md) · [WebAssembly / Browser →](./13-wasm.md) · [Architecture →](./01-philosophy.md)
