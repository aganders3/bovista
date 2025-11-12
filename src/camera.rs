//! Camera system for 3D navigation and projection.
//!
//! Provides a perspective camera with intuitive controls for visualizing 3D data:
//! - Orbit: rotate around a target point
//! - Zoom: move closer/farther from target
//! - Pan: translate the view target
//! - Frustum culling: test visibility of objects

use glam::{Mat4, Vec3, Vec4};

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
    /// Uses exponential scaling for smooth, consistent zoom at all distances
    pub fn zoom(&mut self, delta: f32) {
        let direction = (self.position - self.target).normalize();
        let distance = (self.position - self.target).length();

        // Exponential zoom: each delta unit multiplies distance by a factor
        // Very small factor for smooth, fine control
        let zoom_sensitivity = 0.01_f32; // 1% change per unit
        let scale = (1.0_f32 + zoom_sensitivity).powf(delta);
        let new_distance = (distance * scale).max(0.1);

        self.position = self.target + direction * new_distance;
    }
}

impl Default for Camera {
    fn default() -> Self {
        Self::new(1.0)
    }
}

/// Six planes that define the camera's view frustum
///
/// Each plane is represented as Vec4(a, b, c, d) where the plane equation is:
/// ax + by + cz + d = 0
///
/// Points are inside the frustum if they're on the positive side of all planes.
#[derive(Clone, Debug)]
pub struct FrustumPlanes {
    /// Left plane (points to the right are inside)
    pub left: Vec4,
    /// Right plane (points to the left are inside)
    pub right: Vec4,
    /// Top plane (points below are inside)
    pub top: Vec4,
    /// Bottom plane (points above are inside)
    pub bottom: Vec4,
    /// Near plane (points farther are inside)
    pub near: Vec4,
    /// Far plane (points closer are inside)
    pub far: Vec4,
}

impl FrustumPlanes {
    /// Extract frustum planes from a view-projection matrix
    ///
    /// Uses the Gribb-Hartmann method: planes can be extracted directly from
    /// the rows of the VP matrix.
    pub fn from_view_projection_matrix(vp: &Mat4) -> Self {
        // Extract rows of the matrix
        let row0 = vp.row(0);
        let row1 = vp.row(1);
        let row2 = vp.row(2);
        let row3 = vp.row(3);

        // Extract and normalize planes
        let left = normalize_plane(row3 + row0);
        let right = normalize_plane(row3 - row0);
        let bottom = normalize_plane(row3 + row1);
        let top = normalize_plane(row3 - row1);
        let near = normalize_plane(row3 + row2);
        let far = normalize_plane(row3 - row2);

        Self {
            left,
            right,
            top,
            bottom,
            near,
            far,
        }
    }

    /// Test if an axis-aligned bounding box intersects this frustum
    ///
    /// Returns true if the AABB is fully or partially inside the frustum.
    /// Uses the "p-vertex" method for efficiency.
    pub fn contains_aabb(&self, min: Vec3, max: Vec3) -> bool {
        // Test against each plane
        for plane in [&self.left, &self.right, &self.top, &self.bottom, &self.near, &self.far] {
            // Find the "positive vertex" - the corner of the AABB most in the direction of the plane normal
            let p_vertex = Vec3::new(
                if plane.x >= 0.0 { max.x } else { min.x },
                if plane.y >= 0.0 { max.y } else { min.y },
                if plane.z >= 0.0 { max.z } else { min.z },
            );

            // If the p-vertex is outside (negative side) of this plane, the whole AABB is outside
            if plane.x * p_vertex.x + plane.y * p_vertex.y + plane.z * p_vertex.z + plane.w < 0.0 {
                return false;
            }
        }

        true
    }

    /// Test if an AABB is visible within the frustum, ignoring near/far planes
    ///
    /// This is useful for slice plane visualizations where you want to show
    /// chunks at any depth, and the slice plane intersection handles depth culling.
    /// Only tests against the side planes (left, right, top, bottom).
    pub fn contains_aabb_xy(&self, min: Vec3, max: Vec3) -> bool {
        // Only test against side planes, skip near/far
        for plane in [&self.left, &self.right, &self.top, &self.bottom] {
            let p_vertex = Vec3::new(
                if plane.x >= 0.0 { max.x } else { min.x },
                if plane.y >= 0.0 { max.y } else { min.y },
                if plane.z >= 0.0 { max.z } else { min.z },
            );

            if plane.x * p_vertex.x + plane.y * p_vertex.y + plane.z * p_vertex.z + plane.w < 0.0 {
                return false;
            }
        }

        true
    }
}

impl Camera {
    /// Get the frustum planes for this camera
    ///
    /// The frustum defines the visible region in world space. Objects outside
    /// the frustum can be culled (not rendered).
    pub fn frustum_planes(&self) -> FrustumPlanes {
        FrustumPlanes::from_view_projection_matrix(&self.view_projection_matrix())
    }
}

/// Normalize a plane equation to unit normal length
///
/// Plane equation: ax + by + cz + d = 0
/// After normalization, (a, b, c) has length 1
fn normalize_plane(plane: Vec4) -> Vec4 {
    let length = (plane.x * plane.x + plane.y * plane.y + plane.z * plane.z).sqrt();
    if length > 0.0 {
        plane / length
    } else {
        plane
    }
}
