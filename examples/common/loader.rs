//! Pull-based OME-Zarr tile loader worker pool, shared by the native examples.
//!
//! Workers poll bovista's `wanted` map (the priority-ordered set of tiles it
//! wants right now), claim the highest-priority unclaimed key (coarser LOD
//! first so the volume blocks in low-res before refining), decode it via
//! `read_tile`, and insert the resulting `TileData` into `pending` — but only
//! if the key is STILL wanted at the end (otherwise the view moved on during
//! the fetch). There is no internal scheduler queue: bovista's `wanted` IS the
//! queue, and cancellation is implicit (keys that vanish are simply never
//! picked again).
//!
//! Tiles are pushed via the native `PendingChunks` insert path, NOT the
//! pyo3-only `set_chunk_data_u16`.
//!
//! `#[path]`-included into each example binary; self-contained (no `super::`).

#![allow(dead_code)]

use std::collections::HashSet;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use bovista::visuals::gpu_structs::TileKey;
use bovista::visuals::virtual_texture::{PendingChunks, Wanted};
use bovista::visuals::LodLevelConfig;

use crate::ome_zarr::{read_tile, Level, TileHook};

/// Everything the worker threads need. Built by `ome_zarr::open()`.
pub struct LoaderHandles {
    pub levels: Arc<Vec<Level>>,
    pub lods: Vec<LodLevelConfig>,
    /// Filled in by the example after its visual is constructed.
    pub pending_slot: Arc<Mutex<Option<PendingChunks>>>,
    /// Filled in by the example after its visual is constructed.
    pub wanted_slot: Arc<Mutex<Option<Wanted>>>,
    /// Current displayed timepoint. Kept for parity / future per-tile use;
    /// the read path uses `key.t` (each wanted key already carries its t).
    pub current_t: Arc<AtomicU32>,
    /// Per-tile diagnostics callback.
    pub tile_hook: TileHook,
}

/// Backstop on `pending` size. bovista's publish_wanted already subtracts
/// pending size from its budget, so wanted shrinks as pending fills; this only
/// catches the residual race where a worker claimed against a stale snapshot.
const PENDING_BACKSTOP: usize = 512;

/// Spawn `n_workers` pull-loader threads. Each loops forever.
pub fn spawn_workers(handles: LoaderHandles, n_workers: usize) {
    // Shared "claimed" set so two workers never decode the same key.
    let claimed: Arc<Mutex<HashSet<TileKey>>> = Arc::new(Mutex::new(HashSet::new()));
    // Origin for the per-tile diagnostics timestamp.
    let origin = Instant::now();
    println!("[loader] pool: {} worker(s) (pull-based)", n_workers);

    let LoaderHandles {
        levels,
        lods,
        pending_slot,
        wanted_slot,
        current_t,
        tile_hook,
    } = handles;

    for _ in 0..n_workers {
        let wanted_slot = wanted_slot.clone();
        let claimed = claimed.clone();
        let pending_slot = pending_slot.clone();
        let levels = levels.clone();
        let lods = lods.clone();
        let current_t = current_t.clone();
        let tile_hook = tile_hook.clone();

        thread::spawn(move || loop {
            // Wait for bovista to hand us its wanted handle. Once populated this
            // never goes back to None.
            let wanted = loop {
                if let Some(w) = wanted_slot.lock().unwrap().clone() {
                    break w;
                }
                thread::sleep(Duration::from_millis(20));
            };

            // Pick the highest-priority unclaimed key. Tiebreak: coarser LOD
            // first so the volume blocks in at low resolution before refining.
            let key = {
                let w = wanted.lock().unwrap();
                let mut c = claimed.lock().unwrap();
                let pick = w
                    .iter()
                    .filter(|(k, _)| !c.contains(k))
                    .min_by_key(|(k, p)| (**p, -(k.lod_level as i32)))
                    .map(|(k, _)| *k);
                if let Some(k) = pick {
                    c.insert(k);
                }
                pick
            };
            let Some(key) = key else {
                thread::sleep(Duration::from_millis(20));
                continue;
            };
            if key.lod_level >= levels.len() {
                claimed.lock().unwrap().remove(&key);
                continue;
            }

            // Each wanted key already carries the timepoint it wants; touch
            // `current_t` so the handle is observably used.
            let _ = current_t.load(Ordering::Relaxed);

            let lod_cfg = &lods[key.lod_level];
            let decode_start = Instant::now();
            let result = read_tile(&levels[key.lod_level], lod_cfg, key, key.t);
            let decode_ns = decode_start.elapsed().as_nanos() as u64;

            // Re-check membership: if bovista dropped this key since we claimed
            // it, don't waste a pending-slot.
            let still_wanted = wanted.lock().unwrap().contains_key(&key);
            match result {
                Ok(data) if still_wanted => {
                    if let Some(p) = pending_slot.lock().unwrap().clone() {
                        let mut guard = p.lock().unwrap();
                        if guard.len() < PENDING_BACKSTOP {
                            guard.insert(key, data);
                            drop(guard);
                            let now_ns = origin.elapsed().as_nanos() as u64;
                            (tile_hook)(now_ns, decode_ns);
                        }
                    }
                }
                Ok(_) => { /* dropped by bovista; throw bytes away */ }
                Err(e) => {
                    eprintln!(
                        "[loader] tile load failed (lod={} t={} z={} y={} x={}): {}",
                        key.lod_level, key.t, key.z, key.y, key.x, e
                    );
                }
            }
            claimed.lock().unwrap().remove(&key);
        });
    }
}

/// Cap the global rayon pool (zarrs dispatches chunk reads through it) to a
/// fixed thread count so spawned in-flight workers don't thrash a huge shared
/// pool on many-core machines. Idempotent-ish: only the first call wins.
pub fn cap_rayon_pool() {
    let rayon_threads: usize = std::env::var("RAYON_NUM_THREADS")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(8);
    let _ = rayon::ThreadPoolBuilder::new()
        .num_threads(rayon_threads)
        .build_global();
}
