# Quick Reference

## Source Map

| Component | File |
|-----------|------|
| Renderer (device, queue, render pass) | `src/renderer.rs` |
| Camera (orbit, pan, zoom, frustum) | `src/camera.rs` |
| Scene (visual list, frame loop) | `src/scene.rs` |
| `Visual` trait | `src/visual.rs` |
| Shared Python/WASM logic | `src/bindings_common.rs` |
| `PointsVisual` | `src/visuals/points.rs` |
| `LinesVisual` | `src/visuals/lines.rs` |
| `ImageVisual` (slice rendering) | `src/visuals/image.rs` |
| `VolumeVisual` (ray marching DVR) | `src/visuals/volume.rs` |
| `VirtualTextureData` (atlas + page table + LOD) | `src/visuals/virtual_texture.rs` |
| `AtlasAllocator` | `src/visuals/atlas.rs` |
| `PageTable` | `src/visuals/page_table.rs` |
| Tile types, `TileKey`, `TileData`, uniforms | `src/visuals/tile.rs` |
| `CustomVisual` | `src/visuals/custom.rs` |
| Python bindings (PyO3) | `src/python.rs` |
| WASM bindings (wasm-bindgen) | `src/wasm.rs` |
| Proc-macro (`#[visual_methods]`, `#[camera_methods]`) | `bovista-codegen/src/lib.rs` |

## Shader Map

| Shader | File | Purpose |
|--------|------|---------|
| Virtual tile | `src/shaders/virtual_tile.wgsl` | Slice rendering via atlas + page table |
| Volume ray march | `src/shaders/volume_raymarch.wgsl` | DVR ray marching via atlas + page table |
| Point cloud | `src/shaders/point_cloud.wgsl` | Point rendering |
| Lines | `src/shaders/lines.wgsl` | Line rendering |

## Python API Summary

```python
import bovista as bv

# Viewer
viewer = bv.Viewer(width=800, height=600)
viewer.initialize_with_window(handle, width, height)  # embed in Qt/Tk
viewer.run()                                           # standalone winit window
viewer.render_frame()
viewer.resize(w, h)

# Camera
viewer.set_camera_position(x, y, z)
viewer.set_camera_target(x, y, z)
viewer.set_camera_up(x, y, z)
viewer.orbit_camera(dx, dy)
viewer.pan_camera(dx, dy)
viewer.zoom_camera(delta)
viewer.set_camera_clip_planes(near, far)
viewer.set_camera_projection_mode(bv.ProjectionMode.Perspective)
viewer.set_camera_projection_mode(bv.ProjectionMode.Orthographic)
viewer.set_camera_ortho_height(h)
viewer.get_camera_ortho_height() -> float
viewer.get_camera_distance() -> float

# Scene
idx = viewer.add(visual)
viewer.clear_visuals()
viewer.visual_count() -> int

# LevelMetadata  (all sizes are (depth/z, height/y, width/x))
level = bv.LevelMetadata(
    volume_size=(d, h, w),
    chunk_size=(cd, ch, cw),
    voxel_size=(vz, vy, vx),
    scale_factor=1.0,          # 1.0 for LOD 0, 2.0 for LOD 1, 4.0 for LOD 2, ...
    translation=(tz, ty, tx),  # optional
)

# Image (slice rendering)
image = bv.Image(viewer, levels, max_tiles=500, loader=fn)
image.set_slice_z(z)
image.set_slice_y(y)
image.set_slice_x(x)
image.set_slice_plane(px, py, pz, nx, ny, nz)
image.set_contrast(min, max)
image.set_colormap(rgba_array)    # (256,4) uint8; None = grayscale
image.set_lod_bias(bias)
image.set_debug_mode(True)        # LOD tint
image.get_stats() -> (loaded, visible)
image.set_chunk_data_u16(lod, z, y, x, array)  # numpy uint16

# Volume (ray marching DVR)
volume = bv.Volume(viewer, levels, max_tiles=2000, loader=fn)
volume.set_contrast(min, max)
volume.set_colormap(rgba_array)
volume.set_density_scale(scale)
volume.set_relative_step_size(step)  # 1.0 = Nyquist
volume.set_lod_bias(bias)
volume.set_debug_mode(True)          # LOD tint
volume.set_atlas_debug_mode(True)    # raw atlas
volume.set_step_debug_mode(True)     # step count heatmap
volume.get_stats() -> (loaded, visible)
volume.set_chunk_data_u16(lod, z, y, x, array)

# Points / Lines (static geometry)
points = bv.Points.from_numpy(viewer, positions, colors)  # (N,1,3) float32
lines  = bv.Lines.axis_helper(viewer, length=1.0)

# ChunkStatus
bv.ChunkStatus.Accepted
bv.ChunkStatus.AlreadyPending
bv.ChunkStatus.Rejected
```

## JavaScript API Summary

```javascript
import init, { JsViewer, JsImageVisual, JsVolumeVisual,
               JsLevelMetadata, JsChunkStatus } from './pkg/bovista.js';
await init();

// Viewer
const viewer = await JsViewer.new('canvas-id');
viewer.renderFrame();
viewer.resize(w, h);

// Camera
viewer.setCameraPosition(x, y, z);   viewer.setCameraTarget(x, y, z);
viewer.setCameraUp(x, y, z);         viewer.orbitCamera(dx, dy);
viewer.panCamera(dx, dy);            viewer.zoomCamera(delta);
viewer.setCameraClipPlanes(near, far);
viewer.setCameraProjectionMode(JsProjectionMode.Perspective);
viewer.setCameraOrthoHeight(h);      viewer.getCameraOrthoHeight();
viewer.getCameraDistance();

// Scene
viewer.addImage(image);     viewer.addVolume(volume);
viewer.addPoints(points);   viewer.addLines(lines);
viewer.clearScene();        viewer.visualCount();

// LevelMetadata  (constructor takes individual numbers)
const level = new JsLevelMetadata(
    volumeW, volumeH, volumeD,
    chunkW,  chunkH,  chunkD,
    voxelW,  voxelH,  voxelD,
    scaleFactor,
    translateX, translateY, translateZ
);
// Getters return arrays: level.volume_size, level.chunk_size, level.voxel_size

// JsImageVisual
const image = new JsImageVisual(viewer, levels, maxTiles, chunkLoader);
image.setSliceZ(z);  image.setSliceY(y);  image.setSliceX(x);
image.setSlicePlane(px, py, pz, nx, ny, nz);
image.setContrast(min, max);
image.setColormap(uint8Array);       // 1024 bytes (256 RGBA); empty = grayscale
image.setLodBias(bias);
image.setDebugMode(true);
image.getStats();                    // [loaded, visible]
image.setChunkDataU16(lod, z, y, x, uint16Array, w, h, d);

// JsVolumeVisual
const volume = new JsVolumeVisual(viewer, levels, maxTiles, chunkLoader);
volume.setContrast(min, max);        volume.setColormap(uint8Array);
volume.setDensityScale(scale);       volume.setRelativeStepSize(step);
volume.setLodBias(bias);
volume.setDebugMode(true);           // LOD tint
volume.setAtlasDebugMode(true);      // raw atlas
volume.setStepDebugMode(true);       // step count heatmap
volume.getStats();                   // [loaded, visible]
volume.setChunkDataU16(lod, z, y, x, uint16Array, w, h, d);
```

## Loader Skeleton (Python)

```python
from concurrent.futures import ThreadPoolExecutor
import threading

executor = ThreadPoolExecutor(max_workers=8)
_pending = set()
_lock = threading.Lock()

def request_tile(lod, z, y, x):
    key = (lod, z, y, x)
    with _lock:
        if key in _pending:
            return bv.ChunkStatus.AlreadyPending
        _pending.add(key)
    executor.submit(_load, key)
    return bv.ChunkStatus.Accepted

def _load(key):
    lod, z, y, x = key
    # ... fetch data ...
    image.set_chunk_data_u16(lod, z, y, x, np.asarray(data, dtype=np.uint16))
    with _lock:
        _pending.discard(key)
```

## Loader Skeleton (JavaScript)

```javascript
const pending = new Set();

function requestTile(lod, z, y, x) {
    const key = `${lod}_${z}_${y}_${x}`;
    if (pending.has(key)) return JsChunkStatus.AlreadyPending;
    pending.add(key);
    fetchZarrChunk(lod, z, y, x).then(({ data, w, h, d }) => {
        image.setChunkDataU16(lod, z, y, x, data, w, h, d);
        pending.delete(key);
    }).catch(() => pending.delete(key));
    return JsChunkStatus.Accepted;
}
```

## VRAM Budget

```
VRAM per tile ≈ tile_depth × tile_height × tile_width × 2 bytes  (R16Float)
e.g. 64³ tile = 524 KB;  max_tiles=2000 ≈ 1 GB
```

In the web examples:

```javascript
const tileVram = tileDepth * tileHeight * tileWidth * 2;
const maxTiles = Math.floor((budgetBytes * 0.8) / tileVram);  // 80% headroom
```
