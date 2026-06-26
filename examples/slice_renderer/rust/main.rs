//! Slice renderer — native interactive Rust example.
//!
//! Streams an OME-Zarr multiscale pyramid and renders an arbitrary slice plane
//! through it via bovista's virtual-texture `Image` visual. Tiles are pulled
//! from S3/HTTP (or a local filesystem path) on a background worker pool as the
//! camera and slice plane move.
//!
//! Usage:
//!   cargo run --release --example slice_renderer
//!   cargo run --release --example slice_renderer -- --zarr <path-or-url>
//!
//! Controls:
//!   Left-drag    orbit camera (pan in orthographic mode)
//!   Scroll       zoom
//!   Right-drag   rotate slice (Y axis; +Shift = X axis) + horizontal offset
//!   Middle-drag  window / level (contrast)
//!   [ / ]        LOD bias coarser / finer
//!   P            toggle perspective / orthographic
//!   D            toggle LOD debug tinting
//!   R            reset slice + contrast
//!
//! Flags: --zarr, --cache-tiles, --atlas-count, --max-inflight, --lod-bias,
//!        --contrast-min, --contrast-max.
//!
//! Note: the `zarrs_http` HTTP store pulls in openssl; see the example README
//! for the build dependency.

use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use bovista::{Image, SlicePlane};
use winit::keyboard::Key;

#[path = "../../common/app.rs"]
mod app;
#[path = "../../common/camera_setup.rs"]
mod camera_setup;
#[path = "../../common/cli.rs"]
mod cli;
#[path = "../../common/loader.rs"]
mod loader;
#[path = "../../common/ome_zarr.rs"]
mod ome_zarr;

use app::{BuildCtx, ExampleApp, FrameCtx, SecondaryButton};

const DEFAULT_ZARR: &str =
    "https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/marmoset_neurons.ome.zarr";

struct SliceRenderer {
    setup: ome_zarr::SceneSetup,
    args: ResolvedArgs,

    image: Option<Arc<Mutex<Image>>>,
    world_max: f32,

    // Slice plane state (matches the Python example's angles/offset model).
    slice_angle_x: f32,
    slice_angle_y: f32,
    slice_offset: f32,
    // Window/level contrast state.
    window_center: f32,
    window_width: f32,
    lod_bias: f32,
    debug: bool,
}

struct ResolvedArgs {
    cache_tiles: Option<u32>,
    atlas_count: usize,
    lod_bias: f32,
    contrast_min: f32,
    contrast_max: f32,
}

impl SliceRenderer {
    fn apply_slice_plane(&mut self) {
        let Some(image) = &self.image else { return };
        let (sx, cx) = self.slice_angle_x.sin_cos();
        let (sy, cy) = self.slice_angle_y.sin_cos();
        // Normal from angles (matches remote_ome_zarr.py::_update_slice_plane).
        let mut nx = sy * cx;
        let mut ny = -sx;
        let mut nz = cy * cx;
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len > 1e-4 {
            nx /= len;
            ny /= len;
            nz /= len;
        }
        // World frame is centered on origin (the loader translates LOD0 there),
        // so the slice position is just normal * offset.
        let pos = [nx * self.slice_offset, ny * self.slice_offset, nz * self.slice_offset];
        image
            .lock()
            .unwrap()
            .set_slice_plane(SlicePlane::new(pos, [nx, ny, nz]));
    }

    fn apply_window_level(&mut self) {
        let Some(image) = &self.image else { return };
        let half = self.window_width / 2.0;
        image
            .lock()
            .unwrap()
            .set_contrast_limits(self.window_center - half, self.window_center + half);
    }
}

impl ExampleApp for SliceRenderer {
    fn build(&mut self, ctx: &mut BuildCtx) {
        let r = ctx.renderer;
        let cache = self
            .args
            .cache_tiles
            .unwrap_or(self.setup.cache_capacity) as usize;
        let mut image = Image::new(
            r.device(),
            r.queue(),
            r.surface_format(),
            r.camera_bind_group_layout(),
            self.setup.lods.clone(),
            cache,
            self.args.atlas_count,
        );
        image.set_lod_bias(self.args.lod_bias);
        image.set_contrast_limits(self.args.contrast_min, self.args.contrast_max);
        image.set_name("slice".to_string());

        // Wire the loader: hand it the visual's pending + wanted handles.
        *self.setup.pending_slot.lock().unwrap() = image.pending_chunks();
        *self.setup.wanted_slot.lock().unwrap() = Some(image.wanted_handle());

        let image = Arc::new(Mutex::new(image));
        ctx.scene.add(image.clone());
        self.image = Some(image);

        // Frame the camera on the volume extents.
        *ctx.camera = camera_setup::camera_for_extents(
            ctx.width as f32 / ctx.height as f32,
            self.setup.world_extents,
            2.0,
        );
        self.world_max = camera_setup::world_max(self.setup.world_extents);

        self.window_center = (self.args.contrast_min + self.args.contrast_max) / 2.0;
        self.window_width = (self.args.contrast_max - self.args.contrast_min).max(0.001);
        self.lod_bias = self.args.lod_bias;
        self.apply_slice_plane();
    }

    fn on_key(&mut self, key: &Key, _ctx: &mut FrameCtx) {
        let Some(ch) = app::char_key(key) else { return };
        match ch {
            '[' => {
                self.lod_bias -= 0.5;
                if let Some(i) = &self.image {
                    i.lock().unwrap().set_lod_bias(self.lod_bias);
                }
                println!("[slice] lod_bias = {:+.1}", self.lod_bias);
            }
            ']' => {
                self.lod_bias += 0.5;
                if let Some(i) = &self.image {
                    i.lock().unwrap().set_lod_bias(self.lod_bias);
                }
                println!("[slice] lod_bias = {:+.1}", self.lod_bias);
            }
            'p' | 'P' => {
                let mode = match _ctx.camera.projection_mode {
                    bovista::ProjectionMode::Perspective => bovista::ProjectionMode::Orthographic,
                    bovista::ProjectionMode::Orthographic => bovista::ProjectionMode::Perspective,
                };
                _ctx.camera.set_projection_mode(mode);
                println!("[slice] projection = {:?}", mode);
            }
            'd' | 'D' => {
                self.debug = !self.debug;
                if let Some(i) = &self.image {
                    i.lock().unwrap().set_debug_mode(self.debug);
                }
                println!("[slice] debug = {}", self.debug);
            }
            'r' | 'R' => {
                self.slice_angle_x = 0.0;
                self.slice_angle_y = 0.0;
                self.slice_offset = 0.0;
                self.window_center = (self.args.contrast_min + self.args.contrast_max) / 2.0;
                self.window_width = (self.args.contrast_max - self.args.contrast_min).max(0.001);
                self.apply_slice_plane();
                self.apply_window_level();
                println!("[slice] reset");
            }
            _ => {}
        }
    }

    fn on_secondary_drag(
        &mut self,
        dx: f32,
        dy: f32,
        shift: bool,
        btn: SecondaryButton,
        _ctx: &mut FrameCtx,
    ) {
        match btn {
            SecondaryButton::Right => {
                // Rotate slice (Shift = X axis, else Y axis) + horizontal offset.
                if shift {
                    self.slice_angle_x += dy * 0.01;
                } else {
                    self.slice_angle_y += dy * 0.01;
                }
                self.slice_offset += dx * self.world_max * 0.005;
                self.apply_slice_plane();
            }
            SecondaryButton::Middle => {
                // Window/level: horizontal = level (center), vertical = window.
                self.window_center += dx * 0.002;
                self.window_width = (self.window_width + dy * 0.002).max(0.001);
                self.apply_window_level();
            }
        }
    }

    fn print_stats(&mut self) {
        let Some(image) = &self.image else { return };
        let img = image.lock().unwrap();
        let (loaded, visible) = img.get_stats();
        let s = img.stats();
        println!(
            "[slice] tiles {}/{} | installed {} dropped {} atlas-writes {} | pending {}",
            loaded, visible, s.tiles_installed, s.tiles_dropped, s.atlas_writes, s.pending_len,
        );
    }

    fn controls_help(&self) -> String {
        "\
[slice renderer] controls:
  Left-drag    orbit (pan in ortho)      Scroll      zoom
  Right-drag   rotate slice (+Shift=X) + offset
  Middle-drag  window / level            [ / ]       LOD bias
  P            perspective/ortho         D           debug tint
  R            reset"
            .to_string()
    }
}

fn main() {
    env_logger::init();
    let args = cli::CliArgs::parse();
    loader::cap_rayon_pool();

    let zarr = args.zarr_or(DEFAULT_ZARR);
    println!("[slice] OME-Zarr source: {}", zarr);
    let max_inflight = args.resolved_max_inflight();

    let current_t = Arc::new(AtomicU32::new(0));
    let setup = match ome_zarr::open(&zarr, current_t, ome_zarr::noop_hook(), max_inflight) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[slice] failed to open OME-Zarr at {}: {}", zarr, e);
            std::process::exit(1);
        }
    };

    // Slices show the full intensity range by default; the user narrows the
    // window interactively (middle-drag) or via --contrast-min/--contrast-max.
    let (contrast_min, contrast_max) = args.contrast_or(0.0, 1.0);
    let resolved = ResolvedArgs {
        cache_tiles: args.cache_tiles,
        atlas_count: args.atlas_count,
        lod_bias: args.lod_bias,
        contrast_min,
        contrast_max,
    };
    let app = SliceRenderer {
        setup,
        args: resolved,
        image: None,
        world_max: 1.0,
        slice_angle_x: 0.0,
        slice_angle_y: 0.0,
        slice_offset: 0.0,
        window_center: 0.5,
        window_width: 1.0,
        lod_bias: args.lod_bias,
        debug: false,
    };
    app::run(app, "bovista — slice renderer");
}
