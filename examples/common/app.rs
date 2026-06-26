//! Minimal winit-0.30 interactive harness shared by the native examples.
//!
//! Examples implement [`ExampleApp`]: `build` creates the visual(s) and adds
//! them to the scene; `on_key` / `on_secondary_drag` handle input; `print_stats`
//! emits a console line ~1 Hz; `controls_help` is printed at startup.
//!
//! The harness owns the `Arc<Window>` / `Surface<'static>` / `Renderer` /
//! `Scene` / `Camera`, runs the event loop, and per-frame does
//! `update_camera → CameraInfo → scene.prepare → get_current_texture →
//! render → present`. Left-drag orbits (pans in ortho), right/middle-drag and
//! the scroll wheel are forwarded, and keyboard presses go to `on_key`.
//!
//! `#[path]`-included into each example binary; self-contained (no `super::`).

#![allow(dead_code)]

use std::sync::Arc;
use std::time::Instant;

use bovista::visual::CameraInfo;
use bovista::{Camera, ProjectionMode, Renderer, Scene};

use winit::application::ApplicationHandler;
use winit::event::{ElementState, MouseButton, MouseScrollDelta, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{Key, NamedKey};
use winit::window::{Window, WindowId};

/// Which non-left mouse button drove a secondary drag.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum SecondaryButton {
    Right,
    Middle,
}

/// Context handed to [`ExampleApp::build`]. Borrows the harness's renderer,
/// scene, and camera so the example can construct visuals, add them, and frame
/// the camera.
pub struct BuildCtx<'a> {
    pub renderer: &'a Renderer,
    pub scene: &'a mut Scene,
    pub camera: &'a mut Camera,
    /// Initial viewport size in physical pixels.
    pub width: u32,
    pub height: u32,
}

/// Context handed to per-frame input callbacks.
pub struct FrameCtx<'a> {
    pub renderer: &'a Renderer,
    pub scene: &'a mut Scene,
    pub camera: &'a mut Camera,
    pub width: u32,
    pub height: u32,
}

/// Interface each native example implements.
pub trait ExampleApp {
    /// Build the visual(s), wire the loader slots, add to the scene, and frame
    /// the camera. Called once, after the GPU is up.
    fn build(&mut self, ctx: &mut BuildCtx);
    /// A key was pressed (logical key). Drag/zoom are handled by the harness.
    fn on_key(&mut self, key: &Key, ctx: &mut FrameCtx);
    /// Right- or middle-button drag, in pixels. `shift` reflects the modifier.
    #[allow(unused_variables)]
    fn on_secondary_drag(
        &mut self,
        dx: f32,
        dy: f32,
        shift: bool,
        btn: SecondaryButton,
        ctx: &mut FrameCtx,
    ) {
    }
    /// Emit a one-line console status (called ~1 Hz).
    fn print_stats(&mut self);
    /// Help text printed once at startup.
    fn controls_help(&self) -> String;
}

struct Harness<A: ExampleApp> {
    app: A,
    title: String,

    window: Option<Arc<Window>>,
    surface: Option<wgpu::Surface<'static>>,
    config: Option<wgpu::SurfaceConfiguration>,
    renderer: Option<Renderer>,
    depth_texture: Option<wgpu::TextureView>,
    scene: Scene,
    camera: Camera,
    built: bool,

    // Input state.
    shift: bool,
    left_pressed: bool,
    right_pressed: bool,
    middle_pressed: bool,
    last_mouse: (f64, f64),

    last_stats: Instant,
}

impl<A: ExampleApp> Harness<A> {
    fn new(app: A, title: String) -> Self {
        Self {
            app,
            title,
            window: None,
            surface: None,
            config: None,
            renderer: None,
            depth_texture: None,
            scene: Scene::new(),
            camera: Camera::new(1.0),
            built: false,
            shift: false,
            left_pressed: false,
            right_pressed: false,
            middle_pressed: false,
            last_mouse: (0.0, 0.0),
            last_stats: Instant::now(),
        }
    }

    fn size(&self) -> (u32, u32) {
        self.window
            .as_ref()
            .map(|w| {
                let s = w.inner_size();
                (s.width.max(1), s.height.max(1))
            })
            .unwrap_or((800, 600))
    }
}

impl<A: ExampleApp> ApplicationHandler for Harness<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if self.window.is_some() {
            return;
        }
        let attrs = Window::default_attributes().with_title(self.title.clone());
        let window = Arc::new(event_loop.create_window(attrs).unwrap());
        let size = window.inner_size();
        let (w, h) = (size.width.max(1), size.height.max(1));

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let surface = instance.create_surface(window.clone()).unwrap();
        let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            compatible_surface: Some(&surface),
            force_fallback_adapter: false,
        }))
        .expect("no GPU adapter found");
        let info = adapter.get_info();
        println!("[app] adapter: {} ({:?})", info.name, info.backend);

        let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
            label: Some("example-device"),
            required_features: wgpu::Features::empty(),
            required_limits: wgpu::Limits::default(),
            memory_hints: Default::default(),
            trace: wgpu::Trace::Off,
        }))
        .expect("request_device failed");

        let caps = surface.get_capabilities(&adapter);
        // Non-sRGB format + Opaque alpha so bovista's premultiplied output is
        // composited correctly (sRGB double-encode = washed/dark colors).
        let surface_format = caps
            .formats
            .iter()
            .copied()
            .find(|f| !f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: surface_format,
            width: w,
            height: h,
            present_mode: caps.present_modes[0],
            alpha_mode: wgpu::CompositeAlphaMode::Opaque,
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };

        let renderer = pollster::block_on(Renderer::new(device, queue, surface_format));
        surface.configure(renderer.device(), &config);
        let depth = renderer.create_depth_texture(w, h);

        self.camera.update_aspect_ratio(w as f32 / h as f32);

        // Build the example's visuals once.
        if !self.built {
            let mut ctx = BuildCtx {
                renderer: &renderer,
                scene: &mut self.scene,
                camera: &mut self.camera,
                width: w,
                height: h,
            };
            self.app.build(&mut ctx);
            self.built = true;
            println!("{}", self.app.controls_help());
        }

        self.renderer = Some(renderer);
        self.surface = Some(surface);
        self.config = Some(config);
        self.window = Some(window);
        self.depth_texture = Some(depth);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, _id: WindowId, event: WindowEvent) {
        match event {
            WindowEvent::CloseRequested => event_loop.exit(),

            WindowEvent::Resized(sz) => {
                if sz.width > 0 && sz.height > 0 {
                    if let (Some(surface), Some(config), Some(renderer)) =
                        (&self.surface, &mut self.config, &self.renderer)
                    {
                        config.width = sz.width;
                        config.height = sz.height;
                        surface.configure(renderer.device(), config);
                        self.camera
                            .update_aspect_ratio(sz.width as f32 / sz.height as f32);
                        self.depth_texture =
                            Some(renderer.create_depth_texture(sz.width, sz.height));
                    }
                }
            }

            WindowEvent::ModifiersChanged(mods) => {
                self.shift = mods.state().shift_key();
            }

            WindowEvent::KeyboardInput { event, .. } => {
                if event.state == ElementState::Pressed {
                    let (w, h) = self.size();
                    if let Some(renderer) = &self.renderer {
                        let mut ctx = FrameCtx {
                            renderer,
                            scene: &mut self.scene,
                            camera: &mut self.camera,
                            width: w,
                            height: h,
                        };
                        self.app.on_key(&event.logical_key, &mut ctx);
                    }
                }
            }

            WindowEvent::MouseInput { state, button, .. } => {
                let pressed = state == ElementState::Pressed;
                match button {
                    MouseButton::Left => self.left_pressed = pressed,
                    MouseButton::Right => self.right_pressed = pressed,
                    MouseButton::Middle => self.middle_pressed = pressed,
                    _ => {}
                }
            }

            WindowEvent::CursorMoved { position, .. } => {
                let dx = (position.x - self.last_mouse.0) as f32;
                let dy = (position.y - self.last_mouse.1) as f32;
                self.last_mouse = (position.x, position.y);

                if self.left_pressed {
                    match self.camera.projection_mode {
                        ProjectionMode::Perspective => {
                            self.camera.orbit(dx * 0.005, dy * 0.005);
                        }
                        ProjectionMode::Orthographic => {
                            let speed = self.camera.ortho_height * 0.002;
                            self.camera.pan(-dx * speed, dy * speed);
                        }
                    }
                } else if self.right_pressed || self.middle_pressed {
                    let btn = if self.right_pressed {
                        SecondaryButton::Right
                    } else {
                        SecondaryButton::Middle
                    };
                    let (w, h) = self.size();
                    let shift = self.shift;
                    if let Some(renderer) = &self.renderer {
                        let mut ctx = FrameCtx {
                            renderer,
                            scene: &mut self.scene,
                            camera: &mut self.camera,
                            width: w,
                            height: h,
                        };
                        self.app.on_secondary_drag(dx, dy, shift, btn, &mut ctx);
                    }
                }
            }

            WindowEvent::MouseWheel { delta, .. } => {
                let scroll = match delta {
                    MouseScrollDelta::LineDelta(_, y) => y * 10.0,
                    MouseScrollDelta::PixelDelta(p) => p.y as f32 * 0.1,
                };
                self.camera.zoom(-scroll);
            }

            WindowEvent::RedrawRequested => {
                self.redraw();
            }

            _ => {}
        }
    }

    fn about_to_wait(&mut self, _event_loop: &ActiveEventLoop) {
        if self.last_stats.elapsed().as_secs_f32() >= 1.0 {
            self.app.print_stats();
            self.last_stats = Instant::now();
        }
        if let Some(window) = &self.window {
            window.request_redraw();
        }
    }
}

impl<A: ExampleApp> Harness<A> {
    fn redraw(&mut self) {
        let (Some(surface), Some(renderer), Some(depth)) =
            (&self.surface, &self.renderer, &self.depth_texture)
        else {
            return;
        };
        let (w, h) = self.size();

        renderer.update_camera(&self.camera);
        let camera_info = CameraInfo {
            position: self.camera.position,
            target: self.camera.target,
            fov_y: self.camera.fov_y,
            viewport_width: w,
            viewport_height: h,
            frustum: self.camera.frustum_planes(),
            projection_mode: self.camera.projection_mode,
            ortho_height: self.camera.ortho_height,
            view_proj: self.camera.view_projection_matrix(),
        };
        self.scene
            .prepare(renderer.device(), renderer.queue(), &camera_info);

        match surface.get_current_texture() {
            Ok(output) => {
                let view = output
                    .texture
                    .create_view(&wgpu::TextureViewDescriptor::default());
                renderer.render(
                    &self.scene,
                    &view,
                    depth,
                    wgpu::Color {
                        r: 0.0,
                        g: 0.0,
                        b: 0.0,
                        a: 1.0,
                    },
                );
                output.present();
            }
            Err(wgpu::SurfaceError::Lost) | Err(wgpu::SurfaceError::Outdated) => {
                if let Some(config) = &self.config {
                    surface.configure(renderer.device(), config);
                }
            }
            Err(e) => eprintln!("[app] surface error: {:?}", e),
        }
    }
}

/// Run the event loop with the given example app and window title. Blocks until
/// the window closes.
pub fn run<A: ExampleApp + 'static>(app: A, title: &str) {
    let event_loop = EventLoop::new().expect("failed to create event loop");
    let mut harness = Harness::new(app, title.to_string());
    event_loop
        .run_app(&mut harness)
        .expect("event loop error");
}

/// Helper: was a printable character key pressed? Examples match on this for
/// single-letter shortcuts.
pub fn char_key(key: &Key) -> Option<char> {
    match key {
        Key::Character(s) => s.chars().next(),
        Key::Named(NamedKey::Space) => Some(' '),
        _ => None,
    }
}
