# Quick Reference

## Source Map

| Component | File |
|-----------|------|
| Renderer (device, queue, render pass) | `src/renderer.rs` |
| Camera (orbit, pan, zoom, frustum) | `src/camera.rs` |
| Scene (visual list, frame loop) | `src/scene.rs` |
| `Visual` trait | `src/visual.rs` |
| Shared Python/WASM logic | `src/bindings_common.rs` |
| `Points` | `src/visuals/points.rs` |
| `Lines` | `src/visuals/lines.rs` |
| `Image` (slice rendering) | `src/visuals/image.rs` |
| `DirectVolume` (ray marching DVR) | `src/visuals/volume.rs` |
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
image = bv.Image(viewer, levels, max_tiles=500, atlas_count=1)
image.set_slice_z(z)
image.set_slice_y(y)
image.set_slice_x(x)
image.set_slice_plane(px, py, pz, nx, ny, nz)
image.set_contrast(min, max)
image.set_colormap(rgba_array)    # (256,4) uint8; None = grayscale
image.set_lod_bias(bias)
image.set_debug_mode(True)        # LOD tint
image.get_stats() -> (loaded, visible)
image.wanted_keys()               # [(lod, t, z, y, x, priority), ...]
image.set_chunk_data_u16(lod, t, z, y, x, array)  # numpy uint16

# Volume (ray marching) — five mode classes, same constructor:
#   bv.DirectVolume     front-to-back alpha-compositing DVR (default)
#   bv.MipVolume        maximum intensity projection
#   bv.MinipVolume      minimum intensity projection
#   bv.AverageVolume    mean projection
#   bv.IsosurfaceVolume iso-surface
volume = bv.DirectVolume(viewer, levels, max_tiles=2000, atlas_count=1)

# Shared by all five volume classes:
volume.set_contrast(min, max)
volume.set_colormap(rgba_array)
volume.set_relative_step_size(step)  # 1.0 = Nyquist
volume.set_lod_bias(bias)
volume.get_stats() -> (loaded, visible)
volume.wanted_keys()                 # [(lod, t, z, y, x, priority), ...]
volume.set_chunk_data_u16(lod, t, z, y, x, array)

# DirectVolume extras:
volume.set_density_scale(scale)
volume.set_early_exit_alpha(alpha)
volume.set_debug_mode(True)          # LOD tint
volume.set_atlas_debug_mode(True)    # raw atlas
volume.set_step_debug_mode(True)     # step count heatmap

# MipVolume extra:        volume.set_attenuation(attenuation)  # >0 = attenuated MIP
# IsosurfaceVolume extra: volume.set_iso_threshold(threshold)
# MinipVolume / AverageVolume: no extras

# Points / Lines (static geometry)
points = bv.Points.from_numpy(viewer, positions, colors)  # (N,1,3) float32
lines  = bv.Lines.axis_helper(viewer, length=1.0)

# Tile loading (pull-based): bovista publishes a "wanted" set every frame.
# Poll it, fetch the tiles, and push the data back. There is no loader callback.
for lod, t, z, y, x, priority in volume.wanted_keys():
    data = fetch_tile(lod, t, z, y, x)        # (z, y, x) uint16 numpy array
    volume.set_chunk_data_u16(lod, t, z, y, x, data)
```

## JavaScript API Summary

```javascript
import init, { Viewer, Image, DirectVolume, MipVolume, MinipVolume,
               AverageVolume, IsosurfaceVolume, Points, Lines,
               LevelMetadata, ProjectionMode } from './bovista.js';
await init();

// Viewer
const viewer = await Viewer.new('canvas-id');
viewer.renderFrame();
viewer.resize(w, h);

// Camera
viewer.setCameraPosition(x, y, z);   viewer.setCameraTarget(x, y, z);
viewer.setCameraUp(x, y, z);         viewer.orbitCamera(dx, dy);
viewer.panCamera(dx, dy);            viewer.zoomCamera(delta);
viewer.setCameraClipPlanes(near, far);
viewer.setCameraProjectionMode(ProjectionMode.Perspective);
viewer.setCameraOrthoHeight(h);      viewer.getCameraOrthoHeight();
viewer.getCameraDistance();

// Scene
viewer.addImage(image);          viewer.addDirectVolume(volume);
viewer.addMipVolume(volume);     viewer.addMinipVolume(volume);
viewer.addAverageVolume(volume); viewer.addIsosurfaceVolume(volume);
viewer.addPoints(points);        viewer.addLines(lines);
viewer.clearScene();             viewer.visualCount();

// LevelMetadata  ([z,y,x] arrays for the first three args)
const level = new LevelMetadata(
    [volumeD, volumeH, volumeW],   // volume_size [z,y,x]
    [chunkD,  chunkH,  chunkW],    // chunk_size  [z,y,x]
    [voxelD,  voxelH,  voxelW],    // voxel_size  [z,y,x]
    scaleFactor,                   // number
    [translateD, translateH, translateW]  // translation [z,y,x]
);
// Getters return arrays: level.volume_size, level.chunk_size, level.voxel_size

// Image
const image = new Image(viewer, levels, maxChunks, atlasCount);
image.setSliceZ(z);  image.setSliceY(y);  image.setSliceX(x);
image.setSlicePlane(px, py, pz, nx, ny, nz);
image.setContrast(min, max);
image.setColormap(uint8Array);       // 1024 bytes (256 RGBA); empty = grayscale
image.setLodBias(bias);
image.setDebugMode(true);
image.getStats();                    // [loaded, visible]
image.wantedKeys();                  // Uint32Array [lod,t,z,y,x,prio, ...]
image.setChunkDataU16(lod, t, z, y, x, uint16Array, zShape, yShape, xShape);

// Volume — DirectVolume / MipVolume / MinipVolume / AverageVolume / IsosurfaceVolume
const volume = new DirectVolume(viewer, levels, maxChunks, atlasCount);
volume.setContrast(min, max);        volume.setColormap(uint8Array);
volume.setRelativeStepSize(step);    volume.setLodBias(bias);
volume.getStats();                   // [loaded, visible]
volume.wantedKeys();                 // Uint32Array [lod,t,z,y,x,prio, ...]
volume.setChunkDataU16(lod, t, z, y, x, uint16Array, zShape, yShape, xShape);
// DirectVolume extras: setDensityScale, setEarlyExitAlpha,
//                      setDebugMode, setAtlasDebugMode, setStepDebugMode
// MipVolume extra:        setAttenuation        IsosurfaceVolume extra: setIsoThreshold
```

## Pull Loop (Python)

bovista publishes a "wanted" set every prepare/frame; the app polls it, fetches
the tiles, and pushes the data back. Keys are `(lod, t, z, y, x, priority)`,
sorted by priority (lower = more urgent, 0 = currently viewed). Cancellation is
implicit: keys that leave the wanted set are simply no longer requested.

```python
# Run in a worker thread so fetches don't block rendering.
while running:
    for lod, t, z, y, x, priority in volume.wanted_keys():
        data = fetch_tile(lod, t, z, y, x)   # a (z, y, x) uint16 numpy array
        volume.set_chunk_data_u16(lod, t, z, y, x, data)
```

## Pull Loop (JavaScript)

`wantedKeys()` returns a flat `Uint32Array` with 6 ints per key
(`[lod, t, z, y, x, priority, ...]`), sorted by priority.

```javascript
const w = volume.wantedKeys();
for (let i = 0; i < w.length; i += 6) {
    const [lod, t, z, y, x, prio] = w.slice(i, i + 6);
    fetchTile(lod, t, z, y, x).then(arr =>
        volume.setChunkDataU16(lod, t, z, y, x, arr, zShape, yShape, xShape));
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
