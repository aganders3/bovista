struct CameraUniforms {
    view_proj: mat4x4<f32>,
}

struct ImageUniforms {
    contrast_min: f32,
    contrast_max: f32,
    _padding1: f32,
    _padding2: f32,
    plane_position: vec3<f32>,
    _padding3: f32,
    plane_normal: vec3<f32>,
    _padding4: f32,
    volume_size: vec3<f32>,
    _padding5: f32,
}

@group(0) @binding(0)
var<uniform> camera: CameraUniforms;

@group(1) @binding(0)
var<uniform> image_params: ImageUniforms;

@group(1) @binding(1)
var image_texture: texture_3d<f32>;

@group(1) @binding(2)
var image_sampler: sampler;

struct VertexInput {
    @location(0) position: vec2<f32>,  // Unit quad [0,1] x [0,1]
    @location(1) texcoord: vec2<f32>,
}

struct VertexOutput {
    @builtin(position) clip_position: vec4<f32>,
    @location(0) texcoord: vec2<f32>,
    @location(1) world_pos: vec3<f32>,
}

@vertex
fn vs_main(input: VertexInput) -> VertexOutput {
    // Compute basis vectors for the slice plane
    // Choose an up vector that's not parallel to the normal
    let abs_normal = abs(image_params.plane_normal);
    var up: vec3<f32>;
    if abs_normal.y > 0.9 {
        up = vec3<f32>(0.0, 0.0, 1.0);
    } else {
        up = vec3<f32>(0.0, 1.0, 0.0);
    }

    let tangent = normalize(cross(up, image_params.plane_normal));
    let bitangent = normalize(cross(image_params.plane_normal, tangent));

    // Use input.position directly as it's already [0,1] x [0,1]
    // This will map to world coordinates aligned with the texture volume
    let scale_x = image_params.volume_size.x;
    let scale_y = image_params.volume_size.y;

    // Position on the slice plane in world space
    // Map [0,1] quad to volume coordinates [0, size]
    let world_pos = image_params.plane_position +
                    (input.position.x - 0.5) * scale_x * tangent +
                    (input.position.y - 0.5) * scale_y * bitangent;

    var output: VertexOutput;
    output.clip_position = camera.view_proj * vec4<f32>(world_pos, 1.0);
    output.texcoord = input.texcoord;
    output.world_pos = world_pos;
    return output;
}

@fragment
fn fs_main(input: VertexOutput) -> @location(0) vec4<f32> {
    // Convert world position to texture coordinates [0,1]³
    let tex_coord_3d = input.world_pos / image_params.volume_size;

    // DEBUG: Visualize texture coordinates to verify they're correct
    // Uncomment to debug:
    // return vec4<f32>(tex_coord_3d, 1.0);

    // Sample the 3D texture at the computed position
    let raw_value = textureSample(image_texture, image_sampler, tex_coord_3d).r;

    // Apply contrast limits (normalize to 0-1)
    let normalized = (raw_value - image_params.contrast_min) /
                     (image_params.contrast_max - image_params.contrast_min);
    let clamped = clamp(normalized, 0.0, 1.0);

    // Grayscale output
    return vec4<f32>(clamped, clamped, clamped, 1.0);
}
