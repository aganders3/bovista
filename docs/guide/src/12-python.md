# Python Bindings

Bovista exposes a Python API via [PyO3](https://pyo3.rs/). The binding layer is thin: it converts Python types to Rust, exposes the pull-based tile-loading API, and delegates to the core library.

## Installation

```bash
uv sync   # builds the Rust extension and installs Python dependencies
```

or with maturin directly:

```bash
maturin develop --features python --release
```

```python
import bovista as bv
```

## `Viewer`

The top-level object owns the GPU device, renderer, camera, and scene.

```python
viewer = bv.Viewer(width=800, height=600)
```

Initialization:

- `viewer.initialize()` — headless / offscreen (no window)
- `viewer.initialize_with_window(handle, width, height)` — attach to a native window handle (NSView / HWND / X11 Window) for embedding in Qt or other toolkits
- `viewer.run()` — open a standalone winit window and block until closed

Camera:

```python
viewer.set_camera_position(x, y, z)
viewer.set_camera_target(x, y, z)
viewer.set_camera_up(x, y, z)
viewer.orbit_camera(delta_x, delta_y)       # drag-to-orbit
viewer.pan_camera(delta_x, delta_y)         # drag-to-pan
viewer.zoom_camera(delta)                    # scroll zoom
viewer.set_camera_clip_planes(near, far)
viewer.set_camera_projection_mode(bv.ProjectionMode.Perspective)
viewer.set_camera_projection_mode(bv.ProjectionMode.Orthographic)
viewer.set_camera_ortho_height(height)
viewer.get_camera_ortho_height() -> float
viewer.get_camera_distance() -> float
```

Scene management:

```python
idx = viewer.add(visual)          # add any visual; returns index
viewer.clear_visuals()            # remove all
viewer.visual_count() -> int
```

Render loop (embedded mode):

```python
viewer.render_frame()   # render one frame to the attached window surface
viewer.resize(w, h)     # call when the window is resized
```

## `LevelMetadata`

Describes one resolution level in a multi-resolution pyramid.

```python
level = bv.LevelMetadata(
    volume_size=(depth, height, width),   # voxels at this LOD
    chunk_size=(cd, ch, cw),              # tile size at this LOD
    voxel_size=(vz, vy, vx),             # physical size per voxel (e.g. µm)
    scale_factor=1.0,                     # relative to level 0 (finest)
    translation=(tz, ty, tx),            # optional origin offset
)
```

All sizes use `(depth/z, height/y, width/x)` ordering to match OME-Zarr / NumPy convention.

`scale_factor` is a scalar the LOD selector uses to estimate how many screen pixels a voxel covers at this level. For isotropic 2× downsampling use `2.0`, `4.0`, etc. For anisotropic downsampling (e.g. 2× in XY but not Z), use `max(vz_i/vz_0, vy_i/vy_0, vx_i/vx_0)` — the largest factor across any axis. This biases toward finer LODs, which is correct for a freely-rotating viewer. A future improvement would make LOD selection view-direction-aware.

## `Image`

Renders arbitrary-orientation slice planes through a multi-resolution volume.

```python
image = bv.Image(
    viewer,           # must be initialized first
    levels,           # list[LevelMetadata], finest first
    max_tiles=500,    # VRAM budget in tiles
    atlas_count=1,    # optional; number of atlas pages
)
viewer.add(image)
```

Slice control:

```python
image.set_slice_z(z)                              # axis-aligned Z slice
image.set_slice_y(y)
image.set_slice_x(x)
image.set_slice_plane(px, py, pz, nx, ny, nz)     # arbitrary plane
```

Appearance:

```python
image.set_contrast(min_val, max_val)              # 0.0–1.0 normalized
image.set_colormap(rgba_array)                    # numpy (256, 4) uint8; None = grayscale
image.set_lod_bias(bias)                          # positive = finer LOD
```

Diagnostics:

```python
image.set_debug_mode(True)                        # LOD tint visualization
loaded, visible = image.get_stats()               # tile counts
```

Tile streaming (pull-based; see [Tile loading](#tile-loading-pull-based)):

```python
for lod, t, z, y, x, priority in image.wanted_keys():
    ...                                           # fetch + push
image.set_chunk_data_u16(lod, t, z, y, x, array)  # numpy uint16 array
```

## Volume classes

Direct volume rendering via GPU ray marching, sharing the same virtual-texture back-end as `Image`. There are five classes — one per ray-marching mode. They share an identical constructor and the common controls below; each adds the parameters relevant to its mode.

| Class | Mode | Mode-specific params |
|-------|------|----------------------|
| `bv.DirectVolume` | Front-to-back alpha compositing (default DVR) | `set_density_scale`, `set_early_exit_alpha`, `set_debug_mode`, `set_atlas_debug_mode`, `set_step_debug_mode` |
| `bv.MipVolume` | Maximum intensity projection (`set_attenuation > 0` → napari-style attenuated MIP) | `set_attenuation` |
| `bv.MinipVolume` | Minimum intensity projection | — |
| `bv.AverageVolume` | Mean intensity along the ray | — |
| `bv.IsosurfaceVolume` | First-hit isosurface with Phong shading | `set_iso_threshold` |

```python
volume = bv.DirectVolume(
    viewer,
    levels,
    max_tiles=2000,
    atlas_count=1,    # optional
)
viewer.add(volume)
```

Common controls (all five classes):

```python
volume.set_contrast(min_val, max_val)
volume.set_colormap(rgba_array)
volume.set_relative_step_size(step)      # 1.0 = Nyquist; larger = faster, coarser
volume.set_lod_bias(bias)
```

`DirectVolume` extras:

```python
volume.set_density_scale(scale)          # opacity multiplier per step
volume.set_early_exit_alpha(alpha)       # terminate ray when opacity saturates
volume.set_debug_mode(True)              # LOD tint (mode 1)
volume.set_atlas_debug_mode(True)        # raw atlas visualization (mode 2)
volume.set_step_debug_mode(True)         # ray-march step count heatmap (mode 3)
```

`MipVolume` extra: `volume.set_attenuation(strength)` — `0` is plain MIP; `> 0` attenuates.
`IsosurfaceVolume` extra: `volume.set_iso_threshold(t)` — normalized 0–1 first-hit level.

Stats / tile data (common to all):

```python
loaded, visible = volume.get_stats()
for lod, t, z, y, x, priority in volume.wanted_keys():
    ...
volume.set_chunk_data_u16(lod, t, z, y, x, array)
```

## `Points`, `Lines`, `Custom`

`Points` renders a point cloud; `Lines` renders segments (each vertex pair is one segment). Both are static geometry — no streaming.

```python
points = bv.Points.from_numpy(viewer, positions, colors)
# positions: numpy (N, 1, 3) float32
# colors:    numpy (N, 1, 3) float32  (RGB 0-1)
viewer.add(points)
viewer.add(bv.Points.test_cube(viewer, size=10))       # quick test shape

viewer.add(bv.Lines.axis_helper(viewer, length=1.0))   # RGB axis gizmo
viewer.add(bv.Lines.test_cube(viewer))
```

`Custom` runs a user-defined WGSL shader. It must expose `vs_main` / `fs_main` entry points and declare the camera uniforms at `@group(0) @binding(0)`.

```python
shader = """
struct Uniforms { view_proj: mat4x4<f32> }
@group(0) @binding(0) var<uniform> uniforms: Uniforms;

@vertex
fn vs_main(@location(0) pos: vec3<f32>) -> @builtin(position) vec4<f32> {
    return uniforms.view_proj * vec4<f32>(pos, 1.0);
}
@fragment
fn fs_main() -> @location(0) vec4<f32> { return vec4<f32>(1.0, 0.5, 0.0, 1.0); }
"""

layout = bv.VertexBufferLayout(
    12,                          # stride in bytes
    [(0, "Float32x3", 0)]       # (location, format, byte_offset)
)

custom = bv.Custom.new(viewer, shader, vertices.tobytes(), layout, "triangle_list")
viewer.add(custom)
```

## Tile loading (pull-based)

Bovista uses a **pull** model: each frame it publishes the tiles it wants, sorted by priority, and the application polls that set and pushes data back. There is no loader callback and no hot-path FFI; cancellation is implicit, since keys that leave the wanted set are never re-fetched.

- `image.wanted_keys()` (identical on every volume class) returns a list of `(lod, t, z, y, x, priority)` tuples sorted by priority (lower = more urgent; `0` = currently viewed). `t` is the timepoint.
- `image.set_chunk_data_u16(lod, t, z, y, x, array)` pushes a fetched tile back in. `array` is a NumPy `uint16` array shaped `(z, y, x)`.

```python
import numpy as np

def poll_tiles():
    for lod, t, z, y, x, priority in image.wanted_keys():
        level = zarr_levels[lod]
        cd, ch, cw = level.chunks
        data = level[z*cd:(z+1)*cd, y*ch:(y+1)*ch, x*cw:(x+1)*cw]
        image.set_chunk_data_u16(lod, t, z, y, x, np.asarray(data, dtype=np.uint16))
```

`ProjectionMode` values are `bv.ProjectionMode.Perspective` and `bv.ProjectionMode.Orthographic`.

### Loader pattern (full example)

A background thread polls `wanted_keys()` and dispatches fetches via a thread pool; results are pushed back through `set_chunk_data_u16`. The loop tracks what is in flight to avoid double-fetching; keys that leave the wanted set are never re-submitted.

```python
from concurrent.futures import ThreadPoolExecutor
import threading
import numpy as np

executor = ThreadPoolExecutor(max_workers=8)

def start_loader(volume):
    in_flight = set()
    lock = threading.Lock()
    stop = threading.Event()

    def load(lod, t, z, y, x):
        level = zarr_levels[lod]
        cd, ch, cw = level.chunks
        return np.asarray(
            level[z*cd:(z+1)*cd, y*ch:(y+1)*ch, x*cw:(x+1)*cw],
            dtype=np.uint16,
        )

    def on_loaded(key, future):
        with lock:
            in_flight.discard(key)
        data = future.result()
        if data is not None:
            volume.set_chunk_data_u16(*key, data)

    def poll():
        while not stop.is_set():
            for lod, t, z, y, x, _priority in volume.wanted_keys():
                key = (lod, t, z, y, x)
                with lock:
                    if key in in_flight or len(in_flight) >= 8:
                        continue
                    in_flight.add(key)
                future = executor.submit(load, *key)
                future.add_done_callback(lambda f, k=key: on_loaded(k, f))
            stop.wait(0.02)

    threading.Thread(target=poll, daemon=True).start()
```

## Binding structure

The Python bindings live in `src/python.rs`. The `#[visual_methods(VisualType)]` proc-macro (in `bovista-codegen/`) generates the downcast + error-mapping boilerplate for empty-body methods, keeping the binding file focused on constructors and type conversion.

The `Arc<Mutex<dyn Visual>>` reference type (`VisualRef`) is shared between the Python wrapper and the `Scene`, so the viewer can call `prepare()` and `render()` without the wrapper holding the lock.
