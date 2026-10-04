//! Immediate-mode HUD, dashboard, and settings UI for OmniCast.

mod dashboard;
mod hud;
mod settings;

pub use dashboard::{draw_dashboard, DashboardAction, DashboardDevice, DashboardState};
pub use hud::{HudAction, HudOverlay};
pub use settings::ViewerSettings;
