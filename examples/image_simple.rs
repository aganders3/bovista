use bovista::{Camera, ImageVisual, LinesVisual, Renderer, Scene, SlicePlane};

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};

#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;

use winit::{
    application::ApplicationHandler,
    event::*,
    event_loop::{ActiveEventLoop, EventLoop},
    keyboard::{KeyCode, PhysicalKey},
    window::Window,
};

#[cfg(target_arch = "wasm32")]
use wasm_bindgen::prelude::*;

#[cfg(not(target_arch = "wasm32"))]
type VisualRef = std::sync::Arc<std::sync::Mutex<dyn bovista::Visual>>;

#[cfg(target_arch = "wasm32")]
type VisualRef = std::rc::Rc<std::cell::RefCell<dyn bovista::Visual>>;

struct State {
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    size: winit::dpi::PhysicalSize<u32>,
    window: std::sync::Arc<winit::window::Window>,
    renderer: Renderer,
    camera: Camera,
    scene: Scene,
    image_visual: VisualRef,  // Keep reference to update slice plane
    depth_texture: wgpu::TextureView,
    mouse_pressed: bool,
    last_mouse_pos: winit::dpi::PhysicalPosition<f64>,
    current_slice: f32,
    slice_angle: f32,
}

impl State {
    async fn new(window: std::sync::Arc<winit::window::Window>) -> Self {
        let mut size = window.inner_size();

        #[cfg(target_arch = "wasm32")]
        web_sys::console::log_1(
            &format!(
                "State::new - window.inner_size(): {}x{}",
                size.width, size.height
            )
            .into(),
        );

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

        // Create camera positioned to view a 256³ volume
        let mut camera = Camera::new(size.width as f32 / size.height as f32);
        camera.position = glam::Vec3::new(0.0, 0.0, 500.0); // Pull back to see 256-unit volume
        camera.target = glam::Vec3::new(128.0, 128.0, 128.0); // Look at volume center
        camera.far = 2000.0; // Extend far plane to see geometry at Z=500

        // Create a scene and add visuals to it
        let mut scene = Scene::new();

        // Generate synthetic 3D volume data with more structure
        let width = 256u32;
        let height = 256u32;
        let depth = 256u32;
        let mut volume_data = vec![0u8; (width * height * depth) as usize];

        println!("Generating 256³ volume... (this may take a moment)");

        for z in 0..depth {
            for y in 0..height {
                for x in 0..width {
                    let idx = ((z * height + y) * width + x) as usize;

                    let cx = x as f32 - width as f32 / 2.0;
                    let cy = y as f32 - height as f32 / 2.0;
                    let cz = z as f32 - depth as f32 / 2.0;
                    let dist = (cx * cx + cy * cy + cz * cz).sqrt();
                    let max_dist = width as f32 / 2.0;

                    // Create a more interesting phantom with multiple asymmetric features:
                    // 1. Main sphere with gradient
                    let sphere_value = ((1.0 - (dist / max_dist).min(1.0)) * 180.0) as u8;

                    // 2. Add some spherical shells (rings)
                    let shell1_dist = (dist - 40.0).abs();
                    let shell1 = if shell1_dist < 5.0 { 200 } else { 0 };

                    let shell2_dist = (dist - 70.0).abs();
                    let shell2 = if shell2_dist < 3.0 { 255 } else { 0 };

                    // 3. Add a bright core
                    let core_value = if dist < 20.0 { 255 } else { 0 };

                    // 4. Add a bright cylinder along X axis (offset to the right)
                    let cyl_y = cy;
                    let cyl_z = cz;
                    let cyl_dist = (cyl_y * cyl_y + cyl_z * cyl_z).sqrt();
                    let cylinder = if cx > 10.0 && cx < 80.0 && cyl_dist < 15.0 { 220 } else { 0 };

                    // 5. Add a small bright sphere offset in +Y direction
                    let sphere2_cx = cx;
                    let sphere2_cy = cy - 50.0;
                    let sphere2_cz = cz;
                    let sphere2_dist = (sphere2_cx * sphere2_cx + sphere2_cy * sphere2_cy + sphere2_cz * sphere2_cz).sqrt();
                    let small_sphere = if sphere2_dist < 25.0 { 200 } else { 0 };

                    // 6. Add a rectangular box in -X, -Z quadrant
                    let box_feature = if cx > -70.0 && cx < -30.0 &&
                                         cy > -20.0 && cy < 20.0 &&
                                         cz > -70.0 && cz < -30.0 { 180 } else { 0 };

                    // Combine features (max value)
                    let value = sphere_value
                        .max(shell1)
                        .max(shell2)
                        .max(core_value)
                        .max(cylinder)
                        .max(small_sphere)
                        .max(box_feature);
                    volume_data[idx] = value;
                }
            }
        }

        println!("Volume generated! Creating ImageVisual...");

        // Create ImageVisual with the 3D volume data
        // For now we'll use the simple strategy - later we can demonstrate chunked loading
        let mut image = ImageVisual::new_3d(
            renderer.device(),
            renderer.queue(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            &volume_data,
            width,
            height,
            depth,
        );

        // Set initial slice plane at center of volume
        // SlicePlane needs to be positioned in world space at the volume center
        image.set_slice_plane(SlicePlane::new([128.0, 128.0, 128.0], [0.0, 0.0, 1.0]));

        println!("ImageVisual created with slice at center (128, 128, 128)");

        // Add axis helper for orientation (scaled to match volume)
        let axes = LinesVisual::axis_helper(
            renderer.device(),
            renderer.surface_format(),
            renderer.camera_bind_group_layout(),
            200.0, // Slightly larger than volume for visibility
        );

        // Add visuals with appropriate wrapper type
        let image_visual_ref;
        #[cfg(not(target_arch = "wasm32"))]
        {
            let image_ref = Arc::new(Mutex::new(image));
            image_visual_ref = image_ref.clone();
            scene.add(image_ref);
            scene.add(Arc::new(Mutex::new(axes)));
        }

        #[cfg(target_arch = "wasm32")]
        {
            let image_ref = Rc::new(RefCell::new(image));
            image_visual_ref = image_ref.clone();
            scene.add(image_ref);
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
            image_visual: image_visual_ref,
            depth_texture,
            mouse_pressed: false,
            last_mouse_pos: winit::dpi::PhysicalPosition::new(0.0, 0.0),
            current_slice: 0.0, // Offset from center (128, 128, 128)
            slice_angle: 0.0,
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
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(key_code),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } => {
                match key_code {
                    KeyCode::ArrowUp => {
                        // Rotate slice plane
                        self.slice_angle += 0.1;
                        self.update_slice_plane();
                        true
                    }
                    KeyCode::ArrowDown => {
                        // Rotate slice plane
                        self.slice_angle -= 0.1;
                        self.update_slice_plane();
                        true
                    }
                    KeyCode::ArrowLeft => {
                        // Move slice backward
                        self.current_slice -= 5.0;
                        self.update_slice_plane();
                        true
                    }
                    KeyCode::ArrowRight => {
                        // Move slice forward
                        self.current_slice += 5.0;
                        self.update_slice_plane();
                        true
                    }
                    KeyCode::KeyR => {
                        // Reset to axis-aligned
                        self.slice_angle = 0.0;
                        self.current_slice = 0.0;
                        self.update_slice_plane();
                        true
                    }
                    _ => false,
                }
            }
            _ => false,
        }
    }

    fn update_slice_plane(&mut self) {
        // Calculate normal based on angle (rotate around X axis)
        let normal = [
            0.0,
            self.slice_angle.sin(),
            self.slice_angle.cos(),
        ];

        // Position the plane in the volume
        // The position should be in world coordinates, centered at volume center
        let plane = SlicePlane::new(
            [128.0, 128.0, 128.0 + self.current_slice],
            normal
        );

        println!("Updating slice plane: pos={:?}, normal={:?}, angle={}",
                 plane.position, plane.normal, self.slice_angle);

        // Update the image visual's slice plane
        #[cfg(not(target_arch = "wasm32"))]
        {
            if let Ok(mut visual) = self.image_visual.lock() {
                // Use Any trait to downcast to ImageVisual
                use std::any::Any;
                let visual_any: &mut dyn Any = &mut *visual;
                if let Some(image_visual) = visual_any.downcast_mut::<ImageVisual>() {
                    image_visual.set_slice_plane(plane);
                    println!("Slice plane updated successfully!");
                } else {
                    println!("Failed to downcast to ImageVisual!");
                }
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if let Ok(mut visual) = self.image_visual.try_borrow_mut() {
                use std::any::Any;
                let visual_any: &mut dyn Any = &mut *visual;
                if let Some(image_visual) = visual_any.downcast_mut::<ImageVisual>() {
                    image_visual.set_slice_plane(plane);
                    web_sys::console::log_1(&"Slice plane updated!".into());
                }
            }
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
                    .with_title("Bovista - Image Slice Example");
                let window =
                    std::sync::Arc::new(event_loop.create_window(window_attributes).unwrap());

                let state = pollster::block_on(State::new(window));
                self.state = Some(state);
            }
        }

        #[cfg(target_arch = "wasm32")]
        {
            if self.state.borrow().is_none() {
                use winit::platform::web::WindowExtWebSys;

                let window_attributes =
                    Window::default_attributes().with_title("Bovista - Image Slice Example");
                let window =
                    std::sync::Arc::new(event_loop.create_window(window_attributes).unwrap());

                let web_window = web_sys::window().expect("no web window");
                let document = web_window.document().expect("no document");
                let canvas = window.canvas().expect("no canvas");

                use wasm_bindgen::JsCast;

                canvas.style().set_property("width", "100%").ok();
                canvas.style().set_property("height", "100%").ok();
                canvas.style().set_property("display", "block").ok();

                let container = document
                    .get_element_by_id("canvas-container")
                    .expect("no canvas-container");
                container.append_child(&canvas).ok();

                let canvas_element: web_sys::HtmlCanvasElement =
                    canvas.dyn_into().expect("canvas not HtmlCanvasElement");

                canvas_element.style().remove_property("aspect-ratio").ok();

                let dpr = web_window.device_pixel_ratio();
                let rect = canvas_element.get_bounding_client_rect();
                let width = (rect.width().round() * dpr) as u32;
                let height = (rect.height().round() * dpr) as u32;

                canvas_element.set_width(width);
                canvas_element.set_height(height);

                let _ = window.request_inner_size(winit::dpi::PhysicalSize::new(width, height));

                let state_clone = self.state.clone();
                wasm_bindgen_futures::spawn_local(async move {
                    gloo_timers::future::TimeoutFuture::new(10).await;

                    let state = State::new(window).await;
                    *state_clone.borrow_mut() = Some(state);

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
                                Err(e) => eprintln!("{:?}", e),
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
                            if let Some(web_window) = web_sys::window() {
                                if let Some(canvas_element) = web_window
                                    .document()
                                    .and_then(|doc| doc.query_selector("canvas").ok()?)
                                {
                                    use wasm_bindgen::JsCast;
                                    if let Ok(canvas) =
                                        canvas_element.dyn_into::<web_sys::HtmlCanvasElement>()
                                    {
                                        let dpr = web_window.device_pixel_ratio();
                                        let rect = canvas.get_bounding_client_rect();
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
                                Err(e) => {
                                    web_sys::console::error_1(&format!("{:?}", e).into())
                                }
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
