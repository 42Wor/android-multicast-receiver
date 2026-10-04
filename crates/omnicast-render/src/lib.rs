//! wgpu surface renderer: RGBA texture blit + egui telemetry overlay.

mod gpu;
mod overlay;
mod surface;
mod ui_window;

pub use gpu::GpuContext;
pub use overlay::OverlayHost;
pub use surface::DeviceSurface;
pub use ui_window::UiOnlyWindow;
