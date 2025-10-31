#!/bin/bash
set -e

# Build WASM example for browser deployment
# Usage: ./build_wasm.sh [example_name]

EXAMPLE=${1:-wasm_zarr}
OUTPUT_DIR="web/pkg"

echo "🔨 Building WASM example: $EXAMPLE"

# Add the wasm32 target if not already added
rustup target add wasm32-unknown-unknown 2>/dev/null || true

# Find wasm-bindgen (check multiple locations)
WASM_BINDGEN=""
if command -v wasm-bindgen &> /dev/null; then
    WASM_BINDGEN="wasm-bindgen"
elif [ -f "$HOME/.asdf/installs/rust/1.86.0/bin/wasm-bindgen" ]; then
    WASM_BINDGEN="$HOME/.asdf/installs/rust/1.86.0/bin/wasm-bindgen"
elif [ -f "$HOME/.cargo/bin/wasm-bindgen" ]; then
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
else
    echo "❌ wasm-bindgen not found. Installing..."
    cargo install wasm-bindgen-cli
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
fi

# Build for wasm32
echo "📦 Compiling to WebAssembly..."
cargo build --example $EXAMPLE --target wasm32-unknown-unknown --release

# Generate JS bindings
echo "🔗 Generating JavaScript bindings..."
$WASM_BINDGEN \
    --out-dir $OUTPUT_DIR \
    --out-name $EXAMPLE \
    --target web \
    --no-typescript \
    target/wasm32-unknown-unknown/release/examples/${EXAMPLE}.wasm

echo ""
echo "✅ Build complete!"
echo "📦 Output: $OUTPUT_DIR/${EXAMPLE}.js"
echo ""
echo "To test locally:"
echo "  python3 -m http.server 8000 --directory web"
echo "  open http://localhost:8000"
echo ""
