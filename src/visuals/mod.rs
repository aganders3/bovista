pub mod custom;
pub mod image;
mod image_strategy;  // Private - implementation detail of ImageVisual
pub mod lines;
pub mod points;
pub mod tiled_image;

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
