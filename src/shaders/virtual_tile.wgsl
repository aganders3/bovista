// Virtual-texture tile shader
//
// Accepts a slice-plane intersection polygon (same TileVertex layout as tile.wgsl).
// The vertex `texcoord` carries the volume UV in [0,1]^3 (computed by
// compute_plane_aabb_intersection against the full-volume AABB).
//
// The fragment shader walks the LOD chain via the page table starting at the
// CPU-computed target_lod, falling back to coarser (then finer) as needed.
//
// Bind groups:
//   0 — camera (same as tile.wgsl)
//   1 — atlas, atlas_sampler, page_table, vt_uniforms
//   2 — colormap LUT (same as tile.wgsl)

// ── Group 0: camera ──────────────────────────────────────────────────────────

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// ── Group 1: virtual texture resources ───────────────────────────────────────

@group(1) @binding(0)
var atlas: texture_3d<f32>;

@group(1) @binding(1)
var atlas_sampler: sampler;

@group(1) @binding(2)
var page_table: texture_2d_array<u32>;

struct VTLodInfo {
    // Number of tiles (ceil) in (x, y, z) at this LOD — used for page-table index.
    grid_dims: vec3<u32>,
    _pad: u32,
    // tile_size / volume_size per axis (x, y, z) — used for exact sub-tile UV.
    tile_scale: vec3<f32>,
    _pad2: f32,
    // Fraction of atlas slot filled with data at this LOD = lod_tile_size / atlas_slot_size.
    // Coarser LODs have fewer voxels; multiply within_tile by this to stay in the valid region.
    data_scale: vec3<f32>,
    _pad3: f32,
}

struct VTUniforms {
    atlas_cols: u32,
    atlas_rows: u32,
    // UV advance per tile: 1/atlas_cols, 1/atlas_rows, 1/atlas_layers.
    atlas_tile_pitch_x: f32,
    atlas_tile_pitch_y: f32,
    atlas_tile_pitch_z: f32,
    lod_count: u32,
    contrast_min: f32,
    contrast_max: f32,
    // Non-zero enables debug LOD tinting (green=LOD0, red=coarsest).
    debug_mode: u32,
    // Width of the page table texture (for linear→2D index mapping).
    page_table_width: u32,
    // CPU-computed ideal LOD for this frame (accounts for lod_bias + camera distance).
    // The shader starts its page-table walk here rather than at LOD 0.
    target_lod: u32,
    _pad_c: u32,
    // Offset 48 — VTLodInfo has align 16, 48 mod 16 = 0 ✓
    lods: array<VTLodInfo, 16>,
}

@group(1) @binding(3)
var<uniform> vt: VTUniforms;

// ── Group 2: colormap LUT ────────────────────────────────────────────────────

@group(2) @binding(0)
var colormap: texture_1d<f32>;

@group(2) @binding(1)
var colormap_sampler: sampler;

// ── Vertex stage ─────────────────────────────────────────────────────────────

struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) texcoord: vec3<f32>,   // volume UV in [0,1]^3
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) vol_uv: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    out.clip_position = camera.view_proj * vec4f(input.position, 1.0);
    out.vol_uv = input.texcoord;
    return out;
}

// ── Fragment stage ────────────────────────────────────────────────────────────

// Manual linear→sRGB encoding for linear-format surfaces (Bgra8Unorm).
// Used when the GPU does not apply the conversion automatically (srgb_encode == 1).
fn linear_to_srgb_channel(c: f32) -> f32 {
    if c <= 0.0031308 { return c * 12.92; }
    return 1.055 * pow(c, 1.0 / 2.4) - 0.055;
}
fn encode_srgb(c: vec4f) -> vec4f {
    return vec4f(linear_to_srgb_channel(c.r),
                 linear_to_srgb_channel(c.g),
                 linear_to_srgb_channel(c.b),
                 c.a);
}

// Sample the atlas for the given LOD and vol_uv.
// Returns vec2f(raw_value, lod_f) if tile is resident, or vec2f(0, -1) if not.
fn try_lod(vol_uv: vec3f, lod: i32) -> vec2f {
    let grid = vt.lods[lod].grid_dims;
    let vol_in_tiles = min(vol_uv / vt.lods[lod].tile_scale, vec3f(grid) - vec3f(1e-5));
    let tc = clamp(vec3i(vol_in_tiles), vec3i(0), vec3i(grid) - vec3i(1));
    let linear = u32(tc.z) * grid.y * grid.x + u32(tc.y) * grid.x + u32(tc.x);
    let pt_x = i32(linear % vt.page_table_width);
    let pt_y = i32(linear / vt.page_table_width);
    let entry = textureLoad(page_table, vec2i(pt_x, pt_y), lod, 0).r;
    if (entry >> 24u) == 0u { return vec2f(0.0, -1.0); }
    let slot = entry & 0xFFFFu;
    let atlas_col   = slot % vt.atlas_cols;
    let atlas_row   = (slot / vt.atlas_cols) % vt.atlas_rows;
    let atlas_layer = slot / (vt.atlas_cols * vt.atlas_rows);

    // Scale within-tile UV to the populated region of the atlas slot.
    let within_tile = (vol_in_tiles - floor(vol_in_tiles)) * vt.lods[lod].data_scale;

    let u = (f32(atlas_col)   + within_tile.x) * vt.atlas_tile_pitch_x;
    let v = (f32(atlas_row)   + within_tile.y) * vt.atlas_tile_pitch_y;
    let w = (f32(atlas_layer) + within_tile.z) * vt.atlas_tile_pitch_z;

    // Read via textureLoad (not textureSampleLevel) — see volume_raymarch.wgsl
    // for the NVIDIA GL 3.30 compat bug rationale.
    let adim = vec3f(textureDimensions(atlas, 0));
    let acoord = vec3i(vec3f(u, v, w) * adim);
    return vec2f(textureLoad(atlas, acoord, 0).r, f32(lod));
}

// Walk the page table to find the best resident LOD for this fragment.
// Returns vec2f(raw_voxel_value, lod_index); lod_index = -1 if nothing resident.
//
// Strategy:
//   1. Start at the CPU-computed target_lod (which respects lod_bias and camera distance).
//   2. Walk coarser (target_lod+1, target_lod+2, …) — normal fallback while loading.
//   3. If still nothing, walk finer (target_lod-1, … 0) — use stale fine data rather
//      than showing black during initial load.
fn sample_vvt(vol_uv: vec3f) -> vec2f {
    let ideal_lod = i32(vt.target_lod);
    let max_lod = i32(vt.lod_count) - 1;

    // Pass 1: ideal_lod → coarsest (preferred direction)
    for (var lod = ideal_lod; lod <= max_lod; lod++) {
        let r = try_lod(vol_uv, lod);
        if r.y >= 0.0 { return r; }
    }
    // Pass 2: finer than ideal (fallback — tile is too detailed but beats black)
    for (var lod = ideal_lod - 1; lod >= 0; lod--) {
        let r = try_lod(vol_uv, lod);
        if r.y >= 0.0 { return r; }
    }
    return vec2f(0.0, -1.0);
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    let result  = sample_vvt(input.vol_uv);
    let raw     = result.x;
    let lod_idx = result.y;   // -1 if nothing resident

    let adjusted = clamp((raw - vt.contrast_min) / (vt.contrast_max - vt.contrast_min), 0.0, 1.0);
    let color    = textureLoad(colormap, i32(clamp((adjusted) * f32(textureDimensions(colormap, 0)), 0.0, f32(textureDimensions(colormap, 0)) - 1.0)), 0);

    var out: vec4f;
    if vt.debug_mode != 0u {
        // Compute LOD tint: green (LOD 0) → yellow → red (coarsest).
        // lod_t = 0 at LOD 0, 1 at the coarsest loaded LOD.
        let max_lod = f32(vt.lod_count) - 1.0;
        let lod_t   = select(0.5, lod_idx / max(max_lod, 1.0), lod_idx >= 0.0);
        let tint    = vec3f(clamp(lod_t * 2.0, 0.0, 1.0),
                            clamp(2.0 - lod_t * 2.0, 0.0, 1.0),
                            select(0.0, 1.0, lod_idx < 0.0));  // blue if not found
        // Show image data through the tint: tint provides hue, image provides brightness.
        out = vec4f(tint * (0.3 + 0.7 * adjusted), 1.0);
    } else {
        out = color;
    }

    return encode_srgb(out);
}
