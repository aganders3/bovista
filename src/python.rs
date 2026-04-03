//! Python bindings for Bovista
//!
//! This module provides Python bindings using PyO3 for the Bovista visualization library.

use pyo3::prelude::*;
use numpy::PyReadonlyArray3;
use std::sync::{Arc, Mutex};

use crate::{
    bindings_common::{self, VisualRef},
    Camera, CustomVisual, ImageVisual, LinesVisual, PointsVisual, Renderer, Scene, SlicePlane,
    VertexBufferLayout,
};
use bovista_codegen::visual_methods;

/// Python wrapper for ChunkStatus enum
#[pyclass(name = "ChunkStatus", eq, eq_int)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PyChunkStatus {
    /// The chunk request was accepted and will be loaded asynchronously
    Accepted = 0,
    /// The chunk is already being loaded (request is pending)
    AlreadyPending = 1,
    /// The chunk request was rejected (e.g., at capacity)
    Rejected = 2,
}

/// Python wrapper for ProjectionMode enum
#[pyclass(name = "ProjectionMode", eq, eq_int)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PyProjectionMode {
    /// Perspective projection with field of view
    Perspective = 0,
    /// Orthographic projection with fixed size
    Orthographic = 1,
}

impl From<PyProjectionMode> for crate::ProjectionMode {
    fn from(mode: PyProjectionMode) -> Self {
        match mode {
            PyProjectionMode::Perspective => crate::ProjectionMode::Perspective,
            PyProjectionMode::Orthographic => crate::ProjectionMode::Orthographic,
        }
    }
}

impl From<crate::ProjectionMode> for PyProjectionMode {
    fn from(mode: crate::ProjectionMode) -> Self {
        match mode {
            crate::ProjectionMode::Perspective => PyProjectionMode::Perspective,
            crate::ProjectionMode::Orthographic => PyProjectionMode::Orthographic,
        }
    }
}

/// Python wrapper for the Viewer (combines window, renderer, camera, scene)
#[pyclass(name = "Viewer")]
pub struct PyViewer {
    renderer: Option<Renderer>,
    instance: Option<wgpu::Instance>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    depth_texture: Option<wgpu::TextureView>,
    camera: Camera,
    scene: Scene,
    width: u32,
    height: u32,
}

#[pymethods]
impl PyViewer {
    #[new]
    #[pyo3(signature = (width=800, height=600))]
    fn new(width: u32, height: u32) -> PyResult<Self> {
        Ok(Self {
            renderer: None,
            instance: None,
            surface: None,
            config: None,
            depth_texture: None,
            camera: Camera::new(width as f32 / height as f32),
            scene: Scene::new(),
            width,
            height,
        })
    }

    /// Initialize the renderer (must be called before adding visuals)
    fn initialize(&mut self) -> PyResult<()> {
        // Create WGPU instance and request adapter/device
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // Use Bgra8UnormSrgb which is widely supported
        let surface_format = wgpu::TextureFormat::Bgra8UnormSrgb;

        // Request adapter without surface (for offscreen rendering)
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: None,
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Failed to find GPU adapter"))?;

        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Bovista Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Failed to create device: {}", e)))?;

        let renderer = pollster::block_on(Renderer::new(device, queue, surface_format));

        self.instance = Some(instance);
        self.renderer = Some(renderer);

        Ok(())
    }

    /// Initialize with a native window handle (for embedding in Qt/Tk/etc)
    ///
    /// Args:
    ///     handle: Native window handle (NSView* on macOS, HWND on Windows, Window on X11)
    ///     width: Window width in pixels
    ///     height: Window height in pixels
    fn initialize_with_window(&mut self, handle: usize, width: u32, height: u32) -> PyResult<()> {
        use raw_window_handle::{
            RawDisplayHandle, RawWindowHandle, HasDisplayHandle, HasWindowHandle,
        };

        // Create instance first
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // Create platform-specific raw window handle
        #[cfg(target_os = "macos")]
        let (raw_window_handle, raw_display_handle) = {
            use raw_window_handle::{AppKitWindowHandle, AppKitDisplayHandle};

            let window_handle = AppKitWindowHandle::new(std::ptr::NonNull::new(handle as *mut _)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid window handle"))?);
            let display_handle = AppKitDisplayHandle::new();

            (RawWindowHandle::AppKit(window_handle), RawDisplayHandle::AppKit(display_handle))
        };

        #[cfg(target_os = "windows")]
        let (raw_window_handle, raw_display_handle) = {
            use raw_window_handle::{Win32WindowHandle, WindowsDisplayHandle};

            let window_handle = Win32WindowHandle::new(std::ptr::NonNull::new(handle as *mut _)
                .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyValueError, _>("Invalid window handle"))?);
            let display_handle = WindowsDisplayHandle::new();

            (RawWindowHandle::Win32(window_handle), RawDisplayHandle::Windows(display_handle))
        };

        #[cfg(all(not(target_os = "macos"), not(target_os = "windows")))]
        let (raw_window_handle, raw_display_handle) = {
            use raw_window_handle::{XlibWindowHandle, XlibDisplayHandle};

            // On X11, we need both the window ID and the display pointer
            // For now, assume the handle is the window ID and we'll use the default display
            let window_handle = XlibWindowHandle::new(handle as std::os::raw::c_ulong);

            // We need a display pointer - this is a simplification
            // In a real implementation, we'd need to get this from Tk
            let display_handle = XlibDisplayHandle::new(
                std::ptr::NonNull::new(std::ptr::null_mut()),
                0
            );

            (RawWindowHandle::Xlib(window_handle), RawDisplayHandle::Xlib(display_handle))
        };

        // Create a wrapper that implements the required traits
        struct RawHandles {
            window: RawWindowHandle,
            display: RawDisplayHandle,
        }

        // SAFETY: We're only using this on the main thread where Python is running
        unsafe impl Send for RawHandles {}
        unsafe impl Sync for RawHandles {}

        impl HasWindowHandle for RawHandles {
            fn window_handle(&self) -> Result<raw_window_handle::WindowHandle<'_>, raw_window_handle::HandleError> {
                unsafe { Ok(raw_window_handle::WindowHandle::borrow_raw(self.window)) }
            }
        }

        impl HasDisplayHandle for RawHandles {
            fn display_handle(&self) -> Result<raw_window_handle::DisplayHandle<'_>, raw_window_handle::HandleError> {
                unsafe { Ok(raw_window_handle::DisplayHandle::borrow_raw(self.display)) }
            }
        }

        let handles = RawHandles {
            window: raw_window_handle,
            display: raw_display_handle,
        };

        // Create surface from the raw handle
        let surface = instance.create_surface(handles)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Failed to create surface: {}", e)))?;

        // Request adapter with the surface
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Failed to find GPU adapter"))?;

        // Get surface capabilities and pick format
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Create device and queue
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Bovista Window Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
            },
            None,
        ))
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Failed to create device: {}", e)))?;

        // Configure surface
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

        // Create renderer
        let renderer = pollster::block_on(Renderer::new(device, queue, surface_format));

        // Create depth texture
        let depth_texture = renderer.create_depth_texture(width, height);

        // Update camera aspect ratio
        self.camera.update_aspect_ratio(width as f32 / height as f32);

        // Store everything
        self.instance = Some(instance);
        self.surface = Some(surface);
        self.config = Some(config);
        self.renderer = Some(renderer);
        self.depth_texture = Some(depth_texture);
        self.width = width;
        self.height = height;

        Ok(())
    }

    /// Add any visual to the scene
    ///
    /// # Arguments
    ///
    /// * `visual` - Any visual object (Points, Lines, Image, TiledImage)
    ///
    /// # Returns
    ///
    /// Index that can be used to remove the visual later
    ///
    /// # Example
    ///
    /// ```python
    /// # All of these work:
    /// viewer.add(points_visual)
    /// viewer.add(lines_visual)
    /// viewer.add(image_visual)
    /// viewer.add(tiled_image_visual)
    /// ```
    fn add(&mut self, visual: &Bound<'_, PyAny>) -> PyResult<usize> {
        // Try each visual type
        if let Ok(points) = visual.extract::<PyRef<PyPointsVisual>>() {
            return Ok(self.scene.add(points.inner.clone()));
        }
        if let Ok(lines) = visual.extract::<PyRef<PyLinesVisual>>() {
            return Ok(self.scene.add(lines.inner.clone()));
        }
        if let Ok(image) = visual.extract::<PyRef<PyImageVisual>>() {
            return Ok(self.scene.add(image.inner.clone()));
        }
        if let Ok(custom) = visual.extract::<PyRef<PyCustomVisual>>() {
            return Ok(self.scene.add(custom.inner.clone()));
        }

        Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
            "Expected a visual object (Points, Lines, Image, TiledImage, or Custom)"
        ))
    }

    /// Remove all visuals from the scene
    fn clear_visuals(&mut self) {
        self.scene.clear();
    }

    /// Set camera position
    fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {
        self.camera.position = glam::Vec3::new(x, y, z);
    }

    /// Set camera target (look-at point)
    fn set_camera_target(&mut self, x: f32, y: f32, z: f32) {
        self.camera.target = glam::Vec3::new(x, y, z);
    }

    /// Set camera up vector
    fn set_camera_up(&mut self, x: f32, y: f32, z: f32) {
        self.camera.up = glam::Vec3::new(x, y, z);
    }

    /// Orbit the camera
    fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.orbit(delta_x, delta_y);
    }

    /// Pan the camera (move in view plane)
    fn pan_camera(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.pan(delta_x, delta_y);
    }

    /// Zoom the camera
    fn zoom_camera(&mut self, delta: f32) {
        self.camera.zoom(delta);
    }

    /// Set camera near and far clip planes
    fn set_camera_clip_planes(&mut self, near: f32, far: f32) {
        self.camera.near = near;
        self.camera.far = far;
    }

    /// Set camera projection mode
    fn set_camera_projection_mode(&mut self, mode: PyProjectionMode) {
        self.camera.set_projection_mode(mode.into());
    }

    /// Get camera projection mode
    fn get_camera_projection_mode(&self) -> PyProjectionMode {
        self.camera.projection_mode().into()
    }

    /// Set orthographic camera height
    fn set_camera_ortho_height(&mut self, height: f32) {
        self.camera.set_ortho_height(height);
    }

    /// Get orthographic camera height
    fn get_camera_ortho_height(&self) -> f32 {
        self.camera.ortho_height()
    }

    /// Get the current distance from camera position to target
    fn get_camera_distance(&self) -> f32 {
        (self.camera.position - self.camera.target).length()
    }

    /// Get the number of visuals in the scene
    fn visual_count(&self) -> usize {
        self.scene.len()
    }

    /// Render a single frame (for use with external event loops like Qt/Tk)
    /// Must call initialize_with_window() first
    fn render_frame(&mut self) -> PyResult<()> {
        let surface = self.surface.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("No surface available. Call initialize_with_window() first."))?;

        let renderer = self.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Renderer not initialized"))?;

        let depth_view = self.depth_texture.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Depth texture not created"))?;

        // Update camera and prepare scene
        renderer.update_camera(&self.camera);

        // Build CameraInfo for visuals that need LOD selection / culling
        let camera_info = crate::visual::CameraInfo {
            position: self.camera.position,
            target: self.camera.target,
            fov_y: self.camera.fov_y,
            viewport_width: self.width,
            viewport_height: self.height,
            frustum: self.camera.frustum_planes(),
            projection_mode: self.camera.projection_mode,
            ortho_height: self.camera.ortho_height,
        };

        self.scene.prepare(renderer.device(), renderer.queue(), &camera_info);

        // Get current frame
        let output = surface.get_current_texture()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Failed to get surface texture: {}", e)))?;

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Render
        let clear_color = wgpu::Color {
            r: 0.1,
            g: 0.1,
            b: 0.1,
            a: 1.0,
        };

        renderer.render(&self.scene, &view, depth_view, clear_color);

        // Present
        output.present();

        Ok(())
    }

    /// Resize the rendering surface
    /// Must call initialize_with_window() first
    fn resize(&mut self, width: u32, height: u32) -> PyResult<()> {
        if width == 0 || height == 0 {
            return Ok(());
        }

        let surface = self.surface.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("No surface available. Call initialize_with_window() first."))?;

        let config = self.config.as_mut()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Surface not configured"))?;

        let renderer = self.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Renderer not initialized"))?;

        // Update configuration
        config.width = width;
        config.height = height;

        // Reconfigure surface
        surface.configure(renderer.device(), config);

        // Recreate depth texture
        self.depth_texture = Some(renderer.create_depth_texture(width, height));

        // Update camera aspect ratio
        self.camera.update_aspect_ratio(width as f32 / height as f32);

        self.width = width;
        self.height = height;

        Ok(())
    }

    /// Run an interactive viewer window
    /// This will block until the window is closed
    /// Note: Do NOT call initialize() before run() - this method handles initialization
    fn run(&mut self) -> PyResult<()> {
        // run() should work regardless of whether initialize() was called
        // We'll create fresh GPU resources for the window

        // We need to use winit to create a window and run an event loop
        // Since we're in Python, we'll use a simple polling approach
        use winit::{
            application::ApplicationHandler,
            event::*,
            event_loop::{ActiveEventLoop, EventLoop},
            window::Window,
        };

        struct ViewerApp {
            surface: Option<wgpu::Surface<'static>>,
            config: Option<wgpu::SurfaceConfiguration>,
            window: Option<std::sync::Arc<Window>>,
            renderer: Option<Renderer>,
            camera: Camera,
            scene: Scene,
            depth_texture: Option<wgpu::TextureView>,
            mouse_pressed: bool,
            last_mouse_pos: (f64, f64),
        }

        impl ApplicationHandler for ViewerApp {
            fn resumed(&mut self, event_loop: &ActiveEventLoop) {
                if self.window.is_none() {
                    let window_attributes = Window::default_attributes()
                        .with_title("Bovista Viewer");
                    let window = std::sync::Arc::new(
                        event_loop.create_window(window_attributes).unwrap()
                    );

                    let size = window.inner_size();

                    // Create new wgpu instance for this window
                    let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                        backends: wgpu::Backends::PRIMARY,
                        ..Default::default()
                    });

                    let surface = instance.create_surface(window.clone()).unwrap();

                    let adapter = pollster::block_on(instance.request_adapter(
                        &wgpu::RequestAdapterOptions {
                            power_preference: wgpu::PowerPreference::HighPerformance,
                            compatible_surface: Some(&surface),
                            force_fallback_adapter: false,
                        }
                    )).unwrap();

                    // Create device and queue for this window
                    let (device, queue) = pollster::block_on(adapter.request_device(
                        &wgpu::DeviceDescriptor {
                            label: Some("Bovista Window Device"),
                            required_features: wgpu::Features::empty(),
                            required_limits: wgpu::Limits::default(),
                            memory_hints: Default::default(),
                        },
                        None,
                    )).unwrap();

                    let surface_caps = surface.get_capabilities(&adapter);
                    let surface_format = surface_caps.formats.iter()
                        .copied()
                        .find(|f| f.is_srgb())
                        .unwrap_or(surface_caps.formats[0]);

                    let config = wgpu::SurfaceConfiguration {
                        usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                        format: surface_format,
                        width: size.width,
                        height: size.height,
                        present_mode: surface_caps.present_modes[0],
                        alpha_mode: wgpu::CompositeAlphaMode::Opaque,
                        view_formats: vec![],
                        desired_maximum_frame_latency: 2,
                    };

                    // Create renderer for this window
                    let renderer = pollster::block_on(Renderer::new(device, queue, surface_format));

                    surface.configure(renderer.device(), &config);

                    let depth_texture = renderer.create_depth_texture(size.width, size.height);

                    self.renderer = Some(renderer);
                    self.surface = Some(surface);
                    self.config = Some(config);
                    self.window = Some(window);
                    self.depth_texture = Some(depth_texture);
                }
            }

            fn window_event(
                &mut self,
                event_loop: &ActiveEventLoop,
                _window_id: winit::window::WindowId,
                event: WindowEvent,
            ) {
                match event {
                    WindowEvent::CloseRequested => event_loop.exit(),
                    WindowEvent::RedrawRequested => {
                        if let (Some(surface), Some(_config), Some(depth_view), Some(renderer)) =
                            (&self.surface, &self.config, &self.depth_texture, &self.renderer) {

                            // Update camera and prepare scene
                            renderer.update_camera(&self.camera);

                            // Get viewport dimensions from window
                            let (viewport_width, viewport_height) = if let Some(window) = &self.window {
                                let size = window.inner_size();
                                (size.width, size.height)
                            } else {
                                (800, 600) // Default fallback
                            };

                            // Build CameraInfo for visuals that need LOD selection / culling
                            let camera_info = crate::visual::CameraInfo {
                                position: self.camera.position,
                                target: self.camera.target,
                                fov_y: self.camera.fov_y,
                                viewport_width,
                                viewport_height,
                                frustum: self.camera.frustum_planes(),
                                projection_mode: self.camera.projection_mode,
                                ortho_height: self.camera.ortho_height,
                            };

                            self.scene.prepare(renderer.device(), renderer.queue(), &camera_info);

                            // Render
                            match surface.get_current_texture() {
                                Ok(output) => {
                                    let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

                                    let clear_color = wgpu::Color {
                                        r: 0.1,
                                        g: 0.1,
                                        b: 0.1,
                                        a: 1.0,
                                    };

                                    renderer.render(&self.scene, &view, depth_view, clear_color);
                                    output.present();
                                }
                                Err(e) => eprintln!("Surface error: {:?}", e),
                            }
                        }

                        if let Some(window) = &self.window {
                            window.request_redraw();
                        }
                    }
                    WindowEvent::Resized(physical_size) => {
                        if physical_size.width > 0 && physical_size.height > 0 {
                            if let (Some(surface), Some(config), Some(renderer)) = (&self.surface, &mut self.config, &self.renderer) {
                                config.width = physical_size.width;
                                config.height = physical_size.height;
                                surface.configure(renderer.device(), config);
                                self.camera.update_aspect_ratio(
                                    physical_size.width as f32 / physical_size.height as f32
                                );
                                self.depth_texture = Some(
                                    renderer.create_depth_texture(physical_size.width, physical_size.height)
                                );
                            }
                        }
                    }
                    WindowEvent::MouseInput { state, button, .. } => {
                        if button == MouseButton::Left {
                            self.mouse_pressed = state == ElementState::Pressed;
                        }
                    }
                    WindowEvent::CursorMoved { position, .. } => {
                        if self.mouse_pressed {
                            let delta_x = (position.x - self.last_mouse_pos.0) as f32 * 0.01;
                            let delta_y = (position.y - self.last_mouse_pos.1) as f32 * 0.01;
                            self.camera.orbit(delta_x, delta_y);
                        }
                        self.last_mouse_pos = (position.x, position.y);
                    }
                    WindowEvent::MouseWheel { delta, .. } => {
                        let scroll = match delta {
                            MouseScrollDelta::LineDelta(_, y) => y * 10.0,
                            MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.1,
                        };
                        self.camera.zoom(-scroll);
                    }
                    _ => {}
                }
            }

            fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
                if let Some(window) = &self.window {
                    window.request_redraw();
                }
            }
        }

        let event_loop = EventLoop::new()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Failed to create event loop: {}", e)))?;

        let mut app = ViewerApp {
            surface: None,
            config: None,
            window: None,
            renderer: None,  // Will be created in resumed()
            camera: self.camera.clone(),
            scene: std::mem::replace(&mut self.scene, Scene::new()),
            depth_texture: None,
            mouse_pressed: false,
            last_mouse_pos: (0.0, 0.0),
        };

        event_loop.run_app(&mut app)
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Event loop error: {}", e)))?;

        Ok(())
    }
}

/// Base trait for Python visual wrappers (currently unused but kept for potential future use)
#[allow(dead_code)]
trait PyVisualWrapper {
    fn get_inner(&self) -> VisualRef;
}

/// Python wrapper for PointsVisual
#[pyclass(name = "Points")]
pub struct PyPointsVisual {
    inner: VisualRef,
}

impl PyVisualWrapper for PyPointsVisual {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[pymethods]
impl PyPointsVisual {
    /// Create a point cloud from numpy arrays
    ///
    /// Args:
    ///     viewer: The viewer instance
    ///     positions: Nx3 numpy array of point positions
    ///     colors: Nx3 numpy array of RGB colors (0-1 range)
    #[staticmethod]
    fn from_numpy(
        viewer: &PyViewer,
        positions: PyReadonlyArray3<f32>,
        colors: PyReadonlyArray3<f32>,
    ) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized. Call initialize() first."))?;

        // Convert numpy arrays to vertices
        let pos_array = positions.as_array();
        let col_array = colors.as_array();

        if pos_array.shape() != col_array.shape() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Positions and colors must have the same shape",
            ));
        }

        let n_points = pos_array.shape()[0];
        let mut vertices = Vec::with_capacity(n_points);

        for i in 0..n_points {
            vertices.push(crate::visuals::points::PointVertex {
                position: [pos_array[[i, 0, 0]], pos_array[[i, 0, 1]], pos_array[[i, 0, 2]]],
                color: [col_array[[i, 0, 0]], col_array[[i, 0, 1]], col_array[[i, 0, 2]]],
            });
        }

        let visual = PointsVisual::new(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            vertices,
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
    }

    /// Create a test cube of points
    #[staticmethod]
    fn test_cube(viewer: &PyViewer, size: u32) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        let visual = PointsVisual::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            size,
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
    }
}

/// Python wrapper for LinesVisual
#[pyclass(name = "Lines")]
pub struct PyLinesVisual {
    inner: VisualRef,
}

impl PyVisualWrapper for PyLinesVisual {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[pymethods]
impl PyLinesVisual {
    /// Create an axis helper
    #[staticmethod]
    fn axis_helper(viewer: &PyViewer, length: f32) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        let visual = LinesVisual::axis_helper(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            length,
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
    }

    /// Create a wireframe cube
    #[staticmethod]
    fn test_cube(viewer: &PyViewer) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        let visual = LinesVisual::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
    }
}

/// Python wrapper for ImageVisual
#[pyclass(name = "Image")]
pub struct PyImageVisual {
    inner: VisualRef,
    pending_chunks: crate::visuals::virtual_texture::PendingChunks,
}

impl PyVisualWrapper for PyImageVisual {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[visual_methods(ImageVisual)]
#[pymethods]
impl PyImageVisual {
    #[new]
    #[pyo3(signature = (viewer, levels, max_tiles, loader))]
    fn new(
        viewer: &PyViewer,
        levels: Vec<PyLevelMetadata>,
        max_tiles: usize,
        loader: PyObject,
    ) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        if levels.is_empty() {
            return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                "Must provide at least one LOD level",
            ));
        }

        use crate::visuals::virtual_texture::LodLevelConfig;
        let rust_levels: Vec<LodLevelConfig> = levels
            .iter()
            .map(|l| LodLevelConfig {
                volume_size: l.volume_size,
                tile_size: l.chunk_size,
                voxel_size: l.voxel_size,
                scale_factor: l.scale_factor,
                translation: l.translation,
            })
            .collect();

        use crate::visuals::tile::{TileRequest, TileLoaderFn, ChunkStatus};
        let loader_arc = Arc::new(loader);
        let loader_clone = loader_arc.clone();
        let loader_fn: TileLoaderFn = Box::new(move |request: TileRequest| -> ChunkStatus {
            Python::with_gil(|py| {
                let lod_level = request.lod_level.unwrap_or(0);
                let result = loader_clone.bind(py).call1((lod_level, request.z, request.y, request.x));
                match result {
                    Ok(obj) => {
                        if let Ok(status) = obj.extract::<PyChunkStatus>() {
                            match status {
                                PyChunkStatus::Accepted => ChunkStatus::Accepted,
                                PyChunkStatus::AlreadyPending => ChunkStatus::AlreadyPending,
                                PyChunkStatus::Rejected => ChunkStatus::Rejected,
                            }
                        } else {
                            ChunkStatus::Rejected
                        }
                    }
                    Err(e) => {
                        eprintln!("[Image] Error calling loader: {}", e);
                        ChunkStatus::Rejected
                    }
                }
            })
        });

        let visual = ImageVisual::new(
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            rust_levels,
            max_tiles,
            loader_fn,
        );

        let pending_chunks = visual.pending_chunks().unwrap();
        let inner = Arc::new(Mutex::new(visual));
        Ok(Self { inner, pending_chunks })
    }

    /// Set the slice plane position along Z axis
    fn set_slice_z(&self, z: f32) -> PyResult<()> {}

    /// Set the slice plane position along Y axis
    fn set_slice_y(&self, y: f32) -> PyResult<()> {}

    /// Set the slice plane position along X axis
    fn set_slice_x(&self, x: f32) -> PyResult<()> {}

    /// Set contrast limits
    fn set_contrast(&self, min: f32, max: f32) -> PyResult<()> {
        bindings_common::with_visual_mut::<ImageVisual, _, _>(
            &self.inner,
            |v| v.set_contrast_limits(min, max)
        ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
    }

    /// Set a colormap LUT.
    /// `rgba` should be a numpy array of shape (256, 4) with dtype uint8 (RGBA values 0-255).
    /// Pass None or an empty array to reset to grayscale.
    #[pyo3(signature = (rgba=None))]
    fn set_colormap(&self, rgba: Option<PyReadonlyArray3<u8>>) -> PyResult<()> {
        let bytes: Vec<u8> = match rgba {
            Some(arr) => {
                let a = arr.as_array();
                a.iter().copied().collect()
            }
            None => Vec::new(),
        };
        bindings_common::with_visual_mut::<ImageVisual, _, _>(
            &self.inner,
            |v| v.set_colormap(&bytes)
        ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
    }

    /// Set an arbitrary slice plane
    fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> PyResult<()> {
        bindings_common::with_visual_mut::<ImageVisual, _, _>(
            &self.inner,
            |v| {
                let plane = SlicePlane::new([px, py, pz], [nx, ny, nz]);
                v.set_slice_plane(plane);
            }
        ).map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
    }

    /// Enable or disable debug LOD tinting
    fn set_debug_mode(&self, enabled: bool) -> PyResult<()> {}

    /// Provide uint16 tile data (R16Float in the atlas).
    fn set_chunk_data_u16(
        &self,
        lod_level: usize,
        z: u32,
        y: u32,
        x: u32,
        data: PyReadonlyArray3<u16>,
    ) -> PyResult<()> {
        use crate::visuals::tile::{TileData, TileKey};
        let a = data.as_array();
        let tile_data = TileData {
            data: a.iter()
                .flat_map(|&v| half::f16::from_f32(v as f32 / u16::MAX as f32).to_le_bytes())
                .collect(),
            width: a.shape()[2] as u32,
            height: a.shape()[1] as u32,
            depth: a.shape()[0] as u32,
            format: wgpu::TextureFormat::R16Float,
        };
        self.pending_chunks.lock().unwrap().insert(TileKey { lod_level, z, y, x }, tile_data);
        Ok(())
    }

    /// Set LOD bias (negative = prefer higher resolution).
    fn set_lod_bias(&self, bias: f32) -> PyResult<()> {
        bindings_common::with_visual_mut::<ImageVisual, _, _>(&self.inner, |v| {
            v.set_lod_bias(bias)
        })
        .map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
    }

    /// Returns (loaded_tiles, visible_tiles).
    fn get_stats(&self) -> PyResult<(usize, usize)> {
        bindings_common::with_visual_ref::<ImageVisual, _, _>(&self.inner, |v| v.get_stats())
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyTypeError, _>(e))
    }
}


/// Python wrapper for LevelMetadata
#[pyclass(name = "LevelMetadata")]
#[derive(Clone)]
pub struct PyLevelMetadata {
    #[pyo3(get, set)]
    pub volume_size: (u32, u32, u32),
    #[pyo3(get, set)]
    pub chunk_size: (u32, u32, u32),
    #[pyo3(get, set)]
    pub voxel_size: (f32, f32, f32),
    #[pyo3(get, set)]
    pub scale_factor: f32,
    #[pyo3(get, set)]
    pub translation: (f32, f32, f32),
}

#[pymethods]
impl PyLevelMetadata {
    #[new]
    #[pyo3(signature = (volume_size, chunk_size, voxel_size, scale_factor, translation=None))]
    fn new(
        volume_size: (u32, u32, u32),
        chunk_size: (u32, u32, u32),
        voxel_size: (f32, f32, f32),
        scale_factor: f32,
        translation: Option<(f32, f32, f32)>,
    ) -> Self {
        Self {
            volume_size,
            chunk_size,
            voxel_size,
            scale_factor,
            translation: translation.unwrap_or((0.0, 0.0, 0.0)),
        }
    }
}

/// Python wrapper for CustomVisual
#[pyclass(name = "Custom")]
pub struct PyCustomVisual {
    inner: VisualRef,
}

impl PyVisualWrapper for PyCustomVisual {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[pymethods]
impl PyCustomVisual {
    /// Create a custom visual with a user-provided WGSL shader
    ///
    /// # Arguments
    ///
    /// * `viewer` - The viewer instance
    /// * `shader_source` - WGSL shader source code as a string
    /// * `vertex_data` - Vertex data as bytes (use numpy.tobytes())
    /// * `vertex_layout` - Vertex buffer layout description
    /// * `topology` - Primitive topology ("point_list", "line_list", "triangle_list", etc.)
    ///
    /// # Example
    ///
    /// ```python
    /// import bovista as bv
    /// import numpy as np
    ///
    /// shader = '''
    /// struct Uniforms {
    ///     view_proj: mat4x4<f32>,
    /// }
    /// @group(0) @binding(0) var<uniform> uniforms: Uniforms;
    ///
    /// @vertex
    /// fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    ///     return uniforms.view_proj * vec4<f32>(pos, 1.0);
    /// }
    ///
    /// @fragment
    /// fn fs_main() -> @location(0) vec4<f32> {
    ///     return vec4<f32>(1.0, 0.0, 0.0, 1.0);  // Red
    /// }
    /// '''
    ///
    /// # Triangle vertices
    /// vertices = np.array([
    ///     [0.0, 1.0, 0.0],
    ///     [-1.0, -1.0, 0.0],
    ///     [1.0, -1.0, 0.0],
    /// ], dtype=np.float32)
    ///
    /// # Create layout: one vec3 at location 0
    /// layout = bv.PyVertexBufferLayout(12, [  # stride = 3 floats * 4 bytes
    ///     (0, "Float32x3", 0)  # (location, format, offset)
    /// ])
    ///
    /// visual = bv.PyCustomVisual.new(
    ///     viewer, shader, vertices.tobytes(), layout, "triangle_list"
    /// )
    /// viewer.add(visual)
    /// ```
    #[staticmethod]
    fn new(
        viewer: &PyViewer,
        shader_source: &str,
        vertex_data: &[u8],
        vertex_layout: PyVertexBufferLayout,
        topology: &str,
    ) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        // Parse topology string
        let topology = match topology.to_lowercase().as_str() {
            "point_list" | "pointlist" => wgpu::PrimitiveTopology::PointList,
            "line_list" | "linelist" => wgpu::PrimitiveTopology::LineList,
            "line_strip" | "linestrip" => wgpu::PrimitiveTopology::LineStrip,
            "triangle_list" | "trianglelist" => wgpu::PrimitiveTopology::TriangleList,
            "triangle_strip" | "trianglestrip" => wgpu::PrimitiveTopology::TriangleStrip,
            _ => return Err(PyErr::new::<pyo3::exceptions::PyValueError, _>(
                format!("Invalid topology: '{}'. Use 'point_list', 'line_list', 'line_strip', 'triangle_list', or 'triangle_strip'", topology)
            )),
        };

        let visual = CustomVisual::new(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            shader_source,
            vertex_data,
            vertex_layout.into_rust(),
            topology,
        ).map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(e))?;

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
    }
}

/// Python wrapper for vertex buffer layout
#[pyclass(name = "VertexBufferLayout")]
#[derive(Clone)]
pub struct PyVertexBufferLayout {
    stride: u64,
    attributes: Vec<(u32, String, u64)>,  // (location, format, offset)
}

#[pymethods]
impl PyVertexBufferLayout {
    #[new]
    fn new(stride: u64, attributes: Vec<(u32, String, u64)>) -> Self {
        Self { stride, attributes }
    }
}

impl PyVertexBufferLayout {
    fn into_rust(self) -> VertexBufferLayout {
        use crate::visual::VertexFormat;

        let attributes = self.attributes.into_iter().map(|(location, format_str, _offset)| {
            let format = match format_str.to_lowercase().as_str() {
                "float32" => VertexFormat::Float32,
                "float32x2" => VertexFormat::Float32x2,
                "float32x3" => VertexFormat::Float32x3,
                "float32x4" => VertexFormat::Float32x4,
                _ => panic!("Unknown vertex format: {}", format_str),
            };
            crate::visual::VertexAttribute {
                name: format!("attr_{}", location),
                format,
                location,
            }
        }).collect();

        VertexBufferLayout {
            stride: self.stride,
            attributes,
        }
    }
}


/// Python module definition
#[pymodule]
fn bovista(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyViewer>()?;
    m.add_class::<PyPointsVisual>()?;
    m.add_class::<PyLinesVisual>()?;
    m.add_class::<PyImageVisual>()?;
    m.add_class::<PyLevelMetadata>()?;
    m.add_class::<PyCustomVisual>()?;
    m.add_class::<PyVertexBufferLayout>()?;
    m.add_class::<PyChunkStatus>()?;
    m.add_class::<PyProjectionMode>()?;
    Ok(())
}
