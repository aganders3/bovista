# Code Reference

Quick reference for common WGPU operations.

## Buffers

### Create Vertex Buffer

```rust
let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some("Vertex Buffer"),
    contents: bytemuck::cast_slice(&vertices),
    usage: wgpu::BufferUsages::VERTEX,
});
```

### Create Index Buffer

```rust
let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
    label: Some("Index Buffer"),
    contents: bytemuck::cast_slice(&indices),
    usage: wgpu::BufferUsages::INDEX,
});
```

### Create Uniform Buffer

```rust
let uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    label: Some("Uniform Buffer"),
    size: std::mem::size_of::<Uniforms>() as u64,
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    mapped_at_creation: false,
});
```

### Update Buffer

```rust
queue.write_buffer(&buffer, 0, bytemuck::cast_slice(&data));
```

## Textures

### Create 2D Texture

```rust
let texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("Texture"),
    size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D2,
    format: wgpu::TextureFormat::Rgba8UnormSrgb,
    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    view_formats: &[],
});
```

### Create 3D Texture

```rust
let texture = device.create_texture(&wgpu::TextureDescriptor {
    size: wgpu::Extent3d { width, height, depth_or_array_layers: depth },
    dimension: wgpu::TextureDimension::D3,  // ← 3D!
    format: wgpu::TextureFormat::R8Unorm,
    // ... rest same as 2D
});
```

### Upload Texture Data

```rust
queue.write_texture(
    wgpu::ImageCopyTexture {
        texture: &texture,
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    },
    &data,
    wgpu::ImageDataLayout {
        offset: 0,
        bytes_per_row: Some(width * 4),  // 4 bytes per pixel (RGBA)
        rows_per_image: Some(height),
    },
    texture_size,
);
```

### Create Sampler

```rust
let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
    address_mode_u: wgpu::AddressMode::ClampToEdge,
    address_mode_v: wgpu::AddressMode::ClampToEdge,
    address_mode_w: wgpu::AddressMode::ClampToEdge,
    mag_filter: wgpu::FilterMode::Linear,
    min_filter: wgpu::FilterMode::Linear,
    mipmap_filter: wgpu::FilterMode::Nearest,
    ..Default::default()
});
```

## Bind Groups

### Create Bind Group Layout

```rust
let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
    label: Some("Bind Group Layout"),
    entries: &[
        wgpu::BindGroupLayoutEntry {
            binding: 0,
            visibility: wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Buffer {
                ty: wgpu::BufferBindingType::Uniform,
                has_dynamic_offset: false,
                min_binding_size: None,
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 1,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                multisampled: false,
                view_dimension: wgpu::TextureViewDimension::D2,
                sample_type: wgpu::TextureSampleType::Float { filterable: true },
            },
            count: None,
        },
        wgpu::BindGroupLayoutEntry {
            binding: 2,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
            count: None,
        },
    ],
});
```

### Create Bind Group

```rust
let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    label: Some("Bind Group"),
    layout: &layout,
    entries: &[
        wgpu::BindGroupEntry {
            binding: 0,
            resource: uniform_buffer.as_entire_binding(),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::TextureView(&texture_view),
        },
        wgpu::BindGroupEntry {
            binding: 2,
            resource: wgpu::BindingResource::Sampler(&sampler),
        },
    ],
});
```

## Pipelines

### Create Render Pipeline

```rust
let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    label: Some("Render Pipeline"),
    layout: Some(&pipeline_layout),
    vertex: wgpu::VertexState {
        module: &shader,
        entry_point: Some("vs_main"),
        buffers: &[vertex_buffer_layout],
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
        front_face: wgpu::FrontFace::Ccw,
        cull_mode: Some(wgpu::Face::Back),
        polygon_mode: wgpu::PolygonMode::Fill,
        ..Default::default()
    },
    depth_stencil: Some(wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth32Float,
        depth_write_enabled: true,
        depth_compare: wgpu::CompareFunction::Less,
        stencil: wgpu::StencilState::default(),
        bias: wgpu::DepthBiasState::default(),
    }),
    multisample: wgpu::MultisampleState::default(),
    multiview: None,
    cache: None,
});
```

## Rendering

### Begin Render Pass

```rust
let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    label: Some("Render Pass"),
    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
        view: &view,
        resolve_target: None,
        ops: wgpu::Operations {
            load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
            store: wgpu::StoreOp::Store,
        },
    })],
    depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
        view: &depth_view,
        depth_ops: Some(wgpu::Operations {
            load: wgpu::LoadOp::Clear(1.0),
            store: wgpu::StoreOp::Store,
        }),
        stencil_ops: None,
    }),
    timestamp_writes: None,
    occlusion_query_set: None,
});
```

### Draw Calls

```rust
// Non-indexed
render_pass.draw(0..vertex_count, 0..1);

// Indexed
render_pass.draw_indexed(0..index_count, 0, 0..1);

// Instanced
render_pass.draw_indexed(0..index_count, 0, 0..instance_count);
```

## Bovista-Specific

### File Locations

| Component | File |
|-----------|------|
| Renderer | `src/renderer.rs` |
| Camera | `src/camera.rs` |
| Scene | `src/scene.rs` |
| Visual trait | `src/visual.rs` |
| Shared binding logic | `src/bindings_common.rs` |
| Points | `src/visuals/points.rs` |
| Lines | `src/visuals/lines.rs` |
| Image (slice rendering) | `src/visuals/image.rs` |
| Volume (ray marching DVR) | `src/visuals/volume.rs` |
| Virtual texture (atlas + page table + LOD) | `src/visuals/virtual_texture.rs` |
| Atlas allocator | `src/visuals/atlas.rs` |
| Page table | `src/visuals/page_table.rs` |
| Tile types and uniforms | `src/visuals/tile.rs` |
| Custom shaders | `src/visuals/custom.rs` |
| Python bindings | `src/python.rs` |
| WASM bindings | `src/wasm.rs` |

### Shaders

| Shader | File | Purpose |
|--------|------|---------|
| Virtual tile | `src/shaders/virtual_tile.wgsl` | Slice rendering via atlas + page table |
| Volume ray march | `src/shaders/volume_raymarch.wgsl` | DVR ray marching via atlas + page table |
| Point cloud | `src/shaders/point_cloud.wgsl` | Point rendering |
| Lines | `src/shaders/lines.wgsl` | Line rendering |

---

**Next**: [Further Reading →](./further-reading.md)
