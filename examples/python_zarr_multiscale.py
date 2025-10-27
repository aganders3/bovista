"""
Bovista + OME-Zarr Multiscale Example

This example demonstrates:
1. Creating a multiscale OME-Zarr pyramid using Gaussian downsampling
2. Loading and switching between resolution levels
3. Interactive viewing with camera and slice controls
"""

import sys
import numpy as np
import zarr
from pathlib import Path
from PySide6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                               QLabel, QPushButton, QHBoxLayout, QComboBox)
from PySide6.QtCore import QTimer, Qt
import bovista as bv
from scipy.ndimage import zoom


def create_multiscale_zarr(path, shape=(256, 256, 256), num_levels=4):
    """
    Create a multiscale OME-Zarr dataset with Gaussian pyramid.
    """
    print(f"Creating multiscale OME-Zarr at {path}...")

    # Create zarr group
    store = zarr.DirectoryStore(path)
    root = zarr.group(store=store, overwrite=True)

    depth, height, width = shape

    # Generate full-resolution volume
    print(f"Generating full resolution: {depth}x{height}x{width}")
    cx_base, cy_base, cz_base = width / 2.0, height / 2.0, depth / 2.0
    max_dist = width / 2.0

    full_volume = np.zeros((depth, height, width), dtype=np.uint8)

    for z in range(depth):
        for y in range(height):
            for x in range(width):
                cx, cy, cz = x - cx_base, y - cy_base, z - cz_base
                dist = np.sqrt(cx*cx + cy*cy + cz*cz)

                sphere_value = int((1.0 - min(dist / max_dist, 1.0)) * 180.0)
                core_value = 255 if dist < 20.0 else 0
                shell1 = 200 if abs(dist - 40.0) < 5.0 else 0
                shell2 = 220 if abs(dist - 70.0) < 3.0 else 0

                full_volume[z, y, x] = max(sphere_value, core_value, shell1, shell2)

        if (z + 1) % 32 == 0:
            print(f"  Generated {z + 1}/{depth} slices")

    # Create multiscale pyramid
    datasets_metadata = []

    for level in range(num_levels):
        scale_factor = 2 ** level

        if level == 0:
            # Full resolution
            level_volume = full_volume
            d, h, w = depth, height, width
        else:
            # Downsample using scipy zoom (faster than skimage for this use case)
            print(f"Creating level {level} (downsampled {scale_factor}x)...")
            zoom_factor = 1.0 / scale_factor
            level_volume = zoom(full_volume, zoom_factor, order=1).astype(np.uint8)
            d, h, w = level_volume.shape

        print(f"  Storing level {level}: {d}x{h}x{w}")

        # Determine chunk size
        chunk_size = max(16, min(64, min(d, h, w) // 4))

        # Store in Zarr
        data = root.create_dataset(
            str(level),
            shape=(1, 1, d, h, w),  # (t, c, z, y, x)
            chunks=(1, 1, chunk_size, chunk_size, chunk_size),
            dtype='uint8',
            compressor=None
        )
        data[0, 0, :, :, :] = level_volume

        datasets_metadata.append({
            'path': str(level),
            'coordinateTransformations': [{
                'type': 'scale',
                'scale': [1.0, 1.0, float(scale_factor), float(scale_factor), float(scale_factor)]
            }]
        })

    # OME-Zarr metadata
    root.attrs['multiscales'] = [{
        'version': '0.4',
        'name': 'sample',
        'axes': [
            {'name': 't', 'type': 'time'},
            {'name': 'c', 'type': 'channel'},
            {'name': 'z', 'type': 'space', 'unit': 'micrometer'},
            {'name': 'y', 'type': 'space', 'unit': 'micrometer'},
            {'name': 'x', 'type': 'space', 'unit': 'micrometer'},
        ],
        'datasets': datasets_metadata
    }]

    print("Multiscale OME-Zarr created!")
    return root


class BovistaWidget(QWidget):
    """Qt widget embedding Bovista viewer"""

    def __init__(self, parent=None, width=800, height=600):
        super().__init__(parent)

        self.setMinimumSize(width, height)
        self.viewer = bv.PyViewer(width, height)
        self._initialized = False
        self._setup_callbacks = []

        # Mouse tracking
        self.setMouseTracking(True)
        self.left_mouse_pressed = False
        self.right_mouse_pressed = False
        self.last_x = 0
        self.last_y = 0

        # Slice control
        self.slice_angle = 0.0
        self.slice_offset = 0.0
        self.image_visual = None

        # Render timer
        self.timer = QTimer(self)
        self.timer.timeout.connect(self._render_loop)
        self.timer.start(16)

    def showEvent(self, event):
        super().showEvent(event)
        if not self._initialized:
            try:
                handle = int(self.winId())
                size = self.size()
                self.viewer.initialize_with_window(handle, size.width(), size.height())
                self._initialized = True
                print("Viewer initialized")

                for callback in self._setup_callbacks:
                    callback(self.viewer)
                self._setup_callbacks.clear()
            except Exception as e:
                print(f"Init error: {e}")
                import traceback
                traceback.print_exc()

    def resizeEvent(self, event):
        super().resizeEvent(event)
        if self._initialized:
            size = event.size()
            self.viewer.resize(size.width(), size.height())

    def _render_loop(self):
        if self._initialized:
            try:
                self.viewer.render_frame()
            except Exception as e:
                print(f"Render error: {e}")

    def mousePressEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.left_mouse_pressed = True
            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif event.button() == Qt.RightButton:
            self.right_mouse_pressed = True
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.left_mouse_pressed = False
        elif event.button() == Qt.RightButton:
            self.right_mouse_pressed = False

    def mouseMoveEvent(self, event):
        if self.left_mouse_pressed and self._initialized:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y
            self.viewer.orbit_camera(dx * 0.01, dy * 0.01)
            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif self.right_mouse_pressed and self._initialized and self.image_visual:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y
            self.slice_angle += dy * 0.01
            self.slice_offset += dx * 0.5
            self._update_slice_plane()
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            zoom_delta = -event.angleDelta().y() * 0.1
            self.viewer.zoom_camera(zoom_delta)

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)

    def _update_slice_plane(self):
        if not self.image_visual:
            return

        import math
        normal_y = math.sin(self.slice_angle)
        normal_z = math.cos(self.slice_angle)

        self.image_visual.set_slice_plane(
            128.0, 128.0, 128.0 + self.slice_offset,
            0.0, normal_y, normal_z
        )


class MainWindow(QMainWindow):
    """Main window with multiscale support"""

    def __init__(self, zarr_path):
        super().__init__()

        self.setWindowTitle("Bovista OME-Zarr Multiscale Viewer")
        self.setGeometry(100, 100, 800, 600)

        self.zarr_path = zarr_path
        self.zarr_root = None
        self.current_level = 0

        # Central widget
        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Bovista widget
        self.bovista_widget = BovistaWidget(width=800, height=600)
        layout.addWidget(self.bovista_widget)

        # Controls
        control_layout = QHBoxLayout()

        # Resolution level selector
        control_layout.addWidget(QLabel("Resolution:"))
        self.level_combo = QComboBox()
        self.level_combo.currentIndexChanged.connect(self.on_level_changed)
        control_layout.addWidget(self.level_combo)

        control_layout.addStretch()

        info_label = QLabel("Left-drag: Orbit | Right-drag: Slice | Scroll: Zoom")
        control_layout.addWidget(info_label)

        control_layout.addStretch()

        quit_button = QPushButton("Quit")
        quit_button.clicked.connect(self.close)
        control_layout.addWidget(quit_button)

        layout.addLayout(control_layout)

        # Set up camera
        self.bovista_widget.viewer.set_camera_clip_planes(0.1, 2000.0)
        self.bovista_widget.viewer.set_camera_position(0, 0, 500)
        self.bovista_widget.viewer.set_camera_target(128, 128, 128)

        # Load scene
        self.bovista_widget.on_ready(self.setup_scene)

    def setup_scene(self, viewer):
        """Load OME-Zarr and populate level selector"""
        print("Loading OME-Zarr...")

        # Open zarr
        store = zarr.DirectoryStore(self.zarr_path)
        self.zarr_root = zarr.group(store=store)

        # Get available levels
        multiscales = self.zarr_root.attrs['multiscales'][0]
        datasets = multiscales['datasets']

        print(f"Found {len(datasets)} resolution levels:")
        for i, ds in enumerate(datasets):
            level_data = self.zarr_root[ds['path']]
            shape = level_data.shape
            scale = ds['coordinateTransformations'][0]['scale'][2]  # z scale
            print(f"  Level {i}: {shape} (scale: {scale}x)")
            self.level_combo.addItem(f"Level {i} ({shape[2]}x{shape[3]}x{shape[4]}, {scale}x)", i)

        # Load initial level
        self.load_level(0)

        # Add axes
        axes = bv.PyLinesVisual.axis_helper(viewer, 200.0)
        viewer.add_lines(axes)

        print("Scene loaded!")

    def load_level(self, level):
        """Load a specific resolution level"""
        print(f"Loading level {level}...")

        # Get data
        data = self.zarr_root[str(level)]
        volume = np.array(data[0, 0, :, :, :])

        print(f"  Shape: {volume.shape}, dtype: {volume.dtype}")

        # Remove old image if exists
        if self.bovista_widget.image_visual:
            # Would need a remove_visual method - for now just replace
            pass

        # Create new image visual
        image = bv.PyImageVisual.from_numpy(self.bovista_widget.viewer, volume)
        image.set_slice_plane(128.0, 128.0, 128.0, 0.0, 0.0, 1.0)
        self.bovista_widget.viewer.add_image(image)
        self.bovista_widget.image_visual = image

        self.current_level = level

    def on_level_changed(self, index):
        """Handle resolution level change"""
        if index >= 0 and self.zarr_root is not None:
            level = self.level_combo.itemData(index)
            if level != self.current_level:
                self.load_level(level)


def main():
    """Main entry point"""

    zarr_path = Path("./sample_multiscale.zarr")

    if not zarr_path.exists():
        print("Creating multiscale OME-Zarr...")
        create_multiscale_zarr(str(zarr_path))
    else:
        print(f"Using existing OME-Zarr at {zarr_path}")

    app = QApplication(sys.argv)
    window = MainWindow(str(zarr_path))
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
