//! Video decode pipeline: H.264 NAL stream → raw frames.
//!
//! v0.1.0-alpha ships a mock decoder suitable for pipeline soak testing.
//! Hardware-accelerated backends land in Phase 2.

use bytes::Bytes;
use omnicast_core::{DeviceId, FrameFormat, VideoFrame};
use thiserror::Error;
use tracing::debug;

#[derive(Debug, Error)]
pub enum MediaError {
    #[error("decode error: {0}")]
    Decode(&'static str),
}

/// Trait for pluggable decode backends.
pub trait VideoDecoder: Send {
    fn push_nal(&mut self, nal: &[u8]) -> Result<Option<VideoFrame>, MediaError>;
    fn device_id(&self) -> DeviceId;
}

/// Placeholder decoder that acknowledges NALs but does not emit frames.
/// Real frames in Milestone 1 come from the app's synthetic generator.
#[derive(Debug)]
pub struct StubH264Decoder {
    device_id: DeviceId,
    nal_count: u64,
}

impl StubH264Decoder {
    pub fn new(device_id: DeviceId) -> Self {
        Self {
            device_id,
            nal_count: 0,
        }
    }
}

impl VideoDecoder for StubH264Decoder {
    fn push_nal(&mut self, nal: &[u8]) -> Result<Option<VideoFrame>, MediaError> {
        self.nal_count += 1;
        debug!(
            device = %self.device_id,
            nals = self.nal_count,
            bytes = nal.len(),
            "stub decoder accepted NAL"
        );
        Ok(None)
    }

    fn device_id(&self) -> DeviceId {
        self.device_id
    }
}

/// Generates animated RGBA frames for GPU path verification.
pub struct MockFrameGenerator {
    device_id: DeviceId,
    width: u32,
    height: u32,
    frame_index: u64,
    pixels: Vec<u8>,
}

impl MockFrameGenerator {
    pub fn new(device_id: DeviceId, width: u32, height: u32) -> Self {
        let len = (width * height * 4) as usize;
        Self {
            device_id,
            width,
            height,
            frame_index: 0,
            pixels: vec![0u8; len],
        }
    }

    pub fn next_frame(&mut self) -> VideoFrame {
        let t = self.frame_index;
        self.frame_index = self.frame_index.wrapping_add(1);

        let phase = (t % 255) as u8;
        let w = self.width as usize;
        // Stride the fill for CPU budget while keeping a lively gradient.
        for y in 0..self.height as usize {
            let yf = ((y as u32 * 255) / self.height.max(1)) as u8;
            for x in 0..w {
                let idx = (y * w + x) * 4;
                let xf = ((x as u32 * 255) / self.width.max(1)) as u8;
                self.pixels[idx] = xf.wrapping_add(phase);
                self.pixels[idx + 1] = yf.wrapping_add(phase / 2);
                self.pixels[idx + 2] = 90u8.wrapping_add(phase / 3);
                self.pixels[idx + 3] = 255;
            }
        }

        VideoFrame {
            device_id: self.device_id,
            width: self.width,
            height: self.height,
            format: FrameFormat::Rgba8,
            data: Bytes::copy_from_slice(&self.pixels),
            pts_ms: t.saturating_mul(16),
        }
    }
}
