import * as zarr from 'https://cdn.jsdelivr.net/npm/zarrita@latest/+esm';

// Logical atlas tile size [z, y, x], uniform across all LOD levels — bovista's
// *virtual* tiling, decoupled from the dataset's per-LOD storage chunking
// (bovista's VT assumes one tile size for all LODs). Set per-dataset in
// loadDataset() from the LOD-0 storage chunk (capped), so each tile maps to
// ~one stored chunk: a fixed cube like 64³ would span dozens of thin storage
// chunks (e.g. chunk_z=1) and fire a fetch storm.
let logicalTile = [64, 64, 64];
const MAX_LOGICAL_TILE = 128;

// Configuration
const DATASETS = {
    pawpawsaurus: {
        url: 'https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/pawpawsaurus.ome.zarr',
        description: 'Pawpawsaurus Dinosaur Fossil - CT scan',
        cameraDistance: 600.0,
        zarrVersion: 3,
    },
    beechnut: {
        url: 'https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/beechnut.ome.zarr',
        description: 'Beechnut - High-resolution CT scan',
        cameraDistance: 0.07,
        zarrVersion: 3,
    },
    marmoset_neurons: {
        url: 'https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/marmoset_neurons.ome.zarr',
        description: 'Marmoset Neurons - 3D fluorescence',
        cameraDistance: 400.0,
        zarrVersion: 3,
    },
    exaspim_844958: {
        url: 'https://aind-open-data.s3.amazonaws.com/exaSPIM_844958_2026-04-13_12-25-27/SPIM.ome.zarr/tile_000000_ch_561.zarr',
        description: 'AIND exaSPIM 844958 - tile 000000 ch 561',
        cameraDistance: 10000.0,
        zarrVersion: 2,
    },
    idr0062: {
        // 2-channel CZYX volume (~236×275×271) — exercises multi-channel additive
        // volume rendering. NOTE: chunk_z=1, so it fires many small fetches.
        url: 'https://uk1s3.embassy.ebi.ac.uk/idr/zarr/v0.4/idr0062A/6001240.zarr',
        description: 'IDR0062 HeLa — 2-channel (multi-channel volume test)',
        cameraDistance: 250.0,
        zarrVersion: 2,
    },
    zebrahub_zmns001: {
        // 5D TCZYX, 2 channels (h2afva, mezzo), 5 LODs, ~780µm extent. The data
        // sits low in the u16 range (window ~0–1500), so use Auto contrast after
        // loading. t is fixed at 0 (the examples don't expose the time axis).
        url: 'https://public.czbiohub.org/royerlab/zebrahub/imaging/multi-view/ZMNS001.ome.zarr',
        description: 'Zebrahub ZMNS001 — zebrafish embryo, 2-channel light-sheet',
        cameraDistance: 1000.0,
        zarrVersion: 2,
    },
};

// Cap on in-flight logical-tile loads (shared across channels). Each tile may
// still fan out to a few storage-chunk fetches, so keep this modest to avoid
// ERR_INSUFFICIENT_RESOURCES (too many concurrent requests).
// TODO(loader): this caps logical tiles, not actual network requests. A tile's
// `zarr.get` fans out to N storage-chunk fetches we don't throttle. Wrap the
// zarrita FetchStore (or global fetch) in a semaphore (~16 concurrent) for a
// real request cap — belt-and-suspenders now that tiles are chunk-aligned.
const MAX_PENDING_CHUNKS = 32;

// Global state
let wasmModule = null;
let wasmInitialized = false;
let viewer = null;
// One volume visual per channel. Length 1 for single-channel (Normal blend,
// identical to before); multi-channel gets per-channel colormaps + additive.
let volumeVisuals = [];
let zarrArrays = [];
let numChannels = 1;
let lodLevels = [];
let volumeCenter = [0, 0, 0];
let volumeScale = 1.0;
let animationFrameId = null;

// Per-channel base colors for multi-channel additive display.
const CHANNEL_COLORS = [
    [255, 0, 0], [0, 255, 0], [0, 128, 255],
    [255, 0, 255], [0, 255, 255], [255, 255, 0], [255, 255, 255],
];

// 256-entry RGBA LUT ramping black→`rgb`, with alpha proportional to value.
// In DVR the colormap alpha drives per-sample extinction, so alpha must track
// value — otherwise (flat alpha=255) dim data composites just as opaquely as
// bright data, and dim front samples (background, or the coarse-LOD fallback
// near the camera while fine tiles stream) paint dark and occlude brighter data
// behind. Proportional alpha matches the single-channel default colormap
// ([v,v,v,v]); the additive pipeline still sums per-channel intensity × hue.
function makeChannelColormap(rgb) {
    const lut = new Uint8Array(1024);
    for (let i = 0; i < 256; i++) {
        const f = i / 255;
        lut[i * 4 + 0] = Math.round(rgb[0] * f);
        lut[i * 4 + 1] = Math.round(rgb[1] * f);
        lut[i * 4 + 2] = Math.round(rgb[2] * f);
        lut[i * 4 + 3] = i;  // alpha ∝ value (= round(255·f))
    }
    return lut;
}

// Helpers applying a per-visual op across all channel visuals.
const visualsReady = () => volumeVisuals.length > 0;
const eachVolume = (fn) => { for (const v of volumeVisuals) fn(v); };
const totalStats = () => {
    let loaded = 0, visible = 0;
    for (const v of volumeVisuals) { const s = v.getStats(); loaded += s[0]; visible += s[1]; }
    return [loaded, visible];
};

// Chunk loading state
const pendingLoads = new Set();

// Window/level state — drives middle-drag and syncs with min/max sliders.
let windowCenter = 0.5;
let windowWidth = 1.0;

// Contrast ceiling: sliders operate within [0, contrastCeiling] for fine-grained control.
let contrastCeiling = 1.0;

// Mouse interaction state
let isDragging = false;
let isMiddleDragging = false;
let lastMouseX = 0;
let lastMouseY = 0;
let debugMode = false;
let atlasDebugMode = false;
let stepDebugMode = false;

// Active rendering mode (one of the six volume visuals). Switching rebuilds
// the visual but keeps the rest of the viewer state (camera, contrast,
// step size, lod bias). bovista exposes one wasm_bindgen class per mode;
// VOLUME_MODES maps the dropdown value → (constructor, viewer.add* method,
// flags for which mode-specific sliders/toggles apply).
const VOLUME_MODES = {
    direct:   { ctor: 'DirectVolume',     add: 'addDirectVolume',     density: true,  debug: true  },
    mip:      { ctor: 'MipVolume',        add: 'addMipVolume',        attenuation: true            },
    minip:    { ctor: 'MinipVolume',      add: 'addMinipVolume'                                    },
    average:  { ctor: 'AverageVolume',    add: 'addAverageVolume'                                  },
    iso:      { ctor: 'IsosurfaceVolume', add: 'addIsosurfaceVolume', iso: true                    },
};
let currentMode = 'direct';

// UI elements
const canvas = document.getElementById('canvas');
const statusEl = document.getElementById('status-text');
const loadedChunksEl = document.getElementById('loaded-chunks');
const visibleChunksEl = document.getElementById('visible-chunks');
const pendingLoadsEl = document.getElementById('pending-loads');

function setStatus(message, className = 'status-loading') {
    statusEl.textContent = message;
    statusEl.className = className;
}

function resizeCanvas() {
    const rect = canvas.getBoundingClientRect();
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.floor(rect.width * dpr);
    canvas.height = Math.floor(rect.height * dpr);
    if (viewer) viewer.resize(canvas.width, canvas.height);
}

// Pull-based loader. A polling loop reads each channel visual's `wantedKeys()`
// (flat Uint32Array of [lod, t, z, y, x, priority, ...] sorted by
// priority) and dispatches async fetches. Bovista drops keys from
// `wanted` as soon as they're no longer needed, so cancellation is
// implicit — keys that disappear just don't get re-submitted.
let loaderPollHandle = null;
function startLoaderPoll() {
    if (loaderPollHandle !== null) return;
    loaderPollHandle = setInterval(() => {
        if (volumeVisuals.length === 0) return;
        // Poll every channel's visual; each fetches its own channel's tiles.
        for (let c = 0; c < volumeVisuals.length; c++) {
            const w = volumeVisuals[c].wantedKeys();
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

async function loadChunkAsync(c, lod, t, z, y, x, key) {
    try {
        const zarrArray = zarrArrays[lod];
        if (!zarrArray) throw new Error(`No array for LOD ${lod}`);

        const shape = zarrArray.shape;
        const ndim = shape.length;

        const zIdx = ndim - 3, yIdx = ndim - 2, xIdx = ndim - 1;
        const [chunkD, chunkH, chunkW] = logicalTile;

        const zStart = z * chunkD, yStart = y * chunkH, xStart = x * chunkW;
        const zEnd = Math.min(zStart + chunkD, shape[zIdx]);
        const yEnd = Math.min(yStart + chunkH, shape[yIdx]);
        const xEnd = Math.min(xStart + chunkW, shape[xIdx]);

        let selection;
        if (ndim === 5) {
            selection = [t, c, zarr.slice(zStart, zEnd), zarr.slice(yStart, yEnd), zarr.slice(xStart, xEnd)];
        } else if (ndim === 4) {
            selection = [c, zarr.slice(zStart, zEnd), zarr.slice(yStart, yEnd), zarr.slice(xStart, xEnd)];
        } else {
            selection = [zarr.slice(zStart, zEnd), zarr.slice(yStart, yEnd), zarr.slice(xStart, xEnd)];
        }

        let chunk;
        try {
            chunk = await zarr.get(zarrArray, selection);
        } catch (error) {
            if (error.message && (error.message.includes('404') || error.message.includes('Not Found'))) return;
            throw error;
        }

        const data = chunk.data;
        if (!data || data.length === 0) throw new Error('Empty chunk data');

        // Tile extents come from the RETURNED chunk shape (z, y, x — scalar
        // t/c dims already dropped), never the requested slice bounds, so they
        // always match the data length. setChunkData* wants (z_shape, y_shape,
        // x_shape) in numpy order.
        const sh = chunk.shape;
        const dz = sh[sh.length - 3];
        const dy = sh[sh.length - 2];
        const dx = sh[sh.length - 1];

        // Hand the native tile for this channel to bovista — it normalizes the
        // full dtype range to [0, 1] itself, so no client-side rescaling.
        const visual = volumeVisuals[c];
        if (visual) {
            if (data instanceof Uint16Array) {
                visual.setChunkDataU16(lod, t, z, y, x, data, dz, dy, dx);
            } else {
                const src = data instanceof Uint8Array ? data : new Uint8Array(data.buffer || data);
                visual.setChunkDataU8(lod, t, z, y, x, src, dz, dy, dx);
            }
        }
    } catch (error) {
        console.error(`[ChunkLoader] Error loading ${key}:`, error);
    } finally {
        pendingLoads.delete(key);
        pendingLoadsEl.textContent = pendingLoads.size;
    }
}

// Compute the actual atlas byte size for n tiles at the given tile dimensions.
// The atlas allocator rounds up to ceil(cbrt(n))^2 * ceil(n / ceil(cbrt(n))^2),
// so the atlas is always larger than n * tileSizeBytes.
function atlasSizeBytes(n, tileW, tileH, tileD) {
    const cols   = Math.ceil(Math.cbrt(n));
    const rows   = Math.ceil(Math.cbrt(n));
    const layers = Math.ceil(n / (cols * rows));
    return cols * tileW * rows * tileH * layers * tileD * 2; // R16Float = 2 bytes
}

function computeMaxChunks() {
    if (lodLevels.length === 0) return 256;
    // The atlas uses LOD-0 tile dimensions for every slot.
    const lod0 = lodLevels[0];
    const [tileZ, tileY, tileX] = lod0.chunkSize;
    const vramBudgetGB = parseFloat(document.getElementById('vram-budget').value);
    // Also respect the WebGPU staging-buffer hard limit of 4 GB.
    const budgetBytes = Math.min(
        vramBudgetGB * 1024 ** 3,
        4 * 1024 ** 3
    );

    // Binary search for the largest n tiles whose rounded atlas fits within budget.
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

    // `new <Volume>(...)` consumes (moves) the LevelMetadata array, so build a
    // fresh one per channel. Each shape/size arg is a [z, y, x] array (numpy order).
    const buildLevels = () => lodLevels.map(level => new wasmModule.LevelMetadata(
        level.volumeSize,
        level.chunkSize,
        level.voxelSize,
        level.scaleFactor,
        level.translation,
    ));

    const maxChunks = computeMaxChunks();
    // Split the tile budget across channels so total VRAM stays within budget.
    const perChannel = Math.max(1, Math.floor(maxChunks / numChannels));
    const cfg = VOLUME_MODES[currentMode];
    const additive = numChannels > 1;
    volumeVisuals = [];
    for (let c = 0; c < numChannels; c++) {
        const v = new wasmModule[cfg.ctor](viewer, buildLevels(), perChannel);
        if (additive) {
            v.setColormap(makeChannelColormap(CHANNEL_COLORS[c % CHANNEL_COLORS.length]));
            v.setBlendMode(wasmModule.BlendMode.Additive);
        }
        viewer[cfg.add](v);
        volumeVisuals.push(v);
    }
    startLoaderPoll();

    // Apply all controls (common ones plus the mode-specific subset).
    applyModeCaps();
    applyStepSize();
    applyContrast();
    if (cfg.density) applyDensityScale();
    if (cfg.attenuation) applyAttenuation();
    if (cfg.iso) applyIsoThreshold();
}

// Toggle UI sections so only the controls that apply to the active mode
// are visible/enabled, and clear any debug-toggle state when switching to
// a mode that doesn't honour it.
function applyModeCaps() {
    const cfg = VOLUME_MODES[currentMode];
    document.getElementById('density-scale-group').style.display = cfg.density ? '' : 'none';
    document.getElementById('attenuation-group').style.display = cfg.attenuation ? '' : 'none';
    document.getElementById('iso-threshold-group').style.display = cfg.iso ? '' : 'none';
    for (const id of ['debug-mode', 'atlas-debug-mode', 'step-debug-mode']) {
        document.getElementById(id).disabled = !cfg.debug;
        if (!cfg.debug) document.getElementById(id).checked = false;
    }
    if (!cfg.debug) { debugMode = false; atlasDebugMode = false; stepDebugMode = false; }
}

function applyAttenuation() {
    if (!visualsReady()) return;
    // Slider 0..100 → 0.0..10.0 attenuation strength.
    const v = parseInt(document.getElementById('attenuation').value) / 10.0;
    document.getElementById('attenuation-value').textContent = v.toFixed(2);
    eachVolume(vis => { if (vis.setAttenuation) vis.setAttenuation(v); });
}

function applyIsoThreshold() {
    if (!visualsReady()) return;
    // Slider 1..999 → 0.001..0.999 in contrast-normalised raw units.
    const v = parseInt(document.getElementById('iso-threshold').value) / 1000.0;
    document.getElementById('iso-threshold-value').textContent = v.toFixed(3);
    eachVolume(vis => { if (vis.setIsoThreshold) vis.setIsoThreshold(v); });
}

// Contrast helpers: sliders are integers 0-1000 mapped within [0, contrastCeiling].
function sliderToValue(i) { return (i / 1000.0) * contrastCeiling; }
function valueToSlider(v) { return Math.round(Math.min(1, Math.max(0, v / contrastCeiling)) * 1000); }

function syncContrastDisplay() {
    const minV = sliderToValue(parseInt(document.getElementById('contrast-min').value));
    const maxV = sliderToValue(parseInt(document.getElementById('contrast-max').value));
    document.getElementById('contrast-min-value').textContent = minV.toFixed(3);
    document.getElementById('contrast-max-value').textContent = maxV.toFixed(3);
}

// Controls
function applyContrast() {
    if (!visualsReady()) return;
    const min = sliderToValue(parseInt(document.getElementById('contrast-min').value));
    const max = sliderToValue(parseInt(document.getElementById('contrast-max').value));
    eachVolume(v => v.setContrast(min, max));
    windowCenter = (min + max) / 2;
    windowWidth  = max - min;
}

function setCeiling(newCeiling) {
    const oldMin = sliderToValue(parseInt(document.getElementById('contrast-min').value));
    const oldMax = sliderToValue(parseInt(document.getElementById('contrast-max').value));
    contrastCeiling = Math.max(0.001, newCeiling);
    document.getElementById('contrast-ceiling').value = contrastCeiling.toFixed(3);
    document.getElementById('contrast-min').value = valueToSlider(oldMin);
    document.getElementById('contrast-max').value = valueToSlider(oldMax);
    syncContrastDisplay();
    applyContrast();
}

// Auto-contrast: sample the center chunk of the coarsest LOD for percentile bounds.
async function autoContrast() {
    if (!zarrArrays.length) return;
    const coarsestArray = zarrArrays[zarrArrays.length - 1];
    const shape = coarsestArray.shape;
    const ndim = shape.length;
    const zIdx = ndim - 3, yIdx = ndim - 2, xIdx = ndim - 1;
    const chunks = coarsestArray.chunks;
    const chunkZ = Math.floor(shape[zIdx] / 2 / chunks[zIdx]) * chunks[zIdx];
    const chunkY = Math.floor(shape[yIdx] / 2 / chunks[yIdx]) * chunks[yIdx];
    const chunkX = Math.floor(shape[xIdx] / 2 / chunks[xIdx]) * chunks[xIdx];
    try {
        let selection;
        const sliceZ = zarr.slice(chunkZ, Math.min(chunkZ + chunks[zIdx], shape[zIdx]));
        const sliceY = zarr.slice(chunkY, Math.min(chunkY + chunks[yIdx], shape[yIdx]));
        const sliceX = zarr.slice(chunkX, Math.min(chunkX + chunks[xIdx], shape[xIdx]));
        // Sample channel 0 for the shared auto-contrast bounds.
        if (ndim === 5)      selection = [0, 0, sliceZ, sliceY, sliceX];
        else if (ndim === 4) selection = [0, sliceZ, sliceY, sliceX];
        else                 selection = [sliceZ, sliceY, sliceX];

        const chunk = await zarr.get(coarsestArray, selection);
        const data = chunk.data;
        const maxVal = data instanceof Uint16Array ? 65535 : 255;
        const hist = new Uint32Array(1024);
        for (let i = 0; i < data.length; i++)
            hist[Math.min(1023, Math.round((data[i] / maxVal) * 1023))]++;
        const total = data.length;
        let cum = 0, lo = 0, hi = 1023;
        for (let b = 0; b < 1024; b++) { cum += hist[b]; if (cum / total >= 0.01) { lo = b; break; } }
        cum = 0;
        for (let b = 1023; b >= 0; b--) { cum += hist[b]; if (cum / total >= 0.001) { hi = b; break; } }
        const loNorm = lo / 1023;
        const hiNorm = hi / 1023;
        const newCeiling = Math.min(1.0, hiNorm * 1.1);
        setCeiling(newCeiling);
        document.getElementById('contrast-min').value = valueToSlider(loNorm);
        document.getElementById('contrast-max').value = valueToSlider(hiNorm);
        syncContrastDisplay();
        applyContrast();
    } catch (e) {
        console.warn('Auto-contrast sampling failed:', e);
    }
}

function applyStepSize() {
    if (!visualsReady()) return;
    // Slider maps to relative step size in LOD-0 voxels.
    // 100 → 1.0 voxel/step (default). Range 10→0.1 (oversampling) to 800→8.0 (fast/coarse).
    const relative = parseFloat(document.getElementById('step-size').value) / 100.0;
    document.getElementById('step-size-value').textContent = relative.toFixed(2) + ' vox';
    eachVolume(v => v.setRelativeStepSize(relative));
}

let densityScaleExp = 0;  // log10 exponent; slider range -20..+20 → 0.01× to 100×

function applyDensityScale() {
    if (!visualsReady()) return;
    // Match kiln's normalization: extinction = cs.a * stepSize * maxDim * 0.5
    // where stepSize is in normalized [0,1] volume space and maxDim is in voxels.
    // Equivalent in world-space: density_scale = 0.5 * maxDim / volumeScale,
    // so extinction = cs.a * (0.5 * maxDim / volumeScale) * advance
    //               ≈ cs.a * 0.5 * step_in_voxels  (physically correct per-voxel opacity)
    const lod0 = lodLevels[0];
    const maxDim = Math.max(lod0.volumeSize[0], lod0.volumeSize[1], lod0.volumeSize[2]);
    const baseScale = 0.5 * maxDim / Math.max(volumeScale, 1e-9);
    const multiplier = Math.pow(10, densityScaleExp / 10);
    eachVolume(v => { if (v.setDensityScale) v.setDensityScale(baseScale * multiplier); });
    const displayMult = multiplier < 10 ? multiplier.toFixed(2) : multiplier.toFixed(1);
    document.getElementById('density-scale-value').textContent = displayMult + '×';
}

// Mouse controls
canvas.addEventListener('mousedown', e => {
    if (e.button === 0) { isDragging = true; }
    else if (e.button === 1) { isMiddleDragging = true; e.preventDefault(); }
    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
});

canvas.addEventListener('mousemove', e => {
    if (!viewer) return;
    const dx = e.clientX - lastMouseX;
    const dy = e.clientY - lastMouseY;

    if (isDragging) {
        viewer.orbitCamera(dx * 0.005, dy * 0.005);
    } else if (isMiddleDragging && visualsReady()) {
        windowCenter += dx * 0.002 * contrastCeiling;
        windowWidth  -= dy * 0.002 * contrastCeiling;
        windowWidth   = Math.max(0.001, windowWidth);
        const min = windowCenter - windowWidth / 2;
        const max = windowCenter + windowWidth / 2;
        eachVolume(v => v.setContrast(min, max));
        if (max > contrastCeiling) setCeiling(Math.min(1.0, max * 1.1));
        document.getElementById('contrast-min').value = valueToSlider(min);
        document.getElementById('contrast-max').value = valueToSlider(max);
        syncContrastDisplay();
    }

    lastMouseX = e.clientX;
    lastMouseY = e.clientY;
});

canvas.addEventListener('mouseup', () => { isDragging = false; isMiddleDragging = false; });
canvas.addEventListener('contextmenu', e => e.preventDefault());
canvas.addEventListener('auxclick', e => { if (e.button === 1) e.preventDefault(); });

canvas.addEventListener('wheel', e => {
    if (!viewer) return;
    viewer.zoomCamera(e.deltaY);
    e.preventDefault();
}, { passive: false });

// Keyboard
document.addEventListener('keydown', e => {
    if ((e.key === 'd' || e.key === 'D') && visualsReady()) {
        debugMode = !debugMode;
        atlasDebugMode = false; stepDebugMode = false;
        eachVolume(v => { if (v.setDebugMode) v.setDebugMode(debugMode); });
        document.getElementById('debug-mode').checked = debugMode;
        document.getElementById('atlas-debug-mode').checked = false;
        document.getElementById('step-debug-mode').checked = false;
    }
});

// Dataset loading
async function loadDataset(datasetKey) {
    const dataset = DATASETS[datasetKey];
    if (!dataset) return;

    setStatus(`Loading ${dataset.description}...`);

    try {
        pendingLoads.clear();
        zarrArrays = [];
        lodLevels = [];

        const zarrStore = new zarr.FetchStore(dataset.url);
        const rootLoc = zarr.root(zarrStore);
        const openMethod = dataset.zarrVersion === 3 ? zarr.open.v3 : zarr.open.v2;
        const zarrRoot = await openMethod(rootLoc, { kind: 'group' });

        const multiscales = dataset.zarrVersion === 3
            ? zarrRoot.attrs.ome.multiscales[0]
            : zarrRoot.attrs.multiscales[0];

        for (let i = 0; i < multiscales.datasets.length; i++) {
            const path = multiscales.datasets[i].path;
            const arrayLoc = rootLoc.resolve(path);
            const zarrArray = await openMethod(arrayLoc, { kind: 'array' });
            if (!zarrArray) continue;

            zarrArrays.push(zarrArray);

            const shape = zarrArray.shape;
            const ndim = shape.length;
            const zIdx = ndim - 3, yIdx = ndim - 2, xIdx = ndim - 1;

            const volumeSize = [shape[zIdx], shape[yIdx], shape[xIdx]];
            if (i === 0) {
                // Align the (uniform) logical tile to the LOD-0 storage chunk,
                // capped, so one tile fetch maps to ~one stored chunk.
                const ch = zarrArray.chunks;
                logicalTile = [
                    Math.min(ch[zIdx], MAX_LOGICAL_TILE),
                    Math.min(ch[yIdx], MAX_LOGICAL_TILE),
                    Math.min(ch[xIdx], MAX_LOGICAL_TILE),
                ];
            }
            const chunkSize  = logicalTile;

            const transforms = multiscales.datasets[i].coordinateTransformations || [];
            let voxelSize   = [1.0, 1.0, 1.0];
            let translation = [0.0, 0.0, 0.0];
            for (const t of transforms) {
                if (t.type === 'scale') {
                    const s = t.scale;
                    voxelSize = [s[s.length-3]||1, s[s.length-2]||1, s[s.length-1]||1];
                } else if (t.type === 'translation') {
                    const tr = t.translation;
                    translation = [tr[tr.length-3]||0, tr[tr.length-2]||0, tr[tr.length-1]||0];
                }
            }

            let scaleFactor = 1.0;
            if (i > 0) {
                scaleFactor = Math.max(
                    voxelSize[0] / lodLevels[0].voxelSize[0],
                    voxelSize[1] / lodLevels[0].voxelSize[1],
                    voxelSize[2] / lodLevels[0].voxelSize[2],
                );
            }

            lodLevels.push({ volumeSize, chunkSize, voxelSize, scaleFactor, translation });
        }

        // Channel picker
        const firstArray = zarrArrays[0];
        const hasChannels = firstArray.shape.length >= 4;
        // Assign the module-level global (drives the per-channel visual loop).
        numChannels = hasChannels
            ? (firstArray.shape.length === 5 ? firstArray.shape[1] : firstArray.shape[0])
            : 1;

        // All channels render together (additive); the dropdown is informational.
        const channelSelect = document.getElementById('channel-select');
        channelSelect.innerHTML = '';
        const opt = document.createElement('option');
        opt.textContent = numChannels > 1 ? `${numChannels} channels (additive)` : 'Channel 0';
        channelSelect.appendChild(opt);
        channelSelect.disabled = true;

        // Camera setup
        const lod0 = lodLevels[0];
        volumeCenter = [
            lod0.translation[2] + (lod0.volumeSize[2] * lod0.voxelSize[2]) / 2,
            lod0.translation[1] + (lod0.volumeSize[1] * lod0.voxelSize[1]) / 2,
            lod0.translation[0] + (lod0.volumeSize[0] * lod0.voxelSize[0]) / 2,
        ];
        volumeScale = Math.max(
            lod0.volumeSize[0] * lod0.voxelSize[0],
            lod0.volumeSize[1] * lod0.voxelSize[1],
            lod0.volumeSize[2] * lod0.voxelSize[2],
        );

        viewer.setCameraClipPlanes(volumeScale * 0.001, volumeScale * 100);

        const dist = dataset.cameraDistance;
        viewer.setCameraPosition(volumeCenter[0], volumeCenter[1], volumeCenter[2] + dist);
        viewer.setCameraTarget(volumeCenter[0], volumeCenter[1], volumeCenter[2]);

        // Reset controls before createVisual
        densityScaleExp = 0;
        document.getElementById('density-scale').value = 0;
        windowCenter = 0.5; windowWidth = 1.0; contrastCeiling = 1.0;
        document.getElementById('contrast-ceiling').value = '1.000';
        document.getElementById('contrast-min').value = 0;
        document.getElementById('contrast-max').value = 1000;
        document.getElementById('contrast-min-value').textContent = '0.000';
        document.getElementById('contrast-max-value').textContent = '1.000';
        document.getElementById('step-size').value = 100;

        createVisual();

        // Enable controls
        ['lod-bias','render-mode','contrast-min','contrast-max','contrast-ceiling','auto-contrast-btn','step-size','density-scale','debug-mode','atlas-debug-mode','step-debug-mode'].forEach(id => {
            const el = document.getElementById(id);
            if (el) { el.disabled = false; el.removeAttribute('disabled'); }
        });

        setStatus(`Ready: ${dataset.description}`, 'status-ready');

    } catch (error) {
        console.error('Error loading dataset:', error);
        setStatus(`Error: ${error.message}`, 'status-error');
    }
}

// UI event listeners
document.getElementById('load-btn').addEventListener('click', () => {
    const key = document.getElementById('dataset-select').value;
    if (key) loadDataset(key);
});

// Channel selection is no longer used — all channels render together (additive).

document.getElementById('lod-bias').addEventListener('input', e => {
    const v = parseFloat(e.target.value) / 10.0;
    document.getElementById('lod-bias-value').textContent = v.toFixed(1);
    eachVolume(vis => vis.setLodBias(v));
});

document.getElementById('contrast-min').addEventListener('input', e => {
    const maxI = parseInt(document.getElementById('contrast-max').value);
    if (parseInt(e.target.value) > maxI) e.target.value = maxI;
    syncContrastDisplay();
    applyContrast();
});
document.getElementById('contrast-max').addEventListener('input', e => {
    const minI = parseInt(document.getElementById('contrast-min').value);
    if (parseInt(e.target.value) < minI) e.target.value = minI;
    syncContrastDisplay();
    applyContrast();
});
document.getElementById('contrast-ceiling').addEventListener('change', e => {
    setCeiling(parseFloat(e.target.value));
});
document.getElementById('auto-contrast-btn').addEventListener('click', autoContrast);

document.getElementById('step-size').addEventListener('input', applyStepSize);
document.getElementById('density-scale').addEventListener('input', e => {
    densityScaleExp = parseInt(e.target.value);
    applyDensityScale();
});
document.getElementById('vram-budget').addEventListener('change', () => {
    // Recreate the visuals with the new tile budget when a dataset is loaded.
    if (visualsReady() && lodLevels.length > 0) createVisual();
});

document.getElementById('render-mode').addEventListener('change', e => {
    currentMode = e.target.value;
    if (visualsReady() && lodLevels.length > 0) createVisual();
});

document.getElementById('attenuation').addEventListener('input', applyAttenuation);
document.getElementById('iso-threshold').addEventListener('input', applyIsoThreshold);

document.getElementById('debug-mode').addEventListener('change', e => {
    debugMode = e.target.checked;
    if (debugMode) { atlasDebugMode = false; document.getElementById('atlas-debug-mode').checked = false; }
    eachVolume(v => { if (v.setDebugMode) v.setDebugMode(debugMode); });
});

document.getElementById('atlas-debug-mode').addEventListener('change', e => {
    atlasDebugMode = e.target.checked;
    if (atlasDebugMode) {
        debugMode = false; stepDebugMode = false;
        document.getElementById('debug-mode').checked = false;
        document.getElementById('step-debug-mode').checked = false;
    }
    eachVolume(v => { if (v.setAtlasDebugMode) v.setAtlasDebugMode(atlasDebugMode); });
});

document.getElementById('step-debug-mode').addEventListener('change', e => {
    stepDebugMode = e.target.checked;
    if (stepDebugMode) {
        debugMode = false; atlasDebugMode = false;
        document.getElementById('debug-mode').checked = false;
        document.getElementById('atlas-debug-mode').checked = false;
    }
    eachVolume(v => { if (v.setStepDebugMode) v.setStepDebugMode(stepDebugMode); });
});

// Stats update
setInterval(() => {
    if (visualsReady()) {
        const stats = totalStats();
        loadedChunksEl.textContent = stats[0];
        visibleChunksEl.textContent = stats[1];
    }
}, 500);

// Render loop
function renderLoop() {
    if (viewer && wasmInitialized) {
        try {
            viewer.renderFrame();
        } catch (e) {
            // A render error (e.g. a Rust panic) leaves the wasm module
            // unusable, so stop the loop instead of rescheduling forever and
            // flooding the console with follow-on "recursive use" errors.
            console.error('Render error — stopping render loop:', e);
            setStatus(`Render error: ${e.message || e}`, 'status-error');
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
        if (!navigator.gpu) throw new Error('WebGPU not supported in this browser');

        setStatus('Requesting WebGPU adapter...');
        const adapter = await navigator.gpu.requestAdapter();
        if (!adapter) throw new Error('Failed to get WebGPU adapter');

        const device = await adapter.requestDevice();
        const context = canvas.getContext('webgpu');
        const format = navigator.gpu.getPreferredCanvasFormat();

        context.configure({ device, format, alphaMode: 'opaque' });

        setStatus('Loading WASM module...');
        wasmModule = await import('../../pkg/bovista.js');
        await wasmModule.default();

        viewer = await wasmModule.Viewer.new('canvas');
        wasmInitialized = true;

        setStatus('Ready — select a dataset', 'status-ready');
        renderLoop();

    } catch (error) {
        console.error('Init error:', error);
        setStatus(`Error: ${error.message}`, 'status-error');
    }
}

init();
