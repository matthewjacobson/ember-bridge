//! Ember Bridge — desktop bridge between the Ember embroidery editor
//! (running in a browser) and WiFi-capable embroidery machines.
//!
//! ```text
//! Browser (Ember) ──HTTP──▶ 127.0.0.1:17831 (this app) ──HTTPS──▶ machine
//! ```
//!
//! Module map:
//! * [`machine`] — manufacturer-neutral traits and models (the only machine
//!   API the rest of the app sees).
//! * [`brother`] — the Brother "pedxml" backend.
//! * [`server`] — the localhost REST API consumed by Ember and by the UI.
//! * [`config`] — persisted settings (API token, saved machines, origins).
//! * [`logging`] — in-memory app log.

pub mod brother;
pub mod config;
pub mod logging;
pub mod machine;
pub mod server;

use serde::Serialize;
use server::state::AppState;
use std::sync::Arc;
use tauri::Manager;

/// Everything the React UI needs to reach the localhost API. The UI then
/// uses the same REST API as Ember — one API surface, exercised constantly.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct LocalApiInfo {
    port: u16,
    token: String,
    version: String,
    server_running: bool,
    server_error: Option<String>,
}

#[tauri::command]
async fn local_api_info(state: tauri::State<'_, Arc<AppState>>) -> Result<LocalApiInfo, String> {
    let health = state.server_health.read().await.clone();
    Ok(LocalApiInfo {
        port: health.port,
        token: state.config.get().await.api_token,
        version: server::routes::app_version().to_string(),
        server_running: health.running,
        server_error: health.error,
    })
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info,hyper=warn,reqwest=warn".into()),
        )
        .init();

    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("platform has an app-config directory");
            let config = config::ConfigStore::load_or_create(config_dir)
                .expect("config must be readable; a corrupt file needs manual attention");

            let state = Arc::new(AppState::new(config, server::PORT));

            // A pairing request should be impossible to miss: surface the
            // window when one arrives. Tauri window handles are safe to use
            // from any thread (calls are dispatched to the main thread).
            let handle = app.handle().clone();
            *state.pairing_notify.lock().unwrap() = Some(Box::new(move || {
                if let Some(window) = handle.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.unminimize();
                    let _ = window.set_focus();
                }
            }));

            // The Tauri runtime hosts a tokio runtime; the upload worker is
            // spawned from inside it so `tokio::spawn` has a reactor.
            tauri::async_runtime::spawn({
                let state = state.clone();
                async move {
                    server::jobs::JobQueue::start_worker(state.clone());
                    server::serve(state).await;
                }
            });

            app.manage(state);
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![local_api_info])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
