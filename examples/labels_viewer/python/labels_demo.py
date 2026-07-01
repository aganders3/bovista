#!/usr/bin/env python3
"""
Synthetic Labels Viewer

Demonstrates bovista.Labels — a 2D segmentation-mask visual. A synthetic
multi-LOD label volume (spheres/cubes with distinct integer IDs on a 0
background) is streamed through the same virtual-texture path as Image, but
the shader renders each ID as a flat hashed color (ID 0 = transparent) and the
data is packed by magnitude (integer IDs, not normalized).

No network needed — the volume is generated in memory. Coarser LODs are built
by strided (nearest) downsampling, never mean, so label IDs are never blended.

Controls:
  Left-drag: orbit   Right-drag: slice   Shift+Right: rotate slice   Scroll: zoom
  "Shuffle colors" recolors the labels; the slider moves the slice.
"""

import sys
import numpy as np
from threading import Lock, Event, Thread

from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                             QHBoxLayout, QPushButton, QLabel, QSlider, QSizePolicy)
from PyQt6.QtCore import Qt, QTimer

import bovista as bv


# ── Synthetic data ──────────────────────────────────────────────────────────

def make_label_volume(size=256, n_labels=18, seed=0):
    """A `size`^3 uint16 volume: `n_labels` spheres with distinct IDs 1..N on a
    0 background. Overlaps resolve to the higher ID (arbitrary but stable)."""
    rng = np.random.default_rng(seed)
    vol = np.zeros((size, size, size), dtype=np.uint16)
    zz, yy, xx = np.mgrid[0:size, 0:size, 0:size]
    for label_id in range(1, n_labels + 1):
        cz, cy, cx = rng.integers(size * 0.2, size * 0.8, size=3)
        radius = rng.integers(size * 0.06, size * 0.16)
        inside = (zz - cz) ** 2 + (yy - cy) ** 2 + (xx - cx) ** 2 <= radius ** 2
        vol[inside] = label_id
    return vol


def build_lod_pyramid(vol, n_lods=4):
    """Strided (nearest) downsampling by 2× per level — never averages, so IDs
    stay exact. Returns a list of uint16 arrays, finest first."""
    levels = [vol]
    for _ in range(1, n_lods):
        levels.append(np.ascontiguousarray(levels[-1][::2, ::2, ::2]))
    return levels


# ── Viewer widget (orbit/slice/zoom, trimmed from the OME-Zarr example) ──────

class ViewerWidget(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setMinimumSize(320, 240)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.viewer = bv.Viewer(800, 600)
        self._initialized = False
        self._setup_callbacks = []
        self._labels = None

        self.setMouseTracking(True)
        self.left_pressed = False
        self.right_pressed = False
        self.last_x = 0
        self.last_y = 0

        self.slice_angle_x = 0.0
        self.slice_angle_y = 0.0
        self.slice_offset = 0.0
        self.volume_center = [0, 0, 0]
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
                for callback in self._setup_callbacks:
                    callback(self.viewer)
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
        elif event.button() == Qt.MouseButton.RightButton:
            self.right_pressed = True
        self.last_x = event.position().x()
        self.last_y = event.position().y()

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            self.left_pressed = False
        elif event.button() == Qt.MouseButton.RightButton:
            self.right_pressed = False

    def mouseMoveEvent(self, event):
        if not self._initialized:
            return
        dx = event.position().x() - self.last_x
        dy = event.position().y() - self.last_y
        if self.left_pressed:
            if self.viewer.get_camera_projection_mode() == bv.ProjectionMode.Orthographic:
                pan_speed = self.volume_scale * 0.002
                self.viewer.pan_camera(-dx * pan_speed, dy * pan_speed)
            else:
                self.viewer.orbit_camera(dx * 0.005, dy * 0.005)
        elif self.right_pressed and self._labels:
            modifiers = QApplication.keyboardModifiers()
            if modifiers & Qt.KeyboardModifier.ShiftModifier:
                self.slice_angle_x += dy * 0.01
            else:
                self.slice_angle_y += dy * 0.01
            self.slice_offset += dx * self.volume_scale * 0.005
            self._update_slice_plane()
        self.last_x = event.position().x()
        self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            self.viewer.zoom_camera(-event.angleDelta().y())

    def _update_slice_plane(self):
        if not self._labels:
            return
        nx, ny, nz = self._labels.set_slice_from_angles(
            *self.volume_center, self.slice_angle_x, self.slice_angle_y, self.slice_offset
        )
        if self.viewer.get_camera_projection_mode() == bv.ProjectionMode.Orthographic:
            self.viewer.align_camera_to_slice(*self.volume_center, nx, ny, nz, self.volume_scale * 2.0)

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("Synthetic Labels Viewer")
        self.setGeometry(100, 100, 1024, 768)

        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)

        self.viewer_widget = ViewerWidget()
        layout.addWidget(self.viewer_widget, 1)

        controls = QHBoxLayout()
        controls.addWidget(QLabel("Slice Z:"))
        self.slice_slider = QSlider(Qt.Orientation.Horizontal)
        self.slice_slider.setRange(0, 100)
        self.slice_slider.setValue(50)
        self.slice_slider.valueChanged.connect(self.update_slice)
        controls.addWidget(self.slice_slider)
        self.shuffle_button = QPushButton("Shuffle colors")
        self.shuffle_button.clicked.connect(self.shuffle_colors)
        controls.addWidget(self.shuffle_button)
        layout.addLayout(controls)

        layout.addWidget(QLabel(
            "Left-drag: Orbit | Right-drag: Slice | Shift+Right: Rotate | Scroll: Zoom"))
        self.status_label = QLabel("Generating labels…")
        layout.addWidget(self.status_label)

        self.labels = None
        self.world_depth = 1.0
        self._seed = 0.0
        self.loader_stop = None

        QTimer.singleShot(0, self.build_scene)

    def build_scene(self):
        size = 256
        print("Generating synthetic label volume…")
        vol = make_label_volume(size=size, n_labels=18)
        pyramid = build_lod_pyramid(vol, n_lods=4)
        n_ids = int(vol.max())
        chunk = 64

        lod_levels = []
        for i, arr in enumerate(pyramid):
            lod_levels.append(bv.LevelMetadata(
                volume_size=arr.shape,
                chunk_size=(chunk, chunk, chunk),
                voxel_size=(2.0 ** i, 2.0 ** i, 2.0 ** i),  # coarser LODs cover same world
                scale_factor=float(2 ** i),
                translation=(0.0, 0.0, 0.0),
            ))
            print(f"  LOD {i}: {arr.shape}  (ids ≤ {int(arr.max())})")

        def start_loader(labels):
            stop = self.loader_stop = Event()
            in_flight = set()
            lock = Lock()

            def poll():
                while not stop.is_set():
                    try:
                        wanted = labels.wanted_keys()
                    except Exception:
                        return
                    for lod, t, z, y, x, _pri in wanted:
                        key = (lod, t, z, y, x)
                        with lock:
                            if key in in_flight:
                                continue
                            in_flight.add(key)
                        arr = pyramid[lod]
                        cz, cy, cx = chunk, chunk, chunk
                        sz = arr[z * cz:min((z + 1) * cz, arr.shape[0]),
                                 y * cy:min((y + 1) * cy, arr.shape[1]),
                                 x * cx:min((x + 1) * cx, arr.shape[2])]
                        labels.set_label_data_u16(lod, t, z, y, x, np.ascontiguousarray(sz))
                    stop.wait(0.02)

            Thread(target=poll, daemon=True).start()

        def setup_scene(viewer):
            labels = bv.Labels(viewer, lod_levels, 500)
            start_loader(labels)

            lod0 = lod_levels[0]
            world = tuple(s * v for s, v in zip(lod0.volume_size, lod0.voxel_size))
            center = [world[2] / 2, world[1] / 2, world[0] / 2]
            self.world_depth = world[0]

            labels.set_slice_plane(center[0], center[1], center[2], 0.0, 0.0, 1.0)

            viewer.add(labels)
            self.viewer_widget._labels = labels
            self.viewer_widget.volume_center = center
            self.viewer_widget.volume_scale = max(world)
            self.labels = labels

            max_dim = max(world)
            viewer.set_camera_clip_planes(max_dim * 0.001, max_dim * 100)
            viewer.set_camera_position(center[0], center[1], center[2] + max_dim * 1.6)
            viewer.set_camera_target(*center)

            viewer.add(bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3))
            self.status_label.setText(
                f"{n_ids} labels streamed as {len(lod_levels)} LODs. "
                f"ID 0 is transparent; each ID gets a flat hashed color.")
            print("✓ Ready")

        self.viewer_widget.on_ready(setup_scene)

    def update_slice(self):
        if self.labels:
            z = self.slice_slider.value() / 100.0 * self.world_depth
            self.labels.set_slice_z(z)

    def shuffle_colors(self):
        if self.labels:
            self._seed += 1.0
            self.labels.set_label_seed(self._seed)


def main():
    print("=" * 60)
    print("Synthetic Labels Viewer — bovista.Labels")
    print("=" * 60)
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
