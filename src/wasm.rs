//! WebAssembly bindings for Bovista
//!
//! This module provides JavaScript bindings using wasm-bindgen.

use wasm_bindgen::prelude::*;
use std::rc::Rc;
use std::cell::RefCell;
use js_sys::{Uint8Array, Function};

use crate::{
    Camera, LinesVisual, PointsVisual, Renderer, Scene, TiledImageVisual, Visual,
};
use crate::visuals::tiled_image::{ChunkRequest, ChunkData};

type VisualRef = Rc<RefCell<dyn Visual>>;

/// JavaScript viewer for Bovista
#[wasm_bindgen]
pub struct JsViewer {
    renderer: Option<Renderer>,
    camera: Camera,
    scene: Scene,
    // Note: No surface/window - managed externally by app
}

#[wasm_bindgen]
impl JsViewer {
    #[wasm_bindgen(constructor)]
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            renderer: None,
            camera: Camera::new(width as f32 / height as f32),
            scene: Scene::new(),
        }
    }

    /// Initialize the renderer (call this before adding visuals)
    pub async fn initialize(&mut self, device: wgpu::Device, queue: wgpu::Queue, format: wgpu::TextureFormat) -> Result<(), JsValue> {
        let renderer = Renderer::new(device, queue, format).await;
        self.renderer = Some(renderer);
        Ok(())
    }

    /// Add a visual to the scene
    pub fn add(&mut self, visual: JsVisual) -> usize {
        self.scene.add(visual.inner)
    }

    /// Set camera position
    pub fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {
        self.camera.position = glam::Vec3::new(x, y, z);
    }

    /// Set camera target
    pub fn set_camera_target(&mut self, x: f32, y: f32, z: f32) {
        self.camera.target = glam::Vec3::new(x, y, z);
    }

    /// Orbit camera
    pub fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.orbit(delta_x, delta_y);
    }

    /// Zoom camera
    pub fn zoom_camera(&mut self, delta: f32) {
        self.camera.zoom(delta);
    }

    /// Get visual count
    pub fn visual_count(&self) -> usize {
        self.scene.len()
    }
}

/// JavaScript wrapper for visuals
#[wasm_bindgen]
pub struct JsVisual {
    inner: VisualRef,
}

#[wasm_bindgen]
impl JsVisual {
    /// Create a test cube of points
    #[wasm_bindgen(js_name = testCube)]
    pub fn test_cube(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
        size: u32,
    ) -> Self {
        let visual = PointsVisual::test_cube(device, format, layout, size);
        Self {
            inner: Rc::new(RefCell::new(visual)),
        }
    }

    /// Create axis helper lines
    #[wasm_bindgen(js_name = axisHelper)]
    pub fn axis_helper(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
        length: f32,
    ) -> Self {
        let visual = LinesVisual::axis_helper(device, format, layout, length);
        Self {
            inner: Rc::new(RefCell::new(visual)),
        }
    }
}

/// JavaScript wrapper for TiledImageVisual
#[wasm_bindgen]
pub struct JsTiledImageVisual {
    inner: VisualRef,
}

#[wasm_bindgen]
impl JsTiledImageVisual {
    /// Create a tiled image visual with a JavaScript chunk loader callback
    ///
    /// # Arguments
    ///
    /// * `device` - GPU device
    /// * `format` - Surface format
    /// * `layout` - Camera bind group layout
    /// * `volume_size` - Array [depth, height, width]
    /// * `chunk_size` - Array [cz, cy, cx]
    /// * `loader` - JavaScript function(z, y, x) => Uint8Array | null
    /// * `max_chunks` - Maximum number of loaded chunks
    #[wasm_bindgen(js_name = fromLoader)]
    pub fn from_loader(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
        volume_size: Vec<u32>,
        chunk_size: Vec<u32>,
        loader: Function,
        max_chunks: usize,
    ) -> Result<JsTiledImageVisual, JsValue> {
        if volume_size.len() != 3 || chunk_size.len() != 3 {
            return Err(JsValue::from_str("volume_size and chunk_size must be [z, y, x]"));
        }

        let volume_tuple = (volume_size[0], volume_size[1], volume_size[2]);
        let chunk_tuple = (chunk_size[0], chunk_size[1], chunk_size[2]);

        // Create Rust closure that calls JavaScript loader
        let loader_fn = Rc::new(move |request: ChunkRequest| -> Option<ChunkData> {
            // Call JS function with (z, y, x)
            let this = JsValue::null();
            let args = js_sys::Array::new();
            args.push(&JsValue::from(request.chunk_z));
            args.push(&JsValue::from(request.chunk_y));
            args.push(&JsValue::from(request.chunk_x));

            match loader.apply(&this, &args) {
                Ok(result) => {
                    if result.is_null() || result.is_undefined() {
                        return None;
                    }

                    // Extract Uint8Array
                    if let Ok(uint8_array) = result.dyn_into::<Uint8Array>() {
                        let data = uint8_array.to_vec();

                        // TODO: Get actual dimensions from JS
                        // For now, assume full chunk size
                        Some(ChunkData {
                            data,
                            width: chunk_tuple.2,
                            height: chunk_tuple.1,
                            depth: chunk_tuple.0,
                        })
                    } else {
                        None
                    }
                }
                Err(_) => None,
            }
        });

        let visual = TiledImageVisual::new(
            volume_tuple,
            chunk_tuple,
            loader_fn,
            max_chunks,
        );

        Ok(Self {
            inner: Rc::new(RefCell::new(visual)),
        })
    }

    /// Convert to JsVisual so it can be added to scene
    #[wasm_bindgen(js_name = asVisual)]
    pub fn as_visual(&self) -> JsVisual {
        JsVisual {
            inner: self.inner.clone(),
        }
    }

    /// Set slice plane
    #[wasm_bindgen(js_name = setSlicePlane)]
    pub fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) {
        if let Ok(mut visual) = self.inner.try_borrow_mut() {
            if let Some(tiled) = visual.as_any_mut().downcast_mut::<TiledImageVisual>() {
                let plane = crate::visuals::image::SlicePlane::new([px, py, pz], [nx, ny, nz]);
                tiled.set_slice_plane(plane);
            }
        }
    }

    /// Set contrast limits
    #[wasm_bindgen(js_name = setContrast)]
    pub fn set_contrast(&self, min: f32, max: f32) {
        if let Ok(mut visual) = self.inner.try_borrow_mut() {
            if let Some(tiled) = visual.as_any_mut().downcast_mut::<TiledImageVisual>() {
                tiled.set_contrast_limits(min, max);
            }
        }
    }

    /// Get statistics (returns [loaded, visible])
    #[wasm_bindgen(js_name = getStats)]
    pub fn get_stats(&self) -> Vec<usize> {
        if let Ok(visual) = self.inner.try_borrow() {
            if let Some(tiled) = visual.as_any().downcast_ref::<TiledImageVisual>() {
                return vec![tiled.loaded_chunk_count(), tiled.visible_chunk_count()];
            }
        }
        vec![0, 0]
    }

    /// Prepare chunks (load visible, evict old)
    #[wasm_bindgen(js_name = prepareChunks)]
    pub fn prepare_chunks(
        &self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        format: wgpu::TextureFormat,
        layout: &wgpu::BindGroupLayout,
    ) {
        if let Ok(mut visual) = self.inner.try_borrow_mut() {
            if let Some(tiled) = visual.as_any_mut().downcast_mut::<TiledImageVisual>() {
                tiled.prepare_chunks(device, queue, format, layout);
            }
        }
    }

    /// Set debug mode
    #[wasm_bindgen(js_name = setDebugMode)]
    pub fn set_debug_mode(&self, enabled: bool) {
        if let Ok(mut visual) = self.inner.try_borrow_mut() {
            if let Some(tiled) = visual.as_any_mut().downcast_mut::<TiledImageVisual>() {
                tiled.set_debug_mode(enabled);
            }
        }
    }
}
