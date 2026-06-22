# Bovista Architecture Guide

A comprehensive guide to understanding Bovista's GPU-accelerated volumetric visualization architecture, built with WGPU for rendering massive remote datasets.

> **📝 NOTE:** This guide has been updated to reflect the recent `Image` unification.
> Code examples using old components have been updated or marked with notes where needed.
> The architecture now uses a unified Strategy pattern with `Image`, `TileStrategy`,
> and `tile.wgsl` shader. See repository's NEXT_STEPS.md for implementation details.

## About This Guide

This guide explains **how Bovista works** - from high-level architecture to implementation details. It's designed for:

- **Contributors** wanting to understand the codebase
- **Developers** implementing custom visuals or features
- **Researchers** exploring Bovista's design for their own projects
- **Graphics programmers** learning modern GPU programming with WGPU

## Building the Book

### Prerequisites

Install mdbook:

```bash
cargo install mdbook
```

### Build and Serve

From this directory (`docs/guide`):

```bash
# Build the book
./build.sh

# Or use mdbook directly
mdbook build

# Serve locally with live reload
mdbook serve

# Then open http://localhost:3000
```

The built book will be in the `book/` directory.

## What's Covered

### Bovista Architecture (Chapters 1-4)

Understand Bovista's design and core components:

1. **Overview & Design Philosophy** - Why Bovista exists, key design decisions
2. **Core Components** - Renderer, Scene, Camera, Visual trait
3. **Renderer & Scene Management** - GPU initialization, render loop
4. **Visual System** - Points, Lines, Images, and custom visuals

### Volumetric Rendering (Chapters 5-8)

The heart of Bovista - handling massive 3D volumes:

5. **3D Textures & Sampling** - Volume texture management
6. **Dynamic Slice Geometry** - Plane-AABB intersection algorithm
7. **Chunked Rendering System** - LRU caching for terabyte-scale data
8. **Remote Data Loading** - Zarr, S3, and async loading patterns

### WGPU Implementation (Chapters 9-11)

How Bovista uses WGPU effectively:

9. **Pipelines & Shaders** - Render pipelines and WGSL shaders
10. **Bind Groups & Resources** - Resource organization and binding
11. **Render Loop & Command Buffers** - Frame rendering workflow

### Platform Support (Chapters 12-13)

Cross-platform deployment:

12. **Python Bindings** - PyO3, NumPy integration
13. **WebAssembly & Browser** - wasm-bindgen, JavaScript interop

### Reference

- **Code Reference** - Quick reference for common tasks
- **WGPU vs OpenGL** - Comparison for OpenGL developers
- **Further Reading** - Resources and related projects

## Key Features Explained

### Chunked Remote Rendering

Learn how Bovista enables viewing 32,000³ voxel volumes (~32 TB) with only ~10 GB RAM by:
- Dividing volumes into chunks
- Loading only visible chunks
- Streaming from S3/Zarr
- LRU cache eviction

### Dynamic Geometry Generation

Understand how each frame, Bovista:
- Computes plane-AABB intersections
- Generates triangle fans for slice polygons
- Updates GPU buffers dynamically

### Cross-Platform Support

See how one Rust codebase targets:
- Native (Windows/macOS/Linux via Vulkan/Metal/DX12)
- Web browsers (via WebGPU/WebAssembly)
- Python (PyO3 bindings)

## Project Context

This guide documents the [Bovista library](https://github.com/yourusername/bovista), a GPU-accelerated visualization tool for massive volumetric datasets.

**Repository structure:**
```
bovista/
├── src/              # Rust library code
├── examples/         # Example applications
├── web/              # Web UI components
└── docs/
    └── guide/        # This architecture guide
```

## Contributing

Found an error or want to improve the guide? Please open an issue or pull request in the main Bovista repository.

## License

This guide is part of the Bovista project and follows the same license.
