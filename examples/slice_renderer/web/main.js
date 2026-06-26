import * as zarr from 'https://cdn.jsdelivr.net/npm/zarrita@latest/+esm';

// Logical atlas tile size [z, y, x], uniform across all LOD levels. This is
// bovista's *virtual* tiling and is deliberately decoupled from the dataset's
// on-disk chunk shape — zarrita's slice reads assemble any region from the
// stored chunks, so we never need to match them. (bovista's virtual texture
// assumes one tile size for all LODs; datasets like marmoset_neurons use
// different storage chunks per level, which would otherwise break that.)
// Tuning this toward the dataset's chunking reduces over-fetch but isn't
// required for correctness.
const LOGICAL_TILE = [64, 64, 64];

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
    }
};

const MAX_PENDING_CHUNKS = 64; // Increased to handle more concurrent loads

// Global State
let wasmModule = null;
let wasmInitialized = false;
let viewer = null;
let device = null;
let queue = null;
let context = null;
let format = null;
let tiledImage = null;
let zarrStore = null;
let zarrRoot = null;
let zarrArrays = [];
let currentChannel = 0;
let lodLevels = [];
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
        if (!tiledImage) return;
        const w = tiledImage.wantedKeys();
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

/**
 * Async chunk loading from Zarr
 */
async function loadChunkAsync(lod, t, z, y, x, key) {
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

        const [chunkDepth, chunkHeight, chunkWidth] = LOGICAL_TILE;

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
                currentChannel,  // C - index (not slice!)
                zarr.slice(zStart, zEnd),
                zarr.slice(yStart, yEnd),
                zarr.slice(xStart, xEnd)
            ];
        } else if (ndim === 4) {
            // CZYX - index into channel, slice spatial dims
            selection = [
                currentChannel,  // C - index (not slice!)
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

        // Hand the native tile to bovista — it normalizes the full dtype
        // range to [0, 1] itself, so no client-side rescaling.
        if (tiledImage) {
            if (data instanceof Uint16Array) {
                tiledImage.setChunkDataU16(lod, t, z, y, x, data, dz, dy, dx);
            } else {
                const src = data instanceof Uint8Array ? data : new Uint8Array(data.buffer || data);
                tiledImage.setChunkDataU8(lod, t, z, y, x, src, dz, dy, dx);
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
    if (!tiledImage) return;

    // bovista computes the plane normal from the two angles + offset and
    // returns it so we can face the slice head-on in orthographic mode.
    const n = tiledImage.setSliceFromAngles(
        volumeCenter[0], volumeCenter[1], volumeCenter[2],
        sliceAngleX, sliceAngleY, sliceOffset,
    );

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

    // Each shape/size argument is a [z, y, x] array (numpy order).
    const jsLevels = lodLevels.map(level =>
        new wasmModule.LevelMetadata(
            level.volumeSize,
            level.chunkSize,
            level.voxelSize,
            level.scaleFactor,
            level.translation,
        )
    );

    tiledImage = new wasmModule.Image(
        viewer,     // Pass the viewer instance
        jsLevels,   // Array of LevelMetadata
        512,        // max_chunks
    );

    viewer.addImage(tiledImage);
    startLoaderPoll();

    // Reapply slice plane if volumeCenter is set
    if (volumeCenter) {
        updateSlicePlane();
    }

    {
        tiledImage.setContrast(0.0, 1.0);
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
    } else if (isMiddleDragging && tiledImage) {
        // Horizontal drag: level (center); vertical drag: window (width)
        windowCenter += deltaX * 0.002;
        windowWidth  += deltaY * 0.002;
        windowWidth   = Math.max(0.001, windowWidth);
        tiledImage.setContrast(windowCenter - windowWidth / 2, windowCenter + windowWidth / 2);
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
        if (tiledImage) {
            tiledImage.setDebugMode(debugMode);
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
            // Logical tile, uniform across LODs — NOT the dataset's storage chunk.
            const chunkSize = LOGICAL_TILE;

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
        const numChannels = hasChannels ?
            (firstArray.shape.length === 5 ? firstArray.shape[1] : firstArray.shape[0]) : 1;

        const channelSelect = document.getElementById('channel-select');
        channelSelect.innerHTML = '';
        for (let i = 0; i < numChannels; i++) {
            const option = document.createElement('option');
            option.value = i;
            option.textContent = `Channel ${i}`;
            channelSelect.appendChild(option);
        }
        channelSelect.disabled = numChannels <= 1;

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

        tiledImage.setContrast(0.0, 1.0);

        const stats = tiledImage.getStats();

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

document.getElementById('channel-select').addEventListener('change', (e) => {
    currentChannel = parseInt(e.target.value);

    // Recreate the visual to clear Rust-side cache
    if (tiledImage && lodLevels.length > 0) {
        createVisual();
    }
});

document.getElementById('lod-bias').addEventListener('input', (e) => {
    const value = parseFloat(e.target.value) / 10.0;
    document.getElementById('lod-bias-value').textContent = value.toFixed(1);
    if (tiledImage) {
        tiledImage.setLodBias(value);
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
    if (tiledImage) {
        tiledImage.setDebugMode(debugMode);
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
    if (!tiledImage) return;
    const min = parseFloat(document.getElementById('contrast-min').value) / 1000.0;
    const max = parseFloat(document.getElementById('contrast-max').value) / 1000.0;
    tiledImage.setContrast(min, max);
}

// Stats update interval
setInterval(() => {
    if (tiledImage) {
        const stats = tiledImage.getStats();
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

            // Log stats every 60 frames (about 1 second)
            frameCount++;
            if (frameCount % 60 === 0 && tiledImage) {
                const stats = tiledImage.getStats();
            }
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
        wasmModule = await import('../../pkg/bovista.js');
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
