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
//! - **[`Image`]**: Render 3D volumes with slice plane visualization and on-demand chunk loading
//! - **[`Custom`]**: Base for creating custom visualization types
//!
//! ### Chunked Loading System
//!
//! [`Image`] supports chunked loading with multi-resolution LOD (Level of Detail) for handling
//! datasets that are too large to fit in memory:
//!
//! 1. **Spatial Partitioning**: Volume is divided into tiles at multiple LOD levels
//! 2. **Visibility Culling**: Only visible tiles are loaded based on frustum and LOD selection
//! 3. **LRU Cache**: Least recently used tiles are evicted when memory limit is reached
//! 4. **Async Loading**: Tiles are loaded asynchronously via callbacks (typically from Python)
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use bovista::{Renderer, Scene, Camera, Points};
//!
//! // Initialize renderer with GPU device
//! let renderer = Renderer::new(device, queue, surface_format).await;
//! let mut scene = Scene::new();
//! let camera = Camera::new(aspect_ratio);
//!
//! // Create and add visuals
//! let points = Points::test_cube(&device, &format, &layout, 10);
//! scene.add(Arc::new(Mutex::new(points)));
//!
//! // Render loop
//! renderer.update_camera(&camera);
//! scene.prepare(&device, &queue);
//! renderer.render(&scene, &view, &depth_view, clear_color);
//! ```
//!
//! ## Python Bindings
//!
//! When compiled with the `python` feature, Bovista provides Python bindings via PyO3:
//!
//! ```python
//! import bovista as bv
//! import numpy as np
//!
//! # Create the viewer and attach it to a native window handle from your
//! # toolkit (e.g. Qt's `int(widget.winId())`). The host owns the window and
//! # event loop; bovista renders one frame per call to `render_frame()`.
//! viewer = bv.Viewer(800, 600)
//! viewer.initialize_with_window(handle, width, height)
//!
//! # Add a point cloud
//! positions = np.random.rand(1000, 1, 3).astype(np.float32)
//! colors = np.random.rand(1000, 1, 3).astype(np.float32)
//! points = bv.Points.from_numpy(viewer, positions, colors)
//! viewer.add(points)
//!
//! # Drive rendering from your toolkit's timer / paint callback
//! viewer.render_frame()
//! ```
//!
//! See `examples/*/python/` for a complete Qt integration.
//!
//! ## Remote Zarr Example
//!
//! The most advanced usage is loading multiscale OME-Zarr data from remote S3:
//!
//! ```python
//! import zarr
//! import bovista as bv
//!
//! # Open remote zarr store
//! store = zarr.open("https://s3.../dataset.zarr", mode="r")
//! array = store["0"]  # Resolution level 0
//!
//! # Define tile loader callback
//! def load_chunk(lod_level, tile_z, tile_y, tile_x):
//!     tile_data = array[tile_z:tile_z+tz, tile_y:tile_y+ty, tile_x:tile_x+tx]
//!     return np.array(tile_data, dtype=np.uint8)
//!
//! # Create image with chunked loading and multiple LOD levels
//! image = bv.Image.from_chunked(
//!     viewer,
//!     lod_levels=[...],  # List of LevelMetadata for each LOD
//!     loader=load_chunk,
//!     max_chunks=500
//! )
//! ```

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
    AverageVolume, Custom, DirectVolume, Image, IsosurfaceVolume, Labels,
    Lines, LodLevelConfig, MinipVolume, MipVolume, Points, SliceOrientation,
    SlicePlane,
};
