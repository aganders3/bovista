# Visual System

Bovista renders all 3D content through one interface: the `Visual` trait, which defines the rendering lifecycle plus common handling of transforms, visibility, and naming.

## The Visual Trait

```rust
pub trait Visual: Send {
    fn prepare(&mut self, device: &wgpu::Device, queue: &wgpu::Queue);
    fn render(&self, render_pass: &mut RenderPass);

    fn set_transform(&mut self, transform: Transform);
    fn transform(&self) -> &Transform;

    fn is_visible(&self) -> bool;
    fn set_visible(&mut self, visible: bool);

    fn name(&self) -> &str;
}
```

The **prepare** phase updates GPU resources (buffers, uniforms) before the frame. The **render** phase emits draw commands into the active render pass.

## Built-in Visual Types

| Visual | Description |
|--------|-------------|
| `Points` | Colored point cloud; vertices uploaded once at construction |
| `Lines` | Line segments and wireframes; same pattern as points |
| `Image` | Arbitrary-orientation slice plane through a 3D volume; virtual texture streaming |
| `DirectVolume` | Direct volume rendering via GPU ray marching (front-to-back alpha compositing); same streaming back-end as `Image` |
| `MipVolume` | Maximum intensity projection (attenuated MIP when `set_attenuation` > 0) |
| `MinipVolume` | Minimum intensity projection |
| `AverageVolume` | Mean projection |
| `IsosurfaceVolume` | Iso-surface rendering (`set_iso_threshold`) |
| `Custom` | User-supplied WGSL shader; Bovista provides the camera bind group and depth buffer |

Volume rendering is split across five classes — one visual per mode, not a single mode-flag class — so each exposes only the parameters that apply to it. `Image` and the volume visuals share the virtual texture system; see [Virtual Textures](./01-philosophy.md).

## Adding a New Visual Type

Implement the `Visual` trait for your struct, constructing it with a `wgpu::Device` reference from `viewer.renderer()`. `src/visuals/points.rs` and `src/visuals/lines.rs` are the simplest examples; `src/visuals/image.rs` shows per-frame GPU updates. All visuals share the camera bind group (`@group(0)`), which `Renderer` sets once per frame before iterating the scene.
