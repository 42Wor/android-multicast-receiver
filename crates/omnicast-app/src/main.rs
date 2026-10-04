//! OmniCast desktop receiver — winit multi-window ApplicationHandler.

use anyhow::{Context, Result};
use clap::Parser;
use omnicast_core::{AppEvent, DeviceId, SessionInfo, SessionState};
use omnicast_media::MockFrameGenerator;
use omnicast_protocol::{RtspServer, RtspServerConfig};
use omnicast_render::{DeviceSurface, GpuContext};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::WindowEvent;
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::window::{Window, WindowId};

#[derive(Debug, Parser)]
#[command(name = "omnicast", about = "OmniCast Android casting desktop receiver")]
struct Cli {
    /// Spawn mock device session(s) rendering synthetic frames at ~60 FPS
    #[arg(long)]
    demo: bool,

    /// Number of mock devices when --demo is set
    #[arg(long, default_value_t = 1)]
    devices: usize,

    /// RTSP listen address
    #[arg(long, default_value = "0.0.0.0:8554")]
    rtsp_addr: SocketAddr,

    /// RTP UDP listen port
    #[arg(long, default_value_t = 5004)]
    rtp_port: u16,
}

struct DeviceContext {
    session: SessionInfo,
    surface: DeviceSurface,
    mock: Option<MockFrameGenerator>,
}

struct App {
    gpu: Option<GpuContext>,
    devices: HashMap<WindowId, DeviceContext>,
    device_windows: HashMap<DeviceId, WindowId>,
    pending_sessions: Vec<SessionInfo>,
    event_rx: mpsc::Receiver<AppEvent>,
    last_fps_log: Instant,
    last_frame_tick: Instant,
    frames_drawn: u64,
    demo: bool,
    demo_devices: usize,
    _runtime: Runtime,
}

impl App {
    fn bootstrap(cli: Cli) -> Result<Self> {
        let runtime = Runtime::new().context("tokio runtime")?;
        let (tx, rx) = mpsc::channel::<AppEvent>(256);

        let rtsp = RtspServer::new(RtspServerConfig {
            bind_addr: cli.rtsp_addr,
            rtp_port: cli.rtp_port,
        });
        let rtsp_tx = tx.clone();
        runtime.spawn(async move {
            if let Err(err) = rtsp.run(rtsp_tx).await {
                error!(error = %err, "RTSP server terminated");
            }
        });

        Ok(Self {
            gpu: None,
            devices: HashMap::new(),
            device_windows: HashMap::new(),
            pending_sessions: Vec::new(),
            event_rx: rx,
            last_fps_log: Instant::now(),
            last_frame_tick: Instant::now(),
            frames_drawn: 0,
            demo: cli.demo,
            demo_devices: cli.devices.max(1),
            _runtime: runtime,
        })
    }

    fn drain_events(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::DeviceConnected(session) => {
                    info!(device = %session.device_name, id = %session.id, "device connected");
                    if self.gpu.is_some() {
                        if let Err(err) = self.spawn_window(event_loop, session) {
                            error!(error = %err, "failed to spawn device window");
                        }
                    } else {
                        self.pending_sessions.push(session);
                    }
                }
                AppEvent::DeviceDisconnected { id, reason } => {
                    info!(%id, %reason, "device disconnected");
                    if let Some(window_id) = self.device_windows.remove(&id) {
                        self.devices.remove(&window_id);
                    }
                }
                AppEvent::FrameReady(frame) => {
                    if let Some(window_id) = self.device_windows.get(&frame.device_id).copied() {
                        if let (Some(gpu), Some(ctx)) =
                            (self.gpu.as_ref(), self.devices.get_mut(&window_id))
                        {
                            if let Err(err) = ctx.surface.upload_frame(gpu, &frame) {
                                warn!(error = %err, "frame upload failed");
                            } else {
                                ctx.surface.window.request_redraw();
                            }
                        }
                    }
                }
                AppEvent::Status(msg) => info!(%msg, "status"),
            }
        }
    }

    fn spawn_window(&mut self, event_loop: &ActiveEventLoop, session: SessionInfo) -> Result<()> {
        let gpu = self.gpu.as_ref().context("gpu not initialized")?;
        let index = self.devices.len() as i32;
        let attrs = Window::default_attributes()
            .with_title(format!("{} — OmniCast", session.device_name))
            .with_inner_size(LogicalSize::new(420.0, 780.0))
            .with_position(LogicalPosition::new(
                80.0 + f64::from(index) * 40.0,
                60.0 + f64::from(index) * 40.0,
            ));

        let window = Arc::new(event_loop.create_window(attrs).context("create_window")?);
        let window_id = window.id();
        let surface = DeviceSurface::new(gpu, window).context("DeviceSurface::new")?;

        let mock = if self.demo || session.protocol == "simulate" {
            Some(MockFrameGenerator::new(
                session.id,
                session.width.max(320),
                session.height.max(568),
            ))
        } else {
            None
        };

        self.device_windows.insert(session.id, window_id);
        self.devices.insert(
            window_id,
            DeviceContext {
                session,
                surface,
                mock,
            },
        );
        Ok(())
    }

    fn ensure_gpu(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.gpu.is_some() {
            return Ok(());
        }

        let bootstrap = Arc::new(
            event_loop
                .create_window(
                    Window::default_attributes()
                        .with_title("OmniCast — starting…")
                        .with_visible(false)
                        .with_inner_size(LogicalSize::new(64.0, 64.0)),
                )
                .context("bootstrap window")?,
        );

        let gpu =
            pollster::block_on(GpuContext::new(Some(&bootstrap))).context("GpuContext::new")?;
        self.gpu = Some(gpu);
        drop(bootstrap);

        let pending: Vec<_> = self.pending_sessions.drain(..).collect();
        for session in pending {
            self.spawn_window(event_loop, session)?;
        }

        if self.demo {
            for i in 0..self.demo_devices {
                let mut session = SessionInfo::new(format!("Demo Phone {}", i + 1), "simulate");
                session.state = SessionState::Active;
                // Modest mock resolution keeps CPU fill + upload on a 60 FPS budget.
                session.width = 480;
                session.height = 854;
                session.fps = 60.0;
                self.spawn_window(event_loop, session)?;
            }
            info!(count = self.demo_devices, "demo sessions spawned");
        }

        info!("GPU initialized; receiver ready");
        Ok(())
    }

    fn tick_demo_frames(&mut self) {
        // Pace the CPU mock generator to ~60 Hz.
        if self.last_frame_tick.elapsed() < Duration::from_micros(16_666) {
            return;
        }
        self.last_frame_tick = Instant::now();

        let Some(gpu) = self.gpu.as_ref() else {
            return;
        };
        for ctx in self.devices.values_mut() {
            if let Some(mock) = ctx.mock.as_mut() {
                let frame = mock.next_frame();
                if let Err(err) = ctx.surface.upload_frame(gpu, &frame) {
                    warn!(error = %err, "mock upload failed");
                    continue;
                }
                ctx.surface.window.request_redraw();
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(err) = self.ensure_gpu(event_loop) {
            error!(error = %err, "failed to initialize GPU");
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        // Allow EventLoopProxy injection later; reuse channel handling shape.
        match event {
            AppEvent::DeviceConnected(session) => {
                if self.gpu.is_some() {
                    if let Err(err) = self.spawn_window(event_loop, session) {
                        error!(error = %err, "spawn from user_event failed");
                    }
                } else {
                    self.pending_sessions.push(session);
                }
            }
            AppEvent::DeviceDisconnected { id, reason } => {
                info!(%id, %reason, "device disconnected");
                if let Some(window_id) = self.device_windows.remove(&id) {
                    self.devices.remove(&window_id);
                }
            }
            AppEvent::FrameReady(frame) => {
                if let Some(window_id) = self.device_windows.get(&frame.device_id).copied() {
                    if let (Some(gpu), Some(ctx)) =
                        (self.gpu.as_ref(), self.devices.get_mut(&window_id))
                    {
                        if let Err(err) = ctx.surface.upload_frame(gpu, &frame) {
                            warn!(error = %err, "frame upload failed");
                        } else {
                            ctx.surface.window.request_redraw();
                        }
                    }
                }
            }
            AppEvent::Status(msg) => info!(%msg, "status"),
        }
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.drain_events(event_loop);

        match event {
            WindowEvent::CloseRequested => {
                if let Some(ctx) = self.devices.remove(&window_id) {
                    self.device_windows.remove(&ctx.session.id);
                    info!(device = %ctx.session.device_name, "window closed");
                }
                if self.devices.is_empty() {
                    info!("no device windows remain; exiting");
                    event_loop.exit();
                }
            }
            WindowEvent::Resized(size) => {
                if let (Some(gpu), Some(ctx)) =
                    (self.gpu.as_ref(), self.devices.get_mut(&window_id))
                {
                    ctx.surface.resize(gpu, size.width, size.height);
                    ctx.surface.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if let (Some(gpu), Some(ctx)) = (self.gpu.as_ref(), self.devices.get(&window_id)) {
                    ctx.surface.render(gpu);
                    self.frames_drawn += 1;
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.drain_events(event_loop);
        self.tick_demo_frames();

        if self.last_fps_log.elapsed() >= Duration::from_secs(1) {
            let fps = self.frames_drawn;
            self.frames_drawn = 0;
            self.last_fps_log = Instant::now();
            if fps > 0 {
                info!(fps, windows = self.devices.len(), "present stats");
            }
        }

        if !self.devices.is_empty() {
            event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
                Instant::now() + Duration::from_micros(16_666),
            ));
        }
    }
}

fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| {
                "omnicast=info,omnicast_app=info,omnicast_discovery=info,omnicast_protocol=info,wgpu_core=warn,wgpu_hal=warn".into()
            }),
        )
        .init();

    let cli = Cli::parse();
    info!(demo = cli.demo, devices = cli.devices, "starting OmniCast");

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let mut app = App::bootstrap(cli)?;
    event_loop.run_app(&mut app)?;
    Ok(())
}
