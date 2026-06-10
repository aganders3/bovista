import * as zarr from 'https://cdn.jsdelivr.net/npm/zarrita@latest/+esm';

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
};

const MAX_PENDING_CHUNKS = 64;

// Global state
let wasmModule = null;
let wasmInitialized = false;
let viewer = null;
let volumeVisual = null;
let zarrArrays = [];
let currentChannel = 0;
let lodLevels = [];
let volumeCenter = [0, 0, 0];
let volumeScale = 1.0;
let animationFrameId = null;

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
    direct:   { ctor: 'JsDirectVolume',     add: 'addDirectVolume',     density: true,  debug: true  },
    additive: { ctor: 'JsAdditiveVolume',   add: 'addAdditiveVolume',   density: true                },
    mip:      { ctor: 'JsMipVolume',        add: 'addMipVolume',        attenuation: true            },
    minip:    { ctor: 'JsMinipVolume',      add: 'addMinipVolume'                                    },
    average:  { ctor: 'JsAverageVolume',    add: 'addAverageVolume'                                  },
    iso:      { ctor: 'JsIsosurfaceVolume', add: 'addIsosurfaceVolume', iso: true                    },
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

// Pull-based loader. A polling loop reads `volumeVisual.wantedKeys()`
// (flat Uint32Array of [lod, t, z, y, x, priority, ...] sorted by
// priority) and dispatches async fetches. Bovista drops keys from
// `wanted` as soon as they're no longer needed, so cancellation is
// implicit — keys that disappear just don't get re-submitted.
let loaderPollHandle = null;
function startLoaderPoll() {
    if (loaderPollHandle !== null) return;
    loaderPollHandle = setInterval(() => {
        if (!volumeVisual) return;
        const w = volumeVisual.wantedKeys();
        for (let i = 0; i < w.length; i += 6) {
            const lod = w[i], t = w[i+1], z = w[i+2], y = w[i+3], x = w[i+4];
            const key = `${lod}:${t}:${z}:${y}:${x}`;
            if (pendingLoads.has(key)) continue;
            if (pendingLoads.size >= MAX_PENDING_CHUNKS) break;
            pendingLoads.add(key);
            pendingLoadsEl.textContent = pendingLoads.size;
            loadChunkAsync(lod, t, z, y, x, key);
        }
    }, 25);
}

async function loadChunkAsync(lod, t, z, y, x, key) {
    try {
        const zarrArray = zarrArrays[lod];
        if (!zarrArray) throw new Error(`No array for LOD ${lod}`);

        const chunks = zarrArray.chunks;
        const shape = zarrArray.shape;
        const ndim = shape.length;

        const zIdx = ndim - 3, yIdx = ndim - 2, xIdx = ndim - 1;
        const [chunkD, chunkH, chunkW] = [chunks[zIdx], chunks[yIdx], chunks[xIdx]];

        const zStart = z * chunkD, yStart = y * chunkH, xStart = x * chunkW;
        const zEnd = Math.min(zStart + chunkD, shape[zIdx]);
        const yEnd = Math.min(yStart + chunkH, shape[yIdx]);
        const xEnd = Math.min(xStart + chunkW, shape[xIdx]);

        let selection;
        if (ndim === 5) {
            selection = [t, currentChannel, zarr.slice(zStart, zEnd), zarr.slice(yStart, yEnd), zarr.slice(xStart, xEnd)];
        } else if (ndim === 4) {
            selection = [currentChannel, zarr.slice(zStart, zEnd), zarr.slice(yStart, yEnd), zarr.slice(xStart, xEnd)];
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

        if (volumeVisual) {
            let u16data;
            if (data instanceof Uint16Array) {
                u16data = data;
            } else {
                const src = data instanceof Uint8Array ? data : new Uint8Array(data.buffer || data);
                u16data = new Uint16Array(src.length);
                for (let i = 0; i < src.length; i++) u16data[i] = src[i] * 257;
            }
            volumeVisual.setChunkDataU16(
                lod, t, z, y, x, u16data,
                xEnd - xStart, yEnd - yStart, zEnd - zStart
            );
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
    viewer.clearScene();
    pendingLoads.clear();

    const jsLevels = lodLevels.map(level => new wasmModule.JsLevelMetadata(
        level.volumeSize[2], level.volumeSize[1], level.volumeSize[0],
        level.chunkSize[2],  level.chunkSize[1],  level.chunkSize[0],
        level.voxelSize[2],  level.voxelSize[1],  level.voxelSize[0],
        level.scaleFactor,
        level.translation[2], level.translation[1], level.translation[0]
    ));

    const maxChunks = computeMaxChunks();
    const cfg = VOLUME_MODES[currentMode];
    volumeVisual = new wasmModule[cfg.ctor](viewer, jsLevels, maxChunks);
    viewer[cfg.add](volumeVisual);
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
    if (!volumeVisual || !volumeVisual.setAttenuation) return;
    // Slider 0..100 → 0.0..10.0 attenuation strength.
    const v = parseInt(document.getElementById('attenuation').value) / 10.0;
    document.getElementById('attenuation-value').textContent = v.toFixed(2);
    volumeVisual.setAttenuation(v);
}

function applyIsoThreshold() {
    if (!volumeVisual || !volumeVisual.setIsoThreshold) return;
    // Slider 1..999 → 0.001..0.999 in contrast-normalised raw units.
    const v = parseInt(document.getElementById('iso-threshold').value) / 1000.0;
    document.getElementById('iso-threshold-value').textContent = v.toFixed(3);
    volumeVisual.setIsoThreshold(v);
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
    if (!volumeVisual) return;
    const min = sliderToValue(parseInt(document.getElementById('contrast-min').value));
    const max = sliderToValue(parseInt(document.getElementById('contrast-max').value));
    volumeVisual.setContrast(min, max);
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
        if (ndim === 5)      selection = [0, currentChannel, sliceZ, sliceY, sliceX];
        else if (ndim === 4) selection = [currentChannel, sliceZ, sliceY, sliceX];
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
    if (!volumeVisual) return;
    // Slider maps to relative step size in LOD-0 voxels.
    // 100 → 1.0 voxel/step (default). Range 10→0.1 (oversampling) to 800→8.0 (fast/coarse).
    const relative = parseFloat(document.getElementById('step-size').value) / 100.0;
    document.getElementById('step-size-value').textContent = relative.toFixed(2) + ' vox';
    volumeVisual.setRelativeStepSize(relative);
}

let densityScaleExp = 0;  // log10 exponent; slider range -20..+20 → 0.01× to 100×

function applyDensityScale() {
    if (!volumeVisual || !volumeVisual.setDensityScale) return;
    // Match kiln's normalization: extinction = cs.a * stepSize * maxDim * 0.5
    // where stepSize is in normalized [0,1] volume space and maxDim is in voxels.
    // Equivalent in world-space: density_scale = 0.5 * maxDim / volumeScale,
    // so extinction = cs.a * (0.5 * maxDim / volumeScale) * advance
    //               ≈ cs.a * 0.5 * step_in_voxels  (physically correct per-voxel opacity)
    const lod0 = lodLevels[0];
    const maxDim = Math.max(lod0.volumeSize[0], lod0.volumeSize[1], lod0.volumeSize[2]);
    const baseScale = 0.5 * maxDim / Math.max(volumeScale, 1e-9);
    const multiplier = Math.pow(10, densityScaleExp / 10);
    volumeVisual.setDensityScale(baseScale * multiplier);
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
    } else if (isMiddleDragging && volumeVisual) {
        windowCenter += dx * 0.002 * contrastCeiling;
        windowWidth  -= dy * 0.002 * contrastCeiling;
        windowWidth   = Math.max(0.001, windowWidth);
        const min = windowCenter - windowWidth / 2;
        const max = windowCenter + windowWidth / 2;
        volumeVisual.setContrast(min, max);
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
    if ((e.key === 'd' || e.key === 'D') && volumeVisual && volumeVisual.setDebugMode) {
        debugMode = !debugMode;
        atlasDebugMode = false; stepDebugMode = false;
        volumeVisual.setDebugMode(debugMode);
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
            const chunks = zarrArray.chunks;
            const ndim = shape.length;
            const zIdx = ndim - 3, yIdx = ndim - 2, xIdx = ndim - 1;

            const volumeSize = [shape[zIdx], shape[yIdx], shape[xIdx]];
            const chunkSize  = [chunks[zIdx], chunks[yIdx], chunks[xIdx]];

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
        const numChannels = hasChannels
            ? (firstArray.shape.length === 5 ? firstArray.shape[1] : firstArray.shape[0])
            : 1;

        const channelSelect = document.getElementById('channel-select');
        channelSelect.innerHTML = '';
        for (let i = 0; i < numChannels; i++) {
            const opt = document.createElement('option');
            opt.value = i;
            opt.textContent = `Channel ${i}`;
            channelSelect.appendChild(opt);
        }
        channelSelect.disabled = numChannels <= 1;

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

document.getElementById('channel-select').addEventListener('change', e => {
    currentChannel = parseInt(e.target.value);
    if (volumeVisual && lodLevels.length > 0) createVisual();
});

document.getElementById('lod-bias').addEventListener('input', e => {
    const v = parseFloat(e.target.value) / 10.0;
    document.getElementById('lod-bias-value').textContent = v.toFixed(1);
    if (volumeVisual) volumeVisual.setLodBias(v);
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
    // Recreate the visual with the new tile budget when a dataset is loaded.
    if (volumeVisual && lodLevels.length > 0) createVisual();
});

document.getElementById('render-mode').addEventListener('change', e => {
    currentMode = e.target.value;
    if (volumeVisual && lodLevels.length > 0) createVisual();
});

document.getElementById('attenuation').addEventListener('input', applyAttenuation);
document.getElementById('iso-threshold').addEventListener('input', applyIsoThreshold);

document.getElementById('debug-mode').addEventListener('change', e => {
    debugMode = e.target.checked;
    if (debugMode) { atlasDebugMode = false; document.getElementById('atlas-debug-mode').checked = false; }
    if (volumeVisual && volumeVisual.setDebugMode) volumeVisual.setDebugMode(debugMode);
});

document.getElementById('atlas-debug-mode').addEventListener('change', e => {
    atlasDebugMode = e.target.checked;
    if (atlasDebugMode) {
        debugMode = false; stepDebugMode = false;
        document.getElementById('debug-mode').checked = false;
        document.getElementById('step-debug-mode').checked = false;
    }
    if (volumeVisual && volumeVisual.setAtlasDebugMode) volumeVisual.setAtlasDebugMode(atlasDebugMode);
});

document.getElementById('step-debug-mode').addEventListener('change', e => {
    stepDebugMode = e.target.checked;
    if (stepDebugMode) {
        debugMode = false; atlasDebugMode = false;
        document.getElementById('debug-mode').checked = false;
        document.getElementById('atlas-debug-mode').checked = false;
    }
    if (volumeVisual && volumeVisual.setStepDebugMode) volumeVisual.setStepDebugMode(stepDebugMode);
});

// Stats update
setInterval(() => {
    if (volumeVisual) {
        const stats = volumeVisual.getStats();
        if (stats) {
            loadedChunksEl.textContent = stats[0];
            visibleChunksEl.textContent = stats[1];
        }
    }
}, 500);

// Render loop
function renderLoop() {
    if (viewer && wasmInitialized) {
        try { viewer.renderFrame(); }
        catch (e) { console.error('Render error:', e); }
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

        viewer = await wasmModule.JsViewer.new('canvas');
        wasmInitialized = true;

        setStatus('Ready — select a dataset', 'status-ready');
        renderLoop();

    } catch (error) {
        console.error('Init error:', error);
        setStatus(`Error: ${error.message}`, 'status-error');
    }
}

init();
