use serde::{Deserialize, Serialize};

/// Viewer preferences shared between the HUD and application shell.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ViewerSettings {
    // Display
    pub show_fps: bool,
    pub show_bitrate: bool,
    pub show_uptime: bool,
    pub show_packet_stats: bool,
    pub always_on_top: bool,
    pub lock_aspect_ratio: bool,

    // HUD behaviour
    pub hud_pinned: bool,
    pub hud_autohide: bool,

    // Network
    pub listen_port: u16,
    /// Jitter buffer target in milliseconds (lower = less latency).
    pub buffer_size_ms: u32,

    // Appearance
    pub borderless: bool,
}

impl Default for ViewerSettings {
    fn default() -> Self {
        Self {
            show_fps: true,
            show_bitrate: true,
            show_uptime: true,
            show_packet_stats: true,
            always_on_top: false,
            lock_aspect_ratio: true,
            hud_pinned: false,
            hud_autohide: true,
            listen_port: 8554,
            buffer_size_ms: 120,
            borderless: false,
        }
    }
}

impl ViewerSettings {
    pub fn draw_panel(&mut self, ui: &mut egui::Ui) -> bool {
        let mut changed = false;
        ui.heading("OmniCast Settings");
        ui.separator();

        ui.collapsing("Display", |ui| {
            changed |= ui
                .checkbox(&mut self.show_fps, "Show FPS counter")
                .changed();
            changed |= ui
                .checkbox(&mut self.show_bitrate, "Show bitrate")
                .changed();
            changed |= ui
                .checkbox(&mut self.show_uptime, "Show session uptime")
                .changed();
            changed |= ui
                .checkbox(&mut self.show_packet_stats, "Show frame / drop counters")
                .changed();
            changed |= ui
                .checkbox(&mut self.always_on_top, "Always on top")
                .changed();
            changed |= ui
                .checkbox(&mut self.lock_aspect_ratio, "Lock aspect ratio")
                .changed();
        });

        ui.collapsing("HUD", |ui| {
            changed |= ui.checkbox(&mut self.hud_pinned, "Pin top bar").changed();
            changed |= ui
                .checkbox(&mut self.hud_autohide, "Auto-hide top bar when idle")
                .changed();
            ui.label("Shortcut: press H to toggle pin.");
        });

        ui.collapsing("Network", |ui| {
            ui.horizontal(|ui| {
                ui.label("RTSP listen port");
                changed |= ui
                    .add(egui::DragValue::new(&mut self.listen_port).range(1..=65535))
                    .changed();
            });
            ui.horizontal(|ui| {
                ui.label("Buffer (latency ↔ stability)");
                changed |= ui
                    .add(egui::Slider::new(&mut self.buffer_size_ms, 20..=500).suffix(" ms"))
                    .changed();
            });
            ui.weak("Port changes apply on next receiver restart.");
        });

        ui.collapsing("Appearance", |ui| {
            changed |= ui
                .checkbox(&mut self.borderless, "Borderless window (custom chrome)")
                .changed();
            ui.weak("Native OS borders when unchecked.");
        });

        changed
    }
}
