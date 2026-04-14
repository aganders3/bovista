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

### Performance for Chunked Rendering

Bovista's chunked rendering requires:
- Frequently creating/destroying textures (chunk loading/eviction)
- Dynamic vertex buffer updates (slice geometry)
- Efficient state changes (switching between chunks)

**WGPU advantages:**
- ✅ **Explicit resource lifecycle**: Know exactly when GPU memory is freed
- ✅ **Command buffers**: Record rendering commands on background threads
- ✅ **Efficient pipelines**: Pre-compiled, fast to switch
- ✅ **Bind groups**: Bundle resources, minimize state changes

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
│  - Visuals: Points, Lines, Images, TiledImage       │
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

Data loading uses callbacks, not inheritance:

```rust
pub type ChunkLoaderFn = Arc<dyn Fn(ChunkRequest) -> Option<ChunkData> + Send + Sync>;

pub struct TiledImageVisual {
    loader: ChunkLoaderFn,
    // ...
}
```

**Python:**
```python
def load_chunk(z, y, x):
    return zarr_array[z*cz:(z+1)*cz, y*cy:(y+1)*cy, x*cx:(x+1)*cx]

tiled = bv.TiledImage.from_loader(viewer, volume_size, chunk_size, load_chunk)
```

**Why this works:**
- **Flexibility**: Any data source (Zarr, HDF5, HTTP, custom)
- **No subclassing**: Simple function, not class hierarchy
- **Testability**: Mock loaders for testing

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

## Performance Considerations

### Memory Management

**Chunk cache** (`TiledImageVisual`):
- Target chunk size: **~1 MB** (sweet spot for network + memory)
- Max loaded chunks: 150-1000 (configurable)
- Chunk size: **Matches Zarr chunking** (optimized when data is written)
- **LRU eviction**: Automatic when limit exceeded

**Real-world examples:**

```
Thin Z-slices (common in microscopy):
  1 × 275 × 271 = 74,525 bytes = ~73 KB per chunk
  → Very small, many needed per slice
  → 500 chunks = 36.6 MB total (very efficient!)

Target size (~1 MB, typical for volumetric data):
  128 × 128 × 64 = 1,048,576 bytes = 1 MB per chunk
  → Good balance of network speed and spatial coverage
  → 500 chunks = 500 MB total

Anisotropic (~1 MB, thin in Z):
  16 × 256 × 256 = 1,048,576 bytes = 1 MB per chunk
  → Matches anisotropic acquisition (thin Z, wide XY)
  → 500 chunks = 500 MB total

Large chunks (when spatial coverage is important):
  256 × 256 × 64 = 4,194,304 bytes = ~4 MB per chunk
  → Fewer chunks needed to cover area
  → 150 chunks = 600 MB total
```

**Why ~1 MB chunks?**
- ✅ Fast network loading (~100-200 ms from S3)
- ✅ Reasonable memory usage (500 chunks = 500 MB)
- ✅ Good spatial coverage
- ✅ Efficient GPU uploads

**GPU resources scale with chunk size:**
- 3D texture: Equals chunk data size (~1 MB typical)
- Vertex buffer: ~2 KB (constant, max 8 vertices)
- Index buffer: ~100 bytes (constant)
- Uniforms: ~128 bytes (constant)

**Memory is dominated by texture size** - with 1 MB chunks, even 500 loaded chunks = only 500 MB!

### Rendering Performance

**Typical workload**:
- 15 visible chunks
- 6-8 triangles per chunk
- **Total**: ~100 triangles per frame

**GPU utilization**: ~1% (geometry-bound, not fill-rate bound)

**Bottleneck**: Network bandwidth for chunk loading, not rendering.

**Target**: 60 FPS on integrated GPUs (easily achieved).

### Async Loading Strategy

**Challenge**: Loading chunks takes 500-2000 ms (network), but frames need 16 ms (60 FPS).

**Solution**:
1. **Non-blocking loads**: Loader returns `None` if not ready
2. **Background fetching**: Python thread pool or browser fetch API
3. **Progressive display**: Show loaded chunks immediately
4. **Prefetching**: Load neighboring chunks in background

**Result**: Smooth 60 FPS even during loading.

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

**Next**: [Core Components →](./02-components.md)
