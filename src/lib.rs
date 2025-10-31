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
//! - **[`PointsVisual`]**: Render point clouds with per-point colors
//! - **[`LinesVisual`]**: Render line segments and wireframes
//! - **[`ImageVisual`]**: Render 3D volumes with slice plane visualization
//! - **[`TiledImageVisual`]**: Render massive volumes with on-demand chunk loading and LRU caching
//! - **[`CustomVisual`]**: Base for creating custom visualization types
//!
//! ### Chunked Loading System
//!
//! The [`TiledImageVisual`] implements a sophisticated chunked loading system for handling
//! datasets that are too large to fit in memory:
//!
//! 1. **Spatial Partitioning**: Volume is divided into a 3D grid of chunks
//! 2. **Visibility Culling**: Only chunks intersecting the current slice plane are loaded
//! 3. **LRU Cache**: Least recently used chunks are evicted when memory limit is reached
//! 4. **Async Loading**: Chunks are loaded asynchronously via callbacks (typically from Python)
//!
//! ## Usage Example
//!
//! ```rust,no_run
//! use bovista::{Renderer, Scene, Camera, PointsVisual};
//!
//! // Initialize renderer with GPU device
//! let renderer = Renderer::new(device, queue, surface_format).await;
//! let mut scene = Scene::new();
//! let camera = Camera::new(aspect_ratio);
//!
//! // Create and add visuals
//! let points = PointsVisual::test_cube(&device, &format, &layout, 10);
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
//! # Create viewer
//! viewer = bv.PyViewer(800, 600)
//! viewer.initialize()
//!
//! # Add point cloud
//! positions = np.random.rand(1000, 1, 3).astype(np.float32)
//! colors = np.random.rand(1000, 1, 3).astype(np.float32)
//! points = bv.PyPointsVisual.from_numpy(viewer, positions, colors)
//! viewer.add_points(points)
//!
//! # Run interactive window
//! viewer.run()
//! ```
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
//! # Define chunk loader callback
//! def load_chunk(z, y, x):
//!     chunk_data = array[z:z+cz, y:y+cy, x:x+cx]
//!     return np.array(chunk_data, dtype=np.uint8)
//!
//! # Create tiled image with on-demand loading
//! tiled = bv.PyTiledImageVisual.from_loader(
//!     viewer,
//!     volume_size=(depth, height, width),
//!     chunk_size=(cz, cy, cx),
//!     loader=load_chunk,
//!     max_loaded_chunks=150
//! )
//! ```

pub mod camera;
pub mod renderer;
pub mod scene;
pub mod visual;
pub mod visuals;

#[cfg(feature = "python")]
pub mod python;

pub use camera::Camera;
pub use renderer::{CameraUniforms, Renderer};
pub use scene::Scene;
pub use visual::{Transform, VertexAttribute, VertexBufferLayout, VertexFormat, Visual};
pub use visuals::{
    ChunkData, ChunkLoaderFn, ChunkRequest, CustomVisual, ImageVisual, LinesVisual, PointsVisual,
    SliceOrientation, SlicePlane, TiledImageVisual,
};
