// Volume ray-marching shader
//
// Renders a volume by marching rays through the AABB. The bounding-box back faces
// are rasterised; each fragment reconstructs its ray from camera_pos -> world_pos,
// does an AABB slab test to find the entry/exit points, then marches forward.
//
// Tile data is looked up via the same virtual-texture page table as the slice shader.
// try_lod / sample_vvt are duplicated verbatim from virtual_tile.wgsl.
//
// Bind groups:
//   0 - camera (view_proj matrix)
//   1 - atlas, atlas_sampler, page_table, vt_uniforms  (identical to virtual_tile.wgsl)
//   2 - vol_uniforms, colormap, colormap_sampler
//
// Multiple fragment entry points, one render pipeline per mode:
//   fs_translucent      - front-to-back alpha compositing (Beer-Lambert, default).
//                         Also hosts the debug modes (vol.debug_mode 1/2/3).
//   fs_additive         - additive accumulation of colormapped contributions.
//   fs_mip              - Maximum Intensity Projection.
//   fs_minip            - Minimum Intensity Projection.
//   fs_average          - mean intensity along ray.
//   fs_attenuated_mip   - MIP weighted by exp(-attenuation * t) so near-camera wins.
//   fs_iso              - first-hit isosurface with Phong shading.

// -- Group 0: camera ---------------------------------------------------------

struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// -- Group 1: virtual texture resources --------------------------------------

@group(1) @binding(0) var atlas: texture_3d<f32>;
@group(1) @binding(1) var atlas_sampler: sampler;
@group(1) @binding(2) var page_table: texture_2d_array<u32>;

struct VTLodInfo {
    grid_dims: vec3<u32>,
    _pad: u32,
    tile_scale: vec3<f32>,
    _pad2: f32,
    data_scale: vec3<f32>,
    _pad3: f32,
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
    _pad_c: u32,
    lods: array<VTLodInfo, 16>,
}

@group(1) @binding(3) var<uniform> vt: VTUniforms;

// -- Group 2: volume uniforms + colormap -------------------------------------

struct VolumeUniforms {
    vol_min: vec3<f32>,
    relative_step_size: f32,
    vol_max: vec3<f32>,
    density_scale: f32,
    camera_pos: vec3<f32>,
    early_exit_alpha: f32,
    lod0_dims: vec3<u32>,
    debug_mode: u32,
    /// Isosurface threshold in raw-data (post-contrast-normalised) units, 0..1.
    iso_threshold: f32,
    /// Attenuated-MIP falloff coefficient (per world-space unit along the ray).
    attenuation: f32,
    _pad0: f32,
    _pad1: f32,
}

@group(2) @binding(0) var<uniform> vol: VolumeUniforms;
@group(2) @binding(1) var colormap: texture_1d<f32>;
@group(2) @binding(2) var colormap_sampler: sampler;

// -- Vertex stage ------------------------------------------------------------

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

// -- sRGB encoding (for linear-format surfaces) ------------------------------

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

// -- Page-table sampling (mirrored from virtual_tile.wgsl) -------------------

fn try_lod(vol_uv: vec3f, lod: i32) -> vec2f {
    let grid = vt.lods[lod].grid_dims;
    let vol_in_tiles = min(vol_uv / vt.lods[lod].tile_scale, vec3f(grid) - vec3f(1e-5));
    let tc = clamp(vec3i(vol_in_tiles), vec3i(0), vec3i(grid) - vec3i(1));
    let linear = u32(tc.z) * grid.y * grid.x + u32(tc.y) * grid.x + u32(tc.x);
    let pt_x = i32(linear % vt.page_table_width);
    let pt_y = i32(linear / vt.page_table_width);
    let entry = textureLoad(page_table, vec2i(pt_x, pt_y), lod, 0).r;

    if (entry >> 24u) == 0u { return vec2f(0.0, -1.0); }

    let slot        = entry & 0xFFFFu;
    let atlas_col   = slot % vt.atlas_cols;
    let atlas_row   = (slot / vt.atlas_cols) % vt.atlas_rows;
    let atlas_layer = slot / (vt.atlas_cols * vt.atlas_rows);

    let within_tile = (vol_in_tiles - floor(vol_in_tiles)) * vt.lods[lod].data_scale;

    let u = (f32(atlas_col)   + within_tile.x) * vt.atlas_tile_pitch_x;
    let v = (f32(atlas_row)   + within_tile.y) * vt.atlas_tile_pitch_y;
    let w = (f32(atlas_layer) + within_tile.z) * vt.atlas_tile_pitch_z;

    return vec2f(textureSampleLevel(atlas, atlas_sampler, vec3f(u, v, w), 0.0).r,
                 f32(lod));
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

// -- Shared ray-setup helpers ------------------------------------------------

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

// World-space advance for the next step. For resident tiles, scales step_size to
// the current LOD; for unresident regions, jumps to the next coarsest-LOD tile
// boundary so we don't crawl through gaps.
fn compute_advance(lod_f: f32, step_size: f32, vol_uv: vec3f, ray_dir: vec3f, vol_extent: vec3f, inv_tile0_scale_x: f32) -> f32 {
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

// -- Mode: translucent (also hosts debug modes 1/2/3) ------------------------

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

        // Mode 2: atlas direct ----------------------------------------------
        if vol.debug_mode == 2u {
            let raw      = textureSampleLevel(atlas, atlas_sampler, vol_uv, 0.0).r;
            let adjusted = contrast_normalise(raw);
            if adjusted > 0.01 {
                let cs         = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
                let extinction = cs.a * vol.density_scale * s.step_size;
                let alpha      = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;
                accum_alpha += (1.0 - accum_alpha) * alpha;
            }
            t += s.step_size;
            continue;
        }

        let result = sample_vvt(vol_uv);
        let raw    = result.x;
        let lod_f  = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir, s.vol_extent, s.inv_tile0_scale_x);

        // Mode 1: LOD tinting ----------------------------------------------
        if vol.debug_mode == 1u {
            let adjusted = contrast_normalise(raw);
            if lod_f < 0.0 {
                let extinction = 0.04 * vol.density_scale * advance;
                let alpha = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * vec3f(0.2, 0.2, 1.0);
                accum_alpha += (1.0 - accum_alpha) * alpha;
            } else {
                let max_lod = f32(vt.lod_count) - 1.0;
                let lod_t   = lod_f / max(max_lod, 1.0);
                let tint = vec3f(
                    clamp(lod_t * 2.0, 0.0, 1.0),
                    clamp(2.0 - lod_t * 2.0, 0.0, 1.0),
                    0.0,
                );
                if adjusted > 0.01 {
                    let cs         = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
                    let extinction = cs.a * vol.density_scale * advance;
                    let alpha      = 1.0 - exp(-extinction);
                    accum_color += (1.0 - accum_alpha) * alpha * tint;
                    accum_alpha += (1.0 - accum_alpha) * alpha;
                }
            }
        } else {
            // Mode 0: normal DVR --------------------------------------------
            let adjusted = contrast_normalise(raw);
            if adjusted > 0.01 {
                let cs         = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
                let extinction = cs.a * vol.density_scale * advance;
                let alpha      = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;
                accum_alpha += (1.0 - accum_alpha) * alpha;
            }
        }

        t += advance;
    }

    // Mode 3: step-count heatmap ------------------------------------------
    if vol.debug_mode == 3u {
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

// -- Mode: additive -----------------------------------------------------------
// Each step adds (alpha_step * cs.rgb) to the accumulator with no occlusion.
// Pre-multiplied RGB, alpha = clamp(sum) so the result composites nicely.

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
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir, s.vol_extent, s.inv_tile0_scale_x);

        let adjusted = contrast_normalise(raw);
        if adjusted > 0.01 {
            let cs = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
            accum += cs.a * vol.density_scale * advance * cs.rgb;
        }
        t += advance;
    }

    let alpha = clamp(max(accum.r, max(accum.g, accum.b)), 0.0, 1.0);
    return encode_srgb(vec4f(clamp(accum, vec3f(0.0), vec3f(1.0)), alpha));
}

// -- Mode: minIP --------------------------------------------------------------

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
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir, s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            if raw < min_val { min_val = raw; any_hit = true; }
        }
        t += advance;
    }

    if !any_hit { discard; }
    let adjusted = contrast_normalise(min_val);
    let cs = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
    return encode_srgb(vec4f(cs.rgb, 1.0));
}

// -- Mode: average ------------------------------------------------------------

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
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir, s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            sum += raw;
            n += 1u;
        }
        t += advance;
    }

    if n == 0u { discard; }
    let mean     = sum / f32(n);
    let adjusted = contrast_normalise(mean);
    let cs = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
    return encode_srgb(vec4f(cs.rgb, 1.0));
}

// -- Mode: attenuated MIP ----------------------------------------------------
// Pure distance-based attenuation along the normalised ray (0 at the entry
// face, 1 at the exit face). `attenuation` is in units of 1/ray-length, so
// scale = exp(-attenuation) at the far end and scale = 1 at the near end.
// At attenuation = 0 this degenerates to plain MIP.
//
// Two earlier designs had problems on real data:
//   1) world-distance attenuation: aggressive on big volumes but useless on
//      tiny ones — single slider couldn't span both.
//   2) density-weighted accumulation (vispy-style): elegant in theory but
//      sparse-data rays (lots of empty space + scattered bright points)
//      accumulated almost nothing, so high attenuation values still let the
//      far side bleed through.
//
// To prevent the "dark floor" problem (an empty ray's brightest near sample
// being raw≈0, so the output is black even when there's bright material
// further along), we skip below-floor samples entirely — they don't compete
// for `maxval`. Rays that find no above-floor sample discard.

@fragment
fn fs_attenuated_mip(in: VertexOutput) -> @location(0) vec4<f32> {
    let s = setup_ray(in.world_pos);
    if !s.hit { discard; }

    let ray_len     = max(s.t_exit - s.t_start, 1e-9);
    let inv_ray_len = 1.0 / ray_len;

    var maxval = 0.0;
    var any_hit = false;
    var t = s.t_start;

    loop {
        if t >= s.t_exit { break; }
        let world_p = s.ray_origin + s.ray_dir * t;
        let vol_uv  = clamp((world_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let result  = sample_vvt(vol_uv);
        let raw     = result.x;
        let lod_f   = result.y;
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir, s.vol_extent, s.inv_tile0_scale_x);

        if lod_f >= 0.0 {
            let adjusted = contrast_normalise(raw);
            // Skip below-floor samples so empty space doesn't drag maxval down
            // to ~0 (which would then colormap as the floor colour and create
            // the "floor effect" bug).
            if adjusted > 0.001 {
                let t_norm = (t - s.t_start) * inv_ray_len;
                let scale  = exp(-vol.attenuation * t_norm);

                // Early termination: future scale only shrinks, and future raw
                // contributes at most `contrast_max` to the colormap. If our
                // current best already beats that bound, no future sample wins.
                if maxval > vt.contrast_max * scale { break; }

                let attenuated = raw * scale;
                if attenuated > maxval {
                    maxval = attenuated;
                    any_hit = true;
                }
            }
        }
        t += advance;
    }

    if !any_hit { discard; }
    let adjusted = contrast_normalise(maxval);
    let cs = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
    return encode_srgb(vec4f(cs.rgb, 1.0));
}

// -- Mode: isosurface --------------------------------------------------------
// Coarse ray-march in Nearest to find the first sample above threshold; then
// bisect to locate the *smooth* (trilinear) iso-crossing sub-voxel-precisely;
// then estimate normals from central differences of the same trilinear field.
//
// The atlas sampler is Nearest because the VT doesn't yet have border guards,
// so we synthesise trilinear filtering by combining 8 page-table walks per
// sample. Expensive but only paid for the ~10 lookups after a ray hits — the
// coarse march stays Nearest.

fn sample_adjusted(vol_uv: vec3f) -> f32 {
    let r = sample_vvt(vol_uv);
    if r.y < 0.0 { return 0.0; }
    return contrast_normalise(r.x);
}

/// Voxel size at LOD `lod_i`, in vol_uv units. For dyadic LODs this is just
/// `2^lod_i / lod0_dims`; in general it scales by `tile_scale[lod]/tile_scale[0]`.
fn lod_voxel_size_uv(lod_i: i32) -> vec3f {
    let ratio = vt.lods[lod_i].tile_scale / vt.lods[0].tile_scale;
    return ratio / vec3f(vol.lod0_dims);
}

/// Trilinear interpolation of the contrast-normalised VT field at the
/// *locally-resident* LOD's voxel grid. The center sample reports its LOD,
/// and the 8 corners are spaced one LOD-i voxel apart — so even at coarse
/// LODs we get genuine smoothing across that LOD's voxels, not a no-op.
fn sample_trilinear_adjusted(vol_uv: vec3f) -> f32 {
    let center = sample_vvt(vol_uv);
    if center.y < 0.0 { return 0.0; }
    let lod_i = i32(center.y);

    let h = lod_voxel_size_uv(lod_i);
    let inv_h = vec3f(1.0) / h;
    // Shift by 0.5 so floor() lands on the lower-left voxel *centre*.
    let scaled = vol_uv * inv_h - vec3f(0.5);
    let i = floor(scaled);
    let f = scaled - i;
    let base = (i + vec3f(0.5)) * h;

    let v000 = sample_adjusted(base);
    let v100 = sample_adjusted(base + vec3f(h.x, 0.0, 0.0));
    let v010 = sample_adjusted(base + vec3f(0.0, h.y, 0.0));
    let v110 = sample_adjusted(base + vec3f(h.x, h.y, 0.0));
    let v001 = sample_adjusted(base + vec3f(0.0, 0.0, h.z));
    let v101 = sample_adjusted(base + vec3f(h.x, 0.0, h.z));
    let v011 = sample_adjusted(base + vec3f(0.0, h.y, h.z));
    let v111 = sample_adjusted(base + vec3f(h.x, h.y, h.z));

    let v00 = mix(v000, v100, f.x);
    let v10 = mix(v010, v110, f.x);
    let v01 = mix(v001, v101, f.x);
    let v11 = mix(v011, v111, f.x);

    let v0 = mix(v00, v10, f.y);
    let v1 = mix(v01, v11, f.y);

    return mix(v0, v1, f.z);
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
        let advance = compute_advance(lod_f, s.step_size, vol_uv, s.ray_dir, s.vol_extent, s.inv_tile0_scale_x);

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

    // Sub-voxel bisection using the smooth trilinear field — five halvings
    // brings us within ~1/32 of a coarse-march step of the true crossing.
    var lo = t_prev;
    var hi = hit_t;
    for (var i = 0; i < 5; i++) {
        let mid = 0.5 * (lo + hi);
        let p   = s.ray_origin + s.ray_dir * mid;
        let uv  = clamp((p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));
        let a   = sample_trilinear_adjusted(uv);
        if a >= vol.iso_threshold { hi = mid; } else { lo = mid; }
    }
    let surface_t  = hi;
    let surface_p  = s.ray_origin + s.ray_dir * surface_t;
    let surface_uv = clamp((surface_p - vol.vol_min) / s.vol_extent, vec3f(0.0), vec3f(1.0));

    // Gradient stencil scaled to the resident LOD's voxel size — so at coarse
    // LODs we sample across the appropriate coarse-voxel distance rather than
    // collapsing inside a single coarse voxel (which would zero the gradient).
    let surface_center = sample_vvt(surface_uv);
    let lod_i = max(i32(surface_center.y), 0);
    let h = 0.5 * lod_voxel_size_uv(lod_i);
    let gx = sample_trilinear_adjusted(surface_uv + vec3f(h.x, 0.0, 0.0))
           - sample_trilinear_adjusted(surface_uv - vec3f(h.x, 0.0, 0.0));
    let gy = sample_trilinear_adjusted(surface_uv + vec3f(0.0, h.y, 0.0))
           - sample_trilinear_adjusted(surface_uv - vec3f(0.0, h.y, 0.0));
    let gz = sample_trilinear_adjusted(surface_uv + vec3f(0.0, 0.0, h.z))
           - sample_trilinear_adjusted(surface_uv - vec3f(0.0, 0.0, h.z));
    let grad = vec3f(gx, gy, gz);
    let glen = length(grad);
    let normal = select(vec3f(0.0, 0.0, 1.0), -grad / glen, glen > 1e-6);

    // Phong shading: headlight + a fixed ambient. View dir is -ray_dir.
    let view = -s.ray_dir;
    let light = view;
    let ndotl = max(dot(normal, light), 0.0);
    let refl  = reflect(-light, normal);
    let spec  = pow(max(dot(refl, view), 0.0), 32.0);

    // Iso surfaces use a fixed bright base colour rather than sampling the
    // colormap at `iso_threshold`. Sampling at threshold tied the surface
    // brightness to where the threshold landed, which made the surface
    // mysteriously dark whenever the user set a low threshold to enlarge the
    // surface. The visual identity of an iso is a single value — its colour
    // should be a property of the visual, not the data intensity.
    // TODO: expose a `set_iso_color` setter so users can recolour the iso.
    let base = vec3f(0.85, 0.85, 0.9);   // slightly cool white
    let ambient  = 0.2 * base;
    let diffuse  = 0.7 * ndotl * base;
    let specular = 0.3 * spec * vec3f(1.0);
    return encode_srgb(vec4f(ambient + diffuse + specular, 1.0));
}

