use omnicast_core::DeviceId;

#[derive(Clone, Debug)]
pub struct DashboardDevice {
    pub id: DeviceId,
    pub peer_ip: String,
    pub codec: String,
    pub protocol: String,
    pub uptime_label: String,
    pub feed_open: bool,
}

#[derive(Clone, Debug)]
pub struct DashboardState {
    pub listening: bool,
    pub receiver_name: String,
    pub cast_port: u16,
    pub rtsp_port: u16,
    pub status_line: String,
    pub devices: Vec<DashboardDevice>,
}

impl Default for DashboardState {
    fn default() -> Self {
        Self {
            listening: false,
            receiver_name: "OmniCast (Laptop)".into(),
            cast_port: 8009,
            rtsp_port: 8554,
            status_line: "Stopped — press Start to advertise on the LAN".into(),
            devices: Vec::new(),
        }
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
}

/// Main OmniCast control dashboard (immediate-mode egui).
pub fn draw_dashboard(ctx: &egui::Context, state: &mut DashboardState) -> DashboardAction {
    let mut action = DashboardAction::None;
    let mut rename_requested = None;

    egui::CentralPanel::default()
        .frame(
            egui::Frame::NONE
                .fill(egui::Color32::from_rgb(18, 22, 28))
                .inner_margin(egui::Margin::symmetric(18, 16)),
        )
        .show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.heading(
                    egui::RichText::new("OmniCast")
                        .color(egui::Color32::from_rgb(230, 236, 245))
                        .size(26.0),
                );
                ui.label(
                    egui::RichText::new("Desktop receiver")
                        .color(egui::Color32::from_rgb(140, 150, 165))
                        .italics(),
                );
            });
            ui.add_space(10.0);

            // Status
            ui.horizontal(|ui| {
                if state.listening {
                    ui.colored_label(
                        egui::Color32::from_rgb(80, 220, 140),
                        "● Active / Listening",
                    );
                } else {
                    ui.colored_label(egui::Color32::from_rgb(160, 160, 170), "○ Stopped");
                }
                ui.label(
                    egui::RichText::new(&state.status_line)
                        .color(egui::Color32::from_rgb(150, 160, 175))
                        .small(),
                );
            });
            ui.add_space(8.0);

            // Receiver name
            ui.group(|ui| {
                ui.label("Receiver name (shown on Android Cast list)");
                let response = ui.add(
                    egui::TextEdit::singleline(&mut state.receiver_name)
                        .desired_width(360.0)
                        .hint_text("OmniCast (Laptop)"),
                );
                if response.lost_focus() && response.changed() {
                    rename_requested = Some(state.receiver_name.clone());
                }
                if ui
                    .button("Apply name")
                    .on_hover_text("Update mDNS friendly name (fn TXT)")
                    .clicked()
                {
                    rename_requested = Some(state.receiver_name.clone());
                }
            });
            ui.add_space(8.0);

            // Start / Stop
            ui.horizontal(|ui| {
                if state.listening {
                    if ui
                        .add(
                            egui::Button::new("Stop listeners")
                                .fill(egui::Color32::from_rgb(140, 60, 60)),
                        )
                        .clicked()
                    {
                        action = DashboardAction::Stop;
                    }
                } else if ui
                    .add(
                        egui::Button::new("Start listeners")
                            .fill(egui::Color32::from_rgb(40, 120, 80)),
                    )
                    .clicked()
                {
                    action = DashboardAction::Start;
                }
                ui.label(format!(
                    "Cast TLS :{}  ·  RTSP :{}",
                    state.cast_port, state.rtsp_port
                ));
            });
            ui.add_space(14.0);

            ui.separator();
            ui.heading("Connected devices");
            ui.add_space(6.0);

            if state.devices.is_empty() {
                ui.label(
                    egui::RichText::new("No clients yet. Start listeners, then Cast to this receiver from your phone.")
                        .color(egui::Color32::from_rgb(130, 140, 155)),
                );
            } else {
                egui::ScrollArea::vertical().show(ui, |ui| {
                    for device in &state.devices {
                        egui::Frame::group(ui.style())
                            .fill(egui::Color32::from_rgb(28, 34, 44))
                            .inner_margin(10.0)
                            .show(ui, |ui| {
                                ui.horizontal(|ui| {
                                    ui.vertical(|ui| {
                                        ui.label(
                                            egui::RichText::new(&device.peer_ip)
                                                .strong()
                                                .color(egui::Color32::WHITE),
                                        );
                                        ui.label(format!(
                                            "{} · {} · {}",
                                            device.protocol, device.codec, device.uptime_label
                                        ));
                                    });
                                    ui.with_layout(
                                        egui::Layout::right_to_left(egui::Align::Center),
                                        |ui| {
                                            let label = if device.feed_open {
                                                "Focus feed"
                                            } else {
                                                "Open feed"
                                            };
                                            if ui.button(label).clicked() {
                                                action = DashboardAction::OpenDevice(device.id);
                                            }
                                        },
                                    );
                                });
                            });
                        ui.add_space(6.0);
                    }
                });
            }
        });

    if let Some(name) = rename_requested {
        if action == DashboardAction::None {
            action = DashboardAction::Rename(name);
        }
    }
    action
}
