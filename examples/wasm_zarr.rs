//! WASM-compatible remote Zarr viewer
//!
//! This example demonstrates loading remote Zarr datasets in the browser.
//! Chunks are fetched on-demand from JavaScript using zarrita.
//!
//! Build with:
//!   ./build_wasm.sh wasm_zarr

use bovista::{Camera, Renderer, Scene, TiledImageVisual, LinesVisual};
use bovista::visuals::tiled_image::{ChunkRequest, ChunkData};
use bovista::visuals::image::SlicePlane;
use bovista::visuals::lines::LineVertex;
use std::rc::Rc;
use std::cell::RefCell;
use std::collections::HashMap;
use wasm_bindgen::prelude::*;
use wasm_bindgen_futures::spawn_local;
use web_sys::{window, HtmlCanvasElement};
use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    window::Window,
    platform::web::WindowAttributesExtWebSys,
};

// Global chunk cache shared between JS and Rust
static mut CHUNK_CACHE: Option<Rc<RefCell<HashMap<(u32, u32, u32), Vec<u8>>>>> = None;

// Global volume and chunk dimensions set by init_dataset()
static mut VOLUME_SIZE: Option<(u32, u32, u32)> = None;
static mut CHUNK_SIZE: Option<(u32, u32, u32)> = None;

// Flag to signal when initial chunks are loaded and viewer should start
static mut DATASET_READY: bool = false;

// Track chunks that have been requested (to trigger JS loading)
static mut REQUESTED_CHUNKS: Option<Rc<RefCell<std::collections::HashSet<(u32, u32, u32)>>>> = None;

// Global reference to the app state for JS callbacks
static mut APP_STATE: Option<Rc<RefCell<Option<RendererState>>>> = None;

#[wasm_bindgen]
extern "C" {
    #[wasm_bindgen(js_namespace = console)]
    fn log(s: &str);

    // JS callback to request chunk loading
    #[wasm_bindgen(js_namespace = window, js_name = requestChunkLoad)]
    fn request_chunk_load(z: u32, y: u32, x: u32);
}

macro_rules! console_log {
    ($($t:tt)*) => (log(&format_args!($($t)*).to_string()))
}

struct RendererState {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    window: Rc<Window>,
    renderer: Renderer,
    camera: Camera,
    scene: Scene,
    depth_texture: wgpu::TextureView,
    left_mouse_pressed: bool,
    right_mouse_pressed: bool,
    shift_pressed: bool,
    last_mouse_pos: (f64, f64),
    slice_offset: f32,
    slice_angle_x: f32,
    slice_angle_y: f32,
    volume_center: glam::Vec3,
    debug_mode: bool,
    tiled_image: Option<Rc<RefCell<TiledImageVisual>>>,
}

struct App {
    state: Rc<RefCell<Option<RendererState>>>,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.state.borrow().is_none() {
            let web_window = window().expect("no global `window` exists");
            let document = web_window.document().expect("no document");
            let canvas = document
                .get_element_by_id("bovista-canvas")
                .expect("no canvas with id 'bovista-canvas'")
                .dyn_into::<HtmlCanvasElement>()
                .expect("element is not a canvas");

            // Ensure canvas has dimensions before proceeding
            let width = canvas.width();
            let height = canvas.height();

            if width == 0 || height == 0 {
                // Canvas not ready, try again later
                return;
            }

            let window_attributes = Window::default_attributes()
                .with_canvas(Some(canvas));

            let window = Rc::new(
                event_loop.create_window(window_attributes).unwrap()
            );

            // Spawn async initialization
            let window_clone = window.clone();
            let state_clone = self.state.clone();
            let actual_width = width;
            let actual_height = height;
            spawn_local(async move {
                // Use canvas dimensions directly since winit may return 0x0
                let size = window_clone.inner_size();
                let width = if size.width == 0 { actual_width } else { size.width };
                let height = if size.height == 0 { actual_height } else { size.height };

                let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
                    backends: wgpu::Backends::BROWSER_WEBGPU,
                    ..Default::default()
                });

                let surface = instance.create_surface(window_clone.clone()).unwrap();

                let adapter = instance.request_adapter(
                    &wgpu::RequestAdapterOptions {
                        power_preference: wgpu::PowerPreference::HighPerformance,
                        compatible_surface: Some(&surface),
                        force_fallback_adapter: false,
                    }
                ).await.unwrap();

                let (device, queue) = adapter.request_device(
                    &wgpu::DeviceDescriptor {
                        label: Some("Bovista Device"),
                        required_features: wgpu::Features::empty(),
                        required_limits: wgpu::Limits::downlevel_webgl2_defaults(),
                        memory_hints: Default::default(),
                    },
                    None,
                ).await.unwrap();

                let surface_caps = surface.get_capabilities(&adapter);
                let surface_format = surface_caps.formats[0];

                let config = wgpu::SurfaceConfiguration {
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
                    format: surface_format,
                    width: width.max(1),
                    height: height.max(1),
                    present_mode: surface_caps.present_modes[0],
                    alpha_mode: surface_caps.alpha_modes[0],
                    view_formats: vec![],
                    desired_maximum_frame_latency: 2,
                };

                surface.configure(&device, &config);

                let renderer = Renderer::new(device, queue, surface_format).await;
                let depth_texture = renderer.create_depth_texture(width.max(1), height.max(1));

                let mut camera = Camera::new(width as f32 / height as f32);
                camera.position = glam::Vec3::new(0.0, 0.0, 5.0);
                camera.target = glam::Vec3::ZERO;
                // Increase far plane for large volumes
                camera.far = 2000.0;


                *state_clone.borrow_mut() = Some(RendererState {
                    surface,
                    config,
                    window: window_clone,
                    renderer,
                    camera,
                    scene: Scene::new(),
                    depth_texture,
                    left_mouse_pressed: false,
                    right_mouse_pressed: false,
                    shift_pressed: false,
                    last_mouse_pos: (0.0, 0.0),
                    slice_offset: 0.0,
                    slice_angle_x: 0.0,
                    slice_angle_y: 0.0,
                    volume_center: glam::Vec3::ZERO,
                    debug_mode: false,
                    tiled_image: None,
                });
            });
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        if let Some(state) = &mut *self.state.borrow_mut() {
            match event {
                WindowEvent::CloseRequested => event_loop.exit(),
                WindowEvent::RedrawRequested => {
                    // Prepare tiled image chunks
                    if let Some(tiled) = &state.tiled_image {
                        let mut visual = tiled.borrow_mut();
                        visual.prepare_chunks(
                            state.renderer.device(),
                            state.renderer.queue(),
                            state.renderer.surface_format(),
                            state.renderer.camera_bind_group_layout(),
                        );
                    }

                    // Update camera and prepare scene
                    state.renderer.update_camera(&state.camera);
                    state.scene.prepare(state.renderer.device(), state.renderer.queue());

                    match state.surface.get_current_texture() {
                        Ok(output) => {
                            let view = output.texture.create_view(&wgpu::TextureViewDescriptor::default());

                            let clear_color = wgpu::Color {
                                r: 0.1,
                                g: 0.1,
                                b: 0.1,
                                a: 1.0,
                            };

                            state.renderer.render(&state.scene, &view, &state.depth_texture, clear_color);
                            output.present();
                        }
                        Err(wgpu::SurfaceError::Lost) => {
                            state.surface.configure(state.renderer.device(), &state.config);
                        }
                        Err(wgpu::SurfaceError::OutOfMemory) => {
                            event_loop.exit();
                        }
                        Err(_) => {}
                    }

                    state.window.request_redraw();
                }
                WindowEvent::Resized(physical_size) => {
                    if physical_size.width > 0 && physical_size.height > 0 {
                        state.config.width = physical_size.width;
                        state.config.height = physical_size.height;
                        state.surface.configure(state.renderer.device(), &state.config);
                        state.camera.update_aspect_ratio(
                            physical_size.width as f32 / physical_size.height as f32
                        );
                        state.depth_texture = state.renderer.create_depth_texture(
                            physical_size.width,
                            physical_size.height
                        );
                    }
                }
                WindowEvent::ModifiersChanged(modifiers) => {
                    state.shift_pressed = modifiers.state().shift_key();
                }
                WindowEvent::MouseInput { state: button_state, button, .. } => {
                    match button {
                        MouseButton::Left => {
                            state.left_mouse_pressed = button_state == ElementState::Pressed;
                        }
                        MouseButton::Right => {
                            state.right_mouse_pressed = button_state == ElementState::Pressed;
                        }
                        _ => {}
                    }
                }
                WindowEvent::CursorMoved { position, .. } => {
                    let delta_x = (position.x - state.last_mouse_pos.0) as f32;
                    let delta_y = (position.y - state.last_mouse_pos.1) as f32;

                    if state.left_mouse_pressed {
                        // Left mouse: orbit camera
                        state.camera.orbit(delta_x * 0.01, delta_y * 0.01);
                    } else if state.right_mouse_pressed {
                        // Right mouse: rotate slice plane
                        if state.shift_pressed {
                            // Shift + right: rotate around X axis
                            state.slice_angle_x += delta_y * 0.01;
                        } else {
                            // Right drag: rotate around Y axis
                            state.slice_angle_y += delta_y * 0.01;
                        }
                        // Horizontal movement: offset along Z
                        state.slice_offset += delta_x * 0.5;
                        update_slice_plane(state);
                    }

                    state.last_mouse_pos = (position.x, position.y);
                }
                WindowEvent::MouseWheel { delta, .. } => {
                    let scroll = match delta {
                        MouseScrollDelta::LineDelta(_, y) => y * 10.0,
                        MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.1,
                    };
                    state.camera.zoom(-scroll);
                }
                WindowEvent::KeyboardInput { event, .. } => {
                    if event.state == ElementState::Pressed {
                        if let Some(key) = event.text.as_ref() {
                            if key == "d" || key == "D" {
                                if let Some(tiled) = &state.tiled_image {
                                    // Toggle debug mode
                                    state.debug_mode = !state.debug_mode;
                                    let mut visual = tiled.borrow_mut();
                                    visual.set_debug_mode(state.debug_mode);
                                    console_log!("Debug mode: {}", if state.debug_mode { "ON" } else { "OFF" });
                                }
                            }
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if let Some(state) = &mut *self.state.borrow_mut() {
            // Check if dataset is ready and we haven't initialized yet
            unsafe {
                if DATASET_READY && state.tiled_image.is_none() {
                    initialize_tiled_visual(state);
                }
            }

            state.window.request_redraw();
        }
    }
}

fn update_slice_plane(state: &mut RendererState) {
    if let Some(tiled) = &state.tiled_image {
        let mut visual = tiled.borrow_mut();

        // Calculate rotated normal (matching Python version)
        let cos_y = state.slice_angle_y.cos();
        let sin_y = state.slice_angle_y.sin();
        let cos_x = state.slice_angle_x.cos();
        let sin_x = state.slice_angle_x.sin();

        let mut normal_x = sin_y * cos_x;
        let mut normal_y = -sin_x;
        let mut normal_z = cos_y * cos_x;

        // Normalize
        let length = (normal_x * normal_x + normal_y * normal_y + normal_z * normal_z).sqrt();
        if length > 0.0001 {
            normal_x /= length;
            normal_y /= length;
            normal_z /= length;
        }

        // Position at volume center + offset along Z
        let center = state.volume_center;
        let position_z = center.z + state.slice_offset;

        visual.set_slice_plane(SlicePlane {
            position: [center.x, center.y, position_z],
            normal: [normal_x, normal_y, normal_z],
        });
    }
}

fn initialize_tiled_visual(state: &mut RendererState) {
    if state.tiled_image.is_none() {
        if let Some(tiled) = create_tiled_visual() {
            let tiled_ref = Rc::new(RefCell::new(tiled));
            state.scene.add(tiled_ref.clone());
            state.tiled_image = Some(tiled_ref);

            // Add coordinate axes for orientation
            unsafe {
                if let Some(volume_size) = VOLUME_SIZE {
                    let axis_length = volume_size.0.max(volume_size.1).max(volume_size.2) as f32 * 0.3;
                    let axes = create_axes(
                        state.renderer.device(),
                        state.renderer.surface_format(),
                        state.renderer.camera_bind_group_layout(),
                        axis_length
                    );
                    state.scene.add(Rc::new(RefCell::new(axes)));
                }
            }

            // Position camera to look at volume center (matching Python version)
            unsafe {
                if let Some(volume_size) = VOLUME_SIZE {
                    let center = glam::Vec3::new(
                        volume_size.2 as f32 / 2.0,  // X
                        volume_size.1 as f32 / 2.0,  // Y
                        volume_size.0 as f32 / 2.0,  // Z
                    );

                    // Store volume center for slice plane rotation
                    state.volume_center = center;

                    // Distance based on max of X and Y (not Z), matching Python
                    let xy_max = volume_size.1.max(volume_size.2) as f32;
                    let distance = xy_max * 1.0;

                    state.camera.target = center;
                    // Camera above the center, looking down (add to Z)
                    state.camera.position = glam::Vec3::new(center.x, center.y, center.z + distance);
                }
            }

            // Update slice plane with current angles/offset for the new visual
            update_slice_plane(state);
        }
    }
}

fn create_axes(
    device: &wgpu::Device,
    surface_format: wgpu::TextureFormat,
    camera_bind_group_layout: &wgpu::BindGroupLayout,
    length: f32,
) -> LinesVisual {
    let vertices = vec![
        // X axis (red)
        LineVertex { position: [0.0, 0.0, 0.0], color: [1.0, 0.0, 0.0] },
        LineVertex { position: [length, 0.0, 0.0], color: [1.0, 0.0, 0.0] },
        // Y axis (green)
        LineVertex { position: [0.0, 0.0, 0.0], color: [0.0, 1.0, 0.0] },
        LineVertex { position: [0.0, length, 0.0], color: [0.0, 1.0, 0.0] },
        // Z axis (blue)
        LineVertex { position: [0.0, 0.0, 0.0], color: [0.0, 0.0, 1.0] },
        LineVertex { position: [0.0, 0.0, length], color: [0.0, 0.0, 1.0] },
    ];

    LinesVisual::new(device, surface_format, camera_bind_group_layout, vertices)
}

#[wasm_bindgen]
pub fn init_chunk_cache() {
    unsafe {
        CHUNK_CACHE = Some(Rc::new(RefCell::new(HashMap::new())));
        REQUESTED_CHUNKS = Some(Rc::new(RefCell::new(std::collections::HashSet::new())));
    }
    console_log!("Chunk cache initialized");
}

#[wasm_bindgen]
pub fn store_chunk(z: u32, y: u32, x: u32, data: &[u8]) {
    unsafe {
        if let Some(cache) = &CHUNK_CACHE {
            let mut map = cache.borrow_mut();
            map.insert((z, y, x), data.to_vec());
            console_log!("Stored chunk ({},{},{}): {} bytes", z, y, x, data.len());
        }
    }
}

#[wasm_bindgen]
pub fn init_dataset(
    volume_z: u32,
    volume_y: u32,
    volume_x: u32,
    chunk_z: u32,
    chunk_y: u32,
    chunk_x: u32,
) {
    console_log!("Dataset initialized: {}x{}x{}, chunks: {}x{}x{}",
        volume_z, volume_y, volume_x, chunk_z, chunk_y, chunk_x);

    unsafe {
        VOLUME_SIZE = Some((volume_z, volume_y, volume_x));
        CHUNK_SIZE = Some((chunk_z, chunk_y, chunk_x));
    }
}

#[wasm_bindgen]
pub fn dataset_ready() {
    unsafe {
        DATASET_READY = true;
    }
    console_log!("Dataset ready signal received");
}

#[wasm_bindgen]
pub fn set_contrast_limits(min_val: f32, max_val: f32) {
    unsafe {
        if let Some(app_state) = &APP_STATE {
            if let Ok(state) = app_state.try_borrow_mut() {
                if let Some(renderer_state) = state.as_ref() {
                    if let Some(tiled) = &renderer_state.tiled_image {
                        if let Ok(mut visual) = tiled.try_borrow_mut() {
                            // Normalize values from 0-255 to 0-1
                            let normalized_min = min_val / 255.0;
                            let normalized_max = max_val / 255.0;
                            visual.set_contrast_limits(normalized_min, normalized_max);
                        }
                    }
                }
            }
        }
    }
}

#[wasm_bindgen]
pub fn get_slice_z() -> f32 {
    unsafe {
        if let Some(app_state) = &APP_STATE {
            if let Ok(state) = app_state.try_borrow() {
                if let Some(renderer_state) = state.as_ref() {
                    return renderer_state.volume_center.z + renderer_state.slice_offset;
                }
            }
        }
    }
    0.0
}

#[wasm_bindgen]
pub fn clear_dataset() {
    unsafe {
        DATASET_READY = false;
        CHUNK_CACHE = Some(Rc::new(RefCell::new(HashMap::new())));
        REQUESTED_CHUNKS = Some(Rc::new(RefCell::new(std::collections::HashSet::new())));

        // Clear the scene and tiled image visual
        if let Some(app_state) = &APP_STATE {
            if let Ok(mut state) = app_state.try_borrow_mut() {
                if let Some(renderer_state) = state.as_mut() {
                    // Clear all visuals from the scene
                    renderer_state.scene.clear();
                    // Clear the reference so a new one will be created
                    renderer_state.tiled_image = None;
                }
            }
        }
    }
}

/// Function to be called by the App to create the TiledImageVisual
fn create_tiled_visual() -> Option<TiledImageVisual> {
    unsafe {
        let volume_size = VOLUME_SIZE?;
        let chunk_size = CHUNK_SIZE?;

        // Create chunk loader that reads from the global cache
        // Note: This is safe in WASM since we're single-threaded
        let loader = std::sync::Arc::new(move |request: ChunkRequest| -> Option<ChunkData> {
            unsafe {
                let key = (request.chunk_z, request.chunk_y, request.chunk_x);

                // Check if chunk is in cache
                if let Some(cache) = &CHUNK_CACHE {
                    let map = cache.borrow();
                    if let Some(data) = map.get(&key) {
                        // Calculate actual chunk dimensions (may be smaller for edge chunks)
                        let actual_z = chunk_size.0.min(volume_size.0 - request.chunk_z * chunk_size.0);
                        let actual_y = chunk_size.1.min(volume_size.1 - request.chunk_y * chunk_size.1);
                        let actual_x = chunk_size.2.min(volume_size.2 - request.chunk_x * chunk_size.2);

                        return Some(ChunkData {
                            data: data.clone(),
                            width: actual_x,
                            height: actual_y,
                            depth: actual_z,
                        });
                    }
                }

                // Not in cache - track request and notify JS to load it
                if let Some(requested) = &REQUESTED_CHUNKS {
                    let mut set = requested.borrow_mut();
                    if set.insert(key) {
                        // Call JS callback immediately (non-blocking)
                        request_chunk_load(request.chunk_z, request.chunk_y, request.chunk_x);
                    }
                }

                None
            }
        });

        let mut tiled = TiledImageVisual::new(volume_size, chunk_size, loader, 300);

        // Set initial slice plane at center of volume
        let center_z = volume_size.0 as f32 / 2.0;
        tiled.set_slice_plane(SlicePlane {
            position: [0.0, 0.0, center_z],
            normal: [0.0, 0.0, 1.0],
        });

        Some(tiled)
    }
}

#[wasm_bindgen(start)]
pub fn start() {
    console_error_panic_hook::set_once();
    console_log::init_with_level(log::Level::Info).expect("could not initialize logger");

    init_chunk_cache();

    use winit::platform::web::EventLoopExtWebSys;

    let event_loop = EventLoop::new().unwrap();
    let state = Rc::new(RefCell::new(None));
    let app = App {
        state: state.clone(),
    };

    // Store global reference for JS callbacks
    unsafe {
        APP_STATE = Some(state);
    }

    event_loop.spawn_app(app);
}
