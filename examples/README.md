# Bovista Examples

Bovista's core rendering engine compiles to Python, WebAssembly, and native Rust. Each example below is implemented in all three, so you can compare the API side-by-side.

## Examples

| Example | What it shows | Python | Web | Rust |
|---------|--------------|--------|-----|------|
| [Slice Viewer](slice_viewer/) | Arbitrary-orientation cross-section through a 3D volume | [slice_viewer/python/](slice_viewer/python/) | [slice_viewer/web/](slice_viewer/web/) | [slice_viewer/rust/](slice_viewer/rust/) (stub) |
| [Volume Renderer](volume_renderer/) | Direct volume rendering via GPU ray marching | [volume_renderer/python/](volume_renderer/python/) | [volume_renderer/web/](volume_renderer/web/) | [volume_renderer/rust/](volume_renderer/rust/) (stub) |

## Running the Python examples

```bash
uv sync                                                    # builds Rust extension + installs deps
uv run python examples/slice_viewer/python/remote_ome_zarr.py
uv run python examples/volume_renderer/python/volume_ome_zarr.py
```

## Running the Web examples

```bash
./build_wasm.sh                                            # compile to WASM → examples/pkg/
python -m http.server 8000 --directory examples
# open http://localhost:8000/slice_viewer/
# open http://localhost:8000/volume_renderer/
```

## Datasets

All examples stream from publicly accessible S3 buckets — no download required.

| Dataset | Format | Resolution | Provider |
|---------|--------|------------|----------|
| Marmoset neurons | OME-Zarr 0.5 (v3) | 8 LOD levels | AWS public |
| Pawpawsaurus fossil | OME-Zarr 0.5 (v3) | 4 LOD levels | AWS public |
| Beechnut CT scan | OME-Zarr 0.5 (v3) | 6 LOD levels | AWS public |
| Platybrowser embryo | OME-Zarr 0.4 (v2) | 10 LOD levels | EMBL Embassy |
| HeLa cell (idr0062) | OME-Zarr 0.4 (v2) | 3 channels | EBI |

## Architecture summary

Both rendering modes share the same virtual texture streaming back-end:

```
OME-Zarr data (S3/HTTP)
    ↓  async tile fetch
PendingChunks queue
    ↓  VirtualTextureData::prepare()
Atlas (3D GPU texture)  +  Page table (indirection)
    ↓  single draw call per frame
ImageVisual (virtual_tile.wgsl)   ← slice renderer
VolumeVisual (volume_raymarch.wgsl) ← ray march DVR
```

The loader callback signature is the same across platforms:
```
(lod: int, z: int, y: int, x: int) → ChunkStatus
```

Data is pushed back via `visual.set_chunk_data_u16(lod, z, y, x, data)`.
