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
    /// Friendly name shown in cast / mirror pickers.
    pub instance_name: String,
    pub host_name: String,
    /// Plain RTSP / Miracast-style control port.
    pub rtsp_port: u16,
    /// Google Cast TLS control port (SRV for `_googlecast._tcp`).
    pub cast_port: u16,
    pub advertise_display: bool,
    pub advertise_googlecast: bool,
    pub advertise_rtsp: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            instance_name: "OmniCast (Laptop)".to_string(),
            host_name: "omnicast".to_string(),
            rtsp_port: 8554,
            cast_port: 8009,
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
    config: DiscoveryConfig,
    device_id: String,
    pub local_ipv4: Option<Ipv4Addr>,
}

impl DiscoveryService {
    pub fn start(config: DiscoveryConfig) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        let local_ipv4 = primary_ipv4();
        let mut service = Self {
            daemon,
            registered: Vec::new(),
            device_id: Uuid::new_v4().simple().to_string(),
            config,
            local_ipv4,
        };
        service.register_all()?;
        Ok(service)
    }

    pub fn config(&self) -> &DiscoveryConfig {
        &self.config
    }

    pub fn is_active(&self) -> bool {
        !self.registered.is_empty()
    }

    /// Dynamically update the friendly name seen by Android Cast scanners.
    pub fn set_instance_name(&mut self, name: impl Into<String>) -> Result<(), DiscoveryError> {
        let name = name.into();
        if name == self.config.instance_name {
            return Ok(());
        }
        info!(old = %self.config.instance_name, new = %name, "updating mDNS receiver name");
        self.unregister_all();
        self.config.instance_name = name;
        self.register_all()
    }

    fn register_all(&mut self) -> Result<(), DiscoveryError> {
        let props = cast_properties(&self.config, &self.device_id);

        if self.config.advertise_googlecast {
            self.register_type("_googlecast._tcp.local.", self.config.cast_port, &props)?;
        }
        if self.config.advertise_display {
            self.register_type("_display._tcp.local.", self.config.cast_port, &props)?;
        }
        if self.config.advertise_rtsp {
            let mut rtsp_props = props.clone();
            rtsp_props.insert("proto".into(), "rtsp".into());
            self.register_type("_rtsp._tcp.local.", self.config.rtsp_port, &rtsp_props)?;
        }

        if let Some(ip) = self.local_ipv4 {
            info!(
                %ip,
                instance = %self.config.instance_name,
                cast_port = self.config.cast_port,
                rtsp_port = self.config.rtsp_port,
                "mDNS advertising OmniCast (UDP 5353 + TCP {} / {})",
                self.config.cast_port,
                self.config.rtsp_port
            );
        } else {
            warn!("could not detect primary IPv4; mDNS will still use addr_auto");
        }
        Ok(())
    }

    fn register_type(
        &mut self,
        service_type: &str,
        port: u16,
        props: &HashMap<String, String>,
    ) -> Result<(), DiscoveryError> {
        let host = format!("{}.local.", self.config.host_name);
        let ip: IpAddr = self
            .local_ipv4
            .map(IpAddr::V4)
            .unwrap_or_else(|| IpAddr::V4(Ipv4Addr::UNSPECIFIED));

        let info = if self.local_ipv4.is_some() {
            ServiceInfo::new(
                service_type,
                &self.config.instance_name,
                &host,
                ip,
                port,
                props.clone(),
            )
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?
            .enable_addr_auto()
        } else {
            ServiceInfo::new(
                service_type,
                &self.config.instance_name,
                &host,
                "",
                port,
                props.clone(),
            )
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?
            .enable_addr_auto()
        };

        let fullname = info.get_fullname().to_string();
        let fn_txt = props.get("fn").cloned().unwrap_or_default();
        let md_txt = props.get("md").cloned().unwrap_or_default();

        self.daemon
            .register(info)
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        self.registered.push(fullname.clone());
        info!(
            %service_type,
            %fullname,
            port,
            fn = %fn_txt,
            md = %md_txt,
            "registered mDNS service"
        );
        Ok(())
    }

    fn unregister_all(&mut self) {
        for name in self.registered.drain(..) {
            if let Err(err) = self.daemon.unregister(&name) {
                warn!(%name, error = %err, "failed to unregister mDNS service");
            }
        }
    }

    pub fn shutdown(mut self) {
        self.unregister_all();
        if let Err(err) = self.daemon.shutdown() {
            warn!(error = %err, "mDNS daemon shutdown error");
        }
    }
}

/// Android Cast Quick Settings scanners expect these Chromecast-style TXT keys.
fn cast_properties(config: &DiscoveryConfig, device_id: &str) -> HashMap<String, String> {
    // `id` must be a 32-char hex UUID without dashes (Uuid::simple()).
    debug_assert_eq!(device_id.len(), 32);
    let mut props = HashMap::new();
    props.insert("id".into(), device_id.to_string());
    props.insert("fn".into(), config.instance_name.clone());
    props.insert("md".into(), "Chromecast".into());
    props.insert("ve".into(), "02".into());
    props.insert("st".into(), "0".into());
    props.insert("ca".into(), "4101".into());
    props.insert("ic".into(), "/setup/icon.png".into());
    props.insert("rm".into(), String::new());
    props.insert("bs".into(), "000000000000".into());
    props.insert("rs".into(), String::new());
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
