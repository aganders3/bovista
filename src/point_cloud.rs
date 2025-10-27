use bytemuck::{Pod, Zeroable};

/// A vertex in a point cloud with position and color
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PointVertex {
    pub position: [f32; 3],
    pub color: [f32; 3],
}

impl PointVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<PointVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// A point cloud containing vertex data
pub struct PointCloud {
    pub vertices: Vec<PointVertex>,
}

impl PointCloud {
    pub fn new(vertices: Vec<PointVertex>) -> Self {
        Self { vertices }
    }

    /// Create a simple test point cloud (a cube of points)
    pub fn test_cube(size: u32) -> Self {
        let mut vertices = Vec::new();
        let step = 2.0 / size as f32;

        for i in 0..size {
            for j in 0..size {
                for k in 0..size {
                    let x = -1.0 + i as f32 * step;
                    let y = -1.0 + j as f32 * step;
                    let z = -1.0 + k as f32 * step;

                    // Color based on position (normalized to 0-1)
                    let color = [
                        (x + 1.0) / 2.0,
                        (y + 1.0) / 2.0,
                        (z + 1.0) / 2.0,
                    ];

                    vertices.push(PointVertex {
                        position: [x, y, z],
                        color,
                    });
                }
            }
        }

        Self::new(vertices)
    }

    pub fn vertex_count(&self) -> u32 {
        self.vertices.len() as u32
    }
}
