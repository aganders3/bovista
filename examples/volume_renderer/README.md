# Volume Renderer

Direct volume rendering via GPU ray marching. Uses `VolumeVisual` with the same virtual texture streaming back-end as the slice viewer.

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

volume = bv.Volume(viewer, levels, max_tiles=2000, loader=request_tile)
volume.set_contrast(0.0, 0.3)
volume.set_density_scale(0.01)
volume.set_relative_step_size(1.0)
viewer.add(volume)
```

## API pattern (JavaScript)

```javascript
const viewer = await JsViewer.new('canvas');
const volume = new JsVolumeVisual(viewer, levels, 2000, requestTile);
volume.setContrast(0.0, 0.3);
volume.setDensityScale(0.01);
volume.setRelativeStepSize(1.0);
viewer.addVolume(volume);
```
