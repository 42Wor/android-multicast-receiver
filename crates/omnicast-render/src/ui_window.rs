//! egui-only window surface (main dashboard).

use crate::gpu::GpuContext;
use crate::overlay::OverlayHost;
use omnicast_ui::{draw_dashboard, DashboardAction, DashboardState, ViewerSettings};
use std::sync::Arc;
use thiserror::Error;
use tracing::warn;
use winit::event::WindowEvent;
use winit::window::Window;

#[derive(Debug, Error)]
pub enum UiWindowError {
    #[error("create surface: {0}")]
    Create(#[from] wgpu::CreateSurfaceError),
}

pub struct UiOnlyWindow {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pub overlay: OverlayHost,
}

impl UiOnlyWindow {
    pub fn new(gpu: &GpuContext, window: Arc<Window>) -> Result<Self, UiWindowError> {
        let size = window.inner_size();
        let surface = gpu.instance.create_surface(window.clone())?;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &config);
        let overlay = OverlayHost::new(&window, &gpu.device, format, ViewerSettings::default());
        Ok(Self {
            window,
            surface,
            config,
            overlay,
        })
    }

    pub fn on_window_event(&mut self, event: &WindowEvent) -> bool {
        self.overlay.on_window_event(&self.window, event)
    }

    pub fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn render_dashboard(
        &mut self,
        gpu: &GpuContext,
        state: &mut DashboardState,
    ) -> DashboardAction {
        let frame = match self.surface.get_current_texture() {
            Ok(f) => f,
            Err(err) => {
                warn!(error = ?err, "dashboard surface acquire failed");
                self.surface.configure(&gpu.device, &self.config);
                match self.surface.get_current_texture() {
                    Ok(f) => f,
                    Err(_) => return DashboardAction::None,
                }
            }
        };
        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("dashboard-encoder"),
            });

        {
            let _pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("dashboard-clear"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color {
                            r: 0.07,
                            g: 0.09,
                            b: 0.11,
                            a: 1.0,
                        }),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
        }

        let mut action = DashboardAction::None;
        let cmd_bufs = self.overlay.paint_with(
            &self.window,
            &gpu.device,
            &gpu.queue,
            &mut encoder,
            &view,
            self.config.width,
            self.config.height,
            |ctx| {
                action = draw_dashboard(ctx, state);
            },
        );

        gpu.queue.submit(
            cmd_bufs
                .into_iter()
                .chain(std::iter::once(encoder.finish())),
        );
        self.window.pre_present_notify();
        frame.present();
        action
    }
}
