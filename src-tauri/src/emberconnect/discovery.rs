//! Discovery of EmberConnect dongles via mDNS.
//!
//! Unlike Brother machines, our own firmware announces itself
//! (`_ember-connect._tcp.local.`), so discovery is a short passive browse
//! instead of a /24 sweep: collect announcements for a couple of seconds,
//! then confirm each candidate with a real `/api/health` probe (mDNS caches
//! can be stale; the probe also yields the identity we display).

use crate::machine::{DiscoveredMachine, MachineBackend, ScanProgressFn};
use mdns_sd::{ServiceDaemon, ServiceEvent};
use std::collections::HashSet;
use std::net::IpAddr;
use std::time::Duration;

const SERVICE_TYPE: &str = "_ember-connect._tcp.local.";
/// How long to listen for announcements. Dongles answer the initial query
/// within tens of milliseconds; two seconds is generous for lossy WiFi.
const BROWSE_WINDOW: Duration = Duration::from_secs(2);

/// Browse mDNS for dongle candidates. Blocking (the mdns-sd daemon is
/// thread-based); run via `spawn_blocking`.
fn browse_candidates() -> Vec<IpAddr> {
    let Ok(daemon) = ServiceDaemon::new() else {
        return Vec::new();
    };
    let Ok(receiver) = daemon.browse(SERVICE_TYPE) else {
        return Vec::new();
    };

    let mut found: HashSet<IpAddr> = HashSet::new();
    let deadline = std::time::Instant::now() + BROWSE_WINDOW;
    while let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now()) {
        match receiver.recv_timeout(remaining) {
            Ok(ServiceEvent::ServiceResolved(info)) => {
                found.extend(info.get_addresses().iter().copied().map(IpAddr::from));
            }
            Ok(_) => {}
            Err(_) => break, // window elapsed or channel closed
        }
    }
    let _ = daemon.shutdown();

    // Dongles are IPv4 on home networks; keep orderings stable for the UI.
    let mut candidates: Vec<IpAddr> = found.into_iter().filter(|ip| ip.is_ipv4()).collect();
    candidates.sort();
    candidates
}

/// Discover dongles: mDNS browse, then verify each candidate over HTTP.
pub async fn discover(on_progress: ScanProgressFn) -> Vec<DiscoveredMachine> {
    let candidates = tokio::task::spawn_blocking(browse_candidates)
        .await
        .unwrap_or_default();

    let total = candidates.len().max(1);
    if candidates.is_empty() {
        // Nothing announced; report a completed (trivial) scan.
        on_progress(1, 1);
        return Vec::new();
    }

    let backend = super::EmberConnectBackend::new();
    let mut machines = Vec::new();
    for (index, ip) in candidates.into_iter().enumerate() {
        if let Ok(Some(info)) = backend.probe(ip).await {
            machines.push(DiscoveredMachine { info });
        }
        on_progress(index + 1, total);
    }
    machines
}
