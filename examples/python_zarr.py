"""
Bovista + OME-Zarr Example

This example demonstrates loading microscopy data from OME-Zarr format.
OME-Zarr is a cloud-optimized format for storing multiscale microscopy images.
"""

import sys
import numpy as np
import zarr
from pathlib import Path
from PySide6.QtWidgets import QApplication, QMainWindow, QWidget, QVBoxLayout, QLabel, QPushButton, QHBoxLayout, QProgressBar
from PySide6.QtCore import QTimer, Qt
import bovista as bv


def create_sample_zarr(path, shape=(256, 256, 256)):
    """
    Create a sample OME-Zarr dataset for testing.
    In a real application, you'd load an existing OME-Zarr from disk or HTTP.
    """
    print(f"Creating sample OME-Zarr at {path}...")

    # Create zarr group
    store = zarr.DirectoryStore(path)
    root = zarr.group(store=store, overwrite=True)

    # Create multi-resolution pyramid (typical for OME-Zarr)
    # Level 0: full resolution
    # Level 1: 2x downsampled
    # Level 2: 4x downsampled

    depth, height, width = shape

    # Full resolution level
    print(f"Creating level 0: {depth}x{height}x{width}")
    data = root.create_dataset(
        '0',
        shape=(1, 1, depth, height, width),  # OME-Zarr uses (t, c, z, y, x)
        chunks=(1, 1, 64, 64, 64),  # Chunk size
        dtype='uint8',
        compressor=None  # No compression for speed
    )

    # Generate structured data similar to Qt example
    print("Generating volume data...")
    cx_base = width / 2.0
    cy_base = height / 2.0
    cz_base = depth / 2.0
    max_dist = width / 2.0

    # Fill in chunks
    for z_start in range(0, depth, 64):
        z_end = min(z_start + 64, depth)
        for y_start in range(0, height, 64):
            y_end = min(y_start + 64, height)
            for x_start in range(0, width, 64):
                x_end = min(x_start + 64, width)

                chunk = np.zeros((z_end - z_start, y_end - y_start, x_end - x_start), dtype=np.uint8)

                for z in range(z_end - z_start):
                    for y in range(y_end - y_start):
                        for x in range(x_end - x_start):
                            gz = z_start + z
                            gy = y_start + y
                            gx = x_start + x

                            cx = gx - cx_base
                            cy = gy - cy_base
                            cz = gz - cz_base
                            dist = np.sqrt(cx*cx + cy*cy + cz*cz)

                            sphere_value = int((1.0 - min(dist / max_dist, 1.0)) * 180.0)
                            core_value = 255 if dist < 20.0 else 0
                            shell1_dist = abs(dist - 40.0)
                            shell1 = 200 if shell1_dist < 5.0 else 0

                            chunk[z, y, x] = max(sphere_value, core_value, shell1)

                data[0, 0, z_start:z_end, y_start:y_end, x_start:x_end] = chunk

        print(f"  Generated z={z_start}-{z_end}")

    # Add OME-Zarr metadata
    root.attrs['multiscales'] = [{
        'version': '0.4',
        'name': 'sample',
        'axes': [
            {'name': 't', 'type': 'time', 'unit': 'millisecond'},
            {'name': 'c', 'type': 'channel'},
            {'name': 'z', 'type': 'space', 'unit': 'micrometer'},
            {'name': 'y', 'type': 'space', 'unit': 'micrometer'},
            {'name': 'x', 'type': 'space', 'unit': 'micrometer'},
        ],
        'datasets': [
            {
                'path': '0',
                'coordinateTransformations': [{
                    'type': 'scale',
                    'scale': [1.0, 1.0, 1.0, 1.0, 1.0]
                }]
            }
        ]
    }]

    print("Sample OME-Zarr created!")
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
        self.timer.start(16)  # ~60 FPS

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
                print(f"Failed to initialize: {e}")
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
    """Main window for OME-Zarr viewer"""

    def __init__(self, zarr_path):
        super().__init__()

        self.setWindowTitle("Bovista OME-Zarr Viewer")
        self.setGeometry(100, 100, 800, 600)

        self.zarr_path = zarr_path

        # Central widget
        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Bovista widget
        self.bovista_widget = BovistaWidget(width=800, height=600)
        layout.addWidget(self.bovista_widget)

        # Controls
        control_layout = QHBoxLayout()

        info_label = QLabel("Left-drag: Orbit | Right-drag: Slice | Scroll: Zoom")
        control_layout.addWidget(info_label)

        control_layout.addStretch()

        # Progress bar for loading
        self.progress = QProgressBar()
        self.progress.setVisible(False)
        control_layout.addWidget(self.progress)

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
        """Load OME-Zarr data and set up scene"""
        print("Loading OME-Zarr data...")
        self.progress.setVisible(True)
        self.progress.setRange(0, 100)

        # Open zarr store
        store = zarr.DirectoryStore(self.zarr_path)
        root = zarr.group(store=store)

        # Get full resolution data (level 0)
        # OME-Zarr format: (t, c, z, y, x)
        data = root['0']

        print(f"Loaded zarr array: shape={data.shape}, chunks={data.chunks}, dtype={data.dtype}")

        # For now, load the full volume into memory
        # In the future, we'll implement progressive loading
        print("Reading volume into memory...")
        volume = np.array(data[0, 0, :, :, :])  # Get first timepoint and channel

        print(f"Volume shape: {volume.shape}, dtype: {volume.dtype}")

        # Create image visual
        print("Creating image visual...")
        image = bv.PyImageVisual.from_numpy(viewer, volume)
        image.set_slice_plane(128.0, 128.0, 128.0, 0.0, 0.0, 1.0)
        viewer.add_image(image)
        self.bovista_widget.image_visual = image

        # Add axes
        print("Adding axes...")
        axes = bv.PyLinesVisual.axis_helper(viewer, 200.0)
        viewer.add_lines(axes)

        self.progress.setVisible(False)
        print("Scene loaded!")


def main():
    """Main entry point"""

    # Create or use existing OME-Zarr
    zarr_path = Path("./sample_data.zarr")

    if not zarr_path.exists():
        print("Creating sample OME-Zarr dataset...")
        create_sample_zarr(str(zarr_path))
    else:
        print(f"Using existing OME-Zarr at {zarr_path}")

    # Start Qt application
    app = QApplication(sys.argv)
    window = MainWindow(str(zarr_path))
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
