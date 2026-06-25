//! Camera sizing helpers shared by the native examples. Derive a sensible
//! orbit radius and near/far clip planes from a volume's world extents so data
//! with arbitrary voxel units (nm, µm, mm, m) all fit comfortably.
//!
//! `#[path]`-included into each example binary; self-contained (no `super::`).

#![allow(dead_code)]

use bovista::{Camera, ProjectionMode};

/// Largest of the (z, y, x) world extents.
pub fn world_max(extents: (f32, f32, f32)) -> f32 {
    let (wz, wy, wx) = extents;
    wz.max(wy).max(wx)
}

/// Configure a perspective camera centered on the origin (the OME-Zarr loader
/// centers the world frame there) looking at a volume of the given world
/// extents. Returns the camera; `radius_mult` scales the orbit distance.
pub fn camera_for_extents(aspect: f32, extents: (f32, f32, f32), radius_mult: f32) -> Camera {
    let wmax = world_max(extents);
    let mut camera = Camera::new(aspect);
    camera.target = glam::Vec3::ZERO;
    camera.up = glam::Vec3::Y;
    camera.fov_y = 45.0_f32.to_radians();
    // Scale clip planes to the world extent.
    camera.near = (wmax * 0.001).max(1e-6);
    camera.far = wmax * 100.0;
    camera.projection_mode = ProjectionMode::Perspective;
    let radius = wmax * radius_mult;
    camera.position = glam::Vec3::new(0.0, radius * 0.30, radius);
    // Orthographic height matches the world so a P-toggle is framed sensibly.
    camera.ortho_height = wmax.max(1e-6);
    camera
}
