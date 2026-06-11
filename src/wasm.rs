//! WebAssembly bindings for Bovista
//!
//! This module provides JavaScript bindings using wasm-bindgen.

use wasm_bindgen::prelude::*;

/// Camera projection mode
#[wasm_bindgen(js_name = "ProjectionMode")]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsProjectionMode {
    /// Perspective projection with field of view
    Perspective = 0,
    /// Orthographic projection with fixed size
    Orthographic = 1,
}

impl From<JsProjectionMode> for crate::ProjectionMode {
    fn from(mode: JsProjectionMode) -> Self {
        match mode {
            JsProjectionMode::Perspective => crate::ProjectionMode::Perspective,
            JsProjectionMode::Orthographic => crate::ProjectionMode::Orthographic,
        }
    }
}

impl From<crate::ProjectionMode> for JsProjectionMode {
    fn from(mode: crate::ProjectionMode) -> Self {
        match mode {
            crate::ProjectionMode::Perspective => JsProjectionMode::Perspective,
            crate::ProjectionMode::Orthographic => JsProjectionMode::Orthographic,
        }
    }
}

use std::rc::Rc;
use std::cell::RefCell;
use js_sys::Uint8Array;
use web_sys::console;

use crate::{
    bindings_common::{self, VisualRef},
    Camera, Image, Lines, Points, Renderer, Scene, SlicePlane,
    AverageVolume, DirectVolume, IsosurfaceVolume, MinipVolume, MipVolume,
    visuals::virtual_texture::{LodLevelConfig, PendingChunks},
    visuals::gpu_structs::{TileData, TileKey},
    visuals::points::PointVertex,
    visuals::lines::LineVertex,
};
use bovista_codegen::{camera_methods, visual_methods};

/// JavaScript viewer for Bovista
#[wasm_bindgen(js_name = "Viewer")]
pub struct JsViewer {
    renderer: Renderer,
    camera: Camera,
    scene: Scene,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::TextureView,
}

#[camera_methods]
#[wasm_bindgen]
impl JsViewer {
    /// Create a new viewer from a canvas element ID
    ///
    /// This will initialize WebGPU and set up the render context
    pub async fn new(canvas_id: &str) -> Result<JsViewer, JsValue> {
        // Set up panic hook for better error messages
        console_error_panic_hook::set_once();

        // Get the canvas element
        let window = web_sys::window().ok_or("No window")?;
        let document = window.document().ok_or("No document")?;
        let canvas = document
            .get_element_by_id(canvas_id)
            .ok_or("Canvas not found")?
            .dyn_into::<web_sys::HtmlCanvasElement>()?;

        let width = canvas.width();
        let height = canvas.height();

        console::log_1(&format!("Initializing Bovista viewer: {}x{}", width, height).into());

        // Request WebGPU adapter and device
        let instance = wgpu::Instance::default();

        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas))
            .map_err(|e| JsValue::from_str(&format!("Failed to create surface: {:?}", e)))?;

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .map_err(|_| JsValue::from_str("Failed to find adapter"))?;

        let adapter_limits = adapter.limits();
        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits {
                        // Inherit the adapter's buffer size limit so large atlas textures
                        // and staging buffers don't hit the conservative 256 MB default.
                        max_buffer_size: adapter_limits.max_buffer_size,
                        ..wgpu::Limits::downlevel_webgl2_defaults()
                            .using_resolution(adapter_limits)
                    },
                    memory_hints: wgpu::MemoryHints::default(),
                    trace: wgpu::Trace::Off,
                },
            )
            .await
            .map_err(|e| JsValue::from_str(&format!("Failed to request device: {:?}", e)))?;

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width,
            height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode: surface_caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        surface.configure(&device, &config);

        console::log_1(&format!("WebGPU initialized with format: {:?}", surface_format).into());

        // Create renderer
        let renderer = Renderer::new(device, queue, surface_format).await;
        let camera = Camera::new(width as f32 / height as f32);
        let scene = Scene::new();

        // Create depth texture
        let depth_texture = renderer.create_depth_texture(width, height);

        Ok(Self {
            renderer,
            camera,
            scene,
            surface,
            config,
            depth_texture,
        })
    }

    /// Render a single frame
    #[wasm_bindgen(js_name = renderFrame)]
    pub fn render_frame(&mut self) -> Result<(), JsValue> {
        // Update camera uniforms
        self.renderer.update_camera(&self.camera);

        // Prepare scene (this triggers chunk loading callbacks)
        let camera_info = crate::visual::CameraInfo {
            position: self.camera.position,
            target: self.camera.target,
            fov_y: self.camera.fov_y,
            viewport_width: self.config.width,
            viewport_height: self.config.height,
            frustum: self.camera.frustum_planes(),
            projection_mode: self.camera.projection_mode,
            ortho_height: self.camera.ortho_height,
            view_proj: self.camera.view_projection_matrix(),
        };
        self.scene.prepare(self.renderer.device(), self.renderer.queue(), &camera_info);

        // DEBUG: Log once at frame 60
        static mut FRAME_COUNTER: u32 = 0;
        unsafe {
            FRAME_COUNTER += 1;
            if FRAME_COUNTER == 60 {
                let msg = format!("[Rust] Frame 60: Canvas={}x{}, Visuals={}, Cam=({:.0},{:.0},{:.0})",
                    self.config.width, self.config.height, self.scene.len(),
                    self.camera.position.x, self.camera.position.y, self.camera.position.z);
                console::log_1(&JsValue::from_str(&msg));
            }
        }

        // Get current frame
        let output = self.surface
            .get_current_texture()
            .map_err(|e| JsValue::from_str(&format!("Failed to get surface texture: {:?}", e)))?;

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Render with proper parameters
        let clear_color = wgpu::Color {
            r: 0.05098,  // #0d1117 page background
            g: 0.06667,
            b: 0.09020,
            a: 1.0,
        };
        self.renderer.render(&self.scene, &view, &self.depth_texture, clear_color);

        output.present();

        Ok(())
    }

    /// Get renderer for creating visuals
    fn renderer(&self) -> &Renderer {
        &self.renderer
    }

    /// Add an image visual to the scene
    #[wasm_bindgen(js_name = addImage)]
    pub fn add_image(&mut self, visual: &JsImage) -> usize {
        self.scene.add(visual.get_inner())
    }

    #[wasm_bindgen(js_name = addDirectVolume)]
    pub fn add_direct_volume(&mut self, visual: &JsDirectVolume) -> usize {
        self.scene.add(visual.get_inner())
    }

    #[wasm_bindgen(js_name = addMipVolume)]
    pub fn add_mip_volume(&mut self, visual: &JsMipVolume) -> usize {
        self.scene.add(visual.get_inner())
    }

    #[wasm_bindgen(js_name = addMinipVolume)]
    pub fn add_minip_volume(&mut self, visual: &JsMinipVolume) -> usize {
        self.scene.add(visual.get_inner())
    }

    #[wasm_bindgen(js_name = addAverageVolume)]
    pub fn add_average_volume(&mut self, visual: &JsAverageVolume) -> usize {
        self.scene.add(visual.get_inner())
    }

    #[wasm_bindgen(js_name = addIsosurfaceVolume)]
    pub fn add_isosurface_volume(&mut self, visual: &JsIsosurfaceVolume) -> usize {
        self.scene.add(visual.get_inner())
    }

    #[wasm_bindgen(js_name = setCameraPosition)]
    pub fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {}

    #[wasm_bindgen(js_name = setCameraTarget)]
    pub fn set_camera_target(&mut self, x: f32, y: f32, z: f32) {}

    #[wasm_bindgen(js_name = setCameraUp)]
    pub fn set_camera_up(&mut self, x: f32, y: f32, z: f32) {}

    #[wasm_bindgen(js_name = setCameraClipPlanes)]
    pub fn set_camera_clip_planes(&mut self, near: f32, far: f32) {}

    #[wasm_bindgen(js_name = setCameraProjectionMode)]
    pub fn set_camera_projection_mode(&mut self, mode: JsProjectionMode) {}

    #[wasm_bindgen(js_name = getCameraProjectionMode)]
    pub fn get_camera_projection_mode(&self) -> JsProjectionMode {}

    #[wasm_bindgen(js_name = setCameraOrthoHeight)]
    pub fn set_camera_ortho_height(&mut self, height: f32) {}

    #[wasm_bindgen(js_name = getCameraOrthoHeight)]
    pub fn get_camera_ortho_height(&self) -> f32 {}

    #[wasm_bindgen(js_name = orbitCamera)]
    pub fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {}

    #[wasm_bindgen(js_name = panCamera)]
    pub fn pan_camera(&mut self, delta_x: f32, delta_y: f32) {}

    #[wasm_bindgen(js_name = zoomCamera)]
    pub fn zoom_camera(&mut self, delta: f32) {}

    #[wasm_bindgen(js_name = getCameraDistance)]
    pub fn get_camera_distance(&self) -> f32 {}

    #[wasm_bindgen(js_name = visualCount)]
    pub fn visual_count(&self) -> usize {}

    #[wasm_bindgen(js_name = clearScene)]
    pub fn clear_scene(&mut self) {}

    /// Add a points visual to the scene
    #[wasm_bindgen(js_name = addPoints)]
    pub fn add_points_visual(&mut self, visual: &JsPoints) -> usize {
        self.scene.add(visual.inner.clone())
    }

    /// Add a lines visual to the scene
    #[wasm_bindgen(js_name = addLines)]
    pub fn add_lines_visual(&mut self, visual: &JsLines) -> usize {
        self.scene.add(visual.inner.clone())
    }

    /// Resize the viewer when canvas dimensions change
    #[wasm_bindgen(js_name = resize)]
    pub fn resize(&mut self, width: u32, height: u32) {
        // Update surface configuration
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(self.renderer.device(), &self.config);

        // Recreate depth texture with new dimensions
        self.depth_texture = self.renderer.create_depth_texture(width, height);

        // Update camera aspect ratio
        self.camera.aspect_ratio = width as f32 / height as f32;
    }
}

/// Level metadata for multi-resolution LOD images
///
/// Describes a single LOD level in a chunked image.
#[wasm_bindgen(js_name = "LevelMetadata")]
#[derive(Clone)]
pub struct JsLevelMetadata {
    volume_size: (u32, u32, u32),
    chunk_size: (u32, u32, u32),
    voxel_size: (f32, f32, f32),
    scale_factor: f32,
    translation: (f32, f32, f32),
}

#[wasm_bindgen]
impl JsLevelMetadata {
    /// Construct a level descriptor. Each array is `[z, y, x]` numpy order.
    ///
    /// ```js
    /// new LevelMetadata(
    ///   /* volume_shape */ [1024, 1024, 1024],   // voxel counts along z, y, x
    ///   /* chunk_shape  */ [64, 64, 64],          // tile voxel counts
    ///   /* voxel_size   */ [1.0, 1.0, 1.0],       // world-space units per voxel
    ///   /* scale_factor */ 1.0,                   // relative to LOD 0
    ///   /* translation  */ [0.0, 0.0, 0.0],       // world-space origin offset
    /// );
    /// ```
    #[wasm_bindgen(constructor)]
    pub fn new(
        volume_shape: Vec<u32>,
        chunk_shape: Vec<u32>,
        voxel_size: Vec<f32>,
        scale_factor: f32,
        translation: Vec<f32>,
    ) -> Result<JsLevelMetadata, JsValue> {
        let triple_u32 = |v: &Vec<u32>, name: &str| -> Result<(u32, u32, u32), JsValue> {
            if v.len() != 3 {
                return Err(JsValue::from_str(&format!(
                    "{} must be a 3-element [z, y, x] array, got length {}", name, v.len()
                )));
            }
            Ok((v[0], v[1], v[2]))
        };
        let triple_f32 = |v: &Vec<f32>, name: &str| -> Result<(f32, f32, f32), JsValue> {
            if v.len() != 3 {
                return Err(JsValue::from_str(&format!(
                    "{} must be a 3-element [z, y, x] array, got length {}", name, v.len()
                )));
            }
            Ok((v[0], v[1], v[2]))
        };
        Ok(Self {
            volume_size: triple_u32(&volume_shape, "volume_shape")?,
            chunk_size:  triple_u32(&chunk_shape,  "chunk_shape")?,
            voxel_size:  triple_f32(&voxel_size,   "voxel_size")?,
            scale_factor,
            translation: triple_f32(&translation,  "translation")?,
        })
    }

    /// Volume shape as `[z, y, x]` voxel counts.
    #[wasm_bindgen(getter)]
    pub fn volume_size(&self) -> Vec<u32> {
        vec![self.volume_size.0, self.volume_size.1, self.volume_size.2]
    }

    /// Chunk shape as `[z, y, x]` voxel counts.
    #[wasm_bindgen(getter)]
    pub fn chunk_size(&self) -> Vec<u32> {
        vec![self.chunk_size.0, self.chunk_size.1, self.chunk_size.2]
    }

    /// Voxel size as `[z, y, x]` world-space units per voxel.
    #[wasm_bindgen(getter)]
    pub fn voxel_size(&self) -> Vec<f32> {
        vec![self.voxel_size.0, self.voxel_size.1, self.voxel_size.2]
    }

    /// Get scale factor
    #[wasm_bindgen(getter)]
    pub fn scale_factor(&self) -> f32 {
        self.scale_factor
    }
}

impl JsLevelMetadata {
    fn to_lod_level_config(&self) -> LodLevelConfig {
        LodLevelConfig {
            volume_size: self.volume_size,
            tile_size: self.chunk_size,
            voxel_size: self.voxel_size,
            scale_factor: self.scale_factor,
            translation: self.translation,
        }
    }
}


/// JavaScript wrapper for Image — single-draw-call multiscale rendering.
///
/// Tile data must be provided as uint16 via `setChunkDataU16`.
#[wasm_bindgen(js_name = "Image")]
pub struct JsImage {
    inner: VisualRef,
    pending_chunks: Option<PendingChunks>,
    wanted: crate::visuals::virtual_texture::Wanted,
}

#[visual_methods(Image)]
#[wasm_bindgen]
impl JsImage {
    /// Create an Image.
    ///
    /// Pull-based: bovista publishes the set of tiles it wants each
    /// frame via `wantedKeys()`. The caller (JS) polls this and pushes
    /// data via `setChunkDataU16`. No callback flowing across the FFI
    /// boundary on the hot path.
    #[wasm_bindgen(constructor)]
    pub fn new(
        viewer: &JsViewer,
        levels: Vec<JsLevelMetadata>,
        max_chunks: usize,
        atlas_count: Option<usize>,
    ) -> Self {
        let rust_levels: Vec<LodLevelConfig> =
            levels.iter().map(|l| l.to_lod_level_config()).collect();

        let renderer = viewer.renderer();
        let visual = Image::new(
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            rust_levels,
            max_chunks,
            atlas_count.unwrap_or(1),
        );

        let pending_chunks = visual.pending_chunks();
        let wanted = visual.wanted_handle();
        let inner = Rc::new(RefCell::new(visual));

        Self { inner, pending_chunks, wanted }
    }

    /// Snapshot of the tile keys bovista currently wants. Returned as
    /// a flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
    /// sorted by priority (lower = more urgent). Poll periodically.
    #[wasm_bindgen(js_name = wantedKeys)]
    pub fn wanted_keys(&self) -> js_sys::Uint32Array {
        let flat: Vec<u32> = crate::visuals::virtual_texture::wanted_sorted(&self.wanted)
            .into_iter()
            .flat_map(|(lod, t, z, y, x, p)| [lod as u32, t, z, y, x, p as u32])
            .collect();
        js_sys::Uint32Array::from(flat.as_slice())
    }

    /// Set the slice plane position along Z axis
    #[wasm_bindgen(js_name = setSliceZ)]
    pub fn set_slice_z(&self, z: f32) -> Result<(), JsValue> {}

    /// Set the slice plane position along Y axis
    #[wasm_bindgen(js_name = setSliceY)]
    pub fn set_slice_y(&self, y: f32) -> Result<(), JsValue> {}

    /// Set the slice plane position along X axis
    #[wasm_bindgen(js_name = setSliceX)]
    pub fn set_slice_x(&self, x: f32) -> Result<(), JsValue> {}

    /// Set the slice plane position and normal
    #[wasm_bindgen(js_name = setSlicePlane)]
    pub fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<Image, _, _>(
            &self.inner,
            |img| {
                let plane = SlicePlane::new([px, py, pz], [nx, ny, nz]);
                img.set_slice_plane(plane);
            }
        ).map_err(|e| JsValue::from_str(&e))
    }

    /// Set contrast limits (0.0 to 1.0)
    #[wasm_bindgen(js_name = setContrast)]
    pub fn set_contrast(&self, min: f32, max: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<Image, _, _>(
            &self.inner,
            |img| img.set_contrast_limits(min, max)
        ).map_err(|e| JsValue::from_str(&e))
    }

    /// Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
    /// Pass a zero-length array to reset to grayscale.
    #[wasm_bindgen(js_name = setColormap)]
    pub fn set_colormap(&self, rgba: &Uint8Array) -> Result<(), JsValue> {
        let bytes = rgba.to_vec();
        bindings_common::with_visual_mut::<Image, _, _>(
            &self.inner,
            |img| img.set_colormap(&bytes)
        ).map_err(|e| JsValue::from_str(&e))
    }

    /// Provide uint16 tile data (stored as R16Float).
    #[wasm_bindgen(js_name = setChunkDataU16)]
    pub fn set_chunk_data_u16(
        &self,
        lod: usize,
        t: u32,
        z: u32,
        y: u32,
        x: u32,
        data: &js_sys::Uint16Array,
        z_shape: u32,
        y_shape: u32,
        x_shape: u32,
    ) {
        if let Some(ref pending_chunks) = self.pending_chunks {
            let bytes: Vec<u8> = data
                .to_vec()
                .iter()
                .flat_map(|&v| half::f16::from_f32(v as f32 / u16::MAX as f32).to_le_bytes())
                .collect();
            let tile_data = TileData {
                data: bytes,
                z_shape,
                y_shape,
                x_shape,
                format: wgpu::TextureFormat::R16Float,
            };
            let key = TileKey { lod_level: lod, t, z, y, x };
            pending_chunks.lock().unwrap().insert(key, tile_data);
        }
    }

    /// Set LOD bias (positive = prefer higher resolution / finer LOD, negative = coarser).
    #[wasm_bindgen(js_name = setLodBias)]
    pub fn set_lod_bias(&self, bias: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<Image, _, _>(&self.inner, |img| {
            img.set_lod_bias(bias)
        })
        .map_err(|e| JsValue::from_str(&e))
    }

    /// Enable or disable debug LOD tinting (green=LOD0, red=coarsest).
    #[wasm_bindgen(js_name = setDebugMode)]
    pub fn set_debug_mode(&self, enabled: bool) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<Image, _, _>(&self.inner, |img| {
            img.set_debug_mode(enabled)
        })
        .map_err(|e| JsValue::from_str(&e))
    }

    /// Returns [loaded, visible] tile counts.
    #[wasm_bindgen(js_name = getStats)]
    pub fn get_stats(&self) -> Vec<usize> {
        bindings_common::with_visual_ref::<Image, _, _>(&self.inner, |img| {
            let (loaded, visible) = img.get_stats();
            vec![loaded, visible]
        })
        .unwrap_or_else(|_| vec![0, 0])
    }

    pub(crate) fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

// ── Volume visuals: one wrapper class per render mode ───────────────────────
//
// Pull-based: bovista publishes `wanted` each prepare; JS polls it via
// `wantedKeys()` and pushes tile data via `setChunkDataU16`. No callbacks.

/// Generate the JS wrapper struct + `#[wasm_bindgen]` impl for a volume type.
macro_rules! js_volume_class {
    (
        $wrapper:ident,
        $js_name:literal,
        $rust_ty:ident
        $(, extra: { $($extra:tt)* })?
    ) => {
        #[wasm_bindgen(js_name = $js_name)]
        pub struct $wrapper {
            inner: VisualRef,
            pending_chunks: Option<PendingChunks>,
            wanted: crate::visuals::virtual_texture::Wanted,
        }

        #[wasm_bindgen]
        impl $wrapper {
            #[wasm_bindgen(constructor)]
            pub fn new(
                viewer: &JsViewer,
                levels: Vec<JsLevelMetadata>,
                max_chunks: usize,
                atlas_count: Option<usize>,
            ) -> Self {
                let rust_levels: Vec<LodLevelConfig> =
                    levels.iter().map(|l| l.to_lod_level_config()).collect();
                let renderer = viewer.renderer();
                let visual = $rust_ty::new(
                    renderer.device(),
                    renderer.queue(),
                    renderer.surface_format(),
                    renderer.camera_bind_group_layout(),
                    rust_levels,
                    max_chunks,
                    atlas_count.unwrap_or(1),
                );
                let pending_chunks = visual.pending_chunks();
                let wanted = visual.wanted_handle();
                let inner = Rc::new(RefCell::new(visual));
                Self { inner, pending_chunks, wanted }
            }

            /// Snapshot of the tile keys bovista currently wants. Returned as a
            /// flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
            /// sorted by priority.
            #[wasm_bindgen(js_name = wantedKeys)]
            pub fn wanted_keys(&self) -> js_sys::Uint32Array {
                let flat: Vec<u32> = crate::visuals::virtual_texture::wanted_sorted(&self.wanted)
                    .into_iter()
                    .flat_map(|(lod, t, z, y, x, p)| [lod as u32, t, z, y, x, p as u32])
                    .collect();
                js_sys::Uint32Array::from(flat.as_slice())
            }

            #[wasm_bindgen(js_name = setContrast)]
            pub fn set_contrast(&self, min: f32, max: f32) -> Result<(), JsValue> {
                bindings_common::with_visual_mut::<$rust_ty, _, _>(
                    &self.inner, |v| v.set_contrast_limits(min, max)
                ).map_err(|e| JsValue::from_str(&e))
            }

            /// Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
            /// Pass a zero-length array to reset to grayscale.
            #[wasm_bindgen(js_name = setColormap)]
            pub fn set_colormap(&self, rgba: &Uint8Array) -> Result<(), JsValue> {
                let bytes = rgba.to_vec();
                bindings_common::with_visual_mut::<$rust_ty, _, _>(
                    &self.inner, |v| v.set_colormap(&bytes)
                ).map_err(|e| JsValue::from_str(&e))
            }

            #[wasm_bindgen(js_name = setRelativeStepSize)]
            pub fn set_relative_step_size(&self, step: f32) -> Result<(), JsValue> {
                bindings_common::with_visual_mut::<$rust_ty, _, _>(
                    &self.inner, |v| v.set_relative_step_size(step)
                ).map_err(|e| JsValue::from_str(&e))
            }

            #[wasm_bindgen(js_name = setLodBias)]
            pub fn set_lod_bias(&self, bias: f32) -> Result<(), JsValue> {
                bindings_common::with_visual_mut::<$rust_ty, _, _>(
                    &self.inner, |v| v.set_lod_bias(bias)
                ).map_err(|e| JsValue::from_str(&e))
            }

            /// Provide uint16 tile data (stored as R16Float).
            #[wasm_bindgen(js_name = setChunkDataU16)]
            pub fn set_chunk_data_u16(
                &self,
                lod: usize, t: u32, z: u32, y: u32, x: u32,
                data: &js_sys::Uint16Array,
                z_shape: u32, y_shape: u32, x_shape: u32,
            ) {
                if let Some(ref pending_chunks) = self.pending_chunks {
                    let bytes: Vec<u8> = data
                        .to_vec()
                        .iter()
                        .flat_map(|&v| half::f16::from_f32(v as f32 / u16::MAX as f32).to_le_bytes())
                        .collect();
                    let tile_data = TileData {
                        data: bytes, z_shape, y_shape, x_shape,
                        format: wgpu::TextureFormat::R16Float,
                    };
                    let key = TileKey { lod_level: lod, t, z, y, x };
                    pending_chunks.lock().unwrap().insert(key, tile_data);
                }
            }

            /// Returns [loaded, visible] tile counts.
            #[wasm_bindgen(js_name = getStats)]
            pub fn get_stats(&self) -> Vec<usize> {
                bindings_common::with_visual_ref::<$rust_ty, _, _>(&self.inner, |v| {
                    let (loaded, visible) = v.get_stats();
                    vec![loaded, visible]
                })
                .unwrap_or_else(|_| vec![0, 0])
            }

            $($($extra)*)?
        }

        impl $wrapper {
            pub(crate) fn get_inner(&self) -> VisualRef {
                self.inner.clone()
            }
        }
    };
}

js_volume_class!(JsDirectVolume, "DirectVolume", DirectVolume, extra: {
    #[wasm_bindgen(js_name = setDensityScale)]
    pub fn set_density_scale(&self, scale: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<DirectVolume, _, _>(
            &self.inner, |v| v.set_density_scale(scale)
        ).map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen(js_name = setEarlyExitAlpha)]
    pub fn set_early_exit_alpha(&self, alpha: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<DirectVolume, _, _>(
            &self.inner, |v| v.set_early_exit_alpha(alpha)
        ).map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen(js_name = setDebugMode)]
    pub fn set_debug_mode(&self, enabled: bool) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<DirectVolume, _, _>(
            &self.inner, |v| v.set_debug_mode(enabled)
        ).map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen(js_name = setAtlasDebugMode)]
    pub fn set_atlas_debug_mode(&self, enabled: bool) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<DirectVolume, _, _>(
            &self.inner, |v| v.set_atlas_debug_mode(enabled)
        ).map_err(|e| JsValue::from_str(&e))
    }

    #[wasm_bindgen(js_name = setStepDebugMode)]
    pub fn set_step_debug_mode(&self, enabled: bool) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<DirectVolume, _, _>(
            &self.inner, |v| v.set_step_debug_mode(enabled)
        ).map_err(|e| JsValue::from_str(&e))
    }
});

js_volume_class!(JsMipVolume, "MipVolume", MipVolume, extra: {
    #[wasm_bindgen(js_name = setAttenuation)]
    pub fn set_attenuation(&self, attenuation: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<MipVolume, _, _>(
            &self.inner, |v| v.set_attenuation(attenuation)
        ).map_err(|e| JsValue::from_str(&e))
    }
});

js_volume_class!(JsMinipVolume, "MinipVolume", MinipVolume);
js_volume_class!(JsAverageVolume, "AverageVolume", AverageVolume);

js_volume_class!(JsIsosurfaceVolume, "IsosurfaceVolume", IsosurfaceVolume, extra: {
    #[wasm_bindgen(js_name = setIsoThreshold)]
    pub fn set_iso_threshold(&self, threshold: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<IsosurfaceVolume, _, _>(
            &self.inner, |v| v.set_iso_threshold(threshold)
        ).map_err(|e| JsValue::from_str(&e))
    }
});

/// JavaScript wrapper for Points — colored point cloud.
#[wasm_bindgen(js_name = "Points")]
pub struct JsPoints {
    inner: VisualRef,
}

#[wasm_bindgen]
impl JsPoints {
    /// Create a point cloud from flat Float32Arrays.
    ///
    /// `positions` is a flat array of XYZ triples; `colors` is a flat array of RGB triples
    /// (values 0–1). Both must have length 3 × n_points.
    #[wasm_bindgen(constructor)]
    pub fn new(viewer: &JsViewer, positions: &js_sys::Float32Array, colors: &js_sys::Float32Array) -> Result<JsPoints, JsValue> {
        let pos = positions.to_vec();
        let col = colors.to_vec();
        if pos.len() != col.len() || pos.len() % 3 != 0 {
            return Err(JsValue::from_str("positions and colors must be flat XYZ/RGB arrays of equal length"));
        }
        let n = pos.len() / 3;
        let vertices: Vec<PointVertex> = (0..n)
            .map(|i| PointVertex {
                position: [pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]],
                color:    [col[i * 3], col[i * 3 + 1], col[i * 3 + 2]],
            })
            .collect();
        let renderer = viewer.renderer();
        let visual = Points::new(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            vertices,
        );
        Ok(Self { inner: Rc::new(RefCell::new(visual)) })
    }

    /// Create a test cube of points.
    #[wasm_bindgen(js_name = testCube)]
    pub fn test_cube(viewer: &JsViewer, size: u32) -> JsPoints {
        let renderer = viewer.renderer();
        let visual = Points::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            size,
        );
        Self { inner: Rc::new(RefCell::new(visual)) }
    }
}

/// JavaScript wrapper for Lines — line segments and wireframes.
#[wasm_bindgen(js_name = "Lines")]
pub struct JsLines {
    inner: VisualRef,
}

#[wasm_bindgen]
impl JsLines {
    /// Create a lines visual from flat Float32Arrays.
    ///
    /// Each consecutive pair of vertices defines one line segment.
    /// `positions` and `colors` are flat arrays of length 3 × n_vertices.
    #[wasm_bindgen(constructor)]
    pub fn new(viewer: &JsViewer, positions: &js_sys::Float32Array, colors: &js_sys::Float32Array) -> Result<JsLines, JsValue> {
        let pos = positions.to_vec();
        let col = colors.to_vec();
        if pos.len() != col.len() || pos.len() % 3 != 0 {
            return Err(JsValue::from_str("positions and colors must be flat XYZ/RGB arrays of equal length"));
        }
        let n = pos.len() / 3;
        let vertices: Vec<LineVertex> = (0..n)
            .map(|i| LineVertex {
                position: [pos[i * 3], pos[i * 3 + 1], pos[i * 3 + 2]],
                color:    [col[i * 3], col[i * 3 + 1], col[i * 3 + 2]],
            })
            .collect();
        let renderer = viewer.renderer();
        let visual = Lines::new(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            vertices,
        );
        Ok(Self { inner: Rc::new(RefCell::new(visual)) })
    }

    /// Create a 3-axis helper (X=red, Y=green, Z=blue).
    #[wasm_bindgen(js_name = axisHelper)]
    pub fn axis_helper(viewer: &JsViewer, length: f32) -> JsLines {
        let renderer = viewer.renderer();
        let visual = Lines::axis_helper(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            length,
        );
        Self { inner: Rc::new(RefCell::new(visual)) }
    }

    /// Create a wireframe unit cube.
    #[wasm_bindgen(js_name = testCube)]
    pub fn test_cube(viewer: &JsViewer) -> JsLines {
        let renderer = viewer.renderer();
        let visual = Lines::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
        );
        Self { inner: Rc::new(RefCell::new(visual)) }
    }
}
