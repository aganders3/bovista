//! Shared utilities for Python and WASM bindings
//!
//! This module provides common functionality to reduce duplication between
//! the Python (PyO3) and WASM (wasm-bindgen) bindings.

use std::any::Any;

#[cfg(not(target_arch = "wasm32"))]
use std::sync::{Arc, Mutex};
#[cfg(target_arch = "wasm32")]
use std::rc::Rc;
#[cfg(target_arch = "wasm32")]
use std::cell::RefCell;

use crate::visual::Visual;
use crate::visuals::gpu_structs::TileData;

/// Pack a uint16 tile into `TileData`, normalizing the full u16 range to
/// [0, 1] and storing as R16Float (the atlas format).
///
/// Shared by both bindings so the half-float packing — which is
/// correctness-sensitive — lives in exactly one place.
pub fn pack_u16_tile(
    data: impl IntoIterator<Item = u16>,
    z_shape: u32,
    y_shape: u32,
    x_shape: u32,
) -> TileData {
    let bytes = data
        .into_iter()
        .flat_map(|v| half::f16::from_f32(v as f32 / u16::MAX as f32).to_le_bytes())
        .collect();
    TileData { data: bytes, z_shape, y_shape, x_shape, format: wgpu::TextureFormat::R16Float }
}

/// Pack a uint8 tile into `TileData`, normalizing the full u8 range to
/// [0, 1] and storing as R16Float. Lets callers push native 8-bit data
/// without pre-scaling it to u16 themselves.
pub fn pack_u8_tile(
    data: impl IntoIterator<Item = u8>,
    z_shape: u32,
    y_shape: u32,
    x_shape: u32,
) -> TileData {
    let bytes = data
        .into_iter()
        .flat_map(|v| half::f16::from_f32(v as f32 / u8::MAX as f32).to_le_bytes())
        .collect();
    TileData { data: bytes, z_shape, y_shape, x_shape, format: wgpu::TextureFormat::R16Float }
}

// Type alias that works for both platforms
#[cfg(not(target_arch = "wasm32"))]
pub type VisualRef = Arc<Mutex<dyn Visual>>;
#[cfg(target_arch = "wasm32")]
pub type VisualRef = Rc<RefCell<dyn Visual>>;

/// Generic downcast helper for mutable access - Native version
///
/// Safely downcasts a VisualRef to a specific Visual type and applies a closure.
/// This handles the platform-specific synchronization primitives (Arc<Mutex<>> on native).
#[cfg(not(target_arch = "wasm32"))]
pub fn with_visual_mut<T, F, R>(visual_ref: &VisualRef, f: F) -> Result<R, String>
where
    T: Visual + 'static,
    F: FnOnce(&mut T) -> R,
{
    let mut visual = visual_ref.lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let visual_any: &mut dyn Any = &mut *visual;
    visual_any.downcast_mut::<T>()
        .map(f)
        .ok_or_else(|| format!("Failed to downcast to {}", std::any::type_name::<T>()))
}

/// Generic downcast helper for mutable access - WASM version
///
/// Safely downcasts a VisualRef to a specific Visual type and applies a closure.
/// This handles the platform-specific synchronization primitives (Rc<RefCell<>> on WASM).
#[cfg(target_arch = "wasm32")]
pub fn with_visual_mut<T, F, R>(visual_ref: &VisualRef, f: F) -> Result<R, String>
where
    T: Visual + 'static,
    F: FnOnce(&mut T) -> R,
{
    let mut visual = visual_ref.try_borrow_mut()
        .map_err(|e| format!("Borrow error: {}", e))?;
    let visual_any: &mut dyn Any = &mut *visual;
    visual_any.downcast_mut::<T>()
        .map(f)
        .ok_or_else(|| format!("Failed to downcast to {}", std::any::type_name::<T>()))
}

/// Generic downcast helper for immutable access - Native version
///
/// Safely downcasts a VisualRef to a specific Visual type and applies a closure.
/// Use this for read-only access to the visual.
#[cfg(not(target_arch = "wasm32"))]
pub fn with_visual_ref<T, F, R>(visual_ref: &VisualRef, f: F) -> Result<R, String>
where
    T: Visual + 'static,
    F: FnOnce(&T) -> R,
{
    let visual = visual_ref.lock()
        .map_err(|e| format!("Lock error: {}", e))?;
    let visual_any: &dyn Any = &*visual;
    visual_any.downcast_ref::<T>()
        .map(f)
        .ok_or_else(|| format!("Failed to downcast to {}", std::any::type_name::<T>()))
}

/// Generic downcast helper for immutable access - WASM version
///
/// Safely downcasts a VisualRef to a specific Visual type and applies a closure.
/// Use this for read-only access to the visual.
#[cfg(target_arch = "wasm32")]
pub fn with_visual_ref<T, F, R>(visual_ref: &VisualRef, f: F) -> Result<R, String>
where
    T: Visual + 'static,
    F: FnOnce(&T) -> R,
{
    let visual = visual_ref.try_borrow()
        .map_err(|e| format!("Borrow error: {}", e))?;
    let visual_any: &dyn Any = &*visual;
    visual_any.downcast_ref::<T>()
        .map(f)
        .ok_or_else(|| format!("Failed to downcast to {}", std::any::type_name::<T>()))
}

