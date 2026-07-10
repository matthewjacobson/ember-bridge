//! Serde models for the Brother wire formats.
//!
//! `/info` is JSON; the sewing endpoint answers with a small XML document
//! (parsed in [`crate::brother::protocol`] into [`SewingResponse`]).

use serde::Deserialize;
use std::collections::HashMap;

/// Response of `GET /info`.
///
/// Everything is optional except what we genuinely require, because firmware
/// versions differ in which fields they include and identification should
/// degrade gracefully rather than fail on a missing key.
#[derive(Debug, Clone, Deserialize)]
pub struct BrotherInfo {
    /// Numeric model code (e.g. 56 for the Innov-is BP-series).
    pub model: Option<i64>,
    #[serde(rename = "type")]
    pub machine_type: Option<i64>,
    pub oem: Option<i64>,
    pub version: Option<String>,
    #[serde(rename = "machine-id")]
    pub machine_id: Option<String>,
    pub serial: Option<String>,
    /// User-assigned machine name, e.g. "BETTY".
    pub name: Option<String>,
    /// Available protocol APIs. The design-transfer protocol is `"pedxml"`;
    /// its presence is how we recognize a Brother embroidery machine.
    #[serde(default)]
    pub apis: HashMap<String, BrotherApi>,
    #[serde(default)]
    pub features: BrotherFeatures,
}

#[derive(Debug, Clone, Deserialize)]
pub struct BrotherApi {
    #[serde(default)]
    pub version: i64,
}

/// `features` object of `/info`. Dimensions are in 0.1 mm units
/// (1600 = 160 mm); `postsize` is the maximum upload size in bytes.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct BrotherFeatures {
    pub embwidth: Option<u64>,
    pub embheight: Option<u64>,
    pub needles: Option<u32>,
    pub postsize: Option<u64>,
}

impl BrotherInfo {
    /// Does this device speak the design-transfer protocol we implement?
    pub fn supports_pedxml(&self) -> bool {
        self.apis.contains_key("pedxml")
    }
}

/// Parsed form of the XML returned by `POST /sewing/sewing.cgi`.
///
/// The same response shape is used for the status/handshake call
/// (`req_appstate=2`); it carries the machine's error code, memory usage and
/// current file list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SewingResponse {
    /// 0 means OK; anything else is a machine-side rejection.
    pub error_code: i64,
    pub session_id: Option<String>,
    /// Upload endpoint advertised by the machine (observed:
    /// `/sewing/dataupl.cgi`; the official client still posts to
    /// `sewing.cgi`, and so do we).
    pub upload_path: Option<String>,
    /// Total design memory, bytes.
    pub upload_size: Option<u64>,
    /// Free design memory, bytes.
    pub upload_freesize: Option<u64>,
    /// Files currently in machine memory (names assigned by the machine).
    pub files: Vec<String>,
}
