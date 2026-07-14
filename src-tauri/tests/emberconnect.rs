//! Integration tests for the EmberConnect backend against a mock dongle.
//!
//! The mock is an axum server on a random localhost port speaking the
//! dongle's JSON API (as defined by the firmware's http_api.c). These tests
//! exercise the real reqwest client — URL construction, query encoding,
//! streaming upload with Content-Length, and error-envelope mapping.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::routing::{get, post};
use axum::{Json, Router};
use ember_bridge_lib::emberconnect::EmberConnectClient;
use ember_bridge_lib::machine::{EmbroideryMachine, MachineError, UploadRequest};
use serde_json::json;
use std::collections::HashMap;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};

#[derive(Clone, Default)]
struct MockDongle {
    /// (filename, body bytes) of received uploads.
    uploads: Arc<Mutex<Vec<(String, Vec<u8>)>>>,
    /// Simulate a nearly-full card.
    free_bytes: u64,
}

async fn start_mock(dongle: MockDongle) -> (u16, MockDongle) {
    let free = dongle.free_bytes;
    let app = Router::new()
        .route(
            "/api/health",
            get(|| async {
                Json(json!({
                    "ok": true, "name": "EmberConnect",
                    "version": "0.1.0", "serial": "A1B2C3D4E5F6"
                }))
            }),
        )
        .route(
            "/api/info",
            get(move || async move {
                Json(json!({
                    "name": "EmberConnect", "version": "0.1.0",
                    "serial": "A1B2C3D4E5F6", "ip": "127.0.0.1",
                    "storage": {"totalBytes": 1_000_000u64, "freeBytes": free}
                }))
            }),
        )
        .route(
            "/api/files",
            get(|| async {
                Json(json!({"files": [
                    {"name": "existing.pes", "size": 1024}
                ]}))
            }),
        )
        .route(
            "/api/upload",
            post(
                |State(state): State<MockDongle>,
                 Query(params): Query<HashMap<String, String>>,
                 body: axum::body::Bytes| async move {
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

#[tokio::test]
async fn probe_identifies_a_dongle_and_builds_identity() {
    let (port, _dongle) = start_mock(MockDongle {
        free_bytes: 500_000,
        ..Default::default()
    })
    .await;

    let client = EmberConnectClient::with_port(localhost(), port);
    let info = client.info().await.unwrap();
    assert_eq!(info.identity.manufacturer, "emberconnect");
    assert_eq!(info.identity.name.as_deref(), Some("EmberConnect-E5F6"));
    assert_eq!(info.identity.serial.as_deref(), Some("A1B2C3D4E5F6"));
    assert_eq!(info.identity.firmware.as_deref(), Some("0.1.0"));
    assert!(info.capabilities.formats.iter().any(|f| f == "pes"));
}

#[tokio::test]
async fn storage_merges_stats_and_file_list() {
    let (port, _dongle) = start_mock(MockDongle {
        free_bytes: 400_000,
        ..Default::default()
    })
    .await;

    let client = EmberConnectClient::with_port(localhost(), port);
    let storage = client.storage().await.unwrap();
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

    let client = EmberConnectClient::with_port(localhost(), port);
    let payload = vec![0xEBu8; 100_000];
    let progress_points = Arc::new(Mutex::new(Vec::new()));
    let seen = progress_points.clone();

    let receipt = client
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

    let client = EmberConnectClient::with_port(localhost(), port);
    let err = client
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
    let client = EmberConnectClient::with_port(localhost(), 1);
    let err = client.info().await.unwrap_err();
    assert!(matches!(
        err,
        MachineError::Unreachable(_) | MachineError::Timeout
    ));
}
