# Overview & Design Philosophy

Bovista is designed around three core principles: **handle massive datasets**, **work everywhere**, and **be fast**. This chapter explains the design decisions that make this possible.

## The Problem: Massive Volumetric Data

Modern microscopy and medical imaging produce enormous datasets:

- **Light-sheet microscopy**: 32,000 × 32,000 × 32,000 voxels (~32 TB uncompressed)
- **Whole-brain imaging**: 50,000 × 40,000 × 10,000 voxels (~20 TB)
- **4D time-series**: Multiple 3D volumes over time (100+ TB)

**Traditional approaches don't work:**

❌ **Load entire volume**: 32 TB won't fit in 64 GB RAM
❌ **Downsample**: Lose critical detail
❌ **Pre-slice**: Can't explore interactively
❌ **Server-side rendering**: High latency, expensive infrastructure

## Bovista's Solution: Virtual Texture Streaming

Instead, Bovista:

✅ **Divides volumes into tiles** at multiple LOD resolutions
✅ **Loads only visible tiles** (typically 10-50 resident out of millions virtual)
✅ **Streams from remote storage** (S3, Zarr, HTTP)
✅ **Renders on the client** (Python desktop or web browser) — slice or full DVR
✅ **Manages VRAM automatically** (atlas LRU eviction when tile budget is exceeded)

**Result**: Interactive slice or volumetric rendering of terabyte-scale data with a fixed VRAM footprint (~500 MB - 1 GB for a typical atlas).

## Design Decision: Why WGPU?

Bovista needs to handle highly variable chunk sizes and data layouts. Why WGPU instead of OpenGL/WebGL?

### Cross-Platform Requirements

Bovista needs to work in:
- **Python desktop apps** (Windows, macOS, Linux)
- **Web browsers** (via WebAssembly)
- **Jupyter notebooks** (future goal)

**OpenGL**: Would require separate OpenGL (desktop) and WebGL (browser) implementations
**WGPU**: Single codebase targets native (Vulkan/Metal/DX12) and web (WebGPU)

### Type Safety & Memory Safety

Volumetric visualization involves complex state:
- Dynamic geometry generation
- Async chunk loading
- Resource lifecycle management

**OpenGL**: Runtime errors, manual resource tracking, easy to leak memory
**WGPU + Rust**: Compile-time validation, automatic resource cleanup, ownership prevents leaks

### Performance for Streaming Rendering

Bovista's virtual texture streaming requires:
- Uploading tile data into the atlas texture each frame (`queue.write_texture`)
- Dynamic vertex buffer updates for slice geometry each frame
- Single draw call per visual — no per-tile state changes

**WGPU advantages:**
- ✅ **Explicit resource lifecycle**: Precise control over when the atlas and page-table memory are allocated and freed
- ✅ **Bind groups**: Atlas texture, page table, and camera uniforms bundled once; no per-tile rebinding
- ✅ **Efficient pipelines**: Pre-compiled; both slice and ray-march paths use one pipeline per visual
- ✅ **Queue**: `write_texture` / `write_buffer` allow background threads to push tile data without blocking the render loop

## Architecture Layers

Bovista is organized into clear layers:

```
┌─────────────────────────────────────────────────────┐
│  Application Layer (Python/JS)                      │
│  - Load data from Zarr/S3                           │
│  - Handle UI events                                 │
│  - Call Bovista API                                 │
└─────────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────┐
│  Bindings Layer (PyO3 / wasm-bindgen)               │
│  - Type conversion (NumPy ↔ Rust)                   │
│  - Callback handling                                │
│  - Platform abstractions                            │
└─────────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────┐
│  Visualization Layer (Bovista Core)                 │
│  - Renderer: GPU management                         │
│  - Scene: Visual organization                       │
│  - Visuals: Points, Lines, ImageVisual, VolumeVisual │
│  - Camera: Orbit controls                           │
└─────────────────────────────────────────────────────┘
                      │
                      ▼
┌─────────────────────────────────────────────────────┐
│  GPU Layer (WGPU)                                   │
│  - Resource management (buffers, textures)          │
│  - Pipeline compilation                             │
│  - Command encoding                                 │
│  - Native/Web abstraction                           │
└─────────────────────────────────────────────────────┘
```

### Separation of Concerns

**Application Layer** doesn't know about:
- GPU resources
- Shader compilation
- Vertex layouts

**Visualization Layer** doesn't know about:
- Zarr file format
- Network protocols
- Platform-specific UI

**GPU Layer** doesn't know about:
- Volume chunking
- Data loading
- Coordinate systems

**Benefit**: Each layer can be developed and tested independently.

## Key Design Patterns

### 1. The Visual Trait

All renderable objects implement the `Visual` trait:

```rust
pub trait Visual: Send + Sync {
    fn prepare(&mut self, device: &Device, queue: &Queue);
    fn render(&self, render_pass: &mut RenderPass);
    fn set_transform(&mut self, transform: Transform);
    // ...
}
```

**Why this works:**
- **Polymorphism**: Scene renders any `Visual` without knowing its type
- **Encapsulation**: Each visual manages its own GPU resources
- **Flexibility**: Easy to add new visual types

### 2. Callback-Based Loading

Data loading uses callbacks, not inheritance. The loader is called by the rendering engine each frame for tiles it needs; it returns a `ChunkStatus` immediately and pushes data back asynchronously via `set_chunk_data_u16()`:

```python
def request_tile(lod, z, y, x):
    key = (lod, z, y, x)
    if key in pending:
        return bv.ChunkStatus.AlreadyPending
    pending.add(key)
    executor.submit(load_tile, key)
    return bv.ChunkStatus.Accepted

volume = bv.Volume(viewer, levels, max_tiles=2000, loader=request_tile)
```

**Why this works:**
- **Flexibility**: Any data source (Zarr, HDF5, HTTP, custom)
- **No subclassing**: Simple function, not class hierarchy
- **Non-blocking**: Rendering never stalls waiting for data

### 3. Shared Bind Group Pattern

All visuals share the camera bind group:

```rust
// Renderer sets once per frame
render_pass.set_bind_group(0, &camera_bind_group, &[]);

// Each visual provides its own resources
impl Visual for MyVisual {
    fn render(&self, render_pass: &mut RenderPass) {
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.draw(...);
    }
}
```

**Benefit**: Camera updated once, all visuals use it automatically.

### 4. Platform Abstractions

Rust code uses conditional compilation:

```rust
#[cfg(not(target_arch = "wasm32"))]
pub type VisualRef = Arc<Mutex<dyn Visual>>;  // Native: thread-safe

#[cfg(target_arch = "wasm32")]
pub type VisualRef = Rc<RefCell<dyn Visual>>;  // WASM: single-threaded
```

**Benefit**: One codebase, optimal for each platform.

## Trade-offs & Limitations

### What Bovista Is Good At

✅ **Slice rendering**: Real-time, arbitrary orientations (`ImageVisual`)
✅ **Volume ray marching**: Direct volume rendering with transfer functions (`VolumeVisual`)
✅ **Remote data**: Efficient streaming from S3/Zarr via virtual textures
✅ **Large volumes**: Terabyte-scale with atlas LRU eviction
✅ **Cross-platform**: Native + web from one codebase

### What Bovista Is Not Optimized For

❌ **Real-time editing**: Read-only visualization
❌ **Small volumes**: Virtual texture overhead unnecessary for <1 GB
❌ **Non-volumetric data**: Meshes, graphs need custom visuals

### Design Choices

**Virtual textures (atlas + page table) vs per-tile draw calls:**
- Chose virtual textures: Single draw call regardless of tile count; shader resolves indirection
- Per-tile draws: Simpler shader but O(n) state changes per frame

**Slice rendering vs ray marching:**
- Bovista supports both; `ImageVisual` uses slice geometry for speed, `VolumeVisual` uses ray marching for full semi-transparent DVR

**LRU atlas eviction vs fixed budget:**
- Chose LRU: Automatic, adapts to view changes
- Fixed budget: More predictable but less flexible

**Callback-based loading vs async Rust:**
- Chose callbacks: Simpler Python/WASM integration; any async runtime can back it
- Async: More Rust-idiomatic but complex cross-language interop

## Summary

Bovista's architecture is driven by its goal: **write the core visualization once in Rust, run it anywhere**.

**Key decisions:**
1. **WGPU**: Cross-platform, type-safe, modern GPU API (Vulkan/Metal/DX12 + WebGPU)
2. **Virtual textures**: Atlas + page table; single draw call; only visible tiles resident
3. **Visual trait**: Polymorphic rendering system (slice, ray march, points, custom)
4. **Callback loaders**: Flexible data sources (thread pool, JS fetch, or anything else)
5. **LRU atlas eviction**: Automatic memory management regardless of dataset size

**Result**: View terabyte-scale volumes in Python or a web browser from a single Rust codebase.

---

**Next**: [Visual System →](./04-visuals.md)
