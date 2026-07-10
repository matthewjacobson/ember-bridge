//! Brother machine backend (Innov-is / WLAN-capable models).
//!
//! Implements the manufacturer-neutral traits from [`crate::machine`] on top
//! of the reverse-engineered "pedxml" HTTPS protocol spoken by Brother's
//! *Design Database Transfer* application.
//!
//! Layout:
//! * [`protocol`] — pure wire format: request bodies, response parsing.
//! * [`models`] — serde types for the machine's JSON/XML payloads.
//! * [`client`] — the HTTPS client ([`BrotherClient`]) with its TLS quirks.
//! * [`discovery`] — active /24 subnet sweep.

pub mod client;
pub mod discovery;
pub mod models;
pub mod protocol;

pub use client::BrotherClient;

use crate::machine::{
    DiscoveredMachine, EmbroideryMachine, MachineBackend, MachineError, MachineInfo,
    ScanProgressFn,
};
use async_trait::async_trait;
use std::net::IpAddr;
use std::sync::Arc;

/// The Brother backend registered in [`crate::machine::BackendRegistry`].
pub struct BrotherBackend;

impl BrotherBackend {
    pub fn new() -> Self {
        Self
    }
}

impl Default for BrotherBackend {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl MachineBackend for BrotherBackend {
    fn manufacturer(&self) -> &'static str {
        "brother"
    }

    async fn probe(&self, ip: IpAddr) -> Result<Option<MachineInfo>, MachineError> {
        let client = BrotherClient::new(ip);
        match client.probe_info().await {
            // A device is "ours" iff it advertises the pedxml transfer API.
            Ok(raw) if raw.supports_pedxml() => Ok(Some(client.to_machine_info(&raw))),
            Ok(_) => Ok(None),
            Err(e) => Err(e),
        }
    }

    fn connect(&self, ip: IpAddr) -> Arc<dyn EmbroideryMachine> {
        Arc::new(BrotherClient::new(ip))
    }

    async fn discover(&self, on_progress: ScanProgressFn) -> Vec<DiscoveredMachine> {
        discovery::discover(on_progress).await
    }
}
