//! Shared state behind the localhost API.

use crate::config::ConfigStore;
use crate::logging::LogBuffer;
use crate::machine::{BackendRegistry, DiscoveredMachine};
use crate::server::jobs::JobQueue;
use serde::Serialize;
use std::sync::atomic::AtomicBool;
use std::time::Instant;
use tokio::sync::RwLock;

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
    pub logs: LogBuffer,
    pub jobs: JobQueue,
    pub discovered: RwLock<DiscoveryCache>,
    pub discovery_running: AtomicBool,
    pub server_health: RwLock<ServerHealth>,
    pub started_at: Instant,
}

impl AppState {
    pub fn new(config: ConfigStore, port: u16) -> Self {
        Self {
            config,
            registry: BackendRegistry::with_default_backends(),
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
        }
    }
}
