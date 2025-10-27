// Chunk visual shader - renders 3D positioned texture slices

// Camera uniforms (bind group 0)
struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

// Chunk uniforms (bind group 1)
struct ChunkUniforms {
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
var chunk_texture: texture_3d<f32>;

@group(1) @binding(1)
var chunk_sampler: sampler;

@group(1) @binding(2)
var<uniform> chunk: ChunkUniforms;

// Vertex shader input
struct VertexInput {
    @location(0) position: vec3<f32>,
    @location(1) texcoord: vec3<f32>,
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
    // Debug mode: show chunk boundaries with color-coded layers
    if (chunk.debug_mode > 0.5) {
        // Draw wireframe by checking distance to edges
        let tc = input.texcoord;
        let edge_threshold = 0.05;

        let near_edge = tc.x < edge_threshold || tc.x > (1.0 - edge_threshold) ||
                       tc.y < edge_threshold || tc.y > (1.0 - edge_threshold) ||
                       tc.z < edge_threshold || tc.z > (1.0 - edge_threshold);

        if (near_edge) {
            // White wireframe
            return vec4<f32>(1.0, 1.0, 1.0, 1.0);
        } else {
            // Tint the texture with debug color
            let tex_value = textureSample(chunk_texture, chunk_sampler, input.texcoord).r;
            let adjusted = (tex_value - chunk.contrast_min) / (chunk.contrast_max - chunk.contrast_min);
            let clamped = clamp(adjusted, 0.0, 1.0);

            // Blend texture with debug color
            let tinted = mix(vec3<f32>(clamped, clamped, clamped), chunk.debug_color, 0.3);
            return vec4<f32>(tinted, 1.0);
        }
    }

    // Normal mode: sample the 3D texture at the computed texture coordinates
    let tex_value = textureSample(chunk_texture, chunk_sampler, input.texcoord).r;

    // Apply contrast adjustment
    let adjusted = (tex_value - chunk.contrast_min) / (chunk.contrast_max - chunk.contrast_min);
    let clamped = clamp(adjusted, 0.0, 1.0);

    // Grayscale output
    return vec4<f32>(clamped, clamped, clamped, 1.0);
}
