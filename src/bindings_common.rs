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

// Chunked loading helpers
use crate::visuals::tile::{TileRequest, ChunkStatus};

/// Helper struct to extract coordinates from TileRequest
///
/// This reduces duplication in Python and WASM chunk loader callbacks.
pub struct TileRequestContext {
    pub lod_level: usize,
    pub z: u32,
    pub y: u32,
    pub x: u32,
}

impl TileRequestContext {
    /// Extract coordinates from a TileRequest
    pub fn from_request(request: &TileRequest) -> Self {
        Self {
            lod_level: request.lod_level.unwrap_or(0),
            z: request.z,
            y: request.y,
            x: request.x,
        }
    }
}

/// Convert integer status code to ChunkStatus enum
///
/// This provides a unified conversion that works for both Python and WASM.
/// Status codes: 0 = Accepted, 1 = AlreadyPending, 2+ = Rejected
pub fn status_from_int(code: i32) -> ChunkStatus {
    match code {
        0 => ChunkStatus::Accepted,
        1 => ChunkStatus::AlreadyPending,
        2 => ChunkStatus::Rejected,
        _ => ChunkStatus::Rejected,
    }
}
