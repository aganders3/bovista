"""
Test script for the new unified ImageVisual with SimpleStrategy

This is a minimal test to verify that the refactored ImageVisual works correctly.
It creates a simple synthetic image and displays it using the new tile-based system.

Requirements:
    pip install PySide6 numpy bovista
"""

import sys
import numpy as np
from PySide6.QtWidgets import QApplication, QMainWindow, QWidget, QVBoxLayout, QLabel
from PySide6.QtCore import QTimer, Qt
import bovista as bv


class SimpleViewer(QWidget):
    """Minimal Qt widget for testing Bovista"""

    def __init__(self, parent=None, width=800, height=600):
        super().__init__(parent)
        self.setMinimumSize(width, height)
        self.viewer = bv.Viewer(width, height)
        self._initialized = False
        self._setup_callbacks = []

        self.setMouseTracking(True)
        self.left_mouse_pressed = False
        self.last_x = 0
        self.last_y = 0

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
                print("✓ Viewer initialized")

                # Run setup callbacks
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

    def mouseReleaseEvent(self, event):
        if event.button() == Qt.LeftButton:
            self.left_mouse_pressed = False

    def mouseMoveEvent(self, event):
        if self.left_mouse_pressed and self._initialized:
            dx = event.position().x() - self.last_x
            dy = event.position().y() - self.last_y
            self.viewer.orbit_camera(dx * 0.01, dy * 0.01)
            self.last_x = event.position().x()
            self.last_y = event.position().y()

    def wheelEvent(self, event):
        if self._initialized:
            zoom_delta = -event.angleDelta().y() * 0.2
            self.viewer.zoom_camera(zoom_delta)

    def on_ready(self, callback):
        """Register callback to run when viewer is initialized"""
        if self._initialized:
            callback(self.viewer)
        else:
            self._setup_callbacks.append(callback)


class MainWindow(QMainWindow):
    """Main window for testing new ImageVisual"""

    def __init__(self):
        super().__init__()
        self.setWindowTitle("New ImageVisual Test - SimpleStrategy")
        self.setGeometry(100, 100, 900, 700)

        central_widget = QWidget()
        self.setCentralWidget(central_widget)
        layout = QVBoxLayout(central_widget)

        # Info label
        info_label = QLabel("Testing new unified ImageVisual with SimpleStrategy")
        info_label.setStyleSheet("font-weight: bold; font-size: 14px;")
        layout.addWidget(info_label)

        # Viewer widget
        self.viewer_widget = SimpleViewer(width=900, height=600)
        layout.addWidget(self.viewer_widget)

        # Controls label
        controls_label = QLabel("Left-drag: Orbit | Scroll: Zoom")
        layout.addWidget(controls_label)

        # Setup scene when ready
        self.viewer_widget.on_ready(self.setup_scene)

    def setup_scene(self, viewer):
        """Create a simple test image and display it"""
        print("\n" + "=" * 60)
        print("Setting up test scene...")
        print("=" * 60)

        try:
            # Create a simple 2D gradient image (256x256)
            print("\n1. Creating synthetic test image (256x256)...")
            width, height = 256, 256

            # Create a radial gradient pattern
            x = np.linspace(-1, 1, width)
            y = np.linspace(-1, 1, height)
            xx, yy = np.meshgrid(x, y)

            # Radial gradient with some structure
            r = np.sqrt(xx**2 + yy**2)
            data = (np.sin(r * 10) * 0.5 + 0.5) * 255
            data = data.astype(np.uint8)

            # Add a depth dimension to make it 3D (depth=1 for 2D image)
            data = data[np.newaxis, :, :]  # Shape: (1, 256, 256) = (depth, height, width)

            print(f"   ✓ Image shape: {data.shape}")
            print(f"   ✓ Data type: {data.dtype}")
            print(f"   ✓ Value range: {data.min()}-{data.max()}")

            # Create ImageVisual using the new system
            print("\n2. Creating ImageVisual with SimpleStrategy...")
            image = bv.Image.from_numpy(viewer, data)
            print("   ✓ ImageVisual created successfully!")

            # Set up camera to view the image
            print("\n3. Setting up camera...")
            center_x = width / 2.0
            center_y = height / 2.0
            center_z = 0.5  # Slight depth

            # Position camera to look at the image from above
            cam_distance = max(width, height) * 1.5
            viewer.set_camera_position(center_x, center_y, center_z + cam_distance)
            viewer.set_camera_target(center_x, center_y, center_z)
            viewer.set_camera_clip_planes(0.1, cam_distance * 5.0)

            print(f"   ✓ Camera position: ({center_x}, {center_y}, {center_z + cam_distance})")
            print(f"   ✓ Camera target: ({center_x}, {center_y}, {center_z})")

            # Set slice plane (XY plane at Z=0.5)
            print("\n4. Setting slice plane...")
            image.set_slice_z(0.5)
            print("   ✓ Slice plane: XY at Z=0.5")

            # Set contrast
            image.set_contrast(0.0, 1.0)
            print("   ✓ Contrast: 0.0 - 1.0")

            # Add to scene
            print("\n5. Adding to scene...")
            viewer.add(image)
            print("   ✓ Image added to scene")

            # Add coordinate axes for reference
            axes = bv.Lines.axis_helper(viewer, max(width, height) * 0.3)
            viewer.add(axes)
            print("   ✓ Axes added")

            print("\n" + "=" * 60)
            print("✓ Scene setup complete!")
            print("=" * 60)
            print("\nYou should see a radial gradient pattern.")
            print("Try orbiting with left-drag and zooming with scroll wheel.")
            print("=" * 60 + "\n")

        except Exception as e:
            print(f"\n❌ Error setting up scene: {e}")
            import traceback
            traceback.print_exc()


def main():
    """Main entry point"""
    print("\n" + "=" * 60)
    print("New ImageVisual Test")
    print("=" * 60)
    print("\nThis test verifies the refactored ImageVisual works correctly")
    print("with the new tile-based SimpleStrategy.")
    print("=" * 60 + "\n")

    app = QApplication(sys.argv)
    window = MainWindow()
    window.show()
    sys.exit(app.exec())


if __name__ == "__main__":
    main()
