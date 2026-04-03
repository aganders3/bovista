use std::collections::hash_map::DefaultHasher;
use std::hash::{Hash, Hasher};

fn main() {
    let shaders = [
        "src/shaders/virtual_tile.wgsl",
    ];

    // Tell cargo to re-run this script if any shader file changes.
    for shader in &shaders {
        println!("cargo:rerun-if-changed={shader}");
    }

    // Emit a combined content hash as an env var.  image.rs references this via
    // env!("SHADER_HASH"), which forces rustc to recompile image.rs whenever the
    // hash changes — regardless of whether the incremental build cache notices the
    // include_str! dependency.
    let mut hasher = DefaultHasher::new();
    for shader in &shaders {
        let content = std::fs::read_to_string(shader).unwrap_or_default();
        content.hash(&mut hasher);
    }
    println!("cargo:rustc-env=SHADER_HASH={:016x}", hasher.finish());
}
