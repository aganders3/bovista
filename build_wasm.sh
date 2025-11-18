#!/bin/bash
set -e

# Build WASM library for browser deployment
# Usage: ./build_wasm.sh

OUTPUT_DIR="web/pkg"
OUTPUT_NAME="bovista"

echo "🔨 Building WASM library: bovista"

# Add the wasm32 target if not already added
rustup target add wasm32-unknown-unknown 2>/dev/null || true

# Find wasm-bindgen (check multiple locations)
WASM_BINDGEN=""
if command -v wasm-bindgen &> /dev/null; then
    WASM_BINDGEN="wasm-bindgen"
elif [ -f "$HOME/.cargo/bin/wasm-bindgen" ]; then
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
else
    echo "❌ wasm-bindgen not found. Installing..."
    cargo install wasm-bindgen-cli
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
fi

# Build for wasm32
echo "📦 Compiling to WebAssembly..."
cargo build --lib --target wasm32-unknown-unknown --release

# Generate JS bindings
echo "🔗 Generating JavaScript bindings..."
$WASM_BINDGEN \
    --out-dir $OUTPUT_DIR \
    --out-name $OUTPUT_NAME \
    --target web \
    --no-typescript \
    target/wasm32-unknown-unknown/release/bovista.wasm

echo ""
echo "✅ Build complete!"
echo "📦 Output: $OUTPUT_DIR/${OUTPUT_NAME}.js"
echo "📦 WASM: $OUTPUT_DIR/${OUTPUT_NAME}_bg.wasm"
echo ""
echo "To test locally:"
echo "  python3 -m http.server 8000 --directory web"
echo "  open http://localhost:8000/remote_ome_zarr.html"
echo ""
