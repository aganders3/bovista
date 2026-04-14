# Pipeline Creation

Render pipelines in WGPU are **immutable state objects** that configure the entire rendering workflow. Let's break down pipeline creation using Bovista's `ChunkVisual` as an example.

## OpenGL vs WGPU Pipelines

### OpenGL: Mutable State

In OpenGL, you change state as needed:

```c
// Compile shaders
GLuint vs = glCreateShader(GL_VERTEX_SHADER);
glShaderSource(vs, 1, &vs_source, NULL);
glCompileShader(vs);

GLuint fs = glCreateShader(GL_FRAGMENT_SHADER);
// ... compile fragment shader ...

// Link program
GLuint program = glCreateProgram();
glAttachShader(program, vs);
glAttachShader(program, fs);
glLinkProgram(program);

// Later, change state anytime
glUseProgram(program);
glEnable(GL_DEPTH_TEST);
glDepthFunc(GL_LESS);
glEnable(GL_BLEND);
glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);

// Change depth function for next draw
glDepthFunc(GL_LEQUAL);
```

### WGPU: Immutable Pipelines

In WGPU, **all state is baked into the pipeline**. To change state, you create a new pipeline:

```rust
let pipeline = device.create_render_pipeline(&RenderPipelineDescriptor {
    // Shader
    vertex: VertexState { /* ... */ },
    fragment: Some(FragmentState { /* ... */ }),

    // Rasterization
    primitive: PrimitiveState { /* ... */ },

    // Depth/stencil
    depth_stencil: Some(DepthStencilState { /* ... */ }),

    // Blending
    // (in fragment targets)

    // Multisampling
    multisample: MultisampleState { /* ... */ },

    // Bind groups
    layout: Some(&pipeline_layout),
});
```

**Key insight**: Once created, the pipeline is **immutable**. Switching pipelines is fast—drivers can optimize ahead of time.

## Step-by-Step Pipeline Creation

Let's walk through creating the chunk rendering pipeline from `src/visuals/chunk_visual.rs:236-290`.

### Step 1: Load Shader Module

```rust
let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
    label: Some("Tile Shader"),
    source: wgpu::ShaderSource::Wgsl(include_str!("../shaders/tile.wgsl").into()),
});
```

**OpenGL equivalent:**
```c
const char* source = load_file("tile.vert");
GLuint shader = glCreateShader(GL_VERTEX_SHADER);
glShaderSource(shader, 1, &source, NULL);
glCompileShader(shader);
```

**Key differences:**
- WGPU shaders are compiled at runtime (like OpenGL)
- Validation happens immediately—syntax errors caught here
- One shader module can contain both vertex and fragment stages (WGSL)

### Step 2: Create Pipeline Layout

The pipeline layout describes **which bind groups** the pipeline expects:

```rust
let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
    label: Some("Chunk Pipeline Layout"),
    bind_group_layouts: &[camera_bind_group_layout, &bind_group_layout],
    push_constant_ranges: &[],
});
```

**What this means:**
> "This pipeline expects two bind groups:
> - Group 0: Camera (from renderer)
> - Group 1: Chunk-specific resources (texture, sampler, uniforms)"

**When rendering, you MUST provide matching bind groups:**
```rust
render_pass.set_bind_group(0, &camera_bind_group, &[]);  // Group 0
render_pass.set_bind_group(1, &chunk_bind_group, &[]);   // Group 1
```

**OpenGL has no equivalent**—you just bind things globally.

### Step 3: Vertex State

Define vertex shader entry point and vertex buffer layout:

```rust
vertex: wgpu::VertexState {
    module: &shader,
    entry_point: Some("vs_main"),  // Function name in shader
    buffers: &[ChunkVertex::desc()],  // Vertex buffer layout
    compilation_options: Default::default(),
},
```

The `ChunkVertex::desc()` describes the vertex format (`src/visuals/chunk_visual.rs:23-29`):

```rust
fn desc() -> wgpu::VertexBufferLayout<'static> {
    wgpu::VertexBufferLayout {
        array_stride: std::mem::size_of::<ChunkVertex>() as wgpu::BufferAddress,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 0,
                shader_location: 0,  // @location(0) in shader
            },
            wgpu::VertexAttribute {
                format: wgpu::VertexFormat::Float32x3,
                offset: 12,
                shader_location: 1,  // @location(1) in shader
            },
        ],
    }
}
```

**OpenGL equivalent:**
```c
// Vertex position
glVertexAttribPointer(0, 3, GL_FLOAT, GL_FALSE, sizeof(ChunkVertex), (void*)0);
glEnableVertexAttribArray(0);

// Texture coordinate
glVertexAttribPointer(1, 3, GL_FLOAT, GL_FALSE, sizeof(ChunkVertex), (void*)12);
glEnableVertexAttribArray(1);
```

**Key points:**
- `array_stride`: Bytes per vertex (like OpenGL stride)
- `step_mode`: `Vertex` = per vertex, `Instance` = per instance
- `attributes`: List of vertex attributes with format and offset
- `shader_location`: Matches `@location(N)` in shader

### Step 4: Fragment State

```rust
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
```

**Key components:**

1. **`format`**: Must match render target format
   ```rust
   format: wgpu::TextureFormat::Bgra8UnormSrgb,  // Common for displays
   ```

2. **`blend`**: Blending mode (optional)
   ```rust
   // Alpha blending: src.rgb * src.a + dst.rgb * (1 - src.a)
   blend: Some(wgpu::BlendState::ALPHA_BLENDING),

   // Or custom:
   blend: Some(wgpu::BlendState {
       color: wgpu::BlendComponent {
           src_factor: wgpu::BlendFactor::SrcAlpha,
           dst_factor: wgpu::BlendFactor::OneMinusSrcAlpha,
           operation: wgpu::BlendOperation::Add,
       },
       alpha: wgpu::BlendComponent::REPLACE,
   }),
   ```

3. **`write_mask`**: Which color channels to write
   ```rust
   write_mask: wgpu::ColorWrites::RED | wgpu::ColorWrites::GREEN,  // RG only
   ```

**OpenGL equivalent:**
```c
glEnable(GL_BLEND);
glBlendFunc(GL_SRC_ALPHA, GL_ONE_MINUS_SRC_ALPHA);
glColorMask(GL_TRUE, GL_TRUE, GL_TRUE, GL_TRUE);
```

### Step 5: Primitive State

Configure rasterization behavior:

```rust
primitive: wgpu::PrimitiveState {
    topology: wgpu::PrimitiveTopology::TriangleList,  // GL_TRIANGLES
    strip_index_format: None,
    front_face: wgpu::FrontFace::Ccw,  // Counter-clockwise
    cull_mode: None,  // No back-face culling
    polygon_mode: wgpu::PolygonMode::Fill,
    unclipped_depth: false,
    conservative: false,
},
```

**Topology options:**
- `TriangleList`: Each 3 vertices = 1 triangle (most common)
- `TriangleStrip`: Vertices form a strip
- `LineList`: Each 2 vertices = 1 line
- `PointList`: Each vertex = 1 point

**OpenGL equivalent:**
```c
glFrontFace(GL_CCW);
glDisable(GL_CULL_FACE);
glPolygonMode(GL_FRONT_AND_BACK, GL_FILL);
```

### Step 6: Depth & Stencil State

```rust
depth_stencil: Some(wgpu::DepthStencilState {
    format: wgpu::TextureFormat::Depth32Float,
    depth_write_enabled: true,
    depth_compare: wgpu::CompareFunction::Less,
    stencil: wgpu::StencilState::default(),
    bias: wgpu::DepthBiasState::default(),
}),
```

**Compare functions:**
- `Less`: Pass if new < existing (most common)
- `LessEqual`: Pass if new ≤ existing
- `Greater`: Pass if new > existing
- `Always`: Always pass (effectively disables depth test)
- `Never`: Never pass

**OpenGL equivalent:**
```c
glEnable(GL_DEPTH_TEST);
glDepthFunc(GL_LESS);
glDepthMask(GL_TRUE);
```

**Important**: Must provide depth texture with matching format when rendering!

### Step 7: Multisample State

```rust
multisample: wgpu::MultisampleState {
    count: 1,  // No MSAA (1 sample per pixel)
    mask: !0,  // All samples enabled
    alpha_to_coverage_enabled: false,
},
```

For 4x MSAA:
```rust
multisample: wgpu::MultisampleState {
    count: 4,
    mask: !0,
    alpha_to_coverage_enabled: false,
},
```

**OpenGL equivalent:**
```c
// 1x (no MSAA)
glDisable(GL_MULTISAMPLE);

// 4x MSAA
glEnable(GL_MULTISAMPLE);
// (configured when creating framebuffer)
```

## Full Pipeline Example

Here's the complete pipeline creation from `chunk_visual.rs:248-290`:

```rust
let render_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("Chunk Render Pipeline"),
    layout: Some(&pipeline_layout),

    // Vertex stage
    vertex: wgpu::VertexState {
        module: &shader,
        entry_point: Some("vs_main"),
        buffers: &[ChunkVertex::desc()],
        compilation_options: Default::default(),
    },

    // Fragment stage
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

    // Rasterization
    primitive: wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::TriangleList,
        strip_index_format: None,
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: None,
        polygon_mode: wgpu::PolygonMode::Fill,
        unclipped_depth: false,
        conservative: false,
    },

    // Depth testing
    depth_stencil: Some(wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: true,
        depth_compare: wgpu::CompareFunction::Less,
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }),

    // Multisampling
    multisample: wgpu::MultisampleState {
        count: 1,
        mask: !0,
        alpha_to_coverage_enabled: false,
    },

    multiview: None,
    cache: None,
});
```

## Using the Pipeline

To render with the pipeline:

```rust
render_pass.set_pipeline(&render_pipeline);
render_pass.set_bind_group(0, &camera_bind_group, &[]);
render_pass.set_bind_group(1, &chunk_bind_group, &[]);
render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
render_pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint16);
render_pass.draw_indexed(0..index_count, 0, 0..1);
```

## Pipeline Variants

To render with different state (e.g., different blend mode), create multiple pipelines:

```rust
// Pipeline 1: Alpha blending
let alpha_pipeline = create_pipeline(BlendState::ALPHA_BLENDING);

// Pipeline 2: Additive blending
let additive_pipeline = create_pipeline(BlendState {
    color: BlendComponent {
        src_factor: BlendFactor::One,
        dst_factor: BlendFactor::One,
        operation: BlendOperation::Add,
    },
    alpha: BlendComponent::REPLACE,
});

// Switch between them
render_pass.set_pipeline(&alpha_pipeline);
render_pass.draw(...);

render_pass.set_pipeline(&additive_pipeline);
render_pass.draw(...);
```

## Common Pitfalls

❌ **Mismatched formats**
```rust
// Pipeline expects Bgra8Unorm
let pipeline = create_render_pipeline(format: Bgra8Unorm);

// But render target is Rgba8Unorm
render_pass.begin(...);  // ERROR!
```

✅ **Match all formats**
```rust
// Pipeline, surface, and render target must all match
```

❌ **Forgetting depth texture**
```rust
depth_stencil: Some(...),  // Pipeline expects depth

// But no depth attachment
encoder.begin_render_pass(&RenderPassDescriptor {
    depth_stencil_attachment: None,  // ERROR!
});
```

✅ **Provide depth texture**
```rust
depth_stencil_attachment: Some(RenderPassDepthStencilAttachment {
    view: &depth_view,
    // ...
}),
```

## Best Practices

1. **Create pipelines at startup**: Pipeline creation is expensive
2. **Reuse pipelines**: Store in a cache/map
3. **Minimize pipeline switches**: Group draws by pipeline
4. **Use pipeline cache**: (future feature) speeds up creation
5. **Label pipelines**: Makes debugging easier

## Summary

| Aspect | OpenGL | WGPU |
|--------|--------|------|
| **Mutability** | Mutable state | Immutable pipelines |
| **State Changes** | `glEnable`, `glDepthFunc`, etc. | Create new pipeline |
| **Validation** | At draw time | At pipeline creation |
| **Optimization** | Driver tries to optimize | Pre-optimized |
| **Switching Cost** | Expensive state changes | Fast pipeline switches |

**Key Takeaway**: WGPU pipelines are more verbose to create but faster to use. The immutability enables aggressive driver optimizations.

---

**Next**: [Shaders: WGSL vs GLSL →](./04-shaders.md)
