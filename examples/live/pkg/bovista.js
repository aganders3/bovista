export class AverageVolume {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        AverageVolumeFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_averagevolume_free(ptr, 0);
    }
    /**
     * Returns [loaded, visible] tile counts.
     * @returns {Uint32Array}
     */
    getStats() {
        const ret = wasm.jsaveragevolume_getStats(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @param {Viewer} viewer
     * @param {LevelMetadata[]} levels
     * @param {number} max_chunks
     * @param {number | null} [atlas_count]
     */
    constructor(viewer, levels, max_chunks, atlas_count) {
        _assertClass(viewer, Viewer);
        const ptr0 = passArrayJsValueToWasm0(levels, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsaveragevolume_new(viewer.__wbg_ptr, ptr0, len0, max_chunks, isLikeNone(atlas_count) ? Number.MAX_SAFE_INTEGER : (atlas_count) >>> 0);
        this.__wbg_ptr = ret;
        AverageVolumeFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Set the blend mode (Normal or Additive). Additive enables
     * order-independent multi-channel compositing.
     * @param {BlendMode} mode
     */
    setBlendMode(mode) {
        const ret = wasm.jsaveragevolume_setBlendMode(this.__wbg_ptr, mode);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Provide uint16 tile data (full u16 range maps to [0, 1]).
     *
     * TODO: switch to `({ lod, t, z, y, x, shape, channel }, data)` object-args
     * when multi-channel support lands.
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint16Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU16(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsaveragevolume_setChunkDataU16(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Provide uint8 tile data (full u8 range maps to [0, 1]).
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint8Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU8(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsaveragevolume_setChunkDataU8(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
     * Pass a zero-length array to reset to grayscale.
     * @param {Uint8Array} rgba
     */
    setColormap(rgba) {
        const ret = wasm.jsaveragevolume_setColormap(this.__wbg_ptr, rgba);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} min
     * @param {number} max
     */
    setContrast(min, max) {
        const ret = wasm.jsaveragevolume_setContrast(this.__wbg_ptr, min, max);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} bias
     */
    setLodBias(bias) {
        const ret = wasm.jsaveragevolume_setLodBias(this.__wbg_ptr, bias);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set per-visual opacity in [0, 1].
     * @param {number} opacity
     */
    setOpacity(opacity) {
        const ret = wasm.jsaveragevolume_setOpacity(this.__wbg_ptr, opacity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} step
     */
    setRelativeStepSize(step) {
        const ret = wasm.jsaveragevolume_setRelativeStepSize(this.__wbg_ptr, step);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Snapshot of the tile keys bovista currently wants. Returned as a
     * flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
     * sorted by priority.
     * @returns {Uint32Array}
     */
    wantedKeys() {
        const ret = wasm.jsaveragevolume_wantedKeys(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) AverageVolume.prototype[Symbol.dispose] = AverageVolume.prototype.free;

/**
 * How a visual composites with the framebuffer. `Additive` (order-independent)
 * is the basis for multi-channel compositing.
 * @enum {0 | 1}
 */
export const BlendMode = Object.freeze({
    Normal: 0, "0": "Normal",
    Additive: 1, "1": "Additive",
});

export class DirectVolume {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        DirectVolumeFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_directvolume_free(ptr, 0);
    }
    /**
     * Returns [loaded, visible] tile counts.
     * @returns {Uint32Array}
     */
    getStats() {
        const ret = wasm.jsdirectvolume_getStats(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @param {Viewer} viewer
     * @param {LevelMetadata[]} levels
     * @param {number} max_chunks
     * @param {number | null} [atlas_count]
     */
    constructor(viewer, levels, max_chunks, atlas_count) {
        _assertClass(viewer, Viewer);
        const ptr0 = passArrayJsValueToWasm0(levels, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsdirectvolume_new(viewer.__wbg_ptr, ptr0, len0, max_chunks, isLikeNone(atlas_count) ? Number.MAX_SAFE_INTEGER : (atlas_count) >>> 0);
        this.__wbg_ptr = ret;
        DirectVolumeFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * @param {boolean} enabled
     */
    setAtlasDebugMode(enabled) {
        const ret = wasm.jsdirectvolume_setAtlasDebugMode(this.__wbg_ptr, enabled);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set the blend mode (Normal or Additive). Additive enables
     * order-independent multi-channel compositing.
     * @param {BlendMode} mode
     */
    setBlendMode(mode) {
        const ret = wasm.jsdirectvolume_setBlendMode(this.__wbg_ptr, mode);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Provide uint16 tile data (full u16 range maps to [0, 1]).
     *
     * TODO: switch to `({ lod, t, z, y, x, shape, channel }, data)` object-args
     * when multi-channel support lands.
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint16Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU16(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsdirectvolume_setChunkDataU16(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Provide uint8 tile data (full u8 range maps to [0, 1]).
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint8Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU8(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsdirectvolume_setChunkDataU8(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
     * Pass a zero-length array to reset to grayscale.
     * @param {Uint8Array} rgba
     */
    setColormap(rgba) {
        const ret = wasm.jsdirectvolume_setColormap(this.__wbg_ptr, rgba);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} min
     * @param {number} max
     */
    setContrast(min, max) {
        const ret = wasm.jsdirectvolume_setContrast(this.__wbg_ptr, min, max);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {boolean} enabled
     */
    setDebugMode(enabled) {
        const ret = wasm.jsdirectvolume_setDebugMode(this.__wbg_ptr, enabled);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} scale
     */
    setDensityScale(scale) {
        const ret = wasm.jsdirectvolume_setDensityScale(this.__wbg_ptr, scale);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} alpha
     */
    setEarlyExitAlpha(alpha) {
        const ret = wasm.jsdirectvolume_setEarlyExitAlpha(this.__wbg_ptr, alpha);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} bias
     */
    setLodBias(bias) {
        const ret = wasm.jsdirectvolume_setLodBias(this.__wbg_ptr, bias);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set per-visual opacity in [0, 1].
     * @param {number} opacity
     */
    setOpacity(opacity) {
        const ret = wasm.jsdirectvolume_setOpacity(this.__wbg_ptr, opacity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} step
     */
    setRelativeStepSize(step) {
        const ret = wasm.jsdirectvolume_setRelativeStepSize(this.__wbg_ptr, step);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {boolean} enabled
     */
    setStepDebugMode(enabled) {
        const ret = wasm.jsdirectvolume_setStepDebugMode(this.__wbg_ptr, enabled);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Snapshot of the tile keys bovista currently wants. Returned as a
     * flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
     * sorted by priority.
     * @returns {Uint32Array}
     */
    wantedKeys() {
        const ret = wasm.jsdirectvolume_wantedKeys(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) DirectVolume.prototype[Symbol.dispose] = DirectVolume.prototype.free;

export class Image {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ImageFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_image_free(ptr, 0);
    }
    /**
     * Returns [loaded, visible] tile counts.
     * @returns {Uint32Array}
     */
    getStats() {
        const ret = wasm.jsimage_getStats(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @param {Viewer} viewer
     * @param {LevelMetadata[]} levels
     * @param {number} max_chunks
     * @param {number | null} [atlas_count]
     */
    constructor(viewer, levels, max_chunks, atlas_count) {
        _assertClass(viewer, Viewer);
        const ptr0 = passArrayJsValueToWasm0(levels, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsimage_new(viewer.__wbg_ptr, ptr0, len0, max_chunks, isLikeNone(atlas_count) ? Number.MAX_SAFE_INTEGER : (atlas_count) >>> 0);
        this.__wbg_ptr = ret;
        ImageFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Set the blend mode (Normal or Additive). Additive enables
     * order-independent multi-channel compositing.
     * @param {BlendMode} mode
     */
    setBlendMode(mode) {
        const ret = wasm.jsimage_setBlendMode(this.__wbg_ptr, mode);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Provide uint16 tile data (full u16 range maps to [0, 1]).
     *
     * TODO: switch to `({ lod, t, z, y, x, shape, channel }, data)` object-args
     * when multi-channel support lands.
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint16Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU16(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsimage_setChunkDataU16(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Provide uint8 tile data (full u8 range maps to [0, 1]).
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint8Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU8(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsimage_setChunkDataU8(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
     * Pass a zero-length array to reset to grayscale.
     * @param {Uint8Array} rgba
     */
    setColormap(rgba) {
        const ret = wasm.jsimage_setColormap(this.__wbg_ptr, rgba);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} min
     * @param {number} max
     */
    setContrast(min, max) {
        const ret = wasm.jsimage_setContrast(this.__wbg_ptr, min, max);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * r" Enable or disable debug LOD tinting (green=LOD0, red=coarsest).
     * @param {boolean} enabled
     */
    setDebugMode(enabled) {
        const ret = wasm.jsimage_setDebugMode(this.__wbg_ptr, enabled);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} bias
     */
    setLodBias(bias) {
        const ret = wasm.jsimage_setLodBias(this.__wbg_ptr, bias);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set per-visual opacity in [0, 1].
     * @param {number} opacity
     */
    setOpacity(opacity) {
        const ret = wasm.jsimage_setOpacity(this.__wbg_ptr, opacity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * r" Set an oblique slice plane from two angles + an offset about `center`,
     * r" returning the resulting unit normal `[nx, ny, nz]` — pass it to
     * r" `Viewer.alignCameraToSlice` to face the slice head-on.
     * @param {number} cx
     * @param {number} cy
     * @param {number} cz
     * @param {number} angle_x
     * @param {number} angle_y
     * @param {number} offset
     * @returns {Float32Array}
     */
    setSliceFromAngles(cx, cy, cz, angle_x, angle_y, offset) {
        const ret = wasm.jsimage_setSliceFromAngles(this.__wbg_ptr, cx, cy, cz, angle_x, angle_y, offset);
        if (ret[3]) {
            throw takeFromExternrefTable0(ret[2]);
        }
        var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * r" Set an arbitrary slice plane from a point and normal.
     * @param {number} px
     * @param {number} py
     * @param {number} pz
     * @param {number} nx
     * @param {number} ny
     * @param {number} nz
     */
    setSlicePlane(px, py, pz, nx, ny, nz) {
        const ret = wasm.jsimage_setSlicePlane(this.__wbg_ptr, px, py, pz, nx, ny, nz);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} x
     */
    setSliceX(x) {
        const ret = wasm.jsimage_setSliceX(this.__wbg_ptr, x);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} y
     */
    setSliceY(y) {
        const ret = wasm.jsimage_setSliceY(this.__wbg_ptr, y);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} z
     */
    setSliceZ(z) {
        const ret = wasm.jsimage_setSliceZ(this.__wbg_ptr, z);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Snapshot of the tile keys bovista currently wants. Returned as a
     * flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
     * sorted by priority.
     * @returns {Uint32Array}
     */
    wantedKeys() {
        const ret = wasm.jsimage_wantedKeys(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) Image.prototype[Symbol.dispose] = Image.prototype.free;

export class IsosurfaceVolume {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        IsosurfaceVolumeFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_isosurfacevolume_free(ptr, 0);
    }
    /**
     * Returns [loaded, visible] tile counts.
     * @returns {Uint32Array}
     */
    getStats() {
        const ret = wasm.jsisosurfacevolume_getStats(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @param {Viewer} viewer
     * @param {LevelMetadata[]} levels
     * @param {number} max_chunks
     * @param {number | null} [atlas_count]
     */
    constructor(viewer, levels, max_chunks, atlas_count) {
        _assertClass(viewer, Viewer);
        const ptr0 = passArrayJsValueToWasm0(levels, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsisosurfacevolume_new(viewer.__wbg_ptr, ptr0, len0, max_chunks, isLikeNone(atlas_count) ? Number.MAX_SAFE_INTEGER : (atlas_count) >>> 0);
        this.__wbg_ptr = ret;
        IsosurfaceVolumeFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Set the blend mode (Normal or Additive). Additive enables
     * order-independent multi-channel compositing.
     * @param {BlendMode} mode
     */
    setBlendMode(mode) {
        const ret = wasm.jsisosurfacevolume_setBlendMode(this.__wbg_ptr, mode);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Provide uint16 tile data (full u16 range maps to [0, 1]).
     *
     * TODO: switch to `({ lod, t, z, y, x, shape, channel }, data)` object-args
     * when multi-channel support lands.
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint16Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU16(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsisosurfacevolume_setChunkDataU16(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Provide uint8 tile data (full u8 range maps to [0, 1]).
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint8Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU8(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsisosurfacevolume_setChunkDataU8(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
     * Pass a zero-length array to reset to grayscale.
     * @param {Uint8Array} rgba
     */
    setColormap(rgba) {
        const ret = wasm.jsisosurfacevolume_setColormap(this.__wbg_ptr, rgba);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} min
     * @param {number} max
     */
    setContrast(min, max) {
        const ret = wasm.jsisosurfacevolume_setContrast(this.__wbg_ptr, min, max);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} threshold
     */
    setIsoThreshold(threshold) {
        const ret = wasm.jsisosurfacevolume_setIsoThreshold(this.__wbg_ptr, threshold);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} bias
     */
    setLodBias(bias) {
        const ret = wasm.jsisosurfacevolume_setLodBias(this.__wbg_ptr, bias);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set per-visual opacity in [0, 1].
     * @param {number} opacity
     */
    setOpacity(opacity) {
        const ret = wasm.jsisosurfacevolume_setOpacity(this.__wbg_ptr, opacity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} step
     */
    setRelativeStepSize(step) {
        const ret = wasm.jsisosurfacevolume_setRelativeStepSize(this.__wbg_ptr, step);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Snapshot of the tile keys bovista currently wants. Returned as a
     * flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
     * sorted by priority.
     * @returns {Uint32Array}
     */
    wantedKeys() {
        const ret = wasm.jsisosurfacevolume_wantedKeys(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) IsosurfaceVolume.prototype[Symbol.dispose] = IsosurfaceVolume.prototype.free;

/**
 * Level metadata for multi-resolution LOD images
 *
 * Describes a single LOD level in a chunked image.
 */
export class LevelMetadata {
    static __unwrap(jsValue) {
        if (!(jsValue instanceof LevelMetadata)) {
            return 0;
        }
        return jsValue.__destroy_into_raw();
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        LevelMetadataFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_levelmetadata_free(ptr, 0);
    }
    /**
     * Chunk shape as `[z, y, x]` voxel counts.
     * @returns {Uint32Array}
     */
    get chunk_size() {
        const ret = wasm.jslevelmetadata_chunk_size(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * Construct a level descriptor. Each array is `[z, y, x]` numpy order.
     *
     * ```js
     * new LevelMetadata(
     *   [1024, 1024, 1024],   // volume_shape: voxel counts along z, y, x
     *   [64, 64, 64],         // chunk_shape:  tile voxel counts
     *   [1.0, 1.0, 1.0],      // voxel_size:   world-space units per voxel
     *   1.0,                  // scale_factor: relative to LOD 0
     *   [0.0, 0.0, 0.0],      // translation:  world-space origin offset
     * );
     * ```
     * @param {Uint32Array} volume_shape
     * @param {Uint32Array} chunk_shape
     * @param {Float32Array} voxel_size
     * @param {number} scale_factor
     * @param {Float32Array} translation
     */
    constructor(volume_shape, chunk_shape, voxel_size, scale_factor, translation) {
        const ptr0 = passArray32ToWasm0(volume_shape, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ptr1 = passArray32ToWasm0(chunk_shape, wasm.__wbindgen_malloc);
        const len1 = WASM_VECTOR_LEN;
        const ptr2 = passArrayF32ToWasm0(voxel_size, wasm.__wbindgen_malloc);
        const len2 = WASM_VECTOR_LEN;
        const ptr3 = passArrayF32ToWasm0(translation, wasm.__wbindgen_malloc);
        const len3 = WASM_VECTOR_LEN;
        const ret = wasm.jslevelmetadata_new(ptr0, len0, ptr1, len1, ptr2, len2, scale_factor, ptr3, len3);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        LevelMetadataFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Get scale factor
     * @returns {number}
     */
    get scale_factor() {
        const ret = wasm.jslevelmetadata_scale_factor(this.__wbg_ptr);
        return ret;
    }
    /**
     * Volume shape as `[z, y, x]` voxel counts.
     * @returns {Uint32Array}
     */
    get volume_size() {
        const ret = wasm.jslevelmetadata_volume_size(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * Voxel size as `[z, y, x]` world-space units per voxel.
     * @returns {Float32Array}
     */
    get voxel_size() {
        const ret = wasm.jslevelmetadata_voxel_size(this.__wbg_ptr);
        var v1 = getArrayF32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
}
if (Symbol.dispose) LevelMetadata.prototype[Symbol.dispose] = LevelMetadata.prototype.free;

/**
 * JavaScript wrapper for Lines — line segments and wireframes.
 */
export class Lines {
    static __wrap(ptr) {
        const obj = Object.create(Lines.prototype);
        obj.__wbg_ptr = ptr;
        LinesFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        LinesFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_lines_free(ptr, 0);
    }
    /**
     * Create a 3-axis helper (X=red, Y=green, Z=blue).
     * @param {Viewer} viewer
     * @param {number} length
     * @returns {Lines}
     */
    static axisHelper(viewer, length) {
        _assertClass(viewer, Viewer);
        const ret = wasm.jslines_axisHelper(viewer.__wbg_ptr, length);
        return Lines.__wrap(ret);
    }
    /**
     * Create a lines visual from flat Float32Arrays.
     *
     * Each consecutive pair of vertices defines one line segment.
     * `positions` and `colors` are flat arrays of length 3 × n_vertices.
     * @param {Viewer} viewer
     * @param {Float32Array} positions
     * @param {Float32Array} colors
     */
    constructor(viewer, positions, colors) {
        _assertClass(viewer, Viewer);
        const ret = wasm.jslines_new(viewer.__wbg_ptr, positions, colors);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        LinesFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Create a wireframe unit cube.
     * @param {Viewer} viewer
     * @returns {Lines}
     */
    static testCube(viewer) {
        _assertClass(viewer, Viewer);
        const ret = wasm.jslines_testCube(viewer.__wbg_ptr);
        return Lines.__wrap(ret);
    }
}
if (Symbol.dispose) Lines.prototype[Symbol.dispose] = Lines.prototype.free;

export class MinipVolume {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        MinipVolumeFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_minipvolume_free(ptr, 0);
    }
    /**
     * Returns [loaded, visible] tile counts.
     * @returns {Uint32Array}
     */
    getStats() {
        const ret = wasm.jsminipvolume_getStats(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @param {Viewer} viewer
     * @param {LevelMetadata[]} levels
     * @param {number} max_chunks
     * @param {number | null} [atlas_count]
     */
    constructor(viewer, levels, max_chunks, atlas_count) {
        _assertClass(viewer, Viewer);
        const ptr0 = passArrayJsValueToWasm0(levels, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsminipvolume_new(viewer.__wbg_ptr, ptr0, len0, max_chunks, isLikeNone(atlas_count) ? Number.MAX_SAFE_INTEGER : (atlas_count) >>> 0);
        this.__wbg_ptr = ret;
        MinipVolumeFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Set the blend mode (Normal or Additive). Additive enables
     * order-independent multi-channel compositing.
     * @param {BlendMode} mode
     */
    setBlendMode(mode) {
        const ret = wasm.jsminipvolume_setBlendMode(this.__wbg_ptr, mode);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Provide uint16 tile data (full u16 range maps to [0, 1]).
     *
     * TODO: switch to `({ lod, t, z, y, x, shape, channel }, data)` object-args
     * when multi-channel support lands.
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint16Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU16(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsminipvolume_setChunkDataU16(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Provide uint8 tile data (full u8 range maps to [0, 1]).
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint8Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU8(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsminipvolume_setChunkDataU8(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
     * Pass a zero-length array to reset to grayscale.
     * @param {Uint8Array} rgba
     */
    setColormap(rgba) {
        const ret = wasm.jsminipvolume_setColormap(this.__wbg_ptr, rgba);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} min
     * @param {number} max
     */
    setContrast(min, max) {
        const ret = wasm.jsminipvolume_setContrast(this.__wbg_ptr, min, max);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} bias
     */
    setLodBias(bias) {
        const ret = wasm.jsminipvolume_setLodBias(this.__wbg_ptr, bias);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set per-visual opacity in [0, 1].
     * @param {number} opacity
     */
    setOpacity(opacity) {
        const ret = wasm.jsminipvolume_setOpacity(this.__wbg_ptr, opacity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} step
     */
    setRelativeStepSize(step) {
        const ret = wasm.jsminipvolume_setRelativeStepSize(this.__wbg_ptr, step);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Snapshot of the tile keys bovista currently wants. Returned as a
     * flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
     * sorted by priority.
     * @returns {Uint32Array}
     */
    wantedKeys() {
        const ret = wasm.jsminipvolume_wantedKeys(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) MinipVolume.prototype[Symbol.dispose] = MinipVolume.prototype.free;

export class MipVolume {
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        MipVolumeFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_mipvolume_free(ptr, 0);
    }
    /**
     * Returns [loaded, visible] tile counts.
     * @returns {Uint32Array}
     */
    getStats() {
        const ret = wasm.jsmipvolume_getStats(this.__wbg_ptr);
        var v1 = getArrayU32FromWasm0(ret[0], ret[1]).slice();
        wasm.__wbindgen_free(ret[0], ret[1] * 4, 4);
        return v1;
    }
    /**
     * @param {Viewer} viewer
     * @param {LevelMetadata[]} levels
     * @param {number} max_chunks
     * @param {number | null} [atlas_count]
     */
    constructor(viewer, levels, max_chunks, atlas_count) {
        _assertClass(viewer, Viewer);
        const ptr0 = passArrayJsValueToWasm0(levels, wasm.__wbindgen_malloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsmipvolume_new(viewer.__wbg_ptr, ptr0, len0, max_chunks, isLikeNone(atlas_count) ? Number.MAX_SAFE_INTEGER : (atlas_count) >>> 0);
        this.__wbg_ptr = ret;
        MipVolumeFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * @param {number} attenuation
     */
    setAttenuation(attenuation) {
        const ret = wasm.jsmipvolume_setAttenuation(this.__wbg_ptr, attenuation);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set the blend mode (Normal or Additive). Additive enables
     * order-independent multi-channel compositing.
     * @param {BlendMode} mode
     */
    setBlendMode(mode) {
        const ret = wasm.jsmipvolume_setBlendMode(this.__wbg_ptr, mode);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Provide uint16 tile data (full u16 range maps to [0, 1]).
     *
     * TODO: switch to `({ lod, t, z, y, x, shape, channel }, data)` object-args
     * when multi-channel support lands.
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint16Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU16(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsmipvolume_setChunkDataU16(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Provide uint8 tile data (full u8 range maps to [0, 1]).
     * @param {number} lod
     * @param {number} t
     * @param {number} z
     * @param {number} y
     * @param {number} x
     * @param {Uint8Array} data
     * @param {number} z_shape
     * @param {number} y_shape
     * @param {number} x_shape
     */
    setChunkDataU8(lod, t, z, y, x, data, z_shape, y_shape, x_shape) {
        wasm.jsmipvolume_setChunkDataU8(this.__wbg_ptr, lod, t, z, y, x, data, z_shape, y_shape, x_shape);
    }
    /**
     * Set a colormap LUT (Uint8Array of 1024 bytes: 256 RGBA entries, values 0-255).
     * Pass a zero-length array to reset to grayscale.
     * @param {Uint8Array} rgba
     */
    setColormap(rgba) {
        const ret = wasm.jsmipvolume_setColormap(this.__wbg_ptr, rgba);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} min
     * @param {number} max
     */
    setContrast(min, max) {
        const ret = wasm.jsmipvolume_setContrast(this.__wbg_ptr, min, max);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} bias
     */
    setLodBias(bias) {
        const ret = wasm.jsmipvolume_setLodBias(this.__wbg_ptr, bias);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Set per-visual opacity in [0, 1].
     * @param {number} opacity
     */
    setOpacity(opacity) {
        const ret = wasm.jsmipvolume_setOpacity(this.__wbg_ptr, opacity);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * @param {number} step
     */
    setRelativeStepSize(step) {
        const ret = wasm.jsmipvolume_setRelativeStepSize(this.__wbg_ptr, step);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Snapshot of the tile keys bovista currently wants. Returned as a
     * flat Uint32Array `[lod, t, z, y, x, priority, lod, t, ...]`
     * sorted by priority.
     * @returns {Uint32Array}
     */
    wantedKeys() {
        const ret = wasm.jsmipvolume_wantedKeys(this.__wbg_ptr);
        return ret;
    }
}
if (Symbol.dispose) MipVolume.prototype[Symbol.dispose] = MipVolume.prototype.free;

/**
 * JavaScript wrapper for Points — colored point cloud.
 */
export class Points {
    static __wrap(ptr) {
        const obj = Object.create(Points.prototype);
        obj.__wbg_ptr = ptr;
        PointsFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        PointsFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_points_free(ptr, 0);
    }
    /**
     * Create a point cloud from flat Float32Arrays.
     *
     * `positions` is a flat array of XYZ triples; `colors` is a flat array of RGB triples
     * (values 0–1). Both must have length 3 × n_points.
     * @param {Viewer} viewer
     * @param {Float32Array} positions
     * @param {Float32Array} colors
     */
    constructor(viewer, positions, colors) {
        _assertClass(viewer, Viewer);
        const ret = wasm.jspoints_new(viewer.__wbg_ptr, positions, colors);
        if (ret[2]) {
            throw takeFromExternrefTable0(ret[1]);
        }
        this.__wbg_ptr = ret[0];
        PointsFinalization.register(this, this.__wbg_ptr, this);
        return this;
    }
    /**
     * Create a test cube of points.
     * @param {Viewer} viewer
     * @param {number} size
     * @returns {Points}
     */
    static testCube(viewer, size) {
        _assertClass(viewer, Viewer);
        const ret = wasm.jspoints_testCube(viewer.__wbg_ptr, size);
        return Points.__wrap(ret);
    }
}
if (Symbol.dispose) Points.prototype[Symbol.dispose] = Points.prototype.free;

/**
 * Camera projection mode
 * @enum {0 | 1}
 */
export const ProjectionMode = Object.freeze({
    /**
     * Perspective projection with field of view
     */
    Perspective: 0, "0": "Perspective",
    /**
     * Orthographic projection with fixed size
     */
    Orthographic: 1, "1": "Orthographic",
});

/**
 * JavaScript viewer for Bovista
 */
export class Viewer {
    static __wrap(ptr) {
        const obj = Object.create(Viewer.prototype);
        obj.__wbg_ptr = ptr;
        ViewerFinalization.register(obj, obj.__wbg_ptr, obj);
        return obj;
    }
    __destroy_into_raw() {
        const ptr = this.__wbg_ptr;
        this.__wbg_ptr = 0;
        ViewerFinalization.unregister(this);
        return ptr;
    }
    free() {
        const ptr = this.__destroy_into_raw();
        wasm.__wbg_viewer_free(ptr, 0);
    }
    /**
     * @param {AverageVolume} visual
     * @returns {number}
     */
    addAverageVolume(visual) {
        _assertClass(visual, AverageVolume);
        const ret = wasm.jsviewer_addAverageVolume(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {DirectVolume} visual
     * @returns {number}
     */
    addDirectVolume(visual) {
        _assertClass(visual, DirectVolume);
        const ret = wasm.jsviewer_addDirectVolume(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Add an image visual to the scene
     * @param {Image} visual
     * @returns {number}
     */
    addImage(visual) {
        _assertClass(visual, Image);
        const ret = wasm.jsviewer_addImage(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {IsosurfaceVolume} visual
     * @returns {number}
     */
    addIsosurfaceVolume(visual) {
        _assertClass(visual, IsosurfaceVolume);
        const ret = wasm.jsviewer_addIsosurfaceVolume(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Add a lines visual to the scene
     * @param {Lines} visual
     * @returns {number}
     */
    addLines(visual) {
        _assertClass(visual, Lines);
        const ret = wasm.jsviewer_addLines(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {MinipVolume} visual
     * @returns {number}
     */
    addMinipVolume(visual) {
        _assertClass(visual, MinipVolume);
        const ret = wasm.jsviewer_addMinipVolume(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {MipVolume} visual
     * @returns {number}
     */
    addMipVolume(visual) {
        _assertClass(visual, MipVolume);
        const ret = wasm.jsviewer_addMipVolume(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Add a points visual to the scene
     * @param {Points} visual
     * @returns {number}
     */
    addPoints(visual) {
        _assertClass(visual, Points);
        const ret = wasm.jsviewer_addPoints(this.__wbg_ptr, visual.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * Face an oblique slice plane head-on: position the camera `distance`
     * away from `(cx, cy, cz)` along the slice normal `(nx, ny, nz)`.
     * @param {number} cx
     * @param {number} cy
     * @param {number} cz
     * @param {number} nx
     * @param {number} ny
     * @param {number} nz
     * @param {number} distance
     */
    alignCameraToSlice(cx, cy, cz, nx, ny, nz, distance) {
        wasm.jsviewer_alignCameraToSlice(this.__wbg_ptr, cx, cy, cz, nx, ny, nz, distance);
    }
    clearScene() {
        wasm.jsviewer_clearScene(this.__wbg_ptr);
    }
    /**
     * @returns {number}
     */
    getCameraDistance() {
        const ret = wasm.jsviewer_getCameraDistance(this.__wbg_ptr);
        return ret;
    }
    /**
     * @returns {number}
     */
    getCameraOrthoHeight() {
        const ret = wasm.jsviewer_getCameraOrthoHeight(this.__wbg_ptr);
        return ret;
    }
    /**
     * @returns {ProjectionMode}
     */
    getCameraProjectionMode() {
        const ret = wasm.jsviewer_getCameraProjectionMode(this.__wbg_ptr);
        return ret;
    }
    /**
     * Create a new viewer from a canvas element ID
     *
     * This will initialize WebGPU and set up the render context
     * @param {string} canvas_id
     * @returns {Promise<Viewer>}
     */
    static new(canvas_id) {
        const ptr0 = passStringToWasm0(canvas_id, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
        const len0 = WASM_VECTOR_LEN;
        const ret = wasm.jsviewer_new(ptr0, len0);
        return ret;
    }
    /**
     * @param {number} delta_x
     * @param {number} delta_y
     */
    orbitCamera(delta_x, delta_y) {
        wasm.jsviewer_orbitCamera(this.__wbg_ptr, delta_x, delta_y);
    }
    /**
     * @param {number} delta_x
     * @param {number} delta_y
     */
    panCamera(delta_x, delta_y) {
        wasm.jsviewer_panCamera(this.__wbg_ptr, delta_x, delta_y);
    }
    /**
     * Render a single frame
     */
    renderFrame() {
        const ret = wasm.jsviewer_renderFrame(this.__wbg_ptr);
        if (ret[1]) {
            throw takeFromExternrefTable0(ret[0]);
        }
    }
    /**
     * Resize the viewer when canvas dimensions change
     * @param {number} width
     * @param {number} height
     */
    resize(width, height) {
        wasm.jsviewer_resize(this.__wbg_ptr, width, height);
    }
    /**
     * @param {number} near
     * @param {number} far
     */
    setCameraClipPlanes(near, far) {
        wasm.jsviewer_setCameraClipPlanes(this.__wbg_ptr, near, far);
    }
    /**
     * @param {number} height
     */
    setCameraOrthoHeight(height) {
        wasm.jsviewer_setCameraOrthoHeight(this.__wbg_ptr, height);
    }
    /**
     * @param {number} x
     * @param {number} y
     * @param {number} z
     */
    setCameraPosition(x, y, z) {
        wasm.jsviewer_setCameraPosition(this.__wbg_ptr, x, y, z);
    }
    /**
     * @param {ProjectionMode} mode
     */
    setCameraProjectionMode(mode) {
        wasm.jsviewer_setCameraProjectionMode(this.__wbg_ptr, mode);
    }
    /**
     * @param {number} x
     * @param {number} y
     * @param {number} z
     */
    setCameraTarget(x, y, z) {
        wasm.jsviewer_setCameraTarget(this.__wbg_ptr, x, y, z);
    }
    /**
     * @param {number} x
     * @param {number} y
     * @param {number} z
     */
    setCameraUp(x, y, z) {
        wasm.jsviewer_setCameraUp(this.__wbg_ptr, x, y, z);
    }
    /**
     * @returns {number}
     */
    visualCount() {
        const ret = wasm.jsviewer_visualCount(this.__wbg_ptr);
        return ret >>> 0;
    }
    /**
     * @param {number} delta
     */
    zoomCamera(delta) {
        wasm.jsviewer_zoomCamera(this.__wbg_ptr, delta);
    }
}
if (Symbol.dispose) Viewer.prototype[Symbol.dispose] = Viewer.prototype.free;
function __wbg_get_imports() {
    const import0 = {
        __proto__: null,
        __wbg_Window_defb2c76b3875a73: function(arg0) {
            const ret = arg0.Window;
            return ret;
        },
        __wbg_WorkerGlobalScope_184e45d6565eb3f0: function(arg0) {
            const ret = arg0.WorkerGlobalScope;
            return ret;
        },
        __wbg___wbindgen_debug_string_edece8177ad01481: function(arg0, arg1) {
            const ret = debugString(arg1);
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg___wbindgen_is_function_5cd60d5cf78b4eef: function(arg0) {
            const ret = typeof(arg0) === 'function';
            return ret;
        },
        __wbg___wbindgen_is_null_2042690d351e14f0: function(arg0) {
            const ret = arg0 === null;
            return ret;
        },
        __wbg___wbindgen_is_undefined_35bb9f4c7fd651d5: function(arg0) {
            const ret = arg0 === undefined;
            return ret;
        },
        __wbg___wbindgen_throw_9c31b086c2b26051: function(arg0, arg1) {
            throw new Error(getStringFromWasm0(arg0, arg1));
        },
        __wbg__wbg_cb_unref_3fa391f3fcdb55f8: function(arg0) {
            arg0._wbg_cb_unref();
        },
        __wbg_beginRenderPass_8f60a9940800c223: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.beginRenderPass(arg1);
            return ret;
        }, arguments); },
        __wbg_buffer_8d6798e32d1afd34: function(arg0) {
            const ret = arg0.buffer;
            return ret;
        },
        __wbg_call_dfde26266607c996: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.call(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_configure_6e4ddb3faabc6005: function() { return handleError(function (arg0, arg1) {
            arg0.configure(arg1);
        }, arguments); },
        __wbg_createBindGroupLayout_1dad97f44fda2929: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.createBindGroupLayout(arg1);
            return ret;
        }, arguments); },
        __wbg_createBindGroup_0ee0646375643dfb: function(arg0, arg1) {
            const ret = arg0.createBindGroup(arg1);
            return ret;
        },
        __wbg_createBuffer_474baee88e74f68a: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.createBuffer(arg1);
            return ret;
        }, arguments); },
        __wbg_createCommandEncoder_5229e112300218b4: function(arg0, arg1) {
            const ret = arg0.createCommandEncoder(arg1);
            return ret;
        },
        __wbg_createPipelineLayout_bf0deba7918ca157: function(arg0, arg1) {
            const ret = arg0.createPipelineLayout(arg1);
            return ret;
        },
        __wbg_createRenderPipeline_7baf812b1e7e39e8: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.createRenderPipeline(arg1);
            return ret;
        }, arguments); },
        __wbg_createSampler_5970d585f322df95: function(arg0, arg1) {
            const ret = arg0.createSampler(arg1);
            return ret;
        },
        __wbg_createShaderModule_744311a1dd685c14: function(arg0, arg1) {
            const ret = arg0.createShaderModule(arg1);
            return ret;
        },
        __wbg_createTexture_f357c38256411287: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.createTexture(arg1);
            return ret;
        }, arguments); },
        __wbg_createView_e7a36ec95c4190f3: function() { return handleError(function (arg0, arg1) {
            const ret = arg0.createView(arg1);
            return ret;
        }, arguments); },
        __wbg_document_3540635616a18455: function(arg0) {
            const ret = arg0.document;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_drawIndexed_2ab8259f769f352d: function(arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.drawIndexed(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4, arg5 >>> 0);
        },
        __wbg_draw_b93abd56938fa3b6: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.draw(arg1 >>> 0, arg2 >>> 0, arg3 >>> 0, arg4 >>> 0);
        },
        __wbg_end_2fdce848e1c526bc: function(arg0) {
            arg0.end();
        },
        __wbg_error_a6fa202b58aa1cd3: function(arg0, arg1) {
            let deferred0_0;
            let deferred0_1;
            try {
                deferred0_0 = arg0;
                deferred0_1 = arg1;
                console.error(getStringFromWasm0(arg0, arg1));
            } finally {
                wasm.__wbindgen_free(deferred0_0, deferred0_1, 1);
            }
        },
        __wbg_finish_7370ad1c0e62b448: function(arg0) {
            const ret = arg0.finish();
            return ret;
        },
        __wbg_finish_797b32d15bab2338: function(arg0, arg1) {
            const ret = arg0.finish(arg1);
            return ret;
        },
        __wbg_getContext_47ea64e14d931e3e: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.getContext(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getContext_e1463ff7aa682d57: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.getContext(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        }, arguments); },
        __wbg_getCurrentTexture_628ddb7c08d25190: function() { return handleError(function (arg0) {
            const ret = arg0.getCurrentTexture();
            return ret;
        }, arguments); },
        __wbg_getElementById_78449141d07cd8ef: function(arg0, arg1, arg2) {
            const ret = arg0.getElementById(getStringFromWasm0(arg1, arg2));
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_getMappedRange_9f13d158ba3946fd: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.getMappedRange(arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_getPreferredCanvasFormat_a2b6306e828c2f60: function(arg0) {
            const ret = arg0.getPreferredCanvasFormat();
            return (__wbindgen_enum_GpuTextureFormat.indexOf(ret) + 1 || 96) - 1;
        },
        __wbg_get_3c19db9bed86ee3b: function(arg0, arg1) {
            const ret = arg0[arg1 >>> 0];
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_gpu_ac6dc8fb638a26c3: function(arg0) {
            const ret = arg0.gpu;
            return ret;
        },
        __wbg_height_aef2a2eb10d0d530: function(arg0) {
            const ret = arg0.height;
            return ret;
        },
        __wbg_instanceof_GpuAdapter_fb230cdccb184887: function(arg0) {
            let result;
            try {
                result = arg0 instanceof GPUAdapter;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_GpuCanvasContext_48ec5330c4425d84: function(arg0) {
            let result;
            try {
                result = arg0 instanceof GPUCanvasContext;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_HtmlCanvasElement_a02da0a417f1bf3f: function(arg0) {
            let result;
            try {
                result = arg0 instanceof HTMLCanvasElement;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_instanceof_Window_faa5cf994f49cca7: function(arg0) {
            let result;
            try {
                result = arg0 instanceof Window;
            } catch (_) {
                result = false;
            }
            const ret = result;
            return ret;
        },
        __wbg_label_84abde6506fa15b7: function(arg0, arg1) {
            const ret = arg1.label;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_length_0adf2f3c5da33087: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_13e61aa81636ec86: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_length_56fcd3e2b7e0299d: function(arg0) {
            const ret = arg0.length;
            return ret;
        },
        __wbg_levelmetadata_unwrap: function(arg0) {
            const ret = LevelMetadata.__unwrap(arg0);
            return ret;
        },
        __wbg_limits_a80585efb442d985: function(arg0) {
            const ret = arg0.limits;
            return ret;
        },
        __wbg_limits_d4ef440224084d65: function(arg0) {
            const ret = arg0.limits;
            return ret;
        },
        __wbg_log_eb752234eec406d1: function(arg0) {
            console.log(arg0);
        },
        __wbg_maxBindGroups_2fac2855adae76bb: function(arg0) {
            const ret = arg0.maxBindGroups;
            return ret;
        },
        __wbg_maxBindingsPerBindGroup_7b8b526a1900701d: function(arg0) {
            const ret = arg0.maxBindingsPerBindGroup;
            return ret;
        },
        __wbg_maxBufferSize_475dcdac4810587e: function(arg0) {
            const ret = arg0.maxBufferSize;
            return ret;
        },
        __wbg_maxColorAttachmentBytesPerSample_5a04a2d5a9e122bb: function(arg0) {
            const ret = arg0.maxColorAttachmentBytesPerSample;
            return ret;
        },
        __wbg_maxColorAttachments_2a487bb283cf8f08: function(arg0) {
            const ret = arg0.maxColorAttachments;
            return ret;
        },
        __wbg_maxComputeInvocationsPerWorkgroup_72908780df74614a: function(arg0) {
            const ret = arg0.maxComputeInvocationsPerWorkgroup;
            return ret;
        },
        __wbg_maxComputeWorkgroupSizeX_92eeefd082aefd07: function(arg0) {
            const ret = arg0.maxComputeWorkgroupSizeX;
            return ret;
        },
        __wbg_maxComputeWorkgroupSizeY_54fecee60bfa33ae: function(arg0) {
            const ret = arg0.maxComputeWorkgroupSizeY;
            return ret;
        },
        __wbg_maxComputeWorkgroupSizeZ_868f559f4eebf95f: function(arg0) {
            const ret = arg0.maxComputeWorkgroupSizeZ;
            return ret;
        },
        __wbg_maxComputeWorkgroupStorageSize_b4fd07bd827bde34: function(arg0) {
            const ret = arg0.maxComputeWorkgroupStorageSize;
            return ret;
        },
        __wbg_maxComputeWorkgroupsPerDimension_2d88e3f8e1f984fc: function(arg0) {
            const ret = arg0.maxComputeWorkgroupsPerDimension;
            return ret;
        },
        __wbg_maxDynamicStorageBuffersPerPipelineLayout_d6f0075161a2ec8e: function(arg0) {
            const ret = arg0.maxDynamicStorageBuffersPerPipelineLayout;
            return ret;
        },
        __wbg_maxDynamicUniformBuffersPerPipelineLayout_4fc01fff17019d2b: function(arg0) {
            const ret = arg0.maxDynamicUniformBuffersPerPipelineLayout;
            return ret;
        },
        __wbg_maxSampledTexturesPerShaderStage_224452301226d09a: function(arg0) {
            const ret = arg0.maxSampledTexturesPerShaderStage;
            return ret;
        },
        __wbg_maxSamplersPerShaderStage_34936f99dde7ae66: function(arg0) {
            const ret = arg0.maxSamplersPerShaderStage;
            return ret;
        },
        __wbg_maxStorageBufferBindingSize_9582c7cfe68faac0: function(arg0) {
            const ret = arg0.maxStorageBufferBindingSize;
            return ret;
        },
        __wbg_maxStorageBuffersPerShaderStage_6403069e851040db: function(arg0) {
            const ret = arg0.maxStorageBuffersPerShaderStage;
            return ret;
        },
        __wbg_maxStorageTexturesPerShaderStage_e44d7d6ed85de663: function(arg0) {
            const ret = arg0.maxStorageTexturesPerShaderStage;
            return ret;
        },
        __wbg_maxTextureArrayLayers_0f2fe636cc0fa44f: function(arg0) {
            const ret = arg0.maxTextureArrayLayers;
            return ret;
        },
        __wbg_maxTextureDimension1D_501e1e21119c564f: function(arg0) {
            const ret = arg0.maxTextureDimension1D;
            return ret;
        },
        __wbg_maxTextureDimension2D_ac594a0008180ab6: function(arg0) {
            const ret = arg0.maxTextureDimension2D;
            return ret;
        },
        __wbg_maxTextureDimension3D_8e5638a46d5786b0: function(arg0) {
            const ret = arg0.maxTextureDimension3D;
            return ret;
        },
        __wbg_maxUniformBufferBindingSize_a4c4feca62b81571: function(arg0) {
            const ret = arg0.maxUniformBufferBindingSize;
            return ret;
        },
        __wbg_maxUniformBuffersPerShaderStage_505f14dd3f814492: function(arg0) {
            const ret = arg0.maxUniformBuffersPerShaderStage;
            return ret;
        },
        __wbg_maxVertexAttributes_c4f4881b2d20616b: function(arg0) {
            const ret = arg0.maxVertexAttributes;
            return ret;
        },
        __wbg_maxVertexBufferArrayStride_da0782b779fc1a38: function(arg0) {
            const ret = arg0.maxVertexBufferArrayStride;
            return ret;
        },
        __wbg_maxVertexBuffers_90e848bae7f01af3: function(arg0) {
            const ret = arg0.maxVertexBuffers;
            return ret;
        },
        __wbg_minStorageBufferOffsetAlignment_58fcc139ce14bbb9: function(arg0) {
            const ret = arg0.minStorageBufferOffsetAlignment;
            return ret;
        },
        __wbg_minUniformBufferOffsetAlignment_33ca7570ff80f825: function(arg0) {
            const ret = arg0.minUniformBufferOffsetAlignment;
            return ret;
        },
        __wbg_navigator_3334c390f542c642: function(arg0) {
            const ret = arg0.navigator;
            return ret;
        },
        __wbg_navigator_3db7ba343e05d4d1: function(arg0) {
            const ret = arg0.navigator;
            return ret;
        },
        __wbg_new_02d162bc6cf02f60: function() {
            const ret = new Object();
            return ret;
        },
        __wbg_new_227d7c05414eb861: function() {
            const ret = new Error();
            return ret;
        },
        __wbg_new_310879b66b6e95e1: function() {
            const ret = new Array();
            return ret;
        },
        __wbg_new_from_slice_269e35316ed2d061: function(arg0, arg1) {
            const ret = new Uint8Array(getArrayU8FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_from_slice_f92bf65e9a895613: function(arg0, arg1) {
            const ret = new Uint32Array(getArrayU32FromWasm0(arg0, arg1));
            return ret;
        },
        __wbg_new_typed_c072c4ce9a2a0cdf: function(arg0, arg1) {
            try {
                var state0 = {a: arg0, b: arg1};
                var cb0 = (arg0, arg1) => {
                    const a = state0.a;
                    state0.a = 0;
                    try {
                        return wasm_bindgen__convert__closures_____invoke__h61ab42adbe47db91(a, state0.b, arg0, arg1);
                    } finally {
                        state0.a = a;
                    }
                };
                const ret = new Promise(cb0);
                return ret;
            } finally {
                state0.a = 0;
            }
        },
        __wbg_new_with_byte_offset_and_length_a87e79143162d67f: function(arg0, arg1, arg2) {
            const ret = new Uint8Array(arg0, arg1 >>> 0, arg2 >>> 0);
            return ret;
        },
        __wbg_now_e7c6795a7f81e10f: function(arg0) {
            const ret = arg0.now();
            return ret;
        },
        __wbg_performance_3fcf6e32a7e1ed0a: function(arg0) {
            const ret = arg0.performance;
            return ret;
        },
        __wbg_prototypesetcall_5f9bdc8d75e07276: function(arg0, arg1, arg2) {
            Uint8Array.prototype.set.call(getArrayU8FromWasm0(arg0, arg1), arg2);
        },
        __wbg_prototypesetcall_ca5e190e7872edac: function(arg0, arg1, arg2) {
            Uint16Array.prototype.set.call(getArrayU16FromWasm0(arg0, arg1), arg2);
        },
        __wbg_prototypesetcall_fe9d129a614489fa: function(arg0, arg1, arg2) {
            Float32Array.prototype.set.call(getArrayF32FromWasm0(arg0, arg1), arg2);
        },
        __wbg_push_b77c476b01548d0a: function(arg0, arg1) {
            const ret = arg0.push(arg1);
            return ret;
        },
        __wbg_querySelectorAll_0981bdbbafa5bf17: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = arg0.querySelectorAll(getStringFromWasm0(arg1, arg2));
            return ret;
        }, arguments); },
        __wbg_queueMicrotask_78d584b53af520f5: function(arg0) {
            const ret = arg0.queueMicrotask;
            return ret;
        },
        __wbg_queueMicrotask_b39ea83c7f01971a: function(arg0) {
            queueMicrotask(arg0);
        },
        __wbg_queue_8cb065d04b06cb13: function(arg0) {
            const ret = arg0.queue;
            return ret;
        },
        __wbg_requestAdapter_4814cb479d2dcf15: function(arg0, arg1) {
            const ret = arg0.requestAdapter(arg1);
            return ret;
        },
        __wbg_requestDevice_1b8f321791aa8b00: function(arg0, arg1) {
            const ret = arg0.requestDevice(arg1);
            return ret;
        },
        __wbg_resolve_d17db9352f5a220e: function(arg0) {
            const ret = Promise.resolve(arg0);
            return ret;
        },
        __wbg_setBindGroup_9fdf11fd244aa68a: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5, arg6) {
            arg0.setBindGroup(arg1 >>> 0, arg2, getArrayU32FromWasm0(arg3, arg4), arg5, arg6 >>> 0);
        }, arguments); },
        __wbg_setBindGroup_a7e88440d0ecac88: function(arg0, arg1, arg2) {
            arg0.setBindGroup(arg1 >>> 0, arg2);
        },
        __wbg_setIndexBuffer_495adb9486108a2f: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.setIndexBuffer(arg1, __wbindgen_enum_GpuIndexFormat[arg2], arg3, arg4);
        },
        __wbg_setPipeline_092493bfd77760ee: function(arg0, arg1) {
            arg0.setPipeline(arg1);
        },
        __wbg_setVertexBuffer_1ed692dc2726f6c3: function(arg0, arg1, arg2, arg3, arg4) {
            arg0.setVertexBuffer(arg1 >>> 0, arg2, arg3, arg4);
        },
        __wbg_set_37221b90dcdc9a98: function(arg0, arg1, arg2) {
            arg0.set(arg1, arg2 >>> 0);
        },
        __wbg_set_a0e911be3da02782: function() { return handleError(function (arg0, arg1, arg2) {
            const ret = Reflect.set(arg0, arg1, arg2);
            return ret;
        }, arguments); },
        __wbg_set_a_9b35e9a1b60244b3: function(arg0, arg1) {
            arg0.a = arg1;
        },
        __wbg_set_access_67a4b0ae1c0f32f5: function(arg0, arg1) {
            arg0.access = __wbindgen_enum_GpuStorageTextureAccess[arg1];
        },
        __wbg_set_address_mode_u_1a44abe112b27750: function(arg0, arg1) {
            arg0.addressModeU = __wbindgen_enum_GpuAddressMode[arg1];
        },
        __wbg_set_address_mode_v_764da2dbc77dbf3e: function(arg0, arg1) {
            arg0.addressModeV = __wbindgen_enum_GpuAddressMode[arg1];
        },
        __wbg_set_address_mode_w_7c5135c7d56f9b4c: function(arg0, arg1) {
            arg0.addressModeW = __wbindgen_enum_GpuAddressMode[arg1];
        },
        __wbg_set_alpha_52d96a0bda4f4afe: function(arg0, arg1) {
            arg0.alpha = arg1;
        },
        __wbg_set_alpha_mode_5c1f4c4115bbe318: function(arg0, arg1) {
            arg0.alphaMode = __wbindgen_enum_GpuCanvasAlphaMode[arg1];
        },
        __wbg_set_alpha_to_coverage_enabled_36cef063cffb5d84: function(arg0, arg1) {
            arg0.alphaToCoverageEnabled = arg1 !== 0;
        },
        __wbg_set_array_layer_count_c2e975f5b62596bb: function(arg0, arg1) {
            arg0.arrayLayerCount = arg1 >>> 0;
        },
        __wbg_set_array_stride_eb9d409a95d2c1b3: function(arg0, arg1) {
            arg0.arrayStride = arg1;
        },
        __wbg_set_aspect_a453970e75d8e849: function(arg0, arg1) {
            arg0.aspect = __wbindgen_enum_GpuTextureAspect[arg1];
        },
        __wbg_set_attributes_edcbef656678a257: function(arg0, arg1) {
            arg0.attributes = arg1;
        },
        __wbg_set_b_5708a52cef50b19f: function(arg0, arg1) {
            arg0.b = arg1;
        },
        __wbg_set_base_array_layer_b67db9750fbc7053: function(arg0, arg1) {
            arg0.baseArrayLayer = arg1 >>> 0;
        },
        __wbg_set_base_mip_level_35f2fc3293e96083: function(arg0, arg1) {
            arg0.baseMipLevel = arg1 >>> 0;
        },
        __wbg_set_beginning_of_pass_write_index_8e570668cd8cad5e: function(arg0, arg1) {
            arg0.beginningOfPassWriteIndex = arg1 >>> 0;
        },
        __wbg_set_bind_group_layouts_436bd62a9b35e1ab: function(arg0, arg1) {
            arg0.bindGroupLayouts = arg1;
        },
        __wbg_set_binding_b575483e08d5ba4a: function(arg0, arg1) {
            arg0.binding = arg1 >>> 0;
        },
        __wbg_set_binding_c9feebb53a130ebe: function(arg0, arg1) {
            arg0.binding = arg1 >>> 0;
        },
        __wbg_set_blend_765238525504b1db: function(arg0, arg1) {
            arg0.blend = arg1;
        },
        __wbg_set_buffer_468874ee2bc9df02: function(arg0, arg1) {
            arg0.buffer = arg1;
        },
        __wbg_set_buffer_6c45652fb024e808: function(arg0, arg1) {
            arg0.buffer = arg1;
        },
        __wbg_set_buffers_986b265eda61fae6: function(arg0, arg1) {
            arg0.buffers = arg1;
        },
        __wbg_set_bytes_per_row_c127092e4fe9be22: function(arg0, arg1) {
            arg0.bytesPerRow = arg1 >>> 0;
        },
        __wbg_set_clear_value_764be6f32f502d0a: function(arg0, arg1) {
            arg0.clearValue = arg1;
        },
        __wbg_set_code_0a82fa86cf58ca3b: function(arg0, arg1, arg2) {
            arg0.code = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_color_attachments_fad73def169d9610: function(arg0, arg1) {
            arg0.colorAttachments = arg1;
        },
        __wbg_set_color_c25fb8769b4f010f: function(arg0, arg1) {
            arg0.color = arg1;
        },
        __wbg_set_compare_1e2214a402af6db3: function(arg0, arg1) {
            arg0.compare = __wbindgen_enum_GpuCompareFunction[arg1];
        },
        __wbg_set_compare_e61cc62d8f7e2191: function(arg0, arg1) {
            arg0.compare = __wbindgen_enum_GpuCompareFunction[arg1];
        },
        __wbg_set_count_f285d8c62f4de679: function(arg0, arg1) {
            arg0.count = arg1 >>> 0;
        },
        __wbg_set_cull_mode_e6ca97d167dede04: function(arg0, arg1) {
            arg0.cullMode = __wbindgen_enum_GpuCullMode[arg1];
        },
        __wbg_set_depth_bias_clamp_e8da3e9edc037b18: function(arg0, arg1) {
            arg0.depthBiasClamp = arg1;
        },
        __wbg_set_depth_bias_edf83e7d032bdd2a: function(arg0, arg1) {
            arg0.depthBias = arg1;
        },
        __wbg_set_depth_bias_slope_scale_1746553a2d960310: function(arg0, arg1) {
            arg0.depthBiasSlopeScale = arg1;
        },
        __wbg_set_depth_clear_value_f6f20e460b0c4012: function(arg0, arg1) {
            arg0.depthClearValue = arg1;
        },
        __wbg_set_depth_compare_f18e709a5dc4a0b0: function(arg0, arg1) {
            arg0.depthCompare = __wbindgen_enum_GpuCompareFunction[arg1];
        },
        __wbg_set_depth_fail_op_45f4179ddfc376cd: function(arg0, arg1) {
            arg0.depthFailOp = __wbindgen_enum_GpuStencilOperation[arg1];
        },
        __wbg_set_depth_load_op_c3d15137e916dd5a: function(arg0, arg1) {
            arg0.depthLoadOp = __wbindgen_enum_GpuLoadOp[arg1];
        },
        __wbg_set_depth_or_array_layers_17284e1eac0007a1: function(arg0, arg1) {
            arg0.depthOrArrayLayers = arg1 >>> 0;
        },
        __wbg_set_depth_read_only_9a8994459a0acf9e: function(arg0, arg1) {
            arg0.depthReadOnly = arg1 !== 0;
        },
        __wbg_set_depth_stencil_8716a627d44d796e: function(arg0, arg1) {
            arg0.depthStencil = arg1;
        },
        __wbg_set_depth_stencil_attachment_08fe1db9243adb49: function(arg0, arg1) {
            arg0.depthStencilAttachment = arg1;
        },
        __wbg_set_depth_store_op_881c8dbe0d67045d: function(arg0, arg1) {
            arg0.depthStoreOp = __wbindgen_enum_GpuStoreOp[arg1];
        },
        __wbg_set_depth_write_enabled_9b608c3bf190f04a: function(arg0, arg1) {
            arg0.depthWriteEnabled = arg1 !== 0;
        },
        __wbg_set_device_bf67f52a587f9aab: function(arg0, arg1) {
            arg0.device = arg1;
        },
        __wbg_set_dimension_3144c827eb67b110: function(arg0, arg1) {
            arg0.dimension = __wbindgen_enum_GpuTextureViewDimension[arg1];
        },
        __wbg_set_dimension_9054f7ab0acbfd31: function(arg0, arg1) {
            arg0.dimension = __wbindgen_enum_GpuTextureDimension[arg1];
        },
        __wbg_set_dst_factor_77c088c0ad43b684: function(arg0, arg1) {
            arg0.dstFactor = __wbindgen_enum_GpuBlendFactor[arg1];
        },
        __wbg_set_end_of_pass_write_index_568e7295b1b273a3: function(arg0, arg1) {
            arg0.endOfPassWriteIndex = arg1 >>> 0;
        },
        __wbg_set_entries_247b5e665db79583: function(arg0, arg1) {
            arg0.entries = arg1;
        },
        __wbg_set_entries_bfbb6a7f04b96709: function(arg0, arg1) {
            arg0.entries = arg1;
        },
        __wbg_set_entry_point_1d7ecf099e15fa2b: function(arg0, arg1, arg2) {
            arg0.entryPoint = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_entry_point_a698bc91b472d5a9: function(arg0, arg1, arg2) {
            arg0.entryPoint = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_external_texture_a0db2eb13c76e01c: function(arg0, arg1) {
            arg0.externalTexture = arg1;
        },
        __wbg_set_fail_op_a6890cff6b560a14: function(arg0, arg1) {
            arg0.failOp = __wbindgen_enum_GpuStencilOperation[arg1];
        },
        __wbg_set_format_237b3e4047d1be9a: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuTextureFormat[arg1];
        },
        __wbg_set_format_53479155b45c92ea: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuTextureFormat[arg1];
        },
        __wbg_set_format_7c20bfc7e4802e1e: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuTextureFormat[arg1];
        },
        __wbg_set_format_84af8ed546a319fa: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuVertexFormat[arg1];
        },
        __wbg_set_format_9f32450123336995: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuTextureFormat[arg1];
        },
        __wbg_set_format_c9dcecebf9c1e619: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuTextureFormat[arg1];
        },
        __wbg_set_format_d9d3420c1a9d1c59: function(arg0, arg1) {
            arg0.format = __wbindgen_enum_GpuTextureFormat[arg1];
        },
        __wbg_set_fragment_2c6f6192456fed1c: function(arg0, arg1) {
            arg0.fragment = arg1;
        },
        __wbg_set_front_face_fd34f06ac9239d11: function(arg0, arg1) {
            arg0.frontFace = __wbindgen_enum_GpuFrontFace[arg1];
        },
        __wbg_set_g_7bb456722103c5e6: function(arg0, arg1) {
            arg0.g = arg1;
        },
        __wbg_set_has_dynamic_offset_0c8a607543be8069: function(arg0, arg1) {
            arg0.hasDynamicOffset = arg1 !== 0;
        },
        __wbg_set_height_96611e96eee67c44: function(arg0, arg1) {
            arg0.height = arg1 >>> 0;
        },
        __wbg_set_height_bb0dc35fd1d941f5: function(arg0, arg1) {
            arg0.height = arg1 >>> 0;
        },
        __wbg_set_height_bdd58e6b04e88cca: function(arg0, arg1) {
            arg0.height = arg1 >>> 0;
        },
        __wbg_set_label_01228663ea03b92f: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_13a2dd9810399e8b: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_22e57f4c5b38215f: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_382417d222111912: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_4d049edb707ba31c: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_5724e5c6286e8a4e: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_8354c6463558484f: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_acd66edf00f8e40f: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_b3da7636c69f1a4c: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_b7f797c13bc822c4: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_e026aa2aea731594: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_e6bc3b86ef6deeb8: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_label_e7f76bba99d72971: function(arg0, arg1, arg2) {
            arg0.label = getStringFromWasm0(arg1, arg2);
        },
        __wbg_set_layout_210c56f1b01e883b: function(arg0, arg1) {
            arg0.layout = arg1;
        },
        __wbg_set_layout_44943c4c7d78f826: function(arg0, arg1) {
            arg0.layout = arg1;
        },
        __wbg_set_load_op_103e6ce0a06327b3: function(arg0, arg1) {
            arg0.loadOp = __wbindgen_enum_GpuLoadOp[arg1];
        },
        __wbg_set_lod_max_clamp_3706b17de6a5a11a: function(arg0, arg1) {
            arg0.lodMaxClamp = arg1;
        },
        __wbg_set_lod_min_clamp_1f4b28d77d0c87c4: function(arg0, arg1) {
            arg0.lodMinClamp = arg1;
        },
        __wbg_set_mag_filter_2c8c212ff297b151: function(arg0, arg1) {
            arg0.magFilter = __wbindgen_enum_GpuFilterMode[arg1];
        },
        __wbg_set_mapped_at_creation_12773dff1bb6ea0f: function(arg0, arg1) {
            arg0.mappedAtCreation = arg1 !== 0;
        },
        __wbg_set_mask_94311bacf22c96b1: function(arg0, arg1) {
            arg0.mask = arg1 >>> 0;
        },
        __wbg_set_max_anisotropy_56bff110d0b4830f: function(arg0, arg1) {
            arg0.maxAnisotropy = arg1;
        },
        __wbg_set_min_binding_size_164df827e02821e1: function(arg0, arg1) {
            arg0.minBindingSize = arg1;
        },
        __wbg_set_min_filter_66d70efb70601bf9: function(arg0, arg1) {
            arg0.minFilter = __wbindgen_enum_GpuFilterMode[arg1];
        },
        __wbg_set_mip_level_7f56c1c607dda5cf: function(arg0, arg1) {
            arg0.mipLevel = arg1 >>> 0;
        },
        __wbg_set_mip_level_count_3402f5315a423b69: function(arg0, arg1) {
            arg0.mipLevelCount = arg1 >>> 0;
        },
        __wbg_set_mip_level_count_ed029120a0ee12b8: function(arg0, arg1) {
            arg0.mipLevelCount = arg1 >>> 0;
        },
        __wbg_set_mipmap_filter_1b3d153bd834ec26: function(arg0, arg1) {
            arg0.mipmapFilter = __wbindgen_enum_GpuMipmapFilterMode[arg1];
        },
        __wbg_set_module_119c125107e59165: function(arg0, arg1) {
            arg0.module = arg1;
        },
        __wbg_set_module_3336e0ce2672c07c: function(arg0, arg1) {
            arg0.module = arg1;
        },
        __wbg_set_multisample_1cdb4998c4f73341: function(arg0, arg1) {
            arg0.multisample = arg1;
        },
        __wbg_set_multisampled_e9d7bed0e68d1d6f: function(arg0, arg1) {
            arg0.multisampled = arg1 !== 0;
        },
        __wbg_set_offset_01c04218a4ccfa08: function(arg0, arg1) {
            arg0.offset = arg1;
        },
        __wbg_set_offset_5ea55c758ce6f103: function(arg0, arg1) {
            arg0.offset = arg1;
        },
        __wbg_set_offset_f07a73165707eb4c: function(arg0, arg1) {
            arg0.offset = arg1;
        },
        __wbg_set_operation_845ee9de4c1808be: function(arg0, arg1) {
            arg0.operation = __wbindgen_enum_GpuBlendOperation[arg1];
        },
        __wbg_set_origin_6be40076fb0623f4: function(arg0, arg1) {
            arg0.origin = arg1;
        },
        __wbg_set_pass_op_f4a566b4a6b819e0: function(arg0, arg1) {
            arg0.passOp = __wbindgen_enum_GpuStencilOperation[arg1];
        },
        __wbg_set_power_preference_0721cf46746c0c7d: function(arg0, arg1) {
            arg0.powerPreference = __wbindgen_enum_GpuPowerPreference[arg1];
        },
        __wbg_set_primitive_94234ea756a4c469: function(arg0, arg1) {
            arg0.primitive = arg1;
        },
        __wbg_set_query_set_667d2dcfcedd1ec2: function(arg0, arg1) {
            arg0.querySet = arg1;
        },
        __wbg_set_r_13b3eba61d4ba4d9: function(arg0, arg1) {
            arg0.r = arg1;
        },
        __wbg_set_required_features_5202fa8cd082e531: function(arg0, arg1) {
            arg0.requiredFeatures = arg1;
        },
        __wbg_set_resolve_target_8251ec563ce24501: function(arg0, arg1) {
            arg0.resolveTarget = arg1;
        },
        __wbg_set_resource_c73d0c2d815f7211: function(arg0, arg1) {
            arg0.resource = arg1;
        },
        __wbg_set_rows_per_image_e6e2c0c4a7e4fa8d: function(arg0, arg1) {
            arg0.rowsPerImage = arg1 >>> 0;
        },
        __wbg_set_sample_count_05dcc9952f4fa7ac: function(arg0, arg1) {
            arg0.sampleCount = arg1 >>> 0;
        },
        __wbg_set_sample_type_93886b8f9794f85c: function(arg0, arg1) {
            arg0.sampleType = __wbindgen_enum_GpuTextureSampleType[arg1];
        },
        __wbg_set_sampler_02989e99b27a50db: function(arg0, arg1) {
            arg0.sampler = arg1;
        },
        __wbg_set_shader_location_33bfc2cc417fdde2: function(arg0, arg1) {
            arg0.shaderLocation = arg1 >>> 0;
        },
        __wbg_set_size_b3e6b2bf58d62082: function(arg0, arg1) {
            arg0.size = arg1;
        },
        __wbg_set_size_c2a556d5571231f5: function(arg0, arg1) {
            arg0.size = arg1;
        },
        __wbg_set_size_f7b29f6cb1669c4d: function(arg0, arg1) {
            arg0.size = arg1;
        },
        __wbg_set_src_factor_deefdfd4b6427b5e: function(arg0, arg1) {
            arg0.srcFactor = __wbindgen_enum_GpuBlendFactor[arg1];
        },
        __wbg_set_stencil_back_9971eece60c613eb: function(arg0, arg1) {
            arg0.stencilBack = arg1;
        },
        __wbg_set_stencil_clear_value_4b82f2d7e6d9f496: function(arg0, arg1) {
            arg0.stencilClearValue = arg1 >>> 0;
        },
        __wbg_set_stencil_front_c82ccf19db426a5b: function(arg0, arg1) {
            arg0.stencilFront = arg1;
        },
        __wbg_set_stencil_load_op_dbc3af9713146f24: function(arg0, arg1) {
            arg0.stencilLoadOp = __wbindgen_enum_GpuLoadOp[arg1];
        },
        __wbg_set_stencil_read_mask_926bce469fa8eaf3: function(arg0, arg1) {
            arg0.stencilReadMask = arg1 >>> 0;
        },
        __wbg_set_stencil_read_only_88136eaf94225fec: function(arg0, arg1) {
            arg0.stencilReadOnly = arg1 !== 0;
        },
        __wbg_set_stencil_store_op_e10e50255bf42e8b: function(arg0, arg1) {
            arg0.stencilStoreOp = __wbindgen_enum_GpuStoreOp[arg1];
        },
        __wbg_set_stencil_write_mask_3a7b75284ae036de: function(arg0, arg1) {
            arg0.stencilWriteMask = arg1 >>> 0;
        },
        __wbg_set_step_mode_2a36020da93d6f16: function(arg0, arg1) {
            arg0.stepMode = __wbindgen_enum_GpuVertexStepMode[arg1];
        },
        __wbg_set_storage_texture_50af47fec531be02: function(arg0, arg1) {
            arg0.storageTexture = arg1;
        },
        __wbg_set_store_op_23c150f1deb52749: function(arg0, arg1) {
            arg0.storeOp = __wbindgen_enum_GpuStoreOp[arg1];
        },
        __wbg_set_strip_index_format_bd4035f66052e74e: function(arg0, arg1) {
            arg0.stripIndexFormat = __wbindgen_enum_GpuIndexFormat[arg1];
        },
        __wbg_set_targets_d4d849b9c6abd35a: function(arg0, arg1) {
            arg0.targets = arg1;
        },
        __wbg_set_texture_884f2777c0fe1e91: function(arg0, arg1) {
            arg0.texture = arg1;
        },
        __wbg_set_texture_a1baf7da91d20351: function(arg0, arg1) {
            arg0.texture = arg1;
        },
        __wbg_set_timestamp_writes_fdf8168751575b7f: function(arg0, arg1) {
            arg0.timestampWrites = arg1;
        },
        __wbg_set_topology_0a07d5f484aff275: function(arg0, arg1) {
            arg0.topology = __wbindgen_enum_GpuPrimitiveTopology[arg1];
        },
        __wbg_set_type_109702a7ec65b49d: function(arg0, arg1) {
            arg0.type = __wbindgen_enum_GpuSamplerBindingType[arg1];
        },
        __wbg_set_type_55112c374bcc5a9d: function(arg0, arg1) {
            arg0.type = __wbindgen_enum_GpuBufferBindingType[arg1];
        },
        __wbg_set_unclipped_depth_b15740038a226817: function(arg0, arg1) {
            arg0.unclippedDepth = arg1 !== 0;
        },
        __wbg_set_usage_75d0e81d39ff5284: function(arg0, arg1) {
            arg0.usage = arg1 >>> 0;
        },
        __wbg_set_usage_ac04cadda4108c1a: function(arg0, arg1) {
            arg0.usage = arg1 >>> 0;
        },
        __wbg_set_usage_ad30cd3b0e0f4244: function(arg0, arg1) {
            arg0.usage = arg1 >>> 0;
        },
        __wbg_set_usage_cc34543608cf3335: function(arg0, arg1) {
            arg0.usage = arg1 >>> 0;
        },
        __wbg_set_vertex_13029c78ba2f0430: function(arg0, arg1) {
            arg0.vertex = arg1;
        },
        __wbg_set_view_6f2831c0da43be15: function(arg0, arg1) {
            arg0.view = arg1;
        },
        __wbg_set_view_b18c2cccd427261d: function(arg0, arg1) {
            arg0.view = arg1;
        },
        __wbg_set_view_dimension_50f3edb06107948f: function(arg0, arg1) {
            arg0.viewDimension = __wbindgen_enum_GpuTextureViewDimension[arg1];
        },
        __wbg_set_view_dimension_925bb358df1f2b9d: function(arg0, arg1) {
            arg0.viewDimension = __wbindgen_enum_GpuTextureViewDimension[arg1];
        },
        __wbg_set_view_formats_3d2a72ffb1f55a75: function(arg0, arg1) {
            arg0.viewFormats = arg1;
        },
        __wbg_set_view_formats_ae87e8e12aa858fc: function(arg0, arg1) {
            arg0.viewFormats = arg1;
        },
        __wbg_set_visibility_bdebc70a7c0f236d: function(arg0, arg1) {
            arg0.visibility = arg1 >>> 0;
        },
        __wbg_set_width_25112eb6bf1148df: function(arg0, arg1) {
            arg0.width = arg1 >>> 0;
        },
        __wbg_set_width_40592253da7b2113: function(arg0, arg1) {
            arg0.width = arg1 >>> 0;
        },
        __wbg_set_width_9d385df435c1f79d: function(arg0, arg1) {
            arg0.width = arg1 >>> 0;
        },
        __wbg_set_write_mask_c03e8a45a9236e80: function(arg0, arg1) {
            arg0.writeMask = arg1 >>> 0;
        },
        __wbg_set_x_3c1cb8191b848172: function(arg0, arg1) {
            arg0.x = arg1 >>> 0;
        },
        __wbg_set_y_9024331910317eff: function(arg0, arg1) {
            arg0.y = arg1 >>> 0;
        },
        __wbg_set_z_b6182f4c230116a7: function(arg0, arg1) {
            arg0.z = arg1 >>> 0;
        },
        __wbg_stack_3b0d974bbf31e44f: function(arg0, arg1) {
            const ret = arg1.stack;
            const ptr1 = passStringToWasm0(ret, wasm.__wbindgen_malloc, wasm.__wbindgen_realloc);
            const len1 = WASM_VECTOR_LEN;
            getDataViewMemory0().setInt32(arg0 + 4 * 1, len1, true);
            getDataViewMemory0().setInt32(arg0 + 4 * 0, ptr1, true);
        },
        __wbg_static_accessor_GLOBAL_THIS_02344c9b09eb08a9: function() {
            const ret = typeof globalThis === 'undefined' ? null : globalThis;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_GLOBAL_ac6d4ac874d5cd54: function() {
            const ret = typeof global === 'undefined' ? null : global;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_SELF_9b2406c23aeb2023: function() {
            const ret = typeof self === 'undefined' ? null : self;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_static_accessor_WINDOW_b34d2126934e16ba: function() {
            const ret = typeof window === 'undefined' ? null : window;
            return isLikeNone(ret) ? 0 : addToExternrefTable0(ret);
        },
        __wbg_submit_fc5b9a1154201a58: function(arg0, arg1) {
            arg0.submit(arg1);
        },
        __wbg_then_837494e384b37459: function(arg0, arg1) {
            const ret = arg0.then(arg1);
            return ret;
        },
        __wbg_then_bd927500e8905df2: function(arg0, arg1, arg2) {
            const ret = arg0.then(arg1, arg2);
            return ret;
        },
        __wbg_unmap_50b3be4aaf23fa39: function(arg0) {
            arg0.unmap();
        },
        __wbg_viewer_new: function(arg0) {
            const ret = Viewer.__wrap(arg0);
            return ret;
        },
        __wbg_warn_c4e0780980765a86: function(arg0) {
            console.warn(arg0);
        },
        __wbg_width_e987166926c3367c: function(arg0) {
            const ret = arg0.width;
            return ret;
        },
        __wbg_writeBuffer_7d54524c36f1c7e2: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4, arg5) {
            arg0.writeBuffer(arg1, arg2, arg3, arg4, arg5);
        }, arguments); },
        __wbg_writeTexture_9d4c493be748d189: function() { return handleError(function (arg0, arg1, arg2, arg3, arg4) {
            arg0.writeTexture(arg1, arg2, arg3, arg4);
        }, arguments); },
        __wbindgen_cast_0000000000000001: function(arg0, arg1) {
            // Cast intrinsic for `Closure(Closure { owned: true, function: Function { arguments: [Externref], shim_idx: 141, ret: Result(Unit), inner_ret: Some(Result(Unit)) }, mutable: true }) -> Externref`.
            const ret = makeMutClosure(arg0, arg1, wasm_bindgen__convert__closures_____invoke__hcbb11b929283bdfe);
            return ret;
        },
        __wbindgen_cast_0000000000000002: function(arg0) {
            // Cast intrinsic for `F64 -> Externref`.
            const ret = arg0;
            return ret;
        },
        __wbindgen_cast_0000000000000003: function(arg0, arg1) {
            // Cast intrinsic for `Ref(Slice(U8)) -> NamedExternref("Uint8Array")`.
            const ret = getArrayU8FromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_cast_0000000000000004: function(arg0, arg1) {
            // Cast intrinsic for `Ref(String) -> Externref`.
            const ret = getStringFromWasm0(arg0, arg1);
            return ret;
        },
        __wbindgen_init_externref_table: function() {
            const table = wasm.__wbindgen_externrefs;
            const offset = table.grow(4);
            table.set(0, undefined);
            table.set(offset + 0, undefined);
            table.set(offset + 1, null);
            table.set(offset + 2, true);
            table.set(offset + 3, false);
        },
    };
    return {
        __proto__: null,
        "./bovista_bg.js": import0,
    };
}

function wasm_bindgen__convert__closures_____invoke__hcbb11b929283bdfe(arg0, arg1, arg2) {
    const ret = wasm.wasm_bindgen__convert__closures_____invoke__hcbb11b929283bdfe(arg0, arg1, arg2);
    if (ret[1]) {
        throw takeFromExternrefTable0(ret[0]);
    }
}

function wasm_bindgen__convert__closures_____invoke__h61ab42adbe47db91(arg0, arg1, arg2, arg3) {
    wasm.wasm_bindgen__convert__closures_____invoke__h61ab42adbe47db91(arg0, arg1, arg2, arg3);
}


const __wbindgen_enum_GpuAddressMode = ["clamp-to-edge", "repeat", "mirror-repeat"];


const __wbindgen_enum_GpuBlendFactor = ["zero", "one", "src", "one-minus-src", "src-alpha", "one-minus-src-alpha", "dst", "one-minus-dst", "dst-alpha", "one-minus-dst-alpha", "src-alpha-saturated", "constant", "one-minus-constant", "src1", "one-minus-src1", "src1-alpha", "one-minus-src1-alpha"];


const __wbindgen_enum_GpuBlendOperation = ["add", "subtract", "reverse-subtract", "min", "max"];


const __wbindgen_enum_GpuBufferBindingType = ["uniform", "storage", "read-only-storage"];


const __wbindgen_enum_GpuCanvasAlphaMode = ["opaque", "premultiplied"];


const __wbindgen_enum_GpuCompareFunction = ["never", "less", "equal", "less-equal", "greater", "not-equal", "greater-equal", "always"];


const __wbindgen_enum_GpuCullMode = ["none", "front", "back"];


const __wbindgen_enum_GpuFilterMode = ["nearest", "linear"];


const __wbindgen_enum_GpuFrontFace = ["ccw", "cw"];


const __wbindgen_enum_GpuIndexFormat = ["uint16", "uint32"];


const __wbindgen_enum_GpuLoadOp = ["load", "clear"];


const __wbindgen_enum_GpuMipmapFilterMode = ["nearest", "linear"];


const __wbindgen_enum_GpuPowerPreference = ["low-power", "high-performance"];


const __wbindgen_enum_GpuPrimitiveTopology = ["point-list", "line-list", "line-strip", "triangle-list", "triangle-strip"];


const __wbindgen_enum_GpuSamplerBindingType = ["filtering", "non-filtering", "comparison"];


const __wbindgen_enum_GpuStencilOperation = ["keep", "zero", "replace", "invert", "increment-clamp", "decrement-clamp", "increment-wrap", "decrement-wrap"];


const __wbindgen_enum_GpuStorageTextureAccess = ["write-only", "read-only", "read-write"];


const __wbindgen_enum_GpuStoreOp = ["store", "discard"];


const __wbindgen_enum_GpuTextureAspect = ["all", "stencil-only", "depth-only"];


const __wbindgen_enum_GpuTextureDimension = ["1d", "2d", "3d"];


const __wbindgen_enum_GpuTextureFormat = ["r8unorm", "r8snorm", "r8uint", "r8sint", "r16uint", "r16sint", "r16float", "rg8unorm", "rg8snorm", "rg8uint", "rg8sint", "r32uint", "r32sint", "r32float", "rg16uint", "rg16sint", "rg16float", "rgba8unorm", "rgba8unorm-srgb", "rgba8snorm", "rgba8uint", "rgba8sint", "bgra8unorm", "bgra8unorm-srgb", "rgb9e5ufloat", "rgb10a2uint", "rgb10a2unorm", "rg11b10ufloat", "rg32uint", "rg32sint", "rg32float", "rgba16uint", "rgba16sint", "rgba16float", "rgba32uint", "rgba32sint", "rgba32float", "stencil8", "depth16unorm", "depth24plus", "depth24plus-stencil8", "depth32float", "depth32float-stencil8", "bc1-rgba-unorm", "bc1-rgba-unorm-srgb", "bc2-rgba-unorm", "bc2-rgba-unorm-srgb", "bc3-rgba-unorm", "bc3-rgba-unorm-srgb", "bc4-r-unorm", "bc4-r-snorm", "bc5-rg-unorm", "bc5-rg-snorm", "bc6h-rgb-ufloat", "bc6h-rgb-float", "bc7-rgba-unorm", "bc7-rgba-unorm-srgb", "etc2-rgb8unorm", "etc2-rgb8unorm-srgb", "etc2-rgb8a1unorm", "etc2-rgb8a1unorm-srgb", "etc2-rgba8unorm", "etc2-rgba8unorm-srgb", "eac-r11unorm", "eac-r11snorm", "eac-rg11unorm", "eac-rg11snorm", "astc-4x4-unorm", "astc-4x4-unorm-srgb", "astc-5x4-unorm", "astc-5x4-unorm-srgb", "astc-5x5-unorm", "astc-5x5-unorm-srgb", "astc-6x5-unorm", "astc-6x5-unorm-srgb", "astc-6x6-unorm", "astc-6x6-unorm-srgb", "astc-8x5-unorm", "astc-8x5-unorm-srgb", "astc-8x6-unorm", "astc-8x6-unorm-srgb", "astc-8x8-unorm", "astc-8x8-unorm-srgb", "astc-10x5-unorm", "astc-10x5-unorm-srgb", "astc-10x6-unorm", "astc-10x6-unorm-srgb", "astc-10x8-unorm", "astc-10x8-unorm-srgb", "astc-10x10-unorm", "astc-10x10-unorm-srgb", "astc-12x10-unorm", "astc-12x10-unorm-srgb", "astc-12x12-unorm", "astc-12x12-unorm-srgb"];


const __wbindgen_enum_GpuTextureSampleType = ["float", "unfilterable-float", "depth", "sint", "uint"];


const __wbindgen_enum_GpuTextureViewDimension = ["1d", "2d", "2d-array", "cube", "cube-array", "3d"];


const __wbindgen_enum_GpuVertexFormat = ["uint8", "uint8x2", "uint8x4", "sint8", "sint8x2", "sint8x4", "unorm8", "unorm8x2", "unorm8x4", "snorm8", "snorm8x2", "snorm8x4", "uint16", "uint16x2", "uint16x4", "sint16", "sint16x2", "sint16x4", "unorm16", "unorm16x2", "unorm16x4", "snorm16", "snorm16x2", "snorm16x4", "float16", "float16x2", "float16x4", "float32", "float32x2", "float32x3", "float32x4", "uint32", "uint32x2", "uint32x3", "uint32x4", "sint32", "sint32x2", "sint32x3", "sint32x4", "unorm10-10-10-2", "unorm8x4-bgra"];


const __wbindgen_enum_GpuVertexStepMode = ["vertex", "instance"];
const AverageVolumeFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_averagevolume_free(ptr, 1));
const DirectVolumeFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_directvolume_free(ptr, 1));
const ImageFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_image_free(ptr, 1));
const IsosurfaceVolumeFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_isosurfacevolume_free(ptr, 1));
const LevelMetadataFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_levelmetadata_free(ptr, 1));
const LinesFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_lines_free(ptr, 1));
const MinipVolumeFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_minipvolume_free(ptr, 1));
const MipVolumeFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_mipvolume_free(ptr, 1));
const PointsFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_points_free(ptr, 1));
const ViewerFinalization = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(ptr => wasm.__wbg_viewer_free(ptr, 1));

function addToExternrefTable0(obj) {
    const idx = wasm.__externref_table_alloc();
    wasm.__wbindgen_externrefs.set(idx, obj);
    return idx;
}

function _assertClass(instance, klass) {
    if (!(instance instanceof klass)) {
        throw new Error(`expected instance of ${klass.name}`);
    }
}

const CLOSURE_DTORS = (typeof FinalizationRegistry === 'undefined')
    ? { register: () => {}, unregister: () => {} }
    : new FinalizationRegistry(state => wasm.__wbindgen_destroy_closure(state.a, state.b));

function debugString(val) {
    // primitive types
    const type = typeof val;
    if (type == 'number' || type == 'boolean' || val == null) {
        return  `${val}`;
    }
    if (type == 'string') {
        return `"${val}"`;
    }
    if (type == 'symbol') {
        const description = val.description;
        if (description == null) {
            return 'Symbol';
        } else {
            return `Symbol(${description})`;
        }
    }
    if (type == 'function') {
        const name = val.name;
        if (typeof name == 'string' && name.length > 0) {
            return `Function(${name})`;
        } else {
            return 'Function';
        }
    }
    // objects
    if (Array.isArray(val)) {
        const length = val.length;
        let debug = '[';
        if (length > 0) {
            debug += debugString(val[0]);
        }
        for(let i = 1; i < length; i++) {
            debug += ', ' + debugString(val[i]);
        }
        debug += ']';
        return debug;
    }
    // Test for built-in
    const builtInMatches = /\[object ([^\]]+)\]/.exec(toString.call(val));
    let className;
    if (builtInMatches && builtInMatches.length > 1) {
        className = builtInMatches[1];
    } else {
        // Failed to match the standard '[object ClassName]'
        return toString.call(val);
    }
    if (className == 'Object') {
        // we're a user defined class or Object
        // JSON.stringify avoids problems with cycles, and is generally much
        // easier than looping through ownProperties of `val`.
        try {
            return 'Object(' + JSON.stringify(val) + ')';
        } catch (_) {
            return 'Object';
        }
    }
    // errors
    if (val instanceof Error) {
        return `${val.name}: ${val.message}\n${val.stack}`;
    }
    // TODO we could test for more things here, like `Set`s and `Map`s.
    return className;
}

function getArrayF32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getFloat32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU16FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint16ArrayMemory0().subarray(ptr / 2, ptr / 2 + len);
}

function getArrayU32FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint32ArrayMemory0().subarray(ptr / 4, ptr / 4 + len);
}

function getArrayU8FromWasm0(ptr, len) {
    ptr = ptr >>> 0;
    return getUint8ArrayMemory0().subarray(ptr / 1, ptr / 1 + len);
}

let cachedDataViewMemory0 = null;
function getDataViewMemory0() {
    if (cachedDataViewMemory0 === null || cachedDataViewMemory0.buffer.detached === true || (cachedDataViewMemory0.buffer.detached === undefined && cachedDataViewMemory0.buffer !== wasm.memory.buffer)) {
        cachedDataViewMemory0 = new DataView(wasm.memory.buffer);
    }
    return cachedDataViewMemory0;
}

let cachedFloat32ArrayMemory0 = null;
function getFloat32ArrayMemory0() {
    if (cachedFloat32ArrayMemory0 === null || cachedFloat32ArrayMemory0.byteLength === 0) {
        cachedFloat32ArrayMemory0 = new Float32Array(wasm.memory.buffer);
    }
    return cachedFloat32ArrayMemory0;
}

function getStringFromWasm0(ptr, len) {
    return decodeText(ptr >>> 0, len);
}

let cachedUint16ArrayMemory0 = null;
function getUint16ArrayMemory0() {
    if (cachedUint16ArrayMemory0 === null || cachedUint16ArrayMemory0.byteLength === 0) {
        cachedUint16ArrayMemory0 = new Uint16Array(wasm.memory.buffer);
    }
    return cachedUint16ArrayMemory0;
}

let cachedUint32ArrayMemory0 = null;
function getUint32ArrayMemory0() {
    if (cachedUint32ArrayMemory0 === null || cachedUint32ArrayMemory0.byteLength === 0) {
        cachedUint32ArrayMemory0 = new Uint32Array(wasm.memory.buffer);
    }
    return cachedUint32ArrayMemory0;
}

let cachedUint8ArrayMemory0 = null;
function getUint8ArrayMemory0() {
    if (cachedUint8ArrayMemory0 === null || cachedUint8ArrayMemory0.byteLength === 0) {
        cachedUint8ArrayMemory0 = new Uint8Array(wasm.memory.buffer);
    }
    return cachedUint8ArrayMemory0;
}

function handleError(f, args) {
    try {
        return f.apply(this, args);
    } catch (e) {
        const idx = addToExternrefTable0(e);
        wasm.__wbindgen_exn_store(idx);
    }
}

function isLikeNone(x) {
    return x === undefined || x === null;
}

function makeMutClosure(arg0, arg1, f) {
    const state = { a: arg0, b: arg1, cnt: 1 };
    const real = (...args) => {

        // First up with a closure we increment the internal reference
        // count. This ensures that the Rust closure environment won't
        // be deallocated while we're invoking it.
        state.cnt++;
        const a = state.a;
        state.a = 0;
        try {
            return f(a, state.b, ...args);
        } finally {
            state.a = a;
            real._wbg_cb_unref();
        }
    };
    real._wbg_cb_unref = () => {
        if (--state.cnt === 0) {
            wasm.__wbindgen_destroy_closure(state.a, state.b);
            state.a = 0;
            CLOSURE_DTORS.unregister(state);
        }
    };
    CLOSURE_DTORS.register(real, state, state);
    return real;
}

function passArray32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getUint32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayF32ToWasm0(arg, malloc) {
    const ptr = malloc(arg.length * 4, 4) >>> 0;
    getFloat32ArrayMemory0().set(arg, ptr / 4);
    WASM_VECTOR_LEN = arg.length;
    return ptr;
}

function passArrayJsValueToWasm0(array, malloc) {
    const ptr = malloc(array.length * 4, 4) >>> 0;
    for (let i = 0; i < array.length; i++) {
        const add = addToExternrefTable0(array[i]);
        getDataViewMemory0().setUint32(ptr + 4 * i, add, true);
    }
    WASM_VECTOR_LEN = array.length;
    return ptr;
}

function passStringToWasm0(arg, malloc, realloc) {
    if (realloc === undefined) {
        const buf = cachedTextEncoder.encode(arg);
        const ptr = malloc(buf.length, 1) >>> 0;
        getUint8ArrayMemory0().subarray(ptr, ptr + buf.length).set(buf);
        WASM_VECTOR_LEN = buf.length;
        return ptr;
    }

    let len = arg.length;
    let ptr = malloc(len, 1) >>> 0;

    const mem = getUint8ArrayMemory0();

    let offset = 0;

    for (; offset < len; offset++) {
        const code = arg.charCodeAt(offset);
        if (code > 0x7F) break;
        mem[ptr + offset] = code;
    }
    if (offset !== len) {
        if (offset !== 0) {
            arg = arg.slice(offset);
        }
        ptr = realloc(ptr, len, len = offset + arg.length * 3, 1) >>> 0;
        const view = getUint8ArrayMemory0().subarray(ptr + offset, ptr + len);
        const ret = cachedTextEncoder.encodeInto(arg, view);

        offset += ret.written;
        ptr = realloc(ptr, len, offset, 1) >>> 0;
    }

    WASM_VECTOR_LEN = offset;
    return ptr;
}

function takeFromExternrefTable0(idx) {
    const value = wasm.__wbindgen_externrefs.get(idx);
    wasm.__externref_table_dealloc(idx);
    return value;
}

let cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
cachedTextDecoder.decode();
const MAX_SAFARI_DECODE_BYTES = 2146435072;
let numBytesDecoded = 0;
function decodeText(ptr, len) {
    numBytesDecoded += len;
    if (numBytesDecoded >= MAX_SAFARI_DECODE_BYTES) {
        cachedTextDecoder = new TextDecoder('utf-8', { ignoreBOM: true, fatal: true });
        cachedTextDecoder.decode();
        numBytesDecoded = len;
    }
    return cachedTextDecoder.decode(getUint8ArrayMemory0().subarray(ptr, ptr + len));
}

const cachedTextEncoder = new TextEncoder();

if (!('encodeInto' in cachedTextEncoder)) {
    cachedTextEncoder.encodeInto = function (arg, view) {
        const buf = cachedTextEncoder.encode(arg);
        view.set(buf);
        return {
            read: arg.length,
            written: buf.length
        };
    };
}

let WASM_VECTOR_LEN = 0;

let wasmModule, wasmInstance, wasm;
function __wbg_finalize_init(instance, module) {
    wasmInstance = instance;
    wasm = instance.exports;
    wasmModule = module;
    cachedDataViewMemory0 = null;
    cachedFloat32ArrayMemory0 = null;
    cachedUint16ArrayMemory0 = null;
    cachedUint32ArrayMemory0 = null;
    cachedUint8ArrayMemory0 = null;
    wasm.__wbindgen_start();
    return wasm;
}

async function __wbg_load(module, imports) {
    if (typeof Response === 'function' && module instanceof Response) {
        if (typeof WebAssembly.instantiateStreaming === 'function') {
            try {
                return await WebAssembly.instantiateStreaming(module, imports);
            } catch (e) {
                const validResponse = module.ok && expectedResponseType(module.type);

                if (validResponse && module.headers.get('Content-Type') !== 'application/wasm') {
                    console.warn("`WebAssembly.instantiateStreaming` failed because your server does not serve Wasm with `application/wasm` MIME type. Falling back to `WebAssembly.instantiate` which is slower. Original error:\n", e);

                } else { throw e; }
            }
        }

        const bytes = await module.arrayBuffer();
        return await WebAssembly.instantiate(bytes, imports);
    } else {
        const instance = await WebAssembly.instantiate(module, imports);

        if (instance instanceof WebAssembly.Instance) {
            return { instance, module };
        } else {
            return instance;
        }
    }

    function expectedResponseType(type) {
        switch (type) {
            case 'basic': case 'cors': case 'default': return true;
        }
        return false;
    }
}

function initSync(module) {
    if (wasm !== undefined) return wasm;


    if (module !== undefined) {
        if (Object.getPrototypeOf(module) === Object.prototype) {
            ({module} = module)
        } else {
            console.warn('using deprecated parameters for `initSync()`; pass a single object instead')
        }
    }

    const imports = __wbg_get_imports();
    if (!(module instanceof WebAssembly.Module)) {
        module = new WebAssembly.Module(module);
    }
    const instance = new WebAssembly.Instance(module, imports);
    return __wbg_finalize_init(instance, module);
}

async function __wbg_init(module_or_path) {
    if (wasm !== undefined) return wasm;


    if (module_or_path !== undefined) {
        if (Object.getPrototypeOf(module_or_path) === Object.prototype) {
            ({module_or_path} = module_or_path)
        } else {
            console.warn('using deprecated parameters for the initialization function; pass a single object instead')
        }
    }

    if (module_or_path === undefined) {
        module_or_path = new URL('bovista_bg.wasm', import.meta.url);
    }
    const imports = __wbg_get_imports();

    if (typeof module_or_path === 'string' || (typeof Request === 'function' && module_or_path instanceof Request) || (typeof URL === 'function' && module_or_path instanceof URL)) {
        module_or_path = fetch(module_or_path);
    }

    const { instance, module } = await __wbg_load(await module_or_path, imports);

    return __wbg_finalize_init(instance, module);
}

export { initSync, __wbg_init as default };
