# The wgpu Rendering Pipeline

A ground-up walkthrough of how Bovista draws pixels, from GPU initialization through shader execution. If you're new to wgpu or WebGPU, start here.

---

## Core Concepts

wgpu is a Rust library exposing the **WebGPU API**, a modern, explicit graphics API. "Explicit" means you describe memory layouts in advance, bind resources before drawing, and specify exactly how vertices flow into fragments. The cost is upfront setup; the payoff is predictable performance and one codebase running on Vulkan, Metal, DirectX 12, and WebGPU in the browser.

The GPU executes a **render pipeline** for each draw call:

```
CPU uploads vertex data + uniforms
        │
        ▼
  [ Vertex Stage ]      ← runs once per vertex; outputs clip-space position
        │
        ▼
  [ Rasterization ]     ← GPU converts triangles to fragments (pixels)
        │
        ▼
  [ Fragment Stage ]    ← runs once per fragment; outputs a color
        │
        ▼
  [ Output Merging ]    ← depth test, blending, write to texture
```

Shaders are small programs written in WGSL (WebGPU Shading Language). Each pipeline has one vertex shader and one fragment shader.

Rather than setting individual resource pointers before every draw, wgpu groups related resources into **bind groups** — a bundle of buffers, textures, and samplers with a fixed layout attached to the pipeline once. Bovista uses a consistent three-group layout across all visual pipelines:

| Group | Who owns it | Contents |
|-------|------------|----------|
| `@group(0)` | `Renderer` (set once per frame for all visuals) | Camera view-projection matrix |
| `@group(1)` | Each visual (VT visuals) | Atlas texture, page table, VT uniforms |
| `@group(2)` | Each visual | Per-visual uniforms and colormap |

Simple visuals (points, lines) only use group 0.

The resources themselves are **buffers** (raw bytes: vertex data, index data, uniform structs), **textures** (sampled image data with filtering: atlas, page table, colormap LUT), and **samplers** (which describe how to filter a texture and are bound alongside it).

---

## The Renderer: Setting Up the GPU

`src/renderer.rs` owns the GPU device, queue, and the shared camera bind group. `Device` creates GPU objects; `Queue` sends data and commands to the GPU.

```rust
// In practice, these come from wgpu surface setup (not shown here).
// The device is used to create GPU objects (buffers, textures, pipelines).
// The queue is used to submit commands and write data to GPU memory.
pub struct Renderer {
    device: wgpu::Device,
    queue: wgpu::Queue,
    ...
}
```

Every shader transforms vertices from world space to screen space. The renderer creates a 64-byte uniform buffer holding a single 4×4 matrix and updates it every frame. `UNIFORM` means the shader can read it; `COPY_DST` means the CPU can write to it via `queue.write_buffer`:

```rust
let camera_uniform_buffer = device.create_buffer(&wgpu::BufferDescriptor {
    size: std::mem::size_of::<CameraUniforms>() as u64,  // 64 bytes
    usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
    ...
});
```

The bind group layout declares "binding 0 is a uniform buffer, visible only in the vertex stage", matching the WGSL declaration:

```rust
wgpu::BindGroupLayoutEntry {
    binding: 0,
    visibility: wgpu::ShaderStages::VERTEX,
    ty: wgpu::BindingType::Buffer {
        ty: wgpu::BufferBindingType::Uniform,
        ...
    },
    ...
}
```

```wgsl
// virtual_tile.wgsl, group 0
struct CameraUniforms {
    view_proj: mat4x4<f32>,
}
@group(0) @binding(0)
var<uniform> camera: CameraUniforms;
```

Each frame, `Renderer::update_camera` computes the view-projection matrix and sends it to the GPU. `bytemuck::cast_slice` safely reinterprets the Rust struct as raw bytes:

```rust
pub fn update_camera(&self, camera: &Camera) {
    let uniforms = CameraUniforms {
        view_proj: camera.view_projection_matrix().to_cols_array_2d(),
    };
    self.queue.write_buffer(&self.camera_uniform_buffer, 0,
        bytemuck::cast_slice(&[uniforms]));
}
```

Bovista also creates a depth texture at startup and recreates it on resize. The depth buffer stores the depth of the closest fragment at each pixel; an arriving fragment further away is discarded (the depth test). This makes closer objects correctly occlude farther ones without sorting draw calls.

```rust
let depth_texture = device.create_texture(&wgpu::TextureDescriptor {
    format: wgpu::TextureFormat::Depth24PlusStencil8,
    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
    ...
});
```

Each frame, `Renderer::render` creates a **command encoder** (a recorder for GPU commands), begins a **render pass** (declaring the target texture and clear behavior), sets the camera bind group at `@group(0)` **once** for all visuals, calls `scene.render()` (which iterates each visible `impl Visual` and calls its `render()`), then finishes and submits the command buffer:

```rust
// From renderer.rs
let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
    color_attachments: &[Some(wgpu::RenderPassColorAttachment {
        view,  // the surface texture we're drawing into
        ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear_color), ... },
    })],
    depth_stencil_attachment: Some(...),
    ...
});

render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
scene.render(&mut render_pass);  // each visual sets groups 1, 2 and draws
```

---

## Simple Visuals: Points and Lines

`src/visuals/points.rs` and `src/visuals/lines.rs` show the minimal pipeline pattern. A point has a world-space position and an RGB color:

```rust
#[repr(C)]
#[derive(Copy, Clone, Pod, Zeroable)]
pub struct PointVertex {
    pub position: [f32; 3],  // 12 bytes
    pub color:    [f32; 3],  // 12 bytes — total: 24 bytes/vertex
}
```

The `wgpu::VertexBufferLayout` tells the pipeline what this struct looks like. The attribute numbers (`0 =>`, `1 =>`) match the `@location(0)` and `@location(1)` annotations in the WGSL vertex input struct:

```rust
wgpu::VertexBufferLayout {
    array_stride: 24,  // advance 24 bytes per vertex
    step_mode: wgpu::VertexStepMode::Vertex,
    attributes: &wgpu::vertex_attr_array![
        0 => Float32x3,  // location 0: 3 floats = position
        1 => Float32x3,  // location 1: 3 floats = color
    ],
}
```

`device.create_render_pipeline` is the big upfront declaration:

```rust
device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
    layout: Some(&pipeline_layout),  // which bind group layouts this pipeline uses

    vertex: wgpu::VertexState {
        module: &shader,         // compiled WGSL module
        entry_point: Some("vs_main"),  // the function to call
        buffers: &[PointVertex::desc()],  // vertex buffer layout
    },

    fragment: Some(wgpu::FragmentState {
        module: &shader,
        entry_point: Some("fs_main"),
        targets: &[Some(wgpu::ColorTargetState {
            format: surface_format,  // must match the render pass target
            blend: Some(wgpu::BlendState::REPLACE),  // no transparency
            ...
        })],
    }),

    primitive: wgpu::PrimitiveState {
        topology: wgpu::PrimitiveTopology::PointList,  // draw points, not triangles
        cull_mode: None,
        ...
    },

    depth_stencil: Some(wgpu::DepthStencilState {
        format: wgpu::TextureFormat::Depth24PlusStencil8,
        depth_write_enabled: true,
        depth_compare: wgpu::CompareFunction::LessEqual,
        ...
    }),
})
```

A pipeline is **immutable** once created — topology, shader, and vertex layout are all baked in. This is intentional: the driver compiles the pipeline to native GPU code at creation time, making draw calls fast.

The shader, `point_cloud.wgsl`:

```wgsl
struct Uniforms { view_proj: mat4x4<f32> }
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) color: vec3<f32>,
}
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,  // built-in: goes to rasterizer
    @location(0) color: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // Multiply world position by view-projection matrix to get clip space position.
    out.clip_position = uniforms.view_proj * vec4<f32>(input.position, 1.0);
    out.color = input.color;
    return out;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    return vec4<f32>(input.color, 1.0);  // RGBA; alpha=1 = opaque
}
```

`@builtin(position)` is understood by the rasterizer as the clip-space position. Other outputs (like `color`) are interpolated across the primitive by the hardware and handed to the fragment shader.

The `Visual` trait splits work into two phases. **`prepare()`** runs before the render pass and can write GPU buffers (`queue.write_buffer`), create textures, and compute geometry — for points and lines it's a no-op since vertex data never changes. **`render()`** runs inside the render pass and can only issue draw commands (`set_pipeline`, `set_bind_group`, `draw`). The split exists because `queue.write_buffer` and render pass recording are incompatible — you can't upload data mid-pass.

---

## The Slice Pipeline: Image

`src/visuals/image.rs` renders a planar cross-section through a 3D volume using the virtual texture system. Each frame, `prepare()` computes where the slice plane intersects the volume AABB and generates a polygon mesh (3–6 vertices, depending on slice angle). Each vertex carries a **3D volume UV** (`texcoord: vec3<f32>`), the normalized position within the volume. The fragment shader uses those UVs to look up the voxel value via the page table, then passes it through a contrast window and a colormap LUT to produce the final color.

```rust
#[repr(C)]
pub struct TileVertex {
    pub position: [f32; 3],   // world-space position (on the slice plane)
    pub texcoord: [f32; 3],   // volume UV in [0,1]^3
}
```

The hardware interpolates `texcoord` across the polygon, so every fragment knows where in the volume it sits.

`compute_plane_aabb_intersection` (in `src/visuals/tile.rs`) tests the slice plane against all 12 edges of the volume bounding box. Each intersected edge yields one vertex, giving a convex polygon of 3–6 vertices, triangulated as a fan and uploaded fresh every frame:

```rust
// From Image::prepare()
if let Some((vertices, indices)) =
    compute_plane_aabb_intersection(&tile_plane, &vol_aabb)
{
    self.vt_vertex_buffer = Some(device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
        contents: bytemuck::cast_slice(&vertices),
        usage: wgpu::BufferUsages::VERTEX,
        ...
    }));
    // ... also creates index buffer
}
```

Group 1 (VT resources), declared in `Image::new`, has four entries:

| Binding | Type | Shader access | Purpose |
|---------|------|---------------|---------|
| 0 | `texture_3d<f32>` | Fragment | Physical atlas (packed tile data) |
| 1 | `sampler` | Fragment | How to filter the atlas |
| 2 | `texture_2d_array<u32>` | Fragment | Page table (tile → atlas slot mapping) |
| 3 | uniform buffer | Fragment | VT uniforms: atlas layout, LOD info, contrast |

```rust
// Entry for the page table (binding 2)
wgpu::BindGroupLayoutEntry {
    binding: 2,
    visibility: wgpu::ShaderStages::FRAGMENT,
    ty: wgpu::BindingType::Texture {
        sample_type: wgpu::TextureSampleType::Uint,  // integer texture, not float
        view_dimension: wgpu::TextureViewDimension::D2Array,  // one layer per LOD
        multisampled: false,
    },
    ...
}
```

The page table is a `texture_2d_array` with one array layer per LOD level. `TextureSampleType::Uint` means you read raw 32-bit integers with `textureLoad` — no filtering, since you want exact values, not interpolated ones.

Group 2 (colormap) has two entries:

| Binding | Type | Purpose |
|---------|------|---------|
| 0 | `texture_1d<f32>` | 256-entry RGBA colormap LUT |
| 1 | `sampler` | Linear filter for smooth colormap interpolation |

In `virtual_tile.wgsl`, the vertex stage is trivial — multiply world position by view-projection and pass the UV through:

```wgsl
@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4f(input.position, 1.0);
    out.vol_uv = input.texcoord;  // passed through to fragment stage
    return out;
}
```

The fragment stage does the work — sample, contrast, colormap:

```wgsl
@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // 1. Find the best resident tile for this volume UV
    let result  = sample_vvt(input.vol_uv);
    let raw     = result.x;      // voxel value (R16Float)
    let lod_idx = result.y;      // which LOD level was found (-1 if none)

    // 2. Apply contrast window: remap [contrast_min, contrast_max] → [0, 1]
    let adjusted = clamp(
        (raw - vt.contrast_min) / (vt.contrast_max - vt.contrast_min),
        0.0, 1.0
    );

    // 3. Colormap lookup: 1D texture indexed by adjusted intensity
    let color = textureSample(colormap, colormap_sampler, adjusted);

    return encode_srgb(color);  // manual sRGB encoding for linear-format surfaces
}
```

The key function `sample_vvt(vol_uv)` walks the LOD pyramid for a resident tile — from the ideal LOD up to coarsest (normal fallback while tiles load), then finer than ideal (stale fine data beats black):

```wgsl
fn sample_vvt(vol_uv: vec3f) -> vec2f {
    let ideal_lod = i32(vt.target_lod);  // CPU-computed based on camera distance
    let max_lod   = i32(vt.lod_count) - 1;

    // First try: ideal_lod up to coarsest (normal fallback while tiles load)
    for (var lod = ideal_lod; lod <= max_lod; lod++) {
        let r = try_lod(vol_uv, lod);
        if r.y >= 0.0 { return r; }  // r.y >= 0 means tile was resident
    }
    // Second try: finer than ideal (stale fine data beats black)
    for (var lod = ideal_lod - 1; lod >= 0; lod--) {
        let r = try_lod(vol_uv, lod);
        if r.y >= 0.0 { return r; }
    }
    return vec2f(0.0, -1.0);  // nothing resident
}
```

`try_lod` maps volume UV → page table address → atlas slot → texture sample:

```wgsl
fn try_lod(vol_uv: vec3f, lod: i32) -> vec2f {
    // 1. Which tile grid cell does this UV fall in?
    let grid = vt.lods[lod].grid_dims;
    let vol_in_tiles = min(vol_uv / vt.lods[lod].tile_scale, vec3f(grid) - vec3f(1e-5));
    let tc = clamp(vec3i(vol_in_tiles), vec3i(0), vec3i(grid) - vec3i(1));

    // 2. Linearize 3D tile coordinate to a 1D page table index
    let linear = u32(tc.z) * grid.y * grid.x + u32(tc.y) * grid.x + u32(tc.x);
    let pt_x = i32(linear % vt.page_table_width);
    let pt_y = i32(linear / vt.page_table_width);

    // 3. Read the page table entry (integer texture, so textureLoad not textureSample)
    let entry = textureLoad(page_table, vec2i(pt_x, pt_y), lod, 0).r;

    // Bit 24 is the "resident" flag; bits 0-15 are the atlas slot index
    if (entry >> 24u) == 0u { return vec2f(0.0, -1.0); }  // not resident

    // 4. Decode atlas slot → (col, row, layer) in the 3D atlas texture
    let slot      = entry & 0xFFFFu;
    let atlas_col   = slot % vt.atlas_cols;
    let atlas_row   = (slot / vt.atlas_cols) % vt.atlas_rows;
    let atlas_layer = slot / (vt.atlas_cols * vt.atlas_rows);

    // 5. Compute UV within this atlas slot, then scale to full-atlas UV
    let within_tile = (vol_in_tiles - floor(vol_in_tiles)) * vt.lods[lod].data_scale;
    let u = (f32(atlas_col)   + within_tile.x) * vt.atlas_tile_pitch_x;
    let v = (f32(atlas_row)   + within_tile.y) * vt.atlas_tile_pitch_y;
    let w = (f32(atlas_layer) + within_tile.z) * vt.atlas_tile_pitch_z;

    // 6. Sample the 3D atlas texture
    return vec2f(textureSampleLevel(atlas, atlas_sampler, vec3f(u, v, w), 0.0).r,
                 f32(lod));
}
```

`tile_scale` is `tile_size / volume_size` per axis — what fraction of the full volume UV one tile spans. `data_scale` handles coarser LOD tiles that have fewer voxels than the atlas slot is sized for (the extra space is unused padding; `data_scale` clamps sampling to the populated region).

`Image::render` issues the draw. `draw_indexed` draws triangles using a u16 index buffer, reusing vertices across triangles (the fan triangulation):

```rust
// From Image::render()
fn render(&self, render_pass: &mut RenderPass) {
    if self.vt_index_count == 0 { return; }  // slice missed the volume
    if let (Some(vb), Some(ib)) = (&self.vt_vertex_buffer, &self.vt_index_buffer) {
        render_pass.set_pipeline(&self.vt_render_pipeline);
        render_pass.set_bind_group(1, &self.vt_bind_group, &[]);    // atlas, page table, uniforms
        render_pass.set_bind_group(2, &self.colormap_bind_group, &[]);
        render_pass.set_vertex_buffer(0, vb.slice(..));
        render_pass.set_index_buffer(ib.slice(..), wgpu::IndexFormat::Uint16);
        render_pass.draw_indexed(0..self.vt_index_count, 0, 0..1);
        // Note: group 0 (camera) is already set by Renderer before this is called.
    }
}
```

---

## The Volume Pipeline: DirectVolume

`src/visuals/volume.rs` renders the entire 3D volume via GPU ray marching. Five volume classes share this back-end — `DirectVolume` (default DVR), `MipVolume`, `MinipVolume`, `AverageVolume`, and `IsosurfaceVolume`. The virtual texture back-end (atlas, page table, LOD streaming) is identical to `Image`.

Instead of rasterizing every voxel, the volume uses **proxy geometry**: draw the back faces of a unit cube covering the volume AABB, reconstruct the camera ray through each fragment's world-space position, and march that ray through the volume sampling voxels front-to-back. Back faces are used because the rasterizer then fires a fragment for every pixel the volume occupies in screen space, even when the camera is inside the volume (where front faces would be clipped by the near plane):

```rust
// Volume box: 12 triangles, CW winding = inward-facing normals
const BOX_INDICES: &[u16] = &[ /* 36 indices for all 6 faces */ ];

primitive: wgpu::PrimitiveState {
    cull_mode: Some(wgpu::Face::Front),  // cull front-facing = keep back-facing
    ...
}
```

The vertex stage positions the cube and passes world position for ray reconstruction:

```wgsl
@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    // BOX_VERTICES are in [0,1]^3; scale them to the actual AABB in world space.
    let world = vol.vol_min + in.position * (vol.vol_max - vol.vol_min);
    out.clip_position = camera.view_proj * vec4f(world, 1.0);
    out.world_pos = world;  // pass world position for ray reconstruction in fragment
    return out;
}
```

The fragment shader (`volume_raymarch.wgsl`) does the heavy lifting. It reconstructs the ray, runs an AABB slab test to find entry/exit, then steps along the ray accumulating color front-to-back (**alpha compositing**):

```wgsl
@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // Ray from camera through this fragment's world position (on the back face of the box)
    let ray_origin = vol.camera_pos;
    let ray_dir    = normalize(in.world_pos - vol.camera_pos);

    // AABB slab test: find where the ray enters and exits the volume
    let inv_dir = 1.0 / ray_dir;
    let t_min_v = (vol.vol_min - ray_origin) * inv_dir;
    let t_max_v = (vol.vol_max - ray_origin) * inv_dir;
    let t1 = min(t_min_v, t_max_v);
    let t2 = max(t_min_v, t_max_v);
    let t_enter = max(max(t1.x, t1.y), t1.z);  // ray enters volume here
    let t_exit  = min(min(t2.x, t2.y), t2.z);  // ray exits volume here

    if t_exit <= max(t_enter, 0.0) { discard; }  // ray misses or is behind camera
    ...
```

```wgsl
    var accum_color = vec3f(0.0);
    var accum_alpha = 0.0;
    var t = max(t_enter, 0.0);

    loop {
        if t >= t_exit || accum_alpha >= 0.95 { break; }

        let world_p = ray_origin + ray_dir * t;
        let vol_uv  = (world_p - vol.vol_min) / (vol.vol_max - vol.vol_min);

        // Same page-table lookup as the slice shader
        let result = sample_vvt(vol_uv);
        let raw    = result.x;
        let lod_f  = result.y;

        let adjusted = clamp(
            (raw - vt.contrast_min) / (vt.contrast_max - vt.contrast_min),
            0.0, 1.0
        );
        let cs = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);

        // Beer-Lambert opacity: alpha = 1 - exp(-extinction)
        if adjusted > 0.01 {
            let extinction = cs.a * vol.density_scale * advance;
            let alpha      = 1.0 - exp(-extinction);
            accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;  // front-to-back
            accum_alpha += (1.0 - accum_alpha) * alpha;
        }

        t += advance;  // advance is LOD-adaptive (larger steps in coarser regions)
    }
```

The `accum_alpha >= 0.95` early-out stops once the ray is 95% opaque, since further samples add under 5% to the final color. The step size is LOD-adaptive: in unloaded regions the shader jumps to the next tile boundary instead of crawling one LOD-0 voxel at a time:

```wgsl
if lod_f >= 0.0 {
    // Resident tile: step size proportional to the LOD scale ratio
    advance = step_size * vt.lods[lod_i].tile_scale.x / vt.lods[0].tile_scale.x;
} else {
    // Not resident: compute distance to the nearest coarsest-LOD tile boundary and jump
    let coarse_scale = vt.lods[coarsest].tile_scale;
    // ... (slab test against tile grid boundaries)
    advance = max(min(dt_to_bound.x, min(dt_to_bound.y, dt_to_bound.z)), step_size);
}
```

Volume visuals use `wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING` (not the simple alpha blending used by `Image`). The ray marcher outputs `out = (accum_color * accum_alpha, accum_alpha)`; pre-multiplied alpha composes correctly when multiple transparent layers overlap, which matters when volumes and slices render together.

Volume depth writes are disabled. The volume reads the depth buffer (so opaque geometry occludes it) but doesn't write — otherwise its back-face fragments would block any geometry behind the volume:

```rust
depth_stencil: Some(wgpu::DepthStencilState {
    depth_write_enabled: false,  // transparent volume: don't block other visuals
    depth_compare: wgpu::CompareFunction::LessEqual,
    ...
})
```

---

## VT Uniforms: The CPU-GPU Contract

`VTUniforms` (in `src/visuals/tile.rs`) is the main structure shared between CPU and VT shaders. Every field must match between Rust and WGSL exactly, including padding:

```rust
#[repr(C)]  // C-compatible layout — field order is preserved
#[derive(Pod, Zeroable)]  // bytemuck: safe to cast to/from bytes
pub struct VTUniforms {
    pub atlas_cols: u32,
    pub atlas_rows: u32,
    pub atlas_tile_pitch_x: f32,  // 1/atlas_cols
    pub atlas_tile_pitch_y: f32,  // 1/atlas_rows
    pub atlas_tile_pitch_z: f32,  // 1/atlas_layers
    pub lod_count: u32,
    pub contrast_min: f32,
    pub contrast_max: f32,
    pub debug_mode: u32,
    pub page_table_width: u32,
    pub target_lod: u32,
    pub _pad_c: u32,          // explicit padding to reach 48-byte offset
    pub lods: [VTLodInfo; 16],  // per-LOD data (grid dims, tile scale, data scale)
}
```

WGSL uniform structs require 16-byte alignment for sub-structs (`VTLodInfo`). The `_pad_c` field ensures `lods` starts at offset 48. Getting this wrong produces subtle visual corruption with no error — the GPU silently reads the wrong bytes.

---

## Shader Selection Summary

| Visual | Shader | Topology | Cull mode | Depth write | Blend |
|--------|--------|----------|-----------|-------------|-------|
| `Points` | `point_cloud.wgsl` | PointList | None | Yes | Replace |
| `Lines` | `lines.wgsl` | LineList | None | Yes | Replace |
| `Image` | `virtual_tile.wgsl` | TriangleList | None | Yes | Alpha |
| `DirectVolume` (and siblings) | `volume_raymarch.wgsl` | TriangleList | Front | No | Pre-multiplied Alpha |

One frame, step by step:

```
1. CPU: camera.view_projection_matrix() → queue.write_buffer(camera_uniform)

2. CPU (each visual's prepare()):
   - Image: recompute slice polygon geometry, upload vertex/index buffers,
            write VTUniforms to vt_uniform_buffer
   - DirectVolume: write VTUniforms + VolumeUniforms to GPU buffers
   - Points: no-op (geometry is static)

3. GPU: begin_render_pass (clears color and depth)
   render_pass.set_bind_group(0, camera_bind_group)  ← once for all visuals

4. GPU (each visual's render()):
   - Points:
       set_pipeline(point_pipeline)
       set_vertex_buffer(point_vb)
       draw(0..N)

   - Image:
       set_pipeline(vt_pipeline)
       set_bind_group(1, vt_bind_group)    ← atlas, page table, uniforms
       set_bind_group(2, colormap_bind_group)
       set_vertex_buffer(0, slice_polygon_vb)
       set_index_buffer(slice_polygon_ib)
       draw_indexed(0..index_count)

   - DirectVolume:
       set_pipeline(volume_pipeline)
       set_bind_group(1, vt_bind_group)
       set_bind_group(2, vol_bind_group)   ← vol uniforms + colormap
       set_vertex_buffer(0, box_vb)
       set_index_buffer(box_ib)
       draw_indexed(0..36)  ← 12 triangles, back faces only

5. GPU: end render pass → submit → present to screen
```

The entire visible result — slice planes, volume, overlaid point clouds — comes from this single render pass.
