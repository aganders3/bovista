"""
Test new unified ImageVisual with chunked multi-resolution loading

This tests Image.from_chunked() which uses the new ChunkStatus-based API
with automatic LOD selection, loading chunks on-demand from remote OME-Zarr.

New API features:
- Loader callback returns ChunkStatus (Accepted/AlreadyPending/Rejected)
- Call image.set_chunk_data() when data is ready
- Backpressure control through ChunkStatus instead of capacity_check callback

Requirements:
    pip install PySide6 zarr fsspec requests aiohttp bovista numpy
"""

import sys
import numpy as np
import zarr
from PySide6.QtWidgets import (
    QApplication, QMainWindow, QWidget, QVBoxLayout, QLabel, QPushButton,
    QHBoxLayout, QSlider, QComboBox,
)
from PySide6.QtCore import QTimer, Qt
import bovista as bv
from concurrent.futures import ThreadPoolExecutor
from threading import Lock


# Public IDR OME-Zarr datasets on S3
DATASETS = {
    "Human Mitotic Cell (small)": {
        "url": "https://uk1s3.embassy.ebi.ac.uk/idr/zarr/v0.4/idr0062A/6001240.zarr",
        "description": "HeLa cells, 3 resolution levels",
        "camera_distance": 200.0,
        "zarr_version": 2,  # OME-Zarr 0.4 uses Zarr v2
    },
    "Platybrowser (medium)": {
        "url": "https://s3.embl.de/i2k-2020/platy-raw.ome.zarr",
        "description": "Platynereis embryo, 10 resolution levels",
        "camera_distance": 400.0,
        "zarr_version": 2,  # OME-Zarr 0.4 uses Zarr v2
    },
    # Note: Zarr v3 datasets (Marmoset, Pawpawsaurus, Beechnut) removed
    # because zarr-python v2.x cannot read Zarr v3 format stores.
    # To use Zarr v3 datasets, install zarr>=3.0.0a (but this breaks v2 compatibility).
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
        self.volume_scale = 1.0  # For adaptive zoom

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

            # Scale offset by volume size for consistent control across different scales
            offset_scale = self.volume_scale * 0.005  # 0.5% of volume per pixel
            self.slice_offset += dx * offset_scale
            self._update_slice_plane()
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            # Adaptive zoom: scale by volume size for smooth control at all scales
            zoom_speed = self.volume_scale * 0.001  # 0.1% of volume per wheel tick
            zoom_delta = -event.angleDelta().y() * zoom_speed
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
        self.setGeometry(100, 100, 900, 750)

        self.zarr_data = None
        self.chunk_executor = None
        self.current_channel = 0
        self.dataset_info = None
        self.level_info = None

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Info label
        info_label = QLabel("Testing new Image.from_chunked() with OME-Zarr")
        info_label.setStyleSheet("font-weight: bold; font-size: 14px;")
        layout.addWidget(info_label)

        # Dataset selector
        selector_layout = QHBoxLayout()
        selector_layout.addWidget(QLabel("Dataset:"))
        self.dataset_combo = QComboBox()
        for name, info in DATASETS.items():
            self.dataset_combo.addItem(f"{name} - {info['description']}", name)
        selector_layout.addWidget(self.dataset_combo)
        self.load_button = QPushButton("Load Dataset")
        self.load_button.clicked.connect(self.load_dataset)
        selector_layout.addWidget(self.load_button)
        self.status_label = QLabel("Select a dataset and click 'Load Dataset'")
        selector_layout.addWidget(self.status_label)
        selector_layout.addStretch()
        layout.addLayout(selector_layout)

        # LOD bias and channel selectors
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
        self.lod_label.setMinimumWidth(40)
        lod_layout.addWidget(self.lod_label)
        lod_layout.addWidget(QLabel("Channel:"))
        self.channel_combo = QComboBox()
        self.channel_combo.currentIndexChanged.connect(self.change_channel)
        self.channel_combo.setEnabled(False)
        lod_layout.addWidget(self.channel_combo)
        lod_layout.addStretch()
        layout.addLayout(lod_layout)

        # Contrast controls
        contrast_layout = QHBoxLayout()
        contrast_layout.addWidget(QLabel("Contrast Min:"))
        self.contrast_min_slider = QSlider(Qt.Horizontal)
        self.contrast_min_slider.setMinimum(0)
        self.contrast_min_slider.setMaximum(255)
        self.contrast_min_slider.setValue(0)
        self.contrast_min_slider.setEnabled(False)
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
        self.contrast_max_slider.setEnabled(False)
        self.contrast_max_slider.valueChanged.connect(self.update_contrast)
        contrast_layout.addWidget(self.contrast_max_slider)
        self.contrast_max_label = QLabel("255")
        self.contrast_max_label.setMinimumWidth(50)
        contrast_layout.addWidget(self.contrast_max_label)
        contrast_layout.addStretch()
        layout.addLayout(contrast_layout)

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
        """Load the selected dataset"""
        self.load_button.setEnabled(False)
        self.status_label.setText("Loading metadata...")
        QApplication.processEvents()

        try:
            # Get selected dataset
            dataset_name = self.dataset_combo.currentData()
            dataset_info = DATASETS[dataset_name]

            print("\n" + "=" * 60)
            print(f"Loading dataset: {dataset_name}")
            print(f"URL: {dataset_info['url']}")
            print("=" * 60)

            # Open zarr store
            store = zarr.open(dataset_info["url"], mode="r")
            self.zarr_data = store
            self.dataset_info = dataset_info

            # Get multiscales info (structure differs between v2 and v3)
            zarr_version = dataset_info.get("zarr_version", 2)
            if zarr_version == 3:
                # OME-Zarr 0.5 (Zarr v3): metadata nested under 'ome'
                multiscales = store.attrs["ome"]["multiscales"][0]
            else:
                # OME-Zarr 0.4 (Zarr v2): metadata directly in attrs
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

            # Detect channels
            if "c" in axis_names:
                channel_idx = axis_names.index("c")
                first_array = store[datasets[0]["path"]]
                num_channels = first_array.shape[channel_idx]

                # Populate channel combo
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
                print(f"Found {num_channels} channels, defaulting to channel {last_channel}")
            else:
                # Single channel dataset
                self.channel_combo.blockSignals(True)
                self.channel_combo.clear()
                self.channel_combo.addItem("Channel 0", 0)
                self.channel_combo.setCurrentIndex(0)
                self.channel_combo.blockSignals(False)
                self.channel_combo.setEnabled(False)
                self.current_channel = 0
                print("Single channel dataset")

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

                # Get voxel spacing and translation from coordinate transforms
                transforms = dataset_info.get("coordinateTransformations", [])
                voxel_spacing = [1.0, 1.0, 1.0]
                translation = [0.0, 0.0, 0.0]
                for transform in transforms:
                    if transform.get("type") == "scale":
                        scale = transform.get("scale", [])
                        if len(scale) > max(z_idx, y_idx, x_idx):
                            voxel_spacing = [scale[z_idx], scale[y_idx], scale[x_idx]]
                    elif transform.get("type") == "translation":
                        trans = transform.get("translation", [])
                        if len(trans) > max(z_idx, y_idx, x_idx):
                            translation = [trans[z_idx], trans[y_idx], trans[x_idx]]

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
                    scale_factor=scale_factor,
                    translation=tuple(translation)
                )
                lod_levels.append(level)

                print(f"\nLOD {lod_idx}:")
                print(f"  Volume: {volume_shape}")
                print(f"  Chunks: {chunk_shape}")
                print(f"  Voxel spacing: {voxel_spacing}")
                print(f"  Scale: {scale_factor:.2f}x")
                if lod_idx == 0 and any(t != 0.0 for t in translation):
                    print(f"  Translation: {translation}")

            # Store level info for later use
            self.level_info = lod_levels

            # Enable LOD slider and contrast sliders
            self.lod_slider.setEnabled(True)
            self.contrast_min_slider.setEnabled(True)
            self.contrast_max_slider.setEnabled(True)

            # Setup loader in background thread
            if self.chunk_executor:
                self.chunk_executor.shutdown(wait=False)

            self.chunk_executor = ThreadPoolExecutor(
                max_workers=8, thread_name_prefix="zarr_loader"
            )

            # Create a class-based loader that will have strategy automatically set
            class ChunkLoader:
                """Loader with strategy attribute that will be set by from_chunked"""
                strategy = None  # Will be set by Rust during from_chunked

                def __init__(self, executor):
                    self.executor = executor
                    self.pending_futures = {}
                    self.cache_lock = Lock()
                    self.loaded_count = 0
                    self.MAX_PENDING = 8

                def load_chunk_blocking(self, lod_level, z, y, x):
                    print(f"[Loader] Loading LOD{lod_level} ({z},{y},{x})...")  # DEBUG
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
                        # For non-spatial dimensions, select appropriate index
                        for i in range(len(array.shape)):
                            if i not in spatial_indices:
                                # Check if this is the channel dimension
                                if "c" in axis_names and i == axis_names.index("c"):
                                    slices[i] = current_channel  # Use selected channel
                                else:
                                    slices[i] = 0  # Take first element for other dimensions (e.g., time)

                        slices[z_idx] = slice(z_start, z_end)
                        slices[y_idx] = slice(y_start, y_end)
                        slices[x_idx] = slice(x_start, x_end)

                        chunk_data = array[tuple(slices)]

                        if chunk_data.size == 0:
                            return None

                        # Convert to uint8 using fixed range normalization
                        # This avoids seam artifacts from per-chunk normalization
                        if chunk_data.dtype != np.uint8:
                            # Use dtype range for consistent normalization
                            if np.issubdtype(chunk_data.dtype, np.signedinteger):
                                # Signed integers (e.g., int16: -32768 to 32767)
                                dtype_min = np.iinfo(chunk_data.dtype).min
                                dtype_max = np.iinfo(chunk_data.dtype).max
                            elif np.issubdtype(chunk_data.dtype, np.unsignedinteger):
                                # Unsigned integers (e.g., uint16: 0 to 65535)
                                dtype_min = 0
                                dtype_max = np.iinfo(chunk_data.dtype).max
                            else:
                                # Fallback to per-chunk for floats or unknown types
                                dtype_min = float(chunk_data.min())
                                dtype_max = float(chunk_data.max())

                            # Normalize to 0-255
                            if dtype_max > dtype_min:
                                chunk_data = ((chunk_data.astype(np.float32) - dtype_min) / (dtype_max - dtype_min) * 255).astype(np.uint8)
                            else:
                                chunk_data = np.zeros_like(chunk_data, dtype=np.uint8)

                        return np.array(chunk_data)
                    except Exception as e:
                        print(f"ERROR loading chunk LOD{lod_level} ({z},{y},{x}): {e}")
                        return None

                def on_chunk_loaded(self, lod, z, y, x, future):
                    key = (lod, z, y, x)
                    try:
                        if future.cancelled():
                            with self.cache_lock:
                                self.pending_futures.pop(key, None)
                            return

                        result = future.result()
                        with self.cache_lock:
                            self.pending_futures.pop(key, None)

                        if result is not None and self.strategy is not None:
                            # Provide the loaded data via the strategy
                            # No retry needed - Rust handles queueing internally
                            self.strategy.set_chunk_data(lod, z, y, x, result)
                            self.loaded_count += 1
                            if self.loaded_count <= 5 or self.loaded_count % 50 == 0:
                                print(f"  Loaded LOD{lod} ({z},{y},{x}) - total: {self.loaded_count}")
                    except Exception as e:
                        print(f"ERROR in on_chunk_loaded: {e}")
                        import traceback
                        traceback.print_exc()
                        with self.cache_lock:
                            self.pending_futures.pop(key, None)

                def request(self, lod, z, y, x):
                    """Loader callback that returns ChunkStatus"""
                    key = (lod, z, y, x)
                    with self.cache_lock:
                        # Check if already loading
                        if key in self.pending_futures:
                            return bv.ChunkStatus.AlreadyPending

                        # Check capacity
                        if len(self.pending_futures) >= self.MAX_PENDING:
                            return bv.ChunkStatus.Rejected

                        # Start loading
                        future = self.executor.submit(self.load_chunk_blocking, lod, z, y, x)
                        future.add_done_callback(lambda f: self.on_chunk_loaded(lod, z, y, x, f))
                        self.pending_futures[key] = future

                    return bv.ChunkStatus.Accepted

            # Create the loader instance
            current_channel = self.current_channel  # Capture current channel for closure
            self.loader = ChunkLoader(self.chunk_executor)

            # Setup scene when viewer is ready
            def setup_scene(viewer):
                print("\n" + "=" * 60)
                print("Creating Image.from_chunked()...")
                print("=" * 60)

                # Create image with chunked loading (strategy will be auto-set on loader)
                # Pass the loader object, not the method, so Rust can set strategy attribute
                image = bv.Image.from_chunked(
                    viewer,
                    lod_levels,
                    self.loader,  # Pass object, Rust will call .request() method
                    max_chunks=500
                )

                print("✓ Image created!")
                print(f"  Loader.strategy = {self.loader.strategy}")

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
                self.viewer_widget.volume_scale = max(volume_size_world)  # For adaptive zoom

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
        # Use strategy directly since set_lod_bias is strategy-specific
        if hasattr(self, 'loader') and self.loader and self.loader.strategy:
            bias = self.lod_slider.value() / 10.0
            self.lod_label.setText(f"{bias:.1f}")
            self.loader.strategy.set_lod_bias(bias)

    def update_stats(self):
        # Use strategy for stats since it's strategy-specific
        if hasattr(self, 'loader') and self.loader and self.loader.strategy:
            try:
                loaded, visible = self.loader.strategy.get_stats()
                num_lods = len(self.level_info) if self.level_info else 0
                self.status_label.setText(
                    f"Multi-LOD ({num_lods} levels) | Chunks: {loaded} loaded, {visible} visible"
                )
            except Exception:
                pass

    def update_contrast(self):
        """Update contrast range based on slider values"""
        try:
            if self.viewer_widget._image is None:
                return

            # Get slider values (0-255)
            min_pos = self.contrast_min_slider.value()
            max_pos = self.contrast_max_slider.value()

            # Ensure min < max
            if min_pos >= max_pos:
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

            # Update labels
            self.contrast_min_label.setText(str(min_pos))
            self.contrast_max_label.setText(str(max_pos))

            # Map 0-255 to normalized 0-1 range for shader
            contrast_min_normalized = min_pos / 255.0
            contrast_max_normalized = max_pos / 255.0

            # Apply to image
            self.viewer_widget._image.set_contrast(contrast_min_normalized, contrast_max_normalized)
        except Exception as e:
            print(f"ERROR in update_contrast: {e}")

    def change_channel(self, index):
        """Change the displayed channel"""
        if self.zarr_data is None or index < 0:
            return

        channel = self.channel_combo.itemData(index)
        if channel is None:
            return

        print(f"\nChanging to channel {channel}")
        self.current_channel = channel

        # Reload the dataset with the new channel
        # We need to trigger a reload of all chunks
        self.load_button.click()


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
