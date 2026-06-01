//! Orbiting OME-Zarr stream — renders a volume with an orbiting camera, encodes
//! the frames as fragmented MP4 via ffmpeg, and serves the live stream over
//! plain HTTP. Connect via:
//!
//!   `http://localhost:<port>/`  in a browser (`<video>` plays fMP4 natively)
//!   or in QuickTime ("File → Open Location..."), or `mpv http://...`.
//!
//! With no `--zarr` flag, a synthetic 128³ notched cube is rendered. Pass
//! `--zarr <path-or-url>` to load any OME-Zarr v0.4 (zarr v2) or v0.5 (zarr v3)
//! multiscale pyramid. Filesystem paths (e.g. NFS mounts) and http(s):// URLs
//! are both supported.
//!
//! Usage:
//!   cargo run --release --example orbit_stream
//!   cargo run --release --example orbit_stream -- --zarr /nfs/.../foo.ome.zarr
//!   cargo run --release --example orbit_stream -- --zarr https://… --port 9090
//!
//! Flags:
//!   --zarr PATH-OR-URL   OME-Zarr root; omit for synthetic cube
//!   --width N            (default 1024)
//!   --height N           (default 768)
//!   --fps N              (default 30)
//!   --port N             (default 8080)
//!   --backend NAME       auto | vulkan | gl | metal  (default auto)
//!   --cache-tiles N      atlas slot count (default 1 synthetic, 1024 zarr)
//!   --contrast-min F     contrast window floor (default 0.0)
//!   --contrast-max F     contrast window ceiling (default 1.0)
//!   --density-mult F     multiplier on the auto-density heuristic (default 1.0)
//!
//! Set BOVISTA_FORCE_LIBX264=1 to skip the NVENC probe and always use libx264.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bovista::visual::CameraInfo;
use bovista::visuals::gpu_structs::{ChunkStatus, TileData, TileKey, TileLoaderFn, TileRequest};
use bovista::visuals::virtual_texture::PendingChunks;
use bovista::visuals::{LodLevelConfig, VolumeVisual};
use bovista::{Camera, ProjectionMode, Renderer, Scene};

const VOLUME_DIM: u32 = 128;

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    const KNOWN_FLAGS: &[&str] = &[
        "--width", "--height", "--fps", "--port", "--backend",
        "--zarr", "--cache-tiles",
        "--contrast-min", "--contrast-max", "--density-mult",
    ];
    check_unknown_flags(&args, KNOWN_FLAGS);
    let width: u32 = flag_u32(&args, "--width", 1024);
    let height: u32 = flag_u32(&args, "--height", 768);
    let fps: u32 = flag_u32(&args, "--fps", 30);
    let port: u16 = flag_u32(&args, "--port", 8080) as u16;
    let backend = flag_str(&args, "--backend", "auto");
    let zarr_arg: Option<String> = flag_str_opt(&args, "--zarr");
    let cache_override: Option<u32> = flag_str_opt(&args, "--cache-tiles").and_then(|v| v.parse().ok());
    let contrast_min: f32 = flag_str_opt(&args, "--contrast-min").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let contrast_max: f32 = flag_str_opt(&args, "--contrast-max").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let density_mult: f32 = flag_str_opt(&args, "--density-mult").and_then(|v| v.parse().ok()).unwrap_or(1.0);

    println!("[orbit] {}x{} @ {} fps, serving on port {}, backend={}",
             width, height, fps, port, backend);
    if let Some(z) = &zarr_arg {
        println!("[orbit] OME-Zarr source: {}", z);
    } else {
        println!("[orbit] no --zarr given; rendering synthetic 128³ cube");
    }

    // ── wgpu init ───────────────────────────────────────────────────────────
    let backends = match backend.as_str() {
        "vulkan" => wgpu::Backends::VULKAN,
        "gl"     => wgpu::Backends::GL,
        "metal"  => wgpu::Backends::METAL,
        _        => wgpu::Backends::all(),
    };
    let flags = wgpu::InstanceFlags::default() - wgpu::InstanceFlags::VALIDATION_INDIRECT_CALL;
    let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
        backends, flags, ..Default::default()
    });
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::HighPerformance,
        compatible_surface: None,
        force_fallback_adapter: false,
    })).expect("no GPU adapter found");
    let info = adapter.get_info();
    println!("[orbit] adapter: {} ({:?}, type {:?})", info.name, info.backend, info.device_type);
    // On GL backends, driver_info contains the GL_VERSION + GLSL_VERSION
    // strings. If you see "via Cg compiler" the EGL profile-mask patch
    // didn't take effect — modern NVIDIA shows "4.6.0 NVIDIA …" instead.
    println!("[orbit] driver: {} / {}", info.driver, info.driver_info);

    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("orbit-device"),
        required_features: wgpu::Features::empty(),
        required_limits: wgpu::Limits::default(),
        memory_hints: Default::default(),
        trace: wgpu::Trace::Off,
    })).expect("request_device failed");

    let surface_format = wgpu::TextureFormat::Bgra8UnormSrgb;
    let renderer = pollster::block_on(Renderer::new(device, queue, surface_format));

    // Offscreen render targets
    let color_tex = renderer.device().create_texture(&wgpu::TextureDescriptor {
        label: Some("orbit-color"),
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

    // ── Scene setup: synthetic cube or OME-Zarr pyramid ─────────────────────
    let setup = match &zarr_arg {
        Some(path) => match ome_zarr::open(path) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[orbit] failed to open OME-Zarr at {}: {}", path, e);
                std::process::exit(1);
            }
        },
        None => synthetic_setup(),
    };
    let cache_capacity = cache_override.unwrap_or(setup.cache_capacity);

    let volume = VolumeVisual::new(
        renderer.device(), renderer.queue(), renderer.surface_format(),
        renderer.camera_bind_group_layout(),
        setup.lods.clone(), cache_capacity as usize, setup.loader,
    );
    *setup.pending_slot.lock().unwrap() = Some(volume.pending_chunks().unwrap());

    // Density scales with voxel-to-world ratio so the same visible opacity
    // works whether voxels are 1 mm or 20 nm. Matches the Python OME-Zarr
    // example's auto-density heuristic.
    let lod0 = &setup.lods[0];
    let voxel_min = lod0.voxel_size.0
        .min(lod0.voxel_size.1)
        .min(lod0.voxel_size.2)
        .max(1e-9);
    let base_density = 0.5 / voxel_min;
    println!(
        "[orbit] base_density={:.2} (× {} init); contrast=[{:.3}, {:.3}] init",
        base_density, density_mult, contrast_min, contrast_max,
    );

    // Shared view state — the render loop reads each frame, HTTP `/set`
    // writes from slider events on the served HTML page.
    let view_state = Arc::new(ViewState::new(
        1.0,           // zoom (× world_max for camera radius)
        contrast_min,
        contrast_max,
        density_mult,
        1.0,           // orbit_speed (× 1 revolution per 12s)
    ));

    let volume_arc = Arc::new(Mutex::new(volume));
    let mut scene = Scene::new();
    scene.add(volume_arc.clone());

    // Camera setup. Radius is recomputed each frame from view_state.zoom().
    let (wz, wy, wx) = setup.world_extents;
    let world_max = wz.max(wy).max(wx);
    let mut camera = Camera::new(width as f32 / height as f32);
    camera.target = glam::Vec3::ZERO;
    camera.up = glam::Vec3::Y;
    camera.fov_y = 45.0_f32.to_radians();
    // Scale clip planes to the world extent so volumes with arbitrary voxel
    // units (microns, meters, mm, …) all fit comfortably between near and far.
    camera.near = (world_max * 0.001).max(1e-6);
    camera.far  = world_max * 100.0;
    camera.projection_mode = ProjectionMode::Perspective;

    // ── HTTP server ─────────────────────────────────────────────────────────
    // Routes (single accept loop, thread-per-connection):
    //   GET /              → HTML page with <video> + sliders
    //   GET /stream        → fragmented MP4 (single client)
    //   GET /set?key=val…  → update ViewState, returns 204
    //
    // The render thread (this thread) holds a single ChildStdin in `active`
    // for the current /stream client; idles when no client is connected.
    let active: Arc<Mutex<Option<ChildStdin>>> = Arc::new(Mutex::new(None));
    spawn_http_server(port, width, height, fps, active.clone(), view_state.clone());

    println!("[orbit] ready; open http://localhost:{}/ (Ctrl-C to stop)", port);

    // ── Render loop ─────────────────────────────────────────────────────────
    let frame_dt = Duration::from_secs_f32(1.0 / fps as f32);
    let idle_sleep = Duration::from_millis(100);
    // Orbit angle accumulator (so changing orbit speed mid-flight doesn't
    // teleport the camera — only changes its rate going forward).
    let mut theta: f32 = 0.0;
    let mut last_tick = Instant::now();

    loop {
        // Skip the render entirely when nobody's watching.
        if active.lock().unwrap().is_none() {
            std::thread::sleep(idle_sleep);
            last_tick = Instant::now();
            continue;
        }

        let tick = Instant::now();
        let dt = tick.duration_since(last_tick).as_secs_f32();
        last_tick = tick;

        // Apply current view state to the volume (cheap; writes only on next
        // prepare() submission).
        {
            let mut v = volume_arc.lock().unwrap();
            v.set_contrast_limits(view_state.contrast_min(), view_state.contrast_max());
            v.set_density_scale(base_density * view_state.density_mult());
        }

        // Camera orbit (one revolution per 12s × orbit_speed).
        theta += dt * (std::f32::consts::TAU / 12.0) * view_state.orbit_speed();
        let radius = world_max * 2.0 * view_state.zoom();
        camera.position = glam::Vec3::new(
            radius * theta.cos(),
            radius * 0.30,
            radius * theta.sin(),
        );

        let bgra = render_one_frame(
            &renderer, &mut scene, &camera,
            &color_tex, &color_view, &depth_view,
            width, height,
        );

        // Push the frame to the current client (if still attached). On any
        // write error, drop our handle to the stdin so the accept thread can
        // claim the slot for the next client.
        let write_err = {
            let mut slot = active.lock().unwrap();
            match slot.as_mut() {
                Some(stdin) => stdin.write_all(&bgra).err(),
                None => None,
            }
        };
        if let Some(e) = write_err {
            eprintln!("[orbit] dropping client: ffmpeg stdin write failed ({})", e);
            *active.lock().unwrap() = None;
        }

        let elapsed = tick.elapsed();
        if elapsed < frame_dt {
            std::thread::sleep(frame_dt - elapsed);
        }
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Render

fn render_one_frame(
    renderer: &Renderer,
    scene: &mut Scene,
    camera: &Camera,
    color_tex: &wgpu::Texture,
    color_view: &wgpu::TextureView,
    depth_view: &wgpu::TextureView,
    width: u32, height: u32,
) -> Vec<u8> {
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

    renderer.render(scene, color_view, depth_view, wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 });

    // Copy → staging, map, strip padding.
    let bpp = 4u32;
    let unpadded = width * bpp;
    let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
    let padded = ((unpadded + align - 1) / align) * align;

    let staging = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("orbit-readback"),
        size: (padded * height) as u64,
        usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
        mapped_at_creation: false,
    });
    let mut enc = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
        label: Some("orbit-copy"),
    });
    enc.copy_texture_to_buffer(
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
                bytes_per_row: Some(padded),
                rows_per_image: Some(height),
            },
        },
        wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
    );
    queue.submit(std::iter::once(enc.finish()));

    let slice = staging.slice(..);
    let (tx, rx) = std::sync::mpsc::channel();
    slice.map_async(wgpu::MapMode::Read, move |r| { tx.send(r).unwrap(); });
    let _ = device.poll(wgpu::PollType::Wait);
    rx.recv().unwrap().expect("buffer map failed");

    let mapped = slice.get_mapped_range();
    let row_bytes = unpadded as usize;
    let mut out = Vec::with_capacity(row_bytes * height as usize);
    for row in 0..height as usize {
        let start = row * padded as usize;
        out.extend_from_slice(&mapped[start..start + row_bytes]);
    }
    drop(mapped);
    staging.unmap();
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// Scene setup (synthetic + OME-Zarr share this shape)

struct SceneSetup {
    lods: Vec<LodLevelConfig>,
    loader: TileLoaderFn,
    pending_slot: Arc<Mutex<Option<PendingChunks>>>,
    /// World-space extents of LOD 0 along (z, y, x). Used to size the camera orbit.
    world_extents: (f32, f32, f32),
    /// Default atlas slot count for the visual; CLI `--cache-tiles` overrides.
    cache_capacity: u32,
}

fn synthetic_setup() -> SceneSetup {
    let tile_bytes = Arc::new(generate_cube_volume_r16f(VOLUME_DIM));
    let pending_slot: Arc<Mutex<Option<PendingChunks>>> = Arc::new(Mutex::new(None));
    let pending_for_loader = pending_slot.clone();
    let tile_bytes_for_loader = tile_bytes.clone();
    let loader: TileLoaderFn = Box::new(move |req: TileRequest| -> ChunkStatus {
        let Some(pc) = pending_for_loader.lock().unwrap().clone() else {
            return ChunkStatus::Rejected;
        };
        let key = TileKey {
            lod_level: req.lod_level.unwrap_or(0),
            z: req.z, y: req.y, x: req.x,
        };
        pc.lock().unwrap().insert(key, TileData {
            data: (*tile_bytes_for_loader).clone(),
            width: VOLUME_DIM, height: VOLUME_DIM, depth: VOLUME_DIM,
            format: wgpu::TextureFormat::R16Float,
        });
        ChunkStatus::Accepted
    });
    let half = VOLUME_DIM as f32 / 2.0;
    let lod = LodLevelConfig {
        volume_size: (VOLUME_DIM, VOLUME_DIM, VOLUME_DIM),
        tile_size:   (VOLUME_DIM, VOLUME_DIM, VOLUME_DIM),
        voxel_size:  (1.0, 1.0, 1.0),
        scale_factor: 1.0,
        translation: (-half, -half, -half),
    };
    SceneSetup {
        lods: vec![lod],
        loader,
        pending_slot,
        world_extents: (VOLUME_DIM as f32, VOLUME_DIM as f32, VOLUME_DIM as f32),
        cache_capacity: 1,
    }
}

fn generate_cube_volume_r16f(dim: u32) -> Vec<u8> {
    let n = dim as usize;
    let d = dim as f32;
    let lo = (d * 0.20) as usize;
    let hi = (d * 0.80) as usize;
    let notch_lo = (d * 0.55) as usize;
    let one = half::f16::from_f32(1.0).to_bits();
    let zero = half::f16::from_f32(0.0).to_bits();
    let mut out = Vec::with_capacity(n * n * n * 2);
    for z in 0..n {
        for y in 0..n {
            for x in 0..n {
                let in_cube  = x >= lo && x < hi && y >= lo && y < hi && z >= lo && z < hi;
                let in_notch = x >= notch_lo && x < hi && y >= notch_lo && y < hi && z >= notch_lo && z < hi;
                let bits = if in_cube && !in_notch { one } else { zero };
                out.push((bits & 0xff) as u8);
                out.push((bits >> 8) as u8);
            }
        }
    }
    out
}

// ─────────────────────────────────────────────────────────────────────────────
// ffmpeg

fn spawn_ffmpeg(width: u32, height: u32, fps: u32) -> Child {
    let size = format!("{}x{}", width, height);
    let rate = fps.to_string();

    let force_libx264 = std::env::var("BOVISTA_FORCE_LIBX264").is_ok();
    let nvenc_available = !force_libx264 && probe_encoder("h264_nvenc");

    // Keyframe every second so each fMP4 fragment lands within ~1s and the
    // browser can start decoding without waiting on a long GOP.
    let gop = fps.to_string();
    let (vcodec, codec_args): (&str, Vec<String>) = if nvenc_available {
        ("h264_nvenc", vec![
            "-preset".into(),     "p1".into(),
            "-tune".into(),       "ll".into(),
            "-rc".into(),         "vbr".into(),
            "-b:v".into(),        "8M".into(),
            "-g".into(),          gop.clone(),
            "-keyint_min".into(), gop.clone(),
            "-no-scenecut".into(),"1".into(),
        ])
    } else {
        ("libx264", vec![
            "-preset".into(),     "ultrafast".into(),
            "-tune".into(),       "zerolatency".into(),
            "-crf".into(),        "23".into(),
            "-g".into(),          gop.clone(),
            "-keyint_min".into(), gop.clone(),
            "-x264-params".into(),"scenecut=0".into(),
        ])
    };
    println!("[orbit] ffmpeg encoder: {} (GOP={})", vcodec, gop);

    let mut cmd = Command::new("ffmpeg");
    cmd.args([
        "-y", "-loglevel", "warning", "-hide_banner",
        "-fflags", "+nobuffer",
        "-f", "rawvideo", "-pix_fmt", "bgra",
        "-s", &size, "-r", &rate, "-i", "-",
        "-an",
        "-c:v", vcodec,
    ])
    .args(&codec_args)
    .args([
        "-pix_fmt", "yuv420p",
        "-f", "mp4",
        // Fragmented MP4 so the stream is playable from the first fragment.
        // frag_every_frame keeps fragments tiny so latency tracks ~1/fps.
        "-movflags", "+frag_keyframe+empty_moov+default_base_moof+omit_tfhd_offset+frag_every_frame",
        "-flush_packets", "1",
        "-",
    ])
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::inherit());

    cmd.spawn().expect("ffmpeg failed to spawn — is it on PATH?")
}

fn probe_encoder(name: &str) -> bool {
    let out = Command::new("ffmpeg")
        .args(["-hide_banner", "-loglevel", "error", "-encoders"])
        .stderr(Stdio::null())
        .output();
    match out {
        Ok(o) => String::from_utf8_lossy(&o.stdout)
            .lines()
            .any(|l| l.split_whitespace().nth(1) == Some(name)),
        Err(_) => false,
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// Shared view state — atomic f32-as-bits so HTTP handlers and the render
// thread can both touch it without a mutex on the hot path.

struct ViewState {
    zoom:         AtomicU32,
    contrast_min: AtomicU32,
    contrast_max: AtomicU32,
    density_mult: AtomicU32,
    orbit_speed:  AtomicU32,
}

impl ViewState {
    fn new(zoom: f32, cmin: f32, cmax: f32, dens: f32, speed: f32) -> Self {
        Self {
            zoom:         AtomicU32::new(zoom.to_bits()),
            contrast_min: AtomicU32::new(cmin.to_bits()),
            contrast_max: AtomicU32::new(cmax.to_bits()),
            density_mult: AtomicU32::new(dens.to_bits()),
            orbit_speed:  AtomicU32::new(speed.to_bits()),
        }
    }
    fn zoom(&self)         -> f32 { f32::from_bits(self.zoom.load(Ordering::Relaxed)) }
    fn contrast_min(&self) -> f32 { f32::from_bits(self.contrast_min.load(Ordering::Relaxed)) }
    fn contrast_max(&self) -> f32 { f32::from_bits(self.contrast_max.load(Ordering::Relaxed)) }
    fn density_mult(&self) -> f32 { f32::from_bits(self.density_mult.load(Ordering::Relaxed)) }
    fn orbit_speed(&self)  -> f32 { f32::from_bits(self.orbit_speed.load(Ordering::Relaxed)) }

    /// Apply `key=value` from the /set query string. Unknown keys ignored.
    fn apply(&self, key: &str, value: f32) {
        let cell = match key {
            "zoom"         => &self.zoom,
            "contrast_min" => &self.contrast_min,
            "contrast_max" => &self.contrast_max,
            "density_mult" => &self.density_mult,
            "orbit_speed"  => &self.orbit_speed,
            _ => return,
        };
        cell.store(value.to_bits(), Ordering::Relaxed);
    }
}

// ─────────────────────────────────────────────────────────────────────────────
// HTTP server.
//   GET /              → HTML control page (video + sliders)
//   GET /stream        → fragmented MP4 (one client at a time; fresh ffmpeg
//                        spawned per connection so each gets a moov + IDR).
//   GET /set?k=v&…     → mutate ViewState; returns 204
//
// One thread per connection so slider updates aren't blocked by the long-lived
// /stream client.

fn spawn_http_server(
    port: u16,
    width: u32,
    height: u32,
    fps: u32,
    active: Arc<Mutex<Option<ChildStdin>>>,
    view_state: Arc<ViewState>,
) {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .unwrap_or_else(|e| panic!("bind 0.0.0.0:{} failed: {}", port, e));

    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(socket) = incoming else { continue };
            let active = active.clone();
            let view_state = view_state.clone();
            std::thread::spawn(move || {
                handle_request(socket, width, height, fps, active, view_state);
            });
        }
    });
}

fn handle_request(
    mut socket: TcpStream,
    width: u32, height: u32, fps: u32,
    active: Arc<Mutex<Option<ChildStdin>>>,
    view_state: Arc<ViewState>,
) {
    // Read the request line + headers (up to first blank line).
    let _ = socket.set_read_timeout(Some(Duration::from_millis(500)));
    let mut buf = [0u8; 2048];
    let n = socket.read(&mut buf).unwrap_or(0);
    let _ = socket.set_read_timeout(None);
    if n == 0 {
        return;
    }
    let request = String::from_utf8_lossy(&buf[..n]);
    let path = request.lines().next()
        .and_then(|line| line.split_whitespace().nth(1))
        .unwrap_or("/");

    if path == "/" || path.starts_with("/?") {
        serve_html(&mut socket, &view_state);
    } else if path == "/stream" {
        serve_stream(&mut socket, width, height, fps, active);
    } else if let Some(query) = path.strip_prefix("/set?") {
        serve_set(&mut socket, query, &view_state);
    } else {
        let _ = socket.write_all(
            b"HTTP/1.0 404 Not Found\r\n\
              Content-Length: 0\r\n\
              Connection: close\r\n\r\n",
        );
    }
}

fn serve_html(socket: &mut TcpStream, view: &ViewState) {
    let body = INDEX_HTML
        .replace("{{ZOOM}}",         &format!("{}", view.zoom()))
        .replace("{{CONTRAST_MIN}}", &format!("{}", view.contrast_min()))
        .replace("{{CONTRAST_MAX}}", &format!("{}", view.contrast_max()))
        .replace("{{DENSITY_MULT}}", &format!("{}", view.density_mult()))
        .replace("{{ORBIT_SPEED}}",  &format!("{}", view.orbit_speed()));
    let header = format!(
        "HTTP/1.0 200 OK\r\n\
         Content-Type: text/html; charset=utf-8\r\n\
         Content-Length: {}\r\n\
         Cache-Control: no-store\r\n\
         Connection: close\r\n\r\n",
        body.len(),
    );
    let _ = socket.write_all(header.as_bytes());
    let _ = socket.write_all(body.as_bytes());
}

fn serve_stream(
    socket: &mut TcpStream,
    width: u32, height: u32, fps: u32,
    active: Arc<Mutex<Option<ChildStdin>>>,
) {
    if active.lock().unwrap().is_some() {
        let _ = socket.write_all(
            b"HTTP/1.0 503 Service Unavailable\r\n\
              Content-Type: text/plain\r\n\
              Connection: close\r\n\r\n\
              Another client is already streaming.\n",
        );
        return;
    }

    let mut ffmpeg = spawn_ffmpeg(width, height, fps);
    let stdin = ffmpeg.stdin.take().expect("ffmpeg stdin");
    let mut stdout = ffmpeg.stdout.take().expect("ffmpeg stdout");

    *active.lock().unwrap() = Some(stdin);

    println!("[orbit] HTTP /stream client connected; streaming fMP4");
    let header_ok = socket.write_all(
        b"HTTP/1.0 200 OK\r\n\
          Content-Type: video/mp4\r\n\
          Cache-Control: no-store\r\n\
          Access-Control-Allow-Origin: *\r\n\
          Connection: close\r\n\r\n",
    ).is_ok();

    if header_ok {
        let _ = std::io::copy(&mut stdout, socket);
    }

    *active.lock().unwrap() = None;
    let _ = ffmpeg.kill();
    let _ = ffmpeg.wait();
    println!("[orbit] /stream client disconnected; idle");
}

fn serve_set(socket: &mut TcpStream, query: &str, view: &ViewState) {
    for pair in query.split('&') {
        if let Some((k, v)) = pair.split_once('=') {
            if let Ok(f) = v.parse::<f32>() {
                view.apply(k, f);
            }
        }
    }
    let _ = socket.write_all(
        b"HTTP/1.0 204 No Content\r\n\
          Cache-Control: no-store\r\n\
          Access-Control-Allow-Origin: *\r\n\
          Connection: close\r\n\r\n",
    );
}

const INDEX_HTML: &str = r#"<!DOCTYPE html>
<html><head>
<meta charset="utf-8">
<title>bovista orbit</title>
<style>
  body { background:#111; color:#ccc; font:13px/1.4 ui-monospace,monospace; margin:20px; }
  h1 { margin:0 0 12px 0; font-size:14px; color:#888; font-weight:normal; }
  video { display:block; max-width:100%; background:#000; border:1px solid #333; }
  .row { display:flex; align-items:center; gap:10px; margin:6px 0; }
  .row label  { width:120px; color:#aaa; }
  .row input  { flex:1; }
  .row .val   { width:80px; text-align:right; color:#fff; }
</style>
</head><body>
<h1>bovista orbit_stream</h1>
<video src="/stream" autoplay muted controls></video>
<div id="controls">
  <div class="row"><label>zoom</label>
    <input type="range" id="zoom" min="0.25" max="4" step="0.05" value="{{ZOOM}}">
    <span class="val" id="zoom_val"></span></div>
  <div class="row"><label>contrast min</label>
    <input type="range" id="contrast_min" min="0" max="0.5" step="0.0005" value="{{CONTRAST_MIN}}">
    <span class="val" id="contrast_min_val"></span></div>
  <div class="row"><label>contrast max</label>
    <input type="range" id="contrast_max" min="0.001" max="1" step="0.0005" value="{{CONTRAST_MAX}}">
    <span class="val" id="contrast_max_val"></span></div>
  <div class="row"><label>density ×</label>
    <input type="range" id="density_mult" min="0.05" max="10" step="0.05" value="{{DENSITY_MULT}}">
    <span class="val" id="density_mult_val"></span></div>
  <div class="row"><label>orbit speed ×</label>
    <input type="range" id="orbit_speed" min="0" max="4" step="0.05" value="{{ORBIT_SPEED}}">
    <span class="val" id="orbit_speed_val"></span></div>
</div>
<script>
const fmt = { zoom: v=>(+v).toFixed(2)+'×',
              contrast_min: v=>(+v).toFixed(4),
              contrast_max: v=>(+v).toFixed(4),
              density_mult: v=>(+v).toFixed(2)+'×',
              orbit_speed:  v=>(+v).toFixed(2)+'×' };
for (const id of Object.keys(fmt)) {
  const el  = document.getElementById(id);
  const lbl = document.getElementById(id+'_val');
  const update = ()=>{ lbl.textContent = fmt[id](el.value);
                       fetch('/set?'+id+'='+el.value); };
  el.oninput = update;
  lbl.textContent = fmt[id](el.value);
}
</script>
</body></html>
"#;

// ─────────────────────────────────────────────────────────────────────────────
// CLI flag parsing (tiny inline; no clap dep)

fn flag_u32(args: &[String], name: &str, default: u32) -> u32 {
    flag_str_opt(args, name).and_then(|v| v.parse().ok()).unwrap_or(default)
}
fn flag_str(args: &[String], name: &str, default: &str) -> String {
    flag_str_opt(args, name).unwrap_or_else(|| default.to_string())
}
fn check_unknown_flags(args: &[String], known: &[&str]) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            let name = a.split('=').next().unwrap();
            if !known.contains(&name) {
                eprintln!("[orbit] error: unknown flag {:?}", a);
                eprintln!("[orbit] known flags: {}", known.join(", "));
                std::process::exit(2);
            }
            // Skip the value token if it's a separate arg.
            if !a.contains('=') && i + 1 < args.len() && !args[i + 1].starts_with("--") {
                i += 1;
            }
        }
        i += 1;
    }
}

fn flag_str_opt(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{}=", name);
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        if let Some(rest) = args[i].strip_prefix(&eq) {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}

// ─────────────────────────────────────────────────────────────────────────────
// OME-Zarr loader. Opens a multiscale group on a filesystem path or http URL,
// publishes LOD configs derived from the multiscales metadata, and serves tile
// reads on a small worker pool. Tiles are normalized to R16Float [0, 1].

mod ome_zarr {
    use std::collections::HashSet;
    use std::error::Error;
    use std::sync::{Arc, Mutex};
    use std::thread;

    use bovista::visuals::gpu_structs::{
        ChunkStatus, TileData, TileKey, TileLoaderFn, TileRequest,
    };
    use bovista::visuals::virtual_texture::PendingChunks;
    use bovista::visuals::LodLevelConfig;

    use zarrs::array::{Array, ArraySubset};
    use zarrs::group::Group;
    use zarrs::storage::ReadableStorageTraits;

    use super::SceneSetup;

    /// Max number of zarr-chunk reads we'll have in flight at once. Keeps remote
    /// HTTP stores from drowning in connections; the visual gets `Rejected` for
    /// further requests until older ones complete, which is its backpressure
    /// signal.
    const MAX_INFLIGHT: usize = 8;

    /// Each LOD level we'll serve. We type-erase the storage so the same struct
    /// can hold either a filesystem or http store.
    struct Level {
        array: Array<dyn ReadableStorageTraits>,
        /// Index of (z, y, x) within the array's possibly-higher-dim shape.
        z_idx: usize,
        y_idx: usize,
        x_idx: usize,
        /// Volume dims in (z, y, x) voxel order at this LOD.
        volume_zyx: (u64, u64, u64),
    }

    pub fn open(path_or_url: &str) -> Result<SceneSetup, Box<dyn Error>> {
        let store: Arc<dyn ReadableStorageTraits> = if path_or_url.starts_with("http://")
            || path_or_url.starts_with("https://")
        {
            Arc::new(zarrs_http::HTTPStore::new(path_or_url)?)
        } else {
            Arc::new(zarrs::filesystem::FilesystemStore::new(path_or_url)?)
        };

        let group = Group::open(store.clone(), "/")?;
        let attrs = group.attributes();

        // OME-Zarr v0.5 (zarr v3): attrs["ome"]["multiscales"][0]
        // OME-Zarr v0.4 (zarr v2): attrs["multiscales"][0]
        let multiscales = attrs
            .get("ome")
            .and_then(|o| o.get("multiscales"))
            .or_else(|| attrs.get("multiscales"))
            .and_then(|m| m.as_array())
            .and_then(|arr| arr.first())
            .ok_or("no multiscales metadata in zarr group")?;

        // Axis order, e.g. ["t", "c", "z", "y", "x"] → we want to pluck z/y/x.
        let axes: Vec<String> = multiscales
            .get("axes")
            .and_then(|a| a.as_array())
            .map(|arr| {
                arr.iter()
                    .map(|ax| {
                        ax.get("name")
                            .and_then(|n| n.as_str())
                            .unwrap_or("")
                            .to_string()
                    })
                    .collect()
            })
            .unwrap_or_default();

        let datasets_meta = multiscales
            .get("datasets")
            .and_then(|d| d.as_array())
            .ok_or("multiscales has no datasets[]")?;
        if datasets_meta.is_empty() {
            return Err("multiscales.datasets is empty".into());
        }

        // First, open every level's Array — needed to determine axis indices
        // and chunk shapes.
        let mut levels: Vec<Level> = Vec::with_capacity(datasets_meta.len());
        let mut lod0_voxel_zyx: (f64, f64, f64) = (1.0, 1.0, 1.0);
        let mut lods: Vec<LodLevelConfig> = Vec::with_capacity(datasets_meta.len());

        // Determine z/y/x indices using the LOD 0 array.
        let lod0_path = dataset_path(&datasets_meta[0], 0)?;
        let lod0_arr = Array::open(store.clone(), &absolute_path(&lod0_path))?;
        let ndim = lod0_arr.shape().len();

        let (z_idx, y_idx, x_idx) = match (
            axes.iter().position(|a| a == "z"),
            axes.iter().position(|a| a == "y"),
            axes.iter().position(|a| a == "x"),
        ) {
            (Some(z), Some(y), Some(x)) => (z, y, x),
            _ => {
                if ndim < 3 {
                    return Err(format!("array has {} dimensions, need ≥ 3", ndim).into());
                }
                (ndim - 3, ndim - 2, ndim - 1)
            }
        };

        // The tile size we publish to bovista must be uniform across LODs.
        // Use LOD 0's regular chunk size as the canonical tile size.
        // We want tiles small enough that the GPU atlas stays bounded. Each
        // tile is tz*ty*tx*2 bytes (R16Float); at MAX_TILE_AXIS=128 that's
        // 4 MB/tile, so a 4096-slot atlas tops out at ~16 GB. If the on-disk
        // chunk is smaller along an axis, use the chunk size so we don't
        // require multiple chunk reads per tile; otherwise clamp.
        const MAX_TILE_AXIS: u64 = 128;
        let lod0_chunk = lod0_arr.chunk_shape(&vec![0; ndim])?;
        let clamp = |chunk_dim: u64| -> u32 {
            u32::try_from(chunk_dim.min(MAX_TILE_AXIS)).unwrap_or(64)
        };
        let tile_z = clamp(lod0_chunk[z_idx].get());
        let tile_y = clamp(lod0_chunk[y_idx].get());
        let tile_x = clamp(lod0_chunk[x_idx].get());

        for (i, ds) in datasets_meta.iter().enumerate() {
            let path = dataset_path(ds, i)?;
            let arr = Array::open(store.clone(), &absolute_path(&path))?;

            let shape = arr.shape().to_vec();
            if shape.len() != ndim {
                return Err(format!(
                    "LOD {} has different dimensionality ({} vs {})",
                    i,
                    shape.len(),
                    ndim
                )
                .into());
            }
            let vz = shape[z_idx];
            let vy = shape[y_idx];
            let vx = shape[x_idx];

            // Scale transform: matches axes in length, gives voxel spacing per axis.
            let scale = ds
                .get("coordinateTransformations")
                .and_then(|t| t.as_array())
                .and_then(|arr| arr.iter().find(|t| t.get("type").and_then(|v| v.as_str()) == Some("scale")))
                .and_then(|t| t.get("scale"))
                .and_then(|s| s.as_array())
                .map(|arr| arr.iter().map(|v| v.as_f64().unwrap_or(1.0)).collect::<Vec<_>>())
                .unwrap_or_else(|| vec![1.0; ndim]);

            let vs_z = *scale.get(z_idx).unwrap_or(&1.0);
            let vs_y = *scale.get(y_idx).unwrap_or(&1.0);
            let vs_x = *scale.get(x_idx).unwrap_or(&1.0);

            let scale_factor = if i == 0 {
                lod0_voxel_zyx = (vs_z, vs_y, vs_x);
                1.0
            } else {
                let rz = vs_z / lod0_voxel_zyx.0;
                let ry = vs_y / lod0_voxel_zyx.1;
                let rx = vs_x / lod0_voxel_zyx.2;
                rz.max(ry).max(rx) as f32
            };

            // Center the world frame on the LOD-0 volume center.
            let (translation, voxel_size) = if i == 0 {
                let tz = -(vz as f64 * vs_z) / 2.0;
                let ty = -(vy as f64 * vs_y) / 2.0;
                let tx = -(vx as f64 * vs_x) / 2.0;
                (
                    (tz as f32, ty as f32, tx as f32),
                    (vs_z as f32, vs_y as f32, vs_x as f32),
                )
            } else {
                // Lower-resolution LODs share the world frame: their voxels
                // are larger but cover the same physical extent.
                let lod0_translation = lods[0].translation;
                (
                    lod0_translation,
                    (vs_z as f32, vs_y as f32, vs_x as f32),
                )
            };

            lods.push(LodLevelConfig {
                volume_size: (vz as u32, vy as u32, vx as u32),
                tile_size:   (tile_z, tile_y, tile_x),
                voxel_size,
                scale_factor,
                translation,
            });

            // Erase the storage type so we can hold a heterogenous list. The
            // underlying Arc<dyn ReadableStorageTraits> is what we want for
            // dynamic dispatch anyway.
            levels.push(Level {
                array: arr,
                z_idx,
                y_idx,
                x_idx,
                volume_zyx: (vz, vy, vx),
            });
        }

        println!("[orbit] OME-Zarr: {} LODs, axes={:?}, tile={}×{}×{} (zyx)",
                 lods.len(), axes, tile_z, tile_y, tile_x);
        for (i, l) in lods.iter().enumerate() {
            println!("  LOD {}: vol {}×{}×{} voxels, voxel size {:?}, scale {:.2}×",
                     i, l.volume_size.0, l.volume_size.1, l.volume_size.2,
                     l.voxel_size, l.scale_factor);
        }

        // World extents from LOD 0.
        let world_extents = {
            let l = &lods[0];
            (
                l.volume_size.0 as f32 * l.voxel_size.0,
                l.volume_size.1 as f32 * l.voxel_size.1,
                l.volume_size.2 as f32 * l.voxel_size.2,
            )
        };

        // Tile loader: spawns a worker per request, bounded by MAX_INFLIGHT.
        let levels = Arc::new(levels);
        let inflight: Arc<Mutex<HashSet<TileKey>>> = Arc::new(Mutex::new(HashSet::new()));
        let pending_slot: Arc<Mutex<Option<PendingChunks>>> = Arc::new(Mutex::new(None));
        let lods_for_loader = lods.clone();

        let pending_for_loader = pending_slot.clone();
        let levels_for_loader = levels.clone();
        let inflight_for_loader = inflight.clone();
        let loader: TileLoaderFn = Box::new(move |req: TileRequest| -> ChunkStatus {
            let lod = req.lod_level.unwrap_or(0);
            if lod >= levels_for_loader.len() {
                return ChunkStatus::Rejected;
            }
            let key = TileKey { lod_level: lod, z: req.z, y: req.y, x: req.x };

            // Need the pending queue (set after volume creation).
            let pending = match pending_for_loader.lock().unwrap().clone() {
                Some(p) => p,
                None => return ChunkStatus::Rejected,
            };

            // Dedupe + bound concurrency.
            {
                let mut inflight = inflight_for_loader.lock().unwrap();
                if inflight.contains(&key) {
                    return ChunkStatus::AlreadyPending;
                }
                if inflight.len() >= MAX_INFLIGHT {
                    return ChunkStatus::Rejected;
                }
                inflight.insert(key);
            }

            let levels = levels_for_loader.clone();
            let lod_cfg = lods_for_loader[lod].clone();
            let inflight = inflight_for_loader.clone();
            thread::spawn(move || {
                let result = read_tile(&levels[lod], &lod_cfg, key);
                match result {
                    Ok(tile_data) => {
                        pending.lock().unwrap().insert(key, tile_data);
                    }
                    Err(e) => {
                        eprintln!(
                            "[orbit] tile load failed (lod={} z={} y={} x={}): {}",
                            key.lod_level, key.z, key.y, key.x, e
                        );
                    }
                }
                inflight.lock().unwrap().remove(&key);
            });

            ChunkStatus::Accepted
        });

        Ok(SceneSetup {
            lods,
            loader,
            pending_slot,
            world_extents,
            // Comfortably bigger than a single LOD's tile count for the
            // OME-Zarr datasets we care about. Override with --cache-tiles.
            cache_capacity: 4096,
        })
    }

    fn dataset_path(ds: &serde_json::Value, idx: usize) -> Result<String, Box<dyn Error>> {
        ds.get("path")
            .and_then(|p| p.as_str())
            .map(String::from)
            .ok_or_else(|| format!("datasets[{}] missing \"path\"", idx).into())
    }

    fn absolute_path(rel: &str) -> String {
        if rel.starts_with('/') { rel.to_string() } else { format!("/{}", rel) }
    }

    /// Read the voxel sub-region for this tile and convert to R16Float bytes.
    /// Edge tiles return their actual (partial) extent — bovista's atlas upload
    /// uses TileData.{width,height,depth} as the extent, and the volume shader
    /// is bounded by volume_size with nearest filtering, so no padding needed.
    fn read_tile(
        level: &Level,
        lod_cfg: &LodLevelConfig,
        key: TileKey,
    ) -> Result<TileData, Box<dyn Error + Send + Sync>> {
        let (tz, ty, tx) = lod_cfg.tile_size;
        let (vz, vy, vx) = level.volume_zyx;

        // Voxel ranges, clipped to the actual volume extent.
        let z0 = (key.z as u64) * tz as u64;
        let y0 = (key.y as u64) * ty as u64;
        let x0 = (key.x as u64) * tx as u64;
        let z1 = (z0 + tz as u64).min(vz);
        let y1 = (y0 + ty as u64).min(vy);
        let x1 = (x0 + tx as u64).min(vx);
        if z0 >= vz || y0 >= vy || x0 >= vx {
            return Err(format!("tile out of bounds: z={} y={} x={}", key.z, key.y, key.x).into());
        }

        let extent_z = (z1 - z0) as u32;
        let extent_y = (y1 - y0) as u32;
        let extent_x = (x1 - x0) as u32;

        // Build an N-D subset where non-spatial axes are pinned to 0..1.
        let ndim = level.array.shape().len();
        let mut ranges: Vec<std::ops::Range<u64>> = (0..ndim).map(|_| 0..1).collect();
        ranges[level.z_idx] = z0..z1;
        ranges[level.y_idx] = y0..y1;
        ranges[level.x_idx] = x0..x1;
        let subset = ArraySubset::new_with_ranges(&ranges);

        // Match by Zarr v3 name string; covers both v2 and v3 sources because
        // zarrs maps v2 dtype strings into the same internal type registry.
        let dtype = level.array.data_type();
        let name = dtype
            .name(zarrs::plugin::ZarrVersion::V3)
            .map(|n| n.to_string())
            .unwrap_or_default();

        // One-time per (lod,key) diagnostic so we can spot dtype mismatches.
        static DBG_ONCE: std::sync::Once = std::sync::Once::new();
        DBG_ONCE.call_once(|| {
            println!("[orbit] zarr dtype detected as: \"{}\"", name);
        });

        let f16_bytes = match name.as_str() {
            "uint8" => {
                let v: Vec<u8> = level.array.retrieve_array_subset(&subset)?;
                normalize_to_f16(&v, |x| x as f32 / 255.0)
            }
            "uint16" => {
                let v: Vec<u16> = level.array.retrieve_array_subset(&subset)?;
                normalize_to_f16(&v, |x| x as f32 / 65535.0)
            }
            "int8" => {
                let v: Vec<i8> = level.array.retrieve_array_subset(&subset)?;
                normalize_to_f16(&v, |x| (x as f32 + 128.0) / 255.0)
            }
            "int16" => {
                let v: Vec<i16> = level.array.retrieve_array_subset(&subset)?;
                normalize_to_f16(&v, |x| (x as f32 + 32768.0) / 65535.0)
            }
            "float32" => {
                let v: Vec<f32> = level.array.retrieve_array_subset(&subset)?;
                normalize_to_f16(&v, |x| x.clamp(0.0, 1.0))
            }
            "float16" => {
                let v: Vec<half::f16> = level.array.retrieve_array_subset(&subset)?;
                let mut out = Vec::with_capacity(v.len() * 2);
                for x in v {
                    let bits = x.to_bits();
                    out.push((bits & 0xff) as u8);
                    out.push((bits >> 8) as u8);
                }
                out
            }
            other => return Err(format!("unsupported zarr dtype: {}", other).into()),
        };

        Ok(TileData {
            data: f16_bytes,
            width: extent_x, height: extent_y, depth: extent_z,
            format: wgpu::TextureFormat::R16Float,
        })
    }

    fn normalize_to_f16<T: Copy>(src: &[T], to_unit: impl Fn(T) -> f32) -> Vec<u8> {
        let mut out = Vec::with_capacity(src.len() * 2);
        let mut min_v: f32 = f32::INFINITY;
        let mut max_v: f32 = f32::NEG_INFINITY;
        for &x in src {
            let v = to_unit(x);
            if v < min_v { min_v = v; }
            if v > max_v { max_v = v; }
            let bits = half::f16::from_f32(v).to_bits();
            out.push((bits & 0xff) as u8);
            out.push((bits >> 8) as u8);
        }
        // One-time min/max print so we can see the actual value range.
        static DBG_RANGE: std::sync::Once = std::sync::Once::new();
        DBG_RANGE.call_once(|| {
            println!("[orbit] first tile normalized range: [{:.3}, {:.3}]", min_v, max_v);
        });
        out
    }
}
