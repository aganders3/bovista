//! Tiny CLI flag parser shared by the native examples. No clap dependency —
//! just the handful of flags the viewers need. Supports both `--flag value`
//! and `--flag=value` forms.
//!
//! This module is `#[path]`-included into each example binary and must stay
//! self-contained (no `super::`).

#![allow(dead_code)]

/// Parsed CLI options. The volume-only fields are ignored by slice_viewer.
pub struct CliArgs {
    /// OME-Zarr root (filesystem path or http(s):// URL).
    pub zarr: Option<String>,
    /// Atlas slot count override (`--cache-tiles`).
    pub cache_tiles: Option<u32>,
    /// Number of physical atlas textures (`--atlas-count`, default 1).
    pub atlas_count: usize,
    /// Max in-flight tile reads (`--max-inflight`); None → auto budget.
    pub max_inflight: Option<usize>,
    /// Shader LOD bias (`--lod-bias`).
    pub lod_bias: f32,
    /// Contrast window floor (`--contrast-min`).
    pub contrast_min: f32,
    /// Contrast window ceiling (`--contrast-max`).
    pub contrast_max: f32,

    // ── Volume-only ──────────────────────────────────────────────────────
    /// Render mode string (`--mode`): direct | mip | minip | average | iso.
    pub mode: Option<String>,
    /// Density multiplier on the auto heuristic (`--density-mult`).
    pub density_mult: f32,
    /// MIP attenuation (`--attenuation`).
    pub attenuation: f32,
    /// Isosurface threshold (`--iso-threshold`).
    pub iso_threshold: f32,
    /// Per-step relative size (`--step`).
    pub step: Option<f32>,
}

pub const KNOWN_FLAGS: &[&str] = &[
    "--zarr",
    "--cache-tiles",
    "--atlas-count",
    "--max-inflight",
    "--lod-bias",
    "--contrast-min",
    "--contrast-max",
    "--mode",
    "--density-mult",
    "--attenuation",
    "--iso-threshold",
    "--step",
];

impl CliArgs {
    /// Parse `std::env::args()` (skipping argv[0]). Exits the process on an
    /// unknown flag so typos surface immediately.
    pub fn parse() -> Self {
        let args: Vec<String> = std::env::args().skip(1).collect();
        check_unknown_flags(&args, KNOWN_FLAGS);
        Self {
            zarr: flag_str_opt(&args, "--zarr"),
            cache_tiles: flag_str_opt(&args, "--cache-tiles").and_then(|v| v.parse().ok()),
            atlas_count: flag_str_opt(&args, "--atlas-count")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1),
            max_inflight: flag_str_opt(&args, "--max-inflight").and_then(|v| v.parse().ok()),
            lod_bias: flag_str_opt(&args, "--lod-bias")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            contrast_min: flag_str_opt(&args, "--contrast-min")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            contrast_max: flag_str_opt(&args, "--contrast-max")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
            mode: flag_str_opt(&args, "--mode"),
            density_mult: flag_str_opt(&args, "--density-mult")
                .and_then(|v| v.parse().ok())
                .unwrap_or(1.0),
            attenuation: flag_str_opt(&args, "--attenuation")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.0),
            iso_threshold: flag_str_opt(&args, "--iso-threshold")
                .and_then(|v| v.parse().ok())
                .unwrap_or(0.5),
            step: flag_str_opt(&args, "--step").and_then(|v| v.parse().ok()),
        }
    }

    /// Resolve `--zarr`, falling back to `default_url` when omitted.
    pub fn zarr_or(&self, default_url: &str) -> String {
        self.zarr.clone().unwrap_or_else(|| default_url.to_string())
    }

    /// Budget in-flight worker threads against the CPU count, leaving room for
    /// the render + rayon pools. Mirrors orbit_stream's heuristic.
    pub fn resolved_max_inflight(&self) -> usize {
        if let Some(n) = self.max_inflight {
            return n;
        }
        let rayon_threads: usize = std::env::var("RAYON_NUM_THREADS")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(8);
        let cpus = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(8);
        cpus.saturating_sub(2).saturating_sub(rayon_threads).max(8)
    }
}

pub fn check_unknown_flags(args: &[String], known: &[&str]) {
    let mut i = 0;
    while i < args.len() {
        let a = &args[i];
        if a.starts_with("--") {
            let name = a.split('=').next().unwrap();
            if !known.contains(&name) {
                eprintln!("error: unknown flag {:?}", a);
                eprintln!("known flags: {}", known.join(", "));
                std::process::exit(2);
            }
            if !a.contains('=') && i + 1 < args.len() && !args[i + 1].starts_with("--") {
                i += 1;
            }
        }
        i += 1;
    }
}

pub fn flag_str_opt(args: &[String], name: &str) -> Option<String> {
    let eq = format!("{}=", name);
    let mut i = 0;
    while i < args.len() {
        if args[i] == name {
            return args.get(i + 1).cloned();
        }
        if let Some(rest) = args[i].strip_prefix(&eq) {
            return Some(rest.to_string());
        }
        i += 1;
    }
    None
}
