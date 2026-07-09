//! # Bovista
//!
//! A high-performance 3D visualization library built on WebGPU for rendering large-scale
//! volumetric imaging data with on-demand chunked loading.
//!
//! ## Architecture Overview
//!
//! Bovista is designed around a scene graph architecture with the following key components:
//!
//! ### Core Components
//!
//! - **[`Renderer`]**: Manages GPU resources, render pipelines, and coordinates frame rendering
//! - **[`Scene`]**: Container for visual objects that implements visibility culling and rendering order
//! - **[`Camera`]**: Perspective camera with orbit, zoom, and projection controls
//! - **[`Visual`]**: Base trait for all renderable objects in the scene
//!
//! ### Visual Types
//!
//! Bovista provides several built-in visual types for different use cases:
//!
//! - **[`Points`]**: Render point clouds with per-point colors
//! - **[`Lines`]**: Render line segments and wireframes
//! - **[`Image`]**: Slice-plane rendering through a 3D volume (arbitrary orientation)
//! - **[`DirectVolume`]**, **[`MipVolume`]**, **[`MinipVolume`]**, **[`AverageVolume`]**,
//!   **[`IsosurfaceVolume`]**: volume ray marching, one visual per rendering mode
//! - **[`Custom`]**: Base for creating custom visualization types
//!
//! [`Image`] and the volume visuals share the virtual-texture streaming back-end
//! (atlas + page table + multi-resolution LOD), so terabyte-scale datasets render
//! from a fixed VRAM budget regardless of size.
//!
//! ### Streaming and Loading
//!
//! The virtual-texture visuals stream tiles on demand with a **pull-based** loader:
//!
//! 1. **Spatial partitioning**: the volume is a grid of tiles at multiple LOD levels
//! 2. **Visibility + LOD selection**: each frame, only the tiles in view at the
//!    screen-space-appropriate LOD are wanted
//! 3. **LRU cache**: the atlas is fixed-size; least-recently-used tiles are evicted
//! 4. **Pull-based loading**: the engine publishes a `wanted` set; the application
//!    polls it (`wanted_keys()`), fetches the bytes, and pushes them back
//!    (`set_chunk_data_u16()`). There is no engine callback on the hot path.
//!
//! ## Usage (native Rust)
//!
//! ```ignore
//! use bovista::{Renderer, Scene, Camera};
//!
//! let renderer = Renderer::new(device, queue, surface_format).await;
//! let mut scene = Scene::new();
//! let camera = Camera::new(aspect_ratio);
//!
//! // Visuals are wrapped for the scene's interior mutability.
//! scene.add(Arc::new(Mutex::new(points)));
//!
//! // Per frame:
//! renderer.update_camera(&camera);
//! scene.prepare(&device, &queue, &camera_info);
//! renderer.render(&scene, &view, &depth_view, clear_color);
//! ```
//!
//! ## Python bindings
//!
//! With the `python` feature, Bovista exposes PyO3 bindings. The host toolkit owns
//! the window and event loop; bovista renders one frame per `render_frame()` call.
//!
//! ```python
//! import bovista as bv
//! import numpy as np
//!
//! viewer = bv.Viewer(800, 600)
//! viewer.initialize_with_window(handle, width, height)  # e.g. int(widget.winId())
//!
//! positions = np.random.rand(1000, 1, 3).astype(np.float32)
//! colors = np.random.rand(1000, 1, 3).astype(np.float32)
//! points = bv.Points.from_numpy(viewer, positions, colors)
//! viewer.add(points)
//! viewer.render_frame()
//! ```
//!
//! ## Streaming OME-Zarr (pull-based loader)
//!
//! ```python
//! import bovista as bv
//!
//! # lod_levels: a list of bv.LevelMetadata, finest first.
//! image = bv.Image(viewer, lod_levels, max_tiles=500)
//! viewer.add(image)
//!
//! # A worker thread polls the wanted set, fetches tiles, and pushes them back.
//! # wanted_keys() returns [(lod, t, z, y, x, priority), ...] sorted by priority.
//! for lod, t, z, y, x, priority in image.wanted_keys():
//!     data = fetch_tile(lod, t, z, y, x)          # your fetch (zarr / S3 / HTTP)
//!     image.set_chunk_data_u16(lod, t, z, y, x, data)
//! ```
//!
//! See `examples/*/python/` for complete Qt integrations.

pub mod camera;
pub mod renderer;
pub mod scene;
pub mod spatial;
pub mod visual;
pub mod visuals;

// Shared utilities for bindings
pub mod bindings_common;

#[cfg(feature = "python")]
pub mod python;

#[cfg(target_arch = "wasm32")]
pub mod wasm;

pub use camera::{Camera, FrustumPlanes, ProjectionMode};
pub use renderer::{CameraUniforms, Renderer};
pub use scene::Scene;
pub use spatial::VolumeGrid;
pub use visual::{BlendMode, Transform, VertexAttribute, VertexBufferLayout, VertexFormat, Visual};
pub use visuals::{
    AverageVolume, Custom, DirectVolume, Image, IsosurfaceVolume, LabelVolume, Labels,
    Lines, LodLevelConfig, MinipVolume, MipVolume, Points, SliceOrientation,
    SlicePlane,
};
