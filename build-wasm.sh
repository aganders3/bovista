#!/bin/bash
set -e

# Default example name
EXAMPLE="${1:-point_cloud}"

echo "Building Bovista example '$EXAMPLE' for WebAssembly..."

# Find wasm-bindgen (check multiple locations)
WASM_BINDGEN=""
if command -v wasm-bindgen &> /dev/null; then
    WASM_BINDGEN="wasm-bindgen"
elif [ -f "$HOME/.asdf/installs/rust/1.86.0/bin/wasm-bindgen" ]; then
    WASM_BINDGEN="$HOME/.asdf/installs/rust/1.86.0/bin/wasm-bindgen"
elif [ -f "$HOME/.cargo/bin/wasm-bindgen" ]; then
    WASM_BINDGEN="$HOME/.cargo/bin/wasm-bindgen"
else
    echo "wasm-bindgen-cli is not installed. Installing..."
    cargo install wasm-bindgen-cli
    WASM_BINDGEN="wasm-bindgen"
fi

# Add the wasm32 target if not already added
rustup target add wasm32-unknown-unknown 2>/dev/null || true

# Build the example for WASM
echo "Compiling to WebAssembly..."
cargo build --example "$EXAMPLE" --target wasm32-unknown-unknown --release

# Generate JS bindings
echo "Generating JavaScript bindings..."
$WASM_BINDGEN --out-dir web/pkg --target web \
    target/wasm32-unknown-unknown/release/examples/${EXAMPLE}.wasm

echo "Build complete!"
echo ""
echo "To run locally, use a simple HTTP server:"
echo "  cd web && python3 -m http.server 8080"
echo "  or"
echo "  cd web && npx serve"
echo ""
echo "Then open http://localhost:8080 in your browser"
