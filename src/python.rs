//! Python bindings for Bovista
//!
//! This module provides Python bindings using PyO3 for the Bovista visualization library.

// PyO3's #[pymethods] expansion runs each return value through `.into()`. For
// methods returning `PyResult<()>` (or any `Result<_, PyErr>`), that produces
// `PyErr -> PyErr` and clippy flags it as useless. The conversion is generated
// by the macro, not us, so silence it module-wide.
#![allow(clippy::useless_conversion)]

use pyo3::prelude::*;
use numpy::PyReadonlyArray3;
use std::sync::{Arc, Mutex};

use crate::{
    bindings_common::{self, VisualRef},
    BlendMode, Camera, Custom, Image, Labels, Lines, Points, Renderer, Scene, SlicePlane,
    AverageVolume, DirectVolume, IsosurfaceVolume, MinipVolume, MipVolume,
    VertexBufferLayout, Visual,
};
use bovista_codegen::{camera_methods, visual_methods};

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

/// How a visual composites with the framebuffer. `Additive` (order-independent)
/// is the basis for multi-channel compositing.
#[pyclass(name = "BlendMode", eq, eq_int)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PyBlendMode {
    Normal = 0,
    Additive = 1,
}

impl From<PyBlendMode> for BlendMode {
    fn from(mode: PyBlendMode) -> Self {
        match mode {
            PyBlendMode::Normal => BlendMode::Normal,
            PyBlendMode::Additive => BlendMode::Additive,
        }
    }
}

impl From<BlendMode> for PyBlendMode {
    fn from(mode: BlendMode) -> Self {
        match mode {
            BlendMode::Normal => PyBlendMode::Normal,
            BlendMode::Additive => PyBlendMode::Additive,
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

#[camera_methods]
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
        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        // Create platform-specific raw window handle
        #[cfg(target_os = "macos")]
        let (raw_window_handle, raw_display_handle) = {
            use raw_window_handle::{AppKitWindowHandle, AppKitDisplayHandle};

            let window_handle = AppKitWindowHandle::new(std::ptr::NonNull::new(handle as *mut _)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Invalid window handle"))?);
            let display_handle = AppKitDisplayHandle::new();

            (RawWindowHandle::AppKit(window_handle), RawDisplayHandle::AppKit(display_handle))
        };

        #[cfg(target_os = "windows")]
        let (raw_window_handle, raw_display_handle) = {
            use raw_window_handle::{Win32WindowHandle, WindowsDisplayHandle};

            let window_handle = Win32WindowHandle::new(std::ptr::NonNull::new(handle as *mut _)
                .ok_or_else(|| pyo3::exceptions::PyValueError::new_err("Invalid window handle"))?);
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
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to create surface: {}", e)))?;

        // Request adapter with the surface
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .map_err(|_| pyo3::exceptions::PyRuntimeError::new_err("Failed to find GPU adapter"))?;

        // Get surface capabilities and pick format
        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps.formats.iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Create device and queue
        let (device, queue) = pollster::block_on(adapter.request_device(
            &wgpu::DeviceDescriptor {
                label: Some("Bovista Window Device"),
                required_features: wgpu::Features::empty(),
                required_limits: wgpu::Limits::default(),
                memory_hints: Default::default(),
                trace: wgpu::Trace::Off,
            },
        ))
        .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to create device: {}", e)))?;

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
        if let Ok(points) = visual.extract::<PyRef<PyPoints>>() {
            return Ok(self.scene.add(points.inner.clone()));
        }
        if let Ok(lines) = visual.extract::<PyRef<PyLines>>() {
            return Ok(self.scene.add(lines.inner.clone()));
        }
        if let Ok(image) = visual.extract::<PyRef<PyImage>>() {
            return Ok(self.scene.add(image.inner.clone()));
        }
        if let Ok(labels) = visual.extract::<PyRef<PyLabels>>() {
            return Ok(self.scene.add(labels.inner.clone()));
        }
        if let Ok(v) = visual.extract::<PyRef<PyDirectVolume>>()    { return Ok(self.scene.add(v.inner.clone())); }
        if let Ok(v) = visual.extract::<PyRef<PyMipVolume>>()       { return Ok(self.scene.add(v.inner.clone())); }
        if let Ok(v) = visual.extract::<PyRef<PyMinipVolume>>()     { return Ok(self.scene.add(v.inner.clone())); }
        if let Ok(v) = visual.extract::<PyRef<PyAverageVolume>>()   { return Ok(self.scene.add(v.inner.clone())); }
        if let Ok(v) = visual.extract::<PyRef<PyIsosurfaceVolume>>(){ return Ok(self.scene.add(v.inner.clone())); }
        if let Ok(custom) = visual.extract::<PyRef<PyCustom>>() {
            return Ok(self.scene.add(custom.inner.clone()));
        }

        Err(pyo3::exceptions::PyTypeError::new_err(
            "Expected a visual object (Points, Lines, Image, Labels, *Volume, or Custom)"
        ))
    }

    fn clear_visuals(&mut self) {}
    fn set_camera_position(&mut self, x: f32, y: f32, z: f32) {}
    fn set_camera_target(&mut self, x: f32, y: f32, z: f32) {}
    fn set_camera_up(&mut self, x: f32, y: f32, z: f32) {}
    fn orbit_camera(&mut self, delta_x: f32, delta_y: f32) {}
    fn pan_camera(&mut self, delta_x: f32, delta_y: f32) {}
    fn zoom_camera(&mut self, delta: f32) {}
    fn set_camera_clip_planes(&mut self, near: f32, far: f32) {}
    fn set_camera_projection_mode(&mut self, mode: PyProjectionMode) {}
    fn get_camera_projection_mode(&self) -> PyProjectionMode {}
    fn set_camera_ortho_height(&mut self, height: f32) {}
    fn get_camera_ortho_height(&self) -> f32 {}
    fn get_camera_distance(&self) -> f32 {}
    /// Face an oblique slice plane head-on: position the camera `distance`
    /// away from `(cx, cy, cz)` along the slice normal `(nx, ny, nz)`.
    #[allow(clippy::too_many_arguments)]
    fn align_camera_to_slice(&mut self, cx: f32, cy: f32, cz: f32, nx: f32, ny: f32, nz: f32, distance: f32) {}
    fn visual_count(&self) -> usize {}

    /// Render a single frame (for use with external event loops like Qt/Tk)
    /// Must call initialize_with_window() first
    fn render_frame(&mut self) -> PyResult<()> {
        let surface = self.surface.as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("No surface available. Call initialize_with_window() first."))?;

        let renderer = self.renderer.as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Renderer not initialized"))?;

        let depth_view = self.depth_texture.as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Depth texture not created"))?;

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
            view_proj: self.camera.view_projection_matrix(),
        };

        self.scene.prepare(renderer.device(), renderer.queue(), &camera_info);

        // Get current frame
        let output = surface.get_current_texture()
            .map_err(|e| pyo3::exceptions::PyRuntimeError::new_err(format!("Failed to get surface texture: {}", e)))?;

        let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

        // Render
        let clear_color = wgpu::Color {
            r: 0.0,
            g: 0.0,
            b: 0.0,
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
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("No surface available. Call initialize_with_window() first."))?;

        let config = self.config.as_mut()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Surface not configured"))?;

        let renderer = self.renderer.as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Renderer not initialized"))?;

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
}

/// Base trait for Python visual wrappers (currently unused but kept for potential future use)
#[allow(dead_code)]
trait PyVisualWrapper {
    fn get_inner(&self) -> VisualRef;
}

/// Python wrapper for Points
#[pyclass(name = "Points")]
pub struct PyPoints {
    inner: VisualRef,
}

impl PyVisualWrapper for PyPoints {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[pymethods]
impl PyPoints {
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
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Viewer not initialized. Call initialize() first."))?;

        // Convert numpy arrays to vertices
        let pos_array = positions.as_array();
        let col_array = colors.as_array();

        if pos_array.shape() != col_array.shape() {
            return Err(pyo3::exceptions::PyValueError::new_err(
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

        let visual = Points::new(
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
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Viewer not initialized"))?;

        let visual = Points::test_cube(
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

/// Python wrapper for Lines
#[pyclass(name = "Lines")]
pub struct PyLines {
    inner: VisualRef,
}

impl PyVisualWrapper for PyLines {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[pymethods]
impl PyLines {
    /// Create an axis helper
    #[staticmethod]
    fn axis_helper(viewer: &PyViewer, length: f32) -> PyResult<Self> {
        let renderer = viewer.renderer.as_ref()
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Viewer not initialized"))?;

        let visual = Lines::axis_helper(
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
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Viewer not initialized"))?;

        let visual = Lines::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
        );

        Ok(Self {
            inner: Arc::new(Mutex::new(visual)),
        })
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

impl PyLevelMetadata {
    fn to_lod_level_config(&self) -> crate::visuals::virtual_texture::LodLevelConfig {
        crate::visuals::virtual_texture::LodLevelConfig {
            volume_size: self.volume_size,
            tile_size: self.chunk_size,
            voxel_size: self.voxel_size,
            scale_factor: self.scale_factor,
            translation: self.translation,
        }
    }
}

/// Python wrapper for Custom
#[pyclass(name = "Custom")]
pub struct PyCustom {
    inner: VisualRef,
}

impl PyVisualWrapper for PyCustom {
    fn get_inner(&self) -> VisualRef {
        self.inner.clone()
    }
}

#[pymethods]
impl PyCustom {
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
    /// visual = bv.PyCustom.new(
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
            .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Viewer not initialized"))?;

        // Parse topology string
        let topology = match topology.to_lowercase().as_str() {
            "point_list" | "pointlist" => wgpu::PrimitiveTopology::PointList,
            "line_list" | "linelist" => wgpu::PrimitiveTopology::LineList,
            "line_strip" | "linestrip" => wgpu::PrimitiveTopology::LineStrip,
            "triangle_list" | "trianglelist" => wgpu::PrimitiveTopology::TriangleList,
            "triangle_strip" | "trianglestrip" => wgpu::PrimitiveTopology::TriangleStrip,
            _ => return Err(pyo3::exceptions::PyValueError::new_err(
                format!("Invalid topology: '{}'. Use 'point_list', 'line_list', 'line_strip', 'triangle_list', or 'triangle_strip'", topology)
            )),
        };

        let visual = Custom::new(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            shader_source,
            vertex_data,
            vertex_layout.into_rust(),
            topology,
        ).map_err(pyo3::exceptions::PyRuntimeError::new_err)?;

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


// ── Virtual-texture visuals: slice Image + one class per volume mode ────────
//
// All of these stream tiles through bovista's virtual texture: bovista
// publishes `wanted` each prepare; Python polls it via `wanted_keys()` and
// pushes tile data via `set_chunk_data_u8/u16(...)`. The shared machinery
// lives in the macro; per-type knobs go in each invocation's `extra` block.

/// Generate the Python wrapper struct + `#[pymethods]` impl for a
/// virtual-texture-backed visual (the slice `Image` and every volume mode).
macro_rules! py_vt_visual {
    // Public arm: default (normalized) u16/u8 packers — full-range → [0, 1].
    (
        $wrapper:ident,
        $py_name:literal,
        $rust_ty:ident
        $(, extra: { $($extra:tt)* })?
    ) => {
        py_vt_visual!(
            @full $wrapper, $py_name, $rust_ty,
            bindings_common::pack_u16_tile, bindings_common::pack_u8_tile
            $(, extra: { $($extra)* })?
        );
    };
    // Full arm: caller supplies the tile packers. Labels pass the
    // raw-magnitude variants so integer IDs survive the R16Float atlas
    // (see bindings_common::pack_u16_label_tile).
    (
        @full
        $wrapper:ident,
        $py_name:literal,
        $rust_ty:ident,
        $pack16:path,
        $pack8:path
        $(, extra: { $($extra:tt)* })?
    ) => {
        #[pyclass(name = $py_name)]
        pub struct $wrapper {
            pub(crate) inner: VisualRef,
            pending_chunks: crate::visuals::virtual_texture::PendingChunks,
            wanted: crate::visuals::virtual_texture::Wanted,
        }

        impl PyVisualWrapper for $wrapper {
            fn get_inner(&self) -> VisualRef { self.inner.clone() }
        }

        #[visual_methods($rust_ty)]
        #[pymethods]
        impl $wrapper {
            #[new]
            #[pyo3(signature = (viewer, levels, max_tiles, atlas_count = 1))]
            fn new(
                viewer: &PyViewer,
                levels: Vec<PyLevelMetadata>,
                max_tiles: usize,
                atlas_count: usize,
            ) -> PyResult<Self> {
                let renderer = viewer.renderer.as_ref()
                    .ok_or_else(|| pyo3::exceptions::PyRuntimeError::new_err("Viewer not initialized"))?;

                if levels.is_empty() {
                    return Err(pyo3::exceptions::PyValueError::new_err(
                        "Must provide at least one LOD level",
                    ));
                }

                let rust_levels = levels.iter().map(PyLevelMetadata::to_lod_level_config).collect();

                let visual = $rust_ty::new(
                    renderer.device(),
                    renderer.queue(),
                    renderer.surface_format(),
                    renderer.camera_bind_group_layout(),
                    rust_levels,
                    max_tiles,
                    atlas_count,
                );

                let pending_chunks = visual.pending_chunks().unwrap();
                let wanted = visual.wanted_handle();
                let inner = Arc::new(Mutex::new(visual));
                Ok(Self { inner, pending_chunks, wanted })
            }

            /// Snapshot of the tile keys bovista currently wants in the atlas.
            /// Returned as `[(lod, t, z, y, x, priority), ...]` sorted by
            /// priority (lower = more urgent; 0 = current t, positive = prefetch).
            fn wanted_keys(&self) -> Vec<(usize, u32, u32, u32, u32, i32)> {
                crate::visuals::virtual_texture::wanted_sorted(&self.wanted)
            }

            fn set_contrast(&self, min: f32, max: f32) -> PyResult<()> {
                bindings_common::with_visual_mut::<$rust_ty, _, _>(
                    &self.inner, |v| v.set_contrast_limits(min, max)
                ).map_err(pyo3::exceptions::PyTypeError::new_err)
            }

            /// Set a colormap LUT. `rgba` is a (256, 4) uint8 array; pass None
            /// or an empty array to reset to grayscale.
            #[pyo3(signature = (rgba=None))]
            fn set_colormap(&self, rgba: Option<PyReadonlyArray3<u8>>) -> PyResult<()> {
                let bytes: Vec<u8> = match rgba {
                    Some(arr) => arr.as_array().iter().copied().collect(),
                    None => Vec::new(),
                };
                bindings_common::with_visual_mut::<$rust_ty, _, _>(
                    &self.inner, |v| v.set_colormap(&bytes)
                ).map_err(pyo3::exceptions::PyTypeError::new_err)
            }

            /// LOD bias: positive = prefer finer LOD, negative = prefer coarser.
            fn set_lod_bias(&self, bias: f32) -> PyResult<()> {}

            /// Set the blend mode (Normal or Additive). Additive enables
            /// order-independent multi-channel compositing.
            fn set_blend_mode(&self, mode: PyBlendMode) -> PyResult<()> {
                bindings_common::with_visual_mut::<$rust_ty, _, _>(&self.inner, |v| v.set_blend_mode(mode.into()))
                    .map_err(pyo3::exceptions::PyTypeError::new_err)
            }

            /// Set per-visual opacity in [0, 1].
            fn set_opacity(&self, opacity: f32) -> PyResult<()> {
                bindings_common::with_visual_mut::<$rust_ty, _, _>(&self.inner, |v| v.set_opacity(opacity))
                    .map_err(pyo3::exceptions::PyTypeError::new_err)
            }

            /// Provide uint16 tile data (full u16 range maps to [0, 1]).
            fn set_chunk_data_u16(
                &self,
                lod_level: usize, t: u32, z: u32, y: u32, x: u32,
                data: PyReadonlyArray3<u16>,
            ) -> PyResult<()> {
                let a = data.as_array();
                let tile = $pack16(
                    a.iter().copied(),
                    a.shape()[0] as u32, a.shape()[1] as u32, a.shape()[2] as u32,
                );
                self.pending_chunks.lock().unwrap()
                    .insert(crate::visuals::gpu_structs::TileKey { lod_level, t, z, y, x }, tile);
                Ok(())
            }

            /// Provide uint8 tile data (full u8 range maps to [0, 1]).
            fn set_chunk_data_u8(
                &self,
                lod_level: usize, t: u32, z: u32, y: u32, x: u32,
                data: PyReadonlyArray3<u8>,
            ) -> PyResult<()> {
                let a = data.as_array();
                let tile = $pack8(
                    a.iter().copied(),
                    a.shape()[0] as u32, a.shape()[1] as u32, a.shape()[2] as u32,
                );
                self.pending_chunks.lock().unwrap()
                    .insert(crate::visuals::gpu_structs::TileKey { lod_level, t, z, y, x }, tile);
                Ok(())
            }

            /// Returns (loaded_tiles, visible_tiles).
            fn get_stats(&self) -> PyResult<(usize, usize)> {}

            $($($extra)*)?
        }
    };
}

py_vt_visual!(PyImage, "Image", Image, extra: {
    /// Set the slice plane position along the Z axis.
    fn set_slice_z(&self, z: f32) -> PyResult<()> {}
    /// Set the slice plane position along the Y axis.
    fn set_slice_y(&self, y: f32) -> PyResult<()> {}
    /// Set the slice plane position along the X axis.
    fn set_slice_x(&self, x: f32) -> PyResult<()> {}

    /// Set an arbitrary slice plane from a point and normal.
    fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> PyResult<()> {
        bindings_common::with_visual_mut::<Image, _, _>(
            &self.inner,
            |v| v.set_slice_plane(SlicePlane::new([px, py, pz], [nx, ny, nz]))
        ).map_err(pyo3::exceptions::PyTypeError::new_err)
    }

    /// Set an oblique slice plane from two angles + an offset about `center`,
    /// returning the resulting unit normal `(nx, ny, nz)` — pass it to
    /// `Viewer.align_camera_to_slice` to face the slice head-on.
    fn set_slice_from_angles(
        &self, cx: f32, cy: f32, cz: f32, angle_x: f32, angle_y: f32, offset: f32,
    ) -> PyResult<(f32, f32, f32)> {
        let plane = SlicePlane::from_angles([cx, cy, cz], angle_x, angle_y, offset);
        bindings_common::with_visual_mut::<Image, _, _>(
            &self.inner,
            |v| { v.set_slice_plane(plane); (plane.normal[0], plane.normal[1], plane.normal[2]) }
        ).map_err(pyo3::exceptions::PyTypeError::new_err)
    }

    /// Enable or disable debug LOD tinting.
    fn set_debug_mode(&self, enabled: bool) -> PyResult<()> {}
});

// Labels: a 2D segmentation-mask visual. Same slice-plane surface as Image,
// but tile data is packed by magnitude (integer IDs, not normalized) and the
// shader renders each ID as a flat hashed color (ID 0 transparent). `set_contrast`
// is inherited from the shared macro but has no effect in label color mode.
py_vt_visual!(@full PyLabels, "Labels", Labels,
    bindings_common::pack_u16_label_tile, bindings_common::pack_u8_label_tile,
    extra: {
        /// Set the slice plane position along the Z axis.
        fn set_slice_z(&self, z: f32) -> PyResult<()> {}
        /// Set the slice plane position along the Y axis.
        fn set_slice_y(&self, y: f32) -> PyResult<()> {}
        /// Set the slice plane position along the X axis.
        fn set_slice_x(&self, x: f32) -> PyResult<()> {}

        /// Set an arbitrary slice plane from a point and normal.
        fn set_slice_plane(&self, px: f32, py: f32, pz: f32, nx: f32, ny: f32, nz: f32) -> PyResult<()> {
            bindings_common::with_visual_mut::<Labels, _, _>(
                &self.inner,
                |v| v.set_slice_plane(SlicePlane::new([px, py, pz], [nx, ny, nz]))
            ).map_err(pyo3::exceptions::PyTypeError::new_err)
        }

        /// Set an oblique slice plane from two angles + an offset about `center`,
        /// returning the resulting unit normal `(nx, ny, nz)`.
        fn set_slice_from_angles(
            &self, cx: f32, cy: f32, cz: f32, angle_x: f32, angle_y: f32, offset: f32,
        ) -> PyResult<(f32, f32, f32)> {
            let plane = SlicePlane::from_angles([cx, cy, cz], angle_x, angle_y, offset);
            bindings_common::with_visual_mut::<Labels, _, _>(
                &self.inner,
                |v| { v.set_slice_plane(plane); (plane.normal[0], plane.normal[1], plane.normal[2]) }
            ).map_err(pyo3::exceptions::PyTypeError::new_err)
        }

        /// Reshuffle the label→color mapping (napari "shuffle colors"). Any
        /// distinct value recolors the segmentation without touching the data.
        fn set_label_seed(&self, seed: f32) -> PyResult<()> {}

        /// Provide uint16 label tile data (raw integer IDs; exact up to 2048).
        fn set_label_data_u16(
            &self,
            lod_level: usize, t: u32, z: u32, y: u32, x: u32,
            data: PyReadonlyArray3<u16>,
        ) -> PyResult<()> {
            self.set_chunk_data_u16(lod_level, t, z, y, x, data)
        }

        /// Provide uint8 label tile data (raw integer IDs).
        fn set_label_data_u8(
            &self,
            lod_level: usize, t: u32, z: u32, y: u32, x: u32,
            data: PyReadonlyArray3<u8>,
        ) -> PyResult<()> {
            self.set_chunk_data_u8(lod_level, t, z, y, x, data)
        }

        /// Enable or disable debug LOD tinting.
        fn set_debug_mode(&self, enabled: bool) -> PyResult<()> {}
    }
);

py_vt_visual!(PyDirectVolume, "DirectVolume", DirectVolume, extra: {
    /// Step size in LOD-0 voxels (1.0 = Nyquist at finest LOD).
    fn set_relative_step_size(&self, step: f32) -> PyResult<()> {}
    /// Per-step extinction multiplier. Higher = denser/more opaque volume.
    fn set_density_scale(&self, scale: f32) -> PyResult<()> {}
    /// Front-to-back accumulation cutoff (default 0.95).
    fn set_early_exit_alpha(&self, alpha: f32) -> PyResult<()> {}
    /// Enable LOD tinting + wireframes (mode 1). False restores normal DVR.
    fn set_debug_mode(&self, enabled: bool) -> PyResult<()> {}
    /// Bypass page table and sample raw atlas (mode 2). False restores normal DVR.
    fn set_atlas_debug_mode(&self, enabled: bool) -> PyResult<()> {}
    /// Step-count heatmap (mode 3). False restores normal DVR.
    fn set_step_debug_mode(&self, enabled: bool) -> PyResult<()> {}
});

py_vt_visual!(PyMipVolume, "MipVolume", MipVolume, extra: {
    /// Step size in LOD-0 voxels (1.0 = Nyquist at finest LOD).
    fn set_relative_step_size(&self, step: f32) -> PyResult<()> {}
    /// Attenuated-MIP falloff per accumulated normalised density.
    /// 0.0 = plain MIP; larger = stronger near-camera emphasis.
    fn set_attenuation(&self, attenuation: f32) -> PyResult<()> {}
});

py_vt_visual!(PyMinipVolume, "MinipVolume", MinipVolume, extra: {
    /// Step size in LOD-0 voxels (1.0 = Nyquist at finest LOD).
    fn set_relative_step_size(&self, step: f32) -> PyResult<()> {}
});
py_vt_visual!(PyAverageVolume, "AverageVolume", AverageVolume, extra: {
    /// Step size in LOD-0 voxels (1.0 = Nyquist at finest LOD).
    fn set_relative_step_size(&self, step: f32) -> PyResult<()> {}
});

py_vt_visual!(PyIsosurfaceVolume, "IsosurfaceVolume", IsosurfaceVolume, extra: {
    /// Step size in LOD-0 voxels (1.0 = Nyquist at finest LOD).
    fn set_relative_step_size(&self, step: f32) -> PyResult<()> {}
    /// Isosurface threshold in contrast-normalised raw units (0..1).
    fn set_iso_threshold(&self, threshold: f32) -> PyResult<()> {}
});

/// Python module definition
#[pymodule]
fn bovista(m: &Bound<'_, PyModule>) -> PyResult<()> {
    m.add_class::<PyViewer>()?;
    m.add_class::<PyPoints>()?;
    m.add_class::<PyLines>()?;
    m.add_class::<PyImage>()?;
    m.add_class::<PyLabels>()?;
    m.add_class::<PyDirectVolume>()?;
    m.add_class::<PyMipVolume>()?;
    m.add_class::<PyMinipVolume>()?;
    m.add_class::<PyAverageVolume>()?;
    m.add_class::<PyIsosurfaceVolume>()?;
    m.add_class::<PyLevelMetadata>()?;
    m.add_class::<PyCustom>()?;
    m.add_class::<PyVertexBufferLayout>()?;
    m.add_class::<PyProjectionMode>()?;
    m.add_class::<PyBlendMode>()?;
    Ok(())
}
