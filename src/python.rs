//! Python bindings for Bovista
//!
//! This module provides Python bindings using PyO3 for the Bovista visualization library.

use pyo3::prelude::*;
use numpy::PyReadonlyArray3;
use std::sync::{Arc, Mutex};

use crate::{
    Camera, ImageVisual, LinesVisual, PointsVisual, Renderer, Scene, SlicePlane, TiledImageVisual, Visual,
};
use crate::visuals::tiled_image::{ChunkRequest as TiledChunkRequest, ChunkData as TiledChunkData};

// Type alias for visual references
type VisualRef = Arc<Mutex<dyn Visual>>;

/// Helper macro to downcast a locked Visual to a specific type and execute code
macro_rules! with_visual {
    // Mutable access
    ($visual_ref:expr, $type:ty, $body:expr) => {{
        let mut visual = $visual_ref.lock()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Mutex lock failed: {}", e)))?;

        use std::any::Any;
        let visual_any: &mut dyn Any = &mut *visual;
        if let Some(typed_visual) = visual_any.downcast_mut::<$type>() {
            Ok($body(typed_visual))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                format!("Not a {}", stringify!($type))
            ))
        }
    }};

    // Immutable access (read-only)
    (ref $visual_ref:expr, $type:ty, $body:expr) => {{
        let visual = $visual_ref.lock()
            .map_err(|e| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>(format!("Mutex lock failed: {}", e)))?;

        use std::any::Any;
        let visual_any: &dyn Any = &*visual;
        if let Some(typed_visual) = visual_any.downcast_ref::<$type>() {
            Ok($body(typed_visual))
        } else {
            Err(PyErr::new::<pyo3::exceptions::PyTypeError, _>(
                format!("Not a {}", stringify!($type))
            ))
        }
    }};
}

/// Python wrapper for the Viewer (combines window, renderer, camera, scene)
#[pyclass]
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

    /// Add a points visual to the scene
    fn add_points(&mut self, visual: &PyPointsVisual) -> PyResult<usize> {
        Ok(self.scene.add(visual.inner.clone()))
    }

    /// Add a lines visual to the scene
    fn add_lines(&mut self, visual: &PyLinesVisual) -> PyResult<usize> {
        Ok(self.scene.add(visual.inner.clone()))
    }

    /// Add an image visual to the scene
    fn add_image(&mut self, visual: &PyImageVisual) -> PyResult<usize> {
        Ok(self.scene.add(visual.inner.clone()))
    }

    /// Add a tiled image visual to the scene
    fn add_tiled_image(&mut self, visual: &PyTiledImageVisual) -> PyResult<usize> {
        Ok(self.scene.add(visual.inner.clone()))
    }

    /// Set camera position
    fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {
        self.camera.position = glam::Vec3::new(x, y, z);
    }

    /// Set camera target (look-at point)
    fn set_camera_target(&mut self, x: f32, y: f32, z: f32) {
        self.camera.target = glam::Vec3::new(x, y, z);
    }

    /// Orbit the camera
    fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {
        self.camera.orbit(delta_x, delta_y);
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
        self.scene.prepare(renderer.device(), renderer.queue());

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
                            self.scene.prepare(renderer.device(), renderer.queue());

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

/// Python wrapper for PointsVisual
#[pyclass]
pub struct PyPointsVisual {
    inner: VisualRef,
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
#[pyclass]
pub struct PyLinesVisual {
    inner: VisualRef,
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
#[pyclass]
pub struct PyImageVisual {
    inner: VisualRef,
}

#[pymethods]
impl PyImageVisual {
    /// Create an ImageVisual from a 3D numpy array
    ///
    /// Args:
    ///     viewer: The viewer instance
    ///     volume: 3D numpy array (depth, height, width) of uint8 values
    #[staticmethod]
    fn from_numpy(
        viewer: &PyViewer,
        volume: PyReadonlyArray3<u8>,
    ) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        let array = volume.as_array();
        let shape = array.shape();
        let depth = shape[0] as u32;
        let height = shape[1] as u32;
        let width = shape[2] as u32;

        // Convert to contiguous byte slice
        let data: Vec<u8> = array.iter().copied().collect();

        let visual = ImageVisual::new_3d(
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            &data,
            width,
            height,
            depth,
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
    }

    /// Set the slice plane position along Z axis
    fn set_slice_z(&self, z: f32) -> PyResult<()> {
        with_visual!(self.inner, ImageVisual, |v: &mut ImageVisual| {
            v.set_slice_z(z);
        })
    }

    /// Set the slice plane position along Y axis
    fn set_slice_y(&self, y: f32) -> PyResult<()> {
        with_visual!(self.inner, ImageVisual, |v: &mut ImageVisual| {
            v.set_slice_y(y);
        })
    }

    /// Set the slice plane position along X axis
    fn set_slice_x(&self, x: f32) -> PyResult<()> {
        with_visual!(self.inner, ImageVisual, |v: &mut ImageVisual| {
            v.set_slice_x(x);
        })
    }

    /// Set contrast limits
    fn set_contrast(&self, min: f32, max: f32) -> PyResult<()> {
        with_visual!(self.inner, ImageVisual, |v: &mut ImageVisual| {
            v.set_contrast_limits(min, max);
        })
    }

    /// Set an arbitrary slice plane
    fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> PyResult<()> {
        with_visual!(self.inner, ImageVisual, |v: &mut ImageVisual| {
            let plane = SlicePlane::new([px, py, pz], [nx, ny, nz]);
            v.set_slice_plane(plane);
        })
    }
}

/// Python wrapper for TiledImageVisual
#[pyclass]
pub struct PyTiledImageVisual {
    inner: VisualRef,
}

#[pymethods]
impl PyTiledImageVisual {
    /// Create a tiled image visual with a Python chunk loader callback
    ///
    /// Args:
    ///     viewer: The viewer instance
    ///     volume_size: Tuple of (depth, height, width) for full volume
    ///     chunk_size: Tuple of (depth, height, width) for each chunk
    ///     loader: Python callable that takes (z, y, x) and returns numpy array or None
    ///     max_loaded_chunks: Maximum number of chunks to keep in memory (default: 100)
    #[staticmethod]
    fn from_loader(
        viewer: &PyViewer,
        volume_size: (u32, u32, u32),
        chunk_size: (u32, u32, u32),
        loader: PyObject,
        max_loaded_chunks: Option<usize>,
    ) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        let max_chunks = max_loaded_chunks.unwrap_or(100);

        // Create a Rust closure that calls the Python loader
        let loader_arc = Arc::new(loader);
        let loader_fn = Arc::new(move |request: TiledChunkRequest| -> Option<TiledChunkData> {
            Python::with_gil(|py| {
                // Debug first few calls
                static CALL_COUNT: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
                let count = CALL_COUNT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                if count < 5 {
                    eprintln!("  🦀 RUST BINDINGS v2 call #{}: ChunkRequest z={}, y={}, x={} -> Python({}, {}, {})",
                             count, request.chunk_z, request.chunk_y, request.chunk_x,
                             request.chunk_z, request.chunk_y, request.chunk_x);
                }

                // Call Python loader with chunk indices
                // Call Python loader with (z, y, x) indices
                let result = loader_arc.call1(
                    py,
                    (request.chunk_z, request.chunk_y, request.chunk_x)  // Already in (z, y, x) order
                );

                match result {
                    Ok(obj) => {
                        // Check if it's None
                        if obj.is_none(py) {
                            return None;
                        }

                        // Try to extract as numpy array
                        let array: PyReadonlyArray3<u8> = obj.extract(py).ok()?;
                        let arr = array.as_array();
                        let shape = arr.shape();

                        // Convert to contiguous Vec<u8>
                        let data: Vec<u8> = arr.iter().copied().collect();

                        Some(TiledChunkData {
                            data,
                            width: shape[2] as u32,
                            height: shape[1] as u32,
                            depth: shape[0] as u32,
                        })
                    }
                    Err(_) => {
                        // Chunk loader failed - this is expected for out-of-bounds chunks
                        None
                    }
                }
            })
        });

        // Create TiledImageVisual
        let tiled_visual = TiledImageVisual::new(
            volume_size,
            chunk_size,
            loader_fn,
            max_chunks,
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(tiled_visual)),
        })
    }

    /// Set the slice plane
    fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> PyResult<()> {
        with_visual!(self.inner, TiledImageVisual, |v: &mut TiledImageVisual| {
            let plane = SlicePlane::new([px, py, pz], [nx, ny, nz]);
            v.set_slice_plane(plane);
        })
    }

    /// Set contrast limits
    fn set_contrast(&self, min: f32, max: f32) -> PyResult<()> {
        with_visual!(self.inner, TiledImageVisual, |v: &mut TiledImageVisual| {
            v.set_contrast_limits(min, max);
        })
    }

    /// Get statistics about loaded chunks
    fn get_stats(&self) -> PyResult<(usize, usize)> {
        with_visual!(ref self.inner, TiledImageVisual, |v: &TiledImageVisual| {
            (v.loaded_chunk_count(), v.visible_chunk_count())
        })
    }

    /// Enable/disable debug visualization (wireframes and color-coded Z-layers)
    fn set_debug_mode(&self, enabled: bool) -> PyResult<()> {
        with_visual!(self.inner, TiledImageVisual, |v: &mut TiledImageVisual| {
            v.set_debug_mode(enabled);
        })
    }

    /// Prepare chunks (load visible ones, evict old ones)
    /// This should be called before rendering
    fn prepare_chunks(&self, viewer: &PyViewer) -> PyResult<()> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| PyErr::new::<pyo3::exceptions::PyRuntimeError, _>("Viewer not initialized"))?;

        with_visual!(self.inner, TiledImageVisual, |v: &mut TiledImageVisual| {
            v.prepare_chunks(
                renderer.device(),
                renderer.queue(),
                renderer.surface_format(),
                renderer.camera_bind_group_layout(),
            );
        })
    }
}

/// Python module definition
#[pymodule]
fn bovista(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyViewer>()?;
    m.add_class::<PyPointsVisual>()?;
    m.add_class::<PyLinesVisual>()?;
    m.add_class::<PyImageVisual>()?;
    m.add_class::<PyTiledImageVisual>()?;
    Ok(())
}
