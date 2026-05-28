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

    let shader_src = if use_vec3_probe {
        SHADER_VEC3_PROBE
    } else if use_uniform {
        SHADER_UNIFORM
    } else {
        SHADER_NO_UNIFORM
    };
    let mode_name = if use_vec3_probe { "vec3-probe" }
                   else if use_uniform { "uniform" }
                   else { "no-uniform" };
    println!("[smoke] mode: {}", mode_name);
    let needs_uniform = use_uniform || use_vec3_probe;
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

    let (uniform_layout, uniform_bg) = if needs_uniform {
        let (size, bytes): (u64, &[u8]) = if use_vec3_probe {
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
            label: Some("smoke-bg"),
            layout: &layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() }],
        });
        (Some(layout), Some(bg))
    } else {
        (None, None)
    };

    let pipeline_layout = uniform_layout.as_ref().map(|bgl| {
        device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("smoke-pl"),
            bind_group_layouts: &[bgl],
            push_constant_ranges: &[],
        })
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("smoke-pipeline"),
        layout: pipeline_layout.as_ref(),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_main"),
            buffers: &[],
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
        if let Some(bg) = &uniform_bg {
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
