//! Tile packing helpers shared by the frontend bindings.
//!
//! The half-float packing here is correctness-sensitive — the label packers'
//! whole point is exact integer round-trip (f16 is integer-exact to 2048) — so
//! it lives in the core crate as the single source of truth rather than being
//! reimplemented per binding. The Python and WASM bindings pull these into
//! their local `bindings_common` module.

use crate::visuals::gpu_structs::TileData;

/// Pack a uint16 tile into `TileData`, normalizing the full u16 range to
/// [0, 1] and storing as R16Float (the atlas format).
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
