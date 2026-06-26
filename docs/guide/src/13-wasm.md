# JavaScript Bindings

Bovista compiles to WebAssembly via [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/). The module loads directly in any browser with WebGPU support (Chrome 113+, Edge 113+, Safari 18+), with no server-side component.

## Building

```bash
./build_wasm.sh       # compiles Rust → WASM, generates JS bindings to examples/pkg/
```

The script installs `wasm32-unknown-unknown` if needed and invokes `wasm-bindgen`. To serve locally:

```bash
python3 -m http.server 8000 --directory examples
# open http://localhost:8000/slice_renderer/
```

## JavaScript API

The WASM module exposes camelCase names that mirror the Python API:

| Python | JavaScript |
|--------|-----------|
| `bv.Viewer(w, h)` | `await Viewer.new("canvas-id")` |
| `bv.Image(viewer, ...)` | `new Image(viewer, ...)` |
| `bv.DirectVolume(viewer, ...)` | `new DirectVolume(viewer, ...)` |
| `bv.MipVolume(viewer, ...)` | `new MipVolume(viewer, ...)` |
| `bv.IsosurfaceVolume(viewer, ...)` | `new IsosurfaceVolume(viewer, ...)` |
| `bv.LevelMetadata(...)` | `new LevelMetadata(...)` |
| `viewer.add(image)` | `viewer.addImage(image)` |
| `viewer.add(volume)` | `viewer.addDirectVolume(volume)` |
| `viewer.add(points)` | `viewer.addPoints(points)` |
| `viewer.add(lines)` | `viewer.addLines(lines)` |
| `viewer.render_frame()` | `viewer.renderFrame()` |
| `visual.wanted_keys()` | `visual.wantedKeys()` |
| `visual.set_contrast(...)` | `visual.setContrast(...)` |

## `Viewer`

```javascript
import init, { Viewer, Image, DirectVolume, MipVolume, MinipVolume,
               AverageVolume, IsosurfaceVolume, Points, Lines,
               LevelMetadata, ProjectionMode } from './bovista.js';

await init();

const viewer = await Viewer.new('canvas');  // canvas element ID

// Camera
viewer.setCameraPosition(x, y, z);
viewer.setCameraTarget(x, y, z);
viewer.setCameraUp(x, y, z);
viewer.orbitCamera(dx, dy);
viewer.panCamera(dx, dy);
viewer.zoomCamera(delta);
viewer.setCameraClipPlanes(near, far);
viewer.setCameraProjectionMode(ProjectionMode.Perspective);
viewer.setCameraOrthoHeight(h);
viewer.getCameraOrthoHeight();
viewer.getCameraDistance();

// Scene
viewer.addImage(image);            // returns index
viewer.addDirectVolume(volume);    // one add* per volume mode:
viewer.addMipVolume(volume);       //   addMipVolume / addMinipVolume /
viewer.addAverageVolume(volume);   //   addAverageVolume / addIsosurfaceVolume
viewer.addIsosurfaceVolume(volume);
viewer.clearScene();
viewer.visualCount();

// Render loop (call from requestAnimationFrame)
viewer.renderFrame();
viewer.resize(w, h);            // call on canvas resize
```

## `LevelMetadata`

```javascript
const level = new LevelMetadata(
    [depth, height, width],          // volume_size [z,y,x] at this LOD
    [chunkD, chunkH, chunkW],        // chunk_size  [z,y,x]
    [voxelD, voxelH, voxelW],        // voxel_size  [z,y,x]
    scaleFactor,                     // number, relative to level 0
    [translateD, translateH, translateW]  // translation [z,y,x]
);
```

The first three arguments are `[z, y, x]` arrays (numpy order); `scaleFactor` is a single number. Getters return arrays:

```javascript
level.volume_size  // [depth, height, width]
level.chunk_size   // [depth, height, width]
level.voxel_size   // [depth, height, width]
level.scale_factor // number
```

For anisotropic datasets (e.g. 2× downsampling in XY but not Z), pass `Math.max(vxI/vx0, vyI/vy0, vzI/vz0)` as `scaleFactor` — the largest factor across any axis. See the Python docs for details.

## `Image`

```javascript
const image = new Image(viewer, levels, maxChunks, atlasCount);
// levels:     Array<LevelMetadata>
// maxChunks:  number
// atlasCount: number (optional, default 1)

viewer.addImage(image);

// Slice control
image.setSliceZ(z);
image.setSliceY(y);
image.setSliceX(x);
image.setSlicePlane(px, py, pz, nx, ny, nz);

// Appearance
image.setContrast(min, max);       // 0.0–1.0
image.setColormap(uint8Array);     // Uint8Array of 1024 bytes (256 RGBA); empty = grayscale
image.setLodBias(bias);

// Diagnostics
image.setDebugMode(true);
image.getStats();                  // [loaded, visible]

// Pull-based tile loading
image.wantedKeys();                // Uint32Array [lod,t,z,y,x,prio, ...] (6 ints/key)
image.setChunkDataU16(lod, t, z, y, x, uint16Array, zShape, yShape, xShape);
```

## Volume (`DirectVolume`, `MipVolume`, `MinipVolume`, `AverageVolume`, `IsosurfaceVolume`)

Five classes, each constructed the same way and each exposing only the parameters that apply to it. `DirectVolume` is the default (front-to-back alpha-compositing DVR).

```javascript
const volume = new DirectVolume(viewer, levels, maxChunks, atlasCount);
viewer.addDirectVolume(volume);

// Shared by all five volume classes
volume.setContrast(min, max);
volume.setColormap(uint8Array);
volume.setRelativeStepSize(step);   // 1.0 = Nyquist
volume.setLodBias(bias);
volume.getStats();                  // [loaded, visible]

// Pull-based tile loading
volume.wantedKeys();                // Uint32Array [lod,t,z,y,x,prio, ...] (6 ints/key)
volume.setChunkDataU16(lod, t, z, y, x, uint16Array, zShape, yShape, xShape);

// DirectVolume extras
volume.setDensityScale(scale);
volume.setEarlyExitAlpha(alpha);
volume.setDebugMode(true);          // LOD tint
volume.setAtlasDebugMode(true);     // raw atlas
volume.setStepDebugMode(true);      // step count heatmap

// MipVolume extra:        mip.setAttenuation(attenuation);   // >0 = attenuated MIP
// IsosurfaceVolume extra: iso.setIsoThreshold(threshold);
// MinipVolume / AverageVolume: no extras
```

## Tile loading (pull-based)

There is no loader callback. Bovista publishes a "wanted" set every prepare/frame; the app polls it with `wantedKeys()`, fetches the tiles, and pushes data back with `setChunkDataU16(...)`. Keys are `(lod, t, z, y, x, priority)` sorted by priority (lower = more urgent, 0 = currently viewed). Cancellation is implicit: keys that leave the wanted set are no longer requested.

```javascript
const pending = new Set();

function pollTiles() {
    const w = volume.wantedKeys();   // flat Uint32Array, 6 ints per key
    for (let i = 0; i < w.length; i += 6) {
        const [lod, t, z, y, x, prio] = w.slice(i, i + 6);
        const key = `${lod}_${t}_${z}_${y}_${x}`;
        if (pending.has(key)) continue;   // already in flight
        pending.add(key);
        fetchTile(lod, t, z, y, x).then(({ data, zShape, yShape, xShape }) => {
            volume.setChunkDataU16(lod, t, z, y, x, data, zShape, yShape, xShape);
        }).finally(() => pending.delete(key));
    }
}

// Render loop
function frame() {
    pollTiles();
    viewer.renderFrame();
    requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
```

## Differences from the Python API

- **Initialization:** `Viewer.new()` is async and takes a canvas element ID; Python's `Viewer()` is sync, with a separate `initialize` call.
- **`add*` methods:** JS has one method per visual type (`addImage()`, `addDirectVolume()`, etc.) since wasm-bindgen can't overload via `PyAny`; Python uses a single polymorphic `add()`.
- **`wantedKeys()` return type:** JS returns a flat `Uint32Array` (6 ints per key); Python returns a list of tuples. Both are sorted by priority.
- **Error handling:** JS methods return `Result<T, JsValue>` (thrown exception); Python returns `PyResult<T>` (Python exception).
- **`getStats()` return type:** JS returns `number[]`; Python returns an `(int, int)` tuple.
- **`setChunkDataU16` signature:** JS takes explicit `zShape, yShape, xShape` after the data array; Python infers them from the numpy array shape.
- **Thread safety:** JS is single-threaded — `Rc<RefCell<>>` instead of `Arc<Mutex<>>`. No locking overhead, but the visual must only be touched from the main thread.

## Binding structure

The WASM bindings live in `src/wasm.rs`. As with Python, `#[visual_methods(VisualType)]` generates the downcast + error-mapping boilerplate for empty-body methods. `bindings_common.rs` provides `with_visual_ref` / `with_visual_mut` helpers that work with either `Arc<Mutex<>>` (native) or `Rc<RefCell<>>` (WASM) via a conditional type alias.

## Browser requirements

- Chrome 113+ or Edge 113+ (WebGPU on by default)
- Safari 18+ (feature flag in earlier versions)
- Firefox: behind `dom.webgpu.enabled` flag
- Check support: [webgpureport.org](https://webgpureport.org/)

WebGPU is required; WebGL is not supported. Browsers lacking WebGPU see a fallback message in the examples.
