//! Google Cast TLS control listener (port 8009).
//!
//! Completes the TLS handshake, then runs a Cast V2 control loop that answers
//! CONNECT / PING / GET_STATUS so senders like pychromecast can finish `wait()`.

use crate::cast_v2::{self, handle_message, try_decode_frame};
use omnicast_core::AppEvent;
use rcgen::{
    CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose, RsaKeySize,
    PKCS_RSA_SHA256,
};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::sync::Arc;
use thiserror::Error;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{mpsc, watch};
use tokio_rustls::TlsAcceptor;
use tracing::{debug, info, warn};

#[derive(Debug, Error)]
pub enum CastError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tls error: {0}")]
    Tls(String),
    #[error("cast v2 error: {0}")]
    V2(#[from] cast_v2::CastV2Error),
}

#[derive(Clone, Debug)]
pub struct CastServerConfig {
    pub bind_addr: SocketAddr,
    /// Primary LAN IPv4 included in the certificate SAN list.
    pub lan_ip: Option<Ipv4Addr>,
    /// mDNS hostname without trailing dot (e.g. `omnicast.local`).
    pub mdns_hostname: String,
    /// Friendly name / DNS SAN (e.g. `OmniCast`).
    pub receiver_name: String,
}

impl Default for CastServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 8009)),
            lan_ip: primary_ipv4(),
            mdns_hostname: "omnicast.local".into(),
            receiver_name: "OmniCast".into(),
        }
    }
}

pub struct CastTlsServer {
    config: CastServerConfig,
}

impl CastTlsServer {
    pub fn new(config: CastServerConfig) -> Self {
        Self { config }
    }

    pub async fn run(
        self,
        events: mpsc::Sender<AppEvent>,
        mut running: watch::Receiver<bool>,
    ) -> Result<(), CastError> {
        let acceptor = TlsAcceptor::from(Arc::new(build_server_config(&self.config)?));
        let listener = TcpListener::bind(self.config.bind_addr).await?;
        info!(
            addr = %listener.local_addr().unwrap_or(self.config.bind_addr),
            "Cast TLS listener ACTIVE on port 8009 (Google Cast control)"
        );

        loop {
            if !*running.borrow() {
                info!("Cast TLS listener stopping");
                break;
            }

            tokio::select! {
                _ = running.changed() => {
                    if !*running.borrow() {
                        info!("Cast TLS listener stopping");
                        break;
                    }
                }
                accept = listener.accept() => {
                    let (socket, peer) = accept?;
                    let _ = socket.set_nodelay(true);
                    debug!(%peer, "Cast TLS TCP accept — handing socket to rustls (no pre-read)");
                    let acceptor = acceptor.clone();
                    let events = events.clone();
                    tokio::spawn(async move {
                        match handle_cast_tls(socket, peer, acceptor, events).await {
                            Ok(()) => {}
                            Err(CastError::Tls(msg)) if is_probe_disconnect(&msg) => {
                                debug!(
                                    %peer,
                                    "Client disconnected during TLS handshake (probe/scan)"
                                );
                            }
                            Err(CastError::Io(err))
                                if err.kind() == std::io::ErrorKind::UnexpectedEof
                                    || err.kind() == std::io::ErrorKind::ConnectionReset =>
                            {
                                debug!(%peer, error = %err, "Cast session closed by peer");
                            }
                            Err(err) => {
                                warn!(%peer, error = %err, "Cast TLS session ended");
                            }
                        }
                    });
                }
            }
        }
        Ok(())
    }
}

async fn handle_cast_tls(
    socket: TcpStream,
    peer: SocketAddr,
    acceptor: TlsAcceptor,
    events: mpsc::Sender<AppEvent>,
) -> Result<(), CastError> {
    // CRITICAL: never read/peek the TcpStream before accept().
    let mut tls = match acceptor.accept(socket).await {
        Ok(stream) => stream,
        Err(e) => {
            let msg = e.to_string();
            if is_probe_disconnect(&msg) {
                return Err(CastError::Tls(msg));
            }
            let _ = events.try_send(AppEvent::Status(format!(
                "[WARN] TLS handshake failed with {}: {msg}",
                peer.ip()
            )));
            return Err(CastError::Tls(msg));
        }
    };

    info!(%peer, "Cast TLS handshake completed — starting Cast V2 message loop");
    let _ = events
        .send(AppEvent::Status(format!(
            "[INFO] Incoming connection from {}",
            peer.ip()
        )))
        .await;
    let _ = events
        .send(AppEvent::Status(format!(
            "[INFO] Handshake successful — Cast V2 session open ({})",
            peer.ip()
        )))
        .await;

    run_cast_v2_loop(&mut tls, peer, &events).await
}

async fn run_cast_v2_loop<S>(
    tls: &mut S,
    peer: SocketAddr,
    events: &mpsc::Sender<AppEvent>,
) -> Result<(), CastError>
where
    S: AsyncReadExt + AsyncWriteExt + Unpin,
{
    let mut buf = Vec::with_capacity(4096);
    let mut tmp = [0u8; 4096];
    let mut connected = false;

    loop {
        let n = tls.read(&mut tmp).await?;
        if n == 0 {
            debug!(%peer, "Cast peer closed TLS stream");
            break;
        }
        buf.extend_from_slice(&tmp[..n]);

        while let Some((msg, consumed)) = try_decode_frame(&buf)? {
            buf.drain(..consumed);
            let payload_type = msg
                .payload_json()
                .ok()
                .and_then(|v| v.get("type").and_then(|t| t.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| "(non-json)".into());

            info!(
                %peer,
                namespace = %msg.namespace,
                from = %msg.source_id,
                to = %msg.destination_id,
                r#type = %payload_type,
                "Cast V2 message"
            );

            if msg.namespace == cast_v2::NS_CONNECTION && payload_type == "CONNECT" && !connected
            {
                connected = true;
                let _ = events
                    .send(AppEvent::Status(format!(
                        "[INFO] Cast CONNECT from {} ({})",
                        msg.source_id,
                        peer.ip()
                    )))
                    .await;
            }

            let replies = handle_message(&msg)?;
            for frame in replies {
                tls.write_all(&frame).await?;
                tls.flush().await?;
                debug!(%peer, bytes = frame.len(), "Cast V2 reply sent");
            }

            if msg.namespace == cast_v2::NS_RECEIVER && payload_type == "GET_STATUS" {
                let _ = events
                    .send(AppEvent::Status(format!(
                        "[INFO] RECEIVER_STATUS sent to {}",
                        peer.ip()
                    )))
                    .await;
            }
        }
    }
    Ok(())
}

fn is_probe_disconnect(msg: &str) -> bool {
    let lower = msg.to_ascii_lowercase();
    lower.contains("tls handshake eof")
        || lower.contains("unexpected eof")
        || lower.contains("connection reset")
        || lower.contains("forcibly closed")
        || lower.contains("broken pipe")
}

fn build_server_config(cfg: &CastServerConfig) -> Result<ServerConfig, CastError> {
    let _ = rustls::crypto::ring::default_provider().install_default();

    let lan = cfg.lan_ip.or_else(primary_ipv4);
    let mut ordered = vec![
        "localhost".to_string(),
        "OmniCast".to_string(),
        cfg.receiver_name.clone(),
        cfg.mdns_hostname.clone(),
        "127.0.0.1".to_string(),
    ];
    if let Some(ip) = lan {
        ordered.push(ip.to_string());
    }
    ordered.sort();
    ordered.dedup();

    info!(sans = ?ordered, "generating Cast TLS RSA-2048 certificate with SANs");

    let key_pair = KeyPair::generate_rsa_for(&PKCS_RSA_SHA256, RsaKeySize::_2048)
        .map_err(|e| CastError::Tls(e.to_string()))?;
    let mut params =
        CertificateParams::new(ordered).map_err(|e| CastError::Tls(e.to_string()))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "OmniCast Receiver");
    params.key_usages = vec![
        KeyUsagePurpose::DigitalSignature,
        KeyUsagePurpose::KeyEncipherment,
    ];
    params.extended_key_usages = vec![ExtendedKeyUsagePurpose::ServerAuth];

    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| CastError::Tls(e.to_string()))?;

    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    let mut config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
    .map_err(|e| CastError::Tls(e.to_string()))?
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key_der)
    .map_err(|e| CastError::Tls(e.to_string()))?;

    config.alpn_protocols.clear();
    Ok(config)
}

fn primary_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}
