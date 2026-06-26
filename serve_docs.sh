#!/bin/bash
set -e

# Build and serve the mdBook docs + live web examples together.
# Mirrors the CI deploy-docs workflow but serves locally instead of deploying.
#
# Usage:
#   ./serve_docs.sh          # build everything and serve
#
# Always rebuilds the WASM — it's fast, and serving a stale bovista_bg.wasm
# (out of sync with the Rust source) is a confusing footgun not worth saving
# a few seconds for.

BOOK_DIR="docs/guide/book"
LIVE_DIR="$BOOK_DIR/examples/live"
PORT=8000

# 1. WASM
echo "Building WASM..."
./build_wasm.sh

# 2. mdBook
if ! command -v mdbook &> /dev/null; then
    echo "mdbook not found — installing..."
    cargo install mdbook --locked
fi
echo "Building docs..."
mdbook build docs/guide

# 2b. Rust API docs (rustdoc) → served at /api/bovista/
echo "Building Rust API docs..."
cargo doc --no-deps -p bovista
rm -rf "$BOOK_DIR/api"
cp -r target/doc "$BOOK_DIR/api"

# 3. Assemble: copy examples into the book output, rewrite pkg path
mkdir -p "$LIVE_DIR"
cp -r examples/slice_renderer/web    "$LIVE_DIR/slice_renderer"
cp -r examples/volume_renderer/web "$LIVE_DIR/volume_renderer"
cp -r examples/pkg                 "$LIVE_DIR/pkg"
cp examples/index.html             "$LIVE_DIR/index.html"

# Deployed layout has examples/live/{viewer}/main.js one level from pkg;
# the source files use ../../pkg (two levels from examples/*/web/).
sed -i '' "s|../../pkg/bovista\.js|../pkg/bovista.js|g" \
    "$LIVE_DIR/slice_renderer/main.js" \
    "$LIVE_DIR/volume_renderer/main.js"

echo ""
echo "Serving at http://localhost:$PORT"
echo "  Docs:             http://localhost:$PORT"
echo "  Examples hub:     http://localhost:$PORT/examples/live/"
echo "  Slice viewer:     http://localhost:$PORT/examples/live/slice_renderer/"
echo "  Volume renderer:  http://localhost:$PORT/examples/live/volume_renderer/"
echo "  Rust API docs:    http://localhost:$PORT/api/bovista/"
echo ""
echo "Press Ctrl-C to stop."
python3 -m http.server $PORT --directory "$BOOK_DIR"
