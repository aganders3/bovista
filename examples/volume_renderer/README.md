# Volume Renderer

Direct volume rendering via GPU ray marching. Uses `DirectVolume` with the same virtual texture streaming back-end as the slice viewer.

bovista exposes the other render modes as separate classes — swap `bv.DirectVolume` / `DirectVolume` for any of `MipVolume` (set attenuation > 0 for napari-style attenuated MIP), `MinipVolume`, `AverageVolume`, or `IsosurfaceVolume` (set iso threshold). On the web, each has a matching `viewer.addDirectVolume`/`addMipVolume`/`addMinipVolume`/`addAverageVolume`/`addIsosurfaceVolume` method.

## Implementations

| Platform | Entry point | How to run |
|----------|-------------|------------|
| Python | `python/volume_ome_zarr.py` | `uv run python examples/volume_renderer/python/volume_ome_zarr.py` |
| Web | `web/index.html` | `./build_wasm.sh && python -m http.server 8000 --directory examples` → `http://localhost:8000/volume_renderer/` |
| Rust | `rust/main.rs` | stub — see comments |

## Controls

| Input | Action |
|-------|--------|
| Left-drag | Orbit camera |
| Right-drag | Pan camera |
| Scroll / pinch | Zoom |
| Middle-drag | Adjust contrast |
| D | LOD tint debug |
| A | Atlas debug |
| S | Step count heatmap |

## API pattern (Python)

```python
import bovista as bv

viewer = bv.Viewer(800, 600)
viewer.initialize_with_window(handle, width, height)

volume = bv.DirectVolume(viewer, levels, max_tiles=2000)
volume.set_contrast(0.0, 0.3)
volume.set_density_scale(0.01)
volume.set_relative_step_size(1.0)
viewer.add(volume)

# Pull loop: poll the wanted set, fetch tiles, push them back.
# Run this on a background thread; keys are sorted by priority.
for lod, t, z, y, x, priority in volume.wanted_keys():
    data = fetch_tile(lod, t, z, y, x)        # uint16 numpy array
    volume.set_chunk_data_u16(lod, t, z, y, x, data)
```

## API pattern (JavaScript)

```javascript
const viewer = await Viewer.new('canvas');
const volume = new DirectVolume(viewer, levels, 2000);
volume.setContrast(0.0, 0.3);
volume.setDensityScale(0.01);
volume.setRelativeStepSize(1.0);
viewer.addDirectVolume(volume);

// Pull loop: wantedKeys() is a flat Uint32Array of
// [lod, t, z, y, x, priority, ...] (6 ints per key), sorted by priority.
const w = volume.wantedKeys();
for (let i = 0; i < w.length; i += 6) {
    const [lod, t, z, y, x, prio] = w.slice(i, i + 6);
    fetchTile(lod, t, z, y, x).then(arr =>
        volume.setChunkDataU16(lod, t, z, y, x, arr, zShape, yShape, xShape));
}
```
