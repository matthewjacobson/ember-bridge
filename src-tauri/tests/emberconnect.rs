//! Integration tests for the EmberConnect backend against a mock dongle.
//!
//! The mock is an axum server on a random localhost port speaking the
//! dongle's JSON API (as defined by the firmware's http_api.c). These tests
//! exercise the real reqwest client — URL construction, query encoding,
//! streaming upload with Content-Length, pairing/token auth, and
//! error-envelope mapping.

use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::routing::{get, post};
use axum::{Json, Router};
use ember_bridge_lib::emberconnect::{EmberConnectClient, TokenStore};
use ember_bridge_lib::machine::{EmbroideryMachine, MachineError, UploadRequest};
use serde_json::json;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct MockDongle {
    /// (filename, body bytes) of received uploads.
    uploads: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    /// Simulate a nearly-full card.
    free_bytes: u64,
    /// User-chosen machine name (firmware 0.5.0+); "" = never named.
    device_name: String,
    /// Firmware 0.4.0+ behaviour: everything but health/pair wants a token.
    require_auth: bool,
    /// Whether POST /api/pair currently succeeds.
    pairing_open: bool,
    /// Bearer tokens the mock accepts.
    valid_tokens: Arc<Mutex<Vec<String>>>,
    /// How many times a client paired.
    pair_calls: Arc<Mutex<u32>>,
}

impl MockDongle {
    fn authorized(&self, headers: &HeaderMap) -> bool {
        if !self.require_auth {
            return true;
        }
        let Some(token) = headers
            .get("authorization")
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.strip_prefix("Bearer "))
        else {
            return false;
        };
        self.valid_tokens.lock().unwrap().iter().any(|t| t == token)
    }
}

fn unauthorized() -> (StatusCode, Json<serde_json::Value>) {
    (
        StatusCode::UNAUTHORIZED,
        Json(json!({"error": {"code": "unauthorized", "message": "pair first"}})),
    )
}

async fn start_mock(dongle: MockDongle) -> (u16, MockDongle) {
    let free = dongle.free_bytes;
    let device_name = dongle.device_name.clone();
    let app = Router::new()
        .route(
            "/api/health",
            get(move || async move {
                Json(json!({
                    "ok": true, "name": "EmberConnect", "deviceName": device_name,
                    "version": "0.4.0", "serial": "A1B2C3D4E5F6"
                }))
            }),
        )
        .route(
            "/api/info",
            get(
                move |State(state): State<MockDongle>, headers: HeaderMap| async move {
                    if !state.authorized(&headers) {
                        return unauthorized();
                    }
                    (
                        StatusCode::OK,
                        Json(json!({
                            "name": "EmberConnect", "version": "0.4.0",
                            "serial": "A1B2C3D4E5F6", "ip": "127.0.0.1",
                            "storage": {"totalBytes": 1_000_000u64, "freeBytes": free}
                        })),
                    )
                },
            ),
        )
        .route(
            "/api/files",
            get(
                |State(state): State<MockDongle>, headers: HeaderMap| async move {
                    if !state.authorized(&headers) {
                        return unauthorized();
                    }
                    (
                        StatusCode::OK,
                        Json(json!({"files": [
                            {"name": "existing.pes", "size": 1024}
                        ]})),
                    )
                },
            ),
        )
        .route(
            "/api/pair",
            post(|State(state): State<MockDongle>| async move {
                *state.pair_calls.lock().unwrap() += 1;
                if !state.pairing_open {
                    return (
                        StatusCode::FORBIDDEN,
                        Json(json!({"error": {
                            "code": "pairing_closed",
                            "message": "unplug and replug the dongle (or tap its button), \
                                        then pair within 5 minutes"
                        }})),
                    );
                }
                let token = format!(
                    "mock-token-{}",
                    state.valid_tokens.lock().unwrap().len() + 1
                );
                state.valid_tokens.lock().unwrap().push(token.clone());
                (
                    StatusCode::CREATED,
                    Json(json!({"ok": true, "token": token, "serial": "A1B2C3D4E5F6"})),
                )
            }),
        )
        .route(
            "/api/upload",
            post(
                |State(state): State<MockDongle>,
                 Query(params): Query<HashMap<String, String>>,
                 headers: HeaderMap,
                 body: axum::body::Bytes| async move {
                    if !state.authorized(&headers) {
                        return unauthorized();
                    }
                    let Some(name) = params.get("filename").cloned() else {
                        return (
                            StatusCode::BAD_REQUEST,
                            Json(json!({"error": {"code": "filename_required", "message": ""}})),
                        );
                    };
                    if body.len() as u64 > state.free_bytes {
                        return (
                            StatusCode::INSUFFICIENT_STORAGE,
                            Json(json!({"error": {
                                "code": "insufficient_storage",
                                "message": "not enough free space on the card"
                            }})),
                        );
                    }
                    let size = body.len();
                    state.uploads.lock().unwrap().push((name.clone(), body.to_vec()));
                    (
                        StatusCode::CREATED,
                        Json(json!({"ok": true, "file": {"name": name, "size": size}})),
                    )
                },
            ),
        )
        .with_state(dongle.clone());

    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let port = listener.local_addr().unwrap().port();
    tokio::spawn(async move {
        axum::serve(listener, app).await.unwrap();
    });
    (port, dongle)
}

fn localhost() -> IpAddr {
    "127.0.0.1".parse().unwrap()
}

/// A fresh, empty on-disk token store in a unique temp directory.
fn empty_store() -> Arc<TokenStore> {
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "emberconnect-test-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&dir).unwrap();
    Arc::new(TokenStore::load(&dir))
}

fn client(port: u16, store: Arc<TokenStore>) -> EmberConnectClient {
    EmberConnectClient::with_port(localhost(), port, store)
}

#[tokio::test]
async fn probe_identifies_a_dongle_and_builds_identity() {
    let (port, _dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        ..Default::default()
    })
    .await;

    let info = client(port, empty_store()).info().await.unwrap();
    assert_eq!(info.identity.manufacturer, "emberconnect");
    // Never named → fall back to the setup-hotspot style name.
    assert_eq!(info.identity.name.as_deref(), Some("EmberConnect-E5F6"));
    assert_eq!(info.identity.serial.as_deref(), Some("A1B2C3D4E5F6"));
    assert_eq!(info.identity.firmware.as_deref(), Some("0.4.0"));
    assert!(info.capabilities.formats.iter().any(|f| f == "pes"));
}

#[tokio::test]
async fn user_chosen_device_name_wins_over_serial_fallback() {
    let (port, _dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        device_name: "Sewing room Brother".to_string(),
        ..Default::default()
    })
    .await;

    let info = client(port, empty_store()).info().await.unwrap();
    assert_eq!(info.identity.name.as_deref(), Some("Sewing room Brother"));
}

#[tokio::test]
async fn storage_merges_stats_and_file_list() {
    let (port, _dongle) = start_mock(MockDongle {
        free_bytes: 400_000,
        ..Default::default()
    })
    .await;

    let storage = client(port, empty_store()).storage().await.unwrap();
    assert_eq!(storage.total_bytes, 1_000_000);
    assert_eq!(storage.free_bytes, 400_000);
    assert_eq!(storage.used_bytes, 600_000);
    assert_eq!(storage.files, vec!["existing.pes".to_string()]);
}

#[tokio::test]
async fn upload_streams_bytes_and_reports_progress() {
    let (port, dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        ..Default::default()
    })
    .await;

    let payload = vec![0xEBu8; 100_000];
    let progress_points = Arc::new(Mutex::new(Vec::new()));
    let seen = progress_points.clone();

    let receipt = client(port, empty_store())
        .upload(
            UploadRequest {
                filename: "rose bud.pes".to_string(),
                data: payload.clone().into(),
            },
            Arc::new(move |p| seen.lock().unwrap().push(p.sent_bytes)),
        )
        .await
        .unwrap();

    assert_eq!(receipt.bytes_sent, 100_000);
    assert_eq!(receipt.stored_as.as_deref(), Some("rose bud.pes"));

    // The mock received exactly our bytes under the (URL-decoded) name.
    let uploads = dongle.uploads.lock().unwrap();
    assert_eq!(uploads.len(), 1);
    assert_eq!(uploads[0].0, "rose bud.pes");
    assert_eq!(uploads[0].1, payload);

    // Progress must start at 0, end at the full size, and be monotonic.
    let points = progress_points.lock().unwrap();
    assert_eq!(*points.first().unwrap(), 0);
    assert_eq!(*points.last().unwrap(), 100_000);
    assert!(points.windows(2).all(|w| w[0] <= w[1]));
}

#[tokio::test]
async fn full_card_maps_to_insufficient_storage() {
    let (port, dongle) = start_mock(MockDongle {
        free_bytes: 10,
        ..Default::default()
    })
    .await;

    let err = client(port, empty_store())
        .upload(
            UploadRequest {
                filename: "big.pes".to_string(),
                data: vec![0u8; 1000].into(),
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap_err();

    assert!(matches!(err, MachineError::InsufficientStorage { size: 1000, .. }));
    // Rejected before any bytes were streamed.
    assert!(dongle.uploads.lock().unwrap().is_empty());
}

#[tokio::test]
async fn unreachable_dongle_is_a_transport_error() {
    // Nothing listens on this port.
    let err = client(1, empty_store()).info().await.unwrap_err();
    assert!(matches!(
        err,
        MachineError::Unreachable(_) | MachineError::Timeout
    ));
}

#[tokio::test]
async fn pairs_transparently_on_401_and_persists_the_token() {
    let (port, dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        require_auth: true,
        pairing_open: true,
        ..Default::default()
    })
    .await;

    let store = empty_store();
    let c = client(port, store.clone());

    // An end-to-end upload against an auth-requiring dongle we've never met:
    // the client must pair mid-flight and succeed.
    let receipt = c
        .upload(
            UploadRequest {
                filename: "rose.pes".to_string(),
                data: vec![1u8; 1000].into(),
            },
            Arc::new(|_| {}),
        )
        .await
        .unwrap();
    assert_eq!(receipt.bytes_sent, 1000);

    // The token was persisted under the dongle's serial for next time.
    assert!(store.get("A1B2C3D4E5F6").is_some());
    // And pairing happened exactly once, not per request.
    assert_eq!(*dongle.pair_calls.lock().unwrap(), 1);
}

#[tokio::test]
async fn stored_token_is_reused_without_pairing() {
    let (port, dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        require_auth: true,
        pairing_open: true, // open, but must not be needed
        ..Default::default()
    })
    .await;
    dongle
        .valid_tokens
        .lock()
        .unwrap()
        .push("previously-issued".to_string());

    let store = empty_store();
    store.set("A1B2C3D4E5F6", "previously-issued");

    client(port, store).storage().await.unwrap();
    assert_eq!(*dongle.pair_calls.lock().unwrap(), 0);
}

#[tokio::test]
async fn closed_pairing_window_is_an_actionable_error() {
    let (port, _dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        require_auth: true,
        pairing_open: false,
        ..Default::default()
    })
    .await;

    let err = client(port, empty_store()).storage().await.unwrap_err();
    match err {
        MachineError::PairingRequired { hint } => {
            assert!(hint.contains("replug"), "hint should tell the user what to do: {hint}");
        }
        other => panic!("expected PairingRequired, got {other:?}"),
    }
}

#[tokio::test]
async fn revoked_token_triggers_repairing() {
    let (port, dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        require_auth: true,
        pairing_open: true,
        ..Default::default()
    })
    .await;

    // We hold a token the dongle no longer accepts (factory reset).
    let store = empty_store();
    store.set("A1B2C3D4E5F6", "stale-after-factory-reset");

    client(port, store.clone()).storage().await.unwrap();
    assert_eq!(*dongle.pair_calls.lock().unwrap(), 1);
    // The stale token was replaced with the fresh one.
    assert_eq!(store.get("A1B2C3D4E5F6").as_deref(), Some("mock-token-1"));
}
