# Bovista OME-Zarr Viewer

Interactive web viewer for OME-Zarr datasets with multi-resolution support and async chunk loading.

## Files

- `index.html` (117 lines) - HTML structure
- `style.css` (262 lines) - Styling
- `main.js` (874 lines) - Application logic

**Total: 1253 lines** (vs 1432 in single-file version, **-179 lines / -12.5%**)

## Usage

Serve this directory with a web server that supports WASM:

```bash
# From the web/ directory
python -m http.server 8000
```

Then navigate to `http://localhost:8000/remote_ome_zarr/`

## Features

- Multi-resolution LOD support
- Async chunk loading from remote Zarr stores
- Interactive camera controls (orbit, zoom)
- Slice plane manipulation
- Contrast adjustment
- Debug visualization mode
- Support for both OME-Zarr 0.4 (Zarr v2) and 0.5 (Zarr v3)

## Architecture

This example mirrors the Python `examples/remote_ome_zarr.py` implementation:

1. **Dataset configuration** - Predefined datasets with metadata
2. **Chunk loading** - Async loading via `zarrita` library
3. **Visual creation** - Uses `JsTiledImageVisual` with prototype chain inheritance from `JsImageVisual`
4. **Rendering loop** - Continuous render with WebGPU

## Controls

- **Left click + drag**: Orbit camera
- **Right click + drag horizontal**: Move slice offset
- **Right click + drag vertical**: Rotate slice (Y axis)
- **Shift + right drag vertical**: Rotate slice (X axis)
- **Scroll**: Zoom camera
- **D key**: Toggle debug mode
