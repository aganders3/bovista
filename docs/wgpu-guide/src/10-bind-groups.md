# Bind Groups Deep Dive

Bind groups are WGPU's mechanism for organizing and binding resources to shaders. Understanding them is key to efficient rendering.

## The Bind Group Hierarchy

Think of bind groups as **sets of resources** that you can bind together:

```
Pipeline
    ├─ Group 0: Global resources (camera, lighting)
    ├─ Group 1: Per-material resources (textures, material properties)
    ├─ Group 2: Per-object resources (model matrix, object ID)
    └─ Group 3: Per-frame resources (time, random seed)
```

## Best Practices

### Update Frequency Grouping

**Group resources by how often they change**:

```rust
// Group 0: Once per frame
@group(0) @binding(0) var<uniform> camera: CameraUniforms;
@group(0) @binding(1) var<uniform> lights: LightUniforms;

// Group 1: Per material (changes occasionally)
@group(1) @binding(0) var diffuse_texture: texture_2d<f32>;
@group(1) @binding(1) var normal_texture: texture_2d<f32>;
@group(1) @binding(2) var<uniform> material: MaterialUniforms;

// Group 2: Per object (changes every draw)
@group(2) @binding(0) var<uniform> model_matrix: mat4x4<f32>;
```

**Rendering loop**:
```rust
render_pass.set_bind_group(0, &camera_bind_group, &[]);  // Once

for material in materials {
    render_pass.set_bind_group(1, &material.bind_group, &[]);  // Per material

    for object in material.objects {
        render_pass.set_bind_group(2, &object.bind_group, &[]);  // Per object
        render_pass.draw(...);
    }
}
```

**Why?** Minimize bind group changes (expensive operation).

### Share Layouts

**All pipelines should share the same bind group layouts** for global resources:

```rust
// Create once, share everywhere
let camera_layout = create_camera_bind_group_layout(&device);

// All pipelines use the same layout
let points_pipeline = create_pipeline(&device, &camera_layout, /* ... */);
let lines_pipeline = create_pipeline(&device, &camera_layout, /* ... */);
let image_pipeline = create_pipeline(&device, &camera_layout, /* ... */);

// Single camera bind group works with all pipelines
let camera_bind_group = create_camera_bind_group(&device, &camera_layout, &camera_buffer);

render_pass.set_bind_group(0, &camera_bind_group, &[]);
render_pass.set_pipeline(&points_pipeline);  // Uses group 0
render_pass.draw(...);
render_pass.set_pipeline(&lines_pipeline);   // Still uses group 0!
render_pass.draw(...);
```

## Dynamic Offsets

For many similar objects, use **dynamic offsets**:

```rust
// Single large buffer for all objects
let uniform_buffer = device.create_buffer(&BufferDescriptor {
    size: 256 * num_objects,  // 256 bytes per object
    usage: BufferUsages::UNIFORM,
    // ...
});

// Layout with dynamic offset
let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
    entries: &[BindGroupLayoutEntry {
        binding: 0,
        ty: BindingType::Buffer {
            ty: BufferBindingType::Uniform,
            has_dynamic_offset: true,  // ← Enable dynamic offsets
            // ...
        },
        // ...
    }],
});

// Single bind group
let bind_group = device.create_bind_group(&BindGroupDescriptor {
    layout: &layout,
    entries: &[BindGroupEntry {
        binding: 0,
        resource: BindingResource::Buffer(BufferBinding {
            buffer: &uniform_buffer,
            offset: 0,
            size: Some(NonZeroU64::new(256).unwrap()),
        }),
    }],
});

// Render with different offsets
for (i, object) in objects.iter().enumerate() {
    let offset = (i * 256) as u32;
    render_pass.set_bind_group(0, &bind_group, &[offset]);  // Dynamic offset!
    render_pass.draw(...);
}
```

**Benefits**:
- One bind group for all objects
- Fast offset changes
- Better batching

## Storage Buffers vs Uniform Buffers

### Uniform Buffers

```rust
@group(0) @binding(0)
var<uniform> data: MyData;
```

**Limits**:
- Small size (typically 64 KB max)
- Fast access
- Must align to 256 bytes per dynamic offset

**Use for**: Small, frequently updated data (matrices, parameters)

### Storage Buffers

```rust
@group(0) @binding(0)
var<storage, read> data: array<MyData>;
```

**Limits**:
- Large size (typically 128 MB+)
- Slower than uniform buffers
- Can be read/write

**Use for**: Large arrays, instanced data

## Texture Arrays

Bind multiple textures at once:

```rust
// Shader
@group(1) @binding(0)
var textures: binding_array<texture_2d<f32>>;  // Array of textures!

@group(1) @binding(1)
var samplers: binding_array<sampler>;

// Sample specific texture
let color = textureSample(textures[texture_index], samplers[0], uv);
```

**Rust code**:
```rust
let layout = device.create_bind_group_layout(&BindGroupLayoutDescriptor {
    entries: &[BindGroupLayoutEntry {
        binding: 0,
        count: NonZeroU32::new(100),  // Array of 100 textures
        ty: BindingType::Texture { /* ... */ },
        // ...
    }],
});

let bind_group = device.create_bind_group(&BindGroupDescriptor {
    entries: &[BindGroupEntry {
        binding: 0,
        resource: BindingResource::TextureViewArray(&texture_views),  // Array!
    }],
});
```

**Benefit**: Bind many textures with one bind group, select in shader.

## Push Constants (Not in WGPU Yet)

**Vulkan/DX12 have "push constants"** for tiny amounts of data:

```rust
// Not available in WGPU yet, but may be added
push_constant: MyData,  // 128 bytes max, fastest possible
```

**Workaround**: Use small uniform buffers with dynamic offsets.

## Common Patterns

### Pattern 1: Material System

```rust
struct Material {
    bind_group: BindGroup,  // Contains all textures/uniforms
    pipeline: RenderPipeline,
}

// Render by material
for material in materials {
    render_pass.set_pipeline(&material.pipeline);
    render_pass.set_bind_group(1, &material.bind_group, &[]);

    for mesh in material.meshes {
        render_pass.draw(...);
    }
}
```

### Pattern 2: Bindless Rendering

Use texture arrays + indices:

```rust
// All textures in one bind group
@group(1) @binding(0)
var all_textures: binding_array<texture_2d<f32>, 1000>;

// Per-object data includes texture index
struct ObjectData {
    model_matrix: mat4x4<f32>,
    diffuse_index: u32,
    normal_index: u32,
    // ...
}

// Sample correct textures
let diffuse = textureSample(all_textures[obj.diffuse_index], sampler, uv);
let normal = textureSample(all_textures[obj.normal_index], sampler, uv);
```

**Benefit**: No bind group changes per draw call!

## Debugging Bind Groups

### Validation Errors

Common error:
```
Bind group doesn't match pipeline layout:
  Expected texture at binding 1, found sampler
```

**Fix**: Ensure bind group matches layout exactly.

### Check Limits

```rust
let limits = device.limits();
println!("Max bind groups: {}", limits.max_bind_groups);
println!("Max bindings per bind group: {}", limits.max_bindings_per_bind_group);
println!("Max uniform buffer size: {}", limits.max_uniform_buffer_binding_size);
```

## Summary

| Concept | Use Case |
|---------|----------|
| **Group 0** | Global resources (camera, lights) |
| **Group 1** | Per-material (textures) |
| **Group 2** | Per-object (transforms) |
| **Dynamic offsets** | Many similar objects |
| **Storage buffers** | Large arrays |
| **Texture arrays** | Many textures |
| **Uniform buffers** | Small, fast data |

**Key Takeaway**: Organize bind groups by update frequency to minimize state changes.

---

**Next**: [Code Reference →](./reference.md)
