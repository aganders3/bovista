// Unified tile shader - renders image tiles with 3D texture sampling
//
// This shader handles both simple (single tile) and tiled (multiple tiles) rendering.
// It receives pre-computed 3D geometry from the CPU and samples from a 3D texture.

// Camera uniforms (bind group 0)
struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// Tile uniforms (bind group 1)
struct TileUniforms {
    contrast_min: f32,
    contrast_max: f32,
    debug_mode: f32,
    _padding1: f32,
    plane_position: vec3<f32>,
    _padding2: f32,
    plane_normal: vec3<f32>,
    _padding3: f32,
    volume_size: vec3<f32>,
    _padding4: f32,
    debug_color: vec3<f32>,
    _padding5: f32,
}

@group(1) @binding(0)
var tile_texture: texture_3d<f32>;

@group(1) @binding(1)
var tile_sampler: sampler;

@group(1) @binding(2)
var<uniform> tile: TileUniforms;

// Colormap LUT (bind group 2) - 256-entry 1D RGBA texture
@group(2) @binding(0)
var colormap: texture_1d<f32>;

@group(2) @binding(1)
var colormap_sampler: sampler;

// Vertex shader input
struct VertexInput {
    @location(0) position: vec3<f32>,   // 3D world-space position
    @location(1) texcoord: vec3<f32>,   // 3D texture coordinates
}

// Vertex shader output / Fragment shader input
struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) world_position: vec3<f32>,
    @location(1) texcoord: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    var output: VertexOutput;

    // Transform vertex to clip space
    output.clip_position = camera.view_proj * vec4<f32>(input.position, 1.0);
    output.world_position = input.position;
    output.texcoord = input.texcoord;

    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Both textureSample calls must be in uniform control flow, so do them both up front
    let tex_value = textureSample(tile_texture, tile_sampler, input.texcoord).r;
    let adjusted = (tex_value - tile.contrast_min) / (tile.contrast_max - tile.contrast_min);
    let clamped = clamp(adjusted, 0.0, 1.0);
    let color = textureSample(colormap, colormap_sampler, clamped);

    // Debug mode: show tile boundaries with color-coded visualization
    if (tile.debug_mode > 0.5) {
        let tc = input.texcoord;
        let edge_threshold = 0.05;
        let near_edge = tc.x < edge_threshold || tc.x > (1.0 - edge_threshold) ||
                        tc.y < edge_threshold || tc.y > (1.0 - edge_threshold) ||
                        tc.z < edge_threshold || tc.z > (1.0 - edge_threshold);
        if (near_edge) {
            return vec4<f32>(1.0, 1.0, 1.0, 1.0);
        }
        return vec4<f32>(mix(color.rgb, tile.debug_color, 0.3), 1.0);
    }

    return vec4<f32>(color.rgb, 1.0);
}
