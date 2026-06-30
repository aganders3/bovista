import * as zarr from 'https://cdn.jsdelivr.net/npm/zarrita@latest/+esm';

// Logical atlas tile size [z, y, x], uniform across all LOD levels — bovista's
// *virtual* tiling, decoupled from the dataset's per-LOD storage chunking
// (bovista's VT assumes one tile size for all LODs). Set per-dataset in
// loadDataset() from the LOD-0 storage chunk (capped), so each tile maps to
// ~one stored chunk: a fixed cube like 64³ would span dozens of thin storage
// chunks (e.g. chunk_z=1, as in idr0062) and fire a fetch storm.
let logicalTile = [64, 64, 64];
const MAX_LOGICAL_TILE = 128;

// Configuration
const DATASETS = {
    idr0062: {
        url: 'https://uk1s3.embassy.ebi.ac.uk/idr/zarr/v0.4/idr0062A/6001240.zarr',
        description: 'Human Mitotic Cell (HeLa) - 3 channels, 3 LOD levels',
        cameraDistance: 200.0,
        zarrVersion: 2,  // OME-Zarr 0.4 uses Zarr v2
    },
    marmoset_neurons: {
        url: 'https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/marmoset_neurons.ome.zarr',
        description: 'Marmoset Neurons - Multi-resolution 3D microscopy',
        cameraDistance: 300.0,
        zarrVersion: 3,  // OME-Zarr 0.5 uses Zarr v3
    },
    pawpawsaurus: {
        url: 'https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/pawpawsaurus.ome.zarr',
        description: 'Pawpawsaurus Dinosaur Fossil - CT scan',
        cameraDistance: 500.0,
        zarrVersion: 3,  // OME-Zarr 0.5 uses Zarr v3
    },
    beechnut: {
        url: 'https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/beechnut.ome.zarr',
        description: 'Beechnut - High-resolution CT scan',
        cameraDistance: 0.05,  // Very small physical size (~30mm)
        zarrVersion: 3,  // OME-Zarr 0.5 uses Zarr v3
    },
    exaspim_844958: {
        url: 'https://aind-open-data.s3.amazonaws.com/exaSPIM_844958_2026-04-13_12-25-27/SPIM.ome.zarr/tile_000000_ch_561.zarr',
        description: 'AIND exaSPIM 844958 - tile 000000 ch 561',
        cameraDistance: 10000.0,
        zarrVersion: 2,  // AIND exaSPIM tiles are OME-Zarr 0.4 (Zarr v2)
    },
    zebrahub_zmns001: {
        // 5D TCZYX, 2 channels (h2afva, mezzo), 5 LODs, ~780µm extent. Data sits
        // low in the u16 range (window ~0–1500) — use middle-drag / contrast to
        // brighten. t is fixed at 0 (the time axis isn't exposed here).
        url: 'https://public.czbiohub.org/royerlab/zebrahub/imaging/multi-view/ZMNS001.ome.zarr',
        description: 'Zebrahub ZMNS001 - zebrafish embryo, 2-channel light-sheet',
        cameraDistance: 1000.0,
        zarrVersion: 2,  // OME-Zarr 0.4 (Zarr v2)
    }
};

// Cap on in-flight logical-tile loads (shared across channels). Each tile may
// still fan out to a few storage-chunk fetches, so keep this modest to avoid
// ERR_INSUFFICIENT_RESOURCES (too many concurrent requests).
// TODO(loader): this caps logical tiles, not actual network requests. A tile's
// `zarr.get` fans out to N storage-chunk fetches we don't throttle. Wrap the
// zarrita FetchStore (or global fetch) in a semaphore (~16 concurrent) for a
// real request cap — belt-and-suspenders now that tiles are chunk-aligned.
const MAX_PENDING_CHUNKS = 32;

// Global State
let wasmModule = null;
let wasmInitialized = false;
let viewer = null;
let device = null;
let queue = null;
let context = null;
let format = null;
// One Image visual per channel. Length 1 for single-channel data (Normal blend,
// identical to before); for multi-channel, each visual gets its own colormap and
// additive blending so the channels composite order-independently.
let tiledImages = [];
let zarrStore = null;
let zarrRoot = null;
let zarrArrays = [];
let numChannels = 1;
let lodLevels = [];

// Per-channel base colors for multi-channel additive display (black→color ramps).
const CHANNEL_COLORS = [
    [255, 0, 0],     // red
    [0, 255, 0],     // green
    [0, 128, 255],   // blue
    [255, 0, 255],   // magenta
    [0, 255, 255],   // cyan
    [255, 255, 0],   // yellow
    [255, 255, 255], // white
];

// Build a 256-entry RGBA colormap LUT (1024 bytes) ramping black→`rgb`, with the
// channel value carried in the RGB magnitude (alpha = 255). With the premultiplied
// additive pipeline this yields per-channel intensity × hue, summed across channels.
function makeChannelColormap(rgb) {
    const lut = new Uint8Array(1024);
    for (let i = 0; i < 256; i++) {
        const f = i / 255;
        lut[i * 4 + 0] = Math.round(rgb[0] * f);
        lut[i * 4 + 1] = Math.round(rgb[1] * f);
        lut[i * 4 + 2] = Math.round(rgb[2] * f);
        lut[i * 4 + 3] = 255;
    }
    return lut;
}

// Helpers applying a per-visual operation across all channel visuals.
const visualsReady = () => tiledImages.length > 0;
const applyContrastAll = (min, max) => { for (const v of tiledImages) v.setContrast(min, max); };
const applyDebugModeAll = (on) => { for (const v of tiledImages) v.setDebugMode(on); };
const applyLodBiasAll = (b) => { for (const v of tiledImages) v.setLodBias(b); };
const totalStats = () => {
    let loaded = 0, visible = 0;
    for (const v of tiledImages) { const s = v.getStats(); loaded += s[0]; visible += s[1]; }
    return [loaded, visible];
};

let volumeCenter = [0, 0, 0];
let volumeScale = 1.0;  // Largest dimension of volume for adaptive zoom
let cameraDistance = 1.0;  // Distance from target (synced from Rust after each zoom)
let animationFrameId = null;

let contrastDebounceTimer = null;

// Chunk loading state (matching Python's pending_futures)
// Rust TiledStrategy handles the actual tile cache with LRU eviction
const pendingLoads = new Set();
let loadedChunkCount = 0;

// Mouse interaction state
let isDragging = false;
let isRightDragging = false;
let isMiddleDragging = false;
let lastMouseX = 0;
let lastMouseY = 0;
let windowCenter = 0.5;
let windowWidth = 1.0;
let sliceAngleX = 0.0;
let sliceAngleY = 0.0;
let sliceOffset = 0.0;
let debugMode = false;

// UI Elements
const canvas = document.getElementById('canvas');
const statusEl = document.getElementById('status-text');
const loadedChunksEl = document.getElementById('loaded-chunks');
const visibleChunksEl = document.getElementById('visible-chunks');
const pendingLoadsEl = document.getElementById('pending-loads');
const sliceOffsetEl = document.getElementById('slice-offset');

// Utility Functions
function setStatus(message, className = 'status-loading') {
    statusEl.textContent = message;
    statusEl.className = className;
}

function resizeCanvas() {
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    const newWidth = Math.floor(rect.width * dpr);
    const newHeight = Math.floor(rect.height * dpr);

    canvas.width = newWidth;
    canvas.height = newHeight;

    // Notify the viewer of the resize
    if (viewer) {
        viewer.resize(newWidth, newHeight);
    }
}

// Chunk Loading — pull-based.
//
// Bovista publishes the set of tiles it wants each frame via
// `image.wantedKeys()` (a flat Uint32Array of [lod, t, z, y, x,
// priority, ...] sorted by priority). A polling loop reads that,
// dispatches fetches for new keys, and pushes results back via
// `setChunkDataU16`. Cancellation is implicit — keys that leave the
// wanted set never get re-submitted.
let loaderPollHandle = null;
function startLoaderPoll() {
    if (loaderPollHandle !== null) return;
    loaderPollHandle = setInterval(() => {
        if (tiledImages.length === 0) return;
        // Poll every channel's visual; each fetches its own channel's tiles.
        for (let c = 0; c < tiledImages.length; c++) {
            const w = tiledImages[c].wantedKeys();
            for (let i = 0; i < w.length; i += 6) {
                const lod = w[i], t = w[i+1], z = w[i+2], y = w[i+3], x = w[i+4];
                const key = `${c}:${lod}:${t}:${z}:${y}:${x}`;
                if (pendingLoads.has(key)) continue;
                if (pendingLoads.size >= MAX_PENDING_CHUNKS) break;
                pendingLoads.add(key);
                pendingLoadsEl.textContent = pendingLoads.size;
                loadChunkAsync(c, lod, t, z, y, x, key);
            }
        }
    }, 25);
}

/**
 * Async chunk loading from Zarr
 */
async function loadChunkAsync(c, lod, t, z, y, x, key) {
    try {
        const zarrArray = zarrArrays[lod];
        if (!zarrArray) {
            throw new Error(`No array for LOD ${lod}`);
        }

        const shape = zarrArray.shape;

        const ndim = shape.length;
        const zIdx = ndim - 3;
        const yIdx = ndim - 2;
        const xIdx = ndim - 1;

        const [chunkDepth, chunkHeight, chunkWidth] = logicalTile;

        const zStart = z * chunkDepth;
        const yStart = y * chunkHeight;
        const xStart = x * chunkWidth;
        const zEnd = Math.min(zStart + chunkDepth, shape[zIdx]);
        const yEnd = Math.min(yStart + chunkHeight, shape[yIdx]);
        const xEnd = Math.min(xStart + chunkWidth, shape[xIdx]);

        let selection;
        if (ndim === 5) {
            // TCZYX - index into time and channel, slice spatial dims
            selection = [
                t,  // T - index from bovista's wanted set
                c,  // C - this visual's fixed channel (index, not slice)
                zarr.slice(zStart, zEnd),
                zarr.slice(yStart, yEnd),
                zarr.slice(xStart, xEnd)
            ];
        } else if (ndim === 4) {
            // CZYX - index into channel, slice spatial dims
            selection = [
                c,  // C - this visual's fixed channel (index, not slice)
                zarr.slice(zStart, zEnd),
                zarr.slice(yStart, yEnd),
                zarr.slice(xStart, xEnd)
            ];
        } else {
            // ZYX - no channel, just slice spatial dims
            selection = [
                zarr.slice(zStart, zEnd),
                zarr.slice(yStart, yEnd),
                zarr.slice(xStart, xEnd)
            ];
        }

        let chunk;
        try {
            chunk = await zarr.get(zarrArray, selection);
        } catch (error) {
            if (error.message && (error.message.includes('404') || error.message.includes('Not Found'))) {
                return; // Skip this chunk, it's empty/doesn't exist
            }
            throw error; // Re-throw other errors
        }

        let data = chunk.data;

        if (data.length === 0 || data.byteLength === 0) {
            console.error(`Empty data for chunk ${lod}:${z}:${y}:${x}`);
            throw new Error('Empty chunk data');
        }

        // Tile extents come from the RETURNED chunk shape (z, y, x — scalar
        // t/c dims already dropped), never the requested slice bounds, so they
        // always match u16data.length. setChunkDataU16 wants (z_shape, y_shape,
        // x_shape) in numpy order.
        const sh = chunk.shape;
        const dz = sh[sh.length - 3];
        const dy = sh[sh.length - 2];
        const dx = sh[sh.length - 1];

        // Hand the native tile for this channel to bovista — it normalizes the
        // full dtype range to [0, 1] itself, so no client-side rescaling.
        const visual = tiledImages[c];
        if (visual) {
            if (data instanceof Uint16Array) {
                visual.setChunkDataU16(lod, t, z, y, x, data, dz, dy, dx);
            } else {
                const src = data instanceof Uint8Array ? data : new Uint8Array(data.buffer || data);
                visual.setChunkDataU8(lod, t, z, y, x, src, dz, dy, dx);
            }
            loadedChunkCount++;
        }

    } catch (error) {
        console.error(`[ChunkLoader] Error loading ${key}:`, error);
    } finally {
        pendingLoads.delete(key);
        pendingLoadsEl.textContent = pendingLoads.size;
    }
}

// Slice Plane Control
function updateSlicePlane() {
    if (tiledImages.length === 0) return;

    // bovista computes the plane normal from the two angles + offset and
    // returns it so we can face the slice head-on in orthographic mode.
    // All channels share the same plane.
    let n;
    for (const visual of tiledImages) {
        n = visual.setSliceFromAngles(
            volumeCenter[0], volumeCenter[1], volumeCenter[2],
            sliceAngleX, sliceAngleY, sliceOffset,
        );
    }

    if (viewer.getCameraProjectionMode() === wasmModule.ProjectionMode.Orthographic) {
        viewer.alignCameraToSlice(
            volumeCenter[0], volumeCenter[1], volumeCenter[2],
            n[0], n[1], n[2], volumeScale * 2.0,
        );
    }

    // Use adaptive precision based on volume scale
    const precision = volumeScale < 1.0 ? 6 : volumeScale < 10 ? 3 : 1;
    sliceOffsetEl.textContent = sliceOffset.toFixed(precision);
}

// Actual atlas byte size for n tiles at the given tile dimensions. The
// allocator lays slots out in a cols×rows×layers grid (cube-ish), so the
// real texture rounds up past n. R16Float = 2 bytes/voxel.
function atlasSizeBytes(n, tileW, tileH, tileD) {
    const cols   = Math.ceil(Math.cbrt(n));
    const rows   = Math.ceil(Math.cbrt(n));
    const layers = Math.ceil(n / (cols * rows));
    return cols * tileW * rows * tileH * layers * tileD * 2;
}

// Largest per-channel slot count whose atlas fits the VRAM budget, split
// across the N channel atlases. This doesn't try to predict the working set
// (which depends on slice orientation + tile shape — e.g. thin z-chunks make
// a side-on slice touch far more tiles than a top-down one); it just hands
// the atlas as many slots as the budget allows. If even that's too few for a
// given orientation, the library logs a thrash warning (see VirtualTextureData).
function computeMaxChunks() {
    if (lodLevels.length === 0) return 512;
    const [tileZ, tileY, tileX] = lodLevels[0].chunkSize;
    const vramBudgetGB = parseFloat(document.getElementById('vram-budget').value);
    // Split the budget across channels (each channel owns its own atlas) and
    // respect the WebGPU staging-buffer hard limit of 4 GB per texture.
    const budgetBytes = Math.min(
        (vramBudgetGB * 1024 ** 3) / Math.max(1, numChannels),
        4 * 1024 ** 3,
    );
    let lo = 1, hi = 65536, best = 64;
    while (lo <= hi) {
        const mid = (lo + hi) >> 1;
        if (atlasSizeBytes(mid, tileX, tileY, tileZ) <= budgetBytes) {
            best = mid;
            lo = mid + 1;
        } else {
            hi = mid - 1;
        }
    }
    document.getElementById('max-chunks-display').textContent = best;
    return best;
}

/**
 * Create (or recreate) the tiled image visual
 * Called on initial load and when changing channels
 */
function createVisual() {
    // The renderer initializes asynchronously (WebGPU adapter + WASM). Guard
    // against the Load button being clicked before init() resolves, or after
    // it failed (e.g. no WebGPU adapter) — otherwise `viewer` is null here.
    if (!viewer || !wasmInitialized) {
        setStatus('Renderer not ready yet — waiting for WebGPU…', 'status-error');
        return;
    }

    viewer.clearScene();

    pendingLoads.clear();
    loadedChunkCount = 0;

    // `new Image(...)` consumes (moves) the LevelMetadata array, so build a
    // fresh one per channel. Each shape/size arg is a [z, y, x] array (numpy order).
    const buildLevels = () => lodLevels.map(level =>
        new wasmModule.LevelMetadata(
            level.volumeSize,
            level.chunkSize,
            level.voxelSize,
            level.scaleFactor,
            level.translation,
        )
    );

    // One Image visual per channel. Single channel → Normal blend (identical to
    // before). Multi-channel → each gets its own colormap + additive blending so
    // the channels composite order-independently.
    tiledImages = [];
    const additive = numChannels > 1;
    const maxChunks = computeMaxChunks();
    for (let c = 0; c < numChannels; c++) {
        const visual = new wasmModule.Image(viewer, buildLevels(), maxChunks);
        if (additive) {
            visual.setColormap(makeChannelColormap(CHANNEL_COLORS[c % CHANNEL_COLORS.length]));
            visual.setBlendMode(wasmModule.BlendMode.Additive);
        }
        visual.setContrast(0.0, 1.0);
        viewer.addImage(visual);
        tiledImages.push(visual);
    }

    startLoaderPoll();

    // Reapply slice plane if volumeCenter is set
    if (volumeCenter) {
        updateSlicePlane();
    }
}

// Mouse Controls (matching Python example)
canvas.addEventListener('mousedown', (e) => {
    if (e.button === 0) {
        isDragging = true;
        lastMouseX = e.clientX;
        lastMouseY = e.clientY;
    } else if (e.button === 1) {
        isMiddleDragging = true;
        lastMouseX = e.clientX;
        lastMouseY = e.clientY;
        e.preventDefault();
    } else if (e.button === 2) {
        isRightDragging = true;
        lastMouseX = e.clientX;
        lastMouseY = e.clientY;
        e.preventDefault();
    }
});

canvas.addEventListener('mousemove', (e) => {
    if (!viewer) return;

    const deltaX = e.clientX - lastMouseX;
    const deltaY = e.clientY - lastMouseY;

    if (isDragging) {
        // Check projection mode
        if (viewer.getCameraProjectionMode() === wasmModule.ProjectionMode.Orthographic) {
            // Pan in orthographic mode
            const panSpeed = volumeScale * 0.002;
            viewer.panCamera(-deltaX * panSpeed, deltaY * panSpeed);
        } else {
            viewer.orbitCamera(deltaX * 0.005, deltaY * 0.005);
        }
    } else if (isMiddleDragging && visualsReady()) {
        // Horizontal drag: level (center); vertical drag: window (width)
        windowCenter += deltaX * 0.002;
        windowWidth  += deltaY * 0.002;
        windowWidth   = Math.max(0.001, windowWidth);
        applyContrastAll(windowCenter - windowWidth / 2, windowCenter + windowWidth / 2);
    } else if (isRightDragging) {
        if (e.shiftKey) {
            sliceAngleX += deltaY * 0.01;
            updateSlicePlane();
        } else {
            if (Math.abs(deltaX) > Math.abs(deltaY)) {
                const offsetScale = volumeScale * 0.005;
                sliceOffset += deltaX * offsetScale;
                updateSlicePlane();
            } else {
                sliceAngleY += deltaY * 0.01;
                updateSlicePlane();
            }
        }
    }

    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
});

canvas.addEventListener('mouseup', () => {
    isDragging = false;
    isMiddleDragging = false;
    isRightDragging = false;
});

canvas.addEventListener('contextmenu', (e) => {
    e.preventDefault();
});

canvas.addEventListener('auxclick', (e) => {
    if (e.button === 1) e.preventDefault(); // suppress middle-click default (autoscroll)
});

canvas.addEventListener('wheel', (e) => {
    if (!viewer) return;

    // Use zoomCamera which handles both perspective and orthographic modes
    viewer.zoomCamera(e.deltaY);

    if (viewer.getCameraProjectionMode() === wasmModule.ProjectionMode.Perspective) {
        cameraDistance = viewer.getCameraDistance();
    }

    e.preventDefault();
});

// Keyboard Controls
document.addEventListener('keydown', (e) => {
    if (e.key === 'd' || e.key === 'D') {
        debugMode = !debugMode;
        if (visualsReady()) {
            applyDebugModeAll(debugMode);
        }
        document.getElementById('debug-mode').checked = debugMode;
    }
});

// Dataset Loading
async function loadDataset(datasetKey) {
    const dataset = DATASETS[datasetKey];
    if (!dataset) return;

    setStatus(`Loading ${dataset.description}...`, 'status-loading');
    console.log(`Loading: ${dataset.description}`);

    try {
        pendingLoads.clear();
        loadedChunkCount = 0;
        zarrArrays = [];
        lodLevels = [];

        // Open Zarr store using the root location directly
        zarrStore = new zarr.FetchStore(dataset.url);
        const rootLoc = zarr.root(zarrStore);

        // Use version-specific open method
        const openMethod = dataset.zarrVersion === 3 ? zarr.open.v3 : zarr.open.v2;
        zarrRoot = await openMethod(rootLoc, { kind: 'group' });

        const multiscales = dataset.zarrVersion === 3
            ? zarrRoot.attrs.ome.multiscales[0]  // v3: nested under 'ome'
            : zarrRoot.attrs.multiscales[0];      // v2: directly in attrs
        const datasets = multiscales.datasets;

        console.log(`\nFound ${datasets.length} LOD levels`);

        // Load all available LOD levels
        const maxLods = datasets.length;

        for (let lodIdx = 0; lodIdx < maxLods; lodIdx++) {
            const path = datasets[lodIdx].path;

            // Access array as child of root location
            const arrayLoc = rootLoc.resolve(path);
            const zarrArray = await openMethod(arrayLoc, { kind: 'array' });

            if (!zarrArray) {
                console.error(`Failed to open array at path: ${path}`);
                continue;
            }

            zarrArrays.push(zarrArray);

            const shape = zarrArray.shape;
            const ndim = shape.length;

            const zIdx = ndim - 3;
            const yIdx = ndim - 2;
            const xIdx = ndim - 1;

            const volumeSize = [shape[zIdx], shape[yIdx], shape[xIdx]];
            if (lodIdx === 0) {
                // Align the (uniform) logical tile to the LOD-0 storage chunk,
                // capped, so one tile fetch maps to ~one stored chunk.
                const ch = zarrArray.chunks;
                logicalTile = [
                    Math.min(ch[zIdx], MAX_LOGICAL_TILE),
                    Math.min(ch[yIdx], MAX_LOGICAL_TILE),
                    Math.min(ch[xIdx], MAX_LOGICAL_TILE),
                ];
            }
            const chunkSize = logicalTile;

            const transforms = datasets[lodIdx].coordinateTransformations || [];
            let voxelSize = [1.0, 1.0, 1.0];
            let translation = [0.0, 0.0, 0.0];

            // Debug: log transforms for all LODs

            for (const transform of transforms) {
                if (transform.type === 'scale') {
                    const scale = transform.scale;
                    voxelSize = [
                        scale[scale.length - 3] || 1.0,
                        scale[scale.length - 2] || 1.0,
                        scale[scale.length - 1] || 1.0
                    ];
                } else if (transform.type === 'translation') {
                    const trans = transform.translation;
                    translation = [
                        trans[trans.length - 3] || 0.0,
                        trans[trans.length - 2] || 0.0,
                        trans[trans.length - 1] || 0.0
                    ];
                    if (lodIdx === 0) {
                    }
                }
            }

            let scaleFactor;
            if (lodIdx === 0) {
                scaleFactor = 1.0;
            } else {
                const scaleZ = voxelSize[0] / lodLevels[0].voxelSize[0];
                const scaleY = voxelSize[1] / lodLevels[0].voxelSize[1];
                const scaleX = voxelSize[2] / lodLevels[0].voxelSize[2];
                scaleFactor = Math.max(scaleZ, scaleY, scaleX);
            }

            lodLevels.push({
                volumeSize,
                chunkSize,
                voxelSize,
                scaleFactor,
                translation  // Store translation for coordinate calculations
            });

            if (lodIdx === 0 && (translation[0] !== 0 || translation[1] !== 0 || translation[2] !== 0)) {
            }
        }

        const firstArray = zarrArrays[0];
        const hasChannels = firstArray.shape.length >= 4;
        // Assign the module-level global (drives the per-channel visual loop).
        numChannels = hasChannels ?
            (firstArray.shape.length === 5 ? firstArray.shape[1] : firstArray.shape[0]) : 1;

        // All channels are now shown together (additive). The dropdown is
        // informational only — no per-channel switching.
        const channelSelect = document.getElementById('channel-select');
        channelSelect.innerHTML = '';
        const option = document.createElement('option');
        option.textContent = numChannels > 1 ? `${numChannels} channels (additive)` : 'Channel 0';
        channelSelect.appendChild(option);
        channelSelect.disabled = true;

        // Apply translation offset if present (e.g., OME-Zarr 0.5 datasets)
        const lod0 = lodLevels[0];

        volumeCenter = [
            (lod0.volumeSize[2] * lod0.voxelSize[2]) / 2 + lod0.translation[2],  // x
            (lod0.volumeSize[1] * lod0.voxelSize[1]) / 2 + lod0.translation[1],  // y
            (lod0.volumeSize[0] * lod0.voxelSize[0]) / 2 + lod0.translation[0]   // z
        ];

        const volumeSizeWorld = [
            lod0.volumeSize[0] * lod0.voxelSize[0],
            lod0.volumeSize[1] * lod0.voxelSize[1],
            lod0.volumeSize[2] * lod0.voxelSize[2]
        ];

        const volumeBoundsMin = [
            lod0.translation[2],
            lod0.translation[1],
            lod0.translation[0]
        ];
        const volumeBoundsMax = [
            lod0.translation[2] + volumeSizeWorld[2],
            lod0.translation[1] + volumeSizeWorld[1],
            lod0.translation[0] + volumeSizeWorld[0]
        ];

        createVisual();
        const maxDimension = Math.max(...volumeSizeWorld);
        volumeScale = maxDimension;  // Store for adaptive zoom

        const nearPlane = maxDimension * 0.001;  // 0.1% of max dimension
        const farPlane = maxDimension * 100;     // 100x max dimension
        viewer.setCameraClipPlanes(nearPlane, farPlane);

        cameraDistance = dataset.cameraDistance || 200.0;

        viewer.setCameraPosition(
            volumeCenter[0],
            volumeCenter[1],
            volumeCenter[2] + cameraDistance
        );

        viewer.setCameraTarget(
            volumeCenter[0],
            volumeCenter[1],
            volumeCenter[2]
        );

        sliceAngleX = 0.0;
        sliceAngleY = 0.0;
        sliceOffset = 0.0;
        windowCenter = 0.5;
        windowWidth = 1.0;
        updateSlicePlane();

        applyContrastAll(0.0, 1.0);

        setStatus(`Ready: ${dataset.description}`, 'status-ready');

        // Enable controls
        document.getElementById('lod-bias').disabled = false;
        document.getElementById('contrast-min').disabled = false;
        document.getElementById('contrast-max').disabled = false;
        document.getElementById('debug-mode').disabled = false;
        document.getElementById('projection-toggle').disabled = false;

    } catch (error) {
        console.error('Error loading dataset:', error);
        setStatus(`Error: ${error.message}`, 'status-error');
    }
}

// UI Event Listeners
document.getElementById('load-btn').addEventListener('click', () => {
    const datasetKey = document.getElementById('dataset-select').value;
    if (datasetKey) {
        loadDataset(datasetKey);
    }
});

// Channel selection is no longer used — all channels render together (additive).

document.getElementById('vram-budget').addEventListener('change', () => {
    // Rebuild the visuals with the new per-channel tile budget.
    if (visualsReady() && lodLevels.length > 0) createVisual();
});

document.getElementById('lod-bias').addEventListener('input', (e) => {
    const value = parseFloat(e.target.value) / 10.0;
    document.getElementById('lod-bias-value').textContent = value.toFixed(1);
    if (visualsReady()) {
        applyLodBiasAll(value);
    }
});

document.getElementById('contrast-min').addEventListener('input', (e) => {
    document.getElementById('contrast-min-value').textContent = (parseInt(e.target.value) / 1000.0).toFixed(3);
    updateContrast();
});

document.getElementById('contrast-max').addEventListener('input', (e) => {
    document.getElementById('contrast-max-value').textContent = (parseInt(e.target.value) / 1000.0).toFixed(3);
    updateContrast();
});

document.getElementById('debug-mode').addEventListener('change', (e) => {
    debugMode = e.target.checked;
    if (visualsReady()) {
        applyDebugModeAll(debugMode);
    }
});

document.getElementById('projection-toggle').addEventListener('click', () => {
    if (!viewer) return;

    const currentMode = viewer.getCameraProjectionMode();
    const button = document.getElementById('projection-toggle');

    if (currentMode === wasmModule.ProjectionMode.Perspective) {
        // Switch to orthographic
        viewer.setCameraProjectionMode(wasmModule.ProjectionMode.Orthographic);
        // Set ortho height based on volume size
        viewer.setCameraOrthoHeight(volumeScale * 0.5);
        // Align camera to slice plane
        updateSlicePlane();
        button.textContent = 'Switch to Perspective';
    } else {
        viewer.setCameraProjectionMode(wasmModule.ProjectionMode.Perspective);
        button.textContent = 'Switch to Orthographic';
    }
});


function updateContrast() {
    if (!visualsReady()) return;
    const min = parseFloat(document.getElementById('contrast-min').value) / 1000.0;
    const max = parseFloat(document.getElementById('contrast-max').value) / 1000.0;
    applyContrastAll(min, max);
}

// Stats update interval
setInterval(() => {
    if (visualsReady()) {
        const stats = totalStats();
        if (stats) {
            loadedChunksEl.textContent = stats[0];
            visibleChunksEl.textContent = stats[1];

            // Debug: log when cache is near full
            if (stats[0] >= 250) {
            }
        }
    }
}, 500);

// Rendering Loop
let frameCount = 0;
function renderLoop() {
    if (viewer && wasmInitialized) {
        try {
            viewer.renderFrame();
            frameCount++;
        } catch (error) {
            // A render error (e.g. a Rust panic) leaves the wasm module
            // unusable, so stop the loop instead of rescheduling forever and
            // flooding the console with follow-on "recursive use" errors.
            console.error('Render error — stopping render loop:', error);
            setStatus(`Render error: ${error.message || error}`, 'status-error');
            return;
        }
    }
    animationFrameId = requestAnimationFrame(renderLoop);
}

// Initialization
async function init() {

    resizeCanvas();
    window.addEventListener('resize', resizeCanvas);

    try {
        if (!navigator.gpu) {
            throw new Error('WebGPU not supported in this browser');
        }

        setStatus('Requesting WebGPU adapter...', 'status-loading');

        // Request WebGPU adapter and device
        const adapter = await navigator.gpu.requestAdapter();
        if (!adapter) {
            throw new Error('Failed to get WebGPU adapter');
        }

        device = await adapter.requestDevice();
        queue = device.queue;

        // Configure canvas context
        context = canvas.getContext('webgpu');
        format = navigator.gpu.getPreferredCanvasFormat();

        context.configure({
            device: device,
            format: format,
            alphaMode: 'opaque'
        });

        console.log('✓ WebGPU initialized');

        setStatus('Loading WASM module...', 'status-loading');

        // Import and initialize WASM module
        wasmModule = await import('../pkg/bovista.js');
        await wasmModule.default();

        console.log('✓ WASM module loaded');

        viewer = await wasmModule.Viewer.new('canvas');

        // Don't set clip planes here - they will be set adaptively when dataset loads
        // (Hard-coded values like 0.1-10000 don't work for very small volumes like beechnut)

        console.log('✓ Viewer created');

        wasmInitialized = true;

        setStatus('Ready - Select a dataset', 'status-ready');

        renderLoop();

    } catch (error) {
        console.error('Initialization error:', error);
        setStatus(`Error: ${error.message}`, 'status-error');
        throw error;
    }
}

init().catch(err => {
    console.error('Init error:', err);
    setStatus(`Error: ${err.message}`, 'status-error');
});
