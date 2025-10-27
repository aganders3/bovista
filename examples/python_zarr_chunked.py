"""
Bovista + OME-Zarr Chunked Rendering Example

This example demonstrates TRUE chunked rendering where:
1. Large volumes are stored in Zarr with chunk-based storage
2. Only visible chunks are loaded into GPU memory
3. Chunks are loaded on-demand as you navigate
4. Old chunks are evicted automatically (LRU cache)

This allows visualizing TB-scale volumes that don't fit in memory!
"""

import sys
import numpy as np
import zarr
from pathlib import Path
from PySide6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                               QLabel, QPushButton, QHBoxLayout)
from PySide6.QtCore import QTimer, Qt
import bovista as bv


def create_large_zarr(path, shape=(512, 512, 512), chunk_size=(64, 64, 64)):
    """
    Create a large OME-Zarr dataset for chunked loading demo.
    """
    print(f"Creating large OME-Zarr at {path}...")
    print(f"  Volume size: {shape}")
    print(f"  Chunk size: {chunk_size}")

    store = zarr.DirectoryStore(path)
    root = zarr.group(store=store, overwrite=True)

    depth, height, width = shape

    # Create chunked array
    data = root.create_dataset(
        '0',
        shape=(1, 1, depth, height, width),  # (t, c, z, y, x)
        chunks=(1, 1, *chunk_size),
        dtype='uint8',
        compressor=None,  # No compression for speed
        fill_value=0  # Initialize with zeros
    )

    # Generate data procedurally chunk-by-chunk
    print("Generating volume data...")
    cx_base, cy_base, cz_base = width / 2.0, height / 2.0, depth / 2.0
    max_dist = width / 2.0

    num_chunks = (depth // chunk_size[0]) * (height // chunk_size[1]) * (width // chunk_size[2])
    chunk_count = 0

    for z_start in range(0, depth, chunk_size[0]):
        z_end = min(z_start + chunk_size[0], depth)
        for y_start in range(0, height, chunk_size[1]):
            y_end = min(y_start + chunk_size[1], height)
            for x_start in range(0, width, chunk_size[2]):
                x_end = min(x_start + chunk_size[2], width)

                chunk = np.zeros((z_end - z_start, y_end - y_start, x_end - x_start), dtype=np.uint8)

                for z in range(z_end - z_start):
                    for y in range(y_end - y_start):
                        for x in range(x_end - x_start):
                            gz, gy, gx = z_start + z, y_start + y, x_start + x
                            cx, cy, cz = gx - cx_base, gy - cy_base, gz - cz_base
                            dist = np.sqrt(cx*cx + cy*cy + cz*cz)

                            sphere_value = int((1.0 - min(dist / max_dist, 1.0)) * 180.0)
                            core_value = 255 if dist < 30.0 else 0
                            shell1 = 200 if abs(dist - 60.0) < 8.0 else 0
                            shell2 = 220 if abs(dist - 100.0) < 5.0 else 0

                            chunk[z, y, x] = max(sphere_value, core_value, shell1, shell2)

                data[0, 0, z_start:z_end, y_start:y_end, x_start:x_end] = chunk
                chunk_count += 1

                if chunk_count % 10 == 0:
                    progress = (chunk_count / num_chunks) * 100
                    print(f"  Progress: {progress:.1f}% ({chunk_count}/{num_chunks} chunks)")

    # OME-Zarr metadata
    root.attrs['multiscales'] = [{
        'version': '0.4',
        'name': 'chunked_sample',
        'axes': [
            {'name': 't', 'type': 'time'},
            {'name': 'c', 'type': 'channel'},
            {'name': 'z', 'type': 'space', 'unit': 'micrometer'},
            {'name': 'y', 'type': 'space', 'unit': 'micrometer'},
            {'name': 'x', 'type': 'space', 'unit': 'micrometer'},
        ],
        'datasets': [{
            'path': '0',
            'coordinateTransformations': [{'type': 'scale', 'scale': [1.0, 1.0, 1.0, 1.0, 1.0]}]
        }]
    }]

    print("Large OME-Zarr created!")
    return root


class BovistaWidget(QWidget):
    """Qt widget with Bovista viewer"""

    def __init__(self, parent=None, width=800, height=600):
        super().__init__(parent)
        self.setMinimumSize(width, height)
        self.viewer = bv.PyViewer(width, height)
        self._initialized = False
        self._setup_callbacks = []

        self.setMouseTracking(True)
        self.left_mouse_pressed = False
        self.right_mouse_pressed = False
        self.last_x = 0
        self.last_y = 0

        self.slice_angle_x = 0.0
        self.slice_angle_y = 0.0
        self.slice_offset = 0.0
        self.tiled_image = None
        self.debug_mode = False

        # Enable keyboard focus
        self.setFocusPolicy(Qt.StrongFocus)
        self.setFocus()

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
                # Prepare chunks before rendering
                if self.tiled_image:
                    self.tiled_image.prepare_chunks(self.viewer)

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
        elif self.right_mouse_pressed and self._initialized and self.tiled_image:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y

            modifiers = QApplication.keyboardModifiers()
            if modifiers & Qt.ShiftModifier:
                # Shift + right drag: rotate around X axis
                self.slice_angle_x += dy * 0.01
            else:
                # Right drag: rotate around Y axis
                self.slice_angle_y += dy * 0.01

            self.slice_offset += dx * 0.5
            self._update_slice_plane()
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            zoom_delta = -event.angleDelta().y() * 0.1
            self.viewer.zoom_camera(zoom_delta)

    def keyPressEvent(self, event):
        from PySide6.QtCore import Qt
        print(f"Key pressed: {event.key()} (D key is {Qt.Key_D})")
        if event.key() == Qt.Key_D:
            # Toggle debug mode with 'D' key
            self.debug_mode = not self.debug_mode
            if self.tiled_image:
                self.tiled_image.set_debug_mode(self.debug_mode)
                print(f"Debug mode: {'ON' if self.debug_mode else 'OFF'}")
        event.accept()

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)

    def _update_slice_plane(self):
        if not self.tiled_image:
            return

        import math
        # Compute normal vector from two rotation angles
        # Rotate around Y axis, then around X axis
        cos_y = math.cos(self.slice_angle_y)
        sin_y = math.sin(self.slice_angle_y)
        cos_x = math.cos(self.slice_angle_x)
        sin_x = math.sin(self.slice_angle_x)

        # Normal vector after rotations
        normal_x = sin_y * cos_x
        normal_y = -sin_x
        normal_z = cos_y * cos_x

        # Normalize
        length = math.sqrt(normal_x**2 + normal_y**2 + normal_z**2)
        if length > 0.0001:
            normal_x /= length
            normal_y /= length
            normal_z /= length

        center = 256.0  # Adjust based on your volume size
        self.tiled_image.set_slice_plane(
            center, center, center + self.slice_offset,
            normal_x, normal_y, normal_z
        )


class MainWindow(QMainWindow):
    """Main window with chunked loading"""

    def __init__(self, zarr_path):
        super().__init__()
        self.setWindowTitle("Bovista Chunked OME-Zarr Viewer")
        self.setGeometry(100, 100, 800, 600)

        self.zarr_path = zarr_path
        self.zarr_data = None

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        self.bovista_widget = BovistaWidget(width=800, height=600)
        layout.addWidget(self.bovista_widget)

        # Controls
        control_layout = QHBoxLayout()

        self.stats_label = QLabel("Chunks: 0 loaded, 0 visible")
        control_layout.addWidget(self.stats_label)

        control_layout.addStretch()

        info_label = QLabel("Left-drag: Orbit | Right-drag: Slice | Scroll: Zoom")
        control_layout.addWidget(info_label)

        control_layout.addStretch()

        quit_button = QPushButton("Quit")
        quit_button.clicked.connect(self.close)
        control_layout.addWidget(quit_button)

        layout.addLayout(control_layout)

        # Stats update timer
        self.stats_timer = QTimer()
        self.stats_timer.timeout.connect(self.update_stats)
        self.stats_timer.start(500)  # Update stats every 500ms

        # Camera setup
        self.bovista_widget.viewer.set_camera_clip_planes(0.1, 2000.0)
        self.bovista_widget.viewer.set_camera_position(0, 0, 700)
        self.bovista_widget.viewer.set_camera_target(256, 256, 256)

        # Load scene
        self.bovista_widget.on_ready(self.setup_scene)

    def setup_scene(self, viewer):
        """Load Zarr with chunked strategy"""
        print("Loading OME-Zarr...")

        # Open Zarr
        store = zarr.DirectoryStore(self.zarr_path)
        root = zarr.group(store=store)
        self.zarr_data = root['0']

        shape = self.zarr_data.shape
        chunks = self.zarr_data.chunks
        print(f"Zarr array: shape={shape}, chunks={chunks}, dtype={self.zarr_data.dtype}")

        # Define chunk loader
        def load_chunk(z, y, x):
            """Load a single chunk from Zarr on-demand"""
            try:
                # Calculate chunk boundaries
                cz, cy, cx = chunks[2], chunks[3], chunks[4]
                z_start, z_end = z * cz, min((z + 1) * cz, shape[2])
                y_start, y_end = y * cy, min((y + 1) * cy, shape[3])
                x_start, x_end = x * cx, min((x + 1) * cx, shape[4])

                # Load chunk
                chunk_data = self.zarr_data[0, 0, z_start:z_end, y_start:y_end, x_start:x_end]
                result = np.array(chunk_data)

                # Only print first time a chunk is loaded
                # print(f"Loaded chunk ({z},{y},{x}): shape={result.shape}")
                return result
            except Exception as e:
                print(f"Error loading chunk ({z},{y},{x}): {e}")
                return None

        # Create tiled image visual
        print("Creating tiled image visual...")
        volume_size = (shape[2], shape[3], shape[4])  # (d, h, w)
        chunk_size = (chunks[2], chunks[3], chunks[4])

        tiled_image = bv.PyTiledImageVisual.from_loader(
            viewer,
            volume_size,
            chunk_size,
            load_chunk,
            max_loaded_chunks=200  # Keep max 200 chunks for full slice coverage
        )

        # Set initial slice
        center = volume_size[0] / 2
        tiled_image.set_slice_plane(center, center, center, 0.0, 0.0, 1.0)

        # Add to scene
        viewer.add_tiled_image(tiled_image)
        self.bovista_widget.tiled_image = tiled_image

        # Add axes
        axes = bv.PyLinesVisual.axis_helper(viewer, 300.0)
        viewer.add_lines(axes)

        print("Scene loaded! Move the slice to trigger chunk loading.")

    def update_stats(self):
        """Update chunk statistics display"""
        if self.bovista_widget.tiled_image:
            try:
                loaded, visible = self.bovista_widget.tiled_image.get_stats()
                self.stats_label.setText(f"Chunks: {loaded} loaded, {visible} visible")
            except Exception as e:
                pass


def main():
    """Main entry point"""

    zarr_path = Path("./sample_chunked.zarr")

    if not zarr_path.exists():
        print("Creating large chunked OME-Zarr dataset...")
        print("This will take a minute...")
        create_large_zarr(str(zarr_path), shape=(512, 512, 512), chunk_size=(64, 64, 64))
    else:
        print(f"Using existing OME-Zarr at {zarr_path}")

    app = QApplication(sys.argv)
    window = MainWindow(str(zarr_path))
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
