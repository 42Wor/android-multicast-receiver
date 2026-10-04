use crate::session::{DeviceId, SessionInfo};
use bytes::Bytes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FrameFormat {
    Rgba8,
    Nv12,
}

/// A decoded (or synthetic) video frame ready for GPU upload.
#[derive(Clone, Debug)]
pub struct VideoFrame {
    pub device_id: DeviceId,
    pub width: u32,
    pub height: u32,
    pub format: FrameFormat,
    pub data: Bytes,
    pub pts_ms: u64,
}

impl VideoFrame {
    pub fn rgba(device_id: DeviceId, width: u32, height: u32, data: Bytes, pts_ms: u64) -> Self {
        Self {
            device_id,
            width,
            height,
            format: FrameFormat::Rgba8,
            data,
            pts_ms,
        }
    }
}

/// Events delivered from async networking / media into the UI event loop.
#[derive(Clone, Debug)]
pub enum AppEvent {
    DeviceConnected(SessionInfo),
    DeviceDisconnected { id: DeviceId, reason: String },
    FrameReady(VideoFrame),
    Status(String),
}
