//! egui-wgpu overlay host for telemetry HUD, dashboard, and settings.

use egui_wgpu::{Renderer as EguiRenderer, ScreenDescriptor};
use omnicast_core::MetricsSnapshot;
use omnicast_ui::{HudAction, HudOverlay, ViewerSettings};
use std::sync::Arc;
use winit::event::WindowEvent;
use winit::window::Window;

pub struct OverlayHost {
    pub egui_ctx: egui::Context,
    state: egui_winit::State,
    renderer: EguiRenderer,
    pub hud: HudOverlay,
    pub settings: ViewerSettings,
}

impl OverlayHost {
    pub fn new(
        window: &Arc<Window>,
        device: &wgpu::Device,
        surface_format: wgpu::TextureFormat,
        settings: ViewerSettings,
    ) -> Self {
        let egui_ctx = egui::Context::default();
        egui_ctx.set_visuals(egui::Visuals::dark());

        let state = egui_winit::State::new(
            egui_ctx.clone(),
            egui::ViewportId::ROOT,
            window.as_ref(),
            Some(window.scale_factor() as f32),
            None,
            None,
        );

        let renderer = EguiRenderer::new(device, surface_format, None, 1, false);

        Self {
            egui_ctx,
            state,
            renderer,
            hud: HudOverlay::default(),
            settings,
        }
    }

    pub fn on_window_event(&mut self, window: &Window, event: &WindowEvent) -> bool {
        self.state.on_window_event(window, event).consumed
    }

    /// Run egui HUD/settings and encode draw calls into `encoder` (after video blit).
    pub fn paint(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        metrics: &MetricsSnapshot,
        device_name: &str,
    ) -> (HudAction, Vec<wgpu::CommandBuffer>) {
        let raw_input = self.state.take_egui_input(window);
        let mut action = HudAction::None;
        let full_output = {
            let hud = &mut self.hud;
            let settings = &mut self.settings;
            self.egui_ctx.run(raw_input, |ctx| {
                action = hud.ui(ctx, settings, metrics, device_name);
            })
        };
        let cmd = self.encode_egui(
            window,
            device,
            queue,
            encoder,
            view,
            width,
            height,
            full_output,
        );
        (action, cmd)
    }

    /// Generic egui frame for non-HUD windows (e.g. dashboard).
    pub fn paint_with<F>(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        mut ui: F,
    ) -> Vec<wgpu::CommandBuffer>
    where
        F: FnMut(&egui::Context),
    {
        let raw_input = self.state.take_egui_input(window);
        let full_output = self.egui_ctx.run(raw_input, ui);
        self.encode_egui(
            window,
            device,
            queue,
            encoder,
            view,
            width,
            height,
            full_output,
        )
    }

    fn encode_egui(
        &mut self,
        window: &Window,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        encoder: &mut wgpu::CommandEncoder,
        view: &wgpu::TextureView,
        width: u32,
        height: u32,
        full_output: egui::FullOutput,
    ) -> Vec<wgpu::CommandBuffer> {
        self.state
            .handle_platform_output(window, full_output.platform_output);

        let tris = self
            .egui_ctx
            .tessellate(full_output.shapes, full_output.pixels_per_point);
        let screen = ScreenDescriptor {
            size_in_pixels: [width.max(1), height.max(1)],
            pixels_per_point: full_output.pixels_per_point,
        };

        for (id, image_delta) in &full_output.textures_delta.set {
            self.renderer
                .update_texture(device, queue, *id, image_delta);
        }

        let user_cmd_bufs = self
            .renderer
            .update_buffers(device, queue, encoder, &tris, &screen);

        {
            let pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("egui-overlay"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Load,
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            let mut static_pass = pass.forget_lifetime();
            self.renderer.render(&mut static_pass, &tris, &screen);
        }

        for id in &full_output.textures_delta.free {
            self.renderer.free_texture(id);
        }

        user_cmd_bufs
    }
}
