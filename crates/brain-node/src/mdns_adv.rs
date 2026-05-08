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

#[cfg(test)]
mod tests {
    use super::*;
    use mdns_sd::{ServiceDaemon, ServiceEvent};
    use std::net::Ipv4Addr;
    use std::time::Duration;

    /// Advertise `_aether._tcp.local.` then browse for it on the same machine.
    ///
    /// Skipped in CI — requires multicast UDP on an active network interface.
    /// Run manually: `cargo test -p brain-node -- --ignored`
    #[test]
    #[ignore = "requires multicast networking on an active NIC"]
    fn mdns_advertise_and_discover_round_trip() {
        let port = 19876u16;
        let _daemon = advertise(port, Ipv4Addr::LOCALHOST).unwrap();

        let browser = ServiceDaemon::new().unwrap();
        let rx = browser.browse(SERVICE_TYPE).unwrap();

        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            let remaining = deadline.saturating_duration_since(std::time::Instant::now());
            if remaining.is_zero() {
                panic!("mDNS discovery timed out — {SERVICE_TYPE} not found within 5 s");
            }
            match rx.recv_timeout(remaining) {
                Ok(ServiceEvent::ServiceResolved(info)) if info.get_port() == port => return,
                Ok(_) => continue,
                Err(_) => panic!("mDNS channel closed or timed out"),
            }
        }
    }
}
