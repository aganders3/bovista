//! Categorical color palette shared by the label visuals (`Labels`,
//! `LabelVolume`). The shader hashes each integer label ID into this LUT, so
//! the palette just needs many visually-distinct entries.

/// Build a 256-entry RGBA colormap of visually-distinct categorical colors by
/// stepping hue with the golden-ratio conjugate (low-discrepancy → adjacent
/// LUT indices are far apart in hue).
pub(crate) fn categorical_colormap() -> Vec<u8> {
    const GOLDEN: f32 = 0.618_034;
    let mut out = Vec::with_capacity(256 * 4);
    let mut h = 0.0f32;
    for _ in 0..256 {
        h = (h + GOLDEN).fract();
        // Fixed high saturation/value for vivid, well-separated colors.
        let (r, g, b) = hsv_to_rgb(h, 0.7, 0.95);
        out.extend_from_slice(&[r, g, b, 255]);
    }
    out
}

/// HSV → RGB (all inputs in [0, 1]); returns 8-bit channels.
pub(crate) fn hsv_to_rgb(h: f32, s: f32, v: f32) -> (u8, u8, u8) {
    let i = (h * 6.0).floor();
    let f = h * 6.0 - i;
    let p = v * (1.0 - s);
    let q = v * (1.0 - f * s);
    let t = v * (1.0 - (1.0 - f) * s);
    let (r, g, b) = match (i as i32).rem_euclid(6) {
        0 => (v, t, p),
        1 => (q, v, p),
        2 => (p, v, t),
        3 => (p, q, v),
        4 => (t, p, v),
        _ => (v, p, q),
    };
    (
        (r * 255.0).round() as u8,
        (g * 255.0).round() as u8,
        (b * 255.0).round() as u8,
    )
}
