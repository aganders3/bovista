# Slice Renderer

Renders an arbitrary-orientation cross-section through a remote OME-Zarr volume using `Image` and virtual texture streaming.

## Implementations

| Platform | Entry point | How to run |
|----------|-------------|------------|
| Python | `python/remote_ome_zarr.py` | `uv run python examples/slice_renderer/python/remote_ome_zarr.py` |
| Web | `web/index.html` | `./build_wasm.sh && python -m http.server 8000 --directory examples` → `http://localhost:8000/slice_renderer/` |
| Rust | `rust/main.rs` | stub — see comments |

## Controls

| Input | Action |
|-------|--------|
| Left-drag | Orbit camera |
| Right-drag | Pan camera |
| Scroll / pinch | Zoom |
| Middle-drag | Adjust contrast (window/level) |
| P | Toggle perspective / orthographic |
| D | Toggle LOD debug tint |
| ↑ / ↓ | Move slice plane |

## API pattern (Python)

```python
import bovista as bv

viewer = bv.Viewer(800, 600)
viewer.initialize_with_window(handle, width, height)

image = bv.Image(viewer, levels, max_tiles=500)
image.set_slice_plane(cx, cy, cz, 0.0, 0.0, 1.0)  # XY slice
image.set_contrast(0.0, 0.5)
viewer.add(image)

# Pull loop: poll the wanted set, fetch tiles, push them back.
# Run this on a background thread; keys are sorted by priority.
for lod, t, z, y, x, priority in image.wanted_keys():
    data = fetch_tile(lod, t, z, y, x)        # uint16 numpy array
    image.set_chunk_data_u16(lod, t, z, y, x, data)
```

## API pattern (JavaScript)

```javascript
const viewer = await Viewer.new('canvas');
const image = new Image(viewer, levels, 500);
image.setSlicePlane(cx, cy, cz, 0, 0, 1);
image.setContrast(0.0, 0.5);
viewer.addImage(image);

// Pull loop: wantedKeys() is a flat Uint32Array of
// [lod, t, z, y, x, priority, ...] (6 ints per key), sorted by priority.
const w = image.wantedKeys();
for (let i = 0; i < w.length; i += 6) {
    const [lod, t, z, y, x, prio] = w.slice(i, i + 6);
    fetchTile(lod, t, z, y, x).then(arr =>
        image.setChunkDataU16(lod, t, z, y, x, arr, zShape, yShape, xShape));
}
```
