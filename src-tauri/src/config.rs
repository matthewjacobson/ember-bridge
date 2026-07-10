//! Persistent application configuration.
//!
//! Stored as pretty-printed JSON in the Tauri app-config directory
//! (`~/Library/Application Support/app.ember.bridge/config.json` on macOS).
//! The file contains the localhost API token, so it is created with owner-only
//! permissions on Unix.

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::path::PathBuf;
use tokio::sync::RwLock;

/// A machine the user has saved (manually or from discovery).
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SavedMachine {
    pub ip: IpAddr,
    /// User-facing nickname, e.g. "Sewing room".
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nickname: Option<String>,
    /// Backend that recognized the machine when it was saved, if known.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manufacturer: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AppConfig {
    /// Bearer token required on every localhost API request. Generated on
    /// first launch; the user pastes it into Ember once.
    pub api_token: String,
    /// Web origins allowed to call the API from a browser (CORS allowlist).
    /// Localhost origins and the app's own webview are always allowed;
    /// `"*"` allows every origin (the token remains required).
    #[serde(default)]
    pub allowed_origins: Vec<String>,
    #[serde(default)]
    pub machines: Vec<SavedMachine>,
}

impl AppConfig {
    fn new_with_token() -> Self {
        Self {
            api_token: generate_token(),
            allowed_origins: Vec::new(),
            machines: Vec::new(),
        }
    }
}

/// 128 bits of randomness, hex-encoded.
fn generate_token() -> String {
    use rand::Rng;
    let mut rng = rand::rng();
    (0..32)
        .map(|_| format!("{:x}", rng.random_range(0..16u8)))
        .collect()
}

/// Owns the on-disk config and serializes access to it.
pub struct ConfigStore {
    path: PathBuf,
    config: RwLock<AppConfig>,
}

impl ConfigStore {
    /// Load the config, creating it (with a fresh token) on first launch.
    pub fn load_or_create(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        let path = dir.join("config.json");
        let config = match std::fs::read_to_string(&path) {
            Ok(raw) => serde_json::from_str::<AppConfig>(&raw).map_err(|e| {
                // A corrupt config could silently regenerate the token and
                // break the Ember pairing; fail loudly instead.
                std::io::Error::new(
                    std::io::ErrorKind::InvalidData,
                    format!("{} is not a valid config file: {e}", path.display()),
                )
            })?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                let config = AppConfig::new_with_token();
                write_config(&path, &config)?;
                config
            }
            Err(e) => return Err(e),
        };
        Ok(Self {
            path,
            config: RwLock::new(config),
        })
    }

    pub async fn get(&self) -> AppConfig {
        self.config.read().await.clone()
    }

    /// Mutate the config and persist it atomically.
    pub async fn update<F: FnOnce(&mut AppConfig)>(&self, mutate: F) -> std::io::Result<AppConfig> {
        let mut guard = self.config.write().await;
        mutate(&mut guard);
        write_config(&self.path, &guard)?;
        Ok(guard.clone())
    }
}

/// Write via a temp file + rename so a crash can't truncate the config,
/// with owner-only permissions because it contains the API token.
fn write_config(path: &std::path::Path, config: &AppConfig) -> std::io::Result<()> {
    let json = serde_json::to_string_pretty(config).expect("config serialization cannot fail");
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, json)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&tmp, std::fs::Permissions::from_mode(0o600))?;
    }
    std::fs::rename(&tmp, path)
}
