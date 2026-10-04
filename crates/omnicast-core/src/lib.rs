//! Shared session types and cross-crate messages for OmniCast.

mod error;
mod messages;
mod session;

pub use error::CoreError;
pub use messages::{AppEvent, FrameFormat, VideoFrame};
pub use session::{DeviceId, SessionInfo, SessionState};
