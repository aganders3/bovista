# Python Bindings

Bovista exposes a Python API via [PyO3](https://pyo3.rs/). The binding layer is thin: it translates Python types to Rust types, bridges the async chunk-loading pattern, and otherwise delegates directly to the core library.

<!-- toc -->

## Installation

```bash
uv sync   # builds the Rust extension and installs Python dependencies
```

or with maturin directly:

```bash
maturin develop --features python --release
```

Import as:

```python
import bovista as bv
```

## `Viewer`

The top-level object that owns the GPU device, renderer, camera, and scene.

```python
viewer = bv.Viewer(width=800, height=600)
```

**Initialization:**

- `viewer.initialize()` — headless / offscreen initialization (no window)
- `viewer.initialize_with_window(handle, width, height)` — attach to an existing native window handle (NSView on macOS, HWND on Windows, X11 Window on Linux); used when embedding in Qt or other GUI toolkits
- `viewer.run()` — open a standalone winit window and block until closed

**Camera:**

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

**Scene management:**

```python
idx = viewer.add(visual)          # add any visual; returns index
viewer.clear_visuals()            # remove all
viewer.visual_count() -> int
```

**Render loop (embedded mode):**

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

**`scale_factor` for anisotropic datasets:** `scale_factor` is a single scalar used by the LOD
selection to estimate how many screen pixels a voxel covers at this level. For isotropic 2×
downsampling use `2.0`, `4.0`, etc. For anisotropic downsampling (e.g. 2× in XY but not Z), use
`max(vz_i/vz_0, vy_i/vy_0, vx_i/vx_0)` — the largest downsampling factor across any axis. This is
a conservative approximation: it biases toward finer LODs, which is correct for a viewer where the
user can rotate freely. A future improvement would make LOD selection view-direction-aware so the
relevant axis is weighted automatically.

## `Image`

Renders arbitrary-orientation slice planes through a multi-resolution volume.

```python
image = bv.Image(
    viewer,           # must be initialized first
    levels,           # list[LevelMetadata], finest first
    max_tiles=500,    # VRAM budget in tiles
    loader=request,   # callback: (lod, z, y, x) -> ChunkStatus
)
viewer.add(image)
```

**Slice control:**

```python
image.set_slice_z(z)                              # axis-aligned Z slice
image.set_slice_y(y)
image.set_slice_x(x)
image.set_slice_plane(px, py, pz, nx, ny, nz)     # arbitrary plane
```

**Appearance:**

```python
image.set_contrast(min_val, max_val)              # 0.0–1.0 normalized
image.set_colormap(rgba_array)                    # numpy (256, 4) uint8; None = grayscale
image.set_lod_bias(bias)                          # positive = finer LOD
```

**Diagnostics:**

```python
image.set_debug_mode(True)                        # LOD tint visualization
loaded, visible = image.get_stats()               # tile counts
```

**Push tile data** (called from loader background thread):

```python
image.set_chunk_data_u16(lod, z, y, x, array)    # numpy uint16 array
```

## `Volume`

Direct volume rendering via GPU ray marching. Shares the same virtual-texture back-end as `Image`.

```python
volume = bv.Volume(
    viewer,
    levels,
    max_tiles=2000,
    loader=request,
)
viewer.add(volume)
```

**Controls:**

```python
volume.set_contrast(min_val, max_val)
volume.set_colormap(rgba_array)
volume.set_density_scale(scale)          # opacity multiplier per step
volume.set_relative_step_size(step)      # 1.0 = Nyquist; larger = faster, coarser
volume.set_lod_bias(bias)
```

**Debug modes:**

```python
volume.set_debug_mode(True)              # LOD tint (mode 1)
volume.set_atlas_debug_mode(True)        # raw atlas visualization (mode 2)
volume.set_step_debug_mode(True)         # ray-march step count heatmap (mode 3)
```

**Stats / data:**

```python
loaded, visible = volume.get_stats()
volume.set_chunk_data_u16(lod, z, y, x, array)
```

## `Points`

Point cloud rendering. Static geometry — no streaming.

```python
points = bv.Points.from_numpy(viewer, positions, colors)
# positions: numpy (N, 1, 3) float32
# colors:    numpy (N, 1, 3) float32  (RGB 0-1)
viewer.add(points)

# or a quick test shape:
viewer.add(bv.Points.test_cube(viewer, size=10))
```

## `Lines`

Line segment rendering. Each pair of vertices forms one segment.

```python
viewer.add(bv.Lines.axis_helper(viewer, length=1.0))   # RGB axis gizmo
viewer.add(bv.Lines.test_cube(viewer))
```

## `Custom`

User-defined WGSL shader. The shader must expose `vs_main` / `fs_main` entry points and declare the camera uniforms at `@group(0) @binding(0)`.

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

## `ChunkStatus`

Returned by loader callbacks:

```python
bv.ChunkStatus.Accepted       # load started; data will arrive via set_chunk_data_u16
bv.ChunkStatus.AlreadyPending # already loading; skip
bv.ChunkStatus.Rejected       # can't accept; Bovista will retry next frame
```

## `ProjectionMode`

```python
bv.ProjectionMode.Perspective
bv.ProjectionMode.Orthographic
```

## Loader Pattern (full example)

```python
from concurrent.futures import ThreadPoolExecutor
import threading
import numpy as np

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
    level = zarr_levels[lod]
    cd, ch, cw = level.chunks
    data = level[z*cd:(z+1)*cd, y*ch:(y+1)*ch, x*cw:(x+1)*cw]
    image.set_chunk_data_u16(lod, z, y, x, np.asarray(data, dtype=np.uint16))
    with _lock:
        _pending.discard(key)
```

## Binding Structure

The Python bindings live in `src/python.rs`. The `#[visual_methods(VisualType)]` procedural macro (in `bovista-codegen/`) generates the boilerplate downcast + error mapping for methods with empty bodies — keeping the binding file focused on constructors and type-conversion logic.

The `Arc<Mutex<dyn Visual>>` reference type (`VisualRef`) is shared between the Python wrapper and the `Scene`, so the viewer can call `prepare()` and `render()` on visuals without the Python wrapper holding the lock.
