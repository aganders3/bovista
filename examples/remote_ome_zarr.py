#!/usr/bin/env python3
"""
OME-Zarr Remote Viewer - TiledImage API Demo

Demonstrates the unified TiledImage API for multi-resolution chunked image loading.
The Python API now matches the WASM JsTiledImageVisual API, using a closure pattern
for async chunk loading. All image methods (slice, contrast, LOD) are on the image object.
"""

import sys
import numpy as np
from concurrent.futures import ThreadPoolExecutor
from threading import Lock
from PyQt6.QtWidgets import (QApplication, QMainWindow, QWidget, QVBoxLayout,
                            QHBoxLayout, QPushButton, QLabel, QSlider, QComboBox)
from PyQt6.QtCore import Qt, QTimer

import bovista as bv
import zarr


class ViewerWidget(QWidget):
    """Simple Qt widget wrapper for Bovista viewer"""
    def __init__(self, parent=None):
        super().__init__(parent)
        self.setMinimumSize(800, 600)
        self.viewer = bv.Viewer(800, 600)
        self._initialized = False
        self._setup_callbacks = []
        self._image = None

        # Mouse tracking
        self.setMouseTracking(True)
        self.left_pressed = False
        self.right_pressed = False
        self.last_x = 0
        self.last_y = 0

        # Oblique slicing state
        self.slice_angle_x = 0.0
        self.slice_angle_y = 0.0
        self.slice_offset = 0.0
        self.volume_center = [0, 0, 0]
        self.volume_scale = 1.0

        # Camera distance (synced from Rust after each zoom)
        self.camera_distance = 1.0

        # Window/level state (contrast)
        self.middle_pressed = False
        self.window_center = 0.5
        self.window_width = 1.0

        # Render loop
        self.timer = QTimer(self)
        self.timer.timeout.connect(self._render)
        self.timer.start(16)  # ~60fps

    def showEvent(self, event):
        super().showEvent(event)
        if not self._initialized:
            try:
                handle = int(self.winId())
                size = self.size()
                self.viewer.initialize_with_window(handle, size.width(), size.height())
                # Don't set clip planes here - they will be set adaptively when dataset loads
                # (Hard-coded values like 0.1-10000 don't work for very small volumes like beechnut)
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
        elif event.button() == Qt.MouseButton.RightButton:
            self.right_pressed = True
            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif event.button() == Qt.MouseButton.MiddleButton:
            self.middle_pressed = True
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.MouseButton.LeftButton:
            self.left_pressed = False
        elif event.button() == Qt.MouseButton.RightButton:
            self.right_pressed = False
        elif event.button() == Qt.MouseButton.MiddleButton:
            self.middle_pressed = False

    def mouseMoveEvent(self, event):
        if self.left_pressed and self._initialized:
            import math
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y

            # Check projection mode
            if self.viewer.get_camera_projection_mode() == bv.ProjectionMode.Orthographic:
                # Pan in orthographic mode
                pan_speed = self.volume_scale * 0.002
                self.viewer.pan_camera(-dx * pan_speed, dy * pan_speed)
            else:
                self.viewer.orbit_camera(dx * 0.005, dy * 0.005)

            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif self.right_pressed and self._initialized and self._image:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y

            # Shift key rotates around X axis, otherwise Y axis
            modifiers = QApplication.keyboardModifiers()
            if modifiers & Qt.KeyboardModifier.ShiftModifier:
                self.slice_angle_x += dy * 0.01
            else:
                self.slice_angle_y += dy * 0.01

            # Horizontal drag moves slice offset
            self.slice_offset += dx * self.volume_scale * 0.005
            self._update_slice_plane()
            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif self.middle_pressed and self._initialized and self._image:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y
            # Horizontal drag: level (center); vertical drag: window (width)
            self.window_center += dx * 0.002
            self.window_width  += dy * 0.002
            self.window_width   = max(0.001, self.window_width)
            self._apply_window_level()
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            # Use the zoom_camera method which handles both perspective and orthographic modes
            # Note: Qt has opposite sign from browser (positive = zoom in, negative = zoom out)
            delta_y = -event.angleDelta().y()
            self.viewer.zoom_camera(delta_y)

            if self.viewer.get_camera_projection_mode() == bv.ProjectionMode.Perspective:
                self.camera_distance = self.viewer.get_camera_distance()


    def _update_slice_plane(self):
        """Update slice plane with current angles and offset"""
        if not self._image:
            return

        import math
        cos_y = math.cos(self.slice_angle_y)
        sin_y = math.sin(self.slice_angle_y)
        cos_x = math.cos(self.slice_angle_x)
        sin_x = math.sin(self.slice_angle_x)

        # Calculate normal vector from angles
        normal_x = sin_y * cos_x
        normal_y = -sin_x
        normal_z = cos_y * cos_x

        # Normalize
        length = math.sqrt(normal_x**2 + normal_y**2 + normal_z**2)
        if length > 0.0001:
            normal_x /= length
            normal_y /= length
            normal_z /= length

        # Apply to image - move slice position along the normal vector
        center = self.volume_center
        self._image.set_slice_plane(
            center[0] + normal_x * self.slice_offset,
            center[1] + normal_y * self.slice_offset,
            center[2] + normal_z * self.slice_offset,
            normal_x, normal_y, normal_z
        )

        # If in orthographic mode, align camera to slice plane
        if self.viewer.get_camera_projection_mode() == bv.ProjectionMode.Orthographic:
            self._align_camera_to_slice(normal_x, normal_y, normal_z)

    def _apply_window_level(self):
        if not self._image:
            return
        half = self.window_width / 2.0
        self._image.set_contrast(self.window_center - half, self.window_center + half)

    def _align_camera_to_slice(self, normal_x, normal_y, normal_z):
        """Position camera along slice plane normal"""
        import math
        center = self.volume_center

        # Position camera along the normal at a fixed distance
        distance = self.volume_scale * 2.0
        pos_x = center[0] + normal_x * distance
        pos_y = center[1] + normal_y * distance
        pos_z = center[2] + normal_z * distance

        self.viewer.set_camera_position(pos_x, pos_y, pos_z)
        self.viewer.set_camera_target(center[0], center[1], center[2])

        # Calculate proper up vector perpendicular to normal (forward vector)
        # Try to align with world Y when possible
        if abs(normal_y) > 0.9:
            # Normal is nearly vertical, use Z as reference
            ref_x, ref_y, ref_z = 0.0, 0.0, 1.0
        else:
            # Use Y as reference
            ref_x, ref_y, ref_z = 0.0, 1.0, 0.0

        # Calculate right = forward × reference
        right_x = normal_y * ref_z - normal_z * ref_y
        right_y = normal_z * ref_x - normal_x * ref_z
        right_z = normal_x * ref_y - normal_y * ref_x

        # Normalize right
        right_len = math.sqrt(right_x**2 + right_y**2 + right_z**2)
        if right_len > 0.0001:
            right_x /= right_len
            right_y /= right_len
            right_z /= right_len

        # Calculate up = right × forward
        up_x = right_y * normal_z - right_z * normal_y
        up_y = right_z * normal_x - right_x * normal_z
        up_z = right_x * normal_y - right_y * normal_x

        # Normalize up
        up_len = math.sqrt(up_x**2 + up_y**2 + up_z**2)
        if up_len > 0.0001:
            up_x /= up_len
            up_y /= up_len
            up_z /= up_len

        self.viewer.set_camera_up(up_x, up_y, up_z)

    def on_ready(self, callback):
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)


class MainWindow(QMainWindow):
    def __init__(self):
        super().__init__()
        self.setWindowTitle("OME-Zarr Viewer - TiledImage API")
        self.setGeometry(100, 100, 1024, 768)

        # Central widget
        central = QWidget()
        self.setCentralWidget(central)
        layout = QVBoxLayout(central)

        # Dataset selector with camera distances
        self.datasets = {
            "Human Mitotic Cell (HeLa) - 3 channels": ("https://uk1s3.embassy.ebi.ac.uk/idr/zarr/v0.4/idr0062A/6001240.zarr", 200.0),
            "Marmoset Neurons - 3D microscopy": ("https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/marmoset_neurons.ome.zarr", 300.0),
            "Pawpawsaurus - CT scan fossil": ("https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/pawpawsaurus.ome.zarr", 500.0),
            "Beechnut - CT scan": ("https://ome-zarr-scivis.s3.us-east-1.amazonaws.com/v0.5/96x2/beechnut.ome.zarr", 0.05),
            "Platybrowser - Embryo, 10 LODs": ("https://s3.embl.de/i2k-2020/platy-raw.ome.zarr", 200.0),
        }
        dataset_layout = QHBoxLayout()
        dataset_layout.addWidget(QLabel("Dataset:"))
        self.dataset_combo = QComboBox()
        for name in self.datasets.keys():
            self.dataset_combo.addItem(name)
        dataset_layout.addWidget(self.dataset_combo)
        self.load_button = QPushButton("Load")
        self.load_button.clicked.connect(self.load_zarr)
        dataset_layout.addWidget(self.load_button)
        layout.addLayout(dataset_layout)

        # Viewer
        self.viewer_widget = ViewerWidget()
        layout.addWidget(self.viewer_widget)

        # Controls
        controls = QHBoxLayout()
        controls.addWidget(QLabel("LOD Bias:"))
        self.lod_slider = QSlider(Qt.Orientation.Horizontal)
        self.lod_slider.setRange(-20, 20)
        self.lod_slider.setValue(0)
        self.lod_slider.valueChanged.connect(self.update_lod_bias)
        controls.addWidget(self.lod_slider)
        self.lod_label = QLabel("0.0")
        controls.addWidget(self.lod_label)
        controls.addStretch()
        self.debug_button = QPushButton("Debug LOD: Off")
        self.debug_button.setCheckable(True)
        self.debug_button.clicked.connect(self.toggle_debug_mode)
        controls.addWidget(self.debug_button)
        self.projection_button = QPushButton("Switch to Orthographic")
        self.projection_button.clicked.connect(self.toggle_projection)
        controls.addWidget(self.projection_button)
        layout.addLayout(controls)

        # Controls info
        layout.addWidget(QLabel("Left-drag: Orbit | Middle-drag: Window/Level | Right-drag: Slice | Shift+Right: Rotate | Scroll: Zoom"))

        # Status
        self.status_label = QLabel("Select dataset and click Load")
        layout.addWidget(self.status_label)

        # Stats timer
        self.stats_timer = QTimer()
        self.stats_timer.timeout.connect(self.update_stats)
        self.stats_timer.start(100)

        self.executor = None
        self.image = None
        self.level_info = []

    def load_zarr(self):
        try:
            dataset_name = self.dataset_combo.currentText()
            url, camera_distance = self.datasets[dataset_name]
            print(f"\n{'='*60}")
            print(f"Loading: {url}")
            print('='*60)
            self.status_label.setText(f"Loading...")
            self.load_button.setEnabled(False)

            # Open zarr store (try zarr v3 format first, fallback to v2)
            try:
                store = zarr.open_group(url, mode="r", zarr_format=3)
            except Exception:
                try:
                    store = zarr.open_group(url, mode="r", zarr_format=2)
                except Exception:
                    store = zarr.open_group(url, mode="r")

            # Get multiscales metadata (handle both OME-Zarr 0.4 and 0.5)
            attrs = dict(store.attrs)
            if "ome" in attrs:  # OME-Zarr 0.5+ (uses zarr v3)
                multiscales = attrs["ome"]["multiscales"][0]
            else:  # OME-Zarr 0.4 (uses zarr v2)
                multiscales = attrs.get("multiscales", [{}])[0]
            datasets = multiscales.get("datasets", [])
            axes = multiscales.get("axes", [])
            axis_names = [ax.get("name", "?") for ax in axes]

            # Fallback if no multiscales metadata
            if not datasets:
                fallback_path = "0" if "0" in store else ""
                if not fallback_path and len(store) > 0:
                    fallback_path = list(store.keys())[0]
                datasets = [{"path": fallback_path}]
                axis_names = ["z", "y", "x", "c", "t"][:len(store[datasets[0]["path"]].shape)]

            print(f"Found {len(datasets)} LOD levels, axes: {axis_names}")

            # Find spatial indices (z, y, x)
            spatial_map = {}
            for i, name in enumerate(axis_names):
                if name in ["z", "y", "x"]:
                    spatial_map[name] = i

            if len(spatial_map) != 3:
                # Fallback: assume last 3 dimensions
                ndim = len(store[datasets[0]["path"]].shape)
                spatial_map = {"z": ndim-3, "y": ndim-2, "x": ndim-1}

            z_idx, y_idx, x_idx = spatial_map["z"], spatial_map["y"], spatial_map["x"]
            spatial_indices = (z_idx, y_idx, x_idx)

            # Parse LOD levels
            lod_levels = []
            lod0_spacing = None

            for i, ds_info in enumerate(datasets):
                path = ds_info.get("path", str(i))
                arr = store[path]

                # Get spatial dimensions
                volume_shape = (arr.shape[z_idx], arr.shape[y_idx], arr.shape[x_idx])
                chunk_shape = (arr.chunks[z_idx] if arr.chunks else 64,
                             arr.chunks[y_idx] if arr.chunks else 64,
                             arr.chunks[x_idx] if arr.chunks else 64)

                # Get voxel spacing from coordinate transforms
                transforms = ds_info.get("coordinateTransformations", [])
                scale_transform = next((t for t in transforms if t.get("type") == "scale"), None)

                if scale_transform:
                    scale = scale_transform.get("scale", [1.0] * len(arr.shape))
                    voxel_spacing = (scale[z_idx], scale[y_idx], scale[x_idx])
                else:
                    voxel_spacing = (1.0, 1.0, 1.0)

                # Calculate scale factor
                if i == 0:
                    lod0_spacing = voxel_spacing
                    scale_factor = 1.0
                else:
                    # Use the dimension that changes most
                    scale_factor = max(
                        voxel_spacing[0] / lod0_spacing[0],
                        voxel_spacing[1] / lod0_spacing[1],
                        voxel_spacing[2] / lod0_spacing[2]
                    )

                level = bv.LevelMetadata(
                    volume_size=volume_shape,
                    chunk_size=chunk_shape,
                    voxel_size=voxel_spacing,
                    scale_factor=scale_factor,
                    translation=(0.0, 0.0, 0.0)
                )
                lod_levels.append(level)
                print(f"  LOD {i}: {volume_shape} @ {scale_factor:.1f}x")

            self.level_info = lod_levels
            if len(lod_levels) == 0:
                raise ValueError("No LOD levels found in dataset!")

            # Create executor
            if self.executor:
                self.executor.shutdown(wait=False)
            self.executor = ThreadPoolExecutor(max_workers=8)

            # Create loader closure
            def create_loader():
                image = None
                pending = {}
                lock = Lock()

                def load_chunk(lod, z, y, x):
                    try:
                        path = datasets[lod].get("path", str(lod))
                        arr = store[path]
                        level = lod_levels[lod]
                        cz, cy, cx = level.chunk_size

                        # Calculate slice bounds
                        z_slice = slice(z * cz, min((z + 1) * cz, level.volume_size[0]))
                        y_slice = slice(y * cy, min((y + 1) * cy, level.volume_size[1]))
                        x_slice = slice(x * cx, min((x + 1) * cx, level.volume_size[2]))

                        # Build full slicing tuple (handle non-spatial dimensions)
                        slices = [0] * len(arr.shape)  # Default to first index for non-spatial
                        slices[z_idx] = z_slice
                        slices[y_idx] = y_slice
                        slices[x_idx] = x_slice

                        # Load data — pass uint8 or uint16 directly; Rust handles both
                        data = arr[tuple(slices)]
                        if data.dtype == np.uint8:
                            return np.ascontiguousarray(data)
                        elif data.dtype == np.uint16:
                            return np.ascontiguousarray(data)
                        elif np.issubdtype(data.dtype, np.integer):
                            # Other integer types: normalize to uint16 to preserve precision
                            info = np.iinfo(data.dtype)
                            data = ((data.astype(np.float32) - info.min) / (info.max - info.min) * 65535).astype(np.uint16)
                            return np.ascontiguousarray(data)
                        else:
                            # Float data: normalize to uint16
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
                    nonlocal image, pending
                    with lock:
                        pending.pop((lod, z, y, x), None)
                    if not future.cancelled() and image:
                        data = future.result()
                        if data is not None:
                            if data.dtype == np.uint8:
                                data = (data.astype(np.uint16) * 257)  # 0-255 → 0-65535
                            image.set_chunk_data_u16(lod, z, y, x, data)

                def request(lod, z, y, x):
                    nonlocal pending
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

                def set_image(img):
                    nonlocal image
                    image = img

                request.set_image = set_image
                return request

            loader = create_loader()
            self.level_info = lod_levels  # Store for stats display

            # Setup scene
            def setup_scene(viewer):
                # Create VirtualTiledImage (single draw call, shader-side LOD fallback)
                image = bv.VirtualTiledImage(viewer, lod_levels, 500, loader)
                loader.set_image(image)  # Close the closure loop

                # Calculate world space
                lod0 = lod_levels[0]
                world_size = tuple(s * v for s, v in zip(lod0.volume_size, lod0.voxel_size))
                center = [world_size[2] / 2, world_size[1] / 2, world_size[0] / 2]

                # Configure image
                image.set_slice_plane(center[0], center[1], center[2], 0.0, 0.0, 1.0)
                image.set_contrast(0.0, 1.0)

                # Add to scene
                viewer.add(image)
                self.viewer_widget._image = image
                self.viewer_widget.volume_center = center
                self.viewer_widget.volume_scale = max(world_size)
                self.viewer_widget.window_center = 0.5
                self.viewer_widget.window_width = 1.0
                self.image = image

                # Set adaptive camera clip planes based on volume size
                max_dimension = max(world_size)
                near_plane = max_dimension * 0.001  # 0.1% of max dimension
                far_plane = max_dimension * 100      # 100x max dimension
                viewer.set_camera_clip_planes(near_plane, far_plane)
                print(f"  Clip planes: near={near_plane:.6f}, far={far_plane:.2f}")

                self.viewer_widget.camera_distance = camera_distance
                viewer.set_camera_position(center[0], center[1], center[2] + camera_distance)
                viewer.set_camera_target(*center)
                print(f"  Camera distance: {camera_distance}")

                # Add axes
                viewer.add(bv.Lines.axis_helper(viewer, max(lod0.volume_size) * 0.3))

                self.status_label.setText("Loaded! Use mouse to navigate.")
                print(f"✓ Ready")

            self.viewer_widget.on_ready(setup_scene)

        except Exception as e:
            self.status_label.setText(f"Error: {e}")
            print(f"ERROR: {e}")
            import traceback
            traceback.print_exc()
            self.load_button.setEnabled(True)

    def update_lod_bias(self):
        if self.image:
            bias = self.lod_slider.value() / 10.0
            self.lod_label.setText(f"{bias:.1f}")
            self.image.set_lod_bias(bias)

    def toggle_debug_mode(self):
        enabled = self.debug_button.isChecked()
        self.debug_button.setText(f"Debug LOD: {'On' if enabled else 'Off'}")
        if self.image:
            self.image.set_debug_mode(enabled)

    def toggle_projection(self):
        current_mode = self.viewer_widget.viewer.get_camera_projection_mode()
        if current_mode == bv.ProjectionMode.Perspective:
            # Switch to orthographic
            self.viewer_widget.viewer.set_camera_projection_mode(bv.ProjectionMode.Orthographic)
            # Set ortho height based on volume size
            self.viewer_widget.viewer.set_camera_ortho_height(self.viewer_widget.volume_scale * 0.5)
            # Align camera to slice plane
            self.viewer_widget._update_slice_plane()
            self.projection_button.setText("Switch to Perspective")
        else:
            self.viewer_widget.viewer.set_camera_projection_mode(bv.ProjectionMode.Perspective)
            self.projection_button.setText("Switch to Orthographic")

    def update_stats(self):
        if self.image:
            try:
                loaded, visible = self.image.get_stats()
                num_lods = len(self.level_info)
                self.status_label.setText(
                    f"Multi-LOD ({num_lods} levels) | Chunks: {loaded} loaded, {visible} visible"
                )
            except:
                pass

def main():
    print("=" * 60)
    print("OME-Zarr TiledImage Viewer")
    print("=" * 60)
    print("Demonstrates unified TiledImage API matching WASM")
    print("=" * 60 + "\n")

    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())

if __name__ == "__main__":
    main()
