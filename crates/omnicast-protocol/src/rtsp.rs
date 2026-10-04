//! Minimal async RTSP server for session negotiation scaffolding.

use crate::rtp::{parse_rtp_packet, H264Depacketizer};
use omnicast_core::{AppEvent, DeviceId, SessionInfo, SessionState};
use std::net::SocketAddr;
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, UdpSocket};
use tokio::sync::mpsc;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum RtspError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

#[derive(Clone, Debug)]
pub struct RtspServerConfig {
    pub bind_addr: SocketAddr,
    pub rtp_port: u16,
}

impl Default for RtspServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 8554)),
            rtp_port: 5004,
        }
    }
}

/// Accepts RTSP TCP sessions and demuxes RTP on UDP into NAL callbacks via AppEvent.
pub struct RtspServer {
    config: RtspServerConfig,
}

impl RtspServer {
    pub fn new(config: RtspServerConfig) -> Self {
        Self { config }
    }

    pub async fn run(self, events: mpsc::Sender<AppEvent>) -> Result<(), RtspError> {
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        info!(addr = %self.config.bind_addr, "RTSP listening");

        let rtp_sock = Arc::new(UdpSocket::bind(("0.0.0.0", self.config.rtp_port)).await?);
        info!(port = self.config.rtp_port, "RTP UDP listening");

        let rtp_events = events.clone();
        let rtp_sock_task = Arc::clone(&rtp_sock);
        tokio::spawn(async move {
            if let Err(err) = rtp_loop(rtp_sock_task, rtp_events).await {
                warn!(error = %err, "RTP loop exited");
            }
        });

        loop {
            let (mut socket, peer) = listener.accept().await?;
            let events = events.clone();
            let rtp_port = self.config.rtp_port;
            tokio::spawn(async move {
                if let Err(err) = handle_client(&mut socket, peer, rtp_port, events).await {
                    warn!(%peer, error = %err, "RTSP session ended with error");
                }
            });
        }
    }
}

async fn rtp_loop(sock: Arc<UdpSocket>, events: mpsc::Sender<AppEvent>) -> Result<(), RtspError> {
    let mut buf = vec![0u8; 2048];
    let mut depacketizer = H264Depacketizer::new();
    // Until a real session maps SSRCs → devices, attach NALs to a placeholder stream.
    let placeholder = DeviceId::new();

    loop {
        let (n, _from) = sock.recv_from(&mut buf).await?;
        match parse_rtp_packet(&buf[..n]) {
            Ok(packet) => match depacketizer.push(&packet) {
                Ok(nals) => {
                    for nal in nals {
                        let _ = events
                            .send(AppEvent::Status(format!(
                                "NAL {} bytes (pt={}, seq={}) device={}",
                                nal.data.len(),
                                packet.payload_type,
                                packet.sequence,
                                placeholder
                            )))
                            .await;
                    }
                }
                Err(err) => warn!(error = %err, "H.264 depacketize error"),
            },
            Err(err) => warn!(error = %err, "RTP parse error"),
        }
    }
}

async fn handle_client(
    socket: &mut tokio::net::TcpStream,
    peer: SocketAddr,
    rtp_port: u16,
    events: mpsc::Sender<AppEvent>,
) -> Result<(), RtspError> {
    let mut session = SessionInfo::new(format!("RTSP {peer}"), "rtsp");
    session.state = SessionState::Active;
    let device_id = session.id;
    let _ = events
        .send(AppEvent::DeviceConnected(session.clone()))
        .await;

    let mut buf = vec![0u8; 8192];
    let mut read_buf = Vec::new();

    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            break;
        }
        read_buf.extend_from_slice(&buf[..n]);

        while let Some(idx) = find_header_end(&read_buf) {
            let request = String::from_utf8_lossy(&read_buf[..idx + 4]).to_string();
            read_buf.drain(..idx + 4);
            let response = build_response(&request, rtp_port);
            socket.write_all(response.as_bytes()).await?;
        }
    }

    let _ = events
        .send(AppEvent::DeviceDisconnected {
            id: device_id,
            reason: "tcp_closed".into(),
        })
        .await;
    Ok(())
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn build_response(request: &str, rtp_port: u16) -> String {
    let cseq = request
        .lines()
        .find_map(|l| l.strip_prefix("CSeq:").map(|v| v.trim().to_string()))
        .unwrap_or_else(|| "0".into());

    let method = request
        .lines()
        .next()
        .and_then(|l| l.split_whitespace().next())
        .unwrap_or("");

    match method {
        "OPTIONS" => format!(
            "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nPublic: OPTIONS, DESCRIBE, SETUP, PLAY, TEARDOWN\r\n\r\n"
        ),
        "DESCRIBE" => {
            let sdp = format!(
                "v=0\r\n\
                 o=- 0 0 IN IP4 0.0.0.0\r\n\
                 s=OmniCast\r\n\
                 t=0 0\r\n\
                 m=video {rtp_port} RTP/AVP 96\r\n\
                 a=rtpmap:96 H264/90000\r\n\
                 a=fmtp:96 packetization-mode=1\r\n\
                 a=control:trackID=0\r\n"
            );
            format!(
                "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nContent-Type: application/sdp\r\nContent-Length: {}\r\n\r\n{sdp}",
                sdp.len()
            )
        }
        "SETUP" => format!(
            "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nTransport: RTP/AVP;unicast;client_port={rtp_port}-{};\r\nSession: omnicast\r\n\r\n",
            rtp_port + 1
        ),
        "PLAY" => format!(
            "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: omnicast\r\n\r\n"
        ),
        "TEARDOWN" => format!(
            "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nSession: omnicast\r\n\r\n"
        ),
        _ => format!("RTSP/1.0 501 Not Implemented\r\nCSeq: {cseq}\r\n\r\n"),
    }
}
