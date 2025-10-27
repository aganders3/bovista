#!/usr/bin/env python3
"""
Simple example of using Bovista from Python to visualize a 3D volume.
"""

import numpy as np
import bovista as bv


def main():
    print("Creating viewer...")
    viewer = bv.PyViewer(width=800, height=600)

    # Set up camera to view a 256³ volume
    viewer.set_camera_position(0.0, 0.0, 500.0)
    viewer.set_camera_target(128.0, 128.0, 128.0)

    # Note: For interactive windows with run(), we DON'T call initialize()
    # The run() method handles all GPU initialization internally
    #
    # TODO: Currently cannot add visuals before run() because they need a renderer
    # This will be fixed in a future version

    print(f"Scene has {viewer.visual_count()} visuals")
    print("Opening interactive viewer window...")
    print("Controls: Left mouse drag to orbit, scroll to zoom")
    viewer.run()


if __name__ == "__main__":
    main()
