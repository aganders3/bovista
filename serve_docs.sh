#!/bin/bash
set -e

# Build and serve the mdBook docs + live web examples together.
# Mirrors the CI deploy-docs workflow but serves locally instead of deploying.
#
# Usage:
#   ./serve_docs.sh          # build everything and serve
#   ./serve_docs.sh --skip-wasm  # skip WASM rebuild (faster if already built)

SKIP_WASM=false
for arg in "$@"; do
    case $arg in
        --skip-wasm) SKIP_WASM=true ;;
    esac
done

BOOK_DIR="docs/wgpu-guide/book"
LIVE_DIR="$BOOK_DIR/examples/live"
PORT=8000

# 1. WASM
if [ "$SKIP_WASM" = false ]; then
    echo "Building WASM..."
    ./build_wasm.sh
else
    echo "Skipping WASM build (--skip-wasm)"
fi

# 2. mdBook
if ! command -v mdbook &> /dev/null; then
    echo "mdbook not found — installing..."
    cargo install mdbook mdbook-toc --locked
fi
echo "Building docs..."
mdbook build docs/wgpu-guide

# 3. Assemble: copy examples into the book output, rewrite pkg path
mkdir -p "$LIVE_DIR"
cp -r examples/slice_viewer/web    "$LIVE_DIR/slice_viewer"
cp -r examples/volume_renderer/web "$LIVE_DIR/volume_renderer"
cp -r examples/pkg                 "$LIVE_DIR/pkg"
cp examples/index.html             "$LIVE_DIR/index.html"

# Deployed layout has examples/live/{viewer}/main.js one level from pkg;
# the source files use ../../pkg (two levels from examples/*/web/).
sed -i '' "s|../../pkg/bovista\.js|../pkg/bovista.js|g" \
    "$LIVE_DIR/slice_viewer/main.js" \
    "$LIVE_DIR/volume_renderer/main.js"

echo ""
echo "Serving at http://localhost:$PORT"
echo "  Docs:             http://localhost:$PORT"
echo "  Examples hub:     http://localhost:$PORT/examples/live/"
echo "  Slice viewer:     http://localhost:$PORT/examples/live/slice_viewer/"
echo "  Volume renderer:  http://localhost:$PORT/examples/live/volume_renderer/"
echo ""
echo "Press Ctrl-C to stop."
python3 -m http.server $PORT --directory "$BOOK_DIR"
