# Slice Viewer

Renders an arbitrary-orientation cross-section through a remote OME-Zarr volume using `ImageVisual` and virtual texture streaming.

## Implementations

| Platform | Entry point | How to run |
|----------|-------------|------------|
| Python | `python/remote_ome_zarr.py` | `uv run python examples/slice_viewer/python/remote_ome_zarr.py` |
| Web | `web/index.html` | `./build_wasm.sh && python -m http.server 8000 --directory examples` → `http://localhost:8000/slice_viewer/` |
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

image = bv.Image(viewer, levels, max_tiles=500, loader=request_tile)
image.set_slice_plane(cx, cy, cz, 0.0, 0.0, 1.0)  # XY slice
image.set_contrast(0.0, 0.5)
viewer.add(image)
```

## API pattern (JavaScript)

```javascript
const viewer = await JsViewer.new('canvas');
const image = new JsImageVisual(viewer, levels, 500, requestTile);
image.setSlicePlane(cx, cy, cz, 0, 0, 1);
image.setContrast(0.0, 0.5);
viewer.addImage(image);
```
