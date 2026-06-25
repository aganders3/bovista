//! Volume renderer — native interactive Rust example.
//!
//! Streams an OME-Zarr multiscale pyramid and renders it with one of bovista's
//! volume render modes (direct / MIP / minIP / average / isosurface) via the
//! virtual-texture volume visuals. Tiles are pulled from S3/HTTP (or a local
//! filesystem path) on a background worker pool.
//!
//! Usage:
//!   cargo run --release --example volume_renderer
//!   cargo run --release --example volume_renderer -- --zarr <path-or-url> --mode mip
//!
//! Controls:
//! ```text
//!   Left-drag    orbit camera          Scroll       zoom
//!   M            cycle render mode (rebuilds the atlas — brief reload)
//!   [ / ]        LOD bias coarser / finer
//!   - / =        density (Direct mode)
//!   , / .        step size (finer / coarser)
//!   F / G        contrast floor down / up
//!   C / V        contrast ceiling down / up
//!   A            auto contrast (reset to [0, 1])
//!   9 / 0        attenuation (MIP) / iso threshold (Iso)
//!   D            toggle debug tint
//! ```
//!
//! Flags: --zarr, --mode, --cache-tiles, --atlas-count, --max-inflight,
//!        --lod-bias, --contrast-min, --contrast-max, --density-mult,
//!        --attenuation, --iso-threshold, --step.
//!
//! Note: the `zarrs_http` HTTP store pulls in openssl; see the example README
//! for the build dependency.

use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use bovista::visuals::virtual_texture::{PendingChunks, PrepareStats, Wanted};
use bovista::visuals::{
    AverageVolume, DirectVolume, IsosurfaceVolume, LodLevelConfig, MinipVolume, MipVolume,
};
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

use app::{BuildCtx, ExampleApp, FrameCtx};

const DEFAULT_ZARR: &str =
    "https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/beechnut.ome.zarr";

// ── Mode dispatch ───────────────────────────────────────────────────────────

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
enum VolumeMode {
    Direct,
    Mip,
    Minip,
    Average,
    Iso,
}

impl VolumeMode {
    fn parse(s: &str) -> Option<Self> {
        match s {
            "direct" => Some(Self::Direct),
            "mip" => Some(Self::Mip),
            "minip" => Some(Self::Minip),
            "average" => Some(Self::Average),
            "iso" | "isosurface" => Some(Self::Iso),
            _ => None,
        }
    }
    fn next(self) -> Self {
        match self {
            Self::Direct => Self::Mip,
            Self::Mip => Self::Minip,
            Self::Minip => Self::Average,
            Self::Average => Self::Iso,
            Self::Iso => Self::Direct,
        }
    }
}

/// Wraps any of the five volume visual types behind a single Visual impl. The
/// common bovista methods dispatch through `volume_dispatch!`; the few
/// mode-specific ones (density on Direct, attenuation on MIP, iso on Iso) are
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
            Volume::Direct($v) => $body,
            Volume::Mip($v) => $body,
            Volume::Minip($v) => $body,
            Volume::Average($v) => $body,
            Volume::Iso($v) => $body,
        }
    };
}

impl Volume {
    #[allow(clippy::too_many_arguments)]
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
            VolumeMode::Direct => Volume::Direct(DirectVolume::new(
                device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count,
            )),
            VolumeMode::Mip => Volume::Mip(MipVolume::new(
                device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count,
            )),
            VolumeMode::Minip => Volume::Minip(MinipVolume::new(
                device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count,
            )),
            VolumeMode::Average => Volume::Average(AverageVolume::new(
                device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count,
            )),
            VolumeMode::Iso => Volume::Iso(IsosurfaceVolume::new(
                device, queue, surface_format, camera_bgl, lod_levels, cache_capacity, atlas_count,
            )),
        }
    }

    fn set_contrast_limits(&mut self, min: f32, max: f32) {
        volume_dispatch!(self, v => v.set_contrast_limits(min, max));
    }
    fn set_lod_bias(&mut self, bias: f32) {
        volume_dispatch!(self, v => v.set_lod_bias(bias));
    }
    fn set_relative_step_size(&mut self, step: f32) {
        volume_dispatch!(self, v => v.set_relative_step_size(step));
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
    fn get_stats(&self) -> (usize, usize) {
        volume_dispatch!(self, v => v.get_stats())
    }

    // Mode-specific: only the applicable variant does anything.
    fn try_set_density_scale(&mut self, scale: f32) {
        if let Volume::Direct(v) = self {
            v.set_density_scale(scale);
        }
    }
    fn try_set_attenuation(&mut self, attenuation: f32) {
        if let Volume::Mip(v) = self {
            v.set_attenuation(attenuation);
        }
    }
    fn try_set_iso_threshold(&mut self, threshold: f32) {
        if let Volume::Iso(v) = self {
            v.set_iso_threshold(threshold);
        }
    }
    fn try_set_debug_mode(&mut self, enabled: bool) {
        if let Volume::Direct(v) = self {
            v.set_debug_mode(enabled);
        }
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

// ── App ──────────────────────────────────────────────────────────────────────

struct VolumeRenderer {
    setup: ome_zarr::SceneSetup,
    args: ResolvedArgs,

    volume: Option<Arc<Mutex<Volume>>>,
    base_density: f32,

    mode: VolumeMode,
    lod_bias: f32,
    contrast_min: f32,
    contrast_max: f32,
    density_mult: f32,
    step: f32,
    attenuation: f32,
    iso_threshold: f32,
    debug: bool,
}

struct ResolvedArgs {
    cache: usize,
    atlas_count: usize,
}

impl VolumeRenderer {
    /// Apply all current settings to the live volume.
    fn apply_all(&mut self) {
        let Some(vol) = &self.volume else { return };
        let mut v = vol.lock().unwrap();
        v.set_contrast_limits(self.contrast_min, self.contrast_max);
        v.set_lod_bias(self.lod_bias);
        v.set_relative_step_size(self.step);
        v.try_set_density_scale(self.base_density * self.density_mult);
        v.try_set_attenuation(self.attenuation);
        v.try_set_iso_threshold(self.iso_threshold);
        v.try_set_debug_mode(self.debug);
    }

    /// Construct a Volume in the current mode, wire the loader slots, and apply
    /// settings. Used by both `build` and the `M` mode-cycle rebuild.
    fn make_volume(&mut self, ctx_renderer: &bovista::Renderer) -> Arc<Mutex<Volume>> {
        let r = ctx_renderer;
        let mut volume = Volume::new(
            self.mode,
            r.device(),
            r.queue(),
            r.surface_format(),
            r.camera_bind_group_layout(),
            self.setup.lods.clone(),
            self.args.cache,
            self.args.atlas_count,
        );
        // Re-wire the loader to this volume's handles.
        *self.setup.pending_slot.lock().unwrap() = volume.pending_chunks();
        *self.setup.wanted_slot.lock().unwrap() = Some(volume.wanted_handle());

        volume.set_contrast_limits(self.contrast_min, self.contrast_max);
        volume.set_lod_bias(self.lod_bias);
        volume.set_relative_step_size(self.step);
        volume.try_set_density_scale(self.base_density * self.density_mult);
        volume.try_set_attenuation(self.attenuation);
        volume.try_set_iso_threshold(self.iso_threshold);
        volume.try_set_debug_mode(self.debug);
        Arc::new(Mutex::new(volume))
    }
}

impl ExampleApp for VolumeRenderer {
    fn build(&mut self, ctx: &mut BuildCtx) {
        // Density scales with voxel-to-world ratio so the same opacity works at
        // any voxel size. Matches the Python example's auto-density heuristic.
        let lod0 = &self.setup.lods[0];
        let voxel_min = lod0
            .voxel_size
            .0
            .min(lod0.voxel_size.1)
            .min(lod0.voxel_size.2)
            .max(1e-9);
        self.base_density = 0.5 / voxel_min;

        let volume = self.make_volume(ctx.renderer);
        ctx.scene.add(volume.clone());
        self.volume = Some(volume);

        *ctx.camera = camera_setup::camera_for_extents(
            ctx.width as f32 / ctx.height as f32,
            self.setup.world_extents,
            2.0,
        );
        println!(
            "[volume] mode={:?} base_density={:.2} contrast=[{:.3},{:.3}] lod_bias={:+.1}",
            self.mode, self.base_density, self.contrast_min, self.contrast_max, self.lod_bias,
        );
    }

    fn on_key(&mut self, key: &Key, ctx: &mut FrameCtx) {
        let Some(ch) = app::char_key(key) else { return };
        match ch {
            'm' | 'M' => {
                self.mode = self.mode.next();
                // Rebuild the visual in the new mode and swap it into the scene.
                // The atlas resets, so there's a brief tile reload — acceptable
                // for this light UI (in-place switch needs the backend split).
                let volume = self.make_volume(ctx.renderer);
                ctx.scene.clear();
                ctx.scene.add(volume.clone());
                self.volume = Some(volume);
                println!("[volume] mode → {:?} (atlas reloading)", self.mode);
            }
            '[' => {
                self.lod_bias -= 0.5;
                self.apply_all();
                println!("[volume] lod_bias = {:+.1}", self.lod_bias);
            }
            ']' => {
                self.lod_bias += 0.5;
                self.apply_all();
                println!("[volume] lod_bias = {:+.1}", self.lod_bias);
            }
            '-' | '_' => {
                self.density_mult = (self.density_mult / 1.25).max(0.01);
                self.apply_all();
                println!("[volume] density × {:.2}", self.density_mult);
            }
            '=' | '+' => {
                self.density_mult *= 1.25;
                self.apply_all();
                println!("[volume] density × {:.2}", self.density_mult);
            }
            ',' => {
                self.step = (self.step * 0.8).max(0.05);
                self.apply_all();
                println!("[volume] step = {:.3}", self.step);
            }
            '.' => {
                self.step = (self.step * 1.25).min(8.0);
                self.apply_all();
                println!("[volume] step = {:.3}", self.step);
            }
            'f' | 'F' => {
                self.contrast_min = (self.contrast_min - 0.01).max(0.0);
                self.apply_all();
                println!("[volume] contrast = [{:.3}, {:.3}]", self.contrast_min, self.contrast_max);
            }
            'g' | 'G' => {
                self.contrast_min = (self.contrast_min + 0.01).min(self.contrast_max - 0.001);
                self.apply_all();
                println!("[volume] contrast = [{:.3}, {:.3}]", self.contrast_min, self.contrast_max);
            }
            'c' | 'C' => {
                self.contrast_max = (self.contrast_max - 0.01).max(self.contrast_min + 0.001);
                self.apply_all();
                println!("[volume] contrast = [{:.3}, {:.3}]", self.contrast_min, self.contrast_max);
            }
            'v' | 'V' => {
                self.contrast_max = (self.contrast_max + 0.01).min(1.0);
                self.apply_all();
                println!("[volume] contrast = [{:.3}, {:.3}]", self.contrast_min, self.contrast_max);
            }
            'a' | 'A' => {
                self.contrast_min = 0.0;
                self.contrast_max = 1.0;
                self.apply_all();
                println!("[volume] contrast auto → [0, 1]");
            }
            '9' => {
                if self.mode == VolumeMode::Mip {
                    self.attenuation = (self.attenuation + 0.05).min(10.0);
                    self.apply_all();
                    println!("[volume] attenuation = {:.2}", self.attenuation);
                } else if self.mode == VolumeMode::Iso {
                    self.iso_threshold = (self.iso_threshold - 0.02).max(0.0);
                    self.apply_all();
                    println!("[volume] iso = {:.2}", self.iso_threshold);
                }
            }
            '0' => {
                if self.mode == VolumeMode::Mip {
                    self.attenuation = (self.attenuation - 0.05).max(0.0);
                    self.apply_all();
                    println!("[volume] attenuation = {:.2}", self.attenuation);
                } else if self.mode == VolumeMode::Iso {
                    self.iso_threshold = (self.iso_threshold + 0.02).min(1.0);
                    self.apply_all();
                    println!("[volume] iso = {:.2}", self.iso_threshold);
                }
            }
            'd' | 'D' => {
                self.debug = !self.debug;
                self.apply_all();
                println!("[volume] debug = {} (Direct mode only)", self.debug);
            }
            _ => {}
        }
    }

    fn print_stats(&mut self) {
        let Some(vol) = &self.volume else { return };
        let v = vol.lock().unwrap();
        let (loaded, visible) = v.get_stats();
        let s = v.stats();
        println!(
            "[volume] mode={:?} tiles {}/{} | installed {} dropped {} atlas-writes {} | pending {}",
            self.mode, loaded, visible, s.tiles_installed, s.tiles_dropped, s.atlas_writes, s.pending_len,
        );
    }

    fn controls_help(&self) -> String {
        "\
[volume renderer] controls:
  Left-drag  orbit      Scroll  zoom        M   cycle mode
  [ / ]      LOD bias   - / =   density     , / .  step size
  F/G floor  C/V ceil   A  auto contrast
  9 / 0      attenuation (MIP) / iso (Iso)  D   debug"
            .to_string()
    }
}

fn main() {
    env_logger::init();
    let args = cli::CliArgs::parse();
    loader::cap_rayon_pool();

    let zarr = args.zarr_or(DEFAULT_ZARR);
    let mode = args
        .mode
        .as_deref()
        .and_then(VolumeMode::parse)
        .unwrap_or(VolumeMode::Direct);
    println!("[volume] OME-Zarr source: {} (mode {:?})", zarr, mode);
    let max_inflight = args.resolved_max_inflight();

    let current_t = Arc::new(AtomicU32::new(0));
    let setup = match ome_zarr::open(&zarr, current_t, ome_zarr::noop_hook(), max_inflight) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("[volume] failed to open OME-Zarr at {}: {}", zarr, e);
            std::process::exit(1);
        }
    };

    let cache = args.cache_tiles.unwrap_or(setup.cache_capacity) as usize;
    let resolved = ResolvedArgs {
        cache,
        atlas_count: args.atlas_count,
    };
    let renderer = VolumeRenderer {
        setup,
        args: resolved,
        volume: None,
        base_density: 1.0,
        mode,
        lod_bias: args.lod_bias,
        contrast_min: args.contrast_min,
        contrast_max: args.contrast_max,
        density_mult: args.density_mult,
        step: args.step.unwrap_or(1.0),
        attenuation: args.attenuation,
        iso_threshold: args.iso_threshold,
        debug: false,
    };
    app::run(renderer, "bovista — volume renderer");
}
