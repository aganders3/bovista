//! Camera system for 3D navigation and projection.
//!
//! Provides a perspective camera with intuitive controls for visualizing 3D data:
//! - Orbit: rotate around a target point
//! - Zoom: move closer/farther from target
//! - Pan: translate the view target

use glam::{Mat4, Vec3};

/// A perspective camera for 3D visualization with orbit controls.
///
/// The camera uses a target-based system where the camera orbits around a fixed
/// point in space (the target). This is ideal for inspecting 3D objects and volumes.
///
/// # Coordinate System
///
/// Bovista uses a right-handed coordinate system:
/// - +X: Right
/// - +Y: Up
/// - +Z: Forward (towards viewer)
///
/// # Example
///
/// ```rust
/// use bovista::Camera;
/// use glam::Vec3;
///
/// let mut camera = Camera::new(16.0 / 9.0);
/// camera.position = Vec3::new(0.0, 0.0, 10.0);
/// camera.target = Vec3::ZERO;
///
/// // Orbit around target
/// camera.orbit(0.1, 0.1);
///
/// // Zoom in/out
/// camera.zoom(-1.0);
/// ```
#[derive(Clone)]
pub struct Camera {
    pub position: Vec3,
    pub target: Vec3,
    pub up: Vec3,
    pub fov_y: f32,
    pub aspect_ratio: f32,
    pub near: f32,
    pub far: f32,
}

impl Camera {
    pub fn new(aspect_ratio: f32) -> Self {
        Self {
            position: Vec3::new(0.0, 0.0, 5.0),
            target: Vec3::ZERO,
            up: Vec3::Y,
            fov_y: 45.0_f32.to_radians(),
            aspect_ratio,
            near: 0.1,
            far: 100.0,
        }
    }

    pub fn view_matrix(&self) -> Mat4 {
        Mat4::look_at_rh(self.position, self.target, self.up)
    }

    pub fn projection_matrix(&self) -> Mat4 {
        Mat4::perspective_rh(self.fov_y, self.aspect_ratio, self.near, self.far)
    }

    pub fn view_projection_matrix(&self) -> Mat4 {
        self.projection_matrix() * self.view_matrix()
    }

    pub fn update_aspect_ratio(&mut self, aspect_ratio: f32) {
        self.aspect_ratio = aspect_ratio;
    }

    /// Orbit the camera around the target
    pub fn orbit(&mut self, delta_x: f32, delta_y: f32) {
        let radius = (self.position - self.target).length();

        // Convert to spherical coordinates
        let offset = self.position - self.target;
        let mut theta = offset.z.atan2(offset.x);
        let mut phi = (offset.y / radius).acos();

        // Apply rotation (X inverted for natural feel)
        theta += delta_x;
        phi = (phi - delta_y).clamp(0.1, std::f32::consts::PI - 0.1);

        // Convert back to cartesian
        let x = radius * phi.sin() * theta.cos();
        let y = radius * phi.cos();
        let z = radius * phi.sin() * theta.sin();

        self.position = self.target + Vec3::new(x, y, z);
    }

    /// Zoom by moving camera closer/farther from target
    pub fn zoom(&mut self, delta: f32) {
        let direction = (self.position - self.target).normalize();
        let distance = (self.position - self.target).length();
        let new_distance = (distance + delta).max(0.1);
        self.position = self.target + direction * new_distance;
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new(1.0)
    }
}
