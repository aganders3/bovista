//! Spatial data structures and algorithms for volumetric rendering.
//!
//! This module provides utilities for working with 3D grids, spatial culling,
//! and hierarchical traversal. These are used primarily by TiledImageVisual
//! but could be useful for other volumetric visualizations.

use glam::Vec3;

/// A 3D grid that divides a volume into uniform cells/chunks.
///
/// This is a common pattern for spatial partitioning in volumetric rendering,
/// used for:
/// - Chunk-based loading (TiledImageVisual)
/// - Spatial queries (finding chunks in a region)
/// - LOD management (multi-resolution grids)
#[derive(Debug, Clone)]
pub struct VolumeGrid {
    /// Total size of the volume in voxels (Z, Y, X)
    pub volume_size: (u32, u32, u32),

    /// Size of each cell/chunk in voxels (Z, Y, X)
    pub cell_size: (u32, u32, u32),

    /// Physical size of each voxel in world units (Z, Y, X)
    pub voxel_size: (f32, f32, f32),

    /// Number of cells in each dimension (Z, Y, X)
    pub grid_size: (u32, u32, u32),
}

impl VolumeGrid {
    /// Create a new volume grid from volume and cell sizes
    pub fn new(
        volume_size: (u32, u32, u32),
        cell_size: (u32, u32, u32),
        voxel_size: (f32, f32, f32),
    ) -> Self {
        let grid_size = (
            (volume_size.0 + cell_size.0 - 1) / cell_size.0,
            (volume_size.1 + cell_size.1 - 1) / cell_size.1,
            (volume_size.2 + cell_size.2 - 1) / cell_size.2,
        );

        Self {
            volume_size,
            cell_size,
            voxel_size,
            grid_size,
        }
    }

    /// Compute the axis-aligned bounding box (AABB) for a cell in world space
    ///
    /// # Arguments
    /// * `cell_idx` - The cell index (z, y, x) in the grid
    ///
    /// # Returns
    /// A tuple of (min, max) corners of the AABB in world coordinates
    pub fn cell_aabb(&self, cell_idx: (u32, u32, u32)) -> (Vec3, Vec3) {
        let (cz, cy, cx) = cell_idx;
        let (cell_z, cell_y, cell_x) = self.cell_size;
        let (voxel_z, voxel_y, voxel_x) = self.voxel_size;

        // Voxel range for this cell
        let z_start = cz * cell_z;
        let y_start = cy * cell_y;
        let x_start = cx * cell_x;

        let z_end = ((cz + 1) * cell_z).min(self.volume_size.0);
        let y_end = ((cy + 1) * cell_y).min(self.volume_size.1);
        let x_end = ((cx + 1) * cell_x).min(self.volume_size.2);

        // Convert to world space (note: swap to (X, Y, Z) for Vec3)
        let min = Vec3::new(
            x_start as f32 * voxel_x,
            y_start as f32 * voxel_y,
            z_start as f32 * voxel_z,
        );

        let max = Vec3::new(
            x_end as f32 * voxel_x,
            y_end as f32 * voxel_y,
            z_end as f32 * voxel_z,
        );

        (min, max)
    }

    /// Find all cells that overlap with a given AABB region
    ///
    /// # Arguments
    /// * `region_min` - Minimum corner of the query region in world space
    /// * `region_max` - Maximum corner of the query region in world space
    ///
    /// # Returns
    /// A vector of cell indices (z, y, x) that overlap the region
    pub fn cells_overlapping_aabb(&self, region_min: Vec3, region_max: Vec3) -> Vec<(u32, u32, u32)> {
        let (voxel_z, voxel_y, voxel_x) = self.voxel_size;
        let (cell_z, cell_y, cell_x) = self.cell_size;

        // Convert world coordinates back to voxel coordinates
        let voxel_min_z = (region_min.z / voxel_z).floor() as u32;
        let voxel_min_y = (region_min.y / voxel_y).floor() as u32;
        let voxel_min_x = (region_min.x / voxel_x).floor() as u32;

        let voxel_max_z = (region_max.z / voxel_z).ceil() as u32;
        let voxel_max_y = (region_max.y / voxel_y).ceil() as u32;
        let voxel_max_x = (region_max.x / voxel_x).ceil() as u32;

        // Convert to cell indices
        let cell_min_z = voxel_min_z / cell_z;
        let cell_min_y = voxel_min_y / cell_y;
        let cell_min_x = voxel_min_x / cell_x;

        let cell_max_z = ((voxel_max_z + cell_z - 1) / cell_z).min(self.grid_size.0);
        let cell_max_y = ((voxel_max_y + cell_y - 1) / cell_y).min(self.grid_size.1);
        let cell_max_x = ((voxel_max_x + cell_x - 1) / cell_x).min(self.grid_size.2);

        let mut result = Vec::new();
        for cz in cell_min_z..cell_max_z {
            for cy in cell_min_y..cell_max_y {
                for cx in cell_min_x..cell_max_x {
                    result.push((cz, cy, cx));
                }
            }
        }

        result
    }

    /// Total number of cells in the grid
    pub fn total_cells(&self) -> usize {
        (self.grid_size.0 * self.grid_size.1 * self.grid_size.2) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_grid_creation() {
        let grid = VolumeGrid::new(
            (100, 200, 300),  // volume size
            (10, 20, 30),     // cell size
            (1.0, 1.0, 1.0),  // voxel size
        );

        assert_eq!(grid.grid_size, (10, 10, 10));
        assert_eq!(grid.total_cells(), 1000);
    }

    #[test]
    fn test_cell_aabb() {
        let grid = VolumeGrid::new(
            (100, 100, 100),
            (10, 10, 10),
            (1.0, 1.0, 1.0),
        );

        let (min, max) = grid.cell_aabb((0, 0, 0));
        assert_eq!(min, Vec3::new(0.0, 0.0, 0.0));
        assert_eq!(max, Vec3::new(10.0, 10.0, 10.0));

        let (min, max) = grid.cell_aabb((1, 2, 3));
        assert_eq!(min, Vec3::new(30.0, 20.0, 10.0));
        assert_eq!(max, Vec3::new(40.0, 30.0, 20.0));
    }

    #[test]
    fn test_overlapping_cells() {
        let grid = VolumeGrid::new(
            (100, 100, 100),
            (10, 10, 10),
            (1.0, 1.0, 1.0),
        );

        // Query a region that spans multiple cells
        let region_min = Vec3::new(5.0, 5.0, 5.0);
        let region_max = Vec3::new(25.0, 25.0, 25.0);

        let cells = grid.cells_overlapping_aabb(region_min, region_max);

        // Should overlap cells (0,0,0), (0,0,1), (0,1,0), (0,1,1),
        //                    (1,0,0), (1,0,1), (1,1,0), (1,1,1),
        //                    (2,0,0), (2,0,1), (2,1,0), (2,1,1), (2,2,0), (2,2,1), (2,2,2)
        assert!(cells.len() >= 8);
        assert!(cells.contains(&(0, 0, 0)));
        assert!(cells.contains(&(2, 2, 2)));
    }
}
