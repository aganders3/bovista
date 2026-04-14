# Further Reading

## Official Documentation

### WGPU

- **wgpu.rs**: [https://wgpu.rs](https://wgpu.rs)
- **API Documentation**: [https://docs.rs/wgpu](https://docs.rs/wgpu)
- **Examples**: [https://github.com/gfx-rs/wgpu/tree/trunk/examples](https://github.com/gfx-rs/wgpu/tree/trunk/examples)

### WebGPU Specification

- **W3C Spec**: [https://www.w3.org/TR/webgpu/](https://www.w3.org/TR/webgpu/)
- **WGSL Spec**: [https://www.w3.org/TR/WGSL/](https://www.w3.org/TR/WGSL/)

## Tutorials & Guides

### Learn WGPU

- **Learn WGPU**: [https://sotrh.github.io/learn-wgpu/](https://sotrh.github.io/learn-wgpu/)
  - Excellent beginner tutorial series
  - Covers basics through advanced topics
  - Rust-focused

### WebGPU Fundamentals

- **WebGPU Fundamentals**: [https://webgpufundamentals.org/](https://webgpufundamentals.org/)
  - JavaScript-focused but concepts apply to WGPU
  - Great visual explanations

## Related Graphics APIs

### Vulkan

- **Vulkan Tutorial**: [https://vulkan-tutorial.com/](https://vulkan-tutorial.com/)
  - WGPU is based on Vulkan concepts
  - Understanding Vulkan helps with WGPU

### Metal

- **Metal Programming Guide**: [https://developer.apple.com/metal/](https://developer.apple.com/metal/)
  - Apple's GPU API
  - Similar concepts to WGPU

## Computer Graphics

### Books

- **Real-Time Rendering** (4th Edition) by Tomas Akenine-Möller et al.
  - Comprehensive graphics reference
  - Modern rendering techniques

- **Physically Based Rendering** by Matt Pharr et al.
  - Deep dive into rendering theory
  - Ray tracing focus but applicable concepts

### Online Resources

- **Scratchapixel**: [https://www.scratchapixel.com/](https://www.scratchapixel.com/)
  - Graphics fundamentals explained clearly
  - Math, rendering, ray tracing

- **Learn OpenGL**: [https://learnopengl.com/](https://learnopengl.com/)
  - OpenGL tutorial, but concepts transfer
  - Good for graphics basics

## Rust-Specific

### Bevy Game Engine

- **Bevy**: [https://bevyengine.org/](https://bevyengine.org/)
  - Uses WGPU for rendering
  - Good example of large-scale WGPU usage

### Nannou Creative Coding

- **Nannou**: [https://nannou.cc/](https://nannou.cc/)
  - Creative coding framework using WGPU
  - Many examples

## Scientific Visualization

### Zarr

- **Zarr Python**: [https://zarr.readthedocs.io/](https://zarr.readthedocs.io/)
  - Chunked array storage format
  - Used in Bovista for remote data

### OME-Zarr

- **OME-Zarr Spec**: [https://ngff.openmicroscopy.org/](https://ngff.openmicroscopy.org/)
  - Microscopy image format
  - Multi-resolution volumes

## GPU Programming

### Compute Shaders

- **Compute Shader Guide**: [https://github.com/googlefonts/compute-shader-101](https://github.com/googlefonts/compute-shader-101)
  - WGPU supports compute shaders
  - Parallel processing on GPU

### GPU Performance

- **GPU Gems**: [https://developer.nvidia.com/gpugems](https://developer.nvidia.com/gpugems)
  - Classic GPU programming techniques
  - Older but foundational

## Tools

### Debugging

- **RenderDoc**: [https://renderdoc.org/](https://renderdoc.org/)
  - Capture and inspect GPU frames
  - Works with WGPU

- **PIX** (Windows): [https://devblogs.microsoft.com/pix/](https://devblogs.microsoft.com/pix/)
  - Microsoft's GPU debugger

- **Xcode GPU Debugger** (macOS)
  - Built into Xcode
  - Metal debugging

### Profiling

- **Tracy Profiler**: [https://github.com/wolfpld/tracy](https://github.com/wolfpld/tracy)
  - Real-time profiler with GPU support

## Community

### Discord

- **WGPU Discord**: [https://discord.gg/wgpu](https://discord.gg/wgpu)
  - Active community
  - Get help with WGPU

- **Rust Gamedev**: [https://discord.gg/yNtPTb2](https://discord.gg/yNtPTb2)
  - General Rust game development

### Forums

- **WGPU Matrix**: [https://matrix.to/#/#wgpu:matrix.org](https://matrix.to/#/#wgpu:matrix.org)
  - Alternative to Discord

## Papers & Research

### Chunked Rendering

- **Efficient Rendering of Large-Scale Volume Data**
  - Various papers on LOD, out-of-core rendering
  - Search Google Scholar for "chunked volume rendering"

### Spatial Data Structures

- **Octrees, K-d Trees, BVH**
  - Used for spatial partitioning
  - Relevant for chunk visibility culling

## Related Projects

### Volume Rendering

- **Neuroglancer**: [https://github.com/google/neuroglancer](https://github.com/google/neuroglancer)
  - WebGL-based volume viewer
  - Similar goals to Bovista

### Point Cloud Visualization

- **Potree**: [https://github.com/potree/potree](https://github.com/potree/potree)
  - WebGL point cloud renderer
  - LOD techniques

## Keep Learning

The graphics field is constantly evolving. Stay current by:

1. **Following graphics researchers** on Twitter/Mastodon
2. **Reading SIGGRAPH papers** (annual conference)
3. **Experimenting** with new techniques in WGPU
4. **Contributing** to open-source graphics projects

---

**Congratulations on completing this guide!**

You now understand the fundamentals of WGPU and how it compares to OpenGL. Keep practicing, experiment with the Bovista codebase, and build your own visualizations!
