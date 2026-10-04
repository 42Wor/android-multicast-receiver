use crate::gpu::GpuContext;
use omnicast_core::{FrameFormat, VideoFrame};
use std::sync::Arc;
use thiserror::Error;
use tracing::warn;
use winit::window::Window;

#[derive(Debug, Error)]
pub enum SurfaceError {
    #[error("create surface: {0}")]
    Create(#[from] wgpu::CreateSurfaceError),
    #[error("surface configure: {0}")]
    Configure(String),
    #[error("unsupported frame format {:?}", .0)]
    UnsupportedFormat(FrameFormat),
}

pub struct DeviceSurface {
    pub window: Arc<Window>,
    surface: wgpu::Surface<'static>,
    config: wgpu::SurfaceConfiguration,
    pipeline: wgpu::RenderPipeline,
    frame_texture: wgpu::Texture,
    frame_view: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    tex_width: u32,
    tex_height: u32,
}

impl DeviceSurface {
    pub fn new(gpu: &GpuContext, window: Arc<Window>) -> Result<Self, SurfaceError> {
        let size = window.inner_size();
        let width = size.width.max(1);
        let height = size.height.max(1);

        let surface = gpu.instance.create_surface(window.clone())?;
        let caps = surface.get_capabilities(&gpu.adapter);
        let format = caps
            .formats
            .iter()
            .copied()
            .find(|f| f.is_srgb())
            .unwrap_or(caps.formats[0]);

        let present_mode = if caps.present_modes.contains(&wgpu::PresentMode::Mailbox) {
            wgpu::PresentMode::Mailbox
        } else if caps.present_modes.contains(&wgpu::PresentMode::Immediate) {
            wgpu::PresentMode::Immediate
        } else {
            wgpu::PresentMode::Fifo
        };

        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width,
            height,
            present_mode,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            desired_maximum_frame_latency: 2,
        };
        surface.configure(&gpu.device, &config);

        let pipeline = gpu.create_blit_pipeline(format);
        let (frame_texture, frame_view, bind_group) = create_frame_resources(gpu, 64, 64);

        Ok(Self {
            window,
            surface,
            config,
            pipeline,
            frame_texture,
            frame_view,
            bind_group,
            tex_width: 64,
            tex_height: 64,
        })
    }

    pub fn resize(&mut self, gpu: &GpuContext, width: u32, height: u32) {
        if width == 0 || height == 0 {
            return;
        }
        self.config.width = width;
        self.config.height = height;
        self.surface.configure(&gpu.device, &self.config);
    }

    pub fn upload_frame(
        &mut self,
        gpu: &GpuContext,
        frame: &VideoFrame,
    ) -> Result<(), SurfaceError> {
        if frame.format != FrameFormat::Rgba8 {
            return Err(SurfaceError::UnsupportedFormat(frame.format));
        }
        if frame.width == 0 || frame.height == 0 {
            return Ok(());
        }
        if frame.width != self.tex_width || frame.height != self.tex_height {
            let (tex, view, bind_group) = create_frame_resources(gpu, frame.width, frame.height);
            self.frame_texture = tex;
            self.frame_view = view;
            self.bind_group = bind_group;
            self.tex_width = frame.width;
            self.tex_height = frame.height;
        }

        let bytes_per_row = frame.width * 4;
        gpu.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &self.frame_texture,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            &frame.data,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(bytes_per_row),
                rows_per_image: Some(frame.height),
            },
            wgpu::Extent3d {
                width: frame.width,
                height: frame.height,
                depth_or_array_layers: 1,
            },
        );
        Ok(())
    }

    pub fn render(&self, gpu: &GpuContext) {
        let frame = match self.surface.get_current_texture() {
            Ok(frame) => frame,
            Err(err) => {
                warn!(error = ?err, "get_current_texture failed; reconfiguring");
                self.surface.configure(&gpu.device, &self.config);
                match self.surface.get_current_texture() {
                    Ok(frame) => frame,
                    Err(err) => {
                        warn!(error = ?err, "surface acquire failed");
                        return;
                    }
                }
            }
        };

        let view = frame
            .texture
            .create_view(&wgpu::TextureViewDescriptor::default());
        let mut encoder = gpu
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("omnicast-encoder"),
            });

        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("blit-pass"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &view,
                    resolve_target: None,
                    ops: wgpu::Operations {
                        load: wgpu::LoadOp::Clear(wgpu::Color::BLACK),
                        store: wgpu::StoreOp::Store,
                    },
                })],
                depth_stencil_attachment: None,
                occlusion_query_set: None,
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.pipeline);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }

        gpu.queue.submit(Some(encoder.finish()));
        self.window.pre_present_notify();
        frame.present();
    }
}

fn create_frame_resources(
    gpu: &GpuContext,
    width: u32,
    height: u32,
) -> (wgpu::Texture, wgpu::TextureView, wgpu::BindGroup) {
    let texture = gpu.device.create_texture(&wgpu::TextureDescriptor {
        label: Some("frame-texture"),
        size: wgpu::Extent3d {
            width,
            height,
            depth_or_array_layers: 1,
        },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rgba8UnormSrgb,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    let bind_group = gpu.device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame-bg"),
        layout: &gpu.blit_bind_group_layout,
        entries: &[
            wgpu::BindGroupEntry {
                binding: 0,
                resource: wgpu::BindingResource::TextureView(&view),
            },
            wgpu::BindGroupEntry {
                binding: 1,
                resource: wgpu::BindingResource::Sampler(&gpu.sampler),
            },
        ],
    });
    (texture, view, bind_group)
}
