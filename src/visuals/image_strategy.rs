use wgpu::{Device, Queue, Texture, TextureView};

/// Request for a chunk of image data
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ChunkRequest {
    pub x: u32,
    pub y: u32,
    pub z: u32,
    pub width: u32,
    pub height: u32,
    pub depth: u32,
}

/// Response containing chunk data
pub struct ChunkData {
    pub data: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// Trait for different image loading/rendering strategies
#[cfg(not(target_arch = "wasm32"))]
pub trait ImageStrategy: Send + Sync {
    /// Prepare for rendering (update textures, load chunks, etc.)
    fn prepare(&mut self, device: &Device, queue: &Queue, visible_region: Option<ChunkRequest>);

    /// Get the texture view for rendering
    fn texture_view(&self) -> &TextureView;

    /// Get the dimensions of the image
    fn dimensions(&self) -> (u32, u32, u32);

    /// Check if data is ready to render
    fn is_ready(&self) -> bool;
}

/// Trait for different image loading/rendering strategies (WASM version)
#[cfg(target_arch = "wasm32")]
pub trait ImageStrategy {
    /// Prepare for rendering (update textures, load chunks, etc.)
    fn prepare(&mut self, device: &Device, queue: &Queue, visible_region: Option<ChunkRequest>);

    /// Get the texture view for rendering
    fn texture_view(&self) -> &TextureView;

    /// Get the dimensions of the image
    fn dimensions(&self) -> (u32, u32, u32);

    /// Check if data is ready to render
    fn is_ready(&self) -> bool;
}

/// Simple strategy for small in-memory images
pub struct SimpleImageStrategy {
    _texture: Texture,
    texture_view: TextureView,
    width: u32,
    height: u32,
    depth: u32,
}

impl SimpleImageStrategy {
    pub fn new(
        device: &Device,
        queue: &Queue,
        data: &[u8],
        width: u32,
        height: u32,
        depth: u32,
        format: wgpu::TextureFormat,
    ) -> Self {
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Image Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: if depth > 1 {
                wgpu::TextureDimension::D3
            } else {
                wgpu::TextureDimension::D2
            },
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        // Calculate bytes per pixel based on format
        let bytes_per_pixel = match format {
            wgpu::TextureFormat::R8Unorm => 1,
            wgpu::TextureFormat::Rg8Unorm => 2,
            wgpu::TextureFormat::Rgba8Unorm | wgpu::TextureFormat::Rgba8UnormSrgb => 4,
            wgpu::TextureFormat::R16Float => 2,
            wgpu::TextureFormat::R32Float => 4,
            _ => 4, // Default fallback
        };

        queue.write_texture(
            wgpu::ImageCopyTexture {
                texture: &texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            data,
            wgpu::ImageDataLayout {
                offset: 0,
                bytes_per_row: Some(width * bytes_per_pixel),
                rows_per_image: Some(height),
            },
            texture_size,
        );

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            _texture: texture,
            texture_view,
            width,
            height,
            depth,
        }
    }
}

impl ImageStrategy for SimpleImageStrategy {
    fn prepare(&mut self, _device: &Device, _queue: &Queue, _visible_region: Option<ChunkRequest>) {
        // Simple strategy has all data pre-loaded, nothing to do
    }

    fn texture_view(&self) -> &TextureView {
        &self.texture_view
    }

    fn dimensions(&self) -> (u32, u32, u32) {
        (self.width, self.height, self.depth)
    }

    fn is_ready(&self) -> bool {
        true
    }
}

/// Type alias for chunk loader callback
pub type ChunkLoaderFn = Box<dyn Fn(ChunkRequest) -> Option<ChunkData> + Send + Sync>;

/// Chunked strategy for large/remote images
pub struct ChunkedImageStrategy {
    texture: Texture,
    texture_view: TextureView,
    width: u32,
    height: u32,
    depth: u32,
    chunk_size: (u32, u32, u32),
    loader: ChunkLoaderFn,
    loaded_chunks: std::collections::HashSet<(u32, u32, u32)>,
}

impl ChunkedImageStrategy {
    pub fn new(
        device: &Device,
        width: u32,
        height: u32,
        depth: u32,
        chunk_size: (u32, u32, u32),
        format: wgpu::TextureFormat,
        loader: ChunkLoaderFn,
    ) -> Self {
        let texture_size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: depth,
        };

        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("Chunked Image Texture"),
            size: texture_size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: if depth > 1 {
                wgpu::TextureDimension::D3
            } else {
                wgpu::TextureDimension::D2
            },
            format,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });

        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());

        Self {
            texture,
            texture_view,
            width,
            height,
            depth,
            chunk_size,
            loader,
            loaded_chunks: std::collections::HashSet::new(),
        }
    }
}

impl ImageStrategy for ChunkedImageStrategy {
    fn prepare(&mut self, _device: &Device, queue: &Queue, visible_region: Option<ChunkRequest>) {
        // Determine which chunks to load based on visible region
        let region = visible_region.unwrap_or(ChunkRequest {
            x: 0,
            y: 0,
            z: 0,
            width: self.width,
            height: self.height,
            depth: self.depth,
        });

        // Calculate chunk indices
        let start_chunk_x = region.x / self.chunk_size.0;
        let start_chunk_y = region.y / self.chunk_size.1;
        let start_chunk_z = region.z / self.chunk_size.2;

        let end_chunk_x = (region.x + region.width).div_ceil(self.chunk_size.0);
        let end_chunk_y = (region.y + region.height).div_ceil(self.chunk_size.1);
        let end_chunk_z = (region.z + region.depth).div_ceil(self.chunk_size.2);

        // Load missing chunks
        for cz in start_chunk_z..end_chunk_z {
            for cy in start_chunk_y..end_chunk_y {
                for cx in start_chunk_x..end_chunk_x {
                    if self.loaded_chunks.contains(&(cx, cy, cz)) {
                        continue;
                    }

                    let chunk_req = ChunkRequest {
                        x: cx * self.chunk_size.0,
                        y: cy * self.chunk_size.1,
                        z: cz * self.chunk_size.2,
                        width: self.chunk_size.0.min(self.width - cx * self.chunk_size.0),
                        height: self.chunk_size.1.min(self.height - cy * self.chunk_size.1),
                        depth: self.chunk_size.2.min(self.depth - cz * self.chunk_size.2),
                    };

                    if let Some(chunk_data) = (self.loader)(chunk_req) {
                        // Calculate bytes per pixel based on format (assuming R8Unorm for now)
                        // TODO: Store format in ChunkedImageStrategy for dynamic calculation
                        let bytes_per_pixel = 1u32; // R8Unorm

                        // Upload chunk to GPU
                        queue.write_texture(
                            wgpu::ImageCopyTexture {
                                texture: &self.texture,
                                mip_level: 0,
                                origin: wgpu::Origin3d {
                                    x: chunk_req.x,
                                    y: chunk_req.y,
                                    z: chunk_req.z,
                                },
                                aspect: wgpu::TextureAspect::All,
                            },
                            &chunk_data.data,
                            wgpu::ImageDataLayout {
                                offset: 0,
                                bytes_per_row: Some(chunk_data.width * bytes_per_pixel),
                                rows_per_image: Some(chunk_data.height),
                            },
                            wgpu::Extent3d {
                                width: chunk_data.width,
                                height: chunk_data.height,
                                depth_or_array_layers: chunk_req.depth,
                            },
                        );

                        self.loaded_chunks.insert((cx, cy, cz));
                    }
                }
            }
        }
    }

    fn texture_view(&self) -> &TextureView {
        &self.texture_view
    }

    fn dimensions(&self) -> (u32, u32, u32) {
        (self.width, self.height, self.depth)
    }

    fn is_ready(&self) -> bool {
        !self.loaded_chunks.is_empty()
    }
}
