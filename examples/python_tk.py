"""
Bovista + Tkinter Integration Example

This example shows how to embed Bovista rendering into a Tkinter application.
"""

import tkinter as tk
import numpy as np
import bovista as bv


class BovistaCanvas(tk.Frame):
    """
    A Tkinter widget that embeds a Bovista viewer.

    This widget handles:
    - Getting the native window handle from Tk
    - Initializing the viewer with that handle
    - Running the render loop using Tk's event system
    - Handling mouse input for camera control
    """

    def __init__(self, parent, width=800, height=600, **kwargs):
        super().__init__(parent, width=width, height=height, **kwargs)

        self.width = width
        self.height = height

        # Create viewer (but don't initialize yet)
        self.viewer = bv.PyViewer(width, height)

        # Track initialization state
        self._initialized = False
        self._setup_callbacks = []

        # Mouse tracking
        self.mouse_pressed = False
        self.last_x = 0
        self.last_y = 0

        # Wait for widget to be mapped to screen, then initialize
        self.bind("<Map>", self._on_map)

        # Handle resizing
        self.bind("<Configure>", self._on_configure)

        # Handle mouse input
        self.bind("<ButtonPress-1>", self._on_mouse_down)
        self.bind("<ButtonRelease-1>", self._on_mouse_up)
        self.bind("<B1-Motion>", self._on_mouse_drag)
        self.bind("<MouseWheel>", self._on_scroll)

        # Start render loop
        self._render_loop()

    def _on_map(self, event):
        """Called when widget is mapped to screen - this is when we can get the window handle"""
        if not self._initialized:
            try:
                # Get native window handle
                # On macOS: NSView*
                # On Windows: HWND
                # On Linux: X11 Window ID
                handle = self.winfo_id()

                print(f"Initializing with window handle: {handle}")

                # Initialize with the native handle
                self.viewer.initialize_with_window(
                    handle,
                    self.width,
                    self.height
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

    def _on_configure(self, event):
        """Handle window resize"""
        if self._initialized and (event.width != self.width or event.height != self.height):
            try:
                self.width = event.width
                self.height = event.height
                self.viewer.resize(event.width, event.height)
            except Exception as e:
                print(f"Resize error: {e}")

    def _render_loop(self):
        """Render loop called by Tk event system"""
        if self._initialized:
            try:
                self.viewer.render_frame()
            except Exception as e:
                print(f"Render error: {e}")

        # Call again in ~16ms (60 FPS)
        self.after(16, self._render_loop)

    def _on_mouse_down(self, event):
        """Handle mouse button press"""
        self.mouse_pressed = True
        self.last_x = event.x
        self.last_y = event.y

    def _on_mouse_up(self, event):
        """Handle mouse button release"""
        self.mouse_pressed = False

    def _on_mouse_drag(self, event):
        """Handle mouse drag - orbit camera"""
        if self.mouse_pressed and self._initialized:
            dx = event.x - self.last_x
            dy = event.y - self.last_y
            self.viewer.orbit_camera(dx * 0.01, dy * 0.01)
            self.last_x = event.x
            self.last_y = event.y

    def _on_scroll(self, event):
        """Handle scroll wheel - zoom camera"""
        if self._initialized:
            # event.delta is in different units on different platforms
            # On macOS/Windows: 120 units per notch
            zoom_delta = -event.delta * 0.1
            self.viewer.zoom_camera(zoom_delta)

    def on_ready(self, callback):
        """
        Register a callback to be called when the viewer is ready.

        Use this to add visuals after initialization:

        def setup_scene(viewer):
            points = bv.PyPointsVisual.test_cube(viewer, 20)
            viewer.add_points(points)

        canvas.on_ready(setup_scene)
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
        else:
            raise RuntimeError("Viewer not initialized yet. Use on_ready() callback.")


def main():
    """Example usage of BovistaCanvas"""

    # Create Tk window
    root = tk.Tk()
    root.title("Bovista + Tkinter Example")
    root.geometry("800x600")

    # Create Bovista canvas
    canvas = BovistaCanvas(root, width=800, height=600)
    canvas.pack(fill=tk.BOTH, expand=True)

    # Set up camera
    canvas.set_camera_position(0, 0, 500)
    canvas.set_camera_target(128, 128, 128)

    # Define scene setup function
    def setup_scene(viewer):
        """Called when viewer is ready"""
        print("Setting up scene...")

        # Create a 3D volume (256x256x256 random data)
        print("Creating volume data...")
        volume = np.random.randint(0, 255, (256, 256, 256), dtype=np.uint8)

        # Create image visual
        print("Creating image visual...")
        image = bv.PyImageVisual.from_numpy(viewer, volume)
        image.set_slice_z(128.0)
        viewer.add_image(image)

        # Add axes helper
        print("Creating axes...")
        axes = bv.PyLinesVisual.axis_helper(viewer, 200.0)
        viewer.add_lines(axes)

        # Add a wireframe cube
        print("Creating cube...")
        cube = bv.PyLinesVisual.test_cube(viewer)
        viewer.add_lines(cube)

        print("Scene setup complete!")

    # Register setup callback
    canvas.on_ready(setup_scene)

    # Add some UI controls
    control_frame = tk.Frame(root)
    control_frame.pack(side=tk.BOTTOM, fill=tk.X)

    tk.Label(control_frame, text="Mouse: Drag to orbit, Scroll to zoom").pack(side=tk.LEFT, padx=10)

    quit_btn = tk.Button(control_frame, text="Quit", command=root.quit)
    quit_btn.pack(side=tk.RIGHT, padx=10, pady=5)

    # Start Tk event loop
    print("Starting Tk event loop...")
    root.mainloop()


if __name__ == "__main__":
    main()
