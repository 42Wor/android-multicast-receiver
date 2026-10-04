//! RTSP control plane and RTP / H.264 depacketization.

mod rtp;
mod rtsp;

pub use rtp::{parse_rtp_packet, H264Nal, RtpError, RtpPacket};
pub use rtsp::{RtspServer, RtspServerConfig};
