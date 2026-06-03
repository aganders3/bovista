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
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
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
        "--zarr", "--cache-tiles", "--max-inflight", "--prefetch-cap",
        "--contrast-min", "--contrast-max", "--density-mult",
        "--timepoint", "--bench",
    ];
    check_unknown_flags(&args, KNOWN_FLAGS);

    // ── Thread-budgeting ───────────────────────────────────────────────────
    // zarrs's chunk reads dispatch through the global rayon pool. By default
    // rayon takes available_parallelism() threads, which on a 64-core HPC
    // node is huge and means our spawned in-flight workers all expand into
    // a shared pool of 64+ threads and start thrashing.
    //
    // Cap the rayon pool to a fixed number (default 8, override via env)
    // and budget MAX_INFLIGHT against what's left: leave 2 cores for the
    // render thread + writer thread + ffmpeg, give rayon its share, use
    // the remainder for concurrent NFS reads.
    let rayon_threads: usize = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global();
    let cpus = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(8);
    let auto_inflight = cpus.saturating_sub(2).saturating_sub(rayon_threads).max(8);

    let width: u32 = flag_u32(&args, "--width", 1024);
    let height: u32 = flag_u32(&args, "--height", 768);
    let fps: u32 = flag_u32(&args, "--fps", 30);
    let port: u16 = flag_u32(&args, "--port", 8080) as u16;
    let backend = flag_str(&args, "--backend", "auto");
    let zarr_arg: Option<String> = flag_str_opt(&args, "--zarr");
    let cache_override: Option<u32> = flag_str_opt(&args, "--cache-tiles").and_then(|v| v.parse().ok());
    let max_inflight: usize = flag_str_opt(&args, "--max-inflight")
        .and_then(|v| v.parse().ok())
        .unwrap_or(auto_inflight);
    // Prefetch cap defaults to 4096 (~16 GB at LOD 1 / 128³ R16F = 4 MB/tile);
    // override with --prefetch-cap N. Visible tile counts up to ~1500 at LOD 0
    // benefit from a high cap × (2*lookahead + 1).
    let prefetch_cap: usize = flag_str_opt(&args, "--prefetch-cap")
        .and_then(|v| v.parse().ok())
        .unwrap_or(4096);
    println!(
        "[orbit] thread budget: rayon={} max_inflight={} prefetch_cap={} (cpus={})",
        rayon_threads, max_inflight, prefetch_cap, cpus,
    );

    // Standalone benchmark mode: no rendering, no streaming. Just open the
    // zarr and time per-tile reads at each LOD across a few timepoints.
    if args.iter().any(|a| a == "--bench") {
        let path = zarr_arg.expect("--bench requires --zarr <path>");
        if let Err(e) = bench_zarr(&path) {
            eprintln!("[bench] error: {}", e);
            std::process::exit(1);
        }
        return;
    }
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

    // Shared view state — the render loop reads each frame, HTTP `/set`
    // writes from slider events on the served HTML page. Constructed before
    // the OME-Zarr open so the loader can capture it (the zarr loader reads
    // view_state.timepoint() per tile request).
    let view_state = Arc::new(ViewState::new(
        1.0,           // zoom (× world_max for camera radius)
        contrast_min,
        contrast_max,
        density_mult,
        1.0,           // orbit_speed (× 1 revolution per 12s)
    ));
    let flush_diag = Arc::new(FlushDiag::new());

    // ── Scene setup: synthetic cube or OME-Zarr pyramid ─────────────────────
    let setup = match &zarr_arg {
        Some(path) => match ome_zarr::open(path, view_state.clone(), flush_diag.clone(), max_inflight, prefetch_cap) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("[orbit] failed to open OME-Zarr at {}: {}", path, e);
                std::process::exit(1);
            }
        },
        None => synthetic_setup(),
    };
    let cache_capacity = cache_override.unwrap_or(setup.cache_capacity);
    let n_timepoints = setup.n_timepoints;
    // Bind a clone of the prefetcher's visible-snapshot here so the render
    // loop can refresh it without re-locking setup.
    let setup_visible_snapshot = setup.visible_snapshot.clone();

    let mut volume = VolumeVisual::new(
        renderer.device(), renderer.queue(), renderer.surface_format(),
        renderer.camera_bind_group_layout(),
        setup.lods.clone(), cache_capacity as usize, setup.loader,
    );
    *setup.pending_slot.lock().unwrap() = Some(volume.pending_chunks().unwrap());
    // Background-prefetch ±2 neighbors of the current timepoint via
    // bovista's own request path. Single dedupe (slot_map + request_tile)
    // means already-resident tiles cost a HashMap lookup, not a re-decode.
    if n_timepoints > 1 {
        volume.set_prefetch(2, n_timepoints);
    }

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
    // Latest-frame-only handoff (no backlog). `client_connected` is the
    // render thread's "should I bother rendering" gate, set/cleared by the
    // HTTP connection handler.
    let frame_slot = Arc::new(LatestFrame::new());
    let client_connected = Arc::new(AtomicBool::new(false));
    spawn_http_server(
        port, width, height, fps,
        frame_slot.clone(), client_connected.clone(),
        view_state.clone(), n_timepoints,
    );

    println!("[orbit] ready; open http://localhost:{}/ (Ctrl-C to stop)", port);

    // ── Render loop ─────────────────────────────────────────────────────────
    let frame_dt = Duration::from_secs_f32(1.0 / fps as f32);
    let idle_sleep = Duration::from_millis(100);
    // Orbit angle accumulator (so changing orbit speed mid-flight doesn't
    // teleport the camera — only changes its rate going forward).
    let mut theta: f32 = 0.0;
    let mut last_tick = Instant::now();
    // Anchor for "wall-clock frame schedule". Each frame's target wall time
    // is `loop_start + frame_dt * frame_n`. If we fall further behind than
    // MAX_FRAME_LAG, we skip frames so the stream stays live instead of
    // building backlog inside ffmpeg / kernel pipe / TCP buffer / browser.
    let mut loop_start = Instant::now();
    let mut frame_n: u64 = 0;
    let max_frame_lag = frame_dt * 3; // tolerate up to 3 frames of slip
    // Last seen t_generation so we can detect timepoint changes and flush
    // the atlas exactly once per change (rather than every frame).
    let mut last_t_gen: u32 = view_state.t_generation();
    // True while we're in the post-flush diagnostic window.
    let mut flush_active = false;
    // ns since program start when the current flush began (0 = none).
    let mut flush_started_ns: u64 = 0;
    // Whether we've printed the one-shot [loaded] snapshot for this flush.
    let mut printed_loaded_snapshot = false;
    let mut dropped_total: u64 = 0;

    loop {
        // Skip the render entirely when nobody's watching.
        if !client_connected.load(Ordering::Relaxed) {
            std::thread::sleep(idle_sleep);
            last_tick = Instant::now();
            loop_start = Instant::now();
            frame_n = 0;
            continue;
        }

        let tick = Instant::now();
        let dt = tick.duration_since(last_tick).as_secs_f32();
        last_tick = tick;

        // If we've drifted more than max_frame_lag behind the wall-clock
        // schedule, advance frame_n past the gap so we don't try to "catch
        // up" by feeding ffmpeg a burst of frames (which would queue inside
        // the kernel pipe / TCP send buffer / browser and produce the
        // accumulated-delay effect).
        let target = loop_start + frame_dt * frame_n as u32;
        if tick > target + max_frame_lag {
            let behind = tick - target;
            let skip = (behind.as_nanos() / frame_dt.as_nanos()) as u64;
            frame_n = frame_n.saturating_add(skip);
            dropped_total += skip;
            if dropped_total.is_power_of_two() || skip > 10 {
                eprintln!("[orbit] frame pacer: skipping {} frames (total dropped: {})",
                          skip, dropped_total);
            }
        }

        // Apply current view state to the volume (cheap; writes only on next
        // prepare() submission). If the timepoint changed, flush the atlas
        // so visible tiles re-request with the new t, and arm the flush
        // diagnostics.
        {
            let mut v = volume_arc.lock().unwrap();
            v.set_contrast_limits(view_state.contrast_min(), view_state.contrast_max());
            v.set_density_scale(base_density * view_state.density_mult());
            let gen = view_state.t_generation();
            if gen != last_t_gen {
                if flush_active {
                    print_flush_summary(&flush_diag, "interrupted");
                }
                // Wait-to-swap: bovista keeps showing the previous timepoint
                // until every visible spatial tile is present at the new t.
                // No clear, no flicker — adjacent timepoints stay resident
                // in the atlas as a free cache.
                v.set_desired_timepoint(view_state.timepoint());
                last_t_gen = gen;
                let t_ns = view_state.ns_since_start();
                flush_diag.start(t_ns);
                flush_active = true;
                flush_started_ns = t_ns;
                printed_loaded_snapshot = false;
                println!("[orbit] timepoint → {}", view_state.timepoint());
            }
        }

        // Camera orbit (one revolution per 12s × orbit_speed).
        theta += dt * (std::f32::consts::TAU / 12.0) * view_state.orbit_speed();
        let radius = world_max * 2.0 * view_state.zoom();
        camera.position = glam::Vec3::new(
            radius * theta.cos(),
            radius * 0.30,
            radius * theta.sin(),
        );

        let (bgra, prepare_dt) = render_one_frame(
            &renderer, &mut scene, &camera,
            &color_tex, &color_view, &depth_view,
            width, height,
        );
        // Refresh the visible-spatial snapshot the prefetcher reads.
        // scene.prepare just updated bovista's visible_tile_keys; snapshot
        // it now so the prefetcher always works on fresh geometry rather
        // than an accumulating shadow of stale camera positions.
        {
            let v = volume_arc.lock().unwrap();
            let snap = v.visible_spatial_keys();
            *setup_visible_snapshot.lock().unwrap() = snap;
        }
        // Attribute scene.prepare() time (which includes process_pending_chunks
        // → queue.write_texture for every newly-arrived tile) to the current
        // flush, if any.
        if flush_active {
            flush_diag.upload_ns_total
                .fetch_add(prepare_dt.as_nanos() as u64, Ordering::Relaxed);
            let now_ns = view_state.ns_since_start();
            let elapsed_ms = (now_ns.saturating_sub(flush_started_ns)) / 1_000_000;

            // One-shot [loaded] snapshot 150 ms after flush start: this is
            // long enough for the initial visible-set load to complete (cache
            // hits + GPU upload) but short enough that camera-orbit-driven
            // discoveries haven't polluted the count yet. Captures the
            // "how snappy did t-change feel?" answer.
            let tiles = flush_diag.tiles_loaded.load(Ordering::Relaxed);
            if !printed_loaded_snapshot && elapsed_ms >= 150 {
                let hits   = flush_diag.cache_hits.load(Ordering::Relaxed);
                let misses = flush_diag.cache_misses.load(Ordering::Relaxed);
                let dec_ns = flush_diag.decode_ns_total.load(Ordering::Relaxed);
                let up_ns  = flush_diag.upload_ns_total.load(Ordering::Relaxed);
                println!(
                    "[loaded] @{} ms: {} tiles ({} hit / {} miss) | decode {:.0} ms | upload {:.0} ms",
                    elapsed_ms, tiles, hits, misses,
                    dec_ns as f64 / 1e6,
                    up_ns as f64 / 1e6,
                );
                printed_loaded_snapshot = true;
            }

            // Settle detection: still prints a [flush:settled] when arrivals
            // stop for 1 s. Note that an active camera orbit keeps producing
            // arrivals so this can take a while to fire — that's the orbit
            // bringing new tiles into view, not flush latency.
            let last = flush_diag.last_arrival_ns.load(Ordering::Relaxed);
            let t0 = flush_diag.flush_t0_ns.load(Ordering::Relaxed);
            let since_last_ms = ((now_ns.saturating_sub(t0)).saturating_sub(last)) / 1_000_000;
            if tiles > 0 && since_last_ms > 1000 {
                print_flush_summary(&flush_diag, "settled");
                flush_active = false;
                flush_diag.flush_t0_ns.store(0, Ordering::Relaxed);
            }
        }

        // Hand the frame off to the writer thread via the latest-only slot.
        // This NEVER blocks on network/encoder backpressure — older frames
        // are silently overwritten if the writer hasn't drained them yet.
        // That decouples render thread responsiveness from downstream
        // congestion (background tabs, slow network, ffmpeg buffering).
        frame_slot.push(bgra);

        // Pace against absolute wall-clock target, not relative to start of
        // this frame — so a fast frame doesn't gain budget for the next one
        // and we converge back to 30 fps exactly even after a slow flush.
        frame_n += 1;
        let next_target = loop_start + frame_dt * frame_n as u32;
        let now = Instant::now();
        if now < next_target {
            std::thread::sleep(next_target - now);
        }
    }
}

fn print_flush_summary(diag: &FlushDiag, reason: &str) {
    let tiles  = diag.tiles_loaded.load(Ordering::Relaxed);
    if tiles == 0 { return; }
    let first  = diag.first_arrival_ns.load(Ordering::Relaxed);
    let last   = diag.last_arrival_ns.load(Ordering::Relaxed);
    let dec_ns = diag.decode_ns_total.load(Ordering::Relaxed);
    let up_ns  = diag.upload_ns_total.load(Ordering::Relaxed);
    let hits   = diag.cache_hits.load(Ordering::Relaxed);
    let misses = diag.cache_misses.load(Ordering::Relaxed);
    println!(
        "[flush:{}] {} tiles ({} hit / {} miss = {:.0}%) | first {:.0} ms last {:.0} ms | \
         decode {:.0} ms ({:.1} ms/tile) | upload {:.0} ms",
        reason,
        tiles, hits, misses,
        100.0 * hits as f64 / tiles as f64,
        first as f64 / 1e6,
        last as f64 / 1e6,
        dec_ns as f64 / 1e6,
        dec_ns as f64 / 1e6 / tiles as f64,
        up_ns as f64 / 1e6,
    );
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
) -> (Vec<u8>, Duration) {
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
    let prepare_start = Instant::now();
    scene.prepare(device, queue, &camera_info);
    let prepare_dt = prepare_start.elapsed();

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
    (out, prepare_dt)
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
    /// Number of timepoints if axes include a "t" axis, else 1.
    n_timepoints: u32,
    /// Shared with the prefetcher: render thread refreshes this each frame
    /// with bovista's current visible spatial set. Empty if no prefetcher.
    visible_snapshot: Arc<Mutex<Vec<TileKey>>>,
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
            t: req.t, z: req.z, y: req.y, x: req.x,
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
        n_timepoints: 1,
        visible_snapshot: Arc::new(Mutex::new(Vec::new())),
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
// OME-Zarr decode benchmark. Reads 128³ sub-regions at each LOD (sequential
// and 8-way parallel) and prints per-tile / per-frame timings so we can
// estimate the maximum sustainable playback fps for the dataset.

fn bench_zarr(path_or_url: &str) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    use std::sync::Arc as Arc2;
    use std::time::Instant;
    use zarrs::array::{Array, ArraySubset};
    use zarrs::group::Group;
    use zarrs::storage::ReadableStorageTraits;

    const TILE: u64 = 128;
    const N_TILES: u64 = 16;        // tiles per pass (sequential and parallel)
    const PARALLEL: usize = 8;       // worker count for parallel pass

    let store: Arc2<dyn ReadableStorageTraits> =
        if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            Arc2::new(zarrs_http::HTTPStore::new(path_or_url)?)
        } else {
            Arc2::new(zarrs::filesystem::FilesystemStore::new(path_or_url)?)
        };
    let group = Group::open(store.clone(), "/")?;
    let attrs = group.attributes();
    let multiscales = attrs
        .get("ome").and_then(|o| o.get("multiscales"))
        .or_else(|| attrs.get("multiscales"))
        .and_then(|m| m.as_array())
        .and_then(|a| a.first())
        .ok_or("no multiscales metadata")?;
    let axes: Vec<String> = multiscales.get("axes").and_then(|a| a.as_array())
        .map(|a| a.iter().map(|x| x.get("name").and_then(|n| n.as_str()).unwrap_or("").to_string()).collect())
        .unwrap_or_default();
    let datasets = multiscales.get("datasets").and_then(|d| d.as_array())
        .ok_or("multiscales.datasets missing")?;

    println!("[bench] axes = {:?}", axes);
    for (lod_idx, ds) in datasets.iter().enumerate() {
        let path = ds.get("path").and_then(|p| p.as_str()).unwrap_or("0");
        let abs = if path.starts_with('/') { path.to_string() } else { format!("/{}", path) };
        let array = Array::open(store.clone(), &abs)?;
        let arr = Arc2::new(array);
        let shape = arr.shape().to_vec();
        let ndim = shape.len();
        let (z_idx, y_idx, x_idx) = match (
            axes.iter().position(|a| a == "z"),
            axes.iter().position(|a| a == "y"),
            axes.iter().position(|a| a == "x"),
        ) {
            (Some(z), Some(y), Some(x)) => (z, y, x),
            _ => (ndim - 3, ndim - 2, ndim - 1),
        };
        let t_idx = axes.iter().position(|a| a == "t");

        let extents = (shape[z_idx], shape[y_idx], shape[x_idx]);
        let dtype = arr.data_type().name(zarrs::plugin::ZarrVersion::V3)
            .map(|n| n.to_string()).unwrap_or_default();
        let chunk_shape: Vec<u64> = arr.chunk_shape(&vec![0u64; ndim])?.iter().map(|d| d.get()).collect();
        println!("\n[bench] LOD {}: dtype={} shape(zyx)={:?} chunk_shape={:?}",
                 lod_idx, dtype, extents, chunk_shape);

        // Pick N_TILES non-overlapping 128³ regions tiled along z at the
        // volume's centre in (y, x). If the volume is smaller than 128 along
        // an axis, clamp.
        let tz = TILE.min(extents.0);
        let ty = TILE.min(extents.1);
        let tx = TILE.min(extents.2);
        let cy = extents.1.saturating_sub(ty) / 2;
        let cx = extents.2.saturating_sub(tx) / 2;
        let n = N_TILES.min(extents.0 / tz.max(1));
        if n == 0 {
            println!("  (volume too small for 128³ benchmark; skipping)");
            continue;
        }

        let make_subset = |tile_z: u64, t: u64| -> ArraySubset {
            let mut ranges: Vec<std::ops::Range<u64>> = (0..ndim).map(|_| 0..1).collect();
            ranges[z_idx] = (tile_z * tz)..((tile_z + 1) * tz);
            ranges[y_idx] = cy..(cy + ty);
            ranges[x_idx] = cx..(cx + tx);
            if let Some(ti) = t_idx { ranges[ti] = t..(t + 1); }
            ArraySubset::new_with_ranges(&ranges)
        };

        // Sequential pass: read N_TILES tiles back-to-back at t=0.
        let bytes_per_tile = (tz * ty * tx) as usize * dtype_bytes(&dtype);
        let t0 = Instant::now();
        for i in 0..n {
            let subset = make_subset(i, 0);
            read_subset_any(&arr, &dtype, &subset)?;
        }
        let seq = t0.elapsed();
        println!(
            "  sequential {} × 128³: {:.0} ms total, {:.1} ms/tile, {:.1} MB/s decoded",
            n,
            seq.as_secs_f64() * 1e3,
            seq.as_secs_f64() * 1e3 / n as f64,
            (bytes_per_tile * n as usize) as f64 / seq.as_secs_f64() / 1e6,
        );

        // Parallel pass: same N_TILES across PARALLEL worker threads.
        let t0 = Instant::now();
        std::thread::scope(|scope| {
            for w in 0..PARALLEL {
                let arr = arr.clone();
                let dtype = dtype.clone();
                scope.spawn(move || {
                    let mut i = w as u64;
                    while i < n {
                        let subset = make_subset(i, 0);
                        let _ = read_subset_any(&arr, &dtype, &subset);
                        i += PARALLEL as u64;
                    }
                });
            }
        });
        let par = t0.elapsed();
        println!(
            "  parallel({}×) {} × 128³: {:.0} ms total, {:.1} ms/tile, {:.1} MB/s decoded → speedup {:.2}×",
            PARALLEL, n,
            par.as_secs_f64() * 1e3,
            par.as_secs_f64() * 1e3 / n as f64,
            (bytes_per_tile * n as usize) as f64 / par.as_secs_f64() / 1e6,
            seq.as_secs_f64() / par.as_secs_f64(),
        );

        // Playback simulation: same spatial region at the first N timepoints.
        if let Some(_) = t_idx {
            let n_t = N_TILES.min(shape[t_idx.unwrap()]);
            if n_t > 1 {
                let t0 = Instant::now();
                std::thread::scope(|scope| {
                    for w in 0..PARALLEL {
                        let arr = arr.clone();
                        let dtype = dtype.clone();
                        scope.spawn(move || {
                            let mut t = w as u64;
                            while t < n_t {
                                let subset = make_subset(0, t);
                                let _ = read_subset_any(&arr, &dtype, &subset);
                                t += PARALLEL as u64;
                            }
                        });
                    }
                });
                let dt = t0.elapsed();
                println!(
                    "  playback sim (1 tile × {} timepoints, {}× parallel): {:.0} ms, {:.2} fps",
                    n_t, PARALLEL, dt.as_secs_f64() * 1e3, n_t as f64 / dt.as_secs_f64(),
                );
            }
        }
    }
    Ok(())
}

fn dtype_bytes(name: &str) -> usize {
    match name {
        "uint8" | "int8" => 1,
        "uint16" | "int16" | "float16" => 2,
        "float32" | "int32" | "uint32" => 4,
        _ => 1,
    }
}

fn read_subset_any(
    arr: &zarrs::array::Array<dyn zarrs::storage::ReadableStorageTraits>,
    dtype: &str,
    subset: &zarrs::array::ArraySubset,
) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    match dtype {
        "uint8"   => { let _: Vec<u8>  = arr.retrieve_array_subset(subset)?; }
        "uint16"  => { let _: Vec<u16> = arr.retrieve_array_subset(subset)?; }
        "int8"    => { let _: Vec<i8>  = arr.retrieve_array_subset(subset)?; }
        "int16"   => { let _: Vec<i16> = arr.retrieve_array_subset(subset)?; }
        "float32" => { let _: Vec<f32> = arr.retrieve_array_subset(subset)?; }
        "float16" => { let _: Vec<half::f16> = arr.retrieve_array_subset(subset)?; }
        _ => return Err(format!("unsupported dtype {}", dtype).into()),
    }
    Ok(())
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
            "-preset".into(),       "p1".into(),
            // Ultra-low-latency tune disables lookahead + B-frames at the
            // tune-preset level (Turing+ / Ada Lovelace required).
            "-tune".into(),         "ull".into(),
            "-rc".into(),           "cbr".into(),
            "-b:v".into(),          "8M".into(),
            // Belt-and-braces: explicitly forbid B-frames + lookahead so the
            // encoder can't introduce frame-reordering latency.
            "-bf".into(),           "0".into(),
            "-rc-lookahead".into(), "0".into(),
            "-delay".into(),        "0".into(),
            "-zerolatency".into(),  "1".into(),
            "-g".into(),            gop.clone(),
            "-keyint_min".into(),   gop.clone(),
            "-no-scenecut".into(),  "1".into(),
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
        // Stamp each input frame with the system wall-clock at arrival time
        // instead of N×(1/fps). With a constant -framerate, a slow render
        // would still claim its 33 ms slot and accumulate behind real time;
        // with wallclock timestamps, the browser sees the actual arrival
        // gaps so the stream stays "live" — slow frames become visible
        // hitches, never buildup lag.
        "-use_wallclock_as_timestamps", "1",
        "-f", "rawvideo", "-pix_fmt", "bgra",
        "-s", &size, "-r", &rate, "-i", "-",
        "-an",
        // Variable-fps mode pairs with wallclock input: don't synthesize
        // duplicates / drop frames to hit a target fps, just pass through.
        "-fps_mode", "passthrough",
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
    /// Current OME-Zarr timepoint (or 0 for non-temporal data). The zarr
    /// loader reads this for every tile request; the render loop watches
    /// `t_generation` and flushes the atlas when it bumps.
    timepoint:    AtomicU32,
    /// Increments whenever `timepoint` changes so the render loop can call
    /// VolumeVisual::clear_atlas() once per change (rather than every frame).
    t_generation: AtomicU32,
    /// Wall-clock origin for relative timestamps used by FlushDiag.
    program_start: Instant,
}

/// Single-slot frame handoff between render thread and the ffmpeg writer
/// thread. Render `push`-es overwrites whatever's currently there (latest
/// frame wins); writer `pop`s and blocks if empty. This decouples render
/// from network/encoder backpressure entirely — when downstream stalls
/// (e.g. browser tab in background), the writer blocks on stdin and the
/// render thread keeps producing frames into the slot, with old frames
/// silently overwritten. No backlog accumulates anywhere.
struct LatestFrame {
    inner: Mutex<Option<Vec<u8>>>,
    cvar: Condvar,
    closed: AtomicBool,
}

impl LatestFrame {
    fn new() -> Self {
        Self {
            inner: Mutex::new(None),
            cvar: Condvar::new(),
            closed: AtomicBool::new(false),
        }
    }
    /// Replace whatever's in the slot with this frame. Always succeeds while
    /// open; returns false if the slot has been closed (writer is gone).
    fn push(&self, frame: Vec<u8>) -> bool {
        if self.closed.load(Ordering::Relaxed) { return false; }
        *self.inner.lock().unwrap() = Some(frame);
        self.cvar.notify_one();
        true
    }
    /// Block until a frame is available or the slot is closed.
    fn pop(&self) -> Option<Vec<u8>> {
        let mut guard = self.inner.lock().unwrap();
        loop {
            if let Some(f) = guard.take() { return Some(f); }
            if self.closed.load(Ordering::Relaxed) { return None; }
            guard = self.cvar.wait(guard).unwrap();
        }
    }
    /// Wake any pop()-er with None and refuse future push()es.
    fn close(&self) {
        self.closed.store(true, Ordering::Relaxed);
        self.cvar.notify_all();
    }
    /// Reset (called when a new client connects after a previous disconnect).
    fn reopen(&self) {
        self.closed.store(false, Ordering::Relaxed);
        *self.inner.lock().unwrap() = None;
    }
}

/// End-to-end timing for one timepoint flush. Tile workers fill in
/// per-tile counters; the render loop reads them and prints a summary
/// once arrivals settle.
struct FlushDiag {
    /// ns-since-program-start when the most recent flush began. 0 = no
    /// flush in progress.
    flush_t0_ns:     AtomicU64,
    /// Number of tiles successfully loaded since `flush_t0_ns`.
    tiles_loaded:    AtomicU32,
    /// Sum of per-tile decode durations since flush, in ns.
    decode_ns_total: AtomicU64,
    /// Time of the most recent successful tile arrival, ns since flush_t0.
    last_arrival_ns: AtomicU64,
    /// ns-since-program-start when the FIRST tile of the current flush
    /// arrived. 0 = no tile has arrived yet.
    first_arrival_ns: AtomicU64,
    /// ns of GPU upload time accrued in scene.prepare() since flush.
    upload_ns_total: AtomicU64,
    /// Tile arrivals that came from the prefetch cache (decode_ns = 0).
    cache_hits:   AtomicU32,
    /// Tile arrivals from a real worker decode.
    cache_misses: AtomicU32,
}

impl FlushDiag {
    fn new() -> Self {
        Self {
            flush_t0_ns:      AtomicU64::new(0),
            tiles_loaded:     AtomicU32::new(0),
            decode_ns_total:  AtomicU64::new(0),
            last_arrival_ns:  AtomicU64::new(0),
            first_arrival_ns: AtomicU64::new(0),
            upload_ns_total:  AtomicU64::new(0),
            cache_hits:       AtomicU32::new(0),
            cache_misses:     AtomicU32::new(0),
        }
    }

    fn start(&self, t_ns: u64) {
        self.tiles_loaded.store(0, Ordering::Relaxed);
        self.decode_ns_total.store(0, Ordering::Relaxed);
        self.last_arrival_ns.store(0, Ordering::Relaxed);
        self.first_arrival_ns.store(0, Ordering::Relaxed);
        self.upload_ns_total.store(0, Ordering::Relaxed);
        self.cache_hits.store(0, Ordering::Relaxed);
        self.cache_misses.store(0, Ordering::Relaxed);
        self.flush_t0_ns.store(t_ns, Ordering::Relaxed);
    }

    fn note_tile(&self, now_ns: u64, decode_ns: u64) {
        let t0 = self.flush_t0_ns.load(Ordering::Relaxed);
        if t0 == 0 { return; }
        self.tiles_loaded.fetch_add(1, Ordering::Relaxed);
        self.decode_ns_total.fetch_add(decode_ns, Ordering::Relaxed);
        if decode_ns == 0 {
            self.cache_hits.fetch_add(1, Ordering::Relaxed);
        } else {
            self.cache_misses.fetch_add(1, Ordering::Relaxed);
        }
        let since = now_ns.saturating_sub(t0);
        self.last_arrival_ns.store(since, Ordering::Relaxed);
        let _ = self.first_arrival_ns.compare_exchange(
            0, since, Ordering::Relaxed, Ordering::Relaxed,
        );
    }
}

impl ViewState {
    fn new(zoom: f32, cmin: f32, cmax: f32, dens: f32, speed: f32) -> Self {
        Self {
            zoom:         AtomicU32::new(zoom.to_bits()),
            contrast_min: AtomicU32::new(cmin.to_bits()),
            contrast_max: AtomicU32::new(cmax.to_bits()),
            density_mult: AtomicU32::new(dens.to_bits()),
            orbit_speed:  AtomicU32::new(speed.to_bits()),
            timepoint:    AtomicU32::new(0),
            t_generation: AtomicU32::new(0),
            program_start: Instant::now(),
        }
    }
    fn ns_since_start(&self) -> u64 {
        self.program_start.elapsed().as_nanos() as u64
    }
    fn zoom(&self)         -> f32 { f32::from_bits(self.zoom.load(Ordering::Relaxed)) }
    fn contrast_min(&self) -> f32 { f32::from_bits(self.contrast_min.load(Ordering::Relaxed)) }
    fn contrast_max(&self) -> f32 { f32::from_bits(self.contrast_max.load(Ordering::Relaxed)) }
    fn density_mult(&self) -> f32 { f32::from_bits(self.density_mult.load(Ordering::Relaxed)) }
    fn orbit_speed(&self)  -> f32 { f32::from_bits(self.orbit_speed.load(Ordering::Relaxed)) }
    fn timepoint(&self)    -> u32 { self.timepoint.load(Ordering::Relaxed) }
    fn t_generation(&self) -> u32 { self.t_generation.load(Ordering::Relaxed) }

    /// Apply `key=value` from the /set query string. Unknown keys ignored.
    fn apply(&self, key: &str, value: f32) {
        if key == "timepoint" {
            let new_t = value.max(0.0) as u32;
            let old_t = self.timepoint.swap(new_t, Ordering::Relaxed);
            if new_t != old_t {
                self.t_generation.fetch_add(1, Ordering::Relaxed);
            }
            return;
        }
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
    frame_slot: Arc<LatestFrame>,
    client_connected: Arc<AtomicBool>,
    view_state: Arc<ViewState>,
    n_timepoints: u32,
) {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .unwrap_or_else(|e| panic!("bind 0.0.0.0:{} failed: {}", port, e));

    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(socket) = incoming else { continue };
            let frame_slot = frame_slot.clone();
            let client_connected = client_connected.clone();
            let view_state = view_state.clone();
            std::thread::spawn(move || {
                handle_request(
                    socket, width, height, fps,
                    frame_slot, client_connected, view_state, n_timepoints,
                );
            });
        }
    });
}

fn handle_request(
    mut socket: TcpStream,
    width: u32, height: u32, fps: u32,
    frame_slot: Arc<LatestFrame>,
    client_connected: Arc<AtomicBool>,
    view_state: Arc<ViewState>,
    n_timepoints: u32,
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
        serve_html(&mut socket, &view_state, n_timepoints, width, height);
    } else if path == "/stream" {
        serve_stream(&mut socket, width, height, fps, frame_slot, client_connected);
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

fn serve_html(
    socket: &mut TcpStream,
    view: &ViewState,
    n_timepoints: u32,
    width: u32,
    height: u32,
) {
    // Hide the t-slider entirely on non-temporal datasets.
    let t_row_style = if n_timepoints > 1 { "" } else { "display:none;" };
    let t_max = n_timepoints.saturating_sub(1).max(0);
    let body = INDEX_HTML
        .replace("{{WIDTH}}",        &format!("{}", width))
        .replace("{{HEIGHT}}",       &format!("{}", height))
        .replace("{{ZOOM}}",         &format!("{}", view.zoom()))
        .replace("{{CONTRAST_MIN}}", &format!("{}", view.contrast_min()))
        .replace("{{CONTRAST_MAX}}", &format!("{}", view.contrast_max()))
        .replace("{{DENSITY_MULT}}", &format!("{}", view.density_mult()))
        .replace("{{ORBIT_SPEED}}",  &format!("{}", view.orbit_speed()))
        .replace("{{TIMEPOINT}}",    &format!("{}", view.timepoint()))
        .replace("{{T_MAX}}",        &format!("{}", t_max))
        .replace("{{T_STYLE}}",      t_row_style);
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
    frame_slot: Arc<LatestFrame>,
    client_connected: Arc<AtomicBool>,
) {
    // Single-client policy: refuse if someone else is already streaming.
    if client_connected
        .compare_exchange(false, true, Ordering::Relaxed, Ordering::Relaxed)
        .is_err()
    {
        let _ = socket.write_all(
            b"HTTP/1.0 503 Service Unavailable\r\n\
              Content-Type: text/plain\r\n\
              Connection: close\r\n\r\n\
              Another client is already streaming.\n",
        );
        return;
    }

    let mut ffmpeg = spawn_ffmpeg(width, height, fps);
    let mut ffmpeg_stdin = ffmpeg.stdin.take().expect("ffmpeg stdin");
    let mut stdout = ffmpeg.stdout.take().expect("ffmpeg stdout");

    println!("[orbit] HTTP /stream client connected; streaming fMP4");
    let header_ok = socket.write_all(
        b"HTTP/1.0 200 OK\r\n\
          Content-Type: video/mp4\r\n\
          Cache-Control: no-store\r\n\
          Access-Control-Allow-Origin: *\r\n\
          Connection: close\r\n\r\n",
    ).is_ok();

    if !header_ok {
        client_connected.store(false, Ordering::Relaxed);
        let _ = ffmpeg.kill();
        let _ = ffmpeg.wait();
        return;
    }

    // Writer thread: drains the latest-frame slot into ffmpeg.stdin. If
    // ffmpeg's stdin pipe is full (downstream stalled), this thread blocks
    // here — but the render thread keeps overwriting the slot, so no
    // backlog accumulates and slider events still apply promptly.
    frame_slot.reopen();
    let slot_for_writer = frame_slot.clone();
    let writer = std::thread::spawn(move || {
        while let Some(frame) = slot_for_writer.pop() {
            if ffmpeg_stdin.write_all(&frame).is_err() {
                break;
            }
        }
    });

    // Block here until the client TCP socket closes (i.e. browser
    // disconnects) or ffmpeg's stdout closes.
    let _ = std::io::copy(&mut stdout, socket);

    // Tear down: stop accepting new frames, wake the writer if it's blocked
    // in pop(), kill ffmpeg so any in-flight write_all returns an error.
    frame_slot.close();
    client_connected.store(false, Ordering::Relaxed);
    let _ = ffmpeg.kill();
    let _ = ffmpeg.wait();
    let _ = writer.join();
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
  canvas { display:block; width:100%; max-width:{{WIDTH}}px; height:auto;
           background:#000; border:1px solid #333; image-rendering:auto; }
  .row { display:flex; align-items:center; gap:10px; margin:6px 0; }
  .row label  { width:120px; color:#aaa; }
  .row input  { flex:1; }
  .row .val   { width:80px; text-align:right; color:#fff; }
  #status { color:#666; }
</style>
</head><body>
<h1>bovista orbit_stream
  <span style="float:right"><span id="status">connecting…</span>
  &nbsp;<span id="fps_val" style="color:#fff;font-variant-numeric:tabular-nums">—</span></span></h1>
<canvas id="live" width="{{WIDTH}}" height="{{HEIGHT}}"></canvas>
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
  <div class="row" style="{{T_STYLE}}"><label>timepoint</label>
    <input type="range" id="timepoint" min="0" max="{{T_MAX}}" step="1" value="{{TIMEPOINT}}">
    <span class="val" id="timepoint_val"></span></div>
</div>
<script src="https://cdn.jsdelivr.net/npm/mp4box@0.5.2/dist/mp4box.all.min.js"></script>
<script>
// WebCodecs-based player. We pull the fMP4 stream over HTTP, hand bytes to
// mp4box.js for demuxing, feed each H.264 frame to a VideoDecoder, then
// draw the resulting VideoFrame to the canvas. No <video> element and no
// MediaSource buffer — frames go on screen as fast as they decode, so
// latency is bounded by decoder + network jitter, not by browser policy.
const canvas = document.getElementById('live');
const ctx = canvas.getContext('2d');
const statusEl = document.getElementById('status');
const fpsEl = document.getElementById('fps_val');

let frameCount = 0, lastFpsAt = performance.now();
function tickFps() {
  frameCount++;
  const now = performance.now();
  if (now - lastFpsAt >= 1000) {
    fpsEl.textContent = (frameCount * 1000 / (now - lastFpsAt)).toFixed(1) + ' fps';
    frameCount = 0;
    lastFpsAt = now;
  }
}

const decoder = new VideoDecoder({
  output: (frame) => {
    // Match the canvas internal resolution to the first frame we see, so
    // the volume isn't stretched if --width/--height differ from the
    // backing encoder's output.
    if (canvas.width !== frame.codedWidth || canvas.height !== frame.codedHeight) {
      canvas.width = frame.codedWidth;
      canvas.height = frame.codedHeight;
    }
    ctx.drawImage(frame, 0, 0);
    frame.close();
    tickFps();
  },
  error: (e) => {
    statusEl.textContent = 'decoder error: ' + e.message;
    console.error('VideoDecoder error:', e);
  },
});

const mp4 = MP4Box.createFile();
mp4.onError = (e) => { statusEl.textContent = 'demux error: ' + e; console.error('MP4Box:', e); };
mp4.onReady = (info) => {
  const track = info.videoTracks[0];
  if (!track) { statusEl.textContent = 'no video track'; return; }
  // Extract AVCDecoderConfigurationRecord (avcC) for VideoDecoder.configure.
  // mp4box.js exposes DataStream as a top-level global (not on MP4Box),
  // but some bundles also re-export it via MP4Box.DataStream — accept either.
  const DS = (typeof DataStream !== 'undefined') ? DataStream : MP4Box.DataStream;
  const trak = mp4.getTrackById(track.id);
  let description = null;
  for (const entry of trak.mdia.minf.stbl.stsd.entries) {
    const box = entry.avcC || entry.hvcC || entry.vpcC;
    if (box) {
      const s = new DS(undefined, 0, DS.BIG_ENDIAN);
      box.write(s);
      description = new Uint8Array(s.buffer, 8); // skip 8-byte BoxHeader
      break;
    }
  }
  if (!description) { statusEl.textContent = 'no codec description'; return; }
  decoder.configure({
    codec: track.codec,
    description,
    optimizeForLatency: true,
    hardwareAcceleration: 'prefer-hardware',
  });
  mp4.setExtractionOptions(track.id, null, { nbSamples: 1 });
  mp4.start();
  statusEl.textContent = 'streaming ' + track.codec;
};
mp4.onSamples = (id, _user, samples) => {
  for (const s of samples) {
    decoder.decode(new EncodedVideoChunk({
      type: s.is_sync ? 'key' : 'delta',
      timestamp: s.cts * (1_000_000 / s.timescale),
      duration: (s.duration || 0) * (1_000_000 / s.timescale),
      data: s.data,
    }));
  }
};

(async () => {
  try {
    const resp = await fetch('/stream');
    if (!resp.ok) { statusEl.textContent = 'stream error ' + resp.status; return; }
    const reader = resp.body.getReader();
    let offset = 0;
    while (true) {
      const { done, value } = await reader.read();
      if (done) break;
      // mp4box wants an ArrayBuffer with `fileStart` saying where in the
      // stream these bytes start.
      const buf = value.buffer.slice(value.byteOffset, value.byteOffset + value.byteLength);
      buf.fileStart = offset;
      offset += buf.byteLength;
      mp4.appendBuffer(buf);
    }
    statusEl.textContent = 'stream ended';
  } catch (e) {
    statusEl.textContent = 'fetch error: ' + e.message;
    console.error(e);
  }
})();

// ── Sliders ────────────────────────────────────────────────────────────────
const fmt = { zoom: v=>(+v).toFixed(2)+'×',
              contrast_min: v=>(+v).toFixed(4),
              contrast_max: v=>(+v).toFixed(4),
              density_mult: v=>(+v).toFixed(2)+'×',
              orbit_speed:  v=>(+v).toFixed(2)+'×',
              timepoint:    v=>String(v|0) };
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

    use super::{FlushDiag, SceneSetup, ViewState};

    /// Max number of zarr-chunk reads we'll have in flight at once. NFS/VAST
    /// happily serve many parallel readers; the cap is mostly to keep the
    /// thread-spawn pile from getting ridiculous. For sharded zarr v3 arrays
    /// (e.g. zebrahub), `retrieve_array_subset` decodes only the inner chunks
    /// intersecting the tile region rather than the whole outer shard, so
    /// per-tile cost stays tens of ms even on multi-GB outer chunks.
    ///
    /// Each LOD level we'll serve. We type-erase the storage so the same struct
    /// can hold either a filesystem or http store.
    struct Level {
        array: Array<dyn ReadableStorageTraits>,
        /// Index of (z, y, x) within the array's possibly-higher-dim shape.
        z_idx: usize,
        y_idx: usize,
        x_idx: usize,
        /// Index of the "t" axis, if present.
        t_idx: Option<usize>,
        /// Volume dims in (z, y, x) voxel order at this LOD.
        volume_zyx: (u64, u64, u64),
    }

    pub fn open(
        path_or_url: &str,
        view_state: Arc<ViewState>,
        flush_diag: Arc<FlushDiag>,
        max_inflight: usize,
        prefetch_cap: usize,
    ) -> Result<SceneSetup, Box<dyn Error>> {
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

        let t_idx: Option<usize> = axes.iter().position(|a| a == "t");
        let n_timepoints: u32 = t_idx
            .map(|i| u32::try_from(lod0_arr.shape()[i]).unwrap_or(1).max(1))
            .unwrap_or(1);

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
                t_idx,
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

        // Single priority queue → single worker pool. Priority = distance
        // from the current displayed timepoint, computed at dequeue time so
        // a t-change instantly reorders the queue. The bovista loader and
        // the prefetcher both push into this same queue; whichever entry
        // is currently closest to view_state.timepoint() pops first.
        //
        // Workers process FIFO from the priority queue, decoding into
        // bovista's pending_chunks. No max_inflight cap — the queue's
        // built-in size limit is the backpressure.
        let levels = Arc::new(levels);
        let pending_slot: Arc<Mutex<Option<PendingChunks>>> = Arc::new(Mutex::new(None));
        let visible_snapshot: Arc<Mutex<Vec<TileKey>>> = Arc::new(Mutex::new(Vec::new()));
        let scheduler = LoaderScheduler::new(max_inflight.saturating_mul(16).max(512));
        println!("[orbit] scheduler: {} worker(s), queue cap {} entries",
                 max_inflight, scheduler.cap());

        // Worker pool: each thread loops pop_best → decode → push to pending.
        for _ in 0..max_inflight {
            let scheduler = scheduler.clone();
            let levels = levels.clone();
            let lods = lods.clone();
            let pending_slot = pending_slot.clone();
            let view_state = view_state.clone();
            let flush_diag = flush_diag.clone();
            thread::spawn(move || loop {
                // Pass a closure (not a snapshot) so pop_best re-reads
                // current_t each scan. Otherwise a worker that's been
                // blocked in cvar.wait could wake up and evaluate freshly
                // pushed entries against a long-stale current_t, drop
                // them as "too far," and deadlock the loading pipeline —
                // which is exactly what was happening during scrubbing.
                let key = match scheduler.pop_best(|| view_state.timepoint(), 3) {
                    Some(k) => k,
                    None => break,
                };
                if key.lod_level >= levels.len() { continue; }
                let lod_cfg = &lods[key.lod_level];
                let decode_start = std::time::Instant::now();
                let result = read_tile(&levels[key.lod_level], lod_cfg, key, key.t);
                let decode_ns = decode_start.elapsed().as_nanos() as u64;
                match result {
                    Ok(data) => {
                        if let Some(p) = pending_slot.lock().unwrap().clone() {
                            p.lock().unwrap().insert(key, data);
                            flush_diag.note_tile(view_state.ns_since_start(), decode_ns);
                        }
                    }
                    Err(e) => {
                        eprintln!("[orbit] tile load failed (lod={} t={} z={} y={} x={}): {}",
                                  key.lod_level, key.t, key.z, key.y, key.x, e);
                    }
                }
            });
        }

        // Prefetching now lives inside bovista's `prepare_volume` (via
        // `volume.set_prefetch(...)`), so it shares the SAME request
        // path as current-t — slot_map check + request_tile dedupe.
        // The atlas is the single source of truth for "do we need to
        // fetch this?"; no external dispatcher, no separate dedupe.
        let _ = prefetch_cap;

        // Bovista's loader callback: just push to the scheduler. If the
        // queue is full and our key is lower-priority than what's already
        // queued, return Rejected so bovista doesn't mark it as requested
        // (we'll try again next frame when the queue has drained).
        let scheduler_for_loader = scheduler.clone();
        let view_state_for_loader = view_state.clone();
        let levels_for_loader = levels.clone();
        let loader: TileLoaderFn = Box::new(move |req: TileRequest| -> ChunkStatus {
            let lod = req.lod_level.unwrap_or(0);
            if lod >= levels_for_loader.len() {
                return ChunkStatus::Rejected;
            }
            let key = TileKey { lod_level: lod, t: req.t, z: req.z, y: req.y, x: req.x };
            let current_t = view_state_for_loader.timepoint();
            if scheduler_for_loader.push(key, current_t) {
                ChunkStatus::Accepted
            } else {
                ChunkStatus::Rejected
            }
        });

        println!("[orbit] OME-Zarr: t axis has {} timepoint(s)", n_timepoints);

        Ok(SceneSetup {
            lods,
            loader,
            pending_slot,
            world_extents,
            // Comfortably bigger than a single LOD's tile count for the
            // OME-Zarr datasets we care about. Override with --cache-tiles.
            cache_capacity: 4096,
            n_timepoints,
            visible_snapshot,
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

    /// Spawn the prefetcher: one dispatcher thread that periodically scans
    /// the visible set and enqueues (t+offset, TileKey) reads for the next
    /// few timepoints, plus N worker threads that consume the queue and
    /// fill the cache.
    ///
    /// When the user advances to t+1, the regular loader hits the cache
    /// instead of doing fresh I/O — sequential scrubbing / playback is
    /// reduced to GPU upload time only.
    /// Single priority queue + condvar. Items are tile keys; priority is
    /// `|key.t - current_t|` computed at *pop* time, so changes to the
    /// current timepoint instantly re-prioritize everything in the queue.
    ///
    /// `push` returns false (rejected) only if the queue is full AND the
    /// caller's priority isn't better than the worst item in it; bovista
    /// gets `Rejected` for that case and won't mark it as requested, so
    /// it'll retry next frame.
    #[derive(Clone)]
    pub struct LoaderScheduler {
        inner: Arc<LoaderSchedulerInner>,
    }
    struct LoaderSchedulerInner {
        queue: Mutex<HashSet<TileKey>>,
        cvar:  std::sync::Condvar,
        cap:   usize,
    }
    impl LoaderScheduler {
        fn new(cap: usize) -> Self {
            Self { inner: Arc::new(LoaderSchedulerInner {
                queue: Mutex::new(HashSet::new()),
                cvar:  std::sync::Condvar::new(),
                cap,
            })}
        }
        pub fn cap(&self) -> usize { self.inner.cap }
        /// Enqueue a tile request. If the queue is at capacity, drop the
        /// worst-priority existing entry IF this one is better, or reject.
        pub fn push(&self, key: TileKey, current_t: u32) -> bool {
            let mut q = self.inner.queue.lock().unwrap();
            if q.contains(&key) { return true; }
            if q.len() >= self.inner.cap {
                let our_pri  = (key.t as i32 - current_t as i32).abs();
                if let Some(worst) = q.iter()
                    .max_by_key(|k| (k.t as i32 - current_t as i32).abs())
                    .copied()
                {
                    let worst_pri = (worst.t as i32 - current_t as i32).abs();
                    if our_pri < worst_pri {
                        q.remove(&worst);
                    } else {
                        return false;
                    }
                }
            }
            q.insert(key);
            self.inner.cvar.notify_one();
            true
        }
        /// Block until a tile is available, then pop the one closest to
        /// the current timepoint.
        /// Pop the entry closest to the CURRENT timepoint (re-read each
        /// scan), while opportunistically dropping entries farther than
        /// `max_useful_dist` away. Re-reading `current_t` each iteration
        /// matters: a worker may have been blocked in `cvar.wait` for a
        /// long time while the user scrubbed; bovista pushes entries for
        /// the NEW t, and if we kept using the stale `current_t` from
        /// when this call started, those new entries would look "stale"
        /// and get dropped — which is exactly the deadlock-via-mismatch
        /// bug that produced "nothing ever loads" during scrubbing.
        pub fn pop_best<F: Fn() -> u32>(&self, current_t: F, max_useful_dist: i32) -> Option<TileKey> {
            let mut q = self.inner.queue.lock().unwrap();
            loop {
                let now_t = current_t();
                let mut best: Option<TileKey> = None;
                let mut best_pri = i32::MAX;
                let mut stale: Vec<TileKey> = Vec::new();
                for &k in q.iter() {
                    let pri = (k.t as i32 - now_t as i32).abs();
                    if pri > max_useful_dist {
                        stale.push(k);
                    } else if pri < best_pri {
                        best_pri = pri;
                        best = Some(k);
                    }
                }
                for k in &stale { q.remove(k); }
                if let Some(b) = best {
                    q.remove(&b);
                    return Some(b);
                }
                q = self.inner.cvar.wait(q).unwrap();
            }
        }
    }

    /// Read the voxel sub-region for this tile and convert to R16Float bytes.
    ///
    /// We always go through `retrieve_array_subset`, which respects the codec
    /// pipeline including `sharding_indexed` — for a sharded array (e.g.
    /// zebrahub: outer chunks 512×2048×2048 containing 1024 inner 128³
    /// chunks), zarrs reads the shard index, seeks to just the inner chunks
    /// that intersect this tile, and decompresses only those (~500 KB each
    /// instead of the whole 4 GB shard). For non-sharded arrays the same
    /// call decodes whatever outer chunks the tile intersects.
    fn read_tile(
        level: &Level,
        lod_cfg: &LodLevelConfig,
        key: TileKey,
        timepoint: u32,
    ) -> Result<TileData, Box<dyn Error + Send + Sync>> {
        let (tz, ty, tx) = lod_cfg.tile_size;
        let (vz, vy, vx) = level.volume_zyx;

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

        let ndim = level.array.shape().len();
        let mut ranges: Vec<std::ops::Range<u64>> = (0..ndim).map(|_| 0..1).collect();
        ranges[level.z_idx] = z0..z1;
        ranges[level.y_idx] = y0..y1;
        ranges[level.x_idx] = x0..x1;
        if let Some(ti) = level.t_idx {
            let tmax = level.array.shape()[ti].saturating_sub(1) as u32;
            let t = timepoint.min(tmax) as u64;
            ranges[ti] = t..(t + 1);
        }
        let subset = ArraySubset::new_with_ranges(&ranges);

        let dtype_name = level
            .array
            .data_type()
            .name(zarrs::plugin::ZarrVersion::V3)
            .map(|n| n.to_string())
            .unwrap_or_default();

        static DBG_ONCE: std::sync::Once = std::sync::Once::new();
        DBG_ONCE.call_once(|| {
            println!("[orbit] zarr dtype detected as: \"{}\"", dtype_name);
        });

        let f16_bytes = match dtype_name.as_str() {
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
