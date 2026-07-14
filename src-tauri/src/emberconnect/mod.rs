//! EmberConnect dongle backend.
//!
//! EmberConnect is our own hardware: a WiFi dongle that plugs into an
//! embroidery machine's USB port and emulates a FAT memory stick. Sending a
//! design means HTTP-uploading it to the dongle, which writes it to its
//! microSD card and electrically re-plugs its USB interface so the machine
//! rescans the filesystem. This works with any machine that reads designs
//! from a USB stick — including machines with no network hardware at all.
//!
//! Compared to the Brother backend, the transport is refreshingly boring:
//! plain HTTP on port 80, JSON responses, and mDNS announcements
//! (`_ember-connect._tcp`) instead of a subnet sweep.
//!
//! Layout mirrors `crate::brother`:
//! * [`models`] — serde types for the dongle's JSON payloads.
//! * [`client`] — the HTTP client ([`EmberConnectClient`]).
//! * [`discovery`] — mDNS browse.

pub mod client;
pub mod discovery;
pub mod models;
pub mod tokens;

pub use client::EmberConnectClient;
pub use tokens::TokenStore;

use crate::machine::{
    DiscoveredMachine, EmbroideryMachine, MachineBackend, MachineError, MachineInfo,
    ScanProgressFn,
};
use async_trait::async_trait;
use std::net::IpAddr;
use std::sync::Arc;

pub const MANUFACTURER: &str = "emberconnect";

/// The EmberConnect backend registered in [`crate::machine::BackendRegistry`].
pub struct EmberConnectBackend {
    /// Pairing tokens by dongle serial, shared by every client this backend
    /// hands out (firmware 0.4.0+ requires them; older firmware ignores them).
    tokens: Arc<TokenStore>,
}

impl EmberConnectBackend {
    pub fn new(config_dir: &std::path::Path) -> Self {
        Self::with_tokens(Arc::new(TokenStore::load(config_dir)))
    }

    /// Share a token store owned elsewhere — the USB setup flow pre-pairs
    /// dongles and must write into the same live map this backend reads.
    pub fn with_tokens(tokens: Arc<TokenStore>) -> Self {
        Self { tokens }
    }
}

#[async_trait]
impl MachineBackend for EmberConnectBackend {
    fn manufacturer(&self) -> &'static str {
        MANUFACTURER
    }

    async fn probe(&self, ip: IpAddr) -> Result<Option<MachineInfo>, MachineError> {
        let client = EmberConnectClient::new(ip, self.tokens.clone());
        match client.probe_health().await {
            // A device is "ours" iff /api/health answers with our name.
            Ok(health) if health.is_ember_connect() => {
                Ok(Some(client.to_machine_info(&health)))
            }
            Ok(_) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn connect(&self, ip: IpAddr) -> Arc<dyn EmbroideryMachine> {
        Arc::new(EmberConnectClient::new(ip, self.tokens.clone()))
    }

    async fn discover(&self, on_progress: ScanProgressFn) -> Vec<DiscoveredMachine> {
        discovery::discover(self, on_progress).await
    }
}
