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
//!   --cache-tiles N      atlas slot count (default 1 synthetic, 4096 zarr)
//!   --prefetch N         timepoints to prefetch ahead of desired_t (default 16)
//!   --contrast-min F     contrast window floor (default 0.0)
//!   --contrast-max F     contrast window ceiling (default 1.0)
//!   --density-mult F     multiplier on the auto-density heuristic (default 1.0)
//!   --lod-bias F         shader LOD bias (+ = finer, − = coarser; default 0.0)
//!
//! Set BOVISTA_FORCE_LIBX264=1 to skip the NVENC probe and always use libx264.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::{Duration, Instant};

use bovista::visual::CameraInfo;
use bovista::visuals::virtual_texture::PendingChunks;
use bovista::visuals::{
    AverageVolume, DirectVolume, IsosurfaceVolume,
    LodLevelConfig, MinipVolume, MipVolume,
};
use bovista::visuals::virtual_texture::{PrepareStats, Wanted};
use bovista::{Camera, ProjectionMode, Renderer, Scene};

// Shared OME-Zarr loader machinery, lifted out of this file into examples/common/
// and dedup'd across the native examples. `loader` and `ome_zarr` are mutually
// referential (ome_zarr::open spawns loader::spawn_workers; loader reads
// ome_zarr::read_tile), so both are top-level modules reachable as crate::*.
#[path = "../../common/loader.rs"]
mod loader;
#[path = "../../common/ome_zarr.rs"]
mod ome_zarr;

// ── Mode dispatch ───────────────────────────────────────────────────────────
//
// orbit_stream picks one of bovista's six volume render modes at startup via
// `--mode`. The visual type is fixed for the process lifetime; runtime
// switching without rebuilding the atlas is a follow-up (it needs the
// VolumeBackend split discussed in PR #6's notes).

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum VolumeMode { Direct, Mip, Minip, Average, Iso }

impl VolumeMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "direct"     => Some(Self::Direct),
            "mip"        => Some(Self::Mip),
            "minip"      => Some(Self::Minip),
            "average"    => Some(Self::Average),
            "iso" | "isosurface" => Some(Self::Iso),
            _ => None,
        }
    }
}

/// Wraps any of the six volume visual types behind a single Visual impl. The
/// common bovista methods (contrast, lod_bias, prefetch, t-controls, stats, …)
/// all match-dispatch through `volume_dispatch!`; the few mode-specific ones
/// (density on Direct, attenuation on MIP, iso_threshold on Iso) are
/// no-ops on the variants that don't carry that parameter.
enum Volume {
    Direct(DirectVolume),
    Mip(MipVolume),
    Minip(MinipVolume),
    Average(AverageVolume),
    Iso(IsosurfaceVolume),
}

macro_rules! volume_dispatch {
    ($self:ident, $v:ident => $body:expr) => {
        match $self {
            Volume::Direct($v)   => $body,
            Volume::Mip($v)      => $body,
            Volume::Minip($v)    => $body,
            Volume::Average($v)  => $body,
            Volume::Iso($v)      => $body,
        }
    };
}

impl Volume {
    fn new(
        mode: VolumeMode,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        surface_format: wgpu::TextureFormat,
        camera_bgl: &wgpu::BindGroupLayout,
        lod_levels: Vec<LodLevelConfig>,
        cache_capacity: usize,
        atlas_count: usize,
    ) -> Self {
        match mode {
            VolumeMode::Direct   => Volume::Direct(DirectVolume::new(device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count)),
            VolumeMode::Mip      => Volume::Mip(MipVolume::new(device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count)),
            VolumeMode::Minip    => Volume::Minip(MinipVolume::new(device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count)),
            VolumeMode::Average  => Volume::Average(AverageVolume::new(device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count)),
            VolumeMode::Iso      => Volume::Iso(IsosurfaceVolume::new(device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count)),
        }
    }

    // Common methods — every variant has these.
    fn set_contrast_limits(&mut self, min: f32, max: f32) {
        volume_dispatch!(self, v => v.set_contrast_limits(min, max));
    }
    fn set_lod_bias(&mut self, bias: f32) {
        volume_dispatch!(self, v => v.set_lod_bias(bias));
    }
    fn set_desired_timepoint(&mut self, t: u32) {
        volume_dispatch!(self, v => v.set_desired_timepoint(t));
    }
    fn set_prefetch(&mut self, lookahead: u32, t_count: u32) {
        volume_dispatch!(self, v => v.set_prefetch(lookahead, t_count));
    }
    fn pending_chunks(&self) -> Option<PendingChunks> {
        volume_dispatch!(self, v => v.pending_chunks())
    }
    fn wanted_handle(&self) -> Wanted {
        volume_dispatch!(self, v => v.wanted_handle())
    }
    fn stats(&self) -> PrepareStats {
        volume_dispatch!(self, v => v.stats())
    }
    fn current_t_load_status(&self) -> (usize, usize) {
        volume_dispatch!(self, v => v.current_t_load_status())
    }
    fn desired_t(&self) -> u32 {
        volume_dispatch!(self, v => v.desired_t())
    }

    // Mode-specific: only applicable variants do anything.
    fn try_set_density_scale(&mut self, scale: f32) {
        if let Volume::Direct(v) = self { v.set_density_scale(scale); }
    }
    fn try_set_attenuation(&mut self, attenuation: f32) {
        if let Volume::Mip(v) = self { v.set_attenuation(attenuation); }
    }
    fn try_set_iso_threshold(&mut self, threshold: f32) {
        if let Volume::Iso(v) = self { v.set_iso_threshold(threshold); }
    }
}

impl bovista::Visual for Volume {
    fn prepare(&mut self, d: &wgpu::Device, q: &wgpu::Queue, ci: &bovista::visual::CameraInfo) {
        volume_dispatch!(self, v => v.prepare(d, q, ci));
    }
    fn render(&self, rp: &mut wgpu::RenderPass) {
        volume_dispatch!(self, v => v.render(rp));
    }
    fn set_transform(&mut self, t: bovista::Transform) {
        volume_dispatch!(self, v => v.set_transform(t));
    }
    fn transform(&self) -> &bovista::Transform {
        volume_dispatch!(self, v => v.transform())
    }
    fn is_visible(&self) -> bool {
        volume_dispatch!(self, v => v.is_visible())
    }
    fn set_visible(&mut self, vis: bool) {
        volume_dispatch!(self, v => v.set_visible(vis));
    }
    fn name(&self) -> &str {
        volume_dispatch!(self, v => v.name())
    }
}

fn main() {
    env_logger::init();

    let args: Vec<String> = std::env::args().skip(1).collect();
    const KNOWN_FLAGS: &[&str] = &[
        "--width", "--height", "--fps", "--port", "--backend",
        "--zarr", "--cache-tiles", "--atlas-count", "--max-inflight", "--prefetch",
        "--contrast-min", "--contrast-max", "--density-mult", "--lod-bias",
        "--timepoint", "--mode", "--attenuation", "--iso-threshold",
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
    // 1..=MAX_ATLAS_COUNT; default 1 (single atlas, identical to pre-multi-atlas behaviour).
    let atlas_count: usize = flag_str_opt(&args, "--atlas-count")
        .and_then(|v| v.parse().ok()).unwrap_or(1);
    let max_inflight: usize = flag_str_opt(&args, "--max-inflight")
        .and_then(|v| v.parse().ok())
        .unwrap_or(auto_inflight);
    // How many timepoints ahead of `desired_t` to prefetch. Atlas
    // capacity / visible-tile-count tiles per timepoint puts a natural
    // ceiling (e.g. 4096 / 180 ≈ 22 timepoints). Lookahead = 16 fills
    // most of that on a stock --cache-tiles=4096; bump higher with
    // --cache-tiles for more.
    let prefetch_lookahead: u32 = flag_str_opt(&args, "--prefetch")
        .and_then(|v| v.parse().ok())
        .unwrap_or(16);
    println!(
        "[orbit] thread budget: rayon={} max_inflight={} prefetch={} (cpus={})",
        rayon_threads, max_inflight, prefetch_lookahead, cpus,
    );

    let contrast_min: f32 = flag_str_opt(&args, "--contrast-min").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let contrast_max: f32 = flag_str_opt(&args, "--contrast-max").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let density_mult: f32 = flag_str_opt(&args, "--density-mult").and_then(|v| v.parse().ok()).unwrap_or(1.0);
    let lod_bias: f32 = flag_str_opt(&args, "--lod-bias").and_then(|v| v.parse().ok()).unwrap_or(0.0);
    // Rendering mode is fixed at startup; the per-mode visual stores the right
    // pipeline. Runtime mode-switching (without atlas rebuild) needs the
    // VolumeBackend split — see the follow-up PR.
    let mode: VolumeMode = flag_str_opt(&args, "--mode")
        .as_deref().and_then(VolumeMode::parse).unwrap_or(VolumeMode::Direct);
    let mip_attenuation: f32 = flag_str_opt(&args, "--attenuation")
        .and_then(|v| v.parse().ok()).unwrap_or(0.0);
    let iso_threshold: f32 = flag_str_opt(&args, "--iso-threshold")
        .and_then(|v| v.parse().ok()).unwrap_or(0.5);

    println!("[orbit] {}x{} @ {} fps, serving on port {}, backend={}",
             width, height, fps, port, backend);
    let zarr_arg = zarr_arg.unwrap_or_else(|| {
        eprintln!("[orbit] --zarr <path|url> is required");
        std::process::exit(1);
    });
    println!("[orbit] OME-Zarr source: {}", zarr_arg);

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
        lod_bias,
    ));
    let flush_diag = Arc::new(FlushDiag::new());

    // Current timepoint handle for the shared loader (parity slot; tile reads
    // use key.t). The per-tile diagnostics hook bridges to this binary's
    // FlushDiag, using view_state's clock so flush_t0_ns and arrival times
    // share an origin.
    let current_t = Arc::new(AtomicU32::new(0));
    let tile_hook: ome_zarr::TileHook = {
        let flush_diag = flush_diag.clone();
        let view_state = view_state.clone();
        Arc::new(move |_now_ns, decode_ns| {
            flush_diag.note_tile(view_state.ns_since_start(), decode_ns);
        })
    };

    let setup = match ome_zarr::open(&zarr_arg, current_t.clone(), tile_hook,
                                       max_inflight) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[orbit] failed to open OME-Zarr at {}: {}", zarr_arg, e);
            std::process::exit(1);
        }
    };
    let cache_capacity = cache_override.unwrap_or(setup.cache_capacity);
    let n_timepoints = setup.n_timepoints;
    let queue_count_at = setup.queue_count_at.clone();

    let mut volume = Volume::new(
        mode,
        renderer.device(), renderer.queue(), renderer.surface_format(),
        renderer.camera_bind_group_layout(),
        setup.lods.clone(), cache_capacity as usize, atlas_count,
    );
    // Apply mode-specific startup parameters (no-ops on variants that
    // don't carry them — see Volume::try_set_* for the dispatch table).
    volume.try_set_attenuation(mip_attenuation);
    volume.try_set_iso_threshold(iso_threshold);
    println!(
        "[orbit] render mode: {:?} (--attenuation={} --iso-threshold={})",
        mode, mip_attenuation, iso_threshold,
    );
    *setup.pending_slot.lock().unwrap() = Some(volume.pending_chunks().unwrap());
    *setup.wanted_slot.lock().unwrap() = Some(volume.wanted_handle());
    // Background-prefetch the next `--prefetch` timepoints. Workers
    // pull current-t first (priority 0) and only fetch prefetch keys
    // when current-t is satisfied; `pick_slot` evicts prefetch slots
    // before current ones, so prefetch never starves a live scrub.
    if n_timepoints > 1 {
        volume.set_prefetch(prefetch_lookahead, n_timepoints);
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
        "[orbit] base_density={:.2} (× {} init); contrast=[{:.3}, {:.3}] init; lod_bias={:+.1} init",
        base_density, density_mult, contrast_min, contrast_max, lod_bias,
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
    let perf = Arc::new(PerfCounters::new());
    spawn_http_server(
        port, width, height, fps,
        frame_slot.clone(), client_connected.clone(),
        view_state.clone(), n_timepoints, perf.clone(),
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
    // Perf accumulators — flushed once per wall-clock second.
    let mut perf_last_report = Instant::now();
    let mut perf_frame_count: u32 = 0;
    let mut perf_prepare_ns_sum: u128 = 0;
    let mut perf_prepare_ns_max: u128 = 0;
    #[allow(unused_assignments)]
    let mut perf_last_stats = bovista::visuals::virtual_texture::PrepareStats::default();
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
            // No-op on modes that don't use density (MIP / minIP / Average / Iso).
            v.try_set_density_scale(base_density * view_state.density_mult());
            v.set_lod_bias(view_state.lod_bias());
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
                current_t.store(view_state.timepoint(), Ordering::Relaxed);
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
        // Sample diagnostic counters from bovista for the progress log.
        let (loaded_now, visible_now, current_t_now, prep_stats) = {
            let v = volume_arc.lock().unwrap();
            let (loaded, visible) = v.current_t_load_status();
            (loaded, visible, v.desired_t(), v.stats())
        };
        // Perf accumulators.
        perf_frame_count += 1;
        let prep_ns = prepare_dt.as_nanos();
        perf_prepare_ns_sum += prep_ns;
        if prep_ns > perf_prepare_ns_max { perf_prepare_ns_max = prep_ns; }
        perf_last_stats = prep_stats;

        // Once per wall-clock second: emit [progress] + [perf]. The
        // wall-clock gate (vs `frame_n % 30`) means the report rate
        // is independent of any frame-pacer skips.
        if perf_last_report.elapsed() >= Duration::from_secs(1) {
            let elapsed_s = perf_last_report.elapsed().as_secs_f32();
            let in_queue = (queue_count_at)(current_t_now);
            let fps = perf_frame_count as f32 / elapsed_s;
            let avg_ms = (perf_prepare_ns_sum as f32 / perf_frame_count as f32) / 1_000_000.0;
            let max_ms = perf_prepare_ns_max as f32 / 1_000_000.0;
            let s = &perf_last_stats;
            let pp_ms     = s.process_pending_ns as f32 / 1_000_000.0;
            let rpt_ms    = s.refresh_page_table_ns as f32 / 1_000_000.0;
            let pw_ms     = s.publish_wanted_ns as f32 / 1_000_000.0;
            // Snapshot + reset the cross-thread counters.
            let writer_pops    = perf.writer_pops.swap(0, Ordering::Relaxed);
            let writer_bytes   = perf.writer_bytes.swap(0, Ordering::Relaxed);
            let stream_bytes   = perf.stream_bytes_out.swap(0, Ordering::Relaxed);
            let set_count      = perf.set_count.swap(0, Ordering::Relaxed);
            let set_ns_total   = perf.set_ns_total.swap(0, Ordering::Relaxed);
            let set_ns_max     = perf.set_ns_max.swap(0, Ordering::Relaxed);
            let writer_fps     = writer_pops as f32 / elapsed_s;
            let writer_mibps   = (writer_bytes as f32 / 1_048_576.0) / elapsed_s;
            let stream_mibps   = (stream_bytes as f32 / 1_048_576.0) / elapsed_s;
            let set_avg_ms     = if set_count > 0 {
                (set_ns_total as f32 / set_count as f32) / 1_000_000.0
            } else { 0.0 };
            let set_max_ms     = set_ns_max as f32 / 1_000_000.0;
            let all_loaded = loaded_now == visible_now;
            if !(all_loaded && in_queue == 0) {
                println!("[progress] t={} loaded {}/{} | queued {}",
                         current_t_now, loaded_now, visible_now, in_queue);
            }
            println!(
                "[perf] render fps={:.1} prep avg={:.1}ms max={:.1}ms | \
                 pp={:.1}ms (in {}, drop {}, atlas {}, pt {}) | \
                 refresh={:.1}ms | publish={:.1}ms | \
                 slot_map={} pending={}",
                fps, avg_ms, max_ms,
                pp_ms, s.tiles_installed, s.tiles_dropped, s.atlas_writes, s.page_table_writes,
                rpt_ms, pw_ms,
                s.slot_map_len, s.pending_len,
            );
            println!(
                "[perf] writer fps={:.1} ({:.1} MiB/s to ffmpeg) | \
                 stream out {:.1} MiB/s | \
                 /set count={} avg={:.1}ms max={:.1}ms",
                writer_fps, writer_mibps,
                stream_mibps,
                set_count, set_avg_ms, set_max_ms,
            );
            perf_last_report = Instant::now();
            perf_frame_count = 0;
            perf_prepare_ns_sum = 0;
            perf_prepare_ns_max = 0;
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

// SceneSetup now lives in examples/common/ome_zarr.rs (ome_zarr::SceneSetup),
// shared with the slice_viewer / volume_renderer native examples.

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
    lod_bias:     AtomicU32,
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
/// Per-second counters covering paths that aren't in the render
/// thread's `[perf]` line: the stream writer + socket loop, and the
/// /set HTTP handler. All read+reset once per second from the render
/// loop alongside bovista's stats.
struct PerfCounters {
    /// Successful `LatestFrame::pop()` calls (= frames actually
    /// handed off to ffmpeg). Should be ~fps if encoder is keeping up.
    writer_pops: AtomicU64,
    /// Bytes written to ffmpeg's stdin (BGRA raw frames). Roughly
    /// `writer_pops × width × height × 4`.
    writer_bytes: AtomicU64,
    /// Bytes pulled out of ffmpeg's stdout and written to the /stream
    /// socket. Roughly `bitrate / 8` if encoder + socket are healthy.
    stream_bytes_out: AtomicU64,
    /// Number of /set requests served in this interval.
    set_count: AtomicU64,
    /// Cumulative ns spent in /set requests (from accept to response
    /// flush). `/set_ns_total / set_count` = avg duration.
    set_ns_total: AtomicU64,
    /// Max single-request duration seen.
    set_ns_max: AtomicU64,
}

impl PerfCounters {
    fn new() -> Self {
        Self {
            writer_pops: AtomicU64::new(0),
            writer_bytes: AtomicU64::new(0),
            stream_bytes_out: AtomicU64::new(0),
            set_count: AtomicU64::new(0),
            set_ns_total: AtomicU64::new(0),
            set_ns_max: AtomicU64::new(0),
        }
    }
    /// Atomic max — updates `set_ns_max` to `value` if greater.
    fn record_set_max(&self, value: u64) {
        let mut cur = self.set_ns_max.load(Ordering::Relaxed);
        while value > cur {
            match self.set_ns_max.compare_exchange_weak(
                cur, value, Ordering::Relaxed, Ordering::Relaxed,
            ) {
                Ok(_) => return,
                Err(c) => cur = c,
            }
        }
    }
}

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
    fn new(zoom: f32, cmin: f32, cmax: f32, dens: f32, speed: f32, lod_bias: f32) -> Self {
        Self {
            zoom:         AtomicU32::new(zoom.to_bits()),
            contrast_min: AtomicU32::new(cmin.to_bits()),
            contrast_max: AtomicU32::new(cmax.to_bits()),
            density_mult: AtomicU32::new(dens.to_bits()),
            orbit_speed:  AtomicU32::new(speed.to_bits()),
            lod_bias:     AtomicU32::new(lod_bias.to_bits()),
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
    fn lod_bias(&self)     -> f32 { f32::from_bits(self.lod_bias.load(Ordering::Relaxed)) }
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
            "lod_bias"     => &self.lod_bias,
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
    perf: Arc<PerfCounters>,
) {
    let listener = TcpListener::bind(("0.0.0.0", port))
        .unwrap_or_else(|e| panic!("bind 0.0.0.0:{} failed: {}", port, e));

    std::thread::spawn(move || {
        for incoming in listener.incoming() {
            let Ok(socket) = incoming else { continue };
            let frame_slot = frame_slot.clone();
            let client_connected = client_connected.clone();
            let view_state = view_state.clone();
            let perf = perf.clone();
            std::thread::spawn(move || {
                handle_request(
                    socket, width, height, fps,
                    frame_slot, client_connected, view_state, n_timepoints, perf,
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
    perf: Arc<PerfCounters>,
) {
    let accept_t = Instant::now();
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
        serve_stream(&mut socket, width, height, fps, frame_slot, client_connected, perf);
    } else if let Some(query) = path.strip_prefix("/set?") {
        serve_set(&mut socket, query, &view_state);
        // /set request timing — accept → response written. The
        // [perf] line samples per-second avg + max for this.
        let dt_ns = accept_t.elapsed().as_nanos() as u64;
        perf.set_count.fetch_add(1, Ordering::Relaxed);
        perf.set_ns_total.fetch_add(dt_ns, Ordering::Relaxed);
        perf.record_set_max(dt_ns);
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
        .replace("{{LOD_BIAS}}",     &format!("{}", view.lod_bias()))
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
    perf: Arc<PerfCounters>,
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
    let perf_for_writer = perf.clone();
    let writer = std::thread::spawn(move || {
        while let Some(frame) = slot_for_writer.pop() {
            let n = frame.len() as u64;
            if ffmpeg_stdin.write_all(&frame).is_err() {
                break;
            }
            // Count pops + bytes that successfully reached ffmpeg.
            perf_for_writer.writer_pops.fetch_add(1, Ordering::Relaxed);
            perf_for_writer.writer_bytes.fetch_add(n, Ordering::Relaxed);
        }
    });

    // ffmpeg stdout → socket. Counted copy so we can see whether the
    // socket write side is keeping up with the encoder.
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = match stdout.read(&mut buf) {
            Ok(0) | Err(_) => break,
            Ok(n) => n,
        };
        if socket.write_all(&buf[..n]).is_err() { break; }
        perf.stream_bytes_out.fetch_add(n as u64, Ordering::Relaxed);
    }

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
  <div class="row"><label>LOD bias</label>
    <input type="range" id="lod_bias" min="-3" max="3" step="0.1" value="{{LOD_BIAS}}">
    <span class="val" id="lod_bias_val"></span></div>
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
              lod_bias:     v=>(+v >= 0 ? '+' : '') + (+v).toFixed(1),
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
