//! Google Cast TLS control listener (port 8009).
//!
//! Completes the TLS handshake so Android probes succeed, then keeps a stub
//! Cast V2 control channel until a full protocol implementation lands.

use omnicast_core::AppEvent;
use rcgen::{CertificateParams, KeyPair};
use rustls::ServerConfig;
use rustls_pki_types::{CertificateDer, PrivateKeyDer, PrivatePkcs8KeyDer};
use std::net::SocketAddr;
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
}

impl Default for CastServerConfig {
    fn default() -> Self {
        Self {
            bind_addr: SocketAddr::from(([0, 0, 0, 0], 8009)),
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
        let acceptor = TlsAcceptor::from(Arc::new(build_server_config()?));
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
                        "Cast TLS accept from {peer}"
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
    mut socket: TcpStream,
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
        if peek[0] != 0x16 {
            warn!(%peer, "non-TLS data on Cast port 8009 — closing");
            return Ok(());
        }
    }

    let mut tls = acceptor
        .accept(socket)
        .await
        .map_err(|e| CastError::Tls(e.to_string()))?;
    info!(%peer, "Cast TLS handshake completed (Cast V2 application protocol stub)");
    let _ = events
        .send(AppEvent::Status(format!(
            "Cast TLS handshake OK from {peer} (protocol stub)"
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

fn build_server_config() -> Result<ServerConfig, CastError> {
    // rustls 0.23 requires an explicit crypto provider.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let key_pair = KeyPair::generate().map_err(|e| CastError::Tls(e.to_string()))?;
    let mut params = CertificateParams::new(vec!["omnicast.local".into(), "OmniCast".into()])
        .map_err(|e| CastError::Tls(e.to_string()))?;
    params
        .distinguished_name
        .push(rcgen::DnType::CommonName, "OmniCast Receiver");
    let cert = params
        .self_signed(&key_pair)
        .map_err(|e| CastError::Tls(e.to_string()))?;

    let cert_der = CertificateDer::from(cert.der().to_vec());
    let key_der = PrivateKeyDer::Pkcs8(PrivatePkcs8KeyDer::from(key_pair.serialize_der()));

    let mut config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(vec![cert_der], key_der)
        .map_err(|e| CastError::Tls(e.to_string()))?;
    config.alpn_protocols = vec![b"http/1.1".to_vec()];
    Ok(config)
}

fn hex_preview(data: &[u8]) -> String {
    data.iter()
        .take(32)
        .map(|b| format!("{b:02x}"))
        .collect::<Vec<_>>()
        .join(" ")
}
