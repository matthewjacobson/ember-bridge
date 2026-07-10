//! Integration tests for the localhost API: drives the real axum router
//! (auth, CORS, validation, error shapes) without any Tauri or network
//! machinery. Machine-reaching paths are covered up to the "is this request
//! even acceptable" boundary; the wire protocol itself is unit-tested in
//! `brother::protocol`.

use axum::body::Body;
use axum::http::{header, Method, Request, StatusCode};
use axum::Router;
use ember_bridge_lib::config::ConfigStore;
use ember_bridge_lib::server::state::AppState;
use ember_bridge_lib::server::{build_router, PORT};
use http_body_util::BodyExt;
use std::sync::Arc;
use tower::ServiceExt;

struct TestApp {
    router: Router,
    token: String,
    _dir: tempdir::TempDir,
}

/// Minimal temp-dir helper so we don't pull in a crate for one test file.
mod tempdir {
    use std::path::PathBuf;

    pub struct TempDir(PathBuf);

    impl TempDir {
        pub fn new() -> Self {
            let dir = std::env::temp_dir().join(format!(
                "ember-bridge-test-{}-{:?}",
                std::process::id(),
                std::time::Instant::now()
            ));
            std::fs::create_dir_all(&dir).unwrap();
            Self(dir)
        }

        pub fn path(&self) -> PathBuf {
            self.0.clone()
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
}

async fn test_app() -> TestApp {
    let dir = tempdir::TempDir::new();
    let config = ConfigStore::load_or_create(dir.path()).unwrap();
    let state = Arc::new(AppState::new(config, PORT));
    let token = state.config.get().await.api_token.clone();
    TestApp {
        router: build_router(state),
        token,
        _dir: dir,
    }
}

async fn body_json(response: axum::response::Response) -> serde_json::Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

#[tokio::test]
async fn health_needs_no_token() {
    let app = test_app().await;
    let response = app
        .router
        .oneshot(Request::get("/api/health").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["app"], "ember-bridge");
}

#[tokio::test]
async fn everything_else_requires_the_token() {
    let app = test_app().await;
    for path in ["/api/status", "/api/machines", "/api/jobs", "/api/logs", "/api/settings"] {
        let response = app
            .router
            .clone()
            .oneshot(Request::get(path).body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED, "path {path}");
        let json = body_json(response).await;
        assert_eq!(json["error"]["code"], "unauthorized", "path {path}");
    }
}

#[tokio::test]
async fn wrong_token_is_rejected() {
    let app = test_app().await;
    let response = app
        .router
        .oneshot(
            Request::get("/api/status")
                .header(header::AUTHORIZATION, "Bearer not-the-token")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

#[tokio::test]
async fn bridge_status_with_valid_token() {
    let app = test_app().await;
    let response = app
        .router
        .oneshot(
            Request::get("/api/status")
                .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["apiVersion"], 1);
    assert_eq!(json["pendingUploads"], 0);
}

#[tokio::test]
async fn x_ember_token_header_also_works() {
    let app = test_app().await;
    let response = app
        .router
        .oneshot(
            Request::get("/api/status")
                .header("x-ember-token", app.token.clone())
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
}

#[tokio::test]
async fn public_and_loopback_targets_are_refused() {
    let app = test_app().await;
    for (ip, expected_code) in [("8.8.8.8", "ip_not_local"), ("127.0.0.1", "ip_not_local"), ("nonsense", "invalid_ip")] {
        let response = app
            .router
            .clone()
            .oneshot(
                Request::get(format!("/api/info?ip={ip}"))
                    .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "ip {ip}");
        let json = body_json(response).await;
        assert_eq!(json["error"]["code"], expected_code, "ip {ip}");
    }
}

#[tokio::test]
async fn send_validates_before_queueing() {
    let app = test_app().await;

    // Missing filename.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/send?ip=192.168.1.120")
                .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
                .body(Body::from("design-bytes"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(response).await["error"]["code"], "missing_filename");

    // Empty body.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/send?ip=192.168.1.120&filename=rose.pes")
                .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(response).await["error"]["code"], "empty_body");

    // Valid request is accepted and queued (no worker running in tests, so
    // it stays queued — which is exactly what we assert).
    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/send?ip=192.168.1.120&filename=rose.pes")
                .header(header::AUTHORIZATION, format!("Bearer {}", app.token))
                .body(Body::from("#PES0001fake"))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::ACCEPTED);
    let json = body_json(response).await;
    assert_eq!(json["job"]["state"], "queued");
    assert_eq!(json["job"]["filename"], "rose.pes");
    assert_eq!(json["job"]["totalBytes"], 12);
}

#[tokio::test]
async fn machines_can_be_saved_and_deleted() {
    let app = test_app().await;
    let auth = format!("Bearer {}", app.token);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::post("/api/machines")
                .header(header::AUTHORIZATION, &auth)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    r#"{"ip":"192.168.1.120","nickname":"Sewing room"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);
    let json = body_json(response).await;
    assert_eq!(json["saved"][0]["nickname"], "Sewing room");

    let response = app
        .router
        .clone()
        .oneshot(
            Request::delete("/api/machines/192.168.1.120")
                .header(header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    // Deleting again: not found.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::delete("/api/machines/192.168.1.120")
                .header(header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn cors_preflight_for_allowed_and_denied_origins() {
    let app = test_app().await;

    // Denied: unknown origin gets no CORS headers.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/send")
                .header(header::ORIGIN, "https://evil.example")
                .header("access-control-request-method", "POST")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert!(response
        .headers()
        .get("access-control-allow-origin")
        .is_none());

    // Allowed: localhost origin (Ember dev server / our own UI).
    let response = app
        .router
        .clone()
        .oneshot(
            Request::builder()
                .method(Method::OPTIONS)
                .uri("/api/send")
                .header(header::ORIGIN, "http://localhost:5173")
                .header("access-control-request-method", "POST")
                .header("access-control-request-private-network", "true")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-origin")
            .unwrap(),
        "http://localhost:5173"
    );
    assert_eq!(
        response
            .headers()
            .get("access-control-allow-private-network")
            .unwrap(),
        "true"
    );
}

#[tokio::test]
async fn settings_roundtrip_updates_allowed_origins() {
    let app = test_app().await;
    let auth = format!("Bearer {}", app.token);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::put("/api/settings")
                .header(header::AUTHORIZATION, &auth)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"allowedOrigins":["https://ember.example/"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

    let response = app
        .router
        .clone()
        .oneshot(
            Request::get("/api/settings")
                .header(header::AUTHORIZATION, &auth)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    let json = body_json(response).await;
    // Trailing slash is normalized away.
    assert_eq!(json["allowedOrigins"][0], "https://ember.example");
    assert_eq!(json["apiToken"], app.token);

    // Garbage origins are refused.
    let response = app
        .router
        .clone()
        .oneshot(
            Request::put("/api/settings")
                .header(header::AUTHORIZATION, &auth)
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(r#"{"allowedOrigins":["ember.example"]}"#))
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
    assert_eq!(body_json(response).await["error"]["code"], "invalid_origin");
}
