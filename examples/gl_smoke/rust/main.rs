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

const SHADER: &str = r#"
struct VsOut {
    @builtin(position) pos: vec4<f32>,
};

@vertex
fn vs_main(@builtin(vertex_index) vi: u32) -> VsOut {
    // Big triangle that covers the viewport (no vertex buffer needed).
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

fn main() {
    env_logger::init();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = args.iter().find(|a| !a.starts_with("--")).cloned()
        .unwrap_or_else(|| "out.png".to_string());
    let backend_str = flag(&args, "--backend").unwrap_or_else(|| "auto".to_string());
    let clear_hex = flag(&args, "--clear").unwrap_or_else(|| "ff0000ff".to_string());

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

    let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
        label: Some("smoke-shader"),
        source: wgpu::ShaderSource::Wgsl(SHADER.into()),
    });

    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("smoke-pipeline"),
        layout: None,
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
        depth_stencil: None,
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
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
        });
        pass.set_pipeline(&pipeline);
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
