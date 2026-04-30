# Example: Slice Viewer

The slice viewer renders an arbitrary-orientation cross-section through a remote OME-Zarr volume. It demonstrates the `ImageVisual` rendering path, the virtual texture streaming system, and the cross-platform loader pattern.

<!-- toc -->

## Live Demo

<div class="live-demo-container">
<iframe src="examples/live/slice_viewer/" title="Slice Viewer — live demo"></iframe>
<div class="live-demo-links">
Source:
<a href="https://github.com/aganders3/bovista/tree/main/examples/slice_viewer/web">Web (JS)</a> ·
<a href="https://github.com/aganders3/bovista/tree/main/examples/slice_viewer/python">Python</a> ·
<a href="https://github.com/aganders3/bovista/tree/main/examples/slice_viewer/rust">Rust (stub)</a>
</div>
</div>

## What It Demonstrates

- Loading multi-resolution OME-Zarr data from public S3 buckets
- `ImageVisual`: single-draw-call slice plane with virtual texture streaming
- Asynchronous tile loading (Python `ThreadPoolExecutor` / JS `fetch()`)
- LOD selection and debug visualization
- Camera controls: orbit, pan, zoom, orthographic/perspective toggle
- Window/level contrast adjustment

## Running

**Python:**
```bash
uv sync
uv run python examples/slice_viewer/python/remote_ome_zarr.py
```

**Web:**
```bash
./build_wasm.sh
python3 -m http.server 8000 --directory examples
# open http://localhost:8000/slice_viewer/
```

## Key Controls

| Input | Action |
|-------|--------|
| Left-drag | Orbit camera |
| Right-drag | Pan camera |
| Scroll / pinch | Zoom |
| Middle-drag | Adjust contrast |
| P | Toggle perspective / orthographic |
| D | Toggle LOD debug tint |
| ↑ / ↓ | Move slice plane along normal |

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
ImageVisual → virtual_tile.wgsl → screen
```

The slice plane is a flat polygon whose vertices are the intersection of the current plane with each tile's bounding box. The fragment shader resolves the atlas address for each fragment via the page table and samples the 3D atlas.

## Datasets

All datasets are publicly accessible — no credentials needed.

| Dataset | Format | Levels | Notes |
|---------|--------|--------|-------|
| Marmoset neurons | OME-Zarr 0.5 (v3) | 8 | 3D fluorescence microscopy |
| Pawpawsaurus fossil | OME-Zarr 0.5 (v3) | 4 | Micro-CT scan |
| Beechnut CT | OME-Zarr 0.5 (v3) | 6 | High-res CT |
| Platybrowser embryo | OME-Zarr 0.4 (v2) | 10 | EMBL, large pyramid |

## Code Walkthrough

### 1. Build level metadata

```python
multiscales = zarr_group.attrs['multiscales'][0]
datasets = multiscales['datasets']

levels = []
for i, ds in enumerate(datasets):
    arr = zarr_group[ds['path']]
    transform = ds.get('coordinateTransformations', [{}])
    # ... parse voxel size, translation from OME metadata
    levels.append(bv.LevelMetadata(
        volume_size=(arr.shape[0], arr.shape[1], arr.shape[2]),
        chunk_size=tuple(arr.chunks),
        voxel_size=voxel_size,
        scale_factor=1.0 / (2.0 ** i),
    ))
```

### 2. Create the visual

```python
image = bv.Image(
    viewer,
    levels,
    max_tiles=500,
    loader=request_tile,
)
viewer.add(image)

# Position slice at volume center
cx, cy, cz = compute_center(levels[0])
image.set_slice_plane(cx, cy, cz, 0.0, 0.0, 1.0)  # Z-normal (XY slice)
```

### 3. Loader callback

```python
executor = ThreadPoolExecutor(max_workers=8)

def request_tile(lod, z, y, x):
    key = (lod, z, y, x)
    if key in pending: return bv.ChunkStatus.AlreadyPending
    pending.add(key)
    executor.submit(load_tile, key)
    return bv.ChunkStatus.Accepted

def load_tile(key):
    lod, z, y, x = key
    arr = zarr_levels[lod]
    cd, ch, cw = arr.chunks
    data = arr[z*cd:(z+1)*cd, y*ch:(y+1)*ch, x*cw:(x+1)*cw]
    image.set_chunk_data_u16(lod, z, y, x, np.asarray(data, dtype=np.uint16))
    pending.discard(key)
```

### 4. Render loop (Qt timer)

```python
def on_timer():
    viewer.render_frame()

timer = QTimer()
timer.timeout.connect(on_timer)
timer.start(16)   # ~60 fps
```
