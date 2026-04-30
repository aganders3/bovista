# Further Reading

## WGPU

- **wgpu.rs**: [https://wgpu.rs](https://wgpu.rs) — API reference and overview
- **Learn WGPU**: [https://sotrh.github.io/learn-wgpu/](https://sotrh.github.io/learn-wgpu/) — step-by-step Rust tutorial
- **WebGPU Fundamentals**: [https://webgpufundamentals.org/](https://webgpufundamentals.org/) — conceptual explanations (JS-focused but transferable)

## Zarr & OME-Zarr

- **Zarr Python**: [https://zarr.readthedocs.io/](https://zarr.readthedocs.io/) — chunked array format used by Bovista's examples
- **OME-Zarr Spec**: [https://ngff.openmicroscopy.org/](https://ngff.openmicroscopy.org/) — multiscale microscopy image format
- **Zarrita (JS)**: [https://github.com/manzt/zarrita.js](https://github.com/manzt/zarrita.js) — Zarr v2/v3 reader used by the web examples

## Volume Rendering

- **Neuroglancer**: [https://github.com/google/neuroglancer](https://github.com/google/neuroglancer) — WebGL-based volume viewer with similar streaming goals
- **Real-Time Volume Graphics** (book) — foundational DVR techniques (ray marching, compositing, transfer functions)

## Debugging GPU Code

- **RenderDoc**: [https://renderdoc.org/](https://renderdoc.org/) — frame capture and GPU state inspector; works with wgpu on native targets
- **Xcode GPU Debugger** (macOS) — built into Xcode; useful for Metal frames
