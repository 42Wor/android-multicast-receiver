use crate::settings::ViewerSettings;
use omnicast_core::DeviceId;
use std::collections::VecDeque;
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_LOG_LINES: usize = 500;

#[derive(Clone, Debug)]
pub struct DashboardDevice {
    pub id: DeviceId,
    pub peer_ip: String,
    pub device_name: String,
    pub codec: String,
    pub protocol: String,
    pub uptime_label: String,
    pub video_status: String,
    pub feed_open: bool,
}

#[derive(Clone, Debug)]
pub struct ActivityLine {
    pub stamp: String,
    pub message: String,
}

#[derive(Clone, Debug)]
pub struct DashboardState {
    pub listening: bool,
    pub receiver_name: String,
    pub cast_port: u16,
    pub rtsp_port: u16,
    pub status_line: String,
    pub devices: Vec<DashboardDevice>,
    pub activity_log: VecDeque<ActivityLine>,
    pub auto_scroll: bool,
    pub settings_open: bool,
    pub viewer: ViewerSettings,
    /// Draft name edited inside the settings panel.
    pub settings_name: String,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            listening: false,
            receiver_name: "OmniCast (Laptop)".into(),
            cast_port: 8009,
            rtsp_port: 8554,
            status_line: "Stopped — press Start Listening to advertise on the LAN".into(),
            devices: Vec::new(),
            activity_log: VecDeque::new(),
            auto_scroll: true,
            settings_open: false,
            viewer: ViewerSettings::default(),
            settings_name: "OmniCast (Laptop)".into(),
        }
    }
}

impl DashboardState {
    pub fn push_log(&mut self, message: impl Into<String>) {
        self.activity_log.push_back(ActivityLine {
            stamp: timestamp_hhmmss(),
            message: message.into(),
        });
        while self.activity_log.len() > MAX_LOG_LINES {
            self.activity_log.pop_front();
        }
    }

    pub fn clear_logs(&mut self) {
        self.activity_log.clear();
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum DashboardAction {
    #[default]
    None,
    Start,
    Stop,
    Rename(String),
    OpenDevice(DeviceId),
    ViewerSettingsChanged,
}

/// Polished OmniCast control dashboard (immediate-mode egui).
pub fn draw_dashboard(ctx: &egui::Context, state: &mut DashboardState) -> DashboardAction {
    let mut action = DashboardAction::None;

    // Smooth pulse while listening.
    if state.listening {
        ctx.request_repaint_after(std::time::Duration::from_millis(33));
    }

    let bg = egui::Color32::from_rgb(14, 17, 23);
    let panel = egui::Color32::from_rgb(22, 27, 36);
    let border = egui::Color32::from_rgb(40, 48, 62);
    let text = egui::Color32::from_rgb(230, 236, 245);
    let muted = egui::Color32::from_rgb(130, 142, 160);
    let emerald = egui::Color32::from_rgb(52, 211, 153);
    let emerald_dim = egui::Color32::from_rgb(16, 94, 68);
    let danger = egui::Color32::from_rgb(239, 98, 98);
    let danger_dim = egui::Color32::from_rgb(120, 40, 48);
    let accent = egui::Color32::from_rgb(96, 165, 250);

    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(bg)
                .inner_margin(egui::Margin::symmetric(22, 18)),
        )
        .show(ctx, |ui| {
            // ── Header ──────────────────────────────────────────────
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 10.0;

                // Logo mark
                let (logo_rect, _) =
                    ui.allocate_exact_size(egui::vec2(28.0, 28.0), egui::Sense::hover());
                ui.painter().circle_filled(
                    logo_rect.center(),
                    13.0,
                    egui::Color32::from_rgb(37, 99, 235),
                );
                ui.painter().text(
                    logo_rect.center(),
                    egui::Align2::CENTER_CENTER,
                    "O",
                    egui::FontId::proportional(14.0),
                    egui::Color32::WHITE,
                );

                ui.label(
                    egui::RichText::new("OmniCast")
                        .color(text)
                        .size(24.0)
                        .strong(),
                );

                // Status pill
                let (pill_label, pill_fg, pill_bg) = if state.listening {
                    ("\u{25CF} Listening", emerald, emerald_dim)
                } else {
                    ("\u{25CB} Stopped", muted, egui::Color32::from_rgb(36, 40, 50))
                };
                let pill_stroke = if state.listening {
                    emerald.gamma_multiply(0.55)
                } else {
                    border
                };
                let pill = egui::Frame::NONE
                    .fill(pill_bg)
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::symmetric(12, 5))
                    .stroke(egui::Stroke::new(1.0_f32, pill_stroke));
                pill.show(ui, |ui| {
                    ui.label(egui::RichText::new(pill_label).color(pill_fg).size(13.0));
                });

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let gear = ui
                        .add_sized(
                            [40.0, 32.0],
                            egui::Button::new(
                                egui::RichText::new("\u{2699}").size(18.0).color(text),
                            )
                            .fill(panel)
                            .stroke(egui::Stroke::new(1.0_f32, border))
                            .corner_radius(egui::CornerRadius::same(8)),
                        )
                        .on_hover_text("Settings");
                    if gear.clicked() {
                        state.settings_open = !state.settings_open;
                        if state.settings_open {
                            state.settings_name = state.receiver_name.clone();
                        }
                    }
                });
            });

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(&state.status_line)
                    .color(muted)
                    .size(12.5),
            );
            ui.add_space(18.0);

            // ── Center action ───────────────────────────────────────
            ui.vertical_centered(|ui| {
                ui.add_space(12.0);
                let t = ui.input(|i| i.time);
                let pulse = ((t * 2.8).sin() * 0.5 + 0.5) as f32;

                if state.listening {
                    let fill = egui::Color32::from_rgb(
                        (danger_dim.r() as f32
                            + (danger.r() as f32 - danger_dim.r() as f32) * pulse)
                            as u8,
                        (danger_dim.g() as f32
                            + (danger.g() as f32 - danger_dim.g() as f32) * pulse * 0.35)
                            as u8,
                        (danger_dim.b() as f32
                            + (danger.b() as f32 - danger_dim.b() as f32) * pulse * 0.35)
                            as u8,
                    );
                    let btn = ui.add_sized(
                        [280.0, 52.0],
                        egui::Button::new(
                            egui::RichText::new("\u{23F9}  Stop Listening")
                                .size(18.0)
                                .color(egui::Color32::WHITE)
                                .strong(),
                        )
                        .fill(fill)
                        .corner_radius(egui::CornerRadius::same(12))
                        .stroke(egui::Stroke::new(1.0_f32, danger.gamma_multiply(0.7))),
                    );
                    if btn.clicked() {
                        action = DashboardAction::Stop;
                    }
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new(format!(
                            "Cast TLS :{}  ·  RTSP :{}",
                            state.cast_port, state.rtsp_port
                        ))
                        .color(muted)
                        .size(13.0),
                    );
                } else {
                    let btn = ui.add_sized(
                        [280.0, 52.0],
                        egui::Button::new(
                            egui::RichText::new("\u{25B6}  Start Listening")
                                .size(18.0)
                                .color(egui::Color32::WHITE)
                                .strong(),
                        )
                        .fill(egui::Color32::from_rgb(22, 140, 90))
                        .corner_radius(egui::CornerRadius::same(12))
                        .stroke(egui::Stroke::new(1.0_f32, emerald.gamma_multiply(0.65))),
                    );
                    if btn.clicked() {
                        action = DashboardAction::Start;
                    }
                    ui.add_space(8.0);
                    ui.label(
                        egui::RichText::new("Advertise on the LAN and accept Cast / RTSP probes")
                            .color(muted)
                            .size(13.0),
                    );
                }
            });

            ui.add_space(20.0);

            // ── Connected devices card ──────────────────────────────
            let device_count = state.devices.len();
            egui::Frame::NONE
                .fill(panel)
                .corner_radius(egui::CornerRadius::same(12))
                .stroke(egui::Stroke::new(1.0_f32, border))
                .inner_margin(egui::Margin::same(14))
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(
                            egui::RichText::new("Connected Devices")
                                .color(text)
                                .size(15.0)
                                .strong(),
                        );
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            let badge_bg = if device_count > 0 {
                                emerald_dim
                            } else {
                                egui::Color32::from_rgb(36, 40, 50)
                            };
                            egui::Frame::NONE
                                .fill(badge_bg)
                                .corner_radius(egui::CornerRadius::same(10))
                                .inner_margin(egui::Margin::symmetric(10, 3))
                                .show(ui, |ui| {
                                    ui.label(
                                        egui::RichText::new(format!("{device_count}"))
                                            .color(if device_count > 0 { emerald } else { muted })
                                            .strong(),
                                    );
                                });
                        });
                    });
                    ui.add_space(8.0);

                    if state.devices.is_empty() {
                        ui.label(
                            egui::RichText::new(
                                "No phones connected yet. Start listening, then Cast from Quick Settings.",
                            )
                            .color(muted)
                            .size(13.0),
                        );
                    } else {
                        for device in &state.devices {
                            egui::Frame::NONE
                                .fill(egui::Color32::from_rgb(28, 34, 46))
                                .corner_radius(egui::CornerRadius::same(8))
                                .inner_margin(egui::Margin::same(10))
                                .show(ui, |ui| {
                                    ui.horizontal(|ui| {
                                        ui.vertical(|ui| {
                                            ui.label(
                                                egui::RichText::new(&device.peer_ip)
                                                    .color(egui::Color32::WHITE)
                                                    .size(14.0)
                                                    .strong(),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{} · {} · {} · {}",
                                                    device.device_name,
                                                    device.protocol,
                                                    device.codec,
                                                    device.video_status
                                                ))
                                                .color(muted)
                                                .size(12.0),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "Session {}",
                                                    device.uptime_label
                                                ))
                                                .color(accent)
                                                .size(11.5),
                                            );
                                        });
                                        ui.with_layout(
                                            egui::Layout::right_to_left(egui::Align::Center),
                                            |ui| {
                                                let label = if device.feed_open {
                                                    "Focus feed"
                                                } else {
                                                    "Open feed"
                                                };
                                                if ui
                                                    .add(
                                                        egui::Button::new(label)
                                                            .fill(egui::Color32::from_rgb(
                                                                37, 99, 235,
                                                            ))
                                                            .corner_radius(
                                                                egui::CornerRadius::same(6),
                                                            ),
                                                    )
                                                    .clicked()
                                                {
                                                    action =
                                                        DashboardAction::OpenDevice(device.id);
                                                }
                                            },
                                        );
                                    });
                                });
                            ui.add_space(6.0);
                        }
                    }
                });

            ui.add_space(14.0);

            // ── Live activity console ───────────────────────────────
            ui.horizontal(|ui| {
                ui.label(
                    egui::RichText::new("Live Activity")
                        .color(text)
                        .size(15.0)
                        .strong(),
                );
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui
                        .add(
                            egui::Button::new("Clear Logs")
                                .fill(egui::Color32::from_rgb(40, 44, 56))
                                .stroke(egui::Stroke::new(1.0_f32, border)),
                        )
                        .clicked()
                    {
                        state.clear_logs();
                    }
                    ui.checkbox(
                        &mut state.auto_scroll,
                        egui::RichText::new("Auto-scroll").color(muted).size(12.5),
                    );
                });
            });
            ui.add_space(6.0);

            let console_h = ui.available_height().max(140.0);
            egui::Frame::NONE
                .fill(egui::Color32::from_rgb(8, 10, 14))
                .corner_radius(egui::CornerRadius::same(10))
                .stroke(egui::Stroke::new(
                    1.0_f32,
                    egui::Color32::from_rgb(30, 36, 48),
                ))
                .inner_margin(egui::Margin::same(10))
                .show(ui, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(console_h)
                        .stick_to_bottom(state.auto_scroll)
                        .show(ui, |ui| {
                            ui.style_mut().override_font_id =
                                Some(egui::FontId::monospace(12.5));
                            if state.activity_log.is_empty() {
                                ui.label(
                                    egui::RichText::new(
                                        "Waiting for activity… Start listening to begin.",
                                    )
                                    .color(egui::Color32::from_rgb(90, 100, 115)),
                                );
                            } else {
                                for line in &state.activity_log {
                                    ui.horizontal_wrapped(|ui| {
                                        ui.label(
                                            egui::RichText::new(format!("[{}]", line.stamp))
                                                .color(egui::Color32::from_rgb(90, 200, 140))
                                                .monospace(),
                                        );
                                        ui.label(
                                            egui::RichText::new(&line.message)
                                                .color(egui::Color32::from_rgb(200, 210, 220))
                                                .monospace(),
                                        );
                                    });
                                }
                            }
                        });
                });
        });

    // ── Settings modal ──────────────────────────────────────────────
    if state.settings_open {
        let mut open = true;
        egui::Window::new("Settings")
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .default_width(420.0)
            .frame(
                egui::Frame::window(&ctx.style())
                    .fill(panel)
                    .stroke(egui::Stroke::new(1.0_f32, border))
                    .corner_radius(egui::CornerRadius::same(12))
                    .inner_margin(egui::Margin::same(16)),
            )
            .show(ctx, |ui| {
                ui.label(
                    egui::RichText::new("Device Identity")
                        .color(text)
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(6.0);
                ui.label(egui::RichText::new("Receiver Name").color(muted).size(12.5));
                ui.add(
                    egui::TextEdit::singleline(&mut state.settings_name)
                        .desired_width(360.0)
                        .hint_text("OmniCast (Laptop)"),
                );
                ui.add_space(8.0);
                if ui
                    .add(
                        egui::Button::new("Save & Re-advertise")
                            .fill(egui::Color32::from_rgb(37, 99, 235))
                            .min_size(egui::vec2(160.0, 30.0)),
                    )
                    .clicked()
                {
                    let name = state.settings_name.trim().to_string();
                    if !name.is_empty() {
                        state.receiver_name = name.clone();
                        action = DashboardAction::Rename(name);
                    }
                }

                ui.add_space(16.0);
                ui.separator();
                ui.add_space(10.0);

                ui.label(
                    egui::RichText::new("Viewer HUD Preferences")
                        .color(text)
                        .size(15.0)
                        .strong(),
                );
                ui.add_space(6.0);

                let mut changed = false;
                changed |= ui
                    .checkbox(
                        &mut state.viewer.show_fps,
                        "Show Live FPS counter in window title bar",
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut state.viewer.show_uptime,
                        "Show Session Duration / Uptime (HH:MM:SS)",
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut state.viewer.show_bitrate,
                        "Show Network Throughput (Mbps)",
                    )
                    .changed();
                changed |= ui
                    .checkbox(
                        &mut state.viewer.always_on_top,
                        "Always on Top (pin cast windows above other apps)",
                    )
                    .changed();

                if changed && action == DashboardAction::None {
                    action = DashboardAction::ViewerSettingsChanged;
                }

                ui.add_space(12.0);
                if ui.button("Close").clicked() {
                    state.settings_open = false;
                }
            });
        if !open {
            state.settings_open = false;
        }
    }

    action
}

fn timestamp_hhmmss() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let s = secs % 86_400;
    format!("{:02}:{:02}:{:02}", s / 3600, (s % 3600) / 60, s % 60)
}
