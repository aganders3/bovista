#!/usr/bin/env python3
"""
Test getting native window handle from Tkinter on macOS.
"""

import tkinter as tk
import ctypes
import sys

def main():
    root = tk.Tk()
    root.geometry("800x600")
    root.title("Test Window Handle")

    # On macOS, Tk uses NSView
    # We need to get the window ID which is a pointer
    window_id = root.winfo_id()

    print(f"Platform: {sys.platform}")
    print(f"Window ID: {window_id}")
    print(f"Window ID (hex): {hex(window_id)}")
    print(f"Type: {type(window_id)}")

    # Try to get more info
    try:
        print(f"Window info: {root.winfo_name()}")
        print(f"Window class: {root.winfo_class()}")
    except Exception as e:
        print(f"Error: {e}")

    root.mainloop()

if __name__ == "__main__":
    main()
