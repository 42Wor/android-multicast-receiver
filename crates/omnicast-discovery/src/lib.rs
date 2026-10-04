//! mDNS service advertisement for OmniCast receivers.

use mdns_sd::{ServiceDaemon, ServiceInfo};
use omnicast_core::CoreError;
use std::collections::HashMap;
use thiserror::Error;
use tracing::{info, warn};

#[derive(Debug, Error)]
pub enum DiscoveryError {
    #[error("mDNS error: {0}")]
    Mdns(String),
    #[error(transparent)]
    Core(#[from] CoreError),
}

#[derive(Clone, Debug)]
pub struct DiscoveryConfig {
    pub instance_name: String,
    pub host_name: String,
    pub port: u16,
    pub advertise_display: bool,
    pub advertise_googlecast: bool,
}

impl Default for DiscoveryConfig {
    fn default() -> Self {
        Self {
            instance_name: "OmniCast Receiver".to_string(),
            host_name: "omnicast-receiver".to_string(),
            port: 8554,
            advertise_display: true,
            advertise_googlecast: true,
        }
    }
}

/// Broadcasts receiver presence on the local link via DNS-SD / mDNS.
pub struct DiscoveryService {
    daemon: ServiceDaemon,
    registered: Vec<String>,
}

impl DiscoveryService {
    pub fn start(config: DiscoveryConfig) -> Result<Self, DiscoveryError> {
        let daemon = ServiceDaemon::new().map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        let mut service = Self {
            daemon,
            registered: Vec::new(),
        };

        let props = properties(&config);

        if config.advertise_display {
            service.register_type("_display._tcp.local.", &config, &props)?;
        }
        if config.advertise_googlecast {
            service.register_type("_googlecast._tcp.local.", &config, &props)?;
        }

        info!(
            instance = %config.instance_name,
            port = config.port,
            "mDNS advertisement started"
        );
        Ok(service)
    }

    fn register_type(
        &mut self,
        service_type: &str,
        config: &DiscoveryConfig,
        props: &HashMap<String, String>,
    ) -> Result<(), DiscoveryError> {
        // Full service name: "<instance>.<type>"
        let service_name = format!("{}.{}", config.instance_name, service_type);
        let host = format!("{}.local.", config.host_name);

        let info = ServiceInfo::new(
            service_type,
            &config.instance_name,
            &host,
            "",
            config.port,
            props.clone(),
        )
        .map_err(|e| DiscoveryError::Mdns(e.to_string()))?
        .enable_addr_auto();

        self.daemon
            .register(info)
            .map_err(|e| DiscoveryError::Mdns(e.to_string()))?;
        self.registered.push(service_name);
        info!(%service_type, "registered mDNS service");
        Ok(())
    }

    pub fn shutdown(self) {
        for name in &self.registered {
            if let Err(err) = self.daemon.unregister(name) {
                warn!(%name, error = %err, "failed to unregister mDNS service");
            }
        }
        if let Err(err) = self.daemon.shutdown() {
            warn!(error = %err, "mDNS daemon shutdown error");
        }
    }
}

fn properties(config: &DiscoveryConfig) -> HashMap<String, String> {
    let mut props = HashMap::new();
    props.insert("fn".into(), config.instance_name.clone());
    props.insert("md".into(), "OmniCast".into());
    props.insert("ve".into(), "05".into());
    props.insert("rs".into(), "omnicast-rs".into());
    props.insert("nv".into(), "1".into());
    props.insert("view_only".into(), "1".into());
    props.insert("version".into(), env!("CARGO_PKG_VERSION").into());
    props
}
