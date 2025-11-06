// Private module for rendering individual chunks in 3D space
// Used internally by TiledImageVisual

use crate::visual::{Transform, Visual};
use crate::visuals::image::SlicePlane;
use bytemuck::{Pod, Zeroable};
use glam::Vec3;
use wgpu::util::DeviceExt;
use wgpu::RenderPass;

/// Vertex for chunk rendering (3D positioned quad with texture)
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ChunkVertex {
    position: [f32; 3],     // 3D position in world space
    texcoord: [f32; 3],     // 3D texture coordinates
}

impl ChunkVertex {
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<ChunkVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}

/// Uniforms for chunk rendering
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
struct ChunkUniforms {
    contrast_min: f32,
    contrast_max: f32,
    debug_mode: f32,  // 0.0 = off, 1.0 = on
    _padding1: f32,
    plane_position: [f32; 3],
    _padding2: f32,
    plane_normal: [f32; 3],
    _padding3: f32,
    volume_size: [f32; 3],
    _padding4: f32,
    debug_color: [f32; 3],
    _padding5: f32,
}

/// A single chunk rendered as a 3D textured quad
pub struct ChunkVisual {
    #[allow(dead_code)]  // Held for GPU resource lifetime
    texture: wgpu::Texture,
    #[allow(dead_code)]  // Held for GPU resource lifetime
    texture_view: wgpu::TextureView,
    vertex_buffer: wgpu::Buffer,
    index_buffer: wgpu::Buffer,
    uniform_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    render_pipeline: wgpu::RenderPipeline,

    chunk_size: (u32, u32, u32),
    voxel_size: (f32, f32, f32),
    world_position: Vec3,
    slice_plane: SlicePlane,
    contrast_limits: (f32, f32),
    needs_vertex_update: bool,
    current_vertex_count: u32,
    debug_mode: bool,
    debug_color: [f32; 3],

    transform: Transform,
    visible: bool,
    name: String,
}

impl ChunkVisual {
    /// Create a new chunk visual with 3D positioning
    pub fn new(
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        data: &[u8],
        width: u32,
        height: u32,
        depth: u32,
        voxel_size: (f32, f32, f32),
        world_position: Vec3,
    ) -> Self {
        // Create 3D texture
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Chunk Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Create a vertex buffer large enough for a hexagon (max intersection polygon)
        // We'll write the actual slice geometry in prepare()
        let max_vertices = 8; // Max possible intersection points with AABB
        let vertex_buffer_size = std::mem::size_of::<ChunkVertex>() * max_vertices;

        let vertex_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Chunk Vertex Buffer"),
            size: vertex_buffer_size as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Index buffer for triangle fan (0,1,2, 0,2,3, 0,3,4, ...) for up to 8 vertices
        let max_indices = (max_vertices - 2) * 3; // Triangle fan
        let index_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("Chunk Index Buffer"),
            size: (std::mem::size_of::<u16>() * max_indices) as u64,
            usage: wgpu::BufferUsages::INDEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // Temporary debug color for initialization
        let temp_color = [1.0, 1.0, 1.0];

        // Create uniforms
        let uniforms = ChunkUniforms {
            contrast_min: 0.0,
            contrast_max: 1.0,
            debug_mode: 0.0,
            _padding1: 0.0,
            plane_position: [world_position.x + width as f32 / 2.0, world_position.y + height as f32 / 2.0, world_position.z + depth as f32 / 2.0],
            _padding2: 0.0,
            plane_normal: [0.0, 0.0, 1.0],
            _padding3: 0.0,
            volume_size: [width as f32, height as f32, depth as f32],
            _padding4: 0.0,
            debug_color: temp_color,
            _padding5: 0.0,
        };

        let uniform_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Chunk Uniform Buffer"),
            contents: bytemuck::cast_slice(&[uniforms]),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });

        // Create sampler
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // Create bind group layout for chunk-specific resources
        let bind_group_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("Chunk Bind Group Layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        multisampled: false,
                        view_dimension: wgpu::TextureViewDimension::D3,
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("Chunk Bind Group"),
            layout: &bind_group_layout,
            entries: &[
                wgpu::BindGroupEntry {
                    binding: 0,
                    resource: wgpu::BindingResource::TextureView(&texture_view),
                },
                wgpu::BindGroupEntry {
                    binding: 1,
                    resource: wgpu::BindingResource::Sampler(&sampler),
                },
                wgpu::BindGroupEntry {
                    binding: 2,
                    resource: uniform_buffer.as_entire_binding(),
                },
            ],
        });

        // Create render pipeline
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Chunk Shader"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/chunk.wgsl").into()),
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("Chunk Pipeline Layout"),
            bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout],
            push_constant_ranges: &[],
        });

        let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("Chunk Render Pipeline"),
            layout: Some(&pipeline_layout),
            vertex: wgpu::VertexState {
                module: &shader,
                entry_point: Some("vs_main"),
                buffers: &[ChunkVertex::desc()],
                compilation_options: Default::default(),
            },
            fragment: Some(wgpu::FragmentState {
                module: &shader,
                entry_point: Some("fs_main"),
                targets: &[Some(wgpu::ColorTargetState {
                    format: surface_format,
                    blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
                compilation_options: Default::default(),
            }),
            primitive: wgpu::PrimitiveState {
                topology: wgpu::PrimitiveTopology::TriangleList,
                strip_index_format: None,
                front_face: wgpu::FrontFace::Ccw,
                cull_mode: None,  // No back face culling for slices
                polygon_mode: wgpu::PolygonMode::Fill,
                unclipped_depth: false,
                conservative: false,
            },
            depth_stencil: Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth32Float,
                depth_write_enabled: true,
                depth_compare: wgpu::CompareFunction::LessEqual,
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            }),
            multisample: wgpu::MultisampleState {
                count: 1,
                mask: !0,
                alpha_to_coverage_enabled: false,
            },
            multiview: None,
            cache: None,
        });

        // Compute debug color based on world position (different color per Z-layer)
        let z_index = (world_position.z / depth as f32).floor() as u32;
        let debug_color = match z_index % 6 {
            0 => [1.0, 0.0, 0.0], // Red
            1 => [0.0, 1.0, 0.0], // Green
            2 => [0.0, 0.0, 1.0], // Blue
            3 => [1.0, 1.0, 0.0], // Yellow
            4 => [1.0, 0.0, 1.0], // Magenta
            5 => [0.0, 1.0, 1.0], // Cyan
            _ => [1.0, 1.0, 1.0], // White
        };

        Self {
            texture,
            texture_view,
            vertex_buffer,
            index_buffer,
            uniform_buffer,
            bind_group,
            render_pipeline,
            chunk_size: (width, height, depth),
            voxel_size,
            world_position,
            slice_plane: SlicePlane::default(),
            contrast_limits: (0.0, 1.0),
            needs_vertex_update: true,
            current_vertex_count: 0,
            debug_mode: false,
            debug_color,
            transform: Transform::identity(),
            visible: true,
            name: "Chunk".to_string(),
        }
    }

    pub fn set_slice_plane(&mut self, plane: SlicePlane) {
        self.slice_plane = plane;
        self.needs_vertex_update = true;
    }

    pub fn set_contrast_limits(&mut self, min: f32, max: f32) {
        self.contrast_limits = (min, max);
        // Update uniforms will be called in prepare()
    }

    pub fn set_debug_mode(&mut self, enabled: bool) {
        self.debug_mode = enabled;
    }
}

impl Visual for ChunkVisual {
    fn prepare(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue) {
        // Update slice geometry if needed
        if self.needs_vertex_update {
            let min = self.world_position;
            // Scale chunk_size by voxel_size to get world-space extent
            // NOTE: chunk_size is (width, height, depth) = (X, Y, Z)
            //       but voxel_size is (z_size, y_size, x_size) = (Z, Y, X)
            //       so we need to reorder indices when multiplying
            let max = self.world_position + Vec3::new(
                self.chunk_size.0 as f32 * self.voxel_size.2,  // X: width * x_voxel_size
                self.chunk_size.1 as f32 * self.voxel_size.1,  // Y: height * y_voxel_size
                self.chunk_size.2 as f32 * self.voxel_size.0,  // Z: depth * z_voxel_size
            );

            // Catch panics in geometry generation to avoid poisoning the mutex
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                compute_plane_aabb_intersection(&self.slice_plane, min, max)
            }));

            match result {
                Ok(Some((vertices, indices))) => {
                    // Write vertices to buffer
                    queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));

                    // Write indices to buffer
                    // wgpu requires buffer writes to be 4-byte aligned, so pad if needed
                    let indices_bytes = bytemuck::cast_slice::<u16, u8>(&indices);
                    let aligned_size = (indices_bytes.len() + 3) & !3; // Round up to multiple of 4

                    if indices_bytes.len() == aligned_size {
                        queue.write_buffer(&self.index_buffer, 0, indices_bytes);
                    } else {
                        // Need padding
                        let mut padded = indices_bytes.to_vec();
                        padded.resize(aligned_size, 0);
                        queue.write_buffer(&self.index_buffer, 0, &padded);
                    }

                    self.current_vertex_count = indices.len() as u32;
                }
                Ok(None) => {
                    // No intersection - this chunk doesn't intersect the slice plane
                    self.current_vertex_count = 0;
                }
                Err(e) => {
                    // Panic occurred - log and skip this chunk
                    eprintln!("Error computing plane-AABB intersection for chunk at {:?}: {:?}",
                              self.world_position, e);
                    self.current_vertex_count = 0;
                }
            }

            self.needs_vertex_update = false;
        }

        // Update uniforms with current slice plane and contrast
        let uniforms = ChunkUniforms {
            contrast_min: self.contrast_limits.0,
            contrast_max: self.contrast_limits.1,
            debug_mode: if self.debug_mode { 1.0 } else { 0.0 },
            _padding1: 0.0,
            plane_position: self.slice_plane.position,
            _padding2: 0.0,
            plane_normal: self.slice_plane.normal,
            _padding3: 0.0,
            volume_size: [self.chunk_size.0 as f32, self.chunk_size.1 as f32, self.chunk_size.2 as f32],
            _padding4: 0.0,
            debug_color: self.debug_color,
            _padding5: 0.0,
        };

        queue.write_buffer(&self.uniform_buffer, 0, bytemuck::cast_slice(&[uniforms]));
    }

    fn render(&self, render_pass: &mut RenderPass) {
        if self.current_vertex_count == 0 {
            return; // No intersection, nothing to render
        }

        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_bind_group(1, &self.bind_group, &[]);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.current_vertex_count, 0, 0..1);
    }

    fn transform(&self) -> &Transform {
        &self.transform
    }

    fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }

    fn is_visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    fn name(&self) -> &str {
        &self.name
    }
}

/// Compute the intersection polygon between a plane and an AABB
/// Returns vertices and indices for rendering the slice, or None if no intersection
fn compute_plane_aabb_intersection(
    plane: &SlicePlane,
    min: Vec3,
    max: Vec3,
) -> Option<(Vec<ChunkVertex>, Vec<u16>)> {
    let plane_pos = Vec3::from(plane.position);
    let plane_normal = Vec3::from(plane.normal);

    // 12 edges of the AABB
    let edges = [
        // Bottom face (y = min)
        (Vec3::new(min.x, min.y, min.z), Vec3::new(max.x, min.y, min.z)),
        (Vec3::new(max.x, min.y, min.z), Vec3::new(max.x, min.y, max.z)),
        (Vec3::new(max.x, min.y, max.z), Vec3::new(min.x, min.y, max.z)),
        (Vec3::new(min.x, min.y, max.z), Vec3::new(min.x, min.y, min.z)),
        // Top face (y = max)
        (Vec3::new(min.x, max.y, min.z), Vec3::new(max.x, max.y, min.z)),
        (Vec3::new(max.x, max.y, min.z), Vec3::new(max.x, max.y, max.z)),
        (Vec3::new(max.x, max.y, max.z), Vec3::new(min.x, max.y, max.z)),
        (Vec3::new(min.x, max.y, max.z), Vec3::new(min.x, max.y, min.z)),
        // Vertical edges
        (Vec3::new(min.x, min.y, min.z), Vec3::new(min.x, max.y, min.z)),
        (Vec3::new(max.x, min.y, min.z), Vec3::new(max.x, max.y, min.z)),
        (Vec3::new(max.x, min.y, max.z), Vec3::new(max.x, max.y, max.z)),
        (Vec3::new(min.x, min.y, max.z), Vec3::new(min.x, max.y, max.z)),
    ];

    let mut intersection_points = Vec::new();

    // Find intersections with each edge
    for (p0, p1) in edges.iter() {
        if let Some(t) = line_plane_intersection(*p0, *p1, plane_pos, plane_normal) {
            if t >= 0.0 && t <= 1.0 {
                let point = p0.lerp(*p1, t);
                // Calculate 3D texture coordinate
                let texcoord = Vec3::new(
                    (point.x - min.x) / (max.x - min.x),
                    (point.y - min.y) / (max.y - min.y),
                    (point.z - min.z) / (max.z - min.z),
                );
                intersection_points.push((point, texcoord));
            }
        }
    }

    // Need at least 3 points for a polygon
    if intersection_points.len() < 3 {
        return None;
    }

    // Remove ALL duplicate points (not just consecutive ones)
    let epsilon = 0.01; // Slightly larger epsilon for robustness
    let mut unique_points = Vec::new();
    for point in intersection_points {
        let is_duplicate = unique_points.iter().any(|(p, _): &(Vec3, Vec3)| {
            p.distance(point.0) < epsilon
        });
        if !is_duplicate {
            unique_points.push(point);
        }
    }

    if unique_points.len() < 3 {
        return None;
    }

    let intersection_points = unique_points;

    // Sort points to form a convex polygon
    // Use the plane's coordinate system
    let sorted_points = sort_points_on_plane(&intersection_points, plane_normal);

    // Convert to vertices
    let vertices: Vec<ChunkVertex> = sorted_points
        .iter()
        .map(|(pos, tex)| ChunkVertex {
            position: [pos.x, pos.y, pos.z],
            texcoord: [tex.x, tex.y, tex.z],
        })
        .collect();

    // Generate indices as a triangle fan (0,1,2, 0,2,3, 0,3,4, ...)
    let mut indices = Vec::new();
    for i in 1..(vertices.len() - 1) {
        indices.push(0);
        indices.push(i as u16);
        indices.push((i + 1) as u16);
    }

    Some((vertices, indices))
}

/// Find the intersection point of a line segment with a plane
/// Returns t value where intersection = p0 + t * (p1 - p0)
fn line_plane_intersection(p0: Vec3, p1: Vec3, plane_pos: Vec3, plane_normal: Vec3) -> Option<f32> {
    let ray_dir = p1 - p0;
    let denom = plane_normal.dot(ray_dir);

    // Line is parallel to plane
    if denom.abs() < 1e-6 {
        return None;
    }

    let t = plane_normal.dot(plane_pos - p0) / denom;
    Some(t)
}

/// Sort points to form a convex polygon in counter-clockwise order
fn sort_points_on_plane(points: &[(Vec3, Vec3)], plane_normal: Vec3) -> Vec<(Vec3, Vec3)> {
    if points.is_empty() {
        return Vec::new();
    }

    // Find centroid
    let centroid = points.iter().map(|(p, _)| *p).sum::<Vec3>() / points.len() as f32;

    // Create a coordinate system on the plane
    let up = if plane_normal.y.abs() < 0.9 {
        Vec3::Y
    } else {
        Vec3::X
    };

    let tangent_unnormalized = plane_normal.cross(up);
    let tangent = if tangent_unnormalized.length() > 1e-6 {
        tangent_unnormalized.normalize()
    } else {
        // Fallback if cross product is too small
        Vec3::X
    };

    let bitangent_unnormalized = plane_normal.cross(tangent);
    let bitangent = if bitangent_unnormalized.length() > 1e-6 {
        bitangent_unnormalized.normalize()
    } else {
        Vec3::Y
    };

    // Sort by angle around centroid
    let mut sorted = points.to_vec();
    sorted.sort_by(|(a, _), (b, _)| {
        let va = *a - centroid;
        let vb = *b - centroid;

        let angle_a = va.dot(tangent).atan2(va.dot(bitangent));
        let angle_b = vb.dot(tangent).atan2(vb.dot(bitangent));

        // Handle NaN safely
        angle_a.partial_cmp(&angle_b).unwrap_or(std::cmp::Ordering::Equal)
    });

    sorted
}
