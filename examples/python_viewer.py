#!/usr/bin/env python3
"""
Simple interactive viewer example - no setup needed, just run!
"""

import bovista as bv

def main():
    print("Creating viewer...")
    viewer = bv.PyViewer(width=800, height=600)

    # Set camera position
    viewer.set_camera_position(0.0, 0.0, 5.0)
    viewer.set_camera_target(0.0, 0.0, 0.0)

    print("Opening interactive viewer window...")
    print("Controls: Left mouse drag to orbit, scroll to zoom")
    print("Note: Visuals should be added before calling run()")

    # For now, this will show an empty scene
    # Next step: Allow creating visuals without calling initialize() first
    viewer.run()

if __name__ == "__main__":
    main()
