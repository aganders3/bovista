# WebAssembly & Browser

Bovista compiles to WebAssembly via [wasm-bindgen](https://rustwasm.github.io/wasm-bindgen/). The resulting module loads directly in any browser with WebGPU support (Chrome 113+, Edge 113+, Safari 18+) with no server-side component.

<!-- toc -->

## Building

```bash
./build_wasm.sh       # compiles Rust → WASM, generates JS bindings to examples/pkg/
```

The script installs `wasm32-unknown-unknown` if needed and invokes `wasm-bindgen`.

To serve locally:

```bash
python3 -m http.server 8000 --directory examples
# open http://localhost:8000/slice_viewer/
```

## JavaScript API

The WASM module exposes camelCase JavaScript names. The correspondence to the Python API is intentional:

| Python | JavaScript |
|--------|-----------|
| `bv.Viewer(w, h)` | `await JsViewer.new("canvas-id")` |
| `bv.Image(viewer, ...)` | `new JsImageVisual(viewer, ...)` |
| `bv.Volume(viewer, ...)` | `new JsVolumeVisual(viewer, ...)` |
| `bv.LevelMetadata(...)` | `new JsLevelMetadata(...)` |
| `bv.ChunkStatus` | `JsChunkStatus` |
| `viewer.add(image)` | `viewer.addImage(image)` |
| `viewer.add(volume)` | `viewer.addVolume(volume)` |
| `viewer.add(points)` | `viewer.addPoints(points)` |
| `viewer.add(lines)` | `viewer.addLines(lines)` |
| `viewer.render_frame()` | `viewer.renderFrame()` |
| `visual.set_contrast(...)` | `visual.setContrast(...)` |

## `JsViewer`

```javascript
import init, { JsViewer, JsImageVisual, JsVolumeVisual,
               JsLevelMetadata, JsChunkStatus } from './pkg/bovista.js';

await init();

const viewer = await JsViewer.new('canvas');  // canvas element ID

// Camera
viewer.setCameraPosition(x, y, z);
viewer.setCameraTarget(x, y, z);
viewer.setCameraUp(x, y, z);
viewer.orbitCamera(dx, dy);
viewer.panCamera(dx, dy);
viewer.zoomCamera(delta);
viewer.setCameraClipPlanes(near, far);
viewer.setCameraProjectionMode(JsProjectionMode.Perspective);
viewer.setCameraOrthoHeight(h);
viewer.getCameraOrthoHeight();
viewer.getCameraDistance();

// Scene
viewer.addImage(imageVisual);   // returns index
viewer.addVolume(volumeVisual);
viewer.clearScene();
viewer.visualCount();

// Render loop (call from requestAnimationFrame)
viewer.renderFrame();
viewer.resize(w, h);            // call on canvas resize
```

## `JsLevelMetadata`

```javascript
const level = new JsLevelMetadata(
    width, height, depth,        // volume dimensions at this LOD
    chunkW, chunkH, chunkD,      // tile size
    voxelW, voxelH, voxelD,      // physical voxel size
    scaleFactor,                  // relative to level 0
    translateX, translateY, translateZ
);
```

Note: the constructor takes individual numbers (no objects). Getters return arrays:

```javascript
level.volume_size  // [depth, height, width]
level.chunk_size   // [depth, height, width]
level.voxel_size   // [depth, height, width]
level.scale_factor // number
```

For anisotropic datasets (e.g. 2× downsampling in XY but not Z), pass `Math.max(vxI/vx0, vyI/vy0, vzI/vz0)` as `scaleFactor` — the largest downsampling factor across any axis. See the Python API docs for details.

## `JsImageVisual`

```javascript
const image = new JsImageVisual(viewer, levels, maxTiles, chunkLoader);
// levels:      Array<JsLevelMetadata>
// maxTiles:    number
// chunkLoader: (lod, z, y, x) => JsChunkStatus

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

// Push tile data
image.setChunkDataU16(lod, z, y, x, uint16Array, width, height, depth);
```

## `JsVolumeVisual`

```javascript
const volume = new JsVolumeVisual(viewer, levels, maxTiles, chunkLoader);
viewer.addVolume(volume);

volume.setContrast(min, max);
volume.setColormap(uint8Array);
volume.setDensityScale(scale);
volume.setRelativeStepSize(step);   // 1.0 = Nyquist
volume.setLodBias(bias);

// Debug
volume.setDebugMode(true);          // LOD tint
volume.setAtlasDebugMode(true);     // raw atlas
volume.setStepDebugMode(true);      // step count heatmap

volume.getStats();                  // [loaded, visible]
volume.setChunkDataU16(lod, z, y, x, uint16Array, width, height, depth);
```

## Loader Pattern (full example)

```javascript
const pending = new Set();
const cache = new Map();

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

// Render loop
function frame() {
    viewer.renderFrame();
    requestAnimationFrame(frame);
}
requestAnimationFrame(frame);
```

## Differences from the Python API

**Initialization:** `JsViewer.new()` is async (returns a Promise) and takes a canvas element ID; Python's `Viewer()` is sync and initialization is a separate call.

**`add*` methods:** JavaScript has `addImage()` and `addVolume()` as distinct methods since wasm-bindgen doesn't support overloading via `PyAny`. Python uses a single polymorphic `add()`.

**Error handling:** JavaScript methods return `Result<T, JsValue>` which becomes a thrown exception on error. Python methods return `PyResult<T>` which becomes a Python exception.

**`getStats()` return type:** JavaScript returns `number[]` (array); Python returns `(int, int)` tuple.

**`setChunkDataU16` signature:** JavaScript takes explicit `width, height, depth` parameters after the data array. Python infers dimensions from the numpy array shape.

**Thread safety:** JavaScript is single-threaded — `Rc<RefCell<>>` instead of `Arc<Mutex<>>`. No locking overhead, but the visual must only be accessed from the main thread.

## Binding Structure

The WASM bindings live in `src/wasm.rs`. Like the Python bindings, the `#[visual_methods(VisualType)]` proc-macro generates the downcast + error-mapping boilerplate for empty-body methods.

`bindings_common.rs` provides `with_visual_ref` and `with_visual_mut` helpers that work with either `Arc<Mutex<>>` (native) or `Rc<RefCell<>>` (WASM) via a conditional type alias.

## Browser Requirements

- Chrome 113+ or Edge 113+ (WebGPU enabled by default)
- Safari 18+ (requires WebGPU feature flag in earlier versions)
- Firefox: behind `dom.webgpu.enabled` flag
- Check browser support: [webgpureport.org](https://webgpureport.org/)

WebGPU is required; WebGL is not supported. If a user's browser lacks WebGPU, the examples display a fallback message.
