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
pub mod dongle_setup;
pub mod emberconnect;
pub mod logging;
pub mod machine;
pub mod server;

use serde::Serialize;
use server::state::AppState;
use std::sync::Arc;
use tauri::{
    menu::{CheckMenuItemBuilder, MenuBuilder, MenuItemBuilder},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, WindowEvent,
};
use tauri_plugin_autostart::{ManagerExt, MacosLauncher};

/// Bring the main window to the foreground. Shared by the tray (show item and
/// left-click) and by an incoming pairing request, which must never be missed.
/// Window handles are thread-safe; calls are dispatched to the main thread.
fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

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
        // Autostart launches the app with `--minimized` so a login-time start
        // boots straight to the tray without popping the window.
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .setup(|app| {
            let config_dir = app
                .path()
                .app_config_dir()
                .expect("platform has an app-config directory");
            let config = config::ConfigStore::load_or_create(config_dir)
                .expect("config must be readable; a corrupt file needs manual attention");

            let state = Arc::new(AppState::new(config, server::PORT));

            // A pairing request should be impossible to miss: surface the
            // window when one arrives.
            let handle = app.handle().clone();
            *state.pairing_notify.lock().unwrap() = Some(Box::new(move || {
                show_main_window(&handle);
            }));

            // ---- system tray ------------------------------------------------
            // The bridge is a background service, so it lives in the tray: the
            // window can be closed without stopping uploads or the local API.
            let show_item = MenuItemBuilder::with_id("show", "Show Ember Bridge").build(app)?;
            let autostart_item = CheckMenuItemBuilder::with_id("autostart", "Launch at login")
                .checked(app.autolaunch().is_enabled().unwrap_or(false))
                .build(app)?;
            let quit_item = MenuItemBuilder::with_id("quit", "Quit Ember Bridge").build(app)?;
            let menu = MenuBuilder::new(app)
                .item(&show_item)
                .separator()
                .item(&autostart_item)
                .separator()
                .item(&quit_item)
                .build()?;

            let autostart_check = autostart_item.clone();
            let mut tray = TrayIconBuilder::with_id("main")
                .tooltip("Ember Bridge")
                .menu(&menu)
                // Left-click shows the window (handled below); the menu opens
                // on right-click only.
                .show_menu_on_left_click(false)
                .on_menu_event(move |app, event| match event.id().as_ref() {
                    "show" => show_main_window(app),
                    "quit" => app.exit(0),
                    "autostart" => {
                        let manager = app.autolaunch();
                        let was_enabled = manager.is_enabled().unwrap_or(false);
                        let result = if was_enabled {
                            manager.disable()
                        } else {
                            manager.enable()
                        };
                        match result {
                            // Keep the checkmark in sync with the real state
                            // (the OS operation, not the menu's optimistic flip).
                            Ok(()) => {
                                let _ = autostart_check.set_checked(!was_enabled);
                            }
                            Err(e) => {
                                tracing::warn!("could not toggle launch-at-login: {e}");
                                let _ = autostart_check.set_checked(was_enabled);
                            }
                        }
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::Click {
                        button: MouseButton::Left,
                        button_state: MouseButtonState::Up,
                        ..
                    } = event
                    {
                        show_main_window(tray.app_handle());
                    }
                });
            if let Some(icon) = app.default_window_icon().cloned() {
                tray = tray.icon(icon);
            }
            tray.build(app)?;

            // Closing the window hides it to the tray instead of quitting; the
            // only true exit is the tray's Quit item (or Cmd+Q on macOS).
            if let Some(window) = app.get_webview_window("main") {
                let hide_target = window.clone();
                window.on_window_event(move |event| {
                    if let WindowEvent::CloseRequested { api, .. } = event {
                        let _ = hide_target.hide();
                        api.prevent_close();
                    }
                });
                // Started at login (`--minimized`): begin hidden in the tray.
                if std::env::args().any(|arg| arg == "--minimized") {
                    let _ = window.hide();
                }
            }

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
        .invoke_handler(tauri::generate_handler![
            local_api_info,
            dongle_setup::dongle_list,
            dongle_setup::dongle_info,
            dongle_setup::dongle_scan,
            dongle_setup::dongle_provision,
            dongle_setup::dongle_update_firmware,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
