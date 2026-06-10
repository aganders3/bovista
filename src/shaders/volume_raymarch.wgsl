// Volume ray-marching shader
//
// Renders a volume by marching rays through the AABB. The bounding box back faces
// are rasterised; each fragment reconstructs its ray from camera_pos → world_pos,
// does an AABB slab test to find the entry/exit points, then marches forward.
//
// Tile data is looked up via the same virtual-texture page table as the slice shader.
// try_lod / sample_vvt are duplicated verbatim from virtual_tile.wgsl.
//
// Bind groups:
//   0 — camera (view_proj matrix; shared with all other visuals)
//   1 — atlas, atlas_sampler, page_table, vt_uniforms, vol_uniforms
//   2 — colormap, colormap_sampler
//
// Multiple fragment entry points, one render pipeline per mode:
//   fs_translucent    — front-to-back alpha compositing (Beer-Lambert, default).
//                       Also hosts debug modes 1/2/3 via vol.debug_mode.
//   fs_additive       — additive accumulation of colormapped contributions.
//   fs_mip            — Maximum Intensity Projection (attenuation = 0 degenerates
//                       to plain MIP; vol.attenuation > 0 weights near-camera).
//   fs_minip          — Minimum Intensity Projection.
//   fs_average        — Mean intensity along the ray.
//   fs_iso            — First-hit isosurface with Phong shading and a sub-voxel
//                       bisection refinement against the smooth (manual trilinear)
//                       field; gradient stencil sized to the resident LOD.

struct VTLodInfo {
    grid_dims: vec3<u32>,
    _pad: u32,
    tile_scale: vec3<f32>,
    _pad2: f32,
    data_scale: vec3<f32>,
    _pad3: f32,
    data_offset: vec3<f32>,
    _pad4: f32,
}

struct VTUniforms {
    atlas_cols: u32,
    atlas_rows: u32,
    atlas_tile_pitch_x: f32,
    atlas_tile_pitch_y: f32,
    atlas_tile_pitch_z: f32,
    lod_count: u32,
    contrast_min: f32,
    contrast_max: f32,
    debug_mode: u32,
    page_table_width: u32,
    target_lod: u32,
    desired_t: u32,
    lods: array<VTLodInfo, 16>,
}

struct VolumeUniforms {
    vol_min: vec3<f32>,
    /// Step size in LOD-0 voxels (1.0 = one LOD-0 voxel per step at fine regions).
    /// At coarser LODs the advance scales up proportionally, so a value of 1 always samples
    /// once per equivalent voxel regardless of which LOD is resident at a given position.
    relative_step_size: f32,
    vol_max: vec3<f32>,
    /// Global extinction coefficient (opacity per world-space unit).
    /// Adjust together with colormap alpha to control volume density.
    density_scale: f32,
    camera_pos: vec3<f32>,
    /// Stop accumulating once alpha exceeds this (front-to-back early exit).
    /// 0.95 is a good default; lower saves steps in dense regions, higher is more accurate.
    early_exit_alpha: f32,
    /// Full-volume voxel dimensions at LOD 0 (x, y, z).
    lod0_dims: vec3<u32>,
    /// 0=normal DVR, 1=LOD tint + tile wireframe, 2=atlas direct,
    /// 3=step heatmap. Only honoured by fs_translucent.
    debug_mode: u32,
    /// Isosurface threshold in contrast-normalised raw units (0..1). fs_iso.
    iso_threshold: f32,
    /// Attenuated-MIP falloff per accumulated normalised density. fs_mip.
    attenuation: f32,
    _pad0: f32,
    _pad1: f32,
}

// ── Group 0: camera ──────────────────────────────────────────────────────────

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0) var<uniform> camera: CameraUniforms;

// ── Group 1: virtual texture resources + volume state ────────────────────────
//
// 4 atlas textures are always bound (MAX_ATLAS_COUNT). Unused entries are
// 1×1×1 dummies on the CPU side; the shader's switch on atlas_id never
// indexes them at runtime. This lets a single visual hold up to
// MAX_ATLAS_COUNT × per-resource-cap of voxel data — Chrome's ~2 GiB cap
// otherwise limits one atlas regardless of how much VRAM the GPU has.

@group(1) @binding(0) var atlas0: texture_3d<f32>;
@group(1) @binding(1) var atlas1: texture_3d<f32>;
@group(1) @binding(2) var atlas2: texture_3d<f32>;
@group(1) @binding(3) var atlas3: texture_3d<f32>;
@group(1) @binding(4) var atlas_sampler: sampler;
@group(1) @binding(5) var page_table: texture_2d_array<u32>;
@group(1) @binding(6) var<uniform> vt: VTUniforms;
@group(1) @binding(7) var<uniform> vol: VolumeUniforms;

fn sample_atlas(atlas_id: u32, uv: vec3f) -> f32 {
    switch atlas_id {
        case 1u: { return textureSampleLevel(atlas1, atlas_sampler, uv, 0.0).r; }
        case 2u: { return textureSampleLevel(atlas2, atlas_sampler, uv, 0.0).r; }
        case 3u: { return textureSampleLevel(atlas3, atlas_sampler, uv, 0.0).r; }
        default: { return textureSampleLevel(atlas0, atlas_sampler, uv, 0.0).r; }
    }
}

// ── Group 2: colormap LUT ────────────────────────────────────────────────────

// 2D 256×1 instead of 1D 256 — works around a naga GLSL bug where 1D
// Rgba8Unorm textures sampled on NVIDIA GL 4.6 zero out the RGB channels
// (alpha is preserved).
@group(2) @binding(0) var colormap: texture_2d<f32>;
@group(2) @binding(1) var colormap_sampler: sampler;

// ── Vertex stage ──────────────────────────────────────────────────────────────

struct VertexInput {
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
}

@vertex
fn vs_main(in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    let world = vol.vol_min + in.position * (vol.vol_max - vol.vol_min);
    out.clip_position = camera.view_proj * vec4f(world, 1.0);
    out.world_pos = world;
    return out;
}

// ── sRGB encoding (for linear-format surfaces) ────────────────────────────────

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

// ── Page-table sampling (mirrored from virtual_tile.wgsl) ─────────────────────

fn try_lod(vol_uv: vec3f, lod: i32) -> vec2f {
    let grid = vt.lods[lod].grid_dims;
    // Clamp to [0, grid - ε) before computing frac. Without this, when vol_uv = 1.0 and
    // the volume dimension is an exact multiple of tile_w, vol_in_tiles lands on an integer
    // N, frac(N) = 0, and the shader samples the START of the last tile instead of the END.
    // The ε is well under one texel (tile_w ≥ 16, so one texel ≈ 0.06), no visible shift.
    let vol_in_tiles = min(vol_uv / vt.lods[lod].tile_scale, vec3f(grid) - vec3f(1e-5));
    let tc = clamp(vec3i(vol_in_tiles), vec3i(0), vec3i(grid) - vec3i(1));
    let linear = u32(tc.z) * grid.y * grid.x + u32(tc.y) * grid.x + u32(tc.x);
    let pt_x = i32(linear % vt.page_table_width);
    let pt_y = i32(linear / vt.page_table_width);
    let entry = textureLoad(page_table, vec2i(pt_x, pt_y), lod, 0).r;
    // bit 31 = resident; 29-30 = atlas_id (2 bits); 16-28 = t (13 bits); 0-15 = slot.
    // Not-resident OR wrong-t → tell caller to walk to coarser LOD.
    let resident = (entry >> 31u) & 1u;
    let atlas_id = (entry >> 29u) & 0x3u;
    let slot_t   = (entry >> 16u) & 0x1FFFu;
    if resident == 0u || slot_t != vt.desired_t { return vec2f(0.0, -1.0); }
    let slot = entry & 0xFFFFu;
    let atlas_col   = slot % vt.atlas_cols;
    let atlas_row   = (slot / vt.atlas_cols) % vt.atlas_rows;
    let atlas_layer = slot / (vt.atlas_cols * vt.atlas_rows);

    // voxel_frac in [0, 1] within the tile → atlas-slot UV in
    // [0.5/slot, (slot - 0.5)/slot]. The 0.5 inset on both ends puts
    // the sample at the centre of the first / last texels, avoiding
    // Nearest-filter tie-breaks at the slot boundary.
    let within_tile = vt.lods[lod].data_offset
                    + (vol_in_tiles - floor(vol_in_tiles)) * vt.lods[lod].data_scale;

    let u = (f32(atlas_col)   + within_tile.x) * vt.atlas_tile_pitch_x;
    let v = (f32(atlas_row)   + within_tile.y) * vt.atlas_tile_pitch_y;
    let w = (f32(atlas_layer) + within_tile.z) * vt.atlas_tile_pitch_z;

    return vec2f(sample_atlas(atlas_id, vec3f(u, v, w)), f32(lod));
}

fn sample_vvt(vol_uv: vec3f) -> vec2f {
    let ideal_lod = i32(vt.target_lod);
    let max_lod   = i32(vt.lod_count) - 1;

    for (var lod = ideal_lod; lod <= max_lod; lod++) {
        let r = try_lod(vol_uv, lod);
        if r.y >= 0.0 { return r; }
    }
    for (var lod = ideal_lod - 1; lod >= 0; lod--) {
        let r = try_lod(vol_uv, lod);
        if r.y >= 0.0 { return r; }
    }
    return vec2f(0.0, -1.0);
}

// ── Shared ray-setup helpers ──────────────────────────────────────────────────

struct RaySetup {
    t_start: f32,
    t_exit: f32,
    ray_origin: vec3f,
    ray_dir: vec3f,
    vol_extent: vec3f,
    step_size: f32,
    inv_tile0_scale_x: f32,
    hit: bool,
}

fn setup_ray(world_pos: vec3f) -> RaySetup {
    var s: RaySetup;
    s.ray_origin = vol.camera_pos;
    s.ray_dir    = normalize(world_pos - vol.camera_pos);

    let inv_dir = 1.0 / s.ray_dir;
    let t_min_v = (vol.vol_min - s.ray_origin) * inv_dir;
    let t_max_v = (vol.vol_max - s.ray_origin) * inv_dir;
    let t1 = min(t_min_v, t_max_v);
    let t2 = max(t_min_v, t_max_v);
    let t_enter = max(max(t1.x, t1.y), t1.z);
    let t_exit  = min(min(t2.x, t2.y), t2.z);

    s.hit = t_exit > max(t_enter, 0.0);
    s.t_start = max(t_enter, 0.0);
    s.t_exit  = t_exit;
    s.vol_extent = vol.vol_max - vol.vol_min;

    let voxel_world = s.vol_extent / vec3f(vol.lod0_dims);
    s.step_size = vol.relative_step_size / length(s.ray_dir / voxel_world);
    s.inv_tile0_scale_x = 1.0 / vt.lods[0].tile_scale.x;
    return s;
}

// World-space advance for the next step. Resident tiles → step_size scaled to
// the current LOD; unresident regions → jump to the next coarsest-LOD tile
// boundary so we don't crawl through gaps one LOD-0 voxel at a time.
fn compute_advance(lod_f: f32, step_size: f32, vol_uv: vec3f, ray_dir: vec3f,
                   vol_extent: vec3f, inv_tile0_scale_x: f32) -> f32 {
    if lod_f >= 0.0 {
        let lod_i = i32(lod_f);
        return step_size * vt.lods[lod_i].tile_scale.x * inv_tile0_scale_x;
    }
    let coarsest = i32(vt.lod_count) - 1;
    let coarse_scale = vt.lods[coarsest].tile_scale;
    let duv_dt = ray_dir / vol_extent;
    let tile_idx  = floor(vol_uv / coarse_scale);
    let next_uv   = select(tile_idx, tile_idx + 1.0, duv_dt > vec3f(0.0)) * coarse_scale;
    let dt_to_bound = select(
        vec3f(1e9),
        (next_uv - vol_uv) / duv_dt,
        abs(duv_dt) > vec3f(1e-7)
    );
    return max(min(dt_to_bound.x, min(dt_to_bound.y, dt_to_bound.z)), step_size);
}

fn contrast_normalise(raw: f32) -> f32 {
    return clamp((raw - vt.contrast_min) / (vt.contrast_max - vt.contrast_min), 0.0, 1.0);
}

fn sample_colormap(adjusted: f32) -> vec4f {
    return textureSampleLevel(colormap, colormap_sampler, vec2f(adjusted, 0.5), 0.0);
}

// ── Mode: translucent (also hosts debug modes 1/2/3) ─────────────────────────

@fragment
fn fs_translucent(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    var accum_color = vec3f(0.0);
    var accum_alpha = 0.0;
    var step_count  = 0u;
    var t = s.t_start;

    loop {
        if t >= s.t_exit || accum_alpha >= vol.early_exit_alpha { break; }
        step_count += 1u;

        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));

        // Mode 2: atlas direct — bypass page table, sample atlas 0 directly.
        if vol.debug_mode == 2u {
            let raw      = sample_atlas(0u, vol_uv);
            let adjusted = contrast_normalise(raw);
            if adjusted > 0.01 {
                let cs         = sample_colormap(adjusted);
                let extinction = cs.a * vol.density_scale * s.step_size;
                let alpha      = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;
                accum_alpha += (1.0 - accum_alpha) * alpha;
            }
            t += s.step_size;
            continue;
        }

        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir,
                                       s.vol_extent, s.inv_tile0_scale_x);

        if vol.debug_mode == 1u {
            // Mode 1: LOD tinting.
            let adjusted = contrast_normalise(raw);
            if lod_f < 0.0 {
                let extinction = 0.04 * vol.density_scale * advance;
                let alpha = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * vec3f(0.2, 0.2, 1.0);
                accum_alpha += (1.0 - accum_alpha) * alpha;
            } else {
                let max_lod = f32(vt.lod_count) - 1.0;
                let lod_t   = lod_f / max(max_lod, 1.0);
                let tint    = vec3f(
                    clamp(lod_t * 2.0, 0.0, 1.0),
                    clamp(2.0 - lod_t * 2.0, 0.0, 1.0),
                    0.0,
                );
                if adjusted > 0.01 {
                    let cs         = sample_colormap(adjusted);
                    let extinction = cs.a * vol.density_scale * advance;
                    let alpha      = 1.0 - exp(-extinction);
                    accum_color += (1.0 - accum_alpha) * alpha * tint;
                    accum_alpha += (1.0 - accum_alpha) * alpha;
                }
            }
        } else {
            // Mode 0: normal DVR (Beer-Lambert).
            let adjusted = contrast_normalise(raw);
            if adjusted > 0.01 {
                let cs         = sample_colormap(adjusted);
                let extinction = cs.a * vol.density_scale * advance;
                let alpha      = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;
                accum_alpha += (1.0 - accum_alpha) * alpha;
            }
        }

        t += advance;
    }

    if vol.debug_mode == 3u {
        // Step-count heatmap, normalised against the largest LOD-0 dim.
        let max_dim = f32(max(vol.lod0_dims.x, max(vol.lod0_dims.y, vol.lod0_dims.z)));
        let s_norm = clamp(f32(step_count) / max_dim, 0.0, 1.0);
        let heat = vec3f(
            clamp(s_norm * 2.0 - 1.0, 0.0, 1.0),
            clamp(1.0 - abs(s_norm * 2.0 - 1.0), 0.0, 1.0),
            clamp(1.0 - s_norm * 2.0, 0.0, 1.0),
        );
        return vec4f(heat, 1.0);
    }

    return encode_srgb(vec4f(accum_color, accum_alpha));
}

// ── Mode: additive ───────────────────────────────────────────────────────────
// Each step adds (alpha_step * cs.rgb) to the accumulator with no occlusion.
// Useful for multi-channel composition (stack N AdditiveVolumes).

@fragment
fn fs_additive(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    var accum = vec3f(0.0);
    var t = s.t_start;

    loop {
        if t >= s.t_exit { break; }
        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir,
                                       s.vol_extent, s.inv_tile0_scale_x);

        let adjusted = contrast_normalise(raw);
        if adjusted > 0.01 {
            let cs = sample_colormap(adjusted);
            accum += cs.a * vol.density_scale * advance * cs.rgb;
        }
        t += advance;
    }

    let alpha = clamp(max(accum.r, max(accum.g, accum.b)), 0.0, 1.0);
    return encode_srgb(vec4f(clamp(accum, vec3f(0.0), vec3f(1.0)), alpha));
}

// ── Mode: attenuated MIP ─────────────────────────────────────────────────────
// vispy-style attenuated MIP. Attenuation is driven by accumulated *contrast-
// normalised density*, not by raw ray distance — so empty regions don't burn
// through the attenuation budget. attenuation = 0 degenerates to plain MIP.

@fragment
fn fs_mip(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    let ray_len        = max(s.t_exit - s.t_start, 1e-9);
    let inv_ray_len    = 1.0 / ray_len;
    let inv_clim_range = 1.0 / max(vt.contrast_max - vt.contrast_min, 1e-9);

    var maxval = 0.0;
    var sumval = 0.0;
    var any_hit = false;
    var t = s.t_start;

    loop {
        if t >= s.t_exit { break; }
        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir,
                                       s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            let norm = clamp((raw - vt.contrast_min) * inv_clim_range, 0.0, 1.0);
            sumval += (advance * inv_ray_len) * norm;
            let scale = exp(-vol.attenuation * sumval);

            // Future samples can't beat this — attenuation only grows.
            if maxval > vt.contrast_max * scale { break; }

            let attenuated = raw * scale;
            if attenuated > maxval {
                maxval = attenuated;
                any_hit = true;
            }
        }
        t += advance;
    }

    if !any_hit { discard; }
    let cs = sample_colormap(contrast_normalise(maxval));
    return encode_srgb(vec4f(cs.rgb, 1.0));
}

// ── Mode: minIP ──────────────────────────────────────────────────────────────

@fragment
fn fs_minip(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    var min_val = 1.0e30;
    var any_hit = false;
    var t = s.t_start;

    loop {
        if t >= s.t_exit { break; }
        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir,
                                       s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            if raw < min_val { min_val = raw; any_hit = true; }
        }
        t += advance;
    }

    if !any_hit { discard; }
    let cs = sample_colormap(contrast_normalise(min_val));
    return encode_srgb(vec4f(cs.rgb, 1.0));
}

// ── Mode: average ────────────────────────────────────────────────────────────

@fragment
fn fs_average(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    var sum = 0.0;
    var n   = 0u;
    var t = s.t_start;

    loop {
        if t >= s.t_exit { break; }
        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir,
                                       s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            sum += raw;
            n += 1u;
        }
        t += advance;
    }

    if n == 0u { discard; }
    let mean = sum / f32(n);
    let cs = sample_colormap(contrast_normalise(mean));
    return encode_srgb(vec4f(cs.rgb, 1.0));
}

// ── Mode: isosurface ─────────────────────────────────────────────────────────
// Coarse Nearest ray-march to first sample above threshold, then a short
// bisection to refine where in the last step the surface lies, then estimate
// normals from central differences with a stencil sized to the resident LOD's
// voxel. All samples are Nearest — the smoothness sacrifice is invisible at
// orbit distance and avoids the 8× cost of manual trilinear per probe.

fn sample_adjusted(vol_uv: vec3f) -> f32 {
    let r = sample_vvt(vol_uv);
    if r.y < 0.0 { return 0.0; }
    return contrast_normalise(r.x);
}

/// Voxel size at LOD `lod_i`, in vol_uv units.
fn lod_voxel_size_uv(lod_i: i32) -> vec3f {
    let ratio = vt.lods[lod_i].tile_scale / vt.lods[0].tile_scale;
    return ratio / vec3f(vol.lod0_dims);
}

@fragment
fn fs_iso(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    var t = s.t_start;
    var t_prev = t;
    var hit_t = -1.0;

    loop {
        if t >= s.t_exit { break; }
        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir,
                                       s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            let adjusted = contrast_normalise(raw);
            if adjusted >= vol.iso_threshold {
                hit_t = t;
                break;
            }
            t_prev = t;
        }
        t += advance;
    }

    if hit_t < 0.0 { discard; }

    // Bisection refining the surface t-position. Nearest sampling means we
    // converge to the voxel boundary where the value crosses the threshold,
    // not the smooth iso-crossing — at orbit distance the stair-stepping is
    // invisible and we avoid 8 page-table walks per probe. 3 halvings = 1/8
    // of the coarse step, which at step_size = 1 LOD-0 voxel is well below
    // the visible pixel.
    var lo = t_prev;
    var hi = hit_t;
    for (var i = 0; i < 3; i++) {
        let mid = 0.5 * (lo + hi);
        let p   = s.ray_origin + s.ray_dir * mid;
        let uv  = clamp((p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let a   = sample_adjusted(uv);
        if a >= vol.iso_threshold { hi = mid; } else { lo = mid; }
    }
    let surface_t  = hi;
    let surface_p  = s.ray_origin + s.ray_dir * surface_t;
    let surface_uv = clamp((surface_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));

    // Gradient stencil scaled to the resident LOD's voxel size, sampled with
    // Nearest (one page-table walk per axis instead of eight). The stencil
    // straddles one full voxel so +h and -h always land in different voxels
    // — gradient is well-defined; only the smoothness of the lighting
    // suffers, which is fine for first-hit Phong on a textured volume.
    let surface_center = sample_vvt(surface_uv);
    let lod_i = max(i32(surface_center.y), 0);
    let h = 0.5 * lod_voxel_size_uv(lod_i);
    let gx = sample_adjusted(surface_uv + vec3f(h.x, 0.0, 0.0))
           - sample_adjusted(surface_uv - vec3f(h.x, 0.0, 0.0));
    let gy = sample_adjusted(surface_uv + vec3f(0.0, h.y, 0.0))
           - sample_adjusted(surface_uv - vec3f(0.0, h.y, 0.0));
    let gz = sample_adjusted(surface_uv + vec3f(0.0, 0.0, h.z))
           - sample_adjusted(surface_uv - vec3f(0.0, 0.0, h.z));
    let grad = vec3f(gx, gy, gz);
    let glen = length(grad);
    let normal = select(vec3f(0.0, 0.0, 1.0), -grad / glen, glen > 1e-6);

    // Phong: headlight + ambient.
    let view = -s.ray_dir;
    let light = view;
    let ndotl = max(dot(normal, light), 0.0);
    let refl  = reflect(-light, normal);
    let spec  = pow(max(dot(refl, view), 0.0), 32.0);

    let cs = sample_colormap(vol.iso_threshold);
    let ambient  = 0.2 * cs.rgb;
    let diffuse  = 0.7 * ndotl * cs.rgb;
    let specular = 0.3 * spec * vec3f(1.0);
    return encode_srgb(vec4f(ambient + diffuse + specular, 1.0));
}
