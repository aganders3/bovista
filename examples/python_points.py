#!/usr/bin/env python3
"""
Example of creating a point cloud from Python.
"""

import numpy as np
import bovista as bv

def main():
    print("Creating viewer...")
    viewer = bv.PyViewer(width=800, height=600)

    print("Initializing GPU...")
    viewer.initialize()

    # Set up camera
    viewer.set_camera_position(0.0, 0.0, 5.0)
    viewer.set_camera_target(0.0, 0.0, 0.0)

    # Option 1: Use test cube
    print("Creating test cube...")
    points = bv.PyPointsVisual.test_cube(viewer, size=20)
    viewer.add_points(points)

    # Option 2: Create from numpy arrays
    print("Creating custom point cloud...")
    n_points = 1000

    # Random points in a cube [-1, 1]³
    positions = np.random.rand(n_points, 3).astype(np.float32) * 2 - 1

    # Color based on position (normalized to [0, 1])
    colors = (positions + 1) / 2

    # Reshape to required format (N, 1, 3)
    positions = positions.reshape(n_points, 1, 3)
    colors = colors.reshape(n_points, 1, 3)

    custom_points = bv.PyPointsVisual.from_numpy(viewer, positions, colors)
    viewer.add_points(custom_points)

    # Add axis helper
    axes = bv.PyLinesVisual.axis_helper(viewer, 2.0)
    viewer.add_lines(axes)

    print(f"Scene has {viewer.visual_count()} visuals")
    print("Done!")

if __name__ == "__main__":
    main()
