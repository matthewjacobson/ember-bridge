//! Route handlers for the localhost API.
//!
//! Manufacturer-agnostic by construction: handlers speak only in terms of
//! `crate::machine` traits and models, so new backends appear here for free.
//!
//! | Method | Path              | Purpose                                    |
//! |--------|-------------------|--------------------------------------------|
//! | GET    | /api/health       | Liveness + version (no token)              |
//! | GET    | /api/status       | App status, or machine status with `?ip=`  |
//! | GET    | /api/info?ip=     | Identify a machine                         |
//! | GET    | /api/machines     | Saved machines + last discovery results    |
//! | POST   | /api/machines     | Save a machine `{ip, nickname?}`           |
//! | DELETE | /api/machines/{ip}| Forget a saved machine                     |
//! | POST   | /api/discover     | Sweep the local network (blocks ~5s)       |
//! | POST   | /api/pair         | Browser asks to pair (no token, origin-gated) |
//! | GET    | /api/pair/{id}    | Browser polls its pairing request (no token) |
//! | GET    | /api/pairing      | Pending request, for the desktop UI banner |
//! | POST   | /api/pairing/respond | Approve/deny `{id, approve}` (desktop UI) |
//! | POST   | /api/send         | Enqueue an upload (`?ip=&filename=`, body = design bytes) |
//! | GET    | /api/jobs         | Recent upload jobs, newest first           |
//! | GET    | /api/jobs/{id}    | One job, for progress polling              |
//! | GET    | /api/logs         | App log ring buffer (`?afterSeq=`)         |
//! | GET    | /api/settings     | Current settings (incl. token)             |
//! | PUT    | /api/settings     | Update allowed origins                     |

use crate::config::SavedMachine;
use crate::machine::net::is_local_network_ip;
use crate::server::auth::origin_allowed;
use crate::server::error::ApiError;
use crate::server::pairing::PollOutcome;
use crate::server::state::AppState;
use axum::body::Bytes;
use axum::extract::{Path, Query, State};
use axum::http::{header, HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::Ordering;
use std::sync::Arc;

const API_VERSION: u32 = 1;

pub fn app_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

// ---------------------------------------------------------------------------
// Helpers

/// Parse and vet a target machine address: must parse, and must be on the
/// local network — the bridge refuses to be used as a proxy to the internet.
fn parse_target_ip(raw: &str) -> Result<IpAddr, ApiError> {
    let ip: IpAddr = raw
        .trim()
        .parse()
        .map_err(|_| ApiError::bad_request("invalid_ip", format!("{raw:?} is not an IP address")))?;
    if !is_local_network_ip(ip) {
        return Err(ApiError::bad_request(
            "ip_not_local",
            format!("{ip} is not a private/local network address; refusing to contact it"),
        ));
    }
    Ok(ip)
}

fn required_ip(params: &HashMap<String, String>) -> Result<IpAddr, ApiError> {
    let raw = params
        .get("ip")
        .ok_or_else(|| ApiError::bad_request("missing_ip", "query parameter `ip` is required"))?;
    parse_target_ip(raw)
}

// ---------------------------------------------------------------------------
// Health & status

pub async fn health(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let health = state.server_health.read().await.clone();
    Json(json!({
        "app": "ember-bridge",
        "version": app_version(),
        "apiVersion": API_VERSION,
        "port": health.port,
    }))
}

/// Without `?ip=`: bridge status. With `?ip=`: live status of that machine
/// (storage + file list), matching the Ember-facing API contract.
pub async fn status(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    if params.contains_key("ip") {
        let ip = required_ip(&params)?;
        let (machine, info) = resolve_machine(&state, ip).await?;
        let storage = machine.storage().await?;
        return Ok(Json(json!({ "info": info, "storage": storage })));
    }

    Ok(Json(json!({
        "app": "ember-bridge",
        "version": app_version(),
        "apiVersion": API_VERSION,
        "uptimeSeconds": state.started_at.elapsed().as_secs(),
        "server": *state.server_health.read().await,
        "pendingUploads": state.jobs.pending_count(),
        "savedMachines": state.config.get().await.machines.len(),
        "discoveryRunning": state.discovery_running.load(Ordering::Relaxed),
    })))
}

/// Identify the machine at `?ip=`.
pub async fn info(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, ApiError> {
    let ip = required_ip(&params)?;
    let (_machine, info) = resolve_machine(&state, ip).await?;
    Ok(Json(info))
}

/// Ask the registry which backend owns the device at `ip`.
async fn resolve_machine(
    state: &AppState,
    ip: IpAddr,
) -> Result<
    (
        Arc<dyn crate::machine::EmbroideryMachine>,
        crate::machine::MachineInfo,
    ),
    ApiError,
> {
    match state.registry.identify(ip).await? {
        Some(found) => Ok(found),
        None => Err(ApiError::bad_request(
            "not_a_machine",
            format!("the device at {ip} does not speak any supported embroidery protocol"),
        )),
    }
}

// ---------------------------------------------------------------------------
// Machines: saved list + discovery

pub async fn list_machines(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.get().await;
    let cache = state.discovered.read().await;
    Json(json!({
        "saved": config.machines,
        "discovered": cache.machines,
        "discoveryCompletedAtMs": cache.completed_at_ms,
        "discoveryRunning": state.discovery_running.load(Ordering::Relaxed),
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SaveMachineBody {
    pub ip: String,
    #[serde(default)]
    pub nickname: Option<String>,
    #[serde(default)]
    pub manufacturer: Option<String>,
}

pub async fn save_machine(
    State(state): State<Arc<AppState>>,
    Json(body): Json<SaveMachineBody>,
) -> Result<impl IntoResponse, ApiError> {
    let ip = parse_target_ip(&body.ip)?;
    let nickname = body
        .nickname
        .map(|n| n.trim().to_string())
        .filter(|n| !n.is_empty());
    let config = state
        .config
        .update(|c| {
            c.machines.retain(|m| m.ip != ip);
            c.machines.push(SavedMachine {
                ip,
                nickname: nickname.clone(),
                manufacturer: body.manufacturer.clone(),
            });
        })
        .await
        .map_err(|e| ApiError::internal(format!("could not persist config: {e}")))?;
    state.logs.info(format!("Saved machine {ip}"));
    Ok(Json(json!({ "saved": config.machines })))
}

pub async fn delete_machine(
    State(state): State<Arc<AppState>>,
    Path(ip): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    let ip: IpAddr = ip
        .parse()
        .map_err(|_| ApiError::bad_request("invalid_ip", format!("{ip:?} is not an IP address")))?;
    let mut removed = false;
    let config = state
        .config
        .update(|c| {
            let before = c.machines.len();
            c.machines.retain(|m| m.ip != ip);
            removed = c.machines.len() != before;
        })
        .await
        .map_err(|e| ApiError::internal(format!("could not persist config: {e}")))?;
    if !removed {
        return Err(ApiError::not_found(format!("{ip} is not a saved machine")));
    }
    state.logs.info(format!("Removed saved machine {ip}"));
    Ok(Json(json!({ "saved": config.machines })))
}

/// Sweep the local network. Blocks until the sweep finishes (a few seconds);
/// concurrent sweeps are collapsed into one.
pub async fn discover(State(state): State<Arc<AppState>>) -> Result<impl IntoResponse, ApiError> {
    if state
        .discovery_running
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_err()
    {
        return Err(ApiError::bad_request(
            "discovery_running",
            "a discovery sweep is already in progress",
        ));
    }

    // Reset the flag even if the client disconnects and axum drops this
    // future mid-sweep.
    struct ScanGuard(Arc<AppState>);
    impl Drop for ScanGuard {
        fn drop(&mut self) {
            self.0.discovery_running.store(false, Ordering::SeqCst);
        }
    }
    let _guard = ScanGuard(state.clone());

    state.logs.info("Scanning local network for machines…");
    let machines = state
        .registry
        .discover_all(std::sync::Arc::new(|_done, _total| {}))
        .await;

    state.logs.info(format!(
        "Discovery finished: {} machine(s) found",
        machines.len()
    ));

    let mut cache = state.discovered.write().await;
    cache.machines = machines;
    cache.completed_at_ms = Some(now_ms());

    Ok(Json(json!({
        "discovered": cache.machines,
        "discoveryCompletedAtMs": cache.completed_at_ms,
    })))
}

// ---------------------------------------------------------------------------
// Upload

/// Absolute ceiling on accepted design uploads. Machines advertise their own
/// limit (~3 MB today) which is enforced per-upload by the backend; this is
/// just the transport-level bound.
pub const MAX_UPLOAD_BYTES: usize = 32 * 1024 * 1024;

// ---------------------------------------------------------------------------
// Pairing: browser side (tokenless, origin-gated) + desktop-UI side (token)

/// Extract and vet the caller's `Origin` for the tokenless pairing routes.
/// These are the only handlers where the origin allowlist is enforced
/// server-side: elsewhere CORS merely controls response visibility, but a
/// pairing request must not even be *created* for an unknown origin.
async fn vetted_pairing_origin(
    state: &AppState,
    headers: &HeaderMap,
) -> Result<String, ApiError> {
    let origin = headers
        .get(header::ORIGIN)
        .and_then(|v| v.to_str().ok())
        .ok_or_else(|| {
            ApiError::bad_request(
                "origin_required",
                "pairing is for browsers; non-browser clients read the token from the app's Settings page",
            )
        })?;
    if !origin_allowed(origin, &state.config.get().await.allowed_origins) {
        return Err(ApiError::forbidden(
            "origin_not_allowed",
            format!("{origin} is not on the allowed-origins list"),
        ));
    }
    Ok(origin.to_string())
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairBody {
    /// Display name shown in the approval prompt, e.g. "Ember".
    #[serde(default)]
    pub app_name: Option<String>,
}

pub async fn create_pairing(
    State(state): State<Arc<AppState>>,
    headers: HeaderMap,
    body: Option<Json<PairBody>>,
) -> Result<impl IntoResponse, ApiError> {
    let origin = vetted_pairing_origin(&state, &headers).await?;
    let app_name = body
        .and_then(|b| b.0.app_name)
        .map(|n| n.trim().chars().take(64).collect::<String>())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| "Unnamed app".to_string());

    match state.pairing.begin(origin.clone(), app_name).await {
        Ok(request) => {
            state
                .logs
                .info(format!("{origin} is asking to pair — approve or deny in the app"));
            // Bring the desktop window to the front so the prompt is seen.
            if let Some(notify) = &*state.pairing_notify.lock().unwrap() {
                notify();
            }
            Ok((StatusCode::ACCEPTED, Json(json!({ "request": request }))))
        }
        Err(()) => Err(ApiError::conflict(
            "pairing_in_progress",
            "another pairing request is awaiting a decision; try again shortly",
        )),
    }
}

pub async fn poll_pairing(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<impl IntoResponse, ApiError> {
    let origin = vetted_pairing_origin(&state, &headers).await?;
    match state.pairing.poll(&id, &origin).await {
        PollOutcome::Unknown => Err(ApiError::not_found(
            "no such pairing request (it may have expired — start over)",
        )),
        PollOutcome::WrongOrigin => Err(ApiError::forbidden(
            "pairing_origin_mismatch",
            "this pairing request belongs to a different origin",
        )),
        PollOutcome::Pending => Ok(Json(json!({ "state": "pending" }))),
        PollOutcome::Denied => Ok(Json(json!({ "state": "denied" }))),
        PollOutcome::Approved => {
            // The one moment the token crosses to the browser. The poll()
            // above consumed the request, so this cannot repeat.
            let token = state.config.get().await.api_token;
            state.logs.info(format!("Pairing token released to {origin}"));
            Ok(Json(json!({ "state": "approved", "token": token })))
        }
    }
}

pub async fn pairing_pending(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({ "pending": state.pairing.pending().await }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PairingRespondBody {
    pub id: String,
    pub approve: bool,
}

pub async fn pairing_respond(
    State(state): State<Arc<AppState>>,
    Json(body): Json<PairingRespondBody>,
) -> Result<impl IntoResponse, ApiError> {
    match state.pairing.respond(&body.id, body.approve).await {
        Some(origin) => {
            let verdict = if body.approve { "approved" } else { "denied" };
            state.logs.info(format!("Pairing request from {origin} {verdict}"));
            Ok(Json(json!({ "ok": true })))
        }
        None => Err(ApiError::not_found(
            "no matching pending pairing request (it may have expired)",
        )),
    }
}

// ---------------------------------------------------------------------------
// Uploads

pub async fn send(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
    body: Bytes,
) -> Result<impl IntoResponse, ApiError> {
    let ip = required_ip(&params)?;
    let filename = params
        .get("filename")
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            ApiError::bad_request("missing_filename", "query parameter `filename` is required")
        })?;

    if body.is_empty() {
        return Err(ApiError::bad_request(
            "empty_body",
            "request body must contain the design file bytes",
        ));
    }

    let job = state.jobs.enqueue(ip, filename, body);
    Ok((StatusCode::ACCEPTED, Json(json!({ "job": job }))))
}

pub async fn list_jobs(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    Json(json!({ "jobs": state.jobs.list() }))
}

pub async fn get_job(
    State(state): State<Arc<AppState>>,
    Path(id): Path<String>,
) -> Result<impl IntoResponse, ApiError> {
    state
        .jobs
        .get(&id)
        .map(|job| Json(json!({ "job": job })))
        .ok_or_else(|| ApiError::not_found(format!("no job with id {id:?}")))
}

// ---------------------------------------------------------------------------
// Logs & settings

pub async fn logs(
    State(state): State<Arc<AppState>>,
    Query(params): Query<HashMap<String, String>>,
) -> impl IntoResponse {
    let after = params
        .get("afterSeq")
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(0);
    Json(json!({
        "entries": state.logs.since(after),
        "lastSeq": state.logs.last_seq(),
    }))
}

pub async fn get_settings(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let config = state.config.get().await;
    Json(json!({
        "apiToken": config.api_token,
        "allowedOrigins": config.allowed_origins,
        "port": state.server_health.read().await.port,
    }))
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateSettingsBody {
    pub allowed_origins: Vec<String>,
}

pub async fn update_settings(
    State(state): State<Arc<AppState>>,
    Json(body): Json<UpdateSettingsBody>,
) -> Result<impl IntoResponse, ApiError> {
    let origins: Vec<String> = body
        .allowed_origins
        .into_iter()
        .map(|o| o.trim().trim_end_matches('/').to_string())
        .filter(|o| !o.is_empty())
        .collect();
    for origin in &origins {
        let valid = origin == "*"
            || origin.starts_with("http://")
            || origin.starts_with("https://")
            || origin.starts_with("tauri://");
        if !valid {
            return Err(ApiError::bad_request(
                "invalid_origin",
                format!("{origin:?} is not a valid web origin (expected e.g. https://ember.example)"),
            ));
        }
    }
    state
        .config
        .update(|c| c.allowed_origins = origins)
        .await
        .map_err(|e| ApiError::internal(format!("could not persist config: {e}")))?;
    state.logs.info("Updated allowed origins");
    Ok(Json(json!({ "ok": true })))
}

fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0)
}
