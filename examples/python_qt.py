"""
Bovista + PySide6/Qt Integration Example

This example shows how to embed Bovista rendering into a Qt application.
"""

import sys
import numpy as np
from PySide6.QtWidgets import QApplication, QMainWindow, QWidget, QVBoxLayout, QLabel, QPushButton, QHBoxLayout
from PySide6.QtCore import QTimer, Qt
import bovista as bv


class BovistaWidget(QWidget):
    """
    A Qt widget that embeds a Bovista viewer.

    This widget handles:
    - Getting the native window handle from Qt
    - Initializing the viewer with that handle
    - Running the render loop using Qt's timer system
    - Handling mouse input for camera control
    """

    def __init__(self, parent=None, width=800, height=600):
        super().__init__(parent)

        # Set minimum size
        self.setMinimumSize(width, height)

        # Create viewer (but don't initialize yet)
        self.viewer = bv.PyViewer(width, height)

        # Track initialization state
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
        self.image_visual = None  # Will be set when scene is created

        # Create render timer (60 FPS)
        self.timer = QTimer(self)
        self.timer.timeout.connect(self._render_loop)
        self.timer.start(16)  # ~60 FPS

    def showEvent(self, event):
        """Called when widget is shown - this is when we can get the window handle"""
        super().showEvent(event)

        if not self._initialized:
            try:
                # Get native window handle from Qt
                # On macOS: returns NSView*
                # On Windows: returns HWND
                # On Linux: returns X11 Window ID
                handle = int(self.winId())

                print(f"Initializing with window handle: {handle}")

                # Initialize with the native handle
                size = self.size()
                self.viewer.initialize_with_window(
                    handle,
                    size.width(),
                    size.height()
                )

                self._initialized = True
                print("Viewer initialized successfully")

                # Run any queued setup callbacks
                for callback in self._setup_callbacks:
                    callback(self.viewer)
                self._setup_callbacks.clear()

            except Exception as e:
                print(f"Failed to initialize viewer: {e}")
                import traceback
                traceback.print_exc()

    def resizeEvent(self, event):
        """Handle window resize"""
        super().resizeEvent(event)

        if self._initialized:
            try:
                size = event.size()
                self.viewer.resize(size.width(), size.height())
            except Exception as e:
                print(f"Resize error: {e}")

    def _render_loop(self):
        """Render loop called by Qt timer"""
        if self._initialized:
            try:
                self.viewer.render_frame()
            except Exception as e:
                print(f"Render error: {e}")

    def mousePressEvent(self, event):
        """Handle mouse button press"""
        if event.button() == Qt.LeftButton:
            self.left_mouse_pressed = True
            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif event.button() == Qt.RightButton:
            self.right_mouse_pressed = True
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def mouseReleaseEvent(self, event):
        """Handle mouse button release"""
        if event.button() == Qt.LeftButton:
            self.left_mouse_pressed = False
        elif event.button() == Qt.RightButton:
            self.right_mouse_pressed = False

    def mouseMoveEvent(self, event):
        """Handle mouse move - orbit camera or adjust slice"""
        if self.left_mouse_pressed and self._initialized:
            # Left drag: orbit camera
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y
            self.viewer.orbit_camera(dx * 0.01, dy * 0.01)
            self.last_x = event.position().x()
            self.last_y = event.position().y()
        elif self.right_mouse_pressed and self._initialized and self.image_visual is not None:
            # Right drag: rotate slice plane
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y

            # Update slice angle based on mouse movement
            self.slice_angle += dy * 0.01
            self.slice_offset += dx * 0.5

            # Update the slice plane
            self._update_slice_plane()

            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        """Handle scroll wheel - zoom camera"""
        if self._initialized:
            # event.angleDelta().y() gives scroll amount in 1/8 degree units
            # Typical mouse wheel "click" is 15 degrees = 120 units
            zoom_delta = -event.angleDelta().y() * 0.1
            self.viewer.zoom_camera(zoom_delta)

    def on_ready(self, callback):
        """
        Register a callback to be called when the viewer is ready.

        Use this to add visuals after initialization:

        def setup_scene(viewer):
            points = bv.PyPointsVisual.test_cube(viewer, 20)
            viewer.add_points(points)

        widget.on_ready(setup_scene)
        """
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)

    # Convenience pass-through methods

    def set_camera_position(self, x, y, z):
        """Set camera position"""
        self.viewer.set_camera_position(x, y, z)

    def set_camera_target(self, x, y, z):
        """Set camera target (look-at point)"""
        self.viewer.set_camera_target(x, y, z)

    def add_points(self, visual):
        """Add a points visual to the scene"""
        if self._initialized:
            self.viewer.add_points(visual)
        else:
            raise RuntimeError("Viewer not initialized yet. Use on_ready() callback.")

    def add_lines(self, visual):
        """Add a lines visual to the scene"""
        if self._initialized:
            self.viewer.add_lines(visual)
        else:
            raise RuntimeError("Viewer not initialized yet. Use on_ready() callback.")

    def add_image(self, visual):
        """Add an image visual to the scene"""
        if self._initialized:
            self.viewer.add_image(visual)
            self.image_visual = visual  # Store reference for slice updates
        else:
            raise RuntimeError("Viewer not initialized yet. Use on_ready() callback.")

    def _update_slice_plane(self):
        """Update the slice plane based on current angle and offset"""
        if self.image_visual is None:
            return

        # Calculate normal based on angle (rotate around X axis)
        import math
        normal_y = math.sin(self.slice_angle)
        normal_z = math.cos(self.slice_angle)

        # Position the plane in the volume center with offset
        position_x = 128.0
        position_y = 128.0
        position_z = 128.0 + self.slice_offset

        # Update the slice plane
        self.image_visual.set_slice_plane(
            position_x, position_y, position_z,
            0.0, normal_y, normal_z
        )

        print(f"Slice angle: {self.slice_angle:.2f}, offset: {self.slice_offset:.2f}")


class MainWindow(QMainWindow):
    """Main application window"""

    def __init__(self):
        super().__init__()

        self.setWindowTitle("Bovista + PySide6/Qt Example")
        self.setGeometry(100, 100, 800, 600)

        # Create central widget and layout
        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Create Bovista widget
        self.bovista_widget = BovistaWidget(width=800, height=600)
        layout.addWidget(self.bovista_widget)

        # Add control panel
        control_layout = QHBoxLayout()

        info_label = QLabel("Left-drag: Orbit camera | Right-drag: Rotate slice plane | Scroll: Zoom")
        control_layout.addWidget(info_label)

        control_layout.addStretch()

        quit_button = QPushButton("Quit")
        quit_button.clicked.connect(self.close)
        control_layout.addWidget(quit_button)

        layout.addLayout(control_layout)

        # Set up camera (must set far plane before initialization to see geometry at Z=500)
        self.bovista_widget.viewer.set_camera_clip_planes(0.1, 2000.0)
        self.bovista_widget.set_camera_position(0, 0, 500)
        self.bovista_widget.set_camera_target(128, 128, 128)

        # Set up scene
        self.bovista_widget.on_ready(self.setup_scene)

    def setup_scene(self, viewer):
        """Called when viewer is ready"""
        print("Setting up scene...")

        # Create a 3D volume (256x256x256) with structure like the Rust example
        print("Creating volume data...")
        width = height = depth = 256
        volume = np.zeros((depth, height, width), dtype=np.uint8)

        # Create volume center
        cx_base = width / 2.0
        cy_base = height / 2.0
        cz_base = depth / 2.0
        max_dist = width / 2.0

        for z in range(depth):
            for y in range(height):
                for x in range(width):
                    # Distance from center
                    cx = x - cx_base
                    cy = y - cy_base
                    cz = z - cz_base
                    dist = np.sqrt(cx*cx + cy*cy + cz*cz)

                    # Main sphere with gradient
                    sphere_value = int((1.0 - min(dist / max_dist, 1.0)) * 180.0)

                    # Add bright core
                    core_value = 255 if dist < 20.0 else 0

                    # Add spherical shell
                    shell1_dist = abs(dist - 40.0)
                    shell1 = 200 if shell1_dist < 5.0 else 0

                    # Combine features
                    volume[z, y, x] = max(sphere_value, core_value, shell1)

        print("Volume generated!")

        # Create image visual
        print("Creating image visual...")
        image = bv.PyImageVisual.from_numpy(viewer, volume)
        # Set slice at center with proper normal
        image.set_slice_plane(128.0, 128.0, 128.0, 0.0, 0.0, 1.0)

        # Add to viewer and store reference in widget for slice control
        viewer.add_image(image)
        self.bovista_widget.image_visual = image

        # Add axes helper
        print("Creating axes...")
        axes = bv.PyLinesVisual.axis_helper(viewer, 200.0)
        viewer.add_lines(axes)

        print("Scene setup complete!")


def main():
    """Example usage of BovistaWidget"""

    app = QApplication(sys.argv)

    window = MainWindow()
    window.show()

    sys.exit(app.exec())


if __name__ == "__main__":
    main()
