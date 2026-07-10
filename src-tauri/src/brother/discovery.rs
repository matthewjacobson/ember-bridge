//! Discovery of Brother machines on the local network.
//!
//! Brother machines do not announce themselves via mDNS/SSDP (the official
//! client also scans), so discovery is an active probe:
//!
//!   1. enumerate the host's private IPv4 interfaces,
//!   2. sweep each interface's /24 with a short TCP dial to port 443,
//!   3. for hosts that accept, `GET /info` and check for the `pedxml` API.
//!
//! The sweep is bounded: only RFC-1918/link-local networks, only /24-sized
//! slices (254 addresses), bounded concurrency, sub-second dial timeout —
//! a full scan of one interface takes a few seconds.

use crate::machine::{DiscoveredMachine, MachineBackend, ScanProgressFn};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;

/// How long to wait for a TCP SYN-ACK from a candidate host.
const DIAL_TIMEOUT: Duration = Duration::from_millis(600);
/// Parallel probes. Home routers cope fine with this; it keeps a /24 sweep
/// under ~5 seconds even when most addresses time out.
const CONCURRENCY: usize = 48;

/// The /24 networks to sweep, derived from local interface addresses.
fn candidate_networks() -> Vec<Ipv4Addr> {
    let Ok(interfaces) = if_addrs::get_if_addrs() else {
        return Vec::new();
    };
    let mut networks: Vec<Ipv4Addr> = interfaces
        .into_iter()
        .filter(|iface| !iface.is_loopback())
        .filter_map(|iface| match iface.ip() {
            IpAddr::V4(v4) if v4.is_private() => {
                let octets = v4.octets();
                Some(Ipv4Addr::new(octets[0], octets[1], octets[2], 0))
            }
            _ => None,
        })
        .collect();
    networks.sort();
    networks.dedup();
    networks
}

/// Sweep the local network for Brother machines.
///
/// `on_progress` is called with (probed, total) as addresses complete, so the
/// UI can show a scan bar.
pub async fn discover(on_progress: ScanProgressFn) -> Vec<DiscoveredMachine> {
    let networks = candidate_networks();
    let candidates: Vec<Ipv4Addr> = networks
        .iter()
        .flat_map(|net| {
            let base = net.octets();
            (1u8..=254).map(move |host| Ipv4Addr::new(base[0], base[1], base[2], host))
        })
        .collect();

    let total = candidates.len();
    let semaphore = Arc::new(Semaphore::new(CONCURRENCY));
    let probed = Arc::new(std::sync::atomic::AtomicUsize::new(0));

    let mut tasks = Vec::with_capacity(total);
    for ip in candidates {
        let semaphore = semaphore.clone();
        let probed = probed.clone();
        let on_progress = on_progress.clone();
        tasks.push(tokio::spawn(async move {
            let _permit = semaphore.acquire_owned().await.ok()?;
            let found = probe_address(ip).await;
            let done = probed.fetch_add(1, std::sync::atomic::Ordering::Relaxed) + 1;
            on_progress(done, total);
            found
        }));
    }

    let mut machines = Vec::new();
    for task in tasks {
        if let Ok(Some(machine)) = task.await {
            machines.push(machine);
        }
    }
    machines
}

/// Probe a single address: fast TCP dial, then a real protocol probe.
async fn probe_address(ip: Ipv4Addr) -> Option<DiscoveredMachine> {
    // Cheap reachability filter first: most of the /24 is empty space and a
    // TCP dial is far cheaper than a TLS handshake.
    let addr = SocketAddr::from((ip, 443));
    tokio::time::timeout(DIAL_TIMEOUT, tokio::net::TcpStream::connect(addr))
        .await
        .ok()?
        .ok()?;

    let backend = super::BrotherBackend::new();
    match backend.probe(IpAddr::V4(ip)).await {
        Ok(Some(info)) => Some(DiscoveredMachine { info }),
        _ => None,
    }
}

/// Probe one specific address (used for manual "test connection" flows).
pub async fn probe_one(ip: IpAddr) -> Option<DiscoveredMachine> {
    match ip {
        IpAddr::V4(v4) => probe_address(v4).await,
        IpAddr::V6(_) => None,
    }
}

