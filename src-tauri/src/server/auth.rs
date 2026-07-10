//! Token authentication and browser-facing CORS for the localhost API.
//!
//! Threat model: the server only listens on 127.0.0.1, so the attackers that
//! remain are (a) arbitrary web pages running in the user's browser, which
//! can *send* requests to localhost, and (b) other local processes. Both are
//! handled by requiring a bearer token that only Ember (and our own UI) has;
//! CORS is defense in depth that additionally stops browsers from *reading*
//! responses for non-allowlisted origins.
//!
//! CORS is hand-rolled rather than tower-http because the allowlist lives in
//! mutable user config, and because Chrome's Private Network Access preflight
//! (`Access-Control-Request-Private-Network`) needs a header tower-http does
//! not emit.

use crate::server::error::ApiError;
use crate::server::state::AppState;
use axum::extract::{Request, State};
use axum::http::{header, HeaderValue, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use std::sync::Arc;

/// Paths reachable without a token. `/api/health` exists so Ember can detect
/// that the bridge is installed and running before it has been paired; it
/// exposes nothing but the app name and version.
const PUBLIC_PATHS: &[&str] = &["/api/health"];

pub async fn require_token(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    if PUBLIC_PATHS.contains(&request.uri().path()) {
        return next.run(request).await;
    }

    let presented = bearer_token(&request).or_else(|| header_str(&request, "x-ember-token"));
    let expected = state.config.get().await.api_token;

    match presented {
        Some(token) if constant_time_eq(token.as_bytes(), expected.as_bytes()) => {
            next.run(request).await
        }
        _ => ApiError::unauthorized().into_response(),
    }
}

fn bearer_token(request: &Request) -> Option<String> {
    let value = header_str(request, header::AUTHORIZATION.as_str())?;
    value
        .strip_prefix("Bearer ")
        .or_else(|| value.strip_prefix("bearer "))
        .map(|t| t.trim().to_string())
}

fn header_str(request: &Request, name: &str) -> Option<String> {
    request
        .headers()
        .get(name)
        .and_then(|v| v.to_str().ok())
        .map(|v| v.to_string())
}

/// Length-safe constant-time comparison; token checks should not leak
/// prefix-match timing, even on loopback.
fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    a.iter().zip(b).fold(0u8, |acc, (x, y)| acc | (x ^ y)) == 0
}

/// Hand-rolled CORS with a dynamic origin allowlist.
pub async fn cors(
    State(state): State<Arc<AppState>>,
    request: Request,
    next: Next,
) -> Response {
    let origin = header_str(&request, header::ORIGIN.as_str());
    let path = request.uri().path().to_string();

    let allowed = match &origin {
        Some(origin) => {
            // Health is origin-agnostic so Ember can detect the bridge
            // pre-pairing; everything else consults the allowlist.
            PUBLIC_PATHS.contains(&path.as_str())
                || origin_allowed(origin, &state.config.get().await.allowed_origins)
        }
        // Not a browser request (no Origin header): CORS does not apply.
        None => false,
    };

    // Preflight: answer directly, no token required (browsers never attach
    // custom headers to preflights).
    if request.method() == Method::OPTIONS
        && request
            .headers()
            .contains_key("access-control-request-method")
    {
        let mut response = StatusCode::NO_CONTENT.into_response();
        if allowed {
            let origin = origin.expect("allowed implies origin present");
            let headers = response.headers_mut();
            insert(headers, "access-control-allow-origin", &origin);
            insert(
                headers,
                "access-control-allow-methods",
                "GET,POST,PUT,DELETE,OPTIONS",
            );
            insert(
                headers,
                "access-control-allow-headers",
                "authorization,content-type,x-ember-token,x-filename",
            );
            insert(headers, "access-control-max-age", "600");
            insert(headers, "vary", "Origin");
            // Chrome Private Network Access: a public https:// page calling
            // 127.0.0.1 must be explicitly allowed by the local server.
            if request
                .headers()
                .contains_key("access-control-request-private-network")
            {
                insert(headers, "access-control-allow-private-network", "true");
            }
        }
        return response;
    }

    let mut response = next.run(request).await;
    if allowed {
        let origin = origin.expect("allowed implies origin present");
        let headers = response.headers_mut();
        insert(headers, "access-control-allow-origin", &origin);
        insert(headers, "vary", "Origin");
    }
    response
}

fn insert(headers: &mut axum::http::HeaderMap, name: &'static str, value: &str) {
    if let Ok(value) = HeaderValue::from_str(value) {
        headers.insert(name, value);
    }
}

/// An origin is allowed if it is loopback (the app's own UI in dev, or any
/// local tool the user runs), the app's own webview origin, on the
/// user-managed allowlist, or the allowlist contains `"*"`.
fn origin_allowed(origin: &str, allowlist: &[String]) -> bool {
    if allowlist.iter().any(|a| a == "*" || a == origin) {
        return true;
    }
    if let Some(host) = origin
        .strip_prefix("http://")
        .or_else(|| origin.strip_prefix("https://"))
    {
        let host = host.split(':').next().unwrap_or(host);
        if host == "localhost" || host == "127.0.0.1" || host == "tauri.localhost" {
            return true;
        }
    }
    // Tauri webview origin on macOS/Linux.
    origin == "tauri://localhost"
}

#[cfg(test)]
mod tests {
    use super::origin_allowed;

    #[test]
    fn origin_allowlist_rules() {
        let list = vec!["https://ember.example".to_string()];
        assert!(origin_allowed("https://ember.example", &list));
        assert!(!origin_allowed("https://evil.example", &list));
        assert!(!origin_allowed("https://ember.example.evil.com", &list));
        // Loopback and the Tauri webview are always allowed.
        assert!(origin_allowed("http://localhost:1420", &list));
        assert!(origin_allowed("http://127.0.0.1:8080", &list));
        assert!(origin_allowed("tauri://localhost", &list));
        // Wildcard.
        assert!(origin_allowed("https://anything.example", &["*".to_string()]));
    }
}
