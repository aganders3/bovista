# Visual System

> **⚠️ NOTE:** Some code examples in this chapter are outdated due to recent ImageVisual unification.
> References to `ImageVertex`, `ImageUniforms`, `image.wgsl` have been replaced with `TileVertex`,
> `TileUniforms`, and `tile.wgsl`. The concepts and patterns remain valid, but specific struct
> definitions may differ. See the current implementation in `src/visuals/image.rs` and `src/visuals/tile.rs`.

The visual system in Bovista provides a unified interface for rendering different types of 3D content. All visuals implement the `Visual` trait, which defines the core rendering lifecycle and provides common functionality for transforms, visibility, and naming.

<!-- toc -->

## Core Concepts

### The Visual Trait

Every visual in Bovista implements this trait:

```rust
pub trait Visual: Send {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue);
    fn render(&self, render_pass: &mut RenderPass);

    // Transform management
    fn set_transform(&mut self, transform: Transform);
    fn transform(&self) -> &Transform;

    // Visibility control
    fn is_visible(&self) -> bool;
    fn set_visible(&mut self, visible: bool);

    // Debug/UI naming
    fn name(&self) -> &str;
}
```

The **prepare** phase is where visuals update GPU resources (buffers, uniforms) before rendering. The **render** phase emits GPU commands into a render pass.

### Built-in Visual Types

Bovista provides five core visual types, each optimized for different use cases:

- **PointsVisual** - Fast point cloud rendering for scatter plots and particle systems
- **LinesVisual** - Line segment rendering for wireframes, axes, and connections
- **ImageVisual** - Slice-plane rendering through 3D volumes with virtual texture streaming
- **VolumeVisual** - Direct volume rendering via ray marching with transfer functions; shares the same virtual texture back-end as `ImageVisual`
- **CustomVisual** - User-defined WGSL shaders for specialized rendering

## Annotated Source Code

The following sections provide fully annotated implementations of each visual type, with detailed explanations of how they work at the GPU level.

---

## PointsVisual - Complete Implementation

The `PointsVisual` renders collections of colored points in 3D space. It's one of the simplest visuals, making it an excellent starting point for understanding the system.

### Data Structures

```rust
/// A vertex in a point cloud with position and RGB color.
///
/// Memory layout: 24 bytes total (two vec3s)
/// This struct can be safely transmitted to GPU memory via bytemuck.
#[repr(C)]  // Ensures C-compatible layout for GPU
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct PointVertex {
    pub position: [f32; 3],  // 12 bytes: X, Y, Z world coordinates
    pub color: [f32; 3],     // 12 bytes: RGB values (0.0-1.0)
}

impl PointVertex {
    // Define vertex attributes for the GPU pipeline
    // These map to @location(0) and @location(1) in the shader
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![
            0 => Float32x3,  // position at location 0
            1 => Float32x3   // color at location 1
        ];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            // Stride: how many bytes to advance for each vertex
            array_stride: std::mem::size_of::<PointVertex>() as wgpu::BufferAddress,

            // VertexStepMode::Vertex means advance per vertex (not per instance)
            step_mode: wgpu::VertexStepMode::Vertex,

            attributes: &Self::ATTRIBS,
        }
    }
}
```

### The Visual Structure

```rust
/// The main structure holding all GPU resources needed for rendering
pub struct PointsVisual {
    // GPU buffer containing all vertex data (positions + colors)
    vertex_buffer: wgpu::Buffer,

    // Number of points to render (cached for draw call)
    vertex_count: u32,

    // Compiled rendering pipeline (shaders + state)
    render_pipeline: wgpu::RenderPipeline,

    // Local-to-world transformation matrix
    transform: Transform,

    // Visibility flag (checked before rendering)
    visible: bool,

    // Human-readable name for debugging/UI
    name: String,
}
```

### Constructor - Resource Creation

```rust
impl PointsVisual {
    /// Creates a new point cloud from vertex data
    ///
    /// This function sets up all GPU resources needed for rendering:
    /// 1. Uploads vertex data to GPU
    /// 2. Compiles the shader
    /// 3. Creates the render pipeline with proper state
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        vertices: Vec<PointVertex>,
    ) -> Self {
        let vertex_count = vertices.len() as u32;

        // ═══════════════════════════════════════════════════════════
        // STEP 1: Create vertex buffer and upload data to GPU
        // ═══════════════════════════════════════════════════════════
        let vertex_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Points Vertex Buffer"),

                // bytemuck safely reinterprets Vec<PointVertex> as &[u8]
                contents: bytemuck::cast_slice(&vertices),

                // VERTEX usage means this buffer feeds the vertex shader
                usage: wgpu::BufferUsages::VERTEX,
            }
        );

        // ═══════════════════════════════════════════════════════════
        // STEP 2: Compile the WGSL shader
        // ═══════════════════════════════════════════════════════════
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Points Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/point_cloud.wgsl").into()
            ),
        });

        // ═══════════════════════════════════════════════════════════
        // STEP 3: Create pipeline layout
        // ═══════════════════════════════════════════════════════════
        // This defines what bind groups the pipeline can access
        let pipeline_layout = device.create_pipeline_layout(
            &wgpu::PipelineLayoutDescriptor {
                label: Some("Points Pipeline Layout"),

                // We only need the camera bind group (view/projection matrices)
                bind_group_layouts: &[camera_bind_group_layout],

                // No push constants needed for simple points
                push_constant_ranges: &[],
            }
        );

        // ═══════════════════════════════════════════════════════════
        // STEP 4: Create the render pipeline
        // ═══════════════════════════════════════════════════════════
        // This is the complete GPU rendering configuration
        let render_pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("Points Render Pipeline"),
                layout: Some(&pipeline_layout),

                // --- VERTEX STAGE ---
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),  // Function name in WGSL
                    buffers: &[PointVertex::desc()],  // Input vertex format
                    compilation_options: Default::default(),
                },

                // --- FRAGMENT STAGE ---
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),  // Function name in WGSL
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,  // Must match window surface
                        blend: Some(wgpu::BlendState::REPLACE),  // No blending
                        write_mask: wgpu::ColorWrites::ALL,  // Write all channels
                    })],
                    compilation_options: Default::default(),
                }),

                // --- PRIMITIVE ASSEMBLY ---
                primitive: wgpu::PrimitiveState {
                    // PointList: each vertex is an independent point
                    topology: wgpu::PrimitiveTopology::PointList,

                    strip_index_format: None,  // Not used for points
                    front_face: wgpu::FrontFace::Ccw,  // Not relevant for points
                    cull_mode: None,  // Don't cull points
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },

                // --- DEPTH TESTING ---
                // Ensures points occlude each other correctly in 3D
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24PlusStencil8,
                    depth_write_enabled: true,  // Points write to depth buffer
                    depth_compare: wgpu::CompareFunction::Less,  // Standard depth test
                    stencil: wgpu::StencilState::default(),
                    bias: wgpu::DepthBiasState::default(),
                }),

                // --- MULTISAMPLING ---
                multisample: wgpu::MultisampleState {
                    count: 1,  // No MSAA for now
                    mask: !0,
                    alpha_to_coverage_enabled: false,
                },

                multiview: None,  // Not using VR/multiview
                cache: None,
            }
        );

        // Return the fully initialized visual
        Self {
            vertex_buffer,
            vertex_count,
            render_pipeline,
            transform: Transform::identity(),  // No transformation initially
            visible: true,  // Visible by default
            name: "Points".to_string(),
        }
    }
}
```

### Visual Trait Implementation

```rust
impl Visual for PointsVisual {
    /// Called every frame before rendering to update GPU resources
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {
        // PointsVisual is currently static - no per-frame updates needed
        // Future enhancements could support:
        // - Dynamic vertex buffer updates
        // - Animated colors
        // - LOD switching based on camera distance
    }

    /// Emits GPU commands to render the points
    fn render(&self, render_pass: &mut RenderPass) {
        // Set the pipeline (shaders + render state)
        render_pass.set_pipeline(&self.render_pipeline);

        // Bind vertex buffer to slot 0 (matches pipeline layout)
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

        // Draw all points
        // Parameters: vertex_range (0..vertex_count), instance_range (0..1)
        // We're drawing one instance of all vertices
        render_pass.draw(0..self.vertex_count, 0..1);
    }

    // Transform accessors
    fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }

    fn transform(&self) -> &Transform {
        &self.transform
    }

    // Visibility control
    fn is_visible(&self) -> bool {
        self.visible
    }

    fn set_visible(&mut self, visible: bool) {
        self.visible = visible;
    }

    // Name for debugging
    fn name(&self) -> &str {
        &self.name
    }
}
```

### Key Insights

**Why PointList topology?** Each vertex becomes an independent point primitive. The GPU rasterizes each point as a small square on screen (size controlled by shader).

**Why no index buffer?** Points don't share vertices, so we directly render from the vertex buffer. This is simpler and more efficient than using indices.

**Memory efficiency:** At 24 bytes per vertex, a million points only takes ~24MB of GPU memory, making this highly scalable.

**Pipeline caching:** The pipeline is created once and reused every frame, making rendering very fast (just a few GPU commands per frame).

---

## LinesVisual - Complete Implementation

The `LinesVisual` renders colored line segments, perfect for wireframes, axes, and debug visualizations. It's very similar to `PointsVisual` but uses LineList topology.

### Data Structures

```rust
/// A vertex for line rendering with position and color
///
/// Identical to PointVertex but semantically represents line endpoints
#[repr(C)]
#[derive(Copy, Clone, Debug, Pod, Zeroable)]
pub struct LineVertex {
    pub position: [f32; 3],  // 12 bytes: endpoint position
    pub color: [f32; 3],     // 12 bytes: line color at this endpoint
}

impl LineVertex {
    // Same attribute layout as points
    const ATTRIBS: [wgpu::VertexAttribute; 2] =
        wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];

    pub fn desc() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<LineVertex>() as wgpu::BufferAddress,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &Self::ATTRIBS,
        }
    }
}
```

### The Visual Structure

```rust
pub struct LinesVisual {
    vertex_buffer: wgpu::Buffer,     // Pairs of vertices define line segments
    vertex_count: u32,               // Total vertices (lines = vertex_count / 2)
    render_pipeline: wgpu::RenderPipeline,
    transform: Transform,
    visible: bool,
    name: String,
}
```

### Constructor - The Key Difference

```rust
impl LinesVisual {
    /// Create a new LinesVisual from vertex pairs
    ///
    /// IMPORTANT: Vertices must come in pairs!
    /// vertices[0]-vertices[1] = first line
    /// vertices[2]-vertices[3] = second line
    /// etc.
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        vertices: Vec<LineVertex>,
    ) -> Self {
        let vertex_count = vertices.len() as u32;

        // Upload vertex data (same as points)
        let vertex_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Lines Vertex Buffer"),
                contents: bytemuck::cast_slice(&vertices),
                usage: wgpu::BufferUsages::VERTEX,
            }
        );

        // Compile shader
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Lines Shader"),
            source: wgpu::ShaderSource::Wgsl(
                include_str!("../shaders/lines.wgsl").into()
            ),
        });

        // Pipeline layout (same as points - just needs camera)
        let pipeline_layout = device.create_pipeline_layout(
            &wgpu::PipelineLayoutDescriptor {
                label: Some("Lines Pipeline Layout"),
                bind_group_layouts: &[camera_bind_group_layout],
                push_constant_ranges: &[],
            }
        );

        // Create render pipeline
        let render_pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("Lines Render Pipeline"),
                layout: Some(&pipeline_layout),

                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    buffers: &[LineVertex::desc()],
                    compilation_options: Default::default(),
                },

                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some("fs_main"),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,
                        blend: Some(wgpu::BlendState::REPLACE),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),

                // ═══════════════════════════════════════════════════
                // KEY DIFFERENCE: LineList topology
                // ═══════════════════════════════════════════════════
                primitive: wgpu::PrimitiveState {
                    // LineList: every 2 vertices form a line segment
                    topology: wgpu::PrimitiveTopology::LineList,

                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,  // Lines have no front/back
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },

                // Same depth testing as points
                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24PlusStencil8,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
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
            }
        );

        Self {
            vertex_buffer,
            vertex_count,
            render_pipeline,
            transform: Transform::identity(),
            visible: true,
            name: "Lines".to_string(),
        }
    }

    /// Create a 3D axis helper - demonstrates line construction
    pub fn axis_helper(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        length: f32,
    ) -> Self {
        // Build 3 lines (6 vertices) for X, Y, Z axes
        let vertices = vec![
            // X axis (red) - origin to +X
            LineVertex {
                position: [0.0, 0.0, 0.0],
                color: [1.0, 0.0, 0.0],  // Red start
            },
            LineVertex {
                position: [length, 0.0, 0.0],
                color: [1.0, 0.0, 0.0],  // Red end
            },

            // Y axis (green) - origin to +Y
            LineVertex {
                position: [0.0, 0.0, 0.0],
                color: [0.0, 1.0, 0.0],  // Green start
            },
            LineVertex {
                position: [0.0, length, 0.0],
                color: [0.0, 1.0, 0.0],  // Green end
            },

            // Z axis (blue) - origin to +Z
            LineVertex {
                position: [0.0, 0.0, 0.0],
                color: [0.0, 0.0, 1.0],  // Blue start
            },
            LineVertex {
                position: [0.0, 0.0, length],
                color: [0.0, 0.0, 1.0],  // Blue end
            },
        ];

        Self::new(device, surface_format, camera_bind_group_layout, vertices)
    }
}
```

### Visual Trait Implementation

```rust
impl Visual for LinesVisual {
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {
        // Static lines - no updates needed
    }

    fn render(&self, render_pass: &mut RenderPass) {
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));

        // Same draw call as points - topology determines interpretation
        render_pass.draw(0..self.vertex_count, 0..1);
    }

    fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }

    fn transform(&self) -> &Transform {
        &self.transform
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
```

### Key Insights

**LineList vs LineStrip:** `LineList` treats every pair as independent. `LineStrip` would connect consecutive vertices (v0-v1, v1-v2, v2-v3...), useful for continuous paths.

**Color interpolation:** The GPU automatically interpolates colors between line endpoints, creating smooth gradients.

**Line width:** In WebGPU, line width is always 1 pixel. For thicker lines, you'd need to generate triangle geometry.

---

## CustomVisual - Complete Implementation

The `CustomVisual` allows users to provide their own WGSL shaders while leveraging the Bovista infrastructure. This is the most flexible visual type.

### The Visual Structure

```rust
/// A visual powered by user-provided WGSL shader code
///
/// Gives full shader control while handling GPU resource management
pub struct CustomVisual {
    vertex_buffer: wgpu::Buffer,        // User's vertex data
    vertex_count: u32,
    render_pipeline: wgpu::RenderPipeline,  // Compiled from user shader
    transform: Transform,
    visible: bool,
    name: String,
}
```

### Constructor - Shader Compilation

```rust
impl CustomVisual {
    /// Create a custom visual with user-provided WGSL shader
    ///
    /// # Shader Requirements
    ///
    /// Your shader MUST have:
    /// - Camera uniforms at @group(0) @binding(0)
    /// - Vertex entry point: vs_main
    /// - Fragment entry point: fs_main
    ///
    /// # Arguments
    ///
    /// * `shader_source` - WGSL code as a string
    /// * `vertex_data` - Raw vertex bytes
    /// * `vertex_layout` - Describes vertex structure
    /// * `topology` - How to interpret vertices (Points, Lines, Triangles...)
    pub fn new(
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        camera_bind_group_layout: &wgpu::BindGroupLayout,
        shader_source: &str,
        vertex_data: &[u8],
        vertex_layout: VertexBufferLayout,
        topology: wgpu::PrimitiveTopology,
    ) -> Result<Self, String> {
        // Calculate vertex count from data size and stride
        let vertex_count = (vertex_data.len() as u64 / vertex_layout.stride) as u32;

        // ═══════════════════════════════════════════════════════════
        // STEP 1: Upload user vertex data
        // ═══════════════════════════════════════════════════════════
        let vertex_buffer = device.create_buffer_init(
            &wgpu::util::BufferInitDescriptor {
                label: Some("Custom Visual Vertex Buffer"),
                contents: vertex_data,  // User's raw bytes
                usage: wgpu::BufferUsages::VERTEX,
            }
        );

        // ═══════════════════════════════════════════════════════════
        // STEP 2: Compile user's WGSL shader
        // ═══════════════════════════════════════════════════════════
        // This can fail if shader has syntax errors!
        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("Custom Visual Shader"),
            source: wgpu::ShaderSource::Wgsl(shader_source.into()),
        });

        // ═══════════════════════════════════════════════════════════
        // STEP 3: Create pipeline layout
        // ═══════════════════════════════════════════════════════════
        // We provide the camera bind group, user's shader must expect it
        let pipeline_layout = device.create_pipeline_layout(
            &wgpu::PipelineLayoutDescriptor {
                label: Some("Custom Visual Pipeline Layout"),
                bind_group_layouts: &[camera_bind_group_layout],
                push_constant_ranges: &[],
            }
        );

        // Convert our vertex layout to WGPU format
        let wgpu_vertex_layout = vertex_layout.to_wgpu();

        // ═══════════════════════════════════════════════════════════
        // STEP 4: Create render pipeline with user's shader
        // ═══════════════════════════════════════════════════════════
        let render_pipeline = device.create_render_pipeline(
            &wgpu::RenderPipelineDescriptor {
                label: Some("Custom Visual Render Pipeline"),
                layout: Some(&pipeline_layout),

                vertex: wgpu::VertexState {
                    module: &shader,  // User's shader
                    entry_point: Some("vs_main"),  // Required name
                    buffers: &[wgpu_vertex_layout],  // User's vertex format
                    compilation_options: Default::default(),
                },

                fragment: Some(wgpu::FragmentState {
                    module: &shader,  // User's shader
                    entry_point: Some("fs_main"),  // Required name
                    targets: &[Some(wgpu::ColorTargetState {
                        format: surface_format,

                        // Alpha blending for transparency support
                        blend: Some(wgpu::BlendState::ALPHA_BLENDING),

                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                    compilation_options: Default::default(),
                }),

                primitive: wgpu::PrimitiveState {
                    topology,  // User-specified topology
                    strip_index_format: None,
                    front_face: wgpu::FrontFace::Ccw,
                    cull_mode: None,
                    unclipped_depth: false,
                    polygon_mode: wgpu::PolygonMode::Fill,
                    conservative: false,
                },

                depth_stencil: Some(wgpu::DepthStencilState {
                    format: wgpu::TextureFormat::Depth24PlusStencil8,
                    depth_write_enabled: true,
                    depth_compare: wgpu::CompareFunction::Less,
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
            }
        );

        Ok(Self {
            vertex_buffer,
            vertex_count,
            render_pipeline,
            transform: Transform::identity(),
            visible: true,
            name: "Custom Visual".to_string(),
        })
    }
}
```

### Visual Trait Implementation

```rust
impl Visual for CustomVisual {
    fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {
        // Future: could support dynamic uniform updates
    }

    fn render(&self, render_pass: &mut RenderPass) {
        render_pass.set_pipeline(&self.render_pipeline);
        render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
        render_pass.draw(0..self.vertex_count, 0..1);
    }

    fn set_transform(&mut self, transform: Transform) {
        self.transform = transform;
    }

    fn transform(&self) -> &Transform {
        &self.transform
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
```

### Example Custom Shader

Here's a minimal WGSL shader that works with `CustomVisual`:

```wgsl
// Camera uniforms (REQUIRED at @group(0) @binding(0))
struct Uniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> uniforms: Uniforms;

// Your custom vertex structure (must match VertexBufferLayout)
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec4<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) color: vec4<f32>,
}

// REQUIRED: vs_main entry point
@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;

    // Transform to clip space
    output.clip_position = uniforms.view_proj * vec4<f32>(input.position, 1.0);
    output.color = input.color;

    return output;
}

// REQUIRED: fs_main entry point
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return input.color;  // Simple pass-through
}
```

### Key Insights

**Shader validation:** The GPU driver validates WGSL at runtime. Syntax errors will cause pipeline creation to fail, which we handle with `Result<Self, String>`.

**Cross-platform:** WGSL works identically on native (Vulkan/Metal/DX12) and WebAssembly/WebGPU. Your custom shaders run everywhere!

**Extensibility:** Future versions could support additional bind groups for textures, allowing even more complex custom effects.

---

## ImageVisual and VolumeVisual

`ImageVisual` and `VolumeVisual` both render large volumetric datasets. They share the same streaming back-end (`VirtualTextureData`) but present the data differently: `ImageVisual` intersects a plane through the volume and samples it, while `VolumeVisual` fires a ray per pixel and composites voxel contributions front-to-back for direct volume rendering (DVR).

### Virtual Texture Architecture

Both visuals use a **virtual texture** approach: the full volume is notionally divided into uniformly-sized tiles at each LOD, but only the tiles currently visible are kept in VRAM.

The two key GPU resources are:

**Atlas** (`AtlasAllocator` + a 3D `wgpu::Texture`): a fixed-size texture that holds all physically resident tiles, packed into a grid of slots. The atlas size is set at creation time (`max_tiles` parameter) and never changes. When a new tile arrives and the atlas is full, the LRU slot is evicted.

**Page table** (`PageTable` + a 2D-array `wgpu::Texture`, one layer per LOD): a small integer texture that maps virtual tile addresses `(lod, grid_z, grid_y, grid_x)` to atlas slot indices. The shader looks up this indirection table at runtime to find where each tile lives in the atlas (or learns it's absent and falls back to a coarser LOD).

Because the indirection is resolved in the shader, both `ImageVisual` and `VolumeVisual` issue **a single draw call per frame** — there are no per-tile state changes.

### Tile Streaming

Each frame, `VirtualTextureData::prepare()`:

1. Selects the ideal LOD for the current view (screen-space voxel error)
2. Identifies which tiles at that LOD intersect the visible region
3. For any tile that isn't resident in the atlas, calls the user-supplied `loader(lod, z, y, x)` callback
4. Uploads any tiles that have arrived since last frame (pushed by the loader via `set_chunk_data_u16()`) into the atlas and updates the page table

The loader runs on whatever async runtime the host provides (Python `ThreadPoolExecutor`, JS `fetch()`, etc.) and pushes results back via the `PendingChunks` queue.

### ImageVisual — Slice Rendering

`ImageVisual` renders a slice plane through the volume using polygon geometry:

- Per-frame the AABB of each visible tile is intersected with the slice plane to produce a convex polygon (up to 6 vertices)
- That polygon is uploaded to a vertex buffer and drawn with texture coordinates into the atlas
- The fragment shader (`virtual_tile.wgsl`) resolves the atlas address for each fragment via the page table and samples the 3D atlas texture

```rust
// Slice plane: point + normal
pub struct SlicePlane {
    pub position: [f32; 3],
    pub normal: [f32; 3],
}
```

Controls: `set_slice_plane()`, `set_contrast()`.

### VolumeVisual — Ray Marching DVR

`VolumeVisual` renders a unit-cube bounding box with **back-face culling inverted** (the box is inward-wound, so back faces are rendered — these are the ray exit faces). The fragment shader (`volume_raymarch.wgsl`):

1. Reconstructs the ray entry point from the fragment's clip position and the camera
2. Marches from entry to exit in world space, stepping one voxel at the current LOD per iteration
3. At each step, looks up the page table to find the atlas tile, samples the atlas, applies the contrast window, and evaluates the transfer function (colormap LUT)
4. Composites the sample front-to-back with premultiplied alpha; terminates early when opacity saturates

Because the page table encodes which LOD a tile was actually loaded at, the shader automatically falls back to coarser tiles mid-ray for regions that haven't streamed in yet.

Controls: `set_contrast()`, `set_density_scale()`, `set_relative_step_size()`, `set_colormap()`, `set_lod_bias()`. Debug modes: LOD tinting, atlas visualization, step-count heatmap.

### Bind Groups

Both visuals use three bind group slots:

| Group | Contents |
|-------|----------|
| 0 | Camera uniforms (set once per frame by `Renderer`, shared by all visuals) |
| 1 | VT resources: atlas texture, atlas sampler, page table texture, VT uniforms |
| 2 | Visual-specific: `ImageVisual` uses per-tile uniforms; `VolumeVisual` uses volume AABB + colormap LUT |

---

---

## Summary

All Bovista visuals follow a consistent pattern:

1. **Define vertex structure** - Layout matches shader inputs
2. **Create GPU resources** - Buffers, textures, uniforms
3. **Compile shader** - WGSL → GPU pipeline
4. **Configure pipeline** - Topology, depth, blending
5. **Implement Visual trait** - prepare() and render()

The key differences are:

- **Topology** - Points, Lines, or Triangles
- **Resources** - Simple (just vertices) vs complex (textures, uniforms)
- **Shaders** - Simple pass-through vs texture sampling vs custom

This architecture makes it easy to add new visual types while maintaining consistency and performance.
