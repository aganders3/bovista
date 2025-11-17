//! Scene graph for managing and rendering collections of visual objects.
//!
//! The [`Scene`] is a container that manages multiple visual objects and coordinates
//! their preparation and rendering. It handles platform-specific threading models
//! (`Arc<Mutex>` on native, `Rc<RefCell>` on WASM).

use crate::visual::Visual;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

/// Platform-appropriate reference type for visual objects (thread-safe on native)
#[cfg(not(target_arch = "wasm32"))]
type VisualRef = Arc<Mutex<dyn Visual>>;

/// Platform-appropriate reference type for visual objects (single-threaded on WASM)
#[cfg(target_arch = "wasm32")]
type VisualRef = Rc<RefCell<dyn Visual>>;

/// A scene contains a collection of visual objects to be rendered.
///
/// The scene manages the lifetime of all visual objects and coordinates their
/// preparation (updating GPU buffers) and rendering phases. Visuals are stored
/// in insertion order and rendered in that same order.
///
/// # Platform Differences
///
/// On native platforms, visuals are stored in `Arc<Mutex<>>` for thread-safety.
/// On WASM, where threads aren't available, `Rc<RefCell<>>` is used instead.
///
/// # Example
///
/// ```rust,no_run
/// use bovista::{Scene, PointsVisual};
/// use std::sync::{Arc, Mutex};
///
/// let mut scene = Scene::new();
///
/// // Add visuals to the scene
/// let points = PointsVisual::test_cube(&device, &format, &layout, 10);
/// let index = scene.add(Arc::new(Mutex::new(points)));
///
/// // Prepare and render
/// scene.prepare(&device, &queue);
/// scene.render(&mut render_pass);
///
/// // Remove a visual
/// scene.remove(index);
/// ```
pub struct Scene {
    visuals: Vec<VisualRef>,
}

impl Scene {
    pub fn new() -> Self {
        Self {
            visuals: Vec::new(),
        }
    }

    /// Returns an index that can be used to remove the visual later
    pub fn add(&mut self, visual: VisualRef) -> usize {
        let index = self.visuals.len();
        self.visuals.push(visual);
        index
    }

    pub fn remove(&mut self, index: usize) -> Option<VisualRef> {
        if index < self.visuals.len() {
            Some(self.visuals.remove(index))
        } else {
            None
        }
    }

    /// Get the number of visuals in the scene
    pub fn len(&self) -> usize {
        self.visuals.len()
    }

    /// Check if the scene is empty
    pub fn is_empty(&self) -> bool {
        self.visuals.is_empty()
    }

    /// Clear all visuals from the scene
    pub fn clear(&mut self) {
        self.visuals.clear();
    }

    /// Get an iterator over all visuals in the scene
    pub fn iter(&self) -> impl Iterator<Item = &VisualRef> {
        self.visuals.iter()
    }

    /// Get a mutable iterator over all visuals in the scene
    pub fn iter_mut(&mut self) -> impl Iterator<Item = &mut VisualRef> {
        self.visuals.iter_mut()
    }

    /// Prepare all visuals in the scene for rendering
    #[cfg(not(target_arch = "wasm32"))]
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &crate::visual::CameraInfo) {
        for visual in &self.visuals {
            if let Ok(mut v) = visual.lock() {
                if v.is_visible() {
                    v.prepare(device, queue, camera_info);
                }
            }
        }
    }

    /// Prepare all visuals in the scene for rendering (WASM version)
    #[cfg(target_arch = "wasm32")]
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue, camera_info: &crate::visual::CameraInfo) {
        for visual in &self.visuals {
            if let Ok(mut v) = visual.try_borrow_mut() {
                if v.is_visible() {
                    v.prepare(device, queue, camera_info);
                }
            }
        }
    }

    /// Render all visible visuals in the scene
    #[cfg(not(target_arch = "wasm32"))]
    pub fn render(&self, render_pass: &mut wgpu::RenderPass) {
        for visual in &self.visuals {
            if let Ok(v) = visual.lock() {
                if v.is_visible() {
                    v.render(render_pass);
                }
            }
        }
    }

    /// Render all visible visuals in the scene (WASM version)
    #[cfg(target_arch = "wasm32")]
    pub fn render(&self, render_pass: &mut wgpu::RenderPass) {
        use web_sys::console;
        use wasm_bindgen::JsValue;

        static mut RENDER_COUNT: u32 = 0;
        static mut LAST_VISUAL_COUNT: usize = 0;

        unsafe {
            RENDER_COUNT += 1;
            let visual_count = self.visuals.len();

            // Log when visual count changes OR on first few frames after change
            if visual_count != LAST_VISUAL_COUNT || (visual_count > 0 && RENDER_COUNT < LAST_VISUAL_COUNT as u32 + 5) {
                if visual_count != LAST_VISUAL_COUNT {
                    LAST_VISUAL_COUNT = visual_count;
                    console::log_1(&JsValue::from_str(&format!("[Scene::render] Visual count changed: {} visuals", visual_count)));
                }

                for (i, visual) in self.visuals.iter().enumerate() {
                    if let Ok(v) = visual.try_borrow() {
                        let msg = format!("  Visual {}: visible={}, name={}",
                            i, v.is_visible(), v.name());
                        console::log_1(&JsValue::from_str(&msg));
                    }
                }
            }
        }

        for visual in &self.visuals {
            if let Ok(v) = visual.try_borrow() {
                if v.is_visible() {
                    v.render(render_pass);
                }
            }
        }
    }
}

impl Default for Scene {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::visual::{Transform, Visual};
    use wgpu::RenderPass;

    struct DummyVisual {
        visible: bool,
        transform: Transform,
    }

    impl Visual for DummyVisual {
        fn prepare(&mut self, _device: &wgpu::Device, _queue: &wgpu::Queue) {}
        fn render(&self, _render_pass: &mut RenderPass) {}
        fn set_transform(&mut self, transform: Transform) {
            self.transform = transform;
        }
        fn transform(&self) -> &Transform {
            &self.transform
        }
        fn is_visible(&self) -> bool {
            self.visible
        }
        fn set_visible(&mut self, visible: bool) {
            self.visible = visible;
        }
    }

    #[test]
    #[cfg(not(target_arch = "wasm32"))]
    fn test_scene_add_remove() {
        use std::sync::{Arc, Mutex};

        let mut scene = Scene::new();
        assert_eq!(scene.len(), 0);
        assert!(scene.is_empty());

        let visual = Arc::new(Mutex::new(DummyVisual {
            visible: true,
            transform: Transform::identity(),
        }));

        let idx = scene.add(visual.clone());
        assert_eq!(idx, 0);
        assert_eq!(scene.len(), 1);
        assert!(!scene.is_empty());

        let removed = scene.remove(idx);
        assert!(removed.is_some());
        assert_eq!(scene.len(), 0);
    }

    #[test]
    #[cfg(target_arch = "wasm32")]
    fn test_scene_add_remove() {
        use std::rc::Rc;
        use std::cell::RefCell;

        let mut scene = Scene::new();
        assert_eq!(scene.len(), 0);
        assert!(scene.is_empty());

        let visual = Rc::new(RefCell::new(DummyVisual {
            visible: true,
            transform: Transform::identity(),
        }));

        let idx = scene.add(visual.clone());
        assert_eq!(idx, 0);
        assert_eq!(scene.len(), 1);
        assert!(!scene.is_empty());

        let removed = scene.remove(idx);
        assert!(removed.is_some());
        assert_eq!(scene.len(), 0);
    }
}
