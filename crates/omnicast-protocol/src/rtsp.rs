//! Live async RTSP control server for a single-device Milestone 1 handshake.

use crate::rtp::{parse_rtp_packet, H264Depacketizer};
use omnicast_core::{AppEvent, DeviceId, SessionInfo, SessionState};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket as StdUdpSocket};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

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

/// Parsed session parameters observed during the RTSP handshake.
#[derive(Clone, Debug, Default)]
pub struct SessionParams {
    pub peer_ip: String,
    pub peer_tcp_port: u16,
    pub client_rtp_port: Option<u16>,
    pub client_rtcp_port: Option<u16>,
    pub server_rtp_port: u16,
    pub transport: String,
    pub video_codec: String,
    pub payload_type: Option<u8>,
    pub clock_rate: Option<u32>,
    pub interleaved: bool,
    pub request_uri: String,
    pub user_agent: String,
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
        let local = listener.local_addr().unwrap_or(self.config.bind_addr);
        let server_ip = primary_ipv4()
            .map(|ip| ip.to_string())
            .unwrap_or_else(|| "0.0.0.0".into());

        info!(
            bind = %local,
            server_ip = %server_ip,
            rtp_port = self.config.rtp_port,
            "RTSP handshake listener ACTIVE — waiting for phone to connect to OmniCast"
        );

        let rtp_sock = Arc::new(UdpSocket::bind(("0.0.0.0", self.config.rtp_port)).await?);
        info!(
            port = self.config.rtp_port,
            "RTP/UDP listener ACTIVE (expect H.264 after SETUP/PLAY)"
        );

        let rtp_events = events.clone();
        let rtp_sock_task = Arc::clone(&rtp_sock);
        tokio::spawn(async move {
            if let Err(err) = rtp_loop(rtp_sock_task, rtp_events).await {
                warn!(error = %err, "RTP loop exited");
            }
        });

        loop {
            let (socket, peer) = listener.accept().await?;
            info!(
                peer_ip = %peer.ip(),
                peer_port = peer.port(),
                "accepted incoming TCP connection (RTSP handshake starting)"
            );
            let _ = events
                .send(AppEvent::Status(format!(
                    "TCP accept from {}:{}",
                    peer.ip(),
                    peer.port()
                )))
                .await;

            let events = events.clone();
            let rtp_port = self.config.rtp_port;
            let server_ip = server_ip.clone();
            tokio::spawn(async move {
                if let Err(err) = handle_client(socket, peer, rtp_port, server_ip, events).await {
                    warn!(%peer, error = %err, "RTSP session ended with error");
                }
            });
        }
    }
}

async fn rtp_loop(sock: Arc<UdpSocket>, events: mpsc::Sender<AppEvent>) -> Result<(), RtspError> {
    let mut buf = vec![0u8; 2048];
    let mut depacketizer = H264Depacketizer::new();
    let placeholder = DeviceId::new();
    let mut packet_count: u64 = 0;

    loop {
        let (n, from) = sock.recv_from(&mut buf).await?;
        packet_count += 1;
        log_raw_packet("RTP/UDP", from, &buf[..n]);

        match parse_rtp_packet(&buf[..n]) {
            Ok(packet) => {
                info!(
                    from_ip = %from.ip(),
                    from_port = from.port(),
                    payload_type = packet.payload_type,
                    seq = packet.sequence,
                    timestamp = packet.timestamp,
                    ssrc = packet.ssrc,
                    marker = packet.marker,
                    payload_len = packet.payload.len(),
                    packet_count,
                    video_codec = "H264",
                    "RTP packet"
                );
                match depacketizer.push(&packet) {
                    Ok(nals) => {
                        for nal in nals {
                            let nal_type = nal.data.first().map(|b| b & 0x1f).unwrap_or(0);
                            info!(
                                nal_type,
                                nal_bytes = nal.data.len(),
                                "H.264 NAL unit extracted"
                            );
                            let _ = events
                                .send(AppEvent::Status(format!(
                                    "NAL type={nal_type} {}B pt={} seq={} from={}:{}",
                                    nal.data.len(),
                                    packet.payload_type,
                                    packet.sequence,
                                    from.ip(),
                                    from.port()
                                )))
                                .await;
                            let _ = placeholder;
                        }
                    }
                    Err(err) => warn!(error = %err, "H.264 depacketize error"),
                }
            }
            Err(err) => warn!(error = %err, from = %from, "RTP parse error"),
        }
    }
}

async fn handle_client(
    mut socket: TcpStream,
    peer: SocketAddr,
    rtp_port: u16,
    server_ip: String,
    events: mpsc::Sender<AppEvent>,
) -> Result<(), RtspError> {
    let mut params = SessionParams {
        peer_ip: peer.ip().to_string(),
        peer_tcp_port: peer.port(),
        server_rtp_port: rtp_port,
        video_codec: "H264".into(),
        transport: String::new(),
        request_uri: String::new(),
        user_agent: String::new(),
        ..Default::default()
    };

    let mut session = SessionInfo::new(format!("Phone {}", peer.ip()), "rtsp");
    session.state = SessionState::Connecting;
    let device_id = session.id;

    info!(
        device_id = %device_id,
        peer_ip = %params.peer_ip,
        peer_tcp_port = params.peer_tcp_port,
        "RTSP session created — negotiating"
    );

    let mut buf = vec![0u8; 8192];
    let mut read_buf = Vec::new();
    let mut session_token = format!("omnicast-{}", &device_id.to_string()[..8]);
    let mut connected_emitted = false;

    loop {
        let n = socket.read(&mut buf).await?;
        if n == 0 {
            info!(
                peer_ip = %params.peer_ip,
                "TCP peer closed connection"
            );
            break;
        }

        log_raw_packet("RTSP/TCP", peer, &buf[..n]);
        read_buf.extend_from_slice(&buf[..n]);

        // Interleaved binary RTP over RTSP (`$` framing).
        while let Some(consumed) = try_consume_interleaved(&mut read_buf) {
            info!(
                peer_ip = %params.peer_ip,
                channel = consumed.channel,
                bytes = consumed.payload.len(),
                "interleaved RTP/RTCP over RTSP TCP"
            );
            log_hex_preview("interleaved", &consumed.payload);
        }

        while let Some(msg) = try_parse_rtsp_message(&mut read_buf) {
            info!(
                peer_ip = %params.peer_ip,
                peer_tcp_port = params.peer_tcp_port,
                bytes = msg.raw.len(),
                "<<<< RTSP request from phone\n{}",
                sanitize_for_log(&msg.raw)
            );

            update_params_from_request(&mut params, &msg);
            let response = build_response(&msg, &mut params, &server_ip, &mut session_token);

            info!(
                peer_ip = %params.peer_ip,
                method = %msg.method,
                video_codec = %params.video_codec,
                transport = %params.transport,
                client_rtp = ?params.client_rtp_port,
                client_rtcp = ?params.client_rtcp_port,
                server_rtp = params.server_rtp_port,
                payload_type = ?params.payload_type,
                user_agent = %params.user_agent,
                "session parameters"
            );

            info!(
                peer_ip = %params.peer_ip,
                ">>>> RTSP response to phone\n{}",
                sanitize_for_log(&response)
            );

            socket.write_all(response.as_bytes()).await?;

            if msg.method == "PLAY" && !connected_emitted {
                session.state = SessionState::Active;
                session.device_name = format!("Phone {}", params.peer_ip);
                let _ = events
                    .send(AppEvent::DeviceConnected(session.clone()))
                    .await;
                connected_emitted = true;
                info!(
                    device_id = %device_id,
                    peer_ip = %params.peer_ip,
                    video_codec = %params.video_codec,
                    "PLAY accepted — single-device session ACTIVE"
                );
            }

            if msg.method == "TEARDOWN" {
                info!(peer_ip = %params.peer_ip, "TEARDOWN received");
                break;
            }
        }
    }

    if connected_emitted {
        let _ = events
            .send(AppEvent::DeviceDisconnected {
                id: device_id,
                reason: "tcp_closed".into(),
            })
            .await;
    }

    info!(
        peer_ip = %params.peer_ip,
        video_codec = %params.video_codec,
        client_rtp = ?params.client_rtp_port,
        "RTSP session finished"
    );
    Ok(())
}

struct RtspMessage {
    method: String,
    uri: String,
    headers: Vec<(String, String)>,
    body: String,
    raw: String,
}

struct InterleavedPacket {
    channel: u8,
    payload: Vec<u8>,
}

fn try_consume_interleaved(buf: &mut Vec<u8>) -> Option<InterleavedPacket> {
    if buf.first() != Some(&b'$') || buf.len() < 4 {
        return None;
    }
    let channel = buf[1];
    let len = u16::from_be_bytes([buf[2], buf[3]]) as usize;
    if buf.len() < 4 + len {
        return None;
    }
    let payload = buf[4..4 + len].to_vec();
    buf.drain(..4 + len);
    Some(InterleavedPacket { channel, payload })
}

fn try_parse_rtsp_message(buf: &mut Vec<u8>) -> Option<RtspMessage> {
    // Skip leading interleaved markers — handled elsewhere.
    if buf.first() == Some(&b'$') {
        return None;
    }
    let header_end = find_header_end(buf)?;
    let header_bytes = &buf[..header_end];
    let header_text = String::from_utf8_lossy(header_bytes);
    let content_length = header_text
        .lines()
        .find_map(|l| {
            let (k, v) = split_header(l)?;
            if k.eq_ignore_ascii_case("Content-Length") {
                v.parse::<usize>().ok()
            } else {
                None
            }
        })
        .unwrap_or(0);

    let total = header_end + 4 + content_length;
    if buf.len() < total {
        return None;
    }

    let raw = String::from_utf8_lossy(&buf[..total]).to_string();
    buf.drain(..total);

    let mut lines = raw.split("\r\n");
    let request_line = lines.next().unwrap_or("");
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("").to_string();
    let uri = parts.next().unwrap_or("").to_string();

    let mut headers = Vec::new();
    for line in lines.by_ref() {
        if line.is_empty() {
            break;
        }
        if let Some((k, v)) = split_header(line) {
            headers.push((k.to_string(), v.to_string()));
        }
    }
    let body = lines.collect::<Vec<_>>().join("\r\n");

    Some(RtspMessage {
        method,
        uri,
        headers,
        body,
        raw,
    })
}

fn split_header(line: &str) -> Option<(&str, &str)> {
    let (k, v) = line.split_once(':')?;
    Some((k.trim(), v.trim()))
}

fn find_header_end(buf: &[u8]) -> Option<usize> {
    buf.windows(4).position(|w| w == b"\r\n\r\n")
}

fn update_params_from_request(params: &mut SessionParams, msg: &RtspMessage) {
    params.request_uri = msg.uri.clone();
    for (k, v) in &msg.headers {
        if k.eq_ignore_ascii_case("User-Agent") {
            params.user_agent = v.clone();
        }
        if k.eq_ignore_ascii_case("Transport") {
            params.transport = v.clone();
            parse_transport(params, v);
        }
    }

    if !msg.body.is_empty() {
        parse_sdp_body(params, &msg.body);
    }

    // URI hints: rtsp://host/stream
    if msg.uri.to_ascii_lowercase().contains("h265")
        || msg.uri.to_ascii_lowercase().contains("hevc")
    {
        params.video_codec = "H265".into();
    }
}

fn parse_transport(params: &mut SessionParams, transport: &str) {
    params.interleaved = transport.to_ascii_lowercase().contains("interleaved");
    for part in transport.split(';') {
        let part = part.trim();
        if let Some(ports) = part.strip_prefix("client_port=") {
            let mut split = ports.split('-');
            params.client_rtp_port = split.next().and_then(|p| p.parse().ok());
            params.client_rtcp_port = split.next().and_then(|p| p.parse().ok());
        }
    }
}

fn parse_sdp_body(params: &mut SessionParams, body: &str) {
    for line in body.lines() {
        if let Some(rest) = line.strip_prefix("a=rtpmap:") {
            // e.g. 96 H264/90000
            let mut parts = rest.split_whitespace();
            if let Some(pt) = parts.next().and_then(|p| p.parse().ok()) {
                params.payload_type = Some(pt);
            }
            if let Some(codec_rate) = parts.next() {
                let mut cr = codec_rate.split('/');
                if let Some(codec) = cr.next() {
                    params.video_codec = codec.to_string();
                }
                params.clock_rate = cr.next().and_then(|r| r.parse().ok());
            }
        }
        if let Some(rest) = line.strip_prefix("a=fmtp:") {
            info!(fmtp = %rest, "SDP fmtp from phone / offer");
        }
        if let Some(rest) = line.strip_prefix("m=video") {
            info!(media_line = %rest, "SDP media line");
        }
    }
}

fn build_response(
    msg: &RtspMessage,
    params: &mut SessionParams,
    server_ip: &str,
    session_token: &mut String,
) -> String {
    let cseq = header_value(&msg.headers, "CSeq").unwrap_or_else(|| "0".into());
    let session_hdr =
        header_value(&msg.headers, "Session").unwrap_or_else(|| session_token.clone());
    if header_value(&msg.headers, "Session").is_some() {
        *session_token = session_hdr.clone();
    }

    match msg.method.as_str() {
        "OPTIONS" => format!(
            "RTSP/1.0 200 OK\r\n\
             CSeq: {cseq}\r\n\
             Public: OPTIONS, DESCRIBE, ANNOUNCE, SETUP, PLAY, PAUSE, TEARDOWN, GET_PARAMETER, SET_PARAMETER\r\n\
             Server: OmniCast/0.1\r\n\
             \r\n"
        ),
        "DESCRIBE" => {
            let sdp = format!(
                "v=0\r\n\
                 o=- 0 0 IN IP4 {server_ip}\r\n\
                 s=OmniCast\r\n\
                 c=IN IP4 {server_ip}\r\n\
                 t=0 0\r\n\
                 m=video {server_rtp} RTP/AVP 96\r\n\
                 a=rtpmap:96 H264/90000\r\n\
                 a=fmtp:96 packetization-mode=1\r\n\
                 a=control:trackID=0\r\n\
                 a=recvonly\r\n",
                server_rtp = params.server_rtp_port
            );
            params.video_codec = "H264".into();
            params.payload_type = Some(96);
            params.clock_rate = Some(90_000);
            format!(
                "RTSP/1.0 200 OK\r\n\
                 CSeq: {cseq}\r\n\
                 Content-Base: {}\r\n\
                 Content-Type: application/sdp\r\n\
                 Content-Length: {}\r\n\
                 Server: OmniCast/0.1\r\n\
                 \r\n\
                 {sdp}",
                msg.uri,
                sdp.len()
            )
        }
        "ANNOUNCE" => {
            // Phone may push SDP describing its outbound stream.
            if !msg.body.is_empty() {
                parse_sdp_body(params, &msg.body);
                info!(
                    peer_ip = %params.peer_ip,
                    video_codec = %params.video_codec,
                    payload_type = ?params.payload_type,
                    "ANNOUNCE SDP accepted from phone"
                );
            }
            format!(
                "RTSP/1.0 200 OK\r\nCSeq: {cseq}\r\nServer: OmniCast/0.1\r\n\r\n"
            )
        }
        "SETUP" => {
            let client_rtp = params.client_rtp_port.unwrap_or(0);
            let client_rtcp = params
                .client_rtcp_port
                .unwrap_or(client_rtp.saturating_add(1));
            let transport = if params.interleaved
                || params
                    .transport
                    .to_ascii_lowercase()
                    .contains("rtp/avp/tcp")
            {
                params.interleaved = true;
                "RTP/AVP/TCP;unicast;interleaved=0-1;mode=record".to_string()
            } else {
                format!(
                    "RTP/AVP/UDP;unicast;client_port={client_rtp}-{client_rtcp};server_port={0}-{1};mode=record",
                    params.server_rtp_port,
                    params.server_rtp_port + 1
                )
            };
            params.transport = transport.clone();
            format!(
                "RTSP/1.0 200 OK\r\n\
                 CSeq: {cseq}\r\n\
                 Transport: {transport}\r\n\
                 Session: {session_token};timeout=60\r\n\
                 Server: OmniCast/0.1\r\n\
                 \r\n"
            )
        }
        "PLAY" => format!(
            "RTSP/1.0 200 OK\r\n\
             CSeq: {cseq}\r\n\
             Session: {session_token}\r\n\
             RTP-Info: url=trackID=0;seq=0;rtptime=0\r\n\
             Server: OmniCast/0.1\r\n\
             \r\n"
        ),
        "PAUSE" | "TEARDOWN" | "GET_PARAMETER" | "SET_PARAMETER" => format!(
            "RTSP/1.0 200 OK\r\n\
             CSeq: {cseq}\r\n\
             Session: {session_token}\r\n\
             Server: OmniCast/0.1\r\n\
             \r\n"
        ),
        _ => {
            warn!(method = %msg.method, "unimplemented RTSP method");
            format!(
                "RTSP/1.0 501 Not Implemented\r\n\
                 CSeq: {cseq}\r\n\
                 Server: OmniCast/0.1\r\n\
                 \r\n"
            )
        }
    }
}

fn header_value(headers: &[(String, String)], name: &str) -> Option<String> {
    headers
        .iter()
        .find(|(k, _)| k.eq_ignore_ascii_case(name))
        .map(|(_, v)| v.clone())
}

fn log_raw_packet(kind: &str, from: SocketAddr, data: &[u8]) {
    info!(
        kind,
        from_ip = %from.ip(),
        from_port = from.port(),
        bytes = data.len(),
        "incoming network packet"
    );
    log_hex_preview(kind, data);
    if let Ok(text) = std::str::from_utf8(data) {
        if text
            .chars()
            .all(|c| !c.is_control() || c == '\r' || c == '\n' || c == '\t')
        {
            debug!(kind, payload = %sanitize_for_log(text), "packet as text");
        }
    }
}

fn log_hex_preview(kind: &str, data: &[u8]) {
    const PREVIEW: usize = 64;
    let slice = &data[..data.len().min(PREVIEW)];
    let hex: String = slice
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ");
    info!(kind, hex_preview = %hex, total_bytes = data.len(), "packet hex");
}

fn sanitize_for_log(s: &str) -> String {
    s.replace('\r', "\\r")
}

fn primary_ipv4() -> Option<Ipv4Addr> {
    let sock = StdUdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_setup_transport_ports() {
        let mut params = SessionParams::default();
        parse_transport(
            &mut params,
            "RTP/AVP;unicast;client_port=4000-4001;mode=record",
        );
        assert_eq!(params.client_rtp_port, Some(4000));
        assert_eq!(params.client_rtcp_port, Some(4001));
    }

    #[test]
    fn parses_sdp_rtpmap() {
        let mut params = SessionParams::default();
        parse_sdp_body(&mut params, "a=rtpmap:96 H264/90000\r\n");
        assert_eq!(params.video_codec, "H264");
        assert_eq!(params.payload_type, Some(96));
        assert_eq!(params.clock_rate, Some(90_000));
    }
}
