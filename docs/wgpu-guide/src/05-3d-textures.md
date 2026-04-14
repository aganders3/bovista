# 3D Textures

3D textures (volume textures) are essential for volumetric data visualization. Let's see how they work in WGPU.

## Creating 3D Textures

From `src/visuals/chunk_visual.rs:92-125`:

```rust
let texture = device.create_texture(&wgpu::TextureDescriptor {
    label: Some("Chunk Texture"),
    size: wgpu::Extent3d {
        width,   // X dimension
        height,  // Y dimension
        depth_or_array_layers: depth,  // Z dimension
    },
    mip_level_count: 1,
    sample_count: 1,
    dimension: wgpu::TextureDimension::D3,  // ← 3D texture!
    format: wgpu::TextureFormat::R8Unorm,   // Single channel, 8-bit normalized
    usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
    view_formats: &[],
});
```

**Key points**:
- `dimension: D3` specifies 3D texture (vs `D2` for 2D)
- `depth_or_array_layers` is Z dimension for 3D textures
- `R8Unorm` = single channel, 0-255 maps to 0.0-1.0

**OpenGL equivalent**:
```c
GLuint texture;
glGenTextures(1, &texture);
glBindTexture(GL_TEXTURE_3D, texture);
glTexImage3D(GL_TEXTURE_3D, 0, GL_R8, width, height, depth, 0,
             GL_RED, GL_UNSIGNED_BYTE, NULL);
```

## Uploading Data

```rust
queue.write_texture(
    wgpu::ImageCopyTexture {
        texture: &texture,
        mip_level: 0,
        origin: wgpu::Origin3d::ZERO,
        aspect: wgpu::TextureAspect::All,
    },
    data,  // &[u8] slice
    wgpu::ImageDataLayout {
        offset: 0,
        bytes_per_row: Some(width),
        rows_per_image: Some(height),
    },
    texture_size,
);
```

**Memory layout**:
```
data = [
    // Z=0 plane
    row 0: pixel(0,0,0), pixel(1,0,0), ..., pixel(width-1,0,0),
    row 1: pixel(0,1,0), pixel(1,1,0), ..., pixel(width-1,1,0),
    ...
    row height-1: ...,

    // Z=1 plane
    row 0: pixel(0,0,1), pixel(1,0,1), ...,
    ...
]
```

**Parameters**:
- `bytes_per_row`: Must be multiple of 256 for optimal performance
- `rows_per_image`: Number of rows per Z-slice
- Order: X fastest, then Y, then Z

**OpenGL equivalent**:
```c
glTexImage3D(GL_TEXTURE_3D, 0, GL_R8, width, height, depth, 0,
             GL_RED, GL_UNSIGNED_BYTE, data);
```

## Texture Formats

Common formats:

| Format | Description | Use Case |
|--------|-------------|----------|
| `R8Unorm` | 8-bit grayscale | Medical images, microscopy |
| `R16Unorm` | 16-bit grayscale | High dynamic range |
| `Rgba8Unorm` | 8-bit color | Color volumes |
| `R32Float` | 32-bit float | Scientific data |

**Normalized formats** (`Unorm`): Integers map to [0, 1]
- `R8Unorm`: 0 → 0.0, 255 → 1.0
- `R16Unorm`: 0 → 0.0, 65535 → 1.0

**Float formats**: Store actual floating-point values

## Creating Samplers

Samplers control how textures are sampled:

```rust
let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
    address_mode_u: wgpu::AddressMode::ClampToEdge,
    address_mode_v: wgpu::AddressMode::ClampToEdge,
    address_mode_w: wgpu::AddressMode::ClampToEdge,  // Z dimension
    mag_filter: wgpu::FilterMode::Linear,   // Magnification (upscaling)
    min_filter: wgpu::FilterMode::Linear,   // Minification (downscaling)
    mipmap_filter: wgpu::FilterMode::Nearest,
    ..Default::default()
});
```

**Address modes**:
- `ClampToEdge`: Clamp coordinates to [0, 1]
- `Repeat`: Wrap coordinates (0.0 = 1.0 = 2.0, ...)
- `MirrorRepeat`: Mirror on each repeat

**Filter modes**:
- `Nearest`: No interpolation (pixelated)
- `Linear`: Trilinear interpolation (smooth)

**OpenGL equivalent**:
```c
glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_S, GL_CLAMP_TO_EDGE);
glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_T, GL_CLAMP_TO_EDGE);
glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_WRAP_R, GL_CLAMP_TO_EDGE);
glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MAG_FILTER, GL_LINEAR);
glTexParameteri(GL_TEXTURE_3D, GL_TEXTURE_MIN_FILTER, GL_LINEAR);
```

## Sampling in Shaders

From `tile.wgsl`:

```wgsl
@group(1) @binding(0)
var chunk_texture: texture_3d<f32>;

@group(1) @binding(1)
var chunk_sampler: sampler;

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Sample at 3D coordinate [0, 1]³
    let tex_value = textureSample(chunk_texture, chunk_sampler, input.texcoord).r;

    // tex_value is now in [0, 1] range
    return vec4<f32>(tex_value, tex_value, tex_value, 1.0);
}
```

**Trilinear interpolation** happens automatically with `Linear` filter:
```
Sample at (0.5, 0.5, 0.5):
  1. Sample 8 corner voxels
  2. Interpolate along X (4 values)
  3. Interpolate along Y (2 values)
  4. Interpolate along Z (1 value)
  Result: smooth gradient
```

## Texture Coordinates

Texture coordinates are normalized to [0, 1]³:

```
(0, 0, 0) = corner of volume
(1, 1, 1) = opposite corner
(0.5, 0.5, 0.5) = center
```

**Computing texture coordinates** (from world space):
```rust
let texcoord = Vec3::new(
    (world_pos.x - min.x) / (max.x - min.x),
    (world_pos.y - min.y) / (max.y - min.y),
    (world_pos.z - min.z) / (max.z - min.z),
);
```

## Bind Group Setup

Bind the texture and sampler together:

```rust
let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
    layout: &layout,
    entries: &[
        wgpu::BindGroupEntry {
            binding: 0,
            resource: wgpu::BindingResource::TextureView(&texture_view),
        },
        wgpu::BindGroupEntry {
            binding: 1,
            resource: wgpu::BindingResource::Sampler(&sampler),
        },
    ],
});
```

## Texture Size Limits

Check device limits:

```rust
let limits = device.limits();
println!("Max 3D texture size: {}³", limits.max_texture_dimension_3d);
```

Typical limits:
- Desktop: 2048³ or 4096³
- Mobile: 512³ or 1024³
- WebGPU: Varies by browser

**For larger volumes**: Use chunked approach (like TiledImageVisual)!

## Performance Tips

### 1. Use Smaller Formats When Possible

```rust
// 8-bit: 1 GB for 1024³ volume
format: TextureFormat::R8Unorm,

// 16-bit: 2 GB for 1024³ volume
format: TextureFormat::R16Unorm,

// 32-bit: 4 GB for 1024³ volume
format: TextureFormat::R32Float,
```

### 2. Align `bytes_per_row`

```rust
// Not aligned (slower)
bytes_per_row: Some(width),

// Aligned to 256 (faster)
bytes_per_row: Some(((width + 255) / 256) * 256),
```

### 3. Use Mipmaps for Downsampling

```rust
let texture = device.create_texture(&TextureDescriptor {
    mip_level_count: ((size as f32).log2().floor() as u32) + 1,
    // ...
});

// Generate mipmaps
for level in 1..mip_level_count {
    // Copy and downsample from level-1 to level
}
```

### 4. Cache Texture Views

```rust
// Create view once
let texture_view = texture.create_view(&Default::default());

// Reuse in bind groups
```

## Example: Loading from Zarr

```python
# Python side: Load chunk from Zarr
def load_chunk(z, y, x):
    chunk_data = zarr_array[
        z * chunk_size[0]:(z + 1) * chunk_size[0],
        y * chunk_size[1]:(y + 1) * chunk_size[1],
        x * chunk_size[2]:(x + 1) * chunk_size[2],
    ]
    return np.asarray(chunk_data, dtype=np.uint8)

# Rust side receives numpy array, uploads to 3D texture
let texture = create_3d_texture(device, data, width, height, depth);
```

## Debugging 3D Textures

**Visualize slices**:
```wgsl
// Sample only one Z slice
let z = 0.5;  // Middle slice
let tex_value = textureSample(tex, samp, vec3<f32>(input.uv.x, input.uv.y, z)).r;
```

**Visualize as maximum intensity projection (MIP)**:
```wgsl
// Sample multiple Z slices, keep maximum
var max_value = 0.0;
for (var z = 0.0; z < 1.0; z += 0.01) {
    let value = textureSample(tex, samp, vec3<f32>(uv.x, uv.y, z)).r;
    max_value = max(max_value, value);
}
return vec4<f32>(max_value, max_value, max_value, 1.0);
```

## Summary

| Aspect | 2D Texture | 3D Texture |
|--------|------------|------------|
| **Dimension** | `D2` | `D3` |
| **Size** | `(width, height)` | `(width, height, depth)` |
| **Coordinates** | `(u, v)` | `(u, v, w)` |
| **Memory** | width × height × bytes | width × height × depth × bytes |
| **Interpolation** | Bilinear | Trilinear |
| **Address mode** | `address_mode_u/v` | `address_mode_u/v/w` |

**Key Takeaway**: 3D textures are straightforward in WGPU—just specify `dimension: D3` and use 3D coordinates. Memory is the main constraint for large volumes.

---

**Next**: [Key Takeaways →](./08-key-takeaways.md)
