//! wgpu surface renderer: RGBA texture blit + egui telemetry overlay.

mod gpu;
mod overlay;
mod surface;

pub use gpu::GpuContext;
pub use overlay::OverlayHost;
pub use surface::DeviceSurface;
