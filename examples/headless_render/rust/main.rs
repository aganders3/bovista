//! Headless renderer smoke test.
//!
//! Renders a synthetic R16Float sphere through `VolumeVisual` to either:
//!   - a PNG file (single frame), or
//!   - an MP4 via piping raw BGRA into a child `ffmpeg` process (N frames, orbiting camera).
//!
//! Usage:
//!   cargo run --release --example headless_render -- out.png
//!   cargo run --release --example headless_render -- out.mp4 --frames 120 --fps 30
//!   cargo run --release --example headless_render -- out.png --backend vulkan
//!   cargo run --release --example headless_render -- out.png --backend gl
//!
//! Flags:
//!   --width N       (default 1024)
//!   --height N      (default 768)
//!   --frames N      (default 1 for .png, 120 for .mp4)
//!   --fps N         (default 30, only for .mp4)
//!   --backend auto|vulkan|gl  (default auto)

use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::io::Write;

use bovista::visuals::gpu_structs::{TileData, TileKey};
use bovista::visuals::{LodLevelConfig, VolumeVisual};
use bovista::visual::CameraInfo;
use bovista::{Camera, ProjectionMode, Renderer, Scene};

const VOLUME_DIM: u32 = 128;

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    let output = args.iter().find(|a| !a.starts_with("--")).cloned()
        .unwrap_or_else(|| "out.png".to_string());

    let width: u32 = flag_u32(&args, "--width", 1024);
    let height: u32 = flag_u32(&args, "--height", 768);
    let is_video = !output.to_lowercase().ends_with(".png");
    let default_frames: u32 = if is_video { 120 } else { 1 };
    let frames: u32 = flag_u32(&args, "--frames", default_frames);
    let fps: u32 = flag_u32(&args, "--fps", 30);
    let backend = flag_str(&args, "--backend", "auto");
    // --debug-mode 0=normal DVR, 1=LOD wireframes, 2=atlas-direct, 3=step heatmap.
    // Mode 1 is useful for diagnosing all-black GLES output: if wireframes still
    // render, geometry/depth path is fine and the problem is volume sampling.
    let debug_mode = flag_u32(&args, "--debug-mode", 0);
    // Default clear is black; --clear ffff00ff lets you confirm the framebuffer
    // readback path independently of what the volume visual paints.
    let clear_rgba_hex = flag_str(&args, "--clear", "00000000");

    println!("[headless] output={} {}x{} frames={} backend={} debug_mode={} clear={}",
             output, width, height, frames, backend, debug_mode, clear_rgba_hex);

    let backends = match backend.as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        "gl"     => wgpu::Backends::GL,
        _        => wgpu::Backends::VULKAN | wgpu::Backends::GL | wgpu::Backends::METAL | wgpu::Backends::DX12,
    };

    // Disable wgpu's indirect-draw validation compute pipeline: the bovista
    // pipeline never issues indirect draws, and on NVIDIA's surfaceless GLES
    // context the wgpu-23/24/25/26 validation pipeline fails to build
    // (BUFFER_STORAGE/COMPUTE_SHADER missing in the probed feature set),
    // which would fail device creation entirely.
    let flags = wgpu::InstanceFlags::default() - wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends,
        flags,
        ..Default::default()
    });

    // Enumerate every adapter wgpu can see across every backend, so a missing
    // adapter is diagnosable instead of a bare "None". Run with
    //   RUST_LOG=wgpu_hal=debug,wgpu_core=info
    // to get wgpu's own reasons for skipping a backend.
    let all_adapters: Vec<_> = instance.enumerate_adapters(wgpu::Backends::all());
    println!("[headless] enumerate_adapters across all backends → {} adapter(s)", all_adapters.len());
    for a in &all_adapters {
        let i = a.get_info();
        println!("  - {} (backend={:?} type={:?} driver={:?} vendor=0x{:x})",
                 i.name, i.backend, i.device_type, i.driver, i.vendor);
    }
    if all_adapters.is_empty() {
        eprintln!("\n[headless] No adapters at all. On a Linux GPU node, check:");
        eprintln!("   ldconfig -p | grep -E 'libvulkan|libEGL'");
        eprintln!("   ls /usr/share/vulkan/icd.d/      # NVIDIA: nvidia_icd.json");
        eprintln!("   vulkaninfo --summary             # Vulkan health check");
        eprintln!("   eglinfo -B                       # EGL/GL health check (mesa-utils)");
        eprintln!("   nvidia-smi -L                    # GPU visible to driver");
        eprintln!("Re-run this binary with RUST_LOG=wgpu_hal=debug for wgpu's own reasons.");
        std::process::exit(1);
    }

    // Try the user-selected (or auto) backend set, then degrade gracefully.
    let pick_for = |bs: wgpu::Backends| -> Option<wgpu::Adapter> {
        pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok()
        .filter(|a: &wgpu::Adapter| bs.contains(wgpu::Backends::from(a.get_info().backend)))
    };

    let adapter = pick_for(backends)
        .or_else(|| {
            // request_adapter with the requested set returned nothing — fall
            // back to picking any adapter we already enumerated.
            eprintln!("[headless] request_adapter({:?}) returned no adapter; falling back to first enumerated adapter", backends);
            all_adapters.into_iter().next()
        })
        .expect("No usable GPU adapter (enumerate returned some but none picked — should be unreachable)");

    let info = adapter.get_info();
    println!("[headless] selected: {} (backend={:?} type={:?} driver={})",
             info.name, info.backend, info.device_type, info.driver);
    if matches!(info.device_type, wgpu::DeviceType::Cpu) {
        eprintln!("[headless] WARNING: software adapter selected — rendering will be very slow");
    }

    let (device, queue) = pollster::block_on(adapter.request_device(
        &wgpu::DeviceDescriptor {
            label: Some("headless-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
            trace: wgpu::Trace::Off,
        },
    )).expect("Failed to create device");

    let surface_format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let renderer = pollster::block_on(Renderer::new(device, queue, surface_format));

    // Offscreen color target + depth.
    let color_tex = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("headless-color"),
        size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: surface_format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
        view_formats: &[],
    });
    let color_view = color_tex.create_view(&wgpu::TextureViewDescriptor::default());
    let depth_view = renderer.create_depth_texture(width, height);

    // Synthetic 128³ cube as R16Float bytes.
    let tile_bytes = Arc::new(generate_cube_volume_r16f(VOLUME_DIM));
    println!("[headless] synthetic volume: {0}x{0}x{0} R16Float ({1} bytes)",
             VOLUME_DIM, tile_bytes.len());

    // Center the volume on the origin: translation = -half_size on each axis.
    let half = VOLUME_DIM as f32 / 2.0;
    let lod = LodLevelConfig {
        volume_size: (VOLUME_DIM, VOLUME_DIM, VOLUME_DIM),
        tile_size:   (VOLUME_DIM, VOLUME_DIM, VOLUME_DIM),
        voxel_size:  (1.0, 1.0, 1.0),
        scale_factor: 1.0,
        translation: (-half, -half, -half),
    };

    let mut volume = VolumeVisual::new(
        renderer.device(),
        renderer.queue(),
        renderer.surface_format(),
        renderer.camera_bind_group_layout(),
        vec![lod],
        1,
    );
    // Background fill thread: on every poll, snapshot bovista's
    // `wanted` set and push synthetic tiles for everything in it.
    // Polls instead of using a condvar because bovista doesn't notify
    // and the headless example doesn't care about latency below ~20ms.
    {
        let wanted = volume.wanted_handle();
        let pending = volume.pending_chunks().unwrap();
        let tile_bytes = tile_bytes.clone();
        std::thread::spawn(move || loop {
            let keys: Vec<TileKey> = wanted.lock().unwrap().keys().copied().collect();
            for key in keys {
                pending.lock().unwrap().insert(key, TileData {
                    data: (*tile_bytes).clone(),
                    width: VOLUME_DIM,
                    height: VOLUME_DIM,
                    depth: VOLUME_DIM,
                    format: wgpu::TextureFormat::R16Float,
                });
            }
            std::thread::sleep(std::time::Duration::from_millis(20));
        });
    }
    match debug_mode {
        1 => volume.set_debug_mode(true),
        2 => volume.set_atlas_debug_mode(true),
        3 => volume.set_step_debug_mode(true),
        _ => {}
    }

    let mut scene = Scene::new();
    scene.add(Arc::new(Mutex::new(volume)));

    // Camera: orbit at radius ~ 2.5 * half so the volume fills most of the view.
    let radius = half * 2.5;
    let mut camera = Camera::new(width as f32 / height as f32);
    camera.position = glam::Vec3::new(0.0, 0.0, radius);
    camera.target = glam::Vec3::ZERO;
    camera.up = glam::Vec3::Y;
    camera.fov_y = 45.0_f32.to_radians();
    camera.near = 0.1;
    camera.far = radius * 4.0;
    camera.projection_mode = ProjectionMode::Perspective;

    let clear = parse_clear(&clear_rgba_hex);

    // ── Output ──────────────────────────────────────────────────────────────
    if is_video {
        render_to_ffmpeg(&renderer, &mut scene, &mut camera, &color_tex, &color_view,
                          &depth_view, width, height, frames, fps, &output, radius, clear);
    } else {
        render_single_png(&renderer, &mut scene, &mut camera, &color_tex, &color_view,
                          &depth_view, width, height, &output, clear);
    }
}

fn parse_clear(hex: &str) -> wgpu::Color {
    let h = hex.trim_start_matches('#');
    let parse = |i| u8::from_str_radix(&h[i..i+2], 16).unwrap_or(0) as f64 / 255.0;
    if h.len() >= 8 {
        wgpu::Color { r: parse(0), g: parse(2), b: parse(4), a: parse(6) }
    } else {
        wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 }
    }
}

fn render_single_png(
    renderer: &Renderer,
    scene: &mut Scene,
    camera: &Camera,
    color_tex: &wgpu::Texture,
    color_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    width: u32, height: u32,
    output_path: &str,
    clear: wgpu::Color,
) {
    // Warm up: the VT loader runs at the *end* of prepare(), so the tile
    // doesn't get into the atlas until the *next* prepare(). Render 3 frames
    // and keep the last to guarantee the atlas is populated.
    let mut last_bgra: Vec<u8> = Vec::new();
    for frame in 0..3 {
        last_bgra = render_one_frame(renderer, scene, camera, color_tex, color_view,
                                     depth_view, width, height, frame, clear);
    }

    // BGRA → RGBA for PNG.
    let mut rgba = last_bgra;
    for chunk in rgba.chunks_exact_mut(4) {
        chunk.swap(0, 2);
    }

    let file = std::fs::File::create(output_path)
        .unwrap_or_else(|e| panic!("Failed to create {}: {}", output_path, e));
    let w = std::io::BufWriter::new(file);
    let mut encoder = png::Encoder::new(w, width, height);
    encoder.set_color(png::ColorType::Rgba);
    encoder.set_depth(png::BitDepth::Eight);
    let mut writer = encoder.write_header().expect("PNG header");
    writer.write_image_data(&rgba).expect("PNG write");
    println!("[headless] wrote {}", output_path);
}

fn render_to_ffmpeg(
    renderer: &Renderer,
    scene: &mut Scene,
    camera: &mut Camera,
    color_tex: &wgpu::Texture,
    color_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    width: u32, height: u32,
    frames: u32, fps: u32,
    output_path: &str,
    radius: f32,
    clear: wgpu::Color,
) {
    let size_arg = format!("{}x{}", width, height);
    let fps_arg = fps.to_string();
    let mut child = Command::new("ffmpeg")
        .args([
            "-y",
            "-f", "rawvideo",
            "-pix_fmt", "bgra",
            "-s", &size_arg,
            "-r", &fps_arg,
            "-i", "-",
            "-an",
            "-c:v", "libx264",
            "-pix_fmt", "yuv420p",
            "-preset", "fast",
            "-movflags", "+faststart",
            output_path,
        ])
        .stdin(Stdio::piped())
        .spawn()
        .expect("Failed to spawn ffmpeg — is it on PATH?");

    let mut stdin = child.stdin.take().expect("ffmpeg stdin");

    // Two warm-up frames so the atlas is populated before frame 0 ships.
    for warmup in 0..2 {
        let _ = render_one_frame(renderer, scene, camera, color_tex, color_view,
                                 depth_view, width, height, warmup, clear);
    }

    for frame in 0..frames {
        let t = frame as f32 / frames as f32;
        let theta = t * std::f32::consts::TAU;
        camera.position = glam::Vec3::new(radius * theta.cos(), radius * 0.25, radius * theta.sin());

        let bgra = render_one_frame(renderer, scene, camera, color_tex, color_view,
                                    depth_view, width, height, (frame + 2) as u64, clear);
        if let Err(e) = stdin.write_all(&bgra) {
            eprintln!("[headless] ffmpeg stdin closed at frame {}: {}", frame, e);
            break;
        }
    }
    drop(stdin);
    let status = child.wait().expect("ffmpeg wait");
    println!("[headless] ffmpeg exited with {}, wrote {}", status, output_path);
}

fn render_one_frame(
    renderer: &Renderer,
    scene: &mut Scene,
    camera: &Camera,
    color_tex: &wgpu::Texture,
    color_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    width: u32, height: u32,
    frame_number: u64,
    clear: wgpu::Color,
) -> Vec<u8> {
    let _ = frame_number;
    let device = renderer.device();
    let queue = renderer.queue();

    renderer.update_camera(camera);

    let camera_info = CameraInfo {
        position: camera.position,
        target: camera.target,
        fov_y: camera.fov_y,
        viewport_width: width,
        viewport_height: height,
        frustum: camera.frustum_planes(),
        projection_mode: camera.projection_mode,
        ortho_height: camera.ortho_height,
        view_proj: camera.view_projection_matrix(),
    };
    scene.prepare(device, queue, &camera_info);

    renderer.render(scene, color_view, depth_view, clear);

    // Copy color texture → staging buffer with 256-byte row alignment.
    let bytes_per_pixel = 4u32;
    let unpadded_row = width * bytes_per_pixel;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded_row = ((unpadded_row + align - 1) / align) * align;
    let buffer_size = (padded_row * height) as u64;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("headless-readback"),
        size: buffer_size,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });

    let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("headless-copy"),
    });
    encoder.copy_texture_to_buffer(
        wgpu::TexelCopyTextureInfo {
            texture: color_tex,
            mip_level: 0,
            origin: wgpu::Origin3d::ZERO,
            aspect: wgpu::TextureAspect::All,
        },
        wgpu::TexelCopyBufferInfo {
            buffer: &staging,
            layout: wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(padded_row),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(encoder.finish()));

    // Map and strip per-row padding.
    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
    let _ = device.poll(wgpu::PollType::Wait);
    rx.recv().unwrap().expect("buffer map failed");

    let mapped = slice.get_mapped_range();
    let row_bytes = unpadded_row as usize;
    let mut output = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * padded_row as usize;
        output.extend_from_slice(&mapped[start..start + row_bytes]);
    }
    drop(mapped);
    staging.unmap();
    output
}

/// Build a 128³ axis-aligned cube as little-endian f16 bytes. The cube occupies
/// the centered region [0.2·dim, 0.8·dim) on each axis, with an asymmetric
/// notch cut out of one corner so camera rotation is visually obvious.
fn generate_cube_volume_r16f(dim: u32) -> Vec<u8> {
    let n = dim as usize;
    let d = dim as f32;
    let lo = (d * 0.20) as usize;
    let hi = (d * 0.80) as usize;
    // Asymmetric notch: remove the +x +y +z corner of the cube so the result
    // is chiral and rotation is unambiguous.
    let notch_lo = (d * 0.55) as usize;
    let notch_hi = hi;

    let one = half::f16::from_f32(1.0).to_bits();
    let zero = half::f16::from_f32(0.0).to_bits();

    let mut out = Vec::with_capacity(n * n * n * 2);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let in_cube  = x >= lo && x < hi && y >= lo && y < hi && z >= lo && z < hi;
                let in_notch = x >= notch_lo && x < notch_hi
                            && y >= notch_lo && y < notch_hi
                            && z >= notch_lo && z < notch_hi;
                let bits = if in_cube && !in_notch { one } else { zero };
                out.push((bits & 0xff) as u8);
                out.push((bits >> 8) as u8);
            }
        }
    }
    out
}

// ── tiny arg parsing ─────────────────────────────────────────────────────────
fn flag_u32(args: &[String], name: &str, default: u32) -> u32 {
    flag_str_opt(args, name).and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn flag_str(args: &[String], name: &str, default: &str) -> String {
    flag_str_opt(args, name).unwrap_or_else(|| default.to_string())
}
fn flag_str_opt(args: &[String], name: &str) -> Option<String> {
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        if let Some(rest) = args[i].strip_prefix(&format!("{}=", name)) {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}
