# Bovista

Bovista's thesis is simple: **write the core visualization once in Rust, run it anywhere.**

The same rendering engine — GPU resource management, LOD selection, virtual texture streaming, ray marching — compiles to a Python extension (via PyO3), a WebAssembly module (via wasm-bindgen), or a native Rust binary. There's no separate browser port or Python reimplementation, just idiomatic wrappers.

Bovista is built on [wgpu](https://wgpu.rs), which maps to Vulkan/Metal/DX12 on the desktop and WebGPU in the browser. The core library has no Python or browser dependencies — bindings are thin wrappers at the boundary.

Current capabilities:
- **Slice rendering** (`Image`): arbitrary-orientation plane intersection through 3D volumes
- **Volume ray marching** (`DirectVolume`, `MipVolume`, `MinipVolume`, `AverageVolume`, `IsosurfaceVolume`): one visual per mode, each exposing only the parameters that apply to it (density, attenuation, iso threshold, …)
- **Virtual texture streaming**: atlas + page table architecture, time-resolved 4D; only the tiles in view are required, but the atlas acts as a cache for recently used or prefetched tiles
- **Multi-resolution LOD**: coarse-to-fine pyramid, screen-space error-based LOD selection with a
  configurable bias
- **Pull-based remote loading**: bovista visuals publish a `wanted` set for visual chunks; loaders poll it and push bytes back
- **OME-Zarr** examples: streams tiles from S3 / HTTP / local filesystem - loading is handled at the
  application level
- **Points and lines**: for annotations, axis helpers, point clouds

| Native (PyQt6) | Browser (WebAssembly) |
| :---: | :---: |
| ![PyQt6 volume renderer](screenshots/pyqt-volume.png) | ![Web volume renderer](screenshots/web-volume.png) |
| `examples/volume_renderer/python/` | `examples/volume_renderer/web/` |

*Same renderer, same dataset, different host — both views are produced by the same Rust core.*

---

## A note on project goals, and AI usage

Obviously, if you look at the code here, I have made _extensive_ use of Claude Code in this
repository. My goal here, at least initially, is to explore different graphics concepts and
implementations. The project has gone through several _major_ refactorings already, as I use it to
evaluate different API shapes and technical concepts.

A secondary goal is to use and evaluate different AI coding tools, concepts, and workflows. While I
do read the code here, I have been doing most of the development so far through prompting. I have
mixed feelings about this on a number of levels, but an experimental solo project seems like a safe
place to do this kind of thing. I'd love to have been able to write this project myself, from
scratch, but the truth is even getting the initial scaffolding and build system set up would have
taken me weeks.

I don't know where this project goes. I don't consider it production-ready, and I'm not sure it ever
will be. The _ideas_ here, however, are very exciting to me: flexible deployment of a wgpu-based
library, using "virtual textures" (particularly for mixed-LOD volume rendering), and a fully
decoupled loading scheme have all been kicking around in my head for a long time.

---

## Examples

Examples are organized by rendering mode, with parallel Python, Web, and Rust implementations in each:

```
examples/
  slice_renderer/     — arbitrary-orientation slice plane (Image)
    python/         — PyQt6 + ThreadPoolExecutor
    web/            — Browser + WebAssembly
    rust/           — winit + wgpu (interactive native viewer)
  volume_renderer/  — direct volume rendering via ray marching (DirectVolume, …)
    python/
    web/
    rust/           — winit + wgpu (interactive native viewer)
  remote_volume_renderer/     — headless native reference: orbit a camera, encode to video
  common/           — shared example code (OME-Zarr loader, pull-loader, winit harness)
```

### Prerequisites

- **Rust** (stable) — [rustup.rs](https://rustup.rs)
- **uv** (Python package manager) — `curl -LsSf https://astral.sh/uv/install.sh | sh`
- **WASM only**: `wasm-bindgen-cli` — `cargo install wasm-bindgen-cli`

### Python
```bash
uv sync              # Compiles Rust extension and installs Python deps
uv run python examples/slice_renderer/python/remote_ome_zarr.py
uv run python examples/volume_renderer/python/volume_ome_zarr.py
```

`uv sync` recompiles automatically when Rust source changes (tracked via `cache-keys` in `pyproject.toml`).

**Controls (volume example):**
- Left-drag: Orbit camera
- Scroll: Zoom
- LOD Bias / Density / Step / Contrast sliders in the UI

---

### Web (WASM)
```bash
./build_wasm.sh                              # Compiles to WASM, outputs to examples/pkg/
python -m http.server 8000 --directory examples
open http://localhost:8000/slice_renderer/
```

The WASM build requires `wasm32-unknown-unknown` (the build script installs it automatically).

---

### Rust (native)

Interactive winit + wgpu viewers — no GUI framework, controls are mouse + keyboard, stats print to the console.

```bash
cargo run --release --example slice_renderer    # defaults to the marmoset_neurons dataset
cargo run --release --example volume_renderer  # defaults to the beechnut dataset
cargo run --release --example slice_renderer -- --zarr <path-or-https-url>
```

Pass `--help`-style flags such as `--zarr`, `--lod-bias`, `--contrast-min/--contrast-max`, and (volume only) `--mode`, `--density-mult`, `--step`. Each example prints its full control map on startup.

> Loading remote OME-Zarr over HTTPS pulls in `zarrs_http`, which links OpenSSL — make sure a system OpenSSL (or `OPENSSL_DIR`) is available at build time.

---

## Architecture

The core library is platform-agnostic Rust. Bindings add a thin layer at the edges.

```
src/
  lib.rs                    — crate root, public API
  renderer.rs               — wgpu device/queue, render pass
  camera.rs                 — orbit controls, frustum, projection
  scene.rs                  — collection of visuals
  visual.rs                 — Visual trait
  bindings_common.rs        — shared Python/WASM binding logic
  visuals/
    image.rs                — Image: slice-plane rendering
    volume.rs               — DirectVolume/MipVolume/MinipVolume/AverageVolume/IsosurfaceVolume: ray marching DVR
    virtual_texture.rs      — VirtualTextureData: atlas, page table, LOD
    atlas.rs                — AtlasAllocator: 3D texture atlas
    page_table.rs           — PageTable: 2D-array indirection texture
    gpu_structs.rs          — TileKey, TileData, TileLoaderFn, vertex/uniform structs
    points.rs               — Points
    lines.rs                — Lines
    custom.rs               — Custom (user-defined shaders)
  shaders/
    virtual_tile.wgsl       — slice shader (reads from atlas via page table)
    volume_raymarch.wgsl    — ray marching DVR shader
    point_cloud.wgsl
    lines.wgsl
  python.rs                 — PyO3 bindings (Viewer, Image, Volume, Lines, ...)
  wasm.rs                   — wasm-bindgen bindings

examples/
  slice_renderer/             — arbitrary-orientation slice plane
    python/                 — PyQt6 viewer
    web/                    — browser viewer (WebAssembly)
    rust/                   — native viewer (winit + wgpu)
  volume_renderer/          — direct volume rendering
    python/
    web/
    rust/                   — native viewer (winit + wgpu)
  remote_volume_renderer/             — headless native reference (camera orbit → video)
  common/                   — shared example code (loader, winit harness, CLI)
  pkg/                      — WASM build output (generated by build_wasm.sh, not committed)
```

### Rendering pipeline (both Image and the Volume modes)

Both renderers share the same virtual texture back-end:

1. **`VirtualTextureData::prepare()`** — per-frame LOD selection based on screen-space error; requests missing tiles via the loader callback
2. **Loader callback** (Python thread pool or JS fetch) — loads tile data asynchronously and calls `set_chunk_data_u16()` to push bytes into the pending queue
3. **`VirtualTextureData::upload_pending()`** — writes arrived tiles into the atlas 3D texture, updates the page table
4. **`Image::render()`** — single draw call; slice-plane geometry samples the atlas via the page table
5. **`DirectVolume::render()`** (and the other volume modes) — single draw call; back-face box geometry; fragment shader fires a ray per pixel and composites front-to-back through the atlas

---

## Python API (quick reference)

```python
import bovista as bv

viewer = bv.Viewer(800, 600)
viewer.initialize_with_window(handle, width, height)

# Volume ray marching. Pick one of the five mode classes; each only exposes
# the parameters that apply to its algorithm.
volume = bv.DirectVolume(viewer, lod_levels, max_tiles=2000)
volume.set_contrast(0.0, 1.0)
volume.set_density_scale(0.01)         # Direct only
volume.set_relative_step_size(1.0)
viewer.add(volume)

# Other modes:
# bv.MipVolume(viewer, lod_levels, max_tiles).set_attenuation(0.5)
# bv.MinipVolume(viewer, lod_levels, max_tiles)
# bv.AverageVolume(viewer, lod_levels, max_tiles)
# bv.IsosurfaceVolume(viewer, lod_levels, max_tiles).set_iso_threshold(0.3)

# Slice rendering — same LOD pipeline, plane intersection instead of raymarch.
image = bv.Image(viewer, lod_levels, max_tiles=500)
image.set_slice_plane(cx, cy, cz, nx, ny, nz)  # position + normal
viewer.add(image)

# Annotations
viewer.add(bv.Lines.axis_helper(viewer, 100.0))

# Pull-based tile loading. Drive this from a worker thread.
while True:
    for lod, t, z, y, x, priority in volume.wanted_keys():
        data = fetch_tile(lod, t, z, y, x)        # your fetch
        volume.set_chunk_data_u16(lod, t, z, y, x, data)

# Render loop (called by Qt timer or similar)
viewer.render_frame()
```

LOD levels are described with `bv.LevelMetadata(volume_size, chunk_size, voxel_size, scale_factor, translation)`.

**Loader model.** bovista publishes a `wanted` set every prepare — `volume.wanted_keys()` returns `[(lod, t, z, y, x, priority), ...]` sorted by priority (0 = "user is viewing this t now"; positive offsets = prefetch). Your loader thread polls this list, fetches the data, and pushes back via `volume.set_chunk_data_u16(lod, t, z, y, x, np_array)`. No callback across the Python/Rust boundary on the hot path.

**Multi-atlas.** For very large caches, pass `atlas_count=N` (1..=4) to spread the slot budget across N physical 3D textures. Useful when `max_tiles * 4 MiB` exceeds the per-resource VRAM cap.

**Time-resolved data.** Each `TileKey` carries a `t` index; use `volume.set_desired_timepoint(t)` and `volume.set_prefetch(lookahead, t_count)` to scrub through 4D OME-Zarr datasets. Adjacent timepoints stay resident in the atlas as a free cache.

---

## WASM API (quick reference)

The browser bindings mirror the Python API — same class names, lowerCamelCase methods:

```js
import init, { Viewer, DirectVolume, LevelMetadata } from './bovista.js';
await init();

const viewer = await Viewer.new('canvas-id');           // canvas element
// LevelMetadata args are 5: three [z, y, x] arrays + scale factor + translation array.
const levels = lodLevels.map(l => new LevelMetadata(
    l.volumeSize,   // [z, y, x] voxel counts at this LOD
    l.chunkSize,    // [z, y, x] voxel counts per chunk
    l.voxelSize,    // [z, y, x] world-space units per voxel
    l.scaleFactor,  // resolution ratio relative to LOD 0 (1.0, 2.0, 4.0, …)
    l.translation,  // [z, y, x] world-space origin offset
));

const volume = new DirectVolume(viewer, levels, /*maxChunks=*/2000, /*atlasCount=*/1);
viewer.addDirectVolume(volume);
volume.setContrast(0.0, 1.0);
volume.setDensityScale(0.01);

// Pull-based loader: poll wantedKeys() in a JS loop, fetch, push back.
setInterval(() => {
    const wanted = volume.wantedKeys();  // flat Uint32Array [lod, t, z, y, x, priority, ...]
    for (let i = 0; i < wanted.length; i += 6) {
        const [lod, t, z, y, x, prio] = wanted.slice(i, i + 6);
        fetchTile(lod, t, z, y, x).then(arr => {
            // Shape args are (z, y, x) numpy order — voxel counts along each axis of the tile.
            volume.setChunkDataU16(lod, t, z, y, x, arr, zShape, yShape, xShape);
        });
    }
}, 16);

viewer.renderFrame();  // hook into requestAnimationFrame
```

Add-volume methods are typed per mode (`addDirectVolume`, `addMipVolume`, …); see `examples/volume_renderer/web/` for the full pattern.

---

## Development

```bash
# Build the core library and native examples
cargo build

# Build the WASM bindings (outputs to examples/pkg/)
./build_wasm.sh

# Build the Python extension in dev mode
uv run maturin develop --features python

# Build and serve the mdBook docs
./serve_docs.sh

# Run a native example (see the Examples section for flags)
cargo run --release --example slice_renderer
```

---

## Related work

Bovista is inspired/influenced by many other cool projects:

- **[pygfx](https://github.com/pygfx/pygfx)** — Python visualization library built on wgpu-py
- **[vispy](https://github.com/vispy/vispy)** - OpenGL-based scientific visualization
- **[idetik](https://github.com/chanzuckerberg/idetik)** — OME-Zarr image viewing library
- **[three.js](https://github.com/mrdoob/three.js/)** - Web-based 3D graphics library
- **[neuroglancer](https://github.com/google/neuroglancer)** - WebGL-based volumetric image viewer
- **[viv](https://github.com/hms-dbmi/viv)** -A deck.gl-based visualization library for large image datasets (microscopy)
- **[napari](https://github.com/napari/napari)** - a fast, interactive, multi-dimensional image viewer for python
- **[kiln](https://github.com/MPanknin/kiln-render)** — Standalone TypeScript/WebGPU browser volume renderer using virtual textures. This was particularly helpful implementation in the development of Bovista.
