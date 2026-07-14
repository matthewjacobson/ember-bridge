//! Desktop (USB) setup for EmberConnect dongles.
//!
//! When a dongle is plugged into *this* computer it exposes a CDC serial
//! port next to its flash-drive volume; over it we can scan WiFi, try
//! credentials live (the dongle only commits them once a join succeeds),
//! name the machine, pre-pair this Bridge, and push firmware — all before
//! the dongle ever meets the embroidery machine.
//!
//! Unlike the rest of the app, this surface is deliberately *not* on the
//! localhost REST API: provisioning handles WiFi passwords and talks to
//! local hardware, neither of which paired browser origins have any
//! business reaching. The React UI calls these Tauri commands directly.
//!
//! The wire protocol lives in the firmware repo (EmberConnect,
//! `firmware/main/usb_setup.h`); [`link`] implements the transport.

pub mod link;

use crate::server::state::AppState;
use link::{DongleLink, DongleSummary};
use serde::Serialize;
use serde_json::{json, Value};
use std::sync::Arc;
use std::time::Duration;
use tauri::Emitter;

/// The dongle resolves a provisioning trial in ≤30 s; leave slack on top.
const PROVISION_TIMEOUT: Duration = Duration::from_secs(50);
const COMMAND_TIMEOUT: Duration = Duration::from_secs(10);
/// A scan from a *connected* dongle dwells gently per channel to protect
/// the live connection and can far outlast an idle-radio scan.
const SCAN_TIMEOUT: Duration = Duration::from_secs(25);
/// Between update protocol lines — flash writes make the dongle chatty
/// enough (progress every 64 KiB) that longer silence means it died.
const UPDATE_QUIET_TIMEOUT: Duration = Duration::from_secs(30);

/// One serial conversation at a time, app-wide. The dongle's worker is
/// single-threaded and the port is exclusive; concurrent commands (React
/// re-mounts, an eager user) must queue here instead of failing to open
/// the port or interleaving traffic.
static SESSION: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn session_lock() -> std::sync::MutexGuard<'static, ()> {
    SESSION.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupError {
    pub code: String,
    pub message: String,
}

impl SetupError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

pub type SetupResult<T> = Result<T, SetupError>;

/// What the wizard needs after a successful provision.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProvisionOutcome {
    pub ssid: String,
    pub ip: String,
    pub serial: Option<String>,
    /// Whether this Bridge also minted + stored a LAN API token, so the
    /// dongle shows up ready-to-use once it's on the machine.
    pub paired: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateProgress {
    pub written: u64,
    pub total: u64,
}

/// Commands run blocking serial I/O; keep them off the async runtime.
async fn blocking<T: Send + 'static>(
    f: impl FnOnce() -> SetupResult<T> + Send + 'static,
) -> SetupResult<T> {
    tauri::async_runtime::spawn_blocking(f)
        .await
        .map_err(|e| SetupError::new("task_failed", e.to_string()))?
}

#[tauri::command]
pub async fn dongle_list() -> SetupResult<Vec<DongleSummary>> {
    blocking(|| Ok(link::list())).await
}

#[tauri::command]
pub async fn dongle_info(port: String) -> SetupResult<Value> {
    blocking(move || {
        let _session = session_lock();
        DongleLink::open(&port)?.request("info", json!({}), COMMAND_TIMEOUT, |_| {})
    })
    .await
}

#[tauri::command]
pub async fn dongle_scan(port: String) -> SetupResult<Value> {
    blocking(move || {
        let _session = session_lock();
        DongleLink::open(&port)?.request("scan", json!({}), SCAN_TIMEOUT, |_| {})
    })
    .await
}

/// Provision over USB: try the credentials live, and once the dongle is on
/// the network, pair this Bridge with it so the machine is usable the
/// moment it's plugged in. A wrong password comes back as the error code
/// `wrong_password` — the wizard turns that into an inline retry.
#[tauri::command]
pub async fn dongle_provision(
    state: tauri::State<'_, Arc<AppState>>,
    port: String,
    ssid: String,
    password: String,
    name: String,
) -> SetupResult<ProvisionOutcome> {
    let tokens = state.dongle_tokens.clone();
    blocking(move || {
        let _session = session_lock();
        let mut link = DongleLink::open(&port)?;
        let response = link.request(
            "provision",
            json!({ "ssid": ssid, "password": password, "name": name }),
            PROVISION_TIMEOUT,
            |_| {},
        )?;

        let ip = response
            .get("ip")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();

        // Pairing failure is not a provisioning failure: the dongle is on
        // the network either way, and LAN pairing (power-on window) can
        // still happen later.
        let mut serial = None;
        let mut paired = false;
        match link.request("pair", json!({ "name": "Ember Bridge" }), COMMAND_TIMEOUT, |_| {}) {
            Ok(pair) => {
                serial = pair
                    .get("serial")
                    .and_then(Value::as_str)
                    .map(str::to_string);
                if let (Some(s), Some(token)) =
                    (serial.as_deref(), pair.get("token").and_then(Value::as_str))
                {
                    tokens.set(s, token);
                    paired = true;
                }
            }
            Err(e) => tracing::warn!("USB pairing after provision failed: {}", e.message),
        }

        Ok(ProvisionOutcome {
            ssid,
            ip,
            serial,
            paired,
        })
    })
    .await
}

/// Push a signed firmware image from a local file. Progress reaches the UI
/// as `dongle-update-progress` events; the dongle verifies the signature
/// and reboots itself on success (the port will vanish — that's the "done").
#[tauri::command]
pub async fn dongle_update_firmware(
    app: tauri::AppHandle,
    port: String,
    image_path: String,
) -> SetupResult<Value> {
    blocking(move || {
        let _session = session_lock();
        let image = std::fs::read(&image_path)
            .map_err(|e| SetupError::new("image_unreadable", format!("{image_path}: {e}")))?;
        let total = image.len() as u64;

        let mut link = DongleLink::open(&port)?;
        link.request(
            "update",
            json!({ "size": total }),
            COMMAND_TIMEOUT,
            |_| {},
        )?;

        // The dongle said "ready": stream the image, then collect progress
        // events until the final ok/error response line.
        link.write_raw(&image)?;
        loop {
            let Some(line) = link.next_line(UPDATE_QUIET_TIMEOUT)? else {
                return Err(SetupError::new(
                    "dongle_timeout",
                    "the dongle went quiet mid-update",
                ));
            };
            if line.get("event").and_then(Value::as_str) == Some("update") {
                let _ = app.emit(
                    "dongle-update-progress",
                    UpdateProgress {
                        written: line.get("written").and_then(Value::as_u64).unwrap_or(0),
                        total: line.get("total").and_then(Value::as_u64).unwrap_or(total),
                    },
                );
                continue;
            }
            if line.get("ok").and_then(Value::as_bool) == Some(true) {
                return Ok(line);
            }
            if line.get("ok").is_some() {
                let code = line
                    .pointer("/error/code")
                    .and_then(Value::as_str)
                    .unwrap_or("update_failed");
                let message = line
                    .pointer("/error/message")
                    .and_then(Value::as_str)
                    .unwrap_or("the dongle rejected the image");
                return Err(SetupError::new(code, message));
            }
        }
    })
    .await
}
