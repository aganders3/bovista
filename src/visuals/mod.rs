pub mod atlas;
pub mod custom;
pub mod image;
pub mod image_strategy;  // Tile-based strategy system - public for Python bindings
pub mod lines;
pub mod page_table;
pub mod points;
pub mod tile;  // Shared infrastructure for tile-based rendering

pub use custom::CustomVisual;
pub use image::{ImageVisual, SliceOrientation, SlicePlane};
pub use lines::LinesVisual;
pub use points::PointsVisual;
pub use image_strategy::LodLevelConfig;
