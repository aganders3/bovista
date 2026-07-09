#!/usr/bin/env python3
"""
Synthetic Label-Volume Viewer

Demonstrates bovista.LabelVolume — 3D segmentation rendered as a first-hit
isosurface colored per label (napari's 3D-labels behavior). Each label's outer
shell gets a distinct flat color, shaded via its binary-mask surface normal;
background (ID 0) is empty. Same synthetic volume + in-memory loader as
labels_demo.py, but orbited as a 3D volume instead of sliced.

Controls:  Left-drag: orbit   Scroll: zoom   "Shuffle colors" recolors.
"""

import sys
import numpy as np
from threading import Lock, Event, Thread

from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                             QHBoxLayout, QPushButton, QLabel, QSizePolicy)
from PyQt6.QtCore import Qt, QTimer

import bovista as bv
from synthetic import make_label_volume, build_lod_pyramid


class ViewerWidget(QWidget):
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setMinimumSize(320, 240)
        self.setSizePolicy(QSizePolicy.Policy.Expanding, QSizePolicy.Policy.Expanding)
        self.viewer = bv.Viewer(800, 600)
        self._initialized = False
        self._setup_callbacks = []

        self.setMouseTracking(True)
        self.left_pressed = False
        self.last_x = 0
        self.last_y = 0

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
        self.last_x = event.position().x()
        self.last_y = event.position().y()

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            self.left_pressed = False

    def mouseMoveEvent(self, event):
        if self.left_pressed and self._initialized:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y
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
        self.setWindowTitle("Synthetic Label-Volume Viewer")
        self.setGeometry(100, 100, 1024, 768)

        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)

        self.viewer_widget = ViewerWidget()
        layout.addWidget(self.viewer_widget, 1)

        controls = QHBoxLayout()
        self.shuffle_button = QPushButton("Shuffle colors")
        self.shuffle_button.clicked.connect(self.shuffle_colors)
        controls.addWidget(self.shuffle_button)
        controls.addStretch()
        layout.addLayout(controls)

        layout.addWidget(QLabel("Left-drag: Orbit | Scroll: Zoom"))
        self.status_label = QLabel("Generating labels…")
        layout.addWidget(self.status_label)

        self.labels = None
        self._seed = 0.0
        self.loader_stop = None

        QTimer.singleShot(0, self.build_scene)

    def build_scene(self):
        print("Generating synthetic label volume…")
        vol = make_label_volume(size=256, n_labels=18)
        pyramid = build_lod_pyramid(vol, n_lods=4)
        n_ids = int(vol.max())
        chunk = 64

        lod_levels = []
        for i, arr in enumerate(pyramid):
            lod_levels.append(bv.LevelMetadata(
                volume_size=arr.shape,
                chunk_size=(chunk, chunk, chunk),
                voxel_size=(2.0 ** i, 2.0 ** i, 2.0 ** i),
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
            labels = bv.LabelVolume(viewer, lod_levels, 500)
            start_loader(labels)

            lod0 = lod_levels[0]
            world = tuple(s * v for s, v in zip(lod0.volume_size, lod0.voxel_size))
            center = [world[2] / 2, world[1] / 2, world[0] / 2]
            max_dim = max(world)

            viewer.add(labels)
            self.labels = labels

            viewer.set_camera_clip_planes(max_dim * 0.001, max_dim * 100)
            viewer.set_camera_position(center[0], center[1], center[2] + max_dim * 1.8)
            viewer.set_camera_target(*center)

            viewer.add(bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3))
            self.status_label.setText(
                f"{n_ids} labels as a first-hit isosurface, colored per ID. "
                f"Left-drag to orbit; Shuffle to recolor.")
            print("✓ Ready")

        self.viewer_widget.on_ready(setup_scene)

    def shuffle_colors(self):
        if self.labels:
            self._seed += 1.0
            self.labels.set_label_seed(self._seed)


def main():
    print("=" * 60)
    print("Synthetic Label-Volume Viewer — bovista.LabelVolume")
    print("=" * 60)
    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
