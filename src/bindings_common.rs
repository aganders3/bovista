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

/// Pack a label tile: store the raw integer ID *by magnitude* into R16Float
/// (no normalization), so the shader's LabelHash mode can recover the exact
/// ID. f16 is integer-exact up to 2048 — dense IDs beyond that must be
/// relabelled loader-side (see the labels roadmap). Shared by both bindings
/// and by `pack_u16_label_tile`.
pub fn pack_u16_label_tile(
    data: impl IntoIterator<Item = u16>,
    z_shape: u32,
    y_shape: u32,
    x_shape: u32,
) -> TileData {
    let bytes = data
        .into_iter()
        .flat_map(|v| half::f16::from_f32(v as f32).to_le_bytes())
        .collect();
    TileData { data: bytes, z_shape, y_shape, x_shape, format: wgpu::TextureFormat::R16Float }
}

/// Pack a uint8 label tile by magnitude (see [`pack_u16_label_tile`]). u8 IDs
/// are always within the f16-exact range.
pub fn pack_u8_label_tile(
    data: impl IntoIterator<Item = u8>,
    z_shape: u32,
    y_shape: u32,
    x_shape: u32,
) -> TileData {
    let bytes = data
        .into_iter()
        .flat_map(|v| half::f16::from_f32(v as f32).to_le_bytes())
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

#[cfg(test)]
mod tests {
    use super::*;

    fn decode_f16(tile: &TileData) -> Vec<f32> {
        tile.data
            .chunks_exact(2)
            .map(|b| half::f16::from_le_bytes([b[0], b[1]]).to_f32())
            .collect()
    }

    #[test]
    fn label_packer_preserves_integer_ids() {
        // The whole Labels feature rests on integer identity surviving the
        // R16Float atlas. f16 is integer-exact up to 2048.
        let ids: Vec<u16> = vec![0, 1, 2, 5, 42, 255, 2048];
        let tile = pack_u16_label_tile(ids.iter().copied(), 1, 1, ids.len() as u32);
        let got = decode_f16(&tile);
        for (id, v) in ids.iter().zip(got) {
            assert_eq!(v, *id as f32, "label id {id} must round-trip exactly");
        }
    }

    #[test]
    fn normalized_packer_would_destroy_ids() {
        // Contrast for *why* labels need their own packer: the normalized path
        // maps id 5 to 5/65535, nothing like an integer id — so it must NOT be
        // used for labels.
        let tile = pack_u16_tile([5u16].into_iter(), 1, 1, 1);
        let v = decode_f16(&tile)[0];
        assert!(v < 1e-3, "normalized value {v} is nothing like the id 5");
        assert_ne!(v, 5.0);
    }

    #[test]
    fn u8_label_packer_preserves_ids() {
        let ids: Vec<u8> = vec![0, 1, 7, 200, 255];
        let tile = pack_u8_label_tile(ids.iter().copied(), 1, 1, ids.len() as u32);
        for (id, v) in ids.iter().zip(decode_f16(&tile)) {
            assert_eq!(v, *id as f32);
        }
    }
}

