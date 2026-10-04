//! OmniCast desktop receiver — dashboard-first ApplicationHandler.

use anyhow::{Context, Result};
use clap::Parser;
use omnicast_core::{
    AppEvent, DeviceId, MetricsRegistry, SessionInfo, SessionState, StreamMetrics,
};
use omnicast_discovery::{DiscoveryConfig, DiscoveryService};
use omnicast_media::MockFrameGenerator;
use omnicast_protocol::{CastServerConfig, CastTlsServer, RtspServer, RtspServerConfig};
use omnicast_render::{DeviceSurface, GpuContext, UiOnlyWindow};
use omnicast_ui::{DashboardAction, DashboardDevice, DashboardState, HudAction, ViewerSettings};
use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::runtime::Runtime;
use tokio::sync::{mpsc, watch};
use tracing::{error, info, warn};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{ElementState, KeyEvent, WindowEvent};
use winit::event_loop::{ActiveEventLoop, EventLoop};
use winit::keyboard::{KeyCode, PhysicalKey};
use winit::window::{Window, WindowId};

#[derive(Debug, Parser)]
#[command(
    name = "omnicast-app",
    about = "OmniCast Android casting desktop receiver"
)]
struct Cli {
    #[arg(long)]
    demo: bool,
    #[arg(long, default_value_t = 1)]
    devices: usize,
    #[arg(long)]
    no_discovery: bool,
    #[arg(long, default_value = "OmniCast (Laptop)")]
    receiver_name: String,
    #[arg(long, default_value = "0.0.0.0:8554")]
    rtsp_addr: SocketAddr,
    #[arg(long, default_value = "0.0.0.0:8009")]
    cast_addr: SocketAddr,
    #[arg(long, default_value_t = 5004)]
    rtp_port: u16,
    /// Start listeners immediately (default: wait for dashboard Start)
    #[arg(long)]
    auto_start: bool,
}

struct DeviceContext {
    session: SessionInfo,
    surface: DeviceSurface,
    mock: Option<MockFrameGenerator>,
    metrics: StreamMetrics,
}

struct ConnectedClient {
    session: SessionInfo,
    metrics: StreamMetrics,
    connected_at: Instant,
}

struct App {
    gpu: Option<GpuContext>,
    dashboard: Option<UiOnlyWindow>,
    dashboard_id: Option<WindowId>,
    dashboard_state: DashboardState,
    devices: HashMap<WindowId, DeviceContext>,
    device_windows: HashMap<DeviceId, WindowId>,
    connected: HashMap<DeviceId, ConnectedClient>,
    pending_open: Vec<DeviceId>,
    event_tx: mpsc::Sender<AppEvent>,
    event_rx: mpsc::Receiver<AppEvent>,
    metrics_registry: MetricsRegistry,
    settings: ViewerSettings,
    runtime: Runtime,
    discovery: Option<DiscoveryService>,
    running_tx: watch::Sender<bool>,
    running_rx: watch::Receiver<bool>,
    listeners_started: bool,
    cli: Cli,
    last_frame_tick: Instant,
    last_fps_log: Instant,
    frames_drawn: u64,
}

impl App {
    fn bootstrap(cli: Cli) -> Result<Self> {
        let runtime = Runtime::new().context("tokio runtime")?;
        let (tx, rx) = mpsc::channel::<AppEvent>(256);
        let metrics_registry = MetricsRegistry::new();
        let (running_tx, running_rx) = watch::channel(false);

        let mut settings = ViewerSettings::default();
        settings.listen_port = cli.rtsp_addr.port();

        let mut dashboard_state = DashboardState {
            receiver_name: cli.receiver_name.clone(),
            settings_name: cli.receiver_name.clone(),
            cast_port: cli.cast_addr.port(),
            rtsp_port: cli.rtsp_addr.port(),
            viewer: settings.clone(),
            ..DashboardState::default()
        };
        dashboard_state.push_log("Dashboard ready — press Start Listening to advertise");

        let mut app = Self {
            gpu: None,
            dashboard: None,
            dashboard_id: None,
            dashboard_state,
            devices: HashMap::new(),
            device_windows: HashMap::new(),
            connected: HashMap::new(),
            pending_open: Vec::new(),
            event_tx: tx,
            event_rx: rx,
            metrics_registry,
            settings,
            runtime,
            discovery: None,
            running_tx,
            running_rx,
            listeners_started: false,
            cli,
            last_frame_tick: Instant::now(),
            last_fps_log: Instant::now(),
            frames_drawn: 0,
        };

        if app.cli.auto_start || app.cli.demo {
            app.start_listeners();
        }

        Ok(app)
    }

    fn start_listeners(&mut self) {
        if self.listeners_started {
            return;
        }
        let name = self.dashboard_state.receiver_name.clone();
        let mut lan_ip = None;
        if !self.cli.no_discovery {
            match DiscoveryService::start(DiscoveryConfig {
                instance_name: name.clone(),
                host_name: "omnicast".into(),
                rtsp_port: self.cli.rtsp_addr.port(),
                cast_port: self.cli.cast_addr.port(),
                advertise_display: true,
                advertise_googlecast: true,
                advertise_rtsp: true,
            }) {
                Ok(svc) => {
                    lan_ip = svc.local_ipv4;
                    info!(%name, "mDNS advertising started");
                    self.dashboard_state
                        .push_log(format!("mDNS broadcaster active as \"{name}\""));
                    self.discovery = Some(svc);
                }
                Err(err) => {
                    warn!(error = %err, "mDNS failed to start");
                    self.dashboard_state
                        .push_log(format!("mDNS failed to start: {err}"));
                }
            }
        } else {
            self.dashboard_state
                .push_log("mDNS disabled (--no-discovery)");
        }

        let _ = self.running_tx.send(true);

        let rtsp = RtspServer::new(RtspServerConfig {
            bind_addr: self.cli.rtsp_addr,
            rtp_port: self.cli.rtp_port,
        })
        .with_metrics(self.metrics_registry.clone());
        let rtsp_tx = self.event_tx.clone();
        let rtsp_rx = self.running_rx.clone();
        self.runtime.spawn(async move {
            if let Err(err) = rtsp.run(rtsp_tx, rtsp_rx).await {
                error!(error = %err, "RTSP server terminated");
            }
        });

        let cast = CastTlsServer::new(CastServerConfig {
            bind_addr: self.cli.cast_addr,
            lan_ip,
            mdns_hostname: "omnicast.local".into(),
            receiver_name: name.clone(),
        });
        let cast_tx = self.event_tx.clone();
        let cast_rx = self.running_rx.clone();
        self.runtime.spawn(async move {
            if let Err(err) = cast.run(cast_tx, cast_rx).await {
                error!(error = %err, "Cast TLS server terminated");
            }
        });

        self.listeners_started = true;
        self.dashboard_state.listening = true;
        self.dashboard_state.status_line = format!(
            "Listening — Cast TLS :{} · RTSP :{}",
            self.cli.cast_addr.port(),
            self.cli.rtsp_addr.port()
        );
        self.dashboard_state.push_log(format!(
            "Listeners active — Cast TLS :{} · RTSP :{}",
            self.cli.cast_addr.port(),
            self.cli.rtsp_addr.port()
        ));
        info!("listeners started");
    }

    fn stop_listeners(&mut self) {
        let _ = self.running_tx.send(false);
        if let Some(disco) = self.discovery.take() {
            disco.shutdown();
        }
        self.listeners_started = false;
        self.dashboard_state.listening = false;
        self.dashboard_state.status_line =
            "Stopped — press Start Listening to advertise on the LAN".into();
        self.dashboard_state
            .push_log("Listeners stopped — mDNS and sockets closed");
        info!("listeners stopped");
    }

    fn apply_rename(&mut self, name: String) {
        self.dashboard_state.receiver_name = name.clone();
        self.dashboard_state.settings_name = name.clone();
        if let Some(disco) = self.discovery.as_mut() {
            if let Err(err) = disco.set_instance_name(&name) {
                warn!(error = %err, "failed to update mDNS name");
                self.dashboard_state
                    .push_log(format!("Failed to re-advertise as \"{name}\": {err}"));
            } else {
                self.dashboard_state.status_line = format!("mDNS name updated to '{name}'");
                self.dashboard_state
                    .push_log(format!("mDNS re-advertised as \"{name}\""));
            }
        } else {
            self.dashboard_state
                .push_log(format!("Receiver name set to \"{name}\" (start listening to advertise)"));
        }
    }

    fn apply_viewer_settings(&mut self) {
        self.settings = self.dashboard_state.viewer.clone();
        let on_top = self.settings.always_on_top;
        for ctx in self.devices.values() {
            ctx.surface.window.set_window_level(if on_top {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        }
        self.dashboard_state
            .push_log("Viewer HUD preferences updated");
    }

    fn refresh_dashboard_devices(&mut self) {
        let open = &self.device_windows;
        self.dashboard_state.devices = self
            .connected
            .values()
            .map(|c| {
                let snap = c.metrics.snapshot();
                let feed_open = open.contains_key(&c.session.id);
                let video_status = if feed_open {
                    if snap.fps > 0.5 {
                        format!("Video live ({:.0} FPS)", snap.fps)
                    } else {
                        "Feed open".into()
                    }
                } else {
                    "Idle".into()
                };
                DashboardDevice {
                    id: c.session.id,
                    peer_ip: if c.session.peer_ip.is_empty() {
                        c.session.device_name.clone()
                    } else {
                        c.session.peer_ip.clone()
                    },
                    device_name: c.session.device_name.clone(),
                    codec: c.session.codec.clone(),
                    protocol: c.session.protocol.clone(),
                    uptime_label: snap.uptime_label(),
                    video_status,
                    feed_open,
                }
            })
            .collect();
    }

    fn drain_events(&mut self, event_loop: &ActiveEventLoop) {
        while let Ok(event) = self.event_rx.try_recv() {
            match event {
                AppEvent::DeviceConnected(session) => {
                    info!(device = %session.device_name, id = %session.id, ip = %session.peer_ip, "device connected");
                    let peer = if session.peer_ip.is_empty() {
                        session.device_name.clone()
                    } else {
                        session.peer_ip.clone()
                    };
                    self.dashboard_state.push_log(format!(
                        "Device connected: {peer} ({})",
                        session.protocol
                    ));
                    let metrics = StreamMetrics::new();
                    self.metrics_registry.register(session.id, metrics.clone());
                    self.connected.insert(
                        session.id,
                        ConnectedClient {
                            session,
                            metrics,
                            connected_at: Instant::now(),
                        },
                    );
                    self.refresh_dashboard_devices();
                    if let Some(dash) = &self.dashboard {
                        dash.window.request_redraw();
                    }
                }
                AppEvent::DeviceDisconnected { id, reason } => {
                    info!(%id, %reason, "device disconnected");
                    self.dashboard_state
                        .push_log(format!("Device disconnected: {reason}"));
                    self.metrics_registry.unregister(id);
                    self.connected.remove(&id);
                    if let Some(window_id) = self.device_windows.remove(&id) {
                        self.devices.remove(&window_id);
                    }
                    self.refresh_dashboard_devices();
                }
                AppEvent::FrameReady(frame) => {
                    if let Some(window_id) = self.device_windows.get(&frame.device_id).copied() {
                        if let (Some(gpu), Some(ctx)) =
                            (self.gpu.as_ref(), self.devices.get_mut(&window_id))
                        {
                            ctx.metrics.record_network(frame.data.len() as u64, 1);
                            if let Err(err) = ctx.surface.upload_frame(gpu, &frame) {
                                warn!(error = %err, "frame upload failed");
                            } else {
                                ctx.surface.window.request_redraw();
                            }
                        }
                    }
                }
                AppEvent::Status(msg) => {
                    info!(%msg, "status");
                    self.dashboard_state.status_line = msg.clone();
                    self.dashboard_state.push_log(msg);
                }
            }
        }

        let pending: Vec<_> = self.pending_open.drain(..).collect();
        for id in pending {
            if let Err(err) = self.open_feed(event_loop, id) {
                error!(error = %err, "failed to open feed");
            }
        }
    }

    fn open_feed(&mut self, event_loop: &ActiveEventLoop, id: DeviceId) -> Result<()> {
        if let Some(window_id) = self.device_windows.get(&id).copied() {
            if let Some(ctx) = self.devices.get(&window_id) {
                ctx.surface.window.focus_window();
            }
            return Ok(());
        }
        let client = self.connected.get(&id).context("device not connected")?;
        let session = client.session.clone();
        let metrics = client.metrics.clone();
        self.spawn_feed_window(event_loop, session, metrics)?;
        self.refresh_dashboard_devices();
        Ok(())
    }

    fn spawn_feed_window(
        &mut self,
        event_loop: &ActiveEventLoop,
        session: SessionInfo,
        metrics: StreamMetrics,
    ) -> Result<()> {
        let gpu = self.gpu.as_ref().context("gpu not initialized")?;
        let index = self.devices.len() as i32;
        let attrs = Window::default_attributes()
            .with_title(format!("{} — OmniCast", session.device_name))
            .with_inner_size(LogicalSize::new(420.0, 780.0))
            .with_position(LogicalPosition::new(
                120.0 + f64::from(index) * 40.0,
                80.0 + f64::from(index) * 40.0,
            ))
            .with_window_level(if self.settings.always_on_top {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        let window = Arc::new(event_loop.create_window(attrs).context("create_window")?);
        let window_id = window.id();
        let surface =
            DeviceSurface::new(gpu, window, self.settings.clone()).context("DeviceSurface")?;

        let mock = if self.cli.demo || session.protocol == "simulate" {
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
                metrics,
            },
        );
        Ok(())
    }

    fn ensure_dashboard(&mut self, event_loop: &ActiveEventLoop) -> Result<()> {
        if self.dashboard.is_some() {
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

        let attrs = Window::default_attributes()
            .with_title("OmniCast")
            .with_inner_size(LogicalSize::new(720.0, 820.0))
            .with_min_inner_size(LogicalSize::new(560.0, 640.0))
            .with_position(LogicalPosition::new(64.0, 48.0));
        let window = Arc::new(
            event_loop
                .create_window(attrs)
                .context("dashboard window")?,
        );
        let window_id = window.id();
        let gpu = self.gpu.as_ref().unwrap();
        let dash = UiOnlyWindow::new(gpu, window).context("UiOnlyWindow")?;
        self.dashboard_id = Some(window_id);
        self.dashboard = Some(dash);

        if self.cli.demo {
            for i in 0..self.cli.devices.max(1) {
                let mut session = SessionInfo::new(format!("Demo Phone {}", i + 1), "simulate");
                session.state = SessionState::Active;
                session.peer_ip = format!("192.168.100.{}", 10 + i);
                session.codec = "H264".into();
                session.width = 480;
                session.height = 854;
                let metrics = StreamMetrics::new();
                self.metrics_registry.register(session.id, metrics.clone());
                let id = session.id;
                self.connected.insert(
                    id,
                    ConnectedClient {
                        session,
                        metrics,
                        connected_at: Instant::now(),
                    },
                );
            }
            self.refresh_dashboard_devices();
            info!("demo clients listed on dashboard (open feed from UI)");
        }

        info!("dashboard window ready");
        Ok(())
    }

    fn handle_dashboard_action(&mut self, event_loop: &ActiveEventLoop, action: DashboardAction) {
        match action {
            DashboardAction::None => {}
            DashboardAction::Start => self.start_listeners(),
            DashboardAction::Stop => self.stop_listeners(),
            DashboardAction::Rename(name) => self.apply_rename(name),
            DashboardAction::OpenDevice(id) => {
                if let Err(err) = self.open_feed(event_loop, id) {
                    error!(error = %err, "open feed failed");
                }
            }
            DashboardAction::ViewerSettingsChanged => self.apply_viewer_settings(),
        }
    }

    fn tick_demo_frames(&mut self) {
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
                ctx.metrics
                    .record_network((frame.width * frame.height / 8) as u64, 1);
                if ctx.surface.upload_frame(gpu, &frame).is_ok() {
                    ctx.surface.window.request_redraw();
                }
            }
        }
    }
}

impl ApplicationHandler<AppEvent> for App {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        if let Err(err) = self.ensure_dashboard(event_loop) {
            error!(error = %err, "failed to open dashboard");
            event_loop.exit();
        }
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: AppEvent) {
        let _ = self.event_tx.try_send(event);
        self.drain_events(event_loop);
    }

    fn window_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        window_id: WindowId,
        event: WindowEvent,
    ) {
        self.drain_events(event_loop);

        let is_dashboard = self.dashboard_id == Some(window_id);

        if is_dashboard {
            if let Some(dash) = self.dashboard.as_mut() {
                if dash.on_window_event(&event) {
                    dash.window.request_redraw();
                }
            }
        } else if let Some(ctx) = self.devices.get_mut(&window_id) {
            if ctx.surface.on_window_event(&event) {
                ctx.surface.window.request_redraw();
            }
        }

        match event {
            WindowEvent::CloseRequested => {
                if is_dashboard {
                    info!("dashboard closed — shutting down");
                    self.stop_listeners();
                    event_loop.exit();
                    return;
                }
                if let Some(ctx) = self.devices.remove(&window_id) {
                    self.device_windows.remove(&ctx.session.id);
                    info!(device = %ctx.session.device_name, "feed window closed");
                    self.refresh_dashboard_devices();
                }
            }
            WindowEvent::Resized(size) => {
                if is_dashboard {
                    if let (Some(gpu), Some(dash)) = (self.gpu.as_ref(), self.dashboard.as_mut()) {
                        dash.resize(gpu, size.width, size.height);
                        dash.window.request_redraw();
                    }
                } else if let (Some(gpu), Some(ctx)) =
                    (self.gpu.as_ref(), self.devices.get_mut(&window_id))
                {
                    ctx.surface.resize(gpu, size.width, size.height);
                    ctx.surface.window.request_redraw();
                }
            }
            WindowEvent::KeyboardInput {
                event:
                    KeyEvent {
                        physical_key: PhysicalKey::Code(KeyCode::KeyS),
                        state: ElementState::Pressed,
                        ..
                    },
                ..
            } if !is_dashboard => {
                if let Some(ctx) = self.devices.get_mut(&window_id) {
                    ctx.surface.overlay.hud.settings_open = true;
                    ctx.surface.window.request_redraw();
                }
            }
            WindowEvent::RedrawRequested => {
                if is_dashboard {
                    self.refresh_dashboard_devices();
                    let action = {
                        let gpu = self.gpu.as_ref();
                        let dash = self.dashboard.as_mut();
                        match (gpu, dash) {
                            (Some(gpu), Some(dash)) => {
                                dash.render_dashboard(gpu, &mut self.dashboard_state)
                            }
                            _ => DashboardAction::None,
                        }
                    };
                    self.handle_dashboard_action(event_loop, action);
                    return;
                }

                let Some(gpu) = self.gpu.as_ref() else {
                    return;
                };
                let Some(ctx) = self.devices.get_mut(&window_id) else {
                    return;
                };
                ctx.metrics.record_frame();
                let snap = ctx.metrics.snapshot();
                let name = ctx.session.device_name.clone();
                // Keep feed HUD prefs in sync with dashboard settings.
                ctx.surface.overlay.settings = self.settings.clone();
                let action = ctx.surface.render(gpu, &snap, &name);
                self.frames_drawn += 1;
                // Allow in-feed settings edits to flow back.
                self.settings = ctx.surface.overlay.settings.clone();
                self.dashboard_state.viewer = self.settings.clone();
                if self.settings.show_fps {
                    ctx.surface.window.set_title(&format!(
                        "{} — {:.0} FPS — OmniCast",
                        name, snap.fps
                    ));
                } else {
                    ctx.surface
                        .window
                        .set_title(&format!("{name} — OmniCast"));
                }
                if action == HudAction::OpenSettings {
                    info!("settings panel opened");
                }
            }
            _ => {}
        }
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.drain_events(event_loop);
        self.tick_demo_frames();
        if let Some(dash) = &self.dashboard {
            dash.window.request_redraw();
        }
        if self.last_fps_log.elapsed() >= Duration::from_secs(1) {
            let fps = self.frames_drawn;
            self.frames_drawn = 0;
            self.last_fps_log = Instant::now();
            if fps > 0 {
                info!(fps, windows = self.devices.len(), "present stats");
            }
        }
        event_loop.set_control_flow(winit::event_loop::ControlFlow::WaitUntil(
            Instant::now() + Duration::from_millis(33),
        ));
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
    info!(demo = cli.demo, "starting OmniCast dashboard");

    let event_loop = EventLoop::<AppEvent>::with_user_event().build()?;
    let mut app = App::bootstrap(cli)?;
    event_loop.run_app(&mut app)?;
    Ok(())
}
