# Example: Volume Renderer

The volume renderer performs direct volume rendering (DVR) via GPU ray marching. It uses the same virtual texture back-end as the slice viewer but fires a ray per pixel through the volume rather than intersecting a plane.

<!-- toc -->

## Live Demo

<div class="live-demo-container">
<iframe src="examples/live/volume_renderer/" title="Volume Renderer — live demo"></iframe>
<div class="live-demo-links">
Source:
<a href="https://github.com/aganders3/bovista/tree/main/examples/volume_renderer/web">Web (JS)</a> ·
<a href="https://github.com/aganders3/bovista/tree/main/examples/volume_renderer/python">Python</a> ·
<a href="https://github.com/aganders3/bovista/tree/main/examples/volume_renderer/rust">Rust (stub)</a>
</div>
</div>

## What It Demonstrates

- Five volume classes: `DirectVolume` (DVR), `MipVolume`, `MinipVolume`, `AverageVolume`, `IsosurfaceVolume`
- Direct volume rendering with front-to-back alpha compositing (`DirectVolume`)
- Transfer functions via colormap LUT
- Ray marching step size and density controls
- Same streaming back-end as the slice viewer (atlas + page table)
- Three debug modes: LOD tint, atlas visualization, step-count heatmap

## Running

**Python:**
```bash
uv sync
uv run python examples/volume_renderer/python/volume_ome_zarr.py
```

**Web:**
```bash
./build_wasm.sh
python3 -m http.server 8000 --directory examples
# open http://localhost:8000/volume_renderer/
```

## Key Controls

| Input | Action |
|-------|--------|
| Left-drag | Orbit camera |
| Right-drag | Pan camera |
| Scroll / pinch | Zoom |
| Middle-drag | Adjust contrast |
| D | LOD tint debug mode |
| A | Atlas debug mode |
| S | Step count heatmap |

## Architecture

```
DirectVolume.wanted_keys()  ← published each frame, sorted by priority
     ↓ poll
ThreadPoolExecutor / JS fetch  → OME-Zarr (S3)
     ↓ set_chunk_data_u16()
PendingChunks queue
     ↓ VirtualTextureData::prepare()
Atlas 3D texture + Page table
     ↓ single draw call
DirectVolume → volume_raymarch.wgsl → screen
```

The volume classes render a unit-cube bounding box with back-face culling inverted (the box is inward-wound, so back faces are rendered — these are the ray exit points). The fragment shader:

1. Reconstructs the ray entry point from the clip position and camera
2. Marches from entry to exit in world space, one voxel per step at the current LOD
3. At each step, looks up the page table to find the atlas tile, samples the atlas, applies the contrast window and colormap LUT
4. Composites front-to-back with premultiplied alpha; terminates when opacity saturates

Because the page table records the actual LOD at which each tile was loaded, the shader falls back to coarser tiles mid-ray for regions that haven't streamed in yet.

## Parameters

| Parameter | Method | Description |
|-----------|--------|-------------|
| Contrast | `set_contrast(min, max)` | Normalized 0–1 window (all modes) |
| Colormap | `set_colormap(rgba)` | 256-entry RGBA LUT; `None` = grayscale (all modes) |
| Step size | `set_relative_step_size(step)` | 1.0 = Nyquist at finest LOD (all modes) |
| LOD bias | `set_lod_bias(bias)` | Positive = prefer finer; negative = coarser (all modes) |
| Max tiles | at construction | VRAM budget; ~2 bytes × tile_size³ each |
| Density scale | `set_density_scale(scale)` | Per-step opacity multiplier (`DirectVolume`) |
| Attenuation | `set_attenuation(strength)` | `0` = plain MIP; `> 0` = attenuated MIP (`MipVolume`) |
| Iso threshold | `set_iso_threshold(t)` | First-hit surface level, 0–1 (`IsosurfaceVolume`) |

The five mode classes share the constructor and the "all modes" controls above; each adds only the
parameters relevant to its ray-marching mode. See [Volume classes](12-python.md#volume-classes) for
the full per-class breakdown.

## Code Walkthrough

### 1. Build level metadata (same as slice viewer)

```python
levels = build_levels_from_ome_zarr(zarr_group)
```

### 2. Create the visual

```python
# Swap DirectVolume for MipVolume / MinipVolume / AverageVolume /
# IsosurfaceVolume to change the ray-marching mode.
volume = bv.DirectVolume(
    viewer,
    levels,
    max_tiles=2000,
)
viewer.add(volume)

# Initial parameters
volume.set_contrast(0.0, 0.3)
volume.set_density_scale(0.01)   # DirectVolume only
volume.set_relative_step_size(1.0)
```

### 3. Pull-based loader (identical pattern to slice viewer)

No loader callback — a background thread polls `volume.wanted_keys()` (the same API as `Image`) and
pushes tiles back with `set_chunk_data_u16`. Tile keys carry a timepoint `t`.

```python
def poll_tiles():
    for lod, t, z, y, x, _priority in volume.wanted_keys():
        key = (lod, t, z, y, x)
        if key in in_flight or len(in_flight) >= 8:
            continue
        in_flight.add(key)
        executor.submit(load_tile, key)   # load_tile pushes set_chunk_data_u16(lod, t, z, y, x, data)
```

### 4. Density scale calculation

The examples compute a reasonable initial density scale based on the dataset's physical extent:

```python
extent = max(
    vol_depth  * voxel_z,
    vol_height * voxel_y,
    vol_width  * voxel_x,
)
# Target: ~200 steps across the widest dimension at step_size=1.0
base_density = 1.0 / 200.0
volume.set_density_scale(base_density)
```

## Web-Specific: VRAM Budget

The web example includes a VRAM budget selector that converts a GB target into `max_tiles`:

```javascript
const tileVram = tileDepth * tileHeight * tileWidth * 2; // R16Float = 2 bytes/voxel
const maxTiles = Math.floor((budgetBytes * 0.8) / tileVram); // 80% headroom
// No loader argument — poll volume.wantedKeys() and push setChunkDataU16(...).
const volume = new wasmModule.DirectVolume(viewer, levels, maxTiles);
viewer.addDirectVolume(volume);
```

## Datasets

| Dataset | Format | Notes |
|---------|--------|-------|
| Marmoset neurons | OME-Zarr 0.5 | Dense 3D fluorescence |
| Pawpawsaurus fossil | OME-Zarr 0.5 | Micro-CT, good for DVR |
| Beechnut CT | OME-Zarr 0.5 | High-resolution CT scan |
