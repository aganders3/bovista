//! Minimal wgpu+PNG smoke test.
//!
//! No bovista, no virtual textures, no uniforms, no bind groups. Just:
//!   instance → adapter → device → render pipeline → 1 draw → readback → PNG.
//!
//! Renders a fullscreen triangle in solid green, on top of a configurable
//! clear color. If this fails on a backend where bovista's headless_render
//! also fails, the problem is wgpu+driver, not anything bovista is doing.
//! If this *works* on a backend where bovista's example doesn't, the bug is
//! something bovista's pipeline does that this minimal one doesn't.
//!
//! Usage:
//!   cargo run --release --example gl_smoke -- out.png
//!   cargo run --release --example gl_smoke -- out.png --backend gl
//!   cargo run --release --example gl_smoke -- out.png --backend vulkan
//!   cargo run --release --example gl_smoke -- out.png --clear ff0000ff

use std::sync::mpsc;

const WIDTH:  u32 = 512;
const HEIGHT: u32 = 384;

const SHADER_NO_UNIFORM: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    return vec4<f32>(0.0, 1.0, 0.0, 1.0);
}
"#;

// Same vertex shader, but the fragment color comes from a uniform.
// If --uniform shows the expected magenta on a backend where the no-uniform
// path shows green, uniforms are broken on that backend.
const SHADER_UNIFORM: &str = r#"
struct Probe {
    color: vec4<f32>,
    flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> probe: Probe;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // If uniforms work, fs returns probe.color (we'll set magenta from Rust).
    // If uniforms read as zero, fs returns (0,0,0,0) — transparent black,
    // PNG viewers render that as white/clear-color showing through.
    // flags.x == 0xDEADBEEF lets us cross-check that integer fields also read.
    if probe.flags.x == 0xDEADBEEFu {
        return probe.color;
    }
    // Uniform integer read failed — return solid red to distinguish.
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);
}
"#;

// Same shape as bovista's VolumeUniforms: many vec3<T> + scalar pairs that
// pack in WGSL (16 bytes per pair) but DON'T pack in std140 (32 bytes per
// pair). The Rust side writes WGSL-packed bytes. If naga emits std140 GLSL
// without inserting compensating padding, the shader reads each scalar from
// the wrong offset.
//
// Expected: color = (0.1, 0.3, 0.9) blue-purple if all reads land correctly.
// If any field reads garbage, the color shifts visibly.
// 3D R16Float texture sample probe. Reads voxel (12,12,12) — known value ≈ 0.797.
// If the sampled value is non-zero (gives bright purple-ish), 3D R16Float
// sampling works. If sample returns 0 (black/clear), it's broken.
// 2D-array R32Uint textureLoad probe (matches bovista's page_table sampling).
// Loads texel (layer=2, y=2, x=3) — known value 0xCAFEBABE. If the load works,
// outputs bright magenta; if it returns 0, outputs dim teal.
// Three-bind-group probe. Each group contributes one color channel and one
// magic constant. Output is bright if all three are read correctly.
// Same data as SHADER_THREE_BGS but all three uniforms live in one
// bind group at separate binding slots. If --three-bgs salmoned on HPC
// because wgpu-hal-gles drops bind groups 1 and 2 silently, this should
// come back mid-gray (the expected color when all three reads work).
// VBO + big-struct probe: like --big-struct, but the VS reads vol_min /
// vol_max / view_proj from the UBO and uses them to transform a vertex-buffer
// position into clip space. Mirrors volume.rs's vs_main exactly.
//
// view_proj is identity, vol_min = (-0.9, -0.7, 0.4), vol_max = (0.9, 0.7, 0.6).
// Three vertices (unit positions 0/1 on each axis) cover most of NDC.
// Fragment returns the same channel-encoded reads as --big-struct.
const SHADER_VBO_BIG: &str = r#"
struct BigSub {
    a: vec3<f32>, _p1: f32,
    b: vec3<f32>, _p2: f32,
    c: vec3<f32>, _p3: f32,
}
struct BigInnerVt {
    u0: u32, u1: u32, f0: f32, f1: f32, f2: f32, u2: u32,
    f3: f32, f4: f32, u3: u32, u4: u32, u5: u32, u6: u32,
    lods: array<BigSub, 16>,
}
struct BigInnerVol {
    vol_min: vec3<f32>, rss: f32,
    vol_max: vec3<f32>, density: f32,
    cpos: vec3<f32>, early: f32,
    dims: vec3<u32>, debug_mode: u32,
}
struct BigAll {
    view_proj: mat4x4<f32>,
    vt: BigInnerVt,
    vol: BigInnerVol,
}
@group(0) @binding(0) var<uniform> big: BigAll;

struct VertexInput { @location(0) position: vec3<f32> };
struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(in: VertexInput) -> VsOut {
    var out: VsOut;
    let world = big.vol.vol_min + in.position * (big.vol.vol_max - big.vol.vol_min);
    out.pos = big.view_proj * vec4<f32>(world, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    var r = 0.0;
    if big.vol.debug_mode == 0xCAFEu { r = 0.5; }
    let g = big.vt.lods[5].b.y;
    let b = big.vol.vol_max.z;
    return vec4<f32>(r, g, b, 1.0);
}
"#;

// Big-struct probe: replicates bovista's VolumeAllUniforms layout
// (cross-stage + nested vt with array<Sub,16> + nested vol). Fragment
// outputs (read-OK indicators) per channel.
const SHADER_BIG_STRUCT: &str = r#"
struct BigSub {
    a: vec3<f32>, _p1: f32,
    b: vec3<f32>, _p2: f32,
    c: vec3<f32>, _p3: f32,
}
struct BigInnerVt {
    u0: u32, u1: u32, f0: f32, f1: f32, f2: f32, u2: u32,
    f3: f32, f4: f32, u3: u32, u4: u32, u5: u32, u6: u32,
    lods: array<BigSub, 16>,
}
struct BigInnerVol {
    vol_min: vec3<f32>, rss: f32,
    vol_max: vec3<f32>, density: f32,
    cpos: vec3<f32>, early: f32,
    dims: vec3<u32>, debug_mode: u32,
}
struct BigAll {
    view_proj: mat4x4<f32>,
    vt: BigInnerVt,
    vol: BigInnerVol,
}
@group(0) @binding(0) var<uniform> big: BigAll;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    // Touch fields so the compiler emits them in VS too.
    if big.vol.debug_mode == 0u {
        out.pos.w = 1.0;
    }
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    var r = 0.0;
    if big.vol.debug_mode == 0xCAFEu { r = 0.5; }
    let g = big.vt.lods[5].b.y;     // expect 0.7
    let b = big.vol.vol_max.z;       // expect 0.9
    return vec4<f32>(r, g, b, 1.0);
}
"#;

// Cross-stage uniform probe. VS reads probe.flags.x (forces the compiler
// to declare the uniform block in the VS too); FS reads probe.color.
// Same data layout as SHADER_UNIFORM. If FS returns magenta, cross-stage
// uniforms work. If FS returns red, FS read fails when VS also reads.
const SHADER_CROSS_STAGE: &str = r#"
struct Probe {
    color: vec4<f32>,
    flags: vec4<u32>,
};
@group(0) @binding(0) var<uniform> probe: Probe;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    // Touch probe.flags.x so the compiler emits the uniform block in VS too.
    // The select keeps the output identical regardless of probe contents.
    if probe.flags.x == 0u {
        out.pos.w = 1.0;
    }
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    if probe.flags.x == 0xDEADBEEFu {
        return probe.color;  // magenta if FS uniform read succeeds
    }
    return vec4<f32>(1.0, 0.0, 0.0, 1.0);  // red if it doesn't
}
"#;

// 3D-sample + 2D-array-load probe — exactly the volume shader's broken
// combination after the page-table refactor: textureSampleLevel on a
// 3D float texture AND textureLoad on a 2D-array float texture in the
// same fragment shader.
const SHADER_TEX3D_PLUS_2DARR: &str = r#"
@group(0) @binding(0) var t3d:    texture_3d<f32>;
@group(0) @binding(1) var s3d:    sampler;
@group(0) @binding(2) var t2darr: texture_2d_array<f32>;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    let sampled = textureSampleLevel(t3d, s3d, vec3<f32>(0.75, 0.75, 0.75), 0.0).r;
    let loaded  = textureLoad(t2darr, vec2<i32>(0, 0), 0, 0);
    // sampled ~ 0.797 (R), loaded.r = 0.5 (G channel), loaded.b = 1.0 (B chan)
    // Expected: linear (0.80, 0.50, 1.00, 1.00) — pastel cyan/sky.
    return vec4<f32>(sampled, loaded.r, loaded.b, 1.0);
}
"#;

// Two-texture probe. If both textures read independently we get yellow
// (R from tex A, G from tex B). If they alias to one texture unit on this
// driver, we'd see red-only or green-only.
const SHADER_TWO_TEX: &str = r#"
@group(0) @binding(0) var ta: texture_2d<f32>;
@group(0) @binding(1) var tb: texture_2d<f32>;
@group(0) @binding(2) var sm: sampler;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    let a = textureSampleLevel(ta, sm, vec2<f32>(0.5, 0.5), 0.0).r;
    let b = textureSampleLevel(tb, sm, vec2<f32>(0.5, 0.5), 0.0).r;
    return vec4<f32>(a, b, 0.0, 1.0);
}
"#;

const SHADER_ONE_BG_THREE: &str = r#"
struct GroupUbo {
    magic: u32,
    rgba: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g0: GroupUbo;
@group(0) @binding(1) var<uniform> g1: GroupUbo;
@group(0) @binding(2) var<uniform> g2: GroupUbo;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    var rgba = g0.rgba + g1.rgba + g2.rgba;
    var alpha_ok = 0.0;
    if g0.magic == 0xCAFE0001u { alpha_ok = alpha_ok + 0.34; }
    if g1.magic == 0xCAFE0002u { alpha_ok = alpha_ok + 0.33; }
    if g2.magic == 0xCAFE0003u { alpha_ok = alpha_ok + 0.33; }
    return vec4<f32>(rgba.r, rgba.g, rgba.b, alpha_ok);
}
"#;

const SHADER_THREE_BGS: &str = r#"
struct GroupUbo {
    magic: u32,
    rgba: vec4<f32>,
};
@group(0) @binding(0) var<uniform> g0: GroupUbo;
@group(1) @binding(0) var<uniform> g1: GroupUbo;
@group(2) @binding(0) var<uniform> g2: GroupUbo;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    var rgba = g0.rgba + g1.rgba + g2.rgba;
    var alpha_ok = 0.0;
    if g0.magic == 0xCAFE0001u { alpha_ok = alpha_ok + 0.34; }
    if g1.magic == 0xCAFE0002u { alpha_ok = alpha_ok + 0.33; }
    if g2.magic == 0xCAFE0003u { alpha_ok = alpha_ok + 0.33; }
    return vec4<f32>(rgba.r, rgba.g, rgba.b, alpha_ok);
}
"#;

const SHADER_TEX2DA_UINT: &str = r#"
@group(0) @binding(0) var pt: texture_2d_array<u32>;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    let v = textureLoad(pt, vec2<i32>(3, 2), 2, 0).r;
    if v == 0xCAFEBABEu {
        return vec4<f32>(1.0, 0.0, 1.0, 1.0);  // magenta = read OK
    }
    // Encode the actual value modulo 256 into a channel so we can see what
    // came back instead.
    let lo = f32(v & 0xffu) / 255.0;
    return vec4<f32>(0.0, lo + 0.2, 0.5, 1.0);
}
"#;

const SHADER_TEX3D: &str = r#"
@group(0) @binding(0) var t3d: texture_3d<f32>;
@group(0) @binding(1) var s3d: sampler;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Sample at (0.75, 0.75, 0.75). Expected value ~0.797.
    let v = textureSampleLevel(t3d, s3d, vec3<f32>(0.75, 0.75, 0.75), 0.0).r;
    // Encode the sampled value as color: bright = sample works, dark = sample broken.
    // R = v (so we see ~0.8 red), G = 0.2 baseline so non-zero alpha confirms FS ran.
    return vec4<f32>(v, 0.2, 1.0 - v, 1.0);
}
"#;

const SHADER_VEC3_PROBE: &str = r#"
struct Probe3 {
    a3: vec3<f32>,   // expect (0.1, 0.0, 0.0)
    s1: f32,         // expect 0.3            (packs in WGSL, offset 12)
    b3: vec3<f32>,   // expect (0.0, 0.9, 0.0)
    s2: f32,         // expect 1.0            (packs in WGSL, offset 28)
    c3: vec3<u32>,   // expect (0xCAFEu, 0u, 0u)
    s3: u32,         // expect 0xBABEu        (packs in WGSL, offset 44)
};
@group(0) @binding(0) var<uniform> p: Probe3;

struct VsOut { @builtin(position) pos: vec4<f32> };

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    var ps = array<vec2<f32>, 3>(
        vec2<f32>(-1.0, -1.0),
        vec2<f32>(-1.0,  3.0),
        vec2<f32>( 3.0, -1.0),
    );
    var out: VsOut;
    out.pos = vec4<f32>(ps[vi % 3u], 0.5, 1.0);
    return out;
}

@fragment
fn fs_main() -> @location(0) vec4<f32> {
    // Each field that reads correctly contributes its expected component.
    // If a scalar lands at the wrong offset (std140 layout), it reads as 0
    // and the color channel goes missing. The two integer probes gate red
    // and alpha; if they're 0xCAFE / 0xBABE the corresponding output stays
    // bright. Mac (Metal) reference is bright bluish-purple with full alpha.
    var r: f32 = 0.0;
    var g: f32 = 0.0;
    var b: f32 = 0.0;
    var a: f32 = 0.0;

    // a3.x must be 0.1 (the only red contributor)
    r = p.a3.x;
    // s1 must be 0.3 (offset 12: packed)
    r = r + p.s1;
    // b3.y must be 0.9 (the only green contributor)
    g = p.b3.y;
    // s2 must be 1.0 (offset 28: packed)
    b = p.s2 * 0.9;
    // Integer field reads on the alpha + a tiny red bump
    if p.c3.x == 0xCAFEu {
        a = a + 0.5;
    }
    if p.s3 == 0xBABEu {
        a = a + 0.5;
    }

    return vec4<f32>(r, g, b, a);
}
"#;

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = args.iter().find(|a| !a.starts_with("--")).cloned()
        .unwrap_or_else(|| "out.png".to_string());
    let backend_str = flag(&args, "--backend").unwrap_or_else(|| "auto".to_string());
    let clear_hex = flag(&args, "--clear").unwrap_or_else(|| "ff0000ff".to_string());
    let use_uniform = args.iter().any(|a| a == "--uniform");
    let use_vec3_probe = args.iter().any(|a| a == "--vec3-probe");
    let with_depth = args.iter().any(|a| a == "--depth");
    let use_tex3d = args.iter().any(|a| a == "--tex3d");
    let use_tex2da_uint = args.iter().any(|a| a == "--tex2da-uint");
    let use_three_bgs = args.iter().any(|a| a == "--three-bgs");
    let use_one_bg_three = args.iter().any(|a| a == "--one-bg-three");
    let use_two_tex = args.iter().any(|a| a == "--two-tex");
    let use_cross_stage = args.iter().any(|a| a == "--cross-stage");
    let use_big_struct = args.iter().any(|a| a == "--big-struct");
    let use_vbo_big = args.iter().any(|a| a == "--vbo-big");
    let use_tex3d_plus_2darr = args.iter().any(|a| a == "--tex3d-plus-2darr");

    let backends = match backend_str.as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        "gl"     => wgpu::Backends::GL,
        "metal"  => wgpu::Backends::METAL,
        _        => wgpu::Backends::all(),
    };

    // Disable indirect-call validation (bovista's headless_render needs this
    // workaround on NVIDIA GLES; keep parity here so the smoke test reaches
    // the same code paths).
    let flags = wgpu::InstanceFlags::default() - wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL;

    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        flags,
        ..Default::default()
    });

    let all: Vec<_> = instance.enumerate_adapters(wgpu::Backends::all());
    println!("[smoke] adapters: {}", all.len());
    for a in &all {
        let i = a.get_info();
        println!("  - {} ({:?} {:?})", i.name, i.backend, i.device_type);
    }
    if all.is_empty() {
        eprintln!("[smoke] No adapters. Exiting.");
        std::process::exit(1);
    }

    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    })).expect("request_adapter failed");
    let info = adapter.get_info();
    println!("[smoke] selected: {} ({:?} {:?})", info.name, info.backend, info.device_type);

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("smoke"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
            trace: wgpu::Trace::Off,
        },
    )).expect("request_device failed");

    let format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let color_tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("smoke-color"),
        size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());

    let depth_view = with_depth.then(|| {
        let depth_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke-depth"),
            size: wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Depth24PlusStencil8,  // same as bovista
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            view_formats: &[],
        });
        depth_tex.create_view(&wgpu::TextureViewDescriptor::default())
    });
    println!("[smoke] depth: {}", if with_depth { "on" } else { "off" });

    let shader_src = if use_tex3d_plus_2darr {
        SHADER_TEX3D_PLUS_2DARR
    } else if use_vbo_big {
        SHADER_VBO_BIG
    } else if use_big_struct {
        SHADER_BIG_STRUCT
    } else if use_cross_stage {
        SHADER_CROSS_STAGE
    } else if use_two_tex {
        SHADER_TWO_TEX
    } else if use_one_bg_three {
        SHADER_ONE_BG_THREE
    } else if use_three_bgs {
        SHADER_THREE_BGS
    } else if use_tex2da_uint {
        SHADER_TEX2DA_UINT
    } else if use_tex3d {
        SHADER_TEX3D
    } else if use_vec3_probe {
        SHADER_VEC3_PROBE
    } else if use_uniform {
        SHADER_UNIFORM
    } else {
        SHADER_NO_UNIFORM
    };
    let mode_name = if use_tex3d_plus_2darr { "tex3d-plus-2darr" }
                   else if use_vbo_big { "vbo-big" }
                   else if use_big_struct { "big-struct" }
                   else if use_cross_stage { "cross-stage" }
                   else if use_two_tex { "two-tex" }
                   else if use_one_bg_three { "one-bg-three" }
                   else if use_three_bgs { "three-bgs" }
                   else if use_tex2da_uint { "tex2da-uint" }
                   else if use_tex3d { "tex3d" }
                   else if use_vec3_probe { "vec3-probe" }
                   else if use_uniform { "uniform" }
                   else { "no-uniform" };
    println!("[smoke] mode: {}", mode_name);
    let needs_uniform = use_uniform || use_vec3_probe || use_cross_stage || use_big_struct || use_vbo_big;
    let uniform_visibility = if use_cross_stage || use_big_struct || use_vbo_big {
        wgpu::ShaderStages::VERTEX | wgpu::ShaderStages::FRAGMENT
    } else {
        wgpu::ShaderStages::FRAGMENT
    };
    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smoke-shader"),
        source: wgpu::ShaderSource::Wgsl(shader_src.into()),
    });

    // ── Uniform setup (only used when --uniform) ───────────────────────────
    // Probe { color: vec4<f32>, flags: vec4<u32> } = 32 bytes. We set
    // color=(1,0,1,1) magenta and flags.x=0xDEADBEEF.
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct Probe {
        color: [f32; 4],
        flags: [u32; 4],
    }
    let probe = Probe { color: [1.0, 0.0, 1.0, 1.0], flags: [0xDEADBEEFu32, 0, 0, 0] };
    let probe_bytes: [u8; 32] = unsafe { std::mem::transmute(probe) };

    // Probe3 layout — WGSL packing:
    //   offset 0:  a3.x a3.y a3.z   s1     (16 bytes; vec3+f32 packed)
    //   offset 16: b3.x b3.y b3.z   s2     (16 bytes; vec3+f32 packed)
    //   offset 32: c3.x c3.y c3.z   s3     (16 bytes; vec3<u32>+u32 packed)
    // Total 48 bytes.
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct Probe3Bytes {
        a3: [f32; 3],
        s1: f32,
        b3: [f32; 3],
        s2: f32,
        c3: [u32; 3],
        s3: u32,
    }
    let probe3 = Probe3Bytes {
        a3: [0.1, 0.0, 0.0],
        s1: 0.3,
        b3: [0.0, 0.9, 0.0],
        s2: 1.0,
        c3: [0xCAFEu32, 0, 0],
        s3: 0xBABEu32,
    };
    let probe3_bytes: [u8; 48] = unsafe { std::mem::transmute(probe3) };

    // Big-struct probe — same shape as bovista's VolumeAllUniforms:
    //   view_proj : mat4 (64)
    //   inner_vt  : { 12×u32/f32 header (48) + array<Sub, 16> (16×48 = 768) }
    //   inner_vol : { vec3+f32 × 4 = 64 }
    // Total 944 bytes. We set:
    //   inner_vt.lods[5].tile_scale.y = 0.7
    //   inner_vol.vol_max.z = 0.9
    //   inner_vol.debug_mode = 0xCAFE
    // Shader reads those exact fields, both in VS (just to force the block
    // to declare) and in FS (where output comes from). Returns:
    //   r = (read inner_vol.debug_mode == 0xCAFE)             → 0.5 or 0
    //   g = inner_vt.lods[5].tile_scale.y                     → 0.7 or 0
    //   b = inner_vol.vol_max.z                               → 0.9 or 0
    // Mac reference: green-cyan (linear 0.5, 0.7, 0.9, 1.0).
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct BigSub {
        a: [f32; 3], _p1: f32,
        b: [f32; 3], _p2: f32,
        c: [f32; 3], _p3: f32,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct BigInnerVt {
        u0: u32, u1: u32, f0: f32, f1: f32, f2: f32, u2: u32,
        f3: f32, f4: f32, u3: u32, u4: u32, u5: u32, u6: u32,
        lods: [BigSub; 16],
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct BigInnerVol {
        vol_min: [f32; 3], rss: f32,
        vol_max: [f32; 3], density: f32,
        cpos: [f32; 3], early: f32,
        dims: [u32; 3], debug_mode: u32,
    }
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct BigAll {
        view_proj: [[f32; 4]; 4],
        vt: BigInnerVt,
        vol: BigInnerVol,
    }
    let mut big_lods = [BigSub {
        a: [0.0; 3], _p1: 0.0,
        b: [0.0; 3], _p2: 0.0,
        c: [0.0; 3], _p3: 0.0,
    }; 16];
    big_lods[5].b[1] = 0.7;  // tile_scale.y at index 5
    // For --vbo-big, set vol_min/vol_max so a unit-cube vertex buffer ends up
    // in NDC range with view_proj = identity. Keeps vol_max.z = 0.9 which the
    // fragment shader reads as the blue channel.
    let (vol_min, vol_max) = if use_vbo_big {
        ([-0.9_f32, -0.7, 0.4], [0.9_f32, 0.7, 0.9])
    } else {
        ([0.0_f32, 0.0, 0.0], [0.0_f32, 0.0, 0.9])
    };
    let big = BigAll {
        view_proj: [[1.0, 0.0, 0.0, 0.0],
                    [0.0, 1.0, 0.0, 0.0],
                    [0.0, 0.0, 1.0, 0.0],
                    [0.0, 0.0, 0.0, 1.0]],
        vt: BigInnerVt {
            u0: 0, u1: 0, f0: 0.0, f1: 0.0, f2: 0.0, u2: 0,
            f3: 0.0, f4: 0.0, u3: 0, u4: 0, u5: 0, u6: 0,
            lods: big_lods,
        },
        vol: BigInnerVol {
            vol_min, rss: 0.0,
            vol_max, density: 0.0,
            cpos: [0.0; 3], early: 0.0,
            dims: [0; 3], debug_mode: 0xCAFE,
        },
    };
    let big_bytes: [u8; 944] = unsafe { std::mem::transmute(big) };

    let (uniform_layout, uniform_bg) = if needs_uniform {
        let (size, bytes): (u64, &[u8]) = if use_big_struct || use_vbo_big {
            (944, &big_bytes)
        } else if use_vec3_probe {
            (48, &probe3_bytes)
        } else {
            (32, &probe_bytes)
        };
        let buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke-uniform"),
            size,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        queue.write_buffer(&buf, 0, bytes);
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke-bgl"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: uniform_visibility,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            }],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke-bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
        });
        (Some(layout), Some(bg))
    } else {
        (None, None)
    };

    // ── 3D + 2D-array probe (mirrors volume's textureSampleLevel +
    //    textureLoad combination after the page-table refactor) ──────────────
    let tex3d_plus_2darr = if use_tex3d_plus_2darr {
        const N: u32 = 16;
        let total = (N * N * N) as usize;
        let mut bytes = Vec::with_capacity(total * 2);
        for z in 0..N { for y in 0..N { for x in 0..N {
            let v = (x + y * N + z * N * N) as f32 / (N * N * N) as f32;
            let h = half::f16::from_f32(v).to_bits();
            bytes.push((h & 0xff) as u8); bytes.push((h >> 8) as u8);
        }}}
        let tex3d = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke-tex3d-A"),
            size: wgpu::Extent3d { width: N, height: N, depth_or_array_layers: N },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex3d, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(N * 2), rows_per_image: Some(N) },
            wgpu::Extent3d { width: N, height: N, depth_or_array_layers: N },
        );
        let view3d = tex3d.create_view(&wgpu::TextureViewDescriptor::default());
        let samp3d = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smoke-samp3d-A"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest, min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        // 2D-array Rgba8Unorm with known marker at (layer=0, x=0, y=0)
        // = [0x80, 0x40, 0xFF, 0x00] = (0.5, 0.25, 1.0, 0.0).
        let arr_w: u32 = 4; let arr_h: u32 = 4; let arr_layers: u32 = 2;
        let mut arr_bytes = vec![0u8; (arr_w * arr_h * arr_layers * 4) as usize];
        arr_bytes[0] = 0x80; arr_bytes[1] = 0x40; arr_bytes[2] = 0xff; arr_bytes[3] = 0x00;
        let tex2da = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke-tex2da-A"),
            size: wgpu::Extent3d { width: arr_w, height: arr_h, depth_or_array_layers: arr_layers },
            mip_level_count: 1, sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo { texture: &tex2da, mip_level: 0, origin: wgpu::Origin3d::ZERO, aspect: wgpu::TextureAspect::All },
            &arr_bytes,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(arr_w * 4), rows_per_image: Some(arr_h) },
            wgpu::Extent3d { width: arr_w, height: arr_h, depth_or_array_layers: arr_layers },
        );
        let view2da = tex2da.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke-tex3d-2darr-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry { binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: true }, view_dimension: wgpu::TextureViewDimension::D3, multisampled: false },
                    count: None },
                wgpu::BindGroupLayoutEntry { binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering), count: None },
                wgpu::BindGroupLayoutEntry { binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture { sample_type: wgpu::TextureSampleType::Float { filterable: false }, view_dimension: wgpu::TextureViewDimension::D2Array, multisampled: false },
                    count: None },
            ],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke-tex3d-2darr-bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view3d) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&samp3d) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(&view2da) },
            ],
        });
        Some((layout, bg))
    } else { None };

    // ── 3D texture setup (only used when --tex3d) ────────────────────────
    // Match bovista's atlas: 16x16x16 R16Float volume with a known
    // gradient. The shader samples at uv=(0.75, 0.75, 0.75) and decodes
    // the value to RGB. If the sample returns the expected value, 3D
    // R16Float texture sampling on this driver works. If it returns 0,
    // we've found the bug.
    let (tex3d_layout, tex3d_bg) = if use_tex3d {
        const N: u32 = 16;
        // Build a 16³ R16Float volume where v(x,y,z) = (x + y*N + z*N*N) / N³.
        // At uv=(0.75,0.75,0.75) → voxel (12,12,12) → value = (12 + 12*16 + 12*256)/4096 ≈ 0.797.
        let total = (N * N * N) as usize;
        let mut bytes = Vec::with_capacity(total * 2);
        for z in 0..N { for y in 0..N { for x in 0..N {
            let v = (x + y * N + z * N * N) as f32 / (N * N * N) as f32;
            let h = half::f16::from_f32(v).to_bits();
            bytes.push((h & 0xff) as u8);
            bytes.push((h >> 8) as u8);
        }}}
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke-tex3d"),
            size: wgpu::Extent3d { width: N, height: N, depth_or_array_layers: N },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D3,
            format: wgpu::TextureFormat::R16Float,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &bytes,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(N * 2),
                rows_per_image: Some(N),
            },
            wgpu::Extent3d { width: N, height: N, depth_or_array_layers: N },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor::default());
        let samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smoke-samp3d"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            address_mode_w: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke-bgl3d"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D3,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke-bg3d"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(&samp) },
            ],
        });
        (Some(layout), Some(bg))
    } else {
        (None, None)
    };

    // ── 2D-array R32Uint integer texture probe (matches bovista's page_table) ──
    // 4 layers, each 4×4. Texel at (layer=2, x=3, y=2) is set to 0xCAFEBABE.
    // The shader textureLoad's that location and returns bright if it reads
    // 0xCAFEBABE, dark otherwise.
    let (tex2da_uint_layout, tex2da_uint_bg) = if use_tex2da_uint {
        const W: u32 = 4;
        const H: u32 = 4;
        const LAYERS: u32 = 4;
        let total = (W * H * LAYERS) as usize;
        let mut data = vec![0u32; total];
        // Place a known marker at layer=2, y=2, x=3 (linear index = 2*16 + 2*4 + 3 = 43).
        data[(2 * W * H + 2 * W + 3) as usize] = 0xCAFEBABEu32;
        let tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("smoke-tex2da-uint"),
            size: wgpu::Extent3d { width: W, height: H, depth_or_array_layers: LAYERS },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::R32Uint,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            bytemuck_u32_to_bytes(&data),
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(W * 4),
                rows_per_image: Some(H),
            },
            wgpu::Extent3d { width: W, height: H, depth_or_array_layers: LAYERS },
        );
        let view = tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("smoke-bgl-2da-uint"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Uint,
                    view_dimension: wgpu::TextureViewDimension::D2Array,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("smoke-bg-2da-uint"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            }],
        });
        (Some(layout), Some(bg))
    } else { (None, None) };

    // ── Three-bind-group probe ─────────────────────────────────────────────
    // Mimics bovista's camera/vt/vol layout: 3 bind groups, each with at
    // least one uniform. Shader reads a u32 magic value from each group's
    // uniform and a color contribution. If all three are wired correctly,
    // output is bright yellow-orange.
    #[repr(C)]
    #[derive(Copy, Clone)]
    struct GroupUbo { magic: u32, _pad: [u32; 3], rgba: [f32; 4] }
    let three_bgs_layouts_bgs = if use_three_bgs {
        let make = |magic: u32, rgba: [f32; 4], label: &str| {
            let ubo = GroupUbo { magic, _pad: [0; 3], rgba };
            let bytes: [u8; 32] = unsafe { std::mem::transmute(ubo) };
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &bytes);
            let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
                label: Some(label),
                entries: &[wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                }],
            });
            let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some(label),
                layout: &layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
            });
            (layout, bg)
        };
        // Each group contributes one channel; alpha goes to 1.0 only if all three magics check out.
        let (l0, b0) = make(0xCAFE0001u32, [0.5, 0.0, 0.0, 0.0], "bg0");
        let (l1, b1) = make(0xCAFE0002u32, [0.0, 0.5, 0.0, 0.0], "bg1");
        let (l2, b2) = make(0xCAFE0003u32, [0.0, 0.0, 0.5, 0.0], "bg2");
        Some((l0, b0, l1, b1, l2, b2))
    } else { None };

    // ── Two textures in one bind group probe ───────────────────────────
    // Same bind group has two separate texture bindings. Each texture has
    // a unique single-pixel value. Fragment shader samples both and
    // combines: R from tex A, G from tex B, B = 0, A = 1. If both reads
    // are alive on this driver: bright yellow. If only one is read
    // (whichever happens to win the texture-unit lottery): red or green.
    let two_tex = if use_two_tex {
        let make_tex = |val: u8, label: &str| {
            let t = device.create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::R8Unorm,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &t, mip_level: 0,
                    origin: wgpu::Origin3d::ZERO,
                    aspect: wgpu::TextureAspect::All,
                },
                &[val],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(1), rows_per_image: Some(1) },
                wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
            );
            t.create_view(&wgpu::TextureViewDescriptor::default())
        };
        let view_a = make_tex(0xFF, "texA");  // value = 1.0 (red contributor)
        let view_b = make_tex(0xC0, "texB");  // value ~ 0.75 (green contributor)
        let samp = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("smoke-2tex-samp"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("two-tex-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2, visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
            ],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("two-tex-bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view_a) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&view_b) },
                wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(&samp) },
            ],
        });
        Some((layout, bg))
    } else { None };

    // ── One-bind-group with three uniform entries probe ─────────────────
    // The workaround we're considering for bovista: collapse 3 bind groups
    // into 1 with 3 separate uniform bindings. Same data content as the
    // --three-bgs probe so the expected color is identical.
    let one_bg_three = if use_one_bg_three {
        let make_buf = |magic: u32, rgba: [f32; 4], label: &str| {
            let ubo = GroupUbo { magic, _pad: [0; 3], rgba };
            let bytes: [u8; 32] = unsafe { std::mem::transmute(ubo) };
            let buf = device.create_buffer(&wgpu::BufferDescriptor {
                label: Some(label),
                size: 32,
                usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
                mapped_at_creation: false,
            });
            queue.write_buffer(&buf, 0, &bytes);
            buf
        };
        let b0 = make_buf(0xCAFE0001u32, [0.5, 0.0, 0.0, 0.0], "ubo0");
        let b1 = make_buf(0xCAFE0002u32, [0.0, 0.5, 0.0, 0.0], "ubo1");
        let b2 = make_buf(0xCAFE0003u32, [0.0, 0.0, 0.5, 0.0], "ubo2");
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("one-bg-three-bgl"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
                    count: None,
                },
            ],
        });
        let bg = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("one-bg-three-bg"),
            layout: &layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: b0.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: b1.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: b2.as_entire_binding() },
            ],
        });
        Some((layout, bg))
    } else { None };

    let pipeline_layout = if let Some((l, _)) = &tex3d_plus_2darr {
        Some(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl-3d-2darr"),
            bind_group_layouts: &[l],
            push_constant_ranges: &[],
        }))
    } else if let Some((l, _)) = &two_tex {
        Some(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl-2tex"),
            bind_group_layouts: &[l],
            push_constant_ranges: &[],
        }))
    } else if let Some((l, _)) = &one_bg_three {
        Some(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl-1bg3"),
            bind_group_layouts: &[l],
            push_constant_ranges: &[],
        }))
    } else if let Some((l0, _, l1, _, l2, _)) = &three_bgs_layouts_bgs {
        Some(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl-3bgs"),
            bind_group_layouts: &[l0, l1, l2],
            push_constant_ranges: &[],
        }))
    } else if let Some(bgl) = tex2da_uint_layout.as_ref() {
        Some(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl"),
            bind_group_layouts: &[bgl],
            push_constant_ranges: &[],
        }))
    } else if let Some(bgl) = tex3d_layout.as_ref() {
        Some(device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl"),
            bind_group_layouts: &[bgl],
            push_constant_ranges: &[],
        }))
    } else { uniform_layout.as_ref().map(|bgl| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl"),
            bind_group_layouts: &[bgl],
            push_constant_ranges: &[],
        })
    })};

    // VBO setup for --vbo-big: 3 cube-like vertices forming a triangle
    // spanning most of the NDC region given the vol_min/vol_max we set above.
    let vbo_data: [f32; 9] = [
        0.0, 0.0, 0.5,  // → world (-0.9, -0.7, 0.65)
        1.0, 0.0, 0.5,  // → world ( 0.9, -0.7, 0.65)
        0.5, 1.0, 0.5,  // → world ( 0.0,  0.7, 0.65)
    ];
    let vbo = use_vbo_big.then(|| {
        device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("smoke-vbo"),
            size: std::mem::size_of_val(&vbo_data) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        })
    });
    if let Some(b) = &vbo {
        queue.write_buffer(b, 0, unsafe {
            std::slice::from_raw_parts(vbo_data.as_ptr() as *const u8, std::mem::size_of_val(&vbo_data))
        });
    }
    let vbo_layout = use_vbo_big.then(|| wgpu::VertexBufferLayout {
        array_stride: 12,
        step_mode: wgpu::VertexStepMode::Vertex,
        attributes: &[wgpu::VertexAttribute { format: wgpu::VertexFormat::Float32x3, offset: 0, shader_location: 0 }],
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("smoke-pipeline"),
        layout: pipeline_layout.as_ref(),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: if let Some(l) = vbo_layout.as_ref() { std::slice::from_ref(l) } else { &[] },
            compilation_options: Default::default(),
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_main"),
            targets: &[Some(wgpu::ColorTargetState {
                format,
                blend: None,
                write_mask: wgpu::ColorWrites::ALL,
            })],
            compilation_options: Default::default(),
        }),
        primitive: wgpu::PrimitiveState {
            topology: wgpu::PrimitiveTopology::TriangleList,
            strip_index_format: None,
            front_face: wgpu::FrontFace::Ccw,
            cull_mode: None,
            unclipped_depth: false,
            polygon_mode: wgpu::PolygonMode::Fill,
            conservative: false,
        },
        depth_stencil: if with_depth {
            Some(wgpu::DepthStencilState {
                format: wgpu::TextureFormat::Depth24PlusStencil8,
                depth_write_enabled: false,
                depth_compare: wgpu::CompareFunction::Always,  // never reject
                stencil: wgpu::StencilState::default(),
                bias: wgpu::DepthBiasState::default(),
            })
        } else {
            None
        },
        multisample: wgpu::MultisampleState {
            count: 1,
            mask: !0,
            alpha_to_coverage_enabled: false,
        },
        multiview: None,
        cache: None,
    });

    let clear = parse_clear(&clear_hex);
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("smoke-encoder"),
    });
    {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("smoke-pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view: &color_view,
                resolve_target: None,
                depth_slice: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: depth_view.as_ref().map(|v| wgpu::RenderPassDepthStencilAttachment {
                view: v,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(0),
                    store: wgpu::StoreOp::Store,
                }),
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&pipeline);
        if let Some(b) = &vbo {
            pass.set_vertex_buffer(0, b.slice(..));
        }
        if let Some((_, bg)) = &tex3d_plus_2darr {
            pass.set_bind_group(0, bg, &[]);
        } else if let Some((_, bg)) = &two_tex {
            pass.set_bind_group(0, bg, &[]);
        } else if let Some((_, bg)) = &one_bg_three {
            pass.set_bind_group(0, bg, &[]);
        } else if let Some((_, b0, _, b1, _, b2)) = &three_bgs_layouts_bgs {
            pass.set_bind_group(0, b0, &[]);
            pass.set_bind_group(1, b1, &[]);
            pass.set_bind_group(2, b2, &[]);
        } else if let Some(bg) = &tex2da_uint_bg {
            pass.set_bind_group(0, bg, &[]);
        } else if let Some(bg) = &tex3d_bg {
            pass.set_bind_group(0, bg, &[]);
        } else if let Some(bg) = &uniform_bg {
            pass.set_bind_group(0, bg, &[]);
        }
        pass.draw(0..3, 0..1);
    }

    // Copy color → buffer with 256-byte row alignment.
    let bpp = 4u32;
    let unpadded = WIDTH * bpp;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = ((unpadded + align - 1) / align) * align;
    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("smoke-staging"),
        size: (padded * HEIGHT) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    enc.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: &color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded),
                rows_per_image: Some(HEIGHT),
            },
        },
        wgpu::Extent3d { width: WIDTH, height: HEIGHT, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
    let _ = device.poll(wgpu::PollType::Wait);
    rx.recv().unwrap().expect("buffer map failed");

    let mapped = slice.get_mapped_range();
    let mut rgba = Vec::with_capacity((WIDTH * HEIGHT * 4) as usize);
    for row in 0..HEIGHT as usize {
        let start = row * padded as usize;
        rgba.extend_from_slice(&mapped[start..start + unpadded as usize]);
    }
    // BGRA → RGBA for PNG.
    for c in rgba.chunks_exact_mut(4) { c.swap(0, 2); }

    let file = std::fs::File::create(&output).unwrap();
    let w = std::io::BufWriter::new(file);
    let mut e = png::Encoder::new(w, WIDTH, HEIGHT);
    e.set_color(png::ColorType::Rgba);
    e.set_depth(png::BitDepth::Eight);
    let mut wr = e.write_header().unwrap();
    wr.write_image_data(&rgba).unwrap();
    println!("[smoke] wrote {}", output);
}

fn parse_clear(hex: &str) -> wgpu::Color {
    let h = hex.trim_start_matches('#');
    let p = |i| u8::from_str_radix(&h[i..i+2], 16).unwrap_or(0) as f64 / 255.0;
    if h.len() >= 8 {
        wgpu::Color { r: p(0), g: p(2), b: p(4), a: p(6) }
    } else {
        wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
    }
}

fn bytemuck_u32_to_bytes(data: &[u32]) -> &[u8] {
    unsafe { std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * 4) }
}

fn flag(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name { return args.get(i + 1).cloned(); }
        if let Some(rest) = args[i].strip_prefix(&format!("{}=", name)) {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}
