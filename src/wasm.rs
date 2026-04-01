//! WebAssembly bindings for Bovista
//!
//! This module provides JavaScript bindings using wasm-bindgen.

use wasm_bindgen::prelude::*;

/// Chunk loading status returned by the chunk loader callback
#[wasm_bindgen]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum JsChunkStatus {
    /// The chunk request was accepted and will be loaded asynchronously
    Accepted = 0,
    /// The chunk is already being loaded (request is pending)
    AlreadyPending = 1,
    /// The chunk request was rejected (e.g., at capacity)
    Rejected = 2,
}

/// Camera projection mode
#[wasm_bindgen]
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
use js_sys::{Uint8Array, Function, Array};
use web_sys::console;

use crate::{
    bindings_common::{self, VisualRef},
    Camera, ImageVisual, Renderer, Scene, SlicePlane,
    visuals::image_strategy::{LodLevelConfig, PendingChunks},
    visuals::tile::{TileRequest, TileLoaderFn, ChunkStatus, TileData, TileKey},
};
use bovista_codegen::visual_methods;

/// JavaScript viewer for Bovista
#[wasm_bindgen]
pub struct JsViewer {
    renderer: Renderer,
    camera: Camera,
    scene: Scene,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    depth_texture: wgpu::TextureView,
}

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
            .ok_or("Failed to find adapter")?;

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::downlevel_webgl2_defaults()
                        .using_resolution(adapter.limits()),
                    memory_hints: wgpu::MemoryHints::default(),
                },
                None,
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
            r: 0.1,
            g: 0.1,
            b: 0.1,
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
    pub fn add_image(&mut self, visual: &JsImageVisual) -> usize {
        self.scene.add(visual.get_inner())
    }

    /// Add a tiled image visual to the scene
    #[wasm_bindgen(js_name = addTiledImage)]
    pub fn add_tiled_image(&mut self, visual: &JsTiledImageVisual) -> usize {
        self.scene.add(visual.get_inner())
    }

    /// Set camera position
    #[wasm_bindgen(js_name = setCameraPosition)]
    pub fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {
        self.camera.position = glam::Vec3::new(x, y, z);
    }

    /// Set camera target
    #[wasm_bindgen(js_name = setCameraTarget)]
    pub fn set_camera_target(&mut self, x: f32, y: f32, z: f32) {
        self.camera.target = glam::Vec3::new(x, y, z);
    }

    /// Set camera up vector
    #[wasm_bindgen(js_name = setCameraUp)]
    pub fn set_camera_up(&mut self, x: f32, y: f32, z: f32) {
        self.camera.up = glam::Vec3::new(x, y, z);
    }

    /// Set camera near and far clip planes
    #[wasm_bindgen(js_name = setCameraClipPlanes)]
    pub fn set_camera_clip_planes(&mut self, near: f32, far: f32) {
        self.camera.near = near;
        self.camera.far = far;
    }

    /// Set camera projection mode
    #[wasm_bindgen(js_name = setCameraProjectionMode)]
    pub fn set_camera_projection_mode(&mut self, mode: JsProjectionMode) {
        self.camera.set_projection_mode(mode.into());
    }

    /// Get camera projection mode
    #[wasm_bindgen(js_name = getCameraProjectionMode)]
    pub fn get_camera_projection_mode(&self) -> JsProjectionMode {
        self.camera.projection_mode().into()
    }

    /// Set orthographic camera height
    #[wasm_bindgen(js_name = setCameraOrthoHeight)]
    pub fn set_camera_ortho_height(&mut self, height: f32) {
        self.camera.set_ortho_height(height);
    }

    /// Get orthographic camera height
    #[wasm_bindgen(js_name = getCameraOrthoHeight)]
    pub fn get_camera_ortho_height(&self) -> f32 {
        self.camera.ortho_height()
    }

    /// Orbit camera
    #[wasm_bindgen(js_name = orbitCamera)]
    pub fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.orbit(delta_x, delta_y);
    }

    /// Pan camera (move in view plane)
    #[wasm_bindgen(js_name = panCamera)]
    pub fn pan_camera(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.pan(delta_x, delta_y);
    }

    /// Zoom camera
    #[wasm_bindgen(js_name = zoomCamera)]
    pub fn zoom_camera(&mut self, delta: f32) {
        self.camera.zoom(delta);
    }

    /// Get the current distance from camera position to target
    #[wasm_bindgen(js_name = getCameraDistance)]
    pub fn get_camera_distance(&self) -> f32 {
        (self.camera.position - self.camera.target).length()
    }

    /// Get visual count
    #[wasm_bindgen(js_name = visualCount)]
    pub fn visual_count(&self) -> usize {
        self.scene.len()
    }

    /// Clear all visuals from the scene
    #[wasm_bindgen(js_name = clearScene)]
    pub fn clear_scene(&mut self) {
        self.scene.clear();
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
#[wasm_bindgen]
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
    #[wasm_bindgen(constructor)]
    pub fn new(
        volume_width: u32,
        volume_height: u32,
        volume_depth: u32,
        chunk_width: u32,
        chunk_height: u32,
        chunk_depth: u32,
        voxel_width: f32,
        voxel_height: f32,
        voxel_depth: f32,
        scale_factor: f32,
        translation_x: f32,
        translation_y: f32,
        translation_z: f32,
    ) -> Self {
        Self {
            volume_size: (volume_depth, volume_height, volume_width),
            chunk_size: (chunk_depth, chunk_height, chunk_width),
            voxel_size: (voxel_depth, voxel_height, voxel_width),
            scale_factor,
            translation: (translation_z, translation_y, translation_x),
        }
    }

    /// Get volume size as [depth, height, width]
    #[wasm_bindgen(getter)]
    pub fn volume_size(&self) -> Vec<u32> {
        vec![self.volume_size.0, self.volume_size.1, self.volume_size.2]
    }

    /// Get chunk size as [depth, height, width]
    #[wasm_bindgen(getter)]
    pub fn chunk_size(&self) -> Vec<u32> {
        vec![self.chunk_size.0, self.chunk_size.1, self.chunk_size.2]
    }

    /// Get voxel size as [depth, height, width]
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

/// JavaScript wrapper for simple ImageVisual (in-memory)
#[wasm_bindgen]
pub struct JsImageVisual {
    inner: VisualRef,
}

#[visual_methods(ImageVisual)]
#[wasm_bindgen]
impl JsImageVisual {
    /// Create an ImageVisual from a typed array
    ///
    /// # Arguments
    /// * `viewer` - The JsViewer instance
    /// * `data` - Uint8Array containing image data (depth * height * width)
    /// * `width` - Image width
    /// * `height` - Image height
    /// * `depth` - Image depth
    #[wasm_bindgen(constructor)]
    pub fn new(
        viewer: &JsViewer,
        data: &Uint8Array,
        width: u32,
        height: u32,
        depth: u32,
    ) -> Self {
        let data_vec = data.to_vec();
        let renderer = viewer.renderer();
        let visual = ImageVisual::new_3d(
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            &data_vec,
            width,
            height,
            depth,
        );

        Self {
            inner: Rc::new(RefCell::new(visual)),
        }
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
    ///
    /// # Arguments
    /// * `px`, `py`, `pz` - Plane position
    /// * `nx`, `ny`, `nz` - Plane normal
    #[wasm_bindgen(js_name = setSlicePlane)]
    pub fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<ImageVisual, _, _>(
            &self.inner,
            |img| {
                let plane = SlicePlane::new([px, py, pz], [nx, ny, nz]);
                img.set_slice_plane(plane);
            }
        ).map_err(|e| JsValue::from_str(&e))
    }

    /// Set contrast limits
    ///
    /// # Arguments
    /// * `min` - Minimum value (0.0 to 1.0)
    /// * `max` - Maximum value (0.0 to 1.0)
    #[wasm_bindgen(js_name = setContrast)]
    pub fn set_contrast(&self, min: f32, max: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<ImageVisual, _, _>(
            &self.inner,
            |img| img.set_contrast_limits(min, max)
        ).map_err(|e| JsValue::from_str(&e))
    }

    /// Enable or disable debug visualization
    #[wasm_bindgen(js_name = setDebugMode)]
    pub fn set_debug_mode(&self, enabled: bool) -> Result<(), JsValue> {}

    /// Get the visual reference for adding to viewer
    pub(crate) fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

/// JavaScript wrapper for chunked ImageVisual (multi-LOD with async loading)
///
/// This class provides LOD-specific methods (setLodBias, getStats, setChunkData).
/// Common image methods (setSliceZ, setContrast, etc.) should be inherited from
/// JsImageVisual by setting up the prototype chain in your JavaScript wrapper:
///
/// ```javascript
/// Object.setPrototypeOf(JsTiledImageVisual.prototype, JsImageVisual.prototype);
/// ```
///
/// This will be handled automatically in the NPM package.
#[wasm_bindgen]
pub struct JsTiledImageVisual {
    inner: VisualRef,
    pending_chunks: Option<PendingChunks>,
    /// Keep a reference to the JS function to prevent garbage collection
    #[allow(dead_code)]
    chunk_loader: Function,
}

#[wasm_bindgen]
impl JsTiledImageVisual {
    /// Create a chunked ImageVisual with multi-resolution LOD
    ///
    /// The chunk_loader function will be called with (lod, z, y, x) when chunks are needed.
    /// It should fetch the chunk data asynchronously and then call setChunkData().
    ///
    /// Example JavaScript:
    /// ```javascript
    /// const visual = new JsTiledImageVisual(viewer, levels, 500,
    ///     (lod, z, y, x) => {
    ///         // Fetch chunk asynchronously
    ///         fetchChunk(lod, z, y, x).then(data => {
    ///             visual.setChunkData(lod, z, y, x, data, chunkWidth, chunkHeight, chunkDepth);
    ///         });
    ///     }
    /// );
    /// ```
    ///
    /// # Arguments
    /// * `viewer` - The JsViewer instance
    /// * `levels` - Array of JsLevelMetadata describing each LOD level
    /// * `max_chunks` - Maximum number of chunks to keep in memory
    /// * `chunk_loader` - JavaScript function called when chunks are needed: `(lod, z, y, x) => void`
    #[wasm_bindgen(constructor)]
    pub fn new(
        viewer: &JsViewer,
        levels: Vec<JsLevelMetadata>,
        max_chunks: usize,
        chunk_loader: Function,
    ) -> Self {
        // Convert JsLevelMetadata to LodLevelConfig
        let rust_levels: Vec<LodLevelConfig> = levels
            .iter()
            .map(|js_level| js_level.to_lod_level_config())
            .collect();

        // Clone the chunk_loader for use in the closure
        let chunk_loader_clone = chunk_loader.clone();

        // Create the loader function that will call the JS callback
        let loader_fn: TileLoaderFn = Box::new(move |request: TileRequest| -> ChunkStatus {
            // Extract tile coordinates
            let lod_level = request.lod_level.unwrap_or(0);
            let z = request.z;
            let y = request.y;
            let x = request.x;

            // Call the JavaScript chunk loader function
            // Signature: chunk_loader(lod, z, y, x)
            let this = &JsValue::NULL;
            let args = Array::new();
            args.push(&JsValue::from(lod_level as u32));
            args.push(&JsValue::from(z));
            args.push(&JsValue::from(y));
            args.push(&JsValue::from(x));

            match chunk_loader_clone.apply(this, &args) {
                Ok(result) => {
                    // JavaScript should return JsChunkStatus enum value
                    if let Some(status_num) = result.as_f64() {
                        let status_val = status_num as u32;
                        if status_val == JsChunkStatus::Accepted as u32 {
                            ChunkStatus::Accepted
                        } else if status_val == JsChunkStatus::AlreadyPending as u32 {
                            ChunkStatus::AlreadyPending
                        } else if status_val == JsChunkStatus::Rejected as u32 {
                            ChunkStatus::Rejected
                        } else {
                            console::warn_1(&JsValue::from_str(&format!(
                                "Invalid chunk status returned: {}, treating as Rejected",
                                status_num
                            )));
                            ChunkStatus::Rejected
                        }
                    } else {
                        // Fallback: if no value returned, assume Accepted (backward compat)
                        ChunkStatus::Accepted
                    }
                }
                Err(err) => {
                    console::error_2(
                        &JsValue::from_str("Error calling chunk_loader:"),
                        &err
                    );
                    ChunkStatus::Rejected
                }
            }
        });

        // Get renderer from viewer
        let renderer = viewer.renderer();

        // Create the visual with chunked loading
        let visual = ImageVisual::from_chunked(
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            rust_levels,
            max_chunks,
            loader_fn,
        );

        let pending_chunks = visual.pending_chunks();
        let inner = Rc::new(RefCell::new(visual));

        Self {
            inner,
            pending_chunks,
            chunk_loader,
        }
    }

    /// Provide chunk data for a requested tile
    ///
    /// This is called from JavaScript after the chunk has been loaded.
    ///
    /// # Arguments
    /// * `lod` - LOD level (0 = highest resolution)
    /// * `z` - Z coordinate in chunk grid
    /// * `y` - Y coordinate in chunk grid
    /// * `x` - X coordinate in chunk grid
    /// * `data` - Uint8Array containing chunk data (depth * height * width)
    /// * `width` - Chunk width
    /// * `height` - Chunk height
    /// * `depth` - Chunk depth
    #[wasm_bindgen(js_name = setChunkData)]
    pub fn set_chunk_data(
        &self,
        lod: usize,
        z: u32,
        y: u32,
        x: u32,
        data: &Uint8Array,
        width: u32,
        height: u32,
        depth: u32,
    ) {
        if let Some(ref pending_chunks) = self.pending_chunks {
            let data_vec = data.to_vec();
            let tile_data = TileData {
                data: data_vec,
                width,
                height,
                depth,
            };
            let key = TileKey { lod_level: lod, z, y, x };

            // Insert directly into pending chunks queue
            pending_chunks.lock().unwrap().insert(key, tile_data);
        }
    }

    // Note: Common image methods (setSliceZ, setSliceY, setSliceX, setSlicePlane, setContrast,
    // setDebugMode) are inherited from JsImageVisual via prototype chain.
    // In your NPM package wrapper, set up inheritance:
    //   Object.setPrototypeOf(JsTiledImageVisual.prototype, JsImageVisual.prototype);

    /// Set LOD bias for automatic LOD selection
    ///
    /// Negative values prefer higher resolution, positive prefer lower resolution.
    #[wasm_bindgen(js_name = setLodBias)]
    pub fn set_lod_bias(&self, bias: f32) -> Result<(), JsValue> {
        bindings_common::with_visual_mut::<ImageVisual, _, _>(
            &self.inner,
            |img| img.set_lod_bias(bias)
        ).map_err(|e| JsValue::from_str(&e))
    }

    /// Get statistics (loaded chunks, visible chunks)
    ///
    /// Returns an array [loaded, visible]
    #[wasm_bindgen(js_name = getStats)]
    pub fn get_stats(&self) -> Vec<usize> {
        bindings_common::with_visual_ref::<ImageVisual, _, _>(
            &self.inner,
            |img| {
                let (loaded, visible) = img.get_stats();
                vec![loaded, visible]
            }
        ).unwrap_or_else(|_| vec![0, 0])
    }

    /// Get the visual reference for adding to viewer
    pub(crate) fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

