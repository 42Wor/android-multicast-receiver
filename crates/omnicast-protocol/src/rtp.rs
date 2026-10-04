//! RTP header parsing and H.264 payload extraction (RFC 6184 subset).

use bytes::{Bytes, BytesMut};
use thiserror::Error;

#[derive(Debug, Error, PartialEq, Eq)]
pub enum RtpError {
    #[error("packet too short")]
    TooShort,
    #[error("unsupported RTP version {0}")]
    BadVersion(u8),
    #[error("incomplete FU-A fragment")]
    IncompleteFragment,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RtpPacket {
    pub payload_type: u8,
    pub sequence: u16,
    pub timestamp: u32,
    pub ssrc: u32,
    pub marker: bool,
    pub payload: Bytes,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct H264Nal {
    pub data: Bytes,
}

/// Parse a single RTP packet (no RTCP).
pub fn parse_rtp_packet(buf: &[u8]) -> Result<RtpPacket, RtpError> {
    if buf.len() < 12 {
        return Err(RtpError::TooShort);
    }
    let version = buf[0] >> 6;
    if version != 2 {
        return Err(RtpError::BadVersion(version));
    }
    let padding = (buf[0] & 0x20) != 0;
    let extension = (buf[0] & 0x10) != 0;
    let csrc_count = (buf[0] & 0x0f) as usize;
    let marker = (buf[1] & 0x80) != 0;
    let payload_type = buf[1] & 0x7f;
    let sequence = u16::from_be_bytes([buf[2], buf[3]]);
    let timestamp = u32::from_be_bytes([buf[4], buf[5], buf[6], buf[7]]);
    let ssrc = u32::from_be_bytes([buf[8], buf[9], buf[10], buf[11]]);

    let mut offset = 12 + csrc_count * 4;
    if buf.len() < offset {
        return Err(RtpError::TooShort);
    }
    if extension {
        if buf.len() < offset + 4 {
            return Err(RtpError::TooShort);
        }
        let ext_len = u16::from_be_bytes([buf[offset + 2], buf[offset + 3]]) as usize;
        offset += 4 + ext_len * 4;
        if buf.len() < offset {
            return Err(RtpError::TooShort);
        }
    }

    let mut end = buf.len();
    if padding {
        let pad = *buf.last().unwrap_or(&0) as usize;
        if pad == 0 || pad > end.saturating_sub(offset) {
            return Err(RtpError::TooShort);
        }
        end -= pad;
    }

    Ok(RtpPacket {
        payload_type,
        sequence,
        timestamp,
        ssrc,
        marker,
        payload: Bytes::copy_from_slice(&buf[offset..end]),
    })
}

/// Reassembles H.264 NAL units from RTP payloads (single NAL + FU-A).
#[derive(Default)]
pub struct H264Depacketizer {
    fu_buffer: BytesMut,
    fu_start_hdr: u8,
}

impl H264Depacketizer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn push(&mut self, packet: &RtpPacket) -> Result<Vec<H264Nal>, RtpError> {
        if packet.payload.is_empty() {
            return Ok(Vec::new());
        }
        let nal_type = packet.payload[0] & 0x1f;
        match nal_type {
            1..=23 => Ok(vec![H264Nal {
                data: packet.payload.clone(),
            }]),
            28 => self.push_fu_a(&packet.payload),
            // STAP-A
            24 => Ok(parse_stap_a(&packet.payload)),
            _ => Ok(Vec::new()),
        }
    }

    fn push_fu_a(&mut self, payload: &[u8]) -> Result<Vec<H264Nal>, RtpError> {
        if payload.len() < 3 {
            return Err(RtpError::TooShort);
        }
        let fu_indicator = payload[0];
        let fu_header = payload[1];
        let start = (fu_header & 0x80) != 0;
        let end = (fu_header & 0x40) != 0;
        let nal_type = fu_header & 0x1f;
        let fragment = &payload[2..];

        if start {
            self.fu_buffer.clear();
            self.fu_start_hdr = (fu_indicator & 0xe0) | nal_type;
            self.fu_buffer.extend_from_slice(&[self.fu_start_hdr]);
            self.fu_buffer.extend_from_slice(fragment);
            return Ok(Vec::new());
        }

        if self.fu_buffer.is_empty() {
            return Err(RtpError::IncompleteFragment);
        }
        self.fu_buffer.extend_from_slice(fragment);
        if end {
            let data = Bytes::copy_from_slice(&self.fu_buffer);
            self.fu_buffer.clear();
            Ok(vec![H264Nal { data }])
        } else {
            Ok(Vec::new())
        }
    }
}

fn parse_stap_a(payload: &[u8]) -> Vec<H264Nal> {
    let mut out = Vec::new();
    let mut i = 1usize;
    while i + 2 <= payload.len() {
        let size = u16::from_be_bytes([payload[i], payload[i + 1]]) as usize;
        i += 2;
        if i + size > payload.len() {
            break;
        }
        out.push(H264Nal {
            data: Bytes::copy_from_slice(&payload[i..i + size]),
        });
        i += size;
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_minimal_rtp() {
        let mut buf = vec![0x80, 96, 0x00, 0x01, 0, 0, 0, 0, 0, 0, 0, 1];
        buf.extend_from_slice(&[0x67, 0x42]); // SPS-like single NAL
        let pkt = parse_rtp_packet(&buf).unwrap();
        assert_eq!(pkt.payload_type, 96);
        assert_eq!(pkt.sequence, 1);
        let mut dep = H264Depacketizer::new();
        let nals = dep.push(&pkt).unwrap();
        assert_eq!(nals.len(), 1);
        assert_eq!(nals[0].data.as_ref(), &[0x67, 0x42]);
    }
}
