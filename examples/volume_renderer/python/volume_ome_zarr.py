#!/usr/bin/env python3
"""
OME-Zarr Volume Renderer

Demonstrates direct volume rendering (DVR) of multi-resolution OME-Zarr
datasets with bovista.DirectVolume.

bovista also exposes the other render modes as separate classes — swap
`bv.DirectVolume(...)` below for any of:

    bv.DirectVolume      — front-to-back alpha compositing (default).
    bv.AdditiveVolume    — additive accumulation, no occlusion.
    bv.MipVolume         — Maximum Intensity Projection (set_attenuation>0
                           for napari-style attenuated MIP).
    bv.MinipVolume       — Minimum Intensity Projection.
    bv.AverageVolume     — mean intensity along the ray.
    bv.IsosurfaceVolume  — first-hit isosurface with Phong shading
                           (set_iso_threshold).
"""

import sys
import math
import numpy as np
from concurrent.futures import ThreadPoolExecutor
from threading import Lock
import threading
from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                             QHBoxLayout, QPushButton, QLabel, QSlider, QComboBox)
from PyQt6.QtCore import Qt, QTimer

import bovista as bv
import zarr


class ViewerWidget(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setMinimumSize(800, 600)
        self.viewer = bv.Viewer(800, 600)
        self._initialized = False
        self._setup_callbacks = []
        self._volume = None

        self.setMouseTracking(True)
        self.left_pressed = False
        self.last_x = 0
        self.last_y = 0

        self.volume_scale = 1.0

        self.timer = QTimer(self)
        self.timer.timeout.connect(self._render)
        self.timer.start(16)

    def showEvent(self, event):
        super().showEvent(event)
        if not self._initialized:
            try:
                handle = int(self.winId())
                size = self.size()
                self.viewer.initialize_with_window(handle, size.width(), size.height())
                self._initialized = True
                for cb in self._setup_callbacks:
                    cb(self.viewer)
                self._setup_callbacks.clear()
            except Exception as e:
                print(f"Init error: {e}")

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self._initialized:
            size = event.size()
            self.viewer.resize(size.width(), size.height())

    def _render(self):
        if self._initialized:
            try:
                self.viewer.render_frame()
            except Exception as e:
                print(f"Render error: {e}")

    def mousePressEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            self.left_pressed = True
        self.last_x = event.position().x()
        self.last_y = event.position().y()

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            self.left_pressed = False

    def mouseMoveEvent(self, event):
        dx = event.position().x() - self.last_x
        dy = event.position().y() - self.last_y
        if self.left_pressed and self._initialized:
            self.viewer.orbit_camera(dx * 0.005, dy * 0.005)
        self.last_x = event.position().x()
        self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            self.viewer.zoom_camera(-event.angleDelta().y())

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("OME-Zarr Volume Renderer")
        self.setGeometry(100, 100, 1024, 820)

        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)

        self.datasets = {
            "Beechnut - CT scan": ("https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/beechnut.ome.zarr", 0.06),
            "Pawpawsaurus - CT scan fossil": ("https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/pawpawsaurus.ome.zarr", 700.0),
            "Marmoset Neurons - 3D microscopy": ("https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/marmoset_neurons.ome.zarr", 400.0),
            "Platybrowser - Embryo, 10 LODs": ("https://s3.embl.de/i2k-2020/platy-raw.ome.zarr", 300.0),
        }

        # Dataset row
        ds_row = QHBoxLayout()
        ds_row.addWidget(QLabel("Dataset:"))
        self.dataset_combo = QComboBox()
        for name in self.datasets:
            self.dataset_combo.addItem(name)
        ds_row.addWidget(self.dataset_combo)
        self.load_button = QPushButton("Load")
        self.load_button.clicked.connect(self.load_zarr)
        ds_row.addWidget(self.load_button)
        layout.addLayout(ds_row)

        # Viewer
        self.viewer_widget = ViewerWidget()
        layout.addWidget(self.viewer_widget)

        # Row 1: LOD bias, density, step, debug
        ctrl1 = QHBoxLayout()

        ctrl1.addWidget(QLabel("LOD Bias:"))
        self.lod_slider = QSlider(Qt.Orientation.Horizontal)
        self.lod_slider.setRange(-50, 50)
        self.lod_slider.setValue(0)
        self.lod_slider.setFixedWidth(100)
        self.lod_slider.valueChanged.connect(self._update_lod_bias)
        ctrl1.addWidget(self.lod_slider)
        self.lod_label = QLabel("0.0")
        self.lod_label.setFixedWidth(32)
        ctrl1.addWidget(self.lod_label)

        ctrl1.addWidget(QLabel("  Density:"))
        self.density_slider = QSlider(Qt.Orientation.Horizontal)
        self.density_slider.setRange(-20, 20)
        self.density_slider.setValue(0)
        self.density_slider.setFixedWidth(100)
        self.density_slider.valueChanged.connect(self._update_density)
        ctrl1.addWidget(self.density_slider)
        self.density_label = QLabel("1.00×")
        self.density_label.setFixedWidth(44)
        ctrl1.addWidget(self.density_label)

        ctrl1.addWidget(QLabel("  Step:"))
        self.step_slider = QSlider(Qt.Orientation.Horizontal)
        self.step_slider.setRange(2, 30)   # 0.2 – 3.0 voxels, ×0.1
        self.step_slider.setValue(10)       # default 1.0
        self.step_slider.setFixedWidth(80)
        self.step_slider.valueChanged.connect(self._update_step)
        ctrl1.addWidget(self.step_slider)
        self.step_label = QLabel("1.0")
        self.step_label.setFixedWidth(28)
        ctrl1.addWidget(self.step_label)

        ctrl1.addStretch()

        self.debug_combo = QComboBox()
        self.debug_combo.addItems(["Debug: Off", "Debug: LOD tint", "Debug: Atlas", "Debug: Steps"])
        self.debug_combo.currentIndexChanged.connect(self._update_debug)
        ctrl1.addWidget(self.debug_combo)

        layout.addLayout(ctrl1)

        # Row 2: contrast floor / ceiling / auto
        ctrl2 = QHBoxLayout()

        ctrl2.addWidget(QLabel("Floor:"))
        self.floor_slider = QSlider(Qt.Orientation.Horizontal)
        self.floor_slider.setRange(0, 1000)
        self.floor_slider.setValue(0)
        self.floor_slider.setFixedWidth(120)
        self.floor_slider.valueChanged.connect(self._update_contrast)
        ctrl2.addWidget(self.floor_slider)
        self.floor_label = QLabel("0.000")
        self.floor_label.setFixedWidth(40)
        ctrl2.addWidget(self.floor_label)

        ctrl2.addWidget(QLabel("  Ceiling:"))
        self.ceiling_slider = QSlider(Qt.Orientation.Horizontal)
        self.ceiling_slider.setRange(0, 1000)
        self.ceiling_slider.setValue(1000)
        self.ceiling_slider.setFixedWidth(120)
        self.ceiling_slider.valueChanged.connect(self._update_contrast)
        ctrl2.addWidget(self.ceiling_slider)
        self.ceiling_label = QLabel("1.000")
        self.ceiling_label.setFixedWidth(40)
        ctrl2.addWidget(self.ceiling_label)

        self.auto_button = QPushButton("Auto")
        self.auto_button.setFixedWidth(50)
        self.auto_button.clicked.connect(self._auto_contrast)
        ctrl2.addWidget(self.auto_button)

        ctrl2.addStretch()
        layout.addLayout(ctrl2)

        layout.addWidget(QLabel("Left-drag: Orbit  |  Scroll: Zoom"))

        self.status_label = QLabel("Select dataset and click Load")
        layout.addWidget(self.status_label)

        self.stats_timer = QTimer()
        self.stats_timer.timeout.connect(self._update_stats)
        self.stats_timer.start(100)

        self.executor = None
        self.volume = None
        self.level_info = []
        self._base_density = 1.0
        self._density_exp = 0

        for w in [self.lod_slider, self.density_slider, self.step_slider, self.debug_combo,
                  self.floor_slider, self.ceiling_slider, self.auto_button]:
            w.setEnabled(False)

    # ── loaders ───────────────────────────────────────────────────────────────

    def load_zarr(self):
        try:
            dataset_name = self.dataset_combo.currentText()
            url, camera_distance = self.datasets[dataset_name]
            print(f"\n{'='*60}\nLoading: {url}\n{'='*60}")
            self.status_label.setText("Loading...")
            self.load_button.setEnabled(False)

            try:
                store = zarr.open_group(url, mode="r", zarr_format=3)
            except Exception:
                try:
                    store = zarr.open_group(url, mode="r", zarr_format=2)
                except Exception:
                    store = zarr.open_group(url, mode="r")

            attrs = dict(store.attrs)
            if "ome" in attrs:
                multiscales = attrs["ome"]["multiscales"][0]
            else:
                multiscales = attrs.get("multiscales", [{}])[0]
            datasets = multiscales.get("datasets", [])
            axes = multiscales.get("axes", [])
            axis_names = [ax.get("name", "?") for ax in axes]

            if not datasets:
                path = "0" if "0" in store else (list(store.keys())[0] if store else "")
                datasets = [{"path": path}]
                axis_names = ["z", "y", "x"]

            print(f"Found {len(datasets)} LOD levels, axes: {axis_names}")

            spatial_map = {name: i for i, name in enumerate(axis_names) if name in ("z", "y", "x")}
            if len(spatial_map) != 3:
                ndim = len(store[datasets[0]["path"]].shape)
                spatial_map = {"z": ndim - 3, "y": ndim - 2, "x": ndim - 1}
            z_idx, y_idx, x_idx = spatial_map["z"], spatial_map["y"], spatial_map["x"]

            lod_levels = []
            lod0_spacing = None
            for i, ds_info in enumerate(datasets):
                path = ds_info.get("path", str(i))
                arr = store[path]
                volume_shape = (arr.shape[z_idx], arr.shape[y_idx], arr.shape[x_idx])
                chunk_shape  = (arr.chunks[z_idx] if arr.chunks else 64,
                                arr.chunks[y_idx] if arr.chunks else 64,
                                arr.chunks[x_idx] if arr.chunks else 64)
                transforms = ds_info.get("coordinateTransformations", [])
                scale_t = next((t for t in transforms if t.get("type") == "scale"), None)
                voxel_spacing = tuple(scale_t["scale"][i] for i in [z_idx, y_idx, x_idx]) if scale_t else (1.0, 1.0, 1.0)
                if i == 0:
                    lod0_spacing = voxel_spacing
                    scale_factor = 1.0
                else:
                    scale_factor = max(voxel_spacing[k] / lod0_spacing[k] for k in range(3))

                level = bv.LevelMetadata(
                    volume_size=volume_shape,
                    chunk_size=chunk_shape,
                    voxel_size=voxel_spacing,
                    scale_factor=scale_factor,
                    translation=(0.0, 0.0, 0.0),
                )
                lod_levels.append(level)
                print(f"  LOD {i}: {volume_shape} @ {scale_factor:.1f}×")

            if not lod_levels:
                raise ValueError("No LOD levels found in dataset!")

            self.level_info = lod_levels

            if self.executor:
                self.executor.shutdown(wait=False)
            self.executor = ThreadPoolExecutor(max_workers=8)

            def load_chunk(lod, t, z, y, x):
                try:
                    path = datasets[lod].get("path", str(lod))
                    arr = store[path]
                    level = lod_levels[lod]
                    cz, cy, cx = level.chunk_size
                    slices = [0] * len(arr.shape)
                    slices[z_idx] = slice(z * cz, min((z + 1) * cz, level.volume_size[0]))
                    slices[y_idx] = slice(y * cy, min((y + 1) * cy, level.volume_size[1]))
                    slices[x_idx] = slice(x * cx, min((x + 1) * cx, level.volume_size[2]))
                    # This example doesn't expose t to the UI — t is always 0.
                    data = arr[tuple(slices)]
                    if data.dtype == np.uint8:
                        return np.ascontiguousarray(data)
                    elif data.dtype == np.uint16:
                        return np.ascontiguousarray(data)
                    elif np.issubdtype(data.dtype, np.integer):
                        info = np.iinfo(data.dtype)
                        data = ((data.astype(np.float32) - info.min) /
                                (info.max - info.min) * 65535).astype(np.uint16)
                        return np.ascontiguousarray(data)
                    else:
                        lo, hi = data.min(), data.max()
                        if hi > lo:
                            data = ((data - lo) / (hi - lo) * 65535).astype(np.uint16)
                        else:
                            data = np.zeros_like(data, dtype=np.uint16)
                        return np.ascontiguousarray(data)
                except Exception as e:
                    print(f"Error loading chunk LOD{lod} t={t} ({z},{y},{x}): {e}")
                    return None

            # Pull-based loader. A background thread polls
            # `volume.wanted_keys()` and dispatches fetches via the
            # ThreadPoolExecutor; results are pushed via
            # `set_chunk_data_u16`. Cancellation is implicit.
            def start_loader_thread(volume):
                in_flight = set()
                lock = Lock()
                stop = self.loader_stop = threading.Event()

                def on_loaded(lod, t, z, y, x, future):
                    with lock:
                        in_flight.discard((lod, t, z, y, x))
                    if future.cancelled():
                        return
                    data = future.result()
                    if data is None:
                        return
                    if data.dtype == np.uint8:
                        data = (data.astype(np.uint16) * 257)
                    volume.set_chunk_data_u16(lod, t, z, y, x, data)

                def poll():
                    while not stop.is_set():
                        try:
                            wanted = volume.wanted_keys()
                        except Exception:
                            return
                        for lod, t, z, y, x, _pri in wanted:
                            key = (lod, t, z, y, x)
                            with lock:
                                if key in in_flight or len(in_flight) >= 8:
                                    continue
                                in_flight.add(key)
                            future = self.executor.submit(load_chunk, lod, t, z, y, x)
                            future.add_done_callback(
                                lambda f, k=key: on_loaded(*k, f))
                        stop.wait(0.02)

                threading.Thread(target=poll, daemon=True).start()

            def setup_scene(viewer):
                volume = bv.DirectVolume(viewer, lod_levels, 2000)
                start_loader_thread(volume)

                lod0 = lod_levels[0]
                world_size = tuple(s * v for s, v in zip(lod0.volume_size, lod0.voxel_size))
                center = [world_size[2] / 2, world_size[1] / 2, world_size[0] / 2]
                max_dim = max(world_size)

                self.floor_slider.setValue(0)
                self.ceiling_slider.setValue(1000)
                volume.set_contrast(0.0, 1.0)

                # Auto density scale: ~0.5× traversal opacity over max dimension
                base_density = 0.5 * max(lod0.volume_size) / max(max_dim, 1e-9)
                self._base_density = base_density
                self._density_exp = 0
                self.density_slider.setValue(0)
                volume.set_density_scale(base_density)

                viewer.add(volume)
                self.viewer_widget._volume = volume
                self.viewer_widget.volume_scale = max_dim
                self.volume = volume

                near = max_dim * 0.001
                far  = max_dim * 100
                viewer.set_camera_clip_planes(near, far)

                viewer.set_camera_position(center[0], center[1], center[2] + camera_distance)
                viewer.set_camera_target(*center)

                viewer.add(bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3))

                for w in [self.lod_slider, self.density_slider, self.step_slider, self.debug_combo,
                          self.floor_slider, self.ceiling_slider, self.auto_button]:
                    w.setEnabled(True)
                self.lod_slider.setValue(0)
                self.step_slider.setValue(10)
                self.debug_combo.setCurrentIndex(0)
                self.load_button.setEnabled(True)
                self.status_label.setText("Loaded! Orbit: left-drag  |  Contrast: middle-drag  |  Zoom: scroll")
                print("✓ Ready")

            self.viewer_widget.on_ready(setup_scene)

        except Exception as e:
            self.status_label.setText(f"Error: {e}")
            print(f"ERROR: {e}")
            import traceback; traceback.print_exc()
            self.load_button.setEnabled(True)

    # ── control slots ─────────────────────────────────────────────────────────

    def _update_lod_bias(self):
        if self.volume:
            bias = self.lod_slider.value() / 10.0
            self.lod_label.setText(f"{bias:.1f}")
            self.volume.set_lod_bias(bias)

    def _update_density(self):
        if not self.volume:
            return
        self._density_exp = self.density_slider.value()
        multiplier = math.pow(10, self._density_exp / 10)
        self.volume.set_density_scale(self._base_density * multiplier)
        label = f"{multiplier:.2f}×" if multiplier < 10 else f"{multiplier:.1f}×"
        self.density_label.setText(label)

    def _update_step(self):
        if self.volume:
            step = self.step_slider.value() / 10.0
            self.step_label.setText(f"{step:.1f}")
            self.volume.set_relative_step_size(step)

    def _update_contrast(self):
        if not self.volume:
            return
        floor   = self.floor_slider.value() / 1000.0
        ceiling = self.ceiling_slider.value() / 1000.0
        # Keep floor < ceiling
        if floor >= ceiling:
            if self.sender() is self.floor_slider:
                floor = max(0.0, ceiling - 0.001)
                self.floor_slider.blockSignals(True)
                self.floor_slider.setValue(int(floor * 1000))
                self.floor_slider.blockSignals(False)
            else:
                ceiling = min(1.0, floor + 0.001)
                self.ceiling_slider.blockSignals(True)
                self.ceiling_slider.setValue(int(ceiling * 1000))
                self.ceiling_slider.blockSignals(False)
        self.floor_label.setText(f"{floor:.3f}")
        self.ceiling_label.setText(f"{ceiling:.3f}")
        self.volume.set_contrast(floor, ceiling)

    def _auto_contrast(self):
        self.floor_slider.setValue(0)
        self.ceiling_slider.setValue(1000)
        self._update_contrast()

    def _update_debug(self):
        if not self.volume:
            return
        mode = self.debug_combo.currentIndex()
        self.volume.set_debug_mode(mode == 1)
        self.volume.set_atlas_debug_mode(mode == 2)
        self.volume.set_step_debug_mode(mode == 3)

    def _update_stats(self):
        if self.volume:
            try:
                loaded, visible = self.volume.get_stats()
                self.status_label.setText(
                    f"{len(self.level_info)} LODs  |  {loaded} tiles loaded, {visible} visible"
                )
            except Exception:
                pass


def main():
    print("=" * 60)
    print("OME-Zarr Volume Renderer")
    print("=" * 60)
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
