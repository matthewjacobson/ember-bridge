//! The localhost HTTP API that Ember (and the app's own UI) talks to.
//!
//! Binds strictly to `127.0.0.1` — never to `0.0.0.0` or an interface
//! address — so nothing off-machine can reach it. Browser pages are kept out
//! by token auth + a CORS allowlist (see [`auth`]).

pub mod auth;
pub mod error;
pub mod jobs;
pub mod routes;
pub mod state;

use axum::extract::DefaultBodyLimit;
use axum::middleware;
use axum::routing::{delete, get, post, put};
use axum::Router;
use state::AppState;
use std::net::{Ipv4Addr, SocketAddr};
use std::sync::Arc;

/// Fixed port of the bridge API. Registered nowhere official; chosen to be
/// unlikely to collide and easy for Ember to hardcode.
pub const PORT: u16 = 17831;

/// Build the axum application. Public for integration tests.
pub fn build_router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/api/health", get(routes::health))
        .route("/api/status", get(routes::status))
        .route("/api/info", get(routes::info))
        .route("/api/machines", get(routes::list_machines))
        .route("/api/machines", post(routes::save_machine))
        .route("/api/machines/{ip}", delete(routes::delete_machine))
        .route("/api/discover", post(routes::discover))
        .route("/api/send", post(routes::send))
        .route("/api/jobs", get(routes::list_jobs))
        .route("/api/jobs/{id}", get(routes::get_job))
        .route("/api/logs", get(routes::logs))
        .route("/api/settings", get(routes::get_settings))
        .route("/api/settings", put(routes::update_settings))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth::require_token,
        ))
        // CORS wraps auth so preflights (which carry no token) are answered.
        .layer(middleware::from_fn_with_state(state.clone(), auth::cors))
        .layer(DefaultBodyLimit::max(routes::MAX_UPLOAD_BYTES))
        .with_state(state)
}

/// Bind and serve forever. Records success/failure in
/// [`state::ServerHealth`] so the UI can tell the user about port conflicts.
pub async fn serve(state: Arc<AppState>) {
    // Loopback only. This is a security property, not a default.
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, PORT));

    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(listener) => listener,
        Err(e) => {
            let message = format!("could not bind {addr}: {e}");
            state.logs.error(format!(
                "Local API failed to start ({message}). Is another copy of Ember Bridge running?"
            ));
            let mut health = state.server_health.write().await;
            health.running = false;
            health.error = Some(message);
            return;
        }
    };

    {
        let mut health = state.server_health.write().await;
        health.running = true;
        health.error = None;
    }
    state
        .logs
        .info(format!("Local API listening on http://{addr}"));

    let router = build_router(state.clone());
    if let Err(e) = axum::serve(listener, router).await {
        state.logs.error(format!("Local API stopped: {e}"));
        let mut health = state.server_health.write().await;
        health.running = false;
        health.error = Some(e.to_string());
    }
}
