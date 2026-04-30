# Bovista Examples

This directory contains examples demonstrating Bovista's capabilities.

## Current Examples

### Python Example

**`remote_ome_zarr.py`** - Remote cloud-native OME-Zarr visualization
- Loads large multi-scale datasets directly from remote S3 storage (no download!)
- Demonstrates the unified `Image.from_chunked()` API
- Multi-resolution LOD pyramid with automatic level selection
- On-demand tile loading with LRU cache management
- Only fetches visible chunks from the cloud
- Works with TB-scale datasets that don't fit in memory
- Real-world example using public datasets from EMBL

**Features demonstrated:**
- ✅ Multi-LOD chunked rendering
- ✅ Remote data streaming from S3
- ✅ LRU cache with configurable limits
- ✅ Hierarchical visibility determination
- ✅ Backpressure control
- ✅ Real-time loading statistics

**Requirements:**
```bash
pip install zarr fsspec requests aiohttp
```

**Run:**
```bash
cd ..  # Go to project root
uv sync  # Install dependencies and build Rust extension
uv run python examples/remote_ome_zarr.py
```

**Dataset:**
The example loads a real 11.4k × 25.9k × 27.5k voxel zebrafish embryo dataset from EMBL's public S3 bucket, with 10 LOD levels optimized for cloud-native streaming.

### Interactive Controls

- **Left-drag**: Orbit camera around target
- **Scroll**: Zoom in/out
- **Right-drag**: Rotate slice plane (if enabled)

### What You'll See

The example displays:
- Multi-resolution pyramid information for each LOD level
- Real-time tile loading statistics
- Interactive 3D visualization with automatic LOD selection
- Smooth navigation through massive datasets

## Architecture Overview

The example demonstrates Bovista's unified image rendering architecture:

```
ImageVisual (unified visual for all image types)
└── TiledStrategy (multi-resolution with chunked loading)
    ├── LOD Pyramid (10 levels from 512x to 1x downsampling)
    ├── Visibility Determination (frustum + plane-AABB intersection)
    ├── Tile Loading (async Python callback to zarr)
    └── LRU Cache (automatic eviction of old tiles)
```

### Key Components

1. **ChunkLoaderObject** - Python object that handles async loading
   - Bovista calls `request_chunk(tile_request)` to request data
   - Python loads data asynchronously and calls back with `set_chunk_data()`
   - Decouples rendering (Rust) from data loading (Python)

2. **TiledStrategy** - Rust implementation of multi-LOD rendering
   - Manages LOD pyramid hierarchy
   - Determines visible tiles at each frame
   - Maintains LRU cache of loaded tiles
   - Implements backpressure control

3. **ImageVisual** - Unified rendering
   - Single render pipeline for all tiles
   - Stencil-based LOD blending (prevents z-fighting)
   - Per-tile bind groups with caching

## Development Status

**Working:**
- ✅ Remote OME-Zarr loading from S3
- ✅ Multi-LOD rendering with automatic level selection
- ✅ LRU cache management
- ✅ Chunked loading with backpressure
- ✅ Python async integration

**Removed (Broken due to API changes):**
- ❌ Rust examples (point_cloud.rs, image_simple.rs, wasm_zarr.rs)
- ❌ test_new_image_visual.py (texture dimension issues)

The Rust examples need to be updated to the new API. Contributions welcome!

## Performance

The chunked rendering system is designed for massive datasets:
- **Memory usage**: Constant, independent of dataset size
- **Loading**: Only visible tiles are fetched
- **Cache**: Automatic LRU eviction keeps memory bounded
- **LOD**: Automatic resolution selection based on zoom level

Example performance with platy-raw dataset (11.4k × 25.9k × 27.5k):
- 10 LOD levels (9.2M total tiles)
- ~10-50 tiles visible per frame (depends on zoom)
- ~100MB GPU memory usage with 1000 tile cache
- 60 FPS rendering

## Troubleshooting

**Import Error: No module named 'bovista'**
```bash
cd .. && uv sync
```

**Zarr/fsspec not found**
```bash
pip install zarr fsspec requests aiohttp
```

**Black screen / No rendering**
- Check that GPU/WebGPU is available
- Try updating graphics drivers
- Check console for error messages
- Verify dataset URL is accessible

**Slow loading**
- Check network connection
- Try reducing `max_loaded_chunks` parameter
- Dataset may be rate-limited on server side

## Next Steps

After exploring the example:
1. Read `NEXT_STEPS.md` for unification architecture details
2. Check `UNIFICATION_PROGRESS.md` for implementation status
3. See `API_DESIGN.md` for API documentation
4. Look at `LOD_SELECTION.md` for LOD algorithm details
5. Check `ARCHITECTURE.md` for system overview

## Contributing

Want to contribute examples? We'd love to see:
- Updated Rust examples using the new API
- WASM example with web UI
- Jupyter notebook integration
- napari plugin example
- Custom chunk loader examples (HDF5, TIFF, etc.)

See the main README for contribution guidelines.
