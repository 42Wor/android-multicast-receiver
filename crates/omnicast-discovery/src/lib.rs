//! mDNS / DNS-SD advertisement so phones can discover "OmniCast" on the LAN.

use mdns_sd::{ServiceDaemon, ServiceInfo};
use omnicast_core::CoreError;
use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, UdpSocket};
use thiserror::Error;
use tracing::{info, warn};
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS error: {0}")]
    Mdns(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    /// Friendly name shown in cast / mirror pickers (default: OmniCast).
    pub instance_name: String,
    pub host_name: String,
    /// TCP port phones should connect to (RTSP control).
    pub port: u16,
    pub advertise_display: bool,
    pub advertise_googlecast: bool,
    pub advertise_rtsp: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            instance_name: "OmniCast".to_string(),
            host_name: "omnicast".to_string(),
            port: 8554,
            advertise_display: true,
            advertise_googlecast: true,
            advertise_rtsp: true,
        }
    }
}

/// Broadcasts receiver presence on the local link via DNS-SD / mDNS.
pub struct DiscoveryService {
    daemon: ServiceDaemon,
    registered: Vec<String>,
    pub local_ipv4: Option<Ipv4Addr>,
}

impl DiscoveryService {
    pub fn start(config: DiscoveryConfig) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        let local_ipv4 = primary_ipv4();
        let mut service = Self {
            daemon,
            registered: Vec::new(),
            local_ipv4,
        };

        let props = cast_properties(&config);
        let mut types = Vec::new();
        if config.advertise_rtsp {
            types.push("_rtsp._tcp.local.");
        }
        if config.advertise_display {
            types.push("_display._tcp.local.");
        }
        if config.advertise_googlecast {
            types.push("_googlecast._tcp.local.");
        }

        for service_type in types {
            service.register_type(service_type, &config, &props)?;
        }

        if let Some(ip) = local_ipv4 {
            info!(
                %ip,
                instance = %config.instance_name,
                port = config.port,
                "mDNS actively advertising OmniCast on LAN (allow UDP 5353 / TCP {} in firewall)",
                config.port
            );
        } else {
            warn!("could not detect primary IPv4; mDNS will still use addr_auto");
            info!(
                instance = %config.instance_name,
                port = config.port,
                "mDNS advertisement started"
            );
        }

        Ok(service)
    }

    fn register_type(
        &mut self,
        service_type: &str,
        config: &DiscoveryConfig,
        props: &HashMap<String, String>,
    ) -> Result<(), DiscoveryError> {
        let host = format!("{}.local.", config.host_name);
        let mut info = ServiceInfo::new(
            service_type,
            &config.instance_name,
            &host,
            "",
            config.port,
            props.clone(),
        )
        .map_err(|e| DiscoveryError::Mdns(e.to_string()))?
        .enable_addr_auto();

        // Prefer an explicit IPv4 when known so phones resolve immediately.
        if let Some(ip) = self.local_ipv4 {
            info = ServiceInfo::new(
                service_type,
                &config.instance_name,
                &host,
                IpAddr::V4(ip),
                config.port,
                props.clone(),
            )
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?
            .enable_addr_auto();
        }

        let fullname = info.get_fullname().to_string();
        let addrs: Vec<String> = info
            .get_addresses()
            .iter()
            .map(ToString::to_string)
            .collect();

        self.daemon
            .register(info)
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        self.registered.push(fullname.clone());
        info!(
            %service_type,
            %fullname,
            port = config.port,
            ?addrs,
            "registered mDNS service — phones should see '{}'",
            config.instance_name
        );
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        !self.registered.is_empty()
    }

    pub fn shutdown(self) {
        for name in &self.registered {
            if let Err(err) = self.daemon.unregister(name) {
                warn!(%name, error = %err, "failed to unregister mDNS service");
            } else {
                info!(%name, "unregistered mDNS service");
            }
        }
        if let Err(err) = self.daemon.shutdown() {
            warn!(error = %err, "mDNS daemon shutdown error");
        }
    }
}

fn cast_properties(config: &DiscoveryConfig) -> HashMap<String, String> {
    let mut props = HashMap::new();
    // Google Cast–style TXT keys (discovery surface; full Cast TLS is later).
    let id = Uuid::new_v4().simple().to_string();
    props.insert("id".into(), id);
    props.insert("ve".into(), "05".into());
    props.insert("md".into(), "OmniCast".into());
    props.insert("fn".into(), config.instance_name.clone());
    props.insert("ca".into(), "4101".into());
    props.insert("st".into(), "0".into());
    props.insert("bs".into(), "000000000000".into());
    props.insert("rs".into(), String::new());
    // OmniCast / RTSP hints for scanners and our own clients.
    props.insert("path".into(), "/".into());
    props.insert("proto".into(), "rtsp".into());
    props.insert("codec".into(), "H264".into());
    props.insert("view_only".into(), "1".into());
    props.insert("version".into(), env!("CARGO_PKG_VERSION").into());
    props
}

fn primary_ipv4() -> Option<Ipv4Addr> {
    let sock = UdpSocket::bind("0.0.0.0:0").ok()?;
    sock.connect("8.8.8.8:80").ok()?;
    match sock.local_addr().ok()?.ip() {
        IpAddr::V4(v4) if !v4.is_loopback() => Some(v4),
        _ => None,
    }
}
