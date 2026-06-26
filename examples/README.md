# Bovista Examples

Bovista's core rendering engine compiles to Python, WebAssembly, and native Rust. Each example below is implemented in all three, so you can compare the API side-by-side.

## Examples

| Example | What it shows | Python | Web | Rust |
|---------|--------------|--------|-----|------|
| [Slice Renderer](slice_renderer/) | Arbitrary-orientation cross-section through a 3D volume | [slice_renderer/python/](slice_renderer/python/) | [slice_renderer/web/](slice_renderer/web/) | [slice_renderer/rust/](slice_renderer/rust/) |
| [Volume Renderer](volume_renderer/) | Direct volume rendering via GPU ray marching | [volume_renderer/python/](volume_renderer/python/) | [volume_renderer/web/](volume_renderer/web/) | [volume_renderer/rust/](volume_renderer/rust/) |

A third native-only example, [remote_volume_renderer/](remote_volume_renderer/), renders a volume headlessly with an orbiting camera and streams the frames as video — the server-side rendering use case.

## Running the Python examples

```bash
uv sync                                                    # builds Rust extension + installs deps
uv run python examples/slice_renderer/python/remote_ome_zarr.py
uv run python examples/volume_renderer/python/volume_ome_zarr.py
```

## Running the Web examples

```bash
./build_wasm.sh                                            # compile to WASM → examples/pkg/
python -m http.server 8000 --directory examples
# open http://localhost:8000/slice_renderer/
# open http://localhost:8000/volume_renderer/
```

## Running the native (Rust) examples

```bash
cargo run --release --example slice_renderer               # defaults to marmoset_neurons
cargo run --release --example volume_renderer              # defaults to beechnut
# pass --zarr <path-or-url> for any other OME-Zarr; each prints its controls on startup
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
Image (virtual_tile.wgsl)        ← slice renderer
DirectVolume (volume_raymarch.wgsl) ← ray march DVR
```

Tile loading is pull-based and identical across platforms. Each frame
bovista publishes the set of tiles it wants; the app polls that set and
pushes data back. There is no loader callback.

- Poll: `visual.wanted_keys()` (Python) / `visual.wantedKeys()` (Web)
  returns the wanted tiles as `(lod, t, z, y, x, priority)` keys, sorted
  by priority (lower = more urgent, 0 = currently viewed).
- Push: `visual.set_chunk_data_u16(lod, t, z, y, x, data)` (Python) /
  `visual.setChunkDataU16(lod, t, z, y, x, u16, zShape, yShape, xShape)` (Web).

Cancellation is implicit — keys that leave the wanted set are simply
never re-submitted.
