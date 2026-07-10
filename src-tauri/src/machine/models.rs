//! Manufacturer-neutral data models.
//!
//! These are the only machine-related types the localhost API (and therefore
//! Ember and the React UI) ever sees. Backends translate their native wire
//! formats into these structs.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;

/// Who a machine is: identity data that does not change between requests.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineIdentity {
    /// Backend identifier, e.g. `"brother"`. Stable, lowercase.
    pub manufacturer: String,
    /// Model designation as reported by the machine. Brother machines report a
    /// numeric model code; backends render it into a human-readable string.
    pub model: String,
    /// User-assigned machine name (e.g. `"BETTY"`), if the machine has one.
    pub name: Option<String>,
    /// Firmware version string, e.g. `"1.71"`.
    pub firmware: Option<String>,
    /// Serial number, if reported.
    pub serial: Option<String>,
    /// The address we talked to.
    pub ip: IpAddr,
}

/// What a machine can do: static capabilities and limits.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineCapabilities {
    /// Maximum embroidery area, in millimetres.
    pub emb_width_mm: Option<f64>,
    pub emb_height_mm: Option<f64>,
    /// Number of needles (1 for home machines, more for multi-needle).
    pub needles: Option<u32>,
    /// Largest single design file the machine will accept, in bytes.
    pub max_file_bytes: Option<u64>,
    /// File extensions (lowercase, without dot) the machine can load.
    pub formats: Vec<String>,
}

/// Full identification of a machine: identity + capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MachineInfo {
    pub identity: MachineIdentity,
    pub capabilities: MachineCapabilities,
}

/// Live storage state of a machine's design memory.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct StorageStatus {
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    /// Design files currently held in machine memory. Names are assigned by
    /// the machine itself.
    pub files: Vec<String>,
}

/// A design to be sent to a machine.
#[derive(Debug, Clone)]
pub struct UploadRequest {
    /// Original filename; used for format detection. Some machines (Brother)
    /// ignore it and assign their own name.
    pub filename: String,
    pub data: bytes::Bytes,
}

/// Result of a successful upload.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadReceipt {
    pub bytes_sent: u64,
    /// Name the machine stored the design under, when the protocol lets us
    /// find out (Brother renames uploads, so this is best-effort).
    pub stored_as: Option<String>,
}

/// Progress of an in-flight upload, reported by backends via [`ProgressFn`].
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UploadProgress {
    pub sent_bytes: u64,
    pub total_bytes: u64,
}

/// Callback used by backends to report upload progress.
pub type ProgressFn = std::sync::Arc<dyn Fn(UploadProgress) + Send + Sync>;

/// A machine found during discovery.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveredMachine {
    pub info: MachineInfo,
}
