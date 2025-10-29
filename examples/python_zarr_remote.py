"""
Bovista + Remote Multiscale OME-Zarr Example

This example demonstrates loading and visualizing a real multiscale OME-Zarr dataset
from a remote S3 bucket (no download required!). This showcases Bovista's ability to:

1. Access cloud-native datasets via HTTP/S3
2. Handle multiscale pyramids for different zoom levels
3. Load only visible chunks on-demand (efficient memory usage)
4. Navigate TB-scale datasets interactively

Dataset: IDR (Image Data Resource) public OME-Zarr collection
Source: https://www.openmicroscopy.org/2020/11/04/zarr-data.html
"""

import sys
import numpy as np
import zarr
from PySide6.QtWidgets import (
    QApplication,
    QMainWindow,
    QWidget,
    QVBoxLayout,
    QLabel,
    QPushButton,
    QHBoxLayout,
    QComboBox,
    QSlider,
)
from PySide6.QtCore import QTimer, Qt
import bovista as bv
from concurrent.futures import ThreadPoolExecutor
from threading import Lock


# Public IDR OME-Zarr datasets on S3
DATASETS = {
    "Human Mitotic Cell (small)": {
        "url": "https://uk1s3.embassy.ebi.ac.uk/idr/zarr/v0.4/idr0062A/6001240.zarr",
        "description": "HeLa cells, ~200MB, 5 resolution levels",
        "camera_distance": 200.0,
    },
    "Platybrowser (medium)": {
        "url": "https://s3.embl.de/i2k-2020/platy-raw.ome.zarr",
        "description": "Platynereis embryo, ~1GB, 3 resolution levels",
        "camera_distance": 400.0,
    },
}


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
        self.volume_center = [0, 0, 0]

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
                self.slice_angle_x += dy * 0.01
            else:
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
        if event.key() == Qt.Key_D:
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
        self.tiled_image.set_slice_plane(
            center[0],
            center[1],
            center[2] + self.slice_offset,
            normal_x,
            normal_y,
            normal_z,
        )


class MainWindow(QMainWindow):
    """Main window with remote dataset loading"""

    def __init__(self):
        super().__init__()
        self.setWindowTitle("Bovista Remote OME-Zarr Viewer")
        self.setGeometry(100, 100, 1000, 700)

        self.zarr_data = None
        self.current_resolution = 0
        self.data_range = (0.0, 255.0)  # Will be updated when data loads
        self.dataset_info = None  # Currently loaded dataset info

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Dataset selector
        selector_layout = QHBoxLayout()
        selector_layout.addWidget(QLabel("Dataset:"))

        self.dataset_combo = QComboBox()
        for name, info in DATASETS.items():
            self.dataset_combo.addItem(f"{name} - {info['description']}", name)
        selector_layout.addWidget(self.dataset_combo)

        self.load_button = QPushButton("Load Dataset")
        self.load_button.clicked.connect(self.load_selected_dataset)
        selector_layout.addWidget(self.load_button)

        selector_layout.addStretch()
        layout.addLayout(selector_layout)

        # Resolution selector
        res_layout = QHBoxLayout()
        res_layout.addWidget(QLabel("Resolution:"))

        self.resolution_combo = QComboBox()
        self.resolution_combo.currentIndexChanged.connect(self.change_resolution)
        self.resolution_combo.setEnabled(False)
        res_layout.addWidget(self.resolution_combo)

        res_layout.addStretch()
        layout.addLayout(res_layout)

        # Contrast controls
        contrast_layout = QHBoxLayout()
        contrast_layout.addWidget(QLabel("Contrast Min:"))

        self.contrast_min_slider = QSlider(Qt.Horizontal)
        self.contrast_min_slider.setMinimum(0)
        self.contrast_min_slider.setMaximum(1000)  # Finer granularity
        self.contrast_min_slider.setValue(0)
        self.contrast_min_slider.valueChanged.connect(self.update_contrast)
        contrast_layout.addWidget(self.contrast_min_slider)

        self.contrast_min_label = QLabel("0")
        self.contrast_min_label.setMinimumWidth(50)
        contrast_layout.addWidget(self.contrast_min_label)

        contrast_layout.addWidget(QLabel("Max:"))

        self.contrast_max_slider = QSlider(Qt.Horizontal)
        self.contrast_max_slider.setMinimum(0)
        self.contrast_max_slider.setMaximum(1000)  # Finer granularity
        self.contrast_max_slider.setValue(1000)
        self.contrast_max_slider.valueChanged.connect(self.update_contrast)
        contrast_layout.addWidget(self.contrast_max_slider)

        self.contrast_max_label = QLabel("255")
        self.contrast_max_label.setMinimumWidth(50)
        contrast_layout.addWidget(self.contrast_max_label)

        contrast_layout.addStretch()
        layout.addLayout(contrast_layout)

        # Bovista viewer
        self.bovista_widget = BovistaWidget(width=1000, height=600)
        layout.addWidget(self.bovista_widget)

        # Stats and controls
        control_layout = QHBoxLayout()

        self.stats_label = QLabel("Load a dataset to begin")
        control_layout.addWidget(self.stats_label)

        control_layout.addStretch()

        info_label = QLabel(
            "Left-drag: Orbit | Right-drag: Slice | Scroll: Zoom | D: Debug"
        )
        control_layout.addWidget(info_label)

        control_layout.addStretch()

        quit_button = QPushButton("Quit")
        quit_button.clicked.connect(self.close)
        control_layout.addWidget(quit_button)

        layout.addLayout(control_layout)

        # Stats update timer
        self.stats_timer = QTimer()
        self.stats_timer.timeout.connect(self.update_stats)
        self.stats_timer.start(500)

        # Initial camera setup
        self.bovista_widget.viewer.set_camera_clip_planes(0.1, 10000.0)

    def load_selected_dataset(self):
        """Load the selected remote dataset"""
        dataset_name = self.dataset_combo.currentData()
        dataset_info = DATASETS[dataset_name]

        self.load_button.setEnabled(False)
        self.stats_label.setText("Loading metadata from S3...")
        QApplication.processEvents()

        # Run blocking I/O in background thread
        def load_metadata():
            try:
                print(f"\nLoading remote dataset: {dataset_name}")
                print(f"URL: {dataset_info['url']}")

                # Open remote zarr store (BLOCKING - fetches metadata from S3)
                store = zarr.open(dataset_info["url"], mode="r")

                # Get resolution level paths (fast - no S3 fetches for array metadata)
                if "multiscales" in store.attrs:
                    multiscales = store.attrs["multiscales"][0]
                    datasets = multiscales["datasets"]

                    print(f"Found {len(datasets)} resolution levels (metadata will load on-demand)")
                    level_info = []
                    for i, ds in enumerate(datasets):
                        level_info.append({"path": ds["path"], "index": i})

                    return store, level_info, None
                else:
                    return store, [], "No multiscales metadata found"

            except Exception as e:
                return None, [], str(e)

        def on_metadata_loaded(future):
            try:
                store, level_info, error = future.result()

                if error or not level_info:
                    self.stats_label.setText(f"Error: {error or 'Unknown error'}")
                    print(f"Error loading dataset: {error}")
                    self.load_button.setEnabled(True)
                    return

                self.process_loaded_store(store, level_info, dataset_info, dataset_name)

            except Exception as e:
                self.stats_label.setText(f"Error: {e}")
                print(f"Error in callback: {e}")
                self.load_button.setEnabled(True)

        # Submit to thread pool
        if not hasattr(self, "_metadata_executor"):
            self._metadata_executor = ThreadPoolExecutor(
                max_workers=1, thread_name_prefix="metadata_loader"
            )

        future = self._metadata_executor.submit(load_metadata)
        future.add_done_callback(on_metadata_loaded)

    def process_loaded_store(self, store, level_info, dataset_info, dataset_name):
        """Process loaded zarr store (on main thread)"""
        try:
            # Populate resolution combo with just level numbers (shapes loaded lazily)
            self.resolution_combo.clear()
            for info in level_info:
                i = info["index"]
                self.resolution_combo.addItem(f"Level {i}", i)

            self.zarr_data = store
            self.dataset_info = dataset_info

            # Don't auto-load any resolution - let user pick
            self.current_resolution = -1  # No resolution loaded yet

            # Enable resolution selector but don't select anything yet
            self.resolution_combo.setCurrentIndex(-1)
            self.resolution_combo.setEnabled(True)

            self.stats_label.setText(f"Loaded: {dataset_name} - Select a resolution level")

        except Exception as e:
            self.stats_label.setText(f"Error loading dataset: {e}")
            print(f"Error: {e}")
            import traceback

            traceback.print_exc()
        finally:
            self.load_button.setEnabled(True)

    def setup_scene(self, viewer, dataset_info):
        """Setup the visualization with chunked loading"""
        if self.zarr_data is None:
            return

        try:
            # Get the dataset at current resolution
            multiscales = self.zarr_data.attrs["multiscales"][0]
            dataset_path = multiscales["datasets"][self.current_resolution]["path"]
            array = self.zarr_data[dataset_path]

            print(
                f"\nSetting up visualization for resolution level {self.current_resolution}"
            )
            print(f"  Array shape: {array.shape}")
            print(f"  Chunk size: {array.chunks}")
            print(f"  Data type: {array.dtype}")

            shape = array.shape
            chunks = array.chunks

            # Detect format and extract spatial dimensions using axes metadata
            multiscales = self.zarr_data.attrs.get("multiscales", [{}])[0]
            axes = multiscales.get("axes", [])
            axis_names = [ax.get("name", "?").lower() for ax in axes]

            print(f"  Axes metadata: {axis_names}")

            # Find indices of spatial dimensions (z, y, x)
            spatial_axes = {}
            for i, name in enumerate(axis_names):
                if name in ["z", "y", "x"]:
                    spatial_axes[name] = i

            # If we have proper spatial axes, extract them
            if len(spatial_axes) == 3:
                z_idx = spatial_axes["z"]
                y_idx = spatial_axes["y"]
                x_idx = spatial_axes["x"]

                volume_shape = (shape[z_idx], shape[y_idx], shape[x_idx])
                chunk_shape = (chunks[z_idx], chunks[y_idx], chunks[x_idx])

                # Store indices for chunk loading
                spatial_indices = (z_idx, y_idx, x_idx)
            else:
                # Fallback to old behavior if axes metadata is missing
                ndim = len(shape)
                if ndim == 5:
                    volume_shape = shape[2:]
                    chunk_shape = chunks[2:]
                    spatial_indices = (2, 3, 4)
                elif ndim == 4:
                    volume_shape = shape[1:]
                    chunk_shape = chunks[1:]
                    spatial_indices = (1, 2, 3)
                else:
                    volume_shape = shape
                    chunk_shape = chunks
                    spatial_indices = (0, 1, 2)

            print(f"  Extracted volume shape (z,y,x): {volume_shape}")
            print(f"  Extracted chunk shape: {chunk_shape}")
            print(f"  Spatial indices in array: {spatial_indices}")

            # Compute contrast from a small sample
            print("Computing contrast from data sample...")
            try:
                # Sample a small region from the center
                sample_slices = [0] * len(shape)

                # For non-spatial dimensions, take first element
                for i in range(len(shape)):
                    if i not in spatial_indices:
                        sample_slices[i] = 0

                # Take a small centered sample (keep it small for fast loading from S3)
                z_idx, y_idx, x_idx = spatial_indices
                center_z = volume_shape[0] // 2
                center_y = volume_shape[1] // 2
                center_x = volume_shape[2] // 2

                # Sample just 1 slice in Z, 64x64 in XY
                sample_slices[z_idx] = center_z
                sample_slices[y_idx] = slice(max(0, center_y - 32), min(volume_shape[1], center_y + 32))
                sample_slices[x_idx] = slice(max(0, center_x - 32), min(volume_shape[2], center_x + 32))

                print(f"  Sampling single Z slice at center (z={center_z}, y={center_y-32}:{center_y+32}, x={center_x-32}:{center_x+32})")
                sample_data = array[tuple(sample_slices)]

                data_min = float(sample_data.min())
                data_max = float(sample_data.max())

                print(f"  Sample range: {data_min} to {data_max}")

                # If we got all zeros or a very narrow range, use dtype-based defaults
                if data_max - data_min < 1.0:
                    print(f"  Warning: Sample range too narrow ({data_min}-{data_max}), using dtype defaults")
                    if array.dtype == np.uint16:
                        data_min, data_max = 0.0, 65535.0
                        global_min, global_max = 0.0, 4095.0
                    elif array.dtype == np.uint8:
                        data_min, data_max = 0.0, 255.0
                        global_min, global_max = 0.0, 255.0
                    else:
                        # Float types - use reasonable defaults
                        data_min, data_max = 0.0, 1.0
                        global_min, global_max = 0.0, 1.0
                else:
                    global_min = float(np.percentile(sample_data, 1.0))
                    global_max = float(np.percentile(sample_data, 99.0))

                print(f"  Data range for normalization: {data_min:.1f} to {data_max:.1f}")
                print(f"  Initial contrast window (1-99%): {global_min:.1f} to {global_max:.1f}")

                # Update data range for contrast sliders
                self.data_range = (data_min, data_max)
            except Exception as e:
                print(f"  Error computing contrast: {e}, using defaults")
                import traceback
                traceback.print_exc()
                global_min, global_max = (0.0, 255.0) if array.dtype == np.uint8 else (0.0, 4095.0)
                self.data_range = (global_min, global_max)

            # Track chunk loading stats
            loaded_count = [0]  # Use list to allow mutation in closure

            # Cache for loaded chunks and pending loads
            chunk_cache = {}  # (z,y,x) -> numpy array
            pending_loads = set()  # (z,y,x) tuples currently being loaded
            cache_lock = Lock()

            # Thread pool for async loading
            executor = ThreadPoolExecutor(
                max_workers=4, thread_name_prefix="zarr_loader"
            )

            def load_chunk_blocking(z, y, x):
                """Actually load a chunk from remote Zarr (blocking I/O)"""
                print(f"    load_chunk_blocking({z},{y},{x}) STARTED in thread")
                try:
                    cz, cy, cx = chunk_shape
                    z_start, z_end = z * cz, min((z + 1) * cz, volume_shape[0])
                    y_start, y_end = y * cy, min((y + 1) * cy, volume_shape[1])
                    x_start, x_end = x * cx, min((x + 1) * cx, volume_shape[2])

                    # Check if this chunk is out of bounds
                    if (
                        z_start >= volume_shape[0]
                        or y_start >= volume_shape[1]
                        or x_start >= volume_shape[2]
                    ):
                        return None

                    # Check if chunk would be empty
                    if z_start >= z_end or y_start >= y_end or x_start >= x_end:
                        return None

                    # Build slicing tuple based on spatial indices
                    z_idx, y_idx, x_idx = spatial_indices
                    slices = [slice(None)] * len(shape)  # Start with full slices

                    # For non-spatial dimensions, take first element
                    for i in range(len(shape)):
                        if i not in spatial_indices:
                            slices[i] = 0  # Take first channel/time point

                    # Set spatial slices
                    slices[z_idx] = slice(z_start, z_end)
                    slices[y_idx] = slice(y_start, y_end)
                    slices[x_idx] = slice(x_start, x_end)

                    # Load from remote zarr (THIS IS WHERE THE BLOCKING I/O HAPPENS)
                    print(f"    About to fetch from S3...")
                    chunk_data = array[tuple(slices)]
                    print(f"    S3 fetch complete, got shape {chunk_data.shape}")

                    # Check if we got empty data
                    if chunk_data.size == 0:
                        return None

                    # Convert to uint8 using the FULL data range (not percentiles)
                    # The contrast sliders will remap this range for display
                    if chunk_data.dtype != np.uint8:
                        if data_max > data_min:
                            # Use the full data range for normalization
                            chunk_data = np.clip(chunk_data, data_min, data_max)
                            chunk_data = (
                                (chunk_data - data_min)
                                / (data_max - data_min)
                                * 255
                            ).astype(np.uint8)
                        else:
                            chunk_data = np.zeros_like(chunk_data, dtype=np.uint8)

                    print(f"    load_chunk_blocking({z},{y},{x}) COMPLETED")
                    return np.array(chunk_data)
                except Exception as e:
                    print(f"ERROR loading chunk ({z},{y},{x}): {e}")
                    return None

            def on_chunk_loaded(z, y, x, future):
                """Callback when chunk loading completes"""
                try:
                    result = future.result()
                    with cache_lock:
                        pending_loads.discard((z, y, x))
                        if result is not None:
                            chunk_cache[(z, y, x)] = result
                            loaded_count[0] += 1
                            if loaded_count[0] <= 5 or loaded_count[0] % 50 == 0:
                                print(
                                    f"  Chunk ({z},{y},{x}) loaded in background (total: {loaded_count[0]})"
                                )
                except Exception as e:
                    print(f"  ERROR in chunk callback ({z},{y},{x}): {e}")
                    with cache_lock:
                        pending_loads.discard((z, y, x))

            def load_chunk(z, y, x):
                """Load a chunk (non-blocking - returns cached or None and starts background load)"""
                # Debug: print all requests
                print(f"  load_chunk({z},{y},{x}) called from Rust")

                # Check cache first
                with cache_lock:
                    if (z, y, x) in chunk_cache:
                        print(f"    -> returning cached chunk")
                        return chunk_cache[(z, y, x)]

                    # If not cached and not already loading, start background load
                    if (z, y, x) not in pending_loads:
                        print(f"    -> starting background load")
                        pending_loads.add((z, y, x))
                        future = executor.submit(load_chunk_blocking, z, y, x)
                        future.add_done_callback(lambda f: on_chunk_loaded(z, y, x, f))
                    else:
                        print(f"    -> already loading")

                # Return None for now - chunk will appear when ready
                return None

            # Kick off loading of initial visible chunks in background
            # Don't wait for them - they'll appear when ready
            print("Starting background loading of initial chunks...")
            center_z = volume_shape[0] // 2
            initial_chunks = [(center_z, 0, 0)]  # Just the center Z slice

            for z, y, x in initial_chunks:
                # Trigger async load by calling load_chunk (won't block)
                load_chunk(z, y, x)

            # Create tiled image visual
            volume_size = volume_shape  # Already calculated above
            chunk_size = chunk_shape

            grid_z = (volume_size[0] + chunk_size[0] - 1) // chunk_size[0]
            grid_y = (volume_size[1] + chunk_size[1] - 1) // chunk_size[1]
            grid_x = (volume_size[2] + chunk_size[2] - 1) // chunk_size[2]

            print(f"Creating TiledImageVisual:")
            print(f"  Volume size (z,y,x): {volume_size}")
            print(f"  Chunk size (z,y,x): {chunk_size}")
            print(f"  Grid will be: {grid_z}z × {grid_y}y × {grid_x}x chunks")

            # Warn about highly anisotropic chunks
            if chunk_size[0] < 10 and (chunk_size[1] > 100 or chunk_size[2] > 100):
                print(f"  WARNING: Highly anisotropic chunks detected!")
                print(f"  Data has very thin Z slices ({chunk_size[0]} voxels)")
                print(f"  This may appear as a thin strip in the viewer")
                print(f"  Try using a lower resolution level for better visualization")

            tiled_image = bv.PyTiledImageVisual.from_loader(
                viewer, volume_size, chunk_size, load_chunk, max_loaded_chunks=150
            )

            # Set initial slice plane to show XY plane (looking down Z axis)
            # Center is (x, y, z) in world space
            center = [volume_size[2] / 2, volume_size[1] / 2, volume_size[0] / 2]
            self.bovista_widget.volume_center = center

            # Slice plane normal pointing up in Z (shows XY slice)
            # Position at middle Z to show a cross-section
            tiled_image.set_slice_plane(
                center[0],  # x position
                center[1],  # y position
                center[2],  # z position (middle of volume)
                0.0,  # normal x
                0.0,  # normal y
                1.0,  # normal z (pointing up, shows XY plane)
            )

            # Set initial contrast to use percentiles for better initial display
            # Chunks are normalized using full data range, but we window to percentiles
            data_min, data_max = self.data_range

            # Map percentiles to [0,1] range for shader (R8Unorm textures are normalized)
            if data_max > data_min:
                contrast_min_normalized = (global_min - data_min) / (data_max - data_min)
                contrast_max_normalized = (global_max - data_min) / (data_max - data_min)

                # Set slider positions to match the percentiles
                min_slider_pos = int((global_min - data_min) / (data_max - data_min) * 1000)
                max_slider_pos = int((global_max - data_min) / (data_max - data_min) * 1000)
            else:
                # Fallback if data is constant
                contrast_min_normalized = 0.0
                contrast_max_normalized = 1.0
                min_slider_pos = 0
                max_slider_pos = 1000

            tiled_image.set_contrast(contrast_min_normalized, contrast_max_normalized)

            self.contrast_min_slider.blockSignals(True)
            self.contrast_max_slider.blockSignals(True)
            self.contrast_min_slider.setValue(min_slider_pos)
            self.contrast_max_slider.setValue(max_slider_pos)
            self.contrast_min_slider.blockSignals(False)
            self.contrast_max_slider.blockSignals(False)

            self.contrast_min_label.setText(f"{global_min:.1f}")
            self.contrast_max_label.setText(f"{global_max:.1f}")

            # Debug mode off by default (press 'D' to toggle)
            tiled_image.set_debug_mode(False)

            print(f"Initial setup:")
            print(f"  Center: {center}")
            print(f"  Slice plane position: ({center[0]}, {center[1]}, {center[2]})")
            print(f"  Slice plane normal: (0, 0, 1)")
            print(f"  Expected chunk index: Z={int(center[2])} (for 1-voxel chunks)")
            print(f"  Contrast: 0-255")
            print(f"  Debug mode: ENABLED (shows chunk boundaries)")
            print(f"  Press 'D' to toggle debug mode off/on")
            print(f"\nWaiting for chunks to load...")

            # Setup camera to look down at XY plane from above
            # Use XY dimensions for distance calculation (not Z since it's thin)
            xy_max = max(volume_size[1], volume_size[2])  # max of Y and X
            distance = xy_max * 1.0  # Position camera far enough to see whole slice

            # Camera above the center, looking down
            viewer.set_camera_position(center[0], center[1], center[2] + distance)
            viewer.set_camera_target(center[0], center[1], center[2])

            print(f"Camera setup (top-down view):")
            print(f"  Position: ({center[0]}, {center[1]}, {center[2] + distance})")
            print(f"  Target: ({center[0]}, {center[1]}, {center[2]})")
            print(f"  Distance: {distance}")
            print(f"  View: Looking down Z axis at XY slice")

            # Add to scene
            viewer.add_tiled_image(tiled_image)
            self.bovista_widget.tiled_image = tiled_image

            # Add axes
            axes = bv.PyLinesVisual.axis_helper(viewer, max(volume_size) * 0.3)
            viewer.add_lines(axes)

            print("Scene loaded successfully!")
            print("Chunks will load on-demand as you navigate.")

        except Exception as e:
            print(f"Error setting up scene: {e}")
            import traceback

            traceback.print_exc()

    def change_resolution(self, index):
        """Change the resolution level"""
        if self.zarr_data is None or index < 0:
            return

        resolution_level = self.resolution_combo.itemData(index)
        if resolution_level is None:
            return

        print(f"\nChanging to resolution level {resolution_level}")

        # Clean up old visual if it exists
        if self.bovista_widget.tiled_image is not None:
            print("Clearing old tiled image visual...")
            # Note: Currently we don't have a remove method, so we just
            # release our reference. The old visual will remain in the scene
            # but won't be updated. TODO: Add scene.remove() or scene.clear()
            self.bovista_widget.tiled_image = None
            self.stats_label.setText("Loading new resolution...")

        self.current_resolution = resolution_level

        # Setup scene with new resolution (ensure viewer is ready)
        def setup_with_resolution(viewer):
            self.setup_scene(viewer, self.dataset_info)

        if self.bovista_widget._initialized:
            setup_with_resolution(self.bovista_widget.viewer)
        else:
            self.bovista_widget.on_ready(setup_with_resolution)

    def update_contrast(self):
        """Update contrast range based on slider values"""
        try:
            print(f"update_contrast called, tiled_image={self.bovista_widget.tiled_image is not None}")

            if self.bovista_widget.tiled_image is None:
                print("  -> tiled_image is None, returning early")
                return

            # Get slider positions (0-1000)
            min_pos = self.contrast_min_slider.value()
            max_pos = self.contrast_max_slider.value()
            print(f"  slider values: min={min_pos}, max={max_pos}")

            # Ensure min < max
            if min_pos >= max_pos:
                # Adjust the slider that was just moved
                sender = self.sender()
                if sender == self.contrast_min_slider:
                    min_pos = max_pos - 1
                    self.contrast_min_slider.blockSignals(True)
                    self.contrast_min_slider.setValue(min_pos)
                    self.contrast_min_slider.blockSignals(False)
                else:
                    max_pos = min_pos + 1
                    self.contrast_max_slider.blockSignals(True)
                    self.contrast_max_slider.setValue(max_pos)
                    self.contrast_max_slider.blockSignals(False)

            # Sliders map directly to the data range
            # Chunks are normalized to 0-255 using data_min to data_max
            # So slider position maps to a value in the data range,
            # which then maps to a position in the 0-255 uint8 range
            data_min, data_max = self.data_range
            print(f"  data_range: ({data_min}, {data_max})")

            if data_max > data_min:
                # Map slider to data values
                contrast_min_data = data_min + (data_max - data_min) * (min_pos / 1000.0)
                contrast_max_data = data_min + (data_max - data_min) * (max_pos / 1000.0)

                # Update labels
                self.contrast_min_label.setText(f"{contrast_min_data:.1f}")
                self.contrast_max_label.setText(f"{contrast_max_data:.1f}")

                # Map to 0-1 range for the shader (R8Unorm texture values are normalized to [0,1])
                # Chunks are stored as uint8, but the texture format (R8Unorm) normalizes them to [0,1]
                # So we need to pass contrast limits in [0,1] range
                contrast_min_normalized = (contrast_min_data - data_min) / (data_max - data_min)
                contrast_max_normalized = (contrast_max_data - data_min) / (data_max - data_min)

                print(f"  Contrast: data=({contrast_min_data:.1f}, {contrast_max_data:.1f}), normalized=({contrast_min_normalized:.3f}, {contrast_max_normalized:.3f})")
            else:
                # Fallback if data is constant
                self.contrast_min_label.setText(f"{data_min:.1f}")
                self.contrast_max_label.setText(f"{data_max:.1f}")
                contrast_min_normalized = 0.0
                contrast_max_normalized = 1.0
                print(f"  Using fallback contrast (data constant)")

            # Apply to tiled image
            print(f"  Calling set_contrast({contrast_min_normalized:.3f}, {contrast_max_normalized:.3f})")
            self.bovista_widget.tiled_image.set_contrast(contrast_min_normalized, contrast_max_normalized)
            print(f"  set_contrast completed")
        except Exception as e:
            print(f"ERROR in update_contrast: {e}")
            import traceback
            traceback.print_exc()

    def update_stats(self):
        """Update statistics display"""
        if self.bovista_widget.tiled_image:
            try:
                loaded, visible = self.bovista_widget.tiled_image.get_stats()
                res_level = self.current_resolution
                self.stats_label.setText(
                    f"Resolution {res_level} | Chunks: {loaded} loaded, {visible} visible"
                )
            except Exception:
                pass


def main():
    """Main entry point"""
    print("=" * 70)
    print("Bovista Remote OME-Zarr Viewer")
    print("=" * 70)
    print("\nThis example demonstrates cloud-native visualization:")
    print("  - Loads multiscale OME-Zarr from remote S3")
    print("  - Only downloads visible chunks (efficient!)")
    print("  - Switch between resolution levels")
    print("  - Navigate TB-scale datasets interactively")
    print("\nRequirements:")
    print("  pip install PySide6 zarr fsspec requests aiohttp")
    print("=" * 70)

    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
