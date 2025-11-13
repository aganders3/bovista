//! WebAssembly bindings for Bovista
//!
//! This module provides JavaScript bindings using wasm-bindgen.

use wasm_bindgen::prelude::*;
use std::rc::Rc;
use std::cell::RefCell;
use js_sys::{Uint8Array, Function};

use crate::{
    Camera, LinesVisual, PointsVisual, Renderer, Scene, Visual,
};

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

