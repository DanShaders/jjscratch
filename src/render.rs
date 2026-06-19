//! Headless GPU rendering pipeline: Vello Scene -> offscreen wgpu texture -> PNG.
//!
//! This is the foundation of the whole project's dev loop. There is no window in
//! this environment, so every "screenshot" is produced by rendering a `Scene` to
//! an offscreen `Rgba8Unorm` texture and copying it back to CPU memory.
//!
//! Prefers the Vulkan backend (lavapipe software rasterizer is present in this
//! environment); falls back to the GL backend (mesa llvmpipe) if Vulkan is
//! unavailable.

use anyhow::{anyhow, Context, Result};
use vello::peniko::Color;
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};

/// A reusable headless renderer. Construct once, render many scenes.
pub struct Headless {
    device: wgpu::Device,
    queue: wgpu::Queue,
    renderer: Renderer,
    pub adapter_info: wgpu::AdapterInfo,
}

/// A rendered RGBA8 image in CPU memory.
pub struct Image {
    pub width: u32,
    pub height: u32,
    /// Tightly packed RGBA8 (no row padding).
    pub rgba: Vec<u8>,
}

impl Headless {
    pub fn new() -> Result<Self> {
        pollster::block_on(Self::new_async())
    }

    async fn new_async() -> Result<Self> {
        // VULKAN only: in this headless environment the GL backend can't open
        // /dev/dri (no permission) and only spews warnings, while the Vulkan
        // loader reaches lavapipe (software) via VK_ICD_FILENAMES. Allow an env
        // override for environments with a real GPU.
        let mut desc = wgpu::InstanceDescriptor::new_without_display_handle();
        desc.backends = wgpu::Backends::from_env().unwrap_or(wgpu::Backends::VULKAN);
        let instance = wgpu::Instance::new(desc);

        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: None,
                force_fallback_adapter: false,
            })
            .await
            .map_err(|e| anyhow!("no suitable GPU adapter found: {e:?}"))?;

        let adapter_info = adapter.get_info();

        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("jjscratch-device"),
                required_features: wgpu::Features::empty(),
                required_limits: adapter.limits(),
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::disabled(),
                trace: wgpu::Trace::Off,
            })
            .await
            .context("failed to create wgpu device")?;

        let renderer = Renderer::new(
            &device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .map_err(|e| anyhow!("failed to create vello renderer: {e:?}"))?;

        Ok(Self {
            device,
            queue,
            renderer,
            adapter_info,
        })
    }

    /// Render a Vello scene to an RGBA8 image at the given size.
    pub fn render(
        &mut self,
        scene: &Scene,
        width: u32,
        height: u32,
        base_color: Color,
    ) -> Result<Image> {
        let size = wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        };

        let target = self.device.create_texture(&wgpu::TextureDescriptor {
            label: Some("render-target"),
            size,
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8Unorm,
            usage: wgpu::TextureUsages::STORAGE_BINDING | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        let view = target.create_view(&wgpu::TextureViewDescriptor::default());

        self.renderer
            .render_to_texture(
                &self.device,
                &self.queue,
                scene,
                &view,
                &RenderParams {
                    base_color,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(|e| anyhow!("vello render failed: {e:?}"))?;

        // Copy the texture into a buffer; bytes-per-row must be 256-aligned.
        let bytes_per_pixel = 4u32;
        let unpadded = width * bytes_per_pixel;
        let align = wgpu::COPY_BYTES_PER_ROW_ALIGNMENT;
        let padded = unpadded.div_ceil(align) * align;

        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("readback"),
            size: (padded * height) as u64,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("copy-out"),
            });
        encoder.copy_texture_to_buffer(
            wgpu::TexelCopyTextureInfo {
                texture: &target,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            size,
        );
        self.queue.submit([encoder.finish()]);

        // Map and read back.
        let slice = buffer.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        self.device.poll(wgpu::PollType::wait_indefinitely())?;
        rx.recv()
            .context("map_async channel closed")?
            .context("buffer map failed")?;

        let data = slice.get_mapped_range();
        let mut rgba = Vec::with_capacity((unpadded * height) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            rgba.extend_from_slice(&data[start..start + unpadded as usize]);
        }
        drop(data);
        buffer.unmap();

        Ok(Image {
            width,
            height,
            rgba,
        })
    }
}

impl Image {
    /// Write the image to a PNG file.
    pub fn save_png(&self, path: impl AsRef<std::path::Path>) -> Result<()> {
        image::save_buffer(
            path.as_ref(),
            &self.rgba,
            self.width,
            self.height,
            image::ColorType::Rgba8,
        )
        .context("failed to write PNG")?;
        Ok(())
    }
}
