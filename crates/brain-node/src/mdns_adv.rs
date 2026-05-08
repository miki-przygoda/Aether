use anyhow::Result;
use mdns_sd::{ServiceDaemon, ServiceInfo};
use std::collections::HashMap;

const SERVICE_TYPE: &str = "_aether._tcp.local.";

/// Advertise the brain on the local network via mDNS.
/// Returns the daemon handle — keep it alive for as long as you want to advertise.
pub fn advertise(port: u16, ip: std::net::Ipv4Addr) -> Result<ServiceDaemon> {
    let mdns = ServiceDaemon::new()?;
    let service = ServiceInfo::new(
        SERVICE_TYPE,
        "aether-brain",
        "brain.local.",
        std::net::IpAddr::V4(ip),
        port,
        HashMap::new(),
    )?;
    mdns.register(service)?;
    tracing::info!(ip = %ip, port, "mDNS: advertising {SERVICE_TYPE}");
    Ok(mdns)
}
