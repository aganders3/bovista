#!/usr/bin/env python3
"""
OME-Zarr Volume Renderer

Demonstrates all of bovista's volume rendering algorithms (DirectVolume,
AdditiveVolume, MipVolume, MinipVolume, AverageVolume, IsosurfaceVolume) on
multi-resolution OME-Zarr datasets. Switch between modes from the Mode
dropdown; mode-specific controls activate as appropriate.
"""

import sys
import math
import numpy as np
from concurrent.futures import ThreadPoolExecutor
from threading import Lock
from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                             QHBoxLayout, QPushButton, QLabel, QSlider, QComboBox)
from PyQt6.QtCore import Qt, QTimer

import bovista as bv
import zarr


# Mode-name → (ctor, supports_density, supports_attenuation, supports_iso, supports_debug)
VOLUME_MODES = {
    "Direct (translucent)": (bv.DirectVolume, True,  False, False, True),
    "Additive":             (bv.AdditiveVolume, True,  False, False, False),
    "MIP":                  (bv.MipVolume,      False, True,  False, False),
    "minIP":                (bv.MinipVolume,    False, False, False, False),
    "Average":              (bv.AverageVolume,  False, False, False, False),
    "Isosurface":           (bv.IsosurfaceVolume, False, False, True, False),
}


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
        self.setGeometry(100, 100, 1024, 860)

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

        ds_row.addWidget(QLabel("  Mode:"))
        self.mode_combo = QComboBox()
        for name in VOLUME_MODES:
            self.mode_combo.addItem(name)
        self.mode_combo.currentIndexChanged.connect(self._switch_mode)
        ds_row.addWidget(self.mode_combo)
        layout.addLayout(ds_row)

        # Viewer
        self.viewer_widget = ViewerWidget()
        layout.addWidget(self.viewer_widget)

        # Row 1: LOD bias, step, contrast floor/ceiling
        ctrl1 = QHBoxLayout()

        ctrl1.addWidget(QLabel("LOD Bias:"))
        self.lod_slider = QSlider(Qt.Orientation.Horizontal)
        self.lod_slider.setRange(-50, 50)
        self.lod_slider.setValue(0)
        self.lod_slider.setFixedWidth(100)
        self.lod_slider.valueChanged.connect(self._update_lod_bias)
        ctrl1.addWidget(self.lod_slider)
        self.lod_label = QLabel("0.0"); self.lod_label.setFixedWidth(32)
        ctrl1.addWidget(self.lod_label)

        ctrl1.addWidget(QLabel("  Step:"))
        self.step_slider = QSlider(Qt.Orientation.Horizontal)
        self.step_slider.setRange(2, 30)
        self.step_slider.setValue(10)
        self.step_slider.setFixedWidth(80)
        self.step_slider.valueChanged.connect(self._update_step)
        ctrl1.addWidget(self.step_slider)
        self.step_label = QLabel("1.0"); self.step_label.setFixedWidth(28)
        ctrl1.addWidget(self.step_label)

        ctrl1.addWidget(QLabel("  Floor:"))
        self.floor_slider = QSlider(Qt.Orientation.Horizontal)
        self.floor_slider.setRange(0, 1000); self.floor_slider.setValue(0)
        self.floor_slider.setFixedWidth(120)
        self.floor_slider.valueChanged.connect(self._update_contrast)
        ctrl1.addWidget(self.floor_slider)
        self.floor_label = QLabel("0.000"); self.floor_label.setFixedWidth(40)
        ctrl1.addWidget(self.floor_label)

        ctrl1.addWidget(QLabel("  Ceiling:"))
        self.ceiling_slider = QSlider(Qt.Orientation.Horizontal)
        self.ceiling_slider.setRange(0, 1000); self.ceiling_slider.setValue(1000)
        self.ceiling_slider.setFixedWidth(120)
        self.ceiling_slider.valueChanged.connect(self._update_contrast)
        ctrl1.addWidget(self.ceiling_slider)
        self.ceiling_label = QLabel("1.000"); self.ceiling_label.setFixedWidth(40)
        ctrl1.addWidget(self.ceiling_label)

        self.auto_button = QPushButton("Auto"); self.auto_button.setFixedWidth(50)
        self.auto_button.clicked.connect(self._auto_contrast)
        ctrl1.addWidget(self.auto_button)

        ctrl1.addStretch()
        layout.addLayout(ctrl1)

        # Row 2: mode-specific parameters (only relevant ones are enabled at any time)
        ctrl2 = QHBoxLayout()

        ctrl2.addWidget(QLabel("Density:"))
        self.density_slider = QSlider(Qt.Orientation.Horizontal)
        self.density_slider.setRange(-20, 20); self.density_slider.setValue(0)
        self.density_slider.setFixedWidth(120)
        self.density_slider.valueChanged.connect(self._update_density)
        ctrl2.addWidget(self.density_slider)
        self.density_label = QLabel("1.00×"); self.density_label.setFixedWidth(50)
        ctrl2.addWidget(self.density_label)

        ctrl2.addWidget(QLabel("  Iso:"))
        self.iso_slider = QSlider(Qt.Orientation.Horizontal)
        self.iso_slider.setRange(0, 1000); self.iso_slider.setValue(500)
        self.iso_slider.setFixedWidth(120)
        self.iso_slider.valueChanged.connect(self._update_iso)
        ctrl2.addWidget(self.iso_slider)
        self.iso_label = QLabel("0.500"); self.iso_label.setFixedWidth(40)
        ctrl2.addWidget(self.iso_label)

        ctrl2.addWidget(QLabel("  Attenuation:"))
        self.att_slider = QSlider(Qt.Orientation.Horizontal)
        # Attenuation is in units of 1/(accumulated normalised density). For
        # sparse data (lots of air), sumval at exit is small, so high attenuation
        # values are needed to fully suppress the far side. 0 = plain MIP.
        self.att_slider.setRange(0, 1000); self.att_slider.setValue(0)
        self.att_slider.setFixedWidth(120)
        self.att_slider.valueChanged.connect(self._update_attenuation)
        ctrl2.addWidget(self.att_slider)
        self.att_label = QLabel("0.0"); self.att_label.setFixedWidth(40)
        ctrl2.addWidget(self.att_label)

        ctrl2.addStretch()

        self.debug_combo = QComboBox()
        self.debug_combo.addItems(["Debug: Off", "Debug: LOD tint", "Debug: Atlas", "Debug: Steps"])
        self.debug_combo.currentIndexChanged.connect(self._update_debug)
        ctrl2.addWidget(self.debug_combo)

        layout.addLayout(ctrl2)

        layout.addWidget(QLabel("Left-drag: Orbit  |  Scroll: Zoom"))
        self.status_label = QLabel("Select dataset and click Load")
        layout.addWidget(self.status_label)

        self.stats_timer = QTimer()
        self.stats_timer.timeout.connect(self._update_stats)
        self.stats_timer.start(100)

        # State.
        self.executor = None
        self.volume = None
        self.volume_index = None       # scene index of the current volume
        self.loader = None              # bound chunk loader (re-pointed on mode switch)
        self.lod_levels = []
        self._base_density = 1.0

        # Disable all controls until dataset loaded.
        for w in [self.lod_slider, self.step_slider, self.floor_slider, self.ceiling_slider,
                  self.auto_button, self.density_slider, self.iso_slider, self.att_slider,
                  self.debug_combo, self.mode_combo]:
            w.setEnabled(False)

    def _current_mode_caps(self):
        name = self.mode_combo.currentText()
        return VOLUME_MODES[name]

    def _apply_mode_caps(self):
        """Enable only the mode-specific sliders that apply to the current type."""
        _, density, attenuation, iso, debug = self._current_mode_caps()
        self.density_slider.setEnabled(density)
        self.att_slider.setEnabled(attenuation)
        self.iso_slider.setEnabled(iso)
        self.debug_combo.setEnabled(debug)
        if not debug:
            self.debug_combo.setCurrentIndex(0)

    # ── loaders ──────────────────────────────────────────────────────────────

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

            self.lod_levels = lod_levels

            if self.executor:
                self.executor.shutdown(wait=False)
            self.executor = ThreadPoolExecutor(max_workers=8)

            def create_loader():
                # The active volume is stored mutably so the loader can be re-pointed
                # at a freshly-constructed visual whenever the user switches modes.
                volume = None
                pending = {}
                lock = Lock()

                def load_chunk(lod, z, y, x):
                    try:
                        path = datasets[lod].get("path", str(lod))
                        arr = store[path]
                        level = lod_levels[lod]
                        cz, cy, cx = level.chunk_size
                        slices = [0] * len(arr.shape)
                        slices[z_idx] = slice(z * cz, min((z + 1) * cz, level.volume_size[0]))
                        slices[y_idx] = slice(y * cy, min((y + 1) * cy, level.volume_size[1]))
                        slices[x_idx] = slice(x * cx, min((x + 1) * cx, level.volume_size[2]))
                        data = arr[tuple(slices)]
                        if data.dtype == np.uint8:
                            return np.ascontiguousarray(data)
                        elif data.dtype == np.uint16:
                            return np.ascontiguousarray(data)
                        elif np.issubdtype(data.dtype, np.integer):
                            info = np.iinfo(data.dtype)
                            data = ((data.astype(np.float32) - info.min) / (info.max - info.min) * 65535).astype(np.uint16)
                            return np.ascontiguousarray(data)
                        else:
                            lo, hi = data.min(), data.max()
                            if hi > lo:
                                data = ((data - lo) / (hi - lo) * 65535).astype(np.uint16)
                            else:
                                data = np.zeros_like(data, dtype=np.uint16)
                            return np.ascontiguousarray(data)
                    except Exception as e:
                        print(f"Error loading chunk LOD{lod} ({z},{y},{x}): {e}")
                        return None

                def on_loaded(lod, z, y, x, future):
                    with lock:
                        pending.pop((lod, z, y, x), None)
                    if not future.cancelled() and volume:
                        data = future.result()
                        if data is not None:
                            if data.dtype == np.uint8:
                                data = (data.astype(np.uint16) * 257)
                            volume.set_chunk_data_u16(lod, z, y, x, data)

                def request(lod, z, y, x):
                    key = (lod, z, y, x)
                    with lock:
                        if key in pending:
                            return bv.ChunkStatus.AlreadyPending
                        if len(pending) >= 8:
                            return bv.ChunkStatus.Rejected
                        future = self.executor.submit(load_chunk, lod, z, y, x)
                        future.add_done_callback(lambda f: on_loaded(lod, z, y, x, f))
                        pending[key] = future
                    return bv.ChunkStatus.Accepted

                def set_volume(vol):
                    nonlocal volume
                    volume = vol

                request.set_volume = set_volume
                return request

            self.loader = create_loader()

            def setup_scene(viewer):
                lod0 = lod_levels[0]
                world_size = tuple(s * v for s, v in zip(lod0.volume_size, lod0.voxel_size))
                center = [world_size[2] / 2, world_size[1] / 2, world_size[0] / 2]
                max_dim = max(world_size)

                # Auto density scale baseline (used for DirectVolume / AdditiveVolume).
                self._base_density = 0.5 * max(lod0.volume_size) / max(max_dim, 1e-9)
                self.viewer_widget.volume_scale = max_dim

                near = max_dim * 0.001
                far  = max_dim * 100
                viewer.set_camera_clip_planes(near, far)
                viewer.set_camera_position(center[0], center[1], center[2] + camera_distance)
                viewer.set_camera_target(*center)
                viewer.add(bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3))

                # Build the initial volume of whatever mode is selected.
                self._build_current_volume(viewer)

                for w in [self.lod_slider, self.step_slider, self.floor_slider, self.ceiling_slider,
                          self.auto_button, self.mode_combo]:
                    w.setEnabled(True)
                self._apply_mode_caps()

                self.lod_slider.setValue(0)
                self.step_slider.setValue(10)
                self.debug_combo.setCurrentIndex(0)
                self.load_button.setEnabled(True)
                self.status_label.setText("Loaded! Switch Mode to compare rendering algorithms.")
                print("✓ Ready")

            self.viewer_widget.on_ready(setup_scene)

        except Exception as e:
            self.status_label.setText(f"Error: {e}")
            print(f"ERROR: {e}")
            import traceback; traceback.print_exc()
            self.load_button.setEnabled(True)

    # ── volume (re)construction ──────────────────────────────────────────────

    def _build_current_volume(self, viewer):
        """Construct a fresh volume of the currently-selected type, replacing any prior."""
        ctor, supports_density, _, _, _ = self._current_mode_caps()

        # Replace the previous volume in the scene, if any.
        # Bovista's scene doesn't currently expose a remove-by-index, so we rebuild
        # by clearing everything but the axis helper. For the example this is fine —
        # in production code you'd want a stable remove API.
        if self.volume_index is not None:
            # No remove API yet: easiest is to clear the entire scene and re-add axes
            # alongside the new volume. The viewer.add_lines / clear flow handles it.
            viewer.clear_visuals()
            lod0 = self.lod_levels[0]
            viewer.add(bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3))

        new_volume = ctor(viewer, self.lod_levels, 2000, self.loader)
        self.loader.set_volume(new_volume)

        # Apply common defaults.
        new_volume.set_contrast(self.floor_slider.value() / 1000.0, self.ceiling_slider.value() / 1000.0)
        new_volume.set_relative_step_size(self.step_slider.value() / 10.0)
        new_volume.set_lod_bias(self.lod_slider.value() / 10.0)

        # Apply mode-specific defaults.
        if supports_density and hasattr(new_volume, 'set_density_scale'):
            multiplier = math.pow(10, self.density_slider.value() / 10)
            new_volume.set_density_scale(self._base_density * multiplier)
        if hasattr(new_volume, 'set_iso_threshold'):
            new_volume.set_iso_threshold(self.iso_slider.value() / 1000.0)
        if hasattr(new_volume, 'set_attenuation'):
            new_volume.set_attenuation(self.att_slider.value() / 100.0)

        self.volume_index = viewer.add(new_volume)
        self.volume = new_volume
        self.viewer_widget._volume = new_volume

    def _switch_mode(self):
        if not self.viewer_widget._initialized or not self.lod_levels:
            return
        self._build_current_volume(self.viewer_widget.viewer)
        self._apply_mode_caps()

    # ── control slots ────────────────────────────────────────────────────────

    def _update_lod_bias(self):
        if self.volume:
            bias = self.lod_slider.value() / 10.0
            self.lod_label.setText(f"{bias:.1f}")
            self.volume.set_lod_bias(bias)

    def _update_density(self):
        if not self.volume or not hasattr(self.volume, 'set_density_scale'):
            return
        multiplier = math.pow(10, self.density_slider.value() / 10)
        self.volume.set_density_scale(self._base_density * multiplier)
        label = f"{multiplier:.2f}×" if multiplier < 10 else f"{multiplier:.1f}×"
        self.density_label.setText(label)

    def _update_step(self):
        if self.volume:
            step = self.step_slider.value() / 10.0
            self.step_label.setText(f"{step:.1f}")
            self.volume.set_relative_step_size(step)

    def _update_iso(self):
        if not self.volume or not hasattr(self.volume, 'set_iso_threshold'):
            return
        threshold = self.iso_slider.value() / 1000.0
        self.iso_label.setText(f"{threshold:.3f}")
        self.volume.set_iso_threshold(threshold)

    def _update_attenuation(self):
        if not self.volume or not hasattr(self.volume, 'set_attenuation'):
            return
        # Slider 0-1000 → 0.0-100.0.
        attenuation = self.att_slider.value() / 10.0
        self.att_label.setText(f"{attenuation:.1f}")
        self.volume.set_attenuation(attenuation)

    def _update_contrast(self):
        if not self.volume:
            return
        floor   = self.floor_slider.value() / 1000.0
        ceiling = self.ceiling_slider.value() / 1000.0
        if floor >= ceiling:
            if self.sender() is self.floor_slider:
                floor = max(0.0, ceiling - 0.001)
                self.floor_slider.blockSignals(True); self.floor_slider.setValue(int(floor * 1000)); self.floor_slider.blockSignals(False)
            else:
                ceiling = min(1.0, floor + 0.001)
                self.ceiling_slider.blockSignals(True); self.ceiling_slider.setValue(int(ceiling * 1000)); self.ceiling_slider.blockSignals(False)
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
        if hasattr(self.volume, 'set_debug_mode'):
            self.volume.set_debug_mode(mode == 1)
        if hasattr(self.volume, 'set_atlas_debug_mode'):
            self.volume.set_atlas_debug_mode(mode == 2)
        if hasattr(self.volume, 'set_step_debug_mode'):
            self.volume.set_step_debug_mode(mode == 3)

    def _update_stats(self):
        if self.volume:
            try:
                loaded, visible = self.volume.get_stats()
                self.status_label.setText(
                    f"{len(self.lod_levels)} LODs  |  {loaded} tiles, {visible} visible  |  {self.mode_combo.currentText()}"
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
