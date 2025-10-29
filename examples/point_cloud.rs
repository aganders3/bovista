use bovista::{Camera, LinesVisual, PointsVisual, Renderer, Scene};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

struct State {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    window: std::sync::Arc<winit::window::Window>,
    renderer: Renderer,
    camera: Camera,
    scene: Scene,
    depth_texture: wgpu::TextureView,
    mouse_pressed: bool,
    last_mouse_pos: winit::dpi::PhysicalPosition<f64>,
}

impl State {
    async fn new(window: std::sync::Arc<winit::window::Window>) -> Self {
        let mut size = window.inner_size();

        // Ensure we have a valid size (handle 0x0 window on WASM)
        if size.width == 0 || size.height == 0 {
            size.width = size.width.max(1);
            size.height = size.height.max(1);
        }

        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });

        let surface = instance.create_surface(window.clone()).unwrap();

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            })
            .await
            .unwrap();

        let (device, queue) = adapter
            .request_device(
                &wgpu::DeviceDescriptor {
                    label: None,
                    required_features: wgpu::Features::empty(),
                    required_limits: wgpu::Limits::default(),
                    memory_hints: Default::default(),
                },
                None,
            )
            .await
            .unwrap();

        let surface_caps = surface.get_capabilities(&adapter);
        let surface_format = surface_caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(surface_caps.formats[0]);

        // Prefer opaque alpha mode for better performance and compatibility
        let alpha_mode = surface_caps
            .alpha_modes
            .iter()
            .copied()
            .find(|mode| matches!(mode, wgpu::CompositeAlphaMode::Opaque))
            .unwrap_or(surface_caps.alpha_modes[0]);

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: size.width,
            height: size.height,
            present_mode: surface_caps.present_modes[0],
            alpha_mode,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&device, &config);

        let renderer = Renderer::new(device, queue, surface_format).await;

        let camera = Camera::new(size.width as f32 / size.height as f32);

        // Create a scene and add visuals to it
        let mut scene = Scene::new();

        // Add a test point cloud
        let points = PointsVisual::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            20,
        );

        // Add a wireframe cube outline
        let lines = LinesVisual::test_cube(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
        );

        // Add axis helper for orientation
        let axes = LinesVisual::axis_helper(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            1.5,
        );

        // Add visuals with appropriate wrapper type
        #[cfg(not(target_arch = "wasm32"))]
        {
            scene.add(Arc::new(Mutex::new(points)));
            scene.add(Arc::new(Mutex::new(lines)));
            scene.add(Arc::new(Mutex::new(axes)));
        }

        #[cfg(target_arch = "wasm32")]
        {
            scene.add(Rc::new(RefCell::new(points)));
            scene.add(Rc::new(RefCell::new(lines)));
            scene.add(Rc::new(RefCell::new(axes)));
        }

        let depth_texture = renderer.create_depth_texture(size.width, size.height);

        Self {
            surface,
            config,
            size,
            window,
            renderer,
            camera,
            scene,
            depth_texture,
            mouse_pressed: false,
            last_mouse_pos: winit::dpi::PhysicalPosition::new(0.0, 0.0),
        }
    }

    fn resize(&mut self, new_size: winit::dpi::PhysicalSize<u32>) {
        if new_size.width > 0 && new_size.height > 0 {
            self.size = new_size;
            self.config.width = new_size.width;
            self.config.height = new_size.height;
            self.surface.configure(self.renderer.device(), &self.config);
            self.camera
                .update_aspect_ratio(new_size.width as f32 / new_size.height as f32);
            self.depth_texture = self
                .renderer
                .create_depth_texture(new_size.width, new_size.height);
        }
    }

    fn input(&mut self, event: &WindowEvent) -> bool {
        match event {
            WindowEvent::MouseInput { state, button, .. } => {
                if *button == MouseButton::Left {
                    self.mouse_pressed = *state == ElementState::Pressed;
                    return true;
                }
                false
            }
            WindowEvent::CursorMoved { position, .. } => {
                if self.mouse_pressed {
                    let delta_x = (position.x - self.last_mouse_pos.x) as f32 * 0.01;
                    let delta_y = (position.y - self.last_mouse_pos.y) as f32 * 0.01;
                    self.camera.orbit(delta_x, delta_y);
                }
                self.last_mouse_pos = *position;
                self.mouse_pressed
            }
            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => *y * 0.5,
                    MouseScrollDelta::PixelDelta(pos) => pos.y as f32 * 0.01,
                };
                self.camera.zoom(-scroll);
                true
            }
            _ => false,
        }
    }

    fn update(&mut self) {
        self.renderer.update_camera(&self.camera);
        self.scene
            .prepare(self.renderer.device(), self.renderer.queue());
    }

    fn render(&mut self) -> Result<(), wgpu::SurfaceError> {
        let output = self.surface.get_current_texture()?;
        let view = output
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());

        let clear_color = wgpu::Color {
            r: 0.1,
            g: 0.1,
            b: 0.1,
            a: 1.0,
        };

        self.renderer
            .render(&self.scene, &view, &self.depth_texture, clear_color);

        output.present();

        Ok(())
    }
}

#[cfg(target_arch = "wasm32")]
type AppState = std::rc::Rc<std::cell::RefCell<Option<State>>>;

#[cfg(not(target_arch = "wasm32"))]
struct App {
    state: Option<State>,
}

#[cfg(target_arch = "wasm32")]
struct App {
    state: AppState,
}

impl ApplicationHandler for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if self.state.is_none() {
                let window_attributes = Window::default_attributes()
                    .with_title("Bovista - Point Cloud Example");
                let window = std::sync::Arc::new(event_loop.create_window(window_attributes).unwrap());

                let state = pollster::block_on(State::new(window));
                self.state = Some(state);
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if self.state.borrow().is_none() {
                use winit::platform::web::WindowExtWebSys;

                let window_attributes = Window::default_attributes()
                    .with_title("Bovista - Point Cloud Example");
                let window = std::sync::Arc::new(event_loop.create_window(window_attributes).unwrap());

                // Insert canvas into DOM
                let web_window = web_sys::window().expect("no web window");
                let document = web_window.document().expect("no document");
                let canvas = window.canvas().expect("no canvas");

                use wasm_bindgen::JsCast;

                // Set canvas style to fill container
                canvas.style().set_property("width", "100%").ok();
                canvas.style().set_property("height", "100%").ok();
                canvas.style().set_property("display", "block").ok();

                let container = document.get_element_by_id("canvas-container")
                    .expect("no canvas-container");
                container.append_child(&canvas).ok();

                // Handle device pixel ratio for sharp rendering on Retina displays
                let canvas_element: web_sys::HtmlCanvasElement = canvas.dyn_into()
                    .expect("canvas not HtmlCanvasElement");

                // Remove aspect-ratio that winit might set
                canvas_element.style().remove_property("aspect-ratio").ok();

                let dpr = web_window.device_pixel_ratio();
                let rect = canvas_element.get_bounding_client_rect();
                // Round CSS size first, then multiply by DPR for consistent sizing
                let width = (rect.width().round() * dpr) as u32;
                let height = (rect.height().round() * dpr) as u32;

                canvas_element.set_width(width);
                canvas_element.set_height(height);

                // Force winit to use our canvas size BEFORE creating State
                let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(width, height));

                // Spawn async initialization
                let state_clone = self.state.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    // Small delay to let winit process the size change
                    gloo_timers::future::TimeoutFuture::new(10).await;

                    let state = State::new(window).await;
                    *state_clone.borrow_mut() = Some(state);

                    // Hide loading screen
                    if let Some(window) = web_sys::window() {
                        if let Some(document) = window.document() {
                            if let Some(loading) = document.get_element_by_id("loading") {
                                loading.set_attribute("style", "display: none;").ok();
                            }
                        }
                    }
                });
            }
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        _window_id: winit::window::WindowId,
        event: WindowEvent,
    ) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(state) = &mut self.state {
                if !state.input(&event) {
                    match event {
                        WindowEvent::CloseRequested
                        | WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    physical_key: PhysicalKey::Code(KeyCode::Escape),
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } => event_loop.exit(),
                        WindowEvent::Resized(physical_size) => {
                            state.resize(physical_size);
                        }
                        WindowEvent::RedrawRequested => {
                            state.update();
                            match state.render() {
                                Ok(_) => {}
                                Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
                                Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                                Err(_) => {} // Other surface errors can be ignored (transient)
                            }
                            state.window.request_redraw();
                        }
                        _ => {}
                    }
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(state) = &mut *self.state.borrow_mut() {
                if !state.input(&event) {
                    match event {
                        WindowEvent::CloseRequested
                        | WindowEvent::KeyboardInput {
                            event:
                                KeyEvent {
                                    physical_key: PhysicalKey::Code(KeyCode::Escape),
                                    state: ElementState::Pressed,
                                    ..
                                },
                            ..
                        } => event_loop.exit(),
                        WindowEvent::Resized(physical_size) => {
                            // On WASM, update canvas size to match container
                            if let Some(web_window) = web_sys::window() {
                                if let Some(canvas_element) = web_window.document()
                                    .and_then(|doc| doc.query_selector("canvas").ok()?)
                                {
                                    use wasm_bindgen::JsCast;
                                    if let Ok(canvas) = canvas_element.dyn_into::<web_sys::HtmlCanvasElement>() {
                                        let dpr = web_window.device_pixel_ratio();
                                        let rect = canvas.get_bounding_client_rect();
                                        // Round CSS size first, then multiply by DPR for consistent sizing
                                        let width = (rect.width().round() * dpr) as u32;
                                        let height = (rect.height().round() * dpr) as u32;

                                        canvas.set_width(width);
                                        canvas.set_height(height);
                                    }
                                }
                            }
                            state.resize(physical_size);
                        }
                        WindowEvent::RedrawRequested => {
                            state.update();
                            match state.render() {
                                Ok(_) => {}
                                Err(wgpu::SurfaceError::Lost) => state.resize(state.size),
                                Err(wgpu::SurfaceError::OutOfMemory) => event_loop.exit(),
                                Err(e) => web_sys::console::error_1(&format!("{:?}", e).into()),
                            }
                            state.window.request_redraw();
                        }
                        _ => {}
                    }
                }
            }
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Some(state) = &self.state {
                state.window.request_redraw();
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if let Some(state) = &*self.state.borrow() {
                state.window.request_redraw();
            }
        }
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    env_logger::init();

    let event_loop = EventLoop::new().unwrap();
    let mut app = App { state: None };

    event_loop.run_app(&mut app).unwrap();
}

#[cfg(target_arch = "wasm32")]
fn main() {
    std::panic::set_hook(Box::new(console_error_panic_hook::hook));
    console_log::init().expect("Couldn't initialize logger");

    use winit::platform::web::EventLoopExtWebSys;

    let event_loop = EventLoop::new().unwrap();
    let app = App {
        state: std::rc::Rc::new(std::cell::RefCell::new(None)),
    };

    event_loop.spawn_app(app);
}

#[cfg(target_arch = "wasm32")]
#[wasm_bindgen(start)]
pub fn start() {
    main();
}
