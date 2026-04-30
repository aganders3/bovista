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

- `VolumeVisual`: direct volume rendering with front-to-back alpha compositing
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
OME-Zarr (S3)
     ↓ fetch/read
ThreadPoolExecutor / JS fetch
     ↓ set_chunk_data_u16()
PendingChunks queue
     ↓ VirtualTextureData::prepare()
Atlas 3D texture + Page table
     ↓ single draw call
VolumeVisual → volume_raymarch.wgsl → screen
```

`VolumeVisual` renders a unit-cube bounding box with back-face culling inverted (the box is inward-wound, so back faces are rendered — these are the ray exit points). The fragment shader:

1. Reconstructs the ray entry point from the clip position and camera
2. Marches from entry to exit in world space, one voxel per step at the current LOD
3. At each step, looks up the page table to find the atlas tile, samples the atlas, applies the contrast window and colormap LUT
4. Composites front-to-back with premultiplied alpha; terminates when opacity saturates

Because the page table records the actual LOD at which each tile was loaded, the shader falls back to coarser tiles mid-ray for regions that haven't streamed in yet.

## Parameters

| Parameter | Method | Description |
|-----------|--------|-------------|
| Contrast | `set_contrast(min, max)` | Normalized 0–1 window |
| Colormap | `set_colormap(rgba)` | 256-entry RGBA LUT; `None` = grayscale |
| Density scale | `set_density_scale(scale)` | Per-step opacity multiplier |
| Step size | `set_relative_step_size(step)` | 1.0 = Nyquist at finest LOD |
| LOD bias | `set_lod_bias(bias)` | Positive = prefer finer; negative = coarser |
| Max tiles | at construction | VRAM budget; ~2 bytes × tile_size³ each |

## Code Walkthrough

### 1. Build level metadata (same as slice viewer)

```python
levels = build_levels_from_ome_zarr(zarr_group)
```

### 2. Create the visual

```python
volume = bv.Volume(
    viewer,
    levels,
    max_tiles=2000,
    loader=request_tile,
)
viewer.add(volume)

# Initial parameters
volume.set_contrast(0.0, 0.3)
volume.set_density_scale(0.01)
volume.set_relative_step_size(1.0)
```

### 3. Loader callback (identical pattern to slice viewer)

```python
def request_tile(lod, z, y, x):
    key = (lod, z, y, x)
    if key in pending: return bv.ChunkStatus.AlreadyPending
    pending.add(key)
    executor.submit(load_tile, key)
    return bv.ChunkStatus.Accepted
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
const volume = new JsVolumeVisual(viewer, levels, maxTiles, requestTile);
```

## Datasets

| Dataset | Format | Notes |
|---------|--------|-------|
| Marmoset neurons | OME-Zarr 0.5 | Dense 3D fluorescence |
| Pawpawsaurus fossil | OME-Zarr 0.5 | Micro-CT, good for DVR |
| Beechnut CT | OME-Zarr 0.5 | High-resolution CT scan |
