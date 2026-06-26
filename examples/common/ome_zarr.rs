//! OME-Zarr loader shared by the native examples. Opens a multiscale group on
//! a filesystem path or http(s) URL, publishes `LodLevelConfig`s derived from
//! the multiscales metadata, and serves tile reads on a small worker pool.
//! Tiles are normalized to R16Float [0, 1].
//!
//! Decoupled from any example-specific state: the current timepoint is read
//! through a passed-in `Arc<AtomicU32>`, and per-tile diagnostics fire through
//! an optional `TileHook` callback (default: no-op).
//!
//! This module is `#[path]`-included into each example binary and is fully
//! self-contained (no `super::`).

#![allow(dead_code)]

use std::error::Error;
use std::sync::atomic::AtomicU32;
use std::sync::{Arc, Mutex};

use bovista::visuals::gpu_structs::{TileData, TileKey};
use bovista::visuals::virtual_texture::{PendingChunks, Wanted};
use bovista::visuals::LodLevelConfig;

use zarrs::array::{Array, ArraySubset};
use zarrs::group::Group;
use zarrs::storage::ReadableStorageTraits;

// The loader worker pool lives in a sibling module. Each example `#[path]`-
// includes both `ome_zarr.rs` and `loader.rs` as top-level modules, so we
// reach it via `crate::loader`.
use crate::loader;

/// Per-tile diagnostics hook. Called once for every tile that successfully
/// installs into `pending`, with the wall-clock-since-some-origin in ns and
/// the decode duration in ns. Examples that don't care pass `noop_hook()`.
pub type TileHook = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// A diagnostics hook that does nothing — the default.
pub fn noop_hook() -> TileHook {
    Arc::new(|_now_ns, _decode_ns| {})
}

/// Each LOD level we serve. The storage is type-erased so the same struct
/// holds either a filesystem or http store.
pub(crate) struct Level {
    pub(crate) array: Array<dyn ReadableStorageTraits>,
    /// Index of (z, y, x) within the array's possibly-higher-dim shape.
    pub(crate) z_idx: usize,
    pub(crate) y_idx: usize,
    pub(crate) x_idx: usize,
    /// Index of the "t" axis, if present.
    pub(crate) t_idx: Option<usize>,
    /// Volume dims in (z, y, x) voxel order at this LOD.
    pub(crate) volume_zyx: (u64, u64, u64),
}

/// Output of `open()` — everything the harness needs to build a visual, wire
/// the loader, and size the camera.
pub struct SceneSetup {
    pub lods: Vec<LodLevelConfig>,
    /// Slot for the visual's `pending_chunks()`; filled in after the visual is
    /// constructed (the loader threads read it).
    pub pending_slot: Arc<Mutex<Option<PendingChunks>>>,
    /// World-space extents of LOD 0 along (z, y, x). Sizes the camera orbit.
    pub world_extents: (f32, f32, f32),
    /// Default atlas slot count for the visual; CLI `--cache-tiles` overrides.
    pub cache_capacity: u32,
    /// Number of timepoints if axes include a "t" axis, else 1.
    pub n_timepoints: u32,
    /// Count of tiles in bovista's `wanted` set at a given timepoint. Used by
    /// the progress log.
    pub queue_count_at: Arc<dyn Fn(u32) -> usize + Send + Sync>,
    /// Slot for the visual's `wanted_handle()` so the loader can read what
    /// bovista wants. Filled in after the visual is constructed.
    pub wanted_slot: Arc<Mutex<Option<Wanted>>>,
}

pub fn open(
    path_or_url: &str,
    current_t: Arc<AtomicU32>,
    tile_hook: TileHook,
    max_inflight: usize,
) -> Result<SceneSetup, Box<dyn Error>> {
    let store: Arc<dyn ReadableStorageTraits> =
        if path_or_url.starts_with("http://") || path_or_url.starts_with("https://") {
            Arc::new(zarrs_http::HTTPStore::new(path_or_url)?)
        } else {
            Arc::new(zarrs::filesystem::FilesystemStore::new(path_or_url)?)
        };

    let group = Group::open(store.clone(), "/")?;
    let attrs = group.attributes();

    // OME-Zarr v0.5 (zarr v3): attrs["ome"]["multiscales"][0]
    // OME-Zarr v0.4 (zarr v2): attrs["multiscales"][0]
    let multiscales = attrs
        .get("ome")
        .and_then(|o| o.get("multiscales"))
        .or_else(|| attrs.get("multiscales"))
        .and_then(|m| m.as_array())
        .and_then(|arr| arr.first())
        .ok_or("no multiscales metadata in zarr group")?;

    // Axis order, e.g. ["t", "c", "z", "y", "x"] → we want to pluck z/y/x.
    let axes: Vec<String> = multiscales
        .get("axes")
        .and_then(|a| a.as_array())
        .map(|arr| {
            arr.iter()
                .map(|ax| {
                    ax.get("name")
                        .and_then(|n| n.as_str())
                        .unwrap_or("")
                        .to_string()
                })
                .collect()
        })
        .unwrap_or_default();

    let datasets_meta = multiscales
        .get("datasets")
        .and_then(|d| d.as_array())
        .ok_or("multiscales has no datasets[]")?;
    if datasets_meta.is_empty() {
        return Err("multiscales.datasets is empty".into());
    }

    let mut levels: Vec<Level> = Vec::with_capacity(datasets_meta.len());
    let mut lod0_voxel_zyx: (f64, f64, f64) = (1.0, 1.0, 1.0);
    let mut lods: Vec<LodLevelConfig> = Vec::with_capacity(datasets_meta.len());

    // Determine z/y/x indices using the LOD 0 array.
    let lod0_path = dataset_path(&datasets_meta[0], 0)?;
    let lod0_arr = Array::open(store.clone(), &absolute_path(&lod0_path))?;
    let ndim = lod0_arr.shape().len();

    let t_idx: Option<usize> = axes.iter().position(|a| a == "t");
    let n_timepoints: u32 = t_idx
        .map(|i| u32::try_from(lod0_arr.shape()[i]).unwrap_or(1).max(1))
        .unwrap_or(1);

    let (z_idx, y_idx, x_idx) = match (
        axes.iter().position(|a| a == "z"),
        axes.iter().position(|a| a == "y"),
        axes.iter().position(|a| a == "x"),
    ) {
        (Some(z), Some(y), Some(x)) => (z, y, x),
        _ => {
            if ndim < 3 {
                return Err(format!("array has {} dimensions, need ≥ 3", ndim).into());
            }
            (ndim - 3, ndim - 2, ndim - 1)
        }
    };

    // Uniform tile size across LODs. Use LOD 0's regular chunk size, clamped to
    // MAX_TILE_AXIS so the GPU atlas stays bounded (tz*ty*tx*2 bytes/tile).
    const MAX_TILE_AXIS: u64 = 128;
    let lod0_chunk = lod0_arr.chunk_shape(&vec![0; ndim])?;
    let clamp = |chunk_dim: u64| -> u32 { u32::try_from(chunk_dim.min(MAX_TILE_AXIS)).unwrap_or(64) };
    let tile_z = clamp(lod0_chunk[z_idx].get());
    let tile_y = clamp(lod0_chunk[y_idx].get());
    let tile_x = clamp(lod0_chunk[x_idx].get());

    for (i, ds) in datasets_meta.iter().enumerate() {
        let path = dataset_path(ds, i)?;
        let arr = Array::open(store.clone(), &absolute_path(&path))?;

        let shape = arr.shape().to_vec();
        if shape.len() != ndim {
            return Err(format!(
                "LOD {} has different dimensionality ({} vs {})",
                i,
                shape.len(),
                ndim
            )
            .into());
        }
        let vz = shape[z_idx];
        let vy = shape[y_idx];
        let vx = shape[x_idx];

        let scale = ds
            .get("coordinateTransformations")
            .and_then(|t| t.as_array())
            .and_then(|arr| {
                arr.iter()
                    .find(|t| t.get("type").and_then(|v| v.as_str()) == Some("scale"))
            })
            .and_then(|t| t.get("scale"))
            .and_then(|s| s.as_array())
            .map(|arr| arr.iter().map(|v| v.as_f64().unwrap_or(1.0)).collect::<Vec<_>>())
            .unwrap_or_else(|| vec![1.0; ndim]);

        let vs_z = *scale.get(z_idx).unwrap_or(&1.0);
        let vs_y = *scale.get(y_idx).unwrap_or(&1.0);
        let vs_x = *scale.get(x_idx).unwrap_or(&1.0);

        let scale_factor = if i == 0 {
            lod0_voxel_zyx = (vs_z, vs_y, vs_x);
            1.0
        } else {
            let rz = vs_z / lod0_voxel_zyx.0;
            let ry = vs_y / lod0_voxel_zyx.1;
            let rx = vs_x / lod0_voxel_zyx.2;
            rz.max(ry).max(rx) as f32
        };

        // Center the world frame on the LOD-0 volume center.
        let (translation, voxel_size) = if i == 0 {
            let tz = -(vz as f64 * vs_z) / 2.0;
            let ty = -(vy as f64 * vs_y) / 2.0;
            let tx = -(vx as f64 * vs_x) / 2.0;
            (
                (tz as f32, ty as f32, tx as f32),
                (vs_z as f32, vs_y as f32, vs_x as f32),
            )
        } else {
            let lod0_translation = lods[0].translation;
            (lod0_translation, (vs_z as f32, vs_y as f32, vs_x as f32))
        };

        lods.push(LodLevelConfig {
            volume_size: (vz as u32, vy as u32, vx as u32),
            tile_size: (tile_z, tile_y, tile_x),
            voxel_size,
            scale_factor,
            translation,
        });

        levels.push(Level {
            array: arr,
            z_idx,
            y_idx,
            x_idx,
            t_idx,
            volume_zyx: (vz, vy, vx),
        });
    }

    println!(
        "[ome-zarr] {} LODs, axes={:?}, tile={}×{}×{} (zyx)",
        lods.len(),
        axes,
        tile_z,
        tile_y,
        tile_x
    );
    for (i, l) in lods.iter().enumerate() {
        println!(
            "  LOD {}: vol {}×{}×{} voxels, voxel size {:?}, scale {:.2}×",
            i, l.volume_size.0, l.volume_size.1, l.volume_size.2, l.voxel_size, l.scale_factor
        );
    }

    // World extents from LOD 0.
    let world_extents = {
        let l = &lods[0];
        (
            l.volume_size.0 as f32 * l.voxel_size.0,
            l.volume_size.1 as f32 * l.voxel_size.1,
            l.volume_size.2 as f32 * l.voxel_size.2,
        )
    };

    // Spawn the pull-based loader pool. Workers read bovista's `wanted` map and
    // fill `pending_slot`.
    let pending_slot: Arc<Mutex<Option<PendingChunks>>> = Arc::new(Mutex::new(None));
    let wanted_slot: Arc<Mutex<Option<Wanted>>> = Arc::new(Mutex::new(None));

    let handles = loader::LoaderHandles {
        levels: Arc::new(levels),
        lods: lods.clone(),
        pending_slot: pending_slot.clone(),
        wanted_slot: wanted_slot.clone(),
        current_t,
        tile_hook,
    };
    loader::spawn_workers(handles, max_inflight);

    println!("[ome-zarr] t axis has {} timepoint(s)", n_timepoints);

    Ok(SceneSetup {
        lods,
        pending_slot,
        world_extents,
        cache_capacity: 4096,
        n_timepoints,
        queue_count_at: {
            let ws = wanted_slot.clone();
            Arc::new(move |t| {
                let guard = ws.lock().unwrap();
                match guard.as_ref() {
                    Some(w) => w.lock().unwrap().iter().filter(|(k, _)| k.t == t).count(),
                    None => 0,
                }
            })
        },
        wanted_slot,
    })
}

fn dataset_path(ds: &serde_json::Value, idx: usize) -> Result<String, Box<dyn Error>> {
    ds.get("path")
        .and_then(|p| p.as_str())
        .map(String::from)
        .ok_or_else(|| format!("datasets[{}] missing \"path\"", idx).into())
}

fn absolute_path(rel: &str) -> String {
    if rel.starts_with('/') {
        rel.to_string()
    } else {
        format!("/{}", rel)
    }
}

/// Read the voxel sub-region for this tile and convert to R16Float bytes.
///
/// Always goes through `retrieve_array_subset`, which respects the codec
/// pipeline including `sharding_indexed`.
pub(crate) fn read_tile(
    level: &Level,
    lod_cfg: &LodLevelConfig,
    key: TileKey,
    timepoint: u32,
) -> Result<TileData, Box<dyn Error + Send + Sync>> {
    let (tz, ty, tx) = lod_cfg.tile_size;
    let (vz, vy, vx) = level.volume_zyx;

    let z0 = (key.z as u64) * tz as u64;
    let y0 = (key.y as u64) * ty as u64;
    let x0 = (key.x as u64) * tx as u64;
    let z1 = (z0 + tz as u64).min(vz);
    let y1 = (y0 + ty as u64).min(vy);
    let x1 = (x0 + tx as u64).min(vx);
    if z0 >= vz || y0 >= vy || x0 >= vx {
        return Err(format!("tile out of bounds: z={} y={} x={}", key.z, key.y, key.x).into());
    }
    let extent_z = (z1 - z0) as u32;
    let extent_y = (y1 - y0) as u32;
    let extent_x = (x1 - x0) as u32;

    let ndim = level.array.shape().len();
    let mut ranges: Vec<std::ops::Range<u64>> = (0..ndim).map(|_| 0..1).collect();
    ranges[level.z_idx] = z0..z1;
    ranges[level.y_idx] = y0..y1;
    ranges[level.x_idx] = x0..x1;
    if let Some(ti) = level.t_idx {
        let tmax = level.array.shape()[ti].saturating_sub(1) as u32;
        let t = timepoint.min(tmax) as u64;
        ranges[ti] = t..(t + 1);
    }
    let subset = ArraySubset::new_with_ranges(&ranges);

    let dtype_name = level
        .array
        .data_type()
        .name(zarrs::plugin::ZarrVersion::V3)
        .map(|n| n.to_string())
        .unwrap_or_default();

    static DBG_ONCE: std::sync::Once = std::sync::Once::new();
    DBG_ONCE.call_once(|| {
        println!("[ome-zarr] zarr dtype detected as: \"{}\"", dtype_name);
    });

    let f16_bytes = match dtype_name.as_str() {
        "uint8" => {
            let v: Vec<u8> = level.array.retrieve_array_subset(&subset)?;
            normalize_to_f16(&v, |x| x as f32 / 255.0)
        }
        "uint16" => {
            let v: Vec<u16> = level.array.retrieve_array_subset(&subset)?;
            normalize_to_f16(&v, |x| x as f32 / 65535.0)
        }
        "int8" => {
            let v: Vec<i8> = level.array.retrieve_array_subset(&subset)?;
            normalize_to_f16(&v, |x| (x as f32 + 128.0) / 255.0)
        }
        "int16" => {
            let v: Vec<i16> = level.array.retrieve_array_subset(&subset)?;
            normalize_to_f16(&v, |x| (x as f32 + 32768.0) / 65535.0)
        }
        "float32" => {
            let v: Vec<f32> = level.array.retrieve_array_subset(&subset)?;
            normalize_to_f16(&v, |x| x.clamp(0.0, 1.0))
        }
        "float16" => {
            let v: Vec<half::f16> = level.array.retrieve_array_subset(&subset)?;
            let mut out = Vec::with_capacity(v.len() * 2);
            for x in v {
                let bits = x.to_bits();
                out.push((bits & 0xff) as u8);
                out.push((bits >> 8) as u8);
            }
            out
        }
        other => return Err(format!("unsupported zarr dtype: {}", other).into()),
    };

    Ok(TileData {
        data: f16_bytes,
        z_shape: extent_z,
        y_shape: extent_y,
        x_shape: extent_x,
        format: wgpu::TextureFormat::R16Float,
    })
}

fn normalize_to_f16<T: Copy>(src: &[T], to_unit: impl Fn(T) -> f32) -> Vec<u8> {
    let mut out = Vec::with_capacity(src.len() * 2);
    let mut min_v: f32 = f32::INFINITY;
    let mut max_v: f32 = f32::NEG_INFINITY;
    for &x in src {
        let v = to_unit(x);
        if v < min_v {
            min_v = v;
        }
        if v > max_v {
            max_v = v;
        }
        let bits = half::f16::from_f32(v).to_bits();
        out.push((bits & 0xff) as u8);
        out.push((bits >> 8) as u8);
    }
    static DBG_RANGE: std::sync::Once = std::sync::Once::new();
    DBG_RANGE.call_once(|| {
        println!("[ome-zarr] first tile normalized range: [{:.3}, {:.3}]", min_v, max_v);
    });
    out
}
