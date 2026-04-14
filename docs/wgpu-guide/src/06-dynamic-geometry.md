# Dynamic Geometry

One of the coolest aspects of Bovista is **dynamic geometry generation**. Each chunk computes the intersection polygon between a slice plane and its bounding box every frame. Let's see how this works.

## The Problem

You have:
- A 3D volume chunk (AABB in world space)
- A slice plane (position + normal)

You want:
- The polygon where the plane cuts through the chunk
- Texture coordinates for sampling the 3D texture

```
        +--------+
       /|       /|
      / |      / |  ← Chunk (AABB)
     +--------+  |
     |  |     |  |
     |  +-----|--+  ← Slice plane cuts through
     | /      | /
     |/       |/
     +--------+
          ↓
    Intersection polygon
```

## The Algorithm

From `src/visuals/chunk_visual.rs:452-542`:

### Step 1: Test All 12 Edges

An AABB has 12 edges. Test each edge against the plane:

```rust
fn compute_plane_aabb_intersection(
    plane: &SlicePlane,
    min: Vec3,
    max: Vec3,
) -> Option<(Vec<ChunkVertex>, Vec<u16>)> {
    let plane_pos = Vec3::from(plane.position);
    let plane_normal = Vec3::from(plane.normal);

    // 12 edges of the AABB
    let edges = [
        // Bottom face (y = min)
        (Vec3::new(min.x, min.y, min.z), Vec3::new(max.x, min.y, min.z)),
        (Vec3::new(max.x, min.y, min.z), Vec3::new(max.x, min.y, max.z)),
        // ... 10 more edges ...
    ];

    let mut intersection_points = Vec::new();

    // Find intersections with each edge
    for (p0, p1) in edges.iter() {
        if let Some(t) = line_plane_intersection(*p0, *p1, plane_pos, plane_normal) {
            if t >= 0.0 && t <= 1.0 {  // Intersection within edge
                let point = p0.lerp(*p1, t);
                let texcoord = /* ... compute 3D texture coordinate ... */;
                intersection_points.push((point, texcoord));
            }
        }
    }

    // Need at least 3 points for a polygon
    if intersection_points.len() < 3 {
        return None;  // No intersection
    }

    // ... continue ...
}
```

### Step 2: Line-Plane Intersection

Classic computational geometry formula (`chunk_visual.rs:546-557`):

```rust
fn line_plane_intersection(
    p0: Vec3,         // Line start
    p1: Vec3,         // Line end
    plane_pos: Vec3,  // Point on plane
    plane_normal: Vec3,
) -> Option<f32> {
    let ray_dir = p1 - p0;
    let denom = plane_normal.dot(ray_dir);

    // Line parallel to plane?
    if denom.abs() < 1e-6 {
        return None;
    }

    // Solve: plane_normal · (p0 + t * ray_dir - plane_pos) = 0
    let t = plane_normal.dot(plane_pos - p0) / denom;
    Some(t)
}
```

**Math**: The intersection point is `p0 + t * (p1 - p0)` where:
```
plane_normal · (intersection_point - plane_pos) = 0
```

### Step 3: Compute Texture Coordinates

For each intersection point, compute its 3D texture coordinate:

```rust
let texcoord = Vec3::new(
    (point.x - min.x) / (max.x - min.x),  // Normalize X to [0, 1]
    (point.y - min.y) / (max.y - min.y),  // Normalize Y to [0, 1]
    (point.z - min.z) / (max.z - min.z),  // Normalize Z to [0, 1]
);
```

This maps world-space position to texture-space `[0, 1]³`.

### Step 4: Remove Duplicates

Edges can share vertices, so we get duplicate intersection points:

```rust
let epsilon = 0.01;
let mut unique_points = Vec::new();

for point in intersection_points {
    let is_duplicate = unique_points.iter().any(|(p, _)| {
        p.distance(point.0) < epsilon
    });
    if !is_duplicate {
        unique_points.push(point);
    }
}
```

### Step 5: Sort Points

The intersection points form a **convex polygon**, but they're unordered. Sort them counter-clockwise around the polygon's centroid (`chunk_visual.rs:560-604`):

```rust
fn sort_points_on_plane(points: &[(Vec3, Vec3)], plane_normal: Vec3) -> Vec<(Vec3, Vec3)> {
    // Find centroid
    let centroid = points.iter().map(|(p, _)| *p).sum::<Vec3>() / points.len() as f32;

    // Create coordinate system on plane
    let up = if plane_normal.y.abs() < 0.9 { Vec3::Y } else { Vec3::X };
    let tangent = plane_normal.cross(up).normalize();
    let bitangent = plane_normal.cross(tangent).normalize();

    // Sort by angle around centroid
    let mut sorted = points.to_vec();
    sorted.sort_by(|(a, _), (b, _)| {
        let va = *a - centroid;
        let vb = *b - centroid;

        // Compute angle using atan2
        let angle_a = va.dot(tangent).atan2(va.dot(bitangent));
        let angle_b = vb.dot(tangent).atan2(vb.dot(bitangent));

        angle_a.partial_cmp(&angle_b).unwrap_or(std::cmp::Ordering::Equal)
    });

    sorted
}
```

**Why this works**: We project each point onto the plane's coordinate system (tangent, bitangent), then sort by angle. This gives counter-clockwise order.

### Step 6: Generate Triangle Fan

Convert the sorted polygon into triangles:

```rust
// Polygon: v0, v1, v2, v3, v4, ...
// Triangles: (v0, v1, v2), (v0, v2, v3), (v0, v3, v4), ...

let mut indices = Vec::new();
for i in 1..(vertices.len() - 1) {
    indices.push(0);          // Always start from v0
    indices.push(i as u16);
    indices.push((i + 1) as u16);
}
```

**Result**: `n` vertices → `n-2` triangles (standard for convex polygons).

## Updating Geometry Each Frame

From `chunk_visual.rs:343-396`:

```rust
fn prepare(&mut self, _device: &wgpu::Device, queue: &wgpu::Queue) {
    if self.needs_vertex_update {
        // Compute intersection
        let (vertices, indices) = compute_plane_aabb_intersection(
            &self.slice_plane,
            self.world_position,
            self.world_position + chunk_extents,
        )?;

        // Write to buffers
        queue.write_buffer(&self.vertex_buffer, 0, bytemuck::cast_slice(&vertices));
        queue.write_buffer(&self.index_buffer, 0, bytemuck::cast_slice(&indices));

        self.current_vertex_count = indices.len() as u32;
        self.needs_vertex_update = false;
    }

    // ... update uniforms ...
}
```

**Key pattern**:
1. Pre-allocate buffers for **maximum case** (8 vertices, triangle fan)
2. Compute geometry when slice plane changes
3. Write only the needed vertices/indices
4. Store vertex count for draw call

**Draw call**:
```rust
render_pass.draw_indexed(0..self.current_vertex_count, 0, 0..1);
```

## Why This Approach?

**Alternative 1**: Render full chunk as 3D texture-mapped cube
- ❌ Wastes fill rate (rasterizes many occluded fragments)
- ❌ Hard to get sharp slice (interpolation between faces)

**Alternative 2**: Use geometry shader to generate slice
- ❌ Geometry shaders are slow
- ❌ Not supported in WebGPU

**Alternative 3**: Compute shader generates geometry
- ✅ Could work, but more complex
- ❌ Requires indirect drawing support

**Our approach** (CPU-side geometry generation):
- ✅ Simple, works everywhere
- ✅ Minimal geometry (only the slice polygon)
- ✅ Perfect texture sampling (3D coordinates match exactly)
- ✅ Fast enough (12 edge tests, small sort)

## Visualization of the Process

```
1. Chunk AABB + Plane
   +--------+
   |   /    |  ← Plane cuts through
   |  /     |
   | /      |
   +--------+

2. Test all 12 edges, find 6 intersections
   +--●-----+
   | / \    |
   ●   ●    |
   | \   \  |
   +--●---●-+
      \   /
       ●

3. Sort points CCW
   +--1-----+
   | / \    |
   2   6    |
   | \   \  |
   +--3---5-+
      \   /
       4

4. Triangle fan
   +--1-----+
   | /●\    |
   2   6    |  ← v0 = centroid
   | \●  \  |
   +--3---5-+
      \●  /
       4

5. Render with 3D texture sampling
   [Grayscale slice image]
```

## Performance Considerations

**Cost per chunk per frame**:
- 12 line-plane intersection tests: ~200 cycles
- Duplicate removal: O(n²) but n ≤ 8
- Sorting 3-8 points: O(n log n), negligible
- Buffer write: 1 queue operation

**Optimization**: Only recompute when slice plane changes:

```rust
pub fn set_slice_plane(&mut self, plane: SlicePlane) {
    if self.slice_plane != plane {  // Compare planes
        self.slice_plane = plane;
        self.needs_vertex_update = true;
    }
}
```

**Result**: Typical case is ~100 visible chunks, recompute takes <1ms.

## Edge Cases

### No Intersection

```rust
if intersection_points.len() < 3 {
    return None;
}
```

Chunk not rendered—it doesn't intersect the slice.

### Degenerate Cases

**Plane exactly on edge**: Two edge intersections at same point
→ Duplicate removal handles it

**Plane exactly on vertex**: Multiple edges intersect at same point
→ Duplicate removal collapses to single point

**Plane parallel to face**: No intersections or all intersections
→ Returns None (correct: thin/no slice)

## Extending This Pattern

This technique generalizes to other dynamic geometry:

**Cutting plane for meshes**:
```rust
fn mesh_plane_intersection(mesh: &Mesh, plane: Plane) -> Vec<LineSegment> {
    // For each triangle, test edges against plane
    // Return intersection line segments
}
```

**Isosurface extraction** (marching cubes):
```rust
fn extract_isosurface(volume: &Volume, isovalue: f32) -> Mesh {
    // For each voxel cube, generate triangles at isosurface
}
```

**Bounding volume visualization**:
```rust
fn aabb_wireframe(min: Vec3, max: Vec3) -> Vec<LineVertex> {
    // Generate 12 line segments for AABB edges
}
```

## Summary

**Key Techniques**:
1. **Line-plane intersection** for computational geometry
2. **Pre-allocated buffers** updated dynamically
3. **Triangle fan** for convex polygons
4. **Dirty flags** to minimize recomputation

**Performance**:
- CPU cost: <10µs per chunk
- GPU cost: Minimal (only render what's needed)

**Flexibility**:
- Works with any chunk size
- Works with any slice plane orientation
- Automatic handling of edge cases

---

**Next**: [3D Textures →](./07-3d-textures.md)
