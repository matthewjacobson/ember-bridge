//! The manufacturer-neutral machine abstraction.
//!
//! Everything above this module (the localhost API, the upload queue, the UI)
//! talks to embroidery machines exclusively through the [`EmbroideryMachine`]
//! and [`MachineBackend`] traits. Everything below it (`crate::brother`, and
//! future `janome`/`bernina`/... modules) implements them.
//!
//! To add support for a new manufacturer:
//!   1. create a sibling module implementing both traits,
//!   2. register the backend in [`BackendRegistry::with_default_backends`].
//!
//! No changes to the Ember-facing API are required.

pub mod error;
pub mod models;
pub mod net;

pub use error::MachineError;
pub use models::*;

/// Progress callback for discovery sweeps: `(probed, total)`.
pub type ScanProgressFn = Arc<dyn Fn(usize, usize) + Send + Sync>;

use async_trait::async_trait;
use std::net::IpAddr;
use std::sync::Arc;

/// A handle to one embroidery machine at a known address.
///
/// Implementations must be cheap to construct: connections are established
/// lazily, per request, because home embroidery machines drop idle
/// connections aggressively.
#[async_trait]
pub trait EmbroideryMachine: Send + Sync {
    /// Backend identifier, e.g. `"brother"`.
    fn manufacturer(&self) -> &'static str;

    /// The address this handle points at.
    fn ip(&self) -> IpAddr;

    /// Identify the machine and read its capabilities.
    async fn info(&self) -> Result<MachineInfo, MachineError>;

    /// Read the machine's design-memory usage and file list.
    async fn storage(&self) -> Result<StorageStatus, MachineError>;

    /// Send a design to the machine.
    ///
    /// Implementations are expected to validate the format and size against
    /// the machine's own limits before transmitting, and to report progress
    /// through `progress` as bytes go out.
    async fn upload(
        &self,
        request: UploadRequest,
        progress: ProgressFn,
    ) -> Result<UploadReceipt, MachineError>;
}

/// A manufacturer backend: knows how to recognize and construct machines.
#[async_trait]
pub trait MachineBackend: Send + Sync {
    /// Backend identifier, e.g. `"brother"`. Stable, lowercase.
    fn manufacturer(&self) -> &'static str;

    /// Quickly check whether the device at `ip` speaks this backend's
    /// protocol. Returns `Ok(Some(info))` if it does, `Ok(None)` if the
    /// device answered but is not one of ours. Used by discovery, so it must
    /// use short timeouts.
    async fn probe(&self, ip: IpAddr) -> Result<Option<MachineInfo>, MachineError>;

    /// Construct a handle to the machine at `ip` (no I/O).
    fn connect(&self, ip: IpAddr) -> Arc<dyn EmbroideryMachine>;

    /// Sweep the local network for this manufacturer's machines.
    async fn discover(&self, on_progress: ScanProgressFn) -> Vec<DiscoveredMachine>;
}

/// The set of installed manufacturer backends.
pub struct BackendRegistry {
    backends: Vec<Arc<dyn MachineBackend>>,
}

impl BackendRegistry {
    /// Registry with every backend this build ships with.
    pub fn with_default_backends() -> Self {
        Self {
            backends: vec![
                Arc::new(crate::brother::BrotherBackend::new()),
                Arc::new(crate::emberconnect::EmberConnectBackend::new()),
            ],
        }
    }

    pub fn backends(&self) -> &[Arc<dyn MachineBackend>] {
        &self.backends
    }

    pub fn by_manufacturer(&self, manufacturer: &str) -> Option<Arc<dyn MachineBackend>> {
        self.backends
            .iter()
            .find(|b| b.manufacturer() == manufacturer)
            .cloned()
    }

    /// Ask every backend to probe `ip`; return a machine handle from the
    /// first backend that recognizes the device, along with its info.
    ///
    /// With a single backend this is trivial; with several it turns "what is
    /// at this IP?" into a manufacturer-agnostic question, which is what lets
    /// the localhost API stay Brother-free.
    pub async fn identify(
        &self,
        ip: IpAddr,
    ) -> Result<Option<(Arc<dyn EmbroideryMachine>, MachineInfo)>, MachineError> {
        let mut last_err: Option<MachineError> = None;
        let mut any_answered = false;
        for backend in &self.backends {
            match backend.probe(ip).await {
                Ok(Some(info)) => return Ok(Some((backend.connect(ip), info))),
                Ok(None) => any_answered = true,
                Err(e) => last_err = Some(e),
            }
        }
        match (any_answered, last_err) {
            // Every backend failed to even reach the device: surface that.
            (false, Some(e)) => Err(e),
            // Device answered at least one backend but none claimed it.
            _ => Ok(None),
        }
    }

    /// Run every backend's discovery and merge the results.
    ///
    /// Backends run sequentially: they sweep the same physical network, and
    /// stacking sweeps would double the probe traffic for no latency win.
    pub async fn discover_all(&self, on_progress: ScanProgressFn) -> Vec<DiscoveredMachine> {
        let mut machines = Vec::new();
        for backend in &self.backends {
            machines.extend(backend.discover(on_progress.clone()).await);
        }
        machines
    }
}
