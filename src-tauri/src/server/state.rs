//! Shared state behind the localhost API.

use crate::config::ConfigStore;
use crate::logging::LogBuffer;
use crate::machine::{BackendRegistry, DiscoveredMachine};
use crate::server::jobs::JobQueue;
use crate::server::pairing::Pairing;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Instant;
use tokio::sync::RwLock;

/// Callback invoked when a browser asks to pair, so the desktop shell can
/// bring its window to the front. Kept as a plain closure so the server
/// stays free of Tauri types (and testable without a windowing system).
pub type PairingNotifyFn = Box<dyn Fn() + Send + Sync>;

/// Result of the most recent discovery sweep, kept so `GET /api/machines`
/// can answer instantly.
#[derive(Default)]
pub struct DiscoveryCache {
    pub machines: Vec<DiscoveredMachine>,
    /// Milliseconds since the Unix epoch of the last completed sweep.
    pub completed_at_ms: Option<u64>,
}

/// Health of the embedded HTTP server, surfaced in the UI so a port
/// conflict is visible instead of silently breaking Ember.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ServerHealth {
    pub running: bool,
    pub port: u16,
    pub error: Option<String>,
}

pub struct AppState {
    pub config: ConfigStore,
    pub registry: BackendRegistry,
    /// EmberConnect pairing tokens — one live map shared between the LAN
    /// backend (reads) and the USB setup flow (pre-pairs new dongles).
    pub dongle_tokens: Arc<crate::emberconnect::TokenStore>,
    pub logs: LogBuffer,
    pub jobs: JobQueue,
    pub discovered: RwLock<DiscoveryCache>,
    pub discovery_running: AtomicBool,
    pub server_health: RwLock<ServerHealth>,
    pub started_at: Instant,
    pub pairing: Pairing,
    pub pairing_notify: Mutex<Option<PairingNotifyFn>>,
}

impl AppState {
    pub fn new(config: ConfigStore, port: u16) -> Self {
        let dongle_tokens = Arc::new(crate::emberconnect::TokenStore::load(config.dir()));
        let registry = BackendRegistry::with_default_backends(dongle_tokens.clone());
        Self {
            config,
            registry,
            dongle_tokens,
            logs: LogBuffer::new(),
            jobs: JobQueue::new(),
            discovered: RwLock::new(DiscoveryCache::default()),
            discovery_running: AtomicBool::new(false),
            server_health: RwLock::new(ServerHealth {
                running: false,
                port,
                error: None,
            }),
            started_at: Instant::now(),
            pairing: Pairing::default(),
            pairing_notify: Mutex::new(None),
        }
    }
}
