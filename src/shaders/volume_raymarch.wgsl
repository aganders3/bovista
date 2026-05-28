// Volume ray-marching shader
//
// Renders a volume by marching rays through the AABB. The bounding box back faces
// are rasterised; each fragment reconstructs its ray from camera_pos → world_pos,
// does an AABB slab test to find the entry/exit points, then marches forward with
// front-to-back alpha compositing.
//
// Tile data is looked up via the same virtual-texture page table as the slice shader.
// try_lod / sample_vvt are duplicated verbatim from virtual_tile.wgsl.
//
// Bind groups:
//   0 — camera (view_proj matrix)
//   1 — atlas, atlas_sampler, page_table, vt_uniforms  (identical to virtual_tile.wgsl)
//   2 — vol_uniforms, colormap, colormap_sampler
//
// Debug modes (uni.vol.debug_mode):
//   0 — normal DVR with LOD-adaptive step size
//   1 — LOD tinting + shader-based tile wireframes (green=fine, red=coarse, white=boundary)
//   2 — atlas direct: bypass page-table indirection, sample the raw atlas texture

// ── Single bind group (workaround for wgpu-hal-gles multi-UBO aliasing
//    on NVIDIA 3.30 compat profile — see memory/wgpu-gles-multi-ubo-bug.md).
//    All formerly-separate uniform buffers are now nested inside one struct
//    and live at binding 0. Textures and samplers continue to use their own
//    GL state (texture units, sampler units) and are unaffected by the bug,
//    so they sit alongside at bindings 1–5. ─────────────────────────────────

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
    /// 3=step heatmap, 4=red probe, 5=uniform echo, 6/7/8=geometry probes.
    debug_mode: u32,
}

struct VolumeAllUniforms {
    view_proj: mat4x4<f32>,
    vt: VTUniforms,
    vol: VolumeUniforms,
}

@group(0) @binding(0) var<uniform> uni: VolumeAllUniforms;
@group(0) @binding(1) var atlas: texture_3d<f32>;
@group(0) @binding(2) var atlas_sampler: sampler;
@group(0) @binding(3) var page_table: texture_2d_array<u32>;
@group(0) @binding(4) var colormap: texture_1d<f32>;
@group(0) @binding(5) var colormap_sampler: sampler;

// ── Vertex stage ──────────────────────────────────────────────────────────────

struct VertexInput {
    @location(0) position: vec3<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_pos: vec3<f32>,
}

@vertex
fn vs_main(@builtin(vertex_index) vi: u32, in: VertexInput) -> VertexOutput {
    var out: VertexOutput;
    // TEMP DIAGNOSTIC: debug-mode early-return branches removed to test
    // whether the if-return-if-return-if-return pattern in VS interacts
    // badly with the volume's UBO reads on the HPC GL 3.30 path. Mac
    // doesn't need this path. (mode 6/7/8 probes will no longer work
    // while this is in place.)
    _ = vi;
    let world = uni.vol.vol_min + in.position * (uni.vol.vol_max - uni.vol.vol_min);
    out.clip_position = uni.view_proj * vec4f(world, 1.0);
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
    let grid = uni.vt.lods[lod].grid_dims;
    // Clamp to [0, grid - ε) before computing frac. Without this, when vol_uv = 1.0 and
    // the volume dimension is an exact multiple of tile_w, vol_in_tiles lands on an integer
    // N, frac(N) = 0, and the shader samples the START of the last tile instead of the END.
    // The ε is well under one texel (tile_w ≥ 16, so one texel ≈ 0.06), no visible shift.
    let vol_in_tiles = min(vol_uv / uni.vt.lods[lod].tile_scale, vec3f(grid) - vec3f(1e-5));
    let tc = clamp(vec3i(vol_in_tiles), vec3i(0), vec3i(grid) - vec3i(1));
    let linear = u32(tc.z) * grid.y * grid.x + u32(tc.y) * grid.x + u32(tc.x);
    let pt_x = i32(linear % uni.vt.page_table_width);
    let pt_y = i32(linear / uni.vt.page_table_width);
    let entry = textureLoad(page_table, vec2i(pt_x, pt_y), lod, 0).r;

    if (entry >> 24u) == 0u { return vec2f(0.0, -1.0); }

    let slot        = entry & 0xFFFFu;
    let atlas_col   = slot % uni.vt.atlas_cols;
    let atlas_row   = (slot / uni.vt.atlas_cols) % uni.vt.atlas_rows;
    let atlas_layer = slot / (uni.vt.atlas_cols * uni.vt.atlas_rows);

    let within_tile = (vol_in_tiles - floor(vol_in_tiles)) * uni.vt.lods[lod].data_scale;

    let u = (f32(atlas_col)   + within_tile.x) * uni.vt.atlas_tile_pitch_x;
    let v = (f32(atlas_row)   + within_tile.y) * uni.vt.atlas_tile_pitch_y;
    let w = (f32(atlas_layer) + within_tile.z) * uni.vt.atlas_tile_pitch_z;

    return vec2f(textureSampleLevel(atlas, atlas_sampler, vec3f(u, v, w), 0.0).r,
                 f32(lod));
}

fn sample_vvt(vol_uv: vec3f) -> vec2f {
    let ideal_lod = i32(uni.vt.target_lod);
    let max_lod   = i32(uni.vt.lod_count) - 1;

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

// ── Fragment stage ────────────────────────────────────────────────────────────

@fragment
fn fs_main(in: VertexOutput) -> @location(0) vec4<f32> {
    // TEMP DIAGNOSTIC: short-circuit fs_main to do JUST the AABB slab test
    // and return a color based on the outcome (no discard, no raymarch).
    //   blue   → slab test passed (ray enters the volume box)
    //   red    → slab test failed (ray misses)
    // On Mac we'd see blue where the cube projects and red elsewhere on
    // top of any clear color. If HPC shows all-red, the FS-side uniform
    // reads (vol_min, vol_max, camera_pos) are wrong despite working in
    // VS. If HPC shows blue inside the cube silhouette, the slab test is
    // fine and the bug is further downstream in the raymarch.
    // Same slab test (known to work). On success, sample the atlas DIRECTLY
    // at uv=(0.5, 0.5, 0.5) — bypass the page table. If atlas sampling works,
    // the synthetic 128³ cube data has v ≈ 1.0 at center → returns red. If
    // sampling fails, we get black (or transparent).
    let _ro = uni.vol.camera_pos;
    let _rd = normalize(in.world_pos - uni.vol.camera_pos);
    let _id = 1.0 / _rd;
    let _tmn = (uni.vol.vol_min - _ro) * _id;
    let _tmx = (uni.vol.vol_max - _ro) * _id;
    let _t1 = min(_tmn, _tmx);
    let _t2 = max(_tmn, _tmx);
    let _te = max(max(_t1.x, _t1.y), _t1.z);
    let _tx = min(min(_t2.x, _t2.y), _t2.z);
    if _tx <= max(_te, 0.0) {
        return vec4f(0.0, 0.0, 0.0, 1.0);
    }
    // Slab passed + atlas sampling works (purple-pink on HPC) — keep going.
    // Now read the page table for lod=0, tile (0,0,0). Our example loaded
    // exactly one tile at slot 0; page_table entry should be
    //   (1u << 24) | slot = 0x01000000  → entry >> 24 == 1 (resident).
    // Color encoding:
    //   resident bit set, slot 0   → yellow (1.0, 1.0, 0.0)
    //   resident bit set, other    → orange-ish (1.0, 0.5, 0.0)
    //   resident bit not set       → cyan (0.0, 1.0, 1.0)
    let _e = textureLoad(page_table, vec2<i32>(0, 0), 0, 0);
    let resident = (_e.x >> 24u) != 0u;
    let slot     = _e.x & 0xFFFFu;
    if resident && slot == 0u {
        return vec4f(1.0, 1.0, 0.0, 1.0);
    }
    if resident {
        return vec4f(1.0, 0.5, 0.0, 1.0);
    }
    return vec4f(0.0, 1.0, 1.0, 1.0);

    // ── Debug mode 4: pure-red probe (no uniform reads, no AABB test) ───────
    if uni.vol.debug_mode == 4u {
        return vec4f(1.0, 0.0, 0.0, 1.0);
    }
    // ── Debug mode 5: uniform-echo probe ────────────────────────────────────
    // Encodes the volume's AABB extent in the red/green/blue channels. If you
    // get black with mode 5 but red with mode 4, the uniform buffer is being
    // read at wrong offsets — std140 vs WGSL packing mismatch on this driver.
    // Expected on a 128³ cube: a soft greyish color (extent ≈ 128 / 256 = 0.5
    // each).
    if uni.vol.debug_mode == 5u {
        let extent = (uni.vol.vol_max - uni.vol.vol_min) / 256.0;
        return vec4f(extent, 1.0);
    }
    // ── Debug mode 6: fullscreen-triangle probe (companion to vs_main) ─────
    // The vertex shader emits a fullscreen triangle from vertex_index alone
    // (no camera, no uniform reads). If the screen comes back green, the
    // pipeline can rasterize a triangle on this backend — meaning our cube
    // vertices must be coming out wrong due to a uniform or camera issue.
    // If still blue, the rasterizer / cull / depth-test config itself is
    // broken on this driver, not the shader uniforms.
    if uni.vol.debug_mode == 6u {
        return vec4f(0.0, 1.0, 0.0, 1.0);
    }
    if uni.vol.debug_mode == 7u {
        return vec4f(1.0, 1.0, 0.0, 1.0);  // yellow
    }
    if uni.vol.debug_mode == 8u {
        return vec4f(0.0, 1.0, 1.0, 1.0);  // cyan
    }

    let ray_origin = uni.vol.camera_pos;
    let ray_dir    = normalize(in.world_pos - uni.vol.camera_pos);

    // AABB slab test
    let inv_dir = 1.0 / ray_dir;
    let t_min_v = (uni.vol.vol_min - ray_origin) * inv_dir;
    let t_max_v = (uni.vol.vol_max - ray_origin) * inv_dir;
    let t1 = min(t_min_v, t_max_v);
    let t2 = max(t_min_v, t_max_v);
    let t_enter = max(max(t1.x, t1.y), t1.z);
    let t_exit  = min(min(t2.x, t2.y), t2.z);

    if t_exit <= max(t_enter, 0.0) { discard; }

    let t_start    = max(t_enter, 0.0);
    let vol_extent = uni.vol.vol_max - uni.vol.vol_min;

    // Base step: world-space distance to advance relative_step_size LOD-0 voxels along
    // this ray. 1/||(ray_dir / voxel_world)|| is the world-space distance that moves
    // exactly one voxel along ray_dir, correctly handling anisotropic voxel spacing —
    // using min() oversamples along the coarser axes for non-isotropic datasets.
    let voxel_world = vol_extent / vec3f(uni.vol.lod0_dims);
    let step_size   = uni.vol.relative_step_size / length(ray_dir / voxel_world);

    // tile_scale[lod_i].x / tile_scale[0].x is the LOD-0→LOD-i spatial ratio used
    // to scale `advance` per step. Hoist the division out of the inner loop.
    let inv_tile0_scale_x = 1.0 / uni.vt.lods[0].tile_scale.x;

    var accum_color = vec3f(0.0);
    var accum_alpha = 0.0;
    var step_count  = 0u;
    var t = t_start;

    loop {
        if t >= t_exit || accum_alpha >= uni.vol.early_exit_alpha { break; }
        step_count += 1u;

        let world_p = ray_origin + ray_dir * t;
        let vol_uv  = clamp((world_p - uni.vol.vol_min) / vol_extent, vec3f(0.0), vec3f(1.0));

        // ── Mode 2: atlas direct ────────────────────────────────────────────
        // Bypass the page table entirely — sample the raw packed atlas texture.
        // Useful for verifying atlas allocation and what data is actually loaded.
        if uni.vol.debug_mode == 2u {
            let raw      = textureSampleLevel(atlas, atlas_sampler, vol_uv, 0.0).r;
            let adjusted = clamp((raw - uni.vt.contrast_min) / (uni.vt.contrast_max - uni.vt.contrast_min), 0.0, 1.0);
            if adjusted > 0.01 {
                let cs         = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
                let extinction = cs.a * uni.vol.density_scale * step_size;
                let alpha      = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;
                accum_alpha += (1.0 - accum_alpha) * alpha;
            }
            t += step_size;
            continue;
        }

        // ── Virtual-texture lookup ──────────────────────────────────────────
        let result = sample_vvt(vol_uv);
        let raw    = result.x;
        let lod_f  = result.y;   // integer LOD as float; -1 if not resident

        // LOD-adaptive step: coarser LODs have proportionally larger voxels.
        // tile_scale[k] / tile_scale[0] gives the exact spatial scale ratio,
        // which works for non-dyadic (non-power-of-2) LOD hierarchies too.
        // When not resident (lod_f < 0) jump to the next coarsest-LOD tile boundary
        // instead of crawling one LOD-0 voxel at a time through unloaded space.
        var advance: f32;
        if lod_f >= 0.0 {
            let lod_i = i32(lod_f);
            advance = step_size * uni.vt.lods[lod_i].tile_scale.x * inv_tile0_scale_x;
        } else {
            let coarsest    = i32(uni.vt.lod_count) - 1;
            let coarse_scale = uni.vt.lods[coarsest].tile_scale;
            // Rate of vol_uv change per world-space unit along the ray.
            let duv_dt = ray_dir / vol_extent;
            // Next tile boundary ahead of the ray along each axis.
            let tile_idx  = floor(vol_uv / coarse_scale);
            let next_uv   = select(tile_idx, tile_idx + 1.0, duv_dt > vec3f(0.0)) * coarse_scale;
            // World-space distance to each boundary; use large fallback for axis-parallel rays.
            let dt_to_bound = select(
                vec3f(1e9),
                (next_uv - vol_uv) / duv_dt,
                abs(duv_dt) > vec3f(1e-7)
            );
            // Advance just past the nearest boundary, minimum one step_size.
            advance = max(min(dt_to_bound.x, min(dt_to_bound.y, dt_to_bound.z)), step_size);
        }

        // ── Mode 1: LOD tinting ─────────────────────────────────────────────
        // Use the real data opacity (same as normal DVR) so the volume keeps its
        // natural shape and doesn't saturate. Substitute LOD color for the colormap
        // color so you see the tinting where there is actual tissue/structure.
        // Not-resident regions get a faint blue haze.
        if uni.vol.debug_mode == 1u {
            let adjusted = clamp((raw - uni.vt.contrast_min) / (uni.vt.contrast_max - uni.vt.contrast_min), 0.0, 1.0);
            var tint: vec3f;
            if lod_f < 0.0 {
                // Not resident → faint blue; small fixed opacity so gaps are visible.
                let extinction = 0.04 * uni.vol.density_scale * advance;
                let alpha = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * vec3f(0.2, 0.2, 1.0);
                accum_alpha += (1.0 - accum_alpha) * alpha;
            } else {
                let lod_i   = i32(lod_f);
                let max_lod = f32(uni.vt.lod_count) - 1.0;
                let lod_t   = lod_f / max(max_lod, 1.0);
                tint = vec3f(
                    clamp(lod_t * 2.0, 0.0, 1.0),
                    clamp(2.0 - lod_t * 2.0, 0.0, 1.0),
                    0.0,
                );
                // Same opacity model as normal DVR → no artificial saturation.
                if adjusted > 0.01 {
                    let cs         = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
                    let extinction = cs.a * uni.vol.density_scale * advance;
                    let alpha      = 1.0 - exp(-extinction);
                    accum_color += (1.0 - accum_alpha) * alpha * tint;
                    accum_alpha += (1.0 - accum_alpha) * alpha;
                }
            }
        } else {
            // ── Mode 0: normal DVR ──────────────────────────────────────────
            let adjusted = clamp((raw - uni.vt.contrast_min) / (uni.vt.contrast_max - uni.vt.contrast_min), 0.0, 1.0);
            // Beer-Lambert extinction: cs.a alone controls opacity (TF already encodes
            // density-dependent opacity; multiplying by adjusted would double-penalize
            // low-density regions, making the volume appear too transparent).
            // Guard: skip voxels below the contrast window (adjusted ≈ 0) so they remain
            // fully transparent — without this, zero-density regions pick up any non-zero
            // alpha from the colormap's low end and render as opaque black.
            // Also skips the colormap fetch in sparse/below-threshold regions.
            if adjusted > 0.01 {
                let cs         = textureSampleLevel(colormap, colormap_sampler, adjusted, 0.0);
                let extinction = cs.a * uni.vol.density_scale * advance;
                let alpha      = 1.0 - exp(-extinction);
                accum_color += (1.0 - accum_alpha) * alpha * cs.rgb;
                accum_alpha += (1.0 - accum_alpha) * alpha;
            }
        }

        t += advance;
    }

    // ── Mode 3: step-count heatmap ────────────────────────────────────────────
    // Normalise against the largest LOD-0 dimension (theoretical Nyquist maximum).
    // Blue (few steps / coarse LOD) → green → yellow → red (many steps / fine LOD).
    if uni.vol.debug_mode == 3u {
        let max_dim = f32(max(uni.vol.lod0_dims.x, max(uni.vol.lod0_dims.y, uni.vol.lod0_dims.z)));
        let s = clamp(f32(step_count) / max_dim, 0.0, 1.0);
        let heat = vec3f(
            clamp(s * 2.0 - 1.0, 0.0, 1.0),   // R: lights up in the top half
            clamp(1.0 - abs(s * 2.0 - 1.0), 0.0, 1.0), // G: peaks in the middle
            clamp(1.0 - s * 2.0, 0.0, 1.0),   // B: bright only in the bottom half
        );
        return vec4f(heat, 1.0);
    }

    return encode_srgb(vec4f(accum_color, accum_alpha));
}
