# Initialization & Setup

Let's walk through initializing WGPU and setting up the core rendering infrastructure. We'll use Bovista's `Renderer` as a practical example.

## The Initialization Sequence

### OpenGL Initialization (Recap)

```c
// OpenGL: Create context, make current, initialize
GLFWwindow* window = glfwCreateWindow(800, 600, "App", NULL, NULL);
glfwMakeContextCurrent(window);
gladLoadGLLoader((GLADloadproc)glfwGetProcAddress);

// Now you can use OpenGL
GLuint vbo;
glGenBuffers(1, &vbo);
```

### WGPU Initialization

WGPU requires more setup, but gives you explicit control:

```rust
// 1. Create instance (connection to graphics backend)
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::PRIMARY,  // Vulkan, Metal, DX12, or WebGPU
    ..Default::default()
});

// 2. Request adapter (physical GPU)
let adapter = instance
    .request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,  // Or pass a surface for rendering to window
        force_fallback_adapter: false,
    })
    .await
    .unwrap();

// 3. Request device and queue
let (device, queue) = adapter
    .request_device(
        &wgpu::DeviceDescriptor {
            label: Some("Main Device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
        },
        None,
    )
    .await
    .unwrap();
```

**What's happening:**
- **Instance**: Loads graphics backend (like initializing OpenGL context)
- **Adapter**: Represents your GPU (can query capabilities, features)
- **Device**: Logical GPU connection (creates resources)
- **Queue**: Command submission (sends work to GPU)

## Creating Buffers

### Uniform Buffers

Let's create a uniform buffer for camera data. From `src/renderer.rs:69-74`:

```rust
let camera_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("Camera Uniform Buffer"),
    size: std::mem::size_of::<CameraUniforms>() as u64,
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
});
```

**Key points:**
- `label`: Debug name (shows up in GPU debuggers like RenderDoc)
- `size`: Must be exact size in bytes
- `usage`: **Immutable flags** declaring how buffer will be used
  - `UNIFORM`: Can be bound as uniform buffer
  - `COPY_DST`: Can be written to via `queue.write_buffer()`
- `mapped_at_creation`: Whether to map memory immediately (for initial data)

**OpenGL equivalent:**
```c
GLuint ubo;
glGenBuffers(1, &ubo);
glBindBuffer(GL_UNIFORM_BUFFER, ubo);
glBufferData(GL_UNIFORM_BUFFER, sizeof(CameraUniforms), NULL, GL_DYNAMIC_DRAW);
```

### Vertex Buffers

Creating vertex buffers is similar, but with different usage flags:

```rust
let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some("Vertex Buffer"),
    contents: bytemuck::cast_slice(&vertices),
    usage: wgpu::BufferUsages::VERTEX,
});
```

**Note**: `create_buffer_init` is a helper that uploads data immediately. Equivalent to `mapped_at_creation: true`.

## Bind Group Layouts: The Blueprint

This is **WGPU's most important concept**. Think of it as defining an interface.

From `src/renderer.rs:76-89`:

```rust
let camera_bind_group_layout = device.create_bind_group_layout(
    &wgpu::BindGroupLayoutDescriptor {
        label: Some("Camera Bind Group Layout"),
        entries: &[wgpu::BindGroupLayoutEntry {
            binding: 0,                              // @binding(0) in shader
            visibility: wgpu::ShaderStages::VERTEX,  // Only vertex shader sees it
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,  // Not an array
        }],
    }
);
```

**What this says:**
> "Bind group 0 will have one resource: a uniform buffer at binding 0, visible only to the vertex shader."

**OpenGL analogy:**
```c
// This is like declaring a uniform block in GLSL:
layout(std140, binding = 0) uniform CameraUniforms {
    mat4 view_proj;
};
```

But in WGPU, you define the layout **in code** so pipelines can reference it.

## Bind Groups: The Instance

Now create the actual bind group with real resources (`src/renderer.rs:91-98`):

```rust
let camera_bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("Camera Bind Group"),
    layout: &camera_bind_group_layout,
    entries: &[wgpu::BindGroupEntry {
        binding: 0,
        resource: camera_uniform_buffer.as_entire_binding(),
    }],
});
```

**What this says:**
> "Here's a bind group matching that layout, with the actual `camera_uniform_buffer` at binding 0."

**Relationship:**
```
BindGroupLayout (blueprint)  →  Multiple BindGroups (instances)
     ↓
  Camera, Texture, Sampler    →  BindGroup1 (uses BufferA, TextureX)
                               →  BindGroup2 (uses BufferB, TextureY)
                               →  BindGroup3 (uses BufferC, TextureZ)
```

**OpenGL analogy:**
```c
// Layout = declaring the uniform block
layout(std140, binding = 0) uniform CameraUniforms { /* ... */ };

// Bind group = binding a specific UBO to that binding point
glBindBufferBase(GL_UNIFORM_BUFFER, 0, camera_ubo);
```

## Updating Buffers

### Writing Uniform Data

From `src/renderer.rs:110-119`:

```rust
pub fn update_camera(&self, camera: &Camera) {
    let uniforms = CameraUniforms {
        view_proj: camera.view_projection_matrix().to_cols_array_2d(),
    };
    self.queue.write_buffer(
        &self.camera_uniform_buffer,
        0,  // Offset
        bytemuck::cast_slice(&[uniforms]),
    );
}
```

**Key points:**
- `queue.write_buffer()`: Enqueues a write operation (happens on GPU timeline)
- `bytemuck::cast_slice()`: Safely converts struct to bytes
- No need to bind buffer first (unlike OpenGL)

**OpenGL equivalent:**
```c
glBindBuffer(GL_UNIFORM_BUFFER, camera_ubo);
glBufferSubData(GL_UNIFORM_BUFFER, 0, sizeof(uniforms), &uniforms);
```

## Depth Textures

Creating depth buffers is similar to OpenGL, but more explicit (`src/renderer.rs:121-138`):

```rust
pub fn create_depth_texture(&self, width: u32, height: u32) -> wgpu::TextureView {
    let depth_texture = self.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("Depth Texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Depth32Float,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    });

    depth_texture.create_view(&wgpu::TextureViewDescriptor::default())
}
```

**Key points:**
- `dimension`: `D2` for 2D texture (vs `D3` for volumetric)
- `format`: `Depth32Float` = 32-bit floating-point depth
- `usage`: Must declare both `RENDER_ATTACHMENT` (for depth testing) and `TEXTURE_BINDING` (if you want to read depth)
- Returns a `TextureView` (not raw texture) for rendering

**OpenGL equivalent:**
```c
GLuint depth_texture;
glGenTextures(1, &depth_texture);
glBindTexture(GL_TEXTURE_2D, depth_texture);
glTexImage2D(GL_TEXTURE_2D, 0, GL_DEPTH_COMPONENT32F, width, height, 0,
             GL_DEPTH_COMPONENT, GL_FLOAT, NULL);
```

## The Renderer Pattern

Bovista's `Renderer` struct encapsulates all shared GPU resources:

```rust
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    camera_uniform_buffer: wgpu::Buffer,
    camera_bind_group: wgpu::BindGroup,
    camera_bind_group_layout: wgpu::BindGroupLayout,
    surface_format: wgpu::TextureFormat,
}
```

**Why this pattern?**
- **Centralized resource management**: One place owns GPU resources
- **Shared bind groups**: Camera bind group used by all visuals
- **Surface format**: All pipelines need matching format
- **Accessor methods**: Safe access to device/queue

## Usage Pattern

```rust
// Create renderer
let renderer = Renderer::new(device, queue, surface_format).await;

// Update camera each frame
renderer.update_camera(&camera);

// Create depth texture (on resize)
let depth_view = renderer.create_depth_texture(width, height);

// Visuals access renderer resources
let visual = PointsVisual::new(
    renderer.device(),
    renderer.surface_format(),
    renderer.camera_bind_group_layout(),  // Share this layout!
    vertices,
);

// Render frame
renderer.render(&scene, &view, &depth_view, clear_color);
```

## Best Practices

1. **Label everything**: Makes debugging 10x easier
2. **Declare usage upfront**: Can't change buffer usage after creation
3. **Share bind group layouts**: All visuals should use same camera layout
4. **Centralize resources**: Renderer pattern keeps things organized
5. **Use `bytemuck`**: Safe casting between Rust types and GPU bytes

## Common Pitfalls

❌ **Forgetting to declare usage**
```rust
let buffer = device.create_buffer(&BufferDescriptor {
    usage: BufferUsages::VERTEX,  // Missing COPY_DST!
    // ...
});
queue.write_buffer(&buffer, 0, data);  // ERROR: Can't write to buffer
```

✅ **Include all needed usages**
```rust
let buffer = device.create_buffer(&BufferDescriptor {
    usage: BufferUsages::VERTEX | BufferUsages::COPY_DST,
    // ...
});
```

❌ **Mismatched bind group layout**
```rust
// Pipeline expects binding 0 to be a buffer
let pipeline = create_pipeline(&layout);

// But bind group provides a texture at binding 0
let bind_group = device.create_bind_group(...);  // ERROR!
```

✅ **Match layout exactly**
```rust
// Layout and bind group must match perfectly
```

## Summary

| Concept | OpenGL | WGPU |
|---------|--------|------|
| **GPU Access** | Context | Instance → Adapter → Device |
| **Buffers** | `glGenBuffers` | `device.create_buffer()` |
| **Usage Flags** | Mutable (`glBufferData`) | Immutable (`BufferUsages`) |
| **Resource Binding** | Bind by ID | Bind groups with layouts |
| **Updates** | `glBufferSubData` | `queue.write_buffer()` |

**Key Takeaway**: WGPU initialization is more verbose but gives you explicit control and compile-time safety.

---

**Next**: [Pipeline Creation →](./03-pipelines.md)
