use crate::settings::ViewerSettings;
use omnicast_core::MetricsSnapshot;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum HudAction {
    #[default]
    None,
    OpenSettings,
    TogglePin,
}

/// Translucent top-bar HUD drawn with egui over the video surface.
pub struct HudOverlay {
    pub settings_open: bool,
    last_cursor_in_hud: bool,
    hover_until: f64,
}

impl Default for HudOverlay {
    fn default() -> Self {
        Self {
            settings_open: false,
            last_cursor_in_hud: false,
            hover_until: 0.0,
        }
    }
}

impl HudOverlay {
    pub fn ui(
        &mut self,
        ctx: &egui::Context,
        settings: &mut ViewerSettings,
        metrics: &MetricsSnapshot,
        device_name: &str,
    ) -> HudAction {
        let mut action = HudAction::None;
        let now = ctx.input(|i| i.time);

        if ctx.input(|i| i.key_pressed(egui::Key::H)) {
            settings.hud_pinned = !settings.hud_pinned;
            action = HudAction::TogglePin;
        }

        let pointer_pos = ctx.input(|i| i.pointer.hover_pos());
        let top_band = pointer_pos.is_some_and(|p| p.y < 56.0);
        if top_band {
            self.hover_until = now + 2.0;
            self.last_cursor_in_hud = true;
        } else if self.last_cursor_in_hud {
            self.last_cursor_in_hud = false;
        }

        let show_bar = settings.hud_pinned
            || !settings.hud_autohide
            || now < self.hover_until
            || self.settings_open
            || top_band;

        if show_bar {
            egui::TopBottomPanel::top("omnicast_hud")
                .frame(
                    egui::Frame::NONE
                        .fill(egui::Color32::from_rgba_unmultiplied(12, 16, 22, 180))
                        .inner_margin(egui::Margin::symmetric(10, 6)),
                )
                .show(ctx, |ui| {
                    ui.horizontal(|ui| {
                        let settings_clicked = ui
                            .add(
                                egui::Button::new(egui::RichText::new("⚙ Settings").strong())
                                    .fill(egui::Color32::from_rgb(40, 48, 62)),
                            )
                            .clicked();
                        if settings_clicked {
                            self.settings_open = true;
                            action = HudAction::OpenSettings;
                        }

                        ui.separator();
                        ui.label(
                            egui::RichText::new(device_name)
                                .color(egui::Color32::from_rgb(220, 228, 240))
                                .strong(),
                        );

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if settings.show_uptime {
                                ui.label(
                                    egui::RichText::new(metrics.uptime_label())
                                        .monospace()
                                        .color(egui::Color32::from_rgb(180, 200, 220)),
                                );
                            }
                            if settings.show_bitrate {
                                ui.label(
                                    egui::RichText::new(metrics.bitrate_label())
                                        .monospace()
                                        .color(egui::Color32::from_rgb(120, 220, 170)),
                                );
                            }
                            if settings.show_fps {
                                ui.label(
                                    egui::RichText::new(metrics.fps_label())
                                        .monospace()
                                        .color(egui::Color32::from_rgb(255, 200, 120)),
                                );
                            }
                            if settings.show_packet_stats {
                                ui.label(
                                    egui::RichText::new(format!(
                                        "frames {} · drops {}",
                                        metrics.frames, metrics.drops
                                    ))
                                    .small()
                                    .color(egui::Color32::from_rgb(150, 160, 175)),
                                );
                            }
                            if settings.hud_pinned {
                                ui.weak("PINNED");
                            }
                        });
                    });
                });
        }

        if self.settings_open {
            let mut close_clicked = false;
            egui::Window::new("Settings")
                .collapsible(false)
                .resizable(true)
                .default_width(360.0)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    settings.draw_panel(ui);
                    ui.separator();
                    if ui.button("Close").clicked() {
                        close_clicked = true;
                    }
                });
            if close_clicked {
                self.settings_open = false;
            }
        }

        action
    }
}
