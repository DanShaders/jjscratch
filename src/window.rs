//! Windowed (on-screen) Vello-on-wgpu render path.
//!
//! This is the interactive counterpart to [`crate::render`] (the headless
//! Scene→PNG pipeline). Where the headless path renders to an offscreen texture
//! and reads it back, this path renders to a live wgpu **surface** attached to a
//! real [`winit`] window and presents each frame.
//!
//! It is intentionally a thin, reusable holder: a [`vello::util::RenderContext`]
//! (the wgpu instance + a pool of device handles), a [`Renderer`], and the
//! current [`vello::util::RenderSurface`]. The `ApplicationHandler` event loop
//! that drives it lives in `src/bin/jjscratch.rs`; this module owns only the GPU
//! plumbing so the bin stays focused on event translation + data wiring.
//!
//! ## Vello 0.9 surface gotcha
//!
//! Vello 0.9 has **no** `render_to_surface` method — only
//! [`Renderer::render_to_texture`]. Vello renders via a compute shader and so
//! cannot bind the surface texture directly; instead `RenderSurface` carries an
//! intermediate `target_texture` (which we render into) plus a
//! [`wgpu::util::TextureBlitter`] that copies that intermediate onto the surface
//! texture. So the per-frame sequence is:
//!   1. `surface.get_current_texture()` → the swapchain image,
//!   2. `render_to_texture(... &surface.target_view ...)`,
//!   3. `blitter.copy(target_view → surface_view)` in a command encoder + submit,
//!   4. `frame.present()`.
//! This mirrors vello's own `with_winit` example.

use std::sync::Arc;

use anyhow::{anyhow, Result};
use vello::util::{RenderContext, RenderSurface};
use vello::wgpu;
use vello::{AaConfig, AaSupport, RenderParams, Renderer, RendererOptions, Scene};
use winit::window::Window;

/// Holds everything needed to render Vello `Scene`s to a live window surface.
///
/// Construct once per window (in winit's `resumed`), then call [`Self::render`]
/// each `RedrawRequested` and [`Self::resize`] on size changes.
pub struct WindowRenderer {
    context: RenderContext,
    /// One renderer per device handle in `context.devices`; indexed by the
    /// surface's `dev_id`. A `Vec` keeps room for multi-window/multi-GPU later
    /// without reshaping callers.
    renderers: Vec<Option<Renderer>>,
    surface: RenderSurface<'static>,
    /// Kept alive so the surface's window handle stays valid for the surface's
    /// whole life. `Arc<Window>` yields a `'static` `SurfaceTarget`, which is
    /// why the surface can be `'static` (no self-referential borrow).
    window: Arc<Window>,
}

impl WindowRenderer {
    /// Create the wgpu surface + device + Vello renderer for `window`.
    ///
    /// `width`/`height` are the surface's *physical* pixel dimensions. Uses a
    /// real adapter (HighPerformance, default backends): we deliberately do NOT
    /// force a software backend here — see `docs/RUNNING.md` for the
    /// `VK_ICD_FILENAMES` caveat the repo's `.cargo/config.toml` introduces.
    pub fn new(window: Arc<Window>, width: u32, height: u32) -> Result<Self> {
        let mut context = RenderContext::new();
        // `create_surface` is async (it lazily creates/finds a compatible device
        // handle). Block on it; there is no display-blocking await inside.
        let surface = pollster::block_on(context.create_surface(
            window.clone(),
            width.max(1),
            height.max(1),
            wgpu::PresentMode::AutoVsync,
        ))
        .map_err(|e| anyhow!("creating wgpu surface: {e:?}"))?;

        let mut renderers: Vec<Option<Renderer>> = Vec::new();
        renderers.resize_with(context.devices.len(), || None);
        renderers[surface.dev_id] = Some(Self::make_renderer(&context, &surface)?);

        Ok(Self {
            context,
            renderers,
            surface,
            window,
        })
    }

    /// The wgpu adapter info for the device the surface picked (for logging).
    pub fn adapter_info(&self) -> wgpu::AdapterInfo {
        self.context.devices[self.surface.dev_id]
            .adapter()
            .get_info()
    }

    /// Build a Vello renderer matching the surface's format/device.
    fn make_renderer(context: &RenderContext, surface: &RenderSurface<'_>) -> Result<Renderer> {
        let device = &context.devices[surface.dev_id].device;
        Renderer::new(
            device,
            RendererOptions {
                use_cpu: false,
                antialiasing_support: AaSupport::area_only(),
                num_init_threads: None,
                pipeline_cache: None,
            },
        )
        .map_err(|e| anyhow!("creating vello renderer: {e:?}"))
    }

    /// Reconfigure the surface for a new physical size. No-op for a zero
    /// dimension (a minimised window reports 0×0 on some platforms, and
    /// `resize_surface` panics on zero).
    pub fn resize(&mut self, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.context
            .resize_surface(&mut self.surface, width, height);
    }

    /// Render `scene` to the window surface and present it.
    ///
    /// `base_color` is the clear color (the active theme's base). The scene must
    /// already be laid out at the surface's current physical size.
    pub fn render(&mut self, scene: &Scene, base_color: vello::peniko::Color) -> Result<()> {
        let dev_id = self.surface.dev_id;
        let width = self.surface.config.width;
        let height = self.surface.config.height;

        // Acquire the next swapchain image. wgpu 29 returns a status enum (not a
        // Result): `Success`/`Suboptimal` carry a usable texture; the rest mean
        // the surface went stale (resize/occlude/lost) — reconfigure and skip
        // this frame, the next RedrawRequested will succeed.
        use vello::wgpu::CurrentSurfaceTexture;
        let surface_texture = match self.surface.surface.get_current_texture() {
            CurrentSurfaceTexture::Success(t) | CurrentSurfaceTexture::Suboptimal(t) => t,
            CurrentSurfaceTexture::Outdated
            | CurrentSurfaceTexture::Lost
            | CurrentSurfaceTexture::Occluded
            | CurrentSurfaceTexture::Timeout
            | CurrentSurfaceTexture::Validation => {
                self.context.configure_surface(&self.surface);
                return Ok(());
            }
        };

        let device_handle = &self.context.devices[dev_id];
        let renderer = self.renderers[dev_id]
            .as_mut()
            .ok_or_else(|| anyhow!("no renderer for device {dev_id}"))?;

        // 1) Render into the intermediate target texture (Vello's compute path
        //    cannot bind the surface texture directly).
        renderer
            .render_to_texture(
                &device_handle.device,
                &device_handle.queue,
                scene,
                &self.surface.target_view,
                &RenderParams {
                    base_color,
                    width,
                    height,
                    antialiasing_method: AaConfig::Area,
                },
            )
            .map_err(|e| anyhow!("vello render failed: {e:?}"))?;

        // 2) Blit the intermediate onto the surface image, then present.
        let surface_view = surface_texture
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder =
            device_handle
                .device
                .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                    label: Some("jjscratch-surface-blit"),
                });
        self.surface.blitter.copy(
            &device_handle.device,
            &mut encoder,
            &self.surface.target_view,
            &surface_view,
        );
        device_handle.queue.submit([encoder.finish()]);

        self.window.pre_present_notify();
        surface_texture.present();
        Ok(())
    }
}
