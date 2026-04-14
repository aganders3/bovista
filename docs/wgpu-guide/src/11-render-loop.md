# The Render Loop

Understanding the render loop is crucial for WGPU. Let's walk through a complete frame using Bovista's `Renderer::render()` method.

## OpenGL Render Loop (Recap)

```c
while (!glfwWindowShouldClose(window)) {
    // Update state
    update_camera();

    // Clear
    glClear(GL_COLOR_BUFFER_BIT | GL_DEPTH_BUFFER_BIT);

    // Bind state and draw
    glUseProgram(program);
    glBindVertexArray(vao);
    glDrawElements(GL_TRIANGLES, count, GL_UNSIGNED_INT, 0);

    // Present
    glfwSwapBuffers(window);
    glfwPollEvents();
}
```

**Key point**: Commands execute immediately (or appear to).

## WGPU Render Loop

In WGPU, you explicitly build command buffers and submit them. From `src/renderer.rs:140-184`:

```rust
pub fn render(
    &self,
    scene: &Scene,
    view: &wgpu::TextureView,        // Render target
    depth_view: &wgpu::TextureView,   // Depth buffer
    clear_color: wgpu::Color,
) {
    // 1. Create command encoder
    let mut encoder = self.device.create_command_encoder(
        &wgpu::CommandEncoderDescriptor {
            label: Some("Render Encoder"),
        }
    );

    // 2. Begin render pass (scoped)
    {
        let mut render_pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("Render Pass"),
            color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                view,
                resolve_target: None,
                ops: wgpu::Operations {
                    load: wgpu::LoadOp::Clear(clear_color),
                    store: wgpu::StoreOp::Store,
                },
            })],
            depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                view: depth_view,
                depth_ops: Some(wgpu::Operations {
                    load: wgpu::LoadOp::Clear(1.0),
                    store: wgpu::StoreOp::Store,
                }),
                stencil_ops: None,
            }),
            timestamp_writes: None,
            occlusion_query_set: None,
        });

        // 3. Set shared camera bind group
        render_pass.set_bind_group(0, &self.camera_bind_group, &[]);

        // 4. Render all visuals in the scene
        scene.render(&mut render_pass);

    }  // render_pass dropped here, ending the pass

    // 5. Submit command buffer
    self.queue.submit(std::iter::once(encoder.finish()));
}
```

Let's break this down step by step.

## Step 1: Create Command Encoder

```rust
let mut encoder = self.device.create_command_encoder(&descriptor);
```

The **command encoder** is like a recording session. You record commands (clears, draws, copies) into it, then submit the whole buffer to the GPU at once.

**OpenGL analogy**: Like building a display list, except you always use them.

**Benefits**:
- Build command buffers on multiple threads
- Replay same commands by resubmitting buffer
- Clear separation between CPU and GPU timelines

## Step 2: Begin Render Pass

A **render pass** defines:
1. What you're rendering to (render targets)
2. What to do at the start (clear? load existing?)
3. What to do at the end (store? discard?)

### Color Attachment

```rust
color_attachments: &[Some(wgpu::RenderPassColorAttachment {
    view,  // The texture to render to
    resolve_target: None,  // For MSAA resolve (None = no MSAA)
    ops: wgpu::Operations {
        load: wgpu::LoadOp::Clear(clear_color),  // Clear at start
        store: wgpu::StoreOp::Store,              // Save at end
    },
})],
```

**Load operations**:
- `LoadOp::Clear(color)`: Clear to specified color (like `glClear`)
- `LoadOp::Load`: Keep existing contents

**Store operations**:
- `StoreOp::Store`: Save results (usually what you want)
- `StoreOp::Discard`: Don't save (optimization for mobile GPUs)

**OpenGL equivalent**:
```c
glClear(GL_COLOR_BUFFER_BIT);  // LoadOp::Clear
// (StoreOp is implicit)
```

### Depth/Stencil Attachment

```rust
depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
    view: depth_view,
    depth_ops: Some(wgpu::Operations {
        load: wgpu::LoadOp::Clear(1.0),  // Clear to far plane
        store: wgpu::StoreOp::Store,      // Save depth buffer
    }),
    stencil_ops: None,  // No stencil
}),
```

**OpenGL equivalent**:
```c
glClear(GL_DEPTH_BUFFER_BIT);
glDepthMask(GL_TRUE);
```

**Why explicit load/store?** Mobile GPUs use tile-based rendering. Knowing whether to load/store enables huge bandwidth optimizations.

## Step 3: Set Shared Bind Groups

```rust
render_pass.set_bind_group(0, &self.camera_bind_group, &[]);
```

The renderer sets the **camera bind group** (Group 0) once for all visuals. Each visual then sets its own resources at Group 1+.

**Pattern**:
```
Group 0: Shared resources (camera, lighting)  ← Set once per frame
Group 1: Per-object resources (texture, uniforms)  ← Set per draw call
Group 2: Per-material resources  ← Set when material changes
```

## Step 4: Render Scene

```rust
scene.render(&mut render_pass);
```

The scene iterates over all visuals and calls their `render()` methods. Let's look at what a visual does (from `chunk_visual.rs:417-427`):

```rust
fn render(&self, render_pass: &mut RenderPass) {
    if self.current_vertex_count == 0 {
        return;  // Early out if no geometry
    }

    // Set pipeline
    render_pass.set_pipeline(&self.render_pipeline);

    // Set visual-specific bind group (Group 1)
    render_pass.set_bind_group(1, &self.bind_group, &[]);

    // Set vertex and index buffers
    render_pass.set_vertex_buffer(0, self.vertex_buffer.slice(..));
    render_pass.set_index_buffer(self.index_buffer.slice(..), wgpu::IndexFormat::Uint16);

    // Draw!
    render_pass.draw_indexed(0..self.current_vertex_count, 0, 0..1);
}
```

**OpenGL equivalent**:
```c
if (vertex_count == 0) return;

glUseProgram(program);
glBindVertexArray(vao);
glBindTexture(GL_TEXTURE_3D, texture);
glDrawElements(GL_TRIANGLES, vertex_count, GL_UNSIGNED_SHORT, 0);
```

### Draw Calls

**Indexed drawing**:
```rust
render_pass.draw_indexed(
    indices: Range<u32>,     // Which indices to use (0..count)
    base_vertex: i32,        // Offset added to each index
    instances: Range<u32>,   // Instance range (0..1 = non-instanced)
);
```

**Non-indexed drawing**:
```rust
render_pass.draw(
    vertices: Range<u32>,    // Vertex range
    instances: Range<u32>,   // Instance range
);
```

**Instanced drawing**:
```rust
// Draw 100 instances
render_pass.draw_indexed(0..index_count, 0, 0..100);
```

### Buffer Slices

You can use subranges of buffers:

```rust
// Use entire buffer
render_pass.set_vertex_buffer(0, buffer.slice(..));

// Use subset
render_pass.set_vertex_buffer(0, buffer.slice(1024..2048));
```

**Benefit**: Batch multiple objects in one buffer, use slices to select.

## Step 5: End Render Pass

The render pass ends when it's dropped:

```rust
{
    let mut render_pass = encoder.begin_render_pass(&descriptor);
    // ... draw calls ...
}  // ← render_pass dropped here, automatically ends
```

**Explicit end** (alternative):
```rust
let render_pass = encoder.begin_render_pass(&descriptor);
// ... draw calls ...
drop(render_pass);  // Explicitly end
```

## Step 6: Submit Commands

```rust
self.queue.submit(std::iter::once(encoder.finish()));
```

**What happens**:
1. `encoder.finish()` converts encoder into a `CommandBuffer`
2. `queue.submit()` sends command buffer to GPU
3. GPU executes commands asynchronously

**Batch submissions**:
```rust
let buffer1 = encoder1.finish();
let buffer2 = encoder2.finish();
queue.submit([buffer1, buffer2]);  // Submit multiple buffers
```

## Complete Frame Flow

Here's the full flow in Bovista:

```rust
// Application loop
loop {
    // 1. Update camera
    camera.update(delta_time);
    renderer.update_camera(&camera);  // Write to uniform buffer

    // 2. Prepare scene (update buffers, load chunks, etc.)
    scene.prepare(renderer.device(), renderer.queue());

    // 3. Get next frame from surface
    let output = surface.get_current_texture().unwrap();
    let view = output.texture.create_view(&Default::default());

    // 4. Render
    renderer.render(&scene, &view, &depth_view, clear_color);

    // 5. Present
    output.present();

    // 6. Handle input
    window.poll_events();
}
```

## Multiple Render Passes

You can have multiple passes per frame:

```rust
let mut encoder = device.create_command_encoder(&descriptor);

// Pass 1: Shadow map
{
    let mut shadow_pass = encoder.begin_render_pass(&shadow_descriptor);
    render_shadow_casters(&mut shadow_pass);
}

// Pass 2: Main scene
{
    let mut main_pass = encoder.begin_render_pass(&main_descriptor);
    render_scene(&mut main_pass);
}

// Pass 3: Post-processing
{
    let mut post_pass = encoder.begin_render_pass(&post_descriptor);
    render_fullscreen_quad(&mut post_pass);
}

queue.submit(std::iter::once(encoder.finish()));
```

## Render Pass Optimization

### Minimize Load/Store Operations

**Bad** (mobile):
```rust
// Pass 1: Render geometry
ops: Operations {
    load: LoadOp::Clear(color),
    store: StoreOp::Store,  // Save to memory
}

// Pass 2: Render UI on top
ops: Operations {
    load: LoadOp::Load,  // Load from memory
    store: StoreOp::Store,
}
```

**Good** (mobile):
```rust
// Single pass: Render geometry then UI
ops: Operations {
    load: LoadOp::Clear(color),
    store: StoreOp::Store,  // Only one store
}
```

**Why?** Tile-based GPUs (mobile, Apple Silicon) keep tiles in cache. Load/Store forces memory reads/writes.

### Discard When Possible

**Example: Depth buffer you don't read later**:
```rust
depth_ops: Some(Operations {
    load: LoadOp::Clear(1.0),
    store: StoreOp::Discard,  // Don't save, we're done with it
}),
```

## Profiling Render Passes

Use **timestamp queries** to measure GPU time:

```rust
// Create query set
let query_set = device.create_query_set(&QuerySetDescriptor {
    label: Some("Timestamps"),
    ty: QueryType::Timestamp,
    count: 2,
});

// Record timestamps
render_pass.write_timestamp(&query_set, 0);  // Start
// ... draw calls ...
render_pass.write_timestamp(&query_set, 1);  // End

// Resolve to buffer and read back
// (see wgpu docs for full example)
```

## Common Patterns

### Clear + Draw + Present

```rust
let encoder = /* ... */;
{
    let mut render_pass = encoder.begin_render_pass(/* clear color */);
    render_pass.set_pipeline(&pipeline);
    render_pass.set_bind_group(0, &bind_group, &[]);
    render_pass.draw(0..3, 0..1);
}
queue.submit(Some(encoder.finish()));
output.present();
```

### Load + Draw Over + Present

```rust
let encoder = /* ... */;
{
    let mut render_pass = encoder.begin_render_pass(/* load existing */);
    // Draw UI on top
    render_pass.set_pipeline(&ui_pipeline);
    render_pass.draw(0..vertex_count, 0..1);
}
queue.submit(Some(encoder.finish()));
output.present();
```

## Summary

| Concept | OpenGL | WGPU |
|---------|--------|------|
| **Commands** | Immediate | Deferred (command buffer) |
| **Clear** | `glClear` | `LoadOp::Clear` |
| **Render Pass** | Implicit | Explicit `begin_render_pass` |
| **Draw** | `glDraw*` | `render_pass.draw*` |
| **Submit** | Implicit | `queue.submit()` |
| **Present** | `swapBuffers` | `output.present()` |

**Key Takeaway**: WGPU's explicit command buffer model gives you control over the GPU timeline and enables optimizations impossible in immediate-mode APIs.

---

**Next**: [Dynamic Geometry →](./06-dynamic-geometry.md)
