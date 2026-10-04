//! Google Cast TLS control listener (port 8009).
//!
//! Completes the TLS handshake so Android probes succeed, then keeps a stub
//! Cast V2 control channel until a full protocol implementation lands.

use omnicast_core::AppEvent;
use rcgen::{
    CertificateParams, ExtendedKeyUsagePurpose, KeyPair, KeyUsagePurpose, PKCS_ECDSA_P256_SHA256,
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
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum CastError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("tls error: {0}")]
    Tls(String),
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
                    info!(%peer, "Cast TLS TCP accept (expect ClientHello 0x16)");
                    let _ = events.send(AppEvent::Status(format!(
                        "Incoming probe from {} (TCP accept on Cast :8009)",
                        peer.ip()
                    ))).await;
                    let acceptor = acceptor.clone();
                    let events = events.clone();
                    tokio::spawn(async move {
                        if let Err(err) = handle_cast_tls(socket, peer, acceptor, events).await {
                            warn!(%peer, error = %err, "Cast TLS session ended");
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
    let mut peek = [0u8; 3];
    let n = socket.peek(&mut peek).await?;
    if n > 0 {
        info!(
            %peer,
            first_bytes = format!("{:02x?}", &peek[..n]),
            tls = peek[0] == 0x16,
            "Cast port probe"
        );
        if peek[0] == 0x16 {
            let _ = events
                .send(AppEvent::Status(format!(
                    "Incoming probe from {} (TLS ClientHello received)",
                    peer.ip()
                )))
                .await;
        } else {
            warn!(%peer, "non-TLS data on Cast port 8009 — closing");
            let _ = events
                .send(AppEvent::Status(format!(
                    "Non-TLS data on Cast port from {} — closed",
                    peer.ip()
                )))
                .await;
            return Ok(());
        }
    }

    let mut tls = acceptor.accept(socket).await.map_err(|e| {
        let msg = e.to_string();
        let _ = events.try_send(AppEvent::Status(format!(
            "TLS handshake failed with {}: {msg}",
            peer.ip()
        )));
        CastError::Tls(msg)
    })?;
    info!(%peer, "Cast TLS handshake completed (Cast V2 application protocol stub)");
    let _ = events
        .send(AppEvent::Status(format!(
            "TLS Handshake established with Android device ({})",
            peer.ip()
        )))
        .await;

    // Keep the socket briefly so the phone sees a completed handshake.
    let mut buf = [0u8; 1024];
    match tokio::time::timeout(std::time::Duration::from_secs(5), tls.read(&mut buf)).await {
        Ok(Ok(0)) => info!(%peer, "Cast peer closed after handshake"),
        Ok(Ok(n)) => {
            info!(%peer, bytes = n, hex = %hex_preview(&buf[..n]), "Cast post-handshake data");
            let _ = tls.write_all(&[]).await;
        }
        Ok(Err(err)) => warn!(%peer, error = %err, "Cast read error"),
        Err(_) => info!(%peer, "Cast stub idle timeout"),
    }
    Ok(())
}

fn build_server_config(cfg: &CastServerConfig) -> Result<ServerConfig, CastError> {
    // rustls 0.23 requires an explicit crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let lan = cfg.lan_ip.or_else(primary_ipv4);
    let mut sans = vec![
        "127.0.0.1".into(),
        "0.0.0.0".into(),
        "localhost".into(),
        "OmniCast".into(),
        cfg.receiver_name.clone(),
        cfg.mdns_hostname.clone(),
    ];
    if let Some(ip) = lan {
        sans.push(ip.to_string());
    }
    // Deduplicate while preserving order.
    sans.sort();
    sans.dedup();
    // Keep a stable preferred order for readability in logs.
    let mut ordered = Vec::new();
    for preferred in [
        "127.0.0.1",
        "0.0.0.0",
        "localhost",
        "OmniCast",
        cfg.receiver_name.as_str(),
        cfg.mdns_hostname.as_str(),
    ] {
        if sans.iter().any(|s| s == preferred) && !ordered.iter().any(|s: &String| s == preferred) {
            ordered.push(preferred.to_string());
        }
    }
    if let Some(ip) = lan {
        let s = ip.to_string();
        if !ordered.contains(&s) {
            ordered.push(s);
        }
    }
    for s in sans {
        if !ordered.contains(&s) {
            ordered.push(s);
        }
    }

    info!(sans = ?ordered, "generating Cast TLS certificate with SANs");

    // ECDSA P-256 + SHA-256 — widely accepted by Android TLS stacks.
    let key_pair = KeyPair::generate_for(&PKCS_ECDSA_P256_SHA256)
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

    // Explicit TLS 1.2 + 1.3 with ring's default cipher suites
    // (includes ECDHE-ECDSA-AES128-GCM-SHA256 and TLS 1.3 AES-GCM suites).
    let mut config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
    .map_err(|e| CastError::Tls(e.to_string()))?
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key_der)
    .map_err(|e| CastError::Tls(e.to_string()))?;

    // Cast V2 is not HTTP — do not advertise ALPN (avoids ClientHello rejection).
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

fn hex_preview(data: &[u8]) -> String {
    data.iter()
        .take(32)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
