#!/bin/bash
# Build script for WGPU guide

set -e

echo "Building WGPU for OpenGL Developers guide..."

# Check if mdbook is installed
if ! command -v mdbook &> /dev/null; then
    echo "Error: mdbook is not installed"
    echo "Install it with: cargo install mdbook"
    exit 1
fi

# Build the book
mdbook build

echo "✓ Book built successfully!"
echo ""
echo "To view the book:"
echo "  1. Run: mdbook serve"
echo "  2. Open: http://localhost:3000"
echo ""
echo "Or open the built HTML directly:"
echo "  open book/index.html"
