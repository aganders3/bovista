# Key Takeaways

Let's consolidate everything you've learned about WGPU vs OpenGL.

## Conceptual Differences

### State Management

| OpenGL | WGPU |
|--------|------|
| **Global state machine** | **Explicit per-command** |
| `glBindBuffer(buf)` affects all future calls | Must pass buffer to each command |
| State persists until changed | State never persists |
| Easy to make mistakes | Harder to make mistakes |

**Example**:
```rust
// OpenGL: Accidentally using wrong buffer
glBindBuffer(GL_ARRAY_BUFFER, buffer1);
// ... 100 lines later ...
glBufferData(...);  // Which buffer? Who knows!

// WGPU: Always explicit
queue.write_buffer(&buffer1, 0, data);  // Clear which buffer
```

### Resource Binding

| OpenGL | WGPU |
|--------|------|
| Bind by name/location | Bind groups with layouts |
| `glUniform*` by name | Structured groups |
| No compile-time validation | Validated at pipeline creation |

**Benefit**: WGPU catches binding mismatches early, not at draw time.

### Pipelines

| OpenGL | WGPU |
|--------|------|
| **Mutable state** | **Immutable pipelines** |
| Change depth function anytime | Baked into pipeline |
| Expensive state changes | Fast pipeline switches |
| Runtime validation | Creation-time validation |

**Pattern**: Create multiple pipelines for different states, switch between them.

## Practical Implications

### More Verbose, But Safer

**OpenGL** (short, but risky):
```c
glBindTexture(GL_TEXTURE_2D, tex);
glDrawArrays(GL_TRIANGLES, 0, 3);
```

**WGPU** (verbose, but explicit):
```rust
render_pass.set_pipeline(&pipeline);
render_pass.set_bind_group(0, &camera_bind_group, &[]);
render_pass.set_bind_group(1, &texture_bind_group, &[]);
render_pass.set_vertex_buffer(0, vertex_buffer.slice(..));
render_pass.draw(0..3, 0..1);
```

**Trade-off**: More typing, but fewer bugs and better performance.

### Initialization is Front-Loaded

**OpenGL**: Create resources as needed
```c
GLuint buffer;
glGenBuffers(1, &buffer);  // Quick!
glBindBuffer(...);
glBufferData(...);  // Quick!
```

**WGPU**: More setup upfront
```rust
// Define layout
let layout = create_bind_group_layout(...);  // More work

// Create pipeline
let pipeline = create_render_pipeline(...);  // Validates everything

// Then rendering is fast
```

**Pattern**: Do heavy work at startup, rendering is faster.

### Command Buffers Enable Parallelism

**OpenGL**: Single-threaded
```c
// Must happen on main thread
glDrawArrays(...);
```

**WGPU**: Multi-threaded command recording
```rust
// Thread 1
let encoder1 = device.create_command_encoder(...);
// ... record commands ...

// Thread 2
let encoder2 = device.create_command_encoder(...);
// ... record commands ...

// Main thread: submit both
queue.submit([encoder1.finish(), encoder2.finish()]);
```

**Benefit**: Build complex scenes faster with multiple threads.

## When to Use Each

### Use OpenGL When:

- ✅ Quick prototyping
- ✅ Legacy codebase
- ✅ Simple rendering needs
- ✅ Maximum compatibility (old hardware)
- ✅ Tutorials/learning basics

### Use WGPU When:

- ✅ Production applications
- ✅ Cross-platform (native + web)
- ✅ Complex rendering pipelines
- ✅ Performance-critical code
- ✅ Want modern best practices
- ✅ Multi-threaded rendering

## Common Patterns

### Pattern 1: Renderer + Scene

```rust
struct Renderer {
    device: Device,
    queue: Queue,
    shared_resources: BindGroup,  // Camera, lights, etc.
}

struct Scene {
    visuals: Vec<Visual>,
}

impl Renderer {
    fn render(&self, scene: &Scene) {
        let encoder = self.device.create_command_encoder(...);
        let mut render_pass = encoder.begin_render_pass(...);

        render_pass.set_bind_group(0, &self.shared_resources, &[]);

        for visual in &scene.visuals {
            visual.render(&mut render_pass);
        }
    }
}
```

### Pattern 2: Visual Trait

```rust
trait Visual {
    fn prepare(&mut self, device: &Device, queue: &Queue);
    fn render(&self, render_pass: &mut RenderPass);
}

// Each visual manages its own resources
struct PointsVisual {
    pipeline: RenderPipeline,
    bind_group: BindGroup,
    vertex_buffer: Buffer,
}
```

### Pattern 3: Bind Group Hierarchy

```
Group 0: Global (camera, lighting) ← Set once per frame
Group 1: Per-material (textures)   ← Set when material changes
Group 2: Per-object (transforms)   ← Set per draw call
```

### Pattern 4: Resource Builders

```rust
struct PipelineBuilder {
    layout: Option<PipelineLayout>,
    shader: Option<ShaderModule>,
    // ... other fields ...
}

impl PipelineBuilder {
    fn with_shader(mut self, shader: ShaderModule) -> Self {
        self.shader = Some(shader);
        self
    }

    fn build(self, device: &Device) -> RenderPipeline {
        device.create_render_pipeline(&RenderPipelineDescriptor {
            // ... use self fields ...
        })
    }
}
```

## Mental Model Summary

### OpenGL Mental Model

```
Global State
    ↓
Bind Things
    ↓
Issue Draw Commands
    ↓
GPU Executes (eventually)
```

### WGPU Mental Model

```
Create Immutable Resources (pipelines, bind groups)
    ↓
Build Command Buffer
    ├─ Set pipeline
    ├─ Set bind groups
    ├─ Set buffers
    └─ Draw commands
    ↓
Submit Command Buffer
    ↓
GPU Executes Asynchronously
```

## Performance Considerations

### WGPU is Faster When:

- ✅ Switching between pre-created pipelines
- ✅ Building commands on multiple threads
- ✅ Batching multiple draw calls
- ✅ Using bind groups effectively
- ✅ Minimizing resource recreation

### OpenGL is Faster When:

- ✅ Changing individual state (in simple scenes)
- ✅ Driver has perfect state tracking
- ✅ Single-threaded is sufficient

**Reality**: Modern games/apps use Vulkan/Metal/DX12 patterns (like WGPU) for performance.

## Debugging Tips

### WGPU Validation

Enable validation layers:
```rust
let instance = wgpu::Instance::new(wgpu::InstanceDescriptor {
    backends: wgpu::Backends::PRIMARY,
    flags: wgpu::InstanceFlags::VALIDATION,  // Enable validation!
    // ...
});
```

### Label Everything

```rust
let buffer = device.create_buffer(&BufferDescriptor {
    label: Some("Camera Uniform Buffer"),  // Shows up in debuggers!
    // ...
});
```

### Use GPU Debuggers

- **RenderDoc**: Capture frames, inspect resources
- **PIX** (Windows): Microsoft's GPU debugger
- **Xcode GPU Debugger** (macOS): Apple's tools

Labels help identify resources in these tools!

## Migration Strategy

If migrating from OpenGL to WGPU:

1. **Start with simple shapes** (triangle, cube)
2. **Port vertex/fragment shaders** to WGSL
3. **Create pipelines** for each OpenGL program
4. **Organize resources** into bind groups
5. **Batch draw calls** by pipeline/material
6. **Parallelize** command buffer building

## Final Thoughts

WGPU is **verbose by design**. The verbosity:
- ✅ Prevents hidden state bugs
- ✅ Enables compiler/runtime validation
- ✅ Matches modern GPU architecture
- ✅ Scales to complex applications

**The learning curve is worth it** for production code.

---

**Next**: [Chunked Rendering Architecture →](./09-chunked-rendering.md)
