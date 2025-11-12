pub mod custom;
pub mod image;
mod image_strategy;  // Private - old implementation detail of ImageVisual (deprecated)
pub mod image_strategy_v2;  // New tile-based strategy system - public for Python bindings
pub mod lines;
pub mod points;
pub mod tiled_image;
pub mod tile;  // Shared infrastructure for tile-based rendering

// Private module for chunk rendering (used internally by TiledImageVisual)
mod chunk_visual;

pub use custom::CustomVisual;
pub use image::{ImageVisual, SliceOrientation, SlicePlane};
pub use lines::LinesVisual;
pub use points::PointsVisual;
pub use tiled_image::TiledImageVisual;
pub use tiled_image::{
    CapacityCheckFn, ChunkData, ChunkLoaderFn, ChunkRequest, LevelMetadata,
};
pub use image_strategy_v2::LodLevelConfig;
