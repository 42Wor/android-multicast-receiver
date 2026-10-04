//! Google Cast TLS control listener (port 8009).
//!
//! Completes the TLS handshake so Android probes succeed, then keeps a stub
//! Cast V2 control channel until a full protocol implementation lands.

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
                    let _ = socket.set_nodelay(true);
                    info!(%peer, "Cast TLS TCP accept — handing socket to rustls (no pre-read)");
                    let _ = events.send(AppEvent::Status(format!(
                        "[INFO] Incoming connection from {}",
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
    // CRITICAL: never read/peek the TcpStream before accept().
    // Consuming ClientHello bytes (0x16 0x03 0x01…) causes rustls to see EOF
    // and fail with `tlshandshake eof`.
    let mut tls = acceptor.accept(socket).await.map_err(|e| {
        let msg = e.to_string();
        let _ = events.try_send(AppEvent::Status(format!(
            "[WARN] TLS handshake failed with {}: {msg}",
            peer.ip()
        )));
        CastError::Tls(msg)
    })?;
    info!(%peer, "Cast TLS handshake completed (Cast V2 application protocol stub)");
    let _ = events
        .send(AppEvent::Status(format!(
            "[INFO] Handshake successful — ready for screen stream ({})",
            peer.ip()
        )))
        .await;

    // Keep the socket briefly so the phone sees a completed handshake.
    let mut buf = [0u8; 1024];
    match tokio::time::timeout(std::time::Duration::from_secs(8), tls.read(&mut buf)).await {
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

    // RSA-2048 + SHA-256 — best compatibility with Android Cast sender stacks.
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

    // Explicit TLS 1.2 + 1.3 with ring cipher suites
    // (ECDHE-RSA-AES128-GCM-SHA256, TLS_AES_128_GCM_SHA256, etc.).
    let mut config = ServerConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_protocol_versions(&[&rustls::version::TLS12, &rustls::version::TLS13])
    .map_err(|e| CastError::Tls(e.to_string()))?
    .with_no_client_auth()
    .with_single_cert(vec![cert_der], key_der)
    .map_err(|e| CastError::Tls(e.to_string()))?;

    // Cast V2 is not HTTP — do not advertise ALPN.
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
