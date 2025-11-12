"""
Test new unified ImageVisual with chunked multi-resolution loading

This tests Image.from_chunked() which uses the new tile-based architecture
with automatic LOD selection, loading chunks on-demand from remote OME-Zarr.

This is the SAME functionality as TiledImage.from_levels(), but using the new
unified ImageVisual codebase.

Requirements:
    pip install PySide6 zarr fsspec requests aiohttp bovista numpy
"""

import sys
import numpy as np
import zarr
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QLabel, QPushButton,
    QHBoxLayout, QSlider,
)
from PySide6.QtCore import QTimer, Qt
import bovista as bv
from concurrent.futures import ThreadPoolExecutor
from threading import Lock


# Simplified dataset (just one for testing)
TEST_DATASET = {
    "url": "https://s3.embl.de/i2k-2020/platy-raw.ome.zarr",
    "description": "Platynereis embryo, 10 resolution levels",
}


class SimpleViewer(QWidget):
    """Minimal Qt widget for testing"""

    def __init__(self, parent=None, width=800, height=600):
        super().__init__(parent)
        self.setMinimumSize(width, height)
        self.viewer = bv.Viewer(width, height)
        self._initialized = False
        self._setup_callbacks = []
        self._image = None

        self.setMouseTracking(True)
        self.left_mouse_pressed = False
        self.right_mouse_pressed = False
        self.last_x = 0
        self.last_y = 0
        self.slice_angle_x = 0.0
        self.slice_angle_y = 0.0
        self.slice_offset = 0.0
        self.volume_center = [0, 0, 0]

        self.setFocusPolicy(Qt.ClickFocus)

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
                print("✓ Viewer initialized")

                for callback in self._setup_callbacks:
                    callback(self.viewer)
                self._setup_callbacks.clear()
            except Exception as e:
                print(f"❌ Init error: {e}")
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
                print(f"❌ Render error: {e}")

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
        elif self.right_mouse_pressed and self._initialized and self._image:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y

            modifiers = QApplication.keyboardModifiers()
            if modifiers & Qt.ShiftModifier:
                self.slice_angle_x += dy * 0.01
            else:
                self.slice_angle_y += dy * 0.01

            self.slice_offset += dx * 0.5
            self._update_slice_plane()
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            zoom_delta = -event.angleDelta().y() * 0.2
            self.viewer.zoom_camera(zoom_delta)

    def keyPressEvent(self, event):
        if event.key() == Qt.Key_D and self._image:
            # Toggle debug mode
            # (We'd need to track state, for now just print)
            print("Debug mode toggle (not fully implemented in this test)")
        event.accept()

    def _update_slice_plane(self):
        if not self._image:
            return

        import math
        cos_y = math.cos(self.slice_angle_y)
        sin_y = math.sin(self.slice_angle_y)
        cos_x = math.cos(self.slice_angle_x)
        sin_x = math.sin(self.slice_angle_x)

        normal_x = sin_y * cos_x
        normal_y = -sin_x
        normal_z = cos_y * cos_x

        length = math.sqrt(normal_x**2 + normal_y**2 + normal_z**2)
        if length > 0.0001:
            normal_x /= length
            normal_y /= length
            normal_z /= length

        center = self.volume_center
        self._image.set_slice_plane(
            center[0],
            center[1],
            center[2] + self.slice_offset,
            normal_x,
            normal_y,
            normal_z,
        )

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)


class MainWindow(QMainWindow):
    """Main window for testing"""

    def __init__(self):
        super().__init__()
        self.setWindowTitle("Test Image.from_chunked() - New Unified System")
        self.setGeometry(100, 100, 900, 700)

        self.zarr_data = None
        self.chunk_executor = None

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Info label
        info_label = QLabel("Testing new Image.from_chunked() with OME-Zarr")
        info_label.setStyleSheet("font-weight: bold; font-size: 14px;")
        layout.addWidget(info_label)

        # Load button
        load_layout = QHBoxLayout()
        self.load_button = QPushButton("Load Test Dataset")
        self.load_button.clicked.connect(self.load_dataset)
        load_layout.addWidget(self.load_button)
        self.status_label = QLabel("Click 'Load Test Dataset' to begin")
        load_layout.addWidget(self.status_label)
        load_layout.addStretch()
        layout.addLayout(load_layout)

        # LOD bias slider
        lod_layout = QHBoxLayout()
        lod_layout.addWidget(QLabel("LOD Bias:"))
        self.lod_slider = QSlider(Qt.Horizontal)
        self.lod_slider.setMinimum(-20)
        self.lod_slider.setMaximum(20)
        self.lod_slider.setValue(0)
        self.lod_slider.setEnabled(False)
        self.lod_slider.valueChanged.connect(self.update_lod_bias)
        lod_layout.addWidget(self.lod_slider)
        self.lod_label = QLabel("0.0")
        lod_layout.addWidget(self.lod_label)
        lod_layout.addStretch()
        layout.addLayout(lod_layout)

        # Viewer
        self.viewer_widget = SimpleViewer(width=900, height=550)
        layout.addWidget(self.viewer_widget)

        # Controls
        controls_label = QLabel("Left-drag: Orbit | Right-drag: Slice | Scroll: Zoom")
        layout.addWidget(controls_label)

        # Stats timer
        self.stats_timer = QTimer()
        self.stats_timer.timeout.connect(self.update_stats)
        self.stats_timer.start(500)

        # Camera setup
        self.viewer_widget.viewer.set_camera_clip_planes(0.1, 10000.0)

    def closeEvent(self, event):
        if self.chunk_executor is not None:
            self.chunk_executor.shutdown(wait=False, cancel_futures=True)
        super().closeEvent(event)

    def load_dataset(self):
        """Load the test dataset"""
        self.load_button.setEnabled(False)
        self.status_label.setText("Loading metadata...")
        QApplication.processEvents()

        try:
            print("\n" + "=" * 60)
            print("Loading dataset...")
            print(f"URL: {TEST_DATASET['url']}")
            print("=" * 60)

            # Open zarr store
            store = zarr.open(TEST_DATASET["url"], mode="r")
            self.zarr_data = store

            # Get multiscales info
            multiscales = store.attrs["multiscales"][0]
            datasets = multiscales["datasets"]
            axes = multiscales.get("axes", [])
            axis_names = [ax.get("name", "?").lower() for ax in axes]

            print(f"\nFound {len(datasets)} LOD levels")
            print(f"Axes: {axis_names}")

            # Find spatial indices
            spatial_axes = {}
            for i, name in enumerate(axis_names):
                if name in ["z", "y", "x"]:
                    spatial_axes[name] = i

            if len(spatial_axes) == 3:
                z_idx, y_idx, x_idx = spatial_axes["z"], spatial_axes["y"], spatial_axes["x"]
            else:
                # Fallback
                first_array = store[datasets[0]["path"]]
                ndim = len(first_array.shape)
                if ndim == 5:
                    z_idx, y_idx, x_idx = 2, 3, 4
                else:
                    z_idx, y_idx, x_idx = 0, 1, 2

            spatial_indices = (z_idx, y_idx, x_idx)
            print(f"Spatial indices: z={z_idx}, y={y_idx}, x={x_idx}")

            # Create LevelMetadata for each LOD
            lod_levels = []
            lod0_voxel_spacing = None

            for lod_idx, dataset_info in enumerate(datasets):
                path = dataset_info["path"]
                array = store[path]
                shape = array.shape
                chunks = array.chunks

                volume_shape = (shape[z_idx], shape[y_idx], shape[x_idx])
                chunk_shape = (chunks[z_idx], chunks[y_idx], chunks[x_idx])

                # Get voxel spacing
                transforms = dataset_info.get("coordinateTransformations", [])
                voxel_spacing = [1.0, 1.0, 1.0]
                for transform in transforms:
                    if transform.get("type") == "scale":
                        scale = transform.get("scale", [])
                        if len(scale) > max(z_idx, y_idx, x_idx):
                            voxel_spacing = [scale[z_idx], scale[y_idx], scale[x_idx]]

                if lod_idx == 0:
                    lod0_voxel_spacing = voxel_spacing
                    lod0_volume_shape_ref = volume_shape
                    scale_factor = 1.0
                else:
                    # Calculate scale factor using the dimension that changes between LODs
                    # (Z might be isotropic across LODs in anisotropic datasets)
                    # Try X/Y first (usually these change), fall back to Z
                    if voxel_spacing[2] != lod0_voxel_spacing[2]:
                        scale_factor = voxel_spacing[2] / lod0_voxel_spacing[2]
                    elif voxel_spacing[1] != lod0_voxel_spacing[1]:
                        scale_factor = voxel_spacing[1] / lod0_voxel_spacing[1]
                    elif voxel_spacing[0] != lod0_voxel_spacing[0]:
                        scale_factor = voxel_spacing[0] / lod0_voxel_spacing[0]
                    else:
                        # If voxel spacing is the same, calculate from volume dimensions
                        scale_factor = max(
                            lod0_volume_shape_ref[0] / volume_shape[0],
                            lod0_volume_shape_ref[1] / volume_shape[1],
                            lod0_volume_shape_ref[2] / volume_shape[2]
                        )

                level = bv.LevelMetadata(
                    volume_size=volume_shape,
                    chunk_size=chunk_shape,
                    voxel_size=tuple(voxel_spacing),
                    scale_factor=scale_factor
                )
                lod_levels.append(level)

                print(f"\nLOD {lod_idx}:")
                print(f"  Volume: {volume_shape}")
                print(f"  Chunks: {chunk_shape}")
                print(f"  Voxel spacing: {voxel_spacing}")
                print(f"  Scale: {scale_factor:.2f}x")

            # Setup loader in background thread
            if self.chunk_executor:
                self.chunk_executor.shutdown(wait=False)

            self.chunk_executor = ThreadPoolExecutor(
                max_workers=8, thread_name_prefix="zarr_loader"
            )

            chunk_cache = {}
            pending_futures = {}
            cache_lock = Lock()
            loaded_count = [0]

            def load_chunk_blocking(lod_level, z, y, x):
                try:
                    path = datasets[lod_level]["path"]
                    array = store[path]
                    level = lod_levels[lod_level]

                    cz, cy, cx = level.chunk_size
                    z_start = z * cz
                    y_start = y * cy
                    x_start = x * cx
                    z_end = min((z + 1) * cz, level.volume_size[0])
                    y_end = min((y + 1) * cy, level.volume_size[1])
                    x_end = min((x + 1) * cx, level.volume_size[2])

                    if z_start >= level.volume_size[0] or y_start >= level.volume_size[1] or x_start >= level.volume_size[2]:
                        return None

                    # Build slicing tuple
                    slices = [slice(None)] * len(array.shape)
                    # Take first channel/timepoint if present
                    for i in range(len(array.shape)):
                        if i not in spatial_indices:
                            slices[i] = 0

                    slices[z_idx] = slice(z_start, z_end)
                    slices[y_idx] = slice(y_start, y_end)
                    slices[x_idx] = slice(x_start, x_end)

                    chunk_data = array[tuple(slices)]

                    if chunk_data.size == 0:
                        return None

                    # Convert to uint8 with per-chunk normalization
                    if chunk_data.dtype != np.uint8:
                        chunk_min = float(chunk_data.min())
                        chunk_max = float(chunk_data.max())
                        if chunk_max > chunk_min:
                            chunk_data = ((chunk_data - chunk_min) / (chunk_max - chunk_min) * 255).astype(np.uint8)
                        else:
                            chunk_data = np.zeros_like(chunk_data, dtype=np.uint8)

                    return np.array(chunk_data)
                except Exception as e:
                    print(f"ERROR loading chunk LOD{lod_level} ({z},{y},{x}): {e}")
                    return None

            def on_chunk_loaded(lod, z, y, x, future):
                key = (lod, z, y, x)
                try:
                    if future.cancelled():
                        with cache_lock:
                            pending_futures.pop(key, None)
                        return

                    result = future.result()
                    with cache_lock:
                        pending_futures.pop(key, None)
                        if result is not None:
                            chunk_cache[key] = result
                            loaded_count[0] += 1
                            if loaded_count[0] <= 5 or loaded_count[0] % 50 == 0:
                                print(f"  Loaded LOD{lod} ({z},{y},{x}) - total: {loaded_count[0]}")
                except Exception:
                    with cache_lock:
                        pending_futures.pop(key, None)

            def load_chunk(lod, z, y, x):
                key = (lod, z, y, x)
                with cache_lock:
                    if key in chunk_cache:
                        return chunk_cache[key]
                    if key not in pending_futures:
                        future = self.chunk_executor.submit(load_chunk_blocking, lod, z, y, x)
                        future.add_done_callback(lambda f: on_chunk_loaded(lod, z, y, x, f))
                        pending_futures[key] = future
                return None

            def check_capacity():
                """Return how many more chunk requests can be accepted"""
                with cache_lock:
                    pending = len(pending_futures)
                NUM_WORKERS = 8
                MAX_QUEUE_SIZE = NUM_WORKERS
                capacity = max(0, MAX_QUEUE_SIZE - pending)
                return capacity

            # Setup scene when viewer is ready
            def setup_scene(viewer):
                print("\n" + "=" * 60)
                print("Creating Image.from_chunked()...")
                print("=" * 60)

                # Create image with chunked loading
                image = bv.Image.from_chunked(
                    viewer,
                    lod_levels,
                    load_chunk,
                    max_chunks=500,
                    capacity_check=check_capacity
                )

                print("✓ Image created!")

                # Calculate world dimensions
                lod0 = lod_levels[0]
                volume_size_world = (
                    lod0.volume_size[0] * lod0.voxel_size[0],
                    lod0.volume_size[1] * lod0.voxel_size[1],
                    lod0.volume_size[2] * lod0.voxel_size[2],
                )

                center = [
                    volume_size_world[2] / 2,
                    volume_size_world[1] / 2,
                    volume_size_world[0] / 2,
                ]

                # Set slice plane
                image.set_slice_plane(
                    center[0], center[1], center[2],
                    0.0, 0.0, 1.0  # XY plane
                )

                # Set contrast
                image.set_contrast(0.0, 1.0)

                # Add to scene
                viewer.add(image)
                self.viewer_widget._image = image
                self.viewer_widget.volume_center = center

                # Setup camera
                xy_max = max(volume_size_world[1], volume_size_world[2])
                distance = xy_max * 1.0
                viewer.set_camera_position(center[0], center[1], center[2] + distance)
                viewer.set_camera_target(center[0], center[1], center[2])

                print(f"\n  Volume size (world): {volume_size_world}")
                print(f"  Center: {center}")
                print(f"  Camera position: ({center[0]}, {center[1]}, {center[2] + distance})")
                print(f"  Camera target: ({center[0]}, {center[1]}, {center[2]})")
                print(f"  Distance: {distance}")

                # Add axes
                axes = bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3)
                viewer.add(axes)

                print("✓ Scene ready!")
                print("=" * 60)

                self.lod_slider.setEnabled(True)
                self.status_label.setText("Loaded! Navigate with mouse.")

            self.viewer_widget.on_ready(setup_scene)

        except Exception as e:
            self.status_label.setText(f"Error: {e}")
            print(f"\n❌ Error: {e}")
            import traceback
            traceback.print_exc()
            self.load_button.setEnabled(True)

    def update_lod_bias(self):
        if self.viewer_widget._image:
            bias = self.lod_slider.value() / 10.0
            self.lod_label.setText(f"{bias:.1f}")
            self.viewer_widget._image.set_lod_bias(bias)

    def update_stats(self):
        if self.viewer_widget._image:
            try:
                loaded, visible = self.viewer_widget._image.get_stats()
                self.status_label.setText(
                    f"Chunks: {loaded} loaded, {visible} visible"
                )
            except Exception:
                pass


def main():
    print("\n" + "=" * 60)
    print("Testing Image.from_chunked()")
    print("=" * 60)
    print("\nThis test verifies the new unified ImageVisual works with")
    print("multi-resolution chunked loading from remote OME-Zarr.")
    print("=" * 60 + "\n")

    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
