"""
Bovista + Remote Multiscale OME-Zarr Example (AUTOMATIC LOD SELECTION)

This example demonstrates loading and visualizing a real multiscale OME-Zarr dataset
from a remote S3 bucket with AUTOMATIC spatial LOD selection. This showcases:

1. **Cloud-native access**: Load data directly from S3 via HTTP
2. **Spatial LOD Selection**: Automatically uses high-res near camera, low-res farther away
3. **Mixed-resolution rendering**: Multiple LOD levels displayed simultaneously
4. **Multi-channel support**: View different fluorescence channels
5. **On-demand chunk loading**: Only load visible data (efficient memory usage)
6. **Interactive navigation**: Orbit, zoom, slice plane manipulation
7. **Contrast adjustment**: Real-time intensity windowing
8. **TB-scale datasets**: Navigate datasets too large to fit in memory

Features:
- Multiple remote datasets to choose from
- **Automatic LOD selection** based on camera distance and screen resolution
- **LOD Bias slider**: Adjust preference for high-res vs low-res
- Channel selector (for multi-channel data)
- Contrast min/max sliders
- **LOD statistics display**: See how many chunks loaded from each LOD level
- Debug mode (press 'D' key) to visualize chunk boundaries

Controls:
- Left-drag: Orbit camera around slice
- Right-drag vertical: Rotate slice plane around Y axis
- Shift + Right-drag vertical: Rotate slice plane around X axis
- Right-drag horizontal: Move slice offset along Z
- Scroll: Zoom in/out
- D key: Toggle debug mode (shows chunk boundaries)

Dataset: IDR (Image Data Resource) public OME-Zarr collection
Source: https://www.openmicroscopy.org/2020/11/04/zarr-data.html

Requirements:
    pip install PySide6 zarr fsspec requests aiohttp bovista
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
from PySide6.QtCore import QTimer, Qt, Signal, QObject
import bovista as bv
from concurrent.futures import ThreadPoolExecutor
from threading import Lock


# Public IDR OME-Zarr datasets on S3
DATASETS = {
    "Human Mitotic Cell (small)": {
        "url": "https://uk1s3.embassy.ebi.ac.uk/idr/zarr/v0.4/idr0062A/6001240.zarr",
        "description": "HeLa cells, 3 resolution levels",
        "camera_distance": 200.0,
    },
    "Platybrowser (medium)": {
        "url": "https://s3.embl.de/i2k-2020/platy-raw.ome.zarr",
        "description": "Platynereis embryo, 10 resolution levels",
        "camera_distance": 400.0,
    },
}


class BovistaWidget(QWidget):
    """Qt widget that embeds a Bovista 3D viewer.

    This widget handles the Bovista rendering lifecycle:
    - Initializes GPU resources when first shown
    - Manages render loop with QTimer
    - Handles mouse/keyboard input for camera control
    - Supports slice plane manipulation

    Attributes:
        viewer: Viewer instance for GPU rendering
        tiled_image: Currently displayed TiledImage (if any)
        debug_mode: Whether to show chunk boundaries
        slice_angle_x/y: Current slice plane rotation angles
        slice_offset: Current slice plane offset
        volume_center: Center point of the volume
    """

    def __init__(self, parent=None, width=800, height=600):
        super().__init__(parent)
        self.setMinimumSize(width, height)
        self.viewer = bv.Viewer(width, height)
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
        self._tiled_image = None
        self.debug_mode = False
        self.volume_center = [0, 0, 0]

        # Use ClickFocus instead of StrongFocus to avoid stealing focus from dropdowns
        # StrongFocus would grab focus during tab navigation which can interfere
        # with other widgets (especially on macOS with native window handles)
        self.setFocusPolicy(Qt.ClickFocus)

        self.timer = QTimer(self)
        self.timer.timeout.connect(self._render_loop)
        self.timer.start(16)

    @property
    def tiled_image(self):
        return self._tiled_image

    @tiled_image.setter
    def tiled_image(self, value):
        print("BovistaWidget: Setting new tiled_image")
        self._tiled_image = value
        if self._tiled_image:
            self._update_slice_plane()

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
                if self._tiled_image:
                    self._tiled_image.prepare_chunks(self.viewer)
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
        elif self.right_mouse_pressed and self._initialized and self._tiled_image:
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
        if event.key() == Qt.Key_D:
            self.debug_mode = not self.debug_mode
            if self._tiled_image:
                self._tiled_image.set_debug_mode(self.debug_mode)
                print(f"Debug mode: {'ON' if self.debug_mode else 'OFF'}")
        event.accept()

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)

    def _update_slice_plane(self):
        if not self._tiled_image:
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
        self._tiled_image.set_slice_plane(
            center[0],
            center[1],
            center[2] + self.slice_offset,
            normal_x,
            normal_y,
            normal_z,
        )


class MainWindow(QMainWindow):
    """Main application window for remote OME-Zarr visualization.

    Provides UI controls for:
    - Dataset selection (from predefined list of public datasets)
    - Resolution level (LOD) selection
    - Channel selection (for multi-channel data)
    - Contrast adjustment
    - Statistics display

    The window manages asynchronous loading of Zarr metadata and chunks,
    using ThreadPoolExecutor to avoid blocking the UI thread.
    """

    # Signal for thread-safe communication from background threads to main GUI thread
    metadata_loaded = Signal(object, object, object, str)  # store, level_info, dataset_info, dataset_name
    metadata_error = Signal(str)  # error message

    def __init__(self):
        super().__init__()
        self.setWindowTitle("Bovista Remote OME-Zarr Viewer")
        self.setGeometry(100, 100, 1000, 700)

        # Connect signals to slots for thread-safe UI updates
        self.metadata_loaded.connect(self._on_metadata_loaded)
        self.metadata_error.connect(self._on_metadata_error)

        self.zarr_data = None
        self.level_info = []  # List of LOD levels
        self.current_channel = 0
        self.data_range = (0.0, 255.0)  # Will be updated when data loads
        self.dataset_info = None  # Currently loaded dataset info
        self.camera_initialized = False  # Track if camera has been positioned
        self.chunk_executor = None  # ThreadPoolExecutor for chunk loading

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

        # LOD Bias and channel selectors
        res_layout = QHBoxLayout()
        res_layout.addWidget(QLabel("LOD Bias:"))

        self.lod_bias_slider = QSlider(Qt.Horizontal)
        self.lod_bias_slider.setMinimum(-20)  # -2.0
        self.lod_bias_slider.setMaximum(20)   # +2.0
        self.lod_bias_slider.setValue(0)      # 0.0
        self.lod_bias_slider.setEnabled(False)
        self.lod_bias_slider.valueChanged.connect(self.update_lod_bias)
        res_layout.addWidget(self.lod_bias_slider)

        self.lod_bias_label = QLabel("0.0")
        self.lod_bias_label.setMinimumWidth(40)
        res_layout.addWidget(self.lod_bias_label)

        res_layout.addWidget(QLabel("Channel:"))

        self.channel_combo = QComboBox()
        self.channel_combo.currentIndexChanged.connect(self.change_channel)
        self.channel_combo.setEnabled(False)
        res_layout.addWidget(self.channel_combo)

        res_layout.addStretch()
        layout.addLayout(res_layout)

        # Contrast controls
        contrast_layout = QHBoxLayout()
        contrast_layout.addWidget(QLabel("Contrast Min:"))

        self.contrast_min_slider = QSlider(Qt.Horizontal)
        self.contrast_min_slider.setMinimum(0)
        self.contrast_min_slider.setMaximum(255)
        self.contrast_min_slider.setValue(0)
        self.contrast_min_slider.valueChanged.connect(self.update_contrast)
        contrast_layout.addWidget(self.contrast_min_slider)

        self.contrast_min_label = QLabel("0")
        self.contrast_min_label.setMinimumWidth(50)
        contrast_layout.addWidget(self.contrast_min_label)

        contrast_layout.addWidget(QLabel("Max:"))

        self.contrast_max_slider = QSlider(Qt.Horizontal)
        self.contrast_max_slider.setMinimum(0)
        self.contrast_max_slider.setMaximum(255)
        self.contrast_max_slider.setValue(255)
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

    def closeEvent(self, event):
        """Clean up resources when window closes"""
        if self.chunk_executor is not None:
            print("Shutting down chunk loader...")
            # Cancel all pending futures
            self.chunk_executor.shutdown(wait=False, cancel_futures=True)
        if hasattr(self, "_metadata_executor") and self._metadata_executor is not None:
            self._metadata_executor.shutdown(wait=False, cancel_futures=True)

        super().closeEvent(event)

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
                    # Emit signal to main thread (thread-safe)
                    self.metadata_error.emit(error or 'Unknown error')
                    return

                # Emit signal to main thread (thread-safe)
                self.metadata_loaded.emit(store, level_info, dataset_info, dataset_name)

            except Exception as e:
                # Emit error signal to main thread (thread-safe)
                self.metadata_error.emit(str(e))

        # Submit to thread pool
        if not hasattr(self, "_metadata_executor"):
            self._metadata_executor = ThreadPoolExecutor(
                max_workers=1, thread_name_prefix="metadata_loader"
            )

        future = self._metadata_executor.submit(load_metadata)
        future.add_done_callback(on_metadata_loaded)

    def _on_metadata_loaded(self, store, level_info, dataset_info, dataset_name):
        """Slot called on main thread when metadata is loaded"""
        self.process_loaded_store(store, level_info, dataset_info, dataset_name)

    def _on_metadata_error(self, error):
        """Slot called on main thread when metadata loading fails"""
        self.stats_label.setText(f"Error: {error}")
        print(f"Error loading dataset: {error}")
        self.load_button.setEnabled(True)

    def process_loaded_store(self, store, level_info, dataset_info, dataset_name):
        """Process loaded zarr store (on main thread)"""
        try:
            self.zarr_data = store
            self.dataset_info = dataset_info
            self.level_info = level_info  # Store for multi-LOD setup

            # Detect channels from the first resolution level
            multiscales = store.attrs.get("multiscales", [{}])[0]
            axes = multiscales.get("axes", [])
            axis_names = [ax.get("name", "?").lower() for ax in axes]

            # Check if there's a channel axis
            if "c" in axis_names:
                channel_idx = axis_names.index("c")
                # Get shape from first resolution level
                first_path = multiscales["datasets"][0]["path"]
                first_array = store[first_path]
                num_channels = first_array.shape[channel_idx]

                # Populate channel combo (block signals to avoid triggering change handler)
                self.channel_combo.blockSignals(True)
                self.channel_combo.clear()
                for i in range(num_channels):
                    self.channel_combo.addItem(f"Channel {i}", i)
                # Default to last channel
                last_channel = num_channels - 1
                self.channel_combo.setCurrentIndex(last_channel)
                self.channel_combo.blockSignals(False)
                self.channel_combo.setEnabled(True)
                self.current_channel = last_channel
            else:
                # Single channel dataset
                self.channel_combo.blockSignals(True)
                self.channel_combo.clear()
                self.channel_combo.addItem("Channel 0", 0)
                self.channel_combo.setCurrentIndex(0)
                self.channel_combo.blockSignals(False)
                self.channel_combo.setEnabled(False)
                self.current_channel = 0

            # Reset camera flag when loading new dataset
            self.camera_initialized = False

            # Enable LOD bias slider
            self.lod_bias_slider.setEnabled(True)

            self.stats_label.setText(f"Loaded: {dataset_name} ({len(level_info)} LOD levels) - Loading channel {self.current_channel}")

            # Automatically load with multi-LOD
            if self.bovista_widget._initialized:
                self.setup_scene(self.bovista_widget.viewer, self.dataset_info)
            else:
                self.bovista_widget.on_ready(lambda viewer: self.setup_scene(viewer, self.dataset_info))

        except Exception as e:
            self.stats_label.setText(f"Error loading dataset: {e}")
            print(f"Error: {e}")
            import traceback

            traceback.print_exc()
        finally:
            self.load_button.setEnabled(True)

    def setup_scene(self, viewer, dataset_info):
        """Setup the visualization with multi-LOD chunked loading"""
        if self.zarr_data is None or not self.level_info:
            return

        viewer.clear_visuals()

        try:
            multiscales = self.zarr_data.attrs["multiscales"][0]
            axes = multiscales.get("axes", [])
            axis_names = [ax.get("name", "?").lower() for ax in axes]

            print(f"\nSetting up multi-LOD visualization with {len(self.level_info)} resolution levels")
            print(f"  Axes metadata: {axis_names}")

            # Find indices of spatial dimensions (z, y, x)
            spatial_axes = {}
            for i, name in enumerate(axis_names):
                if name in ["z", "y", "x"]:
                    spatial_axes[name] = i

            # Extract spatial indices
            if len(spatial_axes) == 3:
                z_idx = spatial_axes["z"]
                y_idx = spatial_axes["y"]
                x_idx = spatial_axes["x"]
                spatial_indices = (z_idx, y_idx, x_idx)
            else:
                # Fallback for datasets without proper axes metadata
                first_path = multiscales["datasets"][0]["path"]
                first_array = self.zarr_data[first_path]
                ndim = len(first_array.shape)
                if ndim == 5:
                    spatial_indices = (2, 3, 4)
                elif ndim == 4:
                    spatial_indices = (1, 2, 3)
                else:
                    spatial_indices = (0, 1, 2)

            print(f"  Spatial indices (z, y, x): {spatial_indices}")

            # Create LevelMetadata for each LOD level
            lod_levels = []
            lod0_voxel_spacing = None

            for lod_idx, level_info in enumerate(self.level_info):
                dataset_path = multiscales["datasets"][lod_idx]["path"]
                array = self.zarr_data[dataset_path]

                shape = array.shape
                chunks = array.chunks

                # Extract spatial dimensions
                z_idx, y_idx, x_idx = spatial_indices
                volume_shape = (shape[z_idx], shape[y_idx], shape[x_idx])
                chunk_shape = (chunks[z_idx], chunks[y_idx], chunks[x_idx])

                # Extract voxel spacing from OME-Zarr coordinate transforms
                dataset_transforms = multiscales["datasets"][lod_idx].get("coordinateTransformations", [])
                voxel_spacing = [1.0, 1.0, 1.0]  # Default
                for transform in dataset_transforms:
                    if transform.get("type") == "scale":
                        scale = transform.get("scale", [])
                        if len(scale) > max(z_idx, y_idx, x_idx):
                            voxel_spacing = [scale[z_idx], scale[y_idx], scale[x_idx]]

                # Calculate scale factor relative to LOD 0
                if lod_idx == 0:
                    lod0_voxel_spacing = voxel_spacing
                    scale_factor = 1.0
                else:
                    # Scale factor is the ratio of voxel sizes
                    scale_factor = voxel_spacing[0] / lod0_voxel_spacing[0]

                # Create LevelMetadata
                level_metadata = bv.LevelMetadata(
                    volume_size=volume_shape,
                    chunk_size=chunk_shape,
                    voxel_size=tuple(voxel_spacing),
                    scale_factor=scale_factor
                )
                lod_levels.append(level_metadata)

                print(f"\n  LOD {lod_idx}:")
                print(f"    Volume shape (z,y,x): {volume_shape}")
                print(f"    Chunk shape: {chunk_shape}")
                print(f"    Voxel spacing: {voxel_spacing}")
                print(f"    Scale factor: {scale_factor:.2f}x")

            # Store first level info for camera setup
            lod0_array = self.zarr_data[multiscales["datasets"][0]["path"]]
            lod0_shape = lod0_array.shape
            lod0_volume_shape = lod_levels[0].volume_size
            lod0_voxel_spacing = lod_levels[0].voxel_size

            # Skip contrast sampling - we use per-chunk normalization
            print("\nUsing per-chunk normalization (each chunk normalized to its own min/max)")

            # Set simple defaults based on dtype of LOD 0
            if lod0_array.dtype == np.uint16:
                data_min, data_max = 0.0, 65535.0
            elif lod0_array.dtype == np.uint8:
                data_min, data_max = 0.0, 255.0
            else:
                data_min, data_max = 0.0, 1.0

            self.data_range = (data_min, data_max)
            print(f"  Data type: {lod0_array.dtype}")
            print(f"  Assumed data range: {data_min:.1f} to {data_max:.1f}")

            # Track chunk loading stats
            loaded_count = [0]  # Use list to allow mutation in closure

            # Cache for loaded chunks and pending loads
            chunk_cache = {}  # (lod_level, z, y, x) -> numpy array
            pending_futures = {}  # (lod_level, z, y, x) -> Future
            cache_lock = Lock()

            # Clean up old executor if it exists
            if self.chunk_executor is not None:
                print("Shutting down previous chunk loader...")
                self.chunk_executor.shutdown(wait=False)  # Don't wait for pending tasks

            # Thread pool for async loading
            self.chunk_executor = ThreadPoolExecutor(
                max_workers=8, thread_name_prefix="zarr_loader"
            )
            executor = self.chunk_executor  # Local alias for closure

            def load_chunk_blocking(lod_level, z, y, x):
                """Actually load a chunk from remote Zarr (blocking I/O)"""
                try:
                    # DEBUG: Print requested chunk
                    print(f"  [LOADER] Requesting LOD{lod_level} chunk ({z},{y},{x})")

                    # Get the array for this LOD level
                    dataset_path = multiscales["datasets"][lod_level]["path"]
                    array = self.zarr_data[dataset_path]

                    # Get dimensions for this LOD level
                    level_meta = lod_levels[lod_level]
                    volume_shape = level_meta.volume_size
                    chunk_shape = level_meta.chunk_size

                    shape = array.shape

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

                    # For non-spatial dimensions, select appropriate index
                    for i in range(len(shape)):
                        if i not in spatial_indices:
                            # Check if this is the channel dimension
                            if "c" in axis_names and i == axis_names.index("c"):
                                slices[i] = self.current_channel  # Use selected channel
                            else:
                                slices[i] = 0  # Take first element for other dimensions (e.g., time)

                    # Set spatial slices
                    slices[z_idx] = slice(z_start, z_end)
                    slices[y_idx] = slice(y_start, y_end)
                    slices[x_idx] = slice(x_start, x_end)

                    # Load from remote zarr (BLOCKING I/O)
                    chunk_data = array[tuple(slices)]

                    # Check if we got empty data
                    if chunk_data.size == 0:
                        return None

                    # Convert to uint8 using PER-CHUNK normalization (like web version)
                    # This ensures each chunk shows its full dynamic range
                    if chunk_data.dtype != np.uint8:
                        chunk_min = float(chunk_data.min())
                        chunk_max = float(chunk_data.max())

                        if chunk_max > chunk_min:
                            # Normalize using this chunk's min/max
                            chunk_data = (
                                (chunk_data - chunk_min)
                                / (chunk_max - chunk_min)
                                * 255
                            ).astype(np.uint8)
                        else:
                            # Chunk is constant
                            chunk_data = np.zeros_like(chunk_data, dtype=np.uint8)

                    return np.array(chunk_data)
                except Exception as e:
                    print(f"ERROR loading chunk LOD{lod_level} ({z},{y},{x}): {e}")
                    return None

            def on_chunk_loaded(lod_level, z, y, x, future):
                """Callback when chunk loading completes"""
                chunk_key = (lod_level, z, y, x)
                try:
                    # Check if the future was cancelled
                    if future.cancelled():
                        with cache_lock:
                            pending_futures.pop(chunk_key, None)
                        return

                    result = future.result()
                    with cache_lock:
                        pending_futures.pop(chunk_key, None)
                        if result is not None:
                            chunk_cache[chunk_key] = result
                            loaded_count[0] += 1
                            # Print first 5 chunks, then every 50th chunk
                            count = loaded_count[0]
                            if count <= 5 or (count > 5 and (count - 5) % 50 == 0):
                                print(
                                    f"  Chunk LOD{lod_level} ({z},{y},{x}) loaded (total: {count})"
                                )
                except Exception as e:
                    # Silently ignore errors from cancelled futures
                    from concurrent.futures import CancelledError
                    if not isinstance(e, CancelledError):
                        print(f"  ERROR in chunk callback LOD{lod_level} ({z},{y},{x}): {e}")
                    with cache_lock:
                        pending_futures.pop(chunk_key, None)

            def load_chunk(lod_level, z, y, x):
                """Load a chunk (non-blocking - returns cached or None and starts background load)"""
                chunk_key = (lod_level, z, y, x)

                # Check cache first
                with cache_lock:
                    if chunk_key in chunk_cache:
                        return chunk_cache[chunk_key]

                    # If not cached and not already loading, start background load
                    if chunk_key not in pending_futures:
                        future = executor.submit(load_chunk_blocking, lod_level, z, y, x)
                        future.add_done_callback(lambda f: on_chunk_loaded(lod_level, z, y, x, f))
                        pending_futures[chunk_key] = future

                # Return None for now - chunk will appear when ready
                return None

            def get_pending_count():
                """Return the number of pending chunk load requests (for backpressure control)"""
                with cache_lock:
                    return len(pending_futures)

            # Kick off loading of initial visible chunks in background (LOD 0)
            print("\nStarting background loading of initial chunks...")
            center_z_voxel = lod0_volume_shape[0] // 2
            center_z_chunk = center_z_voxel // lod_levels[0].chunk_size[0]
            initial_chunks = [(0, center_z_chunk, 0, 0)]  # LOD 0, center Z slice

            for lod, z, y, x in initial_chunks:
                # Trigger async load by calling load_chunk (won't block)
                load_chunk(lod, z, y, x)

            # Calculate world-space dimensions using LOD 0
            volume_size_world = (
                lod0_volume_shape[0] * lod0_voxel_spacing[0],  # Z
                lod0_volume_shape[1] * lod0_voxel_spacing[1],  # Y
                lod0_volume_shape[2] * lod0_voxel_spacing[2],  # X
            )

            # Center in world coordinates
            center = [
                volume_size_world[2] / 2,  # X
                volume_size_world[1] / 2,  # Y
                volume_size_world[0] / 2,  # Z
            ]

            # Calculate grid size for LOD 0 (for info only)
            lod0_chunk_shape = lod_levels[0].chunk_size
            grid_z = (lod0_volume_shape[0] + lod0_chunk_shape[0] - 1) // lod0_chunk_shape[0]
            grid_y = (lod0_volume_shape[1] + lod0_chunk_shape[1] - 1) // lod0_chunk_shape[1]
            grid_x = (lod0_volume_shape[2] + lod0_chunk_shape[2] - 1) // lod0_chunk_shape[2]

            print(f"\nCreating Multi-LOD TiledImageVisual:")
            print(f"  LOD 0 volume size (world): {volume_size_world}")
            print(f"  LOD 0 grid: {grid_z}z × {grid_y}y × {grid_x}x chunks")
            print(f"  All LODs will render at the same physical size")
            print(f"  Spatial LOD selection enabled (automatic)")

            # Create TiledImage with multi-LOD support
            tiled_image = bv.TiledImage.from_levels(
                viewer,
                lod_levels,  # List of LevelMetadata
                load_chunk,  # Loader with (lod_level, z, y, x) signature
                max_loaded_chunks=500,  # Higher limit for multi-LOD
                lod_bias=0.0,  # Neutral bias (can adjust with slider)
                target_pixels_per_voxel=1.0  # Target ~1 pixel per voxel
            )

            # Set up backpressure control - limit requests based on queue size
            # Keep queue size equal to worker count so there's no backlog
            # This makes the system responsive: old requests complete quickly (8 chunks max),
            # and new requests for correct LOD can start immediately
            NUM_WORKERS = 8
            MAX_QUEUE_SIZE = NUM_WORKERS
            def check_capacity():
                """Return how many more chunk requests can be accepted"""
                pending = get_pending_count()
                capacity = max(0, MAX_QUEUE_SIZE - pending)
                return capacity

            tiled_image.set_capacity_check(check_capacity)
            print(f"  Backpressure control enabled (queue size: {MAX_QUEUE_SIZE}, workers: {NUM_WORKERS})")

            # Set initial slice plane to show XY plane (looking down Z axis)
            # Slice plane normal pointing up in Z (shows XY slice)
            # Position at middle Z to show a cross-section
            # (center was already calculated in world coordinates above)
            tiled_image.set_slice_plane(
                center[0],  # x position
                center[1],  # y position
                center[2],  # z position (middle of volume)
                0.0,  # normal x
                0.0,  # normal y
                1.0,  # normal z (pointing up, shows XY plane)
            )

            # Set initial contrast to full range (0-255) like web version
            # Chunks are normalized per-chunk, so full range shows everything
            tiled_image.set_contrast(0.0, 1.0)

            self.contrast_min_slider.blockSignals(True)
            self.contrast_max_slider.blockSignals(True)
            self.contrast_min_slider.setValue(0)
            self.contrast_max_slider.setValue(255)
            self.contrast_min_slider.blockSignals(False)
            self.contrast_max_slider.blockSignals(False)

            self.contrast_min_label.setText("0")
            self.contrast_max_label.setText("255")

            # Debug mode off by default (press 'D' to toggle)
            tiled_image.set_debug_mode(False)

            print(f"\nInitial setup:")
            print(f"  World center: {center}")
            print(f"  Slice plane position: ({center[0]}, {center[1]}, {center[2]})")
            print(f"  Slice plane normal: (0, 0, 1)")
            print(f"  Contrast: 0-255")
            print(f"  Debug mode: OFF (press 'D' to toggle)")
            print(f"  LOD selection: AUTOMATIC (use LOD Bias slider to adjust)")
            print(f"\nWaiting for chunks to load...")

            # Update volume center for slice plane calculations (in world space)
            self.bovista_widget.volume_center = center

            # Setup camera only on first load, not on channel changes
            if not self.camera_initialized:
                # Setup camera to look down at XY plane from above
                # Use XY dimensions for distance calculation (not Z since it's thin)
                xy_max = max(volume_size_world[1], volume_size_world[2])  # max of Y and X in world space
                distance = xy_max * 1.0  # Position camera far enough to see whole slice

                # Camera above the center, looking down (in world coordinates)
                viewer.set_camera_position(center[0], center[1], center[2] + distance)
                viewer.set_camera_target(center[0], center[1], center[2])

                self.camera_initialized = True

                print(f"\nCamera setup (top-down view):")
                print(f"  Position: ({center[0]}, {center[1]}, {center[2] + distance})")
                print(f"  Target: ({center[0]}, {center[1]}, {center[2]})")
                print(f"  Distance: {distance}")
                print(f"  View: Looking down Z axis at XY slice")
            else:
                print("\nCamera position preserved from previous view")

            # Add to scene (new unified API)
            viewer.add(tiled_image)
            self.bovista_widget.tiled_image = tiled_image

            # Add axes (new unified API)
            axes = bv.Lines.axis_helper(viewer, max(lod0_volume_shape) * 0.3)
            viewer.add(axes)

            print("Scene loaded successfully!")
            print("Chunks will load on-demand as you navigate.")

        except Exception as e:
            print(f"Error setting up scene: {e}")
            import traceback

            traceback.print_exc()

    def update_lod_bias(self):
        """Update LOD bias for spatial LOD selection"""
        if self.bovista_widget._tiled_image is None:
            return

        # Slider value is -20 to 20, map to -2.0 to 2.0
        bias_value = self.lod_bias_slider.value() / 10.0
        self.lod_bias_label.setText(f"{bias_value:.1f}")

        # Update the TiledImage visual's LOD bias
        self.bovista_widget._tiled_image.set_lod_bias(bias_value)
        print(f"LOD Bias: {bias_value:.1f} (negative = prefer high-res, positive = prefer low-res)")

    def change_channel(self, index):
        """Change the displayed channel"""
        if self.zarr_data is None or index < 0:
            return

        channel = self.channel_combo.itemData(index)
        if channel is None:
            return

        print(f"\nChanging to channel {channel}")

        self.current_channel = channel

        # Setup scene directly
        if self.bovista_widget._initialized:
            self.setup_scene(self.bovista_widget.viewer, self.dataset_info)
        else:
            self.bovista_widget.on_ready(lambda viewer: self.setup_scene(viewer, self.dataset_info))

    def update_contrast(self):
        """Update contrast range based on slider values (simple 0-255 like web version)"""
        try:
            if self.bovista_widget._tiled_image is None:
                return

            # Get slider values (0-255)
            min_pos = self.contrast_min_slider.value()
            max_pos = self.contrast_max_slider.value()

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

            # Simple 0-255 mapping like web version
            # Chunks are normalized per-chunk, so we map slider values directly
            # Update labels
            self.contrast_min_label.setText(str(min_pos))
            self.contrast_max_label.setText(str(max_pos))

            # Map 0-255 to normalized 0-1 range for shader
            # (R8Unorm textures are normalized to [0,1])
            contrast_min_normalized = min_pos / 255.0
            contrast_max_normalized = max_pos / 255.0

            # Apply to tiled image
            self.bovista_widget._tiled_image.set_contrast(contrast_min_normalized, contrast_max_normalized)
        except Exception as e:
            print(f"ERROR in update_contrast: {e}")
            import traceback
            traceback.print_exc()

    def update_stats(self):
        """Update statistics display"""
        if self.bovista_widget._tiled_image:
            try:
                loaded, visible = self.bovista_widget._tiled_image.get_stats()
                num_lods = len(self.level_info) if self.level_info else 0
                self.stats_label.setText(
                    f"Multi-LOD ({num_lods} levels) | Chunks: {loaded} loaded, {visible} visible"
                )
            except Exception:
                pass


def main():
    """Main entry point"""
    print("=" * 70)
    print("Bovista Remote Multiscale OME-Zarr Viewer")
    print("=" * 70)
    print("\nThis example demonstrates cloud-native visualization with AUTOMATIC LOD:")
    print("  - Loads multiscale OME-Zarr from remote S3")
    print("  - AUTOMATIC spatial LOD selection (high-res near camera, low-res far)")
    print("  - Mixed-resolution rendering (multiple LODs in same frame)")
    print("  - Only downloads visible chunks (efficient!)")
    print("  - LOD Bias slider to adjust quality vs performance")
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
