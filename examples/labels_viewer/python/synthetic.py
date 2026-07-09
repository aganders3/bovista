"""Synthetic label-volume generators shared by the labels demos."""

import numpy as np


def make_label_volume(size=256, n_labels=18, seed=0):
    """A `size`^3 uint16 volume: `n_labels` spheres with distinct IDs 1..N on a
    0 background. Overlaps resolve to the higher ID (arbitrary but stable)."""
    rng = np.random.default_rng(seed)
    vol = np.zeros((size, size, size), dtype=np.uint16)
    zz, yy, xx = np.mgrid[0:size, 0:size, 0:size]
    for label_id in range(1, n_labels + 1):
        cz, cy, cx = rng.integers(size * 0.2, size * 0.8, size=3)
        radius = rng.integers(size * 0.06, size * 0.16)
        inside = (zz - cz) ** 2 + (yy - cy) ** 2 + (xx - cx) ** 2 <= radius ** 2
        vol[inside] = label_id
    return vol


def build_lod_pyramid(vol, n_lods=4):
    """Strided (nearest) downsampling by 2x per level — never averages, so IDs
    stay exact. Returns a list of uint16 arrays, finest first."""
    levels = [vol]
    for _ in range(1, n_lods):
        levels.append(np.ascontiguousarray(levels[-1][::2, ::2, ::2]))
    return levels
