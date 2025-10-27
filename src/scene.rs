use crate::visual::Visual;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

#[cfg(not(target_arch = "wasm32"))]
type VisualRef = Arc<Mutex<dyn Visual>>;

#[cfg(target_arch = "wasm32")]
type VisualRef = Rc<RefCell<dyn Visual>>;

/// A scene contains a collection of visual objects to be rendered
pub struct Scene {
    visuals: Vec<VisualRef>,
}

impl Scene {
    /// Create a new empty scene
    pub fn new() -> Self {
        Self {
            visuals: Vec::new(),
        }
    }

    /// Add a visual to the scene
    /// Returns an index that can be used to access or remove the visual later
    pub fn add(&mut self, visual: VisualRef) -> usize {
        let index = self.visuals.len();
        self.visuals.push(visual);
        index
    }

    /// Remove a visual at the given index
    /// Returns the removed visual if the index was valid
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
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for visual in &self.visuals {
            if let Ok(mut v) = visual.lock() {
                if v.is_visible() {
                    v.prepare(device, queue);
                }
            }
        }
    }

    /// Prepare all visuals in the scene for rendering (WASM version)
    #[cfg(target_arch = "wasm32")]
    pub fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue) {
        for visual in &self.visuals {
            if let Ok(mut v) = visual.try_borrow_mut() {
                if v.is_visible() {
                    v.prepare(device, queue);
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
